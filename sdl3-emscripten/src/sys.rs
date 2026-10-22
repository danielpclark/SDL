// Declarations of <emscripten/emscripten.h>, <emscripten/eventloop.h>,
// <emscripten/html5.h>, <emscripten/html5_webgl.h> and <emscripten/dom_pk_codes.h>
// (emscripten 6.0.13), as Simple DirectMedia Layer uses them.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The C API of emscripten that SDL uses: the functions, structures and
//! constants of `emscripten.h`, `eventloop.h`, `html5.h`, `html5_webgl.h`
//! and `dom_pk_codes.h`, declared by hand.
//!
//! The functions are emscripten's JavaScript library functions (wasm
//! imports emscripten resolves when it links), but for
//! `emscripten_compute_dom_pk_code` (C, in emscripten's `libhtml5`) and the
//! `emscripten_webgl_*` context functions (C wrappers in its `libGL`), from
//! the system libraries emscripten links into every program, as its libc.
//!
//! As in C, an event callback is set with its `_on_thread` function: the
//! `emscripten_set_*_callback` names are macros in `html5.h` that pass
//! [`EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD`] (the `beforeunload` one
//! [`EM_CALLBACK_THREAD_CONTEXT_MAIN_RUNTIME_THREAD`]). An event target is a
//! CSS selector or one of the special targets [`EMSCRIPTEN_EVENT_TARGET_DOCUMENT`]
//! (1), [`EMSCRIPTEN_EVENT_TARGET_WINDOW`] (2) and
//! [`EMSCRIPTEN_EVENT_TARGET_SCREEN`] (3). The layouts are checked against
//! emscripten's headers (`sizeof`/`offsetof` from emcc) in `tests/layout.rs`.

#![allow(non_camel_case_types, non_snake_case, missing_docs)]

use core::ffi::{c_char, c_int, c_uint, c_void};
use core::ptr::without_provenance;

// ---------------------------------------------------------------------------
// em_types.h

/// `EM_BOOL`: C's `bool`.
pub type EM_BOOL = bool;
pub const EM_TRUE: EM_BOOL = true;
pub const EM_FALSE: EM_BOOL = false;
/// `EM_UTF8`: the `char` of the event structures' strings (UTF-8).
pub type EM_UTF8 = c_char;

/// `EMSCRIPTEN_RESULT`: what the html5.h functions return.
pub type EMSCRIPTEN_RESULT = c_int;
pub const EMSCRIPTEN_RESULT_SUCCESS: EMSCRIPTEN_RESULT = 0;
pub const EMSCRIPTEN_RESULT_DEFERRED: EMSCRIPTEN_RESULT = 1;
pub const EMSCRIPTEN_RESULT_NOT_SUPPORTED: EMSCRIPTEN_RESULT = -1;
pub const EMSCRIPTEN_RESULT_FAILED_NOT_DEFERRED: EMSCRIPTEN_RESULT = -2;
pub const EMSCRIPTEN_RESULT_INVALID_TARGET: EMSCRIPTEN_RESULT = -3;
pub const EMSCRIPTEN_RESULT_UNKNOWN_TARGET: EMSCRIPTEN_RESULT = -4;
pub const EMSCRIPTEN_RESULT_INVALID_PARAM: EMSCRIPTEN_RESULT = -5;
pub const EMSCRIPTEN_RESULT_FAILED: EMSCRIPTEN_RESULT = -6;
pub const EMSCRIPTEN_RESULT_NO_DATA: EMSCRIPTEN_RESULT = -7;
pub const EMSCRIPTEN_RESULT_TIMED_OUT: EMSCRIPTEN_RESULT = -8;

/// `em_callback_func`.
///
/// `"C-unwind"`, as [`em_arg_callback_func`] and the callback of
/// [`emscripten_set_timeout`]: the JavaScript event loop calls these, and
/// C's `exit()` there (Rust's `std::process::exit()`) ends in a JavaScript
/// exception, `ExitStatus`, that leaves the callback back to the event loop,
/// which handles it. A `"C"` function would abort instead ("panic in a
/// function that cannot unwind"). The html5.h event callbacks, which
/// upstream's code never exits from, stay `"C"`.
pub type em_callback_func = Option<unsafe extern "C-unwind" fn()>;
/// `em_arg_callback_func`.
pub type em_arg_callback_func = Option<unsafe extern "C-unwind" fn(arg: *mut c_void)>;

/// `pthread_t` (a pointer), here only to name the thread an event callback
/// runs on.
pub type pthread_t = usize;

// ---------------------------------------------------------------------------
// emscripten.h

pub const EM_TIMING_SETTIMEOUT: c_int = 0;
pub const EM_TIMING_RAF: c_int = 1;
pub const EM_TIMING_SETIMMEDIATE: c_int = 2;

// ---------------------------------------------------------------------------
// html5.h

pub const EMSCRIPTEN_EVENT_KEYPRESS: c_int = 1;
pub const EMSCRIPTEN_EVENT_KEYDOWN: c_int = 2;
pub const EMSCRIPTEN_EVENT_KEYUP: c_int = 3;
pub const EMSCRIPTEN_EVENT_CLICK: c_int = 4;
pub const EMSCRIPTEN_EVENT_MOUSEDOWN: c_int = 5;
pub const EMSCRIPTEN_EVENT_MOUSEUP: c_int = 6;
pub const EMSCRIPTEN_EVENT_DBLCLICK: c_int = 7;
pub const EMSCRIPTEN_EVENT_MOUSEMOVE: c_int = 8;
pub const EMSCRIPTEN_EVENT_WHEEL: c_int = 9;
pub const EMSCRIPTEN_EVENT_RESIZE: c_int = 10;
pub const EMSCRIPTEN_EVENT_SCROLL: c_int = 11;
pub const EMSCRIPTEN_EVENT_BLUR: c_int = 12;
pub const EMSCRIPTEN_EVENT_FOCUS: c_int = 13;
pub const EMSCRIPTEN_EVENT_FOCUSIN: c_int = 14;
pub const EMSCRIPTEN_EVENT_FOCUSOUT: c_int = 15;
pub const EMSCRIPTEN_EVENT_DEVICEORIENTATION: c_int = 16;
pub const EMSCRIPTEN_EVENT_DEVICEMOTION: c_int = 17;
pub const EMSCRIPTEN_EVENT_ORIENTATIONCHANGE: c_int = 18;
pub const EMSCRIPTEN_EVENT_FULLSCREENCHANGE: c_int = 19;
pub const EMSCRIPTEN_EVENT_POINTERLOCKCHANGE: c_int = 20;
pub const EMSCRIPTEN_EVENT_VISIBILITYCHANGE: c_int = 21;
pub const EMSCRIPTEN_EVENT_TOUCHSTART: c_int = 22;
pub const EMSCRIPTEN_EVENT_TOUCHEND: c_int = 23;
pub const EMSCRIPTEN_EVENT_TOUCHMOVE: c_int = 24;
pub const EMSCRIPTEN_EVENT_TOUCHCANCEL: c_int = 25;
pub const EMSCRIPTEN_EVENT_GAMEPADCONNECTED: c_int = 26;
pub const EMSCRIPTEN_EVENT_GAMEPADDISCONNECTED: c_int = 27;
pub const EMSCRIPTEN_EVENT_BEFOREUNLOAD: c_int = 28;
pub const EMSCRIPTEN_EVENT_BATTERYCHARGINGCHANGE: c_int = 29;
pub const EMSCRIPTEN_EVENT_BATTERYLEVELCHANGE: c_int = 30;
pub const EMSCRIPTEN_EVENT_WEBGLCONTEXTLOST: c_int = 31;
pub const EMSCRIPTEN_EVENT_WEBGLCONTEXTRESTORED: c_int = 32;
pub const EMSCRIPTEN_EVENT_MOUSEENTER: c_int = 33;
pub const EMSCRIPTEN_EVENT_MOUSELEAVE: c_int = 34;
pub const EMSCRIPTEN_EVENT_MOUSEOVER: c_int = 35;
pub const EMSCRIPTEN_EVENT_MOUSEOUT: c_int = 36;
pub const EMSCRIPTEN_EVENT_CANVASRESIZED: c_int = 37;
pub const EMSCRIPTEN_EVENT_POINTERLOCKERROR: c_int = 38;
pub const EMSCRIPTEN_EVENT_CONTEXTMENU: c_int = 39;

pub const EMSCRIPTEN_EVENT_TARGET_INVALID: *const c_char = without_provenance(0);
/// `document`.
pub const EMSCRIPTEN_EVENT_TARGET_DOCUMENT: *const c_char = without_provenance(1);
/// `window`.
pub const EMSCRIPTEN_EVENT_TARGET_WINDOW: *const c_char = without_provenance(2);
/// `screen`.
pub const EMSCRIPTEN_EVENT_TARGET_SCREEN: *const c_char = without_provenance(3);

pub const DOM_KEY_LOCATION_STANDARD: c_uint = 0x00;
pub const DOM_KEY_LOCATION_LEFT: c_uint = 0x01;
pub const DOM_KEY_LOCATION_RIGHT: c_uint = 0x02;
pub const DOM_KEY_LOCATION_NUMPAD: c_uint = 0x03;

pub const EM_HTML5_SHORT_STRING_LEN_BYTES: usize = 32;
pub const EM_HTML5_MEDIUM_STRING_LEN_BYTES: usize = 64;
pub const EM_HTML5_LONG_STRING_LEN_BYTES: usize = 128;

/// `EmscriptenKeyboardEvent`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EmscriptenKeyboardEvent {
    pub timestamp: f64,
    pub location: c_uint,
    pub ctrlKey: bool,
    pub shiftKey: bool,
    pub altKey: bool,
    pub metaKey: bool,
    pub repeat: bool,
    pub charCode: c_uint,
    pub keyCode: c_uint,
    pub which: c_uint,
    pub key: [EM_UTF8; EM_HTML5_SHORT_STRING_LEN_BYTES],
    pub code: [EM_UTF8; EM_HTML5_SHORT_STRING_LEN_BYTES],
    pub charValue: [EM_UTF8; EM_HTML5_SHORT_STRING_LEN_BYTES],
    pub locale: [EM_UTF8; EM_HTML5_SHORT_STRING_LEN_BYTES],
}

/// `em_key_callback_func`.
pub type em_key_callback_func = Option<
    unsafe extern "C" fn(
        eventType: c_int,
        keyEvent: *const EmscriptenKeyboardEvent,
        userData: *mut c_void,
    ) -> bool,
>;

/// `EmscriptenMouseEvent`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EmscriptenMouseEvent {
    pub timestamp: f64,
    pub screenX: c_int,
    pub screenY: c_int,
    pub clientX: c_int,
    pub clientY: c_int,
    pub ctrlKey: bool,
    pub shiftKey: bool,
    pub altKey: bool,
    pub metaKey: bool,
    pub button: u16,
    pub buttons: u16,
    pub movementX: c_int,
    pub movementY: c_int,
    pub targetX: c_int,
    pub targetY: c_int,
    /// Deprecated: no longer reported.
    pub canvasX: c_int,
    /// Deprecated: no longer reported.
    pub canvasY: c_int,
    pub padding: c_int,
}

pub const DOM_DELTA_PIXEL: c_uint = 0x00;
pub const DOM_DELTA_LINE: c_uint = 0x01;
pub const DOM_DELTA_PAGE: c_uint = 0x02;

/// `EmscriptenWheelEvent`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EmscriptenWheelEvent {
    pub mouse: EmscriptenMouseEvent,
    pub deltaX: f64,
    pub deltaY: f64,
    pub deltaZ: f64,
    pub deltaMode: c_uint,
}

/// `em_wheel_callback_func`.
pub type em_wheel_callback_func = Option<
    unsafe extern "C" fn(
        eventType: c_int,
        wheelEvent: *const EmscriptenWheelEvent,
        userData: *mut c_void,
    ) -> bool,
>;

/// `EmscriptenUiEvent`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EmscriptenUiEvent {
    pub detail: c_int,
    pub documentBodyClientWidth: c_int,
    pub documentBodyClientHeight: c_int,
    pub windowInnerWidth: c_int,
    pub windowInnerHeight: c_int,
    pub windowOuterWidth: c_int,
    pub windowOuterHeight: c_int,
    pub scrollTop: c_int,
    pub scrollLeft: c_int,
}

/// `em_ui_callback_func`.
pub type em_ui_callback_func = Option<
    unsafe extern "C" fn(
        eventType: c_int,
        uiEvent: *const EmscriptenUiEvent,
        userData: *mut c_void,
    ) -> bool,
>;

/// `EmscriptenFocusEvent`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EmscriptenFocusEvent {
    pub nodeName: [EM_UTF8; EM_HTML5_LONG_STRING_LEN_BYTES],
    pub id: [EM_UTF8; EM_HTML5_LONG_STRING_LEN_BYTES],
}

/// `em_focus_callback_func`.
pub type em_focus_callback_func = Option<
    unsafe extern "C" fn(
        eventType: c_int,
        focusEvent: *const EmscriptenFocusEvent,
        userData: *mut c_void,
    ) -> bool,
>;

pub const EMSCRIPTEN_DEVICE_MOTION_EVENT_SUPPORTS_ACCELERATION: c_int = 0x01;
pub const EMSCRIPTEN_DEVICE_MOTION_EVENT_SUPPORTS_ACCELERATION_INCLUDING_GRAVITY: c_int = 0x02;
pub const EMSCRIPTEN_DEVICE_MOTION_EVENT_SUPPORTS_ROTATION_RATE: c_int = 0x04;

/// `EmscriptenDeviceMotionEvent`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EmscriptenDeviceMotionEvent {
    pub accelerationX: f64,
    pub accelerationY: f64,
    pub accelerationZ: f64,
    pub accelerationIncludingGravityX: f64,
    pub accelerationIncludingGravityY: f64,
    pub accelerationIncludingGravityZ: f64,
    pub rotationRateAlpha: f64,
    pub rotationRateBeta: f64,
    pub rotationRateGamma: f64,
    pub supportedFields: c_int,
}

/// `em_devicemotion_callback_func`.
pub type em_devicemotion_callback_func = Option<
    unsafe extern "C" fn(
        eventType: c_int,
        deviceMotionEvent: *const EmscriptenDeviceMotionEvent,
        userData: *mut c_void,
    ) -> bool,
>;

pub const EMSCRIPTEN_ORIENTATION_UNSUPPORTED: c_int = 0;
pub const EMSCRIPTEN_ORIENTATION_PORTRAIT_PRIMARY: c_int = 1;
pub const EMSCRIPTEN_ORIENTATION_PORTRAIT_SECONDARY: c_int = 2;
pub const EMSCRIPTEN_ORIENTATION_LANDSCAPE_PRIMARY: c_int = 4;
pub const EMSCRIPTEN_ORIENTATION_LANDSCAPE_SECONDARY: c_int = 8;

/// `EmscriptenOrientationChangeEvent`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EmscriptenOrientationChangeEvent {
    pub orientationIndex: c_int,
    pub orientationAngle: c_int,
}

/// `em_orientationchange_callback_func`.
pub type em_orientationchange_callback_func = Option<
    unsafe extern "C" fn(
        eventType: c_int,
        orientationChangeEvent: *const EmscriptenOrientationChangeEvent,
        userData: *mut c_void,
    ) -> bool,
>;

/// `EmscriptenFullscreenChangeEvent`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EmscriptenFullscreenChangeEvent {
    pub isFullscreen: bool,
    pub fullscreenEnabled: bool,
    pub nodeName: [EM_UTF8; EM_HTML5_LONG_STRING_LEN_BYTES],
    pub id: [EM_UTF8; EM_HTML5_LONG_STRING_LEN_BYTES],
    pub elementWidth: c_int,
    pub elementHeight: c_int,
    pub screenWidth: c_int,
    pub screenHeight: c_int,
}

/// `em_fullscreenchange_callback_func`.
pub type em_fullscreenchange_callback_func = Option<
    unsafe extern "C" fn(
        eventType: c_int,
        fullscreenChangeEvent: *const EmscriptenFullscreenChangeEvent,
        userData: *mut c_void,
    ) -> bool,
>;

/// `EMSCRIPTEN_FULLSCREEN_SCALE`.
pub type EMSCRIPTEN_FULLSCREEN_SCALE = c_int;
pub const EMSCRIPTEN_FULLSCREEN_SCALE_DEFAULT: EMSCRIPTEN_FULLSCREEN_SCALE = 0;
pub const EMSCRIPTEN_FULLSCREEN_SCALE_STRETCH: EMSCRIPTEN_FULLSCREEN_SCALE = 1;
pub const EMSCRIPTEN_FULLSCREEN_SCALE_ASPECT: EMSCRIPTEN_FULLSCREEN_SCALE = 2;
pub const EMSCRIPTEN_FULLSCREEN_SCALE_CENTER: EMSCRIPTEN_FULLSCREEN_SCALE = 3;

/// `EMSCRIPTEN_FULLSCREEN_CANVAS_SCALE`.
pub type EMSCRIPTEN_FULLSCREEN_CANVAS_SCALE = c_int;
pub const EMSCRIPTEN_FULLSCREEN_CANVAS_SCALE_NONE: EMSCRIPTEN_FULLSCREEN_CANVAS_SCALE = 0;
pub const EMSCRIPTEN_FULLSCREEN_CANVAS_SCALE_STDDEF: EMSCRIPTEN_FULLSCREEN_CANVAS_SCALE = 1;
pub const EMSCRIPTEN_FULLSCREEN_CANVAS_SCALE_HIDEF: EMSCRIPTEN_FULLSCREEN_CANVAS_SCALE = 2;

/// `EMSCRIPTEN_FULLSCREEN_FILTERING`.
pub type EMSCRIPTEN_FULLSCREEN_FILTERING = c_int;
pub const EMSCRIPTEN_FULLSCREEN_FILTERING_DEFAULT: EMSCRIPTEN_FULLSCREEN_FILTERING = 0;
pub const EMSCRIPTEN_FULLSCREEN_FILTERING_NEAREST: EMSCRIPTEN_FULLSCREEN_FILTERING = 1;
pub const EMSCRIPTEN_FULLSCREEN_FILTERING_BILINEAR: EMSCRIPTEN_FULLSCREEN_FILTERING = 2;

/// `em_canvasresized_callback_func`.
pub type em_canvasresized_callback_func = Option<
    unsafe extern "C" fn(eventType: c_int, reserved: *const c_void, userData: *mut c_void) -> bool,
>;

/// `EmscriptenFullscreenStrategy`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EmscriptenFullscreenStrategy {
    pub scaleMode: EMSCRIPTEN_FULLSCREEN_SCALE,
    pub canvasResolutionScaleMode: EMSCRIPTEN_FULLSCREEN_CANVAS_SCALE,
    pub filteringMode: EMSCRIPTEN_FULLSCREEN_FILTERING,
    pub canvasResizedCallback: em_canvasresized_callback_func,
    pub canvasResizedCallbackUserData: *mut c_void,
    pub canvasResizedCallbackTargetThread: pthread_t,
}

/// `EmscriptenPointerlockChangeEvent`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EmscriptenPointerlockChangeEvent {
    pub isActive: bool,
    pub nodeName: [EM_UTF8; EM_HTML5_LONG_STRING_LEN_BYTES],
    pub id: [EM_UTF8; EM_HTML5_LONG_STRING_LEN_BYTES],
}

/// `em_pointerlockchange_callback_func`.
pub type em_pointerlockchange_callback_func = Option<
    unsafe extern "C" fn(
        eventType: c_int,
        pointerlockChangeEvent: *const EmscriptenPointerlockChangeEvent,
        userData: *mut c_void,
    ) -> bool,
>;

pub const EMSCRIPTEN_VISIBILITY_HIDDEN: c_int = 0;
pub const EMSCRIPTEN_VISIBILITY_VISIBLE: c_int = 1;
pub const EMSCRIPTEN_VISIBILITY_PRERENDER: c_int = 2;
pub const EMSCRIPTEN_VISIBILITY_UNLOADED: c_int = 3;

/// `EmscriptenVisibilityChangeEvent`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EmscriptenVisibilityChangeEvent {
    pub hidden: bool,
    pub visibilityState: c_int,
}

/// `em_visibilitychange_callback_func`.
pub type em_visibilitychange_callback_func = Option<
    unsafe extern "C" fn(
        eventType: c_int,
        visibilityChangeEvent: *const EmscriptenVisibilityChangeEvent,
        userData: *mut c_void,
    ) -> bool,
>;

/// `EmscriptenGamepadEvent`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EmscriptenGamepadEvent {
    pub timestamp: f64,
    pub numAxes: c_int,
    pub numButtons: c_int,
    pub axis: [f64; 64],
    pub analogButton: [f64; 64],
    pub digitalButton: [bool; 64],
    pub connected: bool,
    pub index: c_int,
    pub id: [EM_UTF8; EM_HTML5_MEDIUM_STRING_LEN_BYTES],
    pub mapping: [EM_UTF8; EM_HTML5_MEDIUM_STRING_LEN_BYTES],
}

/// `em_gamepad_callback_func`.
pub type em_gamepad_callback_func = Option<
    unsafe extern "C" fn(
        eventType: c_int,
        gamepadEvent: *const EmscriptenGamepadEvent,
        userData: *mut c_void,
    ) -> bool,
>;

/// `EmscriptenBatteryEvent`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EmscriptenBatteryEvent {
    pub chargingTime: f64,
    pub dischargingTime: f64,
    pub level: f64,
    pub charging: bool,
}

/// `em_beforeunload_callback`: returns the message to show, or null.
pub type em_beforeunload_callback = Option<
    unsafe extern "C" fn(
        eventType: c_int,
        reserved: *const c_void,
        userData: *mut c_void,
    ) -> *const c_char,
>;

/// The thread that started the program (`html5.h`).
pub const EM_CALLBACK_THREAD_CONTEXT_MAIN_RUNTIME_THREAD: pthread_t = 0x1;
/// The thread that sets the callback: the default of the
/// `emscripten_set_*_callback` macros (`html5.h`).
pub const EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD: pthread_t = 0x2;

// ---------------------------------------------------------------------------
// html5_webgl.h

/// `EMSCRIPTEN_WEBGL_CONTEXT_HANDLE` (`uintptr_t`; 0 is none).
pub type EMSCRIPTEN_WEBGL_CONTEXT_HANDLE = usize;

/// `EMSCRIPTEN_WEBGL_CONTEXT_PROXY_MODE`.
pub type EMSCRIPTEN_WEBGL_CONTEXT_PROXY_MODE = c_int;
pub const EMSCRIPTEN_WEBGL_CONTEXT_PROXY_DISALLOW: EMSCRIPTEN_WEBGL_CONTEXT_PROXY_MODE = 0;
pub const EMSCRIPTEN_WEBGL_CONTEXT_PROXY_FALLBACK: EMSCRIPTEN_WEBGL_CONTEXT_PROXY_MODE = 1;
pub const EMSCRIPTEN_WEBGL_CONTEXT_PROXY_ALWAYS: EMSCRIPTEN_WEBGL_CONTEXT_PROXY_MODE = 2;

/// `EM_WEBGL_POWER_PREFERENCE`.
pub type EM_WEBGL_POWER_PREFERENCE = c_int;
pub const EM_WEBGL_POWER_PREFERENCE_DEFAULT: EM_WEBGL_POWER_PREFERENCE = 0;
pub const EM_WEBGL_POWER_PREFERENCE_LOW_POWER: EM_WEBGL_POWER_PREFERENCE = 1;
pub const EM_WEBGL_POWER_PREFERENCE_HIGH_PERFORMANCE: EM_WEBGL_POWER_PREFERENCE = 2;

/// `EmscriptenWebGLContextAttributes`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EmscriptenWebGLContextAttributes {
    pub alpha: bool,
    pub depth: bool,
    pub stencil: bool,
    pub antialias: bool,
    pub premultipliedAlpha: bool,
    pub preserveDrawingBuffer: bool,
    pub powerPreference: EM_WEBGL_POWER_PREFERENCE,
    pub failIfMajorPerformanceCaveat: bool,
    pub majorVersion: c_int,
    pub minorVersion: c_int,
    pub enableExtensionsByDefault: bool,
    pub explicitSwapControl: bool,
    pub proxyContextToMainThread: EMSCRIPTEN_WEBGL_CONTEXT_PROXY_MODE,
    pub renderViaOffscreenBackBuffer: bool,
    pub desynchronized: bool,
}

// ---------------------------------------------------------------------------
// dom_pk_codes.h: the physical key codes emscripten_compute_dom_pk_code()
// gives for a KeyboardEvent's `code`.

/// `DOM_PK_CODE_TYPE`.
pub type DOM_PK_CODE_TYPE = c_int;
/// `Unidentified`
pub const DOM_PK_UNKNOWN: DOM_PK_CODE_TYPE = 0x0000;
/// `Escape`
pub const DOM_PK_ESCAPE: DOM_PK_CODE_TYPE = 0x0001;
/// `Digit0`
pub const DOM_PK_0: DOM_PK_CODE_TYPE = 0x0002;
/// `Digit1`
pub const DOM_PK_1: DOM_PK_CODE_TYPE = 0x0003;
/// `Digit2`
pub const DOM_PK_2: DOM_PK_CODE_TYPE = 0x0004;
/// `Digit3`
pub const DOM_PK_3: DOM_PK_CODE_TYPE = 0x0005;
/// `Digit4`
pub const DOM_PK_4: DOM_PK_CODE_TYPE = 0x0006;
/// `Digit5`
pub const DOM_PK_5: DOM_PK_CODE_TYPE = 0x0007;
/// `Digit6`
pub const DOM_PK_6: DOM_PK_CODE_TYPE = 0x0008;
/// `Digit7`
pub const DOM_PK_7: DOM_PK_CODE_TYPE = 0x0009;
/// `Digit8`
pub const DOM_PK_8: DOM_PK_CODE_TYPE = 0x000A;
/// `Digit9`
pub const DOM_PK_9: DOM_PK_CODE_TYPE = 0x000B;
/// `Minus`
pub const DOM_PK_MINUS: DOM_PK_CODE_TYPE = 0x000C;
/// `Equal`
pub const DOM_PK_EQUAL: DOM_PK_CODE_TYPE = 0x000D;
/// `Backspace`
pub const DOM_PK_BACKSPACE: DOM_PK_CODE_TYPE = 0x000E;
/// `Tab`
pub const DOM_PK_TAB: DOM_PK_CODE_TYPE = 0x000F;
/// `KeyQ`
pub const DOM_PK_Q: DOM_PK_CODE_TYPE = 0x0010;
/// `KeyW`
pub const DOM_PK_W: DOM_PK_CODE_TYPE = 0x0011;
/// `KeyE`
pub const DOM_PK_E: DOM_PK_CODE_TYPE = 0x0012;
/// `KeyR`
pub const DOM_PK_R: DOM_PK_CODE_TYPE = 0x0013;
/// `KeyT`
pub const DOM_PK_T: DOM_PK_CODE_TYPE = 0x0014;
/// `KeyY`
pub const DOM_PK_Y: DOM_PK_CODE_TYPE = 0x0015;
/// `KeyU`
pub const DOM_PK_U: DOM_PK_CODE_TYPE = 0x0016;
/// `KeyI`
pub const DOM_PK_I: DOM_PK_CODE_TYPE = 0x0017;
/// `KeyO`
pub const DOM_PK_O: DOM_PK_CODE_TYPE = 0x0018;
/// `KeyP`
pub const DOM_PK_P: DOM_PK_CODE_TYPE = 0x0019;
/// `BracketLeft`
pub const DOM_PK_BRACKET_LEFT: DOM_PK_CODE_TYPE = 0x001A;
/// `BracketRight`
pub const DOM_PK_BRACKET_RIGHT: DOM_PK_CODE_TYPE = 0x001B;
/// `Enter`
pub const DOM_PK_ENTER: DOM_PK_CODE_TYPE = 0x001C;
/// `ControlLeft`
pub const DOM_PK_CONTROL_LEFT: DOM_PK_CODE_TYPE = 0x001D;
/// `KeyA`
pub const DOM_PK_A: DOM_PK_CODE_TYPE = 0x001E;
/// `KeyS`
pub const DOM_PK_S: DOM_PK_CODE_TYPE = 0x001F;
/// `KeyD`
pub const DOM_PK_D: DOM_PK_CODE_TYPE = 0x0020;
/// `KeyF`
pub const DOM_PK_F: DOM_PK_CODE_TYPE = 0x0021;
/// `KeyG`
pub const DOM_PK_G: DOM_PK_CODE_TYPE = 0x0022;
/// `KeyH`
pub const DOM_PK_H: DOM_PK_CODE_TYPE = 0x0023;
/// `KeyJ`
pub const DOM_PK_J: DOM_PK_CODE_TYPE = 0x0024;
/// `KeyK`
pub const DOM_PK_K: DOM_PK_CODE_TYPE = 0x0025;
/// `KeyL`
pub const DOM_PK_L: DOM_PK_CODE_TYPE = 0x0026;
/// `Semicolon`
pub const DOM_PK_SEMICOLON: DOM_PK_CODE_TYPE = 0x0027;
/// `Quote`
pub const DOM_PK_QUOTE: DOM_PK_CODE_TYPE = 0x0028;
/// `Backquote`
pub const DOM_PK_BACKQUOTE: DOM_PK_CODE_TYPE = 0x0029;
/// `ShiftLeft`
pub const DOM_PK_SHIFT_LEFT: DOM_PK_CODE_TYPE = 0x002A;
/// `Backslash`
pub const DOM_PK_BACKSLASH: DOM_PK_CODE_TYPE = 0x002B;
/// `KeyZ`
pub const DOM_PK_Z: DOM_PK_CODE_TYPE = 0x002C;
/// `KeyX`
pub const DOM_PK_X: DOM_PK_CODE_TYPE = 0x002D;
/// `KeyC`
pub const DOM_PK_C: DOM_PK_CODE_TYPE = 0x002E;
/// `KeyV`
pub const DOM_PK_V: DOM_PK_CODE_TYPE = 0x002F;
/// `KeyB`
pub const DOM_PK_B: DOM_PK_CODE_TYPE = 0x0030;
/// `KeyN`
pub const DOM_PK_N: DOM_PK_CODE_TYPE = 0x0031;
/// `KeyM`
pub const DOM_PK_M: DOM_PK_CODE_TYPE = 0x0032;
/// `Comma`
pub const DOM_PK_COMMA: DOM_PK_CODE_TYPE = 0x0033;
/// `Period`
pub const DOM_PK_PERIOD: DOM_PK_CODE_TYPE = 0x0034;
/// `Slash`
pub const DOM_PK_SLASH: DOM_PK_CODE_TYPE = 0x0035;
/// `ShiftRight`
pub const DOM_PK_SHIFT_RIGHT: DOM_PK_CODE_TYPE = 0x0036;
/// `NumpadMultiply`
pub const DOM_PK_NUMPAD_MULTIPLY: DOM_PK_CODE_TYPE = 0x0037;
/// `AltLeft`
pub const DOM_PK_ALT_LEFT: DOM_PK_CODE_TYPE = 0x0038;
/// `Space`
pub const DOM_PK_SPACE: DOM_PK_CODE_TYPE = 0x0039;
/// `CapsLock`
pub const DOM_PK_CAPS_LOCK: DOM_PK_CODE_TYPE = 0x003A;
/// `F1`
pub const DOM_PK_F1: DOM_PK_CODE_TYPE = 0x003B;
/// `F2`
pub const DOM_PK_F2: DOM_PK_CODE_TYPE = 0x003C;
/// `F3`
pub const DOM_PK_F3: DOM_PK_CODE_TYPE = 0x003D;
/// `F4`
pub const DOM_PK_F4: DOM_PK_CODE_TYPE = 0x003E;
/// `F5`
pub const DOM_PK_F5: DOM_PK_CODE_TYPE = 0x003F;
/// `F6`
pub const DOM_PK_F6: DOM_PK_CODE_TYPE = 0x0040;
/// `F7`
pub const DOM_PK_F7: DOM_PK_CODE_TYPE = 0x0041;
/// `F8`
pub const DOM_PK_F8: DOM_PK_CODE_TYPE = 0x0042;
/// `F9`
pub const DOM_PK_F9: DOM_PK_CODE_TYPE = 0x0043;
/// `F10`
pub const DOM_PK_F10: DOM_PK_CODE_TYPE = 0x0044;
/// `Pause`
pub const DOM_PK_PAUSE: DOM_PK_CODE_TYPE = 0x0045;
/// `ScrollLock`
pub const DOM_PK_SCROLL_LOCK: DOM_PK_CODE_TYPE = 0x0046;
/// `Numpad7`
pub const DOM_PK_NUMPAD_7: DOM_PK_CODE_TYPE = 0x0047;
/// `Numpad8`
pub const DOM_PK_NUMPAD_8: DOM_PK_CODE_TYPE = 0x0048;
/// `Numpad9`
pub const DOM_PK_NUMPAD_9: DOM_PK_CODE_TYPE = 0x0049;
/// `NumpadSubtract`
pub const DOM_PK_NUMPAD_SUBTRACT: DOM_PK_CODE_TYPE = 0x004A;
/// `Numpad4`
pub const DOM_PK_NUMPAD_4: DOM_PK_CODE_TYPE = 0x004B;
/// `Numpad5`
pub const DOM_PK_NUMPAD_5: DOM_PK_CODE_TYPE = 0x004C;
/// `Numpad6`
pub const DOM_PK_NUMPAD_6: DOM_PK_CODE_TYPE = 0x004D;
/// `NumpadAdd`
pub const DOM_PK_NUMPAD_ADD: DOM_PK_CODE_TYPE = 0x004E;
/// `Numpad1`
pub const DOM_PK_NUMPAD_1: DOM_PK_CODE_TYPE = 0x004F;
/// `Numpad2`
pub const DOM_PK_NUMPAD_2: DOM_PK_CODE_TYPE = 0x0050;
/// `Numpad3`
pub const DOM_PK_NUMPAD_3: DOM_PK_CODE_TYPE = 0x0051;
/// `Numpad0`
pub const DOM_PK_NUMPAD_0: DOM_PK_CODE_TYPE = 0x0052;
/// `NumpadDecimal`
pub const DOM_PK_NUMPAD_DECIMAL: DOM_PK_CODE_TYPE = 0x0053;
/// `PrintScreen`
pub const DOM_PK_PRINT_SCREEN: DOM_PK_CODE_TYPE = 0x0054;
/// `IntlBackslash`
pub const DOM_PK_INTL_BACKSLASH: DOM_PK_CODE_TYPE = 0x0056;
/// `F11`
pub const DOM_PK_F11: DOM_PK_CODE_TYPE = 0x0057;
/// `F12`
pub const DOM_PK_F12: DOM_PK_CODE_TYPE = 0x0058;
/// `NumpadEqual`
pub const DOM_PK_NUMPAD_EQUAL: DOM_PK_CODE_TYPE = 0x0059;
/// `F13`
pub const DOM_PK_F13: DOM_PK_CODE_TYPE = 0x0064;
/// `F14`
pub const DOM_PK_F14: DOM_PK_CODE_TYPE = 0x0065;
/// `F15`
pub const DOM_PK_F15: DOM_PK_CODE_TYPE = 0x0066;
/// `F16`
pub const DOM_PK_F16: DOM_PK_CODE_TYPE = 0x0067;
/// `F17`
pub const DOM_PK_F17: DOM_PK_CODE_TYPE = 0x0068;
/// `F18`
pub const DOM_PK_F18: DOM_PK_CODE_TYPE = 0x0069;
/// `F19`
pub const DOM_PK_F19: DOM_PK_CODE_TYPE = 0x006A;
/// `F20`
pub const DOM_PK_F20: DOM_PK_CODE_TYPE = 0x006B;
/// `F21`
pub const DOM_PK_F21: DOM_PK_CODE_TYPE = 0x006C;
/// `F22`
pub const DOM_PK_F22: DOM_PK_CODE_TYPE = 0x006D;
/// `F23`
pub const DOM_PK_F23: DOM_PK_CODE_TYPE = 0x006E;
/// `KanaMode`
pub const DOM_PK_KANA_MODE: DOM_PK_CODE_TYPE = 0x0070;
/// `Lang2`
pub const DOM_PK_LANG_2: DOM_PK_CODE_TYPE = 0x0071;
/// `Lang1`
pub const DOM_PK_LANG_1: DOM_PK_CODE_TYPE = 0x0072;
/// `IntlRo`
pub const DOM_PK_INTL_RO: DOM_PK_CODE_TYPE = 0x0073;
/// `F24`
pub const DOM_PK_F24: DOM_PK_CODE_TYPE = 0x0076;
/// `Convert`
pub const DOM_PK_CONVERT: DOM_PK_CODE_TYPE = 0x0079;
/// `NonConvert`
pub const DOM_PK_NON_CONVERT: DOM_PK_CODE_TYPE = 0x007B;
/// `IntlYen`
pub const DOM_PK_INTL_YEN: DOM_PK_CODE_TYPE = 0x007D;
/// `NumpadComma`
pub const DOM_PK_NUMPAD_COMMA: DOM_PK_CODE_TYPE = 0x007E;
/// `Paste`
pub const DOM_PK_PASTE: DOM_PK_CODE_TYPE = 0xE00A;
/// `MediaTrackPrevious`
pub const DOM_PK_MEDIA_TRACK_PREVIOUS: DOM_PK_CODE_TYPE = 0xE010;
/// `Cut`
pub const DOM_PK_CUT: DOM_PK_CODE_TYPE = 0xE017;
/// `Copy`
pub const DOM_PK_COPY: DOM_PK_CODE_TYPE = 0xE018;
/// `MediaTrackNext`
pub const DOM_PK_MEDIA_TRACK_NEXT: DOM_PK_CODE_TYPE = 0xE019;
/// `NumpadEnter`
pub const DOM_PK_NUMPAD_ENTER: DOM_PK_CODE_TYPE = 0xE01C;
/// `ControlRight`
pub const DOM_PK_CONTROL_RIGHT: DOM_PK_CODE_TYPE = 0xE01D;
/// `AudioVolumeMute` or `VolumeMute`
pub const DOM_PK_AUDIO_VOLUME_MUTE: DOM_PK_CODE_TYPE = 0xE020;
/// `LaunchApp2`
pub const DOM_PK_LAUNCH_APP_2: DOM_PK_CODE_TYPE = 0xE021;
/// `MediaPlayPause`
pub const DOM_PK_MEDIA_PLAY_PAUSE: DOM_PK_CODE_TYPE = 0xE022;
/// `MediaStop`
pub const DOM_PK_MEDIA_STOP: DOM_PK_CODE_TYPE = 0xE024;
/// `Eject`
pub const DOM_PK_EJECT: DOM_PK_CODE_TYPE = 0xE02C;
/// `AudioVolumeDown` or `VolumeDown`
pub const DOM_PK_AUDIO_VOLUME_DOWN: DOM_PK_CODE_TYPE = 0xE02E;
/// `AudioVolumeUp` or `VolumeUp`
pub const DOM_PK_AUDIO_VOLUME_UP: DOM_PK_CODE_TYPE = 0xE030;
/// `BrowserHome`
pub const DOM_PK_BROWSER_HOME: DOM_PK_CODE_TYPE = 0xE032;
/// `NumpadDivide`
pub const DOM_PK_NUMPAD_DIVIDE: DOM_PK_CODE_TYPE = 0xE035;
/// `AltRight`
pub const DOM_PK_ALT_RIGHT: DOM_PK_CODE_TYPE = 0xE038;
/// `Help`
pub const DOM_PK_HELP: DOM_PK_CODE_TYPE = 0xE03B;
/// `NumLock`
pub const DOM_PK_NUM_LOCK: DOM_PK_CODE_TYPE = 0xE045;
/// `Home`
pub const DOM_PK_HOME: DOM_PK_CODE_TYPE = 0xE047;
/// `ArrowUp`
pub const DOM_PK_ARROW_UP: DOM_PK_CODE_TYPE = 0xE048;
/// `PageUp`
pub const DOM_PK_PAGE_UP: DOM_PK_CODE_TYPE = 0xE049;
/// `ArrowLeft`
pub const DOM_PK_ARROW_LEFT: DOM_PK_CODE_TYPE = 0xE04B;
/// `ArrowRight`
pub const DOM_PK_ARROW_RIGHT: DOM_PK_CODE_TYPE = 0xE04D;
/// `End`
pub const DOM_PK_END: DOM_PK_CODE_TYPE = 0xE04F;
/// `ArrowDown`
pub const DOM_PK_ARROW_DOWN: DOM_PK_CODE_TYPE = 0xE050;
/// `PageDown`
pub const DOM_PK_PAGE_DOWN: DOM_PK_CODE_TYPE = 0xE051;
/// `Insert`
pub const DOM_PK_INSERT: DOM_PK_CODE_TYPE = 0xE052;
/// `Delete`
pub const DOM_PK_DELETE: DOM_PK_CODE_TYPE = 0xE053;
/// `MetaLeft`
pub const DOM_PK_META_LEFT: DOM_PK_CODE_TYPE = 0xE05B;
/// `OSLeft`
pub const DOM_PK_OS_LEFT: DOM_PK_CODE_TYPE = 0xE05B;
/// `MetaRight`
pub const DOM_PK_META_RIGHT: DOM_PK_CODE_TYPE = 0xE05C;
/// `OSRight`
pub const DOM_PK_OS_RIGHT: DOM_PK_CODE_TYPE = 0xE05C;
/// `ContextMenu`
pub const DOM_PK_CONTEXT_MENU: DOM_PK_CODE_TYPE = 0xE05D;
/// `Power`
pub const DOM_PK_POWER: DOM_PK_CODE_TYPE = 0xE05E;
/// `BrowserSearch`
pub const DOM_PK_BROWSER_SEARCH: DOM_PK_CODE_TYPE = 0xE065;
/// `BrowserFavorites`
pub const DOM_PK_BROWSER_FAVORITES: DOM_PK_CODE_TYPE = 0xE066;
/// `BrowserRefresh`
pub const DOM_PK_BROWSER_REFRESH: DOM_PK_CODE_TYPE = 0xE067;
/// `BrowserStop`
pub const DOM_PK_BROWSER_STOP: DOM_PK_CODE_TYPE = 0xE068;
/// `BrowserForward`
pub const DOM_PK_BROWSER_FORWARD: DOM_PK_CODE_TYPE = 0xE069;
/// `BrowserBack`
pub const DOM_PK_BROWSER_BACK: DOM_PK_CODE_TYPE = 0xE06A;
/// `LaunchApp1`
pub const DOM_PK_LAUNCH_APP_1: DOM_PK_CODE_TYPE = 0xE06B;
/// `LaunchMail`
pub const DOM_PK_LAUNCH_MAIL: DOM_PK_CODE_TYPE = 0xE06C;
/// `LaunchMediaPlayer`
pub const DOM_PK_LAUNCH_MEDIA_PLAYER: DOM_PK_CODE_TYPE = 0xE06D;
/// `MediaSelect`
pub const DOM_PK_MEDIA_SELECT: DOM_PK_CODE_TYPE = 0xE06D;

// ---------------------------------------------------------------------------
// The functions

// These two end in a JavaScript exception ('unwind', ExitStatus) that leaves
// the Rust frames for the JavaScript that called into the program (which
// handles it): so "C-unwind".
extern "C-unwind" {
    // emscripten.h
    pub fn emscripten_exit_with_live_runtime() -> !;
    pub fn emscripten_force_exit(status: c_int) -> !;
}

extern "C" {
    // emscripten.h
    pub fn emscripten_set_main_loop(
        func: em_callback_func,
        fps: c_int,
        simulate_infinite_loop: bool,
    );
    pub fn emscripten_set_main_loop_timing(mode: c_int, value: c_int) -> c_int;
    /// Either pointer may be null.
    pub fn emscripten_get_main_loop_timing(mode: *mut c_int, value: *mut c_int);
    pub fn emscripten_cancel_main_loop();
    pub fn emscripten_get_device_pixel_ratio() -> f64;
    pub fn emscripten_set_window_title(title: *const c_char);
    pub fn emscripten_get_screen_size(width: *mut c_int, height: *mut c_int);
    pub fn emscripten_has_asyncify() -> c_int;
    pub fn emscripten_sleep(ms: c_uint);

    // eventloop.h
    pub fn emscripten_set_timeout(
        cb: unsafe extern "C-unwind" fn(user_data: *mut c_void),
        msecs: f64,
        user_data: *mut c_void,
    ) -> c_int;
    pub fn emscripten_clear_timeout(id: c_int);

    // html5.h: event callbacks
    pub fn emscripten_set_keypress_callback_on_thread(
        target: *const c_char,
        userData: *mut c_void,
        useCapture: bool,
        callback: em_key_callback_func,
        targetThread: pthread_t,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_set_keydown_callback_on_thread(
        target: *const c_char,
        userData: *mut c_void,
        useCapture: bool,
        callback: em_key_callback_func,
        targetThread: pthread_t,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_set_keyup_callback_on_thread(
        target: *const c_char,
        userData: *mut c_void,
        useCapture: bool,
        callback: em_key_callback_func,
        targetThread: pthread_t,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_set_wheel_callback_on_thread(
        target: *const c_char,
        userData: *mut c_void,
        useCapture: bool,
        callback: em_wheel_callback_func,
        targetThread: pthread_t,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_set_resize_callback_on_thread(
        target: *const c_char,
        userData: *mut c_void,
        useCapture: bool,
        callback: em_ui_callback_func,
        targetThread: pthread_t,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_set_blur_callback_on_thread(
        target: *const c_char,
        userData: *mut c_void,
        useCapture: bool,
        callback: em_focus_callback_func,
        targetThread: pthread_t,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_set_focus_callback_on_thread(
        target: *const c_char,
        userData: *mut c_void,
        useCapture: bool,
        callback: em_focus_callback_func,
        targetThread: pthread_t,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_set_devicemotion_callback_on_thread(
        userData: *mut c_void,
        useCapture: bool,
        callback: em_devicemotion_callback_func,
        targetThread: pthread_t,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_set_orientationchange_callback_on_thread(
        userData: *mut c_void,
        useCapture: bool,
        callback: em_orientationchange_callback_func,
        targetThread: pthread_t,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_set_fullscreenchange_callback_on_thread(
        target: *const c_char,
        userData: *mut c_void,
        useCapture: bool,
        callback: em_fullscreenchange_callback_func,
        targetThread: pthread_t,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_set_pointerlockchange_callback_on_thread(
        target: *const c_char,
        userData: *mut c_void,
        useCapture: bool,
        callback: em_pointerlockchange_callback_func,
        targetThread: pthread_t,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_set_visibilitychange_callback_on_thread(
        userData: *mut c_void,
        useCapture: bool,
        callback: em_visibilitychange_callback_func,
        targetThread: pthread_t,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_set_gamepadconnected_callback_on_thread(
        userData: *mut c_void,
        useCapture: bool,
        callback: em_gamepad_callback_func,
        targetThread: pthread_t,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_set_gamepaddisconnected_callback_on_thread(
        userData: *mut c_void,
        useCapture: bool,
        callback: em_gamepad_callback_func,
        targetThread: pthread_t,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_set_beforeunload_callback_on_thread(
        userData: *mut c_void,
        callback: em_beforeunload_callback,
        targetThread: pthread_t,
    ) -> EMSCRIPTEN_RESULT;

    // html5.h: state
    pub fn emscripten_get_fullscreen_status(
        fullscreenStatus: *mut EmscriptenFullscreenChangeEvent,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_request_fullscreen_strategy(
        target: *const c_char,
        deferUntilInEventHandler: bool,
        fullscreenStrategy: *const EmscriptenFullscreenStrategy,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_exit_fullscreen() -> EMSCRIPTEN_RESULT;
    pub fn emscripten_request_pointerlock(
        target: *const c_char,
        deferUntilInEventHandler: bool,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_exit_pointerlock() -> EMSCRIPTEN_RESULT;
    pub fn emscripten_sample_gamepad_data() -> EMSCRIPTEN_RESULT;
    pub fn emscripten_get_num_gamepads() -> c_int;
    pub fn emscripten_get_gamepad_status(
        index: c_int,
        gamepadState: *mut EmscriptenGamepadEvent,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_get_battery_status(
        batteryState: *mut EmscriptenBatteryEvent,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_set_canvas_element_size(
        target: *const c_char,
        width: c_int,
        height: c_int,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_get_canvas_element_size(
        target: *const c_char,
        width: *mut c_int,
        height: *mut c_int,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_set_element_css_size(
        target: *const c_char,
        width: f64,
        height: f64,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_get_element_css_size(
        target: *const c_char,
        width: *mut f64,
        height: *mut f64,
    ) -> EMSCRIPTEN_RESULT;

    // html5_webgl.h
    pub fn emscripten_webgl_init_context_attributes(
        attributes: *mut EmscriptenWebGLContextAttributes,
    );
    pub fn emscripten_webgl_create_context(
        target: *const c_char,
        attributes: *const EmscriptenWebGLContextAttributes,
    ) -> EMSCRIPTEN_WEBGL_CONTEXT_HANDLE;
    pub fn emscripten_webgl_make_context_current(
        context: EMSCRIPTEN_WEBGL_CONTEXT_HANDLE,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_webgl_destroy_context(
        context: EMSCRIPTEN_WEBGL_CONTEXT_HANDLE,
    ) -> EMSCRIPTEN_RESULT;
    pub fn emscripten_webgl_get_proc_address(name: *const c_char) -> *mut c_void;

    // dom_pk_codes.h
    pub fn emscripten_compute_dom_pk_code(keyCodeString: *const c_char) -> DOM_PK_CODE_TYPE;
}
