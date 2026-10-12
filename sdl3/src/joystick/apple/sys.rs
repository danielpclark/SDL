// The GameController and CoreHaptics declarations the MFi joystick driver
// uses (from <GameController/GameController.h> and
// <CoreHaptics/CoreHaptics.h>).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Upstream links GameController and CoreHaptics; CoreHaptics only exists
//! from macOS 10.15 (and the driver only runs from macOS 11), while the
//! x86-64 build here may start on older systems. So both frameworks are
//! loaded when the driver initializes (`[GCController class]` being nil
//! without them, as upstream checks), and their exported constants are
//! read through the loader. The classes are looked up by name once loaded.

use std::sync::OnceLock;

use crate::core::apple::objc::{Id, NSInteger};
use crate::loadso::SharedObject;

/// `GCSystemGestureStateEnabled`.
pub(crate) const GC_SYSTEM_GESTURE_STATE_ENABLED: NSInteger = 0;
/// `GCSystemGestureStateDisabled`.
pub(crate) const GC_SYSTEM_GESTURE_STATE_DISABLED: NSInteger = 2;

/// `GCDeviceBatteryStateDischarging`.
pub(crate) const GC_DEVICE_BATTERY_STATE_DISCHARGING: NSInteger = 0;
/// `GCDeviceBatteryStateCharging`.
pub(crate) const GC_DEVICE_BATTERY_STATE_CHARGING: NSInteger = 1;
/// `GCDeviceBatteryStateFull`.
pub(crate) const GC_DEVICE_BATTERY_STATE_FULL: NSInteger = 2;

/// `GCRotationRate`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GCRotationRate {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) z: f64,
}

/// `GCAcceleration`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GCAcceleration {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) z: f64,
}

/// The loaded frameworks, kept for the life of the process (upstream links
/// them).
struct Frameworks {
    game_controller: SharedObject,
    #[allow(dead_code)] // (kept loaded: its classes are looked up by name)
    core_haptics: SharedObject,
}

const GAME_CONTROLLER_PATH: &str =
    "/System/Library/Frameworks/GameController.framework/GameController";
const CORE_HAPTICS_PATH: &str = "/System/Library/Frameworks/CoreHaptics.framework/CoreHaptics";

fn frameworks() -> Option<&'static Frameworks> {
    static FRAMEWORKS: OnceLock<Option<Frameworks>> = OnceLock::new();
    FRAMEWORKS
        .get_or_init(|| {
            Some(Frameworks {
                game_controller: SharedObject::load(GAME_CONTROLLER_PATH).ok()?,
                core_haptics: SharedObject::load(CORE_HAPTICS_PATH).ok()?,
            })
        })
        .as_ref()
}

/// Load GameController and CoreHaptics; whether both are there (upstream's
/// build only enables MFi with both, "to ensure rumble works").
pub(crate) fn load_frameworks() -> bool {
    frameworks().is_some()
}

/// The value of an exported `NSString *const` of a framework, nil when the
/// framework or the symbol is missing.
fn string_constant(lib: fn(&Frameworks) -> &SharedObject, name: &str) -> Id {
    let Some(f) = frameworks() else {
        return std::ptr::null_mut();
    };
    match lib(f).symbol(name) {
        // SAFETY: the symbol is an `NSString *const` the framework exports.
        Ok(p) => unsafe { *(p.as_ptr() as *const Id) },
        Err(_) => std::ptr::null_mut(),
    }
}

/// `GCHapticDurationInfinite` (a `const float`), 0 when missing.
pub(crate) fn gc_haptic_duration_infinite() -> f32 {
    let Some(f) = frameworks() else {
        return 0.0;
    };
    match f.game_controller.symbol("GCHapticDurationInfinite") {
        // SAFETY: the symbol is a `const float`.
        Ok(p) => unsafe { *(p.as_ptr() as *const f32) },
        Err(_) => 0.0,
    }
}

macro_rules! gc_strings {
    ($($fn_name:ident => $sym:literal,)*) => {
        $(
            #[doc = concat!("`", $sym, "`.")]
            pub(crate) fn $fn_name() -> Id {
                string_constant(|f| &f.game_controller, $sym)
            }
        )*
    };
}

macro_rules! ch_strings {
    ($($fn_name:ident => $sym:literal,)*) => {
        $(
            #[doc = concat!("`", $sym, "`.")]
            pub(crate) fn $fn_name() -> Id {
                string_constant(|f| &f.core_haptics, $sym)
            }
        )*
    };
}

gc_strings! {
    gc_input_button_a => "GCInputButtonA",
    gc_input_button_b => "GCInputButtonB",
    gc_input_button_x => "GCInputButtonX",
    gc_input_button_y => "GCInputButtonY",
    gc_input_left_shoulder => "GCInputLeftShoulder",
    gc_input_right_shoulder => "GCInputRightShoulder",
    gc_input_left_trigger => "GCInputLeftTrigger",
    gc_input_right_trigger => "GCInputRightTrigger",
    gc_input_left_thumbstick => "GCInputLeftThumbstick",
    gc_input_right_thumbstick => "GCInputRightThumbstick",
    gc_input_left_thumbstick_button => "GCInputLeftThumbstickButton",
    gc_input_right_thumbstick_button => "GCInputRightThumbstickButton",
    gc_input_button_menu => "GCInputButtonMenu",
    gc_input_button_options => "GCInputButtonOptions",
    gc_input_direction_pad => "GCInputDirectionPad",
    gc_input_dual_shock_touchpad_button => "GCInputDualShockTouchpadButton",
    gc_input_dual_shock_touchpad_one => "GCInputDualShockTouchpadOne",
    gc_input_dual_shock_touchpad_two => "GCInputDualShockTouchpadTwo",
    gc_input_xbox_paddle_one => "GCInputXboxPaddleOne",
    gc_input_xbox_paddle_two => "GCInputXboxPaddleTwo",
    gc_input_xbox_paddle_three => "GCInputXboxPaddleThree",
    gc_input_xbox_paddle_four => "GCInputXboxPaddleFour",
    gc_controller_did_connect_notification => "GCControllerDidConnectNotification",
    gc_controller_did_disconnect_notification => "GCControllerDidDisconnectNotification",
    gc_haptics_locality_handles => "GCHapticsLocalityHandles",
    gc_haptics_locality_left_handle => "GCHapticsLocalityLeftHandle",
    gc_haptics_locality_right_handle => "GCHapticsLocalityRightHandle",
    gc_haptics_locality_triggers => "GCHapticsLocalityTriggers",
    gc_haptics_locality_left_trigger => "GCHapticsLocalityLeftTrigger",
    gc_haptics_locality_right_trigger => "GCHapticsLocalityRightTrigger",
}

ch_strings! {
    ch_haptic_event_parameter_id_haptic_intensity => "CHHapticEventParameterIDHapticIntensity",
    ch_haptic_event_type_haptic_continuous => "CHHapticEventTypeHapticContinuous",
    ch_haptic_dynamic_parameter_id_haptic_intensity_control => "CHHapticDynamicParameterIDHapticIntensityControl",
}

/// The UTF-8 contents of one of the constants above (empty when missing).
pub(crate) fn constant_str(s: Id) -> String {
    // SAFETY: nil or one of the framework's NSStrings.
    unsafe { crate::core::apple::foundation::string_from_ns(s) }.unwrap_or_default()
}
