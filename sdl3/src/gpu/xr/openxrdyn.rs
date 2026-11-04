// Rust translation of src/gpu/xr/SDL_openxrdyn.c, SDL_openxrdyn.h and
// SDL_openxrsym.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The OpenXR loader, loaded at run time: [`hints::OPENXR_LIBRARY`]'s
//! path first, then the platform's name (`libopenxr_loader.dylib` on Apple
//! platforms, `openxr_loader.dll` on Windows, `libopenxr_loader.so` on
//! Android, the Khronos loader of the Gradle package
//! `org.khronos.openxr:openxr_loader_for_android` (not Meta's
//! forwardloader), `libopenxr_loader.so.1` elsewhere), reference counted.
//! The four global functions come from its exports (on Android, after
//! `xrInitializeLoaderKHR()` with the app's Java VM and activity, from its
//! exports or `xrGetInstanceProcAddr(XR_NULL_HANDLE, ...)`), an instance's
//! seven through `xrGetInstanceProcAddr` ([`load_instance_symbols`]).
//!
//! Upstream builds this wherever CMake builds the GPU API
//! (`HAVE_GPU_OPENXR` on every platform but RISC OS), so the stubs of its
//! `#else` ("OpenXR is not enabled in this build of SDL") aren't
//! translated. Upstream's statics are behind a mutex here; its functions
//! are documented as not thread safe.

#![allow(non_snake_case)] // (upstream's symbol names)

use std::ffi::CString;
use std::sync::{Mutex, MutexGuard};

use super::sys::*;
use crate::error::{Error, Result};
use crate::hints;
use crate::loadso::SharedObject;

#[cfg(target_vendor = "apple")]
pub(crate) const OPENXR_LIBRARY_NAMES: &[&str] = &["libopenxr_loader.dylib"];
#[cfg(windows)]
pub(crate) const OPENXR_LIBRARY_NAMES: &[&str] = &["openxr_loader.dll"];
// On Android, use the Khronos OpenXR loader (libopenxr_loader.so) which
// properly exports xrGetInstanceProcAddr. This is bundled via the Gradle
// dependency:
//   implementation 'org.khronos.openxr:openxr_loader_for_android:X.Y.Z'
//
// The Khronos loader handles runtime discovery internally via the Android
// broker pattern and properly supports all pre-instance global functions.
//
// Note: Do NOT use Meta's forwardloader (libopenxr_forwardloader.so) - it
// doesn't export xrGetInstanceProcAddr directly and the function obtained
// via runtime negotiation crashes on pre-instance calls (e.g.,
// xrEnumerateApiLayerProperties).
#[cfg(target_os = "android")]
pub(crate) const OPENXR_LIBRARY_NAMES: &[&str] = &["libopenxr_loader.so"];
#[cfg(not(any(target_vendor = "apple", windows, target_os = "android")))]
pub(crate) const OPENXR_LIBRARY_NAMES: &[&str] = &["libopenxr_loader.so.1"];
// (SDL_ELF_NOTE_DLOPEN "gpu-openxr", "Support for OpenXR with SDL_GPU
// rendering", suggested, "libopenxr_loader.so.1": upstream's ELF note for
// packagers; none here)

/// The global functions (`SDL_OPENXR_SYM`: upstream's `OPENXR_xr*`).
#[derive(Clone, Copy, Default)]
pub(crate) struct GlobalPfns {
    pub(crate) xrGetInstanceProcAddr: Option<PfnXrGetInstanceProcAddr>,
    pub(crate) xrEnumerateApiLayerProperties: Option<PfnXrEnumerateApiLayerProperties>,
    pub(crate) xrCreateInstance: Option<PfnXrCreateInstance>,
    pub(crate) xrEnumerateInstanceExtensionProperties:
        Option<PfnXrEnumerateInstanceExtensionProperties>,
}

/// An instance's functions (`SDL_OPENXR_INSTANCE_SYM`). Translation of
/// `XrInstancePfns`.
#[derive(Clone, Copy)]
#[allow(dead_code)] // (for the backends)
pub(crate) struct XrInstancePfns {
    pub(crate) xrEnumerateSwapchainFormats: PfnXrEnumerateSwapchainFormats,
    pub(crate) xrCreateSession: PfnXrCreateSession,
    pub(crate) xrGetSystem: PfnXrGetSystem,
    pub(crate) xrCreateSwapchain: PfnXrCreateSwapchain,
    pub(crate) xrEnumerateSwapchainImages: PfnXrEnumerateSwapchainImages,
    pub(crate) xrDestroyInstance: PfnXrDestroyInstance,
    pub(crate) xrDestroySwapchain: PfnXrDestroySwapchain,
}

impl std::fmt::Debug for XrInstancePfns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("XrInstancePfns")
    }
}

/// `openxr_loader` (`openxrdynlib`), the symbols and the refcount.
struct State {
    lib: Option<SharedObject>,
    openxr_load_refcount: i32,
    syms: GlobalPfns,
    /// On Android, we need to initialize the loader with JNI context before use
    #[cfg(target_os = "android")]
    openxr_android_loader_initialized: bool,
}

static STATE: Mutex<State> = Mutex::new(State {
    lib: None,
    openxr_load_refcount: 0,
    syms: GlobalPfns {
        xrGetInstanceProcAddr: None,
        xrEnumerateApiLayerProperties: None,
        xrCreateInstance: None,
        xrEnumerateInstanceExtensionProperties: None,
    },
    #[cfg(target_os = "android")]
    openxr_android_loader_initialized: false,
});

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The loaded global functions (copied out, to be called without the
/// lock: the loader may call back into the application).
#[allow(dead_code)] // (for the backends)
pub(crate) fn globals() -> GlobalPfns {
    state().syms
}

/// The loader's reference count (for the tests).
#[cfg(test)]
pub(crate) fn refcount() -> i32 {
    state().openxr_load_refcount
}

/// Translation of `OPENXR_GetSym()`.
///
/// Note (upstream): it never sets `*failed`, so a loader missing one of
/// the four functions is loaded all the same, and upstream later calls it
/// through `NULL`; here the function stays `None` and its callers fail
/// instead (see [`super::gpu_openxr`]).
#[cfg(not(target_os = "android"))]
fn get_sym<F: Copy>(lib: &SharedObject, fnname: &str, _failed: &mut bool) -> Option<F> {
    // SAFETY: F is the symbol's function pointer type (the callers').
    unsafe { lib.function::<F>(fnname) }.ok()
}

/// `xrGetInstanceProcAddr(instance, name)`: the function, as `F`.
///
/// # Safety
///
/// `F` is the function pointer type of `name`.
#[allow(dead_code)] // (for the backends; Android's loader)
pub(crate) unsafe fn get_instance_proc_addr<F: Copy>(
    gipa: PfnXrGetInstanceProcAddr,
    instance: XrInstance,
    name: &str,
) -> (XrResult, Option<F>) {
    const {
        assert!(size_of::<F>() == size_of::<PfnXrVoidFunction>());
    }
    let Ok(cname) = CString::new(name) else {
        return (XrResult::ERROR_VALIDATION_FAILURE, None);
    };
    let mut function: Option<PfnXrVoidFunction> = None;
    // SAFETY: a name and an out-pointer, as the loader takes them.
    let result = unsafe { gipa(instance, cname.as_ptr(), &mut function) };
    // SAFETY: F is the function's type (the caller's contract), of a
    // pointer's size.
    let f = function.map(|f| unsafe { std::mem::transmute_copy::<PfnXrVoidFunction, F>(&f) });
    (result, f)
}

/// Translation of `OPENXR_InitializeAndroidLoader()`.
#[cfg(target_os = "android")]
fn initialize_android_loader(s: &mut State) -> Result<()> {
    if s.openxr_android_loader_initialized {
        return Ok(());
    }
    let Some(lib) = s.lib.as_ref() else {
        return Err(Error::new("Failed to get xrGetInstanceProcAddr from OpenXR loader. Make sure you're using the Khronos loader (libopenxr_loader.so), not Meta's forwardloader."));
    };

    // The Khronos OpenXR loader (libopenxr_loader.so) properly exports
    // xrGetInstanceProcAddr. Get it directly from the library - this is the
    // standard approach.
    // SAFETY: the symbol's type.
    let Ok(loaderGetProcAddr) =
        (unsafe { lib.function::<PfnXrGetInstanceProcAddr>("xrGetInstanceProcAddr") })
    else {
        return Err(Error::new("Failed to get xrGetInstanceProcAddr from OpenXR loader. Make sure you're using the Khronos loader (libopenxr_loader.so), not Meta's forwardloader."));
    };

    // Get xrInitializeLoaderKHR via xrGetInstanceProcAddr
    // SAFETY: the function's type.
    let (result, initializeLoader) = unsafe {
        get_instance_proc_addr::<PfnXrInitializeLoaderKHR>(
            loaderGetProcAddr,
            XR_NULL_HANDLE,
            "xrInitializeLoaderKHR",
        )
    };
    let Some(initializeLoader) = initializeLoader.filter(|_| result.succeeded()) else {
        return Err(Error::new(format!(
            "Failed to get xrInitializeLoaderKHR (result: {})",
            result.0
        )));
    };

    // Get Android environment info from SDL
    let Some(env) = crate::core::android::env::get_env() else {
        return Err(Error::new("Failed to get Android JNI environment"));
    };

    let Some(vm) = env.get_java_vm() else {
        return Err(Error::new("Failed to get JavaVM from JNIEnv"));
    };

    // Note (upstream): the activity is a local reference of this thread's
    // environment that upstream never deletes (nor does this).
    let Some(activity) = crate::core::android::sdl_android::get_android_activity() else {
        return Err(Error::new("Failed to get Android activity"));
    };

    let loaderInitInfo = XrLoaderInitInfoAndroidKHR {
        ty: XR_TYPE_LOADER_INIT_INFO_ANDROID_KHR,
        next: std::ptr::null(),
        application_vm: vm.as_ptr().cast(),
        application_context: activity.as_ptr(),
    };

    // SAFETY: the structure, which outlives the call.
    let result = unsafe {
        initializeLoader(&loaderInitInfo as *const XrLoaderInitInfoAndroidKHR as *const _)
    };
    if result.failed() {
        return Err(Error::new(format!(
            "xrInitializeLoaderKHR failed with result {}",
            result.0
        )));
    }

    // Store the xrGetInstanceProcAddr function - this one properly handles
    // all pre-instance calls (unlike Meta's forwardloader runtime
    // negotiation)
    s.syms.xrGetInstanceProcAddr = Some(loaderGetProcAddr);

    s.openxr_android_loader_initialized = true;
    Ok(())
}

/// Translation of `SDL_OpenXR_UnloadLibrary()`.
pub(crate) fn unload_library() {
    unload_library_locked(&mut state());
}

fn unload_library_locked(s: &mut State) {
    // Don't actually unload if more than one module is using the libs...
    if s.openxr_load_refcount > 0 {
        s.openxr_load_refcount -= 1;
        if s.openxr_load_refcount == 0 {
            #[cfg(target_os = "android")]
            {
                // On Android/Quest, don't actually unload the library or
                // reset the loader state. The Quest OpenXR runtime doesn't
                // support being re-initialized after teardown.
                // xrInitializeLoaderKHR and xrNegotiateLoaderRuntimeInterface
                // must only be called once. We keep the library loaded and
                // the loader initialized.
                //
                // IMPORTANT: We also keep xrGetInstanceProcAddr intact so we
                // can reload other function pointers on the next LoadLibrary
                // call. Only NULL out the other symbols.
                s.syms = GlobalPfns {
                    xrGetInstanceProcAddr: s.syms.xrGetInstanceProcAddr,
                    ..GlobalPfns::default()
                };
            }
            #[cfg(not(target_os = "android"))]
            {
                // On non-Android, NULL everything and unload
                s.syms = GlobalPfns::default();
                s.lib = None;
            }
        }
    }
}

/// The message of a loader that can't be loaded.
const LOAD_FAILED: &str = "Failed to load OpenXR loader library. On Windows, ensure openxr_loader.dll is in your application directory or PATH. On Linux, install the OpenXR loader package (libopenxr-loader) or set LD_LIBRARY_PATH. You can also use the SDL_HINT_OPENXR_LIBRARY hint to specify the loader path.";

/// Translation of `SDL_OpenXR_LoadLibrary()`: whether all needed symbols
/// were loaded.
pub(crate) fn load_library() -> Result<()> {
    let mut s = state();
    let s = &mut *s;
    let mut result = Ok(());

    // deal with multiple modules (gpu, openxr, etc) needing these symbols...
    let first = s.openxr_load_refcount == 0;
    s.openxr_load_refcount += 1;
    if first {
        // On Android, the library may already be loaded if this is a
        // reload after unload (we don't actually unload on Android to
        // preserve runtime state)
        if !cfg!(target_os = "android") || s.lib.is_none() {
            let path_hint = hints::get(hints::OPENXR_LIBRARY);

            // If a hint was specified, try that first
            if let Some(path_hint) = path_hint.filter(|h| !h.is_empty()) {
                s.lib = SharedObject::load(&path_hint).ok();
            }

            // If no hint or hint failed, try the default library names
            if s.lib.is_none() {
                for name in OPENXR_LIBRARY_NAMES {
                    s.lib = SharedObject::load(name).ok();
                    if s.lib.is_some() {
                        break;
                    }
                }
            }

            if s.lib.is_none() {
                s.openxr_load_refcount -= 1;
                return Err(Error::new(LOAD_FAILED));
            }
        }

        // On Android, we need to initialize the loader before other
        // functions work. OPENXR_InitializeAndroidLoader() will return
        // early if already initialized.
        #[cfg(target_os = "android")]
        if let Err(e) = initialize_android_loader(s) {
            s.lib = None;
            s.openxr_load_refcount -= 1;
            return Err(e);
        }

        #[allow(unused_mut)] // (only Android's path fails)
        let mut failed = false;
        let lib = s.lib.as_ref().expect("loaded above");

        #[cfg(target_os = "android")]
        {
            // On Android with Meta's forwardloader, we need special
            // handling. After calling xrInitializeLoaderKHR, the global
            // functions should be available either as direct exports from
            // the forwardloader or via xrGetInstanceProcAddr(NULL, ...).
            //
            // Try getting functions directly from the forwardloader first
            // since they'll go through the proper forwarding path.

            // First try to get functions directly from the forwardloader library
            // SAFETY: the symbols' types.
            unsafe {
                s.syms.xrEnumerateApiLayerProperties =
                    lib.function("xrEnumerateApiLayerProperties").ok();
                s.syms.xrCreateInstance = lib.function("xrCreateInstance").ok();
                s.syms.xrEnumerateInstanceExtensionProperties =
                    lib.function("xrEnumerateInstanceExtensionProperties").ok();
            }

            // If direct loading failed, fall back to xrGetInstanceProcAddr(NULL, ...)
            let gipa = s
                .syms
                .xrGetInstanceProcAddr
                .expect("set by the initialization");
            fn fallback<F: Copy>(
                gipa: PfnXrGetInstanceProcAddr,
                slot: &mut Option<F>,
                name: &str,
                failed: &mut bool,
            ) {
                if slot.is_none() {
                    // SAFETY: F is the function's type (the callers').
                    let (result, f) =
                        unsafe { get_instance_proc_addr::<F>(gipa, XR_NULL_HANDLE, name) };
                    *slot = f;
                    if result.failed() || slot.is_none() {
                        *failed = true;
                    }
                }
            }
            fallback(
                gipa,
                &mut s.syms.xrEnumerateApiLayerProperties,
                "xrEnumerateApiLayerProperties",
                &mut failed,
            );
            fallback(
                gipa,
                &mut s.syms.xrCreateInstance,
                "xrCreateInstance",
                &mut failed,
            );
            fallback(
                gipa,
                &mut s.syms.xrEnumerateInstanceExtensionProperties,
                "xrEnumerateInstanceExtensionProperties",
                &mut failed,
            );
        }
        #[cfg(not(target_os = "android"))]
        {
            s.syms.xrGetInstanceProcAddr = get_sym(lib, "xrGetInstanceProcAddr", &mut failed);
            s.syms.xrEnumerateApiLayerProperties =
                get_sym(lib, "xrEnumerateApiLayerProperties", &mut failed);
            s.syms.xrCreateInstance = get_sym(lib, "xrCreateInstance", &mut failed);
            s.syms.xrEnumerateInstanceExtensionProperties =
                get_sym(lib, "xrEnumerateInstanceExtensionProperties", &mut failed);
        }

        if failed {
            // in case something got loaded...
            unload_library_locked(s);
            // Note (upstream): no error is set on this path (Android's
            // only); here the error says which step failed.
            result = Err(Error::new(
                "Failed to load the OpenXR loader's global functions",
            ));
        }
    }

    result
}

/// Translation of `SDL_OpenXR_GetXrGetInstanceProcAddr()`.
pub(crate) fn get_xr_get_instance_proc_addr() -> Result<PfnXrGetInstanceProcAddr> {
    state()
        .syms
        .xrGetInstanceProcAddr
        .ok_or_else(|| Error::new("The OpenXR loader has not been loaded"))
}

/// Translation of `SDL_OPENXR_LoadInstanceSymbols()`: `None` if one of the
/// seven can't be had.
///
/// Note (upstream): without `xrGetInstanceProcAddr` (a loader that
/// doesn't export it: see [`get_sym`]) upstream calls it through `NULL`;
/// here that is a failure.
#[allow(dead_code)] // (for the backends)
pub(crate) fn load_instance_symbols(instance: XrInstance) -> Option<XrInstancePfns> {
    let gipa = globals().xrGetInstanceProcAddr?;
    macro_rules! sym {
        ($name:ident) => {{
            // SAFETY: the function's type.
            let (result, f) = unsafe { get_instance_proc_addr(gipa, instance, stringify!($name)) };
            if result != XrResult::SUCCESS {
                return None;
            }
            // (a runtime that answers XR_SUCCESS with no function: upstream
            // would store NULL)
            f?
        }};
    }
    Some(XrInstancePfns {
        xrEnumerateSwapchainFormats: sym!(xrEnumerateSwapchainFormats),
        xrCreateSession: sym!(xrCreateSession),
        xrGetSystem: sym!(xrGetSystem),
        xrCreateSwapchain: sym!(xrCreateSwapchain),
        xrEnumerateSwapchainImages: sym!(xrEnumerateSwapchainImages),
        xrDestroyInstance: sym!(xrDestroyInstance),
        xrDestroySwapchain: sym!(xrDestroySwapchain),
    })
}
