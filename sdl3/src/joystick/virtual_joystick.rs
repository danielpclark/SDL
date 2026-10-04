// Rust translation of src/joystick/virtual/SDL_virtualjoystick.c and
// SDL_virtualjoystick_c.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The virtual joystick driver: software-only joysticks whose state the
//! application sets (see [`attach_virtual_joystick`](super::attach_virtual_joystick)).

use std::cell::RefCell;
use std::sync::Arc;

use super::gamepad::{GamepadAxis, GamepadButton, GamepadMapping, InputMapping, MappingKind};
use super::{
    assert_joysticks_locked, create_joystick_guid, private_joystick_added,
    private_joystick_removed, send_joystick_axis, send_joystick_ball, send_joystick_button,
    send_joystick_hat, send_joystick_sensor, send_joystick_touchpad, BallData, JoystickData,
    JoystickDriver, JoystickType, TouchpadFingerInfo, TouchpadInfo, HARDWARE_BUS_VIRTUAL,
    JOYSTICK_AXIS_MIN, PROP_JOYSTICK_CAP_RGB_LED_BOOLEAN, PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN,
    PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::guid::Guid;
use crate::sensor::SensorType;
use crate::thread::ReentrantMutex;

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

const AXES_CHANGED: u32 = 0x00000001;
const BALLS_CHANGED: u32 = 0x00000002;
const BUTTONS_CHANGED: u32 = 0x00000004;
const HATS_CHANGED: u32 = 0x00000008;
const TOUCHPADS_CHANGED: u32 = 0x00000010;

/// Translation of `VirtualSensorEvent`.
#[derive(Clone, Copy)]
struct VirtualSensorEvent {
    sensor_type: SensorType,
    sensor_timestamp: u64,
    data: [f32; 3],
    num_values: usize,
}

/// Data for a virtual, software-only joystick. Translation of `joystick_hwdata`.
struct HwData {
    instance_id: JoystickID,
    name: String,
    guid: Guid,
    desc: VirtualJoystickDesc,
    changes: u32,
    axes: Vec<i16>,
    buttons: Vec<bool>,
    hats: Vec<u8>,
    balls: Vec<BallData>,
    touchpads: Vec<TouchpadInfo>,
    sensors: Vec<(SensorType, f32)>,
    sensors_enabled: bool,
    sensor_events: Vec<VirtualSensorEvent>,

    /// Whether an open joystick uses this device (`hwdata->joystick`)
    opened: bool,
}

/// Translation of `g_VJoys` (guarded by `SDL_event_lock` upstream). The
/// `RefCell` borrow is never held across a callback or an event push.
static VJOYS: ReentrantMutex<RefCell<Vec<HwData>>> = ReentrantMutex::new(RefCell::new(Vec::new()));

fn with_vjoys<R>(f: impl FnOnce(&mut Vec<HwData>) -> R) -> R {
    let guard = VJOYS.lock();
    let mut vjoys = guard.borrow_mut();
    f(&mut vjoys)
}

/// Run `f` on the device of an open virtual joystick (`joystick->hwdata`).
fn with_hwdata<R>(instance_id: JoystickID, f: impl FnOnce(&mut HwData) -> R) -> Option<R> {
    assert_joysticks_locked();

    with_vjoys(|v| {
        v.iter_mut()
            .find(|h| h.instance_id == instance_id && h.opened)
            .map(f)
    })
}

/// Translation of `VIRTUAL_HWDataForIndex()`.
fn with_hwdata_for_index<R>(device_index: usize, f: impl FnOnce(&mut HwData) -> R) -> Option<R> {
    assert_joysticks_locked();

    with_vjoys(|v| v.get_mut(device_index).map(f))
}

/// Translation of `VIRTUAL_FreeHWData()`.
fn free_hwdata(instance_id: JoystickID) {
    assert_joysticks_locked();

    // Remove hwdata from SDL-global list
    let Some(mut hwdata) = with_vjoys(|v| {
        let i = v.iter().position(|h| h.instance_id == instance_id)?;
        Some(v.remove(i))
    }) else {
        return;
    };

    if let Some(cleanup) = hwdata.desc.cleanup.take() {
        cleanup();
    }
}

/// Translation of `SDL_JoystickAttachVirtualInner()`.
pub(super) fn joystick_attach_virtual_inner(mut desc: VirtualJoystickDesc) -> Result<JoystickID> {
    let mut axis_triggerleft = None;
    let mut axis_triggerright = None;

    assert_joysticks_locked();

    let name = match &desc.name {
        Some(name) => name.clone(),
        None => match desc.joystick_type {
            JoystickType::Gamepad => "Virtual Controller",
            JoystickType::Wheel => "Virtual Wheel",
            JoystickType::ArcadeStick => "Virtual Arcade Stick",
            JoystickType::FlightStick => "Virtual Flight Stick",
            JoystickType::DancePad => "Virtual Dance Pad",
            JoystickType::Guitar => "Virtual Guitar",
            JoystickType::DrumKit => "Virtual Drum Kit",
            JoystickType::ArcadePad => "Virtual Arcade Pad",
            JoystickType::Throttle => "Virtual Throttle",
            _ => "Virtual Joystick",
        }
        .to_string(),
    };

    if desc.joystick_type == JoystickType::Gamepad {
        if desc.button_mask == 0 {
            for i in 0..u32::from(desc.nbuttons).min(u32::BITS) {
                desc.button_mask |= 1 << i;
            }
        }

        if desc.axis_mask == 0 {
            if desc.naxes >= 2 {
                desc.axis_mask |=
                    (1 << GamepadAxis::LeftX as u32) | (1 << GamepadAxis::LeftY as u32);
            }
            if desc.naxes >= 4 {
                desc.axis_mask |=
                    (1 << GamepadAxis::RightX as u32) | (1 << GamepadAxis::RightY as u32);
            }
            if desc.naxes >= 6 {
                desc.axis_mask |= (1 << GamepadAxis::LeftTrigger as u32)
                    | (1 << GamepadAxis::RightTrigger as u32);
            }
        }

        // Find the trigger axes
        let mut axis = 0;
        let mut i = 0;
        while axis < usize::from(desc.naxes) && i < GamepadAxis::COUNT {
            if desc.axis_mask & (1 << i) != 0 {
                if i == GamepadAxis::LeftTrigger as usize {
                    axis_triggerleft = Some(axis);
                }
                if i == GamepadAxis::RightTrigger as usize {
                    axis_triggerright = Some(axis);
                }
                axis += 1;
            }
            i += 1;
        }
    }

    let guid = create_joystick_guid(
        HARDWARE_BUS_VIRTUAL,
        desc.vendor_id,
        desc.product_id,
        0,
        None,
        Some(&name),
        b'v',
        desc.joystick_type as u8,
    );

    // Allocate fields for different control-types
    let mut axes = vec![0i16; usize::from(desc.naxes)];

    // Trigger axes are at minimum value at rest
    if let Some(axis) = axis_triggerleft {
        axes[axis] = JOYSTICK_AXIS_MIN;
    }
    if let Some(axis) = axis_triggerright {
        axes[axis] = JOYSTICK_AXIS_MIN;
    }

    let touchpads = desc
        .touchpads
        .iter()
        .map(|touchpad_desc| TouchpadInfo {
            fingers: vec![TouchpadFingerInfo::default(); usize::from(touchpad_desc.nfingers)],
        })
        .collect();
    let sensors = desc
        .sensors
        .iter()
        .map(|s| (s.sensor_type, s.rate))
        .collect();

    let hwdata = HwData {
        // Allocate an instance ID for this device
        instance_id: crate::utils::next_object_id(),
        name,
        guid,
        changes: 0,
        axes,
        buttons: vec![false; usize::from(desc.nbuttons)],
        hats: vec![0; usize::from(desc.nhats)],
        balls: vec![BallData::default(); usize::from(desc.nballs)],
        touchpads,
        sensors,
        sensors_enabled: false,
        sensor_events: Vec::new(),
        opened: false,
        desc,
    };
    let instance_id = hwdata.instance_id;

    // Add virtual joystick to SDL-global lists
    with_vjoys(|v| v.push(hwdata));
    private_joystick_added(instance_id);

    Ok(instance_id)
}

/// Translation of `SDL_JoystickDetachVirtualInner()`.
pub(super) fn joystick_detach_virtual_inner(instance_id: JoystickID) -> Result<()> {
    if with_vjoys(|v| !v.iter().any(|h| h.instance_id == instance_id)) {
        return Err(Error::new("Virtual joystick data not found"));
    }
    free_hwdata(instance_id);
    private_joystick_removed(instance_id);
    Ok(())
}

fn invalid_joystick() -> Error {
    Error::new("Invalid joystick")
}

/// Translation of `SDL_SetJoystickVirtualAxisInner()`.
pub(super) fn set_joystick_virtual_axis_inner(
    joystick: JoystickID,
    axis: usize,
    value: i16,
) -> Result<()> {
    with_hwdata(joystick, |hwdata| {
        let Some(slot) = hwdata.axes.get_mut(axis) else {
            return Err(Error::new("Invalid axis index"));
        };
        *slot = value;
        hwdata.changes |= AXES_CHANGED;
        Ok(())
    })
    .unwrap_or_else(|| Err(invalid_joystick()))
}

/// Translation of `SDL_SetJoystickVirtualBallInner()`.
pub(super) fn set_joystick_virtual_ball_inner(
    joystick: JoystickID,
    ball: usize,
    xrel: i16,
    yrel: i16,
) -> Result<()> {
    with_hwdata(joystick, |hwdata| {
        let Some(data) = hwdata.balls.get_mut(ball) else {
            return Err(Error::new("Invalid ball index"));
        };
        data.dx += i32::from(xrel);
        data.dx = data.dx.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
        data.dy += i32::from(yrel);
        data.dy = data.dy.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
        hwdata.changes |= BALLS_CHANGED;
        Ok(())
    })
    .unwrap_or_else(|| Err(invalid_joystick()))
}

/// Translation of `SDL_SetJoystickVirtualButtonInner()`.
pub(super) fn set_joystick_virtual_button_inner(
    joystick: JoystickID,
    button: usize,
    down: bool,
) -> Result<()> {
    with_hwdata(joystick, |hwdata| {
        let Some(slot) = hwdata.buttons.get_mut(button) else {
            return Err(Error::new("Invalid button index"));
        };
        *slot = down;
        hwdata.changes |= BUTTONS_CHANGED;
        Ok(())
    })
    .unwrap_or_else(|| Err(invalid_joystick()))
}

/// Translation of `SDL_SetJoystickVirtualHatInner()`.
pub(super) fn set_joystick_virtual_hat_inner(
    joystick: JoystickID,
    hat: usize,
    value: u8,
) -> Result<()> {
    with_hwdata(joystick, |hwdata| {
        let Some(slot) = hwdata.hats.get_mut(hat) else {
            return Err(Error::new("Invalid hat index"));
        };
        *slot = value;
        hwdata.changes |= HATS_CHANGED;
        Ok(())
    })
    .unwrap_or_else(|| Err(invalid_joystick()))
}

/// Translation of `SDL_SetJoystickVirtualTouchpadInner()`.
#[allow(clippy::too_many_arguments)]
pub(super) fn set_joystick_virtual_touchpad_inner(
    joystick: JoystickID,
    touchpad: usize,
    finger: usize,
    down: bool,
    x: f32,
    y: f32,
    pressure: f32,
) -> Result<()> {
    with_hwdata(joystick, |hwdata| {
        let Some(touchpad) = hwdata.touchpads.get_mut(touchpad) else {
            return Err(Error::new("Invalid touchpad index"));
        };
        let Some(info) = touchpad.fingers.get_mut(finger) else {
            return Err(Error::new("Invalid finger index"));
        };

        info.down = down;
        info.x = x;
        info.y = y;
        info.pressure = pressure;
        hwdata.changes |= TOUCHPADS_CHANGED;
        Ok(())
    })
    .unwrap_or_else(|| Err(invalid_joystick()))
}

/// Translation of `SDL_SendJoystickVirtualSensorDataInner()`.
pub(super) fn send_joystick_virtual_sensor_data_inner(
    joystick: JoystickID,
    sensor_type: SensorType,
    sensor_timestamp: u64,
    data: &[f32],
) -> Result<()> {
    with_hwdata(joystick, |hwdata| {
        let mut event = VirtualSensorEvent {
            sensor_type,
            sensor_timestamp,
            data: [0.0; 3],
            num_values: data.len().min(3),
        };
        event.data[..event.num_values].copy_from_slice(&data[..event.num_values]);
        hwdata.sensor_events.push(event);
        Ok(())
    })
    .unwrap_or_else(|| Err(invalid_joystick()))
}

/// Translation of `SDL_VIRTUAL_JoystickDriver`.
pub(super) struct VirtualJoystickDriver;

pub(super) static VIRTUAL_JOYSTICK_DRIVER: VirtualJoystickDriver = VirtualJoystickDriver;

/// The callback of an open virtual joystick, or the error upstream reports
/// when the device is gone.
fn callback<T: ?Sized>(
    joystick: JoystickID,
    what: &str,
    pick: impl FnOnce(&VirtualJoystickDesc) -> Option<Arc<T>>,
) -> Result<Option<Arc<T>>> {
    with_hwdata(joystick, |hwdata| pick(&hwdata.desc))
        .ok_or_else(|| Error::new(format!("{what} failed, device disconnected")))
}

impl JoystickDriver for VirtualJoystickDriver {
    fn init(&self) -> Result<()> {
        Ok(())
    }

    fn count(&self) -> usize {
        assert_joysticks_locked();

        with_vjoys(|v| v.len())
    }

    fn detect(&self) {}

    fn is_device_present(
        &self,
        _vendor_id: u16,
        _product_id: u16,
        _version: u16,
        _name: Option<&str>,
    ) -> bool {
        // We don't override any other drivers... or do we?
        false
    }

    fn device_name(&self, device_index: usize) -> Option<String> {
        with_hwdata_for_index(device_index, |h| h.name.clone())
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

    fn set_device_player_index(&self, device_index: usize, player_index: i32) {
        if let Some(Some(set_player_index)) =
            with_hwdata_for_index(device_index, |h| h.desc.set_player_index.clone())
        {
            set_player_index(player_index);
        }
    }

    fn device_guid(&self, device_index: usize) -> Guid {
        with_hwdata_for_index(device_index, |h| h.guid).unwrap_or(Guid::ZERO)
    }

    fn device_instance_id(&self, device_index: usize) -> JoystickID {
        // Upstream returns `true` (instance id 1) for a missing device; it
        // reports 0, the invalid instance id, here.
        with_hwdata_for_index(device_index, |h| h.instance_id).unwrap_or(0)
    }

    fn open(&self, joystick: &mut JoystickData, device_index: usize) -> Result<()> {
        assert_joysticks_locked();

        let Some((
            naxes,
            nbuttons,
            nhats,
            touchpads,
            sensors,
            has_led,
            has_rumble,
            has_trigger_rumble,
        )) = with_hwdata_for_index(device_index, |hwdata| {
            hwdata.opened = true;
            (
                hwdata.axes.len(),
                hwdata.buttons.len(),
                hwdata.hats.len(),
                hwdata
                    .touchpads
                    .iter()
                    .map(|t| t.fingers.len())
                    .collect::<Vec<_>>(),
                hwdata.sensors.clone(),
                hwdata.desc.set_led.is_some(),
                hwdata.desc.rumble.is_some(),
                hwdata.desc.rumble_triggers.is_some(),
            )
        })
        else {
            return Err(Error::new("No such device"));
        };
        joystick.naxes = naxes;
        joystick.nbuttons = nbuttons;
        joystick.nhats = nhats;
        // FIXME (upstream): `nballs` isn't set, so ball motion is never reported

        for nfingers in touchpads {
            joystick.add_touchpad(nfingers);
        }
        for (sensor_type, rate) in sensors {
            joystick.add_sensor(sensor_type, rate);
        }

        if has_led {
            let _ = joystick
                .properties()
                .set(PROP_JOYSTICK_CAP_RGB_LED_BOOLEAN, true);
        }
        if has_rumble {
            let _ = joystick
                .properties()
                .set(PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN, true);
        }
        if has_trigger_rumble {
            let _ = joystick
                .properties()
                .set(PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN, true);
        }
        Ok(())
    }

    fn rumble(
        &self,
        joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        match callback(joystick, "Rumble", |d| d.rumble.clone())? {
            Some(rumble) => rumble(low_frequency_rumble, high_frequency_rumble),
            None => Err(Error::unsupported()),
        }
    }

    fn rumble_triggers(
        &self,
        joystick: JoystickID,
        left_rumble: u16,
        right_rumble: u16,
    ) -> Result<()> {
        match callback(joystick, "Rumble", |d| d.rumble_triggers.clone())? {
            Some(rumble_triggers) => rumble_triggers(left_rumble, right_rumble),
            None => Err(Error::unsupported()),
        }
    }

    fn set_led(&self, joystick: JoystickID, red: u8, green: u8, blue: u8) -> Result<()> {
        match callback(joystick, "SetLED", |d| d.set_led.clone())? {
            Some(set_led) => set_led(red, green, blue),
            None => Err(Error::unsupported()),
        }
    }

    fn send_effect(&self, joystick: JoystickID, data: &[u8]) -> Result<()> {
        match callback(joystick, "SendEffect", |d| d.send_effect.clone())? {
            Some(send_effect) => send_effect(data),
            None => Err(Error::unsupported()),
        }
    }

    fn set_sensors_enabled(&self, joystick: JoystickID, enabled: bool) -> Result<()> {
        let result = match callback(joystick, "SetSensorsEnabled", |d| {
            d.set_sensors_enabled.clone()
        })? {
            Some(set_sensors_enabled) => set_sensors_enabled(enabled),
            None => Ok(()),
        };
        if result.is_ok() {
            with_hwdata(joystick, |hwdata| hwdata.sensors_enabled = enabled);
        }
        result
    }

    fn update(&self, joystick: JoystickID) {
        let timestamp = crate::timer::ticks_ns();

        assert_joysticks_locked();

        let Some(update) = with_hwdata(joystick, |hwdata| hwdata.desc.update.clone()) else {
            return;
        };

        if let Some(update) = update {
            update();
        }

        struct Pending {
            changes: u32,
            axes: Vec<i16>,
            balls: Vec<(u8, i16, i16)>,
            buttons: Vec<bool>,
            hats: Vec<u8>,
            touchpads: Vec<TouchpadInfo>,
            sensor_events: Vec<VirtualSensorEvent>,
        }
        // (the changes are collected first and sent without the device
        // borrowed, since the events may call back into this driver)
        let Some(pending) = with_hwdata(joystick, |hwdata| {
            let changes = std::mem::take(&mut hwdata.changes);
            let mut balls = Vec::new();
            if changes & BALLS_CHANGED != 0 {
                for (i, ball) in hwdata.balls.iter_mut().enumerate() {
                    if ball.dx != 0 || ball.dy != 0 {
                        balls.push((i as u8, ball.dx as i16, ball.dy as i16));
                        ball.dx = 0;
                        ball.dy = 0;
                    }
                }
            }
            let sensor_events = std::mem::take(&mut hwdata.sensor_events);
            Pending {
                changes,
                axes: hwdata.axes.clone(),
                balls,
                buttons: hwdata.buttons.clone(),
                hats: hwdata.hats.clone(),
                touchpads: hwdata.touchpads.clone(),
                sensor_events: if hwdata.sensors_enabled {
                    sensor_events
                } else {
                    Vec::new()
                },
            }
        }) else {
            return;
        };

        if pending.changes & AXES_CHANGED != 0 {
            for (i, &value) in pending.axes.iter().enumerate() {
                send_joystick_axis(timestamp, joystick, i as u8, value);
            }
        }
        for (i, dx, dy) in pending.balls {
            send_joystick_ball(timestamp, joystick, i, dx, dy);
        }
        if pending.changes & BUTTONS_CHANGED != 0 {
            for (i, &down) in pending.buttons.iter().enumerate() {
                send_joystick_button(timestamp, joystick, i as u8, down);
            }
        }
        if pending.changes & HATS_CHANGED != 0 {
            for (i, &value) in pending.hats.iter().enumerate() {
                send_joystick_hat(timestamp, joystick, i as u8, value);
            }
        }
        if pending.changes & TOUCHPADS_CHANGED != 0 {
            for (i, touchpad) in pending.touchpads.iter().enumerate() {
                for (j, finger) in touchpad.fingers.iter().enumerate() {
                    send_joystick_touchpad(
                        timestamp,
                        joystick,
                        i as i32,
                        j as i32,
                        finger.down,
                        finger.x,
                        finger.y,
                        finger.pressure,
                    );
                }
            }
        }
        for event in &pending.sensor_events {
            send_joystick_sensor(
                timestamp,
                joystick,
                event.sensor_type,
                event.sensor_timestamp,
                &event.data[..event.num_values],
            );
        }
    }

    fn close(&self, joystick: &mut JoystickData) {
        assert_joysticks_locked();

        with_vjoys(|v| {
            if let Some(hwdata) = v.iter_mut().find(|h| h.instance_id == joystick.instance_id) {
                hwdata.opened = false;
            }
        });
    }

    fn quit(&self) {
        assert_joysticks_locked();

        while let Some(id) = with_vjoys(|v| v.first().map(|h| h.instance_id)) {
            free_hwdata(id);
        }
    }

    fn gamepad_mapping(&self, device_index: usize) -> Option<GamepadMapping> {
        let (joystick_type, nbuttons, naxes, button_mask, axis_mask) =
            with_hwdata_for_index(device_index, |h| {
                (
                    h.desc.joystick_type,
                    h.buttons.len(),
                    h.axes.len(),
                    h.desc.button_mask,
                    h.desc.axis_mask,
                )
            })?;
        if joystick_type != JoystickType::Gamepad {
            return None;
        }

        let mut out = GamepadMapping::default();
        let mut current_button = 0usize;
        let mut current_axis = 0usize;

        {
            let mut button = |slot: &mut InputMapping, b: GamepadButton| {
                if current_button < nbuttons && (button_mask & (1 << b as u32)) != 0 {
                    slot.kind = MappingKind::Button;
                    slot.target = current_button as u8;
                    current_button += 1;
                }
            };
            button(&mut out.a, GamepadButton::South);
            button(&mut out.b, GamepadButton::East);
            button(&mut out.x, GamepadButton::West);
            button(&mut out.y, GamepadButton::North);
            button(&mut out.back, GamepadButton::Back);
            button(&mut out.guide, GamepadButton::Guide);
            button(&mut out.start, GamepadButton::Start);
            button(&mut out.leftstick, GamepadButton::LeftStick);
            button(&mut out.rightstick, GamepadButton::RightStick);
            button(&mut out.leftshoulder, GamepadButton::LeftShoulder);
            button(&mut out.rightshoulder, GamepadButton::RightShoulder);
            button(&mut out.dpup, GamepadButton::DpadUp);
            button(&mut out.dpdown, GamepadButton::DpadDown);
            button(&mut out.dpleft, GamepadButton::DpadLeft);
            button(&mut out.dpright, GamepadButton::DpadRight);
            button(&mut out.misc1, GamepadButton::Misc1);
            button(&mut out.right_paddle1, GamepadButton::RightPaddle1);
            button(&mut out.left_paddle1, GamepadButton::LeftPaddle1);
            button(&mut out.right_paddle2, GamepadButton::RightPaddle2);
            button(&mut out.left_paddle2, GamepadButton::LeftPaddle2);
            button(&mut out.touchpad, GamepadButton::Touchpad);
            button(&mut out.misc2, GamepadButton::Misc2);
            button(&mut out.misc3, GamepadButton::Misc3);
            button(&mut out.misc4, GamepadButton::Misc4);
            button(&mut out.misc5, GamepadButton::Misc5);
            button(&mut out.misc6, GamepadButton::Misc6);
        }

        let mut axis = |slot: &mut InputMapping, a: GamepadAxis| {
            if current_axis < naxes && (axis_mask & (1 << a as u32)) != 0 {
                slot.kind = MappingKind::Axis;
                slot.target = current_axis as u8;
                current_axis += 1;
            }
        };
        axis(&mut out.leftx, GamepadAxis::LeftX);
        axis(&mut out.lefty, GamepadAxis::LeftY);
        axis(&mut out.rightx, GamepadAxis::RightX);
        axis(&mut out.righty, GamepadAxis::RightY);
        axis(&mut out.lefttrigger, GamepadAxis::LeftTrigger);
        axis(&mut out.righttrigger, GamepadAxis::RightTrigger);

        Some(out)
    }
}
