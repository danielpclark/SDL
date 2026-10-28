// Rust translation of the virtual joystick descriptions in
// include/SDL3/SDL_joystick.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The descriptions of virtual joysticks ([`VirtualJoystickDesc`]), which
//! [`attach_virtual_joystick`](super::attach_virtual_joystick) takes.
//! (Public on every platform, as upstream's header has them; the driver is
//! [`super::virtual_joystick`]'s, where it's built.)

use std::sync::Arc;

use super::JoystickType;
use crate::error::Result;
use crate::sensor::SensorType;

/// Called when the joystick state should be updated.
pub type VirtualUpdateFn = dyn Fn() + Send + Sync;
/// Called when the player index is set.
pub type VirtualSetPlayerIndexFn = dyn Fn(i32) + Send + Sync;
/// Implements [`Joystick::rumble`](super::Joystick::rumble) and
/// [`Joystick::rumble_triggers`](super::Joystick::rumble_triggers).
pub type VirtualRumbleFn = dyn Fn(u16, u16) -> Result<()> + Send + Sync;
/// Implements [`Joystick::set_led`](super::Joystick::set_led).
pub type VirtualSetLedFn = dyn Fn(u8, u8, u8) -> Result<()> + Send + Sync;
/// Implements [`Joystick::send_effect`](super::Joystick::send_effect).
pub type VirtualSendEffectFn = dyn Fn(&[u8]) -> Result<()> + Send + Sync;
/// Implements [`Joystick::set_sensor_enabled`](super::Joystick::set_sensor_enabled).
pub type VirtualSetSensorsEnabledFn = dyn Fn(bool) -> Result<()> + Send + Sync;

/// The structure that describes a virtual joystick touchpad.
/// Translation of `SDL_VirtualJoystickTouchpadDesc`.
#[derive(Clone, Copy, Debug, Default)]
pub struct VirtualJoystickTouchpadDesc {
    /// the number of simultaneous fingers on this touchpad
    pub nfingers: u16,
}

/// The structure that describes a virtual joystick sensor.
/// Translation of `SDL_VirtualJoystickSensorDesc`.
#[derive(Clone, Copy, Debug)]
pub struct VirtualJoystickSensorDesc {
    /// the type of this sensor
    pub sensor_type: SensorType,
    /// the update frequency of this sensor, may be 0.0
    pub rate: f32,
}

/// The structure that describes a virtual joystick.
/// Translation of `SDL_VirtualJoystickDesc`.
///
/// The callbacks replace the C function pointers and their `userdata`; they
/// are called with the joystick lock held, and may call back into the
/// joystick API. `cleanup` runs when the joystick is detached.
#[derive(Default)]
pub struct VirtualJoystickDesc {
    /// The type of this joystick
    pub joystick_type: JoystickType,
    /// the USB vendor ID of this joystick
    pub vendor_id: u16,
    /// the USB product ID of this joystick
    pub product_id: u16,
    /// the number of axes on this joystick
    pub naxes: u16,
    /// the number of buttons on this joystick
    pub nbuttons: u16,
    /// the number of balls on this joystick
    pub nballs: u16,
    /// the number of hats on this joystick
    pub nhats: u16,
    /// the touchpads on this joystick
    pub touchpads: Vec<VirtualJoystickTouchpadDesc>,
    /// the sensors on this joystick
    pub sensors: Vec<VirtualJoystickSensorDesc>,
    /// A mask of which buttons are valid for this controller,
    /// e.g. `1 << GamepadButton::South as u32`
    pub button_mask: u32,
    /// A mask of which axes are valid for this controller,
    /// e.g. `1 << GamepadAxis::LeftX as u32`
    pub axis_mask: u32,
    /// the name of the joystick
    pub name: Option<String>,
    /// Called when the joystick state should be updated
    pub update: Option<Arc<VirtualUpdateFn>>,
    /// Called when the player index is set
    pub set_player_index: Option<Arc<VirtualSetPlayerIndexFn>>,
    /// Implements `Joystick::rumble()`
    pub rumble: Option<Arc<VirtualRumbleFn>>,
    /// Implements `Joystick::rumble_triggers()`
    pub rumble_triggers: Option<Arc<VirtualRumbleFn>>,
    /// Implements `Joystick::set_led()`
    pub set_led: Option<Arc<VirtualSetLedFn>>,
    /// Implements `Joystick::send_effect()`
    pub send_effect: Option<Arc<VirtualSendEffectFn>>,
    /// Implements `Gamepad::set_sensor_enabled()`
    pub set_sensors_enabled: Option<Arc<VirtualSetSensorsEnabledFn>>,
    /// Cleans up when the joystick is detached
    pub cleanup: Option<Box<dyn FnOnce() + Send>>,
}

impl std::fmt::Debug for VirtualJoystickDesc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtualJoystickDesc")
            .field("joystick_type", &self.joystick_type)
            .field("vendor_id", &self.vendor_id)
            .field("product_id", &self.product_id)
            .field("naxes", &self.naxes)
            .field("nbuttons", &self.nbuttons)
            .field("nballs", &self.nballs)
            .field("nhats", &self.nhats)
            .field("touchpads", &self.touchpads)
            .field("sensors", &self.sensors)
            .field("button_mask", &self.button_mask)
            .field("axis_mask", &self.axis_mask)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}
