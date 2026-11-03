// Rust translation of src/render/opengles/SDL_render_gles.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The OpenGL ES 1.1 renderer ("opengles"): draws through an OpenGL ES 1.1
//! context of the window with the fixed-function pipeline, with the GL
//! entry points looked up at run time. Upstream builds it with OpenGL ES
//! on Android and the UIKit platforms (CMake's `SDL_VIDEO_RENDER_OGL_ES`),
//! where it is after "opengles2" in the driver list.
//!
//! Its textures are `SDL_PIXELFORMAT_RGBA32` only (the front end converts
//! the others, YUV ones too, to it), of power of two sizes (the texture
//! coordinates are scaled to the part the texture uses); render targets
//! need `GL_OES_framebuffer_object`, and the blend modes that aren't
//! required of every renderer the `GL_OES_blend_*` and
//! `GL_EXT_blend_minmax` extensions they use.

mod funcs;
#[cfg(test)]
mod tests;

use std::any::Any;

use funcs::*;

use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::events::EventType;
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

/// The name of the OpenGL ES 1.1 renderer (`GLES_RenderDriver.name`).
pub(crate) const GLES_RENDERER: &str = "opengles";

const RENDERER_CONTEXT_MAJOR: i32 = 1;
const RENDERER_CONTEXT_MINOR: i32 = 1;

// OpenGL ES 1.1 renderer implementation, based on the OpenGL renderer

/// Translation of `GLES_FBOList` (a list node: here an element of the
/// renderer's `Vec`).
struct FboList {
    w: u32,
    h: u32,
    fbo: GLuint,
}

/// The render target (`renderer->target`), with what the backend reads of
/// it.
///
/// Note: the texture's GL name stands for the `SDL_Texture` upstream's
/// draw state cache points at (here and in [`DrawStateCache::texture`]):
/// the backend's calls don't see the texture's handle, and two live
/// textures never have the same name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Target {
    texture: GLuint,
    format: PixelFormat,
}

/// Translation of `GLES_DrawStateCache`.
struct DrawStateCache {
    viewport: Rect,
    viewport_dirty: bool,
    /// The GL name of the texture bound (see [`Target`]).
    texture: Option<GLuint>,
    target: Option<Target>,
    drawablew: i32,
    drawableh: i32,
    blend: BlendMode,
    cliprect_enabled_dirty: bool,
    cliprect_enabled: bool,
    cliprect_dirty: bool,
    cliprect: Rect,
    texturing: bool,
    color_dirty: bool,
    color: FColor,
    clear_color_dirty: bool,
    clear_color: FColor,
}

/// The renderer's data. Translation of `GLES_RenderData`, with the parts
/// of `SDL_Renderer` the backend reads (`window`, `target`, the limits)
/// and its vertex storage.
pub(crate) struct GlesRenderer {
    window: Window,
    context: Option<GlContext>,

    gl: GlesFuncs,
    gl_oes_framebuffer_object_supported: bool,
    framebuffers: Vec<FboList>,
    window_framebuffer: GLuint,

    gl_oes_blend_func_separate_supported: bool,
    gl_oes_blend_equation_separate_supported: bool,
    gl_oes_blend_subtract_supported: bool,
    gl_ext_blend_minmax_supported: bool,

    drawstate: DrawStateCache,

    /// Probably GL_TEXTURE_2D, but might be other things, like TEXTURE_RECTANGLE, maybe if there's any extension...?
    textype: GLenum,

    /// (set as upstream sets it, and read nowhere, as there)
    #[allow(dead_code)]
    debug_enabled: bool,

    /// `renderer->target`
    target: Option<Target>,
    /// The vertex data of the queued commands (floats; `first` indexes it).
    verts: Vec<f32>,
    max_texture_size: i32,
}

/// Translation of `GLES_TextureData`.
struct GlesTexture {
    texture: GLuint,
    textype: GLenum,
    texw: GLfloat,
    texh: GLfloat,
    format: GLenum,
    formattype: GLenum,
    texture_scale_mode: ScaleMode,
    texture_address_mode_u: TextureAddressMode,
    texture_address_mode_v: TextureAddressMode,
    pixels: Vec<u8>,
    pitch: i32,
    /// The framebuffer object (`fbo->FBO` of its `GLES_FBOList` entry) of
    /// a render target.
    fbo: Option<GLuint>,
}

/// The GLES data of a texture.
fn gles_texture(texture: &TextureData) -> Option<&GlesTexture> {
    texture.internal.as_ref()?.downcast_ref::<GlesTexture>()
}

fn gles_texture_mut(texture: &mut TextureData) -> Option<&mut GlesTexture> {
    texture.internal.as_mut()?.downcast_mut::<GlesTexture>()
}

/// The error of a failed GL call. Translation of `GLES_SetError()`.
fn gles_set_error(prefix: &str, result: GLenum) -> Error {
    let error = match result {
        GL_NO_ERROR => "GL_NO_ERROR",
        GL_INVALID_ENUM => "GL_INVALID_ENUM",
        GL_INVALID_VALUE => "GL_INVALID_VALUE",
        GL_INVALID_OPERATION => "GL_INVALID_OPERATION",
        GL_STACK_OVERFLOW => "GL_STACK_OVERFLOW",
        GL_STACK_UNDERFLOW => "GL_STACK_UNDERFLOW",
        GL_OUT_OF_MEMORY => "GL_OUT_OF_MEMORY",
        _ => "UNKNOWN",
    };
    Error::new(format!("{prefix}: {error}"))
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
        Some(BlendOperation::Add) => GL_FUNC_ADD_OES,
        Some(BlendOperation::Subtract) => GL_FUNC_SUBTRACT_OES,
        Some(BlendOperation::RevSubtract) => GL_FUNC_REVERSE_SUBTRACT_OES,
        Some(BlendOperation::Minimum) => GL_MIN_EXT,
        Some(BlendOperation::Maximum) => GL_MAX_EXT,
        None => GL_INVALID_ENUM,
    }
}

/// Translation of `SetTextureScaleMode()` (every `SDL_ScaleMode` is known
/// here, so it can't fail).
fn set_texture_scale_mode(gl: &GlesFuncs, textype: GLenum, scale_mode: ScaleMode) {
    let filter = match scale_mode {
        ScaleMode::Nearest | ScaleMode::PixelArt => GL_NEAREST,
        ScaleMode::Linear => GL_LINEAR,
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
    gl: &GlesFuncs,
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

impl GlesRenderer {
    /// The framebuffer object for render targets of `w` x `h`.
    /// Translation of `GLES_GetFBO()`.
    fn get_fbo(&mut self, w: u32, h: u32) -> GLuint {
        if let Some(result) = self.framebuffers.iter().find(|f| f.w == w && f.h == h) {
            return result.fbo;
        }
        let fbo = self.gl.gen_framebuffer_oes();
        self.framebuffers.push(FboList { w, h, fbo });
        fbo
    }

    /// Make the renderer's context current. Translation of
    /// `GLES_ActivateRenderer()`.
    fn activate(&self) -> Result<()> {
        let Some(context) = &self.context else {
            return Err(Error::new("The renderer's GL context is gone"));
        };
        if !context.is_current() {
            context.make_current(Some(&self.window))?;
        }

        Ok(())
    }

    /// Translation of `SetDrawState()`.
    fn set_draw_state(&mut self, cmd: &DrawCmd) {
        let blend = cmd.blend;
        let gl = &self.gl;
        let ds = &mut self.drawstate;

        if ds.viewport_dirty {
            let viewport = ds.viewport;
            let istarget = ds.target.is_some();
            gl.matrix_mode(GL_PROJECTION);
            gl.load_identity();
            gl.viewport(
                viewport.x,
                if istarget {
                    viewport.y
                } else {
                    ds.drawableh - viewport.y - viewport.h
                },
                viewport.w,
                viewport.h,
            );
            if viewport.w != 0 && viewport.h != 0 {
                gl.orthof(
                    0.0,
                    viewport.w as GLfloat,
                    if istarget { 0.0 } else { viewport.h as GLfloat },
                    if istarget { viewport.h as GLfloat } else { 0.0 },
                    0.0,
                    1.0,
                );
            }
            gl.matrix_mode(GL_MODELVIEW);
            ds.viewport_dirty = false;
        }

        if ds.cliprect_enabled_dirty {
            if ds.cliprect_enabled {
                gl.enable(GL_SCISSOR_TEST);
            } else {
                gl.disable(GL_SCISSOR_TEST);
            }
            ds.cliprect_enabled_dirty = false;
        }

        if ds.cliprect_enabled && ds.cliprect_dirty {
            let viewport = ds.viewport;
            let rect = ds.cliprect;
            let istarget = ds.target.is_some();
            gl.scissor(
                viewport.x + rect.x,
                if istarget {
                    viewport.y + rect.y
                } else {
                    ds.drawableh - viewport.y - rect.y - rect.h
                },
                rect.w,
                rect.h,
            );
            ds.cliprect_dirty = false;
        }

        if blend != ds.blend {
            if blend == BlendMode::NONE {
                gl.disable(GL_BLEND);
            } else {
                gl.enable(GL_BLEND);
                if self.gl_oes_blend_func_separate_supported {
                    gl.blend_func_separate_oes(
                        get_blend_func(blend.src_color_factor()),
                        get_blend_func(blend.dst_color_factor()),
                        get_blend_func(blend.src_alpha_factor()),
                        get_blend_func(blend.dst_alpha_factor()),
                    );
                } else {
                    gl.blend_func(
                        get_blend_func(blend.src_color_factor()),
                        get_blend_func(blend.dst_color_factor()),
                    );
                }
                if self.gl_oes_blend_equation_separate_supported {
                    gl.blend_equation_separate_oes(
                        get_blend_equation(blend.color_operation()),
                        get_blend_equation(blend.alpha_operation()),
                    );
                } else if self.gl_oes_blend_subtract_supported {
                    gl.blend_equation_oes(get_blend_equation(blend.color_operation()));
                }
            }
            ds.blend = blend;
        }

        if cmd.texture.is_some() != ds.texturing {
            if cmd.texture.is_none() {
                gl.disable(GL_TEXTURE_2D);
                gl.disable_client_state(GL_TEXTURE_COORD_ARRAY);
                ds.texturing = false;
            } else {
                gl.enable(GL_TEXTURE_2D);
                gl.enable_client_state(GL_TEXTURE_COORD_ARRAY);
                ds.texturing = true;
            }
        }
    }

    /// Translation of `SetCopyState()`.
    fn set_copy_state(&mut self, cmd: &DrawCmd, textures: &mut TextureStore) -> Result<()> {
        let handle = cmd.texture.ok_or_else(invalid_texture)?;
        let texture = textures.get_mut(handle).ok_or_else(invalid_texture)?;
        let textype = self.textype;

        self.set_draw_state(cmd);

        let gl = &self.gl;
        let texturedata = gles_texture_mut(texture).ok_or_else(invalid_texture)?;
        if Some(texturedata.texture) != self.drawstate.texture {
            gl.bind_texture(GL_TEXTURE_2D, texturedata.texture);
            self.drawstate.texture = Some(texturedata.texture);
        }

        if cmd.texture_scale_mode != texturedata.texture_scale_mode {
            set_texture_scale_mode(gl, textype, cmd.texture_scale_mode);
            texturedata.texture_scale_mode = cmd.texture_scale_mode;
        }

        if cmd.texture_address_mode_u != texturedata.texture_address_mode_u
            || cmd.texture_address_mode_v != texturedata.texture_address_mode_v
        {
            set_texture_address_mode(
                gl,
                textype,
                cmd.texture_address_mode_u,
                cmd.texture_address_mode_v,
            );
            texturedata.texture_address_mode_u = cmd.texture_address_mode_u;
            texturedata.texture_address_mode_v = cmd.texture_address_mode_v;
        }
        Ok(())
    }

    /// The address of the command's first vertex.
    fn verts_at(&self, cmd: &DrawCmd) -> *const f32 {
        self.verts.as_ptr().wrapping_add(cmd.first)
    }

    /// The renderer for `window`, which is made an OpenGL ES 1.1 window if
    /// it isn't one. Translation of `GLES_CreateRenderer()`.
    ///
    /// Note (upstream): unlike the other renderers, upstream's doesn't
    /// check the output colorspace that `SDL_SetupRendererColorspace()`
    /// picks: a renderer asked for linear output is made, and draws as if
    /// it were sRGB (the front end converts the colors to linear). Kept.
    pub(crate) fn for_window(
        window: Window,
        _output_colorspace: Colorspace,
    ) -> Result<GlesRenderer> {
        let profile_mask = gl::gl_get_attribute(GlAttr::ContextProfileMask)?;
        let major = gl::gl_get_attribute(GlAttr::ContextMajorVersion)?;
        let minor = gl::gl_get_attribute(GlAttr::ContextMinorVersion)?;

        let _ = window.sync();
        let window_flags = window.flags()?;

        let mut changed_window = false;
        let result = (|| {
            if !window_flags.contains(WindowFlags::OPENGL)
                || profile_mask != gl::GL_CONTEXT_PROFILE_ES
                || major != RENDERER_CONTEXT_MAJOR
                || minor != RENDERER_CONTEXT_MINOR
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
            GlesRenderer::create(window)
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

    /// The creation of the renderer once the window is an OpenGL ES 1.1
    /// window (the rest of `GLES_CreateRenderer()`).
    fn create(window: Window) -> Result<GlesRenderer> {
        let context = GlContext::new(&window)?;
        context.make_current(Some(&window))?;

        let gl = GlesFuncs::load()?;

        let mut data = GlesRenderer {
            window,
            context: Some(context),
            gl,
            gl_oes_framebuffer_object_supported: false,
            framebuffers: Vec::new(),
            window_framebuffer: 0,
            gl_oes_blend_func_separate_supported: false,
            gl_oes_blend_equation_separate_supported: false,
            gl_oes_blend_subtract_supported: false,
            gl_ext_blend_minmax_supported: false,
            drawstate: DrawStateCache {
                viewport: Rect::default(),
                viewport_dirty: false,
                texture: None,
                target: None,
                drawablew: 0,
                drawableh: 0,
                blend: BlendMode::NONE,
                cliprect_enabled_dirty: false,
                cliprect_enabled: false,
                cliprect_dirty: false,
                cliprect: Rect::default(),
                texturing: false,
                color_dirty: false,
                color: FColor::default(),
                clear_color_dirty: false,
                clear_color: FColor::default(),
            },
            textype: GL_TEXTURE_2D,
            debug_enabled: false,
            target: None,
            verts: Vec::new(),
            max_texture_size: 0,
        };
        data.invalidate_cached_state();

        // Check for debug output support
        if gl::gl_get_attribute(GlAttr::ContextFlags)
            .is_ok_and(|value| value & gl::GL_CONTEXT_DEBUG_FLAG != 0)
        {
            data.debug_enabled = true;
        }

        data.max_texture_size = data.gl.get_integer(GL_MAX_TEXTURE_SIZE);

        // Android does not report GL_OES_framebuffer_object but the functionality seems to be there anyway
        // FIXME (upstream): not on the Android emulator, whose OpenGL ES 1
        // translator has the functions (libGLESv1_CM.so exports them) but
        // fails them all with GL_INVALID_OPERATION: glGenFramebuffersOES()
        // leaves its names unset, render targets never complete, and
        // glDeleteFramebuffersOES() of the garbage names crashes
        // (gfxstream's `GLClientState::removeFramebuffers()`) when the
        // renderer is destroyed. Here, without the extension, a framebuffer
        // object is made (and deleted) to see that the functions work.
        let fbo_functions_work = || {
            let gl = &data.gl;
            gl.get_error();
            let fbo = gl.gen_framebuffer_oes();
            if gl.get_error() != GL_NO_ERROR {
                return false;
            }
            gl.delete_framebuffer_oes(fbo);
            true
        };
        if gl::gl_extension_supported("GL_OES_framebuffer_object")
            || (data.gl.gen_framebuffers_oes.is_some() && fbo_functions_work())
        {
            data.gl_oes_framebuffer_object_supported = true;

            data.window_framebuffer = data.gl.get_integer(GL_FRAMEBUFFER_BINDING_OES) as GLuint;
        }

        if gl::gl_extension_supported("GL_OES_blend_func_separate") {
            data.gl_oes_blend_func_separate_supported = true;
        }
        if gl::gl_extension_supported("GL_OES_blend_equation_separate") {
            data.gl_oes_blend_equation_separate_supported = true;
        }
        if gl::gl_extension_supported("GL_OES_blend_subtract") {
            data.gl_oes_blend_subtract_supported = true;
        }
        if gl::gl_extension_supported("GL_EXT_blend_minmax") {
            data.gl_ext_blend_minmax_supported = true;
        }

        data.textype = GL_TEXTURE_2D; // this might be other things, like TEXTURE_RECTANGLE, maybe if there's any extension...?

        // Set up parameters for rendering
        let gl = &data.gl;
        gl.disable(GL_DEPTH_TEST);
        gl.disable(GL_CULL_FACE);

        gl.matrix_mode(GL_MODELVIEW);
        gl.load_identity();

        gl.enable_client_state(GL_VERTEX_ARRAY);
        gl.disable_client_state(GL_TEXTURE_COORD_ARRAY);

        gl.clear_color(1.0, 1.0, 1.0, 1.0);

        data.drawstate.blend = BlendMode::INVALID;
        Ok(data)
    }
}

impl RenderBackend for GlesRenderer {
    fn name(&self) -> &'static str {
        GLES_RENDERER
    }

    /// Translation of `GLES_GetOutputSize()`.
    fn output_size(&self, _textures: &TextureStore) -> Option<Result<(i32, i32)>> {
        None // (the window's size in pixels)
    }

    fn texture_formats(&self) -> Option<Vec<PixelFormat>> {
        Some(vec![PixelFormat::RGBA32])
    }

    /// Note (upstream): the renderer doesn't set
    /// `npot_texture_wrap_unsupported`, though its textures of sizes that
    /// aren't powers of two are drawn from the top left part of a larger GL
    /// texture: a wrapped draw of one samples the GL texture's padding
    /// (`SDL_PROP_RENDERER_TEXTURE_WRAPPING_BOOLEAN` is true). Kept.
    fn npot_texture_wrap_unsupported(&self) -> bool {
        false
    }

    fn max_texture_size(&self) -> Option<i32> {
        Some(self.max_texture_size)
    }

    /// Translation of `GLES_SupportsBlendMode()`.
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
        if (src_color_factor != src_alpha_factor || dst_color_factor != dst_alpha_factor)
            && !self.gl_oes_blend_func_separate_supported
        {
            return false;
        }
        if color_operation != alpha_operation && !self.gl_oes_blend_equation_separate_supported {
            return false;
        }
        if color_operation != Some(BlendOperation::Add) && !self.gl_oes_blend_subtract_supported {
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

    /// Translation of `GLES_CreateTexture()`.
    fn create_texture(
        &mut self,
        texture: &mut TextureData,
        _create_props: &TextureCreateProps,
    ) -> Result<()> {
        let _ = self.activate();

        let (internal_format, format, textype) = match texture.format {
            PixelFormat::RGBA32 => (GL_RGBA as GLint, GL_RGBA, GL_UNSIGNED_BYTE),
            _ => return Err(Error::new("Texture format not supported")),
        };

        let mut data = GlesTexture {
            texture: 0,
            textype: GL_TEXTURE_2D,
            texw: 0.0,
            texh: 0.0,
            format,
            formattype: textype,
            texture_scale_mode: texture.scale_mode,
            texture_address_mode_u: TextureAddressMode::Clamp,
            texture_address_mode_v: TextureAddressMode::Clamp,
            pixels: Vec::new(),
            pitch: 0,
            fbo: None,
        };

        if texture.access == TextureAccess::Streaming {
            data.pitch = texture.w * texture.format.bytes_per_pixel() as i32;
            data.pixels = vec![0; texture.h as usize * data.pitch as usize];
        }

        if texture.access == TextureAccess::Target {
            if !self.gl_oes_framebuffer_object_supported {
                return Err(Error::new("GL_OES_framebuffer_object not supported"));
            }
            data.fbo = Some(self.get_fbo(texture.w as u32, texture.h as u32));
        } else {
            data.fbo = None;
        }

        let gl = &self.gl;
        gl.get_error();
        gl.enable(GL_TEXTURE_2D);
        data.texture = gl.gen_texture();
        let result = gl.get_error();
        if result != GL_NO_ERROR {
            return Err(gles_set_error("glGenTextures()", result));
        }

        data.textype = GL_TEXTURE_2D;
        // no NPOV textures allowed in OpenGL ES (yet)
        let texture_w = crate::render::opengl::powerof2(texture.w);
        let texture_h = crate::render::opengl::powerof2(texture.h);
        data.texw = texture.w as GLfloat / texture_w as GLfloat;
        data.texh = texture.h as GLfloat / texture_h as GLfloat;

        gl.bind_texture(data.textype, data.texture);
        gl.tex_image_2d_empty(
            data.textype,
            internal_format,
            texture_w,
            texture_h,
            format,
            textype,
        );

        set_texture_scale_mode(gl, data.textype, texture.scale_mode);
        // FIXME (upstream): the texture's address modes are recorded as
        // clamped (above), but GL's are left at their default, GL_REPEAT,
        // so a linearly scaled draw of the texture, which asks for clamped
        // ones, blends its edges with the opposite ones (and is unlike the
        // other renderers'). Here they're set, as the OpenGL ES 2
        // renderer's `GLES2_CreateTexture()` does.
        set_texture_address_mode(
            gl,
            data.textype,
            data.texture_address_mode_u,
            data.texture_address_mode_v,
        );

        gl.disable(GL_TEXTURE_2D);
        self.drawstate.texture = Some(data.texture);
        self.drawstate.texturing = false;

        let result = gl.get_error();
        if result != GL_NO_ERROR {
            // Note (upstream): the texture made is not deleted (here
            // neither, as upstream's)
            return Err(gles_set_error("glTexImage2D()", result));
        }

        texture.internal = Some(Box::new(data));
        Ok(())
    }

    fn queue_set_viewport(&mut self, _cmd: &mut RenderCommand) -> Result<()> {
        Ok(()) // nothing to do in this backend.
    }

    fn queue_set_draw_color(&mut self, _cmd: &mut RenderCommand) -> Result<()> {
        Ok(()) // SetViewport and SetDrawColor are (currently) no-ops.
    }

    /// Translation of `GLES_QueueDrawPoints()`.
    fn queue_draw_points(&mut self, cmd: &mut DrawCmd, points: &[FPoint]) -> Result<()> {
        cmd.first = self.verts.len();
        cmd.count = points.len();
        for p in points {
            self.verts.extend_from_slice(&[0.5 + p.x, 0.5 + p.y]);
        }

        Ok(())
    }

    /// Translation of `GLES_QueueDrawLines()`.
    fn queue_draw_lines(&mut self, cmd: &mut DrawCmd, points: &[FPoint]) -> Option<Result<()>> {
        let Some(first) = points.first() else {
            return Some(Ok(()));
        };
        cmd.first = self.verts.len();
        cmd.count = points.len();

        // 0.5f offset to hit the center of the pixel.
        let mut prevx = 0.5 + first.x;
        let mut prevy = 0.5 + first.y;
        self.verts.extend_from_slice(&[prevx, prevy]);

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
            self.verts.extend_from_slice(&[prevx, prevy]);
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

    /// Translation of `GLES_QueueGeometry()`.
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
        let texturedata = match texture {
            Some(t) => Some(gles_texture(t).ok_or_else(invalid_texture)?),
            None => None,
        };

        cmd.first = self.verts.len();
        cmd.count = count;

        for i in 0..count {
            let j = geometry.vertex(i);
            let (x, y) = geometry.xy(j);
            let col = geometry.color(j);

            self.verts.extend_from_slice(&[
                x * scale_x,
                y * scale_y,
                col.r * color_scale,
                col.g * color_scale,
                col.b * color_scale,
                col.a,
            ]);

            if let Some(texturedata) = texturedata {
                let (u, v) = geometry.uv(j);
                self.verts
                    .extend_from_slice(&[u * texturedata.texw, v * texturedata.texh]);
            }
        }
        Ok(())
    }

    /// Translation of `GLES_InvalidateCachedState()`.
    fn invalidate_cached_state(&mut self) {
        let cache = &mut self.drawstate;
        cache.viewport_dirty = true;
        cache.texture = None;
        cache.drawablew = 0;
        cache.drawableh = 0;
        cache.blend = BlendMode::INVALID;
        cache.cliprect_enabled_dirty = true;
        cache.cliprect_dirty = true;
        cache.color_dirty = true;
        cache.clear_color_dirty = true;
    }

    /// Translation of `GLES_RunCommandQueue()`.
    fn run_command_queue(
        &mut self,
        cmds: &[RenderCommand],
        textures: &mut TextureStore,
        _gpu_render_states: &crate::render::sysrender::GpuRenderStates,
    ) -> Result<()> {
        self.activate()?;

        self.drawstate.target = self.target;
        if self.target.is_none() {
            let (w, h) = self.window.size_in_pixels().unwrap_or((0, 0));
            if w != self.drawstate.drawablew || h != self.drawstate.drawableh {
                self.drawstate.viewport_dirty = true; // if the window dimensions changed, invalidate the current viewport, etc.
                self.drawstate.cliprect_dirty = true;
                self.drawstate.drawablew = w;
                self.drawstate.drawableh = h;
            }
        }

        for cmd in cmds {
            match *cmd {
                RenderCommand::SetDrawColor {
                    color, color_scale, ..
                } => {
                    let r = color.r * color_scale;
                    let g = color.g * color_scale;
                    let b = color.b * color_scale;
                    let a = color.a;
                    let ds = &mut self.drawstate;
                    if ds.color_dirty
                        || r != ds.color.r
                        || g != ds.color.g
                        || b != ds.color.b
                        || a != ds.color.a
                    {
                        self.gl.color4f(r, g, b, a);
                        ds.color = FColor::new(r, g, b, a);
                        ds.color_dirty = false;
                    }
                }

                RenderCommand::SetViewport { rect, .. } => {
                    if self.drawstate.viewport != rect {
                        self.drawstate.viewport = rect;
                        self.drawstate.viewport_dirty = true;
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
                    let r = color.r * color_scale;
                    let g = color.g * color_scale;
                    let b = color.b * color_scale;
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

                RenderCommand::Draw(DrawKind::Points, d) => {
                    let count = d.count;
                    let verts = self.verts_at(&d);
                    self.set_draw_state(&d);
                    debug_assert!(d.first + count * 2 <= self.verts.len());
                    // SAFETY: the command's `count` points (two floats
                    // each) are in `self.verts` from `first` on, which
                    // isn't touched until the draw is done.
                    unsafe {
                        self.gl.vertex_pointer(2, 0, verts);
                        self.gl.draw_arrays(GL_POINTS, 0, count as GLsizei);
                    }
                }

                RenderCommand::Draw(DrawKind::Lines, d) => {
                    let verts = self.verts_at(&d);
                    let count = d.count;
                    crate::sdl_assert!(count >= 2);
                    self.set_draw_state(&d);
                    debug_assert!(d.first + count * 2 <= self.verts.len());
                    // SAFETY: as for the points.
                    unsafe {
                        self.gl.vertex_pointer(2, 0, verts);
                        self.gl.draw_arrays(GL_LINE_STRIP, 0, count as GLsizei);
                    }
                }

                RenderCommand::Draw(DrawKind::FillRects, _) => {} // unused

                RenderCommand::Draw(DrawKind::Copy, _) => {} // unused

                RenderCommand::Draw(DrawKind::CopyEx, _) => {} // unused

                RenderCommand::Draw(DrawKind::Geometry, d) => {
                    let verts = self.verts_at(&d);
                    let count = d.count;
                    let floats = 2 + 4 + if d.texture.is_some() { 2 } else { 0 };
                    let stride = (floats * size_of::<f32>()) as GLsizei;

                    if d.texture.is_some() {
                        // (a texture that's gone: nothing drawn, as for
                        // the other renderers' failed state changes)
                        if self.set_copy_state(&d, textures).is_err() {
                            continue;
                        }
                    } else {
                        self.set_draw_state(&d);
                    }

                    let gl = &self.gl;
                    gl.enable_client_state(GL_COLOR_ARRAY);

                    debug_assert!(d.first + count * floats <= self.verts.len());
                    // SAFETY: the command's `count` vertices (positions,
                    // colors and texture coordinates, `floats` floats
                    // each) are in `self.verts` from `first` on, which
                    // isn't touched until the draw is done.
                    unsafe {
                        gl.vertex_pointer(2, stride, verts);
                        gl.color_pointer(4, stride, verts.wrapping_add(2));
                        if d.texture.is_some() {
                            gl.tex_coord_pointer(2, stride, verts.wrapping_add(2 + 4));
                        }

                        gl.draw_arrays(GL_TRIANGLES, 0, count as GLsizei);
                    }

                    gl.disable_client_state(GL_COLOR_ARRAY);
                }

                RenderCommand::NoOp => {}
            }
        }

        Ok(())
    }

    fn reset_vertices(&mut self) {
        self.verts.clear();
    }

    /// (upstream's renderer has no `CreatePalette`: its only texture
    /// format is RGBA32, so a palette texture always has a native one,
    /// whose palette the front end applies, and this isn't called)
    fn create_palette(&mut self) -> Result<Box<dyn Any>> {
        Err(Error::unsupported())
    }

    fn update_palette(&mut self, _palette: &mut dyn Any, _colors: &[Color]) -> Result<()> {
        Err(Error::unsupported())
    }

    fn change_texture_palette(
        &mut self,
        _texture: &mut TextureData,
        _palette: Option<&dyn Any>,
    ) -> Option<Result<()>> {
        None
    }

    /// Translation of `GLES_UpdateTexture()`.
    fn update_texture(
        &mut self,
        texture: &mut TextureData,
        rect: &Rect,
        pixels: &[u8],
        pitch: usize,
    ) -> Result<()> {
        let _ = self.activate();

        // Bail out if we're supposed to update an empty rectangle
        if rect.w <= 0 || rect.h <= 0 {
            return Ok(());
        }

        let data = gles_texture(texture).ok_or_else(invalid_texture)?;

        // Reformat the texture data into a tightly packed array
        let src_pitch = rect.w as usize * texture.format.bytes_per_pixel() as usize;
        let rows = rect.h as usize;
        // (the C code reads the rows without knowing the buffer's size)
        if pixels.len() < (rows - 1) * pitch + src_pitch {
            return Err(Error::invalid_param("pixels"));
        }
        let blob;
        let src = if pitch != src_pitch {
            blob = pixels
                .chunks(pitch)
                .take(rows)
                .flat_map(|row| &row[..src_pitch])
                .copied()
                .collect::<Vec<u8>>();
            &blob[..]
        } else {
            pixels
        };

        // Create a texture subimage with the supplied data
        let gl = &self.gl;
        gl.get_error();
        gl.enable(data.textype);
        gl.bind_texture(data.textype, data.texture);
        gl.pixel_storei(GL_UNPACK_ALIGNMENT, 1);
        gl.tex_sub_image_2d(
            data.textype,
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            data.format,
            data.formattype,
            src,
        );
        gl.disable(data.textype);

        self.drawstate.texture = Some(data.texture);
        self.drawstate.texturing = false;

        if gl.get_error() != GL_NO_ERROR {
            return Err(Error::new("Failed to update texture"));
        }
        Ok(())
    }

    /// Translation of `GLES_LockTexture()`.
    fn lock_texture(&mut self, texture: &mut TextureData, rect: &Rect) -> Result<(usize, i32)> {
        let bpp = texture.format.bytes_per_pixel() as usize;
        let data = gles_texture(texture).ok_or_else(invalid_texture)?;

        let offset = rect.y as usize * data.pitch as usize + rect.x as usize * bpp;
        Ok((offset, data.pitch))
    }

    fn texture_pixels_mut<'t>(&mut self, texture: &'t mut TextureData) -> Option<&'t mut [u8]> {
        gles_texture_mut(texture).map(|t| &mut t.pixels[..])
    }

    /// Translation of `GLES_UnlockTexture()`.
    fn unlock_texture(&mut self, texture: &mut TextureData) {
        // We do whole texture updates, at least for now
        let rect = Rect::new(0, 0, texture.w, texture.h);
        let Some(data) = gles_texture_mut(texture) else {
            return;
        };
        let pixels = std::mem::take(&mut data.pixels);
        let pitch = data.pitch as usize;
        let _ = self.update_texture(texture, &rect, &pixels, pitch);
        if let Some(data) = gles_texture_mut(texture) {
            data.pixels = pixels;
        }
    }

    /// Translation of `GLES_SetRenderTarget()`.
    fn set_render_target(
        &mut self,
        target: Option<Texture>,
        textures: &TextureStore,
    ) -> Result<()> {
        // (renderer->target, which the front end sets before this)
        let texture = match target {
            Some(t) => Some(textures.get(t).ok_or_else(invalid_texture)?),
            None => None,
        };
        let texturedata = match texture {
            Some(t) => Some(gles_texture(t).ok_or_else(invalid_texture)?),
            None => None,
        };
        self.target = texture.zip(texturedata).map(|(t, d)| Target {
            texture: d.texture,
            format: t.format,
        });

        if !self.gl_oes_framebuffer_object_supported {
            return Err(Error::new(
                "Can't enable render target support in this renderer",
            ));
        }

        self.drawstate.viewport_dirty = true;

        let Some(texturedata) = texturedata else {
            self.gl
                .bind_framebuffer_oes(GL_FRAMEBUFFER_OES, self.window_framebuffer);
            return Ok(());
        };

        // (only the target textures have a framebuffer object, and only
        // they are made render targets)
        let fbo = texturedata.fbo.ok_or_else(invalid_texture)?;
        let gl = &self.gl;
        gl.bind_framebuffer_oes(GL_FRAMEBUFFER_OES, fbo);
        // TODO: check if texture pixel format allows this operation
        gl.framebuffer_texture_2d_oes(
            GL_FRAMEBUFFER_OES,
            GL_COLOR_ATTACHMENT0_OES,
            texturedata.textype,
            texturedata.texture,
        );
        // Check FBO status
        let status = gl.check_framebuffer_status_oes(GL_FRAMEBUFFER_OES);
        if status != GL_FRAMEBUFFER_COMPLETE_OES {
            return Err(Error::new("glFramebufferTexture2DOES() failed"));
        }
        Ok(())
    }

    /// Translation of `GLES_RenderReadPixels()`.
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
            self.gl.pixel_storei(GL_PACK_ALIGNMENT, 1);
            self.gl
                .read_pixels(rect.x, y, rect.w, rect.h, GL_RGBA, GL_UNSIGNED_BYTE, pixels);

            // Flip the rows to be top-down if necessary
            if self.target.is_none() {
                surface.flip(FlipMode::Vertical)?;
            }
            Ok(surface)
        })())
    }

    /// Translation of `GLES_RenderPresent()`.
    fn present(&mut self) -> bool {
        let _ = self.activate();

        gl::gl_swap_window(&self.window).is_ok()
    }

    /// Translation of `GLES_DestroyTexture()`.
    fn destroy_texture(&mut self, texture: &mut TextureData) {
        // (the texture is deleted only when the context can be made
        // current: once the window is gone, the delete would go to
        // whichever context is current, and destroying the renderer's
        // context frees it anyway)
        let active = self.activate().is_ok();

        let Some(internal) = texture.internal.take() else {
            return;
        };
        let Ok(data) = internal.downcast::<GlesTexture>() else {
            return;
        };
        if self.drawstate.texture == Some(data.texture) {
            self.drawstate.texture = None;
        }
        if self
            .drawstate
            .target
            .is_some_and(|t| t.texture == data.texture)
        {
            self.drawstate.target = None;
        }

        if data.texture != 0 && active {
            self.gl.delete_texture(data.texture);
        }
    }

    /// Translation of `GLES_SetVSync()`.
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

    /// Translation of `GLES_WindowEvent()`.
    fn window_event(&mut self, event_type: EventType) {
        if event_type == EventType::WINDOW_MINIMIZED {
            // According to Apple documentation, we need to finish drawing NOW!
            self.gl.finish();
        }
    }

    /// Translation of `GLES_DestroyRenderer()`.
    fn destroy(&mut self) {
        if self.context.is_some() {
            // Note: upstream deletes the framebuffer objects in whichever
            // context is current; here only with the renderer's made
            // current (as for the textures)
            if self.activate().is_ok() {
                for fbo in &self.framebuffers {
                    self.gl.delete_framebuffer_oes(fbo.fbo);
                }
            }
            self.framebuffers.clear();
            // SDL_GL_DestroyContext()
            self.context = None;
        }
    }
}
