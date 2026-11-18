//! Hide and seek: one covers its eyes and counts, the others hide behind
//! the edges of windows (or the screen), peeking out now and then. Then it
//! goes looking; whoever gets back to the start before being found is safe.

use super::super::CharacterOverlay;
use super::*;
use crate::render::tuning::get as tn;

/// A place to hide: strip x of a window's (or the screen's) edge, and which
/// side of it is covered (-1 left, 1 right).
#[derive(Clone, Copy, Debug)]
struct Spot {
    edge: f32,
    side: f32,
    // a window's edge (cut off where it covers); the screen's own edge hides by itself
    window: bool,
    checked: bool,
}

/// 0 running off to hide, 1 hidden, 2 found and walking back, 3 running
/// back to get in first, 4 back at the start.
#[derive(Clone, Copy, Debug)]
struct Hider {
    i: usize,
    spot: usize,
    // second one at the same spot: right in behind, nothing showing
    deep: bool,
    state: u8,
    // to the next peek out, or how long this one lasts (s)
    peek: f32,
    peeking: bool,
    // got back in first
    safe: bool,
}

#[derive(Clone, Debug)]
pub(super) struct Hide {
    seeker: usize,
    base: f32,
    spots: Vec<Spot>,
    hiders: Vec<Hider>,
    // 0 counting, 1 looking
    phase: u8,
    t: f32,
    // the spot it's going to look behind, and how long it's been looking there
    going: Option<usize>,
    look: f32,
    // a moment's flash over someone's head: who, prop kind, seconds left
    flash: Vec<(usize, i32, f32)>,
}

impl CharacterOverlay {
    /// Where on `i`'s strip the screen is, (left, right).
    fn screen_span(&self, i: usize) -> (f32, f32) {
        let view = if i == 0 { self.own_view } else { self.extras[i - 1].own_view };
        let lo = view[0] * self.view_w * 0.5;
        (lo, lo + self.view_w)
    }

    /// Edges to hide behind on screen: windows' first, the screen's own too.
    fn hide_spots(&self, i: usize, base: f32) -> Vec<Spot> {
        let (lo, hi) = self.screen_span(i);
        let size = self.width_of(i);
        let ws = self.walk_ref(i).ws;
        let here = self.spaces.active == 0 || ws == self.spaces.active;
        let cols = if here && !super::super::locked() { crate::backend::compositor::current().columns() } else { Vec::new() };
        let mut spots = Vec::new();
        for (x, w) in cols {
            // a window's left edge hides what's to its right, and the other way round
            for (edge, side) in [(x, 1.0), (x + w, -1.0)] {
                if edge > lo + size && edge < hi - size {
                    spots.push(Spot { edge, side, window: true, checked: false });
                }
            }
        }
        spots.push(Spot { edge: lo, side: -1.0, window: false, checked: false });
        spots.push(Spot { edge: hi, side: 1.0, window: false, checked: false });
        // not right by the start, and not two on top of each other
        spots.retain(|s| (s.edge - base).abs() > size * 2.0);
        let mut kept: Vec<Spot> = Vec::new();
        for s in spots {
            if !kept.iter().any(|k| (k.edge - s.edge).abs() < size * 1.2) {
                kept.push(s);
            }
        }
        kept
    }

    /// Where a hider stands at `s`: most of it behind the edge.
    fn hide_x(s: &Spot, size: f32, deep: bool, peeking: bool) -> f32 {
        let show = if deep { -0.1 } else if peeking { 0.48 } else { 0.3 };
        // the side that shows is away from the cover
        s.edge + s.side * (size * 0.5 - show * size)
    }

    pub(super) fn start_hide(&mut self, c: &mut Chat) {
        let seeker = c.members[(self.walk.rand() * c.members.len() as f32) as usize % c.members.len()];
        let base = self.strip_px(seeker);
        let spots = self.hide_spots(seeker, base);
        let mut hiders = Vec::new();
        let mut used = vec![0u8; spots.len()];
        for &i in c.members.clone().iter().filter(|&&i| i != seeker) {
            let free: Vec<usize> = (0..spots.len()).filter(|&k| used[k] < 2).collect();
            if free.is_empty() {
                break;
            }
            let k = free[(self.walk.rand() * free.len() as f32) as usize % free.len()];
            used[k] += 1;
            hiders.push(Hider { i, spot: k, deep: used[k] == 2, state: 0, peek: 3.0 + self.walk.rand() * 4.0, peeking: false, safe: false });
        }
        // no room for the rest: they go off and do something else
        for i in c.members.clone() {
            if i != seeker && !hiders.iter().any(|h| h.i == i) {
                c.members.retain(|&m| m != i);
                self.leave(i, c.id);
            }
        }
        log::debug!("chat {}: hide and seek, {seeker} counts, {} hide, {} spots", c.id, hiders.len(), spots.len());
        for h in &hiders {
            let x = Self::hide_x(&spots[h.spot], self.width_of(h.i), h.deep, false);
            self.go(h.i, x, 0.22);
        }
        // eyes shut, back to the others
        let wk = self.walk_of(seeker);
        wk.eyes_shut = true;
        let left = !wk.left;
        if let Some(r) = wk.social.as_mut() {
            r.face_left = left;
            r.moving = false;
        }
        c.scores = c.members.iter().map(|&i| (i, 0.0)).collect();
        c.talker = seeker;
        c.step = 0;
        c.timer = tn().hide_time;
        c.hide = Some(Hide { seeker, base, spots, hiders, phase: 0, t: 0.0, going: None, look: 0.0, flash: Vec::new() });
        c.stage = Stage::Hide;
    }

    pub(super) fn run_hide(&mut self, c: &mut Chat, dt: f32) {
        let Some(mut g) = c.hide.take() else { return };
        g.hiders.retain(|h| c.members.contains(&h.i));
        if !c.members.contains(&g.seeker) || g.hiders.is_empty() {
            self.hide_over(c, &g);
            return;
        }
        for f in &mut g.flash {
            f.2 -= dt;
        }
        g.flash.retain(|f| f.2 > 0.0);
        g.t += dt;
        self.hiders_tick(c, &mut g, dt);
        if g.phase == 0 {
            // counting: till they've all hidden, give or take
            let hidden = g.hiders.iter().all(|h| h.state == 1);
            if (g.t > tn().hide_count && hidden) || g.t > tn().hide_count * 2.5 {
                log::debug!("chat {}: ready or not, here I come", c.id);
                g.phase = 1;
                g.t = 0.0;
                let wk = self.walk_of(g.seeker);
                wk.eyes_shut = false;
                wk.start_bob(tn().jump_height * 0.25);
            } else {
                self.walk_of(g.seeker).eyes_shut = true;
            }
            c.hide = Some(g);
            return;
        }
        let done = g.hiders.iter().all(|h| h.state == 4);
        if done || c.timer <= 0.0 {
            self.hide_over(c, &g);
            return;
        }
        self.seek(c, &mut g, dt);
        c.hide = Some(g);
    }

    fn hiders_tick(&mut self, c: &mut Chat, g: &mut Hide, dt: f32) {
        let seeker_x = self.strip_px(g.seeker);
        for k in 0..g.hiders.len() {
            let mut h = g.hiders[k];
            let size = self.width_of(h.i);
            let s = g.spots[h.spot];
            // picked up by hand while hiding: well, now everyone's seen it
            if !grounded(self.walk_ref(h.i)) && h.state == 1 {
                self.walk_of(h.i).hide = None;
                h.state = 2;
                self.go(h.i, g.base, 0.14);
            }
            match h.state {
                0 => {
                    if self.arrived(h.i) {
                        h.state = 1;
                        self.walk_of(h.i).hide = s.window.then_some((s.edge, s.side));
                        // looking out the open side
                        if let Some(r) = self.walk_of(h.i).social.as_mut() {
                            r.face_left = s.side > 0.0;
                        }
                    }
                }
                1 => {
                    self.walk_of(h.i).hide = s.window.then_some((s.edge, s.side));
                    // now and then a little further out to see, then back in
                    h.peek -= dt;
                    if h.peek <= 0.0 && !h.deep {
                        h.peeking = !h.peeking;
                        h.peek = if h.peeking { 0.7 + self.walk.rand() * 0.6 } else { 3.0 + self.walk.rand() * 5.0 };
                        let x = Self::hide_x(&s, size, false, h.peeking);
                        self.steer(h.i, x, 0.05);
                    }
                    if self.arrived(h.i) {
                        if let Some(r) = self.walk_of(h.i).social.as_mut() {
                            r.face_left = s.side > 0.0;
                        }
                    }
                    // the seeker's well away from the start: a dash for it
                    let x = self.strip_px(h.i);
                    let (to_base, seeker_off) = ((x - g.base).abs(), (seeker_x - g.base).abs());
                    if g.phase == 1 && seeker_off > to_base && (seeker_x - x).abs() > size * 3.0 && self.walk.rand() < dt * tn().hide_dash {
                        log::debug!("chat {}: {} makes a dash for it", c.id, h.i);
                        self.walk_of(h.i).hide = None;
                        h.state = 3;
                        let to = g.base + (self.walk.rand() - 0.5) * size;
                        self.go(h.i, to, 0.26);
                    }
                }
                2 => {
                    if self.arrived(h.i) {
                        h.state = 4;
                    }
                }
                3 => {
                    if self.arrived(h.i) {
                        // in first: safe!
                        log::debug!("chat {}: {} is home safe", c.id, h.i);
                        h.state = 4;
                        h.safe = true;
                        if let Some(sc) = c.scores.iter_mut().find(|sc| sc.0 == h.i) {
                            sc.1 += 2.0;
                        }
                        let wk = self.walk_of(h.i);
                        wk.start_jump(tn().jump_height * 0.6);
                        wk.smile = wk.smile.max(1.2);
                        g.flash.push((h.i, 15, 1.2));
                    }
                }
                _ => {
                    // back at the start: watching the rest, turned to the seeker
                    if self.arrived(h.i) {
                        self.face(h.i, g.seeker);
                    }
                }
            }
            g.hiders[k] = h;
        }
    }

    /// The seeker goes from one hiding place to the next, nearest first
    /// (more or less), and looks behind each.
    fn seek(&mut self, c: &mut Chat, g: &mut Hide, dt: f32) {
        let s_i = g.seeker;
        let size = self.width_of(s_i);
        let x = self.strip_px(s_i);
        // where to stand to look behind a spot: just out on its open side
        let look_x = |s: &Spot| s.edge - s.side * size * 0.9;
        let Some(k) = g.going else {
            let mut open: Vec<usize> = (0..g.spots.len()).filter(|&k| !g.spots[k].checked).collect();
            if open.is_empty() {
                // looked everywhere: anyone still hidden must be where it missed them
                for s in &mut g.spots {
                    s.checked = false;
                }
                open = (0..g.spots.len()).collect();
            }
            let pick = open
                .iter()
                .copied()
                .map(|k| (k, (look_x(&g.spots[k]) - x).abs() * (0.6 + self.walk.rand() * 0.8)))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|p| p.0);
            if let Some(k) = pick {
                g.going = Some(k);
                g.look = 0.0;
                self.go(s_i, look_x(&g.spots[k]), tn().hide_seek_speed);
            }
            return;
        };
        if !self.arrived(s_i) {
            // turned round to look about on the way, now and then
            let wk = self.walk_of(s_i);
            if wk.rand() < dt * 0.15 {
                wk.start_bob(tn().jump_height * 0.08);
            }
            return;
        }
        let spot = g.spots[k];
        g.look += dt;
        // peering in behind it
        if let Some(r) = self.walk_of(s_i).social.as_mut() {
            r.face_left = spot.edge < x;
        }
        if g.look < 0.9 {
            return;
        }
        g.spots[k].checked = true;
        g.going = None;
        let here: Vec<usize> = (0..g.hiders.len()).filter(|&h| g.hiders[h].spot == k && g.hiders[h].state == 1).collect();
        if here.is_empty() {
            return;
        }
        // not every look finds them: a better seeker misses less
        let miss = tn().hide_miss * self.skilled(s_i, "hide and seek", 1.6, 0.4);
        if self.walk.rand() < miss {
            log::debug!("chat {}: {s_i} looks right past them", c.id);
            return;
        }
        for h in here {
            let i = g.hiders[h].i;
            log::debug!("chat {}: {s_i} finds {i}", c.id);
            g.hiders[h].state = 2;
            let wk = self.walk_of(i);
            wk.hide = None;
            wk.start_jump(tn().jump_height * 0.45);
            wk.smile = wk.smile.max(1.0);
            let to = g.base + (self.walk.rand() - 0.5) * size * 2.0;
            self.go(i, to, 0.13);
            if let Some(sc) = c.scores.iter_mut().find(|sc| sc.0 == s_i) {
                sc.1 += 1.0;
            }
        }
        let wk = self.walk_of(s_i);
        wk.start_jump(tn().jump_height * 0.5);
        wk.smile = wk.smile.max(1.0);
        g.flash.push((s_i, 7, 0.9));
    }

    /// Time's up or everyone's in: those never found get the most out of it.
    fn hide_over(&mut self, c: &mut Chat, g: &Hide) {
        for h in &g.hiders {
            self.walk_of(h.i).hide = None;
            if h.state <= 1 {
                // never found: out it comes, pleased with itself
                if let Some(sc) = c.scores.iter_mut().find(|sc| sc.0 == h.i) {
                    sc.1 += 2.0;
                }
                self.walk_of(h.i).smile = 1.2;
            }
        }
        self.walk_of(g.seeker).eyes_shut = false;
        log::debug!("chat {}: hide and seek over, {:?}", c.id, c.scores);
        self.end_game(c);
    }

    /// The seeker's "found you!" and a star for whoever got in safe.
    pub(super) fn hide_props(&self, v: &mut Vec<(i32, [f32; 4], bool, usize)>) {
        for c in self.chats.iter().filter(|c| c.stage == Stage::Hide) {
            let Some(g) = c.hide.as_ref() else { continue };
            for &(i, kind, left) in &g.flash {
                let r = if i == 0 { self.hit_rect } else if let Some(e) = self.extras.get(i - 1) { e.hit_rect } else { continue };
                let h = (r[3] - r[1]).max(1.0);
                let size = h * 0.3;
                let x = (r[0] + r[2]) * 0.5;
                let y = r[1] - size * (0.4 + (1.0 - left).max(0.0) * 0.3);
                v.push((kind, [x - size * 0.5, y - size * 0.5, x + size * 0.5, y + size * 0.5], false, i));
            }
        }
    }

    /// Nobody stays hidden or blindfolded once out of the game.
    pub(super) fn hide_cleanup(&mut self, chats: &[Chat], n: usize) {
        let playing: Vec<usize> = chats.iter().filter(|c| c.stage == Stage::Hide).flat_map(|c| c.members.iter().copied()).collect();
        for i in 0..n {
            if playing.contains(&i) {
                continue;
            }
            let wk = self.walk_of(i);
            wk.hide = None;
            wk.eyes_shut = false;
        }
    }
}
