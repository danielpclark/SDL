// Rust translation of src/core/gdk/SDL_gdk.cpp and SDL_gdk.h from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Microsoft GDK for PC (upstream's WinGDK build, the `gdk` feature):
//! SDL's global task queue, which the Windows video driver's event pump
//! dispatches, the application-state notifications (suspend and resume,
//! constrained and unconstrained), and the default user.
//!
//! The Gaming Runtime is loaded at run time ([`sys::runtime`]); without it
//! the task queue and the default user fail, as upstream's calls into the
//! runtime do when its library isn't there. The notifications are
//! Windows' (`appnotify.h`), not the runtime's.

pub(crate) mod sys;

use std::ffi::c_void;
use std::sync::{Mutex, MutexGuard};

use windows_sys::Win32::Foundation::{CloseHandle, DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE};
use windows_sys::Win32::System::Threading::{
    CreateEventExW, GetCurrentProcess, ResetEvent, SetEvent, WaitForSingleObject,
    EVENT_MODIFY_STATE, INFINITE, SYNCHRONIZATION_SYNCHRONIZE,
};
use windows_sys::Win32::UI::Shell::{
    RegisterAppConstrainedChangeNotification, RegisterAppStateChangeNotification,
    UnregisterAppConstrainedChangeNotification, UnregisterAppStateChangeNotification,
    PAPPCONSTRAIN_REGISTRATION, PAPPSTATE_REGISTRATION,
};

use self::sys::{
    runtime, XAsyncBlock, XTaskQueueHandle, XUserHandle, E_GAMERUNTIME_DLL_NOT_FOUND,
    XTASKQUEUE_DISPATCH_MODE_MANUAL, XTASKQUEUE_DISPATCH_MODE_THREAD_POOL,
    XTASKQUEUE_PORT_COMPLETION, XUSER_ADD_OPTIONS_ADD_DEFAULT_USER_ALLOWING_UI,
};
use super::windows::error_from_hresult;
use crate::error::{Error, Result};
use crate::events::keyboard;
use crate::events::queue::send_app_event;
use crate::events::EventType;
use crate::log::Category;

/// Translation of `GDK_GlobalTaskQueue` (the handle, or 0), behind a lock.
static GLOBAL_TASK_QUEUE: Mutex<usize> = Mutex::new(0);

fn global_task_queue() -> MutexGuard<'static, usize> {
    GLOBAL_TASK_QUEUE.lock().unwrap_or_else(|e| e.into_inner())
}

/// A reference to SDL's global task queue (work on the thread pool,
/// completions dispatched by the event pump), creating the queue the first
/// time. The caller closes it with `XTaskQueueCloseHandle()`. Translation
/// of `SDL_GetGDKTaskQueue()`.
///
/// Note (upstream): upstream gives the first caller the global queue's own
/// handle ("so they can clean it up"), which leaves the queue SDL
/// dispatches dangling once they close it, as the documentation tells
/// them to (and the next call duplicates a closed handle). Here every
/// caller gets a reference of its own, and the global one is SDL's,
/// released by [`quit_task_queue`]. The lock makes the first call's
/// creation thread-safe too (upstream's isn't).
pub(crate) fn get_task_queue() -> Result<XTaskQueueHandle> {
    let mut global = global_task_queue();

    // If this is the first call, first create the global task queue.
    if *global == 0 {
        // (without the runtime, as upstream's XTaskQueueCreate() fails)
        let runtime =
            runtime().map_err(|_| Error::new("[GDK] Could not create global task queue"))?;
        let mut queue: XTaskQueueHandle = std::ptr::null_mut();
        // SAFETY: queue is a valid out pointer.
        let hr = unsafe {
            (runtime.XTaskQueueCreate)(
                XTASKQUEUE_DISPATCH_MODE_THREAD_POOL,
                XTASKQUEUE_DISPATCH_MODE_MANUAL,
                &mut queue,
            )
        };
        if hr < 0 {
            return Err(Error::new("[GDK] Could not create global task queue"));
        }
        *global = queue as usize;
    }

    // Duplicate the global task queue handle into outTaskQueue
    let runtime = runtime().map_err(|_| Error::new("[GDK] Unable to acquire global task queue"))?;
    let mut queue: XTaskQueueHandle = std::ptr::null_mut();
    // SAFETY: the global handle is open; queue is a valid out pointer.
    let hr =
        unsafe { (runtime.XTaskQueueDuplicateHandle)(*global as XTaskQueueHandle, &mut queue) };
    if hr < 0 {
        return Err(Error::new("[GDK] Unable to acquire global task queue"));
    }
    Ok(queue)
}

/// Terminate the global task queue, dispatch what completes, and close
/// SDL's handle of it (the references given out stay open until their
/// owners close them); the next [`get_task_queue`] makes a new queue.
/// Upstream's `SDL_RunApp()` does this with the first caller's handle,
/// which is the global one there (see [`get_task_queue`]).
pub(crate) fn quit_task_queue() {
    let queue = std::mem::take(&mut *global_task_queue()) as XTaskQueueHandle;
    if queue.is_null() {
        return;
    }
    let Ok(runtime) = runtime() else {
        return;
    };

    // (upstream's comment, in SDL_RunApp():)
    // !!! FIXME: This follows the docs exactly, but for some reason still leaks handles on exit?
    // Terminate the task queue and dispatch any pending tasks
    // SAFETY: the handle is SDL's, closed once, here.
    unsafe {
        (runtime.XTaskQueueTerminate)(queue, false, std::ptr::null_mut(), None);
        while (runtime.XTaskQueueDispatch)(queue, XTASKQUEUE_PORT_COMPLETION, 0) {}

        (runtime.XTaskQueueCloseHandle)(queue);
    }
}

/// Dispatch the completions that are ready on the global task queue (none
/// before it's made). The Windows video driver's event pump calls this at
/// its end. Translation of `GDK_DispatchTaskQueue()`.
pub(crate) fn dispatch_task_queue() {
    /* If there is no global task queue, don't do anything.
     * This gives the option to opt-out for those who want to handle everything themselves.
     */
    // (dispatched through a reference of its own, taken under the lock:
    // the completions may ask for the queue, which takes the lock, and the
    // queue's handle can't be closed under the dispatch)
    let queue = {
        let global = global_task_queue();
        if *global == 0 {
            return;
        }
        let Ok(runtime) = runtime() else {
            return;
        };
        let mut queue: XTaskQueueHandle = std::ptr::null_mut();
        // SAFETY: the global handle is open; queue is a valid out pointer.
        if unsafe { (runtime.XTaskQueueDuplicateHandle)(*global as XTaskQueueHandle, &mut queue) }
            < 0
        {
            return;
        }
        (runtime, queue)
    };
    let (runtime, queue) = queue;

    // Dispatch any callbacks which are ready.
    // SAFETY: queue is an open handle, closed once, after.
    unsafe {
        while (runtime.XTaskQueueDispatch)(queue, XTASKQUEUE_PORT_COMPLETION, 0) {}
        (runtime.XTaskQueueCloseHandle)(queue);
    }
}

/// `hPLM`, `hCPLM` and `plmSuspendComplete`.
struct ChangeNotifications {
    plm: PAPPSTATE_REGISTRATION,
    cplm: PAPPCONSTRAIN_REGISTRATION,
    /// The event the suspension waits on (a `HANDLE`, or 0).
    suspend_complete: usize,
}

static CHANGE_NOTIFICATIONS: Mutex<ChangeNotifications> = Mutex::new(ChangeNotifications {
    plm: 0,
    cplm: 0,
    suspend_complete: 0,
});

fn change_notifications() -> MutexGuard<'static, ChangeNotifications> {
    CHANGE_NOTIFICATIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// A reference of its own to `plmSuspendComplete` (null when there's none),
/// so that the suspension waits on it without holding the lock
/// [`suspend_complete`] takes, and without the event closed under the wait.
fn duplicate_suspend_complete_event() -> HANDLE {
    let notifications = change_notifications();
    let event = notifications.suspend_complete as HANDLE;
    if event.is_null() {
        return std::ptr::null_mut();
    }
    let mut duplicate: HANDLE = std::ptr::null_mut();
    // SAFETY: event is an open handle of this process; duplicate is a valid
    // out pointer.
    let ok = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            event,
            GetCurrentProcess(),
            &mut duplicate,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if ok == 0 {
        std::ptr::null_mut()
    } else {
        duplicate
    }
}

/// The suspend/resume handler (upstream's `rascn` lambda), called by
/// Windows on a thread of its own. It holds no lock of SDL's across the
/// wait. (A panic in it aborts the process: an `extern "system"` function
/// doesn't unwind.)
unsafe extern "system" fn app_state_change(quiesced: bool, _context: *const c_void) {
    crate::log::debug!(
        Category::Application,
        "[GDK] in RegisterAppStateChangeNotification handler"
    );
    if quiesced {
        let event = duplicate_suspend_complete_event();
        // SAFETY: event is an event handle of ours, or null (then these
        // fail, as upstream's calls on a null handle).
        unsafe { ResetEvent(event) };
        send_app_event(EventType::DID_ENTER_BACKGROUND);

        // To defer suspension, we must wait to exit this callback.
        // IMPORTANT: The app must call SDL_GDKSuspendComplete() to release this lock.
        // Note (upstream): the wait has no timeout, as upstream's: the
        // suspension waits for the application, however long it takes.
        // SAFETY: as above.
        unsafe { WaitForSingleObject(event, INFINITE) };
        if !event.is_null() {
            // SAFETY: the duplicate is ours, closed once.
            unsafe { CloseHandle(event) };
        }

        crate::log::debug!(
            Category::Application,
            "[GDK] in RegisterAppStateChangeNotification handler: plmSuspendComplete event signaled."
        );
    } else {
        send_app_event(EventType::WILL_ENTER_FOREGROUND);
    }
}

/// The constrain/unconstrain handler (upstream's `raccn` lambda), called by
/// Windows on a thread of its own: the keyboard focus leaves the windows
/// while the application is constrained (unless the newest window takes
/// text input), and comes back to the newest window. (A panic in it
/// aborts the process, as above.)
unsafe extern "system" fn app_constrained_change(constrained: bool, _context: *const c_void) {
    crate::log::debug!(
        Category::Application,
        "[GDK] in RegisterAppConstrainedChangeNotification handler"
    );
    // FIXME (upstream): this changes the keyboard focus (and sends its
    // window events) from Windows' thread, not the event thread, without
    // a lock of the video subsystem's; here each step takes the state's
    // locks, but the steps can still interleave with the event thread's.
    // (`_this->windows`: the most recently created window)
    let Ok(first) = crate::video::core::with_device(|v| v.windows.first().map(|w| w.core.id))
    else {
        return;
    };
    if constrained && !first.is_some_and(keyboard::text_input_active) {
        let _ = keyboard::set_keyboard_focus(None);
    } else {
        let _ = keyboard::set_keyboard_focus(first);
    }
}

/// Register the application-state notifications: suspend and resume
/// (`SDL_EVENT_DID_ENTER_BACKGROUND`, then a wait for
/// [`suspend_complete`]; `SDL_EVENT_WILL_ENTER_FOREGROUND`), and constrain
/// and unconstrain (the keyboard focus). Translation of
/// `GDK_RegisterChangeNotifications()`.
pub(crate) fn register_change_notifications() -> Result<()> {
    // Register suspend/resume handling
    // SAFETY: no attributes, no name.
    let event = unsafe {
        CreateEventExW(
            std::ptr::null(),
            std::ptr::null(),
            0,
            EVENT_MODIFY_STATE | SYNCHRONIZATION_SYNCHRONIZE,
        )
    };
    if event.is_null() {
        return Err(Error::new(
            "[GDK] Unable to create plmSuspendComplete event",
        ));
    }
    change_notifications().suspend_complete = event as usize;

    let mut plm: PAPPSTATE_REGISTRATION = 0;
    // SAFETY: the handler is a function of the right type; plm is a valid
    // out pointer.
    let error = unsafe {
        RegisterAppStateChangeNotification(Some(app_state_change), std::ptr::null(), &mut plm)
    };
    change_notifications().plm = plm;
    if error != 0 {
        // FIXME (upstream): the error (a Win32 error code) isn't in the
        // message.
        return Err(Error::new(
            "[GDK] Unable to call RegisterAppStateChangeNotification",
        ));
    }

    // Register constrain/unconstrain handling
    let mut cplm: PAPPCONSTRAIN_REGISTRATION = 0;
    // SAFETY: as above.
    let error = unsafe {
        RegisterAppConstrainedChangeNotification(
            Some(app_constrained_change),
            std::ptr::null(),
            &mut cplm,
        )
    };
    change_notifications().cplm = cplm;
    if error != 0 {
        // FIXME (upstream): as above.
        return Err(Error::new(
            "[GDK] Unable to call RegisterAppConstrainedChangeNotification",
        ));
    }

    Ok(())
}

/// Unregister the application-state notifications. Translation of
/// `GDK_UnregisterChangeNotifications()`.
pub(crate) fn unregister_change_notifications() {
    // (the registrations are unregistered without the lock, which the
    // handlers take)
    let (plm, cplm) = {
        let notifications = change_notifications();
        (notifications.plm, notifications.cplm)
    };

    // Unregister suspend/resume handling
    // FIXME (upstream): UnregisterAppStateChangeNotification() crashes
    // on Windows with a null registration (a failed registration's).
    // SAFETY: plm is the registration (or null, as upstream's).
    unsafe { UnregisterAppStateChangeNotification(plm) };
    {
        // Note (upstream): upstream leaves plmSuspendComplete set, so a
        // later SDL_GDKSuspendComplete() signals a closed handle (or
        // another object that has its value by then). Here it's cleared.
        let event = std::mem::take(&mut change_notifications().suspend_complete);
        // SAFETY: the event is ours (or null: then it fails).
        unsafe { CloseHandle(event as HANDLE) };
    }

    // Unregister constrain/unconstrain handling
    // SAFETY: cplm is the registration (or null, as upstream's).
    unsafe { UnregisterAppConstrainedChangeNotification(cplm) };
}

/// Let a suspension the suspend handler is waiting for go on. Translation
/// of `SDL_GDKSuspendComplete()`.
pub(crate) fn suspend_complete() {
    let notifications = change_notifications();
    if notifications.suspend_complete != 0 {
        // SAFETY: the event is ours.
        unsafe { SetEvent(notifications.suspend_complete as HANDLE) };
    }
}

/// The default user, signed in (showing the sign-in UI if needed). The
/// caller closes it with `XUserCloseHandle()`. Translation of
/// `SDL_GetGDKDefaultUser()`.
pub(crate) fn get_default_user() -> Result<XUserHandle> {
    let mut block = XAsyncBlock::default();

    // (without the runtime, as upstream's XUserAddAsync() fails)
    let runtime = runtime()
        .map_err(|_| error_from_hresult(Some("XUserAddAsync"), E_GAMERUNTIME_DLL_NOT_FOUND))?;
    // SAFETY: block stays here until the call is done (below).
    let result = unsafe {
        (runtime.XUserAddAsync)(XUSER_ADD_OPTIONS_ADD_DEFAULT_USER_ALLOWING_UI, &mut block)
    };
    if result < 0 {
        return Err(error_from_hresult(Some("XUserAddAsync"), result));
    }

    // Note (upstream): upstream calls XUserAddResult() until it stops
    // returning E_PENDING, a busy wait on a CPU for as long as the sign-in
    // takes. Here the call is waited for as XAsync.h documents
    // (XAsyncGetStatus() with wait, which returns once it's done: no
    // completion callback is set), and then its result is read.
    // SAFETY: block is the call's.
    unsafe { (runtime.XAsyncGetStatus)(&mut block, true) };
    let mut user: XUserHandle = std::ptr::null_mut();
    // SAFETY: block is the call's; user is a valid out pointer.
    let result = unsafe { (runtime.XUserAddResult)(&mut block, &mut user) };
    if result < 0 {
        return Err(error_from_hresult(Some("XUserAddResult"), result));
    }

    Ok(user)
}

#[cfg(test)]
mod tests;
