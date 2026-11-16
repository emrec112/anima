//! Uma Musume chibi sprites (umamusu.wiki names): a calm "[A]"/_0010 pose,
//! a happy "[B]"/_0011 one next to it, both drawn looking right.

use std::path::{Path, PathBuf};

fn name(path: &Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("")
}

/// Uma sprites look to the right (others are taken to look left).
pub(super) fn faces_right(path: &Path) -> bool {
    let n = name(path);
    n.contains("_0010.") || n.contains("_0011.")
}

/// The happy pose that goes with a calm one, shown while jumping or petted.
pub(super) fn happy_pose(path: &Path) -> Option<PathBuf> {
    let n = name(path);
    n.contains("_0010.")
        .then(|| path.with_file_name(n.replace("[A]", "[B]").replace("_0010.", "_0011.")))
}

/// True when the sibling happy pose is drawn facing the opposite way from the calm sprite
/// (all Uma Musume `[A]` -> `[B]` sprites face left in `[B]` and right in `[A]`).
pub(super) fn happy_opposite(path: &Path) -> bool {
    happy_pose(path).is_some()
}

/// Sleep mask spots set by hand in the settings (`eyes.json`: sprite path ->
/// [x, y] in texture units), read again whenever the file changes.
pub(super) fn eye_fix(source: &str) -> Option<[f32; 2]> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant, SystemTime};
    type Fixes = (Option<Instant>, Option<SystemTime>, std::collections::HashMap<String, [f32; 2]>);
    static FIXES: std::sync::LazyLock<Mutex<Fixes>> = std::sync::LazyLock::new(|| Mutex::new((None, None, std::collections::HashMap::new())));
    let mut f = FIXES.lock().unwrap();
    if f.0.is_none_or(|t| t.elapsed() > Duration::from_secs(1)) {
        f.0 = Some(Instant::now());
        let path = crate::util::data_dir().join("eyes.json");
        let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if stamp != f.1 {
            f.1 = stamp;
            f.2 = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        }
    }
    f.2.get(source).copied()
}


/// The head on a sprite, found from its outline: (left, top, right, neck)
/// in texture units. The top is below the ear tips.
pub(super) fn head_box(img: &image::RgbaImage) -> Option<[f32; 4]> {
    let (w, h) = img.dimensions();
    let (wu, hu) = (w as usize, h as usize);
    let solid = |x: u32, y: u32| img.get_pixel(x, y)[3] > 40;
    let rows: Vec<u32> = (0..h).map(|y| (0..w).filter(|&x| solid(x, y)).count() as u32).collect();
    let top = rows.iter().position(|&c| c > 0)?;
    let bot = rows.iter().rposition(|&c| c > 0)?;
    if bot <= top + 20 {
        return None;
    }
    let all = (bot - top) as f32;
    let smooth: Vec<f32> = (0..hu).map(|y| (y.saturating_sub(3)..(y + 4).min(hu)).map(|k| rows[k] as f32).sum::<f32>() / 7.0).collect();
    let upper = smooth[top..top + (all * 0.5) as usize].iter().copied().fold(0.0, f32::max);
    let htop = (top..bot).find(|&y| smooth[y] >= upper * 0.45)?;
    let (lo, hi) = (top + (all * 0.38) as usize, top + (all * 0.62) as usize);
    let mut neck = (lo..hi).min_by(|&a, &b| smooth[a].total_cmp(&smooth[b]))?;
    if ((neck - htop) as f32) < all * 0.32 {
        neck = htop + (all * 0.4) as usize;
    }
    let head_rows = (neck - htop).max(1) as u32;
    let cols: Vec<usize> = (0..wu).filter(|&x| (htop..neck).filter(|&y| solid(x as u32, y as u32)).count() as u32 * 10 >= head_rows * 4).collect();
    let (hl, hr) = (*cols.first()?, *cols.last()?);
    // the top of the hair itself: the first row across the middle of the head that's solid
    let mid = (hl + hr) / 2;
    let span = ((hr - hl) / 6).max(1);
    let crown = (top..neck).find(|&y| (mid - span..=mid + span).filter(|&x| solid(x as u32, y as u32)).count() > span)?;
    Some([hl as f32 / w as f32, crown as f32 / h as f32, hr as f32 / w as f32, neck as f32 / h as f32])
}

// birthdays from each one's profile on umamusu.wiki (month, day)
const BIRTHDAYS: [(&str, u32, u32); 13] = [
    ("Agnes Tachyon", 4, 13),
    ("Wonder Acute", 3, 14),
    ("Sweep Tosho", 5, 9),
    ("Curren Chan", 3, 31),
    ("Daitaku Helios", 4, 10),
    ("Haru Urara", 2, 27),
    ("Mejiro McQueen", 4, 3),
    ("Nice Nature", 4, 16),
    ("Rice Shower", 3, 5),
    ("Silence Suzuka", 5, 1),
    ("Still in Love", 5, 2),
    ("Kitasan Black", 3, 10),
    ("Agnes Digital", 5, 15),
];

/// Whether today is `name`'s birthday.
pub(super) fn birthday(name: &str, month: u32, day: u32) -> bool {
    BIRTHDAYS.iter().any(|b| b.0 == name && b.1 == month && b.2 == day)
}
