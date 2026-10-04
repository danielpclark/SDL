// Rust translation of src/joystick/hidapi/SDL_hidapi_rumble.c and
// SDL_hidapi_rumble.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Handle rumble on a separate thread so it doesn't block the application.
//!
//! The simple API, [`send_rumble`], replaces any pending rumble with the
//! new data. The advanced API locks the queue ([`lock_rumble`]), so a
//! driver can number its packets in queue order or update a pending
//! request, and sends while unlocking ([`RumbleLock::send_and_unlock`]).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::{HidapiDevice, USB_PACKET_LENGTH};
use crate::error::{Error, Result};
use crate::thread::{Semaphore, Thread, ThreadPriority};

/// The callback of a request, run on the rumble thread once it is sent
/// (`SDL_HIDAPI_RumbleSentCallback` with its `userdata`).
pub(crate) type RumbleSentCallback = Box<dyn FnOnce() + Send>;

/// Translation of `SDL_HIDAPI_RumbleRequest`.
struct RumbleRequest {
    device: Arc<HidapiDevice>,
    /// need enough space for the biggest report: dualshock4 is 78 bytes
    data: [u8; 2 * USB_PACKET_LENGTH],
    size: usize,
    callback: Option<RumbleSentCallback>,
}

/// The queue of requests, oldest first (upstream's `requests_tail` to
/// `requests_head`); its mutex is `SDL_HIDAPI_rumble_lock`.
struct RumbleQueue {
    requests: VecDeque<RumbleRequest>,
}

static RUMBLE_LOCK: Mutex<RumbleQueue> = Mutex::new(RumbleQueue {
    requests: VecDeque::new(),
});

/// Translation of `SDL_HIDAPI_RumbleContext` (without the queue).
struct RumbleContext {
    initialized: AtomicBool,
    running: AtomicBool,
    thread: Mutex<Option<Thread>>,
    request_sem: Semaphore,
}

static RUMBLE_CONTEXT: RumbleContext = RumbleContext {
    initialized: AtomicBool::new(false),
    running: AtomicBool::new(false),
    thread: Mutex::new(None),
    request_sem: Semaphore::new(0),
};

fn lock_queue() -> MutexGuard<'static, RumbleQueue> {
    RUMBLE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `SDL_HIDAPI_RumbleThread()`.
fn rumble_thread() -> i32 {
    let ctx = &RUMBLE_CONTEXT;

    let _ = crate::thread::set_current_thread_priority(ThreadPriority::High);

    while ctx.running.load(Ordering::Acquire) {
        ctx.request_sem.wait();

        let request = lock_queue().requests.pop_front();

        if let Some(request) = request {
            if let Some(dev) = request.device.dev() {
                let _ = dev.write(&request.data[..request.size]);
            }
            if let Some(callback) = request.callback {
                callback();
            }
            request.device.rumble_pending.fetch_sub(1, Ordering::AcqRel);

            // Make sure we're not starving report reads when there's lots of rumble
            crate::timer::delay(Duration::from_millis(10));
        }
    }
    0
}

/// Translation of `SDL_HIDAPI_StopRumbleThread()`.
fn stop_rumble_thread() {
    let ctx = &RUMBLE_CONTEXT;

    ctx.running.store(false, Ordering::Release);

    let thread = ctx.thread.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(thread) = thread {
        ctx.request_sem.signal();
        thread.wait();
    }

    let requests: Vec<RumbleRequest> = lock_queue().requests.drain(..).collect();
    for request in requests {
        if let Some(callback) = request.callback {
            callback();
        }
        request.device.rumble_pending.fetch_sub(1, Ordering::AcqRel);
    }

    // (the semaphore is static; reset its count for the next start)
    while ctx.request_sem.try_wait() {}

    ctx.initialized.store(false, Ordering::Release);
}

/// Translation of `SDL_HIDAPI_StartRumbleThread()`.
fn start_rumble_thread() -> Result<()> {
    let ctx = &RUMBLE_CONTEXT;

    ctx.running.store(true, Ordering::Release);
    match Thread::spawn("HIDAPI Rumble", rumble_thread) {
        Ok(thread) => {
            *ctx.thread.lock().unwrap_or_else(|e| e.into_inner()) = Some(thread);
            Ok(())
        }
        Err(e) => {
            stop_rumble_thread();
            Err(e)
        }
    }
}

/// The locked rumble queue, unlocked on drop (`SDL_HIDAPI_UnlockRumble()`).
pub(crate) struct RumbleLock {
    queue: MutexGuard<'static, RumbleQueue>,
}

impl std::fmt::Debug for RumbleLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RumbleLock")
    }
}

/// Lock the rumble queue, starting the rumble thread on first use.
/// Translation of `SDL_HIDAPI_LockRumble()`.
pub(crate) fn lock_rumble() -> Result<RumbleLock> {
    let ctx = &RUMBLE_CONTEXT;

    if ctx
        .initialized
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        start_rumble_thread()?;
    }

    Ok(RumbleLock {
        queue: lock_queue(),
    })
}

impl RumbleLock {
    /// The data of the newest request still queued for a device, which
    /// may be changed in place: its buffer (the maximum size) and its
    /// length. Translation of `SDL_HIDAPI_GetPendingRumbleLocked()`.
    pub(crate) fn pending_mut(
        &mut self,
        device: &Arc<HidapiDevice>,
    ) -> Option<(&mut [u8], &mut usize)> {
        self.queue
            .requests
            .iter_mut()
            .rev()
            .find(|request| Arc::ptr_eq(&request.device, device))
            .map(|request| (&mut request.data[..], &mut request.size))
    }

    /// Queue a report for a device and unlock. Returns the size queued.
    /// Translation of `SDL_HIDAPI_SendRumbleAndUnlock()`.
    pub(crate) fn send_and_unlock(self, device: &Arc<HidapiDevice>, data: &[u8]) -> Result<usize> {
        self.send_with_callback_and_unlock(device, data, None)
    }

    /// Queue a report for a device, with a callback the rumble thread runs
    /// once it is sent, and unlock. Translation of
    /// `SDL_HIDAPI_SendRumbleWithCallbackAndUnlock()`.
    pub(crate) fn send_with_callback_and_unlock(
        mut self,
        device: &Arc<HidapiDevice>,
        data: &[u8],
        callback: Option<RumbleSentCallback>,
    ) -> Result<usize> {
        let mut request = RumbleRequest {
            device: device.clone(),
            data: [0; 2 * USB_PACKET_LENGTH],
            size: data.len(),
            callback,
        };
        if data.len() > request.data.len() {
            drop(self);
            return Err(Error::new(format!(
                "Couldn't send rumble, size {} is greater than {}",
                data.len(),
                request.data.len()
            )));
        }
        request.data[..data.len()].copy_from_slice(data);

        device.rumble_pending.fetch_add(1, Ordering::AcqRel);

        self.queue.requests.push_back(request);

        // Make sure we unlock before posting the semaphore so the rumble thread can run immediately
        drop(self);

        RUMBLE_CONTEXT.request_sem.signal();

        Ok(data.len())
    }
}

/// Queue a report for a device, replacing the pending report of the same
/// kind (size and report ID) if there is one. Returns the size queued.
/// Translation of `SDL_HIDAPI_SendRumble()`.
pub(crate) fn send_rumble(device: &Arc<HidapiDevice>, data: &[u8]) -> Result<usize> {
    if data.is_empty() {
        return Err(Error::new("Tried to send rumble with invalid size"));
    }

    let mut lock = lock_rumble()?;

    // check if there is a pending request for the device and update it
    if let Some((pending_data, pending_size)) = lock.pending_mut(device) {
        if data.len() == *pending_size && data[0] == pending_data[0] {
            pending_data[..data.len()].copy_from_slice(data);
            return Ok(data.len());
        }
    }

    lock.send_and_unlock(device, data)
}

/// Stop the rumble thread. Translation of `SDL_HIDAPI_QuitRumble()`.
pub(crate) fn quit_rumble() {
    if RUMBLE_CONTEXT.running.load(Ordering::Acquire) {
        stop_rumble_thread();
    }
}
