//! Smart Governor: the thread that watches the compositor (fullscreen /
//! maximized state, active window geometry for `window-inside` /
//! `window-outside` placement, cursor) and reports `GovernorEvent`s. What it
//! watches comes from `compositor::current()`.
//!
//! niri's implementation still lives in this file (see `backend/niri.rs`).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use calloop::channel::Sender;

use crate::render::character::WindowRect;

#[derive(Debug, Clone)]
pub enum GovernorEvent {
    WindowState {
        fullscreen: bool,
        maximized: bool,
        active_rect: Option<WindowRect>,
    },
    FocusedMonitor(String),
    CursorPos(f32, f32),
    Power,
}

pub fn spawn_governor(tx: Sender<GovernorEvent>) {
    super::notify::spawn();
    std::thread::Builder::new()
        .name("anima-governor".into())
        .spawn(move || run(tx))
        .ok();
}

fn run(tx: Sender<GovernorEvent>) {
    if let Some((fs, max, rect)) = query_window_state() {
        let _ = tx.send(GovernorEvent::WindowState {
            fullscreen: fs,
            maximized: max,
            active_rect: rect,
        });
    }

    super::compositor::current().run(tx);
}

/// Window state every 2 s, for compositors without an event stream.
pub(super) fn poll_forever(tx: Sender<GovernorEvent>) {
    let mut last = None;
    loop {
        let state = query_window_state();
        if state != last {
            if let Some((fs, max, rect)) = state {
                if tx
                    .send(GovernorEvent::WindowState {
                        fullscreen: fs,
                        maximized: max,
                        active_rect: rect,
                    })
                    .is_err()
                {
                    return;
                }
            }
            last = state;
        }
        if tx.send(GovernorEvent::Power).is_err() {
            return;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Fullscreen / maximized state and the active window, from the compositor.
pub(super) fn query_window_state() -> Option<(bool, bool, Option<WindowRect>)> {
    super::compositor::current().window_state()
}

// niri: $NIRI_SOCKET, one JSON request per line

/// Workspaces on the focused niri output.
#[derive(Clone, Debug, Default)]
pub struct NiriWorkspaces {
    pub active: u64,
    pub all: Vec<u64>,
    /// workspaces that have at least one window
    pub busy: Vec<u64>,
}


/// What characters walk along on the focused output, logical pixels: niri's
/// scrolling strip, or Hyprland's open workspaces side by side.
#[derive(Clone, Copy, Debug, Default)]
pub struct Strip {
    /// how far the strip is scrolled (left edge of the screen in strip coords)
    pub view_x: f32,
    /// total width of all columns
    pub width: f32,
    pub screen_w: f32,
    pub ws: u64,
    /// view_x comes straight from niri (no smoothing needed)
    pub exact: bool,
}

// tiled windows on the active workspace: (id, x of its column on our strip, tile width),
// so a WindowLayoutsChanged event alone gives the scroll position
static NIRI_WIN_X: std::sync::Mutex<Vec<(u64, f32, f32)>> = std::sync::Mutex::new(Vec::new());

/// Fast path for the per-frame layout events niri sends while scrolling.
/// False if the event needs a full refresh (unknown window, resize, ...).
fn niri_layouts_changed(line: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { return false };
    let Some(changes) = v["WindowLayoutsChanged"]["changes"].as_array() else { return false };
    let wins = NIRI_WIN_X.lock().unwrap();
    let mut view = None;
    for c in changes {
        let (Some(id), layout) = (c[0].as_u64(), &c[1]) else { return false };
        let Some(&(_, col_x, width)) = wins.iter().find(|w| w.0 == id) else { continue };
        let w = layout["tile_size"][0].as_f64().unwrap_or(0.0) as f32;
        if (w - width).abs() > 0.5 {
            return false;
        }
        if let Some(x) = layout["tile_pos_in_workspace_view"][0].as_f64() {
            view.get_or_insert(col_x - x as f32);
        }
    }
    let Some(view) = view else { return false };
    if let Some(st) = STRIP.lock().unwrap().as_mut() {
        st.view_x = view;
    }
    true
}

// something changed: the refresh thread should take a look
static NIRI_DIRTY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// each window as last seen, without its title
static NIRI_SEEN: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<u64, String>>> =
    std::sync::LazyLock::new(Default::default);

/// True if this WindowOpenedOrChanged differs from last time only by title.
fn niri_only_title(line: &str) -> bool {
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(line) else { return false };
    let w = &mut v["WindowOpenedOrChanged"]["window"];
    let Some(id) = w["id"].as_u64() else { return false };
    if let Some(o) = w.as_object_mut() {
        o.remove("title");
    }
    let now = w.to_string();
    NIRI_SEEN.lock().unwrap().insert(id, now.clone()).is_some_and(|old| old == now)
}

// last time something that can scroll the strip happened (niri event, swipe)
static NIRI_ACTIVITY: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
// after such an event keep asking this long, a new window may take a moment to start the scroll
const NIRI_WAKE: Duration = Duration::from_millis(500);
// the strip counts as stopped after not moving for this long
const NIRI_SETTLE: Duration = Duration::from_millis(150);

fn niri_activity() {
    *NIRI_ACTIVITY.lock().unwrap() = Some(std::time::Instant::now());
}

/// Ask niri for the windows and take the scroll position from any tiled one
/// we know. The patched niri updates positions without sending events (that
/// flooded other clients), so we poll while it reports them.
fn niri_poll_view(sock: &Path) -> bool {
    let Some(windows) = niri_request(sock, "\"Windows\"")
        .and_then(|v| v.get("Windows").and_then(|w| w.as_array()).cloned())
    else {
        return false;
    };
    let wins = NIRI_WIN_X.lock().unwrap();
    for w in &windows {
        let Some(&(_, col_x, width)) = wins.iter().find(|c| Some(c.0) == w["id"].as_u64()) else { continue };
        let layout = &w["layout"];
        if (layout["tile_size"][0].as_f64().unwrap_or(0.0) as f32 - width).abs() > 0.5 {
            return false;
        }
        if let Some(x) = layout["tile_pos_in_workspace_view"][0].as_f64() {
            if let Some(st) = STRIP.lock().unwrap().as_mut() {
                st.view_x = col_x - x as f32;
            }
            return true;
        }
    }
    false
}

// niri reports tiled window positions (patched niri): no guessing needed
static NIRI_EXACT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// a three-finger swipe is moving the strip; don't recompute it meanwhile
static NIRI_GESTURE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// view when the swipe started, and whether it just ended (niri then snaps)
static GESTURE_START: std::sync::Mutex<Option<f32>> = std::sync::Mutex::new(None);
static GESTURE_ENDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
// finger -> strip correction on top of niri's formula, measured on this
// touchpad (our libinput acceleration estimate runs a bit low)
const GESTURE_SCALE: f32 = 1.2;

pub(super) fn niri_gesture(active: bool) {
    niri_activity();
    if active {
        *GESTURE_START.lock().unwrap() = STRIP.lock().unwrap().map(|s| s.view_x);
    } else {
        GESTURE_ENDED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    NIRI_GESTURE.store(active, std::sync::atomic::Ordering::Relaxed);
}

/// Follow niri's touchpad swipe: the strip moves by the (accelerated) finger
/// travel times output width / 1200, like niri's view_offset_gesture_update.
pub(super) fn niri_gesture_scroll(dx: f32) {
    niri_activity();
    if !NIRI_GESTURE.load(std::sync::atomic::Ordering::Relaxed)
        || NIRI_EXACT.load(std::sync::atomic::Ordering::Relaxed)
    {
        return;
    }
    // Never take NIRI_VIEWS while holding STRIP: `update_niri_strip` takes
    // them the other way round (NIRI_VIEWS, then STRIP), and a three-finger
    // swipe runs this from the pointer thread while the view thread polls.
    let (view, ws) = {
        let mut strip = STRIP.lock().unwrap();
        let Some(st) = strip.as_mut() else { return };
        st.view_x = (st.view_x + dx * st.screen_w / 1200.0 * GESTURE_SCALE).clamp(-st.screen_w * 0.2, st.width - st.screen_w * 0.8);
        (st.view_x, st.ws)
    };
    if let Some(v) = NIRI_VIEWS.lock().unwrap().iter_mut().find(|v| v.0 == ws) {
        v.1 = view;
    }
}

pub static STRIP: std::sync::Mutex<Option<Strip>> = std::sync::Mutex::new(None);

/// Columns of tiled windows on the active niri workspace: (x on the strip, width), logical px.
static OVERVIEW_OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static FOCUSED_APP: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// app_id of the focused window, as last seen.
pub fn focused_app() -> Option<String> {
    FOCUSED_APP.lock().unwrap().clone()
}

static WORKSPACES: std::sync::Mutex<Option<Vec<Space>>> = std::sync::Mutex::new(None);
static WINDOW_COUNTS: std::sync::Mutex<Vec<(u64, usize)>> = std::sync::Mutex::new(Vec::new());

/// An open park, cafe or farm window: its workspace, and its column there
/// (x on the workspace's strip, width), logical px.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub place: String,
    pub ws: u64,
    pub x: f32,
    pub w: f32,
    pub h: f32,
}

static PLACE_WS: std::sync::Mutex<Vec<Place>> = std::sync::Mutex::new(Vec::new());

/// Where the park, cafe or farm window is, if it's open.
pub fn place_workspace(place: &str) -> Option<u64> {
    place_rect(place).map(|p| p.ws)
}

pub fn place_rect(place: &str) -> Option<Place> {
    PLACE_WS.lock().unwrap().iter().find(|p| p.place == place).cloned()
}

/// Which of them is at `x` on workspace `ws`.
pub fn place_at(ws: u64, x: f32) -> Option<Place> {
    PLACE_WS.lock().unwrap().iter().find(|p| p.ws == ws && x >= p.x && x <= p.x + p.w).cloned()
}

/// All the open ones.
pub fn places() -> Vec<Place> {
    PLACE_WS.lock().unwrap().clone()
}

/// How many windows a workspace has.
pub fn window_count(id: u64) -> usize {
    WINDOW_COUNTS.lock().unwrap().iter().find(|c| c.0 == id).map_or(0, |c| c.1)
}

/// A workspace of the focused output, as niri draws it right now.
#[derive(Clone, Debug, PartialEq)]
pub struct Space {
    pub id: u64,
    /// x, y, width, height, logical px
    pub rect: [f32; 4],
    pub active: bool,
    pub has_windows: bool,
}

/// None without the `WorkspacesGeometry` request of contrib's niri patch:
/// the characters then all share the one view, as on other compositors.
pub fn workspaces() -> Option<Vec<Space>> {
    WORKSPACES.lock().unwrap().clone()
}

fn read_workspaces(sock: &Path) -> Option<Vec<Space>> {
    let reply = niri_request(sock, "\"WorkspacesGeometry\"")?;
    let list = reply.get("WorkspacesGeometry")?.as_array()?.iter().filter_map(|w| {
        let f = |k: &str| w.get(k).and_then(|x| x.as_f64()).map(|x| x as f32);
        let b = |k: &str| w.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
        Some(Space { id: w.get("id")?.as_u64()?, rect: [f("x")?, f("y")?, f("width")?, f("height")?], active: b("active"), has_windows: b("has_windows") })
    });
    Some(list.collect())
}

/// Every frame while workspaces are moving (a switch, a swipe, the overview),
/// a few times a second otherwise: each question costs niri a little.
fn spawn_workspace_poll(sock: std::path::PathBuf) {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("anima-workspaces".into()).spawn(move || {
            let mut last_ask = std::time::Instant::now() - Duration::from_secs(1);
            let mut moved_at = last_ask;
            loop {
                let woken = NIRI_ACTIVITY.lock().unwrap().is_some_and(|t| t.elapsed() < NIRI_WAKE);
                let busy = woken
                    || moved_at.elapsed() < NIRI_SETTLE
                    || OVERVIEW_OPEN.load(std::sync::atomic::Ordering::Relaxed)
                    || NIRI_GESTURE.load(std::sync::atomic::Ordering::Relaxed);
                if busy || last_ask.elapsed() > Duration::from_millis(250) {
                    last_ask = std::time::Instant::now();
                    let list = read_workspaces(&sock);
                    let known = list.is_some();
                    let mut held = WORKSPACES.lock().unwrap();
                    if *held != list {
                        moved_at = last_ask;
                    }
                    *held = list;
                    drop(held);
                    if !known {
                        std::thread::sleep(Duration::from_secs(1));
                    }
                }
                std::thread::sleep(Duration::from_millis(7));
            }
        });
    });
}

/// Where a window sits on the strip: (x, width).
pub fn niri_window_x(id: u64) -> Option<(f32, f32)> {
    NIRI_WIN_X.lock().unwrap().iter().find(|w| w.0 == id).map(|w| (w.1, w.2))
}

pub fn niri_columns() -> Vec<(f32, f32)> {
    let mut cols: Vec<(f32, f32)> = Vec::new();
    for &(_, x, w) in NIRI_WIN_X.lock().unwrap().iter() {
        if !cols.iter().any(|c| c.0 == x) {
            cols.push((x, w));
        }
    }
    cols
}

// remembered scroll position per workspace
static NIRI_VIEWS: std::sync::Mutex<Vec<(u64, f32)>> = std::sync::Mutex::new(Vec::new());

/// niri doesn't report the scroll position, so follow its rule: scroll as
/// little as needed to show the focused column in full.
fn update_niri_strip(sock: &Path) {
    if NIRI_GESTURE.load(std::sync::atomic::Ordering::Relaxed)
        && !NIRI_EXACT.load(std::sync::atomic::Ordering::Relaxed)
    {
        return;
    }
    let Some(ws) = query_niri_workspaces(sock) else { return };
    let Some(windows) = niri_request(sock, "\"Windows\"")
        .and_then(|v| v.get("Windows").and_then(|w| w.as_array()).cloned())
    else {
        return;
    };
    {
        let mut counts: Vec<(u64, usize)> = Vec::new();
        for id in windows.iter().filter_map(|w| w.get("workspace_id").and_then(|v| v.as_u64())) {
            match counts.iter_mut().find(|c| c.0 == id) {
                Some(c) => c.1 += 1,
                None => counts.push((id, 1)),
            }
        }
        *WINDOW_COUNTS.lock().unwrap() = counts;
    }
    let screen_w = niri_output_rect(sock).map(|r| r.2).unwrap_or(1920.0);
    let gap = niri_gaps();
    {
        // the park, cafe and farm windows (told apart by their titles), and
        // where on their workspace's strip each one's column is
        let places: Vec<Place> = windows
            .iter()
            .filter(|w| w.get("app_id").and_then(|a| a.as_str()) == Some(crate::render::character::park::APP_ID))
            .filter_map(|w| {
                let title = w.get("title")?.as_str()?;
                let place = crate::render::character::park::PLACES.iter().find(|p| p.1 == title)?.0;
                let ws = w.get("workspace_id")?.as_u64()?;
                let col = w["layout"]["pos_in_scrolling_layout"].get(0).and_then(|v| v.as_u64());
                // the columns of that workspace, left to right
                let mut cols: Vec<(u64, f32)> = Vec::new();
                for o in windows.iter().filter(|o| o.get("workspace_id").and_then(|v| v.as_u64()) == Some(ws)) {
                    let Some(c) = o["layout"]["pos_in_scrolling_layout"].get(0).and_then(|v| v.as_u64()) else { continue };
                    let width = o["layout"]["tile_size"].get(0).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                    match cols.iter_mut().find(|x| x.0 == c) {
                        Some(x) => x.1 = x.1.max(width),
                        None => cols.push((c, width)),
                    }
                }
                cols.sort_by_key(|c| c.0);
                let mut x = gap;
                let mut at = (0.0, screen_w);
                let height = w["layout"]["tile_size"].get(1).and_then(|v| v.as_f64()).unwrap_or(1000.0) as f32;
                for (c, width) in &cols {
                    if Some(*c) == col {
                        at = (x, *width);
                    }
                    x += width + gap;
                }
                Some(Place { place: place.to_string(), ws, x: at.0, w: at.1, h: height })
            })
            .collect();
        *PLACE_WS.lock().unwrap() = places;
    }
    // column index -> width, and the focused column
    let mut cols: Vec<(u64, f32)> = Vec::new();
    let mut focused_col = None;
    // (column, x on screen) of a tiled window, when niri reports it
    let mut seen_pos: Option<(u64, f32)> = None;
    for w in &windows {
        if w.get("workspace_id").and_then(|v| v.as_u64()) != Some(ws.active)
            || w.get("is_floating").and_then(|v| v.as_bool()) == Some(true)
        {
            continue;
        }
        let layout = &w["layout"];
        let Some(col) = layout["pos_in_scrolling_layout"].get(0).and_then(|v| v.as_u64()) else { continue };
        let width = layout["tile_size"].get(0).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        match cols.iter_mut().find(|c| c.0 == col) {
            Some(c) => c.1 = c.1.max(width),
            None => cols.push((col, width)),
        }
        if w.get("is_focused").and_then(|v| v.as_bool()) == Some(true) {
            focused_col = Some(col);
        }
        if let Some(x) = layout["tile_pos_in_workspace_view"].get(0).and_then(|v| v.as_f64()) {
            seen_pos.get_or_insert((col, x as f32));
        }
    }
    cols.sort_by_key(|c| c.0);
    {
        let mut col_x = Vec::new();
        let mut x = gap;
        for (col, w) in &cols {
            col_x.push((*col, x, *w));
            x += w + gap;
        }
        let mut cache = NIRI_WIN_X.lock().unwrap();
        cache.clear();
        for w in &windows {
            if w.get("workspace_id").and_then(|v| v.as_u64()) != Some(ws.active) {
                continue;
            }
            let (Some(id), Some(col)) = (w["id"].as_u64(), w["layout"]["pos_in_scrolling_layout"][0].as_u64()) else {
                continue;
            };
            if let Some(&(_, x, cw)) = col_x.iter().find(|c| c.0 == col) {
                cache.push((id, x, cw));
            }
        }
    }
    let width = cols.iter().map(|c| c.1 + gap).sum::<f32>() + gap;
    let mut views = NIRI_VIEWS.lock().unwrap();
    let mut view = views.iter().find(|v| v.0 == ws.active).map(|v| v.1).unwrap_or(0.0);
    let ended = GESTURE_ENDED.swap(false, std::sync::atomic::Ordering::Relaxed);
    if let Some((pc, px)) = seen_pos {
        // exact: our x of that column minus where niri shows it
        NIRI_EXACT.store(true, std::sync::atomic::Ordering::Relaxed);
        let mut x = gap;
        for (col, w) in &cols {
            if *col == pc {
                view = x - px;
                break;
            }
            x += w + gap;
        }
    } else if let Some(fc) = focused_col {
        let mut x = gap;
        for (col, w) in &cols {
            if *col == fc {
                let (left, right) = (x - gap, x + w + gap - screen_w);
                if ended {
                    // after a swipe niri snaps the focused column to one edge:
                    // take the one nearest to our estimate
                    let est = view;
                    view = if (left - est).abs() <= (right - est).abs() { left } else { right };
                    if let Some(start) = GESTURE_START.lock().unwrap().take() {
                        log::debug!("niri swipe: est {:.0} real {:.0}", est - start, view - start);
                    }
                } else {
                    // smallest scroll that shows [x - gap, x + w + gap]
                    view = view.min(left).max(right);
                }
                break;
            }
            x += w + gap;
        }
    }
    if seen_pos.is_none() {
        view = view.clamp(0.0, (width - screen_w).max(0.0));
    }
    match views.iter_mut().find(|v| v.0 == ws.active) {
        Some(v) => v.1 = view,
        None => views.push((ws.active, view)),
    }
    // release before locking STRIP (see niri_gesture_scroll for the order)
    drop(views);
    let new = Strip { view_x: view, width: width.max(screen_w), screen_w, ws: ws.active, exact: seen_pos.is_some() };
    let mut strip = STRIP.lock().unwrap();
    if strip.is_none_or(|o| o.view_x != new.view_x || o.width != new.width) {
        log::debug!("niri strip: view {:.0} of {:.0}", new.view_x, new.width);
    }
    *strip = Some(new);
}

/// `layout { gaps N }` from the niri config (DMS keeps it in dms/layout.kdl).
fn niri_gaps() -> f32 {
    static GAPS: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *GAPS.get_or_init(|| {
        let dir = dirs::config_dir().unwrap_or_default().join("niri");
        let mut files = vec![dir.join("config.kdl")];
        if let Ok(rd) = std::fs::read_dir(dir.join("dms")) {
            files.extend(rd.flatten().map(|e| e.path()));
        }
        files
            .iter()
            .filter_map(|f| std::fs::read_to_string(f).ok())
            .flat_map(|t| t.lines().map(str::to_owned).collect::<Vec<_>>())
            .filter(|l| !l.trim_start().starts_with("//"))
            .find_map(|l| l.trim().strip_prefix("gaps ").and_then(|v| v.trim().parse().ok()))
            .unwrap_or(16.0)
    })
}

fn query_niri_workspaces(sock: &Path) -> Option<NiriWorkspaces> {
    let out = niri_focused_output(sock)?.get("name")?.as_str()?.to_string();
    let list = niri_request(sock, "\"Workspaces\"")?.get("Workspaces")?.as_array()?.clone();
    let mut ws = NiriWorkspaces::default();
    for w in list.iter().filter(|w| w.get("output").and_then(|o| o.as_str()) == Some(out.as_str())) {
        let Some(id) = w.get("id").and_then(|v| v.as_u64()) else { continue };
        ws.all.push(id);
        if w.get("is_active").and_then(|v| v.as_bool()) == Some(true) {
            ws.active = id;
        }
        if w.get("active_window_id").is_some_and(|v| !v.is_null()) {
            ws.busy.push(id);
        }
    }
    Some(ws)
}

pub(super) fn niri_socket() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("NIRI_SOCKET")?);
    p.exists().then_some(p)
}

/// Exact pointer position from niri (patched niri only): Some(None) when the
/// pointer is on no output, None when niri can't tell.
pub(super) fn niri_cursor() -> Option<Option<(f32, f32)>> {
    let sock = niri_socket()?;
    let v = niri_request(&sock, "\"CursorPosition\"")?;
    let c = v.get("CursorPosition")?;
    if c.is_null() {
        return Some(None);
    }
    Some(Some((c["x"].as_f64()? as f32, c["y"].as_f64()? as f32)))
}

fn niri_request(sock: &Path, req: &str) -> Option<serde_json::Value> {
    let mut stream = UnixStream::connect(sock).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(200))).ok()?;
    stream.write_all(req.as_bytes()).ok()?;
    stream.write_all(b"\n").ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    let v: serde_json::Value = serde_json::from_str(&line).ok()?;
    v.get("Ok").cloned()
}

fn niri_focused_output(sock: &Path) -> Option<serde_json::Value> {
    niri_request(sock, "\"FocusedOutput\"")?.get("FocusedOutput").cloned()
}

pub(super) fn query_niri_window(sock: &Path) -> Option<(bool, bool, Option<WindowRect>)> {
    let win = niri_request(sock, "\"FocusedWindow\"")?.get("FocusedWindow").cloned()?;
    *FOCUSED_APP.lock().unwrap() = win.get("app_id").and_then(|a| a.as_str()).map(String::from);
    if win.is_null() {
        return Some((false, false, None));
    }
    let logical = niri_focused_output(sock).and_then(|o| o.get("logical").cloned());
    let num = |v: Option<&serde_json::Value>, i: usize| {
        v.and_then(|a| a.get(i)).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32
    };
    let layout = win.get("layout");
    let wsize = layout.and_then(|l| l.get("window_size"));
    let tsize = layout.and_then(|l| l.get("tile_size"));
    let (ox, oy, ow, oh) = match &logical {
        Some(l) => (
            l["x"].as_f64().unwrap_or(0.0) as f32,
            l["y"].as_f64().unwrap_or(0.0) as f32,
            l["width"].as_f64().unwrap_or(0.0) as f32,
            l["height"].as_f64().unwrap_or(0.0) as f32,
        ),
        None => (0.0, 0.0, 0.0, 0.0),
    };
    // niri has no fullscreen flag; a window covering the whole output counts
    let fullscreen = ow > 0.0 && num(wsize, 0) >= ow && num(wsize, 1) >= oh;
    let maximized = !fullscreen && ow > 0.0 && num(tsize, 0) >= ow * 0.95;

    let pos = layout.and_then(|l| l.get("tile_pos_in_workspace_view"));
    let off = layout.and_then(|l| l.get("window_offset_in_tile"));
    let rect = match pos {
        Some(p) if !p.is_null() => {
            let w = num(wsize, 0);
            let h = num(wsize, 1);
            (w > 0.0 && h > 0.0).then(|| WindowRect {
                x: ox + num(Some(p), 0) + num(off, 0),
                y: oy + num(Some(p), 1) + num(off, 1),
                w,
                h,
            })
        }
        _ => None,
    };
    Some((fullscreen, maximized, rect))
}

pub(super) fn run_niri(tx: Sender<GovernorEvent>, sock: &Path) {
    let Ok(mut stream) = UnixStream::connect(sock) else {
        poll_forever(tx);
        return;
    };
    if stream.write_all(b"\"EventStream\"\n").is_err() {
        poll_forever(tx);
        return;
    }

    // power checks + window geometry (niri emits no event on resize/scroll)
    let tx_poll = tx.clone();
    let sock_poll = sock.to_path_buf();
    std::thread::Builder::new()
        .name("anima-niri-poll".into())
        .spawn(move || {
            let mut last = None;
            let mut ticks = 0u32;
            loop {
                // with exact positions the event stream keeps it current
                if !NIRI_EXACT.load(std::sync::atomic::Ordering::Relaxed) {
                    update_niri_strip(&sock_poll);
                }
                let state = query_niri_window(&sock_poll);
                if state != last {
                    last = state;
                    if let Some((fs, max, rect)) = state {
                        if tx_poll
                            .send(GovernorEvent::WindowState { fullscreen: fs, maximized: max, active_rect: rect })
                            .is_err()
                        {
                            return;
                        }
                    }
                }
                ticks = ticks.wrapping_add(1);
                if ticks % 10 == 0 && tx_poll.send(GovernorEvent::Power).is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        })
        .ok();

    // cursor: niri has no IPC for it, approximate from evdev
    let bounds: super::pointer::Bounds = std::sync::Arc::new(std::sync::Mutex::new((
        niri_output_rect(sock).unwrap_or((0.0, 0.0, 1920.0, 1080.0)),
        niri_focused_output(sock)
            .and_then(|o| o.get("name").and_then(|n| n.as_str()).map(String::from))
            .unwrap_or_default(),
    )));
    super::pointer::spawn(tx.clone(), bounds.clone(), false);

    // exact scroll position: poll about once per frame
    let sock_view = sock.to_path_buf();
    std::thread::Builder::new()
        .name("anima-niri-view".into())
        .spawn(move || {
            let mut last = std::time::Instant::now();
            let mut last_view = f32::NAN;
            let mut moved_at = std::time::Instant::now() - Duration::from_secs(1);
            loop {
            if NIRI_EXACT.load(std::sync::atomic::Ordering::Relaxed) {
                // each request costs niri a bit, and it's busy animating right then: ask
                // every frame only just after something happened or while the strip is
                // really moving; check often so a scroll is caught at once
                let woken = NIRI_ACTIVITY
                    .lock()
                    .unwrap()
                    .is_some_and(|t| t.elapsed() < NIRI_WAKE);
                let busy = woken
                    || moved_at.elapsed() < NIRI_SETTLE
                    || NIRI_GESTURE.load(std::sync::atomic::Ordering::Relaxed);
                if busy || last.elapsed() > Duration::from_millis(250) {
                    niri_poll_view(&sock_view);
                    last = std::time::Instant::now();
                    let view = STRIP.lock().unwrap().map_or(f32::NAN, |s| s.view_x);
                    if (view - last_view).abs() > 0.01 {
                        moved_at = last;
                    }
                    last_view = view;
                }
                std::thread::sleep(Duration::from_millis(7));
            } else {
                std::thread::sleep(Duration::from_millis(200));
            }
            }
        })
        .ok();

    log::info!("listening to niri events at {}", sock.display());

    // a burst of events is one look at niri, not a handful of requests each
    let (sock_bg, tx_bg, bounds_bg) = (sock.to_path_buf(), tx.clone(), bounds.clone());
    std::thread::Builder::new()
        .name("anima-niri-refresh".into())
        .spawn(move || {
            let mut last_output = String::new();
            loop {
                if !NIRI_DIRTY.swap(false, std::sync::atomic::Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(8));
                    continue;
                }
                update_niri_strip(&sock_bg);
                if let Some(o) = niri_focused_output(&sock_bg) {
                    if let Some(name) = o.get("name").and_then(|n| n.as_str()).map(String::from) {
                        if name != last_output {
                            last_output = name.clone();
                            if let Some(r) = niri_output_rect(&sock_bg) {
                                *bounds_bg.lock().unwrap() = (r, name.clone());
                            }
                            if tx_bg.send(GovernorEvent::FocusedMonitor(name)).is_err() {
                                return;
                            }
                        }
                    }
                }
                if let Some((fs, max, rect)) = query_niri_window(&sock_bg) {
                    if tx_bg
                        .send(GovernorEvent::WindowState { fullscreen: fs, maximized: max, active_rect: rect })
                        .is_err()
                    {
                        return;
                    }
                }
                std::thread::sleep(Duration::from_millis(33));
            }
        })
        .ok();

    spawn_workspace_poll(sock.to_path_buf());
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        super::desktop::window_event(&line);
        if line.starts_with("{\"OverviewOpenedOrClosed\"") {
            OVERVIEW_OPEN.store(line.contains("\"is_open\":true"), std::sync::atomic::Ordering::Relaxed);
            continue;
        }
        // things that can scroll the strip (title changes etc. can't)
        if ["WindowFocusChanged", "WindowLayoutsChanged", "WorkspaceActivated", "WorkspaceActiveWindowChanged", "WindowClosed", "WindowsChanged"]
            .iter()
            .any(|k| line.starts_with(&format!("{{\"{k}\"")))
        {
            niri_activity();
        }
        let relevant = ["WindowFocusChanged", "WindowOpenedOrChanged", "WindowClosed",
            "WorkspaceActivated", "WindowLayoutsChanged", "WindowsChanged"]
            .iter()
            .any(|k| line.starts_with(&format!("{{\"{k}\"")));
        if !relevant {
            continue;
        }
        // while scrolling niri sends one of these per frame: read the
        // position straight from it instead of asking niri again
        if line.starts_with("{\"WindowLayoutsChanged\"") && niri_layouts_changed(&line) {
            continue;
        }
        // a browser or terminal retitling itself changes nothing we care about
        if line.starts_with("{\"WindowOpenedOrChanged\"") && niri_only_title(&line) {
            continue;
        }
        NIRI_DIRTY.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    poll_forever(tx);
}

fn niri_output_rect(sock: &Path) -> Option<(f32, f32, f32, f32)> {
    let l = niri_focused_output(sock)?.get("logical")?.clone();
    let f = |k: &str| l.get(k).and_then(|v| v.as_f64()).map(|v| v as f32);
    Some((f("x")?, f("y")?, f("width")?, f("height")?))
}
