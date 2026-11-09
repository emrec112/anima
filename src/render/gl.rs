//! Small OpenGL ES 3.0 helpers shared by the renderers.

use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};
use glow::HasContext;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Nearest,
    Linear,
}

/// A compiled GL program with lazily resolved uniform locations.
pub struct Program {
    pub id: glow::Program,
    uniforms: HashMap<String, Option<glow::UniformLocation>>,
}

impl Program {
    pub fn new(gl: &glow::Context, vertex: &str, fragment: &str) -> Result<Self> {
        unsafe {
            let program = gl.create_program().map_err(|e| anyhow!(e))?;
            let vs = compile_shader(gl, glow::VERTEX_SHADER, vertex)?;
            let fs = compile_shader(gl, glow::FRAGMENT_SHADER, fragment)?;
            gl.attach_shader(program, vs);
            gl.attach_shader(program, fs);
            gl.link_program(program);

            let linked = gl.get_program_link_status(program);
            let log = gl.get_program_info_log(program);
            gl.detach_shader(program, vs);
            gl.detach_shader(program, fs);
            gl.delete_shader(vs);
            gl.delete_shader(fs);

            if !linked {
                gl.delete_program(program);
                bail!("shader link failed: {log}");
            }

            Ok(Self {
                id: program,
                uniforms: HashMap::new(),
            })
        }
    }

    pub fn bind(&self, gl: &glow::Context) {
        unsafe { gl.use_program(Some(self.id)) };
    }

    pub fn uniform(&mut self, gl: &glow::Context, name: &str) -> Option<glow::UniformLocation> {
        if let Some(loc) = self.uniforms.get(name) {
            return *loc;
        }
        let loc = unsafe { gl.get_uniform_location(self.id, name) };
        self.uniforms.insert(name.to_string(), loc);
        loc
    }

    pub fn set_f32(&mut self, gl: &glow::Context, name: &str, value: f32) {
        if let Some(loc) = self.uniform(gl, name) {
            unsafe { gl.uniform_1_f32(Some(&loc), value) };
        }
    }

    pub fn set_i32(&mut self, gl: &glow::Context, name: &str, value: i32) {
        if let Some(loc) = self.uniform(gl, name) {
            unsafe { gl.uniform_1_i32(Some(&loc), value) };
        }
    }

    pub fn set_vec2(&mut self, gl: &glow::Context, name: &str, x: f32, y: f32) {
        if let Some(loc) = self.uniform(gl, name) {
            unsafe { gl.uniform_2_f32(Some(&loc), x, y) };
        }
    }

    pub fn set_vec3(&mut self, gl: &glow::Context, name: &str, x: f32, y: f32, z: f32) {
        if let Some(loc) = self.uniform(gl, name) {
            unsafe { gl.uniform_3_f32(Some(&loc), x, y, z) };
        }
    }

    pub fn set_vec4(&mut self, gl: &glow::Context, name: &str, v: [f32; 4]) {
        if let Some(loc) = self.uniform(gl, name) {
            unsafe { gl.uniform_4_f32(Some(&loc), v[0], v[1], v[2], v[3]) };
        }
    }

    /// Delete the GL program.  Requires a current context.
    pub fn destroy(&self, gl: &glow::Context) {
        unsafe { gl.delete_program(self.id) };
    }
}

impl Drop for Program {
    fn drop(&mut self) {
        // NOTE: the GL context is normally current when renderers are dropped.
        // Nothing to do here: glow needs a context, so we leak on shutdown,
        // which is harmless because the context dies with the process.
    }
}

fn compile_shader(gl: &glow::Context, kind: u32, source: &str) -> Result<glow::Shader> {
    unsafe {
        let shader = gl.create_shader(kind).map_err(|e| anyhow!(e))?;
        // Windows has desktop OpenGL, which takes the same shaders
        #[cfg(windows)]
        let source = &source.replacen("#version 300 es", "#version 330 core", 1);
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
        if !gl.get_shader_compile_status(shader) {
            let log = gl.get_shader_info_log(shader);
            gl.delete_shader(shader);
            let which = if kind == glow::VERTEX_SHADER {
                "vertex"
            } else {
                "fragment"
            };
            bail!("{which} shader compile failed: {log}");
        }
        Ok(shader)
    }
}

/// Fullscreen triangle: three vertices that cover the viewport with no diagonal
/// seam. Attribute 0 is `vec2 position` in clip space; the varying `v_uv` is
/// available to fragment shaders.
pub struct Triangle {
    pub vao: glow::VertexArray,
    pub vbo: glow::Buffer,
}

impl Triangle {
    pub fn new(gl: &glow::Context) -> Result<Self> {
        const VERTS: [f32; 6] = [-1.0, -1.0, 3.0, -1.0, -1.0, 3.0];
        unsafe {
            let vao = gl.create_vertex_array().map_err(|e| anyhow!(e))?;
            let vbo = gl.create_buffer().map_err(|e| anyhow!(e))?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck_cast(&VERTS),
                glow::STATIC_DRAW,
            );
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            gl.bind_vertex_array(None);
            Ok(Self { vao, vbo })
        }
    }

    pub fn draw(&self, gl: &glow::Context) {
        unsafe {
            gl.bind_vertex_array(Some(self.vao));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.bind_vertex_array(None);
        }
    }
}

/// f32 slice -> &[u8] without pulling in bytemuck.
pub fn bytemuck_cast(data: &[f32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(data)) }
}

pub fn create_texture(
    gl: &glow::Context,
    width: u32,
    height: u32,
    pixels: &[u8],
    filter: Filter,
    mipmaps: bool,
) -> Result<glow::Texture> {
    create_texture_ex(gl, width, height, pixels, filter, mipmaps, true)
}

pub fn create_texture_ex(
    gl: &glow::Context,
    width: u32,
    height: u32,
    pixels: &[u8],
    filter: Filter,
    mipmaps: bool,
    clamp_uvs: bool,
) -> Result<glow::Texture> {
    unsafe {
        let tex = gl.create_texture().map_err(|e| anyhow!(e))?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            width as i32,
            height as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(pixels)),
        );
        let min = match (filter, mipmaps) {
            (_, true) => glow::LINEAR_MIPMAP_LINEAR,
            (Filter::Linear, false) => glow::LINEAR,
            (Filter::Nearest, false) => glow::NEAREST,
        };
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, min as i32);
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            match filter {
                Filter::Linear => glow::LINEAR,
                Filter::Nearest => glow::NEAREST,
            } as i32,
        );
        let wrap = if clamp_uvs {
            glow::CLAMP_TO_EDGE
        } else {
            glow::REPEAT
        } as i32;
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, wrap);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, wrap);
        if mipmaps {
            gl.generate_mipmap(glow::TEXTURE_2D);
        }
        gl.bind_texture(glow::TEXTURE_2D, None);
        Ok(tex)
    }
}

/// UV scale that makes a `src` aspect ratio cover a `dst` area (CSS `cover`).
pub fn cover_uv(src_w: f32, src_h: f32, dst_w: f32, dst_h: f32) -> (f32, f32) {
    if src_w <= 0.0 || src_h <= 0.0 || dst_w <= 0.0 || dst_h <= 0.0 {
        return (1.0, 1.0);
    }
    let src_aspect = src_w / src_h;
    let dst_aspect = dst_w / dst_h;
    if src_aspect > dst_aspect {
        (dst_aspect / src_aspect, 1.0)
    } else {
        (1.0, src_aspect / dst_aspect)
    }
}
