// Rust translation of the joystick GUID and device classification functions
// of src/joystick/SDL_joystick.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Joystick GUIDs (their layout, creation and decoding) and the guesses of
//! what kind of device a joystick is from its USB IDs and GUID.

use super::controller_type::{guess_controller_name, guess_controller_type, ControllerType};
use super::gamepad::GamepadType;
use super::tables::*;
use super::usb_ids::*;
use super::vidpid::VidPidList;
use super::JoystickType;
use crate::guid::Guid;
use crate::hints;

// Device bus definitions
/// Translation of `SDL_HARDWARE_BUS_UNKNOWN`.
pub(crate) const HARDWARE_BUS_UNKNOWN: u16 = 0x00;
/// Translation of `SDL_HARDWARE_BUS_USB`.
pub(crate) const HARDWARE_BUS_USB: u16 = 0x03;
/// Translation of `SDL_HARDWARE_BUS_BLUETOOTH`.
#[allow(dead_code)] // (used by the Bluetooth drivers)
pub(crate) const HARDWARE_BUS_BLUETOOTH: u16 = 0x05;
/// Translation of `SDL_HARDWARE_BUS_VIRTUAL`.
pub(crate) const HARDWARE_BUS_VIRTUAL: u16 = 0xFF;

pub(super) static OLD_XBOXONE_CONTROLLERS: VidPidList = VidPidList::new(
    Some("SDL_JOYSTICK_OLD_XBOXONE_CONTROLLERS"),
    Some("SDL_JOYSTICK_OLD_XBOXONE_CONTROLLERS_EXCLUDED"),
    INITIAL_OLD_XBOXONE_CONTROLLERS,
);
pub(super) static ARCADESTICK_DEVICES: VidPidList = VidPidList::new(
    Some(hints::JOYSTICK_ARCADESTICK_DEVICES),
    Some(hints::JOYSTICK_ARCADESTICK_DEVICES_EXCLUDED),
    INITIAL_ARCADESTICK_DEVICES,
);
pub(super) static BLACKLIST_DEVICES: VidPidList = VidPidList::new(
    Some(hints::JOYSTICK_BLACKLIST_DEVICES),
    Some(hints::JOYSTICK_BLACKLIST_DEVICES_EXCLUDED),
    INITIAL_BLACKLIST_DEVICES,
);
pub(super) static FLIGHTSTICK_DEVICES: VidPidList = VidPidList::new(
    Some(hints::JOYSTICK_FLIGHTSTICK_DEVICES),
    Some(hints::JOYSTICK_FLIGHTSTICK_DEVICES_EXCLUDED),
    INITIAL_FLIGHTSTICK_DEVICES,
);
pub(super) static GAMECUBE_DEVICES: VidPidList = VidPidList::new(
    Some(hints::JOYSTICK_GAMECUBE_DEVICES),
    Some(hints::JOYSTICK_GAMECUBE_DEVICES_EXCLUDED),
    INITIAL_GAMECUBE_DEVICES,
);
pub(super) static ROG_GAMEPAD_MICE: VidPidList = VidPidList::new(
    Some(hints::ROG_GAMEPAD_MICE),
    Some(hints::ROG_GAMEPAD_MICE_EXCLUDED),
    INITIAL_ROG_GAMEPAD_MICE,
);
pub(super) static THROTTLE_DEVICES: VidPidList = VidPidList::new(
    Some(hints::JOYSTICK_THROTTLE_DEVICES),
    Some(hints::JOYSTICK_THROTTLE_DEVICES_EXCLUDED),
    INITIAL_THROTTLE_DEVICES,
);
pub(super) static WHEEL_DEVICES: VidPidList = VidPidList::new(
    Some(hints::JOYSTICK_WHEEL_DEVICES),
    Some(hints::JOYSTICK_WHEEL_DEVICES_EXCLUDED),
    INITIAL_WHEEL_DEVICES,
);
pub(super) static GUITAR_DEVICES: VidPidList = VidPidList::new(
    Some(hints::JOYSTICK_GUITAR_DEVICES),
    None,
    INITIAL_GUITAR_DEVICES,
);
pub(super) static DRUM_DEVICES: VidPidList = VidPidList::new(
    Some(hints::JOYSTICK_DRUM_DEVICES),
    None,
    INITIAL_DRUM_DEVICES,
);
pub(super) static ZERO_CENTERED_DEVICES: VidPidList = VidPidList::new(
    Some(hints::JOYSTICK_ZERO_CENTERED_DEVICES),
    None,
    INITIAL_ZERO_CENTERED_DEVICES,
);

/// The lists, in the order `SDL_InitJoysticks()` loads them.
pub(super) static VIDPID_LISTS: [&VidPidList; 11] = [
    &OLD_XBOXONE_CONTROLLERS,
    &ARCADESTICK_DEVICES,
    &BLACKLIST_DEVICES,
    &DRUM_DEVICES,
    &FLIGHTSTICK_DEVICES,
    &GAMECUBE_DEVICES,
    &GUITAR_DEVICES,
    &ROG_GAMEPAD_MICE,
    &THROTTLE_DEVICES,
    &WHEEL_DEVICES,
    &ZERO_CENTERED_DEVICES,
];

fn guid16(guid: &Guid, i: usize) -> u16 {
    u16::from_le_bytes([guid.0[i * 2], guid.0[i * 2 + 1]])
}

fn set_guid16(guid: &mut Guid, i: usize, value: u16) {
    guid.0[i * 2..i * 2 + 2].copy_from_slice(&value.to_le_bytes());
}

/// The USB vendor ID, product ID, product version and name CRC encoded in a
/// joystick GUID, each 0 if unavailable: `(vendor, product, version, crc16)`.
/// Translation of `SDL_GetJoystickGUIDInfo()`.
pub fn joystick_guid_info(guid: Guid) -> (u16, u16, u16, u16) {
    let bus = guid16(&guid, 0);

    if (bus < u16::from(b' ') || bus == HARDWARE_BUS_VIRTUAL)
        && guid16(&guid, 3) == 0x0000
        && guid16(&guid, 5) == 0x0000
    {
        /* This GUID fits the standard form:
         * 16-bit bus
         * 16-bit CRC16 of the joystick name (can be zero)
         * 16-bit vendor ID
         * 16-bit zero
         * 16-bit product ID
         * 16-bit zero
         * 16-bit version
         * 8-bit driver identifier ('h' for HIDAPI, 'x' for XInput, etc.)
         * 8-bit driver-dependent type info
         */
        (
            guid16(&guid, 2),
            guid16(&guid, 4),
            guid16(&guid, 6),
            guid16(&guid, 1),
        )
    } else if bus < u16::from(b' ') || bus == HARDWARE_BUS_VIRTUAL {
        /* This GUID fits the unknown VID/PID form:
         * 16-bit bus
         * 16-bit CRC16 of the joystick name (can be zero)
         * 11 characters of the joystick name, null terminated
         */
        (0, 0, 0, guid16(&guid, 1))
    } else {
        (0, 0, 0, 0)
    }
}

/// A standardized name for a controller. Translation of `SDL_CreateJoystickName()`.
#[allow(dead_code)] // (used by the platform drivers)
pub(crate) fn create_joystick_name(
    vendor: u16,
    product: u16,
    vendor_name: Option<&str>,
    product_name: Option<&str>,
) -> Option<String> {
    if let Some(custom_name) = guess_controller_name(vendor, product) {
        return Some(custom_name.to_string());
    }

    crate::utils::create_device_name(
        vendor,
        product,
        vendor_name,
        product_name,
        Some("Controller"),
    )
}

/// A GUID for a joystick, from its bus, USB IDs and name (and the driver's
/// signature byte and data byte). Translation of `SDL_CreateJoystickGUID()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_joystick_guid(
    bus: u16,
    vendor: u16,
    product: u16,
    version: u16,
    vendor_name: Option<&str>,
    product_name: Option<&str>,
    driver_signature: u8,
    driver_data: u8,
) -> Guid {
    let mut guid = Guid::ZERO;
    let mut crc = 0u16;

    match (vendor_name, product_name) {
        (Some(vendor_name), Some(product_name))
            if !vendor_name.is_empty() && !product_name.is_empty() =>
        {
            crc = crate::stdlib::crc16(crc, vendor_name.as_bytes());
            crc = crate::stdlib::crc16(crc, b" ");
            crc = crate::stdlib::crc16(crc, product_name.as_bytes());
        }
        (_, Some(product_name)) => {
            crc = crate::stdlib::crc16(crc, product_name.as_bytes());
        }
        _ => {}
    }

    // We only need 16 bits for each of these; space them out to fill 128.
    // Byteswap so devices get same GUID on little/big endian platforms.
    set_guid16(&mut guid, 0, bus);
    set_guid16(&mut guid, 1, crc);

    if vendor != 0 {
        set_guid16(&mut guid, 2, vendor);
        set_guid16(&mut guid, 4, product);
        set_guid16(&mut guid, 6, version);
        guid.0[14] = driver_signature;
        guid.0[15] = driver_data;
    } else {
        let mut available_space = guid.0.len() - 4;

        if driver_signature != 0 {
            available_space -= 2;
            guid.0[14] = driver_signature;
            guid.0[15] = driver_data;
        }
        if let Some(product_name) = product_name {
            // (SDL_strlcpy: up to available_space - 1 bytes and a terminator)
            let bytes = product_name.as_bytes();
            let n = bytes.len().min(available_space - 1);
            guid.0[4..4 + n].copy_from_slice(&bytes[..n]);
            guid.0[4 + n] = 0;
        }
    }
    guid
}

/// A GUID for a joystick known only by name.
/// Translation of `SDL_CreateJoystickGUIDForName()`.
#[allow(dead_code)] // (used by the platform drivers)
pub(crate) fn create_joystick_guid_for_name(name: &str) -> Guid {
    create_joystick_guid(HARDWARE_BUS_UNKNOWN, 0, 0, 0, None, Some(name), 0, 0)
}

/// Translation of `SDL_SetJoystickGUIDVendor()`.
#[allow(dead_code)] // (used by the platform drivers)
pub(crate) fn set_joystick_guid_vendor(guid: &mut Guid, vendor: u16) {
    set_guid16(guid, 2, vendor);
}

/// Translation of `SDL_SetJoystickGUIDProduct()`.
#[allow(dead_code)] // (used by the platform drivers)
pub(crate) fn set_joystick_guid_product(guid: &mut Guid, product: u16) {
    set_guid16(guid, 4, product);
}

/// Translation of `SDL_SetJoystickGUIDVersion()`.
pub(crate) fn set_joystick_guid_version(guid: &mut Guid, version: u16) {
    set_guid16(guid, 6, version);
}

/// Translation of `SDL_SetJoystickGUIDCRC()`.
pub(crate) fn set_joystick_guid_crc(guid: &mut Guid, crc: u16) {
    set_guid16(guid, 1, crc);
}

/// The gamepad type of a device. Translation of `SDL_GetGamepadTypeFromVIDPID()`.
pub(crate) fn gamepad_type_from_vidpid(
    vendor: u16,
    product: u16,
    name: Option<&str>,
    for_ui: bool,
) -> GamepadType {
    let mut gamepad_type = GamepadType::Standard;

    if vendor == 0x0000 && product == 0x0000 {
        // Some devices are only identifiable by their name
        if matches!(
            name,
            Some("Lic Pro Controller" | "Nintendo Wireless Gamepad" | "Wireless Gamepad")
        ) {
            // HORI or PowerA Switch Pro Controller clone
            gamepad_type = GamepadType::NintendoSwitchPro;
        }
    } else if vendor == 0x0001 && product == 0x0001 {
        gamepad_type = GamepadType::Standard;
    } else if vendor == USB_VENDOR_NINTENDO
        && (product == USB_PRODUCT_NINTENDO_SWITCH_JOYCON_LEFT
            || product == USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_LEFT)
    {
        gamepad_type = GamepadType::NintendoSwitchJoyconLeft;
    } else if vendor == USB_VENDOR_NINTENDO
        && (product == USB_PRODUCT_NINTENDO_SWITCH_JOYCON_RIGHT
            || product == USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_RIGHT)
    {
        if name.is_some_and(|n| n.contains("NES Controller")) {
            // We don't have a type for the Nintendo Online NES Controller
            gamepad_type = GamepadType::Standard;
        } else {
            gamepad_type = GamepadType::NintendoSwitchJoyconRight;
        }
    } else if vendor == USB_VENDOR_NINTENDO && product == USB_PRODUCT_NINTENDO_SWITCH_JOYCON_GRIP {
        if name.is_some_and(|n| n.contains("(L)")) {
            gamepad_type = GamepadType::NintendoSwitchJoyconLeft;
        } else if name.is_some_and(|n| n.contains("L+R")) {
            gamepad_type = GamepadType::NintendoSwitchJoyconPair;
        } else {
            gamepad_type = GamepadType::NintendoSwitchJoyconRight;
        }
    } else if vendor == USB_VENDOR_NINTENDO
        && (product == USB_PRODUCT_NINTENDO_SWITCH_JOYCON_PAIR
            || product == USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_PAIR)
    {
        gamepad_type = GamepadType::NintendoSwitchJoyconPair;
    } else if for_ui && is_joystick_gamecube(vendor, product) {
        gamepad_type = GamepadType::Gamecube;
    } else {
        use ControllerType as C;
        match guess_controller_type(vendor, product) {
            C::XBox360Controller => gamepad_type = GamepadType::Xbox360,
            C::XBoxOneController | C::XBoxEliteController => gamepad_type = GamepadType::XboxOne,
            C::PS3Controller => gamepad_type = GamepadType::Ps3,
            C::PS4Controller => gamepad_type = GamepadType::Ps4,
            C::PS5Controller | C::PS5EdgeController => gamepad_type = GamepadType::Ps5,
            C::XInputPS4Controller => {
                gamepad_type = if for_ui {
                    GamepadType::Ps4
                } else {
                    GamepadType::Standard
                };
            }
            C::SwitchProController
            | C::Switch2ProController
            | C::SwitchInputOnlyController
            | C::Switch2InputOnlyController => gamepad_type = GamepadType::NintendoSwitchPro,
            C::XInputSwitchController => {
                gamepad_type = if for_ui {
                    GamepadType::NintendoSwitchPro
                } else {
                    GamepadType::Xbox360
                };
            }
            C::SteamController
            | C::SteamControllerV2
            | C::SteamControllerNeptune
            | C::SteamControllerTriton
            | C::HoriSteamController
            | C::UnknownSteamController => gamepad_type = GamepadType::Steam,
            _ => {}
        }
    }
    gamepad_type
}

/// The gamepad type of a joystick GUID. Translation of `SDL_GetGamepadTypeFromGUID()`.
pub(crate) fn gamepad_type_from_guid(guid: Guid, name: Option<&str>) -> GamepadType {
    let (vendor, product, _, _) = joystick_guid_info(guid);
    let gamepad_type = gamepad_type_from_vidpid(vendor, product, name, true);
    if gamepad_type == GamepadType::Standard {
        if is_joystick_xinput(guid) {
            // This is probably an Xbox One controller
            return GamepadType::XboxOne;
        }
        #[cfg(any(windows, target_os = "linux"))]
        if is_joystick_hidapi(guid) {
            return super::hidapi::get_gamepad_type_from_guid(guid);
        }
    }
    gamepad_type
}

/// Whether a joystick GUID uses the version field.
/// Translation of `SDL_JoystickGUIDUsesVersion()`.
pub(crate) fn joystick_guid_uses_version(guid: Guid) -> bool {
    if is_joystick_mfi(guid) {
        // The version bits are used as button capability mask
        return false;
    }

    let (vendor, product, _, _) = joystick_guid_info(guid);
    vendor != 0 && product != 0
}

/// Translation of `SDL_IsJoystickXboxOne()`.
#[allow(dead_code)] // (used by the platform drivers)
pub(crate) fn is_joystick_xbox_one(vendor_id: u16, product_id: u16) -> bool {
    let e_type = guess_controller_type(vendor_id, product_id);
    e_type == ControllerType::XBoxOneController || e_type == ControllerType::XBoxEliteController
}

/// Translation of `SDL_IsJoystickXboxOneElite()`.
pub(crate) fn is_joystick_xbox_one_elite(vendor_id: u16, product_id: u16) -> bool {
    guess_controller_type(vendor_id, product_id) == ControllerType::XBoxEliteController
}

/// Translation of `SDL_IsJoystickXboxSeriesX()`.
pub(crate) fn is_joystick_xbox_series_x(vendor_id: u16, product_id: u16) -> bool {
    // Most new controllers have the share button, so we'll default to true and
    // have a list of older XBox One controllers that are known not to have it.
    !OLD_XBOXONE_CONTROLLERS.contains(vendor_id, product_id)
}

/// Translation of `SDL_IsJoystickBluetoothXboxOne()`.
#[allow(dead_code)] // (used by the HIDAPI drivers)
pub(crate) fn is_joystick_bluetooth_xbox_one(vendor_id: u16, product_id: u16) -> bool {
    vendor_id == USB_VENDOR_MICROSOFT
        && (product_id == USB_PRODUCT_XBOX_ONE_ADAPTIVE_BLUETOOTH
            || product_id == USB_PRODUCT_XBOX_ONE_ADAPTIVE_BLE
            || product_id == USB_PRODUCT_XBOX_ONE_S_REV1_BLUETOOTH
            || product_id == USB_PRODUCT_XBOX_ONE_S_REV2_BLUETOOTH
            || product_id == USB_PRODUCT_XBOX_ONE_S_REV2_BLE
            || product_id == USB_PRODUCT_XBOX_ONE_ELITE_SERIES_2_BLUETOOTH
            || product_id == USB_PRODUCT_XBOX_ONE_ELITE_SERIES_2_BLE
            || product_id == USB_PRODUCT_XBOX_SERIES_X_BLE)
}

/// Translation of `SDL_IsJoystickPS4()`.
#[allow(dead_code)] // (used by the HIDAPI drivers)
pub(crate) fn is_joystick_ps4(vendor_id: u16, product_id: u16) -> bool {
    guess_controller_type(vendor_id, product_id) == ControllerType::PS4Controller
}

/// Translation of `SDL_IsJoystickPS5()`.
#[allow(dead_code)] // (used by the HIDAPI drivers)
pub(crate) fn is_joystick_ps5(vendor_id: u16, product_id: u16) -> bool {
    let e_type = guess_controller_type(vendor_id, product_id);
    e_type == ControllerType::PS5Controller || e_type == ControllerType::PS5EdgeController
}

/// Translation of `SDL_IsJoystickDualSenseEdge()`.
pub(crate) fn is_joystick_dual_sense_edge(vendor_id: u16, product_id: u16) -> bool {
    guess_controller_type(vendor_id, product_id) == ControllerType::PS5EdgeController
}

/// Translation of `SDL_IsJoystickNintendoSwitchPro()`.
pub(crate) fn is_joystick_nintendo_switch_pro(vendor_id: u16, product_id: u16) -> bool {
    let e_type = guess_controller_type(vendor_id, product_id);
    e_type == ControllerType::SwitchProController
        || e_type == ControllerType::SwitchInputOnlyController
}

/// Translation of `SDL_IsJoystickNintendoSwitch2Pro()`.
pub(crate) fn is_joystick_nintendo_switch2_pro(vendor_id: u16, product_id: u16) -> bool {
    let e_type = guess_controller_type(vendor_id, product_id);
    e_type == ControllerType::Switch2ProController
        || e_type == ControllerType::Switch2InputOnlyController
}

/// Translation of `SDL_IsJoystickNintendoSwitchProInputOnly()`.
pub(crate) fn is_joystick_nintendo_switch_pro_input_only(vendor_id: u16, product_id: u16) -> bool {
    guess_controller_type(vendor_id, product_id) == ControllerType::SwitchInputOnlyController
}

/// Translation of `SDL_IsJoystickNintendoSwitch2ProInputOnly()`.
pub(crate) fn is_joystick_nintendo_switch2_pro_input_only(vendor_id: u16, product_id: u16) -> bool {
    guess_controller_type(vendor_id, product_id) == ControllerType::Switch2InputOnlyController
}

/// Translation of `SDL_IsJoystickNintendoSwitchJoyCon()`.
#[allow(dead_code)] // (used by the HIDAPI drivers)
pub(crate) fn is_joystick_nintendo_switch_joycon(vendor_id: u16, product_id: u16) -> bool {
    let e_type = guess_controller_type(vendor_id, product_id);
    e_type == ControllerType::SwitchJoyConLeft || e_type == ControllerType::SwitchJoyConRight
}

/// Translation of `SDL_IsJoystickNintendoSwitchJoyConLeft()`.
#[allow(dead_code)] // (used by the HIDAPI drivers)
pub(crate) fn is_joystick_nintendo_switch_joycon_left(vendor_id: u16, product_id: u16) -> bool {
    guess_controller_type(vendor_id, product_id) == ControllerType::SwitchJoyConLeft
}

/// Translation of `SDL_IsJoystickNintendoSwitchJoyConRight()`.
#[allow(dead_code)] // (used by the HIDAPI drivers)
pub(crate) fn is_joystick_nintendo_switch_joycon_right(vendor_id: u16, product_id: u16) -> bool {
    guess_controller_type(vendor_id, product_id) == ControllerType::SwitchJoyConRight
}

/// Translation of `SDL_IsJoystickNintendoSwitchJoyConGrip()`.
#[allow(dead_code)] // (used by the HIDAPI drivers)
pub(crate) fn is_joystick_nintendo_switch_joycon_grip(vendor_id: u16, product_id: u16) -> bool {
    vendor_id == USB_VENDOR_NINTENDO && product_id == USB_PRODUCT_NINTENDO_SWITCH_JOYCON_GRIP
}

/// Translation of `SDL_IsJoystickNintendoSwitchJoyConPair()`.
pub(crate) fn is_joystick_nintendo_switch_joycon_pair(vendor_id: u16, product_id: u16) -> bool {
    vendor_id == USB_VENDOR_NINTENDO
        && (product_id == USB_PRODUCT_NINTENDO_SWITCH_JOYCON_PAIR
            || product_id == USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_PAIR)
}

/// Whether a joystick is a Nintendo GameCube style controller.
/// Translation of `SDL_IsJoystickGameCube()`.
pub(crate) fn is_joystick_gamecube(vendor_id: u16, product_id: u16) -> bool {
    GAMECUBE_DEVICES.contains(vendor_id, product_id)
}

/// Translation of `SDL_IsJoystickAmazonLunaController()`.
pub(crate) fn is_joystick_amazon_luna_controller(vendor_id: u16, product_id: u16) -> bool {
    (vendor_id == USB_VENDOR_AMAZON && product_id == USB_PRODUCT_AMAZON_LUNA_CONTROLLER)
        || (vendor_id == BLUETOOTH_VENDOR_AMAZON && product_id == BLUETOOTH_PRODUCT_LUNA_CONTROLLER)
}

/// Translation of `SDL_IsJoystickGoogleStadiaController()`.
pub(crate) fn is_joystick_google_stadia_controller(vendor_id: u16, product_id: u16) -> bool {
    vendor_id == USB_VENDOR_GOOGLE && product_id == USB_PRODUCT_GOOGLE_STADIA_CONTROLLER
}

/// Translation of `SDL_IsJoystickNVIDIASHIELDController()`.
pub(crate) fn is_joystick_nvidia_shield_controller(vendor_id: u16, product_id: u16) -> bool {
    vendor_id == USB_VENDOR_NVIDIA
        && (product_id == USB_PRODUCT_NVIDIA_SHIELD_CONTROLLER_V103
            || product_id == USB_PRODUCT_NVIDIA_SHIELD_CONTROLLER_V104)
}

/// Whether a joystick is a Steam Virtual Gamepad.
/// Translation of `SDL_IsJoystickSteamVirtualGamepad()`.
pub(crate) fn is_joystick_steam_virtual_gamepad(
    vendor_id: u16,
    product_id: u16,
    version: u16,
) -> bool {
    if cfg!(target_os = "macos") {
        vendor_id == USB_VENDOR_MICROSOFT
            && product_id == USB_PRODUCT_XBOX360_WIRED_CONTROLLER
            && version == 0
    } else {
        let _ = version;
        vendor_id == USB_VENDOR_VALVE && product_id == USB_PRODUCT_STEAM_VIRTUAL_GAMEPAD
    }
}

/// Translation of `SDL_IsJoystickSteamController()`.
pub(crate) fn is_joystick_steam_controller(vendor_id: u16, product_id: u16) -> bool {
    let e_type = guess_controller_type(vendor_id, product_id);
    e_type == ControllerType::SteamController || e_type == ControllerType::SteamControllerV2
}

/// Translation of `SDL_IsJoystickHoriSteamController()`.
pub(crate) fn is_joystick_hori_steam_controller(vendor_id: u16, product_id: u16) -> bool {
    guess_controller_type(vendor_id, product_id) == ControllerType::HoriSteamController
}

/// Whether a joystick is an SInput (Open Format) controller.
/// Translation of `SDL_IsJoystickSInputController()`.
pub(crate) fn is_joystick_sinput_controller(vendor_id: u16, product_id: u16) -> bool {
    if vendor_id == USB_VENDOR_RASPBERRYPI
        && (product_id == USB_PRODUCT_HANDHELDLEGEND_SINPUT_GENERIC
            || product_id == USB_PRODUCT_HANDHELDLEGEND_PROGCC
            || product_id == USB_PRODUCT_HANDHELDLEGEND_GCULTIMATE
            || product_id == USB_PRODUCT_BONZIRICHANNEL_FIREBIRD
            || product_id == USB_PRODUCT_VOIDGAMING_PS4FIREBIRD)
    {
        return true;
    }
    vendor_id == USB_VENDOR_ANDGAMER && product_id == USB_PRODUCT_VOIDGAMING_GENESIS_SINPUT
}

/// Translation of `SDL_IsJoystickFlydigiController()`.
pub(crate) fn is_joystick_flydigi_controller(vendor_id: u16, product_id: u16) -> bool {
    if vendor_id == USB_VENDOR_FLYDIGI_V1 && product_id == USB_PRODUCT_FLYDIGI_V1_GAMEPAD {
        return true;
    }
    vendor_id == USB_VENDOR_FLYDIGI_V2
        && (product_id == USB_PRODUCT_FLYDIGI_V2_APEX
            || product_id == USB_PRODUCT_FLYDIGI_V2_VADER
            || product_id == USB_PRODUCT_FLYDIGI_V2_APEX6)
}

/// Translation of `SDL_IsJoystickGameSirController()`.
pub(crate) fn is_joystick_gamesir_controller(vendor_id: u16, product_id: u16) -> bool {
    if vendor_id != USB_VENDOR_GAMESIR {
        return false;
    }
    product_id == USB_PRODUCT_GAMESIR_GAMEPAD_G7_PRO_8K
        || product_id == USB_PRODUCT_GAMESIR_GAMEPAD_TARANTULA_8K
}

/// Translation of `SDL_IsJoystickSteamDeck()`.
pub(crate) fn is_joystick_steam_deck(vendor_id: u16, product_id: u16) -> bool {
    guess_controller_type(vendor_id, product_id) == ControllerType::SteamControllerNeptune
}

/// Translation of `SDL_IsJoystickSteamTriton()`.
pub(crate) fn is_joystick_steam_triton(vendor_id: u16, product_id: u16) -> bool {
    guess_controller_type(vendor_id, product_id) == ControllerType::SteamControllerTriton
}

/// Whether a joystick GUID comes from the XInput driver. Translation of `SDL_IsJoystickXInput()`.
pub(crate) fn is_joystick_xinput(guid: Guid) -> bool {
    guid.0[14] == b'x'
}

/// Whether a joystick GUID comes from the WGI driver. Translation of `SDL_IsJoystickWGI()`.
pub(crate) fn is_joystick_wgi(guid: Guid) -> bool {
    guid.0[14] == b'w'
}

/// Whether a joystick GUID comes from the GameInput driver. Translation of `SDL_IsJoystickGameInput()`.
pub(crate) fn is_joystick_gameinput(guid: Guid) -> bool {
    guid.0[14] == b'g'
}

/// Whether a joystick GUID comes from the HIDAPI driver. Translation of `SDL_IsJoystickHIDAPI()`.
pub(crate) fn is_joystick_hidapi(guid: Guid) -> bool {
    guid.0[14] == b'h'
}

/// Whether a joystick GUID comes from the MFI driver. Translation of `SDL_IsJoystickMFI()`.
pub(crate) fn is_joystick_mfi(guid: Guid) -> bool {
    guid.0[14] == b'm'
}

/// Whether a joystick GUID comes from the RAWINPUT driver. Translation of `SDL_IsJoystickRAWINPUT()`.
pub(crate) fn is_joystick_rawinput(guid: Guid) -> bool {
    guid.0[14] == b'r'
}

/// Whether a joystick GUID comes from the virtual driver. Translation of `SDL_IsJoystickVIRTUAL()`.
pub(crate) fn is_joystick_virtual_guid(guid: Guid) -> bool {
    guid.0[14] == b'v'
}

/// Whether a joystick is a wheel. Translation of `SDL_IsJoystickWheel()`.
pub(crate) fn is_joystick_wheel(vendor_id: u16, product_id: u16, crc: u16) -> bool {
    if vendor_id == 0x11FF && product_id == 0x3331 && (crc == 0xFAF6 || crc == 0x2004) {
        // Oklick W-2 racing wheel controller (on Windows via DirectInput).
        return true;
    }
    WHEEL_DEVICES.contains(vendor_id, product_id)
}

fn is_joystick_arcade_stick(vendor_id: u16, product_id: u16) -> bool {
    ARCADESTICK_DEVICES.contains(vendor_id, product_id)
}

fn is_joystick_flight_stick(vendor_id: u16, product_id: u16) -> bool {
    FLIGHTSTICK_DEVICES.contains(vendor_id, product_id)
}

fn is_joystick_throttle(vendor_id: u16, product_id: u16) -> bool {
    THROTTLE_DEVICES.contains(vendor_id, product_id)
}

fn is_joystick_guitar(vendor_id: u16, product_id: u16) -> bool {
    GUITAR_DEVICES.contains(vendor_id, product_id)
}

fn is_joystick_drum_kit(vendor_id: u16, product_id: u16) -> bool {
    DRUM_DEVICES.contains(vendor_id, product_id)
}

/// The kind of joystick a GUID describes. Translation of `SDL_GetJoystickGUIDType()`.
pub(crate) fn joystick_guid_type(guid: Guid) -> JoystickType {
    let (vendor, product, _, crc) = joystick_guid_info(guid);

    if is_joystick_wheel(vendor, product, crc) {
        return JoystickType::Wheel;
    }

    if is_joystick_arcade_stick(vendor, product) {
        return JoystickType::ArcadeStick;
    }

    if is_joystick_flight_stick(vendor, product) {
        return JoystickType::FlightStick;
    }

    if is_joystick_throttle(vendor, product) {
        return JoystickType::Throttle;
    }

    if is_joystick_guitar(vendor, product) {
        return JoystickType::Guitar;
    }

    if is_joystick_drum_kit(vendor, product) {
        return JoystickType::DrumKit;
    }

    if is_joystick_xinput(guid) {
        // XInput GUID, get the type based on the XInput device subtype
        return match guid.0[15] {
            0x01 => JoystickType::Gamepad,     // XINPUT_DEVSUBTYPE_GAMEPAD
            0x02 => JoystickType::Wheel,       // XINPUT_DEVSUBTYPE_WHEEL
            0x03 => JoystickType::ArcadeStick, // XINPUT_DEVSUBTYPE_ARCADE_STICK
            0x04 => JoystickType::FlightStick, // XINPUT_DEVSUBTYPE_FLIGHT_STICK
            0x05 => JoystickType::DancePad,    // XINPUT_DEVSUBTYPE_DANCE_PAD
            // XINPUT_DEVSUBTYPE_GUITAR, XINPUT_DEVSUBTYPE_GUITAR_ALTERNATE, XINPUT_DEVSUBTYPE_GUITAR_BASS
            0x06 | 0x07 | 0x0B => JoystickType::Guitar,
            0x08 => JoystickType::DrumKit, // XINPUT_DEVSUBTYPE_DRUM_KIT
            0x13 => JoystickType::ArcadePad, // XINPUT_DEVSUBTYPE_ARCADE_PAD
            _ => JoystickType::Unknown,
        };
    }

    if is_joystick_wgi(guid) || is_joystick_gameinput(guid) || is_joystick_virtual_guid(guid) {
        // (a value past the last joystick type reads as unknown)
        return JoystickType::from_u8(guid.0[15]);
    }

    #[cfg(any(windows, target_os = "linux"))]
    if is_joystick_hidapi(guid) {
        return super::hidapi::get_joystick_type_from_guid(guid);
    }

    if guess_controller_type(vendor, product) != ControllerType::UnknownNonSteamController {
        return JoystickType::Gamepad;
    }

    JoystickType::Unknown
}

/// Whether a joystick should be ignored. Translation of `SDL_ShouldIgnoreJoystick()`.
#[allow(dead_code)] // (used by the platform drivers)
pub(crate) fn should_ignore_joystick(
    vendor_id: u16,
    product_id: u16,
    version: u16,
    name: Option<&str>,
) -> bool {
    // Check the joystick blacklist
    if BLACKLIST_DEVICES.contains(vendor_id, product_id) {
        return true;
    }
    if !hints::get_bool(hints::JOYSTICK_ROG_CHAKRAM, false)
        && ROG_GAMEPAD_MICE.contains(vendor_id, product_id)
    {
        return true;
    }

    super::gamepad::should_ignore_gamepad(vendor_id, product_id, version, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_layout() {
        let guid = create_joystick_guid(
            HARDWARE_BUS_USB,
            0x045e,
            0x028e,
            0x0114,
            Some("Microsoft"),
            Some("X-Box 360 pad"),
            b'h',
            0,
        );
        assert_eq!(guid.to_string()[..4], *"0300");
        assert_eq!(guid.to_string()[8..], *"5e0400008e02000014016800");
        let (vendor, product, version, crc) = joystick_guid_info(guid);
        assert_eq!((vendor, product, version), (0x045e, 0x028e, 0x0114));
        let expected_crc = crate::stdlib::crc16(0, b"Microsoft X-Box 360 pad");
        assert_eq!(crc, expected_crc);

        // Name-only GUIDs keep the start of the name
        let guid = create_joystick_guid_for_name("A Long Joystick Name");
        assert_eq!(&guid.0[4..16], b"A Long Joys\0");
        assert_eq!(joystick_guid_info(guid).0, 0);
        let guid = create_joystick_guid(
            HARDWARE_BUS_VIRTUAL,
            0,
            0,
            0,
            None,
            Some("Virtual Joystick"),
            b'v',
            0,
        );
        assert_eq!(&guid.0[4..14], b"Virtual J\0");
        assert!(is_joystick_virtual_guid(guid));
        assert_eq!(joystick_guid_type(guid), JoystickType::Unknown);
    }

    #[test]
    fn gamepad_types() {
        assert_eq!(
            gamepad_type_from_vidpid(0x045e, 0x028e, None, true),
            GamepadType::Xbox360
        );
        assert_eq!(
            gamepad_type_from_vidpid(0x054c, 0x0ce6, None, true),
            GamepadType::Ps5
        );
        assert_eq!(
            gamepad_type_from_vidpid(0, 0, Some("Wireless Gamepad"), true),
            GamepadType::NintendoSwitchPro
        );
        assert_eq!(
            gamepad_type_from_vidpid(
                USB_VENDOR_NINTENDO,
                USB_PRODUCT_NINTENDO_SWITCH_JOYCON_GRIP,
                Some("Joy-Con L+R"),
                true
            ),
            GamepadType::NintendoSwitchJoyconPair
        );
    }
}
