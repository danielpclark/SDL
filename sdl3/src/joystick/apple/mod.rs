// Rust translation of src/joystick/apple/SDL_mfijoystick.m and
// SDL_mfijoystick_c.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The MFi joystick driver (`SDL_IOS_JoystickDriver`, built with
//! `SDL_JOYSTICK_MFI` on macOS): the controllers the GameController
//! framework knows (`GCController`), from macOS 11, read through their
//! physical input profile, with rumble through CoreHaptics engines, motion
//! sensors, battery, light and the DualShock touchpad.
//!
//! Only the macOS build is translated; the iOS/tvOS parts (UIKit's game
//! controller interaction, the Siri Remote's micro gamepad and rotation
//! hint, the tvOS menu button, CoreMotion) are left out. The extended
//! gamepad path for systems before macOS 11 is translated as upstream has
//! it, though the driver doesn't start there.
//!
//! Upstream's device list is a linked list of `SDL_JoystickDeviceItem`s that
//! open joysticks point into (`hwdata`); here the devices are a `Vec`, and
//! an open joystick's `hwdata` is the device with its instance ID whose
//! `joystick` is set (a removed device is gone from the list, as upstream
//! clears `hwdata`).

mod sys;
#[cfg(test)]
mod tests;

use std::ffi::{c_char, c_void};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use self::sys::*;
use super::gamepad::{
    GamepadAxis, GamepadButton, GamepadMapping, GamepadType, InputMapping, MappingKind,
};
use super::usb_ids::*;
use super::{
    create_joystick_guid, create_joystick_name, private_joystick_added, private_joystick_removed,
    send_joystick_axis, send_joystick_button, send_joystick_hat, send_joystick_power_info,
    send_joystick_sensor, send_joystick_touchpad, should_ignore_joystick, JoystickData,
    JoystickDriver, HARDWARE_BUS_BLUETOOTH, HAT_CENTERED, HAT_DOWN, HAT_LEFT, HAT_RIGHT, HAT_UP,
    PROP_JOYSTICK_CAP_RGB_LED_BOOLEAN, PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN,
    PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN,
};
use crate::core::apple::block::RcBlock;
use crate::core::apple::foundation::{
    kCFBooleanTrue, macos_at_least, ns_array_items, ns_string, string_from_ns, NSTimeInterval,
};
use crate::core::apple::iokit::{IOHIDDeviceGetProperty, IOHIDDeviceRef, K_IOHID_VIRTUAL_HIDEVICE};
use crate::core::apple::objc::{
    autoreleasepool, class, is_kind_of, msg_send, sel, Bool, Id, NSInteger, NSUInteger, StrongPtr,
    NIL, NO, YES,
};
use crate::error::{Error, Result};
use crate::events::JoystickID;
use crate::guid::Guid;
use crate::hints;
use crate::power::PowerState;
use crate::sensor::{SensorType, STANDARD_GRAVITY};

/// `NSASCIIStringEncoding`.
const NS_ASCII_STRING_ENCODING: NSUInteger = 1;

/// `1 << SDL_GAMEPAD_BUTTON_*`, for `button_mask`.
const fn button_bit(button: GamepadButton) -> u32 {
    1 << (button as u32)
}

/// Translation of `struct joystick_hwdata` (`SDL_JoystickDeviceItem`).
#[derive(Default)]
pub(crate) struct MfiDevice {
    /// The `GCController` (retained; upstream's `CFBridgingRetain()`).
    controller: StrongPtr,
    rumble: Option<Arc<RumbleContext>>,
    pause_button_index: i32,
    pause_button_pressed: u64,

    name: String,
    /// The open joystick (`joystick`, whose `hwdata` is this device).
    joystick: Option<JoystickID>,
    instance_id: JoystickID,
    guid: Guid,

    naxes: usize,
    nbuttons: usize,
    nhats: usize,
    button_mask: u32,
    is_xbox: bool,
    is_ps4: bool,
    is_ps5: bool,
    is_switch_pro: bool,
    is_switch_joycon_pair: bool,
    is_switch_joycon_l: bool,
    is_switch_joycon_r: bool,
    is_shield: bool,
    is_stadia: bool,
    is_backbone_one: bool,
    is_siri_remote: i32,

    /// The element names used as axes, and as buttons (upstream's retained
    /// `NSArray`s of the profile's keys), with their `NSString`s.
    axes: Vec<ElementKey>,
    buttons: Vec<ElementKey>,

    has_dualshock_touchpad: bool,
    has_xbox_paddles: bool,
    has_xbox_share_button: bool,
    has_nintendo_buttons: bool,
}

/// A key of the physical input profile's element dictionary.
#[derive(Clone, Debug, Default)]
pub(crate) struct ElementKey {
    pub(crate) name: String,
    key: StrongPtr,
}

impl ElementKey {
    #[cfg(test)]
    pub(crate) fn new(name: &str) -> ElementKey {
        ElementKey {
            name: name.to_owned(),
            key: ns_string(name),
        }
    }

    /// An existing `NSString` key (retained).
    ///
    /// # Safety
    ///
    /// `key` must be a valid `NSString`.
    unsafe fn from_ns(key: Id) -> ElementKey {
        // SAFETY: the caller's contract.
        unsafe {
            ElementKey {
                name: string_from_ns(key).unwrap_or_default(),
                key: StrongPtr::retain(key),
            }
        }
    }
}

/// The driver's state: `deviceList`/`numjoysticks` and the notification
/// observers (`connectObserver`, `disconnectObserver`).
#[derive(Default)]
struct MfiState {
    devices: Vec<MfiDevice>,
    connect_observer: StrongPtr,
    disconnect_observer: StrongPtr,
}

impl MfiState {
    /// Translation of `GetDeviceForIndex()`.
    fn device_for_index(&mut self, device_index: usize) -> Option<&mut MfiDevice> {
        self.devices.get_mut(device_index)
    }

    /// `joystick->hwdata`: the device of an open joystick.
    fn hwdata(&mut self, joystick: JoystickID) -> Option<&mut MfiDevice> {
        self.devices
            .iter_mut()
            .find(|d| d.instance_id == joystick && d.joystick == Some(joystick))
    }
}

static STATE: Mutex<MfiState> = Mutex::new(MfiState {
    devices: Vec::new(),
    connect_observer: StrongPtr::nil(),
    disconnect_observer: StrongPtr::nil(),
});

fn lock_state() -> MutexGuard<'static, MfiState> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `SDL_AppleTVRemoteOpenedAsJoystick` (read by UIKit's
/// keyboard handling, which isn't translated; kept as upstream counts it).
pub(crate) static APPLE_TV_REMOTE_OPENED_AS_JOYSTICK: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

/// The `GCInput*` element names upstream compares with, read once from the
/// framework.
pub(crate) struct InputNames {
    pub(crate) button_a: String,
    pub(crate) button_b: String,
    pub(crate) button_x: String,
    pub(crate) button_y: String,
    pub(crate) left_shoulder: String,
    pub(crate) right_shoulder: String,
    pub(crate) left_trigger: String,
    pub(crate) right_trigger: String,
    pub(crate) left_thumbstick_button: String,
    pub(crate) right_thumbstick_button: String,
    pub(crate) button_menu: String,
    pub(crate) button_options: String,
    pub(crate) dual_shock_touchpad_button: String,
    pub(crate) xbox_paddle_one: String,
    pub(crate) xbox_paddle_two: String,
    pub(crate) xbox_paddle_three: String,
    pub(crate) xbox_paddle_four: String,
}

/// The [`InputNames`] (empty strings when GameController isn't there).
pub(crate) fn input_names() -> &'static InputNames {
    static NAMES: std::sync::OnceLock<InputNames> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        load_frameworks();
        InputNames {
            button_a: constant_str(gc_input_button_a()),
            button_b: constant_str(gc_input_button_b()),
            button_x: constant_str(gc_input_button_x()),
            button_y: constant_str(gc_input_button_y()),
            left_shoulder: constant_str(gc_input_left_shoulder()),
            right_shoulder: constant_str(gc_input_right_shoulder()),
            left_trigger: constant_str(gc_input_left_trigger()),
            right_trigger: constant_str(gc_input_right_trigger()),
            left_thumbstick_button: constant_str(gc_input_left_thumbstick_button()),
            right_thumbstick_button: constant_str(gc_input_right_thumbstick_button()),
            button_menu: constant_str(gc_input_button_menu()),
            button_options: constant_str(gc_input_button_options()),
            dual_shock_touchpad_button: constant_str(gc_input_dual_shock_touchpad_button()),
            xbox_paddle_one: constant_str(gc_input_xbox_paddle_one()),
            xbox_paddle_two: constant_str(gc_input_xbox_paddle_two()),
            xbox_paddle_three: constant_str(gc_input_xbox_paddle_three()),
            xbox_paddle_four: constant_str(gc_input_xbox_paddle_four()),
        }
    })
}

/// `[s isEqualToString:other]` for an `NSString` property, nil being
/// unequal.
///
/// # Safety
///
/// `s` must be nil or a valid `NSString`.
unsafe fn ns_equals(s: Id, other: &str) -> bool {
    // SAFETY: the caller's contract.
    unsafe { string_from_ns(s) }.is_some_and(|s| s == other)
}

/// `controller.productCategory` (macOS 10.15+).
///
/// # Safety
///
/// `controller` must be a valid `GCController`.
unsafe fn product_category(controller: Id) -> Option<String> {
    // SAFETY: the caller's contract.
    unsafe { string_from_ns(msg_send![controller, productCategory]) }
}

/// `controller.vendorName`.
///
/// # Safety
///
/// `controller` must be a valid `GCController`.
unsafe fn vendor_name(controller: Id) -> Option<String> {
    // SAFETY: the caller's contract.
    unsafe { string_from_ns(msg_send![controller, vendorName]) }
}

/// The `IsController*()` checks of the product category (macOS 10.15+),
/// else of the vendor name for the ones that have a fallback.
///
/// # Safety
///
/// `controller` must be a valid `GCController`.
unsafe fn is_controller_category(
    controller: Id,
    category: &str,
    vendor_fallback: Option<&str>,
) -> bool {
    // SAFETY: the caller's contract.
    unsafe {
        if macos_at_least(10, 15, 0) {
            if product_category(controller).is_some_and(|c| c == category) {
                return true;
            }
        } else if let Some(fallback) = vendor_fallback {
            if vendor_name(controller).is_some_and(|v| v.contains(fallback)) {
                return true;
            }
        }
    }
    false
}

/// Translation of `IsControllerPS4()`.
unsafe fn is_controller_ps4(controller: Id) -> bool {
    // SAFETY: the caller's contract.
    unsafe { is_controller_category(controller, "DualShock 4", Some("DUALSHOCK")) }
}

/// Translation of `IsControllerPS5()`.
unsafe fn is_controller_ps5(controller: Id) -> bool {
    // SAFETY: the caller's contract.
    unsafe { is_controller_category(controller, "DualSense", Some("DualSense")) }
}

/// Translation of `IsControllerXbox()`.
unsafe fn is_controller_xbox(controller: Id) -> bool {
    // SAFETY: the caller's contract.
    unsafe { is_controller_category(controller, "Xbox One", Some("Xbox")) }
}

/// Translation of `IsControllerSwitchPro()`.
unsafe fn is_controller_switch_pro(controller: Id) -> bool {
    // SAFETY: the caller's contract.
    unsafe { is_controller_category(controller, "Switch Pro Controller", None) }
}

/// Translation of `IsControllerSwitchJoyConL()`.
unsafe fn is_controller_switch_joycon_l(controller: Id) -> bool {
    // SAFETY: the caller's contract.
    unsafe { is_controller_category(controller, "Nintendo Switch Joy-Con (L)", None) }
}

/// Translation of `IsControllerSwitchJoyConR()`.
unsafe fn is_controller_switch_joycon_r(controller: Id) -> bool {
    // SAFETY: the caller's contract.
    unsafe { is_controller_category(controller, "Nintendo Switch Joy-Con (R)", None) }
}

/// Translation of `IsControllerSwitchJoyConPair()`.
unsafe fn is_controller_switch_joycon_pair(controller: Id) -> bool {
    // SAFETY: the caller's contract.
    unsafe { is_controller_category(controller, "Nintendo Switch Joy-Con (L/R)", None) }
}

/// Translation of `IsControllerNVIDIASHIELD()`.
unsafe fn is_controller_nvidia_shield(controller: Id) -> bool {
    // SAFETY: the caller's contract.
    unsafe { vendor_name(controller) }.is_some_and(|v| v.starts_with("NVIDIA Controller"))
}

/// Translation of `IsControllerStadia()`.
unsafe fn is_controller_stadia(controller: Id) -> bool {
    // SAFETY: the caller's contract.
    unsafe { vendor_name(controller) }.is_some_and(|v| v.starts_with("Stadia"))
}

/// Translation of `IsControllerBackboneOne()`.
unsafe fn is_controller_backbone_one(controller: Id) -> bool {
    // SAFETY: the caller's contract.
    unsafe { vendor_name(controller) }.is_some_and(|v| v.starts_with("Backbone One"))
}

/// `SDL_sscanf(category, "Siri Remote (%i%*s Generation)", &n)`: the
/// generation number, `None` when the `%i` conversion doesn't happen (white
/// space in the format matches any amount of it, `%i` takes a sign and a
/// `0x`/`0` base prefix as `strtol()` does).
pub(crate) fn scan_siri_remote_generation(category: &str) -> Option<i32> {
    let mut s = category;
    for (i, literal) in ["Siri", "Remote", "("].into_iter().enumerate() {
        if i > 0 {
            s = s.trim_start_matches(|c: char| c.is_ascii_whitespace());
        }
        s = s.strip_prefix(literal)?;
    }
    // %i skips leading white space
    let s = s.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let (negative, s) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let (radix, digits) = if (s.starts_with("0x") || s.starts_with("0X"))
        && s[2..].starts_with(|c: char| c.is_ascii_hexdigit())
    {
        (16, &s[2..])
    } else if s.starts_with('0') {
        (8, s)
    } else {
        (10, s)
    };
    let len = digits
        .find(|c: char| !c.is_digit(radix))
        .unwrap_or(digits.len());
    if len == 0 {
        return None;
    }
    let value = i64::from_str_radix(&digits[..len], radix).unwrap_or(i64::MAX);
    let value = if negative {
        value.wrapping_neg()
    } else {
        value
    };
    Some(value as i32)
}

/// Translation of `CheckControllerSiriRemote()`.
unsafe fn check_controller_siri_remote(controller: Id) -> i32 {
    if macos_at_least(10, 15, 0) {
        // SAFETY: the caller's contract.
        if let Some(category) = unsafe { product_category(controller) } {
            if category.starts_with("Siri Remote") {
                return scan_siri_remote_generation(&category).unwrap_or(1);
            }
        }
    }
    0
}

/// Translation of `ElementAlreadyHandled()`; `has` says whether the
/// profile has an element (`elements[name] != nil`).
#[allow(clippy::collapsible_if)] // (upstream's nesting)
pub(crate) fn element_already_handled(
    device: &MfiDevice,
    element: &str,
    has: &dyn Fn(&str) -> bool,
) -> bool {
    if element == "Left Thumbstick Left" || element == "Left Thumbstick Right" {
        if has("Left Thumbstick X Axis") {
            return true;
        }
    }
    if element == "Left Thumbstick Up" || element == "Left Thumbstick Down" {
        if has("Left Thumbstick Y Axis") {
            return true;
        }
    }
    if element == "Right Thumbstick Left" || element == "Right Thumbstick Right" {
        if has("Right Thumbstick X Axis") {
            return true;
        }
    }
    if element == "Right Thumbstick Up" || element == "Right Thumbstick Down" {
        if has("Right Thumbstick Y Axis") {
            return true;
        }
    }
    if device.is_siri_remote != 0 {
        if element == "Direction Pad Left" || element == "Direction Pad Right" {
            if has("Direction Pad X Axis") {
                return true;
            }
        }
        if element == "Direction Pad Up" || element == "Direction Pad Down" {
            if has("Direction Pad Y Axis") {
                return true;
            }
        }
    } else {
        if element == "Direction Pad X Axis" {
            if has("Direction Pad Left") && has("Direction Pad Right") {
                return true;
            }
        }
        if element == "Direction Pad Y Axis" {
            if has("Direction Pad Up") && has("Direction Pad Down") {
                return true;
            }
        }
    }
    if element == "Cardinal Direction Pad X Axis" {
        if has("Cardinal Direction Pad Left") && has("Cardinal Direction Pad Right") {
            return true;
        }
    }
    if element == "Cardinal Direction Pad Y Axis" {
        if has("Cardinal Direction Pad Up") && has("Cardinal Direction Pad Down") {
            return true;
        }
    }
    if matches!(
        element,
        "Touchpad 1 X Axis"
            | "Touchpad 1 Y Axis"
            | "Touchpad 1 Left"
            | "Touchpad 1 Right"
            | "Touchpad 1 Up"
            | "Touchpad 1 Down"
            | "Touchpad 2 X Axis"
            | "Touchpad 2 Y Axis"
            | "Touchpad 2 Left"
            | "Touchpad 2 Right"
            | "Touchpad 2 Up"
            | "Touchpad 2 Down"
    ) {
        // The touchpad is handled separately
        return true;
    }
    if element == "Button Home" {
        if device.is_switch_joycon_pair {
            // The Nintendo Switch JoyCon home button doesn't ever show as being held down
            return true;
        }
    }
    if element == "Button Share" {
        if device.is_backbone_one {
            // The Backbone app uses share button
            return true;
        }
    }
    false
}

/// What an element of the physical input profile is (the class checks of
/// the element filters).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ElementKind {
    /// `GCControllerAxisInput`
    Axis,
    /// `GCControllerButtonInput`
    Button,
    /// Anything else (direction pads)
    Other,
}

/// The two `filteredArrayUsingPredicate:` filters of
/// `IOS_AddMFIJoystickDevice()` over the profile's keys sorted with
/// `caseInsensitiveCompare:`: "Provide both axes and analog buttons as SDL
/// axes", and every button as a button. `elements` are the sorted keys with
/// their kind and whether they are analog; the result is the indices of the
/// axes and of the buttons.
pub(crate) fn select_elements(
    device: &MfiDevice,
    elements: &[(&str, ElementKind, bool)],
    has: &dyn Fn(&str) -> bool,
) -> (Vec<usize>, Vec<usize>) {
    let axes = elements
        .iter()
        .enumerate()
        .filter(|(_, &(name, kind, analog))| {
            if element_already_handled(device, name, has) {
                return false;
            }
            analog && matches!(kind, ElementKind::Axis | ElementKind::Button)
        })
        .map(|(i, _)| i)
        .collect();
    let buttons = elements
        .iter()
        .enumerate()
        .filter(|(_, &(name, kind, _))| {
            if element_already_handled(device, name, has) {
                return false;
            }
            kind == ElementKind::Button
        })
        .map(|(i, _)| i)
        .collect();
    (axes, buttons)
}

/// `physicalInputProfile.elements` of a controller (macOS 11+).
///
/// # Safety
///
/// `controller` must be a valid `GCController`.
unsafe fn profile_elements(controller: Id) -> Id {
    // SAFETY: the caller's contract.
    unsafe {
        let profile: Id = msg_send![controller, physicalInputProfile];
        msg_send![profile, elements]
    }
}

/// `physicalInputProfile.buttons` of a controller (macOS 11+).
///
/// # Safety
///
/// `controller` must be a valid `GCController`.
unsafe fn profile_buttons(controller: Id) -> Id {
    // SAFETY: the caller's contract.
    unsafe {
        let profile: Id = msg_send![controller, physicalInputProfile];
        msg_send![profile, buttons]
    }
}

/// `dict[key]` with a string key.
///
/// # Safety
///
/// `dict` must be nil or a valid `NSDictionary`.
unsafe fn dict_get(dict: Id, key: &str) -> Id {
    let key = ns_string(key);
    // SAFETY: the caller's contract.
    unsafe { msg_send![dict, objectForKey: key.get()] }
}

/// The values of an `NSDictionary` with their keys (`for (id key in dict)`).
///
/// # Safety
///
/// `dict` must be nil or a valid `NSDictionary`.
unsafe fn dict_entries(dict: Id) -> Vec<(Id, Id)> {
    // SAFETY: the caller's contract; the keys are the dictionary's.
    unsafe {
        let keys: Id = msg_send![dict, allKeys];
        ns_array_items(keys)
            .into_iter()
            .map(|k| (k, msg_send![dict, objectForKey: k]))
            .collect()
    }
}

/// Set `preferredSystemGestureState` of the buttons bound to a system
/// gesture (the loops of `IOS_AddMFIJoystickDevice()`, `IOS_JoystickOpen()`
/// and `IOS_JoystickClose()`, macOS 11+).
///
/// # Safety
///
/// `controller` must be a valid `GCController`.
unsafe fn set_system_gesture_state(controller: Id, state: NSInteger) {
    // SAFETY: the caller's contract.
    unsafe {
        for (_, button) in dict_entries(profile_buttons(controller)) {
            let bound: Bool = msg_send![button, isBoundToSystemGesture];
            if bound.is_true() {
                let _: () = msg_send![button, setPreferredSystemGestureState: state];
            }
        }
    }
}

/// Whether one of the HIDAPI drivers handles the controller, the long
/// condition of `IOS_AddMFIJoystickDevice()`.
fn hidapi_handles(device: &MfiDevice, name: &str) -> bool {
    use super::hidapi::{is_device_present, is_device_type_present};
    let present = |vendor, product| is_device_present(vendor, product, 0, Some(""));
    (device.is_xbox
        && (is_device_type_present(GamepadType::XboxOne)
            || is_device_type_present(GamepadType::Xbox360)))
        || (device.is_ps4 && is_device_type_present(GamepadType::Ps4))
        || (device.is_ps5 && is_device_type_present(GamepadType::Ps5))
        || (device.is_switch_pro && is_device_type_present(GamepadType::NintendoSwitchPro))
        || (device.is_switch_joycon_pair
            && present(USB_VENDOR_NINTENDO, USB_PRODUCT_NINTENDO_SWITCH_JOYCON_PAIR))
        || (device.is_shield
            && present(USB_VENDOR_NVIDIA, USB_PRODUCT_NVIDIA_SHIELD_CONTROLLER_V104))
        || (device.is_stadia && present(USB_VENDOR_GOOGLE, USB_PRODUCT_GOOGLE_STADIA_CONTROLLER))
        || (device.is_switch_joycon_l
            && present(USB_VENDOR_NINTENDO, USB_PRODUCT_NINTENDO_SWITCH_JOYCON_LEFT))
        || (device.is_switch_joycon_r
            && present(
                USB_VENDOR_NINTENDO,
                USB_PRODUCT_NINTENDO_SWITCH_JOYCON_RIGHT,
            ))
        || (name.contains("GameCube Controller Adapter")
            && (present(USB_VENDOR_NINTENDO, USB_PRODUCT_NINTENDO_GAMECUBE_ADAPTER)
                || present(
                    USB_VENDOR_DRAGONRISE,
                    USB_PRODUCT_EVORETRO_GAMECUBE_ADAPTER1,
                )
                || present(
                    USB_VENDOR_DRAGONRISE,
                    USB_PRODUCT_EVORETRO_GAMECUBE_ADAPTER2,
                )
                || present(
                    USB_VENDOR_DRAGONRISE,
                    USB_PRODUCT_EVORETRO_GAMECUBE_ADAPTER3,
                )))
        || (name == "8Bitdo SN30 Pro"
            && (present(USB_VENDOR_8BITDO, USB_PRODUCT_8BITDO_SN30_PRO)
                || present(USB_VENDOR_8BITDO, USB_PRODUCT_8BITDO_SN30_PRO_BT)))
        || (name == "8BitDo Pro 2"
            && (present(USB_VENDOR_8BITDO, USB_PRODUCT_8BITDO_PRO_2)
                || present(USB_VENDOR_8BITDO, USB_PRODUCT_8BITDO_PRO_2_BT)))
        || (name == "8BitDo Pro 3" && present(USB_VENDOR_8BITDO, USB_PRODUCT_8BITDO_PRO_3))
        || (name.starts_with("8BitDo Ultimate 2 Wireless")
            && present(USB_VENDOR_8BITDO, USB_PRODUCT_8BITDO_ULTIMATE2_WIRELESS))
        || (name.starts_with("Steam Controller") && is_device_type_present(GamepadType::Steam))
}

/// The signature of a device for its GUID (macOS 11+): the CRC-16 of its
/// name and of the names of its axes and buttons.
pub(crate) fn element_signature(device: &MfiDevice) -> u16 {
    let mut signature = 0;
    signature = crate::stdlib::crc16(signature, device.name.as_bytes());
    for key in &device.axes {
        signature = crate::stdlib::crc16(signature, key.name.as_bytes());
    }
    for key in &device.buttons {
        signature = crate::stdlib::crc16(signature, key.name.as_bytes());
    }
    signature
}

/// Translation of `IOS_AddMFIJoystickDevice()`.
///
/// # Safety
///
/// `controller` must be a valid `GCController`.
unsafe fn add_mfi_joystick_device(device: &mut MfiDevice, controller: Id) -> bool {
    let vendor: u16;
    let product: u16;
    let mut subtype = 0u8;

    // SAFETY: the caller's contract; the messages are GCController's and its
    // profile's (macOS 11+ ones behind their checks).
    unsafe {
        // Technically we only want to do this when the controller is opened, but we ALSO
        // want to do this when the controller is opened via HIDAPI or IOKit, and we don't
        // have an easy way to know if those devices correspond to a specific GCController.
        //
        // Since the fact that we're initializing means that the application is likely to
        // be opening available controllers, this is probably the right thing to do for now.
        if macos_at_least(11, 0, 0) {
            set_system_gesture_state(controller, GC_SYSTEM_GESTURE_STATE_DISABLED);
        }

        if macos_at_least(11, 3, 0) {
            let gc = class!(GCController);
            let monitor: Bool = msg_send![gc, shouldMonitorBackgroundEvents];
            if !monitor.is_true() {
                let _: () = msg_send![gc, setShouldMonitorBackgroundEvents: YES];
            }
        }

        /* Explicitly retain the controller because SDL_JoystickDeviceItem is a
         * struct, and ARC doesn't work with structs. */
        device.controller = StrongPtr::retain(controller);

        let name = if let Some(vendor_name) = vendor_name(controller) {
            Some(vendor_name)
        } else if macos_at_least(10, 15, 0) {
            product_category(controller)
        } else {
            None
        };

        let mut name = name.unwrap_or_else(|| "MFi Gamepad".to_owned());

        device.is_xbox = is_controller_xbox(controller);
        device.is_ps4 = is_controller_ps4(controller);
        device.is_ps5 = is_controller_ps5(controller);
        device.is_switch_pro = is_controller_switch_pro(controller);
        device.is_switch_joycon_pair = is_controller_switch_joycon_pair(controller);
        device.is_shield = is_controller_nvidia_shield(controller);
        device.is_stadia = is_controller_stadia(controller);
        device.is_backbone_one = is_controller_backbone_one(controller);
        device.is_switch_joycon_l = is_controller_switch_joycon_l(controller);
        device.is_switch_joycon_r = is_controller_switch_joycon_r(controller);
        if hidapi_handles(device, &name) {
            // The HIDAPI driver is taking care of this device
            return false;
        }
        if device.is_xbox && name.starts_with("GamePad-") {
            // This is a Steam Virtual Gamepad, which isn't supported by GCController
            return false;
        }
        device.is_siri_remote = check_controller_siri_remote(controller);

        if device.is_siri_remote != 0 && !hints::get_bool(hints::TV_REMOTE_AS_JOYSTICK, true) {
            // Ignore remotes, they'll be handled as keyboard input
            return false;
        }

        if macos_at_least(11, 0, 0) {
            let buttons = profile_buttons(controller);
            let names = input_names();
            if !dict_get(buttons, &names.dual_shock_touchpad_button).is_null() {
                device.has_dualshock_touchpad = true;
            }
            if !dict_get(buttons, &names.xbox_paddle_one).is_null() {
                device.has_xbox_paddles = true;
            }
            if !dict_get(buttons, "Button Share").is_null() {
                device.has_xbox_share_button = true;
            }
        }

        let extended_gamepad: Id = msg_send![controller, extendedGamepad];
        if device.is_backbone_one {
            vendor = USB_VENDOR_BACKBONE;
            if device.is_ps5 {
                product = USB_PRODUCT_BACKBONE_ONE_IOS_PS5;
            } else {
                product = USB_PRODUCT_BACKBONE_ONE_IOS;
            }
        } else if device.is_xbox {
            vendor = USB_VENDOR_MICROSOFT;
            if device.has_xbox_paddles {
                // Assume Xbox One Elite Series 2 Controller unless/until GCController flows VID/PID
                product = USB_PRODUCT_XBOX_ONE_ELITE_SERIES_2_BLUETOOTH;

                // controller.vendorName returns a generic name for Xbox controllers ("Controller"),
                // and controller.productCategory only returns "Xbox One" for those controllers,
                // so we give them proper names based on the ones from SDL_gamepad_db.h
                name = "Xbox One Elite 2 Controller".to_owned();
            } else if device.has_xbox_share_button {
                // Assume Xbox Series X Controller unless/until GCController flows VID/PID
                product = USB_PRODUCT_XBOX_SERIES_X_BLE;
                name = "Xbox Series X Controller".to_owned();
            } else {
                // Assume Xbox One S Bluetooth Controller unless/until GCController flows VID/PID
                product = USB_PRODUCT_XBOX_ONE_S_REV1_BLUETOOTH;
                name = "Xbox One Wireless Controller".to_owned();
            }
        } else if device.is_ps4 {
            // Assume DS4 Slim unless/until GCController flows VID/PID
            vendor = USB_VENDOR_SONY;
            product = USB_PRODUCT_SONY_DS4_SLIM;
            if device.has_dualshock_touchpad {
                subtype = 1;
            }
        } else if device.is_ps5 {
            vendor = USB_VENDOR_SONY;
            product = USB_PRODUCT_SONY_DS5;
        } else if device.is_switch_pro {
            vendor = USB_VENDOR_NINTENDO;
            product = USB_PRODUCT_NINTENDO_SWITCH_PRO;
            device.has_nintendo_buttons = true;
        } else if device.is_switch_joycon_pair {
            vendor = USB_VENDOR_NINTENDO;
            product = USB_PRODUCT_NINTENDO_SWITCH_JOYCON_PAIR;
            device.has_nintendo_buttons = true;
        } else if device.is_switch_joycon_l {
            vendor = USB_VENDOR_NINTENDO;
            product = USB_PRODUCT_NINTENDO_SWITCH_JOYCON_LEFT;
        } else if device.is_switch_joycon_r {
            vendor = USB_VENDOR_NINTENDO;
            product = USB_PRODUCT_NINTENDO_SWITCH_JOYCON_RIGHT;
        } else if macos_at_least(11, 0, 0) {
            vendor = USB_VENDOR_APPLE;
            product = 4;
            subtype = 4;
        } else if !extended_gamepad.is_null() {
            vendor = USB_VENDOR_APPLE;
            product = 1;
            subtype = 1;
        } else {
            // We don't know how to get input events from this device
            return false;
        }

        if should_ignore_joystick(vendor, product, 0, Some(&name)) {
            return false;
        }

        if macos_at_least(11, 0, 0) {
            let elements = profile_elements(controller);

            // Provide both axes and analog buttons as SDL axes
            let keys: Id = msg_send![elements, allKeys];
            let keys: Id = msg_send![keys, sortedArrayUsingSelector: sel!(caseInsensitiveCompare:)];
            let axis_class = class!(GCControllerAxisInput);
            let button_class = class!(GCControllerButtonInput);
            let keys = ns_array_items(keys);
            let entries: Vec<(String, ElementKind, bool)> = keys
                .iter()
                .map(|&key| {
                    let element: Id = msg_send![elements, objectForKey: key];
                    let kind = if is_kind_of(element, axis_class) {
                        ElementKind::Axis
                    } else if is_kind_of(element, button_class) {
                        ElementKind::Button
                    } else {
                        ElementKind::Other
                    };
                    let analog: Bool = msg_send![element, isAnalog];
                    (
                        string_from_ns(key).unwrap_or_default(),
                        kind,
                        analog.is_true(),
                    )
                })
                .collect();
            let entries: Vec<(&str, ElementKind, bool)> = entries
                .iter()
                .map(|(name, kind, analog)| (name.as_str(), *kind, *analog))
                .collect();
            let has = |name: &str| !dict_get(elements, name).is_null();
            // (upstream filters the sorted keys with NSPredicate blocks)
            let (axes, buttons) = select_elements(device, &entries, &has);
            /* Explicitly retain the arrays because SDL_JoystickDeviceItem is a
             * struct, and ARC doesn't work with structs. */
            device.axes = axes.iter().map(|&i| ElementKey::from_ns(keys[i])).collect();
            device.naxes = device.axes.len();
            device.buttons = buttons
                .iter()
                .map(|&i| ElementKey::from_ns(keys[i]))
                .collect();
            device.nbuttons = device.buttons.len();
            subtype = 4;
        } else if !extended_gamepad.is_null() {
            let gamepad = extended_gamepad;
            let mut nbuttons = 0;
            let mut has_direct_menu = false;

            // These buttons are part of the original MFi spec
            device.button_mask |= button_bit(GamepadButton::South);
            device.button_mask |= button_bit(GamepadButton::East);
            device.button_mask |= button_bit(GamepadButton::West);
            device.button_mask |= button_bit(GamepadButton::North);
            device.button_mask |= button_bit(GamepadButton::LeftShoulder);
            device.button_mask |= button_bit(GamepadButton::RightShoulder);
            nbuttons += 6;

            // These buttons are available on some newer controllers
            if macos_at_least(10, 14, 1) {
                let b: Id = msg_send![gamepad, leftThumbstickButton];
                if !b.is_null() {
                    device.button_mask |= button_bit(GamepadButton::LeftStick);
                    nbuttons += 1;
                }
                let b: Id = msg_send![gamepad, rightThumbstickButton];
                if !b.is_null() {
                    device.button_mask |= button_bit(GamepadButton::RightStick);
                    nbuttons += 1;
                }
            }
            if macos_at_least(10, 15, 0) {
                let b: Id = msg_send![gamepad, buttonOptions];
                if !b.is_null() {
                    device.button_mask |= button_bit(GamepadButton::Back);
                    nbuttons += 1;
                }
            }
            device.button_mask |= button_bit(GamepadButton::Start);
            nbuttons += 1;

            if macos_at_least(10, 15, 0) {
                let b: Id = msg_send![gamepad, buttonMenu];
                if !b.is_null() {
                    has_direct_menu = true;
                }
            }
            if !has_direct_menu {
                device.pause_button_index = nbuttons - 1;
            }

            device.naxes = 6; // 2 thumbsticks and 2 triggers
            device.nhats = 1; // d-pad
            device.nbuttons = nbuttons as usize;
        } else {
            // We don't know how to get input events from this device
            return false;
        }

        device.name = create_joystick_name(0, 0, None, Some(&name)).unwrap_or_else(|| name.clone());

        let signature = if macos_at_least(11, 0, 0) {
            element_signature(device)
        } else {
            device.button_mask as u16
        };
        device.guid = create_joystick_guid(
            HARDWARE_BUS_BLUETOOTH,
            vendor,
            product,
            signature,
            None,
            Some(&name),
            b'm',
            subtype,
        );

        /* This will be set when the first button press of the controller is
         * detected. */
        let _: () = msg_send![controller, setPlayerIndex: -1 as NSInteger];
    }
    true
}

/// Translation of `IOS_AddJoystickDevice()`.
///
/// # Safety
///
/// `controller` must be nil or a valid `GCController`.
unsafe fn add_joystick_device(controller: Id) {
    if lock_state()
        .devices
        .iter()
        .any(|d| d.controller.get() == controller)
    {
        return;
    }

    let mut device = MfiDevice {
        instance_id: crate::utils::next_object_id(),
        pause_button_index: -1,
        ..MfiDevice::default()
    };

    // SAFETY: the caller's contract.
    if !controller.is_null() && !unsafe { add_mfi_joystick_device(&mut device, controller) } {
        // Note (upstream): the controller and arrays retained before the
        // device was refused are leaked there; here they are released.
        return;
    }

    let instance_id = device.instance_id;
    lock_state().devices.push(device);

    private_joystick_added(instance_id);
}

/// Translation of `IOS_RemoveJoystickDevice()`: unlink the device at
/// `index` and report it removed.
fn remove_joystick_device(index: usize) {
    let device = {
        let mut state = lock_state();
        if index >= state.devices.len() {
            return;
        }
        state.devices.remove(index)
    };

    // (an open joystick's hwdata goes with the device)

    autoreleasepool(|| {
        // These were explicitly retained in the struct, so they should be explicitly released before freeing the struct.
        if !device.controller.is_nil() {
            // SAFETY: the device's controller.
            let _: () = unsafe { msg_send![device.controller, setControllerPausedHandler: NIL] };
        }
        // Note (upstream): the rumble context of a joystick still open when its
        // controller goes away is leaked there (IOS_JoystickClose() returns
        // early without hwdata); here it is released with the device.
        drop(device.rumble);
        drop(device.controller);
        drop(device.axes);
        drop(device.buttons);
    });

    private_joystick_removed(device.instance_id);
}

/// The `GCControllerDidConnectNotification` block.
fn connect_block() -> RcBlock {
    RcBlock::new1(|note: Id| {
        // SAFETY: the notification's object is the GCController.
        let controller: Id = unsafe { msg_send![note, object] };
        let _lock = super::lock_joysticks();
        // SAFETY: as above.
        unsafe { add_joystick_device(controller) };
    })
}

/// The `GCControllerDidDisconnectNotification` block.
fn disconnect_block() -> RcBlock {
    RcBlock::new1(|note: Id| {
        // SAFETY: the notification's object is the GCController.
        let controller: Id = unsafe { msg_send![note, object] };
        let _lock = super::lock_joysticks();
        let index = lock_state()
            .devices
            .iter()
            .position(|d| d.controller.get() == controller);
        if let Some(index) = index {
            remove_joystick_device(index);
        }
    })
}

/// Translation of `IOS_MFIJoystickHatStateForDPad()`.
///
/// # Safety
///
/// `dpad` must be nil or a valid `GCControllerDirectionPad`.
unsafe fn hat_state_for_dpad(dpad: Id) -> u8 {
    let mut hat = 0;

    // SAFETY: the caller's contract; the directions are buttons.
    unsafe {
        let pressed = |button: Id| -> bool {
            let p: Bool = msg_send![button, isPressed];
            p.is_true()
        };
        if pressed(msg_send![dpad, up]) {
            hat |= HAT_UP;
        } else if pressed(msg_send![dpad, down]) {
            hat |= HAT_DOWN;
        }

        if pressed(msg_send![dpad, left]) {
            hat |= HAT_LEFT;
        } else if pressed(msg_send![dpad, right]) {
            hat |= HAT_RIGHT;
        }
    }

    if hat == 0 {
        return HAT_CENTERED;
    }

    hat
}

/// What `IOS_MFIJoystickUpdate()` sends, collected with the state locked.
enum Input {
    Axis(u8, i16),
    Button(u8, bool),
    Hat(u8, u8),
    Touchpad(i32, bool, f32, f32),
    Sensor(SensorType, [f32; 3]),
    Power(PowerState, i32),
}

/// `-[GCControllerAxisInput value]`/`-[GCControllerButtonInput value]`.
///
/// # Safety
///
/// `input` must be nil or a valid axis or button input.
unsafe fn input_value(input: Id) -> f32 {
    // SAFETY: the caller's contract.
    unsafe { msg_send![input, value] }
}

/// `-[GCControllerButtonInput isPressed]`.
///
/// # Safety
///
/// `button` must be nil or a valid button input.
unsafe fn is_pressed(button: Id) -> bool {
    // SAFETY: the caller's contract.
    let p: Bool = unsafe { msg_send![button, isPressed] };
    p.is_true()
}

/// Translation of `IOS_MFIJoystickUpdate()`, the reading part: what the
/// controller's state says to send.
fn mfi_joystick_update(device: &mut MfiDevice, timestamp_ms: u64) -> Vec<Input> {
    let mut inputs = Vec::new();
    autoreleasepool(|| {
        let controller = device.controller.get();
        let mut hatstate = HAT_CENTERED;

        // SAFETY: the device's controller and its elements, by their types.
        unsafe {
            if macos_at_least(11, 0, 0) {
                let elements = profile_elements(controller);
                let buttons = profile_buttons(controller);

                for (axis, key) in device.axes.iter().enumerate() {
                    let element: Id = msg_send![elements, objectForKey: key.key.get()];
                    // (upstream casts to GCControllerAxisInput or to
                    // GCControllerButtonInput by the element's class; their
                    // `value` is the same message)
                    let value = (input_value(element) * 32767.0) as i16;
                    inputs.push(Input::Axis(axis as u8, value));
                }

                for (button, key) in device.buttons.iter().enumerate() {
                    let input: Id = msg_send![buttons, objectForKey: key.key.get()];
                    inputs.push(Input::Button(button as u8, is_pressed(input)));
                }
            } else {
                let gamepad: Id = msg_send![controller, extendedGamepad];
                if !gamepad.is_null() {
                    let stick_x = |stick: Id| -> f32 {
                        let a: Id = msg_send![stick, xAxis];
                        input_value(a)
                    };
                    let stick_y = |stick: Id| -> f32 {
                        let a: Id = msg_send![stick, yAxis];
                        input_value(a)
                    };
                    let left: Id = msg_send![gamepad, leftThumbstick];
                    let right: Id = msg_send![gamepad, rightThumbstick];
                    let left_trigger: Id = msg_send![gamepad, leftTrigger];
                    let right_trigger: Id = msg_send![gamepad, rightTrigger];

                    // Axis order matches the XInput Windows mappings.
                    let axes = [
                        (stick_x(left) * 32767.0) as i16,
                        (stick_y(left) * -32767.0) as i16,
                        ((input_value(left_trigger) * 65535.0) - 32768.0) as i16,
                        (stick_x(right) * 32767.0) as i16,
                        (stick_y(right) * -32767.0) as i16,
                        ((input_value(right_trigger) * 65535.0) - 32768.0) as i16,
                    ];

                    // Button order matches the XInput Windows mappings.
                    let mut buttons = Vec::with_capacity(device.nbuttons);

                    // These buttons are part of the original MFi spec
                    buttons.push(is_pressed(msg_send![gamepad, buttonA]));
                    buttons.push(is_pressed(msg_send![gamepad, buttonB]));
                    buttons.push(is_pressed(msg_send![gamepad, buttonX]));
                    buttons.push(is_pressed(msg_send![gamepad, buttonY]));
                    buttons.push(is_pressed(msg_send![gamepad, leftShoulder]));
                    buttons.push(is_pressed(msg_send![gamepad, rightShoulder]));

                    // These buttons are available on some newer controllers
                    if macos_at_least(10, 14, 1) {
                        if device.button_mask & button_bit(GamepadButton::LeftStick) != 0 {
                            buttons.push(is_pressed(msg_send![gamepad, leftThumbstickButton]));
                        }
                        if device.button_mask & button_bit(GamepadButton::RightStick) != 0 {
                            buttons.push(is_pressed(msg_send![gamepad, rightThumbstickButton]));
                        }
                    }
                    if macos_at_least(10, 15, 0)
                        && device.button_mask & button_bit(GamepadButton::Back) != 0
                    {
                        buttons.push(is_pressed(msg_send![gamepad, buttonOptions]));
                    }
                    if device.button_mask & button_bit(GamepadButton::Start) != 0 {
                        if device.pause_button_index >= 0 {
                            // Guaranteed if buttonMenu is not supported on this OS
                            buttons.push(device.pause_button_pressed > 0);
                        } else if macos_at_least(10, 15, 0) {
                            buttons.push(is_pressed(msg_send![gamepad, buttonMenu]));
                        }
                    }

                    hatstate = hat_state_for_dpad(msg_send![gamepad, dpad]);

                    for (i, &value) in axes.iter().enumerate() {
                        inputs.push(Input::Axis(i as u8, value));
                    }

                    for (i, &down) in buttons.iter().enumerate() {
                        inputs.push(Input::Button(i as u8, down));
                    }
                }
            }

            if device.nhats > 0 {
                inputs.push(Input::Hat(0, hatstate));
            }

            if device.pause_button_pressed != 0 {
                // The pause callback is instantaneous, so we extend the duration to allow "holding down" by pressing it repeatedly
                const PAUSE_BUTTON_PRESS_DURATION_MS: u64 = 250;
                if timestamp_ms >= device.pause_button_pressed + PAUSE_BUTTON_PRESS_DURATION_MS {
                    device.pause_button_pressed = 0;
                }
            }

            if macos_at_least(11, 0, 0) && device.has_dualshock_touchpad {
                let profile: Id = msg_send![controller, physicalInputProfile];
                let dpads: Id = msg_send![profile, dpads];
                for (finger, name) in [
                    gc_input_dual_shock_touchpad_one(),
                    gc_input_dual_shock_touchpad_two(),
                ]
                .into_iter()
                .enumerate()
                {
                    let dpad: Id = msg_send![dpads, objectForKey: name];
                    let x = input_value(msg_send![dpad, xAxis]);
                    let y = input_value(msg_send![dpad, yAxis]);
                    if x != 0.0 || y != 0.0 {
                        inputs.push(Input::Touchpad(
                            finger as i32,
                            true,
                            (1.0 + x) * 0.5,
                            1.0 - (1.0 + y) * 0.5,
                        ));
                    } else {
                        inputs.push(Input::Touchpad(finger as i32, false, 0.0, 0.0));
                    }
                }
            }

            if macos_at_least(11, 0, 0) {
                let motion: Id = msg_send![controller, motion];
                let active: Bool = msg_send![motion, sensorsActive];
                if !motion.is_null() && active.is_true() {
                    let rate: GCRotationRate = msg_send![motion, rotationRate];
                    inputs.push(Input::Sensor(
                        SensorType::Gyro,
                        [rate.x as f32, rate.z as f32, -rate.y as f32],
                    ));

                    let accel: GCAcceleration = msg_send![motion, acceleration];
                    inputs.push(Input::Sensor(
                        SensorType::Accel,
                        [
                            -accel.x as f32 * STANDARD_GRAVITY,
                            -accel.y as f32 * STANDARD_GRAVITY,
                            -accel.z as f32 * STANDARD_GRAVITY,
                        ],
                    ));
                }
            }

            if macos_at_least(11, 0, 0) {
                let battery: Id = msg_send![controller, battery];
                if !battery.is_null() {
                    let level: f32 = msg_send![battery, batteryLevel];
                    let percent = (level * 100.0).round() as i32;
                    let battery_state: NSInteger = msg_send![battery, batteryState];
                    let state = match battery_state {
                        GC_DEVICE_BATTERY_STATE_DISCHARGING => PowerState::OnBattery,
                        GC_DEVICE_BATTERY_STATE_CHARGING => PowerState::Charging,
                        GC_DEVICE_BATTERY_STATE_FULL => PowerState::Charged,
                        _ => PowerState::Unknown,
                    };
                    inputs.push(Input::Power(state, percent));
                }
            }
        }
    });
    inputs
}

/// A rumble motor: a CoreHaptics engine of one of the controller's haptics
/// localities and the player of its continuous pattern. Translation of
/// `SDL3_RumbleMotor`; upstream's Objective-C class is a Rust structure
/// here (nothing else messages it), its properties behind a mutex the
/// engine's handlers share through a weak reference (upstream's `weakSelf`).
struct RumbleMotor {
    state: Arc<Mutex<MotorState>>,
}

#[derive(Default)]
struct MotorState {
    engine: StrongPtr,
    player: StrongPtr,
    active: bool,
}

/// The `localizedDescription` of an `NSError`.
///
/// # Safety
///
/// `error` must be a valid `NSError`.
unsafe fn error_description(error: Id) -> String {
    // SAFETY: the caller's contract.
    unsafe { string_from_ns(msg_send![error, localizedDescription]) }.unwrap_or_default()
}

impl RumbleMotor {
    fn lock(&self) -> MutexGuard<'_, MotorState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `-[SDL3_RumbleMotor cleanup]`.
    fn cleanup(&self) {
        autoreleasepool(|| {
            if macos_at_least(11, 0, 0) {
                let (player, engine) = {
                    let mut s = self.lock();
                    (std::mem::take(&mut s.player), std::mem::take(&mut s.engine))
                };
                // SAFETY: the motor's player and engine (or nil).
                unsafe {
                    if !player.is_nil() {
                        let _: Bool =
                            msg_send![player, cancelAndReturnError: std::ptr::null_mut::<Id>()];
                    }
                    if !engine.is_nil() {
                        let _: () = msg_send![engine, stopWithCompletionHandler: std::ptr::null_mut::<c_void>()];
                    }
                }
            }
        });
    }

    /// `-[SDL3_RumbleMotor setIntensity:]`.
    fn set_intensity(&self, intensity: f32) -> Result<()> {
        autoreleasepool(|| {
            if macos_at_least(11, 0, 0) {
                let mut s = self.lock();
                let mut error: Id = NIL;

                if s.engine.is_nil() {
                    return Err(Error::new("Haptics engine was stopped"));
                }

                // SAFETY: the motor's engine and player, and CoreHaptics'
                // classes with their initializers' argument types.
                unsafe {
                    if intensity == 0.0 {
                        if !s.player.is_nil() && s.active {
                            let _: Bool = msg_send![s.player, stopAtTime: 0.0 as NSTimeInterval error: &mut error as *mut Id];
                        }
                        s.active = false;
                        return Ok(());
                    }

                    if s.player.is_nil() {
                        let event_param: Id = msg_send![class!(CHHapticEventParameter), alloc];
                        let event_param = StrongPtr::from_owned(msg_send![event_param,
                            initWithParameterID: ch_haptic_event_parameter_id_haptic_intensity()
                            value: 1.0f32]);
                        let params: Id =
                            msg_send![class!(NSArray), arrayWithObject: event_param.get()];
                        let event: Id = msg_send![class!(CHHapticEvent), alloc];
                        let event = StrongPtr::from_owned(msg_send![event,
                            initWithEventType: ch_haptic_event_type_haptic_continuous()
                            parameters: params
                            relativeTime: 0.0 as NSTimeInterval
                            duration: gc_haptic_duration_infinite() as NSTimeInterval]);
                        let events: Id = msg_send![class!(NSArray), arrayWithObject: event.get()];
                        let empty: Id = msg_send![class!(NSArray), alloc];
                        let empty = StrongPtr::from_owned(msg_send![empty, init]);
                        let pattern: Id = msg_send![class!(CHHapticPattern), alloc];
                        let pattern = StrongPtr::from_owned(msg_send![pattern,
                            initWithEvents: events
                            parameters: empty.get()
                            error: &mut error as *mut Id]);
                        if !error.is_null() {
                            return Err(Error::new(format!(
                                "Couldn't create haptic pattern: {}",
                                error_description(error)
                            )));
                        }

                        let player: Id = msg_send![s.engine, createPlayerWithPattern: pattern.get() error: &mut error as *mut Id];
                        s.player = StrongPtr::retain(player);
                        if !error.is_null() {
                            return Err(Error::new(format!(
                                "Couldn't create haptic player: {}",
                                error_description(error)
                            )));
                        }
                        s.active = false;
                    }

                    let param: Id = msg_send![class!(CHHapticDynamicParameter), alloc];
                    let param = StrongPtr::from_owned(msg_send![param,
                        initWithParameterID: ch_haptic_dynamic_parameter_id_haptic_intensity_control()
                        value: intensity
                        relativeTime: 0.0 as NSTimeInterval]);
                    let params: Id = msg_send![class!(NSArray), arrayWithObject: param.get()];
                    let _: Bool = msg_send![s.player, sendParameters: params atTime: 0.0 as NSTimeInterval error: &mut error as *mut Id];
                    if !error.is_null() {
                        return Err(Error::new(format!(
                            "Couldn't update haptic player: {}",
                            error_description(error)
                        )));
                    }

                    if !s.active {
                        let _: Bool = msg_send![s.player, startAtTime: 0.0 as NSTimeInterval error: &mut error as *mut Id];
                        s.active = true;
                    }
                }
            }

            Ok(())
        })
    }

    /// `-[SDL3_RumbleMotor initWithController:locality:]`.
    ///
    /// # Safety
    ///
    /// `controller` must be a valid `GCController` with haptics.
    unsafe fn new(controller: Id, locality: Id) -> Result<RumbleMotor> {
        autoreleasepool(|| {
            let motor = RumbleMotor {
                state: Arc::new(Mutex::new(MotorState::default())),
            };
            let weak_self: Weak<Mutex<MotorState>> = Arc::downgrade(&motor.state);

            // SAFETY: the caller's contract; the engine's methods.
            unsafe {
                let haptics: Id = msg_send![controller, haptics];
                let engine: Id = msg_send![haptics, createEngineWithLocality: locality];
                if engine.is_null() {
                    return Err(Error::new("Couldn't create haptics engine"));
                }
                let engine = StrongPtr::retain(engine);

                let mut error: Id = NIL;
                let _: Bool = msg_send![engine, startAndReturnError: &mut error as *mut Id];
                if !error.is_null() {
                    return Err(Error::new("Couldn't start haptics engine"));
                }

                let weak = weak_self.clone();
                let stopped = RcBlock::new1(move |_stopped_reason: NSInteger| {
                    let Some(this) = weak.upgrade() else {
                        return;
                    };
                    let (player, engine) = {
                        let mut s = this.lock().unwrap_or_else(|e| e.into_inner());
                        (std::mem::take(&mut s.player), std::mem::take(&mut s.engine))
                    };
                    // (released outside the lock)
                    drop((player, engine));
                });
                let _: () = msg_send![engine, setStoppedHandler: stopped.as_ptr()];
                let weak = weak_self;
                let reset = RcBlock::new0(move || {
                    let Some(this) = weak.upgrade() else {
                        return;
                    };
                    let (player, engine) = {
                        let mut s = this.lock().unwrap_or_else(|e| e.into_inner());
                        (std::mem::take(&mut s.player), s.engine.clone())
                    };
                    drop(player);
                    // (the motor's engine, or nil)
                    let _: Bool =
                        msg_send![engine, startAndReturnError: std::ptr::null_mut::<Id>()];
                });
                let _: () = msg_send![engine, setResetHandler: reset.as_ptr()];

                motor.lock().engine = engine;
            }

            Ok(motor)
        })
    }
}

/// Translation of `SDL3_RumbleContext` (a Rust structure, as
/// [`RumbleMotor`]).
struct RumbleContext {
    low_frequency_motor: RumbleMotor,
    high_frequency_motor: RumbleMotor,
    left_trigger_motor: Option<RumbleMotor>,
    right_trigger_motor: Option<RumbleMotor>,
}

/// Both results of `result &= [motor setIntensity:]` twice: the last error
/// set wins.
fn and_results(a: Result<()>, b: Result<()>) -> Result<()> {
    match (a, b) {
        (_, Err(e)) | (Err(e), Ok(())) => Err(e),
        (Ok(()), Ok(())) => Ok(()),
    }
}

impl RumbleContext {
    /// `-[SDL3_RumbleContext rumbleWithLowFrequency:andHighFrequency:]`.
    fn rumble(&self, low_frequency_rumble: u16, high_frequency_rumble: u16) -> Result<()> {
        let low = self
            .low_frequency_motor
            .set_intensity(f32::from(low_frequency_rumble) / 65535.0);
        let high = self
            .high_frequency_motor
            .set_intensity(f32::from(high_frequency_rumble) / 65535.0);
        and_results(low, high)
    }

    /// `-[SDL3_RumbleContext rumbleLeftTrigger:andRightTrigger:]`.
    fn rumble_triggers(&self, left_rumble: u16, right_rumble: u16) -> Result<()> {
        match (&self.left_trigger_motor, &self.right_trigger_motor) {
            (Some(left), Some(right)) => {
                // Note (upstream): `result` starts false, so `result &=` keeps
                // it false and trigger rumble always reports failure (with
                // whatever error was set last) after applying the intensities.
                // Here it starts true, as in rumbleWithLowFrequency:.
                let l = left.set_intensity(f32::from(left_rumble) / 65535.0);
                let r = right.set_intensity(f32::from(right_rumble) / 65535.0);
                and_results(l, r)
            }
            _ => Err(Error::unsupported()),
        }
    }

    /// `-[SDL3_RumbleContext cleanup]` (the trigger motors are left to be
    /// released, as upstream).
    fn cleanup(&self) {
        self.low_frequency_motor.cleanup();
        self.high_frequency_motor.cleanup();
    }
}

/// Translation of `IOS_JoystickInitRumble()`.
///
/// # Safety
///
/// `controller` must be a valid `GCController` with haptics.
unsafe fn init_rumble(controller: Id) -> Option<RumbleContext> {
    autoreleasepool(|| {
        if macos_at_least(11, 0, 0) {
            // SAFETY: the caller's contract.
            unsafe {
                let low_frequency_motor =
                    RumbleMotor::new(controller, gc_haptics_locality_left_handle());
                let high_frequency_motor =
                    RumbleMotor::new(controller, gc_haptics_locality_right_handle());
                let left_trigger_motor =
                    RumbleMotor::new(controller, gc_haptics_locality_left_trigger());
                let right_trigger_motor =
                    RumbleMotor::new(controller, gc_haptics_locality_right_trigger());
                if let (Ok(low_frequency_motor), Ok(high_frequency_motor)) =
                    (low_frequency_motor, high_frequency_motor)
                {
                    return Some(RumbleContext {
                        low_frequency_motor,
                        high_frequency_motor,
                        left_trigger_motor: left_trigger_motor.ok(),
                        right_trigger_motor: right_trigger_motor.ok(),
                    });
                }
            }
        }
        None
    })
}

/// The device's rumble context, made on first use (the start of
/// `IOS_JoystickRumble()`/`IOS_JoystickRumbleTriggers()`); taken out of the
/// state to drive it unlocked.
fn device_rumble(joystick: JoystickID) -> Result<Option<Arc<RumbleContext>>> {
    let controller = {
        let mut state = lock_state();
        let Some(device) = state.hwdata(joystick) else {
            return Err(Error::new("Controller is no longer connected"));
        };
        if let Some(rumble) = &device.rumble {
            return Ok(Some(rumble.clone()));
        }
        device.controller.clone()
    };

    if macos_at_least(11, 0, 0) && !controller.is_nil() {
        // SAFETY: the device's controller.
        let haptics: Id = unsafe { msg_send![controller, haptics] };
        if !haptics.is_null() {
            // SAFETY: a controller with haptics.
            if let Some(rumble) = unsafe { init_rumble(controller.get()) } {
                let rumble = Arc::new(rumble);
                if let Some(device) = lock_state().hwdata(joystick) {
                    device.rumble = Some(rumble.clone());
                }
                return Ok(Some(rumble));
            }
        }
    }
    Ok(None)
}

/// Translation of `IOS_SupportedHIDDevice()`: whether GameController
/// handles a HID device (the IOKit driver leaves those to this one).
///
/// # Safety
///
/// `device` must be a valid HID device.
pub(crate) unsafe fn supported_hid_device(device: IOHIDDeviceRef) -> bool {
    if !hints::get_bool(hints::JOYSTICK_MFI, true) {
        return false;
    }

    let key = crate::core::apple::foundation::cf_string(K_IOHID_VIRTUAL_HIDEVICE);
    // SAFETY: the caller's contract.
    if unsafe { IOHIDDeviceGetProperty(device, key.get()) } == unsafe { kCFBooleanTrue } {
        // Steam virtual gamepads always have kIOHIDVirtualHIDevice property unlike real devices, and are also not supported as GCController
        return false;
    }

    if macos_at_least(11, 0, 0) && load_frameworks() {
        const MAX_ATTEMPTS: i32 = 3;
        for _ in 0..MAX_ATTEMPTS {
            // SAFETY: GCController's class method taking the device.
            let supported: Bool =
                unsafe { msg_send![class!(GCController), supportsHIDDevice: device] };
            if supported.is_true() {
                return true;
            }

            // The framework may not have seen the device yet
            crate::timer::delay(std::time::Duration::from_millis(10));
        }
    }
    false
}

/// Translation of `GetAppleSFSymbolsNameForElement()`: the element's
/// `sfSymbolsName` as ASCII (`getCString:maxLength:255`), empty when it
/// has none or it doesn't fit.
///
/// # Safety
///
/// `element` must be nil or a valid `GCControllerElement`.
unsafe fn sf_symbols_name_for_element(element: Id) -> String {
    let mut name = [0u8; 256];
    if macos_at_least(11, 0, 0) && !element.is_null() {
        // SAFETY: the caller's contract; the buffer holds 255 bytes and a
        // terminator.
        unsafe {
            let symbol: Id = msg_send![element, sfSymbolsName];
            let _: Bool = msg_send![symbol,
                getCString: name.as_mut_ptr() as *mut c_char
                maxLength: 255 as NSUInteger
                encoding: NS_ASCII_STRING_ENCODING];
        }
    }
    let len = name.iter().position(|&b| b == 0).unwrap_or(0);
    String::from_utf8_lossy(&name[..len]).into_owned()
}

/// Translation of `GetDirectionalPadForController()`.
///
/// # Safety
///
/// `controller` must be a valid `GCController`.
unsafe fn directional_pad_for_controller(controller: Id) -> Id {
    // SAFETY: the caller's contract.
    unsafe {
        if macos_at_least(11, 0, 0) {
            let profile: Id = msg_send![controller, physicalInputProfile];
            let dpads: Id = msg_send![profile, dpads];
            return msg_send![dpads, objectForKey: gc_input_direction_pad()];
        }

        let extended: Id = msg_send![controller, extendedGamepad];
        if !extended.is_null() {
            return msg_send![extended, dpad];
        }

        let micro: Id = msg_send![controller, microGamepad];
        if !micro.is_null() {
            return msg_send![micro, dpad];
        }
    }
    NIL
}

/// The controller of an open joystick of this driver (`gamepad &&
/// SDL_GetGamepadJoystick(gamepad)->driver == &SDL_IOS_JoystickDriver`).
fn joystick_controller(joystick: JoystickID) -> Option<StrongPtr> {
    if super::joystick_driver_index(joystick) != Some(super::MFI_DRIVER_INDEX) {
        return None;
    }
    lock_state().hwdata(joystick).map(|d| d.controller.clone())
}

/// Translation of `IOS_GetAppleSFSymbolsNameForButton()`.
pub(crate) fn apple_sf_symbols_name_for_button(
    joystick: JoystickID,
    button: GamepadButton,
) -> Option<String> {
    let controller = joystick_controller(joystick)?;
    if !macos_at_least(11, 0, 0) {
        return None;
    }
    let element_name = autoreleasepool(|| {
        // SAFETY: the joystick's controller and its profile's elements.
        unsafe {
            let controller = controller.get();
            let elements = profile_elements(controller);
            let element = |key: Id| -> Id { msg_send![elements, objectForKey: key] };
            let element_named = |name: &str| dict_get(elements, name);
            let dpad_direction = |direction: fn(Id) -> Id, fallback: &str| -> String {
                let dpad = directional_pad_for_controller(controller);
                if dpad.is_null() {
                    return String::new();
                }
                let mut name = sf_symbols_name_for_element(direction(dpad));
                if name.is_empty() {
                    name = fallback.to_owned();
                }
                name
            };
            match button {
                GamepadButton::South => sf_symbols_name_for_element(element(gc_input_button_a())),
                GamepadButton::East => sf_symbols_name_for_element(element(gc_input_button_b())),
                GamepadButton::West => sf_symbols_name_for_element(element(gc_input_button_x())),
                GamepadButton::North => sf_symbols_name_for_element(element(gc_input_button_y())),
                GamepadButton::Back => {
                    sf_symbols_name_for_element(element(gc_input_button_options()))
                }
                GamepadButton::Guide => sf_symbols_name_for_element(element_named("Button Home")),
                GamepadButton::Start => {
                    sf_symbols_name_for_element(element(gc_input_button_menu()))
                }
                GamepadButton::LeftStick => {
                    sf_symbols_name_for_element(element(gc_input_left_thumbstick_button()))
                }
                GamepadButton::RightStick => {
                    sf_symbols_name_for_element(element(gc_input_right_thumbstick_button()))
                }
                GamepadButton::LeftShoulder => {
                    sf_symbols_name_for_element(element(gc_input_left_shoulder()))
                }
                GamepadButton::RightShoulder => {
                    sf_symbols_name_for_element(element(gc_input_right_shoulder()))
                }
                GamepadButton::DpadUp => dpad_direction(|d| msg_send![d, up], "dpad.up.fill"),
                GamepadButton::DpadDown => dpad_direction(|d| msg_send![d, down], "dpad.down.fill"),
                GamepadButton::DpadLeft => dpad_direction(|d| msg_send![d, left], "dpad.left.fill"),
                GamepadButton::DpadRight => {
                    dpad_direction(|d| msg_send![d, right], "dpad.right.fill")
                }
                GamepadButton::Misc1 => {
                    sf_symbols_name_for_element(element(gc_input_dual_shock_touchpad_button()))
                }
                GamepadButton::RightPaddle1 => {
                    sf_symbols_name_for_element(element(gc_input_xbox_paddle_one()))
                }
                GamepadButton::LeftPaddle1 => {
                    sf_symbols_name_for_element(element(gc_input_xbox_paddle_three()))
                }
                GamepadButton::RightPaddle2 => {
                    sf_symbols_name_for_element(element(gc_input_xbox_paddle_two()))
                }
                GamepadButton::LeftPaddle2 => {
                    sf_symbols_name_for_element(element(gc_input_xbox_paddle_four()))
                }
                GamepadButton::Touchpad => {
                    sf_symbols_name_for_element(element(gc_input_dual_shock_touchpad_button()))
                }
                _ => String::new(),
            }
        }
    });
    (!element_name.is_empty()).then_some(element_name)
}

/// Translation of `IOS_GetAppleSFSymbolsNameForAxis()`.
pub(crate) fn apple_sf_symbols_name_for_axis(
    joystick: JoystickID,
    axis: GamepadAxis,
) -> Option<String> {
    let controller = joystick_controller(joystick)?;
    if !macos_at_least(11, 0, 0) {
        return None;
    }
    let element_name = autoreleasepool(|| {
        // SAFETY: the joystick's controller and its profile's elements.
        unsafe {
            let elements = profile_elements(controller.get());
            let element = |key: Id| -> Id { msg_send![elements, objectForKey: key] };
            match axis {
                GamepadAxis::LeftX | GamepadAxis::LeftY => {
                    sf_symbols_name_for_element(element(gc_input_left_thumbstick()))
                }
                GamepadAxis::RightX | GamepadAxis::RightY => {
                    sf_symbols_name_for_element(element(gc_input_right_thumbstick()))
                }
                GamepadAxis::LeftTrigger => {
                    sf_symbols_name_for_element(element(gc_input_left_trigger()))
                }
                GamepadAxis::RightTrigger => {
                    sf_symbols_name_for_element(element(gc_input_right_trigger()))
                }
                _ => String::new(),
            }
        }
    });
    (!element_name.is_empty()).then_some(element_name)
}

/// The mapping `IOS_JoystickGetGamepadMapping()` makes from the device's
/// element names (macOS 11+).
pub(crate) fn gamepad_mapping_for(device: &MfiDevice, names: &InputNames) -> GamepadMapping {
    let mut out = GamepadMapping::default();

    for (axis, key) in device.axes.iter().enumerate() {
        let key = key.name.as_str();
        let axis = axis as u8;
        if key == "Left Thumbstick X Axis" || key == "Direction Pad X Axis" {
            out.leftx.kind = MappingKind::Axis;
            out.leftx.target = axis;
        } else if key == "Left Thumbstick Y Axis" || key == "Direction Pad Y Axis" {
            out.lefty.kind = MappingKind::Axis;
            out.lefty.target = axis;
            out.lefty.axis_reversed = true;
        } else if key == "Right Thumbstick X Axis" {
            out.rightx.kind = MappingKind::Axis;
            out.rightx.target = axis;
        } else if key == "Right Thumbstick Y Axis" {
            out.righty.kind = MappingKind::Axis;
            out.righty.target = axis;
            out.righty.axis_reversed = true;
        } else if key == names.left_trigger {
            out.lefttrigger.kind = MappingKind::Axis;
            out.lefttrigger.target = axis;
            out.lefttrigger.half_axis_positive = true;
        } else if key == names.right_trigger {
            out.righttrigger.kind = MappingKind::Axis;
            out.righttrigger.target = axis;
            out.righttrigger.half_axis_positive = true;
        }
    }

    for (button, key) in device.buttons.iter().enumerate() {
        let key = key.name.as_str();
        let mapping: Option<&mut InputMapping> = if key == names.button_a {
            if device.is_siri_remote > 1 {
                // GCInputButtonA is triggered for any D-Pad press, ignore it in favor of "Button Center"
                None
            } else if device.has_nintendo_buttons {
                Some(&mut out.b)
            } else {
                Some(&mut out.a)
            }
        } else if key == names.button_b {
            if device.has_nintendo_buttons {
                Some(&mut out.a)
            } else if device.is_switch_joycon_l || device.is_switch_joycon_r {
                Some(&mut out.x)
            } else {
                Some(&mut out.b)
            }
        } else if key == names.button_x {
            if device.has_nintendo_buttons {
                Some(&mut out.y)
            } else if device.is_switch_joycon_l || device.is_switch_joycon_r {
                Some(&mut out.b)
            } else {
                Some(&mut out.x)
            }
        } else if key == names.button_y {
            if device.has_nintendo_buttons {
                Some(&mut out.x)
            } else {
                Some(&mut out.y)
            }
        } else if key == "Direction Pad Left" {
            Some(&mut out.dpleft)
        } else if key == "Direction Pad Right" {
            Some(&mut out.dpright)
        } else if key == "Direction Pad Up" {
            Some(&mut out.dpup)
        } else if key == "Direction Pad Down" {
            Some(&mut out.dpdown)
        } else if key == "Cardinal Direction Pad Left" {
            Some(&mut out.dpleft)
        } else if key == "Cardinal Direction Pad Right" {
            Some(&mut out.dpright)
        } else if key == "Cardinal Direction Pad Up" {
            Some(&mut out.dpup)
        } else if key == "Cardinal Direction Pad Down" {
            Some(&mut out.dpdown)
        } else if key == names.left_shoulder {
            Some(&mut out.leftshoulder)
        } else if key == names.right_shoulder {
            Some(&mut out.rightshoulder)
        } else if key == names.left_thumbstick_button {
            Some(&mut out.leftstick)
        } else if key == names.right_thumbstick_button {
            Some(&mut out.rightstick)
        } else if key == "Button Home" {
            Some(&mut out.guide)
        } else if key == names.button_menu {
            if device.is_siri_remote != 0 {
                Some(&mut out.b)
            } else {
                Some(&mut out.start)
            }
        } else if key == names.button_options {
            Some(&mut out.back)
        } else if key == "Button Share" {
            Some(&mut out.misc1)
        } else if key == names.xbox_paddle_one {
            Some(&mut out.right_paddle1)
        } else if key == names.xbox_paddle_two {
            Some(&mut out.right_paddle2)
        } else if key == names.xbox_paddle_three {
            Some(&mut out.left_paddle1)
        } else if key == names.xbox_paddle_four {
            Some(&mut out.left_paddle2)
        } else if key == names.left_trigger {
            Some(&mut out.lefttrigger)
        } else if key == names.right_trigger {
            Some(&mut out.righttrigger)
        } else if key == names.dual_shock_touchpad_button {
            Some(&mut out.touchpad)
        } else if key == "Button Center" {
            Some(&mut out.a)
        } else {
            None
        };
        if let Some(mapping) = mapping {
            if mapping.kind == MappingKind::None {
                mapping.kind = MappingKind::Button;
                mapping.target = button as u8;
            }
        }
    }

    out
}

/// Translation of `SDL_IOS_JoystickDriver`.
pub(crate) struct MfiJoystickDriver;

pub(crate) static MFI_JOYSTICK_DRIVER: MfiJoystickDriver = MfiJoystickDriver;

impl JoystickDriver for MfiJoystickDriver {
    /// Translation of `IOS_JoystickInit()`.
    fn init(&self) -> Result<()> {
        if !hints::get_bool(hints::JOYSTICK_MFI, true) {
            return Ok(());
        }

        if !macos_at_least(11, 0, 0) {
            // Continue with initialization on macOS 11+
            return Ok(());
        }

        autoreleasepool(|| {
            // GameController.framework was added in iOS 7.
            // (and is loaded here, see sys)
            if !load_frameworks() || class!(GCController).is_null() {
                return;
            }

            // SAFETY: GCController's class methods and the notification
            // center's, with blocks taking the notification.
            unsafe {
                /* For whatever reason, this always returns an empty array on
                macOS 11.0.1 */
                let controllers: Id = msg_send![class!(GCController), controllers];
                for controller in ns_array_items(controllers) {
                    add_joystick_device(controller);
                }

                let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];

                let block = connect_block();
                let observer: Id = msg_send![center,
                    addObserverForName: gc_controller_did_connect_notification()
                    object: NIL
                    queue: NIL
                    usingBlock: block.as_ptr()];
                lock_state().connect_observer = StrongPtr::retain(observer);

                let block = disconnect_block();
                let observer: Id = msg_send![center,
                    addObserverForName: gc_controller_did_disconnect_notification()
                    object: NIL
                    queue: NIL
                    usingBlock: block.as_ptr()];
                lock_state().disconnect_observer = StrongPtr::retain(observer);
            }
        });

        Ok(())
    }

    /// Translation of `IOS_JoystickGetCount()`.
    fn count(&self) -> usize {
        lock_state().devices.len()
    }

    /// Translation of `IOS_JoystickDetect()`.
    fn detect(&self) {}

    /// Translation of `IOS_JoystickIsDevicePresent()`.
    fn is_device_present(
        &self,
        _vendor_id: u16,
        _product_id: u16,
        _version: u16,
        _name: Option<&str>,
    ) -> bool {
        // We don't override any other drivers through this method
        false
    }

    /// Translation of `IOS_JoystickGetDeviceName()`.
    fn device_name(&self, device_index: usize) -> Option<String> {
        Some(
            lock_state()
                .device_for_index(device_index)
                .map_or_else(|| "Unknown".to_owned(), |d| d.name.clone()),
        )
    }

    /// Translation of `IOS_JoystickGetDevicePath()`.
    fn device_path(&self, _device_index: usize) -> Option<String> {
        None
    }

    /// Translation of `IOS_JoystickGetDeviceSteamVirtualGamepadSlot()`.
    fn device_steam_virtual_gamepad_slot(&self, _device_index: usize) -> i32 {
        -1
    }

    /// Translation of `IOS_JoystickGetDevicePlayerIndex()`.
    fn device_player_index(&self, device_index: usize) -> i32 {
        let controller = lock_state()
            .device_for_index(device_index)
            .map(|d| d.controller.clone());
        match controller {
            Some(controller) if !controller.is_nil() => {
                // SAFETY: the device's controller.
                let index: NSInteger = unsafe { msg_send![controller, playerIndex] };
                index as i32
            }
            _ => -1,
        }
    }

    /// Translation of `IOS_JoystickSetDevicePlayerIndex()`.
    fn set_device_player_index(&self, device_index: usize, player_index: i32) {
        let controller = lock_state()
            .device_for_index(device_index)
            .map(|d| d.controller.clone());
        if let Some(controller) = controller.filter(|c| !c.is_nil()) {
            // SAFETY: the device's controller.
            let _: () = unsafe { msg_send![controller, setPlayerIndex: player_index as NSInteger] };
        }
    }

    /// Translation of `IOS_JoystickGetDeviceGUID()`.
    fn device_guid(&self, device_index: usize) -> Guid {
        lock_state()
            .device_for_index(device_index)
            .map_or(Guid::ZERO, |d| d.guid)
    }

    /// Translation of `IOS_JoystickGetDeviceInstanceID()`.
    fn device_instance_id(&self, device_index: usize) -> JoystickID {
        lock_state()
            .device_for_index(device_index)
            .map_or(0, |d| d.instance_id)
    }

    /// Translation of `IOS_JoystickOpen()`.
    fn open(&self, joystick: &mut JoystickData, device_index: usize) -> Result<()> {
        let (controller, pause_button_index, is_siri_remote) = {
            let mut state = lock_state();
            let Some(device) = state.device_for_index(device_index) else {
                return Err(Error::new(
                    "Could not open Joystick: no hardware device for the specified index",
                ));
            };

            joystick.naxes = device.naxes;
            joystick.nhats = device.nhats;
            joystick.nbuttons = device.nbuttons;

            if device.has_dualshock_touchpad {
                joystick.add_touchpad(2);
            }

            device.joystick = Some(joystick.instance_id);
            (
                device.controller.clone(),
                device.pause_button_index,
                device.is_siri_remote,
            )
        };

        let mut caps = Vec::new();
        autoreleasepool(|| {
            let controller = controller.get();
            // SAFETY: the device's controller and its (macOS 11+) properties.
            unsafe {
                if pause_button_index >= 0 {
                    let instance_id = joystick.instance_id;
                    let handler = RcBlock::new1(move |_c: Id| {
                        if let Some(device) = lock_state().hwdata(instance_id) {
                            device.pause_button_pressed = crate::timer::ticks_ms();
                        }
                    });
                    let _: () = msg_send![controller, setControllerPausedHandler: handler.as_ptr()];
                }

                if macos_at_least(11, 0, 0) {
                    let motion: Id = msg_send![controller, motion];
                    let has_rotation_rate: Bool = msg_send![motion, hasRotationRate];
                    if !motion.is_null() && has_rotation_rate.is_true() {
                        joystick.add_sensor(SensorType::Gyro, 0.0);
                        joystick.add_sensor(SensorType::Accel, 0.0);
                    }
                }

                if macos_at_least(11, 0, 0) {
                    set_system_gesture_state(controller, GC_SYSTEM_GESTURE_STATE_DISABLED);
                }

                if macos_at_least(11, 0, 0) {
                    let light: Id = msg_send![controller, light];
                    if !light.is_null() {
                        caps.push(PROP_JOYSTICK_CAP_RGB_LED_BOOLEAN);
                    }

                    let haptics: Id = msg_send![controller, haptics];
                    if !haptics.is_null() {
                        let localities: Id = msg_send![haptics, supportedLocalities];
                        let localities: Id = msg_send![localities, allObjects];
                        let handles = constant_str(gc_haptics_locality_handles());
                        let triggers = constant_str(gc_haptics_locality_triggers());
                        for locality in ns_array_items(localities) {
                            if ns_equals(locality, &handles) {
                                caps.push(PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN);
                            } else if ns_equals(locality, &triggers) {
                                caps.push(PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN);
                            }
                        }
                    }
                }
            }
        });
        let props = joystick.properties();
        for cap in caps {
            let _ = props.set(cap, true);
        }
        if is_siri_remote != 0 {
            APPLE_TV_REMOTE_OPENED_AS_JOYSTICK.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        Ok(())
    }

    /// Translation of `IOS_JoystickRumble()`.
    fn rumble(
        &self,
        joystick: JoystickID,
        low_frequency_rumble: u16,
        high_frequency_rumble: u16,
    ) -> Result<()> {
        match device_rumble(joystick)? {
            Some(rumble) => rumble.rumble(low_frequency_rumble, high_frequency_rumble),
            None => Err(Error::unsupported()),
        }
    }

    /// Translation of `IOS_JoystickRumbleTriggers()`.
    fn rumble_triggers(
        &self,
        joystick: JoystickID,
        left_rumble: u16,
        right_rumble: u16,
    ) -> Result<()> {
        match device_rumble(joystick)? {
            Some(rumble) => rumble.rumble_triggers(left_rumble, right_rumble),
            None => Err(Error::unsupported()),
        }
    }

    /// Translation of `IOS_JoystickSetLED()`.
    fn set_led(&self, joystick: JoystickID, red: u8, green: u8, blue: u8) -> Result<()> {
        autoreleasepool(|| {
            let Some(controller) = lock_state().hwdata(joystick).map(|d| d.controller.clone())
            else {
                return Err(Error::new("Controller is no longer connected"));
            };

            if macos_at_least(11, 0, 0) {
                // SAFETY: the device's controller, its light and GCColor.
                unsafe {
                    let light: Id = msg_send![controller, light];
                    if !light.is_null() {
                        let color: Id = msg_send![class!(GCColor), alloc];
                        let color = StrongPtr::from_owned(msg_send![color,
                            initWithRed: f32::from(red) / 255.0
                            green: f32::from(green) / 255.0
                            blue: f32::from(blue) / 255.0]);
                        let _: () = msg_send![light, setColor: color.get()];
                        return Ok(());
                    }
                }
            }
            Err(Error::unsupported())
        })
    }

    /// Translation of `IOS_JoystickSendEffect()`.
    fn send_effect(&self, _joystick: JoystickID, _data: &[u8]) -> Result<()> {
        Err(Error::unsupported())
    }

    /// Translation of `IOS_JoystickSetSensorsEnabled()`.
    fn set_sensors_enabled(&self, joystick: JoystickID, enabled: bool) -> Result<()> {
        autoreleasepool(|| {
            let Some(controller) = lock_state().hwdata(joystick).map(|d| d.controller.clone())
            else {
                return Err(Error::new("Controller is no longer connected"));
            };

            if macos_at_least(11, 0, 0) {
                // SAFETY: the device's controller and its motion.
                unsafe {
                    let motion: Id = msg_send![controller, motion];
                    if !motion.is_null() {
                        let _: () =
                            msg_send![motion, setSensorsActive: if enabled { YES } else { NO }];
                        return Ok(());
                    }
                }
            }
            Err(Error::unsupported())
        })
    }

    /// Translation of `IOS_JoystickUpdate()` (and `IOS_MFIJoystickUpdate()`).
    fn update(&self, joystick: JoystickID) {
        let timestamp = crate::timer::ticks_ns();
        let inputs = {
            let mut state = lock_state();
            let Some(device) = state.hwdata(joystick) else {
                return;
            };

            if device.controller.is_nil() {
                return;
            }
            mfi_joystick_update(device, crate::timer::ticks_ms())
        };

        // (sent with the driver state unlocked: event watchers may call
        // back into the joystick API)
        for input in inputs {
            match input {
                Input::Axis(i, value) => send_joystick_axis(timestamp, joystick, i, value),
                Input::Button(i, down) => send_joystick_button(timestamp, joystick, i, down),
                Input::Hat(i, value) => send_joystick_hat(timestamp, joystick, i, value),
                Input::Touchpad(finger, down, x, y) => {
                    send_joystick_touchpad(timestamp, joystick, 0, finger, down, x, y, 1.0)
                }
                Input::Sensor(sensor_type, data) => {
                    send_joystick_sensor(timestamp, joystick, sensor_type, timestamp, &data)
                }
                Input::Power(state, percent) => send_joystick_power_info(joystick, state, percent),
            }
        }
    }

    /// Translation of `IOS_JoystickClose()`.
    fn close(&self, joystick: &mut JoystickData) {
        let (rumble, controller, is_siri_remote) = {
            let mut state = lock_state();
            let Some(device) = state.hwdata(joystick.instance_id) else {
                return;
            };

            device.joystick = None;
            (
                device.rumble.take(),
                device.controller.clone(),
                device.is_siri_remote,
            )
        };

        autoreleasepool(|| {
            if let Some(rumble) = rumble {
                rumble.cleanup();
            }

            if !controller.is_nil() {
                // SAFETY: the device's controller.
                unsafe {
                    let _: () = msg_send![controller, setControllerPausedHandler: NIL];
                    let _: () = msg_send![controller, setPlayerIndex: -1 as NSInteger];

                    if macos_at_least(11, 0, 0) {
                        set_system_gesture_state(controller.get(), GC_SYSTEM_GESTURE_STATE_ENABLED);
                    }
                }
            }
        });

        if is_siri_remote != 0 {
            APPLE_TV_REMOTE_OPENED_AS_JOYSTICK.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Translation of `IOS_JoystickQuit()`.
    fn quit(&self) {
        autoreleasepool(|| {
            let (connect_observer, disconnect_observer) = {
                let mut state = lock_state();
                (
                    std::mem::take(&mut state.connect_observer),
                    std::mem::take(&mut state.disconnect_observer),
                )
            };
            // SAFETY: the notification center and the observers it returned.
            unsafe {
                let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];

                if !connect_observer.is_nil() {
                    let _: () = msg_send![center,
                        removeObserver: connect_observer.get()
                        name: gc_controller_did_connect_notification()
                        object: NIL];
                }

                if !disconnect_observer.is_nil() {
                    let _: () = msg_send![center,
                        removeObserver: disconnect_observer.get()
                        name: gc_controller_did_disconnect_notification()
                        object: NIL];
                }
            }

            while !lock_state().devices.is_empty() {
                remove_joystick_device(0);
            }
        });
    }

    /// Translation of `IOS_JoystickGetGamepadMapping()`.
    fn gamepad_mapping(&self, device_index: usize) -> Option<GamepadMapping> {
        let mut state = lock_state();
        let device = state.device_for_index(device_index)?;

        if macos_at_least(11, 0, 0) {
            return Some(gamepad_mapping_for(device, input_names()));
        }
        None
    }
}
