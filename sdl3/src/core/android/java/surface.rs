// Rust translation of android-project/app/src/main/java/org/libsdl/app/
// SDLSurface.java from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `class SDLSurface extends SurfaceView implements SurfaceHolder.Callback,
//! View.OnApplyWindowInsetsListener, View.OnKeyListener,
//! View.OnTouchListener, SensorEventListener,
//! ScaleGestureDetector.OnScaleGestureListener`: what SDL draws on, so it
//! needs to know when it's created in order to do anything useful (that's
//! where the SDL thread is set up, through the activity's state machine:
//! the native state is resumed once the surface is ready, and paused when
//! it's destroyed). Its holder's callbacks pass the surface's life on to
//! the video driver (`onNativeSurfaceCreated()`, `...Changed()`,
//! `...Destroyed()`), with the display's metrics
//! (`nativeSetScreenResolution()`: the surface's size, the real metrics',
//! their density and the refresh rate) and a resize (`onNativeResize()`);
//! the window insets become the window's safe area
//! (`onNativeInsetsChanged()`) and the screen keyboard's visibility; the
//! accelerometer tells the display's rotation (`onNativeRotationChanged()`,
//! as the activity's orientation may be fixed).
//!
//! The constructor's code (the holder's callback, the listeners, the
//! focus, the display and sensor manager) runs in [`create_sdl_surface`],
//! after the DEX file's constructor, which only calls `SurfaceView`'s;
//! the fields are the DEX file's (`super::super::classes`). Note:
//! `scaleGestureDetector` isn't `final` there (it's set from native code).
//!
//! Its touch events (`onTouch()`) are the mouse's, a pen's or a finger's,
//! by each pointer's tool (`onNativeMouse()`, `onNativePen()`,
//! `onNativeTouch()`), and feed the pinch's `ScaleGestureDetector`
//! (`onScale()`, `onScaleBegin()`, `onScaleEnd()`: `onNativePinch*()`);
//! the other motion events go to the generic motion listener
//! (`super::controller_manager`), the captured pointer's too
//! (`onCapturedPointerEvent()`). Its key events (`onKey()`) are
//! `SDLActivity.handleKeyEvent()`'s, without an input connection.

use super::super::env::{Arg, Env, JniResult, Local, Obj};
use super::super::jni::*;
use super::super::ndk::LogPriority;
use super::controller_manager::{
    self, ACTION_POINTER_DOWN, ACTION_POINTER_UP, TOOL_TYPE_ERASER, TOOL_TYPE_MOUSE,
    TOOL_TYPE_STYLUS,
};
use super::{activity, sensor_manager};

/// `Log.v("SDL", message)`.
fn log(message: &str) {
    super::super::ndk::log_write(LogPriority::Verbose, "SDL", message);
}

/// `Sensor.TYPE_ACCELEROMETER`.
pub(crate) const TYPE_ACCELEROMETER: jint = 1;
/// `SensorManager.SENSOR_DELAY_GAME`.
const SENSOR_DELAY_GAME: jint = 1;

/// `android.view.SurfaceView`, the superclass, for `super.method()`.
fn surface_view_superclass<'a>(env: Env<'a>) -> JniResult<'a, Local<'a>> {
    env.find_class(c"android/view/SurfaceView")
}

/// Run a native method's body, whose Java exceptions reach the framework
/// (left pending), as from Java.
fn native<'a>(env: Env<'a>, body: impl FnOnce(Env<'a>) -> JniResult<'a, ()>) {
    if let Err(thrown) = body(env) {
        // (pending, for the framework)
        drop(thrown);
    }
}

/// [`native`] for a method returning a boolean (false when it throws).
fn native_boolean<'a>(env: Env<'a>, body: impl FnOnce(Env<'a>) -> JniResult<'a, bool>) -> jboolean {
    match body(env) {
        Ok(result) => result as jboolean,
        Err(thrown) => {
            // (pending, for the framework)
            drop(thrown);
            JNI_FALSE
        }
    }
}

/// [`native`] for a method returning an object (null when it throws).
fn native_object<'a>(
    env: Env<'a>,
    body: impl FnOnce(Env<'a>) -> JniResult<'a, Option<Local<'a>>>,
) -> jobject {
    match body(env) {
        Ok(Some(result)) => result.into_raw(),
        Ok(None) => std::ptr::null_mut(),
        Err(thrown) => {
            // (pending, for the framework)
            drop(thrown);
            std::ptr::null_mut()
        }
    }
}

/// `context.getSystemService(name)`.
fn system_service<'a>(
    env: Env<'a>,
    context: Obj<'_>,
    name: &str,
) -> JniResult<'a, Option<Local<'a>>> {
    let name = env.new_string(name)?;
    Ok(env
        .call_method(
            context,
            c"getSystemService",
            c"(Ljava/lang/String;)Ljava/lang/Object;",
            &[Arg::Object(name.obj())],
        )?
        .object())
}

/// A `boolean` field of the surface.
fn boolean_field<'a>(env: Env<'a>, this: Obj<'_>, name: &std::ffi::CStr) -> JniResult<'a, bool> {
    Ok(env.get_field(this, name, c"Z")?.boolean())
}

fn set_boolean_field<'a>(
    env: Env<'a>,
    this: Obj<'_>,
    name: &std::ffi::CStr,
    value: bool,
) -> JniResult<'a, ()> {
    env.set_field(this, name, c"Z", Arg::Boolean(value))
}

/// A `float` field of the surface (`mWidth`, `mHeight`).
fn float_field<'a>(env: Env<'a>, this: Obj<'_>, name: &std::ffi::CStr) -> JniResult<'a, f32> {
    Ok(env.get_field(this, name, c"F")?.float())
}

/// `mDisplay`.
fn display<'a>(env: Env<'a>, this: Obj<'_>) -> JniResult<'a, Option<Local<'a>>> {
    Ok(env
        .get_field(this, c"mDisplay", c"Landroid/view/Display;")?
        .object())
}

/// Java's `NullPointerException` (of a null `mDisplay`, ...): thrown.
fn null_pointer<'a, T>(env: Env<'a>, what: &std::ffi::CStr) -> JniResult<'a, T> {
    let npe = env.find_class(c"java/lang/NullPointerException")?;
    env.throw_new(npe.obj(), what);
    env.pending()?;
    unreachable!("ThrowNew() threw")
}

// Startup
/// `protected SDLSurface(Context context)`: the DEX file's constructor
/// (`super(context)`), then the constructor's code. Translation of
/// `SDLActivity.createSDLSurface()` (`new SDLSurface(context)`) and of the
/// constructor.
pub(crate) fn create_sdl_surface<'a>(env: Env<'a>, context: Obj<'_>) -> JniResult<'a, Local<'a>> {
    let class = super::super::env::app_class(super::super::classes::SDL_SURFACE)
        .expect("JNI_OnLoad() found SDLSurface");
    let this = env.new_object(
        class.obj(),
        c"(Landroid/content/Context;)V",
        &[Arg::Object(context)],
    )?;
    let this_obj = this.obj();

    let holder = env
        .call_method(
            this_obj,
            c"getHolder",
            c"()Landroid/view/SurfaceHolder;",
            &[],
        )?
        .object()
        .expect("a holder");
    env.call_method(
        holder.obj(),
        c"addCallback",
        c"(Landroid/view/SurfaceHolder$Callback;)V",
        &[Arg::Object(this_obj)],
    )?;

    let detector_class = env.find_class(c"android/view/ScaleGestureDetector")?;
    let scale_gesture_detector = env.new_object(
        detector_class.obj(),
        c"(Landroid/content/Context;Landroid/view/ScaleGestureDetector$OnScaleGestureListener;)V",
        &[Arg::Object(context), Arg::Object(this_obj)],
    )?;
    env.set_field(
        this_obj,
        c"scaleGestureDetector",
        c"Landroid/view/ScaleGestureDetector;",
        Arg::Object(scale_gesture_detector.obj()),
    )?;

    take_focus_and_listen(env, this_obj)?;

    let window_manager = system_service(env, context, "window")?; // Context.WINDOW_SERVICE
    let Some(window_manager) = window_manager else {
        return null_pointer(env, c"no WindowManager");
    };
    let display = env
        .call_method(
            window_manager.obj(),
            c"getDefaultDisplay",
            c"()Landroid/view/Display;",
            &[],
        )?
        .object();
    env.set_field(
        this_obj,
        c"mDisplay",
        c"Landroid/view/Display;",
        Arg::Object(display.as_ref().map_or(Obj::NULL, Local::obj)),
    )?;
    let sensor_manager = system_service(env, context, "sensor")?; // Context.SENSOR_SERVICE
    env.set_field(
        this_obj,
        c"mSensorManager",
        c"Landroid/hardware/SensorManager;",
        Arg::Object(sensor_manager.as_ref().map_or(Obj::NULL, Local::obj)),
    )?;

    let motion_listener = activity::get_motion_listener(env)?;
    env.call_method(
        this_obj,
        c"setOnGenericMotionListener",
        c"(Landroid/view/View$OnGenericMotionListener;)V",
        &[Arg::Object(motion_listener.obj())],
    )?;

    // Some arbitrary defaults to avoid a potential division by zero
    env.set_field(this_obj, c"mWidth", c"F", Arg::Float(1.0))?;
    env.set_field(this_obj, c"mHeight", c"F", Arg::Float(1.0))?;

    set_boolean_field(env, this_obj, c"mIsSurfaceReady", false)?;

    Ok(this)
}

/// `setFocusable(true)`, `setFocusableInTouchMode(true)`, `requestFocus()`
/// and the listeners (`setOnApplyWindowInsetsListener(this)`,
/// `setOnKeyListener(this)`, `setOnTouchListener(this)`): the constructor's
/// and `handleResume()`'s.
fn take_focus_and_listen<'a>(env: Env<'a>, this: Obj<'_>) -> JniResult<'a, ()> {
    env.call_method(this, c"setFocusable", c"(Z)V", &[Arg::Boolean(true)])?;
    env.call_method(
        this,
        c"setFocusableInTouchMode",
        c"(Z)V",
        &[Arg::Boolean(true)],
    )?;
    env.call_method(this, c"requestFocus", c"()Z", &[])?;
    env.call_method(
        this,
        c"setOnApplyWindowInsetsListener",
        c"(Landroid/view/View$OnApplyWindowInsetsListener;)V",
        &[Arg::Object(this)],
    )?;
    env.call_method(
        this,
        c"setOnKeyListener",
        c"(Landroid/view/View$OnKeyListener;)V",
        &[Arg::Object(this)],
    )?;
    env.call_method(
        this,
        c"setOnTouchListener",
        c"(Landroid/view/View$OnTouchListener;)V",
        &[Arg::Object(this)],
    )?;
    Ok(())
}

/// `mIsSurfaceReady`: is the SurfaceView ready for rendering.
pub(crate) fn is_surface_ready(env: Env<'_>, surface: Obj<'_>) -> bool {
    boolean_field(env, surface, c"mIsSurfaceReady").unwrap_or_else(|thrown| {
        thrown.clear();
        false
    })
}

/// Translation of `handlePause()`.
pub(crate) fn handle_pause<'a>(env: Env<'a>, surface: Obj<'_>) -> JniResult<'a, ()> {
    enable_sensor(env, surface, TYPE_ACCELEROMETER, false)
}

/// Translation of `handleResume()`.
pub(crate) fn handle_resume<'a>(env: Env<'a>, surface: Obj<'_>) -> JniResult<'a, ()> {
    take_focus_and_listen(env, surface)?;
    enable_sensor(env, surface, TYPE_ACCELEROMETER, true)
}

/// The holder's surface. Translation of `getNativeSurface()`.
pub(crate) fn get_native_surface<'a>(
    env: Env<'a>,
    surface: Obj<'_>,
) -> JniResult<'a, Option<Local<'a>>> {
    let holder = env
        .call_method(
            surface,
            c"getHolder",
            c"()Landroid/view/SurfaceHolder;",
            &[],
        )?
        .object()
        .expect("a holder");
    Ok(env
        .call_method(
            holder.obj(),
            c"getSurface",
            c"()Landroid/view/Surface;",
            &[],
        )?
        .object())
}

/// Called when we have a valid drawing surface. Translation of
/// `surfaceCreated()`.
pub(crate) extern "system" fn surface_created(env: *mut JNIEnv, _this: jobject, _holder: jobject) {
    // SAFETY: a native method's environment.
    let env = unsafe { Env::from_raw(env) };
    native(env, |_env| {
        log("surfaceCreated()");
        super::super::on_surface_created();
        Ok(())
    });
}

/// Called when we lose the surface. Translation of `surfaceDestroyed()`.
pub(crate) extern "system" fn surface_destroyed(env: *mut JNIEnv, this: jobject, _holder: jobject) {
    // SAFETY: a native method's environment and object.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    native(env, |env| {
        log("surfaceDestroyed()");

        // Transition to pause, if needed
        activity::set_next_native_state(env, false)?;

        set_boolean_field(env, this, c"mIsSurfaceReady", false)?;
        super::super::on_surface_destroyed();
        Ok(())
    });
}

/// `ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE`, `_PORTRAIT`,
/// `_SENSOR_LANDSCAPE`, `_SENSOR_PORTRAIT`.
const SCREEN_ORIENTATION_LANDSCAPE: jint = 0;
const SCREEN_ORIENTATION_PORTRAIT: jint = 1;
const SCREEN_ORIENTATION_SENSOR_LANDSCAPE: jint = 6;
const SCREEN_ORIENTATION_SENSOR_PORTRAIT: jint = 7;

/// The `try` block of `surfaceChanged()`: the display's real size, and
/// its density (`densityDpi / 160`).
fn real_metrics<'a>(env: Env<'a>, this: Obj<'_>) -> JniResult<'a, (jint, jint, f32)> {
    let metrics_class = env.find_class(c"android/util/DisplayMetrics")?;
    let real_metrics = env.new_object(metrics_class.obj(), c"()V", &[])?;
    let Some(display) = display(env, this)? else {
        return null_pointer(env, c"SDLSurface.mDisplay is null");
    };
    env.call_method(
        display.obj(),
        c"getRealMetrics",
        c"(Landroid/util/DisplayMetrics;)V",
        &[Arg::Object(real_metrics.obj())],
    )?;
    let w = env
        .get_field(real_metrics.obj(), c"widthPixels", c"I")?
        .int();
    let h = env
        .get_field(real_metrics.obj(), c"heightPixels", c"I")?
        .int();
    // Use densityDpi instead of density to more closely match what the UI scale is
    let dpi = env
        .get_field(real_metrics.obj(), c"densityDpi", c"I")?
        .int();
    Ok((w, h, dpi as f32 / 160.0))
}

/// Called when the surface is resized. Translation of `surfaceChanged()`.
pub(crate) extern "system" fn surface_changed(
    env: *mut JNIEnv,
    this: jobject,
    _holder: jobject,
    _format: jint,
    width: jint,
    height: jint,
) {
    // SAFETY: a native method's environment and object.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    native(env, |env| {
        log("surfaceChanged()");

        let Some(singleton) = activity::singleton(env) else {
            return Ok(());
        };

        env.set_field(this, c"mWidth", c"F", Arg::Float(width as f32))?;
        env.set_field(this, c"mHeight", c"F", Arg::Float(height as f32))?;
        let mut n_device_width = width;
        let mut n_device_height = height;
        let mut density = 1.0f32;
        match real_metrics(env, this) {
            Ok((w, h, d)) => {
                n_device_width = w;
                n_device_height = h;
                density = d;
            }
            // catch(Exception ignored)
            Err(thrown) => drop(thrown.catch_exception()?),
        }

        let Some(context) = activity::get_context(env) else {
            // (synchronized of null)
            return null_pointer(env, c"SDLActivity.getContext() is null");
        };
        env.synchronized(context.obj(), || {
            // In case we're waiting on a size change after going fullscreen, send a notification.
            env.call_method(context.obj(), c"notifyAll", c"()V", &[])
                .map(drop)
        })??;

        log(&format!("Window size: {width}x{height}"));
        log(&format!("Device size: {n_device_width}x{n_device_height}"));
        let Some(display) = display(env, this)? else {
            return null_pointer(env, c"SDLSurface.mDisplay is null");
        };
        let rate = env
            .call_method(display.obj(), c"getRefreshRate", c"()F", &[])?
            .float();
        super::super::set_screen_resolution(
            width,
            height,
            n_device_width,
            n_device_height,
            density,
            rate,
        );
        super::super::on_resize();

        // Prevent a screen distortion glitch,
        // for instance when the device is in Landscape and a Portrait App is resumed.
        let mut skip = false;
        let requested_orientation = env
            .call_method(singleton.obj(), c"getRequestedOrientation", c"()I", &[])?
            .int();

        let (m_width, m_height) = (width as f32, height as f32);
        if requested_orientation == SCREEN_ORIENTATION_PORTRAIT
            || requested_orientation == SCREEN_ORIENTATION_SENSOR_PORTRAIT
        {
            if m_width > m_height {
                skip = true;
            }
        } else if (requested_orientation == SCREEN_ORIENTATION_LANDSCAPE
            || requested_orientation == SCREEN_ORIENTATION_SENSOR_LANDSCAPE)
            && m_width < m_height
        {
            skip = true;
        }

        // Special Patch for Square Resolution: Black Berry Passport
        if skip {
            let min = m_width.min(m_height) as f64;
            let max = m_width.max(m_height) as f64;

            if max / min < 1.20 {
                log("Don't skip on such aspect-ratio. Could be a square resolution.");
                skip = false;
            }
        }

        // Don't skip if we might be multi-window or have popup dialogs
        if skip && super::super::sdk_int(env) >= 24
        /* Android 7.0 (N) */
        {
            skip = false;
        }

        if skip {
            log("Skip .. Surface is not ready.");
            set_boolean_field(env, this, c"mIsSurfaceReady", false)?;
            return Ok(());
        }

        /* If the surface has been previously destroyed by onNativeSurfaceDestroyed, recreate it here */
        super::super::on_surface_changed();

        /* Surface is ready */
        set_boolean_field(env, this, c"mIsSurfaceReady", true)?;

        activity::set_next_native_state(env, true)
    });
}

/// Window inset. Translation of `onApplyWindowInsets()`.
pub(crate) extern "system" fn on_apply_window_insets(
    env: *mut JNIEnv,
    this: jobject,
    _v: jobject,
    insets: jobject,
) -> jobject {
    // SAFETY: a native method's environment and arguments.
    let (env, this, insets) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(this),
            Obj::from_raw(insets),
        )
    };
    native_object(env, |env| {
        if super::super::sdk_int(env) >= 30
        /* Android 11 (R) */
        {
            let types = env.find_class(c"android/view/WindowInsets$Type")?;
            let mut mask = 0;
            for name in [
                c"systemBars",
                c"systemGestures",
                c"mandatorySystemGestures",
                c"tappableElement",
                c"displayCutout",
            ] {
                mask |= env
                    .call_static_method(types.obj(), name, c"()I", &[])?
                    .int();
            }
            let combined = env
                .call_method(
                    insets,
                    c"getInsets",
                    c"(I)Landroid/graphics/Insets;",
                    &[Arg::Int(mask)],
                )?
                .object()
                .expect("Insets");
            let side = |name| env.get_field(combined.obj(), name, c"I").map(|v| v.int());
            let (left, right, top, bottom) = (
                side(c"left")?,
                side(c"right")?,
                side(c"top")?,
                side(c"bottom")?,
            );

            super::super::on_insets_changed(left, right, top, bottom);

            let ime = env
                .call_static_method(types.obj(), c"ime", c"()I", &[])?
                .int();
            if env
                .call_method(insets, c"isVisible", c"(I)Z", &[Arg::Int(ime)])?
                .boolean()
            {
                if !boolean_field(env, this, c"mKeyboardVisible")? {
                    set_boolean_field(env, this, c"mKeyboardVisible", true)?;
                    super::super::on_screen_keyboard_shown();
                }
            } else if boolean_field(env, this, c"mKeyboardVisible")? {
                set_boolean_field(env, this, c"mKeyboardVisible", false)?;
                super::super::on_screen_keyboard_hidden();
            }
        }

        // Pass these to any child views in case they need them
        Ok(env.new_local_ref(insets))
    })
}

// Key events
/// Translation of `onKey()`.
pub(crate) extern "system" fn on_key(
    env: *mut JNIEnv,
    _this: jobject,
    v: jobject,
    key_code: jint,
    event: jobject,
) -> jboolean {
    // SAFETY: a native method's environment and arguments.
    let (env, v, event) = unsafe { (Env::from_raw(env), Obj::from_raw(v), Obj::from_raw(event)) };
    native_boolean(env, |env| {
        activity::handle_key_event(env, v, key_code, event, Obj::NULL)
    })
}

/// Translation of `getNormalizedX()`.
fn get_normalized_x<'a>(env: Env<'a>, this: Obj<'_>, x: f32) -> JniResult<'a, f32> {
    let m_width = float_field(env, this, c"mWidth")?;
    Ok(if m_width <= 1.0 {
        0.5
    } else {
        x / (m_width - 1.0)
    })
}

/// Translation of `getNormalizedY()`.
fn get_normalized_y<'a>(env: Env<'a>, this: Obj<'_>, y: f32) -> JniResult<'a, f32> {
    let m_height = float_field(env, this, c"mHeight")?;
    Ok(if m_height <= 1.0 {
        0.5
    } else {
        y / (m_height - 1.0)
    })
}

/// Touch events: each pointer of the event (the one that went down or up,
/// for a non-primary one) to the mouse, a pen or a finger, by its tool,
/// then the event to the pinch's detector. Translation of `onTouch()`.
pub(crate) extern "system" fn on_touch(
    env: *mut JNIEnv,
    this: jobject,
    _v: jobject,
    event: jobject,
) -> jboolean {
    // SAFETY: a native method's environment and arguments.
    let (env, this, event) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(this),
            Obj::from_raw(event),
        )
    };
    native_boolean(env, |env| {
        /* Ref: http://developer.android.com/training/gestures/multi.html */
        let touch_dev_id = env.call_method(event, c"getDeviceId", c"()I", &[])?.int();
        let pointer_count = env
            .call_method(event, c"getPointerCount", c"()I", &[])?
            .int();
        let action = env
            .call_method(event, c"getActionMasked", c"()I", &[])?
            .int();
        let mut i = 0;

        if action == ACTION_POINTER_UP || action == ACTION_POINTER_DOWN {
            i = env
                .call_method(event, c"getActionIndex", c"()I", &[])?
                .int();
        }

        loop {
            let tool_type = controller_manager::get_tool_type(env, event, i)?;

            if tool_type == TOOL_TYPE_MOUSE {
                let button_state = controller_manager::get_button_state(env, event)?;

                // We need to check if we're in relative mouse mode and get the axis offset rather than the x/y values
                // if we are. We'll leverage our existing mouse motion listener
                let motion_listener = activity::get_motion_listener(env)?;
                let x = controller_manager::get_event_x(env, motion_listener.obj(), event, i)?;
                let y = controller_manager::get_event_y(env, motion_listener.obj(), event, i)?;
                // Note (upstream): from API 26 the listener's getEventX()
                // and getEventY() are the event's position (the captured
                // pointer's events, which are relative, come through
                // onCapturedPointerEvent()), so a mouse's touch event in
                // the relative mode (one that isn't captured: an injected
                // one) is sent as a motion by its position.
                let relative = controller_manager::in_relative_mode(env, motion_listener.obj())?;

                super::super::on_mouse(button_state, action, x, y, relative);
            } else if tool_type == TOOL_TYPE_STYLUS || tool_type == TOOL_TYPE_ERASER {
                let pointer_id = controller_manager::get_pointer_id(env, event, i)?;
                let x = controller_manager::get_x(env, event, i)?;
                let y = controller_manager::get_y(env, event, i)?;
                let p = controller_manager::get_pressure(env, event, i)?;

                // BUTTON_STYLUS_PRIMARY is 2^5, so shift by 4, and apply SDL_PEN_INPUT_DOWN/SDL_PEN_INPUT_ERASER_TIP
                let button_state = controller_manager::pen_buttons(env, event, tool_type)?;

                let motion_listener = activity::get_motion_listener(env)?;
                let device = controller_manager::get_device(env, event)?;
                let device_type = controller_manager::get_pen_device_type(
                    env,
                    motion_listener.obj(),
                    device.as_ref().map_or(Obj::NULL, Local::obj),
                )?;
                super::super::on_pen(pointer_id, device_type, button_state, action, x, y, p);
            } else {
                // MotionEvent.TOOL_TYPE_FINGER or MotionEvent.TOOL_TYPE_UNKNOWN
                let pointer_id = controller_manager::get_pointer_id(env, event, i)?;
                let x = get_normalized_x(env, this, controller_manager::get_x(env, event, i)?)?;
                let y = get_normalized_y(env, this, controller_manager::get_y(env, event, i)?)?;
                let p = controller_manager::get_pressure(env, event, i)?;

                super::super::on_touch(touch_dev_id, pointer_id, action, x, y, p);
            }

            // Non-primary up/down
            if action == ACTION_POINTER_UP || action == ACTION_POINTER_DOWN {
                break;
            }
            i += 1;
            if i >= pointer_count {
                break;
            }
        }

        let detector = env
            .get_field(
                this,
                c"scaleGestureDetector",
                c"Landroid/view/ScaleGestureDetector;",
            )?
            .object();
        let Some(detector) = detector else {
            return null_pointer(env, c"SDLSurface.scaleGestureDetector is null");
        };
        env.call_method(
            detector.obj(),
            c"onTouchEvent",
            c"(Landroid/view/MotionEvent;)Z",
            &[Arg::Object(event)],
        )?;

        Ok(true)
    })
}

// Sensor events
/// TODO: This uses getDefaultSensor - what if we have >1 accels?
/// Translation of `enableSensor()`.
pub(crate) fn enable_sensor<'a>(
    env: Env<'a>,
    surface: Obj<'_>,
    sensortype: i32,
    enabled: bool,
) -> JniResult<'a, ()> {
    let manager = env
        .get_field(
            surface,
            c"mSensorManager",
            c"Landroid/hardware/SensorManager;",
        )?
        .object();
    let Some(manager) = manager else {
        return null_pointer(env, c"SDLSurface.mSensorManager is null");
    };
    let sensor = env
        .call_method(
            manager.obj(),
            c"getDefaultSensor",
            c"(I)Landroid/hardware/Sensor;",
            &[Arg::Int(sensortype)],
        )?
        .object();
    let sensor = sensor.as_ref().map_or(Obj::NULL, Local::obj);
    if enabled {
        sensor_manager::register_listener(env, manager.obj(), surface, sensor, SENSOR_DELAY_GAME)
    } else {
        sensor_manager::unregister_listener(env, manager.obj(), surface, sensor)
    }
}

/// Translation of `onAccuracyChanged()`.
pub(crate) extern "system" fn on_accuracy_changed(
    _env: *mut JNIEnv,
    _this: jobject,
    _sensor: jobject,
    _accuracy: jint,
) {
    // TODO
}

/// `Surface.ROTATION_90`, `_180`, `_270`.
const ROTATION_90: jint = 1;
const ROTATION_180: jint = 2;
const ROTATION_270: jint = 3;

/// Since we may have an orientation set, we won't receive
/// `onConfigurationChanged` events: the accelerometer's events tell the
/// display's rotation. Translation of `onSensorChanged()`. (Its `x` and
/// `y`, the event's values turned with the display, are unused upstream:
/// left out.)
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
        let sensor = env
            .get_field(event, c"sensor", c"Landroid/hardware/Sensor;")?
            .object();
        let Some(sensor) = sensor else {
            return null_pointer(env, c"SensorEvent.sensor is null");
        };
        if env
            .call_method(sensor.obj(), c"getType", c"()I", &[])?
            .int()
            == TYPE_ACCELEROMETER
        {
            // Since we may have an orientation set, we won't receive onConfigurationChanged events.
            // We thus should check here.
            let Some(display) = display(env, this)? else {
                return null_pointer(env, c"SDLSurface.mDisplay is null");
            };
            let new_rotation = match env
                .call_method(display.obj(), c"getRotation", c"()I", &[])?
                .int()
            {
                ROTATION_90 => 90,
                ROTATION_180 => 180,
                ROTATION_270 => 270,
                // Surface.ROTATION_0, default
                _ => 0,
            };

            if new_rotation != activity::current_rotation() {
                activity::set_current_rotation(new_rotation);
                super::super::on_rotation_changed(new_rotation);
            }
        }
        Ok(())
    });
}

/// Prevent android internal NullPointerException
/// (https://github.com/libsdl-org/SDL/issues/13306). Translation of
/// `onResolvePointerIcon()`.
pub(crate) extern "system" fn on_resolve_pointer_icon(
    env: *mut JNIEnv,
    this: jobject,
    event: jobject,
    pointer_index: jint,
) -> jobject {
    // SAFETY: a native method's environment and arguments.
    let (env, this, event) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(this),
            Obj::from_raw(event),
        )
    };
    native_object(env, |env| {
        let superclass = surface_view_superclass(env)?;
        match env.call_nonvirtual_method(
            this,
            superclass.obj(),
            c"onResolvePointerIcon",
            c"(Landroid/view/MotionEvent;I)Landroid/view/PointerIcon;",
            &[Arg::Object(event), Arg::Int(pointer_index)],
        ) {
            Ok(icon) => Ok(icon.object()),
            Err(thrown) => {
                // catch (NullPointerException e)
                drop(thrown.catch_any(&[c"java/lang/NullPointerException"])?);
                Ok(None)
            }
        }
    })
}

/// Captured pointer events for API 26: the motion listener's. Translation
/// of `onCapturedPointerEvent()`.
pub(crate) extern "system" fn on_captured_pointer_event(
    env: *mut JNIEnv,
    this: jobject,
    event: jobject,
) -> jboolean {
    // SAFETY: a native method's environment and arguments.
    let (env, this, event) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(this),
            Obj::from_raw(event),
        )
    };
    native_boolean(env, |env| {
        let motion_listener = activity::get_motion_listener(env)?;
        controller_manager::generic_motion(env, motion_listener.obj(), this, event)
    })
}

/// The detector's span and focus, normalized: `(span_x, span_y, focus_x,
/// focus_y)`.
fn pinch<'a>(
    env: Env<'a>,
    this: Obj<'_>,
    detector: Obj<'_>,
) -> JniResult<'a, (f32, f32, f32, f32)> {
    let get = |name: &std::ffi::CStr| -> JniResult<'a, f32> {
        Ok(env.call_method(detector, name, c"()F", &[])?.float())
    };
    let span_x = get_normalized_x(env, this, get(c"getCurrentSpanX")?)?;
    let span_y = get_normalized_y(env, this, get(c"getCurrentSpanY")?)?;
    let focus_x = get_normalized_x(env, this, get(c"getFocusX")?)?;
    let focus_y = get_normalized_y(env, this, get(c"getFocusY")?)?;
    Ok((span_x, span_y, focus_x, focus_y))
}

/// Translation of `onScale()`.
pub(crate) extern "system" fn on_scale(
    env: *mut JNIEnv,
    this: jobject,
    detector: jobject,
) -> jboolean {
    // SAFETY: a native method's environment and arguments.
    let (env, this, detector) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(this),
            Obj::from_raw(detector),
        )
    };
    native_boolean(env, |env| {
        let scale = env
            .call_method(detector, c"getScaleFactor", c"()F", &[])?
            .float();
        let (span_x, span_y, focus_x, focus_y) = pinch(env, this, detector)?;
        super::super::on_pinch_update(scale, span_x, span_y, focus_x, focus_y);
        Ok(true)
    })
}

/// Translation of `onScaleBegin()`.
pub(crate) extern "system" fn on_scale_begin(
    env: *mut JNIEnv,
    this: jobject,
    detector: jobject,
) -> jboolean {
    // SAFETY: a native method's environment and arguments.
    let (env, this, detector) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(this),
            Obj::from_raw(detector),
        )
    };
    native_boolean(env, |env| {
        let (span_x, span_y, focus_x, focus_y) = pinch(env, this, detector)?;
        super::super::on_pinch_start(span_x, span_y, focus_x, focus_y);
        Ok(true)
    })
}

/// Translation of `onScaleEnd()`.
pub(crate) extern "system" fn on_scale_end(_env: *mut JNIEnv, _this: jobject, _detector: jobject) {
    super::super::on_pinch_end();
}
