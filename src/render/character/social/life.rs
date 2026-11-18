//! A chat's life: everyone comes in one way and goes out one way, every
//! stage has a longest time, and whoever leaves rests a while before the
//! next thing can take it.

use super::super::CharacterOverlay;
use super::{Chat, Stage};
use crate::render::tuning::get as tn;

/// Longest a chat may stay in one stage before it's ended for good.
pub(super) fn stage_limit(stage: Stage) -> f32 {
    let t = tn();
    match stage {
        Stage::Notice | Stage::Approach | Stage::Stroll => 90.0,
        Stage::Talk | Stage::RaceCall => 240.0,
        Stage::Part | Stage::Cheer => 60.0,
        Stage::RaceWait | Stage::Concert => 600.0,
        Stage::Fishing => t.fish_time + 120.0,
        Stage::Wrestle => 180.0,
        Stage::Snow => t.snow_longest + 300.0,
        Stage::Hide => t.hide_time + 60.0,
        Stage::Play => 900.0,
        _ => t.chat_max,
    }
}

impl CharacterOverlay {
    /// `i` is out of chat `id`: free, after a rest.
    pub(super) fn leave(&mut self, i: usize, id: u32) {
        let wk = self.walk_of(i);
        if wk.social.is_some_and(|s| s.chat == id) {
            wk.social = None;
            let rest = tn().cool_base + wk.rand() * tn().cool_rand;
            wk.social_cool = wk.social_cool.max(rest);
        }
    }

    /// The chat is over: everyone in it and watching it goes.
    pub(super) fn finish(&mut self, c: &Chat, n: usize) {
        let big = c.members.len() >= tn().big_group as usize;
        for &i in c.watchers.iter().filter(|&&i| i < n) {
            self.leave(i, c.id);
        }
        for &i in c.members.iter().filter(|&&i| i < n) {
            // up a jetty or a seat: down on foot, not a fall
            if self.walk_ref(i).air > 0.05 && self.walk_ref(i).social.is_some_and(|s| s.chat == c.id) {
                self.push_down(i);
            }
            self.leave(i, c.id);
            if big {
                let wk = self.walk_of(i);
                wk.crowd_rest = tn().crowd_rest_base + wk.rand() * tn().crowd_rest_rand;
            }
        }
    }

    /// A chat belongs to one world, the desktop or the windows: a talker who
    /// wanders over into the other one is out of it. All of them going over
    /// together take the chat with them.
    pub(super) fn keep_world(&mut self, c: &mut Chat, n: usize) {
        let inside = |this: &Self, i: usize| this.place_here(i).is_some();
        let members: Vec<usize> = c.members.iter().copied().filter(|&i| i < n).collect();
        if members.is_empty() {
            return;
        }
        let first = inside(self, members[0]);
        if members.iter().all(|&i| inside(self, i) == first) {
            c.world = Some(first);
            return;
        }
        let Some(world) = c.world else { return };
        if !matches!(c.stage, Stage::Notice | Stage::Approach | Stage::Talk) {
            return;
        }
        for i in members {
            if inside(self, i) != world && !self.walk_ref(i).social.is_some_and(|s| s.grabbed) {
                log::debug!("chat {}: {i} is in the other world now, out of it", c.id);
                c.members.retain(|&m| m != i);
                self.leave(i, c.id);
            }
        }
    }

    /// Time in the current stage; false once it's gone on far too long.
    pub(super) fn within_limit(&mut self, c: &mut Chat, dt: f32) -> bool {
        if c.stage != c.last_stage {
            c.last_stage = c.stage;
            c.stage_t = 0.0;
        }
        c.stage_t += dt;
        if c.stage_t <= stage_limit(c.stage) {
            return true;
        }
        log::warn!("chat {}: {:?} ran over its time ({:.0}s), ended", c.id, c.stage, c.stage_t);
        false
    }
}

/// What a stage lets its members do; anything else is put back.
pub(super) struct Allows {
    pub(super) lean: bool,
    pub(super) perched: bool,
    pub(super) game_flags: bool,
}

pub(super) fn allows(stage: Stage) -> Allows {
    let talk = matches!(stage, Stage::Notice | Stage::Approach | Stage::Talk | Stage::Stroll | Stage::Part | Stage::Cheer);
    Allows {
        lean: matches!(stage, Stage::Tug | Stage::Wrestle | Stage::Play | Stage::Snow),
        // up on a net, a box, a seat or a jetty
        perched: matches!(stage, Stage::Volley | Stage::Gift | Stage::Play | Stage::Fishing),
        game_flags: !talk,
    }
}
