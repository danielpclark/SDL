// Rust translation of src/render/opengl/SDL_render_gl.c and SDL_glfuncs.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The OpenGL renderer: fixed-function OpenGL 2.1 with client-side vertex
//! arrays, plus GLSL shaders (when the driver has them) for RGB, paletted,
//! YUV and NV12/NV21 textures and pixel art scaling.
//!
//! The GL entry points are loaded at run time through
//! [`gl_get_proc_address`](crate::video::gl::gl_get_proc_address) from the
//! OpenGL library of the window's video driver.

mod shaders;
#[cfg(test)]
mod tests;

use std::any::Any;
use std::ffi::{c_char, c_void, CStr};
use std::sync::Mutex;

use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::events::EventType;
use crate::hints;
use crate::log::{self, Category};
use crate::properties::Properties;
use crate::render::sysrender::{
    CopyEx, DrawCmd, DrawKind, Geometry, RenderBackend, RenderCommand, TextureCreateProps,
    TextureData, TextureStore,
};
use crate::render::{Texture, TextureAccess, TextureAddressMode};
use crate::video::blendmode::{BlendFactor, BlendOperation};
use crate::video::gl::{self, GlAttr, GlContext};
use crate::video::pixels::{Color, Colorspace, FColor, PixelFormat};
use crate::video::rect::{FPoint, FRect, Rect};
use crate::video::surface::{ScaleMode, Surface};
use crate::video::{BlendMode, FlipMode, Window};

use shaders::{Shader, ShaderContext, ShaderParams};

/// The name of the OpenGL renderer (`GL_RenderDriver.name`).
pub(crate) const OPENGL_RENDERER: &str = "opengl";

/// The GLuint texture associated with a texture
/// (`SDL_PROP_TEXTURE_OPENGL_TEXTURE_NUMBER`).
pub const PROP_TEXTURE_OPENGL_TEXTURE_NUMBER: &str = "SDL.texture.opengl.texture";
/// The GLuint texture of the UV plane of an NV12 texture
/// (`SDL_PROP_TEXTURE_OPENGL_TEXTURE_UV_NUMBER`).
pub const PROP_TEXTURE_OPENGL_TEXTURE_UV_NUMBER: &str = "SDL.texture.opengl.texture_uv";
/// The GLuint texture of the U plane of a YUV texture
/// (`SDL_PROP_TEXTURE_OPENGL_TEXTURE_U_NUMBER`).
pub const PROP_TEXTURE_OPENGL_TEXTURE_U_NUMBER: &str = "SDL.texture.opengl.texture_u";
/// The GLuint texture of the V plane of a YUV texture
/// (`SDL_PROP_TEXTURE_OPENGL_TEXTURE_V_NUMBER`).
pub const PROP_TEXTURE_OPENGL_TEXTURE_V_NUMBER: &str = "SDL.texture.opengl.texture_v";
/// The GLenum texture target (`GL_TEXTURE_2D`, `GL_TEXTURE_RECTANGLE_ARB`,
/// ...) of the texture (`SDL_PROP_TEXTURE_OPENGL_TEXTURE_TARGET_NUMBER`).
pub const PROP_TEXTURE_OPENGL_TEXTURE_TARGET_NUMBER: &str = "SDL.texture.opengl.target";
/// The texture coordinate width of the texture (1.0 unless rectangle or
/// power-of-two textures are used) (`SDL_PROP_TEXTURE_OPENGL_TEX_W_FLOAT`).
pub const PROP_TEXTURE_OPENGL_TEX_W_FLOAT: &str = "SDL.texture.opengl.tex_w";
/// The texture coordinate height of the texture
/// (`SDL_PROP_TEXTURE_OPENGL_TEX_H_FLOAT`).
pub const PROP_TEXTURE_OPENGL_TEX_H_FLOAT: &str = "SDL.texture.opengl.tex_h";

/* To prevent unnecessary window recreation,
 * these should match the defaults selected in SDL_GL_ResetAttributes
 */
const RENDERER_CONTEXT_MAJOR: i32 = 2;
const RENDERER_CONTEXT_MINOR: i32 = 1;

// ---------------------------------------------------------------------------
// GL types and constants (SDL_opengl.h)
// ---------------------------------------------------------------------------

pub(crate) type GLenum = u32;
pub(crate) type GLint = i32;
type GLuint = u32;
type GLsizei = i32;
type GLfloat = f32;
type GLdouble = f64;
type GLbitfield = u32;
type GLubyte = u8;

const GL_NO_ERROR: GLenum = 0;
const GL_ZERO: GLenum = 0;
const GL_ONE: GLenum = 1;
const GL_POINTS: GLenum = 0x0000;
const GL_LINES: GLenum = 0x0001;
const GL_LINE_STRIP: GLenum = 0x0003;
const GL_TRIANGLES: GLenum = 0x0004;
const GL_SRC_COLOR: GLenum = 0x0300;
const GL_ONE_MINUS_SRC_COLOR: GLenum = 0x0301;
const GL_SRC_ALPHA: GLenum = 0x0302;
const GL_ONE_MINUS_SRC_ALPHA: GLenum = 0x0303;
const GL_DST_ALPHA: GLenum = 0x0304;
const GL_ONE_MINUS_DST_ALPHA: GLenum = 0x0305;
const GL_DST_COLOR: GLenum = 0x0306;
const GL_ONE_MINUS_DST_COLOR: GLenum = 0x0307;
const GL_INVALID_ENUM: GLenum = 0x0500;
const GL_INVALID_VALUE: GLenum = 0x0501;
const GL_INVALID_OPERATION: GLenum = 0x0502;
const GL_STACK_OVERFLOW: GLenum = 0x0503;
const GL_STACK_UNDERFLOW: GLenum = 0x0504;
const GL_OUT_OF_MEMORY: GLenum = 0x0505;
const GL_CULL_FACE: GLenum = 0x0B44;
const GL_DEPTH_TEST: GLenum = 0x0B71;
const GL_BLEND: GLenum = 0x0BE2;
const GL_SCISSOR_TEST: GLenum = 0x0C11;
const GL_UNPACK_ROW_LENGTH: GLenum = 0x0CF2;
const GL_UNPACK_ALIGNMENT: GLenum = 0x0CF5;
const GL_PACK_ROW_LENGTH: GLenum = 0x0D02;
const GL_PACK_ALIGNMENT: GLenum = 0x0D05;
const GL_MAX_TEXTURE_SIZE: GLenum = 0x0D33;
const GL_TEXTURE_2D: GLenum = 0x0DE1;
const GL_UNSIGNED_BYTE: GLenum = 0x1401;
const GL_FLOAT: GLenum = 0x1406;
const GL_MODELVIEW: GLenum = 0x1700;
const GL_PROJECTION: GLenum = 0x1701;
const GL_RGBA: GLenum = 0x1908;
const GL_LUMINANCE: GLenum = 0x1909;
const GL_LUMINANCE_ALPHA: GLenum = 0x190A;
const GL_VERSION: GLenum = 0x1F02;
const GL_NEAREST: GLint = 0x2600;
const GL_LINEAR: GLint = 0x2601;
const GL_TEXTURE_MAG_FILTER: GLenum = 0x2800;
const GL_TEXTURE_MIN_FILTER: GLenum = 0x2801;
const GL_TEXTURE_WRAP_S: GLenum = 0x2802;
const GL_TEXTURE_WRAP_T: GLenum = 0x2803;
const GL_REPEAT: GLint = 0x2901;
const GL_COLOR_BUFFER_BIT: GLbitfield = 0x4000;
const GL_FUNC_ADD: GLenum = 0x8006;
const GL_MIN: GLenum = 0x8007;
const GL_MAX: GLenum = 0x8008;
const GL_FUNC_SUBTRACT: GLenum = 0x800A;
const GL_FUNC_REVERSE_SUBTRACT: GLenum = 0x800B;
const GL_TABLE_TOO_LARGE: GLenum = 0x8031;
#[cfg(target_os = "macos")]
const GL_RGB8: GLint = 0x8051;
const GL_RGBA8: GLint = 0x8058;
const GL_VERTEX_ARRAY: GLenum = 0x8074;
const GL_COLOR_ARRAY: GLenum = 0x8076;
const GL_TEXTURE_COORD_ARRAY: GLenum = 0x8078;
const GL_BGRA: GLenum = 0x80E1;
const GL_CLAMP_TO_EDGE: GLint = 0x812F;
const GL_DEBUG_OUTPUT_SYNCHRONOUS_ARB: GLenum = 0x8242;
const GL_DEBUG_CALLBACK_FUNCTION_ARB: GLenum = 0x8244;
const GL_DEBUG_CALLBACK_USER_PARAM_ARB: GLenum = 0x8245;
const GL_DEBUG_TYPE_ERROR_ARB: GLenum = 0x824C;
const GL_TEXTURE0_ARB: GLenum = 0x84C0;
const GL_TEXTURE1_ARB: GLenum = 0x84C1;
const GL_TEXTURE2_ARB: GLenum = 0x84C2;
const GL_MAX_TEXTURE_UNITS_ARB: GLenum = 0x84E2;
const GL_TEXTURE_RECTANGLE_ARB: GLenum = 0x84F5;
const GL_MAX_RECTANGLE_TEXTURE_SIZE_ARB: GLenum = 0x84F8;
#[cfg(target_os = "macos")]
const GL_UNPACK_CLIENT_STORAGE_APPLE: GLenum = 0x85B2;
#[cfg(target_os = "macos")]
const GL_YCBCR_422_APPLE: GLenum = 0x85B9;
#[cfg(target_os = "macos")]
const GL_UNSIGNED_SHORT_8_8_APPLE: GLenum = 0x85BA;
#[cfg(target_os = "macos")]
const GL_TEXTURE_STORAGE_HINT_APPLE: GLenum = 0x85BC;
#[cfg(target_os = "macos")]
const GL_STORAGE_CACHED_APPLE: GLint = 0x85BE;
#[cfg(target_os = "macos")]
const GL_STORAGE_SHARED_APPLE: GLint = 0x85BF;
const GL_FRAMEBUFFER_COMPLETE_EXT: GLenum = 0x8CD5;
const GL_COLOR_ATTACHMENT0_EXT: GLenum = 0x8CE0;
const GL_FRAMEBUFFER_EXT: GLenum = 0x8D40;
// This is always the same number between the various EXT/ARB/GLES extensions.
const GL_FRAMEBUFFER_SRGB: GLenum = 0x8DB9;

/// `GLDEBUGPROCARB`.
type DebugProc = unsafe extern "system" fn(
    GLenum,
    GLenum,
    GLuint,
    GLenum,
    GLsizei,
    *const c_char,
    *const c_void,
);
/// `PFNGLDEBUGMESSAGECALLBACKARBPROC`.
type PfnDebugMessageCallback = unsafe extern "system" fn(Option<DebugProc>, *const c_void);
/// `PFNGLACTIVETEXTUREARBPROC`.
type PfnActiveTexture = unsafe extern "system" fn(GLenum);

/// A GL function by name, recording the error of a missing one as
/// `SDL_PROC` does (the last error is reported).
///
/// # Safety
///
/// `F` must be the function pointer type of the GL entry point `name`.
unsafe fn load_function<F: Copy>(name: &str, error: &mut Option<Error>) -> Option<F> {
    const { assert!(size_of::<F>() == size_of::<*mut c_void>()) };
    match gl::get_proc_address(name) {
        // SAFETY: the caller's contract; F is a function pointer type.
        Ok(Some(p)) => Some(unsafe { std::mem::transmute_copy::<*mut c_void, F>(&p.as_ptr()) }),
        Ok(None) => {
            // (upstream appends SDL_GetError(), which the lookup doesn't set
            // when the driver merely lacks the function)
            *error = Some(Error::new(format!(
                "Couldn't load GL function {name}: no such function"
            )));
            None
        }
        Err(e) => {
            *error = Some(Error::new(format!(
                "Couldn't load GL function {name}: {}",
                e.message()
            )));
            None
        }
    }
}

/// Declares [`GlFuncs`] and its loader from the `SDL_PROC` entries of
/// `SDL_glfuncs.h`.
macro_rules! gl_functions {
    ($($name:ident: fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        /// The OpenGL functions of the renderer (the `SDL_PROC` entries of
        /// `SDL_glfuncs.h`; all are required, used or not).
        #[allow(non_snake_case, dead_code)]
        struct GlFuncs {
            $($name: unsafe extern "system" fn($($arg),*) $(-> $ret)?,)*
        }

        impl GlFuncs {
            /// Translation of `GL_LoadFunctions()`.
            fn load() -> Result<GlFuncs> {
                let mut error = None;
                $(
                    #[allow(non_snake_case)]
                    // SAFETY: the type is the entry point's.
                    let $name = unsafe {
                        load_function::<unsafe extern "system" fn($($arg),*) $(-> $ret)?>(
                            stringify!($name),
                            &mut error,
                        )
                    };
                )*
                if let Some(e) = error {
                    return Err(e);
                }
                Ok(GlFuncs {
                    $($name: $name.expect("checked above"),)*
                })
            }
        }
    };
}

gl_functions! {
    glBegin: fn(GLenum);
    glBindTexture: fn(GLenum, GLuint);
    glBlendEquation: fn(GLenum);
    glBlendFuncSeparate: fn(GLenum, GLenum, GLenum, GLenum);
    glClear: fn(GLbitfield);
    glClearColor: fn(GLfloat, GLfloat, GLfloat, GLfloat);
    glColor3fv: fn(*const GLfloat);
    glColor4f: fn(GLfloat, GLfloat, GLfloat, GLfloat);
    glColor4ub: fn(GLubyte, GLubyte, GLubyte, GLubyte);
    glColorPointer: fn(GLint, GLenum, GLsizei, *const c_void);
    glDeleteTextures: fn(GLsizei, *const GLuint);
    glDepthFunc: fn(GLenum);
    glDisable: fn(GLenum);
    glDisableClientState: fn(GLenum);
    glDrawArrays: fn(GLenum, GLint, GLsizei);
    glDrawPixels: fn(GLsizei, GLsizei, GLenum, GLenum, *const c_void);
    glEnable: fn(GLenum);
    glEnableClientState: fn(GLenum);
    glEnd: fn();
    glGenTextures: fn(GLsizei, *mut GLuint);
    glGetError: fn() -> GLenum;
    glGetFloatv: fn(GLenum, *mut GLfloat);
    glGetIntegerv: fn(GLenum, *mut GLint);
    glGetPointerv: fn(GLenum, *mut *mut c_void);
    glGetString: fn(GLenum) -> *const GLubyte;
    glLineWidth: fn(GLfloat);
    glLoadIdentity: fn();
    glMatrixMode: fn(GLenum);
    glOrtho: fn(GLdouble, GLdouble, GLdouble, GLdouble, GLdouble, GLdouble);
    glPixelStorei: fn(GLenum, GLint);
    glPointSize: fn(GLfloat);
    glRasterPos2i: fn(GLint, GLint);
    glReadBuffer: fn(GLenum);
    glReadPixels: fn(GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, *mut c_void);
    glRectf: fn(GLfloat, GLfloat, GLfloat, GLfloat);
    glRotatef: fn(GLfloat, GLfloat, GLfloat, GLfloat);
    glScissor: fn(GLint, GLint, GLsizei, GLsizei);
    glShadeModel: fn(GLenum);
    glTexCoord2f: fn(GLfloat, GLfloat);
    glTexCoordPointer: fn(GLint, GLenum, GLsizei, *const c_void);
    glTexEnvf: fn(GLenum, GLenum, GLfloat);
    glTexImage2D: fn(GLenum, GLint, GLint, GLsizei, GLsizei, GLint, GLenum, GLenum, *const c_void);
    glTexParameteri: fn(GLenum, GLenum, GLint);
    glTexSubImage2D: fn(GLenum, GLint, GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, *const c_void);
    glVertex2f: fn(GLfloat, GLfloat);
    glVertex3fv: fn(*const GLfloat);
    glVertexPointer: fn(GLint, GLenum, GLsizei, *const c_void);
    glViewport: fn(GLint, GLint, GLsizei, GLsizei);
}

/// The `GL_EXT_framebuffer_object` functions.
#[allow(non_snake_case)]
#[derive(Clone, Copy)]
struct FboFuncs {
    glGenFramebuffersEXT: unsafe extern "system" fn(GLsizei, *mut GLuint),
    glDeleteFramebuffersEXT: unsafe extern "system" fn(GLsizei, *const GLuint),
    glFramebufferTexture2DEXT: unsafe extern "system" fn(GLenum, GLenum, GLenum, GLuint, GLint),
    glBindFramebufferEXT: unsafe extern "system" fn(GLenum, GLuint),
    glCheckFramebufferStatusEXT: unsafe extern "system" fn(GLenum) -> GLenum,
}

impl FboFuncs {
    fn load() -> Option<FboFuncs> {
        // SAFETY: each type is the type of the entry point of that name.
        unsafe {
            Some(FboFuncs {
                glGenFramebuffersEXT: gl::gl_function("glGenFramebuffersEXT")?,
                glDeleteFramebuffersEXT: gl::gl_function("glDeleteFramebuffersEXT")?,
                glFramebufferTexture2DEXT: gl::gl_function("glFramebufferTexture2DEXT")?,
                glBindFramebufferEXT: gl::gl_function("glBindFramebufferEXT")?,
                glCheckFramebufferStatusEXT: gl::gl_function("glCheckFramebufferStatusEXT")?,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Renderer data
// ---------------------------------------------------------------------------

/// A framebuffer object for render targets of one size. Translation of
/// `GL_FBOList` (an entry; the list is a `Vec`).
#[derive(Clone, Copy, Debug)]
struct Fbo {
    w: i32,
    h: i32,
    fbo: GLuint,
}

/// Translation of `GL_DrawStateCache`. Textures are identified by the id
/// of their [`GlTextureData`] (upstream compares `SDL_Texture` pointers).
struct DrawStateCache {
    viewport_dirty: bool,
    viewport: Rect,
    texture: Option<u64>,
    target: Option<u64>,
    drawablew: i32,
    drawableh: i32,
    blend: BlendMode,
    shader: Option<Shader>,
    shader_params: Option<ShaderParams>,
    cliprect_enabled_dirty: bool,
    cliprect_enabled: bool,
    cliprect_dirty: bool,
    cliprect: Rect,
    texturing: bool,
    texturing_dirty: bool,
    vertex_array: bool,
    color_array: bool,
    texture_array: bool,
    color_dirty: bool,
    color: FColor,
    clear_color_dirty: bool,
    clear_color: FColor,
}

impl Default for DrawStateCache {
    fn default() -> DrawStateCache {
        DrawStateCache {
            viewport_dirty: false,
            viewport: Rect::default(),
            texture: None,
            target: None,
            drawablew: 0,
            drawableh: 0,
            blend: BlendMode::NONE,
            shader: None,
            shader_params: None,
            cliprect_enabled_dirty: false,
            cliprect_enabled: false,
            cliprect_dirty: false,
            cliprect: Rect::default(),
            texturing: false,
            texturing_dirty: false,
            vertex_array: false,
            color_array: false,
            texture_array: false,
            color_dirty: false,
            color: FColor::default(),
            clear_color_dirty: false,
            clear_color: FColor::default(),
        }
    }
}

/// The `GL_ARB_debug_output` state: the errors the callback recorded and
/// the callback it replaced. Boxed, as the callback's user parameter
/// points at it.
struct DebugOutput {
    errors: Mutex<Vec<String>>,
    next_error_callback: Option<DebugProc>,
    next_error_userparam: *mut c_void,
    set_callback: PfnDebugMessageCallback,
}

/// What `GL_CreateRenderer()` sets on the front end (reported through the
/// [`RenderBackend`] hooks).
#[derive(Clone, Debug, Default)]
struct RendererSetup {
    /// `SDL_AddSupportedTextureFormat()`
    texture_formats: Vec<PixelFormat>,
    /// `renderer->npot_texture_wrap_unsupported`
    npot_texture_wrap_unsupported: bool,
    /// `SDL_PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER`
    max_texture_size: i32,
}

/// Translation of `GL_PaletteData`.
struct GlPaletteData {
    texture: GLuint,
}

/// Translation of `GL_TextureData`.
struct GlTextureData {
    /// Identifies the texture in the draw state cache.
    id: u64,
    texture: GLuint,
    texture_external: bool,
    texw: GLfloat,
    texh: GLfloat,
    format: GLenum,
    formattype: GLenum,
    shader: Shader,
    texel_size: [f32; 4],
    shader_params: Option<ShaderParams>,
    pixels: Vec<u8>,
    pitch: i32,
    locked_rect: Rect,
    // YUV texture support
    yuv: bool,
    nv12: bool,
    utexture: GLuint,
    utexture_external: bool,
    vtexture: GLuint,
    vtexture_external: bool,
    texture_scale_mode: ScaleMode,
    texture_address_mode_u: TextureAddressMode,
    texture_address_mode_v: TextureAddressMode,
    fbo: Option<GLuint>,
    /// The GL texture of the palette (`texture->palette->internal`), set by
    /// [`RenderBackend::change_texture_palette`]: the command queue can't
    /// reach the renderer's palettes.
    palette: Option<GLuint>,
}

fn gl_data(texture: &TextureData) -> Option<&GlTextureData> {
    texture.internal.as_ref()?.downcast_ref::<GlTextureData>()
}

fn gl_data_mut(texture: &mut TextureData) -> Option<&mut GlTextureData> {
    texture.internal.as_mut()?.downcast_mut::<GlTextureData>()
}

/// The OpenGL renderer's data. Translation of `GL_RenderData`.
pub(crate) struct GlRenderer {
    window: Window,
    /// `None` once destroyed.
    context: Option<GlContext>,

    debug_enabled: bool,
    /// Set when `GL_ARB_debug_output` is supported (dropped after the
    /// context, which may still call the callback).
    debug: Option<Box<DebugOutput>>,
    pixelart_supported: bool,

    textype: GLenum,

    gl_arb_texture_non_power_of_two_supported: bool,
    gl_arb_texture_rectangle_supported: bool,
    /// The `GL_EXT_framebuffer_object` functions
    /// (`GL_EXT_framebuffer_object_supported`).
    fbo_funcs: Option<FboFuncs>,
    framebuffers: Vec<Fbo>,

    // OpenGL functions
    gl: GlFuncs,

    // Multitexture support
    gl_arb_multitexture_supported: bool,
    gl_active_texture_arb: Option<PfnActiveTexture>,
    num_texture_units: GLint,

    // Shader support
    shaders: Option<ShaderContext>,

    drawstate: DrawStateCache,

    /// The vertex data of the queued commands (`first` indexes floats).
    verts: Vec<f32>,
    /// `renderer->target`.
    target: Option<Texture>,
    /// What creation set on the front end.
    setup: RendererSetup,
    next_texture_id: u64,
}

/// The file upstream's errors name.
const ERROR_FILE: &str = "SDL_render_gl.c";

/// `GL_CheckError(prefix, renderer)` in `function` (upstream's
/// `SDL_FUNCTION`; the line is this file's).
macro_rules! gl_check_error {
    ($self:expr, $prefix:expr, $function:literal) => {
        $self.check_all_errors($prefix, line!(), $function)
    };
}

/// Translation of `GL_TranslateError()`.
fn translate_error(error: GLenum) -> &'static str {
    match error {
        GL_INVALID_ENUM => "GL_INVALID_ENUM",
        GL_INVALID_VALUE => "GL_INVALID_VALUE",
        GL_INVALID_OPERATION => "GL_INVALID_OPERATION",
        GL_OUT_OF_MEMORY => "GL_OUT_OF_MEMORY",
        GL_NO_ERROR => "GL_NO_ERROR",
        GL_STACK_OVERFLOW => "GL_STACK_OVERFLOW",
        GL_STACK_UNDERFLOW => "GL_STACK_UNDERFLOW",
        GL_TABLE_TOO_LARGE => "GL_TABLE_TOO_LARGE",
        _ => "UNKNOWN",
    }
}

/// Translation of `GL_HandleDebugMessage()`.
unsafe extern "system" fn handle_debug_message(
    source: GLenum,
    type_: GLenum,
    id: GLuint,
    severity: GLenum,
    length: GLsizei,
    message: *const c_char,
    user_param: *const c_void,
) {
    // SAFETY: the user parameter is the renderer's DebugOutput, which
    // outlives the callback's registration.
    let debug = unsafe { &*(user_param as *const DebugOutput) };
    let text = if message.is_null() {
        String::new()
    } else {
        // SAFETY: GL passes a NUL-terminated message.
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    };

    if type_ == GL_DEBUG_TYPE_ERROR_ARB {
        // Record this error
        debug
            .errors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(text.clone());
    }

    // If there's another error callback, pass it along, otherwise log it
    if let Some(next) = debug.next_error_callback {
        // SAFETY: the callback and its parameter were installed together.
        unsafe {
            next(
                source,
                type_,
                id,
                severity,
                length,
                message,
                debug.next_error_userparam,
            )
        };
    } else if type_ == GL_DEBUG_TYPE_ERROR_ARB {
        log::error!(Category::Render, "{}", text);
    } else {
        log::debug!(Category::Render, "{}", text);
    }
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
        Some(BlendOperation::Minimum) => GL_MIN,
        Some(BlendOperation::Maximum) => GL_MAX,
        None => GL_INVALID_ENUM,
    }
}

/// Translation of `GL_SupportsBlendMode()`.
fn supports_blend_mode(blend_mode: BlendMode) -> bool {
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
    if color_operation != alpha_operation {
        return false;
    }
    true
}

/// The GL internal format, format and type of a pixel format.
/// Translation of `convert_format()`.
fn convert_format(pixel_format: PixelFormat) -> Option<(GLint, GLenum, GLenum)> {
    use PixelFormat as F;
    Some(match pixel_format {
        // (the type was GL_UNSIGNED_INT_8_8_8_8_REV previously, upstream is
        // seeing if GL_UNSIGNED_BYTE is better in modern times)
        F::BGRA32 | F::BGRX32 => (GL_RGBA8, GL_BGRA, GL_UNSIGNED_BYTE),
        F::RGBA32 | F::RGBX32 => (GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE),
        F::INDEX8 | F::YV12 | F::IYUV | F::I444 | F::NV12 | F::NV21 => {
            (GL_LUMINANCE as GLint, GL_LUMINANCE, GL_UNSIGNED_BYTE)
        }
        #[cfg(target_os = "macos")]
        F::UYVY => (GL_RGB8, GL_YCBCR_422_APPLE, GL_UNSIGNED_SHORT_8_8_APPLE),
        _ => return None,
    })
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

/// The start of the pixels `glTexSubImage2D()` reads for a `w` x `h`
/// region of `bpp`-byte pixels whose rows are `row_length` pixels apart
/// (`GL_UNPACK_ROW_LENGTH`, 0 for `w`) at `offset` in `pixels`, with an
/// unpack alignment of 1; an error if `pixels` is too short. Upstream
/// trusts the caller's sizes.
fn unpack_source(
    pixels: &[u8],
    offset: usize,
    w: i32,
    h: i32,
    row_length: i32,
    bpp: usize,
) -> Result<*const c_void> {
    let (w, h) = (w.max(0) as usize, h.max(0) as usize);
    let stride = if row_length > 0 {
        row_length as usize
    } else {
        w
    } * bpp;
    let needed = if w == 0 || h == 0 {
        0
    } else {
        (h - 1) * stride + w * bpp
    };
    match pixels.get(offset..) {
        Some(rest) if rest.len() >= needed => Ok(rest.as_ptr() as *const c_void),
        _ => Err(Error::invalid_param("pixels")),
    }
}

/// Translation of `SDL_powerof2()`.
fn powerof2(x: i32) -> i32 {
    if x <= 0 {
        // Return some sane value - we shouldn't hit this in our use cases
        return 1;
    }
    // This trick works for 32-bit values
    let mut value = x - 1;
    value |= value >> 1;
    value |= value >> 2;
    value |= value >> 4;
    value |= value >> 8;
    value |= value >> 16;
    value + 1
}

/// The `OPENGL` texture creation property `name`, if set (0 is unset).
fn external_texture(name: Option<u32>) -> Option<GLuint> {
    name.filter(|&t| t != 0)
}

impl GlRenderer {
    /// Translation of `GL_ClearErrors()`.
    fn clear_errors(&self) {
        if !self.debug_enabled {
            return;
        }
        if let Some(debug) = &self.debug {
            debug
                .errors
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
        } else {
            // SAFETY: the renderer's context is current.
            while unsafe { (self.gl.glGetError)() } != GL_NO_ERROR {
                // continue;
            }
        }
    }

    /// Translation of `GL_CheckAllErrors()`: the last error of the GL calls
    /// since the errors were cleared, when the context has debugging on.
    fn check_all_errors(&self, prefix: &str, line: u32, function: &str) -> Result<()> {
        let mut result = Ok(());

        if !self.debug_enabled {
            return Ok(());
        }
        if let Some(debug) = &self.debug {
            let errors =
                std::mem::take(&mut *debug.errors.lock().unwrap_or_else(|e| e.into_inner()));
            for message in errors {
                result = Err(Error::new(format!(
                    "{prefix}: {ERROR_FILE} ({line}): {function} {message}"
                )));
            }
        } else {
            // check gl errors (can return multiple errors)
            loop {
                // SAFETY: the renderer's context is current.
                let error = unsafe { (self.gl.glGetError)() };
                if error != GL_NO_ERROR {
                    let prefix = if prefix.is_empty() { "generic" } else { prefix };
                    result = Err(Error::new(format!(
                        "{prefix}: {ERROR_FILE} ({line}): {function} {} (0x{error:X})",
                        translate_error(error)
                    )));
                } else {
                    break;
                }
            }
        }
        result
    }

    /// Make the renderer's context current. Translation of
    /// `GL_ActivateRenderer()`.
    ///
    /// Note (upstream): most callers go on with their GL calls when this
    /// fails; here they stop, as the GL functions belong to the window's
    /// library, which is unloaded once the window is destroyed (upstream
    /// destroys the renderer before that).
    fn activate(&self) -> Result<()> {
        if !self.window.is_valid() {
            return Err(Error::new("Invalid window"));
        }
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| Error::new("Renderer's OpenGL context was destroyed"))?;
        if !context.is_current() {
            context.make_current(Some(&self.window))?;
        }

        self.clear_errors();

        Ok(())
    }

    /// `glActiveTextureARB(unit)` (when multitexturing is supported).
    fn active_texture(&self, unit: GLenum) {
        if let Some(active_texture) = self.gl_active_texture_arb {
            // SAFETY: the renderer's context is current.
            unsafe { active_texture(unit) };
        }
    }

    /// The framebuffer object for render targets of `w` x `h`.
    /// Translation of `GL_GetFBO()`.
    fn get_fbo(&mut self, fbo_funcs: &FboFuncs, w: i32, h: i32) -> GLuint {
        if let Some(fbo) = self.framebuffers.iter().find(|f| f.w == w && f.h == h) {
            return fbo.fbo;
        }
        let mut fbo = 0;
        // SAFETY: the renderer's context is current; one name is written.
        unsafe { (fbo_funcs.glGenFramebuffersEXT)(1, &mut fbo) };
        self.framebuffers.push(Fbo { w, h, fbo });
        fbo
    }

    /// Translation of `SetTextureScaleMode()` (every `ScaleMode` is known,
    /// so it can't fail).
    fn set_texture_scale_mode(&self, textype: GLenum, format: PixelFormat, scale_mode: ScaleMode) {
        let gl = &self.gl;
        let (min, mag) = match scale_mode {
            ScaleMode::Nearest => (GL_NEAREST, GL_NEAREST),
            // Uses linear sampling if supported
            ScaleMode::PixelArt if !self.pixelart_supported => (GL_NEAREST, GL_NEAREST),
            ScaleMode::PixelArt | ScaleMode::Linear => {
                if format == PixelFormat::INDEX8 {
                    // We'll do linear sampling in the shader
                    (GL_NEAREST, GL_NEAREST)
                } else {
                    (GL_LINEAR, GL_LINEAR)
                }
            }
        };
        // SAFETY: the renderer's context is current.
        unsafe {
            (gl.glTexParameteri)(textype, GL_TEXTURE_MIN_FILTER, min);
            (gl.glTexParameteri)(textype, GL_TEXTURE_MAG_FILTER, mag);
        }
    }

    /// Translation of `SetTextureAddressMode()`.
    fn set_texture_address_mode(
        &self,
        textype: GLenum,
        address_mode_u: TextureAddressMode,
        address_mode_v: TextureAddressMode,
    ) {
        // SAFETY: the renderer's context is current.
        unsafe {
            (self.gl.glTexParameteri)(
                textype,
                GL_TEXTURE_WRAP_S,
                translate_address_mode(address_mode_u),
            );
            (self.gl.glTexParameteri)(
                textype,
                GL_TEXTURE_WRAP_T,
                translate_address_mode(address_mode_v),
            );
        }
    }

    /// `glTexImage2D()` without pixels, then the texture's filtering and
    /// wrapping (the steps for each plane in `GL_CreateTexture()`).
    #[allow(clippy::too_many_arguments)]
    fn alloc_plane(
        &self,
        texture: GLuint,
        internal_format: GLint,
        w: i32,
        h: i32,
        format: GLenum,
        type_: GLenum,
        pixel_format: PixelFormat,
        data: &GlTextureData,
    ) {
        let textype = self.textype;
        // SAFETY: the renderer's context is current; no pixels are read.
        unsafe {
            (self.gl.glBindTexture)(textype, texture);
            (self.gl.glTexImage2D)(
                textype,
                0,
                internal_format,
                w,
                h,
                0,
                format,
                type_,
                std::ptr::null(),
            );
        }
        self.set_texture_scale_mode(textype, pixel_format, data.texture_scale_mode);
        self.set_texture_address_mode(
            textype,
            data.texture_address_mode_u,
            data.texture_address_mode_v,
        );
    }

    /// A texture name: the external one given, or a new one.
    fn texture_name(&self, external: Option<GLuint>) -> (GLuint, bool) {
        match external {
            Some(t) => (t, true),
            None => {
                let mut t = 0;
                // SAFETY: the renderer's context is current; one name is
                // written.
                unsafe { (self.gl.glGenTextures)(1, &mut t) };
                (t, false)
            }
        }
    }

    /// Translation of `GL_UpdateTexture()` on the texture data.
    fn update_texture_data(
        &mut self,
        format: PixelFormat,
        data: &GlTextureData,
        rect: &Rect,
        pixels: &[u8],
        pitch: usize,
    ) -> Result<()> {
        let textype = self.textype;
        let texturebpp = format.bytes_per_pixel() as usize;

        crate::sdl_assert_release!(texturebpp != 0); // otherwise, division by zero later.

        self.activate()?;

        self.drawstate.texture = None; // we trash this state.

        let gl = &self.gl;
        let pitch_i = pitch as i32;
        let row_length = (pitch / texturebpp) as GLint;
        let (x, y, w, h) = (rect.x, rect.y, rect.w, rect.h);
        let src = unpack_source(pixels, 0, w, h, row_length, texturebpp)?;
        // SAFETY: the renderer's context is current; `src` holds the rows
        // GL reads (checked by unpack_source).
        unsafe {
            (gl.glBindTexture)(textype, data.texture);
            (gl.glPixelStorei)(GL_UNPACK_ALIGNMENT, 1);
            (gl.glPixelStorei)(GL_UNPACK_ROW_LENGTH, row_length);
            (gl.glTexSubImage2D)(textype, 0, x, y, w, h, data.format, data.formattype, src);
        }
        // FIXME (upstream): the U and V planes are taken to follow the
        // rows of `rect` (rect->h * pitch past `pixels`), not the planes of
        // a whole texture, so unlocking part of a YUV texture uploads the
        // wrong chroma (and reads past the pixels near the bottom, which is
        // an error here).
        let mut offset = h as usize * pitch;
        if data.yuv {
            if format == PixelFormat::I444 {
                // Skip to the correct offset into the next texture
                let u = unpack_source(pixels, offset, w, h, row_length, 1)?;
                offset += h as usize * pitch;
                let v = unpack_source(pixels, offset, w, h, row_length, 1)?;
                // SAFETY: as above.
                unsafe {
                    (gl.glBindTexture)(textype, data.utexture);
                    (gl.glTexSubImage2D)(textype, 0, x, y, w, h, data.format, data.formattype, u);

                    (gl.glBindTexture)(textype, data.vtexture);
                    (gl.glTexSubImage2D)(textype, 0, x, y, w, h, data.format, data.formattype, v);
                }
            } else {
                let half_pitch = (pitch_i + 1) / 2;
                let (cx, cy, cw, ch) = (x / 2, y / 2, (w + 1) / 2, (h + 1) / 2);

                // Skip to the correct offset into the next texture
                let first = unpack_source(pixels, offset, cw, ch, half_pitch, 1)?;
                offset += ch as usize * half_pitch as usize;
                let second = unpack_source(pixels, offset, cw, ch, half_pitch, 1)?;
                let (first_tex, second_tex) = if format == PixelFormat::YV12 {
                    (data.vtexture, data.utexture)
                } else {
                    (data.utexture, data.vtexture)
                };
                // SAFETY: as above.
                unsafe {
                    (gl.glPixelStorei)(GL_UNPACK_ROW_LENGTH, half_pitch);

                    (gl.glBindTexture)(textype, first_tex);
                    (gl.glTexSubImage2D)(
                        textype,
                        0,
                        cx,
                        cy,
                        cw,
                        ch,
                        data.format,
                        data.formattype,
                        first,
                    );

                    (gl.glBindTexture)(textype, second_tex);
                    (gl.glTexSubImage2D)(
                        textype,
                        0,
                        cx,
                        cy,
                        cw,
                        ch,
                        data.format,
                        data.formattype,
                        second,
                    );
                }
            }
        }

        if data.nv12 {
            let half_pitch = (pitch_i + 1) / 2;
            let (cx, cy, cw, ch) = (x / 2, y / 2, (w + 1) / 2, (h + 1) / 2);
            // Skip to the correct offset into the next texture
            let uv = unpack_source(pixels, h as usize * pitch, cw, ch, half_pitch, 2)?;
            // SAFETY: as above.
            unsafe {
                (gl.glPixelStorei)(GL_UNPACK_ROW_LENGTH, half_pitch);
                (gl.glBindTexture)(textype, data.utexture);
                (gl.glTexSubImage2D)(
                    textype,
                    0,
                    cx,
                    cy,
                    cw,
                    ch,
                    GL_LUMINANCE_ALPHA,
                    GL_UNSIGNED_BYTE,
                    uv,
                );
            }
        }
        gl_check_error!(self, "glTexSubImage2D()", "GL_UpdateTexture")
    }

    /// Translation of `SetDrawState()`.
    fn set_draw_state(
        &mut self,
        kind: DrawKind,
        cmd: &DrawCmd,
        shader: Shader,
        shader_params: Option<ShaderParams>,
    ) -> bool {
        let blend = cmd.blend;
        let gl = &self.gl;
        let ds = &mut self.drawstate;

        // SAFETY (all the GL calls below): the renderer's context is current.
        if ds.viewport_dirty {
            let istarget = ds.target.is_some();
            let viewport = ds.viewport;
            unsafe {
                (gl.glMatrixMode)(GL_PROJECTION);
                (gl.glLoadIdentity)();
                (gl.glViewport)(
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
                    (gl.glOrtho)(
                        0.0,
                        viewport.w as GLdouble,
                        (if istarget { 0 } else { viewport.h }) as GLdouble,
                        (if istarget { viewport.h } else { 0 }) as GLdouble,
                        0.0,
                        1.0,
                    );
                }
                (gl.glMatrixMode)(GL_MODELVIEW);
            }
            ds.viewport_dirty = false;
        }

        if ds.cliprect_enabled_dirty {
            unsafe {
                if !ds.cliprect_enabled {
                    (gl.glDisable)(GL_SCISSOR_TEST);
                } else {
                    (gl.glEnable)(GL_SCISSOR_TEST);
                }
            }
            ds.cliprect_enabled_dirty = false;
        }

        if ds.cliprect_enabled && ds.cliprect_dirty {
            let viewport = ds.viewport;
            let rect = ds.cliprect;
            unsafe {
                (gl.glScissor)(
                    viewport.x + rect.x,
                    if ds.target.is_some() {
                        viewport.y + rect.y
                    } else {
                        ds.drawableh - viewport.y - rect.y - rect.h
                    },
                    rect.w,
                    rect.h,
                );
            }
            ds.cliprect_dirty = false;
        }

        if blend != ds.blend {
            unsafe {
                if blend == BlendMode::NONE {
                    (gl.glDisable)(GL_BLEND);
                } else {
                    (gl.glEnable)(GL_BLEND);
                    (gl.glBlendFuncSeparate)(
                        get_blend_func(blend.src_color_factor()),
                        get_blend_func(blend.dst_color_factor()),
                        get_blend_func(blend.src_alpha_factor()),
                        get_blend_func(blend.dst_alpha_factor()),
                    );
                    (gl.glBlendEquation)(get_blend_equation(blend.color_operation()));
                }
            }
            ds.blend = blend;
        }

        if let Some(shaders) = &mut self.shaders {
            if Some(shader) != ds.shader || shader_params != ds.shader_params {
                shaders.select(shader, shader_params.as_ref());
                ds.shader = Some(shader);
                ds.shader_params = shader_params;
            }
        }

        if ds.texturing_dirty || (cmd.texture.is_some() != ds.texturing) {
            unsafe {
                if cmd.texture.is_none() {
                    (gl.glDisable)(self.textype);
                    ds.texturing = false;
                } else {
                    (gl.glEnable)(self.textype);
                    ds.texturing = true;
                }
            }
            ds.texturing_dirty = false;
        }

        let vertex_array = matches!(
            kind,
            DrawKind::Points | DrawKind::Lines | DrawKind::Geometry
        );
        let color_array = kind == DrawKind::Geometry;
        let texture_array = cmd.texture.is_some();

        if vertex_array != ds.vertex_array {
            unsafe {
                if vertex_array {
                    (gl.glEnableClientState)(GL_VERTEX_ARRAY);
                } else {
                    (gl.glDisableClientState)(GL_VERTEX_ARRAY);
                }
            }
            ds.vertex_array = vertex_array;
        }

        if color_array != ds.color_array {
            unsafe {
                if color_array {
                    (gl.glEnableClientState)(GL_COLOR_ARRAY);
                } else {
                    (gl.glDisableClientState)(GL_COLOR_ARRAY);
                }
            }
            ds.color_array = color_array;
        }

        /* This is a little awkward but should avoid texcoord arrays getting into
        a bad state if the application is manually binding textures */
        if texture_array != ds.texture_array {
            unsafe {
                if texture_array {
                    (gl.glEnableClientState)(GL_TEXTURE_COORD_ARRAY);
                } else {
                    (gl.glDisableClientState)(GL_TEXTURE_COORD_ARRAY);
                }
            }
            ds.texture_array = texture_array;
        }

        true
    }

    /// Translation of `SetCopyState()`.
    fn set_copy_state(
        &mut self,
        kind: DrawKind,
        cmd: &DrawCmd,
        textures: &mut TextureStore,
    ) -> bool {
        let Some(texture) = cmd.texture.and_then(|t| textures.get_mut(t)) else {
            return false;
        };
        let format = texture.format;
        let Some(texturedata) = gl_data_mut(texture) else {
            return false;
        };
        let textype = self.textype;
        let mut shader = texturedata.shader;
        let mut shader_params = texturedata.shader_params;

        let pixelart = cmd.texture_scale_mode == ScaleMode::PixelArt && self.pixelart_supported;
        let scaled_shader = match shader {
            Shader::PaletteNearest if cmd.texture_scale_mode == ScaleMode::Linear => {
                Some(Shader::PaletteLinear)
            }
            Shader::PaletteNearest if pixelart => Some(Shader::PalettePixelart),
            Shader::Rgb if pixelart => Some(Shader::RgbPixelart),
            Shader::Rgba if pixelart => Some(Shader::RgbaPixelart),
            _ => None,
        };
        if let Some(scaled_shader) = scaled_shader {
            shader = scaled_shader;
            shader_params = Some(ShaderParams::TexelSize(texturedata.texel_size));
        }
        self.set_draw_state(kind, cmd, shader, shader_params);

        let gl = &self.gl;
        if Some(texturedata.id) != self.drawstate.texture {
            // SAFETY: the renderer's context is current.
            unsafe {
                if texturedata.yuv {
                    self.active_texture(GL_TEXTURE2_ARB);
                    (gl.glBindTexture)(textype, texturedata.vtexture);

                    self.active_texture(GL_TEXTURE1_ARB);
                    (gl.glBindTexture)(textype, texturedata.utexture);
                }
                if texturedata.nv12 {
                    self.active_texture(GL_TEXTURE1_ARB);
                    (gl.glBindTexture)(textype, texturedata.utexture);
                }
                if let Some(palette) = texturedata.palette {
                    self.active_texture(GL_TEXTURE1_ARB);
                    (gl.glBindTexture)(textype, palette);
                }
                if self.gl_arb_multitexture_supported {
                    self.active_texture(GL_TEXTURE0_ARB);
                }
                (gl.glBindTexture)(textype, texturedata.texture);
            }

            self.drawstate.texture = Some(texturedata.id);
        }

        if cmd.texture_scale_mode != texturedata.texture_scale_mode {
            if texturedata.yuv {
                self.active_texture(GL_TEXTURE2_ARB);
                self.set_texture_scale_mode(textype, format, cmd.texture_scale_mode);

                self.active_texture(GL_TEXTURE1_ARB);
                self.set_texture_scale_mode(textype, format, cmd.texture_scale_mode);

                self.active_texture(GL_TEXTURE0_ARB);
            } else if texturedata.nv12 {
                self.active_texture(GL_TEXTURE1_ARB);
                self.set_texture_scale_mode(textype, format, cmd.texture_scale_mode);

                self.active_texture(GL_TEXTURE0_ARB);
            }
            if texturedata.palette.is_some() {
                self.active_texture(GL_TEXTURE1_ARB);
                self.set_texture_scale_mode(textype, PixelFormat::UNKNOWN, ScaleMode::Nearest);

                self.active_texture(GL_TEXTURE0_ARB);
            }
            self.set_texture_scale_mode(textype, format, cmd.texture_scale_mode);

            texturedata.texture_scale_mode = cmd.texture_scale_mode;
        }

        if cmd.texture_address_mode_u != texturedata.texture_address_mode_u
            || cmd.texture_address_mode_v != texturedata.texture_address_mode_v
        {
            let (u, v) = (cmd.texture_address_mode_u, cmd.texture_address_mode_v);
            if texturedata.yuv {
                self.active_texture(GL_TEXTURE2_ARB);
                self.set_texture_address_mode(textype, u, v);

                self.active_texture(GL_TEXTURE1_ARB);
                self.set_texture_address_mode(textype, u, v);

                self.active_texture(GL_TEXTURE0_ARB);
            } else if texturedata.nv12 {
                self.active_texture(GL_TEXTURE1_ARB);
                self.set_texture_address_mode(textype, u, v);

                self.active_texture(GL_TEXTURE0_ARB);
            }
            if texturedata.palette.is_some() {
                self.active_texture(GL_TEXTURE1_ARB);
                self.set_texture_address_mode(
                    textype,
                    TextureAddressMode::Clamp,
                    TextureAddressMode::Clamp,
                );

                self.active_texture(GL_TEXTURE0_ARB);
            }
            self.set_texture_address_mode(textype, u, v);

            texturedata.texture_address_mode_u = u;
            texturedata.texture_address_mode_v = v;
        }

        true
    }

    /// Translation of `GL_DestroyRenderer()` (once; also when creation
    /// fails halfway, as the front end does upstream).
    fn destroy_renderer(&mut self) {
        let Some(context) = self.context.take() else {
            return;
        };

        // make sure we delete the right resources!
        // (only when the window, and so the GL library, is still there)
        let active = self.window.is_valid()
            && (context.is_current() || context.make_current(Some(&self.window)).is_ok());
        if active {
            self.clear_errors();
            if let Some(debug) = &self.debug {
                // Uh oh, we don't have a safe way of removing ourselves from the callback chain, if it changed after we set our callback.
                // For now, just always replace the callback with the original one
                // SAFETY: the context is current; the previous callback
                // and its parameter go back together.
                unsafe {
                    (debug.set_callback)(debug.next_error_callback, debug.next_error_userparam)
                };
            }
            if let Some(shaders) = self.shaders.take() {
                shaders.destroy();
            }
            if let Some(fbo_funcs) = self.fbo_funcs {
                for fbo in std::mem::take(&mut self.framebuffers) {
                    // delete the framebuffer object
                    // SAFETY: the context is current; the name is ours.
                    unsafe { (fbo_funcs.glDeleteFramebuffersEXT)(1, &fbo.fbo) };
                    let _ = gl_check_error!(self, "", "GL_DestroyRenderer");
                }
            }
        }
        // SDL_GL_DestroyContext() (before the debug state the context's
        // callback points at goes)
        drop(context);
        self.debug = None;
    }
}

impl Drop for GlRenderer {
    fn drop(&mut self) {
        self.destroy_renderer();
    }
}

impl GlRenderer {
    /// Create the OpenGL renderer for `window`, which is reconfigured for
    /// OpenGL if needed. Translation of `GL_CreateRenderer()`.
    pub(crate) fn for_window(window: Window, output_colorspace: Colorspace) -> Result<GlRenderer> {
        // SDL_SetupRendererColorspace(renderer, create_props)
        if output_colorspace != Colorspace::SRGB {
            return Err(Error::new("Unsupported output colorspace"));
        }

        let profile_mask = gl::gl_get_attribute(GlAttr::ContextProfileMask)?;
        let major = gl::gl_get_attribute(GlAttr::ContextMajorVersion)?;
        let minor = gl::gl_get_attribute(GlAttr::ContextMinorVersion)?;

        let _ = window.sync();
        let window_flags = window.flags().unwrap_or_default();
        let mut changed_window = false;
        let result = (|| {
            if !window_flags.contains(WindowFlags::OPENGL)
                || profile_mask == gl::GL_CONTEXT_PROFILE_ES
                || major != RENDERER_CONTEXT_MAJOR
                || minor != RENDERER_CONTEXT_MINOR
            {
                changed_window = true;
                let _ = gl::gl_set_attribute(GlAttr::ContextProfileMask, 0);
                let _ = gl::gl_set_attribute(GlAttr::ContextMajorVersion, RENDERER_CONTEXT_MAJOR);
                let _ = gl::gl_set_attribute(GlAttr::ContextMinorVersion, RENDERER_CONTEXT_MINOR);

                window.reconfigure(
                    (window_flags & !(WindowFlags::VULKAN | WindowFlags::METAL))
                        | WindowFlags::OPENGL,
                )?;
            }
            // (a renderer that fails halfway is dropped, destroying what it
            // has, before the window is put back)
            GlRenderer::create_on(window)
        })();

        if result.is_err() && changed_window {
            // Uh oh, better try to put it back...
            let _ = gl::gl_set_attribute(GlAttr::ContextProfileMask, profile_mask);
            let _ = gl::gl_set_attribute(GlAttr::ContextMajorVersion, major);
            let _ = gl::gl_set_attribute(GlAttr::ContextMinorVersion, minor);
            let _ = window.recreate(window_flags);
        }
        result
    }

    /// The part of `GL_CreateRenderer()` once the window is an OpenGL
    /// window.
    fn create_on(window: Window) -> Result<GlRenderer> {
        let _ = gl::gl_set_attribute(GlAttr::FramebufferSrgbCapable, 0);
        let context = GlContext::new(&window)?;
        context.make_current(Some(&window))?;

        let gl = GlFuncs::load()?;

        let mut data = GlRenderer {
            window,
            context: Some(context),
            debug_enabled: false,
            debug: None,
            pixelart_supported: false,
            textype: GL_TEXTURE_2D,
            gl_arb_texture_non_power_of_two_supported: false,
            gl_arb_texture_rectangle_supported: false,
            fbo_funcs: None,
            framebuffers: Vec::new(),
            gl,
            gl_arb_multitexture_supported: false,
            gl_active_texture_arb: None,
            num_texture_units: 0,
            shaders: None,
            drawstate: DrawStateCache::default(),
            verts: Vec::new(),
            target: None,
            setup: RendererSetup::default(),
            next_texture_id: 1,
        };
        data.invalidate_cached_state();

        // Check for debug output support
        if gl::gl_get_attribute(GlAttr::ContextFlags)
            .is_ok_and(|value| value & gl::GL_CONTEXT_DEBUG_FLAG != 0)
        {
            data.debug_enabled = true;
        }
        if data.debug_enabled && gl::gl_extension_supported("GL_ARB_debug_output") {
            // SAFETY: the type is glDebugMessageCallbackARB's.
            let set_callback =
                unsafe { gl::gl_function::<PfnDebugMessageCallback>("glDebugMessageCallbackARB") };
            // Note (upstream): upstream calls the function without checking
            // that it was found; without it, errors are read with glGetError().
            if let Some(set_callback) = set_callback {
                let mut next_callback: *mut c_void = std::ptr::null_mut();
                let mut next_userparam: *mut c_void = std::ptr::null_mut();
                // SAFETY: the context is current; the out-parameters hold a
                // pointer each.
                unsafe {
                    (data.gl.glGetPointerv)(GL_DEBUG_CALLBACK_FUNCTION_ARB, &mut next_callback);
                    (data.gl.glGetPointerv)(GL_DEBUG_CALLBACK_USER_PARAM_ARB, &mut next_userparam);
                }
                let debug = Box::new(DebugOutput {
                    errors: Mutex::new(Vec::new()),
                    // SAFETY: GL_DEBUG_CALLBACK_FUNCTION_ARB is a
                    // GLDEBUGPROCARB (or NULL).
                    next_error_callback: unsafe {
                        std::mem::transmute::<*mut c_void, Option<DebugProc>>(next_callback)
                    },
                    next_error_userparam: next_userparam,
                    set_callback,
                });
                // SAFETY: the context is current; the user parameter is the
                // boxed state, which lives until the context is destroyed or
                // the callback is put back.
                unsafe {
                    set_callback(
                        Some(handle_debug_message),
                        &*debug as *const DebugOutput as *const c_void,
                    );
                }
                data.debug = Some(debug);

                // Make sure our callback is called when errors actually happen
                // SAFETY: the context is current.
                unsafe { (data.gl.glEnable)(GL_DEBUG_OUTPUT_SYNCHRONOUS_ARB) };
            }
        }

        // SAFETY: the context is current; the result is NULL or a
        // NUL-terminated string owned by GL.
        let verstr = unsafe {
            let s = (data.gl.glGetString)(GL_VERSION);
            (!s.is_null()).then(|| {
                CStr::from_ptr(s as *const c_char)
                    .to_string_lossy()
                    .into_owned()
            })
        };
        let (mut real_major, mut real_minor) = (0, 0);
        if let Some(verstr) = verstr {
            // (SDL_strlcpy() into a 16-byte buffer)
            let verbuf: String = verstr.chars().take(15).collect();
            if let Some((major, minor)) = verbuf.split_once('.') {
                real_minor = crate::stdlib::atoi(minor);
                real_major = crate::stdlib::atoi(major);
            }
        }

        let hint_allows = |name: &str| hints::get(name).is_none_or(|h| !h.starts_with('0'));

        let mut non_power_of_two_supported = false;
        if hint_allows("GL_ARB_texture_non_power_of_two")
            && (real_major >= 2 || gl::gl_extension_supported("GL_ARB_texture_non_power_of_two"))
        {
            non_power_of_two_supported = true;
        }

        let mut setup = RendererSetup {
            texture_formats: Vec::new(),
            // texture-rectangle doesn't support GL_REPEAT, it has to be the full NPOT extension (or real OpenGL 2.0+)
            npot_texture_wrap_unsupported: !non_power_of_two_supported,
            max_texture_size: 0,
        };

        let mut value: GLint = 0;
        data.textype = GL_TEXTURE_2D;
        // SAFETY (the glGetIntegerv calls): the context is current; `value`
        // is an out-parameter.
        if non_power_of_two_supported {
            data.gl_arb_texture_non_power_of_two_supported = true;
            unsafe { (data.gl.glGetIntegerv)(GL_MAX_TEXTURE_SIZE, &mut value) };
        } else if gl::gl_extension_supported("GL_ARB_texture_rectangle")
            || gl::gl_extension_supported("GL_EXT_texture_rectangle")
        {
            data.gl_arb_texture_rectangle_supported = true;
            data.textype = GL_TEXTURE_RECTANGLE_ARB;
            unsafe { (data.gl.glGetIntegerv)(GL_MAX_RECTANGLE_TEXTURE_SIZE_ARB, &mut value) };
        } else {
            unsafe { (data.gl.glGetIntegerv)(GL_MAX_TEXTURE_SIZE, &mut value) };
        }
        setup.max_texture_size = value;

        // Check for multitexture support
        if gl::gl_extension_supported("GL_ARB_multitexture") {
            // SAFETY: the type is glActiveTextureARB's.
            data.gl_active_texture_arb =
                unsafe { gl::gl_function::<PfnActiveTexture>("glActiveTextureARB") };
            if data.gl_active_texture_arb.is_some() {
                data.gl_arb_multitexture_supported = true;
                // SAFETY: as above.
                unsafe {
                    (data.gl.glGetIntegerv)(GL_MAX_TEXTURE_UNITS_ARB, &mut data.num_texture_units)
                };
            }
        }

        // Check for texture format support
        let mut bgra_supported = false;
        if hint_allows("GL_EXT_bgra")
            && (real_major > 1
                || (real_major == 1 && real_minor >= 2)
                || gl::gl_extension_supported("GL_EXT_bgra"))
        {
            bgra_supported = true;
        }

        let formats = &mut setup.texture_formats;
        // RGBA32 is always supported with OpenGL
        if bgra_supported {
            formats.push(PixelFormat::BGRA32); // SDL_PIXELFORMAT_ARGB8888 on little endian systems
        }
        formats.push(PixelFormat::RGBA32);

        // Check for shader support
        data.shaders = ShaderContext::new();
        log::info!(
            Category::Render,
            "OpenGL shaders: {}",
            if data.shaders.is_some() {
                "ENABLED"
            } else {
                "DISABLED"
            }
        );
        let supports = |shader| data.shaders.as_ref().is_some_and(|s| s.supports(shader));
        if supports(Shader::Rgb) {
            if bgra_supported {
                formats.push(PixelFormat::BGRX32);
            }
            formats.push(PixelFormat::RGBX32);
        } else {
            log::info!(Category::Render, "OpenGL RGB shaders not supported");
        }
        // We support PIXELART mode using a shader
        if supports(Shader::RgbPixelart) && supports(Shader::RgbaPixelart) {
            data.pixelart_supported = true;
        } else {
            log::info!(Category::Render, "OpenGL PIXELART shaders not supported");
        }
        // We support INDEX8 textures using 2 textures and a shader
        if supports(Shader::PaletteNearest)
            && supports(Shader::PaletteLinear)
            && (!data.pixelart_supported || supports(Shader::PalettePixelart))
            && data.num_texture_units >= 2
        {
            formats.push(PixelFormat::INDEX8);
        } else {
            log::info!(Category::Render, "OpenGL palette shaders not supported");
        }
        // We support YV12 textures using 3 textures and a shader
        if supports(Shader::Yuv) && data.num_texture_units >= 3 {
            formats.push(PixelFormat::YV12);
            formats.push(PixelFormat::IYUV);
            formats.push(PixelFormat::I444);
        } else {
            log::info!(Category::Render, "OpenGL YUV not supported");
        }

        // We support NV12 textures using 2 textures and a shader
        if supports(Shader::Nv12Ra)
            && supports(Shader::Nv12Rg)
            && supports(Shader::Nv21Ra)
            && supports(Shader::Nv21Rg)
            && data.num_texture_units >= 2
        {
            formats.push(PixelFormat::NV12);
            formats.push(PixelFormat::NV21);
        } else {
            log::info!(Category::Render, "OpenGL NV12/NV21 not supported");
        }
        #[cfg(target_os = "macos")]
        formats.push(PixelFormat::UYVY);

        // Note (upstream): upstream doesn't check that the extension's
        // functions were found; here a driver without them can't make
        // render targets either.
        data.fbo_funcs = if gl::gl_extension_supported("GL_EXT_framebuffer_object") {
            FboFuncs::load()
        } else {
            None
        };
        if data.fbo_funcs.is_none() {
            return Err(Error::new(
                "Can't create render targets, GL_EXT_framebuffer_object not available",
            ));
        }

        let gl = &data.gl;
        // SAFETY: the context is current.
        unsafe {
            if real_major >= 3
                || gl::gl_extension_supported("GL_EXT_framebuffer_sRGB")
                || gl::gl_extension_supported("GL_ARB_framebuffer_sRGB")
            {
                (gl.glDisable)(GL_FRAMEBUFFER_SRGB);
            }

            // Set up parameters for rendering
            (gl.glMatrixMode)(GL_MODELVIEW);
            (gl.glLoadIdentity)();
            (gl.glDisable)(GL_DEPTH_TEST);
            (gl.glDisable)(GL_CULL_FACE);
            (gl.glDisable)(GL_SCISSOR_TEST);
            (gl.glDisable)(data.textype);
            (gl.glClearColor)(1.0, 1.0, 1.0, 1.0);
            (gl.glColor4f)(1.0, 1.0, 1.0, 1.0);
            // This ended up causing video discrepancies between OpenGL and Direct3D
            // (gl.glEnable)(GL_LINE_SMOOTH);
        }

        let white = FColor::new(1.0, 1.0, 1.0, 1.0);
        data.drawstate.color = white;
        data.drawstate.clear_color = white;

        data.setup = setup;
        Ok(data)
    }

    /// `GL_CreateTexture()`'s texture size, setting the texture
    /// coordinate scale.
    fn texture_size(&self, w: i32, h: i32, data: &mut GlTextureData) -> (i32, i32) {
        if self.gl_arb_texture_non_power_of_two_supported {
            data.texw = 1.0;
            data.texh = 1.0;
            (w, h)
        } else if self.gl_arb_texture_rectangle_supported {
            data.texw = w as GLfloat;
            data.texh = h as GLfloat;
            (w, h)
        } else {
            let texture_w = powerof2(w);
            let texture_h = powerof2(h);
            data.texw = w as GLfloat / texture_w as GLfloat;
            data.texh = h as GLfloat / texture_h as GLfloat;
            (texture_w, texture_h)
        }
    }
}

impl RenderBackend for GlRenderer {
    fn name(&self) -> &'static str {
        OPENGL_RENDERER
    }

    fn output_size(&self, _textures: &TextureStore) -> Option<Result<(i32, i32)>> {
        None // (the window's size in pixels)
    }

    fn texture_formats(&self) -> Option<Vec<PixelFormat>> {
        Some(self.setup.texture_formats.clone())
    }

    fn npot_texture_wrap_unsupported(&self) -> bool {
        self.setup.npot_texture_wrap_unsupported
    }

    fn max_texture_size(&self) -> Option<i32> {
        Some(self.setup.max_texture_size)
    }

    fn supports_blend_mode(&self, mode: BlendMode) -> bool {
        supports_blend_mode(mode)
    }

    /// Translation of `GL_CreateTexture()`.
    fn create_texture(
        &mut self,
        texture: &mut TextureData,
        create_props: &TextureCreateProps,
    ) -> Result<()> {
        let textype = self.textype;
        let (w, h, pixel_format, access) = (texture.w, texture.h, texture.format, texture.access);

        self.activate()?;

        self.drawstate.texture = None; // we trash this state.
        self.drawstate.texturing_dirty = true; // we trash this state.

        if access == TextureAccess::Target && self.fbo_funcs.is_none() {
            return Err(Error::new("Render targets not supported by OpenGL"));
        }

        let Some((internal_format, format, type_)) = convert_format(pixel_format) else {
            return Err(Error::new(format!(
                "Texture format {} not supported by OpenGL",
                pixel_format.name()
            )));
        };

        let id = self.next_texture_id;
        self.next_texture_id += 1;
        let mut data = GlTextureData {
            id,
            texture: 0,
            texture_external: false,
            texw: 0.0,
            texh: 0.0,
            format,
            formattype: type_,
            shader: Shader::None,
            texel_size: [0.0; 4],
            shader_params: None,
            pixels: Vec::new(),
            pitch: 0,
            locked_rect: Rect::default(),
            yuv: false,
            nv12: false,
            utexture: 0,
            utexture_external: false,
            vtexture: 0,
            vtexture_external: false,
            texture_scale_mode: texture.scale_mode,
            texture_address_mode_u: TextureAddressMode::Clamp,
            texture_address_mode_v: TextureAddressMode::Clamp,
            fbo: None,
            palette: None,
        };

        if access == TextureAccess::Streaming {
            data.pitch = w * pixel_format.bytes_per_pixel() as i32;
            let pitch = data.pitch as usize;
            let hu = h as usize;
            let mut size = hu * pitch;
            if pixel_format == PixelFormat::YV12 || pixel_format == PixelFormat::IYUV {
                // Need to add size for the U and V planes
                size += 2 * hu.div_ceil(2) * pitch.div_ceil(2);
            }
            if pixel_format == PixelFormat::I444 {
                // Need to add size for the U and V planes
                size += 2 * hu * pitch;
            }
            if pixel_format == PixelFormat::NV12 || pixel_format == PixelFormat::NV21 {
                // Need to add size for the U/V plane
                size += 2 * hu.div_ceil(2) * pitch.div_ceil(2);
            }
            data.pixels = vec![0; size];
        }

        if access == TextureAccess::Target {
            if let Some(fbo_funcs) = self.fbo_funcs {
                data.fbo = Some(self.get_fbo(&fbo_funcs, w, h));
            }
        } else {
            data.fbo = None;
        }

        if let Some(t) = external_texture(create_props.opengl_texture) {
            data.texture = t;
            data.texture_external = true;
        } else {
            let _ = gl_check_error!(self, "", "GL_CreateTexture");
            data.texture = self.texture_name(None).0;
            gl_check_error!(self, "glGenTextures()", "GL_CreateTexture")?;
        }

        let (texture_w, texture_h) = self.texture_size(w, h, &mut data);
        let props = texture.props.get_or_insert_with(Properties::new).clone();
        let _ = props.set(PROP_TEXTURE_OPENGL_TEXTURE_NUMBER, data.texture as i64);
        let _ = props.set(PROP_TEXTURE_OPENGL_TEXTURE_TARGET_NUMBER, textype as i64);
        let _ = props.set(PROP_TEXTURE_OPENGL_TEX_W_FLOAT, data.texw);
        let _ = props.set(PROP_TEXTURE_OPENGL_TEX_H_FLOAT, data.texh);

        // (texture->internal = data: a failure from here on leaves the GL
        // textures to GL_DestroyTexture())
        let data = texture
            .internal
            .insert(Box::new(data))
            .downcast_mut::<GlTextureData>()
            .expect("just inserted");

        let gl = &self.gl;
        // SAFETY: the renderer's context is current; no pixels are read
        // (but for Apple's client storage, which reads the zeroed pixels
        // of a streaming texture, `pitch` wide).
        unsafe {
            (gl.glEnable)(textype);
            (gl.glBindTexture)(textype, data.texture);
            #[cfg(target_os = "macos")]
            {
                if access == TextureAccess::Streaming {
                    (gl.glTexParameteri)(
                        textype,
                        GL_TEXTURE_STORAGE_HINT_APPLE,
                        GL_STORAGE_SHARED_APPLE,
                    );
                } else {
                    (gl.glTexParameteri)(
                        textype,
                        GL_TEXTURE_STORAGE_HINT_APPLE,
                        GL_STORAGE_CACHED_APPLE,
                    );
                }
            }
            #[cfg(target_os = "macos")]
            let client_storage = access == TextureAccess::Streaming
                && pixel_format == PixelFormat::ARGB8888
                && (w % 8) == 0;
            #[cfg(not(target_os = "macos"))]
            let client_storage = false;
            if client_storage {
                #[cfg(target_os = "macos")]
                {
                    (gl.glPixelStorei)(GL_UNPACK_CLIENT_STORAGE_APPLE, 1);
                    (gl.glPixelStorei)(GL_UNPACK_ALIGNMENT, 1);
                    (gl.glPixelStorei)(
                        GL_UNPACK_ROW_LENGTH,
                        data.pitch / pixel_format.bytes_per_pixel() as i32,
                    );
                    (gl.glTexImage2D)(
                        textype,
                        0,
                        internal_format,
                        texture_w,
                        texture_h,
                        0,
                        format,
                        type_,
                        data.pixels.as_ptr() as *const c_void,
                    );
                    (gl.glPixelStorei)(GL_UNPACK_CLIENT_STORAGE_APPLE, 0);
                }
            } else {
                (gl.glTexImage2D)(
                    textype,
                    0,
                    internal_format,
                    texture_w,
                    texture_h,
                    0,
                    format,
                    type_,
                    std::ptr::null(),
                );
            }
        }
        gl_check_error!(self, "glTexImage2D()", "GL_CreateTexture")?;
        self.set_texture_scale_mode(textype, pixel_format, data.texture_scale_mode);
        self.set_texture_address_mode(
            textype,
            data.texture_address_mode_u,
            data.texture_address_mode_v,
        );

        if pixel_format == PixelFormat::YV12
            || pixel_format == PixelFormat::IYUV
            || pixel_format == PixelFormat::I444
        {
            data.yuv = true;

            (data.utexture, data.utexture_external) =
                self.texture_name(external_texture(create_props.opengl_texture_u));
            (data.vtexture, data.vtexture_external) =
                self.texture_name(external_texture(create_props.opengl_texture_v));

            let (plane_w, plane_h) = if pixel_format == PixelFormat::I444 {
                (texture_w, texture_h)
            } else {
                ((texture_w + 1) / 2, (texture_h + 1) / 2)
            };
            self.alloc_plane(
                data.utexture,
                internal_format,
                plane_w,
                plane_h,
                format,
                type_,
                pixel_format,
                data,
            );
            let _ = props.set(PROP_TEXTURE_OPENGL_TEXTURE_U_NUMBER, data.utexture as i64);

            self.alloc_plane(
                data.vtexture,
                internal_format,
                plane_w,
                plane_h,
                format,
                type_,
                pixel_format,
                data,
            );
            let _ = props.set(PROP_TEXTURE_OPENGL_TEXTURE_V_NUMBER, data.vtexture as i64);
        }

        if pixel_format == PixelFormat::NV12 || pixel_format == PixelFormat::NV21 {
            data.nv12 = true;

            (data.utexture, data.utexture_external) =
                self.texture_name(external_texture(create_props.opengl_texture_uv));
            self.alloc_plane(
                data.utexture,
                GL_LUMINANCE_ALPHA as GLint,
                (texture_w + 1) / 2,
                (texture_h + 1) / 2,
                GL_LUMINANCE_ALPHA,
                GL_UNSIGNED_BYTE,
                pixel_format,
                data,
            );
            let _ = props.set(PROP_TEXTURE_OPENGL_TEXTURE_UV_NUMBER, data.utexture as i64);
        }

        data.shader = if pixel_format == PixelFormat::INDEX8 {
            Shader::PaletteNearest
        } else if pixel_format == PixelFormat::RGBA32 || pixel_format == PixelFormat::BGRA32 {
            Shader::Rgba
        } else {
            Shader::Rgb
        };

        data.texel_size = [1.0 / w as f32, 1.0 / h as f32, w as f32, h as f32];

        if data.yuv || data.nv12 {
            let nv12_rg = hints::get_bool("SDL_RENDER_OPENGL_NV12_RG_SHADER", false);
            data.shader = if data.yuv {
                Shader::Yuv
            } else if pixel_format == PixelFormat::NV12 {
                if nv12_rg {
                    Shader::Nv12Rg
                } else {
                    Shader::Nv12Ra
                }
            } else if nv12_rg {
                Shader::Nv21Rg
            } else {
                Shader::Nv21Ra
            };
            let matrix = texture
                .colorspace
                .ycbcr_to_rgb_matrix(h as u32, 8)
                .ok_or_else(|| Error::new("Unsupported YUV colorspace"))?;
            let mut params = [0.0; 16];
            params[..3].copy_from_slice(&matrix.offset);
            for (row, coeff) in matrix.coeff.iter().enumerate() {
                params[4 + 4 * row..7 + 4 * row].copy_from_slice(coeff);
            }
            data.shader_params = Some(ShaderParams::YcbcrMatrix(params));
        }

        // SAFETY: the renderer's context is current.
        unsafe { (self.gl.glDisable)(textype) };

        gl_check_error!(self, "", "GL_CreateTexture")
    }

    fn queue_set_viewport(&mut self, _cmd: &mut RenderCommand) -> Result<()> {
        Ok(()) // nothing to do in this backend.
    }

    fn queue_set_draw_color(&mut self, _cmd: &mut RenderCommand) -> Result<()> {
        Ok(()) // nothing to do in this backend.
    }

    /* !!! FIXME: all these Queue* calls set up the vertex buffer the way the immediate mode
    !!! FIXME:  renderer wants it, but this might want to operate differently if we move to
    !!! FIXME:  VBOs at some point. */

    /// Translation of `GL_QueueDrawPoints()`.
    fn queue_draw_points(&mut self, cmd: &mut DrawCmd, points: &[FPoint]) -> Result<()> {
        cmd.first = self.verts.len();
        cmd.count = points.len();
        for p in points {
            self.verts.extend_from_slice(&[0.5 + p.x, 0.5 + p.y]);
        }
        Ok(())
    }

    /// Translation of `GL_QueueDrawLines()`.
    fn queue_draw_lines(&mut self, cmd: &mut DrawCmd, points: &[FPoint]) -> Option<Result<()>> {
        cmd.first = self.verts.len();
        cmd.count = points.len();
        let Some(first) = points.first() else {
            return Some(Ok(()));
        };

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

    /// Translation of `GL_QueueGeometry()`.
    fn queue_geometry(
        &mut self,
        cmd: &mut DrawCmd,
        texture: Option<&TextureData>,
        geometry: &Geometry<'_>,
        scale_x: f32,
        scale_y: f32,
    ) -> Result<()> {
        let texturedata = texture.and_then(gl_data);
        let count = geometry.count();
        let color_scale = cmd.color_scale;

        cmd.first = self.verts.len();
        cmd.count = count;
        self.verts
            .reserve(count * (6 + if texturedata.is_some() { 2 } else { 0 }));

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

    /// Translation of `GL_InvalidateCachedState()`.
    fn invalidate_cached_state(&mut self) {
        let cache = &mut self.drawstate;
        cache.viewport_dirty = true;
        cache.texture = None;
        cache.drawablew = 0;
        cache.drawableh = 0;
        cache.blend = BlendMode::INVALID;
        cache.shader = None;
        cache.cliprect_enabled_dirty = true;
        cache.cliprect_dirty = true;
        cache.texturing_dirty = true;
        cache.vertex_array = false; // !!! FIXME: this resets to false at the end of GL_RunCommandQueue, but we could cache this more aggressively.
        cache.color_array = false; // !!! FIXME: this resets to false at the end of GL_RunCommandQueue, but we could cache this more aggressively.
        cache.texture_array = false; // !!! FIXME: this resets to false at the end of GL_RunCommandQueue, but we could cache this more aggressively.
        cache.color_dirty = true;
        cache.clear_color_dirty = true;
    }

    /// Translation of `GL_RunCommandQueue()`.
    fn run_command_queue(
        &mut self,
        cmds: &[RenderCommand],
        textures: &mut TextureStore,
        _gpu_render_states: &crate::render::sysrender::GpuRenderStates,
    ) -> Result<()> {
        // !!! FIXME: it'd be nice to use a vertex buffer instead of immediate mode...
        self.activate()?;

        self.drawstate.target = self
            .target
            .and_then(|t| textures.get(t))
            .and_then(gl_data)
            .map(|d| d.id);
        if self.drawstate.target.is_none() {
            let (w, h) = self.window.size_in_pixels().unwrap_or((0, 0));
            if (w != self.drawstate.drawablew) || (h != self.drawstate.drawableh) {
                self.drawstate.viewport_dirty = true; // if the window dimensions changed, invalidate the current viewport, etc.
                self.drawstate.cliprect_dirty = true;
                self.drawstate.drawablew = w;
                self.drawstate.drawableh = h;
            }
        }

        #[cfg(target_os = "macos")]
        {
            // On macOS on older systems, the OpenGL view change and resize events aren't
            // necessarily synchronized, so just always reset it.
            // Workaround for: https://discourse.libsdl.org/t/sdl-2-0-22-prerelease/35306/6
            self.drawstate.viewport_dirty = true;
        }

        // (the vertices are read through raw pointers while the commands run)
        let verts = std::mem::take(&mut self.verts);
        let vertex_ptr = |first: usize| verts[first.min(verts.len())..].as_ptr() as *const c_void;

        let mut i = 0;
        while i < cmds.len() {
            match cmds[i] {
                RenderCommand::SetDrawColor {
                    color_scale, color, ..
                } => {
                    let r = color.r * color_scale;
                    let g = color.g * color_scale;
                    let b = color.b * color_scale;
                    let a = color.a;
                    let ds = &mut self.drawstate;
                    if ds.color_dirty
                        || (r != ds.color.r)
                        || (g != ds.color.g)
                        || (b != ds.color.b)
                        || (a != ds.color.a)
                    {
                        // SAFETY: the renderer's context is current.
                        unsafe { (self.gl.glColor4f)(r, g, b, a) };
                        ds.color = FColor::new(r, g, b, a);
                        ds.color_dirty = false;
                    }
                }

                RenderCommand::SetViewport { rect, .. } => {
                    let ds = &mut self.drawstate;
                    if ds.viewport != rect {
                        ds.viewport = rect;
                        ds.viewport_dirty = true;
                        ds.cliprect_dirty = true;
                    }
                }

                RenderCommand::SetClipRect { enabled, rect } => {
                    let ds = &mut self.drawstate;
                    if ds.cliprect_enabled != enabled {
                        ds.cliprect_enabled = enabled;
                        ds.cliprect_enabled_dirty = true;
                    }

                    if ds.cliprect != rect {
                        ds.cliprect = rect;
                        ds.cliprect_dirty = true;
                    }
                }

                RenderCommand::Clear {
                    color_scale, color, ..
                } => {
                    let r = color.r * color_scale;
                    let g = color.g * color_scale;
                    let b = color.b * color_scale;
                    let a = color.a;
                    let gl = &self.gl;
                    let ds = &mut self.drawstate;
                    // SAFETY: the renderer's context is current.
                    unsafe {
                        if ds.clear_color_dirty
                            || (r != ds.clear_color.r)
                            || (g != ds.clear_color.g)
                            || (b != ds.clear_color.b)
                            || (a != ds.clear_color.a)
                        {
                            (gl.glClearColor)(r, g, b, a);
                            ds.clear_color = FColor::new(r, g, b, a);
                            ds.clear_color_dirty = false;
                        }

                        if ds.cliprect_enabled || ds.cliprect_enabled_dirty {
                            (gl.glDisable)(GL_SCISSOR_TEST);
                            ds.cliprect_enabled_dirty = ds.cliprect_enabled;
                        }

                        (gl.glClear)(GL_COLOR_BUFFER_BIT);
                    }
                }

                // unused
                RenderCommand::Draw(DrawKind::FillRects | DrawKind::Copy | DrawKind::CopyEx, _) => {
                }

                RenderCommand::Draw(DrawKind::Lines, cmd) => {
                    if self.set_draw_state(DrawKind::Lines, &cmd, Shader::Solid, None) {
                        let mut count = cmd.count;
                        let gl = &self.gl;

                        // SetDrawState handles glEnableClientState.
                        // SAFETY: the renderer's context is current; the
                        // vertices of this and the grouped commands follow
                        // each other from `first`.
                        unsafe {
                            (gl.glVertexPointer)(2, GL_FLOAT, 8, vertex_ptr(cmd.first));

                            if count > 2 {
                                // joined lines cannot be grouped
                                (gl.glDrawArrays)(GL_LINE_STRIP, 0, count as GLsizei);
                            } else {
                                // let's group non joined lines
                                let mut finalcmd = i;
                                let thisblend = cmd.blend;

                                for (n, next) in cmds.iter().enumerate().skip(i + 1) {
                                    let RenderCommand::Draw(DrawKind::Lines, next) = next else {
                                        break; // can't go any further on this draw call, different render command up next.
                                    };
                                    if next.count != 2 {
                                        break; // can't go any further on this draw call, those are joined lines
                                    } else if next.blend != thisblend {
                                        break; // can't go any further on this draw call, different blendmode copy up next.
                                    } else {
                                        finalcmd = n; // we can combine copy operations here. Mark this one as the furthest okay command.
                                        count += next.count;
                                    }
                                }

                                (gl.glDrawArrays)(GL_LINES, 0, count as GLsizei);
                                i = finalcmd; // skip any copy commands we just combined in here.
                            }
                        }
                    }
                }

                RenderCommand::Draw(thiscmdtype @ (DrawKind::Points | DrawKind::Geometry), cmd) => {
                    /* as long as we have the same copy command in a row, with the
                    same texture, we can combine them all into a single draw call. */
                    let thistexture = cmd.texture;
                    let thisblend = cmd.blend;
                    let thisscalemode = cmd.texture_scale_mode;
                    let thisaddressmode_u = cmd.texture_address_mode_u;
                    let thisaddressmode_v = cmd.texture_address_mode_v;
                    let mut finalcmd = i;
                    let mut count = cmd.count;
                    for (n, next) in cmds.iter().enumerate().skip(i + 1) {
                        let RenderCommand::Draw(nextcmdtype, next) = next else {
                            break; // can't go any further on this draw call, different render command up next.
                        };
                        if *nextcmdtype != thiscmdtype {
                            break; // can't go any further on this draw call, different render command up next.
                        } else if next.texture != thistexture
                            || next.texture_scale_mode != thisscalemode
                            || next.texture_address_mode_u != thisaddressmode_u
                            || next.texture_address_mode_v != thisaddressmode_v
                            || next.blend != thisblend
                        {
                            break; // can't go any further on this draw call, different texture/blendmode copy up next.
                        } else {
                            finalcmd = n; // we can combine copy operations here. Mark this one as the furthest okay command.
                            count += next.count;
                        }
                    }

                    let ret = if thistexture.is_some() {
                        self.set_copy_state(thiscmdtype, &cmd, textures)
                    } else {
                        self.set_draw_state(thiscmdtype, &cmd, Shader::Solid, None)
                    };

                    if ret {
                        let gl = &self.gl;
                        let verts = |offset: usize| vertex_ptr(cmd.first + offset);
                        let op = if thiscmdtype == DrawKind::Points {
                            GL_POINTS
                        } else {
                            GL_TRIANGLES // SDL_RENDERCMD_GEOMETRY
                        };

                        // SAFETY: the renderer's context is current; the
                        // vertices of this and the grouped commands follow
                        // each other from `first`, in the layout given.
                        unsafe {
                            if thiscmdtype == DrawKind::Points {
                                // SetDrawState handles glEnableClientState.
                                (gl.glVertexPointer)(2, GL_FLOAT, 8, verts(0));
                            } else {
                                // SetDrawState handles glEnableClientState.
                                if thistexture.is_some() {
                                    (gl.glVertexPointer)(2, GL_FLOAT, 4 * 8, verts(0));
                                    (gl.glColorPointer)(4, GL_FLOAT, 4 * 8, verts(2));
                                    (gl.glTexCoordPointer)(2, GL_FLOAT, 4 * 8, verts(6));
                                } else {
                                    (gl.glVertexPointer)(2, GL_FLOAT, 4 * 6, verts(0));
                                    (gl.glColorPointer)(4, GL_FLOAT, 4 * 6, verts(2));
                                }
                            }

                            (gl.glDrawArrays)(op, 0, count as GLsizei);

                            // Restore previously set color when we're done.
                            if thiscmdtype != DrawKind::Points {
                                let c = self.drawstate.color;
                                (gl.glColor4f)(c.r, c.g, c.b, c.a);
                            }
                        }
                    }

                    i = finalcmd; // skip any copy commands we just combined in here.
                }

                RenderCommand::NoOp => {}
            }

            i += 1;
        }
        self.verts = verts;

        /* Turn off vertex array state when we're done, in case external code
        relies on it being off. */
        let gl = &self.gl;
        let ds = &mut self.drawstate;
        // SAFETY: the renderer's context is current.
        unsafe {
            if ds.vertex_array {
                (gl.glDisableClientState)(GL_VERTEX_ARRAY);
                ds.vertex_array = false;
            }
            if ds.color_array {
                (gl.glDisableClientState)(GL_COLOR_ARRAY);
                ds.color_array = false;
            }
            if ds.texture_array {
                (gl.glDisableClientState)(GL_TEXTURE_COORD_ARRAY);
                ds.texture_array = false;
            }
        }

        gl_check_error!(self, "", "GL_RunCommandQueue")
    }

    fn reset_vertices(&mut self) {
        self.verts.clear();
    }

    /// Translation of `GL_CreatePalette()`.
    fn create_palette(&mut self) -> Result<Box<dyn Any>> {
        // FIXME (upstream): GL_CreatePalette() doesn't activate the
        // renderer's context (GL_UpdatePalette() does), so the texture is
        // made in whatever context is current.
        // (but not once the window, and its GL library, is gone)
        if !self.window.is_valid() {
            return Err(Error::new("Invalid window"));
        }
        self.drawstate.texture = None; // we trash this state.

        let textype = self.textype;
        let mut palettedata = GlPaletteData { texture: 0 };
        // SAFETY: a context is current (see above); no pixels are read.
        unsafe {
            (self.gl.glGenTextures)(1, &mut palettedata.texture);
            (self.gl.glBindTexture)(textype, palettedata.texture);
            (self.gl.glTexImage2D)(
                textype,
                0,
                GL_RGBA8,
                256,
                1,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                std::ptr::null(),
            );
        }
        gl_check_error!(self, "glTexImage2D()", "GL_CreatePalette")?;
        self.set_texture_scale_mode(textype, PixelFormat::UNKNOWN, ScaleMode::Nearest);
        self.set_texture_address_mode(
            textype,
            TextureAddressMode::Clamp,
            TextureAddressMode::Clamp,
        );
        Ok(Box::new(palettedata))
    }

    /// Translation of `GL_UpdatePalette()`.
    fn update_palette(&mut self, palette: &mut dyn Any, colors: &[Color]) -> Result<()> {
        let palettedata = palette
            .downcast_ref::<GlPaletteData>()
            .ok_or_else(|| Error::invalid_param("palette"))?;

        self.activate()?;

        self.drawstate.texture = None; // we trash this state.

        let textype = self.textype;
        let ncolors = colors.len() as GLint;
        // SAFETY: the renderer's context is current; `colors` holds
        // `ncolors` RGBA8 pixels (`Color` is `repr(C)` r, g, b, a bytes).
        unsafe {
            (self.gl.glBindTexture)(textype, palettedata.texture);
            (self.gl.glPixelStorei)(GL_UNPACK_ALIGNMENT, 1);
            (self.gl.glPixelStorei)(GL_UNPACK_ROW_LENGTH, ncolors);
            (self.gl.glTexSubImage2D)(
                textype,
                0,
                0,
                0,
                ncolors,
                1,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                colors.as_ptr() as *const c_void,
            );
        }

        gl_check_error!(self, "glTexSubImage2D()", "GL_UpdatePalette")
    }

    /// Translation of `GL_DestroyPalette()`.
    fn destroy_palette(&mut self, palette: Box<dyn Any>) {
        if let Ok(palettedata) = palette.downcast::<GlPaletteData>() {
            // (upstream doesn't activate the context here; the GL library
            // may be gone with the window)
            if self.activate().is_ok() {
                // SAFETY: the renderer's context is current; the name is ours.
                unsafe { (self.gl.glDeleteTextures)(1, &palettedata.texture) };
            }
        }
    }

    /// Upstream has no `ChangeTexturePalette` here: the command queue reads
    /// the palette through the texture. This records the palette's GL
    /// texture in the texture data for the queue.
    fn change_texture_palette(
        &mut self,
        texture: &mut TextureData,
        palette: Option<&dyn Any>,
    ) -> Option<Result<()>> {
        let palette = palette
            .and_then(|p| p.downcast_ref::<GlPaletteData>())
            .map(|p| p.texture);
        // FIXME (upstream): the palette is bound with the texture, only when
        // the texture isn't the one bound already, so a texture given a
        // palette that exists already (no GL_CreatePalette() to trash the
        // bound texture) is drawn with its previous palette until another
        // texture is drawn.
        if let Some(data) = gl_data_mut(texture) {
            data.palette = palette;
        }
        Some(Ok(()))
    }

    /// Translation of `GL_UpdateTexture()`.
    fn update_texture(
        &mut self,
        texture: &mut TextureData,
        rect: &Rect,
        pixels: &[u8],
        pitch: usize,
    ) -> Result<()> {
        let format = texture.format;
        let data = gl_data(texture).ok_or_else(crate::render::sysrender::invalid_texture)?;
        self.update_texture_data(format, data, rect, pixels, pitch)
    }

    /// Translation of `GL_UpdateTextureYUV()`.
    fn update_texture_yuv(
        &mut self,
        texture: &mut TextureData,
        rect: &Rect,
        (yplane, ypitch): (&[u8], usize),
        (uplane, upitch): (&[u8], usize),
        (vplane, vpitch): (&[u8], usize),
    ) -> Option<Result<()>> {
        let format = texture.format;
        let Some(data) = gl_data(texture) else {
            return Some(Err(crate::render::sysrender::invalid_texture()));
        };
        let textype = self.textype;
        Some((|| {
            self.activate()?;

            self.drawstate.texture = None; // we trash this state.

            let (x, y, w, h) = (rect.x, rect.y, rect.w, rect.h);
            let (cx, cy, cw, ch) = if format == PixelFormat::I444 {
                (x, y, w, h)
            } else {
                (x / 2, y / 2, (w + 1) / 2, (h + 1) / 2)
            };
            let ysrc = unpack_source(yplane, 0, w, h, ypitch as GLint, 1)?;
            let usrc = unpack_source(uplane, 0, cw, ch, upitch as GLint, 1)?;
            let vsrc = unpack_source(vplane, 0, cw, ch, vpitch as GLint, 1)?;
            let gl = &self.gl;
            // SAFETY: the renderer's context is current; the planes hold
            // the rows GL reads (checked by unpack_source).
            unsafe {
                (gl.glBindTexture)(textype, data.texture);
                (gl.glPixelStorei)(GL_UNPACK_ALIGNMENT, 1);
                (gl.glPixelStorei)(GL_UNPACK_ROW_LENGTH, ypitch as GLint);
                (gl.glTexSubImage2D)(textype, 0, x, y, w, h, data.format, data.formattype, ysrc);

                (gl.glPixelStorei)(GL_UNPACK_ROW_LENGTH, upitch as GLint);
                (gl.glBindTexture)(textype, data.utexture);
                (gl.glTexSubImage2D)(
                    textype,
                    0,
                    cx,
                    cy,
                    cw,
                    ch,
                    data.format,
                    data.formattype,
                    usrc,
                );

                (gl.glPixelStorei)(GL_UNPACK_ROW_LENGTH, vpitch as GLint);
                (gl.glBindTexture)(textype, data.vtexture);
                (gl.glTexSubImage2D)(
                    textype,
                    0,
                    cx,
                    cy,
                    cw,
                    ch,
                    data.format,
                    data.formattype,
                    vsrc,
                );
            }

            gl_check_error!(self, "glTexSubImage2D()", "GL_UpdateTextureYUV")
        })())
    }

    /// Translation of `GL_UpdateTextureNV()`.
    fn update_texture_nv(
        &mut self,
        texture: &mut TextureData,
        rect: &Rect,
        (yplane, ypitch): (&[u8], usize),
        (uvplane, uvpitch): (&[u8], usize),
    ) -> Option<Result<()>> {
        let Some(data) = gl_data(texture) else {
            return Some(Err(crate::render::sysrender::invalid_texture()));
        };
        let textype = self.textype;
        Some((|| {
            self.activate()?;

            self.drawstate.texture = None; // we trash this state.

            let (x, y, w, h) = (rect.x, rect.y, rect.w, rect.h);
            let (cx, cy, cw, ch) = (x / 2, y / 2, (w + 1) / 2, (h + 1) / 2);
            let ysrc = unpack_source(yplane, 0, w, h, ypitch as GLint, 1)?;
            let uvsrc = unpack_source(uvplane, 0, cw, ch, (uvpitch / 2) as GLint, 2)?;
            let gl = &self.gl;
            // SAFETY: as in update_texture_yuv.
            unsafe {
                (gl.glBindTexture)(textype, data.texture);
                (gl.glPixelStorei)(GL_UNPACK_ALIGNMENT, 1);
                (gl.glPixelStorei)(GL_UNPACK_ROW_LENGTH, ypitch as GLint);
                (gl.glTexSubImage2D)(textype, 0, x, y, w, h, data.format, data.formattype, ysrc);

                (gl.glPixelStorei)(GL_UNPACK_ROW_LENGTH, (uvpitch / 2) as GLint);
                (gl.glBindTexture)(textype, data.utexture);
                (gl.glTexSubImage2D)(
                    textype,
                    0,
                    cx,
                    cy,
                    cw,
                    ch,
                    GL_LUMINANCE_ALPHA,
                    GL_UNSIGNED_BYTE,
                    uvsrc,
                );
            }

            gl_check_error!(self, "glTexSubImage2D()", "GL_UpdateTextureNV")
        })())
    }

    /// Translation of `GL_LockTexture()`.
    fn lock_texture(&mut self, texture: &mut TextureData, rect: &Rect) -> Result<(usize, i32)> {
        let bpp = texture.format.bytes_per_pixel() as usize;
        let data = gl_data_mut(texture).ok_or_else(crate::render::sysrender::invalid_texture)?;

        data.locked_rect = *rect;
        let offset = rect.y as usize * data.pitch as usize + rect.x as usize * bpp;
        Ok((offset, data.pitch))
    }

    fn texture_pixels_mut<'t>(&mut self, texture: &'t mut TextureData) -> Option<&'t mut [u8]> {
        gl_data_mut(texture).map(|d| d.pixels.as_mut_slice())
    }

    /// Translation of `GL_UnlockTexture()`.
    fn unlock_texture(&mut self, texture: &mut TextureData) {
        let format = texture.format;
        let bpp = format.bytes_per_pixel() as usize;
        let Some(data) = gl_data_mut(texture) else {
            return;
        };
        let rect = data.locked_rect;
        let offset = rect.y as usize * data.pitch as usize + rect.x as usize * bpp;
        let pitch = data.pitch as usize;
        let pixels = std::mem::take(&mut data.pixels);
        let Some(data) = gl_data(texture) else {
            return;
        };
        let _ = self.update_texture_data(
            format,
            data,
            &rect,
            pixels.get(offset..).unwrap_or(&[]),
            pitch,
        );
        if let Some(data) = gl_data_mut(texture) {
            data.pixels = pixels;
        }
    }

    /// Translation of `GL_SetRenderTarget()`.
    fn set_render_target(
        &mut self,
        target: Option<Texture>,
        textures: &TextureStore,
    ) -> Result<()> {
        self.target = target;

        self.activate()?;

        let Some(fbo_funcs) = self.fbo_funcs else {
            return Err(Error::new("Render targets not supported by OpenGL"));
        };

        self.drawstate.viewport_dirty = true;

        let Some(texture) = target else {
            // SAFETY: the renderer's context is current.
            unsafe { (fbo_funcs.glBindFramebufferEXT)(GL_FRAMEBUFFER_EXT, 0) };
            return Ok(());
        };

        let texturedata = textures
            .get(texture)
            .and_then(gl_data)
            .ok_or_else(crate::render::sysrender::invalid_texture)?;
        // SAFETY: the renderer's context is current; the names are ours.
        let status = unsafe {
            (fbo_funcs.glBindFramebufferEXT)(GL_FRAMEBUFFER_EXT, texturedata.fbo.unwrap_or(0));
            // TODO: check if texture pixel format allows this operation
            (fbo_funcs.glFramebufferTexture2DEXT)(
                GL_FRAMEBUFFER_EXT,
                GL_COLOR_ATTACHMENT0_EXT,
                self.textype,
                texturedata.texture,
                0,
            );
            // Check FBO status
            (fbo_funcs.glCheckFramebufferStatusEXT)(GL_FRAMEBUFFER_EXT)
        };
        if status != GL_FRAMEBUFFER_COMPLETE_EXT {
            return Err(Error::new("glFramebufferTexture2DEXT() failed"));
        }
        Ok(())
    }

    /// Translation of `GL_RenderReadPixels()`.
    fn read_pixels(
        &mut self,
        rect: &Rect,
        textures: &mut TextureStore,
    ) -> Option<Result<Surface<'static>>> {
        let target = self.target.and_then(|t| textures.get(t));
        let format = target.map_or(PixelFormat::RGBA32, |t| t.format);
        let has_target = target.is_some();
        Some((|| {
            let _ = self.activate();
            if !self.window.is_valid() {
                return Err(Error::new("Invalid window"));
            }

            let Some((_internal_format, target_format, type_)) = convert_format(format) else {
                return Err(Error::new(format!(
                    "Texture format {} not supported by OpenGL",
                    format.name()
                )));
            };

            let mut surface = Surface::new_uninitialized(rect.w, rect.h, format)?;

            let mut y = rect.y;
            if !has_target {
                let (_w, h) = self.window.size_in_pixels().unwrap_or((0, 0));
                y = (h - y) - rect.h;
            }

            let row_length = surface.pitch() / format.bytes_per_pixel() as i32;
            let pixels = surface
                .pixels_mut()
                .ok_or_else(|| Error::new("Surface has no pixels"))?;
            // SAFETY: the renderer's context is current; the surface holds
            // rect.h rows of row_length pixels.
            unsafe {
                (self.gl.glPixelStorei)(GL_PACK_ALIGNMENT, 1);
                (self.gl.glPixelStorei)(GL_PACK_ROW_LENGTH, row_length);
                (self.gl.glReadPixels)(
                    rect.x,
                    y,
                    rect.w,
                    rect.h,
                    target_format,
                    type_,
                    pixels.as_mut_ptr() as *mut c_void,
                );
            }

            gl_check_error!(self, "glReadPixels()", "GL_RenderReadPixels")?;

            // Flip the rows to be top-down if necessary
            if !has_target {
                surface.flip(FlipMode::Vertical)?;
            }
            Ok(surface)
        })())
    }

    /// Translation of `GL_RenderPresent()`.
    fn present(&mut self) -> bool {
        if self.activate().is_err() {
            return false;
        }

        gl::gl_swap_window(&self.window).is_ok()
    }

    /// Translation of `GL_DestroyTexture()`.
    fn destroy_texture(&mut self, texture: &mut TextureData) {
        let active = self.activate().is_ok();

        let Some(data) = texture.internal.take() else {
            return;
        };
        let Ok(data) = data.downcast::<GlTextureData>() else {
            return;
        };
        if self.drawstate.texture == Some(data.id) {
            self.drawstate.texture = None;
            self.drawstate.shader_params = None;
        }
        if self.drawstate.target == Some(data.id) {
            self.drawstate.target = None;
        }

        if !active {
            // (the GL objects go with the context)
            return;
        }
        let gl = &self.gl;
        // SAFETY: the renderer's context is current; the names are ours.
        unsafe {
            if data.texture != 0 && !data.texture_external {
                (gl.glDeleteTextures)(1, &data.texture);
            }
            if data.yuv {
                if !data.utexture_external {
                    (gl.glDeleteTextures)(1, &data.utexture);
                }
                if !data.vtexture_external {
                    (gl.glDeleteTextures)(1, &data.vtexture);
                }
            }
            if data.nv12 && !data.utexture_external {
                (gl.glDeleteTextures)(1, &data.utexture);
            }
        }
    }

    /// Translation of `GL_SetVSync()`.
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

    /// Translation of `GL_WindowEvent()`.
    fn window_event(&mut self, event_type: EventType) {
        /* If the window x/y/w/h changed at all, assume the viewport has been
         * changed behind our backs. x/y changes might seem weird but viewport
         * resets have been observed on macOS at minimum!
         */
        if event_type == EventType::WINDOW_RESIZED || event_type == EventType::WINDOW_MOVED {
            self.drawstate.viewport_dirty = true;
        }
    }

    fn destroy(&mut self) {
        self.destroy_renderer();
    }
}
