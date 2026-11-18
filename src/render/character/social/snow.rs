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
    // how long it'll keep at it (s): runs down, and it's had enough
    patience: f32,
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

    /// Asked for by hand: those picked play it out on the widest window.
    pub(super) fn start_snow(&mut self, c: &mut Chat) {
        let (x, len) = self.big_window();
        self.snow_setup(c, x + 60.0, x + len - 60.0, None);
    }

    /// Two sides on the stretch `lo`..`hi`, by where they stand; `opener`
    /// throws the first one.
    fn snow_setup(&mut self, c: &mut Chat, lo: f32, hi: f32, opener: Option<usize>) {
        log::debug!("chat {}: snowball fight, {:?}", c.id, c.members);
        let mut players = c.members.clone();
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
        c.snow = Some(Snow { lo, hi, ground: 0.0, sides, hands: Vec::new(), balls: Vec::new(), puffs: Vec::new(), roll: None });
        if let Some(o) = opener {
            c.talker = o;
        }
        self.align_park_y(&c.members);
        c.scores = c.members.iter().map(|&i| (i, 0.0)).collect();
        c.step = 9;
        c.wait = 6.0;
        c.timer = tn().snow_longest;
        c.stage = Stage::Snow;
    }

    fn snow_patience(&mut self) -> f32 {
        tn().snow_stay_min + self.walk.rand() * tn().snow_stay_rand
    }

    /// Free to start or join a snowball fight.
    fn snow_free(&self, i: usize) -> bool {
        self.free_for(i, Want::Join) && self.walk_ref(i).snow_cool <= 0.0
    }

    /// It's snowing: someone with nothing to do scoops up a handful and
    /// throws it at someone nearby, and that's a snowball fight.
    pub(super) fn snow_spark(&mut self, chats: &[Chat], n: usize, every: f32) -> Option<Chat> {
        if !super::super::weather::now().snow || self.social_age < tn().warm_small || self.moods.fretting {
            return None;
        }
        if self.walk.rand() >= every / tn().snow_start_every.max(1.0) {
            return None;
        }
        let ready: Vec<usize> = (0..n).filter(|&i| self.snow_free(i) && self.on_screen(i)).collect();
        if ready.len() < 2 {
            return None;
        }
        let i = ready[(self.walk.rand() * ready.len() as f32) as usize % ready.len()];
        let ws = self.walk_ref(i).ws;
        // one fight to a workspace: the others can join that one
        if chats.iter().any(|c| c.stage == Stage::Snow && c.members.first().is_some_and(|&m| self.walk_ref(m).ws == ws)) {
            return None;
        }
        let x = self.strip_px(i);
        let j = ready
            .iter()
            .copied()
            .filter(|&j| j != i && self.walk_ref(j).ws == ws && (self.strip_px(j) - x).abs() < self.view_w * 0.5)
            .min_by(|&a, &b| (self.strip_px(a) - x).abs().total_cmp(&(self.strip_px(b) - x).abs()))?;
        self.chat_id = self.chat_id.wrapping_add(1);
        let id = self.chat_id;
        self.enlist(i, id);
        self.enlist(j, id);
        let mid = (x + self.strip_px(j)) * 0.5;
        let size = self.width_of(i);
        let width = self.strip_width_of(i);
        let half = self.view_w * 0.4;
        let (lo, hi) = ((mid - half).max(size), (mid + half).min(width - size));
        let (lo, hi) = if hi - lo < size * 6.0 { (mid - size * 3.0, mid + size * 3.0) } else { (lo, hi) };
        log::debug!("chat {id}: {i} throws a snowball at {j}");
        let mut c = Chat::new(id, vec![i, j], Stage::Snow, 0.0);
        self.snow_setup(&mut c, lo, hi, Some(i));
        c.played.push(Game::Snow);
        c.games = 1;
        Some(c)
    }

    /// Someone free nearby sees the snowballs flying and joins the side
    /// with fewer on it. There's no limit: the more the merrier.
    pub(super) fn snow_join(&mut self, c: &mut Chat, n: usize, every: f32) {
        if c.step == 9 || c.timer <= 0.0 || !super::super::weather::now().snow {
            return;
        }
        let Some(s) = c.snow.as_ref() else { return };
        let Some(&first) = c.members.first() else { return };
        let ws = self.walk_ref(first).ws;
        let mid = (s.lo + s.hi) * 0.5;
        let reach = (s.hi - s.lo) * 0.5 + self.view_w * 0.3;
        let Some(j) = (0..n).find(|&j| !c.members.contains(&j) && self.walk_ref(j).ws == ws && self.snow_free(j) && (self.strip_px(j) - mid).abs() < reach) else { return };
        let keen = 1.0 + c.members.iter().map(|&m| self.bond(j, m).0).fold(0.0, f32::max);
        if self.walk_of(j).rand() >= every * tn().snow_join * keen {
            return;
        }
        self.enlist(j, c.id);
        let patience = self.snow_patience();
        let Some(s) = c.snow.as_mut() else { return };
        let side = if s.sides[0].len() != s.sides[1].len() {
            usize::from(s.sides[1].len() < s.sides[0].len())
        } else {
            usize::from(self.strip_px(j) > mid)
        };
        s.sides[side].push(j);
        s.hands.push(Thrower { i: j, phase: 0, t: -0.8, ball: 0.0, target: j, patience });
        let (a, b) = Self::snow_zone(s, side);
        let goal = a + self.walk.rand() * (b - a);
        c.members.push(j);
        c.scores.push((j, 0.0));
        self.go(j, goal, 0.18);
        log::debug!("chat {}: {j} joins the snowball fight on side {side}", c.id);
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
        let crew = s.roll.as_ref().map_or(0, |r| r.crew.len());
        if s.sides[0].len() + s.sides[1].len() + crew < 2 {
            self.snow_over(c);
            return;
        }
        if c.step == 9 {
            c.wait -= dt;
            let ready = s.sides.iter().flatten().all(|&i| self.arrived(i));
            if ready || c.wait <= 0.0 {
                self.clear_happy(&c.members);
                s.ground = c.members.iter().map(|&i| self.body_rect(i)[3]).fold(f32::MIN, f32::max);
                for i in s.sides.iter().flatten().copied().collect::<Vec<_>>() {
                    // not all at once; whoever started it has one ready
                    let t = if i == c.talker && c.games == 1 { 0.9 } else { -self.walk.rand() * 1.5 };
                    let patience = self.snow_patience();
                    s.hands.push(Thrower { i, phase: 0, t, ball: 0.0, target: i, patience });
                }
                c.step = 0;
            } else {
                self.chatter(c, dt);
            }
            c.snow = Some(s);
            return;
        }
        self.snow_tired(c, &mut s, dt);
        self.snow_even(c, &mut s);
        self.snow_crew(c, &mut s, dt);
        let throwing = !s.sides[0].is_empty() && !s.sides[1].is_empty();
        self.snow_hands(c, &mut s, dt, throwing);
        self.snow_balls(c, &mut s, dt);
        self.snow_roll(c, &mut s, dt);
        c.snow = Some(s);
    }

    /// Patience runs down (faster once the snow stops, or it's gone on very
    /// long); whoever's had enough, between throws, goes off happy.
    fn snow_tired(&mut self, c: &mut Chat, s: &mut Snow, dt: f32) {
        let rate = if !super::super::weather::now().snow || c.timer <= 0.0 { 4.0 } else { 1.0 };
        let mut done = Vec::new();
        for h in &mut s.hands {
            h.patience -= dt * rate;
            if h.patience <= 0.0 && h.phase == 0 && h.t < 0.0 {
                done.push(h.i);
            }
        }
        for i in done {
            log::debug!("chat {}: {i} has had enough snowballs", c.id);
            s.hands.retain(|h| h.i != i);
            for side in &mut s.sides {
                side.retain(|&m| m != i);
            }
            c.members.retain(|&m| m != i);
            c.scores.retain(|sc| sc.0 != i);
            let wk = self.walk_of(i);
            wk.smile = wk.smile.max(1.0);
            wk.start_bob(tn().jump_height * 0.25);
            wk.snow_cool = tn().snow_again;
            self.leave(i, c.id);
        }
    }

    /// A side's gone empty: one from the other side comes over to it.
    fn snow_even(&mut self, c: &Chat, s: &mut Snow) {
        for side in 0..=1 {
            if !s.sides[side].is_empty() || s.sides[1 - side].len() < 2 {
                continue;
            }
            let mid = (s.lo + s.hi) * 0.5;
            let other = &s.sides[1 - side];
            let Some(&i) = other.iter().min_by(|&&a, &&b| (self.strip_px(a) - mid).abs().total_cmp(&(self.strip_px(b) - mid).abs())) else { continue };
            s.sides[1 - side].retain(|&m| m != i);
            s.sides[side].push(i);
            let (a, b) = Self::snow_zone(s, side);
            let goal = a + self.walk.rand() * (b - a);
            self.steer(i, goal, 0.2);
            log::debug!("chat {}: {i} goes over to the other side", c.id);
        }
    }

    /// Enough of them: now and then a couple go off and roll a big one.
    fn snow_crew(&mut self, c: &Chat, s: &mut Snow, dt: f32) {
        let players = s.sides[0].len() + s.sides[1].len();
        if s.roll.is_some() || players < tn().snow_roll_min as usize || self.walk.rand() >= dt * tn().snow_roll_rate {
            return;
        }
        let want = if players >= 7 { 3 } else { 2 };
        let mut crew = Vec::new();
        for _ in 0..want {
            // from the bigger side, and never the last one on a side
            let side = usize::from(s.sides[1].len() > s.sides[0].len());
            if s.sides[side].len() < 2 {
                break;
            }
            let team = &s.sides[side];
            // not mid-throw
            let free: Vec<usize> = team.iter().copied().filter(|&i| s.hands.iter().any(|h| h.i == i && h.phase == 0)).collect();
            if free.is_empty() {
                break;
            }
            let i = free[(self.walk.rand() * free.len() as f32) as usize % free.len()];
            s.sides[side].retain(|&m| m != i);
            crew.push(i);
        }
        if crew.len() < 2 {
            // not enough to roll it: back to throwing
            for i in crew {
                let side = usize::from(s.sides[1].len() < s.sides[0].len());
                s.sides[side].push(i);
            }
            return;
        }
        // their patience is kept for when they come back to throwing
        s.hands.iter_mut().filter(|h| crew.contains(&h.i)).for_each(|h| h.ball = 0.0);
        let height = (self.body_rect(crew[0])[3] - self.body_rect(crew[0])[1]).max(1.0);
        let (za, _) = Self::snow_zone(s, 1);
        let (_, zb) = Self::snow_zone(s, 0);
        log::debug!("chat {}: {crew:?} go and roll a big one", c.id);
        s.roll = Some(Roll { crew, x: (za + zb) * 0.5, size: height * 0.2, dir: if self.walk.rand() < 0.5 { -1.0 } else { 1.0 }, phase: 0, t: 0.0 });
    }

    /// Everyone's gone but one: it's over, no winners, just a good time.
    fn snow_over(&mut self, c: &mut Chat) {
        log::debug!("chat {}: snowball fight over", c.id);
        for i in c.members.clone() {
            let wk = self.walk_of(i);
            wk.smile = wk.smile.max(1.0);
            wk.snow_cool = tn().snow_again;
            if let Some(r) = wk.social.as_mut() {
                r.face_lock = false;
                r.lean = 0.0;
            }
            self.leave(i, c.id);
        }
        c.members.clear();
        c.snow = None;
        c.timer = 0.0;
        c.stage = Stage::Part;
    }

    fn snow_hands(&mut self, c: &Chat, s: &mut Snow, dt: f32, throwing: bool) {
        for k in 0..s.hands.len() {
            let mut h = s.hands[k];
            let i = h.i;
            // off rolling the big one
            if !s.sides[0].contains(&i) && !s.sides[1].contains(&i) {
                continue;
            }
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
                    for &i in &r.crew {
                        let side = usize::from(s.sides[1].len() < s.sides[0].len());
                        s.sides[side].push(i);
                        let (a, b) = Self::snow_zone(s, side);
                        let goal = a + self.walk.rand() * (b - a);
                        self.steer(i, goal, 0.16);
                        if let Some(h) = s.hands.iter_mut().find(|h| h.i == i) {
                            h.phase = 0;
                            h.t = -1.0;
                        } else {
                            let patience = tn().snow_stay_min;
                            s.hands.push(Thrower { i, phase: 0, t: -1.0, ball: 0.0, target: i, patience });
                        }
                    }
                    return;
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
