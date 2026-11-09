//! Approximate global cursor position for compositors that don't expose it
//! (niri). Reads motion from mouse/touchpad evdev nodes, applies an estimate
//! of libinput's default acceleration and integrates it, clamped to the
//! output (hitting an edge re-syncs it). Needs read access to those nodes,
//! see `contrib/70-anima-pointer.rules`.

use std::fs::File;
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use calloop::channel::Sender;

use super::governor::GovernorEvent;

const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;
const EV_ABS: u16 = 0x03;
const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const BTN_TOUCH: u16 = 0x14a;
const BTN_LEFT: u16 = 0x110;
const BTN_TOOL_DOUBLETAP: u16 = 0x14d;
const EV_SYN: u16 = 0x00;

pub use super::pointer_state::*;

/// Focused output: logical rect (x, y, w, h) and connector name.
pub type Bounds = Arc<Mutex<((f32, f32, f32, f32), String)>>;

struct Shared {
    pos: Mutex<(f32, f32)>,
    bounds: Bounds,
    moved: Mutex<Option<Instant>>,
    last_click: Mutex<Option<Instant>>,
    // raw (unclamped, never snapped) path for gesture detection
    trail: Mutex<(f32, f32, std::collections::VecDeque<(Instant, f32, f32)>)>,
}

impl Shared {
    fn click(&self) {
        CLICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let now = Instant::now();
        let mut last = self.last_click.lock().unwrap();
        if last.is_some_and(|t| now - t < Duration::from_millis(400)) {
            log::debug!("pointer: double click at {:?}", *self.pos.lock().unwrap());
            DOUBLE_CLICK.store(true, std::sync::atomic::Ordering::Relaxed);
            *last = None;
        } else {
            *last = Some(now);
        }
    }

    fn nudge(&self, dx: f32, dy: f32) {
        let (bx, by, bw, bh) = self.bounds.lock().unwrap().0;
        let now = Instant::now();
        *self.moved.lock().unwrap() = Some(now);
        {
            let mut t = self.trail.lock().unwrap();
            t.0 += dx;
            t.1 += dy;
            let (x, y) = (t.0, t.1);
            t.2.push_back((now, x, y));
            while t.2.front().is_some_and(|p| now - p.0 > Duration::from_millis(1500)) {
                t.2.pop_front();
            }
        }
        let mut p = self.pos.lock().unwrap();
        p.0 = (p.0 + dx).clamp(bx, bx + bw.max(1.0) - 1.0);
        p.1 = (p.1 + dy).clamp(by, by + bh.max(1.0) - 1.0);
    }
}

/// `exact`: the compositor reports the real cursor (fed in through `SYNC`),
/// so there is no need to centre it first.
pub fn spawn(tx: Sender<GovernorEvent>, bounds: Bounds, exact: bool) {
    let devices = find_devices();
    if devices.is_empty() {
        log::info!("pointer: no readable mouse/touchpad (install contrib/70-anima-pointer.rules)");
        return;
    }
    let (bx, by, bw, bh) = bounds.lock().unwrap().0;
    // some compositors can be asked where the pointer is (polled below),
    // Hyprland feeds it through SYNC; otherwise estimate from evdev
    let polled = super::compositor::current().cursor().is_some();
    if polled {
        log::info!("pointer: position from {}", super::compositor::current().name());
    } else if exact {
        log::info!("pointer: position comes from the compositor");
    } else if super::warp::warp(bw / 2.0, bh / 2.0, bw, bh) {
        // start from a known spot: put the real cursor in the middle of the output
        log::info!("pointer: cursor moved to the centre to start in sync");
    } else {
        log::info!("pointer: can't move the cursor, estimate starts unsynced");
    }
    let shared = Arc::new(Shared {
        pos: Mutex::new((bx + bw / 2.0, by + bh / 2.0)),
        bounds,
        moved: Mutex::new(None),
        last_click: Mutex::new(None),
        trail: Mutex::new((0.0, 0.0, Default::default())),
    });
    // read every mouse/touchpad, including ones plugged in later
    let sh = shared.clone();
    std::thread::Builder::new()
        .name("anima-pointer-scan".into())
        .spawn(move || {
            let open: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
            loop {
                for (path, touchpad) in find_devices() {
                    if open.lock().unwrap().contains(&path) {
                        continue;
                    }
                    let Ok(file) = File::open(&path) else { continue };
                    log::info!("pointer: reading {}", path.display());
                    open.lock().unwrap().push(path.clone());
                    let (s, open) = (sh.clone(), open.clone());
                    std::thread::Builder::new()
                        .name("anima-pointer".into())
                        .spawn(move || {
                            read_device(file, touchpad, &s);
                            // unplugged: allow reopening
                            open.lock().unwrap().retain(|p| p != &path);
                        })
                        .ok();
                }
                std::thread::sleep(Duration::from_secs(3));
            }
        })
        .ok();
    // publish at ~120 Hz, only on change
    std::thread::Builder::new()
        .name("anima-pointer-pub".into())
        .spawn(move || {
            let mut last = (f32::NAN, f32::NAN);
            let mut warper = if polled || exact { None } else { super::warp::Warper::new() };
            let mut last_ask = Instant::now() - Duration::from_secs(1);
            let exact_trail: Mutex<std::collections::VecDeque<(Instant, f32, f32)>> = Mutex::new(Default::default());
            loop {
                if polled {
                    // ask often while the pointer moves, rarely when it doesn't
                    let moving = shared.moved.lock().unwrap().is_some_and(|t| t.elapsed() < Duration::from_millis(400));
                    // moving: every 8 ms when a character needs it, else every 25 ms
                    // (still plenty of points for circles); still: every 250 ms
                    let wanted = CURSOR_WANTED.load(std::sync::atomic::Ordering::Relaxed)
                        || BUTTON_DOWN.load(std::sync::atomic::Ordering::Relaxed);
                    let every = match (moving, wanted) {
                        (true, true) => 0,
                        (true, false) => 25,
                        (false, _) => 250,
                    };
                    if last_ask.elapsed() >= Duration::from_millis(every) {
                        last_ask = Instant::now();
                        if let Some(Some((x, y))) = super::compositor::current().cursor() {
                            let ((bx, by, _, _), _) = shared.bounds.lock().unwrap().clone();
                            let p = (bx + x, by + y);
                            *shared.pos.lock().unwrap() = p;
                            // gestures from real positions too, the evdev trail is only an estimate
                            let now = Instant::now();
                            let mut t = exact_trail.lock().unwrap();
                            if t.back().is_none_or(|q: &(Instant, f32, f32)| (q.1, q.2) != p) {
                                t.push_back((now, p.0, p.1));
                            }
                            while t.front().is_some_and(|q| now - q.0 > Duration::from_millis(1500)) {
                                t.pop_front();
                            }
                        }
                    }
                }
                {
                    let mut t = shared.trail.lock().unwrap();
                    let mut et = exact_trail.lock().unwrap();
                    let found = if polled { is_circle(&et) } else { is_circle(&t.2) };
                    if let Some((dx, dy, r)) = found {
                        // the circle's centre is offset from where the pointer is now
                        let p = *shared.pos.lock().unwrap();
                        *CIRCLE_AT.lock().unwrap() = Some((p.0 - dx, p.1 - dy, r));
                        log::debug!("pointer: circle gesture at {:?}", (p.0 - dx, p.1 - dy, r));
                        CIRCLE_GESTURE.store(true, std::sync::atomic::Ordering::Relaxed);
                        t.2.clear();
                        et.clear();
                    }
                }
                if let Some(p) = SYNC.lock().unwrap().take() {
                    *shared.pos.lock().unwrap() = p;
                }
                let p = *shared.pos.lock().unwrap();
                if p != last {
                    last = p;
                    if DRAGGING.load(std::sync::atomic::Ordering::Relaxed) {
                        if let Some(wp) = warper.as_mut() {
                            let ((bx, by, bw, bh), _) = shared.bounds.lock().unwrap().clone();
                            wp.warp(p.0 - bx, p.1 - by, bw, bh);
                        }
                    }
                    if tx.send(GovernorEvent::CursorPos(p.0, p.1)).is_err() {
                        return;
                    }
                }
                std::thread::sleep(Duration::from_millis(8));
            }
        })
        .ok();
}

/// Readable event nodes that report pointer motion: (path, is_touchpad).
fn find_devices() -> Vec<(PathBuf, bool)> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir("/sys/class/input") else {
        return out;
    };
    for entry in dir.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("event") {
            continue;
        }
        let caps = entry.path().join("device/capabilities");
        let rel = hex_caps(&caps.join("rel"));
        let abs = hex_caps(&caps.join("abs"));
        let key = std::fs::read_to_string(caps.join("key")).unwrap_or_default();
        let has_rel_xy = rel & 0b11 == 0b11;
        let has_abs_xy = abs & 0b11 == 0b11;
        // touchpads advertise BTN_TOOL_FINGER; tablets/joysticks are skipped
        let touchpad = !has_rel_xy && has_abs_xy && key_bit(&key, 0x145);
        if !(has_rel_xy || touchpad) {
            continue;
        }
        let dev = PathBuf::from("/dev/input").join(&name);
        if File::open(&dev).is_ok() {
            out.push((dev, touchpad));
        }
    }
    out
}

fn hex_caps(path: &std::path::Path) -> u64 {
    // last word holds the low bits
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.split_whitespace().last().map(String::from))
        .and_then(|w| u64::from_str_radix(&w, 16).ok())
        .unwrap_or(0)
}

fn key_bit(caps: &str, bit: usize) -> bool {
    let words: Vec<&str> = caps.split_whitespace().rev().collect();
    let (idx, off) = (bit / 64, bit % 64);
    words
        .get(idx)
        .and_then(|w| u64::from_str_radix(w, 16).ok())
        .is_some_and(|v| v >> off & 1 == 1)
}

/// Touchpad axis resolution in units per mm via EVIOCGABS.
fn abs_resolution(file: &File, axis: u16) -> Option<f32> {
    let mut info = [0i32; 6]; // value, min, max, fuzz, flat, resolution
    let req = 0x8018_4540u64 + axis as u64;
    let r = unsafe { libc::ioctl(file.as_raw_fd(), req as _, info.as_mut_ptr()) };
    (r >= 0 && info[5] > 0).then(|| info[5] as f32)
}

/// libinput's default ("adaptive") mouse curve at accel-speed 0, for a
/// 1000 dpi mouse. `v` is in device units per ms.
fn mouse_accel(v: f32) -> f32 {
    let f = if v < 0.07 {
        10.0 * v + 0.3
    } else if v < 0.4 {
        1.0
    } else {
        1.1 * (v - 0.4) + 1.0
    };
    f.min(2.0)
}

/// Rough match of libinput's touchpad curve; `v` in mm/s. Returns pixels
/// per mm of finger travel.
fn touchpad_accel(v: f32) -> f32 {
    // libinput normalises to 1000 dpi and slows touchpads down by ~0.3
    const BASE: f32 = 1000.0 / 25.4 * 0.2968;
    let f = if v < 7.0 { 0.3 + 0.7 * v / 7.0 } else if v < 50.0 { 1.0 } else { 1.0 + (v - 50.0) / 120.0 };
    BASE * f.min(2.8)
}

fn read_device(mut file: File, touchpad: bool, s: &Shared) {
    let res_x = abs_resolution(&file, ABS_X).unwrap_or(30.0);
    let res_y = abs_resolution(&file, ABS_Y).unwrap_or(30.0);
    let mut buf = [0u8; 24 * 64];
    let (mut last_x, mut last_y): (Option<i32>, Option<i32>) = (None, None);
    let mut multi = false;
    // three/four fingers: the compositor's workspace or strip swipe
    let mut three = false;
    let mut gx = 0.0f32;
    // motion collected until the next SYN_REPORT (one hardware frame)
    let (mut fx, mut fy) = (0.0f32, 0.0f32);
    let mut last_frame: Option<f64> = None;
    let mut speed = 0.0f32;
    // touchpad tap: short touch without much travel
    let mut touch_start: Option<Instant> = None;
    let mut travel = 0.0f32;
    loop {
        let n = match file.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        for ev in buf[..n].chunks_exact(24) {
            let sec = i64::from_ne_bytes(ev[0..8].try_into().unwrap());
            let usec = i64::from_ne_bytes(ev[8..16].try_into().unwrap());
            let kind = u16::from_ne_bytes([ev[16], ev[17]]);
            let code = u16::from_ne_bytes([ev[18], ev[19]]);
            let value = i32::from_ne_bytes([ev[20], ev[21], ev[22], ev[23]]);
            match (kind, code) {
                (EV_REL, REL_X) => fx += value as f32,
                (EV_REL, REL_Y) => fy += value as f32,
                (EV_SYN, 0) => {
                    if gx != 0.0 {
                        let mm = gx / res_x;
                        let t_ms = sec as f64 * 1000.0 + usec as f64 / 1000.0;
                        let dt = last_frame.map(|l| (t_ms - l) as f32).unwrap_or(8.0).clamp(1.0, 100.0);
                        last_frame = Some(t_ms);
                        let v = mm.abs() / dt;
                        speed = if dt > 50.0 { v } else { speed * 0.5 + v * 0.5 };
                        super::compositor::current().swipe_scroll(mm * touchpad_accel(speed * 1000.0));
                        gx = 0.0;
                    }
                    if fx == 0.0 && fy == 0.0 {
                        continue;
                    }
                    let t_ms = sec as f64 * 1000.0 + usec as f64 / 1000.0;
                    let dt = last_frame.map(|l| (t_ms - l) as f32).unwrap_or(8.0).clamp(1.0, 100.0);
                    last_frame = Some(t_ms);
                    let (dx, dy) = if touchpad {
                        // device units -> mm
                        (fx / res_x, fy / res_y)
                    } else {
                        (fx, fy)
                    };
                    let v = (dx * dx + dy * dy).sqrt() / dt;
                    // smooth like libinput's velocity trackers
                    speed = if dt > 50.0 { v } else { speed * 0.5 + v * 0.5 };
                    let f = if touchpad { touchpad_accel(speed * 1000.0) } else { mouse_accel(speed) };
                    s.nudge(dx * f, dy * f);
                    if touchpad {
                        travel += (dx.abs() + dy.abs()) * f;
                    }
                    (fx, fy) = (0.0, 0.0);
                }
                (EV_KEY, BTN_LEFT) => {
                    BUTTON_DOWN.store(value != 0, std::sync::atomic::Ordering::Relaxed);
                    if value == 1 {
                        s.click();
                    }
                }
                (EV_KEY, BTN_TOUCH) if value == 1 => {
                    touch_start = Some(Instant::now());
                    travel = 0.0;
                }
                (EV_KEY, BTN_TOUCH) if value == 0 => {
                    if touchpad
                        && !multi
                        && touch_start.is_some_and(|t| t.elapsed() < Duration::from_millis(200))
                        && travel < 8.0
                    {
                        s.click();
                    }
                    touch_start = None;
                    (last_x, last_y) = (None, None);
                    last_frame = None;
                }
                // two+ fingers = scrolling/gestures, not pointer motion
                (EV_KEY, c) if (BTN_TOOL_DOUBLETAP..=BTN_TOOL_DOUBLETAP + 3).contains(&c) || c == 0x14f => {
                    multi = value != 0;
                    // three or four fingers: the compositor's swipe
                    if c == BTN_TOOL_DOUBLETAP + 1 || c == BTN_TOOL_DOUBLETAP + 2 {
                        three = value != 0;
                        log::debug!("pointer: swipe fingers {} {}", c - BTN_TOOL_DOUBLETAP + 2, if three { "down" } else { "up" });
                        super::compositor::current().swipe(three);
                    }
                    (last_x, last_y) = (None, None);
                    last_frame = None;
                }
                (EV_ABS, ABS_X) if touchpad && three => {
                    if let Some(l) = last_x {
                        gx += (value - l) as f32;
                    }
                    last_x = Some(value);
                }
                (EV_ABS, ABS_X) if touchpad => {
                    if let (Some(l), false) = (last_x, multi) {
                        fx += (value - l) as f32;
                    }
                    last_x = Some(value);
                }
                (EV_ABS, ABS_Y) if touchpad => {
                    if let (Some(l), false) = (last_y, multi) {
                        fy += (value - l) as f32;
                    }
                    last_y = Some(value);
                }
                _ => {}
            }
        }
    }
}
