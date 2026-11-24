// Rust translation of src/main/gdk/SDL_sysmain_runapp.cpp from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `SDL_RunApp()` for the Microsoft GDK for PC (the `gdk` feature,
//! upstream's WinGDK): the Gaming Runtime initialized, SDL's task queue made
//! the process's, Xbox Live initialized when the game has a title id (and
//! its library is there), the application-state notifications registered
//! around the main function, and all of it undone after. Without the
//! Gaming Runtime it shows upstream's message box and returns -1, without
//! calling the main function.

use std::ffi::CString;

use crate::core::gdk::sys::{runtime, E_GAMERUNTIME_DLL_NOT_FOUND};
use crate::core::gdk::xsapi::{xbox_services, XblInitArgs, XBOX_SERVICES_THUNKS_DLL};
use crate::core::gdk::{
    get_task_queue, quit_task_queue, register_change_notifications, unregister_change_notifications,
};
use crate::error::Error;
use crate::log::Category;
use crate::video::messagebox::{show_simple_message_box, MessageBoxFlags};

/// Translation of the WinGDK `SDL_RunApp()`.
pub(super) fn run_app(main_function: impl FnOnce(&[String]) -> i32) -> i32 {
    // (WIN_CheckDefaultArgcArgv() and its message box: the arguments are
    // std::env::args(), already converted from the wide command line, as
    // on Win32: see super::call_main_function())

    let mut result = -1;
    // (upstream links the runtime: there XGameRuntimeInitialize() returns
    // E_GAMERUNTIME_DLL_NOT_FOUND when its library isn't found; here that's
    // the loader's failure)
    let runtime = runtime();
    let hr = match runtime {
        // SAFETY: no preconditions; uninitialized below.
        Ok(runtime) => unsafe { (runtime.XGameRuntimeInitialize)() },
        Err(_) => E_GAMERUNTIME_DLL_NOT_FOUND,
    };

    // (SDL_GetGDKTaskQueue(): a reference of run_app's own)
    let initialized = match runtime {
        Ok(runtime) if hr >= 0 => get_task_queue().ok().map(|queue| (runtime, queue)),
        _ => None,
    };
    if let Some((runtime, task_queue)) = initialized {
        let mut titleid: u32 = 0;

        // SAFETY: the handle is open (the runtime duplicates it).
        unsafe { (runtime.XTaskQueueSetCurrentProcessTaskQueue)(task_queue) };
        // FIXME (upstream): the process task queue keeps a reference of its
        // own to SDL's queue (XTaskQueueSetCurrentProcessTaskQueue()
        // duplicates the handle), which is never given back: after the
        // teardown below it's a terminated queue, and its handle is still
        // open at exit (maybe the leak upstream's FIXME there sees).

        // Try to get the title ID and initialize Xbox Live
        // (the SCID's buffer lives until the end, as upstream's)
        let mut _scid_buffer = None;
        // SAFETY: titleid is a valid out pointer.
        let hr = unsafe { (runtime.XGameGetXboxTitleId)(&mut titleid) };
        if hr >= 0 {
            let scid = CString::new(format!("00000000-0000-0000-0000-0000{titleid:08X}"))
                .expect("no NUL in a hexadecimal number");
            let xbl_args = XblInitArgs {
                queue: task_queue,
                scid: scid.as_ptr(),
            };
            _scid_buffer = Some(scid);
            // (Xbox Live's library is the application's to ship: without
            // it, Xbox Live isn't initialized, and the application is told)
            match xbox_services() {
                Ok(xbl) => {
                    // FIXME (upstream): XblInitialize()'s result isn't
                    // checked, and XblCleanupAsync() is never called.
                    // SAFETY: xbl_args and its strings outlive the call (the
                    // SCID is copied).
                    let _ = unsafe { (xbl.XblInitialize)(&xbl_args) };
                }
                Err(e) => crate::log::info!(
                    Category::Application,
                    "[GDK] Note: Xbox Live isn't initialized without {XBOX_SERVICES_THUNKS_DLL} in the application's directory ({e})"
                ),
            }
        } else {
            // (SDL_SetError(): the main function can read it upstream; here
            // nothing can, so it's logged too)
            let e = Error::new(
                "[GDK] Unable to get titleid. Will not call XblInitialize. Check MicrosoftGame.config!",
            );
            crate::log::info!(Category::Application, "{e}");
        }

        match register_change_notifications() {
            // Note (upstream): upstream returns -1 here without the
            // teardown below: the notifications registered so far (and the
            // suspend event) stay, as do the task queue, Xbox Live and the
            // runtime. Fixed here: the main function isn't called, and the
            // teardown runs (unregister_change_notifications() skips what
            // wasn't registered). (SDL_SetError(): nothing reads it here,
            // so it's logged.)
            Err(e) => crate::log::error!(Category::Application, "{e}"),
            Ok(()) => {
                // Run the application main() code
                result = super::call_main_function(main_function);
            }
        }

        unregister_change_notifications();

        // (upstream terminates, dispatches and closes the queue's first
        // handle, which is SDL's global one there: here run_app closes its
        // reference, and quit_task_queue() does upstream's with SDL's)
        // SAFETY: the reference is run_app's, closed once.
        unsafe { (runtime.XTaskQueueCloseHandle)(task_queue) };
        quit_task_queue();

        // SAFETY: initialized above.
        unsafe { (runtime.XGameRuntimeUninitialize)() };
    } else {
        // FIXME (upstream): when the runtime was initialized and the task
        // queue failed, the runtime isn't uninitialized, and the message
        // doesn't say what failed (nor does it when
        // XGameRuntimeInitialize() fails with another error).
        if hr == E_GAMERUNTIME_DLL_NOT_FOUND {
            let _ = show_simple_message_box(
                MessageBoxFlags::ERROR,
                "Fatal Error",
                "[GDK] Gaming Runtime library not found (xgameruntime.dll)",
                None,
            );
        } else {
            let _ = show_simple_message_box(
                MessageBoxFlags::ERROR,
                "Fatal Error",
                "[GDK] Could not initialize - aborting",
                None,
            );
        }
    }

    result
}
