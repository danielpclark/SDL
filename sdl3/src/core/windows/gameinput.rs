// Rust translation of src/core/windows/SDL_gameinput.cpp and SDL_gameinput.h
// from Simple DirectMedia Layer, with the parts of the GameInput API (v3)
// they use from src/core/windows/gameinput/gameinput.h and the loader of
// src/core/windows/gameinput/gameinput.cpp (both Copyright (c) Microsoft
// Corporation, MIT License).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! GameInput, the input API of the Microsoft GDK, shared by the GameInput
//! joystick driver and the Windows video driver's raw keyboard and mouse.
//!
//! Upstream compiles Microsoft's `gameinput.cpp` in: its
//! `GameInputInitialize()` finds the newest of the inbox `GameInput.dll`
//! (System32), the redistributable `GameInputRedist.dll` (System32 or the
//! directory in the registry) and one next to the executable, loads it and
//! asks it for the interface. [`init_game_input`] does the same, loading
//! the DLL at run time (and keeping it loaded, as `gameinput.cpp` does).
//!
//! The COM interfaces are declared by hand, in the vtable order of
//! `gameinput.h` (API version 3, the `GameInput::v3` namespace); the slots
//! SDL doesn't call are placeholders. [`GameInputRef`] is a reference of
//! `SDL_InitGameInput()`'s count; dropping it is `SDL_QuitGameInput()`.

#![allow(clippy::upper_case_acronyms)]

use std::ffi::{c_void, CStr};
use std::ptr::NonNull;
use std::sync::Mutex;

use windows_sys::core::{GUID, HRESULT};
use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_FILE_NOT_FOUND, ERROR_PROC_NOT_FOUND, ERROR_SUCCESS, E_NOINTERFACE,
    E_POINTER, HMODULE,
};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileAttributesW, GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    FILE_ATTRIBUTE_DIRECTORY, INVALID_FILE_ATTRIBUTES, VS_FIXEDFILEINFO,
};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Registry::{
    RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6432KEY,
};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

use super::com::{ComPtr, IUnknownVtbl};
use super::{
    error_from_hresult, get_module_path, has_broken_ezfrd64_dll, utf8_to_wide, wide_to_utf8,
};
use crate::error::{Error, Result};
use crate::hints;

/// Translation of `GAMEINPUT_API_VERSION` (of the bundled header).
pub(crate) const GAMEINPUT_API_VERSION: u32 = 3;

/// The default of `SDL_HINT_JOYSTICK_GAMEINPUT`. Translation of
/// `SDL_GAMEINPUT_DEFAULT` (true from GameInput API version 3).
pub(crate) const GAMEINPUT_DEFAULT: bool = GAMEINPUT_API_VERSION >= 3;

/// `IID_IGameInput` (of the v3 API).
pub(crate) const IID_IGAMEINPUT: GUID = GUID::from_u128(0x20efc1c7_5d9a_43ba_b26f_b807fa48609c);

/// `IID_IGameInput_v0` (`gameinput.cpp`): the interface of the GameInput
/// versions without `GameInputInitialize()`.
const IID_IGAMEINPUT_V0: GUID = GUID::from_u128(0x11be2a7e_4254_445a_9c09_ffc40f006918);

/// `GAMEINPUT_E_READING_NOT_FOUND`.
pub(crate) const GAMEINPUT_E_READING_NOT_FOUND: HRESULT = 0x838A_0003_u32 as HRESULT;

/// `GameInputKind`.
pub(crate) mod kind {
    pub(crate) const UNKNOWN: u32 = 0x00000000;
    pub(crate) const RAW_DEVICE_REPORT: u32 = 0x00000001;
    pub(crate) const CONTROLLER: u32 = 0x0000000E;
    pub(crate) const KEYBOARD: u32 = 0x00000010;
    pub(crate) const MOUSE: u32 = 0x00000020;
    pub(crate) const SENSORS: u32 = 0x00000040;
    pub(crate) const ARCADE_STICK: u32 = 0x00010000;
    pub(crate) const FLIGHT_STICK: u32 = 0x00020000;
    pub(crate) const GAMEPAD: u32 = 0x00040000;
    pub(crate) const RACING_WHEEL: u32 = 0x00080000;
}

/// `GameInputEnumerationKind`: `GameInputBlockingEnumeration`.
pub(crate) const BLOCKING_ENUMERATION: i32 = 2;

/// `GameInputFocusPolicy`.
pub(crate) mod focus_policy {
    pub(crate) const ENABLE_BACKGROUND_INPUT: u32 = 0x00000040;
    pub(crate) const ENABLE_BACKGROUND_GUIDE_BUTTON: u32 = 0x00000080;
    pub(crate) const ENABLE_BACKGROUND_SHARE_BUTTON: u32 = 0x00000100;
}

/// `GameInputSwitchPosition`.
pub(crate) mod switch_position {
    pub(crate) const CENTER: i32 = 0;
    pub(crate) const UP: i32 = 1;
    pub(crate) const UP_RIGHT: i32 = 2;
    pub(crate) const RIGHT: i32 = 3;
    pub(crate) const DOWN_RIGHT: i32 = 4;
    pub(crate) const DOWN: i32 = 5;
    pub(crate) const DOWN_LEFT: i32 = 6;
    pub(crate) const LEFT: i32 = 7;
    pub(crate) const UP_LEFT: i32 = 8;
}

/// `GameInputMouseButtons`.
pub(crate) mod mouse_buttons {
    pub(crate) const LEFT_BUTTON: u32 = 0x00000001;
    pub(crate) const RIGHT_BUTTON: u32 = 0x00000002;
    pub(crate) const MIDDLE_BUTTON: u32 = 0x00000004;
    pub(crate) const BUTTON4: u32 = 0x00000008;
    pub(crate) const BUTTON5: u32 = 0x00000010;
    pub(crate) const WHEEL_TILT_LEFT: u32 = 0x00000020;
    pub(crate) const WHEEL_TILT_RIGHT: u32 = 0x00000040;
}

/// `GameInputSensorsKind`.
pub(crate) mod sensors {
    pub(crate) const ACCELEROMETER: u32 = 0x00000001;
    pub(crate) const GYROMETER: u32 = 0x00000002;
}

/// `GameInputGamepadButtons`.
pub(crate) mod gamepad_buttons {
    pub(crate) const MENU: u32 = 0x00000001;
    pub(crate) const VIEW: u32 = 0x00000002;
    pub(crate) const A: u32 = 0x00000004;
    pub(crate) const B: u32 = 0x00000008;
    pub(crate) const X: u32 = 0x00000010;
    pub(crate) const Y: u32 = 0x00000020;
    pub(crate) const DPAD_UP: u32 = 0x00000040;
    pub(crate) const DPAD_DOWN: u32 = 0x00000080;
    pub(crate) const DPAD_LEFT: u32 = 0x00000100;
    pub(crate) const DPAD_RIGHT: u32 = 0x00000200;
    pub(crate) const LEFT_SHOULDER: u32 = 0x00000400;
    pub(crate) const RIGHT_SHOULDER: u32 = 0x00000800;
    pub(crate) const LEFT_THUMBSTICK: u32 = 0x00001000;
    pub(crate) const RIGHT_THUMBSTICK: u32 = 0x00002000;
}

/// `GameInputSystemButtons`.
pub(crate) mod system_buttons {
    pub(crate) const NONE: u32 = 0x00000000;
    pub(crate) const GUIDE: u32 = 0x00000001;
    pub(crate) const SHARE: u32 = 0x00000002;
}

/// `GameInputDeviceStatus`: `GameInputDeviceConnected`.
pub(crate) const DEVICE_CONNECTED: u32 = 0x00000001;

/// `GameInputRumbleMotors`.
pub(crate) mod rumble_motors {
    pub(crate) const LOW_FREQUENCY: u32 = 0x00000001;
    pub(crate) const HIGH_FREQUENCY: u32 = 0x00000002;
    pub(crate) const LEFT_TRIGGER: u32 = 0x00000004;
    pub(crate) const RIGHT_TRIGGER: u32 = 0x00000008;
}

/// `APP_LOCAL_DEVICE_ID_SIZE`.
pub(crate) const APP_LOCAL_DEVICE_ID_SIZE: usize = 32;

/// `GameInputCallbackToken`.
pub(crate) type CallbackToken = u64;

/// `GameInputDeviceCallback`.
pub(crate) type DeviceCallback = unsafe extern "system" fn(
    callback_token: CallbackToken,
    context: *mut c_void,
    device: *mut c_void,
    timestamp: u64,
    current_status: u32,
    previous_status: u32,
);

/// `GameInputSystemButtonCallback`.
pub(crate) type SystemButtonCallback = unsafe extern "system" fn(
    callback_token: CallbackToken,
    context: *mut c_void,
    device: *mut c_void,
    timestamp: u64,
    current_buttons: u32,
    previous_buttons: u32,
);

/// `GameInputKeyState`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct KeyState {
    pub(crate) scan_code: u32,
    pub(crate) code_point: u32,
    pub(crate) virtual_key: u8,
    pub(crate) is_dead_key: bool,
}

/// `GameInputMouseState`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MouseState {
    pub(crate) buttons: u32,
    pub(crate) positions: u32,
    pub(crate) position_x: i64,
    pub(crate) position_y: i64,
    pub(crate) absolute_position_x: i64,
    pub(crate) absolute_position_y: i64,
    pub(crate) wheel_x: i64,
    pub(crate) wheel_y: i64,
}

/// `GameInputSensorsState`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SensorsState {
    pub(crate) acceleration_in_g_x: f32,
    pub(crate) acceleration_in_g_y: f32,
    pub(crate) acceleration_in_g_z: f32,
    pub(crate) angular_velocity_in_rad_per_sec_x: f32,
    pub(crate) angular_velocity_in_rad_per_sec_y: f32,
    pub(crate) angular_velocity_in_rad_per_sec_z: f32,
    pub(crate) heading_in_degrees_from_magnetic_north: f32,
    pub(crate) heading_accuracy: i32,
    pub(crate) orientation_w: f32,
    pub(crate) orientation_x: f32,
    pub(crate) orientation_y: f32,
    pub(crate) orientation_z: f32,
}

/// `GameInputGamepadState`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct GamepadState {
    pub(crate) buttons: u32,
    pub(crate) left_trigger: f32,
    pub(crate) right_trigger: f32,
    pub(crate) left_thumbstick_x: f32,
    pub(crate) left_thumbstick_y: f32,
    pub(crate) right_thumbstick_x: f32,
    pub(crate) right_thumbstick_y: f32,
}

/// `GameInputRumbleParams`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct RumbleParams {
    pub(crate) low_frequency: f32,
    pub(crate) high_frequency: f32,
    pub(crate) left_trigger: f32,
    pub(crate) right_trigger: f32,
}

/// `GameInputUsage`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Usage {
    pub(crate) page: u16,
    pub(crate) id: u16,
}

/// `GameInputVersion`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Version {
    pub(crate) major: u16,
    pub(crate) minor: u16,
    pub(crate) build: u16,
    pub(crate) revision: u16,
}

/// `GameInputKeyboardInfo`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct KeyboardInfo {
    pub(crate) kind: i32,
    pub(crate) layout: u32,
    pub(crate) key_count: u32,
    pub(crate) function_key_count: u32,
    pub(crate) max_simultaneous_keys: u32,
    pub(crate) platform_type: u32,
    pub(crate) platform_subtype: u32,
}

/// `GameInputSensorsInfo`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SensorsInfo {
    pub(crate) supported_sensors: u32,
}

/// `GameInputControllerInfo` (the label arrays are left opaque).
#[repr(C)]
#[derive(Debug)]
pub(crate) struct ControllerInfo {
    pub(crate) controller_axis_count: u32,
    controller_axis_labels: *const i32,
    pub(crate) controller_button_count: u32,
    controller_button_labels: *const i32,
    pub(crate) controller_switch_count: u32,
    controller_switch_info: *const c_void,
}

/// `GameInputDeviceInfo`. The pointers are GameInput's, valid as long as
/// the device; they are private, and read through the accessors.
#[repr(C)]
pub(crate) struct DeviceInfo {
    pub(crate) vendor_id: u16,
    pub(crate) product_id: u16,
    pub(crate) revision_number: u16,
    pub(crate) usage: Usage,
    pub(crate) hardware_version: Version,
    pub(crate) firmware_version: Version,
    /// `APP_LOCAL_DEVICE_ID`
    pub(crate) device_id: [u8; APP_LOCAL_DEVICE_ID_SIZE],
    /// `APP_LOCAL_DEVICE_ID`
    pub(crate) device_root_id: [u8; APP_LOCAL_DEVICE_ID_SIZE],
    pub(crate) device_family: i32,
    pub(crate) supported_input: u32,
    pub(crate) supported_rumble_motors: u32,
    pub(crate) supported_system_buttons: u32,
    pub(crate) container_id: GUID,
    display_name: *const std::ffi::c_char,
    pnp_path: *const std::ffi::c_char,
    keyboard_info: *const KeyboardInfo,
    mouse_info: *const c_void,
    sensors_info: *const SensorsInfo,
    controller_info: *const ControllerInfo,
    arcade_stick_info: *const c_void,
    flight_stick_info: *const c_void,
    gamepad_info: *const c_void,
    racing_wheel_info: *const c_void,
    pub(crate) force_feedback_motor_count: u32,
    force_feedback_motor_info: *const c_void,
    pub(crate) input_report_count: u32,
    input_report_info: *const c_void,
    pub(crate) output_report_count: u32,
    output_report_info: *const c_void,
}

impl Default for DeviceInfo {
    /// A device with nothing: no IDs, no input, NULL pointers.
    fn default() -> DeviceInfo {
        // SAFETY: all-zero is valid for every field (integers, GUID, NULL
        // pointers).
        unsafe { std::mem::zeroed() }
    }
}

impl DeviceInfo {
    /// A NUL-terminated string of the info, if there is one.
    fn string(ptr: *const std::ffi::c_char) -> Option<String> {
        // SAFETY: the info's strings are NULL or NUL-terminated and live as
        // long as the info (a `DeviceInfo` is only ever seen behind its
        // device, or zeroed).
        (!ptr.is_null()).then(|| {
            unsafe { CStr::from_ptr(ptr) }
                .to_string_lossy()
                .into_owned()
        })
    }

    /// A pointed-to part of the info, if there is one.
    fn part<T>(&self, ptr: *const T) -> Option<&T> {
        // SAFETY: as for `string()`.
        unsafe { ptr.as_ref() }
    }

    /// `displayName`.
    pub(crate) fn display_name(&self) -> Option<String> {
        DeviceInfo::string(self.display_name)
    }

    /// `pnpPath`.
    pub(crate) fn pnp_path(&self) -> Option<String> {
        DeviceInfo::string(self.pnp_path)
    }

    /// `keyboardInfo`.
    pub(crate) fn keyboard_info(&self) -> Option<&KeyboardInfo> {
        self.part(self.keyboard_info)
    }

    /// `sensorsInfo`.
    pub(crate) fn sensors_info(&self) -> Option<&SensorsInfo> {
        self.part(self.sensors_info)
    }

    /// `controllerInfo`.
    pub(crate) fn controller_info(&self) -> Option<&ControllerInfo> {
        self.part(self.controller_info)
    }
}

/// An unused vtable slot.
type Slot = usize;

/// `IGameInput`'s vtable.
#[repr(C)]
pub(crate) struct IGameInputVtbl {
    base: IUnknownVtbl,
    get_current_timestamp: unsafe extern "system" fn(*mut c_void) -> u64,
    get_current_reading:
        unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut *mut c_void) -> HRESULT,
    get_next_reading: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        *mut c_void,
        *mut *mut c_void,
    ) -> HRESULT,
    /// GetPreviousReading, RegisterReadingCallback
    _get_previous_reading: [Slot; 2],
    register_device_callback: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        i32,
        *mut c_void,
        DeviceCallback,
        *mut CallbackToken,
    ) -> HRESULT,
    register_system_button_callback: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        *mut c_void,
        SystemButtonCallback,
        *mut CallbackToken,
    ) -> HRESULT,
    /// RegisterKeyboardLayoutCallback, StopCallback
    _register_keyboard_layout_callback: [Slot; 2],
    unregister_callback: unsafe extern "system" fn(*mut c_void, CallbackToken) -> bool,
    /// CreateDispatcher, FindDeviceFromId, FindDeviceFromPlatformString
    _create_dispatcher: [Slot; 3],
    set_focus_policy: unsafe extern "system" fn(*mut c_void, u32),
    /// CreateAggregateDevice, DisableAggregateDevice
    _create_aggregate_device: [Slot; 2],
}

/// `IGameInputReading`'s vtable.
#[repr(C)]
pub(crate) struct IGameInputReadingVtbl {
    base: IUnknownVtbl,
    _get_input_kind: Slot,
    get_timestamp: unsafe extern "system" fn(*mut c_void) -> u64,
    /// GetDevice, GetControllerAxisCount
    _get_device: [Slot; 2],
    get_controller_axis_state: unsafe extern "system" fn(*mut c_void, u32, *mut f32) -> u32,
    _get_controller_button_count: Slot,
    get_controller_button_state: unsafe extern "system" fn(*mut c_void, u32, *mut bool) -> u32,
    _get_controller_switch_count: Slot,
    get_controller_switch_state: unsafe extern "system" fn(*mut c_void, u32, *mut i32) -> u32,
    _get_key_count: Slot,
    get_key_state: unsafe extern "system" fn(*mut c_void, u32, *mut KeyState) -> u32,
    get_mouse_state: unsafe extern "system" fn(*mut c_void, *mut MouseState) -> bool,
    get_sensors_state: unsafe extern "system" fn(*mut c_void, *mut SensorsState) -> bool,
    /// GetArcadeStickState, GetFlightStickState
    _get_arcade_stick_state: [Slot; 2],
    get_gamepad_state: unsafe extern "system" fn(*mut c_void, *mut GamepadState) -> bool,
    _get_racing_wheel_state: Slot,
    get_raw_report: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> bool,
}

/// `IGameInputDevice`'s vtable.
#[repr(C)]
pub(crate) struct IGameInputDeviceVtbl {
    base: IUnknownVtbl,
    get_device_info: unsafe extern "system" fn(*mut c_void, *mut *const DeviceInfo) -> HRESULT,
    _get_haptic_info: Slot,
    get_device_status: unsafe extern "system" fn(*mut c_void) -> u32,
    _create_force_feedback_effect: Slot,
    is_force_feedback_motor_powered_on: unsafe extern "system" fn(*mut c_void, u32) -> bool,
    _set_force_feedback_motor_gain: Slot,
    set_rumble_state: unsafe extern "system" fn(*mut c_void, *const RumbleParams),
    /// DirectInputEscape, CreateInputMapper, GetExtraAxisCount,
    /// GetExtraButtonCount, GetExtraAxisIndexes, GetExtraButtonIndexes,
    /// CreateRawDeviceReport, SendRawDeviceOutput
    _direct_input_escape: [Slot; 8],
}

/// `IGameInputRawDeviceReport`'s vtable.
#[repr(C)]
pub(crate) struct IGameInputRawDeviceReportVtbl {
    base: IUnknownVtbl,
    /// GetDevice, GetReportInfo, GetRawDataSize
    _get_device: [Slot; 3],
    get_raw_data: unsafe extern "system" fn(*mut c_void, usize, *mut c_void) -> usize,
    _set_raw_data: Slot,
}

/// The `IGameInput` interface.
#[derive(Clone, Debug)]
pub(crate) struct GameInput(ComPtr<IGameInputVtbl>);

// SAFETY: GameInput's interfaces are free-threaded: its methods may be
// called from any thread at once.
unsafe impl Sync for GameInput {}

impl GameInput {
    fn this(&self) -> *mut c_void {
        self.0.as_ptr().cast()
    }

    /// `GetCurrentTimestamp()`, in microseconds.
    pub(crate) fn current_timestamp(&self) -> u64 {
        // SAFETY: the interface is alive.
        unsafe { (self.0.vtbl().get_current_timestamp)(self.this()) }
    }

    /// `GetCurrentReading()`.
    pub(crate) fn current_reading(
        &self,
        input_kind: u32,
        device: Option<&Device>,
    ) -> std::result::Result<Reading, HRESULT> {
        let device = device.map_or(std::ptr::null_mut(), Device::this);
        // SAFETY: the reading is stored on success.
        unsafe {
            ComPtr::from_out(|out: *mut *mut _| {
                (self.0.vtbl().get_current_reading)(self.this(), input_kind, device, out.cast())
            })
        }
        .map(Reading)
    }

    /// `GetNextReading()`.
    pub(crate) fn next_reading(
        &self,
        reference_reading: &Reading,
        input_kind: u32,
        device: Option<&Device>,
    ) -> std::result::Result<Reading, HRESULT> {
        let device = device.map_or(std::ptr::null_mut(), Device::this);
        // SAFETY: the reading is stored on success.
        unsafe {
            ComPtr::from_out(|out: *mut *mut _| {
                (self.0.vtbl().get_next_reading)(
                    self.this(),
                    reference_reading.this(),
                    input_kind,
                    device,
                    out.cast(),
                )
            })
        }
        .map(Reading)
    }

    /// `RegisterDeviceCallback()` for all devices: the token.
    ///
    /// # Safety
    ///
    /// `context` must stay valid for `callback` until the callback is
    /// unregistered.
    pub(crate) unsafe fn register_device_callback(
        &self,
        input_kind: u32,
        status_filter: u32,
        enumeration_kind: i32,
        context: *mut c_void,
        callback: DeviceCallback,
    ) -> std::result::Result<CallbackToken, HRESULT> {
        let mut token = 0;
        // SAFETY: the interface is alive; the caller vouches for context.
        let hr = unsafe {
            (self.0.vtbl().register_device_callback)(
                self.this(),
                std::ptr::null_mut(),
                input_kind,
                status_filter,
                enumeration_kind,
                context,
                callback,
                &mut token,
            )
        };
        if hr < 0 {
            Err(hr)
        } else {
            Ok(token)
        }
    }

    /// `RegisterSystemButtonCallback()` for a device: the token.
    ///
    /// # Safety
    ///
    /// As for [`register_device_callback`](GameInput::register_device_callback).
    pub(crate) unsafe fn register_system_button_callback(
        &self,
        device: &Device,
        button_filter: u32,
        context: *mut c_void,
        callback: SystemButtonCallback,
    ) -> std::result::Result<CallbackToken, HRESULT> {
        let mut token = 0;
        // SAFETY: as above.
        let hr = unsafe {
            (self.0.vtbl().register_system_button_callback)(
                self.this(),
                device.this(),
                button_filter,
                context,
                callback,
                &mut token,
            )
        };
        if hr < 0 {
            Err(hr)
        } else {
            Ok(token)
        }
    }

    /// `UnregisterCallback()` (which waits for a running callback to
    /// return).
    pub(crate) fn unregister_callback(&self, callback_token: CallbackToken) -> bool {
        // SAFETY: the interface is alive.
        unsafe { (self.0.vtbl().unregister_callback)(self.this(), callback_token) }
    }

    /// `SetFocusPolicy()`.
    pub(crate) fn set_focus_policy(&self, policy: u32) {
        // SAFETY: the interface is alive.
        unsafe { (self.0.vtbl().set_focus_policy)(self.this(), policy) }
    }
}

/// An `IGameInputDevice`, with its info.
#[derive(Clone, Debug)]
pub(crate) struct Device {
    device: ComPtr<IGameInputDeviceVtbl>,
    info: NonNull<DeviceInfo>,
}

// SAFETY: GameInput's objects are free-threaded; the info is immutable.
unsafe impl Send for Device {}

impl Device {
    /// A reference of our own to a device GameInput passes a callback, and
    /// its info (`GetDeviceInfo()`), or the error getting it.
    ///
    /// # Safety
    ///
    /// `device` must be NULL or a live `IGameInputDevice`.
    pub(crate) unsafe fn from_borrowed(
        device: *mut c_void,
    ) -> std::result::Result<Device, HRESULT> {
        // SAFETY: the caller's contract; the clone (AddRef) is ours.
        let borrowed = std::mem::ManuallyDrop::new(
            unsafe { ComPtr::<IGameInputDeviceVtbl>::from_raw(device.cast()) }.ok_or(E_POINTER)?,
        );
        let device = (*borrowed).clone();
        let mut info: *const DeviceInfo = std::ptr::null();
        // SAFETY: GetDeviceInfo stores a pointer to the device's info.
        let hr = unsafe { (device.vtbl().get_device_info)(device.as_ptr().cast(), &mut info) };
        if hr < 0 {
            return Err(hr);
        }
        let info = NonNull::new(info.cast_mut()).ok_or(E_POINTER)?;
        Ok(Device { device, info })
    }

    fn this(&self) -> *mut c_void {
        self.device.as_ptr().cast()
    }

    /// Whether `self` and `other` are the same device.
    pub(crate) fn same(&self, other: &Device) -> bool {
        self.device.same(&other.device)
    }

    /// Whether this is the interface pointer `device`.
    pub(crate) fn is(&self, device: *mut c_void) -> bool {
        self.this() == device
    }

    /// The device's info (`GetDeviceInfo()`).
    pub(crate) fn info(&self) -> &DeviceInfo {
        // SAFETY: the info lives as long as the device, which we hold.
        unsafe { self.info.as_ref() }
    }

    /// `GetDeviceStatus()`.
    pub(crate) fn status(&self) -> u32 {
        // SAFETY: the interface is alive.
        unsafe { (self.device.vtbl().get_device_status)(self.this()) }
    }

    /// `IsForceFeedbackMotorPoweredOn()`.
    pub(crate) fn is_force_feedback_motor_powered_on(&self, motor_index: u32) -> bool {
        // SAFETY: the interface is alive.
        unsafe { (self.device.vtbl().is_force_feedback_motor_powered_on)(self.this(), motor_index) }
    }

    /// `SetRumbleState()`.
    pub(crate) fn set_rumble_state(&self, params: &RumbleParams) {
        // SAFETY: the interface is alive; params is read during the call.
        unsafe { (self.device.vtbl().set_rumble_state)(self.this(), params) }
    }
}

/// An `IGameInputReading`.
#[derive(Debug)]
pub(crate) struct Reading(ComPtr<IGameInputReadingVtbl>);

impl Reading {
    fn this(&self) -> *mut c_void {
        self.0.as_ptr().cast()
    }

    /// `GetTimestamp()`, in microseconds.
    pub(crate) fn timestamp(&self) -> u64 {
        // SAFETY: the interface is alive.
        unsafe { (self.0.vtbl().get_timestamp)(self.this()) }
    }

    /// Fill a state array with a `Get*State(count, array)` method; the
    /// states it wrote.
    fn states<T: Default + Clone>(
        &self,
        count: usize,
        get: unsafe extern "system" fn(*mut c_void, u32, *mut T) -> u32,
    ) -> Vec<T> {
        let count = count.min(u32::MAX as usize);
        let mut states = vec![T::default(); count];
        // SAFETY: the array holds `count` states.
        let written = unsafe { get(self.this(), count as u32, states.as_mut_ptr()) } as usize;
        states.truncate(written);
        states
    }

    /// `GetControllerAxisState()`: up to `count` axes.
    pub(crate) fn controller_axis_state(&self, count: usize) -> Vec<f32> {
        self.states(count, self.0.vtbl().get_controller_axis_state)
    }

    /// `GetControllerButtonState()`: up to `count` buttons.
    pub(crate) fn controller_button_state(&self, count: usize) -> Vec<bool> {
        self.states(count, self.0.vtbl().get_controller_button_state)
    }

    /// `GetControllerSwitchState()`: up to `count` switch positions.
    pub(crate) fn controller_switch_state(&self, count: usize) -> Vec<i32> {
        self.states(count, self.0.vtbl().get_controller_switch_state)
    }

    /// `GetKeyState()`: up to `count` pressed keys.
    pub(crate) fn key_state(&self, count: usize) -> Vec<KeyState> {
        self.states(count, self.0.vtbl().get_key_state)
    }

    /// Read a state with a `Get*State(state)` method, if the reading has
    /// it.
    fn state<T: Default>(
        &self,
        get: unsafe extern "system" fn(*mut c_void, *mut T) -> bool,
    ) -> Option<T> {
        let mut state = T::default();
        // SAFETY: the interface is alive; the state is written.
        unsafe { get(self.this(), &mut state) }.then_some(state)
    }

    /// `GetMouseState()`.
    pub(crate) fn mouse_state(&self) -> Option<MouseState> {
        self.state(self.0.vtbl().get_mouse_state)
    }

    /// `GetSensorsState()`.
    pub(crate) fn sensors_state(&self) -> Option<SensorsState> {
        self.state(self.0.vtbl().get_sensors_state)
    }

    /// `GetGamepadState()`.
    pub(crate) fn gamepad_state(&self) -> Option<GamepadState> {
        self.state(self.0.vtbl().get_gamepad_state)
    }

    /// `GetRawReport()`.
    pub(crate) fn raw_report(&self) -> Option<RawDeviceReport> {
        let mut report: *mut c_void = std::ptr::null_mut();
        // SAFETY: the interface is alive; the report is stored (or NULL).
        if !unsafe { (self.0.vtbl().get_raw_report)(self.this(), &mut report) } {
            return None;
        }
        // SAFETY: the report is a reference of ours to a raw report.
        unsafe { ComPtr::from_raw(report.cast()) }.map(RawDeviceReport)
    }
}

/// An `IGameInputRawDeviceReport`.
#[derive(Debug)]
pub(crate) struct RawDeviceReport(ComPtr<IGameInputRawDeviceReportVtbl>);

impl RawDeviceReport {
    /// `GetRawData()`: the bytes copied into `buffer`.
    pub(crate) fn raw_data(&self, buffer: &mut [u8]) -> usize {
        // SAFETY: the buffer holds the given number of bytes.
        unsafe {
            (self.0.vtbl().get_raw_data)(
                self.0.as_ptr().cast(),
                buffer.len(),
                buffer.as_mut_ptr().cast(),
            )
        }
    }
}

// --- the loader of gameinput.cpp ---

/// `HRESULT_FROM_WIN32()`.
fn hresult_from_win32(error: u32) -> HRESULT {
    if error as HRESULT <= 0 {
        error as HRESULT
    } else {
        ((error & 0x0000_FFFF) | (7 << 16) | 0x8000_0000) as HRESULT
    }
}

/// `HRESULT_FROM_WIN32(GetLastError())`.
fn last_error() -> HRESULT {
    // SAFETY: GetLastError has no preconditions.
    hresult_from_win32(unsafe { GetLastError() })
}

/// The System32 directory. Translation of `GetSystemDirectory()`.
fn system_directory() -> std::result::Result<String, HRESULT> {
    // SAFETY: a NULL buffer asks for the length.
    let length = unsafe { GetSystemDirectoryW(std::ptr::null_mut(), 0) };
    if length == 0 {
        return Err(last_error());
    }
    let mut dir = vec![0u16; length as usize];
    // SAFETY: the buffer holds `length` units.
    let length = unsafe { GetSystemDirectoryW(dir.as_mut_ptr(), length) };
    Ok(wide_to_utf8(&dir[..(length as usize).min(dir.len())]))
}

/// The directory of the executable, with its trailing separator.
/// Translation of `GetApplicationDirectory()`.
fn application_directory() -> Option<String> {
    let mut path = get_module_path(std::ptr::null_mut()).ok()?;
    // Trim the executable filename, leaving only the directory path.
    let length = path.rfind(['\\', '/']).map_or(0, |i| i + 1);
    path.truncate(length);
    Some(path)
}

/// The directory the GameInput redistributable is installed in. Translation
/// of `GetRedistDirectory()` (which loads `RegGetValueW()` itself).
fn redist_directory() -> std::result::Result<String, HRESULT> {
    let key = utf8_to_wide("SOFTWARE\\Microsoft\\GameInput");
    let value = utf8_to_wide("RedistDir");
    let flags = RRF_RT_REG_SZ | RRF_SUBKEY_WOW6432KEY;

    let mut cb_size = 0u32;
    // SAFETY: a NULL buffer asks for the size; the names are NUL-terminated.
    let error = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            flags,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut cb_size,
        )
    };
    if error != ERROR_SUCCESS {
        return Err(hresult_from_win32(error));
    }

    let mut dir = vec![0u16; (cb_size as usize).div_ceil(2)];
    // SAFETY: the buffer holds cb_size bytes.
    let error = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            flags,
            std::ptr::null_mut(),
            dir.as_mut_ptr().cast(),
            &mut cb_size,
        )
    };
    if error != ERROR_SUCCESS {
        return Err(hresult_from_win32(error));
    }
    dir.truncate(cb_size as usize / 2);
    Ok(wide_to_utf8(&dir))
}

/// The file version of a file (`dwFileVersionMS` in the high half).
/// Translation of `GetFileVersion()` (which loads the version functions
/// itself).
fn file_version(path: &[u16]) -> Option<u64> {
    let mut unused = 0;
    // SAFETY: the path is NUL-terminated.
    let buffer_size = unsafe { GetFileVersionInfoSizeW(path.as_ptr(), &mut unused) };
    if buffer_size == 0 {
        return None;
    }
    let mut buffer = vec![0u8; buffer_size as usize];
    // SAFETY: the buffer holds buffer_size bytes.
    if unsafe { GetFileVersionInfoW(path.as_ptr(), 0, buffer_size, buffer.as_mut_ptr().cast()) }
        == 0
    {
        return None;
    }
    let mut ver_info: *mut c_void = std::ptr::null_mut();
    let mut ver_length = 0u32;
    let root = utf8_to_wide("\\");
    // SAFETY: the buffer is a version resource; VerQueryValue points into it.
    if unsafe {
        VerQueryValueW(
            buffer.as_ptr().cast(),
            root.as_ptr(),
            &mut ver_info,
            &mut ver_length,
        )
    } == 0
        || ver_info.is_null()
        || (ver_length as usize) < size_of::<VS_FIXEDFILEINFO>()
    {
        return None;
    }
    // SAFETY: the root block is a VS_FIXEDFILEINFO inside the buffer.
    let ver_info = unsafe { ver_info.cast::<VS_FIXEDFILEINFO>().read_unaligned() };
    Some(((ver_info.dwFileVersionMS as u64) << 32) | ver_info.dwFileVersionLS as u64)
}

/// The version of a file if it exists (zero where the version can't be
/// read). Translation of `GetFileInfo()` and `FileExists()`.
fn file_info(path: &str) -> Option<u64> {
    let path = utf8_to_wide(path);
    // SAFETY: the path is NUL-terminated.
    let attributes = unsafe { GetFileAttributesW(path.as_ptr()) };
    if attributes == INVALID_FILE_ATTRIBUTES || (attributes & FILE_ATTRIBUTE_DIRECTORY) != 0 {
        return None;
    }

    // Best effort attempt to get DLL verison; will return zero
    // on platforms which do not support this query.
    Some(file_version(&path).unwrap_or(0))
}

/// Join two paths with one separator. Translation of `PathJoin()`.
pub(crate) fn path_join(path1: &str, path2: &str) -> String {
    if path1.is_empty() {
        return path2.to_string();
    }
    if path2.is_empty() {
        return path1.to_string();
    }

    let mut result = path1.to_string();
    if !result.ends_with(['\\', '/']) {
        result.push('\\');
    }
    result.push_str(path2.strip_prefix(['\\', '/']).unwrap_or(path2));
    result
}

/// The candidate with the highest version, of the ones found (`path`,
/// `version`), in the order evaluated. Ties prefer redist over inbox and
/// sideloaded over both, matching the order in which candidates are
/// evaluated. (Part of `LoadGameInputDll()`.)
pub(crate) fn select_game_input_dll<P>(
    candidates: impl IntoIterator<Item = Option<(P, u64)>>,
) -> Option<P> {
    candidates
        .into_iter()
        .flatten()
        .fold(None, |best: Option<(P, u64)>, (path, version)| match best {
            Some((_, best_version)) if version < best_version => best,
            _ => Some((path, version)),
        })
        .map(|(path, _)| path)
}

/// Find and load the newest GameInput DLL. Translation of
/// `LoadGameInputDll()`.
fn load_game_input_dll() -> std::result::Result<HMODULE, HRESULT> {
    let system_dir = system_directory()?;

    let found = |path: String| file_info(&path).map(|version| (path, version));

    let inbox = found(path_join(&system_dir, "GameInput.dll"));

    let mut redist = found(path_join(&system_dir, "GameInputRedist.dll"));
    if redist.is_none() {
        // GameInputRedist.dll can be found in System32 and Program Files;
        // check both locations for improved compatibility.
        redist = redist_directory()
            .ok()
            .and_then(|dir| found(path_join(&dir, "GameInputRedist.dll")));
    }

    let sideloaded =
        application_directory().and_then(|dir| found(path_join(&dir, "GameInputRedist.dll")));

    let path = select_game_input_dll([inbox, redist, sideloaded])
        .ok_or(hresult_from_win32(ERROR_FILE_NOT_FOUND))?;

    let wpath = utf8_to_wide(&path);
    // SAFETY: the path is NUL-terminated.
    let module = unsafe { LoadLibraryW(wpath.as_ptr()) };
    if module.is_null() {
        return Err(last_error());
    }
    Ok(module)
}

/// Translation of `g_gameInputDll` (loaded once, never unloaded).
static GAME_INPUT_DLL: Mutex<usize> = Mutex::new(0);

/// `GameInputInitialize()`, of the DLL.
type GameInputInitializeFn = unsafe extern "C" fn(*const GUID, *mut *mut c_void) -> HRESULT;
/// `GameInputCreate()`, of the DLLs before `GameInputInitialize()`.
type GameInputCreateFn = unsafe extern "C" fn(*mut *mut c_void) -> HRESULT;

/// Load the DLL (the first time) and ask it for the interface `riid`.
/// Translation of `GameInputCreateWithVersion()`.
fn game_input_create_with_version(riid: &GUID) -> std::result::Result<*mut c_void, HRESULT> {
    let module = {
        let mut dll = GAME_INPUT_DLL.lock().unwrap_or_else(|e| e.into_inner());
        if *dll == 0 {
            *dll = load_game_input_dll()? as usize;
        }
        *dll as HMODULE
    };

    let mut ppv: *mut c_void = std::ptr::null_mut();
    // SAFETY: the module is loaded; the name is NUL-terminated.
    if let Some(proc) = unsafe { GetProcAddress(module, c"GameInputInitialize".as_ptr().cast()) } {
        // SAFETY: the export has this signature.
        let game_input_initialize: GameInputInitializeFn = unsafe { std::mem::transmute(proc) };
        // SAFETY: riid and ppv are valid.
        let hr = unsafe { game_input_initialize(riid, &mut ppv) };
        return if hr < 0 { Err(hr) } else { Ok(ppv) };
    }

    if super::is_equal_guid(riid, &IID_IGAMEINPUT_V0) {
        // All recent versions of GameInput support the GameInputInitialize export. As we
        // did not find it via above query, we must be running an old version of GameInput
        // which only supports the v0 API. Don't attempt to use it for newer API versions.

        // SAFETY: as above.
        if let Some(proc) = unsafe { GetProcAddress(module, c"GameInputCreate".as_ptr().cast()) } {
            // SAFETY: the export has this signature.
            let game_input_create: GameInputCreateFn = unsafe { std::mem::transmute(proc) };
            // SAFETY: ppv is valid.
            let hr = unsafe { game_input_create(&mut ppv) };
            return if hr < 0 { Err(hr) } else { Ok(ppv) };
        }

        return Err(hresult_from_win32(ERROR_PROC_NOT_FOUND));
    }

    Err(E_NOINTERFACE)
}

/// `GameInputCreate()` (`gameinput.h`): the v3 `IGameInput`.
fn game_input_create() -> std::result::Result<GameInput, HRESULT> {
    let ppv = game_input_create_with_version(&IID_IGAMEINPUT)?;
    // SAFETY: GameInputInitialize() stored a reference to an IGameInput.
    unsafe { ComPtr::from_raw(ppv.cast()) }
        .map(GameInput)
        .ok_or(E_POINTER)
}

// --- SDL_gameinput.cpp ---

/// `g_pGameInput` and `g_nGameInputRefCount`.
struct Global {
    game_input: Option<GameInput>,
    ref_count: u32,
}

static GLOBAL: Mutex<Global> = Mutex::new(Global {
    game_input: None,
    ref_count: 0,
});

fn global() -> std::sync::MutexGuard<'static, Global> {
    GLOBAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// One reference of [`init_game_input`] to the shared `IGameInput`; the
/// last one dropped releases it (`SDL_QuitGameInput()`).
#[derive(Debug)]
pub(crate) struct GameInputRef {
    game_input: GameInput,
}

impl std::ops::Deref for GameInputRef {
    type Target = GameInput;

    fn deref(&self) -> &GameInput {
        &self.game_input
    }
}

impl Drop for GameInputRef {
    /// Translation of `SDL_QuitGameInput()`.
    fn drop(&mut self) {
        let mut global = global();
        crate::sdl_assert!(global.ref_count > 0);

        global.ref_count = global.ref_count.saturating_sub(1);
        if global.ref_count == 0 {
            global.game_input = None;
            // (the DLL stays loaded, as gameinput.cpp leaves it)
        }
    }
}

/// Create (or share) the `IGameInput` interface. Translation of
/// `SDL_InitGameInput()`.
pub(crate) fn init_game_input() -> Result<GameInputRef> {
    let mut global = global();
    if global.ref_count == 0 {
        if has_broken_ezfrd64_dll() {
            return Err(Error::new(
                "GameInput disabled to prevent application crashing",
            ));
        }

        // This is recommended, as Microsoft's GameInputCreate() is robust
        // and better handles various GameInput installations
        match game_input_create() {
            Ok(game_input) => global.game_input = Some(game_input),
            Err(hr) => return Err(error_from_hresult(Some("GameInputCreate failed"), hr)),
        }
    }
    let Some(game_input) = global.game_input.clone() else {
        return Err(Error::new("GameInput isn't initialized"));
    };
    global.ref_count += 1;

    Ok(GameInputRef { game_input })
}

/// Whether the `IGameInput` interface exists. Translation of
/// `SDL_GameInputReady()`.
pub(crate) fn game_input_ready() -> bool {
    global().game_input.is_some()
}

/// Whether GameInput handles the XInput controllers (the other Windows
/// drivers then leave them alone). Translation of
/// `SDL_UsingGameInputForXInputControllers()`.
pub(crate) fn using_game_input_for_xinput_controllers() -> bool {
    hints::get_bool(hints::JOYSTICK_GAMEINPUT, GAMEINPUT_DEFAULT) && game_input_ready()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn vtable_slots_match_the_header() {
        // Slot numbers of gameinput.h's methods (pointer-to-member values of
        // a C++ harness built with the header).
        let ptr = size_of::<usize>();
        let slot = |offset: usize| offset / ptr;
        assert_eq!(slot(offset_of!(IGameInputVtbl, get_current_timestamp)), 3);
        assert_eq!(slot(offset_of!(IGameInputVtbl, get_current_reading)), 4);
        assert_eq!(slot(offset_of!(IGameInputVtbl, get_next_reading)), 5);
        assert_eq!(
            slot(offset_of!(IGameInputVtbl, register_device_callback)),
            8
        );
        assert_eq!(
            slot(offset_of!(IGameInputVtbl, register_system_button_callback)),
            9
        );
        assert_eq!(slot(offset_of!(IGameInputVtbl, unregister_callback)), 12);
        assert_eq!(slot(offset_of!(IGameInputVtbl, set_focus_policy)), 16);
        assert_eq!(size_of::<IGameInputVtbl>(), 19 * ptr);

        assert_eq!(slot(offset_of!(IGameInputReadingVtbl, get_timestamp)), 4);
        assert_eq!(
            slot(offset_of!(IGameInputReadingVtbl, get_controller_axis_state)),
            7
        );
        assert_eq!(
            slot(offset_of!(
                IGameInputReadingVtbl,
                get_controller_button_state
            )),
            9
        );
        assert_eq!(
            slot(offset_of!(
                IGameInputReadingVtbl,
                get_controller_switch_state
            )),
            11
        );
        assert_eq!(slot(offset_of!(IGameInputReadingVtbl, get_key_state)), 13);
        assert_eq!(slot(offset_of!(IGameInputReadingVtbl, get_mouse_state)), 14);
        assert_eq!(
            slot(offset_of!(IGameInputReadingVtbl, get_sensors_state)),
            15
        );
        assert_eq!(
            slot(offset_of!(IGameInputReadingVtbl, get_gamepad_state)),
            18
        );
        assert_eq!(slot(offset_of!(IGameInputReadingVtbl, get_raw_report)), 20);
        assert_eq!(size_of::<IGameInputReadingVtbl>(), 21 * ptr);

        assert_eq!(slot(offset_of!(IGameInputDeviceVtbl, get_device_info)), 3);
        assert_eq!(slot(offset_of!(IGameInputDeviceVtbl, get_device_status)), 5);
        assert_eq!(
            slot(offset_of!(
                IGameInputDeviceVtbl,
                is_force_feedback_motor_powered_on
            )),
            7
        );
        assert_eq!(slot(offset_of!(IGameInputDeviceVtbl, set_rumble_state)), 9);
        // (SendRawDeviceOutput is slot 17, the last)
        assert_eq!(size_of::<IGameInputDeviceVtbl>(), 18 * ptr);

        assert_eq!(
            slot(offset_of!(IGameInputRawDeviceReportVtbl, get_raw_data)),
            6
        );
        assert_eq!(size_of::<IGameInputRawDeviceReportVtbl>(), 8 * ptr);
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn structures_match_the_header() {
        // sizeof/offsetof of the C++ harness (x86_64)
        assert_eq!(size_of::<KeyState>(), 12);
        assert_eq!(offset_of!(KeyState, virtual_key), 8);
        assert_eq!(offset_of!(KeyState, is_dead_key), 9);
        assert_eq!(size_of::<MouseState>(), 56);
        assert_eq!(offset_of!(MouseState, position_x), 8);
        assert_eq!(offset_of!(MouseState, wheel_x), 40);
        assert_eq!(offset_of!(MouseState, wheel_y), 48);
        assert_eq!(size_of::<SensorsState>(), 48);
        assert_eq!(
            offset_of!(SensorsState, angular_velocity_in_rad_per_sec_x),
            12
        );
        assert_eq!(offset_of!(SensorsState, orientation_z), 44);
        assert_eq!(size_of::<GamepadState>(), 28);
        assert_eq!(offset_of!(GamepadState, right_thumbstick_y), 24);
        assert_eq!(size_of::<RumbleParams>(), 16);
        assert_eq!(size_of::<KeyboardInfo>(), 28);
        assert_eq!(offset_of!(KeyboardInfo, max_simultaneous_keys), 16);
        assert_eq!(size_of::<SensorsInfo>(), 4);
        assert_eq!(size_of::<ControllerInfo>(), 48);
        assert_eq!(offset_of!(ControllerInfo, controller_button_count), 16);
        assert_eq!(offset_of!(ControllerInfo, controller_switch_count), 32);

        assert_eq!(size_of::<DeviceInfo>(), 256);
        assert_eq!(offset_of!(DeviceInfo, usage), 6);
        assert_eq!(offset_of!(DeviceInfo, hardware_version), 10);
        assert_eq!(offset_of!(DeviceInfo, firmware_version), 18);
        assert_eq!(offset_of!(DeviceInfo, device_id), 26);
        assert_eq!(offset_of!(DeviceInfo, device_root_id), 58);
        assert_eq!(offset_of!(DeviceInfo, device_family), 92);
        assert_eq!(offset_of!(DeviceInfo, supported_input), 96);
        assert_eq!(offset_of!(DeviceInfo, supported_rumble_motors), 100);
        assert_eq!(offset_of!(DeviceInfo, supported_system_buttons), 104);
        assert_eq!(offset_of!(DeviceInfo, container_id), 108);
        assert_eq!(offset_of!(DeviceInfo, display_name), 128);
        assert_eq!(offset_of!(DeviceInfo, pnp_path), 136);
        assert_eq!(offset_of!(DeviceInfo, keyboard_info), 144);
        assert_eq!(offset_of!(DeviceInfo, sensors_info), 160);
        assert_eq!(offset_of!(DeviceInfo, controller_info), 168);
        assert_eq!(offset_of!(DeviceInfo, racing_wheel_info), 200);
        assert_eq!(offset_of!(DeviceInfo, force_feedback_motor_count), 208);
        assert_eq!(offset_of!(DeviceInfo, output_report_info), 248);
    }

    #[test]
    fn constants_match_the_header() {
        let text = |g: &GUID| {
            format!(
                "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{}",
                g.data1,
                g.data2,
                g.data3,
                g.data4[0],
                g.data4[1],
                g.data4[2..]
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<String>()
            )
        };
        // DECLARE_INTERFACE_IID_(IGameInput, IUnknown, "...")
        assert_eq!(
            text(&IID_IGAMEINPUT),
            "20EFC1C7-5D9A-43BA-B26F-B807FA48609C"
        );
        // {0x11be2a7e, 0x4254, 0x445a, {0x9c, 0x09, 0xff, 0xc4, 0x0f, 0x00, 0x69, 0x18}}
        assert_eq!(
            text(&IID_IGAMEINPUT_V0),
            "11BE2A7E-4254-445A-9C09-FFC40F006918"
        );
        assert_eq!(GAMEINPUT_E_READING_NOT_FOUND as u32, 0x838A0003);
        assert_eq!(kind::CONTROLLER, 0x2 | 0x4 | 0x8); // axis | button | switch
        assert_eq!(kind::RAW_DEVICE_REPORT, 0x1);
        assert_eq!(kind::GAMEPAD, 0x40000);
        const { assert!(GAMEINPUT_DEFAULT) };
    }

    #[test]
    fn paths_join_with_one_separator() {
        assert_eq!(path_join("", "b.dll"), "b.dll");
        assert_eq!(path_join("C:\\a", ""), "C:\\a");
        assert_eq!(path_join("C:\\a", "b.dll"), "C:\\a\\b.dll");
        assert_eq!(path_join("C:\\a\\", "b.dll"), "C:\\a\\b.dll");
        assert_eq!(path_join("C:/a/", "\\b.dll"), "C:/a/b.dll");
        assert_eq!(path_join("C:\\a", "/b.dll"), "C:\\a\\b.dll");
    }

    #[test]
    fn the_newest_dll_wins_and_later_candidates_win_ties() {
        let pick = |c: [Option<(&'static str, u64)>; 3]| select_game_input_dll(c);
        assert_eq!(pick([None, None, None]), None);
        assert_eq!(pick([Some(("inbox", 5)), None, None]), Some("inbox"));
        assert_eq!(
            pick([Some(("inbox", 5)), Some(("redist", 4)), None]),
            Some("inbox")
        );
        assert_eq!(
            pick([Some(("inbox", 5)), Some(("redist", 5)), None]),
            Some("redist")
        );
        assert_eq!(
            pick([Some(("inbox", 5)), Some(("redist", 6)), Some(("side", 6))]),
            Some("side")
        );
        assert_eq!(
            pick([Some(("inbox", 7)), Some(("redist", 6)), Some(("side", 6))]),
            Some("inbox")
        );
        assert_eq!(pick([None, None, Some(("side", 0))]), Some("side"));
    }

    #[test]
    fn hresults_from_win32_errors() {
        assert_eq!(hresult_from_win32(0), 0);
        assert_eq!(hresult_from_win32(ERROR_FILE_NOT_FOUND) as u32, 0x80070002);
        assert_eq!(hresult_from_win32(ERROR_PROC_NOT_FOUND) as u32, 0x8007007F);
    }

    #[test]
    fn device_info_parts_are_absent_when_null() {
        let info = DeviceInfo::default();
        assert_eq!(info.display_name(), None);
        assert_eq!(info.pnp_path(), None);
        assert!(info.keyboard_info().is_none());
        assert!(info.sensors_info().is_none());
        assert!(info.controller_info().is_none());
    }

    #[test]
    fn init_fails_cleanly_without_game_input() {
        let _l = crate::test_support::test_lock();
        // Wine and the Windows CI runners have no GameInput DLL; where there
        // is one, initialization is reference counted.
        match init_game_input() {
            Err(e) => {
                crate::test_support::skip("gameinput", format_args!("no GameInput here ({e})"));
                assert!(!game_input_ready());
                assert!(!using_game_input_for_xinput_controllers());
            }
            Ok(first) => {
                assert!(game_input_ready());
                let second = init_game_input().unwrap();
                drop(first);
                assert!(game_input_ready());
                let _ = second.current_timestamp();
                drop(second);
                assert!(!game_input_ready());
            }
        }
    }
}
