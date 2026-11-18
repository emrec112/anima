//! Snowball fight: two sides scoop up little snowballs and throw them at
//! each other. Now and then a few of them go off together and roll a big
//! one along the floor, growing as it goes, and it ends up a snowman.

use super::super::CharacterOverlay;
use super::*;
use crate::render::tuning::get as tn;

pub(super) const SNOWBALL: i32 = 43;
pub(super) const SNOWMAN: i32 = 44;
pub(super) const PUFF: i32 = 45;

/// One of those throwing. phase 0: a breather while `t` < 0, then scooping
/// a ball up; 1: winding up to throw at `target`.
#[derive(Clone, Copy, Debug)]
pub(super) struct Thrower {
    pub(super) i: usize,
    phase: u8,
    t: f32,
    // the ball in hand, 0..1 of a full one
    pub(super) ball: f32,
    target: usize,
}

/// A snowball in the air: strip x and screen y it left from and is going to.
#[derive(Clone, Copy, Debug)]
pub(super) struct Flying {
    pub(super) by: usize,
    target: usize,
    pub(super) from: [f32; 2],
    pub(super) to: [f32; 2],
    pub(super) t: f32,
    pub(super) dur: f32,
    // ducks out of the way just before it gets there
    dodge: bool,
}

/// A few rolling a big ball together. phase 0: getting behind it, 1:
/// pushing, 2: it's a snowman, cheering.
#[derive(Clone, Debug)]
pub(super) struct Roll {
    pub(super) crew: Vec<usize>,
    // strip x of its middle, how wide (px), which way it's going
    pub(super) x: f32,
    pub(super) size: f32,
    dir: f32,
    phase: u8,
    t: f32,
}

#[derive(Clone, Debug)]
pub(super) struct Snow {
    lo: f32,
    hi: f32,
    // screen y of the floor
    pub(super) ground: f32,
    sides: [Vec<usize>; 2],
    pub(super) hands: Vec<Thrower>,
    pub(super) balls: Vec<Flying>,
    // bursts of snow where a ball came down: strip x, screen y, age (s)
    pub(super) puffs: Vec<(f32, f32, f32)>,
    pub(super) roll: Option<Roll>,
}

/// A snowman left standing after the game: strip x, workspace, floor y,
/// width of its bottom ball, and when it's gone.
pub(super) struct Snowman {
    pub(super) x: f32,
    pub(super) ws: u64,
    pub(super) ground: f32,
    pub(super) size: f32,
    until: std::time::Instant,
}

pub(super) static SNOWMEN: std::sync::Mutex<Vec<Snowman>> = std::sync::Mutex::new(Vec::new());

const PUFF_LIFE: f32 = 0.45;

impl CharacterOverlay {
    fn body_rect(&self, i: usize) -> [f32; 4] {
        if i == 0 { self.hit_rect } else { self.extras[i - 1].hit_rect }
    }

    pub(super) fn start_snow(&mut self, c: &mut Chat) {
        log::debug!("chat {}: snowball fight", c.id);
        let (x, len) = self.big_window();
        let (lo, hi) = (x + 60.0, x + len - 60.0);
        let mut players = c.members.clone();
        // a few go off and roll a big one, if there are enough of them
        let mut crew = Vec::new();
        if players.len() >= tn().snow_roll_min as usize && self.walk.rand() < tn().snow_roll {
            let k = if players.len() >= 7 { 3 } else { 2 };
            for _ in 0..k {
                let j = (self.walk.rand() * players.len() as f32) as usize % players.len();
                crew.push(players.remove(j));
            }
        }
        // two sides, by where they stand: nobody has to cross over
        players.sort_by(|a, b| self.strip_px(*a).total_cmp(&self.strip_px(*b)));
        let half = players.len() / 2;
        let sides = [players[..half].to_vec(), players[half..].to_vec()];
        let span = (hi - lo) * 0.28;
        for (side, team) in sides.iter().enumerate() {
            for (k, &i) in team.iter().enumerate() {
                let at = (k as f32 + 0.5) / team.len().max(1) as f32 * span;
                let goal = if side == 0 { lo + at } else { hi - at };
                self.go(i, goal, 0.16);
                if let Some(r) = self.walk_of(i).social.as_mut() {
                    r.face_left = side == 1;
                }
            }
        }
        let roll = (!crew.is_empty()).then(|| {
            let size = (self.body_rect(crew[0])[3] - self.body_rect(crew[0])[1]).max(1.0) * 0.2;
            Roll { crew: crew.clone(), x: lo + span + self.width_of(crew[0]) * 1.5, size, dir: 1.0, phase: 0, t: 0.0 }
        });
        if let Some(r) = &roll {
            log::debug!("chat {}: {:?} go and roll a big one", c.id, r.crew);
        }
        c.snow = Some(Snow { lo, hi, ground: 0.0, sides, hands: Vec::new(), balls: Vec::new(), puffs: Vec::new(), roll });
        self.align_park_y(&c.members);
        c.scores = c.members.iter().map(|&i| (i, 0.0)).collect();
        c.step = 9;
        c.wait = 6.0;
        c.timer = tn().snow_time_min + self.walk.rand() * tn().snow_time_rand;
        c.regroup = true;
        c.stage = Stage::Snow;
    }

    /// Where `side` stands: (from, to) on the strip.
    fn snow_zone(s: &Snow, side: usize) -> (f32, f32) {
        let span = (s.hi - s.lo) * 0.28;
        if side == 0 { (s.lo, s.lo + span) } else { (s.hi - span, s.hi) }
    }

    pub(super) fn run_snow(&mut self, c: &mut Chat, dt: f32) {
        let Some(mut s) = c.snow.take() else { return };
        for side in &mut s.sides {
            side.retain(|i| c.members.contains(i));
        }
        s.hands.retain(|h| c.members.contains(&h.i));
        if let Some(r) = s.roll.as_mut() {
            r.crew.retain(|i| c.members.contains(i));
        }
        if s.roll.as_ref().is_some_and(|r| r.crew.is_empty()) {
            s.roll = None;
        }
        if s.sides[0].is_empty() || s.sides[1].is_empty() {
            self.end_game(c);
            return;
        }
        if c.step == 9 {
            c.wait -= dt;
            let ready = s.sides.iter().flatten().all(|&i| self.arrived(i));
            if ready || c.wait <= 0.0 {
                self.clear_happy(&c.members);
                s.ground = c.members.iter().map(|&i| self.body_rect(i)[3]).fold(f32::MIN, f32::max);
                for &i in s.sides.iter().flatten() {
                    // not all at once: some get going straight away
                    let t = -self.walk.rand() * 1.5;
                    s.hands.push(Thrower { i, phase: 0, t, ball: 0.0, target: i });
                }
                c.step = 0;
            } else {
                self.chatter(c, dt);
            }
            c.snow = Some(s);
            return;
        }
        let throwing = c.timer > 0.0;
        if !throwing && s.balls.is_empty() && s.roll.as_ref().is_none_or(|r| r.phase == 2 && r.t > 2.0) {
            log::debug!("chat {}: snowball fight over, {:?}", c.id, c.scores);
            self.end_game(c);
            return;
        }
        self.snow_hands(c, &mut s, dt, throwing);
        self.snow_balls(c, &mut s, dt);
        self.snow_roll(c, &mut s, dt);
        c.snow = Some(s);
    }

    fn snow_hands(&mut self, c: &Chat, s: &mut Snow, dt: f32, throwing: bool) {
        for k in 0..s.hands.len() {
            let mut h = s.hands[k];
            let i = h.i;
            let side = usize::from(s.sides[1].contains(&i));
            if !self.arrived(i) {
                s.hands[k] = h;
                continue;
            }
            h.t += dt;
            match h.phase {
                0 if h.t < 0.0 => {}
                0 => {
                    // down for a handful, patting it into a ball
                    h.ball = (h.t / 0.9).clamp(0.0, 1.0);
                    let wk = self.walk_of(i);
                    if wk.rand() < dt * 2.0 {
                        wk.start_bob(tn().jump_height * 0.05);
                    }
                    if h.ball >= 1.0 && throwing {
                        let foes = &s.sides[1 - side];
                        let pick = foes[(self.walk.rand() * foes.len() as f32) as usize % foes.len()];
                        h.target = pick;
                        h.phase = 1;
                        h.t = 0.0;
                        self.face(i, pick);
                        self.walk_of(i).gesture = true;
                    }
                }
                _ => {
                    if !c.members.contains(&h.target) {
                        h.phase = 0;
                        h.t = 0.0;
                    } else if h.t > 0.35 {
                        let (r, tr) = (self.body_rect(i), self.body_rect(h.target));
                        let (hh, th) = ((r[3] - r[1]).max(1.0), (tr[3] - tr[1]).max(1.0));
                        let (x, tx) = (self.strip_px(i), self.strip_px(h.target));
                        let dir = if tx < x { -1.0 } else { 1.0 };
                        let from = [x + dir * self.width_of(i) * 0.28, r[1] + hh * 0.45];
                        let to = [tx, tr[1] + th * 0.45];
                        let dur = (0.5 + (tx - x).abs() / 900.0).clamp(0.5, 1.2);
                        let knack = self.skilled(h.target, "snowball fight", 0.6, 1.4);
                        let dodge = self.walk.rand() < tn().snow_dodge * knack;
                        s.balls.push(Flying { by: i, target: h.target, from, to, t: 0.0, dur, dodge });
                        self.walk_of(i).start_bob(tn().jump_height * 0.12);
                        h.ball = 0.0;
                        h.phase = 0;
                        // a breather, and now and then a step somewhere else
                        h.t = -(0.8 + self.walk.rand() * 1.8);
                        if self.walk.rand() < 0.4 {
                            let (a, b) = Self::snow_zone(s, side);
                            let goal = a + self.walk.rand() * (b - a);
                            self.steer(i, goal, 0.14);
                        }
                    }
                }
            }
            // eyes on the other side
            if h.phase == 0 && self.arrived(i) {
                if let Some(r) = self.walk_of(i).social.as_mut() {
                    r.face_left = side == 1;
                }
            }
            s.hands[k] = h;
        }
    }

    fn snow_balls(&mut self, c: &mut Chat, s: &mut Snow, dt: f32) {
        for p in &mut s.puffs {
            p.2 += dt;
        }
        s.puffs.retain(|p| p.2 < PUFF_LIFE);
        let mut landed = Vec::new();
        for k in 0..s.balls.len() {
            let b = &mut s.balls[k];
            let before = b.t / b.dur;
            b.t += dt;
            let now = b.t / b.dur;
            // the one it's meant for sees it coming and hops out of the way
            if b.dodge && before < 0.45 && now >= 0.45 {
                let target = b.target;
                let wk = self.walk_of(target);
                if grounded(wk) {
                    wk.start_jump(tn().jump_height * 0.5);
                }
            }
            if now >= 1.0 {
                landed.push(k);
            }
        }
        for &k in landed.iter().rev() {
            let b = s.balls.remove(k);
            let here = c.members.contains(&b.target);
            let close = here && (self.strip_px(b.target) - b.to[0]).abs() < self.width_of(b.target) * 0.45;
            if close && grounded(self.walk_ref(b.target)) {
                // a hit: a burst of snow, a smile, no hard feelings
                s.puffs.push((b.to[0], b.to[1], 0.0));
                let wk = self.walk_of(b.target);
                wk.smile = wk.smile.max(0.7);
                wk.start_bob(tn().jump_height * 0.18);
                if let Some(sc) = c.scores.iter_mut().find(|sc| sc.0 == b.by) {
                    sc.1 += 1.0;
                }
                let wk = self.walk_of(b.by);
                wk.smile = wk.smile.max(0.6);
                log::debug!("chat {}: {} hits {}", c.id, b.by, b.target);
            } else {
                // missed: it comes down in the snow behind
                let dir = if b.to[0] >= b.from[0] { 1.0 } else { -1.0 };
                s.puffs.push((b.to[0] + dir * 40.0, s.ground - 6.0, 0.0));
            }
        }
    }

    fn snow_roll(&mut self, c: &mut Chat, s: &mut Snow, dt: f32) {
        let Some(mut r) = s.roll.take() else { return };
        let first = r.crew[0];
        let height = (self.body_rect(first)[3] - self.body_rect(first)[1]).max(1.0);
        let width = self.width_of(first);
        let biggest = height * tn().snow_big;
        // the stretch between the two sides; the whole field if that's too short
        let (za, _) = Self::snow_zone(s, 1);
        let (_, zb) = Self::snow_zone(s, 0);
        let (lo, hi) = if za - zb > biggest * 3.0 { (zb + width, za - width) } else { (s.lo, s.hi) };
        // places behind the ball, one after the other
        let spot = |r: &Roll, k: usize| r.x - r.dir * (r.size * 0.5 + width * 0.3 + k as f32 * width * 0.45);
        r.t += dt;
        match r.phase {
            0 => {
                let mut there = true;
                for (k, &i) in r.crew.iter().enumerate() {
                    let at = spot(&r, k);
                    if (self.strip_px(i) - at).abs() > 6.0 {
                        there = false;
                        if self.arrived(i) {
                            self.steer(i, at, 0.16);
                        }
                    }
                }
                if there || r.t > 8.0 {
                    r.phase = 1;
                    r.t = 0.0;
                }
            }
            1 => {
                // heavier the bigger it gets
                let speed = tn().snow_roll_speed * (1.0 - 0.45 * (r.size / biggest).min(1.0));
                r.x += r.dir * speed * dt;
                r.size = (r.size + speed * dt * 0.07).min(biggest);
                for (k, &i) in r.crew.clone().iter().enumerate() {
                    let at = spot(&r, k);
                    self.steer(i, at, 0.12);
                    let wk = self.walk_of(i);
                    if wk.rand() < dt * 0.8 {
                        wk.start_bob(tn().jump_height * 0.06);
                    }
                    if let Some(sr) = wk.social.as_mut() {
                        sr.face_left = r.dir < 0.0;
                        sr.face_lock = true;
                        sr.lean = -r.dir * 0.14;
                    }
                }
                if r.size >= biggest {
                    // big enough: on goes a head, and there's a snowman
                    log::debug!("chat {}: the big snowball is a snowman", c.id);
                    let ws = self.walk_ref(first).ws;
                    let stay = std::time::Duration::from_secs_f32(tn().snowman_stay.max(1.0));
                    let mut men = SNOWMEN.lock().unwrap();
                    men.retain(|m| m.until > std::time::Instant::now());
                    men.push(Snowman { x: r.x, ws, ground: s.ground, size: r.size, until: std::time::Instant::now() + stay });
                    drop(men);
                    for &i in &r.crew {
                        let wk = self.walk_of(i);
                        wk.start_jump(tn().jump_height * 0.6);
                        wk.smile = wk.smile.max(2.0);
                        if let Some(sr) = wk.social.as_mut() {
                            sr.face_lock = false;
                            sr.lean = 0.0;
                        }
                    }
                    r.phase = 2;
                    r.t = 0.0;
                } else if (r.dir > 0.0 && r.x + r.size * 0.5 >= hi) || (r.dir < 0.0 && r.x - r.size * 0.5 <= lo) {
                    // the end of the floor: round to the other side and back
                    r.dir = -r.dir;
                    r.phase = 0;
                    r.t = 0.0;
                    for &i in &r.crew {
                        if let Some(sr) = self.walk_of(i).social.as_mut() {
                            sr.face_lock = false;
                            sr.lean = 0.0;
                        }
                    }
                }
            }
            _ => {
                // proud of it for a moment, then off to join the fight
                for &i in &r.crew {
                    self.face_x(i, r.x);
                    let wk = self.walk_of(i);
                    if grounded(wk) && wk.rand() < dt * 1.5 {
                        wk.start_bob(tn().jump_height * 0.25);
                    }
                }
                if r.t > 2.0 {
                    if c.timer > 0.0 {
                        for &i in &r.crew {
                            let side = usize::from(s.sides[1].len() < s.sides[0].len());
                            s.sides[side].push(i);
                            let (a, b) = Self::snow_zone(s, side);
                            let goal = a + self.walk.rand() * (b - a);
                            self.steer(i, goal, 0.16);
                            s.hands.push(Thrower { i, phase: 0, t: -1.0, ball: 0.0, target: i });
                        }
                    }
                    if c.timer > 0.0 {
                        return;
                    }
                }
            }
        }
        s.roll = Some(r);
    }

    fn face_x(&mut self, i: usize, x: f32) {
        let left = x < self.strip_px(i);
        if let Some(r) = self.walk_of(i).social.as_mut() {
            r.face_left = left;
        }
    }

    /// Snowballs in hand and in the air, bursts of snow, the big one being
    /// rolled, and any snowmen standing about.
    pub(super) fn snow_props(&self, v: &mut Vec<(i32, [f32; 4], bool, usize)>) {
        for c in self.chats.iter().filter(|c| c.stage == Stage::Snow && c.step != 9) {
            let Some(s) = c.snow.as_ref() else { continue };
            let Some(&owner) = c.members.first() else { continue };
            let own = if owner == 0 { self.own_view } else { self.extras[owner - 1].own_view };
            let shift = own[0] * self.view_w * 0.5;
            let height = (self.body_rect(owner)[3] - self.body_rect(owner)[1]).max(1.0);
            let ball = height * 0.15;
            for h in s.hands.iter().filter(|h| h.ball > 0.0) {
                let r = self.body_rect(h.i);
                let side = if self.walk_ref(h.i).left { -1.0 } else { 1.0 };
                let x = (r[0] + r[2]) * 0.5 + side * (r[2] - r[0]) * 0.28;
                let y = r[1] + (r[3] - r[1]) * 0.55;
                let size = ball * (0.35 + 0.65 * h.ball) * 0.5;
                v.push((SNOWBALL, [x - size, y - size, x + size, y + size], false, h.i));
            }
            for b in &s.balls {
                let u = (b.t / b.dur).clamp(0.0, 1.0);
                let lift = height * 0.35 + (b.to[0] - b.from[0]).abs() * 0.12;
                let x = b.from[0] + (b.to[0] - b.from[0]) * u - shift;
                let y = b.from[1] + (b.to[1] - b.from[1]) * u - lift * 4.0 * u * (1.0 - u);
                let size = ball * 0.5;
                v.push((SNOWBALL, [x - size, y - size, x + size, y + size], false, b.by));
            }
            for &(px, py, age) in &s.puffs {
                let k = age / PUFF_LIFE;
                let size = height * (0.12 + 0.18 * k);
                let x = px - shift;
                v.push((PUFF, [x - size, py - size, x + size, py + size], false, owner));
            }
            if let Some(r) = s.roll.as_ref().filter(|r| r.phase < 2) {
                let x = r.x - shift;
                let half = r.size * 0.5;
                v.push((SNOWBALL, [x - half, s.ground - r.size, x + half, s.ground], false, r.crew[0]));
            }
        }
        let n = self.extras.len() + 1;
        let now = std::time::Instant::now();
        for m in SNOWMEN.lock().unwrap().iter().filter(|m| m.until > now) {
            // drawn with someone on its workspace (the picture follows their view)
            let Some(owner) = (0..n).find(|&i| self.walk_ref(i).ws == m.ws) else { continue };
            let own = if owner == 0 { self.own_view } else { self.extras[owner - 1].own_view };
            let x = m.x - own[0] * self.view_w * 0.5;
            // the shader's bottom ball is 0.42 of the box height, the box 0.6 as wide as tall
            let h = m.size / 0.42;
            let w = h * 0.6;
            v.push((SNOWMAN, [x - w * 0.5, m.ground - h, x + w * 0.5, m.ground], false, owner));
        }
    }
}
