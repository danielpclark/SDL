// Rust translation of src/core/windows/SDL_xinput.c and SDL_xinput.h from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! XInput, loaded at run time from `XInput1_4.dll` (Windows 8 and later),
//! `XInput1_3.dll` (the redistributable) or `XInput9_1_0.dll` (Vista and
//! 7), with the declarations SDL uses written by hand (`SDL_xinput.h`
//! declares them itself when the SDK lacks `xinput.h`). The DLL is
//! reference counted: [`load_xinput_dll`] and [`unload_xinput_dll`] are
//! `WIN_LoadXInputDLL()` and `WIN_UnloadXInputDLL()`.

#![allow(non_camel_case_types, non_snake_case)]

use std::sync::Mutex;

use crate::loadso::SharedObject;

pub(crate) const XUSER_MAX_COUNT: u32 = 4;
pub(crate) const XUSER_INDEX_ANY: u32 = 0x000000FF;
pub(crate) const XINPUT_CAPS_FFB_SUPPORTED: u16 = 0x0001;
pub(crate) const XINPUT_CAPS_WIRELESS: u16 = 0x0002;

pub(crate) const XINPUT_DEVSUBTYPE_UNKNOWN: u8 = 0x00;
pub(crate) const XINPUT_DEVSUBTYPE_GAMEPAD: u8 = 0x01;
pub(crate) const XINPUT_DEVSUBTYPE_WHEEL: u8 = 0x02;
pub(crate) const XINPUT_DEVSUBTYPE_ARCADE_STICK: u8 = 0x03;
pub(crate) const XINPUT_DEVSUBTYPE_FLIGHT_STICK: u8 = 0x04;
pub(crate) const XINPUT_DEVSUBTYPE_DANCE_PAD: u8 = 0x05;
pub(crate) const XINPUT_DEVSUBTYPE_GUITAR: u8 = 0x06;
pub(crate) const XINPUT_DEVSUBTYPE_GUITAR_ALTERNATE: u8 = 0x07;
pub(crate) const XINPUT_DEVSUBTYPE_DRUM_KIT: u8 = 0x08;
pub(crate) const XINPUT_DEVSUBTYPE_GUITAR_BASS: u8 = 0x0B;
pub(crate) const XINPUT_DEVSUBTYPE_ARCADE_PAD: u8 = 0x13;

pub(crate) const XINPUT_FLAG_GAMEPAD: u32 = 0x01;

pub(crate) const XINPUT_GAMEPAD_DPAD_UP: u16 = 0x0001;
pub(crate) const XINPUT_GAMEPAD_DPAD_DOWN: u16 = 0x0002;
pub(crate) const XINPUT_GAMEPAD_DPAD_LEFT: u16 = 0x0004;
pub(crate) const XINPUT_GAMEPAD_DPAD_RIGHT: u16 = 0x0008;
pub(crate) const XINPUT_GAMEPAD_START: u16 = 0x0010;
pub(crate) const XINPUT_GAMEPAD_BACK: u16 = 0x0020;
pub(crate) const XINPUT_GAMEPAD_LEFT_THUMB: u16 = 0x0040;
pub(crate) const XINPUT_GAMEPAD_RIGHT_THUMB: u16 = 0x0080;
pub(crate) const XINPUT_GAMEPAD_LEFT_SHOULDER: u16 = 0x0100;
pub(crate) const XINPUT_GAMEPAD_RIGHT_SHOULDER: u16 = 0x0200;
pub(crate) const XINPUT_GAMEPAD_A: u16 = 0x1000;
pub(crate) const XINPUT_GAMEPAD_B: u16 = 0x2000;
pub(crate) const XINPUT_GAMEPAD_X: u16 = 0x4000;
pub(crate) const XINPUT_GAMEPAD_Y: u16 = 0x8000;

pub(crate) const XINPUT_GAMEPAD_GUIDE: u16 = 0x0400;

pub(crate) const BATTERY_DEVTYPE_GAMEPAD: u8 = 0x00;

pub(crate) const BATTERY_TYPE_DISCONNECTED: u8 = 0x00;
pub(crate) const BATTERY_TYPE_WIRED: u8 = 0x01;
pub(crate) const BATTERY_TYPE_UNKNOWN: u8 = 0xFF;
pub(crate) const BATTERY_LEVEL_EMPTY: u8 = 0x00;
pub(crate) const BATTERY_LEVEL_LOW: u8 = 0x01;
pub(crate) const BATTERY_LEVEL_MEDIUM: u8 = 0x02;
pub(crate) const BATTERY_LEVEL_FULL: u8 = 0x03;

/// This is the same as XINPUT_BATTERY_INFORMATION, but always defined
/// instead of just if WIN32_WINNT >= _WIN32_WINNT_WIN8.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct XINPUT_BATTERY_INFORMATION_EX {
    pub(crate) BatteryType: u8,
    pub(crate) BatteryLevel: u8,
}

/// `XINPUT_GAMEPAD`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct XINPUT_GAMEPAD {
    pub(crate) wButtons: u16,
    pub(crate) bLeftTrigger: u8,
    pub(crate) bRightTrigger: u8,
    pub(crate) sThumbLX: i16,
    pub(crate) sThumbLY: i16,
    pub(crate) sThumbRX: i16,
    pub(crate) sThumbRY: i16,
}

/// `XINPUT_STATE`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct XINPUT_STATE {
    pub(crate) dwPacketNumber: u32,
    pub(crate) Gamepad: XINPUT_GAMEPAD,
}

/// `XINPUT_VIBRATION`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct XINPUT_VIBRATION {
    pub(crate) wLeftMotorSpeed: u16,
    pub(crate) wRightMotorSpeed: u16,
}

/// `XINPUT_CAPABILITIES`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct XINPUT_CAPABILITIES {
    pub(crate) Type: u8,
    pub(crate) SubType: u8,
    pub(crate) Flags: u16,
    pub(crate) Gamepad: XINPUT_GAMEPAD,
    pub(crate) Vibration: XINPUT_VIBRATION,
}

/// This struct is not defined in XInput headers.
/// Translation of `SDL_XINPUT_CAPABILITIES_EX`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct XINPUT_CAPABILITIES_EX {
    pub(crate) Capabilities: XINPUT_CAPABILITIES,
    pub(crate) VendorId: u16,
    pub(crate) ProductId: u16,
    pub(crate) ProductVersion: u16,
    pub(crate) unk1: u16,
    pub(crate) unk2: u32,
}

/// `XInputGetState_t`
pub(crate) type XInputGetState_t =
    unsafe extern "system" fn(dwUserIndex: u32, pState: *mut XINPUT_STATE) -> u32;
/// `XInputSetState_t`
pub(crate) type XInputSetState_t =
    unsafe extern "system" fn(dwUserIndex: u32, pVibration: *mut XINPUT_VIBRATION) -> u32;
/// `XInputGetCapabilities_t`
pub(crate) type XInputGetCapabilities_t = unsafe extern "system" fn(
    dwUserIndex: u32,
    dwFlags: u32,
    pCapabilities: *mut XINPUT_CAPABILITIES,
) -> u32;
/// Only available in XInput 1.4 that is shipped with Windows 8 and newer.
/// `XInputGetCapabilitiesEx_t`
pub(crate) type XInputGetCapabilitiesEx_t = unsafe extern "system" fn(
    dwReserved: u32,
    dwUserIndex: u32,
    dwFlags: u32,
    pCapabilitiesEx: *mut XINPUT_CAPABILITIES_EX,
) -> u32;
/// `XInputGetBatteryInformation_t`
pub(crate) type XInputGetBatteryInformation_t = unsafe extern "system" fn(
    dwUserIndex: u32,
    devType: u8,
    pBatteryInformation: *mut XINPUT_BATTERY_INFORMATION_EX,
) -> u32;

/// The XInput functions SDL uses (`SDL_XInputGetState` and the other
/// function pointers); valid while the DLL stays loaded.
#[derive(Clone, Copy, Debug)]
pub(crate) struct XInputFunctions {
    get_state: XInputGetState_t,
    set_state: XInputSetState_t,
    get_capabilities: XInputGetCapabilities_t,
    get_capabilities_ex: Option<XInputGetCapabilitiesEx_t>,
    get_battery_information: Option<XInputGetBatteryInformation_t>,
}

impl XInputFunctions {
    /// `XINPUTGETSTATE(userid, &state)`: the result code and the state.
    pub(crate) fn get_state(&self, user_index: u32) -> (u32, XINPUT_STATE) {
        let mut state = XINPUT_STATE::default();
        // SAFETY: the function was loaded with this signature from a DLL
        // that is still loaded (a reference is held while this is used);
        // state is writable.
        let result = unsafe { (self.get_state)(user_index, &mut state) };
        (result, state)
    }

    /// `XINPUTSETSTATE(userid, &vibration)`
    pub(crate) fn set_state(&self, user_index: u32, vibration: &mut XINPUT_VIBRATION) -> u32 {
        // SAFETY: as for get_state().
        unsafe { (self.set_state)(user_index, vibration) }
    }

    /// `XINPUTGETCAPABILITIES(userid, flags, &capabilities)`
    pub(crate) fn get_capabilities(
        &self,
        user_index: u32,
        flags: u32,
    ) -> (u32, XINPUT_CAPABILITIES) {
        let mut capabilities = XINPUT_CAPABILITIES::default();
        // SAFETY: as for get_state().
        let result = unsafe { (self.get_capabilities)(user_index, flags, &mut capabilities) };
        (result, capabilities)
    }

    /// `XINPUTGETCAPABILITIESEX(1, userid, flags, &capabilities)`, if the
    /// DLL has it.
    pub(crate) fn get_capabilities_ex(
        &self,
        reserved: u32,
        user_index: u32,
        flags: u32,
    ) -> Option<(u32, XINPUT_CAPABILITIES_EX)> {
        let f = self.get_capabilities_ex?;
        let mut capabilities = XINPUT_CAPABILITIES_EX::default();
        // SAFETY: as for get_state().
        let result = unsafe { f(reserved, user_index, flags, &mut capabilities) };
        Some((result, capabilities))
    }

    /// `XINPUTGETBATTERYINFORMATION(userid, devtype, &info)`, if the DLL
    /// has it.
    pub(crate) fn get_battery_information(
        &self,
        user_index: u32,
        dev_type: u8,
    ) -> Option<(u32, XINPUT_BATTERY_INFORMATION_EX)> {
        let f = self.get_battery_information?;
        let mut info = XINPUT_BATTERY_INFORMATION_EX::default();
        // SAFETY: as for get_state().
        let result = unsafe { f(user_index, dev_type, &mut info) };
        Some((result, info))
    }
}

/// `s_pXInputDLL`, `s_XInputDLLRefCount` and the function pointers.
struct XInputDll {
    dll: Option<SharedObject>,
    ref_count: i32,
    functions: Option<XInputFunctions>,
}

static XINPUT: Mutex<XInputDll> = Mutex::new(XInputDll {
    dll: None,
    ref_count: 0,
    functions: None,
});

fn lock() -> std::sync::MutexGuard<'static, XInputDll> {
    XINPUT.lock().unwrap_or_else(|e| e.into_inner())
}

/// The loaded XInput functions (`SDL_XInputGetState` and so on, NULL when
/// the DLL isn't loaded).
pub(crate) fn xinput() -> Option<XInputFunctions> {
    lock().functions
}

/// Load XInput (counted). Translation of `WIN_LoadXInputDLL()`.
pub(crate) fn load_xinput_dll() -> bool {
    let mut x = lock();
    if x.dll.is_some() {
        crate::sdl_assert!(x.ref_count > 0);
        x.ref_count += 1;
        return true; // already loaded
    }

    /* NOTE: Don't load XinputUap.dll
     * This is XInput emulation over Windows.Gaming.Input, and has all the
     * limitations of that API (no devices at startup, no background input, etc.)
     */
    let dll = SharedObject::load("XInput1_4.dll") // 1.4 Ships with Windows 8.
        .or_else(|_| SharedObject::load("XInput1_3.dll")) // 1.3 can be installed as a redistributable component.
        // "9.1.0" Ships with Vista and Win7, and is more limited than 1.3+ (e.g. XInputGetStateEx is not available.)
        .or_else(|_| SharedObject::load("XInput9_1_0.dll"));
    let Ok(dll) = dll else {
        return false;
    };

    crate::sdl_assert!(x.ref_count == 0);
    x.ref_count = 1;

    // SAFETY: each symbol is loaded with the type of its XInput prototype.
    let functions = unsafe {
        // 100 is the ordinal for _XInputGetStateEx, which returns the same struct as XinputGetState, but with extra data in wButtons for the guide button, we think...
        let get_state = dll
            .function_by_ordinal::<XInputGetState_t>(100)
            .or_else(|_| dll.function::<XInputGetState_t>("XInputGetState"));
        let set_state = dll.function::<XInputSetState_t>("XInputSetState");
        let get_capabilities = dll.function::<XInputGetCapabilities_t>("XInputGetCapabilities");
        // 108 is the ordinal for _XInputGetCapabilitiesEx, which additionally returns VID/PID of the controller.
        let get_capabilities_ex = dll
            .function_by_ordinal::<XInputGetCapabilitiesEx_t>(108)
            .ok();
        let get_battery_information = dll
            .function::<XInputGetBatteryInformation_t>("XInputGetBatteryInformation")
            .ok();
        match (get_state, set_state, get_capabilities) {
            (Ok(get_state), Ok(set_state), Ok(get_capabilities)) => Some(XInputFunctions {
                get_state,
                set_state,
                get_capabilities,
                get_capabilities_ex,
                get_battery_information,
            }),
            _ => None,
        }
    };
    x.dll = Some(dll);
    let Some(functions) = functions else {
        drop(x);
        unload_xinput_dll();
        return false;
    };
    x.functions = Some(functions);

    true
}

/// Release a reference to XInput, unloading it with the last one.
/// Translation of `WIN_UnloadXInputDLL()`.
pub(crate) fn unload_xinput_dll() {
    let mut x = lock();
    if x.dll.is_some() {
        crate::sdl_assert!(x.ref_count > 0);
        x.ref_count -= 1;
        if x.ref_count == 0 {
            x.functions = None;
            x.dll = None;
        }
    } else {
        crate::sdl_assert!(x.ref_count == 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts() {
        assert_eq!(size_of::<XINPUT_GAMEPAD>(), 12);
        assert_eq!(size_of::<XINPUT_STATE>(), 16);
        assert_eq!(size_of::<XINPUT_CAPABILITIES>(), 20);
        assert_eq!(size_of::<XINPUT_CAPABILITIES_EX>(), 32);
        assert_eq!(size_of::<XINPUT_BATTERY_INFORMATION_EX>(), 2);
    }

    #[test]
    fn load_and_query_without_controllers() {
        let _l = crate::test_support::test_lock();
        if !load_xinput_dll() {
            println!("note: no XInput DLL here, skipping the XInput test");
            return;
        }
        // Counted
        assert!(load_xinput_dll());
        let x = xinput().unwrap();
        const ERROR_DEVICE_NOT_CONNECTED: u32 = 1167;
        for userid in 0..XUSER_MAX_COUNT {
            // A slot without a controller reports ERROR_DEVICE_NOT_CONNECTED
            let (result, _) = x.get_capabilities(userid, XINPUT_FLAG_GAMEPAD);
            assert!(
                result == 0 || result == ERROR_DEVICE_NOT_CONNECTED,
                "{result}"
            );
            let (result, _) = x.get_state(userid);
            assert!(
                result == 0 || result == ERROR_DEVICE_NOT_CONNECTED,
                "{result}"
            );
        }
        unload_xinput_dll();
        assert!(xinput().is_some());
        unload_xinput_dll();
        assert!(xinput().is_none());
        // Unloading more often than loading is harmless
        unload_xinput_dll();
    }
}
