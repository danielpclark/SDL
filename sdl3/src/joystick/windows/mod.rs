// Rust translation of src/joystick/windows/SDL_windowsjoystick.c and
// SDL_windowsjoystick_c.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Windows joystick drivers.
//!
//! The Windows driver (`SDL_WINDOWS_JoystickDriver`, this file) combines
//! DirectInput ([`dinput`]) and XInput ([`xinput`]) devices; a thread with a
//! message-only window watches for HID devices coming and going (and
//! receives the RawInput driver's input), and polls XInput's slots when
//! device notifications don't work. The RawInput driver ([`rawinput`]) and
//! the Windows.Gaming.Input driver ([`windows_gaming_input`]) are drivers
//! of their own, before and after this one.
//!
//! The GameInput driver ([`gameinput`]) comes before them all; while it
//! handles the XInput controllers
//! (`SDL_UsingGameInputForXInputControllers()`), the others leave them
//! alone.

pub(crate) mod dinput;
pub(super) mod gameinput;
pub(super) mod rawinput;
mod wgi_abi;
pub(super) mod windows_gaming_input;
mod xinput;

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

use windows_sys::core::HRESULT;
use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HWND, LPARAM, LRESULT, S_OK, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, CreateWindowExW, DestroyWindow, DispatchMessageW, GetMessageW, KillTimer,
    PostThreadMessageW, RegisterClassExW, RegisterDeviceNotificationW, SetTimer, TranslateMessage,
    UnregisterClassW, UnregisterDeviceNotification, DBT_DEVICEARRIVAL, DBT_DEVICEREMOVECOMPLETE,
    DBT_DEVTYP_DEVICEINTERFACE, DEVICE_NOTIFY_WINDOW_HANDLE, DEV_BROADCAST_DEVICEINTERFACE_W,
    DEV_BROADCAST_HDR, HDEVNOTIFY, HWND_MESSAGE, MSG, WM_DEVICECHANGE, WM_QUIT, WM_TIMER,
    WNDCLASSEXW,
};

use super::gamepad::GamepadMapping;
use super::{
    assert_joysticks_locked, private_joystick_added, private_joystick_removed, send_joystick_axis,
    send_joystick_button, send_joystick_hat, with_joysticks_unlocked, Joystick, JoystickData,
    JoystickDriver,
};
use crate::core::windows::directx::{InputDevice, DIDEVCAPS, DIDEVICEINSTANCEW};
use crate::core::windows::hid::{
    get_last_device_notification, init_device_notification, quit_device_notification,
    GUID_DEVINTERFACE_HID,
};
use crate::core::windows::xinput::{xinput, XINPUT_FLAG_GAMEPAD, XUSER_MAX_COUNT};
use crate::core::windows::{co_initialize, co_uninitialize, set_error, utf8_to_wide};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::guid::Guid;
use crate::hints;
use crate::thread::{ReentrantMutex, Thread};

#[cfg(test)]
mod tests;

/// A device of the driver. Translation of `JoyStick_DeviceData`.
pub(super) struct JoyStickDeviceData {
    guid: Guid,
    joystickname: String,
    send_add_event: bool,
    n_instance_id: JoystickID,
    b_xinput_device: bool,
    sub_type: u8,
    xinput_user_id: u8,
    /// The DirectInput device instance (all zeros for XInput devices).
    dxdevice: DIDEVICEINSTANCEW,
    path: String,
    steam_virtual_gamepad_slot: i32,
}

impl JoyStickDeviceData {
    /// A copy of the device's data.
    fn duplicate(&self) -> JoyStickDeviceData {
        JoyStickDeviceData {
            guid: self.guid,
            joystickname: self.joystickname.clone(),
            send_add_event: self.send_add_event,
            n_instance_id: self.n_instance_id,
            b_xinput_device: self.b_xinput_device,
            sub_type: self.sub_type,
            xinput_user_id: self.xinput_user_id,
            dxdevice: self.dxdevice,
            path: self.path.clone(),
            steam_virtual_gamepad_slot: self.steam_virtual_gamepad_slot,
        }
    }
}

/// The private structure used to keep track of a joystick. Translation
/// of `struct joystick_hwdata`.
pub(super) struct HwData {
    /// The joystick this belongs to
    instance_id: JoystickID,
    #[allow(dead_code)] // (kept like upstream; nothing reads it)
    guid: Guid,

    /// The DirectInput half (`InputDevice`, `Capabilities` and the rest),
    /// for DirectInput devices.
    dinput: Option<dinput::DinputHwData>,

    /// true if this device supports using the xinput API rather than DirectInput
    b_xinput_device: bool,
    /// Supports force feedback via XInput.
    #[allow(dead_code)] // (kept like upstream; nothing reads it)
    b_xinput_haptic: bool,
    /// XInput userid index for this joystick
    userid: u8,
    dw_packet_number: u32,
}

/// The driver's device lists.
struct WindowsState {
    /// Translation of `SYS_Joystick` (most recently added first).
    sys_joystick: Vec<JoyStickDeviceData>,
    /// The open joysticks' `joystick->hwdata`.
    open: Vec<HwData>,
}

/// Guarded by the joystick lock upstream; the `RefCell` borrow is never
/// held across an event push or a call back into the joystick API.
static STATE: ReentrantMutex<RefCell<WindowsState>> =
    ReentrantMutex::new(RefCell::new(WindowsState {
        sys_joystick: Vec::new(),
        open: Vec::new(),
    }));

fn with_state<R>(f: impl FnOnce(&mut WindowsState) -> R) -> R {
    let guard = STATE.lock();
    let mut state = guard.borrow_mut();
    f(&mut state)
}

/// Translation of `s_bJoystickThread`.
static JOYSTICK_THREAD_ENABLED: AtomicBool = AtomicBool::new(false);
/// Translation of `s_bJoystickThreadQuit`, guarded by `s_mutexJoyStickEnum`.
static MUTEX_JOYSTICK_ENUM: Mutex<bool> = Mutex::new(false);
/// Translation of `s_condJoystickThread`.
static COND_JOYSTICK_THREAD: Condvar = Condvar::new();
/// Translation of `s_joystickThread`.
static JOYSTICK_THREAD: Mutex<Option<Thread>> = Mutex::new(None);
/// The Win32 ID of the joystick thread, for `PostThreadMessage()`
/// (`SDL_GetThreadID(s_joystickThread)`), once it runs.
static JOYSTICK_THREAD_ID: AtomicU32 = AtomicU32::new(0);
/// Translation of `s_lastDeviceChange`.
static LAST_DEVICE_CHANGE: AtomicU64 = AtomicU64::new(0);

fn lock_enum() -> MutexGuard<'static, bool> {
    MUTEX_JOYSTICK_ENUM
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Translation of `WindowsDeviceChanged()`.
fn windows_device_changed() -> bool {
    LAST_DEVICE_CHANGE.load(Ordering::Acquire) != get_last_device_notification()
}

/// Translation of `SetWindowsDeviceChanged()`.
fn set_windows_device_changed() {
    LAST_DEVICE_CHANGE.store(0, Ordering::Release);
}

/// Translation of `WINDOWS_RAWINPUTEnabledChanged()`.
pub(super) fn rawinput_enabled_changed() {
    set_windows_device_changed();
}

/// A directive of the `SDL_sscanf()` formats the Windows drivers read
/// Steam virtual gamepad slots with.
#[derive(Clone, Copy, Debug)]
pub(super) enum Scan<'a> {
    /// Literal text, which must match.
    Lit(&'a str),
    /// `%*X`: a hexadecimal number, skipped.
    SkipHex,
    /// `%*u`: an unsigned decimal number, skipped.
    SkipUnsigned,
    /// `%d`: the decimal number to return.
    Int,
}

/// Scan `text` like `SDL_sscanf(text, format, &value)` with a format of
/// one `%d` ([`Scan::Int`]): the value, if the scan got that far.
pub(super) fn scan_int(text: &str, format: &[Scan]) -> Option<i32> {
    let mut rest = text.as_bytes();
    let mut value = None;
    for directive in format {
        let (number, used) = match directive {
            Scan::Lit(lit) => match rest.strip_prefix(lit.as_bytes()) {
                Some(after) => {
                    rest = after;
                    continue;
                }
                None => break,
            },
            Scan::SkipHex => {
                let (number, used) = crate::stdlib::string::strtoul(rest, 16);
                (number as i64, used)
            }
            Scan::SkipUnsigned => {
                let (number, used) = crate::stdlib::string::strtoul(rest, 10);
                (number as i64, used)
            }
            Scan::Int => crate::stdlib::string::strtol(rest, 10),
        };
        if used == 0 {
            break;
        }
        rest = &rest[used..];
        if let Scan::Int = directive {
            value = Some(number as i32);
        }
    }
    value
}

/// Translation of `SDL_DeviceNotificationData`.
struct DeviceNotificationData {
    coinitialized: HRESULT,
    /// (`wincl.lpszClassName`)
    class_name: Vec<u16>,
    /// (`wincl.hInstance`)
    h_instance: windows_sys::Win32::Foundation::HINSTANCE,
    message_window: HWND,
    h_notify: HDEVNOTIFY,
}

// SAFETY: the handles are only used by one thread at a time (the joystick
// thread, or the joystick lock's holder).
unsafe impl Send for DeviceNotificationData {}

impl DeviceNotificationData {
    const fn new() -> DeviceNotificationData {
        DeviceNotificationData {
            coinitialized: 0,
            class_name: Vec::new(),
            h_instance: std::ptr::null_mut(),
            message_window: std::ptr::null_mut(),
            h_notify: std::ptr::null_mut(),
        }
    }
}

const IDT_SDL_DEVICE_CHANGE_TIMER_1: usize = 1200;
const IDT_SDL_DEVICE_CHANGE_TIMER_2: usize = 1201;

/// windowproc for our joystick detect thread message only window, to
/// detect any USB device addition/removal. Translation of
/// `SDL_PrivateJoystickDetectProc()`.
unsafe extern "system" fn private_joystick_detect_proc(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    match msg {
        WM_DEVICECHANGE => {
            if w_param == DBT_DEVICEARRIVAL as WPARAM
                || w_param == DBT_DEVICEREMOVECOMPLETE as WPARAM
            {
                // SAFETY: for these events lParam points to a DEV_BROADCAST_HDR.
                let devicetype =
                    unsafe { (*(l_param as *const DEV_BROADCAST_HDR)).dbch_devicetype };
                if devicetype == DBT_DEVTYP_DEVICEINTERFACE {
                    // notify 300ms and 2 seconds later to ensure all APIs have updated status
                    // SAFETY: hwnd is our window; no timer procedure is used.
                    unsafe {
                        SetTimer(hwnd, IDT_SDL_DEVICE_CHANGE_TIMER_1, 300, None);
                        SetTimer(hwnd, IDT_SDL_DEVICE_CHANGE_TIMER_2, 2000, None);
                    }
                }
            }
            return 1;
        }
        WM_TIMER
            if w_param == IDT_SDL_DEVICE_CHANGE_TIMER_1
                || w_param == IDT_SDL_DEVICE_CHANGE_TIMER_2 =>
        {
            // SAFETY: hwnd is our window and the timer is ours.
            unsafe {
                KillTimer(hwnd, w_param);
            }
            set_windows_device_changed();
            return 1;
        }
        _ => {}
    }

    // SAFETY: passing the message on to the RawInput window procedure,
    // which passes what it doesn't handle on to the default one.
    unsafe { CallWindowProcW(Some(rawinput::window_proc), hwnd, msg, w_param, l_param) }
}

/// Translation of `SDL_CleanupDeviceNotification()`.
fn cleanup_device_notification(data: &mut DeviceNotificationData) {
    let _ = rawinput::unregister_notifications();

    // SAFETY: the handles were created by create_device_notification() and
    // are released once, here.
    unsafe {
        if !data.h_notify.is_null() {
            UnregisterDeviceNotification(data.h_notify);
        }

        if !data.message_window.is_null() {
            DestroyWindow(data.message_window);
        }

        if !data.class_name.is_empty() {
            UnregisterClassW(data.class_name.as_ptr(), data.h_instance);
        }
    }

    if data.coinitialized == S_OK {
        co_uninitialize();
    }
    *data = DeviceNotificationData::new();
}

/// Translation of `SDL_CreateDeviceNotification()`.
fn create_device_notification(data: &mut DeviceNotificationData) -> Result<()> {
    *data = DeviceNotificationData::new();

    data.coinitialized = co_initialize();

    // SAFETY: GetModuleHandleW(NULL) is the executable's handle.
    data.h_instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    data.class_name = utf8_to_wide("Message");
    let wincl = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        lpfnWndProc: Some(private_joystick_detect_proc), // This function is called by windows
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: data.h_instance,
        hIcon: std::ptr::null_mut(),
        hCursor: std::ptr::null_mut(),
        hbrBackground: std::ptr::null_mut(),
        lpszMenuName: std::ptr::null(),
        lpszClassName: data.class_name.as_ptr(),
        hIconSm: std::ptr::null_mut(),
    };

    // SAFETY: wincl is filled in and its class name outlives the class.
    if unsafe { RegisterClassExW(&wincl) } == 0 {
        let error = set_error("Failed to create register class for joystick autodetect");
        cleanup_device_notification(data);
        return Err(error);
    }

    // SAFETY: the class is registered; a message-only window has no parent
    // or menu.
    data.message_window = unsafe {
        CreateWindowExW(
            0,
            data.class_name.as_ptr(),
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
    if data.message_window.is_null() {
        let error = set_error("Failed to create message window for joystick autodetect");
        cleanup_device_notification(data);
        return Err(error);
    }

    // SAFETY: an all-zero DEV_BROADCAST_DEVICEINTERFACE_W is valid.
    let mut dbh: DEV_BROADCAST_DEVICEINTERFACE_W = unsafe { std::mem::zeroed() };
    dbh.dbcc_size = size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32;
    dbh.dbcc_devicetype = DBT_DEVTYP_DEVICEINTERFACE;
    dbh.dbcc_classguid = GUID_DEVINTERFACE_HID;

    // SAFETY: the window is ours and dbh is a filled-in filter.
    data.h_notify = unsafe {
        RegisterDeviceNotificationW(
            data.message_window,
            (&dbh as *const DEV_BROADCAST_DEVICEINTERFACE_W).cast(),
            DEVICE_NOTIFY_WINDOW_HANDLE,
        )
    };
    if data.h_notify.is_null() {
        let error = set_error("Failed to create notify device for joystick autodetect");
        cleanup_device_notification(data);
        return Err(error);
    }

    let _ = rawinput::register_notifications(data.message_window);
    Ok(())
}

/// Translation of `SDL_WaitForDeviceNotification()`: runs the message
/// loop with the enumeration mutex unlocked, until a device change or
/// `WM_QUIT`. Returns the relocked mutex and whether the loop didn't fail.
fn wait_for_device_notification(
    message_window: HWND,
    mutex: MutexGuard<'static, bool>,
) -> (MutexGuard<'static, bool>, bool) {
    let mut lastret = 1;

    if message_window.is_null() {
        return (mutex, false); // device notifications require a window
    }

    drop(mutex);
    // SAFETY: an all-zero MSG is valid to be overwritten.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while lastret > 0 && !windows_device_changed() {
        // SAFETY: msg is writable; this thread owns the window.
        lastret = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) }; // WM_QUIT causes return value of 0
        if lastret > 0 {
            // SAFETY: msg was filled in by GetMessageW.
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    (lock_enum(), lastret != -1)
}

/// Translation of `s_notification_data`, for the device notification set
/// up without the joystick thread (the thread keeps its own).
static NOTIFICATION_DATA: Mutex<DeviceNotificationData> = Mutex::new(DeviceNotificationData::new());

/// Function/thread to scan the system for joysticks.
/// Translation of `SDL_JoystickThread()`.
fn joystick_thread() -> i32 {
    let mut b_opened_xinput_devices = [false; XUSER_MAX_COUNT as usize];

    // SAFETY: GetCurrentThreadId has no preconditions.
    JOYSTICK_THREAD_ID.store(unsafe { GetCurrentThreadId() }, Ordering::Release);

    let mut notification_data = DeviceNotificationData::new();
    if create_device_notification(&mut notification_data).is_err() {
        return 0;
    }

    let mut guard = lock_enum();
    while !*guard {
        let (relocked, ok) = wait_for_device_notification(notification_data.message_window, guard);
        guard = relocked;
        if !ok {
            // WM_DEVICECHANGE not working, poll for new XINPUT controllers
            guard = COND_JOYSTICK_THREAD
                .wait_timeout(guard, Duration::from_millis(1000))
                .unwrap_or_else(|e| e.into_inner())
                .0;
            if xinput::xinput_enabled() {
                if let Some(x) = xinput() {
                    // scan for any change in XInput devices
                    for (user_id, opened) in b_opened_xinput_devices.iter_mut().enumerate() {
                        let (result, _) = x.get_capabilities(user_id as u32, XINPUT_FLAG_GAMEPAD);
                        let available = result == ERROR_SUCCESS;
                        if *opened != available {
                            set_windows_device_changed();
                            *opened = available;
                        }
                    }
                }
            }
        }
        // A signalled device change makes the wait above return at once
        // until WINDOWS_JoystickDetect() takes it, so wait for that (or for
        // the thread to be stopped) instead of spinning
        while !*guard && windows_device_changed() {
            guard = COND_JOYSTICK_THREAD
                .wait(guard)
                .unwrap_or_else(|e| e.into_inner());
        }
    }

    drop(guard);

    cleanup_device_notification(&mut notification_data);

    1
}

/// spin up the thread to detect hotplug of devices.
/// Translation of `SDL_StartJoystickThread()`.
fn start_joystick_thread() -> Result<()> {
    *lock_enum() = false;
    JOYSTICK_THREAD_ID.store(0, Ordering::Release);
    let thread = Thread::spawn("SDL_joystick", joystick_thread)?;
    *JOYSTICK_THREAD.lock().unwrap_or_else(|e| e.into_inner()) = Some(thread);
    Ok(())
}

/// Translation of `SDL_StopJoystickThread()`.
fn stop_joystick_thread() {
    let Some(thread) = JOYSTICK_THREAD
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
    else {
        return;
    };

    {
        let mut quit = lock_enum();
        *quit = true;
        COND_JOYSTICK_THREAD.notify_all(); // signal the joystick thread to quit
    }
    // (the thread posts its ID as soon as it runs; until then it can't have
    // a message queue, and sees the quit flag before waiting)
    let thread_id = JOYSTICK_THREAD_ID.load(Ordering::Acquire);
    if thread_id != 0 {
        // SAFETY: posting a message to a thread has no memory preconditions.
        unsafe {
            PostThreadMessageW(thread_id, WM_QUIT, 0, 0);
        }
    }

    // Unlock joysticks while the joystick thread finishes processing messages
    // (the RawInput window procedure takes the lock on that thread)
    assert_joysticks_locked();
    with_joysticks_unlocked(|| thread.wait()); // wait for it to bugger off
}

/// Translation of `WINDOWS_AddJoystickDevice()`.
fn add_joystick_device(sys_joystick: &mut Vec<JoyStickDeviceData>, mut device: JoyStickDeviceData) {
    device.send_add_event = true;
    device.n_instance_id = crate::utils::next_object_id();
    sys_joystick.insert(0, device);
}

/// detect any new joysticks being inserted into the system.
/// Translation of `WINDOWS_JoystickDetect()`.
pub(super) fn joystick_detect() {
    // only enum the devices if the joystick thread told us something changed
    if !windows_device_changed() {
        return; // thread hasn't signaled, nothing to do right now.
    }

    let (removed, added) = {
        let guard = lock_enum();

        LAST_DEVICE_CHANGE.store(get_last_device_notification(), Ordering::Release);
        // the joystick thread waits for the change to be taken
        COND_JOYSTICK_THREAD.notify_all();

        let mut cur_list = with_state(|s| std::mem::take(&mut s.sys_joystick));
        let mut sys_joystick = Vec::new();

        // Look for DirectInput joysticks, wheels, head trackers, gamepads, etc..
        dinput::joystick_detect(&mut cur_list, &mut sys_joystick);

        // Look for XInput devices. Do this last, so they're first in the final list.
        xinput::joystick_detect(&mut cur_list, &mut sys_joystick);

        drop(guard);

        // (the devices left in the previous list are gone)
        let removed: Vec<(JoystickID, Option<DIDEVICEINSTANCEW>)> = cur_list
            .iter()
            .map(|d| (d.n_instance_id, (!d.b_xinput_device).then_some(d.dxdevice)))
            .collect();
        let added = with_state(|s| {
            s.sys_joystick = sys_joystick;
            s.sys_joystick
                .iter_mut()
                .filter(|d| d.send_add_event)
                .map(|d| {
                    d.send_add_event = false;
                    (d.n_instance_id, (!d.b_xinput_device).then_some(d.dxdevice))
                })
                .collect::<Vec<_>>()
        });
        (removed, added)
    };

    for (instance_id, dxdevice) in removed {
        if let Some(dxdevice) = dxdevice {
            crate::haptic::windows::dinput_haptic_maybe_remove_device(&dxdevice);
        }
        private_joystick_removed(instance_id);
    }

    for (instance_id, dxdevice) in added {
        if let Some(dxdevice) = dxdevice {
            crate::haptic::windows::dinput_haptic_maybe_add_device(&dxdevice);
        }
        private_joystick_added(instance_id);
    }
}

/// Run `f` on the device at `device_index`.
fn with_device<R>(device_index: usize, f: impl FnOnce(&JoyStickDeviceData) -> R) -> Option<R> {
    with_state(|s| s.sys_joystick.get(device_index).map(f))
}

/// Run `f` on the device of an open joystick (`joystick->hwdata`).
fn with_hwdata<R>(instance_id: JoystickID, f: impl FnOnce(&mut HwData) -> R) -> Option<R> {
    with_state(|s| {
        s.open
            .iter_mut()
            .find(|h| h.instance_id == instance_id)
            .map(f)
    })
}

/// The device instances of the driver's devices (`device->dxdevice` of
/// each `SYS_Joystick` entry), for the haptic driver's initialization.
pub(crate) fn device_instances() -> Vec<DIDEVICEINSTANCEW> {
    with_state(|s| s.sys_joystick.iter().map(|d| d.dxdevice).collect())
}

/// The DirectInput device and capabilities of an open joystick of this
/// driver (`joystick->hwdata->InputDevice` and `Capabilities`), for the
/// haptic driver: `None` for a joystick of another driver (`joystick->driver
/// != &SDL_WINDOWS_JoystickDriver`), no device (and zeroed capabilities)
/// for an XInput joystick.
pub(crate) fn joystick_dinput_device(
    joystick: &Joystick,
) -> Option<(Option<InputDevice>, DIDEVCAPS)> {
    assert_joysticks_locked();

    let (instance_id, driver) = joystick.with(|j| (j.instance_id, j.driver)).ok()?;
    if driver != super::WINDOWS_DRIVER_INDEX {
        return None;
    }
    with_hwdata(instance_id, |h| match &h.dinput {
        Some(d) => (Some(d.input_device.clone()), d.capabilities),
        None => (None, DIDEVCAPS::default()),
    })
}

/// Send input events of a joystick.
fn send_inputs(joystick: JoystickID, inputs: Vec<xinput::Input>) {
    let timestamp = crate::timer::ticks_ns();
    for input in inputs {
        match input {
            xinput::Input::Axis(axis, value) => {
                send_joystick_axis(timestamp, joystick, axis, value)
            }
            xinput::Input::Button(button, down) => {
                send_joystick_button(timestamp, joystick, button, down)
            }
            xinput::Input::Hat(hat, value) => send_joystick_hat(timestamp, joystick, hat, value),
        }
    }
}

/// Translation of `SDL_WINDOWS_JoystickDriver`.
pub(super) struct WindowsJoystickDriver;

pub(super) static WINDOWS_JOYSTICK_DRIVER: WindowsJoystickDriver = WindowsJoystickDriver;

impl JoystickDriver for WindowsJoystickDriver {
    /// Translation of `WINDOWS_JoystickInit()`.
    fn init(&self) -> Result<()> {
        if !xinput::joystick_init() {
            self.quit();
            return Err(Error::new("XInput initialization failed"));
        }

        if let Err(e) = dinput::joystick_init() {
            self.quit();
            return Err(e);
        }

        init_device_notification();

        let thread = hints::get_bool(hints::JOYSTICK_THREAD, true);
        JOYSTICK_THREAD_ENABLED.store(thread, Ordering::Relaxed);
        if thread {
            start_joystick_thread()?;
        } else {
            create_device_notification(
                &mut NOTIFICATION_DATA.lock().unwrap_or_else(|e| e.into_inner()),
            )?;
        }

        set_windows_device_changed(); // force a scan of the system for joysticks this first time

        joystick_detect();

        Ok(())
    }

    /// return the number of joysticks that are connected right now.
    /// Translation of `WINDOWS_JoystickGetCount()`.
    fn count(&self) -> usize {
        with_state(|s| s.sys_joystick.len())
    }

    /// detect any new joysticks being inserted into the system.
    fn detect(&self) {
        joystick_detect();
    }

    /// Translation of `WINDOWS_JoystickIsDevicePresent()`.
    fn is_device_present(
        &self,
        vendor_id: u16,
        product_id: u16,
        version: u16,
        _name: Option<&str>,
    ) -> bool {
        if dinput::joystick_present(vendor_id, product_id, version) {
            return true;
        }
        if xinput::joystick_present(vendor_id, product_id, version) {
            return true;
        }
        false
    }

    /// Translation of `WINDOWS_JoystickGetDeviceName()`.
    fn device_name(&self, device_index: usize) -> Option<String> {
        with_device(device_index, |d| d.joystickname.clone())
    }

    /// Translation of `WINDOWS_JoystickGetDevicePath()`.
    fn device_path(&self, device_index: usize) -> Option<String> {
        with_device(device_index, |d| d.path.clone())
    }

    /// Translation of `WINDOWS_JoystickGetDeviceSteamVirtualGamepadSlot()`.
    fn device_steam_virtual_gamepad_slot(&self, device_index: usize) -> i32 {
        let Some((b_xinput_device, userid, slot)) = with_device(device_index, |d| {
            (
                d.b_xinput_device,
                d.xinput_user_id,
                d.steam_virtual_gamepad_slot,
            )
        }) else {
            return -1;
        };

        if b_xinput_device {
            // The slot for XInput devices can change as controllers are seated
            return xinput::get_steam_virtual_gamepad_slot(userid);
        }
        slot
    }

    /// Translation of `WINDOWS_JoystickGetDevicePlayerIndex()`.
    fn device_player_index(&self, device_index: usize) -> i32 {
        with_device(device_index, |d| {
            if d.b_xinput_device {
                d.xinput_user_id as i32
            } else {
                -1
            }
        })
        .unwrap_or(-1)
    }

    /// Translation of `WINDOWS_JoystickSetDevicePlayerIndex()`.
    fn set_device_player_index(&self, _device_index: usize, _player_index: i32) {}

    /// return the stable device guid for this device index.
    /// Translation of `WINDOWS_JoystickGetDeviceGUID()`.
    fn device_guid(&self, device_index: usize) -> Guid {
        with_device(device_index, |d| d.guid).unwrap_or(Guid::ZERO)
    }

    /// Function to perform the mapping between current device instance and
    /// this joysticks instance id. Translation of
    /// `WINDOWS_JoystickGetDeviceInstanceID()`.
    fn device_instance_id(&self, device_index: usize) -> JoystickID {
        with_device(device_index, |d| d.n_instance_id).unwrap_or(0)
    }

    /// Function to open a joystick for use. This should fill the nbuttons
    /// and naxes fields of the joystick structure.
    /// Translation of `WINDOWS_JoystickOpen()`.
    fn open(&self, joystick: &mut JoystickData, device_index: usize) -> Result<()> {
        let Some(device) = with_device(device_index, JoyStickDeviceData::duplicate) else {
            return Err(Error::new("No such device"));
        };

        // allocate memory for system specific hardware data
        let mut hwdata = HwData {
            instance_id: joystick.instance_id,
            guid: device.guid,
            dinput: None,
            b_xinput_device: false,
            b_xinput_haptic: false,
            userid: 0,
            dw_packet_number: 0,
        };

        if device.b_xinput_device {
            xinput::joystick_open(joystick, &device, &mut hwdata)?;
        } else {
            hwdata.dinput = Some(dinput::joystick_open(joystick, &device)?);
        }
        with_state(|s| s.open.push(hwdata));
        Ok(())
    }

    /// Translation of `WINDOWS_JoystickRumble()`.
    fn rumble(
        &self,
        joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        let Some(result) = with_hwdata(joystick, |hwdata| {
            if hwdata.b_xinput_device {
                return xinput::joystick_rumble(
                    hwdata,
                    low_frequency_rumble,
                    high_frequency_rumble,
                );
            }
            match &mut hwdata.dinput {
                Some(dinput) => {
                    dinput::joystick_rumble(dinput, low_frequency_rumble, high_frequency_rumble)
                }
                None => Err(Error::unsupported()),
            }
        }) else {
            return Err(Error::invalid_param("joystick"));
        };
        result
    }

    /// Translation of `WINDOWS_JoystickRumbleTriggers()`.
    fn rumble_triggers(
        &self,
        _joystick: JoystickID,
        _left_rumble: u16,
        _right_rumble: u16,
    ) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `WINDOWS_JoystickSetLED()`.
    fn set_led(&self, _joystick: JoystickID, _red: u8, _green: u8, _blue: u8) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `WINDOWS_JoystickSendEffect()`.
    fn send_effect(&self, _joystick: JoystickID, _data: &[u8]) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `WINDOWS_JoystickSetSensorsEnabled()`.
    fn set_sensors_enabled(&self, _joystick: JoystickID, _enabled: bool) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `WINDOWS_JoystickUpdate()`.
    fn update(&self, joystick: JoystickID) {
        let Some((b_xinput_device, userid, dw_packet_number)) = with_hwdata(joystick, |h| {
            (h.b_xinput_device, h.userid, h.dw_packet_number)
        }) else {
            return;
        };

        if b_xinput_device {
            // (the events are sent without the device borrowed)
            let dw_packet_number = xinput::joystick_update(joystick, userid, dw_packet_number);
            with_hwdata(joystick, |h| h.dw_packet_number = dw_packet_number);
            return;
        }
        let inputs = with_hwdata(joystick, |h| h.dinput.as_mut().map(dinput::joystick_update));
        if let Some(Some(inputs)) = inputs {
            send_inputs(joystick, inputs);
        }
    }

    /// Function to close a joystick after use.
    /// Translation of `WINDOWS_JoystickClose()`.
    fn close(&self, joystick: &mut JoystickData) {
        let hwdata = with_state(|s| {
            let i = s
                .open
                .iter()
                .position(|h| h.instance_id == joystick.instance_id)?;
            Some(s.open.remove(i))
        });
        let Some(hwdata) = hwdata else {
            return;
        };
        if hwdata.b_xinput_device {
            xinput::joystick_close();
        } else if let Some(dinput) = hwdata.dinput {
            dinput::joystick_close(dinput);
        }
    }

    /// Function to perform any system-specific joystick related cleanup.
    /// Translation of `WINDOWS_JoystickQuit()`.
    fn quit(&self) {
        with_state(|s| s.sys_joystick.clear());

        if JOYSTICK_THREAD_ENABLED.load(Ordering::Relaxed) {
            stop_joystick_thread();
        } else {
            cleanup_device_notification(
                &mut NOTIFICATION_DATA.lock().unwrap_or_else(|e| e.into_inner()),
            );
        }

        dinput::joystick_quit();
        xinput::joystick_quit();

        quit_device_notification();
    }

    /// Translation of `WINDOWS_JoystickGetGamepadMapping()`.
    fn gamepad_mapping(&self, _device_index: usize) -> Option<GamepadMapping> {
        None
    }
}
