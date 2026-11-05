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
use crate::backend::envlight::Light;
use glam::{Mat4, Quat, Vec3};
use glow::HasContext;
use gltf::animation::{Interpolation, Property, util::ReadOutputs};

use super::gl::{Filter, Program, create_texture};

const MAX_JOINTS: usize = 160;

const MODEL_VS: &str = r#"#version 300 es
precision highp float;
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;
layout(location = 3) in vec4 a_joints;
layout(location = 4) in vec4 a_weights;
layout(location = 5) in vec3 a_morph;

uniform mat4 u_joints[160];
uniform mat4 u_model; // node transform of unskinned meshes
uniform mat4 u_view;
uniform mat4 u_proj;
uniform int u_skinned;
uniform float u_morph;
// outline pass: how far to push the hull out along the normal, clip units per pixel
uniform vec2 u_outline;

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
    vec4 p = u_proj * u_view * m * vec4(a_pos + a_morph * u_morph, 1.0);
    if (u_outline.x > 0.0) {
        // constant width on screen whatever the depth
        vec2 d = (mat3(u_proj) * v_n).xy;
        if (dot(d, d) > 0.0) p.xy += normalize(d) * u_outline * p.w;
    }
    gl_Position = p;
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
uniform int u_is_outline;
// environment: shadow and light tints (brightest channel 1), exposure, and
// the light direction in view space
uniform vec3 u_ambient;
uniform vec3 u_key;
uniform float u_exposure;
uniform vec3 u_light;

out vec4 fragColor;

void main() {
    vec4 c = texture(u_tex, v_uv) * u_color;
    if (u_alpha_mode == 1 && c.a < u_cutoff) discard;
    if (u_is_outline == 1) {
        // ink: a dark, slightly purple version of the surface colour
        fragColor = vec4(c.rgb * vec3(0.30, 0.26, 0.34), 1.0);
        return;
    }
    // cel shading like the game: a hard two-tone step with a cool purple
    // shadow, plus a faint rim light on the silhouette; the surroundings tint
    // the shadow, the brightest part of the screen tints the light
    vec3 shadow = vec3(0.78, 0.72, 0.88) * mix(vec3(1.0), u_ambient, 0.45);
    vec3 light = mix(vec3(1.0), u_key, 0.3);
    vec3 shade = light;
    if (dot(v_n, v_n) > 0.0) {
        vec3 n = normalize(v_n);
        float ndl = dot(n, u_light);
        float lit = smoothstep(0.02, 0.10, ndl);
        shade = mix(shadow, light, lit);
        float rim = pow(1.0 - max(n.z, 0.0), 4.0) * lit;
        shade += rim * 0.25 * light;
    }
    shade *= u_exposure;
    float a = u_alpha_mode == 2 ? c.a : 1.0;
    // premultiplied, over a transparent background
    fragColor = vec4(c.rgb * shade * a, a);
}
"#;

/// Ink outline width in texture pixels.
const OUTLINE_PX: f32 = 1.6;

/// Which clip the character should play.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pose {
    Idle,
    Walk,
    /// chasing the cursor
    Run,
    Pat,
    Held,
    /// thrown, on the way down
    Fall,
}

/// The model's clips by role. Extras (idle variants, gestures, run, panic)
/// are only used where they stay inside the framing of the basic ones.
#[derive(Default)]
struct Roles {
    /// looping idles
    idle: Vec<usize>,
    walk: Option<usize>,
    run: Option<usize>,
    pat: Vec<usize>,
    held: Option<usize>,
    fall: Option<usize>,
    /// one-shot gestures played now and then while idle, each with an
    /// optional follow-up (victory pose, then its ending)
    flourish: Vec<(usize, Option<usize>)>,
}

/// What the current clip does when it reaches its end.
#[derive(Clone, Copy, PartialEq)]
enum Playing {
    Loop,
    /// play once, then `then` once, then back to idling
    Once { then: Option<usize> },
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
    // mouth shape nodes, and the one kept visible
    mouths: Vec<usize>,
    smile: Option<usize>,
    // smoothed environment light: ambient tint, key tint, exposure, direction
    light: [f32; 10],
    program: Program,
    prims: Vec<Prim>,
    textures: Vec<glow::Texture>,
    white: glow::Texture,
    roles: Roles,
    pose: Pose,
    clip: Option<usize>,
    playing: Playing,
    time: f32,
    // idle seconds until the next gesture
    next_flourish: f32,
    seed: u32,
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

/// Gestures played while idling (start, optional follow-up), if the model has
/// them and they fit its framing.
const FLOURISHES: [(&str, Option<&str>); 8] = [
    ("Victory_Start", Some("Victory_End")),
    ("Victory_Start_Random", Some("Victory_End_Random")),
    ("Public01", None),
    ("Public01_globefish", None),
    ("Normal_Callsign", None),
    ("Move_Callsign", None),
    ("Stand_Callsign", None),
    ("Exs", None),
];

/// BA mouth shapes, happiest first: open grin, open smile, closed smiles.
const HAPPY_MOUTHS: [&str; 6] = ["Mouth_701", "Mouth_703", "Mouth_700", "Mouth_705", "Mouth_704", "Mouth_405"];

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

        // basic roles fall back to anything with a similar name
        let pick = |keys: &[&str]| {
            keys.iter().find_map(|k| {
                asset.clips.iter().position(|c| c.name.eq_ignore_ascii_case(k)).or_else(|| {
                    let k = k.to_ascii_lowercase();
                    asset.clips.iter().position(|c| c.name.to_ascii_lowercase().contains(&k))
                })
            })
        };
        let exact = |k: &str| asset.clips.iter().position(|c| c.name.eq_ignore_ascii_case(k));
        let idle = pick(&["Cafe_Idle", "Normal_Idle", "idle"]).or((!asset.clips.is_empty()).then_some(0));
        let walk = pick(&["Cafe_Walk", "Move_Ing", "walk", "run", "move_ing", "move"]).or(idle);
        let pat = pick(&["Cafe_Reaction", "reaction", "react"]).or(idle);
        let held = pick(&["Formation_Pickup", "pickup", "pick"]).or(pat);
        let basic = [idle, walk, pat, held];

        let get = |b: gltf::Buffer| buffers.get(b.index()).map(|d| &d.0[..]);
        let mut prims = Vec::new();
        let mut geometry = Vec::new();
        for node in doc.nodes() {
            let Some(mesh) = node.mesh() else { continue };
            // a desktop pet doesn't carry guns or shields around
            let name = format!("{} {}", node.name().unwrap_or(""), mesh.name().unwrap_or("")).to_ascii_lowercase();
            if name.contains("weapon") || name.contains("shield") {
                continue;
            }
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

        // fit an orthographic camera around every pose the character can
        // take; extra clips only count (and get used) if they stay close to
        // the basic ones, so no character shrinks or gets cut off for them
        let view = Mat4::from_rotation_x(PITCH) * Mat4::from_rotation_y(YAW);
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for clip in basic.iter().copied().collect::<std::collections::BTreeSet<_>>() {
            let (a, b) = clip_bounds(&asset, &prims, &geometry, view, clip);
            (lo, hi) = (lo.min(a), hi.max(b));
        }
        if lo.x > hi.x {
            (lo, hi) = (Vec3::splat(-1.0), Vec3::splat(1.0));
        }
        let slack = (hi - lo) * 0.15;
        let (lim_lo, lim_hi) = (lo - slack, hi + slack);
        let mut fits = |clip: Option<usize>| {
            let Some(c) = clip else { return None };
            let (a, b) = clip_bounds(&asset, &prims, &geometry, view, Some(c));
            let ok = a.x >= lim_lo.x && a.y >= lim_lo.y && b.x <= lim_hi.x && b.y <= lim_hi.y;
            if ok {
                (lo, hi) = (lo.min(a), hi.max(b));
            }
            ok.then_some(c)
        };
        let mut roles = Roles {
            walk,
            held,
            pat: pat.into_iter().collect(),
            idle: idle.into_iter().collect(),
            ..Default::default()
        };
        roles.idle.extend(["Formation_Idle", "Rearrange_Idle"].iter().filter_map(|k| fits(exact(k))));
        roles.pat.extend(["Rearrange_Reaction"].iter().filter_map(|k| fits(exact(k))));
        roles.run = ["Move_Ing", "Move_Ing01"].iter().find_map(|k| fits(exact(k)));
        roles.fall = ["Vital_Panic", "Vital_Panic01"].iter().find_map(|k| fits(exact(k)));
        for (start, end) in FLOURISHES {
            if let Some(s) = fits(exact(start)) {
                let then = end.and_then(|e| fits(exact(e)));
                roles.flourish.push((s, then));
            }
        }
        roles.idle.dedup();
        roles.pat.dedup();
        let names = |v: &[usize]| v.iter().map(|&i| asset.clips[i].name.as_str()).collect::<Vec<_>>();
        log::info!(
            "model {}: idle {:?} walk {:?} run {:?} pat {:?} held {:?} fall {:?} gestures {:?}",
            path.display(),
            names(&roles.idle),
            roles.walk.map(|i| &asset.clips[i].name),
            roles.run.map(|i| &asset.clips[i].name),
            names(&roles.pat),
            roles.held.map(|i| &asset.clips[i].name),
            roles.fall.map(|i| &asset.clips[i].name),
            roles.flourish.iter().map(|f| &asset.clips[f.0].name).collect::<Vec<_>>(),
        );
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
        // BA faces: one mesh per mouth shape, shown by morph weight 0. Keep
        // the widest grin the model has instead of what the clips switch to.
        let mouths: Vec<(usize, String)> = doc
            .nodes()
            .filter_map(|n| Some((n.index(), n.name()?.to_string())))
            .filter(|(_, name)| name.starts_with("Mouth_"))
            .collect();
        let smile = HAPPY_MOUTHS
            .iter()
            .find_map(|want| mouths.iter().find(|(_, name)| name == want).map(|(i, _)| *i));
        let mouths = if smile.is_some() { mouths.into_iter().map(|(i, _)| i).collect() } else { Vec::new() };

        Ok(Self {
            asset,
            mouths,
            smile,
            light: [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, -0.45, 0.55, 0.7],
            program: Program::new(gl, MODEL_VS, MODEL_FS)?,
            prims,
            textures,
            white,
            clip: roles.idle.first().copied(),
            roles,
            pose: Pose::Idle,
            playing: Playing::Loop,
            time: 0.0,
            next_flourish: 0.0,
            seed: path.as_os_str().len() as u32 ^ (std::process::id() << 3) ^ 0x9e37_79b9,
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

    fn rand(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    fn any(&mut self, v: &[usize]) -> Option<usize> {
        (!v.is_empty()).then(|| v[((self.rand() * v.len() as f32) as usize).min(v.len() - 1)])
    }

    fn clip_for(&mut self, pose: Pose) -> Option<usize> {
        let r = &self.roles;
        let (idle, pat) = (r.idle.clone(), r.pat.clone());
        match pose {
            Pose::Idle => self.any(&idle),
            Pose::Walk => self.roles.walk,
            Pose::Run => self.roles.run.or(self.roles.walk),
            Pose::Pat => self.any(&pat),
            Pose::Held => self.roles.held.or(self.roles.fall),
            Pose::Fall => self.roles.fall.or(self.roles.held),
        }
    }

    fn clip_len(&self) -> f32 {
        self.clip.map_or(1.0, |c| self.asset.clips[c].dur.max(1e-3))
    }

    /// Switch clips with a crossfade.
    fn play(&mut self, clip: Option<usize>, playing: Playing) {
        if clip != self.clip || playing != self.playing {
            self.fade = Some((self.clip, self.time, FADE));
            self.clip = clip;
            self.time = 0.0;
        }
        self.playing = playing;
    }

    /// Idling: switch idle loops now and then, and play a gesture every few
    /// seconds.
    fn idle_tick(&mut self, dt: f32) {
        let len = self.clip_len();
        match self.playing {
            Playing::Once { then } if self.time >= len => match then {
                Some(next) => self.play(Some(next), Playing::Once { then: None }),
                None => {
                    let idle = self.roles.idle.clone();
                    let clip = self.any(&idle);
                    self.play(clip, Playing::Loop);
                    self.next_flourish = 5.0 + self.rand() * 10.0;
                }
            },
            Playing::Once { .. } => {}
            Playing::Loop => {
                self.next_flourish -= dt;
                if self.next_flourish <= 0.0 && !self.roles.flourish.is_empty() {
                    let n = self.roles.flourish.len();
                    let i = ((self.rand() * n as f32) as usize).min(n - 1);
                    let (start, then) = self.roles.flourish[i];
                    self.play(Some(start), Playing::Once { then });
                } else if self.time >= len && self.roles.idle.len() > 1 && self.rand() < 0.35 {
                    // at the end of a loop, sometimes a different idle
                    let idle = self.roles.idle.clone();
                    let clip = self.any(&idle);
                    self.play(clip, Playing::Loop);
                } else if self.time >= len {
                    self.time -= len;
                }
            }
        }
    }

    fn ease_light(&mut self, dt: f32, light: Light, mirrored: bool) {
        let tint = |c: [f32; 3]| {
            let m = c[0].max(c[1]).max(c[2]).max(1e-3);
            c.map(|v| v / m)
        };
        let luma = 0.2126 * light.ambient[0] + 0.7152 * light.ambient[1] + 0.0722 * light.ambient[2];
        // dark surroundings dim the character a little, bright ones lift it
        let exposure = 0.82 + 0.24 * (luma.sqrt() * 1.6).min(1.0);
        let dx = if mirrored { -light.dir[0] } else { light.dir[0] };
        let dir = Vec3::new(dx * 1.2, light.dir[1] * 1.2 + 0.2, 0.75).normalize();
        let a = tint(light.ambient);
        let k = tint(light.key);
        let target = [a[0], a[1], a[2], k[0], k[1], k[2], exposure, dir.x, dir.y, dir.z];
        let f = 1.0 - (-dt * 2.0).exp();
        for (v, t) in self.light.iter_mut().zip(target) {
            *v += (t - *v) * f;
        }
    }

    /// The offscreen texture (premultiplied alpha) and its size.
    pub fn texture(&self) -> (glow::Texture, u32, u32) {
        (self.color, self.size.0, self.size.1)
    }

    /// Advance the animation and redraw the offscreen texture.
    /// Leaves the framebuffer, viewport and clear colour as it found them.
    /// `light` is the environment around the character; `mirrored` when the
    /// overlay flips the texture (walking left), so light from the right
    /// still lands on the right.
    pub fn draw(&mut self, gl: &glow::Context, dt: f32, pose: Pose, light: Light, mirrored: bool) {
        self.ease_light(dt, light, mirrored);
        if pose != self.pose {
            self.pose = pose;
            let clip = self.clip_for(pose);
            self.play(clip, Playing::Loop);
            self.next_flourish = 3.0 + self.rand() * 8.0;
        }
        self.time += dt;
        if pose == Pose::Idle {
            self.idle_tick(dt);
        }

        let t = match self.playing {
            // hold the last frame instead of wrapping around
            Playing::Once { .. } => self.time.min(self.clip_len() - 1e-3).max(0.0),
            Playing::Loop => self.time,
        };
        self.asset.sample(self.clip, t, &mut self.local);
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
        if let Some(smile) = self.smile {
            for &m in &self.mouths {
                self.local[m].w = if m == smile { 0.0 } else { 1.0 };
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
            let l = self.light;
            self.program.set_vec3(gl, "u_ambient", l[0], l[1], l[2]);
            self.program.set_vec3(gl, "u_key", l[3], l[4], l[5]);
            self.program.set_f32(gl, "u_exposure", l[6]);
            self.program.set_vec3(gl, "u_light", l[7], l[8], l[9]);
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
                    if !pass {
                        // inverted hull: the inflated back faces peek out around the edges
                        let px = OUTLINE_PX * 2.0;
                        self.program.set_vec2(gl, "u_outline", px / self.size.0 as f32, px / self.size.1 as f32);
                        self.program.set_i32(gl, "u_is_outline", 1);
                        gl.enable(glow::CULL_FACE);
                        gl.cull_face(glow::BACK); // these exports wind clockwise
                        gl.draw_elements(glow::TRIANGLES, count, glow::UNSIGNED_INT, 0);
                        gl.disable(glow::CULL_FACE);
                        self.program.set_vec2(gl, "u_outline", 0.0, 0.0);
                        self.program.set_i32(gl, "u_is_outline", 0);
                    }
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
/// View-space bounds of the model over a clip (8 samples).
fn clip_bounds(asset: &Asset, prims: &[Prim], geometry: &[Geometry], view: Mat4, clip: Option<usize>) -> (Vec3, Vec3) {
    let mut local = Vec::new();
    let mut globals = Vec::new();
    let mut joints = Vec::new();
    let mut points = Vec::new();
    {
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
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for p in &points {
        let v = view.transform_point3(*p);
        lo = lo.min(v);
        hi = hi.max(v);
    }
    (lo, hi)
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
        for (pose, t) in [(Pose::Idle, 1.0), (Pose::Walk, 0.4), (Pose::Run, 0.3), (Pose::Pat, 0.8), (Pose::Held, 1.5), (Pose::Fall, 0.6)] {
            for _ in 0..(t / 0.05) as usize {
                model.draw(&gl, 0.05, pose, Light::default(), false);
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

    /// Sample frames of named clips, for picking animations:
    /// `ANIMA_TEST_MODEL=... ANIMA_TEST_OUT=/dir ANIMA_TEST_CLIPS=a,b cargo test clip_frames`
    #[test]
    fn clip_frames() {
        use glutin::config::{ConfigSurfaceTypes, ConfigTemplateBuilder};
        use glutin::context::{ContextApi, ContextAttributesBuilder, Version};
        use glutin::display::{GlDisplay, GetGlDisplay};
        let (Ok(path), Ok(out), Ok(names)) =
            (std::env::var("ANIMA_TEST_MODEL"), std::env::var("ANIMA_TEST_OUT"), std::env::var("ANIMA_TEST_CLIPS"))
        else {
            return;
        };
        let device = glutin::api::egl::device::Device::query_devices().unwrap().next().unwrap();
        let display = unsafe { glutin::api::egl::display::Display::with_device(&device, None) }.unwrap();
        let config = unsafe { display.find_configs(ConfigTemplateBuilder::new().with_surface_type(ConfigSurfaceTypes::empty()).build()) }.unwrap().next().unwrap();
        let attrs = ContextAttributesBuilder::new().with_context_api(ContextApi::Gles(Some(Version::new(3, 0)))).build(None);
        let ctx = unsafe { config.display().create_context(&config, &attrs) }.unwrap();
        let _ctx = ctx.make_current_surfaceless().unwrap();
        let gl = unsafe { glow::Context::from_loader_function_cstr(|s| display.get_proc_address(s) as *const _) };
        let mut model = Model3d::load(&gl, Path::new(&path), 384).unwrap();
        let (_, w, h) = model.texture();
        for name in names.split(',') {
            let Some(c) = model.asset.clips.iter().position(|c| c.name == name) else { continue };
            let dur = model.asset.clips[c].dur;
            for k in 1..=3 {
                model.clip = Some(c);
                model.playing = Playing::Loop;
                model.time = dur * k as f32 / 4.0;
                model.fade = None;
                model.draw(&gl, 0.0, Pose::Idle, Light::default(), false);
                let mut px = vec![0u8; (w * h * 4) as usize];
                unsafe {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(model.fbo));
                    gl.read_pixels(0, 0, w as i32, h as i32, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut px)));
                    gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                }
                image::RgbaImage::from_raw(w, h, px).unwrap().save(format!("{out}/{name}_{k}.png")).unwrap();
            }
            println!("{name} {dur:.2}s");
        }
    }

}
