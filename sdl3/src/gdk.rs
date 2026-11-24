// The GDK functions of SDL_system.h (SDL_GetGDKTaskQueue(),
// SDL_GetGDKDefaultUser()) and SDL_main.h (SDL_GDKSuspendComplete()), from
// src/core/gdk/SDL_gdk.cpp of Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! What SDL offers a program built for the Microsoft GDK for PC (the `gdk`
//! feature, upstream's WinGDK): SDL's global task queue, the default user,
//! and the end of a suspension.
//!
//! They need the Gaming Runtime (`xgameruntime.dll`, which Windows' Gaming
//! Services installs), loaded at run time: without it [`task_queue`] and
//! [`default_user`] fail. The handles are owned: dropping a [`TaskQueue`]
//! closes its reference (`XTaskQueueCloseHandle()`), and dropping a
//! [`User`] closes its handle (`XUserCloseHandle()`), as upstream's
//! documentation tells the caller to do.

use std::fmt;
use std::ptr::NonNull;

use crate::core::gdk::{self as core_gdk, sys::runtime};
use crate::error::Result;

pub use crate::core::gdk::sys::{XTaskQueueHandle, XTaskQueueObject, XUser, XUserHandle};

/// A reference to SDL's global task queue (`XTaskQueueHandle`), closed when
/// dropped.
///
/// The queue's work runs on the system's thread pool; its completions are
/// dispatched by the event pump (the Windows video driver's), on the event
/// thread.
pub struct TaskQueue(NonNull<XTaskQueueObject>);

// SAFETY: a task queue handle may be used, and closed, on any thread.
unsafe impl Send for TaskQueue {}
// SAFETY: as above; nothing changes through a shared reference.
unsafe impl Sync for TaskQueue {}

impl TaskQueue {
    /// The handle, for the GDK's functions (valid while this value lives;
    /// `XTaskQueueDuplicateHandle()` makes a reference of one's own).
    pub fn as_raw(&self) -> XTaskQueueHandle {
        self.0.as_ptr()
    }
}

impl fmt::Debug for TaskQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("TaskQueue").field(&self.0).finish()
    }
}

impl Drop for TaskQueue {
    /// `XTaskQueueCloseHandle()`.
    fn drop(&mut self) {
        // (a handle can only come from a loaded runtime)
        if let Ok(runtime) = runtime() {
            // SAFETY: the reference is ours, closed once.
            unsafe { (runtime.XTaskQueueCloseHandle)(self.0.as_ptr()) };
        }
    }
}

/// A reference to SDL's global async task queue, which is made the first
/// time. Translation of `SDL_GetGDKTaskQueue()`.
///
/// FIXME (upstream): the first call's value is the global queue's own
/// reference, which dropping it closes, leaving the queue SDL dispatches
/// dangling.
pub fn task_queue() -> Result<TaskQueue> {
    let queue = core_gdk::get_task_queue()?;
    // (a successful call gives a handle)
    Ok(TaskQueue(
        NonNull::new(queue).expect("XTaskQueueCreate gave a handle"),
    ))
}

/// A user's handle (`XUserHandle`), closed when dropped.
pub struct User(NonNull<XUser>);

// SAFETY: a user handle may be used, and closed, on any thread.
unsafe impl Send for User {}
// SAFETY: as above; nothing changes through a shared reference.
unsafe impl Sync for User {}

impl User {
    /// The handle, for the GDK's functions (valid while this value lives;
    /// `XUserDuplicateHandle()` makes one of one's own).
    pub fn as_raw(&self) -> XUserHandle {
        self.0.as_ptr()
    }
}

impl fmt::Debug for User {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("User").field(&self.0).finish()
    }
}

impl Drop for User {
    /// `XUserCloseHandle()`.
    fn drop(&mut self) {
        // (a handle can only come from a loaded runtime)
        if let Ok(runtime) = runtime() {
            // SAFETY: the handle is ours, closed once.
            unsafe { (runtime.XUserCloseHandle)(self.0.as_ptr()) };
        }
    }
}

/// The default user: a synchronous `XUserAddAsync()`, which always prefers
/// the default user and allows a sign-in UI. It blocks until the user is
/// signed in (or that fails). Translation of `SDL_GetGDKDefaultUser()`.
pub fn default_user() -> Result<User> {
    let user = core_gdk::get_default_user()?;
    // (a successful call gives a handle)
    Ok(User(
        NonNull::new(user).expect("XUserAddResult gave a handle"),
    ))
}

/// Let the suspension go on, once the application has handled
/// `SDL_EVENT_DID_ENTER_BACKGROUND` (in an event watch, where the event is
/// sent: it isn't queued). Until then the suspend handler waits, without a
/// timeout. (Upstream's renderer and GPU suspensions, which do nothing on
/// WinGDK, come before it.) Nothing happens when no suspend handler is
/// registered. Translation of `SDL_GDKSuspendComplete()`.
pub fn suspend_complete() {
    core_gdk::suspend_complete();
}
