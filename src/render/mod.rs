use anyhow::Result;

pub mod character;
pub mod gl;
pub mod image;
pub mod model3d;
pub mod scene;
pub mod shader;
pub mod video;
pub mod web;

/// Per-frame state handed to every renderer.
#[derive(Debug, Clone)]
pub struct FrameInfo {
    /// Render target size in physical pixels.
    pub width: i32,
    pub height: i32,
    /// Seconds since the wallpaper was loaded.
    pub time: f64,
    /// Seconds since the previous rendered frame.
    pub delta: f64,
    /// Monotonic frame counter.
    pub frame: u64,
    /// Wallpaper is currently frozen (fullscreen window, battery, user pause).
    pub paused: bool,
    /// Low / mid / high / level audio bands, 0.0..=1.0.
    pub audio: [f32; 4],
    /// 64 logarithmic frequency spectrum bars, 0.0..=1.0.
    pub spectrum: [f32; 64],
    /// Beat / transient onset pulse, 0.0..=1.0.
    pub beat: f32,
    /// Fused vocal viseme parameters: `[mouth_open (0..1), mouth_form (-1..1), sibilance (0..1), active (0..1)]`.
    pub mouth: [f32; 4],
    /// Cursor position in physical pixels, relative to this output.
    pub mouse: [f32; 2],
    /// Content scale (HiDPI factor).
    pub scale: f32,
}

/// A renderer owns its GL objects and draws one frame into the current
/// framebuffer (FBO 0).
pub trait Renderer {
    /// Draw a frame. Returns whether another frame should be requested soon.
    fn render(&mut self, gl: &glow::Context, frame: &FrameInfo) -> Result<bool>;

    fn resize(&mut self, gl: &glow::Context, width: i32, height: i32) -> Result<()>;

    /// Called when the pause state changes so renderers can stop decoding.
    fn set_paused(&mut self, paused: bool);

    fn set_volume(&mut self, volume: f32);

    /// Apply a runtime user property (`anima prop set <key> <value>`).
    fn set_property(&mut self, _key: &str, _value: &str) -> Result<()> {
        Ok(())
    }

    /// Whether this renderer reacts to desktop audio (so the audio analyzer can
    /// sleep at 0% CPU when no output needs it).
    fn uses_audio(&self) -> bool {
        false
    }

    /// Short description for `anima status`.
    fn describe(&self) -> String;

    /// Whether this renderer keeps producing frames on its own.
    fn is_animated(&self) -> bool {
        true
    }
}
