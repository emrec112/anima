//! Races: now and then one character wants to race, the others catch the
//! mood one by one, gather at one end of a big window and set off together.

use crate::render::tuning::get as tn;
use super::CharacterOverlay;
use super::walk::{Act, WalkState};

/// Set over IPC (`anima race`): start a race now.
pub static RACE_NOW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Leg {
    // on the way to the gathering spot
    ToStart,
    // there, wandering about until everyone has come
    Mill,
    // not racing: off to the finish line, and cheering there
    Watch,
    Watching,
    // going round telling the others, and stopped to tell one
    Invite,
    Invited,
    // everyone's here: bunch up on the start line
    Line,
    Lined,
    Run,
    Done,
}

/// One character's part in a race. Goals are strip positions in logical px
/// (turned into walk offsets per character, they differ in size).
#[derive(Clone, Copy, Debug)]
pub(super) struct Racer {
    pub(super) leg: Leg,
    pub(super) goal_px: f32,
    pub(super) goal: f32,
    pub(super) speed: f32,
    // seconds until the next stroll while milling about
    pub(super) wait: f32,
    // said hello on reaching the gathering spot
    pub(super) greeted: bool,
    // running: the pace it's aiming for, seconds to the next change of
    // pace, and to the next try at hopping over the one ahead
    pub(super) pace: f32,
    pub(super) next_pace: f32,
    pub(super) hop_cool: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Gather,
    Line,
    Run,
    // at the finish: the winner hops in the middle of the crowd
    Cheer,
}

/// The race as a whole, run by the main character.
pub(super) struct Race {
    start_px: f32,
    finish_px: f32,
    // gathering spot: this far from the start line towards the finish
    zone: f32,
    phase: Phase,
    // seconds until the next character catches the racing mood
    next_join: f32,
    // all on the line: seconds to the off
    countdown: f32,
    winner: Option<usize>,
    // who came in, in order
    order: Vec<usize>,
    // the winner hops about this long
    cheer: f32,
    // seconds in this phase, to give up on stuck ones
    age: f32,
    // the one going round inviting the others, and its pause after each
    herald: Option<usize>,
    // said no when asked: they stay out of it
    refused: Vec<usize>,
    // who it has just asked, waiting for the answer
    asking: Option<usize>,
    invite_pause: f32,
}

impl CharacterOverlay {
    pub(super) fn walk_ref(&self, i: usize) -> &WalkState {
        if i == 0 { &self.walk } else { &self.extras[i - 1].walk }
    }

    pub(super) fn walk_of(&mut self, i: usize) -> &mut WalkState {
        if i == 0 { &mut self.walk } else { &mut self.extras[i - 1].walk }
    }

    /// Strip position of a character's centre, logical px.
    pub(super) fn strip_px(&self, i: usize) -> f32 {
        let r = if i == 0 { self.hit_rect } else { self.extras[i - 1].hit_rect };
        (r[0] + r[2]) * 0.5 + self.view[0] * self.view_w * 0.5
    }

    /// In on it: a little hop, and off to the gathering spot.
    fn recruit(&mut self, i: usize, lo: f32, hi: f32) {
        let wk = self.walk_of(i);
        let goal_px = lo + wk.rand() * (hi - lo);
        let speed = tn().race_gather_speed + wk.rand() * tn().race_gather_rand;
        wk.race = Some(Racer { leg: Leg::ToStart, goal_px, goal: 0.0, speed, wait: 0.0, greeted: false, pace: 0.0, next_pace: 0.0, hop_cool: 0.0 });
        wk.circle = false;
        wk.social = None;
        if matches!(wk.act, Act::Idle | Act::Walk) {
            wk.start_jump(tn().jump_height * 0.5);
        }
    }

    /// One racer goes to each of the others in turn, hops and invites them,
    /// and comes back when there's nobody left to tell.
    fn run_herald(&mut self, race: &mut Race, dt: f32, lo: f32, hi: f32, n: usize) {
        let free: Vec<usize> = (0..n).filter(|&i| self.walk_ref(i).climb.is_none() && self.walk_ref(i).race.is_none() && !race.refused.contains(&i)).collect();
        if race.herald.is_some_and(|h| self.walk_ref(h).race.is_none()) {
            race.herald = None;
        }
        if race.herald.is_none() {
            // whoever is closest to someone not in on it yet
            let near = |this: &Self, h: usize| free.iter().map(|&f| (this.strip_px(f) - this.strip_px(h)).abs()).fold(f32::MAX, f32::min);
            race.herald = (0..n)
                .filter(|&i| self.walk_ref(i).race.is_some_and(|r| !matches!(r.leg, Leg::Watch | Leg::Watching)))
                .filter(|_| !free.is_empty())
                .min_by(|&a, &b| near(self, a).total_cmp(&near(self, b)));
            if let Some(h) = race.herald {
                log::debug!("race: {h} goes round inviting {free:?}");
            }
        }
        let Some(h) = race.herald else { return };
        race.invite_pause -= dt;
        if race.invite_pause > 0.0 {
            return;
        }
        if let Some(t) = race.asking.take() {
            if self.walk_ref(t).race.is_none() {
                if n >= tn().race_refuse_min as usize && self.walk.rand() < tn().race_refuse {
                    // no thanks: straight to the finish to watch
                    log::debug!("race: {t} says no to {h}");
                    race.refused.push(t);
                    let goal_px = race.finish_px - (race.finish_px - race.start_px).signum() * (60.0 + self.walk.rand() * 240.0);
                    let wk = self.walk_of(t);
                    wk.race = Some(Racer { leg: Leg::Watch, goal_px, goal: 0.0, speed: tn().race_watch_speed, wait: 0.0, greeted: true, pace: 0.0, next_pace: 0.0, hop_cool: 0.0 });
                    wk.circle = false;
                    wk.social = None;
                } else {
                    log::debug!("race: {t} is in");
                    self.recruit(t, lo, hi);
                }
            }
            race.invite_pause = tn().race_invite_pause;
            return;
        }
        let x = self.strip_px(h);
        let Some(t) = free.iter().copied().min_by(|&a, &b| (self.strip_px(a) - x).abs().total_cmp(&(self.strip_px(b) - x).abs())) else {
            // everyone knows: back to the others
            log::debug!("race: {h} is done inviting");
            let wk = self.walk_of(h);
            let goal_px = lo + wk.rand() * (hi - lo);
            if let Some(r) = wk.race.as_mut() {
                r.leg = Leg::ToStart;
                r.goal_px = goal_px;
                r.speed = tn().race_herald_speed;
            }
            race.herald = None;
            return;
        };
        let tx = self.strip_px(t);
        let reach = self.width_of(h) * 0.8;
        let on_ground = matches!(self.walk_ref(h).act, Act::Idle | Act::Walk);
        if (tx - x).abs() <= reach && on_ground {
            // asks, and waits for the answer
            log::debug!("race: {h} asks {t}");
            let hop = tn().jump_height * 0.7;
            let wk = self.walk_of(h);
            wk.gesture = true;
            wk.left = tx < x;
            if let Some(r) = wk.race.as_mut() {
                r.leg = Leg::Invited;
            }
            wk.start_jump(hop);
            race.asking = Some(t);
            race.invite_pause = tn().race_ask_time;
        } else if let Some(r) = self.walk_of(h).race.as_mut() {
            r.leg = Leg::Invite;
            r.goal_px = tx + (x - tx).signum() * reach * 0.7;
            r.speed = tn().race_herald_speed;
        }
    }

    /// Very rarely one character wants to race; the mood spreads to the
    /// others one by one, they gather at one end of a big window and wander
    /// about there, and once all have come they bunch up and run to its
    /// other end.
    pub(super) fn run_race(&mut self, dt: f32, w: f32) {
        self.view_w = w;
        let n = self.extras.len() + 1;
        let asked = RACE_NOW.swap(false, std::sync::atomic::Ordering::Relaxed);
        if self.race.is_none() {
            // only ever by asking: a small group agrees on one in a chat
            self.race_quiet = (self.race_quiet - dt).max(0.0);
            if n < 2 || !asked {
                return;
            }
            // from one end of the strip to the other, whatever the windows
            let width = self.strip_width();
            let (a, b) = (tn().edge_px.min(width * 0.25), (width - tn().edge_px).max(width * 0.75));
            let len = b - a;
            let (start_px, finish_px) = if self.walk.rand() < 0.5 { (a, b) } else { (b, a) };
            log::debug!("race: from {start_px:.0} to {finish_px:.0}");
            self.race = Some(Race {
                start_px,
                finish_px,
                zone: (len * tn().race_zone).clamp(220.0, 600.0),
                phase: Phase::Gather,
                next_join: 0.0,
                countdown: tn().race_countdown,
                winner: None,
                order: Vec::new(),
                cheer: 0.0,
                age: 0.0,
                herald: None,
                refused: Vec::new(),
                asking: None,
                invite_pause: 0.0,
            });
        }
        let Some(mut race) = self.race.take() else { return };
        race.age += dt;
        let dir = (race.finish_px - race.start_px).signum();
        let (lo, hi) = {
            let far = race.start_px + dir * race.zone;
            (race.start_px.min(far), race.start_px.max(far))
        };
        let grounded = |wk: &WalkState| matches!(wk.act, Act::Idle | Act::Walk);
        let mut over = false;

        match race.phase {
            Phase::Gather => {
                // the group that decided on it goes first, in a quick wave
                race.next_join -= dt;
                let mut crew = std::mem::take(&mut self.race_crew);
                crew.retain(|&i| i < n && self.walk_ref(i).race.is_none());
                if race.next_join <= 0.0 {
                    let started = (0..n).any(|i| self.walk_ref(i).race.is_some());
                    let pick = if !crew.is_empty() {
                        Some(crew.remove(0))
                    } else if !started {
                        Some((self.walk.rand() * n as f32) as usize % n)
                    } else {
                        None
                    };
                    if let Some(i) = pick {
                        self.recruit(i, lo, hi);
                    }
                    race.next_join = tn().race_wave_min + self.walk.rand() * tn().race_wave_rand;
                }
                let waved = crew.is_empty();
                self.race_crew = crew;
                // then one of them goes round everybody else
                if waved {
                    self.run_herald(&mut race, dt, lo, hi, n);
                }
                for i in 0..n {
                    let x = self.strip_px(i);
                    let wk = self.walk_of(i);
                    let Some(mut r) = wk.race else { continue };
                    // stepped into the gathering spot: "I'm here!"
                    if !r.greeted && x >= lo && x <= hi {
                        r.greeted = true;
                        if grounded(wk) {
                            wk.start_jump(tn().jump_height * 0.6);
                        }
                    }
                    // wander about the spot
                    if r.leg == Leg::Mill {
                        r.wait -= dt;
                        if r.wait <= 0.0 {
                            r.goal_px = lo + wk.rand() * (hi - lo);
                            r.speed = tn().race_mill_speed + wk.rand() * tn().race_mill_rand;
                            r.wait = 1.5 + wk.rand() * 2.5;
                        }
                    }
                    wk.race = Some(r);
                }
                let here = (0..n)
                    .filter(|&i| {
                        let wk = self.walk_ref(i);
                        wk.race.is_some_and(|r| r.leg == Leg::Mill) && grounded(wk)
                    })
                    .count();
                // stragglers far along the strip don't hold it up forever
                if here + race.refused.len() == n || (race.age > tn().race_wait && here >= 2 && race.herald.is_none()) {
                    let (start, finish) = (race.start_px, race.finish_px);
                    for i in 0..n {
                        let wk = self.walk_of(i);
                        match wk.race.as_mut() {
                            Some(r) if r.leg == Leg::Mill => {
                                r.leg = Leg::Line;
                                r.goal_px = start;
                                r.speed = 0.2;
                            }
                            Some(r) if matches!(r.leg, Leg::Watch | Leg::Watching) => {}
                            // not in it: they go and cheer at the finish
                            _ => {
                                let goal_px = finish - dir * (60.0 + wk.rand() * 240.0);
                                wk.race = Some(Racer { leg: Leg::Watch, goal_px, goal: 0.0, speed: tn().race_watch_speed, wait: 0.0, greeted: true, pace: 0.0, next_pace: 0.0, hop_cool: 0.0 });
                                wk.circle = false;
                                wk.social = None;
                            }
                        }
                    }
                    self.race_crew.clear();
                    race.phase = Phase::Line;
                    race.age = 0.0;
                }
            }
            Phase::Line => {
                let lined = (0..n).all(|i| {
                    let wk = self.walk_ref(i);
                    wk.race.is_none_or(|r| matches!(r.leg, Leg::Watch | Leg::Watching) || (r.leg == Leg::Lined && grounded(wk)))
                });
                if lined {
                    race.countdown -= dt;
                }
                if race.countdown <= 0.0 {
                    // off they go, all at once; starting paces spread evenly
                    // (shuffled) so the bunch pulls apart
                    let mut runners: Vec<usize> = (0..n).filter(|&i| self.walk_ref(i).race.is_some_and(|r| !matches!(r.leg, Leg::Watch | Leg::Watching))).collect();
                    for k in (1..runners.len()).rev() {
                        let j = (self.walk.rand() * (k + 1) as f32) as usize % (k + 1);
                        runners.swap(k, j);
                    }
                    let m = runners.len().max(2) as f32 - 1.0;
                    for (k, &i) in runners.iter().enumerate() {
                        let finish = race.finish_px;
                        let wk = self.walk_of(i);
                        let pace = tn().race_pace + k as f32 / m * tn().race_pace_range;
                        let next_pace = 1.0 + wk.rand() * 1.5;
                        if let Some(r) = wk.race.as_mut() {
                            r.leg = Leg::Run;
                            r.goal_px = finish;
                            r.speed = pace;
                            r.pace = pace;
                            r.next_pace = next_pace;
                            r.hop_cool = 0.0;
                        }
                    }
                    race.phase = Phase::Run;
                    race.age = 0.0;
                }
                over = race.age > 60.0;
            }
            Phase::Run => {
                // where everyone is along the track, and how fast
                let width = (self.hit_rect[2] - self.hit_rect[0]).max(1.0);
                let field: Vec<(usize, f32, f32)> = (0..n)
                    .filter_map(|i| {
                        let r = self.walk_ref(i).race?;
                        (r.leg == Leg::Run).then(|| (i, self.strip_px(i) * dir, r.speed))
                    })
                    .collect();
                let settle = race.age > 1.0;
                let mut hits = Vec::new();
                for &(i, p, _) in &field {
                    let ahead = field.iter().filter(|o| o.1 > p).min_by(|a, b| a.1.total_cmp(&b.1)).copied();
                    let lead = field.iter().filter(|o| o.1 > p).count();
                    let wk = self.walk_of(i);
                    let Some(mut r) = wk.race else { continue };
                    // bursts and lulls
                    r.next_pace -= dt;
                    if r.next_pace <= 0.0 {
                        r.pace = tn().race_pace + wk.rand() * tn().race_pace_range;
                        r.next_pace = tn().race_pace_gap + wk.rand() * 1.7;
                    }
                    r.hop_cool -= dt;
                    let mut want = r.pace;
                    // right on someone's heels: hop over them, or ease off
                    if let Some((victim, q, their)) = ahead.filter(|a| settle && a.1 - p < width * 0.45) {
                        if r.hop_cool <= 0.0 {
                            r.hop_cool = 1.5;
                            // only the front runners fight for it by hopping over
                            if wk.rand() < tn().race_hit_chance && grounded(wk) {
                                hits.push(victim);
                            } else if lead < tn().race_leaders as usize && wk.rand() < tn().race_hop_chance && grounded(wk) {
                                wk.start_jump(tn().jump_height * 0.55);
                                r.pace = r.pace.max(their + 0.06);
                                r.speed = r.pace;
                            }
                        }
                        if wk.act != Act::Jump && q - p < width * 0.3 {
                            want = want.min(their - 0.02);
                        }
                    }
                    r.speed = (r.speed + (want - r.speed) * (dt * 3.0).min(1.0)).clamp(tn().race_pace * 0.75, tn().race_pace + tn().race_pace_range * 1.6);
                    wk.race = Some(r);
                }
                // whoever got hit goes tumbling back the way they came
                for v in hits {
                    let wk = self.walk_of(v);
                    if !grounded(wk) {
                        continue;
                    }
                    let distance = tn().race_hit_min + wk.rand() * tn().race_hit_rand;
                    let time = 0.5 + wk.rand() * 0.3;
                    log::debug!("race: {v} got hit");
                    wk.race_down = true;
                    wk.vx = -dir * (distance * 2.0 / w.max(1.0)) / time;
                    wk.vy = 20.0 * time * 0.7;
                    wk.air = 0.1;
                    wk.jump = 0.0;
                    wk.held_prev = None;
                    wk.act = Act::Fall;
                    wk.tumble = -dir * (6.0 + wk.rand() * 8.0);
                }
                for i in 0..n {
                    if self.walk_ref(i).race.is_some_and(|r| r.leg == Leg::Done) && !race.order.contains(&i) {
                        race.order.push(i);
                        if race.winner.is_none() {
                            race.winner = Some(i);
                            let wk = self.walk_of(i);
                            if grounded(wk) {
                                wk.start_jump(tn().jump_height * 1.2);
                            }
                        }
                    }
                }
                let all_done = (0..n).all(|i| self.walk_ref(i).race.is_none_or(|r| matches!(r.leg, Leg::Done | Leg::Watch | Leg::Watching)));
                if all_done && race.winner.is_some() {
                    // gather round: the winner in the middle, the rest on both
                    // sides in finishing order, half overlapping like a crowd
                    let spacing = width * 0.5;
                    let m = race.order.len() as f32;
                    let centre = race.finish_px - dir * spacing * (m * 0.5).floor();
                    for (k, &i) in race.order.clone().iter().enumerate() {
                        let wk = self.walk_of(i);
                        let side = if k % 2 == 1 { 1.0 } else { -1.0 };
                        let slot = k.div_ceil(2) as f32;
                        let jitter = (wk.rand() - 0.5) * spacing * 0.3;
                        let goal_px = if k == 0 { centre } else { centre + side * slot * spacing + jitter };
                        if let Some(r) = wk.race.as_mut() {
                            r.leg = Leg::Line;
                            r.goal_px = goal_px;
                            r.speed = 0.45;
                        }
                    }
                    race.phase = Phase::Cheer;
                    race.cheer = tn().race_cheer;
                } else {
                    over = race.age > 180.0;
                }
            }
            Phase::Cheer => {
                race.cheer -= dt;
                if let Some(i) = race.winner {
                    let wk = self.walk_of(i);
                    if race.cheer > 0.0 && grounded(wk) && wk.race.is_some_and(|r| r.leg == Leg::Lined) {
                        wk.start_jump(tn().jump_height * 0.9);
                    }
                }
                over = race.cheer <= 0.0;
            }
        }
        // the crowd at the finish, facing the runners
        for i in 0..n {
            let wk = self.walk_of(i);
            if wk.race.is_some_and(|r| r.leg == Leg::Watching) && grounded(wk) {
                wk.left = dir > 0.0;
                if wk.rand() < dt * tn().race_watch_cheer {
                    let height = tn().jump_height * (0.3 + wk.rand() * 0.5);
                    wk.start_jump(height);
                }
            }
        }
        if over || race.age > 300.0 {
            log::debug!("race: over, winner {:?}", race.winner);
            self.race_quiet = tn().race_cool;
            for i in 0..n {
                let wk = self.walk_of(i);
                let raced = wk.race.take().is_some();
                wk.race_down = false;
                if raced {
                    wk.crowd_rest = tn().crowd_rest_base + wk.rand() * tn().crowd_rest_rand;
                }
                if raced && Some(i) != race.winner && grounded(wk) {
                    wk.start_jump(tn().jump_height * 0.4);
                }
            }
            return;
        }
        self.race = Some(race);
    }
}

impl WalkState {
    /// A racer on the ground: walk to the goal the race gives, stand about
    /// once there. None when not racing (or busy jumping, held, ...).
    pub(super) fn race_step(&mut self, dt: f32) -> Option<(f32, bool, f32)> {
        if let Some(r) = self.race.as_mut().filter(|_| matches!(self.act, Act::Idle | Act::Walk | Act::Chase)) {
            // carried off the start line: back to it
            if r.leg == Leg::Lined && (r.goal - self.x).abs() > 0.02 {
                r.leg = Leg::Line;
            }
            // knocked down: picked itself up, starts again from a jog
            if std::mem::take(&mut self.race_down) {
                r.speed = tn().race_pace * 0.75;
            }
            let walking = matches!(r.leg, Leg::ToStart | Leg::Mill | Leg::Watch | Leg::Invite | Leg::Line | Leg::Run);
            let d = r.goal - self.x;
            let s = r.speed * dt;
            if walking && d.abs() > s {
                self.x += d.signum() * s;
                self.left = d < 0.0;
                self.target = r.goal;
                self.act = Act::Walk;
                return Some((self.x, true, 0.0));
            }
            if walking {
                self.x = r.goal;
                r.leg = match r.leg {
                    Leg::ToStart => Leg::Mill,
                    Leg::Invite => Leg::Invite,
                    Leg::Watch => Leg::Watching,
                    Leg::Line => Leg::Lined,
                    Leg::Run => Leg::Done,
                    l => l,
                };
            }
            self.act = Act::Idle;
            self.timer = 1.0;
            self.target = self.x;
            return Some((self.x, false, 0.0));
        }
        None
    }

    /// Hopping over someone mid-race: keep going.
    pub(super) fn race_hop(&mut self, dt: f32) {
        // getting up from a fall isn't running
        if self.race_down {
            return;
        }
        if let Some(r) = self.race.as_mut().filter(|r| r.leg == Leg::Run) {
            let d = r.goal - self.x;
            let s = r.speed * dt;
            if d.abs() <= s {
                self.x = r.goal;
                r.leg = Leg::Done;
            } else {
                self.x += d.signum() * s;
                self.left = d < 0.0;
            }
        }
    }
}
