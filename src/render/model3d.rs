//! Rigged 3D characters (`.glb` / `.gltf`), e.g. the Blue Archive chibis from
//! <https://github.com/lihaohong6/BlueArchiveModels>.
//!
//! The model is skinned on the GPU and drawn into a small offscreen texture
//! (transparent background, its own depth buffer). The character overlay then
//! shows that texture like any other sprite, so walking, jumping, dragging and
//! patting keep working unchanged: they only move and scale the sprite box.
//!
//! Supported: skins, node TRS animation (step / linear / cubic spline treated as
//! linear), the first morph target (BA models use it to hide unused mouth
//! shapes), base colour textures, opaque / mask / blend materials.

use std::num::NonZeroU32;
use std::path::Path;

use anyhow::{anyhow, bail, Result};
use glam::{Mat4, Quat, Vec3};
use glow::HasContext;
use gltf::animation::{Interpolation, Property, util::ReadOutputs};

use super::gl::{Filter, Program, create_texture};

const MAX_JOINTS: usize = 128;

const MODEL_VS: &str = r#"#version 300 es
precision highp float;
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;
layout(location = 3) in vec4 a_joints;
layout(location = 4) in vec4 a_weights;
layout(location = 5) in vec3 a_morph;

uniform mat4 u_joints[128];
uniform mat4 u_model; // node transform of unskinned meshes
uniform mat4 u_view;
uniform mat4 u_proj;
uniform int u_skinned;
uniform float u_morph;

out vec2 v_uv;
out vec3 v_n;

void main() {
    mat4 m = u_model;
    if (u_skinned == 1) {
        m = a_weights.x * u_joints[int(a_joints.x)]
          + a_weights.y * u_joints[int(a_joints.y)]
          + a_weights.z * u_joints[int(a_joints.z)]
          + a_weights.w * u_joints[int(a_joints.w)];
    }
    v_uv = a_uv;
    v_n = mat3(u_view) * mat3(m) * a_normal;
    gl_Position = u_proj * u_view * m * vec4(a_pos + a_morph * u_morph, 1.0);
}
"#;

const MODEL_FS: &str = r#"#version 300 es
precision highp float;
in vec2 v_uv;
in vec3 v_n;

uniform sampler2D u_tex;
uniform vec4 u_color;
// 0 = opaque, 1 = mask, 2 = blend
uniform int u_alpha_mode;
uniform float u_cutoff;

out vec4 fragColor;

void main() {
    vec4 c = texture(u_tex, v_uv) * u_color;
    if (u_alpha_mode == 1 && c.a < u_cutoff) discard;
    // mostly flat, like the game's toon look; a soft key light from the front
    float l = 1.0;
    if (dot(v_n, v_n) > 0.0) {
        l = 0.82 + 0.18 * max(dot(normalize(v_n), normalize(vec3(-0.3, 0.6, 0.8))), 0.0);
    }
    float a = u_alpha_mode == 2 ? c.a : 1.0;
    // premultiplied, over a transparent background
    fragColor = vec4(c.rgb * l * a, a);
}
"#;

/// Which clip the character should play.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pose {
    Idle,
    Walk,
    Pat,
    Held,
}

#[derive(Clone, Copy)]
struct Trs {
    t: Vec3,
    r: Quat,
    s: Vec3,
    // first morph target weight
    w: f32,
}

struct Track {
    node: usize,
    prop: Property,
    step: bool,
    times: Vec<f32>,
    // `width` floats per key
    values: Vec<f32>,
    width: usize,
}

struct Clip {
    name: String,
    dur: f32,
    tracks: Vec<Track>,
}

struct Skin {
    joints: Vec<usize>,
    inverse_bind: Vec<Mat4>,
}

/// CPU side of a primitive, kept only while fitting the camera at load time.
struct Geometry {
    pos: Vec<Vec3>,
    morph: Vec<Vec3>,
    joints: Vec<[u16; 4]>,
    weights: Vec<[f32; 4]>,
}

struct Prim {
    node: usize,
    skin: Option<usize>,
    vao: glow::VertexArray,
    count: i32,
    tex: Option<usize>,
    color: [f32; 4],
    alpha_mode: i32,
    cutoff: f32,
}

/// Parsed glTF scene without any GL objects.
struct Asset {
    parents: Vec<Option<usize>>,
    // parents before children
    order: Vec<usize>,
    rest: Vec<Trs>,
    skins: Vec<Skin>,
    clips: Vec<Clip>,
}

impl Asset {
    fn parse(doc: &gltf::Document, buffers: &[gltf::buffer::Data]) -> Result<Self> {
        let n = doc.nodes().count();
        let mut parents = vec![None; n];
        let mut rest = Vec::with_capacity(n);
        for node in doc.nodes() {
            for c in node.children() {
                parents[c.index()] = Some(node.index());
            }
            let (t, r, s) = node.transform().decomposed();
            let w = node
                .weights()
                .or_else(|| node.mesh().and_then(|m| m.weights()))
                .and_then(|w| w.first().copied())
                .unwrap_or(0.0);
            rest.push(Trs { t: t.into(), r: Quat::from_array(r), s: s.into(), w });
        }
        let mut order = Vec::with_capacity(n);
        let mut stack: Vec<usize> = (0..n).filter(|&i| parents[i].is_none()).collect();
        while let Some(i) = stack.pop() {
            order.push(i);
            stack.extend(doc.nodes().nth(i).into_iter().flat_map(|nd| nd.children()).map(|c| c.index()));
        }

        let get = |b: gltf::Buffer| buffers.get(b.index()).map(|d| &d.0[..]);
        let mut skins = Vec::new();
        for skin in doc.skins() {
            let joints: Vec<usize> = skin.joints().map(|j| j.index()).collect();
            if joints.len() > MAX_JOINTS {
                bail!("skin has {} joints (max {MAX_JOINTS})", joints.len());
            }
            let inverse_bind = match skin.reader(get).read_inverse_bind_matrices() {
                Some(m) => m.map(|m| Mat4::from_cols_array_2d(&m)).collect(),
                None => vec![Mat4::IDENTITY; joints.len()],
            };
            skins.push(Skin { joints, inverse_bind });
        }

        let mut clips = Vec::new();
        for anim in doc.animations() {
            let mut tracks = Vec::new();
            let mut dur: f32 = 0.0;
            for ch in anim.channels() {
                let r = ch.reader(get);
                let Some(times) = r.read_inputs().map(|i| i.collect::<Vec<f32>>()) else { continue };
                let values: Vec<f32> = match r.read_outputs() {
                    Some(ReadOutputs::Translations(v)) | Some(ReadOutputs::Scales(v)) => v.flatten().collect(),
                    Some(ReadOutputs::Rotations(v)) => v.into_f32().flatten().collect(),
                    Some(ReadOutputs::MorphTargetWeights(v)) => v.into_f32().collect(),
                    None => continue,
                };
                if times.is_empty() || values.is_empty() {
                    continue;
                }
                let mut width = values.len() / times.len();
                let cubic = ch.sampler().interpolation() == Interpolation::CubicSpline;
                // cubic spline keys are (in-tangent, value, out-tangent): keep the value
                let values = if cubic {
                    width /= 3;
                    values.chunks(width * 3).flat_map(|k| k[width..width * 2].to_vec()).collect()
                } else {
                    values
                };
                if width == 0 {
                    continue;
                }
                dur = dur.max(*times.last().unwrap());
                tracks.push(Track {
                    node: ch.target().node().index(),
                    prop: ch.target().property(),
                    step: ch.sampler().interpolation() == Interpolation::Step,
                    times,
                    values,
                    width,
                });
            }
            clips.push(Clip {
                name: anim.name().unwrap_or("").to_string(),
                dur: dur.max(0.01),
                tracks,
            });
        }
        Ok(Self { parents, order, rest, skins, clips })
    }

    /// Local transforms of every node for `clip` at time `t` (looped).
    fn sample(&self, clip: Option<usize>, t: f32, out: &mut Vec<Trs>) {
        out.clear();
        out.extend_from_slice(&self.rest);
        let Some(clip) = clip.and_then(|c| self.clips.get(c)) else { return };
        let t = t.rem_euclid(clip.dur);
        for tr in &clip.tracks {
            let k = tr.times.partition_point(|&x| x <= t);
            let (a, b, f) = if k == 0 {
                (0, 0, 0.0)
            } else if k >= tr.times.len() {
                (k - 1, k - 1, 0.0)
            } else {
                let (t0, t1) = (tr.times[k - 1], tr.times[k]);
                let f = if tr.step { 0.0 } else { (t - t0) / (t1 - t0).max(1e-6) };
                (k - 1, k, f)
            };
            let w = tr.width;
            let va = &tr.values[a * w..a * w + w];
            let vb = &tr.values[b * w..b * w + w];
            let Some(node) = out.get_mut(tr.node) else { continue };
            match tr.prop {
                Property::Translation if w >= 3 => {
                    node.t = Vec3::from_slice(va).lerp(Vec3::from_slice(vb), f);
                }
                Property::Scale if w >= 3 => {
                    node.s = Vec3::from_slice(va).lerp(Vec3::from_slice(vb), f);
                }
                Property::Rotation if w >= 4 => {
                    let qa = Quat::from_slice(va).normalize();
                    let qb = Quat::from_slice(vb).normalize();
                    node.r = qa.slerp(qb, f);
                }
                Property::MorphTargetWeights => node.w = va[0] + (vb[0] - va[0]) * f,
                _ => {}
            }
        }
    }

    fn globals(&self, local: &[Trs], out: &mut Vec<Mat4>) {
        out.clear();
        out.resize(local.len(), Mat4::IDENTITY);
        for &i in &self.order {
            let l = local[i];
            let m = Mat4::from_scale_rotation_translation(l.s, l.r, l.t);
            out[i] = match self.parents[i] {
                Some(p) => out[p] * m,
                None => m,
            };
        }
    }

    fn joint_mats(&self, skin: usize, globals: &[Mat4], out: &mut Vec<Mat4>) {
        let s = &self.skins[skin];
        out.clear();
        out.extend(s.joints.iter().zip(&s.inverse_bind).map(|(&j, ib)| globals[j] * *ib));
    }
}

pub struct Model3d {
    asset: Asset,
    program: Program,
    prims: Vec<Prim>,
    textures: Vec<glow::Texture>,
    white: glow::Texture,
    // clip per pose: idle, walk, pat, held
    clips: [Option<usize>; 4],
    pose: Pose,
    time: f32,
    // crossfade from the previous clip: (clip, its time, remaining fade)
    fade: Option<(Option<usize>, f32, f32)>,
    view: Mat4,
    proj: Mat4,
    fbo: glow::Framebuffer,
    color: glow::Texture,
    #[allow(dead_code)]
    depth: glow::Renderbuffer,
    size: (u32, u32),
    // scratch buffers reused every frame
    local: Vec<Trs>,
    local_b: Vec<Trs>,
    globals: Vec<Mat4>,
    joints: Vec<Mat4>,
}

const FADE: f32 = 0.25;

/// Three-quarter view turned toward the walking direction (screen right;
/// the overlay mirrors the texture for the other way), seen a bit from above.
const YAW: f32 = 0.6;
const PITCH: f32 = 0.2;

pub fn is_model_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("glb") || e.eq_ignore_ascii_case("gltf"))
}

impl Model3d {
    /// Load a model. `height` is the offscreen texture height in pixels.
    pub fn load(gl: &glow::Context, path: &Path, height: u32) -> Result<Self> {
        let (doc, buffers, images) = gltf::import(path)?;
        let asset = Asset::parse(&doc, &buffers)?;

        let pick = |keys: &[&str]| {
            keys.iter().find_map(|k| {
                asset.clips.iter().position(|c| c.name.eq_ignore_ascii_case(k)).or_else(|| {
                    let k = k.to_ascii_lowercase();
                    asset.clips.iter().position(|c| c.name.to_ascii_lowercase().contains(&k))
                })
            })
        };
        let idle = pick(&["Cafe_Idle", "Normal_Idle", "idle"]).or((!asset.clips.is_empty()).then_some(0));
        let walk = pick(&["Cafe_Walk", "walk", "run", "move"]).or(idle);
        let pat = pick(&["Cafe_Reaction", "reaction", "react"]).or(idle);
        let held = pick(&["Formation_Pickup", "pickup", "pick"]).or(pat);
        let clips = [idle, walk, pat, held];
        log::info!(
            "model {}: clips {:?}, using idle {:?} walk {:?} pat {:?} held {:?}",
            path.display(),
            asset.clips.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            idle.map(|i| &asset.clips[i].name),
            walk.map(|i| &asset.clips[i].name),
            pat.map(|i| &asset.clips[i].name),
            held.map(|i| &asset.clips[i].name),
        );

        let get = |b: gltf::Buffer| buffers.get(b.index()).map(|d| &d.0[..]);
        let mut prims = Vec::new();
        let mut geometry = Vec::new();
        for node in doc.nodes() {
            let Some(mesh) = node.mesh() else { continue };
            let skin = node.skin().map(|s| s.index());
            for p in mesh.primitives() {
                if p.mode() != gltf::mesh::Mode::Triangles {
                    continue;
                }
                let r = p.reader(get);
                let Some(pos) = r.read_positions() else { continue };
                let pos: Vec<Vec3> = pos.map(Vec3::from).collect();
                let n = pos.len();
                let normals: Vec<[f32; 3]> = r.read_normals().map_or(vec![[0.0; 3]; n], |v| v.collect());
                let uvs: Vec<[f32; 2]> = r.read_tex_coords(0).map_or(vec![[0.0; 2]; n], |v| v.into_f32().collect());
                let joints: Vec<[u16; 4]> = r.read_joints(0).map_or(vec![[0; 4]; n], |v| v.into_u16().collect());
                let weights: Vec<[f32; 4]> = r.read_weights(0).map_or(vec![[0.0; 4]; n], |v| v.into_f32().collect());
                let morph: Vec<Vec3> = r
                    .read_morph_targets()
                    .next()
                    .and_then(|(p, _, _)| p)
                    .map_or(vec![Vec3::ZERO; n], |v| v.map(Vec3::from).collect());
                let indices: Vec<u32> = r.read_indices().map_or((0..n as u32).collect(), |i| i.into_u32().collect());
                if normals.len() != n || uvs.len() != n || joints.len() != n || weights.len() != n || morph.len() != n {
                    continue;
                }

                let mut verts = Vec::with_capacity(n * 19);
                for i in 0..n {
                    verts.extend_from_slice(&pos[i].to_array());
                    verts.extend_from_slice(&normals[i]);
                    verts.extend_from_slice(&uvs[i]);
                    verts.extend(joints[i].map(f32::from));
                    verts.extend_from_slice(&weights[i]);
                    verts.extend_from_slice(&morph[i].to_array());
                }
                let vao = upload(gl, &verts, &indices)?;

                let mat = p.material();
                let pbr = mat.pbr_metallic_roughness();
                let (alpha_mode, cutoff) = match mat.alpha_mode() {
                    gltf::material::AlphaMode::Opaque => (0, 0.0),
                    gltf::material::AlphaMode::Mask => (1, mat.alpha_cutoff().unwrap_or(0.5)),
                    gltf::material::AlphaMode::Blend => (2, 0.0),
                };
                prims.push(Prim {
                    node: node.index(),
                    skin,
                    vao,
                    count: indices.len() as i32,
                    tex: pbr.base_color_texture().map(|t| t.texture().source().index()),
                    color: pbr.base_color_factor(),
                    alpha_mode,
                    cutoff,
                });
                geometry.push(Geometry { pos, morph, joints, weights });
            }
        }
        if prims.is_empty() {
            bail!("no triangle meshes in {}", path.display());
        }

        let mut textures = Vec::with_capacity(images.len());
        for img in &images {
            let rgba = to_rgba(img);
            let tex = create_texture(gl, img.width, img.height, &rgba, Filter::Linear, true)?;
            unsafe {
                gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::REPEAT as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::REPEAT as i32);
                gl.bind_texture(glow::TEXTURE_2D, None);
            }
            textures.push(tex);
        }
        let white = create_texture(gl, 1, 1, &[255; 4], Filter::Nearest, false)?;

        // fit an orthographic camera around every pose the character can take
        let (view, lo, hi) = fit_camera(&asset, &prims, &geometry, &clips);
        let pad = (hi - lo) * Vec3::new(0.04, 0.03, 0.05);
        let (lo, hi) = (lo - pad, hi + pad);
        // orthographic; y flipped since the overlay samples sprites top row
        // first, and nearer (larger view z) gets the smaller depth
        let d = hi - lo;
        let proj = Mat4::from_scale(Vec3::new(2.0 / d.x, -2.0 / d.y, -2.0 / d.z))
            * Mat4::from_translation(-(lo + hi) * 0.5);
        let aspect = ((hi.x - lo.x) / (hi.y - lo.y)).clamp(0.2, 5.0);
        let size = (((height as f32 * aspect).round() as u32).max(16), height.max(16));

        let (fbo, color, depth) = create_target(gl, size.0, size.1)?;
        Ok(Self {
            asset,
            program: Program::new(gl, MODEL_VS, MODEL_FS)?,
            prims,
            textures,
            white,
            clips,
            pose: Pose::Idle,
            time: 0.0,
            fade: None,
            view,
            proj,
            fbo,
            color,
            depth,
            size,
            local: Vec::new(),
            local_b: Vec::new(),
            globals: Vec::new(),
            joints: Vec::new(),
        })
    }

    /// The offscreen texture (premultiplied alpha) and its size.
    pub fn texture(&self) -> (glow::Texture, u32, u32) {
        (self.color, self.size.0, self.size.1)
    }

    /// Advance the animation and redraw the offscreen texture.
    /// Leaves the framebuffer, viewport and clear colour as it found them.
    pub fn draw(&mut self, gl: &glow::Context, dt: f32, pose: Pose) {
        let clip_of = |p: Pose| self.clips[p as usize];
        if pose != self.pose {
            if clip_of(pose) != clip_of(self.pose) {
                self.fade = Some((clip_of(self.pose), self.time, FADE));
                self.time = 0.0;
            }
            self.pose = pose;
        }
        self.time += dt;

        self.asset.sample(clip_of(self.pose), self.time, &mut self.local);
        if let Some((clip, t, left)) = &mut self.fade {
            *t += dt;
            *left -= dt;
            if *left <= 0.0 {
                self.fade = None;
            } else {
                let k = *left / FADE;
                self.asset.sample(*clip, *t, &mut self.local_b);
                for (a, b) in self.local.iter_mut().zip(&self.local_b) {
                    a.t = a.t.lerp(b.t, k);
                    a.r = a.r.slerp(b.r, k);
                    a.s = a.s.lerp(b.s, k);
                }
            }
        }
        self.asset.globals(&self.local, &mut self.globals);

        unsafe {
            let prev_fbo = gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING);
            let mut vp = [0; 4];
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut vp);
            let mut clear = [0.0; 4];
            gl.get_parameter_f32_slice(glow::COLOR_CLEAR_VALUE, &mut clear);

            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.viewport(0, 0, self.size.0 as i32, self.size.1 as i32);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.disable(glow::CULL_FACE);

            self.program.bind(gl);
            let view = self.view.to_cols_array();
            let proj = self.proj.to_cols_array();
            let loc = self.program.uniform(gl, "u_view");
            gl.uniform_matrix_4_f32_slice(loc.as_ref(), false, &view);
            let loc = self.program.uniform(gl, "u_proj");
            gl.uniform_matrix_4_f32_slice(loc.as_ref(), false, &proj);
            self.program.set_i32(gl, "u_tex", 0);
            gl.active_texture(glow::TEXTURE0);

            // opaque and masked first, blended last without depth writes
            let mut bound_skin = None;
            for pass in [false, true] {
                if pass {
                    gl.enable(glow::BLEND);
                    gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
                    gl.depth_mask(false);
                }
                for i in 0..self.prims.len() {
                    let p = &self.prims[i];
                    if (p.alpha_mode == 2) != pass {
                        continue;
                    }
                    let morph = self.local[p.node].w;
                    if let Some(s) = p.skin {
                        if bound_skin != Some(s) {
                            self.asset.joint_mats(s, &self.globals, &mut self.joints);
                            let flat: Vec<f32> = self.joints.iter().flat_map(|m| m.to_cols_array()).collect();
                            let loc = self.program.uniform(gl, "u_joints[0]");
                            gl.uniform_matrix_4_f32_slice(loc.as_ref(), false, &flat);
                            bound_skin = Some(s);
                        }
                    } else {
                        let m = self.globals[p.node].to_cols_array();
                        let loc = self.program.uniform(gl, "u_model");
                        gl.uniform_matrix_4_f32_slice(loc.as_ref(), false, &m);
                    }
                    let p = &self.prims[i];
                    let tex = p.tex.and_then(|t| self.textures.get(t).copied()).unwrap_or(self.white);
                    let (skinned, color, mode, cutoff, vao, count) =
                        (p.skin.is_some(), p.color, p.alpha_mode, p.cutoff, p.vao, p.count);
                    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                    self.program.set_i32(gl, "u_skinned", i32::from(skinned));
                    self.program.set_f32(gl, "u_morph", morph);
                    self.program.set_vec4(gl, "u_color", color);
                    self.program.set_i32(gl, "u_alpha_mode", mode);
                    self.program.set_f32(gl, "u_cutoff", cutoff);
                    gl.bind_vertex_array(Some(vao));
                    gl.draw_elements(glow::TRIANGLES, count, glow::UNSIGNED_INT, 0);
                }
            }

            gl.depth_mask(true);
            gl.disable(glow::BLEND);
            gl.disable(glow::DEPTH_TEST);
            gl.bind_vertex_array(None);
            gl.bind_texture(glow::TEXTURE_2D, None);
            let prev = NonZeroU32::new(prev_fbo as u32).map(glow::NativeFramebuffer);
            gl.bind_framebuffer(glow::FRAMEBUFFER, prev);
            gl.viewport(vp[0], vp[1], vp[2], vp[3]);
            gl.clear_color(clear[0], clear[1], clear[2], clear[3]);
        }
    }
}

/// View rotation plus view-space bounds of the model over sampled frames of
/// every clip in use, skinned on the CPU.
fn fit_camera(
    asset: &Asset,
    prims: &[Prim],
    geometry: &[Geometry],
    clips: &[Option<usize>; 4],
) -> (Mat4, Vec3, Vec3) {
    let mut local = Vec::new();
    let mut globals = Vec::new();
    let mut joints = Vec::new();
    let mut points = Vec::new();
    let mut used: Vec<Option<usize>> = clips.to_vec();
    used.dedup();
    if used.is_empty() {
        used.push(None);
    }
    for &clip in &used {
        let dur = clip.map_or(0.0, |c| asset.clips[c].dur);
        for k in 0..8 {
            asset.sample(clip, dur * k as f32 / 8.0, &mut local);
            asset.globals(&local, &mut globals);
            for (p, g) in prims.iter().zip(geometry) {
                let w = local[p.node].w;
                // hidden shapes (BA mouths) collapse under their morph target
                if w > 0.99 && g.morph.iter().any(|m| *m != Vec3::ZERO) {
                    continue;
                }
                if let Some(s) = p.skin {
                    asset.joint_mats(s, &globals, &mut joints);
                }
                for (i, v) in g.pos.iter().enumerate().step_by(3) {
                    let v = *v + g.morph[i] * w;
                    let m = match p.skin {
                        Some(_) => {
                            let (j, wt) = (g.joints[i], g.weights[i]);
                            (0..4)
                                .map(|n| *joints.get(j[n] as usize).unwrap_or(&Mat4::IDENTITY) * wt[n])
                                .fold(Mat4::ZERO, |a, b| a + b)
                        }
                        None => globals[p.node],
                    };
                    points.push(m.transform_point3(v));
                }
            }
        }
    }
    // glTF models face +Z: turn them toward screen right, then tilt down
    let view = Mat4::from_rotation_x(PITCH) * Mat4::from_rotation_y(YAW);
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for p in &points {
        let v = view.transform_point3(*p);
        lo = lo.min(v);
        hi = hi.max(v);
    }
    if points.is_empty() {
        lo = Vec3::splat(-1.0);
        hi = Vec3::splat(1.0);
    }
    (view, lo, hi)
}

fn to_rgba(img: &gltf::image::Data) -> Vec<u8> {
    use gltf::image::Format;
    let px = &img.pixels;
    match img.format {
        Format::R8G8B8A8 => px.clone(),
        Format::R8G8B8 => px.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        Format::R8G8 => px.chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        Format::R8 => px.iter().flat_map(|&c| [c, c, c, 255]).collect(),
        // 16-bit / float images: take the high byte of each channel
        Format::R16G16B16A16 => px.chunks(2).map(|c| c[1]).collect(),
        Format::R16G16B16 => px.chunks(6).flat_map(|c| [c[1], c[3], c[5], 255]).collect(),
        _ => vec![255; (img.width * img.height * 4) as usize],
    }
}

fn upload(gl: &glow::Context, verts: &[f32], indices: &[u32]) -> Result<glow::VertexArray> {
    unsafe {
        let vao = gl.create_vertex_array().map_err(|e| anyhow!(e))?;
        let vbo = gl.create_buffer().map_err(|e| anyhow!(e))?;
        let ibo = gl.create_buffer().map_err(|e| anyhow!(e))?;
        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, super::gl::bytemuck_cast(verts), glow::STATIC_DRAW);
        let idx: Vec<u8> = indices.iter().flat_map(|i| i.to_ne_bytes()).collect();
        gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ibo));
        gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, &idx, glow::STATIC_DRAW);
        let stride = 19 * 4;
        let mut off = 0;
        for (loc, n) in [(0, 3), (1, 3), (2, 2), (3, 4), (4, 4), (5, 3)] {
            gl.enable_vertex_attrib_array(loc);
            gl.vertex_attrib_pointer_f32(loc, n, glow::FLOAT, false, stride, off);
            off += n * 4;
        }
        gl.bind_vertex_array(None);
        Ok(vao)
    }
}

fn create_target(gl: &glow::Context, w: u32, h: u32) -> Result<(glow::Framebuffer, glow::Texture, glow::Renderbuffer)> {
    unsafe {
        let color = create_texture(gl, w, h, &vec![0; (w * h * 4) as usize], Filter::Linear, false)?;
        let depth = gl.create_renderbuffer().map_err(|e| anyhow!(e))?;
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
        gl.renderbuffer_storage(glow::RENDERBUFFER, glow::DEPTH_COMPONENT24, w as i32, h as i32);
        gl.bind_renderbuffer(glow::RENDERBUFFER, None);

        let prev = gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING);
        let fbo = gl.create_framebuffer().map_err(|e| anyhow!(e))?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(color), 0);
        gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::DEPTH_ATTACHMENT, glow::RENDERBUFFER, Some(depth));
        let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        gl.bind_framebuffer(glow::FRAMEBUFFER, NonZeroU32::new(prev as u32).map(glow::NativeFramebuffer));
        if status != glow::FRAMEBUFFER_COMPLETE {
            bail!("model framebuffer incomplete: {status:#x}");
        }
        Ok((fbo, color, depth))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ANIMA_TEST_MODEL=/path/model.glb cargo test model3d -- --nocapture`
    #[test]
    fn parse_model() {
        let Ok(path) = std::env::var("ANIMA_TEST_MODEL") else { return };
        let (doc, buffers, _) = gltf::import(&path).unwrap();
        let asset = Asset::parse(&doc, &buffers).unwrap();
        for c in &asset.clips {
            println!("clip {:?} {:.2}s {} tracks", c.name, c.dur, c.tracks.len());
        }
        let get = |b: gltf::Buffer| buffers.get(b.index()).map(|d| &d.0[..]);
        let mut local = Vec::new();
        let mut globals = Vec::new();
        let mut joints = Vec::new();
        for clip in [None, Some(1), Some(2)] {
            asset.sample(clip, 0.3, &mut local);
            asset.globals(&local, &mut globals);
            let (mut face, mut nf, mut all, mut na) = (Vec3::ZERO, 0.0f32, Vec3::ZERO, 0.0f32);
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for node in doc.nodes() {
                let Some(mesh) = node.mesh() else { continue };
                for p in mesh.primitives() {
                    let r = p.reader(get);
                    let pos: Vec<[f32; 3]> = r.read_positions().unwrap().collect();
                    let js: Vec<[u16; 4]> = r.read_joints(0).map_or(vec![[0; 4]; pos.len()], |v| v.into_u16().collect());
                    let ws: Vec<[f32; 4]> = r.read_weights(0).map_or(vec![[0.0; 4]; pos.len()], |v| v.into_f32().collect());
                    if let Some(s) = node.skin() {
                        asset.joint_mats(s.index(), &globals, &mut joints);
                    }
                    let is_face = node.name().unwrap_or("").starts_with("Mouth");
                    for i in 0..pos.len() {
                        let m = if node.skin().is_some() {
                            (0..4).map(|n| joints[js[i][n] as usize] * ws[i][n]).fold(Mat4::ZERO, |a, b| a + b)
                        } else {
                            globals[node.index()]
                        };
                        let v = m.transform_point3(Vec3::from(pos[i]));
                        if is_face {
                            face += v;
                            nf += 1.0;
                        }
                        all += v;
                        na += 1.0;
                        lo = lo.min(v);
                        hi = hi.max(v);
                    }
                }
            }
            println!("clip {clip:?}: centre {:?} face {:?} bounds {lo:?}..{hi:?}", all / na, face / nf.max(1.0));
        }
    }

    /// Headless render through a surfaceless EGL context:
    /// `ANIMA_TEST_MODEL=... ANIMA_TEST_OUT=/dir cargo test model3d -- --nocapture`
    #[test]
    fn render_model() {
        use glutin::config::{ConfigSurfaceTypes, ConfigTemplateBuilder};
        use glutin::context::{ContextApi, ContextAttributesBuilder, Version};
        use glutin::display::{GlDisplay, GetGlDisplay};
        let (Ok(path), Ok(out)) = (std::env::var("ANIMA_TEST_MODEL"), std::env::var("ANIMA_TEST_OUT")) else { return };
        let device = glutin::api::egl::device::Device::query_devices().unwrap().next().unwrap();
        let display = unsafe { glutin::api::egl::display::Display::with_device(&device, None) }.unwrap();
        let config = unsafe { display.find_configs(ConfigTemplateBuilder::new().with_surface_type(ConfigSurfaceTypes::empty()).build()) }.unwrap().next().unwrap();
        let attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(Some(Version::new(3, 0))))
            .build(None);
        let ctx = unsafe { config.display().create_context(&config, &attrs) }.unwrap();
        let _ctx = ctx.make_current_surfaceless().unwrap();
        let gl = unsafe {
            glow::Context::from_loader_function_cstr(|s| display.get_proc_address(s) as *const _)
        };
        let mut model = Model3d::load(&gl, Path::new(&path), 512).unwrap();
        let (_, w, h) = model.texture();
        for (pose, t) in [(Pose::Idle, 1.0), (Pose::Walk, 0.4), (Pose::Pat, 0.8), (Pose::Held, 1.5)] {
            for _ in 0..(t / 0.05) as usize {
                model.draw(&gl, 0.05, pose);
            }
            let mut px = vec![0u8; (w * h * 4) as usize];
            unsafe {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(model.fbo));
                gl.read_pixels(0, 0, w as i32, h as i32, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut px)));
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            }
            // the projection is y-flipped, so the first row read back is the top
            let img = image::RgbaImage::from_raw(w, h, px).unwrap();
            img.save(format!("{out}/{pose:?}.png")).unwrap();
        }
    }
}
