// Rust translation of src/joystick/dummy/SDL_sysjoystick.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The dummy joystick driver: no joysticks.

use super::gamepad::GamepadMapping;
use super::{JoystickData, JoystickDriver};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::guid::Guid;

/// Translation of `SDL_DUMMY_JoystickDriver`.
pub(super) struct DummyJoystickDriver;

pub(super) static DUMMY_JOYSTICK_DRIVER: DummyJoystickDriver = DummyJoystickDriver;

impl JoystickDriver for DummyJoystickDriver {
    fn init(&self) -> Result<()> {
        Ok(())
    }

    fn count(&self) -> usize {
        0
    }

    fn detect(&self) {}

    fn is_device_present(
        &self,
        _vendor_id: u16,
        _product_id: u16,
        _version: u16,
        _name: Option<&str>,
    ) -> bool {
        false
    }

    fn device_name(&self, _device_index: usize) -> Option<String> {
        None
    }

    fn device_path(&self, _device_index: usize) -> Option<String> {
        None
    }

    fn device_steam_virtual_gamepad_slot(&self, _device_index: usize) -> i32 {
        -1
    }

    fn device_player_index(&self, _device_index: usize) -> i32 {
        -1
    }

    fn set_device_player_index(&self, _device_index: usize, _player_index: i32) {}

    fn device_guid(&self, _device_index: usize) -> Guid {
        Guid::ZERO
    }

    fn device_instance_id(&self, _device_index: usize) -> JoystickID {
        0
    }

    fn open(&self, _joystick: &mut JoystickData, _device_index: usize) -> Result<()> {
        Err(Error::new("Logic error: No joysticks available"))
    }

    fn rumble(
        &self,
        _joystick: JoystickID,
        _low_frequency_rumble: u16,
        _high_frequency_rumble: u16,
    ) -> Result<()> {
        Err(Error::unsupported())
    }

    fn rumble_triggers(
        &self,
        _joystick: JoystickID,
        _left_rumble: u16,
        _right_rumble: u16,
    ) -> Result<()> {
        Err(Error::unsupported())
    }

    fn set_led(&self, _joystick: JoystickID, _red: u8, _green: u8, _blue: u8) -> Result<()> {
        Err(Error::unsupported())
    }

    fn send_effect(&self, _joystick: JoystickID, _data: &[u8]) -> Result<()> {
        Err(Error::unsupported())
    }

    fn set_sensors_enabled(&self, _joystick: JoystickID, _enabled: bool) -> Result<()> {
        Err(Error::unsupported())
    }

    fn update(&self, _joystick: JoystickID) {}

    fn close(&self, _joystick: &mut JoystickData) {}

    fn quit(&self) {}

    fn gamepad_mapping(&self, _device_index: usize) -> Option<GamepadMapping> {
        None
    }
}
