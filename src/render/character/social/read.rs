//! Reading on the desktop: someone with nothing to do takes out a book and
//! reads where it stands, laughing at it now and then (the happy pose, nothing
//! else). A friend passing by may come and stand beside it with a book of its
//! own; laughs sometimes go round the two of them at once.

use super::super::CharacterOverlay;
use super::super::walk::Act;
use super::*;
use crate::render::tuning::get as tn;

/// Picture kinds: the book open, and shut (taking it out, putting it away).
pub(super) const BOOK_OPEN: i32 = 48;
pub(super) const BOOK_SHUT: i32 = 49;

// taking the book out or putting it away (s)
const HANDLE: f32 = 0.6;

#[derive(Clone, Copy, Debug)]
pub(super) struct Reader {
    i: usize,
    // 0 walking over, 1 opening the book, 2 reading, 3 closing it
    phase: u8,
    t: f32,
    // seconds of reading left
    left: f32,
    // laughing for this much longer, starting in `due` seconds
    laugh: f32,
    due: f32,
    // seconds to its next laugh of its own
    next: f32,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Read {
    readers: Vec<Reader>,
}

/// How fond of books a character is, 0..1, the same every time for a name:
/// some read a lot, some hardly ever.
pub(super) fn love(name: &str) -> f32 {
    let mut h: u32 = 2166136261;
    for b in name.bytes() {
        h = (h ^ b as u32).wrapping_mul(16777619);
    }
    let x = (h % 1000) as f32 / 999.0;
    x * x
}

impl CharacterOverlay {
    fn read_free(&self, i: usize) -> bool {
        self.free_for(i, Want::Join) && self.walk_ref(i).read_cool <= 0.0 && self.place_here(i).is_none()
    }

    fn book_love(&self, i: usize) -> f32 {
        love(&self.character_name(i))
    }

    fn new_reader(&mut self, i: usize) -> Reader {
        let t = tn();
        let keen = 0.5 + self.book_love(i);
        let wk = self.walk_of(i);
        let left = (t.read_min + wk.rand() * (t.read_max - t.read_min).max(0.0)) * keen;
        let next = t.read_laugh_every * (0.4 + wk.rand() * 1.2);
        Reader { i, phase: 0, t: 0.0, left, laugh: 0.0, due: 0.0, next }
    }

    /// `i` stops where it is and opens a book.
    fn read_alone(&mut self, i: usize) -> Chat {
        self.chat_id = self.chat_id.wrapping_add(1);
        let id = self.chat_id;
        self.enlist(i, id);
        self.walk_of(i).bored = 0.0;
        let x = self.strip_px(i);
        self.go(i, x, 0.14);
        log::debug!("chat {id}: {i} takes out a book");
        let mut c = Chat::new(id, vec![i], Stage::Read, 0.0);
        let r = self.new_reader(i);
        c.read = Some(Read { readers: vec![r] });
        c.timer = f32::MAX;
        c
    }

    /// Bored on the desktop: someone who likes books sits down to one, so to speak.
    pub(super) fn read_spark(&mut self, n: usize, every: f32) -> Option<Chat> {
        if self.social_age < tn().warm_small || self.moods.fretting {
            return None;
        }
        let ready: Vec<usize> = (0..n)
            .filter(|&i| {
                let wk = self.walk_ref(i);
                wk.act == Act::Idle && wk.bored >= tn().read_bored && self.read_free(i) && self.on_screen(i)
            })
            .collect();
        let i = *ready.get((self.walk.rand() * ready.len() as f32) as usize % ready.len().max(1))?;
        let keen = self.book_love(i) * 2.0;
        if self.walk_of(i).rand() >= every / tn().read_every.max(1.0) * keen {
            return None;
        }
        Some(self.read_alone(i))
    }

    /// Asked for by hand: whoever is free nearest the middle of the screen.
    pub(super) fn read_now(&mut self, chats: &[Chat], n: usize) -> Option<Chat> {
        let busy: Vec<usize> = chats.iter().flat_map(|c| c.members.iter().copied()).collect();
        let middle = self.view[0] * self.view_w * 0.5 + self.view_w * 0.5;
        let i = (0..n)
            .filter(|&i| !busy.contains(&i) && self.unbound(i) && self.on_screen(i) && self.place_here(i).is_none() && matches!(self.walk_ref(i).act, Act::Idle | Act::Walk))
            .min_by(|&a, &b| (self.strip_px(a) - middle).abs().total_cmp(&(self.strip_px(b) - middle).abs()))?;
        let wk = self.walk_of(i);
        wk.act = Act::Idle;
        wk.target = wk.x;
        Some(self.read_alone(i))
    }

    /// Someone free close by, a friend more likely, comes and stands beside
    /// them with a book of its own.
    pub(super) fn read_join(&mut self, c: &mut Chat, n: usize, every: f32) {
        let Some(r) = c.read.as_ref() else { return };
        if c.members.len() >= tn().read_most as usize || r.readers.iter().any(|r| r.phase == 3) {
            return;
        }
        let ws = self.walk_ref(c.members[0]).ws;
        let xs: Vec<f32> = r.readers.iter().map(|r| self.strip_px(r.i)).collect();
        let (lo, hi) = (xs.iter().copied().fold(f32::MAX, f32::min), xs.iter().copied().fold(f32::MIN, f32::max));
        let range = self.view_w * 0.5;
        let Some(j) = (0..n).find(|&j| !c.members.contains(&j) && self.walk_ref(j).ws == ws && self.read_free(j) && (self.strip_px(j) - (lo + hi) * 0.5).abs() < range) else {
            return;
        };
        let keen = (1.0 + c.members.iter().map(|&m| self.bond(j, m).0).fold(0.0, f32::max) * 2.0) * self.book_love(j) * 2.0;
        if self.walk_of(j).rand() >= every * tn().read_join * keen {
            return;
        }
        let size = self.width_of(j);
        let x = self.strip_px(j);
        let width = self.strip_width_of(j);
        // right beside the nearer end
        let spot = if (x - lo).abs() < (x - hi).abs() { lo - size * 0.85 } else { hi + size * 0.85 };
        let spot = spot.clamp(size, (width - size).max(size));
        self.enlist(j, c.id);
        c.members.push(j);
        self.go(j, spot, 0.14);
        let reader = self.new_reader(j);
        if let Some(r) = c.read.as_mut() {
            r.readers.push(reader);
        }
        log::debug!("chat {}: {j} comes to read beside them", c.id);
    }

    pub(super) fn run_read(&mut self, c: &mut Chat, dt: f32) {
        let Some(mut r) = c.read.take() else { return };
        r.readers.retain(|a| c.members.contains(&a.i));
        let mut done = Vec::new();
        let mut laughs = Vec::new();
        let first = r.readers.first().map(|a| a.i);
        for k in 0..r.readers.len() {
            let mut a = r.readers[k];
            // wandered into the park, cafe or farm: that's another world
            if self.place_here(a.i).is_some() {
                done.push(a.i);
                continue;
            }
            // picked up, petted, put down: the book waits till it's back in place
            if !self.arrived(a.i) || self.walk_ref(a.i).act != Act::Idle {
                a.laugh = 0.0;
                a.due = 0.0;
                if a.phase == 2 {
                    a.phase = 1;
                    a.t = 0.0;
                }
                r.readers[k] = a;
                continue;
            }
            a.t += dt;
            match a.phase {
                0 => {
                    a.phase = 1;
                    a.t = 0.0;
                    // side by side, both the same way round, as if reading together
                    let left = first.filter(|&f| f != a.i).and_then(|f| self.walk_ref(f).social.map(|s| s.face_left));
                    if let (Some(left), Some(s)) = (left, self.walk_of(a.i).social.as_mut()) {
                        s.face_left = left;
                    }
                }
                1 if a.t >= HANDLE => {
                    a.phase = 2;
                    a.t = 0.0;
                }
                2 => {
                    a.left -= dt;
                    if a.left <= 0.0 {
                        a.phase = 3;
                        a.t = 0.0;
                        a.laugh = 0.0;
                    } else if a.laugh > 0.0 {
                        if a.due > 0.0 {
                            a.due -= dt;
                        } else {
                            a.laugh -= dt;
                        }
                    } else {
                        a.next -= dt;
                        if a.next <= 0.0 {
                            let t = tn();
                            let wk = self.walk_of(a.i);
                            a.laugh = t.read_laugh_len * (0.6 + wk.rand() * 0.8);
                            a.due = 0.0;
                            a.next = t.read_laugh_every * (0.4 + wk.rand() * 1.2);
                            laughs.push(a.i);
                        }
                    }
                }
                3 if a.t >= HANDLE => done.push(a.i),
                _ => {}
            }
            r.readers[k] = a;
        }
        // a laugh goes round: the others reading may laugh along, all but together
        for &who in &laughs {
            for k in 0..r.readers.len() {
                let mut b = r.readers[k];
                if b.i == who || b.phase != 2 || b.laugh > 0.0 {
                    continue;
                }
                let keen = 1.0 + self.bond(who, b.i).0;
                let wk = self.walk_of(b.i);
                if wk.rand() < tn().read_sync * keen {
                    b.laugh = tn().read_laugh_len * (0.7 + wk.rand() * 0.5);
                    b.due = wk.rand() * 0.25;
                    b.next = b.next.max(tn().read_laugh_every * 0.5);
                    r.readers[k] = b;
                }
            }
        }
        for a in &r.readers {
            let happy = a.phase == 2 && a.laugh > 0.0 && a.due <= 0.0;
            if let Some(s) = self.walk_of(a.i).social.as_mut() {
                s.happy = happy;
            }
        }
        for &i in &done {
            log::debug!("chat {}: {i} puts its book away", c.id);
            let cool = tn().read_again * (0.7 + self.walk_of(i).rand() * 0.6);
            let wk = self.walk_of(i);
            wk.read_cool = wk.read_cool.max(cool);
            self.leave(i, c.id);
        }
        c.members.retain(|i| !done.contains(i));
        r.readers.retain(|a| !done.contains(&a.i));
        if r.readers.is_empty() {
            c.members.clear();
            c.timer = 0.0;
            c.stage = Stage::Part;
            return;
        }
        c.read = Some(r);
    }

    /// The books in hand: held up in front, shut while being taken out or put away.
    pub(super) fn read_props(&self, v: &mut Vec<(i32, [f32; 4], bool, usize)>) {
        for c in self.chats.iter().filter(|c| c.stage == Stage::Read) {
            let Some(r) = c.read.as_ref() else { continue };
            for a in &r.readers {
                if a.phase == 0 || !self.arrived(a.i) || self.walk_ref(a.i).act != Act::Idle {
                    continue;
                }
                let Some(rect) = (if a.i == 0 { Some(self.hit_rect) } else { self.extras.get(a.i - 1).map(|e| e.hit_rect) }) else { continue };
                let h = rect[3] - rect[1];
                if h <= 0.0 {
                    continue;
                }
                let left = self.walk_ref(a.i).social.is_some_and(|s| s.face_left);
                let dir = if left { -1.0 } else { 1.0 };
                let open = a.phase == 2 || (a.phase == 1 && a.t >= HANDLE * 0.5) || (a.phase == 3 && a.t < HANDLE * 0.5);
                let size = h * if open { 0.34 } else { 0.26 };
                let x = (rect[0] + rect[2]) * 0.5 + dir * h * 0.14;
                let y = rect[1] + h * 0.6;
                v.push((if open { BOOK_OPEN } else { BOOK_SHUT }, [x - size * 0.5, y - size * 0.5, x + size * 0.5, y + size * 0.5], false, a.i));
            }
        }
    }
}
