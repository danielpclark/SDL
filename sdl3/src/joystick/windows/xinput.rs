// Rust translation of src/joystick/windows/SDL_xinputjoystick.c and
// SDL_xinputjoystick_c.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! XInput controllers for the Windows joystick driver: up to four, in
//! XInput's user slots, with XInput's fixed layout of six axes, eleven
//! buttons and a hat.
//!
//! When the RawInput driver is enabled, it handles the XInput controllers
//! (it isn't limited to four); while GameInput handles them, XInput is off.

use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Foundation::{ERROR_DEVICE_NOT_CONNECTED, ERROR_SUCCESS};

use super::super::{
    create_joystick_guid, create_joystick_name, joystick_handled_by_another_driver,
    send_joystick_axis, send_joystick_button, send_joystick_hat, send_joystick_power_info,
    should_ignore_joystick, JoystickData, HARDWARE_BUS_USB, HAT_CENTERED, HAT_DOWN, HAT_LEFT,
    HAT_RIGHT, HAT_UP, PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN, USB_PRODUCT_STEAM_VIRTUAL_GAMEPAD,
    USB_PRODUCT_XBOX360_XUSB_CONTROLLER, USB_VENDOR_MICROSOFT, USB_VENDOR_VALVE,
};
use super::{add_joystick_device, HwData, JoyStickDeviceData};
use crate::core::windows::directx::DIDEVICEINSTANCEW;
use crate::core::windows::xinput::*;
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::power::PowerState;

// Internal stuff

/// Translation of `s_bXInputEnabled`.
static XINPUT_ENABLED: AtomicBool = AtomicBool::new(false);

/// Translation of `SDL_XINPUT_Enabled()`.
pub(super) fn xinput_enabled() -> bool {
    XINPUT_ENABLED.load(Ordering::Relaxed)
}

/// The loaded XInput functions; the driver holds a reference to the DLL
/// while XInput is enabled.
fn functions() -> Option<XInputFunctions> {
    xinput()
}

/// Translation of `SDL_XINPUT_JoystickInit()`.
pub(super) fn joystick_init() -> bool {
    let mut enabled = true;

    if !hints::get_bool(hints::XINPUT_ENABLED, true)
        || crate::core::windows::gameinput::using_game_input_for_xinput_controllers()
    {
        enabled = false;
    }

    if enabled && !load_xinput_dll() {
        enabled = false; // oh well.
    }
    XINPUT_ENABLED.store(enabled, Ordering::Relaxed);

    true
}

/// Translation of `GetXInputName()`.
pub(super) fn get_xinput_name(userid: u8, sub_type: u8) -> String {
    let n = 1 + userid as u32;
    match sub_type {
        XINPUT_DEVSUBTYPE_GAMEPAD => format!("XInput Controller #{n}"),
        XINPUT_DEVSUBTYPE_WHEEL => format!("XInput Wheel #{n}"),
        XINPUT_DEVSUBTYPE_ARCADE_STICK => format!("XInput ArcadeStick #{n}"),
        XINPUT_DEVSUBTYPE_FLIGHT_STICK => format!("XInput FlightStick #{n}"),
        XINPUT_DEVSUBTYPE_DANCE_PAD => format!("XInput DancePad #{n}"),
        XINPUT_DEVSUBTYPE_GUITAR
        | XINPUT_DEVSUBTYPE_GUITAR_ALTERNATE
        | XINPUT_DEVSUBTYPE_GUITAR_BASS => format!("XInput Guitar #{n}"),
        XINPUT_DEVSUBTYPE_DRUM_KIT => format!("XInput DrumKit #{n}"),
        XINPUT_DEVSUBTYPE_ARCADE_PAD => format!("XInput ArcadePad #{n}"),
        _ => format!("XInput Device #{n}"),
    }
}

/// The vendor, product and version of the controller in a slot, from
/// `XInputGetCapabilitiesEx()`; whether that worked, and the IDs (a
/// generic XInput controller's when it didn't, with the version
/// untouched). Translation of `GetXInputDeviceInfo()`.
fn get_xinput_device_info(userid: u8, version: &mut u16) -> (bool, u16, u16) {
    let ex = functions().and_then(|x| x.get_capabilities_ex(1, userid as u32, 0));
    let Some((ERROR_SUCCESS, mut capabilities)) = ex else {
        // Use a generic VID/PID representing an XInput controller
        return (
            false,
            USB_VENDOR_MICROSOFT,
            USB_PRODUCT_XBOX360_XUSB_CONTROLLER,
        );
    };

    // Fixup for Wireless Xbox 360 Controller
    if capabilities.ProductId == 0 && capabilities.Capabilities.Flags & XINPUT_CAPS_WIRELESS != 0 {
        capabilities.VendorId = USB_VENDOR_MICROSOFT;
        capabilities.ProductId = USB_PRODUCT_XBOX360_XUSB_CONTROLLER;
    }

    *version = capabilities.ProductVersion;
    (true, capabilities.VendorId, capabilities.ProductId)
}

/// Translation of `SDL_XINPUT_GetSteamVirtualGamepadSlot()`.
pub(super) fn get_steam_virtual_gamepad_slot(userid: u8) -> i32 {
    match functions().and_then(|x| x.get_capabilities_ex(1, userid as u32, 0)) {
        Some((ERROR_SUCCESS, capabilities))
            if capabilities.VendorId == USB_VENDOR_VALVE
                && capabilities.ProductId == USB_PRODUCT_STEAM_VIRTUAL_GAMEPAD =>
        {
            capabilities.unk2 as i32
        }
        _ => -1,
    }
}

/// Translation of `AddXInputDevice()`: `context` is the device list of the
/// previous detection, `sys_joystick` the one being built.
pub(super) fn add_xinput_device(
    userid: u8,
    sub_type: u8,
    context: &mut Vec<JoyStickDeviceData>,
    sys_joystick: &mut Vec<JoyStickDeviceData>,
) {
    if super::rawinput::is_enabled() {
        // The raw input driver handles more than 4 controllers, so prefer that when available
        /* We do this check here rather than at the top of SDL_XINPUT_JoystickDetect() because
          we need to check XInput state before RAWINPUT gets a hold of the device, otherwise
          when a controller is connected via the wireless adapter, it will shut down at the
          first subsequent XInput call. This seems like a driver stack bug?

          Reference: https://github.com/libsdl-org/SDL/issues/3468
        */
        return;
    }

    if sub_type == XINPUT_DEVSUBTYPE_UNKNOWN {
        return;
    }

    if let Some(i) = context
        .iter()
        .position(|j| j.b_xinput_device && j.xinput_user_id == userid && j.sub_type == sub_type)
    {
        // (taken out of the previous list, wherever it is)
        let joystick = context.remove(i);
        sys_joystick.insert(0, joystick);
        return; // already in the list.
    }

    let name = get_xinput_name(userid, sub_type);
    let mut version = 0;
    let (_, vendor, product) = get_xinput_device_info(userid, &mut version);
    if should_ignore_joystick(vendor, product, version, Some(&name))
        || joystick_handled_by_another_driver(
            super::super::WINDOWS_DRIVER_INDEX,
            vendor,
            product,
            version,
            Some(&name),
        )
    {
        return;
    }

    let Some(joystickname) = create_joystick_name(vendor, product, None, Some(&name)) else {
        return; // better luck next time?
    };
    let new_joystick = JoyStickDeviceData {
        guid: create_joystick_guid(
            HARDWARE_BUS_USB,
            vendor,
            product,
            version,
            None,
            Some(&name),
            b'x',
            sub_type,
        ),
        joystickname,
        send_add_event: false,
        n_instance_id: 0,
        b_xinput_device: true,
        sub_type,
        xinput_user_id: userid,
        dxdevice: DIDEVICEINSTANCEW::new(),
        path: format!("XInput#{userid}"),
        steam_virtual_gamepad_slot: 0,
    };

    add_joystick_device(sys_joystick, new_joystick);
}

/// Translation of `SDL_XINPUT_JoystickDetect()`.
pub(super) fn joystick_detect(
    context: &mut Vec<JoyStickDeviceData>,
    sys_joystick: &mut Vec<JoyStickDeviceData>,
) {
    if !xinput_enabled() {
        return;
    }
    let Some(x) = functions() else {
        return;
    };

    // iterate in reverse, so these are in the final list in ascending numeric order.
    for userid in (0..XUSER_MAX_COUNT as u8).rev() {
        let (result, capabilities) = x.get_capabilities(userid as u32, XINPUT_FLAG_GAMEPAD);
        if result == ERROR_SUCCESS {
            add_xinput_device(userid, capabilities.SubType, context, sys_joystick);
        }
    }
}

/// Translation of `SDL_XINPUT_JoystickPresent()`.
pub(super) fn joystick_present(vendor: u16, product: u16, version: u16) -> bool {
    if !xinput_enabled() {
        return false;
    }

    // iterate in reverse, so these are in the final list in ascending numeric order.
    for userid in 0..XUSER_MAX_COUNT as u8 {
        let mut slot_version = 0;
        let (found, slot_vendor, slot_product) = get_xinput_device_info(userid, &mut slot_version);
        if found && vendor == slot_vendor && product == slot_product && version == slot_version {
            return true;
        }
    }
    false
}

/// Translation of `SDL_XINPUT_JoystickOpen()`.
pub(super) fn joystick_open(
    joystick: &mut JoystickData,
    joystickdevice: &JoyStickDeviceData,
    hwdata: &mut HwData,
) -> Result<()> {
    let user_id = joystickdevice.xinput_user_id;

    crate::sdl_assert!(xinput_enabled());
    let x = functions().ok_or_else(|| Error::new("XInput isn't loaded"))?;
    crate::sdl_assert!((user_id as u32) < XUSER_MAX_COUNT);

    hwdata.b_xinput_device = true;

    if x.get_capabilities(user_id as u32, XINPUT_FLAG_GAMEPAD).0 != ERROR_SUCCESS {
        return Err(Error::new(
            "Failed to obtain XInput device capabilities. Device disconnected?",
        ));
    }
    let mut state = XINPUT_VIBRATION::default();
    hwdata.b_xinput_haptic = x.set_state(user_id as u32, &mut state) == ERROR_SUCCESS;
    hwdata.userid = user_id;

    // The XInput API has a hard coded button/axis mapping, so we just match it
    joystick.naxes = 6;
    joystick.nbuttons = 11;
    joystick.nhats = 1;

    let _ = joystick
        .properties()
        .set(PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN, true);

    Ok(())
}

/// Translation of `UpdateXInputJoystickBatteryInformation()`: the power
/// state and percentage to report.
pub(super) fn battery_information(
    battery_information: &XINPUT_BATTERY_INFORMATION_EX,
) -> (PowerState, i32) {
    let state = match battery_information.BatteryType {
        BATTERY_TYPE_WIRED => PowerState::Charging,
        BATTERY_TYPE_UNKNOWN | BATTERY_TYPE_DISCONNECTED => PowerState::Unknown,
        _ => PowerState::OnBattery,
    };
    let percent = if state == PowerState::OnBattery || state == PowerState::Charging {
        match battery_information.BatteryLevel {
            BATTERY_LEVEL_EMPTY => 10,
            BATTERY_LEVEL_LOW => 40,
            BATTERY_LEVEL_MEDIUM => 70,
            _ => 100, // (and BATTERY_LEVEL_FULL)
        }
    } else {
        -1
    };
    (state, percent)
}

/// An input event of an XInput state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Input {
    Axis(u8, i16),
    Button(u8, bool),
    Hat(u8, u8),
}

/// The inputs `UpdateXInputJoystickState()` sends for a state, in order.
pub(super) fn state_inputs(state: &XINPUT_STATE) -> Vec<Input> {
    const XINPUT_BUTTONS: [u16; 11] = [
        XINPUT_GAMEPAD_A,
        XINPUT_GAMEPAD_B,
        XINPUT_GAMEPAD_X,
        XINPUT_GAMEPAD_Y,
        XINPUT_GAMEPAD_LEFT_SHOULDER,
        XINPUT_GAMEPAD_RIGHT_SHOULDER,
        XINPUT_GAMEPAD_BACK,
        XINPUT_GAMEPAD_START,
        XINPUT_GAMEPAD_LEFT_THUMB,
        XINPUT_GAMEPAD_RIGHT_THUMB,
        XINPUT_GAMEPAD_GUIDE,
    ];
    let pad = &state.Gamepad;
    let w_buttons = pad.wButtons;
    let mut hat = HAT_CENTERED;

    let mut out = vec![
        Input::Axis(0, pad.sThumbLX),
        Input::Axis(1, !pad.sThumbLY),
        Input::Axis(2, ((pad.bLeftTrigger as i32 * 257) - 32768) as i16),
        Input::Axis(3, pad.sThumbRX),
        Input::Axis(4, !pad.sThumbRY),
        Input::Axis(5, ((pad.bRightTrigger as i32 * 257) - 32768) as i16),
    ];

    for (button, &mask) in XINPUT_BUTTONS.iter().enumerate() {
        let down = (w_buttons & mask) != 0;
        out.push(Input::Button(button as u8, down));
    }

    if w_buttons & XINPUT_GAMEPAD_DPAD_UP != 0 {
        hat |= HAT_UP;
    }
    if w_buttons & XINPUT_GAMEPAD_DPAD_DOWN != 0 {
        hat |= HAT_DOWN;
    }
    if w_buttons & XINPUT_GAMEPAD_DPAD_LEFT != 0 {
        hat |= HAT_LEFT;
    }
    if w_buttons & XINPUT_GAMEPAD_DPAD_RIGHT != 0 {
        hat |= HAT_RIGHT;
    }
    out.push(Input::Hat(0, hat));
    out
}

/// Translation of `UpdateXInputJoystickState()`.
fn update_xinput_joystick_state(
    joystick: JoystickID,
    state: &XINPUT_STATE,
    battery_info: &XINPUT_BATTERY_INFORMATION_EX,
) {
    let timestamp = crate::timer::ticks_ns();

    for input in state_inputs(state) {
        match input {
            Input::Axis(axis, value) => send_joystick_axis(timestamp, joystick, axis, value),
            Input::Button(button, down) => send_joystick_button(timestamp, joystick, button, down),
            Input::Hat(hat, value) => send_joystick_hat(timestamp, joystick, hat, value),
        }
    }

    let (power, percent) = battery_information(battery_info);
    send_joystick_power_info(joystick, power, percent);
}

/// Translation of `SDL_XINPUT_JoystickRumble()`.
pub(super) fn joystick_rumble(
    hwdata: &HwData,
    low_frequency_rumble: u16,
    high_frequency_rumble: u16,
) -> Result<()> {
    let Some(x) = functions() else {
        return Err(Error::unsupported());
    };

    let mut vibration = XINPUT_VIBRATION {
        wLeftMotorSpeed: low_frequency_rumble,
        wRightMotorSpeed: high_frequency_rumble,
    };
    if x.set_state(hwdata.userid as u32, &mut vibration) != ERROR_SUCCESS {
        return Err(Error::new("XInputSetState() failed"));
    }
    Ok(())
}

/// Translation of `SDL_XINPUT_JoystickUpdate()`: reads the state of the
/// open joystick `joystick` with device data `(userid, packet number)`,
/// sends its events if it changed, and returns the new packet number.
pub(super) fn joystick_update(joystick: JoystickID, userid: u8, dw_packet_number: u32) -> u32 {
    let Some(x) = functions() else {
        return dw_packet_number;
    };

    let (result, xinput_state) = x.get_state(userid as u32);
    if result == ERROR_DEVICE_NOT_CONNECTED {
        return dw_packet_number;
    }

    // FIXME: This does end up making a device ioctl() to query data, we shouldn't do this every update.
    let battery = x
        .get_battery_information(userid as u32, BATTERY_DEVTYPE_GAMEPAD)
        .map(|(_, info)| info)
        .unwrap_or_default();

    // only fire events if the data changed from last time
    if xinput_state.dwPacketNumber != 0 && xinput_state.dwPacketNumber != dw_packet_number {
        update_xinput_joystick_state(joystick, &xinput_state, &battery);
        return xinput_state.dwPacketNumber;
    }
    dw_packet_number
}

/// Translation of `SDL_XINPUT_JoystickClose()`.
pub(super) fn joystick_close() {}

/// Translation of `SDL_XINPUT_JoystickQuit()`.
pub(super) fn joystick_quit() {
    if xinput_enabled() {
        XINPUT_ENABLED.store(false, Ordering::Relaxed);
        unload_xinput_dll();
    }
}
