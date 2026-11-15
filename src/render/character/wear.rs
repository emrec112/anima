//! Things worn and held: hats, glasses, a scarf, wings, an umbrella in the
//! rain, a Santa hat at Christmas, snow on the head. Each is placed on the
//! sprite's own anchors (head, eyes, neck, back, hand) and drawn with the
//! sprite's own transform, so it turns, flips, hops, breathes and lies down
//! with the character.
//!
//! The built-in set is drawn by the prop shader (rope.rs). Anyone can add
//! more: a PNG in `~/.local/share/anima/accessories/`, with an optional
//! TOML of the same name:
//!
//! ```toml
//! anchor = "crown"    # crown, eyes, neck, back or hand
//! size = [0.6, 0.5]   # width and height, in head widths and heights
//! pivot = [0.5, 1.0]  # the point of the picture that sits on the anchor
//! offset = [0.0, 0.1] # moved by this much (head widths, head heights down)
//! behind = false      # drawn behind the character (wings, backpacks)
//! ```
//!
//! Who wears what is kept in `wardrobe.json` (character name -> item ids).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use super::walk::Act;
use super::CharacterOverlay;
use crate::render::tuning::get as tn;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    Crown,
    Eyes,
    Neck,
    Back,
    Hand,
    // the middle of the chest, where it holds things close
    Chest,
    // upper/front arm between shoulder and hand
    Arm,
    // the other hand, the one at the back
    OffHand,
    // the feet (shoes come in pairs)
    Feet,
}

impl Anchor {
    pub const ALL: [Anchor; 9] = [
        Anchor::Crown,
        Anchor::Eyes,
        Anchor::Neck,
        Anchor::Chest,
        Anchor::Arm,
        Anchor::Back,
        Anchor::Hand,
        Anchor::OffHand,
        Anchor::Feet,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Crown => "Head",
            Self::Eyes => "Face",
            Self::Neck => "Neck",
            Self::Chest => "Body",
            Self::Arm => "Arm",
            Self::Back => "Back",
            Self::Hand => "Hand",
            Self::OffHand => "Off-hand",
            Self::Feet => "Feet",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "crown" | "head" => Self::Crown,
            "eyes" | "face" => Self::Eyes,
            "neck" => Self::Neck,
            "back" => Self::Back,
            "hand" => Self::Hand,
            "chest" | "body" => Self::Chest,
            "arm" => Self::Arm,
            "offhand" | "other_hand" => Self::OffHand,
            "feet" | "foot" => Self::Feet,
            _ => return None,
        })
    }
}

/// One kind of thing to wear.
#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub label: String,
    /// prop shader kind, or 100 for a picture
    kind: i32,
    image: Option<PathBuf>,
    pub anchor: Anchor,
    size: [f32; 2],
    pivot: [f32; 2],
    offset: [f32; 2],
    pub behind: bool,
    /// worn twice, mirrored, this far either side (head widths): shoes, wings
    pair: f32,
}

/// Pictures in the props folder worn like the rest: id, label, file, anchor,
/// size, pivot (where the picture is held or sits), offset, behind, pair.
#[allow(clippy::type_complexity)]
const PICTURES: [(&str, &str, &str, Anchor, [f32; 2], [f32; 2], [f32; 2], bool, f32); 31] = [
    ("helmet", "Helmet", "helmet", Anchor::Crown, [1.05, 0.95], [0.5, 0.72], [0.0, 0.42], false, 0.0),
    ("cap", "Cap", "cap", Anchor::Crown, [0.85, 0.7], [0.45, 0.8], [0.05, 0.25], false, 0.0),
    ("grad_cap", "Graduation cap", "grad_cap", Anchor::Crown, [0.9, 0.8], [0.5, 0.75], [0.0, 0.22], false, 0.0),
    ("chick", "Chick on the head", "chick", Anchor::Crown, [0.42, 0.42], [0.5, 0.92], [0.1, 0.12], false, 0.0),
    ("butterfly", "Butterfly", "butterfly", Anchor::Crown, [0.32, 0.32], [0.5, 0.5], [-0.32, 0.14], false, 0.0),
    ("sword", "Sword", "sword", Anchor::Hand, [0.8, 0.8], [0.8, 0.22], [0.0, 0.0], false, 0.0),
    ("shield", "Shield", "shield", Anchor::OffHand, [0.62, 0.62], [0.5, 0.5], [0.0, 0.0], false, 0.0),
    ("swords", "Swords on the back", "swords", Anchor::Back, [1.0, 1.0], [0.5, 0.5], [0.0, -0.3], true, 0.0),
    ("wand", "Magic wand", "wand", Anchor::Hand, [0.6, 0.6], [0.2, 0.85], [0.0, 0.0], false, 0.0),
    ("bow_arrow", "Bow and arrow", "bow_arrow", Anchor::Hand, [0.8, 0.8], [0.3, 0.6], [0.0, 0.0], false, 0.0),
    ("axe", "Axe", "axe", Anchor::Hand, [0.7, 0.7], [0.65, 0.85], [0.0, 0.0], false, 0.0),
    ("hammer", "Hammer", "hammer", Anchor::Hand, [0.7, 0.7], [0.7, 0.8], [0.0, 0.0], false, 0.0),
    ("broom", "Broom", "broom", Anchor::Hand, [1.1, 1.1], [0.72, 0.28], [0.0, 0.0], false, 0.0),
    ("mic", "Microphone", "mic", Anchor::Hand, [0.5, 0.5], [0.75, 0.82], [0.0, 0.0], false, 0.0),
    ("lollipop", "Lollipop", "lollipop", Anchor::Hand, [0.5, 0.5], [0.85, 0.85], [0.0, 0.0], false, 0.0),
    ("ice_cream", "Ice cream", "ice_cream", Anchor::Hand, [0.45, 0.45], [0.5, 0.7], [0.0, 0.0], false, 0.0),
    ("bubble_tea", "Bubble tea", "bubble_tea", Anchor::Hand, [0.4, 0.5], [0.5, 0.7], [0.0, 0.0], false, 0.0),
    ("trophy", "Trophy", "trophy", Anchor::Hand, [0.5, 0.5], [0.5, 0.9], [0.0, 0.0], false, 0.0),
    ("handbag", "Handbag", "handbag", Anchor::Hand, [0.5, 0.5], [0.5, 0.12], [0.0, 0.0], false, 0.0),
    ("flashlight", "Flashlight", "flashlight", Anchor::Hand, [0.55, 0.55], [0.75, 0.75], [0.0, 0.0], false, 0.0),
    ("crystal_ball", "Crystal ball", "crystal_ball", Anchor::Hand, [0.45, 0.45], [0.5, 0.7], [0.0, 0.0], false, 0.0),
    ("gem", "Gem", "gem", Anchor::Hand, [0.32, 0.32], [0.5, 0.5], [0.0, 0.0], false, 0.0),
    ("teddy", "Teddy bear", "teddy", Anchor::Chest, [0.55, 0.55], [0.5, 0.5], [0.0, 0.0], false, 0.0),
    ("guitar", "Guitar", "guitar", Anchor::Chest, [0.95, 0.95], [0.5, 0.5], [0.0, 0.0], false, 0.0),
    ("medal", "Medal", "medal", Anchor::Neck, [0.42, 0.42], [0.5, 0.1], [0.0, 0.0], false, 0.0),
    ("wings_feathered", "Feathered wings", "wing", Anchor::Back, [0.8, 0.8], [0.06, 0.6], [0.0, -0.05], true, 0.06),
    ("sneakers", "Sneakers", "sneaker", Anchor::Feet, [0.36, 0.36], [0.5, 0.85], [0.0, 0.0], false, 0.17),
    ("heels", "High heels", "heel", Anchor::Feet, [0.36, 0.36], [0.5, 0.88], [0.0, 0.0], false, 0.17),
    ("flats", "Flat shoes", "flat_shoe", Anchor::Feet, [0.36, 0.3], [0.5, 0.85], [0.0, 0.0], false, 0.17),
    ("boots", "Boots", "hiking_boot", Anchor::Feet, [0.36, 0.36], [0.5, 0.88], [0.0, 0.0], false, 0.17),
    ("ballet", "Ballet shoes", "ballet", Anchor::Feet, [0.6, 0.45], [0.5, 0.85], [0.0, 0.0], false, 0.0),
];

/// What the wardrobe offers: id, label, kind, anchor, size, pivot, offset, behind.
const BUILT_IN: [(&str, &str, i32, Anchor, [f32; 2], [f32; 2], [f32; 2], bool); 12] = [
    ("party_hat", "Party hat", 27, Anchor::Crown, [0.5, 0.62], [0.5, 1.0], [0.1, 0.18], false),
    ("top_hat", "Top hat", 28, Anchor::Crown, [0.62, 0.55], [0.5, 1.0], [0.0, 0.16], false),
    ("crown", "Crown", 37, Anchor::Crown, [0.5, 0.32], [0.5, 1.0], [0.0, 0.14], false),
    ("bow", "Big bow", 29, Anchor::Crown, [0.44, 0.24], [0.5, 0.5], [0.3, 0.14], false),
    ("flower", "Flower", 32, Anchor::Crown, [0.28, 0.25], [0.5, 0.5], [-0.3, 0.16], false),
    ("headphones", "Headphones", 36, Anchor::Crown, [1.08, 0.62], [0.5, 0.12], [0.0, 0.0], false),
    ("glasses", "Round glasses", 30, Anchor::Eyes, [0.8, 0.22], [0.5, 0.5], [0.0, 0.0], false),
    ("sunglasses", "Sunglasses", 31, Anchor::Eyes, [0.82, 0.24], [0.5, 0.5], [0.0, 0.0], false),
    ("sleep_mask", "Sleep mask (asleep)", 26, Anchor::Eyes, [0.95, 0.3], [0.5, 0.5], [0.0, 0.0], false),
    ("scarf", "Scarf", 33, Anchor::Neck, [0.78, 0.5], [0.5, 0.3], [0.0, 0.0], false),
    ("backpack", "Backpack", 34, Anchor::Back, [0.62, 0.62], [0.5, 0.5], [0.0, 0.05], true),
    ("wings", "Angel wings", 35, Anchor::Back, [1.9, 0.9], [0.5, 0.5], [0.0, 0.0], true),
];

pub fn accessories_dir() -> PathBuf {
    crate::util::data_dir().join("accessories")
}

/// Hand-made adjustments: character name -> item id -> [dx, dy, wide, tall]:
/// moved by dx, dy (texture units of the calm sprite) and scaled.
pub fn fit_path() -> PathBuf {
    crate::util::data_dir().join("fit.json")
}

pub type Fits = HashMap<String, HashMap<String, [f32; 4]>>;

/// The key an item's turn is kept under: [angle (radians, anticlockwise),
/// mirrored (1) or not (0), 0, 0].
pub fn turn_key(id: &str, happy: bool) -> String {
    format!("{}~turn", fit_key(id, happy))
}

/// Raw per-character turn/mirror/layer record: `[turn, mirror, behind_override, 0.0]`.
pub fn turn_raw(fits: Option<&HashMap<String, [f32; 4]>>, id: &str, happy: bool) -> [f32; 4] {
    fits.and_then(|f| f.get(&turn_key(id, happy)).or_else(|| f.get(&turn_key(id, false))))
        .copied()
        .unwrap_or([0.0; 4])
}

/// (angle, mirrored) set by hand for `id`, the happy pose's own if it has one.
pub fn turn_of(fits: Option<&HashMap<String, [f32; 4]>>, id: &str, happy: bool) -> (f32, bool) {
    let t = turn_raw(fits, id, happy);
    (t[0], t[1] > 0.5)
}

/// Whether `id` is drawn behind the character (`t[2] > 0.5` forces behind,
/// `t[2] < -0.5` forces in front, `0.0` keeps `default`).
pub fn behind_of(fits: Option<&HashMap<String, [f32; 4]>>, id: &str, happy: bool, default: bool) -> bool {
    let v = turn_raw(fits, id, happy)[2];
    if v > 0.5 {
        true
    } else if v < -0.5 {
        false
    } else {
        default
    }
}

/// Key suffix for an independent side of a pair (`-1.0` = Left, `+1.0` = Right).
pub fn side_id(id: &str, side: f32) -> String {
    if side < 0.0 { format!("{id}~L") } else { format!("{id}~R") }
}

/// Fit for one side of a pair, preferring `id~L` / `id~R` and falling back to
/// the legacy shared key (`(fit, is_independent)`).
pub fn fit_of_side(fits: Option<&HashMap<String, [f32; 4]>>, id: &str, side: f32, happy: bool) -> (Option<[f32; 4]>, bool) {
    let sid = side_id(id, side);
    if let Some(f) = fits.and_then(|m| m.get(&fit_key(&sid, happy)).or_else(|| m.get(&sid))) {
        return (Some(*f), true);
    }
    let shared = fits.and_then(|m| m.get(&fit_key(id, happy)).or_else(|| m.get(id))).copied();
    (shared, false)
}

/// Turn & mirror for one side of a pair, preferring `id~L` / `id~R` and
/// falling back to the legacy shared turn.
pub fn turn_of_side(fits: Option<&HashMap<String, [f32; 4]>>, id: &str, side: f32, happy: bool) -> (f32, bool) {
    let sid = side_id(id, side);
    if let Some(t) = fits.and_then(|f| f.get(&turn_key(&sid, happy)).or_else(|| f.get(&turn_key(&sid, false)))) {
        return (t[0], t[1] > 0.5);
    }
    let (t, m) = turn_of(fits, id, happy);
    (if pair_mirrored(side) { -t } else { t }, m)
}

/// Behind override for one side of a pair, falling back to shared override then `default`.
pub fn behind_of_side(fits: Option<&HashMap<String, [f32; 4]>>, id: &str, side: f32, happy: bool, default: bool) -> bool {
    let sid = side_id(id, side);
    if let Some(t) = fits.and_then(|f| f.get(&turn_key(&sid, happy)).or_else(|| f.get(&turn_key(&sid, false)))) {
        if t[2] > 0.5 {
            return true;
        }
        if t[2] < -0.5 {
            return false;
        }
    }
    behind_of(fits, id, happy, default)
}

/// Key for per-character part/anchor & draw order overrides: `[anchor_idx_1based, z_order, 0.0, 0.0]`.
pub fn meta_key(id: &str, happy: bool) -> String {
    format!("{}~meta", fit_key(id, happy))
}

pub fn meta_raw(fits: Option<&HashMap<String, [f32; 4]>>, id: &str, happy: bool) -> [f32; 4] {
    fits.and_then(|f| f.get(&meta_key(id, happy)).or_else(|| f.get(&meta_key(id, false))))
        .copied()
        .unwrap_or([0.0; 4])
}

pub fn anchor_of(fits: Option<&HashMap<String, [f32; 4]>>, id: &str, happy: bool, default: Anchor) -> Anchor {
    let idx = meta_raw(fits, id, happy)[0].round() as usize;
    idx.checked_sub(1).and_then(|k| Anchor::ALL.get(k).copied()).unwrap_or(default)
}

pub fn anchor_of_side(fits: Option<&HashMap<String, [f32; 4]>>, id: &str, side: f32, happy: bool, default: Anchor) -> Anchor {
    let sid = side_id(id, side);
    let idx = meta_raw(fits, &sid, happy)[0].round() as usize;
    idx.checked_sub(1).and_then(|k| Anchor::ALL.get(k).copied()).unwrap_or_else(|| anchor_of(fits, id, happy, default))
}

pub fn order_of(fits: Option<&HashMap<String, [f32; 4]>>, id: &str, happy: bool) -> f32 {
    meta_raw(fits, id, happy)[1]
}

pub fn order_of_side(fits: Option<&HashMap<String, [f32; 4]>>, id: &str, side: f32, happy: bool) -> f32 {
    let sid = side_id(id, side);
    if fits.is_some_and(|f| f.contains_key(&meta_key(&sid, happy)) || f.contains_key(&meta_key(&sid, false))) {
        return order_of(fits, &sid, happy);
    }
    order_of(fits, id, happy)
}

/// Where `item` goes on a sprite with these anchors: left, top, width,
/// height in texture units of the calm sprite (not mirrored), with the
/// hand-made `fit` applied. `faces_right`: the sprite looks right (Uma
/// sprites do), so the right hand is the one in front.
pub fn layout(a: &crate::render::anchors::Anchors, item: &Item, faces_right: bool, fit: Option<[f32; 4]>) -> [f32; 4] {
    let (hw, hh) = ((a.head[2] - a.head[0]).max(0.01), (a.head[3] - a.head[1]).max(0.01));
    let p = anchor_point(a, item.anchor, faces_right);
    let [dx, dy, sw, sh] = fit.unwrap_or([0.0, 0.0, 1.0, 1.0]);
    let (w, h) = (item.size[0] * hw * sw, item.size[1] * hh * sh);
    // scaled about where it sits, then moved
    let left = p[0] + item.offset[0] * hw - item.pivot[0] * w + dx;
    let top = p[1] + item.offset[1] * hh - item.pivot[1] * h + dy;
    [left, top, w, h]
}

pub fn anchor_point(a: &crate::render::anchors::Anchors, anchor: Anchor, faces_right: bool) -> [f32; 2] {
    match anchor {
        Anchor::Crown => [(a.head[0] + a.head[2]) * 0.5, a.head[1]],
        Anchor::Eyes => [(a.eyes[0][0] + a.eyes[1][0]) * 0.5, (a.eyes[0][1] + a.eyes[1][1]) * 0.5],
        Anchor::Neck => a.neck,
        Anchor::Back => a.back,
        Anchor::Hand => if faces_right { a.hands[1] } else { a.hands[0] },
        Anchor::Chest => [(a.hands[0][0] + a.hands[1][0]) * 0.5, a.neck[1] + (a.feet[1] - a.neck[1]) * 0.2],
        Anchor::Arm => {
            let hand = if faces_right { a.hands[1] } else { a.hands[0] };
            [a.neck[0] * 0.45 + hand[0] * 0.55, a.neck[1] * 0.55 + hand[1] * 0.45]
        }
        Anchor::OffHand => if faces_right { a.hands[0] } else { a.hands[1] },
        Anchor::Feet => a.feet,
    }
}

/// The happy pose's anchors, with what can't be seen there (eyes shut in
/// a smile, hands out of sight) taken from the calm pose, in the same place
/// relative to the head and the body.
pub fn happy_anchors(calm: &crate::render::anchors::Anchors, mut happy: crate::render::anchors::Anchors) -> crate::render::anchors::Anchors {
    let rel = |p: [f32; 2], from: &[f32; 4], to: &[f32; 4]| {
        let (fw, fh) = ((from[2] - from[0]).max(0.01), (from[3] - from[1]).max(0.01));
        [to[0] + (p[0] - from[0]) / fw * (to[2] - to[0]), to[1] + (p[1] - from[1]) / fh * (to[3] - to[1])]
    };
    if !happy.eyes_found {
        happy.eyes = [rel(calm.eyes[0], &calm.head, &happy.head), rel(calm.eyes[1], &calm.head, &happy.head)];
        happy.eye_size = calm.eye_size;
    }
    for k in 0..2 {
        if !happy.hands_found[k] {
            happy.hands[k] = rel(calm.hands[k], &calm.body, &happy.body);
        }
    }
    happy
}

/// The key a hand-made fit is kept under: the item's id, with "@happy" for
/// the happy pose.
pub fn fit_key(id: &str, happy: bool) -> String {
    if happy { format!("{id}@happy") } else { id.to_string() }
}

/// The happy pose's picture next to a calm Uma sprite.
pub fn happy_path(calm: &str) -> Option<String> {
    let p = std::path::Path::new(calm);
    let name = p.file_name()?.to_str()?;
    name.contains("_0010.").then(|| p.with_file_name(name.replace("[A]", "[B]").replace("_0010.", "_0011.")).to_string_lossy().into_owned())
}

/// The picture `name` sees for `item`.
pub fn picture_for(item: &Item, _name: &str) -> Option<PathBuf> {
    picture_of(item)
}

/// How far apart a pair is worn (head widths), 0 for a single thing.
pub fn pair_of(item: &Item) -> f32 {
    item.pair
}

pub fn pair_copy_side(item: &Item, side: f32, fit: Option<[f32; 4]>, independent: bool) -> (Item, Option<[f32; 4]>) {
    let mut one = item.clone();
    one.offset[0] += side * item.pair;
    if side < 0.0 {
        one.pivot[0] = 1.0 - one.pivot[0];
    }
    let fit = if independent { fit } else { fit.map(|f| [f[0] * side, f[1], f[2], f[3]]) };
    (one, fit)
}

/// Whether this side of a pair is drawn mirrored.
pub fn pair_mirrored(side: f32) -> bool {
    side > 0.0
}

/// What each character has found (character name -> item ids), beyond what
/// everyone starts with.
pub fn inventory_path() -> PathBuf {
    crate::util::data_dir().join("inventory.json")
}

/// Everyone has these from the start (the umbrella comes with the weather).
pub const STARTERS: [&str; 1] = ["scarf"];

pub type Inventory = HashMap<String, Vec<String>>;

fn read_inventory() -> Inventory {
    std::fs::read_to_string(inventory_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Whether `name` has `id` to wear.
pub fn owns(inv: &Inventory, name: &str, id: &str) -> bool {
    // "*" in the inventory: everything is open to everyone
    STARTERS.contains(&id) || inv.contains_key("*") || inv.get(name).is_some_and(|v| v.iter().any(|x| x == id))
}

/// `name` found `id`: it's theirs from now on.
pub(super) fn give(name: &str, id: &str) {
    let mut inv = read_inventory();
    let mine = inv.entry(name.to_string()).or_default();
    if !mine.iter().any(|x| x == id) {
        mine.push(id.to_string());
        if let Ok(text) = serde_json::to_string_pretty(&inv) {
            let _ = crate::util::atomic_write(&inventory_path(), text.as_bytes());
        }
    }
}

/// Something `name` hasn't found yet, at random (`roll` 0..1), as its place
/// in the catalog; None once it has everything.
pub(super) fn unfound(name: &str, roll: f32) -> Option<usize> {
    let shared = wardrobe();
    let left: Vec<usize> = (0..shared.1.len()).filter(|&k| !owns(&shared.3, name, &shared.1[k].id) && shared.1[k].id != "sleep_mask").collect();
    (!left.is_empty()).then(|| left[(roll * left.len() as f32) as usize % left.len()])
}

/// The picture an item is drawn with, if any: its own, or the props
/// folder's for its kind.
pub fn picture_of(item: &Item) -> Option<PathBuf> {
    item.image.clone().or_else(|| super::rope::prop_image(item.kind))
}

/// The labels of what `name` has found (starters not counted).
pub(super) fn found_labels(name: &str) -> Vec<String> {
    let shared = wardrobe();
    shared.3.get(name).map(|ids| ids.iter().filter_map(|id| shared.1.iter().find(|i| &i.id == id).map(|i| i.label.clone())).collect()).unwrap_or_default()
}

/// A catalog entry by its place: its label, and its picture or drawn kind.
pub(super) fn catalog_entry(k: usize) -> Option<(String, String, Option<PathBuf>, i32)> {
    let shared = wardrobe();
    shared.1.get(k).map(|it| (it.id.clone(), it.label.clone(), it.image.clone(), it.kind))
}

pub fn wardrobe_path() -> PathBuf {
    crate::util::data_dir().join("wardrobe.json")
}

/// Everything there is to wear: the built-in set and the pictures added.
pub fn catalog() -> Vec<Item> {
    let mut v: Vec<Item> = BUILT_IN
        .iter()
        .map(|&(id, label, kind, anchor, size, pivot, offset, behind)| Item {
            id: id.into(),
            label: label.into(),
            kind,
            image: None,
            anchor,
            size,
            pivot,
            offset,
            behind,
            pair: 0.0,
        })
        .collect();
    let props = crate::util::data_dir().join("props");
    for &(id, label, file, anchor, size, pivot, offset, behind, pair) in &PICTURES {
        let image = props.join(format!("{file}.png"));
        if image.is_file() {
            v.push(Item { id: id.into(), label: label.into(), kind: 100, image: Some(image), anchor, size, pivot, offset, behind, pair });
        }
    }
    // weapons in props/weapons: each one held in the hand, handle at the
    // bottom left of its picture, tip up and away
    if let Ok(dir) = std::fs::read_dir(props.join("weapons")) {
        let mut found: Vec<PathBuf> = dir.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "png")).collect();
        found.sort();
        for p in found {
            let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let mut label: String = stem.replace('_', " ");
            if let Some(c) = label.get_mut(0..1) {
                c.make_ascii_uppercase();
            }
            v.push(Item { id: format!("weapon:{stem}"), label, kind: 100, image: Some(p), anchor: Anchor::Hand, size: [0.85, 0.85], pivot: [0.2, 0.8], offset: [0.0, 0.0], behind: false, pair: 0.0 });
        }
    }
    let Ok(dir) = std::fs::read_dir(accessories_dir()) else { return v };
    let mut pngs: Vec<PathBuf> = dir.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"))).collect();
    pngs.sort();
    for png in pngs {
        let stem = png.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let spec: toml::Table = std::fs::read_to_string(png.with_extension("toml")).ok().and_then(|t| t.parse().ok()).unwrap_or_default();
        let pair = |k: &str, d: [f32; 2]| {
            spec.get(k).and_then(|v| v.as_array()).and_then(|a| Some([a.first()?.as_float()? as f32, a.get(1)?.as_float()? as f32])).unwrap_or(d)
        };
        // a square-ish hat by default, as wide as the head, sitting on it
        let (iw, ih) = image::image_dimensions(&png).unwrap_or((1, 1));
        let default_size = [0.8, 0.8 * ih as f32 / iw.max(1) as f32];
        v.push(Item {
            id: format!("custom:{stem}"),
            label: spec.get("label").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| stem.replace(['_', '-'], " ")),
            kind: 100,
            image: Some(png.clone()),
            anchor: spec.get("anchor").and_then(|v| v.as_str()).and_then(Anchor::parse).unwrap_or(Anchor::Crown),
            size: pair("size", default_size),
            pivot: pair("pivot", [0.5, 1.0]),
            offset: pair("offset", [0.0, 0.12]),
            behind: spec.get("behind").and_then(|v| v.as_bool()).unwrap_or(false),
            pair: spec.get("pair").and_then(|v| v.as_float()).unwrap_or(0.0) as f32,
        });
    }
    v
}

/// The wardrobe as saved and the catalog, read again whenever the file or
/// the accessories folder changes; shared, as it's asked for every frame.
type Wardrobe = std::sync::Arc<(HashMap<String, Vec<String>>, Vec<Item>, Fits, Inventory)>;

fn wardrobe() -> Wardrobe {
    type State = (Option<Instant>, Option<SystemTime>, Wardrobe);
    static STATE: std::sync::LazyLock<Mutex<State>> = std::sync::LazyLock::new(|| Mutex::new((None, None, Wardrobe::default())));
    let mut s = STATE.lock().unwrap();
    // checked twice a second: fitting by hand shows on the desktop as it's done
    if s.0.is_none_or(|t| t.elapsed() > Duration::from_millis(500)) {
        s.0 = Some(Instant::now());
        let stamp = std::fs::metadata(wardrobe_path()).and_then(|m| m.modified()).ok();
        let dir_stamp = std::fs::metadata(accessories_dir()).and_then(|m| m.modified()).ok();
        let fit_stamp = std::fs::metadata(fit_path()).and_then(|m| m.modified()).ok();
        let inv_stamp = std::fs::metadata(inventory_path()).and_then(|m| m.modified()).ok();
        let both = stamp.max(dir_stamp).max(fit_stamp).max(inv_stamp);
        if both != s.1 || s.2.1.is_empty() {
            s.1 = both;
            let saved = std::fs::read_to_string(wardrobe_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
            let fits: Fits = std::fs::read_to_string(fit_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
            s.2 = std::sync::Arc::new((saved, catalog(), fits, read_inventory()));
        }
    }
    s.2.clone()
}

/// One thing to draw this frame: prop kind, its box in sprite space
/// (x0, y0, x1, y1 from -1 to 1, y up), mirrored, behind the character,
/// and the picture for kind 100.
#[derive(Clone, Debug)]
pub(super) struct Worn {
    // what it is, for things that can be lost: "umbrella", "weapon", an accessory id, or empty
    pub(super) id: String,
    pub(super) kind: i32,
    pub(super) rect: [f32; 4],
    // turned about its middle (radians), by hand in the fitting
    pub(super) turn: f32,
    pub(super) flip: bool,
    pub(super) behind: bool,
    pub(super) image: Option<PathBuf>,
}

/// Things worn on the head, face, or in the hand that can be knocked off on hard impact.
fn droppable(id: &str, anchor: Anchor) -> String {
    if id == "sleep_mask" || !matches!(anchor, Anchor::Crown | Anchor::Eyes | Anchor::Hand | Anchor::OffHand) {
        return String::new();
    }
    if id.starts_with("weapon:") {
        // weapons fly out of the hand too; one id for all of them is enough to
        // know it's lost and to have it back
        return "weapon".to_string();
    }
    id.to_string()
}

impl CharacterOverlay {
    fn own_name(&self) -> String {
        let path = std::path::Path::new(&self.config.source);
        path.parent().and_then(|p| p.file_name()).or(path.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// What it has on right now: its wardrobe, and what the weather and the
    /// season put on it.
    /// The anchors of the pose drawn now (see `happy_anchors`).
    fn pose_anchors(&self) -> Option<crate::render::anchors::Anchors> {
        let calm = self.anchors?;
        if !self.drawn_happy {
            return Some(calm);
        }
        Some(self.anchors_happy.map_or(calm, |h| happy_anchors(&calm, h)))
    }

    pub(super) fn worn(&self) -> Vec<Worn> {
        let Some(a) = self.pose_anchors() else { return Vec::new() };
        let flip = self.pose.flip;
        let (hw, hh) = (a.head[2] - a.head[0], a.head[3] - a.head[1]);
        if hw <= 0.0 || hh <= 0.0 {
            return Vec::new();
        }
        let crown = anchor_point(&a, Anchor::Crown, self.faces_right);
        // the hand on the side it faces holds things
        let hand = anchor_point(&a, Anchor::Hand, self.faces_right);
        let at = |anchor: Anchor| anchor_point(&a, anchor, self.faces_right);
        // texture units -> sprite space, mirrored as the sprite is
        let sx = |u: f32| (if flip { 1.0 - u } else { u }) * 2.0 - 1.0;
        let sy = |v: f32| 1.0 - 2.0 * v;
        let place = |anchor: Anchor, size: [f32; 2], pivot: [f32; 2], offset: [f32; 2]| {
            let p = at(anchor);
            let (w, h) = (size[0] * hw, size[1] * hh);
            let left = p[0] + offset[0] * hw - pivot[0] * w;
            let top = p[1] + offset[1] * hh - pivot[1] * h;
            let (a, b) = (sx(left), sx(left + w));
            [a.min(b), sy(top + h), a.max(b), sy(top)]
        };
        let mut out: Vec<(f32, Worn)> = Vec::new();
        let wk = &self.walk;
        let asleep = wk.act == Act::Rest && wk.rest.is_some_and(|r| r.kind == super::moods::RestKind::Sleep);
        let shared = wardrobe();
        let (wardrobe, items) = (&shared.0, &shared.1);
        let name = self.own_name();
        let fits = shared.2.get(&name);
        let happy = self.drawn_happy;
        let fit_for = |id: &str| fits.and_then(|f| f.get(&fit_key(id, happy)).or_else(|| f.get(id))).copied();
        let place_with = |it: &Item, fit: Option<[f32; 4]>| {
            let [l, t, w, h] = layout(&a, it, self.faces_right, fit);
            let (x0, x1) = (sx(l), sx(l + w));
            [x0.min(x1), sy(t + h), x0.max(x1), sy(t)]
        };
        let fitted = |it: &Item| {
            let fit = fit_for(&it.id);
            let [l, t, w, h] = layout(&a, it, self.faces_right, fit);
            let (x0, x1) = (sx(l), sx(l + w));
            [x0.min(x1), sy(t + h), x0.max(x1), sy(t)]
        };
        if let Some(ids) = wardrobe.get(&name) {
            for id in ids {
                let Some(it) = items.iter().find(|i| &i.id == id) else { continue };
                // the sleep mask only goes on in its sleep
                if (it.id == "sleep_mask" && !asleep) || wk.lost.iter().any(|l| l == &it.id || (l == "weapon" && it.id.starts_with("weapon:"))) || !owns(&shared.3, &name, &it.id) {
                    continue;
                }
                if it.pair > 0.0 {
                    // a pair: one each side, each side configurable on its own
                    for side in [-1.0f32, 1.0] {
                        let (fit, independent) = fit_of_side(fits, &it.id, side, happy);
                        let (mut one, fit) = pair_copy_side(it, side, fit, independent);
                        one.anchor = anchor_of_side(fits, &it.id, side, happy, it.anchor);
                        let (t, m) = turn_of_side(fits, &it.id, side, happy);
                        let t = if flip { -t } else { t };
                        let behind = behind_of_side(fits, &it.id, side, happy, it.behind);
                        let z = order_of_side(fits, &it.id, side, happy);
                        out.push((z, Worn { turn: t, id: String::new(), kind: it.kind, rect: place_with(&one, fit), flip: flip ^ pair_mirrored(side) ^ m, behind, image: it.image.clone() }));
                    }
                } else {
                    // turned and mirrored by hand; on a mirrored sprite the turn goes the other way
                    let mut one = it.clone();
                    one.anchor = anchor_of(fits, &it.id, happy, it.anchor);
                    let (turn, mirror) = turn_of(fits, &it.id, happy);
                    let turn = if flip { -turn } else { turn };
                    let behind = behind_of(fits, &it.id, happy, it.behind);
                    let z = order_of(fits, &it.id, happy);
                    let image = if it.kind == 100 { picture_for(it, &name) } else { None };
                    out.push((z, Worn { turn, id: droppable(&it.id, one.anchor), kind: it.kind, rect: fitted(&one), flip: flip ^ mirror, behind, image }));
                }
            }
        }
        // its favourite keepsake: hugged to its chest in its sleep, now and
        // then carried on its back
        if let Some(kind) = wk.keepsake {
            if asleep {
                out.push((0.0, Worn { turn: 0.0, id: String::new(), kind, rect: place(Anchor::Chest, [0.46, 0.4], [0.5, 0.5], [0.0, 0.0]), flip, behind: false, image: None }));
            } else if wk.carry && wk.act != Act::Rest {
                out.push((0.0, Worn { turn: 0.0, id: String::new(), kind, rect: place(Anchor::Back, [0.5, 0.44], [0.5, 0.5], [0.0, -0.05]), flip, behind: true, image: None }));
            }
        }
        // a sign held up in both hands, in front of its chest
        if let Some(k) = wk.sign.filter(|_| wk.act == Act::Idle) {
            if let Some(image) = super::sign::picture(k) {
                out.push((0.0, Worn { turn: 0.0, id: String::new(), kind: 100, rect: place(Anchor::Chest, [1.15, 0.75], [0.5, 0.1], [0.0, 0.0]), flip: false, behind: false, image: Some(image) }));
            }
        }
        let weather = super::weather::now();
        let hatted = out.iter().any(|(_, w)| matches!(w.kind, 27 | 28 | 37 | 36) || (w.kind == 100));
        // snow that has settled on its head (or hat)
        let pile = wk.snow_pile;
        if pile >= 0.05 && tn().weather_show >= 0.5 {
            let (size, pivot, offset) = if hatted {
                ([0.45 * (0.7 + 0.3 * pile), 0.1 + 0.2 * pile], [0.5, 0.8], [0.0, -0.3])
            } else {
                ([0.6 + 0.3 * pile, 0.12 + 0.42 * pile], [0.5, 0.8], [0.0, 0.1])
            };
            out.push((0.0, Worn { turn: 0.0, id: String::new(), kind: 24, rect: place(Anchor::Crown, size, pivot, offset), flip: false, behind: false, image: None }));
        }
        // rain: what it does with the umbrella depends on what it's up to
        let has = weather.rain && tn().weather_show >= 0.5 && !wk.lost.iter().any(|l| l == "umbrella");
        let mode = if !has || asleep || wk.act == Act::Rest {
            0
        } else if matches!(wk.act, Act::Travel | Act::Planted) {
            3
        } else {
            wk.brolly
        };
        let tex_aspect = self.sprite_tex.map_or(1.0, |(_, w, h)| w as f32 / h.max(1) as f32);
        // an open one: the top a set size just clear of the head, the stick from
        // it down to `hold`, however far that is
        let mut open = |hold: [f32; 2], behind: bool| {
            let wide = 1.3 * hw;
            let tall = wide * 0.5 * tex_aspect;
            let rim = crown[1] - 0.015;
            let (x0, x1) = (sx(hold[0] - wide * 0.5), sx(hold[0] + wide * 0.5));
            out.push((0.0, Worn { turn: 0.0, id: "umbrella".to_string(), kind: 38, rect: [x0.min(x1), sy(rim), x0.max(x1), sy(rim - tall)], flip: false, behind, image: None }));
            let thin = wide * 0.04;
            let (a0, a1) = (sx(hold[0] - thin * 0.5), sx(hold[0] + thin * 0.5));
            let stick = [a0.min(a1) - (a1 - a0).abs() * 2.5, sy(hold[1] + hh * 0.04), a0.max(a1) + (a1 - a0).abs() * 2.5, sy(rim - tall * 0.9)];
            // the hook turns towards the character
            out.push((0.0, Worn { turn: 0.0, id: "umbrella".to_string(), kind: 39, rect: stick, flip: self.faces_right == flip, behind: true, image: None }));
        };
        match mode {
            1 => open(hand, false),
            // strapped upright to its back, open over its head
            2 => open([a.back[0], a.back[1]], true),
            // rolled up and slung across its back, sticking out past the shoulder
            3 => out.push((0.0, Worn { turn: 0.0, id: String::new(), kind: 42, rect: place(Anchor::Back, [0.85, 1.3], [0.5, 0.5], [0.2, -0.22]), flip, behind: true, image: None })),
            _ => {}
        }
        // a picture fills its box's width at its own shape, about the box's middle
        let (bw, bh) = (self.pose.half[0] * self.pose.size[0], self.pose.half[1] * self.pose.size[1]);
        if bw > 0.0 && bh > 0.0 {
            for (_, w) in out.iter_mut().filter(|(_, w)| w.kind != 38) {
                let picture = w.image.clone().or_else(|| super::rope::prop_image(w.kind));
                let Some(aspect) = picture.and_then(|p| super::rope::picture_aspect(&p)) else { continue };
                let [x0, y0, x1, y1] = w.rect;
                let want = (x1 - x0) * bw / aspect / bh;
                let mid = (y0 + y1) * 0.5;
                w.rect = [x0, mid - want * 0.5, x1, mid + want * 0.5];
            }
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out.into_iter().map(|(_, w)| w).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_anchor_and_order_per_side() {
        let mut map = HashMap::new();
        // arm = 1-based index of Anchor::Arm, z = 3.0
        let arm_idx = Anchor::ALL.iter().position(|&a| a == Anchor::Arm).unwrap() as f32 + 1.0;
        map.insert(meta_key("boots~L", false), [arm_idx, 3.0, 0.0, 0.0]);
        map.insert(meta_key("hat", true), [Anchor::ALL.iter().position(|&a| a == Anchor::Eyes).unwrap() as f32 + 1.0, -2.0, 0.0, 0.0]);

        assert_eq!(anchor_of_side(Some(&map), "boots", -1.0, false, Anchor::Feet), Anchor::Arm);
        assert_eq!(anchor_of_side(Some(&map), "boots", 1.0, false, Anchor::Feet), Anchor::Feet);
        assert_eq!(order_of_side(Some(&map), "boots", -1.0, false), 3.0);
        assert_eq!(order_of_side(Some(&map), "boots", 1.0, false), 0.0);
        assert_eq!(anchor_of(Some(&map), "hat", true, Anchor::Crown), Anchor::Eyes);
        assert_eq!(order_of(Some(&map), "hat", true), -2.0);
    }
}
