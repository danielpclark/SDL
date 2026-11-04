// Rust translation of src/video/kmsdrm/SDL_kmsdrmdyn.c, SDL_kmsdrmdyn.h
// and SDL_kmsdrmsym.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Loading libgbm and libdrm at run time (upstream's
//! `SDL_KMSDRM_SHARED` build: `libgbm.so.1` and `libdrm.so.2`).
//!
//! Upstream's `SDL_kmsdrmsym.h` lists every function SDL calls, grouped in
//! the modules `LIBDRM`, `GBM` and `ATOMIC`; `SDL_kmsdrmdyn.c` looks each
//! one up in the libraries it opened (libgbm first, then libdrm) and clears
//! a module's `SDL_KMSDRM_HAVE_*` flag when a required symbol is missing
//! (`SDL_KMSDRM_SYM_OPT` symbols may be missing). Here a module is a struct
//! of function pointers that exists exactly when all of its required
//! symbols were found. As upstream, loading succeeds when `LIBDRM` and
//! `GBM` are there; `ATOMIC` may be missing (`None`).
//!
//! The libraries are reference counted like `kmsdrm_load_refcount`: each
//! user holds a [`KmsdrmSyms`] handle, and the libraries are closed when the
//! last one goes.

#![allow(non_snake_case)] // (the names of SDL_kmsdrmsym.h)
#![allow(dead_code)] // (the symbol table lists every function of SDL_kmsdrmsym.h)

use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void};
use std::sync::{Arc, Mutex, Weak};

use super::sys::*;
use crate::loadso::SharedObject;

const DEBUG_DYNAMIC_KMSDRM: bool = false;

/// The libraries SDL looks symbols up in (`kmsdrmlibs[]`), in order:
/// `SDL_VIDEO_DRIVER_KMSDRM_DYNAMIC_GBM`, then
/// `SDL_VIDEO_DRIVER_KMSDRM_DYNAMIC`.
const KMSDRMLIBS: [&str; 2] = ["libgbm.so.1", "libdrm.so.2"];

/// Look a symbol up in the loaded libraries. Translation of
/// `KMSDRM_GetSym()`: a missing required symbol clears `have_module`
/// ("kill this module").
fn kmsdrm_get_sym<F: Copy>(
    libs: &[SharedObject],
    fnname: &str,
    have_module: &mut bool,
    required: bool,
) -> Option<F> {
    let mut found = None;
    for lib in libs {
        // SAFETY: F is the function pointer type declared for `fnname` in
        // the module tables below, matching the C prototype in
        // SDL_kmsdrmsym.h; the libraries outlive the pointers (they are kept
        // in the same KmsdrmSyms).
        if let Ok(f) = unsafe { lib.function::<F>(fnname) } {
            found = Some((f, lib.name()));
            break;
        }
    }

    if DEBUG_DYNAMIC_KMSDRM {
        match &found {
            Some((_, libname)) => crate::log!("KMSDRM: Found '{fnname}' in {libname}"),
            None => crate::log!("KMSDRM: Symbol '{fnname}' NOT FOUND!"),
        }
    }

    if found.is_none() && required {
        *have_module = false; // kill this module.
    }

    found.map(|(f, _)| f)
}

/// Declare a module of `SDL_kmsdrmsym.h`: a struct of the module's function
/// pointers that loads when all of its required ones are found; the
/// optional ones (`SDL_KMSDRM_SYM_OPT`) are `Option`s.
macro_rules! kmsdrm_module {
    ($(#[$doc:meta])* $Name:ident {
        $($sym:ident: $ty:ty,)*
    } $(optional {
        $($osym:ident: $oty:ty,)*
    })?) => {
        $(#[$doc])*
        pub(crate) struct $Name {
            $(pub(crate) $sym: $ty,)*
            $($(pub(crate) $osym: Option<$oty>,)*)?
        }

        impl $Name {
            /// Look up the module's symbols (`SDL_KMSDRM_HAVE_*` stays set
            /// only if every required one is found).
            fn load(libs: &[SharedObject]) -> Option<$Name> {
                let mut have = true; // default yes
                $( let $sym = kmsdrm_get_sym::<$ty>(libs, stringify!($sym), &mut have, true); )*
                $($( let $osym = kmsdrm_get_sym::<$oty>(libs, stringify!($osym), &mut have, false); )*)?
                if !have {
                    return None;
                }
                Some($Name { $($sym: $sym?,)* $($($osym,)*)? })
            }
        }
    };
}

kmsdrm_module! {
    /// `SDL_KMSDRM_MODULE(LIBDRM)`: libdrm.
    Libdrm {
        drmModeFreeResources: unsafe extern "C" fn(*mut drmModeRes),
        drmModeFreeFB: unsafe extern "C" fn(*mut drmModeFB),
        drmModeFreeCrtc: unsafe extern "C" fn(*mut drmModeCrtc),
        drmModeFreeConnector: unsafe extern "C" fn(*mut drmModeConnector),
        drmModeFreeEncoder: unsafe extern "C" fn(*mut drmModeEncoder),
        drmGetCap: unsafe extern "C" fn(c_int, u64, *mut u64) -> c_int,
        drmSetMaster: unsafe extern "C" fn(c_int) -> c_int,
        drmDropMaster: unsafe extern "C" fn(c_int) -> c_int,
        drmAuthMagic: unsafe extern "C" fn(c_int, drm_magic_t) -> c_int,
        drmModeGetResources: unsafe extern "C" fn(c_int) -> *mut drmModeRes,
        drmModeAddFB: unsafe extern "C" fn(c_int, u32, u32, u8, u8, u32, u32, *mut u32) -> c_int,
        drmModeRmFB: unsafe extern "C" fn(c_int, u32) -> c_int,
        drmModeGetFB: unsafe extern "C" fn(c_int, u32) -> *mut drmModeFB,
        drmModeGetCrtc: unsafe extern "C" fn(c_int, u32) -> *mut drmModeCrtc,
        drmModeSetCrtc: unsafe extern "C" fn(c_int, u32, u32, u32, u32, *mut u32, c_int, *mut drmModeModeInfo) -> c_int,
        drmModeSetCursor: unsafe extern "C" fn(c_int, u32, u32, u32, u32) -> c_int,
        drmModeSetCursor2: unsafe extern "C" fn(c_int, u32, u32, u32, u32, i32, i32) -> c_int,
        drmModeMoveCursor: unsafe extern "C" fn(c_int, u32, c_int, c_int) -> c_int,
        drmModeGetEncoder: unsafe extern "C" fn(c_int, u32) -> *mut drmModeEncoder,
        drmModeGetConnector: unsafe extern "C" fn(c_int, u32) -> *mut drmModeConnector,
        drmHandleEvent: unsafe extern "C" fn(c_int, *mut drmEventContext) -> c_int,
        drmModePageFlip: unsafe extern "C" fn(c_int, u32, u32, u32, *mut c_void) -> c_int,

        // Planes stuff.
        drmSetClientCap: unsafe extern "C" fn(c_int, u64, u64) -> c_int,
        drmModeGetPlaneResources: unsafe extern "C" fn(c_int) -> *mut drmModePlaneRes,
        drmModeGetPlane: unsafe extern "C" fn(c_int, u32) -> *mut drmModePlane,
        drmModeObjectGetProperties: unsafe extern "C" fn(c_int, u32, u32) -> *mut drmModeObjectProperties,
        drmModeObjectSetProperty: unsafe extern "C" fn(c_int, u32, u32, u32, u64) -> c_int,
        drmModeGetProperty: unsafe extern "C" fn(c_int, u32) -> *mut drmModePropertyRes,

        drmModeFreeProperty: unsafe extern "C" fn(*mut drmModePropertyRes),
        drmModeFreeObjectProperties: unsafe extern "C" fn(*mut drmModeObjectProperties),
        drmModeFreePlane: unsafe extern "C" fn(*mut drmModePlane),
        drmModeFreePlaneResources: unsafe extern "C" fn(*mut drmModePlaneRes),
        drmModeSetPlane: unsafe extern "C" fn(c_int, u32, u32, u32, u32, i32, i32, u32, u32, u32, u32, u32, u32) -> c_int,
        // Planes stuff ends.
    } optional {
        drmModeAddFB2: unsafe extern "C" fn(c_int, u32, u32, u32, *const u32, *const u32, *const u32, *mut u32, u32) -> c_int,
        drmModeAddFB2WithModifiers: unsafe extern "C" fn(c_int, u32, u32, u32, *const u32, *const u32, *const u32, *const u64, *mut u32, u32) -> c_int,
        drmModeGetConnectorTypeName: unsafe extern "C" fn(u32) -> *const c_char,
    }
}

kmsdrm_module! {
    /// `SDL_KMSDRM_MODULE(GBM)`: libgbm.
    Gbm {
        gbm_device_is_format_supported: unsafe extern "C" fn(*mut gbm_device, u32, u32) -> c_int,
        gbm_device_destroy: unsafe extern "C" fn(*mut gbm_device),
        gbm_create_device: unsafe extern "C" fn(c_int) -> *mut gbm_device,
        gbm_bo_get_width: unsafe extern "C" fn(*mut gbm_bo) -> c_uint,
        gbm_bo_get_height: unsafe extern "C" fn(*mut gbm_bo) -> c_uint,
        gbm_bo_get_stride: unsafe extern "C" fn(*mut gbm_bo) -> u32,
        gbm_bo_get_format: unsafe extern "C" fn(*mut gbm_bo) -> u32,
        gbm_bo_get_handle: unsafe extern "C" fn(*mut gbm_bo) -> gbm_bo_handle,
        gbm_bo_write: unsafe extern "C" fn(*mut gbm_bo, *const c_void, usize) -> c_int,
        gbm_bo_get_device: unsafe extern "C" fn(*mut gbm_bo) -> *mut gbm_device,
        gbm_bo_set_user_data: unsafe extern "C" fn(*mut gbm_bo, *mut c_void, gbm_bo_user_data_destroy_t),
        gbm_bo_get_user_data: unsafe extern "C" fn(*mut gbm_bo) -> *mut c_void,
        gbm_bo_destroy: unsafe extern "C" fn(*mut gbm_bo),
        gbm_bo_create: unsafe extern "C" fn(*mut gbm_device, u32, u32, u32, u32) -> *mut gbm_bo,
        gbm_surface_create: unsafe extern "C" fn(*mut gbm_device, u32, u32, u32, u32) -> *mut gbm_surface,
        gbm_surface_destroy: unsafe extern "C" fn(*mut gbm_surface),
        gbm_surface_lock_front_buffer: unsafe extern "C" fn(*mut gbm_surface) -> *mut gbm_bo,
        gbm_surface_release_buffer: unsafe extern "C" fn(*mut gbm_surface, *mut gbm_bo),
    } optional {
        gbm_bo_get_modifier: unsafe extern "C" fn(*mut gbm_bo) -> u64,
        gbm_bo_get_plane_count: unsafe extern "C" fn(*mut gbm_bo) -> c_int,
        gbm_bo_get_offset: unsafe extern "C" fn(*mut gbm_bo, c_int) -> u32,
        gbm_bo_get_stride_for_plane: unsafe extern "C" fn(*mut gbm_bo, c_int) -> u32,
        gbm_bo_get_handle_for_plane: unsafe extern "C" fn(*mut gbm_bo, c_int) -> gbm_bo_handle,
    }
}

kmsdrm_module! {
    /// `SDL_KMSDRM_MODULE(ATOMIC)`: libdrm's atomic interface.
    Atomic {
        drmIoctl: unsafe extern "C" fn(c_int, c_ulong, *mut c_void) -> c_int,
        drmModeAtomicAlloc: unsafe extern "C" fn() -> *mut drmModeAtomicReq,
        drmModeAtomicFree: unsafe extern "C" fn(*mut drmModeAtomicReq),
        drmModeAtomicCommit: unsafe extern "C" fn(c_int, *mut drmModeAtomicReq, u32, *mut c_void) -> c_int,
        drmModeAtomicAddProperty: unsafe extern "C" fn(*mut drmModeAtomicReq, u32, u32, u64) -> c_int,
        drmModeCreatePropertyBlob: unsafe extern "C" fn(c_int, *const c_void, usize, *mut u32) -> c_int,
    }
}

/// The loaded libdrm and libgbm functions: the modules of
/// `SDL_kmsdrmsym.h` (`SDL_KMSDRM_HAVE_*`).
pub(crate) struct KmsdrmSyms {
    /// `SDL_KMSDRM_HAVE_LIBDRM`
    pub(crate) drm: Libdrm,
    /// `SDL_KMSDRM_HAVE_GBM`
    pub(crate) gbm: Gbm,
    /// `SDL_KMSDRM_HAVE_ATOMIC` (`None`: 0; upstream calls its functions
    /// regardless, see [`super::video`]'s `set_client_atomic_caps()`)
    pub(crate) atomic: Option<Atomic>,
    /// The libraries the functions live in, unloaded on drop (after the
    /// function pointers above, which are never used past that point).
    _libs: Vec<SharedObject>,
}

impl KmsdrmSyms {
    /// Open the libraries and look up all the symbols; `None` if a required
    /// module isn't all there. The body of `SDL_KMSDRM_LoadSymbols()`.
    fn load() -> Option<KmsdrmSyms> {
        let libs: Vec<SharedObject> = KMSDRMLIBS
            .iter()
            .filter_map(|libname| SharedObject::load(libname).ok())
            .collect();

        let drm = Libdrm::load(&libs);
        let gbm = Gbm::load(&libs);
        let atomic = Atomic::load(&libs);

        // All required symbols loaded (in case something got loaded
        // otherwise, dropping `libs` unloads it).
        Some(KmsdrmSyms {
            drm: drm?,
            gbm: gbm?,
            atomic,
            _libs: libs,
        })
    }
}

/// The loaded symbols, shared while anyone holds them
/// (`kmsdrm_load_refcount`).
static LOADED: Mutex<Weak<KmsdrmSyms>> = Mutex::new(Weak::new());

/// Load the libraries (or take another reference to them). Translation of
/// `SDL_KMSDRM_LoadSymbols()`; dropping the handle is
/// `SDL_KMSDRM_UnloadSymbols()`.
pub(crate) fn load_symbols() -> Option<Arc<KmsdrmSyms>> {
    let mut loaded = LOADED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(syms) = loaded.upgrade() {
        return Some(syms);
    }
    let syms = Arc::new(KmsdrmSyms::load()?);
    *loaded = Arc::downgrade(&syms);
    Some(syms)
}
