// Rust translation of src/joystick/emscripten/SDL_sysjoystick.c and
// src/joystick/emscripten/SDL_sysjoystick_c.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Emscripten joystick driver: the browser's Gamepad API.
//!
//! The gamepads already connected when the driver initializes (in
//! `navigator.getGamepads()`, through `emscripten_sample_gamepad_data()`)
//! are added then; afterwards the window's `gamepadconnected` and
//! `gamepaddisconnected` events (`emscripten_set_gamepadconnected_callback()`
//! and `…disconnected…`) add and remove them, from the browser's event loop.
//! Each update of an open joystick samples the gamepads again and sends
//! what changed since its last timestamp.
//!
//! A gamepad with the browser's `"standard"` mapping and 16 buttons or more
//! is laid out as a game console's: its four d-pad buttons (12 to 15)
//! become a hat, and its two trigger buttons (6 and 7) are dropped from the
//! buttons; where the browser gives the triggers as analog buttons (four
//! axes, as Chrome does) they become two more axes, where it gives them as
//! axes (six, as Firefox does) they stay those. The other gamepads keep the
//! browser's buttons and axes, except that on Windows the axis 9 of one
//! with ten axes or more is taken for a hat (the browsers turn an unmapped
//! controller's hat into it). The `"default"` mapping of the gamepad
//! database, for Emscripten only, maps the standard layout, so every
//! gamepad is an [`SDL_Gamepad`](super::gamepad::Gamepad) with it unless
//! the database has one of its own.
//!
//! The names, GUIDs and USB IDs come from the gamepad's `id`, in the
//! browsers' formats (Chrome's `"Name (STANDARD GAMEPAD Vendor: 054c
//! Product: 09cc)"`, Firefox's and Safari's `"054c-09cc-Name"`), the bus
//! and the GUID's last byte from the operating system `navigator.userAgent`
//! names. Rumble is the `vibrationActuator`'s `'dual-rumble'` effect, also
//! for the triggers where its `effects` include `'trigger-rumble'`. There
//! are no paths, serials or player indices.
//!
//! The JavaScript is upstream's, in the `sdl3-emscripten` crate
//! (`sdl3_emscripten::joystick`). Where upstream keeps `joystick->hwdata`
//! pointing at the gamepad's list item (and clears it when the gamepad is
//! disconnected), the item here records the open joystick's ID, and the
//! functions that act on an open joystick find its item by it.
//!
//! Without the Gamepad API (under node: no `navigator.getGamepads`) the
//! driver is unavailable, as upstream's is ("Gamepads not supported"), and
//! the joystick subsystem has the virtual driver alone.
//!
//! Note (upstream), what the driver does, kept as it is:
//!
//! * the buttons and axes a gamepad has when it connects are its state
//!   then, not sent: a button held or a stick pushed while it connects
//!   reads released or centered until it changes (and so the hat);
//! * a standard gamepad's two trigger axes are kept as the axis values
//!   they're sent as (-1 to 1) when it connects, but as the trigger
//!   buttons' values (0 to 1) from the first update on: so the first update
//!   (the one when the joystick opens) sends them although they didn't
//!   change (-32767 when released), and a trigger fully pressed then isn't
//!   sent until it changes;
//! * the hat of a standard gamepad is made of all four d-pad buttons when
//!   it connects, but up wins over down and left over right in updates;
//! * the update walks the browser's buttons past a standard gamepad's last
//!   one (the four hat buttons and two triggers it skipped), and sends the
//!   buttons past the joystick's, which the joystick front end drops.
//!   Upstream reads those from the rest of emscripten's event structure
//!   (stale, or past its 64 buttons); here they are released.

use std::ffi::{c_char, c_int, c_void, CStr};
use std::sync::{Mutex, MutexGuard};

use sdl3_emscripten::joystick as js;
use sdl3_emscripten::sys::{self, EmscriptenGamepadEvent};

use super::gamepad::GamepadMapping;
use super::{
    assert_joysticks_locked, create_joystick_guid, create_joystick_guid_for_name,
    create_joystick_name, lock_joysticks, private_joystick_added, private_joystick_removed,
    send_joystick_axis, send_joystick_button, send_joystick_hat, JoystickData, JoystickDriver,
    HARDWARE_BUS_BLUETOOTH, HARDWARE_BUS_UNKNOWN, HARDWARE_BUS_USB, HAT_CENTERED, HAT_DOWN,
    HAT_LEFT, HAT_RIGHT, HAT_UP, PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN,
    PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN, USB_PRODUCT_XBOX360_XUSB_CONTROLLER,
    USB_PRODUCT_XBOX_ONE_S, USB_PRODUCT_XBOX_ONE_S_REV1_BLUETOOTH, USB_VENDOR_MICROSOFT,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::guid::Guid;

/// The number of axes and buttons emscripten's event structure holds (and
/// upstream's list item: `!!! FIXME: don't hardcode 64 on all of these.`).
const MAX_INPUTS: usize = 64;

/// A gamepad the driver knows. Translation of `SDL_joylist_item` (which is
/// also the driver's `joystick_hwdata`). Upstream also keeps a copy of the
/// browser's mapping string, which nothing reads.
#[derive(Clone, Debug)]
struct JoylistItem {
    /// The gamepad's index in `navigator.getGamepads()`.
    index: i32,
    name: String,
    device_instance: JoystickID,
    /// The open joystick (`joystick`, the `SDL_Joystick *`).
    joystick: Option<JoystickID>,
    first_hat_button: i32,
    first_trigger_button: i32,
    triggers_are_buttons: bool,
    hat_is_axis: bool,
    nhats: usize,
    guid: Guid,
    nbuttons: usize,
    naxes: usize,
    timestamp: f64,
    axis: [f64; MAX_INPUTS],
    analog_button: [f64; MAX_INPUTS],
    digital_button: [bool; MAX_INPUTS],
    /// There is (currently) only ever one of these, faked from the d-pad
    /// buttons or the axis 9.
    hat: u8,
    rumble_available: bool,
    trigger_rumble_available: bool,
    weak_magnitude_rumble: u16,
    strong_magnitude_rumble: u16,
    left_trigger_rumble: u16,
    right_trigger_rumble: u16,
}

/// Translation of `SDL_joylist` (and `SDL_joylist_tail`; `numjoysticks` is
/// its length): the gamepads, in the order they connected.
static JOYLIST: Mutex<Vec<JoylistItem>> = Mutex::new(Vec::new());

fn joylist() -> MutexGuard<'static, Vec<JoylistItem>> {
    JOYLIST.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `SDL_GetEmscriptenJoystickVendor()`.
fn emscripten_joystick_vendor(device_index: i32) -> i32 {
    // SAFETY: plain JavaScript.
    unsafe { js::sdl3_joystick_vendor(device_index) }
}

/// Translation of `SDL_GetEmscriptenJoystickProduct()`.
fn emscripten_joystick_product(device_index: i32) -> i32 {
    // SAFETY: plain JavaScript.
    unsafe { js::sdl3_joystick_product(device_index) }
}

/// Translation of `SDL_IsEmscriptenJoystickXInput()`.
fn is_emscripten_joystick_xinput(device_index: i32) -> bool {
    // SAFETY: plain JavaScript.
    unsafe { js::sdl3_joystick_is_xinput(device_index) != 0 }
}

/// Translation of `SDL_GetEmscriptenOSID()`: 1 Android, 2 Linux, 3 iPhone,
/// 4 Macintosh, 5 Windows, 0 none of them.
fn emscripten_os_id() -> i32 {
    // SAFETY: plain JavaScript.
    unsafe { js::sdl3_joystick_os_id() }
}

/// Translation of `SDL_GetEmscriptenNormalizedName()` (into upstream's
/// 128-byte buffer).
fn emscripten_normalized_name(device_index: i32) -> String {
    let mut name = [0 as c_char; 128];
    // SAFETY: the JavaScript writes at most the buffer's length, with the
    // NUL.
    unsafe {
        js::sdl3_joystick_normalized_name(device_index, name.as_mut_ptr(), name.len() as i32)
    };
    // SAFETY: NUL-terminated by stringToUTF8() (the buffer isn't empty).
    unsafe { CStr::from_ptr(name.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

/// `IS_EQUAL_APPROX(a, b)`.
fn is_equal_approx(a: f64, b: f64) -> bool {
    (a - b) < 0.00001 && (a - b) > -0.00001
}

/// The hat position of an axis that is a hat (Windows browsers' axis 9 of
/// an unmapped controller): eight positions from -1 to 1 in steps of 2/7,
/// up first and clockwise. Translation of `SDL_GetHatFromEmscriptenAxis()`.
#[allow(clippy::eq_op)] // (upstream's -7.0/7 and 7.0/7, as the other positions)
fn hat_from_emscripten_axis(hat_axis: f32) -> u8 {
    let hat_axis = f64::from(hat_axis);
    if is_equal_approx(hat_axis, -7.0 / 7.0) {
        HAT_UP
    } else if is_equal_approx(hat_axis, -5.0 / 7.0) {
        HAT_UP | HAT_RIGHT
    } else if is_equal_approx(hat_axis, -3.0 / 7.0) {
        HAT_RIGHT
    } else if is_equal_approx(hat_axis, -1.0 / 7.0) {
        HAT_DOWN | HAT_RIGHT
    } else if is_equal_approx(hat_axis, 1.0 / 7.0) {
        HAT_DOWN
    } else if is_equal_approx(hat_axis, 3.0 / 7.0) {
        HAT_DOWN | HAT_LEFT
    } else if is_equal_approx(hat_axis, 5.0 / 7.0) {
        HAT_LEFT
    } else if is_equal_approx(hat_axis, 7.0 / 7.0) {
        HAT_UP | HAT_LEFT
    } else {
        HAT_CENTERED
    }
}

/// The C string of one of the event's fixed-size strings.
fn event_string(s: &[c_char]) -> &[u8] {
    // SAFETY: c_char and u8 have the same layout.
    let bytes = unsafe { std::slice::from_raw_parts(s.as_ptr().cast::<u8>(), s.len()) };
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    &bytes[..len]
}

/// How the driver lays a gamepad out, from what it has when it connects
/// (the part of `Emscripten_JoyStickConnected()` that decides it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    first_hat_button: i32,
    first_trigger_button: i32,
    triggers_are_buttons: bool,
    hat_is_axis: bool,
    nhats: usize,
    naxes: usize,
    nbuttons: usize,
}

impl Layout {
    /// A gamepad with the browser's `"standard"` mapping (`standard`) or
    /// another, `num_buttons` buttons and `num_axes` axes, on the operating
    /// system [`emscripten_os_id()`] says.
    fn new(standard: bool, num_buttons: i32, num_axes: i32, os_id: i32) -> Layout {
        let mut first_trigger_button = -1;
        let mut first_hat_button = -1;
        let mut num_buttons = num_buttons;
        let mut num_axes = num_axes;
        let mut triggers_are_buttons = false;
        let mut hat_is_axis = false;
        if standard && num_buttons >= 16 {
            // maps to a game console gamepad layout, turn the d-pad into a hat, treat triggers as analog.
            num_buttons -= 4; // 4 dpad buttons become a hat.
            first_hat_button = 12;

            if num_axes == 4 {
                // Chrome gives the triggers analog button values, Firefox exposes them as extra axes. Both have the digital buttons.
                num_axes += 2; // the two trigger "buttons"
                triggers_are_buttons = true;
            }

            // dump the digital trigger buttons in any case.
            first_trigger_button = 6;
            num_buttons -= 2;
        } else if !standard && num_axes >= 10 && os_id == 5 {
            // The browsers (Chromium and Firefox) on Windows usually convert the hat on unmapped controllers to axis 9
            // We can't know for sure that this axis is a hat, but most of the time it is
            hat_is_axis = true;
        }

        Layout {
            first_hat_button,
            first_trigger_button,
            triggers_are_buttons,
            hat_is_axis,
            nhats: if first_hat_button >= 0 || hat_is_axis {
                1
            } else {
                0
            },
            naxes: num_axes.max(0) as usize,
            nbuttons: num_buttons.max(0) as usize,
        }
    }

    /// The browser's button each of the joystick's buttons is, for the
    /// `real_button_count` buttons the browser has: `(i, buttonidx)` as
    /// upstream's loops walk them, past the hat's and the triggers'.
    fn buttons(&self, real_button_count: i32) -> impl Iterator<Item = (usize, usize)> {
        let (first_hat_button, first_trigger_button) =
            (self.first_hat_button, self.first_trigger_button);
        let count = real_button_count.clamp(0, MAX_INPUTS as i32);
        let mut buttonidx = 0i32;
        (0..count).map(move |i| {
            if buttonidx == first_hat_button {
                buttonidx += 4; // skip these buttons, we're treating them as hat input.
            } else if buttonidx == first_trigger_button {
                buttonidx += 2; // skip these buttons, we're treating them as axes.
            }
            let this = buttonidx;
            buttonidx += 1;
            (i as usize, this as usize)
        })
    }
}

impl JoylistItem {
    fn layout(&self) -> Layout {
        Layout {
            first_hat_button: self.first_hat_button,
            first_trigger_button: self.first_trigger_button,
            triggers_are_buttons: self.triggers_are_buttons,
            hat_is_axis: self.hat_is_axis,
            nhats: self.nhats,
            naxes: self.naxes,
            nbuttons: self.nbuttons,
        }
    }
}

/// The bus, USB IDs and the GUID's driver data (the subtype) of a gamepad
/// (the part of `Emscripten_JoyStickConnected()` that finds them): from
/// the IDs in its `id`, whether that says XInput, the operating system,
/// whether it can rumble its triggers, and whether the browser maps it
/// (`standard`).
fn identify(
    vendor: u16,
    product: u16,
    is_xinput: bool,
    os_id: i32,
    trigger_rumble_available: bool,
    standard: bool,
) -> (u16, u16, u16, u8) {
    let mut bus = HARDWARE_BUS_UNKNOWN;
    let mut vendor = vendor;
    let mut product = product;
    let mut subtype = os_id as u8;

    if os_id != 0 {
        if os_id == 1 || os_id == 3 {
            // Android or iOS (mobile)
            bus = HARDWARE_BUS_BLUETOOTH;
        } else {
            // Desktop
            bus = HARDWARE_BUS_USB;
        }
    }

    if vendor == 0 && product == 0 && is_xinput {
        // Use a generic VID/PID representing an XInput controller
        vendor = USB_VENDOR_MICROSOFT;
        product = USB_PRODUCT_XBOX360_XUSB_CONTROLLER;

        if trigger_rumble_available {
            // Assume Xbox One S Controller
            if bus == HARDWARE_BUS_BLUETOOTH {
                product = USB_PRODUCT_XBOX_ONE_S_REV1_BLUETOOTH;
            } else {
                product = USB_PRODUCT_XBOX_ONE_S;
            }
        }
    }

    if standard {
        // We should differentiate between devices that are mapped or unmapped by the browser.
        subtype = subtype.wrapping_add(0x80);
    }

    (bus, vendor, product, subtype)
}

/// The GUID of a gamepad: from its bus and USB IDs and its normalized
/// name where it has both IDs, else from its SDL name, with the subtype as
/// the last byte.
fn emscripten_guid(
    bus: u16,
    vendor: u16,
    product: u16,
    subtype: u8,
    name: &str,
    item_name: &str,
) -> Guid {
    if vendor != 0 && product != 0 {
        create_joystick_guid(bus, vendor, product, 0, None, Some(name), 0, subtype)
    } else {
        let mut guid = create_joystick_guid_for_name(item_name);
        guid.0[15] = subtype;
        guid
    }
}

/// `(Sint16)(32767.0f * value)`.
fn axis_value(value: f64) -> i16 {
    (32767.0f32 as f64 * value) as i16
}

/// A gamepad's event structure, zeroed for emscripten to fill in.
fn zeroed_gamepad_state() -> EmscriptenGamepadEvent {
    // SAFETY: the structure is numbers, booleans and C strings, for which
    // zero bytes are valid values.
    unsafe { std::mem::zeroed() }
}

/// Translation of `Emscripten_JoyStickConnected()`: emscripten's
/// `gamepadconnected` handler calls it with the event it filled in, and
/// the driver's initialization with each gamepad already connected.
unsafe extern "C-unwind" fn joystick_connected(
    _event_type: c_int,
    gamepad_event: *const EmscriptenGamepadEvent,
    _user_data: *mut c_void,
) -> bool {
    // SAFETY: emscripten passes its event structure, filled in.
    let gamepad_event = unsafe { &*gamepad_event };
    let _lock = lock_joysticks();

    let added = 'done: {
        if joystick_by_index(&joylist(), gamepad_event.index).is_some() {
            break 'done None;
        }

        let index = gamepad_event.index;
        let standard = event_string(&gamepad_event.mapping) == b"standard";

        let vendor = emscripten_joystick_vendor(index) as u16;
        let product = emscripten_joystick_product(index) as u16;
        let is_xinput = is_emscripten_joystick_xinput(index);

        let os_id = emscripten_os_id();

        // SAFETY: plain JavaScript.
        let trigger_rumble_available =
            unsafe { js::sdl3_joystick_trigger_rumble_available(index) } != 0;

        let (bus, vendor, product, subtype) = identify(
            vendor,
            product,
            is_xinput,
            os_id,
            trigger_rumble_available,
            standard,
        );

        let name = emscripten_normalized_name(index);
        let Some(item_name) = create_joystick_name(vendor, product, None, Some(&name)) else {
            break 'done None;
        };

        let guid = emscripten_guid(bus, vendor, product, subtype, &name, &item_name);

        let real_button_count = gamepad_event.numButtons;
        let real_axis_count = gamepad_event.numAxes;
        let layout = Layout::new(standard, real_button_count, real_axis_count, os_id);

        let mut item = JoylistItem {
            index,
            name: item_name,
            device_instance: crate::utils::next_object_id(),
            joystick: None,
            first_hat_button: layout.first_hat_button,
            first_trigger_button: layout.first_trigger_button,
            triggers_are_buttons: layout.triggers_are_buttons,
            hat_is_axis: layout.hat_is_axis,
            nhats: layout.nhats,
            guid,
            nbuttons: layout.nbuttons,
            naxes: layout.naxes,
            timestamp: gamepad_event.timestamp,
            axis: [0.0; MAX_INPUTS],
            analog_button: [0.0; MAX_INPUTS],
            digital_button: [false; MAX_INPUTS],
            hat: HAT_CENTERED,
            rumble_available: false,
            trigger_rumble_available,
            weak_magnitude_rumble: 0,
            strong_magnitude_rumble: 0,
            left_trigger_rumble: 0,
            right_trigger_rumble: 0,
        };

        for (i, buttonidx) in layout.buttons(real_button_count) {
            item.analog_button[i] = gamepad_event
                .analogButton
                .get(buttonidx)
                .copied()
                .unwrap_or(0.0);
            item.digital_button[i] = gamepad_event
                .digitalButton
                .get(buttonidx)
                .copied()
                .unwrap_or(false);
        }

        let real_axis_count = real_axis_count.clamp(0, MAX_INPUTS as i32) as usize;
        item.axis[..real_axis_count].copy_from_slice(&gamepad_event.axis[..real_axis_count]);

        if item.triggers_are_buttons {
            let first_trigger_button = item.first_trigger_button as usize;
            item.axis[real_axis_count] =
                (gamepad_event.analogButton[first_trigger_button] * 2.0) - 1.0;
            item.axis[real_axis_count + 1] =
                (gamepad_event.analogButton[first_trigger_button + 1] * 2.0) - 1.0;
        }

        debug_assert!(item.nhats <= 1); // there is (currently) only ever one of these, faked from the d-pad buttons or the axis 9.
        if item.first_hat_button != -1 {
            let first_hat_button = item.first_hat_button as usize;
            let mut value = HAT_CENTERED;
            // this currently expects the first button to be up, then down, then left, then right.
            if gamepad_event.digitalButton[first_hat_button] {
                value |= HAT_UP;
            }
            if gamepad_event.digitalButton[first_hat_button + 1] {
                value |= HAT_DOWN;
            }
            if gamepad_event.digitalButton[first_hat_button + 2] {
                value |= HAT_LEFT;
            }
            if gamepad_event.digitalButton[first_hat_button + 3] {
                value |= HAT_RIGHT;
            }
            item.hat = value;
        } else if item.hat_is_axis {
            item.hat = hat_from_emscripten_axis(gamepad_event.axis[9] as f32);
            item.axis[9] = 0.0;
        }

        let device_instance = item.device_instance;
        joylist().push(item);

        break 'done Some(device_instance);
    };

    if let Some(device_instance) = added {
        private_joystick_added(device_instance);
    }

    true
}

/// Translation of `Emscripten_JoyStickDisconnected()`: emscripten's
/// `gamepaddisconnected` handler calls it with the event it filled in.
unsafe extern "C-unwind" fn joystick_disconnected(
    _event_type: c_int,
    gamepad_event: *const EmscriptenGamepadEvent,
    _user_data: *mut c_void,
) -> bool {
    // SAFETY: emscripten passes its event structure, filled in.
    let gamepad_event = unsafe { &*gamepad_event };
    let _lock = lock_joysticks();

    let removed = {
        let mut list = joylist();
        list.iter()
            .position(|item| item.index == gamepad_event.index)
            // (the open joystick's hwdata goes with the item, and the count
            // is decremented before the event is posted)
            .map(|position| list.remove(position).device_instance)
    };

    if let Some(device_instance) = removed {
        private_joystick_removed(device_instance);
    }

    true
}

/// Translation of `JoystickByIndex()`: the item of the gamepad at `index`
/// in `navigator.getGamepads()`.
fn joystick_by_index(list: &[JoylistItem], index: i32) -> Option<&JoylistItem> {
    if index < 0 {
        return None;
    }

    list.iter().find(|item| item.index == index)
}

/// Run `f` on the item at `device_index` (translation of
/// `JoystickByDeviceIndex()`), or answer `default`.
fn with_device<R>(device_index: usize, default: R, f: impl FnOnce(&mut JoylistItem) -> R) -> R {
    joylist().get_mut(device_index).map_or(default, f)
}

/// Run `f` on the item of the open joystick `joystick` (upstream's
/// `joystick->hwdata`), if it's still connected.
fn with_hwdata<R>(joystick: JoystickID, f: impl FnOnce(&mut JoylistItem) -> R) -> Option<R> {
    joylist()
        .iter_mut()
        .find(|item| item.joystick == Some(joystick))
        .map(f)
}

/// Translation of `Emscripten_UpdateRumble()`.
fn update_rumble(item: &JoylistItem) -> Result<()> {
    // SAFETY: plain JavaScript.
    let result = unsafe {
        js::sdl3_joystick_update_rumble(
            item.index,
            i32::from(item.weak_magnitude_rumble),
            i32::from(item.strong_magnitude_rumble),
            i32::from(item.left_trigger_rumble),
            i32::from(item.right_trigger_rumble),
        )
    };
    if result != 0 {
        Ok(())
    } else {
        // (upstream sets no error: the gamepad isn't in navigator.getGamepads())
        Err(Error::new("Couldn't rumble the gamepad"))
    }
}

/// What an update sends, collected while the list is locked and sent after
/// (the events may call back into the driver).
enum Change {
    Button(u8, bool),
    Axis(u8, i16),
    Hat(u8),
}

/// Translation of `SDL_EMSCRIPTEN_JoystickDriver`.
pub(super) struct EmscriptenJoystickDriver;

pub(super) static EMSCRIPTEN_JOYSTICK_DRIVER: EmscriptenJoystickDriver = EmscriptenJoystickDriver;

impl JoystickDriver for EmscriptenJoystickDriver {
    /// Translation of `EMSCRIPTEN_JoystickInit()`: scans the system for
    /// joysticks.
    fn init(&self) -> Result<()> {
        joylist().clear();

        // SAFETY: a plain call.
        let rc = unsafe { sys::emscripten_sample_gamepad_data() };

        // Check if gamepad is supported by browser
        if rc == sys::EMSCRIPTEN_RESULT_NOT_SUPPORTED {
            return Err(Error::new("Gamepads not supported"));
        }

        // SAFETY: a plain call, after a successful sample.
        let numjs = unsafe { sys::emscripten_get_num_gamepads() };

        // handle already connected gamepads
        for i in 0..numjs.max(0) {
            let mut gamepad_state = zeroed_gamepad_state();
            // SAFETY: emscripten fills in the structure.
            let rc = unsafe { sys::emscripten_get_gamepad_status(i, &mut gamepad_state) };
            if rc == sys::EMSCRIPTEN_RESULT_SUCCESS {
                // SAFETY: the callback's own parameters.
                unsafe {
                    joystick_connected(
                        sys::EMSCRIPTEN_EVENT_GAMEPADCONNECTED,
                        &gamepad_state,
                        std::ptr::null_mut(),
                    )
                };
            }
        }

        // SAFETY: an extern "C-unwind" fn with the parameters emscripten calls
        // it with, for the life of the program; the user data isn't used.
        let rc = unsafe {
            sys::emscripten_set_gamepadconnected_callback_on_thread(
                std::ptr::null_mut(),
                false,
                Some(joystick_connected),
                sys::EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
            )
        };

        if rc != sys::EMSCRIPTEN_RESULT_SUCCESS {
            self.quit();
            return Err(Error::new("Could not set gamepad connect callback"));
        }

        // SAFETY: as above.
        let rc = unsafe {
            sys::emscripten_set_gamepaddisconnected_callback_on_thread(
                std::ptr::null_mut(),
                false,
                Some(joystick_disconnected),
                sys::EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
            )
        };
        if rc != sys::EMSCRIPTEN_RESULT_SUCCESS {
            self.quit();
            return Err(Error::new("Could not set gamepad disconnect callback"));
        }

        Ok(())
    }

    /// Translation of `EMSCRIPTEN_JoystickGetCount()`.
    fn count(&self) -> usize {
        joylist().len()
    }

    /// Translation of `EMSCRIPTEN_JoystickDetect()`.
    fn detect(&self) {}

    /// Translation of `EMSCRIPTEN_JoystickIsDevicePresent()`.
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

    /// Translation of `EMSCRIPTEN_JoystickGetDeviceName()`.
    fn device_name(&self, device_index: usize) -> Option<String> {
        with_device(device_index, None, |item| Some(item.name.clone()))
    }

    /// Translation of `EMSCRIPTEN_JoystickGetDevicePath()`.
    fn device_path(&self, _device_index: usize) -> Option<String> {
        None
    }

    /// Translation of `EMSCRIPTEN_JoystickGetDeviceSteamVirtualGamepadSlot()`.
    fn device_steam_virtual_gamepad_slot(&self, _device_index: usize) -> i32 {
        -1
    }

    /// Translation of `EMSCRIPTEN_JoystickGetDevicePlayerIndex()`.
    fn device_player_index(&self, _device_index: usize) -> i32 {
        -1
    }

    /// Translation of `EMSCRIPTEN_JoystickSetDevicePlayerIndex()`.
    fn set_device_player_index(&self, _device_index: usize, _player_index: i32) {}

    /// Translation of `EMSCRIPTEN_JoystickGetDeviceGUID()`.
    fn device_guid(&self, device_index: usize) -> Guid {
        with_device(device_index, Guid::ZERO, |item| item.guid)
    }

    /// Translation of `EMSCRIPTEN_JoystickGetDeviceInstanceID()`.
    fn device_instance_id(&self, device_index: usize) -> JoystickID {
        with_device(device_index, 0, |item| item.device_instance)
    }

    /// Translation of `EMSCRIPTEN_JoystickOpen()`.
    fn open(&self, joystick: &mut JoystickData, device_index: usize) -> Result<()> {
        let index = with_device(device_index, Err(Error::new("No such device")), |item| {
            if item.joystick.is_some() {
                return Err(Error::new("Joystick already opened"));
            }

            item.joystick = Some(joystick.instance_id);

            // HTML5 Gamepad API doesn't offer hats, but we can fake it from the d-pad buttons on the "standard" mapping.
            joystick.nhats = item.nhats;
            joystick.nbuttons = item.nbuttons;
            joystick.naxes = item.naxes;
            Ok(item.index)
        })?;

        // SAFETY: plain JavaScript.
        let rumble_available = unsafe { js::sdl3_joystick_rumble_available(index) } != 0;
        let trigger_rumble_available = with_device(device_index, false, |item| {
            item.rumble_available = rumble_available;
            item.trigger_rumble_available
        });

        if rumble_available {
            let _ = joystick
                .properties()
                .set(PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN, true);
        }

        // item->trigger_rumble_available is set in Emscripten_JoyStickConnected
        if trigger_rumble_available {
            let _ = joystick
                .properties()
                .set(PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN, true);
        }

        Ok(())
    }

    /// Translation of `EMSCRIPTEN_JoystickRumble()`.
    fn rumble(
        &self,
        joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        let item = with_hwdata(joystick, |item| {
            item.rumble_available.then(|| {
                item.strong_magnitude_rumble = low_frequency_rumble;
                item.weak_magnitude_rumble = high_frequency_rumble;
                item.clone()
            })
        })
        .flatten()
        .ok_or_else(Error::unsupported)?;
        update_rumble(&item)
    }

    /// Translation of `EMSCRIPTEN_JoystickRumbleTriggers()`.
    fn rumble_triggers(
        &self,
        joystick: JoystickID,
        left_rumble: u16,
        right_rumble: u16,
    ) -> Result<()> {
        let item = with_hwdata(joystick, |item| {
            item.trigger_rumble_available.then(|| {
                item.left_trigger_rumble = left_rumble;
                item.right_trigger_rumble = right_rumble;
                item.clone()
            })
        })
        .flatten()
        .ok_or_else(Error::unsupported)?;
        update_rumble(&item)
    }

    /// Translation of `EMSCRIPTEN_JoystickSetLED()`.
    fn set_led(&self, _joystick: JoystickID, _red: u8, _green: u8, _blue: u8) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `EMSCRIPTEN_JoystickSendEffect()`.
    fn send_effect(&self, _joystick: JoystickID, _data: &[u8]) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `EMSCRIPTEN_JoystickSetSensorsEnabled()`.
    fn set_sensors_enabled(&self, _joystick: JoystickID, _enabled: bool) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `EMSCRIPTEN_JoystickUpdate()`: the state of a
    /// joystick, as a device poll.
    fn update(&self, joystick: JoystickID) {
        assert_joysticks_locked();

        let timestamp = crate::timer::ticks_ns();

        // SAFETY: a plain call.
        unsafe { sys::emscripten_sample_gamepad_data() };

        let Some(index) = with_hwdata(joystick, |item| item.index) else {
            return;
        };
        let mut gamepad_state = zeroed_gamepad_state();
        // SAFETY: emscripten fills in the structure.
        let result = unsafe { sys::emscripten_get_gamepad_status(index, &mut gamepad_state) };
        if result != sys::EMSCRIPTEN_RESULT_SUCCESS {
            return;
        }

        let changes = with_hwdata(joystick, |item| {
            let mut changes = Vec::new();
            if gamepad_state.timestamp != 0.0 && gamepad_state.timestamp == item.timestamp {
                return changes;
            }
            let layout = item.layout();
            let first_hat_button = item.first_hat_button;
            let first_trigger_button = item.first_trigger_button;
            let real_button_count = gamepad_state.numButtons;
            let real_axis_count = gamepad_state.numAxes.clamp(0, MAX_INPUTS as i32) as usize;

            for (i, buttonidx) in layout.buttons(real_button_count) {
                let digital = gamepad_state
                    .digitalButton
                    .get(buttonidx)
                    .copied()
                    .unwrap_or(false);
                if item.digital_button[i] != digital {
                    changes.push(Change::Button(i as u8, digital));
                }

                // store values to compare them in the next update
                item.analog_button[i] = gamepad_state
                    .analogButton
                    .get(buttonidx)
                    .copied()
                    .unwrap_or(0.0);
                item.digital_button[i] = digital;
            }

            for i in 0..real_axis_count {
                if item.hat_is_axis && i == 9 {
                    continue; // this axis is a hat
                }
                if item.axis[i] != gamepad_state.axis[i] {
                    changes.push(Change::Axis(i as u8, axis_value(gamepad_state.axis[i])));
                    item.axis[i] = gamepad_state.axis[i];
                }
            }

            if item.triggers_are_buttons {
                let first_trigger_button = first_trigger_button as usize;
                for i in 0..2 {
                    let analog = gamepad_state.analogButton[first_trigger_button + i];
                    if item.axis[real_axis_count + i] != analog {
                        changes.push(Change::Axis(
                            (real_axis_count + i) as u8,
                            axis_value((analog * 2.0) - 1.0),
                        ));
                        item.axis[real_axis_count + i] = analog;
                    }
                }
            }

            debug_assert!(item.nhats <= 1); // there is (currently) only ever one of these, faked from the d-pad buttons.
            if item.nhats != 0 {
                let mut value = HAT_CENTERED;
                if first_hat_button >= 0 {
                    let first_hat_button = first_hat_button as usize;
                    // this currently expects the first button to be up, then down, then left, then right.
                    if gamepad_state.digitalButton[first_hat_button] {
                        value |= HAT_UP;
                    } else if gamepad_state.digitalButton[first_hat_button + 1] {
                        value |= HAT_DOWN;
                    }
                    if gamepad_state.digitalButton[first_hat_button + 2] {
                        value |= HAT_LEFT;
                    } else if gamepad_state.digitalButton[first_hat_button + 3] {
                        value |= HAT_RIGHT;
                    }
                } else if item.hat_is_axis {
                    value = hat_from_emscripten_axis(gamepad_state.axis[9] as f32);
                }
                if item.hat != value {
                    item.hat = value;
                    changes.push(Change::Hat(value));
                }
            }

            item.timestamp = gamepad_state.timestamp;
            changes
        })
        .unwrap_or_default();

        for change in changes {
            match change {
                Change::Button(button, down) => {
                    send_joystick_button(timestamp, joystick, button, down)
                }
                Change::Axis(axis, value) => send_joystick_axis(timestamp, joystick, axis, value),
                Change::Hat(value) => send_joystick_hat(timestamp, joystick, 0, value),
            }
        }
    }

    /// Translation of `EMSCRIPTEN_JoystickClose()`.
    fn close(&self, joystick: &mut JoystickData) {
        with_hwdata(joystick.instance_id, |item| item.joystick = None);
    }

    /// Translation of `EMSCRIPTEN_JoystickQuit()`.
    fn quit(&self) {
        joylist().clear();

        // SAFETY: removes the callbacks (NULL), which emscripten allows.
        unsafe {
            sys::emscripten_set_gamepadconnected_callback_on_thread(
                std::ptr::null_mut(),
                false,
                None,
                sys::EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
            );
            sys::emscripten_set_gamepaddisconnected_callback_on_thread(
                std::ptr::null_mut(),
                false,
                None,
                sys::EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
            );
        }
    }

    /// Translation of `EMSCRIPTEN_JoystickGetGamepadMapping()`.
    fn gamepad_mapping(&self, _device_index: usize) -> Option<GamepadMapping> {
        None
    }
}

#[cfg(test)]
mod tests;
