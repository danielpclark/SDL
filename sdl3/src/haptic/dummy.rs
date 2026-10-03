// Rust translation of src/haptic/dummy/SDL_syshaptic.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The dummy haptic driver: no haptic devices.

use super::{HapticData, HapticDriver, HapticEffect, HapticID};
use crate::error::{Error, Result};
use crate::joystick::Joystick;

/// Translation of `SDL_SYS_LogicError()`.
fn logic_error() -> Error {
    Error::new("Logic error: No haptic devices available.")
}

pub(super) struct DummyHapticDriver;

pub(super) static DUMMY_HAPTIC_DRIVER: DummyHapticDriver = DummyHapticDriver;

impl HapticDriver for DummyHapticDriver {
    fn init(&self) -> Result<()> {
        Ok(())
    }

    fn count(&self) -> usize {
        0
    }

    fn instance_id(&self, _index: usize) -> HapticID {
        // (upstream also sets the logic error)
        0
    }

    fn name(&self, _index: usize) -> Option<String> {
        None
    }

    fn open(&self, _haptic: &mut HapticData) -> Result<()> {
        Err(logic_error())
    }

    fn mouse(&self) -> Option<usize> {
        None
    }

    fn joystick_is_haptic(&self, _joystick: &Joystick) -> bool {
        false
    }

    fn open_from_joystick(&self, _haptic: &mut HapticData, _joystick: &Joystick) -> Result<()> {
        Err(logic_error())
    }

    fn joystick_same_haptic(&self, _haptic: &HapticData, _joystick: &Joystick) -> bool {
        false
    }

    fn close(&self, _haptic: &mut HapticData) {}

    fn quit(&self) {}

    fn new_effect(
        &self,
        _haptic: &mut HapticData,
        _effect: usize,
        _base: &HapticEffect,
    ) -> Result<()> {
        Err(logic_error())
    }

    fn update_effect(
        &self,
        _haptic: &mut HapticData,
        _effect: usize,
        _data: &HapticEffect,
    ) -> Result<()> {
        Err(logic_error())
    }

    fn run_effect(&self, _haptic: &mut HapticData, _effect: usize, _iterations: u32) -> Result<()> {
        Err(logic_error())
    }

    fn stop_effect(&self, _haptic: &mut HapticData, _effect: usize) -> Result<()> {
        Err(logic_error())
    }

    fn destroy_effect(&self, _haptic: &mut HapticData, _effect: usize) {}

    fn effect_status(&self, _haptic: &mut HapticData, _effect: usize) -> Result<bool> {
        Err(logic_error())
    }

    fn set_gain(&self, _haptic: &mut HapticData, _gain: i32) -> Result<()> {
        Err(logic_error())
    }

    fn set_autocenter(&self, _haptic: &mut HapticData, _autocenter: i32) -> Result<()> {
        Err(logic_error())
    }

    fn pause(&self, _haptic: &mut HapticData) -> Result<()> {
        Err(logic_error())
    }

    fn resume(&self, _haptic: &mut HapticData) -> Result<()> {
        Err(logic_error())
    }

    fn stop_all(&self, _haptic: &mut HapticData) -> Result<()> {
        Err(logic_error())
    }
}
