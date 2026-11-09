//! Climbing floating windows and strolling along their top edge.

use crate::backend::platforms::Platform;
use crate::render::tuning::get as tn;
use super::CharacterOverlay;
use super::walk::{Act, WalkState};

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Stage {
    Approach,
    Up,
    Over,
    Top,
    Leave,
    // in the air on the way to another window
    Leap,
    // walking over to the one who'll throw it
    ToBuddy,
}

/// How to get from one window to another.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Way {
    Ground,
    // down off the end, or a short jump up
    Leap,
    // too high to jump: up its side
    Wall,
    // too far: someone has to throw it
    Far,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Climb {
    id: isize,
    pub(super) stage: Stage,
    // which end of the top edge: -1 left, 1 right
    side: f32,
    t: f32,
    stay: f32,
    // the window's left edge last frame
    left: f32,
    // loses its grip this far up (share of the climb)
    slip: Option<f32>,
    // the window to jump to from here
    next: Option<isize>,
    way: Way,
    // the leap: where from (x, air), how long, where to land (share of the
    // top edge), how high above the straight line, how far it turns
    from: [f32; 2],
    dur: f32,
    spot: f32,
    peak: f32,
    spin: f32,
    // the leap ends on a window's side, to climb from there
    to_wall: bool,
    // height the climb started at
    base: f32,
    // asks to be thrown there; answered by `pair_throws`
    want_throw: Option<isize>,
    throw_to: isize,
    by: Option<usize>,
    buddy_x: f32,
    // just left the thrower's hands
    fresh: bool,
}

/// Where this character is on the surface, in the units `step` uses.
#[derive(Clone, Copy)]
pub(super) struct Geo {
    pub w: f32,
    pub h: f32,
    // screen y of the feet when on the ground
    pub feet: f32,
    pub center_x: f32,
    pub half: [f32; 2],
    pub view: f32,
    pub lo: f32,
    pub hi: f32,
    // how many more may go up right now
    pub room: bool,
}

impl Geo {
    fn off(&self, px: f32) -> f32 {
        px * 2.0 / self.w - 1.0 - self.center_x + self.view
    }

    fn air_at(&self, y: f32) -> f32 {
        ((self.feet - y) * 2.0 / self.h / self.half[1]).max(0.0)
    }

    fn sprite(&self) -> (f32, f32) {
        (self.half[0] * self.w, self.half[1] * self.h)
    }
}

impl WalkState {
    /// One frame of climbing, before `step`. Returns the range to stroll in.
    pub(super) fn perch(&mut self, dt: f32, g: &Geo, windows: &[Platform]) -> (f32, f32) {
        let Some(mut c) = self.climb else {
            self.try_climb(dt, g, windows);
            return (g.lo, g.hi);
        };
        if matches!(self.act, Act::Held | Act::Fall | Act::Planted) {
            self.climb = None;
            return (g.lo, g.hi);
        }
        if c.stage == Stage::Approach && (self.race.is_some() || self.social.is_some() || self.circle) {
            self.climb = None;
            return (g.lo, g.hi);
        }
        let Some(p) = windows.iter().find(|p| p.id == c.id) else {
            self.get_off(c.side, false);
            return (g.lo, g.hi);
        };

        let (pw, _) = g.sprite();
        let edge = if c.side < 0.0 { p.x0 } else { p.x1 };
        let outside = g.off(edge + c.side * pw * 0.3);
        let inside = g.off(edge - c.side * pw * 0.45);
        let mut range = (g.lo, g.hi);
        match c.stage {
            Stage::Approach => {
                c.t += dt;
                let goal = outside.clamp(g.lo, g.hi);
                let afoot = matches!(self.act, Act::Idle | Act::Walk);
                if afoot {
                    self.act = Act::Walk;
                    self.target = goal;
                }
                if c.t > tn().climb_give_up {
                    self.climb = None;
                    return range;
                }
                if afoot && (self.x - goal).abs() < 0.012 {
                    c.stage = Stage::Up;
                    c.t = 0.0;
                    self.act = Act::Climb;
                }
            }
            Stage::Up => {
                c.t += dt;
                self.x = outside;
                self.target = outside;
                self.left = c.side > 0.0;
                self.air += tn().climb_speed * dt;
                self.rot = (c.t * 14.0).sin() * 0.09;
                let top = g.air_at(p.y);
                if c.slip.is_some_and(|s| self.air >= c.base + (top - c.base) * s) {
                    self.slip_off(c.side);
                    return (g.lo, g.hi);
                }
                if self.air >= top {
                    self.air = top;
                    self.rot = 0.0;
                    c.stage = Stage::Over;
                    c.t = 0.0;
                }
            }
            Stage::Over => {
                c.t += dt;
                let k = (c.t / 0.35).min(1.0);
                self.x = outside + (inside - outside) * k;
                self.target = self.x;
                self.air = g.air_at(p.y) + (k * std::f32::consts::PI).sin() * 0.25;
                if k >= 1.0 {
                    self.air = g.air_at(p.y);
                    self.act = Act::Idle;
                    self.timer = 0.6;
                    c.stage = Stage::Top;
                    c.stay = tn().climb_stay_min + self.rand() * tn().climb_stay_rand;
                }
            }
            Stage::Top | Stage::Leave | Stage::ToBuddy => {
                // the window was dragged: ride along
                let shift = (p.left - c.left) * 2.0 / g.w;
                self.x += shift;
                self.target += shift;
                c.buddy_x += shift;
                let inset = pw * 0.25;
                let (lo, hi) = (g.off(p.x0 + inset), g.off(p.x1 - inset));
                // something covered this spot, or the window shrank under it
                if hi <= lo || self.x < lo - 0.03 || self.x > hi + 0.03 {
                    self.get_off(c.side, false);
                    return (g.lo, g.hi);
                }
                self.x = self.x.clamp(lo, hi);
                self.target = self.target.clamp(lo, hi);
                self.air = g.air_at(p.y);
                range = (lo, hi);
                if c.stage == Stage::Top {
                    c.stay -= dt;
                    if c.stay <= 0.0 && matches!(self.act, Act::Idle | Act::Walk) {
                        c.stage = Stage::Leave;
                        c.way = Way::Ground;
                        c.next = None;
                        c.side = if self.x - lo < hi - self.x { -1.0 } else { 1.0 };
                        let other = self.next_window(c.id, g, windows).and_then(|id| windows.iter().find(|o| o.id == id));
                        if let Some(o) = other {
                            match way_to(p, o, g) {
                                Way::Far => {
                                    c.want_throw = Some(o.id);
                                    c.stage = Stage::Top;
                                    c.stay = 1000.0;
                                }
                                Way::Ground => {}
                                way => {
                                    c.way = way;
                                    c.next = Some(o.id);
                                    c.side = if g.off((o.x0 + o.x1) * 0.5) < self.x { -1.0 } else { 1.0 };
                                }
                            }
                        }
                    }
                } else if c.stage == Stage::ToBuddy {
                    c.t += dt;
                    let afoot = matches!(self.act, Act::Idle | Act::Walk);
                    if afoot {
                        self.act = Act::Walk;
                        self.target = c.buddy_x.clamp(lo, hi);
                    }
                    let to = windows.iter().find(|o| o.id == c.throw_to);
                    if to.is_none() || c.t > 8.0 {
                        c.stage = Stage::Top;
                        c.stay = 2.0;
                        c.by = None;
                    } else if let (true, true, Some(to)) = (afoot, (self.x - c.buddy_x).abs() < 0.06, to) {
                        let spot = 0.2 + self.rand() * 0.6;
                        let land = g.off(to.x0 + (to.x1 - to.x0) * spot);
                        c.spin = (land - self.x).signum() * -std::f32::consts::TAU * 2.0;
                        c.id = to.id;
                        c.left = to.left;
                        c.stage = Stage::Leap;
                        c.t = 0.0;
                        c.from = [self.x, self.air];
                        c.dur = 1.0 + (land - self.x).abs() * 0.4;
                        c.spot = spot;
                        c.peak = tn().climb_throw;
                        c.fresh = true;
                        self.left = land < self.x;
                        self.act = Act::Climb;
                        self.climb = Some(c);
                        return (g.lo, g.hi);
                    }
                } else {
                    // a window high above: walk under its side instead of the end
                    let wall = c.next.and_then(|id| windows.iter().find(|o| o.id == id)).filter(|_| c.way == Way::Wall).map(|o| {
                        let side = -c.side;
                        let edge = if side < 0.0 { o.x0 } else { o.x1 };
                        (side, g.off(edge + side * pw * 0.3))
                    });
                    let goal = match wall {
                        Some((_, x)) => x.clamp(lo, hi),
                        None if c.side < 0.0 => lo,
                        None => hi,
                    };
                    if matches!(self.act, Act::Idle | Act::Walk) {
                        self.act = Act::Walk;
                        self.target = goal;
                        if (self.x - goal).abs() < 0.012 {
                            let to = c.next.and_then(|id| windows.iter().find(|o| o.id == id));
                            match (to, wall) {
                                (Some(to), Some((side, x))) => {
                                    c.id = to.id;
                                    c.left = to.left;
                                    c.side = side;
                                    c.slip = (self.rand() < tn().climb_slip).then(|| 0.35 + self.rand() * 0.55);
                                    c.base = self.air;
                                    c.t = 0.0;
                                    c.next = None;
                                    self.act = Act::Climb;
                                    if (x - goal).abs() < 0.02 {
                                        c.stage = Stage::Up;
                                    } else {
                                        // a small gap: over to its side first
                                        c.stage = Stage::Leap;
                                        c.to_wall = true;
                                        c.from = [self.x, self.air];
                                        c.dur = 0.5;
                                        c.peak = 0.4;
                                        c.spin = 0.0;
                                    }
                                    self.left = c.side > 0.0;
                                    self.climb = Some(c);
                                }
                                (Some(to), None) => {
                                    // off the end, to the nearest spot on the other window
                                    let edge = if c.side < 0.0 { p.x0 } else { p.x1 };
                                    let px = (edge + c.side * pw * 0.6).clamp(to.x0 + inset, (to.x1 - inset).max(to.x0 + inset));
                                    let land = g.off(px);
                                    c.spot = (px - to.x0) / (to.x1 - to.x0).max(1.0);
                                    c.peak = if to.y < p.y { tn().climb_leap } else { 0.5 };
                                    c.spin = 0.0;
                                    c.dur = 0.5 + (land - self.x).abs() * 0.5;
                                    c.id = to.id;
                                    c.left = to.left;
                                    c.stage = Stage::Leap;
                                    c.t = 0.0;
                                    c.from = [self.x, self.air];
                                    self.left = land < self.x;
                                    self.act = Act::Climb;
                                    self.climb = Some(c);
                                }
                                _ => self.get_off(c.side, true),
                            }
                            return (g.lo, g.hi);
                        }
                    }
                }
            }
            Stage::Leap => {
                c.t += dt;
                let k = (c.t / c.dur).min(1.0);
                let land = if c.to_wall { outside } else { g.off(p.x0 + (p.x1 - p.x0) * c.spot) };
                let top = if c.to_wall { c.from[1] } else { g.air_at(p.y) };
                self.x = c.from[0] + (land - c.from[0]) * k;
                self.target = self.x;
                self.air = c.from[1] + (top - c.from[1]) * k + 4.0 * k * (1.0 - k) * c.peak;
                self.rot = c.spin * k;
                if k >= 1.0 && c.to_wall {
                    self.air = top;
                    self.rot = 0.0;
                    c.to_wall = false;
                    c.stage = Stage::Up;
                    c.t = 0.0;
                } else if k >= 1.0 {
                    self.air = top;
                    self.rot = 0.0;
                    c.by = None;
                    self.act = Act::Idle;
                    self.timer = 0.6;
                    c.stage = Stage::Top;
                    c.next = None;
                    c.stay = tn().climb_stay_min + self.rand() * tn().climb_stay_rand;
                }
            }
        }
        c.left = p.left;
        self.climb = Some(c);
        range
    }

    /// Now and then, set off for a window.
    fn try_climb(&mut self, dt: f32, g: &Geo, windows: &[Platform]) {
        self.climb_cool -= dt;
        if self.climb_cool > 0.0 {
            return;
        }
        self.climb_cool = tn().climb_every * (0.5 + self.rand());
        let free = matches!(self.act, Act::Idle | Act::Walk)
            && self.air <= 0.0
            && self.race.is_none()
            && self.social.is_none()
            && !self.circle;
        if !free || !g.room {
            return;
        }
        let (pw, ph) = g.sprite();
        let cx = (self.x + g.center_x - g.view + 1.0) * 0.5 * g.w;
        let ends = |p: &Platform| {
            let left = (p.x0 - pw * 0.3 > pw * 0.2).then_some((-1.0, p.x0));
            let right = (p.x1 + pw * 0.3 < g.w - pw * 0.2).then_some((1.0, p.x1));
            [left, right]
        };
        let fits: Vec<&Platform> = windows
            .iter()
            .filter(|p| p.x1 - p.x0 >= tn().climb_min_width && p.y >= ph + 8.0 && g.air_at(p.y) >= 1.5)
            .filter(|p| ends(p).iter().any(Option::is_some))
            .collect();
        if fits.is_empty() {
            return;
        }
        let p = fits[((self.rand() * fits.len() as f32) as usize).min(fits.len() - 1)];
        let side = ends(p)
            .into_iter()
            .flatten()
            .min_by(|a, b| (a.1 - cx).abs().total_cmp(&(b.1 - cx).abs()))
            .map_or(-1.0, |e| e.0);
        let slip = (self.rand() < tn().climb_slip).then(|| 0.35 + self.rand() * 0.55);
        self.climb = Some(Climb {
            id: p.id,
            stage: Stage::Approach,
            side,
            t: 0.0,
            stay: 0.0,
            left: p.left,
            slip,
            next: None,
            way: Way::Ground,
            from: [0.0, 0.0],
            dur: 1.0,
            spot: 0.5,
            peak: 0.0,
            spin: 0.0,
            to_wall: false,
            base: 0.0,
            want_throw: None,
            throw_to: 0,
            by: None,
            buddy_x: 0.0,
            fresh: false,
        });
    }

    /// Another window to jump to, if the dice say so and there is one.
    fn next_window(&mut self, here: isize, g: &Geo, windows: &[Platform]) -> Option<isize> {
        let (_, ph) = g.sprite();
        let others: Vec<isize> = windows
            .iter()
            .filter(|p| p.id != here && p.x1 - p.x0 >= tn().climb_min_width && p.y >= ph + 8.0 && g.air_at(p.y) >= 1.5)
            .map(|p| p.id)
            .collect();
        if others.is_empty() || self.rand() >= tn().climb_hop {
            return None;
        }
        Some(others[((self.rand() * others.len() as f32) as usize).min(others.len() - 1)])
    }

    /// Lost its grip: flung off the wall, spinning, and stuck in the ground
    /// when it lands, like one thrown from high up.
    fn slip_off(&mut self, side: f32) {
        self.climb = None;
        self.act = Act::Fall;
        self.slipped = true;
        self.vx = side * 0.12;
        self.vy = 0.6;
        self.tumble = side * (8.0 + self.rand() * 6.0);
    }

    /// Off the window: a jump when it means to, a plain drop when the window
    /// went away from under it.
    fn get_off(&mut self, side: f32, leap: bool) {
        self.climb = None;
        self.rot = 0.0;
        if self.air > 0.0 {
            self.act = Act::Fall;
            self.leaped = leap;
            self.vx = if leap { side * 0.25 } else { 0.0 };
            self.vy = if leap { 1.5 } else { 0.0 };
        } else {
            self.act = Act::Idle;
            self.timer = 0.5;
        }
    }
}

fn way_to(from: &Platform, to: &Platform, g: &Geo) -> Way {
    let (pw, ph) = g.sprite();
    let gap = (to.x0 - from.x1).max(from.x0 - to.x1).max(0.0);
    if gap > tn().climb_reach * pw {
        return Way::Far;
    }
    let rise = from.y - to.y;
    if rise <= tn().climb_jump_up * ph {
        return Way::Leap;
    }
    // climbing needs a side to climb that's on the screen
    let toward_right = (to.x0 + to.x1) * 0.5 > (from.x0 + from.x1) * 0.5;
    let fits = if toward_right { to.x0 - pw * 0.3 > pw * 0.2 } else { to.x1 + pw * 0.3 < g.w - pw * 0.2 };
    if fits { Way::Wall } else { Way::Ground }
}

impl CharacterOverlay {
    /// Those asking to be thrown to a far window get someone standing near
    /// them on the same one; with nobody around they jump down instead.
    pub(super) fn pair_throws(&mut self) {
        let n = self.extras.len() + 1;
        for i in 0..n {
            let Some(c) = self.walk_ref(i).climb else { continue };
            if c.fresh {
                self.walk_of(i).climb.as_mut().unwrap().fresh = false;
                if let Some(by) = c.by.filter(|&by| by < n) {
                    let thrower = self.walk_of(by);
                    if matches!(thrower.act, Act::Idle | Act::Walk) {
                        thrower.start_jump(tn().jump_height * 0.6);
                    }
                }
            }
            let Some(to) = c.want_throw else { continue };
            let x = self.walk_ref(i).x;
            let buddy = (0..n).filter(|&j| j != i).find(|&j| {
                let w = self.walk_ref(j);
                matches!(w.act, Act::Idle | Act::Walk)
                    && (w.x - x).abs() <= 0.5
                    && w.climb.is_some_and(|o| o.id == c.id && o.stage == Stage::Top && o.want_throw.is_none())
            });
            let buddy_x = buddy.map(|j| self.walk_ref(j).x);
            let cl = self.walk_of(i).climb.as_mut().unwrap();
            cl.want_throw = None;
            match (buddy, buddy_x) {
                (Some(j), Some(bx)) => {
                    cl.stage = Stage::ToBuddy;
                    cl.t = 0.0;
                    cl.buddy_x = bx;
                    cl.throw_to = to;
                    cl.by = Some(j);
                    let t = self.walk_of(j);
                    t.act = Act::Idle;
                    t.timer = 6.0;
                    t.left = x < t.x;
                    t.climb.as_mut().unwrap().stay += 6.0;
                }
                _ => {
                    cl.stage = Stage::Leave;
                    cl.way = Way::Ground;
                    cl.next = None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn geo() -> Geo {
        Geo { w: 1920.0, h: 1040.0, feet: 1040.0, center_x: 0.0, half: [0.1, 0.15], view: 0.0, lo: -0.9, hi: 0.9, room: true }
    }

    fn window(left: f32, right: f32, y: f32) -> Platform {
        Platform { id: 7, left, x0: left, x1: right, y }
    }

    fn walker() -> WalkState {
        WalkState { act: Act::Idle, timer: 100.0, seed: 3, team: 4.0, ..Default::default() }
    }

    fn run(wk: &mut WalkState, g: &Geo, list: &[Platform], until: impl Fn(&WalkState) -> bool) -> bool {
        run_with(wk, g, list, false, until)
    }

    fn run_with(wk: &mut WalkState, g: &Geo, list: &[Platform], keep_slip: bool, until: impl Fn(&WalkState) -> bool) -> bool {
        for _ in 0..60 * 120 {
            // tests that want a slip set it themselves
            if let Some(c) = wk.climb.as_mut().filter(|_| !keep_slip) {
                c.slip = None;
            }
            let (lo, hi) = wk.perch(DT, g, list);
            wk.step(DT, lo, hi, None, 0.0);
            if until(wk) {
                return true;
            }
        }
        false
    }

    fn on_top(wk: &WalkState) -> bool {
        wk.climb.is_some_and(|c| c.stage == Stage::Top)
    }

    #[test]
    fn climbs_and_stands_on_the_edge() {
        let g = geo();
        let list = [window(600.0, 1200.0, 300.0)];
        let mut wk = walker();
        assert!(run(&mut wk, &g, &list, on_top), "never got on top");
        let top = g.air_at(300.0);
        assert!((wk.air - top).abs() < 1e-3, "air {} want {top}", wk.air);
        let px = (wk.x + 1.0) * 0.5 * g.w;
        assert!((600.0..=1200.0).contains(&px), "at {px}");
        assert_eq!(wk.rot, 0.0);
    }

    #[test]
    fn stays_inside_and_rides_a_moving_window() {
        let g = geo();
        let mut list = [window(600.0, 1200.0, 300.0)];
        let mut wk = walker();
        assert!(run(&mut wk, &g, &list, on_top));
        wk.climb.as_mut().unwrap().stay = 1000.0;
        for _ in 0..60 * 20 {
            let (lo, hi) = wk.perch(DT, &g, &list);
            wk.step(DT, lo, hi, None, 0.0);
            let px = (wk.x + 1.0) * 0.5 * g.w;
            assert!((600.0..=1200.0).contains(&px), "walked off to {px}");
        }
        wk.act = Act::Idle;
        wk.timer = 100.0;
        let before = wk.x;
        list[0].left += 192.0;
        list[0].x0 += 192.0;
        list[0].x1 += 192.0;
        wk.perch(DT, &g, &list);
        assert!((wk.x - before - 0.2).abs() < 1e-3, "moved {}", wk.x - before);
    }

    #[test]
    fn falls_when_the_window_goes_away() {
        let g = geo();
        let list = [window(600.0, 1200.0, 300.0)];
        let mut wk = walker();
        assert!(run(&mut wk, &g, &list, on_top));
        wk.perch(DT, &g, &[]);
        assert_eq!(wk.act, Act::Fall);
        assert!(wk.climb.is_none() && !wk.leaped);
    }

    #[test]
    fn jumps_down_when_bored() {
        let g = geo();
        let list = [window(600.0, 1200.0, 300.0)];
        let mut wk = walker();
        assert!(run(&mut wk, &g, &list, on_top));
        wk.climb.as_mut().unwrap().stay = 0.5;
        assert!(run(&mut wk, &g, &list, |w| w.act == Act::Fall), "never left");
        assert!(wk.leaped && wk.climb.is_none());
    }

    #[test]
    fn skips_windows_it_cant_stand_on() {
        let g = geo();
        // too narrow, too close to the top of the screen, right on the floor
        let list = [window(600.0, 640.0, 300.0), window(600.0, 1200.0, 20.0), window(600.0, 1200.0, 1000.0)];
        let mut wk = walker();
        assert!(!run(&mut wk, &g, &list, |w| w.climb.is_some()));
    }

    #[test]
    fn some_lose_their_grip_and_get_stuck() {
        let g = geo();
        let list = [window(600.0, 1200.0, 300.0)];
        let mut wk = walker();
        assert!(run(&mut wk, &g, &list, |w| w.climb.is_some()));
        wk.climb.as_mut().unwrap().slip = Some(0.5);
        assert!(run_with(&mut wk, &g, &list, true, |w| w.act == Act::Fall), "never slipped");
        assert!(wk.slipped && wk.climb.is_none() && wk.tumble != 0.0);
        assert!(wk.air < g.air_at(300.0) * 0.6, "slipped at {}", wk.air);
    }

    #[test]
    fn jumps_across_to_another_window() {
        let g = geo();
        let a = window(200.0, 800.0, 300.0);
        let b = Platform { id: 9, left: 1100.0, x0: 1100.0, x1: 1700.0, y: 500.0 };
        let mut wk = walker();
        assert!(run(&mut wk, &g, &[a], on_top));
        let c = wk.climb.as_mut().unwrap();
        c.stage = Stage::Leave;
        c.next = Some(9);
        c.way = Way::Leap;
        c.side = 1.0;
        assert!(run(&mut wk, &g, &[a, b], |w| w.climb.is_some_and(|c| c.stage == Stage::Top && c.id == 9)), "never landed");
        assert!((wk.air - g.air_at(500.0)).abs() < 1e-3);
        let px = (wk.x + 1.0) * 0.5 * g.w;
        assert!((1100.0..=1700.0).contains(&px), "landed at {px}");
        assert_eq!(wk.act, Act::Idle);
    }

    fn other(left: f32, right: f32, y: f32) -> Platform {
        Platform { id: 9, left, x0: left, x1: right, y }
    }

    #[test]
    fn picks_the_way_by_height_and_distance() {
        let g = geo();
        let a = window(600.0, 1000.0, 500.0);
        assert_eq!(way_to(&a, &other(1100.0, 1500.0, 800.0), &g), Way::Leap, "below");
        assert_eq!(way_to(&a, &other(1100.0, 1500.0, 400.0), &g), Way::Leap, "a little higher");
        assert_eq!(way_to(&a, &other(1100.0, 1500.0, 100.0), &g), Way::Wall, "far above");
        assert_eq!(way_to(&a, &other(1700.0, 1900.0, 500.0), &g), Way::Far, "too far");
    }

    #[test]
    fn climbs_up_to_a_window_high_above() {
        let g = geo();
        let a = window(200.0, 800.0, 600.0);
        let b = other(700.0, 1400.0, 200.0);
        let mut wk = walker();
        assert!(run(&mut wk, &g, &[a], on_top));
        let c = wk.climb.as_mut().unwrap();
        c.stage = Stage::Leave;
        c.next = Some(9);
        c.way = Way::Wall;
        c.side = 1.0;
        let mut seen_up = false;
        for _ in 0..60 * 60 {
            let (lo, hi) = wk.perch(DT, &g, &[a, b]);
            wk.step(DT, lo, hi, None, 0.0);
            if let Some(c) = wk.climb.as_mut() {
                c.slip = None;
                seen_up |= c.stage == Stage::Up;
                if c.stage == Stage::Top && c.id == 9 {
                    break;
                }
            }
        }
        assert!(seen_up, "should go up the side, not fly");
        assert!(wk.climb.is_some_and(|c| c.id == 9 && c.stage == Stage::Top), "never got there");
        assert!((wk.air - g.air_at(200.0)).abs() < 1e-3);
    }

    #[test]
    fn thrown_to_a_far_window() {
        let g = geo();
        let a = window(100.0, 700.0, 300.0);
        let b = other(1500.0, 1900.0, 500.0);
        let mut wk = walker();
        assert!(run(&mut wk, &g, &[a], on_top));
        let x = wk.x;
        let c = wk.climb.as_mut().unwrap();
        c.stage = Stage::ToBuddy;
        c.buddy_x = x;
        c.throw_to = 9;
        c.t = 0.0;
        assert!(run(&mut wk, &g, &[a, b], |w| w.climb.is_some_and(|c| c.stage == Stage::Top && c.id == 9)), "never landed");
        assert_eq!(wk.rot, 0.0);
        assert!((wk.air - g.air_at(500.0)).abs() < 1e-3);
    }

    #[test]
    fn being_picked_up_ends_it() {
        let g = geo();
        let list = [window(600.0, 1200.0, 300.0)];
        let mut wk = walker();
        assert!(run(&mut wk, &g, &list, on_top));
        wk.act = Act::Held;
        wk.perch(DT, &g, &list);
        assert!(wk.climb.is_none());
    }
}
