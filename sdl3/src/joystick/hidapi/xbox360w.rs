// Rust translation of src/joystick/hidapi/SDL_hidapi_xbox360w.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Xbox 360 wireless receiver driver: each receiver reports one
//! controller, which connects and disconnects as it is turned on and off.

use super::rumble::send_rumble;
use super::xbox360::{send_state_axes, XInputCapabilities, FLAG_FORCE_FEEDBACK, FLAG_WIRELESS};
use super::{
    load16, DeviceCtx, DriverContext, DriverImpl, HidapiDevice, HintWatch, JoystickCaps,
    SDL_HIDAPI_DEFAULT, USB_PACKET_LENGTH,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::{
    joystick_player_index_for_id, JoystickConnectionState, JoystickData, JoystickType, HAT_DOWN,
    HAT_LEFT, HAT_RIGHT, HAT_UP,
};
use crate::power::PowerState;

/// The Xbox 360 wireless driver's static functions.
pub(crate) struct Xbox360WDriver;

impl DriverImpl for Xbox360WDriver {
    /// Translation of `HIDAPI_DriverXbox360W_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[
            hints::JOYSTICK_HIDAPI_XBOX,
            hints::JOYSTICK_HIDAPI_XBOX_360,
            hints::JOYSTICK_HIDAPI_XBOX_360_WIRELESS,
        ]
    }

    /// Translation of `HIDAPI_DriverXbox360W_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_XBOX_360_WIRELESS,
            hints::get_bool(
                hints::JOYSTICK_HIDAPI_XBOX_360,
                hints::get_bool(
                    hints::JOYSTICK_HIDAPI_XBOX,
                    hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
                ),
            ),
        )
    }

    /// Translation of `HIDAPI_DriverXbox360W_IsSupportedDevice()`.
    fn is_supported_device(
        &self,
        _device: Option<&HidapiDevice>,
        _name: &str,
        gamepad_type: GamepadType,
        vendor_id: u16,
        product_id: u16,
        _version: u16,
        _interface_number: i32,
        _interface_class: i32,
        _interface_subclass: i32,
        interface_protocol: i32,
    ) -> bool {
        const XB360W_IFACE_PROTOCOL: i32 = 129; // Wireless

        if vendor_id == USB_VENDOR_MICROSOFT && product_id == USB_PRODUCT_XBOX360_BIGBUTTON_RECEIVER
        {
            // This is the BigButton wireless receiver, which talks a different protocol
            return false;
        }
        (vendor_id == USB_VENDOR_MICROSOFT
            && (product_id == USB_PRODUCT_XBOX360_WIRELESS_RECEIVER_THIRDPARTY2
                || product_id == USB_PRODUCT_XBOX360_WIRELESS_RECEIVER_THIRDPARTY1
                || product_id == USB_PRODUCT_XBOX360_WIRELESS_RECEIVER)
            && interface_protocol == 0)
            || (gamepad_type == GamepadType::Xbox360 && interface_protocol == XB360W_IFACE_PROTOCOL)
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(Xbox360WContext::default())
    }
}

/// Translation of `SDL_DriverXbox360W_Context`.
#[derive(Debug)]
struct Xbox360WContext {
    connected: bool,
    player_index: i32,
    player_lights: bool,
    capabilities: XInputCapabilities,
    last_state: [u8; USB_PACKET_LENGTH],
    /// The `SDL_PlayerLEDHintChanged()` callback.
    player_led_hint: Option<HintWatch>,
}

impl Default for Xbox360WContext {
    fn default() -> Self {
        Xbox360WContext {
            connected: false,
            player_index: 0,
            player_lights: false,
            capabilities: XInputCapabilities::default(),
            last_state: [0; USB_PACKET_LENGTH],
            player_led_hint: None,
        }
    }
}

/// Translation of `SetSlotLED()`.
fn set_slot_led(device: &DeviceCtx<'_>, slot: u8, on: bool) -> bool {
    const BLINK: bool = false;
    let mode = if on {
        (if BLINK { 0x02 } else { 0x06 }) + slot
    } else {
        0
    };
    let mut led_packet = [
        0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    led_packet[3] = 0x40 + (mode % 0x0e);
    device.write(&led_packet).ok() == Some(led_packet.len())
}

/// Translation of `UpdatePowerLevel()`.
fn update_power_level(device: &mut DeviceCtx<'_>, joystick: JoystickID, level: u8) {
    let percent = ((f32::from(level) / 255.0) * 100.0).round() as i32;
    device.send_power_info(joystick, PowerState::OnBattery, percent);
}

/// The XInput device subtype of a capabilities report as a joystick type,
/// if it is a known one.
fn joystick_type_of_sub_type(sub_type: u8) -> Option<JoystickType> {
    match sub_type {
        0x01 => Some(JoystickType::Gamepad), // XINPUT_DEVSUBTYPE_GAMEPAD
        0x02 => Some(JoystickType::Wheel),   // XINPUT_DEVSUBTYPE_WHEEL
        0x03 => Some(JoystickType::ArcadeStick), // XINPUT_DEVSUBTYPE_ARCADE_STICK
        0x04 => Some(JoystickType::FlightStick), // XINPUT_DEVSUBTYPE_FLIGHT_STICK
        0x05 => Some(JoystickType::DancePad), // XINPUT_DEVSUBTYPE_DANCE_PAD
        0x06 | 0x07 | 0x0B => Some(JoystickType::Guitar), // XINPUT_DEVSUBTYPE_GUITAR, _GUITAR_ALTERNATE, _GUITAR_BASS
        0x08 => Some(JoystickType::DrumKit),              // XINPUT_DEVSUBTYPE_DRUM_KIT
        0x13 => Some(JoystickType::ArcadePad),            // XINPUT_DEVSUBTYPE_ARCADE_PAD
        _ => None,
    }
}

impl Xbox360WContext {
    /// Translation of `UpdateSlotLED()`.
    fn update_slot_led(&self, device: &DeviceCtx<'_>) {
        if self.player_lights && self.player_index >= 0 {
            set_slot_led(device, (self.player_index % 4) as u8, true);
        } else {
            set_slot_led(device, 0, false);
        }
    }

    /// Translation of `SDL_PlayerLEDHintChanged()`, for a change recorded
    /// since the last call.
    fn player_led_hint_changed(&mut self, device: &mut DeviceCtx<'_>) {
        let Some(hint) = self.player_led_hint.as_ref().and_then(HintWatch::take) else {
            return;
        };
        let player_lights = hints::string_to_bool(hint.as_deref(), true);

        if player_lights != self.player_lights {
            self.player_lights = player_lights;

            self.update_slot_led(device);
            device.update_device_properties();
        }
    }

    /// Translation of `HIDAPI_DriverXbox360W_HandleStatePacket()`.
    fn handle_state_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        data: &[u8],
        size: usize,
    ) {
        const INVERT_Y_AXES: bool = true;
        let timestamp = crate::timer::ticks_ns();

        if self.last_state[2] != data[2] {
            let mut hat = 0;

            if data[2] & 0x01 != 0 {
                hat |= HAT_UP;
            }
            if data[2] & 0x02 != 0 {
                hat |= HAT_DOWN;
            }
            if data[2] & 0x04 != 0 {
                hat |= HAT_LEFT;
            }
            if data[2] & 0x08 != 0 {
                hat |= HAT_RIGHT;
            }
            device.send_hat(timestamp, joystick, 0, hat);

            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Start as u8,
                data[2] & 0x10 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Back as u8,
                data[2] & 0x20 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::LeftStick as u8,
                data[2] & 0x40 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::RightStick as u8,
                data[2] & 0x80 != 0,
            );
        }

        if self.last_state[3] != data[3] {
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::LeftShoulder as u8,
                data[3] & 0x01 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::RightShoulder as u8,
                data[3] & 0x02 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::Guide as u8,
                data[3] & 0x04 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::South as u8,
                data[3] & 0x10 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::East as u8,
                data[3] & 0x20 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::West as u8,
                data[3] & 0x40 != 0,
            );
            device.send_button(
                timestamp,
                joystick,
                GamepadButton::North as u8,
                data[3] & 0x80 != 0,
            );
        }

        send_state_axes(device, timestamp, joystick, data, INVERT_Y_AXES);

        let n = size.min(self.last_state.len());
        self.last_state[..n].copy_from_slice(&data[..n]);
    }

    /// One report of `HIDAPI_DriverXbox360W_UpdateDevice()`; `data` is the
    /// whole read buffer.
    fn handle_report(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: Option<JoystickID>,
        data: &[u8],
        size: usize,
    ) {
        if size == 2 && data[0] == 0x08 {
            let connected = data[1] & 0x80 != 0;
            if connected != self.connected {
                self.connected = connected;

                if connected {
                    device.joystick_connected();
                } else if let Some(&first) = device.joysticks().first() {
                    device.joystick_disconnected(first);
                }
            }
        } else if size == 29
            && data[0] == 0x00
            && data[1] == 0x0f
            && data[2] == 0x00
            && data[3] == 0xf0
        {
            // Serial number is data[7-13]
            if let Some(joystick) = joystick {
                update_power_level(device, joystick, data[17]);
            }
            self.capabilities.type_ = 1;
            self.capabilities.sub_type = data[25] & 0x7f;
            if data[25] & 0x80 != 0 {
                self.capabilities.flags |= FLAG_FORCE_FEEDBACK;
            }
            if let Some(joystick_type) = joystick_type_of_sub_type(data[25] & 0x7f) {
                device.set_joystick_type(joystick_type);
            }
            device.set_guid_byte(15, self.capabilities.sub_type);
            const CAPABILITIES_PACKET: [u8; 12] = [
                0x00, 0x00, 0x02, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ];
            if device.write(&CAPABILITIES_PACKET).ok() != Some(CAPABILITIES_PACKET.len()) {
                // "Couldn't write capabilities_packet packet" (the error is
                // only set upstream)
            }
        } else if size == 29
            && data[0] == 0x00
            && data[1] == 0x00
            && data[2] == 0x00
            && data[3] == 0x13
        {
            if let Some(joystick) = joystick {
                update_power_level(device, joystick, data[4]);
            }
        } else if data[0] == 0x00 && data[1] == 0x05 && data[5] == 0x12 {
            // FIXME (upstream): the size of this report isn't checked, so a
            // short one leaves bytes of an earlier report in the
            // capabilities (the buffer is always big enough).
            self.capabilities.gamepad.buttons = load16(data[6], data[7]) as u16;
            self.capabilities.gamepad.left_trigger = data[8];
            self.capabilities.gamepad.right_trigger = data[9];
            self.capabilities.gamepad.thumb_lx = load16(data[10], data[11]);
            self.capabilities.gamepad.thumb_ly = load16(data[12], data[13]);
            self.capabilities.gamepad.thumb_rx = load16(data[14], data[15]);
            self.capabilities.gamepad.thumb_ry = load16(data[16], data[17]);
            self.capabilities.flags |= u16::from(data[20]);
            self.capabilities.vibration.left_motor_speed = u16::from(data[18]) << 8;
            self.capabilities.vibration.right_motor_speed = u16::from(data[19]) << 8;
        } else if size == 29 && data[0] == 0x00 && (data[1] & 0x01) == 0x01 {
            if let Some(joystick) = joystick {
                self.handle_state_packet(device, joystick, &data[4..], size - 4);
            }
        }
    }
}

impl DriverContext for Xbox360WContext {
    /// Translation of `HIDAPI_DriverXbox360W_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        // Requests controller presence information from the wireless dongle
        const INIT_PACKET: [u8; 12] = [
            0x08, 0x00, 0x0F, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];

        device.set_device_name("Xbox 360 Wireless Controller");

        if device.write(&INIT_PACKET).ok() != Some(INIT_PACKET.len()) {
            return Err(Error::new("Couldn't write init packet"));
        }

        device.set_gamepad_type(GamepadType::Xbox360);

        Ok(())
    }

    /// Translation of `HIDAPI_DriverXbox360W_SetDevicePlayerIndex()`.
    fn set_device_player_index(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _instance_id: JoystickID,
        player_index: i32,
    ) {
        self.player_index = player_index;

        self.update_slot_led(device);
    }

    /// Translation of `HIDAPI_DriverXbox360W_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        self.last_state = [0; USB_PACKET_LENGTH];

        // Initialize player index (needed for setting LEDs)
        self.player_index = joystick_player_index_for_id(joystick.instance_id);
        self.player_lights = hints::get_bool(hints::JOYSTICK_HIDAPI_XBOX_360_PLAYER_LED, true);
        self.update_slot_led(device);

        self.player_led_hint = Some(HintWatch::new(hints::JOYSTICK_HIDAPI_XBOX_360_PLAYER_LED));
        self.player_led_hint_changed(device);

        // Initialize the joystick capabilities
        joystick.nbuttons = 11;
        joystick.naxes = GamepadAxis::COUNT;
        joystick.nhats = 1;
        joystick.connection_state = JoystickConnectionState::Wireless;
        self.capabilities.type_ = 1;
        self.capabilities.flags = FLAG_WIRELESS;
        self.capabilities.sub_type = JoystickType::Gamepad as u8;
        self.capabilities.gamepad.buttons = 0xFFFF;
        self.capabilities.gamepad.left_trigger = 0xFF;
        self.capabilities.gamepad.right_trigger = 0xFF;
        self.capabilities.gamepad.thumb_lx = 0xFFC0u16 as i16;
        self.capabilities.gamepad.thumb_ly = 0xFFC0u16 as i16;
        self.capabilities.gamepad.thumb_rx = 0xFFC0u16 as i16;
        self.capabilities.gamepad.thumb_ry = 0xFFC0u16 as i16;
        self.capabilities.vibration.left_motor_speed = 0xFFFF;
        self.capabilities.vibration.right_motor_speed = 0xFFFF;

        Ok(())
    }

    /// Translation of `HIDAPI_DriverXbox360W_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        let mut rumble_packet = [
            0x00, 0x01, 0x0f, 0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];

        rumble_packet[5] = (low_frequency_rumble >> 8) as u8;
        rumble_packet[6] = (high_frequency_rumble >> 8) as u8;

        if send_rumble(device.device(), &rumble_packet).ok() != Some(rumble_packet.len()) {
            return Err(Error::new("Couldn't send rumble packet"));
        }
        Ok(())
    }

    /// Translation of `HIDAPI_DriverXbox360W_GetJoystickCapabilities()`.
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

    /// Translation of `HIDAPI_DriverXbox360W_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        let joystick = device.open_joystick_id();

        // (the player LED hint callback of upstream)
        if joystick.is_some() {
            self.player_led_hint_changed(device);
        }

        let mut data = [0u8; USB_PACKET_LENGTH];
        let read_error = loop {
            match device.read_timeout(&mut data, 0) {
                Ok(0) => break false,
                Ok(size) => self.handle_report(device, joystick, &data, size),
                Err(_) => break true,
            }
        };

        if read_error {
            if let Some(&first) = device.joysticks().first() {
                // Read error, device is disconnected
                device.joystick_disconnected(first);
            }
        }
        !read_error
    }

    /// Translation of `HIDAPI_DriverXbox360W_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {
        self.player_led_hint = None;
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{run, test_device};
    use super::*;
    use crate::hidapi::DeviceInfo;

    #[test]
    fn supported_devices() {
        let driver = Xbox360WDriver;
        let supported = |gamepad_type, vid, pid, protocol| {
            driver.is_supported_device(None, "", gamepad_type, vid, pid, 0, 0, 0, 0, protocol)
        };
        assert!(supported(
            GamepadType::Standard,
            USB_VENDOR_MICROSOFT,
            USB_PRODUCT_XBOX360_WIRELESS_RECEIVER,
            0
        ));
        assert!(supported(GamepadType::Xbox360, 0x1234, 0x5678, 129));
        assert!(!supported(GamepadType::Xbox360, 0x1234, 0x5678, 1));
        assert!(!supported(
            GamepadType::Standard,
            USB_VENDOR_MICROSOFT,
            USB_PRODUCT_XBOX360_BIGBUTTON_RECEIVER,
            0
        ));
    }

    #[test]
    fn receiver_reports() {
        let _l = crate::test_support::test_lock();
        let _lock = crate::joystick::lock_joysticks();
        let device = test_device(&DeviceInfo {
            vendor_id: USB_VENDOR_MICROSOFT,
            product_id: USB_PRODUCT_XBOX360_WIRELESS_RECEIVER,
            ..DeviceInfo::default()
        });
        let mut ctx = Xbox360WContext::default();
        let mut data = [0u8; USB_PACKET_LENGTH];

        // A controller connects
        data[..2].copy_from_slice(&[0x08, 0x80]);
        let ((), events) = run(&device, |d| ctx.handle_report(d, None, &data, 2));
        assert_eq!(events, ["added"]);
        assert_eq!(device.num_joysticks(), 1);
        let joystick = device.joysticks()[0];

        // Its capabilities: a wheel with force feedback, and its battery
        data[..29].fill(0);
        data[..4].copy_from_slice(&[0x00, 0x0f, 0x00, 0xf0]);
        data[17] = 0x80;
        data[25] = 0x82;
        let ((), events) = run(&device, |d| ctx.handle_report(d, Some(joystick), &data, 29));
        assert_eq!(events, ["power OnBattery 50"]);
        assert_eq!(ctx.capabilities.sub_type, 2);
        assert_ne!(ctx.capabilities.flags & FLAG_FORCE_FEEDBACK, 0);
        assert_eq!(device.guid().0[15], 2);
        assert_eq!(device.state().joystick_type, JoystickType::Wheel);

        // A state report
        data[..29].fill(0);
        data[..4].copy_from_slice(&[0x00, 0x01, 0x00, 0xf0]);
        data[4 + 3] = 0x10; // A
        let ((), events) = run(&device, |d| ctx.handle_report(d, Some(joystick), &data, 29));
        assert!(events.contains(&"button 0 1".to_owned()), "{events:?}");

        // ... and disconnects
        data[..2].copy_from_slice(&[0x08, 0x00]);
        let ((), events) = run(&device, |d| ctx.handle_report(d, Some(joystick), &data, 2));
        assert_eq!(events, ["removed"]);
        assert_eq!(device.num_joysticks(), 0);
        super::super::NUMJOYSTICKS.store(0, std::sync::atomic::Ordering::Relaxed);
    }
}
