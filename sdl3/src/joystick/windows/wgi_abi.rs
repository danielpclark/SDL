// Rust translation of the WinRT declarations src/joystick/windows/SDL_rawinputjoystick.c
// and SDL_windows_gaming_input.c from Simple DirectMedia Layer use (from
// <windows.gaming.input.h>, <windows.devices.power.h>, <roapi.h> and
// <winstring.h>, which `windows-sys` doesn't have), with the IIDs and
// event handler objects those files define.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Hand-declared Windows.Gaming.Input ABI: the vtables (in the order of
//! the WinRT metadata, methods SDL doesn't call kept as placeholders), the
//! IIDs SDL defines itself, the structures, `HSTRING` handling through
//! `combase.dll`, and the static event handler objects.

#![allow(
    non_camel_case_types,
    non_snake_case,
    dead_code,
    clippy::upper_case_acronyms
)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::OnceLock;

use windows_sys::core::{GUID, HRESULT, PCWSTR};
use windows_sys::Win32::Foundation::{E_INVALIDARG, E_NOINTERFACE, E_OUTOFMEMORY, S_OK};

use crate::core::windows::com::{
    ComObject, ComPtr, IUnknownVtbl, IID_IAGILEOBJECT, IID_IMARSHAL, IID_IUNKNOWN,
};
use crate::core::windows::{is_equal_guid, load_combase_function, utf8_to_wide, wide_to_utf8};

/// `HSTRING`
pub(super) type HSTRING = *mut c_void;

/// `HSTRING_HEADER` (24 bytes on 64-bit Windows, 20 on 32-bit)
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct HSTRING_HEADER {
    reserved: [usize; if cfg!(target_pointer_width = "64") {
        3
    } else {
        5
    }],
}

/// `EventRegistrationToken`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct EventRegistrationToken {
    pub(super) value: i64,
}

/// `GamepadReading`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct GamepadReading {
    pub(super) Timestamp: u64,
    /// `GamepadButtons`
    pub(super) Buttons: u32,
    pub(super) LeftTrigger: f64,
    pub(super) RightTrigger: f64,
    pub(super) LeftThumbstickX: f64,
    pub(super) LeftThumbstickY: f64,
    pub(super) RightThumbstickX: f64,
    pub(super) RightThumbstickY: f64,
}

/// `GamepadVibration`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct GamepadVibration {
    pub(super) LeftMotor: f64,
    pub(super) RightMotor: f64,
    pub(super) LeftTrigger: f64,
    pub(super) RightTrigger: f64,
}

/// The guide button bit of `GamepadButtons` (an undocumented bit SDL
/// defines as `GamepadButtons_GUIDE`).
pub(super) const GAMEPAD_BUTTONS_GUIDE: u32 = 0x40000000;

/// `GameControllerSwitchPosition`
pub(super) mod switch_position {
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

/// `BatteryStatus`
pub(super) mod battery_status {
    pub(crate) const NOT_PRESENT: i32 = 0;
    pub(crate) const DISCHARGING: i32 = 1;
    pub(crate) const IDLE: i32 = 2;
    pub(crate) const CHARGING: i32 = 3;
}

/// A method SDL doesn't call (its arguments aren't declared).
type Unused = unsafe extern "system" fn();

/// `IInspectableVtbl`
#[repr(C)]
pub(super) struct IInspectableVtbl {
    pub(super) base: IUnknownVtbl,
    pub(super) GetIids: Unused,
    pub(super) GetRuntimeClassName: Unused,
    pub(super) GetTrustLevel: Unused,
}
/// `IInspectable`
pub(super) type IInspectable = ComObject<IInspectableVtbl>;

/// A typed `IEventHandler<T>` vtable.
#[repr(C)]
pub(super) struct IEventHandlerVtbl<T> {
    pub(super) QueryInterface: unsafe extern "system" fn(
        *mut ComObject<IEventHandlerVtbl<T>>,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    pub(super) AddRef: unsafe extern "system" fn(*mut ComObject<IEventHandlerVtbl<T>>) -> u32,
    pub(super) Release: unsafe extern "system" fn(*mut ComObject<IEventHandlerVtbl<T>>) -> u32,
    pub(super) Invoke: unsafe extern "system" fn(
        *mut ComObject<IEventHandlerVtbl<T>>,
        *mut IInspectable,
        *mut ComObject<T>,
    ) -> HRESULT,
}
/// `__FIEventHandler_1_Windows__CGaming__CInput__CGamepad`
pub(super) type IEventHandlerGamepad = ComObject<IEventHandlerVtbl<IGamepadVtbl>>;
/// `__FIEventHandler_1_Windows__CGaming__CInput__CRawGameController`
pub(super) type IEventHandlerRawGameController =
    ComObject<IEventHandlerVtbl<IRawGameControllerVtbl>>;

/// A typed `IVectorView<T>` vtable.
#[repr(C)]
pub(super) struct IVectorViewVtbl<T> {
    pub(super) base: IInspectableVtbl,
    pub(super) GetAt: unsafe extern "system" fn(
        *mut ComObject<IVectorViewVtbl<T>>,
        u32,
        *mut *mut ComObject<T>,
    ) -> HRESULT,
    pub(super) get_Size:
        unsafe extern "system" fn(*mut ComObject<IVectorViewVtbl<T>>, *mut u32) -> HRESULT,
    pub(super) IndexOf: Unused,
    pub(super) GetMany: Unused,
}

/// `IGamepadVtbl`
#[repr(C)]
pub(super) struct IGamepadVtbl {
    pub(super) base: IInspectableVtbl,
    pub(super) get_Vibration:
        unsafe extern "system" fn(*mut IGamepad, *mut GamepadVibration) -> HRESULT,
    pub(super) put_Vibration: unsafe extern "system" fn(*mut IGamepad, GamepadVibration) -> HRESULT,
    pub(super) GetCurrentReading:
        unsafe extern "system" fn(*mut IGamepad, *mut GamepadReading) -> HRESULT,
}
pub(super) type IGamepad = ComObject<IGamepadVtbl>;

/// `IGamepadStaticsVtbl`
#[repr(C)]
pub(super) struct IGamepadStaticsVtbl {
    pub(super) base: IInspectableVtbl,
    pub(super) add_GamepadAdded: unsafe extern "system" fn(
        *mut IGamepadStatics,
        *mut IEventHandlerGamepad,
        *mut EventRegistrationToken,
    ) -> HRESULT,
    pub(super) remove_GamepadAdded:
        unsafe extern "system" fn(*mut IGamepadStatics, EventRegistrationToken) -> HRESULT,
    pub(super) add_GamepadRemoved: unsafe extern "system" fn(
        *mut IGamepadStatics,
        *mut IEventHandlerGamepad,
        *mut EventRegistrationToken,
    ) -> HRESULT,
    pub(super) remove_GamepadRemoved:
        unsafe extern "system" fn(*mut IGamepadStatics, EventRegistrationToken) -> HRESULT,
    pub(super) get_Gamepads: unsafe extern "system" fn(
        *mut IGamepadStatics,
        *mut *mut ComObject<IVectorViewVtbl<IGamepadVtbl>>,
    ) -> HRESULT,
}
pub(super) type IGamepadStatics = ComObject<IGamepadStaticsVtbl>;

/// A `*Statics2` interface with only `FromGameController()`, which gives an
/// `R` (`IGamepadStatics2`, `IArcadeStickStatics2`, `IRacingWheelStatics2`).
#[repr(C)]
pub(super) struct IFromGameControllerVtbl {
    pub(super) base: IInspectableVtbl,
    pub(super) FromGameController: unsafe extern "system" fn(
        *mut ComObject<IFromGameControllerVtbl>,
        *mut IGameController,
        *mut *mut IInspectable,
    ) -> HRESULT,
}

/// `IFlightStickStaticsVtbl`
#[repr(C)]
pub(super) struct IFlightStickStaticsVtbl {
    pub(super) base: IInspectableVtbl,
    pub(super) add_FlightStickAdded: Unused,
    pub(super) remove_FlightStickAdded: Unused,
    pub(super) add_FlightStickRemoved: Unused,
    pub(super) remove_FlightStickRemoved: Unused,
    pub(super) get_FlightSticks: Unused,
    pub(super) FromGameController: unsafe extern "system" fn(
        *mut ComObject<IFlightStickStaticsVtbl>,
        *mut IGameController,
        *mut *mut IInspectable,
    ) -> HRESULT,
}

/// `IRawGameControllerVtbl`
#[repr(C)]
pub(super) struct IRawGameControllerVtbl {
    pub(super) base: IInspectableVtbl,
    pub(super) get_AxisCount:
        unsafe extern "system" fn(*mut IRawGameController, *mut i32) -> HRESULT,
    pub(super) get_ButtonCount:
        unsafe extern "system" fn(*mut IRawGameController, *mut i32) -> HRESULT,
    pub(super) get_ForceFeedbackMotors: Unused,
    pub(super) get_HardwareProductId:
        unsafe extern "system" fn(*mut IRawGameController, *mut u16) -> HRESULT,
    pub(super) get_HardwareVendorId:
        unsafe extern "system" fn(*mut IRawGameController, *mut u16) -> HRESULT,
    pub(super) get_SwitchCount:
        unsafe extern "system" fn(*mut IRawGameController, *mut i32) -> HRESULT,
    pub(super) GetButtonLabel: Unused,
    pub(super) GetCurrentReading: unsafe extern "system" fn(
        *mut IRawGameController,
        u32,
        *mut u8,
        u32,
        *mut i32,
        u32,
        *mut f64,
        *mut u64,
    ) -> HRESULT,
    pub(super) GetSwitchKind: Unused,
}
pub(super) type IRawGameController = ComObject<IRawGameControllerVtbl>;

/// `IRawGameController2Vtbl`
#[repr(C)]
pub(super) struct IRawGameController2Vtbl {
    pub(super) base: IInspectableVtbl,
    pub(super) get_SimpleHapticsControllers: Unused,
    pub(super) get_NonRoamableId:
        unsafe extern "system" fn(*mut IRawGameController2, *mut HSTRING) -> HRESULT,
    pub(super) get_DisplayName:
        unsafe extern "system" fn(*mut IRawGameController2, *mut HSTRING) -> HRESULT,
}
pub(super) type IRawGameController2 = ComObject<IRawGameController2Vtbl>;

/// `IRawGameControllerStaticsVtbl`
#[repr(C)]
pub(super) struct IRawGameControllerStaticsVtbl {
    pub(super) base: IInspectableVtbl,
    pub(super) add_RawGameControllerAdded: unsafe extern "system" fn(
        *mut IRawGameControllerStatics,
        *mut IEventHandlerRawGameController,
        *mut EventRegistrationToken,
    ) -> HRESULT,
    pub(super) remove_RawGameControllerAdded: unsafe extern "system" fn(
        *mut IRawGameControllerStatics,
        EventRegistrationToken,
    ) -> HRESULT,
    pub(super) add_RawGameControllerRemoved: unsafe extern "system" fn(
        *mut IRawGameControllerStatics,
        *mut IEventHandlerRawGameController,
        *mut EventRegistrationToken,
    ) -> HRESULT,
    pub(super) remove_RawGameControllerRemoved: unsafe extern "system" fn(
        *mut IRawGameControllerStatics,
        EventRegistrationToken,
    ) -> HRESULT,
    pub(super) get_RawGameControllers: unsafe extern "system" fn(
        *mut IRawGameControllerStatics,
        *mut *mut ComObject<IVectorViewVtbl<IRawGameControllerVtbl>>,
    ) -> HRESULT,
    pub(super) FromGameController: Unused,
}
pub(super) type IRawGameControllerStatics = ComObject<IRawGameControllerStaticsVtbl>;

/// `IGameControllerVtbl`
#[repr(C)]
pub(super) struct IGameControllerVtbl {
    pub(super) base: IInspectableVtbl,
    pub(super) add_HeadsetConnected: Unused,
    pub(super) remove_HeadsetConnected: Unused,
    pub(super) add_HeadsetDisconnected: Unused,
    pub(super) remove_HeadsetDisconnected: Unused,
    pub(super) add_UserChanged: Unused,
    pub(super) remove_UserChanged: Unused,
    pub(super) get_Headset: Unused,
    pub(super) get_IsWireless: unsafe extern "system" fn(*mut IGameController, *mut u8) -> HRESULT,
    pub(super) get_User: Unused,
}
pub(super) type IGameController = ComObject<IGameControllerVtbl>;

/// `IGameControllerBatteryInfoVtbl`
#[repr(C)]
pub(super) struct IGameControllerBatteryInfoVtbl {
    pub(super) base: IInspectableVtbl,
    pub(super) TryGetBatteryReport: unsafe extern "system" fn(
        *mut IGameControllerBatteryInfo,
        *mut *mut IBatteryReport,
    ) -> HRESULT,
}
pub(super) type IGameControllerBatteryInfo = ComObject<IGameControllerBatteryInfoVtbl>;

/// `IBatteryReportVtbl` (Windows.Devices.Power)
#[repr(C)]
pub(super) struct IBatteryReportVtbl {
    pub(super) base: IInspectableVtbl,
    pub(super) get_ChargeRateInMilliwatts: Unused,
    pub(super) get_DesignCapacityInMilliwattHours: Unused,
    pub(super) get_FullChargeCapacityInMilliwattHours:
        unsafe extern "system" fn(*mut IBatteryReport, *mut *mut IReferenceInt32) -> HRESULT,
    pub(super) get_RemainingCapacityInMilliwattHours:
        unsafe extern "system" fn(*mut IBatteryReport, *mut *mut IReferenceInt32) -> HRESULT,
    pub(super) get_Status: unsafe extern "system" fn(*mut IBatteryReport, *mut i32) -> HRESULT,
}
pub(super) type IBatteryReport = ComObject<IBatteryReportVtbl>;

/// `__FIReference_1_INT32Vtbl` (`__FIReference_1_int`)
#[repr(C)]
pub(super) struct IReferenceInt32Vtbl {
    pub(super) base: IInspectableVtbl,
    pub(super) get_Value: unsafe extern "system" fn(*mut IReferenceInt32, *mut i32) -> HRESULT,
}
pub(super) type IReferenceInt32 = ComObject<IReferenceInt32Vtbl>;

/// A GUID from its parts.
const fn guid(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> GUID {
    GUID {
        data1,
        data2,
        data3,
        data4,
    }
}

// WinRT headers in official Windows SDK contain only declarations, and we have to define these GUIDs ourselves.
// https://stackoverflow.com/a/55605485/1795050
pub(super) const IID_IEVENTHANDLER_RAWGAMECONTROLLER: GUID = guid(
    0x00621c22,
    0x42e8,
    0x529f,
    [0x92, 0x70, 0x83, 0x6b, 0x32, 0x93, 0x1d, 0x72],
);
pub(super) const IID_IEVENTHANDLER_GAMEPAD: GUID = guid(
    0x8a7639ee,
    0x624a,
    0x501a,
    [0xbb, 0x53, 0x56, 0x2d, 0x1e, 0xc1, 0x1b, 0x52],
);
pub(super) const IID_IARCADESTICKSTATICS: GUID = guid(
    0x5c37b8c8,
    0x37b1,
    0x4ad8,
    [0x94, 0x58, 0x20, 0x0f, 0x1a, 0x30, 0x01, 0x8e],
);
pub(super) const IID_IARCADESTICKSTATICS2: GUID = guid(
    0x52b5d744,
    0xbb86,
    0x445a,
    [0xb5, 0x9c, 0x59, 0x6f, 0x0e, 0x2a, 0x49, 0xdf],
);
pub(super) const IID_IFLIGHTSTICKSTATICS: GUID = guid(
    0x5514924a,
    0xfecc,
    0x435e,
    [0x83, 0xdc, 0x5c, 0xec, 0x8a, 0x18, 0xa5, 0x20],
);
pub(super) const IID_IGAMECONTROLLER: GUID = guid(
    0x1baf6522,
    0x5f64,
    0x42c5,
    [0x82, 0x67, 0xb9, 0xfe, 0x22, 0x15, 0xbf, 0xbd],
);
pub(super) const IID_IGAMECONTROLLERBATTERYINFO: GUID = guid(
    0xdcecc681,
    0x3963,
    0x4da6,
    [0x95, 0x5d, 0x55, 0x3f, 0x3b, 0x6f, 0x61, 0x61],
);
pub(super) const IID_IGAMEPADSTATICS: GUID = guid(
    0x8bbce529,
    0xd49c,
    0x39e9,
    [0x95, 0x60, 0xe4, 0x7d, 0xde, 0x96, 0xb7, 0xc8],
);
pub(super) const IID_IGAMEPADSTATICS2: GUID = guid(
    0x42676dc5,
    0x0856,
    0x47c4,
    [0x92, 0x13, 0xb3, 0x95, 0x50, 0x4c, 0x3a, 0x3c],
);
pub(super) const IID_IRACINGWHEELSTATICS: GUID = guid(
    0x3ac12cd5,
    0x581b,
    0x4936,
    [0x9f, 0x94, 0x69, 0xf1, 0xe6, 0x51, 0x4c, 0x7d],
);
pub(super) const IID_IRACINGWHEELSTATICS2: GUID = guid(
    0xe666bcaa,
    0xedfd,
    0x4323,
    [0xa9, 0xf6, 0x3c, 0x38, 0x40, 0x48, 0xd1, 0xed],
);
pub(super) const IID_IRAWGAMECONTROLLER: GUID = guid(
    0x7cad6d91,
    0xa7e1,
    0x4f71,
    [0x9a, 0x78, 0x33, 0xe9, 0xc5, 0xdf, 0xea, 0x62],
);
pub(super) const IID_IRAWGAMECONTROLLER2: GUID = guid(
    0x43c0c035,
    0xbb73,
    0x4756,
    [0xa7, 0x87, 0x3e, 0xd6, 0xbe, 0xa6, 0x17, 0xbd],
);
pub(super) const IID_IRAWGAMECONTROLLERSTATICS: GUID = guid(
    0xeb8d0792,
    0xe95a,
    0x4b19,
    [0xaf, 0xc7, 0x0a, 0x59, 0xf8, 0xbf, 0x75, 0x9e],
);

/// `RuntimeClass_Windows_Gaming_Input_*`
pub(super) const RUNTIME_CLASS_RAW_GAME_CONTROLLER: &str = "Windows.Gaming.Input.RawGameController";
pub(super) const RUNTIME_CLASS_ARCADE_STICK: &str = "Windows.Gaming.Input.ArcadeStick";
pub(super) const RUNTIME_CLASS_FLIGHT_STICK: &str = "Windows.Gaming.Input.FlightStick";
pub(super) const RUNTIME_CLASS_GAMEPAD: &str = "Windows.Gaming.Input.Gamepad";
pub(super) const RUNTIME_CLASS_RACING_WHEEL: &str = "Windows.Gaming.Input.RacingWheel";

type CoIncrementMTAUsage_t = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;
type RoGetActivationFactory_t =
    unsafe extern "system" fn(HSTRING, *const GUID, *mut *mut c_void) -> HRESULT;
type WindowsCreateStringReference_t =
    unsafe extern "system" fn(PCWSTR, u32, *mut HSTRING_HEADER, *mut HSTRING) -> HRESULT;
type WindowsDeleteString_t = unsafe extern "system" fn(HSTRING) -> HRESULT;
type WindowsGetStringRawBuffer_t = unsafe extern "system" fn(HSTRING, *mut u32) -> PCWSTR;

/// The combase functions the drivers load (`WIN_LoadComBaseFunction()`).
#[derive(Clone, Copy, Debug)]
pub(super) struct WinRt {
    pub(super) co_increment_mta_usage: Option<CoIncrementMTAUsage_t>,
    ro_get_activation_factory: Option<RoGetActivationFactory_t>,
    windows_create_string_reference: Option<WindowsCreateStringReference_t>,
    windows_delete_string: Option<WindowsDeleteString_t>,
    windows_get_string_raw_buffer: Option<WindowsGetStringRawBuffer_t>,
}

/// Load a combase function as type `T` (a function pointer type).
///
/// # Safety
///
/// `T` must be the function's prototype.
unsafe fn combase<T: Copy>(name: &std::ffi::CStr) -> Option<T> {
    let f = load_combase_function(name)?;
    // SAFETY: `T` is a function pointer type of the function's prototype
    // (the caller's contract), as big as the generic one.
    Some(unsafe { std::mem::transmute_copy::<unsafe extern "system" fn() -> isize, T>(&f) })
}

/// The combase functions (loaded once).
pub(super) fn winrt() -> WinRt {
    static WINRT: OnceLock<WinRt> = OnceLock::new();
    *WINRT.get_or_init(|| {
        // SAFETY: each function is loaded with its prototype.
        unsafe {
            WinRt {
                co_increment_mta_usage: combase(c"CoIncrementMTAUsage"),
                ro_get_activation_factory: combase(c"RoGetActivationFactory"),
                windows_create_string_reference: combase(c"WindowsCreateStringReference"),
                windows_delete_string: combase(c"WindowsDeleteString"),
                windows_get_string_raw_buffer: combase(c"WindowsGetStringRawBuffer"),
            }
        }
    })
}

impl WinRt {
    /// Whether the string and activation functions are there.
    pub(super) fn has_activation(&self) -> bool {
        self.ro_get_activation_factory.is_some() && self.windows_create_string_reference.is_some()
    }

    /// Whether every function the WGI driver resolves is there (its
    /// `RESOLVE()`s), or the name of the first missing one.
    pub(super) fn resolve_all(&self) -> Result<(), &'static str> {
        if self.co_increment_mta_usage.is_none() {
            return Err("CoIncrementMTAUsage");
        }
        if self.ro_get_activation_factory.is_none() {
            return Err("RoGetActivationFactory");
        }
        if self.windows_create_string_reference.is_none() {
            return Err("WindowsCreateStringReference");
        }
        if self.windows_delete_string.is_none() {
            return Err("WindowsDeleteString");
        }
        if self.windows_get_string_raw_buffer.is_none() {
            return Err("WindowsGetStringRawBuffer");
        }
        Ok(())
    }

    /// `WindowsCreateStringReference()` for `class_name`, then
    /// `RoGetActivationFactory()` for its interface `iid`.
    pub(super) fn get_activation_factory<V>(
        &self,
        class_name: &str,
        iid: &GUID,
    ) -> Result<ComPtr<V>, HRESULT> {
        let (Some(create_string_reference), Some(get_activation_factory)) = (
            self.windows_create_string_reference,
            self.ro_get_activation_factory,
        ) else {
            return Err(E_NOINTERFACE);
        };
        let name = utf8_to_wide(class_name);
        // SAFETY: a header is plain data that WindowsCreateStringReference
        // fills in.
        let mut header: HSTRING_HEADER = unsafe { std::mem::zeroed() };
        let mut hstring: HSTRING = std::ptr::null_mut();
        // SAFETY: name is NUL-terminated and outlives the reference, as the
        // header (on this stack frame) does.
        let hr = unsafe {
            create_string_reference(
                name.as_ptr(),
                (name.len() - 1) as u32,
                &mut header,
                &mut hstring,
            )
        };
        if hr < 0 {
            return Err(hr);
        }
        // SAFETY: RoGetActivationFactory stores a reference to the factory's
        // `iid` interface on success; the caller names its vtable.
        unsafe {
            ComPtr::from_out(|out: *mut *mut ComObject<V>| {
                get_activation_factory(hstring, iid, out.cast())
            })
        }
    }

    /// The text of an owned `HSTRING`, which this deletes
    /// (`WindowsGetStringRawBuffer()`, `WIN_StringToUTF8W()` and
    /// `WindowsDeleteString()`).
    ///
    /// # Safety
    ///
    /// `hstring` must be an owned HSTRING (or NULL, the empty string).
    pub(super) unsafe fn take_string(&self, hstring: HSTRING) -> Option<String> {
        let get_raw_buffer = self.windows_get_string_raw_buffer?;
        // SAFETY: the string is valid until deleted below; the buffer is
        // NUL-terminated.
        let text = unsafe {
            let string = get_raw_buffer(hstring, std::ptr::null_mut());
            (!string.is_null()).then(|| {
                let mut n = 0;
                while *string.add(n) != 0 {
                    n += 1;
                }
                wide_to_utf8(std::slice::from_raw_parts(string, n))
            })
        };
        if let Some(delete_string) = self.windows_delete_string {
            // SAFETY: the caller gave us the string to delete.
            unsafe {
                delete_string(hstring);
            }
        }
        text
    }
}

/// A static event handler object (`GamepadDelegate`,
/// `RawGameControllerDelegate`): a vtable and a reference count, never
/// freed.
#[repr(C)]
pub(super) struct Delegate<T: 'static> {
    vtbl: &'static IEventHandlerVtbl<T>,
    refcount: AtomicI32,
}

impl<T: 'static> Delegate<T> {
    pub(super) const fn new(vtbl: &'static IEventHandlerVtbl<T>) -> Delegate<T> {
        Delegate {
            vtbl,
            refcount: AtomicI32::new(1),
        }
    }

    /// The interface pointer to hand to WinRT (`&delegate.iface`); the
    /// object only changes through its atomic reference count.
    pub(super) fn iface(&'static self) -> *mut ComObject<IEventHandlerVtbl<T>> {
        (self as *const Delegate<T>).cast_mut().cast()
    }
}

/// `IEventHandler_*Vtbl_QueryInterface()` for an event handler with the
/// IID `iid`.
///
/// # Safety
///
/// `this` must be a [`Delegate`]; `riid` must be valid and `ppv_object`
/// NULL or writable.
pub(super) unsafe fn delegate_query_interface<T: 'static>(
    this: *mut ComObject<IEventHandlerVtbl<T>>,
    riid: *const GUID,
    ppv_object: *mut *mut c_void,
    iid: &GUID,
) -> HRESULT {
    if ppv_object.is_null() {
        return E_INVALIDARG;
    }

    // SAFETY: the caller's contract.
    unsafe {
        *ppv_object = std::ptr::null_mut();
        let riid = &*riid;
        if is_equal_guid(riid, &IID_IUNKNOWN)
            || is_equal_guid(riid, &IID_IAGILEOBJECT)
            || is_equal_guid(riid, iid)
        {
            *ppv_object = this.cast();
            delegate_add_ref(this);
            S_OK
        } else if is_equal_guid(riid, &IID_IMARSHAL) {
            // This seems complicated. Let's hope it doesn't happen.
            E_OUTOFMEMORY
        } else {
            E_NOINTERFACE
        }
    }
}

/// `IEventHandler_*Vtbl_AddRef()`
///
/// # Safety
///
/// `this` must be a [`Delegate`].
pub(super) unsafe extern "system" fn delegate_add_ref<T: 'static>(
    this: *mut ComObject<IEventHandlerVtbl<T>>,
) -> u32 {
    // SAFETY: the caller's contract.
    let delegate = unsafe { &*this.cast::<Delegate<T>>() };
    (delegate.refcount.fetch_add(1, Ordering::AcqRel) + 1) as u32
}

/// `IEventHandler_*Vtbl_Release()`
///
/// # Safety
///
/// `this` must be a [`Delegate`].
pub(super) unsafe extern "system" fn delegate_release<T: 'static>(
    this: *mut ComObject<IEventHandlerVtbl<T>>,
) -> u32 {
    // SAFETY: the caller's contract.
    let delegate = unsafe { &*this.cast::<Delegate<T>>() };
    let rc = delegate.refcount.fetch_sub(1, Ordering::AcqRel) - 1;
    // Should never free the static delegate objects
    crate::sdl_assert!(rc > 0);
    rc as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_match_the_headers() {
        // (values from mingw-w64's windows.gaming.input.h, checked with a C program)
        assert_eq!(size_of::<GamepadReading>(), 64);
        assert_eq!(std::mem::offset_of!(GamepadReading, LeftTrigger), 16);
        assert_eq!(size_of::<GamepadVibration>(), 32);
        let ptr = size_of::<usize>();
        assert_eq!(size_of::<IInspectableVtbl>(), 6 * ptr);
        assert_eq!(size_of::<IGamepadVtbl>(), 9 * ptr);
        assert_eq!(size_of::<IGamepadStaticsVtbl>(), 11 * ptr);
        assert_eq!(size_of::<IRawGameControllerVtbl>(), 15 * ptr);
        assert_eq!(size_of::<IRawGameController2Vtbl>(), 9 * ptr);
        assert_eq!(size_of::<IRawGameControllerStaticsVtbl>(), 12 * ptr);
        assert_eq!(size_of::<IGameControllerVtbl>(), 15 * ptr);
        assert_eq!(size_of::<IFlightStickStaticsVtbl>(), 12 * ptr);
        assert_eq!(size_of::<IBatteryReportVtbl>(), 11 * ptr);
        assert_eq!(size_of::<IVectorViewVtbl<IGamepadVtbl>>(), 10 * ptr);
        assert_eq!(size_of::<IEventHandlerVtbl<IGamepadVtbl>>(), 4 * ptr);
        assert_eq!(
            size_of::<HSTRING_HEADER>(),
            if cfg!(target_pointer_width = "64") {
                24
            } else {
                20
            }
        );
    }

    #[test]
    fn activation_without_crashing() {
        // Wine has (some of) Windows.Gaming.Input; either way this must not
        // fail other than by HRESULT.
        let winrt = winrt();
        if !winrt.has_activation() {
            return;
        }
        let hr = crate::core::windows::ro_initialize();
        if hr < 0 {
            return;
        }
        let statics = winrt.get_activation_factory::<IGamepadStaticsVtbl>(
            RUNTIME_CLASS_GAMEPAD,
            &IID_IGAMEPADSTATICS,
        );
        if let Ok(statics) = statics {
            // SAFETY: get_Gamepads stores a vector view on success.
            let gamepads = unsafe {
                ComPtr::from_out(|out| (statics.vtbl().get_Gamepads)(statics.as_ptr(), out))
            };
            if let Ok(gamepads) = gamepads {
                let mut size = u32::MAX;
                // SAFETY: get_Size writes the count.
                let hr = unsafe { (gamepads.vtbl().get_Size)(gamepads.as_ptr(), &mut size) };
                if hr >= 0 {
                    assert_eq!(size, 0, "no gamepads here");
                }
            }
        }
        crate::core::windows::ro_uninitialize();
    }
}
