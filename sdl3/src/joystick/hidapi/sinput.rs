// Rust translation of src/joystick/hidapi/SDL_hidapi_sinput.c and
// SDL_hidapi_sinput.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The SInput (Open Format) controller driver: controllers that describe
//! their features (buttons, sticks, triggers, sensors, touchpads, LEDs and
//! rumble) in a reply to a command, which also gives the gamepad mapping
//! the closest sane layout.
//!
//! This protocol is documented at:
//! <https://docs.handheldlegend.com/s/sinput>
//!
//! The commands and their replies go through the HID I/O of the Valve
//! drivers ([`SteamHid`]), which the tests fake.

use std::time::Duration;

use super::rumble::send_rumble;
use super::steam::SteamHid;
use super::{
    DeviceCtx, DriverContext, DriverImpl, HidapiDevice, JoystickCaps, SDL_HIDAPI_DEFAULT,
    USB_PACKET_LENGTH,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::device_info::is_joystick_sinput_controller;
use crate::joystick::gamepad::{GamepadButton, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::{JoystickData, HAT_CENTERED, HAT_DOWN, HAT_LEFT, HAT_RIGHT, HAT_UP};
use crate::power::PowerState;
use crate::sensor::{SensorType, STANDARD_GRAVITY};

// SDL_hidapi_sinput.h: the styles of the controller parts, which make the
// version of the GUID

const SINPUT_ANALOGSTYLE_NONE: i32 = 0;
const SINPUT_ANALOGSTYLE_LEFTONLY: i32 = 1;
const SINPUT_ANALOGSTYLE_RIGHTONLY: i32 = 2;
const SINPUT_ANALOGSTYLE_LEFTRIGHT: i32 = 3;

const SINPUT_BUMPERSTYLE_NONE: i32 = 0;
const SINPUT_BUMPERSTYLE_ONE: i32 = 1;
const SINPUT_BUMPERSTYLE_TWO: i32 = 2;
const SINPUT_BUMPERSTYLE_MAX: i32 = 3;

const SINPUT_TRIGGERSTYLE_NONE: i32 = 0;
const SINPUT_TRIGGERSTYLE_ANALOG: i32 = 1;
const SINPUT_TRIGGERSTYLE_DIGITAL: i32 = 2;
const SINPUT_TRIGGERSTYLE_DUALSTAGE: i32 = 3;
const SINPUT_TRIGGERSTYLE_MAX: i32 = 4;

const SINPUT_PADDLESTYLE_NONE: i32 = 0;
const SINPUT_PADDLESTYLE_TWO: i32 = 1;
const SINPUT_PADDLESTYLE_FOUR: i32 = 2;
const SINPUT_PADDLESTYLE_MAX: i32 = 3;

const SINPUT_METASTYLE_NONE: i32 = 0;
const SINPUT_METASTYLE_BACK: i32 = 1;
const SINPUT_METASTYLE_BACKGUIDE: i32 = 2;
const SINPUT_METASTYLE_BACKGUIDESHARE: i32 = 3;
const SINPUT_METASTYLE_MAX: i32 = 4;

const SINPUT_TOUCHSTYLE_NONE: i32 = 0;
const SINPUT_TOUCHSTYLE_SINGLE: i32 = 1;
const SINPUT_TOUCHSTYLE_DOUBLE: i32 = 2;
const SINPUT_TOUCHSTYLE_MAX: i32 = 3;

const SINPUT_MISCSTYLE_NONE: i32 = 0;
const SINPUT_MISCSTYLE_1: i32 = 1;
const SINPUT_MISCSTYLE_2: i32 = 2;
const SINPUT_MISCSTYLE_3: i32 = 3;
const SINPUT_MISCSTYLE_4: i32 = 4;
const SINPUT_MISCSTYLE_MAX: i32 = 5;

/// Size of input reports (And CMD Input reports)
const SINPUT_DEVICE_REPORT_SIZE: usize = 64;
/// Size of command OUTPUT reports
const SINPUT_DEVICE_REPORT_COMMAND_SIZE: usize = 48;

const SINPUT_DEVICE_REPORT_ID_JOYSTICK_INPUT: u8 = 0x01;
const SINPUT_DEVICE_REPORT_ID_INPUT_CMDDAT: u8 = 0x02;
const SINPUT_DEVICE_REPORT_ID_OUTPUT_CMDDAT: u8 = 0x03;

const SINPUT_DEVICE_COMMAND_HAPTIC: u8 = 0x01;
const SINPUT_DEVICE_COMMAND_FEATURES: u8 = 0x02;
const SINPUT_DEVICE_COMMAND_PLAYERLED: u8 = 0x03;
const SINPUT_DEVICE_COMMAND_JOYSTICKRGB: u8 = 0x04;

const SINPUT_REPORT_IDX_BUTTONS_0: usize = 3;
const SINPUT_REPORT_IDX_LEFT_X: usize = 7;
const SINPUT_REPORT_IDX_LEFT_Y: usize = 9;
const SINPUT_REPORT_IDX_RIGHT_X: usize = 11;
const SINPUT_REPORT_IDX_RIGHT_Y: usize = 13;
const SINPUT_REPORT_IDX_LEFT_TRIGGER: usize = 15;
const SINPUT_REPORT_IDX_RIGHT_TRIGGER: usize = 17;
const SINPUT_REPORT_IDX_IMU_TIMESTAMP: usize = 19;
const SINPUT_REPORT_IDX_IMU_ACCEL_X: usize = 23;
const SINPUT_REPORT_IDX_IMU_ACCEL_Y: usize = 25;
const SINPUT_REPORT_IDX_IMU_ACCEL_Z: usize = 27;
const SINPUT_REPORT_IDX_IMU_GYRO_X: usize = 29;
const SINPUT_REPORT_IDX_IMU_GYRO_Y: usize = 31;
const SINPUT_REPORT_IDX_IMU_GYRO_Z: usize = 33;
const SINPUT_REPORT_IDX_TOUCH1_X: usize = 35;
const SINPUT_REPORT_IDX_TOUCH1_Y: usize = 37;
const SINPUT_REPORT_IDX_TOUCH1_P: usize = 39;
const SINPUT_REPORT_IDX_TOUCH2_X: usize = 41;
const SINPUT_REPORT_IDX_TOUCH2_Y: usize = 43;
const SINPUT_REPORT_IDX_TOUCH2_P: usize = 45;

const SINPUT_BUTTON_IDX_DPAD_UP: u8 = 4;
const SINPUT_BUTTON_IDX_DPAD_DOWN: u8 = 5;
const SINPUT_BUTTON_IDX_DPAD_LEFT: u8 = 6;
const SINPUT_BUTTON_IDX_DPAD_RIGHT: u8 = 7;

const SINPUT_BUTTONMASK_LEFT_STICK: u8 = 0x01;
const SINPUT_BUTTONMASK_RIGHT_STICK: u8 = 0x02;
const SINPUT_BUTTONMASK_LEFT_BUMPER: u8 = 0x04;
const SINPUT_BUTTONMASK_RIGHT_BUMPER: u8 = 0x08;
const SINPUT_BUTTONMASK_LEFT_TRIGGER: u8 = 0x10;
const SINPUT_BUTTONMASK_RIGHT_TRIGGER: u8 = 0x20;
const SINPUT_BUTTONMASK_LEFT_PADDLE1: u8 = 0x40;
const SINPUT_BUTTONMASK_RIGHT_PADDLE1: u8 = 0x80;
const SINPUT_BUTTONMASK_START: u8 = 0x01;
const SINPUT_BUTTONMASK_BACK: u8 = 0x02;
const SINPUT_BUTTONMASK_GUIDE: u8 = 0x04;
const SINPUT_BUTTONMASK_CAPTURE: u8 = 0x08;
const SINPUT_BUTTONMASK_LEFT_PADDLE2: u8 = 0x10;
const SINPUT_BUTTONMASK_RIGHT_PADDLE2: u8 = 0x20;
const SINPUT_BUTTONMASK_TOUCHPAD1: u8 = 0x40;
const SINPUT_BUTTONMASK_TOUCHPAD2: u8 = 0x80;

const SINPUT_REPORT_IDX_COMMAND_RESPONSE_ID: usize = 1;
const SINPUT_REPORT_IDX_COMMAND_RESPONSE_BULK: usize = 2;

const SINPUT_REPORT_IDX_PLUG_STATUS: usize = 1;
const SINPUT_REPORT_IDX_CHARGE_LEVEL: usize = 2;

const SINPUT_MAX_ALLOWED_TOUCHPADS: u8 = 2;

/// `EXTRACTSINT16()`
fn extract_i16(data: &[u8], idx: usize) -> i16 {
    i16::from_le_bytes([data[idx], data[idx + 1]])
}

/// `EXTRACTUINT16()`
fn extract_u16(data: &[u8], idx: usize) -> u16 {
    u16::from_le_bytes([data[idx], data[idx + 1]])
}

/// `EXTRACTUINT32()`
fn extract_u32(data: &[u8], idx: usize) -> u32 {
    u32::from_le_bytes([data[idx], data[idx + 1], data[idx + 2], data[idx + 3]])
}

/// Converts raw int16_t gyro scale setting (`CalculateGyroScale()`)
fn calculate_gyro_scale(dps_range: u16) -> f32 {
    std::f32::consts::PI / 180.0 / (32768.0 / f32::from(dps_range))
}

/// Converts raw int16_t accel scale setting (`CalculateAccelScale()`)
fn calculate_accel_scale(g_range: u16) -> f32 {
    STANDARD_GRAVITY / (32768.0 / f32::from(g_range))
}

/// A command report (`SINPUT_DEVICE_REPORT_COMMAND_SIZE` bytes, zero
/// padded).
fn command_report(command: u8, args: &[u8]) -> [u8; SINPUT_DEVICE_REPORT_COMMAND_SIZE] {
    let mut report = [0u8; SINPUT_DEVICE_REPORT_COMMAND_SIZE];
    report[0] = SINPUT_DEVICE_REPORT_ID_OUTPUT_CMDDAT;
    report[1] = command;
    report[2..2 + args.len()].copy_from_slice(args);
    report
}

/// The rumble report of `HIDAPI_DriverSInput_RumbleJoystick()`: type 2
/// haptics, the basic ERM simulation model (`SINPUT_HAPTIC_S` packed by
/// `HapticsType2Pack()`), without brakes.
fn rumble_report(
    low_frequency_rumble: u16,
    high_frequency_rumble: u16,
) -> [u8; SINPUT_DEVICE_REPORT_COMMAND_SIZE] {
    // Low Frequency  = Left
    // High Frequency = Right
    let left_amplitude = (low_frequency_rumble >> 8) as u8;
    let right_amplitude = (high_frequency_rumble >> 8) as u8;
    let (left_brake, right_brake) = (false, false);

    command_report(
        SINPUT_DEVICE_COMMAND_HAPTIC,
        &[
            // Type of haptics
            2,
            left_amplitude,
            u8::from(left_brake),
            right_amplitude,
            u8::from(right_brake),
        ],
    )
}

/// The SInput driver's static functions.
pub(crate) struct SInputDriver;

impl DriverImpl for SInputDriver {
    /// Translation of `HIDAPI_DriverSInput_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_SINPUT]
    }

    /// Translation of `HIDAPI_DriverSInput_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_SINPUT,
            hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
        )
    }

    /// Translation of `HIDAPI_DriverSInput_IsSupportedDevice()`.
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
        is_joystick_sinput_controller(vendor_id, product_id)
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(SInputContext::default())
    }
}

/// Translation of `SDL_DriverSInput_Context` (without the fields upstream
/// only writes: `protocol_version`, `usb_device_version`, `player_idx`,
/// `sub_product`, `is_handheld`, `accelRange` and `gyroRange`).
#[derive(Debug)]
struct SInputContext {
    sensors_enabled: bool,

    player_leds_supported: bool,
    joystick_rgb_supported: bool,
    rumble_supported: bool,
    accelerometer_supported: bool,
    gyroscope_supported: bool,
    left_analog_stick_supported: bool,
    right_analog_stick_supported: bool,
    left_analog_trigger_supported: bool,
    right_analog_trigger_supported: bool,
    dpad_supported: bool,
    touchpad_supported: bool,

    /// 2 touchpads maximum
    touchpad_count: u8,
    /// 2 fingers for one touchpad, or 1 per touchpad (2 max)
    touchpad_finger_count: u8,

    polling_rate_us: u16,

    /// Scale factor for accelerometer values
    accel_scale: f32,
    /// Scale factor for gyroscope values
    gyro_scale: f32,
    last_state: [u8; USB_PACKET_LENGTH],

    axes_count: u8,
    buttons_count: u8,
    usage_masks: [u8; 4],

    last_imu_timestamp_us: u32,

    /// Nanoseconds. We accumulate with received deltas
    imu_timestamp_ns: u64,
}

impl Default for SInputContext {
    fn default() -> Self {
        SInputContext {
            sensors_enabled: false,
            player_leds_supported: false,
            joystick_rgb_supported: false,
            rumble_supported: false,
            accelerometer_supported: false,
            gyroscope_supported: false,
            left_analog_stick_supported: false,
            right_analog_stick_supported: false,
            left_analog_trigger_supported: false,
            right_analog_trigger_supported: false,
            dpad_supported: false,
            touchpad_supported: false,
            touchpad_count: 0,
            touchpad_finger_count: 0,
            polling_rate_us: 0,
            accel_scale: 0.0,
            gyro_scale: 0.0,
            last_state: [0; USB_PACKET_LENGTH],
            axes_count: 0,
            buttons_count: 0,
            usage_masks: [0; 4],
            last_imu_timestamp_us: 0,
            imu_timestamp_ns: 0,
        }
    }
}

/// The name of a controller, if it has a better one than its USB name
/// (part of `HIDAPI_DriverSInput_InitDevice()`).
fn device_name(product_id: u16) -> Option<&'static str> {
    match product_id {
        USB_PRODUCT_HANDHELDLEGEND_GCULTIMATE => Some("HHL GC Ultimate"),
        USB_PRODUCT_HANDHELDLEGEND_PROGCC => Some("HHL ProGCC"),
        USB_PRODUCT_VOIDGAMING_PS4FIREBIRD => Some("Void Gaming PS4 FireBird"),
        USB_PRODUCT_VOIDGAMING_GENESIS_SINPUT => Some("Void Gaming Void GENESIS"),
        USB_PRODUCT_BONZIRICHANNEL_FIREBIRD => Some("Bonziri FireBird"),
        // USB_PRODUCT_HANDHELDLEGEND_SINPUT_GENERIC and the others:
        // use the USB product name
        _ => None,
    }
}

impl SInputContext {
    /// Translation of `DeviceDynamicEncodingSetup()`: this uses base-n
    /// encoding to encode features into the version GUID bytes that
    /// properly represents the supported device features; this also sets
    /// the driver context button mask correctly based on the features.
    /// Returns the version.
    fn device_dynamic_encoding_setup(&mut self) -> u16 {
        // A new button mask is generated to provide
        // SDL with a mapping string that is sane. In case of
        // an unconventional gamepad setup, the closest sane
        // mapping is provided to the driver.
        let mut mask = [0u8; 4];

        // For all gamepads, there is a minimum SInput expectation
        // to have dpad, abxy, and start buttons

        // ABXY + D-Pad
        mask[0] = 0xFF;
        self.dpad_supported = true;

        // Start button
        mask[2] |= SINPUT_BUTTONMASK_START;

        // Bumpers
        let left_bumper = (self.usage_masks[1] & SINPUT_BUTTONMASK_LEFT_BUMPER) != 0;
        let right_bumper = (self.usage_masks[1] & SINPUT_BUTTONMASK_RIGHT_BUMPER) != 0;

        let mut bumper_style = SINPUT_BUMPERSTYLE_NONE;
        if left_bumper && right_bumper {
            bumper_style = SINPUT_BUMPERSTYLE_TWO;
            mask[1] |= SINPUT_BUTTONMASK_LEFT_BUMPER | SINPUT_BUTTONMASK_RIGHT_BUMPER;
        } else if left_bumper || right_bumper {
            bumper_style = SINPUT_BUMPERSTYLE_ONE;

            if left_bumper {
                mask[1] |= SINPUT_BUTTONMASK_LEFT_BUMPER;
            } else if right_bumper {
                mask[1] |= SINPUT_BUTTONMASK_RIGHT_BUMPER;
            }
        }

        // Trigger bits live in mask[1]
        let digital_triggers = (self.usage_masks[1]
            & (SINPUT_BUTTONMASK_LEFT_TRIGGER | SINPUT_BUTTONMASK_RIGHT_TRIGGER))
            != 0;

        let analog_triggers =
            self.left_analog_trigger_supported || self.right_analog_trigger_supported;

        // Touchpads
        let t1 = (self.usage_masks[2] & SINPUT_BUTTONMASK_TOUCHPAD1) != 0;
        let t2 = (self.usage_masks[2] & SINPUT_BUTTONMASK_TOUCHPAD2) != 0;

        let mut analog_style = SINPUT_ANALOGSTYLE_NONE;
        if self.left_analog_stick_supported && self.right_analog_stick_supported {
            analog_style = SINPUT_ANALOGSTYLE_LEFTRIGHT;
            mask[1] |= SINPUT_BUTTONMASK_LEFT_STICK | SINPUT_BUTTONMASK_RIGHT_STICK;
        } else if self.left_analog_stick_supported {
            analog_style = SINPUT_ANALOGSTYLE_LEFTONLY;
            mask[1] |= SINPUT_BUTTONMASK_LEFT_STICK;
        } else if self.right_analog_stick_supported {
            analog_style = SINPUT_ANALOGSTYLE_RIGHTONLY;
            mask[1] |= SINPUT_BUTTONMASK_RIGHT_STICK;
        }

        let mut trigger_style = SINPUT_TRIGGERSTYLE_NONE;

        if analog_triggers && digital_triggers {
            // When we have both analog triggers and digital triggers
            // this is interpreted as having dual-stage triggers
            trigger_style = SINPUT_TRIGGERSTYLE_DUALSTAGE;
            mask[1] |= SINPUT_BUTTONMASK_LEFT_TRIGGER | SINPUT_BUTTONMASK_RIGHT_TRIGGER;
        } else if analog_triggers {
            trigger_style = SINPUT_TRIGGERSTYLE_ANALOG;
        } else if digital_triggers {
            trigger_style = SINPUT_TRIGGERSTYLE_DIGITAL;
            mask[1] |= SINPUT_BUTTONMASK_LEFT_TRIGGER | SINPUT_BUTTONMASK_RIGHT_TRIGGER;
        }

        // Paddle bits may touch mask[1] and mask[2]
        let pg1 = (self.usage_masks[1]
            & (SINPUT_BUTTONMASK_LEFT_PADDLE1 | SINPUT_BUTTONMASK_RIGHT_PADDLE1))
            != 0;
        let pg2 = (self.usage_masks[2]
            & (SINPUT_BUTTONMASK_LEFT_PADDLE2 | SINPUT_BUTTONMASK_RIGHT_PADDLE2))
            != 0;

        let mut paddle_style = SINPUT_PADDLESTYLE_NONE;
        if pg1 && pg2 {
            paddle_style = SINPUT_PADDLESTYLE_FOUR;
            mask[1] |= SINPUT_BUTTONMASK_LEFT_PADDLE1 | SINPUT_BUTTONMASK_RIGHT_PADDLE1;
            mask[2] |= SINPUT_BUTTONMASK_LEFT_PADDLE2 | SINPUT_BUTTONMASK_RIGHT_PADDLE2;
        } else if pg1 {
            paddle_style = SINPUT_PADDLESTYLE_TWO;
            mask[1] |= SINPUT_BUTTONMASK_LEFT_PADDLE1 | SINPUT_BUTTONMASK_RIGHT_PADDLE1;
        }

        // Meta Buttons (Back, Guide, Share)
        let back = (self.usage_masks[2] & SINPUT_BUTTONMASK_BACK) != 0;
        let guide = (self.usage_masks[2] & SINPUT_BUTTONMASK_GUIDE) != 0;
        let share = (self.usage_masks[2] & SINPUT_BUTTONMASK_CAPTURE) != 0;

        let mut meta_style = SINPUT_METASTYLE_NONE;
        if share {
            meta_style = SINPUT_METASTYLE_BACKGUIDESHARE;
            mask[2] |= SINPUT_BUTTONMASK_BACK | SINPUT_BUTTONMASK_GUIDE | SINPUT_BUTTONMASK_CAPTURE;
        } else if guide {
            meta_style = SINPUT_METASTYLE_BACKGUIDE;
            mask[2] |= SINPUT_BUTTONMASK_BACK | SINPUT_BUTTONMASK_GUIDE;
        } else if back {
            meta_style = SINPUT_METASTYLE_BACK;
            mask[2] |= SINPUT_BUTTONMASK_BACK;
        }

        let mut touch_style = SINPUT_TOUCHSTYLE_NONE;
        if t1 && t2 {
            touch_style = SINPUT_TOUCHSTYLE_DOUBLE;
            mask[2] |= SINPUT_BUTTONMASK_TOUCHPAD1 | SINPUT_BUTTONMASK_TOUCHPAD2;
        } else if t1 {
            touch_style = SINPUT_TOUCHSTYLE_SINGLE;
            mask[2] |= SINPUT_BUTTONMASK_TOUCHPAD1;
        }

        // Misc Buttons
        let extra_misc = self.usage_masks[3] & 0x0F;
        let misc_style = match extra_misc {
            0x0F => {
                mask[3] = 0x0F;
                SINPUT_MISCSTYLE_4
            }
            0x07 => {
                mask[3] = 0x07;
                SINPUT_MISCSTYLE_3
            }
            0x03 => {
                mask[3] = 0x03;
                SINPUT_MISCSTYLE_2
            }
            0x01 => {
                mask[3] = 0x01;
                SINPUT_MISCSTYLE_1
            }
            _ => {
                mask[3] = 0x00;
                SINPUT_MISCSTYLE_NONE
            }
        };

        let mut version = analog_style;
        version = version * SINPUT_BUMPERSTYLE_MAX + bumper_style;
        version = version * SINPUT_TRIGGERSTYLE_MAX + trigger_style;
        version = version * SINPUT_PADDLESTYLE_MAX + paddle_style;
        version = version * SINPUT_METASTYLE_MAX + meta_style;
        version = version * SINPUT_TOUCHSTYLE_MAX + touch_style;
        version = version * SINPUT_MISCSTYLE_MAX + misc_style;

        // Overwrite our button usage masks
        // with our sanitized masks
        self.usage_masks = mask;

        version.clamp(0, i32::from(u16::MAX)) as u16
    }

    /// Translation of `ProcessSDLFeaturesResponse()`, on the bulk data of
    /// the features command response.
    fn process_sdl_features_response(&mut self, device: &DeviceCtx<'_>, data: &[u8]) {
        // Obtain protocol version
        // (EXTRACTUINT16(data, 0), which isn't used)

        // Bitfields are not portable, so we unpack them into a struct value
        self.rumble_supported = (data[2] & 0x01) != 0;
        self.player_leds_supported = (data[2] & 0x02) != 0;
        self.accelerometer_supported = (data[2] & 0x04) != 0;
        self.gyroscope_supported = (data[2] & 0x08) != 0;

        self.left_analog_stick_supported = (data[2] & 0x10) != 0;
        self.right_analog_stick_supported = (data[2] & 0x20) != 0;
        self.left_analog_trigger_supported = (data[2] & 0x40) != 0;
        self.right_analog_trigger_supported = (data[2] & 0x80) != 0;

        self.touchpad_supported = (data[3] & 0x01) != 0;
        self.joystick_rgb_supported = (data[3] & 0x02) != 0;

        // (is_handheld is data[3] & 0x04)

        // The gamepad type represents a style of gamepad that most closely
        // resembles the gamepad in question (Button style, button layout)
        // Note (upstream): upstream clamps the type to SDL_GAMEPAD_TYPE_COUNT,
        // which isn't a gamepad type; a GamepadType can't be one, so that
        // (and every larger value) is Unknown here.
        let gamepad_type = GamepadType::ALL
            .get(usize::from(data[4]))
            .copied()
            .unwrap_or(GamepadType::Unknown);
        device.set_gamepad_type(gamepad_type);

        // The 3 MSB represent a button layout style SDL_GamepadFaceStyle
        // The 5 LSB represent a device sub-type
        device.set_guid_byte(15, data[5]);

        self.polling_rate_us = extract_u16(data, 6);

        let accel_range = extract_u16(data, 8);
        let gyro_range = extract_u16(data, 10);

        self.usage_masks.copy_from_slice(&data[12..16]);

        // Get and validate touchpad parameters
        self.touchpad_count = data[16];
        self.touchpad_finger_count = data[17];

        // Get device Serial - MAC address
        let serial = format!(
            "{:02x}-{:02x}-{:02x}-{:02x}-{:02x}-{:02x}",
            data[18], data[19], data[20], data[21], data[22], data[23]
        );

        device.set_device_serial(&serial);

        self.accel_scale = calculate_accel_scale(accel_range);
        self.gyro_scale = calculate_gyro_scale(gyro_range);

        let mut axes = 0;
        if self.left_analog_stick_supported {
            axes += 2;
        }

        if self.right_analog_stick_supported {
            axes += 2;
        }

        if self.left_analog_trigger_supported || self.right_analog_trigger_supported {
            // Always add both analog trigger axes if one is present
            axes += 2;
        }

        self.axes_count = axes;

        let version = self.device_dynamic_encoding_setup();
        // Overwrite 'Version' field of the GUID data
        device.set_guid_byte(12, (version & 0xFF) as u8);
        device.set_guid_byte(13, (version >> 8) as u8);

        // Derive button count from mask
        for mask in self.usage_masks {
            self.buttons_count += mask.count_ones() as u8;
        }

        // Convert DPAD to hat
        const DPAD_MASK: u8 = (1 << SINPUT_BUTTON_IDX_DPAD_UP)
            | (1 << SINPUT_BUTTON_IDX_DPAD_DOWN)
            | (1 << SINPUT_BUTTON_IDX_DPAD_LEFT)
            | (1 << SINPUT_BUTTON_IDX_DPAD_RIGHT);
        if (self.usage_masks[0] & DPAD_MASK) == DPAD_MASK {
            self.dpad_supported = true;
            self.usage_masks[0] &= !DPAD_MASK;
            self.buttons_count -= 4;
        }
    }

    /// Translation of `RetrieveSDLFeatures()`.
    fn retrieve_sdl_features(&mut self, device: &DeviceCtx<'_>, dev: &dyn SteamHid) -> Result<()> {
        let features_get_command = command_report(SINPUT_DEVICE_COMMAND_FEATURES, &[]);

        // Attempt to send the SDL features get command.
        let mut written = false;
        for _ in 0..8 {
            // This write will occasionally return -1, so ignore failure here and try again
            if dev.write(&features_get_command).ok() == Some(SINPUT_DEVICE_REPORT_COMMAND_SIZE) {
                written = true;
                break;
            }
        }

        if !written {
            return Err(Error::new(
                "SInput device SDL Features GET command could not write",
            ));
        }

        // Read the reply
        for _ in 0..100 {
            crate::timer::delay(Duration::from_millis(1));

            let mut data = [0u8; USB_PACKET_LENGTH];
            let read = match dev.read_timeout(&mut data, 0) {
                Err(_) => {
                    return Err(Error::new(
                        "SInput device SDL Features GET command could not read",
                    ));
                }
                Ok(0) => continue,
                Ok(read) => read,
            };

            if read == SINPUT_DEVICE_REPORT_SIZE
                && data[0] == SINPUT_DEVICE_REPORT_ID_INPUT_CMDDAT
                && data[SINPUT_REPORT_IDX_COMMAND_RESPONSE_ID] == SINPUT_DEVICE_COMMAND_FEATURES
            {
                self.process_sdl_features_response(
                    device,
                    &data[SINPUT_REPORT_IDX_COMMAND_RESPONSE_BULK..],
                );
                return Ok(());
            }
        }

        // (upstream sets no error)
        Err(Error::new(
            "SInput device SDL Features GET command got no reply",
        ))
    }

    /// `HIDAPI_DriverSInput_InitDevice()` on `dev`.
    fn init(&mut self, device: &mut DeviceCtx<'_>, dev: &dyn SteamHid) -> Result<()> {
        self.retrieve_sdl_features(device, dev)?;

        // (the USB Device Version is stored upstream, because the GUID's
        // is overwritten, but never used)

        if let Some(name) = device_name(device.product_id()) {
            device.set_device_name(name);
        }

        device.joystick_connected();
        Ok(())
    }

    /// The player LED command of `HIDAPI_DriverSInput_SetDevicePlayerIndex()`,
    /// if player LEDs are supported.
    fn player_led_command(
        &self,
        player_index: i32,
    ) -> Option<[u8; SINPUT_DEVICE_REPORT_COMMAND_SIZE]> {
        if !self.player_leds_supported {
            return None;
        }
        let player_num = player_index.saturating_add(1).clamp(0, 255) as u8;

        // Set player number, finalizing the setup
        Some(command_report(
            SINPUT_DEVICE_COMMAND_PLAYERLED,
            &[player_num],
        ))
    }

    /// The command of `HIDAPI_DriverSInput_SetJoystickLED()`, if the
    /// joystick RGB LED is supported.
    fn joystick_rgb_command(
        &self,
        red: u8,
        green: u8,
        blue: u8,
    ) -> Option<[u8; SINPUT_DEVICE_REPORT_COMMAND_SIZE]> {
        self.joystick_rgb_supported
            .then(|| command_report(SINPUT_DEVICE_COMMAND_JOYSTICKRGB, &[red, green, blue]))
    }

    /// Translation of `HIDAPI_DriverSInput_HandleStatePacket()`.
    fn handle_state_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        let timestamp = crate::timer::ticks_ns();
        let mut imu_values = [0.0f32; 3];
        let mut output_idx: u8 = 0;

        // Process digital buttons according to the supplied
        // button mask to create a contiguous button input set
        for (processes, &usage_mask) in self.usage_masks.iter().enumerate() {
            let button_idx = SINPUT_REPORT_IDX_BUTTONS_0 + processes;

            for buttons in 0..8 {
                // If a button is enabled by our usage mask
                let mask = 0x01 << buttons;
                if (usage_mask & mask) != 0 {
                    let down = (data[button_idx] & mask) != 0;

                    if usize::from(output_idx) < GamepadButton::COUNT
                        && self.last_state[button_idx] != data[button_idx]
                    {
                        device.send_button(timestamp, joystick, output_idx, down);
                    }

                    output_idx += 1;
                }
            }
        }

        if self.dpad_supported {
            let buttons = data[SINPUT_REPORT_IDX_BUTTONS_0];
            let mut hat = HAT_CENTERED;

            if buttons & (1 << SINPUT_BUTTON_IDX_DPAD_UP) != 0 {
                hat |= HAT_UP;
            }
            if buttons & (1 << SINPUT_BUTTON_IDX_DPAD_DOWN) != 0 {
                hat |= HAT_DOWN;
            }
            if buttons & (1 << SINPUT_BUTTON_IDX_DPAD_LEFT) != 0 {
                hat |= HAT_LEFT;
            }
            if buttons & (1 << SINPUT_BUTTON_IDX_DPAD_RIGHT) != 0 {
                hat |= HAT_RIGHT;
            }
            device.send_hat(timestamp, joystick, 0, hat);
        }

        // Analog inputs map to a signed Sint16 range of -32768 to 32767 from the device.
        // Use an axis index because not all gamepads will have the same axis inputs.
        let mut axis_idx = 0;
        let mut axis = |device: &mut DeviceCtx<'_>, idx: usize| {
            device.send_axis(timestamp, joystick, axis_idx, extract_i16(data, idx));
            axis_idx += 1;
        };

        // Left Analog Stick
        if self.left_analog_stick_supported {
            axis(device, SINPUT_REPORT_IDX_LEFT_X);
            axis(device, SINPUT_REPORT_IDX_LEFT_Y);
        }

        // Right Analog Stick
        if self.right_analog_stick_supported {
            axis(device, SINPUT_REPORT_IDX_RIGHT_X);
            axis(device, SINPUT_REPORT_IDX_RIGHT_Y);
        }

        // Left Analog Trigger
        if self.left_analog_trigger_supported {
            axis(device, SINPUT_REPORT_IDX_LEFT_TRIGGER);
        }

        // Right Analog Trigger
        if self.right_analog_trigger_supported {
            axis(device, SINPUT_REPORT_IDX_RIGHT_TRIGGER);
        }

        // Battery/Power state handling
        if self.last_state[SINPUT_REPORT_IDX_PLUG_STATUS] != data[SINPUT_REPORT_IDX_PLUG_STATUS]
            || self.last_state[SINPUT_REPORT_IDX_CHARGE_LEVEL]
                != data[SINPUT_REPORT_IDX_CHARGE_LEVEL]
        {
            let status = data[SINPUT_REPORT_IDX_PLUG_STATUS];
            // Ensure percent is within valid range
            let percent = i32::from(data[SINPUT_REPORT_IDX_CHARGE_LEVEL]).clamp(0, 100);

            let (state, percent) = match status {
                1 => (PowerState::NoBattery, 0),
                2 => (PowerState::Charging, percent),
                3 => (PowerState::Charged, 100),
                4 => (PowerState::OnBattery, percent),
                _ => (PowerState::Unknown, percent),
            };

            if state != PowerState::Unknown {
                device.send_power_info(joystick, state, percent);
            }
        }

        // Extract the IMU timestamp delta (in microseconds)
        let imu_timestamp_us = extract_u32(data, SINPUT_REPORT_IDX_IMU_TIMESTAMP);

        // Check if we should process IMU data and if sensors are enabled
        if self.sensors_enabled {
            // (the rollover case of upstream is the wrapping difference)
            let imu_time_delta_us = imu_timestamp_us.wrapping_sub(self.last_imu_timestamp_us);

            // Convert delta to nanoseconds and update running timestamp
            self.imu_timestamp_ns = self
                .imu_timestamp_ns
                .wrapping_add(u64::from(imu_time_delta_us) * 1000);

            // Update last timestamp
            self.last_imu_timestamp_us = imu_timestamp_us;

            // Process Gyroscope
            if self.gyroscope_supported {
                let gyro = |idx| f32::from(extract_i16(data, idx)) * self.gyro_scale;
                imu_values[2] = -gyro(SINPUT_REPORT_IDX_IMU_GYRO_Y); // Y-axis rotation
                imu_values[1] = gyro(SINPUT_REPORT_IDX_IMU_GYRO_Z); // Z-axis rotation
                imu_values[0] = -gyro(SINPUT_REPORT_IDX_IMU_GYRO_X); // X-axis rotation

                device.send_sensor(
                    timestamp,
                    joystick,
                    SensorType::Gyro,
                    self.imu_timestamp_ns,
                    &imu_values,
                );
            }

            // Process Accelerometer
            if self.accelerometer_supported {
                let accel = |idx| f32::from(extract_i16(data, idx)) * self.accel_scale;
                imu_values[2] = -accel(SINPUT_REPORT_IDX_IMU_ACCEL_Y); // Y-axis acceleration
                imu_values[1] = accel(SINPUT_REPORT_IDX_IMU_ACCEL_Z); // Z-axis acceleration
                imu_values[0] = -accel(SINPUT_REPORT_IDX_IMU_ACCEL_X); // X-axis acceleration

                device.send_sensor(
                    timestamp,
                    joystick,
                    SensorType::Accel,
                    self.imu_timestamp_ns,
                    &imu_values,
                );
            }
        }

        // Check if we should process touchpad
        if self.touchpad_supported && self.touchpad_count > 0 {
            let mut touchpad = 0;
            let mut finger = 0;

            let touch = |x, y, p| {
                let pressure = extract_u16(data, p);
                (
                    pressure > 0,
                    f32::from(extract_i16(data, x)) / 65536.0 + 0.5,
                    f32::from(extract_i16(data, y)) / 65536.0 + 0.5,
                    f32::from(pressure) / 32768.0,
                )
            };

            let (down, x, y, pressure) = touch(
                SINPUT_REPORT_IDX_TOUCH1_X,
                SINPUT_REPORT_IDX_TOUCH1_Y,
                SINPUT_REPORT_IDX_TOUCH1_P,
            );
            device.send_touchpad(timestamp, joystick, touchpad, finger, down, x, y, pressure);

            if self.touchpad_count > 1 {
                touchpad += 1;
            } else if self.touchpad_finger_count > 1 {
                finger += 1;
            }

            if touchpad > 0 || finger > 0 {
                let (down, x, y, pressure) = touch(
                    SINPUT_REPORT_IDX_TOUCH2_X,
                    SINPUT_REPORT_IDX_TOUCH2_Y,
                    SINPUT_REPORT_IDX_TOUCH2_P,
                );
                device.send_touchpad(timestamp, joystick, touchpad, finger, down, x, y, pressure);
            }
        }

        let n = size.min(self.last_state.len());
        self.last_state[..n].copy_from_slice(&data[..n]);
    }

    /// One report of `HIDAPI_DriverSInput_UpdateDevice()`.
    fn handle_report(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        if data[0] == SINPUT_DEVICE_REPORT_ID_JOYSTICK_INPUT {
            self.handle_state_packet(device, joystick, data, size);
        }
    }

    /// The touchpads of an opened joystick (part of
    /// `HIDAPI_DriverSInput_OpenJoystick()`): how many, with how many
    /// fingers each.
    fn touchpads(&mut self) -> (u8, u8) {
        if !self.touchpad_supported {
            return (0, 0);
        }

        // If touchpad is supported, minimum 1, max is capped
        self.touchpad_count = self.touchpad_count.clamp(1, SINPUT_MAX_ALLOWED_TOUCHPADS);

        if self.touchpad_count > 1 {
            // Support two separate touchpads with 1 finger each
            // or support one touchpad with 2 fingers max
            self.touchpad_finger_count = 1;
        }

        (self.touchpad_count, self.touchpad_finger_count)
    }
}

impl DriverContext for SInputContext {
    /// Translation of `HIDAPI_DriverSInput_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        let dev = device.device().clone();
        self.init(device, &*dev)
    }

    /// Translation of `HIDAPI_DriverSInput_SetDevicePlayerIndex()`.
    fn set_device_player_index(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _instance_id: JoystickID,
        player_index: i32,
    ) {
        if let Some(command) = self.player_led_command(player_index) {
            if device.write(&command).is_err() {
                // (upstream sets the error "SInput device player led
                // command could not write")
            }
        }
    }

    /// Translation of `HIDAPI_DriverSInput_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
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

                    self.handle_report(device, joystick, &data, size);
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

    /// Translation of `HIDAPI_DriverSInput_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        joystick.nbuttons = usize::from(self.buttons_count);

        self.last_state = [0; USB_PACKET_LENGTH];

        joystick.naxes = usize::from(self.axes_count);

        if self.dpad_supported {
            joystick.nhats = 1;
        }

        let sensor_rate = 1000000.0 / f32::from(self.polling_rate_us);
        if self.gyroscope_supported {
            joystick.add_sensor(SensorType::Gyro, sensor_rate);
        }

        if self.accelerometer_supported {
            joystick.add_sensor(SensorType::Accel, sensor_rate);
        }

        let (touchpad_count, touchpad_finger_count) = self.touchpads();
        for _ in 0..touchpad_count {
            joystick.add_touchpad(usize::from(touchpad_finger_count));
        }

        Ok(())
    }

    /// Translation of `HIDAPI_DriverSInput_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        if self.rumble_supported {
            let haptic_report = rumble_report(low_frequency_rumble, high_frequency_rumble);

            // (whether it is sent isn't checked, as upstream)
            let _ = send_rumble(device.device(), &haptic_report);

            return Ok(());
        }

        Err(Error::unsupported())
    }

    /// Translation of `HIDAPI_DriverSInput_GetJoystickCapabilities()`.
    fn get_joystick_capabilities(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
    ) -> JoystickCaps {
        let mut caps = JoystickCaps(0);
        if self.rumble_supported {
            caps |= JoystickCaps::RUMBLE;
        }

        if self.player_leds_supported {
            caps |= JoystickCaps::PLAYER_LED;
        }

        if self.joystick_rgb_supported {
            caps |= JoystickCaps::RGB_LED;
        }

        caps
    }

    /// Translation of `HIDAPI_DriverSInput_SetJoystickLED()`.
    fn set_joystick_led(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        red: u8,
        green: u8,
        blue: u8,
    ) -> Result<()> {
        if let Some(joystick_rgb_command) = self.joystick_rgb_command(red, green, blue) {
            if device.write(&joystick_rgb_command).is_err() {
                return Err(Error::new(
                    "SInput device joystick rgb command could not write",
                ));
            }

            return Ok(());
        }
        Err(Error::unsupported())
    }

    /// Translation of `HIDAPI_DriverSInput_SetJoystickSensorsEnabled()`.
    fn set_joystick_sensors_enabled(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        enabled: bool,
    ) -> Result<()> {
        if self.accelerometer_supported || self.gyroscope_supported {
            self.sensors_enabled = enabled;
            return Ok(());
        }
        Err(Error::unsupported())
    }

    /// Translation of `HIDAPI_DriverSInput_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {}
}

#[cfg(test)]
mod tests;
