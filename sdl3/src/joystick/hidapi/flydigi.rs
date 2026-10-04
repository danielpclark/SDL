// Rust translation of src/joystick/hidapi/SDL_hidapi_flydigi.c and
// SDL_hidapi_flydigi.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Flydigi controller driver (the Apex and Vader controllers), with
//! the original protocol of the early controllers and the newer one,
//! which needs the controller to be acquired from the Flydigi Space
//! Station app.
//!
//! The commands and their replies go through the HID I/O of the Valve
//! drivers ([`SteamHid`]), which the tests fake.

use std::time::Duration;

use super::rumble::send_rumble;
use super::steam::SteamHid;
use super::{
    load16, remap_val, DeviceCtx, DriverContext, DriverImpl, HidapiDevice, JoystickCaps,
    SDL_HIDAPI_DEFAULT, USB_PACKET_LENGTH,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::device_info::is_joystick_flydigi_controller;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::{
    JoystickConnectionState, JoystickData, HAT_CENTERED, HAT_DOWN, HAT_LEFT, HAT_LEFTDOWN,
    HAT_LEFTUP, HAT_RIGHT, HAT_RIGHTDOWN, HAT_RIGHTUP, HAT_UP,
};
use crate::power::PowerState;
use crate::sensor::{SensorType, STANDARD_GRAVITY};

/// The values used in the controller type byte of the controller GUID.
/// Translation of `SDL_FlyDigiControllerType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
enum FlydigiControllerType {
    Unknown = 0,
    Apex2 = 1 << 0,
    Apex3,
    Apex4,
    Apex5,
    Apex6,
    Vader2 = 1 << 4,
    Vader2Pro,
    Vader3,
    Vader3Pro,
    Vader4Pro,
    Vader5Pro,
}

/// `SDL_GAMEPAD_BUTTON_FLYDIGI_M1`
const BUTTON_FLYDIGI_M1: u8 = 11;
/// `SDL_GAMEPAD_BUTTON_FLYDIGI_M2`
const BUTTON_FLYDIGI_M2: u8 = 12;
/// `SDL_GAMEPAD_BUTTON_FLYDIGI_M3`
const BUTTON_FLYDIGI_M3: u8 = 13;
/// `SDL_GAMEPAD_BUTTON_FLYDIGI_M4`
const BUTTON_FLYDIGI_M4: u8 = 14;
/// `SDL_GAMEPAD_NUM_BASE_FLYDIGI_BUTTONS`
const NUM_BASE_FLYDIGI_BUTTONS: u8 = 15;

/// `SDL_NS_PER_SECOND`
const NS_PER_SECOND: u64 = 1_000_000_000;

/// Rate of IMU Sensor Packets over wireless dongle observed in testcontroller at 1000hz
const SENSOR_INTERVAL_VADER4_PRO_DONGLE_RATE_HZ: u64 = 1000;
const SENSOR_INTERVAL_VADER4_PRO_DONGLE_NS: u64 =
    NS_PER_SECOND / SENSOR_INTERVAL_VADER4_PRO_DONGLE_RATE_HZ;
/// Rate of IMU Sensor Packets over wired connection observed in testcontroller at 500hz
const SENSOR_INTERVAL_VADER4_PRO_WIRED_RATE_HZ: u64 = 500;
const SENSOR_INTERVAL_VADER4_PRO_WIRED_NS: u64 =
    NS_PER_SECOND / SENSOR_INTERVAL_VADER4_PRO_WIRED_RATE_HZ;
/// Rate of IMU Sensor Packets over wired connection observed in testcontroller at 500hz
const SENSOR_INTERVAL_VADER5_PRO_RATE_HZ: u64 = 500;
const SENSOR_INTERVAL_VADER5_PRO_NS: u64 = NS_PER_SECOND / SENSOR_INTERVAL_VADER5_PRO_RATE_HZ;

/// Rate of IMU Sensor Packets over wireless dongle observed in testcontroller at 295hz
const SENSOR_INTERVAL_APEX5_DONGLE_RATE_HZ: u64 = 295;
const SENSOR_INTERVAL_APEX5_DONGLE_NS: u64 = NS_PER_SECOND / SENSOR_INTERVAL_APEX5_DONGLE_RATE_HZ;
/// Rate of IMU Sensor Packets over wired connection observed in testcontroller at 970hz
const SENSOR_INTERVAL_APEX5_WIRED_RATE_HZ: u64 = 970;
const SENSOR_INTERVAL_APEX5_WIRED_NS: u64 = NS_PER_SECOND / SENSOR_INTERVAL_APEX5_WIRED_RATE_HZ;

/// Milliseconds.
const FLYDIGI_ACQUIRE_CONTROLLER_HEARTBEAT_TIME: u64 = 1000 * 30;

const FLYDIGI_V1_CMD_REPORT_ID: u8 = 0x05;
const FLYDIGI_V1_HAPTIC_COMMAND: u8 = 0x0F;
const FLYDIGI_V1_GET_INFO_COMMAND: u8 = 0xEC;

const FLYDIGI_V2_CMD_REPORT_ID: u8 = 0x03;
const FLYDIGI_V2_MAGIC1: u8 = 0x5A;
const FLYDIGI_V2_MAGIC2: u8 = 0xA5;
const FLYDIGI_V2_GET_INFO_COMMAND: u8 = 0x01;
const FLYDIGI_V2_CHECK_ARCHITECTURE_COMMAND: u8 = 0x07;
const FLYDIGI_V2_GET_STATUS_COMMAND: u8 = 0x10;
const FLYDIGI_V2_SET_STATUS_COMMAND: u8 = 0x11;
const FLYDIGI_V2_HAPTIC_COMMAND: u8 = 0x12;
const FLYDIGI_V2_ACQUIRE_CONTROLLER_COMMAND: u8 = 0x1C;
const FLYDIGI_V2_INPUT_REPORT: u8 = 0xEF;

/// `DEG2RAD()`
fn deg2rad(x: f32) -> f32 {
    x * (std::f32::consts::PI / 180.0)
}

/// The Flydigi driver's static functions.
pub(crate) struct FlydigiDriver;

impl DriverImpl for FlydigiDriver {
    /// Translation of `HIDAPI_DriverFlydigi_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_FLYDIGI]
    }

    /// Translation of `HIDAPI_DriverFlydigi_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_FLYDIGI,
            hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
        )
    }

    /// Translation of `HIDAPI_DriverFlydigi_IsSupportedDevice()`.
    fn is_supported_device(
        &self,
        _device: Option<&HidapiDevice>,
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
        if is_joystick_flydigi_controller(vendor_id, product_id) {
            if vendor_id == USB_VENDOR_FLYDIGI_V1 {
                if interface_number == 2 {
                    // Early controllers have their custom protocol on interface 2
                    return true;
                }
            } else {
                // Newer controllers have their custom protocol on interface 1 or 2, but
                // only expose one HID interface, so we'll accept any interface we see.
                return true;
            }
        }
        false
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(FlydigiContext::default())
    }
}

/// Translation of `SDL_DriverFlydigi_Context` (its `device` is the
/// device the driver functions are called with).
#[derive(Debug)]
struct FlydigiContext {
    device_id: u8,
    available: bool,
    has_cz: bool,
    has_lmrm: bool,
    has_circle: bool,
    wireless: bool,
    sensors_supported: bool,
    sensors_enabled: bool,
    firmware_version: u16,
    /// Simulate onboard clock. Advance by known time step. Nanoseconds.
    sensor_timestamp_ns: u64,
    /// Based on observed rate of receipt of IMU sensor packets.
    sensor_timestamp_step_ns: u64,
    accel_scale: f32,
    gyro_scale: f32,
    next_heartbeat: u64,
    last_packet: u64,
    last_state: [u8; USB_PACKET_LENGTH],
}

impl Default for FlydigiContext {
    fn default() -> Self {
        FlydigiContext {
            device_id: 0,
            available: false,
            has_cz: false,
            has_lmrm: false,
            has_circle: false,
            wireless: false,
            sensors_supported: false,
            sensors_enabled: false,
            firmware_version: 0,
            sensor_timestamp_ns: 0,
            sensor_timestamp_step_ns: 0,
            accel_scale: 0.0,
            gyro_scale: 0.0,
            next_heartbeat: 0,
            last_packet: 0,
            last_state: [0; USB_PACKET_LENGTH],
        }
    }
}

/// The controller type of a device ID, or guessed from the name of the
/// controller (part of `HIDAPI_DriverFlydigi_UpdateDeviceIdentity()`).
fn controller_type_of(device_id: u8, name: &str) -> FlydigiControllerType {
    use FlydigiControllerType::*;
    match device_id {
        19 => Apex2,
        24 | 26 | 29 => Apex3,
        84 => Apex4,
        20 | 21 | 23 => Vader2,
        22 => Vader2Pro,
        28 => Vader3,
        80 | 81 => Vader3Pro,
        85 | 91 | 105 => Vader4Pro,
        128 | 129 => Apex5,
        130 => Vader5Pro,
        133 | 134 => Apex5,
        149 // Apex6
        | 150 // Apex6 Pro
        | 152 // Apex 6 Pro Phantom Blade Zero
        => Apex6,
        _ => {
            // Try to guess from the name of the controller
            if crate::stdlib::string::strcasestr(name, "VADER").is_some() {
                if name.contains("VADER2") {
                    Vader2
                } else if name.contains("VADER3") {
                    Vader3
                } else if name.contains("VADER4") {
                    Vader4Pro
                } else if name.contains("Vader 5") {
                    Vader5Pro
                } else {
                    Unknown
                }
            } else if name.contains("APEX") {
                if name.contains("APEX2") {
                    Apex2
                } else if name.contains("APEX3") {
                    Apex3
                } else if name.contains("APEX4") {
                    Apex4
                } else if name.contains("APEX5") {
                    Apex5
                } else if name.contains("APEX6") {
                    Apex6
                } else {
                    Unknown
                }
            } else {
                Unknown
            }
        }
    }
}

/// The report of a command of the newer controllers.
fn v2_command(command: u8, args: &[u8]) -> Vec<u8> {
    let mut cmd = vec![
        FLYDIGI_V2_CMD_REPORT_ID,
        FLYDIGI_V2_MAGIC1,
        FLYDIGI_V2_MAGIC2,
        command,
    ];
    cmd.extend_from_slice(args);
    cmd
}

/// The command of `SDL_HIDAPI_Flydigi_CheckNewArchitectureRequest()`.
fn check_new_architecture_command() -> Vec<u8> {
    v2_command(FLYDIGI_V2_CHECK_ARCHITECTURE_COMMAND, &[0, 0])
}

/// The command of `SDL_HIDAPI_Flydigi_SendInfoRequest()`.
fn info_command() -> Vec<u8> {
    v2_command(FLYDIGI_V2_GET_INFO_COMMAND, &[2, 0])
}

/// The command of `SDL_HIDAPI_Flydigi_SendStatusRequest()`.
fn status_command() -> Vec<u8> {
    v2_command(FLYDIGI_V2_GET_STATUS_COMMAND, &[])
}

/// The command of `SDL_HIDAPI_Flydigi_SendAcquireRequest()`.
fn acquire_command(acquire: bool) -> Vec<u8> {
    let mut cmd = v2_command(
        FLYDIGI_V2_ACQUIRE_CONTROLLER_COMMAND,
        &[23, u8::from(acquire), b'S', b'D', b'L'],
    );
    cmd.resize(32, 0);
    cmd
}

/// The rumble report of `HIDAPI_DriverFlydigi_RumbleJoystick()`.
fn rumble_packet(vendor_id: u16, low_frequency_rumble: u16, high_frequency_rumble: u16) -> Vec<u8> {
    let (low, high) = (
        (low_frequency_rumble >> 8) as u8,
        (high_frequency_rumble >> 8) as u8,
    );
    if vendor_id == USB_VENDOR_FLYDIGI_V1 {
        vec![
            FLYDIGI_V1_CMD_REPORT_ID,
            FLYDIGI_V1_HAPTIC_COMMAND,
            low,
            high,
        ]
    } else {
        let mut packet = v2_command(FLYDIGI_V2_HAPTIC_COMMAND, &[6, 0, 0, 0, 0, 0]);
        packet[5] = low;
        packet[6] = high;
        packet
    }
}

/// Translation of `HIDAPI_DriverFlydigi_WritePacket()`.
fn write_packet(device: &HidapiDevice, dev: &dyn SteamHid, data: &[u8]) -> Result<usize> {
    // We know that at the very least the Vader 5 now uses unnumbered reports for commands instead of FLYDIGI_V2_CMD_REPORT_ID.
    // If other Flydigi things prove to do the same, we can tweak this check to be more general.
    let uses_unnumbered_reports = device.vendor_id() == USB_VENDOR_FLYDIGI_V2
        && (device.product_id() == USB_PRODUCT_FLYDIGI_V2_VADER
            || device.product_id() == USB_PRODUCT_FLYDIGI_V2_APEX6);

    if uses_unnumbered_reports && data[0] == FLYDIGI_V2_CMD_REPORT_ID {
        // Zero out the report byte.
        let size = data.len().min(USB_PACKET_LENGTH);
        let mut output = [0u8; USB_PACKET_LENGTH];
        output[..size].copy_from_slice(&data[..size]);
        output[0] = 0;
        return dev.write(&output[..size]);
    }

    dev.write(data)
}

/// Translation of `GetReply()`: wait for the reply to a command of the
/// newer controllers, in `data` without its report ID.
fn get_reply(dev: &dyn SteamHid, command: u8, data: &mut [u8]) -> bool {
    for _ in 0..100 {
        crate::timer::delay(Duration::from_millis(1));

        let size = match dev.read_timeout(data, 0) {
            Err(_) => break,
            Ok(0) => continue,
            Ok(size) => size,
        };

        if size == 32 {
            if data[1] == FLYDIGI_V2_MAGIC1 && data[2] == FLYDIGI_V2_MAGIC2 {
                // Skip the report ID
                data.copy_within(1..size, 0);
                data[size - 1] = 0;
            }
            if data[0] == FLYDIGI_V2_MAGIC1 && data[1] == FLYDIGI_V2_MAGIC2 && data[2] == command {
                return true;
            }
        }
    }
    false
}

/// The hat of a direction bit mask (up, right, down, left from the low bit).
fn hat_of_mask(value: u8) -> u8 {
    match value & 0x0F {
        0x01 => HAT_UP,
        0x03 => HAT_RIGHTUP,
        0x02 => HAT_RIGHT,
        0x06 => HAT_RIGHTDOWN,
        0x04 => HAT_DOWN,
        0x0C => HAT_LEFTDOWN,
        0x08 => HAT_LEFT,
        0x09 => HAT_LEFTUP,
        _ => HAT_CENTERED,
    }
}

/// A stick axis of an early controller's state packet (`READ_STICK_AXIS()`).
fn read_stick_axis(value: u8) -> i16 {
    if value == 0x7f {
        0
    } else {
        remap_val(
            (i32::from(value) - 0x7f) as f32,
            -0x7f as f32,
            (0xff - 0x7f) as f32,
            f32::from(i16::MIN),
            f32::from(i16::MAX),
        ) as i16
    }
}

/// A trigger axis of a state packet (`READ_TRIGGER_AXIS()`).
fn read_trigger_axis(value: u8) -> i16 {
    (i32::from(value) * 257 - 32768) as i16
}

/// A Y axis of a newer controller's state packet, inverted.
fn read_inverted_axis(a: u8, b: u8) -> i16 {
    // (the negation wraps in the Sint16, as upstream's)
    let axis = load16(a, b).wrapping_neg();
    if axis <= -32768 {
        32767
    } else {
        axis
    }
}

/// `LOAD16()` negated as an int, as a float.
fn negated(a: u8, b: u8) -> f32 {
    -i32::from(load16(a, b)) as f32
}

impl FlydigiContext {
    /// Translation of `HIDAPI_DriverFlydigi_UpdateDeviceIdentity()`.
    fn update_device_identity(&mut self, device: &DeviceCtx<'_>) {
        let controller_type = controller_type_of(self.device_id, &device.name());
        device.set_guid_byte(15, controller_type as u8);

        // This is the previous sensor default of 125hz.
        // Override this in the switch statement below based on observed sensor packet rate.
        self.sensor_timestamp_step_ns = NS_PER_SECOND / 125;

        let apex5_step = if self.wireless {
            SENSOR_INTERVAL_APEX5_DONGLE_NS
        } else {
            SENSOR_INTERVAL_APEX5_WIRED_NS
        };
        let vader4_step = if self.wireless {
            SENSOR_INTERVAL_VADER4_PRO_DONGLE_NS
        } else {
            SENSOR_INTERVAL_VADER4_PRO_WIRED_NS
        };
        match controller_type {
            FlydigiControllerType::Apex2 => {
                device.set_device_name("Flydigi Apex 2");
            }
            FlydigiControllerType::Apex3 => {
                device.set_device_name("Flydigi Apex 3");
            }
            FlydigiControllerType::Apex4 => {
                // The Apex 4 controller has sensors, but they're only reported when gyro mouse is enabled
                device.set_device_name("Flydigi Apex 4");
            }
            FlydigiControllerType::Apex5 => {
                device.set_device_name("Flydigi Apex 5");
                self.has_lmrm = true;
                self.sensors_supported = true;
                self.accel_scale = STANDARD_GRAVITY / 4096.0;
                self.gyro_scale = deg2rad(2000.0);
                self.sensor_timestamp_step_ns = apex5_step;
            }
            FlydigiControllerType::Apex6 => {
                device.set_device_name("Flydigi Apex 6");
                self.has_lmrm = true;
                self.sensors_supported = true;
                self.accel_scale = STANDARD_GRAVITY / 4096.0;
                self.gyro_scale = deg2rad(2000.0);
                self.sensor_timestamp_step_ns = apex5_step;
            }
            FlydigiControllerType::Vader2 => {
                // The Vader 2 controller has sensors, but they're only reported when gyro mouse is enabled
                device.set_device_name("Flydigi Vader 2");
                self.has_cz = true;
            }
            FlydigiControllerType::Vader2Pro => {
                device.set_device_name("Flydigi Vader 2 Pro");
                self.has_cz = true;
            }
            FlydigiControllerType::Vader3 => {
                device.set_device_name("Flydigi Vader 3");
                self.has_cz = true;
            }
            FlydigiControllerType::Vader3Pro => {
                device.set_device_name("Flydigi Vader 3 Pro");
                self.has_cz = true;
                self.sensors_supported = true;
                self.accel_scale = STANDARD_GRAVITY / 256.0;
                self.sensor_timestamp_step_ns = vader4_step;
            }
            FlydigiControllerType::Vader4Pro => {
                device.set_device_name("Flydigi Vader 4 Pro");
                self.has_cz = true;
                self.sensors_supported = true;
                self.accel_scale = STANDARD_GRAVITY / 256.0;
                self.sensor_timestamp_step_ns = vader4_step;
            }
            FlydigiControllerType::Vader5Pro => {
                device.set_device_name("Flydigi Vader 5 Pro");
                self.has_cz = true;
                self.has_lmrm = true;
                self.has_circle = true;
                self.sensors_supported = true;
                self.accel_scale = STANDARD_GRAVITY / 4096.0;
                self.gyro_scale = deg2rad(2000.0);
                self.sensor_timestamp_step_ns = SENSOR_INTERVAL_VADER5_PRO_NS;
            }
            FlydigiControllerType::Unknown => {
                crate::log::debug!(
                    crate::log::Category::Input,
                    "Unknown FlyDigi controller with ID {}, name '{}'",
                    self.device_id,
                    device.name()
                );
            }
        }
    }

    /// Translation of `HIDAPI_DriverFlydigi_SetAvailable()`.
    fn set_available(&mut self, device: &mut DeviceCtx<'_>, available: bool) {
        if available == self.available {
            return;
        }

        if available {
            if device.num_joysticks() == 0 {
                device.joystick_connected();
            }
        } else if let Some(&joystick) = device.joysticks().first() {
            device.joystick_disconnected(joystick);
        }
        self.available = available;
    }

    /// The information of a `FLYDIGI_V1_GET_INFO_COMMAND` reply (part of
    /// `HIDAPI_DriverFlydigi_InitControllerV1()`); its serial number.
    fn apply_v1_info(&mut self, data: &[u8]) -> String {
        self.device_id = data[3];
        self.firmware_version = load16(data[9], data[10]) as u16;

        let serial = format!(
            "{:02x}{:02x}{:02x}{:02x}",
            data[5], data[6], data[7], data[8]
        );

        // The Vader 2 with firmware 6.0.4.9 doesn't report the connection state
        if self.firmware_version >= 0x6400 {
            match data[13] {
                0 => {
                    // Wireless connection
                    self.wireless = true;
                }
                1 => {
                    // Wired connection
                    self.wireless = false;
                }
                _ => {}
            }
        }
        serial
    }

    /// Translation of `HIDAPI_DriverFlydigi_InitControllerV1()`.
    fn init_controller_v1(&mut self, device: &mut DeviceCtx<'_>, dev: &dyn SteamHid) {
        // Detecting the Vader 2 can take over 1000 read retries, so be generous here
        let mut attempt = 0;
        while self.device_id == 0 && attempt < 30 {
            attempt += 1;
            let request = [
                FLYDIGI_V1_CMD_REPORT_ID,
                FLYDIGI_V1_GET_INFO_COMMAND,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            ];
            // This write will occasionally return -1, so ignore failure here and try again
            let _ = write_packet(device, dev, &request);

            // Read the reply
            for _ in 0..100 {
                crate::timer::delay(Duration::from_millis(1));

                let mut data = [0u8; USB_PACKET_LENGTH];
                let size = match dev.read_timeout(&mut data, 0) {
                    Err(_) => break,
                    Ok(0) => continue,
                    Ok(size) => size,
                };

                if size == 32 && data[15] == 236 {
                    let serial = self.apply_v1_info(&data);
                    device.set_device_serial(&serial);

                    // Done!
                    break;
                }
            }
        }

        self.update_device_identity(device);

        self.set_available(device, true);
    }

    /// The firmware and connection of a `FLYDIGI_V2_GET_INFO_COMMAND` reply
    /// (part of `HIDAPI_DriverFlydigi_InitControllerV2()`).
    fn apply_v2_info(
        &mut self,
        product_id: u16,
        data: &[u8],
        is_new_architecture: bool,
    ) -> Result<()> {
        // Check the firmware version
        self.firmware_version = load16(data[16], data[15]) as u16;
        let min_firmware_version = match product_id {
            USB_PRODUCT_FLYDIGI_V2_APEX => {
                // Minimum supported firmware version, Apex 5
                0x7031
            }
            USB_PRODUCT_FLYDIGI_V2_VADER => {
                // Minimum supported firmware version, Vader 5 Pro
                0x7141
            }
            _ => {
                // Unknown product, presumably this version is okay?
                0
            }
        };
        if self.firmware_version < min_firmware_version {
            return Err(Error::new("Unsupported firmware version"));
        }

        if is_new_architecture {
            match data[6] {
                0 => {
                    // Wired connection
                    self.wireless = false;
                }
                1 => {
                    // Wireless connection
                    self.wireless = true;
                }
                _ => {}
            }
        } else {
            match data[6] {
                1 => {
                    // Wired connection
                    self.wireless = false;
                }
                2 => {
                    // Wireless connection
                    self.wireless = true;
                }
                _ => {}
            }
        }
        self.device_id = data[5];
        Ok(())
    }

    /// Translation of `HIDAPI_DriverFlydigi_InitControllerV2()`.
    fn init_controller_v2(&mut self, device: &mut DeviceCtx<'_>, dev: &dyn SteamHid) -> Result<()> {
        // Check whether is the new architecture
        let mut version_data = [0u8; USB_PACKET_LENGTH];
        if write_packet(device, dev, &check_new_architecture_command()).is_err() {
            return Err(Error::new("Couldn't query controller info"));
        }
        if !get_reply(
            dev,
            FLYDIGI_V2_CHECK_ARCHITECTURE_COMMAND,
            &mut version_data,
        ) {
            return Err(Error::new("Couldn't get controller info"));
        }

        let is_new_architecture = version_data[3] == 1 && version_data[4] == 0;

        let mut data = [0u8; USB_PACKET_LENGTH];

        if write_packet(device, dev, &info_command()).is_err() {
            return Err(Error::new("Couldn't query controller info"));
        }
        if !get_reply(dev, FLYDIGI_V2_GET_INFO_COMMAND, &mut data) {
            return Err(Error::new("Couldn't get controller info"));
        }

        self.apply_v2_info(device.product_id(), &data, is_new_architecture)?;

        self.update_device_identity(device);

        // See whether we can acquire the controller
        let _ = self.send_status_request(device, dev);

        Ok(())
    }

    /// Translation of `SDL_HIDAPI_Flydigi_SendStatusRequest()`.
    fn send_status_request(&self, device: &HidapiDevice, dev: &dyn SteamHid) -> Result<()> {
        if write_packet(device, dev, &status_command()).is_err() {
            return Err(Error::new("Couldn't query controller status"));
        }
        Ok(())
    }

    /// Translation of `HIDAPI_DriverFlydigi_HandleInfoResponse()`.
    fn handle_info_response(&self, device: &mut DeviceCtx<'_>, joystick: JoystickID, data: &[u8]) {
        let status = (data[11] >> 4) & 0x0F;
        let level = i32::from(data[11] & 0x0F);
        let is_apex6 = device.guid().0[15] == FlydigiControllerType::Apex6 as u8;

        let (state, percent) = match status {
            0 => (
                PowerState::OnBattery,
                if is_apex6 { level * 10 } else { level * 20 },
            ),
            1 => (
                PowerState::Charging,
                if is_apex6 { level * 10 } else { level * 20 },
            ),
            2 => (PowerState::Charged, 100),
            _ => (PowerState::Unknown, 0),
        };
        device.send_power_info(joystick, state, percent);
    }

    /// Translation of `HIDAPI_DriverFlydigi_HandleStatusResponse()`.
    fn handle_status_response(&mut self, device: &mut DeviceCtx<'_>, data: &[u8]) {
        if data[9] == 1 {
            self.set_available(device, true);
        } else {
            // Click "Allow third-party apps to take over mappings" in the FlyDigi Space Station app
            self.set_available(device, false);
        }
    }

    /// Translation of `HIDAPI_DriverFlydigi_HandleAcquireResponse()`.
    fn handle_acquire_response(&mut self, device: &mut DeviceCtx<'_>, data: &[u8]) {
        if data[5] != 1 && data[6] == 0 {
            // Controller acquiring failed or has been disabled
            self.set_available(device, false);
        }
    }

    /// Translation of `HIDAPI_DriverFlydigi_HandleStatePacketV1()`.
    fn handle_state_packet_v1(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        let timestamp = crate::timer::ticks_ns();

        let extra_button_index = NUM_BASE_FLYDIGI_BUTTONS;

        let button = |device: &mut DeviceCtx<'_>, button: u8, down: bool| {
            device.send_button(timestamp, joystick, button, down);
        };

        if self.last_state[9] != data[9] {
            device.send_hat(timestamp, joystick, 0, hat_of_mask(data[9]));

            button(device, GamepadButton::South as u8, data[9] & 0x10 != 0);
            button(device, GamepadButton::East as u8, data[9] & 0x20 != 0);
            button(device, GamepadButton::Back as u8, data[9] & 0x40 != 0);
            button(device, GamepadButton::West as u8, data[9] & 0x80 != 0);
        }

        if self.last_state[10] != data[10] {
            button(device, GamepadButton::North as u8, data[10] & 0x01 != 0);
            button(device, GamepadButton::Start as u8, data[10] & 0x02 != 0);
            button(
                device,
                GamepadButton::LeftShoulder as u8,
                data[10] & 0x04 != 0,
            );
            button(
                device,
                GamepadButton::RightShoulder as u8,
                data[10] & 0x08 != 0,
            );
            button(device, GamepadButton::LeftStick as u8, data[10] & 0x40 != 0);
            button(
                device,
                GamepadButton::RightStick as u8,
                data[10] & 0x80 != 0,
            );
        }

        if self.last_state[7] != data[7] {
            button(device, BUTTON_FLYDIGI_M1, data[7] & 0x04 != 0);
            button(device, BUTTON_FLYDIGI_M2, data[7] & 0x08 != 0);
            button(device, BUTTON_FLYDIGI_M3, data[7] & 0x10 != 0);
            button(device, BUTTON_FLYDIGI_M4, data[7] & 0x20 != 0);
            if self.has_cz {
                button(device, extra_button_index, data[7] & 0x01 != 0);
                button(device, extra_button_index + 1, data[7] & 0x02 != 0);
            }
        }

        if self.last_state[8] != data[8] {
            button(device, GamepadButton::Guide as u8, data[8] & 0x08 != 0);
            // The '+' button is used to toggle gyro mouse mode, so don't pass that to the application
            // SDL_SendJoystickButton(timestamp, joystick, extra_button_index++, ((data[8] & 0x01) != 0));
            // The '-' button is only available on the Vader 2, for simplicity let's ignore that
            // SDL_SendJoystickButton(timestamp, joystick, extra_button_index++, ((data[8] & 0x10) != 0));
        }

        let mut axis = |axis: GamepadAxis, value: i16| {
            device.send_axis(timestamp, joystick, axis as u8, value);
        };
        axis(GamepadAxis::LeftX, read_stick_axis(data[17]));
        axis(GamepadAxis::LeftY, read_stick_axis(data[19]));
        axis(GamepadAxis::RightX, read_stick_axis(data[21]));
        axis(GamepadAxis::RightY, read_stick_axis(data[22]));

        axis(GamepadAxis::LeftTrigger, read_trigger_axis(data[23]));
        axis(GamepadAxis::RightTrigger, read_trigger_axis(data[24]));

        if self.sensors_enabled {
            // Advance the imu sensor time stamp based on the observed rate of receipt of packets in the testcontroller app.
            // This varies between Product ID and connection type.
            let sensor_timestamp = self.sensor_timestamp_ns;
            self.sensor_timestamp_ns = self
                .sensor_timestamp_ns
                .wrapping_add(self.sensor_timestamp_step_ns);

            // Pitch and yaw scales may be receiving extra filtering for the sake of bespoke direct mouse output.
            // As result, roll has a different scaling factor than pitch and yaw.
            // These values were estimated using the testcontroller tool in lieux of hard data sheet references.
            let pitch_and_yaw_scale = deg2rad(72000.0);
            let roll_scale = deg2rad(1200.0);

            let remap = |value: f32, scale: f32| {
                remap_val(
                    value,
                    f32::from(i16::MIN),
                    f32::from(i16::MAX),
                    -scale,
                    scale,
                )
            };
            let values = [
                remap(-f32::from(load16(data[26], data[27])), pitch_and_yaw_scale),
                remap(-f32::from(load16(data[18], data[20])), pitch_and_yaw_scale),
                remap(-f32::from(load16(data[29], data[30])), roll_scale),
            ];
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Gyro,
                sensor_timestamp,
                &values,
            );

            let accel_scale = self.accel_scale;
            let values = [
                negated(data[11], data[12]) * accel_scale, // Acceleration along pitch axis
                f32::from(load16(data[15], data[16])) * accel_scale, // Acceleration along yaw axis
                f32::from(load16(data[13], data[14])) * accel_scale, // Acceleration along roll axis
            ];
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Accel,
                sensor_timestamp,
                &values,
            );
        }

        let n = size.min(self.last_state.len());
        self.last_state[..n].copy_from_slice(&data[..n]);
    }

    /// Translation of `HIDAPI_DriverFlydigi_HandlePacketV1()`.
    fn handle_packet_v1(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: Option<JoystickID>,
        data: &[u8],
        size: usize,
    ) {
        if data[0] != 0x04 || data[1] != 0xFE {
            // We don't know how to handle this report, ignore it
            return;
        }

        if let Some(joystick) = joystick {
            self.handle_state_packet_v1(device, joystick, data, size);
        }
    }

    /// Translation of `HIDAPI_DriverFlydigi_HandleStatePacketV2()`.
    fn handle_state_packet_v2(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        let timestamp = crate::timer::ticks_ns();

        let mut extra_button_index = NUM_BASE_FLYDIGI_BUTTONS;
        let mut next_extra_button = || {
            let index = extra_button_index;
            extra_button_index += 1;
            index
        };

        let button = |device: &mut DeviceCtx<'_>, button: u8, down: bool| {
            device.send_button(timestamp, joystick, button, down);
        };

        if self.last_state[11] != data[11] {
            device.send_hat(timestamp, joystick, 0, hat_of_mask(data[11]));

            button(device, GamepadButton::South as u8, data[11] & 0x10 != 0);
            button(device, GamepadButton::East as u8, data[11] & 0x20 != 0);
            button(device, GamepadButton::Back as u8, data[11] & 0x40 != 0);
            button(device, GamepadButton::West as u8, data[11] & 0x80 != 0);
        }

        if self.last_state[12] != data[12] {
            button(device, GamepadButton::North as u8, data[12] & 0x01 != 0);
            button(device, GamepadButton::Start as u8, data[12] & 0x02 != 0);
            button(
                device,
                GamepadButton::LeftShoulder as u8,
                data[12] & 0x04 != 0,
            );
            button(
                device,
                GamepadButton::RightShoulder as u8,
                data[12] & 0x08 != 0,
            );
            button(device, GamepadButton::LeftStick as u8, data[12] & 0x40 != 0);
            button(
                device,
                GamepadButton::RightStick as u8,
                data[12] & 0x80 != 0,
            );
        }

        if self.last_state[13] != data[13] {
            button(device, BUTTON_FLYDIGI_M1, data[13] & 0x04 != 0);
            button(device, BUTTON_FLYDIGI_M2, data[13] & 0x08 != 0);
            button(device, BUTTON_FLYDIGI_M3, data[13] & 0x10 != 0);
            button(device, BUTTON_FLYDIGI_M4, data[13] & 0x20 != 0);
            if self.has_cz {
                button(device, next_extra_button(), data[13] & 0x01 != 0);
                button(device, next_extra_button(), data[13] & 0x02 != 0);
            }
            if self.has_lmrm {
                button(device, next_extra_button(), data[13] & 0x40 != 0);
                button(device, next_extra_button(), data[13] & 0x80 != 0);
            }
        } else {
            if self.has_cz {
                next_extra_button();
                next_extra_button();
            }
            if self.has_lmrm {
                next_extra_button();
                next_extra_button();
            }
        }

        if self.last_state[14] != data[14] {
            button(device, GamepadButton::Guide as u8, data[14] & 0x08 != 0);
            if self.has_circle {
                button(device, next_extra_button(), data[14] & 0x01 != 0);
            }
        }

        let mut axis = |axis: GamepadAxis, value: i16| {
            device.send_axis(timestamp, joystick, axis as u8, value);
        };
        axis(GamepadAxis::LeftX, load16(data[3], data[4]));
        axis(GamepadAxis::LeftY, read_inverted_axis(data[5], data[6]));
        axis(GamepadAxis::RightX, load16(data[7], data[8]));
        axis(GamepadAxis::RightY, read_inverted_axis(data[9], data[10]));

        axis(GamepadAxis::LeftTrigger, read_trigger_axis(data[15]));
        axis(GamepadAxis::RightTrigger, read_trigger_axis(data[16]));

        if self.sensors_enabled {
            // Advance the imu sensor time stamp based on the observed rate of receipt of packets in the testcontroller app.
            // This varies between Product ID and connection type.
            let sensor_timestamp = self.sensor_timestamp_ns;
            self.sensor_timestamp_ns = self
                .sensor_timestamp_ns
                .wrapping_add(self.sensor_timestamp_step_ns);

            let gyro_scale = self.gyro_scale;
            let remap = |value: f32| {
                remap_val(
                    value,
                    f32::from(i16::MIN),
                    f32::from(i16::MAX),
                    -gyro_scale,
                    gyro_scale,
                )
            };
            let values = [
                remap(f32::from(load16(data[17], data[18]))),
                remap(f32::from(load16(data[21], data[22]))),
                remap(-f32::from(load16(data[19], data[20]))),
            ];
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Gyro,
                sensor_timestamp,
                &values,
            );

            let accel_scale = self.accel_scale;
            let values = [
                f32::from(load16(data[23], data[24])) * accel_scale, // Acceleration along pitch axis
                f32::from(load16(data[27], data[28])) * accel_scale, // Acceleration along yaw axis
                negated(data[25], data[26]) * accel_scale,           // Acceleration along roll axis
            ];
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Accel,
                sensor_timestamp,
                &values,
            );
        }

        let n = size.min(self.last_state.len());
        self.last_state[..n].copy_from_slice(&data[..n]);
    }

    /// Translation of `HIDAPI_DriverFlydigi_HandlePacketV2()`.
    fn handle_packet_v2(
        &mut self,
        device: &mut DeviceCtx<'_>,
        dev: &dyn SteamHid,
        joystick: Option<JoystickID>,
        mut data: &[u8],
        mut size: usize,
    ) {
        if size > 0 && data[0] != 0x5A {
            // If first byte is not 0x5A, it must be REPORT_ID, we need to remove it.
            data = &data[1..];
            size -= 1;
        }
        if size < 31 || data[0] != FLYDIGI_V2_MAGIC1 || data[1] != FLYDIGI_V2_MAGIC2 {
            // We don't know how to handle this report, ignore it
            return;
        }

        match data[2] {
            FLYDIGI_V2_GET_INFO_COMMAND => {
                if let Some(joystick) = joystick {
                    self.handle_info_response(device, joystick, data);
                }
            }
            FLYDIGI_V2_SET_STATUS_COMMAND => {
                // (HIDAPI_DriverFlydigi_HandleStatusUpdate())
                // The status changed, see if we can acquire the controller now
                let _ = self.send_status_request(device, dev);
            }
            FLYDIGI_V2_GET_STATUS_COMMAND => {
                self.handle_status_response(device, data);
            }
            FLYDIGI_V2_ACQUIRE_CONTROLLER_COMMAND => {
                self.handle_acquire_response(device, data);
            }
            FLYDIGI_V2_INPUT_REPORT => {
                if let Some(joystick) = joystick {
                    self.handle_state_packet_v2(device, joystick, data, size);
                }
            }
            _ => {
                // We don't recognize this command, ignore it
            }
        }
    }

    /// `HIDAPI_DriverFlydigi_UpdateDevice()` on `dev` at `now` (`SDL_GetTicks()`).
    fn update(
        &mut self,
        device: &mut DeviceCtx<'_>,
        dev: &dyn SteamHid,
        joystick: Option<JoystickID>,
        now: u64,
    ) -> bool {
        let is_v2 = device.vendor_id() == USB_VENDOR_FLYDIGI_V2;

        if is_v2 && joystick.is_some() && (self.next_heartbeat == 0 || now >= self.next_heartbeat) {
            // (SDL_HIDAPI_Flydigi_SendAcquireRequest() and
            // SDL_HIDAPI_Flydigi_SendInfoRequest(), whose failures are ignored)
            let _ = write_packet(device, dev, &acquire_command(true));
            let _ = write_packet(device, dev, &info_command());
            self.next_heartbeat = now + FLYDIGI_ACQUIRE_CONTROLLER_HEARTBEAT_TIME;
        }

        let mut data = [0u8; USB_PACKET_LENGTH];
        let read_error = loop {
            match dev.read_timeout(&mut data, 0) {
                Ok(0) => break false,
                Ok(size) => {
                    self.last_packet = now;

                    if is_v2 {
                        self.handle_packet_v2(device, dev, joystick, &data, size);
                    } else {
                        self.handle_packet_v1(device, joystick, &data, size);
                    }
                }
                Err(_) => break true,
            }
        };

        if is_v2 {
            // If we haven't gotten a packet in a while, check to make sure we can still acquire it
            const INPUT_TIMEOUT_MS: u64 = 100;
            if now >= self.last_packet + INPUT_TIMEOUT_MS {
                self.next_heartbeat = now;
            }
        }

        if read_error {
            if let Some(&first) = device.joysticks().first() {
                // Read error, device is disconnected
                device.joystick_disconnected(first);
            }
        }
        !read_error
    }

    /// `HIDAPI_DriverFlydigi_InitDevice()` on `dev`.
    fn init(&mut self, device: &mut DeviceCtx<'_>, dev: &dyn SteamHid) -> Result<()> {
        if device.vendor_id() == USB_VENDOR_FLYDIGI_V1 {
            self.init_controller_v1(device, dev);
            Ok(())
        } else {
            self.init_controller_v2(device, dev)
        }
    }
}

impl DriverContext for FlydigiContext {
    /// Translation of `HIDAPI_DriverFlydigi_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        let dev = device.device().clone();
        self.init(device, &*dev)
    }

    /// Translation of `HIDAPI_DriverFlydigi_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        let dev = device.device().clone();
        let joystick = device.open_joystick_id();
        self.update(device, &*dev, joystick, crate::timer::ticks_ms())
    }

    /// Translation of `HIDAPI_DriverFlydigi_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        self.last_state = [0; USB_PACKET_LENGTH];

        // Initialize the joystick capabilities
        let mut nbuttons = usize::from(NUM_BASE_FLYDIGI_BUTTONS);
        if self.has_cz {
            nbuttons += 2;
        }
        if self.has_lmrm {
            nbuttons += 2;
        }
        if self.has_circle {
            nbuttons += 1;
        }
        joystick.nbuttons = nbuttons;
        joystick.naxes = GamepadAxis::COUNT;
        joystick.nhats = 1;

        if self.wireless {
            joystick.connection_state = JoystickConnectionState::Wireless;
        }

        if self.sensors_supported {
            let sensor_rate = if self.wireless {
                SENSOR_INTERVAL_VADER4_PRO_DONGLE_RATE_HZ
            } else {
                SENSOR_INTERVAL_VADER4_PRO_WIRED_RATE_HZ
            } as f32;
            joystick.add_sensor(SensorType::Gyro, sensor_rate);
            joystick.add_sensor(SensorType::Accel, sensor_rate);
        }
        Ok(())
    }

    /// Translation of `HIDAPI_DriverFlydigi_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        let rumble_packet = rumble_packet(
            device.vendor_id(),
            low_frequency_rumble,
            high_frequency_rumble,
        );

        if send_rumble(device.device(), &rumble_packet).ok() != Some(rumble_packet.len()) {
            return Err(Error::new("Couldn't send rumble packet"));
        }
        Ok(())
    }

    /// Translation of `HIDAPI_DriverFlydigi_GetJoystickCapabilities()`.
    fn get_joystick_capabilities(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
    ) -> JoystickCaps {
        JoystickCaps::RUMBLE
    }

    /// Translation of `HIDAPI_DriverFlydigi_SetJoystickSensorsEnabled()`.
    fn set_joystick_sensors_enabled(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        enabled: bool,
    ) -> Result<()> {
        if self.sensors_supported {
            self.sensors_enabled = enabled;
            return Ok(());
        }
        Err(Error::unsupported())
    }

    /// Translation of `HIDAPI_DriverFlydigi_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {
        // Don't unacquire the controller, someone else might be using it too.
        // The controller will automatically unacquire itself after a little while
        //SDL_HIDAPI_Flydigi_SendAcquireRequest(device, false);
    }
}

#[cfg(test)]
mod tests;
