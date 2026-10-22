// Layout tests of sdl3-emscripten's html5.h declarations (src/sys.rs).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The sizes, alignments, field offsets and field sizes of the structures,
//! and the values of the constants, as emscripten 6.0.13's headers give
//! them: the expected values are what a C program built with emcc printed
//! (`sizeof`, `_Alignof`, `offsetof`, the macros' values), one assertion
//! per line. The last test links every declared function, so a wrong name
//! fails the link.

#![cfg(target_os = "emscripten")]

use sdl3_emscripten::sys::*;
use std::mem::{align_of, offset_of, size_of};

/// The size of a field, without a value of the structure.
macro_rules! field_size {
    ($t:ty, $f:ident) => {{
        fn size_of_pointee<T>(_: *const T) -> usize {
            size_of::<T>()
        }
        let u = std::mem::MaybeUninit::<$t>::uninit();
        // SAFETY: only the field's address is taken, nothing is read.
        size_of_pointee(unsafe { std::ptr::addr_of!((*u.as_ptr()).$f) })
    }};
}

#[test]
fn emscripten_keyboard_event() {
    assert_eq!(size_of::<EmscriptenKeyboardEvent>(), 160);
    assert_eq!(align_of::<EmscriptenKeyboardEvent>(), 8);
    assert_eq!(offset_of!(EmscriptenKeyboardEvent, timestamp), 0);
    assert_eq!(field_size!(EmscriptenKeyboardEvent, timestamp), 8);
    assert_eq!(offset_of!(EmscriptenKeyboardEvent, location), 8);
    assert_eq!(field_size!(EmscriptenKeyboardEvent, location), 4);
    assert_eq!(offset_of!(EmscriptenKeyboardEvent, ctrlKey), 12);
    assert_eq!(field_size!(EmscriptenKeyboardEvent, ctrlKey), 1);
    assert_eq!(offset_of!(EmscriptenKeyboardEvent, shiftKey), 13);
    assert_eq!(field_size!(EmscriptenKeyboardEvent, shiftKey), 1);
    assert_eq!(offset_of!(EmscriptenKeyboardEvent, altKey), 14);
    assert_eq!(field_size!(EmscriptenKeyboardEvent, altKey), 1);
    assert_eq!(offset_of!(EmscriptenKeyboardEvent, metaKey), 15);
    assert_eq!(field_size!(EmscriptenKeyboardEvent, metaKey), 1);
    assert_eq!(offset_of!(EmscriptenKeyboardEvent, repeat), 16);
    assert_eq!(field_size!(EmscriptenKeyboardEvent, repeat), 1);
    assert_eq!(offset_of!(EmscriptenKeyboardEvent, charCode), 20);
    assert_eq!(field_size!(EmscriptenKeyboardEvent, charCode), 4);
    assert_eq!(offset_of!(EmscriptenKeyboardEvent, keyCode), 24);
    assert_eq!(field_size!(EmscriptenKeyboardEvent, keyCode), 4);
    assert_eq!(offset_of!(EmscriptenKeyboardEvent, which), 28);
    assert_eq!(field_size!(EmscriptenKeyboardEvent, which), 4);
    assert_eq!(offset_of!(EmscriptenKeyboardEvent, key), 32);
    assert_eq!(field_size!(EmscriptenKeyboardEvent, key), 32);
    assert_eq!(offset_of!(EmscriptenKeyboardEvent, code), 64);
    assert_eq!(field_size!(EmscriptenKeyboardEvent, code), 32);
    assert_eq!(offset_of!(EmscriptenKeyboardEvent, charValue), 96);
    assert_eq!(field_size!(EmscriptenKeyboardEvent, charValue), 32);
    assert_eq!(offset_of!(EmscriptenKeyboardEvent, locale), 128);
    assert_eq!(field_size!(EmscriptenKeyboardEvent, locale), 32);
}

#[test]
fn emscripten_mouse_event() {
    assert_eq!(size_of::<EmscriptenMouseEvent>(), 64);
    assert_eq!(align_of::<EmscriptenMouseEvent>(), 8);
    assert_eq!(offset_of!(EmscriptenMouseEvent, timestamp), 0);
    assert_eq!(field_size!(EmscriptenMouseEvent, timestamp), 8);
    assert_eq!(offset_of!(EmscriptenMouseEvent, screenX), 8);
    assert_eq!(field_size!(EmscriptenMouseEvent, screenX), 4);
    assert_eq!(offset_of!(EmscriptenMouseEvent, screenY), 12);
    assert_eq!(field_size!(EmscriptenMouseEvent, screenY), 4);
    assert_eq!(offset_of!(EmscriptenMouseEvent, clientX), 16);
    assert_eq!(field_size!(EmscriptenMouseEvent, clientX), 4);
    assert_eq!(offset_of!(EmscriptenMouseEvent, clientY), 20);
    assert_eq!(field_size!(EmscriptenMouseEvent, clientY), 4);
    assert_eq!(offset_of!(EmscriptenMouseEvent, ctrlKey), 24);
    assert_eq!(field_size!(EmscriptenMouseEvent, ctrlKey), 1);
    assert_eq!(offset_of!(EmscriptenMouseEvent, shiftKey), 25);
    assert_eq!(field_size!(EmscriptenMouseEvent, shiftKey), 1);
    assert_eq!(offset_of!(EmscriptenMouseEvent, altKey), 26);
    assert_eq!(field_size!(EmscriptenMouseEvent, altKey), 1);
    assert_eq!(offset_of!(EmscriptenMouseEvent, metaKey), 27);
    assert_eq!(field_size!(EmscriptenMouseEvent, metaKey), 1);
    assert_eq!(offset_of!(EmscriptenMouseEvent, button), 28);
    assert_eq!(field_size!(EmscriptenMouseEvent, button), 2);
    assert_eq!(offset_of!(EmscriptenMouseEvent, buttons), 30);
    assert_eq!(field_size!(EmscriptenMouseEvent, buttons), 2);
    assert_eq!(offset_of!(EmscriptenMouseEvent, movementX), 32);
    assert_eq!(field_size!(EmscriptenMouseEvent, movementX), 4);
    assert_eq!(offset_of!(EmscriptenMouseEvent, movementY), 36);
    assert_eq!(field_size!(EmscriptenMouseEvent, movementY), 4);
    assert_eq!(offset_of!(EmscriptenMouseEvent, targetX), 40);
    assert_eq!(field_size!(EmscriptenMouseEvent, targetX), 4);
    assert_eq!(offset_of!(EmscriptenMouseEvent, targetY), 44);
    assert_eq!(field_size!(EmscriptenMouseEvent, targetY), 4);
    assert_eq!(offset_of!(EmscriptenMouseEvent, canvasX), 48);
    assert_eq!(field_size!(EmscriptenMouseEvent, canvasX), 4);
    assert_eq!(offset_of!(EmscriptenMouseEvent, canvasY), 52);
    assert_eq!(field_size!(EmscriptenMouseEvent, canvasY), 4);
    assert_eq!(offset_of!(EmscriptenMouseEvent, padding), 56);
    assert_eq!(field_size!(EmscriptenMouseEvent, padding), 4);
}

#[test]
fn emscripten_wheel_event() {
    assert_eq!(size_of::<EmscriptenWheelEvent>(), 96);
    assert_eq!(align_of::<EmscriptenWheelEvent>(), 8);
    assert_eq!(offset_of!(EmscriptenWheelEvent, mouse), 0);
    assert_eq!(field_size!(EmscriptenWheelEvent, mouse), 64);
    assert_eq!(offset_of!(EmscriptenWheelEvent, deltaX), 64);
    assert_eq!(field_size!(EmscriptenWheelEvent, deltaX), 8);
    assert_eq!(offset_of!(EmscriptenWheelEvent, deltaY), 72);
    assert_eq!(field_size!(EmscriptenWheelEvent, deltaY), 8);
    assert_eq!(offset_of!(EmscriptenWheelEvent, deltaZ), 80);
    assert_eq!(field_size!(EmscriptenWheelEvent, deltaZ), 8);
    assert_eq!(offset_of!(EmscriptenWheelEvent, deltaMode), 88);
    assert_eq!(field_size!(EmscriptenWheelEvent, deltaMode), 4);
}

#[test]
fn emscripten_ui_event() {
    assert_eq!(size_of::<EmscriptenUiEvent>(), 36);
    assert_eq!(align_of::<EmscriptenUiEvent>(), 4);
    assert_eq!(offset_of!(EmscriptenUiEvent, detail), 0);
    assert_eq!(field_size!(EmscriptenUiEvent, detail), 4);
    assert_eq!(offset_of!(EmscriptenUiEvent, documentBodyClientWidth), 4);
    assert_eq!(field_size!(EmscriptenUiEvent, documentBodyClientWidth), 4);
    assert_eq!(offset_of!(EmscriptenUiEvent, documentBodyClientHeight), 8);
    assert_eq!(field_size!(EmscriptenUiEvent, documentBodyClientHeight), 4);
    assert_eq!(offset_of!(EmscriptenUiEvent, windowInnerWidth), 12);
    assert_eq!(field_size!(EmscriptenUiEvent, windowInnerWidth), 4);
    assert_eq!(offset_of!(EmscriptenUiEvent, windowInnerHeight), 16);
    assert_eq!(field_size!(EmscriptenUiEvent, windowInnerHeight), 4);
    assert_eq!(offset_of!(EmscriptenUiEvent, windowOuterWidth), 20);
    assert_eq!(field_size!(EmscriptenUiEvent, windowOuterWidth), 4);
    assert_eq!(offset_of!(EmscriptenUiEvent, windowOuterHeight), 24);
    assert_eq!(field_size!(EmscriptenUiEvent, windowOuterHeight), 4);
    assert_eq!(offset_of!(EmscriptenUiEvent, scrollTop), 28);
    assert_eq!(field_size!(EmscriptenUiEvent, scrollTop), 4);
    assert_eq!(offset_of!(EmscriptenUiEvent, scrollLeft), 32);
    assert_eq!(field_size!(EmscriptenUiEvent, scrollLeft), 4);
}

#[test]
fn emscripten_focus_event() {
    assert_eq!(size_of::<EmscriptenFocusEvent>(), 256);
    assert_eq!(align_of::<EmscriptenFocusEvent>(), 1);
    assert_eq!(offset_of!(EmscriptenFocusEvent, nodeName), 0);
    assert_eq!(field_size!(EmscriptenFocusEvent, nodeName), 128);
    assert_eq!(offset_of!(EmscriptenFocusEvent, id), 128);
    assert_eq!(field_size!(EmscriptenFocusEvent, id), 128);
}

#[test]
fn emscripten_device_motion_event() {
    assert_eq!(size_of::<EmscriptenDeviceMotionEvent>(), 80);
    assert_eq!(align_of::<EmscriptenDeviceMotionEvent>(), 8);
    assert_eq!(offset_of!(EmscriptenDeviceMotionEvent, accelerationX), 0);
    assert_eq!(field_size!(EmscriptenDeviceMotionEvent, accelerationX), 8);
    assert_eq!(offset_of!(EmscriptenDeviceMotionEvent, accelerationY), 8);
    assert_eq!(field_size!(EmscriptenDeviceMotionEvent, accelerationY), 8);
    assert_eq!(offset_of!(EmscriptenDeviceMotionEvent, accelerationZ), 16);
    assert_eq!(field_size!(EmscriptenDeviceMotionEvent, accelerationZ), 8);
    assert_eq!(
        offset_of!(EmscriptenDeviceMotionEvent, accelerationIncludingGravityX),
        24
    );
    assert_eq!(
        field_size!(EmscriptenDeviceMotionEvent, accelerationIncludingGravityX),
        8
    );
    assert_eq!(
        offset_of!(EmscriptenDeviceMotionEvent, accelerationIncludingGravityY),
        32
    );
    assert_eq!(
        field_size!(EmscriptenDeviceMotionEvent, accelerationIncludingGravityY),
        8
    );
    assert_eq!(
        offset_of!(EmscriptenDeviceMotionEvent, accelerationIncludingGravityZ),
        40
    );
    assert_eq!(
        field_size!(EmscriptenDeviceMotionEvent, accelerationIncludingGravityZ),
        8
    );
    assert_eq!(
        offset_of!(EmscriptenDeviceMotionEvent, rotationRateAlpha),
        48
    );
    assert_eq!(
        field_size!(EmscriptenDeviceMotionEvent, rotationRateAlpha),
        8
    );
    assert_eq!(
        offset_of!(EmscriptenDeviceMotionEvent, rotationRateBeta),
        56
    );
    assert_eq!(
        field_size!(EmscriptenDeviceMotionEvent, rotationRateBeta),
        8
    );
    assert_eq!(
        offset_of!(EmscriptenDeviceMotionEvent, rotationRateGamma),
        64
    );
    assert_eq!(
        field_size!(EmscriptenDeviceMotionEvent, rotationRateGamma),
        8
    );
    assert_eq!(offset_of!(EmscriptenDeviceMotionEvent, supportedFields), 72);
    assert_eq!(field_size!(EmscriptenDeviceMotionEvent, supportedFields), 4);
}

#[test]
fn emscripten_orientation_change_event() {
    assert_eq!(size_of::<EmscriptenOrientationChangeEvent>(), 8);
    assert_eq!(align_of::<EmscriptenOrientationChangeEvent>(), 4);
    assert_eq!(
        offset_of!(EmscriptenOrientationChangeEvent, orientationIndex),
        0
    );
    assert_eq!(
        field_size!(EmscriptenOrientationChangeEvent, orientationIndex),
        4
    );
    assert_eq!(
        offset_of!(EmscriptenOrientationChangeEvent, orientationAngle),
        4
    );
    assert_eq!(
        field_size!(EmscriptenOrientationChangeEvent, orientationAngle),
        4
    );
}

#[test]
fn emscripten_fullscreen_change_event() {
    assert_eq!(size_of::<EmscriptenFullscreenChangeEvent>(), 276);
    assert_eq!(align_of::<EmscriptenFullscreenChangeEvent>(), 4);
    assert_eq!(offset_of!(EmscriptenFullscreenChangeEvent, isFullscreen), 0);
    assert_eq!(
        field_size!(EmscriptenFullscreenChangeEvent, isFullscreen),
        1
    );
    assert_eq!(
        offset_of!(EmscriptenFullscreenChangeEvent, fullscreenEnabled),
        1
    );
    assert_eq!(
        field_size!(EmscriptenFullscreenChangeEvent, fullscreenEnabled),
        1
    );
    assert_eq!(offset_of!(EmscriptenFullscreenChangeEvent, nodeName), 2);
    assert_eq!(field_size!(EmscriptenFullscreenChangeEvent, nodeName), 128);
    assert_eq!(offset_of!(EmscriptenFullscreenChangeEvent, id), 130);
    assert_eq!(field_size!(EmscriptenFullscreenChangeEvent, id), 128);
    assert_eq!(
        offset_of!(EmscriptenFullscreenChangeEvent, elementWidth),
        260
    );
    assert_eq!(
        field_size!(EmscriptenFullscreenChangeEvent, elementWidth),
        4
    );
    assert_eq!(
        offset_of!(EmscriptenFullscreenChangeEvent, elementHeight),
        264
    );
    assert_eq!(
        field_size!(EmscriptenFullscreenChangeEvent, elementHeight),
        4
    );
    assert_eq!(
        offset_of!(EmscriptenFullscreenChangeEvent, screenWidth),
        268
    );
    assert_eq!(field_size!(EmscriptenFullscreenChangeEvent, screenWidth), 4);
    assert_eq!(
        offset_of!(EmscriptenFullscreenChangeEvent, screenHeight),
        272
    );
    assert_eq!(
        field_size!(EmscriptenFullscreenChangeEvent, screenHeight),
        4
    );
}

#[test]
fn emscripten_fullscreen_strategy() {
    assert_eq!(size_of::<EmscriptenFullscreenStrategy>(), 24);
    assert_eq!(align_of::<EmscriptenFullscreenStrategy>(), 4);
    assert_eq!(offset_of!(EmscriptenFullscreenStrategy, scaleMode), 0);
    assert_eq!(field_size!(EmscriptenFullscreenStrategy, scaleMode), 4);
    assert_eq!(
        offset_of!(EmscriptenFullscreenStrategy, canvasResolutionScaleMode),
        4
    );
    assert_eq!(
        field_size!(EmscriptenFullscreenStrategy, canvasResolutionScaleMode),
        4
    );
    assert_eq!(offset_of!(EmscriptenFullscreenStrategy, filteringMode), 8);
    assert_eq!(field_size!(EmscriptenFullscreenStrategy, filteringMode), 4);
    assert_eq!(
        offset_of!(EmscriptenFullscreenStrategy, canvasResizedCallback),
        12
    );
    assert_eq!(
        field_size!(EmscriptenFullscreenStrategy, canvasResizedCallback),
        4
    );
    assert_eq!(
        offset_of!(EmscriptenFullscreenStrategy, canvasResizedCallbackUserData),
        16
    );
    assert_eq!(
        field_size!(EmscriptenFullscreenStrategy, canvasResizedCallbackUserData),
        4
    );
    assert_eq!(
        offset_of!(
            EmscriptenFullscreenStrategy,
            canvasResizedCallbackTargetThread
        ),
        20
    );
    assert_eq!(
        field_size!(
            EmscriptenFullscreenStrategy,
            canvasResizedCallbackTargetThread
        ),
        4
    );
}

#[test]
fn emscripten_pointerlock_change_event() {
    assert_eq!(size_of::<EmscriptenPointerlockChangeEvent>(), 257);
    assert_eq!(align_of::<EmscriptenPointerlockChangeEvent>(), 1);
    assert_eq!(offset_of!(EmscriptenPointerlockChangeEvent, isActive), 0);
    assert_eq!(field_size!(EmscriptenPointerlockChangeEvent, isActive), 1);
    assert_eq!(offset_of!(EmscriptenPointerlockChangeEvent, nodeName), 1);
    assert_eq!(field_size!(EmscriptenPointerlockChangeEvent, nodeName), 128);
    assert_eq!(offset_of!(EmscriptenPointerlockChangeEvent, id), 129);
    assert_eq!(field_size!(EmscriptenPointerlockChangeEvent, id), 128);
}

#[test]
fn emscripten_visibility_change_event() {
    assert_eq!(size_of::<EmscriptenVisibilityChangeEvent>(), 8);
    assert_eq!(align_of::<EmscriptenVisibilityChangeEvent>(), 4);
    assert_eq!(offset_of!(EmscriptenVisibilityChangeEvent, hidden), 0);
    assert_eq!(field_size!(EmscriptenVisibilityChangeEvent, hidden), 1);
    assert_eq!(
        offset_of!(EmscriptenVisibilityChangeEvent, visibilityState),
        4
    );
    assert_eq!(
        field_size!(EmscriptenVisibilityChangeEvent, visibilityState),
        4
    );
}

#[test]
fn emscripten_gamepad_event() {
    assert_eq!(size_of::<EmscriptenGamepadEvent>(), 1240);
    assert_eq!(align_of::<EmscriptenGamepadEvent>(), 8);
    assert_eq!(offset_of!(EmscriptenGamepadEvent, timestamp), 0);
    assert_eq!(field_size!(EmscriptenGamepadEvent, timestamp), 8);
    assert_eq!(offset_of!(EmscriptenGamepadEvent, numAxes), 8);
    assert_eq!(field_size!(EmscriptenGamepadEvent, numAxes), 4);
    assert_eq!(offset_of!(EmscriptenGamepadEvent, numButtons), 12);
    assert_eq!(field_size!(EmscriptenGamepadEvent, numButtons), 4);
    assert_eq!(offset_of!(EmscriptenGamepadEvent, axis), 16);
    assert_eq!(field_size!(EmscriptenGamepadEvent, axis), 512);
    assert_eq!(offset_of!(EmscriptenGamepadEvent, analogButton), 528);
    assert_eq!(field_size!(EmscriptenGamepadEvent, analogButton), 512);
    assert_eq!(offset_of!(EmscriptenGamepadEvent, digitalButton), 1040);
    assert_eq!(field_size!(EmscriptenGamepadEvent, digitalButton), 64);
    assert_eq!(offset_of!(EmscriptenGamepadEvent, connected), 1104);
    assert_eq!(field_size!(EmscriptenGamepadEvent, connected), 1);
    assert_eq!(offset_of!(EmscriptenGamepadEvent, index), 1108);
    assert_eq!(field_size!(EmscriptenGamepadEvent, index), 4);
    assert_eq!(offset_of!(EmscriptenGamepadEvent, id), 1112);
    assert_eq!(field_size!(EmscriptenGamepadEvent, id), 64);
    assert_eq!(offset_of!(EmscriptenGamepadEvent, mapping), 1176);
    assert_eq!(field_size!(EmscriptenGamepadEvent, mapping), 64);
}

#[test]
fn emscripten_battery_event() {
    assert_eq!(size_of::<EmscriptenBatteryEvent>(), 32);
    assert_eq!(align_of::<EmscriptenBatteryEvent>(), 8);
    assert_eq!(offset_of!(EmscriptenBatteryEvent, chargingTime), 0);
    assert_eq!(field_size!(EmscriptenBatteryEvent, chargingTime), 8);
    assert_eq!(offset_of!(EmscriptenBatteryEvent, dischargingTime), 8);
    assert_eq!(field_size!(EmscriptenBatteryEvent, dischargingTime), 8);
    assert_eq!(offset_of!(EmscriptenBatteryEvent, level), 16);
    assert_eq!(field_size!(EmscriptenBatteryEvent, level), 8);
    assert_eq!(offset_of!(EmscriptenBatteryEvent, charging), 24);
    assert_eq!(field_size!(EmscriptenBatteryEvent, charging), 1);
}

#[test]
fn emscripten_webgl_context_attributes() {
    assert_eq!(size_of::<EmscriptenWebGLContextAttributes>(), 36);
    assert_eq!(align_of::<EmscriptenWebGLContextAttributes>(), 4);
    assert_eq!(offset_of!(EmscriptenWebGLContextAttributes, alpha), 0);
    assert_eq!(field_size!(EmscriptenWebGLContextAttributes, alpha), 1);
    assert_eq!(offset_of!(EmscriptenWebGLContextAttributes, depth), 1);
    assert_eq!(field_size!(EmscriptenWebGLContextAttributes, depth), 1);
    assert_eq!(offset_of!(EmscriptenWebGLContextAttributes, stencil), 2);
    assert_eq!(field_size!(EmscriptenWebGLContextAttributes, stencil), 1);
    assert_eq!(offset_of!(EmscriptenWebGLContextAttributes, antialias), 3);
    assert_eq!(field_size!(EmscriptenWebGLContextAttributes, antialias), 1);
    assert_eq!(
        offset_of!(EmscriptenWebGLContextAttributes, premultipliedAlpha),
        4
    );
    assert_eq!(
        field_size!(EmscriptenWebGLContextAttributes, premultipliedAlpha),
        1
    );
    assert_eq!(
        offset_of!(EmscriptenWebGLContextAttributes, preserveDrawingBuffer),
        5
    );
    assert_eq!(
        field_size!(EmscriptenWebGLContextAttributes, preserveDrawingBuffer),
        1
    );
    assert_eq!(
        offset_of!(EmscriptenWebGLContextAttributes, powerPreference),
        8
    );
    assert_eq!(
        field_size!(EmscriptenWebGLContextAttributes, powerPreference),
        4
    );
    assert_eq!(
        offset_of!(
            EmscriptenWebGLContextAttributes,
            failIfMajorPerformanceCaveat
        ),
        12
    );
    assert_eq!(
        field_size!(
            EmscriptenWebGLContextAttributes,
            failIfMajorPerformanceCaveat
        ),
        1
    );
    assert_eq!(
        offset_of!(EmscriptenWebGLContextAttributes, majorVersion),
        16
    );
    assert_eq!(
        field_size!(EmscriptenWebGLContextAttributes, majorVersion),
        4
    );
    assert_eq!(
        offset_of!(EmscriptenWebGLContextAttributes, minorVersion),
        20
    );
    assert_eq!(
        field_size!(EmscriptenWebGLContextAttributes, minorVersion),
        4
    );
    assert_eq!(
        offset_of!(EmscriptenWebGLContextAttributes, enableExtensionsByDefault),
        24
    );
    assert_eq!(
        field_size!(EmscriptenWebGLContextAttributes, enableExtensionsByDefault),
        1
    );
    assert_eq!(
        offset_of!(EmscriptenWebGLContextAttributes, explicitSwapControl),
        25
    );
    assert_eq!(
        field_size!(EmscriptenWebGLContextAttributes, explicitSwapControl),
        1
    );
    assert_eq!(
        offset_of!(EmscriptenWebGLContextAttributes, proxyContextToMainThread),
        28
    );
    assert_eq!(
        field_size!(EmscriptenWebGLContextAttributes, proxyContextToMainThread),
        4
    );
    assert_eq!(
        offset_of!(
            EmscriptenWebGLContextAttributes,
            renderViaOffscreenBackBuffer
        ),
        32
    );
    assert_eq!(
        field_size!(
            EmscriptenWebGLContextAttributes,
            renderViaOffscreenBackBuffer
        ),
        1
    );
    assert_eq!(
        offset_of!(EmscriptenWebGLContextAttributes, desynchronized),
        33
    );
    assert_eq!(
        field_size!(EmscriptenWebGLContextAttributes, desynchronized),
        1
    );
}

#[test]
fn constants() {
    assert_eq!(EM_TRUE as i32, 1);
    assert_eq!(EM_FALSE as i32, 0);
    assert_eq!(EMSCRIPTEN_RESULT_SUCCESS, 0);
    assert_eq!(EMSCRIPTEN_RESULT_DEFERRED, 1);
    assert_eq!(EMSCRIPTEN_RESULT_NOT_SUPPORTED, -1);
    assert_eq!(EMSCRIPTEN_RESULT_FAILED_NOT_DEFERRED, -2);
    assert_eq!(EMSCRIPTEN_RESULT_INVALID_TARGET, -3);
    assert_eq!(EMSCRIPTEN_RESULT_UNKNOWN_TARGET, -4);
    assert_eq!(EMSCRIPTEN_RESULT_INVALID_PARAM, -5);
    assert_eq!(EMSCRIPTEN_RESULT_FAILED, -6);
    assert_eq!(EMSCRIPTEN_RESULT_NO_DATA, -7);
    assert_eq!(EMSCRIPTEN_RESULT_TIMED_OUT, -8);
    assert_eq!(EM_TIMING_SETTIMEOUT, 0);
    assert_eq!(EM_TIMING_RAF, 1);
    assert_eq!(EM_TIMING_SETIMMEDIATE, 2);
    assert_eq!(EMSCRIPTEN_EVENT_KEYPRESS, 1);
    assert_eq!(EMSCRIPTEN_EVENT_KEYDOWN, 2);
    assert_eq!(EMSCRIPTEN_EVENT_KEYUP, 3);
    assert_eq!(EMSCRIPTEN_EVENT_CLICK, 4);
    assert_eq!(EMSCRIPTEN_EVENT_MOUSEDOWN, 5);
    assert_eq!(EMSCRIPTEN_EVENT_MOUSEUP, 6);
    assert_eq!(EMSCRIPTEN_EVENT_DBLCLICK, 7);
    assert_eq!(EMSCRIPTEN_EVENT_MOUSEMOVE, 8);
    assert_eq!(EMSCRIPTEN_EVENT_WHEEL, 9);
    assert_eq!(EMSCRIPTEN_EVENT_RESIZE, 10);
    assert_eq!(EMSCRIPTEN_EVENT_SCROLL, 11);
    assert_eq!(EMSCRIPTEN_EVENT_BLUR, 12);
    assert_eq!(EMSCRIPTEN_EVENT_FOCUS, 13);
    assert_eq!(EMSCRIPTEN_EVENT_FOCUSIN, 14);
    assert_eq!(EMSCRIPTEN_EVENT_FOCUSOUT, 15);
    assert_eq!(EMSCRIPTEN_EVENT_DEVICEORIENTATION, 16);
    assert_eq!(EMSCRIPTEN_EVENT_DEVICEMOTION, 17);
    assert_eq!(EMSCRIPTEN_EVENT_ORIENTATIONCHANGE, 18);
    assert_eq!(EMSCRIPTEN_EVENT_FULLSCREENCHANGE, 19);
    assert_eq!(EMSCRIPTEN_EVENT_POINTERLOCKCHANGE, 20);
    assert_eq!(EMSCRIPTEN_EVENT_VISIBILITYCHANGE, 21);
    assert_eq!(EMSCRIPTEN_EVENT_TOUCHSTART, 22);
    assert_eq!(EMSCRIPTEN_EVENT_TOUCHEND, 23);
    assert_eq!(EMSCRIPTEN_EVENT_TOUCHMOVE, 24);
    assert_eq!(EMSCRIPTEN_EVENT_TOUCHCANCEL, 25);
    assert_eq!(EMSCRIPTEN_EVENT_GAMEPADCONNECTED, 26);
    assert_eq!(EMSCRIPTEN_EVENT_GAMEPADDISCONNECTED, 27);
    assert_eq!(EMSCRIPTEN_EVENT_BEFOREUNLOAD, 28);
    assert_eq!(EMSCRIPTEN_EVENT_BATTERYCHARGINGCHANGE, 29);
    assert_eq!(EMSCRIPTEN_EVENT_BATTERYLEVELCHANGE, 30);
    assert_eq!(EMSCRIPTEN_EVENT_WEBGLCONTEXTLOST, 31);
    assert_eq!(EMSCRIPTEN_EVENT_WEBGLCONTEXTRESTORED, 32);
    assert_eq!(EMSCRIPTEN_EVENT_MOUSEENTER, 33);
    assert_eq!(EMSCRIPTEN_EVENT_MOUSELEAVE, 34);
    assert_eq!(EMSCRIPTEN_EVENT_MOUSEOVER, 35);
    assert_eq!(EMSCRIPTEN_EVENT_MOUSEOUT, 36);
    assert_eq!(EMSCRIPTEN_EVENT_CANVASRESIZED, 37);
    assert_eq!(EMSCRIPTEN_EVENT_POINTERLOCKERROR, 38);
    assert_eq!(EMSCRIPTEN_EVENT_CONTEXTMENU, 39);
    assert_eq!(EMSCRIPTEN_EVENT_TARGET_INVALID as usize, 0);
    assert_eq!(EMSCRIPTEN_EVENT_TARGET_DOCUMENT as usize, 1);
    assert_eq!(EMSCRIPTEN_EVENT_TARGET_WINDOW as usize, 2);
    assert_eq!(EMSCRIPTEN_EVENT_TARGET_SCREEN as usize, 3);
    assert_eq!(DOM_KEY_LOCATION_STANDARD, 0);
    assert_eq!(DOM_KEY_LOCATION_LEFT, 1);
    assert_eq!(DOM_KEY_LOCATION_RIGHT, 2);
    assert_eq!(DOM_KEY_LOCATION_NUMPAD, 3);
    assert_eq!(EM_HTML5_SHORT_STRING_LEN_BYTES, 32);
    assert_eq!(EM_HTML5_MEDIUM_STRING_LEN_BYTES, 64);
    assert_eq!(EM_HTML5_LONG_STRING_LEN_BYTES, 128);
    assert_eq!(DOM_DELTA_PIXEL, 0);
    assert_eq!(DOM_DELTA_LINE, 1);
    assert_eq!(DOM_DELTA_PAGE, 2);
    assert_eq!(EMSCRIPTEN_DEVICE_MOTION_EVENT_SUPPORTS_ACCELERATION, 1);
    assert_eq!(
        EMSCRIPTEN_DEVICE_MOTION_EVENT_SUPPORTS_ACCELERATION_INCLUDING_GRAVITY,
        2
    );
    assert_eq!(EMSCRIPTEN_DEVICE_MOTION_EVENT_SUPPORTS_ROTATION_RATE, 4);
    assert_eq!(EMSCRIPTEN_ORIENTATION_UNSUPPORTED, 0);
    assert_eq!(EMSCRIPTEN_ORIENTATION_PORTRAIT_PRIMARY, 1);
    assert_eq!(EMSCRIPTEN_ORIENTATION_PORTRAIT_SECONDARY, 2);
    assert_eq!(EMSCRIPTEN_ORIENTATION_LANDSCAPE_PRIMARY, 4);
    assert_eq!(EMSCRIPTEN_ORIENTATION_LANDSCAPE_SECONDARY, 8);
    assert_eq!(EMSCRIPTEN_FULLSCREEN_SCALE_DEFAULT, 0);
    assert_eq!(EMSCRIPTEN_FULLSCREEN_SCALE_STRETCH, 1);
    assert_eq!(EMSCRIPTEN_FULLSCREEN_SCALE_ASPECT, 2);
    assert_eq!(EMSCRIPTEN_FULLSCREEN_SCALE_CENTER, 3);
    assert_eq!(EMSCRIPTEN_FULLSCREEN_CANVAS_SCALE_NONE, 0);
    assert_eq!(EMSCRIPTEN_FULLSCREEN_CANVAS_SCALE_STDDEF, 1);
    assert_eq!(EMSCRIPTEN_FULLSCREEN_CANVAS_SCALE_HIDEF, 2);
    assert_eq!(EMSCRIPTEN_FULLSCREEN_FILTERING_DEFAULT, 0);
    assert_eq!(EMSCRIPTEN_FULLSCREEN_FILTERING_NEAREST, 1);
    assert_eq!(EMSCRIPTEN_FULLSCREEN_FILTERING_BILINEAR, 2);
    assert_eq!(EMSCRIPTEN_VISIBILITY_HIDDEN, 0);
    assert_eq!(EMSCRIPTEN_VISIBILITY_VISIBLE, 1);
    assert_eq!(EMSCRIPTEN_VISIBILITY_PRERENDER, 2);
    assert_eq!(EMSCRIPTEN_VISIBILITY_UNLOADED, 3);
    assert_eq!(EM_CALLBACK_THREAD_CONTEXT_MAIN_RUNTIME_THREAD, 1);
    assert_eq!(EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD, 2);
    assert_eq!(EMSCRIPTEN_WEBGL_CONTEXT_PROXY_DISALLOW, 0);
    assert_eq!(EMSCRIPTEN_WEBGL_CONTEXT_PROXY_FALLBACK, 1);
    assert_eq!(EMSCRIPTEN_WEBGL_CONTEXT_PROXY_ALWAYS, 2);
    assert_eq!(EM_WEBGL_POWER_PREFERENCE_DEFAULT, 0);
    assert_eq!(EM_WEBGL_POWER_PREFERENCE_LOW_POWER, 1);
    assert_eq!(EM_WEBGL_POWER_PREFERENCE_HIGH_PERFORMANCE, 2);
    assert_eq!(DOM_PK_UNKNOWN, 0);
    assert_eq!(DOM_PK_ESCAPE, 1);
    assert_eq!(DOM_PK_0, 2);
    assert_eq!(DOM_PK_1, 3);
    assert_eq!(DOM_PK_2, 4);
    assert_eq!(DOM_PK_3, 5);
    assert_eq!(DOM_PK_4, 6);
    assert_eq!(DOM_PK_5, 7);
    assert_eq!(DOM_PK_6, 8);
    assert_eq!(DOM_PK_7, 9);
    assert_eq!(DOM_PK_8, 10);
    assert_eq!(DOM_PK_9, 11);
    assert_eq!(DOM_PK_MINUS, 12);
    assert_eq!(DOM_PK_EQUAL, 13);
    assert_eq!(DOM_PK_BACKSPACE, 14);
    assert_eq!(DOM_PK_TAB, 15);
    assert_eq!(DOM_PK_Q, 16);
    assert_eq!(DOM_PK_W, 17);
    assert_eq!(DOM_PK_E, 18);
    assert_eq!(DOM_PK_R, 19);
    assert_eq!(DOM_PK_T, 20);
    assert_eq!(DOM_PK_Y, 21);
    assert_eq!(DOM_PK_U, 22);
    assert_eq!(DOM_PK_I, 23);
    assert_eq!(DOM_PK_O, 24);
    assert_eq!(DOM_PK_P, 25);
    assert_eq!(DOM_PK_BRACKET_LEFT, 26);
    assert_eq!(DOM_PK_BRACKET_RIGHT, 27);
    assert_eq!(DOM_PK_ENTER, 28);
    assert_eq!(DOM_PK_CONTROL_LEFT, 29);
    assert_eq!(DOM_PK_A, 30);
    assert_eq!(DOM_PK_S, 31);
    assert_eq!(DOM_PK_D, 32);
    assert_eq!(DOM_PK_F, 33);
    assert_eq!(DOM_PK_G, 34);
    assert_eq!(DOM_PK_H, 35);
    assert_eq!(DOM_PK_J, 36);
    assert_eq!(DOM_PK_K, 37);
    assert_eq!(DOM_PK_L, 38);
    assert_eq!(DOM_PK_SEMICOLON, 39);
    assert_eq!(DOM_PK_QUOTE, 40);
    assert_eq!(DOM_PK_BACKQUOTE, 41);
    assert_eq!(DOM_PK_SHIFT_LEFT, 42);
    assert_eq!(DOM_PK_BACKSLASH, 43);
    assert_eq!(DOM_PK_Z, 44);
    assert_eq!(DOM_PK_X, 45);
    assert_eq!(DOM_PK_C, 46);
    assert_eq!(DOM_PK_V, 47);
    assert_eq!(DOM_PK_B, 48);
    assert_eq!(DOM_PK_N, 49);
    assert_eq!(DOM_PK_M, 50);
    assert_eq!(DOM_PK_COMMA, 51);
    assert_eq!(DOM_PK_PERIOD, 52);
    assert_eq!(DOM_PK_SLASH, 53);
    assert_eq!(DOM_PK_SHIFT_RIGHT, 54);
    assert_eq!(DOM_PK_NUMPAD_MULTIPLY, 55);
    assert_eq!(DOM_PK_ALT_LEFT, 56);
    assert_eq!(DOM_PK_SPACE, 57);
    assert_eq!(DOM_PK_CAPS_LOCK, 58);
    assert_eq!(DOM_PK_F1, 59);
    assert_eq!(DOM_PK_F2, 60);
    assert_eq!(DOM_PK_F3, 61);
    assert_eq!(DOM_PK_F4, 62);
    assert_eq!(DOM_PK_F5, 63);
    assert_eq!(DOM_PK_F6, 64);
    assert_eq!(DOM_PK_F7, 65);
    assert_eq!(DOM_PK_F8, 66);
    assert_eq!(DOM_PK_F9, 67);
    assert_eq!(DOM_PK_F10, 68);
    assert_eq!(DOM_PK_PAUSE, 69);
    assert_eq!(DOM_PK_SCROLL_LOCK, 70);
    assert_eq!(DOM_PK_NUMPAD_7, 71);
    assert_eq!(DOM_PK_NUMPAD_8, 72);
    assert_eq!(DOM_PK_NUMPAD_9, 73);
    assert_eq!(DOM_PK_NUMPAD_SUBTRACT, 74);
    assert_eq!(DOM_PK_NUMPAD_4, 75);
    assert_eq!(DOM_PK_NUMPAD_5, 76);
    assert_eq!(DOM_PK_NUMPAD_6, 77);
    assert_eq!(DOM_PK_NUMPAD_ADD, 78);
    assert_eq!(DOM_PK_NUMPAD_1, 79);
    assert_eq!(DOM_PK_NUMPAD_2, 80);
    assert_eq!(DOM_PK_NUMPAD_3, 81);
    assert_eq!(DOM_PK_NUMPAD_0, 82);
    assert_eq!(DOM_PK_NUMPAD_DECIMAL, 83);
    assert_eq!(DOM_PK_PRINT_SCREEN, 84);
    assert_eq!(DOM_PK_INTL_BACKSLASH, 86);
    assert_eq!(DOM_PK_F11, 87);
    assert_eq!(DOM_PK_F12, 88);
    assert_eq!(DOM_PK_NUMPAD_EQUAL, 89);
    assert_eq!(DOM_PK_F13, 100);
    assert_eq!(DOM_PK_F14, 101);
    assert_eq!(DOM_PK_F15, 102);
    assert_eq!(DOM_PK_F16, 103);
    assert_eq!(DOM_PK_F17, 104);
    assert_eq!(DOM_PK_F18, 105);
    assert_eq!(DOM_PK_F19, 106);
    assert_eq!(DOM_PK_F20, 107);
    assert_eq!(DOM_PK_F21, 108);
    assert_eq!(DOM_PK_F22, 109);
    assert_eq!(DOM_PK_F23, 110);
    assert_eq!(DOM_PK_KANA_MODE, 112);
    assert_eq!(DOM_PK_LANG_2, 113);
    assert_eq!(DOM_PK_LANG_1, 114);
    assert_eq!(DOM_PK_INTL_RO, 115);
    assert_eq!(DOM_PK_F24, 118);
    assert_eq!(DOM_PK_CONVERT, 121);
    assert_eq!(DOM_PK_NON_CONVERT, 123);
    assert_eq!(DOM_PK_INTL_YEN, 125);
    assert_eq!(DOM_PK_NUMPAD_COMMA, 126);
    assert_eq!(DOM_PK_PASTE, 57354);
    assert_eq!(DOM_PK_MEDIA_TRACK_PREVIOUS, 57360);
    assert_eq!(DOM_PK_CUT, 57367);
    assert_eq!(DOM_PK_COPY, 57368);
    assert_eq!(DOM_PK_MEDIA_TRACK_NEXT, 57369);
    assert_eq!(DOM_PK_NUMPAD_ENTER, 57372);
    assert_eq!(DOM_PK_CONTROL_RIGHT, 57373);
    assert_eq!(DOM_PK_AUDIO_VOLUME_MUTE, 57376);
    assert_eq!(DOM_PK_LAUNCH_APP_2, 57377);
    assert_eq!(DOM_PK_MEDIA_PLAY_PAUSE, 57378);
    assert_eq!(DOM_PK_MEDIA_STOP, 57380);
    assert_eq!(DOM_PK_EJECT, 57388);
    assert_eq!(DOM_PK_AUDIO_VOLUME_DOWN, 57390);
    assert_eq!(DOM_PK_AUDIO_VOLUME_UP, 57392);
    assert_eq!(DOM_PK_BROWSER_HOME, 57394);
    assert_eq!(DOM_PK_NUMPAD_DIVIDE, 57397);
    assert_eq!(DOM_PK_ALT_RIGHT, 57400);
    assert_eq!(DOM_PK_HELP, 57403);
    assert_eq!(DOM_PK_NUM_LOCK, 57413);
    assert_eq!(DOM_PK_HOME, 57415);
    assert_eq!(DOM_PK_ARROW_UP, 57416);
    assert_eq!(DOM_PK_PAGE_UP, 57417);
    assert_eq!(DOM_PK_ARROW_LEFT, 57419);
    assert_eq!(DOM_PK_ARROW_RIGHT, 57421);
    assert_eq!(DOM_PK_END, 57423);
    assert_eq!(DOM_PK_ARROW_DOWN, 57424);
    assert_eq!(DOM_PK_PAGE_DOWN, 57425);
    assert_eq!(DOM_PK_INSERT, 57426);
    assert_eq!(DOM_PK_DELETE, 57427);
    assert_eq!(DOM_PK_META_LEFT, 57435);
    assert_eq!(DOM_PK_OS_LEFT, 57435);
    assert_eq!(DOM_PK_META_RIGHT, 57436);
    assert_eq!(DOM_PK_OS_RIGHT, 57436);
    assert_eq!(DOM_PK_CONTEXT_MENU, 57437);
    assert_eq!(DOM_PK_POWER, 57438);
    assert_eq!(DOM_PK_BROWSER_SEARCH, 57445);
    assert_eq!(DOM_PK_BROWSER_FAVORITES, 57446);
    assert_eq!(DOM_PK_BROWSER_REFRESH, 57447);
    assert_eq!(DOM_PK_BROWSER_STOP, 57448);
    assert_eq!(DOM_PK_BROWSER_FORWARD, 57449);
    assert_eq!(DOM_PK_BROWSER_BACK, 57450);
    assert_eq!(DOM_PK_LAUNCH_APP_1, 57451);
    assert_eq!(DOM_PK_LAUNCH_MAIL, 57452);
    assert_eq!(DOM_PK_LAUNCH_MEDIA_PLAYER, 57453);
    assert_eq!(DOM_PK_MEDIA_SELECT, 57453);
}

/// Every declared function is in the program (a misspelt name fails the
/// link); nothing is called.
#[test]
fn functions_link() {
    let functions: [*const (); 47] = [
        emscripten_set_main_loop as *const (),
        emscripten_set_main_loop_timing as *const (),
        emscripten_get_main_loop_timing as *const (),
        emscripten_cancel_main_loop as *const (),
        emscripten_exit_with_live_runtime as *const (),
        emscripten_force_exit as *const (),
        emscripten_get_device_pixel_ratio as *const (),
        emscripten_set_window_title as *const (),
        emscripten_get_screen_size as *const (),
        emscripten_has_asyncify as *const (),
        emscripten_sleep as *const (),
        emscripten_set_timeout as *const (),
        emscripten_clear_timeout as *const (),
        emscripten_set_keypress_callback_on_thread as *const (),
        emscripten_set_keydown_callback_on_thread as *const (),
        emscripten_set_keyup_callback_on_thread as *const (),
        emscripten_set_wheel_callback_on_thread as *const (),
        emscripten_set_resize_callback_on_thread as *const (),
        emscripten_set_blur_callback_on_thread as *const (),
        emscripten_set_focus_callback_on_thread as *const (),
        emscripten_set_devicemotion_callback_on_thread as *const (),
        emscripten_set_orientationchange_callback_on_thread as *const (),
        emscripten_set_fullscreenchange_callback_on_thread as *const (),
        emscripten_set_pointerlockchange_callback_on_thread as *const (),
        emscripten_set_visibilitychange_callback_on_thread as *const (),
        emscripten_set_gamepadconnected_callback_on_thread as *const (),
        emscripten_set_gamepaddisconnected_callback_on_thread as *const (),
        emscripten_set_beforeunload_callback_on_thread as *const (),
        emscripten_get_fullscreen_status as *const (),
        emscripten_request_fullscreen_strategy as *const (),
        emscripten_exit_fullscreen as *const (),
        emscripten_request_pointerlock as *const (),
        emscripten_exit_pointerlock as *const (),
        emscripten_sample_gamepad_data as *const (),
        emscripten_get_num_gamepads as *const (),
        emscripten_get_gamepad_status as *const (),
        emscripten_get_battery_status as *const (),
        emscripten_set_canvas_element_size as *const (),
        emscripten_get_canvas_element_size as *const (),
        emscripten_set_element_css_size as *const (),
        emscripten_get_element_css_size as *const (),
        emscripten_webgl_init_context_attributes as *const (),
        emscripten_webgl_create_context as *const (),
        emscripten_webgl_make_context_current as *const (),
        emscripten_webgl_destroy_context as *const (),
        emscripten_webgl_get_proc_address as *const (),
        emscripten_compute_dom_pk_code as *const (),
    ];
    assert!(std::hint::black_box(functions).iter().all(|f| !f.is_null()));
}
