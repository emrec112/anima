//! Who is free for what: one place for every "can it come along?" question,
//! so chats, games, watching and work all ask the same thing.

use super::super::CharacterOverlay;
use super::super::walk::Act;
use super::chat_cool;
use crate::render::tuning::get as tn;

/// What someone would be taken away for.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(in crate::render::character) enum Want {
    /// noticing someone standing close (a new pair)
    Pair,
    /// standing inside a chat's member: hard to miss
    Overlap,
    /// walking over to join a chat
    Join,
    /// called over by a friend in a chat
    Called,
    /// coming over to watch a game
    Watch,
    /// joining someone fishing
    Fish,
    /// going off to find company
    Seek,
    /// calling over from another workspace, etc. (desktop fishing stays on desktop windows)
    /// a go on something in the park
    Play,
}

impl CharacterOverlay {
    /// Belongs to nothing and isn't busy with itself.
    pub(in crate::render::character) fn unbound(&self, i: usize) -> bool {
        let wk = self.walk_ref(i);
        !wk.fret && !wk.fetching && wk.sign.is_none() && wk.social.is_none() && wk.race.is_none() && wk.crowd.is_none() && !wk.circle
    }

    /// May `i` be taken into `want` right now?
    pub(in crate::render::character) fn free_for(&self, i: usize, want: Want) -> bool {
        if !self.unbound(i) {
            return false;
        }
        let wk = self.walk_ref(i);
        let standing = wk.act == Act::Idle;
        let grounded = matches!(wk.act, Act::Idle | Act::Walk);
        if !grounded {
            return false;
        }
        let cool = chat_cool(wk);
        let t = tn();
        match want {
            Want::Pair | Want::Overlap => standing && self.on_screen(i),
            Want::Join => wk.energy >= t.energy_join_need && cool <= 0.0 && self.on_screen(i),
            Want::Called => cool <= 0.0,
            Want::Watch => wk.energy >= t.energy_watch_need && cool <= t.overlap_cool && self.on_screen(i),
            Want::Fish => wk.fish_cool <= 0.0 && cool <= 0.0 && self.on_screen(i),
            Want::Seek => standing && cool <= 0.0 && !wk.climb_like(),
            Want::Play => standing && wk.trip.is_none() && wk.rest.is_none() && cool <= t.overlap_cool && wk.energy >= t.energy_watch_need,
        }
    }
}
