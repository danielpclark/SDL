// Rust translation of android-project/app/src/main/java/org/libsdl/app/
// SDLControllerManager.java from Simple DirectMedia Layer (part).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see
// LICENSE.txt.

//! The generic motion listeners, `class SDLGenericMotionListener_API14
//! implements View.OnGenericMotionListener` and its subclasses
//! `SDLGenericMotionListener_API24`, `_API26` and `_API29`: the surface's
//! listener of the motion events that aren't touches (the mouse hovering
//! and its wheel, the stylus hovering; a joystick's, which go to the
//! joystick handler), and of the captured pointer's (the relative mouse
//! mode, `SDLSurface.onCapturedPointerEvent()`), each newer one with more
//! of the relative mode: none before API 24, the mouse's relative axes from
//! API 24, the pointer captured from API 26 (the content view's
//! `requestPointerCapture()`), and the pen's device type from API 29.
//! `SDLActivity.getMotionListener()` makes the one of the device's API
//! level.
//!
//! The classes are the DEX file's (`super::super::classes`: the framework
//! calls `onGenericMotion()`, which is native), with their instance
//! fields (`mRelativeModeEnabled`, which API 24's and API 26's each have:
//! API 26's shadows API 24's); their other methods are Rust functions,
//! which dispatch on the listener's class as Java's virtual calls do
//! ([`Api`]).
//!
//! Not translated yet (with the joysticks): the rest of the file, the
//! controller manager, its joystick and haptic handlers, and so the
//! joystick's motion events: `handleJoystickMotionEvent()` handles none
//! (`mJoystickHandler` is null until `SDLControllerManager.initialize()`).

use super::super::env::{Arg, Env, JniResult, Local, Obj};
use super::super::jni::*;
use super::activity;

use super::super::classes::{
    SDL_GENERIC_MOTION_LISTENER_API14, SDL_GENERIC_MOTION_LISTENER_API24,
    SDL_GENERIC_MOTION_LISTENER_API26, SDL_GENERIC_MOTION_LISTENER_API29,
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
        return Ok(handle_joystick_motion_event(env, event));
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

/// Translation of `SDLControllerManager.handleJoystickMotionEvent()`:
/// `mJoystickHandler`, which `SDLControllerManager.initialize()` makes, is
/// null until the joysticks' translation, so no event is handled.
fn handle_joystick_motion_event(_env: Env<'_>, _event: Obj<'_>) -> bool {
    // (mJoystickHandler != null && mJoystickHandler.handleMotionEvent(event))
    false
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
