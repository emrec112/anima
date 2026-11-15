//! Each character lives on one niri workspace: drawn where niri draws that
//! workspace (sliding with it, shrunk in the overview), and now and then
//! moving up or down to the next one. The more at home it feels, and the
//! better it knows those around it, the more it wants to see who's elsewhere.

use std::sync::atomic::Ordering::Relaxed;

use crate::backend::desktop;
use crate::backend::governor::{self, Space};
use crate::render::FrameInfo;
use crate::render::tuning::get as tn;
use super::walk::{Act, WalkState};
use super::{CharacterOverlay, SceneZoom};

// half-heights: far enough to be right out of sight below the floor
const BELOW: f32 = 2.3;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Leg {
    Sinking,
    Rising,
    Emerging,
    // ends a fullscreen visit: down and out of sight
    Vanishing,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Trip {
    to: u64,
    leg: Leg,
    t: f32,
}

#[derive(Default)]
pub(super) struct Spaces {
    // seconds until anyone may set off again
    quiet: f32,
    list: Vec<Space>,
    pub(super) active: u64,
    // scroll position of each workspace when last looked at
    views: Vec<(u64, [f32; 2])>,
    tick: f32,
    // how much each workspace has been looked at lately, seconds
    used: Vec<(u64, f32)>,
    // a fullscreen app is up: seconds to the next visit
    pub(super) fs_on: bool,
    // where each character was (by name): workspace index, kept between runs
    places: Option<std::collections::HashMap<String, usize>>,
    places_save: f32,
    locked: bool,
    fs_wait: f32,
}

impl WalkState {
    pub(super) fn set_off(&mut self, to: u64, down: bool) {
        self.trip = Some(Trip { to, leg: if down { Leg::Sinking } else { Leg::Rising }, t: 0.0 });
        self.act = Act::Travel;
        self.target = self.x;
        self.here_t = 0.0;
    }

    /// Comes up out of the floor for a visit.
    fn come_out(&mut self, stay: f32) {
        self.backstage = false;
        self.visiting = true;
        self.visit_left = stay;
        self.trip = Some(Trip { to: self.ws, leg: Leg::Emerging, t: 0.0 });
        self.act = Act::Travel;
        self.sink = BELOW;
        self.target = self.x;
    }

    fn go_back(&mut self) {
        self.trip = Some(Trip { to: self.ws, leg: Leg::Vanishing, t: 0.0 });
        self.act = Act::Travel;
    }

    /// `top`: the height at which it's out of sight above the screen.
    fn trip_tick(&mut self, dt: f32, top: f32) {
        if self.act != Act::Travel {
            self.trip = None;
            return;
        }
        let Some(mut t) = self.trip else {
            self.act = Act::Idle;
            return;
        };
        let speed = tn().ws_speed;
        t.t += dt;
        match t.leg {
            Leg::Sinking => {
                self.sink += speed * dt;
                if self.sink >= BELOW {
                    // through the floor and in from the top of the one below
                    self.ws = t.to;
                    self.sink = 0.0;
                    self.air = top;
                    self.vx = 0.0;
                    self.vy = 0.0;
                    self.leaped = self.rand() < 0.7;
                    self.act = Act::Fall;
                    self.trip = None;
                    self.newcomer = true;
                    return;
                }
            }
            Leg::Vanishing => {
                self.sink += speed * dt;
                if self.sink >= BELOW {
                    self.sink = 0.0;
                    self.act = Act::Idle;
                    self.timer = 1.0;
                    self.trip = None;
                    self.backstage = true;
                    self.visiting = false;
                    return;
                }
            }
            Leg::Rising => {
                self.air += speed * dt;
                self.rot = (t.t * 14.0).sin() * 0.09;
                if self.air >= top {
                    // up and out, and up through the floor of the one above
                    self.ws = t.to;
                    self.air = 0.0;
                    self.rot = 0.0;
                    self.sink = BELOW;
                    t.leg = Leg::Emerging;
                }
            }
            Leg::Emerging => {
                self.sink -= speed * dt;
                if self.sink <= 0.0 {
                    self.sink = 0.0;
                    self.act = Act::Idle;
                    self.timer = 1.0;
                    self.trip = None;
                    self.newcomer = true;
                    return;
                }
            }
        }
        self.trip = Some(t);
    }
}

/// How many characters each workspace should have, from how much it's used
/// and how many windows it has. Everyone gets at least `floor` of the team.
fn fair_shares(stats: &[(f32, f32)], n: usize, floor: f32, use_weight: f32) -> Vec<usize> {
    let k = stats.len().max(1) as f32;
    let floor = floor.clamp(0.0, 1.0 / k);
    let (used, windows) = stats.iter().fold((0.0, 0.0), |t, s| (t.0 + s.0, t.1 + s.1));
    let part = |v: f32, total: f32| if total > 0.0 { v / total } else { 1.0 / k };
    let quota: Vec<f32> = stats
        .iter()
        .map(|s| {
            let score = use_weight * part(s.0, used) + (1.0 - use_weight) * part(s.1, windows);
            n as f32 * (floor + (1.0 - k * floor) * score)
        })
        .collect();
    let mut seats: Vec<usize> = quota.iter().map(|q| q.floor() as usize).collect();
    // the seats left over go to the biggest remainders
    let mut order: Vec<usize> = (0..quota.len()).collect();
    order.sort_by(|&a, &b| (quota[b].fract()).total_cmp(&quota[a].fract()));
    let given: usize = seats.iter().sum();
    for &i in order.iter().take(n.saturating_sub(given)) {
        seats[i] += 1;
    }
    // nobody's left with an empty desktop while there are enough to go round
    if n >= seats.len() {
        for i in 0..seats.len() {
            if seats[i] == 0 {
                if let Some(big) = (0..seats.len()).max_by_key(|&j| seats[j]) {
                    seats[big] -= 1;
                    seats[i] = 1;
                }
            }
        }
    }
    seats
}

fn holds(s: &Space, p: [f32; 2]) -> bool {
    p[0] >= s.rect[0] && p[0] <= s.rect[0] + s.rect[2] && p[1] >= s.rect[1] && p[1] <= s.rect[1] + s.rect[3]
}

impl CharacterOverlay {
    pub(super) fn actor_ref(&self, i: usize) -> &CharacterOverlay {
        if i == 0 { self } else { &self.extras[i - 1] }
    }

    fn actor_mut(&mut self, i: usize) -> &mut CharacterOverlay {
        if i == 0 { self } else { &mut self.extras[i - 1] }
    }

    /// Where this frame's workspaces are, for every character.
    pub(super) fn sync_spaces(&mut self, info: &FrameInfo) {
        let n = self.extras.len() + 1;
        let list = governor::workspaces().unwrap_or_default();
        let size = [info.width.max(1) as f32, info.height.max(1) as f32];
        let scale = info.scale.max(1.0);
        self.spaces.active = list.iter().find(|s| s.active).map_or(0, |s| s.id);
        let locked = super::locked();
        if locked != self.spaces.locked {
            self.spaces.locked = locked;
            let here = self.spaces.active;
            for i in 0..n {
                let wk = self.walk_of(i);
                if locked {
                    // all on the one screen, back home afterwards
                    wk.lock_home = Some(wk.ws);
                    if here != 0 {
                        wk.ws = here;
                    }
                    wk.backstage = false;
                    wk.visiting = false;
                    if wk.act == Act::Travel {
                        wk.trip = None;
                        wk.sink = 0.0;
                        wk.air = 0.0;
                        wk.act = Act::Idle;
                    }
                } else if let Some(home) = wk.lock_home.take() {
                    wk.ws = home;
                }
            }
        }
        let hosts: Vec<u64> = list.iter().filter(|s| s.active || s.has_windows).map(|s| s.id).collect();
        for i in 0..n {
            let a = self.actor_mut(i);
            a.screen_mouse = info.mouse;
            if list.is_empty() || locked {
                a.zoom = None;
                a.visible = !a.walk.backstage;
                a.walk.ws = 0;
            }
        }
        if !list.is_empty() && !locked {
            for i in 0..n {
                if !list.iter().any(|s| s.id == self.walk_ref(i).ws) {
                    // the park (cafe, farm) is home while it's open
                    let homes: Vec<u64> = place_homes().into_iter().filter(|p| list.iter().any(|s| s.id == *p)).collect();
                    if !homes.is_empty() {
                        let k = (self.walk_of(i).rand() * homes.len() as f32) as usize % homes.len();
                        self.walk_of(i).ws = homes[k];
                        continue;
                    }
                    // back where it was last time, or with its team
                    if let Some(ws) = self.home_for(i, &list, n) {
                        self.walk_of(i).ws = ws;
                        continue;
                    }
                    // whichever workspace is shortest of its share
                    let seats = self.seats(&hosts, n);
                    let lacking = |h: u64, want: usize| want as i64 - (0..n).filter(|&j| self.walk_ref(j).ws == h).count() as i64;
                    let to = hosts.iter().zip(&seats).max_by_key(|(h, w)| lacking(**h, **w)).map(|(h, _)| *h);
                    self.walk_of(i).ws = to.unwrap_or(list[0].id);
                }
                let ws = self.walk_ref(i).ws;
                let Some(s) = list.iter().find(|s| s.id == ws) else { continue };
                let [x, y, w, h] = s.rect;
                let on_screen = x * scale < size[0] && (x + w) * scale > 0.0 && y * scale < size[1] && (y + h) * scale > 0.0;
                let zoom = SceneZoom::new(s.rect, size, scale);
                let a = self.actor_mut(i);
                a.zoom = zoom;
                a.visible = on_screen && !a.walk.backstage;
            }
        }
        // workspaces sliding (a switch, the overview): characters go with them at once
        if self.spaces.list.iter().map(|s| (s.id, s.rect)).ne(list.iter().map(|s| (s.id, s.rect))) {
            super::scene_moved();
        }
        self.spaces.list = list;
    }

    /// Where `i` belongs when it first shows up: its place from last time.
    fn home_for(&mut self, i: usize, list: &[Space], n: usize) -> Option<u64> {
        if self.spaces.places.is_none() {
            let saved = std::fs::read_to_string(places_path()).ok().and_then(|t| serde_json::from_str(&t).ok());
            self.spaces.places = Some(saved.unwrap_or_default());
        }
        let name = self.character_name(i);
        if let Some(&k) = self.spaces.places.as_ref().and_then(|p| p.get(&name)) {
            return list.get(k.min(list.len().saturating_sub(1))).map(|s| s.id);
        }
        let _ = n;
        None
    }

    /// Writes who is on which workspace now and then.
    fn keep_places(&mut self, dt: f32, n: usize) {
        self.spaces.places_save -= dt;
        if self.spaces.places_save > 0.0 || super::locked() {
            return;
        }
        self.spaces.places_save = 20.0;
        let mut map = self.spaces.places.clone().unwrap_or_default();
        for i in 0..n {
            let ws = self.walk_ref(i).ws;
            if let Some(k) = self.spaces.list.iter().position(|s| s.id == ws) {
                map.insert(self.character_name(i), k);
            }
        }
        if self.spaces.places.as_ref() != Some(&map) {
            if let Ok(text) = serde_json::to_string(&map) {
                let _ = crate::util::atomic_write(&places_path(), text.as_bytes());
            }
            self.spaces.places = Some(map);
        }
    }

    /// Fair number of characters for each of `hosts`.
    fn seats(&self, hosts: &[u64], n: usize) -> Vec<usize> {
        let stats: Vec<(f32, f32)> = hosts
            .iter()
            .map(|&h| {
                let used = self.spaces.used.iter().find(|u| u.0 == h).map_or(0.0, |u| u.1);
                (used, governor::window_count(h) as f32)
            })
            .collect();
        fair_shares(&stats, n, tn().ws_floor, tn().ws_use_weight)
    }

    /// The strip's scroll position, as each character's workspace had it.
    pub(super) fn share_views(&mut self, view: [f32; 2], strip_ws: Option<u64>) {
        let active = self.spaces.active;
        // during a switch the strip and the workspace list change at different
        // moments: file the view under the workspace it was really read from
        let from = if active == 0 { 0 } else { strip_ws.unwrap_or(active) };
        if let Some(v) = self.spaces.views.iter_mut().find(|v| v.0 == from) {
            v.1 = view;
        } else {
            self.spaces.views.push((from, view));
        }
        let n = self.extras.len() + 1;
        for i in 0..n {
            let ws = self.walk_ref(i).ws;
            let fallback = if ws == active { view } else { [0.0, 0.0] };
            let own = self.spaces.views.iter().find(|v| v.0 == ws).map_or(fallback, |v| v.1);
            self.actor_mut(i).own_view = own;
        }
    }

    pub(super) fn run_spaces(&mut self, dt: f32, n: usize) {
        for i in 0..n {
            let top = self.actor_ref(i).top_air;
            self.walk_of(i).trip_tick(dt, top);
        }
        self.run_visits(dt, n);
        if self.spaces.list.is_empty() {
            return;
        }
        self.drop_between_spaces(n);
        self.keep_places(dt, n);
        for i in 0..n {
            if self.walk_ref(i).newcomer && self.walk_ref(i).at_leisure() {
                self.walk_of(i).newcomer = false;
                self.greet_newcomer(i, n);
            }
        }
        self.spaces.tick -= dt;
        if self.spaces.tick <= 0.0 && !self.spaces.fs_on && !super::locked() {
            self.spaces.tick = 1.0;
            self.roam(n);
        }
    }

    /// A fullscreen app we may visit: nearly everyone is out of sight, and
    /// now and then one to three come out and potter along the bottom.
    fn run_visits(&mut self, dt: f32, n: usize) {
        let on = desktop::FS_VISIT.load(Relaxed) && !super::locked();
        let here = self.spaces.active;
        if on != self.spaces.fs_on {
            self.spaces.fs_on = on;
            for i in 0..n {
                let wk = self.walk_of(i);
                wk.backstage = on && (here == 0 || wk.ws == here);
                wk.visiting = false;
                wk.visit_left = 0.0;
            }
            let roll = self.walk.rand();
            self.spaces.fs_wait = tn().fs_first * (0.5 + roll);
        }
        if !on {
            return;
        }
        for i in 0..n {
            let wk = self.walk_of(i);
            if wk.visiting {
                wk.fret = false;
                wk.newcomer = false;
                wk.visit_left -= dt;
                if wk.visit_left <= 0.0 && wk.at_leisure() {
                    wk.go_back();
                }
            }
        }
        self.spaces.fs_wait -= dt;
        if self.spaces.fs_wait > 0.0 {
            return;
        }
        let (roll, spread) = (self.walk.rand(), self.walk.rand());
        self.spaces.fs_wait = tn().fs_every * (0.6 + spread * 0.8);
        if roll >= tn().fs_chance {
            return;
        }
        let want = 1 + (self.walk.rand() * tn().fs_max) as usize;
        let mut free: Vec<usize> = (0..n).filter(|&i| {
            let wk = self.walk_ref(i);
            wk.backstage && wk.at_leisure()
        }).collect();
        for _ in 0..want.min(free.len()) {
            let k = ((self.walk.rand() * free.len() as f32) as usize).min(free.len() - 1);
            let i = free.swap_remove(k);
            let a = self.actor_mut(i);
            let (r, screen) = (a.hit_rect, a.surface[0].max(1.0));
            let middle = (r[0] + r[2]) * 0.5;
            let spot = screen * (0.1 + a.walk.rand() * 0.8);
            let stay = tn().fs_stay_min + a.walk.rand() * tn().fs_stay_rand;
            a.walk.x += (spot - middle) * 2.0 / screen;
            a.walk.come_out(stay);
            // now and then a visitor sits down and fishes for a while
            if a.walk.rand() < tn().fs_fish {
                a.walk.fish_on_arrival = true;
            }
        }
    }

    /// A race on the workspace in view: some from the others come over.
    /// Asked again and again while a race gathers on `here`: whoever is
    /// free on another workspace makes up its mind once (`stay`: not coming).
    pub(super) fn call_to_race(&mut self, n: usize, here: u64, stay: &mut Vec<usize>) {
        let order: Vec<u64> = self.spaces.list.iter().map(|s| s.id).collect();
        let Some(at) = order.iter().position(|&w| w == here) else { return };
        for i in 0..n {
            let ws = self.walk_ref(i).ws;
            let wk = self.walk_ref(i);
            if ws == here || stay.contains(&i) || !wk.at_leisure() || wk.backstage || wk.visiting {
                continue;
            }
            if self.walk_of(i).rand() >= tn().race_travel {
                stay.push(i);
                continue;
            }
            if let Some(from) = order.iter().position(|&w| w == ws) {
                log::debug!("race: {i} comes over from workspace {ws}");
                self.walk_of(i).set_off(here, at > from);
            }
        }
    }

    /// Sets `i` off for workspace `to`; false if there's no such way.
    pub(super) fn send_to(&mut self, i: usize, to: u64) -> bool {
        let order: Vec<u64> = self.spaces.list.iter().map(|s| s.id).collect();
        let from = self.walk_ref(i).ws;
        let (Some(a), Some(b)) = (order.iter().position(|&w| w == from), order.iter().position(|&w| w == to)) else { return false };
        if a != b {
            self.walk_of(i).set_off(to, b > a);
        }
        a != b
    }

    /// The workspace `i` is on its way to.
    pub(super) fn heading_to(&self, i: usize) -> Option<u64> {
        self.walk_ref(i).trip.map(|t| t.to)
    }

    /// Let go of in the overview over another workspace: lands there.
    fn drop_between_spaces(&mut self, n: usize) {
        for i in 0..n {
            let Some(point) = self.walk_of(i).dropped_at.take() else { continue };
            let Some(old) = self.actor_ref(i).zoom.filter(|z| z.shrunk()) else { continue };
            let ws = self.walk_ref(i).ws;
            let Some(to) = self.spaces.list.iter().find(|s| holds(s, point) && s.id != ws).cloned() else { continue };
            let a = self.actor_ref(i);
            let r = a.hit_rect;
            let surface = a.surface;
            let Some(new) = SceneZoom::new(to.rect, surface, 1.0) else { continue };
            let (from, into) = (old.to_scene(point), new.to_scene(point));
            let half = (r[3] - r[1]).max(1.0) / surface[1].max(1.0);
            let wk = self.walk_of(i);
            wk.x += (into[0] - from[0]) * 2.0 / surface[0].max(1.0);
            wk.air = (wk.air - (into[1] - from[1]) * 2.0 / surface[1].max(1.0) / half).max(0.0);
            wk.target = wk.x;
            wk.ws = to.id;
            wk.here_t = 0.0;
            wk.newcomer = true;
        }
    }

    /// Once a second: is anyone where it isn't needed? Characters gather
    /// where the windows and the work are; an empty workspace is left, a
    /// crowded one for how little it's used gives one up now and then.
    fn roam(&mut self, n: usize) {
        for i in 0..n {
            self.walk_of(i).here_t += 1.0;
        }
        let homes: Vec<u64> = place_homes().into_iter().filter(|p| self.spaces.list.iter().any(|s| s.id == *p)).collect();
        if !homes.is_empty() {
            self.park_roam(n, &homes);
            return;
        }
        // workspaces with windows on them; the one in use only if it has some too.
        // Not the park's (cafe's, farm's): they're only visited, for a reason
        let windows = place_windows();
        let mut hosts: Vec<u64> = self.spaces.list.iter().filter(|s| s.has_windows && !windows.contains(&s.id)).map(|s| s.id).collect();
        if hosts.is_empty() {
            hosts = self.spaces.list.iter().filter(|s| s.active && !windows.contains(&s.id)).map(|s| s.id).collect();
        }
        if hosts.is_empty() {
            return;
        }
        let count = |this: &Self, ws: u64| (0..n).filter(|&i| this.walk_ref(i).ws == ws || this.heading_to(i) == Some(ws)).count();
        // older use counts for less and less
        let fade = 0.5f32.powf(1.0 / tn().ws_use_half.max(1.0));
        self.spaces.used.iter_mut().for_each(|u| u.1 *= fade);
        let active = self.spaces.active;
        match self.spaces.used.iter_mut().find(|u| u.0 == active) {
            Some(u) => u.1 += 1.0,
            None => self.spaces.used.push((active, 1.0)),
        }
        let seats = self.seats(&hosts, n);
        // how many more each workspace should have (negative: too many)
        let short = |this: &Self, at: usize| seats[at] as f32 - count(this, hosts[at]) as f32;
        let moving = (0..n).filter(|&i| self.walk_ref(i).trip.is_some()).count();
        if moving >= tn().ws_max as usize {
            return;
        }
        self.spaces.quiet -= 1.0;
        // the neediest workspace
        let Some(to) = (0..hosts.len()).max_by(|&a, &b| short(self, a).total_cmp(&short(self, b))) else { return };
        let empty_spot = (0..n).find(|&i| {
            let wk = self.walk_ref(i);
            wk.at_leisure() && wk.trip.is_none() && !hosts.contains(&wk.ws) && !wk.visiting && !wk.backstage && wk.visit_for == 0 && !windows.contains(&wk.ws)
        });
        // on an empty workspace: off to where the work is, no waiting about
        let leaver = if let Some(i) = empty_spot.filter(|&i| self.walk_ref(i).here_t >= 20.0) {
            Some(i)
        } else {
            if self.spaces.quiet > 0.0 || short(self, to) < 1.0 {
                return;
            }
            // from the most overcrowded one, someone who's been there a while
            let Some(from) = (0..hosts.len()).filter(|&k| k != to).min_by(|&a, &b| short(self, a).total_cmp(&short(self, b))) else { return };
            if short(self, from) > -1.0 {
                return;
            }
            let pool: Vec<usize> = (0..n)
                .filter(|&i| {
                    let wk = self.walk_ref(i);
                    wk.ws == hosts[from] && wk.at_leisure() && wk.trip.is_none() && wk.here_t >= tn().ws_stay && !wk.visiting && !wk.backstage && wk.visit_for == 0
                })
                .collect();
            if pool.is_empty() || self.walk.rand() >= 1.0 / tn().ws_every.max(1.0) * pool.len() as f32 * 4.0 {
                return;
            }
            pool.into_iter().max_by(|&a, &b| self.walk_ref(a).here_t.total_cmp(&self.walk_ref(b).here_t))
        };
        let Some(i) = leaver else { return };
        let ws = self.walk_ref(i).ws;
        log::debug!("workspaces: {i} goes from {ws} to {}", hosts[to]);
        self.send_to(i, hosts[to]);
        self.spaces.quiet = tn().ws_gap;
        // a close friend may tag along, rarely
        if tn().ws_friends < 1.0 {
            return;
        }
        let pal = (0..n)
            .filter(|&j| j != i && self.close(i, j) && self.walk_ref(j).ws == ws && self.walk_ref(j).at_leisure())
            .max_by(|&a, &b| self.bond(i, a).0.total_cmp(&self.bond(i, b).0));
        if let Some(j) = pal.filter(|_| self.walk.rand() < tn().close_follow * 0.3) {
            log::debug!("workspaces: {j} goes along with {i}");
            self.send_to(j, hosts[to]);
        }
    }
}

/// The workspaces of the park, cafe and farm windows, when the characters
/// only visit them (modes 0 and 3): not somewhere to roam to.
fn place_windows() -> Vec<u64> {
    if (0.5..2.5).contains(&tn().park_mode) {
        return Vec::new();
    }
    governor::places().into_iter().map(|p| p.ws).collect()
}

impl CharacterOverlay {
    /// Back to the desktop after a visit: where it came from if that's still
    /// there, else the busiest workspace with windows. None: nowhere else.
    pub(super) fn desktop_home(&self, from: u64) -> Option<u64> {
        let windows: Vec<u64> = governor::places().into_iter().map(|p| p.ws).collect();
        let ok = |id: u64| self.spaces.list.iter().any(|s| s.id == id) && !windows.contains(&id);
        if ok(from) {
            return Some(from);
        }
        self.spaces
            .list
            .iter()
            .filter(|s| s.has_windows && !windows.contains(&s.id))
            .max_by_key(|s| governor::window_count(s.id))
            .map(|s| s.id)
            .or_else(|| self.spaces.list.iter().find(|s| s.active && !windows.contains(&s.id)).map(|s| s.id))
    }
}

/// The workspaces of the open park, cafe and farm windows, when the
/// characters should live there.
fn place_homes() -> Vec<u64> {
    // 0: the desktop only; 3: the desktop, visiting the windows (play.rs)
    if tn().park_mode < 0.5 || tn().park_mode > 2.5 {
        return Vec::new();
    }
    let mut v: Vec<u64> = governor::places().into_iter().map(|p| p.ws).collect();
    v.dedup();
    v
}

impl CharacterOverlay {
    /// Off on foot to somewhere in `p` (it's on the same workspace).
    pub(super) fn walk_into(&mut self, i: usize, p: &governor::Place) {
        let size = self.width_of(i);
        let r = self.walk_of(i).rand();
        let px = p.x + size + r * (p.w - size * 2.0).max(0.0);
        let here = self.strip_px(i);
        let surface = self.actor_ref(i).surface[0].max(1.0);
        let wk = self.walk_of(i);
        wk.target = wk.x + (px - here) * 2.0 / surface;
        wk.park_target_y = wk.rand() * super::park::MEADOW;
        wk.act = Act::Walk;
        wk.timer = 3.0;
    }

    /// Anima Park is open and home: everyone lives there (or travelling, if one
    /// is on another workspace); in mode 2 a few go out to the other workspaces
    /// and come back after a while.
    fn park_roam(&mut self, n: usize, homes: &[u64]) {
        let places = governor::places();
        if places.is_empty() {
            return;
        }
        let moving = (0..n).filter(|&i| self.walk_ref(i).trip.is_some()).count();
        let free = |this: &Self, i: usize| {
            let wk = this.walk_ref(i);
            wk.at_leisure() && wk.trip.is_none() && !wk.visiting && !wk.backstage
        };
        let in_place = |this: &Self, i: usize| governor::place_at(this.walk_ref(i).ws, this.strip_px(i)).map(|p| p.place);
        let only = tn().park_mode < 1.5;
        // out long enough (or not allowed out at all): back to the park
        let back = (0..n).find(|&i| free(self, i) && !homes.contains(&self.walk_ref(i).ws) && (only || self.walk_ref(i).here_t >= tn().park_out_stay));
        if let Some(i) = back {
            if moving < tn().ws_max as usize {
                let to = &places[0];
                log::debug!("park: {i} goes back to the {}", to.place);
                self.send_to(i, to.ws);
            }
            return;
        }
        // on the right workspace but not in the park (beside another window): in it goes
        if let Some(i) = (0..n).find(|&i| free(self, i) && homes.contains(&self.walk_ref(i).ws) && in_place(self, i).is_none() && self.walk_ref(i).act == Act::Idle) {
            if let Some(p) = places.iter().find(|p| p.ws == self.walk_ref(i).ws).cloned() {
                self.walk_into(i, &p);
            }
        }
        if only || moving >= tn().ws_max as usize {
            return;
        }
        let out = (0..n).filter(|&i| !homes.contains(&self.walk_ref(i).ws) || self.heading_to(i).is_some_and(|t| !homes.contains(&t))).count();
        if out as f32 >= tn().park_out || self.walk.rand() >= 1.0 / tn().park_out_every.max(1.0) {
            return;
        }
        // somewhere with windows, or the workspace in view
        let away: Vec<u64> = self.spaces.list.iter().filter(|s| !homes.contains(&s.id) && (s.has_windows || s.active)).map(|s| s.id).collect();
        if away.is_empty() {
            return;
        }
        let pool: Vec<usize> = (0..n).filter(|&i| free(self, i) && homes.contains(&self.walk_ref(i).ws) && self.walk_ref(i).here_t >= 30.0).collect();
        if pool.is_empty() {
            return;
        }
        let i = pool[(self.walk.rand() * pool.len() as f32) as usize % pool.len()];
        let to = away[(self.walk.rand() * away.len() as f32) as usize % away.len()];
        log::debug!("park: {i} goes out to workspace {to}");
        self.send_to(i, to);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn walker(ws: u64) -> WalkState {
        WalkState { act: Act::Idle, timer: 100.0, seed: 9, ws, ..Default::default() }
    }

    #[test]
    fn a_visitor_comes_up_and_goes_back_down() {
        let mut wk = walker(1);
        wk.backstage = true;
        wk.come_out(5.0);
        assert!(!wk.backstage && wk.visiting);
        for _ in 0..60 * 5 {
            wk.trip_tick(DT, 12.0);
        }
        assert_eq!((wk.act, wk.sink), (Act::Idle, 0.0));
        wk.go_back();
        for _ in 0..60 * 5 {
            wk.trip_tick(DT, 12.0);
        }
        assert!(wk.backstage && !wk.visiting && wk.trip.is_none());
        assert_eq!(wk.ws, 1, "stays on its workspace");
    }

    #[test]
    fn busy_workspaces_get_more_but_quiet_ones_keep_some() {
        // used a lot with many windows, against hardly used with one
        let seats = fair_shares(&[(900.0, 6.0), (30.0, 1.0)], 13, 0.15, 0.5);
        assert_eq!(seats.iter().sum::<usize>(), 13);
        assert!(seats[0] >= 9, "{seats:?}");
        assert!(seats[1] >= 2, "{seats:?}");
    }

    #[test]
    fn with_nothing_known_yet_shares_are_even() {
        assert_eq!(fair_shares(&[(0.0, 0.0), (0.0, 0.0)], 12, 0.15, 0.5), vec![6, 6]);
    }

    #[test]
    fn nobody_is_left_empty() {
        let seats = fair_shares(&[(1000.0, 9.0), (0.0, 0.0), (0.0, 0.0)], 5, 0.0, 0.5);
        assert_eq!(seats.iter().sum::<usize>(), 5);
        assert!(seats.iter().all(|&s| s >= 1), "{seats:?}");
    }

    #[test]
    fn goes_down_through_the_floor_and_falls_in_from_the_top() {
        let mut wk = walker(1);
        wk.set_off(2, true);
        for _ in 0..60 * 5 {
            wk.trip_tick(DT, 12.0);
            if wk.act != Act::Travel {
                break;
            }
        }
        assert_eq!((wk.ws, wk.act), (2, Act::Fall));
        assert_eq!(wk.air, 12.0);
        assert_eq!(wk.sink, 0.0);
        assert!(wk.newcomer);
    }

    #[test]
    fn goes_up_and_comes_out_of_the_floor_above() {
        let mut wk = walker(2);
        wk.set_off(1, false);
        let mut swapped = false;
        for _ in 0..60 * 20 {
            wk.trip_tick(DT, 12.0);
            swapped |= wk.ws == 1 && wk.act == Act::Travel && wk.sink > 0.0;
            if wk.act != Act::Travel {
                break;
            }
        }
        assert!(swapped, "never came up through the floor");
        assert_eq!((wk.ws, wk.act, wk.air, wk.sink), (1, Act::Idle, 0.0, 0.0));
        assert!(wk.newcomer && wk.trip.is_none());
    }

    #[test]
    fn picked_up_on_the_way_stays_where_it_was() {
        let mut wk = walker(1);
        wk.set_off(2, true);
        wk.trip_tick(DT, 12.0);
        wk.act = Act::Held;
        wk.trip_tick(DT, 12.0);
        assert!(wk.trip.is_none());
        assert_eq!(wk.ws, 1);
    }
}

fn places_path() -> std::path::PathBuf {
    crate::util::data_dir().join("places.json")
}
