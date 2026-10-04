// Rust translation of src/joystick/hidapi/SDL_hidapi_steam_hori.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Wireless HORIPAD For Steam driver.

use super::steam::remap_val_clamped;
use super::{
    load16, remap_val, DeviceCtx, DriverContext, DriverImpl, HidapiDevice, SDL_HIDAPI_DEFAULT,
    USB_PACKET_LENGTH,
};
use crate::error::Result;
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadCapSenseType, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::{
    is_joystick_hori_steam_controller, with_joystick, JoystickData, HAT_CENTERED, HAT_DOWN,
    HAT_LEFT, HAT_LEFTDOWN, HAT_LEFTUP, HAT_RIGHT, HAT_RIGHTDOWN, HAT_RIGHTUP, HAT_UP,
};
use crate::power::PowerState;
use crate::sensor::{SensorType, STANDARD_GRAVITY};

/// `SDL_GAMEPAD_BUTTON_HORI_QAM`
const SDL_GAMEPAD_BUTTON_HORI_QAM: u8 = 11;
/// `SDL_GAMEPAD_BUTTON_HORI_FR`
const SDL_GAMEPAD_BUTTON_HORI_FR: u8 = 12;
/// `SDL_GAMEPAD_BUTTON_HORI_FL`
const SDL_GAMEPAD_BUTTON_HORI_FL: u8 = 13;
/// `SDL_GAMEPAD_BUTTON_HORI_M1`
const SDL_GAMEPAD_BUTTON_HORI_M1: u8 = 14;
/// `SDL_GAMEPAD_BUTTON_HORI_M2`
const SDL_GAMEPAD_BUTTON_HORI_M2: u8 = 15;
/// `SDL_GAMEPAD_NUM_HORI_BUTTONS`
const SDL_GAMEPAD_NUM_HORI_BUTTONS: usize = 16;

/// Translation of `SDL_DriverSteamHori_Context`.
#[derive(Debug)]
pub(crate) struct SteamHoriContext {
    last_state: [u8; USB_PACKET_LENGTH],
    sensor_ticks: u64,
    last_tick: u32,
    simulated_sensor_step_ns: u64,
    simulated_sensor_time_stamp: u64,
    wireless: bool,
    serial_needs_init: bool,
}

impl Default for SteamHoriContext {
    fn default() -> Self {
        SteamHoriContext {
            last_state: [0; USB_PACKET_LENGTH],
            sensor_ticks: 0,
            last_tick: 0,
            simulated_sensor_step_ns: 0,
            simulated_sensor_time_stamp: 0,
            wireless: false,
            serial_needs_init: false,
        }
    }
}

/// The Steam HORI driver's static functions.
pub(crate) struct SteamHoriDriver;

impl DriverImpl for SteamHoriDriver {
    /// Translation of `HIDAPI_DriverSteamHori_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_STEAM_HORI]
    }

    /// Translation of `HIDAPI_DriverSteamHori_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_STEAM_HORI,
            hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
        )
    }

    /// Translation of `HIDAPI_DriverSteamHori_IsSupportedDevice()`.
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
        is_joystick_hori_steam_controller(vendor_id, product_id)
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(SteamHoriContext::default())
    }
}

/// `DEG2RAD()`
fn deg2rad(x: f32) -> f32 {
    x * (std::f32::consts::PI / 180.0)
}

const REPORT_HEADER_USB: u8 = 0x07;
const REPORT_HEADER_BT: u8 = 0x00;

/// `READ_STICK_AXIS()`
fn read_stick_axis(value: u8) -> i16 {
    if value == 0x80 {
        0
    } else {
        remap_val(
            (i32::from(value) - 0x80) as f32,
            -0x80 as f32,
            (0xff - 0x80) as f32,
            f32::from(i16::MIN),
            f32::from(i16::MAX),
        ) as i16
    }
}

/// `READ_TRIGGER_AXIS()`
fn read_trigger_axis(value: u8) -> i16 {
    ((i32::from(value) * 257) - 32768) as i16
}

/// The serial number in a USB state report.
fn usb_serial(data: &[u8]) -> String {
    format!(
        "{:02x}-{:02x}-{:02x}-{:02x}-{:02x}-{:02x}",
        data[38], data[39], data[40], data[41], data[42], data[43]
    )
}

impl SteamHoriContext {
    /// Translation of `HIDAPI_DriverSteamHori_HandleStatePacket()`.
    fn handle_state_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        let timestamp = crate::timer::ticks_ns();

        // Make sure it's gamepad state and not OTA FW update info
        if data[0] != REPORT_HEADER_USB && data[0] != REPORT_HEADER_BT {
            /* We don't know how to handle this report */
            return;
        }

        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftX as u8,
            read_stick_axis(data[1]),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftY as u8,
            read_stick_axis(data[2]),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightX as u8,
            read_stick_axis(data[3]),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightY as u8,
            read_stick_axis(data[4]),
        );

        if self.last_state[5] != data[5] {
            let hat = match data[5] & 0xF {
                0 => HAT_UP,
                1 => HAT_RIGHTUP,
                2 => HAT_RIGHT,
                3 => HAT_RIGHTDOWN,
                4 => HAT_DOWN,
                5 => HAT_LEFTDOWN,
                6 => HAT_LEFT,
                7 => HAT_LEFTUP,
                _ => HAT_CENTERED,
            };
            device.send_hat(timestamp, joystick, 0, hat);
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::South as u8,
                data[5] & 0x10 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::East as u8,
                data[5] & 0x20 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_HORI_QAM,
                data[5] & 0x40 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::West as u8,
                data[5] & 0x80 != 0,
            );
        }

        if self.last_state[6] != data[6] {
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::North as u8,
                data[6] & 0x01 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_HORI_M1, /* M1 */
                data[6] & 0x02 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::LeftShoulder as u8,
                data[6] & 0x04 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::RightShoulder as u8,
                data[6] & 0x08 != 0,
            );

            // TODO: can we handle the digital trigger mode? The data seems to come through analog regardless of the trigger state
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Back as u8,
                data[6] & 0x40 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Start as u8,
                data[6] & 0x80 != 0,
            );
        }

        if self.last_state[7] != data[7] {
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Guide as u8,
                data[7] & 0x01 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::LeftStick as u8,
                data[7] & 0x02 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::RightStick as u8,
                data[7] & 0x04 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_HORI_M2,
                data[7] & 0x08 != 0,
            );
            device.send_capsense(
                timestamp,
                joystick,
                GamepadCapSenseType::LeftStick,
                data[7] & 0x10 != 0,
            );
            device.send_capsense(
                timestamp,
                joystick,
                GamepadCapSenseType::RightStick,
                data[7] & 0x20 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_HORI_FR,
                data[7] & 0x40 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                SDL_GAMEPAD_BUTTON_HORI_FL,
                data[7] & 0x80 != 0,
            );
        }

        if !self.wireless && self.serial_needs_init {
            let serial = usb_serial(data);

            crate::joystick::assert_joysticks_locked();
            with_joystick(joystick, |j| j.serial = Some(serial));
            self.serial_needs_init = false;
        }

        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::RightTrigger as u8,
            read_trigger_axis(data[8]),
        );
        device.send_axis(
            timestamp,
            joystick,
            GamepadAxis::LeftTrigger as u8,
            read_trigger_axis(data[9]),
        );

        {
            /* 16-bit timestamp */
            let tick = load16(data[10], data[11]) as u16;
            let delta = if self.last_tick <= u32::from(tick) {
                u32::from(tick) - self.last_tick
            } else {
                u32::from(u16::MAX) - self.last_tick + u32::from(tick) + 1
            };

            self.last_tick = u32::from(tick);
            self.sensor_ticks += u64::from(delta);

            /* Sensor timestamp is in 1us units, but there seems to be some issues with the values reported from the device */
            // sensor_timestamp = timestamp; // if the values were good we would call SDL_US_TO_NS(ctx->sensor_ticks);

            /* New approach - simulate a fixed rate of 250hz (from observation). This reduces stutter from dropped/racing bluetooth packets.*/
            self.simulated_sensor_time_stamp += self.simulated_sensor_step_ns;
            let sensor_timestamp = self.simulated_sensor_time_stamp;

            let (gyro, accel) = sensor_values(data);
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Gyro,
                sensor_timestamp,
                &gyro,
            );
            device.send_sensor(
                timestamp,
                joystick,
                SensorType::Accel,
                sensor_timestamp,
                &accel,
            );
        }

        if self.last_state[24] != data[24] {
            let charging = (data[24] & 0x10) != 0;
            let percent = i32::from(data[24] & 0xF) * 10;

            let state = if charging {
                PowerState::Charging
            } else if self.wireless {
                PowerState::OnBattery
            } else {
                PowerState::Charged
            };

            device.send_power_info(joystick, state, percent);
        }

        let n = size.min(self.last_state.len());
        self.last_state[..n].copy_from_slice(&data[..n]);
    }
}

/// The gyro and accelerometer values of a state report.
fn sensor_values(data: &[u8]) -> ([f32; 3], [f32; 3]) {
    let accel_scale = STANDARD_GRAVITY * 8.0 / 32768.0;
    let gyro_scale = deg2rad(2048.0);
    let load = |offset: usize| f32::from(load16(data[offset], data[offset + 1]));
    let gyro = |offset: usize| {
        remap_val_clamped(
            -load(offset), // (-1.0f * LOAD16())
            f32::from(i16::MIN),
            f32::from(i16::MAX),
            -gyro_scale,
            gyro_scale,
        )
    };

    let mut imu_data = [0.0; 3];
    imu_data[1] = gyro(12);
    imu_data[2] = gyro(14);
    imu_data[0] = gyro(16);
    let gyro_data = imu_data;

    //  SDL_Log("%u %f, %f, %f ", data[0], imu_data[0], imu_data[1], imu_data[2] );
    imu_data[2] = load(18) * accel_scale;
    imu_data[1] = (-i32::from(load16(data[20], data[21]))) as f32 * accel_scale;
    imu_data[0] = load(22) * accel_scale;
    (gyro_data, imu_data)
}

impl DriverContext for SteamHoriContext {
    /// Translation of `HIDAPI_DriverSteamHori_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        self.serial_needs_init = true;

        device.set_device_name("Wireless HORIPAD For Steam");

        device.joystick_connected();
        Ok(())
    }

    /// Translation of `HIDAPI_DriverSteamHori_UpdateDevice()`.
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

                    self.handle_state_packet(device, joystick, &data, size);
                }
                Err(_) => break true,
            }
        };

        if read_error {
            /* Read error, device is disconnected */
            device.joystick_disconnected(first);
        }
        !read_error
    }

    /// Translation of `HIDAPI_DriverSteamHori_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        self.last_state = [0; USB_PACKET_LENGTH];

        /* Initialize the joystick capabilities */
        joystick.nbuttons = SDL_GAMEPAD_NUM_HORI_BUTTONS;
        joystick.naxes = GamepadAxis::COUNT;
        joystick.nhats = 1;

        self.wireless = device.product_id() == USB_PRODUCT_HORI_STEAM_CONTROLLER_BT;

        if let Some(serial) = device.serial().filter(|_| self.wireless) {
            joystick.serial = Some(serial);
            self.serial_needs_init = false;
        } else if !self.wireless {
            // Need to actual read from the device to init the serial
            // FIXME (upstream): the joystick being opened isn't open yet,
            // so the reports read here are dropped and the serial is only
            // set by the first update after the open.
            self.update_device(device);
        }

        let sensorupdaterate = if self.wireless { 120.0 } else { 250.0 };

        joystick.add_sensor(SensorType::Gyro, sensorupdaterate);
        joystick.add_sensor(SensorType::Accel, sensorupdaterate);

        let sensorupdatestep_ms: u64 = if self.wireless { 8333 } else { 4000 }; // Equivalent to 120hz / 250hz respectively
        self.simulated_sensor_step_ns = sensorupdatestep_ms * super::NS_PER_US;

        joystick.add_capsense(GamepadCapSenseType::LeftStick);
        joystick.add_capsense(GamepadCapSenseType::RightStick);

        Ok(())
    }

    // (RumbleJoystick: device doesn't support rumble)

    /// Translation of `HIDAPI_DriverSteamHori_SetJoystickSensorsEnabled()`.
    fn set_joystick_sensors_enabled(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        _enabled: bool,
    ) -> Result<()> {
        Ok(())
    }

    /// Translation of `HIDAPI_DriverSteamHori_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {}
}

#[cfg(test)]
mod tests;
