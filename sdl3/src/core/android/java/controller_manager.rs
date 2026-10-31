// Rust translation of android-project/app/src/main/java/org/libsdl/app/
// SDLControllerManager.java from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see
// LICENSE.txt.

//! `class SDLControllerManager`, which backs the joystick and haptic
//! drivers, and the other classes of its file:
//!
//! * the controller manager's statics: `initialize()` (from
//!   `SDL.initialize()`: the joystick handler, and the haptic handler of
//!   the device's API level), `initializeDeviceListener()` (from the
//!   activity's `onCreate()`), the calls SDL makes through JNI
//!   (`detectDevices()`, `joystickSetLED()`, `joystickSetSensorsEnabled()`,
//!   `detectHapticDevices()`, `hapticRun()`, `hapticRumble()`,
//!   `hapticStop()`), `isDeviceSDLJoystick()` (whether a device's keys are a
//!   game controller's, `SDLActivity.handleKeyEvent()` asks), and its
//!   natives (`onNativePadDown()`, ..., `nativeAddJoystick()`, ...);
//! * `SDLDeviceListener implements InputDeviceListener`: the input
//!   devices added, changed and removed, passed to the joystick handler;
//! * `SDLJoystickHandler`: the game controllers among the input devices
//!   (`InputDevice.getDeviceIds()`), each with its axes (the joystick
//!   class's motion ranges, sorted as SDL's gamepad axes go: X, Y, RX, RY,
//!   Z, RZ, the triggers), hats, buttons (`hasKeys()`), and, on API 31 and
//!   later, its vibrators, RGB lights and motion sensors; its joystick
//!   motion events (`handleMotionEvent()`), the LED and the sensors;
//! * `SDLHapticHandler`, `SDLHapticHandler_API26` and
//!   `SDLHapticHandler_API31`: the system's vibrator, run with a length
//!   (and an amplitude from API 26), and a controller's vibrators rumbled
//!   (API 31);
//! * the generic motion listeners, `class SDLGenericMotionListener_API14
//!   implements View.OnGenericMotionListener` and its subclasses
//!   `SDLGenericMotionListener_API24`, `_API26` and `_API29`: the surface's
//!   listener of the motion events that aren't touches (the mouse hovering
//!   and its wheel, the stylus hovering; a joystick's, which go to the
//!   joystick handler), and of the captured pointer's (the relative mouse
//!   mode, `SDLSurface.onCapturedPointerEvent()`), each newer one with more
//!   of the relative mode: none before API 24, the mouse's relative axes
//!   from API 24, the pointer captured from API 26 (the content view's
//!   `requestPointerCapture()`), and the pen's device type from API 29.
//!   `SDLActivity.getMotionListener()` makes the one of the device's API
//!   level;
//! * `SDLJoySensorListener implements SensorEventListener`: a controller's
//!   accelerometer and gyroscope events, passed to `onNativeJoySensor()`.
//!
//! The classes the framework calls are the DEX file's
//! (`super::super::classes`): `SDLControllerManager` (its natives, and the
//! statics SDL calls declared native too, so that Java can call them),
//! `SDLDeviceListener`, `SDLJoySensorListener` (with its `device_id`
//! field), and the motion listeners (the framework calls
//! `onGenericMotion()`, which is native), with their instance fields
//! (`mRelativeModeEnabled`, which API 24's and API 26's each have: API
//! 26's shadows API 24's); their other methods are Rust functions, which
//! dispatch on the listener's class as Java's virtual calls do ([`Api`]).
//! The joystick and haptic handlers aren't the framework's: they are Rust
//! (their lists behind a mutex, and a reentrant one for the monitor their
//! `synchronized` methods take); the haptic handler's subclasses are a
//! [`HapticApi`] of it. A joystick's motion ranges are kept as their axes,
//! sources, minimums and ranges (final fields), read when it is added.

use std::ffi::CStr;
use std::sync::{Arc, Mutex, OnceLock};

use super::super::env::{Arg, Env, Global, JniResult, Local, Obj};
use super::super::jni::*;
use super::{activity, sdl};
use crate::thread::ReentrantMutex;

use super::super::classes::{
    SDL_DEVICE_LISTENER, SDL_GENERIC_MOTION_LISTENER_API14, SDL_GENERIC_MOTION_LISTENER_API24,
    SDL_GENERIC_MOTION_LISTENER_API26, SDL_GENERIC_MOTION_LISTENER_API29, SDL_JOY_SENSOR_LISTENER,
};

/// `MotionEvent.ACTION_*`.
pub(crate) const ACTION_DOWN: jint = 0;
pub(crate) const ACTION_UP: jint = 1;
pub(crate) const ACTION_MOVE: jint = 2;
pub(crate) const ACTION_CANCEL: jint = 3;
pub(crate) const ACTION_POINTER_DOWN: jint = 5;
pub(crate) const ACTION_POINTER_UP: jint = 6;
pub(crate) const ACTION_HOVER_MOVE: jint = 7;
pub(crate) const ACTION_SCROLL: jint = 8;
pub(crate) const ACTION_HOVER_ENTER: jint = 9;
pub(crate) const ACTION_HOVER_EXIT: jint = 10;

/// `MotionEvent.TOOL_TYPE_*`.
pub(crate) const TOOL_TYPE_STYLUS: jint = 2;
pub(crate) const TOOL_TYPE_MOUSE: jint = 3;
pub(crate) const TOOL_TYPE_ERASER: jint = 4;

/// `MotionEvent.AXIS_*`.
const AXIS_VSCROLL: jint = 9;
const AXIS_HSCROLL: jint = 10;
const AXIS_RELATIVE_X: jint = 27;
const AXIS_RELATIVE_Y: jint = 28;

/// `MotionEvent.BUTTON_TERTIARY`.
pub(crate) const BUTTON_TERTIARY: jint = 4;

/// `InputDevice.SOURCE_JOYSTICK`, `SOURCE_MOUSE_RELATIVE`.
const SOURCE_JOYSTICK: jint = 0x0100_0010;
const SOURCE_MOUSE_RELATIVE: jint = 0x0002_0004;

/// `SDL_PEN_DEVICE_TYPE_UNKNOWN`, `_DIRECT`, `_INDIRECT` (the listener's
/// constants).
const SDL_PEN_DEVICE_TYPE_UNKNOWN: jint = 0;
const SDL_PEN_DEVICE_TYPE_DIRECT: jint = 1;
const SDL_PEN_DEVICE_TYPE_INDIRECT: jint = 2;

/// Which of the listener classes an object is: the class whose methods a
/// virtual call runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Api {
    Api14,
    Api24,
    Api26,
    Api29,
}

impl Api {
    /// The class of the API level's listener (`getMotionListener()`'s).
    fn class_name(self) -> &'static str {
        match self {
            Api::Api14 => SDL_GENERIC_MOTION_LISTENER_API14,
            Api::Api24 => SDL_GENERIC_MOTION_LISTENER_API24,
            Api::Api26 => SDL_GENERIC_MOTION_LISTENER_API26,
            Api::Api29 => SDL_GENERIC_MOTION_LISTENER_API29,
        }
    }

    /// The listener's class (the most derived one it is an instance of).
    fn of(env: Env<'_>, listener: Obj<'_>) -> Api {
        for api in [Api::Api29, Api::Api26, Api::Api24] {
            let class = super::super::env::app_class(api.class_name());
            if class.is_some_and(|class| env.is_instance_of(listener, class.obj())) {
                return api;
            }
        }
        Api::Api14
    }
}

/// `new SDLGenericMotionListener_APIxx()`.
pub(crate) fn new_listener<'a>(env: Env<'a>, api: Api) -> JniResult<'a, Local<'a>> {
    let class = super::super::env::app_class(api.class_name())
        .expect("JNI_OnLoad() found the motion listeners");
    env.new_object(class.obj(), c"()V", &[])
}

/// `mRelativeModeEnabled`: the listener's class's own (API 26's shadows
/// API 24's, and the methods reading it are those of the listener's class:
/// the field of its class is found first).
fn relative_mode_enabled<'a>(env: Env<'a>, listener: Obj<'_>) -> JniResult<'a, bool> {
    Ok(env
        .get_field(listener, c"mRelativeModeEnabled", c"Z")?
        .boolean())
}

fn set_relative_mode_enabled<'a>(
    env: Env<'a>,
    listener: Obj<'_>,
    enabled: bool,
) -> JniResult<'a, ()> {
    env.set_field(
        listener,
        c"mRelativeModeEnabled",
        c"Z",
        Arg::Boolean(enabled),
    )
}

fn source<'a>(env: Env<'a>, event: Obj<'_>) -> JniResult<'a, jint> {
    Ok(env.call_method(event, c"getSource", c"()I", &[])?.int())
}

pub(crate) fn get_x<'a>(env: Env<'a>, event: Obj<'_>, i: jint) -> JniResult<'a, f32> {
    Ok(env
        .call_method(event, c"getX", c"(I)F", &[Arg::Int(i)])?
        .float())
}

pub(crate) fn get_y<'a>(env: Env<'a>, event: Obj<'_>, i: jint) -> JniResult<'a, f32> {
    Ok(env
        .call_method(event, c"getY", c"(I)F", &[Arg::Int(i)])?
        .float())
}

/// `event.getPressure(i)`, at most 1: it may be larger than 1.0f on some
/// devices (see the documentation of getPressure(i)).
pub(crate) fn get_pressure<'a>(env: Env<'a>, event: Obj<'_>, i: jint) -> JniResult<'a, f32> {
    let mut p = env
        .call_method(event, c"getPressure", c"(I)F", &[Arg::Int(i)])?
        .float();
    if p > 1.0 {
        // may be larger than 1.0f on some devices
        // see the documentation of getPressure(i)
        p = 1.0;
    }
    Ok(p)
}

pub(crate) fn get_tool_type<'a>(env: Env<'a>, event: Obj<'_>, i: jint) -> JniResult<'a, jint> {
    Ok(env
        .call_method(event, c"getToolType", c"(I)I", &[Arg::Int(i)])?
        .int())
}

pub(crate) fn get_pointer_id<'a>(env: Env<'a>, event: Obj<'_>, i: jint) -> JniResult<'a, jint> {
    Ok(env
        .call_method(event, c"getPointerId", c"(I)I", &[Arg::Int(i)])?
        .int())
}

pub(crate) fn get_button_state<'a>(env: Env<'a>, event: Obj<'_>) -> JniResult<'a, jint> {
    Ok(env
        .call_method(event, c"getButtonState", c"()I", &[])?
        .int())
}

fn get_axis_value<'a>(env: Env<'a>, event: Obj<'_>, axis: jint, i: jint) -> JniResult<'a, f32> {
    Ok(env
        .call_method(
            event,
            c"getAxisValue",
            c"(II)F",
            &[Arg::Int(axis), Arg::Int(i)],
        )?
        .float())
}

/// `event.getDevice()`.
pub(crate) fn get_device<'a>(env: Env<'a>, event: Obj<'_>) -> JniResult<'a, Option<Local<'a>>> {
    Ok(env
        .call_method(event, c"getDevice", c"()Landroid/view/InputDevice;", &[])?
        .object())
}

/// The pen's buttons for `onNativePen()`: BUTTON_STYLUS_PRIMARY is 2^5, so
/// shift by 4, and apply SDL_PEN_INPUT_DOWN/SDL_PEN_INPUT_ERASER_TIP (and
/// the tertiary button as the third). Upstream's `onGenericMotion()` and
/// `SDLSurface.onTouch()` each have this code.
pub(crate) fn pen_buttons<'a>(
    env: Env<'a>,
    event: Obj<'_>,
    tool_type: jint,
) -> JniResult<'a, jint> {
    let button_state = get_button_state(env, event)?;
    // BUTTON_STYLUS_PRIMARY is 2^5, so shift by 4, and apply SDL_PEN_INPUT_DOWN/SDL_PEN_INPUT_ERASER_TIP
    let mut buttons =
        (button_state >> 4) | (1 << if tool_type == TOOL_TYPE_STYLUS { 0 } else { 30 });
    if (button_state & BUTTON_TERTIARY) != 0 {
        buttons |= 0x08;
    }
    Ok(buttons)
}

/// Generic Motion (mouse hover, joystick...) events go here. Translation
/// of `SDLGenericMotionListener_API14.onGenericMotion()` (which the
/// others inherit).
pub(crate) extern "system" fn on_generic_motion(
    env: *mut JNIEnv,
    this: jobject,
    v: jobject,
    event: jobject,
) -> jboolean {
    // SAFETY: a native method's environment and arguments.
    let (env, this, v, event) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(this),
            Obj::from_raw(v),
            Obj::from_raw(event),
        )
    };
    match generic_motion(env, this, v, event) {
        Ok(consumed) => consumed as jboolean,
        Err(thrown) => {
            // (pending, for the framework)
            drop(thrown);
            JNI_FALSE
        }
    }
}

/// The body of [`on_generic_motion`] (`SDLSurface.onCapturedPointerEvent()`
/// calls it too).
pub(crate) fn generic_motion<'a>(
    env: Env<'a>,
    this: Obj<'_>,
    _v: Obj<'_>,
    event: Obj<'_>,
) -> JniResult<'a, bool> {
    if source(env, event)? == SOURCE_JOYSTICK {
        return handle_joystick_motion_event(env, event);
    }

    let action = env
        .call_method(event, c"getActionMasked", c"()I", &[])?
        .int();
    let pointer_count = env
        .call_method(event, c"getPointerCount", c"()I", &[])?
        .int();
    let mut consumed = false;

    for i in 0..pointer_count {
        let tool_type = get_tool_type(env, event, i)?;

        if tool_type == TOOL_TYPE_MOUSE {
            match action {
                ACTION_SCROLL => {
                    let x = get_axis_value(env, event, AXIS_HSCROLL, i)?;
                    let y = get_axis_value(env, event, AXIS_VSCROLL, i)?;
                    super::super::on_mouse(0, action, x, y, false);
                    consumed = true;
                }

                ACTION_HOVER_MOVE => {
                    let x = get_event_x(env, this, event, i)?;
                    let y = get_event_y(env, this, event, i)?;

                    let relative = check_relative_event(env, this, event)?;
                    super::super::on_mouse(0, action, x, y, relative);
                    consumed = true;
                }

                // Note (upstream): the other actions are left, the captured
                // pointer's buttons' too (its ACTION_DOWN, ACTION_MOVE and
                // ACTION_UP, from onCapturedPointerEvent()): in the relative
                // mode, the mouse's buttons and its motion with one pressed
                // send nothing.
                _ => {}
            }
        } else if tool_type == TOOL_TYPE_STYLUS || tool_type == TOOL_TYPE_ERASER {
            if let ACTION_HOVER_ENTER | ACTION_HOVER_MOVE | ACTION_HOVER_EXIT = action {
                let x = get_x(env, event, i)?;
                let y = get_y(env, event, i)?;
                let p = get_pressure(env, event, i)?;

                let buttons = pen_buttons(env, event, tool_type)?;

                let pointer_id = get_pointer_id(env, event, i)?;
                let device = get_device(env, event)?;
                let device_type =
                    get_pen_device_type(env, this, device.as_ref().map_or(Obj::NULL, Local::obj))?;
                super::super::on_pen(pointer_id, device_type, buttons, action, x, y, p);
                consumed = true;
            }
        }
    }

    Ok(consumed)
}

/// Translation of `supportsRelativeMouse()` (API 14's, 24's and 26's).
pub(crate) fn supports_relative_mouse<'a>(env: Env<'a>, this: Obj<'_>) -> JniResult<'a, bool> {
    Ok(match Api::of(env, this) {
        Api::Api14 => false,
        Api::Api24 => true,
        Api::Api26 | Api::Api29 => {
            !activity::is_dex_mode(env)? || super::super::sdk_int(env) >= 27
            /* Android 8.1 (O_MR1) */
        }
    })
}

/// Translation of `inRelativeMode()` (API 14's, 24's and 26's).
pub(crate) fn in_relative_mode<'a>(env: Env<'a>, this: Obj<'_>) -> JniResult<'a, bool> {
    match Api::of(env, this) {
        Api::Api14 => Ok(false),
        Api::Api24 => relative_mode_enabled(env, this),
        Api::Api26 | Api::Api29 => relative_mode_enabled(env, this),
    }
}

/// Translation of `setRelativeMouseEnabled()` (API 14's, 24's and 26's).
pub(crate) fn set_relative_mouse_enabled<'a>(
    env: Env<'a>,
    this: Obj<'_>,
    enabled: bool,
) -> JniResult<'a, bool> {
    match Api::of(env, this) {
        Api::Api14 => Ok(false),
        Api::Api24 => {
            set_relative_mode_enabled(env, this, enabled)?;
            Ok(true)
        }
        Api::Api26 | Api::Api29 => {
            if super::super::sdk_int(env) < 26
            /* Android 8.0 (O) */
            {
                /* Silence 'lint' warning */
                return Ok(false);
            }

            if !activity::is_dex_mode(env)? || super::super::sdk_int(env) >= 27
            /* Android 8.1 (O_MR1) */
            {
                if let Some(content_view) = activity::get_content_view(env) {
                    if enabled {
                        env.call_method(content_view.obj(), c"requestPointerCapture", c"()V", &[])?;
                    } else {
                        env.call_method(content_view.obj(), c"releasePointerCapture", c"()V", &[])?;
                    }
                }
                set_relative_mode_enabled(env, this, enabled)?;
                Ok(true)
            } else {
                Ok(false)
            }
        }
    }
}

/// Translation of `reclaimRelativeMouseModeIfNeeded()` (API 14's and
/// 26's).
pub(crate) fn reclaim_relative_mouse_mode_if_needed<'a>(
    env: Env<'a>,
    this: Obj<'_>,
) -> JniResult<'a, ()> {
    match Api::of(env, this) {
        Api::Api14 | Api::Api24 => Ok(()),
        Api::Api26 | Api::Api29 => {
            if super::super::sdk_int(env) < 26
            /* Android 8.0 (O) */
            {
                /* Silence 'lint' warning */
                return Ok(());
            }

            if relative_mode_enabled(env, this)? && !activity::is_dex_mode(env)? {
                if let Some(content_view) = activity::get_content_view(env) {
                    env.call_method(content_view.obj(), c"requestPointerCapture", c"()V", &[])?;
                }
            }
            Ok(())
        }
    }
}

/// Translation of `checkRelativeEvent()` (API 14's and 26's).
fn check_relative_event<'a>(env: Env<'a>, this: Obj<'_>, event: Obj<'_>) -> JniResult<'a, bool> {
    match Api::of(env, this) {
        Api::Api14 | Api::Api24 => in_relative_mode(env, this),
        Api::Api26 | Api::Api29 => {
            if super::super::sdk_int(env) < 26
            /* Android 8.0 (O) */
            {
                /* Silence 'lint' warning */
                return Ok(false);
            }
            Ok(source(env, event)? == SOURCE_MOUSE_RELATIVE)
        }
    }
}

/// Translation of `getEventX()` (API 14's, 24's and 26's: relative mouse
/// in capture mode will only have relative for X/Y).
pub(crate) fn get_event_x<'a>(
    env: Env<'a>,
    this: Obj<'_>,
    event: Obj<'_>,
    pointer_index: jint,
) -> JniResult<'a, f32> {
    get_event_axis(env, this, event, pointer_index, AXIS_RELATIVE_X, get_x)
}

/// Translation of `getEventY()` (API 14's, 24's and 26's).
pub(crate) fn get_event_y<'a>(
    env: Env<'a>,
    this: Obj<'_>,
    event: Obj<'_>,
    pointer_index: jint,
) -> JniResult<'a, f32> {
    get_event_axis(env, this, event, pointer_index, AXIS_RELATIVE_Y, get_y)
}

fn get_event_axis<'a>(
    env: Env<'a>,
    this: Obj<'_>,
    event: Obj<'_>,
    pointer_index: jint,
    relative_axis: jint,
    absolute: fn(Env<'a>, Obj<'_>, jint) -> JniResult<'a, f32>,
) -> JniResult<'a, f32> {
    match Api::of(env, this) {
        Api::Api24 => {
            if super::super::sdk_int(env) < 24
            /* Android 7.0 (N) */
            {
                /* Silence 'lint' warning */
                return Ok(0.0);
            }

            if relative_mode_enabled(env, this)?
                && get_tool_type(env, event, pointer_index)? == TOOL_TYPE_MOUSE
            {
                get_axis_value(env, event, relative_axis, pointer_index)
            } else {
                absolute(env, event, pointer_index)
            }
        }
        // (API 26's: relative mouse in capture mode will only have
        // relative for X/Y)
        Api::Api14 | Api::Api26 | Api::Api29 => absolute(env, event, pointer_index),
    }
}

/// Translation of `getPenDeviceType()` (API 14's and 29's).
pub(crate) fn get_pen_device_type<'a>(
    env: Env<'a>,
    this: Obj<'_>,
    pen_device: Obj<'_>,
) -> JniResult<'a, jint> {
    match Api::of(env, this) {
        Api::Api14 | Api::Api24 | Api::Api26 => Ok(SDL_PEN_DEVICE_TYPE_UNKNOWN),
        Api::Api29 => {
            if pen_device.is_null() {
                return Ok(SDL_PEN_DEVICE_TYPE_UNKNOWN);
            }

            Ok(
                if env
                    .call_method(pen_device, c"isExternal", c"()Z", &[])?
                    .boolean()
                {
                    SDL_PEN_DEVICE_TYPE_INDIRECT
                } else {
                    SDL_PEN_DEVICE_TYPE_DIRECT
                },
            )
        }
    }
}

/// The listener of the device's API level, for `getMotionListener()`.
pub(crate) fn api_for(sdk_int: jint) -> Api {
    if sdk_int >= 29
    /* Android 10 (Q) */
    {
        Api::Api29
    } else if sdk_int >= 26
    /* Android 8.0 (O) */
    {
        Api::Api26
    } else if sdk_int >= 24
    /* Android 7.0 (N) */
    {
        Api::Api24
    } else {
        Api::Api14
    }
}

// ---------------------------------------------------------------------------
// SDLControllerManager

/// `mJoystickHandler` (made once, by [`initialize`]).
static JOYSTICK_HANDLER: OnceLock<SdlJoystickHandler> = OnceLock::new();
/// `mHapticHandler` (made once, by [`initialize`], when SDL has haptics).
static HAPTIC_HANDLER: OnceLock<SdlHapticHandler> = OnceLock::new();
/// `mDeviceListener`.
static DEVICE_LISTENER: Mutex<Option<Global>> = Mutex::new(None);

/// `Context.INPUT_SERVICE`.
const INPUT_SERVICE: &str = "input";
/// `Context.VIBRATOR_SERVICE`.
const VIBRATOR_SERVICE: &str = "vibrator";

/// Java's `NullPointerException` (of a static that is null): thrown.
fn null_pointer<'a, T>(env: Env<'a>, what: &CStr) -> JniResult<'a, T> {
    let npe = env.find_class(c"java/lang/NullPointerException")?;
    env.throw_new(npe.obj(), what);
    env.pending()?;
    unreachable!("ThrowNew() threw")
}

/// `mJoystickHandler`, or the `NullPointerException` of using it before
/// `initialize()`.
fn joystick_handler<'a>(env: Env<'a>) -> JniResult<'a, &'static SdlJoystickHandler> {
    match JOYSTICK_HANDLER.get() {
        Some(handler) => Ok(handler),
        None => null_pointer(env, c"SDLControllerManager.mJoystickHandler is null"),
    }
}

/// `SDL.getContext()`, or the `NullPointerException` of using it null.
fn context_or_throw<'a>(env: Env<'a>) -> JniResult<'a, Local<'a>> {
    match sdl::get_context(env) {
        Some(context) => Ok(context),
        None => null_pointer(env, c"SDL.getContext() is null"),
    }
}

/// `context.getSystemService(name)`.
fn system_service<'a>(env: Env<'a>, name: &str) -> JniResult<'a, Option<Local<'a>>> {
    let context = context_or_throw(env)?;
    let name = env.new_string(name)?;
    Ok(env
        .call_method(
            context.obj(),
            c"getSystemService",
            c"(Ljava/lang/String;)Ljava/lang/Object;",
            &[Arg::Object(name.obj())],
        )?
        .object())
}

/// Translation of `SDLControllerManager.initialize()`: the joystick
/// handler, and the haptic handler of the device's API level.
pub(crate) fn initialize(env: Env<'_>) {
    JOYSTICK_HANDLER.get_or_init(SdlJoystickHandler::new);

    if HAPTIC_HANDLER.get().is_none() && sdl::is_subsystem_compiled(sdl::SDL_INIT_HAPTIC) {
        let api = if super::super::sdk_int(env) >= 31
        /* Android 12.0 (S) */
        {
            HapticApi::Api31
        } else if super::super::sdk_int(env) >= 26
        /* Android 8.0 (O) */
        {
            HapticApi::Api26
        } else {
            HapticApi::Base
        };
        HAPTIC_HANDLER.get_or_init(|| SdlHapticHandler::new(api));
    }
}

/// Translation of `initializeDeviceListener()`: the input device listener
/// registered with the input manager (on the main thread's looper).
pub(crate) fn initialize_device_listener<'a>(env: Env<'a>) -> JniResult<'a, ()> {
    let mut device_listener = DEVICE_LISTENER.lock().unwrap_or_else(|e| e.into_inner());
    if device_listener.is_none() {
        let class = super::super::env::app_class(SDL_DEVICE_LISTENER)
            .expect("JNI_OnLoad() found SDLDeviceListener");
        let listener = env.new_object(class.obj(), c"()V", &[])?;
        *device_listener = Global::new(env, listener.obj());

        let im = system_service(env, INPUT_SERVICE)?;
        env.call_method(
            im.as_ref().map_or(Obj::NULL, Local::obj),
            c"registerInputDeviceListener",
            c"(Landroid/hardware/input/InputManager$InputDeviceListener;Landroid/os/Handler;)V",
            &[Arg::Object(listener.obj()), Arg::Object(Obj::NULL)],
        )?;
    }
    Ok(())
}

/// Translation of `shutdownDeviceListener()` (unused upstream too).
#[allow(dead_code)]
pub(crate) fn shutdown_device_listener<'a>(env: Env<'a>) -> JniResult<'a, ()> {
    let mut device_listener = DEVICE_LISTENER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(listener) = device_listener.as_ref() {
        let im = system_service(env, INPUT_SERVICE)?;
        env.call_method(
            im.as_ref().map_or(Obj::NULL, Local::obj),
            c"unregisterInputDeviceListener",
            c"(Landroid/hardware/input/InputManager$InputDeviceListener;)V",
            &[Arg::Object(listener.obj())],
        )?;
        *device_listener = None;
    }
    Ok(())
}

// Joystick glue code, just a series of stubs that redirect to the SDLJoystickHandler instance

/// Translation of `handleJoystickMotionEvent()`.
fn handle_joystick_motion_event<'a>(env: Env<'a>, event: Obj<'_>) -> JniResult<'a, bool> {
    match JOYSTICK_HANDLER.get() {
        Some(handler) => handler.handle_motion_event(env, event),
        None => Ok(false),
    }
}

/// This method is called by SDL using JNI. Translation of
/// `detectDevices()`.
pub(crate) fn detect_devices<'a>(env: Env<'a>) -> JniResult<'a, ()> {
    joystick_handler(env)?.detect_devices(env)
}

/// This method is called by SDL using JNI. Translation of
/// `joystickSetLED()`.
pub(crate) fn joystick_set_led<'a>(
    env: Env<'a>,
    device_id: jint,
    red: jint,
    green: jint,
    blue: jint,
) -> JniResult<'a, ()> {
    joystick_handler(env)?.set_led(env, device_id, red, green, blue)
}

/// This method is called by SDL using JNI. Translation of
/// `joystickSetSensorsEnabled()`.
pub(crate) fn joystick_set_sensors_enabled<'a>(
    env: Env<'a>,
    device_id: jint,
    enabled: bool,
) -> JniResult<'a, ()> {
    joystick_handler(env)?.set_sensors_enabled(env, device_id, enabled)
}

/// This method is called by SDL using JNI. Translation of
/// `detectHapticDevices()`.
pub(crate) fn detect_haptic_devices<'a>(env: Env<'a>) -> JniResult<'a, ()> {
    if let Some(handler) = HAPTIC_HANDLER.get() {
        handler.detect_haptic_devices(env)?;
    }
    Ok(())
}

/// This method is called by SDL using JNI. Translation of `hapticRun()`.
pub(crate) fn haptic_run<'a>(
    env: Env<'a>,
    device_id: jint,
    intensity: jfloat,
    length: jint,
) -> JniResult<'a, ()> {
    if let Some(handler) = HAPTIC_HANDLER.get() {
        handler.run(env, device_id, intensity, length)?;
    }
    Ok(())
}

/// This method is called by SDL using JNI. Translation of
/// `hapticRumble()`.
pub(crate) fn haptic_rumble<'a>(
    env: Env<'a>,
    device_id: jint,
    low_frequency_intensity: jfloat,
    high_frequency_intensity: jfloat,
    length: jint,
) -> JniResult<'a, ()> {
    if let Some(handler) = HAPTIC_HANDLER.get() {
        handler.rumble(
            env,
            device_id,
            low_frequency_intensity,
            high_frequency_intensity,
            length,
        )?;
    }
    Ok(())
}

/// This method is called by SDL using JNI. Translation of `hapticStop()`.
pub(crate) fn haptic_stop<'a>(env: Env<'a>, device_id: jint) -> JniResult<'a, ()> {
    if let Some(handler) = HAPTIC_HANDLER.get() {
        handler.stop(env, device_id)?;
    }
    Ok(())
}

/// `InputDevice.SOURCE_CLASS_JOYSTICK`, `SOURCE_DPAD`, `SOURCE_GAMEPAD`.
const SOURCE_CLASS_JOYSTICK: jint = 0x10;
const SOURCE_DPAD: jint = 0x201;
const SOURCE_GAMEPAD: jint = 0x401;

/// Check if a given device is considered a possible SDL joystick.
/// Translation of `SDLControllerManager.isDeviceSDLJoystick(InputDevice)`.
pub(crate) fn is_device_sdl_joystick<'a>(
    env: Env<'a>,
    device: Option<Obj<'_>>,
) -> JniResult<'a, bool> {
    // No virtual device (eg nonnegative deviceId. VIRTUAL_KEYBOARD equals -1)
    let Some(device) = device else {
        return Ok(false);
    };
    if env
        .call_method(device, c"isVirtual", c"()Z", &[])?
        .boolean()
    {
        return Ok(false);
    }
    let sources = env.call_method(device, c"getSources", c"()I", &[])?.int();

    /* This is called for every button press, so let's not spam the logs */
    // (upstream's commented-out Log.v() of the joystick class, the d-pad
    // and the gamepad)

    Ok((sources & SOURCE_CLASS_JOYSTICK) != 0
        || ((sources & SOURCE_DPAD) == SOURCE_DPAD)
        || ((sources & SOURCE_GAMEPAD) == SOURCE_GAMEPAD))
}

/// `InputDevice.getDevice(deviceId)`.
fn input_device<'a>(env: Env<'a>, device_id: jint) -> JniResult<'a, Option<Local<'a>>> {
    let input_device = env.find_class(c"android/view/InputDevice")?;
    Ok(env
        .call_static_method(
            input_device.obj(),
            c"getDevice",
            c"(I)Landroid/view/InputDevice;",
            &[Arg::Int(device_id)],
        )?
        .object())
}

/// Translation of `isDeviceSDLJoystick(int deviceId)`.
fn is_device_sdl_joystick_id<'a>(env: Env<'a>, device_id: jint) -> JniResult<'a, bool> {
    let device = input_device(env, device_id)?;
    is_device_sdl_joystick(env, device.as_ref().map(Local::obj))
}

/// `SDLControllerManager.onNativePadDown()`.
pub(crate) fn on_native_pad_down(device_id: jint, keycode: jint, scancode: jint) -> bool {
    super::super::on_pad_down(device_id, keycode, scancode)
}

/// `SDLControllerManager.onNativePadUp()`.
pub(crate) fn on_native_pad_up(device_id: jint, keycode: jint, scancode: jint) -> bool {
    super::super::on_pad_up(device_id, keycode, scancode)
}

// The static methods SDL calls (Rust functions, which the DEX file's class
// declares `native` too, so that Java calls them as it calls upstream's)

/// A native method's body: an exception it throws stays pending, for the
/// caller.
fn native<'a>(env: Env<'a>, body: impl FnOnce(Env<'a>) -> JniResult<'a, ()>) {
    if let Err(thrown) = body(env) {
        // (pending, for the caller)
        drop(thrown);
    }
}

/// `SDLControllerManager.detectDevices()` called from Java.
pub(crate) extern "system" fn detect_devices_java(env: *mut JNIEnv, _cls: jclass) {
    // SAFETY: a native method's environment.
    let env = unsafe { Env::from_raw(env) };
    native(env, detect_devices);
}

/// `SDLControllerManager.joystickSetLED()` called from Java.
pub(crate) extern "system" fn joystick_set_led_java(
    env: *mut JNIEnv,
    _cls: jclass,
    device_id: jint,
    red: jint,
    green: jint,
    blue: jint,
) {
    // SAFETY: a native method's environment.
    let env = unsafe { Env::from_raw(env) };
    native(env, |env| {
        joystick_set_led(env, device_id, red, green, blue)
    });
}

/// `SDLControllerManager.joystickSetSensorsEnabled()` called from Java.
pub(crate) extern "system" fn joystick_set_sensors_enabled_java(
    env: *mut JNIEnv,
    _cls: jclass,
    device_id: jint,
    enabled: jboolean,
) {
    // SAFETY: a native method's environment.
    let env = unsafe { Env::from_raw(env) };
    native(env, |env| {
        joystick_set_sensors_enabled(env, device_id, enabled != 0)
    });
}

/// `SDLControllerManager.detectHapticDevices()` called from Java.
pub(crate) extern "system" fn detect_haptic_devices_java(env: *mut JNIEnv, _cls: jclass) {
    // SAFETY: a native method's environment.
    let env = unsafe { Env::from_raw(env) };
    native(env, detect_haptic_devices);
}

/// `SDLControllerManager.hapticRun()` called from Java.
pub(crate) extern "system" fn haptic_run_java(
    env: *mut JNIEnv,
    _cls: jclass,
    device_id: jint,
    intensity: jfloat,
    length: jint,
) {
    // SAFETY: a native method's environment.
    let env = unsafe { Env::from_raw(env) };
    native(env, |env| haptic_run(env, device_id, intensity, length));
}

/// `SDLControllerManager.hapticRumble()` called from Java.
pub(crate) extern "system" fn haptic_rumble_java(
    env: *mut JNIEnv,
    _cls: jclass,
    device_id: jint,
    low_frequency_intensity: jfloat,
    high_frequency_intensity: jfloat,
    length: jint,
) {
    // SAFETY: a native method's environment.
    let env = unsafe { Env::from_raw(env) };
    native(env, |env| {
        haptic_rumble(
            env,
            device_id,
            low_frequency_intensity,
            high_frequency_intensity,
            length,
        )
    });
}

/// `SDLControllerManager.hapticStop()` called from Java.
pub(crate) extern "system" fn haptic_stop_java(env: *mut JNIEnv, _cls: jclass, device_id: jint) {
    // SAFETY: a native method's environment.
    let env = unsafe { Env::from_raw(env) };
    native(env, |env| haptic_stop(env, device_id));
}

// ---------------------------------------------------------------------------
// SDLDeviceListener implements InputDeviceListener

/// Translation of `SDLDeviceListener.onInputDeviceAdded()`.
pub(crate) extern "system" fn on_input_device_added(
    env: *mut JNIEnv,
    _this: jobject,
    device_id: jint,
) {
    // SAFETY: a native method's environment.
    let env = unsafe { Env::from_raw(env) };
    native(env, |env| {
        if is_device_sdl_joystick_id(env, device_id)? {
            joystick_handler(env)?.device_added(env, device_id)?;
        }
        Ok(())
    });
}

/// Translation of `SDLDeviceListener.onInputDeviceChanged()`.
pub(crate) extern "system" fn on_input_device_changed(
    env: *mut JNIEnv,
    _this: jobject,
    device_id: jint,
) {
    // SAFETY: a native method's environment.
    let env = unsafe { Env::from_raw(env) };
    native(env, |env| {
        // The controller might not have had joystick sources previously
        if is_device_sdl_joystick_id(env, device_id)? {
            joystick_handler(env)?.device_added(env, device_id)?;
        }
        Ok(())
    });
}

/// Translation of `SDLDeviceListener.onInputDeviceRemoved()`.
pub(crate) extern "system" fn on_input_device_removed(
    env: *mut JNIEnv,
    _this: jobject,
    device_id: jint,
) {
    // SAFETY: a native method's environment.
    let env = unsafe { Env::from_raw(env) };
    native(env, |env| {
        // InputDevice.getDevice(deviceId) returns NULL. so we cannot check device type
        joystick_handler(env)?.device_removed(env, device_id);
        Ok(())
    });
}

// ---------------------------------------------------------------------------
// SDLJoystickHandler: actual joystick functionality available for API >=
// 19 devices

/// `MotionEvent.AXIS_*` (the joystick's).
const AXIS_Z: jint = 11;
const AXIS_RZ: jint = 14;
const AXIS_HAT_X: jint = 15;
const AXIS_HAT_Y: jint = 16;
const AXIS_GAS: jint = 22;
const AXIS_BRAKE: jint = 23;

/// `Sensor.TYPE_ACCELEROMETER`, `Sensor.TYPE_GYROSCOPE`.
const SENSOR_TYPE_ACCELEROMETER: jint = 1;
const SENSOR_TYPE_GYROSCOPE: jint = 4;
/// `SensorManager.SENSOR_DELAY_GAME`.
const SENSOR_DELAY_GAME: jint = 1;

/// An `InputDevice.MotionRange`'s axis and range (its fields are final:
/// read once, when the device is added).
#[derive(Clone, Copy, Debug)]
struct MotionRange {
    axis: jint,
    source: jint,
    min: f32,
    range: f32,
}

/// Translation of `SDLJoystickHandler.SDLJoystick`.
#[derive(Debug)]
struct SdlJoystick {
    device_id: jint,
    name: String,
    desc: String,
    axes: Vec<MotionRange>,
    hats: Vec<MotionRange>,
    lights: Vec<Global>,
    lights_session: Mutex<Option<Global>>,
    sensor_manager: Option<Global>,
    sensor_listener: Option<Global>,
    accelerometer_sensor: Option<Global>,
    gyroscope_sensor: Option<Global>,
}

/// Translation of `SDLJoystickHandler.RangeComparator.compare()`.
fn compare_ranges(arg0: &MotionRange, arg1: &MotionRange) -> std::cmp::Ordering {
    // Some controllers, like the Moga Pro 2, return AXIS_GAS (22) for right trigger and AXIS_BRAKE (23) for left trigger - swap them so they're sorted in the right order for SDL
    let mut arg0_axis = arg0.axis;
    let mut arg1_axis = arg1.axis;
    if arg0_axis == AXIS_GAS {
        arg0_axis = AXIS_BRAKE;
    } else if arg0_axis == AXIS_BRAKE {
        arg0_axis = AXIS_GAS;
    }
    if arg1_axis == AXIS_GAS {
        arg1_axis = AXIS_BRAKE;
    } else if arg1_axis == AXIS_BRAKE {
        arg1_axis = AXIS_GAS;
    }

    // Make sure the AXIS_Z is sorted between AXIS_RY and AXIS_RZ.
    // This is because the usual pairing are:
    // - AXIS_X + AXIS_Y (left stick).
    // - AXIS_RX, AXIS_RY (sometimes the right stick, sometimes triggers).
    // - AXIS_Z, AXIS_RZ (sometimes the right stick, sometimes triggers).
    // This sorts the axes in the above order, which tends to be correct
    // for Xbox-ish game pads that have the right stick on RX/RY and the
    // triggers on Z/RZ.
    //
    // Gamepads that don't have AXIS_Z/AXIS_RZ but use
    // AXIS_LTRIGGER/AXIS_RTRIGGER are unaffected by this.
    //
    // References:
    // - https://developer.android.com/develop/ui/views/touch-and-input/game-controllers/controller-input
    // - https://www.kernel.org/doc/html/latest/input/gamepad.html
    if arg0_axis == AXIS_Z {
        arg0_axis = AXIS_RZ - 1;
    } else if arg0_axis > AXIS_Z && arg0_axis < AXIS_RZ {
        arg0_axis -= 1;
    }
    if arg1_axis == AXIS_Z {
        arg1_axis = AXIS_RZ - 1;
    } else if arg1_axis > AXIS_Z && arg1_axis < AXIS_RZ {
        arg1_axis -= 1;
    }

    (arg0_axis - arg1_axis).cmp(&0)
}

/// Translation of `class SDLJoystickHandler`: its devices (`mJoysticks`),
/// and the monitor its `synchronized` methods take.
struct SdlJoystickHandler {
    joysticks: Mutex<Vec<Arc<SdlJoystick>>>,
    monitor: ReentrantMutex<()>,
}

/// `java.lang.Math.round(float)`.
fn java_round(value: f32) -> jint {
    if value.is_nan() {
        return 0;
    }
    (value + 0.5).floor() as jint
}

impl SdlJoystickHandler {
    fn new() -> SdlJoystickHandler {
        SdlJoystickHandler {
            joysticks: Mutex::new(Vec::new()),
            monitor: ReentrantMutex::new(()),
        }
    }

    fn joysticks(&self) -> std::sync::MutexGuard<'_, Vec<Arc<SdlJoystick>>> {
        self.joysticks.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Handles adding and removing of input devices. Translation of
    /// `detectDevices()` (synchronized).
    fn detect_devices<'a>(&self, env: Env<'a>) -> JniResult<'a, ()> {
        let _synchronized = self.monitor.lock();
        let input_device = env.find_class(c"android/view/InputDevice")?;
        let device_ids = env
            .call_static_method(input_device.obj(), c"getDeviceIds", c"()[I", &[])?
            .object();
        let device_ids = match &device_ids {
            Some(ids) => env.get_int_array(ids.obj())?,
            None => Vec::new(),
        };
        for device_id in device_ids {
            if is_device_sdl_joystick_id(env, device_id)? {
                self.device_added(env, device_id)?;
            }
        }
        Ok(())
    }

    /// Translation of `deviceAdded()`.
    fn device_added<'a>(&self, env: Env<'a>, device_id: jint) -> JniResult<'a, ()> {
        let Some(joystick_device) = input_device(env, device_id)? else {
            return Ok(());
        };
        let joystick_device = joystick_device.obj();

        let joystick = match self.get_joystick(device_id) {
            Some(joystick) => joystick,
            None => {
                let name = env
                    .call_method(joystick_device, c"getName", c"()Ljava/lang/String;", &[])?
                    .object();
                let name = env.get_string(name.as_ref().map_or(Obj::NULL, Local::obj))?;
                let desc = get_joystick_descriptor(env, joystick_device)?;
                let mut axes = Vec::new();
                let mut hats = Vec::new();
                let mut axis_strs_set = std::collections::HashSet::new();
                let mut lights = Vec::new();

                let mut ranges = motion_ranges(env, joystick_device)?;
                ranges.sort_by(compare_ranges);
                for range in ranges {
                    if ((range.source & SOURCE_CLASS_JOYSTICK) != 0)
                        && axis_strs_set.insert(range.axis)
                    {
                        if range.axis == AXIS_HAT_X || range.axis == AXIS_HAT_Y {
                            hats.push(range);
                        } else {
                            axes.push(range);
                        }
                    }
                }

                let mut lights_session = None;
                let mut sensor_manager = None;
                let mut sensor_listener = None;
                let mut accelerometer_sensor = None;
                let mut gyroscope_sensor = None;
                if super::super::sdk_int(env) >= 31
                /* Android 12.0 (S) */
                {
                    let lights_manager = env
                        .call_method(
                            joystick_device,
                            c"getLightsManager",
                            c"()Landroid/hardware/lights/LightsManager;",
                            &[],
                        )?
                        .object();
                    let lights_manager = lights_manager.as_ref().map_or(Obj::NULL, Local::obj);
                    for light in list_elements(
                        env,
                        env.call_method(lights_manager, c"getLights", c"()Ljava/util/List;", &[])?
                            .object(),
                    )? {
                        if env
                            .call_method(light.obj(), c"hasRgbControl", c"()Z", &[])?
                            .boolean()
                        {
                            lights.extend(Global::new(env, light.obj()));
                        }
                    }
                    if !lights.is_empty() {
                        let session = env
                            .call_method(
                                lights_manager,
                                c"openSession",
                                c"()Landroid/hardware/lights/LightsManager$LightsSession;",
                                &[],
                            )?
                            .object();
                        lights_session =
                            session.and_then(|session| Global::new(env, session.obj()));
                    }
                    let manager = env
                        .call_method(
                            joystick_device,
                            c"getSensorManager",
                            c"()Landroid/hardware/SensorManager;",
                            &[],
                        )?
                        .object();
                    if let Some(manager) = manager {
                        sensor_manager = Global::new(env, manager.obj());
                        let listener = new_joy_sensor_listener(env, device_id)?;
                        sensor_listener = Global::new(env, listener.obj());
                        accelerometer_sensor =
                            default_sensor(env, manager.obj(), SENSOR_TYPE_ACCELEROMETER)?;
                        gyroscope_sensor =
                            default_sensor(env, manager.obj(), SENSOR_TYPE_GYROSCOPE)?;
                    }
                }

                let joystick = Arc::new(SdlJoystick {
                    device_id,
                    name,
                    desc,
                    axes,
                    hats,
                    lights,
                    lights_session: Mutex::new(lights_session),
                    sensor_manager,
                    sensor_listener,
                    accelerometer_sensor,
                    gyroscope_sensor,
                });
                self.joysticks().push(Arc::clone(&joystick));
                joystick
            }
        };

        let mut can_rumble = false;
        let mut has_rgb_led = false;
        let mut has_accelerometer = false;
        let mut has_gyroscope = false;
        if super::super::sdk_int(env) >= 31
        /* Android 12.0 (S) */
        {
            let vibrators = vibrator_ids(env, joystick_device)?;
            if !vibrators.is_empty() {
                can_rumble = true;
            }
            if !joystick.lights.is_empty() {
                has_rgb_led = true;
            }
            if joystick.accelerometer_sensor.is_some() {
                has_accelerometer = true;
            }
            if joystick.gyroscope_sensor.is_some() {
                has_gyroscope = true;
            }
        }

        let vendor_id = get_vendor_id(env, joystick_device)?;
        let product_id = get_product_id(env, joystick_device)?;
        let button_mask = get_button_mask(env, joystick_device)?;
        crate::joystick::android::add_joystick(
            joystick.device_id,
            Some(&joystick.name),
            &joystick.desc,
            vendor_id,
            product_id,
            button_mask,
            joystick.axes.len() as jint,
            get_axis_mask(&joystick.axes),
            (joystick.hats.len() / 2) as jint,
            can_rumble,
            has_rgb_led,
            has_accelerometer,
            has_gyroscope,
        );
        Ok(())
    }

    /// Translation of `deviceRemoved()`.
    fn device_removed(&self, env: Env<'_>, device_id: jint) {
        let known = self
            .joysticks()
            .iter()
            .any(|joystick| joystick.device_id == device_id);
        if !known {
            return;
        }

        crate::joystick::android::remove_joystick(device_id);

        let mut joysticks = self.joysticks();
        let Some(i) = joysticks
            .iter()
            .position(|joystick| joystick.device_id == device_id)
        else {
            return;
        };
        if super::super::sdk_int(env) >= 31
        /* Android 12.0 (S) */
        {
            let session = joysticks[i]
                .lights_session
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            if let Some(session) = session {
                // try { ... } catch (Exception e) { ... }
                if let Err(thrown) = env.call_method(session.obj(), c"close", c"()V", &[]) {
                    // Session may already be unregistered when device disconnects
                    // (Note: an Error thrown is cleared too)
                    thrown.clear();
                }
            }
        }
        joysticks.remove(i);
    }

    /// Translation of `getJoystick()` (synchronized).
    fn get_joystick(&self, device_id: jint) -> Option<Arc<SdlJoystick>> {
        let _synchronized = self.monitor.lock();
        self.joysticks()
            .iter()
            .find(|joystick| joystick.device_id == device_id)
            .cloned()
    }

    /// Handles given MotionEvent: whether it was processed. Translation of
    /// `handleMotionEvent()`.
    fn handle_motion_event<'a>(&self, env: Env<'a>, event: Obj<'_>) -> JniResult<'a, bool> {
        let action_pointer_index = env
            .call_method(event, c"getActionIndex", c"()I", &[])?
            .int();
        let action = env
            .call_method(event, c"getActionMasked", c"()I", &[])?
            .int();
        if action == ACTION_MOVE {
            let device_id = env.call_method(event, c"getDeviceId", c"()I", &[])?.int();
            if let Some(joystick) = self.get_joystick(device_id) {
                for (i, range) in joystick.axes.iter().enumerate() {
                    /* Normalize the value to -1...1 */
                    let value = (get_axis_value(env, event, range.axis, action_pointer_index)?
                        - range.min)
                        / range.range
                        * 2.0
                        - 1.0;
                    super::super::on_joy(joystick.device_id, i as jint, value);
                }
                for i in 0..joystick.hats.len() / 2 {
                    let hat_x = java_round(get_axis_value(
                        env,
                        event,
                        joystick.hats[2 * i].axis,
                        action_pointer_index,
                    )?);
                    let hat_y = java_round(get_axis_value(
                        env,
                        event,
                        joystick.hats[2 * i + 1].axis,
                        action_pointer_index,
                    )?);
                    super::super::on_hat(joystick.device_id, i as jint, hat_x, hat_y);
                }
            }
        }
        Ok(true)
    }

    /// Translation of `setLED()`.
    fn set_led<'a>(
        &self,
        env: Env<'a>,
        device_id: jint,
        red: jint,
        green: jint,
        blue: jint,
    ) -> JniResult<'a, ()> {
        if super::super::sdk_int(env) < 31
        /* Android 12.0 (S) */
        {
            return Ok(());
        }

        let Some(joystick) = self.get_joystick(device_id) else {
            return Ok(());
        };
        if joystick.lights.is_empty() {
            return Ok(());
        }

        let builder_class = env.find_class(c"android/hardware/lights/LightsRequest$Builder")?;
        let lights_request = env.new_object(builder_class.obj(), c"()V", &[])?;
        let color = env.find_class(c"android/graphics/Color")?;
        let rgb = env
            .call_static_method(
                color.obj(),
                c"rgb",
                c"(III)I",
                &[Arg::Int(red), Arg::Int(green), Arg::Int(blue)],
            )?
            .int();
        let state_builder_class = env.find_class(c"android/hardware/lights/LightState$Builder")?;
        let state_builder = env.new_object(state_builder_class.obj(), c"()V", &[])?;
        let state_builder = env
            .call_method(
                state_builder.obj(),
                c"setColor",
                c"(I)Landroid/hardware/lights/LightState$Builder;",
                &[Arg::Int(rgb)],
            )?
            .object();
        let light_state = env
            .call_method(
                state_builder.as_ref().map_or(Obj::NULL, Local::obj),
                c"build",
                c"()Landroid/hardware/lights/LightState;",
                &[],
            )?
            .object();
        let light_state = light_state.as_ref().map_or(Obj::NULL, Local::obj);
        for light in &joystick.lights {
            if env
                .call_method(light.obj(), c"hasRgbControl", c"()Z", &[])?
                .boolean()
            {
                env.call_method(
                    lights_request.obj(),
                    c"addLight",
                    c"(Landroid/hardware/lights/Light;Landroid/hardware/lights/LightState;)Landroid/hardware/lights/LightsRequest$Builder;",
                    &[Arg::Object(light.obj()), Arg::Object(light_state)],
                )?;
            }
        }
        let request = env
            .call_method(
                lights_request.obj(),
                c"build",
                c"()Landroid/hardware/lights/LightsRequest;",
                &[],
            )?
            .object();
        let session = joystick
            .lights_session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|session| env.new_local_ref(session.obj()));
        let Some(session) = session else {
            return null_pointer(env, c"SDLJoystick.lightsSession is null");
        };
        env.call_method(
            session.obj(),
            c"requestLights",
            c"(Landroid/hardware/lights/LightsRequest;)V",
            &[Arg::Object(request.as_ref().map_or(Obj::NULL, Local::obj))],
        )?;
        Ok(())
    }

    /// Translation of `setSensorsEnabled()`.
    fn set_sensors_enabled<'a>(
        &self,
        env: Env<'a>,
        device_id: jint,
        enabled: bool,
    ) -> JniResult<'a, ()> {
        if super::super::sdk_int(env) < 31
        /* Android 12.0 (S) */
        {
            return Ok(());
        }

        let Some(joystick) = self.get_joystick(device_id) else {
            return Ok(());
        };
        let Some(sensor_manager) = &joystick.sensor_manager else {
            return Ok(());
        };
        let listener = joystick
            .sensor_listener
            .as_ref()
            .map_or(Obj::NULL, Global::obj);
        for sensor in [&joystick.accelerometer_sensor, &joystick.gyroscope_sensor]
            .into_iter()
            .flatten()
        {
            if enabled {
                super::sensor_manager::register_listener(
                    env,
                    sensor_manager.obj(),
                    listener,
                    sensor.obj(),
                    SENSOR_DELAY_GAME,
                )?;
            } else {
                super::sensor_manager::unregister_listener(
                    env,
                    sensor_manager.obj(),
                    listener,
                    sensor.obj(),
                )?;
            }
        }
        Ok(())
    }
}

/// The elements of a `java.util.List` (none for null).
fn list_elements<'a>(env: Env<'a>, list: Option<Local<'a>>) -> JniResult<'a, Vec<Local<'a>>> {
    let mut out = Vec::new();
    let Some(list) = list else {
        return Ok(out);
    };
    let size = env.call_method(list.obj(), c"size", c"()I", &[])?.int();
    for i in 0..size {
        if let Some(element) = env
            .call_method(list.obj(), c"get", c"(I)Ljava/lang/Object;", &[Arg::Int(i)])?
            .object()
        {
            out.push(element);
        }
    }
    Ok(out)
}

/// `joystickDevice.getMotionRanges()`: each range's axis, source, minimum
/// and range.
fn motion_ranges<'a>(env: Env<'a>, joystick_device: Obj<'_>) -> JniResult<'a, Vec<MotionRange>> {
    let list = env
        .call_method(
            joystick_device,
            c"getMotionRanges",
            c"()Ljava/util/List;",
            &[],
        )?
        .object();
    let mut ranges = Vec::new();
    for range in list_elements(env, list)? {
        let range = range.obj();
        ranges.push(MotionRange {
            axis: env.call_method(range, c"getAxis", c"()I", &[])?.int(),
            source: env.call_method(range, c"getSource", c"()I", &[])?.int(),
            min: env.call_method(range, c"getMin", c"()F", &[])?.float(),
            range: env.call_method(range, c"getRange", c"()F", &[])?.float(),
        });
    }
    Ok(ranges)
}

/// `sensorManager.getDefaultSensor(type)`.
fn default_sensor<'a>(
    env: Env<'a>,
    sensor_manager: Obj<'_>,
    sensor_type: jint,
) -> JniResult<'a, Option<Global>> {
    let sensor = env
        .call_method(
            sensor_manager,
            c"getDefaultSensor",
            c"(I)Landroid/hardware/Sensor;",
            &[Arg::Int(sensor_type)],
        )?
        .object();
    Ok(sensor.and_then(|sensor| Global::new(env, sensor.obj())))
}

/// `device.getVibratorManager().getVibratorIds()`.
fn vibrator_ids<'a>(env: Env<'a>, device: Obj<'_>) -> JniResult<'a, Vec<jint>> {
    let manager = env
        .call_method(
            device,
            c"getVibratorManager",
            c"()Landroid/os/VibratorManager;",
            &[],
        )?
        .object();
    let ids = env
        .call_method(
            manager.as_ref().map_or(Obj::NULL, Local::obj),
            c"getVibratorIds",
            c"()[I",
            &[],
        )?
        .object();
    match ids {
        Some(ids) => env.get_int_array(ids.obj()),
        None => Ok(Vec::new()),
    }
}

/// Translation of `getJoystickDescriptor()`.
fn get_joystick_descriptor<'a>(env: Env<'a>, joystick_device: Obj<'_>) -> JniResult<'a, String> {
    let desc = env
        .call_method(
            joystick_device,
            c"getDescriptor",
            c"()Ljava/lang/String;",
            &[],
        )?
        .object();

    if let Some(desc) = desc {
        let desc = env.get_string(desc.obj())?;
        if !desc.is_empty() {
            return Ok(desc);
        }
    }

    let name = env
        .call_method(joystick_device, c"getName", c"()Ljava/lang/String;", &[])?
        .object();
    env.get_string(name.as_ref().map_or(Obj::NULL, Local::obj))
}

/// Translation of `getProductId()`.
fn get_product_id<'a>(env: Env<'a>, joystick_device: Obj<'_>) -> JniResult<'a, jint> {
    Ok(env
        .call_method(joystick_device, c"getProductId", c"()I", &[])?
        .int())
}

/// Translation of `getVendorId()`.
fn get_vendor_id<'a>(env: Env<'a>, joystick_device: Obj<'_>) -> JniResult<'a, jint> {
    Ok(env
        .call_method(joystick_device, c"getVendorId", c"()I", &[])?
        .int())
}

/// Translation of `getAxisMask()`.
fn get_axis_mask(ranges: &[MotionRange]) -> jint {
    // For compatibility, keep computing the axis mask like before,
    // only really distinguishing 2, 4 and 6 axes.
    let mut axis_mask = 0;
    if ranges.len() >= 2 {
        // ((1 << SDL_GAMEPAD_AXIS_LEFTX) | (1 << SDL_GAMEPAD_AXIS_LEFTY))
        axis_mask |= 0x0003;
    }
    if ranges.len() >= 4 {
        // ((1 << SDL_GAMEPAD_AXIS_RIGHTX) | (1 << SDL_GAMEPAD_AXIS_RIGHTY))
        axis_mask |= 0x000c;
    }
    if ranges.len() >= 6 {
        // ((1 << SDL_GAMEPAD_AXIS_LEFT_TRIGGER) | (1 << SDL_GAMEPAD_AXIS_RIGHT_TRIGGER))
        axis_mask |= 0x0030;
    }
    // Also add an indicator bit for whether the sorting order has changed.
    // This serves to disable outdated gamecontrollerdb.txt mappings.
    let mut have_z = false;
    let mut have_past_z_before_rz = false;
    for range in ranges {
        let axis = range.axis;
        if axis == AXIS_Z {
            have_z = true;
        } else if axis > AXIS_Z && axis < AXIS_RZ {
            have_past_z_before_rz = true;
        }
    }
    if have_z && have_past_z_before_rz {
        // If both these exist, the compare() function changed sorting order.
        // Set a bit to indicate this fact.
        axis_mask |= 0x8000;
    }
    axis_mask
}

/// `KeyEvent.KEYCODE_*` of [`get_button_mask`], and their masks.
const BUTTON_KEYS: [(jint, u32); 38] = [
    (96, 1 << 0),   // A -> A
    (97, 1 << 1),   // B -> B
    (99, 1 << 2),   // X -> X
    (100, 1 << 3),  // Y -> Y
    (4, 1 << 4),    // BACK -> BACK
    (82, 1 << 6),   // MENU -> START
    (110, 1 << 5),  // MODE -> GUIDE
    (108, 1 << 6),  // START -> START
    (106, 1 << 7),  // THUMBL -> LEFTSTICK
    (107, 1 << 8),  // THUMBR -> RIGHTSTICK
    (102, 1 << 9),  // L1 -> LEFTSHOULDER
    (103, 1 << 10), // R1 -> RIGHTSHOULDER
    (19, 1 << 11),  // DPAD_UP -> DPAD_UP
    (20, 1 << 12),  // DPAD_DOWN -> DPAD_DOWN
    (21, 1 << 13),  // DPAD_LEFT -> DPAD_LEFT
    (22, 1 << 14),  // DPAD_RIGHT -> DPAD_RIGHT
    (109, 1 << 4),  // SELECT -> BACK
    (23, 1 << 0),   // DPAD_CENTER -> A
    // These don't map into any SDL controller buttons directly
    (104, 1 << 15), // L2 -> ??
    (105, 1 << 16), // R2 -> ??
    (98, 1 << 17),  // C -> ??
    (101, 1 << 18), // Z -> ??
    (188, 1 << 20), // 1 -> ??
    (189, 1 << 21), // 2 -> ??
    (190, 1 << 22), // 3 -> ??
    (191, 1 << 23), // 4 -> ??
    (192, 1 << 24), // 5 -> ??
    (193, 1 << 25), // 6 -> ??
    (194, 1 << 26), // 7 -> ??
    (195, 1 << 27), // 8 -> ??
    (196, 1 << 28), // 9 -> ??
    (197, 1 << 29), // 10 -> ??
    (198, 1 << 30), // 11 -> ??
    (199, 1 << 31), // 12 -> ??
    // We're out of room...
    (200, 0xFFFF_FFFF), // 13 -> ??
    (201, 0xFFFF_FFFF), // 14 -> ??
    (202, 0xFFFF_FFFF), // 15 -> ??
    (203, 0xFFFF_FFFF), // 16 -> ??
];

/// Translation of `getButtonMask()`: the masks of the keys the device has
/// (`hasKeys()`).
fn get_button_mask<'a>(env: Env<'a>, joystick_device: Obj<'_>) -> JniResult<'a, jint> {
    let mut button_mask = 0u32;
    let keys: Vec<jint> = BUTTON_KEYS.iter().map(|&(key, _)| key).collect();
    let keys = env.new_int_array(&keys)?;
    let has_keys = env
        .call_method(
            joystick_device,
            c"hasKeys",
            c"([I)[Z",
            &[Arg::Object(keys.obj())],
        )?
        .object();
    let has_keys = match &has_keys {
        Some(has_keys) => env.get_boolean_array(has_keys.obj())?,
        None => Vec::new(),
    };
    for (i, &(_, mask)) in BUTTON_KEYS.iter().enumerate() {
        if has_keys.get(i).copied().unwrap_or(false) {
            button_mask |= mask;
        }
    }
    Ok(button_mask as jint)
}

// ---------------------------------------------------------------------------
// The haptic handlers: SDLHapticHandler, SDLHapticHandler_API26 and
// SDLHapticHandler_API31 (each extending SDLHapticHandler)

/// Which of the haptic handler classes the handler is: the class whose
/// methods a virtual call runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HapticApi {
    Base,
    Api26,
    Api31,
}

/// Translation of `SDLHapticHandler.SDLHaptic`.
#[derive(Debug)]
struct SdlHaptic {
    device_id: jint,
    name: String,
    vib: Global,
}

/// Translation of `class SDLHapticHandler` (and its subclasses, by
/// `api`): its vibrators (`mHaptics`), and the monitor its `synchronized`
/// methods take.
struct SdlHapticHandler {
    api: HapticApi,
    haptics: Mutex<Vec<Arc<SdlHaptic>>>,
    monitor: ReentrantMutex<()>,
}

/// `VibrationEffect.createOneShot(length, amplitude)`.
fn create_one_shot<'a>(env: Env<'a>, length: jint, amplitude: jint) -> JniResult<'a, Local<'a>> {
    let vibration_effect = env.find_class(c"android/os/VibrationEffect")?;
    let effect = env
        .call_static_method(
            vibration_effect.obj(),
            c"createOneShot",
            c"(JI)Landroid/os/VibrationEffect;",
            &[Arg::Long(length as jlong), Arg::Int(amplitude)],
        )?
        .object();
    match effect {
        Some(effect) => Ok(effect),
        None => null_pointer(env, c"VibrationEffect.createOneShot() is null"),
    }
}

/// `vibrator.vibrate(VibrationEffect.createOneShot(length, value))`, else
/// (an `Exception` thrown) `vibrator.vibrate(length)`.
fn vibrate_one_shot<'a>(
    env: Env<'a>,
    vibrator: Obj<'_>,
    length: jint,
    value: jint,
) -> JniResult<'a, ()> {
    let body = || -> JniResult<'a, ()> {
        let effect = create_one_shot(env, length, value)?;
        env.call_method(
            vibrator,
            c"vibrate",
            c"(Landroid/os/VibrationEffect;)V",
            &[Arg::Object(effect.obj())],
        )?;
        Ok(())
    };
    // try { ... } catch (Exception e) { ... }
    if let Err(thrown) = body() {
        thrown.catch_exception()?;
        // Fall back to the generic method, which uses DEFAULT_AMPLITUDE, but works even if
        // something went horribly wrong with the Android 8.0 APIs.
        vibrate(env, vibrator, length)?;
    }
    Ok(())
}

/// `vibrator.vibrate(length)`.
fn vibrate<'a>(env: Env<'a>, vibrator: Obj<'_>, length: jint) -> JniResult<'a, ()> {
    env.call_method(vibrator, c"vibrate", c"(J)V", &[Arg::Long(length as jlong)])?;
    Ok(())
}

/// `vibrator.cancel()`.
fn cancel<'a>(env: Env<'a>, vibrator: Obj<'_>) -> JniResult<'a, ()> {
    env.call_method(vibrator, c"cancel", c"()V", &[])?;
    Ok(())
}

impl SdlHapticHandler {
    fn new(api: HapticApi) -> SdlHapticHandler {
        SdlHapticHandler {
            api,
            haptics: Mutex::new(Vec::new()),
            monitor: ReentrantMutex::new(()),
        }
    }

    fn haptics(&self) -> std::sync::MutexGuard<'_, Vec<Arc<SdlHaptic>>> {
        self.haptics.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Translation of `run()` (`SDLHapticHandler`'s, `_API26`'s and
    /// `_API31`'s).
    fn run<'a>(
        &self,
        env: Env<'a>,
        device_id: jint,
        intensity: f32,
        length: jint,
    ) -> JniResult<'a, ()> {
        match self.api {
            HapticApi::Api31 => {
                if let Some(haptic) = self.get_haptic(device_id) {
                    self.vibrate_api31(env, haptic.vib.obj(), intensity, length)?;
                }
            }
            HapticApi::Api26 => {
                if super::super::sdk_int(env) < 26
                /* Android 8.0 (O) */
                {
                    /* Silence 'lint' warning */
                    return Ok(());
                }
                if let Some(haptic) = self.get_haptic(device_id) {
                    if intensity == 0.0 {
                        self.stop(env, device_id)?;
                        return Ok(());
                    }

                    let mut vibe_value = java_round(intensity * 255.0);
                    if vibe_value > 255 {
                        vibe_value = 255;
                    }
                    if vibe_value < 1 {
                        self.stop(env, device_id)?;
                        return Ok(());
                    }
                    vibrate_one_shot(env, haptic.vib.obj(), length, vibe_value)?;
                }
            }
            HapticApi::Base => {
                if let Some(haptic) = self.get_haptic(device_id) {
                    vibrate(env, haptic.vib.obj(), length)?;
                }
            }
        }
        Ok(())
    }

    /// Translation of `rumble()` (`SDLHapticHandler`'s, not supported in
    /// older APIs, and `_API31`'s: the device's vibrators).
    fn rumble<'a>(
        &self,
        env: Env<'a>,
        device_id: jint,
        low_frequency_intensity: f32,
        high_frequency_intensity: f32,
        length: jint,
    ) -> JniResult<'a, ()> {
        if self.api != HapticApi::Api31 {
            // Not supported in older APIs
            return Ok(());
        }

        let Some(device) = input_device(env, device_id)? else {
            return Ok(());
        };

        if super::super::sdk_int(env) < 31
        /* Android 12.0 (S) */
        {
            /* Silence 'lint' warning */
            return Ok(());
        }

        let manager = env
            .call_method(
                device.obj(),
                c"getVibratorManager",
                c"()Landroid/os/VibratorManager;",
                &[],
            )?
            .object();
        let manager = manager.as_ref().map_or(Obj::NULL, Local::obj);
        let vibrators = vibrator_ids(env, device.obj())?;
        let vibrator = |id: jint| -> JniResult<'a, Option<Local<'a>>> {
            Ok(env
                .call_method(
                    manager,
                    c"getVibrator",
                    c"(I)Landroid/os/Vibrator;",
                    &[Arg::Int(id)],
                )?
                .object())
        };
        if vibrators.len() >= 2 {
            let v0 = vibrator(vibrators[0])?;
            self.vibrate_api31(
                env,
                v0.as_ref().map_or(Obj::NULL, Local::obj),
                low_frequency_intensity,
                length,
            )?;
            let v1 = vibrator(vibrators[1])?;
            self.vibrate_api31(
                env,
                v1.as_ref().map_or(Obj::NULL, Local::obj),
                high_frequency_intensity,
                length,
            )?;
        } else if vibrators.len() == 1 {
            let intensity = (low_frequency_intensity * 0.6) + (high_frequency_intensity * 0.4);
            let v0 = vibrator(vibrators[0])?;
            self.vibrate_api31(
                env,
                v0.as_ref().map_or(Obj::NULL, Local::obj),
                intensity,
                length,
            )?;
        }
        Ok(())
    }

    /// Translation of `SDLHapticHandler_API31.vibrate()`.
    fn vibrate_api31<'a>(
        &self,
        env: Env<'a>,
        vibrator: Obj<'_>,
        intensity: f32,
        length: jint,
    ) -> JniResult<'a, ()> {
        if super::super::sdk_int(env) < 31
        /* Android 12.0 (S) */
        {
            /* Silence 'lint' warning */
            return Ok(());
        }

        if vibrator.is_null() {
            return null_pointer(env, c"the Vibrator is null");
        }

        if intensity == 0.0 {
            cancel(env, vibrator)?;
            return Ok(());
        }

        let mut value = java_round(intensity * 255.0);
        if value > 255 {
            value = 255;
        }
        if value < 1 {
            cancel(env, vibrator)?;
            return Ok(());
        }
        vibrate_one_shot(env, vibrator, length, value)
    }

    /// Translation of `stop()`.
    fn stop<'a>(&self, env: Env<'a>, device_id: jint) -> JniResult<'a, ()> {
        if let Some(haptic) = self.get_haptic(device_id) {
            cancel(env, haptic.vib.obj())?;
        }
        Ok(())
    }

    /// The system's vibrator (`VIBRATOR_SERVICE`), if it has one, passed
    /// to `nativeAddHaptic()` once, and to `nativeRemoveHaptic()` once it
    /// hasn't. Translation of `detectHapticDevices()` (synchronized).
    fn detect_haptic_devices<'a>(&self, env: Env<'a>) -> JniResult<'a, ()> {
        let _synchronized = self.monitor.lock();
        const DEVICE_ID_VIBRATOR_SERVICE: jint = 999999;
        let mut has_vibrator_service = false;

        /* Check VIBRATOR_SERVICE */
        if let Some(vib) = system_service(env, VIBRATOR_SERVICE)? {
            has_vibrator_service = env
                .call_method(vib.obj(), c"hasVibrator", c"()Z", &[])?
                .boolean();

            if has_vibrator_service && self.get_haptic(DEVICE_ID_VIBRATOR_SERVICE).is_none() {
                if let Some(vib) = Global::new(env, vib.obj()) {
                    let haptic = Arc::new(SdlHaptic {
                        device_id: DEVICE_ID_VIBRATOR_SERVICE,
                        name: "VIBRATOR_SERVICE".to_owned(),
                        vib,
                    });
                    self.haptics().push(Arc::clone(&haptic));
                    crate::haptic::android::add_haptic(haptic.device_id, &haptic.name);
                }
            }
        }

        /* Check removed devices */
        let removed_devices: Vec<jint> = self
            .haptics()
            .iter()
            .map(|haptic| haptic.device_id)
            // (else: don't remove the vibrator if it is still present)
            .filter(|&device_id| device_id != DEVICE_ID_VIBRATOR_SERVICE || !has_vibrator_service)
            .collect();

        for device_id in removed_devices {
            crate::haptic::android::remove_haptic(device_id);
            let mut haptics = self.haptics();
            if let Some(i) = haptics
                .iter()
                .position(|haptic| haptic.device_id == device_id)
            {
                haptics.remove(i);
            }
        }
        Ok(())
    }

    /// Translation of `getHaptic()` (synchronized).
    fn get_haptic(&self, device_id: jint) -> Option<Arc<SdlHaptic>> {
        let _synchronized = self.monitor.lock();
        self.haptics()
            .iter()
            .find(|haptic| haptic.device_id == device_id)
            .cloned()
    }
}

// ---------------------------------------------------------------------------
// SDLJoySensorListener implements SensorEventListener

/// `new SDLJoySensorListener(device_id)`.
fn new_joy_sensor_listener<'a>(env: Env<'a>, device_id: jint) -> JniResult<'a, Local<'a>> {
    let class = super::super::env::app_class(SDL_JOY_SENSOR_LISTENER)
        .expect("JNI_OnLoad() found SDLJoySensorListener");
    let listener = env.new_object(class.obj(), c"()V", &[])?;
    env.set_field(listener.obj(), c"device_id", c"I", Arg::Int(device_id))?;
    Ok(listener)
}

/// Translation of `SDLJoySensorListener.onAccuracyChanged()`.
pub(crate) extern "system" fn on_accuracy_changed(
    _env: *mut JNIEnv,
    _this: jobject,
    _sensor: jobject,
    _accuracy: jint,
) {
}

/// Translation of `SDLJoySensorListener.onSensorChanged()`.
pub(crate) extern "system" fn on_sensor_changed(env: *mut JNIEnv, this: jobject, event: jobject) {
    // SAFETY: a native method's environment and arguments.
    let (env, this, event) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(this),
            Obj::from_raw(event),
        )
    };
    native(env, |env| {
        let device_id = env.get_field(this, c"device_id", c"I")?.int();
        let sensor = env
            .get_field(event, c"sensor", c"Landroid/hardware/Sensor;")?
            .object();
        if sensor.is_none() {
            return null_pointer(env, c"SensorEvent.sensor is null");
        }
        let sensor_type = env
            .call_method(
                sensor.as_ref().map_or(Obj::NULL, Local::obj),
                c"getType",
                c"()I",
                &[],
            )?
            .int();
        let timestamp = env.get_field(event, c"timestamp", c"J")?.long();
        let values = env.get_field(event, c"values", c"[F")?.object();
        let Some(values) = values else {
            return null_pointer(env, c"SensorEvent.values is null");
        };
        let [x, y, z] = env.get_float_array_region::<3>(values.obj(), 0)?;
        super::super::on_joy_sensor(device_id, sensor_type, timestamp, x, y, z);
        Ok(())
    });
}
