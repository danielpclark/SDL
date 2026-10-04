// Rust translation of src/joystick/hidapi/SDL_hidapi_8bitdo.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The 8BitDo controller driver (SF30 Pro, SN30 Pro, Pro 2, Pro 3 and the
//! Ultimate 2 Wireless and Ultimate 3), with the enhanced reports of their
//! newer firmware: sensors, battery and the extra buttons.

use super::ps4::{hat_of, read_feature_report};
use super::rumble::send_rumble;
use super::{
    load16, load32, remap_val, DeviceCtx, DriverContext, DriverImpl, HidapiDevice, JoystickCaps,
    NS_PER_US, SDL_HIDAPI_DEFAULT, USB_PACKET_LENGTH,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::JoystickData;
use crate::power::PowerState;
use crate::sensor::{SensorType, STANDARD_GRAVITY};

/// `SDL_GAMEPAD_BUTTON_8BITDO_L4`
const BUTTON_8BITDO_L4: u8 = 11;
/// `SDL_GAMEPAD_BUTTON_8BITDO_R4`
const BUTTON_8BITDO_R4: u8 = 12;
/// `SDL_GAMEPAD_BUTTON_8BITDO_PL`
const BUTTON_8BITDO_PL: u8 = 13;
/// `SDL_GAMEPAD_BUTTON_8BITDO_PR`
const BUTTON_8BITDO_PR: u8 = 14;
/// `SDL_GAMEPAD_NUM_8BITDO_BUTTONS`
const NUM_8BITDO_BUTTONS: usize = 15;

/// `SDL_GAMEPAD_BUTTON_8BITDO_SHARE`
const BUTTON_8BITDO_SHARE: u8 = 15;
/// `SDL_GAMEPAD_NUM_8BITDO_ULTIMATE3_BUTTONS`
const NUM_8BITDO_ULTIMATE3_BUTTONS: usize = 16;

const FEATURE_REPORTID: u8 = 0x30;
const FEATURE_REPORTID_ENABLE_SDL_REPORTID: u8 = 0x06;
const REPORTID_SDL_REPORTID: u8 = 0x04;
const REPORTID_NOT_SUPPORTED_SDL_REPORTID: u8 = 0x03;
const BT_REPORTID_SDL_REPORTID: u8 = 0x01;

const SENSOR_TIMESTAMP_ENABLE: u8 = 0xAA;
const ABITDO_ACCEL_SCALE: f32 = 4096.0;
const ABITDO_GYRO_MAX_DEGREES_PER_SECOND: f32 = 2000.0;

/// `SDL_NS_PER_SECOND`
const NS_PER_SECOND: u64 = 1_000_000_000;

/// `DEG2RAD()`
fn deg2rad(x: f32) -> f32 {
    x * (std::f32::consts::PI / 180.0)
}

/// The 8BitDo driver's static functions.
pub(crate) struct EightBitDoDriver;

impl DriverImpl for EightBitDoDriver {
    /// Translation of `HIDAPI_Driver8BitDo_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_8BITDO]
    }

    /// Translation of `HIDAPI_Driver8BitDo_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_8BITDO,
            hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
        )
    }

    /// Translation of `HIDAPI_Driver8BitDo_IsSupportedDevice()`.
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
        vendor_id == USB_VENDOR_8BITDO
            && matches!(
                product_id,
                USB_PRODUCT_8BITDO_SF30_PRO
                    | USB_PRODUCT_8BITDO_SF30_PRO_BT
                    | USB_PRODUCT_8BITDO_SN30_PRO
                    | USB_PRODUCT_8BITDO_SN30_PRO_BT
                    | USB_PRODUCT_8BITDO_PRO_2
                    | USB_PRODUCT_8BITDO_PRO_2_BT
                    | USB_PRODUCT_8BITDO_PRO_3
                    | USB_PRODUCT_8BITDO_ULTIMATE2_WIRELESS
                    | USB_PRODUCT_8BITDO_ULTIMATE3
            )
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(EightBitDoContext::default())
    }
}

/// Translation of `SDL_Driver8BitDo_Context` (without the fields upstream
/// never uses: `touchpad_01_supported`, `touchpad_02_supported`,
/// `rumble_type`, `player_led_supported`, `serial`, `version` and
/// `version_beta`).
#[derive(Debug)]
struct EightBitDoContext {
    sensors_supported: bool,
    sensors_enabled: bool,
    rumble_supported: bool,
    trigger_rumble_supported: bool,
    rgb_supported: bool,
    powerstate_supported: bool,
    sensor_timestamp_supported: bool,
    accel_scale: f32,
    gyro_scale: f32,
    last_state: [u8; USB_PACKET_LENGTH],
    /// Nanoseconds.  Simulate onboard clock. Different models have different rates vs different connection styles.
    sensor_timestamp: u64,
    sensor_timestamp_interval: u64,
    rumble_left_value: u8,
    rumble_right_value: u8,
    trigger_rumble_left_value: u8,
    trigger_rumble_right_value: u8,
    last_tick: u32,
    /// The number of buttons of the open joystick (`joystick->nbuttons`).
    nbuttons: usize,
}

impl Default for EightBitDoContext {
    fn default() -> Self {
        EightBitDoContext {
            sensors_supported: false,
            sensors_enabled: false,
            rumble_supported: false,
            trigger_rumble_supported: false,
            rgb_supported: false,
            powerstate_supported: false,
            sensor_timestamp_supported: false,
            accel_scale: 0.0,
            gyro_scale: 0.0,
            last_state: [0; USB_PACKET_LENGTH],
            sensor_timestamp: 0,
            sensor_timestamp_interval: 0,
            rumble_left_value: 0,
            rumble_right_value: 0,
            trigger_rumble_left_value: 0,
            trigger_rumble_right_value: 0,
            last_tick: 0,
            nbuttons: 0,
        }
    }
}

/// A Bluetooth MAC address as a serial number.
fn mac_serial(mac: [u8; 6]) -> String {
    format!(
        "{:02x}-{:02x}-{:02x}-{:02x}-{:02x}-{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

/// The name of a controller, if it has a better one than its USB name
/// (part of `HIDAPI_Driver8BitDo_InitDevice()`).
fn device_name(product_id: u16) -> Option<&'static str> {
    match product_id {
        USB_PRODUCT_8BITDO_SF30_PRO | USB_PRODUCT_8BITDO_SF30_PRO_BT => Some("8BitDo SF30 Pro"),
        USB_PRODUCT_8BITDO_SN30_PRO | USB_PRODUCT_8BITDO_SN30_PRO_BT => Some("8BitDo SN30 Pro"),
        USB_PRODUCT_8BITDO_PRO_2 | USB_PRODUCT_8BITDO_PRO_2_BT => Some("8BitDo Pro 2"),
        USB_PRODUCT_8BITDO_PRO_3 => Some("8BitDo Pro 3"),
        _ => None,
    }
}

/// A stick axis of a state packet (`READ_STICK_AXIS()`).
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

impl EightBitDoContext {
    /// The features of a `SDL_8BITDO_FEATURE_REPORTID` report of the
    /// Ultimate 3 (part of `HIDAPI_Driver8BitDo_InitDevice()`); its serial
    /// number, if it has one.
    fn apply_ultimate3_features(&mut self, data: &[u8], size: usize) -> Option<String> {
        if data[3] == 0 {
            self.rumble_supported = false;
            self.trigger_rumble_supported = false;
        }
        if data[3] & 0x02 != 0 {
            self.trigger_rumble_supported = true;
        }

        if data[4] == 0 {
            self.sensors_supported = false;
            self.sensor_timestamp_supported = false;
        }
        (size >= 17 && data[11] != 0)
            .then(|| mac_serial([data[16], data[15], data[14], data[13], data[12], data[11]]))
    }

    /// The features of a `SDL_8BITDO_FEATURE_REPORTID_ENABLE_SDL_REPORTID`
    /// report (part of `HIDAPI_Driver8BitDo_InitDevice()`); its serial
    /// number, if it has one.
    fn apply_features(&mut self, data: &[u8], size: usize) -> Option<String> {
        self.sensors_supported = true;
        self.rumble_supported = true;
        self.powerstate_supported = true;

        if size >= 14 && data[13] == SENSOR_TIMESTAMP_ENABLE {
            self.sensor_timestamp_supported = true;
        }

        // Set the serial number to the Bluetooth MAC address
        (size >= 12 && data[10] != 0)
            .then(|| mac_serial([data[10], data[9], data[8], data[7], data[6], data[5]]))
    }

    /// Translation of `HIDAPI_Driver8BitDo_GetIMURateForProductID()`.
    fn imu_rate(&self, product_id: u16, is_bluetooth: bool) -> u64 {
        // TODO: If sensor time stamp is sent, these fixed settings from observation can be replaced
        match product_id {
            USB_PRODUCT_8BITDO_SF30_PRO
            | USB_PRODUCT_8BITDO_SF30_PRO_BT
            | USB_PRODUCT_8BITDO_SN30_PRO
            | USB_PRODUCT_8BITDO_SN30_PRO_BT => {
                if is_bluetooth {
                    // Note, This is estimated by observation of Bluetooth packets received in the testcontroller tool
                    70 // Observed to be anywhere between 60-90 hz. Possibly lossy in current state
                } else if self.sensor_timestamp_supported {
                    // This firmware appears to update at 200 Hz over USB
                    200
                } else {
                    // This firmware appears to update at 100 Hz over USB
                    100
                }
            }
            USB_PRODUCT_8BITDO_PRO_2
            | USB_PRODUCT_8BITDO_PRO_2_BT // Note, labeled as "BT" but appears this way when wired.
            | USB_PRODUCT_8BITDO_PRO_3 => {
                if is_bluetooth {
                    // Note, This is estimated by observation of Bluetooth packets received in the testcontroller tool
                    85 // Observed Bluetooth packet rate seems to be 80-90hz
                } else if self.sensor_timestamp_supported {
                    // This firmware appears to update at 200 Hz over USB
                    200
                } else {
                    // This firmware appears to update at 100 Hz over USB
                    100
                }
            }
            USB_PRODUCT_8BITDO_ULTIMATE2_WIRELESS => {
                if is_bluetooth {
                    // Note, This is estimated by observation of Bluetooth packets received in the testcontroller tool
                    120 // Observed Bluetooth packet rate seems to be 120hz
                } else {
                    // This firmware appears to update at 1000 Hz over USB dongle
                    1000
                }
            }
            USB_PRODUCT_8BITDO_ULTIMATE3 => 120,
            _ => 120,
        }
    }

    /// The rumble report of `HIDAPI_Driver8BitDo_RumbleJoystick()`, if
    /// rumble is supported.
    fn rumble_packet(
        &mut self,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Option<[u8; 5]> {
        if !self.rumble_supported {
            return None;
        }

        let mut rumble_packet = [0x05, 0x00, 0x00, 0x00, 0x00];
        rumble_packet[1] = (low_frequency_rumble >> 8) as u8;
        rumble_packet[2] = (high_frequency_rumble >> 8) as u8;
        if self.trigger_rumble_supported {
            rumble_packet[3] = self.trigger_rumble_left_value;
            rumble_packet[4] = self.trigger_rumble_right_value;
            self.rumble_left_value = rumble_packet[1];
            self.rumble_right_value = rumble_packet[2];
        }
        Some(rumble_packet)
    }

    /// The rumble report of `HIDAPI_Driver8BitDo_RumbleJoystickTriggers()`,
    /// if trigger rumble is supported.
    fn trigger_rumble_packet(&mut self, left_rumble: u16, right_rumble: u16) -> Option<[u8; 5]> {
        if !self.trigger_rumble_supported {
            return None;
        }

        let mut rumble_packet = [0x05, 0x00, 0x00, 0x00, 0x00];
        rumble_packet[3] = (left_rumble >> 8) as u8;
        rumble_packet[4] = (right_rumble >> 8) as u8;
        if self.rumble_supported {
            rumble_packet[1] = self.rumble_left_value;
            rumble_packet[2] = self.rumble_right_value;
            self.trigger_rumble_left_value = rumble_packet[3];
            self.trigger_rumble_right_value = rumble_packet[4];
        }
        Some(rumble_packet)
    }

    /// Translation of `HIDAPI_Driver8BitDo_HandleOldStatePacket()`.
    fn handle_old_state_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        let timestamp = crate::timer::ticks_ns();

        if self.last_state[2] != data[2] {
            device.send_hat(timestamp, joystick, 0, hat_of(data[2]));
        }

        if self.last_state[0] != data[0] {
            let mut button = |button: GamepadButton, down: bool| {
                device.send_button(timestamp, joystick, button as u8, down);
            };
            button(GamepadButton::South, data[0] & 0x01 != 0);
            button(GamepadButton::East, data[0] & 0x02 != 0);
            button(GamepadButton::West, data[0] & 0x08 != 0);
            button(GamepadButton::North, data[0] & 0x10 != 0);
            button(GamepadButton::LeftShoulder, data[0] & 0x40 != 0);
            button(GamepadButton::RightShoulder, data[0] & 0x80 != 0);
        }

        if self.last_state[1] != data[1] {
            let mut button = |button: GamepadButton, down: bool| {
                device.send_button(timestamp, joystick, button as u8, down);
            };
            button(GamepadButton::Guide, data[1] & 0x10 != 0);
            button(GamepadButton::Back, data[1] & 0x04 != 0);
            button(GamepadButton::Start, data[1] & 0x08 != 0);
            button(GamepadButton::LeftStick, data[1] & 0x20 != 0);
            button(GamepadButton::RightStick, data[1] & 0x40 != 0);

            let trigger = |down: bool| if down { i16::MAX } else { i16::MIN };
            device.send_axis(
                timestamp,
                joystick,
                GamepadAxis::LeftTrigger as u8,
                trigger(data[1] & 0x01 != 0),
            );
            device.send_axis(
                timestamp,
                joystick,
                GamepadAxis::RightTrigger as u8,
                trigger(data[1] & 0x02 != 0),
            );
        }

        let mut axis = |axis: GamepadAxis, value: i16| {
            device.send_axis(timestamp, joystick, axis as u8, value);
        };
        axis(GamepadAxis::LeftX, read_stick_axis(data[3]));
        axis(GamepadAxis::LeftY, read_stick_axis(data[4]));
        axis(GamepadAxis::RightX, read_stick_axis(data[5]));
        axis(GamepadAxis::RightY, read_stick_axis(data[6]));

        let n = size.min(self.last_state.len());
        self.last_state[..n].copy_from_slice(&data[..n]);
    }

    /// Translation of `HIDAPI_Driver8BitDo_HandleStatePacket()`.
    fn handle_state_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        let timestamp = crate::timer::ticks_ns();

        match data[0] {
            REPORTID_NOT_SUPPORTED_SDL_REPORTID // Firmware without enhanced mode
            | REPORTID_SDL_REPORTID // Enhanced mode USB report
            | BT_REPORTID_SDL_REPORTID => {} // Enhanced mode Bluetooth report
            _ => {
                // We don't know how to handle this report
                return;
            }
        }

        if self.last_state[1] != data[1] {
            device.send_hat(timestamp, joystick, 0, hat_of(data[1]));
        }

        let nbuttons = self.nbuttons;
        let mut button = |button: u8, down: bool| {
            device.send_button(timestamp, joystick, button, down);
        };
        if self.last_state[8] != data[8] {
            button(GamepadButton::South as u8, data[8] & 0x01 != 0);
            button(GamepadButton::East as u8, data[8] & 0x02 != 0);
            button(GamepadButton::West as u8, data[8] & 0x08 != 0);
            button(GamepadButton::North as u8, data[8] & 0x10 != 0);
            button(GamepadButton::LeftShoulder as u8, data[8] & 0x40 != 0);
            button(GamepadButton::RightShoulder as u8, data[8] & 0x80 != 0);

            button(BUTTON_8BITDO_PL, data[8] & 0x20 != 0);
            button(BUTTON_8BITDO_PR, data[8] & 0x04 != 0);
        }

        if self.last_state[9] != data[9] {
            button(GamepadButton::Guide as u8, data[9] & 0x10 != 0);
            button(GamepadButton::Back as u8, data[9] & 0x04 != 0);
            button(GamepadButton::Start as u8, data[9] & 0x08 != 0);
            button(GamepadButton::LeftStick as u8, data[9] & 0x20 != 0);
            button(GamepadButton::RightStick as u8, data[9] & 0x40 != 0);
            if nbuttons >= NUM_8BITDO_ULTIMATE3_BUTTONS {
                button(BUTTON_8BITDO_SHARE, data[9] & 0x80 != 0);
            }
        }

        if size > 10 && self.last_state[10] != data[10] {
            button(BUTTON_8BITDO_L4, data[10] & 0x01 != 0);
            button(BUTTON_8BITDO_R4, data[10] & 0x02 != 0);
        }

        let mut axis = |axis: GamepadAxis, value: i16| {
            device.send_axis(timestamp, joystick, axis as u8, value);
        };
        axis(GamepadAxis::LeftX, read_stick_axis(data[2]));
        axis(GamepadAxis::LeftY, read_stick_axis(data[3]));
        axis(GamepadAxis::RightX, read_stick_axis(data[4]));
        axis(GamepadAxis::RightY, read_stick_axis(data[5]));

        axis(GamepadAxis::LeftTrigger, read_trigger_axis(data[7]));
        axis(GamepadAxis::RightTrigger, read_trigger_axis(data[6]));

        if self.powerstate_supported {
            let mut status = data[14] >> 7;
            let level = data[14] & 0x7f;
            if level == 100 {
                status = 2;
            }
            let (state, percent) = match status {
                0 => (PowerState::OnBattery, i32::from(level)),
                1 => (PowerState::Charging, i32::from(level)),
                2 => (PowerState::Charged, 100),
                _ => (PowerState::Unknown, 0),
            };
            device.send_power_info(joystick, state, percent);
        }

        if self.sensors_enabled {
            // (the ABITDO_SENSORS at data[15])
            let sensor = |offset: usize| i32::from(load16(data[15 + offset], data[16 + offset]));
            let (accel_x, accel_y, accel_z) = (sensor(0), sensor(2), sensor(4));
            let (gyro_x, gyro_y, gyro_z) = (sensor(6), sensor(8), sensor(10));

            if self.sensor_timestamp_supported {
                let tick = load32(data[27], data[28], data[29], data[30]);

                if self.last_tick != 0 {
                    let delta = if self.last_tick <= tick {
                        tick - self.last_tick
                    } else {
                        (u32::MAX - self.last_tick)
                            .wrapping_add(tick)
                            .wrapping_add(1)
                    };
                    // Sanity check the delta value
                    if delta < 100000 {
                        self.sensor_timestamp_interval = u64::from(delta) * NS_PER_US;
                    }
                }
                self.last_tick = tick;
            }

            // Note: we cannot use the time stamp of the receiving computer due to packet delay creating "spiky" timings.
            // The imu time stamp is intended to be the sample time of the on-board hardware.
            // In the absence of time stamp data from the data[], we can simulate that by
            // advancing a time stamp by the observed/known imu clock rate. This is 8ms = 125 Hz
            let sensor_timestamp = self.sensor_timestamp;
            self.sensor_timestamp = self
                .sensor_timestamp
                .wrapping_add(self.sensor_timestamp_interval);

            // This device's IMU values are reported differently from SDL
            // Thus we perform a rotation of the coordinate system to match the SDL standard.

            // By observation of this device:
            // Hardware x is reporting roll (rotation about the power jack's axis)
            // Hardware y is reporting pitch (rotation about the horizontal axis)
            // Hardware z is reporting yaw (rotation about the joysticks' center axis)
            let values = [
                (-gyro_y) as f32 * self.gyro_scale, // Rotation around pitch axis
                gyro_z as f32 * self.gyro_scale,    // Rotation around yaw axis
                (-gyro_x) as f32 * self.gyro_scale, // Rotation around roll axis
            ];
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Gyro,
                sensor_timestamp,
                &values,
            );

            // By observation of this device:
            // Accelerometer X is positive when front of the controller points toward the sky.
            // Accelerometer y is positive when left side of the controller points toward the sky.
            // Accelerometer Z is positive when sticks point toward the sky.
            let values = [
                (-accel_y) as f32 * self.accel_scale, // Acceleration along pitch axis
                accel_z as f32 * self.accel_scale,    // Acceleration along yaw axis
                (-accel_x) as f32 * self.accel_scale, // Acceleration along roll axis
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

    /// One report of `HIDAPI_Driver8BitDo_UpdateDevice()`.
    fn handle_report(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        if size == 9 {
            // Old firmware USB report for the SF30 Pro and SN30 Pro controllers
            self.handle_old_state_packet(device, joystick, data, size);
        } else {
            self.handle_state_packet(device, joystick, data, size);
        }
    }
}

impl DriverContext for EightBitDoContext {
    /// Translation of `HIDAPI_Driver8BitDo_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        if device.product_id() == USB_PRODUCT_8BITDO_ULTIMATE2_WIRELESS {
            // The Ultimate 2 Wireless v1.02 firmware has 12 byte reports, v1.03 firmware has 34 byte reports
            const ULTIMATE2_WIRELESS_V103_REPORT_SIZE: usize = 34;
            const MAX_ATTEMPTS: usize = 3;

            for _ in 0..MAX_ATTEMPTS {
                let mut data = [0u8; USB_PACKET_LENGTH];
                match device.read_timeout(&mut data, 80) {
                    Ok(0) => {
                        // Try again
                        continue;
                    }
                    Ok(size) if size >= ULTIMATE2_WIRELESS_V103_REPORT_SIZE => {
                        self.sensors_supported = true;
                        self.rumble_supported = true;
                        self.powerstate_supported = true;
                    }
                    _ => {}
                }
                break;
            }
        } else if device.product_id() == USB_PRODUCT_8BITDO_ULTIMATE3 {
            // Supported by default
            self.sensors_supported = true;
            self.rumble_supported = true;
            self.powerstate_supported = true;
            self.sensor_timestamp_supported = true;
            let mut data = [0u8; USB_PACKET_LENGTH];
            const MAX_ATTEMPTS: usize = 5;
            for _ in 0..MAX_ATTEMPTS {
                if let Ok(size @ 1..) = read_feature_report(device, FEATURE_REPORTID, &mut data) {
                    if let Some(serial) = self.apply_ultimate3_features(&data, size) {
                        device.set_device_serial(&serial);
                    }
                    break;
                }
                crate::timer::delay(std::time::Duration::from_millis(10));
            }
        } else {
            let mut data = [0u8; USB_PACKET_LENGTH];
            const MAX_ATTEMPTS: usize = 5;
            for _ in 0..MAX_ATTEMPTS {
                if let Ok(size @ 1..) =
                    read_feature_report(device, FEATURE_REPORTID_ENABLE_SDL_REPORTID, &mut data)
                {
                    if let Some(serial) = self.apply_features(&data, size) {
                        device.set_device_serial(&serial);
                    }
                    break;
                }

                // Try again
                crate::timer::delay(std::time::Duration::from_millis(10));
            }
        }

        if let Some(name) = device_name(device.product_id()) {
            device.set_device_name(name);
        }

        device.joystick_connected();
        Ok(())
    }

    /// Translation of `HIDAPI_Driver8BitDo_UpdateDevice()`.
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

    /// Translation of `HIDAPI_Driver8BitDo_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        self.last_state = [0; USB_PACKET_LENGTH];

        // Initialize the joystick capabilities
        joystick.nbuttons = match device.product_id() {
            USB_PRODUCT_8BITDO_PRO_2
            | USB_PRODUCT_8BITDO_PRO_2_BT
            | USB_PRODUCT_8BITDO_PRO_3
            | USB_PRODUCT_8BITDO_ULTIMATE2_WIRELESS => {
                // This controller has additional buttons
                NUM_8BITDO_BUTTONS
            }
            USB_PRODUCT_8BITDO_ULTIMATE3 => NUM_8BITDO_ULTIMATE3_BUTTONS,
            _ => 11,
        };
        self.nbuttons = joystick.nbuttons;
        joystick.naxes = GamepadAxis::COUNT;
        joystick.nhats = 1;

        if self.sensors_supported {
            // Different 8Bitdo controllers in different connection modes have different polling rates.
            let imu_polling_rate = self.imu_rate(device.product_id(), device.is_bluetooth());
            self.sensor_timestamp_interval = NS_PER_SECOND / imu_polling_rate;

            joystick.add_sensor(SensorType::Gyro, imu_polling_rate as f32);
            joystick.add_sensor(SensorType::Accel, imu_polling_rate as f32);

            self.accel_scale = STANDARD_GRAVITY / ABITDO_ACCEL_SCALE;
            // Hardware senses +/- N Degrees per second mapped to +/- INT16_MAX
            self.gyro_scale = deg2rad(ABITDO_GYRO_MAX_DEGREES_PER_SECOND) / f32::from(i16::MAX);
        }

        Ok(())
    }

    /// Translation of `HIDAPI_Driver8BitDo_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        let Some(rumble_packet) = self.rumble_packet(low_frequency_rumble, high_frequency_rumble)
        else {
            return Err(Error::unsupported());
        };

        if send_rumble(device.device(), &rumble_packet).ok() != Some(rumble_packet.len()) {
            return Err(Error::new("Couldn't send rumble packet"));
        }
        Ok(())
    }

    /// Translation of `HIDAPI_Driver8BitDo_RumbleJoystickTriggers()`.
    fn rumble_joystick_triggers(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        left_rumble: u16,
        right_rumble: u16,
    ) -> Result<()> {
        let Some(rumble_packet) = self.trigger_rumble_packet(left_rumble, right_rumble) else {
            return Err(Error::unsupported());
        };

        if send_rumble(device.device(), &rumble_packet).ok() != Some(rumble_packet.len()) {
            return Err(Error::new("Couldn't send rumble packet"));
        }
        Ok(())
    }

    /// Translation of `HIDAPI_Driver8BitDo_GetJoystickCapabilities()`.
    fn get_joystick_capabilities(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
    ) -> JoystickCaps {
        let mut caps = JoystickCaps(0);
        if self.rumble_supported {
            caps |= JoystickCaps::RUMBLE;
        }
        if self.rgb_supported {
            caps |= JoystickCaps::RGB_LED;
        }
        caps
    }

    /// Translation of `HIDAPI_Driver8BitDo_SetJoystickSensorsEnabled()`.
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

    /// Translation of `HIDAPI_Driver8BitDo_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {}
}

#[cfg(test)]
mod tests;
