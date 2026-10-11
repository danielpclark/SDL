// Rust translation of src/timer/windows/SDL_systimer.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Windows performance counter (`QueryPerformanceCounter()`) and delay
//! (a high resolution waitable timer per thread, else a wait on an event
//! that is never signaled, else `Sleep()`), and the system timer
//! resolution `SDL_timer.c` requests with `timeBeginPeriod()`.

use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

use windows_sys::core::{BOOL, PCWSTR};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows_sys::Win32::System::Threading::{
    CreateEventW, Sleep, WaitForSingleObject, WaitForSingleObjectEx, INFINITE, PTIMERAPCROUTINE,
    TIMER_ALL_ACCESS,
};

use super::NS_PER_MS;

/// Translation of `SDL_GetPerformanceCounter()` (Windows).
pub(super) fn get_performance_counter() -> u64 {
    let mut counter: i64 = 0;
    // SAFETY: counter is a valid out-parameter.
    let rc = unsafe { QueryPerformanceCounter(&mut counter) };
    debug_assert!(rc != 0); // this should _never_ fail if you're on XP or later.
    counter as u64
}

/// Translation of `SDL_GetPerformanceFrequency()` (Windows).
pub(super) fn get_performance_frequency() -> u64 {
    let mut frequency: i64 = 0;
    // SAFETY: frequency is a valid out-parameter.
    let rc = unsafe { QueryPerformanceFrequency(&mut frequency) };
    debug_assert!(rc != 0); // this should _never_ fail if you're on XP or later.
    frequency as u64
}

/// A handle closed when the thread that made it exits
/// (`SDL_CleanupWaitableHandle()` as the TLS destructor).
struct WaitableHandle(HANDLE);

impl Drop for WaitableHandle {
    fn drop(&mut self) {
        // SAFETY: the handle was created by this thread and is closed once.
        unsafe { CloseHandle(self.0) };
    }
}

thread_local! {
    static TLS_EVENT_HANDLE: std::cell::RefCell<Option<WaitableHandle>> =
        const { std::cell::RefCell::new(None) };
    static TLS_TIMER_HANDLE: std::cell::RefCell<Option<WaitableHandle>> =
        const { std::cell::RefCell::new(None) };
}

/// Translation of `SDL_GetWaitableEvent()`: this thread's auto-reset event
/// (never signaled), or `None`.
fn get_waitable_event() -> Option<HANDLE> {
    TLS_EVENT_HANDLE
        .try_with(|tls| {
            let mut tls = tls.borrow_mut();
            if tls.is_none() {
                // SAFETY: no attributes, auto-reset, initially unsignaled, unnamed.
                let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
                if !event.is_null() {
                    *tls = Some(WaitableHandle(event));
                }
            }
            tls.as_ref().map(|h| h.0)
        })
        .ok()
        .flatten()
}

/* CREATE_WAITABLE_TIMER_HIGH_RESOLUTION flag was added in Windows 10 version 1803. */
const CREATE_WAITABLE_TIMER_HIGH_RESOLUTION: u32 = 0x2;

type PfnCreateWaitableTimerExW =
    unsafe extern "system" fn(*const SECURITY_ATTRIBUTES, PCWSTR, u32, u32) -> HANDLE;
type PfnSetWaitableTimerEx = unsafe extern "system" fn(
    HANDLE,
    *const i64,
    i32,
    PTIMERAPCROUTINE,
    *const c_void,
    *const c_void,
    u32,
) -> BOOL;
type PfnCreateWaitableTimerW =
    unsafe extern "system" fn(*const SECURITY_ATTRIBUTES, BOOL, PCWSTR) -> HANDLE;
type PfnSetWaitableTimer = unsafe extern "system" fn(
    HANDLE,
    *const i64,
    i32,
    PTIMERAPCROUTINE,
    *const c_void,
    BOOL,
) -> BOOL;

/// The waitable timer functions found in kernel32.dll.
#[derive(Clone, Copy)]
struct TimerFunctions {
    create_waitable_timer_ex_w: Option<PfnCreateWaitableTimerExW>,
    set_waitable_timer_ex: Option<PfnSetWaitableTimerEx>,
    create_waitable_timer_w: Option<PfnCreateWaitableTimerW>,
    set_waitable_timer: Option<PfnSetWaitableTimer>,
}

/// The lookup `SDL_GetWaitableTimer()` does the first time (`initialized`
/// is whether a timer can be created and set: `Some`).
fn timer_functions() -> Option<TimerFunctions> {
    static FUNCTIONS: OnceLock<Option<TimerFunctions>> = OnceLock::new();
    *FUNCTIONS.get_or_init(|| {
        let name = crate::core::windows::utf8_to_wide("kernel32.dll");
        // SAFETY: name is NUL-terminated; kernel32 stays loaded, and each
        // export found has the signature it is cast to.
        unsafe {
            let module = GetModuleHandleW(name.as_ptr());
            if module.is_null() {
                return None;
            }
            let get = |f: &std::ffi::CStr| GetProcAddress(module, f.as_ptr().cast());
            let mut fns = TimerFunctions {
                create_waitable_timer_ex_w: get(c"CreateWaitableTimerExW")
                    .map(|f| std::mem::transmute::<_, PfnCreateWaitableTimerExW>(f)), // Windows 7 and up
                set_waitable_timer_ex: None,
                create_waitable_timer_w: None,
                set_waitable_timer: None,
            };
            if fns.create_waitable_timer_ex_w.is_none() {
                fns.create_waitable_timer_w = get(c"CreateWaitableTimerW")
                    .map(|f| std::mem::transmute::<_, PfnCreateWaitableTimerW>(f));
            }
            fns.set_waitable_timer_ex = get(c"SetWaitableTimerEx")
                .map(|f| std::mem::transmute::<_, PfnSetWaitableTimerEx>(f)); // Windows Vista and up
            if fns.set_waitable_timer_ex.is_none() {
                fns.set_waitable_timer = get(c"SetWaitableTimer")
                    .map(|f| std::mem::transmute::<_, PfnSetWaitableTimer>(f));
            }
            let initialized = (fns.create_waitable_timer_ex_w.is_some()
                || fns.create_waitable_timer_w.is_some())
                && (fns.set_waitable_timer_ex.is_some() || fns.set_waitable_timer.is_some());
            initialized.then_some(fns)
        }
    })
}

/// Translation of `SDL_GetWaitableTimer()`: this thread's waitable timer
/// (high resolution where Windows has those), or `None`.
fn get_waitable_timer() -> Option<(HANDLE, TimerFunctions)> {
    let fns = timer_functions()?;

    let timer = TLS_TIMER_HANDLE
        .try_with(|tls| {
            let mut tls = tls.borrow_mut();
            if tls.is_none() {
                // SAFETY: no attributes, unnamed; the flags are documented ones.
                let timer = unsafe {
                    if let Some(create) = fns.create_waitable_timer_ex_w {
                        create(
                            std::ptr::null(),
                            std::ptr::null(),
                            CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                            TIMER_ALL_ACCESS,
                        )
                    } else if let Some(create) = fns.create_waitable_timer_w {
                        create(std::ptr::null(), 1, std::ptr::null())
                    } else {
                        std::ptr::null_mut()
                    }
                };
                if !timer.is_null() {
                    *tls = Some(WaitableHandle(timer));
                }
            }
            tls.as_ref().map(|h| h.0)
        })
        .ok()
        .flatten()?;
    Some((timer, fns))
}

/// Translation of `SDL_SYS_DelayNS()` (Windows).
pub(super) fn sys_delay_ns(mut ns: u64) {
    if let Some((timer, fns)) = get_waitable_timer() {
        let due_time: i64 = -((ns as i64) / 100);
        // SAFETY: timer is this thread's timer; due_time outlives the
        // calls; no completion routine.
        let set = unsafe {
            match (fns.set_waitable_timer_ex, fns.set_waitable_timer) {
                (Some(set_ex), _)
                    if set_ex(
                        timer,
                        &due_time,
                        0,
                        None,
                        std::ptr::null(),
                        std::ptr::null(),
                        0,
                    ) != 0 =>
                {
                    true
                }
                // Note (upstream): when SetWaitableTimerEx() exists but
                // fails, upstream calls the NULL pSetWaitableTimer.
                (_, Some(set)) => set(timer, &due_time, 0, None, std::ptr::null(), 0) != 0,
                _ => false,
            }
        };
        if set {
            // SAFETY: timer is a valid handle.
            unsafe { WaitForSingleObject(timer, INFINITE) };
        }
        return;
    }

    let max_delay: u64 = 0xffffffff * NS_PER_MS;
    if ns > max_delay {
        ns = max_delay;
    }
    let delay = (ns / NS_PER_MS) as u32;

    if let Some(event) = get_waitable_event() {
        // SAFETY: event is a valid handle.
        unsafe { WaitForSingleObjectEx(event, delay, 0) };
        return;
    }

    // SAFETY: Sleep has no preconditions.
    unsafe { Sleep(delay) };
}

/// The period of our `timeBeginPeriod()` request (0: none).
static TIMER_PERIOD: Mutex<i32> = Mutex::new(0);

/// The Windows part of `SDL_SetSystemTimerResolutionMS()`
/// (`HAVE_TIME_BEGIN_PERIOD`).
pub(super) fn set_system_timer_resolution_ms(period: i32) {
    let mut timer_period = TIMER_PERIOD.lock().unwrap_or_else(|e| e.into_inner());
    if period != *timer_period {
        if *timer_period != 0 {
            // SAFETY: balances our earlier timeBeginPeriod().
            unsafe { timeEndPeriod(*timer_period as u32) };
        }

        *timer_period = period;

        if *timer_period != 0 {
            // SAFETY: timeBeginPeriod has no preconditions.
            unsafe { timeBeginPeriod(*timer_period as u32) };
        }
    }
}

#[cfg(test)]
pub(super) fn timer_period() -> i32 {
    *TIMER_PERIOD.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
pub(super) fn has_waitable_timer() -> bool {
    get_waitable_timer().is_some()
}
