//! How the walking characters get about on their own: strolling, idling,
//! hopping, running after the cursor or into a drawn circle.

use crate::render::tuning::get as tn;
use super::race::Racer;
use super::social::Social;

/// What the character is doing right now.
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub(super) enum Act {
    #[default]
    Idle,
    Walk,
    Jump,
    Chase,
    Pat,
    Held,
    Fall,
    // dropped from high up: stuck in the ground for a moment
    Planted,
    // going up the side of a window
    Climb,
}

/// Random behaviour: walk (likely), idle, jump (less likely) and, rarely,
/// run after the cursor and jump once it's caught.
/// `x` is the offset in clip units from the anchored position.
#[derive(Default)]
pub(super) struct WalkState {
    pub(super) x: f32,
    pub(super) target: f32,
    pub(super) left: bool,
    pub(super) seed: u32,
    pub(super) act: Act,
    pub(super) timer: f32,
    pub(super) jump: f32,
    // current jump: peak height in half-heights of the character, and length
    pub(super) jump_amp: f32,
    pub(super) jump_dur: f32,
    // picked to answer a circle gesture
    pub(super) circle: bool,
    // centre and radius of that circle (surface px): run there and jump into it
    pub(super) circle_target: Option<[f32; 3]>,
    // height above the ground while held/falling (half-heights), fall speed
    pub(super) air: f32,
    pub(super) vy: f32,
    // sideways speed after a throw (clip units/s)
    pub(super) vx: f32,
    // last held position, for the throw speed
    pub(super) held_prev: Option<(f32, f32)>,
    // where the cursor grabbed the sprite, relative to its centre (clip)
    pub(super) grab: Option<[f32; 2]>,
    // seconds until a jump queued by a notification
    pub(super) notify_jump: Option<f32>,
    // jumping at the cursor after a chase
    pub(super) catch_jump: bool,
    // hanging on to the cursor: seconds left
    pub(super) hang: f32,
    // dropped at a random spot on the first frame
    pub(super) placed: bool,
    pub(super) race: Option<Racer>,
    pub(super) social: Option<Social>,
    // seconds before it's up for another chat
    pub(super) social_cool: f32,
    // play a gesture now (3D characters)
    pub(super) gesture: bool,
    // social energy, see social.rs
    pub(super) energy: f32,
    // turned (radians) and sunk into the ground (half-heights)
    pub(super) rot: f32,
    // characters on the team
    pub(super) team: f32,
    // hanging on the cursor: swing angle, its speed, the cursor speed last frame
    pub(super) swing: f32,
    pub(super) swing_v: f32,
    pub(super) swing_last: Option<[f32; 2]>,
    pub(super) swing_vel: [f32; 2],
    // turns about the grip (until upright again)
    pub(super) grip: bool,
    // spinning while knocked through the air (radians/s, 0 when not)
    pub(super) tumble: f32,
    // landed spinning: rolling on upright the way it was going
    pub(super) rolling: bool,
    // knocked over in a race, until back on its feet
    pub(super) race_down: bool,
    // how it is stuck in the ground while planted
    pub(super) plant_pose: (f32, f32),
    // turned about the feet instead of the middle
    pub(super) feet: bool,
    pub(super) sink: f32,
    // seconds of no new chats after a race or a big gathering
    pub(super) crowd_rest: f32,
    // seconds since it landed from being carried or thrown, counting down
    pub(super) dropped: f32,
    // how far a jump must go to reach the cursor (half-heights), last frame
    pub(super) reach: f32,
    // on its way up, on top of, or down from a window
    pub(super) climb: Option<super::perch::Climb>,
    pub(super) climb_cool: f32,
    // jumped off a window on purpose: lands on its feet
    pub(super) leaped: bool,
    // lost its grip climbing: lands stuck in the ground
    pub(super) slipped: bool,
    // how many of the team are on or heading for windows
    pub(super) climbers: u32,
    // animation time not yet given to the 3D model (skipped frames)
    pub(super) model_dt: f32,
}

pub(super) const PAT_DUR: f32 = 1.4;

impl WalkState {
    pub(super) fn rand(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    pub(super) fn pick(&mut self, min: f32, max: f32, can_chase: bool) {
        let t = tn();
        // the four chances count relative to each other
        // the bigger the team, the fewer run after the cursor
        let chase = t.chase_chance * 2.0 / self.team.max(2.0);
        let r = self.rand() * (t.walk_chance + t.idle_chance + t.jump_chance + chase).max(0.001);
        if r < t.walk_chance {
            // short or long stroll in a random direction
            let d = self.rand();
            let dist = if d < t.short_share {
                t.short_min + self.rand() * t.short_rand
            } else if d < t.short_share + t.mid_share {
                t.mid_min + self.rand() * t.mid_rand
            } else {
                // a trip somewhere along niri's strip
                self.rand() * (max - min)
            };
            let dir = if self.rand() < 0.5 { -1.0 } else { 1.0 };
            self.target = self.stroll_target(dir * dist, min, max);
            self.act = Act::Walk;
        } else if r < t.walk_chance + t.idle_chance {
            self.act = Act::Idle;
            self.timer = t.idle_min + self.rand() * t.idle_rand;
        } else if r < t.walk_chance + t.idle_chance + t.jump_chance || !can_chase {
            self.start_jump(tn().jump_height);
        } else {
            self.act = Act::Chase;
            self.timer = tn().chase_give_up; // give up if the cursor can't be reached
        }
    }

    /// Where a walk of `step` ends. Walks that would run off the end of the
    /// area turn around instead of all piling up on its edge; only now and
    /// then does one go all the way there.
    fn stroll_target(&mut self, step: f32, min: f32, max: f32) -> f32 {
        let margin = ((max - min) * 0.1).min(tn().edge_margin);
        let (lo, hi) = (min + margin, max - margin);
        let t = self.x + step;
        if (lo..=hi).contains(&t) {
            return t;
        }
        if self.rand() < tn().edge_trip {
            return t.clamp(min, max);
        }
        let back = self.x - step;
        if (lo..=hi).contains(&back) {
            back
        } else {
            lo + self.rand() * (hi - lo).max(0.0)
        }
    }

    pub(super) fn start_pat(&mut self) {
        self.act = Act::Pat;
        self.timer = tn().pat_keep;
        self.jump = 0.0;
    }

    pub(super) fn start_jump(&mut self, amp: f32) {
        self.catch_jump = false;
        self.act = Act::Jump;
        self.jump_amp = amp;
        // higher jumps hang in the air a bit longer
        self.jump_dur = tn().jump_time * (amp / tn().jump_height).sqrt().max(1.0);
        self.jump = self.jump_dur;
    }

    /// Advance one frame. `mouse` is the cursor x relative to the anchor.
    /// Returns (offset, moving, jump height 0..1).
    pub(super) fn step(&mut self, dt: f32, min: f32, max: f32, mouse: Option<f32>, reach: f32) -> (f32, bool, f32) {
        if matches!(self.act, Act::Held | Act::Fall | Act::Planted | Act::Climb) {
            return (self.x, false, 0.0);
        }
        if let Some(t) = self.notify_jump.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                self.notify_jump = None;
                if !matches!(self.act, Act::Pat | Act::Jump) {
                    self.start_jump(tn().jump_height);
                }
            }
        }
        if let Some(r) = self.race_step(dt) {
            return r;
        }
        if let Some(r) = self.social_step(dt) {
            return r;
        }
        let mut moving = false;
        // circle drawn with the pointer: drop everything and chase it
        if std::mem::take(&mut self.circle)
            && mouse.is_some()
            && self.act != Act::Jump
        {
            self.act = Act::Chase;
            self.timer = tn().chase_give_up;
        }
        match self.act {
            Act::Idle => {
                self.timer -= dt;
                if self.timer <= 0.0 {
                    self.pick(min, max, mouse.is_some());
                }
            }
            Act::Walk | Act::Chase => {
                let chase = self.act == Act::Chase;
                if chase {
                    self.timer -= dt;
                    match mouse {
                        Some(m) => self.target = m.clamp(min, max),
                        None => self.timer = 0.0,
                    }
                }
                let speed = if chase { tn().chase_speed } else { tn().walk_speed };
                let d = self.target - self.x;
                // close enough counts as caught, or a cursor circling fast
                // keeps flipping the target left and right
                let catch = if chase { tn().catch_radius } else { 0.0 };
                if d.abs() <= speed * dt + catch {
                    if !chase {
                        self.x = self.target;
                    }
                    if chase {
                        // caught it: jump up to the cursor, chase is over
                        self.start_jump(reach.max(tn().jump_height));
                        self.catch_jump = true;
                    } else {
                        self.pick(min, max, mouse.is_some());
                    }
                } else if chase && self.timer <= 0.0 {
                    self.pick(min, max, false);
                } else {
                    // only turn around when the target is clearly on the other side
                    if !chase || d.abs() > 0.1 {
                        self.left = d < 0.0;
                    }
                    self.x += d.signum() * speed * dt;
                    self.energy = (self.energy + speed * dt / tn().energy_walk.max(0.01)).min(1.0);
                    moving = true;
                }
            }
            Act::Jump | Act::Held | Act::Fall | Act::Planted | Act::Climb => {}
            Act::Pat => {
                self.timer -= dt;
                if self.timer <= 0.0 {
                    // happy little hop afterwards
                    self.start_jump(tn().jump_height * 0.5);
                }
            }
        }
        let mut height = 0.0;
        if self.act == Act::Jump {
            self.race_hop(dt);
            self.social_hop(dt);
            self.jump -= dt;
            let t = 1.0 - (self.jump / self.jump_dur).max(0.0);
            height = 4.0 * t * (1.0 - t);
            if self.jump <= 0.0 {
                self.act = Act::Idle;
                self.timer = 0.5 + self.rand();
            }
        }
        (self.x, moving, height)
    }
}
