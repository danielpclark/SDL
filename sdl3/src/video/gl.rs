// Rust translation of the OpenGL and EGL parts of src/video/SDL_video.c
// (and of include/SDL3/SDL_video.h and SDL_egl.h) from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The OpenGL front end: library loading, context attributes, contexts,
//! swap intervals, and the EGL accessors.
//!
//! This is upstream's default desktop build (`SDL_VIDEO_OPENGL`, plus
//! `SDL_VIDEO_OPENGL_EGL` on Unix): the X11 driver provides GLX and EGL
//! contexts, with every library (libGL, libEGL, libGLESv2, ...) loaded at
//! run time. The Windows driver's WGL and EGL contexts aren't translated
//! yet (it behaves as a build without OpenGL), nor are the EGL contexts of
//! the offscreen driver (`SDL_offscreenopengles.c`).
//!
//! A [`GlContext`] is destroyed when dropped; functions are looked up with
//! [`gl_get_proc_address`] once a library is loaded (by creating an
//! [`OPENGL`](WindowFlags::OPENGL) window or with [`gl_load_library`]).

use std::cell::Cell;
use std::ffi::{c_char, c_void, CStr};
use std::marker::PhantomData;
use std::num::NonZeroUsize;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::events::WindowID;
use crate::hints;

use super::core::{self, dll_not_supported, driver, uninitialized_video, with_device, with_window};
use super::window::Window;

// ---------------------------------------------------------------------------
// Public types of SDL_video.h
// ---------------------------------------------------------------------------

/// The driver's handle of an OpenGL context (a `GLXContext`, `HGLRC` or
/// `EGLContext`). Translation of `SDL_GLContext` as a plain value; the
/// owning type is [`GlContext`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RawGlContext(NonZeroUsize);

impl RawGlContext {
    /// A handle from a native pointer (`None` for NULL).
    pub fn from_ptr(ptr: *mut c_void) -> Option<RawGlContext> {
        NonZeroUsize::new(ptr as usize).map(RawGlContext)
    }

    /// The native handle.
    pub fn as_ptr(self) -> *mut c_void {
        self.0.get() as *mut c_void
    }
}

/// An OpenGL context, destroyed when dropped. Translation of
/// `SDL_GLContext` (created by `SDL_GL_CreateContext()`, destroyed by
/// `SDL_GL_DestroyContext()`).
///
/// A context may be moved to another thread and made current there, but
/// not shared between threads.
#[derive(Debug)]
pub struct GlContext {
    raw: RawGlContext,
    /// The video device the context belongs to (see [`GL_GENERATION`]).
    generation: u64,
    _not_sync: PhantomData<Cell<()>>,
}

impl GlContext {
    /// Create an OpenGL context for an OpenGL window, and make it current.
    /// Translation of `SDL_GL_CreateContext()`.
    pub fn new(window: &Window) -> Result<GlContext> {
        gl_create_context(window)
    }

    /// The driver's handle.
    pub fn raw(&self) -> RawGlContext {
        self.raw
    }

    /// Make this context current for `window` (`None`: without a window,
    /// where the platform supports it). Translation of `SDL_GL_MakeCurrent()`.
    pub fn make_current(&self, window: Option<&Window>) -> Result<()> {
        gl_make_current(window, Some(self))
    }

    /// Whether this is the calling thread's current context.
    pub fn is_current(&self) -> bool {
        current_thread_context() == Some(self.raw)
    }

    /// Destroy the context, reporting failure (dropping it ignores errors).
    /// Translation of `SDL_GL_DestroyContext()`.
    pub fn destroy(self) -> Result<()> {
        let this = std::mem::ManuallyDrop::new(self);
        destroy_raw(this.raw, this.generation)
    }
}

impl Drop for GlContext {
    fn drop(&mut self) {
        let _ = destroy_raw(self.raw, self.generation);
    }
}

/// An OpenGL configuration attribute. Translation of `SDL_GLAttr`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GlAttr {
    /// the minimum number of bits for the red channel of the color buffer; defaults to 8.
    RedSize,
    /// the minimum number of bits for the green channel of the color buffer; defaults to 8.
    GreenSize,
    /// the minimum number of bits for the blue channel of the color buffer; defaults to 8.
    BlueSize,
    /// the minimum number of bits for the alpha channel of the color buffer; defaults to 8.
    AlphaSize,
    /// the minimum number of bits for frame buffer size; defaults to 0.
    BufferSize,
    /// whether the output is single or double buffered; defaults to double buffering on.
    DoubleBuffer,
    /// the minimum number of bits in the depth buffer; defaults to 16.
    DepthSize,
    /// the minimum number of bits in the stencil buffer; defaults to 0.
    StencilSize,
    /// the minimum number of bits for the red channel of the accumulation buffer; defaults to 0.
    AccumRedSize,
    /// the minimum number of bits for the green channel of the accumulation buffer; defaults to 0.
    AccumGreenSize,
    /// the minimum number of bits for the blue channel of the accumulation buffer; defaults to 0.
    AccumBlueSize,
    /// the minimum number of bits for the alpha channel of the accumulation buffer; defaults to 0.
    AccumAlphaSize,
    /// whether the output is stereo 3D; defaults to off.
    Stereo,
    /// the number of buffers used for multisample anti-aliasing; defaults to 0.
    MultisampleBuffers,
    /// the number of samples used around the current pixel used for multisample anti-aliasing.
    MultisampleSamples,
    /// set to 1 to require hardware acceleration, set to 0 to force software rendering; defaults to allow either.
    AcceleratedVisual,
    /// not used (deprecated).
    RetainedBacking,
    /// OpenGL context major version.
    ContextMajorVersion,
    /// OpenGL context minor version.
    ContextMinorVersion,
    /// some combination of 0 or more of the `GL_CONTEXT_*_FLAG`s; defaults to 0.
    ContextFlags,
    /// type of GL context (Core, Compatibility, ES); default value depends on platform.
    ContextProfileMask,
    /// OpenGL context sharing; defaults to 0.
    ShareWithCurrentContext,
    /// requests sRGB capable visual; defaults to 0.
    FramebufferSrgbCapable,
    /// sets context the release behavior; defaults to FLUSH.
    ContextReleaseBehavior,
    /// set context reset notification; defaults to NO_NOTIFICATION.
    ContextResetNotification,
    /// whether to create a context without error checking (`GL_KHR_no_error`); defaults to 0.
    ContextNoError,
    /// requests a floating point color buffer; defaults to 0.
    FloatBuffers,
    /// the EGL platform (`EGL_PLATFORM_*`) for `eglGetPlatformDisplay`; defaults to the driver's.
    EglPlatform,
}

/// OpenGL Core Profile context. Translation of `SDL_GL_CONTEXT_PROFILE_CORE`.
pub const GL_CONTEXT_PROFILE_CORE: i32 = 0x0001;
/// OpenGL Compatibility Profile context. Translation of `SDL_GL_CONTEXT_PROFILE_COMPATIBILITY`.
pub const GL_CONTEXT_PROFILE_COMPATIBILITY: i32 = 0x0002;
/// GLX_CONTEXT_ES2_PROFILE_BIT_EXT. Translation of `SDL_GL_CONTEXT_PROFILE_ES`.
pub const GL_CONTEXT_PROFILE_ES: i32 = 0x0004;
/// Translation of `SDL_GL_CONTEXT_DEBUG_FLAG`.
pub const GL_CONTEXT_DEBUG_FLAG: i32 = 0x0001;
/// Translation of `SDL_GL_CONTEXT_FORWARD_COMPATIBLE_FLAG`.
pub const GL_CONTEXT_FORWARD_COMPATIBLE_FLAG: i32 = 0x0002;
/// Translation of `SDL_GL_CONTEXT_ROBUST_ACCESS_FLAG`.
pub const GL_CONTEXT_ROBUST_ACCESS_FLAG: i32 = 0x0004;
/// Translation of `SDL_GL_CONTEXT_RESET_ISOLATION_FLAG`.
pub const GL_CONTEXT_RESET_ISOLATION_FLAG: i32 = 0x0008;
/// Translation of `SDL_GL_CONTEXT_RELEASE_BEHAVIOR_NONE`.
pub const GL_CONTEXT_RELEASE_BEHAVIOR_NONE: i32 = 0x0000;
/// Translation of `SDL_GL_CONTEXT_RELEASE_BEHAVIOR_FLUSH`.
pub const GL_CONTEXT_RELEASE_BEHAVIOR_FLUSH: i32 = 0x0001;
/// Translation of `SDL_GL_CONTEXT_RESET_NO_NOTIFICATION`.
pub const GL_CONTEXT_RESET_NO_NOTIFICATION: i32 = 0x0000;
/// Translation of `SDL_GL_CONTEXT_RESET_LOSE_CONTEXT`.
pub const GL_CONTEXT_RESET_LOSE_CONTEXT: i32 = 0x0001;

// ---------------------------------------------------------------------------
// SDL_egl.h
// ---------------------------------------------------------------------------

/// An EGL display (`EGLDisplay`). Translation of `SDL_EGLDisplay`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct EglDisplay(NonZeroUsize);

/// An EGL configuration (`EGLConfig`). Translation of `SDL_EGLConfig`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct EglConfig(NonZeroUsize);

/// An EGL surface (`EGLSurface`). Translation of `SDL_EGLSurface`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct EglSurface(NonZeroUsize);

macro_rules! egl_handle {
    ($($t:ident),*) => {$(
        impl $t {
            /// A handle from a native pointer (`None` for NULL).
            pub fn from_ptr(ptr: *mut c_void) -> Option<$t> {
                NonZeroUsize::new(ptr as usize).map($t)
            }

            /// The native handle.
            pub fn as_ptr(self) -> *mut c_void {
                self.0.get() as *mut c_void
            }
        }
    )*};
}
egl_handle!(EglDisplay, EglConfig, EglSurface);

/// An EGL attribute (`EGLAttrib`). Translation of `SDL_EGLAttrib`.
pub type EglAttrib = isize;
/// An EGL integer (`EGLint`). Translation of `SDL_EGLint`.
pub type EglInt = i32;

/// Supplies the attributes for `eglGetPlatformDisplay` (`None` fails the
/// library load). Translation of `SDL_EGLAttribArrayCallback`; the list
/// ends at `EGL_NONE` (or with the vector).
pub type EglAttribArrayCallback = Box<dyn Fn() -> Option<Vec<EglAttrib>> + Send + Sync>;

/// Supplies attributes for `eglCreateWindowSurface` or `eglCreateContext`
/// (`None` fails the creation). Translation of `SDL_EGLIntArrayCallback`;
/// the key/value list ends at `EGL_NONE` (or with the vector).
pub type EglIntArrayCallback =
    Box<dyn Fn(EglDisplay, Option<EglConfig>) -> Option<Vec<EglInt>> + Send + Sync>;

/// The callbacks of [`egl_set_attribute_callbacks`].
#[derive(Default)]
#[allow(missing_debug_implementations)] // (closures)
pub struct EglAttribCallbacks {
    /// The platform attributes (`platformAttribCallback`).
    pub platform: Option<EglAttribArrayCallback>,
    /// The window surface attributes (`surfaceAttribCallback`).
    pub surface: Option<EglIntArrayCallback>,
    /// The context attributes (`contextAttribCallback`).
    pub context: Option<EglIntArrayCallback>,
}

// ---------------------------------------------------------------------------
// The device's OpenGL state
// ---------------------------------------------------------------------------

/// The OpenGL attributes for new contexts. Translation of `gl_config`
/// (`driver_loaded` and `driver_path` live in the video device).
#[derive(Clone, Debug, Default)]
pub(crate) struct GlConfig {
    pub(crate) red_size: i32,
    pub(crate) green_size: i32,
    pub(crate) blue_size: i32,
    pub(crate) alpha_size: i32,
    pub(crate) depth_size: i32,
    pub(crate) buffer_size: i32,
    pub(crate) stencil_size: i32,
    pub(crate) double_buffer: i32,
    pub(crate) accum_red_size: i32,
    pub(crate) accum_green_size: i32,
    pub(crate) accum_blue_size: i32,
    pub(crate) accum_alpha_size: i32,
    pub(crate) stereo: i32,
    pub(crate) multisamplebuffers: i32,
    pub(crate) multisamplesamples: i32,
    pub(crate) floatbuffers: i32,
    pub(crate) accelerated: i32,
    pub(crate) major_version: i32,
    pub(crate) minor_version: i32,
    pub(crate) flags: i32,
    pub(crate) profile_mask: i32,
    pub(crate) share_with_current_context: i32,
    pub(crate) release_behavior: i32,
    pub(crate) reset_notification: i32,
    pub(crate) framebuffer_srgb_capable: i32,
    pub(crate) no_error: i32,
    pub(crate) retained_backing: i32,
    pub(crate) egl_platform: i32,
    #[allow(dead_code)] // (only WGL finds it out; WGL is not translated yet)
    pub(crate) has_gl_arb_color_buffer_float: bool,
}

/// The OpenGL part of the video device: the attributes, the current window
/// and context (`current_glwin`, `current_glctx`), `gl_allow_no_surface`
/// and the EGL attribute callbacks.
struct GlState {
    config: GlConfig,
    current: Option<(Option<WindowID>, RawGlContext)>,
    allow_no_surface: bool,
    egl_callbacks: Option<Arc<EglAttribCallbacks>>,
}

const DEFAULT_CONFIG: GlConfig = GlConfig {
    red_size: 0,
    green_size: 0,
    blue_size: 0,
    alpha_size: 0,
    depth_size: 0,
    buffer_size: 0,
    stencil_size: 0,
    double_buffer: 0,
    accum_red_size: 0,
    accum_green_size: 0,
    accum_blue_size: 0,
    accum_alpha_size: 0,
    stereo: 0,
    multisamplebuffers: 0,
    multisamplesamples: 0,
    floatbuffers: 0,
    accelerated: 0,
    major_version: 0,
    minor_version: 0,
    flags: 0,
    profile_mask: 0,
    share_with_current_context: 0,
    release_behavior: 0,
    reset_notification: 0,
    framebuffer_srgb_capable: 0,
    no_error: 0,
    retained_backing: 0,
    egl_platform: 0,
    has_gl_arb_color_buffer_float: false,
};

static GL_STATE: Mutex<GlState> = Mutex::new(GlState {
    config: DEFAULT_CONFIG,
    current: None,
    allow_no_surface: false,
    egl_callbacks: None,
});

/// Which video device the thread-local current context and the
/// [`GlContext`]s belong to: upstream frees the device (and its TLS slots)
/// on quit, so neither survives a re-initialization.
static GL_GENERATION: AtomicU64 = AtomicU64::new(0);

fn gl_state() -> std::sync::MutexGuard<'static, GlState> {
    GL_STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The thread's current window and context, with the device generation
/// they were set in.
#[derive(Clone, Copy)]
struct ThreadCurrent {
    generation: u64,
    window: Option<WindowID>,
    context: Option<RawGlContext>,
}

thread_local! {
    /// Translation of `current_glwin_tls` and `current_glctx_tls`.
    static CURRENT: Cell<ThreadCurrent> = const {
        Cell::new(ThreadCurrent { generation: 0, window: None, context: None })
    };
}

/// Reset the device's OpenGL state for a new video device (`_this` is
/// allocated zeroed upstream).
pub(crate) fn reset_device_state() {
    GL_GENERATION.fetch_add(1, Ordering::Relaxed);
    let mut state = gl_state();
    state.current = None;
    state.allow_no_surface = false;
    state.egl_callbacks = None;
}

/// The attributes new contexts are created with (for the GL drivers).
pub(crate) fn gl_config() -> GlConfig {
    gl_state().config.clone()
}

/// Change the attributes from a driver (`_this->gl_config.x = ...`).
#[cfg_attr(not(all(unix, not(target_vendor = "apple"))), allow(dead_code))]
pub(crate) fn with_gl_config<R>(f: impl FnOnce(&mut GlConfig) -> R) -> R {
    f(&mut gl_state().config)
}

/// `_this->gl_allow_no_surface`.
#[cfg_attr(not(all(unix, not(target_vendor = "apple"))), allow(dead_code))]
pub(crate) fn allow_no_surface() -> bool {
    gl_state().allow_no_surface
}

/// `_this->gl_allow_no_surface = true` (by EGL).
#[cfg_attr(not(all(unix, not(target_vendor = "apple"))), allow(dead_code))]
pub(crate) fn set_allow_no_surface() {
    gl_state().allow_no_surface = true;
}

/// The EGL attribute callbacks (`egl_*attrib_callback`), if set.
#[cfg_attr(not(all(unix, not(target_vendor = "apple"))), allow(dead_code))]
pub(crate) fn egl_attrib_callbacks() -> Option<Arc<EglAttribCallbacks>> {
    gl_state().egl_callbacks.clone()
}

/// `gl_config.driver_loaded` (0 when video isn't up).
#[cfg_attr(not(all(unix, not(target_vendor = "apple"))), allow(dead_code))]
pub(crate) fn driver_loaded() -> i32 {
    with_device(|v| v.gl_driver_loaded).unwrap_or(0)
}

/// Adjust `gl_config.driver_loaded` from a driver (the increment around
/// extension initialization, EGL's reset to 0...).
#[cfg_attr(not(all(unix, not(target_vendor = "apple"))), allow(dead_code))]
pub(crate) fn set_driver_loaded(f: impl FnOnce(i32) -> i32) {
    let _ = with_device(|v| v.gl_driver_loaded = f(v.gl_driver_loaded));
}

/// Set `gl_config.driver_path` from a driver (`None`: cleared).
#[cfg_attr(not(all(unix, not(target_vendor = "apple"))), allow(dead_code))]
pub(crate) fn set_driver_path(path: Option<&str>) {
    let _ = with_device(|v| v.gl_driver_path = path.map(str::to_owned));
}

/// The window of the device's current context (`_this->current_glwin`).
pub(crate) fn current_window_id() -> Option<WindowID> {
    gl_state().current.and_then(|(w, _)| w)
}

/// Forget a destroyed window as the current context window.
pub(crate) fn forget_window(window: WindowID) {
    let mut state = gl_state();
    if let Some((w, _)) = &mut state.current {
        if *w == Some(window) {
            *w = None;
        }
    }
}

fn thread_current() -> ThreadCurrent {
    let current = CURRENT.with(|c| c.get());
    if current.generation == GL_GENERATION.load(Ordering::Relaxed) {
        current
    } else {
        ThreadCurrent {
            generation: current.generation,
            window: None,
            context: None,
        }
    }
}

fn set_thread_current(window: Option<WindowID>, context: Option<RawGlContext>) {
    CURRENT.with(|c| {
        c.set(ThreadCurrent {
            generation: GL_GENERATION.load(Ordering::Relaxed),
            window,
            context,
        })
    });
}

/// The window of this thread's current context (`SDL_GL_GetCurrentWindow()`
/// for the drivers).
pub(crate) fn current_thread_window() -> Option<WindowID> {
    thread_current().window
}

/// This thread's current context (`SDL_GL_GetCurrentContext()` for the
/// drivers).
pub(crate) fn current_thread_context() -> Option<RawGlContext> {
    thread_current().context
}

// ---------------------------------------------------------------------------
// GL entry points the front end calls (through SDL_GL_GetProcAddress)
// ---------------------------------------------------------------------------

type GLenum = u32;
type GLint = i32;
type GLuint = u32;

type PfnGlGetError = unsafe extern "system" fn() -> GLenum;
type PfnGlGetIntegerv = unsafe extern "system" fn(GLenum, *mut GLint);
type PfnGlGetString = unsafe extern "system" fn(GLenum) -> *const u8;
type PfnGlGetStringi = unsafe extern "system" fn(GLenum, GLuint) -> *const u8;
type PfnGlEnable = unsafe extern "system" fn(GLenum);
type PfnGlBindFramebuffer = unsafe extern "system" fn(GLenum, GLuint);
type PfnGlGetFramebufferAttachmentParameteriv =
    unsafe extern "system" fn(GLenum, GLenum, GLenum, *mut GLint);

const GL_NO_ERROR: GLenum = 0;
const GL_NONE: GLint = 0;
const GL_INVALID_ENUM: GLenum = 0x0500;
const GL_INVALID_VALUE: GLenum = 0x0501;
const GL_BACK_LEFT: GLenum = 0x0402;
const GL_DOUBLEBUFFER: GLenum = 0x0C32;
const GL_STEREO: GLenum = 0x0C33;
const GL_RED_BITS: GLenum = 0x0D52;
const GL_GREEN_BITS: GLenum = 0x0D53;
const GL_BLUE_BITS: GLenum = 0x0D54;
const GL_ALPHA_BITS: GLenum = 0x0D55;
const GL_DEPTH_BITS: GLenum = 0x0D56;
const GL_STENCIL_BITS: GLenum = 0x0D57;
const GL_ACCUM_RED_BITS: GLenum = 0x0D58;
const GL_ACCUM_GREEN_BITS: GLenum = 0x0D59;
const GL_ACCUM_BLUE_BITS: GLenum = 0x0D5A;
const GL_ACCUM_ALPHA_BITS: GLenum = 0x0D5B;
const GL_DEPTH: GLenum = 0x1801;
const GL_STENCIL: GLenum = 0x1802;
const GL_VERSION: GLenum = 0x1F02;
const GL_EXTENSIONS: GLenum = 0x1F03;
const GL_SAMPLE_BUFFERS: GLenum = 0x80A8;
const GL_SAMPLES: GLenum = 0x80A9;
const GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE: GLenum = 0x8212;
const GL_FRAMEBUFFER_ATTACHMENT_GREEN_SIZE: GLenum = 0x8213;
const GL_FRAMEBUFFER_ATTACHMENT_BLUE_SIZE: GLenum = 0x8214;
const GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE: GLenum = 0x8215;
const GL_FRAMEBUFFER_ATTACHMENT_DEPTH_SIZE: GLenum = 0x8216;
const GL_FRAMEBUFFER_ATTACHMENT_STENCIL_SIZE: GLenum = 0x8217;
const GL_FRAMEBUFFER_DEFAULT: GLint = 0x8218;
#[cfg_attr(not(all(unix, not(target_vendor = "apple"))), allow(dead_code))]
pub(crate) const GL_MAJOR_VERSION: GLenum = 0x821B;
const GL_NUM_EXTENSIONS: GLenum = 0x821D;
const GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE: GLenum = 0x8CD0;
const GL_DRAW_FRAMEBUFFER_BINDING: GLenum = 0x8CA6;
const GL_DRAW_FRAMEBUFFER: GLenum = 0x8CA9;
const GL_FRAMEBUFFER: GLenum = 0x8D40;
/// GL_CONTEXT_RELEASE_BEHAVIOR and GL_CONTEXT_RELEASE_BEHAVIOR_KHR have the same number.
const GL_CONTEXT_RELEASE_BEHAVIOR: GLenum = 0x82FB;
/// GL_CONTEXT_RELEASE_BEHAVIOR_FLUSH and GL_CONTEXT_RELEASE_BEHAVIOR_FLUSH_KHR have the same number.
const GL_CONTEXT_RELEASE_BEHAVIOR_FLUSH_GL: GLint = 0x82FC;
/// This is always the same number between the various EXT/ARB/GLES extensions.
const GL_FRAMEBUFFER_SRGB: GLenum = 0x8DB9;
const GL_RGBA_FLOAT_MODE_ARB: GLenum = 0x8820;

/// `SDL_GL_GetProcAddress()` keeping the error: `Err` when no driver can
/// be asked, `Ok(None)` when the driver doesn't have the function.
fn get_proc_address(proc_name: &str) -> Result<Option<NonNull<c_void>>> {
    let loaded = with_device(|v| v.gl_driver_loaded)?;
    match driver()?.gl_get_proc_address(proc_name) {
        Some(func) => {
            if loaded != 0 {
                Ok(func)
            } else {
                Err(Error::new("No GL driver has been loaded"))
            }
        }
        None => Err(Error::new(format!(
            "No dynamic GL support in current SDL video driver ({})",
            core::current_video_driver().unwrap_or("")
        ))),
    }
}

/// A GL function by name, cast to its type.
///
/// # Safety
///
/// `F` must be the function pointer type of the GL entry point `name`.
pub(crate) unsafe fn gl_function<F: Copy>(name: &str) -> Option<F> {
    const { assert!(size_of::<F>() == size_of::<*mut c_void>()) };
    let p = get_proc_address(name).ok()??;
    // SAFETY: the caller's contract; F is a function pointer type.
    Some(unsafe { std::mem::transmute_copy::<*mut c_void, F>(&p.as_ptr()) })
}

/// A GL function the caller can't do without: the error is the one
/// `SDL_GL_GetProcAddress()` sets, or that the function is missing.
///
/// # Safety
///
/// As for [`gl_function`].
unsafe fn required_gl_function<F: Copy>(name: &str) -> Result<F> {
    const { assert!(size_of::<F>() == size_of::<*mut c_void>()) };
    match get_proc_address(name)? {
        // SAFETY: the caller's contract; F is a function pointer type.
        Some(p) => Ok(unsafe { std::mem::transmute_copy::<*mut c_void, F>(&p.as_ptr()) }),
        None => Err(Error::new(format!("Couldn't find OpenGL function {name}"))),
    }
}

/// `glGetString(name)` as a Rust string.
fn gl_get_string(get_string: PfnGlGetString, name: GLenum) -> Option<String> {
    // SAFETY: a GL context is current (glGetString returns NULL otherwise);
    // the result is NULL or a NUL-terminated string owned by GL.
    unsafe {
        let s = get_string(name);
        if s.is_null() {
            None
        } else {
            Some(
                CStr::from_ptr(s as *const c_char)
                    .to_string_lossy()
                    .into_owned(),
            )
        }
    }
}

/// Translation of `isAtLeastGL3()`.
fn is_at_least_gl3(verstr: Option<&str>) -> bool {
    verstr.is_some_and(|v| crate::stdlib::atoi(v) >= 3)
}

/// Whether `extension` is a whole word of the space-separated
/// `extensions`, the way upstream parses extension strings ("It takes a
/// bit of care to be fool-proof about parsing the OpenGL extensions
/// string. Don't be fooled by sub-strings, etc.").
///
/// `from_search_start` selects the variant of the GLX and WGL
/// `HasExtension()`, which accepts a match at the place the previous
/// search stopped as the start of a word.
pub(crate) fn extension_in_list(
    extensions: &str,
    extension: &str,
    from_search_start: bool,
) -> bool {
    let haystack = extensions.as_bytes();
    let needle = extension.as_bytes();
    if needle.is_empty() {
        // (strstr finds an empty string at the start)
        return haystack.first().is_none_or(|&c| c == b' ');
    }
    let mut start = 0;
    while let Some(pos) = haystack[start..]
        .windows(needle.len())
        .position(|w| w == needle)
    {
        let where_ = start + pos;
        let terminator = where_ + needle.len();
        let at_word_start = if from_search_start {
            // FIXME (upstream): the GLX and WGL HasExtension() compare with
            // where the search started, so after a partial match ("xabc"
            // looking for "abc") a match right after it ("xabcabc") counts
            // as a whole word.
            where_ == start
        } else {
            where_ == 0
        };
        if (at_word_start || haystack[where_ - 1] == b' ')
            && (terminator == haystack.len() || haystack[terminator] == b' ')
        {
            return true;
        }

        start = terminator;
    }
    false
}

// ---------------------------------------------------------------------------
// Library loading
// ---------------------------------------------------------------------------

/// Load an OpenGL library (`None`: the default one); calls are reference
/// counted. Translation of `SDL_GL_LoadLibrary()`.
pub fn gl_load_library(path: Option<&str>) -> Result<()> {
    let (loaded, loaded_path) = with_device(|v| (v.gl_driver_loaded, v.gl_driver_path.clone()))?;
    let driver = driver()?;
    let result = if loaded != 0 {
        if let Some(path) = path {
            if path != loaded_path.as_deref().unwrap_or("") {
                return Err(Error::new("OpenGL library already loaded"));
            }
        }
        Ok(())
    } else {
        match driver.gl_load_library(path) {
            None => return Err(dll_not_supported("OpenGL")),
            Some(result) => result,
        }
    };
    match result {
        Ok(()) => {
            with_device(|v| v.gl_driver_loaded += 1)?;
            Ok(())
        }
        Err(e) => {
            let _ = driver.gl_unload_library();
            Err(e)
        }
    }
}

/// Get an OpenGL function by name (once a library is loaded); `None` if
/// there is no such function or no library. Translation of
/// `SDL_GL_GetProcAddress()`.
pub fn gl_get_proc_address(proc_name: &str) -> Option<*const c_void> {
    get_proc_address(proc_name)
        .ok()
        .flatten()
        .map(|p| p.as_ptr() as *const c_void)
}

/// Get an EGL library function by name (once EGL is loaded); `None` if
/// there is no such function or EGL isn't in use. Translation of
/// `SDL_EGL_GetProcAddress()`.
pub fn egl_get_proc_address(proc_name: &str) -> Option<*const c_void> {
    #[cfg(all(unix, not(target_vendor = "apple")))]
    {
        if !core::initialized() {
            // (SDL_UninitializedVideo())
            return None;
        }
        if !super::egl::is_loaded() {
            // (SDL_SetError("No EGL library has been loaded"))
            return None;
        }
        super::egl::get_proc_address_internal(proc_name).map(|p| p.as_ptr() as *const c_void)
    }
    #[cfg(not(all(unix, not(target_vendor = "apple"))))]
    {
        // (SDL_SetError("SDL was not built with EGL support"))
        let _ = proc_name;
        None
    }
}

/// Unload the OpenGL library loaded by [`gl_load_library`] (when the last
/// reference goes). Translation of `SDL_GL_UnloadLibrary()`.
pub fn gl_unload_library() {
    let Ok(Some(driver)) = with_device(|v| {
        if v.gl_driver_loaded > 0 {
            v.gl_driver_loaded -= 1;
            if v.gl_driver_loaded > 0 {
                return None;
            }
            Some(v.driver.clone())
        } else {
            None
        }
    }) else {
        return;
    };
    let _ = driver.gl_unload_library();
}

/// Whether an OpenGL extension is supported by the current context (the
/// hint or environment variable of the extension's name set to "0" hides
/// it). Translation of `SDL_GL_ExtensionSupported()`.
pub fn gl_extension_supported(extension: &str) -> bool {
    // Extension names should not have spaces.
    if extension.contains(' ') || extension.is_empty() {
        return false;
    }
    // See if there's a hint or environment variable override
    if hints::get(extension).is_some_and(|start| start.starts_with('0')) {
        return false;
    }

    // Lookup the available extensions

    // SAFETY: glGetString's type.
    let Some(gl_get_string_func) = (unsafe { gl_function::<PfnGlGetString>("glGetString") }) else {
        return false;
    };

    if is_at_least_gl3(gl_get_string(gl_get_string_func, GL_VERSION).as_deref()) {
        // SAFETY: the functions' types.
        let (get_stringi, get_integerv) = unsafe {
            (
                gl_function::<PfnGlGetStringi>("glGetStringi"),
                gl_function::<PfnGlGetIntegerv>("glGetIntegerv"),
            )
        };
        let (Some(gl_get_stringi_func), Some(gl_get_integerv_func)) = (get_stringi, get_integerv)
        else {
            return false;
        };

        let mut num_exts: GLint = 0;
        // SAFETY: a context is current; num_exts is a valid out-parameter.
        unsafe { gl_get_integerv_func(GL_NUM_EXTENSIONS, &mut num_exts) };
        for i in 0..num_exts.max(0) {
            // SAFETY: i < GL_NUM_EXTENSIONS; the result is NULL or a
            // NUL-terminated string owned by GL.
            let thisext = unsafe { gl_get_stringi_func(GL_EXTENSIONS, i as GLuint) };
            // FIXME (upstream): strcmp() on a NULL glGetStringi() result
            // crashes; NULL is skipped here.
            if thisext.is_null() {
                continue;
            }
            // SAFETY: as above.
            if unsafe { CStr::from_ptr(thisext as *const c_char) }.to_bytes()
                == extension.as_bytes()
            {
                return true;
            }
        }

        return false;
    }

    // Try the old way with glGetString(GL_EXTENSIONS) ...

    let Some(extensions) = gl_get_string(gl_get_string_func, GL_EXTENSIONS) else {
        return false;
    };
    extension_in_list(&extensions, extension, false)
}

/// The highest OpenGL ES version the current context's
/// `GL_ARB_ES*_compatibility` extensions support (there is no direct
/// query). Translation of `SDL_GL_DeduceMaxSupportedESProfile()`.
///
/// This is normally only called when the OpenGL driver supports
/// {GLX,WGL}_EXT_create_context_es2_profile; it requires an existing GL
/// context that has been made current.
#[cfg_attr(not(all(unix, not(target_vendor = "apple"))), allow(dead_code))]
pub(crate) fn gl_deduce_max_supported_es_profile() -> (i32, i32) {
    // XXX This is fragile; it will break in the event of release of
    // new versions of OpenGL ES.
    if gl_extension_supported("GL_ARB_ES3_2_compatibility") {
        (3, 2)
    } else if gl_extension_supported("GL_ARB_ES3_1_compatibility") {
        (3, 1)
    } else if gl_extension_supported("GL_ARB_ES3_compatibility") {
        (3, 0)
    } else {
        (2, 0)
    }
}

/// Set the callbacks that supply extra EGL attributes (cleared by
/// [`gl_reset_attributes`]). Translation of `SDL_EGL_SetAttributeCallbacks()`.
pub fn egl_set_attribute_callbacks(callbacks: EglAttribCallbacks) {
    if !core::initialized() {
        return;
    }
    gl_state().egl_callbacks = Some(Arc::new(callbacks));
}

/// Reset all previously set OpenGL context attributes to their defaults.
/// Translation of `SDL_GL_ResetAttributes()`.
pub fn gl_reset_attributes() {
    let Ok(driver) = driver() else { return };

    let mut config = GlConfig {
        red_size: 8,
        green_size: 8,
        blue_size: 8,
        alpha_size: 8,
        buffer_size: 0,
        depth_size: 16,
        stencil_size: 0,
        double_buffer: 1,
        accum_red_size: 0,
        accum_green_size: 0,
        accum_blue_size: 0,
        accum_alpha_size: 0,
        stereo: 0,
        multisamplebuffers: 0,
        multisamplesamples: 0,
        floatbuffers: 0,
        retained_backing: 1,
        accelerated: -1, // accelerated or not, both are fine
        // (SDL_VIDEO_OPENGL)
        major_version: 2,
        minor_version: 1,
        profile_mask: 0,
        flags: 0,
        framebuffer_srgb_capable: 0,
        no_error: 0,
        release_behavior: GL_CONTEXT_RELEASE_BEHAVIOR_FLUSH,
        reset_notification: GL_CONTEXT_RESET_NO_NOTIFICATION,
        share_with_current_context: 0,
        egl_platform: 0,
        has_gl_arb_color_buffer_float: false,
    };

    let _ = driver.gl_set_default_profile_config(&mut config);
    let mut state = gl_state();
    state.egl_callbacks = None;
    // (HAS_GL_ARB_color_buffer_float isn't reset)
    config.has_gl_arb_color_buffer_float = state.config.has_gl_arb_color_buffer_float;
    state.config = config;
}

// ---------------------------------------------------------------------------
// Attributes
// ---------------------------------------------------------------------------

/// Set an OpenGL window attribute before window creation. Translation of
/// `SDL_GL_SetAttribute()`.
pub fn gl_set_attribute(attr: GlAttr, value: i32) -> Result<()> {
    if !core::initialized() {
        return Err(uninitialized_video());
    }
    let mut state = gl_state();
    let c = &mut state.config;
    match attr {
        GlAttr::RedSize => c.red_size = value,
        GlAttr::GreenSize => c.green_size = value,
        GlAttr::BlueSize => c.blue_size = value,
        GlAttr::AlphaSize => c.alpha_size = value,
        GlAttr::DoubleBuffer => c.double_buffer = value,
        GlAttr::BufferSize => c.buffer_size = value,
        GlAttr::DepthSize => c.depth_size = value,
        GlAttr::StencilSize => c.stencil_size = value,
        GlAttr::AccumRedSize => c.accum_red_size = value,
        GlAttr::AccumGreenSize => c.accum_green_size = value,
        GlAttr::AccumBlueSize => c.accum_blue_size = value,
        GlAttr::AccumAlphaSize => c.accum_alpha_size = value,
        GlAttr::Stereo => c.stereo = value,
        GlAttr::MultisampleBuffers => c.multisamplebuffers = value,
        GlAttr::MultisampleSamples => c.multisamplesamples = value,
        GlAttr::FloatBuffers => c.floatbuffers = value,
        GlAttr::AcceleratedVisual => c.accelerated = value,
        GlAttr::RetainedBacking => c.retained_backing = value,
        GlAttr::ContextMajorVersion => c.major_version = value,
        GlAttr::ContextMinorVersion => c.minor_version = value,
        GlAttr::ContextFlags => {
            if value
                & !(GL_CONTEXT_DEBUG_FLAG
                    | GL_CONTEXT_FORWARD_COMPATIBLE_FLAG
                    | GL_CONTEXT_ROBUST_ACCESS_FLAG
                    | GL_CONTEXT_RESET_ISOLATION_FLAG)
                != 0
            {
                return Err(Error::new(format!("Unknown OpenGL context flag {value}")));
            }
            c.flags = value;
        }
        GlAttr::ContextProfileMask => {
            if value != 0
                && value != GL_CONTEXT_PROFILE_CORE
                && value != GL_CONTEXT_PROFILE_COMPATIBILITY
                && value != GL_CONTEXT_PROFILE_ES
            {
                return Err(Error::new(format!(
                    "Unknown OpenGL context profile {value}"
                )));
            }
            c.profile_mask = value;
        }
        GlAttr::ShareWithCurrentContext => c.share_with_current_context = value,
        GlAttr::FramebufferSrgbCapable => c.framebuffer_srgb_capable = value,
        GlAttr::ContextReleaseBehavior => c.release_behavior = value,
        GlAttr::ContextResetNotification => c.reset_notification = value,
        GlAttr::ContextNoError => c.no_error = value,
        GlAttr::EglPlatform => c.egl_platform = value,
    }
    Ok(())
}

/// How [`gl_get_attribute`] finds an attribute's value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AttrQuery {
    /// Known without asking GL.
    Value(i32),
    /// `glGetIntegerv(attrib)`, or with OpenGL 3+ and a nonzero
    /// `attachment_attrib` `glGetFramebufferAttachmentParameteriv()` on
    /// the window framebuffer's `attachment`.
    Gl {
        attrib: GLenum,
        attachment: GLenum,
        attachment_attrib: GLenum,
    },
    /// The sum of the color sizes.
    BufferSize,
    /// `HAS_GL_ARB_color_buffer_float` is false.
    Unavailable,
    /// No case for it (`default:`).
    Unknown,
}

/// The query for an attribute (the `switch` of `SDL_GL_GetAttribute()`).
fn attribute_query(attr: GlAttr, config: &GlConfig) -> AttrQuery {
    let gl = |attrib, attachment, attachment_attrib| AttrQuery::Gl {
        attrib,
        attachment,
        attachment_attrib,
    };
    match attr {
        GlAttr::RedSize => gl(
            GL_RED_BITS,
            GL_BACK_LEFT,
            GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE,
        ),
        GlAttr::BlueSize => gl(
            GL_BLUE_BITS,
            GL_BACK_LEFT,
            GL_FRAMEBUFFER_ATTACHMENT_BLUE_SIZE,
        ),
        GlAttr::GreenSize => gl(
            GL_GREEN_BITS,
            GL_BACK_LEFT,
            GL_FRAMEBUFFER_ATTACHMENT_GREEN_SIZE,
        ),
        GlAttr::AlphaSize => gl(
            GL_ALPHA_BITS,
            GL_BACK_LEFT,
            GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE,
        ),
        GlAttr::DoubleBuffer => gl(GL_DOUBLEBUFFER, GL_BACK_LEFT, 0),
        GlAttr::DepthSize => gl(
            GL_DEPTH_BITS,
            GL_DEPTH,
            GL_FRAMEBUFFER_ATTACHMENT_DEPTH_SIZE,
        ),
        GlAttr::StencilSize => gl(
            GL_STENCIL_BITS,
            GL_STENCIL,
            GL_FRAMEBUFFER_ATTACHMENT_STENCIL_SIZE,
        ),
        GlAttr::AccumRedSize => gl(GL_ACCUM_RED_BITS, GL_BACK_LEFT, 0),
        GlAttr::AccumGreenSize => gl(GL_ACCUM_GREEN_BITS, GL_BACK_LEFT, 0),
        GlAttr::AccumBlueSize => gl(GL_ACCUM_BLUE_BITS, GL_BACK_LEFT, 0),
        GlAttr::AccumAlphaSize => gl(GL_ACCUM_ALPHA_BITS, GL_BACK_LEFT, 0),
        GlAttr::Stereo => gl(GL_STEREO, GL_BACK_LEFT, 0),
        GlAttr::MultisampleBuffers => gl(GL_SAMPLE_BUFFERS, GL_BACK_LEFT, 0),
        GlAttr::MultisampleSamples => gl(GL_SAMPLES, GL_BACK_LEFT, 0),
        GlAttr::ContextReleaseBehavior => gl(GL_CONTEXT_RELEASE_BEHAVIOR, GL_BACK_LEFT, 0),
        GlAttr::BufferSize => AttrQuery::BufferSize,
        // Note that we can _not_ reasonably detect this once we have a context.
        GlAttr::AcceleratedVisual => AttrQuery::Value((config.accelerated != 0) as i32),
        GlAttr::RetainedBacking => AttrQuery::Value(config.retained_backing),
        GlAttr::ContextMajorVersion => AttrQuery::Value(config.major_version),
        GlAttr::ContextMinorVersion => AttrQuery::Value(config.minor_version),
        GlAttr::ContextFlags => AttrQuery::Value(config.flags),
        GlAttr::ContextProfileMask => AttrQuery::Value(config.profile_mask),
        GlAttr::ShareWithCurrentContext => AttrQuery::Value(config.share_with_current_context),
        GlAttr::FramebufferSrgbCapable => AttrQuery::Value(config.framebuffer_srgb_capable),
        GlAttr::ContextNoError => AttrQuery::Value(config.no_error),
        GlAttr::EglPlatform => AttrQuery::Value(config.egl_platform),
        // FIXME (upstream): SDL_GL_ContextResetNotification has no case in
        // SDL_GL_GetAttribute(), which fails with "Unknown OpenGL
        // attribute" for it.
        GlAttr::ContextResetNotification => AttrQuery::Unknown,
        GlAttr::FloatBuffers => {
            if config.has_gl_arb_color_buffer_float {
                gl(GL_RGBA_FLOAT_MODE_ARB, GL_BACK_LEFT, 0)
            } else {
                AttrQuery::Unavailable
            }
        }
    }
}

/// The actual value of an attribute for the current context. Translation
/// of `SDL_GL_GetAttribute()`.
pub fn gl_get_attribute(attr: GlAttr) -> Result<i32> {
    if !core::initialized() {
        return Err(uninitialized_video());
    }

    let config = gl_config();
    let (attrib, attachment, attachment_attrib) = match attribute_query(attr, &config) {
        AttrQuery::Value(value) => return Ok(value),
        AttrQuery::BufferSize => {
            // There doesn't seem to be a single flag in OpenGL for this!
            let rsize = gl_get_attribute(GlAttr::RedSize)?;
            let gsize = gl_get_attribute(GlAttr::GreenSize)?;
            let bsize = gl_get_attribute(GlAttr::BlueSize)?;
            let asize = gl_get_attribute(GlAttr::AlphaSize)?;
            return Ok(rsize + gsize + bsize + asize);
        }
        AttrQuery::Unknown => return Err(Error::new("Unknown OpenGL attribute")),
        // FIXME (upstream): SDL_GL_FLOATBUFFERS fails ("return 0") without
        // setting an error when GL_ARB_color_buffer_float is unknown; and
        // only WGL ever finds that out. This reports "unsupported".
        AttrQuery::Unavailable => return Err(Error::unsupported()),
        AttrQuery::Gl {
            attrib,
            attachment,
            attachment_attrib,
        } => (attrib, attachment, attachment_attrib),
    };

    let mut value: GLint = 0;

    // SAFETY: glGetString's type.
    let gl_get_string_func = unsafe { required_gl_function::<PfnGlGetString>("glGetString")? };

    /*
     * Some queries in Core Profile desktop OpenGL 3+ contexts require
     * glGetFramebufferAttachmentParameteriv instead of glGetIntegerv. Note that
     * the enums we use for the former function don't exist in OpenGL ES 2, and
     * the function itself doesn't exist prior to OpenGL 3 and OpenGL ES 2.
     */
    if attachment_attrib != 0
        && is_at_least_gl3(gl_get_string(gl_get_string_func, GL_VERSION).as_deref())
    {
        // glGetFramebufferAttachmentParameteriv needs to operate on the window framebuffer for this, so bind FBO 0 if necessary.
        let mut current_fbo: GLint = 0;
        // SAFETY: the functions' types.
        let (get_integerv, bind_framebuffer, get_attachment_parameteriv) = unsafe {
            (
                gl_function::<PfnGlGetIntegerv>("glGetIntegerv"),
                gl_function::<PfnGlBindFramebuffer>("glBindFramebuffer"),
                gl_function::<PfnGlGetFramebufferAttachmentParameteriv>(
                    "glGetFramebufferAttachmentParameteriv",
                ),
            )
        };
        if let (Some(get_integerv), Some(_)) = (get_integerv, bind_framebuffer) {
            // SAFETY: a context is current; current_fbo is a valid out-parameter.
            unsafe { get_integerv(GL_DRAW_FRAMEBUFFER_BINDING, &mut current_fbo) };
        }

        let Some(get_attachment_parameteriv) = get_attachment_parameteriv else {
            return Err(Error::new(
                "Couldn't find OpenGL function glGetFramebufferAttachmentParameteriv",
            ));
        };
        // SAFETY: a context is current; the out-parameters are valid.
        unsafe {
            if let (Some(bind), true) = (bind_framebuffer, current_fbo != 0) {
                bind(GL_DRAW_FRAMEBUFFER, 0);
            }
            // glGetFramebufferAttachmentParameterivFunc may cause GL_INVALID_OPERATION when querying depth/stencil size if the
            // bits is 0. From the GL docs:
            //      If the value of GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE is GL_NONE, then either no framebuffer is bound to target;
            //      or a default framebuffer is queried, attachment is GL_DEPTH or GL_STENCIL, and the number of depth or stencil bits,
            //      respectively, is zero. In this case querying pname GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME will return zero, and all
            //      other queries will generate an error.
            let mut fbo_type: GLint = GL_FRAMEBUFFER_DEFAULT;
            if attachment == GL_DEPTH || attachment == GL_STENCIL {
                get_attachment_parameteriv(
                    GL_FRAMEBUFFER,
                    attachment,
                    GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE,
                    &mut fbo_type,
                );
            }
            if fbo_type != GL_NONE {
                get_attachment_parameteriv(
                    GL_FRAMEBUFFER,
                    attachment,
                    attachment_attrib,
                    &mut value,
                );
            } else {
                value = 0;
            }
            if let (Some(bind), true) = (bind_framebuffer, current_fbo != 0) {
                bind(GL_DRAW_FRAMEBUFFER, current_fbo as GLuint);
            }
        }
    } else {
        // SAFETY: glGetIntegerv's type.
        let get_integerv = unsafe { required_gl_function::<PfnGlGetIntegerv>("glGetIntegerv")? };
        // SAFETY: a context is current; value is a valid out-parameter.
        unsafe { get_integerv(attrib, &mut value) };
    }

    // SAFETY: glGetError's type.
    let gl_get_error_func = unsafe { required_gl_function::<PfnGlGetError>("glGetError")? };

    // SAFETY: glGetError has no preconditions.
    let error = unsafe { gl_get_error_func() };
    if error != GL_NO_ERROR {
        if error == GL_INVALID_ENUM {
            return Err(Error::new("OpenGL error: GL_INVALID_ENUM"));
        } else if error == GL_INVALID_VALUE {
            return Err(Error::new("OpenGL error: GL_INVALID_VALUE"));
        }
        return Err(Error::new(format!("OpenGL error: {error:08X}")));
    }

    // convert GL_CONTEXT_RELEASE_BEHAVIOR values back to SDL_GL_CONTEXT_RELEASE_BEHAVIOR values
    if attr == GlAttr::ContextReleaseBehavior {
        value = if value == GL_CONTEXT_RELEASE_BEHAVIOR_FLUSH_GL {
            GL_CONTEXT_RELEASE_BEHAVIOR_FLUSH
        } else {
            GL_CONTEXT_RELEASE_BEHAVIOR_NONE
        };
    }

    Ok(value)
}

// ---------------------------------------------------------------------------
// Contexts
// ---------------------------------------------------------------------------

/// The error for a window without [`WindowFlags::OPENGL`].
fn not_an_opengl_window() -> Error {
    Error::new("The specified window isn't an OpenGL window")
}

/// Create an OpenGL context for an OpenGL window, and make it current.
/// Translation of `SDL_GL_CreateContext()`.
pub fn gl_create_context(window: &Window) -> Result<GlContext> {
    let flags = window.flags()?;
    if !flags.contains(WindowFlags::OPENGL) {
        return Err(not_an_opengl_window());
    }

    let srgb_requested = match hints::get(hints::OPENGL_FORCE_SRGB_FRAMEBUFFER) {
        Some(srgbhint) if !srgbhint.is_empty() => {
            hints::string_to_bool(Some(&srgbhint), false) as i32
        }
        _ => -1,
    };

    let ctx = driver()?
        .gl_create_context(window.id())
        .unwrap_or_else(|| Err(core::context_not_supported("OpenGL")))?;

    // Creating a context is assumed to make it current in the SDL driver.
    gl_state().current = Some((Some(window.id()), ctx));
    set_thread_current(Some(window.id()), Some(ctx));
    let context = GlContext {
        raw: ctx,
        generation: GL_GENERATION.load(Ordering::Relaxed),
        _not_sync: PhantomData,
    };

    // try to force the window framebuffer to the requested sRGB state.
    if srgb_requested != -1 {
        // SAFETY: glEnable and glDisable have the same type; glGetString's.
        let (toggle, get_string) = unsafe {
            (
                gl_function::<PfnGlEnable>(if srgb_requested != 0 {
                    "glEnable"
                } else {
                    "glDisable"
                }),
                gl_function::<PfnGlGetString>("glGetString"),
            )
        };
        if let (Some(gl_toggle_func), Some(gl_get_string_func)) = (toggle, get_string) {
            let supported = if gl_config().profile_mask & GL_CONTEXT_PROFILE_ES != 0 {
                gl_extension_supported("GL_EXT_sRGB_write_control") // GL_FRAMEBUFFER_SRGB is not core in any GLES version at the moment.
            } else {
                is_at_least_gl3(gl_get_string(gl_get_string_func, GL_VERSION).as_deref()) // no extensions needed in OpenGL 3+.
                    || gl_extension_supported("GL_EXT_framebuffer_sRGB")
                    || gl_extension_supported("GL_ARB_framebuffer_sRGB")
            };

            if supported {
                // SAFETY: the new context is current.
                unsafe { gl_toggle_func(GL_FRAMEBUFFER_SRGB) };
            }
        }
    }

    Ok(context)
}

/// Make a context current for a window (`None`, `None`: release the
/// current context). Translation of `SDL_GL_MakeCurrent()`.
pub fn gl_make_current(window: Option<&Window>, context: Option<&GlContext>) -> Result<()> {
    make_current_raw(window.map(|w| w.id()), context.map(|c| c.raw))
}

/// Release this thread's current context (`SDL_GL_MakeCurrent(NULL, NULL)`).
pub fn gl_release_current() -> Result<()> {
    make_current_raw(None, None)
}

/// `SDL_GL_MakeCurrent()` with raw handles.
pub(crate) fn make_current_raw(
    mut window: Option<WindowID>,
    context: Option<RawGlContext>,
) -> Result<()> {
    if !core::initialized() {
        return Err(uninitialized_video());
    }

    if window == current_thread_window() && context == current_thread_context() {
        // We're already current.
        return Ok(());
    }

    let driver = driver()?;
    if context.is_none() {
        window = None;
    } else if let Some(w) = window {
        if !with_window(w, |w| w.flags().contains(WindowFlags::OPENGL))? {
            return Err(not_an_opengl_window());
        }
    } else if !allow_no_surface() {
        return Err(Error::new(
            "Use of OpenGL without a window is not supported on this platform",
        ));
    }

    driver
        .gl_make_current(window, context)
        .unwrap_or_else(|| Err(core::context_not_supported("OpenGL")))?;
    gl_state().current = context.map(|c| (window, c));
    set_thread_current(window, context);
    Ok(())
}

/// The window of this thread's current context. Translation of
/// `SDL_GL_GetCurrentWindow()`.
pub fn gl_current_window() -> Result<Option<Window>> {
    if !core::initialized() {
        return Err(uninitialized_video());
    }
    Ok(current_thread_window().map(Window::from_raw))
}

/// This thread's current context. Translation of `SDL_GL_GetCurrentContext()`.
pub fn gl_current_context() -> Result<Option<RawGlContext>> {
    if !core::initialized() {
        return Err(uninitialized_video());
    }
    Ok(current_thread_context())
}

/// The current EGL display. Translation of `SDL_EGL_GetCurrentDisplay()`.
pub fn egl_current_display() -> Result<EglDisplay> {
    #[cfg(all(unix, not(target_vendor = "apple")))]
    {
        if !core::initialized() {
            return Err(uninitialized_video());
        }
        super::egl::current_display()
            .flatten()
            .ok_or_else(|| Error::new("There is no current EGL display"))
    }
    #[cfg(not(all(unix, not(target_vendor = "apple"))))]
    {
        Err(Error::new("SDL was not built with EGL support"))
    }
}

/// The current EGL config (`None` before one is chosen). Translation of
/// `SDL_EGL_GetCurrentConfig()`.
pub fn egl_current_config() -> Result<Option<EglConfig>> {
    #[cfg(all(unix, not(target_vendor = "apple")))]
    {
        if !core::initialized() {
            return Err(uninitialized_video());
        }
        super::egl::current_config().ok_or_else(|| Error::new("There is no current EGL display"))
    }
    #[cfg(not(all(unix, not(target_vendor = "apple"))))]
    {
        Err(Error::new("SDL was not built with EGL support"))
    }
}

/// The EGL surface of a window (`None` if it has none). Translation of
/// `SDL_EGL_GetWindowSurface()`.
pub fn egl_window_surface(window: &Window) -> Result<Option<EglSurface>> {
    #[cfg(all(unix, not(target_vendor = "apple")))]
    {
        if !core::initialized() {
            return Err(uninitialized_video());
        }
        if !super::egl::is_loaded() {
            return Err(Error::new("There is no current EGL display"));
        }
        Ok(driver()?
            .gl_get_egl_surface(window.id())
            .and_then(EglSurface::from_ptr))
    }
    #[cfg(not(all(unix, not(target_vendor = "apple"))))]
    {
        let _ = window;
        Err(Error::new("SDL was not built with EGL support"))
    }
}

/// Set the swap interval for the current context (0 immediate, 1 vsync, -1
/// adaptive vsync). Translation of `SDL_GL_SetSwapInterval()`.
pub fn gl_set_swap_interval(interval: i32) -> Result<()> {
    if !core::initialized() {
        return Err(uninitialized_video());
    }
    if current_thread_context().is_none() {
        return Err(Error::new("No OpenGL context has been made current"));
    }
    driver()?
        .gl_set_swap_interval(interval)
        .unwrap_or_else(|| Err(Error::new("Setting the swap interval is not supported")))
}

/// The swap interval of the current context. Translation of
/// `SDL_GL_GetSwapInterval()`.
pub fn gl_get_swap_interval() -> Result<i32> {
    if !core::initialized() {
        return Err(Error::new("no video driver"));
    }
    if current_thread_context().is_none() {
        return Err(Error::new("no current context"));
    }
    driver()?
        .gl_get_swap_interval()
        .unwrap_or_else(|| Err(Error::new("not implemented")))
}

/// Swap the buffers of an OpenGL window whose context is current.
/// Translation of `SDL_GL_SwapWindow()`.
pub fn gl_swap_window(window: &Window) -> Result<()> {
    if !window.flags()?.contains(WindowFlags::OPENGL) {
        return Err(not_an_opengl_window());
    }

    if current_thread_window() != Some(window.id()) {
        return Err(Error::new("The specified window has not been made current"));
    }

    driver()?
        .gl_swap_window(window.id())
        .unwrap_or_else(|| Err(core::context_not_supported("OpenGL")))
}

/// Delete an OpenGL context. Translation of `SDL_GL_DestroyContext()`.
fn destroy_raw(context: RawGlContext, generation: u64) -> Result<()> {
    if !core::initialized() {
        return Err(uninitialized_video());
    }
    if generation != GL_GENERATION.load(Ordering::Relaxed) {
        // Note (upstream): a context of a video device that has been shut
        // down is a dangling pointer for the new device; it is not passed
        // to the driver.
        return Err(Error::invalid_param("context"));
    }

    if current_thread_context() == Some(context) {
        let _ = make_current_raw(None, None);
    }

    driver()?
        .gl_destroy_context(context)
        .unwrap_or_else(|| Err(core::context_not_supported("OpenGL")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The C reference's SDL_GL_ExtensionSupported() string matching (see
    /// gl-ref/extmatch.c): (extensions, extension, front end, GLX/WGL).
    const EXTENSION_CASES: &[(&str, &str, bool, bool)] = &[
        ("GL_ARB_a GL_ARB_b", "GL_ARB_a", true, true),
        ("GL_ARB_a GL_ARB_b", "GL_ARB_b", true, true),
        ("GL_ARB_a GL_ARB_b", "GL_ARB", false, false),
        ("GL_ARB_ab GL_ARB_a", "GL_ARB_a", true, true),
        ("xGL_ARB_a", "GL_ARB_a", false, false),
        ("xabcabc", "abc", false, true),
        ("xabcabc def", "abc", false, true),
        ("abc", "abc", true, true),
        ("", "abc", false, false),
        ("abcd abc", "abc", true, true),
        ("abcabc", "abc", false, true),
        ("ab abc ", "abc", true, true),
        ("  abc", "abc", true, true),
    ];

    #[test]
    fn extension_strings_match_like_upstream() {
        for &(list, ext, front, glx) in EXTENSION_CASES {
            assert_eq!(
                extension_in_list(list, ext, false),
                front,
                "{list:?} {ext:?}"
            );
            assert_eq!(
                extension_in_list(list, ext, true),
                glx,
                "{list:?} {ext:?} (GLX)"
            );
        }
    }

    #[test]
    fn gl3_detection() {
        assert!(is_at_least_gl3(Some("3.0 Mesa 25.2.8")));
        assert!(is_at_least_gl3(Some("4.6 (Compatibility Profile)")));
        assert!(!is_at_least_gl3(Some("2.1 Mesa")));
        assert!(!is_at_least_gl3(Some("OpenGL ES 3.2 Mesa")));
        assert!(!is_at_least_gl3(None));
    }

    #[test]
    fn attribute_queries() {
        let config = GlConfig {
            accelerated: -1,
            major_version: 3,
            ..GlConfig::default()
        };
        assert_eq!(
            attribute_query(GlAttr::AcceleratedVisual, &config),
            AttrQuery::Value(1)
        );
        assert_eq!(
            attribute_query(GlAttr::ContextMajorVersion, &config),
            AttrQuery::Value(3)
        );
        assert_eq!(
            attribute_query(GlAttr::DepthSize, &config),
            AttrQuery::Gl {
                attrib: 0x0D56,
                attachment: 0x1801,
                attachment_attrib: 0x8216
            }
        );
        assert_eq!(
            attribute_query(GlAttr::DoubleBuffer, &config),
            AttrQuery::Gl {
                attrib: 0x0C32,
                attachment: 0x0402,
                attachment_attrib: 0
            }
        );
        assert_eq!(
            attribute_query(GlAttr::FloatBuffers, &config),
            AttrQuery::Unavailable
        );
    }

    #[test]
    fn attributes_without_video() {
        let _l = crate::test_support::test_lock();
        assert!(gl_set_attribute(GlAttr::RedSize, 8).is_err());
        assert!(gl_get_attribute(GlAttr::RedSize).is_err());
        assert!(gl_get_proc_address("glGetString").is_none());
        assert!(!gl_extension_supported("GL_ARB_multisample"));
        assert!(gl_release_current().is_err());
    }
}
