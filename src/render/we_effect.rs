//! Wallpaper Engine effect pipeline.
//!
//! Wallpaper Engine scenes describe effects as a list of passes.  Each pass
//! renders a material shader into either the layer canvas or a named render
//! target (`fbos`), with other named targets bound to texture slots.  The
//! shaders themselves are near-GLSL with a small HLSL compatibility layer
//! (`mul`, `CAST2`, `texSample2D`, `saturate`, ...).  This module implements
//! that layer: shader preprocessing, material/pass merging, framebuffers,
//! texture binding and uniform reflection.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use glow::HasContext;
use serde_json::Value;

use super::gl::{bytemuck_cast, create_texture_ex, Filter, Program};
use crate::pkg::PackageFs;

// ---------------------------------------------------------------------------
// Small GL render targets
// ---------------------------------------------------------------------------

pub struct Canvas {
    pub tex: glow::Texture,
    pub fbo: glow::Framebuffer,
    pub w: i32,
    pub h: i32,
}

impl Canvas {
    pub fn new(gl: &glow::Context, w: i32, h: i32) -> Result<Self> {
        let w = w.max(1);
        let h = h.max(1);
        unsafe {
            let tex = gl.create_texture().map_err(|e| anyhow!(e))?;
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                w,
                h,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
            gl.bind_texture(glow::TEXTURE_2D, None);

            let fbo = gl.create_framebuffer().map_err(|e| anyhow!(e))?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(tex),
                0,
            );
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            if status != glow::FRAMEBUFFER_COMPLETE {
                bail!("framebuffer incomplete: {status:#x}");
            }
            Ok(Self { tex, fbo, w, h })
        }
    }

    pub fn bind(&self, gl: &glow::Context) {
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.viewport(0, 0, self.w, self.h);
        }
    }

    /// Release the GPU resources backing this canvas.  `Canvas` has no `Drop`
    /// impl because deletion needs a current GL context, so every place that
    /// discards one must call this explicitly.
    pub fn destroy(self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.fbo);
            gl.delete_texture(self.tex);
        }
    }
}

// ---------------------------------------------------------------------------
// Texture cache / built-in utility textures
// ---------------------------------------------------------------------------

struct CachedTexture {
    tex: glow::Texture,
    w: u32,
    h: u32,
    spritesheet: Option<crate::pkg::SpritesheetMeta>,
    is_r8: bool,
}

pub struct TextureCache {
    map: HashMap<String, Option<CachedTexture>>,
}

impl TextureCache {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// Delete every cached texture.  Requires a current GL context.
    pub fn clear(&mut self, gl: &glow::Context) {
        for (_, cached) in self.map.drain() {
            if let Some(t) = cached {
                unsafe { gl.delete_texture(t.tex) };
            }
        }
    }

    pub fn get(
        &mut self,
        gl: &glow::Context,
        pkg: &PackageFs,
        path: &str,
    ) -> Option<(glow::Texture, u32, u32)> {
        self.get_spritesheet(gl, pkg, path)
            .map(|(tex, w, h, _, _)| (tex, w, h))
    }

    pub fn get_spritesheet(
        &mut self,
        gl: &glow::Context,
        pkg: &PackageFs,
        path: &str,
    ) -> Option<(glow::Texture, u32, u32, Option<crate::pkg::SpritesheetMeta>, bool)> {
        let key = normalize(path);
        if let Some(cached) = self.map.get(&key) {
            return cached
                .as_ref()
                .map(|t| (t.tex, t.w, t.h, t.spritesheet, t.is_r8));
        }

        let loaded = match pkg.load_rgba(&key) {
            Ok(decoded) => {
                let (w, h) = (decoded.image.width(), decoded.image.height());
                let spritesheet = decoded.spritesheet;
                let is_r8 = decoded.is_r8;
                let filter = if decoded.no_interpolation {
                    Filter::Nearest
                } else {
                    Filter::Linear
                };
                create_texture_ex(
                    gl,
                    w,
                    h,
                    decoded.image.as_raw(),
                    filter,
                    true,
                    decoded.clamp_uvs,
                )
                .ok()
                .map(|tex| CachedTexture {
                    tex,
                    w,
                    h,
                    spritesheet,
                    is_r8,
                })
            }
            Err(err) => {
                if let Some((w, h, pixels)) = builtin_texture(&key) {
                    let clamp = matches!(
                        key.to_ascii_lowercase().as_str(),
                        "util/white" | "util/black" | "util/transparent" | "util/noflow"
                    );
                    create_texture_ex(gl, w, h, &pixels, Filter::Linear, !clamp, clamp)
                        .ok()
                        .map(|tex| CachedTexture {
                            tex,
                            w,
                            h,
                            spritesheet: None,
                            is_r8: false,
                        })
                } else {
                    log::debug!("effect texture {key}: {err}");
                    // Unknown `util/*` helpers are almost always masks; white is
                    // a safe "fully visible" default.
                    if key.starts_with("util/") {
                        builtin_texture("util/white").and_then(|(w, h, px)| {
                            create_texture_ex(gl, w, h, &px, Filter::Linear, false, true)
                                .ok()
                                .map(|tex| CachedTexture {
                                    tex,
                                    w,
                                    h,
                                    spritesheet: None,
                                    is_r8: false,
                                })
                        })
                    } else {
                        None
                    }
                }
            }
        };

        self.map.insert(key.clone(), loaded);
        self.map
            .get(&key)
            .and_then(|v| v.as_ref())
            .map(|t| (t.tex, t.w, t.h, t.spritesheet, t.is_r8))
    }
}

fn normalize(path: &str) -> String {
    let p = path.trim().trim_start_matches('/');
    let p = p.strip_suffix(".tex").unwrap_or(p);
    let p = p.strip_suffix(".png").unwrap_or(p);
    let p = p.strip_suffix(".jpg").unwrap_or(p);
    p.to_string()
}

fn builtin_texture(key: &str) -> Option<(u32, u32, Vec<u8>)> {
    let key = key.to_ascii_lowercase();
    match key.as_str() {
        "util/white" | "util/white.tex" => Some((4, 4, vec![255; 4 * 4 * 4])),
        "util/black" => Some((4, 4, vec![0, 0, 0, 255].repeat(16))),
        "util/transparent" => Some((4, 4, vec![0; 4 * 4 * 4])),
        "util/noflow" => {
            let mut px = Vec::with_capacity(64 * 64 * 4);
            for _ in 0..64 * 64 {
                px.extend_from_slice(&[128, 128, 255, 255]);
            }
            Some((64, 64, px))
        }
        "util/noise" => Some((256, 256, noise_rgba(256, 256, 0x1234_5678))),
        "util/clouds_256" | "util/clouds" => Some((256, 256, cloud_rgba(256))),
        "util/waterripplenormal" | "util/waternormal" => Some((256, 256, normal_rgba(256))),
        _ => None,
    }
}

fn noise_rgba(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    let mut s = seed | 1;
    for _ in 0..w * h {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        let v = (s >> 24) as u8;
        out.extend_from_slice(&[v, v, v, 255]);
    }
    out
}

fn cloud_rgba(size: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    let mut seed = 0xA53C_91D7u32;
    let mut lattice = vec![0.0f32; 32 * 32];
    for v in &mut lattice {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        *v = (seed >> 8) as f32 / 16_777_215.0;
    }
    let sample = |x: f32, y: f32, freq: i32| -> f32 {
        let fx = x * freq as f32;
        let fy = y * freq as f32;
        let x0 = fx.floor() as i32;
        let y0 = fy.floor() as i32;
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;
        let sx = tx * tx * (3.0 - 2.0 * tx);
        let sy = ty * ty * (3.0 - 2.0 * ty);
        let at = |ix: i32, iy: i32| {
            let ix = ix.rem_euclid(freq) as usize;
            let iy = iy.rem_euclid(freq) as usize;
            lattice[iy * 32 + ix]
        };
        let a = at(x0, y0);
        let b = at(x0 + 1, y0);
        let c = at(x0, y0 + 1);
        let d = at(x0 + 1, y0 + 1);
        let top = a + (b - a) * sx;
        let bottom = c + (d - c) * sx;
        top + (bottom - top) * sy
    };
    for y in 0..size {
        for x in 0..size {
            let u = x as f32 / size as f32;
            let v = y as f32 / size as f32;
            let mut n = 0.0;
            n += sample(u, v, 3) * 0.5;
            n += sample(u, v, 7) * 0.25;
            n += sample(u, v, 15) * 0.125;
            n += sample(u, v, 31) * 0.0625;
            let v8 = (n.clamp(0.0, 1.0) * 255.0) as u8;
            out.extend_from_slice(&[v8, v8, v8, 255]);
        }
    }
    out
}

fn normal_rgba(size: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let u = x as f32 / size as f32 * std::f32::consts::TAU;
            let v = y as f32 / size as f32 * std::f32::consts::TAU;
            let dx = (u * 7.0).cos() * 0.35 + (v * 5.0).sin() * 0.25 + (u * 13.0 + v * 3.0).sin() * 0.12;
            let dy = (v * 9.0).cos() * 0.35 + (u * 6.0).sin() * 0.25 + (u * 4.0 + v * 11.0).cos() * 0.12;
            let n = (1.0 / (1.0 + dx * dx + dy * dy).sqrt(), dx, dy);
            out.extend_from_slice(&[
                ((n.1 * 0.5 + 0.5) * 255.0) as u8,
                ((n.2 * 0.5 + 0.5) * 255.0) as u8,
                ((n.0 * 0.5 + 0.5) * 255.0) as u8,
                255,
            ]);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Shader sources: preprocessor + HLSL compatibility layer
// ---------------------------------------------------------------------------

const PRELUDE: &str = r#"
#define M_PI 3.14159265359
#define M_PI_HALF 1.57079632679
#define M_PI_2 6.28318530718
#define SQRT_2 1.41421356237
#define SQRT_3 1.73205080756
#define CAST2(x) vec2(x)
#define CAST3(x) vec3(x)
#define CAST4(x) vec4(x)
#define CAST3X3(x) mat3(x)
#define CASTF(x) float(x)
#define CASTI(x) int(x)
#define CASTU(x) uint(x)
#define texSample2D(t, uv) texture(t, (uv).xy)
#define texSample2DLod(t, uv, l) textureLod(t, (uv).xy, float(l))
#define frac(x) fract(x)
#define fmod(x, y) mod(x, y)
#define lerp(a, b, t) we_mix(a, b, t)
#define atan2(y, x) atan(y, x)
#define ddx(x) dFdx(x)
#define ddy(x) dFdy(x)
#define log10(x) (log2(x) * 0.3010299956639812)
#define TEX0FORMAT 0
#define TEX1FORMAT 0
#define TEX2FORMAT 0
#define TEX3FORMAT 0
#define TEX4FORMAT 0
#define TEX5FORMAT 0
#define TEX6FORMAT 0
#define TEX7FORMAT 0

float saturate(float x) { return clamp(x, 0.0, 1.0); }
vec2 saturate(vec2 x) { return clamp(x, 0.0, 1.0); }
vec3 saturate(vec3 x) { return clamp(x, 0.0, 1.0); }
vec4 saturate(vec4 x) { return clamp(x, 0.0, 1.0); }

// HLSL implicitly converts between vector sizes in `mix`; GLSL ES neither
// allows implicit conversion nor overloading a built-in name, so route every
// call through our own helper.
vec2 we_mix(vec2 a, vec2 b, float t) { return mix(a, b, t); }
vec3 we_mix(vec3 a, vec3 b, float t) { return mix(a, b, t); }
vec4 we_mix(vec4 a, vec4 b, float t) { return mix(a, b, t); }
float we_mix(float a, float b, float t) { return mix(a, b, t); }
vec2 we_mix(vec2 a, vec2 b, vec2 t) { return mix(a, b, t); }
vec3 we_mix(vec3 a, vec3 b, vec3 t) { return mix(a, b, t); }
vec4 we_mix(vec4 a, vec4 b, vec4 t) { return mix(a, b, t); }
vec3 we_mix(vec4 a, vec3 b, float t) { return mix(a.rgb, b, t); }
vec3 we_mix(vec3 a, vec4 b, float t) { return mix(a, b.rgb, t); }
vec3 we_mix(vec4 a, vec3 b, vec3 t) { return mix(a.rgb, b, t); }
vec3 we_mix(vec3 a, vec4 b, vec3 t) { return mix(a, b.rgb, t); }
vec4 we_mix(vec4 a, float b, float t) { return mix(a, vec4(b), t); }
vec3 we_mix(vec3 a, float b, float t) { return mix(a, vec3(b), t); }
vec2 we_mix(vec2 a, float b, float t) { return mix(a, vec2(b), t); }
vec4 we_mix(float a, vec4 b, float t) { return mix(vec4(a), b, t); }
vec3 we_mix(float a, vec3 b, float t) { return mix(vec3(a), b, t); }
vec2 we_mix(float a, vec2 b, float t) { return mix(vec2(a), b, t); }
vec4 we_mix(float a, vec4 b, vec4 t) { return mix(vec4(a), b, t); }
vec3 we_mix(float a, vec3 b, vec3 t) { return mix(vec3(a), b, t); }
vec2 we_mix(float a, vec2 b, vec2 t) { return mix(vec2(a), b, t); }
vec4 we_mix(vec4 a, float b, vec4 t) { return mix(a, vec4(b), t); }
vec3 we_mix(vec3 a, float b, vec3 t) { return mix(a, vec3(b), t); }
vec2 we_mix(vec2 a, float b, vec2 t) { return mix(a, vec2(b), t); }
#define mix(a, b, t) we_mix(a, b, t)

float we_pow(float a, float b) { return pow(max(a, 0.0), b); }
vec2 we_pow(vec2 a, vec2 b) { return pow(max(a, vec2(0.0)), b); }
vec3 we_pow(vec3 a, vec3 b) { return pow(max(a, vec3(0.0)), b); }
vec4 we_pow(vec4 a, vec4 b) { return pow(max(a, vec4(0.0)), b); }
vec2 we_pow(vec2 a, float b) { return pow(max(a, vec2(0.0)), vec2(b)); }
vec3 we_pow(vec3 a, float b) { return pow(max(a, vec3(0.0)), vec3(b)); }
vec4 we_pow(vec4 a, float b) { return pow(max(a, vec4(0.0)), vec4(b)); }
#define pow(a, b) we_pow(a, b)

float we_max(float a, float b) { return max(a, b); }
vec2 we_max(vec2 a, vec2 b) { return max(a, b); }
vec3 we_max(vec3 a, vec3 b) { return max(a, b); }
vec4 we_max(vec4 a, vec4 b) { return max(a, b); }
vec2 we_max(vec2 a, float b) { return max(a, vec2(b)); }
vec3 we_max(vec3 a, float b) { return max(a, vec3(b)); }
vec4 we_max(vec4 a, float b) { return max(a, vec4(b)); }
vec2 we_max(float a, vec2 b) { return max(vec2(a), b); }
vec3 we_max(float a, vec3 b) { return max(vec3(a), b); }
vec4 we_max(float a, vec4 b) { return max(vec4(a), b); }
#define max(a, b) we_max(a, b)

// HLSL's smoothstep with equal edges behaves like a step (saturate(inf));
// GLSL leaves it undefined, which breaks audio `bounds` like "0.6 0.6".
float we_smoothstep(float a, float b, float x) { return a == b ? step(a, x) : smoothstep(a, b, x); }
vec2 we_smoothstep(float a, float b, vec2 x) { return a == b ? step(vec2(a), x) : smoothstep(a, b, x); }
vec3 we_smoothstep(float a, float b, vec3 x) { return a == b ? step(vec3(a), x) : smoothstep(a, b, x); }
vec4 we_smoothstep(float a, float b, vec4 x) { return a == b ? step(vec4(a), x) : smoothstep(a, b, x); }
vec2 we_smoothstep(vec2 a, vec2 b, vec2 x) { return mix(step(a, x), smoothstep(a, b, x), vec2(notEqual(a, b))); }
vec3 we_smoothstep(vec3 a, vec3 b, vec3 x) { return mix(step(a, x), smoothstep(a, b, x), vec3(notEqual(a, b))); }
vec4 we_smoothstep(vec4 a, vec4 b, vec4 x) { return mix(step(a, x), smoothstep(a, b, x), vec4(notEqual(a, b))); }
#define smoothstep(a, b, x) we_smoothstep(a, b, x)

vec4 mmul(vec4 v, mat4 m) { return m * v; }
vec4 mmul(mat4 m, vec4 v) { return m * v; }
vec3 mmul(vec3 v, mat3 m) { return m * v; }
vec3 mmul(mat3 m, vec3 v) { return m * v; }
vec2 mmul(vec2 v, mat2 m) { return m * v; }
vec2 mmul(mat2 m, vec2 v) { return m * v; }
mat4 mmul(mat4 a, mat4 b) { return a * b; }
mat3 mmul(mat3 a, mat3 b) { return a * b; }
mat2 mmul(mat2 a, mat2 b) { return a * b; }
#define mul(a, b) mmul(a, b)

vec2 modV2(vec2 x, vec2 y) { return mod(x, y); }
"#;

const COMMON_H: &str = r#"
vec3 hsv2rgb(vec3 c) {
    vec4 K = vec4(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    vec3 p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
    return c.z * mix(K.xxx, clamp(p - K.xxx, 0.0, 1.0), c.y);
}
vec3 rgb2hsv(vec3 RGB) {
    vec4 P = (RGB.g < RGB.b) ? vec4(RGB.bg, -1.0, 2.0 / 3.0) : vec4(RGB.gb, 0.0, -1.0 / 3.0);
    vec4 Q = (RGB.r < P.x) ? vec4(P.xyw, RGB.r) : vec4(RGB.r, P.yzx);
    float C = Q.x - min(Q.w, Q.y);
    float H = abs((Q.w - Q.y) / (6.0 * C + 1e-10) + Q.z);
    vec3 HCV = vec3(H, C, Q.x);
    float S = HCV.y / (HCV.z + 1e-10);
    return vec3(HCV.x, S, HCV.z);
}
vec2 rotateVec2(vec2 v, float r) {
    vec2 cs = vec2(cos(r), sin(r));
    return vec2(v.x * cs.x - v.y * cs.y, v.x * cs.y + v.y * cs.x);
}
float greyscale(vec3 color) { return dot(color, vec3(0.11, 0.59, 0.3)); }
"#;

const COMMON_BLENDING_H: &str = r#"
vec3 _blend(vec3 base, vec3 blend, int mode) {
    if (mode == 1) return min(base, blend);
    if (mode == 2) return base * blend;
    if (mode == 3) return max(1.0 - (1.0 - base) / max(blend, vec3(1e-5)), vec3(0.0));
    if (mode == 4) return max(base + blend - 1.0, 0.0);
    if (mode == 5) return min(base, blend);
    if (mode == 6) return max(base, blend);
    if (mode == 7) return 1.0 - (1.0 - base) * (1.0 - blend);
    if (mode == 8) return min(base / max(1.0 - blend, vec3(1e-5)), 1.0);
    if (mode == 9 || mode == 31) return base + blend;
    if (mode == 10) return max(base, blend);
    if (mode == 11) return mix(2.0 * base * blend, 1.0 - 2.0 * (1.0 - base) * (1.0 - blend), step(0.5, base));
    if (mode == 12) {
        vec3 sl = 2.0 * base * blend + base * base * (1.0 - 2.0 * blend);
        vec3 sh = sqrt(base) * (2.0 * blend - 1.0) + 2.0 * base * (1.0 - blend);
        return mix(sl, sh, step(0.5, blend));
    }
    if (mode == 13) return mix(2.0 * base * blend, 1.0 - 2.0 * (1.0 - base) * (1.0 - blend), step(0.5, blend));
    if (mode == 14) {
        vec3 lo = max(1.0 - (1.0 - base) / max(2.0 * blend, vec3(1e-5)), vec3(0.0));
        vec3 hi = min(base / max(1.0 - 2.0 * blend, vec3(1e-5)), 1.0);
        return mix(lo, hi, step(0.5, blend));
    }
    if (mode == 15) {
        vec3 lo = max(base + 2.0 * blend - 1.0, 0.0);
        vec3 hi = min(base + 2.0 * (blend - 0.5), 1.0);
        return mix(lo, hi, step(0.5, blend));
    }
    if (mode == 16) {
        vec3 lo = min(base, 2.0 * blend);
        vec3 hi = max(base, 2.0 * (blend - 0.5));
        return mix(lo, hi, step(0.5, blend));
    }
    if (mode == 17) {
        vec3 v = mix(max(1.0 - (1.0 - base) / max(2.0 * blend, vec3(1e-5)), vec3(0.0)), min(base / max(1.0 - 2.0 * blend, vec3(1e-5)), 1.0), step(0.5, blend));
        return step(vec3(0.5), v);
    }
    if (mode == 18) return abs(base - blend);
    if (mode == 19) return base + blend - 2.0 * base * blend;
    if (mode == 20) return max(base + blend - 1.0, 0.0);
    if (mode == 21) return min(base * base / max(1.0 - blend, vec3(1e-5)), 1.0);
    if (mode == 22) return min(blend * blend / max(1.0 - base, vec3(1e-5)), 1.0);
    if (mode == 23) return min(base, blend) - max(base, blend) + 1.0;
    if (mode == 24) return (base + blend) * 0.5;
    if (mode == 25) return 1.0 - abs(1.0 - base - blend);
    if (mode == 30) return vec3(max(base.r, max(base.g, base.b))) * blend;
    if (mode == 32) return base + base * blend;
    return blend;
}

vec3 ApplyBlending(const int blendMode, in vec3 A, in vec3 B, in float opacity) {
    vec3 res = _blend(A, B, blendMode);
    if (blendMode == 5) return min(A, B);
    if (blendMode == 10) return max(A, B);
    if (blendMode == 31) return A + B * opacity;
    if (blendMode == 32) return mix(A, A + A * B, opacity);
    return mix(A, res, opacity);
}
"#;

const COMMON_BLUR_H: &str = r#"
vec3 blur13(vec2 u, vec2 d) {
    vec2 o1 = vec2(1.4091998770852122) * d;
    vec2 o2 = vec2(3.2979348079914822) * d;
    vec2 o3 = vec2(5.2062900776825969) * d;
    return texSample2D(g_Texture0, u).rgb * 0.1976406528809576
        + texSample2D(g_Texture0, u + o1).rgb * 0.2959855056006557
        + texSample2D(g_Texture0, u - o1).rgb * 0.2959855056006557
        + texSample2D(g_Texture0, u + o2).rgb * 0.0935333619980593
        + texSample2D(g_Texture0, u - o2).rgb * 0.0935333619980593
        + texSample2D(g_Texture0, u + o3).rgb * 0.0116608059608062
        + texSample2D(g_Texture0, u - o3).rgb * 0.0116608059608062;
}
vec3 blur7(vec2 u, vec2 d) {
    vec2 o1 = vec2(2.3515644035337887) * d;
    vec2 o2 = vec2(0.469433779698372) * d;
    vec2 o3 = vec2(1.4091998770852121) * d;
    vec2 o4 = vec2(3.0) * d;
    return texSample2D(g_Texture0, u + o1).rgb * 0.2028175528299753
        + texSample2D(g_Texture0, u + o2).rgb * 0.4044856614512112
        + texSample2D(g_Texture0, u - o3).rgb * 0.3213933537319605
        + texSample2D(g_Texture0, u - o4).rgb * 0.0713034319868530;
}
vec3 blur3(vec2 u, vec2 d) {
    return texSample2D(g_Texture0, u + d).rgb * 0.25
        + texSample2D(g_Texture0, u).rgb * 0.5
        + texSample2D(g_Texture0, u - d).rgb * 0.25;
}
vec4 blur13a(vec2 u, vec2 d) {
    vec2 o1 = vec2(1.4091998770852122) * d;
    vec2 o2 = vec2(3.2979348079914822) * d;
    vec2 o3 = vec2(5.2062900776825969) * d;
    return texSample2D(g_Texture0, u) * 0.1976406528809576
        + texSample2D(g_Texture0, u + o1) * 0.2959855056006557
        + texSample2D(g_Texture0, u - o1) * 0.2959855056006557
        + texSample2D(g_Texture0, u + o2) * 0.0935333619980593
        + texSample2D(g_Texture0, u - o2) * 0.0935333619980593
        + texSample2D(g_Texture0, u + o3) * 0.0116608059608062
        + texSample2D(g_Texture0, u - o3) * 0.0116608059608062;
}
vec4 blur7a(vec2 u, vec2 d) {
    vec2 o1 = vec2(2.3515644035337887) * d;
    vec2 o2 = vec2(0.469433779698372) * d;
    vec2 o3 = vec2(1.4091998770852121) * d;
    vec2 o4 = vec2(3.0) * d;
    return texSample2D(g_Texture0, u + o1) * 0.2028175528299753
        + texSample2D(g_Texture0, u + o2) * 0.4044856614512112
        + texSample2D(g_Texture0, u - o3) * 0.3213933537319605
        + texSample2D(g_Texture0, u - o4) * 0.0713034319868530;
}
vec4 blur3a(vec2 u, vec2 d) {
    return texSample2D(g_Texture0, u + d) * 0.25
        + texSample2D(g_Texture0, u) * 0.5
        + texSample2D(g_Texture0, u - d) * 0.25;
}
vec2 blurRotateVec2(vec2 v, float r) {
    vec2 cs = vec2(cos(r), sin(r));
    return vec2(v.x * cs.x - v.y * cs.y, v.x * cs.y + v.y * cs.x);
}
vec4 blurRadial13a(vec2 u, vec2 center, float amt) {
    vec2 delta = u - center;
    amt *= 0.025;
    vec2 r1 = blurRotateVec2(delta, 1.4091998770852122 * amt) - delta;
    vec2 r2 = blurRotateVec2(delta, 3.2979348079914822 * amt) - delta;
    vec2 r3 = blurRotateVec2(delta, 5.2062900776825969 * amt) - delta;
    return texSample2D(g_Texture0, u) * 0.1976406528809576
        + texSample2D(g_Texture0, center + r1 + delta) * 0.2959855056006557
        + texSample2D(g_Texture0, center - r1 + delta) * 0.2959855056006557
        + texSample2D(g_Texture0, center + r2 + delta) * 0.0935333619980593
        + texSample2D(g_Texture0, center - r2 + delta) * 0.0935333619980593
        + texSample2D(g_Texture0, center + r3 + delta) * 0.0116608059608062
        + texSample2D(g_Texture0, center - r3 + delta) * 0.0116608059608062;
}
vec4 blurRadial7a(vec2 u, vec2 center, float amt) {
    vec2 delta = u - center;
    amt *= 0.025;
    vec2 r1 = blurRotateVec2(delta, 2.3515644035337887 * amt) - delta;
    vec2 r2 = blurRotateVec2(delta, 0.469433779698372 * amt) - delta;
    vec2 r3 = blurRotateVec2(delta, -1.4091998770852121 * amt) - delta;
    vec2 r4 = blurRotateVec2(delta, -3.0 * amt) - delta;
    return texSample2D(g_Texture0, center + r1 + delta) * 0.2028175528299753
        + texSample2D(g_Texture0, center + r2 + delta) * 0.4044856614512112
        + texSample2D(g_Texture0, center + r3 + delta) * 0.3213933537319605
        + texSample2D(g_Texture0, center + r4 + delta) * 0.0713034319868530;
}
vec4 blurRadial3a(vec2 u, vec2 center, float amt) {
    vec2 delta = u - center;
    float o1 = amt * 0.025;
    vec2 r1 = blurRotateVec2(delta, o1) - delta;
    return texSample2D(g_Texture0, center + delta) * 0.5
        + texSample2D(g_Texture0, center + r1 + delta) * 0.25
        + texSample2D(g_Texture0, center - r1 + delta) * 0.25;
}
"#;

const COMMON_COMPOSITE_H: &str = r#"
uniform float g_CompositeAlpha; // {"material":"compositealpha","default":1}
uniform vec2 g_CompositeOffset; // {"material":"compositeoffset","default":"0 0"}
uniform vec3 g_CompositeColor; // {"material":"compositecolor","default":"1 1 1"}

vec2 ApplyCompositeOffset(vec2 texCoords, vec2 textureResolution) {
#if COMPOSITE != 0
    return texCoords + g_CompositeOffset / textureResolution;
#else
    return texCoords;
#endif
}
vec4 ApplyComposite(vec4 original, vec4 effect) {
#if COMPOSITEMONO == 1
    effect.rgb = vec3(greyscale(effect.rgb));
#endif
    effect.rgb *= g_CompositeColor;
#if COMPOSITE == 0
    return effect;
#endif
#if COMPOSITE == 1
    effect.rgb = ApplyBlending(BLENDMODE, original.rgb, effect.rgb, effect.a * g_CompositeAlpha);
    effect.a = max(effect.a * saturate(g_CompositeAlpha), original.a);
#endif
#if COMPOSITE == 2
    effect.a *= saturate(g_CompositeAlpha);
    effect = mix(effect, original, original.a);
#endif
#if COMPOSITE == 3
    effect.a *= saturate(g_CompositeAlpha);
    effect.a *= 1.0 - original.a;
#endif
    return effect;
}
"#;

const COMMON_PERSPECTIVE_H: &str = r#"
mat3 squareToQuad(vec2 p0, vec2 p1, vec2 p2, vec2 p3) {
    mat3 m = mat3(1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0);
    float dx0 = p0.x, dy0 = p0.y;
    float dx1 = p1.x, dy1 = p1.y;
    float dx2 = p3.x, dy2 = p3.y;
    float dx3 = p2.x, dy3 = p2.y;
    float diffx1 = dx1 - dx3, diffy1 = dy1 - dy3;
    float diffx2 = dx2 - dx3, diffy2 = dy2 - dy3;
    float det = diffx1 * diffy2 - diffx2 * diffy1;
    float sumx = dx0 - dx1 + dx3 - dx2;
    float sumy = dy0 - dy1 + dy3 - dy2;
    if (det == 0.0 || (sumx == 0.0 && sumy == 0.0)) {
        m[0][0] = dx1 - dx0; m[0][1] = dy1 - dy0;
        m[1][0] = dx3 - dx1; m[1][1] = dy3 - dy1;
        m[2][0] = dx0; m[2][1] = dy0;
        return m;
    }
    float ovdet = 1.0 / det;
    float g = (sumx * diffy2 - diffx2 * sumy) * ovdet;
    float h = (diffx1 * sumy - sumx * diffy1) * ovdet;
    m[0][0] = dx1 - dx0 + g * dx1; m[0][1] = dy1 - dy0 + g * dy1; m[0][2] = g;
    m[1][0] = dx2 - dx0 + h * dx2; m[1][1] = dy2 - dy0 + h * dy2; m[1][2] = h;
    m[2][0] = dx0; m[2][1] = dy0; m[2][2] = 1.0;
    return m;
}
"#;

const COMMON_FRAGMENT_H: &str = r#"
vec3 DecompressNormal(vec4 normal) {
    normal.xy = normal.wy * 2.0 - 1.0;
    normal.z = sqrt(saturate(1.0 - normal.x * normal.x - normal.y * normal.y));
    return normal.xyz;
}
vec4 DecompressNormalWithMask(vec4 normal) {
    normal.xw = normal.wx;
    normal.xy = normal.xy * 2.0 - 1.0;
    normal.z = sqrt(saturate(1.0 - normal.x * normal.x - normal.y * normal.y));
    return normal;
}
float ComputeMaterialSpecularPower(const float roughness, const float metallic) { return (1.01 - roughness) * mix(400.0, 250.0, metallic); }
float ComputeMaterialSpecularStrength(const float roughness, const float metallic) { return (0.5 + metallic * 0.5) * (1.0 - roughness * 0.9); }
vec3 ComputeLight(const vec3 normal, const vec3 lightDelta, const vec3 color, const float radius) {
    float d = length(lightDelta);
    float attn = saturate((radius - d) / radius);
    return color * saturate(dot(lightDelta / max(d, 1e-5), normal)) * attn * attn;
}
vec3 ComputeLightSpecular(const vec3 normal, const vec3 lightDelta, const vec3 color, const float radius, const vec3 viewDir, const float specularPower, const float specularStrength, const float halfLambert, const float metallicTerm, inout vec3 specularResult) {
    float d = length(lightDelta);
    float attn = saturate((radius - d) / radius);
    vec3 lightDir = lightDelta / max(d, 1e-5);
    float specular = max(0.0, dot(normalize(viewDir + lightDir), normal));
    specularResult += pow(specular, specularPower) * specularStrength * attn * color;
    float lightDot = dot(lightDir, normal);
    float hl = lightDot * 0.5 + 0.5;
    lightDot = mix(lightDot, hl, halfLambert);
    float rim = metallicTerm * 2.0;
    rim = pow((1.0 - saturate(dot(normal, viewDir))) * pow(hl, 0.25), 6.0 - rim) * rim;
    return color * (saturate(lightDot) + rim) * attn * attn;
}
float ConvertSampleR8(vec4 _sample) { return _sample.r; }
vec4 ConvertTexture0Format(vec4 _sample) { return _sample; }
vec4 ConvertTextureFormat(const int format, vec4 _sample) { return _sample; }
"#;

const COMMON_VERTEX_H: &str = r#"
mat3 BuildTangentSpace(const vec3 normal, const vec4 signedTangent) {
    vec3 tangent = signedTangent.xyz;
    vec3 bitangent = cross(normal, tangent) * signedTangent.w;
    return mat3(tangent, bitangent, normal);
}
mat3 BuildTangentSpace(const mat3 modelTransform, const vec3 normal, const vec4 signedTangent) {
    vec3 tangent = signedTangent.xyz;
    vec3 bitangent = cross(normal, tangent) * signedTangent.w;
    return mat3(mmul(tangent, modelTransform), mmul(bitangent, modelTransform), mmul(normal, modelTransform));
}
"#;

fn header_for(name: &str) -> Option<&'static str> {
    match name {
        "common.h" => Some(COMMON_H),
        "common_blending.h" => Some(COMMON_BLENDING_H),
        "common_blur.h" => Some(COMMON_BLUR_H),
        "common_composite.h" => Some(COMMON_COMPOSITE_H),
        "common_perspective.h" => Some(COMMON_PERSPECTIVE_H),
        "common_fragment.h" => Some(COMMON_FRAGMENT_H),
        "common_vertex.h" => Some(COMMON_VERTEX_H),
        _ => None,
    }
}

fn load_header_source(name: &str) -> Option<String> {
    if let Some(s) = we_asset(&format!("shaders/{name}")) {
        return Some(s);
    }
    header_for(name).map(|s| s.to_string())
}

fn expand_includes_inner(
    src: &str,
    depth: usize,
    included: &mut std::collections::HashSet<String>,
) -> String {
    if depth > 8 {
        return src.to_string();
    }
    let mut out = String::with_capacity(src.len() + 2048);
    let mut deferred_blur = false;
    let mut g_tex0_seen = false;
    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("#require") {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("#include") {
            let name = rest
                .trim()
                .trim_start_matches('"')
                .trim_end_matches('"')
                .trim();
            if included.contains(name) {
                continue;
            }
            if name == "common_blur.h" && !g_tex0_seen {
                deferred_blur = true;
                continue;
            }
            if let Some(header) = load_header_source(name) {
                included.insert(name.to_string());
                out.push_str(&expand_includes_inner(&header, depth + 1, included));
                out.push('\n');
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
        if trimmed.starts_with("uniform sampler2D g_Texture0") {
            g_tex0_seen = true;
            if deferred_blur && !included.contains("common_blur.h") {
                if let Some(header) = load_header_source("common_blur.h") {
                    included.insert("common_blur.h".to_string());
                    out.push_str(&expand_includes_inner(&header, depth + 1, included));
                    out.push('\n');
                }
                deferred_blur = false;
            }
        }
    }
    if deferred_blur && !included.contains("common_blur.h") {
        if let Some(header) = load_header_source("common_blur.h") {
            included.insert("common_blur.h".to_string());
            out.push_str(&expand_includes_inner(&header, depth + 1, included));
            out.push('\n');
        }
    }
    out
}

fn expand_includes(src: &str, depth: usize) -> String {
    let mut included = std::collections::HashSet::new();
    expand_includes_inner(src, depth, &mut included)
}

fn preprocess(src: &str, vertex: bool, combos: &[(String, i32)]) -> String {
    let mut src = expand_includes(&src.replace("\r\n", "\n"), 0);
    src = src
        .lines()
        .filter(|l| !l.trim_start().starts_with("#version"))
        .collect::<Vec<_>>()
        .join("\n");

    if vertex {
        src = replace_word(&src, "attribute", "in");
        src = replace_word(&src, "varying", "out");
    } else {
        src = replace_word(&src, "varying", "in");
        src = replace_word(&src, "attribute", "in");
        src = src.replace("gl_FragColor", "fragColor");
    }
    src = src.replace("texture2D(", "texture(");
    // `sample` is a reserved word in GLSL ES 3.00 but a common local variable
    // name in Wallpaper Engine's shaders.
    src = replace_word(&src, "sample", "sample_");

    // Relax HLSL-style `const` on local runtime initializers and unsized arrays.
    src = src
        .replace("const vec2 ", "vec2 ")
        .replace("const vec3 ", "vec3 ")
        .replace("const vec4 ", "vec4 ")
        .replace("const float ", "float ")
        .replace("const int ", "int ")
        .replace("const mat3 ", "mat3 ")
        .replace("const mat4 ", "mat4 ")
        .replace("float weight[5] = float[](", "float weight[5] = float[5](")
        .replace("v_PointerUV * (", "v_PointerUV.xy * (")
        .replace("* v_PixelSize;", "* v_PixelSize.xy;");

    // HLSL numeric coercions: promote integer literals and wrap integer
    // variables used in float arithmetic (GLSL ES forbids both implicitly).
    let float_names = float_identifiers(&src);
    src = src
        .lines()
        .map(|line| floatify_int_literals(line, &float_names))
        .collect::<Vec<_>>()
        .join("\n");
    let combo_names: Vec<String> = combos.iter().map(|(n, _)| n.clone()).collect();
    let mut int_like = combo_names.clone();
    for name in int_define_names(&src) {
        if !int_like.iter().any(|n| n == &name) {
            int_like.push(name);
        }
    }
    src = wrap_int_vars(&src, &int_like);

    // Repair common HLSL semantics that GLSL ES rejects: implicit vector
    // truncation/promotion, integer booleans, and non-constant global
    // initializers.
    src = rename_builtin_redefinitions(&src);
    src = coerce_bool_arithmetic(&src);
    let types = identifier_widths(&src);
    let functions = function_widths(&src);
    let params = function_params(&src, &types);
    src = coerce_mixed_arithmetic(&src, &types, &functions);
    src = coerce_assignments(&src, &types, &functions);
    src = coerce_call_args(&src, &types, &functions, &params);
    src = coerce_for_headers(&src, &types);
    src = coerce_float_modulo(&src, &types, &functions);
    src = coerce_array_indices(&src, &types);
    src = coerce_return_values(&src, &types, &functions);
    src = hoist_global_initializers(&src);
    let mut int_names = int_identifiers(&src);
    for name in &combo_names {
        if !int_names.iter().any(|n| n == name) {
            int_names.push(name.clone());
        }
    }
    src = coerce_bool_conditions(&src, &int_names);

    // Balance stray `#endif` directives in custom Workshop shaders.
    let mut if_depth = 0i32;
    let mut balanced_lines = Vec::new();
    for line in src.lines() {
        let t = line.trim_start();
        if t.starts_with("#if") {
            if_depth += 1;
            balanced_lines.push(line);
        } else if t.starts_with("#endif") {
            if if_depth > 0 {
                if_depth -= 1;
                balanced_lines.push(line);
            }
        } else {
            balanced_lines.push(line);
        }
    }
    src = balanced_lines.join("\n");
    for _ in 0..if_depth {
        src.push_str("\n#endif\n");
    }

    let mut out = String::with_capacity(src.len() + PRELUDE.len() + 512);
    out.push_str("#version 300 es\n");
    out.push_str("#extension GL_EXT_shader_implicit_conversions : enable\n");
    out.push_str("precision highp float;\nprecision highp int;\nprecision highp sampler2D;\n");
    let mut combos = combos.to_vec();
    combos.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, value) in &combos {
        out.push_str(&format!("#define {name} {value}\n"));
    }
    if !vertex && !src.contains("out vec4 fragColor") && !src.contains("out highp vec4 fragColor")
    {
        out.push_str("out vec4 fragColor;\n");
    }
    // Avoid duplicate `#define M_PI` if the shader/header already defines it.
    let prelude = if src.contains("#define M_PI") {
        PRELUDE.replace("#define M_PI 3.14159265359\n", "")
    } else {
        PRELUDE.to_string()
    };
    // A shader with its own `modV2` must not inherit our helper.
    let prelude = if src.contains("modV2") {
        prelude.replace("vec2 modV2(vec2 x, vec2 y) { return mod(x, y); }\n", "")
    } else {
        prelude
    };
    out.push_str(&prelude);
    // Wallpaper Engine relies on undefined #if identifiers evaluating to 0
    // (the OpenGL ES preprocessor instead rejects them).
    let mut defined: Vec<String> = combos.iter().map(|(n, _)| n.clone()).collect();
    for line in prelude.lines().chain(src.lines()) {
        if let Some(rest) = line.trim_start().strip_prefix("#define ") {
            if let Some(name) = rest.split(|c: char| c.is_whitespace() || c == '(').next() {
                if !name.is_empty() {
                    defined.push(name.to_string());
                }
            }
        }
    }
    for name in condition_macros(&src) {
        if !defined.iter().any(|n| n == &name) {
            out.push_str(&format!("#define {name} 0\n"));
            defined.push(name);
        }
    }
    out.push_str(&src);
    if std::env::var_os("ANIMA_DUMP_SHADERS").is_some() {
        let dir = std::path::Path::new("/tmp/anima-shaders");
        let _ = std::fs::create_dir_all(dir);
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for b in out.bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let kind = if vertex { "vert" } else { "frag" };
        let path = dir.join(format!("{kind}-{hash:016x}.glsl"));
        if std::fs::write(&path, &out).is_err() {
            log::warn!("could not write {}", path.display());
        }
    }
    out
}

/// Identifiers referenced by `#if` / `#elif` expressions.
fn condition_macros(src: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in src.lines() {
        let line_no_comment = line.split("//").next().unwrap_or(line);
        let trimmed = line_no_comment.trim_start();
        let expr = if let Some(rest) = trimmed.strip_prefix("#if ") {
            Some(rest)
        } else if let Some(rest) = trimmed.strip_prefix("#elif ") {
            Some(rest)
        } else {
            None
        };
        let Some(expr) = expr else { continue };
        let bytes = expr.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if is_ident_start(bytes[i]) {
                let start = i;
                while i < bytes.len() && is_ident_char(bytes[i]) {
                    i += 1;
                }
                let word = &expr[start..i];
                if word != "defined" && !out.iter().any(|w| w == word) {
                    out.push(word.to_string());
                }
            } else {
                i += 1;
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// HLSL -> GLSL numeric coercions
// ---------------------------------------------------------------------------

fn is_ident_start(c: u8) -> bool {
    // ASCII only: shader sources can contain UTF-8 comments, and treating
    // continuation bytes as letters would cut a slice inside a character.
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn is_qualifier(word: &str) -> bool {
    matches!(
        word,
        "in" | "out" | "inout" | "const" | "uniform" | "attribute" | "varying" | "highp"
            | "mediump" | "lowp" | "flat" | "smooth" | "noperspective" | "invariant"
    )
}

/// Strip a trailing `//` comment (keeping strings intact is unnecessary here).
fn strip_line_comment(line: &str) -> &str {
    line.split("//").next().unwrap_or(line)
}

/// Whether `line` declares a variable of type `ty` (word boundary aware, so
/// `preTransformPoint` does not look like an `int` declaration).
fn declares_type(line: &str, ty: &str) -> bool {
    let bytes = line.as_bytes();
    let needle = ty.as_bytes();
    let mut i = 0;
    while i + needle.len() < bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            let before_ok = i == 0 || !is_ident_char(bytes[i - 1]);
            let after = bytes[i + needle.len()];
            if before_ok && after.is_ascii_whitespace() {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// HLSL accepts `2` where GLSL ES requires `2.0`.  When a source line clearly
/// mixes literals with floating-point types, promote its integer literals.
fn floatify_int_literals(line: &str, float_names: &[String]) -> String {
    let code = strip_line_comment(line);
    let trimmed = code.trim_start();
    if trimmed.starts_with('#') {
        return line.to_string();
    }
    let lower = code.to_ascii_lowercase();
    // Index/bit contexts must keep integers, and declarations of integer
    // variables have their own semantics.
    for bad in [
        "[", "%", "<<", ">>", "&", "|", "^", "for (", "for(", "while", "switch",
        "case ", "bool ", "texelFetch", "textureSize", "bitfield", " gl_", "discard",
    ] {
        if lower.contains(bad) {
            return line.to_string();
        }
    }
    if declares_type(&lower, "int") || declares_type(&lower, "uint") {
        return line.to_string();
    }
    let has_float_ctx = lower.contains("float ")
        || lower.contains("vec2 ")
        || lower.contains("vec3 ")
        || lower.contains("vec4 ")
        || lower.contains("mat2 ")
        || lower.contains("mat3 ")
        || lower.contains("mat4 ")
        || lower.contains("double ")
        // Float constructors: `vec3(1)`, `float(2)`.
        || ["vec2(", "vec3(", "vec4(", "float(", "mat2(", "mat3(", "mat4("]
            .iter()
            .any(|c| code.contains(c))
        // Assignment/comparison involving a variable declared as float-like,
        // e.g. `v_TexCoord.w = 0;` in an otherwise integer-free line.
        || float_names.iter().any(|name| contains_word(&code, name))
        || {
            let b = code.as_bytes();
            b.windows(2).any(|w| w[0] == b'.' && w[1].is_ascii_digit())
                || b.windows(2).any(|w| w[0].is_ascii_digit() && w[1] == b'.')
        };
    if !has_float_ctx {
        return line.to_string();
    }
    let bytes = code.as_bytes();
    let mut out = String::with_capacity(line.len() + 8);
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_digit()
            && (i == 0 || (!is_ident_char(bytes[i - 1]) && bytes[i - 1] != b'.'))
        {
            let start = i;
            // Hex literals keep their form.
            if bytes[i] == b'0'
                && i + 1 < bytes.len()
                && (bytes[i + 1] | 32) == b'x'
            {
                i += 2;
                while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                    i += 1;
                }
                while i < bytes.len() && is_ident_char(bytes[i]) {
                    i += 1;
                }
                out.push_str(&code[start..i]);
                continue;
            }
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let mut is_float = false;
            if i < bytes.len() && bytes[i] == b'.' {
                is_float = true;
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if i < bytes.len() && (bytes[i] | 32) == b'e' {
                is_float = true;
                i += 1;
                if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                    i += 1;
                }
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let suffix_start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            out.push_str(&code[start..i]);
            if !is_float && i == suffix_start {
                out.push_str(".0");
            }
            continue;
        }
        let ch = code[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    // Preserve the original comment suffix, if any.
    if let Some(pos) = line.find("//") {
        out.push_str(&line[pos..]);
    }
    out
}

/// Collect identifiers declared as `int` / `uint` in the shader.
fn int_identifiers(src: &str) -> Vec<String> {
    let mut names = Vec::new();
    for raw in src.lines() {
        let line = strip_line_comment(raw);
        let mut from = 0usize;
        while let Some(pos) = line[from..].find("int ") {
            let abs = from + pos;
            let before_ok = abs == 0 || !is_ident_char(line.as_bytes()[abs - 1]);
            let mut i = abs + 4;
            let bytes = line.as_bytes();
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            if before_ok && i > start {
                let name = &line[start..i];
                if !names.iter().any(|n| n == name) {
                    names.push(name.to_string());
                }
            }
            from = abs + 4;
        }
    }
    names
}

/// Whether `word` occurs in `text` as a standalone identifier.
fn contains_word(text: &str, word: &str) -> bool {
    let bytes = text.as_bytes();
    let needle = word.as_bytes();
    if needle.is_empty() || needle.len() > bytes.len() {
        return false;
    }
    for i in 0..=bytes.len() - needle.len() {
        if &bytes[i..i + needle.len()] != needle {
            continue;
        }
        let before_ok = i == 0 || !is_ident_char(bytes[i - 1]);
        let after_ok = i + needle.len() == bytes.len() || !is_ident_char(bytes[i + needle.len()]);
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

/// Collect identifiers declared with a floating-point type (`float`, `vec*`,
/// `mat*`, `double`).  Used to recognise assignments such as
/// `v_TexCoord.w = 0;` as float contexts even though the line itself has no
/// type keyword.
fn float_identifiers(src: &str) -> Vec<String> {
    const TYPES: [&str; 9] = [
        "float", "double", "vec2", "vec3", "vec4", "mat2", "mat3", "mat4", "mat3x3",
    ];
    let mut names: Vec<String> = Vec::new();
    for raw in src.lines() {
        let line = strip_line_comment(raw);
        let bytes = line.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if !is_ident_start(bytes[i]) || (i > 0 && is_ident_char(bytes[i - 1])) {
                i += 1;
                continue;
            }
            let start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            let word = &line[start..i];
            if !TYPES.iter().any(|t| word.eq_ignore_ascii_case(t)) {
                continue;
            }
            let mut j = i;
            loop {
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j >= bytes.len() || !is_ident_start(bytes[j]) {
                    break;
                }
                let ns = j;
                while j < bytes.len() && is_ident_char(bytes[j]) {
                    j += 1;
                }
                let name = &line[ns..j];
                if !is_qualifier(name)
                    && !TYPES.iter().any(|t| name.eq_ignore_ascii_case(t))
                    && !names.iter().any(|n| n == name)
                {
                    names.push(name.to_string());
                }
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'[' {
                    while j < bytes.len() && bytes[j] != b']' {
                        j += 1;
                    }
                    j = (j + 1).min(bytes.len());
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                }
                if j < bytes.len() && bytes[j] == b',' {
                    j += 1;
                    continue;
                }
                break;
            }
        }
    }
    names
}

/// Wrap integer variables in `float(...)` when they appear next to an
/// arithmetic operator in a floating-point expression.  HLSL allows such
/// mixed arithmetic; GLSL ES does not.
fn wrap_int_vars(src: &str, extra_ints: &[String]) -> String {
    let mut ints = int_identifiers(src);
    for name in extra_ints {
        if !ints.iter().any(|n| n == name) {
            ints.push(name.clone());
        }
    }
    if ints.is_empty() {
        return src.to_string();
    }
    let mut out = String::with_capacity(src.len() + 64);
    for raw in src.lines() {
        let code = strip_line_comment(raw);
        let trimmed = code.trim_start();
        let lower = code.to_ascii_lowercase();
        let skip = trimmed.starts_with('#')
            || lower.contains("for (")
            || lower.contains("for(")
            || lower.contains("while")
            || lower.contains("switch")
            || declares_type(&lower, "int")
            || declares_type(&lower, "uint")
            || lower.contains("[");
        if skip {
            out.push_str(raw);
            out.push('\n');
            continue;
        }
        let bytes = raw.as_bytes();
        let mut line_out = String::with_capacity(raw.len() + 16);
        let mut i = 0;
        while i < bytes.len() {
            if is_ident_start(bytes[i]) && (i == 0 || !is_ident_char(bytes[i - 1])) {
                let start = i;
                while i < bytes.len() && is_ident_char(bytes[i]) {
                    i += 1;
                }
                let word = &raw[start..i];
                let prev_char = if start == 0 {
                    0
                } else {
                    raw.as_bytes()[start - 1]
                };
                // `.` field selections (`vec.x`) are not variables.
                if !ints.iter().any(|n| n == word) || prev_char == b'.' {
                    line_out.push_str(word);
                    continue;
                }
                let mut p = i;
                while p < bytes.len() && bytes[p].is_ascii_whitespace() {
                    p += 1;
                }
                let next = bytes.get(p).copied().unwrap_or(0);
                let prev_pos = raw[..start].trim_end().len();
                let prev = raw[..start]
                    .trim_end()
                    .as_bytes()
                    .last()
                    .copied()
                    .unwrap_or(0);
                let arith = (b"*/+-".contains(&next) || b"*/+-".contains(&prev))
                    && !(bytes.get(p + 1) == Some(&b'='))
                    && !(next == b'+' && bytes.get(p + 1) == Some(&b'+'))
                    && !(next == b'-' && bytes.get(p + 1) == Some(&b'-'))
                    && !(prev == b'+' && prev_pos >= 2 && bytes[prev_pos - 2] == b'+')
                    && !(prev == b'-' && prev_pos >= 2 && bytes[prev_pos - 2] == b'-');
                if arith && next != b'=' {
                    line_out.push_str("float(");
                    line_out.push_str(word);
                    line_out.push(')');
                } else {
                    line_out.push_str(word);
                }
                continue;
            }
            let ch = raw[i..].chars().next().unwrap();
            line_out.push(ch);
            i += ch.len_utf8();
        }
        out.push_str(&line_out);
        out.push('\n');
    }
    out
}

/// Column of the closing `)` matching the `(` at `open`, if balanced.
fn matching_paren(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.get(open) != Some(&b'(') {
        return None;
    }
    let mut depth = 0i32;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Length of a `.xyzw` / `.rgba` / `.stpq` swizzle starting at `i`.
fn parse_swizzle_len(text: &str, i: usize) -> (u8, usize) {
    let bytes = text.as_bytes();
    if bytes.get(i) != Some(&b'.') {
        return (0, i);
    }
    let mut j = i + 1;
    let mut n = 0u8;
    while j < bytes.len() && matches!(bytes[j], b'x' | b'y' | b'z' | b'w' | b'r' | b'g' | b'b' | b'a' | b's' | b't' | b'p' | b'q')
    {
        n += 1;
        j += 1;
    }
    if n == 0 {
        (0, i)
    } else {
        (n, j)
    }
}

/// Best-effort width (1..=4) of an expression: how many vector components it
/// yields.  Used to emulate HLSL's implicit truncation/broadcast.
fn infer_width(expr: &str, types: &HashMap<String, u8>, funcs: &HashMap<String, u8>) -> u8 {
    let bytes = expr.as_bytes();
    let mut i = 0usize;
    let mut max_w = 0u8;
    let mut atom = 0u8;
    let mut pending = false;
    while i < bytes.len() {
        let c = bytes[i];
        if is_ident_start(c) && (i == 0 || !is_ident_char(bytes[i - 1])) {
            let start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            let word = &expr[start..i];
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'(' {
                let end = matching_paren(expr, j).unwrap_or(bytes.len());
                let args = &expr[j + 1..end];
                let w = match word {
                    "texSample2D" | "texture" | "textureLod" | "textureGrad" => 4,
                    "float" | "int" | "uint" | "bool" => 1,
                    "vec2" => 2,
                    "vec3" => 3,
                    "vec4" => 4,
                    "CAST2" => 2,
                    "CAST3" => 3,
                    "CAST4" => 4,
                    "CASTF" | "CASTI" | "CASTU" => 1,
                    _ => {
                        if let Some(&w) = funcs.get(word) {
                            w
                        } else if matches!(
                            word,
                            "dot" | "length" | "distance" | "determinant" | "any" | "all"
                        ) {
                            1
                        } else {
                            let mut w = 0u8;
                            for arg in split_top_level(args, ',') {
                                w = w.max(infer_width(arg, types, funcs));
                            }
                            w
                        }
                    }
                };
                atom = w;
                i = end + 1;
                let (sw, ni) = parse_swizzle_len(expr, i);
                if sw > 0 {
                    atom = sw;
                    i = ni;
                }
                pending = true;
                continue;
            }
            let mut w = *types.get(word).unwrap_or(&0);
            let (sw, ni) = parse_swizzle_len(expr, i);
            if sw > 0 {
                w = sw;
                i = ni;
            }
            atom = w;
            pending = true;
            continue;
        }
        if c == b'.' {
            let (sw, ni) = parse_swizzle_len(expr, i);
            if sw > 0 {
                atom = sw;
                i = ni;
                pending = true;
                continue;
            }
        }
        if matches!(
            c,
            b'+' | b'-' | b'*' | b'/' | b'%' | b'?' | b':' | b',' | b'<' | b'>' | b'=' | b'&' | b'|'
        ) && pending
        {
            max_w = max_w.max(atom);
            atom = 0;
            pending = false;
        }
        i += 1;
    }
    if pending {
        max_w = max_w.max(atom);
    }
    max_w
}

/// Split `text` on `sep` at paren/bracket depth zero.
fn split_top_level(text: &str, sep: char) -> Vec<&str> {
    let bytes = text.as_bytes();
    let sep = sep as u8;
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
        if b == sep && depth == 0 {
            out.push(&text[start..i]);
            start = i + 1;
        }
    }
    out.push(&text[start..]);
    out
}

/// Index of the first top-level `=` that is a plain assignment (not `==`,
/// `<=`, `>=`, `!=`, `+=`, ...).
fn find_assignment(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    for i in 0..bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'=' if depth == 0 => {
                let next = bytes.get(i + 1).copied().unwrap_or(0);
                let prev = if i == 0 { 0 } else { bytes[i - 1] };
                if next == b'=' || matches!(prev, b'=' | b'<' | b'>' | b'!' | b'+' | b'-' | b'*' | b'/' | b'%')
                {
                    continue;
                }
                return Some(i);
            }
            _ => {}
        }
    }
    None
}

/// Map declared variable/parameter names to vector width.
fn identifier_widths(src: &str) -> HashMap<String, u8> {
    let mut map: HashMap<String, u8> = HashMap::new();
    for raw in src.lines() {
        let line = strip_line_comment(raw);
        let bytes = line.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if !is_ident_start(bytes[i]) || (i > 0 && is_ident_char(bytes[i - 1])) {
                i += 1;
                continue;
            }
            let start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            let width = match &line[start..i] {
                "float" => 1,
                "vec2" => 2,
                "vec3" => 3,
                "vec4" => 4,
                _ => 0,
            };
            if width == 0 {
                continue;
            }
            let mut j = i;
            loop {
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j >= bytes.len() || !is_ident_start(bytes[j]) {
                    break;
                }
                let ns = j;
                while j < bytes.len() && is_ident_char(bytes[j]) {
                    j += 1;
                }
                let name = &line[ns..j];
                if !is_qualifier(name) && !matches!(name, "float" | "vec2" | "vec3" | "vec4") {
                    use std::collections::hash_map::Entry;
                    match map.entry(name.to_string()) {
                        Entry::Vacant(e) => {
                            e.insert(width);
                        }
                        Entry::Occupied(mut e) => {
                            // Same name declared with a different type (often in
                            // mutually exclusive `#if` branches): its type is
                            // ambiguous, so never use it for coercion.
                            if *e.get() != width {
                                e.insert(0);
                            }
                        }
                    }
                }
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'[' {
                    while j < bytes.len() && bytes[j] != b']' {
                        j += 1;
                    }
                    j = (j + 1).min(bytes.len());
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                }
                if j < bytes.len() && bytes[j] == b',' {
                    j += 1;
                    continue;
                }
                break;
            }
        }
    }
    map
}

/// Map user function names to their return width.
fn function_widths(src: &str) -> HashMap<String, u8> {
    let mut map: HashMap<String, u8> = HashMap::new();
    for raw in src.lines() {
        let line = strip_line_comment(raw);
        let bytes = line.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if !is_ident_start(bytes[i]) || (i > 0 && is_ident_char(bytes[i - 1])) {
                i += 1;
                continue;
            }
            let start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            let width = match &line[start..i] {
                "float" => 1,
                "vec2" => 2,
                "vec3" => 3,
                "vec4" => 4,
                _ => 0,
            };
            if width == 0 {
                continue;
            }
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let ns = j;
            if j >= bytes.len() || !is_ident_start(bytes[j]) {
                continue;
            }
            while j < bytes.len() && is_ident_char(bytes[j]) {
                j += 1;
            }
            let name = &line[ns..j];
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'(' {
                map.entry(name.to_string()).or_insert(width);
            }
        }
    }
    map
}

/// Map user function names to the widths of their parameters.
fn function_params(src: &str, types: &HashMap<String, u8>) -> HashMap<String, Vec<u8>> {
    let mut map: HashMap<String, Vec<u8>> = HashMap::new();
    let text = src;
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if !is_ident_start(bytes[i]) || (i > 0 && is_ident_char(bytes[i - 1])) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && is_ident_char(bytes[i]) {
            i += 1;
        }
        let word = &text[start..i];
        let ret_width = match word {
            "float" => 1,
            "vec2" => 2,
            "vec3" => 3,
            "vec4" => 4,
            _ => 0,
        };
        if ret_width == 0 {
            continue;
        }
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let ns = j;
        if j >= bytes.len() || !is_ident_start(bytes[j]) {
            continue;
        }
        while j < bytes.len() && is_ident_char(bytes[j]) {
            j += 1;
        }
        let name = &text[ns..j];
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if bytes.get(j) != Some(&b'(') {
            continue;
        }
        let Some(end) = matching_paren(text, j) else {
            continue;
        };
        let params_src = &text[j + 1..end];
        let mut widths = Vec::new();
        for param in split_top_level(params_src, ',') {
            let mut w = 0u8;
            for token in param.split_whitespace() {
                if is_qualifier(token) {
                    continue;
                }
                w = w.max(*types.get(token).unwrap_or(&0));
            }
            if w == 0 {
                for token in param.split_whitespace() {
                    if is_qualifier(token) {
                        continue;
                    }
                    w = w.max(match token {
                        "float" => 1,
                        "vec2" => 2,
                        "vec3" => 3,
                        "vec4" => 4,
                        _ => 0,
                    });
                }
            }
            widths.push(w);
        }
        map.entry(name.to_string()).or_insert(widths);
        i = end + 1;
    }
    map
}

/// Rename user definitions that collide with GLSL ES built-ins (notably
/// `float mod(float, float)`, which is illegal in GLSL ES 3.00).
fn rename_builtin_redefinitions(src: &str) -> String {
    const BUILTINS: [&str; 12] = [
        "mod", "mix", "pow", "max", "min", "clamp", "saturate", "step", "smoothstep", "fract",
        "lerp", "frac",
    ];
    let mut out = String::with_capacity(src.len());
    for raw in src.lines() {
        let trimmed = raw.trim_start();
        let indent = &raw[..raw.len() - trimmed.len()];
        let mut rebuilt = None;
        for ty in ["void", "float", "vec2", "vec3", "vec4"] {
            if let Some(rest) = trimmed.strip_prefix(ty) {
                if !rest.starts_with(|c: char| c.is_whitespace()) {
                    continue;
                }
                let rest = rest.trim_start();
                let name_end = rest
                    .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .unwrap_or(rest.len());
                let name = &rest[..name_end];
                let after = rest[name_end..].trim_start();
                if BUILTINS.contains(&name) && after.starts_with('(') {
                    rebuilt = Some(format!(
                        "{indent}{ty} we_user_{name}{}",
                        &rest[name_end..]
                    ));
                }
                break;
            }
        }
        if let Some(line) = rebuilt {
            out.push_str(&line);
        } else {
            out.push_str(raw);
        }
        out.push('\n');
    }
    out
}

/// True when the expression consists only of numeric literals and operators
/// (no identifiers), i.e. it evaluates to a scalar.
fn is_literal_expression(expr: &str) -> bool {
    !expr.is_empty()
        && expr.chars().all(|c| {
            c.is_ascii_digit()
                || matches!(
                    c,
                    '.' | '+' | '-' | '*' | '/' | '%' | '(' | ')' | ' ' | '\t' | 'e' | 'E' | 'f'
                        | 'F'
                )
        })
}

fn coerce_expr(
    expr: &str,
    lhs_width: u8,
    types: &HashMap<String, u8>,
    funcs: &HashMap<String, u8>,
) -> String {
    let trimmed = expr.trim();
    if trimmed.is_empty() || lhs_width == 0 {
        return expr.to_string();
    }
    if lhs_width == 1 && trimmed.starts_with("int(") {
        return format!("float({trimmed})");
    }
    let mut rhs_width = infer_width(trimmed, types, funcs);
    if rhs_width == 0 && is_literal_expression(trimmed) {
        rhs_width = 1;
    }
    if rhs_width > lhs_width && lhs_width < 4 {
        let swizzle = match lhs_width {
            1 => "x",
            2 => "xy",
            _ => "rgb",
        };
        return format!("({trimmed}).{swizzle}");
    }
    if rhs_width == 1 && (2..=4).contains(&lhs_width) {
        return format!("vec{lhs_width}({trimmed})");
    }
    expr.to_string()
}

/// Insert swizzles/constructors so assignments match GLSL ES types where HLSL
/// would implicitly truncate or broadcast.
fn coerce_assignments(
    src: &str,
    types: &HashMap<String, u8>,
    funcs: &HashMap<String, u8>,
) -> String {
    let mut out = String::with_capacity(src.len() + 128);
    for raw in src.lines() {
        out.push_str(&coerce_assignment_line(raw, types, funcs));
        out.push('\n');
    }
    out
}

fn coerce_assignment_line(
    raw: &str,
    types: &HashMap<String, u8>,
    funcs: &HashMap<String, u8>,
) -> String {
    let code = strip_line_comment(raw);
    let trimmed = code.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return raw.to_string();
    }
    let comment = raw.find("//").map(|p| &raw[p..]).unwrap_or("");
    let raw_trimmed = raw.trim_start();
    let indent = &raw[..raw.len() - raw_trimmed.len()];

    let first_word = trimmed
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .map(|p| &trimmed[..p])
        .unwrap_or(trimmed);
    let decl_width = match first_word {
        "float" => 1,
        "vec2" => 2,
        "vec3" => 3,
        "vec4" => 4,
        _ => 0,
    };

    if decl_width > 0 {
        let rest = trimmed[first_word.len()..].trim_start();
        let semi = match rest.find(';') {
            Some(p) => p,
            None => return raw.to_string(),
        };
        let body = &rest[..semi];
        // Function definition/prototype: `vec3 foo(...)`.
        if let Some(paren) = body.find('(') {
            if body.find('=').map_or(true, |eq| paren < eq) {
                return raw.to_string();
            }
        }
        let mut pieces = Vec::new();
        for declarator in split_top_level(body, ',') {
            if declarator.contains('[') || declarator.trim().is_empty() {
                pieces.push(declarator.to_string());
                continue;
            }
            if let Some(eq) = find_assignment(declarator) {
                let lhs = &declarator[..eq];
                let rhs = declarator[eq + 1..].trim();
                let coerced = coerce_expr(rhs, decl_width, types, funcs);
                pieces.push(format!("{lhs}= {coerced}"));
            } else {
                pieces.push(declarator.to_string());
            }
        }
        return format!("{indent}{first_word} {};{comment}", pieces.join(", "));
    }

    // Plain assignment: `<ident> = <expr>;`
    let Some(eq) = find_assignment(trimmed) else {
        return raw.to_string();
    };
    let lhs = trimmed[..eq].trim();
    if !lhs.bytes().all(is_ident_char) {
        return raw.to_string();
    }
    let Some(&lhs_width) = types.get(lhs) else {
        return raw.to_string();
    };
    let Some(semi) = trimmed[eq + 1..].find(';') else {
        return raw.to_string();
    };
    let rhs = trimmed[eq + 1..eq + 1 + semi].trim();
    let coerced = coerce_expr(rhs, lhs_width, types, funcs);
    format!("{indent}{lhs} = {coerced};{comment}")
}

/// Coerce arguments of calls to user-defined functions (HLSL lets a `vec4`
/// argument bind to a `vec3` parameter by truncation).
fn coerce_call_args(
    src: &str,
    types: &HashMap<String, u8>,
    funcs: &HashMap<String, u8>,
    params: &HashMap<String, Vec<u8>>,
) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len() + 64);
    let mut i = 0usize;
    while i < bytes.len() {
        if is_ident_start(bytes[i]) && (i == 0 || !is_ident_char(bytes[i - 1])) {
            let start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            let word = &src[start..i];
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if bytes.get(j) == Some(&b'(') && params.contains_key(word) {
                // Skip the definition itself: the preceding token is a type.
                let mut k = start;
                while k > 0 && src.as_bytes()[k - 1].is_ascii_whitespace() {
                    k -= 1;
                }
                let mut w = k;
                while w > 0 && is_ident_char(src.as_bytes()[w - 1]) {
                    w -= 1;
                }
                let prev = &src[w..k];
                if matches!(prev, "float" | "vec2" | "vec3" | "vec4" | "void" | "int" | "uint") {
                    out.push_str(word);
                    continue;
                }
                let Some(end) = matching_paren(src, j) else {
                    out.push_str(word);
                    continue;
                };
                let args = &src[j + 1..end];
                let expected = params.get(word).cloned().unwrap_or_default();
                let mut coerced = Vec::new();
                for (idx, arg) in split_top_level(args, ',').into_iter().enumerate() {
                    let expected_w = expected.get(idx).copied().unwrap_or(0);
                    if expected_w == 0 || arg.trim().is_empty() {
                        coerced.push(arg.to_string());
                        continue;
                    }
                    let nested = coerce_call_args(arg, types, funcs, params);
                    let c = coerce_expr(&nested, expected_w, types, funcs);
                    coerced.push(c);
                }
                out.push_str(word);
                out.push('(');
                out.push_str(&coerced.join(", "));
                out.push(')');
                i = end + 1;
                continue;
            }
            out.push_str(word);
            continue;
        }
        let ch = src[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Column of the `(` matching the `)` at `close`, scanning backwards.
fn matching_paren_back(text: &str, close: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.get(close) != Some(&b')') {
        return None;
    }
    let mut depth = 0i32;
    let mut i = close as isize;
    while i >= 0 {
        match bytes[i as usize] {
            b')' => depth += 1,
            b'(' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i as usize);
                }
            }
            _ => {}
        }
        i -= 1;
    }
    None
}

fn swizzle_for_width(w: u8) -> &'static str {
    match w {
        1 => "x",
        2 => "xy",
        3 => "xyz",
        _ => "xyzw",
    }
}

/// Locate the operand immediately before the operator at `op`.
fn left_atom(
    text: &str,
    op: usize,
    types: &HashMap<String, u8>,
    funcs: &HashMap<String, u8>,
) -> Option<(usize, usize, u8)> {
    let bytes = text.as_bytes();
    if op == 0 {
        return None;
    }
    let mut j = op - 1;
    if bytes[j].is_ascii_whitespace() {
        while j > 0 && bytes[j].is_ascii_whitespace() {
            j -= 1;
        }
        if bytes[j].is_ascii_whitespace() {
            return None;
        }
    }
    let end = j + 1;
    if bytes[j] == b')' {
        let open = matching_paren_back(text, j)?;
        let mut start = open;
        let mut k = open;
        while k > 0 && bytes[k - 1].is_ascii_whitespace() {
            k -= 1;
        }
        if k > 0 && is_ident_char(bytes[k - 1]) {
            let mut s = k;
            while s > 0 && is_ident_char(bytes[s - 1]) {
                s -= 1;
            }
            start = s;
        }
        let w = infer_width(&text[start..end], types, funcs);
        return Some((start, end, w));
    }
    if is_ident_char(bytes[j]) {
        let mut start = end;
        loop {
            let mut s = start;
            while s > 0 && is_ident_char(bytes[s - 1]) {
                s -= 1;
            }
            if s < start {
                start = s;
            }
            if start > 0 && bytes[start - 1] == b'.' {
                start -= 1;
                continue;
            }
            break;
        }
        let w = infer_width(&text[start..end], types, funcs);
        return Some((start, end, w));
    }
    if bytes[j].is_ascii_digit() {
        let mut start = end;
        while start > 0 && (bytes[start - 1].is_ascii_digit() || bytes[start - 1] == b'.') {
            start -= 1;
        }
        return Some((start, end, 1));
    }
    None
}

/// Locate the operand immediately after the operator at `op`.
fn right_atom(
    text: &str,
    op: usize,
    types: &HashMap<String, u8>,
    funcs: &HashMap<String, u8>,
) -> Option<(usize, usize, u8)> {
    let bytes = text.as_bytes();
    let mut j = op + 1;
    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
        j += 1;
    }
    while j < bytes.len() && (bytes[j] == b'-' || bytes[j] == b'+') {
        j += 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
    }
    if j >= bytes.len() {
        return None;
    }
    let start = j;
    if bytes[j] == b'(' {
        let close = matching_paren(text, j)?;
        let end = close + 1;
        let w = infer_width(&text[start..end], types, funcs);
        return Some((start, end, w));
    }
    if is_ident_start(bytes[j]) {
        while j < bytes.len() && is_ident_char(bytes[j]) {
            j += 1;
        }
        loop {
            if j < bytes.len() && bytes[j] == b'.' {
                let (sw, ni) = parse_swizzle_len(text, j);
                if sw == 0 {
                    break;
                }
                j = ni;
                continue;
            }
            break;
        }
        let mut k = j;
        while k < bytes.len() && bytes[k].is_ascii_whitespace() {
            k += 1;
        }
        if k < bytes.len() && bytes[k] == b'(' {
            if let Some(close) = matching_paren(text, k) {
                j = close + 1;
            }
        }
        let w = infer_width(&text[start..j], types, funcs);
        return Some((start, j, w));
    }
    if bytes[j].is_ascii_digit() {
        while j < bytes.len()
            && (bytes[j].is_ascii_digit()
                || bytes[j] == b'.'
                || bytes[j] == b'e'
                || bytes[j] == b'E')
        {
            j += 1;
        }
        return Some((start, j, 1));
    }
    None
}

/// HLSL shaders in the Workshop occasionally mix vector widths in arithmetic
/// (`vec3 - vec2`).  GLSL ES rejects this, so truncate the wider operand to the
/// narrower width.
fn coerce_mixed_arithmetic(
    src: &str,
    types: &HashMap<String, u8>,
    funcs: &HashMap<String, u8>,
) -> String {
    let mut out = String::with_capacity(src.len() + 64);
    for raw in src.lines() {
        let code = strip_line_comment(raw);
        let trimmed = code.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            out.push_str(raw);
            out.push('\n');
            continue;
        }
        let bytes = code.as_bytes();
        let mut inserts: Vec<(usize, String)> = Vec::new();
        let mut i = 0usize;
        while i < bytes.len() {
            let c = bytes[i];
            if matches!(c, b'+' | b'-' | b'*' | b'/' | b'%') {
                let next = bytes.get(i + 1).copied().unwrap_or(0);
                let prev = if i == 0 { 0 } else { bytes[i - 1] };
                if next == b'=' || next == c || prev == c {
                    i += 1;
                    continue;
                }
                let mut p = i;
                while p > 0 && bytes[p - 1].is_ascii_whitespace() {
                    p -= 1;
                }
                let prev_char = if p == 0 { 0 } else { bytes[p - 1] };
                let unary = p == 0
                    || matches!(
                        prev_char,
                        b'(' | b',' | b'=' | b'<' | b'>' | b'!' | b'+' | b'-' | b'*' | b'/'
                            | b'%' | b'&' | b'|' | b'?'
                    );
                if unary {
                    i += 1;
                    continue;
                }
                if let (Some((_, le, lw)), Some((_, re, rw))) =
                    (left_atom(code, i, types, funcs), right_atom(code, i, types, funcs))
                {
                    if lw >= 2 && rw >= 2 && lw != rw {
                        let swizzle = swizzle_for_width(lw.min(rw));
                        if lw > rw {
                            inserts.push((le, format!(".{swizzle}")));
                        } else {
                            inserts.push((re, format!(".{swizzle}")));
                        }
                    }
                }
            }
            i += 1;
        }
        if inserts.is_empty() {
            out.push_str(raw);
            out.push('\n');
            continue;
        }
        inserts.sort_by(|a, b| b.0.cmp(&a.0));
        let mut line = raw.to_string();
        for (pos, s) in inserts {
            if pos <= line.len() {
                line.insert_str(pos, &s);
            }
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Move global initializers that reference uniforms into `main()`; GLSL ES
/// requires global initializers to be constant expressions.
fn hoist_global_initializers(src: &str) -> String {
    let mut uniforms: Vec<String> = Vec::new();
    for raw in src.lines() {
        let line = strip_line_comment(raw).trim_start();
        if let Some(rest) = line.strip_prefix("uniform ") {
            if let Some(eq) = find_assignment(rest) {
                let lhs = rest[..eq].trim();
                if let Some(name) = lhs.split_whitespace().last() {
                    uniforms.push(name.trim_end_matches(|c: char| c == '[' || c == ']').to_string());
                }
            } else {
                let lhs = rest.trim().trim_end_matches(';').trim();
                if let Some(name) = lhs.split_whitespace().last() {
                    let name = name.split('[').next().unwrap_or(name);
                    uniforms.push(name.to_string());
                }
            }
        }
    }
    if uniforms.is_empty() {
        return src.to_string();
    }

    let mut depth = 0i32;
    let mut lines: Vec<String> = Vec::new();
    let mut assigns: Vec<String> = Vec::new();
    for raw in src.lines() {
        let code = strip_line_comment(raw);
        let trimmed = code.trim_start();
        let mut replaced = raw.to_string();
        if depth == 0 && !trimmed.starts_with('#') {
            let first_word = trimmed
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .map(|p| &trimmed[..p])
                .unwrap_or(trimmed);
            let width = match first_word {
                "float" | "vec2" | "vec3" | "vec4" => 1,
                _ => 0,
            };
            if width > 0 && trimmed.contains(';') {
                let rest = trimmed[first_word.len()..].trim_start();
                let body = &rest[..rest.find(';').unwrap()];
                if !body.contains('(') || body.find('=').map_or(false, |eq| eq < body.find('(').unwrap()) {
                    if let Some(eq) = find_assignment(body) {
                        let lhs = body[..eq].trim();
                        let rhs = body[eq + 1..].trim();
                        if !lhs.contains(',')
                            && !lhs.contains('[')
                            && uniforms.iter().any(|u| contains_word(rhs, u))
                        {
                            let indent = &raw[..raw.len() - raw.trim_start().len()];
                            let name = lhs.split_whitespace().last().unwrap_or(lhs);
                            replaced = format!("{indent}{first_word} {name};");
                            assigns.push(format!("{name} = {rhs};"));
                        }
                    }
                }
            }
        }
        lines.push(replaced);
        depth += code.matches('{').count() as i32;
        depth -= code.matches('}').count() as i32;
    }
    let mut out = lines.join("\n");
    if !assigns.is_empty() {
        if let Some(pos) = out.find("void main") {
            if let Some(open) = out[pos..].find('{') {
                let at = pos + open + 1;
                let insert = format!("\n\t{}", assigns.join("\n\t"));
                out.insert_str(at, &insert);
            }
        }
    }
    out
}

/// Names defined by object-like `#define NAME <integer>` macros.  GLSL ES
/// treats those as `int`, while the surrounding HLSL arithmetic often expects
/// floats (`albedo /= kernel + kernel + 2.0`).
fn int_define_names(src: &str) -> Vec<String> {
    let mut names = Vec::new();
    for raw in src.lines() {
        let line = strip_line_comment(raw).trim();
        let Some(rest) = line.strip_prefix("#define ") else {
            continue;
        };
        let rest = rest.trim_start();
        let name_end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        if name_end == 0 {
            continue;
        }
        let name = &rest[..name_end];
        let value = rest[name_end..].trim();
        if value.is_empty() || value.starts_with('(') {
            continue;
        }
        let value = value.strip_prefix('-').unwrap_or(value);
        if !value.is_empty() && value.chars().all(|c| c.is_ascii_digit()) {
            names.push(name.to_string());
        }
    }
    names
}

/// True when `text` contains a comparison operator at paren depth zero.
fn has_comparison(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    for i in 0..bytes.len() {
        match bytes[i] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'<' | b'>' => {
                if depth == 0 {
                    // Exclude shift operators.
                    if bytes.get(i + 1) == Some(&bytes[i]) {
                        continue;
                    }
                    return true;
                }
            }
            b'=' => {
                if depth == 0 && bytes.get(i + 1) == Some(&b'=') {
                    return true;
                }
            }
            b'!' => {
                if depth == 0 && bytes.get(i + 1) == Some(&b'=') {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// HLSL allows `bool * float`; GLSL ES does not.  Wrap comparison groups that
/// are used as arithmetic operands in `float(...)`.
fn coerce_bool_arithmetic(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 32);
    for raw in src.lines() {
        let code = strip_line_comment(raw);
        let trimmed = code.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.contains("bool ") {
            out.push_str(raw);
            out.push('\n');
            continue;
        }
        let bytes = code.as_bytes();
        let mut wraps: Vec<(usize, usize)> = Vec::new();
        for i in 0..bytes.len() {
            if bytes[i] != b'(' {
                continue;
            }
            let Some(close) = matching_paren(code, i) else {
                continue;
            };
            if !has_comparison(&code[i + 1..close]) {
                continue;
            }
            let before = code[..i].trim_end().as_bytes().last().copied().unwrap_or(0);
            let after = code[close + 1..]
                .trim_start()
                .as_bytes()
                .first()
                .copied()
                .unwrap_or(0);
            let arithmetic =
                |c: u8| matches!(c, b'*' | b'+' | b'-' | b'/' | b'%' | b'=');
            // A comparison group followed by `?` is a ternary condition.
            if after == b'?' {
                continue;
            }
            if arithmetic(before) || arithmetic(after) {
                wraps.push((i, close));
            }
        }
        if wraps.is_empty() {
            out.push_str(raw);
            out.push('\n');
            continue;
        }
        let mut inserts: Vec<(usize, &str)> = Vec::new();
        for (open, close) in wraps {
            inserts.push((open, "float("));
            inserts.push((close + 1, ")"));
        }
        inserts.sort_by(|a, b| b.0.cmp(&a.0));
        let mut line = raw.to_string();
        for (pos, s) in inserts {
            if pos <= line.len() {
                line.insert_str(pos, s);
            }
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Whether `pos` lies inside an `int(...)` cast.
fn inside_int_call(expr: &str, pos: usize) -> bool {
    let bytes = expr.as_bytes();
    let mut stack: Vec<bool> = Vec::new();
    let mut i = 0usize;
    while i < pos && i < bytes.len() {
        match bytes[i] {
            b'(' => {
                let mut k = i;
                while k > 0 && bytes[k - 1].is_ascii_whitespace() {
                    k -= 1;
                }
                let mut w = k;
                while w > 0 && is_ident_char(bytes[w - 1]) {
                    w -= 1;
                }
                stack.push(&expr[w..k] == "int");
            }
            b')' => {
                stack.pop();
            }
            _ => {}
        }
        i += 1;
    }
    stack.iter().any(|&b| b)
}

/// Wrap float-typed identifiers in `int(...)` so they can be used where GLSL
/// requires an integer (typically `for (int i = 0; i < FLOAT_VAR; i++)`).
fn wrap_int_in_expr(expr: &str, floats: &[String]) -> String {
    let bytes = expr.as_bytes();
    let mut out = String::with_capacity(expr.len() + 8);
    let mut i = 0usize;
    while i < bytes.len() {
        if is_ident_start(bytes[i]) && (i == 0 || !is_ident_char(bytes[i - 1])) {
            let start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            let word = &expr[start..i];
            if floats.iter().any(|f| f == word) && !inside_int_call(expr, start) {
                let mut k = start;
                while k > 0 && bytes[k - 1].is_ascii_whitespace() {
                    k -= 1;
                }
                let mut w = k;
                while w > 0 && is_ident_char(bytes[w - 1]) {
                    w -= 1;
                }
                if &expr[w..k] == "int" {
                    out.push_str(word);
                } else {
                    out.push_str("int(");
                    out.push_str(word);
                    out.push(')');
                }
            } else {
                out.push_str(word);
            }
            continue;
        }
        let ch = expr[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// `for (int i = -iterations; i <= iterations; i++)` where `iterations` is a
/// float: convert the float operands to ints.
fn coerce_for_headers(src: &str, types: &HashMap<String, u8>) -> String {
    let floats: Vec<String> = types
        .iter()
        .filter(|(_, w)| **w == 1)
        .map(|(n, _)| n.clone())
        .collect();
    if floats.is_empty() {
        return src.to_string();
    }
    let mut out = String::with_capacity(src.len() + 32);
    for raw in src.lines() {
        let code = strip_line_comment(raw);
        let trimmed = code.trim_start();
        if trimmed.starts_with('#') {
            out.push_str(raw);
            out.push('\n');
            continue;
        }
        let Some(pos) = trimmed.find("for") else {
            out.push_str(raw);
            out.push('\n');
            continue;
        };
        let after = &trimmed[pos + 3..];
        let after_trimmed = after.trim_start();
        if !after_trimmed.starts_with('(') {
            out.push_str(raw);
            out.push('\n');
            continue;
        }
        let open_in_trimmed = trimmed.len() - after_trimmed.len();
        let Some(close) = matching_paren(trimmed, open_in_trimmed) else {
            out.push_str(raw);
            out.push('\n');
            continue;
        };
        let header = &trimmed[open_in_trimmed + 1..close];
        let parts = split_top_level(header, ';');
        if parts.len() != 3 {
            out.push_str(raw);
            out.push('\n');
            continue;
        }
        let init = parts[0].trim();
        if !declares_type(init, "int") {
            out.push_str(raw);
            out.push('\n');
            continue;
        }
        let loop_var = init
            .split(|c: char| c.is_whitespace() || c == '=')
            .filter(|t| !t.is_empty() && !matches!(*t, "int" | "uint"))
            .next()
            .unwrap_or("")
            .to_string();
        let mut new_init = init.to_string();
        if let Some(eq) = find_assignment(init) {
            let lhs = init[..eq].trim();
            let rhs = init[eq + 1..].trim();
            new_init = format!("{lhs} = {}", wrap_int_in_expr(rhs, &floats));
        }
        let cond = parts[1].trim();
        let mut new_cond = cond.to_string();
        for op in ["<=", ">=", "==", "!=", "<", ">"] {
            if let Some(at) = cond.find(op) {
                let lhs = cond[..at].trim();
                let rhs = cond[at + op.len()..].trim();
                let new_lhs = if lhs == loop_var {
                    lhs.to_string()
                } else {
                    wrap_int_in_expr(lhs, &floats)
                };
                let new_rhs = if rhs == loop_var {
                    rhs.to_string()
                } else {
                    wrap_int_in_expr(rhs, &floats)
                };
                new_cond = format!("{new_lhs} {op} {new_rhs}");
                break;
            }
        }
        let indent = &raw[..raw.len() - raw.trim_start().len()];
        let tail = &trimmed[close + 1..];
        out.push_str(&format!(
            "{indent}for ({new_init}; {new_cond}; {}){tail}",
            parts[2].trim()
        ));
        out.push('\n');
    }
    out
}

/// Column of the `]` matching the `[` at `open`.
fn matching_bracket(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.get(open) != Some(&b'[') {
        return None;
    }
    let mut depth = 0i32;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn atom_is_float(text: &str, types: &HashMap<String, u8>) -> bool {
    let t = text.trim();
    if t.contains('.') {
        return true;
    }
    matches!(types.get(t), Some(1))
}

/// HLSL allows `float % float` (truncating to ints); GLSL ES only defines `%`
/// for integers.  Rewrite float modulo to `mod(a, b)`.
fn coerce_float_modulo(
    src: &str,
    types: &HashMap<String, u8>,
    funcs: &HashMap<String, u8>,
) -> String {
    let mut out = String::with_capacity(src.len() + 32);
    for raw in src.lines() {
        let code = strip_line_comment(raw);
        let trimmed = code.trim_start();
        if trimmed.starts_with('#') || (!code.contains('%') && !code.contains('/')) {
            out.push_str(raw);
            out.push('\n');
            continue;
        }
        let bytes = code.as_bytes();
        let mut edits: Vec<(usize, usize, String)> = Vec::new();
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] == b'%' && bytes.get(i + 1) != Some(&b'=') {
                let prev_c = if i == 0 { 0 } else { bytes[i - 1] };
                if prev_c == b'%' {
                    i += 1;
                    continue;
                }
                if let (Some((ls, _, _)), Some((_, re, _))) = (
                    left_atom(code, i, types, funcs),
                    right_atom(code, i, types, funcs),
                ) {
                    let left = &code[ls..i];
                    let right = &code[i + 1..re];
                    if atom_is_float(left, types) || atom_is_float(right, types) {
                        let l = left.trim();
                        let r = right.trim();
                        let rf = if r.contains('.') || matches!(types.get(r), Some(1)) {
                            r.to_string()
                        } else {
                            format!("float({r})")
                        };
                        edits.push((ls, re, format!("mod({l}, {rf})")));
                    }
                }
            } else if bytes[i] == b'/'
                && bytes.get(i + 1) != Some(&b'=')
                && (i == 0 || bytes[i - 1] != b'/')
            {
                if let (Some((ls, _, _)), Some((rs, re, _))) = (
                    left_atom(code, i, types, funcs),
                    right_atom(code, i, types, funcs),
                ) {
                    let left = &code[ls..i];
                    let right = code[rs..re].trim();
                    if atom_is_float(left, types)
                        && !right.is_empty()
                        && right.chars().all(|c| c.is_ascii_digit())
                    {
                        edits.push((rs, re, format!("float({right})")));
                    }
                }
            }
            i += 1;
        }
        if edits.is_empty() {
            out.push_str(raw);
            out.push('\n');
            continue;
        }
        edits.sort_by(|a, b| b.0.cmp(&a.0));
        let mut line = raw.to_string();
        for (s, e, text) in edits {
            if e <= line.len() {
                line.replace_range(s..e, &text);
            }
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Collect names declared as arrays (`uniform float name[64];`) so their
/// indices can be coerced.
fn array_names(src: &str) -> Vec<String> {
    let mut names = Vec::new();
    for raw in src.lines() {
        let line = strip_line_comment(raw);
        let bytes = line.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if !is_ident_start(bytes[i]) || (i > 0 && is_ident_char(bytes[i - 1])) {
                i += 1;
                continue;
            }
            let start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            let word = &line[start..i];
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if bytes.get(j) != Some(&b'[') {
                continue;
            }
            // Only declarations (`float name[...]`) introduce arrays; matrix
            // and vector indexing at use sites must not be flattened.
            let mut q = start;
            while q > 0 && bytes[q - 1].is_ascii_whitespace() {
                q -= 1;
            }
            let mut w = q;
            while w > 0 && is_ident_char(bytes[w - 1]) {
                w -= 1;
            }
            let prev_word = &line[w..q];
            if matches!(
                prev_word,
                "float" | "vec2" | "vec3" | "vec4" | "int" | "uint" | "mat3" | "mat4"
            ) && !names.iter().any(|n| n == word)
            {
                names.push(word.to_string());
            }
        }
    }
    names
}

fn int_cast(expr: &str) -> String {
    let t = expr.trim();
    if t.starts_with("int(") {
        t.to_string()
    } else {
        format!("int({t})")
    }
}

/// GLSL requires integer array indices; HLSL truncates floats.  Also flatten
/// double indexing of 1-D arrays, which Workshop shaders use on the audio
/// spectrum uniforms (`arr[i / 4][i % 4]`).
fn coerce_array_indices(
    src: &str,
    types: &HashMap<String, u8>,
) -> String {
    let arrays = array_names(src);
    if arrays.is_empty() {
        return src.to_string();
    }
    let mut out = String::with_capacity(src.len() + 32);
    for raw in src.lines() {
        let code = strip_line_comment(raw);
        let trimmed = code.trim_start();
        if trimmed.starts_with('#') {
            out.push_str(raw);
            out.push('\n');
            continue;
        }
        let bytes = code.as_bytes();
        let mut edits: Vec<(usize, usize, String)> = Vec::new();
        let mut i = 0usize;
        while i < bytes.len() {
            if !is_ident_start(bytes[i]) || (i > 0 && is_ident_char(bytes[i - 1])) {
                i += 1;
                continue;
            }
            let start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            let word = &code[start..i];
            if !arrays.iter().any(|n| n == word) {
                continue;
            }
            // Skip the declaration itself (`float name[64]`).
            let mut q = start;
            while q > 0 && bytes[q - 1].is_ascii_whitespace() {
                q -= 1;
            }
            let mut w = q;
            while w > 0 && is_ident_char(bytes[w - 1]) {
                w -= 1;
            }
            let prev_word = &code[w..q];
            if matches!(
                prev_word,
                "float" | "vec2" | "vec3" | "vec4" | "int" | "uint" | "mat3" | "mat4"
            ) {
                continue;
            }
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if bytes.get(j) != Some(&b'[') {
                continue;
            }
            let Some(close1) = matching_bracket(code, j) else {
                continue;
            };
            let inner1 = &code[j + 1..close1];
            let mut k = close1 + 1;
            while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                k += 1;
            }
            if bytes.get(k) == Some(&b'[') {
                let Some(close2) = matching_bracket(code, k) else {
                    continue;
                };
                let inner2 = &code[k + 1..close2];
                let flat = format!(
                    "{word}[{} * 4 + {}]",
                    int_cast(inner1),
                    int_cast(inner2)
                );
                edits.push((start, close2 + 1, flat));
                i = close2 + 1;
                continue;
            }
            let needs_int = inner1.contains('.')
                || inner1
                    .split(|c: char| !is_ident_char(c as u8))
                    .any(|tok| matches!(types.get(tok), Some(1)))
                || inner1.contains("mod(");
            if needs_int && !inner1.trim().starts_with("int(") {
                edits.push((start, close1 + 1, format!("{word}[{}]", int_cast(inner1))));
                i = close1 + 1;
            }
        }
        if edits.is_empty() {
            out.push_str(raw);
            out.push('\n');
            continue;
        }
        edits.sort_by(|a, b| b.0.cmp(&a.0));
        let mut line = raw.to_string();
        for (s, e, text) in edits {
            if e <= line.len() {
                line.replace_range(s..e, &text);
            }
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Width of the return type if `line` starts a function definition.
fn function_def_width(line: &str) -> Option<u8> {
    let t = line.trim_start();
    if t.starts_with("return") || t.starts_with('#') {
        return None;
    }
    let bytes = t.as_bytes();
    let mut i = 0usize;
    if !is_ident_start(bytes.first().copied().unwrap_or(0)) {
        return None;
    }
    while i < bytes.len() && is_ident_char(bytes[i]) {
        i += 1;
    }
    let width = match &t[..i] {
        "float" => 1,
        "vec2" => 2,
        "vec3" => 3,
        "vec4" => 4,
        _ => return None,
    };
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() || !is_ident_start(bytes[i]) {
        return None;
    }
    while i < bytes.len() && is_ident_char(bytes[i]) {
        i += 1;
    }
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if bytes.get(i) == Some(&b'(') {
        Some(width)
    } else {
        None
    }
}

/// Find the nearest preceding declaration of `name` (`vec4 name`, `float
/// name`, ...) before line `from`.  Needed when a name is declared with
/// different types in different functions, which makes the global map
/// ambiguous.
fn declared_width_above(lines: &[&str], name: &str, from: usize) -> Option<u8> {
    for idx in (0..from).rev() {
        let line = strip_line_comment(lines[idx]);
        let bytes = line.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if !is_ident_start(bytes[i]) || (i > 0 && is_ident_char(bytes[i - 1])) {
                i += 1;
                continue;
            }
            let start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            let word = &line[start..i];
            let width = match word {
                "float" => 1,
                "vec2" => 2,
                "vec3" => 3,
                "vec4" => 4,
                _ => continue,
            };
            let mut k = i;
            while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                k += 1;
            }
            let ns = k;
            while k < bytes.len() && is_ident_char(bytes[k]) {
                k += 1;
            }
            if &line[ns..k] == name {
                return Some(width);
            }
        }
    }
    None
}

fn coerce_return_expr(
    expr: &str,
    ret: u8,
    types: &HashMap<String, u8>,
    funcs: &HashMap<String, u8>,
    lines: &[&str],
    idx: usize,
) -> String {
    let coerced = coerce_expr(expr, ret, types, funcs);
    if coerced != expr {
        return coerced;
    }
    let t = expr.trim();
    if t.is_empty() || !t.bytes().all(is_ident_char) {
        return coerced;
    }
    match declared_width_above(lines, t, idx) {
        Some(w) if w > ret && ret < 4 => {
            format!("({t}).{}", swizzle_for_width(ret))
        }
        Some(1) if ret > 1 => format!("vec{ret}({t})"),
        _ => coerced,
    }
}

/// HLSL truncates on `return vec4` from a `vec3` function; GLSL ES does not.
fn coerce_return_values(
    src: &str,
    types: &HashMap<String, u8>,
    funcs: &HashMap<String, u8>,
) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = String::with_capacity(src.len() + 32);
    let mut depth = 0i32;
    let mut stack: Vec<u8> = Vec::new();
    let mut pending: Option<u8> = None;
    for (idx, raw) in lines.iter().enumerate() {
        let raw = *raw;
        let code = strip_line_comment(raw);
        let trimmed = code.trim_start();
        if depth == 0 && !trimmed.starts_with('#') {
            if let Some(w) = function_def_width(trimmed) {
                pending = Some(w);
            }
        }
        let mut line = raw.to_string();
        if let Some(rest) = trimmed.strip_prefix("return ") {
            let ret = stack.iter().rev().copied().find(|&w| w > 0).unwrap_or(0);
            if ret > 0 {
                if let Some(semi) = rest.rfind(';') {
                    let expr = rest[..semi].trim();
                    let coerced = coerce_return_expr(expr, ret, types, funcs, &lines, idx);
                    if coerced != expr {
                        let comment = raw.find("//").map(|p| &raw[p..]).unwrap_or("");
                        let indent = &raw[..raw.len() - raw.trim_start().len()];
                        line = format!("{indent}return {coerced};{comment}");
                    }
                }
            }
        }
        for ch in code.chars() {
            match ch {
                '{' => {
                    if depth == 0 {
                        stack.push(pending.take().unwrap_or(0));
                    } else {
                        stack.push(0);
                    }
                    depth += 1;
                }
                '}' => {
                    if depth > 0 {
                        depth -= 1;
                        stack.pop();
                    }
                }
                _ => {}
            }
        }
        if pending.is_some() && !code.contains('{') && trimmed.ends_with(';') {
            pending = None;
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Convert integer conditions (`INVERT ? a : b`, `if (INVERT)`) to booleans.
fn coerce_bool_conditions(src: &str, int_names: &[String]) -> String {
    let mut out = String::with_capacity(src.len() + 32);
    for raw in src.lines() {
        let code = strip_line_comment(raw);
        if code.trim_start().starts_with('#') {
            out.push_str(raw);
            out.push('\n');
            continue;
        }
        let mut line = raw.to_string();
        // `if (IDENT)` / `while (IDENT)`
        for kw in ["if", "while"] {
            let mut from = 0usize;
            while let Some(p) = line[from..].find(kw) {
                let at = from + p;
                let after_kw = at + kw.len();
                let mut j = after_kw;
                while j < line.len() && line.as_bytes()[j].is_ascii_whitespace() {
                    j += 1;
                }
                if line.as_bytes().get(j) == Some(&b'(') {
                    if let Some(end) = matching_paren(&line, j) {
                        let inner = line[j + 1..end].trim();
                        if int_names.iter().any(|n| n == inner) {
                            let replacement = format!("({inner} != 0)");
                            line.replace_range(j + 1..end, &replacement);
                        }
                        from = end + 1;
                        continue;
                    }
                }
                from = after_kw;
            }
        }
        // Ternary conditions: `IDENT ? ... : ...`
        if let Some(q) = line.find('?') {
            let mut k = q;
            while k > 0 && line.as_bytes()[k - 1].is_ascii_whitespace() {
                k -= 1;
            }
            let end = k;
            while k > 0 && is_ident_char(line.as_bytes()[k - 1]) {
                k -= 1;
            }
            if k < end {
                let word = &line[k..end];
                if int_names.iter().any(|n| n == word) {
                    let before_ok = k == 0
                        || matches!(
                            line.as_bytes()[k - 1],
                            b'(' | b'=' | b'!' | b'&' | b'|' | b',' | b' ' | b'\t'
                        );
                    if before_ok {
                        line.replace_range(k..end, &format!("({word} != 0)"));
                    }
                }
            }
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Make vertex outputs and fragment inputs agree on varying types (WE/HLSL
/// tolerates mismatched vector widths; GLSL ES refuses to link).
fn reconcile_varyings(vert: &str, frag: &str) -> (String, String) {
    let mut vert_out: HashMap<String, u8> = HashMap::new();
    for raw in vert.lines() {
        let t = raw.trim_start();
        if let Some(rest) = t.strip_prefix("out ") {
            let mut words = rest.split_whitespace();
            if let (Some(ty), Some(name)) = (words.next(), words.next()) {
                let w = match ty {
                    "float" => 1,
                    "vec2" => 2,
                    "vec3" => 3,
                    "vec4" => 4,
                    _ => 0,
                };
                if w > 0 {
                    vert_out.insert(name.trim_end_matches(';').to_string(), w);
                }
            }
        }
    }
    let mut frag_in: HashMap<String, u8> = HashMap::new();
    for raw in frag.lines() {
        let t = raw.trim_start();
        if let Some(rest) = t.strip_prefix("in ") {
            let mut words = rest.split_whitespace();
            if let (Some(ty), Some(name)) = (words.next(), words.next()) {
                let w = match ty {
                    "float" => 1,
                    "vec2" => 2,
                    "vec3" => 3,
                    "vec4" => 4,
                    _ => 0,
                };
                if w > 0 {
                    frag_in.insert(name.trim_end_matches(';').to_string(), w);
                }
            }
        }
    }
    let mut replace: HashMap<String, (u8, u8)> = HashMap::new();
    for (name, &vw) in &vert_out {
        if let Some(&fw) = frag_in.get(name) {
            if vw != fw {
                replace.insert(name.clone(), (vw, fw));
            }
        }
    }
    if replace.is_empty() {
        return (vert.to_string(), frag.to_string());
    }
    let name_for = |w: u8| match w {
        1 => "float",
        2 => "vec2",
        3 => "vec3",
        _ => "vec4",
    };
    let mut new_vert = String::with_capacity(vert.len());
    for raw in vert.lines() {
        let t = raw.trim_start();
        if let Some(rest) = t.strip_prefix("out ") {
            let mut words = rest.split_whitespace();
            if let (Some(_ty), Some(name)) = (words.next(), words.next()) {
                let name = name.trim_end_matches(';');
                if let Some(&(_, fw)) = replace.get(name) {
                    let indent = &raw[..raw.len() - t.len()];
                    new_vert.push_str(&format!("{indent}out {} {name};", name_for(fw)));
                    new_vert.push('\n');
                    continue;
                }
            }
        }
        // Drop assignments to components the new type does not have.
        let mut dropped = false;
        for (name, &(_, fw)) in &replace {
            if let Some(rest) = t.strip_prefix(&format!("{name}.")) {
                let sw = &rest[..rest.find(|c: char| !matches!(c, 'x' | 'y' | 'z' | 'w' | 'r' | 'g' | 'b' | 'a' | 's' | 't' | 'p' | 'q')).unwrap_or(rest.len())];
                let max_comp = sw.bytes().filter_map(|c| match c {
                    b'x' | b'r' | b's' => Some(0u8),
                    b'y' | b'g' | b't' => Some(1),
                    b'z' | b'b' | b'p' => Some(2),
                    b'w' | b'a' | b'q' => Some(3),
                    _ => None,
                }).max();
                if max_comp.map_or(false, |m| m >= fw) {
                    dropped = true;
                }
            }
        }
        if !dropped {
            new_vert.push_str(raw);
            new_vert.push('\n');
        }
    }
    (new_vert, frag.to_string())
}

fn replace_word(src: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let is_word_start = is_ident_start(bytes[i]);
        if is_word_start {
            let start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            let word = &src[start..i];
            if word == from {
                out.push_str(to);
            } else {
                out.push_str(word);
            }
        } else {
            let ch = src[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Uniform reflection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum UniformKind {
    Float,
    Int,
    Bool,
    Vec2,
    Vec3,
    Vec4,
    Mat3,
    Mat4,
    Sampler,
    FloatArray(usize),
    Other,
}

#[derive(Debug, Clone)]
struct UniformSpec {
    name: String,
    kind: UniformKind,
    material: Option<String>,
    alt_materials: Vec<String>,
    label: Option<String>,
    default: Option<Value>,
    default_tex: Option<String>,
    slot: Option<usize>,
}

struct CompiledProgram {
    prog: Program,
    uniforms: Vec<UniformSpec>,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    blending: BlendMode,
}

unsafe impl Send for CompiledProgram {}

impl CompiledProgram {
    /// Release the GL program, VAO and VBO.  Needs a current context, so it
    /// cannot live in `Drop`.
    fn destroy(&self, gl: &glow::Context) {
        self.prog.destroy(gl);
        unsafe {
            gl.delete_vertex_array(self.vao);
            gl.delete_buffer(self.vbo);
        }
    }
}

impl CompiledProgram {
    fn new(
        gl: &glow::Context,
        name: &str,
        vert_src: &str,
        frag_src: &str,
        combos: &[(String, i32)],
        blending: BlendMode,
    ) -> Result<Self> {
        let vs = preprocess(vert_src, true, combos);
        let fs = preprocess(frag_src, false, combos);
        let (vs, fs) = reconcile_varyings(&vs, &fs);
        let prog = match Program::new(gl, &vs, &fs) {
            Ok(p) => p,
            Err(err) => {
                if std::env::var_os("ANIMA_DUMP_SHADERS").is_some() {
                    let dir = std::path::Path::new("/tmp/anima-shaders");
                    let _ = std::fs::create_dir_all(dir);
                    let safe = name.replace('/', "_");
                    let _ = std::fs::write(dir.join(format!("FAIL-{safe}.vert.glsl")), &vs);
                    let _ = std::fs::write(dir.join(format!("FAIL-{safe}.frag.glsl")), &fs);
                }
                return Err(err);
            }
        };
        let exp_frag = filter_active_lines(&expand_includes(frag_src, 0), combos);
        let exp_vert = filter_active_lines(&expand_includes(vert_src, 0), combos);
        let uniforms = parse_uniforms(&exp_frag);
        let uniforms_v = parse_uniforms(&exp_vert);
        let mut all = uniforms;
        for u in uniforms_v {
            if let Some(existing) = all.iter_mut().find(|x| x.name == u.name) {
                if existing.material.is_none() {
                    existing.material = u.material.clone();
                } else if let Some(m) = u.material {
                    if existing.material.as_deref() != Some(m.as_str())
                        && !existing.alt_materials.contains(&m)
                    {
                        existing.alt_materials.push(m);
                    }
                }
                if existing.label.is_none() {
                    existing.label = u.label;
                }
                if existing.default.is_none() {
                    existing.default = u.default;
                }
                if existing.default_tex.is_none() {
                    existing.default_tex = u.default_tex;
                }
            } else {
                all.push(u);
            }
        }

        // One fullscreen quad per program, configured for the attribute
        // locations the linker actually assigned.  Render targets store the
        // scene top-down (row 0 = top), and WE shaders treat uv.y = 0 as the
        // top, so the top of the output (NDC y = -1) samples v = 0.
        const QUAD: [f32; 20] = [
            -1.0, -1.0, 0.0, 0.0, 0.0,
             1.0, -1.0, 0.0, 1.0, 0.0,
            -1.0,  1.0, 0.0, 0.0, 1.0,
             1.0,  1.0, 0.0, 1.0, 1.0,
        ];
        unsafe {
            let vao = gl.create_vertex_array().map_err(|e| anyhow!(e))?;
            let vbo = gl.create_buffer().map_err(|e| anyhow!(e))?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck_cast(&QUAD), glow::STATIC_DRAW);
            let stride = 5 * 4;
            let pos_attr = gl
                .get_attrib_location(prog.id, "a_Position")
                .or_else(|| gl.get_attrib_location(prog.id, "a_Pos"));
            if let Some(pos) = pos_attr {
                gl.enable_vertex_attrib_array(pos);
                gl.vertex_attrib_pointer_f32(pos, 3, glow::FLOAT, false, stride, 0);
            }
            if let Some(uv) = gl.get_attrib_location(prog.id, "a_TexCoord") {
                gl.enable_vertex_attrib_array(uv);
                gl.vertex_attrib_pointer_f32(uv, 2, glow::FLOAT, false, stride, 12);
            }
            gl.bind_vertex_array(None);
            Ok(Self { prog, uniforms: all, vao, vbo, blending })
        }
    }
}

/// Evaluate simple `#if` / `#elif` / `#else` / `#endif` combo blocks so
/// uniform metadata from inactive shader branches (such as `#if MODE == 1` in
/// `foliagesway.vert` when `MODE == 0`) does not shadow active declarations.
fn filter_active_lines(src: &str, combos: &[(String, i32)]) -> String {
    let lookup = |name: &str| -> i32 {
        combos
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| *v)
            .unwrap_or(0)
    };
    fn eval_atom(expr: &str, lookup: &dyn Fn(&str) -> i32) -> i32 {
        let e = expr.trim().trim_start_matches('(').trim_end_matches(')').trim();
        if let Some(rest) = e.strip_prefix('!') {
            return if eval_atom(rest, lookup) == 0 { 1 } else { 0 };
        }
        if let Some((a, b)) = e.split_once("==") {
            let va = a.trim().parse::<i32>().unwrap_or_else(|_| lookup(a.trim()));
            let vb = b.trim().parse::<i32>().unwrap_or_else(|_| lookup(b.trim()));
            return i32::from(va == vb);
        }
        if let Some((a, b)) = e.split_once("!=") {
            let va = a.trim().parse::<i32>().unwrap_or_else(|_| lookup(a.trim()));
            let vb = b.trim().parse::<i32>().unwrap_or_else(|_| lookup(b.trim()));
            return i32::from(va != vb);
        }
        if let Some((a, b)) = e.split_once(">=") {
            let va = a.trim().parse::<i32>().unwrap_or_else(|_| lookup(a.trim()));
            let vb = b.trim().parse::<i32>().unwrap_or_else(|_| lookup(b.trim()));
            return i32::from(va >= vb);
        }
        if let Some((a, b)) = e.split_once("<=") {
            let va = a.trim().parse::<i32>().unwrap_or_else(|_| lookup(a.trim()));
            let vb = b.trim().parse::<i32>().unwrap_or_else(|_| lookup(b.trim()));
            return i32::from(va <= vb);
        }
        if let Some((a, b)) = e.split_once('>') {
            let va = a.trim().parse::<i32>().unwrap_or_else(|_| lookup(a.trim()));
            let vb = b.trim().parse::<i32>().unwrap_or_else(|_| lookup(b.trim()));
            return i32::from(va > vb);
        }
        if let Some((a, b)) = e.split_once('<') {
            let va = a.trim().parse::<i32>().unwrap_or_else(|_| lookup(a.trim()));
            let vb = b.trim().parse::<i32>().unwrap_or_else(|_| lookup(b.trim()));
            return i32::from(va < vb);
        }
        e.parse::<i32>().unwrap_or_else(|_| lookup(e))
    }
    fn eval_cond(expr: &str, lookup: &dyn Fn(&str) -> i32) -> bool {
        let clean = expr.split("//").next().unwrap_or(expr).trim();
        clean.split("||").any(|or_part| {
            or_part
                .split("&&")
                .all(|and_part| eval_atom(and_part, lookup) != 0)
        })
    }

    // Stack of (parent_active, branch_active, matched_any_branch)
    let mut stack: Vec<(bool, bool, bool)> = Vec::new();
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let t = line.trim_start();
        let parent_active = stack.last().map(|&(_, a, _)| a).unwrap_or(true);
        if let Some(rest) = t.strip_prefix("#ifdef ") {
            let name = rest.split_whitespace().next().unwrap_or("");
            let cond = parent_active && lookup(name) != 0;
            stack.push((parent_active, cond, cond));
            continue;
        }
        if let Some(rest) = t.strip_prefix("#ifndef ") {
            let name = rest.split_whitespace().next().unwrap_or("");
            let cond = parent_active && lookup(name) == 0;
            stack.push((parent_active, cond, cond));
            continue;
        }
        if let Some(rest) = t.strip_prefix("#if ") {
            let cond = parent_active && eval_cond(rest, &lookup);
            stack.push((parent_active, cond, cond));
            continue;
        }
        if let Some(rest) = t.strip_prefix("#elif ") {
            if let Some((p_act, b_act, seen)) = stack.last_mut() {
                if *p_act && !*seen && eval_cond(rest, &lookup) {
                    *b_act = true;
                    *seen = true;
                } else {
                    *b_act = false;
                }
            }
            continue;
        }
        if t.starts_with("#else") {
            if let Some((p_act, b_act, seen)) = stack.last_mut() {
                *b_act = *p_act && !*seen;
                *seen = true;
            }
            continue;
        }
        if t.starts_with("#endif") {
            stack.pop();
            continue;
        }
        if parent_active {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn parse_uniforms(src: &str) -> Vec<UniformSpec> {
    let mut out = Vec::new();
    for raw_line in src.lines() {
        let line = raw_line.trim();
        if !line.starts_with("uniform ") {
            continue;
        }
        let rest = &line["uniform ".len()..];
        let rest = rest.split(';').next().unwrap_or(rest).trim();
        let mut parts = rest.split_whitespace();
        let Some(ty) = parts.next() else { continue };
        let Some(name_part) = parts.next() else { continue };
        let name = name_part.split('[').next().unwrap_or(name_part).trim().to_string();
        if name.is_empty() {
            continue;
        }
        let kind = match ty {
            "float" => {
                if let Some(len) = name_part.split('[').nth(1) {
                    let n = len.trim_end_matches(']').parse().unwrap_or(0);
                    UniformKind::FloatArray(n)
                } else {
                    UniformKind::Float
                }
            }
            "int" => UniformKind::Int,
            "bool" => UniformKind::Bool,
            "vec2" => UniformKind::Vec2,
            "vec3" => UniformKind::Vec3,
            "vec4" => UniformKind::Vec4,
            "mat3" => UniformKind::Mat3,
            "mat4" => UniformKind::Mat4,
            "sampler2D" => UniformKind::Sampler,
            _ => UniformKind::Other,
        };
        let meta = line
            .split_once("//")
            .and_then(|(_, c)| serde_json::from_str::<Value>(c.trim()).ok());
        let material = meta
            .as_ref()
            .and_then(|m| m.get("material"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let label = meta
            .as_ref()
            .and_then(|m| m.get("label"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let default = meta.as_ref().and_then(|m| m.get("default")).cloned();
        let default_tex = meta
            .as_ref()
            .and_then(|m| m.get("default"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let slot = if matches!(kind, UniformKind::Sampler) {
            name.strip_prefix("g_Texture")
                .and_then(|s| s.parse::<usize>().ok())
        } else {
            None
        };
        out.push(UniformSpec {
            name,
            kind,
            material,
            alt_materials: Vec::new(),
            label,
            default,
            default_tex,
            slot,
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Effect definitions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    Normal,
    Additive,
    Multiply,
    Screen,
}

impl BlendMode {
    fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "additive" | "add" => BlendMode::Additive,
            "multiply" | "mult" => BlendMode::Multiply,
            "screen" => BlendMode::Screen,
            _ => BlendMode::Normal,
        }
    }
}

struct EffectPass {
    program: Option<CompiledProgram>,
    /// texture index -> either "previous"/"input" or a named FBO.
    binds: HashMap<usize, String>,
    target: Option<String>,
    /// Declared textures for this pass (index -> material texture path).
    textures: Vec<Option<String>>,
    /// constant key -> value.
    constants: HashMap<String, Value>,
    /// combo definitions for this pass.
    combos: Vec<(String, i32)>,
}

pub struct EffectInstance {
    passes: Vec<EffectPass>,
    fbos: Vec<(String, i32)>,
    named: HashMap<String, Canvas>,
    dims: (i32, i32),
}

impl EffectInstance {
    /// Release every canvas and shader program owned by this effect chain.
    pub fn destroy(self, gl: &glow::Context) {
        for (_, canvas) in self.named {
            canvas.destroy(gl);
        }
        for pass in self.passes {
            if let Some(program) = pass.program {
                program.destroy(gl);
            }
        }
    }

    /// Whether any pass in this effect reads audio spectrum uniforms, enables
    /// `AUDIOPROCESSING`, or binds an audio-reactive property script.
    pub fn uses_audio(&self) -> bool {
        self.passes.iter().any(|p| {
            p.combos
                .iter()
                .any(|(k, v)| k == "AUDIOPROCESSING" && *v > 0)
                || p.program.as_ref().is_some_and(|prog| {
                    prog.uniforms
                        .iter()
                        .any(|u| u.name.starts_with("g_AudioSpectrum"))
                })
                || p.constants.values().any(|v| {
                    v.get("script")
                        .and_then(|s| s.as_str())
                        .is_some_and(|s| s.contains("registerAudioBuffers"))
                })
        })
    }
}

pub struct EffectContext<'a> {
    pub time: f32,
    pub frametime: f32,
    pub daytime: f32,
    pub mouse: [f32; 2],
    pub audio: [f32; 4],
    pub spectrum: &'a [f32; 64],
    pub projection_inverse: [f32; 16],
    pub composites: &'a HashMap<i64, Canvas>,
    pub self_id: Option<i64>,
    pub dependencies: &'a [i64],
}

fn read_text(pkg: &PackageFs, path: &str) -> Option<String> {
    if let Ok(s) = pkg.read_to_string(path) {
        return Some(s);
    }
    we_asset(path)
}

fn we_asset(path: &str) -> Option<String> {
    let p = we_asset_path(path)?;
    std::fs::read_to_string(p).ok()
}

fn we_asset_path(path: &str) -> Option<PathBuf> {
    crate::pkg::resolve_we_asset(path)
}

fn read_json(pkg: &PackageFs, path: &str) -> Option<Value> {
    read_text(pkg, path).and_then(|s| serde_json::from_str(&s).ok())
}

fn parse_combo_defaults(src: &str) -> Vec<(String, i32)> {
    let mut out = Vec::new();
    for line in src.lines() {
        let Some((_, tail)) = line.split_once("[COMBO") else {
            continue;
        };
        let Ok(meta) = serde_json::from_str::<Value>(tail.trim_start_matches(']').trim()) else {
            continue;
        };
        let Some(name) = meta.get("combo").and_then(|v| v.as_str()) else {
            continue;
        };
        let value = meta
            .get("default")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        out.push((name.to_string(), value));
    }
    out
}

fn merge_combos(
    a: &[(String, i32)],
    b: &[(String, i32)],
) -> Vec<(String, i32)> {
    let mut out: Vec<(String, i32)> = a.to_vec();
    for (k, v) in b {
        if let Some(existing) = out.iter_mut().find(|(name, _)| name == k) {
            existing.1 = *v;
        } else {
            out.push((k.clone(), *v));
        }
    }
    out
}

impl EffectInstance {
    /// Build an effect from an object's `effects[]` entry.  Returns `None` if
    /// the effect definition or all of its shaders are missing.
    pub fn load(
        gl: &glow::Context,
        pkg: &PackageFs,
        effect_file: &str,
        scene_effect: &Value,
    ) -> Option<Self> {
        let def = read_json(pkg, effect_file)?;
        let def_passes = def.get("passes")?.as_array()?;
        let scene_passes = scene_effect
            .get("passes")
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default();

        let mut passes = Vec::new();
        for (i, def_pass) in def_passes.iter().enumerate() {
            let material_path = def_pass.get("material")?.as_str()?;
            let material = read_json(pkg, material_path)?;
            let mat_pass = material
                .get("passes")
                .and_then(|p| p.as_array())
                .and_then(|a| a.first())?;
            let shader = mat_pass.get("shader")?.as_str()?;
            let vert_path = format!("shaders/{shader}.vert");
            let frag_path = format!("shaders/{shader}.frag");
            let Some(vert_src) = read_text(pkg, &vert_path) else {
                log::debug!("effect {effect_file}: missing {vert_path}");
                return None;
            };
            let Some(frag_src) = read_text(pkg, &frag_path) else {
                log::debug!("effect {effect_file}: missing {frag_path}");
                return None;
            };

            let scene_pass = scene_passes.get(i);
            let mut combos = parse_combo_defaults(&vert_src);
            combos = merge_combos(&combos, &parse_combo_defaults(&frag_src));
            if let Some(c) = mat_pass.get("combos").and_then(|c| c.as_object()) {
                for (k, v) in c {
                    if let Some(n) = v.as_i64() {
                        combos = merge_combos(&combos, &[(k.clone(), n as i32)]);
                    }
                }
            }

            // A texture bound to a slot whose uniform declares a `MASK`/other
            // combo enables that combo unless the scene pass explicitly overrides it.
            let mut textures: Vec<Option<String>> = mat_pass
                .get("textures")
                .and_then(|t| t.as_array())
                .map(|a| a.iter().map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            if let Some(t) = scene_pass
                .and_then(|p| p.get("textures"))
                .and_then(|t| t.as_array())
            {
                for (idx, v) in t.iter().enumerate() {
                    if idx >= textures.len() {
                        textures.resize(idx + 1, None);
                    }
                    if let Some(s) = v.as_str() {
                        textures[idx] = Some(s.to_string());
                    }
                }
            }

            // Texture combo inference from uniform metadata across both stages.
            let combined_src = format!("{vert_src}\n{frag_src}");
            for spec in parse_uniforms(&combined_src) {
                if let Some(slot) = spec.slot {
                    if textures.get(slot).and_then(|t| t.as_deref()).is_some() {
                        for line in combined_src.lines() {
                            if line.contains(&format!("uniform sampler2D {}", spec.name)) {
                                if let Some((_, c)) = line.split_once("//") {
                                    if let Ok(meta) = serde_json::from_str::<Value>(c.trim()) {
                                        if let Some(combo) = meta.get("combo").and_then(|v| v.as_str()) {
                                            combos = merge_combos(&combos, &[(combo.to_string(), 1)]);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Merge explicit `scene_pass.combos` AFTER texture-combo inference
            // so authored combo values (such as `"MASK": 0`) are never clobbered.
            if let Some(c) = scene_pass
                .and_then(|p| p.get("combos"))
                .and_then(|c| c.as_object())
            {
                for (k, v) in c {
                    if let Some(n) = v.as_i64() {
                        combos = merge_combos(&combos, &[(k.clone(), n as i32)]);
                    }
                }
            }

            let blending = BlendMode::from_str(
                mat_pass
                    .get("blending")
                    .and_then(|v| v.as_str())
                    .unwrap_or("normal"),
            );
            let program = match CompiledProgram::new(gl, shader, &vert_src, &frag_src, &combos, blending)
            {
                Ok(p) => Some(p),
                Err(err) => {
                    log::warn!("effect shader {shader} failed: {err}");
                    return None;
                }
            };

            let mut constants = HashMap::new();
            if let Some(c) = mat_pass.get("constantshadervalues").and_then(|v| v.as_object()) {
                for (k, v) in c {
                    constants.insert(k.to_ascii_lowercase(), v.clone());
                }
            }
            if let Some(c) = scene_pass
                .and_then(|p| p.get("constantshadervalues"))
                .and_then(|v| v.as_object())
            {
                for (k, v) in c {
                    constants.insert(k.to_ascii_lowercase(), v.clone());
                }
            }

            let mut binds = HashMap::new();
            if let Some(b) = def_pass.get("bind").and_then(|v| v.as_array()) {
                for bind in b {
                    let name = bind.get("name").and_then(|v| v.as_str());
                    let index = bind.get("index").and_then(|v| v.as_u64());
                    if let (Some(name), Some(index)) = (name, index) {
                        binds.insert(index as usize, name.to_string());
                    }
                }
            }

            passes.push(EffectPass {
                program,
                binds,
                target: def_pass
                    .get("target")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                textures,
                constants,
                combos,
            });
        }

        if passes.is_empty() {
            return None;
        }

        let fbos = def
            .get("fbos")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|f| {
                        Some((
                            f.get("name")?.as_str()?.to_string(),
                            f.get("scale").and_then(|s| s.as_i64()).unwrap_or(1) as i32,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();

        Some(Self {
            passes,
            fbos,
            named: HashMap::new(),
            dims: (0, 0),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.passes.is_empty()
    }

    fn ensure_targets(&mut self, gl: &glow::Context, w: i32, h: i32) -> Result<()> {
        if self.dims == (w, h) && self.named.contains_key("__step") {
            return Ok(());
        }
        self.dims = (w, h);
        // Resizing must release the previous targets; otherwise every size
        // change leaks a full set of textures/FBOs (GPU memory is system RAM).
        for (_, canvas) in self.named.drain() {
            canvas.destroy(gl);
        }
        let defs = self.fbos.clone();
        for (name, scale) in defs {
            let scale = scale.max(1);
            let cw = (w / scale).max(1);
            let ch = (h / scale).max(1);
            let canvas = Canvas::new(gl, cw, ch)?;
            self.named.insert(name, canvas);
        }
        // Passes can target buffers that are not declared in `fbos`.
        for pass in &self.passes {
            if let Some(name) = &pass.target {
                if !self.named.contains_key(name) {
                    let canvas = Canvas::new(gl, w, h)?;
                    self.named.insert(name.clone(), canvas);
                }
            }
        }
        self.named.insert("__step".to_string(), Canvas::new(gl, w, h)?);
        Ok(())
    }

    pub fn apply(
        &mut self,
        gl: &glow::Context,
        pkg: &PackageFs,
        cache: &mut TextureCache,
        input: &Canvas,
        output: &Canvas,
        ctx: &EffectContext,
    ) -> Result<()> {
        self.ensure_targets(gl, output.w, output.h)?;

        // Track the latest "previous" full-canvas result across passes so multi-pass
        // effects without named targets (e.g. `blurprecise`) chain properly instead
        // of reading from and writing to the same FBO simultaneously.
        let mut prev_source: (glow::Texture, u32, u32) =
            (input.tex, input.w as u32, input.h as u32);
        let untargeted_count = self.passes.iter().filter(|p| p.target.is_none()).count();
        let mut untargeted_seen = 0usize;

        for pass in &mut self.passes {
            let target_name = pass.target.clone();
            let use_step = if target_name.is_none() {
                untargeted_seen += 1;
                untargeted_seen < untargeted_count
            } else {
                false
            };
            let effective_target = if use_step {
                Some("__step")
            } else {
                target_name.as_deref()
            };
            let (tw, th) = match effective_target {
                Some(n) => self
                    .named
                    .get(n)
                    .map(|c| (c.w, c.h))
                    .unwrap_or((output.w, output.h)),
                None => (output.w, output.h),
            };
            let (fbo, target_tex) = match effective_target {
                Some(n) => (
                    self.named.get(n).map(|c| c.fbo),
                    self.named.get(n).map(|c| (c.tex, c.w as u32, c.h as u32)),
                ),
                None => (
                    Some(output.fbo),
                    Some((output.tex, output.w as u32, output.h as u32)),
                ),
            };
            let Some(fbo) = fbo else { continue };

            unsafe {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.viewport(0, 0, tw, th);
                gl.disable(glow::DEPTH_TEST);
                gl.clear_color(0.0, 0.0, 0.0, 0.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
                if pass.program.as_ref().map(|p| p.blending) == Some(BlendMode::Additive) {
                    gl.enable(glow::BLEND);
                    gl.blend_func(glow::SRC_ALPHA, glow::ONE);
                } else {
                    gl.disable(glow::BLEND);
                }
            }

            let Some(program) = pass.program.as_mut() else { continue };
            program.prog.bind(gl);

            // Resolve every sampler slot first so our texture-unit mapping is
            // stable and `g_TextureNResolution` matches the real texture.
            let mut textures: HashMap<usize, (glow::Texture, u32, u32)> = HashMap::new();
            for slot in 0..8usize {
                let bound = pass.binds.get(&slot).map(|s| s.as_str());
                let tex = match bound {
                    Some("previous") => Some(prev_source),
                    Some("input") => Some((input.tex, input.w as u32, input.h as u32)),
                    Some(name) => self
                        .named
                        .get(name)
                        .map(|c| (c.tex, c.w as u32, c.h as u32)),
                    None => {
                        if let Some(path) = pass.textures.get(slot).and_then(|t| t.as_deref()) {
                            if let Some(rest) = path.strip_prefix("_rt_imageLayerComposite_") {
                                let id_str = rest.split('_').next().unwrap_or("");
                                let parsed_id = id_str.parse::<i64>().ok();
                                let from_comp = parsed_id
                                    .and_then(|id| ctx.composites.get(&id))
                                    .or_else(|| {
                                        ctx.dependencies
                                            .get(slot)
                                            .and_then(|id| ctx.composites.get(id))
                                    })
                                    .map(|c| (c.tex, c.w as u32, c.h as u32));
                                if from_comp.is_some() {
                                    from_comp
                                } else if parsed_id.is_some() && parsed_id == ctx.self_id {
                                    Some((input.tex, input.w as u32, input.h as u32))
                                } else {
                                    Some(prev_source)
                                }
                            } else {
                                cache.get(gl, pkg, path)
                            }
                        } else if let Some(def_tex) = program
                            .uniforms
                            .iter()
                            .find(|u| u.slot == Some(slot))
                            .and_then(|u| u.default_tex.as_deref())
                        {
                            cache.get(gl, pkg, def_tex)
                        } else {
                            None
                        }
                    }
                };
                let tex = tex.or_else(|| {
                    if slot == 0 {
                        Some(prev_source)
                    } else {
                        cache.get(gl, pkg, "util/white")
                    }
                });
                if let Some((tex, w, h)) = tex {
                    unsafe {
                        gl.active_texture(glow::TEXTURE0 + slot as u32);
                        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                    }
                    textures.insert(slot, (tex, w, h));
                }
            }

            for spec in &program.uniforms {
                let loc_name = spec.name.clone();
                if let Some(slot) = spec.slot {
                    program
                        .prog
                        .set_i32(gl, &loc_name, slot as i32);
                    if let Some((_, w, h)) = textures.get(&slot) {
                        let res_name = format!("g_Texture{slot}Resolution");
                        if res_name != loc_name {
                            program.prog.set_vec4(
                                gl,
                                &res_name,
                                [*w as f32, *h as f32, *w as f32, *h as f32],
                            );
                        }
                    }
                    continue;
                }

                if let Some(rest) = spec
                    .name
                    .strip_prefix("g_Texture")
                    .and_then(|s| s.strip_suffix("Resolution"))
                {
                    if let Ok(slot) = rest.parse::<usize>() {
                        let (w, h) = textures
                            .get(&slot)
                            .map(|(_, w, h)| (*w as f32, *h as f32))
                            .unwrap_or((tw.max(1) as f32, th.max(1) as f32));
                        program.prog.set_vec4(gl, &loc_name, [w, h, w, h]);
                        continue;
                    }
                }

                if spec.name == "g_Time" {
                    program.prog.set_f32(gl, &loc_name, ctx.time);
                    continue;
                }
                if spec.name == "g_Frametime" {
                    program.prog.set_f32(gl, &loc_name, ctx.frametime);
                    continue;
                }
                if spec.name == "g_Daytime" {
                    program.prog.set_f32(gl, &loc_name, ctx.daytime);
                    continue;
                }
                if spec.name == "g_ViewUp" || spec.name == "g_OrientationUp" {
                    if matches!(spec.kind, UniformKind::Vec2) {
                        program.prog.set_vec2(gl, &loc_name, 0.0, 1.0);
                    } else {
                        program.prog.set_vec3(gl, &loc_name, 0.0, 1.0, 0.0);
                    }
                    continue;
                }
                if spec.name == "g_TextureReductionScale" {
                    program.prog.set_f32(gl, &loc_name, 1.0);
                    continue;
                }
                if spec.name == "g_PointerPosition" {
                    program
                        .prog
                        .set_vec2(gl, &loc_name, ctx.mouse[0], ctx.mouse[1]);
                    continue;
                }
                if spec.name == "g_PointerPositionLast" {
                    program
                        .prog
                        .set_vec2(gl, &loc_name, ctx.mouse[0], ctx.mouse[1]);
                    continue;
                }
                if spec.name == "g_ParallaxPosition" {
                    program
                        .prog
                        .set_vec2(gl, &loc_name, ctx.mouse[0], 1.0 - ctx.mouse[1]);
                    continue;
                }
                if spec.name == "g_TexelSize" {
                    program.prog.set_vec2(
                        gl,
                        &loc_name,
                        1.0 / (tw.max(1) as f32),
                        1.0 / (th.max(1) as f32),
                    );
                    continue;
                }
                if spec.name == "g_TexelSizeHalf" {
                    program.prog.set_vec2(
                        gl,
                        &loc_name,
                        0.5 / (tw.max(1) as f32),
                        0.5 / (th.max(1) as f32),
                    );
                    continue;
                }
                if spec.name == "g_EffectTextureProjectionMatrixInverse" {
                    set_mat4(gl, &mut program.prog, &loc_name, &ctx.projection_inverse);
                    continue;
                }
                if spec.name == "g_ModelViewProjectionMatrix"
                    || spec.name == "g_ModelMatrix"
                    || spec.name == "g_ViewProjectionMatrix"
                    || spec.name == "g_EffectTextureProjectionMatrix"
                {
                    set_mat4(gl, &mut program.prog, &loc_name, &identity4());
                    continue;
                }
                if spec.name == "g_Resolution" || spec.name == "g_ScreenResolution" {
                    program.prog.set_vec4(
                        gl,
                        &loc_name,
                        [tw as f32, th as f32, tw as f32, th as f32],
                    );
                    continue;
                }
                if spec.name.starts_with("g_AudioSpectrum") {
                    let n = match spec.kind {
                        UniformKind::FloatArray(len) => len.clamp(1, 64),
                        _ => 64,
                    };
                    let mut values = [0.0f32; 64];
                    if n == 16 {
                        for (i, out) in values.iter_mut().take(16).enumerate() {
                            let base = i * 4;
                            *out = (ctx.spectrum[base]
                                + ctx.spectrum[base + 1]
                                + ctx.spectrum[base + 2]
                                + ctx.spectrum[base + 3])
                                * 0.25;
                        }
                    } else if n == 32 {
                        for (i, out) in values.iter_mut().take(32).enumerate() {
                            let base = i * 2;
                            *out = (ctx.spectrum[base] + ctx.spectrum[base + 1]) * 0.5;
                        }
                    } else {
                        for (i, v) in values.iter_mut().enumerate() {
                            *v = ctx.spectrum.get(i).copied().unwrap_or(0.0);
                        }
                    }
                    set_f32_array(gl, &mut program.prog, &loc_name, &values[..n]);
                    continue;
                }

                let lookup_mat = |m: &str| -> Option<Value> {
                    let ml = m.to_ascii_lowercase();
                    pass.constants
                        .get(&ml)
                        .or_else(|| pass.constants.get(&format!("ui_editor_properties_{ml}")))
                        .cloned()
                };
                let value = spec
                    .material
                    .as_deref()
                    .and_then(lookup_mat)
                    .or_else(|| {
                        spec.alt_materials
                            .iter()
                            .find_map(|m| lookup_mat(m.as_str()))
                    })
                    .or_else(|| {
                        spec.label
                            .as_ref()
                            .and_then(|l| pass.constants.get(&l.to_ascii_lowercase()))
                            .cloned()
                    })
                    .or_else(|| {
                        let short = spec
                            .name
                            .strip_prefix("g_")
                            .or_else(|| spec.name.strip_prefix("u_"))
                            .unwrap_or(&spec.name)
                            .to_ascii_lowercase();
                        pass.constants.get(&short).cloned()
                    })
                    .or_else(|| spec.default.clone());
                if let Some(raw_value) = value {
                    let resolved = eval_dynamic_constant(&raw_value, ctx);
                    set_uniform_value(gl, &mut program.prog, spec, &resolved);
                }
            }

            unsafe {
                gl.bind_vertex_array(Some(program.vao));
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
                gl.bind_vertex_array(None);
            }
            if target_name.is_none() {
                if let Some(tt) = target_tex {
                    prev_source = tt;
                }
            }
        }

        unsafe {
            gl.disable(glow::BLEND);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        Ok(())
    }
}

/// Evaluate Wallpaper Engine dynamic shader constant objects:
/// - keyframe timeline animations (`"animation": { "c0": [...], "options": ... }`),
/// - `engine.timeOfDay` smoothStep blend scripts (`START_HOUR` / `END_HOUR`),
/// - `engine.registerAudioBuffers` audio-frequency response scripts.
fn eval_dynamic_constant(val: &Value, ctx: &EffectContext) -> Value {
    let Some(obj) = val.as_object() else {
        return val.clone();
    };
    if let Some(anim) = obj.get("animation").and_then(|a| a.as_object()) {
        let opts = anim.get("options").and_then(|o| o.as_object());
        let fps = opts
            .and_then(|o| o.get("fps"))
            .and_then(json_f32)
            .unwrap_or(30.0)
            .max(1.0);
        let length = opts
            .and_then(|o| o.get("length"))
            .and_then(json_f32)
            .unwrap_or(60.0)
            .max(1.0);
        let mode = opts
            .and_then(|o| o.get("mode"))
            .and_then(|m| m.as_str())
            .unwrap_or("loop");
        let raw_f = ctx.time * fps;
        let frame = match mode {
            "mirror" => {
                let period = (length * 2.0).max(1.0);
                let m = raw_f.rem_euclid(period);
                if m > length { period - m } else { m }
            }
            "single" => raw_f.clamp(0.0, length),
            _ => raw_f.rem_euclid(length),
        };
        let eval_curve = |key: &str| -> Option<f32> {
            let frames = anim.get(key)?.as_array()?;
            if frames.is_empty() {
                return None;
            }
            let mut pts: Vec<(f32, f32)> = frames
                .iter()
                .filter_map(|k| {
                    let f = k.get("frame").and_then(json_f32)?;
                    let v = k.get("value").and_then(json_f32)?;
                    Some((f, v))
                })
                .collect();
            if pts.is_empty() {
                return None;
            }
            pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            if frame <= pts[0].0 {
                return Some(pts[0].1);
            }
            for w in pts.windows(2) {
                let (f0, v0) = w[0];
                let (f1, v1) = w[1];
                if frame <= f1 {
                    let span = (f1 - f0).max(1e-5);
                    let t = ((frame - f0) / span).clamp(0.0, 1.0);
                    let s = t * t * (3.0 - 2.0 * t);
                    return Some(v0 + (v1 - v0) * s);
                }
            }
            pts.last().map(|&(_, v)| v)
        };
        if let Some(c0) = eval_curve("c0") {
            if let (Some(c1), Some(c2)) = (eval_curve("c1"), eval_curve("c2")) {
                return Value::String(format!("{c0} {c1} {c2}"));
            }
            if let Some(n) = serde_json::Number::from_f64(c0 as f64) {
                return Value::Number(n);
            }
        }
    }
    if let Some(script) = obj.get("script").and_then(|s| s.as_str()) {
        if script.contains("engine.timeOfDay") && script.contains("START_HOUR") {
            let parse_const = |name: &str| -> Option<f32> {
                let idx = script.find(name)?;
                let rest = &script[idx + name.len()..];
                let eq = rest.find('=')?;
                let after = rest[eq + 1..].trim_start();
                let end = after
                    .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
                    .unwrap_or(after.len());
                after[..end].parse::<f32>().ok()
            };
            if let (Some(start_h), Some(end_h)) =
                (parse_const("START_HOUR"), parse_const("END_HOUR"))
            {
                let blend_dur = parse_const("BLEND_DURATION").unwrap_or(0.05).max(0.001);
                let tod = ctx.daytime.clamp(0.0, 1.0);
                let smooth = |e0: f32, e1: f32, x: f32| -> f32 {
                    if (e1 - e0).abs() < 1e-6 {
                        return if x >= e1 { 1.0 } else { 0.0 };
                    }
                    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
                    t * t * (3.0 - 2.0 * t)
                };
                let v = smooth((start_h - blend_dur) / 24.0, start_h / 24.0, tod)
                    * smooth(end_h / 24.0, (end_h - blend_dur) / 24.0, tod);
                if let Some(n) = serde_json::Number::from_f64(v as f64) {
                    return Value::Number(n);
                }
            }
        }
        if script.contains("registerAudioBuffers") && script.contains("scriptProperties.frequency")
        {
            let base = obj.get("value").and_then(json_f32).unwrap_or(0.15);
            let props = obj.get("scriptproperties").and_then(|p| p.as_object());
            let freq = props
                .and_then(|p| p.get("frequency"))
                .and_then(json_f32)
                .unwrap_or(0.0)
                .clamp(0.0, 15.0) as usize;
            let min_val = props
                .and_then(|p| p.get("minvalue"))
                .and_then(json_f32)
                .unwrap_or(0.8);
            let max_val = props
                .and_then(|p| p.get("maxvalue"))
                .and_then(json_f32)
                .unwrap_or(1.2);
            let idx = freq * 4;
            let level = ((ctx.spectrum[idx]
                + ctx.spectrum[idx + 1]
                + ctx.spectrum[idx + 2]
                + ctx.spectrum[idx + 3])
                * 0.25)
                .clamp(0.0, 1.0);
            let v = base * (level * (max_val - min_val) + min_val);
            if let Some(n) = serde_json::Number::from_f64(v as f64) {
                return Value::Number(n);
            }
        }
    }
    val.clone()
}

fn set_uniform_value(
    gl: &glow::Context,
    prog: &mut Program,
    spec: &UniformSpec,
    value: &Value,
) {
    let name = &spec.name;
    match spec.kind {
        UniformKind::Float => {
            if let Some(v) = json_f32(value) {
                prog.set_f32(gl, name, v);
            }
        }
        UniformKind::Int => {
            if let Some(v) = json_f32(value) {
                prog.set_i32(gl, name, v as i32);
            }
        }
        UniformKind::Bool => {
            if let Some(v) = json_bool(value) {
                prog.set_i32(gl, name, if v { 1 } else { 0 });
            }
        }
        UniformKind::Vec2 => {
            let v = json_vec(value, 2);
            prog.set_vec2(gl, name, v[0], v[1]);
        }
        UniformKind::Vec3 => {
            let v = json_vec(value, 3);
            prog.set_vec3(gl, name, v[0], v[1], v[2]);
        }
        UniformKind::Vec4 => {
            let v = json_vec(value, 4);
            prog.set_vec4(gl, name, [v[0], v[1], v[2], v[3]]);
        }
        UniformKind::Mat3 | UniformKind::Mat4 => {}
        UniformKind::FloatArray(n) => {
            let vals = json_vec(value, n.max(1));
            set_f32_array(gl, prog, name, &vals[..n.min(vals.len())]);
        }
        UniformKind::Sampler | UniformKind::Other => {}
    }
}

fn set_mat4(gl: &glow::Context, prog: &mut Program, name: &str, m: &[f32; 16]) {
    if let Some(loc) = prog.uniform(gl, name) {
        unsafe { gl.uniform_matrix_4_f32_slice(Some(&loc), false, m) };
    }
}

fn set_f32_array(gl: &glow::Context, prog: &mut Program, name: &str, values: &[f32]) {
    if values.is_empty() {
        return;
    }
    if let Some(loc) = prog.uniform(gl, name) {
        unsafe { gl.uniform_1_f32_slice(Some(&loc), values) };
    }
}

fn identity4() -> [f32; 16] {
    [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0,
        1.0,
    ]
}

fn json_f32(v: &Value) -> Option<f32> {
    match v {
        Value::Number(n) => n.as_f64().map(|f| f as f32),
        Value::String(s) => s.trim().parse::<f32>().ok(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::Object(o) => o.get("value").and_then(json_f32),
        _ => None,
    }
}

fn json_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => Some(n.as_f64().unwrap_or(0.0) != 0.0),
        Value::String(s) => Some(s.eq_ignore_ascii_case("true") || s == "1"),
        Value::Object(o) => o.get("value").and_then(json_bool),
        _ => None,
    }
}

fn json_vec(v: &Value, n: usize) -> Vec<f32> {
    if let Value::Object(o) = v {
        if let Some(inner) = o.get("value") {
            return json_vec(inner, n);
        }
    }
    let mut out = vec![0.0; n];
    match v {
        Value::String(s) => {
            for (i, p) in s
                .split(|c: char| c.is_whitespace() || c == ',')
                .filter(|p| !p.is_empty())
                .take(n)
                .enumerate()
            {
                out[i] = p.parse().unwrap_or(0.0);
            }
        }
        Value::Array(a) => {
            for (i, item) in a.iter().take(n).enumerate() {
                out[i] = json_f32(item).unwrap_or(0.0);
            }
        }
        other => {
            if let Some(f) = json_f32(other) {
                out[0] = f;
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Content drawing helpers shared with the scene renderer
// ---------------------------------------------------------------------------

pub const CONTENT_VS: &str = r#"#version 300 es
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec2 a_uv;
uniform vec2 u_scale;
out vec2 v_uv;
void main() {
    gl_Position = vec4(a_pos.x * u_scale.x, a_pos.y * u_scale.y, 0.0, 1.0);
    v_uv = a_uv;
}
"#;

pub const CONTENT_FS: &str = r#"#version 300 es
precision highp float;
in vec2 v_uv;
uniform sampler2D u_tex;
uniform vec4 u_color;
out vec4 fragColor;
void main() {
    fragColor = texture(u_tex, v_uv) * u_color;
}
"#;

pub const PUPPET_VS: &str = r#"#version 300 es
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec2 a_uv;
uniform vec2 u_scale;
uniform vec2 u_offset;
out vec2 v_uv;
void main() {
    gl_Position = vec4((a_pos.x - u_offset.x) * u_scale.x, (a_pos.y - u_offset.y) * u_scale.y, 0.0, 1.0);
    v_uv = a_uv;
}
"#;

pub const PUPPET_FS: &str = r#"#version 300 es
precision highp float;
in vec2 v_uv;
uniform sampler2D u_tex;
uniform vec4 u_color;
out vec4 fragColor;
void main() {
    fragColor = texture(u_tex, v_uv) * u_color;
}
"#;

pub const COMPOSITE_VS: &str = r#"#version 300 es
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec2 a_uv;
uniform vec2 u_origin;
uniform vec2 u_axis_x;
uniform vec2 u_axis_y;
uniform vec2 u_ortho;
out vec2 v_uv;
void main() {
    vec2 world = u_origin + a_pos.x * u_axis_x + a_pos.y * u_axis_y;
    vec2 ndc = vec2(world.x / u_ortho.x * 2.0 - 1.0, world.y / u_ortho.y * 2.0 - 1.0);
    gl_Position = vec4(ndc, 0.0, 1.0);
    v_uv = a_uv;
}
"#;

pub const COMPOSITE_FS: &str = r#"#version 300 es
precision highp float;
in vec2 v_uv;
uniform sampler2D u_tex;
uniform vec4 u_color;
uniform float u_brightness;
out vec4 fragColor;
void main() {
    vec4 c = texture(u_tex, v_uv) * u_color;
    c.rgb *= u_brightness;
    fragColor = c;
}
"#;

pub fn create_quad(gl: &glow::Context) -> Result<(glow::VertexArray, glow::Buffer)> {
    // Unit quad. pos.y = +1 is the top row; a_uv.y = 0 is the top of the
    // source texture, matching decoded images (row 0 = top).
    const VERTS: [f32; 16] = [
        -1.0, 1.0, 0.0, 0.0,
         1.0, 1.0, 1.0, 0.0,
        -1.0, -1.0, 0.0, 1.0,
         1.0, -1.0, 1.0, 1.0,
    ];
    unsafe {
        let vao = gl.create_vertex_array().map_err(|e| anyhow!(e))?;
        let vbo = gl.create_buffer().map_err(|e| anyhow!(e))?;
        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck_cast(&VERTS), glow::STATIC_DRAW);
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 16, 0);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, 16, 8);
        gl.bind_vertex_array(None);
        Ok((vao, vbo))
    }
}

pub const COPY_FS: &str = r#"#version 300 es
precision highp float;
in vec2 v_uv;
uniform sampler2D u_tex;
out vec4 fragColor;
void main() {
    fragColor = texture(u_tex, v_uv);
}
"#;

pub const COPY_VS: &str = r#"#version 300 es
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec2 a_uv;
out vec2 v_uv;
void main() {
    v_uv = a_uv;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
"#;
