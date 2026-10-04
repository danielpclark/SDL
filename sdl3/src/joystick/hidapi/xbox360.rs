// Rust translation of src/joystick/hidapi/SDL_hidapi_xbox360.c and
// SDL_hidapi_xbox360.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The wired Xbox 360 controller driver.
//!
//! Not translated: the macOS parts (the 360Controller driver and the
//! GCController checks) and the XInput capabilities read through libusb,
//! whose backends aren't translated.

use super::rumble::send_rumble;
use super::{
    DeviceCtx, DriverContext, DriverImpl, HidapiDevice, HintWatch, JoystickCaps,
    SDL_HIDAPI_DEFAULT, USB_PACKET_LENGTH,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::gamepad::{GamepadAxis, GamepadButton, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::{
    is_joystick_steam_virtual_gamepad, joystick_player_index_for_id, JoystickData, HAT_DOWN,
    HAT_LEFT, HAT_RIGHT, HAT_UP,
};

// SDL_hidapi_xbox360.h

/// `FLAG_FORCE_FEEDBACK`
pub(crate) const FLAG_FORCE_FEEDBACK: u16 = 0x01;
/// `FLAG_WIRELESS`
pub(crate) const FLAG_WIRELESS: u16 = 0x02;

/// The gamepad part of [`XInputCapabilities`].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct XInputGamepadCapabilities {
    pub(crate) buttons: u16,
    pub(crate) left_trigger: u8,
    pub(crate) right_trigger: u8,
    pub(crate) thumb_lx: i16,
    pub(crate) thumb_ly: i16,
    pub(crate) thumb_rx: i16,
    pub(crate) thumb_ry: i16,
}

/// The vibration part of [`XInputCapabilities`].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct XInputVibrationCapabilities {
    pub(crate) left_motor_speed: u16,
    pub(crate) right_motor_speed: u16,
}

/// The XInput capabilities of a controller (kept, as upstream, though
/// nothing reads them yet). Translation of `SDL_xinput_capabilities`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct XInputCapabilities {
    pub(crate) type_: u8,
    pub(crate) sub_type: u8,
    pub(crate) flags: u16,
    pub(crate) gamepad: XInputGamepadCapabilities,
    pub(crate) vibration: XInputVibrationCapabilities,
}

/// The Xbox 360 driver's static functions.
pub(crate) struct Xbox360Driver;

impl DriverImpl for Xbox360Driver {
    /// Translation of `HIDAPI_DriverXbox360_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_XBOX, hints::JOYSTICK_HIDAPI_XBOX_360]
    }

    /// Translation of `HIDAPI_DriverXbox360_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_XBOX_360,
            hints::get_bool(
                hints::JOYSTICK_HIDAPI_XBOX,
                hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
            ),
        )
    }

    /// Translation of `HIDAPI_DriverXbox360_IsSupportedDevice()`.
    fn is_supported_device(
        &self,
        _device: Option<&HidapiDevice>,
        _name: &str,
        gamepad_type: GamepadType,
        vendor_id: u16,
        product_id: u16,
        _version: u16,
        interface_number: i32,
        _interface_class: i32,
        _interface_subclass: i32,
        interface_protocol: i32,
    ) -> bool {
        const XB360W_IFACE_PROTOCOL: i32 = 129; // Wireless

        if vendor_id == USB_VENDOR_ASTRO && product_id == USB_PRODUCT_ASTRO_C40_XBOX360 {
            // This is the ASTRO C40 in Xbox 360 mode
            return true;
        }
        if vendor_id == USB_VENDOR_NVIDIA {
            // This is the NVIDIA Shield controller which doesn't talk Xbox controller protocol
            return false;
        }
        if (vendor_id == USB_VENDOR_MICROSOFT
            && (product_id == USB_PRODUCT_XBOX360_WIRELESS_RECEIVER_THIRDPARTY2
                || product_id == USB_PRODUCT_XBOX360_WIRELESS_RECEIVER))
            || (gamepad_type == GamepadType::Xbox360 && interface_protocol == XB360W_IFACE_PROTOCOL)
        {
            // This is the wireless dongle, which talks a different protocol
            return false;
        }
        if vendor_id == USB_VENDOR_MICROSOFT && product_id == USB_PRODUCT_XBOX360_BIGBUTTON_RECEIVER
        {
            // This is the BigButton wireless receiver, which talks a different protocol
            return false;
        }
        if interface_number > 0 {
            // This is the chatpad or other input interface, not the Xbox 360 interface
            return false;
        }
        gamepad_type == GamepadType::Xbox360
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(Xbox360Context::default())
    }
}

/// Translation of `SDL_DriverXbox360_Context`.
#[derive(Debug)]
struct Xbox360Context {
    joystick: Option<JoystickID>,
    player_index: i32,
    player_lights: bool,
    #[allow(dead_code)] // (filled in through libusb upstream)
    capabilities: XInputCapabilities,
    last_state: [u8; USB_PACKET_LENGTH],
    /// The `SDL_PlayerLEDHintChanged()` callback.
    player_led_hint: Option<HintWatch>,
}

impl Default for Xbox360Context {
    fn default() -> Self {
        Xbox360Context {
            joystick: None,
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
    let led_packet = [0x01, 0x03, mode];

    device.write(&led_packet).ok() == Some(led_packet.len())
}

impl Xbox360Context {
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

    /// Translation of `HIDAPI_DriverXbox360_HandleStatePacket()`.
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
}

/// The axes of an Xbox 360 state packet (`data` from the button bytes),
/// shared with the wireless driver.
pub(crate) fn send_state_axes(
    device: &mut DeviceCtx<'_>,
    timestamp: u64,
    joystick: JoystickID,
    data: &[u8],
    invert_y_axes: bool,
) {
    let mut axis = (i32::from(data[4]) * 257 - 32768) as i16;
    device.send_axis(timestamp, joystick, GamepadAxis::LeftTrigger as u8, axis);
    axis = (i32::from(data[5]) * 257 - 32768) as i16;
    device.send_axis(timestamp, joystick, GamepadAxis::RightTrigger as u8, axis);
    axis = i16::from_le_bytes([data[6], data[7]]);
    device.send_axis(timestamp, joystick, GamepadAxis::LeftX as u8, axis);
    axis = i16::from_le_bytes([data[8], data[9]]);
    if invert_y_axes {
        axis = !axis;
    }
    device.send_axis(timestamp, joystick, GamepadAxis::LeftY as u8, axis);
    axis = i16::from_le_bytes([data[10], data[11]]);
    device.send_axis(timestamp, joystick, GamepadAxis::RightX as u8, axis);
    axis = i16::from_le_bytes([data[12], data[13]]);
    if invert_y_axes {
        axis = !axis;
    }
    device.send_axis(timestamp, joystick, GamepadAxis::RightY as u8, axis);
}

/// The slot of a Steam virtual gamepad named "GamePad-N" (`sscanf("GamePad-%d")`).
pub(crate) fn steam_virtual_gamepad_slot(product_string: &str) -> Option<i32> {
    let rest = product_string.strip_prefix("GamePad-")?;
    let (slot, len) = crate::stdlib::string::strtol(rest, 10);
    // (sscanf leaves the slot at 0 when there is no number)
    Some(if len > 0 { slot as i32 } else { 0 })
}

impl DriverContext for Xbox360Context {
    /// Translation of `HIDAPI_DriverXbox360_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        device.set_gamepad_type(GamepadType::Xbox360);

        if is_joystick_steam_virtual_gamepad(
            device.vendor_id(),
            device.product_id(),
            device.version(),
        ) {
            if let Some(slot) = device.product_string().and_then(steam_virtual_gamepad_slot) {
                device.set_steam_virtual_gamepad_slot(slot - 1);
            }
        }

        device.joystick_connected();
        Ok(())
    }

    /// Translation of `HIDAPI_DriverXbox360_SetDevicePlayerIndex()`.
    fn set_device_player_index(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _instance_id: JoystickID,
        player_index: i32,
    ) {
        if self.joystick.is_none() {
            return;
        }

        self.player_index = player_index;

        self.update_slot_led(device);
    }

    /// Translation of `HIDAPI_DriverXbox360_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        self.joystick = Some(joystick.instance_id);
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
        // (FetchXInputCapabilities() needs the libusb backend)
        Ok(())
    }

    /// Translation of `HIDAPI_DriverXbox360_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        _joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        let mut rumble_packet = [0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

        rumble_packet[3] = (low_frequency_rumble >> 8) as u8;
        rumble_packet[4] = (high_frequency_rumble >> 8) as u8;

        if send_rumble(device.device(), &rumble_packet).ok() != Some(rumble_packet.len()) {
            return Err(Error::new("Couldn't send rumble packet"));
        }
        Ok(())
    }

    /// Translation of `HIDAPI_DriverXbox360_GetJoystickCapabilities()`.
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

    /// Translation of `HIDAPI_DriverXbox360_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        let Some(&first) = device.joysticks().first() else {
            return false;
        };
        let joystick = device.joystick_open(first).then_some(first);

        // (the player LED hint callback of upstream)
        if joystick.is_some() {
            self.player_led_hint_changed(device);
        }

        let mut data = [0u8; USB_PACKET_LENGTH];
        let read_error = loop {
            match device.read_timeout(&mut data, 0) {
                Ok(0) => break false,
                Ok(size) => {
                    let Some(joystick) = joystick else {
                        continue;
                    };

                    if data[0] == 0x00 {
                        self.handle_state_packet(device, joystick, &data, size);
                    }
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

    /// Translation of `HIDAPI_DriverXbox360_CloseJoystick()`.
    fn close_joystick(&mut self, _device: &mut DeviceCtx<'_>, _joystick: JoystickID) {
        self.player_led_hint = None;

        self.joystick = None;
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{run, test_device};
    use super::*;
    use crate::hidapi::DeviceInfo;

    fn device() -> std::sync::Arc<HidapiDevice> {
        test_device(&DeviceInfo {
            vendor_id: 0x045e,
            product_id: 0x028e,
            interface_number: 0,
            ..DeviceInfo::default()
        })
    }

    #[test]
    fn supported_devices() {
        let driver = Xbox360Driver;
        let supported = |vid, pid, iface, protocol| {
            driver.is_supported_device(
                None,
                "",
                GamepadType::Xbox360,
                vid,
                pid,
                0,
                iface,
                0,
                0,
                protocol,
            )
        };
        assert!(supported(0x045e, 0x028e, 0, 0));
        // The chatpad interface, the wireless dongle and the BigButton receiver
        assert!(!supported(0x045e, 0x028e, 1, 0));
        assert!(!supported(0x045e, 0x028e, 0, 129));
        assert!(!supported(
            USB_VENDOR_MICROSOFT,
            USB_PRODUCT_XBOX360_WIRELESS_RECEIVER,
            0,
            0
        ));
        assert!(!supported(USB_VENDOR_NVIDIA, 0x7210, 0, 0));
        assert!(driver.is_supported_device(
            None,
            "",
            GamepadType::Standard,
            USB_VENDOR_ASTRO,
            USB_PRODUCT_ASTRO_C40_XBOX360,
            0,
            0,
            0,
            0,
            0
        ));
        assert_eq!(steam_virtual_gamepad_slot("GamePad-3"), Some(3));
        assert_eq!(steam_virtual_gamepad_slot("GamePad-x"), Some(0));
        assert_eq!(steam_virtual_gamepad_slot("Other"), None);
    }

    #[test]
    fn state_packets() {
        let _l = crate::test_support::test_lock();
        let device = device();
        let mut ctx = Xbox360Context::default();
        let mut data = [0u8; USB_PACKET_LENGTH];
        data[..14].copy_from_slice(&[
            0x00, 0x14, 0x11, 0x14, 0xff, 0x00, 0x00, 0x80, 0xff, 0x7f, 0x34, 0x12, 0x00, 0x00,
        ]);
        let ((), events) = run(&device, |d| ctx.handle_state_packet(d, 1, &data, 20));
        assert_eq!(
            events,
            [
                "hat 0 1",
                "button 6 1", // start
                "button 4 0",
                "button 7 0",
                "button 8 0",
                "button 9 0",
                "button 10 0",
                "button 5 1", // guide
                "button 0 1", // A
                "button 1 0",
                "button 2 0",
                "button 3 0",
                "axis 4 32767",
                "axis 5 -32768",
                "axis 0 -32768",
                "axis 1 -32768", // (inverted)
                "axis 2 4660",
                "axis 3 -1",
            ]
        );
        // Unchanged buttons aren't sent again
        let ((), events) = run(&device, |d| ctx.handle_state_packet(d, 1, &data, 20));
        assert_eq!(events.len(), 6);
    }
}
