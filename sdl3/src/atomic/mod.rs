// Rust translation of src/atomic/SDL_spinlock.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Atomic operations.
//!
//! Upstream `SDL_atomic.c` is a per-compiler shim over compare-and-swap that
//! `core::sync::atomic` already provides portably. The one piece of real
//! logic, the spin lock, is translated here with a guard-based API.

use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicI32, Ordering};

/// A spin lock protecting a value. Translation of `SDL_SpinLock`.
///
/// Spin locks are appropriate only for very short critical sections; the
/// translated SDL uses one to hand timers to the timer thread.
pub struct SpinLock<T = ()> {
    lock: AtomicI32,
    data: UnsafeCell<T>,
}

// SAFETY: access to `data` is serialized by `lock`.
unsafe impl<T: Send> Send for SpinLock<T> {}
unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T: Default> Default for SpinLock<T> {
    fn default() -> Self {
        SpinLock::new(T::default())
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for SpinLock<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.try_lock() {
            Some(g) => f.debug_struct("SpinLock").field("data", &*g).finish(),
            None => f
                .debug_struct("SpinLock")
                .field("data", &"<locked>")
                .finish(),
        }
    }
}

impl<T> SpinLock<T> {
    /// An unlocked spin lock around `data`.
    pub const fn new(data: T) -> Self {
        SpinLock {
            lock: AtomicI32::new(0),
            data: UnsafeCell::new(data),
        }
    }

    /// Try to lock without blocking. Translation of `SDL_TryLockSpinlock()`.
    #[inline]
    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        // __sync_lock_test_and_set(lock, 1) == 0
        if self.lock.swap(1, Ordering::Acquire) == 0 {
            Some(SpinLockGuard { lock: self })
        } else {
            None
        }
    }

    /// Lock, spinning (then yielding) until available. Translation of `SDL_LockSpinlock()`.
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        let mut iterations = 0;
        // FIXME: Should we have an eventual timeout?
        loop {
            if let Some(guard) = self.try_lock() {
                return guard;
            }
            if iterations < 32 {
                iterations += 1;
                std::hint::spin_loop(); // SDL_CPUPauseInstruction()
            } else {
                // !!! FIXME: this doesn't definitely give up the current timeslice, it does different things on various platforms.
                std::thread::yield_now(); // SDL_Delay(0)
            }
        }
    }

    /// Consume the lock and return the protected value.
    pub fn into_inner(self) -> T {
        self.data.into_inner()
    }

    /// Mutable access without locking (statically exclusive).
    pub fn get_mut(&mut self) -> &mut T {
        self.data.get_mut()
    }
}

/// Guard returned by [`SpinLock::lock`]; unlocks on drop (`SDL_UnlockSpinlock()`).
pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<T> Deref for SpinLockGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: we hold the lock.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: we hold the lock exclusively.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        // __sync_lock_release(lock)
        self.lock.lock.store(0, Ordering::Release);
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for SpinLockGuard<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (**self).fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn spinlock_excludes() {
        let lock = Arc::new(SpinLock::new(0usize));
        let mut hs = vec![];
        for _ in 0..4 {
            let lock = lock.clone();
            hs.push(std::thread::spawn(move || {
                for _ in 0..10_000 {
                    *lock.lock() += 1;
                }
            }));
        }
        for h in hs {
            h.join().unwrap();
        }
        assert_eq!(*lock.lock(), 40_000);
        let g = lock.try_lock().unwrap();
        assert!(lock.try_lock().is_none());
        assert!(format!("{lock:?}").contains("locked"));
        drop(g);
        assert!(lock.try_lock().is_some());
    }
}
