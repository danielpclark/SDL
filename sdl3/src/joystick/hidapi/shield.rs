// Rust translation of src/joystick/hidapi/SDL_hidapi_shield.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The NVIDIA SHIELD controller driver (the 2015 controller, V103, and the
//! 2017 one, V104).

use super::ps4::hat_of;
use super::rumble::{lock_rumble, send_rumble};
use super::{
    load16, DeviceCtx, DriverContext, DriverImpl, HidapiDevice, JoystickCaps, SDL_HIDAPI_DEFAULT,
    USB_PACKET_LENGTH,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::{is_joystick_nvidia_shield_controller, JoystickData};
use crate::power::PowerState;

const CMD_BATTERY_STATE: u8 = 0x07;
const CMD_RUMBLE: u8 = 0x39;
const CMD_CHARGE_STATE: u8 = 0x3A;

/// Milliseconds between polls of battery state
const BATTERY_POLL_INTERVAL_MS: u64 = 60000;

/// Milliseconds between retransmission of rumble to keep motors running
const RUMBLE_REFRESH_INTERVAL_MS: u64 = 500;

/// Reports that are too small are dropped over Bluetooth
const HID_REPORT_SIZE: usize = 33;

/// `SDL_GAMEPAD_BUTTON_SHIELD_SHARE`
const BUTTON_SHIELD_SHARE: u8 = 11;
/// `SDL_GAMEPAD_BUTTON_SHIELD_V103_TOUCHPAD`
const BUTTON_SHIELD_V103_TOUCHPAD: u8 = 12;
/// `SDL_GAMEPAD_BUTTON_SHIELD_V103_MINUS`
const BUTTON_SHIELD_V103_MINUS: u8 = 13;
/// `SDL_GAMEPAD_BUTTON_SHIELD_V103_PLUS`
const BUTTON_SHIELD_V103_PLUS: u8 = 14;
/// `SDL_GAMEPAD_NUM_SHIELD_V103_BUTTONS`
const NUM_SHIELD_V103_BUTTONS: usize = 15;
/// `SDL_GAMEPAD_NUM_SHIELD_V104_BUTTONS`
const NUM_SHIELD_V104_BUTTONS: usize = BUTTON_SHIELD_SHARE as usize + 1;

// EShieldReportId
const REPORT_ID_CONTROLLER_STATE: u8 = 0x01;
const REPORT_ID_CONTROLLER_TOUCH: u8 = 0x02;
const REPORT_ID_COMMAND_RESPONSE: u8 = 0x03;
const REPORT_ID_COMMAND_REQUEST: u8 = 0x04;

/// The payload size of a `ShieldCommandReport_t`, the report of both
/// requests and responses (report ID, command, sequence number, payload).
const COMMAND_PAYLOAD_SIZE: usize = HID_REPORT_SIZE - 3;

/// The SHIELD driver's static functions.
pub(crate) struct ShieldDriver;

impl DriverImpl for ShieldDriver {
    /// Translation of `HIDAPI_DriverShield_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_SHIELD]
    }

    /// Translation of `HIDAPI_DriverShield_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_SHIELD,
            hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
        )
    }

    /// Translation of `HIDAPI_DriverShield_IsSupportedDevice()`.
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
        is_joystick_nvidia_shield_controller(vendor_id, product_id)
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(ShieldContext::default())
    }
}

/// Translation of `SDL_DriverShield_Context`.
#[derive(Debug)]
struct ShieldContext {
    seq_num: u8,

    has_charging: bool,
    charging: u8,
    has_battery_level: bool,
    battery_level: u8,
    last_battery_query_time: u64,

    rumble_report_pending: bool,
    rumble_update_pending: bool,
    left_motor_amplitude: u8,
    right_motor_amplitude: u8,
    last_rumble_time: u64,

    last_state: [u8; USB_PACKET_LENGTH],
}

impl Default for ShieldContext {
    fn default() -> Self {
        ShieldContext {
            seq_num: 0,
            has_charging: false,
            charging: 0,
            has_battery_level: false,
            battery_level: 0,
            last_battery_query_time: 0,
            rumble_report_pending: false,
            rumble_update_pending: false,
            left_motor_amplitude: 0,
            right_motor_amplitude: 0,
            last_rumble_time: 0,
            last_state: [0; USB_PACKET_LENGTH],
        }
    }
}

/// The rumble report of the V103 controller (the packet of
/// `HIDAPI_DriverShield_RumbleJoystick()`).
fn v103_rumble_packet(low_frequency_rumble: u16, high_frequency_rumble: u16) -> [u8; 7] {
    let mut rumble_packet = [0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

    rumble_packet[2] = (low_frequency_rumble >> 8) as u8;
    rumble_packet[4] = (high_frequency_rumble >> 8) as u8;
    rumble_packet
}

/// An axis of a state packet (`LOAD16(A, B) - 0x8000`, kept to 16 bits).
fn read_axis(a: u8, b: u8) -> i16 {
    (i32::from(load16(a, b)) - 0x8000) as i16
}

impl ShieldContext {
    /// The command request report of `HIDAPI_DriverShield_SendCommand()`,
    /// numbered with the next sequence number. `data` fits in the payload.
    fn command_packet(&mut self, cmd: u8, data: &[u8]) -> [u8; HID_REPORT_SIZE] {
        let mut cmd_pkt = [0u8; HID_REPORT_SIZE];

        cmd_pkt[0] = REPORT_ID_COMMAND_REQUEST;
        cmd_pkt[1] = cmd;
        cmd_pkt[2] = self.seq_num;
        self.seq_num = self.seq_num.wrapping_add(1);
        // (the unused part of the payload stays zeroed)
        cmd_pkt[3..3 + data.len()].copy_from_slice(data);
        cmd_pkt
    }

    /// Translation of `HIDAPI_DriverShield_SendCommand()`.
    fn send_command(&mut self, device: &mut DeviceCtx<'_>, cmd: u8, data: &[u8]) -> Result<()> {
        if cfg!(target_os = "macos") {
            // We hang for several seconds when trying to send output reports on macOS
            return Err(Error::unsupported());
        }

        if data.len() > COMMAND_PAYLOAD_SIZE {
            return Err(Error::new("Command data exceeds HID report size"));
        }

        let lock = lock_rumble()?;

        let cmd_pkt = self.command_packet(cmd, data);

        if lock.send_and_unlock(device.device(), &cmd_pkt).ok() != Some(cmd_pkt.len()) {
            return Err(Error::new("Couldn't send command packet"));
        }

        Ok(())
    }

    /// The payload of the next rumble command, if there is a rumble update
    /// to send (the state change of `HIDAPI_DriverShield_SendNextRumble()`).
    fn next_rumble(&mut self) -> Option<[u8; 3]> {
        if !self.rumble_update_pending {
            return None;
        }

        let rumble_data = [
            0x01, // enable
            self.left_motor_amplitude,
            self.right_motor_amplitude,
        ];

        self.rumble_update_pending = false;
        self.last_rumble_time = crate::timer::ticks_ms();

        Some(rumble_data)
    }

    /// Translation of `HIDAPI_DriverShield_SendNextRumble()`.
    fn send_next_rumble(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        match self.next_rumble() {
            Some(rumble_data) => self.send_command(device, CMD_RUMBLE, &rumble_data),
            None => Ok(()),
        }
    }

    /// The rumble state change of the V104 controller in
    /// `HIDAPI_DriverShield_RumbleJoystick()`; whether to send it now.
    fn set_rumble(&mut self, low_frequency_rumble: u16, high_frequency_rumble: u16) -> bool {
        // The rumble motors are quite intense, so tone down the intensity like the official driver does
        self.left_motor_amplitude = (low_frequency_rumble >> 11) as u8;
        self.right_motor_amplitude = (high_frequency_rumble >> 11) as u8;
        self.rumble_update_pending = true;

        // FIXME (upstream): rumble_report_pending is never set, so an update
        // is never held back until the previous one is acknowledged.
        if self.rumble_report_pending {
            // We will service this after the hardware acknowledges the previous request
            return false;
        }

        true
    }

    /// Translation of `HIDAPI_DriverShield_HandleStatePacketV103()`.
    fn handle_state_packet_v103(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        let timestamp = crate::timer::ticks_ns();

        if self.last_state[3] != data[3] {
            device.send_hat(timestamp, joystick, 0, hat_of(data[3]));
        }

        let mut button = |button: u8, down: bool| {
            device.send_button(timestamp, joystick, button, down);
        };
        if self.last_state[1] != data[1] {
            button(GamepadButton::South as u8, data[1] & 0x01 != 0);
            button(GamepadButton::East as u8, data[1] & 0x02 != 0);
            button(GamepadButton::West as u8, data[1] & 0x04 != 0);
            button(GamepadButton::North as u8, data[1] & 0x08 != 0);
            button(GamepadButton::LeftShoulder as u8, data[1] & 0x10 != 0);
            button(GamepadButton::RightShoulder as u8, data[1] & 0x20 != 0);
            button(GamepadButton::LeftStick as u8, data[1] & 0x40 != 0);
            button(GamepadButton::RightStick as u8, data[1] & 0x80 != 0);
        }

        if self.last_state[2] != data[2] {
            button(GamepadButton::Start as u8, data[2] & 0x02 != 0);
            button(BUTTON_SHIELD_V103_PLUS, data[2] & 0x08 != 0);
            button(BUTTON_SHIELD_V103_MINUS, data[2] & 0x10 != 0);
            //button(GamepadButton::Guide as u8, data[2] & 0x20 != 0);
            button(GamepadButton::Back as u8, data[2] & 0x40 != 0);
            //button(BUTTON_SHIELD_SHARE, data[2] & 0x80 != 0);
            button(GamepadButton::Guide as u8, data[2] & 0x80 != 0);
        }

        let mut axis = |axis: GamepadAxis, value: i16| {
            device.send_axis(timestamp, joystick, axis as u8, value);
        };
        axis(GamepadAxis::LeftX, read_axis(data[4], data[5]));
        axis(GamepadAxis::LeftY, read_axis(data[6], data[7]));

        axis(GamepadAxis::RightX, read_axis(data[8], data[9]));
        axis(GamepadAxis::RightY, read_axis(data[10], data[11]));

        axis(GamepadAxis::LeftTrigger, read_axis(data[12], data[13]));
        axis(GamepadAxis::RightTrigger, read_axis(data[14], data[15]));

        let n = size.min(self.last_state.len());
        self.last_state[..n].copy_from_slice(&data[..n]);
    }

    /// Translation of `HIDAPI_DriverShield_HandleTouchPacketV103()`.
    fn handle_touch_packet_v103(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
    ) {
        let timestamp = crate::timer::ticks_ns();

        device.send_button(
            timestamp,
            joystick,
            BUTTON_SHIELD_V103_TOUCHPAD,
            data[1] & 0x01 != 0,
        );

        // It's a triangular pad, but just use the center as the usable touch area
        let touchpad_down = data[1] & 0x80 == 0;
        let touchpad_x = ((i32::from(data[2]) - 0x70) as f32 / 0x50 as f32).clamp(0.0, 1.0);
        let touchpad_y = ((i32::from(data[4]) - 0x40) as f32 / 0x15 as f32).clamp(0.0, 1.0);
        device.send_touchpad(
            timestamp,
            joystick,
            0,
            0,
            touchpad_down,
            touchpad_x,
            touchpad_y,
            if touchpad_down { 1.0 } else { 0.0 },
        );
    }

    /// Translation of `HIDAPI_DriverShield_HandleStatePacketV104()`.
    fn handle_state_packet_v104(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        let timestamp = crate::timer::ticks_ns();

        if size < 23 {
            return;
        }

        if self.last_state[2] != data[2] {
            device.send_hat(timestamp, joystick, 0, hat_of(data[2]));
        }

        let mut button = |button: u8, down: bool| {
            device.send_button(timestamp, joystick, button, down);
        };
        if self.last_state[3] != data[3] {
            button(GamepadButton::South as u8, data[3] & 0x01 != 0);
            button(GamepadButton::East as u8, data[3] & 0x02 != 0);
            button(GamepadButton::West as u8, data[3] & 0x04 != 0);
            button(GamepadButton::North as u8, data[3] & 0x08 != 0);
            button(GamepadButton::LeftShoulder as u8, data[3] & 0x10 != 0);
            button(GamepadButton::RightShoulder as u8, data[3] & 0x20 != 0);
            button(GamepadButton::LeftStick as u8, data[3] & 0x40 != 0);
            button(GamepadButton::RightStick as u8, data[3] & 0x80 != 0);
        }

        if self.last_state[4] != data[4] {
            button(GamepadButton::Start as u8, data[4] & 0x01 != 0);
        }

        let mut axis = |axis: GamepadAxis, value: i16| {
            device.send_axis(timestamp, joystick, axis as u8, value);
        };
        axis(GamepadAxis::LeftX, read_axis(data[9], data[10]));
        axis(GamepadAxis::LeftY, read_axis(data[11], data[12]));

        axis(GamepadAxis::RightX, read_axis(data[13], data[14]));
        axis(GamepadAxis::RightY, read_axis(data[15], data[16]));

        axis(GamepadAxis::LeftTrigger, read_axis(data[19], data[20]));
        axis(GamepadAxis::RightTrigger, read_axis(data[21], data[22]));

        if self.last_state[17] != data[17] {
            let mut button = |button: u8, down: bool| {
                device.send_button(timestamp, joystick, button, down);
            };
            //button(BUTTON_SHIELD_SHARE, data[17] & 0x01 != 0);
            button(GamepadButton::Back as u8, data[17] & 0x02 != 0);
            //button(GamepadButton::Guide as u8, data[17] & 0x04 != 0);
            button(GamepadButton::Guide as u8, data[17] & 0x01 != 0);
        }

        let n = size.min(self.last_state.len());
        self.last_state[..n].copy_from_slice(&data[..n]);
    }

    /// Translation of `HIDAPI_DriverShield_UpdatePowerInfo()`.
    fn update_power_info(&self, device: &mut DeviceCtx<'_>, joystick: JoystickID) {
        if !self.has_charging || !self.has_battery_level {
            return;
        }

        let state = if self.charging != 0 {
            PowerState::Charging
        } else {
            PowerState::OnBattery
        };
        let percent = i32::from(self.battery_level) * 20;
        device.send_power_info(joystick, state, percent);
    }

    /// One report of `HIDAPI_DriverShield_UpdateDevice()`.
    fn handle_report(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        // Byte 0 is HID report ID
        match data[0] {
            REPORT_ID_CONTROLLER_STATE => {
                if size == 16 {
                    self.handle_state_packet_v103(device, joystick, data, size);
                } else {
                    self.handle_state_packet_v104(device, joystick, data, size);
                }
            }
            REPORT_ID_CONTROLLER_TOUCH => {
                self.handle_touch_packet_v103(device, joystick, data);
            }
            REPORT_ID_COMMAND_RESPONSE => {
                // (cmd and payload of the ShieldCommandReport_t)
                let payload = &data[3..];
                match data[1] {
                    CMD_RUMBLE => {
                        self.rumble_report_pending = false;
                        let _ = self.send_next_rumble(device);
                    }
                    CMD_CHARGE_STATE => {
                        self.has_charging = true;
                        self.charging = payload[0];
                        self.update_power_info(device, joystick);
                    }
                    CMD_BATTERY_STATE => {
                        self.has_battery_level = true;
                        self.battery_level = payload[2];
                        self.update_power_info(device, joystick);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

impl DriverContext for ShieldContext {
    /// Translation of `HIDAPI_DriverShield_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        device.set_device_name("NVIDIA SHIELD Controller");

        device.joystick_connected();
        Ok(())
    }

    /// Translation of `HIDAPI_DriverShield_UpdateDevice()`.
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

        // Ask for battery state again if we're due for an update
        if joystick.is_some()
            && crate::timer::ticks_ms() >= self.last_battery_query_time + BATTERY_POLL_INTERVAL_MS
        {
            self.last_battery_query_time = crate::timer::ticks_ms();
            let _ = self.send_command(device, CMD_BATTERY_STATE, &[]);
        }

        // Retransmit rumble packets if they've lasted longer than the hardware supports
        if (self.left_motor_amplitude != 0 || self.right_motor_amplitude != 0)
            && crate::timer::ticks_ms() >= self.last_rumble_time + RUMBLE_REFRESH_INTERVAL_MS
        {
            self.rumble_update_pending = true;
            let _ = self.send_next_rumble(device);
        }

        if read_error {
            // Read error, device is disconnected
            device.joystick_disconnected(first);
        }
        !read_error
    }

    /// Translation of `HIDAPI_DriverShield_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        self.rumble_report_pending = false;
        self.rumble_update_pending = false;
        self.left_motor_amplitude = 0;
        self.right_motor_amplitude = 0;
        self.last_rumble_time = 0;
        self.last_state = [0; USB_PACKET_LENGTH];

        // Initialize the joystick capabilities
        if device.product_id() == USB_PRODUCT_NVIDIA_SHIELD_CONTROLLER_V103 {
            joystick.nbuttons = NUM_SHIELD_V103_BUTTONS;
            joystick.naxes = GamepadAxis::COUNT;
            joystick.nhats = 1;

            joystick.add_touchpad(1);
        } else {
            joystick.nbuttons = NUM_SHIELD_V104_BUTTONS;
            joystick.naxes = GamepadAxis::COUNT;
            joystick.nhats = 1;
        }

        // Request battery and charging info
        self.last_battery_query_time = crate::timer::ticks_ms();
        let _ = self.send_command(device, CMD_CHARGE_STATE, &[]);
        let _ = self.send_command(device, CMD_BATTERY_STATE, &[]);

        Ok(())
    }

    /// Translation of `HIDAPI_DriverShield_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        if device.product_id() == USB_PRODUCT_NVIDIA_SHIELD_CONTROLLER_V103 {
            let rumble_packet = v103_rumble_packet(low_frequency_rumble, high_frequency_rumble);

            if send_rumble(device.device(), &rumble_packet).ok() != Some(rumble_packet.len()) {
                return Err(Error::new("Couldn't send rumble packet"));
            }
            Ok(())
        } else if self.set_rumble(low_frequency_rumble, high_frequency_rumble) {
            self.send_next_rumble(device)
        } else {
            Ok(())
        }
    }

    /// Translation of `HIDAPI_DriverShield_GetJoystickCapabilities()`.
    fn get_joystick_capabilities(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
    ) -> JoystickCaps {
        JoystickCaps::RUMBLE
    }

    /// Translation of `HIDAPI_DriverShield_SendJoystickEffect()`.
    fn send_joystick_effect(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        data: &[u8],
    ) -> Result<()> {
        match data.split_first() {
            // Single command byte followed by a variable length payload, or
            // with no payload
            Some((&cmd, payload)) => self.send_command(device, cmd, payload),
            None => Err(Error::new(
                "Effect data must at least contain a command byte",
            )),
        }
    }

    /// Translation of `HIDAPI_DriverShield_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {}
}

#[cfg(test)]
mod tests;
