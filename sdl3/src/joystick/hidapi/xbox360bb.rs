// Rust translation of src/joystick/hidapi/SDL_hidapi_xbox360bb.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Xbox 360 Big Button controller driver: the receiver of up to four
//! Scene It? controllers.

use super::{
    DeviceCtx, DriverContext, DriverImpl, HidapiDevice, SDL_HIDAPI_DEFAULT, USB_PACKET_LENGTH,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::gamepad::{GamepadButton, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::{
    with_joystick, JoystickConnectionState, JoystickData, HAT_DOWN, HAT_LEFT, HAT_RIGHT, HAT_UP,
};

const MAX_CONTROLLERS: usize = 4;

/// `SDL_GAMEPAD_BUTTON_XBOX360_BIG_BUTTON`
const BUTTON_XBOX360_BIG_BUTTON: u8 = GamepadButton::Start as u8 + 1;
/// `SDL_GAMEPAD_NUM_XBOX360BB_BUTTONS`
const NUM_XBOX360BB_BUTTONS: usize = BUTTON_XBOX360_BIG_BUTTON as usize + 1;

/// The driver's name (it has no hint of its own).
pub(crate) const DRIVER_NAME: &str = "SDL_JOYSTICK_HIDAPI_XBOX_360_BIGBUTTON";

/// The Xbox 360 Big Button driver's static functions.
pub(crate) struct Xbox360BbDriver;

impl DriverImpl for Xbox360BbDriver {
    /// Translation of `HIDAPI_DriverXBOX360BB_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_XBOX, hints::JOYSTICK_HIDAPI_XBOX_360]
    }

    /// Translation of `HIDAPI_DriverXBOX360BB_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_XBOX_360,
            hints::get_bool(
                hints::JOYSTICK_HIDAPI_XBOX,
                hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
            ),
        )
    }

    /// Translation of `HIDAPI_DriverXBOX360BB_IsSupportedDevice()`.
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
        vendor_id == USB_VENDOR_MICROSOFT && product_id == USB_PRODUCT_XBOX360_BIGBUTTON_RECEIVER
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(Xbox360BbContext::default())
    }
}

/// Translation of `SDL_DriverXbox360BB_Context`.
#[derive(Debug)]
struct Xbox360BbContext {
    joysticks: [JoystickID; MAX_CONTROLLERS],
    last_state: [[u8; USB_PACKET_LENGTH]; MAX_CONTROLLERS],
    last_packet: [u64; MAX_CONTROLLERS],
}

impl Default for Xbox360BbContext {
    fn default() -> Self {
        Xbox360BbContext {
            joysticks: [0; MAX_CONTROLLERS],
            last_state: [[0; USB_PACKET_LENGTH]; MAX_CONTROLLERS],
            last_packet: [0; MAX_CONTROLLERS],
        }
    }
}

/// Whether a joystick is open (`SDL_GetJoystickFromID() != NULL`).
fn joystick_is_open(joystick: JoystickID) -> bool {
    joystick != 0 && with_joystick(joystick, |_| ()).is_some()
}

impl Xbox360BbContext {
    /// Translation of `HIDAPI_DriverXBOX360BB_HandleStatePacket()`.
    fn handle_state_packet(&mut self, device: &mut DeviceCtx<'_>, data: &[u8], size: usize) {
        let timestamp = crate::timer::ticks_ns();

        let i = usize::from(data[2]);
        if i >= MAX_CONTROLLERS {
            // Note (upstream): upstream only asserts the controller index and
            // reads and writes past its arrays for a bad one; the report is
            // ignored here.
            return;
        }
        // Note (upstream): upstream sends the events of a joystick that isn't
        // open to a NULL joystick, which it dereferences; they are dropped
        // here.
        let joystick = self.joysticks[i];
        self.last_packet[i] = timestamp;

        if self.last_state[i][3] != data[3] {
            let mut hat = 0;

            if data[3] & 0x01 != 0 {
                hat |= HAT_UP;
            }
            if data[3] & 0x02 != 0 {
                hat |= HAT_DOWN;
            }
            if data[3] & 0x04 != 0 {
                hat |= HAT_LEFT;
            }
            if data[3] & 0x08 != 0 {
                hat |= HAT_RIGHT;
            }
            device.send_hat(timestamp, joystick, 0, hat);

            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Start as u8,
                data[3] & 0x10 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Back as u8,
                data[3] & 0x20 != 0,
            );
        }

        if self.last_state[i][4] != data[4] {
            let mut button = |button: u8, down: bool| {
                device.send_button(timestamp, joystick, button, down);
            };
            button(GamepadButton::Guide as u8, data[4] & 0x04 != 0);
            button(BUTTON_XBOX360_BIG_BUTTON, data[4] & 0x08 != 0);
            button(GamepadButton::South as u8, data[4] & 0x10 != 0);
            button(GamepadButton::East as u8, data[4] & 0x20 != 0);
            button(GamepadButton::West as u8, data[4] & 0x40 != 0);
            button(GamepadButton::North as u8, data[4] & 0x80 != 0);
        }

        let n = size.min(self.last_state[i].len());
        self.last_state[i][..n].copy_from_slice(&data[..n]);
    }

    /// Translation of `HIDAPI_DriverXBOX360BB_HandleReleaseEvents()`, at
    /// `timestamp`. `is_open` tells whether a joystick is open.
    fn handle_release_events(
        &mut self,
        device: &mut DeviceCtx<'_>,
        timestamp: u64,
        is_open: impl Fn(JoystickID) -> bool,
    ) {
        // The receiver only sends packets when a button is down, handle our own release logic
        for i in 0..MAX_CONTROLLERS {
            // FIXME: Even when a button is continuously pressed, we get _huge_
            // delays between packets. This is the smallest number I could get
            // without erroneous button releases. Yeesh.
            // -flibit
            const XBOXBB_BUTTON_RELEASE_TIMEOUT_MS: u64 = 120;
            if timestamp.wrapping_sub(self.last_packet[i]) / 1_000_000
                >= XBOXBB_BUTTON_RELEASE_TIMEOUT_MS
            {
                let joystick = self.joysticks[i];
                if is_open(joystick) {
                    device.send_hat(timestamp, joystick, 0, 0);
                    for button in [
                        GamepadButton::South as u8,
                        GamepadButton::East as u8,
                        GamepadButton::West as u8,
                        GamepadButton::North as u8,
                        GamepadButton::Back as u8,
                        GamepadButton::Guide as u8,
                        GamepadButton::Start as u8,
                        BUTTON_XBOX360_BIG_BUTTON,
                    ] {
                        device.send_button(timestamp, joystick, button, false);
                    }
                }
                self.last_state[i] = [0; USB_PACKET_LENGTH];
                self.last_packet[i] = timestamp;
            }
        }
    }
}

impl DriverContext for Xbox360BbContext {
    /// Translation of `HIDAPI_DriverXBOX360BB_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        device.set_device_name("Xbox 360 Big Button Controller");

        for joystick in &mut self.joysticks {
            *joystick = device.joystick_connected();
        }

        Ok(())
    }

    /// Translation of `HIDAPI_DriverXBOX360BB_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        let mut data = [0u8; USB_PACKET_LENGTH];
        let ok = loop {
            match device.read_timeout(&mut data, 0) {
                Ok(0) => break true,
                Ok(size) => {
                    if size == 5 {
                        self.handle_state_packet(device, &data, size);
                    }
                }
                Err(_) => break false,
            }
        };

        self.handle_release_events(device, crate::timer::ticks_ns(), joystick_is_open);

        ok
    }

    /// Translation of `HIDAPI_DriverXBOX360BB_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        if !self.joysticks.contains(&joystick.instance_id) {
            // Should never get here! (upstream fails without an error)
            return Err(Error::new("Couldn't find joystick"));
        }
        joystick.nbuttons = NUM_XBOX360BB_BUTTONS;
        joystick.nhats = 1;
        joystick.connection_state = JoystickConnectionState::Wireless;
        Ok(())
    }

    /// Translation of `HIDAPI_DriverXBOX360BB_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {}
}

#[cfg(test)]
mod tests;
