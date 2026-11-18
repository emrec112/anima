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
    // asleep or sitting down (see `rest`)
    Rest,
    // on its way to another workspace
    Travel,
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
    // horizontal coasting speed after exiting a slide (clip units/s)
    pub(super) glide_vx: f32,
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
    // over in the park for a reason (visit.rs), for how long, where from
    pub(super) visit_for: u8,
    pub(super) visit_t: f32,
    pub(super) visit_from: u64,
    // standing about doing nothing in the park (s)
    pub(super) idle_t: f32,
    // depth into the park meadow (0 at the bottom .. park::MEADOW at the back)
    pub(super) park_y: f32,
    pub(super) park_target_y: f32,
    // crowd scale inside the park (1.0 when few, smaller for 20-30 characters)
    pub(super) park_crowd: f32,
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
    pub(super) rest: Option<super::moods::Rest>,
    // which workspace it's on (0: not known), how long it's been there
    pub(super) ws: u64,
    pub(super) here_t: f32,
    // its workspace before the lock screen gathered everyone
    pub(super) lock_home: Option<u64>,
    pub(super) trip: Option<super::spaces::Trip>,
    // fullscreen: out of sight, or come out for a visit and for how long
    pub(super) backstage: bool,
    pub(super) visiting: bool,
    pub(super) visit_left: f32,
    // just arrived, hasn't met anyone yet
    pub(super) newcomer: bool,
    // let go of here (screen px): might land on another workspace
    pub(super) dropped_at: Option<[f32; 2]>,
    // jumped down on purpose: lands on its feet
    pub(super) leaped: bool,
    // games played lately, wears off slowly
    pub(super) played: f32,
    // seconds until it dozes off
    pub(super) sleep_wait: Option<f32>,
    // the processor is flat out and it's in a state about it
    pub(super) fret: bool,
    // laughing it off after a tumble (seconds), and whether that just began
    pub(super) giggle: f32,
    pub(super) giggle_new: bool,
    // until it waves at a friend again
    pub(super) greet_cool: f32,
    // seconds with nothing to do and nobody around
    pub(super) bored: f32,
    // snow settled on its head, 0..1
    pub(super) snow_pile: f32,
    // shook its umbrella out of its hand, and how hard it's been swung (s)
    pub(super) lost: Vec<String>,
    // bringing a friend its dropped thing: not to be taken into anything else
    pub(super) fetching: bool,
    // a short happy face (s): a loser getting up with a smile, no laughing fit
    pub(super) smile: f32,
    // hiding behind a window: strip x of its edge, and which side of it is
    // covered (-1 left, 1 right); nothing of it is drawn there
    pub(super) hide: Option<(f32, f32)>,
    // counting with its eyes covered (hide and seek)
    pub(super) eyes_shut: bool,
    pub(super) shake: f32,
    // swung violently while held; triggers item drop when the throw finishes
    pub(super) shaken: bool,
    // non-zero impact direction from landing hard / fast throw, triggers drop once
    pub(super) impact_dir: Option<f32>,
    // what it does with its umbrella now (social.rs, umbrella_mode), and
    // whether one is open over it (it then floats down slowly)
    pub(super) brolly: u8,
    pub(super) parasol: bool,
    // this jump started under an open umbrella
    pub(super) floaty: bool,
    // already knocked about in this tumble
    pub(super) jolted: bool,
    // playing a game right now (not just chatting), kept up by the main character
    pub(super) in_game: bool,
    // trained a while ago: not again for a bit
    pub(super) train_cool: f32,
    // its favourite keepsake, when to look again, whether it carries it about now, till when
    pub(super) keepsake: Option<i32>,
    pub(super) fav_t: f32,
    pub(super) carry: bool,
    pub(super) carry_t: f32,
    // a sign held up (which one), for how much longer, until it thinks of one again
    pub(super) sign: Option<usize>,
    pub(super) sign_left: f32,
    pub(super) sign_t: f32,
    // night: seconds until it goes to bed, and whether it's in bed for the night
    pub(super) night_wait: Option<f32>,
    pub(super) night_sleep: bool,
    // woken by the rain starting: seconds till it's up
    pub(super) rain_wake: Option<f32>,
    // the state machine's bookkeeping: current state, seconds in it, and
    // whether a walk is getting anywhere
    pub(super) state_act: Act,
    pub(super) state_age: f32,
    pub(super) walk_from: f32,
    pub(super) walk_stall: f32,
    // just gave up fishing: not again for a while
    pub(super) fish_cool: f32,
    // visiting a fullscreen app: gets a rod out once it's up
    pub(super) fish_on_arrival: bool,
    // petted while busy: a happy moment that doesn't stop what it's doing
    pub(super) petted: f32,
    // reacting to accepting a "Hug?" sign: torso hug squeeze, then a joyful hop
    pub(super) hug: f32,
    // waiting for its turn to be petted
    pub(super) crowd: Option<super::pat::PatCrowd>,
    // animation time not yet given to the 3D model (skipped frames)
    pub(super) model_dt: f32,
}

pub(super) const PAT_DUR: f32 = 1.4;

impl WalkState {
    /// An open umbrella on its back in a game or a race catches the air:
    /// a bit slower on its feet.
    pub(super) fn drag(&self) -> f32 {
        if self.parasol && self.brolly == 2 { tn().parasol_drag } else { 1.0 }
    }

    /// Up or on its way to somewhere else, not on the ground strolling.
    pub(super) fn climb_like(&self) -> bool {
        self.trip.is_some() || self.rest.is_some()
    }

    /// Laughs for a moment: stays put, happy face, little hops.
    pub(super) fn laugh(&mut self, secs: f32) {
        if secs > self.giggle {
            self.giggle = secs;
            self.giggle_new = true;
        }
    }

    /// One frame of laughing, on the ground.
    pub(super) fn giggle_step(&mut self, dt: f32) -> (f32, bool, f32) {
        self.giggle -= dt;
        self.act = Act::Idle;
        self.timer = 0.5;
        self.target = self.x;
        if self.rand() < dt * 3.5 {
            let h = tn().jump_height * (0.2 + self.rand() * 0.2);
            self.start_jump(h);
        }
        (self.x, false, 0.0)
    }

    pub(super) fn rand(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    pub(super) fn pick_in(&mut self, min: f32, max: f32, can_chase: bool, max_y: f32) {
        let t = tn();
        // in a panic: off to somewhere else at once, no standing about
        if self.fret {
            let mut to = min + self.rand() * (max - min);
            if (to - self.x).abs() < 0.25 {
                to = if self.x < (min + max) * 0.5 { self.x + 0.5 } else { self.x - 0.5 };
            }
            self.target = to.clamp(min, max);
            if max_y > 0.0 {
                self.park_target_y = self.rand() * max_y;
            }
            self.act = Act::Walk;
            return;
        }
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
            if max_y > 0.0 {
                let y = if self.rand() < 0.5 {
                    self.park_y + (self.rand() - 0.5) * max_y * 0.8
                } else {
                    self.rand() * max_y
                };
                self.park_target_y = y.clamp(0.0, max_y);
            }
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

    pub(super) fn start_hug(&mut self) {
        self.sign = None;
        self.hug = 1.65;
        self.act = Act::Pat;
        self.timer = 0.72;
        self.jump = 0.0;
        self.energy = (self.energy + 0.12).min(1.0);
    }

    /// Height of a jump `t` of the way through it (0..1, peak 1). Under an
    /// open umbrella it goes up quickly and drifts down slowly.
    pub(super) fn arc(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let u = if self.floaty {
            let peak = tn().parasol_peak;
            if t < peak { t / peak * 0.5 } else { 0.5 + (t - peak) / (1.0 - peak).max(0.01) * 0.5 }
        } else {
            t
        };
        4.0 * u * (1.0 - u)
    }

    pub(super) fn start_jump(&mut self, amp: f32) {
        if self.act == Act::Jump && self.jump > 0.0 {
            return;
        }
        self.catch_jump = false;
        self.act = Act::Jump;
        self.jump_amp = amp;
        // higher jumps hang in the air a bit longer, and longer still under an
        // open umbrella (carrot grab, tag, any game in the rain)
        let hang = if self.parasol { tn().parasol_hang } else { 1.0 };
        self.floaty = self.parasol;
        self.jump_dur = tn().jump_time * (amp / tn().jump_height).sqrt().max(1.0) * hang;
        self.jump = self.jump_dur;
    }

    /// Advance one frame. `mouse` is the cursor x relative to the anchor.
    /// Returns (offset, moving, jump height 0..1).
    #[cfg(test)]
    pub(super) fn step(&mut self, dt: f32, min: f32, max: f32, mouse: Option<f32>, reach: f32) -> (f32, bool, f32) {
        self.step_in(dt, min, max, mouse, reach, 0.0, 0.0)
    }

    /// Moves `park_y` smoothly toward `park_target_y` while grounded in the park.
    fn step_park_y(&mut self, dt: f32, max_y: f32, y_per_clip: f32, speed: f32) -> bool {
        if max_y <= 0.0 || y_per_clip <= 0.0 || self.social.is_some_and(|s| s.perched) {
            return false;
        }
        self.park_target_y = self.park_target_y.clamp(0.0, max_y);
        let dy = self.park_target_y - self.park_y;
        if dy.abs() <= 0.0005 {
            return false;
        }
        let step_y = speed * self.drag() * y_per_clip * dt;
        if dy.abs() <= step_y {
            self.park_y = self.park_target_y;
            false
        } else {
            self.park_y += dy.signum() * step_y;
            true
        }
    }

    /// Applies and damps slide-exit horizontal momentum, syncing `target` and
    /// `social.goal` so `social_step` does not snap back to the pre-glide spot.
    fn step_glide(&mut self, dt: f32, min: f32, max: f32) {
        if self.glide_vx == 0.0 {
            return;
        }
        if self.crowd.is_some()
            || self.race.is_some()
            || matches!(self.act, Act::Chase | Act::Pat)
            || self.social.as_ref().is_some_and(|r| r.moving)
        {
            self.glide_vx = 0.0;
            return;
        }
        let next = self.x + self.glide_vx * dt;
        if (next <= min && self.glide_vx < 0.0) || (next >= max && self.glide_vx > 0.0) {
            self.x = next.clamp(min, max);
            self.glide_vx = 0.0;
        } else {
            self.x = next.clamp(min, max);
            if self.act == Act::Jump {
                self.glide_vx *= (1.0 - 0.8 * dt).max(0.0);
            } else {
                let decel = 0.35 * dt;
                if self.glide_vx.abs() <= decel {
                    self.glide_vx = 0.0;
                } else {
                    self.glide_vx -= self.glide_vx.signum() * decel;
                }
            }
        }
        self.target = self.x;
        if self.act == Act::Walk {
            self.act = Act::Idle;
        }
        if self.act == Act::Idle && self.glide_vx != 0.0 {
            self.timer = self.timer.max(0.05);
        }
        if let Some(r) = self.social.as_mut().filter(|r| !r.moving) {
            r.goal = self.x;
        }
    }

    /// Advance one frame, including 2.5D depth when inside Anima Park
    /// (`max_y > 0.0`, `y_per_clip` converting clip-width units to height share).
    pub(super) fn step_in(&mut self, dt: f32, min: f32, max: f32, mouse: Option<f32>, reach: f32, max_y: f32, y_per_clip: f32) -> (f32, bool, f32) {
        self.hug = (self.hug - dt).max(0.0);
        if max_y <= 0.0 {
            self.park_y = 0.0;
            self.park_target_y = 0.0;
        }
        if matches!(self.act, Act::Held | Act::Fall | Act::Planted | Act::Rest | Act::Travel) {
            self.glide_vx = 0.0;
            return (self.x, false, 0.0);
        }
        if self.fret || self.sign.is_some() {
            self.notify_jump = None;
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
        self.step_glide(dt, min, max);
        if self.giggle > 0.0 && self.social.is_none() && self.race.is_none() && matches!(self.act, Act::Idle | Act::Walk) {
            return self.giggle_step(dt);
        }
        let speed = if self.crowd.is_some() {
            tn().pat_crowd_speed.max(tn().walk_speed)
        } else {
            self.social.as_ref().map_or(tn().walk_speed, |r| if r.moving { r.speed.max(tn().walk_speed) } else { tn().walk_speed })
        };
        if let Some(mut r) = self.crowd_step(dt, min, max) {
            r.1 |= self.step_park_y(dt, max_y, y_per_clip, speed);
            return r;
        }
        if let Some(r) = self.race_step(dt) {
            return r;
        }
        if self.social.is_some() {
            let y_moving = self.step_park_y(dt, max_y, y_per_clip, speed);
            if let Some(mut r) = self.social_step(dt) {
                r.1 |= y_moving;
                return r;
            }
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
                // looks about now and then
                if self.rand() < dt * tn().look_around {
                    self.left = !self.left;
                }
                self.timer -= dt;
                if self.fret {
                    self.timer = 0.0;
                }
                if self.timer <= 0.0 {
                    self.pick_in(min, max, mouse.is_some(), max_y);
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
                let speed = if chase {
                    tn().chase_speed
                } else if self.fret {
                    tn().walk_speed * 4.0
                } else {
                    tn().walk_speed
                };
                let d = self.target - self.x;
                let dy = if !chase && max_y > 0.0 && y_per_clip > 0.0 {
                    self.park_target_y = self.park_target_y.clamp(0.0, max_y);
                    self.park_target_y - self.park_y
                } else {
                    0.0
                };
                let dy_eq = if y_per_clip > 0.0 { dy / y_per_clip } else { 0.0 };
                let dist = d.hypot(dy_eq);
                // close enough counts as caught, or a cursor circling fast
                // keeps flipping the target left and right
                let catch = if chase { tn().catch_radius } else { 0.0 };
                if dist <= speed * dt + catch {
                    if !chase {
                        self.x = self.target;
                        if max_y > 0.0 {
                            self.park_y = self.park_target_y;
                        }
                    }
                    if chase {
                        // caught it: jump up to the cursor, chase is over
                        self.start_jump(reach.max(tn().jump_height));
                        self.catch_jump = true;
                    } else {
                        self.pick_in(min, max, mouse.is_some(), max_y);
                    }
                } else if chase && self.timer <= 0.0 {
                    self.pick_in(min, max, false, max_y);
                } else {
                    // only turn around when the target is clearly on the other side
                    if (!chase || d.abs() > 0.1) && d.abs() > 0.005 {
                        self.left = d < 0.0;
                    }
                    let inv = if dist > 1e-5 { speed * dt / dist } else { 0.0 };
                    self.x += d * inv;
                    self.park_y = (self.park_y + dy * inv).clamp(0.0, max_y.max(0.0));
                    self.energy = (self.energy + speed * dt / tn().energy_walk.max(0.01)).min(1.0);
                    moving = true;
                }
            }
            Act::Jump | Act::Held | Act::Fall | Act::Planted | Act::Rest | Act::Travel => {}
            Act::Pat => {
                self.timer -= dt;
                if self.timer <= 0.0 {
                    if self.hug > 0.0 {
                        // joyful finish after a hug: hop and a quick giggle
                        self.start_jump(tn().jump_height * 0.8);
                        self.laugh(1.2);
                    } else {
                        // happy little hop afterwards
                        self.start_jump(tn().jump_height * 0.5);
                    }
                }
            }
        }
        let mut height = 0.0;
        if self.act == Act::Jump {
            self.race_hop(dt);
            self.social_hop(dt);
            self.jump -= dt;
            let t = 1.0 - (self.jump / self.jump_dur).max(0.0);
            height = self.arc(t);
            if self.jump <= 0.0 {
                self.act = Act::Idle;
                // landed without catching the cursor
                self.catch_jump = false;
                self.timer = 0.5 + self.rand();
            }
        }
        (self.x, moving, height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hug_sign_squeezes_then_hops_joyfully() {
        assert!(super::super::sign::is_hug(super::super::sign::HUG));
        assert!(!super::super::sign::is_hug(0));
        let mut wk = WalkState::default();
        wk.sign = Some(super::super::sign::HUG);
        wk.energy = 0.5;
        wk.start_hug();
        assert!(wk.sign.is_none());
        assert_eq!(wk.act, Act::Pat);
        assert!(wk.hug > 1.0);
        assert!(wk.energy > 0.5);
        wk.step(0.8, -1.0, 1.0, None, 0.0);
        assert_eq!(wk.act, Act::Jump);
        assert!(wk.giggle > 0.0);
    }

    #[test]
    fn crowd_aligns_park_y() {
        let mut wk = WalkState {
            act: Act::Idle,
            x: 0.2,
            target: 0.2,
            park_y: 0.02,
            park_target_y: 0.28,
            crowd: Some(super::super::pat::PatCrowd::curious(0.2, 0.2, 2.0, false)),
            ..Default::default()
        };
        for _ in 0..240 {
            wk.step_in(1.0 / 60.0, -1.0, 1.0, None, 0.0, 0.35, 1.0);
        }
        assert!((wk.park_y - 0.28).abs() < 1e-3, "stuck at {}", wk.park_y);
    }

    #[test]
    fn mid_air_bob_does_not_snap_jump_to_ground() {
        let mut wk = WalkState { act: Act::Idle, ..Default::default() };
        wk.start_jump(2.0);
        let dur = wk.jump_dur;
        // Advance halfway through the jump near peak height
        let (_, _, peak_h) = wk.step(dur * 0.5, -1.0, 1.0, None, 0.0);
        assert!(peak_h > 0.9, "expected near-peak height, got {peak_h}");

        // Losing/passing the carrot calls start_bob (or start_jump); height must not snap to 0
        wk.start_bob(0.2);
        wk.start_jump(0.5);
        assert_eq!(wk.act, Act::Jump);
        assert_eq!(wk.jump_dur, dur);
        let (_, _, next_h) = wk.step(1.0 / 60.0, -1.0, 1.0, None, 0.0);
        assert!(next_h > 0.7, "jump snapped to ground unexpectedly: {next_h}");
    }

    #[test]
    fn glide_vx_coasts_during_exit_hop_and_decelerates_on_grass() {
        let mut social = Social::default();
        social.goal = 0.0;
        social.moving = false;
        let mut wk = WalkState {
            x: 0.0,
            target: 0.0,
            act: Act::Idle,
            social: Some(social),
            ..Default::default()
        };
        wk.start_jump(tn().jump_height * 0.45);
        wk.glide_vx = 0.36;

        let dt = 1.0 / 60.0;
        while wk.act == Act::Jump {
            wk.step(dt, -1.0, 1.0, None, 0.0);
        }
        let landed_x = wk.x;
        assert!(landed_x > 0.05, "expected forward progress during exit hop, got {landed_x}");
        assert!(wk.glide_vx > 0.0 && wk.glide_vx < 0.36, "expected light air drag, got {}", wk.glide_vx);
        assert_eq!(wk.social.unwrap().goal, landed_x);

        for _ in 0..120 {
            wk.step(dt, -1.0, 1.0, None, 0.0);
        }
        let stopped_x = wk.x;
        assert_eq!(wk.glide_vx, 0.0);
        assert!(stopped_x > landed_x, "expected grass coasting past {landed_x}, got {stopped_x}");
        assert_eq!(wk.target, stopped_x);
        assert_eq!(wk.social.unwrap().goal, stopped_x);

        // Must not snap back toward pre-glide goal
        for _ in 0..30 {
            wk.step(dt, -1.0, 1.0, None, 0.0);
        }
        assert_eq!(wk.x, stopped_x);
    }

    #[test]
    fn glide_vx_stops_at_boundary_and_clears_on_interrupts() {
        let mut wk = WalkState { x: 0.98, target: 0.98, act: Act::Idle, glide_vx: 0.5, ..Default::default() };
        wk.step(0.1, -1.0, 1.0, None, 0.0);
        assert_eq!(wk.x, 1.0);
        assert_eq!(wk.glide_vx, 0.0);

        for act in [Act::Held, Act::Fall, Act::Rest, Act::Travel] {
            let mut w = WalkState { act, glide_vx: 0.4, ..Default::default() };
            w.step(1.0 / 60.0, -1.0, 1.0, None, 0.0);
            assert_eq!(w.glide_vx, 0.0, "expected glide_vx cleared for {act:?}");
        }

        let mut moving_social = Social::default();
        moving_social.moving = true;
        moving_social.goal = -0.5;
        moving_social.speed = 0.2;
        let mut w = WalkState {
            act: Act::Idle,
            glide_vx: 0.4,
            social: Some(moving_social),
            ..Default::default()
        };
        w.step(1.0 / 60.0, -1.0, 1.0, None, 0.0);
        assert_eq!(w.glide_vx, 0.0);
    }
}
