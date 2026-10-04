// Rust translation of src/core/windows/SDL_hid.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `hid.dll`, loaded at run time ([`load_hid_dll`]), and device arrival
//! and removal notifications, through `CM_Register_Notification()`
//! (loaded from `cfgmgr32.dll` at run time): [`get_last_device_notification`]
//! changes when a HID interface comes or goes, and the hotplug thread
//! rechecks the keyboards and mice of the Windows video driver.

#![allow(non_camel_case_types, non_snake_case)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::Mutex;

use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Threading::{
    CreateEventW, SetEvent, WaitForSingleObject, INFINITE,
};

use crate::loadso::SharedObject;
use crate::thread::Thread;

/// `HidD_GetString_t`
type HidDGetStringFn = unsafe extern "system" fn(HANDLE, *mut c_void, u32) -> u8;

/// The loaded `hid.dll` and the functions SDL uses from it (`s_pHIDDLL`
/// and the `SDL_HidD_*` pointers).
struct HidDll {
    /// Translation of `s_pHIDDLL`.
    _lib: SharedObject,
    /// Translation of `s_HIDDLLRefCount`.
    ref_count: i32,
    get_manufacturer_string: HidDGetStringFn,
    get_product_string: HidDGetStringFn,
}

static HID: Mutex<Option<HidDll>> = Mutex::new(None);

fn hid() -> std::sync::MutexGuard<'static, Option<HidDll>> {
    HID.lock().unwrap_or_else(|e| e.into_inner())
}

/// Load (or take another reference to) `hid.dll`; `false` if it or one of
/// the functions SDL needs is missing. Translation of `WIN_LoadHIDDLL()`.
pub(crate) fn load_hid_dll() -> bool {
    let mut h = hid();
    if let Some(dll) = h.as_mut() {
        crate::sdl_assert!(dll.ref_count > 0);
        dll.ref_count += 1;
        return true; // already loaded
    }

    let Ok(lib) = SharedObject::load("hid.dll") else {
        return false;
    };
    // SAFETY: both functions have the HidD_GetString_t prototype.
    let manufacturer = unsafe { lib.function::<HidDGetStringFn>("HidD_GetManufacturerString") };
    // SAFETY: as above.
    let product = unsafe { lib.function::<HidDGetStringFn>("HidD_GetProductString") };
    // (the others are used by the RawInput joystick driver)
    let others = [
        "HidD_GetAttributes",
        "HidP_GetCaps",
        "HidP_GetButtonCaps",
        "HidP_GetValueCaps",
        "HidP_MaxDataListLength",
        "HidP_GetData",
    ];
    let (Ok(get_manufacturer_string), Ok(get_product_string)) = (manufacturer, product) else {
        return false;
    };
    if others.iter().any(|name| lib.symbol(name).is_err()) {
        return false;
    }
    *h = Some(HidDll {
        _lib: lib,
        ref_count: 1,
        get_manufacturer_string,
        get_product_string,
    });
    true
}

/// Release a reference taken by [`load_hid_dll`]. Translation of
/// `WIN_UnloadHIDDLL()`.
pub(crate) fn unload_hid_dll() {
    let mut h = hid();
    if let Some(dll) = h.as_mut() {
        crate::sdl_assert!(dll.ref_count > 0);
        dll.ref_count -= 1;
        if dll.ref_count == 0 {
            *h = None;
        }
    }
}

/// `SDL_HidD_GetManufacturerString(handle, buf, sizeof(buf))`; `false` if
/// it fails or `hid.dll` isn't loaded.
pub(crate) fn hidd_get_manufacturer_string(handle: HANDLE, buf: &mut [u16]) -> bool {
    let f = hid().as_ref().map(|dll| dll.get_manufacturer_string);
    // SAFETY: the buffer holds the size passed.
    f.is_some_and(|f| unsafe { f(handle, buf.as_mut_ptr().cast(), size_of_val(buf) as u32) != 0 })
}

/// `SDL_HidD_GetProductString(handle, buf, sizeof(buf))`; `false` if it
/// fails or `hid.dll` isn't loaded.
pub(crate) fn hidd_get_product_string(handle: HANDLE, buf: &mut [u16]) -> bool {
    let f = hid().as_ref().map(|dll| dll.get_product_string);
    // SAFETY: the buffer holds the size passed.
    f.is_some_and(|f| unsafe { f(handle, buf.as_mut_ptr().cast(), size_of_val(buf) as u32) != 0 })
}

// CM_Register_Notification definitions

const CR_SUCCESS: u32 = 0;

/// `HCMNOTIFICATION`
type HCMNOTIFICATION = *mut c_void;

/// `CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE`
const CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE: i32 = 0;

/// The filter of a `CM_NOTIFY_FILTER` (its union `u`).
#[repr(C)]
#[derive(Clone, Copy)]
union CM_NOTIFY_FILTER_U {
    /// `DeviceInterface.ClassGuid`
    ClassGuid: GUID,
    /// `DeviceHandle.hTarget`
    hTarget: HANDLE,
    /// `DeviceInstance.InstanceId`
    InstanceId: [u16; 200],
}

/// `CM_NOTIFY_FILTER`
#[repr(C)]
#[derive(Clone, Copy)]
struct CM_NOTIFY_FILTER {
    cbSize: u32,
    Flags: u32,
    FilterType: i32,
    Reserved: u32,
    u: CM_NOTIFY_FILTER_U,
}

/// `CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL`
const CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL: i32 = 0;
/// `CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL`
const CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL: i32 = 1;

/// `PCM_NOTIFY_CALLBACK` (the event data is not read)
type PCM_NOTIFY_CALLBACK = unsafe extern "system" fn(
    hNotify: HCMNOTIFICATION,
    Context: *mut c_void,
    Action: i32,
    EventData: *const c_void,
    EventDataSize: u32,
) -> u32;

/// `CM_Register_NotificationFunc`
type CM_Register_NotificationFunc = unsafe extern "system" fn(
    pFilter: *const CM_NOTIFY_FILTER,
    pContext: *mut c_void,
    pCallback: PCM_NOTIFY_CALLBACK,
    pNotifyContext: *mut HCMNOTIFICATION,
) -> u32;
/// `CM_Unregister_NotificationFunc`
type CM_Unregister_NotificationFunc =
    unsafe extern "system" fn(NotifyContext: HCMNOTIFICATION) -> u32;

/// `GUID_DEVINTERFACE_HID`
pub(crate) const GUID_DEVINTERFACE_HID: GUID = GUID {
    data1: 0x4D1E55B2,
    data2: 0xF16F,
    data3: 0x11CF,
    data4: [0x88, 0xCB, 0x00, 0x11, 0x11, 0x00, 0x00, 0x30],
};

/// The state `WIN_InitDeviceNotification()` sets up.
struct DeviceNotification {
    /// Translation of `s_DeviceNotificationsRequested`.
    requested: i32,
    /// Translation of `cfgmgr32_lib_handle`.
    cfgmgr32_lib_handle: Option<SharedObject>,
    /// Translation of `CM_Unregister_Notification`.
    cm_unregister_notification: Option<CM_Unregister_NotificationFunc>,
    /// Translation of `s_DeviceNotificationFuncHandle`.
    device_notification_func_handle: HCMNOTIFICATION,
    /// Translation of `s_HotplugThread`.
    hotplug_thread: Option<Thread>,
}

// SAFETY: the notification handle is an opaque token used only with the
// mutex held.
unsafe impl Send for DeviceNotification {}

static STATE: Mutex<DeviceNotification> = Mutex::new(DeviceNotification {
    requested: 0,
    cfgmgr32_lib_handle: None,
    cm_unregister_notification: None,
    device_notification_func_handle: std::ptr::null_mut(),
    hotplug_thread: None,
});

/// Translation of `s_LastDeviceNotification`.
static LAST_DEVICE_NOTIFICATION: AtomicU64 = AtomicU64::new(1);
/// Translation of `s_HotplugEvent` (null for `INVALID_HANDLE_VALUE`).
static HOTPLUG_EVENT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
/// Translation of `s_HotplugRunning`.
static HOTPLUG_RUNNING: AtomicBool = AtomicBool::new(false);

/// Translation of `DeviceHotplugThread()`.
fn device_hotplug_thread() -> i32 {
    let hid_loaded = load_hid_dll();

    // Always run the initial device detection
    loop {
        crate::video::drivers::windows::events::check_keyboard_and_mouse_hotplug(hid_loaded);
        // SAFETY: the event handle stays open until this thread is joined.
        unsafe {
            WaitForSingleObject(HOTPLUG_EVENT.load(Ordering::Acquire), INFINITE);
        }
        if !HOTPLUG_RUNNING.load(Ordering::Acquire) {
            break;
        }
    }

    if hid_loaded {
        unload_hid_dll();
    }
    0
}

/// Translation of `SDL_DeviceNotificationFunc()`.
unsafe extern "system" fn device_notification_func(
    _h_notify: HCMNOTIFICATION,
    _context: *mut c_void,
    action: i32,
    _event_data: *const c_void,
    _event_data_size: u32,
) -> u32 {
    if action == CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL
        || action == CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL
    {
        LAST_DEVICE_NOTIFICATION.store(crate::timer::ticks_ns(), Ordering::Release);
        // SAFETY: the event handle stays open while notifications are
        // registered.
        unsafe {
            SetEvent(HOTPLUG_EVENT.load(Ordering::Acquire));
        }
    }
    0 // ERROR_SUCCESS
}

/// Start watching for HID devices (counted: each call needs a
/// [`quit_device_notification`]). Translation of `WIN_InitDeviceNotification()`.
pub(crate) fn init_device_notification() {
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    s.requested += 1;
    if s.requested > 1 {
        return;
    }

    // Start the device hotplug thread
    HOTPLUG_RUNNING.store(true, Ordering::Release);
    // SAFETY: an unnamed auto-reset event with default security.
    let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
    HOTPLUG_EVENT.store(event, Ordering::Release);
    s.hotplug_thread = Thread::spawn("DeviceHotplugThread", device_hotplug_thread).ok();

    if let Ok(lib) = SharedObject::load("cfgmgr32.dll") {
        // SAFETY: the symbols are loaded with their cfgmgr32 prototypes.
        let register =
            unsafe { lib.function::<CM_Register_NotificationFunc>("CM_Register_Notification") };
        // SAFETY: as above.
        let unregister =
            unsafe { lib.function::<CM_Unregister_NotificationFunc>("CM_Unregister_Notification") };
        s.cm_unregister_notification = unregister.ok();
        s.cfgmgr32_lib_handle = Some(lib);
        if let (Ok(register), Some(_)) = (register, s.cm_unregister_notification) {
            let notify_filter = CM_NOTIFY_FILTER {
                cbSize: size_of::<CM_NOTIFY_FILTER>() as u32,
                Flags: 0,
                FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE,
                Reserved: 0,
                u: CM_NOTIFY_FILTER_U {
                    ClassGuid: GUID_DEVINTERFACE_HID,
                },
            };
            let mut handle: HCMNOTIFICATION = std::ptr::null_mut();
            // SAFETY: the filter and the handle are valid for the call; the
            // callback stays valid while the DLL is loaded.
            if unsafe {
                register(
                    &notify_filter,
                    std::ptr::null_mut(),
                    device_notification_func,
                    &mut handle,
                )
            } == CR_SUCCESS
            {
                s.device_notification_func_handle = handle;
            }
        }
    }

    // FIXME: Should we log errors?
}

/// The time (`ticks_ns()`) of the last device arrival or removal (1 until
/// there is one). Translation of `WIN_GetLastDeviceNotification()`.
pub(crate) fn get_last_device_notification() -> u64 {
    LAST_DEVICE_NOTIFICATION.load(Ordering::Acquire)
}

/// Release a reference taken by [`init_device_notification`].
/// Translation of `WIN_QuitDeviceNotification()`.
pub(crate) fn quit_device_notification() {
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    s.requested -= 1;
    if s.requested > 0 {
        return;
    }
    // Make sure we have balanced calls to init/quit
    crate::sdl_assert!(s.requested == 0);

    // Stop the device hotplug thread
    HOTPLUG_RUNNING.store(false, Ordering::Release);
    let event = HOTPLUG_EVENT.load(Ordering::Acquire);
    // SAFETY: the event was created by init_device_notification().
    unsafe {
        SetEvent(event);
    }
    if let Some(thread) = s.hotplug_thread.take() {
        thread.wait();
    }

    if s.cfgmgr32_lib_handle.is_some() {
        if let Some(unregister) = s.cm_unregister_notification {
            if !s.device_notification_func_handle.is_null() {
                // SAFETY: the handle came from CM_Register_Notification.
                unsafe {
                    unregister(s.device_notification_func_handle);
                }
                s.device_notification_func_handle = std::ptr::null_mut();
            }
        }

        s.cm_unregister_notification = None;
        s.cfgmgr32_lib_handle = None;
    }

    // FIXME (upstream): s_HotplugEvent is never closed, so every init/quit
    // cycle leaks an event handle.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_start_and_stop() {
        let _l = crate::test_support::test_lock();
        assert_eq!(size_of::<CM_NOTIFY_FILTER>(), 416);
        init_device_notification();
        init_device_notification();
        assert!(get_last_device_notification() >= 1);
        quit_device_notification();
        assert!(STATE.lock().unwrap().hotplug_thread.is_some());
        quit_device_notification();
        assert!(STATE.lock().unwrap().hotplug_thread.is_none());
    }
}
