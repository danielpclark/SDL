// Rust translation of src/joystick/hidapi/SDL_hidapi_ps3.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The PS3 controller drivers: Sony's DualShock 3 (and its ShanWan
//! clones), third party PS3 controllers, and the DualShock 3 through Sony's
//! sixaxis.sys Windows driver.

use super::ps4::{hat_of, read_feature_report};
use super::rumble::send_rumble;
use super::{
    load16, supports_playstation_detection, DeviceCtx, DriverContext, DriverImpl, HidapiDevice,
    JoystickCaps, SDL_HIDAPI_DEFAULT, USB_PACKET_LENGTH,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::{
    joystick_player_index_for_id, JoystickConnectionState, JoystickData, JoystickType, HAT_DOWN,
    HAT_LEFT, HAT_RIGHT, HAT_UP, JOYSTICK_AXIS_MAX, JOYSTICK_AXIS_MIN,
};
use crate::sensor::{SensorType, STANDARD_GRAVITY};

// EPS3ReportId
const REPORT_ID_STATE: u8 = 1;
const REPORT_ID_EFFECTS: u8 = 1;

// EPS3SonySixaxisReportId
const SONY_SIXAXIS_REPORT_ID_STATE: u8 = 0;
const SONY_SIXAXIS_REPORT_ID_EFFECTS: u8 = 0;

// Commands for Sony's sixaxis.sys Windows driver
// All commands must be sent using 49-byte buffer containing output report
// Byte 0 indicates reportId and must always be 0
// Byte 1 indicates a command, supported values are specified below:
// (EPS3SixaxisDriverCommands)

/// This command allows to set user LEDs.
/// Bytes 5,6.7.8 contain mode for corresponding LED: 0 - LED is off, 1 - LED in on, 2 - LED is flashing.
/// Bytes 9-16 specify 64-bit LED flash period in 100 ns units if some LED is flashing, otherwise not used.
const SIXAXIS_COMMAND_SET_LEDS: u8 = 1;

/// This command allows to set left and right motors.
/// Byte 5 is right motor duration (0-255) and byte 6, if not zero, activates right motor. Zero value disables right motor.
/// Byte 7 is left motor duration (0-255) and byte 8 is left motor amplitude (0-255).
const SIXAXIS_COMMAND_SET_MOTORS: u8 = 2;

// (k_EPS3SixaxisCommandBlockLEDs = 3, k_EPS3SixaxisCommandRefreshDriverSetting
// = 9 and k_EPS3SixaxisCommandClearPairing = 10 aren't used)

/// The size of an effects output report.
const EFFECTS_REPORT_SIZE: usize = 49;

/// The offsets of the buttons reported as axes, in the order they appear
/// in the button enumeration (0 for the buttons that aren't).
type ButtonAxisOffsets = [usize; 15];

/// The analog buttons of the DualShock 3 reports.
const PS3_BUTTON_AXIS_OFFSETS: ButtonAxisOffsets = [
    24, // SDL_GAMEPAD_BUTTON_SOUTH
    23, // SDL_GAMEPAD_BUTTON_EAST
    25, // SDL_GAMEPAD_BUTTON_WEST
    22, // SDL_GAMEPAD_BUTTON_NORTH
    0,  // SDL_GAMEPAD_BUTTON_BACK
    0,  // SDL_GAMEPAD_BUTTON_GUIDE
    0,  // SDL_GAMEPAD_BUTTON_START
    0,  // SDL_GAMEPAD_BUTTON_LEFT_STICK
    0,  // SDL_GAMEPAD_BUTTON_RIGHT_STICK
    20, // SDL_GAMEPAD_BUTTON_LEFT_SHOULDER
    21, // SDL_GAMEPAD_BUTTON_RIGHT_SHOULDER
    14, // SDL_GAMEPAD_BUTTON_DPAD_UP
    16, // SDL_GAMEPAD_BUTTON_DPAD_DOWN
    17, // SDL_GAMEPAD_BUTTON_DPAD_LEFT
    15, // SDL_GAMEPAD_BUTTON_DPAD_RIGHT
];

/// The analog buttons of the 18 byte third party reports.
const THIRD_PARTY_18_BUTTON_AXIS_OFFSETS: ButtonAxisOffsets = [
    12, // SDL_GAMEPAD_BUTTON_SOUTH
    11, // SDL_GAMEPAD_BUTTON_EAST
    13, // SDL_GAMEPAD_BUTTON_WEST
    10, // SDL_GAMEPAD_BUTTON_NORTH
    0,  // SDL_GAMEPAD_BUTTON_BACK
    0,  // SDL_GAMEPAD_BUTTON_GUIDE
    0,  // SDL_GAMEPAD_BUTTON_START
    0,  // SDL_GAMEPAD_BUTTON_LEFT_STICK
    0,  // SDL_GAMEPAD_BUTTON_RIGHT_STICK
    14, // SDL_GAMEPAD_BUTTON_LEFT_SHOULDER
    15, // SDL_GAMEPAD_BUTTON_RIGHT_SHOULDER
    8,  // SDL_GAMEPAD_BUTTON_DPAD_UP
    9,  // SDL_GAMEPAD_BUTTON_DPAD_DOWN
    7,  // SDL_GAMEPAD_BUTTON_DPAD_LEFT
    6,  // SDL_GAMEPAD_BUTTON_DPAD_RIGHT
];

/// The analog buttons of the 19+ byte third party reports.
const THIRD_PARTY_19_BUTTON_AXIS_OFFSETS: ButtonAxisOffsets = [
    13, // SDL_GAMEPAD_BUTTON_SOUTH
    12, // SDL_GAMEPAD_BUTTON_EAST
    14, // SDL_GAMEPAD_BUTTON_WEST
    11, // SDL_GAMEPAD_BUTTON_NORTH
    0,  // SDL_GAMEPAD_BUTTON_BACK
    0,  // SDL_GAMEPAD_BUTTON_GUIDE
    0,  // SDL_GAMEPAD_BUTTON_START
    0,  // SDL_GAMEPAD_BUTTON_LEFT_STICK
    0,  // SDL_GAMEPAD_BUTTON_RIGHT_STICK
    15, // SDL_GAMEPAD_BUTTON_LEFT_SHOULDER
    16, // SDL_GAMEPAD_BUTTON_RIGHT_SHOULDER
    9,  // SDL_GAMEPAD_BUTTON_DPAD_UP
    10, // SDL_GAMEPAD_BUTTON_DPAD_DOWN
    8,  // SDL_GAMEPAD_BUTTON_DPAD_LEFT
    7,  // SDL_GAMEPAD_BUTTON_DPAD_RIGHT
];

/// An axis of a full range byte (`((int)data[offset] * 257) - 32768`).
fn byte_axis(value: u8) -> i16 {
    (i32::from(value) * 257 - 32768) as i16
}

/// Translation of `HIDAPI_DriverPS3_ScaleAccel()`.
fn scale_accel(value: i16) -> f32 {
    // Accelerometer values are in big endian order
    // FIXME (upstream): LOAD16() already reads the bytes in little endian
    // order on every host, so this swap (SDL_Swap16BE()) only gives the big
    // endian value on little endian hosts.
    let value = i16::from_be(value);
    ((i32::from(value) - 511) as f32 / 113.0) * STANDARD_GRAVITY
}

/// Translation of `HIDAPI_DriverPS3ThirdParty_ScaleAccel()`.
fn third_party_scale_accel(value: i16) -> f32 {
    ((i32::from(value) - 512) as f32 / 113.0) * STANDARD_GRAVITY
}

/// The effects output report of `HIDAPI_DriverPS3_SendJoystickEffect()`.
fn effect_report(effect: &[u8]) -> [u8; EFFECTS_REPORT_SIZE] {
    let mut data = [0u8; EFFECTS_REPORT_SIZE];

    data[0] = REPORT_ID_EFFECTS;
    let offset = 1;
    let n = (data.len() - offset).min(effect.len());
    data[offset..offset + n].copy_from_slice(&effect[..n]);
    data
}

/// The effects output report of
/// `HIDAPI_DriverPS3SonySixaxis_SendJoystickEffect()`.
fn sixaxis_effect_report(effect: &[u8]) -> [u8; EFFECTS_REPORT_SIZE] {
    let mut data = [0u8; EFFECTS_REPORT_SIZE];

    data[0] = SONY_SIXAXIS_REPORT_ID_EFFECTS;

    // No offset with Sony sixaxis.sys driver
    let n = data.len().min(effect.len());
    data[..n].copy_from_slice(&effect[..n]);
    data
}

/// Send an effects output report as rumble.
fn send_effect_report(device: &DeviceCtx<'_>, data: &[u8; EFFECTS_REPORT_SIZE]) -> Result<()> {
    if send_rumble(device.device(), data).ok() != Some(data.len()) {
        return Err(Error::new("Couldn't send rumble packet"));
    }
    Ok(())
}

/// Whether a DualShock 3 is a ShanWan clone (part of
/// `HIDAPI_DriverPS3_InitDevice()`).
fn is_shanwan(vendor_id: u16, name: &str) -> bool {
    (vendor_id == USB_VENDOR_SONY
        && name
            .as_bytes()
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"ShanWan")))
        || vendor_id == USB_VENDOR_SHANWAN
        || vendor_id == USB_VENDOR_SHANWAN_ALT
}

/// The joystick type of a third party controller, if it isn't a gamepad
/// (part of `HIDAPI_DriverPS3ThirdParty_InitDevice()`).
fn third_party_joystick_type(vendor_id: u16, product_id: u16) -> Option<JoystickType> {
    match (vendor_id, product_id) {
        (
            USB_VENDOR_HARMONIX,
            USB_PRODUCT_HARMONIX_WII_RB1_GUITAR | USB_PRODUCT_HARMONIX_WII_RB2_GUITAR,
        ) => Some(JoystickType::Guitar),
        (
            USB_VENDOR_HARMONIX,
            USB_PRODUCT_HARMONIX_WII_RB1_DRUMS
            | USB_PRODUCT_HARMONIX_WII_RB2_DRUMS
            | USB_PRODUCT_HARMONIX_WII_RB3_MPA_DRUMS_MODE,
        ) => Some(JoystickType::DrumKit),
        (
            USB_VENDOR_SCEA,
            USB_PRODUCT_SCEA_PS3_GH_GUITAR
            | USB_PRODUCT_SCEA_PS3_RB_GUITAR
            | USB_PRODUCT_SCEA_PS3WIIU_GHLIVE,
        ) => Some(JoystickType::Guitar),
        (
            USB_VENDOR_SCEA,
            USB_PRODUCT_SCEA_PS3_GH_DRUMS
            | USB_PRODUCT_SCEA_PS3_RB_DRUMS
            | USB_PRODUCT_SCEA_PS3_RB3_MPA_DRUMS_MODE,
        ) => Some(JoystickType::DrumKit),
        _ => None,
    }
}

/// Whether a third party controller is the Switch retro controller, a
/// wireless controller without analog buttons.
fn is_switch_retro_controller(device: &HidapiDevice) -> bool {
    device.vendor_id() == USB_VENDOR_SWITCH
        && device.product_id() == USB_PRODUCT_SWITCH_RETRO_CONTROLLER
}

/// Translation of `SDL_DriverPS3_Context`, shared by the three drivers
/// (without its `device` and `joystick`, which are only passed along).
#[derive(Debug)]
struct Ps3Context {
    is_shanwan: bool,
    has_analog_buttons: bool,
    report_sensors: bool,
    effects_updated: bool,
    player_index: i32,
    rumble_left: u8,
    rumble_right: u8,
    last_state: [u8; USB_PACKET_LENGTH],
}

impl Default for Ps3Context {
    fn default() -> Self {
        Ps3Context {
            is_shanwan: false,
            has_analog_buttons: false,
            report_sensors: false,
            effects_updated: false,
            player_index: 0,
            rumble_left: 0,
            rumble_right: 0,
            last_state: [0; USB_PACKET_LENGTH],
        }
    }
}

impl Ps3Context {
    /// The joystick part of `HIDAPI_DriverPS3_OpenJoystick()`, shared with
    /// the other drivers.
    fn open(&mut self, joystick: &mut JoystickData) {
        self.last_state = [0; USB_PACKET_LENGTH];

        // Initialize the joystick capabilities
        joystick.nbuttons = 11;
        joystick.naxes = 6;
        if self.has_analog_buttons {
            joystick.naxes += 10;
        }
        joystick.nhats = 1;
    }

    /// The effects of `HIDAPI_DriverPS3_UpdateEffects()`.
    fn effects(&self) -> [u8; 35] {
        let mut effects = [
            0x01, 0xff, 0x00, 0xff, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, //
            0xff, 0x27, 0x10, 0x00, 0x32, //
            0xff, 0x27, 0x10, 0x00, 0x32, //
            0xff, 0x27, 0x10, 0x00, 0x32, //
            0xff, 0x27, 0x10, 0x00, 0x32, //
            0x00, 0x00, 0x00, 0x00, 0x00,
        ];

        effects[2] = u8::from(self.rumble_right != 0);
        effects[4] = self.rumble_left;

        // Note (upstream): a player index below -1 shifts by a negative
        // amount, which is undefined in C; no LED is lit for it here.
        effects[9] = 0x01u8
            .checked_shl((1 + self.player_index % 4) as u32)
            .unwrap_or(0);
        effects
    }

    /// Translation of `HIDAPI_DriverPS3_UpdateEffects()`.
    fn update_effects(&self, device: &DeviceCtx<'_>) -> Result<()> {
        send_effect_report(device, &effect_report(&self.effects()))
    }

    /// The effects of `HIDAPI_DriverPS3_UpdateRumbleSonySixaxis()`.
    fn sixaxis_rumble_effects(&self) -> [u8; 9] {
        let mut effects = [
            0x0,                        // Report Id
            SIXAXIS_COMMAND_SET_MOTORS, // 2 = Set Motors
            0x00,
            0x00,
            0x00, // padding
            0xff, // Small Motor duration - 0xff is forever
            0x00, // Small Motor off/on (0 or 1)
            0xff, // Large Motor duration - 0xff is forever
            0x00, // Large Motor force (0 to 255)
        ];

        effects[6] = u8::from(self.rumble_right != 0); // Small motor
        effects[8] = self.rumble_left; // Large motor
        effects
    }

    /// The effects of `HIDAPI_DriverPS3_UpdateLEDsSonySixaxis()`.
    fn sixaxis_led_effects(&self) -> [u8; 9] {
        let mut effects = [
            0x0,                      // Report Id
            SIXAXIS_COMMAND_SET_LEDS, // 1 = Set LEDs
            0x00,
            0x00,
            0x00, // padding
            0x00,
            0x00,
            0x00,
            0x00, // LED #4, LED #3, LED #2, LED #1 (0 = Off, 1 = On, 2 = Flashing)
        ];

        // Turn on LED light on DS3 Controller for relevant player (player_index 0 lights up LED #1, player_index 1 lights up LED #2, etc)
        // Note (upstream): upstream writes past the end of the effects for a
        // negative player index (-1 when there is none); nothing is lit here.
        if (0..4).contains(&self.player_index) {
            effects[8 - self.player_index as usize] = 1;
        }
        effects
    }

    /// Copy a report into the last state.
    fn save_state(&mut self, data: &[u8], size: usize) {
        let n = size.min(self.last_state.len()).min(data.len());
        self.last_state[..n].copy_from_slice(&data[..n]);
    }

    /// Translation of `HIDAPI_DriverPS3_HandleMiniStatePacket()`.
    fn handle_mini_state_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        let timestamp = crate::timer::ticks_ns();

        if self.last_state[4] != data[4] {
            device.send_hat(timestamp, joystick, 0, hat_of(data[4] & 0x0f));

            let mut button = |button: GamepadButton, down: bool| {
                device.send_button(timestamp, joystick, button as u8, down);
            };
            button(GamepadButton::North, data[4] & 0x10 != 0);
            button(GamepadButton::East, data[4] & 0x20 != 0);
            button(GamepadButton::South, data[4] & 0x40 != 0);
            button(GamepadButton::West, data[4] & 0x80 != 0);
        }

        if self.last_state[5] != data[5] {
            let trigger = |down: bool| {
                if down {
                    JOYSTICK_AXIS_MAX
                } else {
                    JOYSTICK_AXIS_MIN
                }
            };
            let mut button = |button: GamepadButton, down: bool| {
                device.send_button(timestamp, joystick, button as u8, down);
            };
            button(GamepadButton::LeftShoulder, data[5] & 0x01 != 0);
            button(GamepadButton::RightShoulder, data[5] & 0x02 != 0);
            device.send_axis(
                timestamp,
                joystick,
                GamepadAxis::LeftTrigger as u8,
                trigger(data[5] & 0x04 != 0),
            );
            device.send_axis(
                timestamp,
                joystick,
                GamepadAxis::RightTrigger as u8,
                trigger(data[5] & 0x08 != 0),
            );
            let mut button = |button: GamepadButton, down: bool| {
                device.send_button(timestamp, joystick, button as u8, down);
            };
            button(GamepadButton::Back, data[5] & 0x10 != 0);
            button(GamepadButton::Start, data[5] & 0x20 != 0);
            button(GamepadButton::LeftStick, data[5] & 0x40 != 0);
            button(GamepadButton::RightStick, data[5] & 0x80 != 0);
        }

        let mut axis = |axis: GamepadAxis, value: u8| {
            device.send_axis(timestamp, joystick, axis as u8, byte_axis(value));
        };
        axis(GamepadAxis::LeftX, data[2]);
        axis(GamepadAxis::LeftY, data[3]);
        axis(GamepadAxis::RightX, data[0]);
        axis(GamepadAxis::RightY, data[1]);

        self.save_state(data, size);
    }

    /// The buttons reported as axes, after the six axes.
    fn send_button_axes(
        &self,
        device: &mut DeviceCtx<'_>,
        timestamp: u64,
        joystick: JoystickID,
        data: &[u8],
        button_axis_offsets: &ButtonAxisOffsets,
    ) {
        // Buttons are mapped as axes in the order they appear in the button enumeration
        if self.has_analog_buttons {
            let mut axis_index = 6;

            for &offset in button_axis_offsets {
                if offset == 0 {
                    // This button doesn't report as an axis
                    continue;
                }

                device.send_axis(timestamp, joystick, axis_index, byte_axis(data[offset]));
                axis_index += 1;
            }
        }
    }

    /// Translation of `HIDAPI_DriverPS3_HandleStatePacket()`, which is the
    /// same as `HIDAPI_DriverPS3SonySixaxis_HandleStatePacket()`.
    fn handle_state_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        let timestamp = crate::timer::ticks_ns();

        if self.last_state[2] != data[2] {
            let mut hat = 0;

            let mut button = |button: GamepadButton, down: bool| {
                device.send_button(timestamp, joystick, button as u8, down);
            };
            button(GamepadButton::Back, data[2] & 0x01 != 0);
            button(GamepadButton::LeftStick, data[2] & 0x02 != 0);
            button(GamepadButton::RightStick, data[2] & 0x04 != 0);
            button(GamepadButton::Start, data[2] & 0x08 != 0);

            if data[2] & 0x10 != 0 {
                hat |= HAT_UP;
            }
            if data[2] & 0x20 != 0 {
                hat |= HAT_RIGHT;
            }
            if data[2] & 0x40 != 0 {
                hat |= HAT_DOWN;
            }
            if data[2] & 0x80 != 0 {
                hat |= HAT_LEFT;
            }
            device.send_hat(timestamp, joystick, 0, hat);
        }

        let mut button = |button: GamepadButton, down: bool| {
            device.send_button(timestamp, joystick, button as u8, down);
        };
        if self.last_state[3] != data[3] {
            button(GamepadButton::LeftShoulder, data[3] & 0x04 != 0);
            button(GamepadButton::RightShoulder, data[3] & 0x08 != 0);
            button(GamepadButton::North, data[3] & 0x10 != 0);
            button(GamepadButton::East, data[3] & 0x20 != 0);
            button(GamepadButton::South, data[3] & 0x40 != 0);
            button(GamepadButton::West, data[3] & 0x80 != 0);
        }

        if self.last_state[4] != data[4] {
            button(GamepadButton::Guide, data[4] & 0x01 != 0);
        }

        let mut axis = |axis: GamepadAxis, value: u8| {
            device.send_axis(timestamp, joystick, axis as u8, byte_axis(value));
        };
        axis(GamepadAxis::LeftTrigger, data[18]);
        axis(GamepadAxis::RightTrigger, data[19]);
        axis(GamepadAxis::LeftX, data[6]);
        axis(GamepadAxis::LeftY, data[7]);
        axis(GamepadAxis::RightX, data[8]);
        axis(GamepadAxis::RightY, data[9]);

        self.send_button_axes(device, timestamp, joystick, data, &PS3_BUTTON_AXIS_OFFSETS);

        if self.report_sensors {
            let sensor_data = [
                scale_accel(load16(data[41], data[42])),
                -scale_accel(load16(data[45], data[46])),
                -scale_accel(load16(data[43], data[44])),
            ];
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Accel,
                timestamp,
                &sensor_data,
            );
        }

        self.save_state(data, size);
    }

    /// Translation of `HIDAPI_DriverPS3ThirdParty_HandleStatePacket18()`.
    fn handle_state_packet_18(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        let timestamp = crate::timer::ticks_ns();

        let mut button = |button: GamepadButton, down: bool| {
            device.send_button(timestamp, joystick, button as u8, down);
        };
        if self.last_state[0] != data[0] {
            button(GamepadButton::West, data[0] & 0x01 != 0);
            button(GamepadButton::South, data[0] & 0x02 != 0);
            button(GamepadButton::East, data[0] & 0x04 != 0);
            button(GamepadButton::North, data[0] & 0x08 != 0);
            button(GamepadButton::LeftShoulder, data[0] & 0x10 != 0);
            button(GamepadButton::RightShoulder, data[0] & 0x20 != 0);
        }

        if self.last_state[1] != data[1] {
            button(GamepadButton::Back, data[1] & 0x01 != 0);
            button(GamepadButton::Start, data[1] & 0x02 != 0);
            button(GamepadButton::LeftStick, data[1] & 0x04 != 0);
            button(GamepadButton::RightStick, data[1] & 0x08 != 0);

            device.send_hat(timestamp, joystick, 0, hat_of(data[1] >> 4));
        }

        let mut axis = |axis: GamepadAxis, value: u8| {
            device.send_axis(timestamp, joystick, axis as u8, byte_axis(value));
        };
        axis(GamepadAxis::LeftTrigger, data[16]);
        axis(GamepadAxis::RightTrigger, data[17]);
        axis(GamepadAxis::LeftX, data[2]);
        axis(GamepadAxis::LeftY, data[3]);
        axis(GamepadAxis::RightX, data[4]);
        axis(GamepadAxis::RightY, data[5]);

        self.send_button_axes(
            device,
            timestamp,
            joystick,
            data,
            &THIRD_PARTY_18_BUTTON_AXIS_OFFSETS,
        );

        self.save_state(data, size);
    }

    /// Translation of `HIDAPI_DriverPS3ThirdParty_HandleStatePacket19()`.
    fn handle_state_packet_19(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        let timestamp = crate::timer::ticks_ns();

        let mut button = |button: GamepadButton, down: bool| {
            device.send_button(timestamp, joystick, button as u8, down);
        };
        if self.last_state[0] != data[0] {
            button(GamepadButton::West, data[0] & 0x01 != 0);
            button(GamepadButton::South, data[0] & 0x02 != 0);
            button(GamepadButton::East, data[0] & 0x04 != 0);
            button(GamepadButton::North, data[0] & 0x08 != 0);
            button(GamepadButton::LeftShoulder, data[0] & 0x10 != 0);
            button(GamepadButton::RightShoulder, data[0] & 0x20 != 0);
        }

        if self.last_state[1] != data[1] {
            button(GamepadButton::Back, data[1] & 0x01 != 0);
            button(GamepadButton::Start, data[1] & 0x02 != 0);
            button(GamepadButton::LeftStick, data[1] & 0x04 != 0);
            button(GamepadButton::RightStick, data[1] & 0x08 != 0);
            button(GamepadButton::Guide, data[1] & 0x10 != 0);
        }

        if device.vendor_id() == USB_VENDOR_SAITEK
            && device.product_id() == USB_PRODUCT_SAITEK_CYBORG_V3
        {
            // Cyborg V.3 Rumble Pad doesn't set the dpad bits as expected, so use the axes instead
            let mut hat = 0;

            if data[7] != 0 {
                hat |= HAT_RIGHT;
            }
            if data[8] != 0 {
                hat |= HAT_LEFT;
            }
            if data[9] != 0 {
                hat |= HAT_UP;
            }
            if data[10] != 0 {
                hat |= HAT_DOWN;
            }
            device.send_hat(timestamp, joystick, 0, hat);
        } else if self.last_state[2] != data[2] {
            device.send_hat(timestamp, joystick, 0, hat_of(data[2] & 0x0f));
        }

        let left_trigger = if data[0] & 0x40 != 0 {
            32767
        } else {
            byte_axis(data[17])
        };
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftTrigger as u8,
            left_trigger,
        );
        let right_trigger = if data[0] & 0x80 != 0 {
            32767
        } else {
            byte_axis(data[18])
        };
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightTrigger as u8,
            right_trigger,
        );
        let mut axis = |axis: GamepadAxis, value: u8| {
            device.send_axis(timestamp, joystick, axis as u8, byte_axis(value));
        };
        axis(GamepadAxis::LeftX, data[3]);
        axis(GamepadAxis::LeftY, data[4]);
        axis(GamepadAxis::RightX, data[5]);
        axis(GamepadAxis::RightY, data[6]);

        self.send_button_axes(
            device,
            timestamp,
            joystick,
            data,
            &THIRD_PARTY_19_BUTTON_AXIS_OFFSETS,
        );

        if self.report_sensors {
            let sensor_data = [
                -third_party_scale_accel(load16(data[19], data[20])),
                -third_party_scale_accel(load16(data[21], data[22])),
                -third_party_scale_accel(load16(data[23], data[24])),
            ];
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Accel,
                timestamp,
                &sensor_data,
            );
        }

        self.save_state(data, size);
    }
}

/// Read reports until there are no more, handling them with `handle`;
/// `false` on a read error, which disconnects the device's joystick (the
/// loop of the drivers' `UpdateDevice`).
fn read_reports(
    device: &mut DeviceCtx<'_>,
    mut handle: impl FnMut(&mut DeviceCtx<'_>, JoystickID, &[u8], usize),
) -> bool {
    let Some(&first) = device.joysticks().first() else {
        return false;
    };
    let joystick = device.joystick_open(first).then_some(first);

    let mut data = [0u8; USB_PACKET_LENGTH];
    let read_error = loop {
        match device.read_timeout(&mut data, 0) {
            Ok(0) => break false,
            Ok(size) => {
                let Some(joystick) = joystick else {
                    continue;
                };

                handle(device, joystick, &data, size);
            }
            Err(_) => break true,
        }
    };

    if read_error {
        // Read error, device is disconnected
        device.joystick_disconnected(first);
    }
    !read_error
}

// The DualShock 3 driver

/// The PS3 driver's static functions.
pub(crate) struct Ps3Driver;

impl DriverImpl for Ps3Driver {
    /// Translation of `HIDAPI_DriverPS3_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_PS3]
    }

    /// Translation of `HIDAPI_DriverPS3_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        let mut default_value = if cfg!(target_os = "macos") {
            // This works well on macOS
            true
        } else if cfg!(windows) {
            /* For official Sony driver (sixaxis.sys) use SDL_HINT_JOYSTICK_HIDAPI_PS3_SIXAXIS_DRIVER.
             *
             * See https://github.com/ViGEm/DsHidMini as an alternative driver
             */
            false
        } else if cfg!(target_os = "linux") {
            /* Linux drivers do a better job of managing the transition between
             * USB and Bluetooth. There are also some quirks in communicating
             * with PS3 controllers that have been implemented in SDL's hidapi
             * for libusb, but are not possible to support using hidraw if the
             * kernel doesn't already know about them.
             */
            false
        } else {
            // Untested, default off
            false
        };

        if default_value {
            default_value = hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT);
        }
        hints::get_bool(hints::JOYSTICK_HIDAPI_PS3, default_value)
    }

    /// Translation of `HIDAPI_DriverPS3_IsSupportedDevice()`.
    fn is_supported_device(
        &self,
        _device: Option<&HidapiDevice>,
        _name: &str,
        _gamepad_type: GamepadType,
        vendor_id: u16,
        product_id: u16,
        _version: u16,
        _interface_number: i32,
        _interface_class: i32,
        _interface_subclass: i32,
        _interface_protocol: i32,
    ) -> bool {
        (vendor_id == USB_VENDOR_SONY && product_id == USB_PRODUCT_SONY_DS3)
            || (vendor_id == USB_VENDOR_SHANWAN && product_id == USB_PRODUCT_SHANWAN_DS3)
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(Ps3OfficialContext::default())
    }
}

/// The context of the PS3 driver.
#[derive(Debug, Default)]
struct Ps3OfficialContext(Ps3Context);

impl Ps3OfficialContext {
    /// One report of `HIDAPI_DriverPS3_UpdateDevice()`; whether this is
    /// the first report, after which the LEDs are set (the controller
    /// stops blinking).
    fn handle_report(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) -> bool {
        let ctx = &mut self.0;

        if size == 7 {
            // Seen on a ShanWan PS2 -> PS3 USB converter
            ctx.handle_mini_state_packet(device, joystick, data, size);
        } else if data[0] == REPORT_ID_STATE {
            if data[1] == 0xFF {
                // Invalid data packet, ignore
                return false;
            }
            ctx.handle_state_packet(device, joystick, data, size);
        } else {
            // (unknown packet)
            return false;
        }

        // Wait for the first report to set the LED state after the controller stops blinking
        !std::mem::replace(&mut ctx.effects_updated, true)
    }
}

impl DriverContext for Ps3OfficialContext {
    /// Translation of `HIDAPI_DriverPS3_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        let ctx = &mut self.0;
        ctx.is_shanwan = is_shanwan(device.vendor_id(), &device.name());
        ctx.has_analog_buttons = true;

        if device.is_bluetooth() {
            // Set the controller into report mode over Bluetooth
            let data = [0xf4, 0x42, 0x03, 0x00, 0x00];

            if let Some(dev) = device.dev() {
                let _ = dev.send_feature_report(&data);
            }
        } else {
            // Set the controller into report mode over USB
            let mut data = [0u8; USB_PACKET_LENGTH];

            if read_feature_report(device, 0xf2, &mut data[..17]).is_err() {
                crate::log::debug!(
                    crate::log::Category::Input,
                    "HIDAPI_DriverPS3_InitDevice(): Couldn't read feature report 0xf2"
                );
                return Err(Error::new("Couldn't read feature report 0xf2"));
            }
            if read_feature_report(device, 0xf5, &mut data[..8]).is_err() {
                crate::log::debug!(
                    crate::log::Category::Input,
                    "HIDAPI_DriverPS3_InitDevice(): Couldn't read feature report 0xf5"
                );
                return Err(Error::new("Couldn't read feature report 0xf5"));
            }
            if !ctx.is_shanwan {
                // An output report could cause ShanWan controllers to rumble non-stop
                let _ = device.write(&data[..1]);
            }
        }

        device.set_gamepad_type(GamepadType::Ps3);
        device.set_device_name("PS3 Controller");

        device.joystick_connected();
        Ok(())
    }

    /// Translation of `HIDAPI_DriverPS3_SetDevicePlayerIndex()`.
    fn set_device_player_index(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _instance_id: JoystickID,
        player_index: i32,
    ) {
        self.0.player_index = player_index;

        // This will set the new LED state based on the new player index
        let _ = self.0.update_effects(device);
    }

    /// Translation of `HIDAPI_DriverPS3_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        read_reports(device, |device, joystick, data, size| {
            if self.handle_report(device, joystick, data, size) {
                let _ = self.0.update_effects(device);
            }
        })
    }

    /// Translation of `HIDAPI_DriverPS3_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        let ctx = &mut self.0;
        ctx.effects_updated = false;
        ctx.rumble_left = 0;
        ctx.rumble_right = 0;

        // Initialize player index (needed for setting LEDs)
        ctx.player_index = joystick_player_index_for_id(joystick.instance_id);

        ctx.open(joystick);

        joystick.add_sensor(SensorType::Accel, 100.0);

        Ok(())
    }

    /// Translation of `HIDAPI_DriverPS3_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        self.0.rumble_left = (low_frequency_rumble >> 8) as u8;
        self.0.rumble_right = (high_frequency_rumble >> 8) as u8;

        self.0.update_effects(device)
    }

    /// Translation of `HIDAPI_DriverPS3_GetJoystickCapabilities()`.
    fn get_joystick_capabilities(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
    ) -> JoystickCaps {
        JoystickCaps::RUMBLE
    }

    /// Translation of `HIDAPI_DriverPS3_SendJoystickEffect()`.
    fn send_joystick_effect(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        data: &[u8],
    ) -> Result<()> {
        send_effect_report(device, &effect_report(data))
    }

    /// Translation of `HIDAPI_DriverPS3_SetJoystickSensorsEnabled()`.
    fn set_joystick_sensors_enabled(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        enabled: bool,
    ) -> Result<()> {
        self.0.report_sensors = enabled;

        Ok(())
    }

    /// Translation of `HIDAPI_DriverPS3_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {}
}

// The third party PS3 controller driver

/// The third party PS3 driver's static functions.
pub(crate) struct Ps3ThirdPartyDriver;

impl DriverImpl for Ps3ThirdPartyDriver {
    /// Translation of `HIDAPI_DriverPS3_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_PS3]
    }

    /// Translation of `HIDAPI_DriverPS3ThirdParty_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_PS3,
            hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
        )
    }

    /// Translation of `HIDAPI_DriverPS3ThirdParty_IsSupportedDevice()`.
    fn is_supported_device(
        &self,
        device: Option<&HidapiDevice>,
        _name: &str,
        gamepad_type: GamepadType,
        vendor_id: u16,
        product_id: u16,
        _version: u16,
        _interface_number: i32,
        _interface_class: i32,
        _interface_subclass: i32,
        _interface_protocol: i32,
    ) -> bool {
        if vendor_id == USB_VENDOR_LOGITECH && product_id == USB_PRODUCT_LOGITECH_CHILLSTREAM {
            return true;
        }

        if (gamepad_type == GamepadType::Ps3 && vendor_id != USB_VENDOR_SONY)
            || supports_playstation_detection(vendor_id, product_id)
        {
            return match device.filter(|d| d.dev().is_some()) {
                Some(device) => {
                    let mut data = [0u8; USB_PACKET_LENGTH];
                    let size = read_feature_report(device, 0x03, &mut data);
                    if matches!(size, Ok(8)) && data[2] == 0x26 {
                        // Supported third party controller
                        true
                    } else {
                        // Some third party controllers don't have report ids
                        let size = read_feature_report(device, 0x00, &mut data);
                        // Supported third party controller
                        matches!(size, Ok(9)) && data[2] == 0x26
                    }
                }
                None => {
                    // Might be supported by this driver, enumerate and find out
                    true
                }
            };
        }
        false
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(Ps3ThirdPartyContext::default())
    }
}

/// The context of the third party PS3 driver.
#[derive(Debug, Default)]
struct Ps3ThirdPartyContext(Ps3Context);

impl Ps3ThirdPartyContext {
    /// One report of `HIDAPI_DriverPS3ThirdParty_UpdateDevice()`.
    fn handle_report(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        if size >= 19 {
            self.0.handle_state_packet_19(device, joystick, data, size);
        } else if size == 18 {
            // This packet format was seen with the Logitech ChillStream
            self.0.handle_state_packet_18(device, joystick, data, size);
        } else {
            // (unknown packet)
        }
    }
}

impl DriverContext for Ps3ThirdPartyContext {
    /// Translation of `HIDAPI_DriverPS3ThirdParty_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        self.0.has_analog_buttons = !is_switch_retro_controller(device);

        device.set_gamepad_type(GamepadType::Ps3);

        if device.vendor_id() == USB_VENDOR_LOGITECH
            && device.product_id() == USB_PRODUCT_LOGITECH_CHILLSTREAM
        {
            device.set_device_name("Logitech ChillStream");
        }

        if let Some(joystick_type) =
            third_party_joystick_type(device.vendor_id(), device.product_id())
        {
            device.set_joystick_type(joystick_type);
        }

        device.joystick_connected();
        Ok(())
    }

    /// Translation of `HIDAPI_DriverPS3ThirdParty_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        read_reports(device, |device, joystick, data, size| {
            self.handle_report(device, joystick, data, size)
        })
    }

    /// Translation of `HIDAPI_DriverPS3ThirdParty_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        self.0.open(joystick);

        if is_switch_retro_controller(device) {
            // This is a wireless controller using a USB dongle
            joystick.connection_state = JoystickConnectionState::Wireless;
        }

        joystick.add_sensor(SensorType::Accel, 100.0);

        Ok(())
    }

    /// Translation of `HIDAPI_DriverPS3ThirdParty_SetJoystickSensorsEnabled()`.
    fn set_joystick_sensors_enabled(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        enabled: bool,
    ) -> Result<()> {
        self.0.report_sensors = enabled;

        Ok(())
    }

    /// Translation of `HIDAPI_DriverPS3ThirdParty_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {}
}

// The sixaxis.sys driver

/// The Sony sixaxis.sys PS3 driver's static functions.
pub(crate) struct Ps3SonySixaxisDriver;

impl DriverImpl for Ps3SonySixaxisDriver {
    /// Translation of `HIDAPI_DriverPS3SonySixaxis_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_PS3_SIXAXIS_DRIVER]
    }

    /// Translation of `HIDAPI_DriverPS3SonySixaxis_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        cfg!(windows) && hints::get_bool(hints::JOYSTICK_HIDAPI_PS3_SIXAXIS_DRIVER, false)
    }

    /// Translation of `HIDAPI_DriverPS3SonySixaxis_IsSupportedDevice()`.
    fn is_supported_device(
        &self,
        _device: Option<&HidapiDevice>,
        _name: &str,
        _gamepad_type: GamepadType,
        vendor_id: u16,
        product_id: u16,
        _version: u16,
        _interface_number: i32,
        _interface_class: i32,
        _interface_subclass: i32,
        _interface_protocol: i32,
    ) -> bool {
        vendor_id == USB_VENDOR_SONY && product_id == USB_PRODUCT_SONY_DS3
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(Ps3SonySixaxisContext::default())
    }
}

/// The context of the Sony sixaxis.sys PS3 driver.
#[derive(Debug, Default)]
struct Ps3SonySixaxisContext(Ps3Context);

impl Ps3SonySixaxisContext {
    /// The handling of the feature report of
    /// `HIDAPI_DriverPS3SonySixaxis_UpdateDevice()`; whether this is the
    /// first report, after which the LEDs are set.
    fn handle_report(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) -> bool {
        match data[0] {
            SONY_SIXAXIS_REPORT_ID_STATE => {
                // report data starts in data[1]
                // Note (upstream): for an empty report upstream copies the
                // last state from one past the end of the buffer; here it
                // stops at the end.
                self.0
                    .handle_state_packet(device, joystick, &data[1..], size.wrapping_sub(1));

                // Wait for the first report to set the LED state after the controller stops blinking
                !std::mem::replace(&mut self.0.effects_updated, true)
            }
            _ => false,
        }
    }

    /// Translation of `HIDAPI_DriverPS3_UpdateRumbleSonySixaxis()`.
    fn update_rumble(&self, device: &DeviceCtx<'_>) -> Result<()> {
        send_effect_report(
            device,
            &sixaxis_effect_report(&self.0.sixaxis_rumble_effects()),
        )
    }

    /// Translation of `HIDAPI_DriverPS3_UpdateLEDsSonySixaxis()`.
    fn update_leds(&self, device: &DeviceCtx<'_>) -> Result<()> {
        send_effect_report(
            device,
            &sixaxis_effect_report(&self.0.sixaxis_led_effects()),
        )
    }
}

impl DriverContext for Ps3SonySixaxisContext {
    /// Translation of `HIDAPI_DriverPS3SonySixaxis_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        self.0.has_analog_buttons = true;

        let mut data = [0u8; USB_PACKET_LENGTH];

        if read_feature_report(device, 0xf2, &mut data).is_err() {
            crate::log::debug!(
                crate::log::Category::Input,
                "HIDAPI_DriverPS3SonySixaxis_InitDevice(): Couldn't read feature report 0xf2. Trying again with 0x0."
            );
            if read_feature_report(device, 0x00, &mut data).is_err() {
                crate::log::debug!(
                    crate::log::Category::Input,
                    "HIDAPI_DriverPS3SonySixaxis_InitDevice(): Couldn't read feature report 0x00."
                );
                return Err(Error::new("Couldn't read feature report 0x00"));
            }
        }

        device.set_gamepad_type(GamepadType::Ps3);
        device.set_device_name("PS3 Controller");

        device.joystick_connected();
        Ok(())
    }

    /// Translation of `HIDAPI_DriverPS3SonySixaxis_SetDevicePlayerIndex()`.
    fn set_device_player_index(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _instance_id: JoystickID,
        player_index: i32,
    ) {
        self.0.player_index = player_index;

        // This will set the new LED state based on the new player index
        let _ = self.update_leds(device);
    }

    /// Translation of `HIDAPI_DriverPS3SonySixaxis_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        let Some(joystick) = device.open_joystick_id() else {
            return false;
        };

        // With sixaxis.sys driver we need to use hid_get_feature_report instead of hid_read
        let mut data = [0u8; USB_PACKET_LENGTH];
        let Ok(size) = read_feature_report(device, 0x0, &mut data) else {
            crate::log::debug!(
                crate::log::Category::Input,
                "HIDAPI_DriverPS3SonySixaxis_UpdateDevice(): Couldn't read feature report 0x00"
            );
            return false;
        };

        if self.handle_report(device, joystick, &data, size) {
            let _ = self.update_leds(device);
        }

        // (upstream checks for a read error again here, which can't be one)
        true
    }

    /// Translation of `HIDAPI_DriverPS3SonySixaxis_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        let ctx = &mut self.0;
        ctx.effects_updated = false;
        ctx.rumble_left = 0;
        ctx.rumble_right = 0;

        // Initialize player index (needed for setting LEDs)
        ctx.player_index = joystick_player_index_for_id(joystick.instance_id);

        ctx.open(joystick);

        joystick.add_sensor(SensorType::Accel, 100.0);

        Ok(())
    }

    /// Translation of `HIDAPI_DriverPS3SonySixaxis_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        self.0.rumble_left = (low_frequency_rumble >> 8) as u8;
        self.0.rumble_right = (high_frequency_rumble >> 8) as u8;

        self.update_rumble(device)
    }

    /// Translation of `HIDAPI_DriverPS3SonySixaxis_SendJoystickEffect()`.
    fn send_joystick_effect(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        data: &[u8],
    ) -> Result<()> {
        send_effect_report(device, &sixaxis_effect_report(data))
    }

    /// Translation of `HIDAPI_DriverPS3SonySixaxis_SetJoystickSensorsEnabled()`.
    fn set_joystick_sensors_enabled(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        enabled: bool,
    ) -> Result<()> {
        self.0.report_sensors = enabled;

        Ok(())
    }

    /// Translation of `HIDAPI_DriverPS3SonySixaxis_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {}
}

#[cfg(test)]
mod tests;
