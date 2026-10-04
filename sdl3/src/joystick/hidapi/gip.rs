// Rust translation of src/joystick/hidapi/SDL_hidapi_gip.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The GIP driver: wired Xbox One and Xbox Series controllers, and the
//! devices plugged into them (the chatpad, as a keyboard), speaking the
//! Game Input Protocol directly. It comes before the Xbox One driver in
//! the driver list, so it takes the wired controllers when it's enabled.
//!
//! This driver is based on the Microsoft GIP spec at:
//! <https://aka.ms/gipdocs>
//! <https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-gipusb/e7c90904-5e21-426e-b9ad-d82adeee0dbc>
//!
//! The device's attachments (the controller is attachment 0) each get the
//! handshake: the hello, the metadata (requested, or faked when the
//! controller doesn't answer and the reset hint is off), then the init
//! sequence that connects the joystick. Upstream's packet dumps
//! (`DEBUG_XBOX_PROTOCOL`, compiled out) and the protocol constants it
//! doesn't use are left out.
//!
//! Not translated: the macOS GCController check, whose backend isn't
//! translated.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::rumble::{lock_rumble, RumbleSentCallback};
use super::{
    DeviceCtx, DriverContext, DriverImpl, HidapiDevice, JoystickCaps, SDL_HIDAPI_DEFAULT,
    USB_PACKET_LENGTH,
};
use crate::error::{Error, Result};
use crate::events::keyboard::{keymap, Keymod, Scancode};
use crate::events::{JoystickID, KeyboardID};
use crate::hints;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::{
    is_joystick_xbox_series_x, with_joystick, JoystickData, JoystickType, HAT_DOWN, HAT_LEFT,
    HAT_RIGHT, HAT_UP, RUMBLE_RESEND_MS,
};
use crate::log::Category;
use crate::power::PowerState;

const MAX_MESSAGE_LENGTH: u64 = 0x4000;
const MAX_ATTACHMENTS: usize = 8;

const GIP_DATA_CLASS_SHIFT: u8 = 5;

// System messages
const GIP_CMD_PROTO_CONTROL: u8 = 0x01;
const GIP_CMD_HELLO_DEVICE: u8 = 0x02;
const GIP_CMD_STATUS_DEVICE: u8 = 0x03;
const GIP_CMD_METADATA: u8 = 0x04;
const GIP_CMD_SET_DEVICE_STATE: u8 = 0x05;
const GIP_CMD_SECURITY: u8 = 0x06;
const GIP_CMD_GUIDE_BUTTON: u8 = 0x07;
const GIP_CMD_AUDIO_CONTROL: u8 = 0x08;
const GIP_CMD_LED: u8 = 0x0a;
const GIP_CMD_HID_REPORT: u8 = 0x0b;
const GIP_CMD_FIRMWARE: u8 = 0x0c;
const GIP_CMD_EXTENDED: u8 = 0x1e;
const GIP_AUDIO_DATA: u8 = 0x60;

// Navigation vendor messages
const GIP_CMD_DIRECT_MOTOR: u8 = 0x09;
const GIP_LL_INPUT_REPORT: u8 = 0x20;
const GIP_LL_OVERFLOW_INPUT_REPORT: u8 = 0x26;

// Wheel and ArcadeStick vendor messages
const GIP_CMD_INITIAL_REPORTS_REQUEST: u8 = 0x0a;
const GIP_LL_STATIC_CONFIGURATION: u8 = 0x21;
const GIP_LL_BUTTON_INFO_REPORT: u8 = 0x22;

// FlightStick vendor messages
const GIP_CMD_DEVICE_CAPABILITIES: u8 = 0x00;

// Undocumented Elite 2 vendor messages
const GIP_CMD_RAW_REPORT: u8 = 0x0c;
const GIP_CMD_GUIDE_COLOR: u8 = 0x0e;
const GIP_SL_ELITE_CONFIG: u8 = 0x4d;

const GIP_BTN_OFFSET_XBE1: usize = 28;
const GIP_BTN_OFFSET_XBE2: usize = 14;

const GIP_FLAG_FRAGMENT: u8 = 1 << 7;
const GIP_FLAG_INIT_FRAG: u8 = 1 << 6;
const GIP_FLAG_SYSTEM: u8 = 1 << 5;
const GIP_FLAG_ACME: u8 = 1 << 4;
const GIP_FLAG_ATTACHMENT_MASK: u8 = 0x7;

// Protocol Control constants
const GIP_CONTROL_CODE_ACK: u8 = 0;

// Status Device constants
const GIP_NOT_CHARGING: u8 = 0;
const GIP_CHARGING: u8 = 1;

const GIP_BATTERY_ABSENT: u8 = 0;
const GIP_BATTERY_STANDARD: u8 = 1;
const GIP_BATTERY_RECHARGEABLE: u8 = 2;

const GIP_BATTERY_CRITICAL: u8 = 0;
const GIP_BATTERY_LOW: u8 = 1;
const GIP_BATTERY_MEDIUM: u8 = 2;
const GIP_BATTERY_FULL: u8 = 3;

// Metadata constants
const GIP_MESSAGE_FLAG_DOWNSTREAM: u32 = 1 << 3;
const GIP_MESSAGE_FLAG_UPSTREAM: u32 = 1 << 4;
const GIP_MESSAGE_FLAG_DS_REQUEST_RESPONSE: u32 = 1 << 5;

// Set Device State constants
const GIP_STATE_START: u8 = 0;
const GIP_STATE_UNK6: u8 = 6;
const GIP_STATE_RESET: u8 = 7;

// Guide Button Status constants
const GIP_LED_GUIDE: u8 = 0;

const GIP_LED_GUIDE_ON: u8 = 1;

// Direct Motor Command constants
const GIP_MOTOR_ALL: u8 = 0xF;

// Extended Command constants
const GIP_EXTCMD_GET_SERIAL_NUMBER: u8 = 0x04;

const GIP_EXTENDED_STATUS_OK: u8 = 0;

// Internal constants, not part of protocol
const GIP_HELLO_TIMEOUT: u64 = 2000;
const GIP_ACME_TIMEOUT: i32 = 10;

const GIP_DEFAULT_IN_SYSTEM_MESSAGES: u32 = 0x5e;
const GIP_DEFAULT_OUT_SYSTEM_MESSAGES: u32 = 0x472;

const GIP_FEATURE_CONSOLE_FUNCTION_MAP: u32 = 1 << 0;
const GIP_FEATURE_CONSOLE_FUNCTION_MAP_OVERFLOW: u32 = 1 << 1;
const GIP_FEATURE_ELITE_BUTTONS: u32 = 1 << 2;
const GIP_FEATURE_DYNAMIC_LATENCY_INPUT: u32 = 1 << 3;
const GIP_FEATURE_SECURITY_OPT_OUT: u32 = 1 << 4;
const GIP_FEATURE_MOTOR_CONTROL: u32 = 1 << 5;
const GIP_FEATURE_GUIDE_COLOR: u32 = 1 << 6;
const GIP_FEATURE_EXTENDED_SET_DEVICE_STATE: u32 = 1 << 7;

const GIP_QUIRK_NO_HELLO: u32 = 1 << 0;
const GIP_QUIRK_NO_IMPULSE_VIBRATION: u32 = 1 << 2;

/// Translation of `GIP_MetadataStatus`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum MetadataStatus {
    #[default]
    None,
    Got,
    Faked,
    Pending,
}

/// `VK_LWIN`
const VK_LWIN: u8 = 0x5b;

/// Translation of `GIP_AttachmentType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum AttachmentType {
    Unknown,
    #[default]
    Gamepad,
    ArcadeStick,
    Wheel,
    FlightStick,
    NavigationController,
    Chatpad,
    Headset,
    Guitar,
    DrumKit,
    LiveGuitar,
}

/// Translation of `GIP_RumbleState`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum RumbleState {
    #[default]
    Idle,
    Queued,
    Busy,
}

/// Translation of `GIP_EliteButtonFormat`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum EliteButtonFormat {
    #[default]
    Unknown,
    Xbe1,
    Xbe2Raw,
    Xbe24,
    Xbe25,
}

/// An interface GUID. These come across the wire as little-endian, so
/// they're kept as such, to be compared bytewise (`MAKE_GUID()`).
type InterfaceGuid = [u8; 16];

const fn make_guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> InterfaceGuid {
    let a = a.to_le_bytes();
    let b = b.to_le_bytes();
    let c = c.to_le_bytes();
    [
        a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6],
        d[7],
    ]
}

const GUID_ARCADE_STICK: InterfaceGuid = make_guid(
    0x332054cc,
    0xa34b,
    0x41d5,
    [0xa3, 0x4a, 0xa6, 0xa6, 0x71, 0x1e, 0xc4, 0xb3],
);
const GUID_DYNAMIC_LATENCY_INPUT: InterfaceGuid = make_guid(
    0x87f2e56b,
    0xc3bb,
    0x49b1,
    [0x82, 0x65, 0xff, 0xff, 0xf3, 0x77, 0x99, 0xee],
);
const GUID_FLIGHT_STICK: InterfaceGuid = make_guid(
    0x03f1a011,
    0xefe9,
    0x4cc1,
    [0x96, 0x9c, 0x38, 0xdc, 0x55, 0xf4, 0x04, 0xd0],
);
const GUID_IHEADSET: InterfaceGuid = make_guid(
    0xbc25d1a3,
    0xc24e,
    0x4992,
    [0x9d, 0xda, 0xef, 0x4f, 0x12, 0x3e, 0xf5, 0xdc],
);
const GUID_ICONSOLE_FUNCTION_MAP_INPUT_REPORT: InterfaceGuid = make_guid(
    0xecddd2fe,
    0xd387,
    0x4294,
    [0xbd, 0x96, 0x1a, 0x71, 0x2e, 0x3d, 0xc7, 0x7d],
);
const GUID_ICONSOLE_FUNCTION_MAP_OVERFLOW_INPUT_REPORT: InterfaceGuid = make_guid(
    0x137d4bd0,
    0x9347,
    0x4472,
    [0xaa, 0x26, 0x8c, 0x34, 0xa0, 0x8f, 0xf9, 0xbd],
);
const GUID_ICONTROLLER: InterfaceGuid = make_guid(
    0x9776ff56,
    0x9bfd,
    0x4581,
    [0xad, 0x45, 0xb6, 0x45, 0xbb, 0xa5, 0x26, 0xd6],
);
const GUID_IDEV_AUTH_PC_OPT_OUT: InterfaceGuid = make_guid(
    0x7a34ce77,
    0x7de2,
    0x45c6,
    [0x8c, 0xa4, 0x00, 0x42, 0xc0, 0x8b, 0xd9, 0x4a],
);
const GUID_IELITE_BUTTONS: InterfaceGuid = make_guid(
    0x37d19ff7,
    0xb5c6,
    0x49d1,
    [0xa7, 0x5e, 0x03, 0xb2, 0x4b, 0xef, 0x8c, 0x89],
);
const GUID_IGAMEPAD: InterfaceGuid = make_guid(
    0x082e402c,
    0x07df,
    0x45e1,
    [0xa5, 0xab, 0xa3, 0x12, 0x7a, 0xf1, 0x97, 0xb5],
);
const GUID_NAVIGATION_CONTROLLER: InterfaceGuid = make_guid(
    0xb8f31fe7,
    0x7386,
    0x40e9,
    [0xa9, 0xf8, 0x2f, 0x21, 0x26, 0x3a, 0xcf, 0xb7],
);
const GUID_WHEEL: InterfaceGuid = make_guid(
    0x646979cf,
    0x6b71,
    0x4e96,
    [0x8d, 0xf9, 0x59, 0xe3, 0x98, 0xd7, 0x42, 0x0c],
);
const GUID_PDP_GUITAR: InterfaceGuid = make_guid(
    0x1a266af6,
    0x3a46,
    0x45e3,
    [0xb9, 0xb6, 0x0f, 0x2c, 0x0b, 0x2c, 0x1e, 0xbe],
);
const GUID_MADCATZ_GUITAR: InterfaceGuid = make_guid(
    0x0d2ae438,
    0x7f7d,
    0x4933,
    [0x86, 0x93, 0x30, 0xfc, 0x55, 0x01, 0x8e, 0x77],
);
const GUID_PDP_DRUM_KIT: InterfaceGuid = make_guid(
    0xa503f9b0,
    0x955e,
    0x47c4,
    [0xa2, 0xed, 0xb1, 0x33, 0x6f, 0xa7, 0x70, 0x3e],
);
const GUID_MADCATZ_DRUM_KIT: InterfaceGuid = make_guid(
    0x06182893,
    0xCCE0,
    0x4B85,
    [0x92, 0x71, 0x0A, 0x10, 0xDB, 0xAB, 0x7E, 0x07],
);
const GUID_GUITAR_HERO_LIVE_GUITAR: InterfaceGuid = make_guid(
    0xfd12fdd9,
    0x8e73,
    0x47c7,
    [0xa2, 0x31, 0x96, 0x26, 0x8c, 0x38, 0x00, 0x9a],
);

/*
 * The following GUIDs are observed, but the exact meanings aren't known, so
 * for now we document them but don't use them anywhere.
 *
 * MAKE_GUID(GUID_GamepadEmu, 0xe2e5f1bc, 0xa6e6, 0x41a2, 0x8f, 0x43, 0x33, 0xcf, 0xa2, 0x51, 0x09, 0x81);
 * MAKE_GUID(GUID_IAudioOnly, 0x92844cd1, 0xf7c8, 0x49ef, 0x97, 0x77, 0x46, 0x7d, 0xa7, 0x08, 0xad, 0x10);
 * MAKE_GUID(GUID_IControllerProfileModeState, 0xf758dc66, 0x022c, 0x48b8, 0xa4, 0xf6, 0x45, 0x7b, 0xa8, 0x0e, 0x2a, 0x5b);
 * MAKE_GUID(GUID_ICustomAudio, 0x63fd9cc9, 0x94ee, 0x4b5d, 0x9c, 0x4d, 0x8b, 0x86, 0x4c, 0x14, 0x9c, 0xac);
 * MAKE_GUID(GUID_IExtendedDeviceFlags, 0x34ad9b1e, 0x36ad, 0x4fb5, 0x8a, 0xc7, 0x17, 0x23, 0x4c, 0x9f, 0x54, 0x6f);
 * MAKE_GUID(GUID_IProgrammableGamepad, 0x31c1034d, 0xb5b7, 0x4551, 0x98, 0x13, 0x87, 0x69, 0xd4, 0xa0, 0xe4, 0xf9);
 * MAKE_GUID(GUID_IVirtualDevice, 0xdfd26825, 0x110a, 0x4e94, 0xb9, 0x37, 0xb2, 0x7c, 0xe4, 0x7b, 0x25, 0x40);
 * MAKE_GUID(GUID_OnlineDevAuth, 0x632b1fd1, 0xa3e9, 0x44f9, 0x84, 0x20, 0x5c, 0xe3, 0x44, 0xa0, 0x64, 0x04);
 *
 * Seen on Elite Controller, Adaptive Controller: 9ebd00a3-b5e6-4c08-a33b-673126459ec4
 * Seen on Adaptive Controller: ce1e58c5-221c-4bdb-9c24-bf3941601320
 * Seen on Elite 2 Controller: f758dc66-022c-48b8-a4f6-457ba80e2a5b (IControllerProfileModeState)
 * Seen on Elite 2 Controller: 31c1034d-b5b7-4551-9813-8769d4a0e4f9 (IProgrammableGamepad)
 * Seen on Elite 2 Controller: 34ad9b1e-36ad-4fb5-8ac7-17234c9f546f (IExtendedDeviceFlags)
 * Seen on Elite 2 Controller: 88e0b694-6bd9-4416-a560-e7fafdfa528f
 * Seen on Elite 2 Controller: ea96c8c0-b216-448b-be80-7e5deb0698e2
 */

/// The most a message of each data class can carry (`GIP_DataClassMtu`).
const DATA_CLASS_MTU: [usize; 8] = [64, 64, 64, 2048, 0, 0, 0, 0];

/// Translation of `GIP_Quirks`.
struct Quirks {
    vendor_id: u16,
    product_id: u16,
    attachment_index: u8,
    added_features: u32,
    filtered_features: u32,
    quirks: u32,
    extra_in_system: [u32; 8],
    extra_out_system: [u32; 8],
    device_type: AttachmentType,
    extra_buttons: u8,
    extra_axes: u8,
}

impl Quirks {
    const fn new(vendor_id: u16, product_id: u16) -> Quirks {
        Quirks {
            vendor_id,
            product_id,
            attachment_index: 0,
            added_features: 0,
            filtered_features: 0,
            quirks: 0,
            extra_in_system: [0; 8],
            extra_out_system: [0; 8],
            device_type: AttachmentType::Gamepad,
            extra_buttons: 0,
            extra_axes: 0,
        }
    }
}

/// Translation of `quirks`.
const QUIRKS: [Quirks; 9] = [
    Quirks {
        added_features: GIP_FEATURE_ELITE_BUTTONS,
        filtered_features: GIP_FEATURE_CONSOLE_FUNCTION_MAP,
        ..Quirks::new(USB_VENDOR_MICROSOFT, USB_PRODUCT_XBOX_ONE_ELITE_SERIES_1)
    },
    Quirks {
        added_features: GIP_FEATURE_ELITE_BUTTONS
            | GIP_FEATURE_DYNAMIC_LATENCY_INPUT
            | GIP_FEATURE_CONSOLE_FUNCTION_MAP
            | GIP_FEATURE_GUIDE_COLOR
            | GIP_FEATURE_EXTENDED_SET_DEVICE_STATE,
        extra_in_system: [1 << GIP_CMD_FIRMWARE, 0, 0, 0, 0, 0, 0, 0],
        extra_out_system: [1 << GIP_CMD_FIRMWARE, 0, 0, 0, 0, 0, 0, 0],
        ..Quirks::new(USB_VENDOR_MICROSOFT, USB_PRODUCT_XBOX_ONE_ELITE_SERIES_2)
    },
    Quirks {
        added_features: GIP_FEATURE_DYNAMIC_LATENCY_INPUT,
        ..Quirks::new(USB_VENDOR_MICROSOFT, USB_PRODUCT_XBOX_SERIES_X)
    },
    Quirks {
        quirks: GIP_QUIRK_NO_HELLO,
        ..Quirks::new(USB_VENDOR_PDP, USB_PRODUCT_PDP_ROCK_CANDY)
    },
    Quirks {
        filtered_features: GIP_FEATURE_MOTOR_CONTROL,
        ..Quirks::new(USB_VENDOR_POWERA, USB_PRODUCT_BDA_XB1_FIGHTPAD)
    },
    Quirks {
        quirks: GIP_QUIRK_NO_IMPULSE_VIBRATION,
        ..Quirks::new(USB_VENDOR_POWERA, USB_PRODUCT_BDA_XB1_CLASSIC)
    },
    Quirks {
        quirks: GIP_QUIRK_NO_IMPULSE_VIBRATION,
        ..Quirks::new(USB_VENDOR_POWERA, USB_PRODUCT_BDA_XB1_SPECTRA_PRO)
    },
    Quirks {
        filtered_features: GIP_FEATURE_MOTOR_CONTROL,
        device_type: AttachmentType::ArcadeStick,
        ..Quirks::new(USB_VENDOR_RAZER, USB_PRODUCT_RAZER_ATROX)
    },
    Quirks {
        filtered_features: GIP_FEATURE_MOTOR_CONTROL,
        device_type: AttachmentType::FlightStick,
        extra_buttons: 5,
        extra_axes: 3,
        ..Quirks::new(
            USB_VENDOR_THRUSTMASTER,
            USB_PRODUCT_THRUSTMASTER_T_FLIGHT_HOTAS_ONE,
        )
    },
];

/// Translation of `GIP_Header`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct Header {
    message_type: u8,
    flags: u8,
    sequence_id: u8,
    length: u64,
}

/// Translation of `GIP_DeviceMetadata`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct DeviceMetadata {
    in_system_messages: [u32; 8],
    out_system_messages: [u32; 8],

    // FIXME (upstream): the audio formats are two bytes each, but only
    // as many bytes as there are formats are copied. They aren't used.
    audio_formats: Vec<u8>,
    /// The types, up to their first NUL (as the C strings)
    preferred_types: Vec<Vec<u8>>,
    supported_interfaces: Vec<InterfaceGuid>,
    hid_descriptor: Vec<u8>,
}

/// Translation of `GIP_MessageMetadata`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct MessageMetadata {
    message_type: u8,
    length: u16,
    data_type: u16,
    flags: u32,
    period: u16,
    persistence_timeout: u16,
}

/// Translation of `GIP_Metadata`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct Metadata {
    version_major: u16,
    version_minor: u16,

    device: DeviceMetadata,

    message_metadata: Vec<MessageMetadata>,
}

/// What the chatpad does with the keyboard API, done when the driver
/// function is done with the device's attachments.
#[derive(Clone, PartialEq, Eq, Debug)]
enum KeyboardAction {
    /// `SDL_AddKeyboard()`
    Add(KeyboardID),
    /// `SDL_RemoveKeyboard()`
    Remove(KeyboardID),
    /// `SDL_SendKeyboardKey()`
    Key(u64, KeyboardID, Scancode, bool),
    /// `SDL_SendKeyboardText()`
    Text(String),
}

/// The name of the chatpad keyboard.
const CHATPAD_NAME: &str = "Xbox One Chatpad";

/// Do the chatpad's keyboard actions.
fn send_keyboard_actions(actions: Vec<KeyboardAction>) {
    use crate::events::keyboard as kb;
    for action in actions {
        match action {
            KeyboardAction::Add(keyboard) => kb::add_keyboard(keyboard, Some(CHATPAD_NAME)),
            KeyboardAction::Remove(keyboard) => kb::remove_keyboard(keyboard),
            KeyboardAction::Key(timestamp, keyboard, scancode, down) => {
                kb::send_keyboard_key(Duration::from_nanos(timestamp), keyboard, 0, scancode, down);
            }
            KeyboardAction::Text(text) => kb::send_keyboard_text(&text),
        }
    }
}

/// A keyboard ID for a chatpad. Upstream uses the attachment's address;
/// here the IDs count down from the top of the range, away from the small
/// IDs of the keyboard backends.
fn new_keyboard_id() -> KeyboardID {
    static NEXT_KEYBOARD_ID: AtomicU32 = AtomicU32::new(u32::MAX);
    NEXT_KEYBOARD_ID.fetch_sub(1, Ordering::Relaxed)
}

/// What the GIP driver does through its device (the `SDL_hid_*()` calls on
/// `device->dev`, the rumble queue, the clock and the joystick lookups):
/// the HIDAPI device, or a test's fake one.
pub(crate) trait GipPort {
    /// `SDL_hid_read_timeout()`
    fn read_timeout(&self, data: &mut [u8], milliseconds: i32) -> Result<usize>;
    /// `SDL_hid_write()`
    fn write(&self, data: &[u8]) -> Result<usize>;
    /// `SDL_HIDAPI_LockRumble()` and
    /// `SDL_HIDAPI_SendRumbleWithCallbackAndUnlock()`: queue a report for
    /// the rumble thread; `on_sent` gets the ticks when it was sent.
    fn send_async(
        &self,
        data: &[u8],
        on_sent: Option<Box<dyn FnOnce(u64) + Send>>,
    ) -> Result<usize>;
    /// `SDL_GetTicks()`
    fn ticks_ms(&self) -> u64;
    /// Whether a joystick is open (`SDL_GetJoystickFromID() != NULL`).
    fn joystick_open(&self, joystick: JoystickID) -> bool;
}

/// The HIDAPI device as the GIP driver uses it.
struct HidapiPort<'a>(&'a Arc<HidapiDevice>);

impl GipPort for HidapiPort<'_> {
    fn read_timeout(&self, data: &mut [u8], milliseconds: i32) -> Result<usize> {
        self.0
            .dev()
            .ok_or_else(|| Error::invalid_param("device"))?
            .read_timeout_ms(data, milliseconds)
    }
    fn write(&self, data: &[u8]) -> Result<usize> {
        self.0
            .dev()
            .ok_or_else(|| Error::invalid_param("device"))?
            .write(data)
    }
    fn send_async(
        &self,
        data: &[u8],
        on_sent: Option<Box<dyn FnOnce(u64) + Send>>,
    ) -> Result<usize> {
        let lock = lock_rumble()?;
        let callback = on_sent.map(|on_sent| -> RumbleSentCallback {
            Box::new(move || on_sent(crate::timer::ticks_ms()))
        });
        lock.send_with_callback_and_unlock(self.0, data, callback)
    }
    fn ticks_ms(&self) -> u64 {
        crate::timer::ticks_ms()
    }
    fn joystick_open(&self, joystick: JoystickID) -> bool {
        joystick != 0 && with_joystick(joystick, |_| ()).is_some()
    }
}

/// The device and the front end, as a driver function gives them to the
/// attachments (the `attachment->device->device` of upstream).
struct Io<'a, 'b> {
    port: &'a dyn GipPort,
    device: &'a mut DeviceCtx<'b>,
    /// The chatpads' keyboard actions, done when the function returns
    keyboard: Vec<KeyboardAction>,
}

impl<'a, 'b> Io<'a, 'b> {
    fn new(port: &'a dyn GipPort, device: &'a mut DeviceCtx<'b>) -> Io<'a, 'b> {
        Io {
            port,
            device,
            keyboard: Vec::new(),
        }
    }
}

/// The bytes of a message for the handlers that read fixed offsets: the
/// message in a buffer of at least a USB packet.
// FIXME (upstream): the handlers of the guide button, the gamepad and the
// Elite 2 paddles read fixed offsets past the end of a short message (into
// the read buffer, or past the reassembled fragments); here the bytes past
// the message read as 0.
fn padded(data: &[u8]) -> Vec<u8> {
    let mut packet = data.to_vec();
    if packet.len() < USB_PACKET_LENGTH {
        packet.resize(USB_PACKET_LENGTH, 0);
    }
    packet
}

/// Translation of `GIP_DecodeLength()`: the length and the bytes it took.
fn decode_length(bytes: &[u8]) -> (u64, usize) {
    let mut length = 0u64;
    let mut offset = 0;

    while offset < bytes.len() {
        let byte = bytes[offset];
        // FIXME (upstream): past nine bytes, the shift is wider than the
        // length (undefined in C); here it wraps, as the x86 shift does.
        length |= u64::from(byte & 0x7f).wrapping_shl((offset * 7) as u32);
        offset += 1;
        if byte & 0x80 == 0 {
            break;
        }
    }
    (length, offset)
}

/// Translation of `GIP_EncodeLength()`: the bytes written to `bytes`.
fn encode_length(mut length: u64, bytes: &mut [u8]) -> usize {
    let mut offset = 0;

    while offset < bytes.len() {
        let mut byte = (length & 0x7f) as u8;
        length >>= 7;
        if length != 0 {
            byte |= 0x80;
        }
        bytes[offset] = byte;
        offset += 1;
        if length == 0 {
            break;
        }
    }
    offset
}

/// Translation of `GIP_SendRawMessage()`; `asynchronous` sends through the
/// rumble thread, which calls `on_sent` once it's sent.
fn send_raw_message(
    port: &dyn GipPort,
    message_type: u8,
    flags: u8,
    seq: u8,
    bytes: &[u8],
    asynchronous: bool,
    on_sent: Option<Box<dyn FnOnce(u64) + Send>>,
) -> bool {
    if bytes.len() > DATA_CLASS_MTU[usize::from(message_type >> GIP_DATA_CLASS_SHIFT)] {
        crate::log::error!(
            Category::Input,
            "Attempted to send a message that requires fragmenting, which is not yet supported."
        );
        return false;
    }

    let mut buffer = Vec::with_capacity(bytes.len() + 6);
    buffer.extend_from_slice(&[message_type, flags, seq]);
    let mut length = [0u8; 10];
    let length_size = encode_length(bytes.len() as u64, &mut length);
    buffer.extend_from_slice(&length[..length_size]);
    buffer.extend_from_slice(bytes);

    let sent = if asynchronous {
        port.send_async(&buffer, on_sent)
    } else {
        port.write(&buffer)
    };
    sent.ok() == Some(buffer.len())
}

/// Translation of `GIP_Acknowledge()`.
fn acknowledge(
    port: &dyn GipPort,
    header: &Header,
    fragment_offset: u32,
    bytes_remaining: u16,
) -> bool {
    let offset = fragment_offset.to_le_bytes();
    let remaining = bytes_remaining.to_le_bytes();
    let buffer = [
        GIP_CONTROL_CODE_ACK,
        header.message_type,
        header.flags & GIP_FLAG_SYSTEM,
        offset[0],
        offset[1],
        offset[2],
        offset[3],
        remaining[0],
        remaining[1],
    ];

    send_raw_message(
        port,
        GIP_CMD_PROTO_CONTROL,
        GIP_FLAG_SYSTEM | (header.flags & GIP_FLAG_ATTACHMENT_MASK),
        header.sequence_id,
        &buffer,
        false,
        None,
    )
}

/// A little-endian 16-bit value of a message.
fn le16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

/// A little-endian 32-bit value of a message.
fn le32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

/// The bits of the system messages a list of a metadata block names.
fn add_system_messages(messages: &mut [u32; 8], list: &[u8]) {
    for &message in list {
        messages[usize::from(message >> 5)] |= 1u32 << (message & 0x1F);
    }
}

/// Translation of `GIP_ParseDeviceMetadata()`, at `*offset` in `bytes`.
fn parse_device_metadata(metadata: &mut Metadata, bytes: &[u8], offset: &mut usize) -> bool {
    let device = &mut metadata.device;

    let bytes = &bytes[*offset..];
    let num_bytes = bytes.len();
    if num_bytes < 16 {
        return false;
    }

    let length = usize::from(le16(bytes, 0));
    if num_bytes < length {
        return false;
    }

    /* Skip supported firmware versions for now */

    let buffer_offset = usize::from(le16(bytes, 4));
    if buffer_offset >= length {
        return false;
    }
    if buffer_offset > 0 {
        let num_audio_formats = usize::from(bytes[buffer_offset]);
        if buffer_offset + num_audio_formats + 1 > length {
            return false;
        }
        device.audio_formats = bytes[buffer_offset + 1..][..num_audio_formats].to_vec();
    }

    let buffer_offset = usize::from(le16(bytes, 6));
    if buffer_offset >= length {
        return false;
    }
    if buffer_offset > 0 {
        let count = usize::from(bytes[buffer_offset]);
        if buffer_offset + count + 1 > length {
            return false;
        }

        add_system_messages(
            &mut device.in_system_messages,
            &bytes[buffer_offset + 1..][..count],
        );
    }

    let buffer_offset = usize::from(le16(bytes, 8));
    if buffer_offset >= length {
        return false;
    }
    if buffer_offset > 0 {
        let count = usize::from(bytes[buffer_offset]);
        if buffer_offset + count + 1 > length {
            return false;
        }

        add_system_messages(
            &mut device.out_system_messages,
            &bytes[buffer_offset + 1..][..count],
        );
    }

    let mut buffer_offset = usize::from(le16(bytes, 10));
    if buffer_offset >= length {
        return false;
    }
    if buffer_offset > 0 {
        let num_preferred_types = bytes[buffer_offset];
        buffer_offset += 1;
        for _ in 0..num_preferred_types {
            if buffer_offset + 2 >= length {
                return false;
            }

            // FIXME (upstream): the length's high byte is never read (the
            // low byte is or-ed in twice), so it's the low byte alone.
            let count = usize::from(bytes[buffer_offset]);
            buffer_offset += 2;
            if buffer_offset + count > length {
                return false;
            }

            let text = &bytes[buffer_offset..][..count];
            let end = text.iter().position(|&b| b == 0).unwrap_or(count);
            device.preferred_types.push(text[..end].to_vec());
            buffer_offset += count;
        }
    }

    let buffer_offset = usize::from(le16(bytes, 12));
    if buffer_offset >= length {
        return false;
    }
    if buffer_offset > 0 {
        let num_supported_interfaces = usize::from(bytes[buffer_offset]);
        if buffer_offset + 1 + num_supported_interfaces * size_of::<InterfaceGuid>() > length {
            return false;
        }
        device.supported_interfaces = bytes[buffer_offset + 1..]
            .chunks_exact(size_of::<InterfaceGuid>())
            .take(num_supported_interfaces)
            .map(|guid| guid.try_into().unwrap_or_default())
            .collect();
    }

    if metadata.version_major > 1 || metadata.version_minor >= 1 {
        /* HID descriptor support added in metadata version 1.1 */
        let buffer_offset = usize::from(le16(bytes, 14));
        if buffer_offset >= length {
            return false;
        }
        if buffer_offset > 0 {
            let hid_descriptor_size = usize::from(bytes[buffer_offset]);
            if buffer_offset + 1 + hid_descriptor_size > length {
                return false;
            }
            metadata.device.hid_descriptor =
                bytes[buffer_offset + 1..][..hid_descriptor_size].to_vec();
        }
    }

    *offset += length;
    true
}

/// Translation of `GIP_ParseMessageMetadata()`, at `*offset` in `bytes`.
fn parse_message_metadata(bytes: &[u8], offset: &mut usize) -> Option<MessageMetadata> {
    let bytes = &bytes[*offset..];
    let num_bytes = bytes.len();

    if num_bytes < 2 {
        return None;
    }
    let length = usize::from(le16(bytes, 0));
    if num_bytes < length {
        return None;
    }

    if length < 15 {
        return None;
    }

    let metadata = MessageMetadata {
        message_type: bytes[2],
        length: le16(bytes, 3),
        data_type: le16(bytes, 5),
        flags: le32(bytes, 7),
        period: le16(bytes, 11),
        persistence_timeout: le16(bytes, 13),
    };

    *offset += length;
    Some(metadata)
}

/// Translation of `GIP_ParseMetadata()`.
fn parse_metadata(bytes: &[u8]) -> Option<Metadata> {
    let num_bytes = bytes.len();
    let mut metadata = Metadata::default();

    if num_bytes < 16 {
        return None;
    }

    let header_size = usize::from(le16(bytes, 0));
    if num_bytes < header_size || header_size < 16 {
        return None;
    }
    metadata.version_major = le16(bytes, 2);
    metadata.version_minor = le16(bytes, 4);
    /* Middle bytes are reserved */
    let metadata_size = usize::from(le16(bytes, 14));

    if num_bytes < metadata_size || metadata_size < header_size {
        return None;
    }
    let mut offset = header_size;

    if !parse_device_metadata(&mut metadata, bytes, &mut offset) {
        return None;
    }

    if offset >= num_bytes {
        return None;
    }
    let num_messages = bytes[offset];
    offset += 1;
    for _ in 0..num_messages {
        metadata
            .message_metadata
            .push(parse_message_metadata(bytes, &mut offset)?);
    }

    Some(metadata)
}

/// One step of a sequence number (`seq = x++; if (!seq) seq = x++;`).
fn next_sequence(seq: &mut u8) -> u8 {
    let mut next = *seq;
    *seq = seq.wrapping_add(1);
    if next == 0 {
        next = *seq;
        *seq = seq.wrapping_add(1);
    }
    next
}

/// The state of the device shared by its attachments (the `GIP_Device`
/// besides its attachments).
#[derive(Debug, Default)]
struct GipDevice {
    hello_deadline: u64,
    got_hello: bool,
    reset_for_metadata: bool,
    timeout: i32,
}

/// Translation of `GIP_Attachment`.
#[derive(Debug)]
struct Attachment {
    attachment_index: u8,
    joystick: JoystickID,
    keyboard: KeyboardID,

    fragment_message: u8,
    total_length: u16,
    fragment_data: Option<Vec<u8>>,
    fragment_offset: u32,
    fragment_timer: u64,
    fragment_retries: i32,

    firmware_major_version: u16,
    firmware_minor_version: u16,

    got_metadata: MetadataStatus,
    metadata_next: u64,
    metadata_retries: i32,
    metadata: Metadata,

    seq_system: u8,
    seq_security: u8,
    seq_extended: u8,
    seq_audio: u8,
    seq_vendor: u8,

    device_state: u8,

    rumble_state: RumbleState,
    /// Set by the rumble thread when a rumble packet was sent
    /// (`HIDAPI_DriverGIP_RumbleSent()`).
    rumble_time: Arc<AtomicU64>,
    rumble_pending: bool,
    left_impulse_level: u8,
    right_impulse_level: u8,
    left_vibration_level: u8,
    right_vibration_level: u8,

    last_input: [u8; 64],

    last_modifiers: u8,
    capslock: bool,
    last_key: u8,
    altcode: u32,
    altcode_digit: i32,

    attachment_type: AttachmentType,
    xbe_format: EliteButtonFormat,
    features: u32,
    quirks: u32,
    share_button_idx: u8,
    paddle_idx: u8,

    extra_button_idx: u8,
    extra_buttons: i32,
    extra_axes: i32,
}

impl Attachment {
    /// The new attachment of `GIP_EnsureAttachment()`.
    fn new(attachment_index: u8) -> Attachment {
        let mut metadata = Metadata::default();
        metadata.device.in_system_messages[0] = GIP_DEFAULT_IN_SYSTEM_MESSAGES;
        metadata.device.out_system_messages[0] = GIP_DEFAULT_OUT_SYSTEM_MESSAGES;
        Attachment {
            attachment_index,
            joystick: 0,
            keyboard: 0,
            fragment_message: 0,
            total_length: 0,
            fragment_data: None,
            fragment_offset: 0,
            fragment_timer: 0,
            fragment_retries: 0,
            firmware_major_version: 0,
            firmware_minor_version: 0,
            got_metadata: MetadataStatus::None,
            metadata_next: 0,
            metadata_retries: 0,
            metadata,
            seq_system: 0,
            seq_security: 0,
            seq_extended: 0,
            seq_audio: 0,
            seq_vendor: 0,
            device_state: 0,
            rumble_state: RumbleState::Idle,
            rumble_time: Arc::new(AtomicU64::new(0)),
            rumble_pending: false,
            left_impulse_level: 0,
            right_impulse_level: 0,
            left_vibration_level: 0,
            right_vibration_level: 0,
            last_input: [0; 64],
            last_modifiers: 0,
            capslock: false,
            last_key: 0,
            altcode: 0,
            altcode_digit: 0,
            attachment_type: if attachment_index > 0 {
                AttachmentType::Unknown
            } else {
                AttachmentType::Gamepad
            },
            xbe_format: EliteButtonFormat::Unknown,
            features: 0,
            quirks: 0,
            share_button_idx: 0,
            paddle_idx: 0,
            extra_button_idx: 0,
            extra_buttons: 0,
            extra_axes: 0,
        }
    }

    /// Translation of `GIP_SupportsSystemMessage()`.
    fn supports_system_message(&self, command: u8, upstream: bool) -> bool {
        let messages = if upstream {
            &self.metadata.device.in_system_messages
        } else {
            &self.metadata.device.out_system_messages
        };
        // Note (upstream): the shift isn't masked (undefined in C for the
        // messages past 31); it is here, which is what's meant.
        messages[usize::from(command >> 5)] & 1u32.wrapping_shl(u32::from(command)) != 0
    }

    /// Translation of `GIP_SupportsVendorMessage()`.
    fn supports_vendor_message(&self, command: u8, upstream: bool) -> bool {
        for metadata in &self.metadata.message_metadata {
            if metadata.message_type != command {
                continue;
            }
            if metadata.flags & GIP_MESSAGE_FLAG_DS_REQUEST_RESPONSE != 0 {
                return true;
            }
            if upstream {
                return metadata.flags & GIP_MESSAGE_FLAG_UPSTREAM != 0;
            } else {
                return metadata.flags & GIP_MESSAGE_FLAG_DOWNSTREAM != 0;
            }
        }
        false
    }

    /// Translation of `GIP_SequenceNext()`.
    fn sequence_next(&mut self, command: u8, system: bool) -> u8 {
        if system {
            match command {
                GIP_CMD_SECURITY => next_sequence(&mut self.seq_security),
                GIP_CMD_EXTENDED => next_sequence(&mut self.seq_extended),
                GIP_AUDIO_DATA => next_sequence(&mut self.seq_audio),
                _ => next_sequence(&mut self.seq_system),
            }
        } else {
            if command == GIP_CMD_DIRECT_MOTOR {
                // The motor sequence number is optional and always works with 0
                return 0;
            }

            next_sequence(&mut self.seq_vendor)
        }
    }

    /// Translation of `GIP_HandleQuirks()`.
    fn handle_quirks(&mut self, vendor_id: u16, product_id: u16) {
        for quirk in &QUIRKS {
            if quirk.vendor_id != vendor_id {
                continue;
            }
            if quirk.product_id != product_id {
                continue;
            }
            if quirk.attachment_index != self.attachment_index {
                continue;
            }
            self.features |= quirk.added_features;
            self.features &= !quirk.filtered_features;
            self.quirks = quirk.quirks;
            self.attachment_type = quirk.device_type;

            for j in 0..8 {
                self.metadata.device.in_system_messages[j] |= quirk.extra_in_system[j];
                self.metadata.device.out_system_messages[j] |= quirk.extra_out_system[j];
            }

            self.extra_buttons = i32::from(quirk.extra_buttons);
            self.extra_axes = i32::from(quirk.extra_axes);
            break;
        }
    }

    /// Translation of `GIP_SendSystemMessage()`.
    fn send_system_message(
        &mut self,
        port: &dyn GipPort,
        message_type: u8,
        flags: u8,
        bytes: &[u8],
    ) -> bool {
        let seq = self.sequence_next(message_type, true);
        send_raw_message(
            port,
            message_type,
            GIP_FLAG_SYSTEM | self.attachment_index | flags,
            seq,
            bytes,
            false,
            None,
        )
    }

    /// Translation of `GIP_SendVendorMessage()`.
    fn send_vendor_message(
        &mut self,
        port: &dyn GipPort,
        message_type: u8,
        flags: u8,
        bytes: &[u8],
    ) -> bool {
        let seq = self.sequence_next(message_type, false);
        send_raw_message(port, message_type, flags, seq, bytes, true, None)
    }

    /// Translation of `GIP_AttachmentIsController()`.
    fn is_controller(&self) -> bool {
        self.attachment_type != AttachmentType::Chatpad
            && self.attachment_type != AttachmentType::Headset
    }

    /// Translation of `GIP_FragmentFailed()`.
    fn fragment_failed(&mut self, port: &dyn GipPort, header: &Header) -> bool {
        self.fragment_retries += 1;
        if self.fragment_retries > 8 {
            self.fragment_data = None;
            self.fragment_message = 0;
        }
        acknowledge(
            port,
            header,
            self.fragment_offset,
            (u32::from(self.total_length).wrapping_sub(self.fragment_offset)) as u16,
        )
    }

    /// Translation of `GIP_EnableEliteButtons()`.
    fn enable_elite_buttons(&mut self, io: &mut Io<'_, '_>) -> bool {
        if io.device.vendor_id() == USB_VENDOR_MICROSOFT {
            if io.device.product_id() == USB_PRODUCT_XBOX_ONE_ELITE_SERIES_1 {
                self.xbe_format = EliteButtonFormat::Xbe1;
            } else if io.device.product_id() == USB_PRODUCT_XBOX_ONE_ELITE_SERIES_2 {
                if self.firmware_major_version == 4 {
                    self.xbe_format = EliteButtonFormat::Xbe24;
                } else if self.firmware_major_version == 5 {
                    /*
                     * The exact range for this being necessary is unknown, but it
                     * starts at 5.11 and at either 5.16 or 5.17. This approach
                     * still works on 5.21, even if it's not necessary, so having
                     * a loose upper limit is fine.
                     */
                    if self.firmware_minor_version >= 11 && self.firmware_minor_version < 17 {
                        self.xbe_format = EliteButtonFormat::Xbe2Raw;
                    } else {
                        self.xbe_format = EliteButtonFormat::Xbe25;
                    }
                }
            }
        }
        if self.xbe_format == EliteButtonFormat::Xbe2Raw {
            /*
             * The meaning of this packet is unknown and not documented, but it's
             * needed for the Elite 2 controller to send raw reports
             */
            const ENABLE_RAW_REPORT: [u8; 2] = [7, 0];

            return self.send_vendor_message(io.port, GIP_SL_ELITE_CONFIG, 0, &ENABLE_RAW_REPORT);
        }

        true
    }

    /// Translation of `GIP_SendGuideButtonLED()`.
    fn send_guide_button_led(&mut self, port: &dyn GipPort, pattern: u8, intensity: u8) -> bool {
        let buffer = [GIP_LED_GUIDE, pattern, intensity];

        if !self.supports_system_message(GIP_CMD_LED, false) {
            return true;
        }
        self.send_system_message(port, GIP_CMD_LED, 0, &buffer)
    }

    /// Translation of `GIP_SendQueryFirmware()`.
    fn send_query_firmware(&mut self, port: &dyn GipPort, slot: u8) -> bool {
        /* The "slot" variable might not be correct; the packet format is still unclear */
        let buffer = [0x1, slot, 0, 0, 0];

        self.send_system_message(port, GIP_CMD_FIRMWARE, 0, &buffer)
    }

    /// Translation of `GIP_SendSetDeviceState()`.
    fn send_set_device_state(&mut self, port: &dyn GipPort, state: u8) -> bool {
        let buffer = [state];
        let flags = self.attachment_index;
        self.send_system_message(port, GIP_CMD_SET_DEVICE_STATE, flags, &buffer)
    }

    /// Translation of `GIP_SendInitSequence()`.
    fn send_init_sequence(&mut self, io: &mut Io<'_, '_>) -> bool {
        if self.features & GIP_FEATURE_EXTENDED_SET_DEVICE_STATE != 0 {
            /*
             * The meaning of this packet is unknown and not documented, but it's
             * needed for the Elite 2 controller to start up on older firmwares
             */
            const SET_DEVICE_STATE: [u8; 15] = [
                GIP_STATE_UNK6,
                0x0,
                0x0,
                0x0,
                0x0,
                0x0,
                0x0,
                0x55,
                0x53,
                0x0,
                0x0,
                0x0,
                0x0,
                0x0,
                0x0,
            ];

            if !self.send_system_message(io.port, GIP_CMD_SET_DEVICE_STATE, 0, &SET_DEVICE_STATE) {
                return false;
            }
        }
        if !self.enable_elite_buttons(io) {
            return false;
        }
        if !self.send_set_device_state(io.port, GIP_STATE_START) {
            return false;
        }
        self.device_state = GIP_STATE_START;

        if !self.send_guide_button_led(io.port, GIP_LED_GUIDE_ON, 20) {
            return false;
        }

        if self.supports_system_message(GIP_CMD_SECURITY, false)
            && self.features & GIP_FEATURE_SECURITY_OPT_OUT == 0
        {
            /* TODO: Implement Security command property */
            let buffer = [0x1, 0x0];
            self.send_system_message(io.port, GIP_CMD_SECURITY, 0, &buffer);
        }

        if self.supports_vendor_message(GIP_CMD_INITIAL_REPORTS_REQUEST, false) {
            // (a zeroed GIP_InitialReportsRequest: its type and two data bytes)
            let request = [0u8; 3];
            self.send_vendor_message(io.port, GIP_CMD_INITIAL_REPORTS_REQUEST, 0, &request);
        }

        if self.supports_vendor_message(GIP_CMD_DEVICE_CAPABILITIES, false) {
            self.send_vendor_message(io.port, GIP_CMD_DEVICE_CAPABILITIES, 0, &[]);
        }

        if (self.attachment_index == 0 || self.is_controller()) && self.joystick == 0 {
            self.joystick = io.device.joystick_connected();
            return true;
        }
        if self.attachment_type == AttachmentType::Chatpad && self.keyboard == 0 {
            self.add_keyboard(io);
        }
        true
    }

    /// The chatpad's keyboard (the `SDL_AddKeyboard()` of upstream).
    fn add_keyboard(&mut self, io: &mut Io<'_, '_>) {
        self.keyboard = new_keyboard_id();
        io.keyboard.push(KeyboardAction::Add(self.keyboard));
    }

    /// Translation of `GIP_EnsureMetadata()`.
    fn ensure_metadata(&mut self, gip: &mut GipDevice, io: &mut Io<'_, '_>) -> bool {
        match self.got_metadata {
            MetadataStatus::Got | MetadataStatus::Faked => true,
            MetadataStatus::None => {
                if gip.got_hello {
                    gip.timeout = GIP_ACME_TIMEOUT;
                    self.got_metadata = MetadataStatus::Pending;
                    self.metadata_next = io.port.ticks_ms() + 500;
                    self.metadata_retries = 0;
                    self.send_system_message(io.port, GIP_CMD_METADATA, 0, &[])
                } else {
                    if gip.reset_for_metadata {
                        return true;
                    }
                    self.set_metadata_defaults(gip, io)
                }
            }
            MetadataStatus::Pending => true,
        }
    }

    /// Translation of `GIP_SetMetadataDefaults()`.
    fn set_metadata_defaults(&mut self, gip: &mut GipDevice, io: &mut Io<'_, '_>) -> bool {
        let vendor_id = io.device.vendor_id();
        let product_id = io.device.product_id();
        if self.attachment_index == 0 {
            /* Some decent default settings */
            self.features |= GIP_FEATURE_MOTOR_CONTROL;
            self.attachment_type = AttachmentType::Gamepad;
            self.metadata.device.in_system_messages[0] |= 1 << GIP_CMD_GUIDE_BUTTON;

            if is_joystick_xbox_series_x(vendor_id, product_id) {
                self.features |= GIP_FEATURE_CONSOLE_FUNCTION_MAP;
            }
        }

        self.handle_quirks(vendor_id, product_id);

        if self.supports_system_message(GIP_CMD_FIRMWARE, false) {
            self.send_query_firmware(io.port, 2);
        }

        self.got_metadata = MetadataStatus::Faked;
        gip.hello_deadline = 0;
        if self.joystick == 0 {
            self.joystick = io.device.joystick_connected();
        }
        true
    }

    /// Translation of `GIP_HandleCommandProtocolControl()`.
    fn handle_command_protocol_control(&mut self) -> bool {
        // TODO
        crate::log::debug!(
            Category::Input,
            "GIP: Unimplemented Protocol Control message"
        );
        false
    }

    /// Translation of `GIP_HandleCommandHelloDevice()`.
    fn handle_command_hello_device(
        &mut self,
        gip: &mut GipDevice,
        io: &mut Io<'_, '_>,
        header: &Header,
        bytes: &[u8],
        num_bytes: usize,
    ) -> bool {
        if num_bytes != 28 {
            return false;
        }

        let device_id = u64::from_le_bytes(bytes[0..8].try_into().unwrap_or_default());
        let vendor_id = le16(bytes, 8);
        let product_id = le16(bytes, 10);
        let firmware_major_version = le16(bytes, 12);
        let firmware_minor_version = le16(bytes, 14);
        let firmware_build_version = le16(bytes, 16);
        let firmware_revision = le16(bytes, 18);
        // (the hardware version, bytes 20 and 21, isn't used)
        let rf_proto_major_version = bytes[22];
        let rf_proto_minor_version = bytes[23];
        let security_major_version = bytes[24];
        let security_minor_version = bytes[25];
        let gip_major_version = bytes[26];
        let gip_minor_version = bytes[27];

        crate::log::info!(
            Category::Input,
            "GIP: Device hello from {:x} ({:04x}:{:04x})",
            device_id,
            vendor_id,
            product_id
        );
        crate::log::info!(
            Category::Input,
            "GIP: Firmware version {}.{}.{} rev {}",
            firmware_major_version,
            firmware_minor_version,
            firmware_build_version,
            firmware_revision
        );

        /*
         * The GIP spec specifies that the host should reject the device if any of these are wrong.
         * I don't know if Windows or an Xbox do, however, so let's just log warnings instead.
         */
        // FIXME (upstream): the version checks only warn when both numbers
        // are wrong (`&&` where `||` is meant).
        if rf_proto_major_version != 1 && rf_proto_minor_version != 0 {
            crate::log::warn!(
                Category::Input,
                "GIP: Invalid RF protocol version {}.{}, expected 1.0",
                rf_proto_major_version,
                rf_proto_minor_version
            );
        }

        if security_major_version != 1 && security_minor_version != 0 {
            crate::log::warn!(
                Category::Input,
                "GIP: Invalid security protocol version {}.{}, expected 1.0",
                security_major_version,
                security_minor_version
            );
        }

        if gip_major_version != 1 && gip_minor_version != 0 {
            crate::log::warn!(
                Category::Input,
                "GIP: Invalid GIP version {}.{}, expected 1.0",
                gip_major_version,
                gip_minor_version
            );
        }

        if header.flags & GIP_FLAG_ATTACHMENT_MASK != 0 {
            self.send_system_message(io.port, GIP_CMD_METADATA, 0, &[])
        } else {
            self.firmware_major_version = firmware_major_version;
            self.firmware_minor_version = firmware_minor_version;

            if self.attachment_index == 0 {
                gip.hello_deadline = 0;
                gip.got_hello = true;
            }
            if self.got_metadata == MetadataStatus::Faked {
                self.got_metadata = MetadataStatus::None;
            }
            self.ensure_metadata(gip, io);
            true
        }
    }

    /// Translation of `GIP_HandleCommandStatusDevice()`.
    fn handle_command_status_device(
        &mut self,
        gip: &mut GipDevice,
        io: &mut Io<'_, '_>,
        bytes: &[u8],
        num_bytes: usize,
    ) -> bool {
        if num_bytes < 1 {
            return false;
        }
        let battery_level = bytes[0] & 3;
        let battery_type = (bytes[0] >> 2) & 3;
        let charge = (bytes[0] >> 4) & 3;
        // (the power level, the top two bits, isn't used)

        if self.joystick != 0 && io.port.joystick_open(self.joystick) {
            let power_percent = match battery_level {
                GIP_BATTERY_CRITICAL => 1,
                GIP_BATTERY_LOW => 25,
                GIP_BATTERY_MEDIUM => 50,
                GIP_BATTERY_FULL => 100,
                _ => 0,
            };
            let mut power_state = match charge {
                GIP_CHARGING => {
                    if battery_level == GIP_BATTERY_FULL {
                        PowerState::Charged
                    } else {
                        PowerState::Charging
                    }
                }
                GIP_NOT_CHARGING => PowerState::OnBattery,
                _ => PowerState::Unknown,
            };

            match battery_type {
                GIP_BATTERY_ABSENT => power_state = PowerState::NoBattery,
                GIP_BATTERY_STANDARD | GIP_BATTERY_RECHARGEABLE => {}
                _ => power_state = PowerState::Unknown,
            }

            io.device
                .send_power_info(self.joystick, power_state, power_percent);
        }

        if num_bytes >= 4 {
            // (whether the device is active, bit 0 of byte 1, isn't used)
            if bytes[1] & 2 != 0 {
                /* Events present */
                if num_bytes < 5 {
                    return false;
                }
                let num_events = usize::from(bytes[4]);
                if num_events > 5 {
                    crate::log::warn!(
                        Category::Input,
                        "GIP: Device reported too many events, {} > 5",
                        num_events
                    );
                    return false;
                }
                if 5 + num_events * 10 > num_bytes {
                    return false;
                }
                // (the events, their type, fault tag and fault address,
                // aren't used)
                // FIXME (upstream): the fault address is read into the
                // fault tag.
            }
        }

        self.ensure_metadata(gip, io);
        true
    }

    /// Translation of `GIP_HandleCommandMetadataRespose()`.
    fn handle_command_metadata_response(&mut self, io: &mut Io<'_, '_>, bytes: &[u8]) -> bool {
        let Some(metadata) = parse_metadata(bytes) else {
            return false;
        };

        self.metadata = metadata;
        self.got_metadata = MetadataStatus::Got;
        self.features = 0;

        /// The preferred types known, their attachment types and the
        /// interfaces they should have.
        const PREFERRED_TYPES: [(&[u8], AttachmentType, Option<InterfaceGuid>); 15] = [
            (
                b"Windows.Xbox.Input.Gamepad",
                AttachmentType::Gamepad,
                Some(GUID_IGAMEPAD),
            ),
            (
                b"Microsoft.Xbox.Input.ArcadeStick",
                AttachmentType::ArcadeStick,
                Some(GUID_ARCADE_STICK),
            ),
            (
                b"Windows.Xbox.Input.ArcadeStick",
                AttachmentType::ArcadeStick,
                Some(GUID_ARCADE_STICK),
            ),
            (
                b"Microsoft.Xbox.Input.FlightStick",
                AttachmentType::FlightStick,
                Some(GUID_FLIGHT_STICK),
            ),
            (
                b"Windows.Xbox.Input.FlightStick",
                AttachmentType::FlightStick,
                Some(GUID_FLIGHT_STICK),
            ),
            (
                b"Microsoft.Xbox.Input.Wheel",
                AttachmentType::Wheel,
                Some(GUID_WHEEL),
            ),
            (
                b"Windows.Xbox.Input.Wheel",
                AttachmentType::Wheel,
                Some(GUID_WHEEL),
            ),
            (
                b"Windows.Xbox.Input.NavigationController",
                AttachmentType::NavigationController,
                Some(GUID_NAVIGATION_CONTROLLER),
            ),
            (b"Windows.Xbox.Input.Chatpad", AttachmentType::Chatpad, None),
            (
                b"Windows.Xbox.Input.Headset",
                AttachmentType::Headset,
                Some(GUID_IHEADSET),
            ),
            (
                b"Activision.Xbox.Input.GH7",
                AttachmentType::LiveGuitar,
                Some(GUID_GUITAR_HERO_LIVE_GUITAR),
            ),
            (
                b"MadCatz.Xbox.Guitar.Stratocaster",
                AttachmentType::Guitar,
                Some(GUID_MADCATZ_GUITAR),
            ),
            (
                b"PDP.Xbox.Guitar.Jaguar",
                AttachmentType::Guitar,
                Some(GUID_PDP_GUITAR),
            ),
            (
                b"MadCatz.Xbox.Drums.Glam",
                AttachmentType::DrumKit,
                Some(GUID_MADCATZ_DRUM_KIT),
            ),
            (
                b"PDP.Xbox.Drums.Tablah",
                AttachmentType::DrumKit,
                Some(GUID_PDP_DRUM_KIT),
            ),
        ];

        self.attachment_type = AttachmentType::Unknown;
        let mut expected_guid = None;
        for preferred_type in &self.metadata.device.preferred_types {
            if let Some(&(_, attachment_type, guid)) = PREFERRED_TYPES
                .iter()
                .find(|(name, _, _)| name == preferred_type)
            {
                self.attachment_type = attachment_type;
                expected_guid = guid;
                break;
            }
        }

        let mut found_expected_guid = expected_guid.is_none();
        let mut found_controller_guid = false;
        for guid in &self.metadata.device.supported_interfaces {
            if expected_guid.as_ref() == Some(guid) {
                found_expected_guid = true;
            }
            match *guid {
                GUID_ICONTROLLER => found_controller_guid = true,
                GUID_IDEV_AUTH_PC_OPT_OUT => self.features |= GIP_FEATURE_SECURITY_OPT_OUT,
                GUID_ICONSOLE_FUNCTION_MAP_INPUT_REPORT => {
                    self.features |= GIP_FEATURE_CONSOLE_FUNCTION_MAP
                }
                GUID_ICONSOLE_FUNCTION_MAP_OVERFLOW_INPUT_REPORT => {
                    self.features |= GIP_FEATURE_CONSOLE_FUNCTION_MAP_OVERFLOW
                }
                GUID_IELITE_BUTTONS => self.features |= GIP_FEATURE_ELITE_BUTTONS,
                GUID_DYNAMIC_LATENCY_INPUT => self.features |= GIP_FEATURE_DYNAMIC_LATENCY_INPUT,
                _ => {}
            }
        }

        for message in &self.metadata.message_metadata {
            if message.message_type == GIP_CMD_DIRECT_MOTOR
                && message.length >= 9
                && message.flags & GIP_MESSAGE_FLAG_DOWNSTREAM != 0
            {
                self.features |= GIP_FEATURE_MOTOR_CONTROL;
            }
        }

        if !found_expected_guid || (self.is_controller() && !found_controller_guid) {
            crate::log::debug!(
                Category::Input,
                "GIP: Controller was missing expected GUID. This controller probably won't work on an actual Xbox."
            );
        }

        if self.features & GIP_FEATURE_GUIDE_COLOR != 0
            && !self.supports_vendor_message(GIP_CMD_GUIDE_COLOR, false)
        {
            self.features &= !GIP_FEATURE_GUIDE_COLOR;
        }

        self.handle_quirks(io.device.vendor_id(), io.device.product_id());

        self.send_init_sequence(io)
    }

    /// Translation of `GIP_HandleCommandSecurity()`.
    fn handle_command_security(&mut self) -> bool {
        // TODO
        crate::log::debug!(Category::Input, "GIP: Unimplemented Security message");
        false
    }

    /// Translation of `GIP_HandleCommandGuideButtonStatus()`.
    fn handle_command_guide_button_status(&mut self, io: &mut Io<'_, '_>, bytes: &[u8]) -> bool {
        let timestamp = crate::timer::ticks_ns();

        if io.device.num_joysticks() < 1 {
            return true;
        }

        if !io.port.joystick_open(self.joystick) {
            return false;
        }
        if bytes[1] == VK_LWIN {
            io.device.send_button(
                timestamp,
                self.joystick,
                GamepadButton::Guide as u8,
                bytes[0] & 0x03 != 0,
            );
        }

        true
    }

    /// Translation of `GIP_HandleCommandAudioControl()`.
    fn handle_command_audio_control(&mut self) -> bool {
        // TODO
        crate::log::debug!(Category::Input, "GIP: Unimplemented Audio Control message");
        false
    }

    /// Translation of `GIP_HandleCommandFirmware()`.
    fn handle_command_firmware(
        &mut self,
        io: &mut Io<'_, '_>,
        bytes: &[u8],
        num_bytes: usize,
    ) -> bool {
        if num_bytes < 1 {
            return false;
        }
        if bytes[0] == 1 {
            if num_bytes < 14 {
                crate::log::debug!(
                    Category::Input,
                    "GIP: Discarding too-short firmware message"
                );

                return false;
            }
            let major = le16(bytes, 6);
            let minor = le16(bytes, 8);
            let build = le16(bytes, 10);
            let rev = le16(bytes, 12);

            crate::log::debug!(
                Category::Input,
                "GIP: Firmware version: {}.{}.{} rev {}",
                major,
                minor,
                build,
                rev
            );

            self.firmware_major_version = major;
            self.firmware_minor_version = minor;

            if io.device.vendor_id() == USB_VENDOR_MICROSOFT
                && io.device.product_id() == USB_PRODUCT_XBOX_ONE_ELITE_SERIES_2
            {
                return self.enable_elite_buttons(io);
            }
            true
        } else {
            crate::log::debug!(Category::Input, "GIP: Unimplemented Firmware message");

            false
        }
    }

    /// The four paddle buttons.
    fn send_paddles(&self, io: &mut Io<'_, '_>, timestamp: u64, paddles: [bool; 4]) {
        for (i, down) in (0u8..).zip(paddles) {
            let button = self.paddle_idx.wrapping_add(i);
            io.device
                .send_button(timestamp, self.joystick, button, down);
        }
    }

    /// Translation of `GIP_HandleCommandRawReport()`.
    fn handle_command_raw_report(
        &mut self,
        io: &mut Io<'_, '_>,
        bytes: &[u8],
        num_bytes: usize,
    ) -> bool {
        let timestamp = crate::timer::ticks_ns();

        if io.device.num_joysticks() < 1 {
            return true;
        }

        if !io.port.joystick_open(self.joystick) {
            return true;
        }

        if num_bytes < 17 {
            crate::log::debug!(Category::Input, "GIP: Discarding too-short raw report");
            return false;
        }

        if self.features & GIP_FEATURE_ELITE_BUTTONS != 0
            && self.xbe_format == EliteButtonFormat::Xbe2Raw
        {
            if bytes[15] & 3 != 0 {
                self.send_paddles(io, timestamp, [false; 4]);
            } else {
                let paddles = bytes[GIP_BTN_OFFSET_XBE2];
                self.send_paddles(
                    io,
                    timestamp,
                    [
                        paddles & 0x01 != 0,
                        paddles & 0x02 != 0,
                        paddles & 0x04 != 0,
                        paddles & 0x08 != 0,
                    ],
                );
            }
        }
        true
    }

    /// A key of the chatpad.
    fn send_key(&self, io: &mut Io<'_, '_>, timestamp: u64, scancode: Scancode, down: bool) {
        io.keyboard.push(KeyboardAction::Key(
            timestamp,
            self.keyboard,
            scancode,
            down,
        ));
    }

    /// Translation of `GIP_HandleCommandHidReport()`.
    fn handle_command_hid_report(
        &mut self,
        io: &mut Io<'_, '_>,
        bytes: &[u8],
        num_bytes: usize,
    ) -> bool {
        let timestamp = crate::timer::ticks_ns();
        // SDL doesn't have HID descriptor parsing, so we have to hardcode for the Chatpad descriptor instead.
        // I don't know of any other devices that emit HID reports, so this should be safe.
        if self.attachment_type != AttachmentType::Chatpad || self.keyboard == 0 || num_bytes != 8 {
            crate::log::debug!(Category::Input, "GIP: Unimplemented HID Report message");
            return false;
        }

        let modifiers = bytes[0];
        let changed_modifiers = modifiers ^ self.last_modifiers;
        if changed_modifiers & 0x02 != 0 {
            self.send_key(io, timestamp, Scancode::LSHIFT, modifiers & 0x02 != 0);
        }
        // The chatpad has several non-ASCII characters that it sends as Alt codes
        if changed_modifiers & 0x04 != 0 {
            if modifiers & 0x04 != 0 {
                self.altcode_digit = 0;
                self.altcode = 0;
            } else {
                if self.altcode_digit == 4 {
                    // Some Alt codes don't match their Unicode codepoint for some reason
                    let codepoint = match self.altcode {
                        128 => 0x20AC,
                        138 => 0x0160,
                        140 => 0x0152,
                        154 => 0x0161,
                        156 => 0x0153,
                        altcode => altcode,
                    };
                    let mut utf8 = [0u8; 4];
                    let text = crate::stdlib::string::ucs4_to_utf8(codepoint, &mut utf8);
                    // (the text ends at a NUL, as the C string)
                    let text = text.split('\0').next().unwrap_or_default();
                    io.keyboard.push(KeyboardAction::Text(text.to_owned()));
                }
                self.altcode_digit = -1;
                self.send_key(io, timestamp, Scancode::NUMLOCKCLEAR, true);
                self.send_key(io, timestamp, Scancode::NUMLOCKCLEAR, false);
            }
        }

        if bytes[2] == 0 && self.last_key != 0 {
            let last_key = Scancode(u16::from(self.last_key));
            if last_key == Scancode::CAPSLOCK {
                self.capslock = !self.capslock;
            }
            self.send_key(io, timestamp, last_key, false);
            if self.last_modifiers & 0xfd == 0 {
                let modstate = if self.last_modifiers & 0x02 != 0 || self.capslock {
                    Keymod::SHIFT
                } else {
                    Keymod::NONE
                };
                let keycode = keymap::default_key_from_scancode(last_key, modstate);
                if keycode.0 != 0 && keycode.0 < 0x80 {
                    let text = char::from(keycode.0 as u8).to_string();
                    io.keyboard.push(KeyboardAction::Text(text));
                }
            }
            self.last_key = 0;
        } else {
            // FIXME (upstream): a report without a key, when none was down,
            // lands here: it presses scancode 0, and ends an Alt code (so
            // pressing Alt before the first digit loses the code).
            self.send_key(io, timestamp, Scancode(u16::from(bytes[2])), true);
            self.last_key = bytes[2];

            if modifiers & 0x04 != 0 && self.altcode_digit >= 0 {
                let digit = i32::from(bytes[2]) - i32::from(Scancode::KP_1.0) + 1;
                if !(1..=10).contains(&digit) {
                    self.altcode_digit = -1;
                } else {
                    self.altcode_digit += 1;
                    self.altcode = self.altcode.wrapping_mul(10);
                    if digit < 10 {
                        self.altcode = self.altcode.wrapping_add(digit as u32);
                    }
                }
            }
        }

        self.last_modifiers = modifiers;
        true
    }

    /// Translation of `GIP_HandleCommandExtended()`.
    fn handle_command_extended(
        &mut self,
        io: &mut Io<'_, '_>,
        header: &Header,
        bytes: &[u8],
        num_bytes: usize,
    ) -> bool {
        if num_bytes < 2 {
            return false;
        }

        match bytes[0] {
            GIP_EXTCMD_GET_SERIAL_NUMBER => {
                if bytes[1] != GIP_EXTENDED_STATUS_OK {
                    return true;
                }
                if header.flags & GIP_FLAG_ATTACHMENT_MASK != 0 {
                    return true;
                }
                // (at most 32 bytes, up to a NUL, as the C string)
                let serial = &bytes[2..num_bytes.min(2 + 32)];
                let end = serial.iter().position(|&b| b == 0).unwrap_or(serial.len());
                io.device
                    .set_device_serial(&String::from_utf8_lossy(&serial[..end]));
            }
            _ => {
                // TODO
                crate::log::debug!(
                    Category::Input,
                    "GIP: Extended message type {:02x}",
                    bytes[0]
                );
                return false;
            }
        }

        true
    }

    /// Translation of `GIP_HandleNavigationReport()`.
    fn handle_navigation_report(&self, io: &mut Io<'_, '_>, timestamp: u64, bytes: &[u8]) {
        let joystick = self.joystick;
        if self.last_input[0] != bytes[0] {
            use GamepadButton as B;
            for (button, mask) in [
                (B::Start, 0x04),
                (B::Back, 0x08),
                (B::South, 0x10),
                (B::East, 0x20),
                (B::West, 0x40),
                (B::North, 0x80),
            ] {
                io.device
                    .send_button(timestamp, joystick, button as u8, bytes[0] & mask != 0);
            }
        }

        if self.last_input[1] != bytes[1] {
            let mut hat = 0;

            if bytes[1] & 0x01 != 0 {
                hat |= HAT_UP;
            }
            if bytes[1] & 0x02 != 0 {
                hat |= HAT_DOWN;
            }
            if bytes[1] & 0x04 != 0 {
                hat |= HAT_LEFT;
            }
            if bytes[1] & 0x08 != 0 {
                hat |= HAT_RIGHT;
            }
            io.device.send_hat(timestamp, joystick, 0, hat);

            let (first, second) = if self.attachment_type == AttachmentType::ArcadeStick {
                /* Previous, then next */
                (GamepadButton::RightShoulder, GamepadButton::LeftShoulder)
            } else {
                (GamepadButton::LeftShoulder, GamepadButton::RightShoulder)
            };
            io.device
                .send_button(timestamp, joystick, first as u8, bytes[1] & 0x10 != 0);
            io.device
                .send_button(timestamp, joystick, second as u8, bytes[1] & 0x20 != 0);
        }
    }

    /// Translation of `GIP_HandleGamepadReport()`.
    fn handle_gamepad_report(&self, io: &mut Io<'_, '_>, timestamp: u64, bytes: &[u8]) {
        let joystick = self.joystick;

        io.device.send_button(
            timestamp,
            joystick,
            GamepadButton::LeftStick as u8,
            bytes[1] & 0x40 != 0,
        );
        io.device.send_button(
            timestamp,
            joystick,
            GamepadButton::RightStick as u8,
            bytes[1] & 0x80 != 0,
        );

        io.device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftTrigger as u8,
            trigger_axis(bytes[2], bytes[3]),
        );
        io.device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightTrigger as u8,
            trigger_axis(bytes[4], bytes[5]),
        );

        let axis = |offset: usize| le16(bytes, offset) as i16;
        io.device
            .send_axis(timestamp, joystick, GamepadAxis::LeftX as u8, axis(6));
        io.device
            .send_axis(timestamp, joystick, GamepadAxis::LeftY as u8, !axis(8));
        io.device
            .send_axis(timestamp, joystick, GamepadAxis::RightX as u8, axis(10));
        io.device
            .send_axis(timestamp, joystick, GamepadAxis::RightY as u8, !axis(12));
    }

    /// Translation of `GIP_HandleGuitarReport()`.
    fn handle_guitar_report(
        &self,
        io: &mut Io<'_, '_>,
        timestamp: u64,
        bytes: &[u8],
        num_bytes: usize,
    ) {
        const GUITAR_BUTTONS: [u16; 11] = [
            0x0010, // SDL_GAMEPAD_BUTTON_SOUTH
            0x0020, // SDL_GAMEPAD_BUTTON_EAST
            0x0040, // SDL_GAMEPAD_BUTTON_WEST
            0x0080, // SDL_GAMEPAD_BUTTON_NORTH
            0x0008, // SDL_GAMEPAD_BUTTON_BACK
            0,      // The guide button is not available
            0x0004, // SDL_GAMEPAD_BUTTON_START
            0,      // right joystick click unavailable
            0x4000, // SDL_GAMEPAD_BUTTON_RIGHT_STICK
            0x1000, // SDL_GAMEPAD_BUTTON_LEFT_SHOULDER
            0,      // right shoulder unavailable
        ];
        let joystick = self.joystick;
        let mut hat = 0;
        let buttons = le16(bytes, 0);
        if num_bytes >= 10 {
            for (btnidx, button_mask) in (0u8..).zip(GUITAR_BUTTONS) {
                if button_mask == 0 {
                    continue;
                }
                let down = buttons & button_mask != 0;
                io.device.send_button(timestamp, joystick, btnidx, down);
            }
            if buttons & 0x0100 != 0 {
                hat |= HAT_UP;
            }
            if buttons & 0x0200 != 0 {
                hat |= HAT_DOWN;
            }
            if buttons & 0x0400 != 0 {
                hat |= HAT_LEFT;
            }
            if buttons & 0x0800 != 0 {
                hat |= HAT_RIGHT;
            }
            io.device.send_hat(timestamp, joystick, 0, hat);
            io.device.send_axis(
                timestamp,
                joystick,
                GamepadAxis::RightX as u8,
                (i32::from(bytes[3]) * 257 - 32768) as i16,
            );
            // PS3 RB guitars had tilt on right shoulder
            io.device.send_button(
                timestamp,
                joystick,
                GamepadButton::RightShoulder as u8,
                bytes[2] >= 0xD0,
            );
            // PS3 RB guitars send L2 when using solo buttons
            io.device.send_axis(
                timestamp,
                joystick,
                GamepadAxis::LeftTrigger as u8,
                if bytes[6] != 0 { 32767 } else { -32768 },
            );
            // Align pickup selector mappings with PS3 instruments
            const EFFECTS_MAPPINGS: [i16; 5] = [-26880, -13568, -1792, 11008, 24576];
            // FIXME (upstream): a selector past the table (above 4) reads
            // past it; here the axis isn't sent.
            if let Some(&value) = EFFECTS_MAPPINGS.get(usize::from(bytes[4] >> 4)) {
                io.device
                    .send_axis(timestamp, joystick, GamepadAxis::RightY as u8, value);
            }
        }
    }

    /// Translation of `GIP_HandleDrumKitReport()`.
    fn handle_drum_kit_report(
        &self,
        io: &mut Io<'_, '_>,
        timestamp: u64,
        bytes: &[u8],
        num_bytes: usize,
    ) {
        let joystick = self.joystick;
        let mut hat = 0;
        let buttons = le16(bytes, 0);
        if num_bytes >= 6 {
            if buttons & 0x0100 != 0 || bytes[4] & 0xf0 != 0 {
                hat |= HAT_UP;
            }
            if buttons & 0x0200 != 0 || bytes[4] & 0x0f != 0 {
                hat |= HAT_DOWN;
            }
            if buttons & 0x0400 != 0 {
                hat |= HAT_LEFT;
            }
            if buttons & 0x0800 != 0 {
                hat |= HAT_RIGHT;
            }
            io.device.send_hat(timestamp, joystick, 0, hat);
            // The rest of the instruments are mapped to PS3 style inputs
            // The PS3 used flags for dictating if a pad or a cymbal was hit, so we emulate that here
            use GamepadButton as B;
            for (button, down) in [
                (B::Back, buttons & 0x0008 != 0),
                (B::Start, buttons & 0x0004 != 0),
                (B::LeftShoulder, buttons & 0x1000 != 0),
                (B::RightShoulder, buttons & 0x2000 != 0),
                (
                    B::South,
                    bytes[3] & 0x0f != 0 || bytes[5] & 0xf0 != 0 || buttons & 0x0010 != 0,
                ),
                (B::East, bytes[2] & 0xf0 != 0 || buttons & 0x0020 != 0),
                (
                    B::West,
                    bytes[3] & 0xf0 != 0 || bytes[4] & 0x0f != 0 || buttons & 0x0040 != 0,
                ),
                (
                    B::North,
                    bytes[2] & 0x0f != 0 || bytes[4] & 0xf0 != 0 || buttons & 0x0080 != 0,
                ),
                (B::LeftStick, bytes[2] != 0 || bytes[3] != 0),
                (B::RightStick, bytes[4] != 0 || bytes[5] != 0),
            ] {
                io.device
                    .send_button(timestamp, joystick, button as u8, down);
            }
        }
    }

    /// Translation of `GIP_HandleArcadeStickReport()`.
    fn handle_arcade_stick_report(
        &self,
        io: &mut Io<'_, '_>,
        timestamp: u64,
        bytes: &[u8],
        num_bytes: usize,
    ) {
        let joystick = self.joystick;
        io.device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftTrigger as u8,
            trigger_axis(bytes[2], bytes[3]),
        );
        io.device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightTrigger as u8,
            trigger_axis(bytes[4], bytes[5]),
        );

        if num_bytes >= 19 {
            /* Extra button 6 */
            io.device.send_axis(
                timestamp,
                joystick,
                GamepadAxis::RightTrigger as u8,
                if bytes[18] & 0x40 != 0 { 32767 } else { -32768 },
            );
            /* Extra button 7 */
            io.device.send_axis(
                timestamp,
                joystick,
                GamepadAxis::LeftTrigger as u8,
                if bytes[18] & 0x80 != 0 { 32767 } else { -32768 },
            );
        }
    }

    /// Translation of `GIP_HandleFlightStickReport()`.
    fn handle_flight_stick_report(
        &self,
        io: &mut Io<'_, '_>,
        timestamp: u64,
        bytes: &[u8],
        num_bytes: usize,
    ) {
        let joystick = self.joystick;

        if num_bytes < 19 {
            return;
        }

        if self.last_input[2] != bytes[2] {
            /* Fire 1 and 2 */
            io.device.send_button(
                timestamp,
                joystick,
                GamepadButton::LeftStick as u8,
                bytes[2] & 0x01 != 0,
            );
            io.device.send_button(
                timestamp,
                joystick,
                GamepadButton::RightStick as u8,
                bytes[2] & 0x02 != 0,
            );
        }
        let mut i = 0;
        while i < self.extra_buttons {
            let byte = (i / 8 + 3) as usize;
            if self.last_input[byte] != bytes[byte] {
                while i < self.extra_buttons {
                    let byte = (i / 8 + 3) as usize;
                    // FIXME (upstream): the bit isn't taken modulo 8, so the
                    // buttons past the first eight are never down.
                    let mask = 1u32.wrapping_shl(i as u32);
                    io.device.send_button(
                        timestamp,
                        joystick,
                        self.extra_button_idx.wrapping_add(i as u8),
                        u32::from(bytes[byte]) & mask != 0,
                    );
                    i += 1;
                }
            } else {
                i += 8;
            }
        }

        /* Roll, pitch and yaw are signed. Throttle and any extra axes are unsigned. All values are full-range. */
        io.device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftX as u8,
            le16(bytes, 11) as i16,
        );
        io.device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftY as u8,
            le16(bytes, 13) as i16,
        );
        io.device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightX as u8,
            le16(bytes, 15) as i16,
        );

        /* There are no more signed values, so skip RIGHTY */

        io.device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftTrigger as u8,
            unsigned_axis(bytes[17], bytes[18]),
        );

        for i in 0..self.extra_axes.max(0) as usize {
            if 20 + i * 2 >= num_bytes {
                return;
            }
            io.device.send_axis(
                timestamp,
                joystick,
                (GamepadAxis::RightTrigger as u8).wrapping_add(i as u8),
                unsigned_axis(bytes[19 + i * 2], bytes[20 + i * 2]),
            );
        }
    }

    /// Translation of `GIP_HandleLLInputReport()`.
    fn handle_ll_input_report(
        &mut self,
        gip: &mut GipDevice,
        io: &mut Io<'_, '_>,
        bytes: &[u8],
        num_bytes: usize,
    ) -> bool {
        let timestamp = crate::timer::ticks_ns();

        if io.device.num_joysticks() < 1 {
            self.ensure_metadata(gip, io);
            if self.got_metadata != MetadataStatus::Got
                && self.got_metadata != MetadataStatus::Faked
            {
                return true;
            }
        }

        if !io.port.joystick_open(self.joystick) {
            return false;
        }

        if self.device_state != GIP_STATE_START {
            crate::log::debug!(Category::Input, "GIP: Discarding early input report");
            self.device_state = GIP_STATE_START;
            return true;
        }

        if num_bytes < 6 {
            crate::log::debug!(Category::Input, "GIP: Discarding too-short input report");
            return false;
        }

        self.handle_navigation_report(io, timestamp, bytes);

        match self.attachment_type {
            AttachmentType::ArcadeStick => {
                self.handle_arcade_stick_report(io, timestamp, bytes, num_bytes)
            }
            AttachmentType::FlightStick => {
                self.handle_flight_stick_report(io, timestamp, bytes, num_bytes)
            }
            AttachmentType::Guitar => self.handle_guitar_report(io, timestamp, bytes, num_bytes),
            AttachmentType::DrumKit => self.handle_drum_kit_report(io, timestamp, bytes, num_bytes),
            _ => self.handle_gamepad_report(io, timestamp, bytes),
        }

        if self.features & GIP_FEATURE_ELITE_BUTTONS != 0 {
            let mut clear = false;
            if self.xbe_format == EliteButtonFormat::Xbe1
                && num_bytes > GIP_BTN_OFFSET_XBE1
                && self.last_input[GIP_BTN_OFFSET_XBE1] != bytes[GIP_BTN_OFFSET_XBE1]
                && bytes[GIP_BTN_OFFSET_XBE1] & 0x10 != 0
            {
                let paddles = bytes[GIP_BTN_OFFSET_XBE1];
                self.send_paddles(
                    io,
                    timestamp,
                    [
                        paddles & 0x02 != 0,
                        paddles & 0x08 != 0,
                        paddles & 0x01 != 0,
                        paddles & 0x04 != 0,
                    ],
                );
            } else if (self.xbe_format == EliteButtonFormat::Xbe24
                || self.xbe_format == EliteButtonFormat::Xbe25)
                && num_bytes > GIP_BTN_OFFSET_XBE2
            {
                let profile_offset = if self.xbe_format == EliteButtonFormat::Xbe24 {
                    15
                } else {
                    20
                };
                if self.last_input[GIP_BTN_OFFSET_XBE2] != bytes[GIP_BTN_OFFSET_XBE2]
                    || self.last_input[profile_offset] != bytes[profile_offset]
                {
                    if bytes[profile_offset] & 3 != 0 {
                        clear = true;
                    } else {
                        let paddles = bytes[GIP_BTN_OFFSET_XBE2];
                        self.send_paddles(
                            io,
                            timestamp,
                            [
                                paddles & 0x01 != 0,
                                paddles & 0x02 != 0,
                                paddles & 0x04 != 0,
                                paddles & 0x08 != 0,
                            ],
                        );
                    }
                }
            } else {
                clear = true;
            }
            if clear {
                self.send_paddles(io, timestamp, [false; 4]);
            }
        }
        // Input reports are at a minimum 5 bytes, and the console function map is always 18 bytes
        if self.features & GIP_FEATURE_CONSOLE_FUNCTION_MAP != 0 && num_bytes >= 23 {
            let mut function_map_offset = None;
            if self.features & GIP_FEATURE_DYNAMIC_LATENCY_INPUT != 0 {
                /* The dynamic latency input bytes are after the console function map */
                if num_bytes >= 40 {
                    function_map_offset = Some(num_bytes - 26);
                }
            } else {
                function_map_offset = Some(num_bytes - 18);
            }
            if let Some(offset) = function_map_offset.filter(|&offset| offset >= 5) {
                // FIXME (upstream): past the saved input (a report of more
                // than 82 bytes, reassembled from fragments), the last
                // input is read past its buffer; here it reads as changed.
                if self.last_input.get(offset) != Some(&bytes[offset]) {
                    io.device.send_button(
                        timestamp,
                        self.joystick,
                        self.share_button_idx,
                        bytes[offset] & 0x01 != 0,
                    );
                }
            }
        }

        let saved = num_bytes.min(self.last_input.len());
        self.last_input[..saved].copy_from_slice(&bytes[..saved]);

        true
    }

    /// Translation of `GIP_HandleLLStaticConfiguration()`.
    fn handle_ll_static_configuration(&mut self) -> bool {
        // TODO
        crate::log::debug!(
            Category::Input,
            "GIP: Unimplemented Static Configuration message"
        );
        false
    }

    /// Translation of `GIP_HandleLLButtonInfoReport()`.
    fn handle_ll_button_info_report(&mut self) -> bool {
        // TODO
        crate::log::debug!(
            Category::Input,
            "GIP: Unimplemented Button Info Report message"
        );
        false
    }

    /// Translation of `GIP_HandleLLOverflowInputReport()`.
    fn handle_ll_overflow_input_report(&mut self) -> bool {
        // TODO
        crate::log::debug!(
            Category::Input,
            "GIP: Unimplemented Overflow Input Report message"
        );
        false
    }

    /// Translation of `GIP_HandleAudioData()`.
    fn handle_audio_data(&mut self) -> bool {
        // TODO
        crate::log::debug!(Category::Input, "GIP: Unimplemented Audio Data message");
        false
    }

    /// Translation of `GIP_HandleSystemMessage()`.
    fn handle_system_message(
        &mut self,
        gip: &mut GipDevice,
        io: &mut Io<'_, '_>,
        header: &Header,
        message: &[u8],
    ) -> bool {
        let num_bytes = message.len();
        if self.attachment_index > 0 && self.attachment_type == AttachmentType::Unknown {
            // XXX If we reattach to a controller after it's been initialized, it might have
            // attachments we don't know about. Try to figure out what this one is.
            if header.message_type == GIP_CMD_HID_REPORT && num_bytes == 8 {
                if self.keyboard == 0 {
                    self.add_keyboard(io);
                }
                self.attachment_type = AttachmentType::Chatpad;
                self.metadata.device.in_system_messages[0] |= 1 << GIP_CMD_HID_REPORT;
            }
        }
        if !self.supports_system_message(header.message_type, true) {
            crate::log::warn!(
                Category::Input,
                "GIP: Received claimed-unsupported system message type {:02x}",
                header.message_type
            );
            return false;
        }
        let bytes = &padded(message);
        match header.message_type {
            GIP_CMD_PROTO_CONTROL => self.handle_command_protocol_control(),
            GIP_CMD_HELLO_DEVICE => {
                self.handle_command_hello_device(gip, io, header, bytes, num_bytes)
            }
            GIP_CMD_STATUS_DEVICE => self.handle_command_status_device(gip, io, bytes, num_bytes),
            GIP_CMD_METADATA => self.handle_command_metadata_response(io, message),
            GIP_CMD_SECURITY => self.handle_command_security(),
            GIP_CMD_GUIDE_BUTTON => self.handle_command_guide_button_status(io, bytes),
            GIP_CMD_AUDIO_CONTROL => self.handle_command_audio_control(),
            GIP_CMD_FIRMWARE => self.handle_command_firmware(io, bytes, num_bytes),
            GIP_CMD_HID_REPORT => self.handle_command_hid_report(io, bytes, num_bytes),
            GIP_CMD_EXTENDED => self.handle_command_extended(io, header, bytes, num_bytes),
            GIP_AUDIO_DATA => self.handle_audio_data(),
            _ => {
                crate::log::warn!(
                    Category::Input,
                    "GIP: Received unknown system message type {:02x}",
                    header.message_type
                );
                false
            }
        }
    }

    /// Translation of `GIP_HandleMessage()`.
    fn handle_message(
        &mut self,
        gip: &mut GipDevice,
        io: &mut Io<'_, '_>,
        header: &Header,
        message: &[u8],
    ) -> bool {
        if header.flags & GIP_FLAG_SYSTEM != 0 {
            return self.handle_system_message(gip, io, header, message);
        }
        let num_bytes = message.len();
        let bytes = &padded(message);
        match header.message_type {
            GIP_CMD_RAW_REPORT => {
                if self.features & GIP_FEATURE_ELITE_BUTTONS != 0 {
                    return self.handle_command_raw_report(io, bytes, num_bytes);
                }
            }
            GIP_LL_INPUT_REPORT => return self.handle_ll_input_report(gip, io, bytes, num_bytes),
            GIP_LL_STATIC_CONFIGURATION => return self.handle_ll_static_configuration(),
            GIP_LL_BUTTON_INFO_REPORT => return self.handle_ll_button_info_report(),
            GIP_LL_OVERFLOW_INPUT_REPORT => return self.handle_ll_overflow_input_report(),
            _ => {}
        }
        crate::log::warn!(
            Category::Input,
            "GIP: Received unknown vendor message type {:02x}",
            header.message_type
        );
        false
    }

    /// Translation of `HIDAPI_DriverGIP_UpdateRumble()`.
    fn update_rumble(&mut self, port: &dyn GipPort) -> Result<()> {
        if self.features & GIP_FEATURE_MOTOR_CONTROL == 0 {
            return Ok(());
        }

        if self.rumble_state == RumbleState::Queued && self.rumble_time.load(Ordering::Acquire) != 0
        {
            self.rumble_state = RumbleState::Busy;
        }

        if self.rumble_state == RumbleState::Busy {
            const RUMBLE_BUSY_TIME_MS: u64 = 10;
            if port.ticks_ms() >= self.rumble_time.load(Ordering::Acquire) + RUMBLE_BUSY_TIME_MS {
                self.rumble_time.store(0, Ordering::Release);
                self.rumble_state = RumbleState::Idle;
            }
        }

        if !self.rumble_pending {
            return Ok(());
        }

        if self.rumble_state != RumbleState::Idle {
            return Ok(());
        }

        // We're no longer pending, even if we fail to send the rumble below
        self.rumble_pending = false;

        // (a zero byte, then the GIP_DirectMotor: the motors, their
        // levels, the duration, the delay and the repeat count)
        let message = [
            0,
            GIP_MOTOR_ALL,
            self.left_impulse_level,
            self.right_impulse_level,
            self.left_vibration_level,
            self.right_vibration_level,
            (RUMBLE_RESEND_MS / 10 + 5) as u8, // Add a 50ms leniency, just in case
            0,
            0,
        ];

        // (translation of HIDAPI_DriverGIP_RumbleSent())
        let rumble_time = self.rumble_time.clone();
        let on_sent: Box<dyn FnOnce(u64) + Send> =
            Box::new(move |ticks| rumble_time.store(ticks, Ordering::Release));

        let seq = self.sequence_next(GIP_CMD_DIRECT_MOTOR, false);
        if !send_raw_message(
            port,
            GIP_CMD_DIRECT_MOTOR,
            self.attachment_index,
            seq,
            &message,
            true,
            Some(on_sent),
        ) {
            return Err(Error::new("Couldn't send rumble packet"));
        }

        self.rumble_state = RumbleState::Queued;

        Ok(())
    }
}

/// A trigger of the gamepad and arcade stick reports: 10 bits, clamped,
/// to the full range.
fn trigger_axis(low: u8, high: u8) -> i16 {
    let axis = i16::from_le_bytes([low, high]).clamp(0, 1023);
    let axis = (axis - 512) * 64;
    if axis == 32704 {
        32767
    } else {
        axis
    }
}

/// An unsigned full-range axis of the flight stick report.
fn unsigned_axis(low: u8, high: u8) -> i16 {
    let axis = ((i32::from(high) << 8) - 0x8000) as i16;
    axis | i16::from(low)
}

/// Translation of `GIP_Device`.
#[derive(Debug, Default)]
struct GipContext {
    device: GipDevice,
    attachments: [Option<Attachment>; MAX_ATTACHMENTS],
}

impl GipContext {
    /// Translation of `GIP_EnsureAttachment()`.
    fn ensure_attachment(&mut self, attachment_index: u8) -> &mut Attachment {
        self.attachments[usize::from(attachment_index)]
            .get_or_insert_with(|| Attachment::new(attachment_index))
    }

    /// Translation of `GIP_ReceivePacket()`.
    fn receive_packet(&mut self, io: &mut Io<'_, '_>, bytes: &[u8]) {
        let num_bytes = bytes.len();
        let mut offset = 3;
        let mut ok = true;
        let mut fragment_offset: u64;
        let mut bytes_remaining: u16 = 0;

        if num_bytes < 5 {
            return;
        }

        let (length, length_size) = decode_length(&bytes[offset..]);
        let header = Header {
            message_type: bytes[0],
            flags: bytes[1],
            sequence_id: bytes[2],
            length,
        };
        offset += length_size;

        let is_fragment = header.flags & GIP_FLAG_FRAGMENT != 0;
        let attachment_index = header.flags & GIP_FLAG_ATTACHMENT_MASK;
        self.ensure_attachment(attachment_index);
        let GipContext {
            device: gip,
            attachments,
        } = self;
        let Some(attachment) = attachments[usize::from(attachment_index)].as_mut() else {
            return;
        };

        /* Handle coalescing fragmented messages */
        if is_fragment {
            if header.flags & GIP_FLAG_INIT_FRAG != 0 {
                if attachment.fragment_message != 0 {
                    /*
                     * Reset fragment buffer if we get a new initial
                     * fragment before finishing the last message.
                     * TODO: Is this the correct behavior?
                     */
                    attachment.fragment_data = None;
                }
                let (total_length, total_length_size) = decode_length(&bytes[offset..]);
                offset += total_length_size;
                if total_length > MAX_MESSAGE_LENGTH {
                    return;
                }
                attachment.total_length = total_length as u16;
                attachment.fragment_message = header.message_type;
                if header.length > (num_bytes - offset) as u64 {
                    crate::log::warn!(
                        Category::Input,
                        "GIP: Received fragment that claims to be {} bytes, expected {}",
                        header.length,
                        num_bytes - offset
                    );
                    return;
                }
                if header.length > total_length {
                    crate::log::warn!(
                        Category::Input,
                        "GIP: Received too long fragment, {} bytes, exceeds {}",
                        header.length,
                        attachment.total_length
                    );
                    return;
                }
                // Note (upstream): a buffer left by a transfer that timed
                // out leaks there; here it is dropped.
                let length = header.length as usize;
                let mut fragment_data = vec![0u8; usize::from(attachment.total_length)];
                fragment_data[..length].copy_from_slice(&bytes[offset..][..length]);
                attachment.fragment_data = Some(fragment_data);
                fragment_offset = header.length;
                attachment.fragment_offset = fragment_offset as u32;
                bytes_remaining =
                    u64::from(attachment.total_length).wrapping_sub(fragment_offset) as u16;
            } else {
                if header.message_type != attachment.fragment_message {
                    crate::log::warn!(
                        Category::Input,
                        "GIP: Received out of sequence message type {:02x}, expected {:02x}",
                        header.message_type,
                        attachment.fragment_message
                    );
                    attachment.fragment_failed(io.port, &header);
                    return;
                }

                let (claimed_offset, offset_size) = decode_length(&bytes[offset..]);
                fragment_offset = claimed_offset;
                offset += offset_size;
                // FIXME (upstream): the end of the fragment can wrap
                // around, passing the length check (and the copy then runs
                // far past both buffers); here it counts as too long.
                let fragment_end = fragment_offset.checked_add(header.length);
                if fragment_offset != u64::from(attachment.fragment_offset) {
                    crate::log::warn!(
                        Category::Input,
                        "GIP: Received out of sequence fragment, (claimed {}, expected {})",
                        fragment_offset,
                        attachment.fragment_offset
                    );
                    acknowledge(
                        io.port,
                        &header,
                        attachment.fragment_offset,
                        (u32::from(attachment.total_length)
                            .wrapping_sub(attachment.fragment_offset))
                            as u16,
                    );
                    return;
                } else if fragment_end.is_none_or(|end| end > u64::from(attachment.total_length)) {
                    crate::log::warn!(
                        Category::Input,
                        "GIP: Received too long fragment, {} exceeds {}",
                        fragment_offset.wrapping_add(header.length),
                        attachment.total_length
                    );
                    attachment.fragment_failed(io.port, &header);
                    return;
                }

                bytes_remaining = attachment
                    .total_length
                    .wrapping_sub(fragment_offset.wrapping_add(header.length) as u16);
                // FIXME (upstream): after an initial fragment that was too
                // long, the fragments go to a buffer that was never made
                // (a null pointer); here they're dropped, and so is the
                // message.
                if header.length != 0 {
                    if let Some(fragment_data) = attachment.fragment_data.as_mut() {
                        // FIXME (upstream): the fragment's claimed length
                        // isn't checked against the report, and the copy
                        // reads past it; here the bytes past the report
                        // read as 0.
                        let start = fragment_offset as usize;
                        let length = header.length as usize;
                        let available = &bytes[offset..];
                        let copied = length.min(available.len());
                        fragment_data[start..start + copied].copy_from_slice(&available[..copied]);
                        fragment_data[start + copied..start + length].fill(0);
                    }
                } else {
                    ok = match attachment.fragment_data.take() {
                        Some(fragment_data) => {
                            attachment.handle_message(gip, io, &header, &fragment_data)
                        }
                        None => false,
                    };
                    attachment.fragment_data = None;
                    attachment.fragment_message = 0;
                }
                fragment_offset = fragment_offset.wrapping_add(header.length);
                attachment.fragment_offset = u32::from(fragment_offset as u16);
            }
            attachment.fragment_timer = io.port.ticks_ms();
        } else if header.length.wrapping_add(offset as u64) > num_bytes as u64 {
            crate::log::warn!(
                Category::Input,
                "GIP: Received message with erroneous length (claimed {}, actual {}), discarding",
                header.length.wrapping_add(offset as u64),
                num_bytes
            );
            return;
        } else {
            fragment_offset = header.length;
            ok = attachment.handle_message(gip, io, &header, &bytes[offset..]);
        }

        if ok && header.flags & GIP_FLAG_ACME != 0 {
            acknowledge(io.port, &header, fragment_offset as u32, bytes_remaining);
        }
    }

    /// Translation of `HIDAPI_DriverGIP_FindAttachment()`.
    fn find_attachment(&mut self, joystick: JoystickID) -> Option<&mut Attachment> {
        crate::joystick::assert_joysticks_locked();

        self.attachments
            .iter_mut()
            .flatten()
            .find(|attachment| attachment.joystick == joystick)
    }

    /// The `InitDevice` of the driver, with its port.
    fn init(&mut self, io: &mut Io<'_, '_>) {
        self.device.reset_for_metadata =
            hints::get_bool(hints::JOYSTICK_HIDAPI_GIP_RESET_FOR_METADATA, true);

        let vendor_id = io.device.vendor_id();
        let product_id = io.device.product_id();
        self.ensure_attachment(0);
        let GipContext {
            device: gip,
            attachments,
        } = self;
        let Some(attachment) = attachments[0].as_mut() else {
            return;
        };
        attachment.handle_quirks(vendor_id, product_id);

        if attachment.quirks & GIP_QUIRK_NO_HELLO != 0 {
            gip.got_hello = true;
            attachment.ensure_metadata(gip, io);
        } else {
            gip.hello_deadline = io.port.ticks_ms() + GIP_HELLO_TIMEOUT;
        }

        io.device.set_gamepad_type(GamepadType::XboxOne);
    }

    /// The `OpenJoystick` of the driver, with its port.
    fn open(&mut self, io: &mut Io<'_, '_>, joystick: &mut JoystickData) -> Result<()> {
        let Some(attachment) = self.find_attachment(joystick.instance_id) else {
            return Err(Error::new("Invalid joystick"));
        };

        crate::joystick::assert_joysticks_locked();

        attachment.left_impulse_level = 0;
        attachment.right_impulse_level = 0;
        attachment.left_vibration_level = 0;
        attachment.right_vibration_level = 0;
        attachment.rumble_state = RumbleState::Idle;
        attachment.rumble_time.store(0, Ordering::Release);
        attachment.rumble_pending = false;
        attachment.last_input = [0; 64];

        // Initialize the joystick capabilities
        joystick.nbuttons = 11;
        attachment.enable_elite_buttons(io);
        if attachment.xbe_format != EliteButtonFormat::Unknown
            || (io.device.vendor_id() == USB_VENDOR_MICROSOFT
                && io.device.product_id() == USB_PRODUCT_XBOX_ONE_ELITE_SERIES_2)
        {
            attachment.paddle_idx = joystick.nbuttons as u8;
            joystick.nbuttons += 4;
        }
        if attachment.features & GIP_FEATURE_CONSOLE_FUNCTION_MAP != 0 {
            attachment.share_button_idx = joystick.nbuttons as u8;
            joystick.nbuttons += 1;
        }
        if attachment.extra_buttons > 0 {
            attachment.extra_button_idx = joystick.nbuttons as u8;
            joystick.nbuttons += attachment.extra_buttons as usize;
        }

        joystick.naxes = GamepadAxis::COUNT;
        if attachment.attachment_type == AttachmentType::FlightStick {
            /* Flight sticks have at least 4 axes, but only 3 are signed values, so we leave RIGHTY unused */
            joystick.naxes = (joystick.naxes as i32 + attachment.extra_axes - 1) as usize;
        }

        if attachment.attachment_type == AttachmentType::Guitar {
            io.device.set_joystick_type(JoystickType::Guitar);
        }

        if attachment.attachment_type == AttachmentType::DrumKit {
            io.device.set_joystick_type(JoystickType::DrumKit);
        }

        if attachment.attachment_type == AttachmentType::LiveGuitar {
            io.device.set_joystick_type(JoystickType::Guitar);
        }

        joystick.nhats = 1;

        Ok(())
    }

    /// The `RumbleJoystick` of the driver, with its port.
    fn rumble(
        &mut self,
        port: &dyn GipPort,
        joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        let Some(attachment) = self.find_attachment(joystick) else {
            return Err(Error::new("Invalid joystick"));
        };

        if attachment.features & GIP_FEATURE_MOTOR_CONTROL == 0 {
            return Err(Error::unsupported());
        }

        // Magnitude is 1..100 so scale the 16-bit input here
        attachment.left_vibration_level = (low_frequency_rumble / 655) as u8;
        attachment.right_vibration_level = (high_frequency_rumble / 655) as u8;
        attachment.rumble_pending = true;

        attachment.update_rumble(port)
    }

    /// The `RumbleJoystickTriggers` of the driver, with its port.
    fn rumble_triggers(
        &mut self,
        port: &dyn GipPort,
        joystick: JoystickID,
        left_rumble: u16,
        right_rumble: u16,
    ) -> Result<()> {
        let Some(attachment) = self.find_attachment(joystick) else {
            return Err(Error::new("Invalid joystick"));
        };

        if attachment.features & GIP_FEATURE_MOTOR_CONTROL == 0
            || attachment.quirks & GIP_QUIRK_NO_IMPULSE_VIBRATION != 0
        {
            return Err(Error::unsupported());
        }

        // Magnitude is 1..100 so scale the 16-bit input here
        attachment.left_impulse_level = (left_rumble / 655) as u8;
        attachment.right_impulse_level = (right_rumble / 655) as u8;
        attachment.rumble_pending = true;

        attachment.update_rumble(port)
    }

    /// The `SetJoystickLED` of the driver, with its port.
    fn set_led(
        &mut self,
        port: &dyn GipPort,
        joystick: JoystickID,
        red: u8,
        green: u8,
        blue: u8,
    ) -> Result<()> {
        let Some(attachment) = self.find_attachment(joystick) else {
            return Err(Error::new("Invalid joystick"));
        };

        if attachment.features & GIP_FEATURE_GUIDE_COLOR == 0 {
            return Err(Error::unsupported());
        }

        let buffer = [
            0x00, 0x00, // Whiteness? Sets white intensity when RGB is 0, seems additive
            red, green, blue,
        ];

        if !attachment.send_vendor_message(port, GIP_CMD_GUIDE_COLOR, 0, &buffer) {
            return Err(Error::new("Couldn't send LED packet"));
        }
        Ok(())
    }

    /// The `UpdateDevice` of the driver, with its port.
    fn update(&mut self, io: &mut Io<'_, '_>) -> bool {
        let mut bytes = [0u8; USB_PACKET_LENGTH];
        let mut perform_reset = false;

        let read_error = loop {
            match io.port.read_timeout(&mut bytes, self.device.timeout) {
                Ok(0) => break false,
                Ok(num_bytes) => {
                    self.device.timeout = 0;
                    self.receive_packet(io, &bytes[..num_bytes.min(bytes.len())]);
                }
                Err(_) => break true,
            }
        };

        let timestamp = io.port.ticks_ms();
        if self.device.hello_deadline != 0 && timestamp >= self.device.hello_deadline {
            self.device.hello_deadline = 0;
            perform_reset = true;
        }
        let GipContext {
            device: gip,
            attachments,
        } = self;
        for attachment in attachments.iter_mut().flatten() {
            if attachment.fragment_message != 0 && timestamp >= attachment.fragment_timer + 1000 {
                crate::log::warn!(Category::Input, "GIP: Reliable message transfer failed");
                attachment.fragment_message = 0;
            }
            if !perform_reset
                && attachment.got_metadata == MetadataStatus::Pending
                && timestamp >= attachment.metadata_next
                && attachment.fragment_message != GIP_CMD_METADATA
            {
                if attachment.metadata_retries < 3 {
                    crate::log::warn!(Category::Input, "GIP: Retrying metadata request");
                    attachment.metadata_retries += 1;
                    attachment.metadata_next = timestamp + 500;
                    attachment.send_system_message(io.port, GIP_CMD_METADATA, 0, &[]);
                } else {
                    perform_reset = true;
                }
            }
            if perform_reset {
                if gip.reset_for_metadata {
                    attachment.send_set_device_state(io.port, GIP_STATE_RESET);
                } else {
                    attachment.set_metadata_defaults(gip, io);
                    attachment.send_init_sequence(io);
                }
                perform_reset = false;
            }
            let _ = attachment.update_rumble(io.port);
        }

        if read_error && io.device.num_joysticks() > 0 {
            // Read error, device is disconnected
            for attachment in attachments.iter().flatten() {
                io.device.joystick_disconnected(attachment.joystick);
            }
        }
        !read_error
    }

    /// The `FreeDevice` of the driver: the chatpads' keyboards to remove.
    fn free(&mut self) -> Vec<KeyboardAction> {
        let mut actions = Vec::new();
        for slot in &mut self.attachments {
            let Some(attachment) = slot.take() else {
                continue;
            };
            if attachment.keyboard != 0 {
                actions.push(KeyboardAction::Remove(attachment.keyboard));
            }
        }
        actions
    }
}

/// The GIP driver's static functions.
pub(crate) struct GipDriver;

impl DriverImpl for GipDriver {
    /// Translation of `HIDAPI_DriverGIP_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[
            hints::JOYSTICK_HIDAPI_GIP,
            hints::JOYSTICK_HIDAPI_GIP_RESET_FOR_METADATA,
        ]
    }

    /// Translation of `HIDAPI_DriverGIP_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_GIP,
            hints::get_bool(
                hints::JOYSTICK_HIDAPI_XBOX_ONE,
                hints::get_bool(
                    hints::JOYSTICK_HIDAPI_XBOX,
                    hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
                ),
            ),
        )
    }

    /// Translation of `HIDAPI_DriverGIP_IsSupportedDevice()`.
    fn is_supported_device(
        &self,
        device: Option<&HidapiDevice>,
        _name: &str,
        gamepad_type: GamepadType,
        _vendor_id: u16,
        _product_id: u16,
        _version: u16,
        _interface_number: i32,
        _interface_class: i32,
        _interface_subclass: i32,
        _interface_protocol: i32,
    ) -> bool {
        // Xbox One controllers speak HID over bluetooth instead of GIP
        if device.is_some_and(HidapiDevice::is_bluetooth) {
            return false;
        }
        gamepad_type == GamepadType::XboxOne
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(GipContext::default())
    }
}

/// Run a driver function on the HIDAPI device, then do the chatpads'
/// keyboard actions.
fn with_port<R>(device: &mut DeviceCtx<'_>, f: impl FnOnce(&mut Io<'_, '_>) -> R) -> R {
    let hid = device.device().clone();
    let port = HidapiPort(&hid);
    let mut io = Io::new(&port, device);
    let result = f(&mut io);
    // (upstream calls the keyboard API as it goes, with the joystick lock
    // held; its events are about the chatpad alone, so they go after)
    send_keyboard_actions(io.keyboard);
    result
}

impl DriverContext for GipContext {
    /// Translation of `HIDAPI_DriverGIP_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        with_port(device, |io| self.init(io));
        Ok(())
    }

    /// Translation of `HIDAPI_DriverGIP_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        with_port(device, |io| self.update(io))
    }

    /// Translation of `HIDAPI_DriverGIP_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        with_port(device, |io| self.open(io, joystick))
    }

    /// Translation of `HIDAPI_DriverGIP_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        let hid = device.device().clone();
        self.rumble(
            &HidapiPort(&hid),
            joystick,
            low_frequency_rumble,
            high_frequency_rumble,
        )
    }

    /// Translation of `HIDAPI_DriverGIP_RumbleJoystickTriggers()`.
    fn rumble_joystick_triggers(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        left_rumble: u16,
        right_rumble: u16,
    ) -> Result<()> {
        let hid = device.device().clone();
        self.rumble_triggers(&HidapiPort(&hid), joystick, left_rumble, right_rumble)
    }

    /// Translation of `HIDAPI_DriverGIP_GetJoystickCapabilities()`.
    fn get_joystick_capabilities(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
    ) -> JoystickCaps {
        let mut result = JoystickCaps(0);
        let Some(attachment) = self.find_attachment(joystick) else {
            return result;
        };

        if attachment.features & GIP_FEATURE_MOTOR_CONTROL != 0 {
            result |= JoystickCaps::RUMBLE;
            if attachment.quirks & GIP_QUIRK_NO_IMPULSE_VIBRATION == 0 {
                result |= JoystickCaps::TRIGGER_RUMBLE;
            }
        }

        if attachment.features & GIP_FEATURE_GUIDE_COLOR != 0 {
            result |= JoystickCaps::RGB_LED;
        }

        result
    }

    /// Translation of `HIDAPI_DriverGIP_SetJoystickLED()`.
    fn set_joystick_led(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        red: u8,
        green: u8,
        blue: u8,
    ) -> Result<()> {
        let hid = device.device().clone();
        self.set_led(&HidapiPort(&hid), joystick, red, green, blue)
    }

    /// Translation of `HIDAPI_DriverGIP_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {}

    /// Translation of `HIDAPI_DriverGIP_FreeDevice()`.
    fn free_device(&mut self, _device: &mut DeviceCtx<'_>) {
        send_keyboard_actions(self.free());
    }
}

#[cfg(test)]
mod tests;
