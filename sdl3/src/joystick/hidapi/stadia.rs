// Rust translation of src/joystick/hidapi/SDL_hidapi_stadia.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Google Stadia controller driver.

use super::ps4::hat_of;
use super::rumble::send_rumble;
use super::{
    remap_val, DeviceCtx, DriverContext, DriverImpl, HidapiDevice, JoystickCaps,
    SDL_HIDAPI_DEFAULT, USB_PACKET_LENGTH,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadType};
use crate::joystick::{is_joystick_google_stadia_controller, JoystickData};

/// `SDL_GAMEPAD_BUTTON_STADIA_CAPTURE`
const BUTTON_STADIA_CAPTURE: u8 = 11;
/// `SDL_GAMEPAD_BUTTON_STADIA_GOOGLE_ASSISTANT`
const BUTTON_STADIA_GOOGLE_ASSISTANT: u8 = 12;
/// `SDL_GAMEPAD_NUM_STADIA_BUTTONS`
const NUM_STADIA_BUTTONS: usize = 13;

/// The Stadia driver's static functions.
pub(crate) struct StadiaDriver;

impl DriverImpl for StadiaDriver {
    /// Translation of `HIDAPI_DriverStadia_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_STADIA]
    }

    /// Translation of `HIDAPI_DriverStadia_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_STADIA,
            hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
        )
    }

    /// Translation of `HIDAPI_DriverStadia_IsSupportedDevice()`.
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
        is_joystick_google_stadia_controller(vendor_id, product_id)
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(StadiaContext::default())
    }
}

/// Translation of `SDL_DriverStadia_Context`.
#[derive(Debug)]
struct StadiaContext {
    rumble_supported: bool,
    last_state: [u8; USB_PACKET_LENGTH],
}

impl Default for StadiaContext {
    fn default() -> Self {
        StadiaContext {
            rumble_supported: false,
            last_state: [0; USB_PACKET_LENGTH],
        }
    }
}

/// The rumble report (the packet of `HIDAPI_DriverStadia_RumbleJoystick()`).
fn rumble_packet(low_frequency_rumble: u16, high_frequency_rumble: u16) -> [u8; 5] {
    let [low_lo, low_hi] = low_frequency_rumble.to_le_bytes();
    let [high_lo, high_hi] = high_frequency_rumble.to_le_bytes();
    [0x05, low_lo, low_hi, high_lo, high_hi]
}

/// A stick axis of a state packet (`READ_STICK_AXIS()`).
fn read_stick_axis(value: u8) -> i16 {
    if value == 0x80 {
        0
    } else {
        // Note (upstream): 0x00 maps below the i16 range, which is
        // undefined behaviour in C (it wraps to 32510 on x86); the cast
        // here saturates to -32768.
        remap_val(
            (i32::from(value) - 0x80) as f32,
            (0x01 - 0x80) as f32,
            (0xff - 0x80) as f32,
            f32::from(i16::MIN),
            f32::from(i16::MAX),
        ) as i16
    }
}

impl StadiaContext {
    /// Translation of `HIDAPI_DriverStadia_HandleStatePacket()`.
    fn handle_state_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        let timestamp = crate::timer::ticks_ns();

        // The format is the same but the original FW will send 10 bytes and January '21 FW update will send 11
        if size < 10 || data[0] != 0x03 {
            // We don't know how to handle this report
            return;
        }

        if self.last_state[1] != data[1] {
            device.send_hat(timestamp, joystick, 0, hat_of(data[1]));
        }

        let mut button = |button: u8, down: bool| {
            device.send_button(timestamp, joystick, button, down);
        };
        if self.last_state[2] != data[2] {
            button(GamepadButton::Back as u8, data[2] & 0x40 != 0);
            button(GamepadButton::Guide as u8, data[2] & 0x10 != 0);
            button(GamepadButton::Start as u8, data[2] & 0x20 != 0);
            button(GamepadButton::RightStick as u8, data[2] & 0x80 != 0);
            button(BUTTON_STADIA_CAPTURE, data[2] & 0x01 != 0);
            button(BUTTON_STADIA_GOOGLE_ASSISTANT, data[2] & 0x02 != 0);
        }

        if self.last_state[3] != data[3] {
            button(GamepadButton::South as u8, data[3] & 0x40 != 0);
            button(GamepadButton::East as u8, data[3] & 0x20 != 0);
            button(GamepadButton::West as u8, data[3] & 0x10 != 0);
            button(GamepadButton::North as u8, data[3] & 0x08 != 0);
            button(GamepadButton::LeftShoulder as u8, data[3] & 0x04 != 0);
            button(GamepadButton::RightShoulder as u8, data[3] & 0x02 != 0);
            button(GamepadButton::LeftStick as u8, data[3] & 0x01 != 0);
        }

        let mut axis = |axis: GamepadAxis, value: i16| {
            device.send_axis(timestamp, joystick, axis as u8, value);
        };
        axis(GamepadAxis::LeftX, read_stick_axis(data[4]));
        axis(GamepadAxis::LeftY, read_stick_axis(data[5]));
        axis(GamepadAxis::RightX, read_stick_axis(data[6]));
        axis(GamepadAxis::RightY, read_stick_axis(data[7]));

        // (READ_TRIGGER_AXIS())
        axis(
            GamepadAxis::LeftTrigger,
            (i32::from(data[8]) * 257 - 32768) as i16,
        );
        axis(
            GamepadAxis::RightTrigger,
            (i32::from(data[9]) * 257 - 32768) as i16,
        );

        let n = size.min(self.last_state.len());
        self.last_state[..n].copy_from_slice(&data[..n]);
    }
}

impl DriverContext for StadiaContext {
    /// Translation of `HIDAPI_DriverStadia_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        // Check whether rumble is supported
        if device.write(&rumble_packet(0, 0)).is_ok() {
            self.rumble_supported = true;
        }

        device.set_device_name("Google Stadia Controller");

        device.joystick_connected();
        Ok(())
    }

    /// Translation of `HIDAPI_DriverStadia_UpdateDevice()`.
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
            // Read error, device is disconnected
            device.joystick_disconnected(first);
        }
        !read_error
    }

    /// Translation of `HIDAPI_DriverStadia_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        self.last_state = [0; USB_PACKET_LENGTH];

        // Initialize the joystick capabilities
        joystick.nbuttons = NUM_STADIA_BUTTONS;
        joystick.naxes = GamepadAxis::COUNT;
        joystick.nhats = 1;

        Ok(())
    }

    /// Translation of `HIDAPI_DriverStadia_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        if !self.rumble_supported {
            return Err(Error::unsupported());
        }

        let rumble_packet = rumble_packet(low_frequency_rumble, high_frequency_rumble);
        if send_rumble(device.device(), &rumble_packet).ok() != Some(rumble_packet.len()) {
            return Err(Error::new("Couldn't send rumble packet"));
        }
        Ok(())
    }

    /// Translation of `HIDAPI_DriverStadia_GetJoystickCapabilities()`.
    fn get_joystick_capabilities(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
    ) -> JoystickCaps {
        let mut caps = JoystickCaps(0);

        if self.rumble_supported {
            caps |= JoystickCaps::RUMBLE;
        }
        caps
    }

    /// Translation of `HIDAPI_DriverStadia_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {}
}

#[cfg(test)]
mod tests;
