//! Coming over to watch a game: from beside the row, for a while, and off
//! again when it's had enough or the game's over.

use super::super::CharacterOverlay;
use super::*;
use crate::render::tuning::get as tn;

impl CharacterOverlay {
    /// Watchers lose interest one by one, games or not.
    pub(super) fn watch_tick(&mut self, c: &mut Chat, dt: f32) {
        c.watch_left.retain(|w| c.watchers.contains(&w.0));
        let mut bored = Vec::new();
        for w in &mut c.watch_left {
            w.1 -= dt;
            if w.1 <= 0.0 {
                bored.push(w.0);
            }
        }
        for i in bored {
            log::debug!("chat {}: {i} has watched enough", c.id);
            c.watchers.retain(|&x| x != i);
            c.watch_left.retain(|w| w.0 != i);
            self.leave(i, c.id);
        }
    }

    /// Someone around sees a game going on: they come and stand by to watch,
    /// and join when it's over.
    pub(super) fn try_watch(&mut self, c: &mut Chat, n: usize) {
        let fishing = c.stage == Stage::Fishing;
        // not before the anglers are in their places; and nobody stands about
        // watching them in the park, cafe or farm (they've things to do there)
        if fishing && (c.step == 9 || c.members.iter().any(|&m| self.place_here(m).is_some())) {
            // any already watching there go off about their business
            for w in std::mem::take(&mut c.watchers) {
                self.leave(w, c.id);
            }
            return;
        }
        // something happened (a point, someone sent flying): the watchers react
        let stir = self.game_stir(c);
        let event = !c.watch_stir.is_nan() && (stir - c.watch_stir).abs() > 1e-3;
        c.watch_stir = stir;
        let focus = self.game_focus(c);
        for &w in &c.watchers.clone() {
            if fishing || Some(w) == c.cheater || !self.arrived(w) {
                continue;
            }
            // eyes on where it's happening (wrestling sees to its own)
            if c.stage != Stage::Wrestle {
                let left = focus < self.strip_px(w);
                if let Some(r) = self.walk_of(w).social.as_mut() {
                    r.face_left = left;
                }
            }
            let wk = self.walk_of(w);
            if event && wk.rand() < 0.7 {
                wk.smile = wk.smile.max(1.0 + wk.rand() * 0.6);
                wk.start_bob(tn().jump_height * 0.2);
            } else if wk.rand() < 0.03 {
                // now and then caught up in it anyway
                if wk.rand() < 0.4 {
                    wk.smile = wk.smile.max(0.8);
                }
                wk.start_bob(tn().jump_height * 0.1);
            }
        }
        // nobody new once the group would be too big anyway
        if c.members.len() + c.watchers.len() >= tn().max_group as usize {
            return;
        }
        let spots: Vec<f32> = c.members.iter().map(|&i| self.strip_px(i)).collect();
        let size = self.width_of(c.members[0]);
        let free: Vec<(usize, f32)> = (0..n)
            .filter(|&i| {
                self.walk_ref(i).ws == self.walk_ref(c.members[0]).ws && self.free_for(i, Want::Watch)
            })
            .map(|i| {
                let x = self.strip_px(i);
                (i, spots.iter().map(|p| (p - x).abs()).fold(f32::MAX, f32::min))
            })
            .filter(|f| f.1 < size * tn().see_range)
            .collect();
        let Some(&(i, _)) = free.iter().min_by(|a, b| a.1.total_cmp(&b.1)) else { return };
        if self.walk.rand() >= tn().see_chance {
            return;
        }
        self.spend(i, tn().energy_watch_cost);
        self.enlist(i, c.id);
        c.watchers.push(i);
        let stay = tn().watch_stay_min + self.walk.rand() * tn().watch_stay_rand;
        c.watch_left.push((i, stay));
        log::debug!("chat {}: {i} comes to watch", c.id);
        // beside the row, not in the way
        let lo = c.members.iter().map(|&m| self.goal_of(m)).fold(f32::MAX, f32::min);
        let hi = c.members.iter().map(|&m| self.goal_of(m)).fold(f32::MIN, f32::max);
        let step = size * 0.85 * c.watchers.len() as f32;
        let goal = if self.strip_px(i) < (lo + hi) * 0.5 { lo - step } else { hi + step };
        let goal = goal.clamp(0.0, self.strip_width_of(i));
        self.go(i, goal, 0.18);
        self.match_park_y(i, &c.members);
    }
    /// Where the game's going on: the ball, whoever's "it", else the middle of the players.
    fn game_focus(&self, c: &Chat) -> f32 {
        if let Some(v) = c.volley.as_ref() {
            return self.strip_px(v.ball);
        }
        if c.stage == Stage::Tag && c.members.contains(&c.it) {
            return self.strip_px(c.it);
        }
        c.members.iter().map(|&i| self.strip_px(i)).sum::<f32>() / c.members.len().max(1) as f32
    }

    /// A number that changes whenever something worth a cheer happens.
    fn game_stir(&self, c: &Chat) -> f32 {
        let scores: f32 = c.scores.iter().map(|s| s.1).sum();
        let points = c.volley.as_ref().map_or(0.0, |v| (v.score[0] + v.score[1]) as f32);
        let flying = c.members.iter().filter(|&&i| self.walk_ref(i).social.is_some_and(|s| s.flying)).count() as f32;
        scores + points * 10.0 + flying * 100.0
    }

    /// Game over: those watching cheer it, a hop and a smile each.
    pub(super) fn watchers_cheer(&mut self, c: &Chat) {
        for &w in &c.watchers {
            if Some(w) == c.cheater {
                continue;
            }
            let wk = self.walk_of(w);
            wk.smile = wk.smile.max(1.2 + wk.rand() * 0.6);
            wk.start_bob(tn().jump_height * 0.3);
        }
    }

    /// The game's over: some of those watching come in, the rest move on.
    pub(super) fn take_in_watchers(&mut self, c: &mut Chat) {
        for i in std::mem::take(&mut c.watchers) {
            if self.walk.rand() < tn().watch_join {
                log::debug!("chat {}: {i} joins after the game", c.id);
                c.members.push(i);
                c.regroup = true;
                c.quiet = 0.0;
            } else {
                self.leave(i, c.id);
            }
        }
    }
    /// Those watching the anglers: stroll about the bank, stop to look, or
    /// have a word with someone next to them.
    pub(super) fn fish_watchers(&mut self, c: &Chat, f: &mut Fishing, dt: f32) {
        f.idle.retain(|d| c.watchers.contains(&d.0));
        for &w in &c.watchers {
            if !f.idle.iter().any(|d| d.0 == w) {
                f.idle.push((w, 1.0 + self.walk.rand() * 2.0, None));
            }
        }
        if f.beast.is_some() || f.anglers.is_empty() {
            return;
        }
        let xs: Vec<f32> = f.anglers.iter().map(|a| a.at).collect();
        let (lo, hi) = (xs.iter().copied().fold(f32::MAX, f32::min), xs.iter().copied().fold(f32::MIN, f32::max));
        for k in 0..f.idle.len() {
            let (w, mut t, mut with) = f.idle[k];
            if !self.arrived(w) {
                continue;
            }
            t -= dt;
            // the one it was talking to has gone off: the talk's over
            if with.is_some_and(|o| !c.watchers.contains(&o) && !f.anglers.iter().any(|a| a.i == o)) {
                with = None;
                t = t.min(0.5);
            }
            if let Some(o) = with {
                // talking: faces the other one and nods along now and then
                self.face(w, o);
                if self.walk.rand() < dt * 0.2 {
                    self.walk_of(w).start_bob(tn().jump_height * 0.07);
                }
                if self.walk.rand() < dt * 0.25 {
                    self.walk_of(w).gesture = true;
                }
            }
            if t > 0.0 {
                f.idle[k] = (w, t, with);
                continue;
            }
            if with.take().is_some() {
                if let Some(r) = self.walk_of(w).social.as_mut() {
                    r.happy = false;
                }
            }
            let size = self.width_of(w);
            let width = self.strip_width_of(w);
            let x = self.strip_px(w);
            let roll = self.walk.rand();
            // someone near to talk to: another watcher or an angler waiting for a bite
            let near = c.watchers.iter().copied().filter(|&o| o != w && self.arrived(o))
                .chain(f.anglers.iter().filter(|a| a.phase == 2 && !a.blown).map(|a| a.i))
                .filter(|&o| (self.strip_px(o) - x).abs() < size * 2.5)
                .min_by(|&a, &b| (self.strip_px(a) - x).abs().total_cmp(&(self.strip_px(b) - x).abs()));
            if roll < 0.35 && near.is_some() {
                let o = near.unwrap_or(w);
                with = Some(o);
                self.face(w, o);
                let wk = self.walk_of(w);
                if let Some(r) = wk.social.as_mut() {
                    r.happy = true;
                }
                wk.gesture = true;
                t = 2.5 + self.walk.rand() * 2.5;
            } else if roll < 0.8 {
                // a stroll along the bank, never right on top of an angler
                let (a, b) = ((lo - size * 3.0).max(size), (hi + size * 3.0).min((width - size).max(size)));
                let (a, b) = if a < b { (a, b) } else { (lo, hi) };
                let mut goal = x;
                for _ in 0..6 {
                    let g = a + self.walk.rand() * (b - a).max(1.0);
                    if xs.iter().all(|&ax| (ax - g).abs() > size * 0.8) {
                        goal = g;
                        break;
                    }
                }
                let pace = 0.1 + self.walk.rand() * 0.05;
                self.go(w, goal, pace);
                t = 1.5 + self.walk.rand() * 2.0;
            } else {
                // stops to look at the nearest rod
                if let Some(a) = f.anglers.iter().min_by(|p, q| (self.strip_px(p.i) - x).abs().total_cmp(&(self.strip_px(q.i) - x).abs())) {
                    let i = a.i;
                    self.face(w, i);
                }
                t = 2.0 + self.walk.rand() * 2.0;
            }
            f.idle[k] = (w, t, with);
        }
    }
}
