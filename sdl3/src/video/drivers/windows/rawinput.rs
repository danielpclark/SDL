// Rust translation of src/video/windows/SDL_windowsrawinput.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Raw mouse and keyboard input, read on a dedicated thread with a
//! message-only window, unless the driver reads them through GameInput.

use std::os::windows::io::AsRawHandle;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HWND, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{
    CreateEventW, GetCurrentThread, SetEvent, SetThreadPriority, WaitForMultipleObjects,
    WaitForSingleObject, INFINITE, THREAD_PRIORITY_TIME_CRITICAL,
};
use windows_sys::Win32::UI::Input::{
    RegisterRawInputDevices, RAWINPUTDEVICE, RIDEV_INPUTSINK, RIDEV_NOHOTKEYS, RIDEV_NOLEGACY,
    RIDEV_REMOVE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetQueueStatus, MsgWaitForMultipleObjects, HWND_MESSAGE,
    QS_RAWINPUT,
};

use super::events::poll_raw_input;
use super::{video_data, VideoData};
use crate::core::windows::{set_error, utf8_to_wide};
use crate::error::{Error, Result};
use crate::events::{keyboard, pen};
use crate::log::Category;

const ENABLE_RAW_MOUSE_INPUT: u32 = 0x01;
const RAW_MOUSE_FLAG_NOLEGACY: u32 = 0x02;
const ENABLE_RAW_KEYBOARD_INPUT: u32 = 0x10;
const RAW_KEYBOARD_FLAG_NOHOTKEYS: u32 = 0x20;
const RAW_KEYBOARD_FLAG_INPUTSINK: u32 = 0x40;

const USB_USAGEPAGE_GENERIC_DESKTOP: u16 = 0x0001;
const USB_USAGE_GENERIC_MOUSE: u16 = 0x0002;
const USB_USAGE_GENERIC_KEYBOARD: u16 = 0x0006;

/// An event handle that may move between threads.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Event(HANDLE);
// SAFETY: kernel handles are process-wide.
unsafe impl Send for Event {}

/// Translation of `RawInputThreadData` (`done` and `flags` are the atomics
/// below; the thread handle is the `JoinHandle`'s).
struct RawInputThreadData {
    ready_event: Option<Event>,
    signal_event: Option<Event>,
    thread: Option<JoinHandle<()>>,
}

static THREAD_DATA: Mutex<RawInputThreadData> = Mutex::new(RawInputThreadData {
    ready_event: None,
    signal_event: None,
    thread: None,
});
/// `thread_data.done`
static DONE: AtomicBool = AtomicBool::new(false);
/// `thread_data.flags`: the thread sets this to the actually-set flags if updating state failed
static FLAGS: AtomicU32 = AtomicU32::new(0);

fn thread_data() -> std::sync::MutexGuard<'static, RawInputThreadData> {
    THREAD_DATA.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `RawInputIterateResult`.
#[derive(PartialEq, Eq)]
enum RawInputIterateResult {
    Quit,
    Update,
    Continue,
}

/// Translation of `IterateRawInputThread()`.
fn iterate_raw_input_thread(signal_event: HANDLE) -> RawInputIterateResult {
    // The high-order word of GetQueueStatus() will let us know if there's immediate raw input to be processed.
    // A (necessary!) side effect is that it also marks the message queue bits as stale,
    // so MsgWaitForMultipleObjects will block.

    // Any pending flag update won't be processed until the queue drains, but this is
    // at most one poll cycle since GetQueueStatus clears the wake bits.
    // SAFETY: GetQueueStatus has no preconditions.
    if ((unsafe { GetQueueStatus(QS_RAWINPUT) } >> 16) & QS_RAWINPUT) != 0 {
        return if DONE.load(Ordering::Acquire) {
            RawInputIterateResult::Quit
        } else {
            RawInputIterateResult::Continue
        };
    }

    const WAIT_SIGNAL: u32 = WAIT_OBJECT_0;
    const WAIT_INPUT: u32 = WAIT_OBJECT_0 + 1;

    // There wasn't anything, so we'll wait until new data (or signal_event) wakes us up.
    // SAFETY: signal_event is a live event handle.
    let wait_status =
        unsafe { MsgWaitForMultipleObjects(1, &signal_event, 0, INFINITE, QS_RAWINPUT) };
    if wait_status == WAIT_SIGNAL {
        // signal_event can only be set if we need to update or quit.
        if DONE.load(Ordering::Acquire) {
            RawInputIterateResult::Quit
        } else {
            RawInputIterateResult::Update
        }
    } else if wait_status == WAIT_INPUT {
        RawInputIterateResult::Continue
    } else {
        crate::log::warn!(
            Category::Input,
            "Raw input thread exiting, unexpected wait result: {}",
            wait_status
        );
        RawInputIterateResult::Quit
    }
}

/// Translation of `UpdateRawInputDeviceFlags()`.
fn update_raw_input_device_flags(window: HWND, last_flags: u32, new_flags: u32) -> bool {
    // We had nothing enabled, and we're trying to stop everything. Nothing to do.
    if last_flags == new_flags {
        return true;
    }

    let empty = RAWINPUTDEVICE {
        usUsagePage: 0,
        usUsage: 0,
        dwFlags: 0,
        hwndTarget: std::ptr::null_mut(),
    };
    let mut devices = [empty, empty];
    let mut count = 0usize;

    let old_mouse_flags = last_flags & (ENABLE_RAW_MOUSE_INPUT | RAW_MOUSE_FLAG_NOLEGACY);
    let new_mouse_flags = new_flags & (ENABLE_RAW_MOUSE_INPUT | RAW_MOUSE_FLAG_NOLEGACY);

    if old_mouse_flags != new_mouse_flags {
        devices[count].usUsagePage = USB_USAGEPAGE_GENERIC_DESKTOP;
        devices[count].usUsage = USB_USAGE_GENERIC_MOUSE;

        if (new_flags & ENABLE_RAW_MOUSE_INPUT) != 0 {
            devices[count].dwFlags = 0;
            devices[count].hwndTarget = window;
            if (new_mouse_flags & RAW_MOUSE_FLAG_NOLEGACY) != 0 {
                devices[count].dwFlags |= RIDEV_NOLEGACY;
            }
        } else {
            devices[count].dwFlags = RIDEV_REMOVE;
            devices[count].hwndTarget = std::ptr::null_mut();
        }

        count += 1;
    }

    let kb_mask =
        ENABLE_RAW_KEYBOARD_INPUT | RAW_KEYBOARD_FLAG_NOHOTKEYS | RAW_KEYBOARD_FLAG_INPUTSINK;
    let old_kb_flags = last_flags & kb_mask;
    let new_kb_flags = new_flags & kb_mask;

    if old_kb_flags != new_kb_flags {
        devices[count].usUsagePage = USB_USAGEPAGE_GENERIC_DESKTOP;
        devices[count].usUsage = USB_USAGE_GENERIC_KEYBOARD;

        if (new_kb_flags & ENABLE_RAW_KEYBOARD_INPUT) != 0 {
            devices[count].dwFlags = 0;
            devices[count].hwndTarget = window;
            if (new_kb_flags & RAW_KEYBOARD_FLAG_NOHOTKEYS) != 0 {
                devices[count].dwFlags |= RIDEV_NOHOTKEYS;
            }
            if (new_kb_flags & RAW_KEYBOARD_FLAG_INPUTSINK) != 0 {
                devices[count].dwFlags |= RIDEV_INPUTSINK;
            }
        } else {
            devices[count].dwFlags = RIDEV_REMOVE;
            devices[count].hwndTarget = std::ptr::null_mut();
        }

        count += 1;
    }

    // SAFETY: `count` devices are filled in.
    unsafe {
        RegisterRawInputDevices(
            devices.as_ptr(),
            count as u32,
            size_of::<RAWINPUTDEVICE>() as u32,
        ) != 0
    }
}

/// Translation of `WIN_RawInputThread()` (named by `std::thread`, as
/// `SDL_SYS_SetupThread("SDLRawInput")` does).
fn raw_input_thread(ready_event: Event, signal_event: Event) {
    let Some(data) = video_data() else {
        return;
    };

    let class = utf8_to_wide("Message");
    // SAFETY: a message-only window of the predefined "Message" class.
    let window = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            std::ptr::null(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if window.is_null() {
        return;
    }

    // This doesn't *really* need to be atomic, because the parent is waiting for us
    let mut last_flags = FLAGS.load(Ordering::Acquire);
    if !update_raw_input_device_flags(window, 0, last_flags) {
        // SAFETY: created above.
        unsafe { DestroyWindow(window) };
        return;
    }

    // Make sure we get events as soon as possible
    // SAFETY: the current thread's pseudo handle; ready_event is live.
    unsafe {
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);

        // Tell the parent we're ready to go!
        SetEvent(ready_event.0);
    }

    let mut idle_begin = crate::timer::ticks_ns();
    loop {
        let iter_result = iterate_raw_input_thread(signal_event.0);
        match iter_result {
            RawInputIterateResult::Quit => break,
            RawInputIterateResult::Update => {
                let new_flags = FLAGS.load(Ordering::Acquire);
                if !update_raw_input_device_flags(window, last_flags, new_flags) {
                    // Revert the shared flags so the main thread can detect the failure
                    FLAGS.store(last_flags, Ordering::Release);
                } else {
                    last_flags = new_flags;
                }
            }
            RawInputIterateResult::Continue => {
                let idle_end = crate::timer::ticks_ns();
                let idle_time = idle_end.wrapping_sub(idle_begin);
                let usb_8khz_interval: u64 = 125 * 1000;
                let poll_start = if idle_time < usb_8khz_interval {
                    data.raw.with(|r| r.last_rawinput_poll)
                } else {
                    idle_end
                };

                poll_raw_input(&data, poll_start);

                // Reset idle_begin for the next go-around
                idle_begin = crate::timer::ticks_ns();
            }
        }
    }

    let fake_pen_id = data
        .raw
        .with(|r| std::mem::take(&mut r.raw_input_fake_pen_id));
    if fake_pen_id != 0 {
        pen::remove_pen_device(Duration::ZERO, keyboard::keyboard_focus(), fake_pen_id);
    }

    // Reset this here, since if we're exiting due to failure, WIN_UpdateRawInputEnabled would see a stale value.
    FLAGS.store(0, Ordering::Release);

    update_raw_input_device_flags(std::ptr::null_mut(), last_flags, 0);

    // SAFETY: created above.
    unsafe { DestroyWindow(window) };
}

/// Translation of `CleanupRawInputThreadData()`.
fn cleanup_raw_input_thread_data(td: &mut RawInputThreadData) {
    if let Some(thread) = td.thread.take() {
        DONE.store(true, Ordering::Release);
        // SAFETY: the event and thread handles are live.
        unsafe {
            if let Some(signal) = td.signal_event {
                SetEvent(signal.0);
            }
            WaitForSingleObject(thread.as_raw_handle(), 3000);
        }
        // (CloseHandle: the JoinHandle closes the thread handle when dropped)
        drop(thread);
    }

    // SAFETY: the events were created by CreateEvent.
    unsafe {
        if let Some(ready) = td.ready_event.take() {
            CloseHandle(ready.0);
        }

        if let Some(signal) = td.signal_event.take() {
            CloseHandle(signal.0);
        }
    }

    DONE.store(false, Ordering::Release);
    FLAGS.store(0, Ordering::Release);
}

/// `CreateEvent(NULL, FALSE, FALSE, NULL)`
fn create_event() -> Result<Event> {
    // SAFETY: an unnamed auto-reset event.
    let h = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
    if h.is_null() {
        return Err(set_error("CreateEvent"));
    }
    Ok(Event(h))
}

/// Computes the desired raw input flags from SDL_VideoData and ensures the
/// raw input thread's device registrations match.
/// Creates the thread on first use, only WIN_QuitRawInput actually shuts it down.
/// Translation of `WIN_UpdateRawInputEnabled()`.
fn update_raw_input_enabled(data: &VideoData) -> Result<()> {
    let mut desired_flags = 0u32;

    let gameinput_context = super::gameinput::has_game_input(data);
    data.state.with(|s| {
        if gameinput_context {
            return;
        }
        if s.raw_mouse_enabled {
            desired_flags |= ENABLE_RAW_MOUSE_INPUT;
            if s.raw_mouse_flag_nolegacy {
                desired_flags |= RAW_MOUSE_FLAG_NOLEGACY;
            }
        }
        if s.raw_keyboard_enabled {
            desired_flags |= ENABLE_RAW_KEYBOARD_INPUT;
            if s.raw_keyboard_flag_nohotkeys {
                desired_flags |= RAW_KEYBOARD_FLAG_NOHOTKEYS;
            }
            if s.raw_keyboard_flag_inputsink {
                desired_flags |= RAW_KEYBOARD_FLAG_INPUTSINK;
            }
        }
    });

    if desired_flags == FLAGS.load(Ordering::Acquire) {
        return Ok(());
    }

    let mut td = thread_data();

    // If the thread exited unexpectedly (e.g. MsgWaitForMultipleObjects failed),
    // the handle is stale. Clean it up so the creation path below can recover.
    if let Some(thread) = td.thread.as_ref() {
        // SAFETY: a live thread handle.
        if unsafe { WaitForSingleObject(thread.as_raw_handle(), 0) } != WAIT_TIMEOUT {
            cleanup_raw_input_thread_data(&mut td);
        }
    }

    // The thread will read from this to update its flags
    FLAGS.store(desired_flags, Ordering::Release);

    let result = if td.thread.is_some() {
        // Thread is already running. Fire (the event) and forget, it'll read the atomic flags on wakeup.

        // If RegisterRawInputDevices fails, the thread reverts the atomic and the next call
        // to this function will see the mismatch and retry.
        crate::sdl_assert!(td.signal_event.is_some());
        if let Some(signal) = td.signal_event {
            // SAFETY: a live event handle.
            unsafe { SetEvent(signal.0) };
        }
        Ok(())
    } else if desired_flags != 0 {
        // Thread isn't running, spin it up
        (|| -> Result<()> {
            let ready_event = create_event()?;
            td.ready_event = Some(ready_event);

            DONE.store(false, Ordering::Release);
            let signal_event = create_event()?;
            td.signal_event = Some(signal_event);

            let thread = std::thread::Builder::new()
                .name("SDLRawInput".into())
                .spawn(move || raw_input_thread(ready_event, signal_event))
                .map_err(|_| set_error("CreateThread"))?;
            let thread_handle = thread.as_raw_handle();
            td.thread = Some(thread);

            // Wait for the thread to complete initial setup or exit
            let wait_handles: [HANDLE; 2] = [ready_event.0, thread_handle];
            // SAFETY: both handles are live.
            if unsafe { WaitForMultipleObjects(2, wait_handles.as_ptr(), 0, INFINITE) }
                != WAIT_OBJECT_0
            {
                return Err(Error::new("Couldn't set up raw input handling"));
            }
            Ok(())
        })()
    } else {
        // Thread isn't running and we tried to disable raw input, nothing to do
        Ok(())
    };
    if result.is_err() {
        cleanup_raw_input_thread_data(&mut td);
    }
    result
}

/// Translation of `WIN_SetRawMouseEnabled()`.
pub(crate) fn set_raw_mouse_enabled(data: &VideoData, enabled: bool) -> Result<()> {
    data.state.with(|s| s.raw_mouse_enabled = enabled);
    let result = if super::gameinput::has_game_input(data) {
        super::gameinput::update_game_input_enabled(data)
    } else {
        update_raw_input_enabled(data)
    };
    if let Err(e) = result {
        data.state.with(|s| s.raw_mouse_enabled = !enabled);
        return Err(e);
    }
    Ok(())
}

/// Translation of `WIN_SetRawMouseFlag_NoLegacy()`.
pub(crate) fn set_raw_mouse_flag_no_legacy(data: &VideoData, enabled: bool) -> Result<()> {
    data.state.with(|s| s.raw_mouse_flag_nolegacy = enabled);

    update_raw_input_enabled(data)
}

/// Translation of `WIN_SetRawKeyboardEnabled()`.
pub(crate) fn set_raw_keyboard_enabled(data: &VideoData, enabled: bool) -> Result<()> {
    data.state.with(|s| s.raw_keyboard_enabled = enabled);
    let result = if super::gameinput::has_game_input(data) {
        super::gameinput::update_game_input_enabled(data)
    } else {
        update_raw_input_enabled(data)
    };
    if let Err(e) = result {
        data.state.with(|s| s.raw_keyboard_enabled = !enabled);
        return Err(e);
    }
    Ok(())
}

/// Translation of `WIN_SetRawKeyboardFlag_NoHotkeys()`.
pub(crate) fn set_raw_keyboard_flag_no_hotkeys(data: &VideoData, enabled: bool) -> Result<()> {
    data.state.with(|s| s.raw_keyboard_flag_nohotkeys = enabled);

    update_raw_input_enabled(data)
}

/// Translation of `WIN_SetRawKeyboardFlag_Inputsink()`.
pub(crate) fn set_raw_keyboard_flag_inputsink(data: &VideoData, enabled: bool) -> Result<()> {
    data.state.with(|s| s.raw_keyboard_flag_inputsink = enabled);

    update_raw_input_enabled(data)
}

/// Translation of `WIN_QuitRawInput()`.
pub(crate) fn quit_raw_input(_data: &VideoData) {
    cleanup_raw_input_thread_data(&mut thread_data());
}
