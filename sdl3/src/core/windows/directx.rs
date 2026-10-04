// Rust translation of src/core/windows/SDL_directx.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The DirectInput 8 declarations SDL uses (`SDL_directx.h` includes
//! `<dinput.h>` with `DIRECTINPUT_VERSION` 0x0800; `windows-sys` has none of
//! it), written by hand: the structures, constants and GUIDs, and the
//! vtables of `IDirectInput8W`, `IDirectInputDevice8W` and
//! `IDirectInputEffect`. DirectInput itself is created through COM
//! (`CoCreateInstance(CLSID_DirectInput8)`), which loads `dinput8.dll` at
//! run time, as upstream does.

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use std::ffi::c_void;

use windows_sys::core::{BOOL, GUID, HRESULT, PCWSTR};
use windows_sys::Win32::Foundation::{HANDLE, HINSTANCE, HWND};
use windows_sys::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

use super::com::{ComObject, ComPtr, IUnknownVtbl};

/// `DIRECTINPUT_VERSION`
pub(crate) const DIRECTINPUT_VERSION: u32 = 0x0800;

/// A DirectInput GUID with the `{xxxxxxxx-C9F3-11CF-BFC7-444553540000}`
/// tail of the object types.
const fn object_guid(data1: u32) -> GUID {
    GUID {
        data1,
        data2: 0xC9F3,
        data3: 0x11CF,
        data4: [0xBF, 0xC7, 0x44, 0x45, 0x53, 0x54, 0x00, 0x00],
    }
}

/// A DirectInput GUID with the `{13541Cxx-8E33-11D0-9AD0-00A0C9A06E35}`
/// tail of the predefined effects.
const fn effect_guid(data1: u32) -> GUID {
    GUID {
        data1,
        data2: 0x8E33,
        data3: 0x11D0,
        data4: [0x9A, 0xD0, 0x00, 0xA0, 0xC9, 0xA0, 0x6E, 0x35],
    }
}

/// `CLSID_DirectInput8`
pub(crate) const CLSID_DIRECTINPUT8: GUID = GUID {
    data1: 0x25E609E4,
    data2: 0xB259,
    data3: 0x11CF,
    data4: [0xBF, 0xC7, 0x44, 0x45, 0x53, 0x54, 0x00, 0x00],
};
/// `IID_IDirectInput8W` (`IID_IDirectInput8` in a Unicode build)
pub(crate) const IID_IDIRECTINPUT8W: GUID = GUID {
    data1: 0xBF798031,
    data2: 0x483A,
    data3: 0x4DA2,
    data4: [0xAA, 0x99, 0x5D, 0x64, 0xED, 0x36, 0x97, 0x00],
};

pub(crate) const GUID_XAXIS: GUID = object_guid(0xA36D02E0);
pub(crate) const GUID_YAXIS: GUID = object_guid(0xA36D02E1);
pub(crate) const GUID_ZAXIS: GUID = object_guid(0xA36D02E2);
pub(crate) const GUID_RXAXIS: GUID = object_guid(0xA36D02F4);
pub(crate) const GUID_RYAXIS: GUID = object_guid(0xA36D02F5);
pub(crate) const GUID_RZAXIS: GUID = object_guid(0xA36D02E3);
pub(crate) const GUID_SLIDER: GUID = object_guid(0xA36D02E4);
pub(crate) const GUID_POV: GUID = object_guid(0xA36D02F2);

pub(crate) const GUID_CONSTANTFORCE: GUID = effect_guid(0x13541C20);
pub(crate) const GUID_RAMPFORCE: GUID = effect_guid(0x13541C21);
pub(crate) const GUID_SQUARE: GUID = effect_guid(0x13541C22);
pub(crate) const GUID_SINE: GUID = effect_guid(0x13541C23);
pub(crate) const GUID_TRIANGLE: GUID = effect_guid(0x13541C24);
pub(crate) const GUID_SAWTOOTHUP: GUID = effect_guid(0x13541C25);
pub(crate) const GUID_SAWTOOTHDOWN: GUID = effect_guid(0x13541C26);
pub(crate) const GUID_SPRING: GUID = effect_guid(0x13541C27);
pub(crate) const GUID_DAMPER: GUID = effect_guid(0x13541C28);
pub(crate) const GUID_INERTIA: GUID = effect_guid(0x13541C29);
pub(crate) const GUID_FRICTION: GUID = effect_guid(0x13541C2A);
pub(crate) const GUID_CUSTOMFORCE: GUID = effect_guid(0x13541C2B);

// Return codes
pub(crate) const DI_OK: HRESULT = 0;
pub(crate) const DI_BUFFEROVERFLOW: HRESULT = 1; // S_FALSE
pub(crate) const DI_POLLEDDEVICE: HRESULT = 2;
/// `HRESULT_FROM_WIN32(ERROR_READ_FAULT)`
pub(crate) const DIERR_INPUTLOST: HRESULT = 0x8007_001E_u32 as HRESULT;
/// `HRESULT_FROM_WIN32(ERROR_INVALID_ACCESS)`
pub(crate) const DIERR_NOTACQUIRED: HRESULT = 0x8007_000C_u32 as HRESULT;
/// `MAKE_DIHRESULT(0x205)`
pub(crate) const DIERR_NOTEXCLUSIVEACQUIRED: HRESULT = 0x8004_0205_u32 as HRESULT;

// IDirectInput8::EnumDevices
pub(crate) const DI8DEVCLASS_ALL: u32 = 0;
pub(crate) const DI8DEVCLASS_DEVICE: u32 = 1;
pub(crate) const DI8DEVCLASS_POINTER: u32 = 2;
pub(crate) const DI8DEVCLASS_KEYBOARD: u32 = 3;
pub(crate) const DI8DEVCLASS_GAMECTRL: u32 = 4;
pub(crate) const DIEDFL_ATTACHEDONLY: u32 = 0x0000_0001;
pub(crate) const DIEDFL_FORCEFEEDBACK: u32 = 0x0000_0100;
pub(crate) const DIENUM_STOP: BOOL = 0;
pub(crate) const DIENUM_CONTINUE: BOOL = 1;
pub(crate) const DIDEVTYPE_HID: u32 = 0x0001_0000;

// DIDEVCAPS.dwFlags
pub(crate) const DIDC_ATTACHED: u32 = 0x0000_0001;
pub(crate) const DIDC_FORCEFEEDBACK: u32 = 0x0000_0100;

// Object types
pub(crate) const DIDFT_ALL: u32 = 0x0000_0000;
pub(crate) const DIDFT_AXIS: u32 = 0x0000_0003;
pub(crate) const DIDFT_BUTTON: u32 = 0x0000_000C;
pub(crate) const DIDFT_POV: u32 = 0x0000_0010;
pub(crate) const DIDFT_ANYINSTANCE: u32 = 0x00FF_FF00;
pub(crate) const DIDFT_OPTIONAL: u32 = 0x8000_0000;
pub(crate) const DIDOI_FFACTUATOR: u32 = 0x0000_0001;
pub(crate) const DIDOI_ASPECTPOSITION: u32 = 0x0000_0100;
pub(crate) const DIDOI_ASPECTVELOCITY: u32 = 0x0000_0200;
pub(crate) const DIDOI_ASPECTACCEL: u32 = 0x0000_0300;
pub(crate) const DIDOI_ASPECTFORCE: u32 = 0x0000_0400;
pub(crate) const DIDF_ABSAXIS: u32 = 0x0000_0001;

// Cooperative levels
pub(crate) const DISCL_EXCLUSIVE: u32 = 0x0000_0001;
pub(crate) const DISCL_BACKGROUND: u32 = 0x0000_0008;

// Properties: DIPROP_* are small integers cast to REFGUID (MAKEDIPROP())
pub(crate) const DIPROP_BUFFERSIZE: usize = 1;
pub(crate) const DIPROP_RANGE: usize = 4;
pub(crate) const DIPROP_DEADZONE: usize = 5;
pub(crate) const DIPROP_FFGAIN: usize = 7;
pub(crate) const DIPROP_AUTOCENTER: usize = 9;
pub(crate) const DIPROP_GUIDANDPATH: usize = 12;
pub(crate) const DIPROP_PRODUCTNAME: usize = 14;
pub(crate) const DIPROP_VIDPID: usize = 24;
pub(crate) const DIPH_DEVICE: u32 = 0;
pub(crate) const DIPH_BYID: u32 = 2;
pub(crate) const DIPROPAUTOCENTER_OFF: u32 = 0;
pub(crate) const DIPROPAUTOCENTER_ON: u32 = 1;

// Force feedback
pub(crate) const DISFFC_RESET: u32 = 0x0000_0001;
pub(crate) const DISFFC_STOPALL: u32 = 0x0000_0002;
pub(crate) const DISFFC_PAUSE: u32 = 0x0000_0004;
pub(crate) const DISFFC_CONTINUE: u32 = 0x0000_0008;
pub(crate) const DISFFC_SETACTUATORSON: u32 = 0x0000_0010;
pub(crate) const DIEFT_ALL: u32 = 0x0000_0000;
pub(crate) const DIEFF_OBJECTOFFSETS: u32 = 0x0000_0002;
pub(crate) const DIEFF_CARTESIAN: u32 = 0x0000_0010;
pub(crate) const DIEFF_POLAR: u32 = 0x0000_0020;
pub(crate) const DIEFF_SPHERICAL: u32 = 0x0000_0040;
pub(crate) const DIEP_DURATION: u32 = 0x0000_0001;
pub(crate) const DIEP_TRIGGERBUTTON: u32 = 0x0000_0008;
pub(crate) const DIEP_TRIGGERREPEATINTERVAL: u32 = 0x0000_0010;
pub(crate) const DIEP_DIRECTION: u32 = 0x0000_0040;
pub(crate) const DIEP_ENVELOPE: u32 = 0x0000_0080;
pub(crate) const DIEP_TYPESPECIFICPARAMS: u32 = 0x0000_0100;
pub(crate) const DIEP_STARTDELAY: u32 = 0x0000_0200;
pub(crate) const DIEB_NOTRIGGER: u32 = 0xFFFF_FFFF;

/// `MAX_PATH`, the size of the DirectInput string buffers.
pub(crate) const MAX_PATH: usize = 260;

/// `DIDEVICEINSTANCEW`
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct DIDEVICEINSTANCEW {
    pub(crate) dwSize: u32,
    pub(crate) guidInstance: GUID,
    pub(crate) guidProduct: GUID,
    pub(crate) dwDevType: u32,
    pub(crate) tszInstanceName: [u16; MAX_PATH],
    pub(crate) tszProductName: [u16; MAX_PATH],
    pub(crate) guidFFDriver: GUID,
    pub(crate) wUsagePage: u16,
    pub(crate) wUsage: u16,
}

impl DIDEVICEINSTANCEW {
    /// An empty structure with its size set.
    pub(crate) fn new() -> DIDEVICEINSTANCEW {
        // SAFETY: the structure is plain data; all zeros is valid.
        let mut instance: DIDEVICEINSTANCEW = unsafe { std::mem::zeroed() };
        instance.dwSize = size_of::<DIDEVICEINSTANCEW>() as u32;
        instance
    }

    /// The structure's bytes, which upstream compares with `SDL_memcmp()`.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        // SAFETY: the structure is plain data without padding (all fields
        // are 2- or 4-aligned and fill it exactly), so every byte is
        // initialized.
        unsafe {
            std::slice::from_raw_parts(
                (self as *const DIDEVICEINSTANCEW).cast::<u8>(),
                size_of::<DIDEVICEINSTANCEW>(),
            )
        }
    }
}

/// `DIDEVCAPS`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DIDEVCAPS {
    pub(crate) dwSize: u32,
    pub(crate) dwFlags: u32,
    pub(crate) dwDevType: u32,
    pub(crate) dwAxes: u32,
    pub(crate) dwButtons: u32,
    pub(crate) dwPOVs: u32,
    pub(crate) dwFFSamplePeriod: u32,
    pub(crate) dwFFMinTimeResolution: u32,
    pub(crate) dwFirmwareRevision: u32,
    pub(crate) dwHardwareRevision: u32,
    pub(crate) dwFFDriverVersion: u32,
}

/// `DIPROPHEADER`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DIPROPHEADER {
    pub(crate) dwSize: u32,
    pub(crate) dwHeaderSize: u32,
    pub(crate) dwObj: u32,
    pub(crate) dwHow: u32,
}

impl DIPROPHEADER {
    /// The header of a property structure `T`.
    pub(crate) fn of<T>(dw_obj: u32, dw_how: u32) -> DIPROPHEADER {
        DIPROPHEADER {
            dwSize: size_of::<T>() as u32,
            dwHeaderSize: size_of::<DIPROPHEADER>() as u32,
            dwObj: dw_obj,
            dwHow: dw_how,
        }
    }
}

/// `DIPROPDWORD`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct DIPROPDWORD {
    pub(crate) diph: DIPROPHEADER,
    pub(crate) dwData: u32,
}

impl DIPROPDWORD {
    /// A device-wide (`DIPH_DEVICE`) value.
    pub(crate) fn device(dw_data: u32) -> DIPROPDWORD {
        DIPROPDWORD {
            diph: DIPROPHEADER::of::<DIPROPDWORD>(0, DIPH_DEVICE),
            dwData: dw_data,
        }
    }
}

/// `DIPROPRANGE`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct DIPROPRANGE {
    pub(crate) diph: DIPROPHEADER,
    pub(crate) lMin: i32,
    pub(crate) lMax: i32,
}

/// `DIPROPSTRING`
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct DIPROPSTRING {
    pub(crate) diph: DIPROPHEADER,
    pub(crate) wsz: [u16; MAX_PATH],
}

/// `DIPROPGUIDANDPATH`
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct DIPROPGUIDANDPATH {
    pub(crate) diph: DIPROPHEADER,
    pub(crate) guidClass: GUID,
    pub(crate) wszPath: [u16; MAX_PATH],
}

/// `DIOBJECTDATAFORMAT`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct DIOBJECTDATAFORMAT {
    pub(crate) pguid: *const GUID,
    pub(crate) dwOfs: u32,
    pub(crate) dwType: u32,
    pub(crate) dwFlags: u32,
}

/// `DIDATAFORMAT`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct DIDATAFORMAT {
    pub(crate) dwSize: u32,
    pub(crate) dwObjSize: u32,
    pub(crate) dwFlags: u32,
    pub(crate) dwDataSize: u32,
    pub(crate) dwNumObjs: u32,
    pub(crate) rgodf: *const DIOBJECTDATAFORMAT,
}

/// `DIJOYSTATE2`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct DIJOYSTATE2 {
    pub(crate) lX: i32,
    pub(crate) lY: i32,
    pub(crate) lZ: i32,
    pub(crate) lRx: i32,
    pub(crate) lRy: i32,
    pub(crate) lRz: i32,
    pub(crate) rglSlider: [i32; 2],
    pub(crate) rgdwPOV: [u32; 4],
    pub(crate) rgbButtons: [u8; 128],
    pub(crate) lVX: i32,
    pub(crate) lVY: i32,
    pub(crate) lVZ: i32,
    pub(crate) lVRx: i32,
    pub(crate) lVRy: i32,
    pub(crate) lVRz: i32,
    pub(crate) rglVSlider: [i32; 2],
    pub(crate) lAX: i32,
    pub(crate) lAY: i32,
    pub(crate) lAZ: i32,
    pub(crate) lARx: i32,
    pub(crate) lARy: i32,
    pub(crate) lARz: i32,
    pub(crate) rglASlider: [i32; 2],
    pub(crate) lFX: i32,
    pub(crate) lFY: i32,
    pub(crate) lFZ: i32,
    pub(crate) lFRx: i32,
    pub(crate) lFRy: i32,
    pub(crate) lFRz: i32,
    pub(crate) rglFSlider: [i32; 2],
}

impl Default for DIJOYSTATE2 {
    fn default() -> DIJOYSTATE2 {
        // SAFETY: the structure is plain integers; all zeros is valid.
        unsafe { std::mem::zeroed() }
    }
}

// Offsets into DIJOYSTATE2 (`DIJOFS_*`).
pub(crate) const DIJOFS_X: u32 = 0;
pub(crate) const DIJOFS_Y: u32 = 4;
pub(crate) const DIJOFS_Z: u32 = 8;
pub(crate) const DIJOFS_RX: u32 = 12;
pub(crate) const DIJOFS_RY: u32 = 16;
pub(crate) const DIJOFS_RZ: u32 = 20;
/// `DIJOFS_SLIDER(n)`
pub(crate) const fn dijofs_slider(n: u32) -> u32 {
    24 + n * 4
}
/// `DIJOFS_POV(n)`
pub(crate) const fn dijofs_pov(n: u32) -> u32 {
    32 + n * 4
}
/// `DIJOFS_BUTTON0`
pub(crate) const DIJOFS_BUTTON0: u32 = 48;
/// `DIJOFS_BUTTON(n)`
pub(crate) const fn dijofs_button(n: u32) -> u32 {
    DIJOFS_BUTTON0 + n
}

/// `DIDEVICEOBJECTDATA`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DIDEVICEOBJECTDATA {
    pub(crate) dwOfs: u32,
    pub(crate) dwData: u32,
    pub(crate) dwTimeStamp: u32,
    pub(crate) dwSequence: u32,
    pub(crate) uAppData: usize,
}

/// `DIDEVICEOBJECTINSTANCEW`
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct DIDEVICEOBJECTINSTANCEW {
    pub(crate) dwSize: u32,
    pub(crate) guidType: GUID,
    pub(crate) dwOfs: u32,
    pub(crate) dwType: u32,
    pub(crate) dwFlags: u32,
    pub(crate) tszName: [u16; MAX_PATH],
    pub(crate) dwFFMaxForce: u32,
    pub(crate) dwFFForceResolution: u32,
    pub(crate) wCollectionNumber: u16,
    pub(crate) wDesignatorIndex: u16,
    pub(crate) wUsagePage: u16,
    pub(crate) wUsage: u16,
    pub(crate) dwDimension: u32,
    pub(crate) wExponent: u16,
    pub(crate) wReportId: u16,
}

/// `DIEFFECTINFOW`
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct DIEFFECTINFOW {
    pub(crate) dwSize: u32,
    pub(crate) guid: GUID,
    pub(crate) dwEffType: u32,
    pub(crate) dwStaticParams: u32,
    pub(crate) dwDynamicParams: u32,
    pub(crate) tszName: [u16; MAX_PATH],
}

/// `DIENVELOPE`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DIENVELOPE {
    pub(crate) dwSize: u32,
    pub(crate) dwAttackLevel: u32,
    pub(crate) dwAttackTime: u32,
    pub(crate) dwFadeLevel: u32,
    pub(crate) dwFadeTime: u32,
}

/// `DIEFFECT` (with `dwStartDelay`, DirectInput 6 and later)
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct DIEFFECT {
    pub(crate) dwSize: u32,
    pub(crate) dwFlags: u32,
    pub(crate) dwDuration: u32,
    pub(crate) dwSamplePeriod: u32,
    pub(crate) dwGain: u32,
    pub(crate) dwTriggerButton: u32,
    pub(crate) dwTriggerRepeatInterval: u32,
    pub(crate) cAxes: u32,
    pub(crate) rgdwAxes: *mut u32,
    pub(crate) rglDirection: *mut i32,
    pub(crate) lpEnvelope: *mut DIENVELOPE,
    pub(crate) cbTypeSpecificParams: u32,
    pub(crate) lpvTypeSpecificParams: *mut c_void,
    pub(crate) dwStartDelay: u32,
}

/// `DICONSTANTFORCE`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DICONSTANTFORCE {
    pub(crate) lMagnitude: i32,
}

/// `DIPERIODIC`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DIPERIODIC {
    pub(crate) dwMagnitude: u32,
    pub(crate) lOffset: i32,
    pub(crate) dwPhase: u32,
    pub(crate) dwPeriod: u32,
}

/// `DICONDITION`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DICONDITION {
    pub(crate) lOffset: i32,
    pub(crate) lPositiveCoefficient: i32,
    pub(crate) lNegativeCoefficient: i32,
    pub(crate) dwPositiveSaturation: u32,
    pub(crate) dwNegativeSaturation: u32,
    pub(crate) lDeadBand: i32,
}

/// `DIRAMPFORCE`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DIRAMPFORCE {
    pub(crate) lStart: i32,
    pub(crate) lEnd: i32,
}

/// `DICUSTOMFORCE`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct DICUSTOMFORCE {
    pub(crate) cChannels: u32,
    pub(crate) dwSamplePeriod: u32,
    pub(crate) cSamples: u32,
    pub(crate) rglForceData: *mut i32,
}

/// `LPDIENUMDEVICESCALLBACKW`
pub(crate) type LPDIENUMDEVICESCALLBACKW =
    unsafe extern "system" fn(*const DIDEVICEINSTANCEW, *mut c_void) -> BOOL;
/// `LPDIENUMDEVICEOBJECTSCALLBACKW`
pub(crate) type LPDIENUMDEVICEOBJECTSCALLBACKW =
    unsafe extern "system" fn(*const DIDEVICEOBJECTINSTANCEW, *mut c_void) -> BOOL;
/// `LPDIENUMEFFECTSCALLBACKW`
pub(crate) type LPDIENUMEFFECTSCALLBACKW =
    unsafe extern "system" fn(*const DIEFFECTINFOW, *mut c_void) -> BOOL;

/// A method SDL doesn't call (its arguments aren't declared).
type Unused = unsafe extern "system" fn();

/// `IDirectInput8WVtbl`
#[repr(C)]
pub(crate) struct IDirectInput8WVtbl {
    pub(crate) base: IUnknownVtbl,
    pub(crate) CreateDevice: unsafe extern "system" fn(
        *mut IDirectInput8W,
        *const GUID,
        *mut *mut IDirectInputDevice8W,
        *mut c_void,
    ) -> HRESULT,
    pub(crate) EnumDevices: unsafe extern "system" fn(
        *mut IDirectInput8W,
        u32,
        LPDIENUMDEVICESCALLBACKW,
        *mut c_void,
        u32,
    ) -> HRESULT,
    pub(crate) GetDeviceStatus:
        unsafe extern "system" fn(*mut IDirectInput8W, *const GUID) -> HRESULT,
    pub(crate) RunControlPanel:
        unsafe extern "system" fn(*mut IDirectInput8W, HWND, u32) -> HRESULT,
    pub(crate) Initialize:
        unsafe extern "system" fn(*mut IDirectInput8W, HINSTANCE, u32) -> HRESULT,
    pub(crate) FindDevice:
        unsafe extern "system" fn(*mut IDirectInput8W, *const GUID, PCWSTR, *mut GUID) -> HRESULT,
    pub(crate) EnumDevicesBySemantics: Unused,
    pub(crate) ConfigureDevices: Unused,
}
/// `IDirectInput8W`
pub(crate) type IDirectInput8W = ComObject<IDirectInput8WVtbl>;

/// `IDirectInputDevice8WVtbl`
#[repr(C)]
pub(crate) struct IDirectInputDevice8WVtbl {
    pub(crate) base: IUnknownVtbl,
    pub(crate) GetCapabilities:
        unsafe extern "system" fn(*mut IDirectInputDevice8W, *mut DIDEVCAPS) -> HRESULT,
    pub(crate) EnumObjects: unsafe extern "system" fn(
        *mut IDirectInputDevice8W,
        LPDIENUMDEVICEOBJECTSCALLBACKW,
        *mut c_void,
        u32,
    ) -> HRESULT,
    pub(crate) GetProperty: unsafe extern "system" fn(
        *mut IDirectInputDevice8W,
        *const GUID,
        *mut DIPROPHEADER,
    ) -> HRESULT,
    pub(crate) SetProperty: unsafe extern "system" fn(
        *mut IDirectInputDevice8W,
        *const GUID,
        *const DIPROPHEADER,
    ) -> HRESULT,
    pub(crate) Acquire: unsafe extern "system" fn(*mut IDirectInputDevice8W) -> HRESULT,
    pub(crate) Unacquire: unsafe extern "system" fn(*mut IDirectInputDevice8W) -> HRESULT,
    pub(crate) GetDeviceState:
        unsafe extern "system" fn(*mut IDirectInputDevice8W, u32, *mut c_void) -> HRESULT,
    pub(crate) GetDeviceData: unsafe extern "system" fn(
        *mut IDirectInputDevice8W,
        u32,
        *mut DIDEVICEOBJECTDATA,
        *mut u32,
        u32,
    ) -> HRESULT,
    pub(crate) SetDataFormat:
        unsafe extern "system" fn(*mut IDirectInputDevice8W, *const DIDATAFORMAT) -> HRESULT,
    pub(crate) SetEventNotification:
        unsafe extern "system" fn(*mut IDirectInputDevice8W, HANDLE) -> HRESULT,
    pub(crate) SetCooperativeLevel:
        unsafe extern "system" fn(*mut IDirectInputDevice8W, HWND, u32) -> HRESULT,
    pub(crate) GetObjectInfo: unsafe extern "system" fn(
        *mut IDirectInputDevice8W,
        *mut DIDEVICEOBJECTINSTANCEW,
        u32,
        u32,
    ) -> HRESULT,
    pub(crate) GetDeviceInfo:
        unsafe extern "system" fn(*mut IDirectInputDevice8W, *mut DIDEVICEINSTANCEW) -> HRESULT,
    pub(crate) RunControlPanel:
        unsafe extern "system" fn(*mut IDirectInputDevice8W, HWND, u32) -> HRESULT,
    pub(crate) Initialize: unsafe extern "system" fn(
        *mut IDirectInputDevice8W,
        HINSTANCE,
        u32,
        *const GUID,
    ) -> HRESULT,
    pub(crate) CreateEffect: unsafe extern "system" fn(
        *mut IDirectInputDevice8W,
        *const GUID,
        *const DIEFFECT,
        *mut *mut IDirectInputEffect,
        *mut c_void,
    ) -> HRESULT,
    pub(crate) EnumEffects: unsafe extern "system" fn(
        *mut IDirectInputDevice8W,
        LPDIENUMEFFECTSCALLBACKW,
        *mut c_void,
        u32,
    ) -> HRESULT,
    pub(crate) GetEffectInfo: Unused,
    pub(crate) GetForceFeedbackState:
        unsafe extern "system" fn(*mut IDirectInputDevice8W, *mut u32) -> HRESULT,
    pub(crate) SendForceFeedbackCommand:
        unsafe extern "system" fn(*mut IDirectInputDevice8W, u32) -> HRESULT,
    pub(crate) EnumCreatedEffectObjects: Unused,
    pub(crate) Escape: Unused,
    pub(crate) Poll: unsafe extern "system" fn(*mut IDirectInputDevice8W) -> HRESULT,
    pub(crate) SendDeviceData: Unused,
    pub(crate) EnumEffectsInFile: Unused,
    pub(crate) WriteEffectToFile: Unused,
    pub(crate) BuildActionMap: Unused,
    pub(crate) SetActionMap: Unused,
    pub(crate) GetImageInfo: Unused,
}
/// `IDirectInputDevice8W`
pub(crate) type IDirectInputDevice8W = ComObject<IDirectInputDevice8WVtbl>;

/// `IDirectInputEffectVtbl`
#[repr(C)]
pub(crate) struct IDirectInputEffectVtbl {
    pub(crate) base: IUnknownVtbl,
    pub(crate) Initialize:
        unsafe extern "system" fn(*mut IDirectInputEffect, HINSTANCE, u32, *const GUID) -> HRESULT,
    pub(crate) GetEffectGuid:
        unsafe extern "system" fn(*mut IDirectInputEffect, *mut GUID) -> HRESULT,
    pub(crate) GetParameters:
        unsafe extern "system" fn(*mut IDirectInputEffect, *mut DIEFFECT, u32) -> HRESULT,
    pub(crate) SetParameters:
        unsafe extern "system" fn(*mut IDirectInputEffect, *const DIEFFECT, u32) -> HRESULT,
    pub(crate) Start: unsafe extern "system" fn(*mut IDirectInputEffect, u32, u32) -> HRESULT,
    pub(crate) Stop: unsafe extern "system" fn(*mut IDirectInputEffect) -> HRESULT,
    pub(crate) GetEffectStatus:
        unsafe extern "system" fn(*mut IDirectInputEffect, *mut u32) -> HRESULT,
    pub(crate) Download: unsafe extern "system" fn(*mut IDirectInputEffect) -> HRESULT,
    pub(crate) Unload: unsafe extern "system" fn(*mut IDirectInputEffect) -> HRESULT,
    pub(crate) Escape: Unused,
}
/// `IDirectInputEffect`
pub(crate) type IDirectInputEffect = ComObject<IDirectInputEffectVtbl>;

/// The `DIPROP_*` pseudo-GUID for property `n` (`MAKEDIPROP(n)`).
fn diprop(n: usize) -> *const GUID {
    n as *const GUID
}

/// The result of a DirectInput call: `Ok` with the (success) code, or
/// `Err` with the failure.
fn check(hr: HRESULT) -> Result<HRESULT, HRESULT> {
    if hr < 0 {
        Err(hr)
    } else {
        Ok(hr)
    }
}

/// Run `f` on each item of an enumeration through a DirectInput callback,
/// while it returns true.
///
/// # Safety
///
/// `enumerate` must call the callback with valid items and the context it
/// is given, only until it returns.
unsafe fn enumerate<T, F: FnMut(&T) -> bool>(
    mut f: F,
    enumerate: impl FnOnce(
        unsafe extern "system" fn(*const T, *mut c_void) -> BOOL,
        *mut c_void,
    ) -> HRESULT,
) -> HRESULT {
    unsafe extern "system" fn callback<T, F: FnMut(&T) -> bool>(
        item: *const T,
        context: *mut c_void,
    ) -> BOOL {
        // SAFETY: the context is the `F` below, alive for the enumeration,
        // and the item is valid for the call (enumerate's contract).
        let (f, item) = unsafe { (&mut *context.cast::<F>(), &*item) };
        if f(item) {
            DIENUM_CONTINUE
        } else {
            DIENUM_STOP
        }
    }
    enumerate(callback::<T, F>, (&mut f as *mut F).cast())
}

/// An `IDirectInput8W` object.
#[derive(Clone, Debug)]
pub(crate) struct DirectInput(ComPtr<IDirectInput8WVtbl>);

impl DirectInput {
    /// `CoCreateInstance(&CLSID_DirectInput8, NULL, CLSCTX_INPROC_SERVER,
    /// &IID_IDirectInput8, ...)` (COM must be initialized on this thread).
    pub(crate) fn create() -> Result<DirectInput, HRESULT> {
        // SAFETY: CoCreateInstance stores an IDirectInput8W reference on
        // success.
        unsafe {
            ComPtr::from_out(|out: *mut *mut IDirectInput8W| {
                CoCreateInstance(
                    &CLSID_DIRECTINPUT8,
                    std::ptr::null_mut(),
                    CLSCTX_INPROC_SERVER,
                    &IID_IDIRECTINPUT8W,
                    out.cast(),
                )
            })
        }
        .map(DirectInput)
    }

    /// `IDirectInput8_Initialize()`
    pub(crate) fn initialize(&self, instance: HINSTANCE, version: u32) -> Result<HRESULT, HRESULT> {
        // SAFETY: the object is alive; the arguments are plain values.
        check(unsafe { (self.0.vtbl().Initialize)(self.0.as_ptr(), instance, version) })
    }

    /// `IDirectInput8_CreateDevice()`
    pub(crate) fn create_device(&self, guid_instance: &GUID) -> Result<InputDevice, HRESULT> {
        // SAFETY: CreateDevice stores an IDirectInputDevice8W reference on
        // success; no aggregation.
        unsafe {
            ComPtr::from_out(|out| {
                (self.0.vtbl().CreateDevice)(
                    self.0.as_ptr(),
                    guid_instance,
                    out,
                    std::ptr::null_mut(),
                )
            })
        }
        .map(InputDevice)
    }

    /// `IDirectInput8_EnumDevices()`, calling `f` for each device while it
    /// returns true (`DIENUM_CONTINUE`).
    pub(crate) fn enum_devices(
        &self,
        dev_type: u32,
        flags: u32,
        f: impl FnMut(&DIDEVICEINSTANCEW) -> bool,
    ) -> Result<HRESULT, HRESULT> {
        // SAFETY: EnumDevices calls the callback with valid device
        // instances and our context, until it returns.
        check(unsafe {
            enumerate(f, |callback, context| {
                (self.0.vtbl().EnumDevices)(self.0.as_ptr(), dev_type, callback, context, flags)
            })
        })
    }
}

/// An `IDirectInputDevice8W` object.
#[derive(Clone, Debug)]
pub(crate) struct InputDevice(ComPtr<IDirectInputDevice8WVtbl>);

impl InputDevice {
    fn this(&self) -> *mut IDirectInputDevice8W {
        self.0.as_ptr()
    }

    fn vtbl(&self) -> &IDirectInputDevice8WVtbl {
        self.0.vtbl()
    }

    /// `IDirectInputDevice8_GetCapabilities()` into `caps` (whose `dwSize`
    /// the caller sets).
    pub(crate) fn get_capabilities(&self, caps: &mut DIDEVCAPS) -> Result<HRESULT, HRESULT> {
        // SAFETY: the object is alive; caps is writable.
        check(unsafe { (self.vtbl().GetCapabilities)(self.this(), caps) })
    }

    /// `IDirectInputDevice8_GetProperty()` into a property structure `T`
    /// that starts with its header.
    fn get_property<T>(&self, property: usize, value: &mut T) -> Result<HRESULT, HRESULT> {
        // SAFETY: every DIPROP* structure starts with its DIPROPHEADER,
        // whose dwSize tells DirectInput how much it may write.
        check(unsafe {
            (self.vtbl().GetProperty)(self.this(), diprop(property), (value as *mut T).cast())
        })
    }

    /// `IDirectInputDevice8_SetProperty()` from a property structure `T`
    /// that starts with its header.
    fn set_property<T>(&self, property: usize, value: &T) -> Result<HRESULT, HRESULT> {
        // SAFETY: as in get_property(); SetProperty only reads.
        check(unsafe {
            (self.vtbl().SetProperty)(self.this(), diprop(property), (value as *const T).cast())
        })
    }

    /// A `DIPROPDWORD` property of the device (`DIPH_DEVICE`).
    pub(crate) fn get_property_dword(&self, property: usize) -> Result<u32, HRESULT> {
        let mut dipdw = DIPROPDWORD::device(0);
        self.get_property(property, &mut dipdw)?;
        Ok(dipdw.dwData)
    }

    /// Set a `DIPROPDWORD` property: of the device with `DIPH_DEVICE`, or
    /// of an object.
    pub(crate) fn set_property_dword(
        &self,
        property: usize,
        dw_obj: u32,
        dw_how: u32,
        dw_data: u32,
    ) -> Result<HRESULT, HRESULT> {
        let dipdw = DIPROPDWORD {
            diph: DIPROPHEADER::of::<DIPROPDWORD>(dw_obj, dw_how),
            dwData: dw_data,
        };
        self.set_property(property, &dipdw)
    }

    /// Set a `DIPROPRANGE` property of an object.
    pub(crate) fn set_property_range(
        &self,
        property: usize,
        dw_obj: u32,
        dw_how: u32,
        range: (i32, i32),
    ) -> Result<HRESULT, HRESULT> {
        let diprg = DIPROPRANGE {
            diph: DIPROPHEADER::of::<DIPROPRANGE>(dw_obj, dw_how),
            lMin: range.0,
            lMax: range.1,
        };
        self.set_property(property, &diprg)
    }

    /// A `DIPROPSTRING` property of the device, as UTF-16.
    pub(crate) fn get_property_string(&self, property: usize) -> Result<[u16; MAX_PATH], HRESULT> {
        let mut dipstr = DIPROPSTRING {
            diph: DIPROPHEADER::of::<DIPROPSTRING>(0, DIPH_DEVICE),
            wsz: [0; MAX_PATH],
        };
        self.get_property(property, &mut dipstr)?;
        Ok(dipstr.wsz)
    }

    /// The device path (`DIPROP_GUIDANDPATH`'s `wszPath`), as UTF-16.
    pub(crate) fn get_guid_and_path(&self) -> Result<[u16; MAX_PATH], HRESULT> {
        let mut dippath = DIPROPGUIDANDPATH {
            diph: DIPROPHEADER::of::<DIPROPGUIDANDPATH>(0, DIPH_DEVICE),
            // SAFETY: a GUID is plain data.
            guidClass: unsafe { std::mem::zeroed() },
            wszPath: [0; MAX_PATH],
        };
        self.get_property(DIPROP_GUIDANDPATH, &mut dippath)?;
        Ok(dippath.wszPath)
    }

    /// `IDirectInputDevice8_Acquire()`
    pub(crate) fn acquire(&self) -> HRESULT {
        // SAFETY: the object is alive.
        unsafe { (self.vtbl().Acquire)(self.this()) }
    }

    /// `IDirectInputDevice8_Unacquire()`
    pub(crate) fn unacquire(&self) -> HRESULT {
        // SAFETY: the object is alive.
        unsafe { (self.vtbl().Unacquire)(self.this()) }
    }

    /// `IDirectInputDevice8_Poll()`
    pub(crate) fn poll(&self) -> HRESULT {
        // SAFETY: the object is alive.
        unsafe { (self.vtbl().Poll)(self.this()) }
    }

    /// `IDirectInputDevice8_GetDeviceState()` with a `DIJOYSTATE2` (the
    /// data format SDL sets).
    pub(crate) fn get_device_state(&self, state: &mut DIJOYSTATE2) -> HRESULT {
        // SAFETY: the object is alive; state holds the size given.
        unsafe {
            (self.vtbl().GetDeviceState)(
                self.this(),
                size_of::<DIJOYSTATE2>() as u32,
                (state as *mut DIJOYSTATE2).cast(),
            )
        }
    }

    /// `IDirectInputDevice8_GetDeviceData()` into `buffer`: the result and
    /// the number of events stored.
    pub(crate) fn get_device_data(&self, buffer: &mut [DIDEVICEOBJECTDATA]) -> (HRESULT, usize) {
        let mut numevents = buffer.len() as u32;
        // SAFETY: the object is alive; buffer holds numevents entries.
        let hr = unsafe {
            (self.vtbl().GetDeviceData)(
                self.this(),
                size_of::<DIDEVICEOBJECTDATA>() as u32,
                buffer.as_mut_ptr(),
                &mut numevents,
                0,
            )
        };
        (hr, (numevents as usize).min(buffer.len()))
    }

    /// `IDirectInputDevice8_SetDataFormat()`
    pub(crate) fn set_data_format(&self, format: &DIDATAFORMAT) -> Result<HRESULT, HRESULT> {
        // SAFETY: the object is alive; the format (and the objects it
        // points to) are valid, and copied by DirectInput.
        check(unsafe { (self.vtbl().SetDataFormat)(self.this(), format) })
    }

    /// `IDirectInputDevice8_SetCooperativeLevel()`
    pub(crate) fn set_cooperative_level(&self, hwnd: HWND, flags: u32) -> Result<HRESULT, HRESULT> {
        // SAFETY: the object is alive; DirectInput validates the window.
        check(unsafe { (self.vtbl().SetCooperativeLevel)(self.this(), hwnd, flags) })
    }

    /// `IDirectInputDevice8_EnumObjects()`, calling `f` for each object
    /// while it returns true.
    pub(crate) fn enum_objects(
        &self,
        f: impl FnMut(&DIDEVICEOBJECTINSTANCEW) -> bool,
        flags: u32,
    ) -> Result<HRESULT, HRESULT> {
        // SAFETY: EnumObjects calls the callback with valid objects and our
        // context, until it returns.
        check(unsafe {
            enumerate(f, |callback, context| {
                (self.vtbl().EnumObjects)(self.this(), callback, context, flags)
            })
        })
    }

    /// `IDirectInputDevice8_GetDeviceInfo()`
    pub(crate) fn get_device_info(&self) -> Result<DIDEVICEINSTANCEW, HRESULT> {
        let mut instance = DIDEVICEINSTANCEW::new();
        // SAFETY: the object is alive; instance is writable, its size set.
        check(unsafe { (self.vtbl().GetDeviceInfo)(self.this(), &mut instance) })?;
        Ok(instance)
    }

    /// `IDirectInputDevice8_CreateEffect()`
    pub(crate) fn create_effect(&self, guid: &GUID, effect: &DIEFFECT) -> Result<Effect, HRESULT> {
        // SAFETY: CreateEffect stores an IDirectInputEffect reference on
        // success; the effect and what it points to are valid for the call
        // (DirectInput copies them); no aggregation.
        unsafe {
            ComPtr::from_out(|out| {
                (self.vtbl().CreateEffect)(self.this(), guid, effect, out, std::ptr::null_mut())
            })
        }
        .map(Effect)
    }

    /// `IDirectInputDevice8_EnumEffects()`, calling `f` for each effect
    /// while it returns true.
    pub(crate) fn enum_effects(
        &self,
        f: impl FnMut(&DIEFFECTINFOW) -> bool,
        eff_type: u32,
    ) -> Result<HRESULT, HRESULT> {
        // SAFETY: EnumEffects calls the callback with valid effect infos
        // and our context, until it returns.
        check(unsafe {
            enumerate(f, |callback, context| {
                (self.vtbl().EnumEffects)(self.this(), callback, context, eff_type)
            })
        })
    }

    /// `IDirectInputDevice8_SendForceFeedbackCommand()`
    pub(crate) fn send_force_feedback_command(&self, command: u32) -> HRESULT {
        // SAFETY: the object is alive.
        unsafe { (self.vtbl().SendForceFeedbackCommand)(self.this(), command) }
    }
}

/// An `IDirectInputEffect` object.
#[derive(Debug)]
pub(crate) struct Effect(ComPtr<IDirectInputEffectVtbl>);

impl Effect {
    fn this(&self) -> *mut IDirectInputEffect {
        self.0.as_ptr()
    }

    /// `IDirectInputEffect_SetParameters()`
    pub(crate) fn set_parameters(&self, effect: &DIEFFECT, flags: u32) -> HRESULT {
        // SAFETY: the object is alive; the effect and what it points to are
        // valid for the call (DirectInput copies them).
        unsafe { (self.0.vtbl().SetParameters)(self.this(), effect, flags) }
    }

    /// `IDirectInputEffect_Start()`
    pub(crate) fn start(&self, iterations: u32, flags: u32) -> HRESULT {
        // SAFETY: the object is alive.
        unsafe { (self.0.vtbl().Start)(self.this(), iterations, flags) }
    }

    /// `IDirectInputEffect_Stop()`
    pub(crate) fn stop(&self) -> HRESULT {
        // SAFETY: the object is alive.
        unsafe { (self.0.vtbl().Stop)(self.this()) }
    }

    /// `IDirectInputEffect_GetEffectStatus()`
    pub(crate) fn get_effect_status(&self) -> Result<u32, HRESULT> {
        let mut status = 0;
        // SAFETY: the object is alive; status is writable.
        check(unsafe { (self.0.vtbl().GetEffectStatus)(self.this(), &mut status) })?;
        Ok(status)
    }

    /// `IDirectInputEffect_Unload()`
    pub(crate) fn unload(&self) -> HRESULT {
        // SAFETY: the object is alive.
        unsafe { (self.0.vtbl().Unload)(self.this()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::offset_of;

    #[test]
    fn layouts_match_dinput_h() {
        // (values from mingw-w64's dinput.h, checked with a C program)
        assert_eq!(size_of::<DIDEVICEINSTANCEW>(), 1100);
        assert_eq!(size_of::<DIDEVCAPS>(), 44);
        assert_eq!(size_of::<DIPROPDWORD>(), 20);
        assert_eq!(size_of::<DIPROPRANGE>(), 24);
        assert_eq!(size_of::<DIPROPSTRING>(), 536);
        assert_eq!(size_of::<DIPROPGUIDANDPATH>(), 552);
        assert_eq!(size_of::<DIJOYSTATE2>(), 272);
        assert_eq!(offset_of!(DIJOYSTATE2, rgbButtons), DIJOFS_BUTTON0 as usize);
        assert_eq!(offset_of!(DIJOYSTATE2, lVX), 176);
        assert_eq!(offset_of!(DIJOYSTATE2, lAX), 208);
        assert_eq!(offset_of!(DIJOYSTATE2, lFX), 240);
        assert_eq!(offset_of!(DIJOYSTATE2, rglFSlider), 264);
        assert_eq!(size_of::<DIDEVICEOBJECTINSTANCEW>(), 576);
        assert_eq!(size_of::<DIEFFECTINFOW>(), 552);
        if cfg!(target_pointer_width = "64") {
            assert_eq!(size_of::<DIOBJECTDATAFORMAT>(), 24);
            assert_eq!(size_of::<DIDATAFORMAT>(), 32);
            assert_eq!(size_of::<DIDEVICEOBJECTDATA>(), 24);
            assert_eq!(size_of::<DIEFFECT>(), 80);
            assert_eq!(size_of::<DICUSTOMFORCE>(), 24);
        }
        assert_eq!(size_of::<DIENVELOPE>(), 20);
        assert_eq!(size_of::<DIPERIODIC>(), 16);
        assert_eq!(size_of::<DICONDITION>(), 24);
        // The vtables: IUnknown, then 8, 29 and 10 methods
        let ptr = size_of::<usize>();
        assert_eq!(size_of::<IDirectInput8WVtbl>(), (3 + 8) * ptr);
        assert_eq!(size_of::<IDirectInputDevice8WVtbl>(), (3 + 29) * ptr);
        assert_eq!(size_of::<IDirectInputEffectVtbl>(), (3 + 10) * ptr);
    }
}
