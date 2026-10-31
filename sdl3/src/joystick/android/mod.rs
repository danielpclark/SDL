// Rust translation of src/joystick/android/SDL_sysjoystick.c and
// src/joystick/android/SDL_sysjoystick_c.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Android joystick driver: the input devices that the Java side's
//! joystick handler (`SDLControllerManager.java`'s `SDLJoystickHandler`,
//! `crate::core::android::java::controller_manager`) finds game
//! controllers.
//!
//! The handler enumerates the devices when the driver initializes
//! (`Android_JNI_DetectDevices()`), and its input device listener tells of
//! those added and removed afterwards, on the main thread; each comes with
//! its name, descriptor, USB IDs, a mask of the gamepad buttons it has
//! (`InputDevice.hasKeys()`), its axes (the joystick class's motion ranges,
//! sorted as SDL's gamepad axes go), its hats (each a pair of `AXIS_HAT_X`
//! and `AXIS_HAT_Y`), and, on API 31 and later, whether it has vibrators,
//! RGB lights, an accelerometer and a gyroscope. The GUID is a Bluetooth
//! one, from the USB IDs and the descriptor, with the button and axis
//! masks in its last four bytes, which SDL_gamepad.c's Android mapping
//! guesses a gamepad mapping from. Hats are sent as the four d-pad
//! buttons.
//!
//! Then the activity's key events of those devices come as
//! `onNativePadDown()` and `onNativePadUp()` (a key code mapped to a
//! button, else a scan code: the left Joy-Con's d-pad), sent as the open
//! joystick's buttons, or as keyboard keys when the joystick isn't open (a
//! TV remote's d-pad, ...); and the surface's generic motion events of a
//! joystick come as `onNativeJoy()` (the axes, normalized to -1 to 1 by
//! Java) and `onNativeHat()`. Rumble is the device's vibrators (through the
//! haptic handler), the LED its lights session, and the sensors the
//! device's sensor manager's listener (`onNativeJoySensor()`).
//!
//! Where upstream keeps `joystick->hwdata` pointing at the device's list
//! item (and clears it when the device is removed), the item here records
//! the open joystick's ID, and the functions that act on an open joystick
//! find its item by it.

use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use super::gamepad::{GamepadButton, GamepadMapping};
use super::{
    create_joystick_guid, create_joystick_name, joystick_handled_by_another_driver,
    joysticks_initialized, lock_joysticks, private_joystick_added, private_joystick_removed,
    send_joystick_axis, send_joystick_button, send_joystick_sensor, should_ignore_joystick,
    JoystickData, JoystickDriver, HARDWARE_BUS_BLUETOOTH, PROP_JOYSTICK_CAP_RGB_LED_BOOLEAN,
    PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN, USB_PRODUCT_NINTENDO_SWITCH_JOYCON_LEFT, USB_VENDOR_NINTENDO,
};
use crate::core::android as jni;
use crate::error::{Error, Result};
use crate::events::keyboard::{send_keyboard_key, Scancode, GLOBAL_KEYBOARD_ID};
use crate::events::JoystickID;
use crate::guid::Guid;
use crate::hints;
use crate::sensor::SensorType;

// (android/keycodes.h; as of platform android-14, it was missing the
// AKEYCODE_BUTTON_1 to _16 defines, which upstream defines itself)
const AKEYCODE_MENU: i32 = 82;
const AKEYCODE_BACK: i32 = 4;
const AKEYCODE_DPAD_UP: i32 = 19;
const AKEYCODE_DPAD_DOWN: i32 = 20;
const AKEYCODE_DPAD_LEFT: i32 = 21;
const AKEYCODE_DPAD_RIGHT: i32 = 22;
const AKEYCODE_DPAD_CENTER: i32 = 23;
const AKEYCODE_MEDIA_RECORD: i32 = 130;
const AKEYCODE_BUTTON_A: i32 = 96;
const AKEYCODE_BUTTON_B: i32 = 97;
const AKEYCODE_BUTTON_C: i32 = 98;
const AKEYCODE_BUTTON_X: i32 = 99;
const AKEYCODE_BUTTON_Y: i32 = 100;
const AKEYCODE_BUTTON_Z: i32 = 101;
const AKEYCODE_BUTTON_L1: i32 = 102;
const AKEYCODE_BUTTON_R1: i32 = 103;
const AKEYCODE_BUTTON_L2: i32 = 104;
const AKEYCODE_BUTTON_R2: i32 = 105;
const AKEYCODE_BUTTON_THUMBL: i32 = 106;
const AKEYCODE_BUTTON_THUMBR: i32 = 107;
const AKEYCODE_BUTTON_START: i32 = 108;
const AKEYCODE_BUTTON_SELECT: i32 = 109;
const AKEYCODE_BUTTON_MODE: i32 = 110;
const AKEYCODE_BUTTON_1: i32 = 188;
const AKEYCODE_BUTTON_16: i32 = 203;

// (linux/input-event-codes.h)
const BTN_DPAD_UP: i32 = 0x220;
const BTN_DPAD_DOWN: i32 = 0x221;
const BTN_DPAD_LEFT: i32 = 0x222;
const BTN_DPAD_RIGHT: i32 = 0x223;

const ANDROID_MAX_NBUTTONS: i32 = 36;

/// A device the driver knows. Translation of `SDL_joylist_item` (which is
/// also the driver's `joystick_hwdata`).
#[derive(Debug)]
struct JoylistItem {
    device_instance: JoystickID,
    /// Android's device id
    device_id: i32,
    vendor_id: u16,
    #[allow(dead_code)] // (unused upstream too)
    product_id: u16,
    /// "SideWinder 3D Pro" or whatever
    name: String,
    guid: Guid,
    /// The open joystick (`joystick`, the `SDL_Joystick *`).
    joystick: Option<JoystickID>,
    nbuttons: usize,
    naxes: usize,
    nhats: usize,
    dpad_state: i32,
    can_rumble: bool,
    has_rgb_led: bool,
    has_accelerometer: bool,
    has_gyroscope: bool,
}

/// Translation of `SDL_joylist` (and `SDL_joylist_tail`; `numjoysticks` is
/// its length): the devices, in the order they were added.
static JOYLIST: Mutex<Vec<JoylistItem>> = Mutex::new(Vec::new());

fn joylist() -> MutexGuard<'static, Vec<JoylistItem>> {
    JOYLIST.lock().unwrap_or_else(|e| e.into_inner())
}

const BUTTON_SOUTH: i32 = GamepadButton::South as i32;
const BUTTON_EAST: i32 = GamepadButton::East as i32;
const BUTTON_WEST: i32 = GamepadButton::West as i32;
const BUTTON_NORTH: i32 = GamepadButton::North as i32;
const BUTTON_BACK: i32 = GamepadButton::Back as i32;
const BUTTON_GUIDE: i32 = GamepadButton::Guide as i32;
const BUTTON_START: i32 = GamepadButton::Start as i32;
const BUTTON_LEFT_STICK: i32 = GamepadButton::LeftStick as i32;
const BUTTON_RIGHT_STICK: i32 = GamepadButton::RightStick as i32;
const BUTTON_LEFT_SHOULDER: i32 = GamepadButton::LeftShoulder as i32;
const BUTTON_RIGHT_SHOULDER: i32 = GamepadButton::RightShoulder as i32;
const BUTTON_DPAD_UP: i32 = GamepadButton::DpadUp as i32;
const BUTTON_DPAD_DOWN: i32 = GamepadButton::DpadDown as i32;
const BUTTON_DPAD_LEFT: i32 = GamepadButton::DpadLeft as i32;
const BUTTON_DPAD_RIGHT: i32 = GamepadButton::DpadRight as i32;
const BUTTON_MISC1: i32 = GamepadButton::Misc1 as i32;

/// Function to convert Android keyCodes into SDL ones.
/// This code manipulation is done to get a sequential list of codes.
/// FIXME: This is only suited for the case where we use a fixed number of
/// buttons determined by ANDROID_MAX_NBUTTONS. Translation of
/// `keycode_to_SDL()`.
fn keycode_to_sdl(keycode: i32) -> i32 {
    // FIXME: If this function gets too unwieldy in the future, replace with a lookup table
    let button = match keycode {
        // Some gamepad buttons (API 9)
        AKEYCODE_BUTTON_A => BUTTON_SOUTH,
        AKEYCODE_BUTTON_B => BUTTON_EAST,
        AKEYCODE_BUTTON_X => BUTTON_WEST,
        AKEYCODE_BUTTON_Y => BUTTON_NORTH,
        AKEYCODE_BUTTON_L1 => BUTTON_LEFT_SHOULDER,
        AKEYCODE_BUTTON_R1 => BUTTON_RIGHT_SHOULDER,
        AKEYCODE_BUTTON_THUMBL => BUTTON_LEFT_STICK,
        AKEYCODE_BUTTON_THUMBR => BUTTON_RIGHT_STICK,
        AKEYCODE_MENU | AKEYCODE_BUTTON_START => BUTTON_START,
        AKEYCODE_BACK | AKEYCODE_BUTTON_SELECT => BUTTON_BACK,
        AKEYCODE_BUTTON_MODE => BUTTON_GUIDE,
        AKEYCODE_BUTTON_L2 | AKEYCODE_MEDIA_RECORD => BUTTON_MISC1,
        AKEYCODE_BUTTON_R2 => 16,
        AKEYCODE_BUTTON_C => 17,
        AKEYCODE_BUTTON_Z => 18,

        // D-Pad key codes (API 1)
        AKEYCODE_DPAD_UP => BUTTON_DPAD_UP,
        AKEYCODE_DPAD_DOWN => BUTTON_DPAD_DOWN,
        AKEYCODE_DPAD_LEFT => BUTTON_DPAD_LEFT,
        AKEYCODE_DPAD_RIGHT => BUTTON_DPAD_RIGHT,
        AKEYCODE_DPAD_CENTER => {
            // This is handled better by applications as the A button
            // button = 19;
            BUTTON_SOUTH
        }

        // More gamepad buttons (API 12), these get mapped to 20...35
        AKEYCODE_BUTTON_1..=AKEYCODE_BUTTON_16 => 20 + (keycode - AKEYCODE_BUTTON_1),

        _ => return -1,
    };

    // This is here in case future generations, probably with six fingers per hand,
    // happily add new cases up above and forget to update the max number of buttons.
    debug_assert!(button < ANDROID_MAX_NBUTTONS);
    button
}

/// Translation of `scancode_to_SDL()`.
fn scancode_to_sdl(scancode: i32) -> i32 {
    let button = match scancode {
        // D-Pad buttons on the left JoyCon
        BTN_DPAD_UP => BUTTON_DPAD_UP,
        BTN_DPAD_DOWN => BUTTON_DPAD_DOWN,
        BTN_DPAD_LEFT => BUTTON_DPAD_LEFT,
        BTN_DPAD_RIGHT => BUTTON_DPAD_RIGHT,

        _ => return -1,
    };
    debug_assert!(button < ANDROID_MAX_NBUTTONS);
    button
}

/// Translation of `button_to_scancode()`.
fn button_to_scancode(button: i32) -> Scancode {
    match button {
        BUTTON_SOUTH => Scancode::RETURN,
        BUTTON_EAST => Scancode::ESCAPE,
        BUTTON_BACK => Scancode::ESCAPE,
        BUTTON_START => Scancode::MENU,
        BUTTON_DPAD_UP => Scancode::UP,
        BUTTON_DPAD_DOWN => Scancode::DOWN,
        BUTTON_DPAD_LEFT => Scancode::LEFT,
        BUTTON_DPAD_RIGHT => Scancode::RIGHT,

        // Unsupported button
        _ => Scancode::UNKNOWN,
    }
}

/// The open joystick of the device `device_id`, if it's known and open
/// (`item && item->joystick`).
fn open_joystick_by_device_id(device_id: i32) -> Option<JoystickID> {
    joystick_by_device_id(&joylist(), device_id).and_then(|item| item.joystick)
}

/// A gamepad key of the device pressed or released: its button sent, or
/// a keyboard key when its joystick isn't open; whether it was a gamepad
/// key. Translation of `Android_OnPadDown()` and `Android_OnPadUp()`.
fn on_pad(device_id: i32, keycode: i32, scancode: i32, down: bool) -> bool {
    let timestamp = crate::timer::ticks_ns();
    let mut button = keycode_to_sdl(keycode);
    if button < 0 {
        button = scancode_to_sdl(scancode);
    }
    if button >= 0 {
        let _lock = lock_joysticks();
        if let Some(joystick) = open_joystick_by_device_id(device_id) {
            send_joystick_button(timestamp, joystick, button as u8, down);
        } else {
            send_keyboard_key(
                Duration::from_nanos(timestamp),
                GLOBAL_KEYBOARD_ID,
                keycode,
                button_to_scancode(button),
                down,
            );
        }
        return true;
    }

    false
}

/// Translation of `Android_OnPadDown()`.
pub(crate) fn on_pad_down(device_id: i32, keycode: i32, scancode: i32) -> bool {
    on_pad(device_id, keycode, scancode, true)
}

/// Translation of `Android_OnPadUp()`.
pub(crate) fn on_pad_up(device_id: i32, keycode: i32, scancode: i32) -> bool {
    on_pad(device_id, keycode, scancode, false)
}

/// An axis of the device moved. Translation of `Android_OnJoy()`.
pub(crate) fn on_joy(device_id: i32, axis: i32, value: f32) -> bool {
    let timestamp = crate::timer::ticks_ns();
    // Android gives joy info normalized as [-1.0, 1.0] or [0.0, 1.0]

    let _lock = lock_joysticks();
    if let Some(joystick) = open_joystick_by_device_id(device_id) {
        send_joystick_axis(
            timestamp,
            joystick,
            axis as u8,
            (32767.0 * value as f64) as i16,
        );
    }

    true
}

/// A hat of the device moved: its d-pad buttons that changed sent. Whether
/// the position is one (each of `x` and `y` -1, 0 or 1). Translation of
/// `Android_OnHat()`.
pub(crate) fn on_hat(device_id: i32, _hat_id: i32, x: i32, y: i32) -> bool {
    let timestamp = crate::timer::ticks_ns();
    const DPAD_UP_MASK: i32 = 1 << BUTTON_DPAD_UP;
    const DPAD_DOWN_MASK: i32 = 1 << BUTTON_DPAD_DOWN;
    const DPAD_LEFT_MASK: i32 = 1 << BUTTON_DPAD_LEFT;
    const DPAD_RIGHT_MASK: i32 = 1 << BUTTON_DPAD_RIGHT;

    if (-1..=1).contains(&x) && (-1..=1).contains(&y) {
        let _lock = lock_joysticks();
        let changed = {
            let mut list = joylist();
            match list
                .iter_mut()
                .find(|item| item.device_id == device_id)
                .and_then(|item| item.joystick.map(|joystick| (item, joystick)))
            {
                Some((item, joystick)) => {
                    let mut dpad_state = 0;
                    if x < 0 {
                        dpad_state |= DPAD_LEFT_MASK;
                    } else if x > 0 {
                        dpad_state |= DPAD_RIGHT_MASK;
                    }
                    if y < 0 {
                        dpad_state |= DPAD_UP_MASK;
                    } else if y > 0 {
                        dpad_state |= DPAD_DOWN_MASK;
                    }

                    let dpad_delta = dpad_state ^ item.dpad_state;
                    if dpad_delta != 0 {
                        item.dpad_state = dpad_state;
                    }
                    Some((joystick, dpad_state, dpad_delta))
                }
                None => None,
            }
        };
        if let Some((joystick, dpad_state, dpad_delta)) = changed {
            if dpad_delta != 0 {
                for (mask, button) in [
                    (DPAD_UP_MASK, BUTTON_DPAD_UP),
                    (DPAD_DOWN_MASK, BUTTON_DPAD_DOWN),
                    (DPAD_LEFT_MASK, BUTTON_DPAD_LEFT),
                    (DPAD_RIGHT_MASK, BUTTON_DPAD_RIGHT),
                ] {
                    if (dpad_delta & mask) != 0 {
                        let down = (dpad_state & mask) != 0;
                        send_joystick_button(timestamp, joystick, button as u8, down);
                    }
                }
            }
        }
        return true;
    }

    false
}

/// `Sensor.TYPE_ACCELEROMETER`.
const SENSOR_TYPE_ACCELEROMETER: i32 = 1;
/// `Sensor.TYPE_GYROSCOPE`.
const SENSOR_TYPE_GYROSCOPE: i32 = 4;

/// A sensor event of the device. Translation of `Android_OnJoySensor()`.
pub(crate) fn on_joy_sensor(
    device_id: i32,
    sensor_type: i32,
    sensor_timestamp: u64,
    x: f32,
    y: f32,
    z: f32,
) {
    let timestamp = crate::timer::ticks_ns();

    let sensor = if sensor_type == SENSOR_TYPE_ACCELEROMETER {
        SensorType::Accel
    } else if sensor_type == SENSOR_TYPE_GYROSCOPE {
        SensorType::Gyro
    } else {
        // Unsupported sensor
        return;
    };

    let _lock = lock_joysticks();
    let found = {
        let list = joylist();
        joystick_by_device_id(&list, device_id)
            .and_then(|item| item.joystick.map(|joystick| (joystick, item.vendor_id)))
    };
    if let Some((joystick, vendor_id)) = found {
        let mut data = [0.0f32; 3];

        if vendor_id == USB_VENDOR_NINTENDO {
            // The Nintendo driver uses a different axis order than SDL
            data[0] = -y;
            data[1] = z;
            data[2] = -x;

            if sensor == SensorType::Gyro {
                // The values are experimentally 3x what they should be
                data[0] /= 3.0;
                data[1] /= 3.0;
                data[2] /= 3.0;
            }
        } else {
            // The axes of sensor events and their signs are the same as SDL's, so no conversion required
            data[0] = x;
            data[1] = y;
            data[2] = z;
        }
        send_joystick_sensor(timestamp, joystick, sensor, sensor_timestamp, &data);
    }
}

/// The index of this driver in the joystick driver list (after HIDAPI's),
/// for `SDL_JoystickHandledByAnotherDriver()`.
const ANDROID_DRIVER_INDEX: usize = super::ANDROID_DRIVER_INDEX;

/// A device the joystick handler found. Translation of
/// `Android_AddJoystick()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn add_joystick(
    device_id: i32,
    name: Option<&str>,
    desc: &str,
    vendor_id: i32,
    product_id: i32,
    mut button_mask: i32,
    naxes: i32,
    axis_mask: i32,
    mut nhats: i32,
    can_rumble: bool,
    has_rgb_led: bool,
    has_accelerometer: bool,
    _has_gyroscope: bool,
) {
    // Java might notify us about joysticks being added before joysticks have
    // been initialized. That's fine, we'll get called again with the full set
    // in Android_JNI_DetectDevices()
    if !joysticks_initialized() {
        return;
    }

    let _lock = lock_joysticks();
    let vendor_id = vendor_id as u16;
    let product_id = product_id as u16;

    if !hints::get_bool(hints::TV_REMOTE_AS_JOYSTICK, true) {
        // Ignore devices that aren't actually controllers (e.g. remotes), they'll be handled as keyboard input
        if naxes < 2 && nhats < 1 {
            return;
        }
    }

    let Some(name) = name else {
        return;
    };
    if joystick_by_device_id(&joylist(), device_id).is_some() {
        return;
    }

    if joystick_handled_by_another_driver(
        ANDROID_DRIVER_INDEX,
        vendor_id,
        product_id,
        0,
        Some(name),
    ) {
        return;
    }

    if should_ignore_joystick(vendor_id, product_id, 0, Some(name)) {
        return;
    }

    if nhats > 0
        || (vendor_id == USB_VENDOR_NINTENDO
            && product_id == USB_PRODUCT_NINTENDO_SWITCH_JOYCON_LEFT)
    {
        // Hat is translated into DPAD buttons
        // D-Pad on the left JoyCon is a special case, it's not recognized via keycodes or hats, only scancodes
        button_mask |= (1 << BUTTON_DPAD_UP)
            | (1 << BUTTON_DPAD_DOWN)
            | (1 << BUTTON_DPAD_LEFT)
            | (1 << BUTTON_DPAD_RIGHT);
        nhats = 0;
    }

    let mut guid = create_joystick_guid(
        HARDWARE_BUS_BLUETOOTH,
        vendor_id,
        product_id,
        0,
        None,
        Some(desc),
        0,
        0,
    );

    // Update the GUID with capability bits
    guid.0[12..14].copy_from_slice(&(button_mask as u16).to_le_bytes());
    guid.0[14..16].copy_from_slice(&(axis_mask as u16).to_le_bytes());

    let Some(item_name) = create_joystick_name(vendor_id, product_id, None, Some(name)) else {
        return;
    };

    let nbuttons = if button_mask as u32 == 0xFFFF_FFFF {
        ANDROID_MAX_NBUTTONS as usize
    } else {
        let mut nbuttons = 0;
        for i in 0..32 {
            if (button_mask & (1 << i)) != 0 {
                nbuttons = i + 1;
            }
        }
        nbuttons
    };
    let device_instance = crate::utils::next_object_id();
    joylist().push(JoylistItem {
        device_instance,
        device_id,
        vendor_id,
        product_id,
        name: item_name,
        guid,
        joystick: None,
        nbuttons,
        naxes: naxes.max(0) as usize,
        nhats: nhats.max(0) as usize,
        dpad_state: 0,
        can_rumble,
        has_rgb_led,
        has_accelerometer,
        // Note (upstream): has_gyroscope is set from has_accelerometer.
        has_gyroscope: has_accelerometer,
    });

    // Need to increment the joystick count before we post the event

    private_joystick_added(device_instance);
}

/// A device the joystick handler lost. Translation of
/// `Android_RemoveJoystick()`.
pub(crate) fn remove_joystick(device_id: i32) {
    // Java might notify us about joysticks being removed before joysticks have
    // been initialized.
    if !joysticks_initialized() {
        return;
    }

    let _lock = lock_joysticks();

    // Don't call JoystickByDeviceId here or there'll be an infinite loop!
    let removed = {
        let mut list = joylist();
        list.iter()
            .position(|item| item.device_id == device_id)
            .map(|i| list.remove(i))
    };
    let Some(item) = removed else {
        return;
    };

    // (the open joystick's hwdata cleared: it was the removed item)

    // Need to decrement the joystick count before we post the event

    private_joystick_removed(item.device_instance);
}

/// Translation of `JoystickByDeviceId()`.
fn joystick_by_device_id(list: &[JoylistItem], device_id: i32) -> Option<&JoylistItem> {
    list.iter().find(|item| item.device_id == device_id)
}

/// The item of `device_index` (`GetJoystickByDevIndex()`), for `f`; else
/// `default`.
fn with_device<R>(device_index: usize, default: R, f: impl FnOnce(&mut JoylistItem) -> R) -> R {
    joylist().get_mut(device_index).map_or(default, f)
}

/// The item of the open joystick (its hwdata), for `f`; `None` once the
/// device is removed.
fn with_hwdata<R>(joystick: JoystickID, f: impl FnOnce(&mut JoylistItem) -> R) -> Option<R> {
    joylist()
        .iter_mut()
        .find(|item| item.joystick == Some(joystick))
        .map(f)
}

/// Translation of `SDL_ANDROID_JoystickDriver`.
pub(super) struct AndroidJoystickDriver;

pub(super) static ANDROID_JOYSTICK_DRIVER: AndroidJoystickDriver = AndroidJoystickDriver;

impl JoystickDriver for AndroidJoystickDriver {
    /// Translation of `ANDROID_JoystickInit()`.
    fn init(&self) -> Result<()> {
        jni::detect_devices();
        Ok(())
    }

    /// Translation of `ANDROID_JoystickGetCount()`.
    fn count(&self) -> usize {
        joylist().len()
    }

    /// Support for device connect/disconnect is implemented using
    /// InputDeviceListener. Ref:
    /// <http://developer.android.com/reference/android/hardware/input/InputManager.InputDeviceListener.html>.
    /// Translation of `ANDROID_JoystickDetect()`.
    fn detect(&self) {}

    /// Translation of `ANDROID_JoystickIsDevicePresent()`.
    fn is_device_present(
        &self,
        _vendor_id: u16,
        _product_id: u16,
        _version: u16,
        _name: Option<&str>,
    ) -> bool {
        // We don't override any other drivers
        false
    }

    /// Translation of `ANDROID_JoystickGetDeviceName()`.
    fn device_name(&self, device_index: usize) -> Option<String> {
        with_device(device_index, None, |item| Some(item.name.clone()))
    }

    /// Translation of `ANDROID_JoystickGetDevicePath()`.
    fn device_path(&self, _device_index: usize) -> Option<String> {
        None
    }

    /// Translation of `ANDROID_JoystickGetDeviceSteamVirtualGamepadSlot()`.
    fn device_steam_virtual_gamepad_slot(&self, _device_index: usize) -> i32 {
        -1
    }

    /// Translation of `ANDROID_JoystickGetDevicePlayerIndex()`.
    fn device_player_index(&self, _device_index: usize) -> i32 {
        -1
    }

    /// Translation of `ANDROID_JoystickSetDevicePlayerIndex()`.
    fn set_device_player_index(&self, _device_index: usize, _player_index: i32) {}

    /// Translation of `ANDROID_JoystickGetDeviceGUID()`.
    fn device_guid(&self, device_index: usize) -> Guid {
        with_device(device_index, Guid::ZERO, |item| item.guid)
    }

    /// Translation of `ANDROID_JoystickGetDeviceInstanceID()`.
    fn device_instance_id(&self, device_index: usize) -> JoystickID {
        with_device(device_index, 0, |item| item.device_instance)
    }

    /// Translation of `ANDROID_JoystickOpen()`.
    fn open(&self, joystick: &mut JoystickData, device_index: usize) -> Result<()> {
        let (can_rumble, has_rgb_led, has_accelerometer, has_gyroscope) =
            with_device(device_index, Err(Error::new("No such device")), |item| {
                if item.joystick.is_some() {
                    return Err(Error::new("Joystick already opened"));
                }

                item.joystick = Some(joystick.instance_id);
                joystick.nhats = item.nhats;
                joystick.nbuttons = item.nbuttons;
                joystick.naxes = item.naxes;
                Ok((
                    item.can_rumble,
                    item.has_rgb_led,
                    item.has_accelerometer,
                    item.has_gyroscope,
                ))
            })?;

        if can_rumble {
            let _ = joystick
                .properties()
                .set(PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN, true);
        }

        if has_rgb_led {
            let _ = joystick
                .properties()
                .set(PROP_JOYSTICK_CAP_RGB_LED_BOOLEAN, true);
        }

        if has_accelerometer {
            joystick.add_sensor(SensorType::Accel, 0.0);
        }
        if has_gyroscope {
            joystick.add_sensor(SensorType::Gyro, 0.0);
        }

        Ok(())
    }

    /// Translation of `ANDROID_JoystickRumble()` (with haptics, which
    /// SDL is built with).
    fn rumble(
        &self,
        joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        let (device_id, can_rumble) =
            with_hwdata(joystick, |item| (item.device_id, item.can_rumble))
                .ok_or_else(|| Error::new("Rumble failed, device disconnected"))?;
        if !can_rumble {
            return Err(Error::unsupported());
        }

        let low_frequency_intensity = low_frequency_rumble as f32 / u16::MAX as f32;
        let high_frequency_intensity = high_frequency_rumble as f32 / u16::MAX as f32;
        jni::haptic_rumble(
            device_id,
            low_frequency_intensity,
            high_frequency_intensity,
            5000,
        );
        Ok(())
    }

    /// Translation of `ANDROID_JoystickRumbleTriggers()`.
    fn rumble_triggers(
        &self,
        _joystick: JoystickID,
        _left_rumble: u16,
        _right_rumble: u16,
    ) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `ANDROID_JoystickSetLED()`.
    fn set_led(&self, joystick: JoystickID, red: u8, green: u8, blue: u8) -> Result<()> {
        let (device_id, has_rgb_led) =
            with_hwdata(joystick, |item| (item.device_id, item.has_rgb_led))
                .ok_or_else(|| Error::new("SetLED failed, device disconnected"))?;
        if !has_rgb_led {
            return Err(Error::unsupported());
        }
        jni::joystick_set_led(device_id, red as i32, green as i32, blue as i32);
        Ok(())
    }

    /// Translation of `ANDROID_JoystickSendEffect()`.
    fn send_effect(&self, _joystick: JoystickID, _data: &[u8]) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `ANDROID_JoystickSetSensorsEnabled()`.
    fn set_sensors_enabled(&self, joystick: JoystickID, enabled: bool) -> Result<()> {
        let (device_id, has_sensors) = with_hwdata(joystick, |item| {
            (item.device_id, item.has_accelerometer || item.has_gyroscope)
        })
        .ok_or_else(|| Error::new("SetSensorsEnabled failed, device disconnected"))?;
        if !has_sensors {
            return Err(Error::unsupported());
        }
        jni::joystick_set_sensors_enabled(device_id, enabled);
        Ok(())
    }

    /// Translation of `ANDROID_JoystickUpdate()`.
    fn update(&self, _joystick: JoystickID) {}

    /// Translation of `ANDROID_JoystickClose()`.
    fn close(&self, joystick: &mut JoystickData) {
        with_hwdata(joystick.instance_id, |item| item.joystick = None);
    }

    /// Translation of `ANDROID_JoystickQuit()`.
    fn quit(&self) {
        joylist().clear();
    }

    /// Translation of `ANDROID_JoystickGetGamepadMapping()`.
    fn gamepad_mapping(&self, _device_index: usize) -> Option<GamepadMapping> {
        None
    }
}

#[cfg(test)]
mod tests;
