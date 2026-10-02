// Rust translation of the thread functions of src/thread/SDL_thread.c from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use super::{adopt_thread_id, allocate_thread_id, cleanup_tls, ThreadID};
use crate::error::{Error, Result};

/// The SDL thread priority. Translation of `SDL_ThreadPriority`.
///
/// SDL will make system changes as necessary in order to apply the thread
/// priority. Code which attempts to control thread state related to
/// priority should be aware that calling [`set_current_thread_priority`]
/// may alter such state.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ThreadPriority {
    Low,
    #[default]
    Normal,
    High,
    TimeCritical,
}

/// The SDL thread state. Translation of `SDL_ThreadState`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ThreadState {
    /// The thread is not valid
    Unknown,
    /// The thread is currently running
    Alive,
    /// The thread is detached and can't be waited on
    Detached,
    /// The thread has finished and should be cleaned up with [`Thread::wait`]
    Complete,
}

const SDL_THREAD_ALIVE: i32 = 1;
const SDL_THREAD_DETACHED: i32 = 2;
const SDL_THREAD_COMPLETE: i32 = 3;

/// The state shared between a [`Thread`] handle and the running thread
/// (the fields of `struct SDL_Thread` that both sides touch).
#[derive(Debug)]
struct Shared {
    threadid: ThreadID,
    name: Option<String>,
    status: AtomicI32,
    state: AtomicI32,
}

/// A running thread. Translation of `SDL_Thread`.
///
/// Created with [`Thread::spawn`] or [`Thread::builder`]. Call
/// [`wait`](Thread::wait) to collect the thread's return value, or
/// [`detach`](Thread::detach) to let it run on its own; dropping the handle
/// detaches (where C SDL would leak the thread).
#[derive(Debug)]
#[must_use = "dropping a Thread detaches it; call .wait() to join"]
pub struct Thread {
    shared: Arc<Shared>,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// Options for creating a thread (the `SDL_PROP_THREAD_CREATE_*` properties
/// of `SDL_CreateThreadWithProperties()`).
#[derive(Clone, Debug, Default)]
pub struct ThreadBuilder {
    name: Option<String>,
    stacksize: usize,
}

impl ThreadBuilder {
    /// The name of the new thread (`SDL_PROP_THREAD_CREATE_NAME_STRING`),
    /// reported to the operating system where possible.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// The stack size in bytes (`SDL_PROP_THREAD_CREATE_STACKSIZE_NUMBER`);
    /// zero means the system default.
    pub fn stack_size(mut self, bytes: usize) -> Self {
        self.stacksize = bytes;
        self
    }

    /// Create and start the thread. `f`'s return value is the thread's
    /// status, collected by [`Thread::wait`].
    /// Translation of `SDL_CreateThreadWithProperties()`.
    pub fn spawn(self, f: impl FnOnce() -> i32 + Send + 'static) -> Result<Thread> {
        crate::init::init_main_thread();

        let shared = Arc::new(Shared {
            threadid: allocate_thread_id(),
            name: self.name,
            status: AtomicI32::new(-1),
            state: AtomicI32::new(SDL_THREAD_ALIVE),
        });

        // Create the thread and go!
        let mut builder = std::thread::Builder::new();
        if let Some(name) = &shared.name {
            // SDL_SYS_SetupThread(): the OS name (Linux truncates to 15 bytes)
            builder = builder.name(name.clone());
        }
        if self.stacksize != 0 {
            builder = builder.stack_size(self.stacksize);
        }
        let run_shared = shared.clone();
        let handle = builder
            .spawn(move || run_thread(&run_shared, f))
            .map_err(|e| Error::new(format!("Couldn't create thread: {e}")))?;

        // Everything is running now
        Ok(Thread {
            shared,
            handle: Some(handle),
        })
    }
}

/// Translation of `SDL_RunThread()`.
fn run_thread(thread: &Shared, userfunc: impl FnOnce() -> i32) {
    adopt_thread_id(thread.threadid);

    // Run the function (a panic leaves the status at -1, as for a failed thread)
    let status = std::panic::catch_unwind(std::panic::AssertUnwindSafe(userfunc));
    if let Ok(status) = status {
        thread.status.store(status, Ordering::Release);
    }

    // Clean up thread-local storage
    cleanup_tls();

    // Mark us as ready to be joined (or detached)
    let _ = thread.state.compare_exchange(
        SDL_THREAD_ALIVE,
        SDL_THREAD_COMPLETE,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
    // (If something already detached us, the Arc frees the state.)

    if let Err(panic) = status {
        std::panic::resume_unwind(panic);
    }
}

impl Thread {
    /// Options for a new thread.
    pub fn builder() -> ThreadBuilder {
        ThreadBuilder::default()
    }

    /// Create a named thread with the default stack size.
    /// Translation of `SDL_CreateThread()`.
    pub fn spawn(
        name: impl Into<String>,
        f: impl FnOnce() -> i32 + Send + 'static,
    ) -> Result<Thread> {
        ThreadBuilder::default().name(name).spawn(f)
    }

    /// The thread's id. Translation of `SDL_GetThreadID(thread)`.
    pub fn id(&self) -> ThreadID {
        self.shared.threadid
    }

    /// The thread's name. Translation of `SDL_GetThreadName()`.
    pub fn name(&self) -> Option<&str> {
        self.shared.name.as_deref()
    }

    /// The current state of the thread. Translation of `SDL_GetThreadState()`.
    pub fn state(&self) -> ThreadState {
        match self.shared.state.load(Ordering::Acquire) {
            SDL_THREAD_ALIVE => ThreadState::Alive,
            SDL_THREAD_DETACHED => ThreadState::Detached,
            SDL_THREAD_COMPLETE => ThreadState::Complete,
            _ => ThreadState::Unknown,
        }
    }

    /// Wait for the thread to finish and return its status (-1 if it
    /// panicked). Translation of `SDL_WaitThread()`.
    pub fn wait(mut self) -> i32 {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        self.shared.status.load(Ordering::Acquire)
    }

    /// Let the thread run on its own; its resources are freed when it ends.
    /// Translation of `SDL_DetachThread()`.
    pub fn detach(mut self) {
        self.detach_inner();
    }

    fn detach_inner(&mut self) {
        // Grab dibs if the state is alive+joinable.
        if self
            .shared
            .state
            .compare_exchange(
                SDL_THREAD_ALIVE,
                SDL_THREAD_DETACHED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            // The thread may vanish at any time, it's no longer valid
            drop(self.handle.take()); // SDL_SYS_DetachThread()
        } else if self.state() == ThreadState::Complete {
            // already done, clean it up.
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for Thread {
    fn drop(&mut self) {
        if self.handle.is_some() {
            self.detach_inner();
        }
    }
}

/// Set the priority for the current thread. Translation of `SDL_SetCurrentThreadPriority()`.
///
/// Priorities need the platform layer (`setpriority`/RealtimeKit on Linux,
/// `SetThreadPriority` on Windows); until it exists this reports
/// [`Unsupported`](crate::ErrorKind::Unsupported) rather than pretending.
pub fn set_current_thread_priority(_priority: ThreadPriority) -> Result<()> {
    Err(Error::unsupported())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread::{current_thread_id, Semaphore};
    use std::sync::atomic::AtomicU64;

    #[test]
    fn spawn_wait_status_and_ids() {
        let seen = Arc::new(AtomicU64::new(0));
        let s2 = seen.clone();
        let t = Thread::spawn("worker", move || {
            s2.store(current_thread_id(), Ordering::SeqCst);
            assert_eq!(std::thread::current().name(), Some("worker"));
            42
        })
        .unwrap();
        let id = t.id();
        assert_eq!(t.name(), Some("worker"));
        assert_ne!(id, current_thread_id());
        assert_eq!(t.wait(), 42);
        assert_eq!(
            seen.load(Ordering::SeqCst),
            id,
            "the handle knows the id the thread sees"
        );
    }

    #[test]
    fn states_detach_and_panics() {
        let gate = Arc::new(Semaphore::new(0));
        let g2 = gate.clone();
        let t = Thread::builder()
            .stack_size(256 * 1024)
            .spawn(move || {
                g2.wait();
                7
            })
            .unwrap();
        assert_eq!(t.state(), ThreadState::Alive);
        assert_eq!(t.name(), None);
        gate.signal();
        while t.state() != ThreadState::Complete {
            std::thread::yield_now();
        }
        assert_eq!(t.wait(), 7);

        let done = Arc::new(Semaphore::new(0));
        let d2 = done.clone();
        let t = Thread::spawn("detached", move || {
            d2.signal();
            0
        })
        .unwrap();
        t.detach();
        assert!(done.wait_timeout(Some(std::time::Duration::from_secs(5))));

        let t = Thread::spawn("panics", || panic!("expected test panic")).unwrap();
        assert_eq!(t.wait(), -1);

        assert_eq!(
            set_current_thread_priority(ThreadPriority::High)
                .unwrap_err()
                .kind(),
            crate::ErrorKind::Unsupported
        );
    }
}
