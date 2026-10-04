// Rust translation of src/video/x11/SDL_x11opengl.c and SDL_x11opengl.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// Copyright (C) 2021 NVIDIA Corporation
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! GLX implementation of SDL OpenGL support: libGL is loaded at run time
//! (and, as upstream, never unloaded: it may have registered X11 shutdown
//! hooks), the GLX entry points are declared here by hand.
//!
//! The device starts with these GLX functions, or with the EGL ones of
//! [`opengles`](super::opengles) when [`hints::VIDEO_FORCE_EGL`] is set;
//! either switches to the other when the requested profile needs it
//! (upstream swaps the device's function pointers: here [`GlBackend`]).

use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicI32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use super::modes::{display_driver_data_for_window, x11_get_pixel_format_from_visual_info};
use super::sys::*;
use super::video::{x11_use_direct_color_visuals, X11Video};
use super::window::with_x11_window;
use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::events::WindowID;
use crate::hints;
use crate::loadso::SharedObject;
use crate::video::core::with_window;
use crate::video::gl::{self, gl_config, GlConfig, RawGlContext, GL_CONTEXT_PROFILE_ES};

#[cfg(any(target_os = "netbsd", target_os = "openbsd"))]
/// NetBSD and OpenBSD have different GL library versions depending on how
/// the library was installed.
const DEFAULT_OPENGL: &str = "libGL.so";
#[cfg(not(any(target_os = "netbsd", target_os = "openbsd")))]
const DEFAULT_OPENGL: &str = "libGL.so.1";

pub(crate) type GLXContext = *mut c_void;
type GLXFBConfig = *mut c_void;
type GLXDrawable = XID;

const GLX_RGBA: c_int = 4;
const GLX_DOUBLEBUFFER: c_int = 5;
const GLX_STEREO: c_int = 6;
const GLX_RED_SIZE: c_int = 8;
const GLX_GREEN_SIZE: c_int = 9;
const GLX_BLUE_SIZE: c_int = 10;
const GLX_ALPHA_SIZE: c_int = 11;
const GLX_DEPTH_SIZE: c_int = 12;
const GLX_STENCIL_SIZE: c_int = 13;
const GLX_ACCUM_RED_SIZE: c_int = 14;
const GLX_ACCUM_GREEN_SIZE: c_int = 15;
const GLX_ACCUM_BLUE_SIZE: c_int = 16;
const GLX_ACCUM_ALPHA_SIZE: c_int = 17;
const GLX_RENDER_TYPE: c_int = 0x8011;
const GLX_RGBA_BIT: c_int = 0x00000001;
const GLX_BAD_CONTEXT: c_int = 5;

const GLX_NONE_EXT: c_int = 0x8000;

// GLX_ARB_multisample
const GLX_SAMPLE_BUFFERS_ARB: c_int = 100000;
const GLX_SAMPLES_ARB: c_int = 100001;

// GLX_EXT_visual_rating
const GLX_VISUAL_CAVEAT_EXT: c_int = 0x20;
const GLX_SLOW_VISUAL_EXT: c_int = 0x8001;

// GLX_EXT_visual_info
const GLX_X_VISUAL_TYPE_EXT: c_int = 0x22;
const GLX_DIRECT_COLOR_EXT: c_int = 0x8003;

// GLX_ARB_create_context
const GLX_CONTEXT_MAJOR_VERSION_ARB: c_int = 0x2091;
const GLX_CONTEXT_MINOR_VERSION_ARB: c_int = 0x2092;
const GLX_CONTEXT_FLAGS_ARB: c_int = 0x2094;

// GLX_ARB_create_context_profile
const GLX_CONTEXT_PROFILE_MASK_ARB: c_int = 0x9126;

// GLX_ARB_create_context_robustness
const GLX_CONTEXT_RESET_NOTIFICATION_STRATEGY_ARB: c_int = 0x8256;
const GLX_NO_RESET_NOTIFICATION_ARB: c_int = 0x8261;
const GLX_LOSE_CONTEXT_ON_RESET_ARB: c_int = 0x8252;

// GLX_ARB_framebuffer_sRGB
const GLX_FRAMEBUFFER_SRGB_CAPABLE_ARB: c_int = 0x20B2;

// GLX_ARB_fbconfig_float
const GLX_RGBA_FLOAT_TYPE_ARB: c_int = 0x20B9;
const GLX_RGBA_FLOAT_BIT_ARB: c_int = 0x00000004;

// GLX_ARB_create_context_no_error
const GLX_CONTEXT_OPENGL_NO_ERROR_ARB: c_int = 0x31B3;

// GLX_EXT_swap_control
const GLX_SWAP_INTERVAL_EXT: c_int = 0x20F1;

// GLX_EXT_swap_control_tear
const GLX_LATE_SWAPS_TEAR_EXT: c_int = 0x20F3;

// GLX_ARB_context_flush_control
const GLX_CONTEXT_RELEASE_BEHAVIOR_ARB: c_int = 0x2097;
const GLX_CONTEXT_RELEASE_BEHAVIOR_NONE_ARB: c_int = 0x0000;
const GLX_CONTEXT_RELEASE_BEHAVIOR_FLUSH_ARB: c_int = 0x2098;

type PfnGlXQueryExtension = unsafe extern "C" fn(*mut Display, *mut c_int, *mut c_int) -> Bool;
type PfnGlXGetProcAddress = unsafe extern "C" fn(*const u8) -> *mut c_void;
type PfnGlXChooseVisual = unsafe extern "C" fn(*mut Display, c_int, *mut c_int) -> *mut XVisualInfo;
type PfnGlXCreateContext =
    unsafe extern "C" fn(*mut Display, *mut XVisualInfo, GLXContext, Bool) -> GLXContext;
type PfnGlXCreateContextAttribsArb =
    unsafe extern "C" fn(*mut Display, GLXFBConfig, GLXContext, Bool, *const c_int) -> GLXContext;
type PfnGlXChooseFbConfig =
    unsafe extern "C" fn(*mut Display, c_int, *const c_int, *mut c_int) -> *mut GLXFBConfig;
type PfnGlXGetVisualFromFbConfig =
    unsafe extern "C" fn(*mut Display, GLXFBConfig) -> *mut XVisualInfo;
type PfnGlXDestroyContext = unsafe extern "C" fn(*mut Display, GLXContext);
type PfnGlXMakeCurrent = unsafe extern "C" fn(*mut Display, GLXDrawable, GLXContext) -> Bool;
type PfnGlXSwapBuffers = unsafe extern "C" fn(*mut Display, GLXDrawable);
type PfnGlXQueryDrawable = unsafe extern "C" fn(*mut Display, GLXDrawable, c_int, *mut c_uint);
type PfnGlXSwapIntervalExt = unsafe extern "C" fn(*mut Display, GLXDrawable, c_int);
type PfnGlXSwapIntervalSgi = unsafe extern "C" fn(c_int) -> c_int;
type PfnGlXSwapIntervalMesa = unsafe extern "C" fn(c_int) -> c_int;
type PfnGlXGetSwapIntervalMesa = unsafe extern "C" fn() -> c_int;
type PfnGlXGetCurrentContext = unsafe extern "C" fn() -> GLXContext;
type PfnGlXGetCurrentDrawable = unsafe extern "C" fn() -> GLXDrawable;
type PfnGlXQueryExtensionsString = unsafe extern "C" fn(*mut Display, c_int) -> *const c_char;

/// Which functions the device's `GL_*` pointers are: GLX
/// (`X11_GL_*`) or EGL (`X11_GLES_*`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum GlBackend {
    Glx,
    Egl,
}

/// Translation of `SDL_GLSwapIntervalTearBehavior`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
enum SwapIntervalTearBehavior {
    Untested,
    Unknown,
    Mesa,
    Nvidia,
}

/// The GLX entry points of `SDL_GLDriverData` (`None`: NULL).
#[derive(Clone, Copy, Default)]
#[allow(non_snake_case)] // (the GLX names)
struct GlxFns {
    glXQueryExtension: Option<PfnGlXQueryExtension>,
    glXGetProcAddress: Option<PfnGlXGetProcAddress>,
    glXChooseVisual: Option<PfnGlXChooseVisual>,
    glXCreateContext: Option<PfnGlXCreateContext>,
    glXCreateContextAttribsARB: Option<PfnGlXCreateContextAttribsArb>,
    glXChooseFBConfig: Option<PfnGlXChooseFbConfig>,
    glXGetVisualFromFBConfig: Option<PfnGlXGetVisualFromFbConfig>,
    glXDestroyContext: Option<PfnGlXDestroyContext>,
    glXMakeCurrent: Option<PfnGlXMakeCurrent>,
    glXSwapBuffers: Option<PfnGlXSwapBuffers>,
    glXQueryDrawable: Option<PfnGlXQueryDrawable>,
    glXSwapIntervalEXT: Option<PfnGlXSwapIntervalExt>,
    glXSwapIntervalSGI: Option<PfnGlXSwapIntervalSgi>,
    glXSwapIntervalMESA: Option<PfnGlXSwapIntervalMesa>,
    glXGetSwapIntervalMESA: Option<PfnGlXGetSwapIntervalMesa>,
}

/// The extensions `X11_GL_InitExtensions()` found.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
#[allow(non_snake_case)] // (the extension names)
pub(crate) struct GlxExtensions {
    pub(crate) HAS_GLX_EXT_visual_rating: bool,
    pub(crate) HAS_GLX_EXT_visual_info: bool,
    pub(crate) HAS_GLX_EXT_swap_control_tear: bool,
    pub(crate) HAS_GLX_ARB_context_flush_control: bool,
    pub(crate) HAS_GLX_ARB_create_context_robustness: bool,
    pub(crate) HAS_GLX_ARB_create_context_no_error: bool,
    pub(crate) HAS_GLX_ARB_framebuffer_sRGB: bool,

    /* Max version of OpenGL ES context that can be created if the
      implementation supports GLX_EXT_create_context_es2_profile.
      major = minor = 0 when unsupported.
    */
    pub(crate) es_profile_max_supported_version: (i32, i32),
}

/// Translation of `struct SDL_GLDriverData` (with `gl_config.dll_handle`).
pub(crate) struct GlxData {
    /// libGL: never unloaded (see the module documentation).
    dll_handle: &'static SharedObject,
    error_base: c_int,
    #[allow(dead_code)] // (queried like upstream, which doesn't use it either)
    event_base: c_int,
    ext: GlxExtensions,
    f: GlxFns,
    /// A `SwapIntervalTearBehavior`.
    swap_interval_tear_behavior: AtomicU8,
}

/// The OpenGL state of the X11 device: the backend in use and `gl_data`.
pub(crate) struct X11Gl {
    pub(crate) backend: GlBackend,
    pub(crate) glx: Option<Arc<GlxData>>,
}

/// The state `X11_GL_ErrorHandler()` works with (its file statics).
struct ErrorHandlerState {
    handler: XErrorHandler,
    operation: &'static str,
    error_base: c_int,
    error_code: c_int,
    message: Option<String>,
}

static ERROR_HANDLER: Mutex<ErrorHandlerState> = Mutex::new(ErrorHandlerState {
    handler: Option::None,
    operation: "",
    error_base: 0,
    error_code: 0,
    message: Option::None,
});

fn error_handler_state() -> std::sync::MutexGuard<'static, ErrorHandlerState> {
    ERROR_HANDLER.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `X11_GL_ErrorHandler()`.
unsafe extern "C" fn x11_gl_error_handler(d: *mut Display, e: *mut XErrorEvent) -> c_int {
    // SAFETY: Xlib passes a valid error event.
    let error_code = unsafe { (*e).error_code } as c_int;
    let mut x11_error_locale = [0 as c_char; 256];
    let mut x11_error: Option<String> = Option::None;
    if let Some(x) = super::x11dyn::loaded_symbols() {
        // SAFETY: the display is the erroring one; the buffer's size is passed.
        let rc = unsafe {
            (x.XGetErrorText)(
                d,
                error_code,
                x11_error_locale.as_mut_ptr(),
                x11_error_locale.len() as c_int,
            )
        };
        if rc == Success {
            // SAFETY: XGetErrorText NUL-terminates (the buffer starts zeroed).
            x11_error = Some(
                unsafe { CStr::from_ptr(x11_error_locale.as_ptr()) }
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }

    let mut state = error_handler_state();
    state.error_code = error_code;
    state.message = Some(match x11_error {
        Some(x11_error) => format!("Could not {}: {}", state.operation, x11_error),
        Option::None => format!(
            "Could not {}: {} (Base {})",
            state.operation, error_code, state.error_base
        ),
    });

    0
}

/// Translation of `errorCode != Success`: the handler's error, if it ran.
fn take_handler_error() -> Option<Error> {
    let mut state = error_handler_state();
    if state.error_code != Success {
        Some(Error::new(state.message.take().unwrap_or_default()))
    } else {
        Option::None
    }
}

/// Whether `extension` is in the space-separated `extensions`.
/// Translation of `HasExtension()`.
pub(crate) fn has_extension(extension: &str, extensions: Option<&str>) -> bool {
    let Some(extensions) = extensions else {
        return false;
    };

    // Extension names should not have spaces.
    if extension.contains(' ') || extension.is_empty() {
        return false;
    }

    /* It takes a bit of care to be fool-proof about parsing the
     * OpenGL extensions string. Don't be fooled by sub-strings,
     * etc. */
    gl::extension_in_list(extensions, extension, true)
}

/// The attributes `X11_GL_GetAttributes()` builds for `glXChooseFBConfig`
/// (`for_fb_config`) or `glXChooseVisual`, and the index of the
/// `GLX_X_VISUAL_TYPE_EXT` pair (`*_pvistypeattr`), which comes last so
/// that it can be dropped when the first choice fails ("Some targets fail
/// if you use GLX_X_VISUAL_TYPE_EXT/GLX_DIRECT_COLOR_EXT").
pub(crate) fn x11_gl_get_attributes(
    config: &GlConfig,
    ext: &GlxExtensions,
    for_fb_config: bool,
    transparent: bool,
    srgbhint: Option<&str>,
    use_direct_color_visuals: bool,
) -> (Vec<c_int>, Option<usize>) {
    const MAX_ATTRIBUTES: usize = 64;
    let mut attribs: Vec<c_int> = Vec::with_capacity(MAX_ATTRIBUTES);
    let mut pvistypeattr = Option::None;

    // Setup our GLX attributes according to the gl_config.
    if for_fb_config {
        attribs.push(GLX_RENDER_TYPE);
        if config.floatbuffers != 0 {
            attribs.push(GLX_RGBA_FLOAT_BIT_ARB);
        } else {
            attribs.push(GLX_RGBA_BIT);
        }
    } else {
        attribs.push(GLX_RGBA);
    }
    attribs.extend([GLX_RED_SIZE, config.red_size]);
    attribs.extend([GLX_GREEN_SIZE, config.green_size]);
    attribs.extend([GLX_BLUE_SIZE, config.blue_size]);

    if config.alpha_size != 0 {
        attribs.extend([GLX_ALPHA_SIZE, config.alpha_size]);
    }

    if config.double_buffer != 0 {
        attribs.push(GLX_DOUBLEBUFFER);
        if for_fb_config {
            attribs.push(True);
        }
    }

    attribs.extend([GLX_DEPTH_SIZE, config.depth_size]);

    if config.stencil_size != 0 {
        attribs.extend([GLX_STENCIL_SIZE, config.stencil_size]);
    }

    if config.accum_red_size != 0 {
        attribs.extend([GLX_ACCUM_RED_SIZE, config.accum_red_size]);
    }

    if config.accum_green_size != 0 {
        attribs.extend([GLX_ACCUM_GREEN_SIZE, config.accum_green_size]);
    }

    if config.accum_blue_size != 0 {
        attribs.extend([GLX_ACCUM_BLUE_SIZE, config.accum_blue_size]);
    }

    if config.accum_alpha_size != 0 {
        attribs.extend([GLX_ACCUM_ALPHA_SIZE, config.accum_alpha_size]);
    }

    if config.stereo != 0 {
        attribs.push(GLX_STEREO);
        if for_fb_config {
            attribs.push(True);
        }
    }

    if config.multisamplebuffers != 0 {
        attribs.extend([GLX_SAMPLE_BUFFERS_ARB, config.multisamplebuffers]);
    }

    if config.multisamplesamples != 0 {
        attribs.extend([GLX_SAMPLES_ARB, config.multisamplesamples]);
    }

    if config.floatbuffers != 0 {
        attribs.extend([GLX_RENDER_TYPE, GLX_RGBA_FLOAT_TYPE_ARB]);
    }

    if ext.HAS_GLX_ARB_framebuffer_sRGB {
        match srgbhint.filter(|h| !h.is_empty()) {
            Some("skip") => {
                // don't set an attribute at all.
            }
            Some(srgbhint) => {
                attribs.push(GLX_FRAMEBUFFER_SRGB_CAPABLE_ARB);
                // always needed, for_FBConfig or not!
                attribs.push(if hints::string_to_bool(Some(srgbhint), false) {
                    True
                } else {
                    False
                });
            }
            Option::None => {
                if config.framebuffer_srgb_capable != 0 {
                    // default behavior without the hint.
                    attribs.push(GLX_FRAMEBUFFER_SRGB_CAPABLE_ARB);
                    attribs.push(True); // always needed, for_FBConfig or not!
                }
            }
        }
    }

    if config.accelerated >= 0 && ext.HAS_GLX_EXT_visual_rating {
        attribs.push(GLX_VISUAL_CAVEAT_EXT);
        attribs.push(if config.accelerated != 0 {
            GLX_NONE_EXT
        } else {
            GLX_SLOW_VISUAL_EXT
        });
    }

    // Un-wanted when we request a transparent buffer
    if !transparent {
        /* If we're supposed to use DirectColor visuals, and we've got the
        EXT_visual_info extension, then add GLX_X_VISUAL_TYPE_EXT. */
        if use_direct_color_visuals && ext.HAS_GLX_EXT_visual_info {
            pvistypeattr = Some(attribs.len());
            attribs.extend([GLX_X_VISUAL_TYPE_EXT, GLX_DIRECT_COLOR_EXT]);
        }
    }

    attribs.push(None as c_int);

    crate::sdl_assert!(attribs.len() <= MAX_ATTRIBUTES);

    (attribs, pvistypeattr)
}

/// The attributes of a `glXCreateContextAttribsARB` context (the "GL 3.x"
/// path of `X11_GL_CreateContext()`).
pub(crate) fn x11_gl_context_attribs(config: &GlConfig, ext: &GlxExtensions) -> Vec<c_int> {
    // max 14 attributes plus terminator
    let mut attribs: Vec<c_int> = vec![
        GLX_CONTEXT_MAJOR_VERSION_ARB,
        config.major_version,
        GLX_CONTEXT_MINOR_VERSION_ARB,
        config.minor_version,
    ];

    // SDL profile bits match GLX profile bits
    if config.profile_mask != 0 {
        attribs.extend([GLX_CONTEXT_PROFILE_MASK_ARB, config.profile_mask]);
    }

    // SDL flags match GLX flags
    if config.flags != 0 {
        attribs.extend([GLX_CONTEXT_FLAGS_ARB, config.flags]);
    }

    // only set if glx extension is available and not the default setting
    if ext.HAS_GLX_ARB_context_flush_control && config.release_behavior == 0 {
        attribs.push(GLX_CONTEXT_RELEASE_BEHAVIOR_ARB);
        attribs.push(if config.release_behavior != 0 {
            GLX_CONTEXT_RELEASE_BEHAVIOR_FLUSH_ARB
        } else {
            GLX_CONTEXT_RELEASE_BEHAVIOR_NONE_ARB
        });
    }

    // only set if glx extension is available and not the default setting
    if ext.HAS_GLX_ARB_create_context_robustness && config.reset_notification != 0 {
        attribs.push(GLX_CONTEXT_RESET_NOTIFICATION_STRATEGY_ARB);
        attribs.push(if config.reset_notification != 0 {
            GLX_LOSE_CONTEXT_ON_RESET_ARB
        } else {
            GLX_NO_RESET_NOTIFICATION_ARB
        });
    }

    // only set if glx extension is available and not the default setting
    if ext.HAS_GLX_ARB_create_context_no_error && config.no_error != 0 {
        attribs.extend([GLX_CONTEXT_OPENGL_NO_ERROR_ARB, config.no_error]);
    }

    attribs.push(0);
    attribs
}

/// Whether OpenGL ES must go through EGL: always with
/// [`hints::VIDEO_FORCE_EGL`], otherwise (for an ES profile) unless GLX
/// can create the requested ES version. Translation of `X11_GL_UseEGL()`.
pub(crate) fn x11_gl_use_egl(glx: &GlxData, config: &GlConfig) -> bool {
    if hints::get_bool(hints::VIDEO_FORCE_EGL, false) {
        // use of EGL has been requested, even for desktop GL
        return true;
    }

    crate::sdl_assert!(config.profile_mask == GL_CONTEXT_PROFILE_ES);
    let (max_major, max_minor) = glx.ext.es_profile_max_supported_version;
    hints::get_bool(hints::OPENGL_ES_DRIVER, false)
        || config.major_version == 1 // No GLX extension for OpenGL ES 1.x profiles.
        || config.major_version > max_major
        || (config.major_version == max_major && config.minor_version > max_minor)
}

/// The tear behavior `CheckSwapIntervalTearBehavior()` concludes from
/// `GLX_LATE_SWAPS_TEAR_EXT` with no swap interval (and the sign the
/// original value gets).
fn tear_behavior_from_late_swaps(
    allow_late_swap_tearing: c_uint,
    current_allow_late: c_uint,
    original_val: c_int,
) -> (SwapIntervalTearBehavior, c_int) {
    if allow_late_swap_tearing == 0 {
        // GLX_LATE_SWAPS_TEAR_EXT says whether late swapping is currently in use
        let val = if current_allow_late != 0 {
            -original_val
        } else {
            original_val
        };
        (SwapIntervalTearBehavior::Nvidia, val)
    } else if allow_late_swap_tearing == 1 {
        // GLX_LATE_SWAPS_TEAR_EXT says whether the Drawable can use late swapping at all
        (SwapIntervalTearBehavior::Mesa, original_val)
    } else {
        // unexpected outcome!
        (SwapIntervalTearBehavior::Unknown, original_val)
    }
}

/// The swap interval `X11_GL_GetSwapInterval()` reports for the
/// drawable's `GLX_SWAP_INTERVAL_EXT` and `GLX_LATE_SWAPS_TEAR_EXT`.
fn swap_interval_from_query(
    behavior: SwapIntervalTearBehavior,
    val: c_uint,
    allow_late_swap_tearing: c_uint,
) -> c_int {
    match behavior {
        // unsigned int cast to signed that generates negative value if necessary.
        SwapIntervalTearBehavior::Mesa => val as c_int,
        _ => {
            if allow_late_swap_tearing != 0 && val > 0 {
                -(val as c_int)
            } else {
                val as c_int
            }
        }
    }
}

/// `swapinterval` (for the SGI/MESA functions, which can't be queried).
static SWAPINTERVAL: AtomicI32 = AtomicI32::new(0);

impl X11Video {
    fn gl_state(&self) -> std::sync::MutexGuard<'_, X11Gl> {
        self.gl.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The backend the device's `GL_*` functions currently are.
    pub(crate) fn gl_backend(&self) -> GlBackend {
        self.gl_state().backend
    }

    /// Switch the device's `GL_*` functions.
    pub(crate) fn set_gl_backend(&self, backend: GlBackend) {
        self.gl_state().backend = backend;
    }

    /// `_this->gl_data`.
    pub(crate) fn glx_data(&self) -> Option<Arc<GlxData>> {
        self.gl_state().glx.clone()
    }

    /// Whether a window's GL visual and surface come from EGL: `(profile
    /// is ES || SDL_HINT_VIDEO_FORCE_EGL) && (!_this->gl_data ||
    /// X11_GL_UseEGL(_this))`.
    pub(crate) fn x11_gl_window_uses_egl(&self) -> bool {
        let config = gl_config();
        (config.profile_mask == GL_CONTEXT_PROFILE_ES
            || hints::get_bool(hints::VIDEO_FORCE_EGL, false))
            && self
                .glx_data()
                .is_none_or(|glx| x11_gl_use_egl(&glx, &config))
    }

    /// Translation of `X11_GL_LoadLibrary_EGLFallback()`.
    fn x11_gl_load_library_egl_fallback(&self, path: Option<&str>) -> Result<()> {
        self.x11_gl_unload_library();
        self.set_gl_backend(GlBackend::Egl);
        self.x11_gles_load_library(path)
    }

    /// Translation of `X11_GL_LoadLibrary()`.
    pub(crate) fn x11_gl_load_library(&self, path: Option<&str>) -> Result<()> {
        if self.glx_data().is_some() {
            return Err(Error::new("OpenGL context already created"));
        }

        let config = gl_config();
        if config.profile_mask == GL_CONTEXT_PROFILE_ES
            && hints::get_bool(hints::OPENGL_ES_DRIVER, false)
        {
            return self.x11_gl_load_library_egl_fallback(path);
        }

        // Load the OpenGL library
        let path = path
            .map(str::to_owned)
            .or_else(|| hints::get(hints::OPENGL_LIBRARY))
            .unwrap_or_else(|| DEFAULT_OPENGL.to_owned());
        // (dlopen(path, RTLD_NOW | RTLD_GLOBAL), "Failed loading %s: %s")
        let dll_handle: &'static SharedObject =
            Box::leak(Box::new(SharedObject::load_global(&path)?));
        gl::set_driver_path(Some(&path));

        // Load function pointers
        let mut f = GlxFns {
            // SAFETY: the function of this name has this type.
            glXQueryExtension: unsafe { dll_handle.function("glXQueryExtension") }.ok(),
            // SAFETY: as above.
            glXGetProcAddress: unsafe { dll_handle.function("glXGetProcAddressARB") }.ok(),
            ..GlxFns::default()
        };
        let lookup = |f: &GlxFns, name: &str| x11_gl_get_proc_address_raw(f, dll_handle, name);
        // SAFETY: as above.
        unsafe {
            f.glXChooseVisual = cast_fn(lookup(&f, "glXChooseVisual"));
            f.glXCreateContext = cast_fn(lookup(&f, "glXCreateContext"));
            f.glXDestroyContext = cast_fn(lookup(&f, "glXDestroyContext"));
            f.glXMakeCurrent = cast_fn(lookup(&f, "glXMakeCurrent"));
            f.glXSwapBuffers = cast_fn(lookup(&f, "glXSwapBuffers"));
            f.glXQueryDrawable = cast_fn(lookup(&f, "glXQueryDrawable"));
        }

        let (Some(glx_query_extension), Some(_), Some(_), Some(_), Some(_), Some(_)) = (
            f.glXQueryExtension,
            f.glXChooseVisual,
            f.glXCreateContext,
            f.glXDestroyContext,
            f.glXMakeCurrent,
            f.glXSwapBuffers,
        ) else {
            // (gl_data stays allocated until the front end unloads)
            return Err(Error::new("Could not retrieve OpenGL functions"));
        };

        let display = self.display;
        let mut error_base = 0;
        let mut event_base = 0;
        // SAFETY: the display is open; the out-parameters are valid.
        if unsafe { glx_query_extension(display, &mut error_base, &mut event_base) } == 0 {
            return Err(Error::new("GLX is not supported"));
        }

        self.gl_state().glx = Some(Arc::new(GlxData {
            dll_handle,
            error_base,
            event_base,
            ext: GlxExtensions::default(),
            f,
            swap_interval_tear_behavior: AtomicU8::new(SwapIntervalTearBehavior::Untested as u8),
        }));

        // Initialize extensions
        /* See lengthy comment about the inc/dec in
        ../windows/SDL_windowsopengl.c. */
        gl::set_driver_loaded(|n| n + 1);
        self.x11_gl_init_extensions();
        gl::set_driver_loaded(|n| n - 1);

        /* If we need a GL ES context and there's no
         * GLX_EXT_create_context_es2_profile extension, switch over to X11_GLES functions
         */
        let config = gl_config();
        if (config.profile_mask == GL_CONTEXT_PROFILE_ES
            || hints::get_bool(hints::VIDEO_FORCE_EGL, false))
            && self
                .glx_data()
                .is_some_and(|glx| x11_gl_use_egl(&glx, &config))
        {
            return self.x11_gl_load_library_egl_fallback(Option::None);
        }

        Ok(())
    }

    /// Translation of `X11_GL_GetProcAddress()`.
    pub(crate) fn x11_gl_get_proc_address(&self, proc_name: &str) -> Option<NonNull<c_void>> {
        // (gl_data is set whenever the front end asks)
        let glx = self.glx_data()?;
        x11_gl_get_proc_address_raw(&glx.f, glx.dll_handle, proc_name)
    }

    /// Translation of `X11_GL_UnloadLibrary()`.
    pub(crate) fn x11_gl_unload_library(&self) {
        /* Don't actually unload the library, since it may have registered
         * X11 shutdown hooks, per the notes at:
         * http://dri.sourceforge.net/doc/DRIuserguide.html
         */
        // (the handle is never closed: GlxData has it leaked)

        // Free OpenGL memory
        self.gl_state().glx = Option::None;
    }

    /// Translation of `X11_GL_InitExtensions()`.
    fn x11_gl_init_extensions(&self) {
        let Some(glx) = self.glx_data() else { return };
        let x = &self.x;
        let display = self.display;
        // SAFETY: the display is open.
        let screen = unsafe { DefaultScreen(display) };
        let mut w: Window = 0;
        let mut prev_ctx: GLXContext = std::ptr::null_mut();
        let mut prev_drawable: GLXDrawable = 0;
        let mut context: GLXContext = std::ptr::null_mut();
        let f = &glx.f;
        let get = |name: &str| x11_gl_get_proc_address_raw(f, glx.dll_handle, name);
        let (Some(glx_create_context), Some(glx_make_current), Some(glx_destroy_context)) =
            (f.glXCreateContext, f.glXMakeCurrent, f.glXDestroyContext)
        else {
            return;
        };

        if let Ok(vinfo) = self.x11_gl_get_visual_with(&glx, display, screen, false) {
            let vinfo = vinfo.as_ptr();
            // SAFETY: the functions of these names have these types.
            let (get_current_context, get_current_drawable) = unsafe {
                (
                    cast_fn::<PfnGlXGetCurrentContext>(get("glXGetCurrentContext")),
                    cast_fn::<PfnGlXGetCurrentDrawable>(get("glXGetCurrentDrawable")),
                )
            };

            if let (Some(get_current_context), Some(get_current_drawable)) =
                (get_current_context, get_current_drawable)
            {
                // SAFETY: the display is open; vinfo is a visual info GLX
                // returned (freed below); the attributes are valid.
                unsafe {
                    prev_ctx = get_current_context();
                    prev_drawable = get_current_drawable();

                    let mut xattr: XSetWindowAttributes = std::mem::zeroed();
                    xattr.background_pixel = 0;
                    xattr.border_pixel = 0;
                    // FIXME (upstream): this colormap is never freed.
                    xattr.colormap = (x.XCreateColormap)(
                        display,
                        RootWindow(display, screen),
                        (*vinfo).visual,
                        AllocNone,
                    );
                    w = (x.XCreateWindow)(
                        display,
                        RootWindow(display, screen),
                        0,
                        0,
                        32,
                        32,
                        0,
                        (*vinfo).depth,
                        InputOutput,
                        (*vinfo).visual,
                        CWBackPixel | CWBorderPixel | CWColormap,
                        &mut xattr,
                    );

                    context = glx_create_context(display, vinfo, std::ptr::null_mut(), True);
                    if !context.is_null() {
                        glx_make_current(display, w, context);
                    }
                }
            }

            // SAFETY: vinfo came from Xlib/GLX.
            unsafe { (x.XFree)(vinfo.cast()) };
        }

        // SAFETY: the function of this name has this type.
        let query_extensions_string: Option<PfnGlXQueryExtensionsString> =
            unsafe { cast_fn(get("glXQueryExtensionsString")) };
        let extensions: Option<String> = query_extensions_string.and_then(|q| {
            // SAFETY: the display is open; the result is NULL or a
            // NUL-terminated string owned by GLX.
            unsafe {
                let s = q(display, screen);
                (!s.is_null()).then(|| CStr::from_ptr(s).to_string_lossy().into_owned())
            }
        });
        let extensions = extensions.as_deref();

        let mut ext = GlxExtensions::default();
        let mut f = glx.f;

        // Check for GLX_EXT_swap_control(_tear)
        ext.HAS_GLX_EXT_swap_control_tear = false;
        if has_extension("GLX_EXT_swap_control", extensions) {
            // SAFETY (this and the casts below): the functions of these
            // names have these types.
            f.glXSwapIntervalEXT = unsafe { cast_fn(get("glXSwapIntervalEXT")) };
            if has_extension("GLX_EXT_swap_control_tear", extensions) {
                ext.HAS_GLX_EXT_swap_control_tear = true;
            }
        }

        // Check for GLX_MESA_swap_control
        if has_extension("GLX_MESA_swap_control", extensions) {
            // SAFETY: as above.
            f.glXSwapIntervalMESA = unsafe { cast_fn(get("glXSwapIntervalMESA")) };
            // SAFETY: as above.
            f.glXGetSwapIntervalMESA = unsafe { cast_fn(get("glXGetSwapIntervalMESA")) };
        }

        // Check for GLX_SGI_swap_control
        if has_extension("GLX_SGI_swap_control", extensions) {
            // SAFETY: as above.
            f.glXSwapIntervalSGI = unsafe { cast_fn(get("glXSwapIntervalSGI")) };
        }

        // Check for GLX_ARB_create_context
        if has_extension("GLX_ARB_create_context", extensions) {
            // SAFETY: as above.
            unsafe {
                f.glXCreateContextAttribsARB = cast_fn(get("glXCreateContextAttribsARB"));
                f.glXChooseFBConfig = cast_fn(get("glXChooseFBConfig"));
                f.glXGetVisualFromFBConfig = cast_fn(get("glXGetVisualFromFBConfig"));
            }
        }

        // Check for GLX_EXT_visual_rating
        if has_extension("GLX_EXT_visual_rating", extensions) {
            ext.HAS_GLX_EXT_visual_rating = true;
        }

        // Check for GLX_EXT_visual_info
        if has_extension("GLX_EXT_visual_info", extensions) {
            ext.HAS_GLX_EXT_visual_info = true;
        }

        // Check for GLX_EXT_create_context_es2_profile
        if has_extension("GLX_EXT_create_context_es2_profile", extensions) {
            // this wants to call glGetString(), so it needs a context.
            // !!! FIXME: it would be nice not to make a context here though!
            if !context.is_null() {
                ext.es_profile_max_supported_version = gl::gl_deduce_max_supported_es_profile();
            }
        }

        // Check for GLX_ARB_context_flush_control
        if has_extension("GLX_ARB_context_flush_control", extensions) {
            ext.HAS_GLX_ARB_context_flush_control = true;
        }

        // Check for GLX_ARB_create_context_robustness
        if has_extension("GLX_ARB_create_context_robustness", extensions) {
            ext.HAS_GLX_ARB_create_context_robustness = true;
        }

        // Check for GLX_ARB_create_context_no_error
        if has_extension("GLX_ARB_create_context_no_error", extensions) {
            ext.HAS_GLX_ARB_create_context_no_error = true;
        }

        // Check for GLX_ARB_framebuffer_sRGB
        if has_extension("GLX_ARB_framebuffer_sRGB", extensions)
            || has_extension("GLX_EXT_framebuffer_sRGB", extensions)
        // same thing.
        {
            ext.HAS_GLX_ARB_framebuffer_sRGB = true;
        }

        // SAFETY: the display is open; the context and window are ours.
        unsafe {
            if !context.is_null() {
                glx_make_current(display, None, std::ptr::null_mut());
                glx_destroy_context(display, context);
                if !prev_ctx.is_null() && prev_drawable != 0 {
                    glx_make_current(display, prev_drawable, prev_ctx);
                }
            }

            if w != 0 {
                (x.XDestroyWindow)(display, w);
            }
        }

        // (gl_data gets the extensions)
        let glx = GlxData {
            dll_handle: glx.dll_handle,
            error_base: glx.error_base,
            event_base: glx.event_base,
            ext,
            f,
            swap_interval_tear_behavior: AtomicU8::new(
                glx.swap_interval_tear_behavior.load(Ordering::Relaxed),
            ),
        };
        self.gl_state().glx = Some(Arc::new(glx));

        self.x11_pump_events();
    }

    /// Translation of `X11_GL_GetVisual()`: a visual info to free with
    /// `XFree()`.
    pub(crate) fn x11_gl_get_visual(
        &self,
        display: *mut Display,
        screen: c_int,
        transparent: bool,
    ) -> Result<NonNull<XVisualInfo>> {
        let Some(glx) = self.glx_data() else {
            // The OpenGL library wasn't loaded, SDL_GetError() should have info
            return Err(Error::new("The OpenGL library isn't loaded"));
        };
        self.x11_gl_get_visual_with(&glx, display, screen, transparent)
    }

    fn x11_gl_get_visual_with(
        &self,
        glx: &GlxData,
        display: *mut Display,
        screen: c_int,
        transparent: bool,
    ) -> Result<NonNull<XVisualInfo>> {
        let x = &self.x;
        let config = gl_config();
        let srgbhint = hints::get(hints::OPENGL_FORCE_SRGB_FRAMEBUFFER);
        let direct_color = x11_use_direct_color_visuals();
        let mut vinfo: *mut XVisualInfo = std::ptr::null_mut();

        if let (Some(choose_fb_config), Some(get_visual_from_fb_config)) =
            (glx.f.glXChooseFBConfig, glx.f.glXGetVisualFromFBConfig)
        {
            let mut fbcount: c_int = 0;

            let (mut attribs, pvistypeattr) = x11_gl_get_attributes(
                &config,
                &glx.ext,
                true,
                transparent,
                srgbhint.as_deref(),
                direct_color,
            );
            // SAFETY: the display is open; the list is None-terminated;
            // the configs are freed with XFree below.
            unsafe {
                let mut framebuffer_config =
                    choose_fb_config(display, screen, attribs.as_ptr(), &mut fbcount);
                if framebuffer_config.is_null() {
                    if let Some(i) = pvistypeattr {
                        attribs[i] = None as c_int;
                        framebuffer_config =
                            choose_fb_config(display, screen, attribs.as_ptr(), &mut fbcount);
                    }
                }
                // Note (upstream): a non-NULL list of no configs would be read
                // past its end; fbcount is checked.
                let configs: &[GLXFBConfig] = if framebuffer_config.is_null() {
                    &[]
                } else {
                    std::slice::from_raw_parts(framebuffer_config, fbcount.max(0) as usize)
                };

                if transparent {
                    // Return the first transparent Visual
                    for &fbconfig in configs {
                        vinfo = get_visual_from_fb_config(display, fbconfig);
                        // FIXME (upstream): a config without a visual (NULL)
                        // is dereferenced; it is skipped here.
                        if vinfo.is_null() {
                            continue;
                        }
                        let format = x11_get_pixel_format_from_visual_info(x, display, &*vinfo);
                        if format.has_alpha() {
                            // found!
                            (x.XFree)(framebuffer_config.cast());
                            framebuffer_config = std::ptr::null_mut();
                            break;
                        }
                        (x.XFree)(vinfo.cast());
                        vinfo = std::ptr::null_mut();
                    }
                }

                if !framebuffer_config.is_null() && fbcount > 0 {
                    vinfo = get_visual_from_fb_config(display, *framebuffer_config);
                }

                if !framebuffer_config.is_null() {
                    (x.XFree)(framebuffer_config.cast());
                }
            }
        }

        if vinfo.is_null() {
            if let Some(choose_visual) = glx.f.glXChooseVisual {
                let (mut attribs, pvistypeattr) = x11_gl_get_attributes(
                    &config,
                    &glx.ext,
                    false,
                    transparent,
                    srgbhint.as_deref(),
                    direct_color,
                );
                // SAFETY: the display is open; the list is None-terminated.
                unsafe {
                    vinfo = choose_visual(display, screen, attribs.as_mut_ptr());

                    if vinfo.is_null() {
                        if let Some(i) = pvistypeattr {
                            attribs[i] = None as c_int;
                            vinfo = choose_visual(display, screen, attribs.as_mut_ptr());
                        }
                    }
                }
            }
        }

        if transparent && !vinfo.is_null() {
            // SAFETY: vinfo is a visual info GLX returned.
            let format = x11_get_pixel_format_from_visual_info(x, display, unsafe { &*vinfo });
            if !format.has_alpha() {
                // not transparent!
                if let Some(visualinfo) = self.x11_gl_get_transparent_visual_info(display, screen) {
                    // SAFETY: vinfo came from GLX.
                    unsafe { (x.XFree)(vinfo.cast()) };
                    vinfo = visualinfo.as_ptr();
                }
            }
        }

        NonNull::new(vinfo).ok_or_else(|| Error::new("Couldn't find matching GLX visual"))
    }

    /// The first transparent visual. Translation of
    /// `X11_GL_GetTransparentVisualInfo()`.
    pub(crate) fn x11_gl_get_transparent_visual_info(
        &self,
        display: *mut Display,
        screen: c_int,
    ) -> Option<NonNull<XVisualInfo>> {
        let x = &self.x;
        let mut vi_in = XVisualInfo {
            screen,
            ..XVisualInfo::default()
        };
        let mut out_count: c_int = 0;

        // SAFETY: the display is open; the visual infos are freed with XFree.
        unsafe {
            let mut visualinfo =
                (x.XGetVisualInfo)(display, VisualScreenMask, &mut vi_in, &mut out_count);
            if !visualinfo.is_null() {
                let list = std::slice::from_raw_parts(visualinfo, out_count.max(0) as usize);
                for v in list {
                    let format = x11_get_pixel_format_from_visual_info(x, display, v);
                    if format.has_alpha() {
                        vi_in.screen = screen;
                        vi_in.visualid = v.visualid;
                        (x.XFree)(visualinfo.cast());
                        visualinfo = (x.XGetVisualInfo)(
                            display,
                            VisualScreenMask | VisualIDMask,
                            &mut vi_in,
                            &mut out_count,
                        );
                        break;
                    }
                }
            }
            NonNull::new(visualinfo)
        }
    }

    /// Install `X11_GL_ErrorHandler()` for `operation` (after an `XSync`).
    fn begin_gl_error_trap(&self, glx: &GlxData, operation: &'static str) {
        let display = self.display;
        // We do this to create a clean separation between X and GLX errors.
        // SAFETY: the display is open.
        unsafe { (self.x.XSync)(display, False) };
        {
            let mut state = error_handler_state();
            state.operation = operation;
            state.error_base = glx.error_base;
            state.error_code = Success;
            state.message = Option::None;
        }
        // SAFETY: the handler is a valid extern fn.
        let handler = unsafe { (self.x.XSetErrorHandler)(Some(x11_gl_error_handler)) };
        error_handler_state().handler = handler;
    }

    /// Restore the error handler `begin_gl_error_trap` replaced.
    fn end_gl_error_trap(&self) {
        let handler = error_handler_state().handler;
        // SAFETY: the previous handler.
        unsafe { (self.x.XSetErrorHandler)(handler) };
    }

    /// Translation of `X11_GL_CreateContext()`.
    pub(crate) fn x11_gl_create_context(&self, window: WindowID) -> Result<RawGlContext> {
        let Some(glx) = self.glx_data() else {
            return Err(Error::new("OpenGL not initialized"));
        };
        let x = &self.x;
        let display = self.display;
        let config = gl_config();
        let xwindow = with_x11_window(window, |_, d| d.xwindow)?;
        let screen = display_driver_data_for_window(window).map_or(0, |d| d.screen);
        let transparent = with_window(window, |w| w.flags().contains(WindowFlags::TRANSPARENT))?;
        let mut context: GLXContext = std::ptr::null_mut();

        let share_context: GLXContext = if config.share_with_current_context != 0 {
            gl::current_thread_context().map_or(std::ptr::null_mut(), |c| c.as_ptr())
        } else {
            std::ptr::null_mut()
        };

        self.begin_gl_error_trap(&glx, "create GL context");
        // SAFETY: the display is open and the window ours; the visual infos
        // and configs are freed with XFree; the attribute lists are
        // terminated.
        unsafe {
            let mut xattr: XWindowAttributes = std::mem::zeroed();
            (x.XGetWindowAttributes)(display, xwindow, &mut xattr);
            let mut v = XVisualInfo {
                screen,
                visualid: (x.XVisualIDFromVisual)(xattr.visual),
                ..XVisualInfo::default()
            };
            let mut n: c_int = 0;
            let vinfo =
                (x.XGetVisualInfo)(display, VisualScreenMask | VisualIDMask, &mut v, &mut n);
            if !vinfo.is_null() {
                if config.major_version < 3
                    && config.profile_mask == 0
                    && config.flags == 0
                    && !transparent
                {
                    // Create legacy context
                    if let Some(create) = glx.f.glXCreateContext {
                        context = create(display, vinfo, share_context, True);
                    }
                } else {
                    let attribs = x11_gl_context_attribs(&config, &glx.ext);

                    // Get a pointer to the context creation function for GL 3.0
                    match glx.f.glXCreateContextAttribsARB {
                        Option::None => {
                            // FIXME (upstream): this error is overwritten by
                            // "Could not create GL context" below.
                        }
                        Some(create_context_attribs) => {
                            // Create a GL 3.x context
                            let (mut glx_attribs, pvistypeattr) = x11_gl_get_attributes(
                                &config,
                                &glx.ext,
                                true,
                                transparent,
                                hints::get(hints::OPENGL_FORCE_SRGB_FRAMEBUFFER).as_deref(),
                                x11_use_direct_color_visuals(),
                            );

                            if let Some(choose_fb_config) = glx.f.glXChooseFBConfig {
                                let mut fbcount: c_int = 0;
                                // FIXME (upstream): the configs are chosen on
                                // the default screen, not the window's.
                                let mut framebuffer_config = choose_fb_config(
                                    display,
                                    DefaultScreen(display),
                                    glx_attribs.as_ptr(),
                                    &mut fbcount,
                                );

                                if framebuffer_config.is_null() {
                                    if let Some(i) = pvistypeattr {
                                        glx_attribs[i] = None as c_int;
                                        framebuffer_config = choose_fb_config(
                                            display,
                                            DefaultScreen(display),
                                            glx_attribs.as_ptr(),
                                            &mut fbcount,
                                        );
                                    }
                                }

                                if transparent && !framebuffer_config.is_null() {
                                    let configs = std::slice::from_raw_parts(
                                        framebuffer_config,
                                        fbcount.max(0) as usize,
                                    );
                                    for &fbconfig in configs {
                                        let vinfo_temp = glx
                                            .f
                                            .glXGetVisualFromFBConfig
                                            .map_or(std::ptr::null_mut(), |g| g(display, fbconfig));
                                        if !vinfo_temp.is_null() {
                                            let format = x11_get_pixel_format_from_visual_info(
                                                x,
                                                display,
                                                &*vinfo_temp,
                                            );
                                            if format.has_alpha() {
                                                // found!
                                                context = create_context_attribs(
                                                    display,
                                                    fbconfig,
                                                    share_context,
                                                    True,
                                                    attribs.as_ptr(),
                                                );
                                                (x.XFree)(framebuffer_config.cast());
                                                framebuffer_config = std::ptr::null_mut();
                                                (x.XFree)(vinfo_temp.cast());
                                                break;
                                            }
                                            (x.XFree)(vinfo_temp.cast());
                                        }
                                    }
                                }
                                // Note (upstream): fbcount is checked before
                                // the first config is read.
                                if !framebuffer_config.is_null() {
                                    if fbcount > 0 {
                                        context = create_context_attribs(
                                            display,
                                            *framebuffer_config,
                                            share_context,
                                            True,
                                            attribs.as_ptr(),
                                        );
                                    }
                                    (x.XFree)(framebuffer_config.cast());
                                }
                            }
                        }
                    }
                }
                (x.XFree)(vinfo.cast());
            }
            (x.XSync)(display, False);
        }
        self.end_gl_error_trap();

        let Some(context) = RawGlContext::from_ptr(context) else {
            return Err(
                take_handler_error().unwrap_or_else(|| Error::new("Could not create GL context"))
            );
        };

        if let Err(e) = self.x11_gl_make_current(Some(window), Some(context)) {
            self.x11_gl_destroy_context(context);
            return Err(e);
        }

        Ok(context)
    }

    /// Translation of `X11_GL_MakeCurrent()`.
    pub(crate) fn x11_gl_make_current(
        &self,
        window: Option<WindowID>,
        context: Option<RawGlContext>,
    ) -> Result<()> {
        let display = self.display;
        // FIXME (upstream): a context made current without a window (which
        // EGL may have allowed earlier) dereferences the NULL window; no
        // drawable is used here.
        let drawable: Window = match (context, window) {
            (Some(_), Some(window)) => with_x11_window(window, |_, d| d.xwindow)?,
            _ => None,
        };
        let glx_context: GLXContext = context.map_or(std::ptr::null_mut(), |c| c.as_ptr());

        let Some(glx) = self.glx_data() else {
            return Err(Error::new("OpenGL not initialized"));
        };
        let Some(glx_make_current) = glx.f.glXMakeCurrent else {
            return Err(Error::new("OpenGL not initialized"));
        };

        self.begin_gl_error_trap(&glx, "make GL context current");
        // SAFETY: the display is open; the drawable and context are ours.
        let rc = unsafe { glx_make_current(display, drawable, glx_context) };
        self.end_gl_error_trap();

        if let Some(e) = take_handler_error() {
            // uhoh, an X error was thrown!
            return Err(e); // the error handler called SDL_SetError() already.
        } else if rc == 0 {
            // glXMakeCurrent() failed without throwing an X error
            return Err(Error::new("Unable to make GL context current"));
        }

        Ok(())
    }

    /// The X window of the thread's current GL window (upstream's
    /// `SDL_GL_GetCurrentWindow()->internal->xwindow`).
    fn current_gl_drawable(&self) -> Result<Window> {
        // FIXME (upstream): with no current window this dereferences NULL;
        // it fails here.
        let window = gl::current_thread_window().ok_or_else(|| Error::new("Invalid window"))?;
        with_x11_window(window, |_, d| d.xwindow)
    }

    /*
       0 is a valid argument to glXSwapInterval(MESA|EXT) and setting it to 0
       will undo the effect of a previous call with a value that is greater
       than zero (or at least that is what the docs say). OTOH, 0 is an invalid
       argument to glXSwapIntervalSGI and it returns an error if you call it
       with 0 as an argument.
    */

    /// Translation of `X11_GL_SetSwapInterval()`.
    pub(crate) fn x11_gl_set_swap_interval(&self, interval: i32) -> Result<()> {
        let Some(glx) = self.glx_data() else {
            return Err(Error::new("OpenGL not initialized"));
        };

        if interval < 0 && !glx.ext.HAS_GLX_EXT_swap_control_tear {
            Err(Error::new("Negative swap interval unsupported in this GL"))
        } else if let Some(swap_interval_ext) = glx.f.glXSwapIntervalEXT {
            let display = self.display;
            let drawable = self.current_gl_drawable()?;

            /*
             * This is a workaround for a bug in NVIDIA drivers. Bug has been reported
             * and will be fixed in a future release (probably 319.xx).
             *
             * There's a bug where glXSetSwapIntervalEXT ignores updates because
             * it has the wrong value cached. To work around it, we just run a no-op
             * update to the current value.
             */
            let current_interval = self.x11_gl_get_swap_interval().unwrap_or(0);
            // SAFETY: the display is open; the drawable is the current window's.
            unsafe {
                swap_interval_ext(display, drawable, current_interval);
                swap_interval_ext(display, drawable, interval);
            }
            SWAPINTERVAL.store(interval, Ordering::Relaxed);
            Ok(())
        } else if let Some(swap_interval_mesa) = glx.f.glXSwapIntervalMESA {
            // SAFETY: a context is current.
            let rc = unsafe { swap_interval_mesa(interval) };
            if rc == 0 {
                SWAPINTERVAL.store(interval, Ordering::Relaxed);
                Ok(())
            } else {
                Err(Error::new("glXSwapIntervalMESA failed"))
            }
        } else if let Some(swap_interval_sgi) = glx.f.glXSwapIntervalSGI {
            // SAFETY: a context is current.
            let rc = unsafe { swap_interval_sgi(interval) };
            if rc == 0 {
                SWAPINTERVAL.store(interval, Ordering::Relaxed);
                Ok(())
            } else {
                Err(Error::new("glXSwapIntervalSGI failed"))
            }
        } else {
            Err(Error::unsupported())
        }
    }

    /// Translation of `CheckSwapIntervalTearBehavior()`.
    fn check_swap_interval_tear_behavior(
        &self,
        glx: &GlxData,
        drawable: Window,
        current_val: c_uint,
        current_allow_late: c_uint,
    ) -> SwapIntervalTearBehavior {
        /* Mesa and Nvidia interpret GLX_EXT_swap_control_tear differently, as of this writing, so
        figure out which behavior we have.
        Technical details: https://github.com/libsdl-org/SDL/issues/8004#issuecomment-1819603282 */
        let behavior = glx.swap_interval_tear_behavior.load(Ordering::Relaxed);
        if behavior == SwapIntervalTearBehavior::Untested as u8 {
            let new_behavior = match (
                glx.ext.HAS_GLX_EXT_swap_control_tear,
                glx.f.glXSwapIntervalEXT,
                glx.f.glXQueryDrawable,
            ) {
                (true, Some(swap_interval_ext), Some(query_drawable)) => {
                    let display = self.display;
                    let mut allow_late_swap_tearing: c_uint = 22;
                    let original_val = current_val as c_int;

                    // SAFETY: the display is open; the drawable is the
                    // current window's; the out-parameter is valid.
                    unsafe {
                        /*
                         * This is a workaround for a bug in NVIDIA drivers. Bug has been reported
                         * and will be fixed in a future release (probably 319.xx).
                         *
                         * There's a bug where glXSetSwapIntervalEXT ignores updates because
                         * it has the wrong value cached. To work around it, we just run a no-op
                         * update to the current value.
                         */
                        swap_interval_ext(display, drawable, current_val as c_int);

                        // set it to no swap interval and see how it affects GLX_LATE_SWAPS_TEAR_EXT...
                        swap_interval_ext(display, drawable, 0);
                        query_drawable(
                            display,
                            drawable,
                            GLX_LATE_SWAPS_TEAR_EXT,
                            &mut allow_late_swap_tearing,
                        );
                    }

                    let (behavior, original_val) = tear_behavior_from_late_swaps(
                        allow_late_swap_tearing,
                        current_allow_late,
                        original_val,
                    );

                    // set us back to what it was originally...
                    // SAFETY: as above.
                    unsafe { swap_interval_ext(display, drawable, original_val) };
                    behavior
                }
                _ => SwapIntervalTearBehavior::Unknown,
            };
            glx.swap_interval_tear_behavior
                .store(new_behavior as u8, Ordering::Relaxed);
            return new_behavior;
        }

        match behavior {
            1 => SwapIntervalTearBehavior::Unknown,
            2 => SwapIntervalTearBehavior::Mesa,
            3 => SwapIntervalTearBehavior::Nvidia,
            _ => SwapIntervalTearBehavior::Untested,
        }
    }

    /// Translation of `X11_GL_GetSwapInterval()`.
    pub(crate) fn x11_gl_get_swap_interval(&self) -> Result<i32> {
        let Some(glx) = self.glx_data() else {
            return Err(Error::new("OpenGL not initialized"));
        };
        if let Some(_swap_interval_ext) = glx.f.glXSwapIntervalEXT {
            let display = self.display;
            let drawable = self.current_gl_drawable()?;
            let mut allow_late_swap_tearing: c_uint = 0;
            let mut val: c_uint = 0;

            if let Some(query_drawable) = glx.f.glXQueryDrawable {
                // SAFETY: the display is open; the drawable is the current
                // window's; the out-parameters are valid.
                unsafe {
                    if glx.ext.HAS_GLX_EXT_swap_control_tear {
                        allow_late_swap_tearing = 22; // set this to nonsense.
                        query_drawable(
                            display,
                            drawable,
                            GLX_LATE_SWAPS_TEAR_EXT,
                            &mut allow_late_swap_tearing,
                        );
                    }

                    query_drawable(display, drawable, GLX_SWAP_INTERVAL_EXT, &mut val);
                }
            }

            let behavior = self.check_swap_interval_tear_behavior(
                &glx,
                drawable,
                val,
                allow_late_swap_tearing,
            );
            Ok(swap_interval_from_query(
                behavior,
                val,
                allow_late_swap_tearing,
            ))
        } else if let Some(get_swap_interval_mesa) = glx.f.glXGetSwapIntervalMESA {
            // SAFETY: a context is current.
            let val = unsafe { get_swap_interval_mesa() };
            if val == GLX_BAD_CONTEXT {
                return Err(Error::new("GLX_BAD_CONTEXT"));
            }
            Ok(val)
        } else {
            Ok(SWAPINTERVAL.load(Ordering::Relaxed))
        }
    }

    /// Translation of `X11_GL_SwapWindow()`.
    pub(crate) fn x11_gl_swap_window(&self, window: WindowID) -> Result<()> {
        let Some(glx) = self.glx_data() else {
            return Err(Error::new("OpenGL not initialized"));
        };
        let xwindow = with_x11_window(window, |_, d| d.xwindow)?;

        if let Some(swap_buffers) = glx.f.glXSwapBuffers {
            // SAFETY: the display is open and the window ours.
            unsafe { swap_buffers(self.display, xwindow) };
        }

        self.x11_handle_present(window);

        Ok(())
    }

    /// Translation of `X11_GL_DestroyContext()`.
    pub(crate) fn x11_gl_destroy_context(&self, context: RawGlContext) {
        let display = self.display;
        let Some(glx) = self.glx_data() else {
            return;
        };
        if let Some(destroy) = glx.f.glXDestroyContext {
            // SAFETY: the context was created on this display.
            unsafe {
                destroy(display, context.as_ptr());
                (self.x.XSync)(display, False);
            }
        }
    }
}

/// `X11_GL_GetProcAddress()` on given functions and library:
/// `glXGetProcAddress` if there is one, the library's symbol otherwise.
fn x11_gl_get_proc_address_raw(
    f: &GlxFns,
    dll_handle: &SharedObject,
    proc_name: &str,
) -> Option<NonNull<c_void>> {
    if let Some(get_proc_address) = f.glXGetProcAddress {
        let name = CString::new(proc_name).ok()?;
        // SAFETY: the name is NUL-terminated.
        return NonNull::new(unsafe { get_proc_address(name.as_ptr().cast()) });
    }
    dll_handle.symbol(proc_name).ok()
}

/// Cast a looked-up function to its type.
///
/// # Safety
///
/// `F` must be the function's pointer type.
unsafe fn cast_fn<F: Copy>(p: Option<NonNull<c_void>>) -> Option<F> {
    const { assert!(size_of::<F>() == size_of::<*mut c_void>()) };
    // SAFETY: the caller's contract.
    p.map(|p| unsafe { std::mem::transmute_copy::<*mut c_void, F>(&p.as_ptr()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> GlConfig {
        GlConfig {
            red_size: 8,
            green_size: 8,
            blue_size: 8,
            alpha_size: 8,
            depth_size: 16,
            double_buffer: 1,
            accelerated: -1,
            major_version: 2,
            minor_version: 1,
            retained_backing: 1,
            release_behavior: 1,
            ..GlConfig::default()
        }
    }

    #[test]
    fn has_extension_like_upstream() {
        assert!(!has_extension("GLX_ARB_create_context", Option::None));
        assert!(!has_extension("", Some("GLX_a")));
        assert!(!has_extension("GLX a", Some("GLX a")));
        assert!(has_extension(
            "GLX_ARB_create_context",
            Some("GLX_ARB_create_context_profile GLX_ARB_create_context")
        ));
        assert!(!has_extension(
            "GLX_ARB_create_context",
            Some("GLX_ARB_create_context_profile")
        ));
    }

    /// Expected values from the C reference (gl-ref/harness.c, which runs
    /// upstream's X11_GL_GetAttributes() on the same configs).
    #[test]
    fn glx_attributes_like_upstream() {
        let mut c = default_config();
        let mut e = GlxExtensions::default();
        let attrs = |c: &GlConfig, e: &GlxExtensions, fb, transparent, hint, direct| {
            x11_gl_get_attributes(c, e, fb, transparent, hint, direct)
        };
        assert_eq!(
            attrs(&c, &e, true, false, Option::None, true),
            (
                vec![32785, 1, 8, 8, 9, 8, 10, 8, 11, 8, 5, 1, 12, 16, 0],
                Option::None
            )
        );
        assert_eq!(
            attrs(&c, &e, false, false, Option::None, true).0,
            [4, 8, 8, 9, 8, 10, 8, 11, 8, 5, 12, 16, 0]
        );
        e.HAS_GLX_EXT_visual_rating = true;
        e.HAS_GLX_EXT_visual_info = true;
        e.HAS_GLX_ARB_framebuffer_sRGB = true;
        assert_eq!(
            attrs(&c, &e, true, false, Option::None, true),
            (
                vec![32785, 1, 8, 8, 9, 8, 10, 8, 11, 8, 5, 1, 12, 16, 34, 32771, 0],
                Some(14)
            )
        );
        c.accelerated = 0;
        c.stereo = 1;
        c.stencil_size = 8;
        c.accum_red_size = 1;
        c.accum_green_size = 2;
        c.accum_blue_size = 3;
        c.accum_alpha_size = 4;
        c.multisamplebuffers = 1;
        c.multisamplesamples = 4;
        c.floatbuffers = 1;
        c.framebuffer_srgb_capable = 1;
        c.alpha_size = 0;
        c.double_buffer = 0;
        assert_eq!(
            attrs(&c, &e, true, false, Option::None, true),
            (
                vec![
                    32785, 4, 8, 8, 9, 8, 10, 8, 12, 16, 13, 8, 14, 1, 15, 2, 16, 3, 17, 4, 6, 1,
                    100000, 1, 100001, 4, 32785, 8377, 8370, 1, 32, 32769, 34, 32771, 0
                ],
                Some(32)
            )
        );
        assert_eq!(
            attrs(&c, &e, false, true, Some("0"), true),
            (
                vec![
                    4, 8, 8, 9, 8, 10, 8, 12, 16, 13, 8, 14, 1, 15, 2, 16, 3, 17, 4, 6, 100000, 1,
                    100001, 4, 32785, 8377, 8370, 0, 32, 32769, 0
                ],
                Option::None
            )
        );
        assert_eq!(
            attrs(&c, &e, false, false, Some("skip"), false).0,
            [
                4, 8, 8, 9, 8, 10, 8, 12, 16, 13, 8, 14, 1, 15, 2, 16, 3, 17, 4, 6, 100000, 1,
                100001, 4, 32785, 8377, 32, 32769, 0
            ]
        );
        c.accelerated = 1;
        assert_eq!(
            attrs(&c, &e, true, false, Some("1"), true).0,
            [
                32785, 4, 8, 8, 9, 8, 10, 8, 12, 16, 13, 8, 14, 1, 15, 2, 16, 3, 17, 4, 6, 1,
                100000, 1, 100001, 4, 32785, 8377, 8370, 1, 32, 32768, 34, 32771, 0
            ]
        );
    }

    /// Expected values from the C reference (gl-ref/harness.c).
    #[test]
    fn glx_context_attributes_like_upstream() {
        let mut c = default_config();
        let mut e = GlxExtensions::default();
        assert_eq!(x11_gl_context_attribs(&c, &e), [8337, 2, 8338, 1, 0]);
        c.major_version = 3;
        c.minor_version = 3;
        c.profile_mask = 1;
        c.flags = 3;
        c.release_behavior = 0;
        c.reset_notification = 1;
        c.no_error = 1;
        assert_eq!(
            x11_gl_context_attribs(&c, &e),
            [8337, 3, 8338, 3, 37158, 1, 8340, 3, 0]
        );
        e.HAS_GLX_ARB_context_flush_control = true;
        e.HAS_GLX_ARB_create_context_robustness = true;
        e.HAS_GLX_ARB_create_context_no_error = true;
        assert_eq!(
            x11_gl_context_attribs(&c, &e),
            [8337, 3, 8338, 3, 37158, 1, 8340, 3, 8343, 0, 33366, 33362, 12723, 1, 0]
        );
    }

    #[test]
    fn swap_interval_tear_logic() {
        use SwapIntervalTearBehavior::*;
        assert_eq!(tear_behavior_from_late_swaps(0, 1, 2), (Nvidia, -2));
        assert_eq!(tear_behavior_from_late_swaps(0, 0, 2), (Nvidia, 2));
        assert_eq!(tear_behavior_from_late_swaps(1, 1, 2), (Mesa, 2));
        assert_eq!(tear_behavior_from_late_swaps(22, 1, 2), (Unknown, 2));
        assert_eq!(swap_interval_from_query(Mesa, u32::MAX, 1), -1);
        assert_eq!(swap_interval_from_query(Nvidia, 1, 1), -1);
        assert_eq!(swap_interval_from_query(Nvidia, 1, 0), 1);
        assert_eq!(swap_interval_from_query(Unknown, 0, 1), 0);
    }

    #[test]
    fn use_egl_decision() {
        let _l = crate::test_support::test_lock();
        let glx_with = |major, minor| GlxData {
            // (a handle that is never used: the process itself)
            dll_handle: Box::leak(Box::new(SharedObject::load("libc.so.6").unwrap())),
            error_base: 0,
            event_base: 0,
            ext: GlxExtensions {
                es_profile_max_supported_version: (major, minor),
                ..GlxExtensions::default()
            },
            f: GlxFns::default(),
            swap_interval_tear_behavior: AtomicU8::new(0),
        };
        let es = |major, minor| GlConfig {
            profile_mask: GL_CONTEXT_PROFILE_ES,
            major_version: major,
            minor_version: minor,
            ..GlConfig::default()
        };
        let glx = glx_with(3, 2);
        assert!(!x11_gl_use_egl(&glx, &es(3, 2)));
        assert!(!x11_gl_use_egl(&glx, &es(2, 0)));
        assert!(x11_gl_use_egl(&glx, &es(3, 3)));
        assert!(x11_gl_use_egl(&glx, &es(1, 1)));
        assert!(x11_gl_use_egl(&glx_with(0, 0), &es(2, 0)));
        crate::hints::set(crate::hints::VIDEO_FORCE_EGL, "1").unwrap();
        assert!(x11_gl_use_egl(&glx, &default_config()));
        crate::hints::reset_all();
    }
}
