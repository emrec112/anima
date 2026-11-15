//! GLSL fragment shader wallpapers.
//!
//! Supported dialects:
//! * Shadertoy-style `void mainImage(out vec4 color, in vec2 fragCoord)`.
//!   `iTime`, `iResolution`, `iFrame`, `iMouse`, `iDate`, `iAudio`, `iBeat`,
//!   `iChannel0` (64x1 audio spectrum texture) and `iChannel1` (256x256 RGBA
//!   noise texture) are available.
//! * Wallpaper Engine `g_Time`, `g_Resolution`, `g_PointerPosition` uniforms.
//! * Complete GLSL ES 3.0 shaders that define their own `void main()`.

use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result};
use glow::HasContext;

use super::gl::{Filter, Program, Triangle, create_texture};
use super::scene::parse_vec3_str;
use super::{FrameInfo, Renderer};

const VERTEX: &str = r#"#version 300 es
layout(location = 0) in vec2 a_pos;
void main() {
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
"#;

const PRELUDE: &str = r#"#version 300 es
precision highp float;
precision highp int;
out vec4 anima_fragColor;
uniform vec3 iResolution;
uniform float iTime;
uniform float iTimeDelta;
uniform int iFrame;
uniform vec4 iMouse;
uniform vec4 iDate;
uniform vec4 iAudio;
uniform float iBeat;
uniform vec3 u_scheme_color;
uniform sampler2D iChannel0;
uniform sampler2D iChannel1;
#define u_time iTime
#define u_delta iTimeDelta
#define u_resolution iResolution.xy
#define u_frame float(iFrame)
#define u_audio iAudio
#define u_beat iBeat
#define u_mouse iMouse.xy
#define g_Time iTime
#define g_Resolution iResolution
#define fragColor anima_fragColor
#define texture2D texture
"#;

const EPILOGUE: &str = r#"
void main() {
    vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
    mainImage(color, gl_FragCoord.xy);
    anima_fragColor = color;
}
"#;

pub struct ShaderRenderer {
    triangle: Triangle,
    program: Program,
    spectrum_tex: glow::Texture,
    noise_tex: glow::Texture,
    path: PathBuf,
    width: i32,
    height: i32,
    start: Instant,
    last: Instant,
    frame: u64,
    paused: bool,
    paused_at: Option<Instant>,
    speed: f32,
    scheme_color: [f32; 3],
    audio_reactive: bool,
    mtime: Option<SystemTime>,
    last_check: Instant,
    error: Option<String>,
}

impl ShaderRenderer {
    pub fn new(gl: &glow::Context, path: &Path, width: i32, height: i32) -> Result<Self> {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("reading shader {}", path.display()))?;
        let audio_reactive = detect_audio_usage(&source);
        let fragment = wrap(&source)?;
        let program = Program::new(gl, VERTEX, &fragment)
            .with_context(|| format!("compiling shader {}", path.display()))?;
        let triangle = Triangle::new(gl)?;
        let spectrum_tex = create_texture(gl, 64, 1, &[0u8; 64 * 4], Filter::Linear, false)?;
        let noise_tex = create_noise_texture(gl)?;
        Ok(Self {
            triangle,
            program,
            spectrum_tex,
            noise_tex,
            path: path.to_path_buf(),
            width,
            height,
            start: Instant::now(),
            last: Instant::now(),
            frame: 0,
            paused: false,
            paused_at: None,
            speed: 1.0,
            scheme_color: [0.35, 0.65, 1.0],
            audio_reactive,
            mtime: modified(path),
            last_check: Instant::now(),
            error: None,
        })
    }

    /// Recompile when the file changed on disk. Cheap mtime check, throttled by
    /// the caller to about once per second.
    pub fn reload_if_changed(&mut self, gl: &glow::Context) {
        let mtime = modified(&self.path);
        if mtime == self.mtime {
            return;
        }
        self.mtime = mtime;

        let result = std::fs::read_to_string(&self.path)
            .map_err(anyhow::Error::from)
            .and_then(|src| {
                self.audio_reactive = detect_audio_usage(&src);
                wrap(&src)
            })
            .and_then(|frag| Program::new(gl, VERTEX, &frag));

        match result {
            Ok(program) => {
                self.program = program;
                self.error = None;
                self.start = Instant::now();
                self.frame = 0;
                log::info!("reloaded shader {}", self.path.display());
            }
            Err(err) => {
                let msg = format!("{err:#}");
                log::error!("shader reload failed: {msg}");
                self.error = Some(msg);
            }
        }
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

impl Renderer for ShaderRenderer {
    fn render(&mut self, gl: &glow::Context, info: &FrameInfo) -> Result<bool> {
        let now = Instant::now();
        if now.duration_since(self.last_check).as_secs_f32() >= 1.0 {
            self.last_check = now;
            self.reload_if_changed(gl);
        }
        let delta = now.duration_since(self.last).as_secs_f32() * self.speed;
        self.last = now;

        let time = if self.paused {
            self.paused_at
                .unwrap_or(now)
                .duration_since(self.start)
                .as_secs_f32()
                * self.speed
        } else {
            now.duration_since(self.start).as_secs_f32() * self.speed
        };

        if self.audio_reactive {
            let mut spec_rgba = [0u8; 64 * 4];
            for i in 0..64 {
                let v = (info.spectrum[i].clamp(0.0, 1.0) * 255.0) as u8;
                spec_rgba[i * 4] = v;
                spec_rgba[i * 4 + 1] = v;
                spec_rgba[i * 4 + 2] = v;
                spec_rgba[i * 4 + 3] = 255;
            }
            unsafe {
                gl.bind_texture(glow::TEXTURE_2D, Some(self.spectrum_tex));
                gl.tex_sub_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    0,
                    0,
                    64,
                    1,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(&spec_rgba)),
                );
            }
        }

        let date = current_date_vec4();

        unsafe {
            gl.viewport(0, 0, self.width, self.height);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);

            self.program.bind(gl);
            self.program
                .set_vec3(gl, "iResolution", info.width as f32, info.height as f32, 1.0);
            self.program.set_f32(gl, "iTime", time);
            self.program.set_f32(gl, "iTimeDelta", delta.max(0.0));
            self.program
                .set_i32(gl, "iFrame", (self.frame % i32::MAX as u64) as i32);
            self.program.set_vec4(
                gl,
                "iMouse",
                [info.mouse[0], info.height as f32 - info.mouse[1], 0.0, 0.0],
            );
            self.program.set_vec4(gl, "iDate", date);
            self.program.set_vec4(gl, "iAudio", info.audio);
            self.program.set_f32(gl, "iBeat", info.beat);
            self.program.set_vec3(
                gl,
                "u_scheme_color",
                self.scheme_color[0],
                self.scheme_color[1],
                self.scheme_color[2],
            );

            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.spectrum_tex));
            self.program.set_i32(gl, "iChannel0", 0);

            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.noise_tex));
            self.program.set_i32(gl, "iChannel1", 1);

            self.triangle.draw(gl);

            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, None);
        }

        self.frame += 1;
        Ok(true)
    }

    fn resize(&mut self, _gl: &glow::Context, width: i32, height: i32) -> Result<()> {
        self.width = width;
        self.height = height;
        Ok(())
    }

    fn set_paused(&mut self, paused: bool) {
        if paused && !self.paused {
            self.paused_at = Some(Instant::now());
        } else if !paused && self.paused {
            if let Some(at) = self.paused_at.take() {
                self.start += at.elapsed();
            }
        }
        self.paused = paused;
    }

    fn set_volume(&mut self, _volume: f32) {}

    fn set_property(&mut self, key: &str, value: &str) -> Result<()> {
        match key.to_ascii_lowercase().as_str() {
            "speed" => {
                if let Ok(v) = value.parse::<f32>() {
                    self.speed = v.clamp(0.0, 10.0);
                }
            }
            "schemecolor" | "color" => {
                self.scheme_color = parse_vec3_str(value, self.scheme_color);
            }
            _ => {}
        }
        Ok(())
    }

    fn uses_audio(&self) -> bool {
        self.audio_reactive
    }

    fn describe(&self) -> String {
        format!("shader {}", self.path.display())
    }

    fn destroy(&mut self, gl: &glow::Context) {
        unsafe {
            gl.delete_texture(self.spectrum_tex);
            gl.delete_texture(self.noise_tex);
            gl.delete_vertex_array(self.triangle.vao);
            gl.delete_buffer(self.triangle.vbo);
        }
        self.program.destroy(gl);
    }
}

fn detect_audio_usage(source: &str) -> bool {
    source.contains("iAudio")
        || source.contains("u_audio")
        || source.contains("iBeat")
        || source.contains("u_beat")
        || source.contains("iChannel0")
}

fn create_noise_texture(gl: &glow::Context) -> Result<glow::Texture> {
    let mut pixels = vec![0u8; 128 * 128 * 4];
    let mut s = 0x1234_5678u32;
    for b in &mut pixels {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        *b = (s & 0xFF) as u8;
    }
    let tex = create_texture(gl, 128, 128, &pixels, Filter::Linear, false)?;
    unsafe {
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::REPEAT as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::REPEAT as i32);
        gl.bind_texture(glow::TEXTURE_2D, None);
    }
    Ok(tex)
}

fn current_date_vec4() -> [f32; 4] {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        let secs = (tm.tm_hour * 3600 + tm.tm_min * 60 + tm.tm_sec) as f32;
        [
            (tm.tm_year + 1900) as f32,
            tm.tm_mon as f32,
            tm.tm_mday as f32,
            secs,
        ]
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

/// Build a complete fragment shader source from user code.
fn wrap(source: &str) -> Result<String> {
    let body = strip_version(source);

    if body.contains("mainImage") {
        Ok(format!("{PRELUDE}\n{body}\n{EPILOGUE}"))
    } else if body.contains("void main") {
        Ok(format!("{PRELUDE}\n{body}\n"))
    } else {
        anyhow::bail!(
            "shader must define `void mainImage(out vec4 color, in vec2 fragCoord)` \
             or a complete `void main()`"
        )
    }
}

fn strip_version(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        if line.trim_start().starts_with("#version") {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Fallback renderer: a dim, slowly breathing gradient. Used when a wallpaper
/// cannot be loaded so the screen is never left black without feedback.
pub struct FallbackRenderer {
    program: Program,
    triangle: Triangle,
    width: i32,
    height: i32,
    start: Instant,
    message: String,
}

const FALLBACK_FRAG: &str = r#"#version 300 es
precision highp float;
uniform vec2 u_resolution;
uniform float u_time;
out vec4 fragColor;
void main() {
    vec2 uv = gl_FragCoord.xy / u_resolution;
    float glow = 0.5 + 0.5 * sin(u_time * 1.2 + uv.x * 3.0 + uv.y * 2.0);
    vec3 col = mix(vec3(0.03, 0.05, 0.09), vec3(0.12, 0.10, 0.22), glow * 0.6);
    float vignette = smoothstep(0.9, 0.2, length(uv - 0.5));
    fragColor = vec4(col * vignette, 1.0);
}
"#;

impl FallbackRenderer {
    pub fn new(
        gl: &glow::Context,
        width: i32,
        height: i32,
        message: impl Into<String>,
    ) -> Result<Self> {
        Ok(Self {
            program: Program::new(gl, VERTEX, FALLBACK_FRAG)?,
            triangle: Triangle::new(gl)?,
            width,
            height,
            start: Instant::now(),
            message: message.into(),
        })
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl Renderer for FallbackRenderer {
    fn render(&mut self, gl: &glow::Context, info: &FrameInfo) -> Result<bool> {
        unsafe {
            gl.viewport(0, 0, self.width, self.height);
            self.program.bind(gl);
            self.program.set_vec2(
                gl,
                "u_resolution",
                self.width as f32,
                self.height as f32,
            );
            self.program
                .set_f32(gl, "u_time", self.start.elapsed().as_secs_f32());
            self.triangle.draw(gl);
        }
        let _ = info;
        Ok(true)
    }

    fn resize(&mut self, _gl: &glow::Context, width: i32, height: i32) -> Result<()> {
        self.width = width;
        self.height = height;
        Ok(())
    }

    fn set_paused(&mut self, _paused: bool) {}

    fn set_volume(&mut self, _volume: f32) {}

    fn describe(&self) -> String {
        format!("fallback: {}", self.message)
    }

    fn is_animated(&self) -> bool {
        true
    }

    fn destroy(&mut self, gl: &glow::Context) {
        unsafe {
            gl.delete_vertex_array(self.triangle.vao);
            gl.delete_buffer(self.triangle.vbo);
        }
        self.program.destroy(gl);
    }
}

/// Fully transparent background, used when no wallpaper is set so the
/// compositor/shell wallpaper underneath stays visible.
pub struct TransparentRenderer {
    width: i32,
    height: i32,
}

impl TransparentRenderer {
    pub fn new(width: i32, height: i32) -> Self {
        Self { width, height }
    }
}

impl Renderer for TransparentRenderer {
    fn render(&mut self, gl: &glow::Context, _info: &FrameInfo) -> Result<bool> {
        unsafe {
            gl.viewport(0, 0, self.width, self.height);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
        Ok(false)
    }

    fn resize(&mut self, _gl: &glow::Context, width: i32, height: i32) -> Result<()> {
        self.width = width;
        self.height = height;
        Ok(())
    }

    fn set_paused(&mut self, _paused: bool) {}

    fn set_volume(&mut self, _volume: f32) {}

    fn describe(&self) -> String {
        "none (transparent)".into()
    }

    fn is_animated(&self) -> bool {
        false
    }
}
