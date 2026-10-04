// Rust translation of src/joystick/hidapi/SDL_hidapi_steam.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Steam Controller driver: the original Steam Controller, wired,
//! through its wireless dongle or over Bluetooth LE. The protocol's
//! constants and wire structures (`steam/controller_constants.h` and
//! `steam/controller_structs.h`), shared with the Steam Deck and Triton
//! drivers, are in [`controller_constants`] and [`controller_structs`].
//!
//! Not translated: the mouse mode upstream compiles out
//! (`ENABLE_MOUSE_MODE`) and the protocol debug logging
//! (`DEBUG_STEAM_PROTOCOL`, `DEBUG_STEAM_CONTROLLER`).

pub(crate) mod controller_constants;
pub(crate) mod controller_structs;
#[cfg(test)]
pub(super) mod tests;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use super::{
    dump_packet, DeviceCtx, DriverContext, DriverImpl, HidapiDevice, HintWatch, SDL_HIDAPI_DEFAULT,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::{
    is_joystick_steam_controller, JoystickConnectionState, JoystickData, HAT_DOWN, HAT_LEFT,
    HAT_RIGHT, HAT_UP,
};
use crate::sensor::{SensorType, STANDARD_GRAVITY};
use controller_constants::*;
use controller_structs::*;

/// `SDL_HINT_JOYSTICK_HIDAPI_STEAM_PAIRING_ENABLED`
const HINT_JOYSTICK_HIDAPI_STEAM_PAIRING_ENABLED: &str =
    "SDL_JOYSTICK_HIDAPI_STEAM_PAIRING_ENABLED";

/// `SDL_HINT_JOYSTICK_HIDAPI_STEAM_DEFAULT`
fn hint_joystick_hidapi_steam_default() -> bool {
    if cfg!(any(
        target_os = "android",
        target_os = "ios",
        target_os = "tvos"
    )) {
        // This requires prompting for Bluetooth permissions, so make sure the application really wants it
        false
    } else {
        hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT)
    }
}

const PAIRING_STATE_DURATION_SECONDS: u8 = 60;

/// `SDL_GAMEPAD_BUTTON_STEAM_RIGHT_PADDLE`
const SDL_GAMEPAD_BUTTON_STEAM_RIGHT_PADDLE: u8 = 11;
/// `SDL_GAMEPAD_BUTTON_STEAM_LEFT_PADDLE`
const SDL_GAMEPAD_BUTTON_STEAM_LEFT_PADDLE: u8 = 12;
/// `SDL_GAMEPAD_NUM_STEAM_BUTTONS`
const SDL_GAMEPAD_NUM_STEAM_BUTTONS: usize = 13;

/// The state of a controller (`SteamControllerStateInternal_t`).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
struct SteamControllerStateInternal {
    /// Controller Type for this Controller State
    controller_type: u32,

    /// If packet num matches that on your prior call, then the controller state hasn't been changed since
    /// your last call and there is no need to process it
    packet_num: u32,

    /// bit flags for each of the buttons
    buttons: u64,

    left_pad_x: i16,
    left_pad_y: i16,

    right_pad_x: i16,
    right_pad_y: i16,

    left_stick_x: i16,
    left_stick_y: i16,

    trigger_l: u16,
    trigger_r: u16,

    accel_x: i16,
    accel_y: i16,
    accel_z: i16,

    gyro_x: i16,
    gyro_y: i16,
    gyro_z: i16,

    gyro_quat_w: f32,
    gyro_quat_x: f32,
    gyro_quat_y: f32,
    gyro_quat_z: f32,

    gyro_steering_angle: i16,

    battery_level: u16,

    // Pressure sensor data.
    pressure_pad_left: u16,
    pressure_pad_right: u16,

    pressure_bumper_left: u16,
    pressure_bumper_right: u16,

    // Internal state data
    prev_left_pad: [i16; 2],
    prev_left_stick: [i16; 2],
}

// Defines for buttons in SteamControllerStateInternal
#[allow(dead_code)] // (as upstream)
const STEAM_RIGHT_TRIGGER_MASK: u64 = 0x00000001;
#[allow(dead_code)] // (as upstream)
const STEAM_LEFT_TRIGGER_MASK: u64 = 0x00000002;
const STEAM_RIGHT_BUMPER_MASK: u64 = 0x00000004;
const STEAM_LEFT_BUMPER_MASK: u64 = 0x00000008;
const STEAM_BUTTON_NORTH_MASK: u64 = 0x00000010; // Y
const STEAM_BUTTON_EAST_MASK: u64 = 0x00000020; // B
const STEAM_BUTTON_WEST_MASK: u64 = 0x00000040; // X
const STEAM_BUTTON_SOUTH_MASK: u64 = 0x00000080; // A
const STEAM_DPAD_UP_MASK: u64 = 0x00000100; // DPAD UP
const STEAM_DPAD_RIGHT_MASK: u64 = 0x00000200; // DPAD RIGHT
const STEAM_DPAD_LEFT_MASK: u64 = 0x00000400; // DPAD LEFT
const STEAM_DPAD_DOWN_MASK: u64 = 0x00000800; // DPAD DOWN
const STEAM_BUTTON_MENU_MASK: u64 = 0x00001000; // SELECT
const STEAM_BUTTON_STEAM_MASK: u64 = 0x00002000; // GUIDE
const STEAM_BUTTON_ESCAPE_MASK: u64 = 0x00004000; // START
const STEAM_BUTTON_BACK_LEFT_MASK: u64 = 0x00008000;
const STEAM_BUTTON_BACK_RIGHT_MASK: u64 = 0x00010000;
const STEAM_BUTTON_LEFTPAD_CLICKED_MASK: u64 = 0x00020000;
const STEAM_BUTTON_RIGHTPAD_CLICKED_MASK: u64 = 0x00040000;
const STEAM_LEFTPAD_FINGERDOWN_MASK: u64 = 0x00080000;
const STEAM_RIGHTPAD_FINGERDOWN_MASK: u64 = 0x00100000;
const STEAM_JOYSTICK_BUTTON_MASK: u64 = 0x00400000;
const STEAM_LEFTPAD_AND_JOYSTICK_MASK: u64 = 0x00800000;

// Look for report version 0x0001, type WIRELESS (3), length >= 1 byte
/// `D0G_IS_VALID_WIRELESS_EVENT()`
fn d0g_is_valid_wireless_event(data: &[u8]) -> bool {
    data.len() >= 5 && data[0] == 1 && data[1] == 0 && data[2] == 3 && data[3] >= 1
}
/// `D0G_GET_WIRELESS_EVENT_TYPE()`
fn d0g_get_wireless_event_type(data: &[u8]) -> u8 {
    data[4]
}
const D0G_WIRELESS_DISCONNECTED: u8 = 1;

/// `D0G_IS_WIRELESS_DISCONNECT()`
fn d0g_is_wireless_disconnect(data: &[u8]) -> bool {
    d0g_is_valid_wireless_event(data)
        && d0g_get_wireless_event_type(data) == D0G_WIRELESS_DISCONNECTED
}
/// `D0G_IS_WIRELESS_CONNECT()`
fn d0g_is_wireless_connect(data: &[u8]) -> bool {
    d0g_is_valid_wireless_event(data)
        && d0g_get_wireless_event_type(data) != D0G_WIRELESS_DISCONNECTED
}

const MAX_REPORT_SEGMENT_PAYLOAD_SIZE: usize = 18;
const MAX_REPORT_SEGMENT_SIZE: usize = MAX_REPORT_SEGMENT_PAYLOAD_SIZE + 2;
const REPORT_SEGMENT_DATA_FLAG: u8 = 0x80;
const REPORT_SEGMENT_LAST_FLAG: u8 = 0x40;
const BLE_REPORT_NUMBER: u8 = 0x03;

const STEAMCONTROLLER_TRIGGER_MAX_ANALOG: f32 = 26000.0;

// Wireless firmware quirk: the firmware intentionally signals "failure" when performing
// SET_FEATURE / GET_FEATURE when it actually means "pending radio roundtrip". The only
// way to make SET_FEATURE / GET_FEATURE work is to loop several times with a sleep. If
// it takes more than 50ms to get the response for SET_FEATURE / GET_FEATURE, we assume
// that the controller has failed.
const RADIO_WORKAROUND_SLEEP_ATTEMPTS: usize = 50;
const RADIO_WORKAROUND_SLEEP_DURATION_US: u64 = 500;

/// The size of a feature report of the Steam Controller: the firmware
/// always wants a 65-byte buffer (the report number and a 64-byte
/// message).
pub(crate) const FEATURE_REPORT_BUFFER_SIZE: usize = HID_FEATURE_REPORT_BYTES + 1;

/// What the Valve drivers do with their HID device (the `SDL_hid_*()`
/// calls on `device->dev`): the HIDAPI device, or a test's fake one.
pub(crate) trait SteamHid {
    /// `device->is_bluetooth`
    fn is_bluetooth(&self) -> bool;
    /// `SDL_hid_send_feature_report()`
    fn send_feature_report(&self, data: &[u8]) -> Result<usize>;
    /// `SDL_hid_get_feature_report()`
    fn get_feature_report(&self, data: &mut [u8]) -> Result<usize>;
    /// `SDL_hid_read()` (the HIDAPI devices are non-blocking)
    fn read(&self, data: &mut [u8]) -> Result<usize>;
    /// `SDL_hid_read_timeout()`
    fn read_timeout(&self, data: &mut [u8], milliseconds: i32) -> Result<usize>;
    /// `SDL_hid_write()`
    fn write(&self, data: &[u8]) -> Result<usize>;
}

/// The open HID device of a device (`device->dev`).
fn hid_device(device: &HidapiDevice) -> Result<std::sync::Arc<crate::hidapi::HidDevice>> {
    device.dev().ok_or_else(|| Error::invalid_param("device"))
}

impl SteamHid for HidapiDevice {
    fn is_bluetooth(&self) -> bool {
        HidapiDevice::is_bluetooth(self)
    }
    fn send_feature_report(&self, data: &[u8]) -> Result<usize> {
        hid_device(self)?.send_feature_report(data)
    }
    fn get_feature_report(&self, data: &mut [u8]) -> Result<usize> {
        hid_device(self)?.get_feature_report(data)
    }
    fn read(&self, data: &mut [u8]) -> Result<usize> {
        hid_device(self)?.read(data)
    }
    fn read_timeout(&self, data: &mut [u8], milliseconds: i32) -> Result<usize> {
        hid_device(self)?.read_timeout_ms(data, milliseconds)
    }
    fn write(&self, data: &[u8]) -> Result<usize> {
        hid_device(self)?.write(data)
    }
}

/// Translation of `GetSegmentHeader()`.
fn get_segment_header(segment_number: usize, last_packet: bool) -> u8 {
    let mut header = REPORT_SEGMENT_DATA_FLAG;
    header |= segment_number as u8;
    if last_packet {
        header |= REPORT_SEGMENT_LAST_FLAG;
    }

    header
}

/// Translation of `hexdump()`.
fn hexdump(data: &[u8]) {
    dump_packet("Data", data);
}

/// Reassembles the segmented reports of BLE controllers.
/// `SteamControllerPacketAssembler` has to be used when reading output
/// repots from controllers.
#[derive(Clone, Debug)]
struct PacketAssembler {
    buffer: [u8; MAX_REPORT_SEGMENT_PAYLOAD_SIZE * 8 + 1],
    expected_segment_number: usize,
    is_ble: bool,
}

impl Default for PacketAssembler {
    fn default() -> Self {
        PacketAssembler {
            buffer: [0; MAX_REPORT_SEGMENT_PAYLOAD_SIZE * 8 + 1],
            expected_segment_number: 0,
            is_ble: false,
        }
    }
}

impl PacketAssembler {
    /// Translation of `InitializeSteamControllerPacketAssembler()`.
    fn new(is_ble: bool) -> PacketAssembler {
        PacketAssembler {
            is_ble,
            ..PacketAssembler::default()
        }
    }

    /// Translation of `ResetSteamControllerPacketAssembler()`.
    fn reset(&mut self) {
        self.buffer.fill(0);
        self.expected_segment_number = 0;
    }

    /// Add a segment of `segment_length` bytes at the start of `segment`
    /// (which has room for at least 2); the size of the packet in
    /// [`PacketAssembler::buffer`] once it's complete, `None` while it isn't
    /// or on a bad segment. Translation of
    /// `WriteSegmentToSteamControllerPacketAssembler()` (whose errors and
    /// incomplete packets callers treat alike).
    fn write_segment(&mut self, segment: &[u8], segment_length: usize) -> Option<usize> {
        if self.is_ble {
            let segment_header = segment[1];
            let segment_number = usize::from(segment_header & 0x07);

            if segment[0] != BLE_REPORT_NUMBER {
                // We may get keyboard/mouse input events until controller stops sending them
                return None;
            }

            if segment_length != MAX_REPORT_SEGMENT_SIZE {
                crate::log!("Bad segment size! {}", segment_length);
                hexdump(&segment[..segment_length]);
                self.reset();
                return None;
            }

            if segment_header & REPORT_SEGMENT_DATA_FLAG == 0 {
                // We get empty segments, just ignore them
                return None;
            }

            if segment_number != self.expected_segment_number {
                self.reset();

                if segment_number != 0 {
                    // This happens occasionally
                    return None;
                }
            }

            let start = segment_number * MAX_REPORT_SEGMENT_PAYLOAD_SIZE;
            self.buffer[start..start + MAX_REPORT_SEGMENT_PAYLOAD_SIZE].copy_from_slice(
                &segment[2..2 + MAX_REPORT_SEGMENT_PAYLOAD_SIZE], // ignore header and report number
            );

            if segment_header & REPORT_SEGMENT_LAST_FLAG != 0 {
                self.expected_segment_number = 0;
                return Some((segment_number + 1) * MAX_REPORT_SEGMENT_PAYLOAD_SIZE);
            }

            self.expected_segment_number += 1;
        } else {
            // Just pass through
            self.buffer[..segment_length].copy_from_slice(&segment[..segment_length]);
            return Some(segment_length);
        }

        None
    }
}

const BLE_MAX_READ_RETRIES: usize = 8;

/// The segments a feature report is sent in over BLE: the report number,
/// a segment header and up to [`MAX_REPORT_SEGMENT_PAYLOAD_SIZE`] bytes of
/// the report past its own report number (the BLE part of
/// `SetFeatureReport()`).
fn ble_feature_report_segments(report: &[u8]) -> Vec<[u8; MAX_REPORT_SEGMENT_SIZE]> {
    // Skip report number in data
    let data = &report[1..];
    let count = data.len().div_ceil(MAX_REPORT_SEGMENT_PAYLOAD_SIZE);
    data.chunks(MAX_REPORT_SEGMENT_PAYLOAD_SIZE)
        .enumerate()
        .map(|(segment_number, bytes)| {
            // Construct packet
            let mut packet = [0; MAX_REPORT_SEGMENT_SIZE];
            packet[0] = BLE_REPORT_NUMBER;
            packet[1] = get_segment_header(segment_number, segment_number + 1 == count);
            packet[2..2 + bytes.len()].copy_from_slice(bytes);
            packet
        })
        .collect()
}

/// Send the first `actual_data_len` bytes of a feature report (`report`,
/// [`FEATURE_REPORT_BUFFER_SIZE`] bytes). Translation of
/// `SetFeatureReport()`.
fn set_feature_report(dev: &dyn SteamHid, report: &[u8], actual_data_len: usize) -> Result<usize> {
    let mut result = Err(Error::new("Couldn't send feature report"));

    if dev.is_bluetooth() {
        if actual_data_len < 1 {
            return result;
        }

        for packet in ble_feature_report_segments(&report[..actual_data_len]) {
            result = dev.send_feature_report(&packet);
        }
    } else {
        for _ in 0..RADIO_WORKAROUND_SLEEP_ATTEMPTS {
            result = dev.send_feature_report(&report[..FEATURE_REPORT_BUFFER_SIZE]);
            if result.is_ok() {
                break;
            }

            crate::timer::delay(Duration::from_micros(RADIO_WORKAROUND_SLEEP_DURATION_US));
        }
    }

    result
}

/// Whether BLE devices get 2 copies of the feature report ID, one that is
/// removed by ReadFeatureReport, and one that's included in the buffer we
/// receive (on Windows and macOS).
const BLE_DUPLICATE_FEATURE_REPORT_ID: bool = cfg!(any(windows, target_os = "macos"));

/// Read a feature report into `report` ([`FEATURE_REPORT_BUFFER_SIZE`]
/// bytes). Translation of `GetFeatureReport()`.
fn get_feature_report(dev: &dyn SteamHid, report: &mut [u8]) -> Result<usize> {
    get_feature_report_with(dev, report, BLE_DUPLICATE_FEATURE_REPORT_ID)
}

/// [`get_feature_report`], with whether BLE reports come with a second
/// copy of their report ID.
fn get_feature_report_with(
    dev: &dyn SteamHid,
    report: &mut [u8],
    duplicate_report_id: bool,
) -> Result<usize> {
    if dev.is_bluetooth() {
        let mut retries = 0;
        let mut segment_buffer = [0u8; MAX_REPORT_SEGMENT_SIZE + 1];
        let mut bytes_to_read = MAX_REPORT_SEGMENT_SIZE;
        let mut data_start_offset = 0;

        let mut assembler = PacketAssembler::new(dev.is_bluetooth());

        // On Windows and macOS, BLE devices get 2 copies of the feature report ID, one that is removed by ReadFeatureReport,
        // and one that's included in the buffer we receive. We pad the bytes to read and skip over the report ID
        // if necessary.
        if duplicate_report_id {
            bytes_to_read += 1;
            data_start_offset += 1;
        }

        while retries < BLE_MAX_READ_RETRIES {
            segment_buffer.fill(0);
            segment_buffer[0] = BLE_REPORT_NUMBER;
            let result = dev.get_feature_report(&mut segment_buffer[..bytes_to_read]);

            // Zero retry counter if we got data
            if matches!(result, Ok(n) if n > 2)
                && segment_buffer[data_start_offset + 1] & REPORT_SEGMENT_DATA_FLAG != 0
            {
                retries = 0;
            } else {
                retries += 1;
            }

            if let Ok(n) = result {
                if n > 0 {
                    let packet_length = assembler.write_segment(
                        &segment_buffer[data_start_offset..],
                        n.saturating_sub(data_start_offset),
                    );

                    if let Some(packet_length) = packet_length.filter(|&n| n > 0 && n < 65) {
                        // Leave space for "report number"
                        report[0] = 0;
                        report[1..1 + packet_length]
                            .copy_from_slice(&assembler.buffer[..packet_length]);
                        return Ok(packet_length);
                    }
                }
            }
        }
        crate::log!("Could not get a full ble packet after {} retries", retries);
        Err(Error::new("Could not get a full ble packet"))
    } else {
        report[..FEATURE_REPORT_BUFFER_SIZE].fill(0);

        let mut result = Err(Error::new("Couldn't get feature report"));
        for _ in 0..RADIO_WORKAROUND_SLEEP_ATTEMPTS {
            result = dev.get_feature_report(&mut report[..FEATURE_REPORT_BUFFER_SIZE]);
            if result.is_ok() {
                break;
            }

            crate::timer::delay(Duration::from_micros(RADIO_WORKAROUND_SLEEP_DURATION_US));
        }

        result
    }
}

/// Translation of `ReadResponse()`.
fn read_response(dev: &dyn SteamHid, report: &mut [u8], expected_response: u8) -> Result<usize> {
    for _ in 0..10 {
        let Ok(size) = get_feature_report(dev, report) else {
            continue;
        };

        if report[1] != expected_response {
            continue;
        }

        return Ok(size);
    }
    Err(Error::new("Couldn't read response"))
}

/// A feature report message: the report number (0) and the message type
/// in a [`FEATURE_REPORT_BUFFER_SIZE`] buffer, with the length to send.
fn feature_report(msg_type: u8, payload: &[u8]) -> ([u8; FEATURE_REPORT_BUFFER_SIZE], usize) {
    let mut buf = [0; FEATURE_REPORT_BUFFER_SIZE];
    buf[1] = msg_type;
    buf[2..2 + payload.len()].copy_from_slice(payload);
    (buf, 2 + payload.len())
}

/// A settings message (`ID_SET_SETTINGS_VALUES` and the `ADD_SETTING()`s).
fn settings_report(settings: &[(u8, u16)]) -> ([u8; FEATURE_REPORT_BUFFER_SIZE], usize) {
    let mut buf = [0; FEATURE_REPORT_BUFFER_SIZE];
    write_set_settings_values(&mut buf[1..], ID_SET_SETTINGS_VALUES, settings);
    (buf, 3 + settings.len() * CONTROLLER_SETTING_SIZE)
}

/// Send a message of [`feature_report`] or [`settings_report`].
fn send_message(
    dev: &dyn SteamHid,
    message: ([u8; FEATURE_REPORT_BUFFER_SIZE], usize),
) -> Result<usize> {
    set_feature_report(dev, &message.0, message.1)
}

/// Reset steam controller (unmap buttons and pads) and re-fetch capability
/// bits; returns the update rate in microseconds. Translation of
/// `ResetSteamController()`.
fn reset_steam_controller(dev: &dyn SteamHid, suppress_error_spew: bool) -> Result<u32> {
    let mut update_rate_us = 9000; // Good default rate
    let failed = |what: &'static str| {
        if !suppress_error_spew {
            crate::log!("{} for controller {:p}", what, dev);
        }
        Err(Error::new(what))
    };

    // Firmware quirk: Set Feature and Get Feature requests always require a 65-byte buffer.
    if send_message(dev, feature_report(ID_GET_ATTRIBUTES_VALUES, &[])).is_err() {
        return failed("GET_ATTRIBUTES_VALUES failed");
    }

    // Retrieve GET_ATTRIBUTES_VALUES result
    // Wireless controller endpoints without a connected controller will return nAttrs == 0
    let mut buf = [0u8; FEATURE_REPORT_BUFFER_SIZE];
    let res = match read_response(dev, &mut buf, ID_GET_ATTRIBUTES_VALUES) {
        Ok(res) if buf[1] == ID_GET_ATTRIBUTES_VALUES => res,
        _ => return failed("Bad GET_ATTRIBUTES_VALUES response"),
    };

    let attributes_length = usize::from(buf[2]);
    if attributes_length > res {
        return failed("Bad GET_ATTRIBUTES_VALUES response");
    }

    // (msg = (FeatureReportMsg *)&buf[1])
    // Note (upstream): upstream reads the attributes the header's length
    // covers, the last of which can extend past the 65-byte buffer; only
    // the ones within it are read here.
    let count = attributes_length / CONTROLLER_ATTRIBUTE_SIZE;
    for attribute in buf[3..]
        .chunks_exact(CONTROLLER_ATTRIBUTE_SIZE)
        .take(count)
        .map(ControllerAttribute::parse)
    {
        match attribute.attribute_tag {
            ATTRIB_UNIQUE_ID | ATTRIB_PRODUCT_ID | ATTRIB_CAPABILITIES => {}
            ATTRIB_CONNECTION_INTERVAL_IN_US => update_rate_us = attribute.attribute_value,
            _ => {}
        }
    }

    // Clear digital button mappings
    if send_message(dev, feature_report(ID_CLEAR_DIGITAL_MAPPINGS, &[])).is_err() {
        return failed("CLEAR_DIGITAL_MAPPINGS failed");
    }

    // Reset the default settings
    if send_message(dev, feature_report(ID_LOAD_DEFAULT_SETTINGS, &[0])).is_err() {
        return failed("LOAD_DEFAULT_SETTINGS failed");
    }

    // Apply custom settings - clear trackpad modes (cancel mouse emulation), etc
    // (without ENABLE_MOUSE_MODE)
    let settings = settings_report(&[
        (SETTING_WIRELESS_PACKET_VERSION, 2),
        (SETTING_LEFT_TRACKPAD_MODE, TRACKPAD_NONE),
        (SETTING_RIGHT_TRACKPAD_MODE, TRACKPAD_NONE),
        (SETTING_SMOOTH_ABSOLUTE_MOUSE, 0),
    ]);
    if send_message(dev, settings).is_err() {
        return failed("SET_SETTINGS failed");
    }

    Ok(update_rate_us)
}

/// Read from a Steam Controller. Translation of `ReadSteamController()`.
fn read_steam_controller(dev: &dyn SteamHid, data: &mut [u8]) -> Result<usize> {
    data.fill(0);
    data[0] = BLE_REPORT_NUMBER; // hid_read will also overwrite this with the same value, 0x03
    dev.read(data)
}

/// Set Steam Controller pairing state. Translation of `SetPairingState()`.
fn set_pairing_state(dev: &dyn SteamHid, enable_pairing: bool) {
    let _ = send_message(
        dev,
        feature_report(
            ID_ENABLE_PAIRING,
            &[
                2, // 2 payload bytes: bool + timeout
                u8::from(enable_pairing),
                if enable_pairing {
                    PAIRING_STATE_DURATION_SECONDS
                } else {
                    0
                },
            ],
        ),
    );
}

/// Commit Steam Controller pairing. Translation of `CommitPairing()`.
fn commit_pairing(dev: &dyn SteamHid) {
    let _ = send_message(dev, feature_report(ID_DONGLE_COMMIT_DEVICE, &[]));
}

/// Close a Steam Controller. Translation of `CloseSteamController()`.
fn close_steam_controller(dev: &dyn SteamHid) {
    // Switch the Steam Controller back to lizard mode so it works with the OS

    // Reset digital button mappings
    let _ = send_message(dev, feature_report(ID_SET_DEFAULT_DIGITAL_MAPPINGS, &[]));

    // Reset the default settings
    let _ = send_message(dev, feature_report(ID_LOAD_DEFAULT_SETTINGS, &[0]));

    // Reset mouse mode for lizard mode
    let _ = send_message(
        dev,
        settings_report(&[(SETTING_RIGHT_TRACKPAD_MODE, TRACKPAD_ABSOLUTE_MOUSE)]),
    );
}

/// Scale and clamp values to a range. Translation of `RemapValClamped()`.
pub(crate) fn remap_val_clamped(val: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    if a == b {
        if (val - b) >= 0.0 {
            d
        } else {
            c
        }
    } else {
        let mut c_val = (val - a) / (b - a);
        c_val = c_val.clamp(0.0, 1.0);

        c + (d - c) * c_val
    }
}

/// Rotate the pad coordinates. Translation of `RotatePad()`.
fn rotate_pad(x: &mut i32, y: &mut i32, angle_in_rad: f32) {
    use crate::stdlib::math::{cosf, sinf};
    let (orig_x, orig_y) = (*x as f32, *y as f32);

    *x = (cosf(angle_in_rad) * orig_x - sinf(angle_in_rad) * orig_y) as i32;
    *y = (sinf(angle_in_rad) * orig_x + cosf(angle_in_rad) * orig_y) as i32;
}

/// `clamp(val, SDL_MIN_SINT16, SDL_MAX_SINT16)`
fn clamp_i16(val: i32) -> i16 {
    val.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

/// The rotated and offset coordinates of a pad.
fn rotated_pad(x: i16, y: i16, angle_in_rad: f32, finger_down: bool) -> (i16, i16) {
    let (mut x, mut y) = (i32::from(x), i32::from(y));
    rotate_pad(&mut x, &mut y, angle_in_rad);
    let pad_offset = if finger_down { 1000 } else { 0 };
    (clamp_i16(x + pad_offset), clamp_i16(y + pad_offset))
}

/// The analog value of a trigger byte.
fn trigger_value(trigger: u8) -> u16 {
    let trigger = i32::from(trigger);
    remap_val_clamped(
        ((trigger << 7) | trigger) as f32,
        0.0,
        STEAMCONTROLLER_TRIGGER_MAX_ANALOG,
        0.0,
        f32::from(i16::MAX),
    ) as u16
}

/// Reads the chunks of a BLE state packet in turn (`*pData++`).
struct Cursor<'a>(&'a [u8]);

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> &'a [u8] {
        let data = self.0;
        let (chunk, rest) = data.split_at(n);
        self.0 = rest;
        chunk
    }
}

// 15 degrees in rad
const ROTATION_ANGLE: f32 = 0.261799;

impl SteamControllerStateInternal {
    /// Format the first part of the state packet. Translation of
    /// `FormatStatePacketUntilGyro()`.
    fn format_state_packet_until_gyro(&mut self, state_packet: &ValveControllerStatePacket) {
        // (the memset up to sBatteryLevel)
        *self = SteamControllerStateInternal {
            battery_level: self.battery_level,
            pressure_pad_left: self.pressure_pad_left,
            pressure_pad_right: self.pressure_pad_right,
            pressure_bumper_left: self.pressure_bumper_left,
            pressure_bumper_right: self.pressure_bumper_right,
            prev_left_pad: self.prev_left_pad,
            prev_left_stick: self.prev_left_stick,
            ..SteamControllerStateInternal::default()
        };

        // self.controller_type = m_eControllerType;
        self.controller_type = 2; // k_eControllerType_SteamController;
        self.packet_num = state_packet.packet_num;

        // We have a chunk of trigger data in the packet format here, so zero it out afterwards
        self.buttons = state_packet.buttons;
        self.buttons &= !0xFFFF000000;

        // The firmware uses this bit to tell us what kind of data is packed into the left two axes
        if state_packet.buttons & STEAM_LEFTPAD_FINGERDOWN_MASK != 0 {
            // Finger-down bit not set; "left pad" is actually trackpad
            self.left_pad_x = state_packet.left_pad_x;
            self.prev_left_pad[0] = state_packet.left_pad_x;
            self.left_pad_y = state_packet.left_pad_y;
            self.prev_left_pad[1] = state_packet.left_pad_y;

            if state_packet.buttons & STEAM_LEFTPAD_AND_JOYSTICK_MASK != 0 {
                // The controller is interleaving both stick and pad data, both are active
                self.left_stick_x = self.prev_left_stick[0];
                self.left_stick_y = self.prev_left_stick[1];
            } else {
                // The stick is not active
                self.prev_left_stick = [0, 0];
            }
        } else {
            // Finger-down bit not set; "left pad" is actually joystick

            // XXX there's a firmware bug where sometimes padX is 0 and padY is a large number (actually the battery voltage)
            // If that happens skip this packet and report last frames stick
            /*
                    if ( m_eControllerType == k_eControllerType_SteamControllerV2 && pStatePacket->sLeftPadY > 900 ) {
                        pState->sLeftStickX = pState->sPrevLeftStick[0];
                        pState->sLeftStickY = pState->sPrevLeftStick[1];
                    } else
            */
            {
                self.left_stick_x = state_packet.left_pad_x;
                self.prev_left_stick[0] = state_packet.left_pad_x;
                self.left_stick_y = state_packet.left_pad_y;
                self.prev_left_stick[1] = state_packet.left_pad_y;
            }
            /*
                    if (m_eControllerType == k_eControllerType_SteamControllerV2) {
                        UpdateV2JoystickCap(&state);
                    }
            */

            if state_packet.buttons & STEAM_LEFTPAD_AND_JOYSTICK_MASK != 0 {
                // The controller is interleaving both stick and pad data, both are active
                self.left_pad_x = self.prev_left_pad[0];
                self.left_pad_y = self.prev_left_pad[1];
            } else {
                // The trackpad is not active
                self.prev_left_pad = [0, 0];

                // Old controllers send trackpad click for joystick button when trackpad is not active
                if self.buttons & STEAM_BUTTON_LEFTPAD_CLICKED_MASK != 0 {
                    self.buttons &= !STEAM_BUTTON_LEFTPAD_CLICKED_MASK;
                    self.buttons |= STEAM_JOYSTICK_BUTTON_MASK;
                }
            }
        }

        // Fingerdown bit indicates if the packed left axis data was joystick or pad,
        // but if we are interleaving both, the left finger is definitely on the pad.
        if state_packet.buttons & STEAM_LEFTPAD_AND_JOYSTICK_MASK != 0 {
            self.buttons |= STEAM_LEFTPAD_FINGERDOWN_MASK;
        }

        (self.left_pad_x, self.left_pad_y) = rotated_pad(
            self.left_pad_x,
            self.left_pad_y,
            -ROTATION_ANGLE,
            self.buttons & STEAM_LEFTPAD_FINGERDOWN_MASK != 0,
        );
        (self.right_pad_x, self.right_pad_y) = rotated_pad(
            state_packet.right_pad_x,
            state_packet.right_pad_y,
            ROTATION_ANGLE,
            self.buttons & STEAM_RIGHTPAD_FINGERDOWN_MASK != 0,
        );

        self.trigger_l = trigger_value(state_packet.trigger_left());
        self.trigger_r = trigger_value(state_packet.trigger_right());
    }

    /// Update Steam Controller state from a BLE data packet, returns true
    /// if it parsed data. Translation of `UpdateBLESteamControllerState()`.
    fn update_ble(&mut self, data: &[u8]) -> bool {
        let mut data = Cursor(data);
        let mut take = |n: usize| data.take(n);
        let i16_of = |b: &[u8], i: usize| i16::from_le_bytes([b[i * 2], b[i * 2 + 1]]);

        self.packet_num = self.packet_num.wrapping_add(1);
        let header = take(2);
        let option_data_mask = u32::from(header[0] & 0xF0) | (u32::from(header[1]) << 8);
        if option_data_mask & K_EBLE_BUTTON_CHUNK1 != 0 {
            let mut buttons = self.buttons.to_le_bytes();
            buttons[..3].copy_from_slice(take(3));
            self.buttons = u64::from_le_bytes(buttons);
        }
        if option_data_mask & K_EBLE_BUTTON_CHUNK2 != 0 {
            // The middle 2 bytes of the button bits over the wire are triggers when over the wire and non-SC buttons in the internal controller state packet
            let triggers = take(2);
            self.trigger_l = trigger_value(triggers[0]);
            self.trigger_r = trigger_value(triggers[1]);
        }
        if option_data_mask & K_EBLE_BUTTON_CHUNK3 != 0 {
            let mut buttons = self.buttons.to_le_bytes();
            buttons[5..8].copy_from_slice(take(3));
            self.buttons = u64::from_le_bytes(buttons);
        }
        if option_data_mask & K_EBLE_LEFT_JOYSTICK_CHUNK != 0 {
            // This doesn't handle any of the special headcrab stuff for raw joystick which is OK for now since that FW doesn't support
            // this protocol yet either
            let stick = take(4);
            self.left_stick_x = i16_of(stick, 0);
            self.left_stick_y = i16_of(stick, 1);
        }
        if option_data_mask & K_EBLE_LEFT_TRACKPAD_CHUNK != 0 {
            let pad = take(4);
            (self.left_pad_x, self.left_pad_y) = rotated_pad(
                i16_of(pad, 0),
                i16_of(pad, 1),
                -ROTATION_ANGLE,
                self.buttons & STEAM_LEFTPAD_FINGERDOWN_MASK != 0,
            );
        }
        if option_data_mask & K_EBLE_RIGHT_TRACKPAD_CHUNK != 0 {
            let pad = take(4);
            (self.right_pad_x, self.right_pad_y) = rotated_pad(
                i16_of(pad, 0),
                i16_of(pad, 1),
                ROTATION_ANGLE,
                self.buttons & STEAM_RIGHTPAD_FINGERDOWN_MASK != 0,
            );
        }
        if option_data_mask & K_EBLE_IMU_ACCEL_CHUNK != 0 {
            let accel = take(6);
            self.accel_x = i16_of(accel, 0);
            self.accel_y = i16_of(accel, 1);
            self.accel_z = i16_of(accel, 2);
        }
        if option_data_mask & K_EBLE_IMU_GYRO_CHUNK != 0 {
            let gyro = take(6);
            self.gyro_x = i16_of(gyro, 0);
            self.gyro_y = i16_of(gyro, 1);
            self.gyro_z = i16_of(gyro, 2);
        }
        if option_data_mask & K_EBLE_IMU_QUAT_CHUNK != 0 {
            // (the bytes of 4 floats)
            let quat = take(16);
            let f32_of = |i: usize| {
                f32::from_le_bytes([
                    quat[i * 4],
                    quat[i * 4 + 1],
                    quat[i * 4 + 2],
                    quat[i * 4 + 3],
                ])
            };
            self.gyro_quat_w = f32_of(0);
            self.gyro_quat_x = f32_of(1);
            self.gyro_quat_y = f32_of(2);
            self.gyro_quat_z = f32_of(3);
        }
        true
    }

    /// Update Steam Controller state from a data packet, returns true if it
    /// parsed data. Translation of `UpdateSteamControllerState()`.
    fn update(&mut self, data: &[u8]) -> bool {
        let header = ValveInReportHeader::parse(data);

        if header.report_version != K_VALVE_IN_REPORT_MSG_VERSION {
            if (data[0] & 0x0F) == K_EBLE_REPORT_STATE {
                return self.update_ble(data);
            }
            return false;
        }

        if header.report_type != ID_CONTROLLER_STATE
            && header.report_type != ID_CONTROLLER_BLE_STATE
        {
            return false;
        }

        let payload = &data[VALVE_IN_REPORT_HEADER_SIZE..];
        let state_packet = ValveControllerStatePacket::parse(payload);

        // No new data to process; indicate that we received a state packet, but otherwise do nothing.
        if self.packet_num == state_packet.packet_num {
            return true;
        }

        self.format_state_packet_until_gyro(&state_packet);

        if header.report_type == ID_CONTROLLER_STATE {
            self.accel_x = state_packet.accel_x;
            self.accel_y = state_packet.accel_y;
            self.accel_z = state_packet.accel_z;

            self.gyro_quat_w = f32::from(state_packet.gyro_quat_w);
            self.gyro_quat_x = f32::from(state_packet.gyro_quat_x);
            self.gyro_quat_y = f32::from(state_packet.gyro_quat_y);
            self.gyro_quat_z = f32::from(state_packet.gyro_quat_z);

            self.gyro_x = state_packet.gyro_x;
            self.gyro_y = state_packet.gyro_y;
            self.gyro_z = state_packet.gyro_z;
        } else {
            let ble_state_packet = ValveControllerBleStatePacket::parse(payload);

            match ble_state_packet.gyro_data_type {
                1 => {
                    self.gyro_quat_w = f32::from(ble_state_packet.gyro[0]);
                    self.gyro_quat_x = f32::from(ble_state_packet.gyro[1]);
                    self.gyro_quat_y = f32::from(ble_state_packet.gyro[2]);
                    self.gyro_quat_z = f32::from(ble_state_packet.gyro[3]);
                }
                2 => {
                    self.accel_x = ble_state_packet.gyro[0];
                    self.accel_y = ble_state_packet.gyro[1];
                    self.accel_z = ble_state_packet.gyro[2];
                }
                3 => {
                    self.gyro_x = ble_state_packet.gyro[0];
                    self.gyro_y = ble_state_packet.gyro[1];
                    self.gyro_z = ble_state_packet.gyro[2];
                }
                _ => {}
            }
        }

        true
    }
}

/// The context of the dongle in pairing mode (`s_PairingContext`); 0 when
/// none is. Only have one dongle in pairing mode at a time.
static PAIRING_CONTEXT: AtomicU64 = AtomicU64::new(0);
/// The identities of the contexts.
static NEXT_CONTEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Translation of `SDL_DriverSteam_Context`.
#[derive(Debug)]
pub(crate) struct SteamContext {
    /// The context's identity, for [`PAIRING_CONTEXT`]
    id: u64,
    connected: bool,
    report_sensors: bool,
    update_rate_in_us: u32,
    sensor_timestamp: u64,
    pairing_time: u64,

    left_touch_down: bool,
    left_touch_x: f32,
    left_touch_y: f32,
    right_touch_down: bool,
    right_touch_x: f32,
    right_touch_y: f32,

    assembler: PacketAssembler,
    state: SteamControllerStateInternal,
    last_state: SteamControllerStateInternal,

    /// The `SDL_PairingEnabledHintChanged()` callback
    pairing_hint: Option<HintWatch>,
    /// The `SDL_HomeLEDHintChanged()` callback
    home_led_hint: Option<HintWatch>,
}

impl Default for SteamContext {
    fn default() -> Self {
        SteamContext {
            id: NEXT_CONTEXT_ID.fetch_add(1, Ordering::Relaxed),
            connected: false,
            report_sensors: false,
            update_rate_in_us: 0,
            sensor_timestamp: 0,
            pairing_time: 0,
            left_touch_down: false,
            left_touch_x: 0.0,
            left_touch_y: 0.0,
            right_touch_down: false,
            right_touch_x: 0.0,
            right_touch_y: 0.0,
            assembler: PacketAssembler::default(),
            state: SteamControllerStateInternal::default(),
            last_state: SteamControllerStateInternal::default(),
            pairing_hint: None,
            home_led_hint: None,
        }
    }
}

/// Translation of `IsDongle()`.
fn is_dongle(product_id: u16) -> bool {
    product_id == USB_PRODUCT_VALVE_STEAM_CONTROLLER_DONGLE
}

/// The Steam Controller driver's static functions.
pub(crate) struct SteamDriver;

impl DriverImpl for SteamDriver {
    /// Translation of `HIDAPI_DriverSteam_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_STEAM]
    }

    /// Translation of `HIDAPI_DriverSteam_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_STEAM,
            hint_joystick_hidapi_steam_default(),
        )
    }

    /// Translation of `HIDAPI_DriverSteam_IsSupportedDevice()`.
    fn is_supported_device(
        &self,
        device: Option<&HidapiDevice>,
        _name: &str,
        _gamepad_type: GamepadType,
        vendor_id: u16,
        product_id: u16,
        _version: u16,
        interface_number: i32,
        _interface_class: i32,
        _interface_subclass: i32,
        _interface_protocol: i32,
    ) -> bool {
        if !is_joystick_steam_controller(vendor_id, product_id) {
            return false;
        }

        let Some(device) = device else {
            // Might be supported by this driver, enumerate and find out
            return true;
        };

        if device.is_bluetooth() {
            return true;
        }

        if is_dongle(product_id) {
            if (1..=4).contains(&interface_number) {
                // This is one of the wireless controller interfaces
                return true;
            }
        } else if interface_number == 2 {
            // This is the controller interface (not mouse or keyboard)
            return true;
        }
        false
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(SteamContext::default())
    }
}

/// Translation of `FilterTouch()`.
fn filter_touch(new_value: f32, old_value: f32) -> f32 {
    let jitter = 256.0 / (1 << 16) as f32;
    if new_value > (old_value - jitter * 0.5) && new_value < (old_value + jitter * 0.5) {
        return old_value;
    }
    if new_value > (old_value - jitter) && new_value < (old_value + jitter) {
        return old_value * 0.75 + new_value * 0.25;
    }
    if new_value > (old_value - jitter * 2.0) && new_value < (old_value + jitter * 2.0) {
        return (old_value + new_value) * 0.5;
    }
    new_value
}

/// The brightness of a home LED hint, if it's set (part of
/// `SDL_HomeLEDHintChanged()`).
fn home_led_hint_value(hint: Option<&str>) -> Option<u8> {
    let hint = hint.filter(|h| !h.is_empty())?;
    let value = if hint.contains('.') {
        ((100.0 * crate::stdlib::atof(hint) as f32) as i32).min(255)
    } else if hints::string_to_bool(Some(hint), true) {
        100
    } else {
        0
    };
    // (a negative value wraps, as upstream's Uint8)
    Some(value as u8)
}

/// Translation of `SetHomeLED()`.
fn set_home_led(dev: &dyn SteamHid, value: u8) -> Result<()> {
    if send_message(
        dev,
        settings_report(&[(SETTING_LED_USER_BRIGHTNESS, u16::from(value))]),
    )
    .is_err()
    {
        return Err(Error::new("Couldn't write feature report"));
    }
    Ok(())
}

/// The body of `SDL_HomeLEDHintChanged()`.
fn home_led_hint_changed(dev: &dyn SteamHid, hint: Option<&str>) {
    if let Some(value) = home_led_hint_value(hint) {
        let _ = set_home_led(dev, value);
    }
}

/// `HIDAPI_DriverSteam_SendJoystickEffect()` on `dev`.
fn send_effect(dev: &dyn SteamHid, data: &[u8]) -> Result<()> {
    if data.len() == FEATURE_REPORT_BUFFER_SIZE {
        if set_feature_report(dev, data, data.len()).is_err() {
            return Err(Error::new("Couldn't write feature report"));
        }
        return Ok(());
    }
    Err(Error::unsupported())
}

/// The gyro and accelerometer values of a state, in SDL's units and axes.
fn steam_sensor_values(gyro: (i16, i16, i16), accel: (i16, i16, i16)) -> ([f32; 3], [f32; 3]) {
    let gyro_scale = 2000.0 * (std::f32::consts::PI / 180.0);
    let gyro_values = [
        (f32::from(gyro.0) / 32768.0) * gyro_scale,
        (f32::from(gyro.2) / 32768.0) * gyro_scale,
        (f32::from(gyro.1) / 32768.0) * gyro_scale,
    ];
    let accel_values = [
        (f32::from(accel.0) / 32768.0) * 2.0 * STANDARD_GRAVITY,
        (f32::from(accel.2) / 32768.0) * 2.0 * STANDARD_GRAVITY,
        (-(i32::from(accel.1)) as f32 / 32768.0) * 2.0 * STANDARD_GRAVITY,
    ];
    (gyro_values, accel_values)
}

impl SteamContext {
    /// Translation of `HIDAPI_DriverSteam_SetPairingState()`.
    fn set_pairing_state(&mut self, dev: &dyn SteamHid, enabled: bool) {
        // Only have one dongle in pairing mode at a time
        let pairing_context = PAIRING_CONTEXT.load(Ordering::Relaxed);

        if enabled && pairing_context != 0 {
            return;
        }

        if !enabled && pairing_context != self.id {
            return;
        }

        if self.connected {
            return;
        }

        set_pairing_state(dev, enabled);

        if enabled {
            self.pairing_time = crate::timer::ticks_ms();
            PAIRING_CONTEXT.store(self.id, Ordering::Relaxed);
        } else {
            self.pairing_time = 0;
            PAIRING_CONTEXT.store(0, Ordering::Relaxed);
        }
    }

    /// Translation of `HIDAPI_DriverSteam_RenewPairingState()`.
    fn renew_pairing_state(&mut self, dev: &dyn SteamHid) {
        let now = crate::timer::ticks_ms();

        if now >= self.pairing_time + u64::from(PAIRING_STATE_DURATION_SECONDS) * 1000 {
            set_pairing_state(dev, true);
            self.pairing_time = now;
        }
    }

    /// Translation of `HIDAPI_DriverSteam_CommitPairing()`.
    fn commit_pairing(&self, dev: &dyn SteamHid) {
        commit_pairing(dev);
    }

    /// Translation of `SDL_PairingEnabledHintChanged()`, for a change
    /// recorded since the last call.
    fn pairing_enabled_hint_changed(&mut self, dev: &dyn SteamHid) {
        let Some(hint) = self.pairing_hint.as_ref().and_then(HintWatch::take) else {
            return;
        };
        let enabled = hints::string_to_bool(hint.as_deref(), false);

        self.set_pairing_state(dev, enabled);
    }

    /// Translation of `SDL_HomeLEDHintChanged()`, for a change recorded
    /// since the last call.
    fn home_led_hint_changed(&mut self, dev: &dyn SteamHid) {
        let Some(hint) = self.home_led_hint.as_ref().and_then(HintWatch::take) else {
            return;
        };
        home_led_hint_changed(dev, hint.as_deref());
    }

    /// Translation of `ControllerConnected()`: the new joystick, if it's
    /// open.
    fn controller_connected(
        &mut self,
        device: &mut DeviceCtx<'_>,
        dev: &dyn SteamHid,
    ) -> Option<JoystickID> {
        device.joystick_connected();

        // We'll automatically accept this controller if we're in pairing mode
        self.commit_pairing(dev);

        self.connected = true;
        device.open_joystick_id()
    }

    /// Translation of `ControllerDisconnected()`.
    fn controller_disconnected(&mut self, device: &mut DeviceCtx<'_>) {
        if let Some(&joystick) = device.joysticks().first() {
            device.joystick_disconnected(joystick);
        }
        self.connected = false;
    }

    /// Send the events of the current state (the state part of
    /// `HIDAPI_DriverSteam_UpdateDevice()`).
    fn send_state(&mut self, device: &mut DeviceCtx<'_>, joystick: JoystickID) {
        let timestamp = crate::timer::ticks_ns();
        let state = self.state;
        let buttons = state.buttons;

        if buttons != self.last_state.buttons {
            let mut hat = 0;

            let mut button = |button: u8, mask: u64| {
                device.send_button(timestamp, joystick, button, buttons & mask != 0);
            };
            button(GamepadButton::South as u8, STEAM_BUTTON_SOUTH_MASK);
            button(GamepadButton::East as u8, STEAM_BUTTON_EAST_MASK);
            button(GamepadButton::West as u8, STEAM_BUTTON_WEST_MASK);
            button(GamepadButton::North as u8, STEAM_BUTTON_NORTH_MASK);
            button(GamepadButton::LeftShoulder as u8, STEAM_LEFT_BUMPER_MASK);
            button(GamepadButton::RightShoulder as u8, STEAM_RIGHT_BUMPER_MASK);
            button(GamepadButton::Back as u8, STEAM_BUTTON_MENU_MASK);
            button(GamepadButton::Start as u8, STEAM_BUTTON_ESCAPE_MASK);
            button(GamepadButton::Guide as u8, STEAM_BUTTON_STEAM_MASK);
            button(GamepadButton::LeftStick as u8, STEAM_JOYSTICK_BUTTON_MASK);
            button(
                SDL_GAMEPAD_BUTTON_STEAM_LEFT_PADDLE,
                STEAM_BUTTON_BACK_LEFT_MASK,
            );
            button(
                SDL_GAMEPAD_BUTTON_STEAM_RIGHT_PADDLE,
                STEAM_BUTTON_BACK_RIGHT_MASK,
            );
            button(
                GamepadButton::RightStick as u8,
                STEAM_BUTTON_RIGHTPAD_CLICKED_MASK,
            );

            if buttons & STEAM_DPAD_UP_MASK != 0 {
                hat |= HAT_UP;
            }
            if buttons & STEAM_DPAD_DOWN_MASK != 0 {
                hat |= HAT_DOWN;
            }
            if buttons & STEAM_DPAD_LEFT_MASK != 0 {
                hat |= HAT_LEFT;
            }
            if buttons & STEAM_DPAD_RIGHT_MASK != 0 {
                hat |= HAT_RIGHT;
            }
            device.send_hat(timestamp, joystick, 0, hat);
        }

        let trigger_axis = |trigger: u16| (i32::from(trigger) * 2 - 32768) as i16;
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftTrigger as u8,
            trigger_axis(state.trigger_l),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightTrigger as u8,
            trigger_axis(state.trigger_r),
        );

        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftX as u8,
            state.left_stick_x,
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftY as u8,
            !state.left_stick_y,
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightX as u8,
            state.right_pad_x,
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightY as u8,
            !state.right_pad_y,
        );

        // Note that the left pad is normally mapped to D-Pad, so you should ignore that input if you use the touchpad instead.
        {
            let down = buttons & STEAM_LEFTPAD_FINGERDOWN_MASK != 0;
            if down || self.left_touch_down {
                let clicked = buttons & STEAM_BUTTON_LEFTPAD_CLICKED_MASK != 0;
                let left_x = f32::from(state.left_pad_x) / (1 << 16) as f32 + 0.5;
                let left_y = -f32::from(state.left_pad_y) / (1 << 16) as f32 + 0.5;
                let mut pressure = if down { 0.5 } else { 0.0 };
                if clicked {
                    pressure += 0.5;
                }
                if down {
                    self.left_touch_x = filter_touch(left_x, self.left_touch_x);
                    self.left_touch_y = filter_touch(left_y, self.left_touch_y);
                }
                device.send_touchpad(
                    timestamp,
                    joystick,
                    0,
                    0,
                    down,
                    self.left_touch_x,
                    self.left_touch_y,
                    pressure,
                );
                self.left_touch_down = down;
            }
        }

        // Note that the right pad is normally mapped to right thumbstick, so you should ignore that input if you use the touchpad instead.
        {
            let down = buttons & STEAM_RIGHTPAD_FINGERDOWN_MASK != 0;
            if down || self.right_touch_down {
                let clicked = buttons & STEAM_BUTTON_RIGHTPAD_CLICKED_MASK != 0;
                let right_x = f32::from(state.right_pad_x) / (1 << 16) as f32 + 0.5;
                let right_y = -f32::from(state.right_pad_y) / (1 << 16) as f32 + 0.5;
                let mut pressure = if down { 0.5 } else { 0.0 };
                if clicked {
                    pressure += 0.5;
                }
                if down {
                    self.right_touch_x = filter_touch(right_x, self.right_touch_x);
                    self.right_touch_y = filter_touch(right_y, self.right_touch_y);
                }
                device.send_touchpad(
                    timestamp,
                    joystick,
                    1,
                    0,
                    down,
                    self.right_touch_x,
                    self.right_touch_y,
                    pressure,
                );
                self.right_touch_down = down;
            }
        }

        if self.report_sensors {
            self.sensor_timestamp += u64::from(self.update_rate_in_us) * super::NS_PER_US;

            let (gyro, accel) = steam_sensor_values(
                (state.gyro_x, state.gyro_y, state.gyro_z),
                (state.accel_x, state.accel_y, state.accel_z),
            );
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Gyro,
                self.sensor_timestamp,
                &gyro,
            );
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Accel,
                self.sensor_timestamp,
                &accel,
            );
        }

        self.last_state = self.state;
    }

    /// `HIDAPI_DriverSteam_UpdateDevice()` on `dev`, with the first
    /// joystick of the device if it's open.
    fn update(
        &mut self,
        device: &mut DeviceCtx<'_>,
        dev: &dyn SteamHid,
        mut joystick: Option<JoystickID>,
    ) -> bool {
        // (the hint callbacks of upstream)
        if self.pairing_hint.is_some() {
            self.pairing_enabled_hint_changed(dev);
        }
        if joystick.is_some() {
            self.home_led_hint_changed(dev);
        }

        if self.pairing_time != 0 {
            self.renew_pairing_state(dev);
        }

        loop {
            let mut data = [0u8; 128];

            let r = match read_steam_controller(dev, &mut data) {
                Ok(0) => break,
                Ok(r) => r,
                Err(_) => {
                    // Failed to read from controller
                    self.controller_disconnected(device);
                    return false;
                }
            };

            let packet_length = self.assembler.write_segment(&data, r).unwrap_or(0);

            if packet_length > 0 && self.state.update(&self.assembler.buffer) {
                if !self.connected {
                    // Maybe we missed a wireless status packet?
                    joystick = self.controller_connected(device, dev);
                }

                let Some(joystick) = joystick else {
                    continue;
                };

                self.send_state(device, joystick);
            } else {
                let packet = &self.assembler.buffer[..packet_length];
                if !self.connected && d0g_is_wireless_connect(packet) {
                    joystick = self.controller_connected(device, dev);
                } else if self.connected && d0g_is_wireless_disconnect(packet) {
                    self.controller_disconnected(device);
                    joystick = None;
                }
            }
        }
        true
    }

    /// `HIDAPI_DriverSteam_InitDevice()` on `dev`.
    fn init(&mut self, device: &mut DeviceCtx<'_>, dev: &dyn SteamHid) -> Result<()> {
        if cfg!(windows) && device.serial().is_some() {
            // We get a garbage serial number on Windows
            device.clear_device_serial();
        }

        device.set_device_name("Steam Controller");

        // If this is a wireless dongle, request a wireless state update
        if is_dongle(device.product_id()) {
            if send_message(dev, feature_report(ID_DONGLE_GET_WIRELESS_STATE, &[])).is_err() {
                return Err(Error::new(
                    "Failed to send ID_DONGLE_GET_WIRELESS_STATE request",
                ));
            }

            for _ in 0..5 {
                let mut data = [0u8; 128];

                let res = match read_steam_controller(dev, &mut data) {
                    Ok(0) => {
                        crate::timer::delay(Duration::from_millis(1));
                        continue;
                    }
                    Ok(res) => res,
                    Err(_) => break,
                };

                if d0g_is_wireless_connect(&data[..res]) {
                    self.connected = true;
                    break;
                } else if d0g_is_wireless_disconnect(&data[..res]) {
                    self.connected = false;
                    break;
                }
            }

            self.pairing_hint = Some(HintWatch::new(HINT_JOYSTICK_HIDAPI_STEAM_PAIRING_ENABLED));
            self.pairing_enabled_hint_changed(dev);
        } else {
            // Wired and BLE controllers are always connected if HIDAPI can see them
            self.connected = true;
        }

        if self.connected {
            device.joystick_connected();
        }
        // (otherwise we will enumerate any attached controllers in UpdateDevice())
        Ok(())
    }

    /// `HIDAPI_DriverSteam_OpenJoystick()` on `dev`.
    fn open(
        &mut self,
        device: &DeviceCtx<'_>,
        dev: &dyn SteamHid,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        self.report_sensors = false;
        self.assembler = PacketAssembler::default();
        self.state = SteamControllerStateInternal::default();
        self.last_state = SteamControllerStateInternal::default();

        self.update_rate_in_us = match reset_steam_controller(dev, false) {
            Ok(update_rate_in_us) => update_rate_in_us,
            Err(_) => return Err(Error::new("Couldn't reset controller")),
        };
        let mut update_rate_in_hz = 0.0;
        if self.update_rate_in_us > 0 {
            update_rate_in_hz = 1000000.0 / self.update_rate_in_us as f32;
        }

        self.assembler = PacketAssembler::new(device.is_bluetooth());

        // Initialize the joystick capabilities
        joystick.nbuttons = SDL_GAMEPAD_NUM_STEAM_BUTTONS;
        joystick.naxes = GamepadAxis::COUNT;
        joystick.nhats = 1;

        if is_dongle(device.product_id()) {
            joystick.connection_state = JoystickConnectionState::Wireless;
        }

        joystick.add_sensor(SensorType::Gyro, update_rate_in_hz);
        joystick.add_sensor(SensorType::Accel, update_rate_in_hz);

        joystick.add_touchpad(1);
        joystick.add_touchpad(1);

        self.home_led_hint = Some(HintWatch::new(hints::JOYSTICK_HIDAPI_STEAM_HOME_LED));
        self.home_led_hint_changed(dev);

        Ok(())
    }

    /// `HIDAPI_DriverSteam_FreeDevice()` on `dev`.
    fn free(&mut self, device: &DeviceCtx<'_>, dev: &dyn SteamHid) {
        if is_dongle(device.product_id()) {
            self.pairing_hint = None;

            self.set_pairing_state(dev, false);
        }
    }

    /// `HIDAPI_DriverSteam_SetSensorsEnabled()` on `dev`.
    fn set_sensors_enabled(&mut self, dev: &dyn SteamHid, enabled: bool) -> Result<()> {
        let imu_mode = if enabled {
            SETTING_GYRO_MODE_SEND_RAW_ACCEL | SETTING_GYRO_MODE_SEND_RAW_GYRO
        } else {
            SETTING_GYRO_MODE_OFF
        };
        if send_message(dev, settings_report(&[(SETTING_IMU_MODE, imu_mode)])).is_err() {
            return Err(Error::new("Couldn't write feature report"));
        }

        self.report_sensors = enabled;

        Ok(())
    }
}

impl DriverContext for SteamContext {
    /// Translation of `HIDAPI_DriverSteam_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        let dev = device.device().clone();
        self.init(device, &*dev)
    }

    /// Translation of `HIDAPI_DriverSteam_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        let dev = device.device().clone();
        let joystick = device.open_joystick_id();
        self.update(device, &*dev, joystick)
    }

    /// Translation of `HIDAPI_DriverSteam_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        let dev = device.device().clone();
        self.open(device, &*dev, joystick)
    }

    /// Translation of `HIDAPI_DriverSteam_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        _low_frequency_rumble: u16,
        _high_frequency_rumble: u16,
    ) -> Result<()> {
        // You should use the full Steam Input API for rumble support
        Err(Error::unsupported())
    }

    // (GetJoystickCapabilities: you should use the full Steam Input API
    // for extended capabilities; SetJoystickLED: you should use the full
    // Steam Input API for LED support)

    /// Translation of `HIDAPI_DriverSteam_SendJoystickEffect()`.
    fn send_joystick_effect(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        data: &[u8],
    ) -> Result<()> {
        send_effect(&**device.device(), data)
    }

    /// Translation of `HIDAPI_DriverSteam_SetSensorsEnabled()`.
    fn set_joystick_sensors_enabled(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        enabled: bool,
    ) -> Result<()> {
        self.set_sensors_enabled(&**device.device(), enabled)
    }

    /// Translation of `HIDAPI_DriverSteam_CloseJoystick()`.
    fn close_joystick(&mut self, device: &mut DeviceCtx<'_>, _joystick: JoystickID) {
        self.home_led_hint = None;

        close_steam_controller(&**device.device());
    }

    /// Translation of `HIDAPI_DriverSteam_FreeDevice()`.
    fn free_device(&mut self, device: &mut DeviceCtx<'_>) {
        self.free(device, &**device.device());
    }
}
