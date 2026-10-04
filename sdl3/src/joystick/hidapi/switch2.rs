// Rust translation of src/joystick/hidapi/SDL_hidapi_switch2.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Nintendo Switch 2 controller driver: the Switch 2 Pro Controller,
//! the Joy-Con 2 and the Switch 2 GameCube controller, over USB.
//!
//! Code and logic contributed by Valve Corporation under the SDL zlib
//! license.
//!
//! The controllers read their input reports through HID, but are set up
//! (the flash reads of the serial number and calibrations, the
//! initialization commands, the player LEDs and the sensor switch) through
//! the bulk endpoints of their second USB interface, which upstream claims
//! through libusb ([`BulkEndpoints`]). Upstream builds this driver only
//! with libusb (`SDL_JOYSTICK_HIDAPI_SWITCH2` needs `HAVE_LIBUSB`).
//!
//! Not translated: the libusb part (`SDL_InitLibUSB()`, the device handle
//! of the libusb HIDAPI backend, `FindBulkEndpoints()` and claiming and
//! releasing the interface), as the libusb backend isn't translated. So
//! there are no bulk endpoints and the driver stays disabled, as upstream
//! built without libusb, leaving the controllers to the other joystick
//! drivers.

use std::sync::Arc;

use super::rumble::lock_rumble;
use super::{
    remap_val, DeviceCtx, DriverContext, DriverImpl, HidapiDevice, HintWatch, JoystickCaps,
    SDL_HIDAPI_DEFAULT, USB_PACKET_LENGTH,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::{
    joystick_player_index_for_id, JoystickData, HAT_DOWN, HAT_LEFT, HAT_RIGHT, HAT_UP,
};
use crate::sensor::SensorType;

/// Whether the libusb backend is available (`HAVE_LIBUSB`): it isn't
/// translated.
const HAVE_LIBUSB: bool = false;

const RUMBLE_INTERVAL: u64 = 12;
const RUMBLE_MAX: u32 = 29000;

// The Switch 2 Pro Controller's extra buttons
const SDL_GAMEPAD_BUTTON_SWITCH2_PRO_SHARE: u8 = 11;
const SDL_GAMEPAD_BUTTON_SWITCH2_PRO_C: u8 = 12;
const SDL_GAMEPAD_BUTTON_SWITCH2_PRO_RIGHT_PADDLE: u8 = 13;
const SDL_GAMEPAD_BUTTON_SWITCH2_PRO_LEFT_PADDLE: u8 = 14;
const SDL_GAMEPAD_NUM_SWITCH2_PRO_BUTTONS: usize = 15;

// The Joy-Con 2's extra buttons
const SDL_GAMEPAD_BUTTON_SWITCH2_JOYCON_SHARE: u8 = 11;
const SDL_GAMEPAD_BUTTON_SWITCH2_JOYCON_C: u8 = 12;
const SDL_GAMEPAD_BUTTON_SWITCH2_JOYCON_RIGHT_PADDLE1: u8 = 13;
const SDL_GAMEPAD_BUTTON_SWITCH2_JOYCON_LEFT_PADDLE1: u8 = 14;
const SDL_GAMEPAD_BUTTON_SWITCH2_JOYCON_RIGHT_PADDLE2: u8 = 15;
const SDL_GAMEPAD_BUTTON_SWITCH2_JOYCON_LEFT_PADDLE2: u8 = 16;
const SDL_GAMEPAD_NUM_SWITCH2_JOYCON_BUTTONS: usize = 17;

// The GameCube controller's buttons after the face buttons
const SDL_GAMEPAD_BUTTON_SWITCH2_GAMECUBE_GUIDE: u8 = 4;
const SDL_GAMEPAD_BUTTON_SWITCH2_GAMECUBE_START: u8 = 5;
const SDL_GAMEPAD_BUTTON_SWITCH2_GAMECUBE_LEFT_SHOULDER: u8 = 6;
const SDL_GAMEPAD_BUTTON_SWITCH2_GAMECUBE_RIGHT_SHOULDER: u8 = 7;
const SDL_GAMEPAD_BUTTON_SWITCH2_GAMECUBE_SHARE: u8 = 8;
const SDL_GAMEPAD_BUTTON_SWITCH2_GAMECUBE_C: u8 = 9;
/// Full trigger pull click
const SDL_GAMEPAD_BUTTON_SWITCH2_GAMECUBE_LEFT_TRIGGER: u8 = 10;
/// Full trigger pull click
const SDL_GAMEPAD_BUTTON_SWITCH2_GAMECUBE_RIGHT_TRIGGER: u8 = 11;
const SDL_GAMEPAD_NUM_SWITCH2_GAMECUBE_BUTTONS: usize = 12;

/// The command that switches the sensors on (`| 4`) or off.
const SET_FEATURES: [u8; 12] = [
    0x0c, 0x91, 0x00, 0x04, 0x00, 0x04, 0x00, 0x00, 0x23, 0x00, 0x00, 0x00,
];

/// Translation of `Switch2_AxisCalibration`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct AxisCalibration {
    neutral: u16,
    max: u16,
    min: u16,
}

/// Translation of `Switch2_StickCalibration`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct StickCalibration {
    x: AxisCalibration,
    y: AxisCalibration,
}

/// The bulk endpoints of the controller's second interface, claimed
/// through libusb (`ctx->libusb` with its `device_handle`,
/// `interface_number`, `out_endpoint` and `in_endpoint`); releasing the
/// interface is dropping them. Nothing provides them yet, as the libusb
/// backend isn't translated (the tests do).
pub(crate) trait BulkEndpoints: Send {
    /// A `bulk_transfer()` to the OUT endpoint, with a 1000 ms timeout:
    /// the number of bytes transferred.
    fn transfer_out(&self, data: &[u8]) -> Result<usize>;
    /// A `bulk_transfer()` from the IN endpoint, with a 100 ms timeout:
    /// the number of bytes transferred.
    fn transfer_in(&self, data: &mut [u8]) -> Result<usize>;
}

/// The bulk endpoints of a device: the `SDL_InitLibUSB()`, libusb device
/// handle, `FindBulkEndpoints()` and claimed interface of
/// `HIDAPI_DriverSwitch2_InitUSB()`. The libusb backend isn't translated,
/// so the HID devices have no libusb device handle.
fn open_bulk_endpoints(_device: &HidapiDevice) -> Result<Box<dyn BulkEndpoints>> {
    Err(Error::new("Couldn't get libusb device handle"))
}

/// Translation of `SDL_DriverSwitch2_Context`.
struct Switch2Context {
    /// The open joystick (`ctx->joystick`)
    joystick: Option<JoystickID>,

    usb: Option<Box<dyn BulkEndpoints>>,

    rumble_timestamp: u64,
    rumble_seq: u32,
    rumble_hi_amp: u16,
    rumble_hi_freq: u16,
    rumble_lo_amp: u16,
    rumble_lo_freq: u16,
    rumble_error: u32,
    rumble_updated: bool,

    left_stick: StickCalibration,
    right_stick: StickCalibration,
    left_trigger_zero: u8,
    right_trigger_zero: u8,

    gyro_bias_x: f32,
    gyro_bias_y: f32,
    gyro_bias_z: f32,
    accel_bias_x: f32,
    accel_bias_y: f32,
    accel_bias_z: f32,
    sensors_enabled: bool,
    sensors_ready: bool,
    sample_count: i32,
    first_sensor_timestamp: u64,
    sensor_ts_coeff: u64,
    gyro_coeff: f32,

    player_lights: bool,
    player_index: i32,

    vertical_mode: bool,
    last_state: [u8; USB_PACKET_LENGTH],

    /// The `SDL_PlayerLEDHintChanged()` callback.
    player_led_hint: Option<HintWatch>,
}

impl Default for Switch2Context {
    fn default() -> Self {
        Switch2Context {
            joystick: None,
            usb: None,
            rumble_timestamp: 0,
            rumble_seq: 0,
            rumble_hi_amp: 0,
            rumble_hi_freq: 0,
            rumble_lo_amp: 0,
            rumble_lo_freq: 0,
            rumble_error: 0,
            rumble_updated: false,
            left_stick: StickCalibration::default(),
            right_stick: StickCalibration::default(),
            left_trigger_zero: 0,
            right_trigger_zero: 0,
            gyro_bias_x: 0.0,
            gyro_bias_y: 0.0,
            gyro_bias_z: 0.0,
            accel_bias_x: 0.0,
            accel_bias_y: 0.0,
            accel_bias_z: 0.0,
            sensors_enabled: false,
            sensors_ready: false,
            sample_count: 0,
            first_sensor_timestamp: 0,
            sensor_ts_coeff: 0,
            gyro_coeff: 0.0,
            player_lights: false,
            player_index: 0,
            vertical_mode: false,
            last_state: [0; USB_PACKET_LENGTH],
            player_led_hint: None,
        }
    }
}

/// Translation of `ParseStickCalibration()`.
fn parse_stick_calibration(data: &[u8]) -> StickCalibration {
    let low = |a: u8, b: u8| u16::from(a) | (u16::from(b & 0x0F) << 8);
    let high = |a: u8, b: u8| u16::from(a >> 4) | (u16::from(b) << 4);
    StickCalibration {
        x: AxisCalibration {
            neutral: low(data[0], data[1]),
            max: low(data[3], data[4]),
            min: low(data[6], data[7]),
        },
        y: AxisCalibration {
            neutral: high(data[1], data[2]),
            max: high(data[4], data[5]),
            min: high(data[7], data[8]),
        },
    }
}

/// The value of an axis (of `MapJoystickAxis()`).
fn map_joystick_axis_value(calib: &AxisCalibration, mut value: f32, invert: bool) -> i16 {
    let mut mapped_value = if calib.neutral != 0 && calib.min != 0 && calib.max != 0 {
        value -= f32::from(calib.neutral);
        if value < 0.0 {
            value /= f32::from(calib.min);
        } else {
            value /= f32::from(calib.max);
        }
        (value * f32::from(i16::MAX)).clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
    } else {
        remap_val(value, 0.0, 4096.0, f32::from(i16::MIN), f32::from(i16::MAX)) as i16
    };
    if invert {
        mapped_value = !mapped_value;
    }
    mapped_value
}

/// Translation of `MapJoystickAxis()`.
fn map_joystick_axis(
    device: &mut DeviceCtx<'_>,
    timestamp: u64,
    joystick: JoystickID,
    axis: GamepadAxis,
    calib: &AxisCalibration,
    value: f32,
    invert: bool,
) {
    let mapped_value = map_joystick_axis_value(calib, value, invert);
    device.send_axis(timestamp, joystick, axis as u8, mapped_value);
}

/// The value of a trigger (of `MapTriggerAxis()`).
fn map_trigger_axis_value(max: u8, value: f32) -> i16 {
    let max = f32::from(max);
    let t = ((value - max) / (232.0 - max)).clamp(0.0, 1.0);
    remap_val(t, 0.0, 1.0, f32::from(i16::MIN), f32::from(i16::MAX)) as i16
}

/// Translation of `MapTriggerAxis()`.
fn map_trigger_axis(
    device: &mut DeviceCtx<'_>,
    timestamp: u64,
    joystick: JoystickID,
    axis: GamepadAxis,
    max: u8,
    value: f32,
) {
    device.send_axis(
        timestamp,
        joystick,
        axis as u8,
        map_trigger_axis_value(max, value),
    );
}

/// The left stick's axis values of a report.
fn left_stick_values(data: &[u8]) -> (f32, f32) {
    (
        f32::from(u16::from(data[11]) | (u16::from(data[12] & 0x0F) << 8)),
        f32::from(u16::from(data[12] >> 4) | (u16::from(data[13]) << 4)),
    )
}

/// The right stick's axis values of a report.
fn right_stick_values(data: &[u8]) -> (f32, f32) {
    (
        f32::from(u16::from(data[14]) | (u16::from(data[15] & 0x0F) << 8)),
        f32::from(u16::from(data[15] >> 4) | (u16::from(data[16]) << 4)),
    )
}

/// The hat of a report's d-pad byte.
fn dpad_hat(data: u8) -> u8 {
    let mut hat = 0;

    if data & 0x01 != 0 {
        hat |= HAT_DOWN;
    }
    if data & 0x02 != 0 {
        hat |= HAT_UP;
    }
    if data & 0x04 != 0 {
        hat |= HAT_RIGHT;
    }
    if data & 0x08 != 0 {
        hat |= HAT_LEFT;
    }
    hat
}

/// A trigger that is either pressed or not.
fn digital_trigger(pressed: bool) -> i16 {
    if pressed {
        32767
    } else {
        -32768
    }
}

/// Translation of `EncodeHDRumble()`.
fn encode_hd_rumble(
    high_freq: u16,
    high_amp: u16,
    low_freq: u16,
    low_amp: u16,
    rumble_data: &mut [u8],
) {
    rumble_data[0] = (high_freq & 0xFF) as u8;
    rumble_data[1] = (((high_amp >> 4) & 0xfc) | ((high_freq >> 8) & 0x03)) as u8;
    rumble_data[2] = ((high_amp >> 12) | (low_freq << 4)) as u8;
    rumble_data[3] = ((low_amp & 0xc0) | ((low_freq >> 4) & 0x3f)) as u8;
    rumble_data[4] = (low_amp >> 8) as u8;
}

/// The Switch 2 driver's static functions.
pub(crate) struct Switch2Driver;

impl DriverImpl for Switch2Driver {
    /// Translation of `HIDAPI_DriverSwitch2_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_SWITCH2]
    }

    /// Translation of `HIDAPI_DriverSwitch2_IsEnabled()`; never enabled
    /// without libusb, where upstream doesn't build the driver.
    fn is_enabled(&self) -> bool {
        HAVE_LIBUSB
            && hints::get_bool(
                hints::JOYSTICK_HIDAPI_SWITCH2,
                hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
            )
    }

    /// Translation of `HIDAPI_DriverSwitch2_IsSupportedDevice()`.
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
        vendor_id == USB_VENDOR_NINTENDO
            && matches!(
                product_id,
                USB_PRODUCT_NINTENDO_SWITCH2_GAMECUBE_CONTROLLER
                    | USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_LEFT
                    | USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_RIGHT
                    | USB_PRODUCT_NINTENDO_SWITCH2_PRO
            )
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(Switch2Context::default())
    }
}

/// Translation of `HIDAPI_DriverSwitch2_InitBluetooth()`.
fn init_bluetooth() -> Result<()> {
    // FIXME: Need to add Bluetooth support
    Err(Error::new(
        "Nintendo Switch2 controllers not supported over Bluetooth",
    ))
}

impl Switch2Context {
    /// The bulk endpoints (`ctx->device_handle`).
    fn usb(&self) -> Result<&dyn BulkEndpoints> {
        self.usb
            .as_deref()
            .ok_or_else(|| Error::new("Couldn't get libusb device handle"))
    }

    /// Translation of `SendBulkData()`.
    fn send_bulk_data(&self, data: &[u8]) -> Result<usize> {
        self.usb()?.transfer_out(data)
    }

    /// Translation of `RecvBulkData()`: read up to `data.len()` bytes, 64
    /// at a time.
    fn recv_bulk_data(&self, data: &mut [u8]) -> Result<usize> {
        let usb = self.usb()?;
        let mut total_transferred = 0;
        let mut size = data.len();
        let mut offset = 0;

        while size > 0 {
            let current_read = size.min(64);
            let transferred = usb.transfer_in(&mut data[offset..offset + current_read])?;
            total_transferred += transferred;
            size -= transferred;
            offset += current_read;
            if transferred < current_read {
                break;
            }
        }

        Ok(total_transferred)
    }

    /// Translation of `UpdateSlotLED()`.
    fn update_slot_led(&self) -> Result<()> {
        let mut set_led_data = [
            0x09, 0x91, 0x00, 0x07, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        let mut reply = [0u8; 8];
        const PLAYER_PATTERN: [u8; 8] = [0x1, 0x3, 0x7, 0xf, 0x9, 0x5, 0xd, 0x6];

        if self.player_lights && self.player_index >= 0 {
            set_led_data[8] = PLAYER_PATTERN[self.player_index as usize % 8];
        }
        if let Err(e) = self.send_bulk_data(&set_led_data) {
            return Err(Error::new(format!("Couldn't set LED data: {e}")));
        }
        if self.recv_bulk_data(&mut reply)? > 0 {
            Ok(())
        } else {
            Err(Error::new("Couldn't read the LED data reply"))
        }
    }

    /// Translation of `ReadFlashBlock()`.
    fn read_flash_block(&self, address: u32) -> Result<[u8; 0x40]> {
        let mut flash_read_command = [
            0x02, 0x91, 0x00, 0x01, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        let mut buffer = [0u8; 0x50];

        flash_read_command[12..16].copy_from_slice(&address.to_le_bytes());

        self.send_bulk_data(&flash_read_command)?;

        self.recv_bulk_data(&mut buffer)?;

        let mut out = [0u8; 0x40];
        out.copy_from_slice(&buffer[0x10..0x50]);
        Ok(out)
    }

    /// Translation of `SDL_PlayerLEDHintChanged()`.
    fn player_led_hint_changed(&mut self, device: &mut DeviceCtx<'_>, hint: Option<&str>) {
        let player_lights = hints::string_to_bool(hint, true);

        if player_lights != self.player_lights {
            self.player_lights = player_lights;

            let _ = self.update_slot_led();
            device.update_device_properties();
        }
    }

    /// The player LED hint's change since the last call, if any (the
    /// hint callback of upstream).
    fn hint_changes(&mut self, device: &mut DeviceCtx<'_>) {
        if let Some(hint) = self.player_led_hint.as_ref().and_then(HintWatch::take) {
            self.player_led_hint_changed(device, hint.as_deref());
        }
    }

    /// Translation of `HIDAPI_DriverSwitch2_InitUSB()` once it has the
    /// bulk endpoints.
    fn init_usb(&mut self, device: &mut DeviceCtx<'_>, usb: Box<dyn BulkEndpoints>) -> Result<()> {
        self.usb = Some(usb);

        const INIT_SEQUENCE: [&[u8]; 10] = [
            // Unknown purpose
            &[0x7, 0x91, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0],
            // Set feature output bit mask
            &[
                0x0c, 0x91, 0x00, 0x02, 0x00, 0x04, 0x00, 0x00, 0x27, 0x00, 0x00, 0x00,
            ],
            // Unknown purpose
            &[0x11, 0x91, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0],
            // Set rumble data?
            &[
                0x0a, 0x91, 0x00, 0x08, 0x00, 0x14, 0x00, 0x00, 0x01, 0xff, 0xff, 0xff, 0xff, 0xff,
                0xff, 0xff, 0xff, 0x35, 0x00, 0x46, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ],
            // Enable feature output bits
            &[
                0x0c, 0x91, 0x00, 0x04, 0x00, 0x04, 0x00, 0x00, 0x27, 0x00, 0x00, 0x00,
            ],
            // Unknown purpose
            &[0x01, 0x91, 0x0, 0xc, 0x0, 0x0, 0x0, 0x0],
            // Enable rumble
            &[0x01, 0x91, 0x0, 0x1, 0x0, 0x0, 0x0, 0x0],
            // Enable grip buttons on charging grip
            &[0x8, 0x91, 0x0, 0x2, 0x0, 0x4, 0x0, 0x0, 0x01, 0x0, 0x0, 0x0],
            // Set report format
            &[
                0x03, 0x91, 0x00, 0x0a, 0x00, 0x04, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00,
            ],
            // Start output
            &[
                0x03, 0x91, 0x00, 0x0d, 0x00, 0x08, 0x00, 0x00, 0x01, 0x00, 0xff, 0xff, 0xff, 0xff,
                0xff, 0xff,
            ],
        ];

        let warn = |what: &str, e: &Error| {
            crate::log::warn!(crate::log::Category::Input, "Couldn't read {}: {}", what, e);
        };
        let le_f32 = |data: &[u8], at: usize| {
            f32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
        };

        match self.read_flash_block(0x13000) {
            Err(e) => warn("serial number", &e),
            Ok(calibration_data) => {
                // (SDL_strlcpy() of at most 16 characters)
                let serial = &calibration_data[2..2 + 0x10];
                let len = serial.iter().position(|&b| b == 0).unwrap_or(serial.len());
                device.set_device_serial(&String::from_utf8_lossy(&serial[..len]));
            }
        }

        match self.read_flash_block(0x13040) {
            Err(e) => warn("factory calibration data", &e),
            Ok(calibration_data) => {
                self.gyro_bias_x = le_f32(&calibration_data, 4);
                self.gyro_bias_y = le_f32(&calibration_data, 8);
                self.gyro_bias_z = le_f32(&calibration_data, 12);
            }
        }

        match self.read_flash_block(0x13080) {
            Err(e) => warn("factory calibration data", &e),
            Ok(calibration_data) => {
                self.left_stick = parse_stick_calibration(&calibration_data[0x28..]);
            }
        }

        match self.read_flash_block(0x130C0) {
            Err(e) => warn("factory calibration data", &e),
            Ok(calibration_data) => {
                self.right_stick = parse_stick_calibration(&calibration_data[0x28..]);
            }
        }

        match self.read_flash_block(0x13100) {
            Err(e) => warn("factory calibration data", &e),
            Ok(calibration_data) => {
                self.accel_bias_x = le_f32(&calibration_data, 12);
                self.accel_bias_y = le_f32(&calibration_data, 16);
                self.accel_bias_z = le_f32(&calibration_data, 20);
            }
        }

        if device.product_id() == USB_PRODUCT_NINTENDO_SWITCH2_GAMECUBE_CONTROLLER {
            match self.read_flash_block(0x13140) {
                Err(e) => warn("factory calibration data", &e),
                Ok(calibration_data) => {
                    self.left_trigger_zero = calibration_data[0];
                    self.right_trigger_zero = calibration_data[1];
                }
            }
        }

        match self.read_flash_block(0x1FC040) {
            Err(e) => warn("user calibration data", &e),
            Ok(calibration_data) => {
                if calibration_data[0] == 0xb2 && calibration_data[1] == 0xa1 {
                    self.left_stick = parse_stick_calibration(&calibration_data[2..]);
                }
            }
        }

        match self.read_flash_block(0x1FC080) {
            Err(e) => warn("user calibration data", &e),
            Ok(calibration_data) => {
                if calibration_data[0] == 0xb2 && calibration_data[1] == 0xa1 {
                    self.right_stick = parse_stick_calibration(&calibration_data[2..]);
                }
            }
        }

        let mut reply = [0u8; 0x40];
        for command in INIT_SEQUENCE {
            let size = usize::from(command[5]) + 8;
            if let Err(e) = self.send_bulk_data(&command[..size]) {
                return Err(Error::new(format!(
                    "Couldn't send initialization data: {e}"
                )));
            }
            let _ = self.recv_bulk_data(&mut reply);
        }

        Ok(())
    }

    /// The end of `HIDAPI_DriverSwitch2_InitDevice()`, once the controller
    /// is set up.
    fn finish_init(&mut self, device: &mut DeviceCtx<'_>) {
        self.sensor_ts_coeff = 10000;
        self.gyro_coeff = 34.8;

        // Sometimes the device handle isn't available during enumeration so we don't get the device name, so set it explicitly
        match device.product_id() {
            USB_PRODUCT_NINTENDO_SWITCH2_GAMECUBE_CONTROLLER => {
                device.set_device_name("Nintendo GameCube Controller");
            }
            USB_PRODUCT_NINTENDO_SWITCH2_PRO => {
                device.set_device_name("Nintendo Switch Pro Controller");
            }
            _ => {}
        }
        device.joystick_connected();
    }

    /// `HIDAPI_DriverSwitch2_OpenJoystick()`, for a device with a parent
    /// (a combined Joy-Con) or not.
    fn open(&mut self, device: &mut DeviceCtx<'_>, joystick: &mut JoystickData, has_parent: bool) {
        self.joystick = Some(joystick.instance_id);

        // Initialize player index (needed for setting LEDs)
        self.player_index = joystick_player_index_for_id(joystick.instance_id);
        self.player_lights = hints::get_bool(hints::JOYSTICK_HIDAPI_SWITCH_PLAYER_LED, true);
        let _ = self.update_slot_led();

        self.player_led_hint = Some(HintWatch::new(hints::JOYSTICK_HIDAPI_SWITCH_PLAYER_LED));
        self.hint_changes(device);

        // Initialize the joystick capabilities
        if !has_parent {
            joystick.add_sensor(SensorType::Gyro, 250.0);
            joystick.add_sensor(SensorType::Accel, 250.0);
        }
        match device.product_id() {
            USB_PRODUCT_NINTENDO_SWITCH2_GAMECUBE_CONTROLLER => {
                joystick.nbuttons = SDL_GAMEPAD_NUM_SWITCH2_GAMECUBE_BUTTONS;
            }
            USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_LEFT => {
                if has_parent {
                    joystick.add_sensor(SensorType::GyroL, 250.0);
                    joystick.add_sensor(SensorType::AccelL, 250.0);
                }
                joystick.nbuttons = SDL_GAMEPAD_NUM_SWITCH2_JOYCON_BUTTONS;
            }
            USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_RIGHT => {
                if has_parent {
                    joystick.add_sensor(SensorType::Gyro, 250.0);
                    joystick.add_sensor(SensorType::Accel, 250.0);
                    joystick.add_sensor(SensorType::GyroR, 250.0);
                    joystick.add_sensor(SensorType::AccelR, 250.0);
                }
                joystick.nbuttons = SDL_GAMEPAD_NUM_SWITCH2_JOYCON_BUTTONS;
            }
            USB_PRODUCT_NINTENDO_SWITCH2_PRO => {
                joystick.nbuttons = SDL_GAMEPAD_NUM_SWITCH2_PRO_BUTTONS;
            }
            _ => {
                // FIXME: How many buttons does this have?
            }
        }
        joystick.naxes = GamepadAxis::COUNT;
        joystick.nhats = 1;

        self.rumble_hi_freq = 0x187;
        self.rumble_lo_freq = 0x112;

        // Set up for vertical mode
        self.vertical_mode = hints::get_bool(hints::JOYSTICK_HIDAPI_VERTICAL_JOY_CONS, false);
    }

    /// `HIDAPI_DriverSwitch2_SetJoystickSensorsEnabled()` on the context.
    fn set_sensors_enabled(&mut self, enabled: bool) -> Result<()> {
        if self.sensors_ready {
            let mut data = SET_FEATURES;
            let mut reply = [0u8; 12];

            if enabled {
                data[8] |= 4;
            }
            if let Err(e) = self.send_bulk_data(&data) {
                return Err(Error::new(format!("Couldn't set sensors enabled: {e}")));
            }
            let _ = self.recv_bulk_data(&mut reply);
        }
        // FIXME (upstream): this should be `enabled`; as it is, disabling
        // the sensors leaves the sensor reports on, and the sensors, once
        // ready, are never switched off by the state handler.
        self.sensors_enabled = true;
        Ok(())
    }

    /// Translation of `HandleGameCubeState()`.
    fn handle_gamecube_state(
        &self,
        device: &mut DeviceCtx<'_>,
        timestamp: u64,
        joystick: JoystickID,
        data: &[u8],
    ) {
        let last = &self.last_state;
        if data[5] != last[5] {
            for (button, mask) in [
                (GamepadButton::West as u8, 0x01),
                (GamepadButton::North as u8, 0x02),
                (GamepadButton::South as u8, 0x04),
                (GamepadButton::East as u8, 0x08),
                (SDL_GAMEPAD_BUTTON_SWITCH2_GAMECUBE_RIGHT_TRIGGER, 0x40),
                (SDL_GAMEPAD_BUTTON_SWITCH2_GAMECUBE_RIGHT_SHOULDER, 0x80),
            ] {
                device.send_button(timestamp, joystick, button, (data[5] & mask) != 0);
            }
        }

        if data[6] != last[6] {
            for (button, mask) in [
                (SDL_GAMEPAD_BUTTON_SWITCH2_GAMECUBE_START, 0x02),
                (SDL_GAMEPAD_BUTTON_SWITCH2_GAMECUBE_GUIDE, 0x10),
                (SDL_GAMEPAD_BUTTON_SWITCH2_GAMECUBE_SHARE, 0x20),
                (SDL_GAMEPAD_BUTTON_SWITCH2_GAMECUBE_C, 0x40),
            ] {
                device.send_button(timestamp, joystick, button, (data[6] & mask) != 0);
            }
        }

        if data[7] != last[7] {
            device.send_hat(timestamp, joystick, 0, dpad_hat(data[7]));

            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH2_GAMECUBE_LEFT_TRIGGER,
                (data[7] & 0x40) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH2_GAMECUBE_LEFT_SHOULDER,
                (data[7] & 0x80) != 0,
            );
        }

        map_trigger_axis(
            device,
            timestamp,
            joystick,
            GamepadAxis::LeftTrigger,
            self.left_trigger_zero,
            f32::from(data[61]),
        );
        map_trigger_axis(
            device,
            timestamp,
            joystick,
            GamepadAxis::RightTrigger,
            self.right_trigger_zero,
            f32::from(data[62]),
        );

        let (lx, ly) = left_stick_values(data);
        let (rx, ry) = right_stick_values(data);
        map_joystick_axis(
            device,
            timestamp,
            joystick,
            GamepadAxis::LeftX,
            &self.left_stick.x,
            lx,
            false,
        );
        map_joystick_axis(
            device,
            timestamp,
            joystick,
            GamepadAxis::LeftY,
            &self.left_stick.y,
            ly,
            true,
        );
        map_joystick_axis(
            device,
            timestamp,
            joystick,
            GamepadAxis::RightX,
            &self.right_stick.x,
            rx,
            false,
        );
        map_joystick_axis(
            device,
            timestamp,
            joystick,
            GamepadAxis::RightY,
            &self.right_stick.y,
            ry,
            true,
        );
    }

    /// Translation of `HandleCombinedControllerStateL()`.
    fn handle_combined_controller_state_l(
        &self,
        device: &mut DeviceCtx<'_>,
        timestamp: u64,
        joystick: JoystickID,
        data: &[u8],
    ) {
        let last = &self.last_state;
        if data[6] != last[6] {
            for (button, mask) in [
                (GamepadButton::Back as u8, 0x01),
                (GamepadButton::LeftStick as u8, 0x08),
                (SDL_GAMEPAD_BUTTON_SWITCH2_JOYCON_SHARE, 0x20),
            ] {
                device.send_button(timestamp, joystick, button, (data[6] & mask) != 0);
            }
        }

        if data[7] != last[7] {
            device.send_hat(timestamp, joystick, 0, dpad_hat(data[7]));

            device.send_button(
                timestamp,
                joystick,
                GamepadButton::LeftShoulder as u8,
                (data[7] & 0x40) != 0,
            );
        }

        if data[8] != last[8] {
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH2_JOYCON_LEFT_PADDLE1,
                (data[8] & 0x02) != 0,
            );
        }

        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftTrigger as u8,
            digital_trigger(data[7] & 0x80 != 0),
        );

        let (lx, ly) = left_stick_values(data);
        map_joystick_axis(
            device,
            timestamp,
            joystick,
            GamepadAxis::LeftX,
            &self.left_stick.x,
            lx,
            false,
        );
        map_joystick_axis(
            device,
            timestamp,
            joystick,
            GamepadAxis::LeftY,
            &self.left_stick.y,
            ly,
            true,
        );
    }

    /// Translation of `HandleMiniControllerStateL()`.
    fn handle_mini_controller_state_l(
        &self,
        device: &mut DeviceCtx<'_>,
        timestamp: u64,
        joystick: JoystickID,
        data: &[u8],
    ) {
        let last = &self.last_state;
        if data[6] != last[6] {
            for (button, mask) in [
                (GamepadButton::Start as u8, 0x01),
                (GamepadButton::LeftStick as u8, 0x08),
                (GamepadButton::Guide as u8, 0x20),
                (SDL_GAMEPAD_BUTTON_SWITCH2_JOYCON_SHARE, 0x10),
            ] {
                device.send_button(timestamp, joystick, button, (data[6] & mask) != 0);
            }
        }

        if data[7] != last[7] {
            for (button, mask) in [
                (GamepadButton::West as u8, 0x01),
                (GamepadButton::North as u8, 0x02),
                (GamepadButton::South as u8, 0x04),
                (GamepadButton::East as u8, 0x08),
                (GamepadButton::RightShoulder as u8, 0x10),
                (GamepadButton::LeftShoulder as u8, 0x20),
                (SDL_GAMEPAD_BUTTON_SWITCH2_JOYCON_LEFT_PADDLE1, 0x40),
                (SDL_GAMEPAD_BUTTON_SWITCH2_JOYCON_LEFT_PADDLE2, 0x80),
            ] {
                device.send_button(timestamp, joystick, button, (data[7] & mask) != 0);
            }
        }

        let (lx, ly) = left_stick_values(data);
        map_joystick_axis(
            device,
            timestamp,
            joystick,
            GamepadAxis::LeftX,
            &self.left_stick.y,
            ly,
            true,
        );
        map_joystick_axis(
            device,
            timestamp,
            joystick,
            GamepadAxis::LeftY,
            &self.left_stick.x,
            lx,
            true,
        );
    }

    /// Translation of `HandleCombinedControllerStateR()`.
    fn handle_combined_controller_state_r(
        &self,
        device: &mut DeviceCtx<'_>,
        timestamp: u64,
        joystick: JoystickID,
        data: &[u8],
    ) {
        let last = &self.last_state;
        if data[5] != last[5] {
            for (button, mask) in [
                (GamepadButton::West as u8, 0x01),
                (GamepadButton::North as u8, 0x02),
                (GamepadButton::South as u8, 0x04),
                (GamepadButton::East as u8, 0x08),
                (GamepadButton::RightShoulder as u8, 0x40),
            ] {
                device.send_button(timestamp, joystick, button, (data[5] & mask) != 0);
            }
        }

        if data[6] != last[6] {
            for (button, mask) in [
                (GamepadButton::Start as u8, 0x02),
                (GamepadButton::RightStick as u8, 0x04),
                (GamepadButton::Guide as u8, 0x10),
                (SDL_GAMEPAD_BUTTON_SWITCH2_JOYCON_C, 0x40),
            ] {
                device.send_button(timestamp, joystick, button, (data[6] & mask) != 0);
            }
        }

        if data[8] != last[8] {
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH2_JOYCON_RIGHT_PADDLE1,
                (data[8] & 0x01) != 0,
            );
        }

        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightTrigger as u8,
            digital_trigger(data[5] & 0x80 != 0),
        );

        // (the Joy-Con's stick calibration is the first one)
        let (rx, ry) = right_stick_values(data);
        map_joystick_axis(
            device,
            timestamp,
            joystick,
            GamepadAxis::RightX,
            &self.left_stick.x,
            rx,
            false,
        );
        map_joystick_axis(
            device,
            timestamp,
            joystick,
            GamepadAxis::RightY,
            &self.left_stick.y,
            ry,
            true,
        );
    }

    /// Translation of `HandleMiniControllerStateR()`.
    fn handle_mini_controller_state_r(
        &self,
        device: &mut DeviceCtx<'_>,
        timestamp: u64,
        joystick: JoystickID,
        data: &[u8],
    ) {
        let last = &self.last_state;
        if data[5] != last[5] {
            for (button, mask) in [
                (GamepadButton::West as u8, 0x01),
                (GamepadButton::North as u8, 0x02),
                (GamepadButton::South as u8, 0x04),
                (GamepadButton::East as u8, 0x08),
                (GamepadButton::RightShoulder as u8, 0x10),
                (GamepadButton::LeftShoulder as u8, 0x20),
                (SDL_GAMEPAD_BUTTON_SWITCH2_JOYCON_RIGHT_PADDLE1, 0x40),
                (SDL_GAMEPAD_BUTTON_SWITCH2_JOYCON_RIGHT_PADDLE2, 0x80),
            ] {
                device.send_button(timestamp, joystick, button, (data[5] & mask) != 0);
            }
        }

        if data[6] != last[6] {
            for (button, mask) in [
                (GamepadButton::Start as u8, 0x02),
                (GamepadButton::LeftStick as u8, 0x04),
                (GamepadButton::Guide as u8, 0x10),
                (SDL_GAMEPAD_BUTTON_SWITCH2_JOYCON_C, 0x40),
            ] {
                device.send_button(timestamp, joystick, button, (data[6] & mask) != 0);
            }
        }

        let (rx, ry) = right_stick_values(data);
        map_joystick_axis(
            device,
            timestamp,
            joystick,
            GamepadAxis::LeftX,
            &self.left_stick.y,
            ry,
            false,
        );
        map_joystick_axis(
            device,
            timestamp,
            joystick,
            GamepadAxis::LeftY,
            &self.left_stick.x,
            rx,
            false,
        );
    }

    /// Translation of `HandleSwitchProState()`.
    fn handle_switch_pro_state(
        &self,
        device: &mut DeviceCtx<'_>,
        timestamp: u64,
        joystick: JoystickID,
        data: &[u8],
    ) {
        let last = &self.last_state;
        if data[5] != last[5] {
            for (button, mask) in [
                (GamepadButton::West as u8, 0x01),
                (GamepadButton::North as u8, 0x02),
                (GamepadButton::South as u8, 0x04),
                (GamepadButton::East as u8, 0x08),
                (GamepadButton::RightShoulder as u8, 0x40),
            ] {
                device.send_button(timestamp, joystick, button, (data[5] & mask) != 0);
            }
        }

        if data[6] != last[6] {
            for (button, mask) in [
                (GamepadButton::Back as u8, 0x01),
                (GamepadButton::Start as u8, 0x02),
                (GamepadButton::RightStick as u8, 0x04),
                (GamepadButton::LeftStick as u8, 0x08),
                (GamepadButton::Guide as u8, 0x10),
                (SDL_GAMEPAD_BUTTON_SWITCH2_PRO_SHARE, 0x20),
                (SDL_GAMEPAD_BUTTON_SWITCH2_PRO_C, 0x40),
            ] {
                device.send_button(timestamp, joystick, button, (data[6] & mask) != 0);
            }
        }

        if data[7] != last[7] {
            device.send_hat(timestamp, joystick, 0, dpad_hat(data[7]));

            device.send_button(
                timestamp,
                joystick,
                GamepadButton::LeftShoulder as u8,
                (data[7] & 0x40) != 0,
            );
        }

        if data[8] != last[8] {
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH2_PRO_RIGHT_PADDLE,
                (data[8] & 0x01) != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_SWITCH2_PRO_LEFT_PADDLE,
                (data[8] & 0x02) != 0,
            );
        }

        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightTrigger as u8,
            digital_trigger(data[5] & 0x80 != 0),
        );

        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftTrigger as u8,
            digital_trigger(data[7] & 0x80 != 0),
        );

        let (lx, ly) = left_stick_values(data);
        let (rx, ry) = right_stick_values(data);
        map_joystick_axis(
            device,
            timestamp,
            joystick,
            GamepadAxis::LeftX,
            &self.left_stick.x,
            lx,
            false,
        );
        map_joystick_axis(
            device,
            timestamp,
            joystick,
            GamepadAxis::LeftY,
            &self.left_stick.y,
            ly,
            true,
        );
        map_joystick_axis(
            device,
            timestamp,
            joystick,
            GamepadAxis::RightX,
            &self.right_stick.x,
            rx,
            false,
        );
        map_joystick_axis(
            device,
            timestamp,
            joystick,
            GamepadAxis::RightY,
            &self.right_stick.y,
            ry,
            true,
        );
    }

    /// The early returns of `UpdateRumble()`: whether a rumble report is
    /// due at `timestamp` (`SDL_GetTicks()`).
    fn rumble_due(&self, timestamp: u64) -> bool {
        if !self.rumble_updated && self.rumble_lo_amp == 0 && self.rumble_hi_amp == 0 {
            return false;
        }

        timestamp >= self.rumble_timestamp
    }

    /// The rumble report of `UpdateRumble()`, which moves the rumble state
    /// on to the next report.
    fn rumble_report(&mut self, product_id: u16, has_parent: bool, timestamp: u64) -> [u8; 64] {
        let mut interval = RUMBLE_INTERVAL;
        let mut rumble_data = [0u8; 64];

        if product_id == USB_PRODUCT_NINTENDO_SWITCH2_GAMECUBE_CONTROLLER {
            let rumble_max = self.rumble_lo_amp.max(self.rumble_hi_amp);
            rumble_data[0x00] = 0x3;
            rumble_data[1] = 0x50 | (self.rumble_seq & 0xf) as u8;
            if rumble_max == 0 {
                rumble_data[2] = 2;
                self.rumble_error = 0;
            } else if self.rumble_error < u32::from(rumble_max) {
                rumble_data[2] = 1;
                self.rumble_error += u32::from(u16::MAX - rumble_max);
            } else {
                rumble_data[2] = 0;
                self.rumble_error -= u32::from(rumble_max);
            }
        } else {
            // Rumble can get so strong that it might be dangerous to the controller...
            // This is a game controller, not a massage device, so let's clamp it somewhat
            let clamp = |amp: u16| (u32::from(amp) * RUMBLE_MAX / u32::from(u16::MAX)) as u16;
            let low_amp = clamp(self.rumble_lo_amp);
            let high_amp = clamp(self.rumble_hi_amp);
            rumble_data[0x01] = 0x50 | (self.rumble_seq & 0xf) as u8;
            encode_hd_rumble(
                self.rumble_hi_freq,
                high_amp,
                self.rumble_lo_freq,
                low_amp,
                &mut rumble_data[0x02..0x07],
            );
            match product_id {
                USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_LEFT
                | USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_RIGHT => {
                    if has_parent {
                        // FIXME: This shouldn't be necessary, but the rumble thread appears to back up if we don't do this
                        interval *= 2;
                    }
                    rumble_data[0] = 0x1;
                }
                USB_PRODUCT_NINTENDO_SWITCH2_PRO => {
                    rumble_data[0] = 0x2;
                    rumble_data.copy_within(0x01..0x07, 0x11);
                }
                _ => {}
            }
        }
        self.rumble_seq = self.rumble_seq.wrapping_add(1);
        self.rumble_updated = false;
        if self.rumble_lo_amp == 0 && self.rumble_hi_amp == 0 {
            self.rumble_timestamp = 0;
        } else {
            if self.rumble_timestamp == 0 {
                self.rumble_timestamp = timestamp;
            }
            self.rumble_timestamp += interval;
        }
        rumble_data
    }

    /// Translation of `UpdateRumble()`.
    fn update_rumble(&mut self, device: &Arc<HidapiDevice>) -> Result<()> {
        let timestamp = crate::timer::ticks_ms();
        if !self.rumble_due(timestamp) {
            return Ok(());
        }

        let lock = lock_rumble()?;

        let rumble_data =
            self.rumble_report(device.product_id(), device.parent().is_some(), timestamp);

        if lock.send_and_unlock(device, &rumble_data)? != rumble_data.len() {
            return Err(Error::new("Couldn't send rumble packet"));
        }
        Ok(())
    }

    /// Translation of `HIDAPI_DriverSwitch2_HandleStatePacket()`, for a
    /// device with a parent (a combined Joy-Con) or not.
    fn handle_state_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        has_parent: bool,
        data: &[u8],
    ) {
        let timestamp = crate::timer::ticks_ns();
        let product_id = device.product_id();

        if data.len() < 64 {
            // We don't know how to handle this report
            return;
        }

        match product_id {
            USB_PRODUCT_NINTENDO_SWITCH2_GAMECUBE_CONTROLLER => {
                self.handle_gamecube_state(device, timestamp, joystick, data);
            }
            USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_LEFT => {
                if has_parent || self.vertical_mode {
                    self.handle_combined_controller_state_l(device, timestamp, joystick, data);
                } else {
                    self.handle_mini_controller_state_l(device, timestamp, joystick, data);
                }
            }
            USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_RIGHT => {
                if has_parent || self.vertical_mode {
                    self.handle_combined_controller_state_r(device, timestamp, joystick, data);
                } else {
                    self.handle_mini_controller_state_r(device, timestamp, joystick, data);
                }
            }
            USB_PRODUCT_NINTENDO_SWITCH2_PRO => {
                self.handle_switch_pro_state(device, timestamp, joystick, data);
            }
            _ => {
                // FIXME: Need state handling implementation
            }
        }

        let mut sensor_timestamp = u64::from(u32::from_le_bytes([
            data[0x2b], data[0x2c], data[0x2d], data[0x2e],
        ]));
        if sensor_timestamp != 0 && !self.sensors_ready {
            self.sample_count += 1;
            if self.sample_count >= 5 && self.first_sensor_timestamp == 0 {
                self.first_sensor_timestamp = sensor_timestamp;
                self.sample_count = 0;
            } else if self.sample_count == 100 {
                // Calculate timestamp coefficient
                // Timestamp are normally microseconds but sometimes it's something else for no apparent reason
                let coeff = 1000u64
                    .wrapping_mul(sensor_timestamp.wrapping_sub(self.first_sensor_timestamp))
                    / (self.sample_count as u64 * 4);
                if (coeff + 100000) / 200000 == 5 {
                    // Within 10% of 1000
                    self.sensor_ts_coeff = 10000;
                    self.gyro_coeff = 34.8;
                    self.sensors_ready = true;
                } else if let Some(sensor_ts_coeff) = 10000000000u64.checked_div(coeff) {
                    self.sensor_ts_coeff = sensor_ts_coeff;
                    self.gyro_coeff = 40.0;
                    self.sensors_ready = true;
                } else {
                    // Didn't get a valid reading, try again
                    self.first_sensor_timestamp = 0;
                    self.sample_count = 0;
                }

                if self.sensors_ready && !self.sensors_enabled {
                    let mut reply = [0u8; 12];

                    let _ = self.send_bulk_data(&SET_FEATURES);
                    let _ = self.recv_bulk_data(&mut reply);
                }
            }
        }
        if self.sensors_enabled && sensor_timestamp != 0 && self.sensors_ready {
            sensor_timestamp = sensor_timestamp.wrapping_mul(self.sensor_ts_coeff) / 10;
            const G: f32 = 9.80665;
            let accel_scale = G * 8.0 / f32::from(i16::MAX);
            let le = |at: usize| f32::from(i16::from_le_bytes([data[at], data[at + 1]]));
            let int16_max = f32::from(i16::MAX);

            let mut accel_data = [
                le(0x31) * accel_scale,
                le(0x35) * accel_scale,
                le(0x33) * -accel_scale,
            ];

            let mut gyro_data = [
                le(0x37) * self.gyro_coeff / int16_max - self.gyro_bias_x,
                le(0x3b) * self.gyro_coeff / int16_max - self.gyro_bias_z,
                le(0x39) * -self.gyro_coeff / int16_max + self.gyro_bias_y,
            ];

            let mut send = |sensor_type: SensorType, values: &[f32; 3]| {
                device.send_sensor(timestamp, joystick, sensor_type, sensor_timestamp, values);
            };
            match product_id {
                USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_LEFT => {
                    if has_parent {
                        send(SensorType::GyroL, &gyro_data);
                        send(SensorType::AccelL, &accel_data);
                    } else {
                        let tmp = -accel_data[0];
                        accel_data[0] = accel_data[2];
                        accel_data[2] = tmp;

                        let tmp = -gyro_data[0];
                        gyro_data[0] = gyro_data[2];
                        gyro_data[2] = tmp;

                        send(SensorType::Gyro, &gyro_data);
                        send(SensorType::Accel, &accel_data);
                    }
                }
                USB_PRODUCT_NINTENDO_SWITCH2_JOYCON_RIGHT => {
                    if has_parent {
                        send(SensorType::Gyro, &gyro_data);
                        send(SensorType::Accel, &accel_data);
                        send(SensorType::GyroR, &gyro_data);
                        send(SensorType::AccelR, &accel_data);
                    } else {
                        let tmp = accel_data[0];
                        accel_data[0] = -accel_data[2];
                        accel_data[2] = tmp;

                        let tmp = gyro_data[0];
                        gyro_data[0] = -gyro_data[2];
                        gyro_data[2] = tmp;

                        send(SensorType::Gyro, &gyro_data);
                        send(SensorType::Accel, &accel_data);
                    }
                }
                _ => {
                    send(SensorType::Gyro, &gyro_data);
                    send(SensorType::Accel, &accel_data);
                }
            }
        }

        let n = data.len().min(self.last_state.len());
        self.last_state[..n].copy_from_slice(&data[..n]);
    }
}

impl DriverContext for Switch2Context {
    /// Translation of `HIDAPI_DriverSwitch2_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        if device.is_bluetooth() {
            init_bluetooth()?;
        } else {
            let usb = open_bulk_endpoints(device)?;
            self.init_usb(device, usb)?;
        }
        self.finish_init(device);
        Ok(())
    }

    /// Translation of `HIDAPI_DriverSwitch2_SetDevicePlayerIndex()`.
    fn set_device_player_index(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _instance_id: JoystickID,
        player_index: i32,
    ) {
        if self.joystick.is_none() {
            return;
        }

        self.player_index = player_index;

        let _ = self.update_slot_led();
    }

    /// Translation of `HIDAPI_DriverSwitch2_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        let dev = device.device().clone();
        let Some(&first) = device.joysticks().first() else {
            return false;
        };
        let joystick = device.open_joystick_id();

        // (the hint callback of upstream)
        if joystick.is_some() && self.joystick.is_some() {
            self.hint_changes(device);
        }

        let mut data = [0u8; USB_PACKET_LENGTH];
        // (upstream's last `size`: a read error is negative)
        let ok = loop {
            let size = match device.read_timeout(&mut data, 0) {
                Ok(0) => break true,
                Err(_) => break false,
                Ok(size) => size,
            };
            // (DEBUG_SWITCH2_PROTOCOL would dump the packet here)
            let Some(joystick) = joystick else {
                continue;
            };

            let has_parent = device.parent().is_some();
            self.handle_state_packet(device, joystick, has_parent, &data[..size.min(data.len())]);

            let _ = self.update_rumble(&dev);
        };

        if !ok {
            // Read error, device is disconnected
            device.joystick_disconnected(first);
        }
        ok
    }

    /// Translation of `HIDAPI_DriverSwitch2_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        let has_parent = device.parent().is_some();
        self.open(device, joystick, has_parent);
        Ok(())
    }

    /// Translation of `HIDAPI_DriverSwitch2_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        if low_frequency_rumble != self.rumble_lo_amp || high_frequency_rumble != self.rumble_hi_amp
        {
            self.rumble_lo_amp = low_frequency_rumble;
            self.rumble_hi_amp = high_frequency_rumble;
            self.rumble_updated = true;
        }

        Ok(())
    }

    /// Translation of `HIDAPI_DriverSwitch2_GetJoystickCapabilities()`.
    fn get_joystick_capabilities(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
    ) -> JoystickCaps {
        let mut result = JoystickCaps::RUMBLE;

        if self.player_lights {
            result |= JoystickCaps::PLAYER_LED;
        }
        result
    }

    /// Translation of `HIDAPI_DriverSwitch2_SetJoystickSensorsEnabled()`.
    fn set_joystick_sensors_enabled(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        enabled: bool,
    ) -> Result<()> {
        self.set_sensors_enabled(enabled)
    }

    /// Translation of `HIDAPI_DriverSwitch2_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {
        self.player_led_hint = None;

        self.joystick = None;
    }

    /// Translation of `HIDAPI_DriverSwitch2_FreeDevice()`: dropping the
    /// bulk endpoints releases the interface.
    fn free_device(&mut self, _device: &mut DeviceCtx<'_>) {
        self.usb = None;
    }
}

#[cfg(test)]
mod tests;
