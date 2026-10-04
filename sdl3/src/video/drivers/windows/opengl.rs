// Rust translation of src/video/windows/SDL_windowsopengl.c and
// SDL_windowsopengl.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! WGL implementation of SDL OpenGL support: opengl32.dll is loaded at run
//! time, the WGL entry points are declared here by hand (the GDI pixel
//! format functions come from gdi32.dll, as upstream).
//!
//! The device starts with these WGL functions, or with the EGL ones of
//! [`opengles`](super::opengles) when [`hints::VIDEO_FORCE_EGL`] is set;
//! either switches to the other when the requested profile needs it
//! (upstream swaps the device's function pointers: here [`GlBackend`]).
//!
//! The Xbox parts (`SDL_PLATFORM_XBOXONE`/`SDL_PLATFORM_XBOXSERIES`, with
//! the GDI functions from the GL library) aren't translated.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::ptr::NonNull;
use std::sync::{Arc, MutexGuard};

use windows_sys::core::BOOL;
use windows_sys::Win32::Graphics::Gdi::{GetDC, ReleaseDC, HDC};
use windows_sys::Win32::Graphics::OpenGL::{
    ChoosePixelFormat, DescribePixelFormat, SetPixelFormat, SwapBuffers, HGLRC, PFD_DOUBLEBUFFER,
    PFD_DRAW_TO_WINDOW, PFD_MAIN_PLANE, PFD_STEREO, PFD_SUPPORT_OPENGL, PFD_TYPE_RGBA,
    PIXELFORMATDESCRIPTOR,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, WS_DISABLED, WS_POPUP,
};

use super::events::{app_instance, app_name, pump_events_for_hwnd};
use super::window::window_data;
use super::VideoData;
use crate::core::windows::set_error;
use crate::error::{Error, Result};
use crate::events::WindowID;
use crate::hints;
use crate::loadso::SharedObject;
use crate::video::gl::{self, gl_config, GlConfig, RawGlContext, GL_CONTEXT_PROFILE_ES};

const DEFAULT_OPENGL: &str = "OPENGL32.DLL";

const GL_TRUE: c_int = 1;
const GL_FALSE: c_int = 0;

// WGL_ARB_pixel_format
const WGL_DRAW_TO_WINDOW_ARB: c_int = 0x2001;
const WGL_ACCELERATION_ARB: c_int = 0x2003;
const WGL_DOUBLE_BUFFER_ARB: c_int = 0x2011;
const WGL_STEREO_ARB: c_int = 0x2012;
const WGL_PIXEL_TYPE_ARB: c_int = 0x2013;
const WGL_RED_BITS_ARB: c_int = 0x2015;
const WGL_GREEN_BITS_ARB: c_int = 0x2017;
const WGL_BLUE_BITS_ARB: c_int = 0x2019;
const WGL_ALPHA_BITS_ARB: c_int = 0x201B;
const WGL_ACCUM_RED_BITS_ARB: c_int = 0x201E;
const WGL_ACCUM_GREEN_BITS_ARB: c_int = 0x201F;
const WGL_ACCUM_BLUE_BITS_ARB: c_int = 0x2020;
const WGL_ACCUM_ALPHA_BITS_ARB: c_int = 0x2021;
const WGL_DEPTH_BITS_ARB: c_int = 0x2022;
const WGL_STENCIL_BITS_ARB: c_int = 0x2023;
const WGL_NO_ACCELERATION_ARB: c_int = 0x2025;
const WGL_FULL_ACCELERATION_ARB: c_int = 0x2027;

// WGL_ARB_multisample
const WGL_SAMPLE_BUFFERS_ARB: c_int = 0x2041;
const WGL_SAMPLES_ARB: c_int = 0x2042;

// WGL_ARB_create_context
const WGL_CONTEXT_MAJOR_VERSION_ARB: c_int = 0x2091;
const WGL_CONTEXT_MINOR_VERSION_ARB: c_int = 0x2092;
const WGL_CONTEXT_FLAGS_ARB: c_int = 0x2094;

// WGL_ARB_create_context_profile
const WGL_CONTEXT_PROFILE_MASK_ARB: c_int = 0x9126;

// WGL_ARB_create_context_robustness
const WGL_CONTEXT_RESET_NOTIFICATION_STRATEGY_ARB: c_int = 0x8256;
const WGL_NO_RESET_NOTIFICATION_ARB: c_int = 0x8261;
const WGL_LOSE_CONTEXT_ON_RESET_ARB: c_int = 0x8252;

// WGL_ARB_framebuffer_sRGB
const WGL_FRAMEBUFFER_SRGB_CAPABLE_ARB: c_int = 0x20A9;

// WGL_ARB_pixel_format_float
const WGL_TYPE_RGBA_FLOAT_ARB: c_int = 0x21A0;

// WGL_ARB_context_flush_control
const WGL_CONTEXT_RELEASE_BEHAVIOR_ARB: c_int = 0x2097;
const WGL_CONTEXT_RELEASE_BEHAVIOR_NONE_ARB: c_int = 0x0000;
const WGL_CONTEXT_RELEASE_BEHAVIOR_FLUSH_ARB: c_int = 0x2098;

// WGL_ARB_create_context_no_error
const WGL_CONTEXT_OPENGL_NO_ERROR_ARB: c_int = 0x31B3;

type PfnWglGetProcAddress = unsafe extern "system" fn(*const c_char) -> *mut c_void;
type PfnWglCreateContext = unsafe extern "system" fn(HDC) -> HGLRC;
type PfnWglDeleteContext = unsafe extern "system" fn(HGLRC) -> BOOL;
type PfnWglMakeCurrent = unsafe extern "system" fn(HDC, HGLRC) -> BOOL;
type PfnWglShareLists = unsafe extern "system" fn(HGLRC, HGLRC) -> BOOL;
type PfnWglChoosePixelFormatArb =
    unsafe extern "system" fn(HDC, *const c_int, *const f32, u32, *mut c_int, *mut u32) -> BOOL;
type PfnWglGetPixelFormatAttribivArb =
    unsafe extern "system" fn(HDC, c_int, c_int, u32, *const c_int, *mut c_int) -> BOOL;
type PfnWglSwapIntervalExt = unsafe extern "system" fn(c_int) -> BOOL;
type PfnWglGetSwapIntervalExt = unsafe extern "system" fn() -> c_int;
type PfnWglCreateContextAttribsArb = unsafe extern "system" fn(HDC, HGLRC, *const c_int) -> HGLRC;
type PfnWglGetExtensionsStringArb = unsafe extern "system" fn(HDC) -> *const c_char;

/// Which functions the device's `GL_*` pointers are: WGL
/// (`WIN_GL_*`) or EGL (`WIN_GLES_*`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum GlBackend {
    Wgl,
    Egl,
}

/// The WGL entry points of `SDL_GLDriverData` (`None`: NULL).
#[derive(Clone, Copy)]
#[allow(non_snake_case)] // (the WGL names)
struct WglFns {
    wglGetProcAddress: PfnWglGetProcAddress,
    wglCreateContext: PfnWglCreateContext,
    wglDeleteContext: PfnWglDeleteContext,
    wglMakeCurrent: PfnWglMakeCurrent,
    wglShareLists: Option<PfnWglShareLists>,
    wglChoosePixelFormatARB: Option<PfnWglChoosePixelFormatArb>,
    wglGetPixelFormatAttribivARB: Option<PfnWglGetPixelFormatAttribivArb>,
    wglSwapIntervalEXT: Option<PfnWglSwapIntervalExt>,
    wglGetSwapIntervalEXT: Option<PfnWglGetSwapIntervalExt>,
}

/// The extensions `WIN_GL_InitExtensions()` found.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
#[allow(non_snake_case)] // (the extension names)
pub(crate) struct WglExtensions {
    pub(crate) HAS_WGL_ARB_pixel_format: bool,
    pub(crate) HAS_WGL_EXT_swap_control_tear: bool,
    pub(crate) HAS_WGL_ARB_context_flush_control: bool,
    pub(crate) HAS_WGL_ARB_create_context_robustness: bool,
    pub(crate) HAS_WGL_ARB_create_context_no_error: bool,
    pub(crate) HAS_WGL_ARB_pixel_format_float: bool,
    pub(crate) HAS_WGL_EXT_create_context_es2_profile: bool,
    pub(crate) HAS_WGL_ARB_framebuffer_sRGB: bool,

    /* Max version of OpenGL ES context that can be created if the
       implementation supports WGL_EXT_create_context_es2_profile.
       major = minor = 0 when unsupported.
    */
    pub(crate) es_profile_max_supported_version: (i32, i32),
}

/// Translation of `struct SDL_GLDriverData` (with `gl_config.dll_handle`).
pub(crate) struct WglData {
    dll_handle: Arc<SharedObject>,
    pub(crate) ext: WglExtensions,
    f: WglFns,
}

/// The OpenGL state of the Windows device: the backend in use and
/// `gl_data`.
pub(crate) struct WinGl {
    backend: GlBackend,
    wgl: Option<Arc<WglData>>,
}

impl WinGl {
    /// The state of a new device, with the `GL_*` functions of `backend`.
    pub(crate) fn new(backend: GlBackend) -> WinGl {
        WinGl { backend, wgl: None }
    }
}

/// A function pointer of type `F` from a looked-up address.
///
/// # Safety
///
/// `F` must be the function's type.
unsafe fn cast_fn<F: Copy>(p: Option<NonNull<c_void>>) -> Option<F> {
    // SAFETY: F is a function pointer type (the caller's promise), which has
    // the size of a pointer.
    p.map(|p| unsafe { std::mem::transmute_copy::<*mut c_void, F>(&p.as_ptr()) })
}

/// The address of a WGL extension or GL function. Translation of
/// `WIN_GL_GetProcAddress()`.
fn get_proc_address_raw(wgl: &WglData, proc_name: &str) -> Option<NonNull<c_void>> {
    let name = CString::new(proc_name).ok()?;
    // This is to pick up extensions
    // SAFETY: the name is NUL-terminated.
    let func = unsafe { (wgl.f.wglGetProcAddress)(name.as_ptr()) };
    // This is probably a normal GL function
    NonNull::new(func).or_else(|| wgl.dll_handle.symbol(proc_name).ok())
}

/// The pixel format descriptor of the attributes. Translation of
/// `WIN_GL_SetupPixelFormat()`.
pub(crate) fn setup_pixel_format(config: &GlConfig) -> PIXELFORMATDESCRIPTOR {
    // SAFETY: PIXELFORMATDESCRIPTOR is plain data (SDL_zerop).
    let mut pfd: PIXELFORMATDESCRIPTOR = unsafe { std::mem::zeroed() };
    pfd.nSize = size_of::<PIXELFORMATDESCRIPTOR>() as u16;
    pfd.nVersion = 1;
    pfd.dwFlags = PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL;
    if config.double_buffer != 0 {
        pfd.dwFlags |= PFD_DOUBLEBUFFER;
    }
    if config.stereo != 0 {
        pfd.dwFlags |= PFD_STEREO;
    }
    pfd.iLayerType = PFD_MAIN_PLANE as u8;
    pfd.iPixelType = PFD_TYPE_RGBA;
    pfd.cRedBits = config.red_size as u8;
    pfd.cGreenBits = config.green_size as u8;
    pfd.cBlueBits = config.blue_size as u8;
    pfd.cAlphaBits = config.alpha_size as u8;
    if config.buffer_size != 0 {
        pfd.cColorBits = (config.buffer_size - config.alpha_size) as u8;
    } else {
        pfd.cColorBits =
            (u32::from(pfd.cRedBits) + u32::from(pfd.cGreenBits) + u32::from(pfd.cBlueBits)) as u8;
    }
    pfd.cAccumRedBits = config.accum_red_size as u8;
    pfd.cAccumGreenBits = config.accum_green_size as u8;
    pfd.cAccumBlueBits = config.accum_blue_size as u8;
    pfd.cAccumAlphaBits = config.accum_alpha_size as u8;
    pfd.cAccumBits = (u32::from(pfd.cAccumRedBits)
        + u32::from(pfd.cAccumGreenBits)
        + u32::from(pfd.cAccumBlueBits)
        + u32::from(pfd.cAccumAlphaBits)) as u8;
    pfd.cDepthBits = config.depth_size as u8;
    pfd.cStencilBits = config.stencil_size as u8;
    pfd
}

/// How far `pfd` is from `target` (`None`: it doesn't meet or exceed it),
/// for `WIN_GL_ChoosePixelFormat()`.
pub(crate) fn pixel_format_distance(
    pfd: &PIXELFORMATDESCRIPTOR,
    target: &PIXELFORMATDESCRIPTOR,
) -> Option<u32> {
    if (pfd.dwFlags & target.dwFlags) != target.dwFlags {
        return None;
    }

    if pfd.iLayerType != target.iLayerType {
        return None;
    }
    if pfd.iPixelType != target.iPixelType {
        return None;
    }

    let mut dist: u32 = 0;
    for (have, want) in [
        (pfd.cColorBits, target.cColorBits),
        (pfd.cRedBits, target.cRedBits),
        (pfd.cGreenBits, target.cGreenBits),
        (pfd.cBlueBits, target.cBlueBits),
        (pfd.cAlphaBits, target.cAlphaBits),
        (pfd.cAccumBits, target.cAccumBits),
        (pfd.cAccumRedBits, target.cAccumRedBits),
        (pfd.cAccumGreenBits, target.cAccumGreenBits),
        (pfd.cAccumBlueBits, target.cAccumBlueBits),
        (pfd.cAccumAlphaBits, target.cAccumAlphaBits),
        (pfd.cDepthBits, target.cDepthBits),
        (pfd.cStencilBits, target.cStencilBits),
    ] {
        if have < want {
            return None;
        }
        dist += u32::from(have - want);
    }
    Some(dist)
}

/// The 1-based index of the closest of the `count` formats `describe`
/// returns (`None`: one it can't describe), or 0.
pub(crate) fn closest_pixel_format(
    count: c_int,
    target: &PIXELFORMATDESCRIPTOR,
    mut describe: impl FnMut(c_int) -> Option<PIXELFORMATDESCRIPTOR>,
) -> c_int {
    let mut best = 0;
    let mut best_dist = !0u32;

    for index in 1..=count {
        let Some(pfd) = describe(index) else {
            continue;
        };
        let Some(dist) = pixel_format_distance(&pfd, target) else {
            continue;
        };

        if dist < best_dist {
            best = index;
            best_dist = dist;
        }
    }

    best
}

/* Choose the closest pixel format that meets or exceeds the target.
   FIXME: Should we weight any particular attribute over any other?
*/
/// Translation of `WIN_GL_ChoosePixelFormat()`.
fn choose_pixel_format(hdc: HDC, target: &PIXELFORMATDESCRIPTOR) -> c_int {
    let size = size_of::<PIXELFORMATDESCRIPTOR>() as u32;
    // SAFETY: hdc is a window's DC; a NULL descriptor asks for the count.
    let count = unsafe { DescribePixelFormat(hdc, 1, size, std::ptr::null_mut()) };

    closest_pixel_format(count, target, |index| {
        // SAFETY: PIXELFORMATDESCRIPTOR is plain data.
        let mut pfd: PIXELFORMATDESCRIPTOR = unsafe { std::mem::zeroed() };
        // SAFETY: pfd has the size passed.
        (unsafe { DescribePixelFormat(hdc, index, size, &mut pfd) } != 0).then_some(pfd)
    })
}

/// Whether `extension` is in the space-separated `extensions`.
/// Translation of `HasExtension()`.
pub(crate) fn has_extension(extension: &str, extensions: Option<&str>) -> bool {
    // Extension names should not have spaces.
    if extension.contains(' ') || extension.is_empty() {
        return false;
    }

    let Some(extensions) = extensions else {
        return false;
    };

    /* It takes a bit of care to be fool-proof about parsing the
     * OpenGL extensions string. Don't be fooled by sub-strings,
     * etc. */
    gl::extension_in_list(extensions, extension, true)
}

/// The `WGL_ARB_pixel_format` attributes `WIN_GL_SetupWindowInternal()`
/// builds (0-terminated), and the index of the `WGL_ACCELERATION_ARB`
/// value (`iAccelAttr`), which is retried with `WGL_NO_ACCELERATION_ARB`.
pub(crate) fn pixel_format_attribs(
    config: &GlConfig,
    ext: &WglExtensions,
    srgbhint: Option<&str>,
) -> (Vec<c_int>, usize) {
    let mut attribs: Vec<c_int> = Vec::with_capacity(64);

    attribs.extend([WGL_DRAW_TO_WINDOW_ARB, GL_TRUE]);
    attribs.extend([WGL_RED_BITS_ARB, config.red_size]);
    attribs.extend([WGL_GREEN_BITS_ARB, config.green_size]);
    attribs.extend([WGL_BLUE_BITS_ARB, config.blue_size]);

    if config.alpha_size != 0 {
        attribs.extend([WGL_ALPHA_BITS_ARB, config.alpha_size]);
    }

    attribs.extend([WGL_DOUBLE_BUFFER_ARB, config.double_buffer]);

    attribs.extend([WGL_DEPTH_BITS_ARB, config.depth_size]);

    if config.stencil_size != 0 {
        attribs.extend([WGL_STENCIL_BITS_ARB, config.stencil_size]);
    }

    if config.accum_red_size != 0 {
        attribs.extend([WGL_ACCUM_RED_BITS_ARB, config.accum_red_size]);
    }

    if config.accum_green_size != 0 {
        attribs.extend([WGL_ACCUM_GREEN_BITS_ARB, config.accum_green_size]);
    }

    if config.accum_blue_size != 0 {
        attribs.extend([WGL_ACCUM_BLUE_BITS_ARB, config.accum_blue_size]);
    }

    if config.accum_alpha_size != 0 {
        attribs.extend([WGL_ACCUM_ALPHA_BITS_ARB, config.accum_alpha_size]);
    }

    if config.stereo != 0 {
        attribs.extend([WGL_STEREO_ARB, GL_TRUE]);
    }

    if config.multisamplebuffers != 0 {
        attribs.extend([WGL_SAMPLE_BUFFERS_ARB, config.multisamplebuffers]);
    }

    if config.multisamplesamples != 0 {
        attribs.extend([WGL_SAMPLES_ARB, config.multisamplesamples]);
    }

    if ext.HAS_WGL_ARB_pixel_format_float && config.floatbuffers != 0 {
        attribs.extend([WGL_PIXEL_TYPE_ARB, WGL_TYPE_RGBA_FLOAT_ARB]);
    }

    if ext.HAS_WGL_ARB_framebuffer_sRGB {
        match srgbhint.filter(|h| !h.is_empty()) {
            Some("skip") => {
                // don't set an attribute at all.
            }
            Some(srgbhint) => {
                attribs.push(WGL_FRAMEBUFFER_SRGB_CAPABLE_ARB);
                attribs.push(if hints::string_to_bool(Some(srgbhint), false) {
                    GL_TRUE
                } else {
                    GL_FALSE
                });
            }
            None => {
                if config.framebuffer_srgb_capable != 0 {
                    // default behavior without the hint.
                    attribs.extend([WGL_FRAMEBUFFER_SRGB_CAPABLE_ARB, GL_TRUE]);
                }
            }
        }
    }

    /* We always choose either FULL or NO accel on Windows, because of flaky
    drivers. If the app didn't specify, we use FULL, because that's
    probably what they wanted (and if you didn't care and got FULL, that's
    a perfectly valid result in any case). */
    attribs.push(WGL_ACCELERATION_ARB);
    let accel_index = attribs.len();
    if config.accelerated != 0 {
        attribs.push(WGL_FULL_ACCELERATION_ARB);
    } else {
        attribs.push(WGL_NO_ACCELERATION_ARB);
    }

    attribs.push(0);

    (attribs, accel_index)
}

/// The attributes of a `wglCreateContextAttribsARB` context (the "GL 3.x"
/// path of `WIN_GL_CreateContext()`).
pub(crate) fn context_attribs(config: &GlConfig, ext: &WglExtensions) -> Vec<c_int> {
    // max 14 attributes plus terminator
    let mut attribs: Vec<c_int> = vec![
        WGL_CONTEXT_MAJOR_VERSION_ARB,
        config.major_version,
        WGL_CONTEXT_MINOR_VERSION_ARB,
        config.minor_version,
    ];

    // SDL profile bits match WGL profile bits
    if config.profile_mask != 0 {
        attribs.extend([WGL_CONTEXT_PROFILE_MASK_ARB, config.profile_mask]);
    }

    // SDL flags match WGL flags
    if config.flags != 0 {
        attribs.extend([WGL_CONTEXT_FLAGS_ARB, config.flags]);
    }

    // only set if wgl extension is available and not the default setting
    if ext.HAS_WGL_ARB_context_flush_control && config.release_behavior == 0 {
        attribs.push(WGL_CONTEXT_RELEASE_BEHAVIOR_ARB);
        attribs.push(if config.release_behavior != 0 {
            WGL_CONTEXT_RELEASE_BEHAVIOR_FLUSH_ARB
        } else {
            WGL_CONTEXT_RELEASE_BEHAVIOR_NONE_ARB
        });
    }

    // only set if wgl extension is available and not the default setting
    if ext.HAS_WGL_ARB_create_context_robustness && config.reset_notification != 0 {
        attribs.push(WGL_CONTEXT_RESET_NOTIFICATION_STRATEGY_ARB);
        attribs.push(if config.reset_notification != 0 {
            WGL_LOSE_CONTEXT_ON_RESET_ARB
        } else {
            WGL_NO_RESET_NOTIFICATION_ARB
        });
    }

    // only set if wgl extension is available and not the default setting
    if ext.HAS_WGL_ARB_create_context_no_error && config.no_error != 0 {
        attribs.extend([WGL_CONTEXT_OPENGL_NO_ERROR_ARB, config.no_error]);
    }

    attribs.push(0);
    attribs
}

/// Whether OpenGL ES must go through EGL (for an ES profile): unless WGL
/// can create the requested ES version. Translation of `WIN_GL_UseEGL()`.
pub(crate) fn win_gl_use_egl(wgl: &WglData, config: &GlConfig) -> bool {
    crate::sdl_assert!(config.profile_mask == GL_CONTEXT_PROFILE_ES);
    use_egl_with(
        &wgl.ext,
        config,
        hints::get_bool(hints::OPENGL_ES_DRIVER, false),
    )
}

/// `WIN_GL_UseEGL()` with the extensions and `SDL_HINT_OPENGL_ES_DRIVER`.
pub(crate) fn use_egl_with(ext: &WglExtensions, config: &GlConfig, es_driver: bool) -> bool {
    let (max_major, max_minor) = ext.es_profile_max_supported_version;
    // (we don't need EGL to do OpenGL ES if HAS_WGL_EXT_create_context_es2_profile exists.)
    !ext.HAS_WGL_EXT_create_context_es2_profile
        || es_driver
        || config.major_version > max_major
        || (config.major_version == max_major && config.minor_version > max_minor)
}

/// A hidden window for `WIN_GL_InitExtensions()` and
/// `WIN_GL_ChoosePixelFormatARB()` (NULL on failure).
fn create_temp_window() -> windows_sys::Win32::Foundation::HWND {
    let name = app_name();
    // SAFETY: the class is registered (register_app()); the strings are
    // NUL-terminated.
    unsafe {
        CreateWindowExW(
            0,
            name.as_ptr(),
            name.as_ptr(),
            WS_POPUP | WS_DISABLED,
            0,
            0,
            10,
            10,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            app_instance(),
            std::ptr::null(),
        )
    }
}

impl VideoData {
    fn gl_state(&self) -> MutexGuard<'_, WinGl> {
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
    pub(crate) fn wgl_data(&self) -> Option<Arc<WglData>> {
        self.gl_state().wgl.clone()
    }

    /// Translation of `WIN_GL_LoadLibrary_EGLFallback()`.
    fn win_gl_load_library_egl_fallback(&self, path: Option<&str>) -> Result<()> {
        self.win_gl_unload_library();
        self.set_gl_backend(GlBackend::Egl);
        self.win_gles_load_library(path)
    }

    /// Translation of `WIN_GL_LoadLibrary()`.
    pub(crate) fn win_gl_load_library(&self, path: Option<&str>) -> Result<()> {
        let config = gl_config();
        if config.profile_mask == GL_CONTEXT_PROFILE_ES
            && hints::get_bool(hints::OPENGL_ES_DRIVER, false)
        {
            return self.win_gl_load_library_egl_fallback(path);
        }

        let path = path
            .map(str::to_owned)
            .or_else(|| hints::get(hints::OPENGL_LIBRARY))
            .unwrap_or_else(|| DEFAULT_OPENGL.to_owned());
        let dll_handle = SharedObject::load(&path)?;
        gl::set_driver_path(Some(&path));

        // Load function pointers
        // SAFETY: the functions of these names have these types.
        let (get_proc_address, create_context, delete_context, make_current, share_lists) = unsafe {
            (
                dll_handle
                    .function::<PfnWglGetProcAddress>("wglGetProcAddress")
                    .ok(),
                dll_handle
                    .function::<PfnWglCreateContext>("wglCreateContext")
                    .ok(),
                dll_handle
                    .function::<PfnWglDeleteContext>("wglDeleteContext")
                    .ok(),
                dll_handle
                    .function::<PfnWglMakeCurrent>("wglMakeCurrent")
                    .ok(),
                dll_handle
                    .function::<PfnWglShareLists>("wglShareLists")
                    .ok(),
            )
        };

        let (
            Some(get_proc_address),
            Some(create_context),
            Some(delete_context),
            Some(make_current),
        ) = (
            get_proc_address,
            create_context,
            delete_context,
            make_current,
        )
        else {
            return Err(Error::new("Could not retrieve OpenGL functions"));
        };

        // Allocate OpenGL memory
        self.gl_state().wgl = Some(Arc::new(WglData {
            dll_handle: Arc::new(dll_handle),
            ext: WglExtensions::default(),
            f: WglFns {
                wglGetProcAddress: get_proc_address,
                wglCreateContext: create_context,
                wglDeleteContext: delete_context,
                wglMakeCurrent: make_current,
                wglShareLists: share_lists,
                wglChoosePixelFormatARB: None,
                wglGetPixelFormatAttribivARB: None,
                wglSwapIntervalEXT: None,
                wglGetSwapIntervalEXT: None,
            },
        }));

        /* XXX Too sleazy? WIN_GL_InitExtensions looks for certain OpenGL
           extensions via SDL_GL_DeduceMaxSupportedESProfile. This uses
           SDL_GL_ExtensionSupported which in turn calls SDL_GL_GetProcAddress.
           However SDL_GL_GetProcAddress will fail if the library is not
           loaded; it checks for gl_config.driver_loaded > 0. To avoid this
           test failing, increment driver_loaded around the call to
           WIN_GLInitExtensions.

           Successful loading of the library is normally indicated by
           SDL_GL_LoadLibrary incrementing driver_loaded immediately after
           this function returns 0 to it.

           (upstream lists the alternatives to this here)
        */
        gl::set_driver_loaded(|n| n + 1);
        self.win_gl_init_extensions();
        gl::set_driver_loaded(|n| n - 1);

        Ok(())
    }

    /// Translation of `WIN_GL_GetProcAddress()`.
    pub(crate) fn win_gl_get_proc_address(&self, proc_name: &str) -> Option<NonNull<c_void>> {
        // (gl_data is set whenever the front end asks)
        let wgl = self.wgl_data()?;
        get_proc_address_raw(&wgl, proc_name)
    }

    /// Translation of `WIN_GL_UnloadLibrary()`.
    pub(crate) fn win_gl_unload_library(&self) {
        // (the library is unloaded with the last reference to gl_data)

        // Free OpenGL memory
        self.gl_state().wgl = None;
    }

    /// Translation of `WIN_GL_InitExtensions()`.
    fn win_gl_init_extensions(&self) {
        let Some(wgl) = self.wgl_data() else {
            return;
        };
        let f = &wgl.f;

        let hwnd = create_temp_window();
        if hwnd.is_null() {
            return;
        }
        pump_events_for_hwnd(hwnd);

        let pfd = setup_pixel_format(&gl_config());
        // SAFETY: hwnd is our window; pfd is a valid descriptor.
        let (hdc, hglrc) = unsafe {
            let hdc = GetDC(hwnd);

            SetPixelFormat(hdc, ChoosePixelFormat(hdc, &pfd), &pfd);

            (hdc, (f.wglCreateContext)(hdc))
        };
        if hglrc.is_null() {
            // FIXME (upstream): the window and its DC are never released here.
            return;
        }
        // SAFETY: the context was created for hdc.
        unsafe { (f.wglMakeCurrent)(hdc, hglrc) };

        let get = |name: &str| get_proc_address_raw(&wgl, name);
        // SAFETY: the function of this name has this type.
        let get_extensions_string: Option<PfnWglGetExtensionsStringArb> = unsafe {
            cast_fn(NonNull::new((f.wglGetProcAddress)(
                c"wglGetExtensionsStringARB".as_ptr(),
            )))
        };
        let extensions: Option<String> = get_extensions_string.and_then(|g| {
            // SAFETY: a context is current on hdc; the result is NULL or a
            // NUL-terminated string owned by the driver.
            unsafe {
                let s = g(hdc);
                (!s.is_null()).then(|| CStr::from_ptr(s).to_string_lossy().into_owned())
            }
        });
        let extensions = extensions.as_deref();

        let mut ext = WglExtensions::default();
        let mut f = wgl.f;

        // Check for WGL_ARB_pixel_format
        ext.HAS_WGL_ARB_pixel_format = false;
        if has_extension("WGL_ARB_pixel_format", extensions) {
            // SAFETY (this and the casts below): the functions of these
            // names have these types.
            unsafe {
                f.wglChoosePixelFormatARB = cast_fn(get("wglChoosePixelFormatARB"));
                f.wglGetPixelFormatAttribivARB = cast_fn(get("wglGetPixelFormatAttribivARB"));
            }

            if f.wglChoosePixelFormatARB.is_some() && f.wglGetPixelFormatAttribivARB.is_some() {
                ext.HAS_WGL_ARB_pixel_format = true;
            }
        }

        // Check for WGL_EXT_swap_control
        ext.HAS_WGL_EXT_swap_control_tear = false;
        if has_extension("WGL_EXT_swap_control", extensions) {
            // SAFETY: as above.
            unsafe {
                f.wglSwapIntervalEXT = cast_fn(get("wglSwapIntervalEXT"));
                f.wglGetSwapIntervalEXT = cast_fn(get("wglGetSwapIntervalEXT"));
            }
            if has_extension("WGL_EXT_swap_control_tear", extensions) {
                ext.HAS_WGL_EXT_swap_control_tear = true;
            }
        } else {
            f.wglSwapIntervalEXT = None;
            f.wglGetSwapIntervalEXT = None;
        }

        // Check for WGL_EXT_create_context_es2_profile
        // see if we can get at OpenGL ES profiles even if EGL isn't available.
        ext.HAS_WGL_EXT_create_context_es2_profile =
            has_extension("WGL_EXT_create_context_es2_profile", extensions);
        if ext.HAS_WGL_EXT_create_context_es2_profile {
            ext.es_profile_max_supported_version = gl::gl_deduce_max_supported_es_profile();
        }

        // Check for WGL_ARB_context_flush_control
        if has_extension("WGL_ARB_context_flush_control", extensions) {
            ext.HAS_WGL_ARB_context_flush_control = true;
        }

        // Check for WGL_ARB_create_context_robustness
        if has_extension("WGL_ARB_create_context_robustness", extensions) {
            ext.HAS_WGL_ARB_create_context_robustness = true;
        }

        // Check for WGL_ARB_create_context_no_error
        if has_extension("WGL_ARB_create_context_no_error", extensions) {
            ext.HAS_WGL_ARB_create_context_no_error = true;
        }

        // Check for WGL_ARB_framebuffer_sRGB
        if has_extension("WGL_ARB_framebuffer_sRGB", extensions)
            || has_extension("WGL_EXT_framebuffer_sRGB", extensions)
        // same thing.
        {
            ext.HAS_WGL_ARB_framebuffer_sRGB = true;
        }

        /* Check for WGL_ARB_pixel_format_float */
        ext.HAS_WGL_ARB_pixel_format_float =
            has_extension("WGL_ARB_pixel_format_float", extensions);

        // SAFETY: the context, DC and window are ours.
        unsafe {
            (f.wglMakeCurrent)(hdc, std::ptr::null_mut());
            (f.wglDeleteContext)(hglrc);
            ReleaseDC(hwnd, hdc);
            DestroyWindow(hwnd);
        }
        pump_events_for_hwnd(hwnd);

        // (gl_data gets the extensions)
        self.gl_state().wgl = Some(Arc::new(WglData {
            dll_handle: wgl.dll_handle.clone(),
            ext,
            f,
        }));
    }

    /// Translation of `WIN_GL_ChoosePixelFormatARB()`.
    fn win_gl_choose_pixel_format_arb(
        &self,
        wgl: &WglData,
        i_attribs: &[c_int],
        f_attribs: &[f32],
    ) -> c_int {
        let mut pixel_format: c_int = 0;
        let mut matching: u32 = 0;

        // FIXME (upstream): a failed CreateWindow() isn't checked: the
        // screen's DC (GetDC(NULL)) gets the pixel format then.
        let hwnd = create_temp_window();
        pump_events_for_hwnd(hwnd);

        let pfd = setup_pixel_format(&gl_config());
        // SAFETY: hwnd is our window (see above); pfd is a valid descriptor;
        // the attribute lists are 0-terminated; the context is ours.
        unsafe {
            let hdc = GetDC(hwnd);

            SetPixelFormat(hdc, ChoosePixelFormat(hdc, &pfd), &pfd);

            let hglrc = (wgl.f.wglCreateContext)(hdc);
            if !hglrc.is_null() {
                (wgl.f.wglMakeCurrent)(hdc, hglrc);

                if wgl.ext.HAS_WGL_ARB_pixel_format {
                    if let (Some(choose), Some(get_attribiv)) = (
                        wgl.f.wglChoosePixelFormatARB,
                        wgl.f.wglGetPixelFormatAttribivARB,
                    ) {
                        choose(
                            hdc,
                            i_attribs.as_ptr(),
                            f_attribs.as_ptr(),
                            1,
                            &mut pixel_format,
                            &mut matching,
                        );

                        // Check whether we actually got an SRGB capable buffer
                        let mut srgb: c_int = 0;
                        if wgl.ext.HAS_WGL_ARB_framebuffer_sRGB {
                            let q_attrib = WGL_FRAMEBUFFER_SRGB_CAPABLE_ARB;
                            get_attribiv(hdc, pixel_format, 0, 1, &q_attrib, &mut srgb);
                        }
                        gl::with_gl_config(|c| c.framebuffer_srgb_capable = srgb);
                    }
                }

                (wgl.f.wglMakeCurrent)(hdc, std::ptr::null_mut());
                (wgl.f.wglDeleteContext)(hglrc);
            }
            ReleaseDC(hwnd, hdc);
            DestroyWindow(hwnd);
        }
        pump_events_for_hwnd(hwnd);

        pixel_format
    }

    /// The DC of a window (`window->internal->hdc`).
    fn hdc_of(&self, window: WindowID) -> Result<HDC> {
        window_data(window)
            .map(|d| d.hdc)
            .ok_or_else(|| Error::new("Invalid window"))
    }

    /// actual work of WIN_GL_SetupWindow() happens here. Translation of
    /// `WIN_GL_SetupWindowInternal()`.
    fn win_gl_setup_window_internal(&self, window: WindowID) -> Result<()> {
        let hdc = self.hdc_of(window)?;
        // (the front end loads the library before creating OpenGL windows)
        let Some(wgl) = self.wgl_data() else {
            return Err(Error::new("OpenGL not initialized"));
        };
        let config = gl_config();
        let f_attribs: [f32; 1] = [0.0];

        let pfd = setup_pixel_format(&config);

        // setup WGL_ARB_pixel_format attribs
        let srgbhint = hints::get(hints::OPENGL_FORCE_SRGB_FRAMEBUFFER);
        let (mut i_attribs, i_accel_attr) =
            pixel_format_attribs(&config, &wgl.ext, srgbhint.as_deref());

        // Choose and set the closest available pixel format
        let mut pixel_format = self.win_gl_choose_pixel_format_arb(&wgl, &i_attribs, &f_attribs);

        // App said "don't care about accel" and FULL accel failed. Try NO.
        if pixel_format == 0 && config.accelerated < 0 {
            i_attribs[i_accel_attr] = WGL_NO_ACCELERATION_ARB;
            pixel_format = self.win_gl_choose_pixel_format_arb(&wgl, &i_attribs, &f_attribs);
            i_attribs[i_accel_attr] = WGL_FULL_ACCELERATION_ARB; // if we try again.
        }
        if pixel_format == 0 {
            pixel_format = choose_pixel_format(hdc, &pfd);
        }
        if pixel_format == 0 {
            return Err(Error::new("No matching GL pixel format available"));
        }
        // SAFETY: hdc is the window's DC; pfd is a valid descriptor.
        if unsafe { SetPixelFormat(hdc, pixel_format, &pfd) } == 0 {
            return Err(set_error("SetPixelFormat()"));
        }
        Ok(())
    }

    /// Translation of `WIN_GL_SetupWindow()`.
    pub(crate) fn win_gl_setup_window(&self, window: WindowID) -> Result<()> {
        // The current context is lost in here; save it and reset it.
        let current_win = gl::current_thread_window();
        let current_ctx = gl::current_thread_context();
        let result = self.win_gl_setup_window_internal(window);
        let _ = self.win_gl_make_current(current_win, current_ctx);
        result
    }

    /// Translation of `WIN_GL_CreateContext()`.
    pub(crate) fn win_gl_create_context(&self, window: WindowID) -> Result<RawGlContext> {
        let Some(wgl) = self.wgl_data() else {
            return Err(Error::new("OpenGL not initialized"));
        };
        let config = gl_config();

        if config.profile_mask == GL_CONTEXT_PROFILE_ES && win_gl_use_egl(&wgl, &config) {
            // Switch to EGL based functions

            self.win_gl_load_library_egl_fallback(None)?;

            return self.win_gles_create_context(window);
        }

        let hdc = self.hdc_of(window)?;
        let f = &wgl.f;

        let share_context: HGLRC = if config.share_with_current_context != 0 {
            gl::current_thread_context().map_or(std::ptr::null_mut(), |c| c.as_ptr())
        } else {
            std::ptr::null_mut()
        };

        let context: HGLRC =
            if config.major_version < 3 && config.profile_mask == 0 && config.flags == 0 {
                // Create legacy context
                // SAFETY: hdc is the window's DC (with its pixel format set).
                let context = unsafe { (f.wglCreateContext)(hdc) };
                if !share_context.is_null() {
                    // Note (upstream): a missing wglShareLists is called anyway;
                    // it is skipped here.
                    if let Some(share_lists) = f.wglShareLists {
                        // SAFETY: both are WGL contexts (or the new one is NULL,
                        // which fails).
                        unsafe { share_lists(share_context, context) };
                    }
                }
                context
            } else {
                // SAFETY: as above.
                let temp_context = unsafe { (f.wglCreateContext)(hdc) };
                let Some(temp) = RawGlContext::from_ptr(temp_context) else {
                    return Err(Error::new("Could not create GL context"));
                };

                // Make the context current
                if let Err(e) = self.win_gl_make_current(Some(window), Some(temp)) {
                    self.win_gl_destroy_context(temp);
                    return Err(e);
                }

                // SAFETY: the function of this name has this type.
                let create_context_attribs: Option<PfnWglCreateContextAttribsArb> = unsafe {
                    cast_fn(NonNull::new((f.wglGetProcAddress)(
                        c"wglCreateContextAttribsARB".as_ptr(),
                    )))
                };
                match create_context_attribs {
                    None => {
                        // Note (upstream): "GL 3.x is not supported" is set as the
                        // error while the legacy context is returned; a success
                        // carries no error here.
                        temp_context
                    }
                    Some(create_context_attribs) => {
                        let attribs = context_attribs(&config, &wgl.ext);

                        // SAFETY: hdc is the window's DC; the list is
                        // 0-terminated; temp_context is ours.
                        unsafe {
                            // Create the GL 3.x context
                            let context =
                                create_context_attribs(hdc, share_context, attribs.as_ptr());
                            // Delete the GL 2.x context
                            (f.wglDeleteContext)(temp_context);
                            context
                        }
                    }
                }
            };

        let Some(context) = RawGlContext::from_ptr(context) else {
            return Err(set_error("Could not create GL context"));
        };

        if let Err(e) = self.win_gl_make_current(Some(window), Some(context)) {
            self.win_gl_destroy_context(context);
            return Err(e);
        }

        let has_color_buffer_float = gl::gl_extension_supported("GL_ARB_color_buffer_float");
        gl::with_gl_config(|c| c.has_gl_arb_color_buffer_float = has_color_buffer_float);

        Ok(context)
    }

    /// Translation of `WIN_GL_MakeCurrent()`.
    pub(crate) fn win_gl_make_current(
        &self,
        window: Option<WindowID>,
        context: Option<RawGlContext>,
    ) -> Result<()> {
        let Some(wgl) = self.wgl_data() else {
            return Err(Error::new("OpenGL not initialized"));
        };

        // sanity check that higher level handled this.
        crate::sdl_assert!(window.is_some() || context.is_none());

        /* Some Windows drivers freak out if hdc is NULL, even when context is
        NULL, against spec. Since hdc is _supposed_ to be ignored if context
        is NULL, we either use the current GL window, or do nothing if we
        already have no current context. */
        let window = match window.or_else(gl::current_thread_window) {
            Some(window) => window,
            None => {
                crate::sdl_assert!(gl::current_thread_context().is_none());
                return Ok(()); // already done.
            }
        };

        let hdc = self.hdc_of(window)?;
        let hglrc: HGLRC = context.map_or(std::ptr::null_mut(), |c| c.as_ptr());
        // SAFETY: hdc is the window's DC; the context is a WGL context (or
        // NULL).
        if unsafe { (wgl.f.wglMakeCurrent)(hdc, hglrc) } == 0 {
            return Err(set_error("wglMakeCurrent()"));
        }
        Ok(())
    }

    /// Translation of `WIN_GL_SetSwapInterval()`.
    pub(crate) fn win_gl_set_swap_interval(&self, interval: i32) -> Result<()> {
        // (a context is current, so gl_data is set)
        let Some(wgl) = self.wgl_data() else {
            return Err(Error::new("OpenGL not initialized"));
        };
        if interval < 0 && !wgl.ext.HAS_WGL_EXT_swap_control_tear {
            return Err(Error::new("Negative swap interval unsupported in this GL"));
        } else if let Some(swap_interval) = wgl.f.wglSwapIntervalEXT {
            // SAFETY: a context is current.
            if unsafe { swap_interval(interval) } == 0 {
                return Err(set_error("wglSwapIntervalEXT()"));
            }
        } else {
            return Err(Error::unsupported());
        }
        Ok(())
    }

    /// Translation of `WIN_GL_GetSwapInterval()`.
    pub(crate) fn win_gl_get_swap_interval(&self) -> Result<i32> {
        let get_swap_interval = self.wgl_data().and_then(|wgl| wgl.f.wglGetSwapIntervalEXT);
        match get_swap_interval {
            // SAFETY: a context is current.
            Some(get_swap_interval) => Ok(unsafe { get_swap_interval() }),
            None => Err(Error::unsupported()),
        }
    }

    /// Translation of `WIN_GL_SwapWindow()`.
    pub(crate) fn win_gl_swap_window(&self, window: WindowID) -> Result<()> {
        let hdc = self.hdc_of(window)?;

        // SAFETY: hdc is the window's DC.
        if unsafe { SwapBuffers(hdc) } == 0 {
            return Err(set_error("SwapBuffers()"));
        }
        Ok(())
    }

    /// Translation of `WIN_GL_DestroyContext()`.
    pub(crate) fn win_gl_destroy_context(&self, context: RawGlContext) {
        let Some(wgl) = self.wgl_data() else {
            return;
        };
        // SAFETY: the context is a WGL context of this library.
        unsafe { (wgl.f.wglDeleteContext)(context.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    //! The expected values come from the C reference (the upstream
    //! functions over plain structs).
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

    fn full_config() -> GlConfig {
        GlConfig {
            red_size: 10,
            green_size: 10,
            blue_size: 10,
            alpha_size: 0,
            buffer_size: 32,
            depth_size: 24,
            stencil_size: 8,
            accum_red_size: 1,
            accum_green_size: 2,
            accum_blue_size: 3,
            accum_alpha_size: 4,
            stereo: 1,
            multisamplebuffers: 1,
            multisamplesamples: 4,
            floatbuffers: 1,
            framebuffer_srgb_capable: 1,
            accelerated: 1,
            double_buffer: 0,
            ..default_config()
        }
    }

    fn all_extensions() -> WglExtensions {
        WglExtensions {
            HAS_WGL_ARB_pixel_format: true,
            HAS_WGL_EXT_swap_control_tear: true,
            HAS_WGL_ARB_context_flush_control: true,
            HAS_WGL_ARB_create_context_robustness: true,
            HAS_WGL_ARB_create_context_no_error: true,
            HAS_WGL_ARB_pixel_format_float: true,
            HAS_WGL_EXT_create_context_es2_profile: true,
            HAS_WGL_ARB_framebuffer_sRGB: true,
            es_profile_max_supported_version: (3, 2),
        }
    }

    /// The fields the C reference prints.
    fn pfd_fields(p: &PIXELFORMATDESCRIPTOR) -> [u32; 17] {
        [
            p.nSize.into(),
            p.nVersion.into(),
            p.dwFlags,
            p.iPixelType.into(),
            p.cColorBits.into(),
            p.cRedBits.into(),
            p.cGreenBits.into(),
            p.cBlueBits.into(),
            p.cAlphaBits.into(),
            p.cAccumBits.into(),
            p.cAccumRedBits.into(),
            p.cAccumGreenBits.into(),
            p.cAccumBlueBits.into(),
            p.cAccumAlphaBits.into(),
            p.cDepthBits.into(),
            p.cStencilBits.into(),
            p.iLayerType.into(),
        ]
    }

    #[test]
    fn pixel_format_descriptor_like_upstream() {
        assert_eq!(
            pfd_fields(&setup_pixel_format(&default_config())),
            [40, 1, 0x25, 0, 24, 8, 8, 8, 8, 0, 0, 0, 0, 0, 16, 0, 0]
        );
        assert_eq!(
            pfd_fields(&setup_pixel_format(&full_config())),
            [40, 1, 0x26, 0, 32, 10, 10, 10, 0, 10, 1, 2, 3, 4, 24, 8, 0]
        );
    }

    #[test]
    fn closest_pixel_format_like_upstream() {
        // SAFETY: PIXELFORMATDESCRIPTOR is plain data.
        let mut base: PIXELFORMATDESCRIPTOR = unsafe { std::mem::zeroed() };
        base.dwFlags = PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER;
        base.cColorBits = 24;
        base.cRedBits = 8;
        base.cGreenBits = 8;
        base.cBlueBits = 8;
        base.cAlphaBits = 8;
        base.cDepthBits = 24;
        base.cStencilBits = 8;
        let mut table = [base; 6];
        // no GL
        table[0].dwFlags = PFD_DRAW_TO_WINDOW | PFD_DOUBLEBUFFER;
        // 16-bit
        table[1].cColorBits = 16;
        table[1].cRedBits = 5;
        table[1].cGreenBits = 6;
        table[1].cBlueBits = 5;
        table[1].cAlphaBits = 0;
        // 32-bit with accumulation
        table[3].cColorBits = 32;
        table[3].cDepthBits = 32;
        table[3].cAccumBits = 64;
        table[3].cAccumRedBits = 16;
        // single buffered
        table[4].dwFlags = PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL;
        table[4].cDepthBits = 16;
        table[4].cStencilBits = 0;
        // overlay
        table[5].iLayerType = 1;
        table[5].cDepthBits = 16;
        table[5].cStencilBits = 0;

        let choose = |config: &GlConfig| {
            closest_pixel_format(6, &setup_pixel_format(config), |i| {
                Some(table[(i - 1) as usize])
            })
        };
        assert_eq!(choose(&default_config()), 3);
        assert_eq!(
            choose(&GlConfig {
                double_buffer: 0,
                ..default_config()
            }),
            5
        );
        assert_eq!(
            choose(&GlConfig {
                accum_red_size: 8,
                ..default_config()
            }),
            4
        );
        assert_eq!(choose(&full_config()), 0);
        // (formats that can't be described are skipped)
        let target = setup_pixel_format(&default_config());
        assert_eq!(closest_pixel_format(6, &target, |_| None), 0);
    }

    #[test]
    fn extensions_like_upstream() {
        let exts = Some("WGL_ARB_pixel_format WGL_EXT_swap_control_tear WGL_EXT_swap_control");
        for (name, expected) in [
            ("WGL_ARB_pixel_format", true),
            ("WGL_EXT_swap_control", true),
            ("WGL_EXT_swap", false),
            ("control", false),
            ("WGL_EXT_swap_control_tear", true),
            ("", false),
            ("WGL_ARB pixel", false),
        ] {
            assert_eq!(has_extension(name, exts), expected, "{name}");
        }
        assert!(has_extension("abc", Some("xabcabc")));
        assert!(!has_extension("abc", None));
    }

    #[test]
    fn pixel_format_attributes_like_upstream() {
        let none = WglExtensions::default();
        let all = all_extensions();
        // (name, attributes, extensions, sRGB hint, accel_index, attributes)
        type Case<'a> = (
            &'a str,
            GlConfig,
            &'a WglExtensions,
            Option<&'a str>,
            usize,
            Vec<c_int>,
        );
        let cases: [Case; 8] = [
            (
                "defaults none",
                default_config(),
                &none,
                None,
                15,
                vec![
                    0x2001, 0x1, 0x2015, 0x8, 0x2017, 0x8, 0x2019, 0x8, 0x201b, 0x8, 0x2011, 0x1,
                    0x2022, 0x10, 0x2003, 0x2027, 0,
                ],
            ),
            (
                "defaults all",
                default_config(),
                &all,
                None,
                15,
                vec![
                    0x2001, 0x1, 0x2015, 0x8, 0x2017, 0x8, 0x2019, 0x8, 0x201b, 0x8, 0x2011, 0x1,
                    0x2022, 0x10, 0x2003, 0x2027, 0,
                ],
            ),
            (
                "full none",
                full_config(),
                &none,
                None,
                29,
                vec![
                    0x2001, 0x1, 0x2015, 0xa, 0x2017, 0xa, 0x2019, 0xa, 0x2011, 0, 0x2022, 0x18,
                    0x2023, 0x8, 0x201e, 0x1, 0x201f, 0x2, 0x2020, 0x3, 0x2021, 0x4, 0x2012, 0x1,
                    0x2041, 0x1, 0x2042, 0x4, 0x2003, 0x2027, 0,
                ],
            ),
            (
                "full all",
                full_config(),
                &all,
                None,
                33,
                vec![
                    0x2001, 0x1, 0x2015, 0xa, 0x2017, 0xa, 0x2019, 0xa, 0x2011, 0, 0x2022, 0x18,
                    0x2023, 0x8, 0x201e, 0x1, 0x201f, 0x2, 0x2020, 0x3, 0x2021, 0x4, 0x2012, 0x1,
                    0x2041, 0x1, 0x2042, 0x4, 0x2013, 0x21a0, 0x20a9, 0x1, 0x2003, 0x2027, 0,
                ],
            ),
            (
                "full all skip",
                full_config(),
                &all,
                Some("skip"),
                31,
                vec![
                    0x2001, 0x1, 0x2015, 0xa, 0x2017, 0xa, 0x2019, 0xa, 0x2011, 0, 0x2022, 0x18,
                    0x2023, 0x8, 0x201e, 0x1, 0x201f, 0x2, 0x2020, 0x3, 0x2021, 0x4, 0x2012, 0x1,
                    0x2041, 0x1, 0x2042, 0x4, 0x2013, 0x21a0, 0x2003, 0x2027, 0,
                ],
            ),
            (
                "defaults all 1",
                default_config(),
                &all,
                Some("1"),
                17,
                vec![
                    0x2001, 0x1, 0x2015, 0x8, 0x2017, 0x8, 0x2019, 0x8, 0x201b, 0x8, 0x2011, 0x1,
                    0x2022, 0x10, 0x20a9, 0x1, 0x2003, 0x2027, 0,
                ],
            ),
            (
                "full all 0",
                full_config(),
                &all,
                Some("0"),
                33,
                vec![
                    0x2001, 0x1, 0x2015, 0xa, 0x2017, 0xa, 0x2019, 0xa, 0x2011, 0, 0x2022, 0x18,
                    0x2023, 0x8, 0x201e, 0x1, 0x201f, 0x2, 0x2020, 0x3, 0x2021, 0x4, 0x2012, 0x1,
                    0x2041, 0x1, 0x2042, 0x4, 0x2013, 0x21a0, 0x20a9, 0, 0x2003, 0x2027, 0,
                ],
            ),
            (
                "zero all empty",
                GlConfig::default(),
                &all,
                Some(""),
                13,
                vec![
                    0x2001, 0x1, 0x2015, 0, 0x2017, 0, 0x2019, 0, 0x2011, 0, 0x2022, 0, 0x2003,
                    0x2025, 0,
                ],
            ),
        ];
        for (name, config, ext, srgbhint, accel_index, expected) in cases {
            let (attribs, index) = pixel_format_attribs(&config, ext, srgbhint);
            assert_eq!(attribs, expected, "{name}");
            assert_eq!(index, accel_index, "{name}");
        }
    }

    #[test]
    fn context_attributes_like_upstream() {
        let none = WglExtensions::default();
        let all = all_extensions();
        assert_eq!(
            context_attribs(&default_config(), &none),
            [0x2091, 0x2, 0x2092, 0x1, 0]
        );
        let mut core = GlConfig {
            major_version: 3,
            minor_version: 3,
            profile_mask: 1,
            flags: 3,
            ..default_config()
        };
        assert_eq!(
            context_attribs(&core, &all),
            [0x2091, 0x3, 0x2092, 0x3, 0x9126, 0x1, 0x2094, 0x3, 0]
        );
        core.release_behavior = 0;
        core.reset_notification = 1;
        core.no_error = 1;
        assert_eq!(
            context_attribs(&core, &none),
            [0x2091, 0x3, 0x2092, 0x3, 0x9126, 0x1, 0x2094, 0x3, 0]
        );
        assert_eq!(
            context_attribs(&core, &all),
            [
                0x2091, 0x3, 0x2092, 0x3, 0x9126, 0x1, 0x2094, 0x3, 0x2097, 0, 0x8256, 0x8252,
                0x31b3, 0x1, 0
            ]
        );
    }

    #[test]
    fn use_egl_like_upstream() {
        let none = WglExtensions::default();
        let all = all_extensions();
        let es = |major, minor| GlConfig {
            profile_mask: GL_CONTEXT_PROFILE_ES,
            major_version: major,
            minor_version: minor,
            ..default_config()
        };
        assert!(!use_egl_with(&all, &es(3, 2), false));
        assert!(use_egl_with(&all, &es(3, 2), true));
        assert!(use_egl_with(&none, &es(3, 2), false));
        assert!(use_egl_with(&all, &es(3, 3), false));
        assert!(!use_egl_with(&all, &es(2, 0), false));
        assert!(use_egl_with(&all, &es(4, 0), false));
    }
}
