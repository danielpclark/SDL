// Rust translation of src/joystick/hidapi/SDL_hidapi_luna.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Amazon Luna controller driver.

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
use crate::joystick::usb_ids::*;
use crate::joystick::{is_joystick_amazon_luna_controller, JoystickData};
use crate::power::PowerState;

/// Sending rumble on macOS blocks for a long time and eventually fails
/// (`ENABLE_LUNA_BLUETOOTH_RUMBLE`).
const ENABLE_LUNA_BLUETOOTH_RUMBLE: bool = !cfg!(target_os = "macos");

/// `SDL_GAMEPAD_BUTTON_LUNA_MICROPHONE`
const BUTTON_LUNA_MICROPHONE: u8 = 11;
/// `SDL_GAMEPAD_NUM_LUNA_BUTTONS`
const NUM_LUNA_BUTTONS: usize = 12;

/// The Luna driver's static functions.
pub(crate) struct LunaDriver;

impl DriverImpl for LunaDriver {
    /// Translation of `HIDAPI_DriverLuna_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_LUNA]
    }

    /// Translation of `HIDAPI_DriverLuna_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_LUNA,
            hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
        )
    }

    /// Translation of `HIDAPI_DriverLuna_IsSupportedDevice()`.
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
        is_joystick_amazon_luna_controller(vendor_id, product_id)
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(LunaContext::default())
    }
}

/// Translation of `SDL_DriverLuna_Context`.
#[derive(Debug)]
struct LunaContext {
    last_state: [u8; USB_PACKET_LENGTH],
}

impl Default for LunaContext {
    fn default() -> Self {
        LunaContext {
            last_state: [0; USB_PACKET_LENGTH],
        }
    }
}

/// The Bluetooth rumble report (the packet of
/// `HIDAPI_DriverLuna_RumbleJoystick()`).
fn bluetooth_rumble_packet(low_frequency_rumble: u16, high_frequency_rumble: u16) -> [u8; 9] {
    // Same packet as on Xbox One controllers connected via Bluetooth
    let mut rumble_packet = [0x03, 0x0F, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x00, 0xEB];

    // Magnitude is 1..100 so scale the 16-bit input here
    rumble_packet[4] = (low_frequency_rumble / 655) as u8;
    rumble_packet[5] = (high_frequency_rumble / 655) as u8;
    rumble_packet
}

/// Whether the controller is rumbled with the Bluetooth packet (the
/// `ENABLE_LUNA_BLUETOOTH_RUMBLE` product check).
fn has_bluetooth_rumble(device: &HidapiDevice) -> bool {
    // FIXME (upstream): only the product ID is checked, and the USB
    // controller has the same one as the Bluetooth controller
    // (USB_PRODUCT_AMAZON_LUNA_CONTROLLER), so the USB controller reports
    // rumble and is sent the Bluetooth rumble packet too.
    ENABLE_LUNA_BLUETOOTH_RUMBLE && device.product_id() == BLUETOOTH_PRODUCT_LUNA_CONTROLLER
}

/// A stick axis of a state packet (`READ_STICK_AXIS()`).
fn read_stick_axis(value: u8) -> i16 {
    if value == 0x7f {
        0
    } else {
        read_trigger_axis(value)
    }
}

/// A trigger axis of a USB state packet (`READ_TRIGGER_AXIS()`).
fn read_trigger_axis(value: u8) -> i16 {
    remap_val(
        f32::from(value),
        0x00 as f32,
        0xff as f32,
        f32::from(i16::MIN),
        f32::from(i16::MAX),
    ) as i16
}

/// A trigger axis of a Bluetooth state packet (`READ_TRIGGER_AXIS()`).
fn read_bluetooth_trigger_axis(lo: u8, hi: u8) -> i16 {
    let value = (i32::from(u16::from_le_bytes([lo, hi])) & 0x3ff) - 0x200;
    remap_val(
        value as f32,
        (0x00 - 0x200) as f32,
        (0x3ff - 0x200) as f32,
        f32::from(i16::MIN),
        f32::from(i16::MAX),
    ) as i16
}

impl LunaContext {
    /// Translation of `HIDAPI_DriverLuna_HandleUSBStatePacket()`.
    fn handle_usb_state_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        let timestamp = crate::timer::ticks_ns();

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
            button(GamepadButton::Back as u8, data[1] & 0x40 != 0);
            button(GamepadButton::Start as u8, data[1] & 0x80 != 0);
        }
        if self.last_state[2] != data[2] {
            button(GamepadButton::Guide as u8, data[2] & 0x01 != 0);
            button(BUTTON_LUNA_MICROPHONE, data[2] & 0x02 != 0);
            button(GamepadButton::LeftStick as u8, data[2] & 0x04 != 0);
            button(GamepadButton::RightStick as u8, data[2] & 0x08 != 0);
        }

        if self.last_state[3] != data[3] {
            device.send_hat(timestamp, joystick, 0, hat_of(data[3] & 0x0f));
        }

        let mut axis = |axis: GamepadAxis, value: i16| {
            device.send_axis(timestamp, joystick, axis as u8, value);
        };
        axis(GamepadAxis::LeftX, read_stick_axis(data[4]));
        axis(GamepadAxis::LeftY, read_stick_axis(data[5]));
        axis(GamepadAxis::RightX, read_stick_axis(data[6]));
        axis(GamepadAxis::RightY, read_stick_axis(data[7]));

        axis(GamepadAxis::LeftTrigger, read_trigger_axis(data[8]));
        axis(GamepadAxis::RightTrigger, read_trigger_axis(data[9]));

        let n = size.min(self.last_state.len());
        self.last_state[..n].copy_from_slice(&data[..n]);
    }

    /// Translation of `HIDAPI_DriverLuna_HandleBluetoothStatePacket()`.
    fn handle_bluetooth_state_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        let timestamp = crate::timer::ticks_ns();

        if size >= 2 && data[0] == 0x02 {
            // Home button has dedicated report
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Guide as u8,
                data[1] & 0x1 != 0,
            );
            return;
        }

        if size >= 2 && data[0] == 0x04 {
            // Battery level report
            let percent = ((f32::from(data[1]) / 255.0) * 100.0).round() as i32;
            device.send_power_info(joystick, PowerState::OnBattery, percent);
            return;
        }

        if size < 17 || data[0] != 0x01 {
            // We don't know how to handle this report
            return;
        }

        if self.last_state[13] != data[13] {
            // (the hat values start at 1 for up; 0 is centered)
            let hat = hat_of((data[13] & 0x0f).wrapping_sub(1));
            device.send_hat(timestamp, joystick, 0, hat);
        }

        let mut button = |button: u8, down: bool| {
            device.send_button(timestamp, joystick, button, down);
        };
        if self.last_state[14] != data[14] {
            button(GamepadButton::South as u8, data[14] & 0x01 != 0);
            button(GamepadButton::East as u8, data[14] & 0x02 != 0);
            button(GamepadButton::West as u8, data[14] & 0x08 != 0);
            button(GamepadButton::North as u8, data[14] & 0x10 != 0);
            button(GamepadButton::LeftShoulder as u8, data[14] & 0x40 != 0);
            button(GamepadButton::RightShoulder as u8, data[14] & 0x80 != 0);
        }
        if self.last_state[15] != data[15] {
            button(GamepadButton::Start as u8, data[15] & 0x08 != 0);
            button(GamepadButton::LeftStick as u8, data[15] & 0x20 != 0);
            button(GamepadButton::RightStick as u8, data[15] & 0x40 != 0);
        }
        if self.last_state[16] != data[16] {
            button(GamepadButton::Back as u8, data[16] & 0x01 != 0);
            button(BUTTON_LUNA_MICROPHONE, data[16] & 0x02 != 0);
        }

        let mut axis = |axis: GamepadAxis, value: i16| {
            device.send_axis(timestamp, joystick, axis as u8, value);
        };
        axis(GamepadAxis::LeftX, read_stick_axis(data[2]));
        axis(GamepadAxis::LeftY, read_stick_axis(data[4]));
        axis(GamepadAxis::RightX, read_stick_axis(data[6]));
        axis(GamepadAxis::RightY, read_stick_axis(data[8]));

        axis(
            GamepadAxis::LeftTrigger,
            read_bluetooth_trigger_axis(data[9], data[10]),
        );
        axis(
            GamepadAxis::RightTrigger,
            read_bluetooth_trigger_axis(data[11], data[12]),
        );

        let n = size.min(self.last_state.len());
        self.last_state[..n].copy_from_slice(&data[..n]);
    }

    /// One report of `HIDAPI_DriverLuna_UpdateDevice()`.
    fn handle_report(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        match size {
            10 => self.handle_usb_state_packet(device, joystick, data, size),
            _ => self.handle_bluetooth_state_packet(device, joystick, data, size),
        }
    }
}

impl DriverContext for LunaContext {
    /// Translation of `HIDAPI_DriverLuna_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        device.set_device_name("Amazon Luna Controller");

        device.joystick_connected();
        Ok(())
    }

    /// Translation of `HIDAPI_DriverLuna_UpdateDevice()`.
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

    /// Translation of `HIDAPI_DriverLuna_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        self.last_state = [0; USB_PACKET_LENGTH];

        // Initialize the joystick capabilities
        joystick.nbuttons = NUM_LUNA_BUTTONS;
        joystick.naxes = GamepadAxis::COUNT;
        joystick.nhats = 1;

        Ok(())
    }

    /// Translation of `HIDAPI_DriverLuna_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        if has_bluetooth_rumble(device) {
            let rumble_packet =
                bluetooth_rumble_packet(low_frequency_rumble, high_frequency_rumble);

            if send_rumble(device.device(), &rumble_packet).ok() != Some(rumble_packet.len()) {
                return Err(Error::new("Couldn't send rumble packet"));
            }

            return Ok(());
        }

        // There is currently no rumble packet over USB
        Err(Error::unsupported())
    }

    /// Translation of `HIDAPI_DriverLuna_GetJoystickCapabilities()`.
    fn get_joystick_capabilities(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
    ) -> JoystickCaps {
        let mut result = JoystickCaps(0);

        if has_bluetooth_rumble(device) {
            result |= JoystickCaps::RUMBLE;
        }

        result
    }

    /// Translation of `HIDAPI_DriverLuna_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {}
}

#[cfg(test)]
mod tests;
