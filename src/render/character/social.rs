//! Characters taking notice of each other: two standing close look over,
//! step side by side, chat for a while, maybe stroll together, then go their
//! own ways. Others passing by can join in, the more likely the bigger the
//! crowd, and a chat that gathers most of the team turns into a race. Only standing ones start one: walking through each
//! other is fine.

use crate::render::tuning::get as tn;
use super::CharacterOverlay;
use super::walk::{Act, WalkState};

mod desktop;
mod free;
mod life;
mod play;
mod hide;
mod snow;
mod watch;

pub(super) use free::Want;

/// A game asked for from the settings window (`anima game <name>`).
pub static GAME_NOW: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// One character's part in a chat. Goals are strip positions in logical px,
/// like a race's.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Social {
    pub(super) chat: u32,
    pub(super) goal_px: f32,
    pub(super) goal: f32,
    pub(super) speed: f32,
    pub(super) moving: bool,
    pub(super) face_left: bool,
    // in the happy pose (talking, laughing)
    pub(super) happy: bool,
    // playing the cursor game, and seconds to its next try
    pub(super) chasing: bool,
    hop_cool: f32,
    // long jump: in the air (spinning this fast), or planted in the ground
    pub(super) flying: bool,
    spin: f32,
    pub(super) stuck: bool,
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
    pub(super) leaping: bool,
    // tag: has reached the playing window, and only now keeps inside it
    inside: bool,
    // on the rope: nothing but the rope itself can be touched
    pub(super) no_touch: bool,
    // in a game that petting would break
    pub(super) no_pat: bool,
    // long jump: has had its go (or was thrown by hand instead)
    jumped: bool,
    // samba: looks where it's told while stepping somewhere else
    pub(super) face_lock: bool,
    // volleyball: standing on the other half of the net
    pub(super) perched: bool,
    // picked up by hand mid-game, and where it was (or was heading) then
    grabbed: bool,
    grab_px: f32,
    // knocked flying: times it comes back off the ground
    pub(super) bounces: u8,
    // display-only: currently in pure chat (`Stage::Talk`) or a crowded group (3+)
    pub(super) in_talk: bool,
    pub(super) crowd: bool,
    // slide tube visibility override (None = 1.0)
    pub(super) alpha: Option<f32>,
    // crouch scale from the feet up inside slide tubes (None = 1.0)
    pub(super) tuck: Option<f32>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    Notice,
    Approach,
    Talk,
    Stroll,
    // after a race: the first three dance, the rest watch
    Concert,
    // games: each jumps in turn, or all go after the cursor
    Contest,
    Cursor,
    Long,
    Domino,
    Tug,
    Wrestle,
    Tag,
    Carrot,
    Gift,
    Samba,
    Volley,
    // rods out along the floor, reeling in whatever bites
    Fishing,
    // snowballs, and now and then a big one rolled into a snowman
    Snow,
    // one counts, the rest hide behind windows
    Hide,
    // practising alone, or shown how by someone better at it
    Practice,
    Cheer,
    Part,
    // someone's said "let's race": the group talks it over, then sets off
    RaceCall,
    // setting off a few at a time; the rest stay and chat till their turn
    RaceWait,
    // on something built in the park
    Play,
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
    // the ball in someone's hand: for how long
    held: f32,
    // who is leaping up to knock it out of the hand
    knocker: Option<usize>,
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
    // a bystander who'll snatch it (seconds until it tries; <0: nobody),
    // who's on the way, who's eating it and for how much longer
    thief_t: f32,
    thief: Option<usize>,
    eating: Option<usize>,
    eat: f32,
    // until it may slip it to a friend again
    pass_cool: f32,
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

/// One angler: where it sits, what it's doing, what it caught.
#[derive(Clone, Copy, Debug)]
struct Angler {
    i: usize,
    // 0 walking over, 1 casting, 2 waiting, 3 a bite, 4 reeling in, 5 showing it off
    phase: u8,
    t: f32,
    wait: f32,
    item: i32,
    // seconds of interest left; f32::MAX until the rod first goes out
    patience: f32,
    // its place on the floor, and whether it was blown off it
    at: f32,
    blown: bool,
    // picked up: decides on landing whether it comes back
    thrown: bool,
    // seconds left of a happy moment while it waits for a bite
    glee: f32,
    // times it set off back to its place and got nowhere
    tries: u8,
}

/// Fishing: everyone along the floor of the widest window.
struct Fishing {
    anglers: Vec<Angler>,
    // something coming up out of the ground: strip x, seconds out, seconds it stays
    beast: Option<(f32, f32, f32)>,
    // no second one straight after
    beast_cool: f32,
    // those watching: who, seconds till it does something else, who it's chatting with
    idle: Vec<(usize, f32, Option<usize>)>,
}

/// What can come up on the line: prop kind, points, whether it's a good one.
const CATCHES: [(i32, f32, bool); 6] = [(0, 3.0, true), (1, 4.0, true), (15, 5.0, true), (16, 2.0, true), (17, 0.0, false), (18, 0.0, false)];


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

/// Tag-team 1v1 wrestling: two teams wait on either side while front fighters push head-to-head in the middle.
struct Wrestle {
    mid: f32,
    lo: f32,
    hi: f32,
    gap: f32,
    ring: f32,
    unit: f32,
    teams: [Vec<usize>; 2],
    out: [Vec<usize>; 2],
    pair: Option<(usize, usize)>,
    // 0: send front pair to ring, 1: pair walks to center, 2: clash, 3: loser thrown & tumble
    phase: u8,
    t: f32,
    push: f32,
    push_target: f32,
    offset: f32,
    cheer_t: f32,
    // a fan's happy moment and how long it has left (s)
    glee: Vec<(usize, f32)>,
    // the last bout's winner and how long it's still celebrating (s)
    winner: Option<usize>,
    // and who it threw out: in no mood for cheering for a while
    loser: Option<usize>,
    party: f32,
    // phase 4: which sides are still talking over who goes next
    decide: [bool; 2],
    arena_y: f32,
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
    Wrestle,
    Tag,
    Samba,
    Volley,
    Fishing,
    Snow,
    Hide,
}

impl Chat {
    fn new(id: u32, members: Vec<usize>, stage: Stage, timer: f32) -> Self {
        let talker = members[0];
        Chat { by_hand: false, world: None, watch_left: Vec::new(), stage_t: 0.0, last_stage: stage, practice: None, encore_due: false, leavers: Vec::new(), offer: None, hold: 0.0, fishing: None, snow: None, hide: None, part_queue: Vec::new(), bribe: None, cheated: None, cheat_t: -1.0, cheater: None, cheat_side: 0.0, cheat_on: false, id, members, stage, timer, talker, turn: 0.0, bob: 0.0, strolls: 0, joined_walking: false, quiet: 0.0, games: 0, played: Vec::new(), watchers: Vec::new(), spots: Vec::new(), base: Vec::new(), offset: 0.0, pull: 0.0, pull_target: 0.0, it: 0, chase: 0, teams: (Vec::new(), Vec::new()), queue: Vec::new(), step: 0, wait: 0.0, scores: Vec::new(), winner: None, rewarded: false, long: None, regroup: false, forced: None, recent: Vec::new(), tag_free: 0.0, tag_turn: 0.0, tug_grab: None, tug_held: 0.0, tug_clicks: 0, cuts: 0, cut_timer: 0.0, all_cheer: false, volley: None, wrestle: None, mood: 0.0, carrot: None, samba_new: Vec::new(), gift: None, play: None }
    }
}

/// Slipping someone a carrot before the game, to lose on purpose (tug) or
/// add a bit to their jump (long jump). phase: 0 set off, 1 walking over,
/// 2 handing it over, 4 walking back, 3 done.
#[derive(Clone, Copy, Debug)]
struct Bribe {
    giver: usize,
    taker: usize,
    phase: u8,
    t: f32,
    home: f32,
}

/// Long jump: pushed (or shoving itself) further along after landing.
#[derive(Clone, Copy, Debug)]
struct Nudge {
    who: usize,
    by: Option<usize>,
    left: f32,
    orig: f32,
    witness: Option<usize>,
    started: bool,
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
    nudges: Vec<Nudge>,
    // caught being pushed on: where it really landed
    fixed: Vec<(usize, f32)>,
}

pub(super) struct Chat {
    id: u32,
    members: Vec<usize>,
    stage: Stage,
    // seconds in this stage, and which stage that was (see life.rs)
    stage_t: f32,
    last_stage: Stage,
    // in the park (true) or out on the desktop
    world: Option<bool>,
    // started from the settings window
    by_hand: bool,
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
    // and how much longer each of them will stay
    watch_left: Vec<(usize, f32)>,
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
    rewarded: bool,
    long: Option<Long>,
    // scattered by a game: get back together before talking
    regroup: bool,
    // after a game: who's had enough and wanders off (seconds until each goes)
    leavers: Vec<(usize, f32)>,
    practice: Option<Practice>,
    // just finished a game: the next talk decides who plays on
    encore_due: bool,
    // and who's holding up the next game for those staying: proposer, game, seconds left
    offer: Option<(usize, Game, f32)>,
    // someone was picked up: turn-taking games wait this long at most for them
    hold: f32,
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
    // tug of war: seconds until someone watching decides to cheat (<0: nobody
    // will), who, for which side (1 = right), and whether it has hold of the rope
    cheat_t: f32,
    cheater: Option<usize>,
    cheat_side: f32,
    cheat_on: bool,
    // who cheated at tug, and a bribe before a game
    cheated: Option<usize>,
    bribe: Option<Bribe>,
    // parting: who, seconds until it sets off, where to, how fast
    part_queue: Vec<(usize, f32, f32, f32)>,
    cuts: u32,
    cut_timer: f32,
    // a game that ends with everyone hopping, no winner
    all_cheer: bool,
    volley: Option<Volley>,
    wrestle: Option<Wrestle>,
    // seconds talking, towards splitting into talkers and players
    mood: f32,
    carrot: Option<Carrot>,
    fishing: Option<Fishing>,
    snow: Option<snow::Snow>,
    hide: Option<hide::Hide>,
    // samba: still walking over to join the row
    samba_new: Vec<usize>,
    gift: Option<Gift>,
    // playing on something built in the park
    play: Option<play::Play>,
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

thread_local! {
    /// While a game is being set up in the park: the park's stretch of strip.
    static GAME_AREA: std::cell::Cell<Option<(f32, f32)>> = const { std::cell::Cell::new(None) };
}

/// How many play a game started by hand, if the team has that many.
const FORCED_PLAYERS: usize = 6;

/// The place nearest `x` in the window with nobody else's rod on it.
fn free_fishing_spot(f: &Fishing, me: usize, x: f32, lo: f32, hi: f32, size: f32) -> f32 {
    let (min, max) = (lo + size, hi - size);
    let x = if min > max { (lo + hi) * 0.5 } else { x.clamp(min, max) };
    let gap = size * 1.1;
    let free = |p: f32| f.anglers.iter().all(|o| o.i == me || (o.at - p).abs() >= gap);
    let tries = ((hi - lo) / gap).ceil() as i32;
    (0..=tries)
        .flat_map(|k| [k as f32, -(k as f32)])
        .map(|k| x + k * gap)
        .find(|&p| p >= min && p <= max && free(p))
        .unwrap_or(x)
}

/// Still in the hand: no giving up on it yet.
fn wk_held(this: &CharacterOverlay, c: &Chat) -> bool {
    c.members.iter().any(|&i| this.walk_ref(i).act == Act::Held)
}

/// Things worth keeping, by prop kind.
fn keepsake_name(kind: i32) -> String {
    match kind {
        15 => "a star".into(),
        16 => "a rubber duck".into(),
        0 => "a carrot".into(),
        // sea creatures are rare enough to keep
        k if k >= CREATURE => super::rope::sea_name(k).map(|n| format!("a {n}")).unwrap_or_default(),
        _ => String::new(),
    }
}

/// Games that can be practised alone, and how.
const DRILLS: [(&str, Drill); 7] = [
    ("jumping contest", Drill::High),
    ("long jump", Drill::Long),
    ("volleyball", Drill::High),
    ("tag", Drill::Dash),
    ("carrot grab", Drill::Dash),
    ("samba", Drill::Steps),
    ("tug of war", Drill::Pull),
];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drill {
    // jumps on the spot, higher each time
    High,
    // a run-up and a leap forward, then back to the mark
    Long,
    // a sprint out and back
    Dash,
    // samba steps from side to side
    Steps,
    // leaning back hard on an imaginary rope
    Pull,
}

#[derive(Clone, Copy, Debug)]
struct Practice {
    game: &'static str,
    drill: Drill,
    learner: usize,
    mentor: Option<usize>,
    // where it trains, reps left, whose go it is (true: the mentor's), seconds to the next move
    at: f32,
    reps: u32,
    showing: bool,
    wait: f32,
    // shows what it's about to train first
    intro: f32,
}

/// A catch that's something to wear: this plus its place in the catalog.
pub(super) const FOUND: i32 = 30000;
/// A fish (props/fish) or a sea creature (props/creatures): this plus its place there.
pub(super) const FISH: i32 = 40000;
pub(super) const CREATURE: i32 = 41000;

fn grounded(wk: &WalkState) -> bool {
    matches!(wk.act, Act::Idle | Act::Walk)
}

impl WalkState {
    /// In a chat, on the ground: walk to the goal or stand facing the others.
    pub(super) fn social_step(&mut self, dt: f32) -> Option<(f32, bool, f32)> {
        let drag = self.drag();
        if !grounded(self) {
            return None;
        }
        if self.giggle > 0.0 && self.social.is_some() {
            return Some(self.giggle_step(dt));
        }
        let r = self.social.as_mut()?;
        r.leaping = false;
        if r.moving {
            self.glide_vx = 0.0;
            let d = r.goal - self.x;
            let s = r.speed * dt * drag;
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

    /// Lost and took a tumble: up with a short smile and one little hop,
    /// not a laughing fit that sets everyone off.
    pub(super) fn good_sport(&mut self) {
        self.smile = 0.8 + self.rand() * 0.5;
        if grounded(self) {
            self.start_bob(tn().jump_height * 0.15);
        }
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
        if self.air > 0.0 && matches!(self.act, Act::Idle | Act::Walk) && !self.social.is_some_and(|s| s.perched) {
            self.act = Act::Fall;
            self.vx = 0.0;
            self.vy = 0.0;
        }
        if self.act == Act::Rest {
            self.rest_pose(dt);
            return;
        }
        if self.act == Act::Travel {
            self.feet = false;
            return;
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
                // back on its feet: laughs it off (not mid-race)
                if self.race.is_none() {
                    let secs = tn().land_laugh * (0.7 + self.rand() * 0.6);
                    self.laugh(secs);
                }
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
        let drag = self.drag();
        let Some(r) = self.social.as_mut().filter(|r| r.inside && !r.flying) else { return false };
        self.vx = 0.0;
        if r.moving {
            let d = r.goal - self.x;
            let s = r.speed * dt * drag;
            if d.abs() > 1e-4 {
                self.left = d < 0.0;
            }
            self.x = if d.abs() <= s { r.goal } else { self.x + d.signum() * s };
        }
        true
    }

    /// Leaping over someone: keeps moving while in the air.
    pub(super) fn social_hop(&mut self, dt: f32) {
        let drag = self.drag();
        let Some(r) = self.social.as_mut().filter(|r| r.leaping && r.moving) else { return };
        self.glide_vx = 0.0;
        let d = r.goal - self.x;
        let s = r.speed * dt * drag;
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
        if !grounded(self) {
            return;
        }
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
        let park_crowd = if let Some(p) = crate::backend::governor::place_rect("park") {
            let n_park = (0..n).filter(|&i| self.walk_ref(i).ws == p.ws).count();
            let base_h = (360.0 * self.config.scale.clamp(0.2, 4.0)).max(64.0);
            let fit = p.w / ((n_park.max(1) as f32).sqrt() * 2.2);
            (fit / base_h).clamp(0.42, 1.0)
        } else {
            1.0
        };
        for i in 0..n {
            let wk = self.walk_of(i);
            wk.social_cool = (wk.social_cool - dt).max(0.0);
            wk.greet_cool = (wk.greet_cool - dt).max(0.0);
            // nothing to do and nobody to do it with: getting bored
            wk.bored = if wk.social.is_none() && wk.at_leisure() { wk.bored + dt } else { 0.0 };
            wk.fish_cool = (wk.fish_cool - dt).max(0.0);
            wk.train_cool = (wk.train_cool - dt).max(0.0);
            wk.petted = (wk.petted - dt).max(0.0);
            wk.smile = (wk.smile - dt).max(0.0);
            wk.dropped = (wk.dropped - dt).max(0.0);
            wk.team = n as f32;
            wk.park_crowd = park_crowd;
            wk.crowd_rest = (wk.crowd_rest - dt).max(0.0);
            wk.update_pose(dt);
        }
        self.run_pat_crowd(dt, n);
        self.run_moods(dt, n);
        for i in 0..n {
            let wk = self.walk_ref(i);
            if wk.fish_on_arrival && wk.act == Act::Idle && wk.trip.is_none() && wk.social.is_none() {
                self.walk_of(i).fish_on_arrival = false;
                let c = self.fish_alone(i, true);
                self.new_chats.push(c);
            }
        }
        self.run_spaces(dt, n);
        self.grow_bonds(dt);
        let mut chats = std::mem::take(&mut self.chats);
        // asked for by hand: tried again for a while if players have to come over first
        if let Some(name) = GAME_NOW.lock().unwrap().take() {
            self.game_wait = Some((name, 45.0, 0.0));
        }
        if let Some((name, left, next)) = self.game_wait.take() {
            let (left, next) = (left - dt, next - dt);
            if next <= 0.0 {
                if !self.force_game(&mut chats, &name, n) && left > 0.0 {
                    self.game_wait = Some((name, left, 1.0));
                }
            } else if left > 0.0 {
                self.game_wait = Some((name, left, next));
            }
        }
        if self.forced_train.is_some() {
            if let Some(c) = self.start_training(n, 0.0) {
                chats.push(c);
            }
        }
        self.hide_cleanup(&chats, n);
        chats.retain_mut(|c| {
            self.watch_tick(c, dt);
            let keep = self.prune(c, n) && self.within_limit(c, dt) && self.advance(c, dt, mouse);
            if !keep {
                self.finish(c, n);
            }
            keep
        });

        chats.append(&mut self.new_chats);
        self.right_orders(&mut chats, n);
        self.park_tick(&mut chats, dt, n);

        self.social_tick -= dt;
        if self.social_tick <= 0.0 {
            self.social_tick = tn().look_every;
            if self.race.is_none() && self.social_age >= tn().warm_alone && !self.moods.fretting && !self.spaces.fs_on {
                for c in chats.iter_mut().filter(|c| matches!(c.stage, Stage::Fishing | Stage::Contest | Stage::Cursor | Stage::Long | Stage::Domino | Stage::Tug | Stage::Wrestle | Stage::Tag | Stage::Carrot | Stage::Volley | Stage::Snow | Stage::Cheer)) {
                    self.try_watch(c, n);
                }
                for c in chats.iter_mut() {
                    let full = c.members.len() >= self.group_cap();
                    if !matches!(c.stage, Stage::Notice | Stage::Stroll | Stage::Concert | Stage::Fishing | Stage::Long | Stage::Domino | Stage::Tug | Stage::Wrestle | Stage::Tag | Stage::Carrot | Stage::Samba | Stage::Volley | Stage::Snow | Stage::Hide | Stage::Part | Stage::Play | Stage::Practice) && self.overlap_join(c, n, full) {
                        continue;
                    }
                    if !full && matches!(c.stage, Stage::Talk | Stage::Stroll) {
                        self.try_join(c, n);
                    }
                }
                for c in chats.iter_mut().filter(|c| c.stage == Stage::Samba && c.step != 9) {
                    self.try_samba_join(c, n);
                }
                self.seek_company(n, tn().look_every);
                if let Some(c) = self.bored_fishing(n, tn().look_every) {
                    chats.push(c);
                }
                self.park_play(&mut chats, n);
                self.park_tell_unit(n);
                if let Some(c) = self.start_training(n, tn().look_every) {
                    chats.push(c);
                }
                for c in chats.iter_mut().filter(|c| c.stage == Stage::Fishing) {
                    self.join_fishing(c, n, tn().look_every);
                }
                self.wave_at_friends(n);
                for c in chats.iter_mut().filter(|c| c.stage == Stage::Talk) {
                    self.call_friend_over(c, n, tn().look_every);
                }
                let room = chats.len() < (n / (tn().chat_divisor as usize).max(1)).max(1);
                chats.extend(self.find_pair(n, room));
            }
        }
        self.chats = chats;
        self.park_tell_world();
        self.settle_states(dt, n);
    }

    /// The state machine's rules, checked for everyone every frame: what a
    /// character is doing has to agree with what it's part of, and nothing
    /// may hang in a state it can't leave. Whatever doesn't fit is put right
    /// gently (back to standing, flags cleared), never by jumping it about.
    pub(super) fn settle_states(&mut self, dt: f32, n: usize) {
        let ids: Vec<(u32, Stage, usize)> = self.chats.iter().chain(self.new_chats.iter()).map(|c| (c.id, c.stage, c.members.len())).collect();
        let racing = self.race.is_some();
        for i in 0..n {
            let playing = self.in_game(i);
            let mode = self.umbrella_mode(i);
            self.walk_of(i).in_game = playing;
            self.walk_of(i).brolly = mode;
            let w = super::weather::now();
            let wk = self.walk_of(i);
            wk.parasol = w.rain && tn().weather_show >= 0.5 && matches!(mode, 1 | 2) && !wk.lost.iter().any(|l| l == "umbrella");
            let wk = self.walk_of(i);
            let was = wk.act;
            // how long in this state, and whether it's getting anywhere
            if wk.act == wk.state_act {
                wk.state_age += dt;
            } else {
                wk.state_act = wk.act;
                wk.state_age = 0.0;
            }
            let mut fixed: Vec<&str> = Vec::new();
            // part of a chat or a game that's gone
            let info = wk.social.and_then(|s| ids.iter().find(|c| c.0 == s.chat).map(|c| (c.1, c.2)));
            let stage = info.map(|i| i.0);
            if let Some(s) = wk.social.as_mut() {
                s.in_talk = stage == Some(Stage::Talk);
                s.crowd = info.is_some_and(|i| i.1 >= 3);
            }
            if wk.social.is_some() && stage.is_none() {
                wk.social = None;
                fixed.push("chat gone");
            }
            // racing in a race that's over
            if wk.race.is_some() && !racing {
                wk.race = None;
                fixed.push("race gone");
            }
            // only ever one thing to belong to
            if wk.crowd.is_some() && (wk.social.is_some() || wk.race.is_some()) {
                wk.crowd = None;
                fixed.push("queue while busy");
            }
            if wk.act == Act::Rest && wk.rest.is_none() {
                wk.act = Act::Idle;
                fixed.push("resting without a rest");
            } else if wk.rest.is_some() && !matches!(wk.act, Act::Rest | Act::Held | Act::Fall) {
                wk.rest = None;
                fixed.push("rest left over");
            }
            if wk.act == Act::Travel && wk.trip.is_none() {
                wk.act = Act::Idle;
                wk.sink = 0.0;
                fixed.push("travelling nowhere");
            } else if wk.trip.is_some() && matches!(wk.act, Act::Held | Act::Fall) {
                // picked up on the way: the trip is off
                wk.trip = None;
                wk.sink = 0.0;
                fixed.push("trip interrupted");
            }
            // states that can't last: falling while on the floor, planted for ages,
            // a jump that never ended
            let floored = wk.air <= 0.0 && wk.vy <= 0.0;
            let stuck = match wk.act {
                Act::Fall => floored && wk.state_age > 1.5,
                Act::Planted => wk.state_age > 6.0,
                // (a high jump at the cursor, or one under an umbrella, can last longer)
                Act::Jump => wk.state_age > wk.jump_dur.max(3.0) + 1.0,
                Act::Pat => wk.state_age > 60.0,
                Act::Chase => wk.state_age > 40.0,
                _ => false,
            };
            if stuck {
                wk.act = Act::Idle;
                wk.air = 0.0;
                wk.vx = 0.0;
                wk.vy = 0.0;
                wk.jump = 0.0;
                wk.tumble = 0.0;
                wk.timer = 1.0;
                wk.target = wk.x;
                fixed.push("stuck state");
            }
            // flags that belong to one kind of game only
            if let Some(r) = wk.social.as_mut() {
                let st = stage.unwrap_or(Stage::Talk);
                let may = life::allows(st);
                let talk = !may.game_flags;
                if r.lean != 0.0 && !may.lean {
                    r.lean = 0.0;
                    fixed.push("lean");
                }
                if r.perched && !may.perched {
                    r.perched = false;
                    fixed.push("perched");
                }
                if r.alpha.is_some() && st != Stage::Play {
                    r.alpha = None;
                    fixed.push("alpha");
                }
                if r.tuck.is_some() && st != Stage::Play {
                    r.tuck = None;
                    fixed.push("tuck");
                }
                if talk && (r.no_touch || r.no_pat || r.face_lock || r.chasing) {
                    r.no_touch = false;
                    r.no_pat = false;
                    r.face_lock = false;
                    r.chasing = false;
                    fixed.push("game flags in a chat");
                }
                if talk && (r.flying || r.stuck) && matches!(wk.act, Act::Idle | Act::Walk) && wk.state_age > 3.0 {
                    r.flying = false;
                    r.stuck = false;
                    fixed.push("still flying in a chat");
                }
                if r.leaping && !r.moving && matches!(wk.act, Act::Idle | Act::Walk) {
                    r.leaping = false;
                }
                if r.grabbed && matches!(wk.act, Act::Idle | Act::Walk) && wk.state_age > 2.0 {
                    r.grabbed = false;
                    fixed.push("grab left over");
                }
                // walking towards a place it can't reach (squeezed off the edge)
                if r.moving && wk.act == Act::Walk {
                    if (wk.x - wk.walk_from).abs() > 0.002 {
                        wk.walk_from = wk.x;
                        wk.walk_stall = 0.0;
                    } else {
                        wk.walk_stall += dt;
                        if wk.walk_stall > 2.5 {
                            r.moving = false;
                            wk.walk_stall = 0.0;
                            fixed.push("walking on the spot");
                        }
                    }
                }
            }
            if !fixed.is_empty() {
                log::info!("state {i}: {was:?}, put right: {}", fixed.join(", "));
            }
        }
    }

    fn each_pair(members: &[usize]) -> Vec<(usize, usize)> {
        members.iter().enumerate().flat_map(|(k, &a)| members[k + 1..].iter().map(move |&b| (a, b))).collect()
    }

    /// Time spent talking makes friends, a second at a time.
    fn grow_bonds(&mut self, dt: f32) {
        self.moods.bonds.tick(dt);
        self.moods.skills.tick(dt);
        self.moods.keepsakes.tick(dt);
        self.moods.bond_tick -= dt;
        if self.moods.bond_tick > 0.0 {
            return;
        }
        self.moods.bond_tick = 1.0;
        let talking: Vec<(usize, usize)> = self.chats.iter().filter(|c| matches!(c.stage, Stage::Talk | Stage::Stroll)).flat_map(|c| Self::each_pair(&c.members)).collect();
        let step = 1.0 / (tn().bond_minutes * 60.0).max(1.0);
        for (a, b) in talking {
            self.add_bond(a, b, step, 0.0);
        }
    }

    /// Average rivalry in a group.
    fn rivalry(&self, members: &[usize]) -> f32 {
        let pairs = Self::each_pair(members);
        pairs.iter().map(|&(a, b)| self.bond(a, b).1).sum::<f32>() / pairs.len().max(1) as f32
    }


    /// What each character is up to, for the settings window.
    pub(super) fn social_report(&mut self) -> serde_json::Value {
        let n = self.extras.len() + 1;
        let workspaces: Vec<u64> = crate::backend::governor::workspaces().unwrap_or_default().iter().map(|s| s.id).collect();
        let mut out = Vec::new();
        for i in 0..n {
            let name = self.character_name(i);
            let wk = self.walk_ref(i);
            let chat = wk.social.and_then(|s| self.chats.iter().find(|c| c.id == s.chat));
            let with: Vec<String> = chat.map(|c| c.members.iter().chain(&c.watchers).filter(|&&j| j != i).map(|&j| self.character_name(j)).collect()).unwrap_or_default();
            let game = chat.map(|c| match c.stage {
                Stage::Contest => "jumping contest",
                Stage::Cursor => "cursor game",
                Stage::Long => "long jump",
                Stage::Domino => "domino",
                Stage::Tug => "tug of war",
                Stage::Wrestle => "wrestling",
                Stage::Tag => "tag",
                Stage::Carrot => "carrot grab",
                Stage::Gift => "gift",
                Stage::Samba => "samba",
                Stage::Volley => "volleyball",
                Stage::Fishing => "fishing",
                Stage::Snow => "snowball fight",
                Stage::Hide => "hide and seek",
                Stage::Practice => "training",
                Stage::Concert => "concert",
                Stage::Cheer => "celebrating",
                Stage::RaceCall | Stage::RaceWait => "getting ready to race",
                _ => "",
            });
            let doing = if wk.race.is_some() {
                "Racing".to_string()
            } else if wk.act == Act::Rest {
                if wk.rest.is_some_and(|r| r.kind == super::moods::RestKind::Sit) { "Sitting".to_string() } else { "Sleeping".to_string() }
            } else if wk.act == Act::Travel {
                "Going to another workspace".to_string()
            } else if wk.act == Act::Held {
                "In your hand".to_string()
            } else if let Some(g) = game.filter(|g| !g.is_empty()) {
                format!("Playing {g}")
            } else if chat.is_some() {
                "Chatting".to_string()
            } else if wk.crowd.is_some() {
                "Gone to have a look".to_string()
            } else if wk.fret {
                "Running about in a panic".to_string()
            } else if matches!(wk.act, Act::Walk | Act::Chase) {
                "Wandering".to_string()
            } else {
                "Standing about".to_string()
            };
            let ws = workspaces.iter().position(|&w| w == wk.ws).map(|k| k + 1);
            let (energy, played) = (wk.energy, wk.played);
            let bonds: Vec<serde_json::Value> = self
                .bonds_of(&name)
                .into_iter()
                .map(|(other, f, r)| serde_json::json!({ "with": other, "friend": f, "rival": r }))
                .collect();
            let skills: Vec<serde_json::Value> = self.moods.skills.all(&name).into_iter().map(|(g, v)| serde_json::json!({ "game": g, "skill": v })).collect();
            let keepsakes: Vec<String> = self.moods.keepsakes.of(&name).into_iter().map(keepsake_name).collect();
            let found = super::wear::found_labels(&name);
            out.push(serde_json::json!({
                "keepsakes": keepsakes,
                "found": found,
                "skills": skills,
                "name": name,
                "doing": doing,
                "with": with,
                "workspace": ws,
                "energy": energy,
                "played": played,
                "bonds": bonds,
            }));
        }
        serde_json::Value::Array(out)
    }

    /// Busy in a game (not just chatting).
    /// What it does with its umbrella in the rain, by what it's up to: 0 none
    /// (it's the volleyball), 1 open in the hand (chatting, watching, the
    /// samba, the top of the volleyball net), 2 open and strapped to its back
    /// (any other game, a race: hands free, it slows them down, and long
    /// jumpers float down with it), 3 rolled up on its back (domino).
    pub(super) fn umbrella_mode(&self, i: usize) -> u8 {
        let wk = self.walk_ref(i);
        if wk.race.is_some() {
            return 2;
        }
        let Some(s) = wk.social else { return 1 };
        let Some(c) = self.chats.iter().chain(self.new_chats.iter()).find(|c| c.id == s.chat) else { return 1 };
        if !c.members.contains(&i) {
            return 1;
        }
        match c.stage {
            Stage::Notice | Stage::Approach | Stage::Talk | Stage::Stroll | Stage::Part | Stage::Cheer | Stage::RaceCall | Stage::RaceWait => 1,
            Stage::Samba | Stage::Concert => 1,
            // toppling over: rolled up out of the way
            Stage::Domino => 3,
            Stage::Volley => match c.volley.as_ref() {
                Some(v) if v.ball == i => 0,
                Some(v) if v.net.last() == Some(&i) => 1,
                _ => 2,
            },
            // everything else: open on its back, and it catches the air
            _ => 2,
        }
    }

    /// Playing a game with its hands full (watching one doesn't count).
    pub(super) fn in_game(&self, i: usize) -> bool {
        self.walk_ref(i).social.is_some_and(|s| {
            self.chats.iter().any(|c| c.id == s.chat && c.members.contains(&i) && !matches!(c.stage, Stage::Notice | Stage::Approach | Stage::Talk | Stage::Stroll | Stage::Part | Stage::Cheer))
        })
    }

    /// Dancing the samba, or being the net or the ball: petting would only
    /// spoil it.
    pub(super) fn dancing(&self, i: usize) -> bool {
        self.walk_ref(i).social.is_some_and(|s| {
            self.chats.iter().any(|c| c.id == s.chat && (c.stage == Stage::Samba || c.volley.as_ref().is_some_and(|v| v.ball == i || v.net.contains(&i))))
        })
    }

    /// 1 with plenty around, up to 3 with only a few on `i`'s workspace.
    fn fewness(&self, i: usize) -> f32 {
        let n = self.extras.len() + 1;
        let ws = self.walk_ref(i).ws;
        let here = (0..n).filter(|&j| self.walk_ref(j).ws == ws).count().max(1);
        (tn().seek_few / here as f32).clamp(1.0, 3.0)
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
        self.keep_world(c, n);
        let game = matches!(c.stage, Stage::Cursor | Stage::Long | Stage::Tug | Stage::Wrestle | Stage::Volley | Stage::Cheer);
        // tag: you can carry someone off and they're still in it when put down
        let tag = matches!(c.stage, Stage::Fishing | Stage::Tag | Stage::Carrot | Stage::Gift | Stage::Samba | Stage::Long);
        let mut left = Vec::new();
        let carried = crate::backend::pointer::BUTTON_DOWN.load(std::sync::atomic::Ordering::Relaxed);
        // picked up by hand: still in the game while in the air; put down
        // near the others it carries on, thrown far off it's out
        let by_hand = matches!(
            c.stage,
            Stage::Contest | Stage::Domino | Stage::Tug | Stage::Wrestle | Stage::Tag | Stage::Carrot | Stage::Gift | Stage::Samba | Stage::Volley | Stage::Snow | Stage::Hide | Stage::Concert | Stage::Cheer
        );
        // a chat isn't broken off either: the others run after whoever's carried away
        let talk = matches!(c.stage, Stage::Approach | Stage::Talk | Stage::Stroll);
        let (mut back, mut out) = (Vec::new(), Vec::new());
        for k in 0..if by_hand || talk { c.members.len() } else { 0 } {
            let i = c.members[k];
            if i >= n {
                continue;
            }
            let x = self.strip_px(i);
            let wk = self.walk_ref(i);
            let Some(s) = wk.social.filter(|s| s.chat == id) else { continue };
            if !s.grabbed {
                if wk.act == Act::Held && carried && wk.hang <= 0.0 {
                    let at = if s.moving { s.goal_px } else { x };
                    if let Some(r) = self.walk_of(i).social.as_mut() {
                        r.grabbed = true;
                        r.grab_px = at;
                        r.lean = 0.0;
                        // in the hand it's neither flying nor lying where it fell
                        r.flying = false;
                        r.stuck = false;
                    }
                    if !talk {
                        c.hold = 10.0;
                    }
                }
            } else if grounded(wk) {
                let others = c.members.iter().filter(|&&m| m != i && m < n).map(|&m| self.strip_px(m));
                let (lo, hi) = others.fold((s.grab_px, s.grab_px), |(lo, hi), p| (lo.min(p), hi.max(p)));
                let far = self.width_of(i) * tn().throw_far;
                if let Some(r) = self.walk_of(i).social.as_mut() {
                    r.grabbed = false;
                }
                // a ball is never out of the game: someone goes and gets it
                let ball = c.volley.as_ref().is_some_and(|v| v.ball == i);
                if !ball && !talk && (x < lo - far || x > hi + far) {
                    out.push(i);
                } else {
                    back.push((i, s.grab_px));
                }
            }
        }
        for (i, px) in back {
            self.back_in(c, i, px);
        }
        if !out.is_empty() {
            log::debug!("chat {id}: {out:?} thrown too far, out of the game");
        }
        c.members.retain(|&i| {
            let stays = i < n && !out.contains(&i) && {
                let wk = self.walk_ref(i);
                // hanging off the cursor and dropping from it is the game
                // (not when someone's picked it up: that's the button held down)
                let playing = (game && (wk.act == Act::Fall || (wk.act == Act::Held && (wk.hang > 0.0 || !carried))))
                    || (wk.social.is_some_and(|s| s.grabbed) && matches!(wk.act, Act::Held | Act::Fall))
                    || (tag && matches!(wk.act, Act::Held | Act::Fall))
                    || (c.stage == Stage::Play && wk.act == Act::Fall);
                wk.social.is_some_and(|s| s.chat == id) && !wk.circle && (playing || !matches!(wk.act, Act::Held | Act::Fall | Act::Chase))
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
                wk.social.is_some_and(|s| s.chat == id) && !wk.circle && !matches!(wk.act, Act::Held | Act::Fall | Act::Chase)
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
        c.members.len() >= 2 || (matches!(c.stage, Stage::Fishing | Stage::Practice | Stage::Play) && !c.members.is_empty())
    }

    /// Someone was picked up: the rest wait till it's back in its place.
    fn held_up(&mut self, c: &mut Chat, dt: f32) -> bool {
        if c.hold <= 0.0 {
            return false;
        }
        let waiting = c.members.iter().any(|&i| {
            let wk = self.walk_ref(i);
            wk.social.is_some_and(|s| s.grabbed || (s.moving && grounded(wk))) || matches!(wk.act, Act::Held | Act::Fall)
        });
        // the timer of the game stands still meanwhile
        c.hold = if waiting { c.hold - dt } else { 0.0 };
        if waiting && wk_held(self, c) {
            c.hold = c.hold.max(1.0);
        }
        c.timer += dt;
        c.hold > 0.0
    }

    /// Someone in the chat is being carried about: the rest hop along
    /// after it, delighted, and gather underneath.
    fn chase_carried(&mut self, c: &Chat, h: usize) {
        let x = self.strip_px(h);
        let gap = self.width_of(h) * 0.7;
        let width = self.strip_width_of(h);
        let mut k = 0;
        for &i in c.members.iter().filter(|&&i| i != h) {
            k += 1;
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            let slot = (x + side * ((k + 1) / 2) as f32 * gap).clamp(0.0, width);
            let here = self.strip_px(i);
            let near = (here - slot).abs() < gap * 0.4;
            // carried right away: they stop and watch rather than chase it across the screen
            let far = (here - x).abs() > self.view_w * tn().carry_follow;
            let wk = self.walk_of(i);
            if !grounded(wk) || wk.social.is_some_and(|s| s.grabbed) {
                continue;
            }
            if let Some(r) = wk.social.as_mut() {
                r.face_lock = false;
                r.leaping = false;
                if far || near {
                    r.moving = false;
                    r.face_left = x < here;
                } else {
                    r.goal_px = slot;
                    r.speed = tn().carry_chase;
                    r.moving = true;
                }
            }
            // now and then a little hop, not one after another
            if wk.rand() < 0.02 {
                wk.start_bob(tn().jump_height * 0.12);
            }
        }
    }

    /// Put down near enough after being picked up: back to its place.
    fn back_in(&mut self, c: &mut Chat, i: usize, px: f32) {
        log::debug!("chat {}: {i} is put down and carries on", c.id);
        match c.stage {
            // the chat moves to wherever it came down: a hop of welcome, and on they talk
            Stage::Approach | Stage::Talk | Stage::Stroll => {
                self.arrange(c);
                for k in 0..c.members.len() {
                    let m = c.members[k];
                    let wk = self.walk_of(m);
                    if m != i && grounded(wk) {
                        wk.start_bob(tn().jump_height * 0.3);
                    }
                }
            }
            Stage::Volley => {
                let Some(v) = c.volley.as_mut() else { return };
                if i == v.ball {
                    if c.step == 9 {
                        self.steer(i, px, 0.3);
                    } else {
                        // a dropped ball: served again
                        v.rest = v.rest.max(1.0);
                        v.runner = None;
                        v.fumbled = false;
                    }
                } else if let Some(&(_, home)) = v.homes.iter().find(|h| h.0 == i) {
                    self.run_to(i, home, 0.3);
                } else {
                    // part of the net: back to it, and up again
                    if c.step != 9 && v.net.iter().position(|&m| m == i).is_some_and(|k| k > 0) {
                        let wk = self.walk_of(i);
                        wk.air = wk.air.max(0.01);
                        if let Some(r) = wk.social.as_mut() {
                            r.perched = true;
                        }
                    }
                    self.steer(i, px, 0.3);
                }
            }
            // these chase each other about anyway
            Stage::Tag | Stage::Carrot | Stage::Samba => {}
            _ => self.steer(i, px, 0.3),
        }
    }

    pub(super) fn width_of(&self, i: usize) -> f32 {
        let r = if i == 0 { self.hit_rect } else { self.extras[i - 1].hit_rect };
        (r[2] - r[0]).max(1.0)
    }

    fn on_screen(&self, i: usize) -> bool {
        let r = if i == 0 { self.hit_rect } else { self.extras[i - 1].hit_rect };
        // in the park, cafe or farm: alive even while that window is scrolled
        // out of view (the three sit side by side, only one shows at a time)
        if r[2] > r[0] && self.place_here(i).is_some() {
            return true;
        }
        // an empty box: not drawn yet
        r[2] > r[0] && r[2] >= 0.0 && r[0] <= self.view_w
    }

    fn enlist(&mut self, i: usize, id: u32) {
        let wk = self.walk_of(i);
        let face_left = wk.left;
        wk.social = Some(Social { chat: id, goal_px: 0.0, goal: 0.0, speed: 0.0, moving: false, face_left, happy: false, chasing: false, hop_cool: 0.0, flying: false, spin: 0.0, stuck: false, rot_target: 0.0, sink_target: 0.0, bump_cool: 0.0, queued: false, launch_px: 0.0, lean: 0.0, feet: false, pace: 1.0, pace_goal: 1.0, pace_wait: 0.0, leaping: false, inside: false, no_touch: false, no_pat: false, jumped: false, face_lock: false, perched: false, grabbed: false, grab_px: 0.0, bounces: 0, in_talk: false, crowd: false, alpha: None, tuck: None });
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

    /// The strip of `i`'s own workspace, which needn't be the one on screen.
    pub(super) fn strip_width_of(&self, i: usize) -> f32 {
        if super::locked() {
            return self.view_w;
        }
        let view = if i == 0 { self.own_view } else { self.extras.get(i - 1).map_or(self.own_view, |e| e.own_view) };
        self.view_w + view[1] * self.view_w * 0.5
    }

    pub(super) fn strip_width(&self) -> f32 {
        if super::locked() {
            return self.view_w;
        }
        crate::backend::compositor::current().strip().map_or(self.view_w, |s| s.width.max(self.view_w))
    }

    fn crowd_talk(c: &Chat) -> f32 {
        c.members.len().saturating_sub(2) as f32 * tn().crowd_talk
    }

    fn talk_length(&mut self) -> f32 {
        tn().talk_min + self.walk.rand() * tn().talk_rand
    }

    /// Mean park depth of the members currently in Anima Park.
    fn park_row_y(&self, members: &[usize]) -> Option<f32> {
        let mut sum = 0.0f32;
        let mut count = 0usize;
        for &m in members {
            if self.place_here(m).is_some() {
                sum += self.walk_ref(m).park_y;
                count += 1;
            }
        }
        (count > 0).then(|| sum / count as f32)
    }

    /// Brings `i` to the same park depth as `members` when both are in the park.
    pub(super) fn match_park_y(&mut self, i: usize, members: &[usize]) {
        if self.place_here(i).is_some() {
            if let Some(y) = self.park_row_y(members) {
                self.walk_of(i).park_target_y = y;
            }
        }
    }

    /// Brings every member in Anima Park to the row's average depth.
    fn align_park_y(&mut self, members: &[usize]) {
        let Some(y) = self.park_row_y(members) else { return };
        for &i in members {
            if self.place_here(i).is_some() {
                self.walk_of(i).park_target_y = y;
            }
        }
    }

    /// Spreads each in-park member across shuffled depth lanes so wide or
    /// free-moving games don't stack horizontally on one line.
    fn spread_park_y(&mut self, members: &[usize]) {
        let mut in_park: Vec<usize> = members.iter().copied().filter(|&i| self.place_here(i).is_some()).collect();
        if in_park.is_empty() {
            return;
        }
        for k in (1..in_park.len()).rev() {
            let j = (self.walk.rand() * (k + 1) as f32) as usize % (k + 1);
            in_park.swap(k, j);
        }
        let lo = 0.06f32;
        let hi = super::park::MEADOW * 0.86;
        let lane = (hi - lo) / in_park.len() as f32;
        for (k, &i) in in_park.iter().enumerate() {
            let center = lo + (k as f32 + 0.5) * lane;
            let jitter = (self.walk.rand() - 0.5) * lane * 0.4;
            self.walk_of(i).park_target_y = (center + jitter).clamp(lo, hi);
        }
    }

    /// Picks a random Anima Park depth for `i` when chasing or wandering.
    fn pick_park_y(&mut self, i: usize, near: Option<(f32, f32)>) {
        if self.place_here(i).is_none() {
            return;
        }
        let lo = 0.06f32;
        let hi = super::park::MEADOW * 0.85;
        let y = match near {
            Some((center, span)) => center + (self.walk.rand() - 0.5) * span,
            None => lo + self.walk.rand() * (hi - lo),
        };
        self.walk_of(i).park_target_y = y.clamp(lo, hi);
    }

    /// Clears lingering happy poses across a group (e.g. when a gather ends).
    fn clear_happy(&mut self, members: &[usize]) {
        for &i in members {
            if let Some(r) = self.walk_of(i).social.as_mut() {
                r.happy = false;
            }
        }
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
        self.go(i, goal.clamp(0.0, self.strip_width_of(i)), 0.18);
        self.match_park_y(i, &others);
    }

    /// Side by side in a row around their middle, in the order they stand.
    /// Whoever's in the happy pose stays in it.
    fn arrange(&mut self, c: &Chat) {
        let mut row: Vec<(usize, f32)> = c.members.iter().map(|&i| (i, self.strip_px(i))).collect();
        row.sort_by(|a, b| a.1.total_cmp(&b.1));
        let count = row.len() as f32;
        let mid = row.iter().map(|r| r.1).sum::<f32>() / count;
        let mid_y = self.park_row_y(&c.members);
        let size = row.iter().map(|r| self.width_of(r.0)).sum::<f32>() / count;
        let gap = size * 0.72;
        for (k, &(i, _)) in row.iter().enumerate() {
            let happy = self.walk_ref(i).social.is_some_and(|r| r.happy);
            let slot = mid + (k as f32 - (count - 1.0) * 0.5) * gap;
            self.go(i, slot, 0.16);
            if let Some(y) = mid_y {
                if self.place_here(i).is_some() {
                    self.walk_of(i).park_target_y = y;
                }
            }
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
            // still walking over: looks where it's going
            if !self.arrived(i) {
                continue;
            }
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
        if std::mem::take(&mut c.encore_due) && c.stage == Stage::Talk {
            self.plan_encore(c);
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
                    // nobody grins while still on the way over
                    r.happy = in_place && (talking || laugh);
                }
                if laugh && in_place {
                    wk.start_bob(tn().jump_height * 0.15);
                }
            }
            if !laugh && roll < 0.45 && self.arrived(c.talker) {
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
        let width = self.strip_width_of(c.members[0]);
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

    /// Eligible games and pick weights for a group of `len` players.
    fn game_pool(played: &[Game], len: usize, rivalry: f32) -> Vec<(Game, f32)> {
        let t = tn();
        [
            (Game::Contest, t.game_min as usize),
            (Game::Long, t.long_min as usize),
            (Game::Domino, t.party_min as usize),
            (Game::Tug, t.tug_min as usize),
            (Game::Wrestle, t.wrestle_min as usize),
            (Game::Tag, t.tag_min as usize),
            (Game::Carrot, t.carrot_min as usize),
            (Game::Samba, t.samba_min as usize),
            (Game::Volley, t.volley_min as usize),
            (Game::Fishing, t.fish_min as usize),
            (Game::Snow, t.snow_min as usize),
            (Game::Hide, t.hide_min as usize),
        ]
        .iter()
        // snowballs only while it's snowing
        .filter(|g| len >= g.1 && !played.contains(&g.0) && (g.0 != Game::Snow || super::weather::now().snow))
        .map(|g| {
            // samba isn't for a handful: the more there are the likelier
            let weight = if g.0 == Game::Samba {
                let k = ((len - g.1) as f32 / (t.samba_big - g.1 as f32).max(1.0)).clamp(0.0, 1.0);
                t.samba_weight_small + (t.samba_weight_big - t.samba_weight_small) * k
            } else {
                let base = (1.0 + (len - g.1) as f32).powf(-t.game_fit);
                if g.0 == Game::Volley { base * t.volley_weight } else if g.0 == Game::Snow { base * t.snow_weight } else { base }
            };
            // rivals would rather settle it
            let settles = matches!(g.0, Game::Contest | Game::Tug | Game::Wrestle | Game::Volley | Game::Carrot);
            (g.0, if settles { weight * (1.0 + rivalry * t.bond_rival) } else { weight })
        })
        .collect()
    }

    /// A game for this many that they haven't played yet: the closer its
    /// minimum is to their number, the likelier.
    fn choose_game(&mut self, played: &[Game], len: usize, rivalry: f32) -> Option<Game> {
        if len >= tn().game_min as usize && !played.contains(&Game::Cursor) && self.walk.rand() < tn().cursor_game {
            return Some(Game::Cursor);
        }
        let pool = Self::game_pool(played, len, rivalry);
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
        let rivalry = self.rivalry(&c.members);
        let Some(game) = self.choose_game(&c.played, c.members.len(), rivalry) else {
            return false;
        };
        for (a, b) in Self::each_pair(&c.members) {
            self.add_bond(a, b, 0.0, tn().bond_game);
        }
        c.played.push(game);
        for &i in &c.members.clone() {
            self.spend(i, tn().energy_game_cost);
            self.walk_of(i).played += 1.0;
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
        let Some(game) = self.choose_game(&c.played, count, 0.0) else {
            return false;
        };
        self.chat_id = self.chat_id.wrapping_add(1);
        let id = self.chat_id;
        let mut other = Chat::new(id, players, Stage::Approach, 0.0);
        other.played = c.played.clone();
        other.games = c.games;
        other.forced = Some(game);
        log::debug!("chat {}: {:?} go off to play {game:?} as chat {id}, {talkers:?} keep talking", c.id, other.members);
        for &i in &other.members.clone() {
            self.spend(i, tn().energy_game_cost);
            self.walk_of(i).played += 1.0;
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
        // played in the park when that's where the players are (whether asked by hand or automatic)
        let lives_there = matches!(tn().park_mode.round() as i32, 1 | 2);
        let park = crate::backend::governor::place_rect("park").filter(|p| {
            c.members.iter().all(|&i| self.walk_ref(i).ws == p.ws)
                && (c.by_hand || lives_there || c.members.iter().all(|&i| self.place_here(i).is_some_and(|q| q.place == "park")))
        });
        GAME_AREA.with(|a| a.set(park.map(|p| (p.x + p.w * 0.05, (p.w * 0.9).max(80.0)))));
        self.start_game_here(c, game);
        GAME_AREA.with(|a| a.set(None));
    }

    fn start_game_here(&mut self, c: &mut Chat, game: Game) {
        c.rewarded = false;
        c.step = 0;
        for k in 0..c.members.len() {
            let wk = self.walk_of(c.members[k]);
            wk.notify_jump = None;
            if let Some(r) = wk.social.as_mut() {
                r.no_pat = true;
                r.happy = false;
            }
        }
        match game {
            Game::Contest => self.start_contest(c),
            Game::Cursor => self.start_cursor_game(c),
            Game::Long => self.start_long(c),
            Game::Domino => self.start_domino(c),
            Game::Tug => self.start_tug(c),
            Game::Wrestle => self.start_wrestle(c),
            Game::Tag => self.start_tag(c),
            Game::Carrot => self.start_carrot(c),
            Game::Gift => self.start_gift(c),
            Game::Samba => self.start_samba(c),
            Game::Volley => self.start_volley(c),
            Game::Fishing => self.start_fishing(c),
            Game::Snow => self.start_snow(c),
            Game::Hide => self.start_hide(c),
        }
        if matches!(game, Game::Tag | Game::Carrot | Game::Cursor | Game::Volley | Game::Contest) {
            self.spread_park_y(&c.members);
        }
    }

    /// A game asked for by hand: whoever is nearest the middle of the
    /// screen plays right now, whatever they were up to and however tired.
    /// True once the game is under way (or can't ever be).
    fn force_game(&mut self, chats: &mut Vec<Chat>, name: &str, n: usize) -> bool {
        // hold up a hug (or random) sign right away on a free character
        if name == "birthday" {
            let pick = self.free_here_pub(n).first().copied();
            if let Some(b) = pick {
                self.start_birthday(b, n);
            }
            return true;
        }
        if matches!(name, "hug" | "sign" | "goodmorning" | "goodnight") {
            let busy: Vec<usize> = chats.iter().flat_map(|c| c.members.iter().copied()).collect();
            let middle = self.view[0] * self.view_w * 0.5 + self.view_w * 0.5;
            let eligible = |this: &Self, i: usize| {
                let wk = this.walk_ref(i);
                !busy.contains(&i) && wk.at_leisure() && wk.rest.is_none() && !wk.night_sleep
            };
            let pick = (0..n)
                .filter(|&i| eligible(self, i) && self.on_screen(i))
                .min_by(|&a, &b| (self.strip_px(a) - middle).abs().total_cmp(&(self.strip_px(b) - middle).abs()))
                .or_else(|| (0..n).find(|&i| eligible(self, i)));
            if let Some(i) = pick {
                let kind = if name == "hug" {
                    super::sign::HUG
                } else if name == "goodmorning" {
                    super::sign::MORNING
                } else if name == "goodnight" {
                    super::sign::NIGHT
                } else {
                    (self.walk.rand() * super::sign::RANDOM as f32) as usize
                };
                let wk = self.walk_of(i);
                wk.act = Act::Idle;
                wk.target = wk.x;
                wk.timer = wk.timer.max(25.0);
                wk.sign = Some(kind);
                wk.sign_left = 25.0;
            }
            return true;
        }
        // training on request: whoever is free right now, with a teacher if there is one
        if name == "train" {
            let busy: Vec<usize> = chats.iter().flat_map(|c| c.members.iter().copied()).collect();
            let middle = self.view[0] * self.view_w * 0.5 + self.view_w * 0.5;
            let pick = (0..n)
                .filter(|&i| !busy.contains(&i) && self.on_screen(i) && matches!(self.walk_ref(i).act, Act::Idle | Act::Walk))
                .min_by(|&a, &b| (self.strip_px(a) - middle).abs().total_cmp(&(self.strip_px(b) - middle).abs()));
            if let Some(i) = pick {
                for k in 0..n {
                    if busy.contains(&k) {
                        continue;
                    }
                    let wk = self.walk_of(k);
                    if k == i {
                        wk.social = None;
                        wk.race = None;
                        wk.rest = None;
                        wk.act = Act::Idle;
                        wk.energy = wk.energy.max(tn().train_energy);
                        wk.bored = tn().train_bored;
                        wk.train_cool = 0.0;
                    }
                }
                self.forced_train = Some(i);
            }
            return true;
        }
        let t = tn();
        let (game, need) = match name {
            "contest" => (Game::Contest, t.game_min),
            "cursor" => (Game::Cursor, t.game_min),
            "long" => (Game::Long, t.long_min),
            "domino" => (Game::Domino, t.party_min),
            "tug" => (Game::Tug, t.tug_min),
            "wrestle" => (Game::Wrestle, t.wrestle_min),
            "tag" => (Game::Tag, t.tag_min),
            "carrot" => (Game::Carrot, t.carrot_min),
            "gift" => (Game::Gift, t.gift_min),
            "samba" => (Game::Samba, t.samba_min),
            "volley" => (Game::Volley, t.volley_min),
            "fishing" => (Game::Fishing, t.fish_min),
            "snow" => (Game::Snow, t.snow_min),
            "hide" => (Game::Hide, t.hide_min),
            _ => return true,
        };
        let need = need as usize;
        if self.race.is_some() || n < need {
            log::debug!("game {name}: can't start now ({n} characters, needs {need})");
            return true;
        }
        // whoever is already playing carries on; the game goes to the others
        let playing: Vec<usize> = chats
            .iter()
            .filter(|c| {
                c.forced.is_some()
                    || matches!(c.stage, Stage::Fishing | Stage::Contest | Stage::Cursor | Stage::Long | Stage::Domino | Stage::Tug | Stage::Wrestle | Stage::Tag | Stage::Carrot | Stage::Gift | Stage::Samba | Stage::Volley | Stage::Snow | Stage::Hide | Stage::Cheer | Stage::RaceCall | Stage::RaceWait)
            })
            .flat_map(|c| c.members.iter().copied())
            .collect();
        let middle = self.view[0] * self.view_w * 0.5 + self.view_w * 0.5;
        let here = |this: &Self, i: usize| this.spaces.active == 0 || this.walk_ref(i).ws == this.spaces.active;
        // asked for by hand: those asleep here get up for it, a hop doesn't count against anyone
        for i in 0..n {
            if !playing.contains(&i) && here(self, i) && self.walk_ref(i).act == Act::Rest {
                let wk = self.walk_of(i);
                wk.rest = None;
                wk.night_sleep = false;
                wk.act = Act::Idle;
                wk.timer = 1.0;
            }
        }
        // living in the park, cafe and farm: the game's in the park, and the
        // others there come over to it first
        // (only when they live there: in the desktop modes it's the desktop on screen)
        let lives_there = matches!(tn().park_mode.round() as i32, 1 | 2);
        let park = crate::backend::governor::place_rect("park").filter(|_| lives_there);
        let in_park = |this: &Self, i: usize| this.place_here(i).is_some_and(|p| p.place == "park");
        if let Some(p) = park.as_ref() {
            for i in 0..n {
                let wk = self.walk_ref(i);
                if !playing.contains(&i) && wk.ws == p.ws && self.place_here(i).is_some() && !in_park(self, i) && wk.trip.is_none() && matches!(wk.act, Act::Idle | Act::Walk) && wk.race.is_none() {
                    self.walk_of(i).social = None;
                    self.walk_into(i, p);
                }
            }
        }
        let middle = park.as_ref().map_or(middle, |p| p.x + p.w * 0.5);
        let park_ws = park.as_ref().map(|p| p.ws);
        let here = |this: &Self, i: usize| match park_ws {
            Some(ws) => this.walk_ref(i).ws == ws,
            None => here(this, i),
        };
        let mut free: Vec<(usize, f32)> = (0..n)
            .filter(|&i| !playing.contains(&i) && here(self, i) && !self.walk_ref(i).circle && matches!(self.walk_ref(i).act, Act::Idle | Act::Walk | Act::Jump))
            .filter(|&i| park.is_none() || in_park(self, i))
            .map(|i| (i, (self.strip_px(i) - middle).abs()))
            .collect();
        // still walking over to the park: wait for them (asked again in a second)
        if park.is_some() && free.len() < need {
            log::debug!("game {name}: {} in the park so far, waiting for more", free.len());
            return false;
        }
        free.sort_by(|a, b| a.1.total_cmp(&b.1));
        if free.len() < need {
            // not enough here: the others are called over from their workspaces
            let active = self.spaces.active;
            let mut called = 0;
            for i in 0..n {
                let wk = self.walk_ref(i);
                if active != 0 && wk.ws != active && wk.trip.is_none() && !playing.contains(&i) && free.len() + called < need + 1 && !wk.visiting && !wk.backstage {
                    if matches!(wk.act, Act::Idle | Act::Walk | Act::Rest | Act::Jump) {
                        let wk = self.walk_of(i);
                        wk.rest = None;
                        wk.social = None;
                        if wk.act == Act::Rest {
                            wk.act = Act::Idle;
                        }
                        if self.send_to(i, active) {
                            called += 1;
                        }
                    }
                }
            }
            let coming = (0..n).filter(|&i| self.heading_to(i) == Some(active)).count();
            log::debug!("game {name}: only {} free here, {coming} on the way, needs {need}", free.len());
            return free.len() + coming < need && coming == 0 && called == 0;
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
        c.by_hand = true;
        self.arrange(&c);
        chats.push(c);
        true
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
                    self.clear_happy(&c.members);
                    c.wait = 0.6;
                    c.step = 0;
                } else {
                    self.chatter(c, dt);
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
            team.iter().filter(|&&i| i < n && c.members.contains(&i) && !self.walk_ref(i).social.is_some_and(|s| s.grabbed)).map(|&i| hand(i)).collect()
        };
        let (left, right) = (ends(&c.teams.0), ends(&c.teams.1));
        let a = left.iter().copied().min_by(|p, q| p.0.total_cmp(&q.0))?;
        let b = right.iter().copied().max_by(|p, q| p.0.total_cmp(&q.0))?;
        Some([a.0, a.1, b.0, b.1])
    }

    pub(super) fn ropes_owned(&self) -> Vec<([f32; 4], usize)> {
        self.chats.iter().filter_map(|c| Some((self.rope_of(c)?, *c.members.first()?))).collect()
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
        self.align_park_y(&c.members);
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
        c.cheater = None;
        c.cheated = None;
        c.cheat_on = false;
        c.bribe = None;
        if !c.teams.0.is_empty() && !c.teams.1.is_empty() && self.walk.rand() < tn().bribe_chance {
            let (a, b) = if self.walk.rand() < 0.5 { (c.teams.0.clone(), c.teams.1.clone()) } else { (c.teams.1.clone(), c.teams.0.clone()) };
            let giver = a[(self.walk.rand() * a.len() as f32) as usize % a.len()];
            let taker = b[(self.walk.rand() * b.len() as f32) as usize % b.len()];
            c.bribe = Some(Bribe { giver, taker, phase: 0, t: 0.0, home: 0.0 });
        }
        c.cheat_t = if self.walk.rand() < tn().tug_cheat_chance { 3.0 + self.walk.rand() * 5.0 } else { -1.0 };
    }

    /// Someone watching runs round to one end of the rope and hauls on it:
    /// that side wins.
    fn tug_cheat(&mut self, c: &mut Chat, dt: f32) {
        if let Some(i) = c.cheater {
            if !c.watchers.contains(&i) {
                c.cheater = None;
                return;
            }
            if self.arrived(i) {
                c.watchers.retain(|&w| w != i);
                c.cheater = None;
                let right = c.cheat_side > 0.0;
                let losers = if right { c.teams.0.clone() } else { c.teams.1.clone() };
                let x = self.tug_end(c, right) - c.offset;
                self.match_park_y(i, &c.members);
                c.members.push(i);
                c.base.push((i, x));
                if right { c.teams.1.push(i) } else { c.teams.0.push(i) }
                if let Some(r) = self.walk_of(i).social.as_mut() {
                    r.face_left = right;
                    r.face_lock = true;
                    r.no_touch = true;
                }
                for l in losers {
                    self.add_bond(i, l, 0.0, tn().bond_game * 2.0);
                }
                c.cheat_on = true;
                c.cheated = Some(i);
                c.timer = c.timer.max(6.0);
                log::debug!("chat {}: {i} cheats, hauling on the {} side", c.id, if right { "right" } else { "left" });
            }
            return;
        }
        if c.cheat_on || c.cheat_t < 0.0 {
            return;
        }
        c.cheat_t -= dt;
        if c.cheat_t > 0.0 {
            return;
        }
        c.cheat_t = -1.0;
        let n = self.extras.len() + 1;
        let middle = c.base.iter().map(|b| b.1).sum::<f32>() / c.base.len().max(1) as f32 + c.offset;
        let ws = self.walk_ref(c.members[0]).ws;
        let range = self.view_w * tn().tug_cheat_range;
        let seen = (0..n)
            .filter(|&i| !c.members.contains(&i) && self.walk_ref(i).ws == ws && self.walk_ref(i).at_leisure() && self.on_screen(i))
            .map(|i| (i, (self.strip_px(i) - middle).abs()))
            .filter(|s| s.1 <= range)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        let Some((i, _)) = seen else { return };
        // on its friends' side, or either if it knows neither better
        let liking = |this: &Self, team: &[usize]| team.iter().map(|&m| this.bond(i, m).0).fold(0.0, f32::max);
        let (l, r) = (liking(self, &c.teams.0), liking(self, &c.teams.1));
        let roll = self.walk.rand();
        c.cheat_side = if r > l || (r == l && roll < 0.5) { 1.0 } else { -1.0 };
        let right = c.cheat_side > 0.0;
        self.enlist(i, c.id);
        c.watchers.push(i);
        c.cheater = Some(i);
        c.timer = c.timer.max(8.0);
        let to = self.tug_end(c, right);
        self.go(i, to, tn().tug_cheat_speed);
        log::debug!("chat {}: {i} saw the tug and runs to cheat", c.id);
    }

    /// Walk over, hand over a carrot, walk back. True while it's going on.
    fn run_bribe(&mut self, c: &mut Chat, dt: f32) -> bool {
        let Some(mut b) = c.bribe else { return false };
        if b.phase == 3 {
            return false;
        }
        if !c.members.contains(&b.giver) || !c.members.contains(&b.taker) {
            c.bribe = None;
            return false;
        }
        match b.phase {
            0 => {
                b.home = self.strip_px(b.giver);
                let tx = self.strip_px(b.taker);
                let side = if b.home < tx { -1.0 } else { 1.0 };
                let size = self.width_of(b.taker);
                self.go(b.giver, tx + side * size * 0.55, 0.22);
                b.phase = 1;
                log::debug!("chat {}: {} goes to slip {} a carrot", c.id, b.giver, b.taker);
            }
            1 => {
                if self.arrived(b.giver) {
                    self.face(b.giver, b.taker);
                    self.face(b.taker, b.giver);
                    b.phase = 2;
                    b.t = 0.0;
                }
            }
            2 => {
                b.t += dt;
                if b.t >= 2.2 {
                    self.walk_of(b.taker).start_bob(tn().jump_height * 0.25);
                    self.go(b.giver, b.home, 0.22);
                    b.phase = 4;
                }
            }
            _ => {
                if self.arrived(b.giver) {
                    b.phase = 3;
                }
            }
        }
        c.bribe = Some(b);
        b.phase != 3
    }

    /// Carrot grab: a bystander sneaks up, snatches the carrot from whoever
    /// has it and eats it. True once it's been snatched this frame.
    fn carrot_thief(&mut self, c: &mut Chat, v: &mut Carrot, h: usize, size: f32, dt: f32) -> bool {
        let hx = self.strip_px(h);
        if let Some(t) = v.thief {
            if !c.watchers.contains(&t) {
                v.thief = None;
                return false;
            }
            self.run_to(t, hx, tn().carrot_sprint * 1.15);
            if self.place_here(t).is_some() {
                let hy = self.walk_ref(h).park_y.clamp(0.06, super::park::MEADOW * 0.85);
                self.walk_of(t).park_target_y = hy;
            }
            if (self.strip_px(t) - hx).abs() >= size * 0.45 || !grounded(self.walk_ref(t)) {
                return false;
            }
            log::debug!("chat {}: {t} snatches the carrot from {h}", c.id);
            v.thief = None;
            v.holder = None;
            v.eating = Some(t);
            v.eat = tn().carrot_eat;
            let away = if hx < (v.lo + v.hi) * 0.5 { 1.0 } else { -1.0 };
            self.go(t, (hx + away * size * 2.0).clamp(v.lo, v.hi), 0.3);
            for k in 0..c.members.len() {
                let i = c.members[k];
                let here = self.strip_px(i);
                self.go(i, here, 0.2);
                let delay = 0.4 + self.walk.rand() * 0.6;
                self.protest(i, t, 2, delay);
                if let Some(r) = self.walk_of(i).social.as_mut() {
                    r.happy = false;
                }
            }
            if let Some(r) = self.walk_of(t).social.as_mut() {
                r.happy = true;
            }
            return true;
        }
        if v.thief_t < 0.0 || !v.started {
            return false;
        }
        v.thief_t -= dt;
        if v.thief_t >= 0.0 {
            return false;
        }
        v.thief_t = -1.0;
        let n = self.extras.len() + 1;
        let ws = self.walk_ref(h).ws;
        let range = self.view_w * 0.8;
        let pick = (0..n)
            .filter(|&i| !c.members.contains(&i) && self.walk_ref(i).ws == ws && self.walk_ref(i).at_leisure() && self.on_screen(i))
            .map(|i| (i, (self.strip_px(i) - hx).abs()))
            .filter(|p| p.1 <= range)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, _)) = pick {
            log::debug!("chat {}: {i} has its eye on the carrot", c.id);
            self.enlist(i, c.id);
            c.watchers.push(i);
            v.thief = Some(i);
            c.timer = c.timer.max(6.0);
        }
        false
    }

    /// Long jump: just landed, and maybe a friend shoves it further on (or it
    /// shuffles on by itself). True if a nudge started.
    fn start_nudge(&mut self, c: &Chat, l: &mut Long, i: usize) -> bool {
        if self.walk.rand() >= tn().long_cheat_chance {
            return false;
        }
        let x = self.strip_px(i);
        let busy = |this: &Self, j: usize| {
            j == i
                || j == l.referee
                || l.running.iter().any(|t| t.flyer == j || t.thrower == Some(j))
                || l.nudges.iter().any(|nd| nd.by == Some(j) || nd.who == j)
                || this.walk_ref(j).social.is_some_and(|s| s.flying || s.stuck)
        };
        let helper = c
            .members
            .iter()
            .copied()
            .filter(|&j| !busy(self, j) && grounded(self.walk_ref(j)) && (self.strip_px(j) - x).abs() < self.view_w * 0.5)
            .max_by(|a, b| self.bond(i, *a).0.total_cmp(&self.bond(i, *b).0));
        let by = if self.walk.rand() < 0.5 { helper } else { None };
        let seer = c
            .members
            .iter()
            .copied()
            .filter(|&j| j != i && Some(j) != by)
            .min_by(|a, b| (self.strip_px(*a) - x).abs().total_cmp(&(self.strip_px(*b) - x).abs()));
        let witness = seer.filter(|_| self.walk.rand() < tn().long_caught);
        let left = tn().long_push_px * (0.6 + self.walk.rand() * 0.8);
        log::debug!("chat {}: {i} gets nudged on by {by:?}, seen by {witness:?}", c.id);
        l.nudges.push(Nudge { who: i, by, left, orig: x, witness, started: by.is_none() });
        true
    }

    fn shift(&mut self, i: usize, px: f32) {
        let view = self.view_w.max(1.0);
        let wk = self.walk_of(i);
        wk.x += px * 2.0 / view;
        wk.target = wk.x;
    }

    fn run_nudges(&mut self, c: &Chat, l: &mut Long, dt: f32) {
        let mut done = Vec::new();
        for k in 0..l.nudges.len() {
            let mut nd = l.nudges[k];
            if !c.members.contains(&nd.who) {
                done.push((k, false));
                continue;
            }
            let x = self.strip_px(nd.who);
            let size = self.width_of(nd.who);
            match nd.by.filter(|b| c.members.contains(b)) {
                Some(b) => {
                    let behind = x - l.dir * size * 0.55;
                    self.run_to(b, behind, 0.8);
                    if !nd.started && (self.strip_px(b) - behind).abs() < 14.0 {
                        nd.started = true;
                    }
                }
                None => {
                    nd.by = None;
                    nd.started = true;
                }
            }
            // alone it goes in little shuffles
            let shuffling = nd.by.is_none() && (c.timer * 9.0).sin() < 0.0;
            if nd.started && !shuffling {
                let step = (tn().long_push_speed * dt).min(nd.left);
                self.shift(nd.who, step * l.dir);
                nd.left -= step;
                if nd.left <= 0.0 {
                    done.push((k, true));
                }
            }
            l.nudges[k] = nd;
        }
        for &(k, _) in done.iter().rev() {
            let nd = l.nudges.remove(k);
            if !done.iter().any(|d| d.0 == k && d.1) {
                continue;
            }
            if let Some(b) = nd.by {
                self.walk_of(b).start_bob(tn().jump_height * 0.2);
            }
            if let Some(w) = nd.witness.filter(|w| c.members.contains(w)) {
                // spotted: hops about until the referee goes by where it really landed
                self.protest(w, nd.who, 3, 0.2);
                l.fixed.push((nd.who, nd.orig));
            }
            l.landed.push(nd.who);
        }
    }

    /// After a race: the first three line up at the finish and dance, the
    /// others who ran stand round and cheer.
    pub(super) fn start_concert(&mut self, order: &[usize], raced: &[usize], finish: f32, n: usize) {
        if self.walk.rand() >= tn().concert_chance {
            return;
        }
        let top: Vec<usize> = order.iter().copied().filter(|&i| i < n).take(3).collect();
        if top.len() < 2 {
            return;
        }
        let crowd: Vec<usize> = raced.iter().copied().filter(|i| *i < n && !top.contains(i)).collect();
        self.chat_id = self.chat_id.wrapping_add(1);
        let id = self.chat_id;
        for &i in top.iter().chain(&crowd) {
            self.enlist(i, id);
        }
        let size = self.width_of(top[0]);
        let width = self.strip_width_of(top[0]);
        let (top_spots, crowd_spots) = concert_spots(finish, width, size, top.len(), crowd.len());
        for (&i, &x) in top.iter().zip(&top_spots) {
            self.go(i, x, 0.2);
        }
        for (&i, &x) in crowd.iter().zip(&crowd_spots) {
            self.go(i, x, 0.2);
        }
        let all: Vec<usize> = top.iter().chain(&crowd).copied().collect();
        self.align_park_y(&all);
        log::debug!("chat {id}: concert by {top:?}");
        let mut chat = Chat::new(id, top.clone(), Stage::Concert, tn().concert_time);
        chat.watchers = crowd;
        // the race was the game
        chat.games = 1;
        chat.talker = top[0];
        chat.winner = Some(top[0]);
        chat.rewarded = true;
        self.new_chats.push(chat);
    }

    fn run_concert(&mut self, c: &mut Chat, dt: f32) {
        let beat = crate::tempo::current();
        let every = if beat.confidence > 0.3 && beat.bpm > 40.0 { 60.0 / beat.bpm } else { 0.5 };
        c.bob -= dt;
        if c.bob <= 0.0 {
            c.bob += every.max(0.25);
            c.turn += 1.0;
            // a little turn every four beats
            let turn = (c.turn as i32) % 4 == 0;
            for k in 0..c.members.len() {
                let i = c.members[k];
                if !self.arrived(i) {
                    continue;
                }
                let wk = self.walk_of(i);
                if let Some(r) = wk.social.as_mut() {
                    r.happy = true;
                    if turn {
                        r.face_left = !r.face_left;
                    }
                }
                wk.start_bob(tn().jump_height * 0.3);
            }
        }
        let stage = c.members[0];
        for w in c.watchers.clone() {
            if self.arrived(w) {
                self.face(w, stage);
                let wk = self.walk_of(w);
                if wk.rand() < dt * tn().race_watch_cheer {
                    let height = tn().jump_height * (0.3 + wk.rand() * 0.4);
                    wk.start_jump(height);
                }
            }
        }
    }

    /// Bored for a while with nothing going on: gets a rod out right here.
    fn bored_fishing(&mut self, n: usize, every: f32) -> Option<Chat> {
        if self.social_age < tn().warm_small || self.moods.fretting {
            return None;
        }
        let i = (0..n).find(|&i| {
            let wk = self.walk_ref(i);
            wk.bored >= tn().fish_bored && wk.fish_cool <= 0.0 && wk.act == Act::Idle && self.on_screen(i)
        })?;
        if self.walk_of(i).rand() >= every / tn().fish_bored_every.max(1.0) {
            return None;
        }
        self.walk_of(i).bored = 0.0;
        Some(self.fish_alone(i, false))
    }

    /// `i` takes a rod to the window in use (or, visiting a fullscreen app,
    /// right where it stands) and starts fishing; others may join it.
    pub(super) fn fish_alone(&mut self, i: usize, here: bool) -> Chat {
        self.chat_id = self.chat_id.wrapping_add(1);
        let id = self.chat_id;
        self.enlist(i, id);
        let size = self.width_of(i);
        let x = if here {
            self.strip_px(i)
        } else {
            let view = self.view[0] * self.view_w * 0.5;
            let (wx, ww) = match self.active.filter(|w| w.w >= size * 2.5) {
                Some(w) => (w.x + view, w.w),
                None => self.biggest_column(),
            };
            wx + size + self.walk.rand() * (ww - size * 2.0).max(0.0)
        };
        self.go(i, x, 0.14);
        log::debug!("chat {id}: {i} goes fishing");
        let mut c = Chat::new(id, vec![i], Stage::Fishing, 0.0);
        c.fishing = Some(Fishing { anglers: vec![Angler { i, phase: 0, t: 0.0, wait: 0.0, item: 0, patience: f32::MAX, at: x, blown: false, thrown: false, glee: 0.0, tries: 0 }], beast: None, beast_cool: 8.0, idle: Vec::new() });
        c.scores = vec![(i, 0.0)];
        c.step = 9;
        c.timer = 30.0;
        // that's this group's game: no fishing again straight after
        c.played.push(Game::Fishing);
        c.games = 1;
        c
    }

    /// The stretch of floor `i` is fishing on: its window, or the screen
    /// around it when that window isn't known.
    fn fishing_window(&self, i: usize, at: f32) -> (f32, f32) {
        let active = self.spaces.active;
        let here = active == 0 || self.walk_ref(i).ws == active;
        let cols = if here && !super::locked() {
            crate::backend::compositor::current().columns()
        } else {
            Vec::new()
        };
        match cols.into_iter().find(|c| at >= c.0 && at <= c.0 + c.1) {
            Some(c) => (c.0, c.0 + c.1),
            None => (at - self.view_w * 0.5, at + self.view_w * 0.5),
        }
    }

    /// Full of energy with nothing to do: off to practise the game it's worst
    /// at. Someone much better at it may come over and show it how.
    fn start_training(&mut self, n: usize, every: f32) -> Option<Chat> {
        if self.forced_train.is_none() && (self.social_age < tn().warm_small || self.moods.fretting || self.race.is_some()) {
            return None;
        }
        let forced = self.forced_train.take();
        let i = forced.or_else(|| {
            (0..n).find(|&i| {
                let wk = self.walk_ref(i);
                wk.at_leisure() && wk.act == Act::Idle && wk.energy >= tn().train_energy && wk.bored >= tn().train_bored && wk.train_cool <= 0.0 && self.on_screen(i)
            })
        })?;
        if forced.is_none() && self.walk_of(i).rand() >= every / tn().train_every.max(1.0) {
            return None;
        }
        // the weakest of what can be practised alone
        let (game, drill) = DRILLS.iter().copied().min_by(|a, b| self.skill(i, a.0).total_cmp(&self.skill(i, b.0)))?;
        let own = self.skill(i, game);
        let x = self.strip_px(i);
        let ws = self.walk_ref(i).ws;
        // a better one close by and at a loose end may come and teach
        let near: Vec<usize> = (0..n)
            .filter(|&j| j != i && self.walk_ref(j).ws == ws && self.walk_ref(j).at_leisure() && (self.strip_px(j) - x).abs() < self.view_w * 0.6)
            .collect();
        let mut best: Option<(usize, f32)> = None;
        for j in near {
            let s = self.skill(j, game);
            let score = s + self.bond(i, j).0 * 0.2;
            if s >= (own + tn().mentor_gap).max(0.35) && best.is_none_or(|b| score > b.1) {
                best = Some((j, score));
            }
        }
        let mentor = best.map(|b| b.0).filter(|_| forced.is_some() || self.walk.rand() < tn().mentor_chance);
        self.walk_of(i).bored = 0.0;
        self.chat_id = self.chat_id.wrapping_add(1);
        let id = self.chat_id;
        let mut members = vec![i];
        members.extend(mentor);
        for &m in &members {
            self.enlist(m, id);
        }
        let size = self.width_of(i);
        let width = self.strip_width_of(i);
        let at = x.clamp(size * 3.0, (width - size * 3.0).max(size * 3.0));
        self.go(i, at, 0.14);
        if let Some(m) = mentor {
            let side = if self.strip_px(m) < at { -1.0 } else { 1.0 };
            self.go(m, at + side * size * 1.4, 0.16);
        }
        log::debug!("chat {id}: {i} goes to practise {game} ({own:.2}){}", mentor.map_or(String::new(), |m| format!(", {m} shows how")));
        let mut c = Chat::new(id, members, Stage::Practice, 0.0);
        c.games = 1;
        let reps = tn().train_reps as u32 + (self.walk.rand() * 3.0) as u32;
        c.practice = Some(Practice { game, drill, learner: i, mentor, at, reps, showing: mentor.is_some(), wait: 0.0, intro: 2.0 });
        c.step = 9;
        c.timer = 120.0;
        Some(c)
    }

    /// One go at the drill for `who`, from the mark `at`. True while it's
    /// still underway.
    fn drill_move(&mut self, who: usize, p: &Practice, start: bool) -> bool {
        let size = self.width_of(who);
        let dir = if self.walk_ref(who).left { -1.0 } else { 1.0 };
        let home = if Some(who) == p.mentor { self.goal_of(who) } else { p.at };
        if start {
            match p.drill {
                Drill::High => {
                    let h = tn().jump_height * (0.9 + self.walk.rand() * 0.8);
                    self.walk_of(who).start_jump(h);
                }
                Drill::Long => {
                    let wk = self.walk_of(who);
                    wk.start_jump(tn().jump_height * 0.9);
                    if let Some(r) = wk.social.as_mut() {
                        r.goal_px = home + dir * size * 2.2;
                        r.speed = 0.45;
                        r.moving = true;
                        r.leaping = true;
                    }
                }
                Drill::Dash => {
                    self.run_to(who, home + dir * size * 4.0, 0.6);
                }
                Drill::Steps => {
                    let side = if self.walk.rand() < 0.5 { -1.0 } else { 1.0 };
                    let wk = self.walk_of(who);
                    wk.start_bob(tn().jump_height * 0.2);
                    if let Some(r) = wk.social.as_mut() {
                        r.goal_px = home + side * size * 0.4;
                        r.speed = 0.3;
                        r.moving = true;
                        r.leaping = true;
                    }
                }
                Drill::Pull => {
                    if let Some(r) = self.walk_of(who).social.as_mut() {
                        r.lean = if r.face_left { -tn().tug_lean * 1.3 } else { tn().tug_lean * 1.3 };
                    }
                }
            }
            return true;
        }
        // back to the mark once the move is done
        let wk = self.walk_ref(who);
        let busy = !grounded(wk) || wk.social.is_some_and(|s| s.moving);
        if busy {
            return true;
        }
        if (self.strip_px(who) - home).abs() > 6.0 {
            self.go(who, home, 0.3);
            return true;
        }
        if let Some(r) = self.walk_of(who).social.as_mut() {
            r.lean = 0.0;
        }
        false
    }

    /// Training: the mentor shows each move, the learner copies it; alone it
    /// just goes through its reps. It gets better faster with a teacher.
    fn run_practice(&mut self, c: &mut Chat, dt: f32) -> bool {
        let Some(mut p) = c.practice.take() else { return false };
        if !c.members.contains(&p.learner) || c.timer <= 0.0 {
            return false;
        }
        if p.mentor.is_some_and(|m| !c.members.contains(&m)) {
            p.mentor = None;
            p.showing = false;
        }
        if c.step == 9 {
            if self.all_arrived(c) {
                c.step = 0;
                if let Some(m) = p.mentor {
                    self.face(m, p.learner);
                    self.face(p.learner, m);
                    self.walk_of(m).gesture = true;
                }
            }
            c.practice = Some(p);
            return true;
        }
        // first it holds up what it's going to work on
        if p.intro > 0.0 {
            p.intro -= dt;
            c.practice = Some(p);
            return true;
        }
        let who = if p.showing { p.mentor.unwrap_or(p.learner) } else { p.learner };
        if c.step == 1 {
            if self.drill_move(who, &p, false) {
                c.practice = Some(p);
                return true;
            }
            c.step = 0;
            p.wait = 0.5 + self.walk.rand() * 0.6;
            if who == p.learner {
                p.reps = p.reps.saturating_sub(1);
                // the teacher's pleased with it
                if let Some(m) = p.mentor {
                    let wk = self.walk_of(m);
                    wk.start_bob(tn().jump_height * 0.25);
                    wk.laugh(0.6);
                }
            }
            if p.mentor.is_some() {
                p.showing = !p.showing;
                if let (Some(m), true) = (p.mentor, p.showing) {
                    self.face(p.learner, m);
                }
            }
        }
        if p.reps == 0 {
            // done: that's sunk in, and it's tired
            let name = self.character_name(p.learner);
            let teach = p.mentor.map_or(0.0, |m| (self.skill(m, p.game) - self.skill(p.learner, p.game)).max(0.0) * tn().mentor_boost);
            self.moods.skills.practice(&name, p.game, teach);
            if let Some(m) = p.mentor {
                let mn = self.character_name(m);
                self.moods.skills.practice(&mn, p.game, 0.0);
                self.add_bond(p.learner, m, tn().bond_game, 0.0);
            }
            log::debug!("chat {}: {} finished practising {}{}", c.id, p.learner, p.game, if p.mentor.is_some() { " with a teacher" } else { "" });
            for &i in &c.members.clone() {
                self.spend(i, tn().train_cost);
                let wk = self.walk_of(i);
                wk.train_cool = tn().train_again;
                wk.start_jump(tn().jump_height * 0.7);
            }
            return false;
        }
        p.wait -= dt;
        if c.step == 0 && p.wait <= 0.0 {
            self.drill_move(who, &p, true);
            c.step = 1;
        }
        c.practice = Some(p);
        true
    }

    /// Now and then something comes up beside the anglers: they drop
    /// everything and run, and it sinks back after 2-3 seconds.
    fn run_beast(&mut self, f: &mut Fishing, first: usize, watchers: &[usize], dt: f32) {
        if let Some(b) = f.beast.as_mut() {
            b.1 += dt;
            if b.1 >= b.2 {
                f.beast = None;
                f.beast_cool = tn().fish_beast_cool;
                log::debug!("the beast goes back down");
            }
            return;
        }
        f.beast_cool -= dt;
        // one scare at a time anywhere: several groups fishing don't each get their own
        static LAST: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
        let quiet = LAST.lock().unwrap().is_some_and(|t| t.elapsed().as_secs_f32() < tn().fish_beast_cool);
        if quiet || f.beast_cool > 0.0 || self.walk.rand() >= dt * tn().fish_beast {
            return;
        }
        *LAST.lock().unwrap() = Some(std::time::Instant::now());
        let ready: Vec<usize> = (0..f.anglers.len()).filter(|&k| (1..=5).contains(&f.anglers[k].phase) && !f.anglers[k].blown).collect();
        if ready.is_empty() {
            return;
        }
        let near = ready[(self.walk.rand() * ready.len() as f32) as usize % ready.len()];
        let i = f.anglers[near].i;
        let size = self.width_of(i);
        let side = if self.walk.rand() < 0.5 { -1.0 } else { 1.0 };
        let width = self.strip_width_of(first);
        let x = (self.strip_px(i) + side * size * 0.9).clamp(size, (width - size).max(size));
        let stay = 2.0 + self.walk.rand();
        f.beast = Some((x, 0.0, stay));
        log::debug!("a beast comes up at {x:.0} for {stay:.1}s");
        let reach = size * tn().fish_beast_reach;
        for a in &mut f.anglers {
            let at = self.strip_px(a.i);
            if (at - x).abs() > reach {
                continue;
            }
            a.blown = true;
            let dir = if (at - x).abs() < 1.0 { -side } else { (at - x).signum() };
            let far = reach * (1.0 + self.walk.rand() * 0.5);
            self.go(a.i, (x + dir * far).clamp(0.0, width), 0.42);
        }
        // those watching run off too
        for &w in watchers {
            let at = self.strip_px(w);
            if (at - x).abs() > reach * 1.3 {
                continue;
            }
            let dir = if (at - x).abs() < 1.0 { -side } else { (at - x).signum() };
            let far = reach * (1.2 + self.walk.rand() * 0.5);
            if let Some(r) = self.walk_of(w).social.as_mut() {
                r.happy = false;
            }
            self.go(w, (x + dir * far).clamp(0.0, width), 0.42);
            if let Some(d) = f.idle.iter_mut().find(|d| d.0 == w) {
                // looks back for a bit once it's safe
                d.1 = 1.5 + self.walk.rand();
                d.2 = None;
            }
        }
    }


    /// The confetti goes off with a bang: everyone close by is blown back,
    /// lands on their feet and laughs. Returns who went flying.
    fn boom(&mut self, i: usize) -> Vec<usize> {
        let n = self.extras.len() + 1;
        let (x, ws) = (self.strip_px(i), self.walk_ref(i).ws);
        log::debug!("{i}'s confetti goes off with a bang");
        let mut flung = Vec::new();
        let reach = self.width_of(i) * tn().fish_boom_reach;
        for j in 0..n {
            let d = self.strip_px(j) - x;
            let wk = self.walk_ref(j);
            if wk.ws != ws || d.abs() > reach || matches!(wk.act, Act::Held | Act::Fall) || wk.race.is_some() {
                continue;
            }
            let push = 1.0 - d.abs() / reach.max(1.0);
            let dir = if d.abs() < 1.0 { if self.walk.rand() < 0.5 { -1.0 } else { 1.0 } } else { d.signum() };
            let wk = self.walk_of(j);
            wk.act = Act::Fall;
            wk.jump = 0.0;
            wk.air = wk.air.max(0.3);
            wk.vx = dir * (0.4 + push * 0.9);
            wk.vy = 4.0 + push * 6.0;
            wk.tumble = dir * (8.0 + wk.rand() * 8.0);
            wk.held_prev = None;
            // lands on its feet and laughs it off with little hops
            wk.leaped = true;
            let secs = tn().laugh_time * 1.3;
            wk.laugh(secs);
            if let Some(s) = wk.social.as_mut() {
                s.happy = true;
            }
            flung.push(j);
        }
        flung
    }

    /// Confetti all round, and everyone near laughs.
    fn confetti(&mut self, i: usize) {
        let n = self.extras.len() + 1;
        let (x, ws) = (self.strip_px(i), self.walk_ref(i).ws);
        let r = if i == 0 { self.hit_rect } else { self.extras[i - 1].hit_rect };
        let h = (r[3] - r[1]).max(1.0);
        self.moods.effects.push((20, i, x, r[1] + h * 0.3, h * 1.6, 1.4));
        for j in 0..n {
            if j != i && self.walk_ref(j).ws == ws && (self.strip_px(j) - x).abs() < self.view_w * 0.3 {
                let wk = self.walk_of(j);
                let secs = tn().laugh_time * 0.5;
                wk.laugh(secs);
            }
        }
    }

    /// Someone strolling by sees the rods out and joins in beside them.
    fn join_fishing(&mut self, c: &mut Chat, n: usize, every: f32) {
        let Some(f) = c.fishing.as_ref() else { return };
        // nobody new once it's past halfway: it winds down instead of going on and on
        if c.members.len() >= tn().fish_most as usize || (c.step != 9 && c.timer < tn().fish_time * 0.5) {
            return;
        }
        let ws = self.walk_ref(c.members[0]).ws;
        let xs: Vec<f32> = f.anglers.iter().map(|a| self.strip_px(a.i)).collect();
        let (lo, hi) = (xs.iter().copied().fold(f32::MAX, f32::min), xs.iter().copied().fold(f32::MIN, f32::max));
        let range = self.view_w * 0.6;
        let who = (0..n).find(|&j| {
            !c.members.contains(&j) && self.walk_ref(j).ws == ws && self.free_for(j, Want::Fish) && (self.strip_px(j) - (lo + hi) * 0.5).abs() < range
        });
        let Some(j) = who else { return };
        let keen = 1.0 + c.members.iter().map(|&m| self.bond(j, m).0).fold(0.0, f32::max);
        if self.walk_of(j).rand() >= every * tn().fish_join * keen {
            return;
        }
        let size = self.width_of(j);
        let x = self.strip_px(j);
        let width = self.strip_width_of(j);
        // the nearer end of the row, one place along
        let spot = if (x - lo).abs() < (x - hi).abs() { lo - size * 1.1 } else { hi + size * 1.1 };
        let spot = spot.clamp(size, (width - size).max(size));
        self.enlist(j, c.id);
        self.match_park_y(j, &c.members);
        c.members.push(j);
        c.scores.push((j, 0.0));
        self.go(j, spot, 0.14);
        if let Some(f) = c.fishing.as_mut() {
            f.anglers.push(Angler { i: j, phase: 0, t: 0.0, wait: 0.0, item: 0, patience: f32::MAX, at: spot, blown: false, thrown: false, glee: 0.0, tries: 0 });
        }
        log::debug!("chat {}: {j} sees them fishing and joins", c.id);
    }

    /// Fishing along the floor of the widest window: rods out, wait for a
    /// bite, reel in whatever it is.
    fn start_fishing(&mut self, c: &mut Chat) {
        let (x, cw) = self.big_window();
        let (lo, hi) = (x + 80.0, x + cw - 80.0);
        let slot = (hi - lo).max(1.0) / c.members.len() as f32;
        let mut anglers = Vec::new();
        for (k, &i) in c.members.clone().iter().enumerate() {
            let at = lo + (k as f32 + 0.5) * slot;
            self.go(i, at, 0.16);
            anglers.push(Angler { i, phase: 0, t: 0.0, wait: 0.0, item: 0, patience: f32::MAX, at, blown: false, thrown: false, glee: 0.0, tries: 0 });
        }
        log::debug!("chat {}: fishing, {} rods", c.id, anglers.len());
        self.align_park_y(&c.members);
        c.fishing = Some(Fishing { anglers, beast: None, beast_cool: 8.0, idle: Vec::new() });
        c.scores = c.members.iter().map(|&i| (i, 0.0)).collect();
        c.step = 9;
        c.timer = 40.0;
        c.regroup = true;
        c.stage = Stage::Fishing;
    }

    fn run_fishing(&mut self, c: &mut Chat, dt: f32) {
        let Some(mut f) = c.fishing.take() else { return };
        f.anglers.retain(|a| c.members.contains(&a.i));
        if f.anglers.is_empty() || c.timer <= 0.0 {
            self.end_game(c);
            return;
        }
        if c.step == 9 {
            if self.all_arrived(c) {
                self.clear_happy(&c.members);
                c.step = 0;
                c.timer = tn().fish_time;
                for a in &mut f.anglers {
                    a.phase = 1;
                    a.t = 0.0;
                }
                for (k, a) in f.anglers.iter().enumerate() {
                    if let Some(r) = self.walk_of(a.i).social.as_mut() {
                        r.face_left = k % 2 == 1;
                    }
                }
            } else {
                self.chatter(c, dt);
            }
            c.fishing = Some(f);
            return;
        }
        let watchers = c.watchers.clone();
        self.run_beast(&mut f, c.members[0], &watchers, dt);
        self.fish_watchers(c, &mut f, dt);
        let mut bored_off = Vec::new();
        for k in 0..f.anglers.len() {
            let mut a = f.anglers[k];
            if self.walk_ref(a.i).act == Act::Held {
                a.blown = true;
                a.thrown = true;
            }
            // thrown a long way: that's the end of fishing for this one
            if a.thrown && grounded(self.walk_ref(a.i)) {
                a.thrown = false;
                let (x, size) = (self.strip_px(a.i), self.width_of(a.i));
                let (lo, hi) = self.fishing_window(a.i, a.at);
                let far = size * tn().fish_throw_far;
                if x < lo - far || x > hi + far {
                    bored_off.push(a.i);
                    f.anglers[k] = a;
                    continue;
                }
                a.at = free_fishing_spot(&f, a.i, x, lo, hi, size);
                self.go(a.i, a.at, 0.16);
            }
            // blown off its spot: lands, laughs, walks back, casts again
            if a.blown {
                let wk = self.walk_ref(a.i);
                // nobody goes back while the thing is still out
                let settled = grounded(wk) && wk.giggle <= 0.0 && f.beast.is_none();
                if settled {
                    if (self.strip_px(a.i) - a.at).abs() > 12.0 {
                        if !wk.social.is_some_and(|s| s.moving) {
                            a.tries += 1;
                            if a.tries > 3 {
                                // can't get there: fishes where it is
                                a.at = self.strip_px(a.i);
                            } else {
                                self.go(a.i, a.at, 0.16);
                            }
                        }
                    } else if self.arrived(a.i) {
                        a.tries = 0;
                        a.blown = false;
                        a.phase = 1;
                        a.t = 0.0;
                    }
                }
                f.anglers[k] = a;
                continue;
            }
            a.t += dt;
            // came along later: rod out once it's in place, facing the way the others do
            if a.phase == 0 && self.arrived(a.i) {
                a.phase = 1;
                a.t = 0.0;
                let first = f.anglers[0].i;
                let left = if first != a.i { self.walk_ref(first).social.map(|s| s.face_left) } else { None };
                if let Some(r) = self.walk_of(a.i).social.as_mut() {
                    r.face_left = left.unwrap_or_else(|| r.face_left);
                }
            }
            // never got to its place: gives up
            if a.phase == 0 && a.t > 45.0 {
                bored_off.push(a.i);
                f.anglers[k] = a;
                continue;
            }
            if a.phase >= 1 {
                if a.patience == f32::MAX {
                    a.patience = tn().fish_stay_min + self.walk.rand() * tn().fish_stay_rand;
                }
                a.patience -= dt;
                // lost interest: only between catches, never mid-reel
                if a.patience <= 0.0 && matches!(a.phase, 1 | 2) {
                    bored_off.push(a.i);
                    f.anglers[k] = a;
                    continue;
                }
            }
            match a.phase {
                1 if a.t > 0.8 => {
                    a.phase = 2;
                    a.t = 0.0;
                    // beginners wait longer for a bite
                    let patience = self.skilled(a.i, "fishing", 1.6, 0.7);
                    a.wait = (tn().fish_wait_min + self.walk.rand() * tn().fish_wait_rand) * patience;
                }
                2 if a.t > a.wait => {
                    a.phase = 3;
                    a.t = 0.0;
                    if a.glee > 0.0 {
                        a.glee = 0.0;
                        if let Some(r) = self.walk_of(a.i).social.as_mut() {
                            r.happy = false;
                        }
                    }
                }
                2 => {
                    if a.glee > 0.0 {
                        a.glee -= dt;
                        if a.glee <= 0.0 {
                            if let Some(r) = self.walk_of(a.i).social.as_mut() {
                                r.happy = false;
                            }
                        }
                    } else if self.walk.rand() < dt * tn().fish_glee {
                        a.glee = 1.2 + self.walk.rand();
                        let wk = self.walk_of(a.i);
                        if let Some(r) = wk.social.as_mut() {
                            r.happy = true;
                        }
                        wk.start_bob(tn().jump_height * 0.08);
                    }
                }
                3 if a.t > 0.9 => {
                    // quick enough? the more practice, the likelier
                    let quick = self.skilled(a.i, "fishing", 0.45, 0.95);
                    if self.walk.rand() < quick {
                        // the better at it, the better the catch
                        let luck = self.skill(a.i, "fishing");
                        let good = self.walk.rand() < 0.50 + 0.42 * luck;
                        let pool: Vec<&(i32, f32, bool)> = CATCHES.iter().filter(|x| x.2 == good).collect();
                        let pick = pool[(self.walk.rand() * pool.len() as f32) as usize % pool.len()];
                        a.item = pick.0;
                        // a good one is mostly a real fish, now and then a sea creature
                        if good && self.walk.rand() < tn().fish_real {
                            let rare = self.walk.rand() < tn().fish_creature;
                            let (base, count) = if rare { (CREATURE, super::rope::sea_count(CREATURE)) } else { (FISH, super::rope::sea_count(FISH)) };
                            if count > 0 {
                                a.item = base + (self.walk.rand() * count as f32) as i32 % count as i32;
                            }
                        }
                        // good catches (other than rare creatures) can turn up something to wear
                        if good && a.item < CREATURE && self.walk.rand() < tn().fish_find {
                            let name = self.character_name(a.i);
                            let roll = self.walk.rand();
                            if let Some(k) = super::wear::unfound(&name, roll) {
                                a.item = FOUND + k as i32;
                            }
                        }
                        a.phase = 4;
                        log::debug!("chat {}: {} reels in kind {}", c.id, a.i, a.item);
                    } else {
                        // got away
                        log::debug!("chat {}: {} lost one", c.id, a.i);
                        a.phase = 2;
                        a.wait = tn().fish_wait_min * 0.5 + self.walk.rand() * tn().fish_wait_rand * 0.5;
                        a.patience -= tn().fish_lost_cost;
                        self.walk_of(a.i).start_bob(tn().jump_height * 0.15);
                    }
                    a.t = 0.0;
                }
                4 if a.t > tn().fish_reel => {
                    a.phase = 5;
                    a.t = 0.0;
                    let (_, points, good) = if a.item >= CREATURE { (a.item, 8.0, true) } else if a.item >= FISH { (a.item, 4.0, true) } else if a.item >= FOUND { (a.item, 6.0, true) } else { *CATCHES.iter().find(|x| x.0 == a.item).unwrap_or(&CATCHES[0]) };
                    // found something to wear: it's in its things now
                    if let Some((id, label, _, _)) = (FOUND..FISH).contains(&a.item).then(|| super::wear::catalog_entry((a.item - FOUND) as usize)).flatten() {
                        let name = self.character_name(a.i);
                        super::wear::give(&name, &id);
                        log::debug!("chat {}: {} found {label}", c.id, a.i);
                    }
                    if let Some(s) = c.scores.iter_mut().find(|s| s.0 == a.i) {
                        s.1 += points;
                    }
                    // a good catch keeps it interested, junk wears it down
                    a.patience += if good { tn().fish_catch_keeps } else { -tn().fish_lost_cost };
                    if good && !keepsake_name(a.item).is_empty() {
                        let name = self.character_name(a.i);
                        self.moods.keepsakes.add(&name, a.item);
                    }
                    if good {
                        // a good one: up it jumps, high and happy
                        let wk = self.walk_of(a.i);
                        wk.start_jump(tn().jump_height * 1.4);
                        let secs = tn().laugh_time * 1.5;
                        wk.laugh(secs);
                    } else if let Some(r) = self.walk_of(a.i).social.as_mut() {
                        r.happy = false;
                    }
                }
                // a present on the line: opened there and then
                5 if a.item == 1 && a.t > 0.9 => {
                    a.t = 0.0;
                    let inside = [15, 16, 0, 20];
                    a.item = inside[(self.walk.rand() * inside.len() as f32) as usize % inside.len()];
                    log::debug!("chat {}: {} opens the present: kind {}", c.id, a.i, a.item);
                    if !keepsake_name(a.item).is_empty() {
                        let name = self.character_name(a.i);
                        self.moods.keepsakes.add(&name, a.item);
                    }
                    let wk = self.walk_of(a.i);
                    wk.start_jump(tn().jump_height * 1.2);
                    wk.laugh(tn().laugh_time);
                    if a.item == 20 {
                        self.confetti(a.i);
                        // now and then the confetti goes off with a bang
                        if self.walk.rand() < tn().fish_boom {
                            let flung = self.boom(a.i);
                            // whoever went flying has had enough fishing
                            if flung.contains(&a.i) {
                                a.blown = true;
                            }
                            for o in f.anglers.iter_mut().filter(|o| flung.contains(&o.i)) {
                                o.blown = true;
                            }
                        }
                    }
                }
                5 if a.t > 1.8 && a.item != 1 => {
                    a.phase = 1;
                    a.t = 0.0;
                }
                _ => {}
            }
            f.anglers[k] = a;
        }
        // one by one they lose interest and wander off; the last one ends it
        let last = f.anglers.len() == bored_off.len();
        for i in bored_off {
            log::debug!("chat {}: {i} is tired of fishing", c.id);
            if !last {
                self.practice(&[i], "fishing");
                f.anglers.retain(|a| a.i != i);
                c.members.retain(|&m| m != i);
                c.scores.retain(|s| s.0 != i);
                self.leave(i, c.id);
                self.push_down(i);
            }
            let wk = self.walk_of(i);
            wk.fish_cool = tn().fish_again;
            wk.start_bob(tn().jump_height * 0.15);
        }
        if last {
            log::debug!("chat {}: fishing over", c.id);
            for a in &f.anglers {
                self.push_down(a.i);
            }
            self.end_game(c);
            return;
        }
        c.fishing = Some(f);
    }

    /// Just behind the last one pulling on that side.
    fn tug_end(&self, c: &Chat, right: bool) -> f32 {
        let team = if right { &c.teams.1 } else { &c.teams.0 };
        let xs = team.iter().filter_map(|t| c.base.iter().find(|b| b.0 == *t).map(|b| b.1));
        let gap = self.row_size(c) * 0.72;
        let end = if right { xs.fold(f32::MIN, f32::max) + gap } else { xs.fold(f32::MAX, f32::min) - gap };
        end + c.offset
    }

    fn run_tug(&mut self, c: &mut Chat, dt: f32, mouse: [f32; 2]) {
        c.wait -= dt;
        match c.step {
            0 => {
                if c.bribe.is_some_and(|b| b.phase != 0 && b.phase != 3) || self.all_arrived(c) {
                    if self.run_bribe(c, dt) {
                        return;
                    }
                    self.clear_happy(&c.members);
                    for (i, left) in c.teams.0.iter().map(|&i| (i, false)).chain(c.teams.1.iter().map(|&i| (i, true))).collect::<Vec<_>>() {
                        if let Some(r) = self.walk_of(i).social.as_mut() {
                            r.face_left = left;
                            // dragged backwards by the rope, still facing it
                            r.face_lock = true;
                        }
                    }
                    c.timer = tn().tug_time;
                    c.step = 1;
                } else {
                    self.chatter(c, dt);
                }
            }
            1 => {
                // every so often one side digs in harder; the rope follows
                // that smoothly, it never jumps
                if c.wait <= 0.0 {
                    c.wait = 0.5 + self.walk.rand() * 0.5;
                    let mut force = 0.0;
                    for i in c.teams.0.clone() {
                        let knack = self.skilled(i, "tug of war", 0.5, 1.3);
                        force -= self.walk.rand() * knack;
                    }
                    for i in c.teams.1.clone() {
                        let knack = self.skilled(i, "tug of war", 0.5, 1.3);
                        force += self.walk.rand() * knack;
                    }
                    c.pull_target = (force * tn().tug_pull).clamp(-tn().tug_speed, tn().tug_speed);
                    // the bribed one isn't really pulling
                    if let Some(b) = c.bribe.filter(|b| b.phase == 3) {
                        let side = if c.teams.1.contains(&b.giver) { 1.0 } else { -1.0 };
                        c.pull_target = (c.pull_target + side * tn().tug_speed * 0.6).clamp(-tn().tug_speed, tn().tug_speed);
                    }
                }
                self.tug_cheat(c, dt);
                if c.cheat_on {
                    c.pull_target = c.cheat_side * tn().tug_speed;
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
                        // the rope snapped on everyone: that's funny, not a loss
                        if let Some(r) = self.walk_of(i).social.as_mut() {
                            r.happy = true;
                        }
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
                        if let Some(r) = self.walk_of(i).social.as_mut().filter(|r| !r.grabbed) {
                            r.lean = amount;
                        }
                    }
                }
                if c.offset.abs() >= tn().tug_win || c.timer <= 0.0 {
                    let dir = if c.offset >= 0.0 { 1.0 } else { -1.0 };
                    let (winners, losers) = if dir > 0.0 { (c.teams.1.clone(), c.teams.0.clone()) } else { (c.teams.0.clone(), c.teams.1.clone()) };
                    log::debug!("chat {}: tug won by {winners:?}", c.id);
                    // the losers noticed the extra pair of hands: little indignant hops
                    if let Some(ch) = c.cheated.filter(|_| self.walk.rand() < tn().tug_caught) {
                        log::debug!("chat {}: {ch} is caught cheating", c.id);
                        for &l in losers.iter().filter(|l| c.members.contains(l)) {
                            let delay = 1.2 + self.walk.rand() * 0.6;
                            self.protest(l, ch, 3, delay);
                        }
                    }
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

    /// Two teams line up on either side and send one fighter each to clash
    /// head-to-head in the middle until one is pushed out and tumbled away.
    fn start_wrestle(&mut self, c: &mut Chat) {
        log::debug!("chat {}: wrestling", c.id);
        let mut all = c.members.clone();
        for k in (1..all.len()).rev() {
            let j = (self.walk.rand() * (k + 1) as f32) as usize % (k + 1);
            all.swap(k, j);
        }
        let half = all.len() / 2;
        let teams = [all[..half].to_vec(), all[half..].to_vec()];
        let unit = self.width_of(c.members[0]);
        let (x, cw) = self.big_window();
        let (lo, hi, mid) = wrestle_arena(x, cw, unit);
        let win = tn().wrestle_win.min(((hi - lo) * 0.25).max(unit * 0.75));
        let ring = (win + unit * 0.95).min(((hi - lo) * 0.42).max(unit * 0.9));
        let gap = unit * 0.42;

        let arena_y = self
            .park_row_y(&c.members)
            .map_or(0.32, |y| y.clamp(0.14, super::park::MEADOW * 0.55));
        for &i in &c.members.clone() {
            let wk = self.walk_of(i);
            wk.notify_jump = None;
            if let Some(r) = wk.social.as_mut() {
                r.no_touch = true;
                r.lean = 0.0;
                r.face_lock = false;
            }
        }
        let w = Wrestle {
            mid,
            lo,
            hi,
            gap,
            ring,
            unit,
            teams,
            out: [Vec::new(), Vec::new()],
            pair: None,
            phase: 0,
            t: 0.0,
            push: 0.0,
            push_target: 0.0,
            offset: 0.0,
            cheer_t: 0.3,
            glee: Vec::new(),
            winner: None,
            loser: None,
            party: 0.0,
            decide: [false; 2],
            arena_y,
        };
        self.wrestle_benches(&w, None);
        c.step = 9;
        c.wait = 5.0;
        c.timer = tn().wrestle_round;
        c.scores.clear();
        c.regroup = true;
        c.stage = Stage::Wrestle;
        c.wrestle = Some(w);
    }

    fn wrestle_benches(&mut self, w: &Wrestle, active: Option<(usize, usize)>) {
        const LANES: [f32; 4] = [-0.16, -0.06, 0.08, 0.18];
        let span = ((w.hi - w.lo) * 0.5 - w.ring).max(0.0);
        for side in 0..=1 {
            let dir = if side == 0 { -1.0 } else { 1.0 };
            let bench: Vec<usize> = w.teams[side]
                .iter()
                .chain(w.out[side].iter())
                .copied()
                .filter(|&i| active.is_none_or(|(a, b)| i != a && i != b))
                .collect();
            let step = (span / bench.len().saturating_sub(1).max(1) as f32).min(w.unit * 0.9).max(w.unit * 0.5);
            for (slot, i) in bench.into_iter().enumerate() {
                if self.walk_ref(i).social.is_some_and(|r| r.grabbed || r.flying || r.stuck) {
                    continue;
                }
                let x = (w.mid + dir * (w.ring + slot as f32 * step)).clamp(w.lo, w.hi);
                if (self.strip_px(i) - x).abs() > w.unit * 0.12 {
                    self.steer(i, x, 0.20);
                }
                if self.place_here(i).is_some() {
                    self.walk_of(i).park_target_y = (w.arena_y + LANES[slot % LANES.len()]).clamp(0.06, super::park::MEADOW * 0.85);
                }
                if let Some(r) = self.walk_of(i).social.as_mut() {
                    r.face_left = side == 1;
                    r.face_lock = false;
                    r.lean = 0.0;
                }
            }
        }
    }

    fn finish_wrestle(&mut self, c: &mut Chat, w: Wrestle) {
        let win_side = match (!w.teams[0].is_empty(), !w.teams[1].is_empty()) {
            (true, false) => Some(0),
            (false, true) => Some(1),
            _ => None,
        };
        c.scores.clear();
        if let Some(side) = win_side {
            c.scores = w.teams[side]
                .iter()
                .chain(w.out[side].iter())
                .copied()
                .filter(|i| c.members.contains(i))
                .map(|i| (i, 1.0))
                .collect();
        }
        c.wrestle = Some(w);
        self.end_game(c);
    }

    fn run_wrestle(&mut self, c: &mut Chat, dt: f32) {
        let Some(mut w) = c.wrestle.take() else { return };
        for side in 0..=1 {
            w.teams[side].retain(|i| c.members.contains(i));
            w.out[side].retain(|i| c.members.contains(i));
        }
        if let Some((a, b)) = w.pair {
            if !c.members.contains(&a) || !c.members.contains(&b) {
                w.pair = None;
                w.phase = 0;
                w.offset = 0.0;
                w.push = 0.0;
            }
        }
        if w.teams[0].is_empty() || w.teams[1].is_empty() {
            self.finish_wrestle(c, w);
            return;
        }
        if c.step == 9 {
            c.wait -= dt;
            if self.all_arrived(c) || c.wait <= 0.0 {
                for side in 0..=1 {
                    for &i in w.teams[side].iter().chain(w.out[side].iter()) {
                        if let Some(r) = self.walk_of(i).social.as_mut() {
                            r.happy = false;
                            if !r.moving {
                                r.face_left = side == 1;
                            }
                        }
                    }
                }
                c.step = 0;
                // both sides talk over who goes first
                w.decide = [true, true];
                w.phase = 4;
                w.t = 1.2 + self.walk.rand() * 1.0;
            } else {
                self.chatter(c, dt);
            }
            c.wrestle = Some(w);
            return;
        }

        // a happy moment is just a moment: back to watching after it
        for g in &mut w.glee {
            g.1 -= dt;
        }
        for (i, _) in w.glee.iter().filter(|g| g.1 <= 0.0).copied().collect::<Vec<_>>() {
            if let Some(r) = self.walk_of(i).social.as_mut() {
                r.happy = false;
            }
        }
        w.glee.retain(|g| g.1 > 0.0);

        // everyone not in the ring, benches and those who came to watch, keeps
        // its eyes on the duel and turns as it moves along
        let duel = match w.pair {
            Some((a, b)) if w.phase >= 1 => (self.strip_px(a) + self.strip_px(b)) * 0.5,
            _ => w.mid + w.offset,
        };
        let watching: Vec<usize> = w.teams[0]
            .iter()
            .chain(w.teams[1].iter())
            .chain(w.out[0].iter())
            .chain(w.out[1].iter())
            .copied()
            .chain(c.watchers.iter().copied().filter(|&i| Some(i) != c.cheater && self.arrived(i)))
            .filter(|&i| w.pair.is_none_or(|(a, b)| i != a && i != b))
            .filter(|&i| !(w.phase == 4 && (0..=1).any(|s| w.decide[s] && w.teams[s].contains(&i))))
            .collect();
        for &i in &watching {
            let x = self.strip_px(i);
            if (duel - x).abs() < w.unit * 0.15 {
                continue;
            }
            if let Some(r) = self.walk_of(i).social.as_mut() {
                if !r.moving && !r.flying && !r.stuck && !r.grabbed {
                    r.face_left = duel < x;
                }
            }
        }

        // now and then one of them hops and cheers for a moment
        w.cheer_t -= dt;
        if w.cheer_t <= 0.0 {
            w.cheer_t = 0.8 + self.walk.rand() * 0.6;
            let fans: Vec<usize> = watching
                .iter()
                .copied()
                .filter(|&i| Some(i) != w.loser && !w.glee.iter().any(|g| g.0 == i) && self.walk_ref(i).social.is_some_and(|r| !r.moving && !r.flying && !r.stuck))
                .collect();
            if !fans.is_empty() {
                let fan = fans[(self.walk.rand() * fans.len() as f32) as usize % fans.len()];
                let secs = 1.0 + self.walk.rand();
                w.glee.push((fan, secs));
                let wk = self.walk_of(fan);
                if let Some(r) = wk.social.as_mut() {
                    r.happy = true;
                }
                if grounded(wk) {
                    let hop = tn().jump_height * (0.12 + wk.rand() * 0.1);
                    wk.start_bob(hop);
                }
            }
        }

        let win_dist = tn().wrestle_win.min(((w.hi - w.lo) * 0.25).max(w.unit * 0.75));
        match w.phase {
            0 => {
                let (a, b) = (w.teams[0][0], w.teams[1][0]);
                self.wrestle_benches(&w, Some((a, b)));
                for (i, goal, left) in [(a, w.mid - w.gap, false), (b, w.mid + w.gap, true)] {
                    self.go(i, goal, 0.24);
                    if self.place_here(i).is_some() {
                        self.walk_of(i).park_target_y = w.arena_y;
                    }
                    if let Some(r) = self.walk_of(i).social.as_mut() {
                        r.face_left = left;
                        r.face_lock = false;
                        r.lean = 0.0;
                    }
                }
                w.pair = Some((a, b));
                w.offset = 0.0;
                w.push = 0.0;
                w.push_target = 0.0;
                w.t = 0.0;
                w.phase = 1;
            }
            1 => {
                let Some((a, b)) = w.pair else { w.phase = 0; c.wrestle = Some(w); return };
                w.t += dt;
                if (self.arrived(a) && self.arrived(b)) || w.t > 6.0 {
                    for (i, left) in [(a, false), (b, true)] {
                        let wk = self.walk_of(i);
                        if let Some(r) = wk.social.as_mut() {
                            r.face_left = left;
                            r.face_lock = true;
                            r.moving = false;
                        }
                        if grounded(wk) {
                            wk.start_bob(tn().jump_height * 0.14);
                        }
                    }
                    w.phase = 2;
                    w.t = 0.0;
                    c.timer = tn().wrestle_round;
                }
            }
            2 => {
                let Some((a, b)) = w.pair else { w.phase = 0; c.wrestle = Some(w); return };
                w.t -= dt;
                if w.t <= 0.0 {
                    w.t = 0.55 + self.walk.rand() * 0.45;
                    let sa = self.skilled(a, "wrestling", 0.55, 1.35) * (self.walk_ref(a).energy / 100.0).clamp(0.6, 1.15);
                    let sb = self.skilled(b, "wrestling", 0.55, 1.35) * (self.walk_ref(b).energy / 100.0).clamp(0.6, 1.15);
                    let force = (self.walk.rand() * sa - self.walk.rand() * sb) * tn().wrestle_push;
                    w.push_target = force.clamp(-tn().wrestle_push, tn().wrestle_push);
                }
                w.push += (w.push_target - w.push) * (dt * 3.5).min(1.0);
                w.offset = (w.offset + w.push * dt).clamp(-win_dist * 1.15, win_dist * 1.15);

                let heave = (c.timer * 7.0).sin().abs() * 0.04;
                let bias = (w.push / tn().wrestle_push.max(1.0)).clamp(-1.0, 1.0) * 0.07;
                for (i, goal, left, lean) in [
                    (a, (w.mid + w.offset - w.gap).clamp(w.lo, w.hi), false, -(tn().wrestle_lean + heave + bias.max(0.0) - (-bias).max(0.0) * 0.4)),
                    (b, (w.mid + w.offset + w.gap).clamp(w.lo, w.hi), true, tn().wrestle_lean + heave + (-bias).max(0.0) - bias.max(0.0) * 0.4),
                ] {
                    if !self.walk_ref(i).social.is_some_and(|r| r.grabbed) {
                        self.steer(i, goal, 0.45);
                    }
                    if self.place_here(i).is_some() {
                        self.walk_of(i).park_target_y = w.arena_y;
                    }
                    let wk = self.walk_of(i);
                    if let Some(r) = wk.social.as_mut() {
                        r.face_left = left;
                        r.face_lock = true;
                        r.lean = lean;
                    }
                    if grounded(wk) && wk.rand() < dt * 1.1 {
                        wk.start_bob(tn().jump_height * 0.13);
                    }
                }

                if w.offset.abs() >= win_dist || c.timer <= 0.0 {
                    let win_side = if w.offset.abs() >= 2.0 {
                        usize::from(w.offset < 0.0)
                    } else {
                        usize::from(self.walk.rand() < 0.5)
                    };
                    let loser_side = 1 - win_side;
                    let (winner_f, loser, dir) = if win_side == 0 { (a, b, 1.0) } else { (b, a, -1.0) };
                    for i in [a, b] {
                        if let Some(r) = self.walk_of(i).social.as_mut() {
                            r.lean = 0.0;
                            r.face_lock = false;
                            r.moving = false;
                        }
                    }
                    let dist = (tn().wrestle_fling_min + self.walk.rand() * tn().wrestle_fling_rand).min((w.hi - w.lo) * 0.45);
                    let time = 0.55 + self.walk.rand() * 0.25;
                    self.fling(loser, dir, dist, time);
                    let wwk = self.walk_of(winner_f);
                    if let Some(r) = wwk.social.as_mut() {
                        r.happy = true;
                    }
                    if grounded(wwk) {
                        wwk.start_bob(tn().jump_height * 0.45);
                    }
                    w.winner = Some(winner_f);
                    w.loser = Some(loser);
                    w.party = 1.6 + self.walk.rand() * 0.8;
                    // the side that lost one picks who's next
                    w.decide = [false; 2];
                    w.decide[loser_side] = true;
                    w.teams[loser_side].retain(|&m| m != loser);
                    if !w.out[loser_side].contains(&loser) {
                        w.out[loser_side].push(loser);
                    }
                    w.phase = 3;
                    w.t = 1.0;
                }
            }
            4 => {
                // each side still picking: they turn to one another, nod, wave
                // a hand about, then one of them steps up
                w.t -= dt;
                for side in 0..=1 {
                    if !w.decide[side] {
                        continue;
                    }
                    let team = w.teams[side].clone();
                    for (k, &i) in team.iter().enumerate() {
                        let other = if team.len() < 2 { None } else if k % 2 == 0 { team.get(k + 1).or(team.get(k.wrapping_sub(1))) } else { team.get(k - 1) };
                        if let Some(&o) = other {
                            if !self.walk_ref(i).social.is_some_and(|r| r.moving) {
                                self.face(i, o);
                            }
                        }
                        let wk = self.walk_of(i);
                        if grounded(wk) && wk.rand() < dt * 0.8 {
                            wk.start_bob(tn().jump_height * 0.07);
                        }
                        if wk.rand() < dt * 0.6 {
                            wk.gesture = true;
                        }
                    }
                }
                if w.t <= 0.0 {
                    for side in 0..=1 {
                        if !std::mem::take(&mut w.decide[side]) || w.teams[side].is_empty() {
                            continue;
                        }
                        // the keenest steps up: fresh, good at it, and a bit of luck
                        let team = w.teams[side].clone();
                        let mut keen = Vec::new();
                        for &i in team.iter().filter(|&&i| Some(i) != w.winner) {
                            let k = self.walk_ref(i).energy + self.skill(i, "wrestling") + self.walk.rand();
                            keen.push((i, k));
                        }
                        let pick = keen.iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|k| k.0);
                        if let Some(p) = pick {
                            w.teams[side].retain(|&m| m != p);
                            w.teams[side].insert(0, p);
                            log::debug!("chat {}: side {side} sends {p} next", c.id);
                            let wk = self.walk_of(p);
                            if grounded(wk) {
                                wk.start_bob(tn().jump_height * 0.2);
                            }
                            for &o in &team {
                                if o != p {
                                    self.face(o, p);
                                }
                            }
                        }
                    }
                    w.phase = 0;
                }
            }
            _ => {
                let airborne = c.members.iter().any(|&i| self.walk_ref(i).social.is_some_and(|s| s.flying));
                if !airborne {
                    w.t -= dt;
                }
                // the winner's still celebrating: hops about, happy (the
                // time only runs once the loser is down)
                if w.party > 0.0 {
                    if !airborne {
                        w.party -= dt;
                    }
                    if let Some(v) = w.winner {
                        let wk = self.walk_of(v);
                        if grounded(wk) && wk.rand() < dt * 3.0 {
                            let hop = tn().jump_height * (0.2 + wk.rand() * 0.2);
                            wk.start_bob(hop);
                        }
                        if w.party <= 0.0 {
                            if let Some(r) = wk.social.as_mut() {
                                r.happy = false;
                            }
                        }
                    }
                }
                if !airborne && w.t <= 0.0 && w.party <= 0.0 {
                    for &i in w.out[0].iter().chain(w.out[1].iter()) {
                        let wk = self.walk_of(i);
                        if wk.act == Act::Planted {
                            wk.act = Act::Idle;
                        }
                        // the loser gets up with a short smile and one little hop
                        if wk.social.is_some_and(|r| r.stuck) {
                            wk.good_sport();
                        }
                        if let Some(r) = wk.social.as_mut() {
                            r.flying = false;
                            r.stuck = false;
                            r.rot_target = 0.0;
                            r.sink_target = 0.0;
                            r.lean = 0.0;
                            r.happy = false;
                        }
                    }
                    if w.teams[0].is_empty() || w.teams[1].is_empty() {
                        self.finish_wrestle(c, w);
                        return;
                    }
                    w.pair = None;
                    // the side that lost one talks over who goes next; with
                    // nobody to talk to, the last one just steps up
                    let left = (0..=1).filter(|&s| w.decide[s]).map(|s| w.teams[s].len()).max().unwrap_or(0);
                    w.t = if left > 1 { 1.5 + self.walk.rand() * 1.0 } else { 1.0 };
                    w.phase = 4;
                }
            }
        }
        c.wrestle = Some(w);
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
        let (x, cw) = self.big_window();
        // a crowd gets a taller net; the teams must still come out equal
        // (a shorter one in narrower windows or for fewer players, so it stays below the ceiling)
        let tall = all.len() >= tn().volley_big as usize && cw >= 550.0;
        let mut high = if tall { 4 } else if cw >= 380.0 { 3 } else { 2 };
        while high > 1 && all.len() < high + 3 {
            high -= 1;
        }
        if all.len() < high + 3 {
            // not even one a side: no game
            self.end_game(c);
            return;
        }
        if (all.len() - high - 1) % 2 == 1 {
            if let Some(i) = all.pop() {
                self.leave(i, c.id);
            }
        }
        c.members = all.clone();
        let pad = (cw * 0.08).clamp(16.0, 40.0);
        let (lo, hi, mid) = (x + pad, (x + cw - pad).max(x + pad + 40.0), x + cw * 0.5);
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
                // the ball can be picked up and thrown, just not petted
                r.no_touch = i != ball;
                r.no_pat = true;
            }
        }
        c.volley = Some(Volley { held: 0.0, knocker: None, lofted: false, teams, homes, ball, net, fumbled: false, mid, lo, hi, score: [0, 0], server: 0, rest: 0.0, runner: None, to: 0.0, react: 0.0, run_speed: 0.0, top_speed: 0.0, rally: 0 });
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

    /// The ball's been in the hand too long: the nearest player runs under
    /// it, leaps right up to it and knocks it over the net.
    fn knock_from_hand(&mut self, v: &mut Volley) {
        let x = self.strip_px(v.ball);
        let air = self.walk_ref(v.ball).air;
        if let Some(p) = v.knocker {
            // in the air: the hit lands when its head gets to the ball
            let wk = self.walk_ref(p);
            if wk.act != Act::Fall {
                v.knocker = None;
                return;
            }
            if wk.air < air - 1.9 && wk.vy > 0.0 {
                return;
            }
            let half = ((v.hi - v.lo) * 0.5).max(20.0);
            let near = (half * 0.22).min(90.0);
            let far = (half * 0.85).max(near);
            let (a, b) = if v.teams[0].contains(&p) { (v.mid + near, v.mid + far) } else { (v.mid - far, v.mid - near) };
            let to = (a + self.walk.rand() * (b - a).max(0.0)).clamp(v.lo + 8.0, v.hi - 8.0);
            let time = v.clear_time(x, to, air).max(0.9);
            log::debug!("{p} knocks the ball out of the hand");
            self.ball_to(v.ball, to, time);
            // out of the hand for good: it no longer hangs from where it was gripped
            let ball = self.walk_of(v.ball);
            ball.grip = false;
            ball.grab = None;
            ball.swing = 0.0;
            ball.swing_v = 0.0;
            ball.swing_last = None;
            ball.tumble = 0.0;
            ball.rolling = false;
            if let Some(r) = ball.social.as_mut() {
                r.grabbed = false;
            }
            // the hitter drops back with a little kick from the blow
            let wk = self.walk_of(p);
            wk.vy = wk.vy.min(0.0) - 2.0;
            if let Some(&(_, home)) = v.homes.iter().find(|h| h.0 == p) {
                self.run_to(p, home, 0.25);
            }
            v.knocker = None;
            v.held = 0.0;
            v.rest = 0.0;
            v.runner = None;
            v.fumbled = false;
            v.lofted = true;
            v.rally += 1;
            return;
        }
        let free = |this: &Self, p: usize| this.walk_ref(p).petted <= 0.0 && !this.walk_ref(p).social.is_some_and(|s| s.grabbed);
        let Some(p) = v.teams.iter().flatten().copied().filter(|&p| free(self, p)).min_by(|a, b| (self.strip_px(*a) - x).abs().total_cmp(&(self.strip_px(*b) - x).abs())) else {
            return;
        };
        let px = self.strip_px(p);
        if (px - x).abs() > 28.0 || !grounded(self.walk_ref(p)) {
            // right under it, wherever the hand takes it
            self.run_to(p, x, tn().volley_speed);
            return;
        }
        // up it goes, high enough for its head to reach the ball
        let w = self.view_w.max(1.0);
        let apex = (air - 1.2).max(0.6);
        let vy = (2.0 * tn().gravity * apex).sqrt();
        let wk = self.walk_of(p);
        wk.act = Act::Fall;
        wk.air = wk.air.max(0.02);
        wk.vy = vy;
        wk.vx = (x - px) * 2.0 / w / (vy / tn().gravity).max(0.1);
        wk.jump = 0.0;
        wk.held_prev = None;
        wk.left = x < px;
        if let Some(r) = wk.social.as_mut() {
            r.moving = false;
        }
        v.knocker = Some(p);
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
            && v.teams.iter().flatten().all(|i| c.members.contains(i))
            // a side with nobody on it (or no net) can't be played
            && v.teams.iter().all(|t| !t.is_empty())
            && !v.net.is_empty();
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
            if top.social.is_some_and(|s| s.grabbed) {
                continue;
            }
            top.air += (k as f32 * 1.7 - top.air) * (dt * 6.0).min(1.0);
        }
        // the ball is in someone's hand
        if self.walk_ref(v.ball).social.is_some_and(|s| s.grabbed) {
            if self.walk_ref(v.ball).act == Act::Held {
                // held too long: someone runs up, jumps and knocks it out
                v.held += dt;
                if v.held > tn().volley_hold {
                    self.knock_from_hand(&mut v);
                }
            } else if self.walk_ref(v.ball).act == Act::Fall {
                // thrown: a ball in the air like any other
                let wk = self.walk_of(v.ball);
                let spin = if wk.vx < 0.0 { 8.0 } else { -8.0 };
                if let Some(r) = wk.social.as_mut() {
                    r.grabbed = false;
                    r.flying = true;
                    r.stuck = false;
                    r.moving = false;
                    r.spin = spin;
                }
                v.held = 0.0;
                v.knocker = None;
                v.rest = 0.0;
                v.runner = None;
                v.fumbled = false;
                v.lofted = true;
            }
            c.volley = Some(v);
            return;
        }
        v.held = 0.0;
        v.knocker = None;
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
            v.rest = 1.8;
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
        // someone being petted is too happy to play
        let closest = |this: &Self, team: &[usize], at: f32| {
            team.iter().copied().filter(|&p| this.walk_ref(p).petted <= 0.0 && !this.walk_ref(p).social.is_some_and(|s| s.grabbed)).min_by(|a, b| (this.strip_px(*a) - at).abs().total_cmp(&(this.strip_px(*b) - at).abs()))
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
        if let Some(r) = v.runner.filter(|&r| self.walk_ref(r).petted <= 0.0) {
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
            let clumsy = 1.0 - self.skill(player, "volleyball");
            let fumble = tn().volley_fumble + (v.teams[side].len() as f32 - 1.0) * 0.02 + v.rally as f32 * 0.01 + clumsy * tn().skill_volley;
            if v.fumbled || self.walk.rand() < fumble {
                v.fumbled = true;
                c.volley = Some(v);
                return;
            }
            // any power and pace: a hard one flies long and comes off the side,
            // a high one comes off the top
            let dir = if side == 0 { 1.0 } else { -1.0 };
            let half = ((v.hi - v.lo) * 0.5).max(20.0);
            let near = (half * 0.22).min(90.0);
            let far = (half * 0.85).max(near);
            let (a, b) = if side == 0 { (v.mid + near, v.mid + far) } else { (v.mid - far, v.mid - near) };
            let power = if self.walk.rand() < 0.3 { self.walk.rand() * (half * 0.35).min(240.0) } else { 0.0 };
            let to = (a + self.walk.rand() * (b - a).max(0.0) + dir * power).clamp(v.lo + 8.0, v.hi - 8.0);
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
    pub(super) fn props(&self) -> Vec<(i32, [f32; 4], bool, usize)> {
        let mut v: Vec<(i32, [f32; 4], bool, usize)> = self.chats.iter().filter(|c| c.stage == Stage::Carrot && c.step != 9).filter_map(|c| Some((0, self.carrot_rect(c)?, false, *c.members.first()?))).collect();
        // gentle floating hearts while reacting to a hug
        for i in 0..=self.extras.len() {
            let h_left = self.walk_ref(i).hug;
            if h_left <= 0.0 {
                continue;
            }
            let r = if i == 0 { self.hit_rect } else { self.extras[i - 1].hit_rect };
            let w = (r[2] - r[0]).max(1.0);
            let h = (r[3] - r[1]).max(1.0);
            let age = (1.65 - h_left).max(0.0);
            let cx = (r[0] + r[2]) * 0.5;
            let cy = r[1] + h * 0.45;
            for (k, side, delay) in [(0, -1.0f32, 0.0f32), (1, 1.0f32, 0.18f32)] {
                if age < delay {
                    continue;
                }
                let t = ((age - delay) / 1.35).clamp(0.0, 1.0);
                if t >= 1.0 {
                    continue;
                }
                let rise = t * h * 0.52;
                let drift = side * w * (0.14 + 0.08 * t) + (t * 6.0 + k as f32).sin() * w * 0.04;
                let size = h * 0.19 * (1.0 - 0.25 * t);
                let x = cx + drift;
                let y = cy - rise;
                v.push((100, [x - size * 0.5, y - size * 0.5, x + size * 0.5, y + size * 0.5], false, i));
            }
        }
        // training: what it's about to practise, over its head
        for c in self.chats.iter().filter(|c| c.stage == Stage::Practice && c.step != 9) {
            let Some(p) = c.practice else { continue };
            if p.intro <= 0.0 {
                continue;
            }
            let r = if p.learner == 0 { self.hit_rect } else if let Some(e) = self.extras.get(p.learner - 1) { e.hit_rect } else { continue };
            let h = r[3] - r[1];
            let icon = match p.drill {
                Drill::High | Drill::Long => 15,
                Drill::Dash => 7,
                Drill::Steps => 20,
                Drill::Pull => 3,
            };
            let size = h * 0.4;
            let x = (r[0] + r[2]) * 0.5;
            v.push((icon, [x - size * 0.5, r[1] - size * 0.6, x + size * 0.5, r[1] - size * 0.6 + size], false, p.learner));
        }
        // "let's play this next": held up over the head
        for c in self.chats.iter().filter(|c| c.stage == Stage::Talk && c.leavers.is_empty()) {
            let Some((p, game, t)) = c.offer else { continue };
            let r = if p == 0 { self.hit_rect } else if let Some(e) = self.extras.get(p - 1) { e.hit_rect } else { continue };
            let h = r[3] - r[1];
            if h <= 0.0 {
                continue;
            }
            let size = h * 0.42;
            let x = (r[0] + r[2]) * 0.5;
            let top = r[1] - size * 0.55 + (t * 9.0).sin() * h * 0.03;
            v.push((Self::game_icon(game), [x - size * 0.5, top, x + size * 0.5, top + size], false, p));
        }
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
                (1, _) => v.push((1, hand(giver, dir), false, g.giver)),
                (2, _) => v.push((1, hand(taker, -dir), false, g.giver)),
                (3, GiftKind::Carrot) if g.t < tn().carrot_eat => {
                    let r = hand(taker, -dir);
                    let k = (1.0 - g.t / tn().carrot_eat.max(0.1)).clamp(0.15, 1.0) * 0.5;
                    let (cx, cy) = ((r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5);
                    let (hw, hh) = ((r[2] - r[0]) * k, (r[3] - r[1]) * k);
                    v.push((0, [cx - hw, cy - hh, cx + hw, cy + hh], false, g.giver));
                }
                (3, GiftKind::Balloon) => {
                    let w = height * 0.32;
                    let x = middle(taker);
                    v.push((2, [x - w, taker[1] - height * 0.72, x + w, taker[1] + height * 0.08], false, g.giver));
                }
                (3, GiftKind::Prank) => {
                    let view = if g.giver == 0 { self.own_view } else { self.extras[g.giver - 1].own_view };
                    let x = g.box_x - view[0] * self.view_w * 0.5;
                    v.push((1, [x - size * 0.5, g.box_y, x + size * 0.5, g.box_y + size], false, g.giver));
                    // shoots out at once, then quivers on its spring
                    let out = height * (0.75 + 0.06 * (g.t * 40.0).sin()) * (g.t / 0.08).min(1.0);
                    let face = taker[1] + height * 0.3;
                    let (a, b) = (x, x + dir * out);
                    let half = height * 0.17;
                    let top = g.box_y.min(face) + (face - g.box_y).abs() * 0.5 - half;
                    v.push((3, [a.min(b), top, a.max(b), top + half * 2.0], dir < 0.0, g.giver));
                }
                _ => {}
            }
        }
        // fishing: rods, lines and whatever came up
        for c in self.chats.iter().filter(|c| c.stage == Stage::Fishing) {
            let Some(f) = c.fishing.as_ref() else { continue };
            let time = self.start.elapsed().as_secs_f32();
            if let (Some((x, t, stay)), Some(&owner)) = (f.beast, c.members.first()) {
                let (r, o, own) = if owner == 0 {
                    (self.hit_rect, self.opaque, self.own_view)
                } else {
                    let e = &self.extras[owner - 1];
                    (e.hit_rect, e.opaque, e.own_view)
                };
                let h = r[3] - r[1];
                let floor = r[3] - h * (1.0 - o[2]);
                // rises, sways a little, sinks back
                let k = (t / 0.35).min(1.0).min(((stay - t) / 0.35).max(0.0));
                let size = h * 1.25;
                let sx = x - own[0] * self.view_w * 0.5 + (time * 7.0).sin() * size * 0.04;
                if h > 0.0 && k > 0.0 {
                    if super::rope::prop_image(21).is_some() {
                        // a picture: full size, rising out of the ground, shown above it only
                        v.push((21000 + (k * 1000.0) as i32, [sx - size * 0.5, floor - size * k, sx + size * 0.5, floor + size * (1.0 - k)], false, owner));
                    } else {
                        v.push((21, [sx - size * 0.5, floor - size * k, sx + size * 0.5, floor], false, owner));
                    }
                }
            }
            for a in f.anglers.iter().filter(|a| (1..=5).contains(&a.phase) && !a.blown) {
                // the rod is put down while it jumps or laughs
                let rod_out = self.walk_ref(a.i).giggle <= 0.0 && matches!(self.walk_ref(a.i).act, Act::Idle | Act::Walk);
                let (r, o) = if a.i == 0 { (self.hit_rect, self.opaque) } else if let Some(e) = self.extras.get(a.i - 1) { (e.hit_rect, e.opaque) } else { continue };
                let h = r[3] - r[1];
                if h <= 0.0 {
                    continue;
                }
                let side = if self.walk_ref(a.i).left { -1.0 } else { 1.0 };
                let body = (r[2] - r[0]) * o[0];
                let hand = [(r[0] + r[2]) * 0.5 + side * body * 0.7, r[1] + h * 0.58];
                // the rod: up and out from the hand, raised higher while casting or reeling
                let lift = match a.phase {
                    1 => (a.t / 0.6).min(1.0) * 0.25,
                    4 => 0.3,
                    _ => 0.0,
                };
                let len = h * 0.55;
                let tip = [hand[0] + side * len * 0.85, hand[1] - len * (0.55 + lift)];
                let (x0, x1) = (hand[0].min(tip[0]), hand[0].max(tip[0]));
                if rod_out {
                    v.push((13, [x0, tip[1], x1, hand[1]], side < 0.0, a.i));
                }
                // the line down to the floor, the float jumping when something bites
                let floor = r[1] + h * o[2];
                let bob = if a.phase == 3 { (time * 30.0).sin() * h * 0.03 + h * 0.03 } else { (time * 2.0).sin() * h * 0.006 };
                if a.phase <= 3 && rod_out {
                    let w = h * 0.13;
                    v.push((14, [tip[0] - w * 0.5, tip[1], tip[0] + w * 0.5, floor + bob], false, a.i));
                }
                let size = h * 0.3;
                match a.phase {
                    4 => {
                        // up it comes, in an arc to the hand
                        let k = (a.t / tn().fish_reel.max(0.1)).min(1.0);
                        let x = tip[0] + (hand[0] - tip[0]) * k;
                        let y = floor + (hand[1] - floor) * k - (k * std::f32::consts::PI).sin() * h * 0.6;
                        v.push((a.item, [x - size * 0.5, y - size * 0.5, x + size * 0.5, y + size * 0.5], side < 0.0, a.i));
                    }
                    5 => {
                        let y = r[1] + h * 0.35;
                        let x = hand[0];
                        v.push((a.item, [x - size * 0.5, y - size * 0.5, x + size * 0.5, y + size * 0.5], side < 0.0, a.i));
                    }
                    _ => {}
                }
            }
        }
        let time = self.start.elapsed().as_secs_f32();
        // tag: a little arrow bobbing over whoever is "it"
        for c in self.chats.iter().filter(|c| c.stage == Stage::Tag && c.step != 9) {
            let r = if c.it == 0 { self.hit_rect } else if let Some(e) = self.extras.get(c.it - 1) { e.hit_rect } else { continue };
            let height = r[3] - r[1];
            if height <= 0.0 {
                continue;
            }
            let opaque = if c.it == 0 { self.opaque } else { self.extras[c.it - 1].opaque };
            let size = height * 0.16;
            let x = (r[0] + r[2]) * 0.5;
            let head = r[1] + height * opaque[1];
            let y = head - size * (0.9 + 0.25 * (time * 6.0).sin());
            v.push((7, [x - size * 0.5, y - size * 0.5, x + size * 0.5, y + size * 0.5], false, c.it));
        }
        for c in &self.chats {
            let Some(b) = c.bribe.filter(|b| b.phase == 2) else { continue };
            let rect = |i: usize| if i == 0 { self.hit_rect } else { self.extras[i - 1].hit_rect };
            let (g, t) = (rect(b.giver), rect(b.taker));
            let height = (t[3] - t[1]).max(1.0);
            let size = height * 0.36;
            let middle = |r: [f32; 4]| (r[0] + r[2]) * 0.5;
            let dir = if middle(t) >= middle(g) { 1.0 } else { -1.0 };
            let (r, side) = if b.t < 0.9 { (g, dir) } else { (t, -dir) };
            let x = middle(r) + side * height * 0.3;
            let top = r[1] + height * 0.55 - size * 0.5;
            v.push((0, [x - size * 0.5, top, x + size * 0.5, top + size], false, b.giver));
        }
        self.snow_props(&mut v);
        self.hide_props(&mut v);
        self.mood_props(&mut v);
        // a picture in place of a drawn thing fills the box's width at its own shape
        for p in &mut v {
            if (21000..FOUND).contains(&p.0) || p.0 >= FISH {
                continue;
            }
            if let Some(a) = super::rope::prop_image(p.0).and_then(|f| super::rope::picture_aspect(&f)) {
                p.1 = super::rope::picture_box(p.1, a);
            }
        }
        v
    }

    /// The carrot on the screen, surface pixels.
    fn carrot_rect(&self, c: &Chat) -> Option<[f32; 4]> {
        let v0 = c.carrot.as_ref()?;
        let shrink = if v0.eating.is_some() { (v0.eat / tn().carrot_eat.max(0.1)).clamp(0.15, 1.0) } else { 1.0 };
        let v = c.carrot.as_ref()?;
        let first = *c.members.first()?;
        let rect = |i: usize| if i == 0 { self.hit_rect } else { self.extras[i - 1].hit_rect };
        let body = rect(first);
        let size = (body[3] - body[1]).max(1.0) * 0.42 * shrink;
        let half = size * 0.42;
        let (x, top) = match v.eating.or(v.holder) {
            // in its hand, on the side it's running towards
            Some(h) => {
                let r = rect(h);
                let reach = if h == 0 { self.opaque[0] } else { self.extras[h - 1].opaque[0] } * (r[2] - r[0]);
                let side = if self.walk_ref(h).left { -1.0 } else { 1.0 };
                ((r[0] + r[2]) * 0.5 + side * reach * 0.8, r[1] + (r[3] - r[1]) * 0.55 - size * 0.5)
            }
            None => {
                let own = if first == 0 { self.own_view } else { self.extras[first - 1].own_view };
                (v.x - own[0] * self.view_w * 0.5, body[3] - size)
            }
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
        let thief_t = if self.walk.rand() < tn().carrot_thief_chance { 6.0 + self.walk.rand() * 8.0 } else { -1.0 };
        c.carrot = Some(Carrot { thief_t, thief: None, eating: None, eat: 0.0, pass_cool: 0.0, x: lo, lo, hi, holder: None, started: false, burst: 0.0, sprint: 0.0, tired: 0.0, sprint_cool: 0.0, hop_cool: 0.0, grace: 0.0, smart: None, smart_left: 0.0, smart_wait: tn().carrot_smart_every, dir: 1.0, speed: 0.0, decide: 0.0 });
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
                self.clear_happy(&c.members);
                v.x = v.lo + 110.0 + self.walk.rand() * (v.hi - v.lo - 220.0).max(0.0);
                log::debug!("chat {}: a carrot at {:.0}", c.id, v.x);
                c.step = 0;
                c.timer = 30.0;
            } else if c.timer <= 0.0 {
                self.end_game(c);
                return;
            } else {
                self.chatter(c, dt);
            }
            c.carrot = Some(v);
            return;
        }
        if c.members.is_empty() {
            return;
        }
        let size = self.row_size(c);
        if let Some(t) = v.eating {
            // munch munch, bits flying; everyone else just watches it go
            let before = v.eat;
            v.eat -= dt;
            let bite = 0.35;
            if (before / bite).floor() != (v.eat / bite).floor() {
                let count = 2 + (self.walk.rand() * 3.0) as usize;
                self.spray_crumbs(t, count);
            }
            if v.eat <= 0.0 || !c.watchers.contains(&t) {
                log::debug!("chat {}: {t} ate the carrot", c.id);
                let count = 3 + (self.walk.rand() * 2.0) as usize;
                self.smear_crumbs(t, count);
                v.eating = None;
                c.scores.clear();
                c.carrot = Some(v);
                self.end_game(c);
                return;
            }
            c.carrot = Some(v);
            return;
        }
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
            let carrot_y = self.place_here(c.members[0]).map(|_| self.walk_ref(c.members[0]).park_y.clamp(0.06, super::park::MEADOW * 0.85));
            for k in 0..c.members.len() {
                let i = c.members[k];
                let pace = self.tag_pace(i, dt);
                self.run_to(i, v.x, tn().carrot_rush * pace);
                if k > 0 {
                    if let Some(cy) = carrot_y {
                        if self.place_here(i).is_some() {
                            self.walk_of(i).park_target_y = cy;
                        }
                    }
                }
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
        if self.carrot_thief(c, &mut v, h, size, dt) {
            c.carrot = Some(v);
            return;
        }
        v.grace -= dt;
        v.hop_cool -= dt;
        v.sprint_cool -= dt;
        v.pass_cool -= dt;
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
        // about to be caught: slipped to a friend close by instead
        if let Some((n, _)) = catcher.filter(|t| v.grace <= 0.0 && v.pass_cool <= 0.0 && t.1 < size * 0.6) {
            v.pass_cool = 2.5;
            let friend = c
                .members
                .iter()
                .copied()
                .filter(|&f| f != h && f != n && grounded(self.walk_ref(f)) && (self.strip_px(f) - hx).abs() < size * 2.2)
                .max_by(|a, b| self.bond(h, *a).0.total_cmp(&self.bond(h, *b).0));
            if let Some(f) = friend.filter(|_| self.walk.rand() < tn().carrot_pass_chance) {
                log::debug!("chat {}: {h} slips the carrot to {f}", c.id);
                v.holder = Some(f);
                v.grace = tn().carrot_grace;
                v.burst = 0.0;
                v.sprint = 0.0;
                v.tired = 0.0;
                v.smart = None;
                v.decide = 0.0;
                v.speed = tn().carrot_run;
                self.walk_of(h).start_bob(tn().jump_height * 0.2);
                c.carrot = Some(v);
                return;
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
            self.pick_park_y(h, None);
        } else if v.decide <= 0.0 {
            let score = |d: f32| room(d).min(clear(d));
            v.dir = if score(1.0) >= score(-1.0) { 1.0 } else { -1.0 };
            v.decide = 0.6 + self.walk.rand() * 0.6;
            self.pick_park_y(h, None);
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
        let knack = self.skilled(h, "carrot grab", 0.85, 1.15);
        self.run_to(h, (hx + v.dir * 600.0).clamp(v.lo, v.hi), v.speed * knack);
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
        let hy = self.walk_ref(h).park_y.clamp(0.06, super::park::MEADOW * 0.85);
        for &i in &others {
            let pace = self.tag_pace(i, dt);
            if v.smart == Some(i) {
                let ahead = (hx + v.dir * 80.0).clamp(v.lo, v.hi);
                self.run_to(i, ahead, tn().carrot_chase * tn().carrot_smart_speed);
                if self.place_here(i).is_some() {
                    self.walk_of(i).park_target_y = hy;
                }
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
            self.pick_park_y(i, Some((hy, 0.24)));
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
                    // eaten first, bits flying, then they both hop about
                    let eat = tn().carrot_eat;
                    let before = g.t;
                    g.t += dt;
                    if g.t < eat {
                        let bite = 0.35;
                        if (before / bite).floor() != (g.t / bite).floor() {
                            let count = 1 + (self.walk.rand() * 3.0) as usize;
                            self.spray_crumbs(taker, count);
                        }
                        c.gift = Some(g);
                        return;
                    }
                    if before < eat {
                        let count = 3 + (self.walk.rand() * 2.0) as usize;
                        self.smear_crumbs(taker, count);
                    }
                    for i in [giver, taker] {
                        let wk = self.walk_of(i);
                        if grounded(wk) && wk.rand() < dt * 3.0 {
                            let height = tn().jump_height * (0.4 + wk.rand() * 0.4);
                            wk.start_jump(height);
                        }
                    }
                    if g.t >= eat + 2.5 {
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
        c.offset = if mean < self.strip_width_of(c.members[0]) * 0.5 { 1.0 } else { -1.0 };
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
                wk.ws == self.walk_ref(c.members[0]).ws && grounded(wk) && wk.crowd.is_none() && wk.social.is_none() && wk.race.is_none() && !wk.circle && wk.energy >= tn().energy_join_need && chat_cool(wk) <= 0.0
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
        self.match_park_y(i, &c.members);
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
                let left = c.turn < 0.0;
                for k in 0..c.members.len() {
                    if let Some(r) = self.walk_of(c.members[k]).social.as_mut() {
                        r.happy = true;
                        r.face_lock = true;
                        r.face_left = left;
                    }
                }
            } else if c.timer <= 0.0 {
                self.end_game(c);
            } else {
                self.chatter(c, dt);
            }
            return;
        }
        if c.timer <= 0.0 {
            self.end_game(c);
            return;
        }
        c.tag_free -= dt;
        // put down somewhere else: runs back to its place in the row
        // (the strip is at least as wide as where they stand: a park window can
        // sit further along than the workspace's own width says)
        let furthest = c.base.iter().map(|b| b.1).fold(0.0, f32::max);
        let width = self.strip_width_of(c.members[0]).max(furthest + 300.0);
        // dancing in a window (the park): the row stays inside it
        let (floor, ceil) = c
            .members
            .iter()
            .find_map(|&i| self.place_here(i))
            .map_or((0.0, width), |p| (p.x, (p.x + p.w).max(p.x + 1.0)));
        let far = (tn().samba_step * 1.6).max(30.0);
        let slot = |x: f32, marched: f32| (x + marched).clamp(floor, ceil);
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
        let edge = 120.0f32.min((ceil - floor) * 0.25);
        let step = c.offset * tn().samba_step;
        let lo = c.base.iter().map(|b| b.1).fold(f32::MAX, f32::min);
        let hi = c.base.iter().map(|b| b.1).fold(f32::MIN, f32::max);
        if lo + c.pull + step < floor + edge || hi + c.pull + step > ceil - edge {
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
            if matches!(wk.act, Act::Held | Act::Fall) {
                continue;
            }
            if grounded(wk) || (wk.act == Act::Jump && wk.jump <= 0.12) {
                wk.start_bob(tn().jump_height * 0.12);
            }
            wk.left = left;
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

    /// Where a race or a game is held on the desktop: the part of the strip
    /// on screen now. (left edge, width) on the strip, logical px.
    pub(super) fn biggest_column(&self) -> (f32, f32) {
        let w = self.view_w;
        (self.view[0] * w * 0.5, w)
    }

    /// `biggest_column`, at least half a screen long.
    fn big_window(&self) -> (f32, f32) {
        // a game in the park: the park is the playing field
        if let Some(area) = GAME_AREA.with(|a| a.get()) {
            return area;
        }
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
                    self.pick_park_y(i, None);
                }
            }
            if !waiting {
                self.clear_happy(&c.members);
                c.step = 0;
                c.wait = tn().tag_wait;
                c.offset = 0.0;
                c.timer = tn().tag_time_min + self.walk.rand() * tn().tag_time_rand;
                log::debug!("chat {}: everyone's in the window, {} is it", c.id, c.it);
            } else if c.timer <= 0.0 {
                self.end_game(c);
            } else {
                self.chatter(c, dt);
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
            let knack = self.skilled(i, "tag", 0.85, 1.15);
            self.run_to(i, goal.clamp(lo, hi), tn().tag_run * pace * knack);
            self.pick_park_y(i, None);
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
        let knack = self.skilled(c.it, "tag", 0.85, 1.15);
        self.run_to(c.it, if held { ahead.clamp(lo, hi) } else { ahead }, tn().tag_it * pace * knack);
        if self.place_here(c.it).is_some() {
            let cy = self.walk_ref(c.chase).park_y.clamp(0.06, super::park::MEADOW * 0.85);
            self.walk_of(c.it).park_target_y = cy;
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
        other.encore_due = c.encore_due;
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
        let width = self.strip_width_of(c.members[0]);
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
        c.bribe = None;
        if !turns.is_empty() && self.walk.rand() < tn().bribe_chance {
            let giver = turns[(self.walk.rand() * turns.len() as f32) as usize % turns.len()].flyer;
            c.bribe = Some(Bribe { giver, taker: referee, phase: 0, t: 0.0, home: 0.0 });
        }
        c.long = Some(Long { nudges: Vec::new(), fixed: Vec::new(), referee, line, dir, ready: false, turns, next: 0, gap: 0.0, running: Vec::new(), landed: Vec::new(), judging: None, thrown: 0.0 });
        c.scores.clear();
        c.regroup = true;
        c.timer = tn().long_time;
        c.stage = Stage::Long;
    }

    /// Off they go, very far, tumbling.
    fn launch(&mut self, i: usize, dir: f32) {
        // beginners barely get off the ground
        let knack = self.skilled(i, "long jump", 0.4, 1.15);
        let wk = self.walk_of(i);
        let distance = (tn().long_dist_min + wk.rand() * tn().long_dist_rand) * knack;
        let time = tn().long_air_min + wk.rand() * tn().long_air_rand;
        self.fling(i, dir, distance, time);
        if let Some(r) = self.walk_of(i).social.as_mut() {
            r.happy = true;
        }
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
            // a loser thrown out isn't pleased about it; who flies for fun says so itself
            r.happy = false;
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
            if l.ready {
                self.clear_happy(&c.members);
            } else {
                self.chatter(c, dt);
            }
            c.long = Some(l);
            return;
        }
        if self.run_bribe(c, dt) {
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
        let mut fresh = Vec::new();
        for &i in &c.members {
            if let Some(r) = self.walk_of(i).social.as_mut() {
                if r.stuck && !r.queued {
                    r.queued = true;
                    fresh.push(i);
                }
            }
        }
        for i in fresh {
            if !self.start_nudge(c, &mut l, i) {
                l.landed.push(i);
            }
        }
        self.run_nudges(c, &mut l, dt);
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
                        let at = l.fixed.iter().find(|f| f.0 == i).map_or(self.strip_px(i), |f| f.1);
                        let bribed = c.bribe.is_some_and(|b| b.giver == i && b.phase == 3);
                        let distance = ((at - from) * l.dir).max(1.0) * if bribed { 1.0 + tn().long_bribe_bonus } else { 1.0 };
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
    /// The name a game goes by (for skills and the status window).
    fn game_name(stage: Stage) -> &'static str {
        match stage {
            Stage::Contest => "jumping contest",
            Stage::Cursor => "cursor game",
            Stage::Long => "long jump",
            Stage::Domino => "domino",
            Stage::Tug => "tug of war",
            Stage::Wrestle => "wrestling",
            Stage::Tag => "tag",
            Stage::Carrot => "carrot grab",
            Stage::Gift => "gift",
            Stage::Samba => "samba",
            Stage::Volley => "volleyball",
            Stage::Fishing => "fishing",
            Stage::Snow => "snowball fight",
            Stage::Hide => "hide and seek",
            Stage::Concert => "concert",
            _ => "",
        }
    }

    fn game_has_reward(stage: Stage, step: u8) -> bool {
        step != 9 && !Self::game_name(stage).is_empty() && !matches!(stage, Stage::Gift | Stage::Concert)
    }

    fn pick_random_from(pool: &[usize], roll: f32, has_unfound: impl Fn(usize) -> bool) -> Option<usize> {
        if pool.is_empty() {
            return None;
        }
        let unfound: Vec<usize> = pool.iter().copied().filter(|&i| has_unfound(i)).collect();
        let choices = if unfound.is_empty() { pool } else { unfound.as_slice() };
        Some(choices[(roll.clamp(0.0, 0.999999) * choices.len() as f32) as usize % choices.len()])
    }

    fn pick_top_winner(members: &[usize], scores: &[(usize, f32)], roll: f32, has_unfound: impl Fn(usize) -> bool) -> Option<usize> {
        let best = scores.iter().filter(|s| s.1 > 0.0 && members.contains(&s.0)).map(|s| s.1).reduce(f32::max)?;
        let tied: Vec<usize> = scores
            .iter()
            .filter(|s| s.1 >= best - 1e-4 && members.contains(&s.0))
            .map(|s| s.0)
            .collect();
        Self::pick_random_from(&tied, roll, has_unfound)
    }

    fn has_unfound(&self, i: usize) -> bool {
        let name = self.character_name(i);
        super::wear::unfound(&name, 0.0).is_some()
    }

    /// Gives `who` an unlocked accessory, celebrates with confetti and a jump,
    /// and shows the found item bobbing over its head.
    pub(super) fn award_accessory(&mut self, who: usize) -> Option<usize> {
        let name = self.character_name(who);
        let roll = self.walk.rand();
        let k = super::wear::unfound(&name, roll)?;
        let (id, label, _, _) = super::wear::catalog_entry(k)?;
        super::wear::give(&name, &id);
        log::info!("accessory reward: {name} won {label} ({id})");
        let r = if who == 0 { self.hit_rect } else if let Some(e) = self.extras.get(who - 1) { e.hit_rect } else { [0.0; 4] };
        let x = self.strip_px(who);
        self.moods.effects.push((FOUND + k as i32, who, x, r[1], 0.0, 4.5));
        self.confetti(who);
        let wk = self.walk_of(who);
        if grounded(wk) {
            wk.start_jump(tn().jump_height * 1.4);
        }
        let secs = tn().laugh_time * 1.5;
        wk.laugh(secs);
        if let Some(s) = wk.social.as_mut() {
            s.happy = true;
        }
        Some(k)
    }

    fn maybe_reward_game(&mut self, c: &mut Chat, fixed: Option<usize>) {
        if c.rewarded || !Self::game_has_reward(c.stage, c.step) {
            return;
        }
        if self.walk.rand() >= tn().game_reward {
            return;
        }
        let who = match fixed {
            Some(w) => Some(w),
            None => {
                let roll = self.walk.rand();
                Self::pick_random_from(&c.members, roll, |i| self.has_unfound(i))
            }
        };
        if let Some(w) = who {
            c.rewarded = true;
            self.award_accessory(w);
        }
    }

    fn end_game(&mut self, c: &mut Chat) {
        // fishing's over: whoever is up a jetty walks down and is done
        if c.stage == Stage::Fishing {
            for &i in &c.members {
                let wk = self.walk_of(i);
                wk.fish_cool = wk.fish_cool.max(tn().fish_again);
            }
            for i in c.members.clone() {
                if self.walk_ref(i).air > 0.05 {
                    c.members.retain(|&m| m != i);
                    self.leave(i, c.id);
                    self.push_down(i);
                }
            }
            if c.members.is_empty() {
                c.timer = 0.0;
                c.stage = Stage::Part;
                return;
            }
        }
        let game = Self::game_name(c.stage);
        if !game.is_empty() && c.stage != Stage::Gift && c.stage != Stage::Concert {
            let members = c.members.clone();
            self.practice(&members, game);
        }
        for &i in &c.members.clone() {
            // stop here: the game's last goals mean nothing any more, and who
            // looks at whom is worked out from where everyone stands
            let here = self.strip_px(i);
            // went flying or got stuck in the ground: laughs it off first
            let tumbled = self.walk_ref(i).social.is_some_and(|r| r.stuck || r.flying);
            // on the losing side of a game with winners: gets up without the laughing hops
            let lost = !c.scores.is_empty() && !c.scores.iter().any(|s| s.0 == i && s.1 > 0.0);
            if tumbled && lost {
                self.walk_of(i).good_sport();
            }
            if tumbled && !lost {
                let wk = self.walk_of(i);
                let secs = tn().laugh_time * (0.8 + wk.rand() * 0.7);
                wk.laugh(secs);
            }
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
            self.maybe_reward_game(c, None);
            return;
        }
        let roll = self.walk.rand();
        c.winner = Self::pick_top_winner(&c.members, &c.scores, roll, |i| self.has_unfound(i));
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
            self.maybe_reward_game(c, Some(w));
        } else {
            self.maybe_reward_game(c, None);
            self.talk_it_over(c);
        }
    }

    /// A game's not over till it's been talked about.
    fn talk_it_over(&mut self, c: &mut Chat) {
        c.encore_due = true;
        // the cheering is over
        for &i in &c.members.clone() {
            if let Some(r) = self.walk_of(i).social.as_mut() {
                r.happy = false;
            }
        }
        if matches!(c.stage, Stage::Samba | Stage::Concert) {
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

    /// After a game some have had enough and drift off; the lively and the
    /// keen stay, and the liveliest holds up what to play next.
    fn plan_encore(&mut self, c: &mut Chat) {
        // enough games for one group: they talk, then go their ways
        if self.race.is_some() || c.members.len() < 2 || c.games as f32 >= tn().max_games {
            return;
        }
        let (mut keen, mut tired) = (Vec::new(), Vec::new());
        for &i in &c.members.clone() {
            // energy counts most, but it's a mood too: a tired one may still fancy it
            let want = (tn().encore_keen + self.walk_ref(i).energy * tn().encore_energy).clamp(0.05, 0.95);
            if self.walk_of(i).rand() < want { keen.push(i) } else { tired.push(i) }
        }
        if keen.len() < 2 {
            log::debug!("chat {}: nobody much wants another game ({keen:?} of {})", c.id, c.members.len());
            return;
        }
        let rivalry = self.rivalry(&keen);
        let Some(game) = self.choose_game(&c.played, keen.len(), rivalry) else {
            log::debug!("chat {}: {keen:?} would play on, but there's nothing new for {}", c.id, keen.len());
            return;
        };
        let proposer = keen.iter().copied().max_by(|&a, &b| self.walk_ref(a).energy.total_cmp(&self.walk_ref(b).energy)).unwrap_or(keen[0]);
        log::debug!("chat {}: {proposer} wants {game:?} next, {keen:?} stay, {tired:?} have had enough", c.id);
        let mut last = 0.0f32;
        c.leavers.clear();
        for i in tired {
            let delay = 1.0 + self.walk.rand() * 3.0;
            last = last.max(delay);
            c.leavers.push((i, delay));
        }
        c.offer = Some((proposer, game, tn().encore_show));
        c.timer = c.timer.max(last + tn().encore_show + 2.0);
    }

    /// The talk after a game: the tired go one by one, then the proposer
    /// shows what's next and off they go. True once the next game starts.
    fn run_encore(&mut self, c: &mut Chat, dt: f32) -> bool {
        let mut going = Vec::new();
        c.leavers.retain_mut(|l| {
            l.1 -= dt;
            if l.1 <= 0.0 {
                going.push(l.0);
            }
            l.1 > 0.0
        });
        for i in going {
            if c.members.len() <= 2 || !c.members.contains(&i) {
                continue;
            }
            log::debug!("chat {}: {i} has had enough and goes", c.id);
            c.members.retain(|&m| m != i);
            self.leave(i, c.id);
            self.walk_of(i).start_bob(tn().jump_height * 0.2);
        }
        let Some((p, game, t)) = c.offer else { return false };
        if !c.members.contains(&p) || c.members.len() < 2 {
            c.offer = None;
            return false;
        }
        if !c.leavers.is_empty() {
            return false;
        }
        if t == tn().encore_show {
            // "this one!": a hop, and everyone looks
            c.talker = p;
            self.look_at_talker(c);
            let wk = self.walk_of(p);
            wk.gesture = true;
            wk.start_bob(tn().jump_height * 0.35);
        }
        if let Some(r) = self.walk_of(p).social.as_mut() {
            r.happy = true;
        }
        let t = t - dt;
        if t > 0.0 {
            c.offer = Some((p, game, t));
            c.timer = c.timer.max(1.0);
            return false;
        }
        c.offer = None;
        for (a, b) in Self::each_pair(&c.members) {
            self.add_bond(a, b, 0.0, tn().bond_game);
        }
        c.played.push(game);
        c.games += 1;
        c.quiet = 0.0;
        for &i in &c.members.clone() {
            self.spend(i, tn().energy_game_cost);
            self.walk_of(i).played += 1.0;
        }
        self.start_game(c, game);
        true
    }

    /// What the proposer holds up for each game (a prop kind).
    fn game_icon(game: Game) -> i32 {
        match game {
            Game::Carrot => 0,
            Game::Gift => 1,
            Game::Volley => 2,
            Game::Tag => 7,
            Game::Fishing => 13,
            Game::Snow => snow::SNOWBALL,
            Game::Hide => 26,
            Game::Tug | Game::Wrestle => 3,
            _ => 15,
        }
    }

    /// And their own ways, not all at once: some stay put, friends go off
    /// together, most head away from the group but some cut across it or
    /// make for another group.
    fn part(&mut self, c: &mut Chat) {
        log::debug!("chat {}: part", c.id);
        let width = self.strip_width_of(c.members[0]);
        let (lo, hi) = (tn().edge_px.min(width * 0.5), (width - tn().edge_px).max(width * 0.5));
        let mean = c.members.iter().map(|&i| self.strip_px(i)).sum::<f32>() / c.members.len() as f32;
        let ws = self.walk_ref(c.members[0]).ws;
        // where other groups on this workspace are standing
        let others: Vec<f32> = self
            .chats
            .iter()
            .filter(|o| o.id != c.id && !o.members.is_empty() && self.walk_ref(o.members[0]).ws == ws)
            .map(|o| o.members.iter().map(|&i| self.strip_px(i)).sum::<f32>() / o.members.len() as f32)
            .collect();
        c.part_queue.clear();
        let mut left: Vec<usize> = c.members.clone();
        while let Some(i) = left.pop() {
            let x = self.strip_px(i);
            let delay = self.walk.rand() * tn().part_stagger;
            if self.walk.rand() < tn().part_stay {
                // stays where it is, maybe shuffles a step
                let step = (self.walk.rand() - 0.5) * self.width_of(i);
                c.part_queue.push((i, delay, (x + step).clamp(lo, hi), 0.08));
                continue;
            }
            let roll = self.walk.rand();
            let away = if (x - mean).abs() < 1.0 { if self.walk.rand() < 0.5 { -1.0 } else { 1.0 } } else { (x - mean).signum() };
            let dist = 300.0 + self.walk.rand() * 500.0;
            let goal = if roll < tn().part_other && !others.is_empty() {
                // off to see another group
                let k = (self.walk.rand() * others.len() as f32) as usize % others.len();
                let size = self.width_of(i);
                others[k] + (self.walk.rand() - 0.5) * size * 2.0
            } else if roll < tn().part_other + tn().part_cross {
                // right across the group, the other way
                x - away * dist
            } else {
                x + away * dist
            }
            .clamp(lo, hi);
            let speed = 0.09 + self.walk.rand() * 0.07;
            c.part_queue.push((i, delay, goal, speed));
            // a friend close by goes along with it, side by side
            let buddy = left
                .iter()
                .copied()
                .filter(|&f| (self.strip_px(f) - x).abs() < self.width_of(i) * 3.0)
                .max_by(|a, b| self.bond(i, *a).0.total_cmp(&self.bond(i, *b).0));
            if let Some(f) = buddy.filter(|&f| self.bond(i, f).0 >= tn().bond_friend || self.walk.rand() < tn().part_buddy) {
                left.retain(|&o| o != f);
                let dir = (goal - x).signum();
                let beside = (goal - dir * self.width_of(f) * 0.8).clamp(lo, hi);
                c.part_queue.push((f, delay + 0.2, beside, speed));
            }
        }
        c.stage = Stage::Part;
    }

    /// Sends each one off when its moment comes.
    fn part_step(&mut self, c: &mut Chat, dt: f32) {
        let mut go = Vec::new();
        c.part_queue.retain_mut(|p| {
            p.1 -= dt;
            if p.1 <= 0.0 {
                go.push(*p);
                false
            } else {
                true
            }
        });
        for (i, _, goal, speed) in go {
            if c.members.contains(&i) {
                self.go_facing(i, goal, speed);
            }
        }
    }

    /// Run one chat for a frame; false when it's over.
    fn advance(&mut self, c: &mut Chat, dt: f32, mouse: [f32; 2]) -> bool {
        c.timer -= dt;
        // one of a chat is in the hand or in the air: the talk waits, they chase it
        if matches!(c.stage, Stage::Approach | Stage::Talk | Stage::Stroll) {
            if let Some(&h) = c.members.iter().find(|&&i| self.walk_ref(i).social.is_some_and(|s| s.grabbed)) {
                c.timer += dt;
                self.chase_carried(c, h);
                return true;
            }
        }
        let paused = matches!(c.stage, Stage::Contest | Stage::Domino | Stage::Gift) && self.held_up(c, dt);
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
                } else {
                    self.chatter(c, dt);
                }
            }
            Stage::Talk => {
                c.quiet += dt;
                c.mood += dt;
                if (c.offer.is_some() || !c.leavers.is_empty()) && self.run_encore(c, dt) {
                    return true;
                }
                if c.offer.is_some() {
                    c.quiet = 0.0;
                }
                if c.members.len() >= tn().mood_min as usize && c.mood >= tn().mood_after && self.race.is_none() && self.social_age >= tn().warm_small {
                    c.mood = -60.0;
                    if self.mood_split(c) {
                        return true;
                    }
                }
                let pair_only = c.members.len() < tn().game_min as usize;
                let may_play = c.members.len() >= (tn().game_min as usize).min(tn().tug_min as usize)
                    && c.quiet >= tn().game_after
                    && (c.games == 0 || (c.members.len() >= tn().crowd_games as usize && (c.games as f32) < tn().max_games));
                if may_play {
                    // not every chat turns into a game; ask again a good while later
                    let keen = self.fewness(c.members[0]).min(1.6);
                    if self.walk.rand() >= tn().game_chance * keen * if pair_only { tn().tug_pair_chance } else { 1.0 } {
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
            Stage::Contest | Stage::Domino | Stage::Gift if paused => {}
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
                    0 if !self.all_arrived(c) => self.chatter(c, dt),
                    0 if c.wait <= 0.0 => {
                        for &m in &c.members {
                            if let Some(r) = self.walk_of(m).social.as_mut() {
                                r.happy = m == i;
                            }
                        }
                        c.talker = i;
                        self.look_at_talker(c);
                        c.wait = 0.35;
                        c.step = 1;
                    }
                    // up, as high as it can this time
                    1 if c.wait <= 0.0 && grounded(self.walk_ref(i)) => {
                        let knack = self.skilled(i, "jumping contest", 0.55, 1.2);
                        let height = (tn().contest_min + self.walk.rand() * tn().contest_rand) * knack;
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
                    let cursor_y = if let Some(p) = self.place_here(i) {
                        let rect = if i == 0 { self.hit_rect } else { self.extras[i - 1].hit_rect };
                        if rect[3] > rect[1] {
                            let opaque = if i == 0 { self.opaque } else { self.extras[i - 1].opaque };
                            let ch = rect[3] - rect[1];
                            let air_px = self.walk_ref(i).air * ch * 0.5;
                            let up_px = self.walk_ref(i).park_y * p.h;
                            let ground_y = rect[1] + ch * opaque[2] + air_px + up_px;
                            let lane = (k as f32 - (count - 1.0) * 0.5) * 0.05;
                            Some(((ground_y - mouse[1]) / p.h.max(1.0) + lane).clamp(0.06, super::park::MEADOW * 0.85))
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    let wk = self.walk_of(i);
                    if let Some(y) = cursor_y {
                        wk.park_target_y = y;
                    }
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
                    // spread out on the way, but right under the cursor once close
                    let near = (x - cursor).abs() < tn().cursor_reach * 3.0;
                    r.goal_px = if near { cursor } else { cursor + (k as f32 - (count - 1.0) * 0.5) * 30.0 };
                    r.speed = tn().cursor_speed;
                    r.moving = true;
                    r.face_left = cursor < x;
                    let jump = on_ground && r.hop_cool <= 0.0 && (x - cursor).abs() < tn().cursor_reach && reach > 0.0;
                    if jump {
                        r.hop_cool = 0.6 + roll * 0.8;
                        // it keeps after the cursor in the air
                        r.leaping = true;
                        // a bit higher than the head just touching it, so the
                        // hands get there on the way up, not only at the very top
                        wk.start_jump((reach + 0.35).max(tn().jump_height));
                        wk.catch_jump = true;
                    }
                }
            }
            Stage::Long => self.run_long(c, dt),
            Stage::Domino => self.run_domino(c, dt),
            Stage::Concert => {
                if c.timer <= 0.0 {
                    self.talk_it_over(c);
                    return true;
                }
                self.run_concert(c, dt);
            }
            Stage::Tug => self.run_tug(c, dt, mouse),
            Stage::Wrestle => self.run_wrestle(c, dt),
            Stage::Tag => self.run_tag(c, dt),
            Stage::Carrot => self.run_carrot(c, dt),
            Stage::Gift => self.run_gift(c, dt),
            Stage::Samba => self.run_samba(c, dt),
            Stage::Volley => self.run_volley(c, dt),
            Stage::Fishing => self.run_fishing(c, dt),
            Stage::Snow => self.run_snow(c, dt),
            Stage::Hide => self.run_hide(c, dt),
            Stage::Practice => return self.run_practice(c, dt),
            Stage::Play => return self.run_play(c, dt),
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
            Stage::Part => {
                self.part_step(c, dt);
                return !(c.part_queue.is_empty() && self.all_arrived(c));
            }
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

/// Computes `(lo, hi, mid)` bounds in strip pixels from `big_window()`'s `(x, cw)` width tuple.
fn wrestle_arena(x: f32, cw: f32, unit: f32) -> (f32, f32, f32) {
    let pad = (unit * 0.5).min(cw * 0.2);
    let lo = x + pad;
    let hi = (x + cw - pad).max(lo + unit);
    (lo, hi, (lo + hi) * 0.5)
}

/// Computes `(top_spots, crowd_spots)` in strip pixels for a post-race concert so
/// podium performers and cheering spectators fit inside `[size * 0.5, width - size * 0.5]`
/// without piling up on either edge.
fn concert_spots(finish: f32, width: f32, size: f32, top_len: usize, crowd_len: usize) -> (Vec<f32>, Vec<f32>) {
    let lo = size * 0.5;
    let hi = (width - size * 0.5).max(lo);
    let span = (hi - lo).max(0.0);
    let unit_reach = if crowd_len > 0 {
        2.6 + ((crowd_len - 1) / 2) as f32 * 0.9
    } else if top_len >= 2 {
        1.0
    } else {
        0.0
    };
    let gap = if unit_reach > 0.0 {
        (size * 0.8).min(span / (2.0 * unit_reach))
    } else {
        0.0
    };
    let margin = unit_reach * gap;
    let mid = (lo + hi) * 0.5;
    let centre = finish.clamp((lo + margin).min(mid), (hi - margin).max(mid));
    let podium = [centre, centre - gap, centre + gap];
    let top_spots = podium.into_iter().take(top_len).collect();
    let crowd_spots = (0..crowd_len)
        .map(|k| {
            let side = if k % 2 == 0 { -1.0 } else { 1.0 };
            let slot = (k / 2) as f32;
            centre + side * gap * (2.6 + slot * 0.9)
        })
        .collect();
    (top_spots, crowd_spots)
}

#[cfg(test)]
mod game_pick_tests {
    use super::*;

    #[test]
    fn concert_spots_keep_full_crowd_inside_strip_edges() {
        let size = 180.0;
        let width = 1920.0;
        let lo = size * 0.5;
        let hi = width - size * 0.5;

        for finish in [200.0, width - 200.0] {
            let (top, crowd) = concert_spots(finish, width, size, 3, 9);
            assert_eq!(top.len(), 3);
            assert_eq!(crowd.len(), 9);
            let mut all: Vec<f32> = top.into_iter().chain(crowd).collect();
            all.sort_by(f32::total_cmp);
            assert!(all[0] >= lo - 1e-3, "leftmost {} < {lo}", all[0]);
            assert!(*all.last().unwrap() <= hi + 1e-3, "rightmost {} > {hi}", all.last().unwrap());
            for pair in all.windows(2) {
                assert!(pair[1] - pair[0] > size * 0.5, "cramped pair {pair:?}");
            }
        }

        // Wide multi-column strip keeps the podium right at the finish line when there is room.
        let (top, _) = concert_spots(2400.0, 3840.0, size, 3, 6);
        assert!((top[0] - 2400.0).abs() < 1e-3);
    }

    #[test]
    fn wrestle_arena_uses_window_width_not_right_edge() {
        // Rightmost desktop window starting past its own width (e.g. x=1200, width=720)
        // previously collapsed lo == hi because `cw` was treated as `raw_hi`.
        let (lo, hi, mid) = wrestle_arena(1200.0, 720.0, 90.0);
        assert_eq!(lo, 1245.0);
        assert_eq!(hi, 1875.0);
        assert_eq!(mid, 1560.0);
    }

    #[test]
    fn volley_waits_for_large_groups_while_tag_and_carrot_stay_small() {
        let small = CharacterOverlay::game_pool(&[], 3, 0.0);
        assert!(small.iter().any(|(g, _)| *g == Game::Tag));
        assert!(small.iter().any(|(g, _)| *g == Game::Carrot));
        assert!(!small.iter().any(|(g, _)| *g == Game::Volley));

        let six = CharacterOverlay::game_pool(&[], 6, 0.0);
        assert!(!six.iter().any(|(g, _)| *g == Game::Volley));

        let seven = CharacterOverlay::game_pool(&[], 7, 0.0);
        let total: f32 = seven.iter().map(|(_, w)| w).sum();
        let volley = seven.iter().find(|(g, _)| *g == Game::Volley).map(|(_, w)| *w).unwrap_or(0.0);
        assert!(volley > 0.0 && volley / total < 0.25);
        assert!(seven.iter().any(|(g, _)| *g == Game::Wrestle));
        assert!(!small.iter().any(|(g, _)| *g == Game::Wrestle));
    }

    #[test]
    fn reward_picks_random_winning_teammate_or_participant_and_skips_unplayed() {
        assert!(CharacterOverlay::game_has_reward(Stage::Contest, 0));
        assert!(CharacterOverlay::game_has_reward(Stage::Wrestle, 0));
        assert!(CharacterOverlay::game_has_reward(Stage::Tag, 0));
        assert!(CharacterOverlay::game_has_reward(Stage::Samba, 0));
        assert!(!CharacterOverlay::game_has_reward(Stage::Volley, 9));
        assert!(!CharacterOverlay::game_has_reward(Stage::Wrestle, 9));
        assert!(!CharacterOverlay::game_has_reward(Stage::Gift, 0));
        assert!(!CharacterOverlay::game_has_reward(Stage::Concert, 0));
        assert!(!CharacterOverlay::game_has_reward(Stage::Practice, 0));

        // Single top scorer always wins even when already owning everything.
        let members = [0, 1, 2];
        let single = [(0, 1.0), (1, 2.5), (2, 1.5)];
        assert_eq!(CharacterOverlay::pick_top_winner(&members, &single, 0.1, |_| false), Some(1));

        // Team tie (e.g. Tug / Volley) picks randomly among winning teammates with unfound items.
        let team = [(0, 1.0), (1, 0.0), (2, 1.0), (3, 0.0), (4, 1.0)];
        assert_eq!(CharacterOverlay::pick_top_winner(&[0, 1, 2, 3, 4], &team, 0.0, |i| i != 0), Some(2));
        assert_eq!(CharacterOverlay::pick_top_winner(&[0, 1, 2, 3, 4], &team, 0.9, |i| i != 0), Some(4));

        // Winnerless group game picks one random participant, preferring unfound items.
        assert_eq!(CharacterOverlay::pick_random_from(&[10, 20, 30], 0.1, |i| i == 20 || i == 30), Some(20));
        assert_eq!(CharacterOverlay::pick_random_from(&[10, 20, 30], 0.9, |i| i == 20 || i == 30), Some(30));
    }
}
