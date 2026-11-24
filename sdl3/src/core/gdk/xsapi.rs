// The parts of the Xbox Services API (XSAPI) that
// src/main/gdk/SDL_sysmain_runapp.cpp of Simple DirectMedia Layer uses,
// declared by hand from Include/xsapi-c/xbox_live_global_c.h of
// microsoft/xbox-live-api (Copyright (c) Microsoft Corporation, MIT
// License). See docs/LICENSING.md, "The Microsoft GDK (WinGDK)".
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Xbox Live's initialization and cleanup, and its library's loader.
//!
//! Upstream links the GDK's `Microsoft.Xbox.Services.C.Thunks.lib`, whose
//! DLL the application ships beside its executable (with `libHttpClient.dll`
//! and `XCurl.dll`, which it needs). Here nothing is linked: the DLL is
//! loaded by its full path in the application's directory (the executable's,
//! [`crate::filesystem::get_base_path`]), only there, the first time it's
//! needed, and stays loaded. Without it [`xbox_services`] fails, and
//! `run_app()` goes on without Xbox Live.
//!
//! The DLL is xbox-live-api's `Build/Microsoft.Xbox.Services.GDK.C.Thunks`
//! (its README names the DLL `Microsoft.Xbox.Services.C.Thunks.dll`), whose
//! module definition file exports `XblInitialize` and `XblCleanupAsync` by
//! name. The declarations are interface facts (layouts, names, function
//! types) of the MIT header; no part of the GDK was read.

#![allow(non_snake_case)]

use std::ffi::c_char;
use std::sync::Mutex;

use windows_sys::core::HRESULT;

use super::sys::{XAsyncBlock, XTaskQueueHandle};
use crate::error::Result;
use crate::loadso::SharedObject;

/// `struct XblInitArgs`, as `xbox_live_global_c.h` declares it for
/// `HC_PLATFORM_GDK`: the platform libHttpClient's `config.h` picks for
/// `_GAMING_DESKTOP`, which upstream's WinGDK build defines (VisualC-GDK's
/// `Gaming.Desktop.x64`; `SDL_platform_defines.h` tests it). There the
/// structure has `queue` and `scid` only: `localStoragePath` is
/// `HC_PLATFORM_WIN32`'s (desktop Windows without the GDK), and the rest
/// other platforms'.
#[repr(C)]
#[derive(Debug)]
pub(crate) struct XblInitArgs {
    /// The queue for XSAPI's own asynchronous work (optional: null for a
    /// queue on the thread pool).
    pub(crate) queue: XTaskQueueHandle,
    /// The Service Configuration ID (SCID), NUL-terminated (case
    /// sensitive).
    pub(crate) scid: *const c_char,
}

/// `XblInitialize()` (`STDAPI`).
pub(crate) type XblInitializeFn = unsafe extern "system" fn(args: *const XblInitArgs) -> HRESULT;

/// `XblCleanupAsync()` (`STDAPI`): Xbox Live's teardown, done when the
/// call is.
pub(crate) type XblCleanupAsyncFn =
    unsafe extern "system" fn(async_block: *mut XAsyncBlock) -> HRESULT;

/// Xbox Live's library, loaded from the application's directory only.
pub(crate) const XBOX_SERVICES_THUNKS_DLL: &str = "Microsoft.Xbox.Services.C.Thunks.dll";

/// The XSAPI functions SDL calls.
#[derive(Debug)]
pub(crate) struct XboxServices {
    _library: SharedObject,
    pub(crate) XblInitialize: XblInitializeFn,
    pub(crate) XblCleanupAsync: XblCleanupAsyncFn,
}

impl XboxServices {
    /// Load the library from the application's directory and every
    /// function, or fail with the loader's error (`Failed loading
    /// <directory>\Microsoft.Xbox.Services.C.Thunks.dll: ...`, or the first
    /// missing function's).
    fn load() -> Result<XboxServices> {
        let path = format!(
            "{}{XBOX_SERVICES_THUNKS_DLL}",
            crate::filesystem::get_base_path()?
        );
        let library = SharedObject::load(&path)?;
        // SAFETY: each type is the function's, as declared above.
        unsafe {
            Ok(XboxServices {
                XblInitialize: library.function("XblInitialize")?,
                XblCleanupAsync: library.function("XblCleanupAsync")?,
                _library: library,
            })
        }
    }
}

/// The loaded library (never unloaded once loaded).
static XBOX_SERVICES: Mutex<Option<&'static XboxServices>> = Mutex::new(None);

/// Xbox Live's library, loaded the first time (a failed load is tried again
/// the next time).
pub(crate) fn xbox_services() -> Result<&'static XboxServices> {
    let mut services = XBOX_SERVICES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(services) = *services {
        return Ok(services);
    }
    let loaded: &'static XboxServices = Box::leak(Box::new(XboxServices::load()?));
    *services = Some(loaded);
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::c_void;
    use std::mem::{align_of, offset_of, size_of};

    #[test]
    fn layouts_match_the_header() {
        // sizeof/offsetof of a C++ program on xbox_live_global_c.h's
        // XblInitArgs with libHttpClient's config.h and _GAMING_DESKTOP
        // (HC_PLATFORM_GDK; mingw-w64, x86_64: 16, align 8, 0, 8; the same
        // pointer arithmetic on 32-bit targets. HC_PLATFORM_WIN32's is 24,
        // with localStoragePath at 16)
        let ptr = size_of::<*mut c_void>();
        assert_eq!(size_of::<XblInitArgs>(), 2 * ptr);
        assert_eq!(align_of::<XblInitArgs>(), ptr);
        assert_eq!(offset_of!(XblInitArgs, queue), 0);
        assert_eq!(offset_of!(XblInitArgs, scid), ptr);
        #[cfg(target_pointer_width = "64")]
        assert_eq!(size_of::<XblInitArgs>(), 16);
    }

    #[test]
    fn the_loader_looks_in_the_application_directory_only() {
        // (the test programs' directory has no Xbox Live library)
        let base = crate::filesystem::get_base_path().unwrap();
        if std::path::Path::new(&format!("{base}{XBOX_SERVICES_THUNKS_DLL}")).exists() {
            eprintln!("note: {XBOX_SERVICES_THUNKS_DLL} is beside the test program");
            return;
        }
        let e = xbox_services().unwrap_err().to_string();
        assert!(
            e.starts_with(&format!("Failed loading {base}{XBOX_SERVICES_THUNKS_DLL}")),
            "{e}"
        );
        // (tried again, failing the same way)
        assert!(xbox_services().is_err());
    }
}
