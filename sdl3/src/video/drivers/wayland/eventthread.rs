// Rust translation of src/video/wayland/SDL_waylandeventthread.c and
// SDL_waylandeventthread.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A thread dispatching an event queue of its own, so that objects on it
//! (the frame callbacks of animated cursors) keep working while the
//! application doesn't pump events.

use std::ffi::CStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;

use super::client::{Conn, EventQueue, Proxy};
use super::protocols::wayland::*;
use crate::core::unix::{io_ready, IoReadyFlags};

/// The state shared with the thread.
struct Inner {
    conn: Arc<Conn>,
    queue: EventQueue,
    dispatch_lock: Mutex<()>,
    should_exit: AtomicBool,
}

/// Translation of `struct Wayland_EventThreadContext`.
pub(crate) struct EventThreadContext {
    inner: Arc<Inner>,
    thread: Mutex<Option<JoinHandle<i32>>>,
}

impl std::fmt::Debug for EventThreadContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EventThreadContext")
    }
}

/// Translation of `Wayland_EventThreadFunc()`.
fn wayland_event_thread_func(context: &Inner) -> i32 {
    let display = &context.conn;
    let display_fd = display.fd();

    /* The lock must be held whenever dispatching to avoid a race condition when adding
     * or destroying objects using the queue.
     *
     * Any error other than EAGAIN is fatal and causes the thread to exit.
     */
    while !context.should_exit.load(Ordering::Acquire) {
        if display.prepare_read_queue(&context.queue) {
            let mut timeout_ns: i64 = -1;

            if let Err(errno) = display.flush() {
                if errno == libc::EAGAIN {
                    // If the flush failed with EAGAIN, don't block as not to inhibit other threads from reading events.
                    timeout_ns = 1_000_000;
                } else {
                    display.cancel_read();
                    return -1;
                }
            }

            // Wait for a read/write operation to become possible.
            let ret = io_ready(display_fd, IoReadyFlags::READ, timeout_ns);

            if ret <= 0 {
                display.cancel_read();
                if ret < 0 {
                    return -1;
                }

                // Nothing to read, and woke to flush; try again.
                continue;
            }

            if display.read_events().is_err() {
                return -1;
            }
        }

        let ret = {
            let _guard = context
                .dispatch_lock
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            display.dispatch_queue_pending(&context.queue)
        };

        if ret.is_err() {
            return -1;
        }
    }

    0
}

impl EventThreadContext {
    /// Translation of `Wayland_CreateEventThread()`.
    pub(crate) fn create(conn: &Arc<Conn>, queue_name: &CStr) -> Option<Arc<EventThreadContext>> {
        let queue = conn.create_queue(queue_name)?;

        let inner = Arc::new(Inner {
            conn: conn.clone(),
            queue,
            dispatch_lock: Mutex::new(()),
            should_exit: AtomicBool::new(false),
        });

        let thread_inner = inner.clone();
        let thread = std::thread::Builder::new()
            .name("wl_event_thread".into())
            .spawn(move || wayland_event_thread_func(&thread_inner))
            .ok()?;

        Some(Arc::new(EventThreadContext {
            inner,
            thread: Mutex::new(Some(thread)),
        }))
    }

    /// Stop the thread and destroy the queue. Translation of
    /// `Wayland_DestroyEventThread()`.
    pub(crate) fn destroy(&self) {
        let Some(thread) = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take() else {
            return;
        };

        // Dispatch the exit event to unblock the thread and signal it to exit.
        let display = &self.inner.conn;
        let display_wrapper = self.create_proxy_wrapper(display.obj());

        let cb = {
            let _guard = self.lock();
            let mut cb: Proxy<WlCallback> = display_wrapper.sync();
            let inner = Arc::downgrade(&self.inner);
            cb.listen(move |_, _| {
                // Translation of `handle_event_thread_exit()` (the callback
                // is destroyed below, once the thread is gone).
                if let Some(inner) = inner.upgrade() {
                    inner.should_exit.store(true, Ordering::Release);
                }
            });
            cb
        };

        drop(display_wrapper);

        let mut ret = display.flush();
        while ret == Err(libc::EAGAIN) {
            // Shutting down the thread requires a successful flush.
            let r = io_ready(display.fd(), IoReadyFlags::WRITE, -1);
            if r >= 0 {
                ret = display.flush();
            }
        }

        // (upstream destroys the callback here if the flush failed due to a
        // broken connection, and in its exit handler otherwise; it is
        // destroyed once the thread is gone either way)

        // Wait for the thread to return; it will exit automatically on a broken connection.
        let _ = thread.join();
        drop(cb);

        // (the queue and the lock go with the last reference to the context)
    }

    /// A proxy wrapper of `obj` whose new objects go to the thread's queue.
    /// Translation of `Wayland_CreateEventThreadProxyWrapper()`.
    pub(crate) fn create_proxy_wrapper<I: super::client::Interface>(
        &self,
        obj: super::client::Obj<'_, I>,
    ) -> Proxy<I> {
        obj.create_wrapper(&self.inner.queue)
    }

    /// Hold off the thread's dispatching. Translation of
    /// `Wayland_LockEventThread()` and `Wayland_UnlockEventThread()` (on drop
    /// of the guard).
    pub(crate) fn lock(&self) -> MutexGuard<'_, ()> {
        self.inner
            .dispatch_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }
}

/// `Wayland_LockEventThread()` on an optional context: a guard holding the
/// thread's dispatch lock, if there is a thread.
pub(crate) fn lock_event_thread(
    context: &Option<Arc<EventThreadContext>>,
) -> Option<MutexGuard<'_, ()>> {
    context.as_ref().map(|c| c.lock())
}
