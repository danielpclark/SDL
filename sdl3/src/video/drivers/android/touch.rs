// Rust translation of src/video/android/SDL_androidtouch.c and
// SDL_androidtouch.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The touch screens: the devices the activity lists (`initTouch()`: the
//! touch screens and the virtual devices), and the fingers of the
//! surface's touch events (`SDLSurface.onTouch()`'s `onNativeTouch()`).

use std::time::Duration;

use crate::events::touch::{self, TouchDeviceType, TouchID};
use crate::events::{EventType, WindowID};

/// `MotionEvent.ACTION_*`.
const ACTION_DOWN: i32 = 0;
const ACTION_UP: i32 = 1;
const ACTION_MOVE: i32 = 2;
const ACTION_CANCEL: i32 = 3;
// const ACTION_OUTSIDE: i32 = 4;
const ACTION_POINTER_DOWN: i32 = 5;
const ACTION_POINTER_UP: i32 = 6;

/// Add all touch devices. Translation of `Android_InitTouch()`.
pub(crate) fn init_touch() {
    // Add all touch devices
    crate::core::android::init_touch();
}

/// Translation of `Android_QuitTouch()`.
pub(crate) fn quit_touch() {}

/// A Java touch device ID as SDL's. Translation of
/// `Android_ConvertJavaTouchID()`.
pub(crate) fn convert_java_touch_id(touch_id: i32) -> TouchID {
    let mut retval = touch_id as i64;

    if touch_id < 0 {
        // Touch ID -1 appears when using Android emulator, eg:
        //  adb shell input mouse tap 100 100
        //  adb shell input touchscreen tap 100 100
        //
        // Prevent the values -1 and -2, since they're used in SDL internal for synthetic events:
        retval -= 2;
    } else {
        // Touch ID 0 is invalid
        retval += 1;
    }
    retval as TouchID
}

/// A finger of a touch event (`x` and `y` normalized, `p` the pressure).
/// Translation of `Android_OnTouch()`.
pub(crate) fn on_touch(
    window: Option<WindowID>,
    touch_device_id_in: i32,
    pointer_finger_id_in: i32,
    action: i32,
    x: f32,
    y: f32,
    p: f32,
) {
    let Some(window) = window else {
        return;
    };

    let touch_device_id = convert_java_touch_id(touch_device_id_in);

    // Finger ID should be greater than 0
    let finger_id = (pointer_finger_id_in as i64 + 1) as touch::FingerID;

    // (SDL_AddTouch() fails only without memory)
    touch::add_touch(touch_device_id, TouchDeviceType::Direct, "");

    match action {
        ACTION_DOWN | ACTION_POINTER_DOWN => touch::send_touch(
            Duration::ZERO,
            touch_device_id,
            finger_id,
            Some(window),
            EventType::FINGER_DOWN,
            x,
            y,
            p,
        ),

        ACTION_MOVE => touch::send_touch_motion(
            Duration::ZERO,
            touch_device_id,
            finger_id,
            Some(window),
            x,
            y,
            p,
        ),

        ACTION_UP | ACTION_POINTER_UP => touch::send_touch(
            Duration::ZERO,
            touch_device_id,
            finger_id,
            Some(window),
            EventType::FINGER_UP,
            x,
            y,
            p,
        ),

        ACTION_CANCEL => touch::send_touch(
            Duration::ZERO,
            touch_device_id,
            finger_id,
            Some(window),
            EventType::FINGER_CANCELED,
            x,
            y,
            p,
        ),

        _ => {}
    }
}
