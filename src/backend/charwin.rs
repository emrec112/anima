//! Window mode for the walking characters: they're drawn into one
//! transparent window per workspace, which the compositor slides with its
//! workspace (swipes, animations) like any other window. All characters go
//! into one wide offscreen strip (the workspaces side by side), and each
//! window gets its slice of it, with damage hints so the compositor only
//! recomposites around the characters.
//! Wayland clients can't place their own windows: a compositor-specific
//! placer does that (Hyprland: `governor::hypr_place_workspace` + the plugin
//! in contrib/hyprland-plugin). See docs/character-windows.md.

use std::num::NonZeroU32;
use std::ptr::NonNull;

use glow::HasContext;
use glutin::context::PossiblyCurrentContext;
use glutin::display::Display;
use glutin::prelude::*;
use glutin::surface::{Rect, Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use raw_window_handle::{RawWindowHandle, WaylandWindowHandle};
use smithay_client_toolkit::compositor::{CompositorState, Region};
use smithay_client_toolkit::reexports::client::protocol::wl_surface;
use smithay_client_toolkit::reexports::client::{Proxy, QueueHandle};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::xdg::window::{Window, WindowDecorations};

use super::governor::Row;
use super::wayland::App;
use crate::render::character::ActorTarget;

/// app_id of every character window; titles are `anima-character-<slot>`.
pub const APP_ID: &str = "anima-character";

/// Windows on workspaces nobody looks at are refreshed every this many
/// frames (all of them while a swipe or animation shows the neighbours).
const HIDDEN_EVERY: u64 = 6;

struct WsWindow {
    window: Window,
    configured: bool,
    gl_surface: Option<Surface<WindowSurface>>,
    /// damage not yet presented, output pixels relative to this window
    pending: Vec<[f32; 4]>,
    /// input region last set (logical, window coordinates)
    input: Vec<[i32; 4]>,
}

struct Strip {
    fbo: glow::Framebuffer,
    tex: glow::Texture,
    size: (i32, i32),
}

/// One window per workspace of the row (index = slot in the row).
#[derive(Default)]
pub struct WorkspaceWindows {
    list: Vec<WsWindow>,
    /// output size the windows were made for, physical pixels
    output: (i32, i32),
    scale: i32,
    strip: Option<Strip>,
    row: Option<Row>,
    /// sprite boxes drawn last frame (they need repainting too)
    prev: Vec<[f32; 4]>,
    frame: u64,
}

impl WorkspaceWindows {
    /// Match the windows and the strip to the row of workspaces.
    #[allow(clippy::too_many_arguments)]
    pub fn sync(
        &mut self,
        gl: &glow::Context,
        row: Row,
        output: (i32, i32),
        scale: i32,
        xdg: &XdgShell,
        compositor: &CompositorState,
        qh: &QueueHandle<App>,
        display: &Display,
        config: &glutin::config::Config,
        ctx: &PossiblyCurrentContext,
    ) {
        let s = scale.max(1);
        if output != self.output || s != self.scale {
            self.list.clear();
            self.output = output;
            self.scale = s;
        }
        self.list.truncate(row.count);
        while self.list.len() < row.count {
            let slot = self.list.len();
            self.list.push(create(slot, output, s, xdg, compositor, qh));
        }
        for w in &mut self.list {
            if w.configured && w.gl_surface.is_none() {
                w.gl_surface = egl_surface(w.window.wl_surface(), output, display, config, ctx);
                // first picture: the whole window
                w.pending.push([0.0, 0.0, output.0 as f32, output.1 as f32]);
            }
        }
        self.row = Some(row);

        let width = ((row.count.max(1) - 1) as f32 * row.step * s as f32) as i32 + output.0;
        let size = (width.min(16384), output.1);
        if self.strip.as_ref().is_none_or(|st| st.size != size) {
            if let Some(old) = self.strip.take() {
                unsafe {
                    gl.delete_framebuffer(old.fbo);
                    gl.delete_texture(old.tex);
                }
            }
            self.strip = create_strip(gl, size);
        }
    }

    /// An xdg configure for one of ours; false if the window isn't ours.
    pub fn configure(&mut self, window: &Window) -> bool {
        match self.list.iter_mut().find(|w| &w.window == window) {
            Some(w) => {
                w.configured = true;
                true
            }
            None => false,
        }
    }

    /// Top-left of the window showing `surface` relative to the active
    /// workspace (physical pixels), for turning pointer positions into
    /// output coordinates.
    pub fn origin_of(&self, surface: &wl_surface::WlSurface) -> Option<(i32, i32)> {
        let slot = self.list.iter().position(|w| w.window.wl_surface() == surface)?;
        Some((self.slot_x(slot)? as i32, 0))
    }

    /// Left edge of `slot`'s window relative to the active workspace,
    /// physical pixels.
    fn slot_x(&self, slot: usize) -> Option<f32> {
        let row = self.row?;
        Some((slot as f32 - row.active as f32) * row.step * self.scale as f32)
    }

    /// Bind the strip and clear it; the returned target draws every
    /// character into it. None until the strip exists.
    pub fn begin_frame(&mut self, gl: &glow::Context) -> Option<StripTarget> {
        let strip = self.strip.as_ref()?;
        let row = self.row?;
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(strip.fbo));
            gl.viewport(0, 0, strip.size.0, strip.size.1);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
        // overlay clip space covers the active workspace; the strip covers
        // the whole row
        let (w, sw) = (self.output.0 as f32, strip.size.0 as f32);
        let left = row.active as f32 * row.step * self.scale as f32;
        Some(StripTarget { xform: [w / sw, 1.0, (w + 2.0 * left) / sw - 1.0, 0.0] })
    }

    /// Copy each window's slice of the strip to it and present the windows
    /// whose part changed. `rects` are this frame's sprite boxes (output
    /// pixels, relative to the active workspace); `place(slot)` keeps a
    /// window on its workspace.
    pub fn present(&mut self, gl: &glow::Context, ctx: &PossiblyCurrentContext, compositor: &CompositorState, rects: &[[f32; 4]], place: &mut dyn FnMut(usize)) {
        let (Some(strip), Some(row)) = (self.strip.as_ref(), self.row) else { return };
        self.frame += 1;
        let (ow, oh) = (self.output.0 as f32, self.output.1 as f32);
        let s = self.scale;
        let changed: Vec<[f32; 4]> = rects.iter().chain(&self.prev).copied().collect();
        for slot in 0..self.list.len() {
            let x0 = (slot as f32 - row.active as f32) * row.step * s as f32;
            let w = &mut self.list[slot];
            place(slot);
            // damage and clickable area in window coordinates
            let local = |r: &[f32; 4]| [r[0] - x0, r[1], r[2] - x0, r[3]];
            let inside = |r: &[f32; 4]| r[2] > 0.0 && r[0] < ow && r[3] > 0.0 && r[1] < oh;
            w.pending.extend(changed.iter().map(local).filter(inside));
            let input: Vec<[i32; 4]> = rects
                .iter()
                .map(local)
                .filter(inside)
                .map(|r| [(r[0] / s as f32) as i32, (r[1] / s as f32) as i32, ((r[2] - r[0]) / s as f32) as i32, ((r[3] - r[1]) / s as f32) as i32])
                .collect();
            let visible = slot == row.active || row.sliding;
            let due = visible || (self.frame + slot as u64) % HIDDEN_EVERY == 0;
            let Some(surface) = w.gl_surface.as_ref() else { continue };
            if w.pending.is_empty() || !due || ctx.make_current(surface).is_err() {
                continue;
            }
            if input != w.input {
                if let Ok(region) = Region::new(compositor) {
                    for r in &input {
                        region.add(r[0], r[1], r[2], r[3]);
                    }
                    w.window.wl_surface().set_input_region(Some(region.wl_region()));
                }
                w.input = input;
            }
            let sx = x0 + row.active as f32 * row.step * s as f32;
            unsafe {
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(strip.fbo));
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);
                gl.blit_framebuffer(
                    sx as i32,
                    0,
                    sx as i32 + self.output.0,
                    self.output.1,
                    0,
                    0,
                    self.output.0,
                    self.output.1,
                    glow::COLOR_BUFFER_BIT,
                    glow::NEAREST,
                );
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            }
            // EGL damage rects start at the bottom-left
            let damage: Vec<Rect> = w
                .pending
                .drain(..)
                .map(|r| {
                    let (x, y) = (r[0].max(0.0).floor() as i32, r[1].max(0.0).floor() as i32);
                    let (x1, y1) = (r[2].min(ow).ceil() as i32, r[3].min(oh).ceil() as i32);
                    Rect::new(x, self.output.1 - y1, x1 - x, y1 - y)
                })
                .collect();
            swap_with_damage(surface, ctx, &damage);
        }
        self.prev = rects.to_vec();
    }
}

fn swap_with_damage(surface: &Surface<WindowSurface>, ctx: &PossiblyCurrentContext, damage: &[Rect]) {
    match (surface, ctx) {
        (Surface::Egl(s), PossiblyCurrentContext::Egl(c)) => {
            let _ = s.swap_buffers_with_damage(c, damage);
        }
        #[allow(unreachable_patterns)]
        _ => {
            let _ = surface.swap_buffers(ctx);
        }
    }
}

/// Draws into the strip; the overlay's clip space is squeezed onto the
/// active workspace's part of it.
pub struct StripTarget {
    xform: [f32; 4],
}

impl ActorTarget for StripTarget {
    fn begin(&mut self, _: &glow::Context, _: usize, _: [f32; 4]) -> bool {
        true
    }

    fn end(&mut self, _: &glow::Context, _: usize) {}

    fn clip_xform(&self) -> [f32; 4] {
        self.xform
    }
}

fn create(
    slot: usize,
    output: (i32, i32),
    scale: i32,
    xdg: &XdgShell,
    compositor: &CompositorState,
    qh: &QueueHandle<App>,
) -> WsWindow {
    let surface = compositor.create_surface(qh);
    let window = xdg.create_window(surface, WindowDecorations::None, qh);
    window.set_app_id(APP_ID);
    window.set_title(format!("{APP_ID}-{slot}"));
    let logical = ((output.0 / scale) as u32, (output.1 / scale) as u32);
    window.set_min_size(Some(logical));
    window.set_max_size(Some(logical));
    // nothing clickable until a character stands there
    if let Ok(region) = Region::new(compositor) {
        window.wl_surface().set_input_region(Some(region.wl_region()));
    }
    let _ = window.wl_surface().set_buffer_scale(scale);
    window.commit();
    WsWindow { window, configured: false, gl_surface: None, pending: Vec::new(), input: Vec::new() }
}

fn create_strip(gl: &glow::Context, size: (i32, i32)) -> Option<Strip> {
    unsafe {
        let tex = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, size.0, size.1);
        gl.bind_texture(glow::TEXTURE_2D, None);
        let fbo = gl.create_framebuffer().ok()?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
        let ok = gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE;
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        ok.then_some(Strip { fbo, tex, size })
    }
}

fn egl_surface(
    surface: &wl_surface::WlSurface,
    size: (i32, i32),
    display: &Display,
    config: &glutin::config::Config,
    ctx: &PossiblyCurrentContext,
) -> Option<Surface<WindowSurface>> {
    let ptr = NonNull::new(surface.id().as_ptr().cast())?;
    let raw = RawWindowHandle::Wayland(WaylandWindowHandle::new(ptr));
    let (w, h) = (NonZeroU32::new(size.0 as u32)?, NonZeroU32::new(size.1 as u32)?);
    let attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(raw, w, h);
    let gl_surface = unsafe { display.create_window_surface(config, &attrs) }.ok()?;
    // windows on other workspaces get no frame callbacks: never block on them
    ctx.make_current(&gl_surface).ok()?;
    let _ = gl_surface.set_swap_interval(ctx, SwapInterval::DontWait);
    Some(gl_surface)
}
