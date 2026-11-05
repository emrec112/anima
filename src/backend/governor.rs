//! Smart Governor: compositor fullscreen/maximized detection, active window
//! geometry tracking (for `window-inside` / `window-outside` character placement),
//! and cursor tracking.
//!
//! Uses Hyprland's UNIX sockets (`.socket.sock` and `.socket2.sock`) directly
//! without spawning subprocesses, keeping CPU overhead at ~0.0%.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
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

    if let Some(sock) = niri_socket() {
        run_niri(tx, &sock);
        return;
    }

    let Some(path) = hypr_socket2() else {
        poll_forever(tx);
        return;
    };

    let stream = match UnixStream::connect(&path) {
        Ok(stream) => stream,
        Err(err) => {
            log::debug!("cannot connect to Hyprland IPC ({}): {err}", path.display());
            poll_forever(tx);
            return;
        }
    };

    // clicks, drags and gestures come from evdev; the position stays exact
    let bounds: super::pointer::Bounds = std::sync::Arc::new(std::sync::Mutex::new(
        hypr_output_rect().unwrap_or(((0.0, 0.0, 1920.0, 1080.0), String::new())),
    ));
    super::pointer::spawn(tx.clone(), bounds.clone(), true);

    // Lightweight cursor + active window geometry tracker (handles dragging/resizing windows smoothly).
    let tx_poll = tx.clone();
    std::thread::Builder::new()
        .name("anima-cursor".into())
        .spawn(move || {
            let mut ticks = 0u32;
            let mut last_pos = (-1.0f32, -1.0f32);
            let mut last_rect: Option<WindowRect> = None;
            loop {
                if let Some((x, y)) = query_hypr_cursor() {
                    if (x - last_pos.0).abs() > 1.0 || (y - last_pos.1).abs() > 1.0 {
                        last_pos = (x, y);
                        *super::pointer::SYNC.lock().unwrap() = Some((x, y));
                        if tx_poll.send(GovernorEvent::CursorPos(x, y)).is_err() {
                            return;
                        }
                    }
                }
                if ticks % 3 == 0 {
                    if let Some((fs, max, rect)) = query_hypr_active_window() {
                        if rect != last_rect {
                            last_rect = rect;
                            if tx_poll
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
                    }
                }
                ticks = ticks.wrapping_add(1);
                if ticks % 60 == 0 {
                    if tx_poll.send(GovernorEvent::Power).is_err() {
                        return;
                    }
                }
                std::thread::sleep(Duration::from_millis(45));
            }
        })
        .ok();

    update_hypr_strip();
    log::info!("listening to Hyprland events at {}", path.display());
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };

        if ["workspace>>", "focusedmon>>", "createworkspace>>", "destroyworkspace>>", "moveworkspace>>",
            "openwindow>>", "closewindow>>", "movewindow>>"]
            .iter()
            .any(|k| line.starts_with(k))
        {
            update_hypr_strip();
        }

        if let Some(rest) = line.strip_prefix("focusedmon>>") {
            if let Some((name, _workspace)) = rest.split_once(',') {
                if let Some(r) = hypr_output_rect() {
                    *bounds.lock().unwrap() = r;
                }
                if tx
                    .send(GovernorEvent::FocusedMonitor(name.to_string()))
                    .is_err()
                {
                    return;
                }
            }
            continue;
        }

        let relevant = line.starts_with("fullscreen>>")
            || line.starts_with("activewindow>>")
            || line.starts_with("workspace>>")
            || line.starts_with("createwindow>>")
            || line.starts_with("closewindow>>")
            || line.starts_with("movewindow>>")
            || line.starts_with("openwindow>>")
            || line.starts_with("changefloatingmode>>");

        if relevant {
            if let Some((fs, max, rect)) = query_window_state() {
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
        }
    }
}

fn poll_forever(tx: Sender<GovernorEvent>) {
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

fn hypr_dir() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    Some(PathBuf::from(runtime).join("hypr").join(signature))
}

fn hypr_socket1() -> Option<PathBuf> {
    let p = hypr_dir()?.join(".socket.sock");
    p.exists().then_some(p)
}

fn hypr_socket2() -> Option<PathBuf> {
    let p = hypr_dir()?.join(".socket2.sock");
    p.exists().then_some(p)
}

fn hypr_request(sock1: &Path, cmd: &[u8]) -> Option<String> {
    let mut stream = UnixStream::connect(sock1).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(30))).ok()?;
    stream.write_all(cmd).ok()?;
    let mut out = String::new();
    stream.read_to_string(&mut out).ok()?;
    Some(out)
}

fn query_hypr_cursor() -> Option<(f32, f32)> {
    let sock1 = hypr_socket1()?;
    let s = hypr_request(&sock1, b"cursorpos")?;
    let (xs, ys) = s.trim().split_once(',')?;
    let x = xs.trim().parse::<f32>().ok()?;
    let y = ys.trim().parse::<f32>().ok()?;
    Some((x, y))
}

/// Hyprland's open workspaces on the focused monitor, side by side by id,
/// one monitor width plus `general:gaps_workspaces` apart. The compositor
/// reports neither swipe offsets nor animations, so `hypr_view` replays its
/// logic (UnifiedWorkspaceSwipeGesture.cpp, WorkspaceAnimationController.cpp).
struct HyprRow {
    monitor: String,
    /// open workspace ids, sorted
    ids: Vec<i64>,
    idx: usize,
    count: usize,
    monitor_w: f32,
    /// distance between neighbouring workspaces
    step: f32,
    /// animation towards `idx`: start position and time
    from: f32,
    since: std::time::Instant,
    /// swipe in progress: `dx` where the current segment began, and the
    /// last segment delta (Hyprland's `m_delta`)
    swipe: Option<(f32, f32)>,
    /// a released swipe already moved `idx`; the event that follows only confirms it
    predicted: bool,
}

static HYPR_ROW: std::sync::Mutex<Option<HyprRow>> = std::sync::Mutex::new(None);

/// Characters are drawn into one window per workspace, placed through the
/// plugin (`hypr_place_workspace`): Hyprland slides those with their
/// workspaces itself, so the strip view stays on the active workspace
/// instead of following the slide.
pub static WINDOW_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The row of workspaces the characters walk along, for window mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    pub count: usize,
    /// index of the active workspace in the row
    pub active: usize,
    /// distance between neighbouring workspaces, logical pixels
    pub step: f32,
    /// a swipe or workspace animation is moving the row right now
    pub sliding: bool,
}

pub fn hypr_row() -> Option<Row> {
    let row = HYPR_ROW.lock().unwrap();
    let r = row.as_ref()?;
    if r.ids.is_empty() {
        return None;
    }
    // the plugin's active workspace is a frame ahead of the IPC events
    let (active, sliding) = match hypr_plugin::read(&r.monitor) {
        Some((id, offset)) => (r.ids.iter().filter(|&&i| i < id).count().min(r.ids.len() - 1), offset != 0.0),
        None => (r.idx.min(r.ids.len() - 1), false),
    };
    Some(Row { count: r.ids.len(), active, step: r.step, sliding })
}

/// Keep window `slot` on the row's `slot`th workspace, at its top-left.
pub fn hypr_place_workspace(slot: usize) {
    let row = HYPR_ROW.lock().unwrap();
    let Some(r) = row.as_ref() else { return };
    if let Some(&id) = r.ids.get(slot) {
        hypr_plugin::write_window(slot, id, 0.0, 0.0, &r.monitor);
    }
}

/// Whether character windows can be placed (Hyprland with the plugin).
pub fn can_place_windows() -> bool {
    hypr_socket1().is_some() && hypr_plugin::loaded()
}


fn update_hypr_strip() {
    let Some(sock1) = hypr_socket1() else { return };
    let Some(((_, _, w, _), mon)) = hypr_output_rect() else { return };
    let json = |cmd: &[u8]| hypr_request(&sock1, cmd).and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
    let Some(active) = json(b"j/activeworkspace").and_then(|v| v["id"].as_i64()) else { return };
    let Some(list) = json(b"j/workspaces") else { return };
    // workspaces with real windows: characters alone must not keep one alive
    let busy: Vec<i64> = json(b"j/clients")
        .and_then(|c| c.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter(|c| c["class"].as_str() != Some(super::charwin::APP_ID))
        .filter_map(|c| c["workspace"]["id"].as_i64())
        .collect();
    let mut ids: Vec<i64> = list
        .as_array()
        .into_iter()
        .flatten()
        .filter(|ws| ws["monitor"].as_str() == Some(mon.as_str()))
        .filter_map(|ws| ws["id"].as_i64())
        .filter(|&id| id > 0) // special workspaces are negative
        .filter(|id| *id == active || busy.contains(id))
        .collect();
    ids.sort();
    let Some(idx) = ids.iter().position(|&id| id == active) else { return };
    let step = w + hypr_options().gaps;
    let now = std::time::Instant::now();
    let mut row = HYPR_ROW.lock().unwrap();
    let Some(r) = row.as_mut() else {
        *row = Some(HyprRow {
            monitor: mon, ids, idx, count: 0, monitor_w: w, step,
            from: idx as f32 * step, since: now, swipe: None, predicted: false,
        });
        let r = row.as_mut().unwrap();
        r.count = r.ids.len();
        return;
    };
    r.count = ids.len();
    r.ids = ids;
    r.monitor = mon;
    r.monitor_w = w;
    r.step = step;
    if r.idx == idx {
        r.predicted = false;
        return;
    }
    if std::mem::take(&mut r.predicted) || r.swipe.is_some() {
        // guessed wrong: carry on from where the row is drawn
        r.from = r.animated(now);
    } else {
        // keybind etc.: the new workspace slides in from one step away,
        // however far apart the two are
        let dir = if idx > r.idx { 1.0 } else { -1.0 };
        r.from = (idx as f32 - dir) * step;
    }
    r.since = now;
    r.idx = idx;
}

impl HyprRow {
    fn target(&self) -> f32 {
        self.idx as f32 * self.step
    }

    fn animated(&self, now: std::time::Instant) -> f32 {
        let anim = hypr_workspace_anim();
        let t = if anim.duration > 0.0 { (now - self.since).as_secs_f32() / anim.duration } else { 1.0 };
        self.from + (self.target() - self.from) * anim.ease(t.min(1.0))
    }
}

/// Where Hyprland draws the row right now: under the fingers during a
/// three-finger swipe, then its workspace animation to the target.
pub fn hypr_view() -> Option<Strip> {
    let mut row = HYPR_ROW.lock().unwrap();
    let r = row.as_mut()?;
    if let Some((active, offset)) = hypr_plugin::read(&r.monitor) {
        // exact: the active workspace's slot minus where Hyprland draws it.
        // A workspace not listed yet (just created) goes where its id sorts.
        let idx = r.ids.iter().filter(|&&id| id < active).count();
        let count = r.ids.len() + usize::from(!r.ids.contains(&active));
        let slide = if WINDOW_MODE.load(std::sync::atomic::Ordering::Relaxed) { 0.0 } else { offset };
        let view = (idx as f32 - slide) * r.step;
        let width = count.saturating_sub(1) as f32 * r.step + r.monitor_w;
        return Some(Strip { view_x: view, width, screen_w: r.monitor_w, ws: active as u64, exact: true });
    }
    let now = std::time::Instant::now();
    let opts = hypr_options();
    let swipe = *super::pointer::SWIPE.lock().unwrap();
    let last = r.count.saturating_sub(1);
    let view = match (swipe, r.swipe) {
        (Some(sw), seg) => {
            let base = seg.map_or(sw.dx, |s| s.0);
            let raw = (sw.dx - base) * if opts.invert { -1.0 } else { 1.0 };
            // no neighbour past either end: stays put
            let mut delta = raw.clamp(-opts.distance, opts.distance);
            if (delta < 0.0 && r.idx == 0) || (delta > 0.0 && r.idx >= last) {
                delta = 0.0;
            }
            let mut base = base;
            if opts.forever && delta.abs() >= opts.distance {
                // Hyprland switches right away and starts a new segment
                r.idx = if delta < 0.0 { r.idx - 1 } else { r.idx + 1 };
                r.predicted = true;
                base = sw.dx;
                delta = 0.0;
            }
            r.swipe = Some((base, delta));
            let v = r.target() + delta / opts.distance * r.step;
            r.from = v;
            r.since = now;
            v
        }
        (None, Some((_, delta))) => {
            // released: Hyprland commits past the cancel ratio or when fast enough
            r.swipe = None;
            let fast = opts.min_speed > 0.0
                && SWIPE_SPEED.lock().unwrap().is_some_and(|v| v >= opts.min_speed);
            if delta.abs() >= 2.0 && (delta.abs() >= opts.distance * opts.cancel_ratio || fast) {
                r.idx = if delta < 0.0 { r.idx.saturating_sub(1) } else { (r.idx + 1).min(last) };
                r.predicted = true;
            }
            if delta.abs() < 2.0 {
                r.from = r.target();
            }
            r.animated(now)
        }
        (None, None) => r.animated(now),
    };
    *SWIPE_SPEED.lock().unwrap() = swipe.map(|s| s.path / s.frames.max(1) as f32);
    let width = r.count.saturating_sub(1) as f32 * r.step + r.monitor_w;
    Some(Strip { view_x: view, width, screen_w: r.monitor_w, ws: r.idx as u64, exact: true })
}

/// Reader for contrib/hyprland-plugin: each monitor's active workspace and
/// the offset Hyprland draws it at, in workspace steps.
mod hypr_plugin {
    use std::sync::atomic::{AtomicU32, Ordering, fence};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    const MAGIC: u32 = 0x414e_4859;
    const SIZE: usize = 20480;
    const RECORDS: usize = 128;
    const RECORD: usize = 64;
    const MONITORS: usize = 16;

    // mapping address (0 = none) and the last attempt to map it
    static MAP: Mutex<(usize, Option<Instant>)> = Mutex::new((0, None));

    fn map() -> Option<*const u8> {
        let mut m = MAP.lock().unwrap();
        if m.0 != 0 {
            return Some(m.0 as *const u8);
        }
        if m.1.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
            return None;
        }
        m.1 = Some(Instant::now());
        let path = std::path::PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?).join("anima-hyprland.shm");
        let file = std::fs::OpenOptions::new().read(true).write(true).open(path).ok()?;
        // shorter than the mapping would SIGBUS on read
        if file.metadata().ok()?.len() < SIZE as u64 {
            return None;
        }
        use std::os::unix::io::AsRawFd;
        let p = unsafe {
            libc::mmap(std::ptr::null_mut(), SIZE, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, file.as_raw_fd(), 0)
        };
        if p == libc::MAP_FAILED {
            return None;
        }
        log::info!("hyprland plugin: reading workspace offsets");
        m.0 = p as usize;
        Some(p as *const u8)
    }

    /// Mapping of a plugin loaded in this Hyprland instance (version 4+:
    /// 256 window slots).
    fn live() -> Option<*const u8> {
        let p = map()?;
        unsafe {
            if (*(p as *const AtomicU32)).load(Ordering::Acquire) != MAGIC || *(p.add(4) as *const u32) < 4 {
                return None;
            }
            let inst = std::slice::from_raw_parts(p.add(8), 120);
            let inst = &inst[..inst.iter().position(|&b| b == 0).unwrap_or(120)];
            let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
            (inst == sig.as_bytes()).then_some(p)
        }
    }

    pub fn loaded() -> bool {
        live().is_some()
    }

    const WINDOWS: usize = 2048;
    const MAX_WINDOWS: usize = 256;

    /// Window record `actor` (see contrib/hyprland-plugin/main.cpp): where
    /// the plugin should keep our character window.
    pub fn write_window(actor: usize, workspace: i64, x: f32, y: f32, monitor: &str) {
        if actor >= MAX_WINDOWS {
            return;
        }
        let Some(p) = live() else { return };
        unsafe {
            let rec = (p as *mut u8).add(WINDOWS + actor * RECORD);
            let seq = &*(rec as *const AtomicU32);
            let s = seq.load(Ordering::Relaxed);
            seq.store(s.wrapping_add(1) | 1, Ordering::Relaxed);
            fence(Ordering::Release);
            std::ptr::write_volatile(rec.add(4) as *mut u32, std::process::id());
            std::ptr::write_volatile(rec.add(8) as *mut u32, actor as u32);
            std::ptr::write_volatile(rec.add(12) as *mut i32, workspace as i32);
            std::ptr::write_volatile(rec.add(16) as *mut f32, x);
            std::ptr::write_volatile(rec.add(20) as *mut f32, y);
            let mut name = [0u8; 40];
            let n = monitor.len().min(39);
            name[..n].copy_from_slice(&monitor.as_bytes()[..n]);
            std::ptr::copy_nonoverlapping(name.as_ptr(), rec.add(24), 40);
            seq.store((s.wrapping_add(1) | 1).wrapping_add(1), Ordering::Release);
        }
    }

    /// (active workspace id, render offset) of `monitor`, if the plugin is
    /// loaded in this Hyprland instance.
    pub fn read(monitor: &str) -> Option<(i64, f32)> {
        let p = live()?;
        unsafe {
            for i in 0..MONITORS {
                let rec = p.add(RECORDS + i * RECORD);
                let seq = &*(rec as *const AtomicU32);
                for _ in 0..64 {
                    let s1 = seq.load(Ordering::Acquire);
                    if s1 & 1 == 1 {
                        std::hint::spin_loop();
                        continue;
                    }
                    let active = std::ptr::read_volatile(rec.add(4) as *const i32);
                    let offset = std::ptr::read_volatile(rec.add(8) as *const f32);
                    let mut name = [0u8; 48];
                    std::ptr::copy_nonoverlapping(rec.add(16), name.as_mut_ptr(), 48);
                    fence(Ordering::Acquire);
                    if seq.load(Ordering::Relaxed) != s1 {
                        continue;
                    }
                    let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(48)];
                    if name.is_empty() {
                        return None; // records fill from the front
                    }
                    if name == monitor.as_bytes() {
                        return (active > 0).then_some((active as i64, offset));
                    }
                    break;
                }
            }
        }
        None
    }
}

// average per-frame travel of the swipe, kept past its release
static SWIPE_SPEED: std::sync::Mutex<Option<f32>> = std::sync::Mutex::new(None);

struct HyprOptions {
    distance: f32,
    invert: bool,
    forever: bool,
    cancel_ratio: f32,
    min_speed: f32,
    gaps: f32,
}

/// Gesture options and `general:gaps_workspaces`, read once.
fn hypr_options() -> &'static HyprOptions {
    static OPTS: std::sync::OnceLock<HyprOptions> = std::sync::OnceLock::new();
    OPTS.get_or_init(|| {
        let get = |name: &str| {
            let sock1 = hypr_socket1()?;
            let s = hypr_request(&sock1, format!("j/getoption {name}").as_bytes())?;
            let v: serde_json::Value = serde_json::from_str(&s).ok()?;
            v.get("int")
                .and_then(|i| i.as_f64())
                .or_else(|| v.get("float")?.as_f64())
                .or_else(|| v.get("bool")?.as_bool().map(|b| b as u8 as f64))
                .map(|x| x as f32)
        };
        HyprOptions {
            distance: get("gestures:workspace_swipe_distance").unwrap_or(300.0).max(1.0),
            invert: get("gestures:workspace_swipe_invert").unwrap_or(1.0) != 0.0,
            forever: get("gestures:workspace_swipe_forever").unwrap_or(0.0) != 0.0,
            cancel_ratio: get("gestures:workspace_swipe_cancel_ratio").unwrap_or(0.5),
            min_speed: get("gestures:workspace_swipe_min_speed_to_force").unwrap_or(30.0),
            gaps: get("general:gaps_workspaces").unwrap_or(0.0),
        }
    })
}

/// The `workspacesIn` animation (what the incoming workspace plays):
/// duration (speed is in 100 ms units) and bezier, inherited from
/// `workspaces`, then `global`, when not set.
struct HyprAnim {
    duration: f32,
    p: [f32; 4],
}

impl HyprAnim {
    /// Cubic bezier from (0,0) to (1,1): solve x(s) = t, return y(s).
    fn ease(&self, t: f32) -> f32 {
        let [x1, y1, x2, y2] = self.p;
        let bez = |a: f32, b: f32, s: f32| 3.0 * a * s * (1.0 - s).powi(2) + 3.0 * b * s * s * (1.0 - s) + s.powi(3);
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if bez(x1, x2, mid) < t { lo = mid } else { hi = mid }
        }
        bez(y1, y2, (lo + hi) / 2.0)
    }
}

fn hypr_workspace_anim() -> &'static HyprAnim {
    static ANIM: std::sync::OnceLock<HyprAnim> = std::sync::OnceLock::new();
    ANIM.get_or_init(|| {
        let fallback = HyprAnim { duration: 0.8, p: [0.0, 0.75, 0.15, 1.0] };
        let Some(v) = hypr_socket1()
            .and_then(|s| hypr_request(&s, b"j/animations"))
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        else {
            return fallback;
        };
        let find = |name: &str| v[0].as_array()?.iter().find(|a| a["name"].as_str() == Some(name)).cloned();
        let Some(anim) = ["workspacesIn", "workspaces", "global"]
            .iter()
            .filter_map(|n| find(n))
            .find(|a| a["overridden"].as_bool() == Some(true))
        else {
            return fallback;
        };
        if anim["enabled"].as_bool() == Some(false) {
            return HyprAnim { duration: 0.0, ..fallback };
        }
        let name = anim["bezier"].as_str().unwrap_or("default");
        let p = v[1]
            .as_array()
            .and_then(|b| b.iter().find(|b| b["name"].as_str() == Some(name)))
            .map(|b| ["X0", "Y0", "X1", "Y1"].map(|k| b[k].as_f64().unwrap_or(0.0) as f32))
            .unwrap_or(fallback.p);
        HyprAnim { duration: anim["speed"].as_f64().unwrap_or(8.0) as f32 * 0.1, p }
    })
}

/// Focused monitor: logical rect and connector name.
fn hypr_output_rect() -> Option<((f32, f32, f32, f32), String)> {
    let v: serde_json::Value = serde_json::from_str(&hypr_request(&hypr_socket1()?, b"j/monitors")?).ok()?;
    let m = v.as_array()?.iter().find(|m| m["focused"].as_bool() == Some(true))?;
    let f = |k: &str| m[k].as_f64().unwrap_or(0.0) as f32;
    let scale = f("scale").max(0.1);
    let rect = (f("x"), f("y"), f("width") / scale, f("height") / scale);
    Some((rect, m["name"].as_str()?.to_string()))
}

fn query_hypr_active_window() -> Option<(bool, bool, Option<WindowRect>)> {
    let sock1 = hypr_socket1()?;
    let json_str = hypr_request(&sock1, b"j/activewindow")?;
    let value: serde_json::Value = serde_json::from_str(&json_str).ok()?;
    let fs_mode = value.get("fullscreen").and_then(|v| v.as_i64()).unwrap_or(0);

    let at = value.get("at").and_then(|v| v.as_array());
    let size = value.get("size").and_then(|v| v.as_array());
    let rect = match (at, size) {
        (Some(at), Some(sz)) if at.len() >= 2 && sz.len() >= 2 => {
            let x = at[0].as_f64().unwrap_or(0.0) as f32;
            let y = at[1].as_f64().unwrap_or(0.0) as f32;
            let w = sz[0].as_f64().unwrap_or(0.0) as f32;
            let h = sz[1].as_f64().unwrap_or(0.0) as f32;
            if w > 0.0 && h > 0.0 {
                Some(WindowRect { x, y, w, h })
            } else {
                None
            }
        }
        _ => None,
    };

    Some((fs_mode >= 2, fs_mode == 1, rect))
}

fn query_window_state() -> Option<(bool, bool, Option<WindowRect>)> {
    if let Some(res) = query_hypr_active_window() {
        return Some(res);
    }
    if let Some(sock) = niri_socket() {
        return query_niri_window(&sock);
    }

    if std::env::var_os("SWAYSOCK").is_some() {
        let output = Command::new("swaymsg")
            .args(["-t", "get_tree"])
            .output()
            .ok()?;
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            let fs = text.contains("\"focused\": true") && text.contains("\"fullscreen_mode\": 1");
            return Some((fs, false, None));
        }
    }

    None
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

pub fn niri_gesture(active: bool) {
    if active {
        *GESTURE_START.lock().unwrap() = STRIP.lock().unwrap().map(|s| s.view_x);
    } else {
        GESTURE_ENDED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    NIRI_GESTURE.store(active, std::sync::atomic::Ordering::Relaxed);
}

/// Follow niri's touchpad swipe: the strip moves by the (accelerated) finger
/// travel times output width / 1200, like niri's view_offset_gesture_update.
pub fn niri_gesture_scroll(dx: f32) {
    if !NIRI_GESTURE.load(std::sync::atomic::Ordering::Relaxed)
        || NIRI_EXACT.load(std::sync::atomic::Ordering::Relaxed)
    {
        return;
    }
    let mut strip = STRIP.lock().unwrap();
    let Some(st) = strip.as_mut() else { return };
    st.view_x = (st.view_x + dx * st.screen_w / 1200.0 * GESTURE_SCALE).clamp(-st.screen_w * 0.2, st.width - st.screen_w * 0.8);
    if let Some(v) = NIRI_VIEWS.lock().unwrap().iter_mut().find(|v| v.0 == st.ws) {
        v.1 = st.view_x;
    }
}

pub static STRIP: std::sync::Mutex<Option<Strip>> = std::sync::Mutex::new(None);

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
    let screen_w = niri_output_rect(sock).map(|r| r.2).unwrap_or(1920.0);
    let gap = niri_gaps();
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

fn niri_socket() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("NIRI_SOCKET")?);
    p.exists().then_some(p)
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

fn query_niri_window(sock: &Path) -> Option<(bool, bool, Option<WindowRect>)> {
    let win = niri_request(sock, "\"FocusedWindow\"")?.get("FocusedWindow").cloned()?;
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

fn run_niri(tx: Sender<GovernorEvent>, sock: &Path) {
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

    log::info!("listening to niri events at {}", sock.display());
    let mut last_output = String::new();
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
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
        update_niri_strip(sock);
        if let Some(name) = niri_focused_output(sock)
            .and_then(|o| o.get("name").and_then(|n| n.as_str()).map(String::from))
        {
            if name != last_output {
                last_output = name.clone();
                if let Some(r) = niri_output_rect(sock) {
                    *bounds.lock().unwrap() = (r, name.clone());
                }
                if tx.send(GovernorEvent::FocusedMonitor(name)).is_err() {
                    return;
                }
            }
        }
        if let Some((fs, max, rect)) = query_niri_window(sock) {
            if tx
                .send(GovernorEvent::WindowState { fullscreen: fs, maximized: max, active_rect: rect })
                .is_err()
            {
                return;
            }
        }
    }
    poll_forever(tx);
}

fn niri_output_rect(sock: &Path) -> Option<(f32, f32, f32, f32)> {
    let l = niri_focused_output(sock)?.get("logical")?.clone();
    let f = |k: &str| l.get(k).and_then(|v| v.as_f64()).map(|v| v as f32);
    Some((f("x")?, f("y")?, f("width")?, f("height")?))
}
