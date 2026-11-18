//! Snowball fight, everyone for themselves: they scatter over the snow and
//! mostly pick one other to have it out with, one on one, now and then
//! switching to someone else. One may run at its rival, which runs off,
//! throwing back over its shoulder. Now and then a few get together and
//! roll a giant snowball, then let it go at the crowd: everyone under it
//! goes flat, and it bursts against the wall.

use super::super::CharacterOverlay;
use super::*;
use crate::render::tuning::get as tn;

pub(super) const SNOWBALL: i32 = 43;
pub(super) const SNOWMAN: i32 = 44;
pub(super) const PUFF: i32 = 45;
const PILE: i32 = 24;

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
    // the one it's having it out with, how long for, and how far off it likes to stand
    rival: Option<usize>,
    rival_t: f32,
    gap: f32,
    // running at its rival, and for how long so far
    chase: Option<usize>,
    chase_t: f32,
    // running off from someone: for how long (below 0 while slipping past
    // them), which way, and the wait before it throws back again
    flee_t: f32,
    flee_dir: f32,
    shot_t: f32,
}

/// A snowball in the air: strip x and screen y it left from and is going to.
/// No target: a lump of a burst giant ball, coming down anywhere.
#[derive(Clone, Copy, Debug)]
pub(super) struct Flying {
    pub(super) by: usize,
    target: Option<usize>,
    pub(super) from: [f32; 2],
    pub(super) to: [f32; 2],
    pub(super) t: f32,
    pub(super) dur: f32,
    // ducks out of the way just before it gets there
    dodge: bool,
    pub(super) size: f32,
}

/// The giant snowball. phase 0: getting behind it, 1: rolling it about so
/// it grows, 2: lining up to let it go, 3: rolling away at speed.
#[derive(Clone, Debug)]
pub(super) struct Roll {
    pub(super) crew: Vec<usize>,
    // strip x of its middle, how wide (px), which way it's going
    pub(super) x: f32,
    pub(super) size: f32,
    dir: f32,
    phase: u8,
    t: f32,
    squashed: Vec<usize>,
}

#[derive(Clone, Debug)]
pub(super) struct Snow {
    lo: f32,
    hi: f32,
    // screen y of the floor
    pub(super) ground: f32,
    // everyone in it, the giant ball's crew too
    players: Vec<usize>,
    pub(super) hands: Vec<Thrower>,
    pub(super) balls: Vec<Flying>,
    // bursts of snow: strip x, screen y, age (s), size (heights)
    pub(super) puffs: Vec<(f32, f32, f32, f32)>,
    // heaps of snow where a giant one burst: strip x, age (s), width (px)
    piles: Vec<(f32, f32, f32)>,
    pub(super) roll: Option<Roll>,
}

/// A snowman left standing: strip x, workspace, floor y, width of its
/// bottom ball, when it's gone, and something someone put on it (its place
/// in the catalog and where it sits).
pub(super) struct Snowman {
    pub(super) x: f32,
    pub(super) ws: u64,
    pub(super) ground: f32,
    pub(super) size: f32,
    pub(super) until: std::time::Instant,
    pub(super) deco: Option<(usize, super::super::wear::Anchor)>,
}

pub(super) static SNOWMEN: std::sync::Mutex<Vec<Snowman>> = std::sync::Mutex::new(Vec::new());

const PUFF_LIFE: f32 = 0.45;
const PILE_LIFE: f32 = 6.0;

impl CharacterOverlay {
    pub(super) fn body_rect(&self, i: usize) -> [f32; 4] {
        if i == 0 { self.hit_rect } else { self.extras[i - 1].hit_rect }
    }

    /// Asked for by hand: those picked play it out on the widest window.
    pub(super) fn start_snow(&mut self, c: &mut Chat) {
        let (x, len) = self.big_window();
        self.snow_setup(c, x + 60.0, x + len - 60.0, None);
    }

    /// Everyone spread out over `lo`..`hi`; `opener` throws the first one.
    fn snow_setup(&mut self, c: &mut Chat, lo: f32, hi: f32, opener: Option<usize>) {
        log::debug!("chat {}: snowball fight, {:?}", c.id, c.members);
        let mut players = c.members.clone();
        players.sort_by(|a, b| self.strip_px(*a).total_cmp(&self.strip_px(*b)));
        let count = players.len() as f32;
        for (k, &i) in players.iter().enumerate() {
            let jitter = (self.walk.rand() - 0.5) * (hi - lo) / count * 0.5;
            let goal = (lo + (k as f32 + 0.5) / count * (hi - lo) + jitter).clamp(lo, hi);
            self.go(i, goal, 0.16);
        }
        c.snow = Some(Snow { lo, hi, ground: 0.0, players, hands: Vec::new(), balls: Vec::new(), puffs: Vec::new(), piles: Vec::new(), roll: None });
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

    fn thrower(&mut self, i: usize, t: f32) -> Thrower {
        let patience = tn().snow_stay_min + self.walk.rand() * tn().snow_stay_rand;
        let gap = self.height_of(i) * (2.5 + self.walk.rand() * 1.5);
        Thrower { i, phase: 0, t, ball: 0.0, target: i, patience, rival: None, rival_t: 0.0, gap, chase: None, chase_t: 0.0, flee_t: 0.0, flee_dir: 1.0, shot_t: 0.0 }
    }

    /// Free to start or join a snowball fight (or a snowman).
    pub(super) fn snow_free(&self, i: usize) -> bool {
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

    /// Someone free nearby sees the snowballs flying and joins in. There's
    /// no limit: the more the merrier.
    pub(super) fn snow_join(&mut self, c: &mut Chat, n: usize, every: f32) {
        if c.step == 9 || c.timer <= 0.0 || !super::super::weather::now().snow {
            return;
        }
        let Some(s) = c.snow.as_ref() else { return };
        let Some(&first) = c.members.first() else { return };
        let ws = self.walk_ref(first).ws;
        let (lo, hi) = (s.lo, s.hi);
        let mid = (lo + hi) * 0.5;
        let reach = (hi - lo) * 0.5 + self.view_w * 0.3;
        let Some(j) = (0..n).find(|&j| !c.members.contains(&j) && self.walk_ref(j).ws == ws && self.snow_free(j) && (self.strip_px(j) - mid).abs() < reach) else { return };
        let keen = 1.0 + c.members.iter().map(|&m| self.bond(j, m).0).fold(0.0, f32::max);
        if self.walk_of(j).rand() >= every * tn().snow_join * keen {
            return;
        }
        self.enlist(j, c.id);
        let hand = self.thrower(j, -0.8);
        let goal = self.strip_px(j).clamp(lo, hi);
        let Some(s) = c.snow.as_mut() else { return };
        s.players.push(j);
        s.hands.push(hand);
        c.members.push(j);
        c.scores.push((j, 0.0));
        self.go(j, goal, 0.18);
        log::debug!("chat {}: {j} joins the snowball fight", c.id);
    }

    fn crewing(s: &Snow, i: usize) -> bool {
        s.roll.as_ref().is_some_and(|r| r.crew.contains(&i))
    }

    pub(super) fn run_snow(&mut self, c: &mut Chat, dt: f32) {
        let Some(mut s) = c.snow.take() else { return };
        s.players.retain(|i| c.members.contains(i));
        s.hands.retain(|h| c.members.contains(&h.i));
        if let Some(r) = s.roll.as_mut() {
            r.crew.retain(|i| c.members.contains(i));
        }
        if s.roll.as_ref().is_some_and(|r| r.crew.is_empty() && r.phase < 3) {
            s.roll = None;
        }
        if s.players.len() < 2 {
            self.snow_over(c);
            return;
        }
        for p in &mut s.puffs {
            p.2 += dt;
        }
        s.puffs.retain(|p| p.2 < PUFF_LIFE);
        for p in &mut s.piles {
            p.1 += dt;
        }
        s.piles.retain(|p| p.1 < PILE_LIFE);
        if c.step == 9 {
            c.wait -= dt;
            let ready = s.players.iter().all(|&i| self.arrived(i));
            if ready || c.wait <= 0.0 {
                self.clear_happy(&c.members);
                s.ground = c.members.iter().map(|&i| self.body_rect(i)[3]).fold(f32::MIN, f32::max);
                for i in s.players.clone() {
                    // not all at once; whoever started it has one ready
                    let t = if i == c.talker && c.games == 1 { 0.9 } else { -self.walk.rand() * 1.5 };
                    let hand = self.thrower(i, t);
                    s.hands.push(hand);
                }
                c.step = 0;
            } else {
                self.chatter(c, dt);
            }
            c.snow = Some(s);
            return;
        }
        self.snow_tired(c, &mut s, dt);
        self.snow_start_roll(c, &mut s, dt);
        self.snow_hands(c, &mut s, dt);
        self.snow_balls(c, &mut s, dt);
        self.snow_roll(c, &mut s, dt);
        c.snow = Some(s);
    }

    /// Patience runs down (faster once the snow stops, or it's gone on very
    /// long); whoever's had enough, between throws, goes off happy.
    fn snow_tired(&mut self, c: &mut Chat, s: &mut Snow, dt: f32) {
        // nobody wanders off while the giant snowball's under way
        if s.roll.is_some() {
            return;
        }
        let rate = if !super::super::weather::now().snow || c.timer <= 0.0 { 4.0 } else { 1.0 };
        let chased: Vec<usize> = s.hands.iter().filter_map(|h| h.chase).collect();
        let mut done = Vec::new();
        for h in &mut s.hands {
            h.patience -= dt * rate;
            if h.patience <= 0.0 && h.phase == 0 && h.t < 0.0 && h.chase.is_none() && !chased.contains(&h.i) {
                done.push(h.i);
            }
        }
        for i in done {
            log::debug!("chat {}: {i} has had enough snowballs", c.id);
            s.hands.retain(|h| h.i != i);
            s.players.retain(|&m| m != i);
            c.members.retain(|&m| m != i);
            c.scores.retain(|sc| sc.0 != i);
            let wk = self.walk_of(i);
            wk.smile = wk.smile.max(1.0);
            wk.start_bob(tn().jump_height * 0.25);
            wk.snow_cool = tn().snow_again;
            self.leave(i, c.id);
        }
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

    /// A snowball on its way from `i` to `target`; close up it's harder to dodge.
    fn snow_throw(&mut self, s: &mut Snow, i: usize, target: usize) {
        let (r, tr) = (self.body_rect(i), self.body_rect(target));
        let (hh, th) = ((r[3] - r[1]).max(1.0), (tr[3] - tr[1]).max(1.0));
        let (x, tx) = (self.strip_px(i), self.strip_px(target));
        let dir = if tx < x { -1.0 } else { 1.0 };
        let from = [x + dir * self.width_of(i) * 0.28, r[1] + hh * 0.45];
        let dur = (0.35 + (tx - x).abs() / 900.0).clamp(0.3, 1.2);
        // aimed ahead of someone on the run, by how fast it's going
        let lead = self.walk_ref(target).social.filter(|r| r.moving).map_or(0.0, |r| (r.goal_px - tx).signum() * r.speed * self.view_w * 0.5 * dur);
        let to = [tx + lead, tr[1] + th * 0.45];
        let close = (tx - x).abs() < hh * 1.2;
        let knack = self.skilled(target, "snowball fight", 0.6, 1.4);
        let dodge = !close && self.walk.rand() < tn().snow_dodge * knack;
        s.balls.push(Flying { by: i, target: Some(target), from, to, t: 0.0, dur, dodge, size: 1.0 });
        self.walk_of(i).start_bob(tn().jump_height * 0.12);
    }

    /// Someone to have it out with: whoever's after this one already (that
    /// makes a pair), else whoever nobody much is after, the nearer the likelier.
    fn pick_rival(&mut self, s: &Snow, i: usize) -> Option<usize> {
        let x = self.strip_px(i);
        let others: Vec<usize> = s.players.iter().copied().filter(|&o| o != i && !Self::crewing(s, o)).collect();
        if others.is_empty() {
            return None;
        }
        let after_me: Vec<usize> = s.hands.iter().filter(|h| h.rival == Some(i) && others.contains(&h.i)).map(|h| h.i).collect();
        if !after_me.is_empty() && self.walk.rand() < 0.7 {
            return Some(after_me[(self.walk.rand() * after_me.len() as f32) as usize % after_me.len()]);
        }
        let mut best = None;
        let mut best_score = f32::MAX;
        for &o in &others {
            let taken = s.hands.iter().filter(|h| h.rival == Some(o)).count() as f32;
            let score = (self.strip_px(o) - x).abs() * (0.6 + self.walk.rand() * 0.8) + taken * self.view_w * 0.2;
            if score < best_score {
                best_score = score;
                best = Some(o);
            }
        }
        best
    }

    fn snow_hands(&mut self, c: &Chat, s: &mut Snow, dt: f32) {
        let chasing: Vec<(usize, usize)> = s.hands.iter().filter_map(|h| h.chase.map(|t| (h.i, t))).collect();
        for k in 0..s.hands.len() {
            let mut h = s.hands[k];
            let i = h.i;
            // off with the giant one, or flat on the floor
            if Self::crewing(s, i) || self.walk_ref(i).flat > 0.0 {
                continue;
            }
            let x = self.strip_px(i);
            let tall = self.height_of(i);
            h.shot_t -= dt;
            // the one it's having it out with: kept a while, then maybe someone else
            h.rival_t += dt;
            let lost = h.rival.is_none_or(|r| !s.players.contains(&r) || Self::crewing(s, r));
            if lost || (h.rival_t > 12.0 && self.walk.rand() < dt * 0.1) {
                h.rival = self.pick_rival(s, i);
                h.rival_t = 0.0;
            }
            let Some(rival) = h.rival else {
                s.hands[k] = h;
                continue;
            };
            // someone's running at it: off it goes, throwing back over its shoulder
            let after: Vec<usize> = chasing.iter().filter(|p| p.1 == i).map(|p| p.0).collect();
            if !after.is_empty() {
                self.snow_flee(c, s, &mut h, &after, dt);
                s.hands[k] = h;
                continue;
            }
            if h.flee_t != 0.0 {
                h.flee_t = 0.0;
            }
            // running at its rival with a ball: throws once right up close
            if let Some(t) = h.chase {
                h.chase_t += dt;
                let tx = self.strip_px(t);
                if !s.players.contains(&t) || Self::crewing(s, t) || self.walk_ref(t).flat > 0.0 {
                    h.chase = None;
                } else if ((tx - x).abs() < tall * 0.7 || h.chase_t > 6.0) && grounded(self.walk_ref(i)) {
                    self.face(i, t);
                    self.snow_throw(s, i, t);
                    h.chase = None;
                    h.ball = 0.0;
                    h.phase = 0;
                    h.t = -(0.6 + self.walk.rand() * 1.0);
                    // and a step back after
                    let back = if tx > x { -1.0 } else { 1.0 };
                    self.steer(i, (x + back * tall).clamp(s.lo, s.hi), 0.18);
                } else {
                    let ahead = if tx > x { 1.0 } else { -1.0 };
                    self.steer(i, tx + ahead * tall * 0.3, tn().snow_chase_speed);
                }
                s.hands[k] = h;
                continue;
            }
            if !self.arrived(i) {
                s.hands[k] = h;
                continue;
            }
            // keeping its distance from its rival
            let rx = self.strip_px(rival);
            let off = x - rx;
            if (off.abs() - h.gap).abs() > tall * 0.8 && h.phase == 0 {
                let side = if off.abs() < 1.0 { if self.walk.rand() < 0.5 { -1.0 } else { 1.0 } } else { off.signum() };
                let goal = (rx + side * h.gap).clamp(s.lo, s.hi);
                self.steer(i, goal, 0.17);
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
                    if h.ball >= 1.0 && self.walk_ref(rival).flat <= 0.0 {
                        // now and then it runs right at its rival instead
                        let busy = chasing.iter().any(|p| p.1 == rival);
                        if !busy && self.walk.rand() < tn().snow_chase {
                            log::debug!("chat {}: {i} runs at {rival}", c.id);
                            h.chase = Some(rival);
                            h.chase_t = 0.0;
                            s.hands[k] = h;
                            continue;
                        }
                        h.target = rival;
                        h.phase = 1;
                        h.t = 0.0;
                        self.face(i, rival);
                        self.walk_of(i).gesture = true;
                    }
                }
                _ => {
                    if !s.players.contains(&h.target) {
                        h.phase = 0;
                        h.t = 0.0;
                    } else if h.t > 0.35 {
                        self.snow_throw(s, i, h.target);
                        h.ball = 0.0;
                        h.phase = 0;
                        // a breather, and often a step to one side
                        h.t = -(0.8 + self.walk.rand() * 1.6);
                        if self.walk.rand() < 0.5 {
                            let goal = (x + (self.walk.rand() - 0.5) * tall * 2.0).clamp(s.lo, s.hi);
                            self.steer(i, goal, 0.15);
                        }
                    }
                }
            }
            if self.arrived(i) {
                self.face(i, rival);
            }
            s.hands[k] = h;
        }
    }

    /// Run at: away from them flat out, tiring after a bit, to the end of
    /// the snow and no further; it slips past them now and then once
    /// cornered, and throws back whenever it has a ball and a moment.
    fn snow_flee(&mut self, c: &Chat, s: &mut Snow, h: &mut Thrower, after: &[usize], dt: f32) {
        let i = h.i;
        let x = self.strip_px(i);
        let tall = self.height_of(i);
        let near = after.iter().copied().min_by(|&a, &b| (self.strip_px(a) - x).abs().total_cmp(&(self.strip_px(b) - x).abs())).unwrap_or(after[0]);
        let nx = self.strip_px(near);
        if h.flee_t == 0.0 {
            // seen it coming: a startled hop, and off
            h.flee_t = 0.001;
            let wk = self.walk_of(i);
            wk.start_bob(tn().jump_height * 0.25);
            wk.smile = wk.smile.max(1.0);
            h.phase = 0;
        }
        h.flee_t += dt;
        if h.flee_t > 0.0 {
            h.flee_dir = if nx < x { 1.0 } else { -1.0 };
        }
        let margin = tall * 0.5;
        let wall = if h.flee_dir > 0.0 { s.hi + margin } else { s.lo - margin };
        let cornered = (wall - x) * h.flee_dir < margin * 0.5;
        let close = (nx - x).abs() < tall * 1.2;
        if cornered && close && h.flee_t > 0.6 && self.walk.rand() < dt * 1.5 {
            // a dash past them
            h.flee_dir = -h.flee_dir;
            h.flee_t = -0.7;
            log::debug!("chat {}: {i} slips past {near}", c.id);
        }
        let wall = if h.flee_dir > 0.0 { s.hi + margin } else { s.lo - margin };
        let pace = if h.flee_t.abs() < 1.2 { tn().snow_flee_speed } else { tn().snow_flee_speed * 0.65 };
        let step = x + h.flee_dir * tall * 1.5;
        let goal = if h.flee_dir > 0.0 { step.min(wall) } else { step.max(wall) };
        self.steer(i, goal, pace);
        // a new ball made on the run, slowly
        h.ball = (h.ball + dt / 1.8).min(1.0);
        if h.ball >= 1.0 && h.shot_t <= 0.0 && (nx - x).abs() < tall * 4.0 && grounded(self.walk_ref(i)) {
            self.snow_throw(s, i, near);
            h.ball = 0.0;
            h.shot_t = 1.2;
        }
    }

    fn snow_balls(&mut self, c: &mut Chat, s: &mut Snow, dt: f32) {
        let mut landed = Vec::new();
        for k in 0..s.balls.len() {
            let b = &mut s.balls[k];
            let before = b.t / b.dur;
            b.t += dt;
            let now = b.t / b.dur;
            // the one it's meant for sees it coming and hops out of the way
            if let Some(target) = b.target.filter(|_| b.dodge && before < 0.45 && now >= 0.45) {
                let wk = self.walk_of(target);
                if grounded(wk) && wk.flat <= 0.0 {
                    wk.start_jump(tn().jump_height * 0.5);
                }
            }
            if now >= 1.0 {
                landed.push(k);
            }
        }
        for &k in landed.iter().rev() {
            let b = s.balls.remove(k);
            let Some(target) = b.target else {
                // a lump of the giant one, landing in the snow
                s.puffs.push((b.to[0], b.to[1], 0.0, 0.25));
                continue;
            };
            let here = c.members.contains(&target);
            let close = here && (self.strip_px(target) - b.to[0]).abs() < self.width_of(target) * 0.6;
            if close && grounded(self.walk_ref(target)) {
                // a hit: a burst of snow, a smile, no hard feelings
                s.puffs.push((b.to[0], b.to[1], 0.0, 0.25));
                let wk = self.walk_of(target);
                wk.smile = wk.smile.max(0.7);
                wk.start_bob(tn().jump_height * 0.18);
                if let Some(sc) = c.scores.iter_mut().find(|sc| sc.0 == b.by) {
                    sc.1 += 1.0;
                }
                let wk = self.walk_of(b.by);
                wk.smile = wk.smile.max(0.6);
                log::debug!("chat {}: {} hits {}", c.id, b.by, target);
            } else {
                // missed: it comes down in the snow behind
                let dir = if b.to[0] >= b.from[0] { 1.0 } else { -1.0 };
                s.puffs.push((b.to[0] + dir * 40.0, s.ground - 6.0, 0.0, 0.2));
            }
        }
    }

    /// Now and then someone starts a giant snowball; others gather round to
    /// help while it grows.
    fn snow_start_roll(&mut self, c: &Chat, s: &mut Snow, dt: f32) {
        let chased: Vec<usize> = s.hands.iter().filter_map(|h| h.chase).collect();
        let free = |this: &Self, s: &Snow, i: usize| {
            !Self::crewing(s, i) && this.walk_ref(i).flat <= 0.0 && !chased.contains(&i) && s.hands.iter().any(|h| h.i == i && h.phase == 0 && h.chase.is_none())
        };
        if let Some(r) = s.roll.as_ref() {
            if r.phase <= 1 && r.crew.len() < 4 && self.walk.rand() < dt * tn().snow_help {
                let pool: Vec<usize> = s.players.iter().copied().filter(|&i| free(self, s, i)).collect();
                // the nearer ones are likelier to come
                let x = r.x;
                let mut best = None;
                let mut best_d = f32::MAX;
                for &i in &pool {
                    let d = (self.strip_px(i) - x).abs() * (0.5 + self.walk.rand());
                    if d < best_d {
                        best_d = d;
                        best = Some(i);
                    }
                }
                if let Some(i) = best {
                    if let Some(r) = s.roll.as_mut() {
                        r.crew.push(i);
                    }
                    log::debug!("chat {}: {i} comes to help roll the giant one", c.id);
                }
            }
            return;
        }
        if s.players.len() < tn().snow_roll_min as usize || self.walk.rand() >= dt * tn().snow_roll_rate {
            return;
        }
        let pool: Vec<usize> = s.players.iter().copied().filter(|&i| free(self, s, i)).collect();
        let Some(&i) = pool.get((self.walk.rand() * pool.len() as f32) as usize % pool.len().max(1)) else { return };
        let height = self.height_of(i);
        let dir = if self.walk.rand() < 0.5 { -1.0 } else { 1.0 };
        let x = (self.strip_px(i) + dir * self.width_of(i)).clamp(s.lo + height, (s.hi - height).max(s.lo + height));
        log::debug!("chat {}: {i} starts a giant snowball", c.id);
        s.roll = Some(Roll { crew: vec![i], x, size: height * 0.2, dir, phase: 0, t: 0.0, squashed: Vec::new() });
    }

    fn snow_roll(&mut self, c: &mut Chat, s: &mut Snow, dt: f32) {
        let Some(mut r) = s.roll.take() else { return };
        let Some(&first) = r.crew.first().or(c.members.first()) else { return };
        let height = self.height_of(first);
        let width = self.width_of(first);
        let biggest = height * tn().snow_big;
        let (lo, hi) = (s.lo + width, s.hi - width);
        // places behind the ball (`dir` the way it's pushed), one after the other
        let behind = |r: &Roll, k: usize, dir: f32| r.x - dir * (r.size * 0.5 + width * 0.3 + k as f32 * width * 0.45);
        r.t += dt;
        match r.phase {
            0 => {
                let mut there = true;
                for (k, &i) in r.crew.iter().enumerate() {
                    let at = behind(&r, k, r.dir);
                    if (self.strip_px(i) - at).abs() > 6.0 {
                        there = false;
                        self.steer(i, at, 0.2);
                    }
                }
                if there || r.t > 6.0 {
                    r.phase = 1;
                    r.t = 0.0;
                }
            }
            1 => {
                // heavier the bigger it gets, easier with more of them
                let crew = r.crew.len() as f32;
                let speed = tn().snow_roll_speed * (1.0 - 0.45 * (r.size / biggest).min(1.0)) * (0.8 + 0.2 * crew.min(3.0));
                r.x += r.dir * speed * dt;
                r.size = (r.size + speed * dt * tn().snow_grow).min(biggest);
                // helpers just come: they fall in behind as it goes
                self.push_along(&r.crew.clone(), r.dir, |k| behind(&r, k, r.dir), dt);
                if r.size >= biggest {
                    // big enough: let it go at the crowd, the side where there are more
                    let others: Vec<f32> = s.players.iter().filter(|&&i| !r.crew.contains(&i)).map(|&i| self.strip_px(i)).collect();
                    let right = others.iter().filter(|&&p| p > r.x).count();
                    let left = others.len() - right;
                    r.dir = if right == left { r.dir } else if right > left { 1.0 } else { -1.0 };
                    r.phase = 2;
                    r.t = 0.0;
                    self.free_crew(&r.crew);
                    log::debug!("chat {}: the giant snowball is ready", c.id);
                } else if (r.dir > 0.0 && r.x + r.size * 0.5 >= hi) || (r.dir < 0.0 && r.x - r.size * 0.5 <= lo) {
                    // the end of the snow: round to the other side and back
                    r.dir = -r.dir;
                    r.phase = 0;
                    r.t = 0.0;
                    self.free_crew(&r.crew);
                }
            }
            2 => {
                // round behind it for the big push, nobody waits for anybody
                let mut there = true;
                for (k, &i) in r.crew.iter().enumerate() {
                    let at = behind(&r, k, r.dir);
                    if (self.strip_px(i) - at).abs() > 6.0 {
                        there = false;
                        self.steer(i, at, 0.24);
                    } else {
                        self.face_x(i, r.x);
                    }
                }
                if (there && r.t > 0.6) || r.t > 4.0 {
                    log::debug!("chat {}: off goes the giant snowball", c.id);
                    for &i in &r.crew {
                        let wk = self.walk_of(i);
                        wk.start_bob(tn().jump_height * 0.3);
                        wk.smile = wk.smile.max(1.5);
                    }
                    // those it's heading for see it coming
                    for i in s.players.clone() {
                        if r.crew.contains(&i) {
                            continue;
                        }
                        let ahead = (self.strip_px(i) - r.x) * r.dir > 0.0;
                        let wk = self.walk_of(i);
                        if ahead && grounded(wk) {
                            wk.start_bob(tn().jump_height * 0.2);
                        }
                    }
                    r.phase = 3;
                    r.t = 0.0;
                }
            }
            _ => {
                // rolling at speed: everyone in its way goes flat, but its own crew
                r.x += r.dir * tn().snow_giant_speed * dt;
                for i in s.players.clone() {
                    if r.crew.contains(&i) || r.squashed.contains(&i) {
                        continue;
                    }
                    let wk = self.walk_ref(i);
                    if !grounded(wk) || wk.flat > 0.0 {
                        continue;
                    }
                    if (self.strip_px(i) - r.x).abs() < r.size * 0.4 {
                        log::debug!("chat {}: the giant snowball rolls over {i}", c.id);
                        r.squashed.push(i);
                        let wk = self.walk_of(i);
                        wk.flat = tn().snow_flat;
                        if let Some(sr) = wk.social.as_mut() {
                            sr.moving = false;
                            sr.face_lock = false;
                            sr.lean = 0.0;
                        }
                        let y = self.body_rect(i)[3] - height * 0.1;
                        s.puffs.push((self.strip_px(i), y, 0.0, 0.3));
                    }
                }
                let width_all = self.strip_width_of(first);
                let wall_lo = (s.lo - width * 2.0).max(r.size * 0.5);
                let wall_hi = (s.hi + width * 2.0).min(width_all - r.size * 0.5);
                if (r.dir > 0.0 && r.x >= wall_hi) || (r.dir < 0.0 && r.x <= wall_lo) || r.t > 8.0 {
                    self.burst_giant(c, s, &r);
                    self.free_crew(&r.crew);
                    return;
                }
            }
        }
        s.roll = Some(r);
    }

    /// Those pushing keep to their places behind it, leaning in.
    pub(super) fn push_along(&mut self, team: &[usize], dir: f32, at: impl Fn(usize) -> f32, dt: f32) {
        for (k, &i) in team.iter().enumerate() {
            self.steer(i, at(k), 0.14);
            let wk = self.walk_of(i);
            if wk.rand() < dt * 0.8 {
                wk.start_bob(tn().jump_height * 0.06);
            }
            if let Some(sr) = wk.social.as_mut() {
                sr.face_left = dir < 0.0;
                sr.face_lock = true;
                sr.lean = -dir * 0.14;
            }
        }
    }

    pub(super) fn free_crew(&mut self, team: &[usize]) {
        for &i in team {
            if let Some(sr) = self.walk_of(i).social.as_mut() {
                sr.face_lock = false;
                sr.lean = 0.0;
            }
        }
    }

    /// Against the wall: it bursts into lumps and a heap of snow.
    fn burst_giant(&mut self, c: &Chat, s: &mut Snow, r: &Roll) {
        log::debug!("chat {}: the giant snowball bursts", c.id);
        let y = s.ground - r.size * 0.5;
        for k in 0..6 {
            let a = k as f32 / 6.0 * std::f32::consts::TAU;
            s.puffs.push((r.x + a.cos() * r.size * 0.35, y + a.sin() * r.size * 0.3, -(k as f32) * 0.04, 0.6));
        }
        let by = r.crew.first().copied().unwrap_or(0);
        for _ in 0..8 {
            let to = [r.x - r.dir * (self.walk.rand() * r.size * 2.5), s.ground - 4.0];
            let dur = 0.5 + self.walk.rand() * 0.5;
            s.balls.push(Flying { by, target: None, from: [r.x, y], to, t: 0.0, dur, dodge: false, size: 0.6 + self.walk.rand() * 0.8 });
        }
        s.piles.push((r.x, 0.0, r.size * 1.3));
        // the ones it rolled over: up again in a moment, laughing it off
        for &i in &r.squashed {
            self.walk_of(i).smile = 2.0;
        }
        // the crew back in the fight, with a moment to catch their breath
        for &i in &r.crew {
            if let Some(h) = s.hands.iter_mut().find(|h| h.i == i) {
                h.phase = 0;
                h.t = -1.0;
                h.rival = None;
            }
        }
    }

    pub(super) fn face_x(&mut self, i: usize, x: f32) {
        let left = x < self.strip_px(i);
        if let Some(r) = self.walk_of(i).social.as_mut() {
            r.face_left = left;
        }
    }

    /// Snowballs in hand and in the air, bursts of snow, the giant one, and
    /// any snowmen standing about (with whatever's been put on them).
    pub(super) fn snow_props(&self, v: &mut Vec<(i32, [f32; 4], bool, usize)>) {
        for c in self.chats.iter().filter(|c| c.stage == Stage::Snow && c.step != 9) {
            let Some(s) = c.snow.as_ref() else { continue };
            let Some(&owner) = c.members.first() else { continue };
            let own = if owner == 0 { self.own_view } else { self.extras[owner - 1].own_view };
            let shift = own[0] * self.view_w * 0.5;
            let height = self.height_of(owner);
            let ball = height * 0.15;
            for &(px, age, wide) in &s.piles {
                let fade = (1.0 - age / PILE_LIFE).clamp(0.0, 1.0);
                let h = wide * 0.45 * fade.sqrt();
                let x = px - shift;
                v.push((PILE, [x - wide * 0.5, s.ground - h, x + wide * 0.5, s.ground + h * 0.1], false, owner));
            }
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
                let size = ball * 0.5 * b.size;
                v.push((SNOWBALL, [x - size, y - size, x + size, y + size], false, b.by));
            }
            for &(px, py, age, big) in &s.puffs {
                if age < 0.0 {
                    continue;
                }
                let k = age / PUFF_LIFE;
                let size = height * big * (0.5 + 0.7 * k);
                let x = px - shift;
                v.push((PUFF, [x - size, py - size, x + size, py + size], false, owner));
            }
            if let Some(r) = s.roll.as_ref() {
                let x = r.x - shift;
                let half = r.size * 0.5;
                let who = r.crew.first().copied().unwrap_or(owner);
                v.push((SNOWBALL, [x - half, s.ground - r.size, x + half, s.ground], false, who));
            }
        }
        let n = self.extras.len() + 1;
        let now = std::time::Instant::now();
        for m in SNOWMEN.lock().unwrap().iter().filter(|m| m.until > now) {
            // drawn with someone on its workspace (the picture follows their view)
            let Some(owner) = (0..n).find(|&i| self.walk_ref(i).ws == m.ws) else { continue };
            let own = if owner == 0 { self.own_view } else { self.extras[owner - 1].own_view };
            let x = m.x - own[0] * self.view_w * 0.5;
            Self::snowman_props(v, x, m.ground, m.size, m.deco, owner);
        }
        self.build_props(v);
    }

    /// A finished snowman at screen `x` on the floor at `ground`, its bottom
    /// ball `size` across, and whatever it's been given to wear.
    pub(super) fn snowman_props(v: &mut Vec<(i32, [f32; 4], bool, usize)>, x: f32, ground: f32, size: f32, deco: Option<(usize, super::super::wear::Anchor)>, owner: usize) {
        use super::super::wear::Anchor;
        // the shader's bottom ball is 0.42 of the box height, the box 0.6 as wide as tall
        let h = size / 0.42;
        let w = h * 0.6;
        v.push((SNOWMAN, [x - w * 0.5, ground - h, x + w * 0.5, ground], false, owner));
        let Some((k, anchor)) = deco else { return };
        // heights in the shader's terms (-1 at the floor, 1 at the top of the box)
        let at = |q: f32| ground - (q + 1.0) * h * 0.5;
        let (half, top, bottom) = match anchor {
            Anchor::Eyes => (size * 0.28, at(0.6), at(0.46)),
            Anchor::Neck => (size * 0.3, at(0.34), at(0.16)),
            _ => (size * 0.3, at(0.98), at(0.62)),
        };
        v.push((FOUND + k as i32, [x - half, top, x + half, bottom], false, owner));
    }
}
