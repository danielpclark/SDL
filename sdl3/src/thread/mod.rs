// Rust translation of src/thread/SDL_thread.c and src/thread/generic/SDL_sysmutex.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Internal synchronization helpers.
//!
//! `std::thread` and `std::sync` replace SDL's thread API for users of this
//! crate. What the translated core still needs internally is SDL's
//! *recursive* mutex (hint and log callbacks may re-enter the subsystem that
//! invoked them, and upstream relies on this), a counting semaphore for the
//! timer thread, and the `SDL_InitState` lifecycle machine.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::Condvar;
use std::time::Duration;

/// Stable numeric id per OS thread (translation of `SDL_ThreadID`; never 0).
pub(crate) type ThreadID = u64;

static NEXT_THREAD_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static THIS_THREAD_ID: ThreadID = NEXT_THREAD_ID.fetch_add(1, Ordering::Relaxed);
}

/// Translation of `SDL_GetCurrentThreadID()`.
pub(crate) fn current_thread_id() -> ThreadID {
    THIS_THREAD_ID.with(|id| *id)
}

/// Counting semaphore. Translation of the generic `SDL_Semaphore`.
pub(crate) struct Semaphore {
    count: std::sync::Mutex<u32>,
    cond: Condvar,
}

impl Semaphore {
    pub(crate) const fn new(initial_value: u32) -> Self {
        Semaphore {
            count: std::sync::Mutex::new(initial_value),
            cond: Condvar::new(),
        }
    }

    /// Translation of `SDL_WaitSemaphore()`.
    pub(crate) fn wait(&self) {
        let mut count = self.count.lock().unwrap_or_else(|e| e.into_inner());
        while *count == 0 {
            count = self.cond.wait(count).unwrap_or_else(|e| e.into_inner());
        }
        *count -= 1;
    }

    /// Translation of `SDL_TryWaitSemaphore()`.
    pub(crate) fn try_wait(&self) -> bool {
        let mut count = self.count.lock().unwrap_or_else(|e| e.into_inner());
        if *count > 0 {
            *count -= 1;
            true
        } else {
            false
        }
    }

    /// Translation of `SDL_WaitSemaphoreTimeoutNS()`; `None` waits forever.
    pub(crate) fn wait_timeout(&self, timeout: Option<Duration>) -> bool {
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

    /// Translation of `SDL_SignalSemaphore()`.
    pub(crate) fn signal(&self) {
        let mut count = self.count.lock().unwrap_or_else(|e| e.into_inner());
        *count += 1;
        self.cond.notify_one();
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

/// A recursive mutex guarding a value, built on [`RawMutex`].
///
/// The guard hands out `&T` only (re-entrant `&mut` would be unsound); use
/// a `RefCell`/`Mutex` inside for mutation, and never hold such a borrow
/// across a call into user code.
pub(crate) struct ReentrantMutex<T> {
    raw: RawMutex,
    data: T,
}

// SAFETY: only one thread holds the lock at a time and the guard hands out
// `&T` only to that thread, so `T: Send` is sufficient (same reasoning as
// `std::sync::ReentrantLock`).
unsafe impl<T: Send> Send for ReentrantMutex<T> {}
unsafe impl<T: Send> Sync for ReentrantMutex<T> {}

impl<T> ReentrantMutex<T> {
    pub(crate) const fn new(data: T) -> Self {
        ReentrantMutex {
            raw: RawMutex::new(),
            data,
        }
    }

    pub(crate) fn lock(&self) -> ReentrantMutexGuard<'_, T> {
        self.raw.lock();
        ReentrantMutexGuard { m: self }
    }
}

pub(crate) struct ReentrantMutexGuard<'a, T> {
    m: &'a ReentrantMutex<T>,
}

impl<T> std::ops::Deref for ReentrantMutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.m.data
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
pub(crate) struct InitState {
    status: AtomicI32,
    thread: AtomicU64,
}

impl InitState {
    pub(crate) const fn new() -> Self {
        InitState {
            status: AtomicI32::new(UNINITIALIZED),
            thread: AtomicU64::new(0),
        }
    }

    /// Translation of `SDL_ShouldInit()`: returns true if the caller must
    /// perform initialization (and then call `set_initialized`).
    pub(crate) fn should_init(&self) -> bool {
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

    /// Translation of `SDL_ShouldQuit()`.
    pub(crate) fn should_quit(&self) -> bool {
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

    /// Translation of `SDL_SetInitialized()`.
    pub(crate) fn set_initialized(&self, initialized: bool) {
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
