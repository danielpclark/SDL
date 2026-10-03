// Rust translation of src/joystick/controller_type.c and controller_type.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Steam's controller models, and the guesses of a controller's model and
//! name from its USB vendor and product IDs.

use std::cmp::Ordering;

use super::tables::{make_vidpid, CONTROLLERS};
use crate::hints;
use crate::stdlib::string::strncasecmp;

/// Steam Controller models. Translation of `EControllerType`.
///
/// WARNING: DO NOT RENUMBER EXISTING VALUES - STORED IN A DATABASE
#[allow(clippy::enum_variant_names)]
#[allow(dead_code)] // (some models are only named by the tables)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum ControllerType {
    None = -1,
    Unknown = 0,

    // Steam Controllers
    UnknownSteamController = 1,
    SteamController = 2,
    SteamControllerV2 = 3,
    SteamControllerNeptune = 4,

    SteamControllerTriton = 10,

    // Other Controllers
    UnknownNonSteamController = 30,
    XBox360Controller = 31,
    XBoxOneController = 32,
    PS3Controller = 33,
    PS4Controller = 34,
    WiiController = 35,
    AppleController = 36,
    AndroidController = 37,
    SwitchProController = 38,
    SwitchJoyConLeft = 39,
    SwitchJoyConRight = 40,
    SwitchJoyConPair = 41,
    SwitchInputOnlyController = 42,
    MobileTouch = 43,
    /// Client-side only, used to mark Nintendo Switch style controllers as using XInput instead of the Nintendo Switch protocol
    XInputSwitchController = 44,
    PS5Controller = 45,
    XBoxEliteController = 46,
    /// Client-side only, used to mark DualShock 4 style controllers using XInput instead of the DualShock 4 controller protocol
    XInputPS4Controller = 47,
    PS5EdgeController = 48,
    HoriSteamController = 49,
    /// `k_eControllerType_8BitDoController`
    EightBitDoController = 50,
    Switch2ProController = 51,
    Switch2InputOnlyController = 52,
    /// Don't add game controllers below this enumeration - this enumeration can change value
    LastController,

    // Keyboards and Mice
    GenericKeyboard = 400,
    GenericMouse = 800,
}

/// Translation of `ControllerDescription_t`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ControllerDescription {
    pub(crate) device_id: u32,
    pub(crate) controller_type: ControllerType,
    pub(crate) name: Option<&'static str>,
}

/// The controller type override for a device in `SDL_HINT_GAMECONTROLLERTYPE`
/// (the text after `0xVVVV/0xPPPP=`, without a `k_eControllerType_` prefix).
/// Translation of `GetControllerTypeOverride()`.
fn controller_type_override(vid: u16, pid: u16) -> Option<String> {
    let hint = hints::get(hints::GAMECONTROLLERTYPE)?;
    let mut key = format!("0x{vid:04x}/0x{pid:04x}=");
    let mut spot = hint.find(&key);
    if spot.is_none() {
        key = format!("0x{vid:04X}/0x{pid:04X}=");
        spot = hint.find(&key);
    }
    let mut rest = &hint[spot? + key.len()..];
    if let Some(stripped) = rest.strip_prefix("k_eControllerType_") {
        rest = stripped;
    }
    Some(rest.to_string())
}

/// The controller type of a device. Translation of `GuessControllerType()`.
pub(crate) fn guess_controller_type(vid: u16, pid: u16) -> ControllerType {
    let device_id = make_vidpid(vid, pid);

    if let Some(over) = controller_type_override(vid, pid) {
        let starts = |prefix: &str| strncasecmp(&over, prefix, prefix.len()) == Ordering::Equal;
        if starts("Xbox360") {
            return ControllerType::XBox360Controller;
        }
        if starts("XboxOne") {
            return ControllerType::XBoxOneController;
        }
        if starts("PS3") {
            return ControllerType::PS3Controller;
        }
        if starts("PS4") {
            return ControllerType::PS4Controller;
        }
        if starts("PS5") {
            return ControllerType::PS5Controller;
        }
        if starts("SwitchPro") {
            return ControllerType::SwitchProController;
        }
        if starts("Steam") {
            return ControllerType::SteamController;
        }
        return ControllerType::UnknownNonSteamController;
    }

    CONTROLLERS
        .iter()
        .find(|c| c.device_id == device_id)
        .map_or(ControllerType::UnknownNonSteamController, |c| {
            c.controller_type
        })
}

/// The name of a known controller. Translation of `GuessControllerName()`.
#[allow(dead_code)] // (used by the platform drivers)
pub(crate) fn guess_controller_name(vid: u16, pid: u16) -> Option<&'static str> {
    let device_id = make_vidpid(vid, pid);
    CONTROLLERS
        .iter()
        .find(|c| c.device_id == device_id)
        .and_then(|c| c.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses() {
        assert_eq!(
            guess_controller_type(0x054c, 0x0268),
            ControllerType::PS3Controller
        );
        assert_eq!(
            guess_controller_type(0x045e, 0x028e),
            ControllerType::XBox360Controller
        );
        assert_eq!(
            guess_controller_name(0x045e, 0x028e),
            Some("Xbox 360 Controller")
        );
        assert_eq!(
            guess_controller_type(0x1234, 0x5678),
            ControllerType::UnknownNonSteamController
        );
        assert_eq!(guess_controller_name(0x054c, 0x0268), None);
        // the entry written `MAKE_CONTROLLER_ID (0x9886, 0x0024 )` upstream
        assert_eq!(
            guess_controller_type(0x9886, 0x0024),
            ControllerType::XInputPS4Controller
        );
    }
}
