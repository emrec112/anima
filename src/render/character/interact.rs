//! Playing with the characters: picking them up, carrying and throwing them,
//! petting, drawn circles, notification hops, clinging to the cursor.

use crate::render::tuning::get as tn;
use super::CharacterOverlay;
use super::walk::{Act, WalkState};
use crate::render::FrameInfo;

/// Where a clinging character holds on, in half-heights above its centre.
pub(super) const GRIP: f32 = 0.5;

impl CharacterOverlay {
    /// Pointer and desktop events, handled by the main character for the
    /// whole team.
    pub(super) fn handle_input(&mut self, info: &FrameInfo) {
        // a notification arrived: everyone hops, one after another
        if self.actor == 0 {
            let n = crate::backend::notify::NOTIFICATIONS.load(std::sync::atomic::Ordering::Relaxed);
            if n != self.notified {
                self.notified = n;
                let free = |w: &WalkState| w.race.is_none() && !w.social.is_some_and(|s| s.no_touch || s.no_pat);
                if free(&self.walk) {
                    self.walk.notify_jump = Some(0.0);
                }
                for (i, e) in self.extras.iter_mut().enumerate() {
                    if free(&e.walk) {
                        e.walk.notify_jump = Some(0.12 * (i + 1) as f32);
                    }
                }
            }
        }

        // circle gesture: the main character hands it to a random team member
        if self.actor == 0
            && crate::backend::pointer::CIRCLE_GESTURE.swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            // logical px == surface px at scale 1
            let at = crate::backend::pointer::CIRCLE_AT.lock().unwrap().take().map(|c| [c.0, c.1, c.2]);
            // one character answers circles at a time: if one is already on its way, re-aim it
            let busy = |w: &WalkState| w.circle_target.is_some() && (w.circle || w.act == Act::Chase);
            let pick = if busy(&self.walk) {
                0
            } else if let Some(i) = self.extras.iter().position(|e| busy(&e.walk)) {
                i + 1
            } else {
                (self.walk.rand() * (self.extras.len() + 1) as f32) as usize
            };
            let wk = match pick.checked_sub(1).and_then(|i| self.extras.get_mut(i)) {
                Some(e) => &mut e.walk,
                None => &mut self.walk,
            };
            if !wk.social.is_some_and(|s| s.no_touch) {
                wk.circle = true;
                wk.circle_target = at;
            }
        }

        // press and hold on a character: pick it up
        if self.actor == 0 {
            let down = crate::backend::pointer::BUTTON_DOWN.load(std::sync::atomic::Ordering::Relaxed);
            let now = self.start.elapsed().as_secs_f32();
            if !down {
                self.press = None;
            } else if self.press.is_none() {
                let [mx, my] = info.mouse;
                let hit = |a: &CharacterOverlay| a.on_body([mx, my]) && !a.walk.social.is_some_and(|s| s.no_touch);
                let target = (1..=self.extras.len())
                    .rev()
                    .find(|&i| hit(&self.extras[i - 1]))
                    .or_else(|| hit(self).then_some(0));
                // measure the grab point now: the cursor may be far away by pick-up time
                let target = target.map(|i| {
                    let a = if i == 0 { &*self } else { &self.extras[i - 1] };
                    (i, a.body_clip([mx, my]))
                });
                self.press = Some((now, [mx, my], target));
            } else if let Some((t0, at, Some((i, grab)))) = self.press {
                // held still for a moment, or dragged a few pixels: pick it up
                let [mx, my] = info.mouse;
                let moved = (mx - at[0]).hypot(my - at[1]) > tn().drag_px * info.scale.max(1.0);
                if now - t0 > tn().pick_delay || moved {
                    let a = if i == 0 { &mut *self } else { &mut self.extras[i - 1] };
                    if a.walk.act != Act::Held {
                        // caught in flight: it has moved and turned since the press
                        let flying = matches!(a.walk.act, Act::Fall | Act::Planted);
                        a.walk.sink = 0.0;
                        a.walk.race_down = false;
                        let grab = if flying { a.body_clip([mx, my]) } else { grab };
                        // caught mid-jump: keep it at the height it is drawn at
                        if a.walk.act == Act::Jump && a.walk.jump_dur > 0.0 {
                            let t = 1.0 - (a.walk.jump / a.walk.jump_dur).clamp(0.0, 1.0);
                            a.walk.air += 4.0 * t * (1.0 - t) * a.walk.jump_amp;
                            a.walk.jump = 0.0;
                        }
                        a.walk.act = Act::Held;
                        a.walk.leaped = false;
                        a.walk.slipped = false;
                        a.walk.grab = Some(grab);
                        // carries on from the angle and spin it had
                        let pi = std::f32::consts::PI;
                        a.walk.swing = if flying { (a.walk.rot + pi).rem_euclid(2.0 * pi) - pi } else { 0.0 };
                        a.walk.swing_v = if flying { a.walk.tumble.clamp(-30.0, 30.0) } else { 0.0 };
                        a.walk.tumble = 0.0;
                        a.walk.rolling = false;
                        if flying {
                            a.walk.grip = true;
                        }
                        a.walk.swing_last = None;
                        a.walk.swing_vel = [0.0, 0.0];
                        a.walk.held_prev = None;
                        a.walk.vx = 0.0;
                        a.walk.vy = 0.0;
                    }
                    self.press = Some((t0, at, None));
                }
            }
        }

        if self.actor == 0 {
            let carried = |w: &WalkState| w.act == Act::Held && w.hang <= 0.0;
            let held = carried(&self.walk) || self.extras.iter().any(|e| carried(&e.walk));
            crate::backend::pointer::DRAGGING.store(held, std::sync::atomic::Ordering::Relaxed);
            let follows = |w: &WalkState| {
                matches!(w.act, Act::Held | Act::Chase) || w.catch_jump || w.circle || w.social.is_some_and(|s| s.chasing)
            };
            let wanted = follows(&self.walk) || self.extras.iter().any(|e| follows(&e.walk));
            crate::backend::pointer::CURSOR_WANTED.store(wanted, std::sync::atomic::Ordering::Relaxed);
        }

        // clicking on a character that is being petted keeps the petting going
        if self.actor == 0 {
            let n = crate::backend::pointer::CLICKS.load(std::sync::atomic::Ordering::Relaxed);
            if n != self.clicks {
                self.clicks = n;
                let [mx, my] = info.mouse;
                let keep = |a: &mut CharacterOverlay| {
                    if a.walk.act == Act::Pat && a.on_body([mx, my]) {
                        a.walk.timer = a.walk.timer.max(tn().pat_keep);
                        a.patted_again = true;
                    }
                };
                keep(self);
                self.extras.iter_mut().for_each(keep);
            }
        }

        // double click on a character: pet it (topmost one wins)
        if self.actor == 0
            && crate::backend::pointer::DOUBLE_CLICK.swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            let [mx, my] = info.mouse;
            log::debug!(
                "pat: mouse ({mx:.0},{my:.0}) main {:?} extras {:?}",
                self.hit_rect.map(|v| v.round()),
                self.extras.iter().map(|e| e.hit_rect.map(|v| v.round())).collect::<Vec<_>>()
            );
            // only on the ground: petting mid-jump or while carried breaks the motion
            let grounded = |w: &WalkState| !matches!(w.act, Act::Jump | Act::Held | Act::Fall) && !w.social.is_some_and(|s| s.no_touch || s.no_pat);
            if let Some(e) = self.extras.iter_mut().rev().find(|e| e.on_body([mx, my])) {
                if grounded(&e.walk) {
                    e.walk.start_pat();
                }
            } else if self.on_body([mx, my]) && grounded(&self.walk) {
                self.walk.start_pat();
            }
        }
    }

    /// The point it turns about, px from its centre, y up.
    fn pivot_px(&self) -> [f32; 2] {
        match (self.walk.grip, self.walk.feet, self.walk.grab) {
            (true, _, Some(g)) => [g[0] * self.surface[0] * 0.5, g[1] * self.surface[1] * 0.5],
            (_, true, _) => [0.0, -(self.hit_rect[3] - self.hit_rect[1]) * 0.5],
            _ => [0.0, 0.0],
        }
    }

    /// A screen point as it sits on the upright sprite: px from its centre, y up.
    fn body_offset(&self, m: [f32; 2]) -> [f32; 2] {
        let r = self.hit_rect;
        let p = [m[0] - (r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5 - m[1]];
        let piv = self.pivot_px();
        let (s, c) = self.walk.rot.sin_cos();
        let d = [p[0] - piv[0], p[1] - piv[1]];
        [c * d[0] + s * d[1] + piv[0], c * d[1] - s * d[0] + piv[1]]
    }

    /// `body_offset` in clip units, the form a grab point is kept in.
    fn body_clip(&self, m: [f32; 2]) -> [f32; 2] {
        let o = self.body_offset(m);
        [o[0] * 2.0 / self.surface[0].max(1.0), o[1] * 2.0 / self.surface[1].max(1.0)]
    }

    /// The drawn part of the sprite on an upright box: x half width, top, bottom, px, y up.
    fn body_extent(&self) -> [f32; 3] {
        let (w, h) = (self.hit_rect[2] - self.hit_rect[0], self.hit_rect[3] - self.hit_rect[1]);
        let [half, top, bottom] = self.opaque;
        [half * w, (0.5 - top) * h, (0.5 - bottom) * h]
    }

    /// Is this screen point on the character, not the empty margin of its image?
    pub(super) fn on_body(&self, m: [f32; 2]) -> bool {
        let o = self.body_offset(m);
        let [half, top, bottom] = self.body_extent();
        o[0].abs() <= half && o[1] <= top && o[1] >= bottom
    }

    /// Screen box around the drawn body, however it's turned.
    pub(super) fn body_box(&self) -> [f32; 4] {
        let r = self.hit_rect;
        let (cx, cy) = ((r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5);
        let [half, top, bottom] = self.body_extent();
        let piv = self.pivot_px();
        let (s, c) = self.walk.rot.sin_cos();
        let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for (x, y) in [(-half, top), (half, top), (-half, bottom), (half, bottom)] {
            let d = [x - piv[0], y - piv[1]];
            let (px, py) = (cx + piv[0] + c * d[0] - s * d[1], cy - piv[1] - s * d[0] - c * d[1]);
            b = [b[0].min(px), b[1].min(py), b[2].max(px), b[3].max(py)];
        }
        b
    }

    /// Held: follow the cursor; let go: fall back down (thrown).
    pub(super) fn carry(&mut self, info: &FrameInfo, dt: f32, center_clip: [f32; 2], half_clip: [f32; 2]) {
        let (w, h) = (info.width.max(1) as f32, info.height.max(1) as f32);
        let [view, extra] = self.view;
        let mx = (info.mouse[0] / w) * 2.0 - 1.0;
        let my = 1.0 - (info.mouse[1] / h) * 2.0;
        let down = crate::backend::pointer::BUTTON_DOWN.load(std::sync::atomic::Ordering::Relaxed);
        match self.walk.act {
            Act::Held if down || self.walk.hang > 0.0 => {
                if !down {
                    self.walk.hang -= dt;
                }
                let grab = *self.walk.grab.get_or_insert([
                    mx - (center_clip[0] + self.walk.x - view),
                    my - (center_clip[1] + self.walk.air * half_clip[1]),
                ]);
                let left_room = -(center_clip[0] - half_clip[0] + 1.0) + view;
                let right_room = 1.0 - (center_clip[0] + half_clip[0]) + view;
                self.walk.x = (mx - grab[0] - center_clip[0] + view).clamp(left_room, right_room);
                self.walk.target = self.walk.x;
                self.walk.air = ((my - grab[1] - center_clip[1]) / half_clip[1]).max(0.0);
                // throw speed: smoothed motion while held
                if let Some((px, pa)) = self.walk.held_prev {
                    let k = (dt * 12.0).min(1.0);
                    self.walk.vx += ((self.walk.x - px) / dt - self.walk.vx) * k;
                    self.walk.vy += ((self.walk.air - pa) / dt - self.walk.vy) * k;
                }
                self.walk.held_prev = Some((self.walk.x, self.walk.air));
                // held up, by hand or hanging on: a pendulum on the grip, swung
                // round it by the cursor
                {
                    let m = info.mouse;
                    let wk = &mut self.walk;
                    if let Some(l) = wk.swing_last {
                        let v = [(m[0] - l[0]) / dt, -(m[1] - l[1]) / dt];
                        let k = (dt * 25.0).min(1.0);
                        let smooth = [wk.swing_vel[0] + (v[0] - wk.swing_vel[0]) * k, wk.swing_vel[1] + (v[1] - wk.swing_vel[1]) * k];
                        let a = [(smooth[0] - wk.swing_vel[0]) / dt, (smooth[1] - wk.swing_vel[1]) / dt];
                        wk.swing_vel = smooth;
                        let (s, c) = wk.swing.sin_cos();
                        let push = -(a[0] * c + a[1] * s) / 120.0 * tn().cling_swing;
                        wk.swing_v = (wk.swing_v + (push - 40.0 * s - 2.0 * wk.swing_v) * dt).clamp(-30.0, 30.0);
                        wk.swing += wk.swing_v * dt;
                    }
                    wk.swing_last = Some(m);
                    wk.grip = true;
                }
            }
            Act::Held => {
                self.walk.act = Act::Fall;
                self.walk.held_prev = None;
                // let go mid-swing: keeps spinning the same way until it lands,
                // now about its own middle, moved so nothing jumps on screen
                self.walk.tumble = self.walk.swing_v;
                if let (true, Some(g)) = (self.walk.grip, self.walk.grab) {
                    let aspect = w / h;
                    let (s, c) = self.walk.rot.sin_cos();
                    let p = [g[0] * aspect, g[1]];
                    self.walk.x += (p[0] - (c * p[0] - s * p[1])) / aspect;
                    self.walk.air = (self.walk.air + (p[1] - (s * p[0] + c * p[1])) / half_clip[1]).max(0.0);
                    self.walk.grip = false;
                }
                // going to come down from high up: spinning all the way
                let g = tn().gravity;
                let apex = self.walk.air + self.walk.vy.max(0.0).powi(2) / (2.0 * g);
                if self.walk.tumble.abs() < 2.0 && apex >= tn().fall_high && !self.walk.social.is_some_and(|s| s.no_pat) {
                    let dir = if self.walk.vx < 0.0 { 1.0 } else { -1.0 };
                    self.walk.tumble = dir * (8.0 + self.walk.rand() * 6.0);
                }
            }
            Act::Fall => {
                let left_room = -(center_clip[0] - half_clip[0] + 1.0);
                let right_room = 1.0 - (center_clip[0] + half_clip[0]) + extra;
                self.walk.vy -= tn().gravity * dt;
                self.walk.air += self.walk.vy * dt;
                if !self.walk.tag_fall_run(dt) {
                    self.walk.x += self.walk.vx * dt;
                }
                // bounce off the ends of the strip
                if self.walk.x < left_room || self.walk.x > right_room {
                    self.walk.x = self.walk.x.clamp(left_room, right_room);
                    self.walk.vx = -self.walk.vx * tn().wall_bounce;
                }
                // and the top
                let top = (1.0 - center_clip[1]) / half_clip[1] - 1.0;
                if self.walk.air > top {
                    self.walk.air = top;
                    self.walk.vy = -self.walk.vy.abs() * tn().top_bounce;
                }
                self.walk.target = self.walk.x;
                // knocked flying: off the ground again a few times
                if self.walk.air <= 0.0 {
                    if let Some(s) = self.walk.social.as_mut().filter(|s| s.bounces > 0) {
                        s.bounces -= 1;
                        self.walk.air = 0.01;
                        self.walk.vy = self.walk.vy.abs() * tn().gift_bounce;
                        self.walk.vx *= 0.8;
                    }
                }
                if self.walk.air <= 0.0 {
                    let hard = self.walk.vy <= -(2.0 * tn().gravity * tn().fall_high).sqrt();
                    self.walk.air = 0.0;
                    self.walk.vx = 0.0;
                    self.walk.vy = 0.0;
                    // rolls on to upright rather than turning back
                    if self.walk.tumble != 0.0 {
                        let tau = std::f32::consts::TAU;
                        let dir = self.walk.tumble.signum();
                        let rot = (self.walk.rot + tau * 0.5).rem_euclid(tau) - tau * 0.5;
                        self.walk.rot = if rot * dir > 0.6 { rot - dir * tau } else { rot };
                        self.walk.rolling = true;
                    }
                    self.walk.tumble = 0.0;
                    if self.walk.social.is_some_and(|s| s.flying) {
                        self.walk.plant();
                    } else if self.walk.social.is_some_and(|s| s.no_pat) {
                        // straight back to running, no bounce
                        self.walk.act = Act::Idle;
                        self.walk.timer = 1.0;
                    } else {
                        self.walk.dropped = tn().drop_time;
                        let leaped = std::mem::take(&mut self.walk.leaped);
                        if (hard && !leaped) || std::mem::take(&mut self.walk.slipped) || self.walk.race_down {
                            self.walk.stick_in();
                        } else {
                            // land with a little bounce
                            self.walk.start_jump(tn().jump_height * tn().land_hop);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// The catch jump reached the cursor: cling to it by the body for 1-2 s.
    pub(super) fn catch_cling(&mut self, info: &FrameInfo) {
        if self.walk.act == Act::Jump && self.walk.catch_jump {
            let [mx, my] = info.mouse;
            let r = self.hit_rect;
            let size = r[3] - r[1];
            let (hx, hy) = ((r[0] + r[2]) * 0.5, r[1] + size * 0.28);
            let near = size * tn().cling_radius;
            if (mx - hx).powi(2) + (my - hy).powi(2) <= near * near {
                let wk = &mut self.walk;
                let t = 1.0 - (wk.jump / wk.jump_dur.max(0.001)).clamp(0.0, 1.0);
                wk.air += 4.0 * t * (1.0 - t) * wk.jump_amp;
                wk.jump = 0.0;
                wk.catch_jump = false;
                wk.act = Act::Held;
                wk.hang = tn().cling_min + wk.rand() * tn().cling_rand;
                wk.swing = 0.0;
                wk.swing_v = 0.0;
                wk.swing_last = None;
                wk.swing_vel = [0.0, 0.0];
                // held by the upper body: that's the point it swings round
                wk.grab = Some([0.0, (r[3] - r[1]) / info.height.max(1) as f32 * GRIP]);
                wk.held_prev = None;
                wk.vx = 0.0;
                wk.vy = 0.0;
            }
        }
    }

    /// Petting stops as soon as the cursor leaves the character.
    pub(super) fn pat_leaves(&mut self, info: &FrameInfo) {
        let [mx, my] = info.mouse;
        let r = self.hit_rect;
        const M: f32 = 12.0;
        if self.walk.act == Act::Pat
            && !(mx >= r[0] - M && mx <= r[2] + M && my >= r[1] - M && my <= r[3] + M)
        {
            self.walk.start_jump(tn().jump_height * 0.5);
        }
    }
}
