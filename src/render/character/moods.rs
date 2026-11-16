//! The desktop's mood rubbing off: dozing off when nobody's around, sitting
//! down when there are too many windows, fretting when the processor is
//! flat out, going to look at new windows. Friendships and rivalries, kept
//! between runs, are here too.

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI, TAU};
use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;

use crate::backend::desktop;
use crate::render::tuning::get as tn;
use super::CharacterOverlay;
use super::pat::PatCrowd;
use super::walk::{Act, WalkState};

const YAWN: f32 = 1.3;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum RestKind {
    Sleep,
    Sit,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Rest {
    pub(super) kind: RestKind,
    t: f32,
    until: f32,
    // woken: seconds until it's up
    wake: Option<f32>,
    // which way it lies
    turn: f32,
}

impl Rest {
    fn snoring(&self) -> bool {
        self.kind == RestKind::Sleep && self.wake.is_none() && self.t >= YAWN
    }
}

/// Cute indignant hops at someone who got away with something.
#[derive(Clone, Copy, Debug)]
pub(super) struct Protest {
    who: usize,
    at: usize,
    hops: u8,
    wait: f32,
}

/// A bit of carrot: flying off (strip x, screen y, px) or stuck round a mouth.
#[derive(Clone, Copy, Debug)]
pub(super) struct Crumb {
    owner: usize,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
    // stuck to the owner's face: offset from its middle, in heights
    stuck: Option<[f32; 2]>,
    size: f32,
}

// crumbs move in little steps, like hand-drawn frames
const CRUMB_FPS: f32 = 12.0;

/// Something that fell out of a character's hand: tumbling down, bouncing,
/// lying on the floor till its owner comes for it.
#[derive(Clone, Debug)]
pub(super) struct Loose {
    // what it was, to have it back, and whose: by name, since places in the
    // team move when characters are added or taken off
    pub(super) id: String,
    owner_name: String,
    helper_name: Option<String>,
    pub(super) kind: i32,
    // a picture of its own (a wardrobe picture), else the props folder's for its kind
    pub(super) image: Option<std::path::PathBuf>,
    pub(super) owner: usize,
    // strip x and screen y of its middle, px; speed px/s
    pub(super) x: f32,
    pub(super) y: f32,
    vx: f32,
    vy: f32,
    pub(super) rot: f32,
    spin: f32,
    floor: f32,
    pub(super) size: [f32; 2],
    // seconds lying still
    still: f32,
    // a friend fetching it for its owner, and whether it has it in hand yet
    helper: Option<usize>,
    carried: bool,
}

#[derive(Default)]
pub(super) struct Moods {
    pub(super) protests: Vec<Protest>,
    crumbs: Vec<Crumb>,
    // one-off flashes: kind, owner, strip x, screen y, size, seconds left
    pub(super) effects: Vec<(i32, usize, f32, f32, f32, f32)>,
    // things dropped: an umbrella shaken out of the hand
    pub(super) loose: Vec<Loose>,
    crumb_tick: f32,
    idle_t: f32,
    pub(super) fretting: bool,
    seen_opened: u32,
    seen_closed: u32,
    started: bool,
    // the weather as last seen, to notice the night coming and going
    pub(super) weather: super::weather::Weather,
    // a new window, and how long until its place is known
    pending: Option<(Option<u64>, f32)>,
    pub(super) bonds: Bonds,
    pub(super) skills: Skills,
    pub(super) keepsakes: Keepsakes,
    pub(super) bond_tick: f32,
    // the day "good morning" was last said, kept on disk across restarts
    greeted: Option<String>,
    morning_t: f32,
    // a birthday: next look, and (birthday one, sign holder, seconds left)
    bday_t: f32,
    bday: Option<(usize, usize, f32)>,
}

impl WalkState {
    pub(super) fn can_hold_sign(&self) -> bool {
        !self.fret && matches!(self.act, Act::Idle | Act::Walk) && self.social.is_none() && self.race.is_none() && self.crowd.is_none() && !self.circle
    }

    pub(super) fn at_leisure(&self) -> bool {
        self.can_hold_sign() && self.sign.is_none()
    }

    /// `until`: a nap of that many seconds; 0 sleeps until someone's back.
    pub(super) fn lie_down(&mut self, until: f32) {
        let turn = if self.rand() < 0.5 { FRAC_PI_2 } else { -FRAC_PI_2 };
        self.rest = Some(Rest { kind: RestKind::Sleep, t: 0.0, until, wake: None, turn });
        self.act = Act::Rest;
        self.target = self.x;
    }

    fn sit_down(&mut self) {
        let until = tn().sit_min + self.rand() * tn().sit_rand;
        self.rest = Some(Rest { kind: RestKind::Sit, t: 0.0, until, wake: None, turn: 0.0 });
        self.act = Act::Rest;
        self.target = self.x;
    }

    pub(super) fn get_up(&mut self) {
        self.night_sleep = false;
        self.rest = None;
        self.act = Act::Idle;
        self.timer = 0.5 + self.rand();
    }

    /// `idle`: nobody touching anything. `drowsy`: for long enough to doze off.
    pub(super) fn rest_tick(&mut self, dt: f32, idle: bool, drowsy: bool, crowded: bool) {
        if self.fret {
            self.sleep_wait = None;
            if self.act == Act::Rest {
                self.get_up();
            }
            return;
        }
        if self.act != Act::Rest {
            self.rest = None;
            if !drowsy {
                self.sleep_wait = None;
            } else if self.at_leisure() {
                let wait = match self.sleep_wait {
                    Some(w) => w - dt,
                    None => self.rand() * tn().sleep_spread,
                };
                self.sleep_wait = Some(wait);
                if wait <= 0.0 {
                    self.sleep_wait = None;
                    self.lie_down(0.0);
                    return;
                }
            }
            // played itself out: its social battery is flat
            self.played = (self.played - dt / 600.0).max(0.0);
            if self.at_leisure() && self.played >= tn().nap_games && self.energy <= tn().nap_energy && self.rand() < dt / tn().nap_wait.max(0.1) {
                self.played = 0.0;
                let until = tn().nap_min + self.rand() * tn().nap_rand;
                self.lie_down(until);
                return;
            }
            if crowded && !drowsy && self.at_leisure() && self.rand() < dt / tn().sit_every.max(1.0) {
                self.sit_down();
            }
            return;
        }
        let Some(mut r) = self.rest else {
            self.act = Act::Idle;
            return;
        };
        r.t += dt;
        match (r.kind, r.wake) {
            (RestKind::Sit, _) => {
                if r.t >= r.until {
                    self.get_up();
                    return;
                }
            }
            (RestKind::Sleep, Some(w)) => {
                if w - dt <= 0.0 {
                    self.get_up();
                    return;
                }
                r.wake = Some(w - dt);
            }
            (RestKind::Sleep, None) if r.until > 0.0 && r.t < r.until => {
                self.energy = (self.energy + dt / r.until).min(1.0);
            }
            // gone to bed for the night: up when the night is over, not before
            (RestKind::Sleep, None) if self.night_sleep => {}
            (RestKind::Sleep, None) if r.until > 0.0 || !idle => {
                // some jump up with a start, the rest take their time
                if self.rand() < tn().wake_jump {
                    self.get_up();
                    self.start_jump(tn().jump_height * 1.4);
                    return;
                }
                r.wake = Some(tn().wake_min + self.rand() * tn().wake_rand);
                r.t = 0.0;
            }
            _ => {}
        }
        self.rest = Some(r);
    }

    pub(super) fn rest_pose(&mut self, dt: f32) {
        let Some(r) = self.rest else { return };
        let (rot, sink) = match (r.kind, r.wake) {
            (RestKind::Sit, _) => (0.0, tn().sit_sink),
            // rubbing its eyes
            (RestKind::Sleep, Some(_)) => ((r.t * 9.0).sin() * 0.07, 0.0),
            (RestKind::Sleep, None) if r.t < YAWN => ((r.t * 3.5).sin() * 0.1, 0.0),
            (RestKind::Sleep, None) => (r.turn, tn().sleep_sink),
        };
        let k = (dt * 5.0).min(1.0);
        self.rot += ((rot - self.rot + PI).rem_euclid(TAU) - PI) * k;
        self.sink += (sink - self.sink) * k;
        self.feet = false;
    }
}

impl CharacterOverlay {
    pub(super) fn protest(&mut self, who: usize, at: usize, hops: u8, delay: f32) {
        if who != at && !self.moods.protests.iter().any(|p| p.who == who) {
            self.moods.protests.push(Protest { who, at, hops, wait: delay });
        }
    }

    fn actor_rect(&self, i: usize) -> [f32; 4] {
        if i == 0 { self.hit_rect } else { self.extras[i - 1].hit_rect }
    }

    /// Where `i`'s mouth is on screen, and which way it faces (1 right).
    fn mouth(&self, i: usize) -> ([f32; 2], f32) {
        let r = self.actor_rect(i);
        let h = r[3] - r[1];
        let side = if self.walk_ref(i).left { -1.0 } else { 1.0 };
        ([(r[0] + r[2]) * 0.5 + side * h * 0.1, r[1] + h * 0.4], side)
    }

    /// Bits of carrot spraying out of `i`'s mouth.
    pub(super) fn spray_crumbs(&mut self, i: usize, count: usize) {
        let (m, side) = self.mouth(i);
        let h = (self.actor_rect(i)[3] - self.actor_rect(i)[1]).max(1.0);
        let x = self.strip_px(i) + (m[0] - (self.actor_rect(i)[0] + self.actor_rect(i)[2]) * 0.5);
        for _ in 0..count {
            let wk = self.walk_of(i);
            let (a, b, c) = (wk.rand(), wk.rand(), wk.rand());
            self.moods.crumbs.push(Crumb {
                owner: i,
                x,
                y: m[1],
                vx: side * (40.0 + a * 220.0) * if b < 0.25 { -0.5 } else { 1.0 },
                vy: -(120.0 + b * 260.0),
                life: 0.9 + c * 0.8,
                stuck: None,
                size: h * (0.035 + c * 0.03),
            });
        }
    }

    /// A few bits left stuck round the mouth for a couple of seconds.
    pub(super) fn smear_crumbs(&mut self, i: usize, count: usize) {
        let (_, side) = self.mouth(i);
        for _ in 0..count {
            let wk = self.walk_of(i);
            let (a, b, c) = (wk.rand(), wk.rand(), wk.rand());
            self.moods.crumbs.push(Crumb {
                owner: i,
                x: 0.0,
                y: 0.0,
                vx: 0.0,
                vy: 0.0,
                life: 2.0 + a,
                stuck: Some([side * (0.04 + b * 0.12), 0.12 + c * 0.08]),
                size: 0.06 + a * 0.03,
            });
        }
    }

    fn run_crumbs(&mut self, dt: f32, n: usize) {
        self.moods.crumb_tick += dt;
        let step = 1.0 / CRUMB_FPS;
        while self.moods.crumb_tick >= step {
            self.moods.crumb_tick -= step;
            let floors: Vec<f32> = (0..n).map(|i| self.actor_rect(i)[3]).collect();
            for c in &mut self.moods.crumbs {
                c.life -= step;
                if c.stuck.is_some() {
                    continue;
                }
                let floor = floors.get(c.owner).copied().unwrap_or(f32::MAX);
                c.vy += 1100.0 * step;
                c.x += c.vx * step;
                c.y += c.vy * step;
                if c.y >= floor {
                    c.y = floor;
                    c.vy = -c.vy * 0.25;
                    c.vx *= 0.5;
                }
            }
            self.moods.crumbs.retain(|c| c.life > 0.0 && c.owner < n);
        }
    }

    /// Laughter is catching: friends nearby join in for a moment.
    fn spread_laughter(&mut self, n: usize) {
        for i in 0..n {
            if !std::mem::take(&mut self.walk_of(i).giggle_new) {
                continue;
            }
            let (x, ws) = (self.strip_px(i), self.walk_ref(i).ws);
            let chat = self.walk_ref(i).social.map(|s| s.chat);
            for j in 0..n {
                if j == i || self.walk_ref(j).ws != ws || (self.strip_px(j) - x).abs() > self.view_w * 0.35 {
                    continue;
                }
                let wk = self.walk_ref(j);
                // only those it's with: a passer-by doesn't laugh along
                let along = chat.is_some() && wk.social.map(|s| s.chat) == chat;
                if !along || self.bond(i, j).0 < tn().meet_friend || wk.giggle > 0.0 {
                    continue;
                }
                let wk = self.walk_of(j);
                if wk.rand() < tn().laugh_spread {
                    let secs = tn().laugh_time * (0.3 + wk.rand() * 0.3);
                    wk.laugh(secs);
                    wk.giggle_new = false;
                }
            }
        }
    }


    fn run_protests(&mut self, dt: f32, n: usize) {
        let mut list = std::mem::take(&mut self.moods.protests);
        list.retain_mut(|p| {
            if p.who >= n || p.at >= n {
                return false;
            }
            let left = self.strip_px(p.at) < self.strip_px(p.who);
            let wk = self.walk_of(p.who);
            if let Some(r) = wk.social.as_mut() {
                r.face_left = left;
            }
            if wk.act == Act::Idle {
                wk.left = left;
            }
            p.wait -= dt;
            if p.wait <= 0.0 && wk.act == Act::Idle {
                wk.start_bob(tn().jump_height * 0.35);
                p.hops = p.hops.saturating_sub(1);
                p.wait = 0.3 + wk.rand() * 0.15;
            }
            p.hops > 0
        });
        list.append(&mut self.moods.protests);
        self.moods.protests = list;
    }

    pub(super) fn run_moods(&mut self, dt: f32, n: usize) {
        self.run_protests(dt, n);
        self.spread_laughter(n);
        self.run_crumbs(dt, n);
        self.moods.effects.retain_mut(|e| {
            e.5 -= dt;
            e.5 > 0.0
        });
        if !self.moods.started {
            self.moods.started = true;
            desktop::spawn();
            super::weather::set_place(&self.config.weather_place);
            super::weather::spawn();
            self.moods.seen_opened = desktop::OPENED.load(Relaxed);
            self.moods.seen_closed = desktop::CLOSED.load(Relaxed);
        }
        // on the lock screen nobody's touching anything, but they're on show
        let idle = desktop::IDLE.load(Relaxed) && !super::locked();
        self.moods.idle_t = if idle { self.moods.idle_t + dt } else { 0.0 };
        let waited = self.moods.idle_t + desktop::IDLE_AFTER_MS as f32 / 1000.0;
        let drowsy = idle && waited >= tn().sleep_after;
        let crowded = crate::backend::compositor::current().columns().len() >= tn().win_crowded as usize;
        for i in 0..n {
            self.walk_of(i).rest_tick(dt, idle, drowsy, crowded);
        }
        self.run_weather(dt, n);
        self.run_greetings(dt, n, idle);
        self.run_loose(dt, n);

        let opened = desktop::OPENED.load(Relaxed);
        if opened != self.moods.seen_opened {
            self.moods.seen_opened = opened;
            let id = *desktop::LAST_OPENED.lock().unwrap();
            // the compositor lists its layout a moment later
            self.moods.pending = Some((id, 0.5));
        }
        if let Some((id, t)) = self.moods.pending {
            if t - dt > 0.0 {
                self.moods.pending = Some((id, t - dt));
            } else {
                self.moods.pending = None;
                let view = self.view[0] * self.view_w * 0.5;
                let (x, w) = id
                    .and_then(|id| crate::backend::compositor::current().window_x(id))
                    .unwrap_or((view + self.view_w * 0.25, self.view_w * 0.5));
                self.peek_at(x + w * 0.5, w);
            }
        }
        let closed = desktop::CLOSED.load(Relaxed);
        if closed != self.moods.seen_closed {
            self.moods.seen_closed = closed;
            for i in self.few_at_leisure(n) {
                let wk = self.walk_of(i);
                wk.notify_jump = Some(wk.rand() * 0.5);
            }
        }
    }

    /// "Good morning" once a day, the first time someone's at the computer in
    /// the morning, and a little gathering on a birthday.
    fn run_greetings(&mut self, dt: f32, n: usize, idle: bool) {
        let (y, m, d) = super::weather::today();
        let minutes = super::weather::minutes_now();
        if (300..720).contains(&minutes) && !idle && !super::locked() {
            let day = format!("{y}-{m:02}-{d:02}");
            let path = crate::util::data_dir().join("greeted");
            let greeted = self.moods.greeted.get_or_insert_with(|| std::fs::read_to_string(&path).unwrap_or_default().trim().to_string());
            if *greeted != day {
                // a few seconds in, so it's seen
                self.moods.morning_t += dt;
                if self.moods.morning_t > 5.0 {
                    if let Some(i) = self.free_here(n).first().copied() {
                        self.hold_sign(i, super::sign::MORNING);
                        self.moods.greeted = Some(day.clone());
                        let _ = std::fs::write(&path, day);
                    }
                }
            }
        }
        self.moods.bday_t -= dt;
        if self.moods.bday.is_none() && self.moods.bday_t <= 0.0 {
            self.moods.bday_t = 480.0 + self.walk.rand() * 420.0;
            let here = self.spaces.active;
            let b = (0..n).find(|&i| {
                let wk = self.walk_ref(i);
                (here == 0 || wk.ws == here) && wk.at_leisure() && wk.act == Act::Idle && super::uma::birthday(&self.character_name(i), m, d)
            });
            if let Some(b) = b {
                self.start_birthday(b, n);
            }
        }
        if let Some((b, holder, left)) = self.moods.bday {
            let left = left - dt;
            self.moods.bday = (left > 0.0 && b < n && holder < n).then_some((b, holder, left));
            if self.moods.bday.is_some() {
                // the one bringing the sign holds it up once beside her
                let near = (self.strip_px(holder) - self.strip_px(b)).abs() < self.actor_w(b) * 1.6;
                if near && self.walk_ref(holder).act == Act::Idle && self.walk_ref(holder).at_leisure() {
                    self.hold_sign(holder, super::sign::BIRTHDAY);
                    self.moods.bday = None;
                }
            }
        }
    }

    /// Birthday: a few friends nearby walk over to `b`, hop for her, and one
    /// holds up "happy birthday".
    pub(super) fn start_birthday(&mut self, b: usize, n: usize) {
        let mut friends: Vec<usize> = self.free_here(n).into_iter().filter(|&i| i != b).collect();
        friends.sort_by(|&a, &c| self.bond(a, b).0.total_cmp(&self.bond(c, b).0).reverse());
        friends.truncate(3);
        let Some(&holder) = friends.first() else { return };
        let (bx, w) = (self.strip_px(b), self.actor_w(b));
        let wk = self.walk_of(b);
        wk.act = Act::Idle;
        wk.target = wk.x;
        wk.timer = wk.timer.max(20.0);
        for (k, &f) in friends.iter().enumerate() {
            let side = if self.strip_px(f) < bx { -1.0 } else { 1.0 };
            let x = bx + side * w * (0.9 + 0.8 * (k / 2) as f32);
            let walk = (self.strip_px(f) - x).abs() / w.max(1.0) * 0.6;
            self.walk_toward(f, x);
            self.protest(f, b, 4, walk + 0.5);
        }
        self.moods.bday = Some((b, holder, 30.0));
    }

    /// Stands `i` still with sign `k` held up for 20-30 s.
    fn hold_sign(&mut self, i: usize, k: usize) {
        let wk = self.walk_of(i);
        wk.act = Act::Idle;
        wk.target = wk.x;
        wk.timer = wk.timer.max(0.5);
        wk.sign = Some(k);
        wk.sign_left = 20.0 + wk.rand() * 10.0;
        wk.sign_t = wk.sign_t.max(120.0);
    }

    /// Those free on the workspace in use, nearest the middle of the screen first.
    pub(super) fn free_here_pub(&mut self, n: usize) -> Vec<usize> {
        self.free_here(n)
    }

    fn free_here(&mut self, n: usize) -> Vec<usize> {
        let here = self.spaces.active;
        let middle = self.view[0] * self.view_w * 0.5 + self.view_w * 0.5;
        let mut free: Vec<usize> = (0..n)
            .filter(|&i| {
                let wk = self.walk_ref(i);
                wk.at_leisure() && matches!(wk.act, Act::Idle | Act::Walk) && wk.rest.is_none() && (here == 0 || wk.ws == here)
            })
            .collect();
        free.sort_by(|&a, &b| (self.strip_px(a) - middle).abs().total_cmp(&(self.strip_px(b) - middle).abs()));
        free
    }

    fn actor_w(&self, i: usize) -> f32 {
        let r = self.actor_rect(i);
        r[2] - r[0]
    }

    /// A few of those with nothing to do, in no particular order.
    fn few_at_leisure(&mut self, n: usize) -> Vec<usize> {
        let here = self.spaces.active;
        let mut free: Vec<usize> = (0..n).filter(|&i| self.walk_ref(i).at_leisure() && (here == 0 || self.walk_ref(i).ws == here)).collect();
        let mut out = Vec::new();
        while !free.is_empty() && out.len() < tn().win_curious as usize {
            let k = ((self.walk.rand() * free.len() as f32) as usize).min(free.len() - 1);
            out.push(free.swap_remove(k));
        }
        out
    }

    /// A new window: a few wander over to have a look.
    fn peek_at(&mut self, center: f32, width: f32) {
        let n = self.extras.len() + 1;
        for i in self.few_at_leisure(n) {
            let spread = width.min(self.view_w * 0.3);
            let from = self.strip_px(i);
            let linger = tn().win_linger;
            let view_w = self.view_w.max(1.0);
            let wk = self.walk_of(i);
            let to = center + (wk.rand() - 0.5) * spread;
            let walk = (to - from) * 2.0 / view_w;
            if walk.abs() <= 1.4 {
                wk.crowd = Some(PatCrowd::curious(wk.x + walk, wk.x, linger, to > center));
            }
        }
    }

    /// Zs over those asleep, sweat on those fretting.
    /// Night: about half of them go to bed one by one and get up again in
    /// the morning. Snow: they hop after the flakes now and then.
    fn run_weather(&mut self, dt: f32, n: usize) {
        let w = super::weather::now();
        let was = self.moods.weather;
        self.moods.weather = w;
        if w.night && !was.night {
            // who's sleepy tonight
            let now = super::weather::BEDTIME.load(Relaxed);
            let (share, spread) = if now { (1.0, 4.0) } else { (tn().night_sleep, tn().night_spread) };
            for i in 0..n {
                let wk = self.walk_of(i);
                wk.night_wait = if wk.rand() < share { Some(wk.rand() * spread) } else { None };
            }
        }
        // rain coming on: a drop on the face and up they get, umbrella in hand
        // (not those in bed for the night)
        if w.rain && !was.rain {
            for i in 0..n {
                let wk = self.walk_of(i);
                if wk.act == Act::Rest && !wk.night_sleep {
                    wk.rain_wake = Some(wk.rand() * 1.5);
                }
            }
        }
        if !w.night && was.night {
            for i in 0..n {
                let wk = self.walk_of(i);
                wk.night_wait = None;
                if wk.night_sleep {
                    // morning: up they get, one by one
                    wk.night_sleep = false;
                    let wake = 1.0 + wk.rand() * tn().night_spread * 0.3;
                    if let Some(r) = wk.rest.as_mut() {
                        r.wake = Some(wake);
                    }
                }
            }
        }
        for i in 0..n {
            let wk = self.walk_of(i);
            if let Some(t) = wk.rain_wake {
                if t - dt <= 0.0 {
                    wk.rain_wake = None;
                    if wk.act == Act::Rest {
                        wk.get_up();
                        wk.start_jump(tn().jump_height * 0.7);
                    }
                } else {
                    wk.rain_wake = Some(t - dt);
                }
            }
            if let Some(t) = wk.night_wait {
                if !wk.at_leisure() {
                    continue;
                }
                if t - dt <= 0.0 {
                    wk.night_wait = None;
                    wk.lie_down(0.0);
                    wk.night_sleep = true;
                } else {
                    wk.night_wait = Some(t - dt);
                }
            }
            // its favourite keepsake, looked up now and then; some days it
            // carries it about on its back
            wk.fav_t -= dt;
            if wk.fav_t <= 0.0 {
                wk.fav_t = 20.0;
                let name = self.character_name(i);
                let fav = self.moods.keepsakes.favourite(&name);
                self.walk_of(i).keepsake = fav;
            }
            let wk = self.walk_of(i);
            wk.carry_t -= dt;
            if wk.carry_t <= 0.0 {
                wk.carry_t = 60.0 + wk.rand() * 120.0;
                wk.carry = wk.keepsake.is_some() && wk.rand() < tn().carry_keepsake;
            }
            // now and then it stands still and holds up a sign
            let here = self.spaces.active;
            let wk = self.walk_of(i);
            let seen = here == 0 || wk.ws == here;
            wk.sign_t -= dt;
            if wk.sign.is_some() {
                wk.sign_left -= dt;
                if wk.sign_left <= 0.0 || !wk.can_hold_sign() {
                    wk.sign = None;
                } else {
                    wk.act = Act::Idle;
                    wk.timer = wk.timer.max(0.5);
                    wk.target = wk.x;
                }
            } else if wk.sign_t <= 0.0 {
                wk.sign_t = 120.0 + wk.rand() * 300.0;
                if seen && wk.act == Act::Idle && wk.at_leisure() && wk.rand() < 0.6 {
                    // up past 2 at night: "go to sleep" more often than not
                    let late = (120..300).contains(&super::weather::minutes_now());
                    wk.sign = Some(if late && wk.rand() < 0.7 { super::sign::NIGHT } else { (wk.rand() * super::sign::RANDOM as f32) as usize });
                    wk.sign_left = 20.0 + wk.rand() * 10.0;
                }
            }
            // carried fast or shaken hard while held: items fly off mid-hold once
            // momentum builds, and a brief whip still triggers on landing impact
            let wk = self.walk_ref(i);
            if wk.act == Act::Held {
                let speed = (wk.swing_vel[0] * wk.swing_vel[0] + wk.swing_vel[1] * wk.swing_vel[1]).sqrt();
                let whipped = speed > tn().drop_speed || wk.swing_v.abs() > tn().drop_swing;
                let dir = if wk.swing_v.abs() > 1.0 {
                    Some(-wk.swing_v.signum())
                } else if wk.swing_vel[0].abs() > 100.0 {
                    Some(-wk.swing_vel[0].signum())
                } else {
                    None
                };
                let wk = self.walk_of(i);
                if step_hold_shake(&mut wk.shake, &mut wk.shaken, whipped, dt) {
                    self.knock_off(i, 1.0, tn().jolt_drop, dir);
                }
            }
            // interaction finished on impact (or race knockdown): headgear, eyewear, and held items fly off
            let impact_dir = self.walk_of(i).impact_dir.take();
            let wk = self.walk_ref(i);
            let tumbling = impact_dir.is_some() || wk.act == Act::Planted || wk.race_down;
            if tumbling && !wk.jolted {
                self.walk_of(i).jolted = true;
                self.knock_off(i, tn().jolt_drop, tn().jolt_drop * 0.85, impact_dir);
            } else if !tumbling && matches!(wk.act, Act::Idle | Act::Walk) {
                let wk = self.walk_of(i);
                wk.jolted = false;
                wk.shaken = false;
            }
            let wk = self.walk_of(i);
            // snow settles on its head while it stands about, and comes off when it jumps or runs
            let shaken = matches!(wk.act, Act::Jump | Act::Fall | Act::Held | Act::Chase | Act::Planted) || wk.rot.abs() > 0.3;
            wk.snow_pile = if shaken {
                (wk.snow_pile - dt * 3.0).max(0.0)
            } else if w.snow {
                (wk.snow_pile + dt / tn().snow_settle.max(1.0)).min(1.0)
            } else {
                (wk.snow_pile - dt / 40.0).max(0.0)
            };
            if w.snow && wk.at_leisure() && wk.rand() < dt * tn().snow_hop {
                let h = tn().jump_height * (0.5 + wk.rand() * 0.5);
                wk.start_jump(h);
                wk.gesture = true;
            }
        }
    }

    /// Things go flying: the umbrella/handheld items (with `hand` chance),
    /// hats & eyewear (`head` chance), from where they're drawn on impact.
    fn knock_off(&mut self, i: usize, hand: f32, head: f32, impact_dir: Option<f32>) {
        let a = self.actor_ref(i);
        let (r, pose) = (a.hit_rect, a.pose);
        let (w, h) = (r[2] - r[0], r[3] - r[1]);
        if h <= 0.0 {
            return;
        }
        let worn = a.worn();
        let owner_name = self.character_name(i);
        let view = a.own_view[0] * self.view_w * 0.5;
        // where it would stand: the bottom of its box on the ground, less the empty margin
        let ground = (1.0 - (pose.center[1] - pose.half[1])) * 0.5 * pose.size[1];
        let floor = ground - h * (1.0 - a.opaque[2]);
        let mut gone: Vec<(String, i32, [f32; 4], Option<std::path::PathBuf>)> = Vec::new();
        for t in &worn {
            if t.id.is_empty() || gone.iter().any(|g| g.0 == t.id) {
                continue;
            }
            let [x0, y0, x1, y1] = t.rect;
            let px = [r[0] + (x0 + 1.0) * 0.5 * w, r[1] + (1.0 - y1) * 0.5 * h, r[0] + (x1 + 1.0) * 0.5 * w, r[1] + (1.0 - y0) * 0.5 * h];
            // the umbrella flies as a whole, top and stick together
            let (kind, px) = if t.id == "umbrella" {
                (22, [px[0], px[1], px[2], r[1] + h * 0.75])
            } else {
                (t.kind, px)
            };
            gone.push((t.id.clone(), kind, px, t.image.clone()));
        }
        let wk = self.walk_of(i);
        let dir = if let Some(d) = impact_dir.filter(|d| d.abs() > 0.05) {
            d.signum()
        } else if wk.swing_v.abs() > 0.5 {
            -wk.swing_v.signum()
        } else if wk.vx.abs() > 0.01 {
            wk.vx.signum()
        } else if wk.rand() < 0.5 {
            -1.0
        } else {
            1.0
        };
        let mut flying = Vec::new();
        for (idx, (id, kind, px, image)) in gone.into_iter().enumerate() {
            let chance = if id == "umbrella" || kind == 100 { hand.max(head) } else { head };
            if wk.rand() >= chance {
                continue;
            }
            // fan multiple dropped items apart so they don't stack in one spot
            let spread = if idx % 2 == 0 { dir } else { -dir * (0.4 + wk.rand() * 0.6) };
            let (vx, spin) = (spread * (180.0 + wk.rand() * 280.0), -spread * (5.0 + wk.rand() * 7.0));
            let size = [px[2] - px[0], px[3] - px[1]];
            let (x, y) = ((px[0] + px[2]) * 0.5 + view, (px[1] + px[3]) * 0.5);
            let lie = size[0].min(size[1]) * 0.5;
            log::debug!("{i} loses its {id}");
            wk.lost.push(id.clone());
            flying.push(Loose { id, image, owner_name: owner_name.clone(), helper_name: None, kind, owner: i, x, y, vx, vy: -290.0 - wk.rand() * 220.0, rot: 0.0, spin, floor: floor - lie, size, still: 0.0, helper: None, carried: false });
        }
        self.moods.loose.extend(flying);
    }

    /// Dropped things fall and bounce; their owner, once on its feet, goes
    /// and picks them up again.
    pub(super) fn run_loose(&mut self, dt: f32, n: usize) {
        let raining = super::weather::now().rain;
        let mut keep = Vec::new();
        let names: Vec<String> = (0..n).map(|i| self.character_name(i)).collect();
        for mut l in std::mem::take(&mut self.moods.loose) {
            // find its owner (and helper) again: the team may have changed
            let Some(o) = names.iter().position(|x| x == &l.owner_name) else { continue };
            l.owner = o;
            l.helper = l.helper_name.as_ref().and_then(|h| names.iter().position(|x| x == h));
            if l.helper.is_none() {
                l.helper_name = None;
                l.carried = false;
            }
            if l.still <= 0.0 || l.y < l.floor - 1.0 {
                l.vy += 2600.0 * dt;
                l.x += l.vx * dt;
                l.y += l.vy * dt;
                l.rot += l.spin * dt;
                if l.y >= l.floor {
                    l.y = l.floor;
                    l.vy = -l.vy * 0.35;
                    l.vx *= 0.6;
                    l.spin *= 0.5;
                    if l.vy.abs() < 120.0 {
                        l.vy = 0.0;
                        l.vx = 0.0;
                        l.spin = 0.0;
                        // lies flat, on its side
                        l.rot = if (l.rot.rem_euclid(std::f32::consts::TAU)) < std::f32::consts::PI { std::f32::consts::FRAC_PI_2 } else { -std::f32::consts::FRAC_PI_2 };
                        l.still = 0.001;
                    }
                }
            } else {
                l.still += dt;
            }
            let wk = self.walk_ref(o);
            // not raining any more, or left too long: it's gone, and so is the loss
            if (l.id == "umbrella" && !raining && l.still > 8.0) || l.still > 120.0 {
                self.walk_of(o).lost.retain(|x| x != &l.id);
                continue;
            }
            let free = matches!(wk.act, Act::Idle | Act::Walk) && wk.social.is_none() && wk.race.is_none();
            let ready = |this: &Self, j: usize| {
                let w = this.walk_ref(j);
                matches!(w.act, Act::Idle | Act::Walk) && w.social.is_none() && w.race.is_none() && w.crowd.is_none()
            };
            // the owner's busy: a friend at a loose end fetches it and brings it over
            if l.still > 0.6 && !free {
                if l.helper.is_some_and(|h| h >= n || !ready(self, h)) {
                    // the helper got caught up in something: down it goes where it is
                    if let Some(h) = l.helper.filter(|_| l.carried) {
                        l.x = self.strip_px(h);
                        l.y = l.floor;
                        l.rot = std::f32::consts::FRAC_PI_2;
                    }
                    l.helper = None;
                    l.helper_name = None;
                    l.carried = false;
                }
                if l.helper.is_none() && l.still > 3.0 {
                    let ws = wk.ws;
                    let x = l.x;
                    l.helper = (0..n)
                        .filter(|&j| j != o && self.walk_ref(j).ws == ws && ready(self, j) && (self.strip_px(j) - x).abs() < self.view_w * 0.6)
                        .max_by(|&a, &b| {
                            let score = |j: usize| self.bond(o, j).0 * 400.0 - (self.strip_px(j) - x).abs();
                            score(a).total_cmp(&score(b))
                        });
                    if let Some(h) = l.helper {
                        log::debug!("{h} goes to fetch {o}'s {}", l.id);
                        l.helper_name = Some(names[h].clone());
                    }
                }
                if let Some(h) = l.helper {
                    let reach = (self.actor_ref(h).hit_rect[3] - self.actor_ref(h).hit_rect[1]) * 0.35;
                    let hx = self.strip_px(h);
                    if !l.carried {
                        if (hx - l.x).abs() < reach {
                            l.carried = true;
                            self.walk_of(h).start_bob(tn().jump_height * 0.2);
                        } else {
                            self.walk_toward(h, l.x);
                        }
                    }
                    if l.carried {
                        // held up in its hand on the way over
                        let r = self.actor_ref(h).hit_rect;
                        let side = if self.walk_ref(h).left { -1.0 } else { 1.0 };
                        l.x = hx + side * (r[2] - r[0]) * 0.25;
                        l.y = r[1] + (r[3] - r[1]) * 0.45;
                        l.rot = 0.0;
                        let ox = self.strip_px(o);
                        if (hx - ox).abs() < reach * 2.0 {
                            // here you are
                            self.walk_of(o).lost.retain(|x| x != &l.id);
                            self.walk_of(h).start_bob(tn().jump_height * 0.3);
                            let secs = tn().laugh_time * 0.6;
                            self.walk_of(o).laugh(secs);
                            self.add_bond(o, h, tn().bond_game, 0.0);
                            log::debug!("{h} brings {o} its {}", l.id);
                            continue;
                        }
                        self.walk_toward(h, ox);
                    }
                }
            }
            if l.still > 0.6 && free {
                l.helper = None;
                l.helper_name = None;
                l.carried = false;
                let here = self.strip_px(o);
                let reach = (self.actor_ref(o).hit_rect[3] - self.actor_ref(o).hit_rect[1]) * 0.35;
                if (here - l.x).abs() < reach {
                    // got it back
                    let wk = self.walk_of(o);
                    wk.lost.retain(|x| x != &l.id);
                    wk.start_bob(tn().jump_height * 0.3);
                    log::debug!("{o} picks its {} up again", l.id);
                    continue;
                }
                // over to it
                let pose = self.actor_ref(o).pose;
                let to_clip = (l.x - self.actor_ref(o).own_view[0] * self.view_w * 0.5) * 2.0 / self.view_w.max(1.0) - 1.0;
                let wk = self.walk_of(o);
                wk.target = wk.x + (to_clip - pose.center[0]);
                wk.act = Act::Walk;
            }
            keep.push(l);
        }
        self.moods.loose = keep;
    }

    /// Sends `i` walking (on its own, not in a chat) to strip x `x`.
    fn walk_toward(&mut self, i: usize, x: f32) {
        let a = self.actor_ref(i);
        let (pose, view) = (a.pose, a.own_view[0]);
        let to_clip = (x - view * self.view_w * 0.5) * 2.0 / self.view_w.max(1.0) - 1.0;
        let wk = self.walk_of(i);
        wk.target = wk.x + (to_clip - pose.center[0]);
        wk.act = Act::Walk;
    }

    pub(super) fn mood_props(&self, v: &mut Vec<(i32, [f32; 4], bool, usize)>) {
        let time = self.start.elapsed().as_secs_f32();
        // rain: an umbrella over everyone not busy in a game; snow: flakes about them
        let w = self.moods.weather;
        if (w.rain || w.snow) && (0.5..1.5).contains(&tn().weather_show) {
            let actors = std::iter::once((&self.walk, self.hit_rect, self.visible, self.opaque)).chain(self.extras.iter().map(|e| (&e.walk, e.hit_rect, e.visible, e.opaque)));
            for (k, (wk, r, seen, _)) in actors.enumerate() {
                let h = r[3] - r[1];
                if h <= 0.0 || !seen || matches!(wk.act, Act::Held | Act::Fall | Act::Travel | Act::Planted) {
                    continue;
                }
                let cx = (r[0] + r[2]) * 0.5;
                if !w.rain {
                    for f in 0..3 {
                        let seed = k as f32 * 7.31 + f as f32 * 2.17;
                        let t = (time * 0.18 + seed * 0.37).fract();
                        let x = cx + ((seed * 3.7).sin() * 0.9 + (time * 1.3 + seed).sin() * 0.15) * h * 0.6;
                        let y = r[1] - h * 0.5 + t * h * 1.4;
                        let s = h * 0.09;
                        v.push((23, [x - s, y - s, x + s, y + s], false, k));
                    }
                }
            }
        }
        for &(kind, owner, x, y, size, left) in &self.moods.effects {
            let a = if owner == 0 { self } else if let Some(e) = self.extras.get(owner - 1) { e } else { continue };
            // grows quickly, then fades by shrinking
            let k = if left > 0.3 { 1.0 } else { left / 0.3 };
            let (cx, cy, s) = if kind >= super::social::FOUND {
                let r = a.hit_rect;
                let h = r[3] - r[1];
                if h <= 0.0 {
                    continue;
                }
                let s = h * 0.46 * (0.6 + 0.4 * k);
                let bob = (time * 6.0).sin() * h * 0.04;
                ((r[0] + r[2]) * 0.5, r[1] + h * a.opaque[1] - s * 0.58 + bob, s)
            } else {
                (x - a.own_view[0] * self.view_w * 0.5, y, size * (0.6 + 0.4 * k))
            };
            v.push((kind, [cx - s * 0.5, cy - s * 0.5, cx + s * 0.5, cy + s * 0.5], false, owner));
        }
        for c in &self.moods.crumbs {
            let a = if c.owner == 0 { self } else { &self.extras[c.owner - 1] };
            let r = a.hit_rect;
            let h = r[3] - r[1];
            let (x, y, size) = match c.stuck {
                Some([ox, oy]) => ((r[0] + r[2]) * 0.5 + ox * h, r[1] + h * (0.28 + oy), c.size * h),
                None => (c.x - a.own_view[0] * self.view_w * 0.5, c.y, c.size),
            };
            v.push((6, [x - size * 0.5, y - size * 0.5, x + size * 0.5, y + size * 0.5], false, c.owner));
        }
        let time = self.start.elapsed().as_secs_f32();
        let actors = std::iter::once((&self.walk, self.hit_rect)).chain(self.extras.iter().map(|e| (&e.walk, e.hit_rect)));
        for (k, (wk, r)) in actors.enumerate() {
            let height = r[3] - r[1];
            if height <= 0.0 {
                continue;
            }
            let cx = (r[0] + r[2]) * 0.5;
            let offset = k as f32 * 0.37;
            if wk.act == Act::Rest && wk.rest.is_some_and(|x| x.snoring()) {
                for z in 0..3 {
                    let phase = (time * 0.35 + z as f32 / 3.0 + offset).fract();
                    let size = height * 0.17 * (0.6 + 0.6 * phase) * (1.0 - phase.powi(6));
                    let x = cx + height * (0.12 + 0.22 * phase) + (phase * 7.0).sin() * height * 0.03;
                    let y = r[1] + height * (0.62 - 0.5 * phase);
                    v.push((4, [x - size * 0.5, y - size * 0.5, x + size * 0.5, y + size * 0.5], false, k));
                }
            } else if wk.fret && wk.social.is_none() && wk.race.is_none() && matches!(wk.act, Act::Walk | Act::Idle) {
                let phase = (time * 1.6 + offset).fract();
                let size = height * 0.13 * (1.0 - phase.powi(4));
                let side = if wk.left { 1.0 } else { -1.0 };
                let x = cx + side * height * (0.2 + 0.08 * phase);
                let y = r[1] + height * (0.2 + 0.25 * phase);
                v.push((5, [x - size * 0.5, y - size * 0.5, x + size * 0.5, y + size * 0.5], false, k));
            }
        }
    }

    pub(super) fn character_name(&self, i: usize) -> String {
        let source = if i == 0 { &self.config.source } else { &self.extras[i - 1].config.source };
        let path = Path::new(source);
        path.parent().and_then(|p| p.file_name()).or(path.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// (friendship, rivalry) of two characters.
    pub(super) fn bond(&self, a: usize, b: usize) -> (f32, f32) {
        self.moods.bonds.get(&self.character_name(a), &self.character_name(b))
    }

    /// Everyone `name` knows: (other, friendship, rivalry), best friends first.
    pub(super) fn bonds_of(&mut self, name: &str) -> Vec<(String, f32, f32)> {
        let mut v = self.moods.bonds.of(name);
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        v
    }

    /// Friends close enough to stick together.
    pub(super) fn close(&self, a: usize, b: usize) -> bool {
        a != b && self.bond(a, b).0 >= tn().close_friend
    }


    /// How good `i` is at `game`, 0 (first tries) to 1 (a master).
    pub(super) fn skill(&mut self, i: usize, game: &str) -> f32 {
        let name = self.character_name(i);
        self.moods.skills.get(&name, game)
    }

    /// From `lo` for a beginner to `hi` for a master.
    pub(super) fn skilled(&mut self, i: usize, game: &str, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.skill(i, game)
    }

    /// Everyone who played gets a little better; a friend who is better at
    /// it shows them how.
    pub(super) fn practice(&mut self, members: &[usize], game: &str) {
        for &i in members {
            let own = self.skill(i, game);
            let friends: Vec<String> = members
                .iter()
                .filter(|&&j| j != i && self.bond(i, j).0 >= tn().meet_friend)
                .map(|&j| self.character_name(j))
                .collect();
            let best = friends.iter().map(|n| self.moods.skills.get(n, game)).fold(0.0, f32::max);
            let boost = (best - own).max(0.0) * tn().skill_teach;
            let name = self.character_name(i);
            self.moods.skills.practice(&name, game, boost);
        }
    }

    pub(super) fn add_bond(&mut self, a: usize, b: usize, friend: f32, rival: f32) {
        let (x, y) = (self.character_name(a), self.character_name(b));
        self.moods.bonds.add(&x, &y, friend, rival);
    }
}

/// How good each character is at each game (0 to 1), by name, kept in
/// `skills.json` in the data folder. Everyone starts clumsy.
/// Little things each character found and keeps: prop kinds, newest last.
#[derive(Default)]
pub(super) struct Keepsakes {
    map: HashMap<String, Vec<i32>>,
    loaded: bool,
    dirty: bool,
    save_in: f32,
}

impl Keepsakes {
    fn path() -> std::path::PathBuf {
        crate::util::data_dir().join("keepsakes.json")
    }

    fn load(&mut self) {
        if !self.loaded {
            self.loaded = true;
            if let Ok(text) = std::fs::read_to_string(Self::path()) {
                self.map = serde_json::from_str(&text).unwrap_or_default();
            }
        }
    }

    pub(super) fn add(&mut self, name: &str, kind: i32) {
        self.load();
        let v = self.map.entry(name.to_string()).or_default();
        v.push(kind);
        // a pocketful at most: the oldest goes
        if v.len() > 12 {
            v.remove(0);
        }
        self.dirty = true;
    }

    pub(super) fn of(&mut self, name: &str) -> Vec<i32> {
        self.load();
        self.map.get(name).cloned().unwrap_or_default()
    }

    /// The one it's fondest of: whatever it has most of, the newest on a tie.
    pub(super) fn favourite(&mut self, name: &str) -> Option<i32> {
        let v = self.of(name);
        let count = |k: i32| v.iter().filter(|&&x| x == k).count();
        v.iter().rev().copied().max_by_key(|&k| count(k))
    }

    pub(super) fn tick(&mut self, dt: f32) {
        self.save_in -= dt;
        if self.dirty && self.save_in <= 0.0 {
            self.save_in = 60.0;
            self.dirty = false;
            if let Ok(text) = serde_json::to_string(&self.map) {
                let _ = crate::util::atomic_write(&Self::path(), text.as_bytes());
            }
        }
    }
}

#[derive(Default)]
pub(super) struct Skills {
    map: HashMap<String, HashMap<String, f32>>,
    loaded: bool,
    dirty: bool,
    save_in: f32,
}

impl Skills {
    fn path() -> std::path::PathBuf {
        crate::util::data_dir().join("skills.json")
    }

    fn load(&mut self) {
        if !self.loaded {
            self.loaded = true;
            if let Some(map) = std::fs::read_to_string(Self::path()).ok().and_then(|t| serde_json::from_str(&t).ok()) {
                self.map = map;
            }
        }
    }

    pub(super) fn get(&mut self, name: &str, game: &str) -> f32 {
        self.load();
        self.map.get(name).and_then(|g| g.get(game)).copied().unwrap_or(0.0)
    }

    pub(super) fn all(&mut self, name: &str) -> Vec<(String, f32)> {
        self.load();
        let mut v: Vec<(String, f32)> = self.map.get(name).map(|g| g.iter().map(|(k, v)| (k.clone(), *v)).collect()).unwrap_or_default();
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        v
    }

    /// One more game played: a step closer to mastery, the step smaller the
    /// better it already is; `boost` for being shown how by a better friend.
    pub(super) fn practice(&mut self, name: &str, game: &str, boost: f32) {
        self.load();
        let s = self.map.entry(name.to_string()).or_default().entry(game.to_string()).or_insert(0.0);
        *s = (*s + (tn().skill_rate + boost) * (1.0 - *s)).min(1.0);
        self.dirty = true;
    }

    pub(super) fn tick(&mut self, dt: f32) {
        self.load();
        self.save_in -= dt;
        if self.dirty && self.save_in <= 0.0 {
            self.save_in = 120.0;
            self.dirty = false;
            if let Ok(text) = serde_json::to_string(&self.map) {
                let _ = crate::util::atomic_write(&Self::path(), text.as_bytes());
            }
        }
    }
}

/// How well each pair of characters knows each other; the file is
/// `bonds.json` in the data folder, by character name so it survives changes
/// of team.
#[derive(Default)]
pub(super) struct Bonds {
    map: HashMap<String, [f32; 2]>,
    loaded: bool,
    dirty: bool,
    save_in: f32,
}

impl Bonds {
    fn key(a: &str, b: &str) -> String {
        if a < b { format!("{a}|{b}") } else { format!("{b}|{a}") }
    }

    fn path() -> std::path::PathBuf {
        crate::util::data_dir().join("bonds.json")
    }

    fn load(&mut self) {
        if !self.loaded {
            self.loaded = true;
            if let Some(map) = std::fs::read_to_string(Self::path()).ok().and_then(|t| serde_json::from_str(&t).ok()) {
                self.map = map;
            }
        }
    }

    pub(super) fn of(&mut self, name: &str) -> Vec<(String, f32, f32)> {
        self.load();
        self.map
            .iter()
            .filter_map(|(k, v)| {
                let (a, b) = k.split_once('|')?;
                let other = if a == name { b } else if b == name { a } else { return None };
                Some((other.to_string(), v[0], v[1]))
            })
            .collect()
    }

    pub(super) fn get(&self, a: &str, b: &str) -> (f32, f32) {
        self.map.get(&Self::key(a, b)).map_or((0.0, 0.0), |v| (v[0], v[1]))
    }

    pub(super) fn add(&mut self, a: &str, b: &str, friend: f32, rival: f32) {
        self.load();
        let v = self.map.entry(Self::key(a, b)).or_insert([0.0, 0.0]);
        v[0] = (v[0] + friend).min(3.0);
        v[1] = (v[1] + rival).min(3.0);
        self.dirty = true;
    }

    /// Reads the file once at the start, writes it every couple of minutes.
    pub(super) fn tick(&mut self, dt: f32) {
        self.load();
        self.save_in -= dt;
        if self.dirty && self.save_in <= 0.0 {
            self.save_in = 120.0;
            self.dirty = false;
            if let Ok(text) = serde_json::to_string(&self.map) {
                let _ = crate::util::atomic_write(&Self::path(), text.as_bytes());
            }
        }
    }
}

/// Tracks fast carrying or shaking while held. Returns `true` when items should
/// scatter immediately mid-hold, and marks `shaken` for brief whips released into a throw.
fn step_hold_shake(shake: &mut f32, shaken: &mut bool, whipped: bool, dt: f32) -> bool {
    *shake = if whipped { *shake + dt } else { (*shake - dt * 1.2).max(0.0) };
    if *shake > 0.18 {
        *shake = 0.0;
        *shaken = false;
        return true;
    }
    if *shake > 0.08 {
        *shaken = true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn idler() -> WalkState {
        WalkState { act: Act::Idle, timer: 100.0, seed: 11, ..Default::default() }
    }

    #[test]
    fn dozes_off_then_wakes() {
        let mut wk = idler();
        for _ in 0..60 * 30 {
            wk.rest_tick(DT, true, true, false);
            wk.rest_pose(DT);
        }
        assert_eq!(wk.act, Act::Rest);
        assert!(wk.rest.unwrap().snoring());
        assert!((wk.rot.abs() - FRAC_PI_2).abs() < 0.05, "rot {}", wk.rot);
        for _ in 0..60 * 5 {
            wk.rest_tick(DT, false, false, false);
            wk.rest_pose(DT);
            if wk.act != Act::Rest {
                break;
            }
        }
        assert_ne!(wk.act, Act::Rest, "never woke");
        assert!(wk.rest.is_none());
    }

    #[test]
    fn plays_itself_out_and_naps() {
        let mut wk = idler();
        wk.played = 5.0;
        wk.energy = 0.05;
        for _ in 0..60 * 30 {
            wk.rest_tick(DT, false, false, false);
            if wk.act == Act::Rest {
                break;
            }
        }
        assert_eq!(wk.act, Act::Rest, "never napped");
        assert_eq!(wk.played, 0.0);
        let mut frames = 0;
        while wk.act == Act::Rest && frames < 60 * 300 {
            wk.rest_tick(DT, false, false, false);
            frames += 1;
        }
        assert_ne!(wk.act, Act::Rest, "overslept");
        assert!(frames as f32 * DT >= tn().nap_min, "woke after {frames} frames");
        assert!(wk.energy > 0.8, "energy {}", wk.energy);
    }

    #[test]
    fn stays_awake_with_energy_left() {
        let mut wk = idler();
        wk.played = 5.0;
        wk.energy = 0.9;
        for _ in 0..60 * 60 {
            wk.rest_tick(DT, false, false, false);
        }
        assert_ne!(wk.act, Act::Rest);
    }

    #[test]
    fn panics_without_stopping_or_jumping() {
        let mut wk = idler();
        wk.fret = true;
        wk.seed = 21;
        let mut moved = 0.0;
        for _ in 0..60 * 60 {
            let before = wk.x;
            let (_, walking, _) = wk.step(DT, -0.9, 0.9, Some(0.0), 0.0);
            moved += (wk.x - before).abs();
            assert_ne!(wk.act, Act::Jump);
            assert_ne!(wk.act, Act::Chase);
            assert!(walking || wk.act == Act::Walk, "stood still as {:?}", wk.act);
        }
        assert!(moved > 60.0 * tn().walk_speed * 2.0, "only moved {moved}");
    }

    #[test]
    fn panic_gets_it_out_of_bed() {
        let mut wk = idler();
        wk.lie_down(0.0);
        wk.fret = true;
        wk.rest_tick(DT, true, true, false);
        assert_eq!(wk.act, Act::Idle);
    }

    #[test]
    fn stays_awake_if_busy_in_a_chat() {
        let mut wk = idler();
        wk.circle = true;
        for _ in 0..60 * 30 {
            wk.rest_tick(DT, true, true, false);
        }
        assert_ne!(wk.act, Act::Rest);
    }

    #[test]
    fn sits_when_crowded_and_gets_up_again() {
        let mut wk = idler();
        let mut sat = false;
        for _ in 0..60 * 3000 {
            wk.rest_tick(DT, false, false, !sat);
            sat |= wk.act == Act::Rest;
            if sat && wk.act != Act::Rest {
                return;
            }
        }
        panic!("sat {sat}, never got up");
    }

    #[test]
    fn bonds_are_shared_both_ways() {
        let mut b = Bonds { loaded: true, ..Default::default() };
        b.add("Rice", "Suzuka", 0.5, 0.25);
        assert_eq!(b.get("Suzuka", "Rice"), (0.5, 0.25));
        b.add("Suzuka", "Rice", 10.0, 0.0);
        assert_eq!(b.get("Rice", "Suzuka").0, 3.0);
    }

    #[test]
    fn hold_shake_drops_mid_hold_and_remembers_brief_whip() {
        let mut shake = 0.0;
        let mut shaken = false;

        // Gentle carry never triggers either mid-hold drop or landing shaken flag
        for _ in 0..20 {
            assert!(!step_hold_shake(&mut shake, &mut shaken, false, DT));
        }
        assert!(!shaken);

        // Brief whip sets shaken (for landing impact) before hitting mid-hold drop
        for _ in 0..6 {
            assert!(!step_hold_shake(&mut shake, &mut shaken, true, DT));
        }
        assert!(shaken);

        // Sustained fast carry or shake triggers immediate mid-hold drop and clears shaken
        let mut dropped = false;
        for _ in 0..10 {
            if step_hold_shake(&mut shake, &mut shaken, true, DT) {
                dropped = true;
                break;
            }
        }
        assert!(dropped);
        assert!(!shaken);
        assert_eq!(shake, 0.0);
    }

    #[test]
    fn sign_holder_ignores_notify_jump_and_leisure_interrupts() {
        let mut wk = idler();
        assert!(wk.at_leisure());

        wk.sign = Some(super::super::sign::HUG);
        assert!(wk.can_hold_sign());
        assert!(!wk.at_leisure(), "sign holders must not be picked as idle for notifications/curiosity");

        // Even if a notify_jump was pending, stepping while holding a sign clears it without jumping
        wk.notify_jump = Some(0.0);
        wk.step(DT, -1.0, 1.0, None, 0.0);
        assert_eq!(wk.act, Act::Idle);
        assert!(wk.notify_jump.is_none());
        assert!(wk.sign.is_some());
    }
}
