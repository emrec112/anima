//! The rope in a tug of war: a braided strip sagging a little between the
//! two ends of the line.

use super::{ActorTarget, CharacterOverlay};
use super::SceneZoom;
use crate::render::FrameInfo;
use crate::render::gl::{Program, bytemuck_cast};
use anyhow::{Result, anyhow};
use glow::HasContext;

const SEGMENTS: usize = 32;

const VS: &str = r#"#version 300 es
precision highp float;
layout(location = 0) in vec2 a_pos; // x: 0..1 along the rope, y: -1..1 across
out vec2 v_pos;
uniform vec2 u_a;
uniform vec2 u_b;
uniform float u_sag;
uniform float u_half;
uniform vec4 u_clip;
void main() {
    float t = a_pos.x;
    vec2 p = mix(u_a, u_b, t);
    p.y -= u_sag * 4.0 * t * (1.0 - t);
    p.y += a_pos.y * u_half;
    v_pos = a_pos;
    gl_Position = vec4(p * u_clip.xy + u_clip.zw, 0.0, 1.0);
}
"#;

const FS: &str = r#"#version 300 es
precision highp float;
in vec2 v_pos;
out vec4 fragColor;
uniform float u_twists;
void main() {
    // slanted strands, rounded off towards the edges
    float s = fract(v_pos.x * u_twists + v_pos.y * 0.35);
    float strand = smoothstep(0.0, 0.2, s) * (1.0 - smoothstep(0.7, 1.0, s));
    vec3 col = mix(vec3(0.42, 0.27, 0.13), vec3(0.82, 0.64, 0.38), strand);
    col *= 1.0 - 0.45 * v_pos.y * v_pos.y;
    float alpha = 1.0 - smoothstep(0.75, 1.0, abs(v_pos.y));
    fragColor = vec4(col * alpha, alpha);
}
"#;

pub(super) struct RopeGl {
    program: Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
}

impl RopeGl {
    fn new(gl: &glow::Context) -> Result<Self> {
        let program = Program::new(gl, VS, FS)?;
        let mut verts = Vec::with_capacity((SEGMENTS + 1) * 4);
        for k in 0..=SEGMENTS {
            let t = k as f32 / SEGMENTS as f32;
            verts.extend_from_slice(&[t, -1.0, t, 1.0]);
        }
        unsafe {
            let vao = gl.create_vertex_array().map_err(|e| anyhow!(e))?;
            let vbo = gl.create_buffer().map_err(|e| anyhow!(e))?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck_cast(&verts), glow::STATIC_DRAW);
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            gl.bind_vertex_array(None);
            Ok(Self { program, vao, vbo })
        }
    }

    pub(super) fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_vertex_array(self.vao);
            gl.delete_buffer(self.vbo);
        }
        self.program.destroy(gl);
    }
}

impl CharacterOverlay {
    /// `behind`: the park's ropes (drawn before the characters), else the tug rope.
    pub(super) fn draw_ropes(&mut self, gl: &glow::Context, info: &FrameInfo, target: &mut dyn ActorTarget, behind: bool) {
        // (ends, where, half thickness, sagging, twisted)
        let ropes: Vec<([f32; 4], Option<SceneZoom>, f32, bool, bool)> = if behind {
            // the park's swing ropes hang straight from the bar; a seesaw board is one thick plain band
            self.park_ropes().into_iter().filter_map(|(r, o, half, twisted)| self.placed(o).map(|z| (r, z, half, false, twisted))).collect()
        } else {
            self.ropes_owned().into_iter().filter_map(|(r, o)| self.placed(o).map(|z| (r, z, 5.0, true, true))).collect()
        };
        if ropes.is_empty() {
            return;
        }
        if self.rope.is_none() {
            match RopeGl::new(gl) {
                Ok(r) => self.rope = Some(r),
                Err(e) => {
                    log::warn!("rope: {e:#}");
                    return;
                }
            }
        }
        let time = self.start.elapsed().as_secs_f32();
        let (w, h) = (info.width.max(1) as f32, info.height.max(1) as f32);
        let Some(r) = self.rope.as_mut() else { return };
        for ([x0, y0, x1, y1], zoom, half, sagging, twisted) in ropes {
            if !target.begin(gl, 0, [x0.min(x1) - 20.0, y0.min(y1) - 40.0, x0.max(x1) + 20.0, y0.max(y1) + 40.0]) {
                continue;
            }
            let clip = |x: f32, y: f32| [x / w * 2.0 - 1.0, 1.0 - y / h * 2.0];
            let (a, b) = (clip(x0, y0), clip(x1, y1));
            r.program.bind(gl);
            r.program.set_vec2(gl, "u_a", a[0], a[1]);
            r.program.set_vec2(gl, "u_b", b[0], b[1]);
            // pulled nearly tight, trembling a little
            let sag = if sagging { 12.0 + (time * 9.0).sin() * 3.0 } else { 0.0 };
            r.program.set_f32(gl, "u_sag", sag / h * 2.0);
            r.program.set_f32(gl, "u_half", half / h * 2.0);
            let len = if sagging { (x1 - x0).abs() } else { ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt() };
            r.program.set_f32(gl, "u_twists", if twisted { len / 9.0 } else { 0.0 });
            r.program.set_vec4(gl, "u_clip", zoom.map_or_else(|| target.clip_xform(), |z| z.clip_xform()));
            unsafe {
                gl.enable(glow::BLEND);
                gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
                gl.bind_vertex_array(Some(r.vao));
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, ((SEGMENTS + 1) * 2) as i32);
                gl.bind_vertex_array(None);
                gl.disable(glow::BLEND);
            }
            target.end(gl, 0);
        }
    }
}

const CARROT_VS: &str = r#"#version 300 es
precision highp float;
layout(location = 0) in vec2 a_pos;
out vec2 v_pos;
out vec2 v_sclip;
uniform vec4 u_rect; // x0, y0, x1, y1 in clip space
uniform vec4 u_clip;
// worn on a character: a box in its sprite space, moved exactly as CHAR_VS
// moves the sprite (breathing, leaning, turning, hopping)
uniform int u_attach;
uniform vec4 u_sub;
uniform float u_spin;
uniform float u_turn; // a worn thing turned about its middle
uniform vec2 c_center;
uniform vec2 c_half;
uniform vec2 c_tilt;
uniform float c_breath;
uniform float c_bounce;
uniform float c_rot;
uniform float c_sink;
uniform float c_aspect;
uniform vec2 c_pivot;
uniform float c_xscale;
void main() {
    v_pos = a_pos;
    if (u_attach == 1) {
        vec2 p = mix(u_sub.xy, u_sub.zw, a_pos * 0.5 + 0.5);
        if (u_turn != 0.0) {
            // turned in screen proportions, about the middle of its box
            vec2 m = (u_sub.xy + u_sub.zw) * 0.5;
            vec2 k = c_half * vec2(c_aspect, 1.0);
            vec2 v = (p - m) * k;
            float c = cos(u_turn), s = sin(u_turn);
            p = m + vec2(c * v.x - s * v.y, s * v.x + c * v.y) / k;
        }
        float hf = (p.y + 1.0) * 0.5;
        p.y = -1.0 + (p.y + 1.0) * (1.0 + c_breath * 0.025);
        p.x += hf * c_tilt.x * 0.055;
        p.y += hf * c_tilt.y * 0.025;
        p.x *= c_xscale;
        vec2 d = p * c_half;
        d.x *= c_aspect;
        float c = cos(c_rot), s = sin(c_rot);
        vec2 piv = vec2(c_pivot.x * c_aspect, c_pivot.y);
        d -= piv;
        d = vec2(c * d.x - s * d.y, s * d.x + c * d.y);
        d += piv;
        d.x /= c_aspect;
        vec2 clip = c_center + d;
        clip.y += c_bounce - c_sink;
        v_sclip = clip;
        gl_Position = vec4(clip * u_clip.xy + u_clip.zw, 0.0, 1.0);
        return;
    }
    vec2 p = mix(u_rect.xy, u_rect.zw, a_pos * 0.5 + 0.5);
    // turned about its middle (something dropped, tumbling)
    if (u_spin != 0.0) {
        vec2 mid = (u_rect.xy + u_rect.zw) * 0.5;
        vec2 d = p - mid;
        d.x *= c_aspect;
        float c = cos(u_spin), s = sin(u_spin);
        d = vec2(c * d.x - s * d.y, s * d.x + c * d.y);
        d.x /= c_aspect;
        p = mid + d;
    }
    v_sclip = p;
    gl_Position = vec4(p * u_clip.xy + u_clip.zw, 0.0, 1.0);
}
"#;

// y up: 0 a carrot, 1 a wrapped box, 2 a balloon, 3 a boxing glove on a spring,
// 4 a sleeping Z, 5 a drop of sweat, 6 a crumb of carrot, 7 the arrow over "it",
// 8 a blindfold, 9 a bush, 10 a barrel, 11 a crate, 12 a music note,
// 13 a fishing rod, 14 a line and float, 15 a star, 16 a rubber duck, 17 an old boot, 18 a tin can,
// 20 confetti, 21 a beast out of the ground, 22 an umbrella, 23 a snowflake,
// 24 snow settled on a head, 25 a Santa hat (flipped: tip to the right), 26 a sleep
// mask, 27 party hat, 28 top hat, 29 bow, 30 glasses, 31 sunglasses, 32 flower,
// 33 scarf, 34 backpack, 35 wings, 36 headphones, 37 crown, 100 a picture (u_img)
const CARROT_FS: &str = r#"#version 300 es
precision highp float;
in vec2 v_pos;
in vec2 v_sclip;
uniform vec2 u_hide;
out vec4 fragColor;
uniform int u_kind;
uniform int u_flip;
uniform vec4 u_rect;
uniform sampler2D u_img;
uniform vec2 u_fit;      // a picture kept to its own shape inside the box
uniform vec4 u_crop;     // the part of the picture used: x, y, width, height (0..1)
uniform float u_reveal;  // shown from the bottom up to this share (rising out of the ground)
uniform float u_time;
uniform float u_saspect; // screen width / height
uniform float u_opacity;
float hash1(float x) { return fract(sin(x * 127.1) * 43758.5453); }
float blade(vec2 p, float x, float lean) {
    float t = clamp((p.y - 0.25) / 0.7, 0.0, 1.0);
    float d = abs(p.x - x - lean * t) - 0.17 * (1.0 - t);
    return (1.0 - smoothstep(-0.02, 0.03, d)) * step(0.25, p.y) * (1.0 - smoothstep(0.9, 0.95, p.y));
}
float disc(vec2 p, vec2 c, float r) {
    return 1.0 - smoothstep(r - 0.03, r + 0.01, length(p - c));
}
float sdBox(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
float bar(vec2 p, vec2 lo, vec2 hi) {
    return step(lo.x, p.x) * step(p.x, hi.x) * step(lo.y, p.y) * step(p.y, hi.y);
}
vec4 carrot(vec2 p) {
    float t = clamp((p.y + 1.0) / 1.45, 0.0, 1.0);
    float half_w = 0.1 + 0.62 * pow(t, 0.75);
    float d = abs(p.x) - half_w;
    float body = (1.0 - smoothstep(-0.04, 0.02, d)) * step(p.y, 0.45) * step(-1.0, p.y);
    float groove = smoothstep(0.82, 1.0, sin(p.y * 15.0 + p.x * 1.6));
    vec3 orange = mix(vec3(1.0, 0.55, 0.08), vec3(0.8, 0.3, 0.02), groove * 0.7);
    orange *= 1.0 - 0.35 * smoothstep(0.1, 1.0, p.x / max(half_w, 0.01));
    float leaf = max(blade(p, 0.0, 0.0), max(blade(p, -0.14, -0.35), blade(p, 0.14, 0.35)));
    return vec4(body > 0.0 ? orange : vec3(0.2, 0.68, 0.18), max(body, leaf));
}
vec4 box(vec2 p) {
    float body = bar(p, vec2(-0.78, -1.0), vec2(0.78, 0.3));
    float lid = bar(p, vec2(-0.92, 0.3), vec2(0.92, 0.56));
    float ribbon = step(abs(p.x), 0.16);
    float bow = max(disc(p, vec2(-0.27, 0.74), 0.22), disc(p, vec2(0.27, 0.74), 0.22));
    float a = max(max(body, lid), bow);
    vec3 col = mix(lid > 0.0 ? vec3(0.9, 0.2, 0.26) : vec3(0.72, 0.13, 0.2), vec3(1.0, 0.82, 0.25), max(ribbon, bow));
    return vec4(col, a);
}
vec4 balloon(vec2 p) {
    float d = length((p - vec2(0.0, 0.25)) / vec2(0.62, 0.72));
    float body = 1.0 - smoothstep(0.96, 1.0, d);
    float knot = bar(p, vec2(-0.07, -0.56), vec2(0.07, -0.44));
    float string_ = step(abs(p.x - 0.06 * sin(p.y * 9.0)), 0.025) * step(p.y, -0.5);
    float shine = disc(p, vec2(-0.22, 0.5), 0.13);
    vec3 col = mix(vec3(0.95, 0.2, 0.4), vec3(1.0, 0.85, 0.9), shine);
    if (body <= 0.0) col = string_ > 0.0 ? vec3(0.85) : vec3(0.75, 0.15, 0.3);
    return vec4(col, max(max(body, knot), string_));
}
vec4 glove(vec2 p) {
    float fist = max(disc(p, vec2(0.58, 0.0), 0.42), disc(p, vec2(0.36, 0.34), 0.18));
    float cuff = bar(p, vec2(0.04, -0.34), vec2(0.2, 0.34));
    float spring = step(p.x, 0.06) * step(abs(p.y - 0.28 * sin(p.x * 22.0)), 0.09);
    vec3 col = cuff > 0.0 ? vec3(0.95) : (fist > 0.0 ? vec3(0.9, 0.12, 0.12) : vec3(0.75, 0.76, 0.8));
    return vec4(col, max(max(fist, cuff), spring));
}
vec4 zee(vec2 p) {
    float top = bar(p, vec2(-0.8, 0.5), vec2(0.8, 0.85));
    float bottom = bar(p, vec2(-0.8, -0.85), vec2(0.8, -0.5));
    float slash = step(abs(p.x - p.y * 1.2), 0.3) * step(abs(p.y), 0.55);
    return vec4(0.75, 0.85, 1.0, max(max(top, bottom), slash));
}
vec4 sweat(vec2 p) {
    float round_part = disc(p, vec2(0.0, -0.3), 0.55);
    float half_w = 0.5 * (1.0 - (p.y + 0.1) / 0.9);
    float tip = step(abs(p.x), half_w) * step(-0.1, p.y) * step(p.y, 0.8);
    float shine = disc(p, vec2(-0.16, -0.38), 0.14);
    return vec4(mix(vec3(0.45, 0.78, 1.0), vec3(1.0), shine), max(round_part, tip));
}
vec4 crumb(vec2 p) {
    float a = atan(p.y, p.x);
    float d = length(p * vec2(1.0, 1.25)) - (0.72 + 0.14 * sin(a * 5.0 + 1.3));
    vec3 col = mix(vec3(1.0, 0.6, 0.12), vec3(0.85, 0.35, 0.05), smoothstep(-0.4, 0.6, p.x - p.y));
    return vec4(col, 1.0 - smoothstep(-0.06, 0.04, d));
}
vec4 marker(vec2 p) {
    // a fat arrow pointing down, white rim round red
    float outer = step(p.y, 0.75) * step(-0.95 + abs(p.x) * 1.7, p.y) * step(abs(p.x), 0.95);
    float inner = step(p.y, 0.6) * step(-0.75 + abs(p.x) * 1.7, p.y) * step(abs(p.x), 0.8);
    vec3 red = mix(vec3(0.85, 0.12, 0.2), vec3(1.0, 0.4, 0.45), smoothstep(-0.6, 0.6, p.y));
    return vec4(inner > 0.0 ? red : vec3(1.0), outer);
}
vec4 blindfold(vec2 p) {
    float strip = step(abs(p.y), 0.55) * step(abs(p.x), 0.78);
    float knot = disc(p, vec2(0.84, 0.0), 0.28);
    float tail = step(abs(p.y + 0.35 + (p.x - 0.84) * 0.6), 0.16) * step(0.84, p.x);
    vec3 col = mix(vec3(0.7, 0.04, 0.12), vec3(0.98, 0.22, 0.28), smoothstep(-0.5, 0.5, p.y));
    return vec4(col, max(max(strip, knot), tail));
}
vec4 bush(vec2 p) {
    float a = max(max(disc(p, vec2(-0.45, -0.3), 0.52), disc(p, vec2(0.45, -0.3), 0.52)), max(disc(p, vec2(0.0, 0.15), 0.62), bar(p, vec2(-0.92, -1.0), vec2(0.92, -0.35))));
    float spot = disc(p, vec2(-0.2, 0.35), 0.16) + disc(p, vec2(0.4, -0.1), 0.12);
    vec3 col = mix(vec3(0.12, 0.4, 0.16), vec3(0.35, 0.72, 0.3), smoothstep(-0.9, 0.7, p.y));
    col = mix(col, vec3(0.5, 0.85, 0.4), min(spot, 1.0) * 0.6);
    return vec4(col, a);
}
vec4 barrel(vec2 p) {
    float w = 0.66 + 0.14 * (1.0 - p.y * p.y);
    float body = step(abs(p.x), w) * step(abs(p.y), 0.96);
    float hoop = step(abs(abs(p.y) - 0.55), 0.07);
    vec3 col = mix(vec3(0.42, 0.24, 0.11), vec3(0.68, 0.45, 0.24), smoothstep(-w, 0.3, -abs(p.x - 0.15)));
    col = mix(col, vec3(0.32, 0.32, 0.35), hoop);
    return vec4(col, body);
}
vec4 crate_box(vec2 p) {
    float body = bar(p, vec2(-0.92, -0.96), vec2(0.92, 0.86));
    float inner = bar(p, vec2(-0.74, -0.78), vec2(0.74, 0.68));
    float cross_ = step(abs(p.x - p.y * 0.97), 0.13) * inner;
    vec3 col = mix(vec3(0.58, 0.4, 0.22), vec3(0.78, 0.58, 0.34), smoothstep(-1.0, 0.9, p.y));
    col = mix(col, vec3(0.48, 0.32, 0.17), max(1.0 - inner, cross_));
    return vec4(col, body);
}
vec4 note(vec2 p) {
    float head = disc(p, vec2(-0.28, -0.5), 0.34);
    float stem = bar(p, vec2(-0.02, -0.5), vec2(0.12, 0.78));
    float flag = bar(p, vec2(-0.02, 0.52), vec2(0.5, 0.78));
    return vec4(vec3(1.0, 0.86, 0.35), max(max(head, stem), flag));
}
vec4 rod(vec2 p) {
    // bottom left (the hand) to top right (the tip)
    float t = clamp((p.x + 1.0) * 0.5, 0.0, 1.0);
    float y = -1.0 + 2.0 * t;
    float a = step(abs(p.y - y), 0.09 * (1.2 - t));
    vec3 col = mix(vec3(0.35, 0.22, 0.12), vec3(0.75, 0.6, 0.35), t);
    return vec4(col, a);
}
vec4 line_float(vec2 p) {
    float line = step(abs(p.x), 0.07) * step(-0.7, p.y);
    float top = disc(p, vec2(0.0, -0.78), 0.32) * step(-0.78, p.y);
    float bottom = disc(p, vec2(0.0, -0.78), 0.32) * step(p.y, -0.78);
    vec3 col = top > 0.0 ? vec3(0.95, 0.2, 0.2) : bottom > 0.0 ? vec3(1.0) : vec3(0.9);
    return vec4(col, max(line * 0.8, max(top, bottom)));
}
vec4 star(vec2 p) {
    float a = atan(p.y, p.x);
    float r = 0.55 + 0.35 * cos(a * 5.0 + 1.57);
    float body = 1.0 - smoothstep(r - 0.04, r + 0.02, length(p));
    return vec4(mix(vec3(1.0, 0.78, 0.15), vec3(1.0, 0.95, 0.6), smoothstep(0.4, -0.2, length(p))), body);
}
vec4 duck(vec2 p) {
    float body = 1.0 - smoothstep(0.92, 0.98, length((p - vec2(0.0, -0.35)) / vec2(0.8, 0.5)));
    float head = disc(p, vec2(0.35, 0.3), 0.32);
    float beak = bar(p, vec2(0.62, 0.2), vec2(0.9, 0.32));
    float eye = disc(p, vec2(0.42, 0.38), 0.06);
    vec3 col = beak > 0.0 ? vec3(1.0, 0.5, 0.1) : eye > 0.0 ? vec3(0.1) : vec3(1.0, 0.85, 0.2);
    return vec4(col, max(max(body, head), beak));
}
vec4 boot(vec2 p) {
    float leg = bar(p, vec2(-0.5, -0.4), vec2(0.05, 0.9));
    float foot = bar(p, vec2(-0.5, -0.9), vec2(0.85, -0.4)) * (1.0 - step(0.0, (p.x - 0.55) + (p.y + 0.4) * 0.7));
    float sole = bar(p, vec2(-0.55, -0.95), vec2(0.8, -0.8));
    vec3 col = sole > 0.0 ? vec3(0.2, 0.15, 0.1) : vec3(0.45, 0.32, 0.2);
    return vec4(col, max(max(leg, foot), sole));
}
vec4 can(vec2 p) {
    float body = bar(p, vec2(-0.5, -0.85), vec2(0.5, 0.75));
    float label = bar(p, vec2(-0.5, -0.4), vec2(0.5, 0.35));
    vec3 col = label > 0.0 ? vec3(0.3, 0.55, 0.85) : vec3(0.72, 0.74, 0.78);
    col *= 0.75 + 0.25 * smoothstep(0.5, -0.3, abs(p.x + 0.15));
    return vec4(col, body);
}
vec4 beast(vec2 p) {
    float body = 1.0 - smoothstep(0.86, 0.92, length((p - vec2(0.0, -0.25)) / vec2(0.62, 0.8)));
    float arm = max(bar(p, vec2(-0.92, -0.45), vec2(-0.55, 0.05)), bar(p, vec2(0.55, -0.45), vec2(0.92, 0.05)));
    float horn = max(disc(p, vec2(-0.3, 0.62), 0.13), disc(p, vec2(0.3, 0.62), 0.13));
    float eyes = max(disc(p, vec2(-0.24, 0.12), 0.15), disc(p, vec2(0.24, 0.12), 0.15));
    float pupils = max(disc(p, vec2(-0.2, 0.1), 0.06), disc(p, vec2(0.2, 0.1), 0.06));
    float mouth = bar(p, vec2(-0.3, -0.34), vec2(0.3, -0.2));
    float tooth = mouth * step(0.5, fract((p.x + 0.3) * 8.0)) * step(-0.27, p.y);
    vec3 col = vec3(0.25, 0.55, 0.42);
    if (horn > 0.0 && body <= 0.0) col = vec3(0.85, 0.8, 0.6);
    if (mouth > 0.0) col = tooth > 0.0 ? vec3(0.95) : vec3(0.2, 0.03, 0.06);
    if (eyes > 0.0) col = pupils > 0.0 ? vec3(0.8, 0.05, 0.05) : vec3(1.0, 0.95, 0.6);
    return vec4(col, max(max(body, arm), horn));
}
vec4 umbrella(vec2 p) {
    // canopy: a dome whose rim curves up between the ribs
    vec2 q = (p - vec2(0.0, 0.05)) / vec2(0.96, 0.8);
    float dome = step(0.0, q.y) * (1.0 - smoothstep(0.97, 1.0, length(q)));
    float seg = fract((p.x + 0.96) / 0.48);
    float edge = 0.05 + 0.1 * sin(3.14159 * seg);
    dome *= smoothstep(edge - 0.02, edge + 0.01, p.y);
    // ribs from the tip down to each point of the rim
    float ribs = 0.0;
    for (int k = 0; k < 5; k++) {
        float rx = -0.96 + 0.48 * float(k);
        vec2 a = vec2(0.0, 0.85), b = vec2(rx, 0.05);
        vec2 ab = b - a;
        float t = clamp(dot(p - a, ab) / dot(ab, ab), 0.0, 1.0);
        ribs = max(ribs, 1.0 - smoothstep(0.012, 0.03, length(p - a - ab * t)));
    }
    // soft shading: lighter towards the top
    float light = clamp(q.y * 0.6 + 0.4, 0.0, 1.0);
    vec3 cloth = mix(vec3(0.82, 0.33, 0.45), vec3(1.0, 0.62, 0.72), light);
    vec3 col = mix(cloth, vec3(0.62, 0.2, 0.3), ribs * 0.8);
    // the shaft, a little tip on top, and a hooked handle at the bottom
    float shaft = bar(p, vec2(-0.025, -0.82), vec2(0.025, 0.9));
    float tip = disc(p, vec2(0.0, 0.92), 0.04);
    vec2 h = p - vec2(0.1, -0.82);
    float hook = step(h.y, 0.0) * (1.0 - smoothstep(0.02, 0.04, abs(length(h) - 0.1)));
    float stick = max(max(shaft, tip), hook);
    col = mix(col, vec3(0.32, 0.22, 0.16), stick);
    return vec4(col, max(dome, stick));
}
float smin(float a, float b, float k) {
    float h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}
vec4 snowpile(vec2 p) {
    // lumps of snow heaped up into a little mound, a few blobs hanging over the edge
    float d = length(p - vec2(0.0, -0.3)) - 0.62;
    d = smin(d, length(p - vec2(-0.52, -0.45)) - 0.42, 0.18);
    d = smin(d, length(p - vec2(0.5, -0.42)) - 0.45, 0.18);
    d = smin(d, length(p - vec2(-0.18, 0.18)) - 0.4, 0.18);
    d = smin(d, length(p - vec2(0.24, 0.08)) - 0.38, 0.18);
    d = smin(d, length(p - vec2(0.04, 0.58)) - 0.26, 0.16);
    // flat where it sits, except for the drips over the edge
    float base = p.y + 0.62;
    float drips = min(min(length(p - vec2(-0.8, -0.66)) - 0.13, length(p - vec2(-0.32, -0.7)) - 0.1),
                      min(length(p - vec2(0.38, -0.68)) - 0.12, length(p - vec2(0.84, -0.64)) - 0.11));
    float body = max(d, -base);
    float shape = min(body, drips);
    float a = 1.0 - smoothstep(-0.02, 0.02, shape);
    // lit from above: white on top, bluish in the folds and underneath
    float inner = clamp(-shape / 0.25, 0.0, 1.0);
    vec3 col = mix(vec3(0.74, 0.82, 0.96), vec3(1.0), clamp(0.35 + p.y * 0.45 + inner * 0.4, 0.0, 1.0));
    // the creases between lumps
    col = mix(col, vec3(0.8, 0.86, 0.97), (1.0 - smoothstep(0.0, 0.05, abs(d + 0.08))) * 0.25 * step(-0.5, p.y));
    // a few sparkles
    vec2 g = floor(p * 9.0);
    float h = fract(sin(dot(g, vec2(41.3, 289.1))) * 43758.5);
    float spark = step(0.93, h) * (1.0 - smoothstep(0.03, 0.08, length(fract(p * 9.0) - 0.5)));
    col = mix(col, vec3(1.0), spark);
    col = mix(col, vec3(0.86, 0.93, 1.0), spark * 0.0);
    return vec4(col, a);
}
vec4 santa(vec2 p) {
    // the hat: a red cone curving over to one side, fur brim, a bobble
    float brim = sdBox(p - vec2(0.0, -0.78), vec2(0.92, 0.2), 0.18);
    float t = clamp((p.y + 0.6) / 1.45, 0.0, 1.0);
    float bend = 0.55 * t * t;
    float halfw = 0.82 * (1.0 - t) + 0.06;
    float cone = step(-0.62, p.y) * step(p.y, 0.85) * (1.0 - smoothstep(halfw - 0.03, halfw, abs(p.x - bend)));
    float bobble = 1.0 - smoothstep(0.17, 0.21, length(p - vec2(0.62, 0.78)));
    float fur = max(1.0 - smoothstep(-0.02, 0.02, brim), bobble);
    vec3 red = mix(vec3(0.72, 0.05, 0.1), vec3(0.92, 0.18, 0.2), clamp(0.5 - (p.x - bend) * 0.6, 0.0, 1.0));
    vec3 col = mix(red, vec3(0.98, 0.98, 1.0), fur);
    // a little texture on the fur
    col -= fur * 0.06 * step(0.6, fract(sin(dot(floor(p * 18.0), vec2(12.9, 78.2))) * 43758.5));
    return vec4(col, max(cone, fur));
}
vec4 sleepmask(vec2 p) {
    // a padded mask with stitched closed eyes, polka dots and a bow
    float d = sdBox(p, vec2(0.92, 0.62), 0.55);
    float a = 1.0 - smoothstep(-0.02, 0.02, d);
    vec3 col = vec3(0.98, 0.72, 0.82);
    vec2 cell = fract(p * vec2(4.0, 2.6)) - 0.5;
    col = mix(col, vec3(1.0, 0.95, 0.97), (1.0 - smoothstep(0.12, 0.18, length(cell))) * 0.85);
    for (int k = 0; k < 2; k++) {
        vec2 q = (p - vec2(k == 0 ? -0.45 : 0.45, -0.02)) / vec2(0.3, 0.3);
        float curve = q.y - (0.1 - 0.5 * q.x * q.x);
        col = mix(col, vec3(0.42, 0.18, 0.3), smoothstep(0.22, 0.1, abs(curve)) * step(abs(q.x), 0.9));
    }
    col = mix(col, vec3(0.86, 0.5, 0.64), smoothstep(-0.12, 0.0, d) * 0.6);
    vec2 b = (p - vec2(0.82, 0.5)) / 0.32;
    float loops = (1.0 - smoothstep(0.95, 1.05, length(vec2(abs(b.x) - 0.5, b.y * 1.3) / vec2(0.48, 0.42)))) * step(0.08, abs(b.x));
    float knot = 1.0 - smoothstep(0.16, 0.22, length(b));
    float bow = max(loops, knot);
    col = mix(col, mix(vec3(0.93, 0.36, 0.53), vec3(0.76, 0.24, 0.42), knot), bow);
    return vec4(col, max(a, bow));
}
vec4 partyhat(vec2 p) {
    float t = clamp((p.y + 1.0) / 1.75, 0.0, 1.0);
    float cone = step(-1.0, p.y) * step(p.y, 0.75) * (1.0 - smoothstep(0.88 * (1.0 - t) - 0.03, 0.88 * (1.0 - t), abs(p.x)));
    float stripe = step(0.5, fract((p.y + p.x * 0.4) * 2.2));
    vec3 col = mix(vec3(0.35, 0.65, 1.0), vec3(1.0, 0.85, 0.3), stripe);
    float pom = 1.0 - smoothstep(0.17, 0.21, length(p - vec2(0.0, 0.8)));
    col = mix(col, vec3(1.0, 0.45, 0.6), pom);
    return vec4(col, max(cone, pom));
}
vec4 tophat(vec2 p) {
    float brim = sdBox(p - vec2(0.0, -0.85), vec2(0.95, 0.12), 0.1);
    float body = sdBox(p - vec2(0.0, 0.05), vec2(0.6, 0.85), 0.06);
    float a = 1.0 - smoothstep(-0.02, 0.02, min(brim, body));
    vec3 col = mix(vec3(0.1, 0.1, 0.14), vec3(0.22, 0.22, 0.28), clamp(p.x * 0.5 + 0.5, 0.0, 1.0));
    float band = step(abs(p.y + 0.5), 0.13) * step(body, 0.0);
    col = mix(col, vec3(0.75, 0.15, 0.25), band);
    return vec4(col, a);
}
vec4 bigbow(vec2 p) {
    vec2 b = p / vec2(1.0, 0.95);
    float loops = (1.0 - smoothstep(0.95, 1.05, length(vec2(abs(b.x) - 0.5, b.y * 1.15) / vec2(0.5, 0.75)))) * step(0.1, abs(b.x));
    float knot = 1.0 - smoothstep(0.22, 0.28, length(b * vec2(1.0, 0.8)));
    vec3 col = mix(vec3(0.95, 0.35, 0.5), vec3(0.75, 0.18, 0.35), knot);
    col = mix(col, vec3(1.0, 0.6, 0.72), step(0.6, abs(b.x)) * step(0.0, b.y) * 0.4);
    return vec4(col, max(loops, knot));
}
vec4 glasses(vec2 p, bool dark) {
    float l = abs(length((p - vec2(-0.48, 0.0)) / vec2(0.42, 0.85)) - 1.0);
    float r = abs(length((p - vec2(0.48, 0.0)) / vec2(0.42, 0.85)) - 1.0);
    float rim = 1.0 - smoothstep(0.1, 0.18, min(l, r));
    float bridge = bar(p, vec2(-0.08, 0.15), vec2(0.08, 0.3));
    float lens = max(1.0 - step(1.0, length((p - vec2(-0.48, 0.0)) / vec2(0.42, 0.85))), 1.0 - step(1.0, length((p - vec2(0.48, 0.0)) / vec2(0.42, 0.85))));
    vec3 frame = dark ? vec3(0.08) : vec3(0.35, 0.22, 0.15);
    vec4 glass = dark ? vec4(0.05, 0.05, 0.1, 0.85) : vec4(0.85, 0.95, 1.0, 0.18);
    float frm = max(rim, bridge);
    vec3 col = mix(glass.rgb, frame, frm);
    return vec4(col, max(frm, lens * glass.a));
}
vec4 flower(vec2 p) {
    float a = atan(p.y, p.x);
    float r = length(p);
    float petals = 1.0 - smoothstep(0.0, 0.05, r - (0.62 + 0.3 * cos(a * 5.0)));
    float core = 1.0 - smoothstep(0.24, 0.3, r);
    vec3 col = mix(vec3(1.0, 0.7, 0.82), vec3(1.0, 0.85, 0.3), core);
    return vec4(col, max(petals, core));
}
vec4 scarf(vec2 p) {
    // worn: a thick knitted wrap curving under the chin, one end hanging
    // down the front with a fringe
    float bend = 0.28 * (1.0 - p.x * p.x);
    float y = p.y + bend;
    float wrap = step(abs(p.x), 0.97) * (1.0 - smoothstep(0.24, 0.3, abs(y - 0.38)));
    // the knot and the hanging end, on the right of the box
    float knot = 1.0 - smoothstep(0.17, 0.22, length((p - vec2(0.3, 0.15)) / vec2(1.0, 1.1)));
    float tail_x = 0.3 + 0.08 * (p.y + 0.2);
    float tail = step(abs(p.x - tail_x), 0.15 - 0.02 * p.y) * step(-0.75, p.y) * step(p.y, 0.2);
    float fringe = step(-0.95, p.y) * step(p.y, -0.75) * step(abs(p.x - tail_x), 0.15) * step(0.5, fract((p.x - tail_x) * 13.0));
    float a = max(max(wrap, knot), max(tail, fringe));
    // red and cream stripes, knit ridges, shade towards the edges
    float stripes = step(0.5, fract((tail > 0.0 && wrap <= 0.0 ? p.y * 3.0 : p.x * 2.6) + 0.25));
    vec3 col = mix(vec3(0.82, 0.18, 0.24), vec3(0.98, 0.93, 0.85), stripes * 0.85);
    float ridge = 0.08 * sin((tail > 0.0 && wrap <= 0.0 ? p.x : y) * 60.0);
    col *= 0.92 + ridge;
    col *= mix(0.78, 1.0, 1.0 - smoothstep(0.1, 0.3, abs(y - 0.38)) * wrap * 0.0 - 0.0);
    col = mix(col, col * 0.75, (1.0 - smoothstep(0.0, 0.12, 0.3 - abs(y - 0.38))) * wrap);
    col = mix(col, vec3(0.7, 0.13, 0.18), knot * (1.0 - wrap) * 0.6);
    return vec4(col, a);
}
vec4 backpack(vec2 p) {
    float bag = sdBox(p - vec2(0.0, -0.05), vec2(0.8, 0.85), 0.3);
    float a = 1.0 - smoothstep(-0.02, 0.02, bag);
    vec3 col = vec3(0.98, 0.62, 0.3);
    float pocket = 1.0 - smoothstep(-0.02, 0.02, sdBox(p - vec2(0.0, -0.4), vec2(0.5, 0.3), 0.15));
    col = mix(col, vec3(0.92, 0.5, 0.22), pocket);
    return vec4(col, a);
}
vec4 wings(vec2 p) {
    vec2 q = vec2(abs(p.x), p.y);
    float wing = 0.0;
    for (int k = 0; k < 3; k++) {
        float fk = float(k);
        vec2 c = vec2(0.35 + fk * 0.2, 0.25 - fk * 0.28);
        wing = max(wing, 1.0 - smoothstep(0.95, 1.0, length((q - c) / vec2(0.42 - fk * 0.05, 0.3))));
    }
    wing *= step(0.12, q.x);
    vec3 col = mix(vec3(0.88, 0.92, 1.0), vec3(1.0), clamp(p.y * 0.5 + 0.5, 0.0, 1.0));
    return vec4(col, wing * 0.95);
}
vec4 headphones(vec2 p) {
    float band = abs(length((p - vec2(0.0, -0.3)) / vec2(0.9, 1.15)) - 1.0);
    float arc = (1.0 - smoothstep(0.05, 0.1, band)) * step(-0.3, p.y);
    float cups = max(1.0 - smoothstep(-0.02, 0.02, sdBox(p - vec2(-0.86, -0.45), vec2(0.14, 0.3), 0.1)),
                     1.0 - smoothstep(-0.02, 0.02, sdBox(p - vec2(0.86, -0.45), vec2(0.14, 0.3), 0.1)));
    vec3 col = mix(vec3(0.3, 0.3, 0.38), vec3(0.95, 0.45, 0.65), cups);
    return vec4(col, max(arc, cups));
}
vec4 crown(vec2 p) {
    float base = step(-1.0, p.y) * step(p.y, -0.3) * step(abs(p.x), 0.9);
    float spikes = step(-0.3, p.y) * step(p.y, 0.9 - abs(fract(p.x * 1.5 + 0.5) - 0.5) * 3.6) * step(abs(p.x), 0.9);
    float a = max(base, spikes);
    vec3 col = mix(vec3(0.95, 0.75, 0.2), vec3(1.0, 0.9, 0.45), clamp(p.y * 0.5 + 0.5, 0.0, 1.0));
    float gem = 1.0 - smoothstep(0.12, 0.16, length(p - vec2(0.0, -0.62)));
    col = mix(col, vec3(0.85, 0.15, 0.3), gem);
    return vec4(col, a);
}
vec4 picture(vec2 p) {
    // read the picture for every pixel (no early return): texture lookups
    // under a branch pick a wrong, blurry level at its edge and leave faint
    // lines round the box; what's outside the picture is masked after
    vec2 q = p * u_fit;
    vec2 uv = u_crop.xy + clamp(vec2(q.x * 0.5 + 0.5, 0.5 - q.y * 0.5), 0.0, 1.0) * u_crop.zw;
    vec4 c = texture(u_img, uv);
    float inside = step(abs(q.x), 1.0) * step(abs(q.y), 1.0) * step(p.y, -1.0 + 2.0 * u_reveal);
    return c * inside;
}
vec4 canopy(vec2 p) {
    // the umbrella's top on its own: a ribbed dome filling the box, rim at the bottom
    vec2 q = (p - vec2(0.0, -0.85)) / vec2(0.98, 1.72);
    float dome = step(0.0, q.y) * (1.0 - smoothstep(0.97, 1.0, length(q)));
    float seg = fract((p.x + 0.98) / 0.49);
    float edge = -0.85 + 0.22 * sin(3.14159 * seg);
    dome *= smoothstep(edge - 0.03, edge + 0.02, p.y);
    float ribs = 0.0;
    for (int k = 0; k < 5; k++) {
        vec2 a = vec2(0.0, 0.87), b = vec2(-0.98 + 0.49 * float(k), -0.85);
        vec2 ab = b - a;
        float t = clamp(dot(p - a, ab) / dot(ab, ab), 0.0, 1.0);
        ribs = max(ribs, 1.0 - smoothstep(0.015, 0.04, length(p - a - ab * t)));
    }
    float light = clamp(q.y * 0.6 + 0.4, 0.0, 1.0);
    vec3 cloth = mix(vec3(0.82, 0.33, 0.45), vec3(1.0, 0.62, 0.72), light);
    vec3 col = mix(cloth, vec3(0.62, 0.2, 0.3), ribs * 0.8);
    float tip = 1.0 - smoothstep(0.06, 0.1, length(p - vec2(0.0, 0.9)));
    col = mix(col, vec3(0.32, 0.22, 0.16), tip);
    return vec4(col, max(dome, tip));
}
vec4 shaft(vec2 p) {
    // a thin stick down to a hooked handle at the bottom
    float stick = step(abs(p.x), 0.18) * step(-0.86, p.y);
    vec2 h = (p - vec2(0.42, -0.86)) / vec2(0.42, 0.07);
    float hook = step(h.y, 0.0) * (1.0 - smoothstep(0.75, 1.0, abs(length(h) - 1.0) * 6.0)) * step(-1.2, h.y);
    return vec4(vec3(0.32, 0.22, 0.16), max(stick, hook));
}
vec4 rainfall(vec2 p) {
    // slanted streaks over the whole screen, columns falling at their own speed
    vec2 q = vec2((p.x * 0.5 + 0.5) * u_saspect, p.y * 0.5 + 0.5);
    q.x += q.y * 0.12;
    float a = 0.0;
    for (int k = 0; k < 2; k++) {
        float scale = k == 0 ? 70.0 : 120.0;
        float x = q.x * scale;
        float col = floor(x);
        float h = hash1(col + float(k) * 31.0);
        float y = fract(q.y * (1.2 + h) + u_time * (1.1 + h * 0.8) + h * 7.0);
        float streak = step(abs(fract(x) - 0.5), 0.05) * smoothstep(0.0, 0.015, y) * (1.0 - smoothstep(0.05, 0.07, y)) * step(0.45, h);
        a = max(a, streak * (k == 0 ? 0.5 : 0.3));
    }
    return vec4(vec3(0.75, 0.82, 0.95), a);
}
vec4 snowfall(vec2 p) {
    // flakes drifting down and swaying, three depths
    vec2 q = vec2((p.x * 0.5 + 0.5) * u_saspect, p.y * 0.5 + 0.5);
    float a = 0.0;
    for (int k = 0; k < 3; k++) {
        float fk = float(k);
        float scale = 9.0 + fk * 7.0;
        vec2 g = vec2(q.x, q.y + u_time * (0.06 + fk * 0.03)) * scale;
        g.x += sin(u_time * 0.8 + g.y * 0.7 + fk) * 0.35;
        vec2 cell = floor(g);
        float h = hash1(cell.x * 13.0 + cell.y * 71.0 + fk * 5.0);
        vec2 f = fract(g) - vec2(hash1(cell.x + cell.y * 3.0 + fk), hash1(cell.y + cell.x * 7.0 + fk));
        float r = 0.04 + 0.05 * (1.0 - fk / 3.0);
        a = max(a, step(0.55, h) * (1.0 - smoothstep(r * 0.6, r, length(f))) * (0.9 - fk * 0.2));
    }
    return vec4(vec3(1.0), a);
}
vec4 furled(vec2 p) {
    // a rolled-up umbrella slung across the back: tip up one side, hook down the other
    float c = 0.7071;
    vec2 q = vec2(c * p.x + c * p.y, -c * p.x + c * p.y);
    float t = clamp((q.y + 0.9) / 1.7, 0.0, 1.0);
    float body = step(abs(q.x), 0.05 + 0.13 * (1.0 - t)) * step(-0.55, q.y) * step(q.y, 0.85);
    float strap = step(abs(q.y + 0.1), 0.05) * step(abs(q.x), 0.17);
    float stick = step(abs(q.x), 0.03) * step(-0.85, q.y) * step(q.y, -0.55);
    vec2 h = q - vec2(0.09, -0.85);
    float hook = step(h.y, 0.0) * (1.0 - smoothstep(0.02, 0.04, abs(length(h) - 0.09)));
    vec3 col = mix(vec3(0.85, 0.36, 0.48), vec3(1.0, 0.6, 0.7), smoothstep(-0.1, 0.1, q.x));
    col = mix(col, vec3(0.62, 0.2, 0.3), strap);
    float wood = max(stick, hook);
    col = mix(col, vec3(0.32, 0.22, 0.16), wood);
    return vec4(col, max(max(body, strap), wood));
}
vec4 snowflake(vec2 p) {
    float a = atan(p.y, p.x);
    float r = length(p);
    float arms = smoothstep(0.18, 0.05, abs(sin(a * 3.0)) * r) * step(r, 0.9);
    float core = disc(p, vec2(0.0), 0.2);
    return vec4(vec3(0.95, 0.97, 1.0), max(arms, core) * 0.9);
}
vec4 confetti(vec2 p) {
    vec2 cell = floor((p + 1.0) * 4.0);
    vec2 f = fract((p + 1.0) * 4.0) - 0.5;
    float h = fract(sin(dot(cell, vec2(12.9898, 78.233))) * 43758.5453);
    float on = step(0.55, h) * step(abs(f.x), 0.22) * step(abs(f.y), 0.12) * step(length(p), 1.0);
    vec3 col = h > 0.85 ? vec3(1.0, 0.4, 0.6) : h > 0.75 ? vec3(0.4, 0.8, 1.0) : h > 0.65 ? vec3(1.0, 0.85, 0.3) : vec3(0.5, 0.95, 0.5);
    return vec4(col, on);
}
float snowseg(vec2 p, vec2 a, vec2 b) {
    vec2 pa = p - a, ba = b - a;
    float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}
vec4 snowball(vec2 p) {
    float a = 1.0 - smoothstep(0.88, 0.96, length(p));
    // lit from the top left, bluish underneath
    vec3 col = mix(vec3(0.72, 0.8, 0.95), vec3(1.0), clamp(0.55 + p.y * 0.35 - p.x * 0.2, 0.0, 1.0));
    vec2 g = floor(p * 5.0);
    float h = fract(sin(dot(g, vec2(27.1, 61.7))) * 4375.5);
    col = mix(col, vec3(0.83, 0.89, 0.98), step(0.78, h) * 0.4);
    return vec4(col, a);
}
vec4 snowman(vec2 p) {
    // the box is 0.6 as wide as tall: q keeps the balls round
    vec2 q = vec2(p.x * 0.6, p.y);
    vec2 c0 = vec2(0.0, -0.58), c1 = vec2(0.0, -0.03), c2 = vec2(0.0, 0.47);
    float body = max(max(disc(q, c0, 0.42), disc(q, c1, 0.31)), disc(q, c2, 0.23));
    float lit = max(max(1.0 - length(q - c0 - vec2(-0.12, 0.13)) / 0.62, 1.0 - length(q - c1 - vec2(-0.09, 0.1)) / 0.46), 1.0 - length(q - c2 - vec2(-0.07, 0.07)) / 0.34);
    vec3 col = mix(vec3(0.72, 0.8, 0.95), vec3(1.0), clamp(0.25 + lit * 0.9, 0.0, 1.0));
    float a = body;
    // twig arms
    float arms = min(min(snowseg(q, vec2(0.25, 0.02), vec2(0.55, 0.24)), snowseg(q, vec2(0.46, 0.17), vec2(0.5, 0.3))),
                     min(snowseg(q, vec2(-0.25, 0.02), vec2(-0.55, 0.22)), snowseg(q, vec2(-0.47, 0.16), vec2(-0.57, 0.27))));
    float arm = 1.0 - smoothstep(0.012, 0.022, arms);
    col = mix(col, vec3(0.42, 0.27, 0.15), arm * (1.0 - body));
    a = max(a, arm);
    // a red scarf round the neck, one end hanging
    float scarf = max(bar(q, vec2(-0.2, 0.22), vec2(0.2, 0.29)), bar(q, vec2(0.08, 0.05), vec2(0.16, 0.25)));
    col = mix(col, mix(vec3(0.85, 0.15, 0.2), vec3(0.95, 0.35, 0.35), step(0.5, fract(q.x * 12.0))), scarf);
    a = max(a, scarf);
    // coal eyes, mouth and buttons
    float coal = max(disc(q, vec2(-0.075, 0.53), 0.03), disc(q, vec2(0.075, 0.53), 0.03));
    for (int k = 0; k < 5; k++) {
        float x = (float(k) - 2.0) * 0.045;
        coal = max(coal, disc(q, vec2(x, 0.395 - 0.04 * (1.0 - abs(x) * 10.0)), 0.016));
    }
    coal = max(coal, max(disc(q, vec2(0.0, 0.06), 0.028), max(disc(q, vec2(0.0, -0.07), 0.028), disc(q, vec2(0.0, -0.4), 0.032))));
    col = mix(col, vec3(0.12, 0.12, 0.15), coal);
    // a carrot nose, pointing out to the side
    float nx = clamp(q.x / 0.24, 0.0, 1.0);
    float nose = step(0.0, q.x) * step(q.x, 0.24) * step(abs(q.y - 0.465 + nx * 0.03), 0.035 * (1.0 - nx));
    col = mix(col, vec3(0.98, 0.55, 0.15), nose);
    a = max(a, nose);
    return vec4(col, a);
}
vec4 snowpuff(vec2 p) {
    float a = disc(p, vec2(0.0), 0.32);
    for (int k = 0; k < 7; k++) {
        float ang = float(k) * 0.9 + 0.3;
        a = max(a, disc(p, vec2(cos(ang), sin(ang)) * 0.58, 0.2));
    }
    return vec4(vec3(0.97, 0.98, 1.0), a * 0.9);
}
//SLIDE_TUBE

void main() {
    if (u_hide.y != 0.0 && (v_sclip.x - u_hide.x) * u_hide.y > 0.0) discard;
    vec2 p = v_pos;
    if (u_flip == 1) p.x = -p.x;
    vec4 c = u_kind == 1 ? box(p) : u_kind == 2 ? balloon(p) : u_kind == 3 ? glove(p) : u_kind == 4 ? zee(p) : u_kind == 5 ? sweat(p) : u_kind == 6 ? crumb(p) : u_kind == 7 ? marker(p) : u_kind == 8 ? blindfold(p) : u_kind == 9 ? bush(p) : u_kind == 10 ? barrel(p) : u_kind == 11 ? crate_box(p) : u_kind == 12 ? note(p) : u_kind == 13 ? rod(p) : u_kind == 14 ? line_float(p) : u_kind == 15 ? star(p) : u_kind == 16 ? duck(p) : u_kind == 17 ? boot(p) : u_kind == 18 ? can(p) : u_kind == 20 ? confetti(p) : u_kind == 21 ? beast(p) : u_kind == 22 ? umbrella(p) : u_kind == 23 ? snowflake(p) : u_kind == 24 ? snowpile(p) : u_kind == 25 ? santa(p) : u_kind == 26 ? sleepmask(p) : u_kind == 27 ? partyhat(p) : u_kind == 28 ? tophat(p) : u_kind == 29 ? bigbow(p) : u_kind == 30 ? glasses(p, false) : u_kind == 31 ? glasses(p, true) : u_kind == 32 ? flower(p) : u_kind == 33 ? scarf(p) : u_kind == 34 ? backpack(p) : u_kind == 35 ? wings(p) : u_kind == 36 ? headphones(p) : u_kind == 37 ? crown(p) : u_kind == 38 ? canopy(p) : u_kind == 39 ? shaft(p) : u_kind == 40 ? rainfall(p) : u_kind == 41 ? snowfall(p) : u_kind == 42 ? furled(p) : u_kind == 43 ? snowball(p) : u_kind == 44 ? snowman(p) : u_kind == 45 ? snowpuff(p) : u_kind == 100 ? picture(p) : carrot(p);
    c.a *= u_opacity;
    c = apply_slide_tube(c, v_sclip);
    fragColor = vec4(c.rgb * c.a, c.a);
}
"#;

pub(super) struct CarrotGl {
    program: Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    // accessories' pictures, loaded once (None: couldn't be read)
    images: std::collections::HashMap<std::path::PathBuf, Option<(glow::Texture, f32)>>,
}

impl CarrotGl {
    fn new(gl: &glow::Context) -> Result<Self> {
        let fs = CARROT_FS.replace("//SLIDE_TUBE", super::park::SLIDE_TUBE_GLSL);
        let program = Program::new(gl, CARROT_VS, &fs)?;
        let verts: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
        unsafe {
            let vao = gl.create_vertex_array().map_err(|e| anyhow!(e))?;
            let vbo = gl.create_buffer().map_err(|e| anyhow!(e))?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck_cast(&verts), glow::STATIC_DRAW);
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            gl.bind_vertex_array(None);
            Ok(Self { program, vao, vbo, images: Default::default() })
        }
    }

    pub(super) fn destroy(&mut self, gl: &glow::Context) {
        unsafe {
            for (_, tex) in self.images.drain() {
                if let Some((t, _)) = tex {
                    gl.delete_texture(t);
                }
            }
            gl.delete_vertex_array(self.vao);
            gl.delete_buffer(self.vbo);
        }
        self.program.destroy(gl);
    }
}

impl CarrotGl {
    /// A picture's texture and width / height, loaded once.
    fn picture(&mut self, gl: &glow::Context, path: &std::path::Path) -> Option<(glow::Texture, f32)> {
        *self.images.entry(path.to_path_buf()).or_insert_with(|| {
            let img = image::open(path).ok()?.to_rgba8();
            let (iw, ih) = img.dimensions();
            let tex = crate::render::gl::create_texture(gl, iw, ih, img.as_raw(), crate::render::gl::Filter::Linear, true).ok()?;
            Some((tex, iw as f32 / ih.max(1) as f32))
        })
    }
}

/// The canopy of the umbrella picture: the top part, above the stick.
const CANOPY: [f32; 4] = [0.0, 0.08, 1.0, 0.52];

/// How much to stretch the box's coordinates so a picture of `aspect`
/// (width / height) keeps its shape inside a box `w` x `h`, centred.
fn fit(w: f32, h: f32, aspect: f32) -> [f32; 2] {
    let box_aspect = (w / h.max(0.01)).max(0.01);
    if box_aspect > aspect { [box_aspect / aspect, 1.0] } else { [1.0, aspect / box_aspect] }
}

/// The fish or sea creatures in the props folder, in name order, read once.
fn sea_list(base: i32) -> Vec<std::path::PathBuf> {
    static LISTS: std::sync::LazyLock<[Vec<std::path::PathBuf>; 2]> = std::sync::LazyLock::new(|| {
        let read = |dir: &str| {
            let mut v: Vec<std::path::PathBuf> = std::fs::read_dir(crate::util::data_dir().join("props").join(dir))
                .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "png")).collect())
                .unwrap_or_default();
            v.sort();
            v
        };
        [read("fish"), read("creatures")]
    });
    LISTS[usize::from(base >= super::social::CREATURE)].clone()
}

pub(super) fn sea_count(base: i32) -> usize {
    sea_list(base).len()
}

/// The picture of a fish or a sea creature caught.
pub(super) fn sea_picture(kind: i32) -> Option<std::path::PathBuf> {
    let base = if kind >= super::social::CREATURE { super::social::CREATURE } else { super::social::FISH };
    sea_list(base).get((kind - base) as usize).cloned()
}

/// Its name: "sea turtle" from sea_turtle.png.
pub(super) fn sea_name(kind: i32) -> Option<String> {
    Some(sea_picture(kind)?.file_stem()?.to_string_lossy().replace('_', " "))
}

/// A picture's width / height, read once.
pub(super) fn picture_aspect(path: &std::path::Path) -> Option<f32> {
    static SEEN: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, Option<f32>>>> = std::sync::LazyLock::new(Default::default);
    *SEEN.lock().unwrap().entry(path.to_path_buf()).or_insert_with(|| image::image_dimensions(path).ok().map(|(w, h)| w as f32 / h.max(1) as f32))
}

/// A box made as tall as a picture of `aspect` needs at the box's width,
/// about its middle (screen px, y down).
pub(super) fn picture_box(r: [f32; 4], aspect: f32) -> [f32; 4] {
    let (cy, h) = ((r[1] + r[3]) * 0.5, (r[2] - r[0]) / aspect.max(0.01));
    [r[0], cy - h * 0.5, r[2], cy + h * 0.5]
}

/// The picture standing in for a drawn prop: a PNG named after it in
/// `~/.local/share/anima/props/` (looked for again every few seconds).
pub(super) fn prop_image(kind: i32) -> Option<std::path::PathBuf> {
    use std::time::{Duration, Instant};
    // something to wear, found on a fishing line: its own picture, or the
    // props folder's for its drawn kind
    if kind >= super::social::FISH {
        return sea_picture(kind);
    }
    if kind >= super::social::FOUND {
        let (_, _, image, drawn) = super::wear::catalog_entry((kind - super::social::FOUND) as usize)?;
        return image.or_else(|| prop_image(drawn));
    }
    let name = match kind {
        0 => "carrot",
        1 => "gift",
        2 => "balloon",
        12 => "note",
        15 => "star",
        16 => "duck",
        17 => "boot",
        18 => "can",
        21 => "beast",
        22 | 38 => "umbrella",
        42 => "umbrella_closed",
        23 => "snowflake",
        28 => "top_hat",
        29 => "bow",
        30 => "glasses",
        31 => "sunglasses",
        32 => "flower",
        34 => "backpack",
        36 => "headphones",
        37 => "crown",
        _ => return None,
    };
    type Seen = (Option<Instant>, std::collections::HashSet<String>);
    static SEEN: std::sync::LazyLock<std::sync::Mutex<Seen>> = std::sync::LazyLock::new(|| std::sync::Mutex::new((None, std::collections::HashSet::new())));
    let dir = crate::util::data_dir().join("props");
    let mut s = SEEN.lock().unwrap();
    if s.0.is_none_or(|t| t.elapsed() > Duration::from_secs(3)) {
        s.0 = Some(Instant::now());
        s.1 = std::fs::read_dir(&dir).map(|d| d.flatten().filter_map(|e| e.file_name().into_string().ok()).collect()).unwrap_or_default();
    }
    let file = format!("{name}.png");
    s.1.contains(&file).then(|| dir.join(file))
}

// rain or snow over the whole screen moves on at its own, lower frame rate:
// the same picture in between, so the screen needn't be redone every frame
static SHEET_T0: std::sync::LazyLock<std::time::Instant> = std::sync::LazyLock::new(std::time::Instant::now);
static SHEET_SHOWN: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);

/// The weather sheet's frame number and its time (s).
pub(super) fn sheet_frame() -> (i64, f32) {
    let fps = crate::render::tuning::get().weather_fps.max(1.0);
    let step = (SHEET_T0.elapsed().as_secs_f32() * fps).floor();
    (step as i64, step / fps)
}

/// True once per new frame of the weather sheet: the whole screen changed.
pub(super) fn sheet_moved() -> bool {
    let (step, _) = sheet_frame();
    SHEET_SHOWN.swap(step, std::sync::atomic::Ordering::Relaxed) != step
}

impl CharacterOverlay {
    /// Rain or snow over the whole screen, if that's how the weather is shown.
    pub(super) fn weather_sheet(&self) -> Option<i32> {
        let sky = super::weather::now();
        let whole = crate::render::tuning::get().weather_show >= 1.5 && !super::locked();
        if whole && sky.rain {
            Some(40)
        } else if whole && sky.snow {
            Some(41)
        } else {
            None
        }
    }

    /// Worn and held things, drawn with the sprite's own transform: `behind`
    /// picks the ones that go behind it. Called between the sprite's
    /// target begin and end.
    pub(super) fn draw_worn(
        &mut self,
        gl: &glow::Context,
        worn: &[super::wear::Worn],
        clip: [f32; 4],
        behind: bool,
        slide: Option<&super::park::SlideRenderData>,
    ) {
        if !worn.iter().any(|w| w.behind == behind) {
            return;
        }
        if self.carrot_gl.is_none() {
            match CarrotGl::new(gl) {
                Ok(c) => self.carrot_gl = Some(c),
                Err(e) => {
                    log::warn!("worn things: {e:#}");
                    return;
                }
            }
        }
        let pose = self.pose;
        let hide = self.hide_clip(self.surface[0]);
        let Some(c) = self.carrot_gl.as_mut() else { return };
        c.program.bind(gl);
        super::bind_slide_uniforms(gl, &mut c.program, pose.size, slide);
        c.program.set_vec4(gl, "u_clip", clip);
        c.program.set_i32(gl, "u_attach", 1);
        c.program.set_vec2(gl, "c_center", pose.center[0], pose.center[1]);
        c.program.set_vec2(gl, "c_half", pose.half[0], pose.half[1]);
        c.program.set_vec2(gl, "c_tilt", pose.tilt[0], pose.tilt[1]);
        c.program.set_f32(gl, "c_breath", pose.breath);
        c.program.set_f32(gl, "c_bounce", pose.bounce);
        c.program.set_f32(gl, "c_rot", pose.rot);
        c.program.set_f32(gl, "c_sink", pose.sink);
        c.program.set_f32(gl, "c_aspect", pose.aspect);
        c.program.set_vec2(gl, "c_pivot", pose.pivot[0], pose.pivot[1]);
        c.program.set_f32(gl, "c_xscale", pose.xscale);
        c.program.set_f32(gl, "u_reveal", 1.0);
        c.program.set_f32(gl, "u_opacity", pose.opacity);
        c.program.set_vec2(gl, "u_hide", hide[0], hide[1]);
        for w in worn.iter().filter(|w| w.behind == behind) {
            // a picture: its own, or the one in the props folder for this kind
            let picture = w.image.clone().or_else(|| prop_image(w.kind));
            let mut kind = w.kind;
            if let Some(path) = picture {
                let Some((tex, aspect)) = c.picture(gl, &path) else { continue };
                // the umbrella's top: just the canopy of the picture
                let crop = if w.kind == 38 { CANOPY } else { [0.0, 0.0, 1.0, 1.0] };
                let aspect = aspect * crop[2] / crop[3];
                c.program.set_vec4(gl, "u_crop", crop);
                // the box in px, to keep the picture to its own shape
                let [x0, y0, x1, y1] = w.rect;
                let (bw, bh) = ((x1 - x0) * 0.5 * pose.half[0] * pose.size[0], (y1 - y0) * 0.5 * pose.half[1] * pose.size[1]);
                c.program.set_vec2(gl, "u_fit", fit(bw, bh, aspect)[0], fit(bw, bh, aspect)[1]);
                unsafe {
                    gl.active_texture(glow::TEXTURE1);
                    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                }
                c.program.set_i32(gl, "u_img", 1);
                kind = 100;
            }
            c.program.set_vec4(gl, "u_sub", w.rect);
            c.program.set_f32(gl, "u_turn", w.turn);
            c.program.set_i32(gl, "u_kind", kind);
            c.program.set_i32(gl, "u_flip", i32::from(w.flip));
            unsafe {
                gl.enable(glow::BLEND);
                gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
                gl.bind_vertex_array(Some(c.vao));
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
                gl.bind_vertex_array(None);
            }
            if kind == 100 {
                unsafe {
                    gl.bind_texture(glow::TEXTURE_2D, None);
                    gl.active_texture(glow::TEXTURE0);
                }
            }
        }
        c.program.set_i32(gl, "u_attach", 0);
    }

    pub(super) fn draw_carrot(&mut self, gl: &glow::Context, info: &FrameInfo, target: &mut dyn ActorTarget) {
        let (w, h) = (info.width.max(1) as f32, info.height.max(1) as f32);
        // (kind, surface box, mirrored, turned by, where)
        let mut props: Vec<_> = self
            .props()
            .into_iter()
            .filter_map(|p| self.placed(p.3).map(|z| (p.0, p.1, p.2, 0.0f32, z, (p.0 == 100).then(super::sign::hug_heart).flatten())))
            .collect();
        // dropped things, turning as they fall
        for l in &self.moods.loose {
            let Some(a) = (if l.owner == 0 { Some(&*self) } else { self.extras.get(l.owner - 1) }) else { continue };
            let Some(z) = self.placed(l.owner) else { continue };
            let x = l.x - a.own_view[0] * self.view_w * 0.5;
            props.push((l.kind, [x - l.size[0] * 0.5, l.y - l.size[1] * 0.5, x + l.size[0] * 0.5, l.y + l.size[1] * 0.5], false, l.rot, z, l.image.clone()));
        }
        // the weather over the whole screen, under everything else
        if let Some(kind) = self.weather_sheet() {
            props.insert(0, (kind, [0.0, 0.0, w, h], false, 0.0, None, None));
        }
        if props.is_empty() {
            return;
        }
        if self.carrot_gl.is_none() {
            match CarrotGl::new(gl) {
                Ok(c) => self.carrot_gl = Some(c),
                Err(e) => {
                    log::warn!("carrot: {e:#}");
                    return;
                }
            }
        }
        let time = self.start.elapsed().as_secs_f32();
        let Some(c) = self.carrot_gl.as_mut() else { return };
        c.program.bind(gl);
        c.program.set_i32(gl, "u_slide_n", 0);
        c.program.set_i32(gl, "u_attach", 0);
        c.program.set_f32(gl, "u_time", time);
        c.program.set_f32(gl, "u_saspect", w / h);
        c.program.set_f32(gl, "c_aspect", w / h);
        c.program.set_f32(gl, "u_opacity", 1.0);
        c.program.set_vec2(gl, "u_hide", 0.0, 0.0);
        for (kind, rect, flip, spin, zoom, image) in props {
            // the weather sheet keeps to its own frame rate
            c.program.bind(gl);
            c.program.set_f32(gl, "u_time", if matches!(kind, 40 | 41) { sheet_frame().1 } else { time });
            // the creature rising out of the ground says how far it's up in its kind
            let (kind, reveal) = if kind >= super::social::FOUND {
                (kind, 1.0)
            } else if kind >= 21000 {
                (21, (kind - 21000) as f32 / 1000.0)
            } else {
                (kind, 1.0)
            };
            let picture = image.or_else(|| prop_image(kind));
            c.program.bind(gl);
            c.program.set_f32(gl, "u_spin", spin);
            if !target.begin(gl, 0, rect) {
                continue;
            }
            let [x0, y0, x1, y1] = rect;
            c.program.bind(gl);
            c.program.set_vec4(gl, "u_rect", [x0 / w * 2.0 - 1.0, 1.0 - y1 / h * 2.0, x1 / w * 2.0 - 1.0, 1.0 - y0 / h * 2.0]);
            c.program.set_vec4(gl, "u_clip", zoom.map_or_else(|| target.clip_xform(), |z| z.clip_xform()));
            c.program.set_f32(gl, "u_reveal", reveal);
            let kind = match picture.and_then(|p| c.picture(gl, &p)) {
                Some((tex, aspect)) => {
                    c.program.set_vec4(gl, "u_crop", [0.0, 0.0, 1.0, 1.0]);
                    let f = fit(x1 - x0, y1 - y0, aspect);
                    c.program.set_vec2(gl, "u_fit", f[0], f[1]);
                    unsafe {
                        gl.active_texture(glow::TEXTURE1);
                        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                        gl.active_texture(glow::TEXTURE0);
                    }
                    c.program.set_i32(gl, "u_img", 1);
                    100
                }
                None if kind >= super::social::FISH => 15,
                None if kind >= super::social::FOUND => super::wear::catalog_entry((kind - super::social::FOUND) as usize).map_or(15, |e| e.3),
                None => kind,
            };
            c.program.set_i32(gl, "u_kind", kind);
            c.program.set_i32(gl, "u_flip", i32::from(flip));
            unsafe {
                gl.enable(glow::BLEND);
                gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
                gl.bind_vertex_array(Some(c.vao));
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
                gl.bind_vertex_array(None);
                gl.disable(glow::BLEND);
            }
            target.end(gl, 0);
        }
    }
}
