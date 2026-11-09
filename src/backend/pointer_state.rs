//! What the pointer is doing, as the characters read it.

use std::sync::Mutex;
use std::time::Instant;

/// Centre and radius (logical px) of the last circle drawn with the pointer.
pub static CIRCLE_AT: Mutex<Option<(f32, f32, f32)>> = Mutex::new(None);

/// Set when the user draws a circle with the pointer; the character
/// consumes it and runs after the cursor.
pub static CIRCLE_GESTURE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Set on a double click (or double tap); the character under the cursor
/// gets petted.
pub static DOUBLE_CLICK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Left button held down (for picking characters up).
pub static BUTTON_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A character is being carried; the real cursor is pinned to the estimate
/// so the two can't drift apart.
pub static DRAGGING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A character follows the cursor closely right now (carried, chasing,
/// clinging, running to a circle): ask the compositor for it every frame.
/// Otherwise a slower rate does (each ask costs the compositor a little).
pub static CURSOR_WANTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Exact cursor position reported by the compositor while the pointer is
/// over a character; the estimate snaps to it.
pub static SYNC: Mutex<Option<(f32, f32)>> = Mutex::new(None);

/// Every left click (keeps a pat going).
pub static CLICKS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Roughly a full turn around the path's centre, at a fairly even radius.
/// Returns (last point - centre, radius).
pub fn is_circle(trail: &std::collections::VecDeque<(Instant, f32, f32)>) -> Option<(f32, f32, f32)> {
    if trail.len() < 20 {
        return None;
    }
    // thin out: high-rate mice send many tiny steps
    let pts: Vec<(f32, f32)> = trail.iter().step_by((trail.len() / 60).max(1)).map(|p| (p.1, p.2)).collect();
    let n = pts.len() as f32;
    let cx = pts.iter().map(|p| p.0).sum::<f32>() / n;
    let cy = pts.iter().map(|p| p.1).sum::<f32>() / n;
    let radii: Vec<f32> = pts.iter().map(|p| ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()).collect();
    let mean = radii.iter().sum::<f32>() / n;
    if !(25.0..=700.0).contains(&mean) {
        return None;
    }
    let var = radii.iter().map(|r| (r - mean).powi(2)).sum::<f32>() / n;
    if var.sqrt() / mean > 0.4 {
        return None;
    }
    let mut turn = 0.0f32;
    let mut prev = (pts[0].1 - cy).atan2(pts[0].0 - cx);
    for p in &pts[1..] {
        let a = (p.1 - cy).atan2(p.0 - cx);
        let mut d = a - prev;
        if d > std::f32::consts::PI {
            d -= std::f32::consts::TAU;
        } else if d < -std::f32::consts::PI {
            d += std::f32::consts::TAU;
        }
        turn += d;
        prev = a;
    }
    let last = pts[pts.len() - 1];
    (turn.abs() > std::f32::consts::TAU * 0.9).then_some((last.0 - cx, last.1 - cy, mean))
}
