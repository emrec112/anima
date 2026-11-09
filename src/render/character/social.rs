//! Characters taking notice of each other: two standing close look over,
//! step side by side, chat for a while, maybe stroll together, then go their
//! own ways. Others passing by can join in, the more likely the bigger the
//! crowd, and a chat that gathers most of the team turns into a race. Only standing ones start one: walking through each
//! other is fine.

use crate::render::tuning::get as tn;
use super::CharacterOverlay;
use super::walk::{Act, WalkState};

/// A game asked for from the settings window (`anima game <name>`).
pub static GAME_NOW: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// One character's part in a chat. Goals are strip positions in logical px,
/// like a race's.
#[derive(Clone, Copy, Debug)]
pub(super) struct Social {
    chat: u32,
    pub(super) goal_px: f32,
    pub(super) goal: f32,
    speed: f32,
    moving: bool,
    face_left: bool,
    // in the happy pose (talking, laughing)
    pub(super) happy: bool,
    // playing the cursor game, and seconds to its next try
    pub(super) chasing: bool,
    hop_cool: f32,
    // long jump: in the air (spinning this fast), or planted in the ground
    pub(super) flying: bool,
    spin: f32,
    stuck: bool,
    rot_target: f32,
    sink_target: f32,
    bump_cool: f32,
    // the referee knows about this landing
    queued: bool,
    // where it took off from, logical px
    launch_px: f32,
    // leaning back on the rope (radians)
    lean: f32,
    // went over from the feet, like a plank, not spinning about its middle
    feet: bool,
    // tag: speeding up and slowing down (multiplier, where it's heading, seconds left)
    pace: f32,
    pace_goal: f32,
    pace_wait: f32,
    // tag: jumping over "it" to get out of a corner, moving through the air
    leaping: bool,
    // tag: has reached the playing window, and only now keeps inside it
    inside: bool,
    // on the rope: nothing but the rope itself can be touched
    pub(super) no_touch: bool,
    // in a game that petting would break
    pub(super) no_pat: bool,
    // long jump: has had its go (or was thrown by hand instead)
    jumped: bool,
    // samba: looks where it's told while stepping somewhere else
    face_lock: bool,
    // volleyball: standing on the other half of the net
    perched: bool,
    // knocked flying: times it comes back off the ground
    pub(super) bounces: u8,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    Notice,
    Approach,
    Talk,
    Stroll,
    // games: each jumps in turn, or all go after the cursor
    Contest,
    Cursor,
    Long,
    Domino,
    Tug,
    Tag,
    Carrot,
    Gift,
    Samba,
    Volley,
    Cheer,
    Part,
    // someone's said "let's race": the group talks it over, then sets off
    RaceCall,
    // setting off a few at a time; the rest stay and chat till their turn
    RaceWait,
}

/// Volleyball: who does what, and the score.
struct Volley {
    /// hit high enough to go over: the net can't stop it on the way up
    lofted: bool,
    teams: [Vec<usize>; 2],
    homes: Vec<(usize, f32)>,
    ball: usize,
    net: Vec<usize>,
    // this ball was misjudged: nobody gets it
    fumbled: bool,
    mid: f32,
    lo: f32,
    hi: f32,
    score: [u32; 2],
    server: usize,
    // after a point: seconds until the next serve
    rest: f32,
    // who is running for the ball, where to, after what reaction, how fast
    runner: Option<usize>,
    to: f32,
    react: f32,
    run_speed: f32,
    top_speed: f32,
    rally: i32,
}

impl Volley {
    /// Height of the top of the net, in `air` units.
    fn net_top(&self) -> f32 {
        (self.net.len() as f32 - 1.0) * 1.7 + 2.0
    }

    /// The part of the court a team may stand on.
    fn half(&self, team: usize, gap: f32) -> (f32, f32) {
        if team == 0 { (self.lo, self.mid - gap) } else { (self.mid + gap, self.hi) }
    }

    /// Shortest flight from `x` to `to` that passes over the net, or 0 if
    /// it doesn't cross it.
    fn clear_time(&self, x: f32, to: f32, air: f32) -> f32 {
        let f = (self.mid - x) / (to - x);
        if !(0.0..=1.0).contains(&f) {
            return 0.0;
        }
        let need = (self.net_top() + 0.6 - air).max(0.0);
        (2.0 * need / (tn().gravity * (f * (1.0 - f)).max(0.04))).sqrt()
    }
}

/// Carrot grab: where it lies or who's got it, and how the runner is doing.
struct Carrot {
    x: f32,
    lo: f32,
    hi: f32,
    holder: Option<usize>,
    // the 15 seconds have started
    started: bool,
    // seconds sprinting so far, sprinting for how much longer, worn out for how long
    burst: f32,
    sprint: f32,
    tired: f32,
    // until it may sprint or hop again, and can't be caught
    sprint_cool: f32,
    hop_cool: f32,
    grace: f32,
    // the chaser who is thinking for a moment
    smart: Option<usize>,
    smart_left: f32,
    smart_wait: f32,
    dir: f32,
    // its current speed, and seconds before it reconsiders its direction
    speed: f32,
    decide: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum GiftKind {
    Carrot,
    Prank,
    Balloon,
}

/// A present from one to the other.
struct Gift {
    giver: usize,
    taker: usize,
    kind: GiftKind,
    // where the prank box was left, strip x and screen top, and how long
    // since it went off
    box_x: f32,
    box_y: f32,
    t: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Game {
    Carrot,
    Gift,
    Contest,
    Cursor,
    Long,
    Domino,
    Tug,
    Tag,
    Samba,
    Volley,
}

impl Chat {
    fn new(id: u32, members: Vec<usize>, stage: Stage, timer: f32) -> Self {
        let talker = members[0];
        Chat { id, members, stage, timer, talker, turn: 0.0, bob: 0.0, strolls: 0, joined_walking: false, quiet: 0.0, games: 0, played: Vec::new(), watchers: Vec::new(), spots: Vec::new(), base: Vec::new(), offset: 0.0, pull: 0.0, pull_target: 0.0, it: 0, chase: 0, teams: (Vec::new(), Vec::new()), queue: Vec::new(), step: 0, wait: 0.0, scores: Vec::new(), winner: None, long: None, regroup: false, forced: None, recent: Vec::new(), tag_free: 0.0, tag_turn: 0.0, tug_grab: None, tug_held: 0.0, tug_clicks: 0, cuts: 0, cut_timer: 0.0, all_cheer: false, volley: None, mood: 0.0, carrot: None, samba_new: Vec::new(), gift: None }
    }
}

/// One go in the long jump: alone, or thrown by a partner.
#[derive(Clone, Copy, Debug)]
struct Turn {
    flyer: usize,
    thrower: Option<usize>,
}

struct Long {
    referee: usize,
    line: f32,
    dir: f32,
    ready: bool,
    turns: Vec<Turn>,
    next: usize,
    gap: f32,
    // on their way to the line
    running: Vec<Turn>,
    // planted and not looked at yet; the one being looked at, seconds left
    landed: Vec<usize>,
    judging: Option<(usize, f32)>,
    // throwers done so far, standing out of the way past the line
    thrown: f32,
}

pub(super) struct Chat {
    id: u32,
    members: Vec<usize>,
    stage: Stage,
    timer: f32,
    // whose turn to talk, seconds left of it, seconds to the next nod
    talker: usize,
    turn: f32,
    bob: f32,
    strolls: u8,
    // someone joined on the walk: a full talk when they get there
    joined_walking: bool,
    // seconds since anyone joined, and whether this chat has had its game
    quiet: f32,
    games: u8,
    played: Vec<Game>,
    // saw the game and came over: they join when it's done
    watchers: Vec<usize>,
    // the other games: places, where each one started, how far the rope has
    // moved, who's "it" (tag), the two sides of the rope
    spots: Vec<f32>,
    base: Vec<(usize, f32)>,
    offset: f32,
    // rope speed and the speed it's heading for, logical px/s
    pull: f32,
    pull_target: f32,
    it: usize,
    // who "it" is after, for now
    chase: usize,
    teams: (Vec<usize>, Vec<usize>),
    // jumping game: who's still to jump, what they're doing, how long for
    queue: Vec<usize>,
    step: u8,
    wait: f32,
    // best so far: jump height or seconds on the cursor, per member
    scores: Vec<(usize, f32)>,
    winner: Option<usize>,
    long: Option<Long>,
    // scattered by a game: get back together before talking
    regroup: bool,
    // started from the settings window: play this as soon as they're lined up
    forced: Option<Game>,
    // tag: who was "it" just now and can't be tagged straight back, and for
    // how much longer
    recent: Vec<usize>,
    tag_free: f32,
    // how long the one being chased has been on the other side
    tag_turn: f32,
    // the rope held with the cursor: where the cursor and the rope were then
    tug_grab: Option<(f32, f32)>,
    // seconds it's been held like that
    tug_held: f32,
    // clicks seen so far, and those landing on the middle of the rope lately
    tug_clicks: u32,
    cuts: u32,
    cut_timer: f32,
    // a game that ends with everyone hopping, no winner
    all_cheer: bool,
    volley: Option<Volley>,
    // seconds talking, towards splitting into talkers and players
    mood: f32,
    carrot: Option<Carrot>,
    // samba: still walking over to join the row
    samba_new: Vec<usize>,
    gift: Option<Gift>,
}

/// Chance per look that someone nearby joins a chat this big: a pair is
/// hard to join, a crowd draws people in.
fn join_chance(members: usize) -> f32 {
    let t = tn();
    match members {
        0..=2 => t.join_2,
        3 => t.join_3,
        4 => t.join_4,
        5 => t.join_5,
        6 => t.join_6,
        7 => t.join_7,
        _ => t.join_8,
    }
}

fn join_range(members: usize) -> f32 {
    (tn().join_range + members.saturating_sub(2) as f32 * tn().join_range_add).min(tn().join_range_max)
}

/// Seconds before it's up for a chat; a rest after a crowd counts for more
/// than anything that gets past the usual pause.
fn chat_cool(wk: &WalkState) -> f32 {
    if wk.crowd_rest > 0.0 {
        wk.social_cool.max(wk.crowd_rest + tn().overlap_cool + 1.0)
    } else {
        wk.social_cool
    }
}

/// How many play a game started by hand, if the team has that many.
const FORCED_PLAYERS: usize = 6;

fn grounded(wk: &WalkState) -> bool {
    matches!(wk.act, Act::Idle | Act::Walk)
}

impl WalkState {
    /// In a chat, on the ground: walk to the goal or stand facing the others.
    pub(super) fn social_step(&mut self, dt: f32) -> Option<(f32, bool, f32)> {
        if !grounded(self) {
            return None;
        }
        let r = self.social.as_mut()?;
        r.leaping = false;
        if r.moving {
            let d = r.goal - self.x;
            let s = r.speed * dt;
            if d.abs() > s {
                self.x += d.signum() * s;
                self.left = if r.face_lock { r.face_left } else { d < 0.0 };
                self.target = r.goal;
                self.act = Act::Walk;
                return Some((self.x, true, 0.0));
            }
            self.x = r.goal;
            r.moving = false;
        }
        self.left = r.face_left;
        self.act = Act::Idle;
        self.timer = 1.0;
        self.target = self.x;
        Some((self.x, false, 0.0))
    }

    /// Flat out or head first: how far round, and how deep.
    fn stuck_pose(&mut self) -> (f32, f32) {
        use std::f32::consts::{FRAC_PI_2, PI};
        let roll = self.rand();
        if roll < tn().long_head_first {
            (PI, 0.55)
        } else if roll < 0.7 {
            (FRAC_PI_2, 0.25)
        } else {
            (-FRAC_PI_2, 0.25)
        }
    }

    /// Came down hard: stuck in the ground like a long jumper, then hops out.
    pub(super) fn stick_in(&mut self) {
        self.plant_pose = self.stuck_pose();
        self.rolling = false;
        self.grip = false;
        self.feet = false;
        self.act = Act::Planted;
        self.timer = tn().plant_time;
        self.target = self.x;
    }

    /// Landed from a long jump: stuck in the ground, flat out or head first.
    pub(super) fn plant(&mut self) {
        let (rot, sink) = self.stuck_pose();
        self.act = Act::Idle;
        self.timer = 1.0;
        self.target = self.x;
        self.rolling = false;
        if let Some(r) = self.social.as_mut() {
            r.flying = false;
            r.stuck = true;
            r.rot_target = rot;
            r.sink_target = sink;
        }
    }

    /// Spin in the air, stay as planted, otherwise get back upright.
    fn update_pose(&mut self, dt: f32) {
        use std::f32::consts::{PI, TAU};
        // nothing left to stand on: down it comes
        if self.air > 0.0 && self.climb.is_none() && matches!(self.act, Act::Idle | Act::Walk) && !self.social.is_some_and(|s| s.perched) {
            self.act = Act::Fall;
            self.vx = 0.0;
            self.vy = 0.0;
        }
        if self.act == Act::Planted {
            let (rot, sink) = self.plant_pose;
            let k = (dt * 14.0).min(1.0);
            self.rot += ((rot - self.rot + PI).rem_euclid(TAU) - PI) * k;
            self.sink += (sink - self.sink) * k;
            self.feet = false;
            self.timer -= dt;
            if self.timer <= 0.0 {
                self.start_jump(tn().jump_height * tn().plant_hop);
            }
            return;
        }
        if self.act == Act::Held {
            self.rot = self.swing;
            self.feet = false;
            return;
        }
        if self.tumble != 0.0 {
            self.rot += self.tumble * dt;
            self.feet = false;
            return;
        }
        match self.social {
            Some(s) if s.flying => {
                self.rot += s.spin * dt;
                self.feet = false;
            }
            Some(s) if s.stuck => {
                self.feet = s.feet;
                let k = (dt * if s.feet { 9.0 } else { 14.0 }).min(1.0);
                self.rot += ((s.rot_target - self.rot + PI).rem_euclid(TAU) - PI) * k;
                self.sink += (s.sink_target - self.sink) * k;
            }
            _ => {
                let k = (dt * 8.0).min(1.0);
                let lean = self.social.map_or(0.0, |s| s.lean);
                if !self.rolling {
                    self.rot = (self.rot + PI).rem_euclid(TAU) - PI;
                }
                self.rot += (lean - self.rot) * k;
                self.sink -= self.sink * k;
                if self.rot.abs() < 0.02 {
                    self.feet = false;
                    self.grip = false;
                    self.rolling = false;
                }
            }
        }
    }

    /// Tag, dropped by hand: already running on the way down. True if it did.
    pub(super) fn tag_fall_run(&mut self, dt: f32) -> bool {
        let Some(r) = self.social.as_mut().filter(|r| r.inside && !r.flying) else { return false };
        self.vx = 0.0;
        if r.moving {
            let d = r.goal - self.x;
            let s = r.speed * dt;
            if d.abs() > 1e-4 {
                self.left = d < 0.0;
            }
            self.x = if d.abs() <= s { r.goal } else { self.x + d.signum() * s };
        }
        true
    }

    /// Leaping over someone: keeps moving while in the air.
    pub(super) fn social_hop(&mut self, dt: f32) {
        let Some(r) = self.social.as_mut().filter(|r| r.leaping && r.moving) else { return };
        let d = r.goal - self.x;
        let s = r.speed * dt;
        if r.face_lock {
            self.left = r.face_left;
        } else if d.abs() > 1e-4 {
            self.left = d < 0.0;
        }
        if d.abs() <= s {
            self.x = r.goal;
            r.moving = false;
        } else {
            self.x += d.signum() * s;
        }
    }

    /// A short quick bob, for talking and laughing.
    pub(super) fn start_bob(&mut self, amp: f32) {
        self.start_jump(amp);
        self.jump_dur = 0.32;
        self.jump = 0.32;
    }
}

impl CharacterOverlay {
    /// Forget all chats (the team changed, indices don't hold any more).
    pub(super) fn clear_chats(&mut self) {
        self.chats.clear();
        self.new_chats.clear();
        self.walk.social = None;
        for e in &mut self.extras {
            e.walk.social = None;
        }
    }

    pub(super) fn run_social(&mut self, dt: f32, mouse: [f32; 2]) {
        self.social_age += dt;
        let n = self.extras.len() + 1;
        for i in 0..n {
            let wk = self.walk_of(i);
            wk.social_cool = (wk.social_cool - dt).max(0.0);
            wk.dropped = (wk.dropped - dt).max(0.0);
            wk.team = n as f32;
            wk.crowd_rest = (wk.crowd_rest - dt).max(0.0);
            wk.update_pose(dt);
        }
        let mut chats = std::mem::take(&mut self.chats);
        let asked = GAME_NOW.lock().unwrap().take();
        if let Some(name) = asked {
            self.force_game(&mut chats, &name, n);
        }
        chats.retain_mut(|c| {
            let keep = self.prune(c, n) && self.advance(c, dt, mouse);
            if !keep {
                self.finish(c, n);
            }
            keep
        });

        chats.append(&mut self.new_chats);

        self.social_tick -= dt;
        if self.social_tick <= 0.0 {
            self.social_tick = tn().look_every;
            if self.race.is_none() && self.social_age >= tn().warm_alone {
                for c in chats.iter_mut().filter(|c| matches!(c.stage, Stage::Contest | Stage::Cursor | Stage::Long | Stage::Domino | Stage::Tug | Stage::Tag | Stage::Carrot | Stage::Volley | Stage::Cheer)) {
                    self.try_watch(c, n);
                }
                for c in chats.iter_mut() {
                    let full = c.members.len() >= self.group_cap();
                    if !matches!(c.stage, Stage::Notice | Stage::Stroll | Stage::Long | Stage::Domino | Stage::Tug | Stage::Tag | Stage::Carrot | Stage::Samba | Stage::Volley | Stage::Part) && self.overlap_join(c, n, full) {
                        continue;
                    }
                    if !full && matches!(c.stage, Stage::Talk | Stage::Stroll) {
                        self.try_join(c, n);
                    }
                }
                for c in chats.iter_mut().filter(|c| c.stage == Stage::Samba && c.step != 9) {
                    self.try_samba_join(c, n);
                }
                let room = chats.len() < (n / (tn().chat_divisor as usize).max(1)).max(1);
                chats.extend(self.find_pair(n, room));
            }
        }
        self.chats = chats;
    }

    /// Biggest a group may get: small ones only while they're settling in.
    fn group_cap(&self) -> usize {
        let cap = tn().max_group as usize;
        if self.social_age < tn().warm_small { cap.min(3) } else { cap }
    }

    /// Drop members who were picked up, called away and so on; false when
    /// too few are left to go on.
    fn prune(&mut self, c: &mut Chat, n: usize) -> bool {
        let id = c.id;
        let game = matches!(c.stage, Stage::Cursor | Stage::Long | Stage::Tug | Stage::Volley | Stage::Cheer);
        // tag: you can carry someone off and they're still in it when put down
        let tag = matches!(c.stage, Stage::Tag | Stage::Carrot | Stage::Gift | Stage::Samba | Stage::Long);
        let mut left = Vec::new();
        c.members.retain(|&i| {
            let stays = i < n && {
                let wk = self.walk_ref(i);
                // hanging off the cursor and dropping from it is the game
                // (not when someone's picked it up: that's the button held down)
                let carried = crate::backend::pointer::BUTTON_DOWN.load(std::sync::atomic::Ordering::Relaxed);
                let playing = (game && (wk.act == Act::Fall || (wk.act == Act::Held && (wk.hang > 0.0 || !carried))))
                    || (tag && matches!(wk.act, Act::Held | Act::Fall));
                wk.social.is_some_and(|s| s.chat == id) && !wk.circle && (playing || !matches!(wk.act, Act::Held | Act::Fall | Act::Pat | Act::Chase))
            };
            if !stays {
                left.push(i);
            }
            stays
        });
        for i in left.into_iter().filter(|&i| i < n) {
            self.leave(i, id);
        }
        let mut gone = Vec::new();
        c.watchers.retain(|&i| {
            let stays = i < n && {
                let wk = self.walk_ref(i);
                wk.social.is_some_and(|s| s.chat == id) && !wk.circle && !matches!(wk.act, Act::Held | Act::Fall | Act::Pat | Act::Chase)
            };
            if !stays {
                gone.push(i);
            }
            stays
        });
        for i in gone.into_iter().filter(|&i| i < n) {
            self.leave(i, id);
        }
        if !c.members.contains(&c.talker) {
            c.turn = 0.0;
        }
        c.members.len() >= 2
    }

    fn leave(&mut self, i: usize, id: u32) {
        let wk = self.walk_of(i);
        if wk.social.is_some_and(|s| s.chat == id) {
            wk.social = None;
            wk.social_cool = tn().cool_base + wk.rand() * tn().cool_rand;
        }
    }

    fn finish(&mut self, c: &Chat, n: usize) {
        let big = c.members.len() >= tn().big_group as usize;
        for &i in c.watchers.iter().filter(|&&i| i < n) {
            self.leave(i, c.id);
        }
        for &i in c.members.iter().filter(|&&i| i < n) {
            self.leave(i, c.id);
            if big {
                let wk = self.walk_of(i);
                wk.crowd_rest = tn().crowd_rest_base + wk.rand() * tn().crowd_rest_rand;
            }
        }
    }

    pub(super) fn width_of(&self, i: usize) -> f32 {
        let r = if i == 0 { self.hit_rect } else { self.extras[i - 1].hit_rect };
        (r[2] - r[0]).max(1.0)
    }

    fn on_screen(&self, i: usize) -> bool {
        let r = if i == 0 { self.hit_rect } else { self.extras[i - 1].hit_rect };
        // an empty box: not drawn yet
        r[2] > r[0] && r[2] >= 0.0 && r[0] <= self.view_w
    }

    fn enlist(&mut self, i: usize, id: u32) {
        let wk = self.walk_of(i);
        let face_left = wk.left;
        wk.social = Some(Social { chat: id, goal_px: 0.0, goal: 0.0, speed: 0.0, moving: false, face_left, happy: false, chasing: false, hop_cool: 0.0, flying: false, spin: 0.0, stuck: false, rot_target: 0.0, sink_target: 0.0, bump_cool: 0.0, queued: false, launch_px: 0.0, lean: 0.0, feet: false, pace: 1.0, pace_goal: 1.0, pace_wait: 0.0, leaping: false, inside: false, no_touch: false, no_pat: false, jumped: false, face_lock: false, perched: false, bounces: 0 });
    }

    /// Two standing close, on screen and free: maybe they notice each other.
    /// `room`: not too many chats going on (standing inside each other is
    /// worth one anyway).
    fn find_pair(&mut self, n: usize, room: bool) -> Option<Chat> {
        let free: Vec<(usize, f32, f32, f32)> = (0..n)
            .filter(|&i| {
                let wk = self.walk_ref(i);
                wk.act == Act::Idle && wk.climb.is_none() && wk.social.is_none() && wk.race.is_none() && !wk.circle && self.on_screen(i)
            })
            .map(|i| (i, self.strip_px(i), self.width_of(i), chat_cool(self.walk_ref(i))))
            .collect();
        let mut overlap: Option<(f32, usize, usize)> = None;
        let mut near: Option<(f32, usize, usize)> = None;
        let mut crowded: Option<(f32, usize, usize)> = None;
        for (k, p) in free.iter().enumerate() {
            for q in &free[k + 1..] {
                let d = (p.1 - q.1).abs();
                let size = (p.2 + q.2) * 0.5;
                let slot = if d < size * tn().overlap {
                    if p.3 <= tn().overlap_cool && q.3 <= tn().overlap_cool { &mut overlap } else { &mut crowded }
                } else if d < size * tn().near_range && p.3 <= 0.0 && q.3 <= 0.0 {
                    &mut near
                } else {
                    continue;
                };
                if slot.is_none_or(|s| d < s.0) {
                    *slot = Some((d, p.0, q.0));
                }
            }
        }
        let roll = self.walk.rand();
        let pair = match (overlap, near) {
            (Some(o), _) if roll < tn().overlap_chance => o,
            (_, Some(o)) if room && roll < tn().near_chance => o,
            _ => {
                if let Some((d, a, b)) = crowded {
                    if self.walk.rand() < tn().step_chance {
                        let (i, other) = if self.walk.rand() < 0.5 { (a, b) } else { (b, a) };
                        self.step_aside(i, other, d);
                    }
                }
                return None;
            }
        };
        // either may be the one who notices first
        let (a, b) = if self.walk.rand() < 0.5 { (pair.1, pair.2) } else { (pair.2, pair.1) };
        self.chat_id = self.chat_id.wrapping_add(1);
        let id = self.chat_id;
        self.enlist(a, id);
        self.enlist(b, id);
        let timer = 1.2 + self.walk.rand() * 0.8;
        self.face(a, b);
        log::debug!("chat {id}: {a} and {b}{}", if pair.0 < self.width_of(a) * tn().overlap { " (were inside each other)" } else { "" });
        Some(Chat::new(id, vec![a, b], Stage::Notice, timer))
    }

    /// Standing inside someone with nothing to say: one shuffles over.
    fn step_aside(&mut self, i: usize, other: usize, d: f32) {
        let (x, o) = (self.strip_px(i), self.strip_px(other));
        let far = (self.width_of(i) + self.width_of(other)) * 0.5 * 0.8;
        let dist = (far - d).max(30.0);
        let mut dir = if x < o {
            -1.0
        } else if x > o {
            1.0
        } else if self.walk.rand() < 0.5 {
            -1.0
        } else {
            1.0
        };
        if !(0.0..=self.strip_width()).contains(&(x + dir * dist)) {
            dir = -dir;
        }
        let step = dir * dist * 2.0 / self.view_w.max(1.0);
        log::debug!("chat: {i} steps aside from {other}");
        let wk = self.walk_of(i);
        wk.target = wk.x + step;
        wk.act = Act::Walk;
    }

    /// Someone free close to a chat: it may catch their eye. Joining starts
    /// the chat's time over; enough of them set off a race.
    fn try_join(&mut self, c: &mut Chat, n: usize) {
        let spots: Vec<f32> = c.members.iter().map(|&i| self.strip_px(i)).collect();
        let size = self.width_of(c.members[0]);
        let free: Vec<(usize, f32)> = (0..n)
            .filter(|&i| {
                let wk = self.walk_ref(i);
                grounded(wk) && wk.climb.is_none() && wk.social.is_none() && wk.race.is_none() && !wk.circle && wk.energy >= tn().energy_join_need && chat_cool(wk) <= 0.0 && self.on_screen(i)
            })
            .map(|i| {
                let x = self.strip_px(i);
                (i, spots.iter().map(|p| (p - x).abs()).fold(f32::MAX, f32::min))
            })
            .filter(|f| f.1 < size * join_range(c.members.len()))
            .collect();
        let Some(&(i, _)) = free.iter().min_by(|a, b| a.1.total_cmp(&b.1)) else { return };
        let moving = c.stage == Stage::Stroll || self.walk_ref(i).act == Act::Walk;
        let chance = join_chance(c.members.len()) * if moving { tn().walk_join } else { 1.0 };
        if self.walk.rand() >= chance {
            return;
        }
        self.spend(i, tn().energy_join_cost);
        self.join(c, i);
    }

    /// Someone standing inside a member of a chat: a chat right there is
    /// hard to miss, so they join; if they've only just had one, they
    /// shuffle out of the way instead. True if it dealt with someone.
    fn overlap_join(&mut self, c: &mut Chat, n: usize, full: bool) -> bool {
        let mut hit: Option<(f32, usize, usize)> = None;
        for i in 0..n {
            let wk = self.walk_ref(i);
            if wk.act != Act::Idle || wk.social.is_some() || wk.race.is_some() || wk.circle || !self.on_screen(i) {
                continue;
            }
            let x = self.strip_px(i);
            for &m in &c.members {
                let d = (x - self.strip_px(m)).abs();
                let range = if self.walk_ref(i).dropped > 0.0 { tn().drop_range } else { tn().overlap };
                if d < (self.width_of(i) + self.width_of(m)) * 0.5 * range && hit.is_none_or(|h| d < h.0) {
                    hit = Some((d, i, m));
                }
            }
        }
        let Some((d, i, m)) = hit else { return false };
        // put right there by hand: no question about it
        if self.walk_ref(i).dropped > 0.0 {
            self.join(c, i);
        } else if !full && chat_cool(self.walk_ref(i)) <= tn().overlap_cool {
            if self.walk.rand() < tn().overlap_chance {
                self.join(c, i);
            }
        } else if self.walk.rand() < tn().step_chance {
            self.step_aside(i, m, d);
        }
        true
    }

    fn join(&mut self, c: &mut Chat, i: usize) {
        c.quiet = 0.0;
        self.enlist(i, c.id);
        if !matches!(c.stage, Stage::Notice | Stage::Approach | Stage::Talk | Stage::Stroll | Stage::Part | Stage::RaceCall | Stage::RaceWait) {
            if let Some(r) = self.walk_of(i).social.as_mut() {
                r.no_pat = true;
            }
        }
        c.members.push(i);
        self.face(i, c.members[0]);
        self.walk_of(i).start_bob(tn().jump_height * 0.2);
        log::debug!("chat {}: {i} joins, now {}", c.id, c.members.len());
        match c.stage {
            Stage::Stroll => {
                self.catch_up(c, i);
                c.joined_walking = true;
            }
            // straight into the game
            Stage::Cursor => {
                if let Some(r) = self.walk_of(i).social.as_mut() {
                    r.chasing = true;
                }
                c.scores.push((i, 0.0));
            }
            _ => {
                // nobody waits for them and nobody shuffles along: the
                // newcomer walks round to the end of the row, looking in
                self.catch_up(c, i);
                let mean = c.members.iter().filter(|&&m| m != i).map(|&m| self.strip_px(m)).sum::<f32>() / (c.members.len() - 1).max(1) as f32;
                let left_of_row = self.strip_px(i) < mean;
                let pace = 0.11 + self.walk.rand() * 0.05;
                if let Some(r) = self.walk_of(i).social.as_mut() {
                    r.face_left = !left_of_row;
                    r.speed = pace;
                }
                if c.stage == Stage::Contest {
                    c.queue.push(i);
                } else if c.stage == Stage::Talk {
                    c.timer = self.talk_length() + Self::crowd_talk(c);
                }
            }
        }
    }

    /// One of a small group suggests a race; the others agree.
    fn call_race(&mut self, c: &mut Chat) {
        let caller = c.members[(self.walk.rand() * c.members.len() as f32) as usize % c.members.len()];
        log::debug!("chat {}: {} together, {caller} calls a race!", c.id, c.members.len());
        c.it = caller;
        c.talker = caller;
        self.look_at_talker(c);
        let hop = tn().jump_height * 0.7;
        let wk = self.walk_of(caller);
        wk.gesture = true;
        if let Some(r) = wk.social.as_mut() {
            r.happy = true;
        }
        if grounded(wk) {
            wk.start_jump(hop);
        }
        c.turn = 1.2;
        c.bob = 0.0;
        c.timer = tn().race_call;
        c.stage = Stage::RaceCall;
    }

    /// `i` looks towards `other`.
    fn face(&mut self, i: usize, other: usize) {
        let left = self.strip_px(other) < self.strip_px(i);
        if let Some(r) = self.walk_of(i).social.as_mut() {
            r.face_left = left;
        }
    }

    fn go(&mut self, i: usize, goal_px: f32, speed: f32) {
        if let Some(r) = self.walk_of(i).social.as_mut() {
            r.goal_px = goal_px;
            r.speed = speed;
            r.moving = true;
            r.happy = false;
        }
    }

    /// `go`, and still looking that way after getting there (the others may
    /// take longer).
    fn go_facing(&mut self, i: usize, goal_px: f32, speed: f32) {
        let here = self.strip_px(i);
        self.go(i, goal_px, speed);
        if let Some(r) = self.walk_of(i).social.as_mut() {
            if (goal_px - here).abs() > 1.0 {
                r.face_left = goal_px < here;
            }
        }
    }

    fn arrived(&self, i: usize) -> bool {
        let wk = self.walk_ref(i);
        grounded(wk) && wk.social.is_some_and(|s| !s.moving)
    }

    fn all_arrived(&self, c: &Chat) -> bool {
        c.members.iter().all(|&i| self.arrived(i))
    }

    fn goal_of(&self, i: usize) -> f32 {
        self.walk_ref(i).social.map_or(0.0, |s| s.goal_px)
    }

    pub(super) fn strip_width(&self) -> f32 {
        crate::backend::compositor::current().strip().map_or(self.view_w, |s| s.width.max(self.view_w))
    }

    fn crowd_talk(c: &Chat) -> f32 {
        c.members.len().saturating_sub(2) as f32 * tn().crowd_talk
    }

    fn talk_length(&mut self) -> f32 {
        tn().talk_min + self.walk.rand() * tn().talk_rand
    }

    /// Joining a group on the move: head for the end of its row, a little
    /// faster than it walks.
    fn catch_up(&mut self, c: &Chat, i: usize) {
        let others: Vec<usize> = c.members.iter().copied().filter(|&m| m != i).collect();
        let goals: Vec<f32> = others.iter().map(|&m| self.goal_of(m)).collect();
        let mean = others.iter().map(|&m| self.strip_px(m)).sum::<f32>() / others.len() as f32;
        let gap = self.width_of(i) * 0.72;
        let goal = if self.strip_px(i) < mean {
            goals.iter().copied().fold(f32::MAX, f32::min) - gap
        } else {
            goals.iter().copied().fold(f32::MIN, f32::max) + gap
        };
        self.go(i, goal.clamp(0.0, self.strip_width()), 0.18);
    }

    /// Side by side in a row around their middle, in the order they stand.
    /// Whoever's in the happy pose stays in it.
    fn arrange(&mut self, c: &Chat) {
        let mut row: Vec<(usize, f32)> = c.members.iter().map(|&i| (i, self.strip_px(i))).collect();
        row.sort_by(|a, b| a.1.total_cmp(&b.1));
        let count = row.len() as f32;
        let mid = row.iter().map(|r| r.1).sum::<f32>() / count;
        let size = row.iter().map(|r| self.width_of(r.0)).sum::<f32>() / count;
        let gap = size * 0.72;
        for (k, &(i, _)) in row.iter().enumerate() {
            let happy = self.walk_ref(i).social.is_some_and(|r| r.happy);
            let slot = mid + (k as f32 - (count - 1.0) * 0.5) * gap;
            self.go(i, slot, 0.16);
            if let Some(r) = self.walk_of(i).social.as_mut() {
                r.happy = happy;
                // waiting for the others: towards the middle of the row
                if (slot - mid).abs() > 1.0 {
                    r.face_left = mid < slot;
                }
            }
        }
    }

    /// Everyone looks at whoever is talking; the talker at the rest.
    fn look_at_talker(&mut self, c: &Chat) {
        let talker = self.goal_of(c.talker);
        let mean = c.members.iter().map(|&i| self.goal_of(i)).sum::<f32>() / c.members.len() as f32;
        for &i in &c.members {
            let own = self.goal_of(i);
            let at = if i == c.talker { mean } else { talker };
            if (at - own).abs() > 1.0 {
                if let Some(r) = self.walk_of(i).social.as_mut() {
                    r.face_left = at < own;
                }
            }
        }
    }

    fn start_talk(&mut self, c: &mut Chat, length: f32) {
        log::debug!("chat {}: {:?} talk {length:.1}s", c.id, c.members);
        c.talker = c.members[0];
        self.look_at_talker(c);
        c.stage = Stage::Talk;
        c.timer = length + Self::crowd_talk(c);
        c.turn = 0.0;
        c.bob = 0.0;
        let size = c.members.len();
        if self.race.is_none()
            && self.social_age >= tn().warm_race
            && self.race_quiet <= 0.0
            && size >= tn().race_group_min as usize
            && size <= tn().race_group_max as usize
            && self.walk.rand() < tn().race_chance
        {
            self.call_race(c);
        } else if size == 2 && self.race.is_none() && self.social_age >= tn().warm_small && !c.played.contains(&Game::Gift) && self.walk.rand() < tn().gift_chance {
            c.played.push(Game::Gift);
            self.start_gift(c);
        }
    }

    /// Taking turns to talk: the happy pose, a gesture, laughing, nodding.
    fn chatter(&mut self, c: &mut Chat, dt: f32) {
        c.turn -= dt;
        if c.turn <= 0.0 {
            // next one in the row who's in place (a newcomer may still be walking)
            let at = c.members.iter().position(|&i| i == c.talker).map_or(0, |k| k + 1);
            let count = c.members.len();
            if let Some(next) = (0..count).map(|k| c.members[(at + k) % count]).find(|&i| self.arrived(i)) {
                c.talker = next;
            }
            c.turn = 1.0 + self.walk.rand() * 1.6;
            self.look_at_talker(c);
            let roll = self.walk.rand();
            let laugh = roll < 0.15;
            for &i in &c.members {
                let talking = i == c.talker;
                let in_place = self.arrived(i);
                let wk = self.walk_of(i);
                if let Some(r) = wk.social.as_mut() {
                    r.happy = talking || laugh;
                }
                if laugh && in_place {
                    wk.start_bob(tn().jump_height * 0.15);
                }
            }
            if !laugh && roll < 0.45 {
                self.walk_of(c.talker).gesture = true;
            }
        }
        // nodding along while talking
        c.bob -= dt;
        if c.bob <= 0.0 {
            let amp = tn().jump_height * (0.05 + self.walk.rand() * 0.04);
            c.bob = 0.5 + self.walk.rand() * 0.6;
            if self.arrived(c.talker) {
                self.walk_of(c.talker).start_bob(amp);
            }
        }
    }

    /// Off for a walk together, keeping their places in the row.
    fn stroll(&mut self, c: &mut Chat) {
        log::debug!("chat {}: stroll", c.id);
        let width = self.strip_width();
        let spots: Vec<f32> = c.members.iter().map(|&i| self.strip_px(i)).collect();
        let lo = spots.iter().copied().fold(f32::MAX, f32::min);
        let hi = spots.iter().copied().fold(f32::MIN, f32::max);
        let mut dist = tn().stroll_min + self.walk.rand() * tn().stroll_rand;
        let mut dir = if self.walk.rand() < 0.5 { -1.0 } else { 1.0 };
        let room = |d: f32| (if d < 0.0 { lo } else { width - hi } - tn().edge_px).max(0.0);
        if room(dir) < dist {
            if room(-dir) > room(dir) {
                dir = -dir;
            }
            dist = dist.min(room(dir));
        }
        for (k, &i) in c.members.clone().iter().enumerate() {
            self.go_facing(i, spots[k] + dir * dist, 0.12);
        }
        c.strolls += 1;
        c.stage = Stage::Stroll;
    }

    fn spend(&mut self, i: usize, amount: f32) {
        let wk = self.walk_of(i);
        wk.energy = (wk.energy - amount).max(0.0);
    }

    /// A game for this many that they haven't played yet: the closer its
    /// minimum is to their number, the likelier.
    fn choose_game(&mut self, played: &[Game], len: usize) -> Option<Game> {
        if !played.contains(&Game::Cursor) && self.walk.rand() < tn().cursor_game {
            return Some(Game::Cursor);
        }
        let pool: Vec<(Game, f32)> = [
            (Game::Contest, tn().game_min as usize),
            (Game::Long, tn().long_min as usize),
            (Game::Domino, tn().party_min as usize),
            (Game::Tug, tn().party_min as usize),
            (Game::Tag, tn().party_min as usize),
            (Game::Carrot, tn().carrot_min as usize),
            (Game::Samba, tn().samba_min as usize),
            (Game::Volley, tn().volley_min as usize),
        ]
        .iter()
        .filter(|g| len >= g.1 && !played.contains(&g.0))
        .map(|g| {
            // samba isn't for a handful: the more there are the likelier
            let weight = if g.0 == Game::Samba {
                let t = ((len - g.1) as f32 / (tn().samba_big - g.1 as f32).max(1.0)).clamp(0.0, 1.0);
                tn().samba_weight_small + (tn().samba_weight_big - tn().samba_weight_small) * t
            } else {
                (1.0 + (len - g.1) as f32).powf(-tn().game_fit)
            };
            (g.0, weight)
        })
        .collect();
        let mut roll = self.walk.rand() * pool.iter().map(|g| g.1).sum::<f32>();
        for &(game, weight) in &pool {
            roll -= weight;
            if roll <= 0.0 {
                return Some(game);
            }
        }
        pool.last().map(|g| g.0)
    }

    /// Start a game this group hasn't played yet, if it has the energy.
    fn next_game(&mut self, c: &mut Chat) -> bool {
        let mean = c.members.iter().map(|&i| self.walk_ref(i).energy).sum::<f32>() / c.members.len() as f32;
        if mean < tn().energy_game_need || self.social_age < tn().warm_small {
            return false;
        }
        let Some(game) = self.choose_game(&c.played, c.members.len()) else {
            return false;
        };
        c.played.push(game);
        for &i in &c.members.clone() {
            self.spend(i, tn().energy_game_cost);
        }
        self.start_game(c, game);
        true
    }

    /// A big group doesn't all want the same: a few go off to play, the
    /// others stay and talk.
    fn mood_split(&mut self, c: &mut Chat) -> bool {
        let len = c.members.len();
        let min = tn().game_min as usize;
        if len < min + 2 {
            return false;
        }
        let keen = (0..len).filter(|_| self.walk.rand() < tn().mood_game).count();
        let count = keen.clamp(min, len - 2);
        // the livelier end of the row goes, nobody has to cross the others
        let row = self.sorted_row(c);
        let energy = |m: &[usize]| m.iter().map(|&i| self.walk_ref(i).energy).sum::<f32>() / m.len() as f32;
        let (first, rest) = row.split_at(count);
        let (others, last) = row.split_at(len - count);
        let (players, talkers) = if energy(first) >= energy(last) {
            (first.to_vec(), rest.to_vec())
        } else {
            (last.to_vec(), others.to_vec())
        };
        let mean = energy(&players);
        log::debug!("chat {}: mood split? {count} would play, energy {mean:.2}", c.id);
        if mean < tn().energy_game_need * 0.7 {
            return false;
        }
        let Some(game) = self.choose_game(&c.played, count) else {
            return false;
        };
        self.chat_id = self.chat_id.wrapping_add(1);
        let id = self.chat_id;
        let mut other = Chat::new(id, players, Stage::Approach, 0.0);
        other.played = c.played.clone();
        other.forced = Some(game);
        log::debug!("chat {}: {:?} go off to play {game:?} as chat {id}, {talkers:?} keep talking", c.id, other.members);
        for &i in &other.members.clone() {
            self.spend(i, tn().energy_game_cost);
            if let Some(r) = self.walk_of(i).social.as_mut() {
                r.chat = id;
            }
        }
        c.members = talkers;
        c.talker = c.members[0];
        c.turn = 0.0;
        // the ones staying stay where they are
        self.arrange(&other);
        for &i in &other.members {
            if let Some(r) = self.walk_of(i).social.as_mut() {
                r.happy = false;
            }
        }
        let size = self.row_size(c);
        let mean_of = |this: &Self, m: &[usize]| m.iter().map(|&i| this.goal_of(i)).sum::<f32>() / m.len() as f32;
        let away = if mean_of(self, &other.members) < mean_of(self, &c.members) { -1.0 } else { 1.0 };
        for i in other.members.clone() {
            let x = self.goal_of(i);
            self.steer(i, x + away * size * 1.5, 0.16);
        }
        self.new_chats.push(other);
        true
    }

    fn start_game(&mut self, c: &mut Chat, game: Game) {
        for k in 0..c.members.len() {
            let wk = self.walk_of(c.members[k]);
            wk.notify_jump = None;
            if let Some(r) = wk.social.as_mut() {
                r.no_pat = true;
            }
        }
        match game {
            Game::Contest => self.start_contest(c),
            Game::Cursor => self.start_cursor_game(c),
            Game::Long => self.start_long(c),
            Game::Domino => self.start_domino(c),
            Game::Tug => self.start_tug(c),
            Game::Tag => self.start_tag(c),
            Game::Carrot => self.start_carrot(c),
            Game::Gift => self.start_gift(c),
            Game::Samba => self.start_samba(c),
            Game::Volley => self.start_volley(c),
        }
    }

    /// A game asked for by hand: whoever is nearest the middle of the
    /// screen plays right now, whatever they were up to and however tired.
    fn force_game(&mut self, chats: &mut Vec<Chat>, name: &str, n: usize) {
        let t = tn();
        let (game, need) = match name {
            "contest" => (Game::Contest, t.game_min),
            "cursor" => (Game::Cursor, t.game_min),
            "long" => (Game::Long, t.long_min),
            "domino" => (Game::Domino, t.party_min),
            "tug" => (Game::Tug, t.party_min),
            "tag" => (Game::Tag, t.party_min),
            "carrot" => (Game::Carrot, t.carrot_min),
            "gift" => (Game::Gift, t.gift_min),
            "samba" => (Game::Samba, t.samba_min),
            "volley" => (Game::Volley, t.volley_min),
            _ => return,
        };
        let need = need as usize;
        if self.race.is_some() || n < need {
            log::debug!("game {name}: can't start now ({n} characters, needs {need})");
            return;
        }
        // whoever is already playing carries on; the game goes to the others
        let playing: Vec<usize> = chats
            .iter()
            .filter(|c| {
                c.forced.is_some()
                    || matches!(c.stage, Stage::Contest | Stage::Cursor | Stage::Long | Stage::Domino | Stage::Tug | Stage::Tag | Stage::Carrot | Stage::Gift | Stage::Samba | Stage::Volley | Stage::Cheer | Stage::RaceCall | Stage::RaceWait)
            })
            .flat_map(|c| c.members.iter().copied())
            .collect();
        let middle = self.view[0] * self.view_w * 0.5 + self.view_w * 0.5;
        let mut free: Vec<(usize, f32)> = (0..n)
            .filter(|&i| !playing.contains(&i) && self.walk_ref(i).climb.is_none() && !self.walk_ref(i).circle && grounded(self.walk_ref(i)))
            .map(|i| (i, (self.strip_px(i) - middle).abs()))
            .collect();
        free.sort_by(|a, b| a.1.total_cmp(&b.1));
        if free.len() < need {
            log::debug!("game {name}: only {} free, needs {need}", free.len());
            return;
        }
        let count = if game == Game::Gift { 2 } else if game == Game::Volley { free.len() } else { need.max(FORCED_PLAYERS.min(free.len())) };
        let members: Vec<usize> = free.iter().take(count).map(|f| f.0).collect();
        self.chat_id = self.chat_id.wrapping_add(1);
        let id = self.chat_id;
        for &i in &members {
            self.enlist(i, id);
        }
        log::debug!("chat {id}: {name} by hand, {members:?}");
        let mut c = Chat::new(id, members, Stage::Approach, 0.0);
        c.forced = Some(game);
        self.arrange(&c);
        chats.push(c);
    }

    fn steer(&mut self, i: usize, x: f32, speed: f32) {
        if let Some(r) = self.walk_of(i).social.as_mut() {
            r.goal_px = x;
            r.speed = speed;
            r.moving = true;
        }
    }

    /// Like `steer`, and looking where it's going even once it gets there.
    fn run_to(&mut self, i: usize, x: f32, speed: f32) {
        let here = self.strip_px(i);
        if let Some(r) = self.walk_of(i).social.as_mut() {
            if (x - here).abs() > 1.0 {
                r.face_left = x < here;
            }
        }
        self.steer(i, x, speed);
    }

    /// Down flat, falling towards `dir`.
    fn topple(&mut self, i: usize, dir: f32) {
        if let Some(r) = self.walk_of(i).social.as_mut() {
            r.stuck = true;
            r.rot_target = -dir * std::f32::consts::FRAC_PI_2;
            // lying on the ground, not half under it
            r.sink_target = -0.35;
            r.feet = true;
            r.happy = false;
        }
    }

    fn sorted_row(&self, c: &Chat) -> Vec<usize> {
        let mut row = c.members.clone();
        row.sort_by(|a, b| self.strip_px(*a).total_cmp(&self.strip_px(*b)));
        row
    }

    fn row_size(&self, c: &Chat) -> f32 {
        c.members.iter().map(|&i| self.width_of(i)).sum::<f32>() / c.members.len() as f32
    }

    /// Domino: the row goes down one after the other from one end; now and
    /// then the last one stays standing and is very pleased about it.
    fn start_domino(&mut self, c: &mut Chat) {
        log::debug!("chat {}: domino", c.id);
        let mut row = self.sorted_row(c);
        c.offset = if self.walk.rand() < 0.5 { 1.0 } else { -1.0 };
        if c.offset < 0.0 {
            row.reverse();
        }
        c.scores.clear();
        if self.walk.rand() < tn().domino_survive {
            if let Some(last) = row.pop() {
                c.scores.push((last, 1.0));
            }
        }
        c.queue = row;
        // shoulder to shoulder first, or nobody knocks anybody over
        self.arrange(c);
        c.wait = 0.0;
        c.step = 9;
        c.stage = Stage::Domino;
    }

    fn run_domino(&mut self, c: &mut Chat, dt: f32) {
        c.wait -= dt;
        c.queue.retain(|i| c.members.contains(i));
        if c.wait > 0.0 {
            return;
        }
        match c.step {
            9 => {
                if self.all_arrived(c) {
                    c.wait = 0.6;
                    c.step = 0;
                }
            }
            0 => {
                if c.queue.is_empty() {
                    c.wait = tn().domino_rest;
                    c.step = 1;
                } else {
                    let i = c.queue.remove(0);
                    self.topple(i, c.offset);
                    c.wait = tn().domino_gap;
                }
            }
            _ => self.end_game(c),
        }
    }

    /// The rope of a tug of war in play: both ends, surface px (x0, y0, x1,
    /// y1), held at hand height by the one at the back of each side.
    fn rope_of(&self, c: &Chat) -> Option<[f32; 4]> {
        if c.stage != Stage::Tug || c.step != 1 {
            return None;
        }
        let n = self.extras.len() + 1;
        let hand = |i: usize| {
            let r = if i == 0 { self.hit_rect } else { self.extras[i - 1].hit_rect };
            ((r[0] + r[2]) * 0.5, r[1] + (r[3] - r[1]) * 0.62)
        };
        let ends = |team: &Vec<usize>| -> Vec<(f32, f32)> {
            team.iter().filter(|&&i| i < n && c.members.contains(&i)).map(|&i| hand(i)).collect()
        };
        let (left, right) = (ends(&c.teams.0), ends(&c.teams.1));
        let a = left.iter().copied().min_by(|p, q| p.0.total_cmp(&q.0))?;
        let b = right.iter().copied().max_by(|p, q| p.0.total_cmp(&q.0))?;
        Some([a.0, a.1, b.0, b.1])
    }

    pub(super) fn rope_ends(&self) -> Vec<[f32; 4]> {
        self.chats.iter().filter_map(|c| self.rope_of(c)).collect()
    }

    /// The cursor is on the middle third of the rope.
    fn on_rope_middle(&self, c: &Chat, mouse: [f32; 2]) -> bool {
        let Some([x0, y0, x1, y1]) = self.rope_of(c) else { return false };
        let t = (mouse[0] - x0) / (x1 - x0).abs().max(1.0);
        (0.3..=0.7).contains(&t) && (mouse[1] - (y0 + (y1 - y0) * t)).abs() < 34.0
    }

    /// A press that started on the rope takes hold of it.
    fn rope_grab(&self, c: &Chat, mouse: [f32; 2]) -> Option<(f32, f32)> {
        use std::sync::atomic::Ordering::Relaxed;
        if !crate::backend::pointer::BUTTON_DOWN.load(Relaxed) || crate::backend::pointer::DRAGGING.load(Relaxed) {
            return None;
        }
        let (_, at, _) = self.press?;
        let [x0, y0, x1, y1] = self.rope_of(c)?;
        let t = ((at[0] - x0) / (x1 - x0).abs().max(1.0)).clamp(0.0, 1.0);
        let on_rope = at[0] >= x0 - 20.0 && at[0] <= x1 + 20.0 && (at[1] - (y0 + (y1 - y0) * t)).abs() < 30.0;
        on_rope.then_some((mouse[0], c.offset))
    }

    /// Tug of war: two sides facing each other, the whole line swaying until
    /// one side has pulled the other far enough, and those go down.
    fn start_tug(&mut self, c: &mut Chat) {
        log::debug!("chat {}: tug of war", c.id);
        let row = self.sorted_row(c);
        let half = row.len() / 2;
        let mid = row.iter().map(|&i| self.strip_px(i)).sum::<f32>() / row.len() as f32;
        let gap = self.row_size(c) * 0.72;
        c.base.clear();
        let left: Vec<usize> = row[..half].to_vec();
        let right: Vec<usize> = row[row.len() - half..].to_vec();
        for (k, &i) in left.iter().rev().enumerate() {
            c.base.push((i, mid - gap * (k as f32 + 0.6)));
        }
        for (k, &i) in right.iter().enumerate() {
            c.base.push((i, mid + gap * (k as f32 + 0.6)));
        }
        for &(i, x) in &c.base.clone() {
            self.go(i, x, 0.2);
        }
        // an odd one out has no side: walks calmly to the middle and watches
        // (would otherwise carry on with whatever it was doing)
        if row.len() % 2 == 1 {
            self.go_facing(row[half], mid, 0.14);
        }
        // the only way into this game is the rope
        for k in 0..c.members.len() {
            let i = c.members[k];
            let wk = self.walk_of(i);
            wk.notify_jump = None;
            if let Some(r) = wk.social.as_mut() {
                r.no_touch = true;
            }
        }
        c.teams = (left, right);
        c.tug_clicks = crate::backend::pointer::CLICKS.load(std::sync::atomic::Ordering::Relaxed);
        c.cuts = 0;
        c.offset = 0.0;
        c.pull = 0.0;
        c.pull_target = 0.0;
        c.wait = 0.0;
        c.step = 0;
        c.timer = tn().tug_time;
        c.scores.clear();
        c.regroup = true;
        c.stage = Stage::Tug;
    }

    fn run_tug(&mut self, c: &mut Chat, dt: f32, mouse: [f32; 2]) {
        c.wait -= dt;
        match c.step {
            0 => {
                if self.all_arrived(c) {
                    for (i, left) in c.teams.0.iter().map(|&i| (i, false)).chain(c.teams.1.iter().map(|&i| (i, true))).collect::<Vec<_>>() {
                        if let Some(r) = self.walk_of(i).social.as_mut() {
                            r.face_left = left;
                            // dragged backwards by the rope, still facing it
                            r.face_lock = true;
                        }
                    }
                    c.timer = tn().tug_time;
                    c.step = 1;
                }
            }
            1 => {
                // every so often one side digs in harder; the rope follows
                // that smoothly, it never jumps
                if c.wait <= 0.0 {
                    c.wait = 0.5 + self.walk.rand() * 0.5;
                    let mut force = 0.0;
                    for _ in &c.teams.0 {
                        force -= self.walk.rand();
                    }
                    for _ in &c.teams.1 {
                        force += self.walk.rand();
                    }
                    c.pull_target = (force * tn().tug_pull).clamp(-tn().tug_speed, tn().tug_speed);
                }
                c.pull += (c.pull_target - c.pull) * (dt * 3.0).min(1.0);
                // clicked to bits in the middle: it snaps and everybody goes flying
                let clicks = crate::backend::pointer::CLICKS.load(std::sync::atomic::Ordering::Relaxed);
                c.cut_timer -= dt;
                if clicks != c.tug_clicks {
                    c.tug_clicks = clicks;
                    if self.on_rope_middle(c, mouse) {
                        if c.cut_timer <= 0.0 {
                            c.cuts = 0;
                        }
                        c.cuts += 1;
                        c.cut_timer = 1.5;
                        log::debug!("chat {}: rope clicked {} times", c.id, c.cuts);
                    }
                }
                if c.cuts as f32 >= tn().tug_cut_clicks {
                    log::debug!("chat {}: the rope snaps", c.id);
                    for k in 0..c.members.len() {
                        let i = c.members[k];
                        let dir = if self.walk.rand() < 0.5 { -1.0 } else { 1.0 };
                        let distance = tn().tug_fling_min + self.walk.rand() * tn().tug_fling_rand;
                        let time = 0.7 + self.walk.rand() * 0.5;
                        if let Some(r) = self.walk_of(i).social.as_mut() {
                            r.lean = 0.0;
                        }
                        self.fling(i, dir, distance, time);
                    }
                    c.scores.clear();
                    c.wait = 1.6;
                    c.step = 2;
                    return;
                }
                // cheating: the rope goes where the cursor drags it
                if !crate::backend::pointer::BUTTON_DOWN.load(std::sync::atomic::Ordering::Relaxed) {
                    c.tug_grab = None;
                } else if c.tug_grab.is_none() {
                    c.tug_grab = self.rope_grab(c, mouse);
                    c.tug_held = 0.0;
                }
                if let Some((from, offset)) = c.tug_grab {
                    // they fight it hard at first and tire the longer it goes on
                    c.tug_held += dt;
                    let resist = (1.0 - c.tug_held / tn().tug_resist_time.max(0.01)).clamp(0.0, 1.0);
                    let want = offset + (mouse[0] - from) * tn().tug_cursor;
                    let gap = want - c.offset;
                    c.offset += gap * (dt * (1.0 + 11.0 * (1.0 - resist))).min(1.0);
                    c.offset -= gap.signum() * resist * tn().tug_speed * 0.6 * dt;
                    c.pull = 0.0;
                } else {
                    c.offset += c.pull * dt;
                }
                for k in 0..c.base.len() {
                    let (i, x) = c.base[k];
                    if c.members.contains(&i) {
                        self.steer(i, x + c.offset, 0.6);
                    }
                }
                // leaning back, heaving in time, the side that's winning more so
                let heave = (c.timer * 5.0).sin() * 0.05;
                let winning = c.pull / tn().tug_speed * 0.08;
                let lean = [(c.teams.0.clone(), tn().tug_lean + heave - winning), (c.teams.1.clone(), -(tn().tug_lean + heave + winning))];
                for (team, amount) in lean {
                    for i in team {
                        if let Some(r) = self.walk_of(i).social.as_mut() {
                            r.lean = amount;
                        }
                    }
                }
                if c.offset.abs() >= tn().tug_win || c.timer <= 0.0 {
                    let dir = if c.offset >= 0.0 { 1.0 } else { -1.0 };
                    let (winners, losers) = if dir > 0.0 { (c.teams.1.clone(), c.teams.0.clone()) } else { (c.teams.0.clone(), c.teams.1.clone()) };
                    log::debug!("chat {}: tug won by {winners:?}", c.id);
                    c.scores = winners.iter().filter(|i| c.members.contains(i)).map(|&i| (i, 1.0)).collect();
                    for &i in winners.iter().chain(losers.iter()) {
                        if let Some(r) = self.walk_of(i).social.as_mut() {
                            r.lean = 0.0;
                        }
                    }
                    for i in losers.into_iter().filter(|i| c.members.contains(i)) {
                        // the rope gives and they go flying backwards
                        let distance = tn().tug_fling_min + self.walk.rand() * tn().tug_fling_rand;
                        let time = 0.7 + self.walk.rand() * 0.4;
                        self.fling(i, -dir, distance, time);
                    }
                    c.wait = 1.6;
                    c.step = 2;
                }
            }
            _ => {
                let airborne = c.members.iter().any(|&i| self.walk_ref(i).social.is_some_and(|s| s.flying));
                if c.wait <= 0.0 && !airborne {
                    self.end_game(c);
                }
            }
        }
    }

    /// Tag: one is "it" and runs after the nearest; a touch passes it on.
    /// Whoever spent the least time being "it" wins.
    /// Volleyball in the widest window, an even number of characters: three
    /// stand on each other as the net, one is the ball, the rest are two
    /// equal teams.
    fn start_volley(&mut self, c: &mut Chat) {
        log::debug!("chat {}: volleyball", c.id);
        let mut all = c.members.clone();
        for k in (1..all.len()).rev() {
            let j = (self.walk.rand() * (k + 1) as f32) as usize % (k + 1);
            all.swap(k, j);
        }
        // a crowd gets a taller net; the teams must still come out equal
        let tall = all.len() >= tn().volley_big as usize;
        let high = if tall { 4 } else { 3 };
        if (all.len() - high - 1) % 2 == 1 {
            if let Some(i) = all.pop() {
                self.leave(i, c.id);
            }
        }
        c.members = all.clone();
        let (x, cw) = self.big_window();
        let (lo, hi, mid) = (x + 40.0, x + cw - 40.0, x + cw * 0.5);
        let net: Vec<usize> = all[..high].to_vec();
        let ball = all[high];
        let mut teams = [Vec::new(), Vec::new()];
        for (k, &i) in all[high + 1..].iter().enumerate() {
            teams[k % 2].push(i);
        }
        let mut homes = Vec::new();
        for (t, team) in teams.iter().enumerate() {
            let (a, b) = if t == 0 { (lo, mid) } else { (mid, hi) };
            for (k, &i) in team.iter().enumerate() {
                homes.push((i, a + (b - a) * (k + 1) as f32 / (team.len() + 1) as f32));
            }
        }
        let walk = tn().volley_gather;
        for &(i, x) in &homes {
            self.go(i, x, walk);
        }
        self.go(ball, x + cw * 0.35, walk);
        for &i in &net {
            self.go(i, mid, walk);
        }
        c.talker = all[0];
        c.turn = 0.0;
        c.bob = 0.0;
        c.wait = 2.5;
        for &i in &all {
            if let Some(r) = self.walk_of(i).social.as_mut() {
                r.no_touch = true;
            }
        }
        c.volley = Some(Volley { lofted: false, teams, homes, ball, net, fumbled: false, mid, lo, hi, score: [0, 0], server: 0, rest: 0.0, runner: None, to: 0.0, react: 0.0, run_speed: 0.0, top_speed: 0.0, rally: 0 });
        c.scores.clear();
        c.step = 9;
        c.timer = 60.0;
        c.regroup = true;
        c.stage = Stage::Volley;
    }

    /// A serve always clears the net.
    fn serve_time(&self, v: &Volley, to: f32) -> f32 {
        let x = self.strip_px(v.ball);
        let air = self.walk_ref(v.ball).air;
        v.clear_time(x, to, air).max(1.2)
    }

    /// Send the ball in an arc to `to`, landing in `time` seconds.
    fn ball_to(&mut self, ball: usize, to: f32, time: f32) {
        let x = self.strip_px(ball);
        let w = self.view_w.max(1.0);
        let wk = self.walk_of(ball);
        wk.vx = (to - x) * 2.0 / w / time;
        wk.vy = tn().gravity * time * 0.5;
        wk.air = wk.air.max(0.3);
        wk.jump = 0.0;
        wk.held_prev = None;
        wk.act = Act::Fall;
        let spin = if to < x { 8.0 } else { -8.0 };
        if let Some(r) = wk.social.as_mut() {
            r.flying = true;
            r.stuck = false;
            r.moving = false;
            r.spin = spin;
            r.happy = true;
        }
    }

    fn run_volley(&mut self, c: &mut Chat, dt: f32) {
        let Some(mut v) = c.volley.take() else { return };
        let intact = c.members.contains(&v.ball)
            && v.net.iter().all(|i| c.members.contains(i))
            && v.teams.iter().flatten().all(|i| c.members.contains(i));
        if !intact || c.timer <= 0.0 {
            self.end_game(c);
            return;
        }
        if c.step == 9 {
            // they walk over talking, and chat a little more once all are there
            self.chatter(c, dt);
            if self.all_arrived(c) {
                c.wait -= dt;
            }
            if self.all_arrived(c) && c.wait <= 0.0 {
                for k in 0..c.members.len() {
                    if let Some(r) = self.walk_of(c.members[k]).social.as_mut() {
                        r.happy = false;
                    }
                }
                // up onto each other's shoulders, and the first serve
                for k in 1..v.net.len() {
                    let i = v.net[k];
                    if let Some(r) = self.walk_of(i).social.as_mut() {
                        r.perched = true;
                    }
                    self.walk_of(i).air = 0.01;
                }
                for k in 0..2 {
                    for j in 0..v.teams[k].len() {
                        self.face(v.teams[k][j], v.net[0]);
                    }
                }
                log::debug!("chat {}: first serve", c.id);
                let at = self.strip_px(v.teams[0][0]);
                let time = self.serve_time(&v, at);
                self.ball_to(v.ball, at, time);
                c.step = 0;
                c.timer = 150.0;
            }
            c.volley = Some(v);
            return;
        }
        for k in 1..v.net.len() {
            let top = self.walk_of(v.net[k]);
            top.air += (k as f32 * 1.7 - top.air) * (dt * 6.0).min(1.0);
        }
        if v.rest > 0.0 {
            v.rest -= dt;
            if v.rest <= 0.0 {
                let at = self.strip_px(v.teams[v.server][0]);
                let time = self.serve_time(&v, at);
                self.ball_to(v.ball, at, time);
            }
            c.volley = Some(v);
            return;
        }
        let x = self.strip_px(v.ball);
        let side = usize::from(x >= v.mid);
        let (stuck, falling, air) = {
            let wk = self.walk_ref(v.ball);
            (wk.social.is_some_and(|s| s.stuck), wk.act == Act::Fall && wk.vy < 0.0, wk.air)
        };
        if stuck {
            // down on one side: a point to the other
            let scorer = 1 - side;
            v.score[scorer] += 1;
            v.server = scorer;
            v.rest = 1.0;
            v.rally = 0;
            v.fumbled = false;
            v.runner = None;
            log::debug!("chat {}: volleyball {} - {}", c.id, v.score[0], v.score[1]);
            if v.score[scorer] as f32 >= tn().volley_win {
                c.scores = v.teams[scorer].iter().map(|&i| (i, 1.0)).collect();
                self.end_game(c);
                return;
            }
            for k in 0..v.teams[scorer].len() {
                let hopper = self.walk_of(v.teams[scorer][k]);
                if grounded(hopper) {
                    hopper.start_jump(tn().jump_height * 0.5);
                }
            }
            c.volley = Some(v);
            return;
        }
        let w = self.view_w.max(1.0);
        let net_half = self.width_of(v.net[0]) * 0.25;
        let top = v.net_top();
        let wk = self.walk_of(v.ball);
        // off the sides of the window, wherever that takes it
        if wk.act == Act::Fall && ((x < v.lo && wk.vx < 0.0) || (x > v.hi && wk.vx > 0.0)) {
            wk.vx = -wk.vx * 0.8;
        }
        // into the net: it can't get through, and drops back on this side
        if wk.act == Act::Fall && wk.air < top && (x - v.mid).abs() < net_half && (x < v.mid) == (wk.vx > 0.0) && !(v.lofted && wk.vy > 0.0) {
            wk.vx = -wk.vx * 0.35;
            v.fumbled = false;
            log::debug!("chat {}: volley into the net", c.id);
        }
        let closest = |this: &Self, team: &[usize], at: f32| {
            team.iter().copied().min_by(|a, b| (this.strip_px(*a) - at).abs().total_cmp(&(this.strip_px(*b) - at).abs()))
        };
        // where it will really come down, walls included; the chaser follows
        // that as it changes, so a bounce or the net can still be played
        let wk = self.walk_ref(v.ball);
        if wk.act == Act::Fall {
            let g = tn().gravity;
            let t = (wk.vy + (wk.vy * wk.vy + 2.0 * g * (wk.air - 1.5).max(0.0)).sqrt()) / g;
            let mut land = x + wk.vx * w * 0.5 * t;
            if land > v.hi {
                land = v.hi - (land - v.hi) * 0.8;
            } else if land < v.lo {
                land = v.lo + (v.lo - land) * 0.8;
            }
            let to_side = usize::from(land >= v.mid);
            if !v.runner.is_some_and(|r| v.teams[to_side].contains(&r)) {
                if let Some(&(old, home)) = v.runner.and_then(|r| v.homes.iter().find(|h| h.0 == r)) {
                    self.run_to(old, home, 0.25);
                }
                v.runner = closest(self, &v.teams[to_side], land);
                v.react = 0.12;
                v.run_speed = 0.05;
                // more players cover more ground: each is given less time to do it
                v.top_speed = tn().volley_speed * (0.85 + self.walk.rand() * 0.3) / (v.teams[to_side].len() as f32).powf(0.25);
            }
            v.to = land;
        }
        if let Some(r) = v.runner {
            v.react -= dt;
            if v.react <= 0.0 {
                v.run_speed = (v.run_speed + tn().volley_accel * dt).min(v.top_speed);
                // nobody walks through the net
                let (a, b) = v.half(usize::from(v.teams[1].contains(&r)), net_half * 2.0);
                self.run_to(r, v.to.clamp(a, b), v.run_speed);
            }
        }
        // the ball has to come down on someone's head
        let player = closest(self, &v.teams[side], x);
        if let Some(player) = player.filter(|&p| falling && air < 2.2 && air > 0.5 && (self.strip_px(p) - x).abs() < tn().volley_reach) {
            // misjudged: the more of them and the longer it goes on, the likelier
            let fumble = tn().volley_fumble + (v.teams[side].len() as f32 - 1.0) * 0.02 + v.rally as f32 * 0.01;
            if v.fumbled || self.walk.rand() < fumble {
                v.fumbled = true;
                c.volley = Some(v);
                return;
            }
            // any power and pace: a hard one flies long and comes off the side,
            // a high one comes off the top
            let dir = if side == 0 { 1.0 } else { -1.0 };
            let (a, b) = if side == 0 { (v.mid + 100.0, v.hi - 60.0) } else { (v.lo + 60.0, v.mid - 100.0) };
            let power = if self.walk.rand() < 0.3 { self.walk.rand() * 500.0 } else { 0.0 };
            let to = a + self.walk.rand() * (b - a).max(0.0) + dir * power;
            v.rally += 1;
            let mut time = ((tn().volley_time_min + self.walk.rand() * tn().volley_time_rand) * 0.97f32.powi(v.rally)).max(0.5);
            // most are hit high enough; the rest go wherever, the net included
            v.lofted = self.walk.rand() >= tn().volley_net;
            if v.lofted {
                time = time.max(v.clear_time(x, to, air));
            }
            log::debug!("chat {}: volley hit by {player}", c.id);
            self.ball_to(v.ball, to, time);
            let hop = tn().jump_height * 0.5;
            let wk = self.walk_of(player);
            if grounded(wk) {
                wk.start_jump(hop);
            }
            // whoever is nearest reacts next frame; the hitter drifts home
            v.runner = None;
            if let Some(&(_, home)) = v.homes.iter().find(|h| h.0 == player) {
                self.run_to(player, home, 0.25);
            }
        }
        c.volley = Some(v);
    }

    /// Samba: in a row, hopping forward together a step at a time, looking
    /// right, then left, on the beat.
    /// Things drawn beside the characters: (kind, surface box, mirrored).
    pub(super) fn props(&self) -> Vec<(i32, [f32; 4], bool)> {
        let mut v: Vec<(i32, [f32; 4], bool)> = self.carrot_rects().into_iter().map(|r| (0, r, false)).collect();
        for c in self.chats.iter().filter(|c| c.stage == Stage::Gift) {
            let Some(g) = c.gift.as_ref() else { continue };
            let rect = |i: usize| if i == 0 { self.hit_rect } else { self.extras[i - 1].hit_rect };
            let (giver, taker) = (rect(g.giver), rect(g.taker));
            let height = (taker[3] - taker[1]).max(1.0);
            let size = height * 0.36;
            let middle = |r: [f32; 4]| (r[0] + r[2]) * 0.5;
            let dir = if middle(taker) >= middle(giver) { 1.0 } else { -1.0 };
            let hand = |r: [f32; 4], side: f32| {
                let x = middle(r) + side * height * 0.3;
                let top = r[1] + height * 0.55 - size * 0.5;
                [x - size * 0.5, top, x + size * 0.5, top + size]
            };
            match (c.step, g.kind) {
                (1, _) => v.push((1, hand(giver, dir), false)),
                (2, _) => v.push((1, hand(taker, -dir), false)),
                (3, GiftKind::Carrot) => v.push((0, hand(taker, -dir), false)),
                (3, GiftKind::Balloon) => {
                    let w = height * 0.32;
                    let x = middle(taker);
                    v.push((2, [x - w, taker[1] - height * 0.72, x + w, taker[1] + height * 0.08], false));
                }
                (3, GiftKind::Prank) => {
                    let x = g.box_x - self.view[0] * self.view_w * 0.5;
                    v.push((1, [x - size * 0.5, g.box_y, x + size * 0.5, g.box_y + size], false));
                    // shoots out at once, then quivers on its spring
                    let out = height * (0.75 + 0.06 * (g.t * 40.0).sin()) * (g.t / 0.08).min(1.0);
                    let face = taker[1] + height * 0.3;
                    let (a, b) = (x, x + dir * out);
                    let half = height * 0.17;
                    let top = g.box_y.min(face) + (face - g.box_y).abs() * 0.5 - half;
                    v.push((3, [a.min(b), top, a.max(b), top + half * 2.0], dir < 0.0));
                }
                _ => {}
            }
        }
        v
    }

    /// The carrot on the screen, surface pixels.
    pub(super) fn carrot_rects(&self) -> Vec<[f32; 4]> {
        self.chats.iter().filter(|c| c.stage == Stage::Carrot && c.step != 9).filter_map(|c| self.carrot_rect(c)).collect()
    }

    fn carrot_rect(&self, c: &Chat) -> Option<[f32; 4]> {
        let v = c.carrot.as_ref()?;
        let first = *c.members.first()?;
        let rect = |i: usize| if i == 0 { self.hit_rect } else { self.extras[i - 1].hit_rect };
        let body = rect(first);
        let size = (body[3] - body[1]).max(1.0) * 0.42;
        let half = size * 0.42;
        let (x, top) = match v.holder {
            // in its hand, on the side it's running towards
            Some(h) => {
                let r = rect(h);
                let reach = if h == 0 { self.opaque[0] } else { self.extras[h - 1].opaque[0] } * (r[2] - r[0]);
                let side = if self.walk_ref(h).left { -1.0 } else { 1.0 };
                ((r[0] + r[2]) * 0.5 + side * reach * 0.8, r[1] + (r[3] - r[1]) * 0.55 - size * 0.5)
            }
            None => (v.x - self.view[0] * self.view_w * 0.5, body[3] - size),
        };
        Some([x - half, top, x + half, top + size])
    }

    /// Carrot grab: everyone goes to the widest window, a carrot appears, and
    /// whoever holds it when the time's up wins.
    fn start_carrot(&mut self, c: &mut Chat) {
        log::debug!("chat {}: carrot grab", c.id);
        let (x, cw) = self.big_window();
        let (lo, hi) = (x + 40.0, x + cw - 40.0);
        // all over the window in no particular order: a slot each, somewhere
        // in it (well inside, the very ends can be out of a character's reach)
        let mut order = c.members.clone();
        for k in (1..order.len()).rev() {
            let j = (self.walk.rand() * (k + 1) as f32) as usize % (k + 1);
            order.swap(k, j);
        }
        let slot = (hi - lo - 220.0).max(0.0) / order.len() as f32;
        for (k, &i) in order.iter().enumerate() {
            let spot = lo + 110.0 + (k as f32 + self.walk.rand()) * slot;
            self.go_facing(i, spot, tn().tag_run * 1.2);
        }
        c.carrot = Some(Carrot { x: lo, lo, hi, holder: None, started: false, burst: 0.0, sprint: 0.0, tired: 0.0, sprint_cool: 0.0, hop_cool: 0.0, grace: 0.0, smart: None, smart_left: 0.0, smart_wait: tn().carrot_smart_every, dir: 1.0, speed: 0.0, decide: 0.0 });
        c.scores.clear();
        c.step = 9;
        c.timer = 60.0;
        c.regroup = true;
        c.stage = Stage::Carrot;
    }

    fn run_carrot(&mut self, c: &mut Chat, dt: f32) {
        let Some(mut v) = c.carrot.take() else { return };
        if c.step == 9 {
            for k in 0..c.members.len() {
                let i = c.members[k];
                let x = self.strip_px(i);
                let inside = x >= v.lo && x <= v.hi;
                if let Some(r) = self.walk_of(i).social.as_mut() {
                    r.inside |= inside;
                }
            }
            if self.all_arrived(c) {
                v.x = v.lo + 110.0 + self.walk.rand() * (v.hi - v.lo - 220.0).max(0.0);
                log::debug!("chat {}: a carrot at {:.0}", c.id, v.x);
                c.step = 0;
                c.timer = 30.0;
            } else if c.timer <= 0.0 {
                self.end_game(c);
                return;
            }
            c.carrot = Some(v);
            return;
        }
        if c.members.is_empty() {
            return;
        }
        let size = self.row_size(c);
        if v.holder.is_some_and(|h| !c.members.contains(&h)) {
            v.holder = None;
        }
        let Some(h) = v.holder else {
            if c.timer <= 0.0 {
                self.end_game(c);
                return;
            }
            // nobody has it: a mad dash
            let mut closest: Option<(usize, f32)> = None;
            for k in 0..c.members.len() {
                let i = c.members[k];
                let pace = self.tag_pace(i, dt);
                self.run_to(i, v.x, tn().carrot_rush * pace);
                let d = (self.strip_px(i) - v.x).abs();
                if grounded(self.walk_ref(i)) && closest.is_none_or(|n| d < n.1) {
                    closest = Some((i, d));
                }
            }
            if let Some((i, _)) = closest.filter(|n| n.1 < size * 0.4) {
                log::debug!("chat {}: {i} got the carrot", c.id);
                v.holder = Some(i);
                v.grace = tn().carrot_grace;
                v.burst = 0.0;
                v.sprint = 0.0;
                v.tired = 0.0;
                if !v.started {
                    v.started = true;
                    c.timer = tn().carrot_time;
                }
                v.decide = 0.0;
                v.speed = tn().carrot_run;
            }
            c.carrot = Some(v);
            return;
        };
        if c.timer <= 0.0 {
            log::debug!("chat {}: carrot time, {h} has it", c.id);
            c.scores = vec![(h, 1.0)];
            self.end_game(c);
            return;
        }
        v.grace -= dt;
        v.hop_cool -= dt;
        v.sprint_cool -= dt;
        v.tired = (v.tired - dt).max(0.0);
        let hx = self.strip_px(h);
        v.x = hx;
        // who is nearest, and who could actually grab it
        let mut nearest: Option<(usize, f32)> = None;
        let mut catcher: Option<(usize, f32)> = None;
        for &i in c.members.iter().filter(|&&i| i != h) {
            let d = (self.strip_px(i) - hx).abs();
            if nearest.is_none_or(|n| d < n.1) {
                nearest = Some((i, d));
            }
            if grounded(self.walk_ref(i)) && catcher.is_none_or(|n| d < n.1) {
                catcher = Some((i, d));
            }
        }
        // grabbed: it's theirs now, a moment's head start
        if let Some((n, _)) = catcher.filter(|t| v.grace <= 0.0 && t.1 < size * 0.3 && grounded(self.walk_ref(h))) {
            log::debug!("chat {}: {n} takes the carrot from {h}", c.id);
            v.holder = Some(n);
            v.grace = tn().carrot_grace;
            v.burst = 0.0;
            v.sprint = 0.0;
            v.tired = 0.0;
            v.smart = None;
            v.decide = 0.0;
            v.speed = tn().carrot_run;
            c.carrot = Some(v);
            return;
        }
        let Some((near, near_d)) = nearest else {
            c.carrot = Some(v);
            return;
        };
        // picks a way to go and sticks to it for a moment; turning every
        // frame is what makes a runner shake on the spot
        let others: Vec<usize> = c.members.iter().copied().filter(|&i| i != h).collect();
        let positions: Vec<f32> = others.iter().map(|&i| self.strip_px(i) - hx).collect();
        let room = |d: f32| if d > 0.0 { v.hi - hx } else { hx - v.lo };
        let clear = |d: f32| positions.iter().filter(|&&p| p * d > 0.0).map(|p| p.abs()).fold(f32::MAX, f32::min);
        v.decide -= dt;
        let at_wall = room(v.dir) < 70.0;
        if at_wall {
            v.dir = -v.dir;
            v.decide = 0.8;
        } else if v.decide <= 0.0 {
            let score = |d: f32| room(d).min(clear(d));
            v.dir = if score(1.0) >= score(-1.0) { 1.0 } else { -1.0 };
            v.decide = 0.6 + self.walk.rand() * 0.6;
        }
        // sprinting too long wears it out for a good while
        if v.sprint <= 0.0 && v.tired <= 0.0 && v.sprint_cool <= 0.0 && near_d < 320.0 {
            v.sprint_cool = 1.2;
            if self.walk.rand() < 0.6 {
                v.sprint = 1.0 + self.walk.rand();
            }
        }
        let mut wanted = if v.tired > 0.0 {
            v.burst = 0.0;
            v.sprint = 0.0;
            tn().carrot_slow
        } else if v.sprint > 0.0 {
            v.sprint -= dt;
            v.burst += dt;
            if v.burst >= tn().carrot_burst {
                log::debug!("chat {}: {h} is worn out", c.id);
                v.tired = tn().carrot_tired;
            }
            tn().carrot_sprint
        } else {
            v.burst = (v.burst - dt * 0.25).max(0.0);
            tn().carrot_run
        };
        // someone in the way, or turning back at a wall with them close: over
        let blocked = clear(v.dir) < size * 1.1 || (at_wall && near_d < size * 1.5);
        let leap = blocked && grounded(self.walk_ref(h)) && v.hop_cool <= 0.0 && v.tired <= 0.0 && self.walk.rand() < tn().carrot_hop_chance;
        if blocked && v.hop_cool <= 0.0 {
            v.hop_cool = 0.9;
        }
        if leap {
            wanted = wanted.max(tn().carrot_run) * 1.3;
        }
        v.speed += (wanted - v.speed) * (dt * 8.0).min(1.0);
        self.run_to(h, (hx + v.dir * 600.0).clamp(v.lo, v.hi), v.speed);
        if leap {
            let height = tn().jump_height * tn().carrot_hop;
            let wk = self.walk_of(h);
            wk.start_jump(height);
            if let Some(r) = wk.social.as_mut() {
                r.leaping = true;
            }
        }
        // every so often one of them thinks and heads it off
        v.smart_left -= dt;
        v.smart_wait -= dt;
        if v.smart_left <= 0.0 {
            v.smart = None;
        }
        if v.smart_wait <= 0.0 {
            v.smart = Some(if self.walk.rand() < 0.6 { near } else { others[(self.walk.rand() * others.len() as f32) as usize % others.len()] });
            v.smart_left = tn().carrot_smart_time;
            v.smart_wait = tn().carrot_smart_every * (0.7 + self.walk.rand() * 0.6);
        }
        // the others only notice where it went a moment later, and run on
        // past that spot instead of stopping and bunching up on it
        for &i in &others {
            let pace = self.tag_pace(i, dt);
            if v.smart == Some(i) {
                let ahead = (hx + v.dir * 80.0).clamp(v.lo, v.hi);
                self.run_to(i, ahead, tn().carrot_chase * tn().carrot_smart_speed);
                continue;
            }
            let x = self.strip_px(i);
            let wk = self.walk_of(i);
            let wait = tn().carrot_lag * (0.5 + wk.rand());
            let spread = (wk.rand() - 0.5) * 120.0;
            let Some(r) = wk.social.as_mut() else { continue };
            r.hop_cool -= dt;
            if r.hop_cool > 0.0 {
                r.speed = tn().carrot_chase * pace;
                continue;
            }
            r.hop_cool = wait;
            let goal = hx + (hx - x).signum() * 90.0 + spread;
            self.run_to(i, goal.clamp(v.lo, v.hi), tn().carrot_chase * pace);
        }
        c.carrot = Some(v);
    }

    /// Two of them: one hands the other a present and it goes well or it
    /// doesn't.
    fn start_gift(&mut self, c: &mut Chat) {
        let giver = usize::from(self.walk.rand() < 0.5);
        let (giver, taker) = (c.members[giver], c.members[1 - giver]);
        let roll = self.walk.rand();
        let kind = if roll < tn().gift_prank {
            GiftKind::Prank
        } else if roll < tn().gift_prank + tn().gift_balloon {
            GiftKind::Balloon
        } else {
            GiftKind::Carrot
        };
        log::debug!("chat {}: {giver} gives {taker} a present: {kind:?}", c.id);
        for k in 0..c.members.len() {
            if let Some(r) = self.walk_of(c.members[k]).social.as_mut() {
                r.no_pat = true;
                r.happy = false;
            }
        }
        c.gift = Some(Gift { giver, taker, kind, box_x: 0.0, box_y: 0.0, t: 0.0 });
        c.scores.clear();
        c.step = 0;
        c.wait = 0.0;
        c.timer = 40.0;
        c.stage = Stage::Gift;
    }

    fn run_gift(&mut self, c: &mut Chat, dt: f32) {
        let Some(mut g) = c.gift.take() else { return };
        if c.timer <= 0.0 || !c.members.contains(&g.giver) || !c.members.contains(&g.taker) {
            if let Some(r) = self.walk_of(g.taker).social.as_mut() {
                r.perched = false;
            }
            self.end_game(c);
            return;
        }
        c.wait -= dt;
        g.t += dt;
        let (giver, taker) = (g.giver, g.taker);
        let dir = if self.strip_px(taker) >= self.strip_px(giver) { 1.0 } else { -1.0 };
        let happy = |this: &mut Self, i: usize, on: bool| {
            if let Some(r) = this.walk_of(i).social.as_mut() {
                r.happy = on;
            }
        };
        match c.step {
            // held out towards the other, who looks
            0 => {
                if self.all_arrived(c) {
                    self.face(giver, taker);
                    self.face(taker, giver);
                    happy(self, giver, true);
                    let wk = self.walk_of(giver);
                    wk.gesture = true;
                    wk.start_bob(tn().jump_height * 0.2);
                    c.wait = 1.0;
                    c.step = 1;
                }
            }
            // taken, and turned over in the hands
            1 if c.wait <= 0.0 => {
                self.walk_of(taker).gesture = true;
                happy(self, taker, true);
                c.wait = 0.8;
                c.step = 2;
            }
            2 if c.wait <= 0.0 => {
                c.step = 3;
                g.t = 0.0;
                match g.kind {
                    GiftKind::Carrot => c.wait = 2.5,
                    GiftKind::Balloon => {
                        c.wait = tn().gift_balloon_time;
                        let wk = self.walk_of(taker);
                        if let Some(r) = wk.social.as_mut() {
                            r.perched = true;
                        }
                        wk.air = 0.01;
                    }
                    GiftKind::Prank => {
                        // where the box stays once the taker is gone
                        let r = if taker == 0 { self.hit_rect } else { self.extras[taker - 1].hit_rect };
                        let height = (r[3] - r[1]).max(1.0);
                        g.box_x = self.strip_px(taker) - dir * height * 0.3;
                        g.box_y = r[1] + height * 0.55 - height * 0.18;
                        c.wait = 0.45;
                        let speed = tn().gift_punch_speed + self.walk.rand() * tn().gift_punch_rand;
                        let spin = dir * -(12.0 + self.walk.rand() * 6.0);
                        let wk = self.walk_of(taker);
                        wk.vx = dir * speed;
                        wk.vy = tn().gift_punch_up;
                        wk.air = wk.air.max(0.3);
                        wk.jump = 0.0;
                        wk.held_prev = None;
                        wk.act = Act::Fall;
                        wk.tumble = spin;
                        if let Some(r) = wk.social.as_mut() {
                            r.flying = true;
                            r.stuck = false;
                            r.moving = false;
                            r.happy = false;
                            r.spin = spin;
                            r.bounces = tn().gift_bounces as u8;
                        }
                        log::debug!("chat {}: pow!", c.id);
                    }
                }
            }
            3 => match g.kind {
                GiftKind::Carrot => {
                    for i in [giver, taker] {
                        let wk = self.walk_of(i);
                        if grounded(wk) && wk.rand() < dt * 3.0 {
                            let height = tn().jump_height * (0.4 + wk.rand() * 0.4);
                            wk.start_jump(height);
                        }
                    }
                    if c.wait <= 0.0 {
                        self.end_game(c);
                        return;
                    }
                }
                GiftKind::Balloon => {
                    let rise = tn().gift_balloon_height / tn().gift_balloon_time.max(0.1);
                    let spin = if self.walk.rand() < 0.5 { 10.0 } else { -10.0 };
                    let wk = self.walk_of(taker);
                    wk.air += rise * dt;
                    if c.wait <= 0.0 {
                        log::debug!("chat {}: the balloon pops", c.id);
                        wk.act = Act::Fall;
                        wk.vx = 0.0;
                        wk.vy = 0.0;
                        wk.jump = 0.0;
                        wk.held_prev = None;
                        if let Some(r) = wk.social.as_mut() {
                            r.perched = false;
                            r.flying = true;
                            r.spin = spin;
                            r.happy = false;
                        }
                        c.step = 4;
                    }
                }
                GiftKind::Prank => {
                    if c.wait <= 0.0 {
                        c.step = 4;
                    }
                }
            },
            // laughing until the other has come to a stop, then off after it
            4 => {
                if self.walk_ref(taker).social.is_some_and(|s| s.stuck) {
                    log::debug!("chat {}: landed {:.0} px away", c.id, (self.strip_px(taker) - self.strip_px(giver)).abs());
                    let size = self.width_of(taker);
                    let spot = self.strip_px(taker) - dir * size * 0.7;
                    self.go_facing(giver, spot, 0.65);
                    c.step = 5;
                }
            }
            // there: helps it up
            5 => {
                if self.arrived(giver) {
                    self.face(giver, taker);
                    let hop = tn().jump_height * 0.5;
                    let wk = self.walk_of(taker);
                    if let Some(r) = wk.social.as_mut() {
                        r.stuck = false;
                    }
                    wk.start_jump(hop);
                    let wk = self.walk_of(giver);
                    wk.gesture = true;
                    wk.start_bob(tn().jump_height * 0.2);
                    c.wait = 1.0;
                    c.step = 6;
                }
            }
            6 if c.wait <= 0.0 => {
                self.end_game(c);
                return;
            }
            _ => {}
        }
        c.gift = Some(g);
    }

    fn start_samba(&mut self, c: &mut Chat) {
        log::debug!("chat {}: samba", c.id);
        self.arrange(c);
        let mean = c.members.iter().map(|&i| self.goal_of(i)).sum::<f32>() / c.members.len() as f32;
        c.offset = if mean < self.strip_width() * 0.5 { 1.0 } else { -1.0 };
        c.turn = 1.0;
        c.wait = 0.0;
        c.step = 9;
        c.timer = 60.0;
        c.tag_free = tn().samba_join_gap;
        c.samba_new.clear();
        c.scores.clear();
        c.stage = Stage::Samba;
    }

    /// Someone passing sees the dancing and joins the end of the row, one
    /// at a time.
    fn try_samba_join(&mut self, c: &mut Chat, n: usize) {
        if c.tag_free > 0.0 || c.members.len() >= tn().max_group as usize || c.base.is_empty() {
            return;
        }
        let spots: Vec<f32> = c.members.iter().map(|&i| self.strip_px(i)).collect();
        let size = self.width_of(c.members[0]);
        let nearest = (0..n)
            .filter(|&i| {
                let wk = self.walk_ref(i);
                grounded(wk) && wk.climb.is_none() && wk.social.is_none() && wk.race.is_none() && !wk.circle && wk.energy >= tn().energy_join_need && chat_cool(wk) <= 0.0
            })
            .map(|i| {
                let x = self.strip_px(i);
                (i, x, spots.iter().map(|p| (p - x).abs()).fold(f32::MAX, f32::min))
            })
            .filter(|f| f.2 < size * tn().see_range)
            .min_by(|a, b| a.2.total_cmp(&b.2));
        let Some((i, x, _)) = nearest else { return };
        c.tag_free = tn().samba_join_gap * (0.7 + self.walk.rand() * 0.6);
        if self.walk.rand() >= tn().samba_join {
            return;
        }
        let gap = self.row_size(c) * 0.72;
        let lo = c.base.iter().map(|b| b.1).fold(f32::MAX, f32::min);
        let hi = c.base.iter().map(|b| b.1).fold(f32::MIN, f32::max);
        let slot = if x - c.pull < (lo + hi) * 0.5 { lo - gap } else { hi + gap };
        self.spend(i, tn().energy_join_cost);
        self.enlist(i, c.id);
        let wk = self.walk_of(i);
        if let Some(r) = wk.social.as_mut() {
            r.no_pat = true;
        }
        wk.start_bob(tn().jump_height * 0.2);
        c.members.push(i);
        c.base.push((i, slot));
        c.samba_new.push(i);
        log::debug!("chat {}: {i} joins the samba, now {}", c.id, c.members.len());
    }

    fn run_samba(&mut self, c: &mut Chat, dt: f32) {
        if c.step == 9 {
            if self.all_arrived(c) {
                c.base = c.members.iter().map(|&i| (i, self.strip_px(i))).collect();
                c.pull = 0.0;
                c.step = 0;
                c.timer = tn().samba_time;
            } else if c.timer <= 0.0 {
                self.end_game(c);
            }
            return;
        }
        if c.timer <= 0.0 {
            self.end_game(c);
            return;
        }
        c.tag_free -= dt;
        // put down somewhere else: runs back to its place in the row
        let width = self.strip_width();
        let far = (tn().samba_step * 1.6).max(30.0);
        let slot = |x: f32, marched: f32| (x + marched).clamp(0.0, width);
        let mut away = Vec::new();
        for &(i, x) in &c.base.clone() {
            if !c.members.contains(&i) || !grounded(self.walk_ref(i)) {
                continue;
            }
            let goal = slot(x, c.pull);
            if (self.strip_px(i) - goal).abs() > far {
                away.push(i);
                let speed = if c.samba_new.contains(&i) { 0.24 } else { 0.5 };
                if let Some(r) = self.walk_of(i).social.as_mut() {
                    r.goal_px = goal;
                    r.speed = speed;
                    r.moving = true;
                    r.leaping = false;
                    r.face_lock = false;
                    r.happy = false;
                }
            } else {
                c.samba_new.retain(|&m| m != i);
            }
        }
        c.wait -= dt;
        if c.wait > 0.0 {
            return;
        }
        c.wait = tn().samba_beat;
        c.turn = -c.turn;
        // at the end of the strip the whole row turns back
        let edge = 120.0f32.min(width * 0.25);
        let step = c.offset * tn().samba_step;
        let lo = c.base.iter().map(|b| b.1).fold(f32::MAX, f32::min);
        let hi = c.base.iter().map(|b| b.1).fold(f32::MIN, f32::max);
        if lo + c.pull + step < edge || hi + c.pull + step > width - edge {
            c.offset = -c.offset;
        }
        c.pull += c.offset * tn().samba_step;
        let w = self.view_w.max(1.0);
        for &(i, x) in &c.base.clone() {
            if !c.members.contains(&i) || away.contains(&i) {
                continue;
            }
            let goal = slot(x, c.pull);
            let left = c.turn < 0.0;
            let wk = self.walk_of(i);
            if !grounded(wk) {
                continue;
            }
            wk.start_bob(tn().jump_height * 0.12);
            if let Some(r) = wk.social.as_mut() {
                r.goal_px = goal;
                r.speed = tn().samba_step * 2.0 / w / 0.32;
                r.moving = true;
                r.leaping = true;
                r.face_lock = true;
                r.face_left = left;
                r.happy = true;
            }
        }
    }

    fn start_tag(&mut self, c: &mut Chat) {
        log::debug!("chat {}: tag", c.id);
        // one big window is the playground: everyone goes there first
        let (x, cw) = self.big_window();
        let (lo, hi) = (x + 40.0, x + cw - 40.0);
        log::debug!("chat {}: playing in the window at {x:.0}, {cw:.0} wide", c.id);
        c.spots = vec![lo, hi];
        c.it = c.members[(self.walk.rand() * c.members.len() as f32) as usize % c.members.len()];
        for k in 0..c.members.len() {
            if let Some(r) = self.walk_of(c.members[k]).social.as_mut() {
                r.no_pat = true;
            }
        }
        // "it" stays put and waits, the others run there
        for k in 0..c.members.len() {
            let i = c.members[k];
            if i == c.it {
                if let Some(r) = self.walk_of(i).social.as_mut() {
                    r.moving = false;
                    r.happy = false;
                }
            } else {
                let spot = lo + self.walk.rand() * (hi - lo).max(0.0);
                let speed = tn().tag_run * 1.2;
                self.go_facing(i, spot, speed);
            }
        }
        c.scores = c.members.iter().map(|&i| (i, 0.0)).collect();
        c.offset = 0.0;
        c.chase = c.it;
        c.recent.clear();
        c.step = 9;
        // getting there has its own limit, the game's clock starts after
        c.timer = 60.0;
        c.regroup = true;
        c.stage = Stage::Tag;
    }

    /// Where a race or a game is held: the widest window of the workspace,
    /// on screen or not; of those about as wide, the nearest. (left edge,
    /// width) on the strip, logical px.
    pub(super) fn biggest_column(&self) -> (f32, f32) {
        let w = self.view_w;
        let view_px = self.view[0] * w * 0.5;
        let middle = view_px + w * 0.5;
        let cols = crate::backend::governor::niri_columns();
        let widest = cols.iter().map(|c| c.1).fold(0.0, f32::max);
        cols.into_iter()
            .filter(|c| c.1 >= widest * 0.9)
            .min_by(|a, b| (a.0 + a.1 * 0.5 - middle).abs().total_cmp(&(b.0 + b.1 * 0.5 - middle).abs()))
            .unwrap_or((view_px, w))
    }

    /// `biggest_column`, at least half a screen long.
    fn big_window(&self) -> (f32, f32) {
        let (x, cw) = self.biggest_column();
        let len = cw.max(self.view_w * 0.5);
        (x + cw * 0.5 - len * 0.5, len)
    }

    /// Bursts and lulls: a speed multiplier that eases towards a new value
    /// every second or so.
    fn tag_pace(&mut self, i: usize, dt: f32) -> f32 {
        let wk = self.walk_of(i);
        let (wait, goal) = (0.6 + wk.rand() * 0.9, 1.0 + (wk.rand() * 2.0 - 1.0) * tn().tag_pace);
        let Some(r) = wk.social.as_mut() else { return 1.0 };
        r.pace_wait -= dt;
        if r.pace_wait <= 0.0 {
            r.pace_wait = wait;
            r.pace_goal = goal;
        }
        r.pace += (r.pace_goal - r.pace) * (dt * 3.0).min(1.0);
        r.pace
    }

    fn run_tag(&mut self, c: &mut Chat, dt: f32) {
        let (lo, hi) = (c.spots[0], c.spots[1]);
        // nobody is held to the window until they've reached it
        for k in 0..c.members.len() {
            let i = c.members[k];
            let x = self.strip_px(i);
            if let Some(r) = self.walk_of(i).social.as_mut() {
                r.inside |= x >= lo && x <= hi;
            }
        }
        if c.step == 9 {
            let mut waiting = false;
            for k in 0..c.members.len() {
                let i = c.members[k];
                if i == c.it {
                    continue;
                }
                let wk = self.walk_of(i);
                let (spot, wait) = (wk.rand(), 1.0 + wk.rand());
                let Some(r) = wk.social.as_mut() else { continue };
                if !r.inside {
                    waiting = true;
                    continue;
                }
                // there already: stroll about until the rest arrive
                r.hop_cool -= dt;
                if r.hop_cool <= 0.0 {
                    r.hop_cool = wait;
                    self.run_to(i, lo + spot * (hi - lo).max(0.0), tn().tag_run);
                }
            }
            if !waiting {
                c.step = 0;
                c.wait = tn().tag_wait;
                c.offset = 0.0;
                c.timer = tn().tag_time_min + self.walk.rand() * tn().tag_time_rand;
                log::debug!("chat {}: everyone's in the window, {} is it", c.id, c.it);
            } else if c.timer <= 0.0 {
                self.end_game(c);
            }
            return;
        }
        if c.timer <= 0.0 {
            self.end_game(c);
            return;
        }
        if !c.members.contains(&c.it) {
            c.it = c.members[0];
        }
        c.wait -= dt;
        let it_x = self.strip_px(c.it);
        let size = self.row_size(c);
        c.tag_free -= dt;
        if c.tag_free <= 0.0 {
            c.recent.clear();
        }
        // nearest that can be tagged (on the ground), nearest to go after
        // (also someone carried overhead: it runs along underneath)
        let mut nearest: Option<(usize, f32)> = None;
        let mut pursue: Option<(usize, f32)> = None;
        for k in 0..c.members.len() {
            let i = c.members[k];
            if i == c.it {
                continue;
            }
            if let Some(s) = c.scores.iter_mut().find(|s| s.0 == i) {
                s.1 += dt;
            }
            let x = self.strip_px(i);
            let d = x - it_x;
            if !c.recent.contains(&i) && pursue.is_none_or(|n| d.abs() < n.1) {
                pursue = Some((i, d.abs()));
            }
            let in_air = !grounded(self.walk_ref(i));
            if !in_air && !c.recent.contains(&i) && nearest.is_none_or(|n| d.abs() < n.1) {
                nearest = Some((i, d.abs()));
            }
            // runners stick to a decision for a moment, changing their mind every
            // frame just makes them shake on the spot
            let pace = self.tag_pace(i, dt);
            let wk = self.walk_of(i);
            let wait = 0.5 + wk.rand() * 0.5;
            let coin = wk.rand() < 0.5;
            let on_ground = grounded(wk);
            let Some(r) = wk.social.as_mut() else { continue };
            r.speed = tn().tag_run * pace;
            r.hop_cool -= dt;
            if r.hop_cool > 0.0 {
                continue;
            }
            r.hop_cool = wait;
            // away from "it", and round the other way when cornered
            // far from "it" there's nothing to run from: they mill about instead
            // of the whole game streaming off to one end
            let away = if d.abs() > 800.0 {
                if coin { 1.0 } else { -1.0 }
            } else if d >= 0.0 {
                1.0
            } else {
                -1.0
            };
            let goal = if (away > 0.0 && hi - x < 90.0) || (away < 0.0 && x - lo < 90.0) { it_x - away * 500.0 } else { x + away * 500.0 };
            self.run_to(i, goal.clamp(lo, hi), tn().tag_run * pace);
            // backed into the end with "it" right there: over its head
            let cornered = (away > 0.0 && hi - x < 90.0) || (away < 0.0 && x - lo < 90.0);
            if cornered && on_ground && d.abs() < tn().tag_leap_range && pace >= tn().tag_leap_pace {
                log::debug!("chat {}: {i} leaps over {}", c.id, c.it);
                let past = (it_x - away * (tn().tag_leap_range + 120.0)).clamp(lo, hi);
                self.run_to(i, past, tn().tag_run * 2.2);
                let height = tn().jump_height * 0.9;
                let wk = self.walk_of(i);
                wk.start_jump(height);
                if let Some(r) = wk.social.as_mut() {
                    r.leaping = true;
                    r.hop_cool = 1.2;
                }
            }
        }
        let Some((chase_n, chase_d)) = pursue else { return };
        if c.wait > 0.0 {
            // just got caught: stands still and counts before running
            if let Some(r) = self.walk_of(c.it).social.as_mut() {
                r.moving = false;
            }
            return;
        }
        if let Some((n, _)) = nearest.filter(|t| t.1 < size * 0.35) {
            log::debug!("chat {}: {n} is it", c.id);
            // no tagging straight back
            c.recent.clear();
            c.recent.push(c.it);
            c.tag_free = tn().tag_wait + 1.5;
            c.it = n;
            c.offset = 0.0;
            c.tag_turn = 0.0;
            c.wait = tn().tag_wait;
            let wk = self.walk_of(n);
            if grounded(wk) {
                wk.start_bob(tn().jump_height * 0.2);
            }
            return;
        }
        // always after the nearest; a clear margin before switching, or two
        // about as far away would have it turning back and forth
        let current = (c.chase != c.it && !c.recent.contains(&c.chase) && c.members.contains(&c.chase))
            .then(|| (self.strip_px(c.chase) - it_x).abs());
        if current.is_none_or(|now| chase_d + 40.0 < now) {
            c.chase = chase_n;
        }
        // always running at its own pace, and slow to turn round: someone who
        // jumps over it isn't followed until a moment later
        let side = (self.strip_px(c.chase) - it_x).signum();
        if c.offset == 0.0 {
            c.offset = side;
        }
        if side == c.offset {
            c.tag_turn = 0.0;
        } else {
            c.tag_turn += dt;
            if c.tag_turn >= tn().tag_delay {
                c.offset = side;
                c.tag_turn = 0.0;
            }
        }
        let pace = self.tag_pace(c.it, dt);
        let held = self.walk_ref(c.it).social.is_some_and(|s| s.inside);
        let ahead = it_x + c.offset * 600.0;
        self.run_to(c.it, if held { ahead.clamp(lo, hi) } else { ahead }, tn().tag_it * pace);
    }

    /// Someone around sees a game going on: they come and stand by to watch,
    /// and join when it's over.
    fn try_watch(&mut self, c: &mut Chat, n: usize) {
        for &w in &c.watchers.clone() {
            if self.arrived(w) {
                self.face(w, c.talker);
                if self.walk.rand() < 0.12 {
                    self.walk_of(w).start_bob(tn().jump_height * 0.1);
                }
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
                let wk = self.walk_ref(i);
                grounded(wk) && wk.climb.is_none() && wk.social.is_none() && wk.race.is_none() && !wk.circle && wk.energy >= tn().energy_watch_need && chat_cool(wk) <= tn().overlap_cool && self.on_screen(i)
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
        log::debug!("chat {}: {i} comes to watch", c.id);
        // beside the row, not in the way
        let lo = c.members.iter().map(|&m| self.goal_of(m)).fold(f32::MAX, f32::min);
        let hi = c.members.iter().map(|&m| self.goal_of(m)).fold(f32::MIN, f32::max);
        let step = size * 0.85 * c.watchers.len() as f32;
        let goal = if self.strip_px(i) < (lo + hi) * 0.5 { lo - step } else { hi + step };
        self.go(i, goal.clamp(0.0, self.strip_width()), 0.18);
    }

    /// The game's over: some of those watching come in, the rest move on.
    fn take_in_watchers(&mut self, c: &mut Chat) {
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

    /// A big crowd doesn't have to stay one: half of it goes off to do its
    /// own thing as a separate group.
    fn maybe_split(&mut self, c: &mut Chat) {
        if (c.members.len() as f32) < tn().split_min || self.walk.rand() >= tn().split_chance {
            return;
        }
        let row = self.sorted_row(c);
        let (left, right) = row.split_at(row.len() / 2);
        self.chat_id = self.chat_id.wrapping_add(1);
        let id = self.chat_id;
        let mut other = Chat::new(id, right.to_vec(), Stage::Approach, 0.0);
        other.games = c.games;
        other.played = c.played.clone();
        log::debug!("chat {}: splits, {:?} go off as chat {id}", c.id, other.members);
        for &i in &other.members {
            if let Some(r) = self.walk_of(i).social.as_mut() {
                r.chat = id;
            }
        }
        c.members = left.to_vec();
        c.talker = c.members[0];
        c.regroup = true;
        self.arrange(&other);
        self.arrange(c);
        // a bit of room between the two
        let size = self.row_size(c);
        for (members, away) in [(c.members.clone(), -1.0), (other.members.clone(), 1.0)] {
            for i in members {
                let x = self.goal_of(i);
                self.steer(i, x + away * size * 1.5, 0.16);
            }
        }
        self.new_chats.push(other);
    }

    fn start_contest(&mut self, c: &mut Chat) {
        log::debug!("chat {}: jumping game", c.id);
        let mut queue = c.members.clone();
        for k in (1..queue.len()).rev() {
            let j = (self.walk.rand() * (k + 1) as f32) as usize % (k + 1);
            queue.swap(k, j);
        }
        c.queue = queue;
        c.wait = 0.0;
        c.scores.clear();
        c.step = 0;
        c.stage = Stage::Contest;
    }

    /// Long jump: one referee. An odd group plays in pairs, one throwing the
    /// other; an even one each on their own with a run-up.
    fn start_long(&mut self, c: &mut Chat) {
        let mut all = c.members.clone();
        for k in (1..all.len()).rev() {
            let j = (self.walk.rand() * (k + 1) as f32) as usize % (k + 1);
            all.swap(k, j);
        }
        let referee = all.remove(0);
        let pairs = c.members.len() % 2 == 1;
        let turns: Vec<Turn> = if pairs {
            all.chunks_exact(2).map(|p| Turn { flyer: p[0], thrower: Some(p[1]) }).collect()
        } else {
            all.iter().map(|&i| Turn { flyer: i, thrower: None }).collect()
        };
        log::debug!("chat {}: long jump, referee {referee}, {} turns ({})", c.id, turns.len(), if pairs { "pairs" } else { "singles" });
        let width = self.strip_width();
        let mean = c.members.iter().map(|&i| self.strip_px(i)).sum::<f32>() / c.members.len() as f32;
        // plenty of ground ahead
        let dir = if mean < width * 0.5 { 1.0 } else { -1.0 };
        let line = (mean - dir * 150.0).clamp(tn().edge_px.min(width * 0.5), (width - tn().edge_px).max(width * 0.5));
        let size = self.width_of(c.members[0]);
        let mut back = 1.0;
        for t in &turns {
            self.go(t.flyer, line - dir * size * 0.85 * back, 0.16);
            back += 1.0;
            if let Some(th) = t.thrower {
                self.go(th, line - dir * size * 0.85 * back, 0.16);
                back += 1.0;
            }
        }
        self.go(referee, line + dir * size, 0.16);
        c.long = Some(Long { referee, line, dir, ready: false, turns, next: 0, gap: 0.0, running: Vec::new(), landed: Vec::new(), judging: None, thrown: 0.0 });
        c.scores.clear();
        c.regroup = true;
        c.timer = tn().long_time;
        c.stage = Stage::Long;
    }

    /// Off they go, very far, tumbling.
    fn launch(&mut self, i: usize, dir: f32) {
        let wk = self.walk_of(i);
        let distance = tn().long_dist_min + wk.rand() * tn().long_dist_rand;
        let time = tn().long_air_min + wk.rand() * tn().long_air_rand;
        self.fling(i, dir, distance, time);
    }

    /// Through the air, tumbling, to end up planted in the ground.
    fn fling(&mut self, i: usize, dir: f32, distance: f32, time: f32) {
        let w = self.view_w.max(1.0);
        let from = self.strip_px(i);
        let wk = self.walk_of(i);
        let spin = dir * (5.0 + wk.rand() * 9.0);
        wk.vx = dir * (distance * 2.0 / w) / time;
        wk.vy = 20.0 * time;
        wk.air = 0.1;
        wk.jump = 0.0;
        wk.held_prev = None;
        wk.act = Act::Fall;
        if let Some(r) = wk.social.as_mut() {
            r.flying = true;
            r.jumped = true;
            r.launch_px = from;
            r.happy = true;
            r.spin = spin;
            r.moving = false;
            r.bump_cool = 0.2;
        }
    }

    /// Those in the air knock into each other and into whoever is already
    /// stuck in the ground.
    fn long_collisions(&mut self, c: &Chat, dt: f32) {
        let mut flyers = Vec::new();
        let mut planted = Vec::new();
        for &i in &c.members {
            let x = self.strip_px(i);
            let width = self.width_of(i);
            let wk = self.walk_of(i);
            let air = wk.air;
            let Some(r) = wk.social.as_mut() else { continue };
            r.bump_cool = (r.bump_cool - dt).max(0.0);
            if r.flying {
                flyers.push((i, x, air, width, r.bump_cool));
            } else if r.stuck {
                planted.push((x, width));
            }
        }
        for (k, a) in flyers.iter().enumerate() {
            for b in &flyers[k + 1..] {
                if a.4 > 0.0 || b.4 > 0.0 || (a.1 - b.1).abs() > (a.3 + b.3) * 0.3 || (a.2 - b.2).abs() > 1.6 {
                    continue;
                }
                // bang: they swap speeds and both get knocked up
                let (va, vb) = (self.walk_ref(a.0).vx, self.walk_ref(b.0).vx);
                for (i, v) in [(a.0, vb), (b.0, va)] {
                    let wk = self.walk_of(i);
                    wk.vx = v * 0.85;
                    wk.vy = wk.vy.max(6.0) + 3.0;
                    if let Some(r) = wk.social.as_mut() {
                        r.spin = -r.spin * 1.2;
                        r.bump_cool = 0.5;
                    }
                }
            }
        }
        for f in &flyers {
            if f.4 > 0.0 || f.2 > 1.6 {
                continue;
            }
            if planted.iter().any(|p| (f.1 - p.0).abs() < (f.3 + p.1) * 0.3) {
                let wk = self.walk_of(f.0);
                wk.vx = -wk.vx * 0.6;
                wk.vy = wk.vy.max(10.0);
                if let Some(r) = wk.social.as_mut() {
                    r.spin = -r.spin;
                    r.bump_cool = 0.4;
                }
            }
        }
    }

    /// One frame of the long jump; false when it's over.
    fn run_long(&mut self, c: &mut Chat, dt: f32) {
        let Some(mut l) = c.long.take() else { return };
        if c.timer <= 0.0 || !c.members.contains(&l.referee) {
            self.end_game(c);
            return;
        }
        // thrown by hand before its turn: that was its jump
        for k in 0..l.turns.len() {
            let t = l.turns[k];
            if !c.members.contains(&t.flyer) {
                continue;
            }
            let line = l.line;
            let wk = self.walk_of(t.flyer);
            let spin = if wk.vx < 0.0 { -9.0 } else { 9.0 };
            let thrown = wk.act == Act::Fall;
            let Some(r) = wk.social.as_mut().filter(|r| thrown && !r.jumped && !r.flying && !r.stuck) else { continue };
            r.flying = true;
            r.jumped = true;
            r.happy = true;
            r.moving = false;
            r.spin = spin;
            r.launch_px = line;
            log::debug!("chat {}: {} thrown by hand", c.id, t.flyer);
            if let Some(th) = t.thrower.filter(|th| c.members.contains(th) && l.running.iter().any(|x| x.flyer == t.flyer)) {
                let size = self.width_of(th);
                self.go(th, l.line + l.dir * size * (1.9 + 0.85 * l.thrown), 0.3);
                l.thrown += 1.0;
            }
            l.running.retain(|x| x.flyer != t.flyer);
        }
        if !l.ready {
            l.ready = self.all_arrived(c);
            c.long = Some(l);
            return;
        }
        // next one sets off, a little after the last so they meet in the air
        l.gap -= dt;
        // a pair has the line to itself until it has thrown
        let pairs = l.turns.first().is_some_and(|t| t.thrower.is_some());
        while l.gap <= 0.0 && l.next < l.turns.len() && (!pairs || l.running.is_empty()) {
            let t = l.turns[l.next];
            l.next += 1;
            l.gap = tn().long_gap_min + self.walk.rand() * tn().long_gap_rand;
            if !c.members.contains(&t.flyer) || self.walk_ref(t.flyer).social.is_some_and(|s| s.jumped) {
                continue;
            }
            let thrower = t.thrower.filter(|th| c.members.contains(th));
            // everyone goes from the same line, or it isn't fair: alone at a
            // sprint (front of the queue first, so nobody's in the way), a pair
            // walking up, the thrower right behind
            match thrower {
                None => self.go(t.flyer, l.line, 0.7),
                Some(th) => {
                    let size = self.width_of(th);
                    self.go(t.flyer, l.line, 0.3);
                    self.go(th, l.line - l.dir * size * 0.85, 0.3);
                }
            }
            l.running.push(Turn { flyer: t.flyer, thrower });
        }
        for t in l.running.clone() {
            let gone = !c.members.contains(&t.flyer);
            if gone || (self.arrived(t.flyer) && t.thrower.is_none_or(|th| !c.members.contains(&th) || self.arrived(th))) {
                if !gone {
                    if let Some(th) = t.thrower.filter(|th| c.members.contains(th)) {
                        // thrown: clear the line for the next pair
                        let size = self.width_of(th);
                        self.go(th, l.line + l.dir * size * (1.9 + 0.85 * l.thrown), 0.3);
                        l.thrown += 1.0;
                        self.walk_of(th).start_bob(tn().jump_height * 0.2);
                    }
                    self.launch(t.flyer, l.dir);
                }
                l.running.retain(|r| r.flyer != t.flyer);
            }
        }
        self.long_collisions(c, dt);
        for &i in &c.members {
            if let Some(r) = self.walk_of(i).social.as_mut() {
                if r.stuck && !r.queued {
                    r.queued = true;
                    l.landed.push(i);
                }
            }
        }
        // the referee goes over to each one that came down
        l.landed.retain(|i| c.members.contains(i));
        match l.judging {
            Some((i, _)) if !c.members.contains(&i) => l.judging = None,
            Some((i, wait)) => {
                if self.arrived(l.referee) {
                    self.face(l.referee, i);
                    let wait = wait - dt;
                    if wait <= 0.0 {
                        let from = self.walk_ref(i).social.map_or(l.line, |s| s.launch_px);
                        let distance = ((self.strip_px(i) - from) * l.dir).max(1.0);
                        c.scores.push((i, distance));
                        log::debug!("chat {}: {i} jumped {distance:.0}", c.id);
                        if let Some(r) = self.walk_of(i).social.as_mut() {
                            r.stuck = false;
                        }
                        self.walk_of(l.referee).start_bob(tn().jump_height * 0.15);
                        l.judging = None;
                    } else {
                        l.judging = Some((i, wait));
                    }
                }
            }
            None => {
                if !l.landed.is_empty() {
                    let i = l.landed.remove(0);
                    let x = self.strip_px(i);
                    let size = self.width_of(i);
                    self.go(l.referee, x - l.dir * size * 0.7, 0.28);
                    l.judging = Some((i, tn().long_judge));
                }
            }
        }
        let airborne = c.members.iter().any(|&i| self.walk_ref(i).social.is_some_and(|s| s.flying || s.stuck));
        if l.next >= l.turns.len() && l.running.is_empty() && l.landed.is_empty() && l.judging.is_none() && !airborne {
            self.end_game(c);
            return;
        }
        c.long = Some(l);
    }

    fn start_cursor_game(&mut self, c: &mut Chat) {
        log::debug!("chat {}: cursor game", c.id);
        for &i in &c.members.clone() {
            if let Some(r) = self.walk_of(i).social.as_mut() {
                r.chasing = true;
                r.happy = false;
            }
        }
        c.scores = c.members.iter().map(|&i| (i, 0.0)).collect();
        c.timer = tn().cursor_time_min + self.walk.rand() * tn().cursor_time_rand;
        c.stage = Stage::Cursor;
    }

    /// The game's over: the best one hops about, if anyone scored at all.
    fn end_game(&mut self, c: &mut Chat) {
        for &i in &c.members.clone() {
            // stop here: the game's last goals mean nothing any more, and who
            // looks at whom is worked out from where everyone stands
            let here = self.strip_px(i);
            if let Some(r) = self.walk_of(i).social.as_mut() {
                r.goal_px = here;
                r.moving = false;
                r.chasing = false;
                r.happy = false;
                r.no_touch = false;
                r.no_pat = false;
                r.face_lock = false;
                r.perched = false;
                r.flying = false;
                r.stuck = false;
                r.lean = 0.0;
            }
        }
        // tag has no winner: everybody's just happy it's over
        if c.stage == Stage::Tag && c.step != 9 {
            log::debug!("chat {}: tag over, everyone hops", c.id);
            for &i in &c.members.clone() {
                if let Some(r) = self.walk_of(i).social.as_mut() {
                    r.happy = true;
                }
            }
            c.winner = None;
            c.all_cheer = true;
            c.timer = tn().cheer_time;
            c.stage = Stage::Cheer;
            return;
        }
        let best = c.scores.iter().filter(|s| s.1 > 0.0 && c.members.contains(&s.0)).max_by(|a, b| a.1.total_cmp(&b.1));
        c.winner = best.map(|s| s.0);
        log::debug!("chat {}: game over, {:?} won of {:?}", c.id, c.winner, c.scores);
        if let Some(w) = c.winner {
            c.talker = w;
            self.look_at_talker(c);
            if let Some(r) = self.walk_of(w).social.as_mut() {
                r.happy = true;
            }
            c.timer = tn().cheer_time;
            c.bob = 0.0;
            c.stage = Stage::Cheer;
        } else {
            self.talk_it_over(c);
        }
    }

    /// A game's not over till it's been talked about.
    fn talk_it_over(&mut self, c: &mut Chat) {
        // the cheering is over
        for &i in &c.members.clone() {
            if let Some(r) = self.walk_of(i).social.as_mut() {
                r.happy = false;
            }
        }
        if c.stage == Stage::Samba {
            // danced together: everyone stays, watchers too, and talks about it
            c.members.append(&mut c.watchers);
            c.regroup = true;
        } else {
            self.take_in_watchers(c);
            self.maybe_split(c);
        }
        if std::mem::take(&mut c.regroup) {
            self.arrange(c);
            c.stage = Stage::Approach;
            return;
        }
        let length = tn().after_game_min + self.walk.rand() * tn().after_game_rand;
        self.start_talk(c, length);
    }

    /// And their own ways, away from the middle of the group.
    fn part(&mut self, c: &mut Chat) {
        log::debug!("chat {}: part", c.id);
        let width = self.strip_width();
        let mean = c.members.iter().map(|&i| self.strip_px(i)).sum::<f32>() / c.members.len() as f32;
        for &i in &c.members.clone() {
            let x = self.strip_px(i);
            let dist = 300.0 + self.walk.rand() * 500.0;
            let away = if (x - mean).abs() < 1.0 {
                if self.walk.rand() < 0.5 { -1.0 } else { 1.0 }
            } else {
                (x - mean).signum()
            };
            self.go_facing(i, (x + away * dist).clamp(tn().edge_px.min(width * 0.5), (width - tn().edge_px).max(width * 0.5)), 0.12);
        }
        c.stage = Stage::Part;
    }

    /// Run one chat for a frame; false when it's over.
    fn advance(&mut self, c: &mut Chat, dt: f32, mouse: [f32; 2]) -> bool {
        c.timer -= dt;
        match c.stage {
            Stage::Notice => {
                // the other one looks back a moment later
                if c.timer < 0.6 {
                    self.face(c.members[1], c.members[0]);
                }
                if c.timer <= 0.0 {
                    // "oh, hi": a little hop each, then side by side
                    self.arrange(c);
                    c.stage = Stage::Approach;
                    for &i in &c.members {
                        self.walk_of(i).start_bob(tn().jump_height * 0.2);
                    }
                }
            }
            Stage::Approach => {
                if self.all_arrived(c) {
                    if let Some(game) = c.forced.take() {
                        c.played.push(game);
                        c.games += 1;
                        self.start_game(c, game);
                        return true;
                    }
                    // some are just a quick hello
                    let length = if c.members.len() == 2 && self.walk.rand() < tn().short_talk_chance {
                        tn().short_talk_min + self.walk.rand() * tn().short_talk_rand
                    } else {
                        self.talk_length()
                    };
                    self.start_talk(c, length);
                }
            }
            Stage::Talk => {
                c.quiet += dt;
                c.mood += dt;
                if c.members.len() >= tn().mood_min as usize && c.mood >= tn().mood_after && self.race.is_none() && self.social_age >= tn().warm_small {
                    c.mood = -60.0;
                    if self.mood_split(c) {
                        return true;
                    }
                }
                let may_play = c.members.len() >= tn().game_min as usize
                    && c.quiet >= tn().game_after
                    && (c.games == 0 || (c.members.len() >= tn().crowd_games as usize && (c.games as f32) < tn().max_games));
                if may_play {
                    // not every chat turns into a game; ask again a good while later
                    if self.walk.rand() >= tn().game_chance {
                        c.quiet = -2.0 * tn().game_after;
                    } else if self.next_game(c) {
                        c.games += 1;
                        c.quiet = 0.0;
                        return true;
                    }
                }
                if c.timer <= 0.0 {
                    if (c.strolls as f32) < tn().max_strolls && self.walk.rand() < tn().stroll_chance {
                        self.stroll(c);
                    } else {
                        self.part(c);
                    }
                    return true;
                }
                self.chatter(c, dt);
            }
            Stage::Stroll => {
                if self.all_arrived(c) {
                    let length = if std::mem::take(&mut c.joined_walking) {
                        self.talk_length()
                    } else {
                        3.0 + self.walk.rand() * 2.0
                    };
                    self.start_talk(c, length);
                }
            }
            Stage::Contest => {
                c.wait -= dt;
                // the one whose turn it was got picked up or wandered off
                while c.queue.first().is_some_and(|i| !c.members.contains(i)) {
                    c.queue.remove(0);
                    c.step = 0;
                }
                let Some(&i) = c.queue.first() else {
                    self.end_game(c);
                    return true;
                };
                match c.step {
                    // its turn, right where it stands: everyone looks
                    0 if c.wait <= 0.0 => {
                        c.talker = i;
                        self.look_at_talker(c);
                        if let Some(r) = self.walk_of(i).social.as_mut() {
                            r.happy = true;
                        }
                        c.wait = 0.35;
                        c.step = 1;
                    }
                    // up, as high as it can this time
                    1 if c.wait <= 0.0 && grounded(self.walk_ref(i)) => {
                        let height = tn().contest_min + self.walk.rand() * tn().contest_rand;
                        self.walk_of(i).start_jump(tn().jump_height * height);
                        c.scores.push((i, height));
                        c.step = 2;
                    }
                    // landed: the next one
                    2 if grounded(self.walk_ref(i)) => {
                        if let Some(r) = self.walk_of(i).social.as_mut() {
                            r.happy = false;
                        }
                        c.queue.remove(0);
                        c.wait = 0.4;
                        c.step = 0;
                    }
                    _ => {}
                }
            }
            Stage::Cursor => {
                if c.timer <= 0.0 {
                    self.end_game(c);
                    return true;
                }
                // long enough on the cursor: that's the game
                if c.scores.iter().any(|s| s.1 >= tn().cursor_win) {
                    self.end_game(c);
                    return true;
                }
                let cursor = mouse[0] + self.view[0] * self.view_w * 0.5;
                let count = c.members.len() as f32;
                for k in 0..c.members.len() {
                    let i = c.members[k];
                    let x = self.strip_px(i);
                    let wk = self.walk_of(i);
                    // seconds hanging off the cursor are the score
                    if wk.act == Act::Held && wk.hang > 0.0 {
                        if let Some(s) = c.scores.iter_mut().find(|s| s.0 == i) {
                            s.1 += dt;
                        }
                    }
                    let (reach, on_ground) = (wk.reach, grounded(wk));
                    let roll = wk.rand();
                    let Some(r) = wk.social.as_mut() else { continue };
                    r.hop_cool = (r.hop_cool - dt).max(0.0);
                    r.goal_px = cursor + (k as f32 - (count - 1.0) * 0.5) * 30.0;
                    r.speed = tn().cursor_speed;
                    r.moving = true;
                    r.face_left = cursor < x;
                    let jump = on_ground && r.hop_cool <= 0.0 && (x - cursor).abs() < tn().cursor_reach && reach > 0.0;
                    if jump {
                        r.hop_cool = 0.6 + roll * 0.8;
                        wk.start_jump(reach.max(tn().jump_height));
                        wk.catch_jump = true;
                    }
                }
            }
            Stage::Long => self.run_long(c, dt),
            Stage::Domino => self.run_domino(c, dt),
            Stage::Tug => self.run_tug(c, dt, mouse),
            Stage::Tag => self.run_tag(c, dt),
            Stage::Carrot => self.run_carrot(c, dt),
            Stage::Gift => self.run_gift(c, dt),
            Stage::Samba => self.run_samba(c, dt),
            Stage::Volley => self.run_volley(c, dt),
            Stage::Cheer => {
                if c.timer <= 0.0 {
                    c.all_cheer = false;
                    self.talk_it_over(c);
                    return true;
                }
                if c.all_cheer {
                    for k in 0..c.members.len() {
                        let wk = self.walk_of(c.members[k]);
                        // not all in step
                        if grounded(wk) && wk.rand() < dt * 5.0 {
                            let height = tn().jump_height * (0.4 + wk.rand() * 0.5);
                            wk.start_jump(height);
                        }
                    }
                }
                if let Some(w) = c.winner.filter(|w| c.members.contains(w)) {
                    let wk = self.walk_of(w);
                    if grounded(wk) {
                        wk.start_jump(tn().jump_height * 0.7);
                    }
                    // the others cheer along, one little bob at a time
                    c.bob -= dt;
                    if c.bob <= 0.0 {
                        c.bob = 0.3 + self.walk.rand() * 0.4;
                        let others: Vec<usize> = c.members.iter().copied().filter(|&i| i != w).collect();
                        if !others.is_empty() {
                            let i = others[(self.walk.rand() * others.len() as f32) as usize % others.len()];
                            let wk = self.walk_of(i);
                            if let Some(r) = wk.social.as_mut() {
                                r.happy = true;
                            }
                            if grounded(wk) {
                                wk.start_bob(tn().jump_height * 0.1);
                            }
                        }
                    }
                }
            }
            Stage::Part => return !self.all_arrived(c),
            Stage::RaceCall => {
                if c.timer <= 0.0 {
                    // the caller first, then the others
                    let mut crew = vec![c.it];
                    crew.extend(c.members.iter().copied().filter(|&m| m != c.it));
                    log::debug!("chat {}: off to race, {crew:?}", c.id);
                    self.race_crew = crew;
                    super::RACE_NOW.store(true, std::sync::atomic::Ordering::Relaxed);
                    c.timer = 30.0;
                    c.stage = Stage::RaceWait;
                    return true;
                }
                self.chatter(c, dt);
            }
            Stage::RaceWait => {
                if c.timer <= 0.0 {
                    self.part(c);
                    return true;
                }
                self.chatter(c, dt);
            }
        }
        true
    }
}
