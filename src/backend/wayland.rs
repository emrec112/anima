//! Wayland backend: draws wallpapers on the `background` wlr-layer-shell layer,
//! and optionally renders a transparent, click-through `overlay` wlr-layer-shell
//! surface when the 2D character is placed in front of windows (`screen-front`,
//! `window-inside`, `window-outside`).

use std::ffi::c_void;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use calloop::EventLoop;
use calloop_wayland_source::WaylandSource;
use glow::HasContext;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextApi, ContextAttributesBuilder};
use glutin::display::{Display, DisplayApiPreference};
use glutin::prelude::*;
use glutin::surface::{SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{wl_output, wl_surface};
use smithay_client_toolkit::reexports::client::{Connection, Proxy, QueueHandle};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::reexports::client::protocol::{wl_pointer, wl_seat};
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::xdg::window::{Window, WindowConfigure, WindowHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_xdg_shell, delegate_xdg_window, registry_handlers,
};

use crate::audio::AudioAnalyzer;
use crate::config::Config;
use crate::ipc::{self, Command, Request, Response};
use crate::lyrics::{LyricsEngine, fuse_mouth_visemes};
use crate::render::character::{CharacterOverlay, Corner, Placement, SharedSurface, WindowRect};
use crate::render::image::ImageRenderer;
use crate::render::scene::SceneRenderer;
use crate::render::shader::{FallbackRenderer, ShaderRenderer};
use crate::render::video::VideoRenderer;
use crate::render::web::WebRenderer;
use crate::render::{FrameInfo, Renderer};
use crate::util;
use crate::wallpaper::{self, Wallpaper, WallpaperKind};

use self::super::charwin::WorkspaceWindows;
use self::super::governor::{self, GovernorEvent, spawn_governor};

/// Options passed from the CLI to the daemon.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    pub config_path: Option<PathBuf>,
    pub fps: Option<u32>,
    pub verbose: bool,
}

pub fn run(opts: RunOptions) -> Result<()> {
    let config_path = opts
        .config_path
        .clone()
        .unwrap_or_else(util::config_path);
    let mut config = Config::load_or_create(&config_path)?;
    if let Some(fps) = opts.fps {
        config.general.fps = fps.clamp(1, 360);
    }

    let conn = Connection::connect_to_env()
        .context("connecting to the Wayland compositor (is WAYLAND_DISPLAY set?)")?;
    let (globals, mut event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)
        .context("the compositor does not expose wl_compositor")?;
    let layer_shell = LayerShell::bind(&globals, &qh).context(
        "the compositor does not support wlr-layer-shell (GNOME/Mutter does not implement it)",
    )?;
    let output_state = OutputState::new(&globals, &qh);
    let registry_state = RegistryState::new(&globals);
    let seat_state = SeatState::new(&globals, &qh);

    let display_ptr = conn.backend().display_ptr() as *mut c_void;
    if display_ptr.is_null() {
        anyhow::bail!("could not get the wl_display pointer");
    }

    let egl_display = unsafe {
        Display::new(
            RawDisplayHandle::Wayland(
                WaylandDisplayHandle::new(NonNull::new(display_ptr).unwrap()),
            ),
            DisplayApiPreference::Egl,
        )
    }
    .context("creating the EGL display (is libEGL installed?)")?;

    let egl_config = pick_config(&egl_display, false)?;
    let egl_alpha_config = pick_config(&egl_display, true).unwrap_or_else(|_| egl_config.clone());
    let audio = AudioAnalyzer::spawn(false);
    let lyrics = LyricsEngine::spawn(false);

    let socket = util::socket_path();
    let mut app = App {
        registry: registry_state,
        seat_state,
        pointer: None,
        compositor,
        layer_shell,
        xdg_shell: XdgShell::bind(&globals, &qh).ok(),
        output_state,
        qh: qh.clone(),
        egl_display,
        egl_config,
        egl_alpha_config,
        display_ptr,
        config,
        config_path,
        outputs: Vec::new(),
        audio,
        lyrics,
        cursor_pos: None,
        active_window: None,
        focused_monitor: std::env::var("ANIMA_MONITOR").ok(),
        any_fullscreen: false,
        any_maximized: false,
        on_battery: false,
        battery_percent: None,
        last_battery_check: Instant::now() - Duration::from_secs(3600),
        last_playlist_advance: Instant::now(),
        playlist_idx: 0,
        exit: false,
        socket,
        started: Instant::now(),
    };

    let mut event_loop: EventLoop<App> = EventLoop::try_new()?;
    let handle = event_loop.handle();

    let (cmd_tx, cmd_rx) = calloop::channel::channel();
    handle
        .insert_source(cmd_rx, |event, _, app| {
            if let calloop::channel::Event::Msg(cmd) = event {
                app.handle_command(cmd);
            }
        })
        .map_err(|err| anyhow::anyhow!("inserting IPC source: {err}"))?;
    ipc::serve(&app.socket, cmd_tx)?;

    let (gov_tx, gov_rx) = calloop::channel::channel();
    handle
        .insert_source(gov_rx, |event, _, app| {
            if let calloop::channel::Event::Msg(event) = event {
                app.handle_governor(event);
            }
        })
        .map_err(|err| anyhow::anyhow!("inserting governor source: {err}"))?;
    spawn_governor(gov_tx);

    // First roundtrip: discover outputs (this creates the layer surfaces).
    event_queue.roundtrip(&mut app)?;

    let source = WaylandSource::new(conn, event_queue);
    source.insert(handle)?;

    log::info!(
        "anima {} started ({} output(s), socket {})",
        crate::VERSION,
        app.outputs.len(),
        app.socket.display()
    );

    let mut dispatch_errors = 0u32;
    while !app.exit {
        let timeout = app.next_timeout();
        match event_loop.dispatch(Some(timeout), &mut app) {
            Ok(_) => dispatch_errors = 0,
            Err(err) => {
                dispatch_errors += 1;
                log::error!("event loop error: {err:#}");
                if dispatch_errors > 5 {
                    anyhow::bail!("wayland event loop failed repeatedly: {err:#}");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        app.tick();
    }

    app.audio.stop();
    app.lyrics.stop();
    let _ = std::fs::remove_file(&app.socket);
    log::info!("anima stopped");
    Ok(())
}

fn pick_config(display: &Display, transparent: bool) -> Result<glutin::config::Config> {
    let template = ConfigTemplateBuilder::new()
        .with_alpha_size(if transparent { 8 } else { 0 })
        .with_transparency(transparent)
        .with_depth_size(0)
        .with_stencil_size(0)
        .build();
    let configs = unsafe { display.find_configs(template) }?;
    configs
        .reduce(|best, cfg| match (best.num_samples(), cfg.num_samples()) {
            (n, m) if m == 0 && n != 0 => cfg,
            _ => best,
        })
        .context("no suitable EGL framebuffer configuration")
}

/// Dedicated transparent, click-through `Layer::Overlay` surface used when the
/// 2D character is placed in front of windows (`screen-front`, `window-inside`,
/// `window-outside`).
struct FrontOverlayCtx {
    layer: LayerSurface,
    configured: bool,
    frame_pending: bool,
    gl_surface: Option<glutin::surface::Surface<WindowSurface>>,
    gl_context: Option<glutin::context::PossiblyCurrentContext>,
    gl: Option<glow::Context>,
    character: Option<CharacterOverlay>,
    // character boxes drawn last frame (they must be repainted when they move)
    prev_rects: Vec<[f32; 4]>,
    /// Window mode: the characters are drawn into one window per workspace,
    /// placed by the compositor backend; this surface is a 1x1 frame clock.
    windowed: bool,
    windows: WorkspaceWindows,
}

struct OutputCtx {
    output: wl_output::WlOutput,
    layer: LayerSurface,
    name: Option<String>,
    logical_w: i32,
    logical_h: i32,
    width: i32,
    height: i32,
    scale: i32,
    configured: bool,
    paused: bool,
    user_paused: bool,
    gl_surface: Option<glutin::surface::Surface<WindowSurface>>,
    gl_context: Option<glutin::context::PossiblyCurrentContext>,
    gl: Option<glow::Context>,
    renderer: Option<Box<dyn Renderer>>,
    character: Option<CharacterOverlay>,
    front_overlay: Option<FrontOverlayCtx>,
    /// The overlay surface currently has no buffer attached (hidden).
    overlay_hidden: bool,
    wallpaper: Option<Wallpaper>,
    error: Option<String>,
    frame_pending: bool,
    animating: bool,
    due_at: Option<Instant>,
    last_render: Instant,
    start: Instant,
    frames: u64,
    fps_since: Instant,
}

impl OutputCtx {
    fn new(output: wl_output::WlOutput, layer: LayerSurface) -> Self {
        Self {
            output,
            layer,
            name: None,
            logical_w: 0,
            logical_h: 0,
            width: 0,
            height: 0,
            scale: 1,
            configured: false,
            paused: false,
            user_paused: false,
            gl_surface: None,
            gl_context: None,
            gl: None,
            renderer: None,
            character: None,
            front_overlay: None,
            overlay_hidden: true,
            wallpaper: None,
            error: None,
            frame_pending: false,
            animating: true,
            due_at: None,
            last_render: Instant::now(),
            start: Instant::now(),
            frames: 0,
            fps_since: Instant::now(),
        }
    }
}

pub struct App {
    registry: RegistryState,
    seat_state: SeatState,
    pointer: Option<wl_pointer::WlPointer>,
    compositor: CompositorState,
    layer_shell: LayerShell,
    xdg_shell: Option<XdgShell>,
    output_state: OutputState,
    qh: QueueHandle<App>,
    egl_display: Display,
    egl_config: glutin::config::Config,
    egl_alpha_config: glutin::config::Config,
    display_ptr: *mut c_void,

    config: Config,
    config_path: PathBuf,
    outputs: Vec<OutputCtx>,
    audio: AudioAnalyzer,
    lyrics: LyricsEngine,
    cursor_pos: Option<(f32, f32)>,
    active_window: Option<WindowRect>,

    focused_monitor: Option<String>,
    any_fullscreen: bool,
    any_maximized: bool,
    on_battery: bool,
    battery_percent: Option<u8>,
    last_battery_check: Instant,
    last_playlist_advance: Instant,
    playlist_idx: usize,

    exit: bool,
    socket: PathBuf,
    started: Instant,
}

impl App {
    fn output_index_for_surface(&self, surface: &wl_surface::WlSurface) -> Option<(usize, bool)> {
        for (i, oc) in self.outputs.iter().enumerate() {
            if oc.layer.wl_surface().id() == surface.id() {
                return Some((i, false));
            }
            if let Some(fo) = &oc.front_overlay {
                if fo.layer.wl_surface().id() == surface.id() {
                    return Some((i, true));
                }
            }
        }
        None
    }

    fn output_index_for_layer(&self, layer: &LayerSurface) -> Option<(usize, bool)> {
        self.output_index_for_surface(layer.wl_surface())
    }

    fn monitor_name(&self, idx: usize) -> String {
        self.outputs
            .get(idx)
            .and_then(|oc| oc.name.clone())
            .unwrap_or_else(|| format!("output-{idx}"))
    }

    fn effective_fps(&self, idx: usize) -> u32 {
        let name = self.monitor_name(idx);
        let mut fps = self.config.fps_for(&name);
        if self.on_battery {
            fps = fps.min(self.config.general.battery_fps.max(1));
        }

        // A static wallpaper only keeps producing frames because the character
        // overlay is still animating. Cap those redraws (the wallpaper itself
        // does not change) instead of running at the full wallpaper fps.
        if let Some(oc) = self.outputs.get(idx) {
            let wallpaper_animated = oc.renderer.as_ref().is_some_and(|r| r.is_animated());
            let character_active = self.config.character.enabled
                && (oc.character.is_some()
                    || oc.front_overlay.as_ref().is_some_and(|fo| {
                        fo.character.is_some() && !self.outputs[idx].overlay_hidden
                    }));
            if !wallpaper_animated && character_active {
                // nothing else animates: the character's own fps decides
                fps = self.config.character.fps.clamp(1, 360);
                if self.on_battery {
                    fps = fps.min(self.config.general.battery_fps.max(1));
                }
            }
        }
        fps.clamp(1, 360)
    }

    fn update_audio_state(&self) {
        if !self.config.general.audio_capture {
            self.audio.set_enabled(false);
            self.lyrics.set_enabled(false);
            return;
        }
        let mut any_needs_audio = false;
        let mut any_char_audio = false;
        for oc in &self.outputs {
            if oc.paused {
                continue;
            }
            let wp_audio = oc
                .renderer
                .as_ref()
                .map(|r| r.uses_audio())
                .unwrap_or(false);
            let char_audio = oc
                .character
                .as_ref()
                .map(|c| c.uses_audio())
                .unwrap_or(false)
                || oc
                    .front_overlay
                    .as_ref()
                    .and_then(|fo| fo.character.as_ref())
                    .map(|c| c.uses_audio())
                    .unwrap_or(false);
            any_needs_audio |= wp_audio || char_audio;
            any_char_audio |= char_audio;
        }
        self.audio.set_enabled(any_needs_audio);
        self.lyrics.set_enabled(any_char_audio);
    }

    /// Synchronize the front overlay `Layer::Overlay` surface for output `idx`
    /// based on whether the character is enabled and needs a front overlay.
    fn sync_character_surfaces(&mut self, idx: usize) {
        if idx >= self.outputs.len() {
            return;
        }
        let ch_cfg = self.config.character.clone();
        let needs_front = ch_cfg.enabled && ch_cfg.effective_placement().needs_overlay_surface();

        if needs_front {
            // Remove background-layer character so it isn't drawn twice.
            self.outputs[idx].character = None;

            if self.outputs[idx].front_overlay.is_none() {
                let windowed = ch_cfg.windows
                    && ch_cfg.walk
                    && self.xdg_shell.is_some()
                    && governor::can_place_windows();
                governor::WINDOW_MODE.store(windowed, std::sync::atomic::Ordering::Relaxed);
                if windowed {
                    log::info!("characters: one window per workspace, placed by the Hyprland plugin");
                }
                let surface = self.compositor.create_surface(&self.qh);
                // Set an empty 0x0 input region so clicks pass straight through!
                if let Ok(empty_region) = Region::new(&self.compositor) {
                    surface.set_input_region(Some(empty_region.wl_region()));
                }
                let layer = self.layer_shell.create_layer_surface(
                    &self.qh,
                    surface,
                    Layer::Overlay,
                    Some("anima-character"),
                    Some(&self.outputs[idx].output),
                );
                if windowed {
                    layer.set_anchor(Anchor::TOP | Anchor::LEFT);
                    layer.set_size(1, 1);
                } else {
                    layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
                    layer.set_size(0, 0);
                }
                layer.set_exclusive_zone(-1);
                layer.set_keyboard_interactivity(KeyboardInteractivity::None);
                layer.commit();

                self.outputs[idx].overlay_hidden = true;
                self.outputs[idx].front_overlay = Some(FrontOverlayCtx {
                    layer,
                    configured: false,
                    frame_pending: false,
                    gl_surface: None,
                    gl_context: None,
                    gl: None,
                    character: None,
                    prev_rects: Vec::new(),
                    windowed,
                    windows: WorkspaceWindows::default(),
                });
            } else {
                let (w, h) = (self.outputs[idx].width, self.outputs[idx].height);
                if let Some(fo) = self.outputs[idx].front_overlay.as_mut() {
                    if let (Some(ctx), Some(surf), Some(gl)) =
                        (&fo.gl_context, &fo.gl_surface, &fo.gl)
                    {
                        let _ = ctx.make_current(surf);
                        fo.character = CharacterOverlay::new(gl, ch_cfg, w, h).ok();
                    }
                }
            }
        } else {
            // Tear down front overlay surface if it was active.
            if let Some(mut fo) = self.outputs[idx].front_overlay.take() {
                if let (Some(ctx), Some(surf)) = (&fo.gl_context, &fo.gl_surface) {
                    let _ = ctx.make_current(surf);
                }
                fo.character = None;
                fo.gl = None;
                drop(fo.gl_surface.take());
                drop(fo.gl_context.take());
            }

            // Build character in the background wallpaper GL context.
            if let (Some(ctx), Some(surf)) = (
                self.outputs[idx].gl_context.as_ref(),
                self.outputs[idx].gl_surface.as_ref(),
            ) {
                let _ = ctx.make_current(surf);
            }
            if let Some(gl) = self.outputs[idx].gl.as_ref() {
                let (w, h) = (self.outputs[idx].width, self.outputs[idx].height);
                self.outputs[idx].character = CharacterOverlay::new(gl, ch_cfg, w, h).ok();
            }
        }
    }

    fn configure_front_overlay(&mut self, idx: usize) {
        let scale = self.outputs[idx].scale.max(1);
        let width = (self.outputs[idx].logical_w * scale).max(1);
        let height = (self.outputs[idx].logical_h * scale).max(1);
        let ch_cfg = self.config.character.clone();

        let Some(fo) = self.outputs[idx].front_overlay.as_mut() else {
            return;
        };
        // the character still works in output pixels; only the surface shrinks
        let (surf_w, surf_h) = if fo.windowed { (scale, scale) } else { (width, height) };

        // Ensure click-through input region stays empty on configure.
        if let Ok(empty_region) = Region::new(&self.compositor) {
            fo.layer
                .wl_surface()
                .set_input_region(Some(empty_region.wl_region()));
        }

        if fo.gl.is_none() {
            let wl_surface = fo.layer.wl_surface().clone();
            let Some(ptr) = NonNull::new(wl_surface.id().as_ptr().cast()) else {
                return;
            };
            let raw_window = RawWindowHandle::Wayland(WaylandWindowHandle::new(ptr));
            let attrs = ContextAttributesBuilder::new()
                .with_context_api(ContextApi::Gles(None))
                .build(Some(raw_window));
            let Ok(not_current) =
                (unsafe { self.egl_display.create_context(&self.egl_alpha_config, &attrs) })
            else {
                return;
            };
            let (Some(w), Some(h)) = (
                NonZeroU32::new(surf_w as u32),
                NonZeroU32::new(surf_h as u32),
            ) else {
                return;
            };
            let surface_attrs =
                SurfaceAttributesBuilder::<WindowSurface>::new().build(raw_window, w, h);
            let Ok(gl_surface) = (unsafe {
                self.egl_display
                    .create_window_surface(&self.egl_alpha_config, &surface_attrs)
            }) else {
                return;
            };
            let Ok(gl_context) = not_current.make_current(&gl_surface) else {
                return;
            };
            let _ = gl_surface.set_swap_interval(&gl_context, SwapInterval::DontWait);
            let gl = unsafe {
                let display = &self.egl_display;
                glow::Context::from_loader_function_cstr(|name| {
                    display.get_proc_address(name).cast()
                })
            };
            let _ = fo.layer.wl_surface().set_buffer_scale(scale);
            fo.character = CharacterOverlay::new(&gl, ch_cfg, width, height).ok();
            fo.gl_surface = Some(gl_surface);
            fo.gl_context = Some(gl_context);
            fo.gl = Some(gl);
        } else if let (Some(ctx), Some(surface)) = (&fo.gl_context, &fo.gl_surface) {
            if let (Some(w), Some(h)) = (
                NonZeroU32::new(surf_w as u32),
                NonZeroU32::new(surf_h as u32),
            ) {
                let _ = surface.resize(ctx, w, h);
            }
        }
        fo.configured = true;
        if !self.outputs[idx].paused && !self.any_fullscreen {
            self.outputs[idx].overlay_hidden = false;
        }
    }

    // ------------------------------------------------------------------
    // Wallpaper lifecycle
    // ------------------------------------------------------------------

    fn assign_wallpaper(&mut self, idx: usize) {
        let Some(name) = self.outputs[idx].name.clone() else {
            return;
        };
        let path = self.config.wallpaper_for(&name);
        let mut wallpaper = None;
        let mut error = None;

        if let Some(path) = path {
            match Wallpaper::resolve(&path) {
                Ok(w) => wallpaper = Some(w),
                Err(err) => {
                    let msg = format!("{err:#}");
                    log::error!("monitor {name}: {msg}");
                    error = Some(msg);
                }
            }
        }

        self.outputs[idx].wallpaper = wallpaper;
        self.outputs[idx].error = error;
        self.build_renderer(idx);
    }

    fn build_renderer(&mut self, idx: usize) {
        if idx >= self.outputs.len() {
            return;
        }
        if self.outputs[idx].gl.is_none() {
            return;
        }

        let config = &self.config;
        let display_ptr = self.display_ptr;
        let oc = &mut self.outputs[idx];
        if let (Some(ctx), Some(surf)) = (&oc.gl_context, &oc.gl_surface) {
            let _ = ctx.make_current(surf);
        }
        oc.renderer = None;

        let gl = oc.gl.as_ref().unwrap();
        let (w, h) = (oc.width, oc.height);
        let name = oc.name.clone().unwrap_or_default();

        let built: Result<Box<dyn Renderer>> = match &oc.wallpaper {
            None if oc.error.is_none() => Ok(Box::new(
                crate::render::shader::TransparentRenderer::new(w, h),
            ) as Box<dyn Renderer>),
            None => FallbackRenderer::new(
                gl,
                w,
                h,
                "no wallpaper configured — use `anima set <path>`",
            )
            .map(|r| Box::new(r) as Box<dyn Renderer>),
            Some(wp) => match wp.kind {
                WallpaperKind::Image => ImageRenderer::new(gl, &wp.entry, w, h)
                    .map(|r| Box::new(r) as Box<dyn Renderer>),
                WallpaperKind::Shader => ShaderRenderer::new(gl, &wp.entry, w, h)
                    .map(|r| Box::new(r) as Box<dyn Renderer>),
                WallpaperKind::Video => VideoRenderer::new(
                    &wp.entry,
                    w,
                    h,
                    &config.general.hwdec,
                    display_ptr,
                    config.volume_for(&name),
                    config.mute_for(&name),
                )
                .map(|r| Box::new(r) as Box<dyn Renderer>),
                WallpaperKind::Scene => SceneRenderer::new(
                    gl,
                    &wp.root,
                    &wp.entry,
                    w,
                    h,
                    wp.project.as_ref(),
                )
                .map(|r| Box::new(r) as Box<dyn Renderer>),
                WallpaperKind::Web => WebRenderer::new(
                    gl,
                    &wp.root,
                    &wp.entry,
                    w,
                    h,
                    wp.project.as_ref(),
                )
                .map(|r| Box::new(r) as Box<dyn Renderer>),
                WallpaperKind::Application => Err(anyhow::anyhow!(
                    "application wallpapers are not supported on Linux"
                )),
            },
        };

        match built {
            Ok(mut renderer) => {
                renderer.set_paused(oc.paused);
                if let Some(mcfg) = config.monitor(&name) {
                    for (k, v) in &mcfg.properties {
                        let _ = renderer.set_property(k, v);
                    }
                }
                oc.error = None;
                oc.renderer = Some(renderer);
                log::info!(
                    "monitor {name}: {}",
                    oc.wallpaper
                        .as_ref()
                        .map(|w| w.describe())
                        .unwrap_or_default()
                );
            }
            Err(err) => {
                let msg = format!("{err:#}");
                log::error!("monitor {name}: {msg}");
                oc.error = Some(msg.clone());
                oc.renderer = FallbackRenderer::new(gl, w, h, msg)
                    .ok()
                    .map(|r| Box::new(r) as Box<dyn Renderer>);
            }
        }
        oc.last_render = Instant::now() - Duration::from_secs(1);
        self.sync_character_surfaces(idx);
        self.update_audio_state();

        if self.outputs[idx].configured {
            self.outputs[idx].animating = true;
            self.outputs[idx].due_at = None;
            self.render_now(idx);
        }
    }

    // ------------------------------------------------------------------
    // Drawing
    // ------------------------------------------------------------------

    /// Arm the surface that should drive the next frame.
    ///
    /// Normally that is the wallpaper surface. When the wallpaper is static but
    /// a character overlay is animating, only the overlay commits new content,
    /// so its frame callback becomes the clock and the (unchanged) wallpaper
    /// surface is left alone.
    fn arm_frame(&mut self, idx: usize) {
        if idx >= self.outputs.len() {
            return;
        }
        if self.outputs[idx].paused || !self.outputs[idx].configured {
            return;
        }
        if self.overlay_drives(idx) {
            let oc = &mut self.outputs[idx];
            if let Some(fo) = oc.front_overlay.as_mut() {
                if fo.frame_pending {
                    return;
                }
                fo.frame_pending = true;
                let surface = fo.layer.wl_surface().clone();
                let _ = surface.frame(&self.qh, surface.clone());
            }
            return;
        }
        let oc = &mut self.outputs[idx];
        if oc.frame_pending || oc.renderer.is_none() {
            return;
        }
        let surface = oc.layer.wl_surface().clone();
        oc.frame_pending = true;
        let _ = surface.frame(&self.qh, surface.clone());
    }

    /// Whether the character overlay is the only animated content on this
    /// output, i.e. a static wallpaper plus a visible front-layer character.
    fn overlay_drives(&self, idx: usize) -> bool {
        let Some(oc) = self.outputs.get(idx) else {
            return false;
        };
        let static_wallpaper = oc.renderer.as_ref().is_some_and(|r| !r.is_animated());
        static_wallpaper
            && oc.character.is_none()
            && oc.front_overlay.as_ref().is_some_and(|fo| {
                fo.configured && fo.character.is_some() && !oc.overlay_hidden
            })
    }

    fn schedule_render(&mut self, idx: usize) {
        if let Some(oc) = self.outputs.get_mut(idx) {
            if oc.paused || !oc.configured || oc.renderer.is_none() {
                return;
            }
            oc.due_at = Some(Instant::now());
        }
    }

    fn on_frame(&mut self, idx: usize, is_overlay: bool) {
        if idx >= self.outputs.len() {
            return;
        }
        if is_overlay {
            if let Some(fo) = self.outputs[idx].front_overlay.as_mut() {
                fo.frame_pending = false;
            }
            // Stale overlay callback while the wallpaper drives the loop.
            if !self.overlay_drives(idx) {
                return;
            }
        } else {
            self.outputs[idx].frame_pending = false;
            // Stale wallpaper callback while the overlay drives the loop.
            if self.overlay_drives(idx) {
                return;
            }
        }
        if self.outputs[idx].paused || self.outputs[idx].renderer.is_none() {
            return;
        }
        if !self.outputs[idx].animating {
            return;
        }

        let interval = Duration::from_secs_f64(1.0 / self.effective_fps(idx) as f64);
        let due = self.outputs[idx].last_render + interval;
        let now = Instant::now();
        if now < due {
            self.outputs[idx].due_at = Some(due);
            return;
        }
        self.outputs[idx].due_at = None;
        self.render_now(idx);
    }

    fn render_now(&mut self, idx: usize) -> bool {
        let name = self.monitor_name(idx);
        if self.outputs[idx].paused {
            self.outputs[idx].animating = false;
            return false;
        }
        if self.outputs[idx].gl.is_none() {
            return false;
        }
        self.arm_frame(idx);

        let audio_snap = self.audio.snapshot();
        let lyric_snap = self.lyrics.sample();
        let fused_mouth = fuse_mouth_visemes(audio_snap.mouth, &lyric_snap);
        let cursor = self.cursor_pos;
        let active_win = self.active_window;
        let compositor = &self.compositor;
        let (xdg, qh, egl_display, egl_config) = (self.xdg_shell.as_ref(), &self.qh, &self.egl_display, &self.egl_alpha_config);
        let oc = &mut self.outputs[idx];
        let (Some(gl), Some(ctx), Some(gl_surface)) =
            (oc.gl.as_ref(), oc.gl_context.as_ref(), oc.gl_surface.as_ref())
        else {
            return false;
        };

        if let Err(err) = ctx.make_current(gl_surface) {
            log::error!("monitor {name}: make_current failed: {err}");
            return false;
        }

        let mouse = match cursor {
            Some((cx, cy)) => [
                (cx * oc.scale as f32).clamp(0.0, oc.width as f32),
                (cy * oc.scale as f32).clamp(0.0, oc.height as f32),
            ],
            None => [oc.width as f32 * 0.5, oc.height as f32 * 0.5],
        };

        let now = Instant::now();
        let info = FrameInfo {
            width: oc.width,
            height: oc.height,
            time: now.duration_since(oc.start).as_secs_f64(),
            delta: now.duration_since(oc.last_render).as_secs_f64(),
            frame: oc.frames,
            paused: oc.paused,
            audio: audio_snap.bands,
            spectrum: audio_snap.spectrum,
            beat: audio_snap.beat,
            mouth: fused_mouth,
            mouse,
            scale: oc.scale as f32,
        };

        let Some(renderer) = oc.renderer.as_mut() else {
            return false;
        };

        // Static wallpaper already on screen: redrawing it every character
        // frame only burns GPU time, so leave both its buffer and swap alone.
        let skip_wallpaper = oc.frames > 0 && oc.character.is_none() && {
            !renderer.is_animated()
        } && oc.front_overlay.as_ref().is_some_and(|fo| {
            fo.configured && fo.character.is_some() && !oc.overlay_hidden
        });

        let mut keep_going = if skip_wallpaper {
            false
        } else {
            match renderer.render(gl, &info) {
                Ok(mut keep_going) => {
                    if let Some(ch) = oc.character.as_mut() {
                        if ch.config.enabled {
                            let _ = ch.render(gl, &info, active_win, &mut SharedSurface);
                            keep_going = true;
                        }
                    }
                    if let Err(err) = gl_surface.swap_buffers(ctx) {
                        log::warn!("monitor {name}: swap failed: {err}");
                    }
                    oc.last_render = Instant::now();
                    oc.frames += 1;
                    keep_going
                }
                Err(err) => {
                    let msg = format!("{err:#}");
                    log::error!("monitor {name}: render error: {msg}");
                    oc.error = Some(msg);
                    false
                }
            }
        };

        // Also render the transparent front overlay surface if active!
        if let Some(fo) = oc.front_overlay.as_mut() {
            if fo.configured && !oc.overlay_hidden {
                if let (Some(fo_gl), Some(fo_ctx), Some(fo_surf), Some(ch)) = (
                    fo.gl.as_ref(),
                    fo.gl_context.as_ref(),
                    fo.gl_surface.as_ref(),
                    fo.character.as_mut(),
                ) {
                    if fo_ctx.make_current(fo_surf).is_ok() {
                        unsafe {
                            fo_gl.viewport(0, 0, info.width, info.height);
                            fo_gl.clear_color(0.0, 0.0, 0.0, 0.0);
                            fo_gl.clear(glow::COLOR_BUFFER_BIT);
                        }
                        if fo.windowed {
                            if let (Some(xdg), Some(row)) = (xdg, governor::hypr_row()) {
                                let size = (info.width, info.height);
                                fo.windows.sync(fo_gl, row, size, oc.scale, xdg, compositor, qh, egl_display, egl_config, fo_ctx);
                                let _ = fo_ctx.make_current(fo_surf);
                            }
                            if let Some(mut target) = fo.windows.begin_frame(fo_gl) {
                                let _ = ch.render(fo_gl, &info, active_win, &mut target);
                                let mut place = |slot| governor::hypr_place_workspace(slot);
                                fo.windows.present(fo_gl, fo_ctx, compositor, &ch.hit_rects(), &mut place);
                            }
                            let _ = fo_ctx.make_current(fo_surf);
                        } else {
                            let _ = ch.render(fo_gl, &info, active_win, &mut SharedSurface);
                        }
                        // only the characters take clicks, the rest passes through
                        if fo.windowed {
                        } else if let Ok(region) = Region::new(compositor) {
                            let sc = oc.scale.max(1) as f32;
                            for r in ch.hit_rects() {
                                let (x, y) = ((r[0] / sc) as i32, (r[1] / sc) as i32);
                                let (w, h) = (((r[2] - r[0]) / sc) as i32, ((r[3] - r[1]) / sc) as i32);
                                if w > 0 && h > 0 {
                                    region.add(x, y, w, h);
                                }
                            }
                            fo.layer.wl_surface().set_input_region(Some(region.wl_region()));
                        }
                        // tell the compositor only the character boxes changed, not
                        // the whole screen, so it doesn't recomposite everything each
                        // frame (window mode damages its own windows)
                        let rects = if fo.windowed { Vec::new() } else { ch.hit_rects() };
                        let (bw, bh) = (info.width as f32, info.height as f32);
                        let damage: Vec<glutin::surface::Rect> = rects
                            .iter()
                            .chain(fo.prev_rects.iter())
                            .filter(|r| r[2] > r[0] && r[3] > r[1])
                            .map(|r| {
                                // margin for sway/squash; EGL rects start bottom-left
                                const M: f32 = 32.0;
                                let x0 = (r[0] - M).clamp(0.0, bw);
                                let x1 = (r[2] + M).clamp(0.0, bw);
                                let y0 = (r[1] - M).clamp(0.0, bh);
                                let y1 = (r[3] + M).clamp(0.0, bh);
                                glutin::surface::Rect::new(
                                    x0 as i32,
                                    (bh - y1) as i32,
                                    (x1 - x0).ceil() as i32,
                                    (y1 - y0).ceil() as i32,
                                )
                            })
                            .filter(|r| r.width > 0 && r.height > 0)
                            .collect();
                        fo.prev_rects = rects;
                        let swapped = match (fo_surf, fo_ctx) {
                            (
                                glutin::surface::Surface::Egl(s),
                                glutin::context::PossiblyCurrentContext::Egl(c),
                            ) if !damage.is_empty() => s.swap_buffers_with_damage(c, &damage),
                            #[allow(unreachable_patterns)]
                            _ => fo_surf.swap_buffers(fo_ctx),
                        };
                        let _ = swapped;
                        keep_going = true;
                    }
                }
            }
        }

        if skip_wallpaper && keep_going {
            oc.last_render = Instant::now();
            oc.frames += 1;
        }
        // frame rate, for tuning (RUST_LOG=anima::backend::wayland=debug)
        if oc.frames % 300 == 0 && oc.frames > 0 {
            let fps = 300.0 / oc.fps_since.elapsed().as_secs_f32();
            log::debug!("monitor {name}: {fps:.0} fps");
            oc.fps_since = Instant::now();
        }
        oc.animating = keep_going;
        keep_going
    }

    // ------------------------------------------------------------------
    // Pause / power management / playlist
    // ------------------------------------------------------------------

    fn desired_paused(&self, idx: usize) -> bool {
        let g = &self.config.general;
        let global = (g.pause_on_fullscreen && self.any_fullscreen)
            || (g.pause_on_maximized && self.any_maximized)
            || (g.pause_on_battery && self.on_battery)
            || (g.low_battery_pause > 0
                && self
                    .battery_percent
                    .map(|p| p <= g.low_battery_pause)
                    .unwrap_or(false));
        self.outputs[idx].user_paused || global
    }

    fn refresh_pause(&mut self) {
        for idx in 0..self.outputs.len() {
            let desired = self.desired_paused(idx);
            // Never leave a character overlay floating above fullscreen
            // content, even when the wallpaper itself is allowed to keep
            // running.
            let hide_overlay = desired || self.any_fullscreen;
            if self.outputs[idx].front_overlay.is_some() {
                let was_hidden = self.outputs[idx].overlay_hidden;
                if hide_overlay && !was_hidden {
                    self.hide_front_overlay(idx);
                    self.outputs[idx].overlay_hidden = true;
                } else if !hide_overlay && was_hidden {
                    self.outputs[idx].overlay_hidden = false;
                }
            }
            if self.outputs[idx].paused == desired {
                continue;
            }
            self.outputs[idx].paused = desired;
            if let Some(renderer) = &mut self.outputs[idx].renderer {
                renderer.set_paused(desired);
            }
            if desired {
                self.outputs[idx].frame_pending = false;
                self.outputs[idx].due_at = None;
            } else {
                self.outputs[idx].animating = true;
                self.schedule_render(idx);
            }
        }
        self.update_audio_state();
    }

    /// Draw one fully transparent frame over the character overlay.
    ///
    /// Layer-shell surfaces must not be detached with a null buffer: the
    /// compositor treats that as an unmap and rejects the next buffer until a
    /// fresh configure arrives. Clearing to alpha 0 hides the character while
    /// keeping the surface mapped and cheap to re-show.
    fn hide_front_overlay(&mut self, idx: usize) {
        let (width, height) = (self.outputs[idx].width, self.outputs[idx].height);
        let Some(fo) = self.outputs[idx].front_overlay.as_mut() else {
            return;
        };
        // hidden characters must not swallow clicks
        if let Ok(empty) = Region::new(&self.compositor) {
            fo.layer.wl_surface().set_input_region(Some(empty.wl_region()));
        }
        if !fo.configured {
            return;
        }
        let (Some(gl), Some(ctx), Some(surf)) =
            (fo.gl.as_ref(), fo.gl_context.as_ref(), fo.gl_surface.as_ref())
        else {
            return;
        };
        if ctx.make_current(surf).is_err() {
            return;
        }
        unsafe {
            gl.viewport(0, 0, width.max(1), height.max(1));
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
        let _ = surf.swap_buffers(ctx);
    }

    fn refresh_battery(&mut self) {
        let (on_battery, percent) = read_battery();
        if on_battery != self.on_battery || percent != self.battery_percent {
            log::info!(
                "power: {} ({}%)",
                if on_battery { "battery" } else { "AC" },
                percent.map(|p| p.to_string()).unwrap_or_else(|| "?".into())
            );
        }
        self.on_battery = on_battery;
        self.battery_percent = percent;
        self.last_battery_check = Instant::now();
    }

    fn collect_playlist_candidates(&self) -> Vec<PathBuf> {
        if !self.config.general.playlist.is_empty() {
            return self
                .config
                .general
                .playlist
                .iter()
                .map(|p| util::expand_path(p, Some(&util::config_dir())))
                .collect();
        }
        let mut list: Vec<PathBuf> = wallpaper::scan(&self.config.wallpapers_dir())
            .into_values()
            .collect();
        if list.is_empty() {
            list = wallpaper::scan(Path::new("examples/wallpapers"))
                .into_values()
                .collect();
        }
        if list.is_empty() {
            list = wallpaper::scan_workshop()
                .into_iter()
                .map(|w| w.root)
                .collect();
        }
        list
    }

    fn step_playlist(&mut self, monitor: Option<&str>, forward: bool) -> Result<String> {
        let candidates = self.collect_playlist_candidates();
        if candidates.is_empty() {
            anyhow::bail!("no wallpapers found in playlist or wallpapers directory");
        }
        if self.config.general.playlist_shuffle && candidates.len() > 1 {
            let tick = (util::now_secs() * 1000.0) as usize;
            self.playlist_idx =
                (self.playlist_idx + 1 + (tick % (candidates.len() - 1))) % candidates.len();
        } else if forward {
            self.playlist_idx = (self.playlist_idx + 1) % candidates.len();
        } else if self.playlist_idx == 0 {
            self.playlist_idx = candidates.len() - 1;
        } else {
            self.playlist_idx -= 1;
        }

        let chosen = &candidates[self.playlist_idx % candidates.len()];
        let wp = Wallpaper::resolve(chosen)?;
        let targets = self.target_outputs(monitor);
        let mut applied = Vec::new();
        for idx in targets {
            let name = self.monitor_name(idx);
            let cfg = self.config.ensure_monitor(&name);
            cfg.wallpaper = Some(chosen.display().to_string());
            self.outputs[idx].wallpaper = Some(wp.clone());
            self.outputs[idx].error = None;
            self.outputs[idx].frames = 0;
            self.build_renderer(idx);
            self.refresh_pause();
            applied.push(name);
        }
        let _ = self.config.save(&self.config_path);
        self.last_playlist_advance = Instant::now();
        Ok(format!("switched to {} on {}", wp.title, applied.join(", ")))
    }

    fn next_timeout(&self) -> Duration {
        let now = Instant::now();
        let mut timeout = Duration::from_secs(1);
        for oc in &self.outputs {
            if let Some(due) = oc.due_at {
                let wait = due.saturating_duration_since(now);
                timeout = timeout.min(wait);
            }
        }
        timeout
    }

    fn tick(&mut self) {
        if self.last_battery_check.elapsed() > Duration::from_secs(30) {
            if self.config.general.pause_on_battery
                || self.config.general.low_battery_pause > 0
            {
                self.refresh_battery();
            } else {
                self.last_battery_check = Instant::now();
            }
            self.refresh_pause();
        }

        let interval_secs = self.config.general.playlist_interval_secs;
        if interval_secs > 0
            && self.last_playlist_advance.elapsed() >= Duration::from_secs(interval_secs)
        {
            self.last_playlist_advance = Instant::now();
            if let Err(err) = self.step_playlist(None, true) {
                log::debug!("playlist step skipped: {err}");
            }
        }

        let now = Instant::now();
        for idx in 0..self.outputs.len() {
            let oc = &self.outputs[idx];
            if oc.paused || !oc.configured || oc.renderer.is_none() {
                continue;
            }
            if let Some(due) = oc.due_at {
                if now >= due && !oc.frame_pending {
                    self.outputs[idx].due_at = None;
                    self.render_now(idx);
                }
                continue;
            }
            if !oc.animating {
                continue;
            }
            if oc.last_render.elapsed() > Duration::from_secs(2) {
                self.outputs[idx].frame_pending = false;
                self.render_now(idx);
            }
        }
    }

    // ------------------------------------------------------------------
    // Configuration and IPC
    // ------------------------------------------------------------------

    fn pause_targets(&self, monitor: Option<&str>) -> Vec<usize> {
        match monitor {
            Some(name) => self.target_outputs(Some(name)),
            None => (0..self.outputs.len()).collect(),
        }
    }

    fn target_outputs(&self, monitor: Option<&str>) -> Vec<usize> {
        if let Some(name) = monitor {
            if name == "all" || name == "*" {
                return (0..self.outputs.len()).collect();
            }
            return self
                .outputs
                .iter()
                .enumerate()
                .filter(|(_, oc)| oc.name.as_deref() == Some(name))
                .map(|(i, _)| i)
                .collect();
        }
        if let Some(focused) = &self.focused_monitor {
            let matches: Vec<usize> = self
                .outputs
                .iter()
                .enumerate()
                .filter(|(_, oc)| oc.name.as_deref() == Some(focused.as_str()))
                .map(|(i, _)| i)
                .collect();
            if !matches.is_empty() {
                return matches;
            }
        }
        if self.outputs.is_empty() {
            Vec::new()
        } else {
            vec![0]
        }
    }

    fn handle_command(&mut self, cmd: Command) {
        let Command { request, reply } = cmd;
        let response = match request {
            Request::Ping => Response::ok(format!("anima {} is running", crate::VERSION)),
            Request::Quit => {
                self.exit = true;
                Response::ok("shutting down")
            }
            Request::Status { monitor } => {
                let monitors: Vec<serde_json::Value> = self
                    .outputs
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| {
                        monitor
                            .as_deref()
                            .map(|m| self.monitor_name(*i) == m)
                            .unwrap_or(true)
                    })
                    .map(|(i, oc)| {
                        let name = self.monitor_name(i);
                        let props = self
                            .config
                            .monitor(&name)
                            .map(|m| m.properties.clone())
                            .unwrap_or_default();
                        serde_json::json!({
                            "name": name,
                            "size": format!("{}x{}", oc.width, oc.height),
                            "logical_size": format!("{}x{}", oc.logical_w, oc.logical_h),
                            "scale": oc.scale,
                            "configured": oc.configured,
                            "paused": oc.paused,
                            "user_paused": oc.user_paused,
                            "fps": self.effective_fps(i),
                            "frames": oc.frames,
                            "kind": oc.wallpaper.as_ref().map(|w| w.kind.as_str()),
                            "title": oc.wallpaper.as_ref().map(|w| w.title.as_str()),
                            "wallpaper": oc.wallpaper.as_ref().map(|w| w.entry.display().to_string()),
                            "renderer": oc.renderer.as_ref().map(|r| r.describe()),
                            "uses_audio": oc.renderer.as_ref().map(|r| r.uses_audio()).unwrap_or(false),
                            "properties": props,
                            "error": oc.error,
                        })
                    })
                    .collect();
                let lyr = self.lyrics.sample();
                Response::with_data(
                    format!("{} monitor(s)", monitors.len()),
                    serde_json::json!({
                        "version": crate::VERSION,
                        "uptime_secs": self.started.elapsed().as_secs(),
                        "focused_monitor": self.focused_monitor,
                        "fullscreen": self.any_fullscreen,
                        "maximized": self.any_maximized,
                        "on_battery": self.on_battery,
                        "battery_percent": self.battery_percent,
                        "playlist_interval_secs": self.config.general.playlist_interval_secs,
                        "character": self.config.character,
                        "audio": {
                            "level": self.audio.snapshot().bands[3],
                            "bass": self.audio.snapshot().bands[0],
                            "beat": self.audio.snapshot().beat,
                            "playing": self.audio.snapshot().desktop_playing,
                        },
                        "lyrics": {
                            "playing": lyr.playing,
                            "track": format!("{} — {}", lyr.track_artist, lyr.track_title),
                            "synced": lyr.has_synced_lyrics,
                            "active_line": lyr.active_line,
                        },
                        "socket": self.socket.display().to_string(),
                        "monitors": monitors,
                    }),
                )
            }
            Request::ListMonitors => {
                let names: Vec<String> = (0..self.outputs.len())
                    .map(|i| self.monitor_name(i))
                    .collect();
                Response::with_data(
                    names.join(", "),
                    serde_json::json!({ "monitors": names }),
                )
            }
            Request::Set { monitor, path } => {
                let expanded = util::expand_path(&path, std::env::current_dir().ok().as_deref());
                let resolved = Wallpaper::resolve(&expanded)
                    .or_else(|_| Wallpaper::resolve(Path::new(&path)));
                match resolved {
                    Ok(wallpaper) => {
                        let canonical = wallpaper.root.display().to_string();
                        let save_path = if wallpaper.root.join("project.json").exists()
                            || wallpaper.root.join("wallpaper.toml").exists()
                            || wallpaper.root.join("scene.toml").exists()
                        {
                            canonical
                        } else {
                            wallpaper.entry.display().to_string()
                        };
                        let targets = self.target_outputs(monitor.as_deref());
                        let mut applied = Vec::new();
                        for idx in targets {
                            let name = self.monitor_name(idx);
                            let cfg = self.config.ensure_monitor(&name);
                            cfg.wallpaper = Some(save_path.clone());
                            self.outputs[idx].wallpaper = Some(wallpaper.clone());
                            self.outputs[idx].error = None;
                            self.outputs[idx].frames = 0;
                            self.build_renderer(idx);
                            self.refresh_pause();
                            applied.push(name);
                        }
                        if applied.is_empty() {
                            if let Some(name) = monitor.as_deref() {
                                let cfg = self.config.ensure_monitor(name);
                                cfg.wallpaper = Some(save_path);
                            } else {
                                self.config.general.default_wallpaper = Some(save_path);
                            }
                        }
                        if let Err(err) = self.config.save(&self.config_path) {
                            log::error!("saving config: {err:#}");
                        }
                        Response::ok(format!(
                            "set {} [{}] on {}",
                            wallpaper.title,
                            wallpaper.kind.as_str(),
                            if applied.is_empty() {
                                "the default wallpaper".to_string()
                            } else {
                                applied.join(", ")
                            }
                        ))
                    }
                    Err(err) => Response::err(format!("{err:#}")),
                }
            }
            Request::Pause { monitor } => {
                for idx in self.pause_targets(monitor.as_deref()) {
                    self.outputs[idx].user_paused = true;
                }
                self.refresh_pause();
                Response::ok("paused")
            }
            Request::Resume { monitor } => {
                for idx in self.pause_targets(monitor.as_deref()) {
                    self.outputs[idx].user_paused = false;
                }
                self.refresh_pause();
                Response::ok("resumed")
            }
            Request::Toggle { monitor } => {
                let targets = self.pause_targets(monitor.as_deref());
                let pause = targets
                    .first()
                    .map(|i| !self.outputs[*i].user_paused)
                    .unwrap_or(true);
                for idx in targets {
                    self.outputs[idx].user_paused = pause;
                }
                self.refresh_pause();
                Response::ok(if pause { "paused" } else { "resumed" })
            }
            Request::Next { monitor } => match self.step_playlist(monitor.as_deref(), true) {
                Ok(msg) => Response::ok(msg),
                Err(err) => Response::err(format!("{err:#}")),
            },
            Request::Prev { monitor } => match self.step_playlist(monitor.as_deref(), false) {
                Ok(msg) => Response::ok(msg),
                Err(err) => Response::err(format!("{err:#}")),
            },
            Request::Volume {
                monitor,
                volume,
                mute,
            } => {
                let targets = self.pause_targets(monitor.as_deref());
                for idx in targets {
                    let name = self.monitor_name(idx);
                    let mcfg = self.config.ensure_monitor(&name);
                    if let Some(v) = volume {
                        mcfg.volume = Some(v.clamp(0.0, 1.0));
                        if let Some(r) = &mut self.outputs[idx].renderer {
                            r.set_volume(v.clamp(0.0, 1.0));
                        }
                    }
                    if let Some(m) = mute {
                        mcfg.mute = Some(m);
                        if let Some(r) = &mut self.outputs[idx].renderer {
                            let _ = r.set_property("mute", if m { "true" } else { "false" });
                        }
                    }
                }
                let _ = self.config.save(&self.config_path);
                Response::ok("audio settings updated")
            }
            Request::SetProperty {
                monitor,
                key,
                value,
            } => {
                let targets = self.target_outputs(monitor.as_deref());
                for idx in targets {
                    let name = self.monitor_name(idx);
                    let mcfg = self.config.ensure_monitor(&name);
                    mcfg.properties.insert(key.clone(), value.clone());
                    if let Some(r) = &mut self.outputs[idx].renderer {
                        let _ = r.set_property(&key, &value);
                    }
                    if !self.outputs[idx].animating {
                        self.outputs[idx].animating = true;
                        self.schedule_render(idx);
                    }
                }
                self.update_audio_state();
                let _ = self.config.save(&self.config_path);
                Response::ok(format!("property {key} = {value}"))
            }
            Request::ListProperties { monitor } => {
                let targets = self.target_outputs(monitor.as_deref());
                let mut result = Vec::new();
                for idx in targets {
                    let name = self.monitor_name(idx);
                    let overrides = self
                        .config
                        .monitor(&name)
                        .map(|m| m.properties.clone())
                        .unwrap_or_default();
                    let schema = self.outputs[idx]
                        .wallpaper
                        .as_ref()
                        .map(|w| w.properties.clone())
                        .unwrap_or_default();
                    result.push(serde_json::json!({
                        "monitor": name,
                        "wallpaper": self.outputs[idx].wallpaper.as_ref().map(|w| &w.title),
                        "schema": schema,
                        "overrides": overrides,
                    }));
                }
                Response::with_data(
                    "wallpaper properties",
                    serde_json::json!({ "monitors": result }),
                )
            }
            Request::Playlist {
                interval_secs,
                shuffle,
                items,
            } => {
                if let Some(sec) = interval_secs {
                    self.config.general.playlist_interval_secs = sec;
                }
                if let Some(sh) = shuffle {
                    self.config.general.playlist_shuffle = sh;
                }
                if let Some(list) = items {
                    self.config.general.playlist = list;
                }
                self.last_playlist_advance = Instant::now();
                let _ = self.config.save(&self.config_path);
                Response::ok(format!(
                    "playlist updated (interval={}s, shuffle={})",
                    self.config.general.playlist_interval_secs,
                    self.config.general.playlist_shuffle
                ))
            }
            Request::Character {
                enabled,
                source,
                corner,
                placement,
                scale,
                offset_x,
                offset_y,
                opacity,
                flip_x,
                look_at_mouse,
                audio_reactive,
                always_on_top,
            } => {
                let ch = &mut self.config.character;
                if let Some(en) = enabled {
                    ch.enabled = en;
                }
                if let Some(src) = source {
                    ch.source = src;
                    if enabled.is_none() {
                        ch.enabled = true;
                    }
                }
                if let Some(c) = corner {
                    ch.corner = Corner::from_str(&c);
                }
                if let Some(pl) = placement {
                    ch.placement = Placement::from_str(&pl);
                    ch.always_on_top = ch.placement.needs_overlay_surface();
                } else if let Some(aot) = always_on_top {
                    ch.always_on_top = aot;
                    ch.placement = if aot {
                        Placement::ScreenFront
                    } else {
                        Placement::Wallpaper
                    };
                }
                if let Some(s) = scale {
                    ch.scale = s.clamp(0.2, 4.0);
                }
                if let Some(ox) = offset_x {
                    ch.offset[0] = ox;
                }
                if let Some(oy) = offset_y {
                    ch.offset[1] = oy;
                }
                if let Some(op) = opacity {
                    ch.opacity = op.clamp(0.0, 1.0);
                }
                if let Some(fx) = flip_x {
                    ch.flip_x = fx;
                }
                if let Some(lm) = look_at_mouse {
                    ch.look_at_mouse = lm;
                }
                if let Some(ar) = audio_reactive {
                    ch.audio_reactive = ar;
                }

                let new_cfg = self.config.character.clone();
                for idx in 0..self.outputs.len() {
                    self.sync_character_surfaces(idx);
                    self.outputs[idx].animating = true;
                    self.schedule_render(idx);
                }
                self.update_audio_state();
                let _ = self.config.save(&self.config_path);
                Response::with_data(
                    format!(
                        "character: {} ({} @ {} [{}], scale={:.2})",
                        if new_cfg.enabled { "enabled" } else { "disabled" },
                        new_cfg.source,
                        new_cfg.corner.as_str(),
                        new_cfg.effective_placement().as_str(),
                        new_cfg.scale
                    ),
                    serde_json::to_value(&new_cfg).unwrap_or_default(),
                )
            }
            Request::Reload => match Config::load(&self.config_path) {
                Ok(config) => {
                    self.config = config;
                    for idx in 0..self.outputs.len() {
                        self.assign_wallpaper(idx);
                    }
                    self.refresh_pause();
                    Response::ok("configuration reloaded")
                }
                Err(err) => Response::err(format!("{err:#}")),
            },
        };

        if let Some(tx) = reply {
            let _ = tx.send(response);
        }
    }

    fn handle_governor(&mut self, event: GovernorEvent) {
        match event {
            GovernorEvent::WindowState {
                fullscreen,
                maximized,
                active_rect,
            } => {
                self.active_window = active_rect;
                if self.any_fullscreen != fullscreen || self.any_maximized != maximized {
                    log::debug!("fullscreen = {fullscreen}, maximized = {maximized}");
                    self.any_fullscreen = fullscreen;
                    self.any_maximized = maximized;
                    self.refresh_pause();
                }
            }
            GovernorEvent::FocusedMonitor(name) => {
                if self.focused_monitor.as_deref() != Some(name.as_str()) {
                    self.focused_monitor = Some(name);
                }
            }
            GovernorEvent::CursorPos(x, y) => {
                self.cursor_pos = Some((x, y));
            }
            GovernorEvent::Power => {
                self.refresh_battery();
                self.refresh_pause();
            }
        }
    }
}

// ----------------------------------------------------------------------
// Wayland handlers
// ----------------------------------------------------------------------

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        if let Some((idx, _)) = self.output_index_for_surface(surface) {
            self.outputs[idx].scale = new_factor.max(1);
        }
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        if let Some((idx, is_overlay)) = self.output_index_for_surface(surface) {
            self.on_frame(idx, is_overlay);
        }
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Background,
            Some("anima"),
            Some(&output),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(0, 0);
        layer.commit();

        let mut ctx = OutputCtx::new(output.clone(), layer);
        if let Some(info) = self.output_state.info(&output) {
            ctx.name = info.name.clone();
            ctx.scale = info.scale_factor.max(1);
            if let Some((w, h)) = info.logical_size {
                ctx.logical_w = w;
                ctx.logical_h = h;
            }
        }
        log::info!("new output: {:?}", ctx.name);
        self.outputs.push(ctx);
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let Some(idx) = self.outputs.iter().position(|oc| oc.output == output) else {
            return;
        };
        let Some(info) = self.output_state.info(&output) else {
            return;
        };
        let new_scale = info.scale_factor.max(1);
        let name_changed = self.outputs[idx].name != info.name;
        self.outputs[idx].name = info.name.clone();
        self.outputs[idx].scale = new_scale;
        if let Some((w, h)) = info.logical_size {
            if w != self.outputs[idx].logical_w || h != self.outputs[idx].logical_h {
                self.outputs[idx].logical_w = w;
                self.outputs[idx].logical_h = h;
                self.resize_output(idx);
            }
        }
        if name_changed && self.outputs[idx].wallpaper.is_none() {
            self.assign_wallpaper(idx);
        }
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Some(idx) = self.outputs.iter().position(|oc| oc.output == output) {
            self.teardown_output(idx);
        }
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        if let Some((idx, is_overlay)) = self.output_index_for_layer(layer) {
            if is_overlay {
                self.outputs[idx].front_overlay = None;
            } else {
                log::warn!(
                    "layer surface closed by compositor: {:?}",
                    self.outputs[idx].name
                );
                self.teardown_output(idx);
            }
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let Some((idx, is_overlay)) = self.output_index_for_layer(layer) else {
            return;
        };

        if is_overlay {
            self.configure_front_overlay(idx);
            self.outputs[idx].animating = true;
            self.schedule_render(idx);
            return;
        }

        let (super_w, super_h) = configure.new_size;
        let (mut logical_w, mut logical_h) = (super_w as i32, super_h as i32);
        if logical_w == 0 || logical_h == 0 {
            logical_w = self.outputs[idx].logical_w;
            logical_h = self.outputs[idx].logical_h;
        }
        if logical_w == 0 || logical_h == 0 {
            if let Some(info) = self.output_state.info(&self.outputs[idx].output) {
                if let Some((w, h)) = info.logical_size {
                    logical_w = w;
                    logical_h = h;
                    self.outputs[idx].scale = info.scale_factor.max(1);
                }
            }
        }
        if logical_w == 0 || logical_h == 0 {
            logical_w = 1920;
            logical_h = 1080;
        }

        self.outputs[idx].logical_w = logical_w;
        self.outputs[idx].logical_h = logical_h;
        self.outputs[idx].scale = self.outputs[idx].scale.max(1);

        if self.outputs[idx].name.is_none() {
            if let Some(info) = self.output_state.info(&self.outputs[idx].output) {
                self.outputs[idx].name = info.name.clone();
            }
        }

        self.configure_output(idx);
        self.outputs[idx].configured = true;

        if self.outputs[idx].renderer.is_none() {
            self.assign_wallpaper(idx);
        } else {
            self.outputs[idx].animating = true;
            self.schedule_render(idx);
        }
        self.refresh_pause();
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }

    registry_handlers![OutputState, SeatState];
}

impl App {
    fn teardown_output(&mut self, idx: usize) {
        if idx >= self.outputs.len() {
            return;
        }
        log::info!("output removed: {:?}", self.outputs[idx].name);
        let mut ctx = self.outputs.remove(idx);
        if let Some(mut fo) = ctx.front_overlay.take() {
            if let (Some(c), Some(s)) = (&fo.gl_context, &fo.gl_surface) {
                let _ = c.make_current(s);
            }
            fo.character = None;
            fo.gl = None;
        }
        if let (Some(c), Some(s)) = (ctx.gl_context.as_ref(), ctx.gl_surface.as_ref()) {
            let _ = c.make_current(s);
        }
        ctx.renderer = None;
        ctx.character = None;
        ctx.gl = None;
        drop(ctx.gl_surface.take());
        drop(ctx.gl_context.take());
    }

    fn resize_output(&mut self, idx: usize) {
        let oc = &mut self.outputs[idx];
        let scale = oc.scale.max(1);
        let width = oc.logical_w * scale;
        let height = oc.logical_h * scale;
        if width <= 0 || height <= 0 {
            return;
        }
        oc.width = width;
        oc.height = height;

        let _ = oc.layer.wl_surface().set_buffer_scale(scale);

        if let (Some(ctx), Some(surface)) = (oc.gl_context.as_ref(), oc.gl_surface.as_ref()) {
            if let (Some(w), Some(h)) = (
                NonZeroU32::new(width as u32),
                NonZeroU32::new(height as u32),
            ) {
                let _ = surface.resize(ctx, w, h);
            }
        }
        if let (Some(renderer), Some(gl)) = (oc.renderer.as_mut(), oc.gl.as_ref()) {
            let _ = renderer.resize(gl, width, height);
        }
        if let Some(fo) = oc.front_overlay.as_mut() {
            let _ = fo.layer.wl_surface().set_buffer_scale(scale);
            if let (Some(ctx), Some(surface)) = (fo.gl_context.as_ref(), fo.gl_surface.as_ref()) {
                if let (Some(w), Some(h)) = (
                    NonZeroU32::new(width as u32),
                    NonZeroU32::new(height as u32),
                ) {
                    let _ = surface.resize(ctx, w, h);
                }
            }
        }
    }

    fn configure_output(&mut self, idx: usize) {
        let scale = self.outputs[idx].scale.max(1);
        let width = (self.outputs[idx].logical_w * scale).max(1);
        let height = (self.outputs[idx].logical_h * scale).max(1);

        if self.outputs[idx].gl.is_none() {
            let wl_surface = self.outputs[idx].layer.wl_surface().clone();
            let raw_window = {
                let Some(ptr) = NonNull::new(wl_surface.id().as_ptr().cast()) else {
                    log::error!("monitor {}: null wl_surface", self.monitor_name(idx));
                    return;
                };
                RawWindowHandle::Wayland(WaylandWindowHandle::new(ptr))
            };
            let attrs = ContextAttributesBuilder::new()
                .with_context_api(ContextApi::Gles(None))
                .build(Some(raw_window));
            let not_current =
                match unsafe { self.egl_display.create_context(&self.egl_alpha_config, &attrs) } {
                    Ok(ctx) => ctx,
                    Err(err) => {
                        log::error!(
                            "monitor {}: creating GL context failed: {err}",
                            self.monitor_name(idx)
                        );
                        return;
                    }
                };

            let Some(w) = NonZeroU32::new(width as u32) else {
                return;
            };
            let Some(h) = NonZeroU32::new(height as u32) else {
                return;
            };
            let surface_attrs =
                SurfaceAttributesBuilder::<WindowSurface>::new().build(raw_window, w, h);
            let gl_surface = match unsafe {
                self.egl_display
                    .create_window_surface(&self.egl_alpha_config, &surface_attrs)
            } {
                Ok(s) => s,
                Err(err) => {
                    log::error!(
                        "monitor {}: creating EGL surface failed: {err}",
                        self.monitor_name(idx)
                    );
                    return;
                }
            };
            let gl_context = match not_current.make_current(&gl_surface) {
                Ok(c) => c,
                Err(err) => {
                    log::error!(
                        "monitor {}: make_current failed: {err}",
                        self.monitor_name(idx)
                    );
                    return;
                }
            };
            // Pacing is driven by `wl_surface.frame` callbacks, so never block
            // inside `eglSwapBuffers` (headless/virtual outputs may never
            // deliver a VSync event, which would stall the whole event loop).
            let _ = gl_surface.set_swap_interval(&gl_context, SwapInterval::DontWait);

            let gl = unsafe {
                let display = &self.egl_display;
                glow::Context::from_loader_function_cstr(|name| {
                    display.get_proc_address(name).cast()
                })
            };

            self.outputs[idx].width = width;
            self.outputs[idx].height = height;
            self.outputs[idx].gl_surface = Some(gl_surface);
            self.outputs[idx].gl_context = Some(gl_context);
            self.outputs[idx].gl = Some(gl);
            let _ = self.outputs[idx]
                .layer
                .wl_surface()
                .set_buffer_scale(scale);
        } else {
            self.resize_output(idx);
        }
    }
}

fn read_battery() -> (bool, Option<u8>) {
    let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") else {
        return (false, None);
    };
    let mut discharging = false;
    let mut percent = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let kind = std::fs::read_to_string(path.join("type")).unwrap_or_default();
        if kind.trim() != "Battery" {
            continue;
        }
        let status = std::fs::read_to_string(path.join("status")).unwrap_or_default();
        if status.trim() == "Discharging" {
            discharging = true;
        }
        if percent.is_none() {
            percent = std::fs::read_to_string(path.join("capacity"))
                .ok()
                .and_then(|v| v.trim().parse::<u8>().ok());
        }
    }
    (discharging, percent)
}

delegate_compositor!(App);
delegate_xdg_shell!(App);
delegate_xdg_window!(App);

// character windows (window mode, see charwin.rs)
impl WindowHandler for App {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {}

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, window: &Window, _: WindowConfigure, _: u32) {
        for oc in &mut self.outputs {
            if let Some(fo) = oc.front_overlay.as_mut() {
                if fo.windows.configure(window) {
                    return;
                }
            }
        }
    }
}
delegate_seat!(App);
delegate_pointer!(App);

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
    }

    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for App {
    // the pointer only reaches us over a character: use the exact position
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for ev in events {
            if !matches!(ev.kind, PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. }) {
                continue;
            }
            // the overlay covers the output; a character window sits at its
            // last drawn spot
            let Some((oc, (wx, wy))) = self.outputs.iter().find_map(|oc| {
                let fo = oc.front_overlay.as_ref()?;
                if fo.layer.wl_surface() == &ev.surface {
                    return Some((oc, (0, 0)));
                }
                let (x, y) = fo.windows.origin_of(&ev.surface)?;
                let s = oc.scale.max(1);
                Some((oc, (x / s, y / s)))
            }) else {
                continue;
            };
            let (ox, oy) = self
                .output_state
                .info(&oc.output)
                .and_then(|i| i.logical_position)
                .unwrap_or((0, 0));
            let p = ((ox + wx) as f32 + ev.position.0 as f32, (oy + wy) as f32 + ev.position.1 as f32);
            *crate::backend::pointer::SYNC.lock().unwrap() = Some(p);
        }
    }
}
delegate_output!(App);
delegate_layer!(App);
delegate_registry!(App);
