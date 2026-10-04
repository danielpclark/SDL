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

/// Set the priority for the current thread. Translation of
/// `SDL_SetCurrentThreadPriority()` and `SDL_SYS_SetThreadPriority()`.
///
/// On Unix the scheduler policy follows `SDL_HINT_THREAD_PRIORITY_POLICY`
/// and `SDL_HINT_THREAD_FORCE_REALTIME_TIME_CRITICAL`; on Linux the priority
/// is a nice level (or a realtime priority for `SCHED_RR`/`SCHED_FIFO`) set
/// with `setpriority()`. Raising the priority usually needs privileges; the
/// error is then "setpriority() failed" (or "pthread_setschedparam()
/// failed"), as upstream reports it when RealtimeKit refuses too.
pub fn set_current_thread_priority(priority: ThreadPriority) -> Result<()> {
    sys_set_thread_priority(priority)
}

#[cfg(unix)]
fn sys_set_thread_priority(priority: ThreadPriority) -> Result<()> {
    use crate::hints;
    // SAFETY: pthread_self() has no preconditions.
    let thread = unsafe { libc::pthread_self() };
    let policyhint = hints::get(hints::THREAD_PRIORITY_POLICY);
    let timecritical_realtime_hint =
        hints::get_bool(hints::THREAD_FORCE_REALTIME_TIME_CRITICAL, false);

    let mut policy: libc::c_int = 0;
    // SAFETY: sched_param is plain data; both out-pointers are valid.
    let mut sched: libc::sched_param = unsafe { std::mem::zeroed() };
    // SAFETY: as above.
    if unsafe { libc::pthread_getschedparam(thread, &mut policy, &mut sched) } != 0 {
        return Err(Error::new("pthread_getschedparam() failed"));
    }

    /* Higher priority levels may require changing the pthread scheduler policy
     * for the thread.  SDL will make such changes by default but there is
     * also a hint allowing that behavior to be overridden. */
    let mut pri_policy = match priority {
        ThreadPriority::Low | ThreadPriority::Normal => libc::SCHED_OTHER,
        // Apple requires SCHED_RR for high priority threads
        ThreadPriority::High | ThreadPriority::TimeCritical if cfg!(target_vendor = "apple") => {
            libc::SCHED_RR
        }
        ThreadPriority::High | ThreadPriority::TimeCritical => libc::SCHED_OTHER,
    };

    if timecritical_realtime_hint && priority == ThreadPriority::TimeCritical {
        pri_policy = libc::SCHED_RR;
    }

    policy = match policyhint.as_deref() {
        Some("current") => policy, // Leave current thread scheduler policy unchanged
        Some("other") => libc::SCHED_OTHER,
        Some("rr") => libc::SCHED_RR,
        Some("fifo") => libc::SCHED_FIFO,
        _ => pri_policy,
    };

    #[cfg(target_os = "linux")]
    {
        let _ = sched;
        // SAFETY: gettid has no preconditions.
        let linux_tid = unsafe { libc::syscall(libc::SYS_gettid) };
        set_linux_thread_priority_and_policy(linux_tid, priority, policy)
    }
    #[cfg(not(target_os = "linux"))]
    {
        // SAFETY: sched_get_priority_min/max take a policy constant.
        let (min_priority, max_priority) = unsafe {
            (
                libc::sched_get_priority_min(policy),
                libc::sched_get_priority_max(policy),
            )
        };
        sched.sched_priority = match priority {
            ThreadPriority::Low => min_priority,
            ThreadPriority::TimeCritical => max_priority,
            // Apple has a specific set of thread priorities
            _ if cfg!(target_vendor = "apple") && min_priority == 15 && max_priority == 47 => {
                if priority == ThreadPriority::High {
                    45
                } else {
                    37
                }
            }
            _ => {
                let mut p = min_priority + (max_priority - min_priority) / 2;
                if priority == ThreadPriority::High {
                    p += (max_priority - min_priority) / 4;
                }
                p
            }
        };
        // SAFETY: sched is a valid sched_param for this policy.
        if unsafe { libc::pthread_setschedparam(thread, policy, &sched) } != 0 {
            return Err(Error::new("pthread_setschedparam() failed"));
        }
        Ok(())
    }
}

/// The maximum realtime priority (RealtimeKit's `MaxRealtimePriority`
/// default, used without it).
#[cfg(target_os = "linux")]
const RTKIT_MAX_REALTIME_PRIORITY: i32 = 99;

/// Set a Linux thread's nice level. Translation of
/// `SDL_SetLinuxThreadPriority()` (`core/linux/SDL_threadprio.c`), without
/// the RealtimeKit fallback, which needs D-Bus.
#[cfg(target_os = "linux")]
pub fn set_linux_thread_priority(thread_id: i64, priority: i32) -> Result<()> {
    // SAFETY: setpriority takes plain integers.
    if unsafe { libc::setpriority(libc::PRIO_PROCESS, thread_id as libc::id_t, priority) } == 0 {
        return Ok(());
    }
    Err(Error::new("setpriority() failed"))
}

/// Set a Linux thread's priority for a scheduler policy. Translation of
/// `SDL_SetLinuxThreadPriorityAndPolicy()` (`core/linux/SDL_threadprio.c`),
/// without the RealtimeKit fallback, which needs D-Bus.
#[cfg(target_os = "linux")]
pub fn set_linux_thread_priority_and_policy(
    thread_id: i64,
    sdl_priority: ThreadPriority,
    sched_policy: i32,
) -> Result<()> {
    if sched_policy == libc::SCHED_RR || sched_policy == libc::SCHED_FIFO {
        // Realtime priorities are only granted by RealtimeKit
        // (MakeThreadRealtimeWithPID), which needs D-Bus.
        let _os_priority = match sdl_priority {
            ThreadPriority::Low => 1,
            ThreadPriority::High => RTKIT_MAX_REALTIME_PRIORITY * 3 / 4,
            ThreadPriority::TimeCritical => RTKIT_MAX_REALTIME_PRIORITY,
            ThreadPriority::Normal => RTKIT_MAX_REALTIME_PRIORITY / 2,
        };
    } else {
        let os_priority = match sdl_priority {
            ThreadPriority::Low => 19,
            ThreadPriority::High => -10,
            ThreadPriority::TimeCritical => -20,
            ThreadPriority::Normal => 0,
        };
        // SAFETY: setpriority takes plain integers.
        if unsafe { libc::setpriority(libc::PRIO_PROCESS, thread_id as libc::id_t, os_priority) }
            == 0
        {
            return Ok(());
        }
    }
    Err(Error::new("setpriority() failed"))
}

#[cfg(windows)]
fn sys_set_thread_priority(priority: ThreadPriority) -> Result<()> {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST, THREAD_PRIORITY_LOWEST,
        THREAD_PRIORITY_NORMAL, THREAD_PRIORITY_TIME_CRITICAL,
    };
    let value = match priority {
        ThreadPriority::Low => THREAD_PRIORITY_LOWEST,
        ThreadPriority::High => THREAD_PRIORITY_HIGHEST,
        ThreadPriority::TimeCritical => THREAD_PRIORITY_TIME_CRITICAL,
        ThreadPriority::Normal => THREAD_PRIORITY_NORMAL,
    };
    // SAFETY: GetCurrentThread returns a pseudo-handle valid for this call.
    if unsafe { SetThreadPriority(GetCurrentThread(), value) } == 0 {
        return Err(crate::core::windows::set_error("SetThreadPriority()"));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn sys_set_thread_priority(_priority: ThreadPriority) -> Result<()> {
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
    }

    #[test]
    fn priorities() {
        let _l = crate::test_support::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let t = Thread::spawn("priorities", || {
            // Lowering is always allowed; raising may need privileges, and
            // then fails with upstream's message.
            set_current_thread_priority(ThreadPriority::Low).unwrap();
            #[cfg(target_os = "linux")]
            {
                // SAFETY: getpriority takes plain integers.
                let tid = unsafe { libc::syscall(libc::SYS_gettid) };
                // SAFETY: as above.
                assert_eq!(
                    unsafe { libc::getpriority(libc::PRIO_PROCESS, tid as libc::id_t) },
                    19
                );
            }
            match set_current_thread_priority(ThreadPriority::High) {
                Ok(()) => {}
                Err(e) => assert!(
                    ["setpriority() failed", "pthread_setschedparam() failed"]
                        .contains(&e.message())
                        || e.message().starts_with("SetThreadPriority()"),
                    "{}",
                    e.message()
                ),
            }
            0
        })
        .unwrap();
        assert_eq!(t.wait(), 0);
    }
}
