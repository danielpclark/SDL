// Rust translation of src/core/windows/SDL_hid.c and SDL_hid.h from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The HID parsing functions of `hid.dll` (`WIN_LoadHIDDLL()`, the
//! `SDL_HidD_*` and `SDL_HidP_*` function pointers, loaded at run time and
//! reference counted) for the RawInput joystick driver, and device arrival
//! and removal notifications, through `CM_Register_Notification()` (loaded
//! from `cfgmgr32.dll` at run time), for the joystick drivers:
//! [`get_last_device_notification`] changes when a HID interface comes or
//! goes, and the hotplug thread rechecks the keyboards and mice of the
//! Windows video driver.

#![allow(non_camel_case_types, non_snake_case)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::Mutex;

use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::Threading::{
    CreateEventW, SetEvent, WaitForSingleObject, INFINITE,
};

use crate::loadso::SharedObject;
use crate::thread::Thread;

// The hid.dll part (SDL_hid.h)

/// `NTSTATUS`
pub(crate) type NTSTATUS = i32;

/// The opaque preparsed data of a top-level collection
/// (`PHIDP_PREPARSED_DATA`); SDL keeps it as the bytes
/// `GetRawInputDeviceInfo(RIDI_PREPARSEDDATA)` returns.
type PHIDP_PREPARSED_DATA = *mut c_void;

/// `HIDP_REPORT_TYPE`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub(crate) enum HIDP_REPORT_TYPE {
    /// `HidP_Input`
    Input = 0,
    /// `HidP_Output`
    #[allow(dead_code)] // (part of the enumeration)
    Output = 1,
    /// `HidP_Feature`
    #[allow(dead_code)] // (part of the enumeration)
    Feature = 2,
}

/// The `Range` and `NotRange` union of the capability structures: the two
/// share a layout, `NotRange.Usage` being `UsageMin` and
/// `NotRange.DataIndex` being `DataIndexMin`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HIDP_RANGE {
    pub(crate) UsageMin: u16,
    pub(crate) UsageMax: u16,
    pub(crate) StringMin: u16,
    pub(crate) StringMax: u16,
    pub(crate) DesignatorMin: u16,
    pub(crate) DesignatorMax: u16,
    pub(crate) DataIndexMin: u16,
    pub(crate) DataIndexMax: u16,
}

impl HIDP_RANGE {
    /// `NotRange.Usage`
    pub(crate) fn usage(&self) -> u16 {
        self.UsageMin
    }

    /// `NotRange.DataIndex`
    pub(crate) fn data_index(&self) -> u16 {
        self.DataIndexMin
    }
}

/// `HIDP_BUTTON_CAPS`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HIDP_BUTTON_CAPS {
    pub(crate) UsagePage: u16,
    pub(crate) ReportID: u8,
    pub(crate) IsAlias: u8,
    pub(crate) BitField: u16,
    pub(crate) LinkCollection: u16,
    pub(crate) LinkUsage: u16,
    pub(crate) LinkUsagePage: u16,
    pub(crate) IsRange: u8,
    pub(crate) IsStringRange: u8,
    pub(crate) IsDesignatorRange: u8,
    pub(crate) IsAbsolute: u8,
    pub(crate) Reserved: [u32; 10],
    pub(crate) u: HIDP_RANGE,
}

/// `HIDP_VALUE_CAPS`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HIDP_VALUE_CAPS {
    pub(crate) UsagePage: u16,
    pub(crate) ReportID: u8,
    pub(crate) IsAlias: u8,
    pub(crate) BitField: u16,
    pub(crate) LinkCollection: u16,
    pub(crate) LinkUsage: u16,
    pub(crate) LinkUsagePage: u16,
    pub(crate) IsRange: u8,
    pub(crate) IsStringRange: u8,
    pub(crate) IsDesignatorRange: u8,
    pub(crate) IsAbsolute: u8,
    pub(crate) HasNull: u8,
    pub(crate) Reserved: u8,
    pub(crate) BitSize: u16,
    pub(crate) ReportCount: u16,
    pub(crate) Reserved2: [u16; 5],
    pub(crate) UnitsExp: u32,
    pub(crate) Units: u32,
    pub(crate) LogicalMin: i32,
    pub(crate) LogicalMax: i32,
    pub(crate) PhysicalMin: i32,
    pub(crate) PhysicalMax: i32,
    pub(crate) u: HIDP_RANGE,
}

/// `HIDP_CAPS`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HIDP_CAPS {
    pub(crate) Usage: u16,
    pub(crate) UsagePage: u16,
    pub(crate) InputReportByteLength: u16,
    pub(crate) OutputReportByteLength: u16,
    pub(crate) FeatureReportByteLength: u16,
    pub(crate) Reserved: [u16; 17],
    pub(crate) NumberLinkCollectionNodes: u16,
    pub(crate) NumberInputButtonCaps: u16,
    pub(crate) NumberInputValueCaps: u16,
    pub(crate) NumberInputDataIndices: u16,
    pub(crate) NumberOutputButtonCaps: u16,
    pub(crate) NumberOutputValueCaps: u16,
    pub(crate) NumberOutputDataIndices: u16,
    pub(crate) NumberFeatureButtonCaps: u16,
    pub(crate) NumberFeatureValueCaps: u16,
    pub(crate) NumberFeatureDataIndices: u16,
}

/// `HIDP_DATA`; its union of `RawValue` and `On` is `RawValue`, `On`
/// being its low byte.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HIDP_DATA {
    pub(crate) DataIndex: u16,
    pub(crate) Reserved: u16,
    pub(crate) RawValue: u32,
}

impl HIDP_DATA {
    /// `On`
    pub(crate) fn on(&self) -> bool {
        self.RawValue.to_le_bytes()[0] != 0
    }
}

/// `HIDP_ERROR_CODES(p1, p2)`
const fn hidp_error_codes(p1: u32, p2: u32) -> NTSTATUS {
    ((p1 << 28) | (0x11 << 16) | p2) as NTSTATUS
}
/// `HIDP_STATUS_SUCCESS`
pub(crate) const HIDP_STATUS_SUCCESS: NTSTATUS = hidp_error_codes(0x0, 0x0000);

/// `HidD_GetString_t`
type HidD_GetString_t = unsafe extern "system" fn(
    HidDeviceObject: HANDLE,
    Buffer: *mut c_void,
    BufferLength: u32,
) -> u8;
/// `HidP_GetCaps_t`
type HidP_GetCaps_t = unsafe extern "system" fn(
    PreparsedData: PHIDP_PREPARSED_DATA,
    Capabilities: *mut HIDP_CAPS,
) -> NTSTATUS;
/// `HidP_GetButtonCaps_t`
type HidP_GetButtonCaps_t = unsafe extern "system" fn(
    ReportType: HIDP_REPORT_TYPE,
    ButtonCaps: *mut HIDP_BUTTON_CAPS,
    ButtonCapsLength: *mut u16,
    PreparsedData: PHIDP_PREPARSED_DATA,
) -> NTSTATUS;
/// `HidP_GetValueCaps_t`
type HidP_GetValueCaps_t = unsafe extern "system" fn(
    ReportType: HIDP_REPORT_TYPE,
    ValueCaps: *mut HIDP_VALUE_CAPS,
    ValueCapsLength: *mut u16,
    PreparsedData: PHIDP_PREPARSED_DATA,
) -> NTSTATUS;
/// `HidP_MaxDataListLength_t`
type HidP_MaxDataListLength_t = unsafe extern "system" fn(
    ReportType: HIDP_REPORT_TYPE,
    PreparsedData: PHIDP_PREPARSED_DATA,
) -> u32;
/// `HidP_GetData_t`
type HidP_GetData_t = unsafe extern "system" fn(
    ReportType: HIDP_REPORT_TYPE,
    DataList: *mut HIDP_DATA,
    DataLength: *mut u32,
    PreparsedData: PHIDP_PREPARSED_DATA,
    Report: *mut i8,
    ReportLength: u32,
) -> NTSTATUS;

/// The `hid.dll` functions SDL uses (`SDL_HidD_GetManufacturerString` and
/// the others; `SDL_HidD_GetAttributes`, which nothing translated calls,
/// isn't kept); valid while the DLL stays loaded.
///
/// The preparsed data is passed as the bytes the system returned for it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HidFunctions {
    get_manufacturer_string: HidD_GetString_t,
    get_product_string: HidD_GetString_t,
    get_caps: HidP_GetCaps_t,
    get_button_caps: HidP_GetButtonCaps_t,
    get_value_caps: HidP_GetValueCaps_t,
    max_data_list_length: HidP_MaxDataListLength_t,
    get_data: HidP_GetData_t,
}

/// A string from a `HidD_Get*String()` function into a 128 character
/// buffer, if it succeeded.
fn hid_string(f: HidD_GetString_t, device: HANDLE) -> Option<Vec<u16>> {
    let mut string = [0u16; 128];
    // SAFETY: the function was loaded with this signature from a DLL that
    // is still loaded; the buffer holds the size given, in bytes.
    let ok = unsafe {
        f(
            device,
            string.as_mut_ptr().cast(),
            size_of_val(&string) as u32,
        )
    };
    (ok != 0).then(|| string.to_vec())
}

/// `SDL_HidD_GetManufacturerString(handle, buf, sizeof(buf))`; `false` if
/// it fails or `hid.dll` isn't loaded.
pub(crate) fn hidd_get_manufacturer_string(handle: HANDLE, buf: &mut [u16]) -> bool {
    let f = hid().map(|f| f.get_manufacturer_string);
    // SAFETY: the function was loaded with this signature; the buffer holds
    // the size given, in bytes.
    f.is_some_and(|f| unsafe { f(handle, buf.as_mut_ptr().cast(), size_of_val(buf) as u32) != 0 })
}

/// `SDL_HidD_GetProductString(handle, buf, sizeof(buf))`; `false` if it
/// fails or `hid.dll` isn't loaded.
pub(crate) fn hidd_get_product_string(handle: HANDLE, buf: &mut [u16]) -> bool {
    let f = hid().map(|f| f.get_product_string);
    // SAFETY: as above.
    f.is_some_and(|f| unsafe { f(handle, buf.as_mut_ptr().cast(), size_of_val(buf) as u32) != 0 })
}

/// The preparsed data as the pointer the `HidP_*` functions take (they
/// only read it).
fn preparsed(preparsed_data: &[u8]) -> PHIDP_PREPARSED_DATA {
    preparsed_data.as_ptr().cast_mut().cast()
}

impl HidFunctions {
    /// `SDL_HidD_GetManufacturerString()`: the string, as UTF-16 (up to a
    /// NUL), if it worked.
    pub(crate) fn get_manufacturer_string(&self, device: HANDLE) -> Option<Vec<u16>> {
        hid_string(self.get_manufacturer_string, device)
    }

    /// `SDL_HidD_GetProductString()`
    pub(crate) fn get_product_string(&self, device: HANDLE) -> Option<Vec<u16>> {
        hid_string(self.get_product_string, device)
    }

    /// `SDL_HidP_GetCaps()`
    pub(crate) fn get_caps(&self, preparsed_data: &[u8]) -> Result<HIDP_CAPS, NTSTATUS> {
        let mut caps = HIDP_CAPS::default();
        // SAFETY: as in hid_string(); HidP_GetCaps validates and only
        // reads the preparsed data, and fills caps.
        let status = unsafe { (self.get_caps)(preparsed(preparsed_data), &mut caps) };
        if status != HIDP_STATUS_SUCCESS {
            return Err(status);
        }
        Ok(caps)
    }

    /// `SDL_HidP_GetButtonCaps()` with room for `count` entries: the
    /// entries returned.
    pub(crate) fn get_button_caps(
        &self,
        report_type: HIDP_REPORT_TYPE,
        count: u16,
        preparsed_data: &[u8],
    ) -> Result<Vec<HIDP_BUTTON_CAPS>, NTSTATUS> {
        let mut caps = vec![HIDP_BUTTON_CAPS::default(); count as usize];
        let mut length = count;
        // SAFETY: as in get_caps(); caps holds `length` entries.
        let status = unsafe {
            (self.get_button_caps)(
                report_type,
                caps.as_mut_ptr(),
                &mut length,
                preparsed(preparsed_data),
            )
        };
        if status != HIDP_STATUS_SUCCESS {
            return Err(status);
        }
        caps.truncate(length as usize);
        Ok(caps)
    }

    /// `SDL_HidP_GetValueCaps()` with room for `count` entries: the
    /// entries returned.
    pub(crate) fn get_value_caps(
        &self,
        report_type: HIDP_REPORT_TYPE,
        count: u16,
        preparsed_data: &[u8],
    ) -> Result<Vec<HIDP_VALUE_CAPS>, NTSTATUS> {
        let mut caps = vec![HIDP_VALUE_CAPS::default(); count as usize];
        let mut length = count;
        // SAFETY: as in get_caps(); caps holds `length` entries.
        let status = unsafe {
            (self.get_value_caps)(
                report_type,
                caps.as_mut_ptr(),
                &mut length,
                preparsed(preparsed_data),
            )
        };
        if status != HIDP_STATUS_SUCCESS {
            return Err(status);
        }
        caps.truncate(length as usize);
        Ok(caps)
    }

    /// `SDL_HidP_MaxDataListLength()`
    pub(crate) fn max_data_list_length(
        &self,
        report_type: HIDP_REPORT_TYPE,
        preparsed_data: &[u8],
    ) -> u32 {
        // SAFETY: as in get_caps().
        unsafe { (self.max_data_list_length)(report_type, preparsed(preparsed_data)) }
    }

    /// `SDL_HidP_GetData()` into `data`: the number of entries filled in.
    pub(crate) fn get_data(
        &self,
        report_type: HIDP_REPORT_TYPE,
        data: &mut [HIDP_DATA],
        preparsed_data: &[u8],
        report: &[u8],
    ) -> Result<usize, NTSTATUS> {
        let mut length = data.len() as u32;
        // SAFETY: as in get_caps(); data holds `length` entries, and
        // HidP_GetData only reads the report.
        let status = unsafe {
            (self.get_data)(
                report_type,
                data.as_mut_ptr(),
                &mut length,
                preparsed(preparsed_data),
                report.as_ptr().cast_mut().cast(),
                report.len() as u32,
            )
        };
        if status != HIDP_STATUS_SUCCESS {
            return Err(status);
        }
        Ok((length as usize).min(data.len()))
    }
}

/// `s_pHIDDLL`, `s_HIDDLLRefCount` and the function pointers.
struct HidDll {
    dll: Option<SharedObject>,
    ref_count: i32,
    functions: Option<HidFunctions>,
}

static HID: Mutex<HidDll> = Mutex::new(HidDll {
    dll: None,
    ref_count: 0,
    functions: None,
});

fn lock_hid() -> std::sync::MutexGuard<'static, HidDll> {
    HID.lock().unwrap_or_else(|e| e.into_inner())
}

/// The loaded `hid.dll` functions (`None` when it isn't loaded).
pub(crate) fn hid() -> Option<HidFunctions> {
    lock_hid().functions
}

/// Load `hid.dll` (counted). Translation of `WIN_LoadHIDDLL()`.
pub(crate) fn load_hid_dll() -> bool {
    let mut h = lock_hid();
    if h.dll.is_some() {
        crate::sdl_assert!(h.ref_count > 0);
        h.ref_count += 1;
        return true; // already loaded
    }

    let Ok(dll) = SharedObject::load("hid.dll") else {
        return false;
    };

    crate::sdl_assert!(h.ref_count == 0);
    h.ref_count = 1;

    // SAFETY: each symbol is loaded with the type of its hid.dll prototype.
    let functions = unsafe {
        match (
            dll.function::<HidD_GetString_t>("HidD_GetManufacturerString"),
            dll.function::<HidD_GetString_t>("HidD_GetProductString"),
            dll.function::<HidP_GetCaps_t>("HidP_GetCaps"),
            dll.function::<HidP_GetButtonCaps_t>("HidP_GetButtonCaps"),
            dll.function::<HidP_GetValueCaps_t>("HidP_GetValueCaps"),
            dll.function::<HidP_MaxDataListLength_t>("HidP_MaxDataListLength"),
            dll.function::<HidP_GetData_t>("HidP_GetData"),
        ) {
            (
                Ok(get_manufacturer_string),
                Ok(get_product_string),
                Ok(get_caps),
                Ok(get_button_caps),
                Ok(get_value_caps),
                Ok(max_data_list_length),
                Ok(get_data),
            ) => Some(HidFunctions {
                get_manufacturer_string,
                get_product_string,
                get_caps,
                get_button_caps,
                get_value_caps,
                max_data_list_length,
                get_data,
            }),
            _ => None,
        }
    };
    h.dll = Some(dll);
    let Some(functions) = functions else {
        drop(h);
        unload_hid_dll();
        return false;
    };
    h.functions = Some(functions);

    true
}

/// Release a reference to `hid.dll`, unloading it with the last one.
/// Translation of `WIN_UnloadHIDDLL()`.
pub(crate) fn unload_hid_dll() {
    let mut h = lock_hid();
    if h.dll.is_some() {
        crate::sdl_assert!(h.ref_count > 0);
        h.ref_count -= 1;
        if h.ref_count == 0 {
            h.functions = None;
            h.dll = None;
        }
    } else {
        crate::sdl_assert!(h.ref_count == 0);
    }
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

    // The thread is joined and no callback can signal the event any more
    let event = HOTPLUG_EVENT.swap(std::ptr::null_mut(), Ordering::AcqRel);
    if !event.is_null() {
        // SAFETY: the event was created by init_device_notification(), and
        // nothing uses it after this point.
        unsafe {
            CloseHandle(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Foundation::GetHandleInformation;

    #[test]
    fn hid_layouts_and_loading() {
        // (values from mingw-w64's hidpi.h, checked with a C program)
        assert_eq!(size_of::<HIDP_BUTTON_CAPS>(), 72);
        assert_eq!(std::mem::offset_of!(HIDP_BUTTON_CAPS, u), 56);
        assert_eq!(size_of::<HIDP_VALUE_CAPS>(), 72);
        assert_eq!(std::mem::offset_of!(HIDP_VALUE_CAPS, LogicalMin), 40);
        assert_eq!(std::mem::offset_of!(HIDP_VALUE_CAPS, u), 56);
        assert_eq!(size_of::<HIDP_CAPS>(), 64);
        assert_eq!(std::mem::offset_of!(HIDP_CAPS, NumberInputButtonCaps), 46);
        assert_eq!(size_of::<HIDP_DATA>(), 8);
        assert_eq!(HIDP_STATUS_SUCCESS, 0x0011_0000);
        let data = |raw| HIDP_DATA {
            RawValue: raw,
            ..Default::default()
        };
        assert!(data(0x0000_0101).on());
        assert!(!data(0x0000_0100).on());

        let _l = crate::test_support::test_lock();
        assert!(load_hid_dll());
        assert!(load_hid_dll());
        let h = hid().unwrap();
        // Garbage preparsed data is refused
        assert!(h.get_caps(&[0u8; 64]).is_err());
        unload_hid_dll();
        assert!(hid().is_some());
        unload_hid_dll();
        assert!(hid().is_none());
    }

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

    #[test]
    fn hotplug_event_is_closed() {
        let _l = crate::test_support::test_lock();
        for _ in 0..2 {
            init_device_notification();
            let event = HOTPLUG_EVENT.load(Ordering::Acquire);
            assert!(!event.is_null());
            let mut flags = 0;
            // SAFETY: flags is writable; the event is open.
            assert_ne!(unsafe { GetHandleInformation(event, &mut flags) }, 0);
            quit_device_notification();
            // Each init/quit cycle releases its event
            assert!(HOTPLUG_EVENT.load(Ordering::Acquire).is_null());
        }
    }
}
