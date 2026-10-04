// Rust translation of src/joystick/hidapi/SDL_hidapi_gamecube.c from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Nintendo GameCube controller adapter driver: the Wii U adapter
//! (WUP-028), with up to four controllers, and the EVORETRO adapters in PC
//! mode.
//!
//! Not translated: the libusb control transfer that enables input on Nyko
//! and EVORETRO adapters, as the libusb backend isn't translated (upstream
//! skips it too when built without libusb).

use super::rumble::send_rumble;
use super::{
    remap_val, DeviceCtx, DriverContext, DriverImpl, HidapiDevice, HintWatch, JoystickCaps,
    SDL_HIDAPI_DEFAULT, USB_PACKET_LENGTH,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::hints;
use crate::joystick::gamepad::{GamepadAxis, GamepadType};
use crate::joystick::usb_ids::*;
use crate::joystick::{with_joystick, JoystickConnectionState, JoystickData};

const MAX_CONTROLLERS: usize = 4;
const PC_RUMBLE_REFRESH: u64 = 16;

/// Whether each slot of an adapter in PC mode is a separate device
/// (`SDL_PLATFORM_WIN32`).
const SEPARATE_SLOT_DEVICES: bool = cfg!(windows);

/// The GameCube driver's static functions.
pub(crate) struct GameCubeDriver;

impl DriverImpl for GameCubeDriver {
    /// Translation of `HIDAPI_DriverGameCube_RegisterHints()`.
    fn hints(&self) -> &'static [&'static str] {
        &[hints::JOYSTICK_HIDAPI_GAMECUBE]
    }

    /// Translation of `HIDAPI_DriverGameCube_IsEnabled()`.
    fn is_enabled(&self) -> bool {
        hints::get_bool(
            hints::JOYSTICK_HIDAPI_GAMECUBE,
            hints::get_bool(hints::JOYSTICK_HIDAPI, SDL_HIDAPI_DEFAULT),
        )
    }

    /// Translation of `HIDAPI_DriverGameCube_IsSupportedDevice()`.
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
        if vendor_id == USB_VENDOR_NINTENDO && product_id == USB_PRODUCT_NINTENDO_GAMECUBE_ADAPTER {
            // Nintendo Co., Ltd.  Wii U GameCube Controller Adapter
            return true;
        }
        if vendor_id == USB_VENDOR_DRAGONRISE
            && (product_id == USB_PRODUCT_EVORETRO_GAMECUBE_ADAPTER1
                || product_id == USB_PRODUCT_EVORETRO_GAMECUBE_ADAPTER2
                || product_id == USB_PRODUCT_EVORETRO_GAMECUBE_ADAPTER3)
        {
            // EVORETRO GameCube Controller Adapter
            return true;
        }
        false
    }

    fn new_context(&self) -> Box<dyn DriverContext> {
        Box::new(GameCubeContext::default())
    }
}

/// Translation of `SDL_DriverGameCube_Context`.
#[derive(Debug)]
struct GameCubeContext {
    pc_mode: bool,
    /// The joystick of each slot, 0 when there is none.
    joysticks: [JoystickID; MAX_CONTROLLERS],
    wireless: [bool; MAX_CONTROLLERS],
    min_axis: [u8; MAX_CONTROLLERS * GamepadAxis::COUNT],
    max_axis: [u8; MAX_CONTROLLERS * GamepadAxis::COUNT],
    rumble_allowed: [bool; MAX_CONTROLLERS],
    rumble: [u8; 1 + MAX_CONTROLLERS],
    /// Without this variable, hid_write starts to lag a TON
    rumble_update: bool,
    use_rumble_brake: bool,
    rumble_active: bool,
    pc_rumble_sent: u64,
    /// The `SDL_JoystickGameCubeRumbleBrakeHintChanged()` callback.
    rumble_brake_hint: Option<HintWatch>,
}

impl Default for GameCubeContext {
    fn default() -> Self {
        GameCubeContext {
            pc_mode: false,
            joysticks: [0; MAX_CONTROLLERS],
            wireless: [false; MAX_CONTROLLERS],
            min_axis: [0; MAX_CONTROLLERS * GamepadAxis::COUNT],
            max_axis: [0; MAX_CONTROLLERS * GamepadAxis::COUNT],
            rumble_allowed: [false; MAX_CONTROLLERS],
            rumble: [0; 1 + MAX_CONTROLLERS],
            rumble_update: false,
            use_rumble_brake: false,
            rumble_active: false,
            pc_rumble_sent: 0,
            rumble_brake_hint: None,
        }
    }
}

/// Whether a slot's joystick is open (`SDL_GetJoystickFromID() != NULL`).
fn joystick_is_open(joystick: JoystickID) -> bool {
    joystick != 0 && with_joystick(joystick, |_| ()).is_some()
}

/// Translation of `HIDAPI_DriverGameCube_EnableAdapter()` (without its
/// libusb part).
fn enable_adapter(device: &DeviceCtx<'_>) -> Result<()> {
    let init_magic = [0x13];
    if device.write(&init_magic).ok() != Some(init_magic.len()) {
        crate::log::debug!(
            crate::log::Category::Input,
            "HIDAPI_DriverGameCube_InitDevice(): Couldn't initialize WUP-028"
        );
        return Err(Error::new("Couldn't initialize WUP-028"));
    }

    // Wait for the adapter to initialize
    crate::timer::delay(std::time::Duration::from_millis(10));

    Ok(())
}

impl GameCubeContext {
    /// Translation of `ResetAxisRange()`.
    fn reset_axis_range(&mut self, joystick_index: usize) {
        let axes = joystick_index * GamepadAxis::COUNT..(joystick_index + 1) * GamepadAxis::COUNT;
        self.min_axis[axes.clone()].fill(128 - 88);
        self.max_axis[axes].fill(128 + 88);

        // Trigger axes may have a higher resting value
        self.min_axis[joystick_index * GamepadAxis::COUNT + GamepadAxis::LeftTrigger as usize] = 40;
        self.min_axis[joystick_index * GamepadAxis::COUNT + GamepadAxis::RightTrigger as usize] =
            40;
    }

    /// Translation of `SDL_JoystickGameCubeRumbleBrakeHintChanged()`, for a
    /// change recorded since the last call.
    fn rumble_brake_hint_changed(&mut self) {
        if let Some(Some(hint)) = self.rumble_brake_hint.as_ref().and_then(HintWatch::take) {
            self.use_rumble_brake = hints::string_to_bool(Some(hint.as_str()), false);
        }
    }

    /// Connect the joysticks of an adapter in PC mode (part of
    /// `HIDAPI_DriverGameCube_InitDevice()`).
    fn connect_pc_slots(&mut self, device: &mut DeviceCtx<'_>, rumble_allowed: bool) {
        // (with a separate device for each slot, only the first is used)
        let slots = if SEPARATE_SLOT_DEVICES {
            1
        } else {
            MAX_CONTROLLERS
        };
        for i in 0..slots {
            self.rumble_allowed[i] = rumble_allowed;
            self.reset_axis_range(i);
            self.joysticks[i] = device.joystick_connected();
        }
    }

    /// The slot of an open joystick.
    fn slot_of(&self, joystick: JoystickID) -> Option<usize> {
        self.joysticks.iter().position(|&id| id == joystick)
    }

    /// Update a slot from its status byte in an adapter report, connecting
    /// or disconnecting its joystick; whether a controller is plugged in.
    fn update_slot(&mut self, device: &mut DeviceCtx<'_>, i: usize, status: u8) -> bool {
        self.wireless[i] = status & 0x20 != 0;

        // Only allow rumble if the adapter's second USB cable is connected
        self.rumble_allowed[i] = status & 0x04 != 0 && !self.wireless[i];

        if status & 0x30 != 0 {
            // 0x10 - Wired, 0x20 - Wireless
            if self.joysticks[i] == 0 {
                self.reset_axis_range(i);
                self.joysticks[i] = device.joystick_connected();
            }
            true
        } else {
            if self.joysticks[i] != 0 {
                device.joystick_disconnected(self.joysticks[i]);
                self.joysticks[i] = 0;
            }
            false
        }
    }

    /// An axis value, widening the axis range to it (`READ_AXIS()`).
    fn read_axis(&mut self, i: usize, axis: GamepadAxis, v: u8) -> i16 {
        let index = i * GamepadAxis::COUNT + axis as usize;
        if v < self.min_axis[index] {
            self.min_axis[index] = v;
        }
        if v > self.max_axis[index] {
            self.max_axis[index] = v;
        }
        remap_val(
            f32::from(v),
            f32::from(self.min_axis[index]),
            f32::from(self.max_axis[index]),
            f32::from(i16::MIN),
            f32::from(i16::MAX),
        ) as i16
    }

    /// Translation of `HIDAPI_DriverGameCube_HandleJoystickPacket()`.
    /// `is_open` tells whether a joystick is open.
    fn handle_joystick_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        slot: u8,
        packet: &[u8],
        invert_c_stick: bool,
        is_open: impl Fn(JoystickID) -> bool,
    ) {
        let timestamp = crate::timer::ticks_ns();

        let i = if SEPARATE_SLOT_DEVICES {
            // We get a separate device for each slot
            0
        } else {
            let i = usize::from(slot);
            if i >= MAX_CONTROLLERS {
                // Invalid packet?
                return;
            }
            i
        };

        let joystick = self.joysticks[i];
        if !is_open(joystick) {
            // Hasn't been opened yet, skip
            return;
        }

        // (READ_BUTTON())
        const BUTTONS: [(usize, u8, u8); 12] = [
            (0, 0x02, 0), // A
            (0, 0x04, 1), // B
            (0, 0x08, 3), // Y
            (0, 0x01, 2), // X
            (1, 0x80, 4), // DPAD_LEFT
            (1, 0x20, 5), // DPAD_RIGHT
            (1, 0x40, 6), // DPAD_DOWN
            (1, 0x10, 7), // DPAD_UP
            (1, 0x02, 8), // START
            (0, 0x80, 9), // RIGHTSHOULDER
            /* These two buttons are for the bottoms of the analog triggers.
             * More than likely, you're going to want to read the axes instead!
             * -flibit
             */
            (0, 0x20, 10), // TRIGGERRIGHT
            (0, 0x10, 11), // TRIGGERLEFT
        ];
        for (off, flag, button) in BUTTONS {
            device.send_button(timestamp, joystick, button, packet[off] & flag != 0);
        }

        let axes = [
            (2, GamepadAxis::LeftX, false),
            (3, GamepadAxis::LeftY, true),
            (5, GamepadAxis::RightX, invert_c_stick),
            (4, GamepadAxis::RightY, !invert_c_stick),
            (6, GamepadAxis::LeftTrigger, false),
            (7, GamepadAxis::RightTrigger, false),
        ];
        for (off, axis, invert) in axes {
            let v = if invert {
                0xff - packet[off]
            } else {
                packet[off]
            };
            let axis_value = self.read_axis(i, axis, v);
            device.send_axis(timestamp, joystick, axis as u8, axis_value);
        }
    }

    /// Translation of `HIDAPI_DriverGameCube_HandleNintendoPacket()`.
    /// `is_open` tells whether a joystick is open.
    fn handle_nintendo_packet(
        &mut self,
        device: &mut DeviceCtx<'_>,
        packet: &[u8],
        size: usize,
        is_open: impl Fn(JoystickID) -> bool,
    ) {
        let timestamp = crate::timer::ticks_ns();

        if size < 37 || packet[0] != 0x21 {
            return; // Nothing to do right now...?
        }

        // Go through all 4 slots
        for i in 0..MAX_CONTROLLERS {
            let cur_slot = &packet[1 + i * 9..1 + (i + 1) * 9];

            if !self.update_slot(device, i, cur_slot[0]) {
                continue;
            }
            let joystick = self.joysticks[i];

            // Hasn't been opened yet, skip
            if !is_open(joystick) {
                continue;
            }

            // (READ_BUTTON())
            const BUTTONS: [(usize, u8, u8); 12] = [
                (1, 0x01, 0), // A
                (1, 0x02, 1), // B
                (1, 0x04, 2), // X
                (1, 0x08, 3), // Y
                (1, 0x10, 4), // DPAD_LEFT
                (1, 0x20, 5), // DPAD_RIGHT
                (1, 0x40, 6), // DPAD_DOWN
                (1, 0x80, 7), // DPAD_UP
                (2, 0x01, 8), // START
                (2, 0x02, 9), // RIGHTSHOULDER
                /* These two buttons are for the bottoms of the analog triggers.
                 * More than likely, you're going to want to read the axes instead!
                 * -flibit
                 */
                (2, 0x04, 10), // TRIGGERRIGHT
                (2, 0x08, 11), // TRIGGERLEFT
            ];
            for (off, flag, button) in BUTTONS {
                device.send_button(timestamp, joystick, button, cur_slot[off] & flag != 0);
            }

            let axes = [
                (3, GamepadAxis::LeftX),
                (4, GamepadAxis::LeftY),
                (5, GamepadAxis::RightX),
                (6, GamepadAxis::RightY),
                (7, GamepadAxis::LeftTrigger),
                (8, GamepadAxis::RightTrigger),
            ];
            for (off, axis) in axes {
                let axis_value = self.read_axis(i, axis, cur_slot[off]);
                device.send_axis(timestamp, joystick, axis as u8, axis_value);
            }
        }
    }

    /// One report of `HIDAPI_DriverGameCube_UpdateDevice()`.
    fn handle_report(
        &mut self,
        device: &mut DeviceCtx<'_>,
        packet: &[u8],
        size: usize,
        is_open: impl Fn(JoystickID) -> bool,
    ) {
        if self.pc_mode {
            if size == 10 {
                // This is the older firmware
                // The first byte is the index of the connected controller
                // The C stick has an inverted value range compared to the left stick
                self.handle_joystick_packet(
                    device,
                    packet[0].wrapping_sub(1),
                    &packet[1..],
                    true,
                    is_open,
                );
            } else if size == 9 {
                // This is the newer firmware (version 0x7)
                // The C stick has the same value range compared to the left stick
                self.handle_joystick_packet(device, 0, packet, false, is_open);
            } else {
                // How do we handle this packet?
            }
        } else {
            self.handle_nintendo_packet(device, packet, size, is_open);
        }
    }

    /// The PC mode rumble report (the packet of
    /// `HIDAPI_DriverGameCube_SendPCRumble()`).
    fn pc_rumble_packet(&self) -> [u8; 3] {
        let motor = if self.rumble_active { 0xFF } else { 0x00 };
        [0x00, motor, motor]
    }

    /// Translation of `HIDAPI_DriverGameCube_SendPCRumble()`.
    fn send_pc_rumble(&mut self, device: &mut DeviceCtx<'_>) {
        let rumblepkt = self.pc_rumble_packet();
        let _ = send_rumble(device.device(), &rumblepkt);
        self.pc_rumble_sent = crate::timer::ticks_ms();
    }

    /// The rumble state change of `HIDAPI_DriverGameCube_RumbleJoystick()`;
    /// whether to send the PC mode rumble report now.
    fn update_rumble(
        &mut self,
        joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<bool> {
        let Some(i) = self.slot_of(joystick) else {
            // Should never get here!
            return Err(Error::new("Couldn't find joystick"));
        };

        if self.pc_mode {
            if !self.rumble_allowed[i] {
                return Err(Error::new(
                    "Rumble is disabled because the adapter is not at least of firmware 0x7",
                ));
            }
            let mut coast = false;
            if self.use_rumble_brake {
                coast = low_frequency_rumble == 0 && high_frequency_rumble > 0;
            }
            let shouldrumble = low_frequency_rumble > 0 || high_frequency_rumble > 0;
            if coast {
                self.rumble_active = false;
            } else if shouldrumble != self.rumble_active {
                self.rumble_active = shouldrumble;
                return Ok(true);
            }
            Ok(false)
        } else {
            if self.wireless[i] {
                return Err(Error::new(
                    "Nintendo GameCube WaveBird controllers do not support rumble",
                ));
            }
            if !self.rumble_allowed[i] {
                return Err(Error::new("Second USB cable for WUP-028 not connected"));
            }
            let val = if self.use_rumble_brake {
                if low_frequency_rumble == 0 && high_frequency_rumble > 0 {
                    0 // if only low is 0 we want to do a regular stop
                } else if low_frequency_rumble == 0 && high_frequency_rumble == 0 {
                    2 // if both frequencies are 0 we want to do a hard stop
                } else {
                    1 // normal rumble
                }
            } else {
                u8::from(low_frequency_rumble > 0 || high_frequency_rumble > 0)
            };
            if val != self.rumble[i + 1] {
                self.rumble[i + 1] = val;
                self.rumble_update = true;
            }
            Ok(false)
        }
    }
}

impl DriverContext for GameCubeContext {
    /// Translation of `HIDAPI_DriverGameCube_InitDevice()`.
    fn init_device(&mut self, device: &mut DeviceCtx<'_>) -> Result<()> {
        const RUMBLE_MAGIC: u8 = 0x11;
        let mut packet = [0u8; 37];

        self.rumble[0] = RUMBLE_MAGIC;

        if device.vendor_id() != USB_VENDOR_NINTENDO {
            self.pc_mode = true;
        }

        if self.pc_mode {
            // Check to see if this firmware supports rumble
            let mut rumble_allowed = false;
            if let Ok(size @ 1..) = device.read_timeout(&mut packet, 0) {
                if size == 9 {
                    // This is firmware version 0x7 or newer
                    // Rumble is supported if the second USB cable is plugged in
                    rumble_allowed = true;
                }
            }
            self.connect_pc_slots(device, rumble_allowed);
        } else {
            enable_adapter(device)?;

            // Add all the applicable joysticks
            while let Ok(size @ 1..) = device.read_timeout(&mut packet, 0) {
                if size < 37 || packet[0] != 0x21 {
                    continue; // Nothing to do yet...?
                }

                // Go through all 4 slots
                for i in 0..MAX_CONTROLLERS {
                    self.update_slot(device, i, packet[1 + i * 9]);
                }
            }
        }

        self.rumble_brake_hint = Some(HintWatch::new(hints::JOYSTICK_HIDAPI_GAMECUBE_RUMBLE_BRAKE));
        self.rumble_brake_hint_changed();

        device.set_device_name("Nintendo GameCube Controller");

        Ok(())
    }

    /// Translation of `HIDAPI_DriverGameCube_GetDevicePlayerIndex()`.
    fn get_device_player_index(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        instance_id: JoystickID,
    ) -> i32 {
        self.slot_of(instance_id).map_or(-1, |i| i as i32)
    }

    /// Translation of `HIDAPI_DriverGameCube_UpdateDevice()`.
    fn update_device(&mut self, device: &mut DeviceCtx<'_>) -> bool {
        // (the rumble brake hint callback of upstream)
        self.rumble_brake_hint_changed();

        // Read input packet
        let mut packet = [0u8; USB_PACKET_LENGTH];
        while let Ok(size @ 1..) = device.read_timeout(&mut packet, 0) {
            self.handle_report(device, &packet, size, joystick_is_open);
        }

        // PC_Mode rumble needs constant packets in order to keep rumble going
        if self.pc_mode
            && self.rumble_active
            && crate::timer::ticks_ms() >= self.pc_rumble_sent + PC_RUMBLE_REFRESH
        {
            self.send_pc_rumble(device);
        }

        // Write rumble packet
        if self.rumble_update {
            let _ = send_rumble(device.device(), &self.rumble);
            self.rumble_update = false;
        }

        // If we got here, nothing bad happened!
        true
    }

    /// Translation of `HIDAPI_DriverGameCube_OpenJoystick()`.
    fn open_joystick(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        joystick: &mut JoystickData,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        let Some(i) = self.slot_of(joystick.instance_id) else {
            // Should never get here! (upstream fails without an error)
            return Err(Error::new("Couldn't find joystick"));
        };
        joystick.nbuttons = 12;
        joystick.naxes = GamepadAxis::COUNT;
        joystick.connection_state = if self.wireless[i] {
            JoystickConnectionState::Wireless
        } else {
            JoystickConnectionState::Wired
        };
        Ok(())
    }

    /// Translation of `HIDAPI_DriverGameCube_RumbleJoystick()`.
    fn rumble_joystick(
        &mut self,
        device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        crate::joystick::assert_joysticks_locked();

        // (the rumble brake hint callback of upstream)
        self.rumble_brake_hint_changed();

        if self.update_rumble(joystick, low_frequency_rumble, high_frequency_rumble)? {
            self.send_pc_rumble(device);
        }
        Ok(())
    }

    /// Translation of `HIDAPI_DriverGameCube_GetJoystickCapabilities()`.
    fn get_joystick_capabilities(
        &mut self,
        _device: &mut DeviceCtx<'_>,
        joystick: JoystickID,
    ) -> JoystickCaps {
        crate::joystick::assert_joysticks_locked();

        let mut result = JoystickCaps(0);
        if let Some(i) = self.slot_of(joystick) {
            if self.rumble_allowed[i] && (self.pc_mode || !self.wireless[i]) {
                result |= JoystickCaps::RUMBLE;
            }
        }

        result
    }

    /// Translation of `HIDAPI_DriverGameCube_CloseJoystick()`.
    fn close_joystick(&mut self, device: &mut DeviceCtx<'_>, _joystick: JoystickID) {
        // Stop rumble activity
        if self.rumble_update {
            let _ = send_rumble(device.device(), &self.rumble);
            self.rumble_update = false;
        }

        if self.pc_mode && self.rumble_active {
            self.rumble_active = false;
            self.send_pc_rumble(device);
        }
    }

    /// Translation of `HIDAPI_DriverGameCube_FreeDevice()`.
    fn free_device(&mut self, _device: &mut DeviceCtx<'_>) {
        self.rumble_brake_hint = None;
    }
}

#[cfg(test)]
mod tests;
