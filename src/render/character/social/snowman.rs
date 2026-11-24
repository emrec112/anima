//! Building a snowman while it snows: one on its own is enough, a friend or
//! two may come and help. A big ball rolled up for the bottom, a smaller one
//! lifted on top of it, a small one for the head. Passers-by may put on it
//! something of theirs they aren't wearing.

use super::super::wear::Anchor;
use super::super::CharacterOverlay;
use super::snow::{Snowman, SNOWBALL, SNOWMEN};
use super::*;
use crate::render::tuning::get as tn;

/// phase 0: getting behind the ball, 1: rolling it, 2: carrying it over,
/// 3: putting it on, 4: done, admiring it.
#[derive(Clone, Debug)]
pub(super) struct Build {
    // strip x of the snowman, once its bottom is down; the floor's screen y
    base: f32,
    ground: f32,
    // width of the bottom ball
    big: f32,
    // 0 the bottom, 1 the middle, 2 the head, 3 done
    part: usize,
    // the ball being rolled: strip x and width
    ball: (f32, f32),
    dir: f32,
    // where this ball's rolled about
    from: f32,
    phase: u8,
    t: f32,
    carrier: usize,
    ws: u64,
}

/// Putting something on a snowman: which one (strip x), and what.
#[derive(Clone, Copy, Debug)]
pub(super) struct Gift {
    x: f32,
    ws: u64,
    item: (usize, Anchor),
    t: f32,
}

impl CharacterOverlay {
    /// Snowmen standing on a workspace, and being built there.
    fn snowmen_on(chats: &[Chat], ws: u64) -> usize {
        let now = std::time::Instant::now();
        let standing = SNOWMEN.lock().unwrap().iter().filter(|m| m.ws == ws && m.until > now).count();
        standing + chats.iter().filter(|c| c.build.as_ref().is_some_and(|b| b.ws == ws)).count()
    }

    /// It's snowing: someone with nothing to do starts on a snowman.
    pub(super) fn snowman_spark(&mut self, chats: &[Chat], n: usize, every: f32) -> Option<Chat> {
        if !super::super::weather::now().snow || self.social_age < tn().warm_small || self.moods.fretting {
            return None;
        }
        if self.walk.rand() >= every / tn().snowman_every.max(1.0) {
            return None;
        }
        let ready: Vec<usize> = (0..n).filter(|&i| self.snow_free(i) && self.on_screen(i)).collect();
        let i = *ready.get((self.walk.rand() * ready.len() as f32) as usize % ready.len().max(1))?;
        let ws = self.walk_ref(i).ws;
        if Self::snowmen_on(chats, ws) >= tn().snowman_most as usize {
            return None;
        }
        Some(self.snowman_start(i))
    }

    /// Asked for by hand: whoever is free nearest the middle of the screen
    /// starts one, snow or not.
    pub(super) fn snowman_now(&mut self, chats: &[Chat], n: usize) -> Option<Chat> {
        let middle = self.view[0] * self.view_w * 0.5 + self.view_w * 0.5;
        let i = (0..n)
            .filter(|&i| self.snow_free(i) && self.on_screen(i))
            .min_by(|&a, &b| (self.strip_px(a) - middle).abs().total_cmp(&(self.strip_px(b) - middle).abs()))?;
        let ws = self.walk_ref(i).ws;
        if Self::snowmen_on(chats, ws) >= tn().snowman_most as usize {
            return None;
        }
        Some(self.snowman_start(i))
    }

    fn snowman_start(&mut self, i: usize) -> Chat {
        self.chat_id = self.chat_id.wrapping_add(1);
        let id = self.chat_id;
        self.enlist(i, id);
        let ws = self.walk_ref(i).ws;
        let height = self.height_of(i);
        let x = self.strip_px(i);
        let dir = if self.walk.rand() < 0.5 { -1.0 } else { 1.0 };
        log::debug!("chat {id}: {i} starts on a snowman");
        let mut c = Chat::new(id, vec![i], Stage::Snowman, 0.0);
        let ground = self.body_rect(i)[3];
        c.build = Some(Build { base: x, ground, big: height * tn().snowman_size, part: 0, ball: (x + dir * self.width_of(i), height * 0.15), dir, from: x, phase: 0, t: 0.0, carrier: i, ws });
        c.timer = 240.0;
        c
    }

    /// Someone free nearby comes to help with it (three at most).
    pub(super) fn snowman_join(&mut self, c: &mut Chat, n: usize, every: f32) {
        let Some(b) = c.build.as_ref() else { return };
        if b.part >= 3 || b.phase >= 2 || c.members.len() >= 3 {
            return;
        }
        let ws = b.ws;
        let x = b.ball.0;
        let Some(j) = (0..n).find(|&j| !c.members.contains(&j) && self.walk_ref(j).ws == ws && self.snow_free(j) && (self.strip_px(j) - x).abs() < self.view_w * 0.4) else { return };
        let keen = 1.0 + c.members.iter().map(|&m| self.bond(j, m).0).fold(0.0, f32::max);
        if self.walk_of(j).rand() >= every * tn().snowman_help * keen {
            return;
        }
        log::debug!("chat {}: {j} comes to help with the snowman", c.id);
        self.enlist(j, c.id);
        c.members.push(j);
        if let Some(b) = c.build.as_mut() {
            b.phase = 0;
            b.t = 0.0;
        }
    }

    /// Someone passing a snowman with something spare puts it on it.
    pub(super) fn snowman_dress(&mut self, n: usize, every: f32) -> Option<Chat> {
        if self.walk.rand() >= every / tn().snowman_dress_every.max(1.0) {
            return None;
        }
        let now = std::time::Instant::now();
        let bare: Vec<(f32, u64, f32)> = SNOWMEN.lock().unwrap().iter().filter(|m| m.until > now && m.deco.is_none()).map(|m| (m.x, m.ws, m.size)).collect();
        let &(x, ws, size) = bare.get((self.walk.rand() * bare.len() as f32) as usize % bare.len().max(1))?;
        let near: Vec<usize> = (0..n).filter(|&i| self.walk_ref(i).ws == ws && self.free_for(i, Want::Join) && (self.strip_px(i) - x).abs() < self.view_w * 0.35).collect();
        let i = *near.get((self.walk.rand() * near.len() as f32) as usize % near.len().max(1))?;
        let name = self.character_name(i);
        let item = super::super::wear::spare(&name, self.walk.rand())?;
        self.chat_id = self.chat_id.wrapping_add(1);
        let id = self.chat_id;
        self.enlist(i, id);
        let side = if self.strip_px(i) < x { -1.0 } else { 1.0 };
        let at = x + side * (size * 0.5 + self.width_of(i) * 0.4);
        self.go(i, at, 0.14);
        log::debug!("chat {id}: {i} goes to put something on a snowman");
        let mut c = Chat::new(id, vec![i], Stage::Snowman, 0.0);
        c.gift_on = Some(Gift { x, ws, item, t: 0.0 });
        c.timer = 30.0;
        Some(c)
    }

    pub(super) fn run_snowman(&mut self, c: &mut Chat, dt: f32) {
        if c.members.is_empty() || c.timer <= 0.0 {
            self.snowman_over(c);
            return;
        }
        if let Some(mut g) = c.gift_on.take() {
            let i = c.members[0];
            if self.arrived(i) {
                self.face_x(i, g.x);
                if g.t == 0.0 {
                    self.walk_of(i).gesture = true;
                    self.walk_of(i).start_bob(tn().jump_height * 0.2);
                }
                g.t += dt;
                if g.t > 0.6 {
                    let now = std::time::Instant::now();
                    if let Some(m) = SNOWMEN.lock().unwrap().iter_mut().find(|m| m.ws == g.ws && (m.x - g.x).abs() < 1.0 && m.until > now) {
                        m.deco = Some(g.item);
                    }
                    log::debug!("chat {}: {i} dresses the snowman", c.id);
                    let wk = self.walk_of(i);
                    wk.smile = wk.smile.max(1.5);
                    wk.start_bob(tn().jump_height * 0.3);
                    self.snowman_over(c);
                    return;
                }
            }
            c.gift_on = Some(g);
            return;
        }
        let Some(mut b) = c.build.take() else {
            self.snowman_over(c);
            return;
        };
        let team = c.members.clone();
        let height = self.height_of(team[0]);
        let width = self.width_of(team[0]);
        let sizes = [b.big, b.big * 0.74, b.big * 0.55];
        let range = height * 2.2;
        let behind = |b: &Build, k: usize| b.ball.0 - b.dir * (b.ball.1 * 0.5 + width * 0.3 + k as f32 * width * 0.45);
        b.t += dt;
        match b.phase {
            0 => {
                let mut there = true;
                for (k, &i) in team.iter().enumerate() {
                    let at = behind(&b, k);
                    if (self.strip_px(i) - at).abs() > 6.0 {
                        there = false;
                        if self.arrived(i) {
                            self.steer(i, at, 0.16);
                        }
                    }
                }
                if there || b.t > 6.0 {
                    b.phase = 1;
                    b.t = 0.0;
                }
            }
            1 => {
                // easier with more of them
                let speed = tn().snowman_roll_speed * (0.8 + 0.2 * (team.len() as f32).min(3.0));
                b.ball.0 += b.dir * speed * dt;
                b.ball.1 = (b.ball.1 + speed * dt * 0.08).min(sizes[b.part]);
                self.push_along(&team, b.dir, |k| behind(&b, k), dt);
                if b.ball.1 >= sizes[b.part] {
                    self.free_crew(&team);
                    for &i in &team {
                        self.walk_of(i).start_bob(tn().jump_height * 0.15);
                    }
                    if b.part == 0 {
                        // the bottom stays where it is: that's the snowman's place
                        b.base = b.ball.0;
                        self.next_ball(&mut b, width, height);
                    } else {
                        // up it goes, carried over by whoever's nearest
                        let x = b.ball.0;
                        b.carrier = team.iter().copied().min_by(|&p, &q| (self.strip_px(p) - x).abs().total_cmp(&(self.strip_px(q) - x).abs())).unwrap_or(team[0]);
                        let side = if x < b.base { -1.0 } else { 1.0 };
                        self.steer(b.carrier, b.base + side * (b.big * 0.5 + width * 0.35), 0.12);
                        b.phase = 2;
                        b.t = 0.0;
                    }
                } else if (b.ball.0 - b.from) * b.dir > range {
                    // as far as it goes this way: round and back
                    b.dir = -b.dir;
                    b.phase = 0;
                    b.t = 0.0;
                    self.free_crew(&team);
                }
            }
            2 => {
                for &i in team.iter().filter(|&&i| i != b.carrier) {
                    if self.arrived(i) {
                        self.face_x(i, b.base);
                    }
                }
                if self.arrived(b.carrier) || b.t > 8.0 {
                    self.face_x(b.carrier, b.base);
                    b.phase = 3;
                    b.t = 0.0;
                }
            }
            3 => {
                if b.t > 0.6 {
                    let wk = self.walk_of(b.carrier);
                    wk.start_bob(tn().jump_height * 0.2);
                    wk.smile = wk.smile.max(0.8);
                    b.part += 1;
                    if b.part >= 3 {
                        log::debug!("chat {}: the snowman's done", c.id);
                        let stay = std::time::Duration::from_secs_f32(tn().snowman_stay.max(1.0));
                        let mut men = SNOWMEN.lock().unwrap();
                        let now = std::time::Instant::now();
                        men.retain(|m| m.until > now);
                        men.push(Snowman { x: b.base, ws: b.ws, ground: b.ground, size: b.big, until: now + stay, deco: None });
                        drop(men);
                        for &i in &team {
                            let wk = self.walk_of(i);
                            wk.smile = wk.smile.max(2.0);
                            wk.start_jump(tn().jump_height * 0.5);
                        }
                        b.phase = 4;
                        b.t = 0.0;
                    } else {
                        self.next_ball(&mut b, width, height);
                    }
                }
            }
            _ => {
                // a step back to look at it, proud
                for &i in &team {
                    if self.arrived(i) {
                        self.face_x(i, b.base);
                    }
                    let wk = self.walk_of(i);
                    if grounded(wk) && wk.rand() < dt * 1.0 {
                        wk.start_bob(tn().jump_height * 0.2);
                    }
                }
                if b.t > 2.5 {
                    self.snowman_over(c);
                    return;
                }
            }
        }
        c.build = Some(b);
    }

    /// A fresh little ball for the next part, rolled beside the snowman.
    fn next_ball(&mut self, b: &mut Build, width: f32, height: f32) {
        let side = if self.walk.rand() < 0.5 { -1.0 } else { 1.0 };
        let from = b.base + side * (b.big * 0.5 + height * 1.2);
        b.from = from;
        b.dir = side;
        b.ball = (from, height * 0.15);
        let _ = width;
        b.phase = 0;
        b.t = 0.0;
    }

    fn snowman_over(&mut self, c: &mut Chat) {
        for i in c.members.clone() {
            let cool = self.walk_ref(i).snow_cool.max(60.0);
            self.walk_of(i).snow_cool = cool;
            self.leave(i, c.id);
        }
        self.free_crew(&c.members.clone());
        c.members.clear();
        c.build = None;
        c.gift_on = None;
        c.timer = 0.0;
        c.stage = Stage::Part;
    }

    /// The snowman so far: what's been put together, and the ball being rolled or carried.
    pub(super) fn build_props(&self, v: &mut Vec<(i32, [f32; 4], bool, usize)>) {
        for c in self.chats.iter().filter(|c| c.stage == Stage::Snowman) {
            let Some(b) = c.build.as_ref() else { continue };
            let Some(&owner) = c.members.first() else { continue };
            if b.part >= 3 {
                continue;
            }
            let own = if owner == 0 { self.own_view } else { self.extras[owner - 1].own_view };
            let shift = own[0] * self.view_w * 0.5;
            let x = b.base - shift;
            let ball = |v: &mut Vec<(i32, [f32; 4], bool, usize)>, cx: f32, cy: f32, size: f32| {
                let h = size * 0.5;
                v.push((SNOWBALL, [cx - h, cy - h, cx + h, cy + h], false, owner));
            };
            // the parts already in place, where the finished snowman has them
            if b.part >= 1 {
                ball(v, x, b.ground - b.big * 0.5, b.big);
            }
            if b.part >= 2 {
                ball(v, x, b.ground - b.big * 1.155, b.big * 0.74);
            }
            match b.phase {
                0 | 1 => ball(v, b.ball.0 - shift, b.ground - b.ball.1 * 0.5, b.ball.1),
                2 | 3 => {
                    // held up over the head, then lifted on
                    let r = self.body_rect(b.carrier);
                    let held = [(r[0] + r[2]) * 0.5, r[1] - b.ball.1 * 0.35];
                    let up = if b.part == 1 { b.ground - b.big * 1.155 } else { b.ground - b.big * 1.75 };
                    let k = if b.phase == 3 { (b.t / 0.6).clamp(0.0, 1.0) } else { 0.0 };
                    ball(v, held[0] + (x - held[0]) * k, held[1] + (up - held[1]) * k, b.ball.1);
                }
                _ => {}
            }
        }
    }
}
