// Rust translation of src/thread/SDL_thread.c, src/thread/generic/SDL_sysmutex.c
// and src/thread/generic/SDL_syssem.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Threads, thread-local storage and synchronization primitives.
//!
//! [`Thread`] translates `SDL_Thread` (on top of `std::thread`, which is
//! what the pthread/Win32 backends wrap); [`TlsId`] translates `SDL_TLSID`;
//! [`Semaphore`], [`ReentrantMutex`] (SDL's mutexes are recursive) and
//! [`InitState`] translate the generic `SDL_Semaphore`, `SDL_Mutex` and
//! `SDL_InitState`, and [`Condition`] the generic `SDL_Condition` (for use
//! with the recursive mutex). For plain mutexes, reader/writer locks and
//! condition variables use `std::sync::{Mutex, RwLock, Condvar}`; they are
//! what `SDL_Mutex`/`SDL_RWLock`/`SDL_Condition` wrap on every real platform.

use std::cell::{Cell, UnsafeCell};
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::Condvar;
use std::time::Duration;

mod cond;
mod threads;
mod tls;

pub use cond::Condition;

pub use threads::{
    set_current_thread_priority, Thread, ThreadBuilder, ThreadPriority, ThreadState,
};
#[cfg(target_os = "linux")]
pub use threads::{set_linux_thread_priority, set_linux_thread_priority_and_policy};
pub use tls::{cleanup_tls, TlsId};

/// A unique numeric ID that identifies a thread (never 0).
/// Translation of `SDL_ThreadID`.
pub type ThreadID = u64;

static NEXT_THREAD_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static THIS_THREAD_ID: Cell<ThreadID> = const { Cell::new(0) };
}

/// Reserve an id for a thread about to be created (so its handle knows the
/// id before the thread runs, as with `pthread_create`'s out-parameter).
fn allocate_thread_id() -> ThreadID {
    NEXT_THREAD_ID.fetch_add(1, Ordering::Relaxed)
}

/// Adopt a pre-allocated id on a freshly started thread.
fn adopt_thread_id(id: ThreadID) {
    THIS_THREAD_ID.with(|c| c.set(id));
}

/// The thread identifier for the current thread. Translation of `SDL_GetCurrentThreadID()`.
pub fn current_thread_id() -> ThreadID {
    THIS_THREAD_ID.with(|c| {
        if c.get() == 0 {
            c.set(allocate_thread_id());
        }
        c.get()
    })
}

/// A counting semaphore. Translation of the generic `SDL_Semaphore`
/// (`SDL_CreateSemaphore()` is [`Semaphore::new`]; dropping destroys it).
pub struct Semaphore {
    count: std::sync::Mutex<u32>,
    cond: Condvar,
}

impl std::fmt::Debug for Semaphore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Semaphore")
            .field("value", &self.value())
            .finish()
    }
}

impl Semaphore {
    /// Create a semaphore with an initial value. Translation of `SDL_CreateSemaphore()`.
    pub const fn new(initial_value: u32) -> Self {
        Semaphore {
            count: std::sync::Mutex::new(initial_value),
            cond: Condvar::new(),
        }
    }

    /// Wait until the value is non-zero, then decrement it. Translation of `SDL_WaitSemaphore()`.
    pub fn wait(&self) {
        let mut count = self.count.lock().unwrap_or_else(|e| e.into_inner());
        while *count == 0 {
            count = self.cond.wait(count).unwrap_or_else(|e| e.into_inner());
        }
        *count -= 1;
    }

    /// Decrement the value if it is non-zero. Translation of `SDL_TryWaitSemaphore()`.
    pub fn try_wait(&self) -> bool {
        let mut count = self.count.lock().unwrap_or_else(|e| e.into_inner());
        if *count > 0 {
            *count -= 1;
            true
        } else {
            false
        }
    }

    /// Wait up to `timeout` (`None`: forever) for a non-zero value, then
    /// decrement it; `false` on timeout. Translation of `SDL_WaitSemaphoreTimeoutNS()`.
    pub fn wait_timeout(&self, timeout: Option<Duration>) -> bool {
        let Some(timeout) = timeout else {
            self.wait();
            return true;
        };
        if timeout.is_zero() {
            return self.try_wait();
        }
        let deadline = std::time::Instant::now() + timeout;
        let mut count = self.count.lock().unwrap_or_else(|e| e.into_inner());
        while *count == 0 {
            let now = std::time::Instant::now();
            if now >= deadline {
                return false;
            }
            let (guard, _) = self
                .cond
                .wait_timeout(count, deadline - now)
                .unwrap_or_else(|e| e.into_inner());
            count = guard;
        }
        *count -= 1;
        true
    }

    /// Increment the value, waking one waiter. Translation of `SDL_SignalSemaphore()`.
    pub fn signal(&self) {
        let mut count = self.count.lock().unwrap_or_else(|e| e.into_inner());
        *count += 1;
        self.cond.notify_one();
    }

    /// The current value. Translation of `SDL_GetSemaphoreValue()`.
    pub fn value(&self) -> u32 {
        *self.count.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// SDL's recursive mutex (lock-only, no guarded data).
///
/// Direct translation of `struct SDL_Mutex` from `generic/SDL_sysmutex.c`.
pub(crate) struct RawMutex {
    recursive: UnsafeCell<i32>,
    owner: AtomicU64,
    sem: Semaphore,
}

// SAFETY: `recursive` is only ever touched by the thread that currently owns
// the semaphore (see lock/unlock), so it is never accessed concurrently.
unsafe impl Send for RawMutex {}
unsafe impl Sync for RawMutex {}

impl RawMutex {
    pub(crate) const fn new() -> Self {
        // Create the mutex semaphore, with initial value 1
        RawMutex {
            recursive: UnsafeCell::new(0),
            owner: AtomicU64::new(0),
            sem: Semaphore::new(1),
        }
    }

    /// Translation of `SDL_LockMutex()`.
    pub(crate) fn lock(&self) {
        let this_thread = current_thread_id();
        if self.owner.load(Ordering::Acquire) == this_thread {
            // SAFETY: we own the lock, nobody else touches `recursive`.
            unsafe { *self.recursive.get() += 1 };
        } else {
            /* The order of operations is important.
              We set the locking thread id after we obtain the lock
              so unlocks from other threads will fail.
            */
            self.sem.wait();
            self.owner.store(this_thread, Ordering::Release);
            // SAFETY: we now own the lock.
            unsafe { *self.recursive.get() = 0 };
        }
    }

    /// Translation of `SDL_UnlockMutex()`.
    pub(crate) fn unlock(&self) {
        // If we don't own the mutex, we can't unlock it
        if current_thread_id() != self.owner.load(Ordering::Acquire) {
            debug_assert!(false, "Tried to unlock a mutex we don't own!");
            return;
        }
        // SAFETY: we own the lock.
        let recursive = unsafe { &mut *self.recursive.get() };
        if *recursive != 0 {
            *recursive -= 1;
        } else {
            /* The order of operations is important.
              First reset the owner so another thread doesn't lock
              the mutex and set the ownership before we reset it,
              then release the lock semaphore.
            */
            self.owner.store(0, Ordering::Release);
            self.sem.signal();
        }
    }
}

/// RAII lock on a [`RawMutex`]; unlocks on drop.
pub(crate) struct RawMutexGuard<'a>(&'a RawMutex);

impl Drop for RawMutexGuard<'_> {
    fn drop(&mut self) {
        self.0.unlock();
    }
}

impl RawMutex {
    /// Lock and return a guard that unlocks when dropped.
    pub(crate) fn guard(&self) -> RawMutexGuard<'_> {
        self.lock();
        RawMutexGuard(self)
    }
}

/// A recursive mutex guarding a value: the thread holding it may lock it
/// again. Translation of `SDL_Mutex` (whose generic implementation, kept
/// here, is a semaphore plus an owner and a recursion count).
///
/// The guard hands out `&T` only (re-entrant `&mut` would be unsound); use
/// a `Cell`/`RefCell` inside for mutation, and never hold a `RefCell`
/// borrow across a call that might lock again.
pub struct ReentrantMutex<T> {
    raw: RawMutex,
    data: T,
}

// SAFETY: only one thread holds the lock at a time and the guard hands out
// `&T` only to that thread, so `T: Send` is sufficient (same reasoning as
// `std::sync::ReentrantLock`).
unsafe impl<T: Send> Send for ReentrantMutex<T> {}
unsafe impl<T: Send> Sync for ReentrantMutex<T> {}

impl<T: std::fmt::Debug> std::fmt::Debug for ReentrantMutex<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReentrantMutex")
            .field("data", &*self.lock())
            .finish()
    }
}

impl<T> ReentrantMutex<T> {
    /// Translation of `SDL_CreateMutex()`.
    pub const fn new(data: T) -> Self {
        ReentrantMutex {
            raw: RawMutex::new(),
            data,
        }
    }

    /// Lock (recursively). Translation of `SDL_LockMutex()`; the guard's drop is `SDL_UnlockMutex()`.
    pub fn lock(&self) -> ReentrantMutexGuard<'_, T> {
        self.raw.lock();
        ReentrantMutexGuard { m: self }
    }
}

/// The lock on a [`ReentrantMutex`]; unlocks on drop.
pub struct ReentrantMutexGuard<'a, T> {
    m: &'a ReentrantMutex<T>,
}

impl<T: std::fmt::Debug> std::fmt::Debug for ReentrantMutexGuard<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.m.data, f)
    }
}

impl<T> std::ops::Deref for ReentrantMutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.m.data
    }
}

impl<T> ReentrantMutexGuard<'_, T> {
    /// The underlying lock, for [`Condition`] waits.
    pub(crate) fn raw(&self) -> &RawMutex {
        &self.m.raw
    }
}

impl<T> Drop for ReentrantMutexGuard<'_, T> {
    fn drop(&mut self) {
        self.m.raw.unlock();
    }
}

const UNINITIALIZED: i32 = 0;
const INITIALIZING: i32 = 1;
const INITIALIZED: i32 = 2;
const UNINITIALIZING: i32 = 3;

/// Thread-safe one-time initialization/shutdown state. Translation of `SDL_InitState`.
///
/// ```
/// use sdl3::thread::InitState;
/// static STATE: InitState = InitState::new();
/// if STATE.should_init() {
///     // ... initialize ...
///     STATE.set_initialized(true);
/// }
/// ```
pub struct InitState {
    status: AtomicI32,
    thread: AtomicU64,
}

impl std::fmt::Debug for InitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InitState")
            .field("status", &self.status.load(Ordering::Relaxed))
            .finish()
    }
}

impl Default for InitState {
    fn default() -> Self {
        InitState::new()
    }
}

impl InitState {
    /// An uninitialized state.
    pub const fn new() -> Self {
        InitState {
            status: AtomicI32::new(UNINITIALIZED),
            thread: AtomicU64::new(0),
        }
    }

    /// Translation of `SDL_ShouldInit()`: returns true if the caller must
    /// perform initialization (and then call `set_initialized`).
    pub fn should_init(&self) -> bool {
        while self.status.load(Ordering::Acquire) != INITIALIZED {
            if self
                .status
                .compare_exchange(
                    UNINITIALIZED,
                    INITIALIZING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                self.thread.store(current_thread_id(), Ordering::Release);
                return true;
            }
            // Wait for the other thread to complete transition
            std::thread::sleep(Duration::from_millis(1));
        }
        false
    }

    /// Translation of `SDL_ShouldQuit()`: returns true if the caller must
    /// perform cleanup (and then call `set_initialized(false)`).
    pub fn should_quit(&self) -> bool {
        while self.status.load(Ordering::Acquire) != UNINITIALIZED {
            if self
                .status
                .compare_exchange(
                    INITIALIZED,
                    UNINITIALIZING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                self.thread.store(current_thread_id(), Ordering::Release);
                return true;
            }
            // Wait for the other thread to complete transition
            std::thread::sleep(Duration::from_millis(1));
        }
        false
    }

    /// Finish an initialization or cleanup started by [`should_init`](Self::should_init)
    /// or [`should_quit`](Self::should_quit). Translation of `SDL_SetInitialized()`.
    pub fn set_initialized(&self, initialized: bool) {
        debug_assert_eq!(self.thread.load(Ordering::Acquire), current_thread_id());
        self.status.store(
            if initialized {
                INITIALIZED
            } else {
                UNINITIALIZED
            },
            Ordering::Release,
        );
    }

    /// True if initialized, or if this very thread is mid-initialization
    /// (the `SDL_CheckInitLog()` re-entrancy pattern).
    pub(crate) fn is_initialized_or_initializing_here(&self) -> bool {
        let status = self.status.load(Ordering::Acquire);
        status == INITIALIZED
            || (status == INITIALIZING
                && self.thread.load(Ordering::Acquire) == current_thread_id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn thread_ids_unique_and_stable() {
        let a = current_thread_id();
        assert_eq!(a, current_thread_id());
        let b = std::thread::spawn(current_thread_id).join().unwrap();
        assert_ne!(a, b);
        assert_ne!(b, 0);
    }

    #[test]
    fn recursive_lock_and_exclusion() {
        let m = Arc::new(ReentrantMutex::new(std::cell::Cell::new(0)));
        {
            let _a = m.lock();
            let _b = m.lock(); // recursive
        }
        let mut handles = vec![];
        for _ in 0..8 {
            let m = m.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..1000 {
                    let g = m.lock();
                    g.set(g.get() + 1);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(m.lock().get(), 8000);
    }

    #[test]
    fn semaphore_basics() {
        let s = Semaphore::new(0);
        assert!(!s.try_wait());
        assert!(!s.wait_timeout(Some(Duration::from_millis(1))));
        s.signal();
        assert!(s.wait_timeout(Some(Duration::ZERO)));
        let s = Arc::new(Semaphore::new(0));
        let s2 = s.clone();
        let t = std::thread::spawn(move || s2.wait_timeout(None));
        s.signal();
        assert!(t.join().unwrap());
    }

    #[test]
    fn init_state_transitions() {
        let st = InitState::new();
        assert!(st.should_init());
        assert!(st.is_initialized_or_initializing_here());
        st.set_initialized(true);
        assert!(!st.should_init());
        assert!(st.should_quit());
        st.set_initialized(false);
        assert!(!st.should_quit());
    }
}
