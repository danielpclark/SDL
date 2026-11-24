// The parts of the Microsoft GDK's API that src/core/gdk/SDL_gdk.cpp of
// Simple DirectMedia Layer uses, declared by hand: the task queue and the
// asynchronous calls from XTaskQueue.h and XAsync.h of microsoft/libHttpClient
// (Copyright (c) Microsoft Corporation, MIT License), and XUser's from
// Microsoft's public documentation of the GDK. See docs/LICENSING.md, "The
// Microsoft GDK (WinGDK)".
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Gaming Runtime's functions SDL calls, and their loader.
//!
//! Upstream links the GDK's `xgameruntime.lib`. Here nothing is linked: the
//! Gaming Runtime that Gaming Services installs (`xgameruntime.dll`) is
//! loaded by name at run time, the first time it's needed, and stays loaded
//! (the handles it gives out outlive anything that could unload it). Without
//! it [`runtime`] fails, and so does what needs it: there's no runtime on
//! Wine or on the CI runners.
//!
//! The declarations are interface facts (layouts, names, values, function
//! types): the task queue's and `XAsyncBlock`'s from libHttpClient's MIT
//! headers, which are the GDK's own (`XTaskQueue.h`, `XAsync.h`), and
//! XUser's from Microsoft's documentation of the GDK (`XUserHandle`,
//! `XUserAddOptions`, `XUserAddAsync()`, `XUserAddResult()`,
//! `XUserCloseHandle()`, the Gaming Runtime's error codes). No part of the
//! GDK itself (its headers, libraries or DLLs' export tables) was read.

#![allow(non_camel_case_types, non_snake_case)]

use std::ffi::c_void;
use std::sync::Mutex;

use windows_sys::core::HRESULT;

use crate::error::Result;
use crate::loadso::SharedObject;

// --- XTaskQueue.h ---

/// `struct XTaskQueueObject` (opaque).
#[repr(C)]
#[derive(Debug)]
pub struct XTaskQueueObject {
    _private: [u8; 0],
}

/// `XTaskQueueHandle`: a reference to a task queue, closed with
/// `XTaskQueueCloseHandle()`.
pub type XTaskQueueHandle = *mut XTaskQueueObject;

/// `enum class XTaskQueueDispatchMode : uint32_t`.
pub(crate) type XTaskQueueDispatchMode = u32;
/// `XTaskQueueDispatchMode::Manual`: dispatched by `XTaskQueueDispatch()`.
pub(crate) const XTASKQUEUE_DISPATCH_MODE_MANUAL: XTaskQueueDispatchMode = 0;
/// `XTaskQueueDispatchMode::ThreadPool`: dispatched on the system's thread
/// pool.
pub(crate) const XTASKQUEUE_DISPATCH_MODE_THREAD_POOL: XTaskQueueDispatchMode = 1;

/// `enum class XTaskQueuePort : uint32_t`.
pub(crate) type XTaskQueuePort = u32;
/// `XTaskQueuePort::Work`.
pub(crate) const XTASKQUEUE_PORT_WORK: XTaskQueuePort = 0;
/// `XTaskQueuePort::Completion`.
pub(crate) const XTASKQUEUE_PORT_COMPLETION: XTaskQueuePort = 1;

/// `XTaskQueueTerminatedCallback` (`CALLBACK`).
pub(crate) type XTaskQueueTerminatedCallback = unsafe extern "system" fn(context: *mut c_void);

/// `XTaskQueueCreate()`.
pub(crate) type XTaskQueueCreateFn = unsafe extern "system" fn(
    work_dispatch_mode: XTaskQueueDispatchMode,
    completion_dispatch_mode: XTaskQueueDispatchMode,
    queue: *mut XTaskQueueHandle,
) -> HRESULT;
/// `XTaskQueueDuplicateHandle()`.
pub(crate) type XTaskQueueDuplicateHandleFn = unsafe extern "system" fn(
    queue_handle: XTaskQueueHandle,
    duplicated_handle: *mut XTaskQueueHandle,
) -> HRESULT;
/// `XTaskQueueDispatch()` (`STDAPI_(bool)`).
pub(crate) type XTaskQueueDispatchFn = unsafe extern "system" fn(
    queue: XTaskQueueHandle,
    port: XTaskQueuePort,
    timeout_in_ms: u32,
) -> bool;
/// `XTaskQueueCloseHandle()` (`STDAPI_(void)`).
pub(crate) type XTaskQueueCloseHandleFn = unsafe extern "system" fn(queue: XTaskQueueHandle);
/// `XTaskQueueTerminate()`.
pub(crate) type XTaskQueueTerminateFn = unsafe extern "system" fn(
    queue: XTaskQueueHandle,
    wait: bool,
    callback_context: *mut c_void,
    callback: Option<XTaskQueueTerminatedCallback>,
) -> HRESULT;

// --- XAsync.h ---

/// `XAsyncCompletionRoutine` (`CALLBACK`).
pub(crate) type XAsyncCompletionRoutine = unsafe extern "system" fn(async_block: *mut XAsyncBlock);

/// `struct XAsyncBlock`: an asynchronous call's state, which must stay where
/// it is until the call is done.
#[repr(C)]
#[derive(Debug)]
pub(crate) struct XAsyncBlock {
    pub(crate) queue: XTaskQueueHandle,
    pub(crate) context: *mut c_void,
    pub(crate) callback: Option<XAsyncCompletionRoutine>,
    /// "Internal use only".
    pub(crate) internal: [u8; size_of::<*mut c_void>() * 4],
}

impl Default for XAsyncBlock {
    /// `XAsyncBlock block = { 0 };`
    fn default() -> XAsyncBlock {
        XAsyncBlock {
            queue: std::ptr::null_mut(),
            context: std::ptr::null_mut(),
            callback: None,
            internal: [0; size_of::<*mut c_void>() * 4],
        }
    }
}

/// `XAsyncGetStatus()`: `E_PENDING` until the call is done; with `wait`,
/// waits until it's done (or canceled).
pub(crate) type XAsyncGetStatusFn =
    unsafe extern "system" fn(async_block: *mut XAsyncBlock, wait: bool) -> HRESULT;

// --- XUser.h (from the documentation) ---

/// `struct XUser` (opaque).
#[repr(C)]
#[derive(Debug)]
pub struct XUser {
    _private: [u8; 0],
}

/// `XUserHandle`: a reference to a user, closed with `XUserCloseHandle()`.
pub type XUserHandle = *mut XUser;

/// `enum class XUserAddOptions : uint32_t`.
pub(crate) type XUserAddOptions = u32;
/// `XUserAddOptions::None`.
pub(crate) const XUSER_ADD_OPTIONS_NONE: XUserAddOptions = 0x00;
/// `XUserAddOptions::AddDefaultUserSilently`.
pub(crate) const XUSER_ADD_OPTIONS_ADD_DEFAULT_USER_SILENTLY: XUserAddOptions = 0x01;
/// `XUserAddOptions::AllowGuests`.
pub(crate) const XUSER_ADD_OPTIONS_ALLOW_GUESTS: XUserAddOptions = 0x02;
/// `XUserAddOptions::AddDefaultUserAllowingUI`.
pub(crate) const XUSER_ADD_OPTIONS_ADD_DEFAULT_USER_ALLOWING_UI: XUserAddOptions = 0x04;

/// `XUserAddAsync()`.
pub(crate) type XUserAddAsyncFn =
    unsafe extern "system" fn(options: XUserAddOptions, async_block: *mut XAsyncBlock) -> HRESULT;
/// `XUserAddResult()`.
pub(crate) type XUserAddResultFn =
    unsafe extern "system" fn(async_block: *mut XAsyncBlock, new_user: *mut XUserHandle) -> HRESULT;
/// `XUserCloseHandle()`.
pub(crate) type XUserCloseHandleFn = unsafe extern "system" fn(user: XUserHandle);

// --- error codes ---

/// `E_PENDING` (`winerror.h`): an asynchronous call isn't done.
pub(crate) const E_PENDING: HRESULT = 0x8000_000A_u32 as HRESULT;
/// `E_GAMERUNTIME_NOT_INITIALIZED`: the Gaming Runtime isn't initialized.
pub(crate) const E_GAMERUNTIME_NOT_INITIALIZED: HRESULT = 0x8924_0100_u32 as HRESULT;
/// `E_GAMERUNTIME_DLL_NOT_FOUND`: the Gaming Runtime library
/// (`xgameruntime.dll`) wasn't found.
pub(crate) const E_GAMERUNTIME_DLL_NOT_FOUND: HRESULT = 0x8924_0101_u32 as HRESULT;

// --- the loader ---

/// The Gaming Runtime's library, loaded by name (the system's search order:
/// Gaming Services installs it in System32).
pub(crate) const XGAMERUNTIME_DLL: &str = "xgameruntime.dll";

/// The Gaming Runtime's functions SDL calls.
#[derive(Debug)]
pub(crate) struct Runtime {
    _library: SharedObject,
    pub(crate) XTaskQueueCreate: XTaskQueueCreateFn,
    pub(crate) XTaskQueueDuplicateHandle: XTaskQueueDuplicateHandleFn,
    pub(crate) XTaskQueueDispatch: XTaskQueueDispatchFn,
    pub(crate) XTaskQueueCloseHandle: XTaskQueueCloseHandleFn,
    pub(crate) XTaskQueueTerminate: XTaskQueueTerminateFn,
    pub(crate) XAsyncGetStatus: XAsyncGetStatusFn,
    pub(crate) XUserAddAsync: XUserAddAsyncFn,
    pub(crate) XUserAddResult: XUserAddResultFn,
    pub(crate) XUserCloseHandle: XUserCloseHandleFn,
}

impl Runtime {
    /// Load the library and every function, or fail with the loader's
    /// error (`Failed loading xgameruntime.dll: ...`, or the first missing
    /// function's `Failed loading XTaskQueueCreate: ...`).
    fn load() -> Result<Runtime> {
        let library = SharedObject::load(XGAMERUNTIME_DLL)?;
        // SAFETY: each type is the function's, as declared above.
        unsafe {
            Ok(Runtime {
                XTaskQueueCreate: library.function("XTaskQueueCreate")?,
                XTaskQueueDuplicateHandle: library.function("XTaskQueueDuplicateHandle")?,
                XTaskQueueDispatch: library.function("XTaskQueueDispatch")?,
                XTaskQueueCloseHandle: library.function("XTaskQueueCloseHandle")?,
                XTaskQueueTerminate: library.function("XTaskQueueTerminate")?,
                XAsyncGetStatus: library.function("XAsyncGetStatus")?,
                XUserAddAsync: library.function("XUserAddAsync")?,
                XUserAddResult: library.function("XUserAddResult")?,
                XUserCloseHandle: library.function("XUserCloseHandle")?,
                _library: library,
            })
        }
    }
}

/// The loaded runtime (never unloaded once loaded).
static RUNTIME: Mutex<Option<&'static Runtime>> = Mutex::new(None);

/// The Gaming Runtime, loaded the first time (a failed load is tried again
/// the next time).
pub(crate) fn runtime() -> Result<&'static Runtime> {
    let mut runtime = RUNTIME.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(runtime) = *runtime {
        return Ok(runtime);
    }
    let loaded: &'static Runtime = Box::leak(Box::new(Runtime::load()?));
    *runtime = Some(loaded);
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    #[test]
    fn layouts_match_the_headers() {
        // sizeof/offsetof of a C++ program on libHttpClient's XAsync.h and
        // XTaskQueue.h (mingw-w64, x86_64: 56, 0, 8, 16, 24; the same
        // pointer arithmetic on 32-bit targets)
        let ptr = size_of::<*mut c_void>();
        assert_eq!(size_of::<XAsyncBlock>(), 7 * ptr);
        assert_eq!(align_of::<XAsyncBlock>(), ptr);
        assert_eq!(offset_of!(XAsyncBlock, queue), 0);
        assert_eq!(offset_of!(XAsyncBlock, context), ptr);
        assert_eq!(offset_of!(XAsyncBlock, callback), 2 * ptr);
        assert_eq!(offset_of!(XAsyncBlock, internal), 3 * ptr);
        #[cfg(target_pointer_width = "64")]
        assert_eq!(size_of::<XAsyncBlock>(), 56);
        assert_eq!(size_of::<XTaskQueueHandle>(), ptr);
        assert_eq!(size_of::<XUserHandle>(), ptr);
        // (the enumerations are uint32_t; C++'s bool is a byte)
        assert_eq!(size_of::<XTaskQueueDispatchMode>(), 4);
        assert_eq!(size_of::<XTaskQueuePort>(), 4);
        assert_eq!(size_of::<XUserAddOptions>(), 4);
        assert_eq!(size_of::<bool>(), 1);
        assert_eq!(size_of::<Option<XAsyncCompletionRoutine>>(), ptr);
    }

    #[test]
    fn constants_match_the_headers_and_documentation() {
        // XTaskQueue.h's enumerations, in declaration order
        assert_eq!(XTASKQUEUE_DISPATCH_MODE_MANUAL, 0);
        assert_eq!(XTASKQUEUE_DISPATCH_MODE_THREAD_POOL, 1);
        assert_eq!(XTASKQUEUE_PORT_WORK, 0);
        assert_eq!(XTASKQUEUE_PORT_COMPLETION, 1);
        // XUserAddOptions, as documented
        assert_eq!(XUSER_ADD_OPTIONS_NONE, 0x00);
        assert_eq!(XUSER_ADD_OPTIONS_ADD_DEFAULT_USER_SILENTLY, 0x01);
        assert_eq!(XUSER_ADD_OPTIONS_ALLOW_GUESTS, 0x02);
        assert_eq!(XUSER_ADD_OPTIONS_ADD_DEFAULT_USER_ALLOWING_UI, 0x04);
        assert_eq!(E_PENDING as u32, 0x8000000A);
        assert_eq!(E_GAMERUNTIME_NOT_INITIALIZED as u32, 0x89240100);
        assert_eq!(E_GAMERUNTIME_DLL_NOT_FOUND as u32, 0x89240101);
    }

    #[test]
    fn the_loader_fails_cleanly_without_the_runtime() {
        // Wine and the Windows CI runners have no Gaming Runtime (the
        // library isn't there); where there is one, it's loaded once.
        match runtime() {
            Err(e) => {
                // (the library's, or a function's)
                assert!(e.to_string().starts_with("Failed loading "), "{e}");
                crate::test_support::skip(
                    "gdk-runtime",
                    format_args!("no Gaming Runtime here ({e})"),
                );
                // (tried again, failing the same way)
                assert!(runtime().is_err());
            }
            Ok(first) => {
                assert!(std::ptr::eq(first, runtime().unwrap()));
            }
        }
    }
}
