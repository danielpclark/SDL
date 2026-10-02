// Rust translation of src/thread/generic/SDL_syscond.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

// An implementation of condition variables using semaphores and mutexes
/*
  This implementation borrows heavily from the BeOS condition variable
  implementation, written by Christopher Tate and Owen Smith.  Thanks!
*/

use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use super::{ReentrantMutexGuard, Semaphore};

/// Translation of the counters of `SDL_cond_generic`, guarded by what was
/// `signal_sem` (a semaphore with initial value 1, used as a lock).
#[derive(Debug, Default)]
struct Counts {
    num_waiting: i32,
    num_signals: i32,
}

/// A condition variable for SDL's recursive [`ReentrantMutex`](super::ReentrantMutex).
/// Translation of `SDL_Condition` (the generic implementation).
///
/// `std::sync::Condvar` only works with `std::sync::Mutex`; this one works
/// with the recursive mutex. As in C, a wait releases the mutex *once*: if
/// the calling thread holds it recursively, other threads still can't take
/// it while this one waits.
#[derive(Debug)]
pub struct Condition {
    sem: Semaphore,
    handshake_sem: Semaphore,
    signal_sem: Mutex<Counts>,
}

impl Default for Condition {
    fn default() -> Self {
        Condition::new()
    }
}

impl Condition {
    /// Create a condition variable. Translation of `SDL_CreateCondition()`.
    pub const fn new() -> Condition {
        Condition {
            sem: Semaphore::new(0),
            handshake_sem: Semaphore::new(0),
            signal_sem: Mutex::new(Counts {
                num_waiting: 0,
                num_signals: 0,
            }),
        }
    }

    fn counts(&self) -> MutexGuard<'_, Counts> {
        self.signal_sem.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Restart one of the threads that are waiting on the condition variable.
    /// Translation of `SDL_SignalCondition()`.
    pub fn signal(&self) {
        /* If there are waiting threads not already signalled, then
          signal the condition and wait for the thread to respond.
        */
        let mut counts = self.counts();
        if counts.num_waiting > counts.num_signals {
            counts.num_signals += 1;
            self.sem.signal();
            drop(counts);
            self.handshake_sem.wait();
        }
    }

    /// Restart all threads that are waiting on the condition variable.
    /// Translation of `SDL_BroadcastCondition()`.
    pub fn broadcast(&self) {
        /* If there are waiting threads not already signalled, then
          signal the condition and wait for the thread to respond.
        */
        let mut counts = self.counts();
        if counts.num_waiting > counts.num_signals {
            let num_waiting = counts.num_waiting - counts.num_signals;
            counts.num_signals = counts.num_waiting;
            for _ in 0..num_waiting {
                self.sem.signal();
            }
            /* Now all released threads are blocked here, waiting for us.
              Collect them all (and win fabulous prizes!) :-)
            */
            drop(counts);
            for _ in 0..num_waiting {
                self.handshake_sem.wait();
            }
        }
    }

    /// Wait until the condition is signaled. Translation of `SDL_WaitCondition()`.
    ///
    /// The mutex behind `guard` must be held by this thread (the guard
    /// proves it); it is unlocked during the wait and locked again after.
    pub fn wait<T>(&self, guard: &ReentrantMutexGuard<'_, T>) {
        self.wait_timeout(guard, None);
    }

    /// Wait until the condition is signaled or `timeout` passes (`None`
    /// waits forever). Returns `false` on timeout.
    /// Translation of `SDL_WaitConditionTimeout()`.
    ///
    /// Typical use:
    ///
    /// ```text
    /// Thread A:
    ///     let guard = lock.lock();
    ///     while !condition {
    ///         cond.wait(&guard);
    ///     }
    ///     drop(guard);
    ///
    /// Thread B:
    ///     let guard = lock.lock();
    ///     ...
    ///     condition = true;
    ///     ...
    ///     cond.signal();
    ///     drop(guard);
    /// ```
    pub fn wait_timeout<T>(
        &self,
        guard: &ReentrantMutexGuard<'_, T>,
        timeout: Option<Duration>,
    ) -> bool {
        /* Obtain the protection mutex, and increment the number of waiters.
          This allows the signal mechanism to only perform a signal if there
          are waiting threads.
        */
        self.counts().num_waiting += 1;

        // Unlock the mutex, as is required by condition variable semantics
        guard.raw().unlock();

        // Wait for a signal
        let result = self.sem.wait_timeout(timeout);

        /* Let the signaler know we have completed the wait, otherwise
          the signaler can race ahead and get the condition semaphore
          if we are stopped between the mutex unlock and semaphore wait,
          giving a deadlock.  See the following URL for details:
          http://web.archive.org/web/20010914175514/http://www-classic.be.com/aboutbe/benewsletter/volume_III/Issue40.html#Workshop
        */
        {
            let mut counts = self.counts();
            if counts.num_signals > 0 {
                self.handshake_sem.signal();
                counts.num_signals -= 1;
            }
            counts.num_waiting -= 1;
        }

        // Lock the mutex, as is required by condition variable semantics
        guard.raw().lock();

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread::ReentrantMutex;
    use std::cell::Cell;
    use std::sync::Arc;

    #[test]
    fn signal_broadcast_timeout() {
        let shared = Arc::new((ReentrantMutex::new(Cell::new(0)), Condition::new()));

        // Timeout with nothing signaled.
        {
            let guard = shared.0.lock();
            assert!(!shared
                .1
                .wait_timeout(&guard, Some(Duration::from_millis(10))));
        }
        shared.1.signal(); // nobody waiting: no-op
        shared.1.broadcast();

        let mut waiters = Vec::new();
        for _ in 0..3 {
            let s = shared.clone();
            waiters.push(std::thread::spawn(move || {
                let guard = s.0.lock();
                while guard.get() == 0 {
                    s.1.wait(&guard);
                }
                guard.get()
            }));
        }
        std::thread::sleep(Duration::from_millis(20));
        {
            let guard = shared.0.lock();
            guard.set(7);
        }
        shared.1.broadcast();
        for w in waiters {
            assert_eq!(w.join().unwrap(), 7);
        }

        // signal() wakes one waiter.
        let s = shared.clone();
        let w = std::thread::spawn(move || {
            let guard = s.0.lock();
            while guard.get() != 8 {
                s.1.wait(&guard);
            }
        });
        loop {
            let guard = shared.0.lock();
            guard.set(8);
            drop(guard);
            shared.1.signal();
            if w.is_finished() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        w.join().unwrap();
    }
}
