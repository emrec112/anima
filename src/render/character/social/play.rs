//! Anima Park: things built in the park window, how characters walk up to
//! them across the 2.5D meadow and play on them together.

use super::*;
use super::super::park::{self, Park, Thing};
use crate::backend::governor;
use std::sync::Mutex;
use std::time::{Instant, SystemTime};

mod visit;

pub(super) struct Play {
    pub(super) thing: usize,
    pub(super) kind: String,
    riders: Vec<Rider>,
    t: f32,
    stay: f32,
}

#[derive(Clone, Copy)]
struct Rider {
    i: usize,
    phase: u8,
    t: f32,
    // goes down the slide this many times / swings this long
    n: u8,
    want: u8,
    fell: bool,
}

struct Cache {
    checked: Option<Instant>,
    mtime: Option<SystemTime>,
    park: Park,
    unit_written: f32,
    world_written: Option<Instant>,
    world_last: Option<park::World>,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache {
    checked: None,
    mtime: None,
    park: Park { things: Vec::new() },
    unit_written: 0.0,
    world_written: None,
    world_last: None,
});

/// What's built, read again when the file changes (looked at once a second).
fn park_now() -> Park {
    let mut c = CACHE.lock().unwrap();
    if c.checked.is_none_or(|t| t.elapsed().as_secs_f32() >= 1.0) {
        c.checked = Some(Instant::now());
        let m = std::fs::metadata(park::park_path()).and_then(|m| m.modified()).ok();
        if m != c.mtime {
            c.mtime = m;
            c.park = park::read_park();
        }
    }
    c.park.clone()
}

/// Seesaw has two riders actively rocking (past approaching).
fn seesaw_rocking(p: &Play) -> bool {
    p.riders.len() >= 2 && p.riders[0].phase >= 1 && p.riders[1].phase >= 1
}

/// Hands/props idle y (no longer needed outside food), kept internal if needed.
fn column(place: &str) -> Option<(f32, f32)> {
    governor::place_rect(place).map(|p| (p.x, p.w))
}

/// Clamps a walk target `x` into a column `[px, px + pw]` with character margin `pad`.
fn clamp_place(px: f32, pw: f32, pad: f32, x: f32) -> f32 {
    if pw <= 1.0 {
        return x;
    }
    let margin = pad.min(pw * 0.45);
    x.clamp(px + margin, px + pw - margin)
}

/// Ground baseline pixel Y (`gy` at `park_y = 0`) for actor `i`.
fn ground_bottom(this: &CharacterOverlay, i: usize) -> f32 {
    let a = this.actor_ref(i);
    let r = a.hit_rect;
    let h = (r[3] - r[1]).max(1.0);
    let air_px = a.walk.air * h * 0.5;
    let up_px = governor::place_rect("park").map_or(0.0, |p| a.walk.park_y * p.h);
    r[1] + h * a.opaque[2] + air_px + up_px
}

impl CharacterOverlay {
    /// Inside the Anima Park window right now.
    pub(in crate::render::character) fn place_here(&self, i: usize) -> Option<governor::Place> {
        governor::place_at(self.walk_ref(i).ws, self.strip_px(i))
    }

    pub(super) fn height_of(&self, i: usize) -> f32 {
        let r = if i == 0 { self.hit_rect } else { self.extras[i - 1].hit_rect };
        (r[3] - r[1]).max(1.0)
    }

    /// Front-row character height in strip px inside Anima Park (`depth_scale = 1.0`),
    /// factoring in automatic crowd scaling for 20-30 characters.
    fn park_unit(&self, n: usize) -> f32 {
        let mut sum = 0.0;
        let mut cnt = 0usize;
        for i in 0..n {
            if self.place_here(i).is_some() {
                let d = park::depth_scale(self.walk_ref(i).park_y).max(0.5);
                let tuck = self.walk_ref(i).social.and_then(|s| s.tuck).unwrap_or(1.0).max(0.4);
                sum += self.height_of(i) / (d * tuck);
                cnt += 1;
            }
        }
        if cnt > 0 {
            return (sum / cnt as f32).max(20.0);
        }
        let crowd = self.walk_ref(0).park_crowd;
        let crowd = if crowd > 0.0 { crowd } else { 1.0 };
        (self.height_of(0) * crowd).max(20.0)
    }

    /// Character-matched prop unit at `t`'s depth across the 2.5D meadow.
    fn park_unit_at(&self, t: &Thing, n: usize) -> f32 {
        self.park_unit(n) * park::depth_scale(t.y)
    }

    /// Tells the park window how big a front-row character is, so it scales props to match.
    pub(super) fn park_tell_unit(&mut self, n: usize) {
        if governor::places().is_empty() || self.view_w < 1.0 {
            return;
        }
        let width = governor::places().first().map_or(self.view_w, |p| p.w.max(1.0));
        let frac = self.park_unit(n) / width;
        let mut c = CACHE.lock().unwrap();
        if (frac - c.unit_written).abs() > c.unit_written * 0.03 {
            c.unit_written = frac;
            let text = format!("{{\"unit_frac\": {frac:.5}}}");
            let _ = crate::util::atomic_write(&park::view_path(), text.as_bytes());
        }
    }

    /// Lowers someone back down gently if interrupted mid-air (does NOT pull back to `park_y = 0`).
    pub(super) fn push_down(&mut self, i: usize) {
        if self.walk_ref(i).air <= 0.05 {
            self.let_down(i);
            return;
        }
        self.chat_id = self.chat_id.wrapping_add(1);
        let id = self.chat_id;
        self.enlist(i, id);
        let mut c = Chat::new(id, vec![i], Stage::Play, 20.0);
        c.play = Some(Play { thing: usize::MAX, kind: "down".into(), riders: Vec::new(), t: 0.0, stay: 0.0 });
        self.new_chats.push(c);
    }

    /// Keeps a park walk target safely inside `t.place` so corner props never send riders past a wall.
    fn clamp_in_place(&self, t: &Thing, i: usize, unit: f32, x: f32) -> f32 {
        let Some((px, pw)) = column(&t.place) else { return x };
        let pad = (self.width_of(i) * 0.55).max(unit * 0.4);
        clamp_place(px, pw, pad, x)
    }

    /// Walks across the 2.5D park lawn to `(x, y_norm)` at normal speed. True once there.
    fn walk_to_2d(&mut self, i: usize, x: f32, y_norm: f32, speed: f32) -> bool {
        let y_norm = y_norm.clamp(0.0, park::MEADOW);
        let dx = x - self.strip_px(i);
        let dy = y_norm - self.walk_ref(i).park_y;
        {
            let wk = self.walk_of(i);
            wk.park_target_y = y_norm;
            wk.air = 0.0;
        }
        if dx.abs() <= 6.0 && dy.abs() <= 0.015 {
            let wk = self.walk_of(i);
            wk.park_y = y_norm;
            if let Some(r) = wk.social.as_mut() {
                r.perched = false;
                r.moving = false;
            }
            return true;
        }
        if dx.abs() > 6.0 && !self.walk_ref(i).social.is_some_and(|s| s.moving) && self.arrived(i) {
            self.go_facing(i, x, speed);
        } else if let Some(r) = self.walk_of(i).social.as_mut() {
            r.perched = false;
        }
        false
    }

    /// Steps vertically onto or off a low perch (chair plank, ladder, chute) while staying at depth `y_norm`.
    fn walk_up_at(&mut self, i: usize, x: f32, y_norm: f32, up: f32, dt: f32) -> bool {
        if !self.walk_to_2d(i, x, y_norm, 0.15) {
            return false;
        }
        let h = self.height_of(i);
        let now = self.walk_ref(i).air * h * 0.5;
        let dv = up - now;
        if dv.abs() <= 2.0 {
            let wk = self.walk_of(i);
            wk.air = if up < 0.5 { 0.0 } else { up / (h * 0.5) };
            if let Some(r) = wk.social.as_mut() {
                r.perched = up > 0.0;
                r.moving = false;
            }
            return true;
        }
        let unit = self.park_unit(self.extras.len() + 1);
        let step = 0.9 * unit * dt;
        let next = (now + dv.signum() * step.min(dv.abs())).max(0.0);
        let wk = self.walk_of(i);
        wk.air = next / (h * 0.5);
        if let Some(r) = wk.social.as_mut() {
            r.perched = wk.air > 0.0;
        }
        false
    }

    fn thing_px(&self, t: &Thing) -> f32 {
        match column(&t.place) {
            Some((x, w)) => x + t.x * w,
            None => t.x * self.view_w,
        }
    }

    /// Holds `i` at `(px, y_norm)` on the meadow and `up` px above that spot.
    fn hold_at(&mut self, i: usize, px: f32, y_norm: f32, up: f32) {
        let here = self.strip_px(i);
        let h = self.height_of(i);
        let surface = self.actor_ref(i).surface[0].max(1.0);
        let wk = self.walk_of(i);
        wk.x += (px - here) * 2.0 / surface;
        wk.target = wk.x;
        wk.park_y = y_norm;
        wk.park_target_y = y_norm;
        wk.air = (up / (h * 0.5)).max(0.0);
        if matches!(wk.act, Act::Walk) {
            wk.act = Act::Idle;
        }
        if let Some(r) = wk.social.as_mut() {
            r.perched = up > 0.0;
            r.moving = false;
            r.goal = wk.x;
            r.goal_px = px;
        }
    }

    fn actor_surface_w(&self, i: usize) -> f32 {
        let s = self.actor_ref(i).surface[0];
        if s > 1.0 { s } else { self.view_w.max(1.0) }
    }

    /// Off the ride onto the grass right where it is (`park_y` stays untouched).
    fn let_down(&mut self, i: usize) {
        clear_ride_state(self.walk_of(i));
    }

    fn set_happy(&mut self, i: usize, on: bool) {
        if let Some(r) = self.walk_of(i).social.as_mut() {
            r.happy = on;
        }
    }

    fn set_face(&mut self, i: usize, left: bool) {
        if let Some(r) = self.walk_of(i).social.as_mut() {
            r.face_left = left;
        }
    }

    /// Once in a while someone free in the park goes to play on something
    /// there, or joins whoever is on it already.
    pub(super) fn park_play(&mut self, chats: &mut Vec<Chat>, n: usize) {
        let homes = governor::places();
        if homes.is_empty() {
            return;
        }
        let who: Vec<usize> = (0..n)
            .filter(|&i| homes.iter().any(|h| h.ws == self.walk_ref(i).ws) && self.free_for(i, Want::Play))
            .collect();
        if who.is_empty() {
            return;
        }
        let i = who[(self.walk.rand() * who.len() as f32) as usize % who.len()];
        self.play_for(chats, i, n, false);
    }

    /// Picks a park prop with free spots for `i`.
    fn play_for(&mut self, chats: &mut Vec<Chat>, i: usize, n: usize, force: bool) -> bool {
        let park = park_now();
        let Some(place) = self.place_here(i).map(|p| p.place) else { return false };
        let busy = !force && self.too_busy(chats, n);
        if !force && (self.walk.rand() >= tn().park_play || busy) {
            return false;
        }
        let mut options: Vec<(usize, f32)> = Vec::new();
        for (k, t) in park.things.iter().enumerate() {
            let room = park::room(&t.kind);
            if room == 0 || t.place != place {
                continue;
            }
            let on = chats.iter().find(|c| c.stage == Stage::Play && c.play.as_ref().is_some_and(|p| p.thing == k && p.kind == t.kind));
            let used = on.map_or(0, |c| c.members.len());
            if used >= room {
                continue;
            }
            let waiting = matches!(t.kind.as_str(), "seesaw" | "table" | "bench") && used == 1;
            let weight = if waiting { 3.0 } else { 1.0 };
            options.push((k, weight));
        }
        let total: f32 = options.iter().map(|o| o.1).sum();
        if total <= 0.0 {
            return false;
        }
        let mut roll = self.walk.rand() * total;
        let Some(&(k, _)) = options.iter().find(|o| {
            roll -= o.1;
            roll <= 0.0
        }) else {
            return false;
        };
        let kind = park.things[k].kind.clone();
        self.start_play(chats, i, k, &kind);
        true
    }

    /// Cap how many characters occupy park items at once so some remain free for games.
    fn too_busy(&self, chats: &[Chat], n: usize) -> bool {
        let busy: usize = chats
            .iter()
            .filter_map(|c| c.play.as_ref().map(|p| (p, c.members.len())))
            .filter(|(p, _)| p.kind != "down")
            .map(|(_, k)| k)
            .sum();
        busy as f32 >= n as f32 * tn().park_busy_most
    }

    /// Game time in the park: now and then the free ones in it get together
    /// for one of the usual group games.
    fn game_time(&mut self, chats: &mut Vec<Chat>, n: usize) {
        static NEXT: Mutex<Option<Instant>> = Mutex::new(None);
        static GATHER: Mutex<Option<Instant>> = Mutex::new(None);
        let gather_now = {
            let mut g = GATHER.lock().unwrap();
            if g.is_some_and(|t| Instant::now() >= t) {
                *g = None;
                true
            } else {
                false
            }
        };
        if !gather_now {
            let mut next = NEXT.lock().unwrap();
            let every = tn().park_game_every.max(10.0);
            match *next {
                None => {
                    *next = Some(Instant::now() + std::time::Duration::from_secs_f32(every * 0.5));
                    return;
                }
                Some(t) if Instant::now() < t => return,
                _ => {}
            }
            let r = self.walk.rand();
            *next = Some(Instant::now() + std::time::Duration::from_secs_f32(every * (0.6 + r * 0.8)));
            drop(next);
            if visit::visiting_mode() {
                if self.game_volunteers(n) > 0 {
                    *GATHER.lock().unwrap() = Some(Instant::now() + std::time::Duration::from_secs(25));
                }
                return;
            }
        }
        let Some(park) = governor::place_rect("park") else { return };
        if self.race.is_some() {
            return;
        }
        let middle = park.x + park.w * 0.5;
        let mut free: Vec<(usize, f32)> = (0..n)
            .filter(|&i| {
                let wk = self.walk_ref(i);
                wk.ws == park.ws && wk.at_leisure() && wk.trip.is_none() && wk.rest.is_none() && wk.energy >= tn().energy_game_need * 0.5
            })
            .map(|i| (i, (self.strip_px(i) - middle).abs()))
            .collect();
        free.sort_by(|a, b| a.1.total_cmp(&b.1));
        free.truncate(6);
        if free.len() < 3 {
            log::debug!("park: game time, but only {} free", free.len());
            return;
        }
        let members: Vec<usize> = free.iter().map(|f| f.0).collect();
        let rivalry = self.rivalry(&members);
        let Some(game) = self.choose_game(&[], members.len(), rivalry) else { return };
        self.chat_id = self.chat_id.wrapping_add(1);
        let id = self.chat_id;
        for &i in &members {
            self.enlist(i, id);
        }
        log::debug!("chat {id}: game time in the park, {game:?} with {members:?}");
        let mut c = Chat::new(id, members, Stage::Approach, 0.0);
        c.forced = Some(game);
        self.arrange(&c);
        chats.push(c);
    }

    /// `i` goes to play on thing `k`, joining whoever is on it already.
    fn start_play(&mut self, chats: &mut Vec<Chat>, i: usize, k: usize, kind: &str) {
        if let Some(c) = chats.iter_mut().find(|c| c.stage == Stage::Play && c.play.as_ref().is_some_and(|p| p.thing == k && p.kind == kind)) {
            self.enlist(i, c.id);
            c.members.push(i);
            log::debug!("chat {}: {i} joins in on the {kind}", c.id);
            return;
        }
        self.chat_id = self.chat_id.wrapping_add(1);
        let id = self.chat_id;
        self.enlist(i, id);
        let mut c = Chat::new(id, vec![i], Stage::Play, 600.0);
        let stay = match kind {
            "table" => 25.0 + self.walk.rand() * 20.0,
            "chair" | "bench" => 12.0 + self.walk.rand() * 20.0,
            "sandbox" => 15.0 + self.walk.rand() * 15.0,
            "seesaw" => 10.0 + self.walk.rand() * 8.0,
            _ => 0.0,
        };
        c.play = Some(Play { thing: k, kind: kind.to_string(), riders: Vec::new(), t: 0.0, stay });
        log::debug!("chat {id}: {i} goes to the {kind}");
        chats.push(c);
    }

    pub(super) fn run_play(&mut self, c: &mut Chat, dt: f32) -> bool {
        let Some(mut p) = c.play.take() else { return false };
        if p.kind == "down" {
            let i = c.members[0];
            let h = self.height_of(i);
            let now = self.walk_ref(i).air * h * 0.5;
            p.t += dt;
            if now <= 1.0 || p.t > 12.0 {
                self.let_down(i);
                return false;
            }
            let unit = self.park_unit(self.extras.len() + 1);
            let next = (now - 1.5 * unit * dt).max(0.0);
            let wk = self.walk_of(i);
            wk.air = next / (h * 0.5);
            if let Some(r) = wk.social.as_mut() {
                r.perched = wk.air > 0.0;
            }
            c.play = Some(p);
            return true;
        }
        let park = park_now();
        let Some(thing) = park.things.get(p.thing).filter(|t| t.kind == p.kind).filter(|t| governor::place_workspace(&t.place).is_some()).cloned() else {
            for &i in &c.members.clone() {
                self.let_down(i);
            }
            return false;
        };
        p.riders.retain(|r| c.members.contains(&r.i));
        for &i in &c.members {
            if !p.riders.iter().any(|r| r.i == i) {
                let want = 2 + (self.walk.rand() * 3.0) as u8;
                p.riders.push(Rider { i, phase: 0, t: 0.0, n: 0, want, fell: false });
            }
        }
        if p.riders.is_empty() {
            return false;
        }
        p.t += dt;
        let unit = self.park_unit_at(&thing, self.extras.len() + 1);
        let mid = self.thing_px(&thing);
        let done = match thing.kind.as_str() {
            "slide" => self.play_slide(c, &mut p, &thing, mid, unit, dt),
            "swing" => self.play_swing(c, &mut p, &thing, mid, unit, dt),
            "seesaw" => self.play_seesaw(c, &mut p, &thing, mid, unit, dt),
            "chair" | "bench" => self.play_sit(c, &mut p, &thing, mid, unit, dt),
            "table" => self.play_table(c, &mut p, &thing, mid, unit, dt),
            "sandbox" => self.play_sandbox(c, &mut p, &thing, mid, unit),
            _ => true,
        };
        if done {
            for &i in &c.members.clone() {
                self.let_down(i);
                self.leave(i, c.id);
            }
            c.members.clear();
            return false;
        }
        c.play = Some(p);
        !c.members.is_empty()
    }

    fn get_off(&mut self, c: &mut Chat, p: &mut Play, i: usize) {
        self.let_down(i);
        p.riders.retain(|r| r.i != i);
        c.members.retain(|&m| m != i);
        self.leave(i, c.id);
    }

    /// Up the ladder one at a time, down the modular tube path, happy hop at the exit.
    fn play_slide(&mut self, c: &mut Chat, p: &mut Play, t: &Thing, mid: f32, unit: f32, dt: f32) -> bool {
        let layout = park::slide_layout(t);
        let top = layout.top * unit;
        let dir0 = if t.flip { -1.0 } else { 1.0 };
        let ladder = mid + layout.ladder_x * unit;
        let lip = mid + layout.lip_x * unit;
        let busy = p.riders.iter().any(|r| matches!(r.phase, 1 | 2));
        let mut off = Vec::new();
        let mut queue = 0.0;
        for k in 0..p.riders.len() {
            let mut r = p.riders[k];
            r.t += dt;
            match r.phase {
                0 => {
                    let spot = self.clamp_in_place(t, r.i, unit, ladder - dir0 * (0.35 + queue * 0.45) * unit);
                    queue += 1.0;
                    if self.walk_to_2d(r.i, spot, t.y, 0.16) && queue <= 1.0 && !busy {
                        r.phase = 1;
                        r.t = 0.0;
                    }
                }
                1 => {
                    let climb = 0.55 + (layout.top * 0.80).min(3.2);
                    let u = (r.t / climb).min(1.0);
                    let up = u * u * (3.0 - 2.0 * u);
                    self.set_face(r.i, dir0 < 0.0);
                    self.hold_at(r.i, ladder, t.y, top * up);
                    if u >= 1.0 {
                        r.phase = 2;
                        r.t = 0.0;
                    }
                }
                2 => {
                    let u = (r.t / 0.5).min(1.0);
                    self.hold_at(r.i, ladder + (lip - ladder) * u, t.y, top);
                    if let Some(sr) = self.walk_of(r.i).social.as_mut() {
                        sr.tuck = Some(1.0 - 0.28 * u);
                    }
                    if r.t > 0.7 {
                        self.set_happy(r.i, true);
                        r.phase = 3;
                        r.t = 0.0;
                    }
                }
                3 => {
                    let (u, s) = slide_frac(&layout, r.t);
                    let (sx, sh, tx, _th) = layout.at(s);
                    let x = mid + sx * unit;
                    let y = sh * unit;
                    if tx.abs() > 0.08 {
                        self.set_face(r.i, tx < 0.0);
                    }
                    self.hold_at(r.i, x, t.y, y);
                    let h = self.height_of(r.i);
                    let surface = self.actor_surface_w(r.i);
                    slide_phase3_step(self.walk_of(r.i), &mut r, &layout, s, u, sh, tx, unit, h, surface, dt);
                }
                _ => {
                    let wk = self.walk_ref(r.i);
                    let settled = !matches!(wk.act, Act::Fall | Act::Planted)
                        && !wk.rolling
                        && wk.glide_vx.abs() <= 0.001;
                    if r.t > 1.0 && settled && self.arrived(r.i) {
                        if r.n >= r.want {
                            off.push(r.i);
                        } else {
                            r.phase = 0;
                            r.t = 0.0;
                            r.fell = false;
                        }
                    }
                }
            }
            p.riders[k] = r;
        }
        for i in off {
            self.get_off(c, p, i);
        }
        c.members.is_empty()
    }

    /// Back and forth, higher and higher; a friend gives a push from behind, then they swap.
    fn play_swing(&mut self, _c: &mut Chat, p: &mut Play, t: &Thing, mid: f32, unit: f32, dt: f32) -> bool {
        let top = park::SWING_TOP * unit;
        let rope = park::SWING_ROPE * unit;
        let rider = p.riders[0].i;
        let pusher = p.riders.get(1).map(|r| r.i);
        let mut r = p.riders[0];
        r.t += dt;
        match r.phase {
            0 => {
                let target = self.clamp_in_place(t, rider, unit, mid);
                if self.walk_to_2d(rider, target, t.y, 0.16) {
                    r.phase = 1;
                    r.t = 0.0;
                    r.want = 12 + (self.walk.rand() * 8.0) as u8;
                }
            }
            1 => {
                let len = r.want as f32;
                let most = if pusher.is_some() { 0.85 } else { 0.6 };
                let grow = (r.t / 4.0).min(1.0);
                let fade = ((len - r.t) / 3.0).clamp(0.0, 1.0);
                let amp = most * grow * fade;
                let a = amp * (r.t * std::f32::consts::TAU / 2.4).sin();
                self.set_face(rider, false);
                self.set_happy(rider, amp > 0.2);
                self.hold_at(rider, mid + rope * a.sin(), t.y, top - rope * a.cos());
                if let Some(q) = pusher {
                    let behind = self.clamp_in_place(t, q, unit, mid - 0.95 * unit);
                    if self.walk_to_2d(q, behind, t.y, 0.16) {
                        self.set_face(q, false);
                        let back = (r.t * std::f32::consts::TAU / 2.4).sin() < -0.95 * grow;
                        if back && self.walk.rand() < dt * 6.0 {
                            self.walk_of(q).gesture = true;
                            self.walk_of(q).start_bob(tn().jump_height * 0.1);
                        }
                    }
                }
                if r.t >= len {
                    self.let_down(rider);
                    let wk = self.walk_of(rider);
                    wk.start_jump(tn().jump_height * 0.5);
                    let secs = tn().laugh_time * 0.6;
                    wk.laugh(secs);
                    r.phase = 2;
                    r.t = 0.0;
                }
            }
            _ => {
                if r.t > 1.2 {
                    p.riders[0] = r;
                    if let Some(q) = pusher.filter(|_| r.n == 0) {
                        p.riders.swap(0, 1);
                        p.riders[0] = Rider { i: q, phase: 0, t: 0.0, n: 1, want: 0, fell: false };
                        p.riders[1].n = 1;
                        return false;
                    }
                    return true;
                }
            }
        }
        p.riders[0] = r;
        false
    }

    /// One at each end, up and down; alone it waits a while for someone.
    fn play_seesaw(&mut self, _c: &mut Chat, p: &mut Play, t: &Thing, mid: f32, unit: f32, _dt: f32) -> bool {
        let half = park::SEESAW_HALF * unit * 0.92;
        let pivot = park::SEESAW_PIVOT * unit;
        let tilt = park::SEESAW_TILT * unit;
        let ends = [mid - half, mid + half];
        let mut all_on = true;
        for k in 0..p.riders.len().min(2) {
            let i = p.riders[k].i;
            let end = self.clamp_in_place(t, i, unit, ends[k]);
            if p.riders[k].phase == 0 {
                if self.walk_to_2d(i, end, t.y, 0.16) {
                    p.riders[k].phase = 1;
                } else {
                    all_on = false;
                }
            }
        }
        let pair = p.riders.len() >= 2 && all_on;
        if !pair {
            for k in 0..p.riders.len().min(2) {
                if p.riders[k].phase == 1 {
                    let i = p.riders[k].i;
                    self.set_face(i, k == 1);
                    self.hold_at(i, ends[k], t.y, pivot - tilt);
                }
            }
            return p.t > 14.0;
        }
        let sway = (p.t * 2.2).sin() * tilt;
        for (k, d) in [-sway, sway].into_iter().enumerate() {
            let i = p.riders[k].i;
            self.set_face(i, k == 1);
            self.set_happy(i, d > tilt * 0.35);
            self.hold_at(i, ends[k], t.y, pivot + d);
        }
        p.t > p.stay + 10.0
    }

    /// Sits down on a chair or bench (two on a bench turn to chat).
    fn play_sit(&mut self, _c: &mut Chat, p: &mut Play, t: &Thing, mid: f32, unit: f32, dt: f32) -> bool {
        let seats: Vec<f32> = if t.kind == "bench" { vec![mid - 0.25 * unit, mid + 0.25 * unit] } else { vec![mid] };
        let seat = park::SEAT * unit;
        let bench = t.kind == "bench";
        let up = if bench { seat * 0.75 } else { seat + 0.02 * unit };
        for k in 0..p.riders.len().min(seats.len()) {
            let i = p.riders[k].i;
            let spot = seats[k];
            if p.riders[k].phase == 0 {
                let walk_x = self.clamp_in_place(t, i, unit, spot);
                if self.walk_up_at(i, walk_x, t.y, up, dt) {
                    p.riders[k].phase = 1;
                    self.walk_of(i).start_bob(tn().jump_height * 0.15);
                }
                continue;
            }
            self.hold_at(i, spot, t.y, up);
            let other = p.riders.iter().map(|r| r.i).find(|&o| o != i);
            let wk = self.walk_of(i);
            wk.sink = if bench { tn().sit_sink } else { 0.0 };
            if let Some(o) = other {
                if wk.rand() < 0.004 {
                    wk.gesture = true;
                }
                self.face(i, o);
            }
        }
        p.t > p.stay
    }

    /// Round the table, sitting on nearby chairs or standing beside it and chatting.
    fn play_table(&mut self, c: &mut Chat, p: &mut Play, t: &Thing, mid: f32, unit: f32, dt: f32) -> bool {
        let seats = self.table_seats(t, mid, unit);
        for k in 0..p.riders.len() {
            let i = p.riders[k].i;
            let Some(&(x, up)) = seats.get(k) else { continue };
            if p.riders[k].phase == 0 {
                let walk_x = self.clamp_in_place(t, i, unit, x);
                if self.walk_up_at(i, walk_x, t.y, up, dt) {
                    p.riders[k].phase = 1;
                    self.set_face(i, x > mid);
                }
            } else if up > 0.0 {
                self.hold_at(i, x, t.y, up);
            }
        }
        if c.members.len() >= 2 {
            self.chatter(c, dt);
        } else if p.t > 15.0 {
            return true;
        }
        p.t > p.stay
    }

    /// Chairs right beside `t` first, then standing spots around the table.
    pub(super) fn table_seats(&self, t: &Thing, mid: f32, unit: f32) -> Vec<(f32, f32)> {
        let park = park_now();
        let tables: Vec<f32> = park.things.iter().filter(|o| o.kind == "table" && o.place == t.place).map(|o| self.thing_px(o)).collect();
        let mut chairs: Vec<f32> = park
            .things
            .iter()
            .filter(|c| c.kind == "chair" && c.place == t.place && (c.y - t.y).abs() < 0.03)
            .map(|c| self.thing_px(c))
            .filter(|x| (x - mid).abs() < 1.3 * unit)
            .filter(|x| tables.iter().all(|o| (x - o).abs() >= (x - mid).abs() - 0.5))
            .collect();
        chairs.sort_by(|a, b| (a - mid).abs().total_cmp(&(b - mid).abs()));
        let mut seats: Vec<(f32, f32)> = chairs.into_iter().take(4).map(|x| (x, park::SEAT * unit + 0.02 * unit)).collect();
        let others: Vec<f32> = tables.iter().copied().filter(|o| (o - mid).abs() > 1.0).collect();
        for x in [mid - 0.85 * unit, mid + 0.85 * unit, mid - 1.3 * unit, mid + 1.3 * unit] {
            if seats.len() >= 4 {
                break;
            }
            let clear = !seats.iter().any(|s| (s.0 - x).abs() < 0.4 * unit) && others.iter().all(|o| (o - x).abs() > 0.85 * unit);
            if clear {
                seats.push((x, 0.0));
            }
        }
        seats
    }

    /// Walks up to the sandbox across the meadow and plays in the sand.
    fn play_sandbox(&mut self, _c: &mut Chat, p: &mut Play, t: &Thing, mid: f32, unit: f32) -> bool {
        let half = 0.55 * unit;
        for k in 0..p.riders.len() {
            let mut r = p.riders[k];
            if r.phase == 0 {
                let spot = self.clamp_in_place(t, r.i, unit, mid + ((k as f32 % 2.0) * 2.0 - 1.0) * 0.28 * unit);
                if self.walk_to_2d(r.i, spot, t.y, 0.14) {
                    r.phase = 1;
                    r.t = 3.0 + self.walk.rand() * 3.0;
                }
            } else {
                r.t -= 1.0 / 60.0;
                if r.t <= 0.0 && self.arrived(r.i) {
                    let raw = mid - half + self.walk.rand() * half * 2.0;
                    let spot = self.clamp_in_place(t, r.i, unit, raw);
                    self.go_facing(r.i, spot, 0.12);
                    r.t = 4.0 + self.walk.rand() * 4.0;
                }
                if self.arrived(r.i) {
                    let wk = self.walk_of(r.i);
                    wk.sink = tn().sit_sink * 1.2;
                    if wk.rand() < 0.025 {
                        wk.gesture = true;
                    }
                    let other = p.riders.iter().map(|o| o.i).find(|&o| o != r.i);
                    self.set_happy(r.i, other.is_some());
                }
            }
            p.riders[k] = r;
        }
        p.t > p.stay
    }
}

/// Picked by a right double click: (who, when).
static PICKED: Mutex<Option<(usize, Instant)>> = Mutex::new(None);
static ORDERS_SEEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl CharacterOverlay {
    /// Right double click on a character picks it; then a right double click
    /// on the park grass walks it there in 2D, or on a park item sends it to play.
    pub(super) fn right_orders(&mut self, chats: &mut Vec<Chat>, n: usize) {
        use std::sync::atomic::Ordering::Relaxed;
        let expired = {
            let mut picked = PICKED.lock().unwrap();
            let gone = picked.filter(|p| p.1.elapsed().as_secs_f32() > 5.0).map(|p| p.0);
            if gone.is_some() {
                *picked = None;
            }
            gone
        };
        if let Some(i) = expired.filter(|&i| i < n) {
            tell(&format!("{} is no longer picked", self.character_name(i)));
        }
        let now = crate::backend::pointer::RIGHT_DOUBLE.load(Relaxed);
        if ORDERS_SEEN.swap(now, Relaxed) == now || now == 0 {
            return;
        }
        let m = self.screen_mouse;
        let hit = (0..n).find(|&i| {
            let a = self.actor_ref(i);
            let r = a.body_box();
            let r = a.zoom.map_or(r, |z| z.to_screen(r));
            a.visible && m[0] >= r[0] && m[0] <= r[2] && m[1] >= r[1] && m[1] <= r[3]
        });
        if let Some(i) = hit {
            *PICKED.lock().unwrap() = Some((i, Instant::now()));
            let wk = self.walk_of(i);
            if matches!(wk.act, Act::Idle | Act::Walk) && wk.air <= 0.0 {
                wk.start_jump(tn().jump_height * 0.4);
            }
            log::debug!("orders: {i} picked");
            tell(&format!("{} is picked: right double click where it should go", self.character_name(i)));
            return;
        }
        let Some((i, at)) = PICKED.lock().unwrap().take() else { return };
        if at.elapsed().as_secs_f32() > 5.0 || i >= n {
            return;
        }
        let a = self.actor_ref(i);
        if !a.visible {
            return;
        }
        let p = a.zoom.map_or(m, |z| z.to_scene(m));
        let px = p[0] + a.own_view[0] * self.view_w * 0.5;
        for c in chats.iter_mut() {
            if c.members.contains(&i) || c.watchers.contains(&i) {
                c.members.retain(|&x| x != i);
                c.watchers.retain(|&x| x != i);
                self.let_down(i);
                self.leave(i, c.id);
            }
        }
        let wk = self.walk_ref(i);
        if !matches!(wk.act, Act::Idle | Act::Walk | Act::Rest) || wk.race.is_some() || wk.trip.is_some() {
            tell(&format!("{} can't go right now", self.character_name(i)));
            return;
        }
        if wk.act == Act::Rest {
            let wk = self.walk_of(i);
            wk.rest = None;
            wk.act = Act::Idle;
            wk.sink = 0.0;
            wk.rot = 0.0;
        }
        let bottom = ground_bottom(self, i);
        if let Some(place) = governor::place_at(self.walk_ref(i).ws, px) {
            let park = park_now();
            let base_unit = self.park_unit(n);
            let on = park.things.iter().enumerate().filter(|(_, t)| park::room(&t.kind) > 0 && t.place == place.place).find(|(_, t)| {
                let u = base_unit * park::depth_scale(t.y);
                let base = bottom - t.y * place.h;
                (px - self.thing_px(t)).abs() <= park::width(t) * u * 0.5 && p[1] >= base - park::height(t).max(2.0) * u && p[1] <= base + 0.3 * u
            });
            if let Some((k, t)) = on {
                let kind = t.kind.clone();
                log::debug!("orders: {i} sent to the {kind}");
                let label = park::recipe(&kind).map_or(kind.clone(), |r| r.1.to_lowercase());
                tell(&format!("{} goes to the {label}", self.character_name(i)));
                self.start_play(chats, i, k, &kind);
                return;
            }
            let y_norm = ((bottom - p[1]) / place.h.max(1.0)).clamp(0.0, park::MEADOW);
            let here = self.strip_px(i);
            let surface = self.actor_ref(i).surface[0].max(1.0);
            let wk = self.walk_of(i);
            wk.target = wk.x + (px - here) * 2.0 / surface;
            wk.park_target_y = y_norm;
            wk.act = Act::Walk;
            wk.timer = 3.0;
            log::debug!("orders: {i} walks to ({px:.0}, {y_norm:.2})");
            tell(&format!("{} is on its way", self.character_name(i)));
            return;
        }
        let here = self.strip_px(i);
        let surface = self.actor_ref(i).surface[0].max(1.0);
        let wk = self.walk_of(i);
        wk.target = wk.x + (px - here) * 2.0 / surface;
        wk.act = Act::Walk;
        wk.timer = 3.0;
        log::debug!("orders: {i} walks to {px:.0}");
        tell(&format!("{} is on its way", self.character_name(i)));
    }

    /// Swing ropes and seesaw plank drawn on the character overlay layer.
    pub(in crate::render::character) fn park_ropes(&self) -> Vec<([f32; 4], usize, f32, bool)> {
        let park = park_now();
        let n = self.extras.len() + 1;
        let mut out = Vec::new();
        let feet = |this: &Self, i: usize| {
            let rect = if i == 0 { this.hit_rect } else { this.extras[i - 1].hit_rect };
            let a = this.actor_ref(i);
            let h = (rect[3] - rect[1]).max(1.0);
            let soles = rect[1] + h * a.opaque[2];
            (rect, a.own_view[0] * this.view_w * 0.5, soles, soles + a.walk.air * h * 0.5)
        };
        for c in self.chats.iter().filter(|c| c.stage == Stage::Play) {
            let Some(p) = c.play.as_ref() else { continue };
            let Some(t) = park.things.get(p.thing) else { continue };
            let unit = self.park_unit_at(t, n);
            if p.kind == "swing" {
                let Some(r) = p.riders.first().filter(|r| r.phase == 1) else { continue };
                let (rect, off, soles, ground) = feet(self, r.i);
                let top = ground - park::SWING_TOP * unit;
                let px = self.thing_px(t) - off;
                let (hx, hy) = ((rect[0] + rect[2]) * 0.5, soles);
                for side in [-1.0, 1.0] {
                    let d = side * 0.18 * unit;
                    out.push(([px + d, top, hx + d, hy], r.i, 3.0, true));
                }
                out.push(([hx - 0.24 * unit, hy + 0.03 * unit, hx + 0.24 * unit, hy + 0.03 * unit], r.i, 0.035 * unit, false));
            } else if p.kind == "seesaw" && seesaw_rocking(p) {
                let (a, b) = (p.riders[0].i, p.riders[1].i);
                let (_, offa, ya, _) = feet(self, a);
                let (_, _, yb, _) = feet(self, b);
                let half = park::SEESAW_HALF * unit;
                let mid = self.thing_px(t) - offa;
                let k = half / (half * 0.92);
                let (y0, y1) = ((ya + yb) * 0.5 + (ya - yb) * 0.5 * k, (ya + yb) * 0.5 - (ya - yb) * 0.5 * k);
                out.push(([mid - half, y0 + 4.0, mid + half, y1 + 4.0], a, 0.06 * unit, false));
            }
        }
        out
    }

    /// Slide tube occlusion/overdraw chord data for `actor` while on or exiting a slide.
    pub(in crate::render::character) fn actor_slide_render(&self, actor: usize) -> Option<park::SlideRenderData> {
        let (thing, rider) = self.chats.iter().find_map(|c| {
            if c.stage != Stage::Play {
                return None;
            }
            let p = c.play.as_ref().filter(|p| p.kind == "slide")?;
            let r = p.riders.iter().find(|r| {
                r.i == actor && (r.phase == 2 || r.phase == 3 || (r.phase == 4 && !r.fell && r.t < 0.45))
            })?;
            Some((p.thing, *r))
        })?;
        let park = park_now();
        let t = park.things.get(thing)?;
        let layout = park::slide_layout(t);
        let n = self.extras.len() + 1;
        let unit = self.park_unit_at(t, n);
        let a = self.actor_ref(actor);
        let off = a.own_view[0] * self.view_w * 0.5;
        let mid_px = self.thing_px(t) - off;
        let gy_px = a.ground_px;
        let frac = match rider.phase {
            2 => 0.0,
            3 => slide_frac(&layout, rider.t).1,
            _ => 1.0,
        };
        Some(layout.chords(mid_px, gy_px, unit, layout.seg_at(frac)))
    }

    /// Tells the park window which swings/seesaws are currently busy.
    pub(super) fn park_tell_world(&self) {
        let busy: Vec<usize> = self
            .chats
            .iter()
            .filter(|c| c.stage == Stage::Play)
            .filter_map(|c| c.play.as_ref())
            .filter(|p| (p.kind == "swing" && p.riders.first().is_some_and(|r| r.phase == 1)) || (p.kind == "seesaw" && seesaw_rocking(p)))
            .map(|p| p.thing)
            .collect();
        let mut c = CACHE.lock().unwrap();
        if c.world_written.is_some_and(|t| t.elapsed().as_secs_f32() < 0.2) {
            return;
        }
        let world = park::World { busy };
        if c.world_last.as_ref() == Some(&world) {
            return;
        }
        if let Ok(text) = serde_json::to_string(&world) {
            let _ = crate::util::atomic_write(&park::world_path(), text.as_bytes());
        }
        c.world_written = Some(Instant::now());
        c.world_last = Some(world);
    }

    /// Every frame for Anima Park: game time, mode-3 visits, and gentle idle play nudges.
    pub(super) fn park_tick(&mut self, chats: &mut Vec<Chat>, dt: f32, n: usize) {
        for i in 0..n {
            let idle = self.place_here(i).is_some()
                && self.walk_ref(i).at_leisure()
                && self.walk_ref(i).act == Act::Idle
                && self.walk_ref(i).social.is_none();
            let wk = self.walk_of(i);
            wk.idle_t = if idle { wk.idle_t + dt } else { 0.0 };
        }
        self.game_time(chats, n);
        self.visit_tick(chats, dt, n);
        for i in 0..n {
            if self.walk_ref(i).idle_t > 12.0 && self.free_for(i, Want::Play) {
                self.walk_of(i).idle_t = 0.0;
                self.play_for(chats, i, n, true);
            }
        }
    }
}

fn tell(text: &str) {
    let _ = std::process::Command::new("notify-send")
        .args(["-a", "Anima", "-t", "2500", "-h", "string:x-canonical-private-synchronous:anima-orders", "Anima", text])
        .spawn()
        .map(|mut c| std::thread::spawn(move || c.wait()));
}

/// Clears ride-specific pose and visibility overrides while keeping `park_y` intact.
fn clear_ride_state(wk: &mut WalkState) {
    wk.air = 0.0;
    wk.sink = 0.0;
    if let Some(r) = wk.social.as_mut() {
        r.perched = false;
        r.happy = false;
        r.lean = 0.0;
        r.alpha = None;
        r.tuck = None;
    }
}

/// Returns `(u, s)` progress along a slide for elapsed phase-3 time `t`.
fn slide_frac(layout: &park::SlideLayout, t: f32) -> (f32, f32) {
    let dur = 0.35 + 0.48 * layout.total_len;
    let u = (t / dur).min(1.0);
    (u, u * (0.35 + 0.65 * u))
}

/// Per-second probability of losing balance on a slide segment at `s`.
fn slide_fall_rate(cover: park::TubeCover, seg: park::TubeSeg) -> f32 {
    match cover {
        park::TubeCover::Open => match seg {
            park::TubeSeg::Turn => 0.42,
            park::TubeSeg::Straight => 0.24,
        },
        park::TubeCover::Translucent | park::TubeCover::Opaque => 0.0,
    }
}

/// Launches a rider who slipped off an open slide into an airborne `Act::Fall` tumble.
fn launch_slide_fall(wk: &mut WalkState, h: f32, surface: f32, sh: f32, tx: f32, exit_dir: f32, unit: f32) {
    clear_ride_state(wk);
    let dir = if tx.abs() > 0.05 { tx.signum() } else { exit_dir };
    let speed_scale = (0.55 + sh.max(0.0) * 0.25).clamp(0.45, 1.35);
    wk.act = Act::Fall;
    wk.glide_vx = 0.0;
    wk.air = (sh * unit / (h.max(1.0) * 0.5)).max(0.2);
    wk.vx = dir * speed_scale * unit * 2.0 / surface.max(1.0);
    wk.vy = 0.8 + wk.rand() * 0.6;
    wk.tumble = -dir * (3.5 + wk.rand() * 3.0);
    wk.feet = false;
    wk.grip = false;
}

/// Launches a rider off the bottom of the slide with preserved horizontal exit momentum.
fn launch_slide_exit(wk: &mut WalkState, layout: &park::SlideLayout, unit: f32, surface: f32) {
    clear_ride_state(wk);
    if let Some(sr) = wk.social.as_mut() {
        sr.face_left = layout.exit_dir < 0.0;
    }
    let exit_vx = (1.25 + layout.top * 0.55).clamp(1.35, 3.0) * unit * 2.0 / surface.max(1.0);
    wk.start_jump(tn().jump_height * 0.45);
    wk.glide_vx = layout.exit_dir * exit_vx;
    let secs = tn().laugh_time * 0.8;
    wk.laugh(secs);
}

/// Updates slide lean, tuck (`Social::tuck`), mid-slide fall checks, and exit launch.
#[allow(clippy::too_many_arguments)]
fn slide_phase3_step(
    wk: &mut WalkState,
    r: &mut Rider,
    layout: &park::SlideLayout,
    s: f32,
    u: f32,
    sh: f32,
    tx: f32,
    unit: f32,
    h: f32,
    surface: f32,
    dt: f32,
) {
    if let Some(sr) = wk.social.as_mut() {
        sr.lean = (tx * 0.26).clamp(-0.26, 0.26);
        sr.alpha = None;
        sr.tuck = Some(0.72);
    }
    let fall_rate = if !r.fell && (0.20..=0.88).contains(&s) {
        slide_fall_rate(layout.cover_at(s), layout.seg_kind_at(s))
    } else {
        0.0
    };
    if fall_rate > 0.0 && wk.rand() < fall_rate * dt {
        r.fell = true;
        r.phase = 4;
        r.t = 0.0;
        r.n += 1;
        launch_slide_fall(wk, h, surface, sh, tx, layout.exit_dir, unit);
    } else if u >= 1.0 {
        r.phase = 4;
        r.t = 0.0;
        r.n += 1;
        launch_slide_exit(wk, layout, unit, surface);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_place_keeps_corner_queues_inside_window() {
        assert_eq!(clamp_place(0.0, 1920.0, 55.0, 1979.0), 1865.0);
        assert_eq!(clamp_place(0.0, 1920.0, 55.0, -30.0), 55.0);
        assert_eq!(clamp_place(0.0, 1920.0, 55.0, 960.0), 960.0);
        assert_eq!(clamp_place(100.0, 80.0, 60.0, 200.0), 144.0);
    }

    #[test]
    fn stage_play_allows_slope_lean() {
        assert!(super::super::life::allows(Stage::Play).lean);
    }

    #[test]
    fn mid_slide_fall_sets_fall_tumble_and_clears_alpha() {
        let mut wk = WalkState {
            seed: 4242,
            park_y: 0.25,
            social: Some(Social { chat: 1, perched: true, happy: true, alpha: Some(0.36), tuck: Some(0.72), ..Default::default() }),
            ..Default::default()
        };
        launch_slide_fall(&mut wk, 180.0, 1920.0, 1.2, 0.8, 1.0, 120.0);
        assert_eq!(wk.act, Act::Fall);
        assert_eq!(wk.park_y, 0.25);
        assert!(wk.air > 0.2);
        assert!(wk.vx > 0.0);
        assert!((0.8..=1.4).contains(&wk.vy));
        assert!((3.5..=6.5).contains(&wk.tumble.abs()));
        assert!(!wk.feet && !wk.grip);
        let sr = wk.social.unwrap();
        assert!(sr.alpha.is_none());
        assert!(sr.tuck.is_none());
        assert!(!sr.perched);
        assert!(!sr.happy);
    }

    #[test]
    fn slide_fall_rate_depends_on_cover_and_segment() {
        let open_straight = slide_fall_rate(park::TubeCover::Open, park::TubeSeg::Straight);
        let open_turn = slide_fall_rate(park::TubeCover::Open, park::TubeSeg::Turn);
        assert!(open_straight > 0.0);
        assert!(open_turn > open_straight);
        assert_eq!(slide_fall_rate(park::TubeCover::Translucent, park::TubeSeg::Turn), 0.0);
        assert_eq!(slide_fall_rate(park::TubeCover::Opaque, park::TubeSeg::Straight), 0.0);
    }

    #[test]
    fn slide_phase3_keeps_full_alpha_tucks_and_launches_exit_glide() {
        let thing: Thing = serde_json::from_str(r#"{"kind":"slide","x":0.5,"pieces":3}"#).unwrap();
        let layout = park::slide_layout(&thing);
        let mut wk = WalkState {
            seed: 7,
            social: Some(Social { chat: 1, perched: true, alpha: Some(0.36), ..Default::default() }),
            ..Default::default()
        };
        let mut r = Rider { i: 0, phase: 3, t: 0.0, n: 0, want: 2, fell: false };

        for s in [0.1, 0.3, 0.5, 0.7, 0.9] {
            let (_, sh, tx, _) = layout.at(s);
            slide_phase3_step(&mut wk, &mut r, &layout, s, s, sh, tx, 120.0, 180.0, 1920.0, 0.0);
            let sr = wk.social.unwrap();
            assert!(sr.alpha.is_none());
            assert_eq!(sr.tuck, Some(0.72));
        }

        // Reaching exit sets Jump + glide_vx + clears alpha and tuck
        let (_, sh, tx, _) = layout.at(1.0);
        slide_phase3_step(&mut wk, &mut r, &layout, 1.0, 1.0, sh, tx, 120.0, 180.0, 1920.0, 1.0 / 60.0);
        assert_eq!(r.phase, 4);
        assert_eq!(r.n, 1);
        assert_eq!(wk.act, Act::Jump);
        assert!(wk.glide_vx * layout.exit_dir > 0.0);
        assert!(wk.giggle > 0.0);
        let sr = wk.social.unwrap();
        assert!(sr.alpha.is_none());
        assert!(sr.tuck.is_none());
    }
}
