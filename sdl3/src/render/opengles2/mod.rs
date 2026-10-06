// Rust translation of src/render/opengles2/SDL_render_gles2.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The OpenGL ES 2.0 renderer ("opengles2"): draws through an OpenGL ES
//! 2.0 (or later) context of the window, with the GL entry points looked up
//! at run time.
//!
//! As on the desktop builds upstream, vertices are passed in client-side
//! arrays (vertex buffer objects are only used on Emscripten). The texture
//! creation options don't take existing GL textures yet
//! (`SDL_PROP_TEXTURE_CREATE_OPENGLES2_TEXTURE_*_NUMBER`), so every texture
//! owns its GL textures.

mod funcs;
mod shaders;
#[cfg(test)]
mod tests;

use std::any::Any;
use std::ffi::CStr;

use funcs::*;
use shaders::{ShaderInclude, ShaderType};

use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::events::EventType;
use crate::hints;
use crate::properties::Properties;
use crate::render::sysrender::{
    invalid_texture, CopyEx, DrawCmd, DrawKind, Geometry, RenderBackend, RenderCommand,
    TextureCreateProps, TextureData, TextureStore,
};
use crate::render::{Texture, TextureAccess, TextureAddressMode};
use crate::video::blendmode::{BlendFactor, BlendOperation};
use crate::video::gl::{self, GlAttr, GlContext};
use crate::video::pixels::{Color, Colorspace, FColor, PixelFormat};
use crate::video::rect::{FPoint, FRect, Rect};
use crate::video::surface::{ScaleMode, Surface};
use crate::video::{BlendMode, FlipMode, Window};

/// The name of the OpenGL ES 2.0 renderer (`GLES2_RenderDriver.name`).
pub(crate) const GLES2_RENDERER: &str = "opengles2";

/// The GLuint texture of an OpenGL ES 2.0 texture (the Y plane of a YUV or
/// NV12 texture). Translation of `SDL_PROP_TEXTURE_OPENGLES2_TEXTURE_NUMBER`.
pub const PROP_TEXTURE_OPENGLES2_TEXTURE_NUMBER: &str = "SDL.texture.opengles2.texture";
/// The GLuint texture of the UV plane of an NV12 texture.
/// Translation of `SDL_PROP_TEXTURE_OPENGLES2_TEXTURE_UV_NUMBER`.
pub const PROP_TEXTURE_OPENGLES2_TEXTURE_UV_NUMBER: &str = "SDL.texture.opengles2.texture_uv";
/// The GLuint texture of the U plane of a YUV texture.
/// Translation of `SDL_PROP_TEXTURE_OPENGLES2_TEXTURE_U_NUMBER`.
pub const PROP_TEXTURE_OPENGLES2_TEXTURE_U_NUMBER: &str = "SDL.texture.opengles2.texture_u";
/// The GLuint texture of the V plane of a YUV texture.
/// Translation of `SDL_PROP_TEXTURE_OPENGLES2_TEXTURE_V_NUMBER`.
pub const PROP_TEXTURE_OPENGLES2_TEXTURE_V_NUMBER: &str = "SDL.texture.opengles2.texture_v";
/// The GLenum for the texture target (`GL_TEXTURE_2D`, `GL_TEXTURE_EXTERNAL_OES`, etc).
/// Translation of `SDL_PROP_TEXTURE_OPENGLES2_TEXTURE_TARGET_NUMBER`.
pub const PROP_TEXTURE_OPENGLES2_TEXTURE_TARGET_NUMBER: &str = "SDL.texture.opengles2.target";

/* To prevent unnecessary window recreation,
 * these should match the defaults selected in SDL_GL_ResetAttributes
 */
const RENDERER_CONTEXT_MAJOR: i32 = 2;
const RENDERER_CONTEXT_MINOR: i32 = 0;

/*************************************************************************************************
 * Context structures                                                                            *
 *************************************************************************************************/

/// Translation of `GLES2_PaletteData`.
struct PaletteData {
    texture: GLuint,
}

/// Translation of `GLES2_TextureData`.
struct Gles2Texture {
    texture: GLuint,
    /// framebuffer object; this is zero unless this texture is a render target.
    fbo: GLuint,
    texture_external: bool,
    texture_type: GLenum,
    pixel_format: GLenum,
    pixel_type: GLenum,
    pixel_data: Vec<u8>,
    pitch: i32,
    // YUV texture support
    yuv: bool,
    nv12: bool,
    texture_v: GLuint,
    texture_v_external: bool,
    texture_u: GLuint,
    texture_u_external: bool,
    texel_size: [GLfloat; 4],
    texture_scale_mode: ScaleMode,
    texture_address_mode_u: TextureAddressMode,
    texture_address_mode_v: TextureAddressMode,
    /// The texture of the texture's palette (`texture->palette->internal`),
    /// recorded when the front end changes the palette.
    palette_texture: GLuint,
}

/// Translation of `GLES2_Attribute`.
#[derive(Clone, Copy)]
enum Attribute {
    Position = 0,
    Color = 1,
    TexCoord = 2,
}

/// Translation of `GLES2_Uniform`.
#[derive(Clone, Copy)]
enum Uniform {
    Projection,
    Texture,
    TextureU,
    TextureV,
    Palette,
    TexelSize,
    Offset,
    Matrix,
}

/// Translation of `NUM_GLES2_UNIFORMS`.
const NUM_GLES2_UNIFORMS: usize = 8;

/// Translation of `GLES2_UniformNames`.
const UNIFORM_NAMES: [&CStr; NUM_GLES2_UNIFORMS] = [
    c"u_projection",
    c"u_texture",
    c"u_texture_u",
    c"u_texture_v",
    c"u_palette",
    c"u_texel_size",
    c"u_offset",
    c"u_matrix",
];

/// Translation of `GLES2_ProgramCacheEntry`.
struct ProgramCacheEntry {
    id: GLuint,
    vertex_shader: GLuint,
    fragment_shader: GLuint,
    uniform_locations: [GLint; NUM_GLES2_UNIFORMS],
    projection: [[GLfloat; 4]; 4],
    shader_params: Option<Vec<f32>>,
}

impl ProgramCacheEntry {
    fn location(&self, uniform: Uniform) -> GLint {
        self.uniform_locations[uniform as usize]
    }
}

/// Translation of `GLES2_ImageSource` (without `GLES2_IMAGESOURCE_INVALID`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ImageSource {
    Solid,
    TextureIndex8,
    TextureAbgr,
    TextureArgb,
    TextureRgb,
    TextureBgr,
    TextureYuv,
    TextureNv12,
    TextureNv21,
    TextureExternalOes,
}

/// The render target (`renderer->target`), with what the backend reads of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Target {
    format: PixelFormat,
    fbo: GLuint,
}

/// Translation of `GLES2_DrawStateCache`.
struct DrawStateCache {
    viewport: Rect,
    viewport_dirty: bool,
    texture: Option<Texture>,
    target: Option<Target>,
    blend: BlendMode,
    cliprect_enabled_dirty: bool,
    cliprect_enabled: bool,
    cliprect_dirty: bool,
    cliprect: Rect,
    texturing: bool,
    texturing_dirty: bool,
    clear_color: FColor,
    clear_color_dirty: bool,
    drawablew: i32,
    drawableh: i32,
    /// The id of the current program (in the program cache).
    program: Option<GLuint>,
    projection: [[GLfloat; 4]; 4],
}

/// The renderer's data. Translation of `GLES2_RenderData`, with the parts
/// of `SDL_Renderer` the backend sets up (`window`, `target`, the texture
/// formats and limits) and its vertex storage.
pub(crate) struct Gles2Renderer {
    window: Window,
    context: Option<GlContext>,

    debug_enabled: bool,

    gl_oes_egl_image_external_supported: bool,
    gl_ext_blend_minmax_supported: bool,

    gl: Gles2Funcs,
    window_framebuffer: GLuint,

    shader_id_cache: [GLuint; ShaderType::COUNT],

    /// The linked programs, most recently used first.
    program_cache: Vec<ProgramCacheEntry>,

    drawstate: DrawStateCache,
    texcoord_precision_hint: ShaderInclude,

    /// `renderer->target`
    target: Option<Target>,
    /// The vertex data of the queued commands (floats; `first` indexes it).
    verts: Vec<f32>,
    texture_formats: Vec<PixelFormat>,
    npot_texture_wrap_unsupported: bool,
    max_texture_size: i32,
}

/// Translation of `GLES2_MAX_CACHED_PROGRAMS`.
const GLES2_MAX_CACHED_PROGRAMS: usize = 8;

/// The floats of an `SDL_VertexSolid` (position, color).
const VERTEX_SOLID_FLOATS: usize = 6;
/// The floats of an `SDL_Vertex` (position, color, tex_coord).
const VERTEX_FLOATS: usize = 8;

/// Translation of `GL_TranslateError()`.
fn gl_translate_error(error: GLenum) -> &'static str {
    match error {
        GL_INVALID_ENUM => "GL_INVALID_ENUM",
        GL_INVALID_VALUE => "GL_INVALID_VALUE",
        GL_INVALID_OPERATION => "GL_INVALID_OPERATION",
        GL_OUT_OF_MEMORY => "GL_OUT_OF_MEMORY",
        GL_NO_ERROR => "GL_NO_ERROR",
        _ => "UNKNOWN",
    }
}

/// `GL_CheckError(prefix, renderer)`, naming the calling function.
macro_rules! gl_check_error {
    ($data:expr, $prefix:expr, $function:literal) => {
        $data.check_all_errors($prefix, line!(), $function)
    };
}

/// The GLES2 data of a texture.
fn gles2_texture(texture: &TextureData) -> Option<&Gles2Texture> {
    texture.internal.as_ref()?.downcast_ref::<Gles2Texture>()
}

fn gles2_texture_mut(texture: &mut TextureData) -> Option<&mut Gles2Texture> {
    texture.internal.as_mut()?.downcast_mut::<Gles2Texture>()
}

/// Whether drawing into a texture of `format` swaps red and blue (the GL
/// storage is RGBA, so BGRA content is drawn swapped).
fn is_colorswap_format(format: PixelFormat) -> bool {
    format == PixelFormat::BGRA32 || format == PixelFormat::BGRX32
}

/// Whether two float arrays have the same bits (`SDL_memcmp()`).
fn same_bits(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}

/// The YCbCr → RGB shader parameters of `colorspace` (offsets, then the R,
/// G and B coefficients, each padded to four floats), as
/// `SDL_GetYCbCRtoRGBConversionMatrix()` returns them.
fn yuv_shader_params(colorspace: Colorspace, h: i32) -> Option<[f32; 16]> {
    let matrix = colorspace.ycbcr_to_rgb_matrix(h.max(0) as u32, 8)?;
    let mut params = [0.0; 16];
    params[..3].copy_from_slice(&matrix.offset);
    for (row, coeff) in matrix.coeff.iter().enumerate() {
        params[4 + row * 4..4 + row * 4 + 3].copy_from_slice(coeff);
    }
    Some(params)
}

/// Translation of `GetBlendFunc()`.
fn get_blend_func(factor: Option<BlendFactor>) -> GLenum {
    match factor {
        Some(BlendFactor::Zero) => GL_ZERO,
        Some(BlendFactor::One) => GL_ONE,
        Some(BlendFactor::SrcColor) => GL_SRC_COLOR,
        Some(BlendFactor::OneMinusSrcColor) => GL_ONE_MINUS_SRC_COLOR,
        Some(BlendFactor::SrcAlpha) => GL_SRC_ALPHA,
        Some(BlendFactor::OneMinusSrcAlpha) => GL_ONE_MINUS_SRC_ALPHA,
        Some(BlendFactor::DstColor) => GL_DST_COLOR,
        Some(BlendFactor::OneMinusDstColor) => GL_ONE_MINUS_DST_COLOR,
        Some(BlendFactor::DstAlpha) => GL_DST_ALPHA,
        Some(BlendFactor::OneMinusDstAlpha) => GL_ONE_MINUS_DST_ALPHA,
        None => GL_INVALID_ENUM,
    }
}

/// Translation of `GetBlendEquation()`.
fn get_blend_equation(operation: Option<BlendOperation>) -> GLenum {
    match operation {
        Some(BlendOperation::Add) => GL_FUNC_ADD,
        Some(BlendOperation::Subtract) => GL_FUNC_SUBTRACT,
        Some(BlendOperation::RevSubtract) => GL_FUNC_REVERSE_SUBTRACT,
        Some(BlendOperation::Minimum) => GL_MIN_EXT,
        Some(BlendOperation::Maximum) => GL_MAX_EXT,
        None => GL_INVALID_ENUM,
    }
}

/// Translation of `SetTextureScaleMode()` (every `SDL_ScaleMode` is known
/// here, so it can't fail).
fn set_texture_scale_mode(
    gl: &Gles2Funcs,
    textype: GLenum,
    format: PixelFormat,
    scale_mode: ScaleMode,
) {
    let filter = match scale_mode {
        ScaleMode::Nearest => GL_NEAREST,
        ScaleMode::Linear => {
            if format == PixelFormat::INDEX8 {
                // We'll do linear sampling in the shader
                GL_NEAREST
            } else {
                GL_LINEAR
            }
        }
        // We don't have the functions we need, fall back to nearest sampling
        // (the OPENGLES_300 build samples linearly for the pixel art shaders)
        ScaleMode::PixelArt => GL_NEAREST,
    };
    gl.tex_parameteri(textype, GL_TEXTURE_MIN_FILTER, filter);
    gl.tex_parameteri(textype, GL_TEXTURE_MAG_FILTER, filter);
}

/// Translation of `TranslateAddressMode()`.
fn translate_address_mode(address_mode: TextureAddressMode) -> GLint {
    match address_mode {
        TextureAddressMode::Clamp => GL_CLAMP_TO_EDGE,
        TextureAddressMode::Wrap => GL_REPEAT,
        _ => {
            crate::sdl_assert!(!"Unknown texture address mode");
            GL_CLAMP_TO_EDGE
        }
    }
}

/// Translation of `SetTextureAddressMode()`.
fn set_texture_address_mode(
    gl: &Gles2Funcs,
    textype: GLenum,
    address_mode_u: TextureAddressMode,
    address_mode_v: TextureAddressMode,
) {
    gl.tex_parameteri(
        textype,
        GL_TEXTURE_WRAP_S,
        translate_address_mode(address_mode_u),
    );
    gl.tex_parameteri(
        textype,
        GL_TEXTURE_WRAP_T,
        translate_address_mode(address_mode_v),
    );
}

/// The bytes of `pixels` from `offset` on (an error past the end: the C
/// code trusts the caller to pass all the planes).
fn plane(pixels: &[u8], offset: usize) -> Result<&[u8]> {
    pixels
        .get(offset..)
        .ok_or_else(|| Error::invalid_param("pixels"))
}

/// Translation of `GLES2_TexSubImage2D()`: upload rows `pitch` bytes apart,
/// repacking them tightly if needed.
#[allow(clippy::too_many_arguments)]
fn tex_sub_image_2d(
    gl: &Gles2Funcs,
    target: GLenum,
    xoffset: GLint,
    yoffset: GLint,
    width: GLsizei,
    height: GLsizei,
    format: GLenum,
    ty: GLenum,
    pixels: &[u8],
    pitch: usize,
    bpp: usize,
) -> Result<()> {
    if width == 0 || height == 0 || bpp == 0 {
        return Ok(()); // nothing to do
    }

    // Reformat the texture data into a tightly packed array
    let src_pitch = width as usize * bpp;
    let rows = height as usize;
    // (the C code reads the rows without knowing the buffer's size)
    if pixels.len() < (rows - 1) * pitch + src_pitch {
        return Err(Error::invalid_param("pixels"));
    }
    if pitch != src_pitch {
        let mut blob = Vec::with_capacity(src_pitch * rows);
        for row in pixels.chunks(pitch).take(rows) {
            blob.extend_from_slice(&row[..src_pitch]);
        }
        gl.tex_sub_image_2d(target, xoffset, yoffset, width, height, format, ty, &blob);
    } else {
        gl.tex_sub_image_2d(target, xoffset, yoffset, width, height, format, ty, pixels);
    }
    Ok(())
}

/*************************************************************************************************
 * Renderer state APIs                                                                           *
 *************************************************************************************************/

impl Gles2Renderer {
    /// Translation of `GL_ClearErrors()`.
    fn clear_errors(&self) {
        if !self.debug_enabled {
            return;
        }
        while self.gl.get_error() != GL_NO_ERROR {
            // continue;
        }
    }

    /// Translation of `GL_CheckAllErrors()`.
    fn check_all_errors(&self, prefix: &str, line: u32, function: &str) -> Result<()> {
        if !self.debug_enabled {
            return Ok(());
        }
        let mut result = Ok(());
        // check gl errors (can return multiple errors)
        loop {
            let error = self.gl.get_error();
            if error == GL_NO_ERROR {
                break;
            }
            let prefix = if prefix.is_empty() { "generic" } else { prefix };
            result = Err(Error::new(format!(
                "{prefix}: SDL_render_gles2.c ({line}): {function} {} (0x{error:X})",
                gl_translate_error(error)
            )));
        }
        result
    }

    /// Make the renderer's context current. Translation of
    /// `GLES2_ActivateRenderer()`.
    fn activate(&mut self) -> Result<()> {
        let Some(context) = &self.context else {
            return Err(Error::new("The renderer's GL context is gone"));
        };
        if !context.is_current() {
            // Null out the current program to ensure we set it again
            self.drawstate.program = None;

            context.make_current(Some(&self.window))?;
        }

        self.clear_errors();

        Ok(())
    }

    /// Translation of `GLES2_CacheProgram()`.
    ///
    /// Note (upstream): the C cache is a linked list that keeps the tail's
    /// `next` pointing at an evicted entry and doesn't move `tail` when the
    /// tail entry moves to the front, so a lookup after an eviction can
    /// walk into freed memory. The cache here is a `Vec` (most recently
    /// used first), which can't.
    fn cache_program(&mut self, vertex: GLuint, fragment: GLuint) -> Result<GLuint> {
        // Check if we've already cached this program
        if let Some(i) = self
            .program_cache
            .iter()
            .position(|e| e.vertex_shader == vertex && e.fragment_shader == fragment)
        {
            if i != 0 {
                let entry = self.program_cache.remove(i);
                self.program_cache.insert(0, entry);
            }
            return Ok(self.program_cache[0].id);
        }

        // Create a program cache entry
        let gl = &self.gl;
        let mut entry = ProgramCacheEntry {
            id: 0,
            vertex_shader: vertex,
            fragment_shader: fragment,
            uniform_locations: [0; NUM_GLES2_UNIFORMS],
            projection: [[0.0; 4]; 4],
            shader_params: None,
        };

        // Create the program and link it
        entry.id = gl.create_program();
        gl.attach_shader(entry.id, vertex);
        gl.attach_shader(entry.id, fragment);
        gl.bind_attrib_location(entry.id, Attribute::Position as GLuint, c"a_position");
        gl.bind_attrib_location(entry.id, Attribute::Color as GLuint, c"a_color");
        gl.bind_attrib_location(entry.id, Attribute::TexCoord as GLuint, c"a_texCoord");
        gl.link_program(entry.id);
        let link_successful = gl.get_program(entry.id, GL_LINK_STATUS);
        if link_successful == 0 {
            gl.delete_program(entry.id);
            return Err(Error::new("Failed to link shader program"));
        }

        // Predetermine locations of uniform variables
        for (location, name) in entry.uniform_locations.iter_mut().zip(UNIFORM_NAMES) {
            *location = gl.get_uniform_location(entry.id, name);
        }

        gl.use_program(entry.id);
        if entry.location(Uniform::TextureV) != -1 {
            gl.uniform1i(entry.location(Uniform::TextureV), 2); // always texture unit 2.
        }
        if entry.location(Uniform::TextureU) != -1 {
            gl.uniform1i(entry.location(Uniform::TextureU), 1); // always texture unit 1.
        }
        if entry.location(Uniform::Palette) != -1 {
            gl.uniform1i(entry.location(Uniform::Palette), 1); // always texture unit 1.
        }
        if entry.location(Uniform::Texture) != -1 {
            gl.uniform1i(entry.location(Uniform::Texture), 0); // always texture unit 0.
        }
        if entry.location(Uniform::Projection) != -1 {
            gl.uniform_matrix4(entry.location(Uniform::Projection), &entry.projection);
        }

        // Cache the linked program
        let id = entry.id;
        self.program_cache.insert(0, entry);

        // Evict the last entry from the cache if we exceed the limit
        if self.program_cache.len() > GLES2_MAX_CACHED_PROGRAMS {
            if let Some(oldest) = self.program_cache.pop() {
                self.gl.delete_program(oldest.id);
            }
        }
        Ok(id)
    }

    /// Compile a shader and cache it, returning its id. Translation of
    /// `GLES2_CacheShader()`.
    fn cache_shader(&mut self, ty: ShaderType, shader_type: GLenum) -> Result<GLuint> {
        let gl = &self.gl;
        let mut id = 0;
        let mut compile_successful = false;
        let shader_body = ty.source();

        // FIXME (upstream): when the first attempt fails, its shader object
        // is not deleted before the second attempt creates another one.
        for attempt in 0..2 {
            if compile_successful {
                break;
            }
            let mut shader_src_list = Vec::with_capacity(3);

            // (the OPENGLES_300 build starts with "#version 300 es\n")
            shader_src_list.push(ty.prologue());

            if shader_type == GL_FRAGMENT_SHADER {
                if attempt == 0 {
                    shader_src_list.push(self.texcoord_precision_hint.source());
                } else {
                    shader_src_list.push(ShaderInclude::UndefPrecision.source());
                }
            }

            shader_src_list.push(shader_body);

            // Compile
            id = gl.create_shader(shader_type);
            gl.shader_source(id, &shader_src_list);
            gl.compile_shader(id);
            compile_successful = gl.get_shader(id, GL_COMPILE_STATUS) != 0;
        }

        if !compile_successful {
            let length = gl.get_shader(id, GL_INFO_LOG_LENGTH);
            let info = (length > 0).then(|| gl.get_shader_info_log(id, length));
            match info {
                Some(info) => crate::log::error!(
                    crate::log::Category::Render,
                    "Failed to load the shader {}: {}",
                    ty as i32,
                    info
                ),
                None => crate::log::error!(
                    crate::log::Category::Render,
                    "Failed to load the shader {}",
                    ty as i32
                ),
            }
            gl.delete_shader(id);

            return Err(Error::new(format!(
                "Failed to load the shader {}",
                ty as i32
            )));
        }

        // Cache
        self.shader_id_cache[ty as usize] = id;

        Ok(id)
    }

    /// Translation of `GLES2_CacheShaders()`.
    fn cache_shaders(&mut self) -> Result<()> {
        self.texcoord_precision_hint = ShaderInclude::texcoord_precision_from_hint();

        for shader in ShaderType::ALL {
            if shader == ShaderType::FragmentTextureExternalOes {
                break;
            }
            let shader_type = if shader == ShaderType::VertexDefault {
                GL_VERTEX_SHADER
            } else {
                GL_FRAGMENT_SHADER
            };
            self.cache_shader(shader, shader_type)?;
        }
        Ok(())
    }

    /// Translation of `GLES2_SelectProgram()`.
    fn select_program(
        &mut self,
        texture: Option<&TextureData>,
        source: ImageSource,
        scale_mode: ScaleMode,
        colorspace: Colorspace,
    ) -> Result<()> {
        let result = self.select_program_inner(texture, source, scale_mode, colorspace);
        if result.is_err() {
            // (fault:)
            self.drawstate.program = None;
        }
        result
    }

    fn select_program_inner(
        &mut self,
        texture: Option<&TextureData>,
        source: ImageSource,
        scale_mode: ScaleMode,
        colorspace: Colorspace,
    ) -> Result<()> {
        use ShaderType as S;
        let tdata = texture.and_then(gles2_texture);
        let colorswap = self
            .drawstate
            .target
            .is_some_and(|t| is_colorswap_format(t.format));
        let texel_size = tdata.map(|t| &t.texel_size[..]);
        let yuv_params;
        let mut shader_params: Option<&[f32]> = None;

        // FIXME (upstream): the YUV matrix is looked up for a height of 0,
        // so a colorspace without matrix coefficients always converts as
        // BT.601 here, while the texture was checked (and the software
        // conversion picks the matrix) with its real height.
        let yuv_matrix = || {
            yuv_shader_params(colorspace, 0).ok_or_else(|| Error::new("Unsupported YUV colorspace"))
        };

        // Select an appropriate shader pair for the specified modes
        let vtype = S::VertexDefault;
        let ftype = match source {
            ImageSource::Solid => S::FragmentSolid,
            ImageSource::TextureIndex8 => match scale_mode {
                ScaleMode::Nearest => {
                    if colorswap {
                        S::FragmentTexturePaletteNearestColorswap
                    } else {
                        S::FragmentTexturePaletteNearest
                    }
                }
                ScaleMode::Linear => {
                    shader_params = texel_size;
                    if colorswap {
                        S::FragmentTexturePaletteLinearColorswap
                    } else {
                        S::FragmentTexturePaletteLinear
                    }
                }
                ScaleMode::PixelArt => {
                    shader_params = texel_size;
                    if colorswap {
                        S::FragmentTexturePalettePixelartColorswap
                    } else {
                        S::FragmentTexturePalettePixelart
                    }
                }
            },
            ImageSource::TextureAbgr => {
                if scale_mode == ScaleMode::PixelArt {
                    shader_params = texel_size;
                    S::FragmentTextureAbgrPixelart
                } else {
                    S::FragmentTextureAbgr
                }
            }
            ImageSource::TextureArgb => {
                if scale_mode == ScaleMode::PixelArt {
                    shader_params = texel_size;
                    S::FragmentTextureArgbPixelart
                } else {
                    S::FragmentTextureArgb
                }
            }
            ImageSource::TextureRgb => {
                if scale_mode == ScaleMode::PixelArt {
                    shader_params = texel_size;
                    S::FragmentTextureRgbPixelart
                } else {
                    S::FragmentTextureRgb
                }
            }
            ImageSource::TextureBgr => {
                if scale_mode == ScaleMode::PixelArt {
                    shader_params = texel_size;
                    S::FragmentTextureBgrPixelart
                } else {
                    S::FragmentTextureBgr
                }
            }
            ImageSource::TextureYuv => {
                yuv_params = yuv_matrix()?;
                shader_params = Some(&yuv_params);
                S::FragmentTextureYuv
            }
            ImageSource::TextureNv12 => {
                yuv_params = yuv_matrix()?;
                shader_params = Some(&yuv_params);
                if hints::get_bool("SDL_RENDER_OPENGL_NV12_RG_SHADER", false) {
                    S::FragmentTextureNv12Rg
                } else {
                    S::FragmentTextureNv12Ra
                }
            }
            ImageSource::TextureNv21 => {
                yuv_params = yuv_matrix()?;
                shader_params = Some(&yuv_params);
                if hints::get_bool("SDL_RENDER_OPENGL_NV12_RG_SHADER", false) {
                    S::FragmentTextureNv21Rg
                } else {
                    S::FragmentTextureNv21Ra
                }
            }
            ImageSource::TextureExternalOes => S::FragmentTextureExternalOes,
        };

        // Load the requested shaders
        // Note (upstream): the C code stores GLES2_CacheShader()'s bool
        // result as the shader id when a shader isn't cached yet; the id is
        // returned here.
        let vertex = match self.shader_id_cache[vtype as usize] {
            0 => self.cache_shader(vtype, GL_VERTEX_SHADER)?,
            id => id,
        };
        let fragment = match self.shader_id_cache[ftype as usize] {
            0 => self.cache_shader(ftype, GL_FRAGMENT_SHADER)?,
            id => id,
        };

        // Check if we need to change programs at all
        if let Some(program) = self
            .drawstate
            .program
            .and_then(|id| self.program_cache.iter().find(|e| e.id == id))
        {
            if program.vertex_shader == vertex
                && program.fragment_shader == fragment
                && shader_params.is_none_or(|params| {
                    program
                        .shader_params
                        .as_deref()
                        .is_some_and(|p| same_bits(params, p))
                })
            {
                return Ok(());
            }
        }

        // Generate a matching program
        let id = self.cache_program(vertex, fragment)?;

        // Select that program in OpenGL
        self.gl.use_program(id);

        let gl = &self.gl;
        let program = self
            .program_cache
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| Error::new("Failed to link shader program"))?;
        if let Some(params) = shader_params {
            if !program
                .shader_params
                .as_deref()
                .is_some_and(|p| same_bits(params, p))
            {
                if ftype as usize >= S::FragmentTextureYuv as usize {
                    // YUV shader params are Yoffset, 0, Rcoeff, 0, Gcoeff, 0, Bcoeff, 0
                    if program.location(Uniform::Offset) != -1 {
                        gl.uniform3f(
                            program.location(Uniform::Offset),
                            params[0],
                            params[1],
                            params[2],
                        );
                    }
                    if program.location(Uniform::Matrix) != -1 {
                        let matrix = [
                            params[4], params[5], params[6], params[8], params[9], params[10],
                            params[12], params[13], params[14],
                        ];
                        gl.uniform_matrix3(program.location(Uniform::Matrix), &matrix);
                    }
                } else {
                    gl.uniform4f(
                        program.location(Uniform::TexelSize),
                        [params[0], params[1], params[2], params[3]],
                    );
                }

                program.shader_params = Some(params.to_vec());
            }
        }

        // Set the current program
        self.drawstate.program = Some(id);

        // Clean up and return
        Ok(())
    }

    /// Whether the current target swaps red and blue.
    fn colorswap(&self) -> bool {
        self.target.is_some_and(|t| is_colorswap_format(t.format))
    }

    /// The draw color of a command, scaled and swapped for the target.
    fn vertex_color(&self, mut color: FColor, color_scale: f32) -> FColor {
        color.r *= color_scale;
        color.g *= color_scale;
        color.b *= color_scale;

        if self.colorswap() {
            std::mem::swap(&mut color.r, &mut color.b);
        }
        color
    }

    fn push_vertex_solid(&mut self, x: f32, y: f32, color: FColor) {
        self.verts
            .extend_from_slice(&[x, y, color.r, color.g, color.b, color.a]);
    }

    /// Translation of `SetDrawState()`.
    fn set_draw_state(
        &mut self,
        cmd: &DrawCmd,
        imgsrc: ImageSource,
        texture: Option<&TextureData>,
    ) -> Result<()> {
        let blend = cmd.blend;

        crate::sdl_assert!(texture.is_some() == (imgsrc != ImageSource::Solid));

        let gl = &self.gl;
        let ds = &mut self.drawstate;
        if ds.viewport_dirty {
            let viewport = ds.viewport;
            gl.viewport(
                viewport.x,
                if ds.target.is_some() {
                    viewport.y
                } else {
                    ds.drawableh - viewport.y - viewport.h
                },
                viewport.w,
                viewport.h,
            );
            if viewport.w != 0 && viewport.h != 0 {
                ds.projection[0][0] = 2.0 / viewport.w as f32;
                ds.projection[1][1] =
                    if ds.target.is_some() { 2.0 } else { -2.0 } / viewport.h as f32;
                ds.projection[3][1] = if ds.target.is_some() { -1.0 } else { 1.0 };
            }
            ds.viewport_dirty = false;
        }

        if ds.cliprect_enabled_dirty {
            if !ds.cliprect_enabled {
                gl.disable(GL_SCISSOR_TEST);
            } else {
                gl.enable(GL_SCISSOR_TEST);
            }
            ds.cliprect_enabled_dirty = false;
        }

        if ds.cliprect_enabled && ds.cliprect_dirty {
            let viewport = ds.viewport;
            let rect = ds.cliprect;
            gl.scissor(
                viewport.x + rect.x,
                if ds.target.is_some() {
                    viewport.y + rect.y
                } else {
                    ds.drawableh - viewport.y - rect.y - rect.h
                },
                rect.w,
                rect.h,
            );
            ds.cliprect_dirty = false;
        }

        if ds.texturing_dirty || (texture.is_some() != ds.texturing) {
            if texture.is_none() {
                gl.disable_vertex_attrib_array(Attribute::TexCoord as GLuint);
                ds.texturing = false;
            } else {
                gl.enable_vertex_attrib_array(Attribute::TexCoord as GLuint);
                ds.texturing = true;
            }
            ds.texturing_dirty = false;
        }

        let stride = if texture.is_some() {
            VERTEX_FLOATS
        } else {
            VERTEX_SOLID_FLOATS
        };
        let stride_bytes = (stride * size_of::<f32>()) as GLsizei;

        // address of first vertex
        let base = self.verts.as_ptr().wrapping_add(cmd.first);
        if texture.is_some() {
            // SAFETY: the texture coordinates of the command's vertices are
            // in `self.verts`, which isn't touched until the draw calls.
            unsafe {
                gl.vertex_attrib_pointer(
                    Attribute::TexCoord as GLuint,
                    2,
                    GL_FALSE,
                    stride_bytes,
                    base.wrapping_add(6),
                )
            };
        }

        let colorspace = texture.map_or(Colorspace::SRGB, |t| t.colorspace);
        self.select_program(texture, imgsrc, cmd.texture_scale_mode, colorspace)?;

        let gl = &self.gl;
        let projection = self.drawstate.projection;
        let program_id = self.drawstate.program;
        let program = program_id.and_then(|id| self.program_cache.iter_mut().find(|e| e.id == id));
        if let Some(program) = program {
            if program.location(Uniform::Projection) != -1
                && !same_bits(program.projection.as_flattened(), projection.as_flattened())
            {
                gl.uniform_matrix4(program.location(Uniform::Projection), &projection);
                program.projection = projection;
            }
        }

        let gl = &self.gl;
        if blend != self.drawstate.blend {
            if blend == BlendMode::NONE {
                gl.disable(GL_BLEND);
            } else {
                gl.enable(GL_BLEND);
                gl.blend_func_separate(
                    get_blend_func(blend.src_color_factor()),
                    get_blend_func(blend.dst_color_factor()),
                    get_blend_func(blend.src_alpha_factor()),
                    get_blend_func(blend.dst_alpha_factor()),
                );
                gl.blend_equation_separate(
                    get_blend_equation(blend.color_operation()),
                    get_blend_equation(blend.alpha_operation()),
                );
            }
            self.drawstate.blend = blend;
        }

        // all drawing commands use this
        // SAFETY: as above, for the positions and colors.
        unsafe {
            gl.vertex_attrib_pointer(
                Attribute::Position as GLuint,
                2,
                GL_FALSE,
                stride_bytes,
                base,
            );
            gl.vertex_attrib_pointer(
                Attribute::Color as GLuint,
                4,
                GL_TRUE, /* Normalized */
                stride_bytes,
                base.wrapping_add(2),
            );
        }

        Ok(())
    }

    /// Translation of `SetCopyState()`.
    fn set_copy_state(&mut self, cmd: &DrawCmd, textures: &mut TextureStore) -> Result<()> {
        use ImageSource as I;
        use PixelFormat as F;
        let handle = cmd.texture.ok_or_else(invalid_texture)?;
        let texture = textures.get_mut(handle).ok_or_else(invalid_texture)?;
        let format = texture.format;

        // Pick an appropriate shader
        let source_type = match self.target {
            Some(target) => {
                // Check if we need to do color mapping between the source and render target textures
                if target.format != format {
                    match format {
                        F::INDEX8 => I::TextureIndex8,
                        F::BGRA32 => match target.format {
                            F::RGBA32 | F::RGBX32 => I::TextureArgb,
                            F::BGRX32 => I::TextureAbgr,
                            _ => I::TextureAbgr,
                        },
                        F::RGBA32 => match target.format {
                            F::BGRA32 | F::BGRX32 => I::TextureArgb,
                            F::RGBX32 => I::TextureAbgr,
                            _ => I::TextureAbgr,
                        },
                        F::BGRX32 => match target.format {
                            F::RGBA32 => I::TextureArgb,
                            F::BGRA32 => I::TextureBgr,
                            F::RGBX32 => I::TextureArgb,
                            _ => I::TextureAbgr,
                        },
                        F::RGBX32 => match target.format {
                            F::RGBA32 => I::TextureBgr,
                            F::BGRA32 => I::TextureRgb,
                            F::BGRX32 => I::TextureArgb,
                            _ => I::TextureAbgr,
                        },
                        F::IYUV | F::YV12 | F::I444 => I::TextureYuv,
                        F::NV12 => I::TextureNv12,
                        F::NV21 => I::TextureNv21,
                        F::EXTERNAL_OES => I::TextureExternalOes,
                        _ => return Err(Error::new("Unsupported texture format")),
                    }
                } else {
                    I::TextureAbgr // Texture formats match, use the non color mapping shader (even if the formats are not ABGR)
                }
            }
            None => match format {
                F::INDEX8 => I::TextureIndex8,
                F::BGRA32 => I::TextureArgb,
                F::RGBA32 => I::TextureAbgr,
                F::BGRX32 => I::TextureRgb,
                F::RGBX32 => I::TextureBgr,
                F::IYUV | F::YV12 | F::I444 => I::TextureYuv,
                F::NV12 => I::TextureNv12,
                F::NV21 => I::TextureNv21,
                F::EXTERNAL_OES => I::TextureExternalOes,
                _ => return Err(Error::new("Unsupported texture format")),
            },
        };

        let ret = self.set_draw_state(cmd, source_type, Some(texture));

        let has_palette = texture.palette.is_some();
        let gl = &self.gl;
        let tdata = gles2_texture_mut(texture).ok_or_else(invalid_texture)?;
        let textype = tdata.texture_type;

        if Some(handle) != self.drawstate.texture {
            if tdata.yuv {
                gl.active_texture(GL_TEXTURE2);
                gl.bind_texture(textype, tdata.texture_v);

                gl.active_texture(GL_TEXTURE1);
                gl.bind_texture(textype, tdata.texture_u);

                gl.active_texture(GL_TEXTURE0);
            } else if tdata.nv12 {
                gl.active_texture(GL_TEXTURE1);
                gl.bind_texture(textype, tdata.texture_u);

                gl.active_texture(GL_TEXTURE0);
            }
            if has_palette {
                gl.active_texture(GL_TEXTURE1);
                gl.bind_texture(textype, tdata.palette_texture);

                gl.active_texture(GL_TEXTURE0);
            }
            gl.bind_texture(textype, tdata.texture);

            self.drawstate.texture = Some(handle);
        }

        if cmd.texture_scale_mode != tdata.texture_scale_mode {
            if tdata.yuv {
                gl.active_texture(GL_TEXTURE2);
                set_texture_scale_mode(gl, textype, format, cmd.texture_scale_mode);

                gl.active_texture(GL_TEXTURE1);
                set_texture_scale_mode(gl, textype, format, cmd.texture_scale_mode);

                gl.active_texture(GL_TEXTURE0);
            } else if tdata.nv12 {
                gl.active_texture(GL_TEXTURE1);
                set_texture_scale_mode(gl, textype, format, cmd.texture_scale_mode);

                gl.active_texture(GL_TEXTURE0);
            }
            if has_palette {
                gl.active_texture(GL_TEXTURE1);
                set_texture_scale_mode(gl, textype, PixelFormat::UNKNOWN, ScaleMode::Nearest);

                gl.active_texture(GL_TEXTURE0);
            }
            set_texture_scale_mode(gl, textype, format, cmd.texture_scale_mode);

            tdata.texture_scale_mode = cmd.texture_scale_mode;
        }

        if cmd.texture_address_mode_u != tdata.texture_address_mode_u
            || cmd.texture_address_mode_v != tdata.texture_address_mode_v
        {
            let (u, v) = (cmd.texture_address_mode_u, cmd.texture_address_mode_v);
            if tdata.yuv {
                gl.active_texture(GL_TEXTURE2);
                set_texture_address_mode(gl, textype, u, v);

                gl.active_texture(GL_TEXTURE1);
                set_texture_address_mode(gl, textype, u, v);

                gl.active_texture(GL_TEXTURE0);
            } else if tdata.nv12 {
                gl.active_texture(GL_TEXTURE1);
                set_texture_address_mode(gl, textype, u, v);

                gl.active_texture(GL_TEXTURE0);
            }
            if has_palette {
                gl.active_texture(GL_TEXTURE1);
                set_texture_address_mode(
                    gl,
                    textype,
                    TextureAddressMode::Clamp,
                    TextureAddressMode::Clamp,
                );

                gl.active_texture(GL_TEXTURE0);
            }
            set_texture_address_mode(gl, textype, u, v);

            tdata.texture_address_mode_u = u;
            tdata.texture_address_mode_v = v;
        }

        ret
    }

    /// Draw `count` vertices of the current state.
    fn draw_arrays(&self, mode: GLenum, cmd: &DrawCmd, count: usize) {
        let stride = if self.drawstate.texturing {
            VERTEX_FLOATS
        } else {
            VERTEX_SOLID_FLOATS
        };
        debug_assert!(cmd.first + count * stride <= self.verts.len());
        // SAFETY: set_draw_state() pointed the vertex arrays at `cmd.first`
        // in `self.verts`, which holds the `count` vertices of the
        // command(s) drawn.
        unsafe { self.gl.draw_arrays(mode, 0, count as GLsizei) };
    }

    /// The creation of the renderer once the window is an OpenGL ES window
    /// (the middle of `GLES2_CreateRenderer()`).
    fn create(window: Window, major: i32) -> Result<Gles2Renderer> {
        // Create an OpenGL ES 2.0 context
        let _ = gl::gl_set_attribute(GlAttr::FramebufferSrgbCapable, 0);
        let context = GlContext::new(&window)?;
        context.make_current(Some(&window))?;

        let gl = Gles2Funcs::load()?;

        let mut data = Gles2Renderer {
            window,
            context: Some(context),
            debug_enabled: false,
            gl_oes_egl_image_external_supported: false,
            gl_ext_blend_minmax_supported: false,
            gl,
            window_framebuffer: 0,
            shader_id_cache: [0; ShaderType::COUNT],
            program_cache: Vec::new(),
            drawstate: DrawStateCache {
                viewport: Rect::default(),
                viewport_dirty: false,
                texture: None,
                target: None,
                blend: BlendMode::NONE,
                cliprect_enabled_dirty: false,
                cliprect_enabled: false,
                cliprect_dirty: false,
                cliprect: Rect::default(),
                texturing: false,
                texturing_dirty: false,
                clear_color: FColor::default(),
                clear_color_dirty: false,
                drawablew: 0,
                drawableh: 0,
                program: None,
                projection: [[0.0; 4]; 4],
            },
            texcoord_precision_hint: ShaderInclude::None,
            target: None,
            verts: Vec::new(),
            texture_formats: Vec::new(),
            npot_texture_wrap_unsupported: false,
            max_texture_size: 0,
        };
        data.invalidate_cached_state();

        data.cache_shaders()?;

        // Check for debug output support
        if gl::gl_get_attribute(GlAttr::ContextFlags)
            .is_ok_and(|value| value & gl::GL_CONTEXT_DEBUG_FLAG != 0)
        {
            data.debug_enabled = true;
        }

        data.max_texture_size = data.gl.get_integer(GL_MAX_TEXTURE_SIZE);

        // (USE_VERTEX_BUFFER_OBJECTS is Emscripten's)

        data.window_framebuffer = data.gl.get_integer(GL_FRAMEBUFFER_BINDING) as GLuint;

        use PixelFormat as F;
        data.texture_formats = vec![
            F::BGRA32, // SDL_PIXELFORMAT_ARGB8888 on little endian systems
            F::RGBA32,
            F::BGRX32,
            F::RGBX32,
            F::INDEX8,
            F::YV12,
            F::IYUV,
            F::I444,
            F::NV12,
            F::NV21,
        ];
        if gl::gl_extension_supported("GL_OES_EGL_image_external") {
            data.gl_oes_egl_image_external_supported = true;
            data.cache_shader(ShaderType::FragmentTextureExternalOes, GL_FRAGMENT_SHADER)?;
            data.texture_formats.push(F::EXTERNAL_OES);
        }

        if gl::gl_extension_supported("GL_EXT_blend_minmax") {
            data.gl_ext_blend_minmax_supported = true;
        }

        // Full NPOT textures (that can use GL_REPEAT, etc) are a core feature of GLES3,
        //  and an extension in GLES2.
        if major < 3
            && !gl::gl_extension_supported("GL_ARB_texture_non_power_of_two")
            && !gl::gl_extension_supported("GL_OES_texture_npot")
        {
            data.npot_texture_wrap_unsupported = true;
        }

        if gl::gl_extension_supported("GL_EXT_sRGB_write_control") {
            data.gl.disable(GL_FRAMEBUFFER_SRGB);
        }

        // Set up parameters for rendering
        let gl = &data.gl;
        gl.disable(GL_DEPTH_TEST);
        gl.disable(GL_CULL_FACE);
        gl.active_texture(GL_TEXTURE0);
        gl.pixel_storei(GL_PACK_ALIGNMENT, 1);
        gl.pixel_storei(GL_UNPACK_ALIGNMENT, 1);

        gl.enable_vertex_attrib_array(Attribute::Position as GLuint);
        gl.enable_vertex_attrib_array(Attribute::Color as GLuint);
        gl.disable_vertex_attrib_array(Attribute::TexCoord as GLuint);

        gl.clear_color(1.0, 1.0, 1.0, 1.0);

        data.drawstate.clear_color = FColor::new(1.0, 1.0, 1.0, 1.0);
        data.drawstate.projection[3][0] = -1.0;
        data.drawstate.projection[3][3] = 1.0;

        let _ = gl_check_error!(data, "", "GLES2_CreateRenderer");

        Ok(data)
    }

    /// The renderer for `window`, which is made an OpenGL ES window if it
    /// isn't one. Translation of `GLES2_CreateRenderer()`.
    pub(crate) fn for_window(
        window: Window,
        output_colorspace: Colorspace,
    ) -> Result<Gles2Renderer> {
        // (SDL_SetupRendererColorspace())
        if output_colorspace != Colorspace::SRGB {
            return Err(Error::new("Unsupported output colorspace"));
        }

        let profile_mask = gl::gl_get_attribute(GlAttr::ContextProfileMask)?;
        let major = gl::gl_get_attribute(GlAttr::ContextMajorVersion)?;
        let minor = gl::gl_get_attribute(GlAttr::ContextMinorVersion)?;

        let _ = window.sync();
        let window_flags = window.flags()?;

        let mut changed_window = false;
        let result = (|| {
            // OpenGL ES 3.0 is a superset of OpenGL ES 2.0
            if !window_flags.contains(WindowFlags::OPENGL)
                || profile_mask != gl::GL_CONTEXT_PROFILE_ES
                || major < RENDERER_CONTEXT_MAJOR
            {
                changed_window = true;
                let _ = gl::gl_set_attribute(GlAttr::ContextProfileMask, gl::GL_CONTEXT_PROFILE_ES);
                let _ = gl::gl_set_attribute(GlAttr::ContextMajorVersion, RENDERER_CONTEXT_MAJOR);
                let _ = gl::gl_set_attribute(GlAttr::ContextMinorVersion, RENDERER_CONTEXT_MINOR);

                window.reconfigure(
                    (window_flags & !(WindowFlags::VULKAN | WindowFlags::METAL))
                        | WindowFlags::OPENGL,
                )?;
            }
            Gles2Renderer::create(window, major)
        })();

        result.inspect_err(|_| {
            if changed_window {
                // Uh oh, better try to put it back...
                let _ = gl::gl_set_attribute(GlAttr::ContextProfileMask, profile_mask);
                let _ = gl::gl_set_attribute(GlAttr::ContextMajorVersion, major);
                let _ = gl::gl_set_attribute(GlAttr::ContextMinorVersion, minor);
                let _ = window.recreate(window_flags);
            }
        })
    }

    /// Translation of `GLES2_UpdateTexture()` (with the texture's planes in
    /// `pixels`, `pitch` bytes per row).
    fn upload_texture(
        &mut self,
        texture: &TextureData,
        rect: &Rect,
        pixels: &[u8],
        pitch: usize,
    ) -> Result<()> {
        let _ = self.activate();

        // Bail out if we're supposed to update an empty rectangle
        if rect.w <= 0 || rect.h <= 0 {
            return Ok(());
        }

        self.drawstate.texture = None; // we trash this state.

        let gl = &self.gl;
        let tdata = gles2_texture(texture).ok_or_else(invalid_texture)?;
        let textype = tdata.texture_type;
        let (x, y, w, h) = (rect.x, rect.y, rect.w, rect.h);
        let rows = h as usize;

        // Create a texture subimage with the supplied data
        gl.bind_texture(textype, tdata.texture);
        tex_sub_image_2d(
            gl,
            textype,
            x,
            y,
            w,
            h,
            tdata.pixel_format,
            tdata.pixel_type,
            pixels,
            pitch,
            texture.format.bytes_per_pixel() as usize,
        )?;

        if tdata.yuv {
            if texture.format == PixelFormat::I444 {
                // Skip to the correct offset into the next texture
                let pixels = plane(pixels, rows * pitch)?;
                gl.bind_texture(textype, tdata.texture_u);
                tex_sub_image_2d(
                    gl,
                    textype,
                    x,
                    y,
                    w,
                    h,
                    tdata.pixel_format,
                    tdata.pixel_type,
                    pixels,
                    pitch,
                    1,
                )?;

                // Skip to the correct offset into the next texture
                let pixels = plane(pixels, rows * pitch)?;
                gl.bind_texture(textype, tdata.texture_v);
                tex_sub_image_2d(
                    gl,
                    textype,
                    x,
                    y,
                    w,
                    h,
                    tdata.pixel_format,
                    tdata.pixel_type,
                    pixels,
                    pitch,
                    1,
                )?;
            } else {
                let half_pitch = pitch.div_ceil(2);
                // Skip to the correct offset into the next texture
                let pixels = plane(pixels, rows * pitch)?;
                if texture.format == PixelFormat::YV12 {
                    gl.bind_texture(textype, tdata.texture_v);
                } else {
                    gl.bind_texture(textype, tdata.texture_u);
                }
                tex_sub_image_2d(
                    gl,
                    textype,
                    x / 2,
                    y / 2,
                    (w + 1) / 2,
                    (h + 1) / 2,
                    tdata.pixel_format,
                    tdata.pixel_type,
                    pixels,
                    half_pitch,
                    1,
                )?;

                // Skip to the correct offset into the next texture
                let pixels = plane(pixels, rows.div_ceil(2) * half_pitch)?;
                if texture.format == PixelFormat::YV12 {
                    gl.bind_texture(textype, tdata.texture_u);
                } else {
                    gl.bind_texture(textype, tdata.texture_v);
                }
                tex_sub_image_2d(
                    gl,
                    textype,
                    x / 2,
                    y / 2,
                    (w + 1) / 2,
                    (h + 1) / 2,
                    tdata.pixel_format,
                    tdata.pixel_type,
                    pixels,
                    half_pitch,
                    1,
                )?;
            }
        } else if tdata.nv12 {
            // Skip to the correct offset into the next texture
            let pixels = plane(pixels, rows * pitch)?;
            gl.bind_texture(textype, tdata.texture_u);
            tex_sub_image_2d(
                gl,
                textype,
                x / 2,
                y / 2,
                (w + 1) / 2,
                (h + 1) / 2,
                GL_LUMINANCE_ALPHA,
                GL_UNSIGNED_BYTE,
                pixels,
                2 * pitch.div_ceil(2),
                2,
            )?;
        }

        gl_check_error!(self, "glTexSubImage2D()", "GLES2_UpdateTexture")
    }
}

impl RenderBackend for Gles2Renderer {
    fn name(&self) -> &'static str {
        GLES2_RENDERER
    }

    fn output_size(&self, _textures: &TextureStore) -> Option<Result<(i32, i32)>> {
        None // (the window's size in pixels)
    }

    fn texture_formats(&self) -> Option<Vec<PixelFormat>> {
        Some(self.texture_formats.clone())
    }

    fn npot_texture_wrap_unsupported(&self) -> bool {
        self.npot_texture_wrap_unsupported
    }

    fn max_texture_size(&self) -> Option<i32> {
        Some(self.max_texture_size)
    }

    /// Translation of `GLES2_SupportsBlendMode()`.
    fn supports_blend_mode(&self, blend_mode: BlendMode) -> bool {
        let src_color_factor = blend_mode.src_color_factor();
        let src_alpha_factor = blend_mode.src_alpha_factor();
        let color_operation = blend_mode.color_operation();
        let dst_color_factor = blend_mode.dst_color_factor();
        let dst_alpha_factor = blend_mode.dst_alpha_factor();
        let alpha_operation = blend_mode.alpha_operation();

        if get_blend_func(src_color_factor) == GL_INVALID_ENUM
            || get_blend_func(src_alpha_factor) == GL_INVALID_ENUM
            || get_blend_equation(color_operation) == GL_INVALID_ENUM
            || get_blend_func(dst_color_factor) == GL_INVALID_ENUM
            || get_blend_func(dst_alpha_factor) == GL_INVALID_ENUM
            || get_blend_equation(alpha_operation) == GL_INVALID_ENUM
        {
            return false;
        }

        if color_operation == Some(BlendOperation::Minimum) && !self.gl_ext_blend_minmax_supported {
            return false;
        }
        if color_operation == Some(BlendOperation::Maximum) && !self.gl_ext_blend_minmax_supported {
            return false;
        }

        true
    }

    /// Translation of `GLES2_CreateTexture()`.
    fn create_texture(
        &mut self,
        texture: &mut TextureData,
        _props: &TextureCreateProps,
    ) -> Result<()> {
        use PixelFormat as F;
        let _ = self.activate();

        self.drawstate.texture = None; // we trash this state.

        // Determine the corresponding GLES texture format params
        let (format, ty) = match texture.format {
            F::BGRA32 | F::RGBA32 | F::BGRX32 | F::RGBX32 => (GL_RGBA, GL_UNSIGNED_BYTE),
            F::INDEX8 | F::IYUV | F::YV12 | F::I444 | F::NV12 | F::NV21 => {
                (GL_LUMINANCE, GL_UNSIGNED_BYTE)
            }
            F::EXTERNAL_OES if self.gl_oes_egl_image_external_supported => (GL_NONE, GL_NONE),
            _ => return Err(Error::new("Texture format not supported")),
        };

        if texture.format == F::EXTERNAL_OES && texture.access != TextureAccess::Static {
            return Err(Error::new(
                "Unsupported texture access for SDL_PIXELFORMAT_EXTERNAL_OES",
            ));
        }

        let (w, h) = (texture.w, texture.h);
        let mut data = Gles2Texture {
            texture: 0,
            fbo: 0,
            texture_external: false,
            texture_type: if texture.format == F::EXTERNAL_OES {
                GL_TEXTURE_EXTERNAL_OES
            } else {
                GL_TEXTURE_2D
            },
            pixel_format: format,
            pixel_type: ty,
            pixel_data: Vec::new(),
            pitch: 0,
            yuv: matches!(texture.format, F::IYUV | F::YV12 | F::I444),
            nv12: matches!(texture.format, F::NV12 | F::NV21),
            texture_v: 0,
            texture_v_external: false,
            texture_u: 0,
            texture_u_external: false,
            texel_size: [0.0; 4],
            texture_scale_mode: texture.scale_mode,
            texture_address_mode_u: TextureAddressMode::Clamp,
            texture_address_mode_v: TextureAddressMode::Clamp,
            palette_texture: 0,
        };

        // Allocate a blob for image renderdata
        if texture.access == TextureAccess::Streaming {
            data.pitch = w * texture.format.bytes_per_pixel() as i32;
            let pitch = data.pitch as usize;
            let rows = h as usize;
            let mut size = rows * pitch;
            if data.yuv {
                // Need to add size for the U and V planes
                if texture.format == F::I444 {
                    size += 2 * rows * pitch;
                } else {
                    size += 2 * rows.div_ceil(2) * pitch.div_ceil(2);
                }
            } else if data.nv12 {
                // Need to add size for the U/V plane
                size += 2 * rows.div_ceil(2) * pitch.div_ceil(2);
            }
            data.pixel_data = vec![0; size];
        }

        // Allocate the texture
        let _ = gl_check_error!(self, "", "GLES2_CreateTexture");

        data.texel_size = [1.0 / w as f32, 1.0 / h as f32, w as f32, h as f32];

        let props = texture.props.get_or_insert_with(Properties::new).clone();
        let gl = &self.gl;
        let textype = data.texture_type;

        // FIXME (upstream): the U and V (or UV) textures leak when a later
        // step of the creation fails.
        if data.yuv {
            let (yuv_texture_w, yuv_texture_h) = if texture.format == F::I444 {
                (w, h)
            } else {
                ((w + 1) / 2, (h + 1) / 2)
            };

            // (SDL_PROP_TEXTURE_CREATE_OPENGLES2_TEXTURE_V_NUMBER isn't taken)
            data.texture_v = gl.gen_texture();
            gl_check_error!(self, "glGenTexures()", "GLES2_CreateTexture")?;
            gl.active_texture(GL_TEXTURE2);
            gl.bind_texture(textype, data.texture_v);
            gl.tex_image_2d_empty(textype, format, yuv_texture_w, yuv_texture_h, format, ty);
            gl_check_error!(self, "glTexImage2D()", "GLES2_CreateTexture")?;
            set_texture_scale_mode(gl, textype, texture.format, data.texture_scale_mode);
            set_texture_address_mode(
                gl,
                textype,
                data.texture_address_mode_u,
                data.texture_address_mode_v,
            );
            let _ = props.set(
                PROP_TEXTURE_OPENGLES2_TEXTURE_V_NUMBER,
                data.texture_v as i64,
            );

            data.texture_u = gl.gen_texture();
            gl_check_error!(self, "glGenTexures()", "GLES2_CreateTexture")?;
            gl.active_texture(GL_TEXTURE1);
            gl.bind_texture(textype, data.texture_u);
            gl.tex_image_2d_empty(textype, format, yuv_texture_w, yuv_texture_h, format, ty);
            gl_check_error!(self, "glTexImage2D()", "GLES2_CreateTexture")?;
            set_texture_scale_mode(gl, textype, texture.format, data.texture_scale_mode);
            set_texture_address_mode(
                gl,
                textype,
                data.texture_address_mode_u,
                data.texture_address_mode_v,
            );
            let _ = props.set(
                PROP_TEXTURE_OPENGLES2_TEXTURE_U_NUMBER,
                data.texture_u as i64,
            );

            if texture
                .colorspace
                .ycbcr_to_rgb_matrix(h.max(0) as u32, 8)
                .is_none()
            {
                return Err(Error::new("Unsupported YUV colorspace"));
            }
        } else if data.nv12 {
            data.texture_u = gl.gen_texture();
            gl_check_error!(self, "glGenTexures()", "GLES2_CreateTexture")?;
            gl.active_texture(GL_TEXTURE1);
            gl.bind_texture(textype, data.texture_u);
            gl.tex_image_2d_empty(
                textype,
                GL_LUMINANCE_ALPHA,
                (w + 1) / 2,
                (h + 1) / 2,
                GL_LUMINANCE_ALPHA,
                GL_UNSIGNED_BYTE,
            );
            gl_check_error!(self, "glTexImage2D()", "GLES2_CreateTexture")?;
            set_texture_scale_mode(gl, textype, texture.format, data.texture_scale_mode);
            set_texture_address_mode(
                gl,
                textype,
                data.texture_address_mode_u,
                data.texture_address_mode_v,
            );
            let _ = props.set(
                PROP_TEXTURE_OPENGLES2_TEXTURE_UV_NUMBER,
                data.texture_u as i64,
            );

            if texture
                .colorspace
                .ycbcr_to_rgb_matrix(h.max(0) as u32, 8)
                .is_none()
            {
                return Err(Error::new("Unsupported YUV colorspace"));
            }
        }

        // (SDL_PROP_TEXTURE_CREATE_OPENGLES2_TEXTURE_NUMBER isn't taken)
        data.texture = gl.gen_texture();
        gl_check_error!(self, "glGenTexures()", "GLES2_CreateTexture")?;
        gl.active_texture(GL_TEXTURE0);
        gl.bind_texture(textype, data.texture);
        if texture.format != F::EXTERNAL_OES {
            gl.tex_image_2d_empty(textype, format, w, h, format, ty);
            if let Err(e) = gl_check_error!(self, "glTexImage2D()", "GLES2_CreateTexture") {
                if !data.texture_external {
                    gl.delete_texture(data.texture);
                }
                return Err(e);
            }
        }
        set_texture_scale_mode(gl, textype, texture.format, data.texture_scale_mode);
        set_texture_address_mode(
            gl,
            textype,
            data.texture_address_mode_u,
            data.texture_address_mode_v,
        );
        let _ = props.set(PROP_TEXTURE_OPENGLES2_TEXTURE_NUMBER, data.texture as i64);
        let _ = props.set(PROP_TEXTURE_OPENGLES2_TEXTURE_TARGET_NUMBER, textype as i64);

        if texture.access == TextureAccess::Target {
            data.fbo = gl.gen_framebuffer();
            gl.bind_framebuffer(GL_FRAMEBUFFER, data.fbo);
            gl.framebuffer_texture_2d(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, textype, data.texture);
            let status = gl.check_framebuffer_status(GL_FRAMEBUFFER);
            // rebind previous fbo.
            gl.bind_framebuffer(
                GL_FRAMEBUFFER,
                self.target.map_or(self.window_framebuffer, |t| t.fbo),
            );
            if status != GL_FRAMEBUFFER_COMPLETE {
                gl.delete_framebuffer(data.fbo);
                if !data.texture_external {
                    gl.delete_texture(data.texture);
                }
                return Err(Error::new("Texture framebuffer was incomplete"));
            }
        }

        texture.internal = Some(Box::new(data));

        gl_check_error!(self, "", "GLES2_CreateTexture")
    }

    fn queue_set_viewport(&mut self, _cmd: &mut RenderCommand) -> Result<()> {
        Ok(()) // nothing to do in this backend.
    }

    fn queue_set_draw_color(&mut self, _cmd: &mut RenderCommand) -> Result<()> {
        Ok(()) // nothing to do in this backend.
    }

    /// Translation of `GLES2_QueueDrawPoints()`.
    fn queue_draw_points(&mut self, cmd: &mut DrawCmd, points: &[FPoint]) -> Result<()> {
        let color = self.vertex_color(cmd.color, cmd.color_scale);

        cmd.first = self.verts.len();
        cmd.count = points.len();
        for p in points {
            self.push_vertex_solid(0.5 + p.x, 0.5 + p.y, color);
        }

        Ok(())
    }

    /// Translation of `GLES2_QueueDrawLines()`.
    fn queue_draw_lines(&mut self, cmd: &mut DrawCmd, points: &[FPoint]) -> Option<Result<()>> {
        let color = self.vertex_color(cmd.color, cmd.color_scale);
        let Some(first) = points.first() else {
            return Some(Ok(()));
        };

        cmd.first = self.verts.len();
        cmd.count = points.len();

        // 0.5f offset to hit the center of the pixel.
        let mut prevx = 0.5 + first.x;
        let mut prevy = 0.5 + first.y;
        self.push_vertex_solid(prevx, prevy, color);

        /* bump the end of each line segment out a quarter of a pixel, to provoke
        the diamond-exit rule. Without this, you won't just drop the last
        pixel of the last line segment, but you might also drop pixels at the
        edge of any given line segment along the way too. */
        for p in &points[1..] {
            let xstart = prevx;
            let ystart = prevy;
            let xend = p.x + 0.5; // 0.5f to hit pixel center.
            let yend = p.y + 0.5;
            // bump a little in the direction we are moving in.
            let deltax = xend - xstart;
            let deltay = yend - ystart;
            let angle = deltay.atan2(deltax);
            prevx = xend + (angle.cos() * 0.25);
            prevy = yend + (angle.sin() * 0.25);
            self.push_vertex_solid(prevx, prevy, color);
        }

        Some(Ok(()))
    }

    fn has_queue_fill_rects(&self) -> bool {
        false
    }

    fn queue_fill_rects(&mut self, _cmd: &mut DrawCmd, _rects: &[FRect]) -> Result<()> {
        Err(Error::unsupported())
    }

    fn has_queue_copy(&self) -> bool {
        false
    }

    fn queue_copy(
        &mut self,
        _cmd: &mut DrawCmd,
        _texture: &TextureData,
        _srcrect: &FRect,
        _dstrect: &FRect,
    ) -> Result<()> {
        Err(Error::unsupported())
    }

    fn has_queue_copy_ex(&self) -> bool {
        false
    }

    fn queue_copy_ex(
        &mut self,
        _cmd: &mut DrawCmd,
        _texture: &TextureData,
        _copy: &CopyEx,
    ) -> Result<()> {
        Err(Error::unsupported())
    }

    fn has_queue_geometry(&self) -> bool {
        true
    }

    /// Translation of `GLES2_QueueGeometry()`.
    fn queue_geometry(
        &mut self,
        cmd: &mut DrawCmd,
        texture: Option<&TextureData>,
        geometry: &Geometry<'_>,
        scale_x: f32,
        scale_y: f32,
    ) -> Result<()> {
        let count = geometry.count();
        let color_scale = cmd.color_scale;

        cmd.count = count;
        cmd.first = self.verts.len();

        for i in 0..count {
            let j = geometry.vertex(i);
            let (x, y) = geometry.xy(j);
            let col = self.vertex_color(geometry.color(j), color_scale);
            self.push_vertex_solid(x * scale_x, y * scale_y, col);
            if texture.is_some() {
                let (u, v) = geometry.uv(j);
                self.verts.extend_from_slice(&[u, v]);
            }
        }

        Ok(())
    }

    /// Translation of `GLES2_InvalidateCachedState()`.
    fn invalidate_cached_state(&mut self) {
        let cache = &mut self.drawstate;
        cache.viewport_dirty = true;
        cache.texture = None;
        cache.blend = BlendMode::INVALID;
        cache.cliprect_enabled_dirty = true;
        cache.cliprect_dirty = true;
        cache.texturing_dirty = true;
        cache.clear_color_dirty = true;
        cache.drawablew = 0;
        cache.drawableh = 0;
        cache.program = None;
    }

    /// Translation of `GLES2_RunCommandQueue()`.
    fn run_command_queue(
        &mut self,
        cmds: &[RenderCommand],
        textures: &mut TextureStore,
        _gpu_render_states: &crate::render::sysrender::GpuRenderStates,
    ) -> Result<()> {
        let colorswap = self.colorswap();

        self.activate()?;

        self.drawstate.target = self.target;
        if self.drawstate.target.is_none() {
            let (w, h) = self.window.size_in_pixels().unwrap_or((0, 0));
            if w != self.drawstate.drawablew || h != self.drawstate.drawableh {
                self.drawstate.viewport_dirty = true; // if the window dimensions changed, invalidate the current viewport, etc.
                self.drawstate.cliprect_dirty = true;
                self.drawstate.drawablew = w;
                self.drawstate.drawableh = h;
            }
        }

        let mut i = 0;
        while i < cmds.len() {
            match cmds[i] {
                RenderCommand::SetDrawColor { .. } => {}

                RenderCommand::SetViewport { rect, .. } => {
                    if self.drawstate.viewport != rect {
                        self.drawstate.viewport = rect;
                        self.drawstate.viewport_dirty = true;
                        self.drawstate.cliprect_dirty = true;
                    }
                }

                RenderCommand::SetClipRect { enabled, rect } => {
                    if self.drawstate.cliprect_enabled != enabled {
                        self.drawstate.cliprect_enabled = enabled;
                        self.drawstate.cliprect_enabled_dirty = true;
                    }

                    if self.drawstate.cliprect != rect {
                        self.drawstate.cliprect = rect;
                        self.drawstate.cliprect_dirty = true;
                    }
                }

                RenderCommand::Clear {
                    color, color_scale, ..
                } => {
                    let r = if colorswap { color.b } else { color.r } * color_scale;
                    let g = color.g * color_scale;
                    let b = if colorswap { color.r } else { color.b } * color_scale;
                    let a = color.a;
                    let ds = &mut self.drawstate;
                    if ds.clear_color_dirty
                        || r != ds.clear_color.r
                        || g != ds.clear_color.g
                        || b != ds.clear_color.b
                        || a != ds.clear_color.a
                    {
                        self.gl.clear_color(r, g, b, a);
                        ds.clear_color = FColor::new(r, g, b, a);
                        ds.clear_color_dirty = false;
                    }

                    if ds.cliprect_enabled || ds.cliprect_enabled_dirty {
                        self.gl.disable(GL_SCISSOR_TEST);
                        ds.cliprect_enabled_dirty = ds.cliprect_enabled;
                    }

                    self.gl.clear(GL_COLOR_BUFFER_BIT);
                }

                RenderCommand::Draw(DrawKind::FillRects | DrawKind::Copy | DrawKind::CopyEx, _) => {
                    // unused
                }

                RenderCommand::Draw(DrawKind::Lines, d) => {
                    if self.set_draw_state(&d, ImageSource::Solid, None).is_ok() {
                        let mut count = d.count;
                        if count > 2 {
                            // joined lines cannot be grouped
                            self.draw_arrays(GL_LINE_STRIP, &d, count);
                        } else {
                            // let's group non joined lines
                            let mut finalcmd = i;
                            let thisblend = d.blend;

                            for (j, next) in cmds.iter().enumerate().skip(i + 1) {
                                let RenderCommand::Draw(DrawKind::Lines, n) = next else {
                                    break; // can't go any further on this draw call, different render command up next.
                                };
                                if n.count != 2 {
                                    break; // can't go any further on this draw call, those are joined lines
                                } else if n.blend != thisblend {
                                    break; // can't go any further on this draw call, different blendmode copy up next.
                                } else {
                                    finalcmd = j; // we can combine copy operations here. Mark this one as the furthest okay command.
                                    count += n.count;
                                }
                            }

                            self.draw_arrays(GL_LINES, &d, count);
                            i = finalcmd; // skip any copy commands we just combined in here.
                        }
                    }
                }

                RenderCommand::Draw(thiscmdtype @ (DrawKind::Points | DrawKind::Geometry), d) => {
                    /* as long as we have the same copy command in a row, with the
                    same texture, we can combine them all into a single draw call. */
                    let mut finalcmd = i;
                    let mut count = d.count;
                    for (j, next) in cmds.iter().enumerate().skip(i + 1) {
                        let RenderCommand::Draw(nextcmdtype, n) = next else {
                            break; // can't go any further on this draw call, different render command up next.
                        };
                        if *nextcmdtype != thiscmdtype {
                            break; // can't go any further on this draw call, different render command up next.
                        } else if n.texture != d.texture
                            || n.texture_scale_mode != d.texture_scale_mode
                            || n.texture_address_mode_u != d.texture_address_mode_u
                            || n.texture_address_mode_v != d.texture_address_mode_v
                            || n.blend != d.blend
                        {
                            break; // can't go any further on this draw call, different texture/blendmode copy up next.
                        } else {
                            finalcmd = j; // we can combine copy operations here. Mark this one as the furthest okay command.
                            count += n.count;
                        }
                    }

                    let ret = if d.texture.is_some() {
                        self.set_copy_state(&d, textures)
                    } else {
                        self.set_draw_state(&d, ImageSource::Solid, None)
                    };

                    if ret.is_ok() {
                        let op = if thiscmdtype == DrawKind::Points {
                            GL_POINTS
                        } else {
                            GL_TRIANGLES // SDL_RENDERCMD_GEOMETRY
                        };
                        self.draw_arrays(op, &d, count);
                    }

                    i = finalcmd; // skip any copy commands we just combined in here.
                }

                RenderCommand::NoOp => {}
            }

            i += 1;
        }

        gl_check_error!(self, "", "GLES2_RunCommandQueue")
    }

    fn reset_vertices(&mut self) {
        self.verts.clear();
    }

    /// Translation of `GLES2_CreatePalette()`.
    ///
    /// FIXME (upstream): unlike `GLES2_UpdatePalette()`, this (and
    /// `GLES2_DestroyPalette()`) doesn't make the renderer's context current
    /// first.
    fn create_palette(&mut self) -> Result<Box<dyn Any>> {
        self.drawstate.texture = None; // we trash this state.

        let gl = &self.gl;
        let texture = gl.gen_texture();
        gl_check_error!(self, "glGenTexures()", "GLES2_CreatePalette")?;
        gl.bind_texture(GL_TEXTURE_2D, texture);
        gl.tex_image_2d_empty(GL_TEXTURE_2D, GL_RGBA, 256, 1, GL_RGBA, GL_UNSIGNED_BYTE);
        gl_check_error!(self, "glTexImage2D()", "GLES2_CreatePalette")?;
        set_texture_scale_mode(gl, GL_TEXTURE_2D, PixelFormat::UNKNOWN, ScaleMode::Nearest);
        set_texture_address_mode(
            gl,
            GL_TEXTURE_2D,
            TextureAddressMode::Clamp,
            TextureAddressMode::Clamp,
        );
        Ok(Box::new(PaletteData { texture }))
    }

    /// Translation of `GLES2_UpdatePalette()`.
    fn update_palette(&mut self, palette: &mut dyn Any, colors: &[Color]) -> Result<()> {
        let palette = palette
            .downcast_ref::<PaletteData>()
            .ok_or_else(|| Error::invalid_param("palette"))?;

        let _ = self.activate();

        self.drawstate.texture = None; // we trash this state.

        let bytes: Vec<u8> = colors.iter().flat_map(|c| [c.r, c.g, c.b, c.a]).collect();
        self.gl.bind_texture(GL_TEXTURE_2D, palette.texture);
        self.gl.tex_sub_image_2d(
            GL_TEXTURE_2D,
            0,
            0,
            colors.len() as GLsizei,
            1,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            &bytes,
        );

        gl_check_error!(self, "glTexSubImage2D()", "GLES2_UpdatePalette")
    }

    /// Translation of `GLES2_DestroyPalette()`.
    fn destroy_palette(&mut self, palette: Box<dyn Any>) {
        if let Ok(palette) = palette.downcast::<PaletteData>() {
            self.gl.delete_texture(palette.texture);
        }
    }

    /// Record the GL texture of the texture's palette, which the draws bind
    /// (`texture->palette->internal`).
    fn change_texture_palette(
        &mut self,
        texture: &mut TextureData,
        palette: Option<&dyn Any>,
    ) -> Option<Result<()>> {
        let palette_texture = palette
            .and_then(|p| p.downcast_ref::<PaletteData>())
            .map_or(0, |p| p.texture);
        if let Some(tdata) = gles2_texture_mut(texture) {
            tdata.palette_texture = palette_texture;
        }
        self.drawstate.texture = None;
        Some(Ok(()))
    }

    fn update_texture(
        &mut self,
        texture: &mut TextureData,
        rect: &Rect,
        pixels: &[u8],
        pitch: usize,
    ) -> Result<()> {
        self.upload_texture(texture, rect, pixels, pitch)
    }

    /// Translation of `GLES2_UpdateTextureYUV()`.
    fn update_texture_yuv(
        &mut self,
        texture: &mut TextureData,
        rect: &Rect,
        (y_plane, y_pitch): (&[u8], usize),
        (u_plane, u_pitch): (&[u8], usize),
        (v_plane, v_pitch): (&[u8], usize),
    ) -> Option<Result<()>> {
        let _ = self.activate();

        // Bail out if we're supposed to update an empty rectangle
        if rect.w <= 0 || rect.h <= 0 {
            return Some(Ok(()));
        }

        self.drawstate.texture = None; // we trash this state.

        let gl = &self.gl;
        let Some(tdata) = gles2_texture(texture) else {
            return Some(Err(invalid_texture()));
        };
        let textype = tdata.texture_type;
        let (x, y, w, h) = (rect.x, rect.y, rect.w, rect.h);
        let (pf, pt) = (tdata.pixel_format, tdata.pixel_type);

        let result = (|| {
            if texture.format == PixelFormat::I444 {
                gl.bind_texture(textype, tdata.texture_v);
                tex_sub_image_2d(gl, textype, x, y, w, h, pf, pt, v_plane, v_pitch, 1)?;

                gl.bind_texture(textype, tdata.texture_u);
                tex_sub_image_2d(gl, textype, x, y, w, h, pf, pt, u_plane, u_pitch, 1)?;
            } else {
                let (cx, cy, cw, ch) = (x / 2, y / 2, (w + 1) / 2, (h + 1) / 2);
                gl.bind_texture(textype, tdata.texture_v);
                tex_sub_image_2d(gl, textype, cx, cy, cw, ch, pf, pt, v_plane, v_pitch, 1)?;

                gl.bind_texture(textype, tdata.texture_u);
                tex_sub_image_2d(gl, textype, cx, cy, cw, ch, pf, pt, u_plane, u_pitch, 1)?;
            }

            gl.bind_texture(textype, tdata.texture);
            tex_sub_image_2d(gl, textype, x, y, w, h, pf, pt, y_plane, y_pitch, 1)
        })();

        Some(
            result.and_then(|()| {
                gl_check_error!(self, "glTexSubImage2D()", "GLES2_UpdateTextureYUV")
            }),
        )
    }

    /// Translation of `GLES2_UpdateTextureNV()`.
    fn update_texture_nv(
        &mut self,
        texture: &mut TextureData,
        rect: &Rect,
        (y_plane, y_pitch): (&[u8], usize),
        (uv_plane, uv_pitch): (&[u8], usize),
    ) -> Option<Result<()>> {
        let _ = self.activate();

        // Bail out if we're supposed to update an empty rectangle
        if rect.w <= 0 || rect.h <= 0 {
            return Some(Ok(()));
        }

        self.drawstate.texture = None; // we trash this state.

        let gl = &self.gl;
        let Some(tdata) = gles2_texture(texture) else {
            return Some(Err(invalid_texture()));
        };
        let textype = tdata.texture_type;
        let (x, y, w, h) = (rect.x, rect.y, rect.w, rect.h);

        let result = (|| {
            gl.bind_texture(textype, tdata.texture_u);
            tex_sub_image_2d(
                gl,
                textype,
                x / 2,
                y / 2,
                (w + 1) / 2,
                (h + 1) / 2,
                GL_LUMINANCE_ALPHA,
                GL_UNSIGNED_BYTE,
                uv_plane,
                uv_pitch,
                2,
            )?;

            gl.bind_texture(textype, tdata.texture);
            tex_sub_image_2d(
                gl,
                textype,
                x,
                y,
                w,
                h,
                tdata.pixel_format,
                tdata.pixel_type,
                y_plane,
                y_pitch,
                1,
            )
        })();

        Some(
            result
                .and_then(|()| gl_check_error!(self, "glTexSubImage2D()", "GLES2_UpdateTextureNV")),
        )
    }

    /// Translation of `GLES2_LockTexture()`.
    fn lock_texture(&mut self, texture: &mut TextureData, rect: &Rect) -> Result<(usize, i32)> {
        let bpp = texture.format.bytes_per_pixel() as usize;
        let tdata = gles2_texture(texture).ok_or_else(invalid_texture)?;

        // Retrieve the buffer/pitch for the specified region
        let offset = tdata.pitch as usize * rect.y as usize + rect.x as usize * bpp;
        Ok((offset, tdata.pitch))
    }

    fn texture_pixels_mut<'t>(&mut self, texture: &'t mut TextureData) -> Option<&'t mut [u8]> {
        gles2_texture_mut(texture).map(|t| &mut t.pixel_data[..])
    }

    /// Translation of `GLES2_UnlockTexture()`.
    fn unlock_texture(&mut self, texture: &mut TextureData) {
        // We do whole texture updates, at least for now
        let rect = Rect::new(0, 0, texture.w, texture.h);
        let Some(tdata) = gles2_texture_mut(texture) else {
            return;
        };
        let pixels = std::mem::take(&mut tdata.pixel_data);
        let pitch = tdata.pitch as usize;
        let _ = self.upload_texture(texture, &rect, &pixels, pitch);
        if let Some(tdata) = gles2_texture_mut(texture) {
            tdata.pixel_data = pixels;
        }
    }

    /// Translation of `GLES2_SetRenderTarget()`.
    fn set_render_target(
        &mut self,
        target: Option<Texture>,
        textures: &TextureStore,
    ) -> Result<()> {
        self.target = match target {
            Some(t) => {
                let texture = textures.get(t).ok_or_else(invalid_texture)?;
                let tdata = gles2_texture(texture).ok_or_else(invalid_texture)?;
                Some(Target {
                    format: texture.format,
                    fbo: tdata.fbo,
                })
            }
            None => None,
        };
        self.drawstate.viewport_dirty = true;
        self.gl.bind_framebuffer(
            GL_FRAMEBUFFER,
            self.target.map_or(self.window_framebuffer, |t| t.fbo),
        );
        Ok(())
    }

    /// Translation of `GLES2_RenderReadPixels()`.
    fn read_pixels(
        &mut self,
        rect: &Rect,
        _textures: &mut TextureStore,
    ) -> Option<Result<Surface<'static>>> {
        Some((|| {
            let format = self.target.map_or(PixelFormat::RGBA32, |t| t.format);

            let mut surface = Surface::new_uninitialized(rect.w, rect.h, format)?;

            let mut y = rect.y;
            if self.target.is_none() {
                let (_, h) = self.window.size_in_pixels()?;
                y = (h - y) - rect.h;
            }

            // (the rows are read tightly packed, GL_PACK_ALIGNMENT being 1)
            if surface.pitch() != rect.w * 4 {
                return Err(Error::new("Unsupported read pixels format"));
            }
            let pixels = surface
                .pixels_mut()
                .ok_or_else(|| Error::new("Surface pixels are not writable"))?;
            self.gl
                .read_pixels(rect.x, y, rect.w, rect.h, GL_RGBA, GL_UNSIGNED_BYTE, pixels);
            gl_check_error!(self, "glReadPixels()", "GLES2_RenderReadPixels")?;

            // Flip the rows to be top-down if necessary
            if self.target.is_none() {
                surface.flip(FlipMode::Vertical)?;
            }
            Ok(surface)
        })())
    }

    /// Translation of `GLES2_RenderPresent()`.
    fn present(&mut self) -> bool {
        // Tell the video driver to swap buffers
        gl::gl_swap_window(&self.window).is_ok()
    }

    /// Translation of `GLES2_DestroyTexture()`.
    fn destroy_texture(&mut self, texture: &mut TextureData) {
        // (the GL objects are deleted only when the context can be made
        // current: once the window is gone, the deletes would go to
        // whichever context is current, and destroying the renderer's
        // context frees them anyway)
        let active = self.activate().is_ok();

        // (upstream forgets the cached texture only when it is this one;
        // the backend doesn't see the handle here, and a stale cache entry
        // just costs a bind)
        self.drawstate.texture = None;

        // Destroy the texture
        let Some(internal) = texture.internal.take() else {
            return;
        };
        let Ok(tdata) = internal.downcast::<Gles2Texture>() else {
            return;
        };
        if self
            .drawstate
            .target
            .is_some_and(|t| tdata.fbo != 0 && t.fbo == tdata.fbo)
        {
            self.drawstate.target = None;
        }
        if !active {
            return;
        }
        let gl = &self.gl;
        if tdata.fbo != 0 {
            gl.delete_framebuffer(tdata.fbo);
        }
        if tdata.texture != 0 && !tdata.texture_external {
            gl.delete_texture(tdata.texture);
        }
        if tdata.texture_v != 0 && !tdata.texture_v_external {
            gl.delete_texture(tdata.texture_v);
        }
        if tdata.texture_u != 0 && !tdata.texture_u_external {
            gl.delete_texture(tdata.texture_u);
        }
    }

    /// Translation of `GLES2_SetVSync()`.
    fn set_vsync(&mut self, vsync: i32) -> Option<Result<()>> {
        Some((|| {
            gl::gl_set_swap_interval(vsync)?;

            let interval = gl::gl_get_swap_interval()?;

            if interval != vsync {
                return Err(Error::unsupported());
            }
            Ok(())
        })())
    }

    /// Translation of `GLES2_WindowEvent()`.
    fn window_event(&mut self, event_type: EventType) {
        if event_type == EventType::WINDOW_MINIMIZED {
            // According to Apple documentation, we need to finish drawing NOW!
            self.gl.finish();
        }
    }

    /// Translation of `GLES2_DestroyRenderer()`.
    fn destroy(&mut self) {
        // Deallocate everything
        // (only with the context current, as for the textures)
        if self.activate().is_ok() {
            for &id in &self.shader_id_cache {
                if id != 0 {
                    self.gl.delete_shader(id);
                }
            }
            for entry in &self.program_cache {
                self.gl.delete_program(entry.id);
            }
        }
        self.shader_id_cache = [0; ShaderType::COUNT];
        self.program_cache.clear();

        // SDL_GL_DestroyContext()
        self.context = None;
    }
}
