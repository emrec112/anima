//! Environment light for 3D characters: once a second each output is
//! captured with wlr-screencopy and boiled down to a coarse colour grid.
//! `light_at` turns that into the light around a spot on the screen.

use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::sync::Mutex;
use std::time::Duration;

use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

const GW: usize = 16;
const GH: usize = 9;

/// Average linear colour of each cell, row 0 at the top.
struct Grid {
    size: (u32, u32),
    cells: [[f32; 3]; GW * GH],
}

static GRIDS: Mutex<Vec<Grid>> = Mutex::new(Vec::new());

/// Light around a character, in linear RGB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    /// colour of the surroundings, tints the shadows
    pub ambient: [f32; 3],
    /// colour of the brightest part of the screen, tints the lit side
    pub key: [f32; 3],
    /// direction to the key light on screen (x right, y up), length = how
    /// one-sided the light is
    pub dir: [f32; 2],
}

impl Default for Light {
    fn default() -> Self {
        Self { ambient: [0.5; 3], key: [1.0; 3], dir: [-0.45, 0.55] }
    }
}

/// Light at (x, y) pixels on an output of `size` physical pixels; None
/// until that output has been captured.
pub fn light_at(size: (u32, u32), x: f32, y: f32) -> Option<Light> {
    // capture only once a 3D character asks
    static START: std::sync::Once = std::sync::Once::new();
    START.call_once(spawn);
    let grids = GRIDS.lock().unwrap();
    let g = grids.iter().find(|g| g.size == size)?;
    let (w, h) = (size.0 as f32, size.1 as f32);
    let cell = |i: usize| ((i % GW) as f32 + 0.5) * w / GW as f32;
    let row = |i: usize| ((i / GW) as f32 + 0.5) * h / GH as f32;
    let luma = |c: &[f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];

    // ambient: nearby cells count most
    let sigma = w * 0.3;
    let (mut amb, mut wsum) = ([0.0f32; 3], 0.0);
    for (i, c) in g.cells.iter().enumerate() {
        let d2 = (cell(i) - x).powi(2) + (row(i) - y).powi(2);
        let k = (-d2 / (2.0 * sigma * sigma)).exp();
        for j in 0..3 {
            amb[j] += c[j] * k;
        }
        wsum += k;
    }
    let amb = amb.map(|v| v / wsum.max(1e-6));

    // key: the cells clearly brighter than average, and where they are
    let mean = g.cells.iter().map(luma).sum::<f32>() / g.cells.len() as f32;
    let (mut key, mut kw, mut dir) = ([0.0f32; 3], 0.0, [0.0f32; 2]);
    for (i, c) in g.cells.iter().enumerate() {
        let k = (luma(c) - mean).max(0.0).powi(2);
        if k == 0.0 {
            continue;
        }
        for j in 0..3 {
            key[j] += c[j] * k;
        }
        let (dx, dy) = ((cell(i) - x) / w, (y - row(i)) / h);
        let len = (dx * dx + dy * dy).sqrt().max(1e-3);
        dir[0] += dx / len * k;
        dir[1] += dy / len * k;
        kw += k;
    }
    if kw == 0.0 {
        // evenly lit: soft light from the default side
        return Some(Light { ambient: amb, key: amb, dir: Light::default().dir.map(|v| v * 0.3) });
    }
    let key = key.map(|v| v / kw);
    let dir = dir.map(|v| v / kw);
    Some(Light { ambient: amb, key, dir })
}

#[derive(Default)]
struct State {
    shm: Option<wl_shm::WlShm>,
    mgr: Option<ZwlrScreencopyManagerV1>,
    outputs: Vec<wl_output::WlOutput>,
    // the frame being captured
    format: Option<(wl_shm::Format, u32, u32, u32)>,
    buffer_done: bool,
    y_invert: bool,
    done: Option<bool>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(s: &mut Self, reg: &wl_registry::WlRegistry, ev: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = ev {
            match interface.as_str() {
                "wl_shm" => s.shm = Some(reg.bind(name, 1, qh, ())),
                "zwlr_screencopy_manager_v1" => s.mgr = Some(reg.bind(name, version.min(3), qh, ())),
                "wl_output" => s.outputs.push(reg.bind(name, 1, qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for State {
    fn event(s: &mut Self, _: &ZwlrScreencopyFrameV1, ev: zwlr_screencopy_frame_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use zwlr_screencopy_frame_v1::Event;
        match ev {
            Event::Buffer { format: WEnum::Value(f), width, height, stride } if s.format.is_none() => {
                s.format = Some((f, width, height, stride));
            }
            Event::BufferDone => s.buffer_done = true,
            Event::Flags { flags: WEnum::Value(f) } => {
                s.y_invert = f.contains(zwlr_screencopy_frame_v1::Flags::YInvert);
            }
            Event::Ready { .. } => s.done = Some(true),
            Event::Failed => s.done = Some(false),
            _ => {}
        }
    }
}

delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore wl_output::WlOutput);
delegate_noop!(State: ignore ZwlrScreencopyManagerV1);

fn spawn() {
    std::thread::Builder::new()
        .name("anima-envlight".into())
        .spawn(|| {
            if let Err(e) = run() {
                log::info!("envlight: no screen capture ({e}), characters use the default light");
            }
        })
        .ok();
}

fn run() -> anyhow::Result<()> {
    let conn = Connection::connect_to_env()?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut state = State::default();
    queue.roundtrip(&mut state)?;
    anyhow::ensure!(state.mgr.is_some() && state.shm.is_some(), "compositor lacks wlr-screencopy");
    log::info!("envlight: sampling {} output(s) for character lighting", state.outputs.len());
    loop {
        let mut grids = Vec::new();
        for out in state.outputs.clone() {
            if let Some(g) = capture(&mut queue, &mut state, &out)? {
                grids.push(g);
            }
        }
        *GRIDS.lock().unwrap() = grids;
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn capture(queue: &mut wayland_client::EventQueue<State>, state: &mut State, out: &wl_output::WlOutput) -> anyhow::Result<Option<Grid>> {
    let qh = queue.handle();
    state.format = None;
    state.buffer_done = false;
    state.y_invert = false;
    state.done = None;
    let frame = state.mgr.as_ref().unwrap().capture_output(0, out, &qh, ());
    // v3 announces every buffer type, then buffer_done; older ones only shm
    while state.format.is_none() || (frame.version() >= 3 && !state.buffer_done) {
        if state.done.is_some() {
            frame.destroy();
            return Ok(None);
        }
        queue.blocking_dispatch(state)?;
    }
    let (format, w, h, stride) = state.format.unwrap();
    // byte order of a pixel: (r, g, b) index
    let order = match format {
        wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888 => [2, 1, 0],
        wl_shm::Format::Xbgr8888 | wl_shm::Format::Abgr8888 => [0, 1, 2],
        _ => {
            frame.destroy();
            return Ok(None);
        }
    };
    let size = (stride * h) as usize;
    let fd = unsafe {
        let fd = libc::memfd_create(c"anima-envlight".as_ptr(), libc::MFD_CLOEXEC);
        anyhow::ensure!(fd >= 0, "memfd_create failed");
        OwnedFd::from_raw_fd(fd)
    };
    anyhow::ensure!(unsafe { libc::ftruncate(std::os::fd::AsRawFd::as_raw_fd(&fd), size as i64) } == 0, "ftruncate failed");
    let pool = state.shm.as_ref().unwrap().create_pool(fd.as_fd(), size as i32, &qh, ());
    let buffer = pool.create_buffer(0, w as i32, h as i32, stride as i32, format, &qh, ());
    frame.copy(&buffer);
    while state.done.is_none() {
        queue.blocking_dispatch(state)?;
    }
    let ok = state.done == Some(true);
    let grid = ok.then(|| unsafe {
        let p = libc::mmap(std::ptr::null_mut(), size, libc::PROT_READ, libc::MAP_SHARED, std::os::fd::AsRawFd::as_raw_fd(&fd), 0);
        if p == libc::MAP_FAILED {
            return None;
        }
        let px = std::slice::from_raw_parts(p as *const u8, size);
        let g = downsample(px, w, h, stride, order, state.y_invert);
        libc::munmap(p, size);
        Some(g)
    });
    buffer.destroy();
    pool.destroy();
    frame.destroy();
    Ok(grid.flatten())
}

/// Average every 8th pixel of each cell, in linear light.
fn downsample(px: &[u8], w: u32, h: u32, stride: u32, order: [usize; 3], y_invert: bool) -> Grid {
    let lin = |v: u8| (v as f32 / 255.0).powf(2.2);
    let mut cells = [[0.0f32; 3]; GW * GH];
    let mut counts = [0u32; GW * GH];
    for y in (0..h).step_by(8) {
        let row = if y_invert { h - 1 - y } else { y };
        let gy = (y as usize * GH / h as usize).min(GH - 1);
        for x in (0..w).step_by(8) {
            let gx = (x as usize * GW / w as usize).min(GW - 1);
            let i = (row * stride + x * 4) as usize;
            let c = &mut cells[gy * GW + gx];
            for j in 0..3 {
                c[j] += lin(px[i + order[j]]);
            }
            counts[gy * GW + gx] += 1;
        }
    }
    for (c, n) in cells.iter_mut().zip(counts) {
        *c = c.map(|v| v / n.max(1) as f32);
    }
    Grid { size: (w, h), cells }
}

#[cfg(test)]
mod tests {
    /// `ANIMA_TEST_ENVLIGHT=1 cargo test envlight -- --nocapture`
    #[test]
    fn capture_once() {
        if std::env::var_os("ANIMA_TEST_ENVLIGHT").is_none() {
            return;
        }
        super::light_at((1, 1), 0.0, 0.0);
        std::thread::sleep(std::time::Duration::from_secs(3));
        let sizes: Vec<_> = super::GRIDS.lock().unwrap().iter().map(|g| g.size).collect();
        for (w, h) in sizes {
            for (x, y) in [(0.1, 0.5), (0.5, 0.5), (0.9, 0.5)] {
                let l = super::light_at((w, h), x * w as f32, y * h as f32);
                println!("{w}x{h} at ({x}, {y}): {l:?}");
            }
        }
    }
}
