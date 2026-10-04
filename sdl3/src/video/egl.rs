// Rust translation of src/video/SDL_egl.c and SDL_egl_c.h from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! EGL: the OpenGL (ES) support the X11 and Windows drivers share for EGL
//! contexts.
//!
//! libEGL and the GL library (libGL / libGLESv2 / libGLESv1_CM, or
//! `libEGL.dll` with ANGLE's `libGLESv2.dll` on Windows) are loaded at run
//! time; the EGL entry points are declared here by hand. The device's
//! `egl_data` is the `static` [`EGL_DATA`]: there is one video device.
//!
//! The entry points take no lock while calling EGL: they copy what they
//! need (the functions, the display, ...) and keep the libraries alive
//! through a shared handle for the duration of the call.

use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::events::WindowID;
use crate::hints;
use crate::loadso::SharedObject;

use super::gl::{
    self, gl_config, EglConfig, EglDisplay, GlConfig, RawGlContext, GL_CONTEXT_PROFILE_ES,
};

// ---------------------------------------------------------------------------
// EGL types and constants (EGL/egl.h, EGL/eglext.h)
// ---------------------------------------------------------------------------

pub(crate) type EGLDisplay = *mut c_void;
pub(crate) type EGLConfig = *mut c_void;
pub(crate) type EGLContext = *mut c_void;
pub(crate) type EGLSurface = *mut c_void;
pub(crate) type EGLint = i32;
pub(crate) type EGLenum = u32;
pub(crate) type EGLBoolean = u32;
pub(crate) type EGLAttrib = isize;
type EGLDeviceEXT = *mut c_void;
/// `EGLNativeDisplayType`: a `Display *` on X11, an `HDC` on Windows.
pub(crate) type NativeDisplayType = *mut c_void;
/// `EGLNativeWindowType`: an X11 `Window` (an integer the size of a
/// pointer) or an `HWND`.
pub(crate) type NativeWindowType = *mut c_void;

pub(crate) const EGL_FALSE: EGLBoolean = 0;
pub(crate) const EGL_TRUE: EGLBoolean = 1;
pub(crate) const EGL_NO_DISPLAY: EGLDisplay = std::ptr::null_mut();
pub(crate) const EGL_NO_CONTEXT: EGLContext = std::ptr::null_mut();
pub(crate) const EGL_NO_SURFACE: EGLSurface = std::ptr::null_mut();
#[allow(dead_code)] // (for the Windows EGL support, not translated yet)
pub(crate) const EGL_DEFAULT_DISPLAY: NativeDisplayType = std::ptr::null_mut();

const EGL_SUCCESS: EGLint = 0x3000;
const EGL_NOT_INITIALIZED: EGLint = 0x3001;
const EGL_BAD_ACCESS: EGLint = 0x3002;
const EGL_BAD_ALLOC: EGLint = 0x3003;
const EGL_BAD_ATTRIBUTE: EGLint = 0x3004;
const EGL_BAD_CONFIG: EGLint = 0x3005;
const EGL_BAD_CONTEXT: EGLint = 0x3006;
const EGL_BAD_CURRENT_SURFACE: EGLint = 0x3007;
const EGL_BAD_DISPLAY: EGLint = 0x3008;
const EGL_BAD_MATCH: EGLint = 0x3009;
const EGL_BAD_NATIVE_PIXMAP: EGLint = 0x300A;
const EGL_BAD_NATIVE_WINDOW: EGLint = 0x300B;
const EGL_BAD_PARAMETER: EGLint = 0x300C;
const EGL_BAD_SURFACE: EGLint = 0x300D;
const EGL_CONTEXT_LOST: EGLint = 0x300E;
const EGL_BUFFER_SIZE: EGLint = 0x3020;
const EGL_ALPHA_SIZE: EGLint = 0x3021;
const EGL_BLUE_SIZE: EGLint = 0x3022;
const EGL_GREEN_SIZE: EGLint = 0x3023;
const EGL_RED_SIZE: EGLint = 0x3024;
const EGL_DEPTH_SIZE: EGLint = 0x3025;
const EGL_STENCIL_SIZE: EGLint = 0x3026;
const EGL_CONFIG_CAVEAT: EGLint = 0x3027;
pub(crate) const EGL_NATIVE_VISUAL_ID: EGLint = 0x302E;
const EGL_SAMPLES: EGLint = 0x3031;
const EGL_SAMPLE_BUFFERS: EGLint = 0x3032;
const EGL_SURFACE_TYPE: EGLint = 0x3033;
pub(crate) const EGL_NONE: EGLint = 0x3038;
const EGL_RENDERABLE_TYPE: EGLint = 0x3040;
const EGL_DONT_CARE: EGLint = -1;
const EGL_PBUFFER_BIT: EGLint = 0x0001;
const EGL_OPENGL_ES_BIT: EGLint = 0x0001;
const EGL_OPENGL_ES2_BIT: EGLint = 0x0004;
const EGL_OPENGL_BIT: EGLint = 0x0008;
const EGL_VERSION: EGLint = 0x3054;
const EGL_EXTENSIONS: EGLint = 0x3055;
const EGL_HEIGHT: EGLint = 0x3056;
const EGL_WIDTH: EGLint = 0x3057;
const EGL_GL_COLORSPACE_SRGB_KHR: EGLint = 0x3089;
const EGL_GL_COLORSPACE_LINEAR_KHR: EGLint = 0x308A;
const EGL_CONTEXT_CLIENT_VERSION: EGLint = 0x3098;
const EGL_GL_COLORSPACE_KHR: EGLint = 0x309D;
const EGL_OPENGL_ES_API: EGLenum = 0x30A0;
const EGL_OPENGL_API: EGLenum = 0x30A2;
const EGL_CONTEXT_MAJOR_VERSION_KHR: EGLint = 0x3098;
const EGL_CONTEXT_MINOR_VERSION_KHR: EGLint = 0x30FB;
const EGL_CONTEXT_FLAGS_KHR: EGLint = 0x30FC;
const EGL_CONTEXT_OPENGL_PROFILE_MASK_KHR: EGLint = 0x30FD;
const EGL_CONTEXT_OPENGL_NO_ERROR_KHR: EGLint = 0x31B3;
/// EGL_OPENGL_ES3_BIT_KHR was added in version 13 of the extension.
const EGL_OPENGL_ES3_BIT_KHR: EGLint = 0x00000040;
const EGL_COLOR_COMPONENT_TYPE_EXT: EGLint = 0x3339;
const EGL_COLOR_COMPONENT_TYPE_FLOAT_EXT: EGLint = 0x333B;
const EGL_PLATFORM_DEVICE_EXT: EGLenum = 0x313F;
const EGL_PRESENT_OPAQUE_EXT: EGLint = 0x31DF;

/// `SDL_EGL_MAX_DEVICES`.
const SDL_EGL_MAX_DEVICES: usize = 8;

// The libraries tried, in order (the DEFAULT_*/ALT_* of the platform)
#[cfg(windows)]
mod paths {
    // EGL AND OpenGL ES support via ANGLE
    pub(super) const DEFAULT_EGL: &str = "libEGL.dll";
    pub(super) const DEFAULT_OGL: Option<&str> = Some("opengl32.dll");
    pub(super) const ALT_OGL: Option<&str> = None;
    pub(super) const DEFAULT_OGL_ES2: &str = "libGLESv2.dll";
    pub(super) const DEFAULT_OGL_ES_PVR: &str = "libGLES_CM.dll";
    pub(super) const DEFAULT_OGL_ES: &str = "libGLESv1_CM.dll";
}
#[cfg(target_os = "openbsd")]
mod paths {
    // OpenBSD
    pub(super) const DEFAULT_OGL: Option<&str> = Some("libGL.so");
    pub(super) const ALT_OGL: Option<&str> = None;
    pub(super) const DEFAULT_EGL: &str = "libEGL.so";
    pub(super) const DEFAULT_OGL_ES2: &str = "libGLESv2.so";
    pub(super) const DEFAULT_OGL_ES_PVR: &str = "libGLES_CM.so";
    pub(super) const DEFAULT_OGL_ES: &str = "libGLESv1_CM.so";
}
#[cfg(not(any(windows, target_os = "openbsd")))]
mod paths {
    // Desktop Linux/Unix-like
    pub(super) const DEFAULT_OGL: Option<&str> = Some("libGL.so.1");
    pub(super) const DEFAULT_EGL: &str = "libEGL.so.1";
    pub(super) const ALT_OGL: Option<&str> = Some("libOpenGL.so.0");
    pub(super) const DEFAULT_OGL_ES2: &str = "libGLESv2.so.2";
    pub(super) const DEFAULT_OGL_ES_PVR: &str = "libGLES_CM.so.1";
    pub(super) const DEFAULT_OGL_ES: &str = "libGLESv1_CM.so.1";
}
use paths::*;

// The function types (EGLAPIENTRYP is __stdcall on 32-bit Windows)
type PfnEglGetDisplay = unsafe extern "system" fn(NativeDisplayType) -> EGLDisplay;
type PfnEglInitialize =
    unsafe extern "system" fn(EGLDisplay, *mut EGLint, *mut EGLint) -> EGLBoolean;
type PfnEglTerminate = unsafe extern "system" fn(EGLDisplay) -> EGLBoolean;
type PfnEglGetProcAddress = unsafe extern "system" fn(*const c_char) -> *mut c_void;
type PfnEglChooseConfig = unsafe extern "system" fn(
    EGLDisplay,
    *const EGLint,
    *mut EGLConfig,
    EGLint,
    *mut EGLint,
) -> EGLBoolean;
type PfnEglCreateContext =
    unsafe extern "system" fn(EGLDisplay, EGLConfig, EGLContext, *const EGLint) -> EGLContext;
type PfnEglDestroyContext = unsafe extern "system" fn(EGLDisplay, EGLContext) -> EGLBoolean;
type PfnEglCreatePbufferSurface =
    unsafe extern "system" fn(EGLDisplay, EGLConfig, *const EGLint) -> EGLSurface;
type PfnEglCreateWindowSurface =
    unsafe extern "system" fn(EGLDisplay, EGLConfig, NativeWindowType, *const EGLint) -> EGLSurface;
type PfnEglDestroySurface = unsafe extern "system" fn(EGLDisplay, EGLSurface) -> EGLBoolean;
type PfnEglMakeCurrent =
    unsafe extern "system" fn(EGLDisplay, EGLSurface, EGLSurface, EGLContext) -> EGLBoolean;
type PfnEglSwapBuffers = unsafe extern "system" fn(EGLDisplay, EGLSurface) -> EGLBoolean;
type PfnEglSwapInterval = unsafe extern "system" fn(EGLDisplay, EGLint) -> EGLBoolean;
type PfnEglQueryString = unsafe extern "system" fn(EGLDisplay, EGLint) -> *const c_char;
type PfnEglGetConfigAttrib =
    unsafe extern "system" fn(EGLDisplay, EGLConfig, EGLint, *mut EGLint) -> EGLBoolean;
type PfnEglWaitNative = unsafe extern "system" fn(EGLint) -> EGLBoolean;
type PfnEglWaitGl = unsafe extern "system" fn() -> EGLBoolean;
type PfnEglBindApi = unsafe extern "system" fn(EGLenum) -> EGLBoolean;
type PfnEglGetError = unsafe extern "system" fn() -> EGLint;
type PfnEglQueryDevicesExt =
    unsafe extern "system" fn(EGLint, *mut EGLDeviceEXT, *mut EGLint) -> EGLBoolean;
type PfnEglGetPlatformDisplay =
    unsafe extern "system" fn(EGLenum, *mut c_void, *const EGLAttrib) -> EGLDisplay;
type PfnEglGetPlatformDisplayExt =
    unsafe extern "system" fn(EGLenum, *mut c_void, *const EGLint) -> EGLDisplay;
type EGLSyncKHR = *mut c_void;
type PfnEglCreateSyncKhr =
    unsafe extern "system" fn(EGLDisplay, EGLenum, *const EGLint) -> EGLSyncKHR;
type PfnEglDestroySyncKhr = unsafe extern "system" fn(EGLDisplay, EGLSyncKHR) -> EGLBoolean;
type PfnEglDupNativeFenceFdAndroid = unsafe extern "system" fn(EGLDisplay, EGLSyncKHR) -> EGLint;
type PfnEglWaitSyncKhr = unsafe extern "system" fn(EGLDisplay, EGLSyncKHR, EGLint) -> EGLint;
type PfnEglClientWaitSyncKhr =
    unsafe extern "system" fn(EGLDisplay, EGLSyncKHR, EGLint, u64) -> EGLint;

/// The EGL entry points of `SDL_EGL_VideoData`.
#[derive(Clone, Copy)]
#[allow(non_snake_case, dead_code)] // (the EGL names; some are loaded but unused, as upstream)
pub(crate) struct EglFns {
    eglGetDisplay: PfnEglGetDisplay,
    eglInitialize: PfnEglInitialize,
    eglTerminate: PfnEglTerminate,
    eglGetProcAddress: PfnEglGetProcAddress,
    eglChooseConfig: PfnEglChooseConfig,
    eglCreateContext: PfnEglCreateContext,
    eglDestroyContext: PfnEglDestroyContext,
    eglCreatePbufferSurface: PfnEglCreatePbufferSurface,
    eglCreateWindowSurface: PfnEglCreateWindowSurface,
    eglDestroySurface: PfnEglDestroySurface,
    eglMakeCurrent: PfnEglMakeCurrent,
    eglSwapBuffers: PfnEglSwapBuffers,
    eglSwapInterval: PfnEglSwapInterval,
    eglQueryString: PfnEglQueryString,
    pub(crate) eglGetConfigAttrib: PfnEglGetConfigAttrib,
    eglWaitNative: PfnEglWaitNative,
    eglWaitGL: PfnEglWaitGl,
    eglBindAPI: PfnEglBindApi,
    eglGetError: PfnEglGetError,
    eglQueryDevicesEXT: Option<PfnEglQueryDevicesExt>,
    eglGetPlatformDisplay: Option<PfnEglGetPlatformDisplay>,
    eglGetPlatformDisplayEXT: Option<PfnEglGetPlatformDisplayExt>,

    // Atomic functions
    eglCreateSyncKHR: Option<PfnEglCreateSyncKhr>,
    eglDestroySyncKHR: Option<PfnEglDestroySyncKhr>,
    eglDupNativeFenceFDANDROID: Option<PfnEglDupNativeFenceFdAndroid>,
    eglWaitSyncKHR: Option<PfnEglWaitSyncKhr>,
    eglClientWaitSyncKHR: Option<PfnEglClientWaitSyncKhr>,
    // Atomic functions end
}

/// The libraries the functions come from (`opengl_dll_handle`,
/// `egl_dll_handle`), unloaded when the last user lets go.
struct EglLibraries {
    opengl_dll_handle: Option<SharedObject>,
    #[allow(dead_code)] // (kept loaded for the functions)
    egl_dll_handle: Option<SharedObject>,
}

/// The device's EGL state. Translation of `SDL_EGL_VideoData`; a copy is
/// what the entry points work on.
#[derive(Clone)]
pub(crate) struct Egl {
    libraries: Arc<EglLibraries>,
    pub(crate) egl_display: EGLDisplay,
    pub(crate) egl_config: EGLConfig,
    egl_swapinterval: i32,
    egl_surfacetype: i32,
    egl_version_major: i32,
    egl_version_minor: i32,
    egl_required_visual_id: EGLint,
    /// whether EGL display was offscreen
    is_offscreen: bool,
    /// EGL_OPENGL_ES_API, EGL_OPENGL_API, etc
    apitype: EGLenum,
    pub(crate) f: EglFns,
}

// SAFETY: the EGL handles are process-wide tokens EGL accepts from any
// thread; the function pointers are plain code addresses.
unsafe impl Send for Egl {}

/// `_this->egl_data` (`None`: NULL).
static EGL_DATA: Mutex<Option<Egl>> = Mutex::new(None);

fn egl_data() -> std::sync::MutexGuard<'static, Option<Egl>> {
    EGL_DATA.lock().unwrap_or_else(|e| e.into_inner())
}

/// A copy of `_this->egl_data`, if EGL is loaded.
pub(crate) fn egl() -> Option<Egl> {
    egl_data().clone()
}

/// Change `_this->egl_data` (nothing if EGL isn't loaded).
fn with_egl<R>(f: impl FnOnce(&mut Egl) -> R) -> Option<R> {
    egl_data().as_mut().map(f)
}

/// Whether EGL is loaded (`_this->egl_data != NULL`).
pub(crate) fn is_loaded() -> bool {
    egl_data().is_some()
}

/// `egl_data->egl_display` (`None` if EGL isn't loaded; `Some(None)` for
/// `EGL_NO_DISPLAY`).
pub(crate) fn current_display() -> Option<Option<EglDisplay>> {
    egl_data()
        .as_ref()
        .map(|e| EglDisplay::from_ptr(e.egl_display))
}

/// `egl_data->egl_config` (`None` if EGL isn't loaded).
pub(crate) fn current_config() -> Option<Option<EglConfig>> {
    egl_data()
        .as_ref()
        .map(|e| EglConfig::from_ptr(e.egl_config))
}

/// Reset `gl_config.driver_loaded` and `driver_path`, as EGL does when it
/// fails partway.
fn reset_driver_loaded() {
    gl::set_driver_loaded(|_| 0);
    gl::set_driver_path(None);
}

/// Translation of `SDL_EGL_GetErrorName()`.
fn egl_get_error_name(egl_error_code: EGLint) -> &'static str {
    match egl_error_code {
        EGL_SUCCESS => "EGL_SUCCESS",
        EGL_NOT_INITIALIZED => "EGL_NOT_INITIALIZED",
        EGL_BAD_ACCESS => "EGL_BAD_ACCESS",
        EGL_BAD_ALLOC => "EGL_BAD_ALLOC",
        EGL_BAD_ATTRIBUTE => "EGL_BAD_ATTRIBUTE",
        EGL_BAD_CONTEXT => "EGL_BAD_CONTEXT",
        EGL_BAD_CONFIG => "EGL_BAD_CONFIG",
        EGL_BAD_CURRENT_SURFACE => "EGL_BAD_CURRENT_SURFACE",
        EGL_BAD_DISPLAY => "EGL_BAD_DISPLAY",
        EGL_BAD_SURFACE => "EGL_BAD_SURFACE",
        EGL_BAD_MATCH => "EGL_BAD_MATCH",
        EGL_BAD_PARAMETER => "EGL_BAD_PARAMETER",
        EGL_BAD_NATIVE_PIXMAP => "EGL_BAD_NATIVE_PIXMAP",
        EGL_BAD_NATIVE_WINDOW => "EGL_BAD_NATIVE_WINDOW",
        EGL_CONTEXT_LOST => "EGL_CONTEXT_LOST",
        _ => "",
    }
}

/// The error for a failed EGL call. Translation of `SDL_EGL_SetErrorEx()`.
pub(crate) fn egl_set_error_ex(
    message: &str,
    egl_function_name: &str,
    egl_error_code: EGLint,
) -> Error {
    let mut error_text = egl_get_error_name(egl_error_code).to_owned();
    if error_text.is_empty() {
        // An unknown-to-SDL error code was reported.  Report its hexadecimal value, instead of its name.
        error_text = format!("0x{:x}", egl_error_code as u32);
    }
    Error::new(format!(
        "{message} (call to {egl_function_name} failed, reporting an error of {error_text})"
    ))
}

impl Egl {
    /// Translation of `SDL_EGL_SetError()`: the error for a failed call,
    /// with `eglGetError()`.
    fn set_error(&self, message: &str, egl_function_name: &str) -> Error {
        // SAFETY: eglGetError has no preconditions.
        let code = unsafe { (self.f.eglGetError)() };
        egl_set_error_ex(message, egl_function_name, code)
    }

    /// `eglQueryString(display, name)` as a Rust string.
    fn query_string(&self, display: EGLDisplay, name: EGLint) -> Option<String> {
        // SAFETY: eglQueryString accepts any display (failing with NULL
        // for a bad one); the result is NULL or a NUL-terminated string
        // owned by EGL.
        unsafe {
            let s = (self.f.eglQueryString)(display, name);
            (!s.is_null()).then(|| CStr::from_ptr(s).to_string_lossy().into_owned())
        }
    }

    /// `eglGetConfigAttrib()`, keeping `value` when it fails (as upstream's
    /// out-parameter is).
    pub(crate) fn get_config_attrib(
        &self,
        config: EGLConfig,
        attribute: EGLint,
        value: &mut EGLint,
    ) -> EGLBoolean {
        // SAFETY: the display and config are EGL's (invalid ones fail with
        // an error); value is a valid out-parameter.
        unsafe { (self.f.eglGetConfigAttrib)(self.egl_display, config, attribute, value) }
    }
}

/// Which extension string to search.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ExtensionType {
    /// `SDL_EGL_DISPLAY_EXTENSION`
    Display,
    /// `SDL_EGL_CLIENT_EXTENSION`
    Client,
}

/// Whether `ext` is a whole word of `egl_extstr` (the search loop of
/// `SDL_EGL_HasExtension()`).
fn extension_in_string(egl_extstr: &str, ext: &str) -> bool {
    let s = egl_extstr.as_bytes();
    let ext = ext.as_bytes();
    let ext_len = ext.len();
    let mut ext_start = 0;

    while ext_start < s.len() {
        let Some(pos) = s[ext_start..].windows(ext_len).position(|w| w == ext) else {
            return false;
        };
        ext_start += pos;
        // Check if the match is not just a substring of one of the extensions
        if ext_start == 0 || s[ext_start - 1] == b' ' {
            let end = ext_start + ext_len;
            if end == s.len() || s[end] == b' ' {
                return true;
            }
        }
        // If the search stopped in the middle of an extension, skip to the end of it
        ext_start += ext_len;
        while ext_start < s.len() && s[ext_start] != b' ' {
            ext_start += 1;
        }
    }

    false
}

/// Whether the display or client extension string has `ext`; the hint (or
/// environment variable) named like the extension masks it: bit 0 the
/// display extension, bit 1 the client extension. Translation of
/// `SDL_EGL_HasExtension()`.
pub(crate) fn has_extension(e: &Egl, ext_type: ExtensionType, ext: &str) -> bool {
    // Invalid extensions can be rejected early
    if ext.is_empty() || ext.contains(' ') {
        // SDL_LogDebug(SDL_LOG_CATEGORY_VIDEO, "SDL_EGL_HasExtension: Invalid EGL extension");
        return false;
    }

    /* Extensions can be masked with a hint or environment variable.
     * Unlike the OpenGL override, this will use the set bits of an integer
     * to disable the extension.
     *  Bit   Action
     *  0     If set, the display extension is masked and not present to SDL.
     *  1     If set, the client extension is masked and not present to SDL.
     */
    if let Some(ext_override) = hints::get(ext) {
        let disable_ext = crate::stdlib::atoi(&ext_override);
        if (disable_ext & 0x01 != 0 && ext_type == ExtensionType::Display)
            || (disable_ext & 0x02 != 0 && ext_type == ExtensionType::Client)
        {
            return false;
        }
    }

    let egl_extstr = match ext_type {
        ExtensionType::Display => e.query_string(e.egl_display, EGL_EXTENSIONS),
        /* EGL_EXT_client_extensions modifies eglQueryString to return client extensions
         * if EGL_NO_DISPLAY is passed. Implementations without it are required to return NULL.
         * This behavior is included in EGL 1.5.
         */
        ExtensionType::Client => e.query_string(EGL_NO_DISPLAY, EGL_EXTENSIONS),
    };

    egl_extstr.is_some_and(|s| extension_in_string(&s, ext))
}

/// An EGL or GL function by name. Translation of
/// `SDL_EGL_GetProcAddressInternal()`.
pub(crate) fn get_proc_address_internal(proc_name: &str) -> Option<NonNull<c_void>> {
    let e = egl()?;
    let cname = CString::new(proc_name).ok()?;
    let eglver = ((e.egl_version_major as u32) << 16) | (e.egl_version_minor as u32);
    let is_egl_15_or_later = eglver >= ((1u32 << 16) | 5);
    let mut result: Option<NonNull<c_void>> = None;

    // EGL 1.5 can use eglGetProcAddress() for any symbol. 1.4 and earlier can't use it for core entry points.
    if is_egl_15_or_later {
        // SAFETY: the name is NUL-terminated.
        result = NonNull::new(unsafe { (e.f.eglGetProcAddress)(cname.as_ptr()) });
    }

    // Try SDL_LoadFunction() first for EGL <= 1.4, or as a fallback for >= 1.5.
    if result.is_none() {
        result = e
            .libraries
            .opengl_dll_handle
            .as_ref()
            .and_then(|lib| lib.symbol(proc_name).ok());
    }

    // Try eglGetProcAddress if we're on <= 1.4 and still searching...
    if result.is_none() && !is_egl_15_or_later {
        // SAFETY: as above.
        result = NonNull::new(unsafe { (e.f.eglGetProcAddress)(cname.as_ptr()) });
    }
    result
}

/// Translation of `SDL_EGL_UnloadLibrary()`.
pub(crate) fn unload_library() {
    let Some(e) = egl_data().take() else {
        return;
    };
    if !e.egl_display.is_null() {
        // SAFETY: the display was initialized by this EGL.
        unsafe { (e.f.eglTerminate)(e.egl_display) };
    }
    // (the libraries are unloaded when the last copy is dropped)
}

/// Load `name` with `SDL_LoadObject()`, `None` on failure.
fn load_object(name: &str) -> Option<SharedObject> {
    SharedObject::load(name).ok()
}

/// Translation of `SDL_EGL_LoadLibraryInternal()`.
fn load_library_internal(egl_path: Option<&str>) -> Result<Egl> {
    #[cfg(windows)]
    {
        // (the compiler stays loaded: upstream never unloads it)
        match hints::get(hints::VIDEO_WIN_D3DCOMPILER) {
            Some(d3dcompiler) => {
                if !d3dcompiler.eq_ignore_ascii_case("none") {
                    if let Some(lib) = load_object(&d3dcompiler) {
                        std::mem::forget(lib);
                    }
                }
            }
            None => {
                if crate::core::windows::is_windows_vista_or_greater() {
                    // Try the newer d3d compilers first
                    let d3dcompiler_list = ["d3dcompiler_47.dll", "d3dcompiler_46.dll"];
                    for name in d3dcompiler_list {
                        if let Some(lib) = load_object(name) {
                            std::mem::forget(lib);
                            break;
                        }
                    }
                } else if let Some(lib) = load_object("d3dcompiler_43.dll") {
                    std::mem::forget(lib);
                }
            }
        }
    }

    /* A funny thing, loading EGL.so first does not work on the Raspberry, so we load libGL* first */
    let mut path: Option<String> = hints::get(hints::OPENGL_LIBRARY);
    let mut opengl_dll_handle = path.as_deref().and_then(load_object);

    if opengl_dll_handle.is_none() {
        let config = gl_config();
        let candidates: Vec<&str> = if config.profile_mask == GL_CONTEXT_PROFILE_ES {
            if config.major_version > 1 {
                vec![DEFAULT_OGL_ES2]
            } else {
                vec![DEFAULT_OGL_ES, DEFAULT_OGL_ES_PVR]
            }
        } else {
            DEFAULT_OGL.into_iter().chain(ALT_OGL).collect()
        };
        for candidate in candidates {
            path = Some(candidate.to_owned());
            opengl_dll_handle = load_object(candidate);
            if opengl_dll_handle.is_some() {
                break;
            }
        }
    }

    let Some(opengl_dll_handle) = opengl_dll_handle else {
        return Err(Error::new("Could not initialize OpenGL / GLES library"));
    };
    // Note (upstream): SDL_EGL_LoadLibraryOnly() frees egl_data without
    // unloading the GL library when loading EGL fails; it is unloaded here.

    /* Loading libGL* in the previous step took care of loading libEGL.so, but we future proof by double checking */
    let mut egl_dll_handle = egl_path.and_then(load_object);
    // Try loading a EGL symbol, if it does not work try the default library paths
    if egl_dll_handle
        .as_ref()
        .is_none_or(|lib| lib.symbol("eglChooseConfig").is_err())
    {
        drop(egl_dll_handle.take());
        let p = hints::get(hints::EGL_LIBRARY).unwrap_or_else(|| DEFAULT_EGL.to_owned());
        egl_dll_handle = load_object(&p);
        path = Some(p);

        if egl_dll_handle
            .as_ref()
            .is_none_or(|lib| lib.symbol("eglChooseConfig").is_err())
        {
            return Err(Error::new("Could not load EGL library"));
        }
    }
    let egl_dll_handle = egl_dll_handle.expect("checked above");

    // Load new function pointers
    macro_rules! load_func {
        ($name:ident) => {
            // SAFETY: the EGL function of this name has this type.
            match unsafe { egl_dll_handle.function(stringify!($name)) } {
                Ok(f) => f,
                Err(_) => {
                    return Err(Error::new(concat!(
                        "Could not retrieve EGL function ",
                        stringify!($name)
                    )))
                }
            }
        };
    }
    let egl_get_proc_address: PfnEglGetProcAddress = load_func!(eglGetProcAddress);
    // it is allowed to not have some of the EGL extensions on start - attempts to use them will fail later.
    macro_rules! load_func_eglext {
        ($t:ty, $name:ident) => {{
            let cname = concat!(stringify!($name), "\0");
            // SAFETY: the name is NUL-terminated.
            let p = unsafe { egl_get_proc_address(cname.as_ptr().cast()) };
            if p.is_null() {
                None
            } else {
                // SAFETY: eglGetProcAddress returned this extension
                // function, whose type this is.
                Some(unsafe { std::mem::transmute::<*mut c_void, $t>(p) })
            }
        }};
    }
    let f = EglFns {
        eglGetDisplay: load_func!(eglGetDisplay),
        eglInitialize: load_func!(eglInitialize),
        eglTerminate: load_func!(eglTerminate),
        eglGetProcAddress: egl_get_proc_address,
        eglChooseConfig: load_func!(eglChooseConfig),
        eglCreateContext: load_func!(eglCreateContext),
        eglDestroyContext: load_func!(eglDestroyContext),
        eglCreatePbufferSurface: load_func!(eglCreatePbufferSurface),
        eglCreateWindowSurface: load_func!(eglCreateWindowSurface),
        eglDestroySurface: load_func!(eglDestroySurface),
        eglMakeCurrent: load_func!(eglMakeCurrent),
        eglSwapBuffers: load_func!(eglSwapBuffers),
        eglSwapInterval: load_func!(eglSwapInterval),
        eglQueryString: load_func!(eglQueryString),
        eglGetConfigAttrib: load_func!(eglGetConfigAttrib),
        eglWaitNative: load_func!(eglWaitNative),
        eglWaitGL: load_func!(eglWaitGL),
        eglBindAPI: load_func!(eglBindAPI),
        eglGetError: load_func!(eglGetError),
        eglQueryDevicesEXT: load_func_eglext!(PfnEglQueryDevicesExt, eglQueryDevicesEXT),
        eglGetPlatformDisplay: None,
        eglGetPlatformDisplayEXT: load_func_eglext!(
            PfnEglGetPlatformDisplayExt,
            eglGetPlatformDisplayEXT
        ),
        // Atomic functions
        eglCreateSyncKHR: load_func_eglext!(PfnEglCreateSyncKhr, eglCreateSyncKHR),
        eglDestroySyncKHR: load_func_eglext!(PfnEglDestroySyncKhr, eglDestroySyncKHR),
        eglDupNativeFenceFDANDROID: load_func_eglext!(
            PfnEglDupNativeFenceFdAndroid,
            eglDupNativeFenceFDANDROID
        ),
        eglWaitSyncKHR: load_func_eglext!(PfnEglWaitSyncKhr, eglWaitSyncKHR),
        eglClientWaitSyncKHR: load_func_eglext!(PfnEglClientWaitSyncKhr, eglClientWaitSyncKHR),
        // Atomic functions end
    };

    // (path is never NULL here)
    gl::set_driver_path(path.as_deref());

    Ok(Egl {
        libraries: Arc::new(EglLibraries {
            opengl_dll_handle: Some(opengl_dll_handle),
            egl_dll_handle: Some(egl_dll_handle),
        }),
        egl_display: EGL_NO_DISPLAY,
        egl_config: std::ptr::null_mut(),
        egl_swapinterval: 0,
        egl_surfacetype: 0,
        egl_version_major: 0,
        egl_version_minor: 0,
        egl_required_visual_id: 0,
        is_offscreen: false,
        apitype: 0,
        f,
    })
}

/// Load the libraries without a display. Translation of
/// `SDL_EGL_LoadLibraryOnly()`.
pub(crate) fn load_library_only(egl_path: Option<&str>) -> Result<()> {
    if is_loaded() {
        return Err(Error::new("EGL context already created"));
    }

    let e = load_library_internal(egl_path)?;
    *egl_data() = Some(e);
    Ok(())
}

/// The major and minor version of an EGL version string (`sscanf(s,
/// "%d.%d")`).
fn parse_egl_version(egl_version: &str) -> Option<(i32, i32)> {
    let s = egl_version.as_bytes();
    let (major, n) = crate::stdlib::string::strtol(s, 10);
    if n == 0 || s.get(n) != Some(&b'.') {
        return None;
    }
    let (minor, m) = crate::stdlib::string::strtol(&s[n + 1..], 10);
    if m == 0 {
        return None;
    }
    Some((major as i32, minor as i32))
}

/// Translation of `SDL_EGL_GetVersion()`.
fn egl_get_version() {
    let Some(e) = egl() else { return };
    if let Some(egl_version) = e.query_string(e.egl_display, EGL_VERSION) {
        match parse_egl_version(&egl_version) {
            Some((major, minor)) => {
                with_egl(|e| {
                    e.egl_version_major = major;
                    e.egl_version_minor = minor;
                });
            }
            None => crate::warn!(
                crate::log::Category::Video,
                "Could not parse EGL version string: {}",
                egl_version
            ),
        }
    }
}

/// Copy a callback's attribute list up to its `EGL_NONE` key (or its end)
/// and terminate it.
fn terminated_pairs<T: Copy + PartialEq>(list: &[T], none: T) -> Vec<T> {
    let mut out = Vec::with_capacity(list.len() + 1);
    let mut pairs = list.chunks(2);
    for pair in pairs.by_ref() {
        if pair[0] == none || pair.len() < 2 {
            break;
        }
        out.extend_from_slice(pair);
    }
    out.push(none);
    out
}

/// Load EGL and initialize a display for `native_display`. Translation of
/// `SDL_EGL_LoadLibrary()`.
pub(crate) fn load_library(
    egl_path: Option<&str>,
    native_display: NativeDisplayType,
) -> Result<()> {
    load_library_only(egl_path)?;

    with_egl(|e| e.egl_display = EGL_NO_DISPLAY);

    let platform = gl_config().egl_platform as EGLenum;
    if platform != 0 {
        /* EGL 1.5 allows querying for client version with EGL_NO_DISPLAY
         * --
         * Khronos doc: "EGL_BAD_DISPLAY is generated if display is not an EGL display connection, unless display is EGL_NO_DISPLAY and name is EGL_EXTENSIONS."
         * Therefore SDL_EGL_GetVersion() shouldn't work with uninitialized display.
         * - it actually doesn't work on Android that has 1.5 egl client
         * - it works on desktop X11 (using SDL_VIDEO_FORCE_EGL=1) */
        egl_get_version();

        let e = egl().expect("loaded above");
        if e.egl_version_major == 1 && e.egl_version_minor == 5 {
            let f = e
                .libraries
                .egl_dll_handle
                .as_ref()
                // SAFETY: eglGetPlatformDisplay has this type.
                .and_then(|lib| unsafe { lib.function("eglGetPlatformDisplay") }.ok());
            match f {
                Some(f) => {
                    with_egl(|e| e.f.eglGetPlatformDisplay = Some(f));
                }
                None => {
                    return Err(Error::new(
                        "Could not retrieve EGL function eglGetPlatformDisplay",
                    ))
                }
            }
        }

        let e = egl().expect("loaded above");
        if let Some(get_platform_display) = e.f.eglGetPlatformDisplay {
            let mut attribs: Option<Vec<EGLAttrib>> = None;
            let callbacks = gl::egl_attrib_callbacks();
            if let Some(callback) = callbacks.as_ref().and_then(|c| c.platform.as_ref()) {
                match callback() {
                    Some(list) => {
                        // Note (upstream): the list is read up to EGL_NONE
                        // or its end, never past it.
                        attribs = Some(terminated_pairs(&list, EGL_NONE as EGLAttrib));
                    }
                    None => {
                        reset_driver_loaded();
                        return Err(Error::new(
                            "EGL platform attribute callback returned NULL pointer",
                        ));
                    }
                }
            }
            // SAFETY: the native display is the driver's; the attribute
            // list is NULL or EGL_NONE-terminated.
            let display = unsafe {
                get_platform_display(
                    platform,
                    native_display,
                    attribs.as_ref().map_or(std::ptr::null(), |a| a.as_ptr()),
                )
            };
            with_egl(|e| e.egl_display = display);
        } else if has_extension(&e, ExtensionType::Client, "EGL_EXT_platform_base") {
            let f = get_proc_address_internal("eglGetPlatformDisplayEXT").map(|p| {
                // SAFETY: the function of this name has this type.
                unsafe {
                    std::mem::transmute::<*mut c_void, PfnEglGetPlatformDisplayExt>(p.as_ptr())
                }
            });
            with_egl(|e| e.f.eglGetPlatformDisplayEXT = f);
            if let Some(f) = f {
                // SAFETY: as above, with no attributes.
                let display = unsafe { f(platform, native_display, std::ptr::null()) };
                with_egl(|e| e.egl_display = display);
            }
        }
    }

    let e = egl().expect("loaded above");
    let mut display = e.egl_display;
    // Try the implementation-specific eglGetDisplay even if eglGetPlatformDisplay fails
    if display == EGL_NO_DISPLAY
        && hints::get_bool(hints::VIDEO_EGL_ALLOW_GETDISPLAY_FALLBACK, true)
    {
        // SAFETY: the native display is the driver's.
        display = unsafe { (e.f.eglGetDisplay)(native_display) };
        with_egl(|e| e.egl_display = display);
    }
    if display == EGL_NO_DISPLAY {
        reset_driver_loaded();
        return Err(Error::new("Could not get EGL display"));
    }

    // SAFETY: the display came from EGL; the version out-parameters may be NULL.
    if unsafe { (e.f.eglInitialize)(display, std::ptr::null_mut(), std::ptr::null_mut()) }
        != EGL_TRUE
    {
        reset_driver_loaded();
        return Err(Error::new("Could not initialize EGL"));
    }

    // Get the EGL version with a valid egl_display, for EGL <= 1.4
    egl_get_version();

    with_egl(|e| e.is_offscreen = false);

    Ok(())
}

/// Initialize a display on an EGL device (for offscreen rendering), after
/// [`load_library_only`]. Translation of `SDL_EGL_InitializeOffscreen()`.
///
/// On multi GPU machines EGL device 0 is not always the first valid GPU.
/// Container environments can restrict access to some GPUs that are still listed in the EGL
/// device list. If the requested device is a restricted GPU and cannot be used
/// (eglInitialize() will fail) then attempt to automatically and silently select the next
/// valid available GPU for EGL to use.
#[allow(dead_code)] // (for the offscreen driver's EGL support, not translated yet)
pub(crate) fn initialize_offscreen(device: i32) -> Result<()> {
    if gl::driver_loaded() <= 0 {
        return Err(Error::new(
            "SDL_EGL_LoadLibraryOnly() has not been called or has failed.",
        ));
    }
    let Some(e) = egl() else {
        return Err(Error::new("EGL not initialized"));
    };

    // Check for all extensions that are optional until used and fail if any is missing
    let Some(query_devices) = e.f.eglQueryDevicesEXT else {
        return Err(Error::new(
            "eglQueryDevicesEXT is missing (EXT_device_enumeration not supported by the drivers?)",
        ));
    };

    let Some(get_platform_display_ext) = e.f.eglGetPlatformDisplayEXT else {
        return Err(Error::new(
            "eglGetPlatformDisplayEXT is missing (EXT_platform_base not supported by the drivers?)",
        ));
    };

    let mut egl_devices: [EGLDeviceEXT; SDL_EGL_MAX_DEVICES] =
        [std::ptr::null_mut(); SDL_EGL_MAX_DEVICES];
    let mut num_egl_devices: EGLint = 0;
    // SAFETY: the array holds SDL_EGL_MAX_DEVICES devices.
    if unsafe {
        query_devices(
            SDL_EGL_MAX_DEVICES as EGLint,
            egl_devices.as_mut_ptr(),
            &mut num_egl_devices,
        )
    } != EGL_TRUE
    {
        return Err(Error::new("eglQueryDevicesEXT() failed"));
    }
    let num_egl_devices = num_egl_devices.clamp(0, SDL_EGL_MAX_DEVICES as EGLint);

    if let Some(egl_device_hint) = hints::get("SDL_HINT_EGL_DEVICE") {
        let device = crate::stdlib::atoi(&egl_device_hint);

        // FIXME (upstream): a negative device indexes before the array; it
        // is rejected here like one past the end.
        if device >= num_egl_devices || device < 0 {
            return Err(Error::new("Invalid EGL device is requested."));
        }

        // SAFETY: the device came from eglQueryDevicesEXT.
        let display = unsafe {
            get_platform_display_ext(
                EGL_PLATFORM_DEVICE_EXT,
                egl_devices[device as usize],
                std::ptr::null(),
            )
        };
        with_egl(|e| e.egl_display = display);

        if display == EGL_NO_DISPLAY {
            return Err(Error::new("eglGetPlatformDisplayEXT() failed."));
        }

        // SAFETY: the display came from EGL.
        if unsafe { (e.f.eglInitialize)(display, std::ptr::null_mut(), std::ptr::null_mut()) }
            != EGL_TRUE
        {
            return Err(Error::new("Could not initialize EGL"));
        }
    } else {
        // FIXME (upstream): without the hint, the `device` argument is
        // ignored and the first device that initializes is used.
        let _ = device;
        let mut found = false;

        // If no hint is provided lets look for the first device/display that will allow us to eglInit
        for &egl_device in &egl_devices[..num_egl_devices as usize] {
            // SAFETY: the device came from eglQueryDevicesEXT.
            let attempted_egl_display = unsafe {
                get_platform_display_ext(EGL_PLATFORM_DEVICE_EXT, egl_device, std::ptr::null())
            };

            if attempted_egl_display == EGL_NO_DISPLAY {
                continue;
            }

            // SAFETY: the display came from EGL.
            unsafe {
                if (e.f.eglInitialize)(
                    attempted_egl_display,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                ) != EGL_TRUE
                {
                    (e.f.eglTerminate)(attempted_egl_display);
                    continue;
                }
            }

            // We did not fail, we'll pick this one!
            with_egl(|e| e.egl_display = attempted_egl_display);
            found = true;

            break;
        }

        if !found {
            return Err(Error::new(
                "Could not find a valid EGL device to initialize",
            ));
        }
    }

    // Get the EGL version with a valid egl_display, for EGL <= 1.4
    egl_get_version();

    with_egl(|e| e.is_offscreen = true);

    Ok(())
}

/// Require configs of this native visual. Translation of
/// `SDL_EGL_SetRequiredVisualId()`.
#[allow(dead_code)] // (used by drivers that aren't translated yet)
pub(crate) fn set_required_visual_id(visual_id: i32) {
    with_egl(|e| e.egl_required_visual_id = visual_id);
}

/// The attributes `SDL_EGL_PrivateChooseConfig()` asks `eglChooseConfig`
/// for, and the API to bind.
fn choose_config_attribs(
    config: &GlConfig,
    set_config_caveat_none: bool,
    is_offscreen: bool,
    has_khr_create_context: bool,
    egl_surfacetype: i32,
) -> (Vec<EGLint>, EGLenum) {
    let mut attribs: Vec<EGLint> = Vec::with_capacity(64);

    // Get a valid EGL configuration
    attribs.extend([EGL_RED_SIZE, config.red_size]);
    attribs.extend([EGL_GREEN_SIZE, config.green_size]);
    attribs.extend([EGL_BLUE_SIZE, config.blue_size]);

    if set_config_caveat_none {
        attribs.extend([EGL_CONFIG_CAVEAT, EGL_NONE]);
    }

    if config.alpha_size != 0 {
        attribs.extend([EGL_ALPHA_SIZE, config.alpha_size]);
    }

    if config.buffer_size != 0 {
        attribs.extend([EGL_BUFFER_SIZE, config.buffer_size]);
    }

    if config.depth_size != 0 {
        attribs.extend([EGL_DEPTH_SIZE, config.depth_size]);
    }

    if config.stencil_size != 0 {
        attribs.extend([EGL_STENCIL_SIZE, config.stencil_size]);
    }

    if config.multisamplebuffers != 0 {
        attribs.extend([EGL_SAMPLE_BUFFERS, config.multisamplebuffers]);
    }

    if config.multisamplesamples != 0 {
        attribs.extend([EGL_SAMPLES, config.multisamplesamples]);
    }

    if config.floatbuffers != 0 {
        attribs.extend([
            EGL_COLOR_COMPONENT_TYPE_EXT,
            EGL_COLOR_COMPONENT_TYPE_FLOAT_EXT,
        ]);
    }

    if is_offscreen {
        attribs.extend([EGL_SURFACE_TYPE, EGL_PBUFFER_BIT]);
    }

    attribs.push(EGL_RENDERABLE_TYPE);
    let api = if config.profile_mask == GL_CONTEXT_PROFILE_ES {
        if config.major_version >= 3 && has_khr_create_context {
            attribs.push(EGL_OPENGL_ES3_BIT_KHR);
        } else if config.major_version >= 2 {
            attribs.push(EGL_OPENGL_ES2_BIT);
        } else {
            attribs.push(EGL_OPENGL_ES_BIT);
        }
        EGL_OPENGL_ES_API
    } else {
        attribs.push(EGL_OPENGL_BIT);
        EGL_OPENGL_API
    };

    if egl_surfacetype != 0 {
        attribs.extend([EGL_SURFACE_TYPE, egl_surfacetype]);
    }

    attribs.push(EGL_NONE);

    crate::sdl_assert!(attribs.len() < 64);
    (attribs, api)
}

/// From the configs `eglChooseConfig` found (which match or exceed the
/// requested attribs), the one that matches our requirements more closely
/// via a makeshift algorithm (the selection of
/// `SDL_EGL_PrivateChooseConfig()`). `get_attrib(i, attribute, &mut
/// value)` is `eglGetConfigAttrib()` for config `i`.
fn select_config(
    found_configs: usize,
    attribs: &[EGLint],
    egl_required_visual_id: EGLint,
    config: &GlConfig,
    mut get_attrib: impl FnMut(usize, EGLint, &mut EGLint) -> EGLBoolean,
) -> Option<usize> {
    let mut has_matching_format = false;
    let mut best_bitdiff = -1;
    let mut best_truecolor_bitdiff = -1;
    let mut truecolor_config_idx: Option<usize> = None;
    let mut chosen: Option<usize> = None;
    let mut value: EGLint = 0;

    // first ensure that a found config has a matching format, or the function will fall through.
    if egl_required_visual_id != 0 {
        for i in 0..found_configs {
            // Note (upstream): `format` is read uninitialized when
            // eglGetConfigAttrib fails; it starts at 0 here.
            let mut format: EGLint = 0;
            get_attrib(i, EGL_NATIVE_VISUAL_ID, &mut format);
            if egl_required_visual_id == format {
                has_matching_format = true;
                break;
            }
        }
    }

    for i in 0..found_configs {
        let mut is_truecolor = false;
        let mut bitdiff = 0;

        if has_matching_format && egl_required_visual_id != 0 {
            let mut format: EGLint = 0;
            get_attrib(i, EGL_NATIVE_VISUAL_ID, &mut format);
            if egl_required_visual_id != format {
                continue;
            }
        }

        get_attrib(i, EGL_RED_SIZE, &mut value);
        if value == 8 {
            get_attrib(i, EGL_GREEN_SIZE, &mut value);
            if value == 8 {
                get_attrib(i, EGL_BLUE_SIZE, &mut value);
                if value == 8 {
                    is_truecolor = true;
                }
            }
        }

        for pair in attribs.chunks(2) {
            if pair[0] == EGL_NONE || pair.len() < 2 {
                break;
            }

            if pair[1] != EGL_DONT_CARE
                && (pair[0] == EGL_RED_SIZE
                    || pair[0] == EGL_GREEN_SIZE
                    || pair[0] == EGL_BLUE_SIZE
                    || pair[0] == EGL_ALPHA_SIZE
                    || pair[0] == EGL_DEPTH_SIZE
                    || pair[0] == EGL_STENCIL_SIZE)
            {
                get_attrib(i, pair[0], &mut value);
                bitdiff += value - pair[1]; // value is always >= attrib
            }
        }

        if bitdiff < best_bitdiff || best_bitdiff == -1 {
            chosen = Some(i);
            best_bitdiff = bitdiff;
        }

        if is_truecolor && (bitdiff < best_truecolor_bitdiff || best_truecolor_bitdiff == -1) {
            truecolor_config_idx = Some(i);
            best_truecolor_bitdiff = bitdiff;
        }
    }

    /* Some apps request a low color depth, either because they _assume_
    they'll get a larger one but don't want to fail if only smaller ones
    are available, or they just never called SDL_GL_SetAttribute at all and
    got a tiny default. For these cases, a game that would otherwise run
    at 24-bit color might get dithered down to something smaller, which is
    worth avoiding. If the app requested <= 16 bit color and an exact 24-bit
    match is available, favor that. Otherwise, we look for the closest
    match. Note that while the API promises what you request _or better_,
    it's feasible this can be disastrous for performance for custom software
    on small hardware that all expected to actually get 16-bit color. In this
    case, turn off FAVOR_TRUECOLOR (and maybe send a patch to make this more
    flexible). */
    // (FAVOR_TRUECOLOR)
    if (config.red_size + config.blue_size + config.green_size) <= 16 {
        if let Some(idx) = truecolor_config_idx {
            chosen = Some(idx);
        }
    }

    chosen
}

/// Translation of `SDL_EGL_PrivateChooseConfig()`.
fn private_choose_config(set_config_caveat_none: bool) -> bool {
    let Some(e) = egl() else { return false };
    let config = gl_config();
    let has_khr_create_context = config.profile_mask == GL_CONTEXT_PROFILE_ES
        && config.major_version >= 3
        && has_extension(&e, ExtensionType::Display, "EGL_KHR_create_context");
    let (attribs, api) = choose_config_attribs(
        &config,
        set_config_caveat_none,
        e.is_offscreen,
        has_khr_create_context,
        e.egl_surfacetype,
    );
    // SAFETY: binding an API has no preconditions.
    unsafe { (e.f.eglBindAPI)(api) };

    // 128 seems even nicer here
    let mut configs: [EGLConfig; 128] = [std::ptr::null_mut(); 128];
    let mut found_configs: EGLint = 0;
    // SAFETY: the attribute list is EGL_NONE-terminated; the array holds
    // 128 configs.
    if unsafe {
        (e.f.eglChooseConfig)(
            e.egl_display,
            attribs.as_ptr(),
            configs.as_mut_ptr(),
            configs.len() as EGLint,
            &mut found_configs,
        )
    } == EGL_FALSE
        || found_configs == 0
    {
        return false;
    }
    let found = (found_configs as usize).min(configs.len());

    let chosen = select_config(
        found,
        &attribs,
        e.egl_required_visual_id,
        &config,
        |i, attribute, value| e.get_config_attrib(configs[i], attribute, value),
    );
    if let Some(i) = chosen {
        with_egl(|e| e.egl_config = configs[i]);
    }

    // (DUMP_EGL_CONFIG is off)

    true
}

/// Choose the config for new surfaces and contexts. Translation of
/// `SDL_EGL_ChooseConfig()`.
pub(crate) fn choose_config() -> Result<()> {
    let Some(e) = egl() else {
        return Err(Error::new("EGL not initialized"));
    };

    // Try with EGL_CONFIG_CAVEAT set to EGL_NONE, to avoid any EGL_SLOW_CONFIG or EGL_NON_CONFORMANT_CONFIG
    if private_choose_config(true) {
        return Ok(());
    }

    // Fallback with all configs
    if private_choose_config(false) {
        crate::log!("SDL_EGL_ChooseConfig: found a slow EGL config");
        return Ok(());
    }

    Err(e.set_error("Couldn't find matching EGL config", "eglChooseConfig"))
}

/// The attributes of `SDL_EGL_CreateContext()` before the callback's.
fn context_attribs(
    config: &GlConfig,
    has_khr_create_context: impl FnOnce() -> bool,
    has_khr_no_error: impl FnOnce() -> bool,
) -> Result<Vec<EGLint>> {
    let profile_mask = config.profile_mask;
    let major_version = config.major_version;
    let minor_version = config.minor_version;
    let profile_es = profile_mask == GL_CONTEXT_PROFILE_ES;
    let mut attribs: Vec<EGLint> = Vec::with_capacity(33);

    // Set the context version and other attributes.
    if (major_version < 3 || (minor_version == 0 && profile_es))
        && config.flags == 0
        && (profile_mask == 0 || profile_es)
    {
        /* Create a context without using EGL_KHR_create_context attribs.
         * When creating a GLES context without EGL_KHR_create_context we can
         * only specify the major version. When creating a desktop GL context
         * we can't specify any version, so we only try in that case when the
         * version is less than 3.0 (matches SDL's GLX/WGL behavior.)
         */
        if profile_es {
            attribs.extend([EGL_CONTEXT_CLIENT_VERSION, major_version.max(1)]);
        }
    } else {
        /* The Major/minor version, context profiles, and context flags can
         * only be specified when this extension is available.
         */
        if has_khr_create_context() {
            attribs.extend([EGL_CONTEXT_MAJOR_VERSION_KHR, major_version]);
            attribs.extend([EGL_CONTEXT_MINOR_VERSION_KHR, minor_version]);

            // SDL profile bits match EGL profile bits.
            if profile_mask != 0 && profile_mask != GL_CONTEXT_PROFILE_ES {
                attribs.extend([EGL_CONTEXT_OPENGL_PROFILE_MASK_KHR, profile_mask]);
            }

            // SDL flags match EGL flags.
            if config.flags != 0 {
                attribs.extend([EGL_CONTEXT_FLAGS_KHR, config.flags]);
            }
        } else {
            return Err(Error::new(
                "Could not create EGL context (context attributes are not supported)",
            ));
        }
    }

    if config.no_error != 0 && has_khr_no_error() {
        attribs.extend([EGL_CONTEXT_OPENGL_NO_ERROR_KHR, config.no_error]);
    }

    Ok(attribs)
}

/// Append a callback's key/value pairs (up to `EGL_NONE`) to `attribs`,
/// which (with its terminator) holds at most `max_attribs` values.
fn append_user_attribs(
    attribs: &mut Vec<EGLint>,
    user_attribs: &[EGLint],
    max_attribs: usize,
) -> std::result::Result<(), ()> {
    // Note (upstream): the list is read up to EGL_NONE or its end (never
    // past it), and isn't leaked when there are too many attributes.
    for pair in user_attribs.chunks(2) {
        if pair[0] == EGL_NONE {
            break;
        }
        if attribs.len() + 3 >= max_attribs {
            return Err(());
        }
        attribs.push(pair[0]);
        attribs.push(pair.get(1).copied().unwrap_or(EGL_NONE));
    }
    Ok(())
}

/// Create a context for `egl_surface` and make it current. Translation of
/// `SDL_EGL_CreateContext()`.
pub(crate) fn create_context(egl_surface: EGLSurface) -> Result<RawGlContext> {
    let Some(e) = egl() else {
        return Err(Error::new("EGL not initialized"));
    };
    let config = gl_config();
    let profile_es = config.profile_mask == GL_CONTEXT_PROFILE_ES;

    let share_context = if config.share_with_current_context != 0 {
        gl::current_thread_context().map_or(EGL_NO_CONTEXT, |c| c.as_ptr())
    } else {
        EGL_NO_CONTEXT
    };

    // (the EGL_KHR_debug check is Android's)

    let mut attribs = context_attribs(
        &config,
        || has_extension(&e, ExtensionType::Display, "EGL_KHR_create_context"),
        || {
            has_extension(
                &e,
                ExtensionType::Display,
                "EGL_KHR_create_context_no_error",
            )
        },
    )?;

    if let Some(callbacks) = gl::egl_attrib_callbacks() {
        if let Some(callback) = &callbacks.context {
            // max 16 key+value pairs plus terminator.
            const MAX_ATTRIBS: usize = 33;
            let display = EglDisplay::from_ptr(e.egl_display);
            let user_attribs = display.and_then(|d| callback(d, EglConfig::from_ptr(e.egl_config)));
            let Some(user_attribs) = user_attribs else {
                reset_driver_loaded();
                return Err(Error::new(
                    "EGL context attribute callback returned NULL pointer",
                ));
            };
            if append_user_attribs(&mut attribs, &user_attribs, MAX_ATTRIBS).is_err() {
                reset_driver_loaded();
                return Err(Error::new(
                    "EGL context attribute callback returned too many attributes",
                ));
            }
        }
    }

    attribs.push(EGL_NONE);

    // Bind the API
    let apitype = if profile_es {
        EGL_OPENGL_ES_API
    } else {
        EGL_OPENGL_API
    };
    with_egl(|e| e.apitype = apitype);
    // SAFETY: binding an API has no preconditions.
    if unsafe { (e.f.eglBindAPI)(apitype) } == 0 {
        return Err(Error::new("Could not bind EGL API"));
    }

    // SAFETY: the display and config are EGL's; the share context is
    // NULL or a context of this display; the list is EGL_NONE-terminated.
    let egl_context = unsafe {
        (e.f.eglCreateContext)(e.egl_display, e.egl_config, share_context, attribs.as_ptr())
    };

    let Some(context) = RawGlContext::from_ptr(egl_context) else {
        return Err(e.set_error("Could not create EGL context", "eglCreateContext"));
    };

    // The default swap interval is 1, according to the spec, but SDL3's policy is to default vsync to off by default.
    with_egl(|e| e.egl_swapinterval = 0);

    if let Err(err) = make_current(egl_surface, Some(context)) {
        // Delete the context
        destroy_context(context);
        return Err(err);
    }

    /* Check whether making contexts current without a surface is supported.
     * First condition: EGL must support it. That's the case for EGL 1.5
     * or later, or if the EGL_KHR_surfaceless_context extension is present. */
    let e = egl().unwrap_or(e);
    if e.egl_version_major > 1
        || (e.egl_version_major == 1 && e.egl_version_minor >= 5)
        || has_extension(&e, ExtensionType::Display, "EGL_KHR_surfaceless_context")
    {
        // Secondary condition: The client API must support it.
        if profile_es {
            /* On OpenGL ES, the GL_OES_surfaceless_context extension must be
             * present. */
            if gl::gl_extension_supported("GL_OES_surfaceless_context") {
                gl::set_allow_no_surface();
            }
        } else {
            // Desktop OpenGL supports it by default from version 3.0 on.
            type PfnGlGetIntegerv = unsafe extern "system" fn(u32, *mut i32);
            // SAFETY: glGetIntegerv's type.
            if let Some(gl_get_integerv_func) =
                unsafe { gl::gl_function::<PfnGlGetIntegerv>("glGetIntegerv") }
            {
                let mut v = 0;
                // SAFETY: the new context is current; v is a valid out-parameter.
                unsafe { gl_get_integerv_func(gl::GL_MAJOR_VERSION, &mut v) };
                if v >= 3 {
                    gl::set_allow_no_surface();
                }
            }
        }
    }

    let _ = set_swap_interval(0); // EGL tends to default to vsync=1. To make this consistent with the rest of SDL, we force it off at startup. Apps can explicitly enable it afterwards.

    Ok(context)
}

/// Make `context` current with `egl_surface` (both `None`/null: release).
/// Translation of `SDL_EGL_MakeCurrent()`.
pub(crate) fn make_current(egl_surface: EGLSurface, context: Option<RawGlContext>) -> Result<()> {
    let Some(e) = egl() else {
        return Err(Error::new("EGL not initialized"));
    };
    let egl_context = context.map_or(EGL_NO_CONTEXT, |c| c.as_ptr());

    // (eglMakeCurrent is always loaded once egl_data exists)

    // Make sure current thread has a valid API bound to it.
    // SAFETY: binding an API has no preconditions.
    unsafe { (e.f.eglBindAPI)(e.apitype) };

    /* The android emulator crashes badly if you try to eglMakeCurrent
     * with a valid context and invalid surface, so we have to check for both here.
     */
    // SAFETY: the display, surface and context are EGL's (or none).
    unsafe {
        if egl_context.is_null() || (egl_surface.is_null() && !gl::allow_no_surface()) {
            (e.f.eglMakeCurrent)(
                e.egl_display,
                EGL_NO_SURFACE,
                EGL_NO_SURFACE,
                EGL_NO_CONTEXT,
            );
        } else if (e.f.eglMakeCurrent)(e.egl_display, egl_surface, egl_surface, egl_context) == 0 {
            return Err(e.set_error("Unable to make EGL context current", "eglMakeCurrent"));
        }
    }

    Ok(())
}

/// Translation of `SDL_EGL_SetSwapInterval()`.
pub(crate) fn set_swap_interval(interval: i32) -> Result<()> {
    let Some(e) = egl() else {
        return Err(Error::new("EGL not initialized"));
    };

    /* FIXME: Revisit this check when EGL_EXT_swap_control_tear is published:
     * https://github.com/KhronosGroup/EGL-Registry/pull/113
     */
    if interval < 0 {
        return Err(Error::new("Late swap tearing currently unsupported"));
    }

    // SAFETY: the display is EGL's.
    let status = unsafe { (e.f.eglSwapInterval)(e.egl_display, interval) };
    if status == EGL_TRUE {
        with_egl(|e| e.egl_swapinterval = interval);
        return Ok(());
    }

    Err(e.set_error("Unable to set the EGL swap interval", "eglSwapInterval"))
}

/// Translation of `SDL_EGL_GetSwapInterval()`.
pub(crate) fn get_swap_interval() -> Result<i32> {
    match egl() {
        Some(e) => Ok(e.egl_swapinterval),
        None => Err(Error::new("EGL not initialized")),
    }
}

/// Translation of `SDL_EGL_SwapBuffers()`.
pub(crate) fn swap_buffers(egl_surface: EGLSurface) -> Result<()> {
    // FIXME (upstream): SDL_EGL_SwapBuffers() dereferences egl_data
    // without checking it; this fails instead.
    let Some(e) = egl() else {
        return Err(Error::new("EGL not initialized"));
    };
    // SAFETY: the display and surface are EGL's.
    if unsafe { (e.f.eglSwapBuffers)(e.egl_display, egl_surface) } == 0 {
        return Err(e.set_error(
            "unable to show color buffer in an OS-native window",
            "eglSwapBuffers",
        ));
    }
    Ok(())
}

/// Translation of `SDL_EGL_DestroyContext()`.
pub(crate) fn destroy_context(context: RawGlContext) {
    // Clean up GLES and EGL
    let Some(e) = egl() else { return };

    // SAFETY: the context belongs to this display.
    unsafe { (e.f.eglDestroyContext)(e.egl_display, context.as_ptr()) };
}

/// The sRGB and opacity attributes of `SDL_EGL_CreateSurface()`, and the
/// index of the `EGL_PRESENT_OPAQUE_EXT` pair (if any).
fn surface_attribs(
    has_gl_colorspace: bool,
    srgbhint: Option<&str>,
    framebuffer_srgb_capable: i32,
    has_present_opaque: bool,
    transparent: bool,
) -> (Vec<EGLint>, Option<usize>) {
    let mut attribs: Vec<EGLint> = Vec::with_capacity(33);

    if has_gl_colorspace {
        match srgbhint.filter(|h| !h.is_empty()) {
            Some("skip") => {
                // don't set an attribute at all.
            }
            Some(hint) => {
                attribs.push(EGL_GL_COLORSPACE_KHR);
                attribs.push(if hints::string_to_bool(Some(hint), false) {
                    EGL_GL_COLORSPACE_SRGB_KHR
                } else {
                    EGL_GL_COLORSPACE_LINEAR_KHR
                });
            }
            None if framebuffer_srgb_capable >= 0 => {
                // default behavior without the hint.
                attribs.push(EGL_GL_COLORSPACE_KHR);
                attribs.push(if framebuffer_srgb_capable != 0 {
                    EGL_GL_COLORSPACE_SRGB_KHR
                } else {
                    EGL_GL_COLORSPACE_LINEAR_KHR
                });
            }
            None => {}
        }
    }

    let mut opaque_ext_idx = None;

    if has_present_opaque {
        opaque_ext_idx = Some(attribs.len());
        let allow_transparent = transparent;
        attribs.push(EGL_PRESENT_OPAQUE_EXT);
        attribs.push(if allow_transparent {
            EGL_FALSE as EGLint
        } else {
            EGL_TRUE as EGLint
        });
    }

    (attribs, opaque_ext_idx)
}

/// Create a window surface for the native window `nw` (of `window`, if
/// any), choosing the config first. Translation of `SDL_EGL_CreateSurface()`.
pub(crate) fn create_surface(window: Option<WindowID>, nw: NativeWindowType) -> Result<EGLSurface> {
    choose_config()?;
    let Some(e) = egl() else {
        return Err(Error::new("EGL not initialized"));
    };

    // (the Android and QNX parts are theirs)

    let transparent = window.is_some_and(|w| {
        super::core::with_window(w, |w| w.flags().contains(WindowFlags::TRANSPARENT))
            .unwrap_or(false)
    });
    let (mut attribs, opaque_ext_idx) = surface_attribs(
        has_extension(&e, ExtensionType::Display, "EGL_KHR_gl_colorspace"),
        hints::get(hints::OPENGL_FORCE_SRGB_FRAMEBUFFER).as_deref(),
        gl_config().framebuffer_srgb_capable,
        has_extension(&e, ExtensionType::Display, "EGL_EXT_present_opaque"),
        transparent,
    );

    if let Some(callbacks) = gl::egl_attrib_callbacks() {
        if let Some(callback) = &callbacks.surface {
            // max 16 key+value pairs, plus terminator.
            const MAX_ATTRIBS: usize = 33;
            let display = EglDisplay::from_ptr(e.egl_display);
            let user_attribs = display.and_then(|d| callback(d, EglConfig::from_ptr(e.egl_config)));
            let Some(user_attribs) = user_attribs else {
                reset_driver_loaded();
                return Err(Error::new(
                    "EGL surface attribute callback returned NULL pointer",
                ));
            };
            if append_user_attribs(&mut attribs, &user_attribs, MAX_ATTRIBS).is_err() {
                reset_driver_loaded();
                return Err(Error::new(
                    "EGL surface attribute callback returned too many attributes",
                ));
            }
        }
    }

    attribs.push(EGL_NONE);

    // SAFETY: the display and config are EGL's; the native window is the
    // driver's; the list is EGL_NONE-terminated.
    let mut surface =
        unsafe { (e.f.eglCreateWindowSurface)(e.egl_display, e.egl_config, nw, attribs.as_ptr()) };
    if surface == EGL_NO_SURFACE {
        // we had a report of Nvidia drivers that report EGL_BAD_ATTRIBUTE if you try to
        //  use EGL_PRESENT_OPAQUE_EXT, even when EGL_EXT_present_opaque is reported as available.
        //  If we used it, try a second time without this attribute.
        // SAFETY: eglGetError has no preconditions.
        if unsafe { (e.f.eglGetError)() } == EGL_BAD_ATTRIBUTE {
            if let Some(idx) = opaque_ext_idx {
                attribs.drain(idx..idx + 2);
                // SAFETY: as above.
                surface = unsafe {
                    (e.f.eglCreateWindowSurface)(e.egl_display, e.egl_config, nw, attribs.as_ptr())
                };
            }
        }
    }

    if surface == EGL_NO_SURFACE {
        // FIXME (upstream): the first failure's error was already taken by
        // the eglGetError() above, so this usually reports EGL_SUCCESS.
        return Err(e.set_error(
            "unable to create an EGL window surface",
            "eglCreateWindowSurface",
        ));
    }

    Ok(surface)
}

/// A pbuffer surface of `width`x`height`. Translation of
/// `SDL_EGL_CreateOffscreenSurface()`.
#[allow(dead_code)] // (for the offscreen driver's EGL support, not translated yet)
pub(crate) fn create_offscreen_surface(width: i32, height: i32) -> Result<EGLSurface> {
    let attributes: [EGLint; 5] = [EGL_WIDTH, width, EGL_HEIGHT, height, EGL_NONE];

    choose_config()?;
    let Some(e) = egl() else {
        return Err(Error::new("EGL not initialized"));
    };

    // SAFETY: the display and config are EGL's; the list is EGL_NONE-terminated.
    Ok(unsafe { (e.f.eglCreatePbufferSurface)(e.egl_display, e.egl_config, attributes.as_ptr()) })
}

/// Translation of `SDL_EGL_DestroySurface()`.
#[allow(dead_code)] // (for the Windows EGL support, not translated yet)
pub(crate) fn destroy_surface(egl_surface: EGLSurface) {
    let Some(e) = egl() else { return };

    if egl_surface != EGL_NO_SURFACE {
        // SAFETY: the surface belongs to this display.
        unsafe { (e.f.eglDestroySurface)(e.egl_display, egl_surface) };
    }
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
            release_behavior: 1,
            ..GlConfig::default()
        }
    }

    // The expected values below come from the C reference
    // (gl-ref/harness.c, upstream's code on the same inputs).

    #[test]
    fn extension_strings_like_upstream() {
        let cases = [
            ("GL_ARB_a GL_ARB_b", "GL_ARB_a", true),
            ("GL_ARB_a GL_ARB_b", "GL_ARB_b", true),
            ("GL_ARB_a GL_ARB_b", "GL_ARB", false),
            ("GL_ARB_ab GL_ARB_a", "GL_ARB_a", true),
            ("xGL_ARB_a", "GL_ARB_a", false),
            ("xabcabc", "abc", false),
            ("xabcabc def", "abc", false),
            ("abc", "abc", true),
            ("", "abc", false),
            ("abcd abc", "abc", true),
            ("abcabc", "abc", false),
            ("ab abc ", "abc", true),
            ("  abc", "abc", true),
        ];
        for (list, ext, expected) in cases {
            assert_eq!(extension_in_string(list, ext), expected, "{list:?} {ext:?}");
        }
    }

    #[test]
    fn version_strings_like_sscanf() {
        assert_eq!(parse_egl_version("1.5"), Some((1, 5)));
        assert_eq!(parse_egl_version("1.4 Mesa 25.2.8"), Some((1, 4)));
        assert_eq!(parse_egl_version(" 1.5"), Some((1, 5)));
        assert_eq!(parse_egl_version("1. 4"), Some((1, 4)));
        assert_eq!(parse_egl_version("1.x"), Option::None);
        assert_eq!(parse_egl_version("x1.5"), Option::None);
        assert_eq!(parse_egl_version("1"), Option::None);
        assert_eq!(parse_egl_version("-1.+2"), Some((-1, 2)));
        assert_eq!(parse_egl_version(""), Option::None);
        assert_eq!(parse_egl_version("12.34.56"), Some((12, 34)));
    }

    #[test]
    fn choose_config_attributes_like_upstream() {
        let mut c = default_config();
        assert_eq!(
            choose_config_attribs(&c, true, false, false, 0),
            (
                vec![
                    12324, 8, 12323, 8, 12322, 8, 12327, 12344, 12321, 8, 12325, 16, 12352, 8,
                    12344
                ],
                EGL_OPENGL_API
            )
        );
        c.profile_mask = GL_CONTEXT_PROFILE_ES;
        c.major_version = 3;
        c.buffer_size = 32;
        c.stencil_size = 8;
        c.multisamplebuffers = 1;
        c.multisamplesamples = 4;
        c.floatbuffers = 1;
        assert_eq!(
            choose_config_attribs(&c, false, true, true, 4),
            (
                vec![
                    12324, 8, 12323, 8, 12322, 8, 12321, 8, 12320, 32, 12325, 16, 12326, 8, 12338,
                    1, 12337, 4, 13113, 13115, 12339, 1, 12352, 64, 12339, 4, 12344
                ],
                EGL_OPENGL_ES_API
            )
        );
        assert_eq!(
            choose_config_attribs(&c, false, false, false, 0).0,
            [
                12324, 8, 12323, 8, 12322, 8, 12321, 8, 12320, 32, 12325, 16, 12326, 8, 12338, 1,
                12337, 4, 13113, 13115, 12352, 4, 12344
            ]
        );
        c.major_version = 1;
        assert_eq!(
            choose_config_attribs(&c, false, false, true, 0).0,
            [
                12324, 8, 12323, 8, 12322, 8, 12321, 8, 12320, 32, 12325, 16, 12326, 8, 12338, 1,
                12337, 4, 13113, 13115, 12352, 1, 12344
            ]
        );
    }

    #[test]
    fn config_selection_like_upstream() {
        // r, g, b, a, depth, stencil, native visual
        let configs: [[EGLint; 7]; 5] = [
            [5, 6, 5, 0, 16, 0, 11],
            [8, 8, 8, 8, 24, 8, 22],
            [8, 8, 8, 0, 16, 0, 33],
            [10, 10, 10, 2, 24, 0, 44],
            [8, 8, 8, 8, 16, 0, 22],
        ];
        let get = |i: usize, attr: EGLint, value: &mut EGLint| {
            let c = &configs[i];
            *value = match attr {
                EGL_RED_SIZE => c[0],
                EGL_GREEN_SIZE => c[1],
                EGL_BLUE_SIZE => c[2],
                EGL_ALPHA_SIZE => c[3],
                EGL_DEPTH_SIZE => c[4],
                EGL_STENCIL_SIZE => c[5],
                EGL_NATIVE_VISUAL_ID => c[6],
                _ => return EGL_FALSE,
            };
            EGL_TRUE
        };
        let mut c = default_config();
        let (attribs, _) = choose_config_attribs(&c, true, false, false, 0);
        assert_eq!(select_config(5, &attribs, 0, &c, get), Some(0));
        assert_eq!(select_config(5, &attribs, 33, &c, get), Some(2));
        assert_eq!(select_config(5, &attribs, 99, &c, get), Some(0));
        c.red_size = 5;
        c.green_size = 6;
        c.blue_size = 5;
        c.alpha_size = 0;
        let (attribs, _) = choose_config_attribs(&c, true, false, false, 0);
        assert_eq!(select_config(5, &attribs, 0, &c, get), Some(2));
        assert_eq!(select_config(1, &attribs, 0, &c, get), Some(0));
        c.red_size = 10;
        c.green_size = 10;
        c.blue_size = 10;
        c.alpha_size = 2;
        c.depth_size = 24;
        let (attribs, _) = choose_config_attribs(&c, true, false, false, 0);
        assert_eq!(select_config(5, &attribs, 0, &c, get), Some(0));
    }

    #[test]
    fn context_attributes_like_upstream() {
        let mut c = default_config();
        assert_eq!(context_attribs(&c, || true, || true).unwrap(), []);
        c.profile_mask = GL_CONTEXT_PROFILE_ES;
        c.major_version = 3;
        c.minor_version = 0;
        assert_eq!(context_attribs(&c, || false, || false).unwrap(), [12440, 3]);
        c.major_version = 0;
        assert_eq!(context_attribs(&c, || false, || false).unwrap(), [12440, 1]);
        c.major_version = 3;
        c.minor_version = 1;
        assert_eq!(
            context_attribs(&c, || true, || false).unwrap(),
            [12440, 3, 12539, 1]
        );
        assert!(context_attribs(&c, || false, || false).is_err());
        c.profile_mask = 1;
        c.flags = 1;
        c.no_error = 1;
        assert_eq!(
            context_attribs(&c, || true, || true).unwrap(),
            [12440, 3, 12539, 1, 12541, 1, 12540, 1, 12723, 1]
        );
        assert_eq!(
            context_attribs(&c, || true, || false).unwrap(),
            [12440, 3, 12539, 1, 12541, 1, 12540, 1]
        );
    }

    #[test]
    fn user_attributes() {
        let mut attribs = vec![1, 2];
        append_user_attribs(&mut attribs, &[3, 4, EGL_NONE, 5, 6], 33).unwrap();
        assert_eq!(attribs, [1, 2, 3, 4]);
        append_user_attribs(&mut attribs, &[7], 33).unwrap();
        assert_eq!(attribs, [1, 2, 3, 4, 7, EGL_NONE]);
        let mut attribs = Vec::new();
        let many: Vec<EGLint> = (1..=40).collect();
        assert!(append_user_attribs(&mut attribs, &many, 33).is_err());
        assert_eq!(
            terminated_pairs(&[1isize, 2, 0x3038, 4], 0x3038),
            [1, 2, 0x3038]
        );
        assert_eq!(terminated_pairs(&[1isize, 2, 3], 0x3038), [1, 2, 0x3038]);
    }

    #[test]
    fn surface_attributes() {
        assert_eq!(
            surface_attribs(true, Option::None, 0, true, false),
            (vec![0x309D, 0x308A, 0x31DF, 1], Some(2))
        );
        assert_eq!(
            surface_attribs(true, Some("1"), 0, true, true),
            (vec![0x309D, 0x3089, 0x31DF, 0], Some(2))
        );
        assert_eq!(
            surface_attribs(true, Some("skip"), 1, false, false),
            (vec![], Option::None)
        );
        assert_eq!(
            surface_attribs(false, Some("1"), 1, false, false),
            (vec![], Option::None)
        );
    }

    #[test]
    fn error_names() {
        assert_eq!(egl_get_error_name(0x3000), "EGL_SUCCESS");
        assert_eq!(egl_get_error_name(0x300D), "EGL_BAD_SURFACE");
        assert_eq!(
            egl_set_error_ex("Could not create EGL context", "eglCreateContext", 0x3004).message(),
            "Could not create EGL context (call to eglCreateContext failed, reporting an error of EGL_BAD_ATTRIBUTE)"
        );
        assert_eq!(
            egl_set_error_ex("m", "f", 0x1234).message(),
            "m (call to f failed, reporting an error of 0x1234)"
        );
        assert_eq!(
            egl_set_error_ex("m", "f", -1).message(),
            "m (call to f failed, reporting an error of 0xffffffff)"
        );
    }
}
