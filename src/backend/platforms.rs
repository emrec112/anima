//! The tops of floating windows the characters can stand on. Whatever knows
//! the windows (the Windows backend) keeps this up to date; surface pixels.

use std::sync::Mutex;

/// The visible part of one window's top edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Platform {
    pub id: isize,
    /// left edge of the whole window, to ride along when it's moved
    pub left: f32,
    pub x0: f32,
    pub x1: f32,
    pub y: f32,
}

/// A window on screen, topmost first.
#[derive(Clone, Copy, Debug)]
pub struct Win {
    pub id: isize,
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    /// floating: not maximized or fullscreen (those only hide what's below)
    pub stand: bool,
}

static PLATFORMS: Mutex<Vec<Platform>> = Mutex::new(Vec::new());

pub fn set(list: Vec<Platform>) {
    *PLATFORMS.lock().unwrap() = list;
}

pub fn snapshot() -> Vec<Platform> {
    PLATFORMS.lock().unwrap().clone()
}

/// The widest stretch of each floating window's top edge that nothing covers.
pub fn visible(windows: &[Win], width: f32) -> Vec<Platform> {
    let mut out = Vec::new();
    for (i, win) in windows.iter().enumerate() {
        if !win.stand || win.top <= 0.0 {
            continue;
        }
        let mut open = vec![(win.left.max(0.0), win.right.min(width))];
        for above in &windows[..i] {
            if above.top <= win.top && above.bottom > win.top {
                open = open.into_iter().flat_map(|s| cut(s, (above.left, above.right))).collect();
            }
        }
        let widest = open.into_iter().max_by(|a, b| (a.1 - a.0).total_cmp(&(b.1 - b.0)));
        if let Some((x0, x1)) = widest.filter(|s| s.1 > s.0) {
            out.push(Platform { id: win.id, left: win.left, x0, x1, y: win.top });
        }
    }
    out
}

/// What's left of `span` once `hole` is taken out.
fn cut(span: (f32, f32), hole: (f32, f32)) -> Vec<(f32, f32)> {
    let mut left = Vec::new();
    if hole.0 > span.0 {
        left.push((span.0, hole.0.min(span.1)));
    }
    if hole.1 < span.1 {
        left.push((hole.1.max(span.0), span.1));
    }
    left.retain(|s| s.1 > s.0);
    left
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(id: isize, left: f32, top: f32, right: f32, bottom: f32, stand: bool) -> Win {
        Win { id, left, top, right, bottom, stand }
    }

    #[test]
    fn a_window_on_top_cuts_the_edge() {
        let list = [win(1, 300.0, 100.0, 500.0, 400.0, true), win(2, 200.0, 200.0, 900.0, 600.0, true)];
        let p = visible(&list, 1920.0);
        assert_eq!(p.len(), 2);
        assert_eq!((p[0].x0, p[0].x1), (300.0, 500.0));
        // the second one's top is split in two by the first: the wider part wins
        assert_eq!((p[1].id, p[1].x0, p[1].x1), (2, 500.0, 900.0));
    }

    #[test]
    fn maximized_windows_only_hide() {
        let list = [win(1, 0.0, 0.0, 1920.0, 1040.0, false), win(2, 100.0, 100.0, 800.0, 700.0, true)];
        assert!(visible(&list, 1920.0).is_empty());
    }

    #[test]
    fn edges_stop_at_the_screen() {
        let p = visible(&[win(1, -200.0, 100.0, 300.0, 400.0, true)], 1920.0);
        assert_eq!((p[0].left, p[0].x0, p[0].x1), (-200.0, 0.0, 300.0));
    }
}
