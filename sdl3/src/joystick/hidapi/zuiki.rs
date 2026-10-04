// Rust translation of src/joystick/hidapi/SDL_hidapi_zuiki.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The ZUIKI controller driver (the MASCON PRO and the EVOTOP
//! controllers).

use super::ps4::hat_of;
use super::rumble::send_rumble;
use super::{
    load16, remap_val, DeviceCtx, DriverContext, DriverImpl, HidapiDevice, JoystickCaps,
    SDL_HIDAPI_DEFAULT, USB_PACKET_LENGTH,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::JoystickData;
use crate::sensor::{SensorType, STANDARD_GRAVITY};

/// Calculate scaling factor based on gyroscope data range and radians
const GYRO_SCALE: f32 = 1024.0 / 32768.0 * std::f32::consts::PI / 180.0;
/// Calculate acceleration scaling factor based on gyroscope data range and standard gravity
const ACCEL_SCALE: f32 = 8.0 / 32768.0 * STANDARD_GRAVITY;
/// Must be an odd number
const FILTER_SIZE: usize = 11;
/// zuiki device initialization retry count
const MAX_RETRY_COUNT: u8 = 10;

/// Translation of `MedianFilter_t`.
#[derive(Clone, Copy, Debug, Default)]
struct MedianFilter {
    buffer: [f32; FILTER_SIZE],
    index: usize,
    count: usize,
}

impl MedianFilter {
    /// Translation of `median_filter_update()`.
    fn update(&mut self, input: f32) -> f32 {
        self.buffer[self.index] = input;
        self.index = (self.index + 1) % FILTER_SIZE;
        if self.count < FILTER_SIZE {
            self.count += 1;
        }
        let mut temp = self.buffer;
        // (upstream's exchange sort of the values so far)
        for i in 0..self.count - 1 {
            for j in i + 1..self.count {
                if temp[i] > temp[j] {
                    temp.swap(i, j);
                }
            }
        }
        temp[self.count / 2]
    }
}

/// The ZUIKI driver's static functions.
pub(crate) struct ZuikiDriver;

impl DriverImpl for ZuikiDriver {
    /// Translation of `HIDAPI_DriverZUIKI_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_ZUIKI]
    }

    /// Translation of `HIDAPI_DriverZUIKI_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_ZUIKI,
            hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
        )
    }

    /// Translation of `HIDAPI_DriverZUIKI_IsSupportedDevice()`.
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
        vendor_id == USB_VENDOR_ZUIKI
            && matches!(
                product_id,
                USB_PRODUCT_ZUIKI_MASCON_PRO
                    | USB_PRODUCT_ZUIKI_EVOTOP_UWB_DINPUT
                    | USB_PRODUCT_ZUIKI_EVOTOP_PC_DINPUT
                    | USB_PRODUCT_ZUIKI_EVOTOP_PC_BT
                    | USB_PRODUCT_ZUIKI_EVOTOP_AXIS_DINPUT
            )
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(ZuikiContext::default())
    }
}

/// Translation of `SDL_DriverZUIKI_Context` (without its
/// `sensor_timestamp_ns`, which upstream never uses).
#[derive(Debug)]
struct ZuikiContext {
    last_state: [u8; USB_PACKET_LENGTH],
    /// Sensor enabled status flag
    sensors_supported: bool,
    sensor_rate: f32,
    filter_gyro_x: MedianFilter,
    filter_gyro_y: MedianFilter,
    filter_gyro_z: MedianFilter,
}

impl Default for ZuikiContext {
    fn default() -> Self {
        ZuikiContext {
            last_state: [0; USB_PACKET_LENGTH],
            sensors_supported: false,
            sensor_rate: 0.0,
            filter_gyro_x: MedianFilter::default(),
            filter_gyro_y: MedianFilter::default(),
            filter_gyro_z: MedianFilter::default(),
        }
    }
}

/// The rumble report of `HIDAPI_DriverZUIKI_RumbleJoystick()`.
fn rumble_packet(low_frequency_rumble: u16, high_frequency_rumble: u16) -> [u8; 8] {
    let mut rumble_packet = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    rumble_packet[4] = (low_frequency_rumble >> 8) as u8;
    rumble_packet[5] = (high_frequency_rumble >> 8) as u8;
    rumble_packet
}

/// A stick axis of an old state packet (`READ_STICK_AXIS()`).
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

/// An axis of an EVOTOP Bluetooth state packet.
fn read_bt_axis(lo: u8, hi: u8, max: u16) -> i16 {
    // Note (upstream): a trigger value above 0x3ff maps beyond the i16
    // range, which is undefined behaviour in C (it wraps on x86); the cast
    // here saturates.
    remap_val(
        f32::from(u16::from_le_bytes([lo, hi])),
        0x0000 as f32,
        f32::from(max),
        f32::from(i16::MIN),
        f32::from(i16::MAX),
    ) as i16
}

impl ZuikiContext {
    /// The sensor support of a controller, from its first report (part of
    /// `HIDAPI_DriverZUIKI_InitDevice()`); its name, if it has a better one
    /// than its USB name.
    fn apply_first_report(&mut self, product_id: u16, data: &[u8]) -> Option<&'static str> {
        match product_id {
            USB_PRODUCT_ZUIKI_MASCON_PRO => Some("ZUIKI MASCON PRO"),
            USB_PRODUCT_ZUIKI_EVOTOP_PC_DINPUT => {
                self.sensors_supported = true;
                self.sensor_rate = 200.0;
                None
            }
            USB_PRODUCT_ZUIKI_EVOTOP_UWB_DINPUT => {
                self.sensors_supported = true;
                self.sensor_rate = 100.0;
                None
            }
            USB_PRODUCT_ZUIKI_EVOTOP_PC_BT => {
                if data[16] != 0 {
                    self.sensors_supported = true;
                    self.sensor_rate = 50.0;
                }
                Some("ZUIKI EVOTOP")
            }
            USB_PRODUCT_ZUIKI_EVOTOP_AXIS_DINPUT => Some("ZUIKI EVOTOP AXIS"),
            _ => None,
        }
    }

    /// Translation of `HIDAPI_DriverZUIKI_HandleOldStatePacket()`.
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
            button(GamepadButton::North, data[0] & 0x01 != 0);
            button(GamepadButton::East, data[0] & 0x02 != 0);
            button(GamepadButton::South, data[0] & 0x04 != 0);
            button(GamepadButton::West, data[0] & 0x08 != 0);
            button(GamepadButton::LeftShoulder, data[0] & 0x10 != 0);
            button(GamepadButton::RightShoulder, data[0] & 0x20 != 0);
            let trigger = |down: bool| if down { i16::MAX } else { i16::MIN };
            device.send_axis(
                timestamp,
                joystick,
                GamepadAxis::LeftTrigger as u8,
                trigger(data[0] & 0x40 != 0),
            );
            device.send_axis(
                timestamp,
                joystick,
                GamepadAxis::RightTrigger as u8,
                trigger(data[0] & 0x80 != 0),
            );
        }

        if self.last_state[1] != data[1] {
            let mut button = |button: GamepadButton, down: bool| {
                device.send_button(timestamp, joystick, button as u8, down);
            };
            button(GamepadButton::Back, data[1] & 0x01 != 0);
            button(GamepadButton::Start, data[1] & 0x02 != 0);
            button(GamepadButton::LeftStick, data[1] & 0x04 != 0);
            button(GamepadButton::RightStick, data[1] & 0x08 != 0);
            button(GamepadButton::Guide, data[1] & 0x10 != 0);
            button(GamepadButton::Misc1, data[1] & 0x20 != 0);
            /* todo for switch C key */
        }

        let mut axis = |axis: GamepadAxis, value: i16| {
            device.send_axis(timestamp, joystick, axis as u8, value);
        };
        axis(GamepadAxis::LeftX, read_stick_axis(data[3]));
        axis(GamepadAxis::LeftY, read_stick_axis(data[4]));
        axis(GamepadAxis::RightX, read_stick_axis(data[5]));
        axis(GamepadAxis::RightY, read_stick_axis(data[6]));

        if self.sensors_supported {
            let sensor_timestamp = timestamp;
            let raw = |offset: usize| f32::from(load16(data[offset], data[offset + 1]));
            let gyro_values = [
                self.filter_gyro_x.update(raw(8) * GYRO_SCALE),
                self.filter_gyro_y.update(raw(12) * GYRO_SCALE),
                self.filter_gyro_z.update(-raw(10) * GYRO_SCALE),
            ];
            let accel_values = [
                raw(14) * ACCEL_SCALE,
                raw(18) * ACCEL_SCALE,
                -raw(16) * ACCEL_SCALE,
            ];

            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Gyro,
                sensor_timestamp,
                &gyro_values,
            );
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Accel,
                sensor_timestamp,
                &accel_values,
            );
        }

        let n = size.min(self.last_state.len());
        self.last_state[..n].copy_from_slice(&data[..n]);
    }

    /// Translation of `HIDAPI_DriverZUIKI_Handle_EVOTOP_PCBT_StatePacket()`.
    fn handle_evotop_pcbt_state_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        let timestamp = crate::timer::ticks_ns();

        let mut axis = |axis: GamepadAxis, value: i16| {
            device.send_axis(timestamp, joystick, axis as u8, value);
        };
        axis(GamepadAxis::LeftX, read_bt_axis(data[1], data[2], 0xffff));
        axis(GamepadAxis::LeftY, read_bt_axis(data[3], data[4], 0xffff));
        axis(GamepadAxis::RightX, read_bt_axis(data[5], data[6], 0xffff));
        axis(GamepadAxis::RightY, read_bt_axis(data[7], data[8], 0xffff));

        axis(
            GamepadAxis::LeftTrigger,
            read_bt_axis(data[9], data[10], 0x03ff),
        );
        axis(
            GamepadAxis::RightTrigger,
            read_bt_axis(data[11], data[12], 0x03ff),
        );

        if self.last_state[13] != data[13] {
            // (the hat values start at 1 for up; 0 is centered)
            device.send_hat(timestamp, joystick, 0, hat_of(data[13].wrapping_sub(1)));
        }
        let mut button = |button: GamepadButton, down: bool| {
            device.send_button(timestamp, joystick, button as u8, down);
        };
        if self.last_state[14] != data[14] {
            button(GamepadButton::South, data[14] & 0x01 != 0);
            button(GamepadButton::East, data[14] & 0x02 != 0);
            button(GamepadButton::West, data[14] & 0x08 != 0);
            button(GamepadButton::North, data[14] & 0x10 != 0);
            button(GamepadButton::LeftShoulder, data[14] & 0x40 != 0);
            button(GamepadButton::RightShoulder, data[14] & 0x80 != 0);
        }

        if self.last_state[15] != data[15] {
            button(GamepadButton::Back, data[15] & 0x04 != 0);
            button(GamepadButton::Start, data[15] & 0x08 != 0);
            button(GamepadButton::Guide, data[15] & 0x10 != 0);
            button(GamepadButton::LeftStick, data[15] & 0x20 != 0);
            button(GamepadButton::RightStick, data[15] & 0x40 != 0);
        }

        if self.sensors_supported {
            let sensor_timestamp = timestamp;
            let raw = |offset: usize| f32::from(load16(data[offset], data[offset + 1]));
            let gyro_values = [
                self.filter_gyro_x.update(raw(17) * GYRO_SCALE),
                self.filter_gyro_y.update(raw(21) * GYRO_SCALE),
                self.filter_gyro_z.update(-raw(19) * GYRO_SCALE),
            ];
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Gyro,
                sensor_timestamp,
                &gyro_values,
            );
            let accel_values = [
                raw(23) * ACCEL_SCALE,
                raw(27) * ACCEL_SCALE,
                -raw(25) * ACCEL_SCALE,
            ];
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Accel,
                sensor_timestamp,
                &accel_values,
            );
        }

        let n = size.min(self.last_state.len());
        self.last_state[..n].copy_from_slice(&data[..n]);
    }

    /// One report of `HIDAPI_DriverZUIKI_UpdateDevice()`.
    fn handle_report(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        match device.product_id() {
            USB_PRODUCT_ZUIKI_EVOTOP_PC_BT => {
                self.handle_evotop_pcbt_state_packet(device, joystick, data, size)
            }
            USB_PRODUCT_ZUIKI_EVOTOP_PC_DINPUT
            | USB_PRODUCT_ZUIKI_MASCON_PRO
            | USB_PRODUCT_ZUIKI_EVOTOP_UWB_DINPUT
            | USB_PRODUCT_ZUIKI_EVOTOP_AXIS_DINPUT => {
                self.handle_old_state_packet(device, joystick, data, size)
            }
            _ => {}
        }
    }
}

impl DriverContext for ZuikiContext {
    /// Translation of `HIDAPI_DriverZUIKI_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        let mut data = [0u8; USB_PACKET_LENGTH * 2];
        self.sensors_supported = false;

        // Read report data once for device initialization
        let mut size = None;
        for _ in 0..MAX_RETRY_COUNT {
            if let Ok(n @ 1..) = device.read_timeout(&mut data, 10) {
                size = Some(n);
                break;
            }
        }
        if size.is_none() {
            return Err(Error::new("Couldn't read the first ZUIKI report"));
        }

        if let Some(name) = self.apply_first_report(device.product_id(), &data) {
            device.set_device_name(name);
        }

        device.joystick_connected();
        Ok(())
    }

    /// Translation of `HIDAPI_DriverZUIKI_UpdateDevice()`.
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

    /// Translation of `HIDAPI_DriverZUIKI_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        self.last_state = [0; USB_PACKET_LENGTH];

        joystick.nbuttons = 11;
        joystick.naxes = GamepadAxis::COUNT;
        joystick.nhats = 1;
        if self.sensors_supported {
            joystick.add_sensor(SensorType::Gyro, self.sensor_rate);
            joystick.add_sensor(SensorType::Accel, self.sensor_rate);
        }

        Ok(())
    }

    /// Translation of `HIDAPI_DriverZUIKI_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        let rumble_packet = rumble_packet(low_frequency_rumble, high_frequency_rumble);
        if send_rumble(device.device(), &rumble_packet).ok() != Some(rumble_packet.len()) {
            return Err(Error::new("Couldn't send rumble packet"));
        }
        Ok(())
    }

    /// Translation of `HIDAPI_DriverZUIKI_GetJoystickCapabilities()`.
    fn get_joystick_capabilities(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
    ) -> JoystickCaps {
        let mut caps = JoystickCaps(0);
        if device.product_id() != USB_PRODUCT_ZUIKI_EVOTOP_AXIS_DINPUT {
            caps |= JoystickCaps::RUMBLE;
        }
        caps
    }

    /// Translation of `HIDAPI_DriverZUIKI_SendJoystickEffect()`.
    fn send_joystick_effect(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        data: &[u8],
    ) -> Result<()> {
        if send_rumble(device.device(), data).ok() != Some(data.len()) {
            return Err(Error::new("Couldn't send rumble packet"));
        }
        Ok(())
    }

    /// Translation of `HIDAPI_DriverZUIKI_SetJoystickSensorsEnabled()`.
    fn set_joystick_sensors_enabled(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        _enabled: bool,
    ) -> Result<()> {
        if self.sensors_supported {
            return Ok(());
        }
        Err(Error::unsupported())
    }

    /// Translation of `HIDAPI_DriverZUIKI_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {}
}

#[cfg(test)]
mod tests;
