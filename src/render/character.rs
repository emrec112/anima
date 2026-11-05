//! Anima Engine 2D Desktop Character / Mascot / Companion Renderer.
//!
//! Renders an interactive 2D character anchored to any corner/edge of the screen:
//! * Built-in procedural **Reze** (*Chainsaw Man*) character (`anima char reze`):
//!   - Dark violet shoulder-length bob (no bun) with long parted bangs that hide the
//!     eyebrows, one strand falling between the eyes, and short physics-swaying
//!     sidelocks that curl inward at the jaw
//!   - Soft, slightly droopy emerald-green anime eyes that smoothly follow the mouse
//!     cursor and blink naturally, plus her trademark heavy blush
//!   - Signature black neck choker with the silver grenade pull-pin and ring
//!   - White sleeveless collared blouse with a small black bow and bare shoulders
//!   - Audio-reactive lip-sync, breathing, head tilt, and subtle Bomb Devil spark
//!     particles on music beats
//! * Additional procedural presets (`neko`, `cyber`, `spirit`)
//! * Custom character sprites (`.png`, `.gif`, `.webp`, `.tex`) with Live2D-style
//!   2.5D procedural breathing, mouse-look tilt, and audio bounce.

use std::time::Instant;

use anyhow::Result;
use glow::HasContext;
use serde::{Deserialize, Serialize};

use super::FrameInfo;
use super::gl::{Filter, Program, create_texture};
use super::image::ImageRenderer;
use crate::pkg::decode_tex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Corner {
    BottomRight,
    BottomLeft,
    TopRight,
    TopLeft,
    BottomCenter,
}

impl Default for Corner {
    fn default() -> Self {
        Corner::BottomRight
    }
}

impl Corner {
    pub fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "bottom-left" | "bl" | "left" | "sol-alt" | "sol" => Corner::BottomLeft,
            "top-right" | "tr" | "sag-ust" => Corner::TopRight,
            "top-left" | "tl" | "sol-ust" => Corner::TopLeft,
            "bottom-center" | "bc" | "center" | "orta" => Corner::BottomCenter,
            _ => Corner::BottomRight,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Corner::BottomRight => "bottom-right",
            Corner::BottomLeft => "bottom-left",
            Corner::TopRight => "top-right",
            Corner::TopLeft => "top-left",
            Corner::BottomCenter => "bottom-center",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Placement {
    /// Drawn on the wallpaper layer, behind desktop windows.
    Wallpaper,
    /// Drawn in front of all windows, anchored to the screen corner.
    ScreenFront,
    /// Drawn in front of windows, tucked inside the active window's corner.
    WindowInside,
    /// Drawn in front of windows, perched on the outside edge/corner of the active window.
    WindowOutside,
}

impl Default for Placement {
    fn default() -> Self {
        Placement::Wallpaper
    }
}

impl Placement {
    pub fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "screen-front" | "front" | "overlay" | "on-top" | "ontop" | "on" | "ön" | "onde" => {
                Placement::ScreenFront
            }
            "window-inside" | "inside" | "win-in" | "in-window" | "pencere-ici" | "ic" => {
                Placement::WindowInside
            }
            "window-outside" | "outside" | "win-out" | "out-window" | "on-window"
            | "pencere-disi" | "dis" | "ust" => Placement::WindowOutside,
            _ => Placement::Wallpaper,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Placement::Wallpaper => "wallpaper",
            Placement::ScreenFront => "screen-front",
            Placement::WindowInside => "window-inside",
            Placement::WindowOutside => "window-outside",
        }
    }

    pub fn is_front_overlay(self) -> bool {
        !matches!(self, Placement::Wallpaper)
    }

    pub fn needs_overlay_surface(self) -> bool {
        !matches!(self, Placement::Wallpaper)
    }
}

/// Active window geometry in logical compositor coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WindowRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Where each character's sprite is drawn. The front overlay draws them all
/// on its one surface ([`SharedSurface`]); window mode gives each its own
/// window (docs/character-windows.md).
pub trait ActorTarget {
    /// Called right before actor `actor` (0 = main, then the extras) draws
    /// its sprite, whose box is `rect` (x0, y0, x1, y1) in overlay pixels.
    /// Makes the surface to draw into current; false skips the draw.
    fn begin(&mut self, gl: &glow::Context, actor: usize, rect: [f32; 4]) -> bool;
    /// After the sprite is drawn.
    fn end(&mut self, gl: &glow::Context, actor: usize);
    /// Maps the overlay's clip space onto the target's: (scale x, scale y,
    /// offset x, offset y). A target wider than the output (all workspaces
    /// side by side) uses it to fit characters that are off screen.
    fn clip_xform(&self) -> [f32; 4] {
        [1.0, 1.0, 0.0, 0.0]
    }
}

/// Draw every actor on the surface that is already current.
pub struct SharedSurface;

impl ActorTarget for SharedSurface {
    fn begin(&mut self, _: &glow::Context, _: usize, _: [f32; 4]) -> bool {
        true
    }

    fn end(&mut self, _: &glow::Context, _: usize) {}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CharacterConfig {
    /// Enabled flag.
    pub enabled: bool,
    /// Path to a `.png`/`.gif`/`.webp` sprite, or a preset (`reze`, `neko`, `cyber`, `spirit`).
    pub source: String,
    /// Corner anchor (on screen or on the active window).
    pub corner: Corner,
    /// Where to place the character (`wallpaper`, `screen-front`, `window-inside`, `window-outside`).
    pub placement: Placement,
    /// Character size scale (`0.2 .. 4.0`, where `1.0` is ~360px tall).
    pub scale: f32,
    /// Pixel offset from the anchored corner `(x, y)`.
    pub offset: [f32; 2],
    /// Opacity (`0.0 .. 1.0`).
    pub opacity: f32,
    /// Mirror character horizontally.
    pub flip_x: bool,
    /// Track mouse cursor with head/eyes/tilt.
    pub look_at_mouse: bool,
    /// Bounce and lip-sync to desktop audio / music.
    pub audio_reactive: bool,
    /// Legacy alias: when `true` and `placement == Wallpaper`, upgrades to `ScreenFront`.
    pub always_on_top: bool,
    /// Frame rate cap used when the character animates over a *static*
    /// wallpaper. Animated wallpapers keep running at the wallpaper fps.
    pub fps: u32,
    /// Walk back and forth along the bottom of the screen.
    pub walk: bool,
    /// Optional second sprite shown while jumping (auto-detected for
    /// Uma Musume "[A]" sprites).
    pub source_happy: String,
    /// More sprites that roam alongside the main one.
    pub extra: Vec<String>,
    /// Give each walking character its own window where the compositor
    /// supports placing them (Hyprland with contrib/hyprland-plugin), so
    /// they move with the workspaces natively.
    pub windows: bool,
}

impl CharacterConfig {
    pub fn effective_placement(&self) -> Placement {
        if self.placement == Placement::Wallpaper && self.always_on_top {
            Placement::ScreenFront
        } else {
            self.placement
        }
    }
}

impl Default for CharacterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            source: "reze".into(),
            corner: Corner::BottomRight,
            placement: Placement::Wallpaper,
            scale: 1.0,
            offset: [24.0, 0.0],
            opacity: 1.0,
            flip_x: false,
            look_at_mouse: true,
            audio_reactive: true,
            always_on_top: false,
            fps: 30,
            walk: false,
            source_happy: String::new(),
            extra: Vec::new(),
            windows: true,
        }
    }
}

const CHAR_VS: &str = r#"#version 300 es
precision highp float;
layout(location = 0) in vec2 a_pos; // -1..1
out vec2 v_uv;

uniform vec2 u_center;    // clip space center
uniform vec2 u_half_size; // clip space half size
uniform vec2 u_tilt;      // Live2D-style mouse look skew/tilt
uniform float u_breath;   // breathing scale Y
uniform float u_bounce;   // audio bounce Y offset
uniform int u_flip_x;
uniform vec4 u_clip;      // target clip space: xy scale, zw offset

void main() {
    vec2 uv = a_pos * 0.5 + 0.5;
    if (u_flip_x == 1) {
        uv.x = 1.0 - uv.x;
    }
    v_uv = uv;

    // Anchor bottom of character (-1.0) so breathing & bounce happen from waist up
    float height_factor = (a_pos.y + 1.0) * 0.5;
    vec2 p = a_pos;
    p.y = -1.0 + (p.y + 1.0) * (1.0 + u_breath * 0.025);
    p.x += height_factor * u_tilt.x * 0.055;
    p.y += height_factor * u_tilt.y * 0.025;

    vec2 clip = u_center + p * u_half_size;
    clip.y += u_bounce;
    gl_Position = vec4(clip * u_clip.xy + u_clip.zw, 0.0, 1.0);
}
"#;

const CHAR_FS: &str = r#"#version 300 es
precision highp float;
in vec2 v_uv;

uniform sampler2D u_tex;
// 0 = custom texture/sprite, 1 = reze, 2 = neko, 3 = cyber, 4 = spirit
uniform int u_mode;
// u_tex is already premultiplied (3D model render target)
uniform int u_premul;
uniform float u_time;
uniform float u_opacity;
uniform vec2 u_look;  // normalized mouse direction from character (-1..1)
uniform vec4 u_audio; // bass, mid, high, level
uniform float u_beat;
uniform vec4 u_mouth; // x=open(0..1), y=form(-1..1: -1=O/U, 0=A, +1=I/E), z=sibilance(0..1), w=active(0..1)

out vec4 fragColor;

float sdCircle(vec2 p, float r) {
    return length(p) - r;
}

float sdEllipse(vec2 p, vec2 r) {
    return (length(p / r) - 1.0) * min(r.x, r.y);
}

float sdSegment(vec2 p, vec2 a, vec2 b) {
    vec2 pa = p - a, ba = b - a;
    float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

float sdRoundedBox(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

float sdTriangleIsosceles(vec2 p, vec2 q) {
    p.x = abs(p.x);
    vec2 a = p - q * clamp(dot(p, q) / dot(q, q), 0.0, 1.0);
    vec2 b = p - q * vec2(clamp(p.x / q.x, 0.0, 1.0), 1.0);
    float s = -sign(q.y);
    vec2 d = min(vec2(dot(a, a), s * (p.x * q.y - p.y * q.x)),
                  vec2(dot(b, b), s * (p.y - q.y)));
    return -sqrt(d.x) * sign(d.y);
}

void blendOver(inout vec4 dst, vec3 rgb, float alpha) {
    float a = clamp(alpha, 0.0, 1.0);
    dst.rgb = mix(dst.rgb, rgb, a);
    dst.a = max(dst.a, a);
}

// ---------------------------------------------------------------------------
// Shared procedural helpers (character primitives)
// ---------------------------------------------------------------------------
float smin(float a, float b, float k) {
    float h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}

// Tapered hair lock: distance to a segment whose radius shrinks from wa to wb,
// with a lateral "curve" offset towards the tip.
float sdStrand(vec2 p, vec2 a, vec2 b, float wa, float wb, float curve) {
    vec2 pa = p - a, ba = b - a;
    float h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    vec2 c = a + ba * h;
    c.x += curve * h * h;
    // wide for most of the lock, then a sharp point
    return length(p - c) - mix(wa, wb, smoothstep(0.25, 1.0, h));
}

void drawHairStrand(inout vec4 col, vec2 p, vec2 a, vec2 b,
                    float wa, float wb, float curve,
                    vec3 root, vec3 tip, vec3 edge, vec3 shine) {
    float h = clamp(dot(p - a, b - a) / max(dot(b - a, b - a), 1e-6), 0.0, 1.0);

    // dark under-layer so neighbouring locks read as separate strands
    float dEdge = sdStrand(p, a, b, wa * 1.13 + 0.004, wb + 0.005, curve);
    blendOver(col, edge,
              smoothstep(0.004, -0.005, dEdge) * smoothstep(0.10, 0.40, h) * 0.55);

    float d = sdStrand(p, a, b, wa, wb, curve);
    float m = smoothstep(0.0045, -0.0045, d);
    if (m > 0.002) {
        vec3 c = mix(root, tip, smoothstep(0.10, 0.95, h));
        float sh = exp(-abs(h - 0.26) * 7.0) * smoothstep(0.05, 0.22, h);
        c = mix(c, shine, sh * 0.40);
        blendOver(col, c, m);
    }
}

// ============================================================================
// REZE (Chainsaw Man — Bomb Girl Arc) procedural portrait
// Eggplant bob with a side-parted fringe, jade eyes, black choker,
// white collared blouse and a small black ribbon bow.
// ============================================================================
vec4 render_reze(vec2 uv) {
    // design space, then a 1.35x bust framing anchored to the quad bottom
    vec2 q = (uv * 2.0 - 1.0) * vec2(0.88, 1.0);
    vec2 p = q * 0.7692 - vec2(0.0, 0.1308);

    float t = u_time;
    float sway = sin(t * 1.6) * 0.009 + u_look.x * 0.032;
    float nod  = sin(t * 2.8) * 0.006 + u_beat * 0.016;
    vec2 hp = p - vec2(sway, nod);
    vec2 bp = p - vec2(sway * 0.32, 0.0);

    // --- palette (MAPPA Reze arc: eggplant bob, jade eyes, warm pale skin) ---
    vec3 ink        = vec3(0.055, 0.040, 0.085);
    vec3 hairDeep   = vec3(0.075, 0.055, 0.125);
    vec3 hairBase   = vec3(0.195, 0.145, 0.300);
    vec3 hairMid    = vec3(0.300, 0.225, 0.445);
    vec3 hairShine  = vec3(0.520, 0.430, 0.705);
    vec3 skin       = vec3(0.995, 0.925, 0.885);
    vec3 skinShade  = vec3(0.905, 0.765, 0.745);
    vec3 skinDeep   = vec3(0.780, 0.590, 0.605);
    vec3 eyeDark    = vec3(0.020, 0.165, 0.125);
    vec3 eyeMid     = vec3(0.070, 0.480, 0.300);
    vec3 eyeLight   = vec3(0.390, 0.930, 0.570);
    vec3 shirt      = vec3(0.965, 0.975, 0.995);
    vec3 shirtShade = vec3(0.800, 0.820, 0.890);
    vec3 ribbon     = vec3(0.060, 0.060, 0.090);
    vec3 silver     = vec3(0.860, 0.890, 0.960);

    vec4 col = vec4(0.0);

    // 0. Bomb Devil fuse sparks, driven by the beat / bass
    float spark = clamp(u_audio.x * 0.8 + u_beat * 1.2, 0.0, 1.0);
    if (spark > 0.02) {
        for (float i = 0.0; i < 7.0; i += 1.0) {
            float ang = i * 0.897 + t * (1.2 + i * 0.15);
            float rad = 0.55 + 0.10 * sin(t * 2.6 + i * 2.3);
            vec2 sp = hp - vec2(cos(ang) * rad * 0.9, sin(ang) * rad + 0.14);
            float glow = exp(-length(sp) * 26.0) * spark;
            vec3 sc = mix(vec3(1.0, 0.35, 0.10), vec3(1.0, 0.85, 0.40), fract(i * 0.41));
            col.rgb += sc * glow * 0.85;
            col.a = max(col.a, glow * 0.65);
        }
    }

    // ------------------------------------------------------------------
    // 1. Back hair — chin-length bob: straight sides, rounded crown.
    // ------------------------------------------------------------------
    vec2 bob = hp - vec2(0.0, 0.075);
    float d_bobBody  = sdRoundedBox(bob - vec2(0.0, 0.010), vec2(0.322, 0.330), 0.100);
    float d_bobCrown = sdEllipse(bob - vec2(0.0, 0.250), vec2(0.334, 0.230));
    float d_bob = min(d_bobBody, d_bobCrown);

    blendOver(col, ink,     smoothstep(0.008, -0.005, d_bob - 0.007));
    blendOver(col, hairDeep, smoothstep(0.006, -0.005, d_bob));
    if (d_bob < 0.014) {
        float ba = smoothstep(0.011, -0.006, d_bob);
        float lift = smoothstep(0.02, 0.44, bob.y);
        blendOver(col, hairBase, lift * 0.55 * ba);
        blendOver(col, hairMid, lift * lift * 0.30 * ba);

        float band = exp(-abs(bob.y - 0.315 + bob.x * bob.x * 0.9) * 20.0);
        blendOver(col, hairShine,
                  band * smoothstep(0.30, 0.13, abs(bob.x)) * 0.45 * ba);

        blendOver(col, hairDeep, smoothstep(0.16, 0.33, abs(bob.x)) * 0.45 * ba);
        blendOver(col, ink, smoothstep(-0.09, -0.25, bob.y) * 0.45 * ba);
    }

    // ------------------------------------------------------------------
    // 2. Neck (drawn before the blouse, which hides its base)
    // ------------------------------------------------------------------
    vec2 nq = bp - vec2(0.0, -0.312);
    float d_neck = sdRoundedBox(nq, vec2(0.112, 0.132), 0.044);
    if (d_neck < 0.012) {
        float na = smoothstep(0.008, -0.006, d_neck);
        float chin_sh = smoothstep(-0.085, 0.055, nq.y);
        vec3 nCol = mix(skin, skinShade, chin_sh * 0.90);
        nCol = mix(nCol, skinDeep, smoothstep(0.040, 0.092, abs(nq.x)) * 0.55);
        blendOver(col, nCol, na);
    }

    // ------------------------------------------------------------------
    // 3. White blouse — sloping shoulders, filled chest, collar, ribbon.
    // ------------------------------------------------------------------
    float d_torso = sdRoundedBox(bp - vec2(0.0, -0.900), vec2(0.378, 0.380), 0.110);
    float d_shL = sdStrand(bp, vec2(-0.080, -0.372), vec2(-0.348, -0.520), 0.098, 0.098, 0.0);
    float d_shR = sdStrand(bp, vec2( 0.080, -0.372), vec2( 0.348, -0.520), 0.098, 0.098, 0.0);
    float d_armL = sdStrand(bp, vec2(-0.348, -0.520), vec2(-0.376, -1.100), 0.108, 0.108, 0.0);
    float d_armR = sdStrand(bp, vec2( 0.348, -0.520), vec2( 0.376, -1.100), 0.108, 0.108, 0.0);
    // chest yoke keeps the neckline solid instead of leaving a hole
    float d_yoke = sdRoundedBox(bp - vec2(0.0, -0.590), vec2(0.250, 0.205), 0.070);
    float d_shirt = min(min(d_torso, d_yoke), min(min(d_shL, d_shR), min(d_armL, d_armR)));

    if (d_shirt < 0.012) {
        float sha = smoothstep(0.009, -0.006, d_shirt);
        float side_shade = smoothstep(0.14, 0.62, abs(bp.x));
        vec3 sCol = mix(shirt, shirtShade, side_shade * 0.60);
        sCol = mix(sCol, shirtShade, smoothstep(-0.34, -0.48, bp.y) * 0.22);
        sCol = mix(sCol, shirtShade, smoothstep(-0.70, -1.05, bp.y) * 0.45);
        blendOver(col, sCol, sha);

        // sleeve seams between torso and upper arms
        float seamL = sdStrand(bp, vec2(-0.300, -0.470), vec2(-0.360, -1.060), 0.0035, 0.0035, 0.0);
        float seamR = sdStrand(bp, vec2( 0.300, -0.470), vec2( 0.360, -1.060), 0.0035, 0.0035, 0.0);
        blendOver(col, shirtShade, smoothstep(0.006, 0.0, min(seamL, seamR)) * 0.55 * sha);

        // gentle chest folds
        float foldA = sdStrand(bp, vec2(-0.230, -0.650), vec2(-0.140, -0.900), 0.006, 0.006, 0.0);
        float foldB = sdStrand(bp, vec2( 0.230, -0.670), vec2( 0.150, -0.920), 0.006, 0.006, 0.0);
        blendOver(col, shirtShade, smoothstep(0.012, 0.0, min(foldA, foldB)) * 0.35 * sha);

        // ---- pointed shirt collar around the neck ----
        // soft shaded hollow around the neckline (so the white collar reads)
        float d_hollow = sdEllipse(bp - vec2(0.0, -0.470), vec2(0.210, 0.150));
        blendOver(col, shirtShade, smoothstep(0.085, -0.030, d_hollow) * 0.38 * sha);

        // skin V of the open collar
        float d_v = sdTriangleIsosceles(bp - vec2(0.0, -0.350), vec2(0.058, 0.082));
        blendOver(col, mix(skin, skinShade, 0.45), smoothstep(0.005, -0.004, d_v) * 0.96);

        // ---- pointed shirt collar (white flaps over the shaded hollow) ----
        float d_collarL = sdStrand(bp, vec2(-0.078, -0.374), vec2(-0.128, -0.498), 0.048, 0.006, -0.020);
        float d_collarR = sdStrand(bp, vec2( 0.078, -0.374), vec2( 0.128, -0.498), 0.048, 0.006,  0.020);
        float d_collar = min(d_collarL, d_collarR);
        // drop shadow under the flaps
        blendOver(col, shirtShade, smoothstep(0.014, -0.002, d_collar - 0.012) * 0.55 * sha);
        blendOver(col, vec3(0.995, 0.995, 1.000), smoothstep(0.005, -0.004, d_collar) * sha);
        // seam along the outer edge only
        blendOver(col, mix(shirtShade, ink, 0.12),
                  smoothstep(0.0038, 0.0, abs(d_collar) - 0.0032) * 0.45 * sha);

        // shirt placket down the chest
        float d_placket = sdStrand(bp, vec2(0.0, -0.470), vec2(0.0, -1.050), 0.0025, 0.0025, 0.0);
        blendOver(col, shirtShade, smoothstep(0.004, 0.0, d_placket) * 0.45 * sha);

        // ---- black ribbon bow ----
        vec2 rib = bp - vec2(0.0, -0.432);
        float d_loopL = sdEllipse(rib - vec2(-0.040, 0.006), vec2(0.036, 0.018));
        float d_loopR = sdEllipse(rib - vec2( 0.040, 0.006), vec2(0.036, 0.018));
        float d_knot  = sdRoundedBox(rib, vec2(0.016, 0.014), 0.005);
        float d_tailL = sdStrand(rib, vec2(-0.009, -0.010), vec2(-0.033, -0.110), 0.012, 0.003, -0.007);
        float d_tailR = sdStrand(rib, vec2( 0.009, -0.010), vec2( 0.029, -0.104), 0.011, 0.003,  0.006);
        float d_bow = min(min(d_loopL, d_loopR), min(d_knot, min(d_tailL, d_tailR)));
        blendOver(col, ribbon, smoothstep(0.005, -0.004, d_bow));
        blendOver(col, vec3(0.26, 0.26, 0.34),
                  smoothstep(0.004, 0.0, abs(d_knot) - 0.003) * 0.45
                  + smoothstep(0.004, 0.0, abs(d_loopL) - 0.006) * 0.25
                  + smoothstep(0.004, 0.0, abs(d_loopR) - 0.006) * 0.25);
    }

    // ------------------------------------------------------------------
    // 4. Signature black choker with the silver ring
    // ------------------------------------------------------------------
    vec2 chq = bp - vec2(0.0, -0.306);
    float d_choker = sdRoundedBox(chq, vec2(0.120, 0.0175), 0.0065);
    if (d_choker < 0.006) {
        blendOver(col, ribbon, smoothstep(0.005, -0.004, d_choker));
        float hi = sdRoundedBox(chq - vec2(0.0, 0.008), vec2(0.104, 0.0030), 0.0015);
        blendOver(col, vec3(0.22, 0.22, 0.30), smoothstep(0.003, 0.0, hi) * 0.75);
        float ring = abs(sdCircle(chq - vec2(0.0, -0.026), 0.0125)) - 0.0027;
        blendOver(col, silver, smoothstep(0.004, -0.002, ring));
        float pin = sdRoundedBox(chq - vec2(0.0, -0.026), vec2(0.0024, 0.0145), 0.002);
        blendOver(col, silver, smoothstep(0.003, 0.0, pin) * 0.9);
    }

    // ------------------------------------------------------------------
    // 5. Face — egg shaped with a soft, small chin
    // ------------------------------------------------------------------
    vec2 fp = hp - vec2(0.0, 0.075);
    float jaw_drop = u_mouth.x * 0.016;
    vec2 fq = fp;
    fq.y += jaw_drop * smoothstep(-0.06, -0.28, fp.y);

    float d_cran = sdEllipse(fq - vec2(0.0, 0.055), vec2(0.240, 0.270));
    float d_jaw  = sdEllipse(fq - vec2(0.0, -0.136), vec2(0.154, 0.162));
    float d_face = smin(d_cran, d_jaw, 0.062);

    if (d_face < 0.014) {
        float fa = smoothstep(0.011, -0.006, d_face);
        vec3 fCol = mix(skinShade, skin, smoothstep(-0.30, -0.05, fp.y));
        fCol = mix(fCol, skinShade, smoothstep(0.02, 0.24, fp.x) * 0.26);
        blendOver(col, fCol, fa);
        blendOver(col, skinDeep, smoothstep(0.005, 0.0, abs(d_face) - 0.0022) * 0.30);

        // fringe shadow on the forehead
        float fh = smoothstep(0.26, 0.10, fp.y) * smoothstep(0.00, 0.12, fp.y);
        col.rgb = mix(col.rgb, skinShade, fh * 0.32 * fa);

        // warm cheek blush
        vec2 bl = vec2(abs(fp.x) - 0.150, fp.y + 0.150);
        float blush = exp(-dot(bl * vec2(1.0, 2.0), bl * vec2(1.0, 2.0)) * 95.0);
        col.rgb = mix(col.rgb, vec3(0.975, 0.560, 0.610), blush * (0.30 + u_beat * 0.12) * fa);

        // small nose shadow
        vec2 np = fp - vec2(u_look.x * 0.008, -0.128);
        float d_nose = sdEllipse(np, vec2(0.0075, 0.0040));
        blendOver(col, skinDeep, smoothstep(0.005, 0.0, d_nose) * 0.55);

        // ---------------- 6. Emerald eyes ----------------
        float blink_cycle = mod(t, 4.6);
        float blink = smoothstep(0.0, 0.065, abs(blink_cycle - 4.18));
        vec2 look = clamp(u_look, vec2(-1.0), vec2(1.0)) * vec2(0.026, 0.018);
        float side = sign(fp.x);
        vec2 ep = vec2(abs(fp.x) - 0.126, fp.y + 0.030);
        ep.y += ep.x * 0.16;
        float eye_h = max(0.004, 0.060 * blink);

        float d_sclera = sdEllipse(ep, vec2(0.063, eye_h * 0.94));
        if (d_sclera < 0.008 && blink > 0.15) {
            float ea = smoothstep(0.008, -0.005, d_sclera);
            vec3 scleraCol = mix(vec3(0.99, 1.0, 1.0),
                                 vec3(0.78, 0.80, 0.88),
                                 smoothstep(-eye_h * 0.2, eye_h, ep.y));
            blendOver(col, scleraCol, ea);

            vec2 ip = ep - vec2(side * look.x, look.y + 0.013);
            float d_iris = sdEllipse(ip, vec2(0.045, 0.053));
            if (d_iris < 0.007) {
                float ia = smoothstep(0.006, -0.004, d_iris) * ea;
                float igrad = clamp((ip.y + 0.053) / 0.106, 0.0, 1.0);
                vec3 irisCol = mix(eyeLight, mix(eyeMid, eyeDark, igrad), igrad);
                blendOver(col, irisCol, ia);
                // lid shadow across the top of the iris
                blendOver(col, eyeDark,
                          smoothstep(0.020, 0.062, ip.y) * ia * 0.55);

                float rim = smoothstep(0.007, 0.0, abs(d_iris) - 0.004);
                blendOver(col, eyeDark, rim * ia * 0.90);

                float d_pupil = sdEllipse(ip, vec2(0.0160, 0.021));
                blendOver(col, vec3(0.02, 0.08, 0.06), smoothstep(0.004, -0.003, d_pupil) * ia);

                float hl1 = sdCircle(ip - vec2(-0.014 * side, 0.018), 0.0105);
                float hl2 = sdCircle(ip - vec2(0.012 * side, -0.016), 0.0052);
                blendOver(col, vec3(1.0), smoothstep(0.004, -0.003, min(hl1, hl2)) * ia);

                float iglow = exp(-length(ip - vec2(0.0, -0.040)) * 30.0);
                blendOver(col, eyeLight, iglow * ia * 0.35);
            }
        }

        // upper lid: thick curved lash hugging the top of the eye
        float lashMask = smoothstep(0.014, 0.0, abs(d_sclera))
                       * smoothstep(-0.014, 0.014, ep.y);
        blendOver(col, ink, lashMask);
        // outer wing
        vec2 wp = vec2(abs(ep.x), ep.y);
        float d_wing = sdStrand(wp, vec2(0.044, 0.012), vec2(0.088, 0.026), 0.0080, 0.002, 0.0);
        blendOver(col, ink, smoothstep(0.004, -0.003, d_wing));
        // eyelid crease
        float crease = abs(sdEllipse(ep - vec2(0.0, 0.008), vec2(0.060, 0.062))) - 0.0032;
        blendOver(col, skinDeep, smoothstep(0.005, 0.001, crease)
                  * smoothstep(0.006, 0.028, ep.y) * 0.30);

        // lower lash line
        float d_lower = sdSegment(vec2(abs(ep.x), ep.y),
                                  vec2(0.022, -eye_h * 0.88),
                                  vec2(0.060, -eye_h * 0.58)) - 0.0030;
        blendOver(col, hairDeep, smoothstep(0.003, 0.0, d_lower) * 0.45 * smoothstep(0.0, 0.4, blink));

        // delicate eyebrows
        vec2 br = ep - vec2(0.0, 0.098);
        br.y += (br.x + 0.004) * (br.x + 0.004) * 2.2;
        float d_brow = sdRoundedBox(br, vec2(0.046, 0.0038), 0.0024);
        blendOver(col, hairDeep, smoothstep(0.004, -0.003, d_brow) * 0.80);

        // ---------------- 7. Viseme mouth ----------------
        float m_open = clamp(u_mouth.x, 0.0, 1.0);
        float m_form = clamp(u_mouth.y, -1.0, 1.0);
        float m_sib  = clamp(u_mouth.z, 0.0, 1.0);
        float m_act  = clamp(u_mouth.w, 0.0, 1.0);

        vec2 mp = fp - vec2(look.x * 0.24, -0.180 - jaw_drop * 0.45);
        if (m_open > 0.035) {
            float mw = clamp(0.030 + m_form * 0.014 + m_open * 0.006, 0.015, 0.048);
            float mh = clamp(0.004 + m_open * 0.036 * (1.0 - max(m_form, 0.0) * 0.22), 0.005, 0.041);

            vec2 mq = mp;
            mq.y -= (mq.x * mq.x) * (m_form * 2.8);
            float roundness = clamp(0.009 + max(-m_form, 0.0) * 0.011 + m_open * 0.007,
                                    0.004, min(mw, mh));
            float d_mouth = mix(
                sdRoundedBox(mq, vec2(mw, mh), roundness),
                sdEllipse(mq, vec2(mw, mh)),
                clamp(-m_form * 0.85, 0.0, 0.85)
            );

            if (d_mouth < 0.005) {
                float ma = smoothstep(0.004, -0.003, d_mouth);
                vec3 mCol = vec3(0.36, 0.09, 0.15);

                vec2 tp = mq - vec2(m_form * 0.003, -mh * 0.55);
                float d_tongue = sdEllipse(tp, vec2(mw * 0.78, mh * 0.72));
                if (d_tongue < 0.0) {
                    vec3 tongueCol = mix(vec3(0.94, 0.44, 0.52),
                                         vec3(0.76, 0.26, 0.36),
                                         clamp(-tp.y * 18.0, 0.0, 1.0));
                    mCol = mix(mCol, tongueCol, smoothstep(0.003, -0.003, d_tongue));
                }

                float teeth_drop = mh * (0.52 - 0.28 * max(m_sib, max(m_form, 0.0)));
                float d_teeth = mq.y - teeth_drop;
                if (d_teeth > -0.005 && abs(mq.x) < mw * 0.86) {
                    float ta = smoothstep(-0.003, 0.002, d_teeth);
                    mCol = mix(mCol, vec3(0.98, 0.97, 0.99), ta);
                }

                blendOver(col, mCol, ma);
                blendOver(col, vec3(0.32, 0.14, 0.20),
                          smoothstep(0.0045, 0.0, abs(d_mouth) - 0.0018) * 0.85);
            }
            vec2 lip_hl = mp - vec2(0.0, -mh - 0.005);
            float d_lip_hl = sdSegment(lip_hl, vec2(-mw * 0.38, 0.0), vec2(mw * 0.38, 0.0));
            blendOver(col, skinShade * 0.88, smoothstep(0.005, 0.001, d_lip_hl) * 0.55);
        } else if (m_act > 0.22) {
            vec2 bpm = mp;
            float d_closed = sdSegment(bpm, vec2(-0.025, 0.0), vec2(0.025, 0.0));
            blendOver(col, vec3(0.40, 0.20, 0.26), smoothstep(0.0055, 0.001, d_closed));
            float d_phil = sdSegment(bpm - vec2(0.0, -0.006), vec2(-0.010, 0.0), vec2(0.010, 0.0));
            blendOver(col, skinShade * 0.86, smoothstep(0.0045, 0.001, d_phil) * 0.55);
        } else {
            // Reze's soft closed-mouth smile
            vec2 sp = mp;
            sp.y -= (sp.x * sp.x) * 4.0;
            float d_smile = sdSegment(sp, vec2(-0.036, 0.0), vec2(0.038, 0.005));
            blendOver(col, vec3(0.38, 0.17, 0.24), smoothstep(0.0060, 0.001, d_smile));
            float d_corner = sdCircle(sp - vec2(0.034, 0.004), 0.0023);
            blendOver(col, vec3(0.42, 0.22, 0.28), smoothstep(0.004, 0.0, d_corner) * 0.8);
        }
    }

    // ------------------------------------------------------------------
    // 8. Front hair — face-framing locks, fringe, root cap
    // ------------------------------------------------------------------
    for (float s = -1.0; s <= 1.0; s += 2.0) {
        float lock_sway = (sin(t * 2.2 + s * 0.9) * 0.008 + u_look.x * 0.018);
        // main jaw-length lock
        drawHairStrand(col, hp,
            vec2(s * 0.280, 0.330),
            vec2(s * 0.292 + lock_sway, -0.272),
            0.050, 0.011, s * 0.030,
            hairMid, hairDeep, ink, hairShine);
        // thinner inner lock hugging the cheek
        drawHairStrand(col, hp,
            vec2(s * 0.228, 0.352),
            vec2(s * 0.244 + lock_sway * 0.7, -0.195),
            0.034, 0.008, s * 0.012,
            hairBase, hairDeep, ink, hairShine);
    }

    // fringe base: solid bang mass under the individual locks
    float fsway = sin(t * 1.9) * 0.004 + u_look.x * 0.008;
    float frLine = hp.y - (0.152 + 0.026 * sin(hp.x * 7.0 + 2.4) + 0.04 * hp.x);
    float d_frBase = max(sdEllipse(hp - vec2(0.0, 0.290), vec2(0.300, 0.208)), -frLine);
    if (d_frBase < 0.012) {
        float fba = smoothstep(0.010, -0.006, d_frBase);
        vec3 fbCol = mix(hairBase, hairMid, smoothstep(0.16, 0.34, hp.y) * 0.7);
        fbCol = mix(fbCol, hairDeep, smoothstep(0.20, 0.15, hp.y) * 0.35);
        blendOver(col, fbCol, fba);
    }

    // side-parted fringe: every lock roots inside the crown, tapers to a point
    drawHairStrand(col, hp, vec2(-0.290 + fsway, 0.320), vec2(-0.322 + fsway, 0.022),
                   0.056, 0.007, -0.012, hairBase, hairDeep, ink, hairShine);
    drawHairStrand(col, hp, vec2(-0.238 + fsway, 0.352), vec2(-0.280 + fsway, 0.048),
                   0.058, 0.006, -0.016, hairBase, hairDeep, ink, hairShine);
    drawHairStrand(col, hp, vec2(-0.186 + fsway, 0.372), vec2(-0.244 + fsway, 0.020),
                   0.060, 0.005, -0.020, hairBase, hairDeep, ink, hairShine);
    drawHairStrand(col, hp, vec2(-0.128 + fsway, 0.388), vec2(-0.176 + fsway, 0.058),
                   0.062, 0.005, -0.012, hairMid, hairDeep, ink, hairShine);
    drawHairStrand(col, hp, vec2(-0.066 + fsway, 0.396), vec2(-0.098 + fsway, 0.096),
                   0.060, 0.005, -0.006, hairMid, hairDeep, ink, hairShine);
    drawHairStrand(col, hp, vec2( 0.000 + fsway, 0.398), vec2( 0.056 + fsway, 0.092),
                   0.062, 0.005,  0.014, hairMid, hairDeep, ink, hairShine);
    drawHairStrand(col, hp, vec2( 0.070 + fsway, 0.392), vec2( 0.146 + fsway, 0.090),
                   0.062, 0.006,  0.018, hairMid, hairDeep, ink, hairShine);
    drawHairStrand(col, hp, vec2( 0.140 + fsway, 0.382), vec2( 0.222 + fsway, 0.066),
                   0.062, 0.006,  0.020, hairMid, hairDeep, ink, hairShine);
    drawHairStrand(col, hp, vec2( 0.210 + fsway, 0.362), vec2( 0.292 + fsway, 0.038),
                   0.060, 0.007,  0.016, hairBase, hairDeep, ink, hairShine);
    drawHairStrand(col, hp, vec2( 0.272 + fsway, 0.330), vec2( 0.318 + fsway, 0.018),
                   0.056, 0.007,  0.012, hairBase, hairDeep, ink, hairShine);

    // root cap: covers the fringe roots and forms the hairline mass
    float d_capCrown = sdEllipse(hp - vec2(0.0, 0.250), vec2(0.322, 0.238));
    float capLine = hp.y - (0.322 + 0.016 * sin(hp.x * 13.0 + 1.0));
    float d_cap = max(d_capCrown, -capLine);
    if (d_cap < 0.012) {
        float ca = smoothstep(0.010, -0.006, d_cap);
        // identical shading to the back-hair crown so the seam disappears
        float lift = smoothstep(0.02, 0.44, bob.y);
        blendOver(col, hairBase, lift * 0.55 * ca);
        blendOver(col, hairMid, lift * lift * 0.30 * ca);
        float band = exp(-abs(bob.y - 0.315 + bob.x * bob.x * 0.9) * 20.0);
        blendOver(col, hairShine, band * smoothstep(0.30, 0.13, abs(bob.x)) * 0.35 * ca);
        blendOver(col, hairDeep, smoothstep(0.19, 0.32, abs(bob.x)) * 0.45 * ca);
        // soft shadow under the hairline onto the skin
        blendOver(col, ink, smoothstep(0.025, 0.0, abs(d_cap)) * 0.16 * ca);
    }

    // part line
    float d_part = sdStrand(hp, vec2(0.006, 0.520), vec2(-0.014, 0.340), 0.0020, 0.0032, 0.0);
    blendOver(col, ink, smoothstep(0.004, -0.001, d_part) * 0.40);

    col.a *= u_opacity;
    return col;
}


vec4 render_neko(vec2 uv, int style) {
    vec2 p = uv * 2.0 - 1.0;
    float sway = sin(u_time * 2.2) * 0.025 + u_look.x * 0.05;
    vec2 hp = p - vec2(sway, sin(u_time * 3.2) * 0.018 + u_beat * 0.035);

    vec3 primary = (style == 3) ? vec3(0.18, 0.85, 0.98)
                 : (style == 4) ? vec3(0.62, 0.42, 1.00)
                                : vec3(0.96, 0.56, 0.76);
    vec3 hairCol = (style == 3) ? vec3(0.10, 0.16, 0.28)
                 : (style == 4) ? vec3(0.18, 0.12, 0.32)
                                : vec3(0.18, 0.15, 0.24);
    vec3 skinCol = vec3(1.0, 0.93, 0.90);

    vec4 col = vec4(0.0);

    float aura_r = length(hp - vec2(0.0, -0.05));
    float aura = exp(-abs(aura_r - 0.58 - u_audio.x * 0.08) * 18.0) * (0.15 + u_audio.w * 0.75);
    col.rgb += primary * aura;
    col.a = max(col.a, aura * 0.65);

    float back_hair = sdRoundedBox(hp - vec2(0.0, 0.02), vec2(0.42, 0.46), 0.24);
    if (back_hair < 0.01) {
        col = mix(col, vec4(hairCol, 1.0), smoothstep(0.01, -0.01, back_hair));
    }

    vec2 ear_p = vec2(abs(hp.x) - 0.26, hp.y - 0.62);
    ear_p.x += ear_p.y * 0.18;
    float ear_outer = sdTriangleIsosceles(ear_p, vec2(0.18, -0.28));
    if (ear_outer < 0.01) {
        col = mix(col, vec4(hairCol, 1.0), smoothstep(0.01, -0.01, ear_outer));
        float ear_inner = sdTriangleIsosceles(ear_p - vec2(0.0, -0.04), vec2(0.10, -0.18));
        if (ear_inner < 0.01) {
            col = mix(col, vec4(primary * (0.85 + u_audio.x * 0.4), 1.0), smoothstep(0.01, -0.01, ear_inner));
        }
    }

    vec2 body_p = p - vec2(sway * 0.5, -0.68);
    float body = sdRoundedBox(body_p, vec2(0.34, 0.36), 0.16);
    if (body < 0.01) {
        vec3 outfit = mix(hairCol * 0.85, primary, 0.25);
        col = mix(col, vec4(outfit, 1.0), smoothstep(0.01, -0.01, body));
    }

    vec2 face_p = hp - vec2(0.0, 0.10);
    float head = sdRoundedBox(face_p, vec2(0.35, 0.31), 0.24);
    if (head < 0.01) {
        col = mix(col, vec4(skinCol, 1.0), smoothstep(0.01, -0.01, head));
        float blink_cycle = mod(u_time, 4.2);
        float blink = smoothstep(0.0, 0.07, abs(blink_cycle - 3.9));
        vec2 look_offset = clamp(u_look, vec2(-1.0), vec2(1.0)) * vec2(0.038, 0.028);
        vec2 ep = vec2(abs(face_p.x) - 0.145, face_p.y + 0.01);
        float eye_box = sdRoundedBox(ep, vec2(0.058, max(0.008, 0.075 * blink)), 0.028);
        if (eye_box < 0.008) {
            col.rgb = mix(col.rgb, vec3(0.12, 0.10, 0.18), smoothstep(0.008, -0.008, eye_box));
            if (blink > 0.25) {
                vec2 side_look = vec2(face_p.x > 0.0 ? look_offset.x : -look_offset.x, look_offset.y);
                float iris = sdCircle(ep - side_look, 0.042);
                if (iris < 0.006) {
                    col.rgb = mix(col.rgb, primary, smoothstep(0.006, -0.006, iris));
                }
            }
        }
        vec2 bang_p = face_p - vec2(0.0, 0.22);
        float bang = sdRoundedBox(bang_p, vec2(0.36, 0.11), 0.06) + sin(face_p.x * 18.0) * 0.035;
        if (bang < 0.01) {
            col.rgb = mix(col.rgb, hairCol, smoothstep(0.01, -0.01, bang));
        }
    }

    col.a *= u_opacity;
    return col;
}

void main() {
    vec4 c;
    if (u_mode == 0) {
        vec2 uv = vec2(v_uv.x, 1.0 - v_uv.y);
        c = texture(u_tex, clamp(uv, 0.0, 1.0));
        if (u_premul == 1 && c.a > 0.0) c.rgb /= c.a;
        c.a *= u_opacity;
    } else if (u_mode == 1) {
        c = render_reze(v_uv);
    } else {
        c = render_neko(v_uv, u_mode);
    }
    // Premultiplied alpha output so transparent Wayland overlay surfaces composite cleanly
    fragColor = vec4(c.rgb * c.a, c.a);
}
"#;

pub struct CharacterOverlay {
    pub config: CharacterConfig,
    program: Program,
    vao: glow::VertexArray,
    #[allow(dead_code)]
    vbo: glow::Buffer,
    dummy_tex: glow::Texture,
    sprite_renderer: Option<ImageRenderer>,
    sprite_tex: Option<(glow::Texture, u32, u32)>,
    // optional second pose shown while jumping
    sprite_happy: Option<glow::Texture>,
    // sprite artwork looks to the right (Uma Musume chibis do)
    faces_right: bool,
    // rigged 3D character drawn into `sprite_tex` every frame
    model: Option<super::model3d::Model3d>,
    // index of this character; extras are drawn after the main one
    actor: usize,
    extras: Vec<CharacterOverlay>,
    // drawn sprite box from the last frame, surface pixels (x0, y0, x1, y1)
    hit_rect: [f32; 4],
    // left button press: when, and which actor (0 = main) was under it
    press: Option<(f32, Option<usize>)>,
    // notifications seen so far
    notified: u32,
    // left clicks seen so far
    clicks: u32,
    // niri strip scroll, smoothed: (view offset, extra strip width) in clip units
    view: [f32; 2],
    smooth_look: [f32; 2],
    smooth_bounce: f32,
    smooth_pos: Option<[f32; 2]>,
    start: Instant,
    walk: WalkState,
    // re-pose at most every this many frames (crowd budget, set by actor 0)
    pose_every: u64,
}

/// What the character is doing right now.
#[derive(Default, Clone, Copy, PartialEq)]
enum Act {
    #[default]
    Idle,
    Walk,
    Jump,
    Chase,
    Pat,
    Held,
    Fall,
}

/// Random behaviour: walk (likely), idle, jump (less likely) and, rarely,
/// run after the cursor and jump once it's caught.
/// `x` is the offset in clip units from the anchored position.
#[derive(Default)]
struct WalkState {
    x: f32,
    target: f32,
    left: bool,
    seed: u32,
    act: Act,
    timer: f32,
    jump: f32,
    // current jump: peak height in half-heights of the character, and length
    jump_amp: f32,
    jump_dur: f32,
    // picked to answer a circle gesture
    circle: bool,
    // height above the ground while held/falling (half-heights), fall speed
    air: f32,
    vy: f32,
    // sideways speed after a throw (clip units/s)
    vx: f32,
    // last held position, for the throw speed
    held_prev: Option<(f32, f32)>,
    // where the cursor grabbed the sprite, relative to its centre (clip)
    grab: Option<[f32; 2]>,
    // seconds until a jump queued by a notification
    notify_jump: Option<f32>,
    // dropped at a random spot on the first frame
    placed: bool,
    // animation time not yet given to the 3D model (skipped frames)
    model_dt: f32,
}

const PAT_DUR: f32 = 1.4;
// each click while petting keeps it going at least this long
const PAT_KEEP: f32 = 0.6;

/// How often an on-screen 3D character is re-posed; it still moves every
/// frame.
const CONTENT_HZ: f32 = 40.0;
/// Most 3D re-poses per frame, all characters together.
const POSE_BUDGET: usize = 24;
const MIN_POSE_HZ: f32 = 12.0;

const JUMP_DUR: f32 = 0.9;
const JUMP_AMP: f32 = 2.0;

impl WalkState {
    fn rand(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    fn pick(&mut self, min: f32, max: f32, can_chase: bool) {
        let r = self.rand();
        if r < 0.27 {
            // short or long stroll in a random direction
            let d = self.rand();
            let dist = if d < 0.4 {
                0.05 + self.rand() * 0.25
            } else if d < 0.8 {
                0.3 + self.rand() * 0.9
            } else {
                // a trip somewhere along niri's strip
                self.rand() * (max - min)
            };
            let mut dir = if self.rand() < 0.5 { -1.0 } else { 1.0 };
            if (self.x + dir * dist).clamp(min, max) == self.x {
                dir = -dir;
            }
            self.target = (self.x + dir * dist).clamp(min, max);
            self.act = Act::Walk;
        } else if r < 0.82 {
            self.act = Act::Idle;
            self.timer = 1.0 + self.rand() * 3.0;
        } else if r < 0.98 || !can_chase {
            self.start_jump(JUMP_AMP);
        } else {
            self.act = Act::Chase;
            self.timer = 6.0; // give up if the cursor can't be reached
        }
    }

    fn start_pat(&mut self) {
        self.act = Act::Pat;
        self.timer = PAT_KEEP;
        self.jump = 0.0;
    }

    fn start_jump(&mut self, amp: f32) {
        self.act = Act::Jump;
        self.jump_amp = amp;
        // higher jumps hang in the air a bit longer
        self.jump_dur = JUMP_DUR * (amp / JUMP_AMP).sqrt().max(1.0);
        self.jump = self.jump_dur;
    }

    /// Advance one frame. `mouse` is the cursor x relative to the anchor.
    /// Returns (offset, moving, jump height 0..1).
    fn step(&mut self, dt: f32, min: f32, max: f32, mouse: Option<f32>, reach: f32) -> (f32, bool, f32) {
        if matches!(self.act, Act::Held | Act::Fall) {
            return (self.x, false, 0.0);
        }
        if let Some(t) = self.notify_jump.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                self.notify_jump = None;
                if !matches!(self.act, Act::Pat | Act::Jump) {
                    self.start_jump(JUMP_AMP);
                }
            }
        }
        let mut moving = false;
        // circle drawn with the pointer: drop everything and chase it
        if std::mem::take(&mut self.circle)
            && mouse.is_some()
            && self.act != Act::Jump
        {
            self.act = Act::Chase;
            self.timer = 6.0;
        }
        match self.act {
            Act::Idle => {
                self.timer -= dt;
                if self.timer <= 0.0 {
                    self.pick(min, max, mouse.is_some());
                }
            }
            Act::Walk | Act::Chase => {
                let chase = self.act == Act::Chase;
                if chase {
                    self.timer -= dt;
                    match mouse {
                        Some(m) => self.target = m.clamp(min, max),
                        None => self.timer = 0.0,
                    }
                }
                let speed = if chase { 0.4 } else { 0.12 };
                let d = self.target - self.x;
                // close enough counts as caught, or a cursor circling fast
                // keeps flipping the target left and right
                let catch = if chase { 0.05 } else { 0.0 };
                if d.abs() <= speed * dt + catch {
                    if !chase {
                        self.x = self.target;
                    }
                    if chase {
                        // caught it: jump up to the cursor, chase is over
                        self.start_jump(reach.max(JUMP_AMP));
                    } else {
                        self.pick(min, max, mouse.is_some());
                    }
                } else if chase && self.timer <= 0.0 {
                    self.pick(min, max, false);
                } else {
                    // only turn around when the target is clearly on the other side
                    if !chase || d.abs() > 0.1 {
                        self.left = d < 0.0;
                    }
                    self.x += d.signum() * speed * dt;
                    moving = true;
                }
            }
            Act::Jump | Act::Held | Act::Fall => {}
            Act::Pat => {
                self.timer -= dt;
                if self.timer <= 0.0 {
                    // happy little hop afterwards
                    self.start_jump(JUMP_AMP * 0.5);
                }
            }
        }
        let mut height = 0.0;
        if self.act == Act::Jump {
            self.jump -= dt;
            let t = 1.0 - (self.jump / self.jump_dur).max(0.0);
            height = 4.0 * t * (1.0 - t);
            if self.jump <= 0.0 {
                self.act = Act::Idle;
                self.timer = 0.5 + self.rand();
            }
        }
        (self.x, moving, height)
    }
}

impl CharacterOverlay {
    pub fn new(gl: &glow::Context, config: CharacterConfig, width: i32, height: i32) -> Result<Self> {
        Self::new_actor(gl, config, width, height, 0)
    }

    fn new_actor(gl: &glow::Context, config: CharacterConfig, width: i32, height: i32, actor: usize) -> Result<Self> {
        let program = Program::new(gl, CHAR_VS, CHAR_FS)?;
        let (vao, vbo) = create_quad(gl)?;
        let dummy_tex = create_texture(gl, 1, 1, &[0, 0, 0, 0], Filter::Nearest, false)?;

        let mut this = Self {
            config: config.clone(),
            program,
            vao,
            vbo,
            dummy_tex,
            sprite_renderer: None,
            sprite_tex: None,
            sprite_happy: None,
            faces_right: false,
            model: None,
            actor,
            extras: Vec::new(),
            hit_rect: [0.0; 4],
            press: None,
            notified: crate::backend::notify::NOTIFICATIONS.load(std::sync::atomic::Ordering::Relaxed),
            clicks: 0,
            view: [0.0; 2],
            smooth_look: [0.0, 0.0],
            smooth_bounce: 0.0,
            smooth_pos: None,
            start: Instant::now(),
            pose_every: 1,
            walk: WalkState {
                seed: (std::process::id() + actor as u32 * 7919).wrapping_mul(2654435761),
                ..Default::default()
            },
        };
        this.reload_source(gl, width, height)?;
        Ok(this)
    }

    pub fn reload_source(&mut self, gl: &glow::Context, _width: i32, _height: i32) -> Result<()> {
        self.sprite_tex = None;
        self.sprite_happy = None;
        self.sprite_renderer = None;
        self.faces_right = false;
        self.model = None;

        if self.actor == 0 {
            self.extras.clear();
            for src in &self.config.extra {
                let mut cfg = self.config.clone();
                cfg.source = src.clone();
                cfg.source_happy = String::new();
                cfg.extra = Vec::new();
                // numbered by what loaded: windows and hit boxes go by actor
                match Self::new_actor(gl, cfg, _width, _height, self.extras.len() + 1) {
                    Ok(e) => self.extras.push(e),
                    Err(err) => log::warn!("character {src}: {err:#}"),
                }
            }
        }

        let src = self.config.source.trim();
        if matches!(
            src.to_ascii_lowercase().as_str(),
            "" | "default" | "reze" | "neko" | "cyber" | "spirit"
        ) {
            return Ok(());
        }

        let path = crate::util::expand_path(src, std::env::current_dir().ok().as_deref());
        if path.is_file() && super::model3d::is_model_path(&path) {
            // render at ~1.5x the on-screen height for smooth edges
            let px = (540.0 * self.config.scale.clamp(0.2, 4.0)).clamp(256.0, 1024.0) as u32;
            let model = super::model3d::Model3d::load(gl, &path, px)?;
            self.sprite_tex = Some(model.texture());
            // drawn turned to the right, mirrored when walking left
            self.faces_right = true;
            self.model = Some(model);
            return Ok(());
        }
        if path.is_file() {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let img = if ext == "tex" {
                let bytes = std::fs::read(&path)?;
                decode_tex(&bytes)?.image
            } else {
                image::ImageReader::open(&path)?
                    .with_guessed_format()?
                    .decode()?
                    .to_rgba8()
            };
            let (w, h) = img.dimensions();
            let tex = create_texture(gl, w, h, img.as_raw(), Filter::Linear, true)?;
            self.sprite_tex = Some((tex, w, h));

            // happy pose: explicit `source_happy`, else the "[B]"/_0011 sibling
            // of an Uma Musume "[A]"/_0010 sprite
            let happy = if !self.config.source_happy.trim().is_empty() {
                Some(crate::util::expand_path(self.config.source_happy.trim(), None))
            } else {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                self.faces_right = name.contains("_0010.") || name.contains("_0011.");
                name.contains("_0010.")
                    .then(|| path.with_file_name(name.replace("[A]", "[B]").replace("_0010.", "_0011.")))
            };
            if let Some(hp) = happy.filter(|p| p.is_file()) {
                if let Ok(img) = image::ImageReader::open(&hp).and_then(|r| r.with_guessed_format()) {
                    if let Ok(img) = img.decode() {
                        let img = img.to_rgba8();
                        let (hw, hh) = img.dimensions();
                        self.sprite_happy =
                            create_texture(gl, hw, hh, img.as_raw(), Filter::Linear, true).ok();
                    }
                }
            }
        }
        Ok(())
    }

    /// Boxes of all drawn characters (surface pixels), for the input region.
    pub fn hit_rects(&self) -> Vec<[f32; 4]> {
        let mut v = vec![self.hit_rect];
        v.extend(self.extras.iter().map(|e| e.hit_rect));
        v
    }

    pub fn uses_audio(&self) -> bool {
        self.config.enabled && self.config.audio_reactive
    }

    pub fn target_bounds(
        &self,
        out_w: i32,
        out_h: i32,
        scale_factor: f32,
        active_window: Option<WindowRect>,
    ) -> (f32, f32, f32, f32) {
        let base_h = (360.0 * self.config.scale.clamp(0.2, 4.0) * scale_factor).max(64.0);
        let aspect = if let Some((_, tw, th)) = self.sprite_tex {
            (tw.max(1) as f32) / (th.max(1) as f32)
        } else {
            0.88
        };
        let base_w = (base_h * aspect).max(64.0);
        let ox = self.config.offset[0] * scale_factor;
        let oy = self.config.offset[1] * scale_factor;
        let sw = out_w as f32;
        let sh = out_h as f32;

        let placement = self.config.effective_placement();

        if matches!(placement, Placement::WindowInside | Placement::WindowOutside) {
            if let Some(win) = active_window {
                let wx = win.x * scale_factor;
                let wy = win.y * scale_factor;
                let ww = win.w * scale_factor;
                let wh = win.h * scale_factor;
                if ww >= 80.0 && wh >= 80.0 {
                    let (mut x, mut y) = match placement {
                        Placement::WindowInside => match self.config.corner {
                            Corner::BottomRight => (wx + ww - base_w - ox, wy + wh - base_h - oy),
                            Corner::BottomLeft => (wx + ox, wy + wh - base_h - oy),
                            Corner::TopRight => (wx + ww - base_w - ox, wy + oy),
                            Corner::TopLeft => (wx + ox, wy + oy),
                            Corner::BottomCenter => (wx + (ww - base_w) * 0.5 + ox, wy + wh - base_h - oy),
                        },
                        Placement::WindowOutside => {
                            // Perch on the outside edge/corner of the focused window:
                            // - TopLeft / TopRight: sitting on top of the window's top bar
                            // - BottomLeft / BottomRight: peeking out from the left/right outer border
                            //   near the bottom corner (or sitting on the top bar if flush with screen edge)
                            let top_sit_y = wy - base_h * 0.82 - oy;
                            match self.config.corner {
                                Corner::TopRight => {
                                    let y = if top_sit_y >= -base_h * 0.25 {
                                        top_sit_y
                                    } else {
                                        wy + oy
                                    };
                                    (wx + ww - base_w - ox, y)
                                }
                                Corner::TopLeft => {
                                    let y = if top_sit_y >= -base_h * 0.25 {
                                        top_sit_y
                                    } else {
                                        wy + oy
                                    };
                                    (wx + ox, y)
                                }
                                Corner::BottomRight => {
                                    let right_outside_x = wx + ww - base_w * 0.22 + ox;
                                    if right_outside_x + base_w <= sw + base_w * 0.25 {
                                        (right_outside_x, wy + wh - base_h - oy)
                                    } else if top_sit_y >= -base_h * 0.25 {
                                        (wx + ww - base_w - ox, top_sit_y)
                                    } else {
                                        (wx + ww - base_w - ox, wy + wh - base_h - oy)
                                    }
                                }
                                Corner::BottomLeft => {
                                    let left_outside_x = wx - base_w * 0.78 - ox;
                                    if left_outside_x >= -base_w * 0.25 {
                                        (left_outside_x, wy + wh - base_h - oy)
                                    } else if top_sit_y >= -base_h * 0.25 {
                                        (wx + ox, top_sit_y)
                                    } else {
                                        (wx + ox, wy + wh - base_h - oy)
                                    }
                                }
                                Corner::BottomCenter => {
                                    let y = if top_sit_y >= -base_h * 0.25 {
                                        top_sit_y
                                    } else {
                                        wy + wh - base_h - oy
                                    };
                                    (wx + (ww - base_w) * 0.5 + ox, y)
                                }
                            }
                        }
                        _ => unreachable!(),
                    };
                    x = x.clamp(-base_w * 0.35, sw - base_w * 0.65);
                    y = y.clamp(-base_h * 0.35, sh - base_h * 0.65);
                    return (x, y, base_w, base_h);
                }
            }
        }

        let (x, y) = match self.config.corner {
            Corner::BottomRight => (sw - base_w - ox, sh - base_h - oy),
            Corner::BottomLeft => (ox, sh - base_h - oy),
            Corner::TopRight => (sw - base_w - ox, oy),
            Corner::TopLeft => (ox, oy),
            Corner::BottomCenter => ((sw - base_w) * 0.5 + ox, sh - base_h - oy),
        };
        (x, y, base_w, base_h)
    }

    pub fn render(
        &mut self,
        gl: &glow::Context,
        info: &FrameInfo,
        active_window: Option<WindowRect>,
        target: &mut dyn ActorTarget,
    ) -> Result<()> {
        if !self.config.enabled || self.config.opacity <= 0.001 {
            return Ok(());
        }

        // a notification arrived: everyone hops, one after another
        if self.actor == 0 {
            let n = crate::backend::notify::NOTIFICATIONS.load(std::sync::atomic::Ordering::Relaxed);
            if n != self.notified {
                self.notified = n;
                self.walk.notify_jump = Some(0.0);
                for (i, e) in self.extras.iter_mut().enumerate() {
                    e.walk.notify_jump = Some(0.12 * (i + 1) as f32);
                }
            }
        }

        // circle gesture: the main character hands it to a random team member
        if self.actor == 0
            && crate::backend::pointer::CIRCLE_GESTURE.swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            let pick = (self.walk.rand() * (self.extras.len() + 1) as f32) as usize;
            match pick.checked_sub(1).and_then(|i| self.extras.get_mut(i)) {
                Some(e) => e.walk.circle = true,
                None => self.walk.circle = true,
            }
        }

        // press and hold on a character: pick it up
        if self.actor == 0 {
            let down = crate::backend::pointer::BUTTON_DOWN.load(std::sync::atomic::Ordering::Relaxed);
            let now = self.start.elapsed().as_secs_f32();
            if !down {
                self.press = None;
            } else if self.press.is_none() {
                let [mx, my] = info.mouse;
                let hit = |r: &[f32; 4]| mx >= r[0] && mx <= r[2] && my >= r[1] && my <= r[3];
                let target = (1..=self.extras.len())
                    .rev()
                    .find(|&i| hit(&self.extras[i - 1].hit_rect))
                    .or_else(|| hit(&self.hit_rect).then_some(0));
                self.press = Some((now, target));
            } else if let Some((t0, Some(i))) = self.press {
                if now - t0 > 0.15 {
                    let a = if i == 0 { &mut *self } else { &mut self.extras[i - 1] };
                    if a.walk.act != Act::Held {
                        // caught mid-jump: keep it at the height it is drawn at
                        if a.walk.act == Act::Jump && a.walk.jump_dur > 0.0 {
                            let t = 1.0 - (a.walk.jump / a.walk.jump_dur).clamp(0.0, 1.0);
                            a.walk.air += 4.0 * t * (1.0 - t) * a.walk.jump_amp;
                            a.walk.jump = 0.0;
                        }
                        a.walk.act = Act::Held;
                        a.walk.grab = None;
                        a.walk.held_prev = None;
                        a.walk.vx = 0.0;
                        a.walk.vy = 0.0;
                    }
                    self.press = Some((t0, None));
                }
            }
        }

        if self.actor == 0 {
            let held = self.walk.act == Act::Held || self.extras.iter().any(|e| e.walk.act == Act::Held);
            crate::backend::pointer::DRAGGING.store(held, std::sync::atomic::Ordering::Relaxed);
        }

        // clicking on a character that is being petted keeps the petting going
        if self.actor == 0 {
            let n = crate::backend::pointer::CLICKS.load(std::sync::atomic::Ordering::Relaxed);
            if n != self.clicks {
                self.clicks = n;
                let [mx, my] = info.mouse;
                let hit = |r: &[f32; 4]| mx >= r[0] && mx <= r[2] && my >= r[1] && my <= r[3];
                let keep = |a: &mut CharacterOverlay| {
                    if a.walk.act == Act::Pat && hit(&a.hit_rect) {
                        a.walk.timer = a.walk.timer.max(PAT_KEEP);
                    }
                };
                keep(self);
                self.extras.iter_mut().for_each(keep);
            }
        }

        // double click on a character: pet it (topmost one wins)
        if self.actor == 0
            && crate::backend::pointer::DOUBLE_CLICK.swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            let [mx, my] = info.mouse;
            let hit = |r: &[f32; 4]| mx >= r[0] && mx <= r[2] && my >= r[1] && my <= r[3];
            log::debug!(
                "pat: mouse ({mx:.0},{my:.0}) main {:?} extras {:?}",
                self.hit_rect.map(|v| v.round()),
                self.extras.iter().map(|e| e.hit_rect.map(|v| v.round())).collect::<Vec<_>>()
            );
            if let Some(e) = self.extras.iter_mut().rev().find(|e| hit(&e.hit_rect)) {
                e.walk.start_pat();
            } else if hit(&self.hit_rect) {
                self.walk.start_pat();
            }
        }

        let w = info.width.max(1) as f32;
        let h = info.height.max(1) as f32;
        let dt = (info.delta as f32).clamp(0.001, 0.1);
        // strip: Hyprland's row is replayed exactly; niri's scroll is
        // approximated like niri's own animation
        if self.actor == 0 {
            let hypr = crate::backend::governor::hypr_view();
            let strip = hypr.or(*crate::backend::governor::STRIP.lock().unwrap());
            // logical px -> clip units (assumes output scale 1)
            let to_clip = 2.0 / w;
            let target = strip.map_or([0.0, 0.0], |st| {
                [st.view_x * to_clip, ((st.width - w) * to_clip).max(0.0)]
            });
            let exact = strip.is_some_and(|st| st.exact);
            let k = if exact { 1.0 } else { (dt * 20.0).min(1.0) };
            self.view[0] += (target[0] - self.view[0]) * k;
            self.view[1] = target[1];
            let view = self.view;
            for e in &mut self.extras {
                e.view = view;
            }
        }
        let (tx, ty, pw, ph) = self.target_bounds(info.width, info.height, 1.0, active_window);

        let pos = match &mut self.smooth_pos {
            Some(p) => {
                let pos_lerp = (dt * 14.0).clamp(0.08, 1.0);
                p[0] += (tx - p[0]) * pos_lerp;
                p[1] += (ty - p[1]) * pos_lerp;
                *p
            }
            None => {
                self.smooth_pos = Some([tx, ty]);
                [tx, ty]
            }
        };

        let cx_px = pos[0] + pw * 0.5;
        let cy_px = h - (pos[1] + ph * 0.5);
        let center_clip = [(cx_px / w) * 2.0 - 1.0, (cy_px / h) * 2.0 - 1.0];
        let half_clip = [pw / w, ph / h];

        let target_look = if self.config.look_at_mouse {
            let dx = ((info.mouse[0] - cx_px) / (w * 0.45)).clamp(-1.0, 1.0);
            let dy = (((h - info.mouse[1]) - cy_px) / (h * 0.45)).clamp(-1.0, 1.0);
            [dx, dy]
        } else {
            [0.0, 0.0]
        };

        let lerp = (dt * 10.0).clamp(0.05, 1.0);
        self.smooth_look[0] += (target_look[0] - self.smooth_look[0]) * lerp;
        self.smooth_look[1] += (target_look[1] - self.smooth_look[1]) * lerp;

        let time = self.start.elapsed().as_secs_f32();
        let breath = (time * 2.4).sin();

        let target_bounce = if self.config.audio_reactive {
            (info.audio[0] * 0.035 + info.beat * 0.045) * half_clip[1]
        } else {
            0.0
        };
        self.smooth_bounce += (target_bounce - self.smooth_bounce) * (dt * 18.0).clamp(0.1, 1.0);

        let mode = if self.sprite_tex.is_some() {
            0
        } else {
            match self.config.source.to_ascii_lowercase().as_str() {
                "neko" => 2,
                "cyber" => 3,
                "spirit" => 4,
                _ => 1, // "reze" or "default"
            }
        };

        let audio = if self.config.audio_reactive {
            info.audio
        } else {
            [0.0; 4]
        };
        let beat = if self.config.audio_reactive {
            info.beat
        } else {
            0.0
        };
        let mouth = if self.config.audio_reactive {
            info.mouth
        } else {
            [0.0; 4]
        };

        // A 3D pose is rendered into the model's own texture, which the
        // sprite pass reuses: re-pose at ~40 Hz (staggered across actors),
        // ~10 Hz off screen, while the sprite still moves every frame.
        // With a crowd on screen, at most POSE_BUDGET re-poses per frame
        // (the GPU's limit), but never slower than MIN_POSE_HZ.
        let offscreen = self.hit_rect[2] < 0.0 || self.hit_rect[0] > w;
        if self.actor == 0 {
            let on_screen = std::iter::once(self.hit_rect)
                .chain(self.extras.iter().map(|e| e.hit_rect))
                .filter(|r| r[2] >= 0.0 && r[0] <= w)
                .count();
            let crowd = on_screen.div_ceil(POSE_BUDGET).max(1) as u64;
            let slowest = ((1.0 / MIN_POSE_HZ) / dt).round().max(1.0) as u64;
            self.pose_every = crowd.min(slowest);
            for e in &mut self.extras {
                e.pose_every = self.pose_every;
            }
        }
        let hz = if offscreen { 10.0 } else { CONTENT_HZ };
        let every = (((1.0 / hz) / dt).round().clamp(1.0, 16.0) as u64).max(if offscreen { 1 } else { self.pose_every });
        let redraw = (info.frame + self.actor as u64) % every == 0;
        self.walk.model_dt += dt;
        if let (Some(model), true) = (&mut self.model, redraw) {
            use super::model3d::Pose;
            let pose = match self.walk.act {
                Act::Walk => Pose::Walk,
                Act::Chase => Pose::Run,
                Act::Pat => Pose::Pat,
                Act::Held => Pose::Held,
                Act::Fall => Pose::Fall,
                Act::Idle | Act::Jump => Pose::Idle,
            };
            // light around where the character was drawn last frame
            let [x0, y0, x1, y1] = self.hit_rect;
            let light = crate::backend::envlight::light_at(
                (info.width.max(1) as u32, info.height.max(1) as u32),
                (x0 + x1) * 0.5,
                (y0 + y1) * 0.5,
            )
            .unwrap_or_default();
            let mirrored = self.config.flip_x ^ (self.config.walk && self.walk.left == self.faces_right);
            model.draw(gl, std::mem::take(&mut self.walk.model_dt), pose, light, mirrored);
        }

        unsafe {
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);

            self.program.bind(gl);
            self.program
                .set_vec2(gl, "u_center", center_clip[0], center_clip[1]);
            self.program
                .set_vec2(gl, "u_half_size", half_clip[0], half_clip[1]);
            // walking: positions live on niri's scrolling strip (offset from
            // the anchor when the strip isn't scrolled); `view` converts to screen
            let [view, extra] = self.view;
            let (walk_x, moving, jump) = if self.config.walk {
                let left_room = -(center_clip[0] - half_clip[0] + 1.0);
                let right_room = 1.0 - (center_clip[0] + half_clip[0]) + extra;
                if !self.walk.placed {
                    // everyone at once, anywhere along the strip
                    self.walk.placed = true;
                    self.walk.x = left_room + self.walk.rand() * (right_room - left_room);
                    self.walk.target = self.walk.x;
                    self.walk.timer = self.walk.rand() * 1.5;
                }
                let mouse = (self.config.look_at_mouse && info.mouse != [0.0, 0.0])
                    .then(|| (info.mouse[0] / w) * 2.0 - 1.0 - center_clip[0] + view);
                // jump height (in half-heights) needed for the head to touch the cursor
                let cursor_y = 1.0 - (info.mouse[1] / h) * 2.0;
                let reach = (cursor_y - (center_clip[1] + half_clip[1])) / half_clip[1] + 0.3;
                self.walk.step(dt, left_room, right_room, mouse, reach)
            } else {
                (0.0, false, 0.0)
            };
            // held: follow the cursor; released: fall back down
            let walk_x = if self.config.walk || self.walk.act == Act::Held {
                let mx = (info.mouse[0] / w) * 2.0 - 1.0;
                let my = 1.0 - (info.mouse[1] / h) * 2.0;
                let down = crate::backend::pointer::BUTTON_DOWN.load(std::sync::atomic::Ordering::Relaxed);
                match self.walk.act {
                    Act::Held if down => {
                        let grab = *self.walk.grab.get_or_insert([
                            mx - (center_clip[0] + self.walk.x - view),
                            my - (center_clip[1] + self.walk.air * half_clip[1]),
                        ]);
                        let left_room = -(center_clip[0] - half_clip[0] + 1.0) + view;
                        let right_room = 1.0 - (center_clip[0] + half_clip[0]) + view;
                        self.walk.x = (mx - grab[0] - center_clip[0] + view).clamp(left_room, right_room);
                        self.walk.target = self.walk.x;
                        self.walk.air = ((my - grab[1] - center_clip[1]) / half_clip[1]).max(0.0);
                        // throw speed: smoothed motion while held
                        if let Some((px, pa)) = self.walk.held_prev {
                            let k = (dt * 12.0).min(1.0);
                            self.walk.vx += ((self.walk.x - px) / dt - self.walk.vx) * k;
                            self.walk.vy += ((self.walk.air - pa) / dt - self.walk.vy) * k;
                        }
                        self.walk.held_prev = Some((self.walk.x, self.walk.air));
                    }
                    Act::Held => {
                        self.walk.act = Act::Fall;
                        self.walk.held_prev = None;
                    }
                    Act::Fall => {
                        let left_room = -(center_clip[0] - half_clip[0] + 1.0);
                        let right_room = 1.0 - (center_clip[0] + half_clip[0]) + extra;
                        self.walk.vy -= 40.0 * dt;
                        self.walk.air += self.walk.vy * dt;
                        self.walk.x += self.walk.vx * dt;
                        // bounce off the ends of the strip
                        if self.walk.x < left_room || self.walk.x > right_room {
                            self.walk.x = self.walk.x.clamp(left_room, right_room);
                            self.walk.vx = -self.walk.vx * 0.6;
                        }
                        // and the top
                        let top = (1.0 - center_clip[1]) / half_clip[1] - 1.0;
                        if self.walk.air > top {
                            self.walk.air = top;
                            self.walk.vy = -self.walk.vy.abs() * 0.5;
                        }
                        self.walk.target = self.walk.x;
                        if self.walk.air <= 0.0 {
                            self.walk.air = 0.0;
                            self.walk.vx = 0.0;
                            self.walk.vy = 0.0;
                            // land with a little bounce
                            self.walk.start_jump(JUMP_AMP * 0.3);
                        }
                    }
                    _ => {}
                }
                self.walk.x - view
            } else {
                walk_x
            };
            let air = self.walk.air * half_clip[1];
            self.program
                .set_vec2(gl, "u_center", center_clip[0] + walk_x, center_clip[1]);
            {
                // sprite box for click hit-testing (surface pixels)
                let lift_now = (jump * self.walk.jump_amp) * half_clip[1] + air;
                self.hit_rect = [
                    (center_clip[0] + walk_x - half_clip[0] + 1.0) * 0.5 * w,
                    (1.0 - (center_clip[1] + half_clip[1] + lift_now)) * 0.5 * h,
                    (center_clip[0] + walk_x + half_clip[0] + 1.0) * 0.5 * w,
                    (1.0 - (center_clip[1] - half_clip[1] + lift_now)) * 0.5 * h,
                ];
                // petting stops as soon as the cursor leaves the character
                let [mx, my] = info.mouse;
                let r = self.hit_rect;
                const M: f32 = 12.0;
                if self.walk.act == Act::Pat
                    && !(mx >= r[0] - M && mx <= r[2] + M && my >= r[1] - M && my <= r[3] + M)
                {
                    self.walk.start_jump(JUMP_AMP * 0.5);
                }
            }
            // sprites have no rig: idle sway + hop, quicker hops while walking
            let (sway, hop, squash) = if mode == 0 && self.walk.act == Act::Pat {
                // pat pat: quick downward squashes, no sway
                let p = (self.walk.timer / PAT_DUR * std::f32::consts::PI * 5.0).sin().abs();
                (0.0, 0.0, p * 5.0)
            } else if mode == 0 && self.model.is_some() {
                // the rig animates itself: only lift for jumps and drags
                (0.0, jump * self.walk.jump_amp * half_clip[1] + air, 0.0)
            } else if mode == 0 {
                let (freq, amp) = if moving { (7.0, 0.10) } else { (3.0, 0.05) };
                let hop = (time * freq).sin().abs();
                let (hop, amp) = if matches!(self.walk.act, Act::Held | Act::Fall) { (0.0, 0.0) } else { (hop, amp) };
                let lift = (hop * amp * (1.0 - jump) + jump * self.walk.jump_amp) * half_clip[1] + air;
                ((time * 1.5).sin() * 0.6, lift, (1.0 - hop) * 1.5 * (1.0 - jump))
            } else {
                (0.0, 0.0, 0.0)
            };
            self.program.set_vec2(
                gl,
                "u_tilt",
                self.smooth_look[0] + sway,
                self.smooth_look[1],
            );
            self.program.set_f32(gl, "u_breath", breath - squash);
            self.program.set_f32(gl, "u_bounce", self.smooth_bounce + hop);
            self.program
                .set_i32(gl, "u_flip_x", i32::from(self.config.flip_x ^ (self.config.walk && self.walk.left == self.faces_right)));
            self.program.set_i32(gl, "u_mode", mode);
            self.program.set_i32(gl, "u_premul", i32::from(self.model.is_some()));
            self.program.set_f32(gl, "u_time", time);
            self.program.set_f32(gl, "u_opacity", self.config.opacity);
            self.program
                .set_vec2(gl, "u_look", self.smooth_look[0], self.smooth_look[1]);
            self.program.set_vec4(gl, "u_audio", audio);
            self.program.set_f32(gl, "u_beat", beat);
            self.program.set_vec4(gl, "u_mouth", mouth);

            gl.active_texture(glow::TEXTURE0);
            let tex = match (self.walk.act, self.sprite_happy) {
                (Act::Jump | Act::Pat | Act::Held | Act::Fall, Some(h)) => h,
                _ => self.sprite_tex.map(|(t, _, _)| t).unwrap_or(self.dummy_tex),
            };
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            self.program.set_i32(gl, "u_tex", 0);

            if target.begin(gl, self.actor, self.hit_rect) {
                let [sx, sy, ox, oy] = target.clip_xform();
                self.program.set_vec4(gl, "u_clip", [sx, sy, ox, oy]);
                gl.bind_vertex_array(Some(self.vao));
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
                gl.bind_vertex_array(None);
                target.end(gl, self.actor);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.disable(glow::BLEND);
        }

        for e in &mut self.extras {
            e.render(gl, info, active_window, target)?;
        }
        Ok(())
    }
}

fn create_quad(gl: &glow::Context) -> Result<(glow::VertexArray, glow::Buffer)> {
    const VERTS: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
    unsafe {
        let vao = gl.create_vertex_array().map_err(|e| anyhow::anyhow!(e))?;
        let vbo = gl.create_buffer().map_err(|e| anyhow::anyhow!(e))?;
        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.buffer_data_u8_slice(
            glow::ARRAY_BUFFER,
            super::gl::bytemuck_cast(&VERTS),
            glow::STATIC_DRAW,
        );
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
        gl.bind_vertex_array(None);
        Ok((vao, vbo))
    }
}
