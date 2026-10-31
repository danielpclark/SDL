// Rust translation of android-project/app/src/main/java/org/libsdl/app/
// SDLActivity.java from Simple DirectMedia Layer (part).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see
// LICENSE.txt.

//! `public class SDLActivity extends Activity` (the app's activity, which
//! the manifest names), and `class SDLMain implements Runnable` (the SDL
//! thread's code): the activity's creation (`onCreate()`: the libraries,
//! the error dialog when they can't be loaded, `SDL.setupJNI()`,
//! `SDL.initialize()`, the locale and the dark mode), its lifecycle
//! (pause, resume, focus, trimmed memory, configuration changes,
//! destruction), which it passes to SDL's thread through the native state
//! machine (`handleNativeState()`, which starts the "SDLThread" thread,
//! which runs the app's `SDL_main` through `main()` and
//! `nativeRunMain()`, and finishes the activity when it returns), the
//! back button (`onBackPressed()`, the trap's `OnBackInvokedCallback` from
//! API 33, `manualBackButton()`), `dispatchKeyEvent()`, the command
//! handler (`SDLCommandHandler`, the messages `sendCommand()` sends to the
//! main thread), the form factor (`isAndroidTV()`, `isTablet()`,
//! `isChromebook()`, `isDeXMode()`, `getDeviceFormFactor()`), and the
//! manifest's environment variables (`getManifestEnvironmentVariables()`).
//!
//! Its layout: the surface (`SDLSurface`, [`super::surface`], made when
//! video is compiled, `createSDLSurface()`) in a `RelativeLayout`, the
//! activity's content view, with the display's natural orientation and
//! rotation passed down; and what the activity offers the video driver:
//! the window style (fullscreen and immersive, `setWindowStyle()`, with
//! the system UI's visibility listener, `onSystemUiVisibilityChange()`,
//! which hides the bars again), the title (`setActivityTitle()`), the
//! orientation (`setOrientation()`, from the window's size and
//! `SDL_HINT_ORIENTATIONS`), `minimizeWindow()`, `getNativeSurface()`. The
//! state machine resumes once the surface is ready and has the focus.
//!
//! The input it offers the video driver: the motion listener of the
//! device's API level (`getMotionListener()`, the surface's generic motion
//! listener: `super::controller_manager`), the touch devices
//! (`initTouch()`), the cursors (`createCustomCursor()`,
//! `destroyCustomCursor()`, `setCustomCursor()`, `setSystemCursor()`: the
//! surface's `PointerIcon`) and the relative mouse mode
//! (`supportsRelativeMouse()`, `setRelativeMouseEnabled()`, the pointer
//! captured again when the activity gets the focus back); the keys of its
//! views (`handleKeyEvent()`: a game controller's to the controller
//! manager first, the mouse's back and forward keys dropped, the others
//! SDL's, with their text, `isTextInputEvent()`); and text input: its text
//! edit (`mTextEdit`, an `SDLDummyEdit`: `super::dummy_edit`), put over
//! the text input area, focused and given the input method's keyboard by
//! `showTextInput()` (its `ShowTextInputTask`, a class of the DEX file),
//! and hidden by the command handler's `COMMAND_TEXTEDIT_HIDE`.
//!
//! What it offers SDL's clipboard, message boxes and file dialogs is in
//! modules of its own: [`super::clipboard_handler`] (`mClipboardHandler`,
//! an `SDLClipboardHandler`, made here when video is compiled, and
//! `clipboardHasText()`, `clipboardGetText()`, `clipboardSetText()`),
//! [`super::messagebox`] (`messageboxShowMessageBox()`) and
//! [`super::file_dialog`] (`showFileDialog()` and the picker's result,
//! `onActivityResult()`). The "Open with" of another app (the intent's
//! data, a file's path) is a drop, sent by `onCreate()`.
//!
//! The controller manager's input device listener is registered by
//! `onCreate()` ([`super::controller_manager`]), and the audio manager is
//! released by `onDestroy()`. Not translated yet, and so skipped where
//! upstream does it: the HID manager, which comes with the HID backend.
//! What it offers the
//! small backends is here: permissions (`requestPermission()`,
//! `onRequestPermissionsResult()`), URLs (`openURL()`), toasts
//! (`showToast()`), the content resolver's files (`openFileDescriptor()`)
//! and the locales (`getPreferredLocales()`).
//!
//! Note: the activity's methods that upstream's subclasses may override
//! (`getLibraries()`, `getArguments()`, `onUnhandledMessage()`, ...)
//! can't be overridden here: an app's Java can't extend this
//! `SDLActivity`.

use super::super::env::{self, Arg, Env, Global, JniResult, Local, Obj};
use super::super::jni::*;
use super::super::ndk::LogPriority;
use super::{anonymous, controller_manager, sdl, surface};
use std::collections::BTreeMap;
use std::ffi::CStr;
use std::sync::Mutex;

const TAG: &str = "SDL";
const SDL_MAJOR_VERSION: u16 = 3;
const SDL_MINOR_VERSION: u16 = 5;
const SDL_MICRO_VERSION: u16 = 0;

/// `Log.v(TAG, message)` (and `Log.e()`, ...).
fn log(priority: LogPriority, message: &str) {
    super::super::ndk::log_write(priority, TAG, message);
}

/// Handle the state of the native layer. Translation of `NativeState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeState {
    Init,
    Resumed,
    Paused,
}

/// SDLActivity's statics.
struct Statics {
    is_resumed_called: bool,
    has_focus: bool,
    /// `mCurrentLocale`.
    current_locale: Option<Global>,

    next_native_state: NativeState,
    current_native_state: NativeState,
    /// If shared libraries (e.g. SDL or the native application) could not
    /// be loaded.
    broken_libraries: bool,

    // Main components
    singleton: Option<Global>,
    /// `mSurface`.
    surface: Option<Global>,
    /// `mLayout`.
    layout: Option<Global>,
    /// `mCurrentRotation`.
    current_rotation: jint,

    // This is what SDL runs in. It invokes SDL_main(), eventually
    sdl_thread: Option<Global>,
    sdl_main_finished: bool,
    activity_created: bool,
    dispatching_key_event: bool,

    /// `backButtonCallback`.
    back_button_callback: Option<Global>,
    /// `mBackKeyTrapEnabled`.
    back_key_trap_enabled: bool,

    /// `mFullscreenModeActive`.
    fullscreen_mode_active: bool,

    /// `mCursors`: the custom cursors' `PointerIcon`s, by their IDs.
    cursors: BTreeMap<jint, Global>,
    /// `mLastCursorID`.
    last_cursor_id: jint,
    /// `mMotionListener`.
    motion_listener: Option<Global>,
    /// `mTextEdit`, the text edit (an `SDLDummyEdit`), once text input has
    /// started.
    text_edit: Option<Global>,
}

static STATICS: Mutex<Statics> = Mutex::new(Statics {
    is_resumed_called: false,
    has_focus: false,
    current_locale: None,
    next_native_state: NativeState::Init,
    current_native_state: NativeState::Init,
    broken_libraries: true,
    singleton: None,
    surface: None,
    layout: None,
    current_rotation: 0,
    sdl_thread: None,
    sdl_main_finished: false,
    activity_created: false,
    dispatching_key_event: false,
    back_button_callback: None,
    back_key_trap_enabled: false,
    fullscreen_mode_active: false,
    cursors: BTreeMap::new(),
    last_cursor_id: 0,
    motion_listener: None,
    text_edit: None,
});

fn statics() -> std::sync::MutexGuard<'static, Statics> {
    STATICS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The motion listener of the device's API level, made once. Translation
/// of `getMotionListener()`.
pub(crate) fn get_motion_listener<'a>(env: Env<'a>) -> JniResult<'a, Local<'a>> {
    if let Some(listener) = statics()
        .motion_listener
        .as_ref()
        .and_then(|l| env.new_local_ref(l.obj()))
    {
        return Ok(listener);
    }
    let listener = controller_manager::new_listener(
        env,
        controller_manager::api_for(super::super::sdk_int(env)),
    )?;
    statics().motion_listener = Global::new(env, listener.obj());
    Ok(listener)
}

/// `mHasMultiWindow`: `Build.VERSION.SDK_INT >= 24 /* Android 7.0 (N) */`.
fn has_multi_window(env: Env<'_>) -> bool {
    super::super::sdk_int(env) >= 24 /* Android 7.0 (N) */
}

/// `android.app.Activity`, the superclass, for `super.method()`.
fn activity_superclass<'a>(env: Env<'a>) -> JniResult<'a, Local<'a>> {
    env.find_class(c"android/app/Activity")
}

/// `super.name()` of a method returning `void`.
pub(crate) fn call_super<'a>(
    env: Env<'a>,
    this: Obj<'_>,
    name: &CStr,
    descriptor: &CStr,
    args: &[Arg<'_>],
) -> JniResult<'a, ()> {
    let superclass = activity_superclass(env)?;
    env.call_nonvirtual_method(this, superclass.obj(), name, descriptor, args)?;
    Ok(())
}

/// A static `String` field of `android.os.Build` (`Build.MANUFACTURER`),
/// `None` for null.
fn build_field<'a>(env: Env<'a>, field: &CStr) -> JniResult<'a, Option<String>> {
    let build = env.find_class(c"android/os/Build")?;
    let value = env
        .get_static_field(build.obj(), field, c"Ljava/lang/String;")?
        .object();
    value.map(|s| env.get_string(s.obj())).transpose()
}

/// A static `String` field of `android.os.Build`, as Java's string
/// concatenation has it (`"null"` for null).
fn build_string(env: Env<'_>, field: &CStr) -> String {
    match build_field(env, field) {
        Ok(value) => value.unwrap_or_else(|| "null".to_string()),
        Err(thrown) => {
            thrown.clear();
            "null".to_string()
        }
    }
}

/// Run a native method's body, whose Java exceptions (those it doesn't
/// catch) reach the framework, as from Java: left pending.
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

/// Java's `NullPointerException` (of a static that is null, as
/// `mSingleton` before `onCreate()`): thrown.
fn null_pointer<'a, T>(env: Env<'a>, what: &CStr) -> JniResult<'a, T> {
    let npe = env.find_class(c"java/lang/NullPointerException")?;
    env.throw_new(npe.obj(), what);
    env.pending()?;
    unreachable!("ThrowNew() threw")
}

/// `mSingleton`, or the `NullPointerException` of using it before
/// `onCreate()`.
fn singleton_or_throw<'a>(env: Env<'a>) -> JniResult<'a, Local<'a>> {
    match singleton(env) {
        Some(singleton) => Ok(singleton),
        None => null_pointer(env, c"SDLActivity.mSingleton is null"),
    }
}

/// `context instanceof Activity`.
fn is_activity(env: Env<'_>, context: Obj<'_>) -> bool {
    match env.find_class(c"android/app/Activity") {
        Ok(activity) => env.is_instance_of(context, activity.obj()),
        Err(thrown) => {
            thrown.clear();
            false
        }
    }
}

/// `obj instanceof Integer`, and its value.
fn integer_value<'a>(env: Env<'a>, obj: Obj<'_>) -> JniResult<'a, Option<jint>> {
    let integer = env.find_class(c"java/lang/Integer")?;
    if obj.is_null() || !env.is_instance_of(obj, integer.obj()) {
        return Ok(None);
    }
    Ok(Some(env.call_method(obj, c"intValue", c"()I", &[])?.int()))
}

/// `Integer.valueOf(value)` (Java's boxing).
fn new_integer<'a>(env: Env<'a>, value: jint) -> JniResult<'a, Local<'a>> {
    let integer = env.find_class(c"java/lang/Integer")?;
    Ok(env
        .call_static_method(
            integer.obj(),
            c"valueOf",
            c"(I)Ljava/lang/Integer;",
            &[Arg::Int(value)],
        )?
        .object()
        .expect("Integer.valueOf() boxes"))
}

/// `SDL_ORIENTATION_*`, as SDL's `SDL_DisplayOrientation`.
const SDL_ORIENTATION_UNKNOWN: jint = 0;
const SDL_ORIENTATION_LANDSCAPE: jint = 1;
const SDL_ORIENTATION_PORTRAIT: jint = 3;

/// `Configuration.ORIENTATION_PORTRAIT` and `_LANDSCAPE`.
const ORIENTATION_PORTRAIT: jint = 1;
const ORIENTATION_LANDSCAPE: jint = 2;

/// `Surface.ROTATION_0`, `_90`, `_180`, `_270`.
const ROTATION_0: jint = 0;
const ROTATION_90: jint = 1;
const ROTATION_180: jint = 2;
const ROTATION_270: jint = 3;

/// `activity.getWindowManager().getDefaultDisplay()`.
fn default_display<'a>(env: Env<'a>, activity: Obj<'_>) -> JniResult<'a, Local<'a>> {
    let window_manager = env
        .call_method(
            activity,
            c"getWindowManager",
            c"()Landroid/view/WindowManager;",
            &[],
        )?
        .object()
        .expect("a window manager");
    Ok(env
        .call_method(
            window_manager.obj(),
            c"getDefaultDisplay",
            c"()Landroid/view/Display;",
            &[],
        )?
        .object()
        .expect("a display"))
}

/// The display's natural orientation (`SDL_ORIENTATION_LANDSCAPE` or
/// `_PORTRAIT`; unknown without an activity), from its rotation and the
/// configuration's orientation. Translation of `getNaturalOrientation()`.
pub(crate) fn get_natural_orientation<'a>(env: Env<'a>) -> JniResult<'a, jint> {
    let mut result = SDL_ORIENTATION_UNKNOWN;

    if let Some(activity) = get_context(env) {
        let config = configuration(env, activity.obj())?;
        let display = default_display(env, activity.obj())?;
        let rotation = env
            .call_method(display.obj(), c"getRotation", c"()I", &[])?
            .int();
        let orientation = env.get_field(config.obj(), c"orientation", c"I")?.int();
        if ((rotation == ROTATION_0 || rotation == ROTATION_180)
            && orientation == ORIENTATION_LANDSCAPE)
            || ((rotation == ROTATION_90 || rotation == ROTATION_270)
                && orientation == ORIENTATION_PORTRAIT)
        {
            result = SDL_ORIENTATION_LANDSCAPE;
        } else {
            result = SDL_ORIENTATION_PORTRAIT;
        }
    }
    Ok(result)
}

/// The display's rotation, in degrees (0 without an activity).
/// Translation of `getCurrentRotation()`.
pub(crate) fn get_current_rotation<'a>(env: Env<'a>) -> JniResult<'a, jint> {
    let mut result = 0;

    if let Some(activity) = get_context(env) {
        let display = default_display(env, activity.obj())?;
        match env
            .call_method(display.obj(), c"getRotation", c"()I", &[])?
            .int()
        {
            ROTATION_0 => result = 0,
            ROTATION_90 => result = 90,
            ROTATION_180 => result = 180,
            ROTATION_270 => result = 270,
            _ => {}
        }
    }
    Ok(result)
}

/// `mCurrentRotation`.
pub(crate) fn current_rotation() -> jint {
    statics().current_rotation
}

/// `mCurrentRotation = rotation`.
pub(crate) fn set_current_rotation(rotation: jint) {
    statics().current_rotation = rotation;
}

/// `mNextNativeState = RESUMED` (`resumed`) or `PAUSED`, then
/// `handleNativeState()`: the surface's, when it's ready or destroyed.
pub(crate) fn set_next_native_state<'a>(env: Env<'a>, resumed: bool) -> JniResult<'a, ()> {
    statics().next_native_state = if resumed {
        NativeState::Resumed
    } else {
        NativeState::Paused
    };
    handle_native_state(env)
}

/// `context.getResources().getConfiguration()`.
fn configuration<'a>(env: Env<'a>, context: Obj<'_>) -> JniResult<'a, Local<'a>> {
    let resources = env
        .call_method(
            context,
            c"getResources",
            c"()Landroid/content/res/Resources;",
            &[],
        )?
        .object()
        .expect("the resources");
    Ok(env
        .call_method(
            resources.obj(),
            c"getConfiguration",
            c"()Landroid/content/res/Configuration;",
            &[],
        )?
        .object()
        .expect("a configuration"))
}

/// `Configuration.UI_MODE_NIGHT_MASK`.
const UI_MODE_NIGHT_MASK: jint = 0x30;
/// `Configuration.UI_MODE_NIGHT_NO`.
const UI_MODE_NIGHT_NO: jint = 0x10;
/// `Configuration.UI_MODE_NIGHT_YES`.
const UI_MODE_NIGHT_YES: jint = 0x20;

/// `switch (configuration.uiMode & Configuration.UI_MODE_NIGHT_MASK)`:
/// the dark mode to SDL (`onNativeDarkModeChanged()`), when it says.
fn pass_dark_mode<'a>(env: Env<'a>, configuration: Obj<'_>) -> JniResult<'a, ()> {
    let ui_mode = env.get_field(configuration, c"uiMode", c"I")?.int();
    match ui_mode & UI_MODE_NIGHT_MASK {
        UI_MODE_NIGHT_NO => super::super::on_dark_mode_changed(false),
        UI_MODE_NIGHT_YES => super::super::on_dark_mode_changed(true),
        _ => {}
    }
    Ok(())
}

/// The application entry point, called on a dedicated thread
/// (SDLThread): `getMainSharedObject()`'s `getMainFunction()`, with
/// `getArguments()`. Translation of `main()`.
fn main(env: Env<'_>) -> JniResult<'_, ()> {
    let library = get_main_shared_object(env)?;
    let function = get_main_function();
    let arguments = get_arguments(env)?;

    log(
        LogPriority::Verbose,
        &format!("Running main function {function} from library {library}"),
    );
    let library = env.new_string(&library)?;
    let function = env.new_string(function)?;
    super::super::run_main(env, library.obj(), function.obj(), arguments.obj());
    log(LogPriority::Verbose, "Finished main function");
    Ok(())
}

/// This method returns the name of the shared object with the application
/// entry point. Translation of `getMainSharedObject()`.
fn get_main_shared_object(env: Env<'_>) -> JniResult<'_, String> {
    let libraries = get_libraries();
    let library = match libraries.last() {
        Some(last) => format!("lib{last}.so"),
        None => "libmain.so".to_string(),
    };
    let context = sdl::get_context(env).expect("the activity's context");
    let info = env
        .call_method(
            context.obj(),
            c"getApplicationInfo",
            c"()Landroid/content/pm/ApplicationInfo;",
            &[],
        )?
        .object()
        .expect("an ApplicationInfo");
    let dir = env
        .get_field(info.obj(), c"nativeLibraryDir", c"Ljava/lang/String;")?
        .object()
        .map(|d| env.get_string(d.obj()))
        .transpose()?
        .unwrap_or_else(|| "null".to_string());
    Ok(format!("{dir}/{library}"))
}

/// This method returns the name of the application entry point.
/// Translation of `getMainFunction()`.
fn get_main_function() -> &'static str {
    "SDL_main"
}

/// The names of the shared libraries to load. Translation of
/// `getLibraries()`: Note: upstream's are `SDL3` and `main`; SDL is
/// linked into `libmain.so` here.
fn get_libraries() -> &'static [&'static str] {
    &["main"]
}

/// Load the .so. Translation of `loadLibraries()`. (The class's static
/// initializer has loaded `main` already, before the framework could call
/// `onCreate()`, which is native: loading it again finds it loaded.)
fn load_libraries<'a>(env: Env<'a>, this: Obj<'_>) -> JniResult<'a, ()> {
    for lib in get_libraries() {
        sdl::load_library(env, lib, this)?;
    }
    Ok(())
}

/// The arguments after the application name. Translation of
/// `getArguments()`: none (`new String[0]`).
fn get_arguments(env: Env<'_>) -> JniResult<'_, Local<'_>> {
    let string = env.find_class(c"java/lang/String")?;
    env.new_object_array(0, string.obj(), Obj::NULL)
}

/// Translation of `getInitSubsystems()`.
fn get_init_subsystems() -> i32 {
    sdl::SDL_INIT_EVERYTHING
}

/// The static nature of the singleton and Android quirkyness force us to
/// initialize everything here. Translation of `initialize()`.
pub(crate) fn initialize() {
    // The static nature of the singleton and Android quirkyness force us to initialize everything here
    // Otherwise, when exiting the app and returning to it, these variables *keep* their pre exit values
    let mut s = statics();
    s.singleton = None;
    s.surface = None;
    s.text_edit = None;
    s.layout = None;
    drop(s);
    super::clipboard_handler::reset();
    let mut s = statics();
    s.cursors = BTreeMap::new();
    s.last_cursor_id = 0;
    s.sdl_thread = None;
    s.is_resumed_called = false;
    s.has_focus = true;
    s.next_native_state = NativeState::Init;
    s.current_native_state = NativeState::Init;
}

/// `System.err.println(message)`.
fn system_err_println<'a>(env: Env<'a>, message: Obj<'_>) -> JniResult<'a, ()> {
    let system = env.find_class(c"java/lang/System")?;
    let err = env
        .get_static_field(system.obj(), c"err", c"Ljava/io/PrintStream;")?
        .object()
        .expect("System.err");
    env.call_method(
        err.obj(),
        c"println",
        c"(Ljava/lang/String;)V",
        &[Arg::Object(message)],
    )?;
    Ok(())
}

/// `e.getMessage()`, and its text as Java's concatenation has it.
fn message_of<'a>(env: Env<'a>, e: Obj<'_>) -> JniResult<'a, (Option<Local<'a>>, String)> {
    let message = env
        .call_method(e, c"getMessage", c"()Ljava/lang/String;", &[])?
        .object();
    let text = match &message {
        Some(m) => env.get_string(m.obj())?,
        None => "null".to_string(),
    };
    Ok((message, text))
}

// Setup
/// Translation of `onCreate()`. Note: the activity's `commandHandler` and
/// `rehideSystemUi`, which upstream's instance initializers make, are made
/// here first (on the same thread: the constructor has no code of its own
/// here).
pub(crate) extern "system" fn on_create(
    env: *mut JNIEnv,
    this: jobject,
    saved_instance_state: jobject,
) {
    // SAFETY: a native method's environment and arguments.
    let (env, this, saved_instance_state) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(this),
            Obj::from_raw(saved_instance_state),
        )
    };
    native(env, |env| {
        // Handler for the messages
        let handler_class = env::app_class(super::super::classes::SDL_COMMAND_HANDLER)
            .expect("JNI_OnLoad() found SDLCommandHandler");
        let command_handler = env.new_object(handler_class.obj(), c"()V", &[])?;
        env.set_field(
            this,
            c"commandHandler",
            c"Landroid/os/Handler;",
            Arg::Object(command_handler.obj()),
        )?;
        let rehide_system_ui = new_rehide_system_ui(env)?;
        env.set_field(
            this,
            c"rehideSystemUi",
            c"Ljava/lang/Runnable;",
            Arg::Object(rehide_system_ui.obj()),
        )?;

        log(
            LogPriority::Verbose,
            &format!("Manufacturer: {}", build_string(env, c"MANUFACTURER")),
        );
        log(
            LogPriority::Verbose,
            &format!("Device: {}", build_string(env, c"DEVICE")),
        );
        log(
            LogPriority::Verbose,
            &format!("Model: {}", build_string(env, c"MODEL")),
        );
        log(LogPriority::Verbose, "onCreate()");
        call_super(
            env,
            this,
            c"onCreate",
            c"(Landroid/os/Bundle;)V",
            &[Arg::Object(saved_instance_state)],
        )?;

        if super::super::sdk_int(env) >= 30
        /* Android 11 (R) */
        {
            let window = env
                .call_method(this, c"getWindow", c"()Landroid/view/Window;", &[])?
                .object();
            if let Some(window) = window {
                env.call_method(
                    window.obj(),
                    c"setDecorFitsSystemWindows",
                    c"(Z)V",
                    &[Arg::Boolean(false)],
                )?;
            }
        }

        /* Control activity re-creation */
        let (finished, created) = {
            let s = statics();
            (s.sdl_main_finished, s.activity_created)
        };
        if finished || created {
            let allow_recreate = super::super::allow_recreate_activity();
            if finished {
                log(LogPriority::Verbose, "SDL main() finished");
            }
            if allow_recreate {
                log(LogPriority::Verbose, "activity re-created");
            } else {
                log(LogPriority::Verbose, "activity finished");
                system_exit(env, 0)?;
                return Ok(());
            }
        }

        statics().activity_created = true;

        let thread_class = env.find_class(c"java/lang/Thread")?;
        let named = env
            .call_static_method(
                thread_class.obj(),
                c"currentThread",
                c"()Ljava/lang/Thread;",
                &[],
            )
            .and_then(|current| {
                let current = current.object().expect("a current thread");
                let name = env.new_string("SDLActivity")?;
                env.call_method(
                    current.obj(),
                    c"setName",
                    c"(Ljava/lang/String;)V",
                    &[Arg::Object(name.obj())],
                )
            });
        if let Err(thrown) = named {
            let e = thrown.catch_exception()?;
            log(
                LogPriority::Verbose,
                &format!(
                    "modify thread properties failed {}",
                    to_string(env, e.obj())
                ),
            );
        }

        // Load shared libraries
        let mut error_msg_broken_lib = String::new();
        match load_libraries(env, this) {
            Ok(()) => statics().broken_libraries = false, /* success */
            Err(thrown) => {
                // catch(UnsatisfiedLinkError e), catch(Exception e)
                let e = thrown
                    .catch_any(&[c"java/lang/UnsatisfiedLinkError", c"java/lang/Exception"])?;
                let (message, text) = message_of(env, e.obj())?;
                system_err_println(env, message.as_ref().map_or(Obj::NULL, Local::obj))?;
                statics().broken_libraries = true;
                error_msg_broken_lib = text;
            }
        }

        if !statics().broken_libraries {
            let expected_version =
                format!("{SDL_MAJOR_VERSION}.{SDL_MINOR_VERSION}.{SDL_MICRO_VERSION}");
            let version = super::super::get_version(env)
                .and_then(|v| env.get_string(v.obj()).ok())
                .unwrap_or_default();
            if version != expected_version {
                statics().broken_libraries = true;
                error_msg_broken_lib = format!(
                    "SDL C/Java version mismatch (expected {expected_version}, got {version})"
                );
            }
        }

        if statics().broken_libraries {
            statics().singleton = Global::new(env, this);
            show_broken_libraries_dialog(env, this, &error_msg_broken_lib)?;
            return Ok(());
        }

        /* Control activity re-creation */
        /* Robustness: check that the native code is run for the first time.
         * (Maybe Activity was reset, but not the native code.) */
        {
            let run_count = super::super::check_sdl_thread_counter(); /* get and increment a native counter */
            if run_count != 0 {
                let allow_recreate = super::super::allow_recreate_activity();
                if allow_recreate {
                    log(
                        LogPriority::Verbose,
                        &format!("activity re-created // run_count: {run_count}"),
                    );
                } else {
                    log(
                        LogPriority::Verbose,
                        &format!("activity finished // run_count: {run_count}"),
                    );
                    system_exit(env, 0)?;
                    return Ok(());
                }
            }
        }

        // Set up JNI
        sdl::setup_jni(env, get_init_subsystems());

        // Initialize state
        sdl::initialize(env)?;

        // So we can call stuff from static callbacks
        statics().singleton = Global::new(env, this);
        sdl::set_context(env, Some(this));

        if sdl::is_controller_manager_ready() {
            super::controller_manager::initialize_device_listener(env)?;
        }

        if sdl::is_subsystem_compiled(sdl::SDL_INIT_VIDEO) {
            super::clipboard_handler::create(env)?;
        }

        // (the HID device manager when nativeIsHIDAPIEnabled(): with the
        // HID driver)

        // Set up the surface
        if sdl::is_subsystem_initialized(sdl::SDL_INIT_VIDEO) {
            let surface = surface::create_sdl_surface(env, this)?;
            statics().surface = Global::new(env, surface.obj());

            let layout_class = env.find_class(c"android/widget/RelativeLayout")?;
            let layout = env.new_object(
                layout_class.obj(),
                c"(Landroid/content/Context;)V",
                &[Arg::Object(this)],
            )?;
            statics().layout = Global::new(env, layout.obj());
            env.call_method(
                layout.obj(),
                c"addView",
                c"(Landroid/view/View;)V",
                &[Arg::Object(surface.obj())],
            )?;

            // Get our current screen orientation and pass it down.
            super::super::set_natural_orientation(get_natural_orientation(env)?);
            let rotation = get_current_rotation(env)?;
            statics().current_rotation = rotation;
            super::super::on_rotation_changed(rotation);
        }

        let locale = sdl::get_context(env)
            .map(|context| current_locale(env, context.obj()))
            .transpose();
        match locale {
            Ok(Some(locale)) => {
                statics().current_locale = locale.and_then(|l| Global::new(env, l.obj()))
            }
            Ok(None) => {}
            Err(thrown) => {
                // catch(Exception ignored)
                drop(thrown.catch_exception()?);
            }
        }

        let context = sdl::get_context(env).expect("the context, just set");
        let configuration = configuration(env, context.obj())?;
        pass_dark_mode(env, configuration.obj())?;

        let layout = {
            let s = statics();
            s.layout.as_ref().and_then(|l| env.new_local_ref(l.obj()))
        };
        if let Some(layout) = layout {
            env.call_method(
                this,
                c"setContentView",
                c"(Landroid/view/View;)V",
                &[Arg::Object(layout.obj())],
            )?;
            set_window_style(env, false)?;
        }

        let window = env
            .call_method(this, c"getWindow", c"()Landroid/view/Window;", &[])?
            .object()
            .expect("the activity's window");
        let decor_view = env
            .call_method(window.obj(), c"getDecorView", c"()Landroid/view/View;", &[])?
            .object()
            .expect("a decor view");
        env.call_method(
            decor_view.obj(),
            c"setOnSystemUiVisibilityChangeListener",
            c"(Landroid/view/View$OnSystemUiVisibilityChangeListener;)V",
            &[Arg::Object(this)],
        )?;

        // Get filename from "Open with" of another application
        // FIXME (upstream): onCreate() runs before SDL_main, so the drop's
        // events are pushed before SDL_Init() has made the event queue
        // active, and lost (unless the activity is made again while SDL
        // runs); and the singleTop activity's next "Open with" goes to
        // onNewIntent(), which SDLActivity doesn't override.
        if sdl::is_subsystem_initialized(sdl::SDL_INIT_VIDEO) {
            let intent = env
                .call_method(this, c"getIntent", c"()Landroid/content/Intent;", &[])?
                .object();
            let data = match &intent {
                Some(intent) => env
                    .call_method(intent.obj(), c"getData", c"()Landroid/net/Uri;", &[])?
                    .object(),
                None => None,
            };
            if let Some(data) = data {
                let filename = env
                    .call_method(data.obj(), c"getPath", c"()Ljava/lang/String;", &[])?
                    .object();
                if let Some(filename) = filename {
                    let filename = env.get_string(filename.obj())?;
                    log(LogPriority::Verbose, &format!("Got filename: {filename}"));
                    super::super::on_drop_file(&filename);
                }
            }
        }
        Ok(())
    });
}

/// `getConfiguration().locale`, or from API 24
/// `getConfiguration().getLocales().get(0)`.
fn current_locale<'a>(env: Env<'a>, context: Obj<'_>) -> JniResult<'a, Option<Local<'a>>> {
    let configuration = configuration(env, context)?;
    if super::super::sdk_int(env) < 24
    /* Android 7.0 (N) */
    {
        Ok(env
            .get_field(configuration.obj(), c"locale", c"Ljava/util/Locale;")?
            .object())
    } else {
        let locales = env
            .call_method(
                configuration.obj(),
                c"getLocales",
                c"()Landroid/os/LocaleList;",
                &[],
            )?
            .object()
            .expect("a locale list");
        Ok(env
            .call_method(
                locales.obj(),
                c"get",
                c"(I)Ljava/util/Locale;",
                &[Arg::Int(0)],
            )?
            .object())
    }
}

/// The "SDL Error" dialog of `onCreate()` when the libraries are broken,
/// whose button finishes the activity.
fn show_broken_libraries_dialog<'a>(
    env: Env<'a>,
    this: Obj<'_>,
    error_msg_broken_lib: &str,
) -> JniResult<'a, ()> {
    let builder_class = env.find_class(c"android/app/AlertDialog$Builder")?;
    let dlg_alert = env.new_object(
        builder_class.obj(),
        c"(Landroid/content/Context;)V",
        &[Arg::Object(this)],
    )?;
    let system = env.find_class(c"java/lang/System")?;
    let key = env.new_string("line.separator")?;
    let separator = env
        .call_static_method(
            system.obj(),
            c"getProperty",
            c"(Ljava/lang/String;)Ljava/lang/String;",
            &[Arg::Object(key.obj())],
        )?
        .object()
        .map(|s| env.get_string(s.obj()))
        .transpose()?
        .unwrap_or_else(|| "null".to_string());
    let message = env.new_string(&format!(
        "An error occurred while trying to start the application. Please try again and/or reinstall.{separator}{separator}Error: {error_msg_broken_lib}"
    ))?;
    env.call_method(
        dlg_alert.obj(),
        c"setMessage",
        c"(Ljava/lang/CharSequence;)Landroid/app/AlertDialog$Builder;",
        &[Arg::Object(message.obj())],
    )?;
    let title = env.new_string("SDL Error")?;
    env.call_method(
        dlg_alert.obj(),
        c"setTitle",
        c"(Ljava/lang/CharSequence;)Landroid/app/AlertDialog$Builder;",
        &[Arg::Object(title.obj())],
    )?;
    let listener = anonymous::listener(
        env,
        c"android/content/DialogInterface$OnClickListener",
        |env, method, _args| {
            if method == "onClick" {
                // if this button is clicked, close current activity
                let singleton = singleton_or_throw(env)?;
                env.call_method(singleton.obj(), c"finish", c"()V", &[])?;
            }
            Ok(None)
        },
    )?;
    let exit = env.new_string("Exit")?;
    env.call_method(
        dlg_alert.obj(),
        c"setPositiveButton",
        c"(Ljava/lang/CharSequence;Landroid/content/DialogInterface$OnClickListener;)Landroid/app/AlertDialog$Builder;",
        &[Arg::Object(exit.obj()), Arg::Object(listener.obj())],
    )?;
    env.call_method(
        dlg_alert.obj(),
        c"setCancelable",
        c"(Z)Landroid/app/AlertDialog$Builder;",
        &[Arg::Boolean(false)],
    )?;
    let dialog = env
        .call_method(
            dlg_alert.obj(),
            c"create",
            c"()Landroid/app/AlertDialog;",
            &[],
        )?
        .object()
        .expect("a dialog");
    env.call_method(dialog.obj(), c"show", c"()V", &[])?;
    Ok(())
}

/// `System.exit(status)`.
fn system_exit<'a>(env: Env<'a>, status: jint) -> JniResult<'a, ()> {
    let system = env.find_class(c"java/lang/System")?;
    env.call_static_method(system.obj(), c"exit", c"(I)V", &[Arg::Int(status)])?;
    Ok(())
}

/// `obj.toString()` (`"null"` for null, or when it throws).
fn to_string(env: Env<'_>, obj: Obj<'_>) -> String {
    if obj.is_null() {
        return "null".to_string();
    }
    match env.call_method(obj, c"toString", c"()Ljava/lang/String;", &[]) {
        Ok(s) => s
            .object()
            .and_then(|s| env.get_string(s.obj()).ok())
            .unwrap_or_default(),
        Err(thrown) => {
            thrown.clear();
            "null".to_string()
        }
    }
}

/// Translation of `pauseNativeThread()`.
fn pause_native_thread(env: Env<'_>) -> JniResult<'_, ()> {
    let broken = {
        let mut s = statics();
        s.next_native_state = NativeState::Paused;
        s.is_resumed_called = false;
        s.broken_libraries
    };

    if broken {
        return Ok(());
    }

    handle_native_state(env)
}

/// Translation of `resumeNativeThread()`.
fn resume_native_thread(env: Env<'_>) -> JniResult<'_, ()> {
    let broken = {
        let mut s = statics();
        s.next_native_state = NativeState::Resumed;
        s.is_resumed_called = true;
        s.broken_libraries
    };

    if broken {
        return Ok(());
    }

    handle_native_state(env)
}

// Events
/// Translation of `onPause()`.
pub(crate) extern "system" fn on_pause(env: *mut JNIEnv, this: jobject) {
    // SAFETY: a native method's environment and arguments.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    native(env, |env| {
        log(LogPriority::Verbose, "onPause()");
        call_super(env, this, c"onPause", c"()V", &[])?;

        // (mHIDDeviceManager.setFrozen(true): the HID device manager
        // comes with HIDAPI's Android backend)

        if !has_multi_window(env) {
            pause_native_thread(env)?;
        }
        Ok(())
    });
}

/// Translation of `onResume()`.
pub(crate) extern "system" fn on_resume(env: *mut JNIEnv, this: jobject) {
    // SAFETY: a native method's environment and arguments.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    native(env, |env| {
        log(LogPriority::Verbose, "onResume()");
        call_super(env, this, c"onResume", c"()V", &[])?;

        // (mHIDDeviceManager.setFrozen(false): see on_pause())

        if !has_multi_window(env) {
            resume_native_thread(env)?;
        }
        Ok(())
    });
}

/// Translation of `onStop()`.
pub(crate) extern "system" fn on_stop(env: *mut JNIEnv, this: jobject) {
    // SAFETY: a native method's environment and arguments.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    native(env, |env| {
        log(LogPriority::Verbose, "onStop()");
        call_super(env, this, c"onStop", c"()V", &[])?;
        if has_multi_window(env) {
            pause_native_thread(env)?;
        }
        Ok(())
    });
}

/// Translation of `onStart()`.
pub(crate) extern "system" fn on_start(env: *mut JNIEnv, this: jobject) {
    // SAFETY: a native method's environment and arguments.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    native(env, |env| {
        log(LogPriority::Verbose, "onStart()");
        call_super(env, this, c"onStart", c"()V", &[])?;
        if has_multi_window(env) {
            resume_native_thread(env)?;
        }
        Ok(())
    });
}

/// Translation of `onWindowFocusChanged()`.
pub(crate) extern "system" fn on_window_focus_changed(
    env: *mut JNIEnv,
    this: jobject,
    has_focus: jboolean,
) {
    // SAFETY: a native method's environment and arguments.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    let has_focus = has_focus != 0;
    native(env, |env| {
        call_super(
            env,
            this,
            c"onWindowFocusChanged",
            c"(Z)V",
            &[Arg::Boolean(has_focus)],
        )?;
        log(
            LogPriority::Verbose,
            &format!("onWindowFocusChanged(): {has_focus}"),
        );

        // If we are gaining focus, we can always try to restore our USB devices. If we are losing focus,
        // only try to relinquish them if we don't have background events allowed (for multi-window Android setups).
        // (hasFocus, or not nativeGetHintBoolean("SDL_JOYSTICK_ALLOW_BACKGROUND_EVENTS", false):
        // mHIDDeviceManager.setFrozen(!hasFocus), see on_pause())

        if statics().broken_libraries {
            return Ok(());
        }

        statics().has_focus = has_focus;
        if has_focus {
            statics().next_native_state = NativeState::Resumed;
            let motion_listener = get_motion_listener(env)?;
            controller_manager::reclaim_relative_mouse_mode_if_needed(env, motion_listener.obj())?;

            handle_native_state(env)?;
            super::super::focus_changed(true);
        } else {
            super::super::focus_changed(false);
            if !has_multi_window(env) {
                statics().next_native_state = NativeState::Paused;
                handle_native_state(env)?;
            }
        }
        Ok(())
    });
}

/// Translation of `onTrimMemory()`.
pub(crate) extern "system" fn on_trim_memory(env: *mut JNIEnv, this: jobject, level: jint) {
    // SAFETY: a native method's environment and arguments.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    native(env, |env| {
        log(LogPriority::Verbose, "onTrimMemory()");
        call_super(env, this, c"onTrimMemory", c"(I)V", &[Arg::Int(level)])?;

        if statics().broken_libraries {
            return Ok(());
        }

        super::super::low_memory();
        Ok(())
    });
}

/// Translation of `onConfigurationChanged()`.
pub(crate) extern "system" fn on_configuration_changed(
    env: *mut JNIEnv,
    this: jobject,
    new_config: jobject,
) {
    // SAFETY: a native method's environment and arguments.
    let (env, this, new_config) = unsafe {
        (
            Env::from_raw(env),
            Obj::from_raw(this),
            Obj::from_raw(new_config),
        )
    };
    native(env, |env| {
        log(LogPriority::Verbose, "onConfigurationChanged()");
        call_super(
            env,
            this,
            c"onConfigurationChanged",
            c"(Landroid/content/res/Configuration;)V",
            &[Arg::Object(new_config)],
        )?;

        if statics().broken_libraries {
            return Ok(());
        }

        let new_locale = env
            .get_field(new_config, c"locale", c"Ljava/util/Locale;")?
            .object();
        let current = {
            let s = statics();
            s.current_locale
                .as_ref()
                .and_then(|l| env.new_local_ref(l.obj()))
        };
        let changed = match &current {
            None => true,
            Some(current) => !env
                .call_method(
                    current.obj(),
                    c"equals",
                    c"(Ljava/lang/Object;)Z",
                    &[Arg::Object(
                        new_locale.as_ref().map_or(Obj::NULL, Local::obj),
                    )],
                )?
                .boolean(),
        };
        if changed {
            statics().current_locale = new_locale.and_then(|l| Global::new(env, l.obj()));
            super::super::on_locale_changed();
        }

        pass_dark_mode(env, new_config)
    });
}

/// Translation of `onDestroy()`.
pub(crate) extern "system" fn on_destroy(env: *mut JNIEnv, this: jobject) {
    // SAFETY: a native method's environment and arguments.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    native(env, |env| {
        log(LogPriority::Verbose, "onDestroy()");

        // (HIDDeviceManager.release(mHIDDeviceManager), see on_pause())

        super::audio_manager::release(env, this);

        if statics().broken_libraries {
            call_super(env, this, c"onDestroy", c"()V", &[])?;
            return Ok(());
        }

        let sdl_thread = {
            let s = statics();
            s.sdl_thread
                .as_ref()
                .and_then(|t| env.new_local_ref(t.obj()))
        };
        if let Some(sdl_thread) = sdl_thread {
            // Send Quit event to "SDLThread" thread
            super::super::send_quit();

            // Wait for "SDLThread" thread to end
            // Use a timeout because:
            // C SDLmain() thread might have started (mSDLThread.start() called)
            // while the SDL_Init() might not have been called yet,
            // and so the previous QUIT event will be discarded by SDL_Init() and app is running, not exiting.
            if let Err(thrown) =
                env.call_method(sdl_thread.obj(), c"join", c"(J)V", &[Arg::Long(1000)])
            {
                let e = thrown.catch_exception()?;
                log(
                    LogPriority::Verbose,
                    &format!("Problem stopping SDLThread: {}", to_string(env, e.obj())),
                );
            }
        }

        super::super::quit();

        call_super(env, this, c"onDestroy", c"()V", &[])?;
        Ok(())
    });
}

/// Translation of `onBackPressed()`.
pub(crate) extern "system" fn on_back_pressed(env: *mut JNIEnv, this: jobject) {
    // SAFETY: a native method's environment and arguments.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    native(env, |env| {
        // Check if we want to block the back button in case of mouse right click.
        //
        // If we do, the normal hardware back button will no longer work and people have to use home,
        // but the mouse right click will work.
        //
        let trap_back = super::super::get_hint_boolean("SDL_ANDROID_TRAP_BACK_BUTTON", false);
        if trap_back {
            // Exit and let the mouse handler handle this button (if appropriate)
            return Ok(());
        }

        // Default system back button behavior.
        if !env
            .call_method(this, c"isFinishing", c"()Z", &[])?
            .boolean()
        {
            call_super(env, this, c"onBackPressed", c"()V", &[])?;
        }
        Ok(())
    });
}

/// `KeyEvent.KEYCODE_BACK`.
const KEYCODE_BACK: jint = 4;

/// The back button's trap from API 33 (where `onBackPressed()` isn't
/// called): a callback of the activity's back dispatcher, which makes the
/// back button a key (down, then up 500 ms later). Translation of
/// `setBackButtonTrapEnabled()`, and of its anonymous
/// `OnBackInvokedCallback`.
pub(crate) fn set_back_button_trap_enabled<'a>(env: Env<'a>, enabled: bool) -> JniResult<'a, ()> {
    if super::super::sdk_int(env) < 33 {
        // We're old enough that we just use the old onBackPressed behavior.
        return Ok(());
    }

    // Check so we don't register the same callback twice.
    if enabled == statics().back_key_trap_enabled {
        return Ok(());
    }

    if statics().back_button_callback.is_none() {
        // (mBackKeyHandler)
        let back_key_handler: Mutex<Option<Global>> = Mutex::new(None);
        let callback = anonymous::listener(
            env,
            c"android/window/OnBackInvokedCallback",
            move |env, method, _args| {
                if method == "onBackInvoked" {
                    let handler = {
                        let mut handler =
                            back_key_handler.lock().unwrap_or_else(|e| e.into_inner());
                        if handler.is_none() {
                            let looper_class = env.find_class(c"android/os/Looper")?;
                            let looper = env
                                .call_static_method(
                                    looper_class.obj(),
                                    c"getMainLooper",
                                    c"()Landroid/os/Looper;",
                                    &[],
                                )?
                                .object()
                                .expect("the main looper");
                            let handler_class = env.find_class(c"android/os/Handler")?;
                            let new_handler = env.new_object(
                                handler_class.obj(),
                                c"(Landroid/os/Looper;)V",
                                &[Arg::Object(looper.obj())],
                            )?;
                            *handler = Global::new(env, new_handler.obj());
                        }
                        handler
                            .as_ref()
                            .and_then(|h| env.new_local_ref(h.obj()))
                            .expect("mBackKeyHandler")
                    };

                    super::super::on_key_down(KEYCODE_BACK);
                    let key_up = anonymous::runnable(env, |_env| {
                        super::super::on_key_up(KEYCODE_BACK);
                    })?;
                    env.call_method(
                        handler.obj(),
                        c"postDelayed",
                        c"(Ljava/lang/Runnable;J)Z",
                        &[Arg::Object(key_up.obj()), Arg::Long(500)],
                    )?;
                }
                Ok(None)
            },
        )?;
        statics().back_button_callback = Global::new(env, callback.obj());
    }

    let callback = {
        let s = statics();
        s.back_button_callback
            .as_ref()
            .and_then(|c| env.new_local_ref(c.obj()))
            .expect("backButtonCallback")
    };
    let singleton = singleton_or_throw(env)?;
    let dispatcher = env
        .call_method(
            singleton.obj(),
            c"getOnBackInvokedDispatcher",
            c"()Landroid/window/OnBackInvokedDispatcher;",
            &[],
        )?
        .object()
        .expect("the back dispatcher");
    if enabled {
        const PRIORITY_DEFAULT: jint = 0; // OnBackInvokedDispatcher.PRIORITY_DEFAULT
        env.call_method(
            dispatcher.obj(),
            c"registerOnBackInvokedCallback",
            c"(ILandroid/window/OnBackInvokedCallback;)V",
            &[Arg::Int(PRIORITY_DEFAULT), Arg::Object(callback.obj())],
        )?;
    } else {
        env.call_method(
            dispatcher.obj(),
            c"unregisterOnBackInvokedCallback",
            c"(Landroid/window/OnBackInvokedCallback;)V",
            &[Arg::Object(callback.obj())],
        )?;
    }
    statics().back_key_trap_enabled = enabled;
    Ok(())
}

/// Called by JNI from SDL. Translation of `manualBackButton()`.
pub(crate) fn manual_back_button<'a>(env: Env<'a>) -> JniResult<'a, ()> {
    let singleton = singleton_or_throw(env)?;
    press_back_button(env, singleton.obj())
}

/// Used to get us onto the activity's main thread. Translation of
/// `pressBackButton()`, and of its anonymous `Runnable`.
fn press_back_button<'a>(env: Env<'a>, this: Obj<'_>) -> JniResult<'a, ()> {
    let activity = Global::new(env, this).expect("a global reference");
    let runnable = anonymous::runnable(env, move |env| {
        let pressed = env
            .call_method(activity.obj(), c"isFinishing", c"()Z", &[])
            .and_then(|finishing| {
                if !finishing.boolean() {
                    super_on_back_pressed(env, activity.obj())?;
                }
                Ok(())
            });
        if let Err(thrown) = pressed {
            // (pending, for the looper)
            drop(thrown);
        }
    })?;
    env.call_method(
        this,
        c"runOnUiThread",
        c"(Ljava/lang/Runnable;)V",
        &[Arg::Object(runnable.obj())],
    )?;
    Ok(())
}

/// Used to access the system back behavior. Translation of
/// `superOnBackPressed()`.
fn super_on_back_pressed<'a>(env: Env<'a>, this: Obj<'_>) -> JniResult<'a, ()> {
    call_super(env, this, c"onBackPressed", c"()V", &[])
}

/// `KeyEvent.KEYCODE_VOLUME_UP`, `_DOWN`, `KEYCODE_CAMERA`,
/// `KEYCODE_ZOOM_IN` and `_OUT` (API 11).
const KEYCODE_VOLUME_UP: jint = 24;
const KEYCODE_VOLUME_DOWN: jint = 25;
const KEYCODE_CAMERA: jint = 27;
const KEYCODE_ZOOM_IN: jint = 168;
const KEYCODE_ZOOM_OUT: jint = 169;

/// Translation of `dispatchKeyEvent()`.
pub(crate) extern "system" fn dispatch_key_event(
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
        if statics().broken_libraries {
            return Ok(false);
        }

        let key_code = env.call_method(event, c"getKeyCode", c"()I", &[])?.int();
        // Ignore certain special keys so they're handled by Android
        if key_code == KEYCODE_VOLUME_DOWN
            || key_code == KEYCODE_VOLUME_UP
            || key_code == KEYCODE_CAMERA
            || key_code == KEYCODE_ZOOM_IN /* API 11 */
            || key_code == KEYCODE_ZOOM_OUT
        /* API 11 */
        {
            return Ok(false);
        }
        statics().dispatching_key_event = true;
        let superclass = activity_superclass(env)?;
        let result = env.call_nonvirtual_method(
            this,
            superclass.obj(),
            c"dispatchKeyEvent",
            c"(Landroid/view/KeyEvent;)Z",
            &[Arg::Object(event)],
        );
        statics().dispatching_key_event = false;
        Ok(result?.boolean())
    })
}

/// Translation of `dispatchingKeyEvent()`.
pub(crate) fn dispatching_key_event() -> bool {
    statics().dispatching_key_event
}

/// Transition to next state. Translation of `handleNativeState()`.
fn handle_native_state(env: Env<'_>) -> JniResult<'_, ()> {
    let mut s = statics();
    if s.next_native_state == s.current_native_state {
        // Already in same state, discard.
        return Ok(());
    }

    // Try a transition to init state
    if s.next_native_state == NativeState::Init {
        s.current_native_state = s.next_native_state;
        return Ok(());
    }

    let surface = s
        .surface
        .as_ref()
        .and_then(|surface| env.new_local_ref(surface.obj()));

    // Try a transition to paused state
    if s.next_native_state == NativeState::Paused {
        if s.sdl_thread.is_some() {
            super::super::pause();
        }
        if let Some(surface) = &surface {
            drop(s);
            surface::handle_pause(env, surface.obj())?;
            s = statics();
        }
        s.current_native_state = s.next_native_state;
        return Ok(());
    }

    // Try a transition to resumed state
    if s.next_native_state == NativeState::Resumed {
        let ready_to_run = match &surface {
            None => s.is_resumed_called,
            Some(surface) => {
                surface::is_surface_ready(env, surface.obj())
                    && (s.has_focus || has_multi_window(env))
                    && s.is_resumed_called
            }
        };
        if ready_to_run {
            if s.sdl_thread.is_none() {
                // This is the entry point to the C app.
                // Start up the C app thread and enable sensor input for the first time
                // FIXME: Why aren't we enabling sensor input at start?

                let thread = new_sdl_thread(env)?;
                s.sdl_thread = Global::new(env, thread.obj());
                drop(s);
                if let Some(surface) = &surface {
                    const TYPE_ACCELEROMETER: jint = 1; // Sensor.TYPE_ACCELEROMETER
                    surface::enable_sensor(env, surface.obj(), TYPE_ACCELEROMETER, true)?;
                }
                env.call_method(thread.obj(), c"start", c"()V", &[])?;
                s = statics();

                // No nativeResume(), don't signal Android_ResumeSem
            } else {
                super::super::resume();
            }
            if let Some(surface) = &surface {
                drop(s);
                surface::handle_resume(env, surface.obj())?;
                s = statics();
            }

            s.current_native_state = s.next_native_state;
        }
    }
    Ok(())
}

/// `new Thread(new SDLMain(), "SDLThread")`.
fn new_sdl_thread(env: Env<'_>) -> JniResult<'_, Local<'_>> {
    let sdl_main_class =
        env::app_class(super::super::classes::SDL_MAIN).expect("JNI_OnLoad() found SDLMain");
    let runnable = env.new_object(sdl_main_class.obj(), c"()V", &[])?;
    let name = env.new_string("SDLThread")?;
    let thread_class = env.find_class(c"java/lang/Thread")?;
    env.new_object(
        thread_class.obj(),
        c"(Ljava/lang/Runnable;Ljava/lang/String;)V",
        &[Arg::Object(runnable.obj()), Arg::Object(name.obj())],
    )
}

// Messages from the SDLMain thread
const COMMAND_CHANGE_TITLE: jint = 1;
const COMMAND_CHANGE_WINDOW_STYLE: jint = 2;
const COMMAND_TEXTEDIT_HIDE: jint = 3;
const COMMAND_SET_KEEP_SCREEN_ON: jint = 5;
#[allow(dead_code)] // (the commands from here up are the app's)
const COMMAND_USER: jint = 0x8000;

/// This method is called by SDL if SDL did not handle a message itself.
/// This happens if a received message contains an unsupported command.
/// Translation of `onUnhandledMessage()` (which an app's subclass would
/// override: see the module's description).
fn on_unhandled_message(_command: jint, _param: Obj<'_>) -> bool {
    false
}

/// `WindowManager.LayoutParams.FLAG_FULLSCREEN`.
const FLAG_FULLSCREEN: jint = 0x400;
/// `WindowManager.LayoutParams.FLAG_FORCE_NOT_FULLSCREEN`.
const FLAG_FORCE_NOT_FULLSCREEN: jint = 0x800;
/// `WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON`.
const FLAG_KEEP_SCREEN_ON: jint = 0x80;
/// `WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS`.
const LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS: jint = 3;
/// `WindowInsetsController.BEHAVIOR_DEFAULT`.
const BEHAVIOR_DEFAULT: jint = 1;
/// `WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE`.
const BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE: jint = 2;
// View.SYSTEM_UI_FLAG_* and View.INVISIBLE
const SYSTEM_UI_FLAG_VISIBLE: jint = 0;
const SYSTEM_UI_FLAG_HIDE_NAVIGATION: jint = 0x2;
const SYSTEM_UI_FLAG_FULLSCREEN: jint = 0x4;
const SYSTEM_UI_FLAG_LAYOUT_STABLE: jint = 0x100;
const SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION: jint = 0x200;
const SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN: jint = 0x400;
const SYSTEM_UI_FLAG_IMMERSIVE_STICKY: jint = 0x1000;
const INVISIBLE: jint = 0x4;

/// A Handler class for Messages from native SDL applications. It uses
/// current Activities as target (e.g. for the title). Translation of
/// `SDLCommandHandler.handleMessage()`.
pub(crate) extern "system" fn command_handler_handle_message(
    env: *mut JNIEnv,
    _this: jobject,
    msg: jobject,
) {
    // SAFETY: a native method's environment and arguments.
    let (env, msg) = unsafe { (Env::from_raw(env), Obj::from_raw(msg)) };
    native(env, |env| {
        let Some(context) = sdl::get_context(env) else {
            log(
                LogPriority::Error,
                "error handling message, getContext() returned null",
            );
            return Ok(());
        };
        let context = context.obj();
        let arg1 = env.get_field(msg, c"arg1", c"I")?.int();
        let obj = env.get_field(msg, c"obj", c"Ljava/lang/Object;")?.object();
        let obj = obj.as_ref().map_or(Obj::NULL, Local::obj);
        match arg1 {
            COMMAND_CHANGE_TITLE => {
                if is_activity(env, context) {
                    env.call_method(
                        context,
                        c"setTitle",
                        c"(Ljava/lang/CharSequence;)V",
                        &[Arg::Object(obj)],
                    )?;
                } else {
                    log(
                        LogPriority::Error,
                        "error handling message, getContext() returned no Activity",
                    );
                }
            }
            COMMAND_CHANGE_WINDOW_STYLE => {
                if is_activity(env, context) {
                    let window = env
                        .call_method(context, c"getWindow", c"()Landroid/view/Window;", &[])?
                        .object();
                    if let Some(window) = window {
                        change_window_style(env, window.obj(), integer_value(env, obj)?)?;
                    }
                } else {
                    log(
                        LogPriority::Error,
                        "error handling message, getContext() returned no Activity",
                    );
                }
            }
            COMMAND_TEXTEDIT_HIDE => {
                if let Some(text_edit) = text_edit(env) {
                    // Note: On some devices setting view to GONE creates a flicker in landscape.
                    // Setting the View's sizes to 0 is similar to GONE but without the flicker.
                    // The sizes will be set to useful values when the keyboard is shown again.
                    let params = new_relative_layout_params(env, 0, 0)?;
                    env.call_method(
                        text_edit.obj(),
                        c"setLayoutParams",
                        c"(Landroid/view/ViewGroup$LayoutParams;)V",
                        &[Arg::Object(params.obj())],
                    )?;

                    let imm = input_method_manager(env, context)?;
                    let token = env
                        .call_method(
                            text_edit.obj(),
                            c"getWindowToken",
                            c"()Landroid/os/IBinder;",
                            &[],
                        )?
                        .object();
                    env.call_method(
                        imm.obj(),
                        c"hideSoftInputFromWindow",
                        c"(Landroid/os/IBinder;I)Z",
                        &[
                            Arg::Object(token.as_ref().map_or(Obj::NULL, Local::obj)),
                            Arg::Int(0),
                        ],
                    )?;

                    super::super::on_screen_keyboard_hidden();

                    let surface = statics()
                        .surface
                        .as_ref()
                        .and_then(|s| env.new_local_ref(s.obj()));
                    let Some(surface) = surface else {
                        return null_pointer(env, c"SDLActivity.mSurface is null");
                    };
                    env.call_method(surface.obj(), c"requestFocus", c"()Z", &[])?;
                }
            }
            COMMAND_SET_KEEP_SCREEN_ON => {
                if is_activity(env, context) {
                    let window = env
                        .call_method(context, c"getWindow", c"()Landroid/view/Window;", &[])?
                        .object();
                    if let Some(window) = window {
                        let keep_on = integer_value(env, obj)?.is_some_and(|v| v != 0);
                        let method = if keep_on { c"addFlags" } else { c"clearFlags" };
                        env.call_method(
                            window.obj(),
                            method,
                            c"(I)V",
                            &[Arg::Int(FLAG_KEEP_SCREEN_ON)],
                        )?;
                    }
                }
            }
            _ => {
                let activity_class = env::app_class(super::super::classes::SDL_ACTIVITY)
                    .expect("JNI_OnLoad() found SDLActivity");
                if env.is_instance_of(context, activity_class.obj())
                    && !on_unhandled_message(arg1, obj)
                {
                    log(
                        LogPriority::Error,
                        &format!("error handling message, command is {arg1}"),
                    );
                }
            }
        }
        Ok(())
    });
}

/// `COMMAND_CHANGE_WINDOW_STYLE` on the activity's window: fullscreen
/// (`msg.obj`, an `Integer` that isn't 0), or not.
fn change_window_style<'a>(
    env: Env<'a>,
    window: Obj<'_>,
    fullscreen: Option<jint>,
) -> JniResult<'a, ()> {
    let modern = super::super::sdk_int(env) >= 30; /* Android 11 (R) */
    if fullscreen.is_some_and(|v| v != 0) {
        if modern {
            // The legacy setSystemUiVisibility() flags are ignored on
            // Android 15+ (API 35+), where edge-to-edge is enforced for
            // apps targeting that SDK, so the status/navigation bars
            // would never hide. Use WindowInsetsController instead.
            let controller = env
                .call_method(
                    window,
                    c"getInsetsController",
                    c"()Landroid/view/WindowInsetsController;",
                    &[],
                )?
                .object();
            if let Some(controller) = controller {
                let types = env.find_class(c"android/view/WindowInsets$Type")?;
                let system_bars = env
                    .call_static_method(types.obj(), c"systemBars", c"()I", &[])?
                    .int();
                env.call_method(controller.obj(), c"hide", c"(I)V", &[Arg::Int(system_bars)])?;
                // Sticky-immersive (replaces SYSTEM_UI_FLAG_IMMERSIVE_STICKY):
                // a swipe shows the bars transiently, then they auto-hide.
                env.call_method(
                    controller.obj(),
                    c"setSystemBarsBehavior",
                    c"(I)V",
                    &[Arg::Int(BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE)],
                )?;
            }
        } else {
            let flags = SYSTEM_UI_FLAG_FULLSCREEN
                | SYSTEM_UI_FLAG_HIDE_NAVIGATION
                | SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                | SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                | SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                | SYSTEM_UI_FLAG_LAYOUT_STABLE
                | INVISIBLE;
            set_system_ui_visibility(env, window, flags)?;
            env.call_method(window, c"addFlags", c"(I)V", &[Arg::Int(FLAG_FULLSCREEN)])?;
            env.call_method(
                window,
                c"clearFlags",
                c"(I)V",
                &[Arg::Int(FLAG_FORCE_NOT_FULLSCREEN)],
            )?;
        }
        statics().fullscreen_mode_active = true;
    } else {
        if modern {
            // The legacy setSystemUiVisibility() flags are ignored on
            // API 30+, so the bars hidden by the modern enter path above
            // would never come back. Restore them via WindowInsetsController.
            let controller = env
                .call_method(
                    window,
                    c"getInsetsController",
                    c"()Landroid/view/WindowInsetsController;",
                    &[],
                )?
                .object();
            if let Some(controller) = controller {
                env.call_method(
                    controller.obj(),
                    c"setSystemBarsBehavior",
                    c"(I)V",
                    &[Arg::Int(BEHAVIOR_DEFAULT)],
                )?;
                let types = env.find_class(c"android/view/WindowInsets$Type")?;
                let system_bars = env
                    .call_static_method(types.obj(), c"systemBars", c"()I", &[])?
                    .int();
                env.call_method(controller.obj(), c"show", c"(I)V", &[Arg::Int(system_bars)])?;
            }
        } else {
            let flags = SYSTEM_UI_FLAG_LAYOUT_STABLE | SYSTEM_UI_FLAG_VISIBLE;
            set_system_ui_visibility(env, window, flags)?;
            env.call_method(
                window,
                c"addFlags",
                c"(I)V",
                &[Arg::Int(FLAG_FORCE_NOT_FULLSCREEN)],
            )?;
            env.call_method(window, c"clearFlags", c"(I)V", &[Arg::Int(FLAG_FULLSCREEN)])?;
        }
        statics().fullscreen_mode_active = false;
    }
    if modern {
        let attributes = env
            .call_method(
                window,
                c"getAttributes",
                c"()Landroid/view/WindowManager$LayoutParams;",
                &[],
            )?
            .object()
            .expect("the window's attributes");
        env.set_field(
            attributes.obj(),
            c"layoutInDisplayCutoutMode",
            c"I",
            Arg::Int(LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS),
        )?;
    }
    Ok(())
}

/// `window.getDecorView().setSystemUiVisibility(flags)`.
fn set_system_ui_visibility<'a>(env: Env<'a>, window: Obj<'_>, flags: jint) -> JniResult<'a, ()> {
    let decor_view = env
        .call_method(window, c"getDecorView", c"()Landroid/view/View;", &[])?
        .object()
        .expect("a decor view");
    env.call_method(
        decor_view.obj(),
        c"setSystemUiVisibility",
        c"(I)V",
        &[Arg::Int(flags)],
    )?;
    Ok(())
}

/// Send a message from the SDLMain thread: to the activity's
/// `commandHandler`, on its looper (the main thread's), with `arg1` the
/// command and `obj` the data. For `COMMAND_CHANGE_WINDOW_STYLE`, wait
/// (up to 500 ms) for the surface to change its size when it will.
/// Translation of `sendCommand()`.
fn send_command<'a>(
    env: Env<'a>,
    this: Obj<'_>,
    command: jint,
    data: Obj<'_>,
) -> JniResult<'a, bool> {
    let Some(command_handler) = env
        .get_field(this, c"commandHandler", c"Landroid/os/Handler;")?
        .object()
    else {
        return null_pointer(env, c"SDLActivity.commandHandler is null");
    };
    let msg = env
        .call_method(
            command_handler.obj(),
            c"obtainMessage",
            c"()Landroid/os/Message;",
            &[],
        )?
        .object()
        .expect("a message");
    env.set_field(msg.obj(), c"arg1", c"I", Arg::Int(command))?;
    env.set_field(msg.obj(), c"obj", c"Ljava/lang/Object;", Arg::Object(data))?;
    let result = env
        .call_method(
            command_handler.obj(),
            c"sendMessage",
            c"(Landroid/os/Message;)Z",
            &[Arg::Object(msg.obj())],
        )?
        .boolean();

    if command == COMMAND_CHANGE_WINDOW_STYLE {
        // Ensure we don't return until the resize has actually happened,
        // or 500ms have passed.

        let mut should_wait = false;

        if let Some(data) = integer_value(env, data)? {
            // Let's figure out if we're already laid out fullscreen or not.
            let window = env.new_string("window")?; // Context.WINDOW_SERVICE
            let window_manager = env
                .call_method(
                    this,
                    c"getSystemService",
                    c"(Ljava/lang/String;)Ljava/lang/Object;",
                    &[Arg::Object(window.obj())],
                )?
                .object()
                .expect("the window manager");
            let display = env
                .call_method(
                    window_manager.obj(),
                    c"getDefaultDisplay",
                    c"()Landroid/view/Display;",
                    &[],
                )?
                .object()
                .expect("a display");
            let metrics_class = env.find_class(c"android/util/DisplayMetrics")?;
            let real_metrics = env.new_object(metrics_class.obj(), c"()V", &[])?;
            env.call_method(
                display.obj(),
                c"getRealMetrics",
                c"(Landroid/util/DisplayMetrics;)V",
                &[Arg::Object(real_metrics.obj())],
            )?;

            let surface = {
                let s = statics();
                s.surface
                    .as_ref()
                    .and_then(|surface| env.new_local_ref(surface.obj()))
            };
            let full_screen_layout = match surface {
                None => false,
                Some(surface) => {
                    let width = env
                        .get_field(real_metrics.obj(), c"widthPixels", c"I")?
                        .int();
                    let height = env
                        .get_field(real_metrics.obj(), c"heightPixels", c"I")?
                        .int();
                    width
                        == env
                            .call_method(surface.obj(), c"getWidth", c"()I", &[])?
                            .int()
                        && height
                            == env
                                .call_method(surface.obj(), c"getHeight", c"()I", &[])?
                                .int()
                }
            };

            if data == 1 {
                // If we aren't laid out fullscreen or actively in fullscreen mode already, we're going
                // to change size and should wait for surfaceChanged() before we return, so the size
                // is right back in native code.  If we're already laid out fullscreen, though, we're
                // not going to change size even if we change decor modes, so we shouldn't wait for
                // surfaceChanged() -- which may not even happen -- and should return immediately.
                should_wait = !full_screen_layout;
            } else {
                // If we're laid out fullscreen (even if the status bar and nav bar are present),
                // or are actively in fullscreen, we're going to change size and should wait for
                // surfaceChanged before we return, so the size is right back in native code.
                should_wait = full_screen_layout;
            }
        }

        if should_wait {
            if let Some(context) = sdl::get_context(env) {
                // We'll wait for the surfaceChanged() method, which will notify us
                // when called.  That way, we know our current size is really the
                // size we need, instead of grabbing a size that's still got
                // the navigation and/or status bars before they're hidden.
                //
                // We'll wait for up to half a second, because some devices
                // take a surprisingly long time for the surface resize, but
                // then we'll just give up and return.
                //
                let waited =
                    env.synchronized(context.obj(), || {
                        match env.call_method(context.obj(), c"wait", c"(J)V", &[Arg::Long(500)]) {
                            Ok(_) => Ok(()),
                            Err(thrown) => thrown
                                .catch_any(&[c"java/lang/InterruptedException"])
                                .map(|ie| {
                                    if let Err(thrown) =
                                        env.call_method(ie.obj(), c"printStackTrace", c"()V", &[])
                                    {
                                        thrown.clear();
                                    }
                                }),
                        }
                    })?;
                waited?;
            }
        }
    }

    Ok(result)
}

/// This method is called by SDL using JNI: the title (from the SDLMain
/// thread, on the main one). Translation of `setActivityTitle()`.
pub(crate) fn set_activity_title<'a>(env: Env<'a>, title: &str) -> JniResult<'a, bool> {
    // Called from SDLMain() thread and can't directly affect the view
    let singleton = singleton_or_throw(env)?;
    let title = env.new_string(title)?;
    send_command(env, singleton.obj(), COMMAND_CHANGE_TITLE, title.obj())
}

/// This method is called by SDL using JNI: fullscreen or not (from the
/// SDLMain thread, on the main one). Translation of `setWindowStyle()`.
pub(crate) fn set_window_style<'a>(env: Env<'a>, fullscreen: bool) -> JniResult<'a, ()> {
    // Called from SDLMain() thread and can't directly affect the view
    let singleton = singleton_or_throw(env)?;
    let style = new_integer(env, if fullscreen { 1 } else { 0 })?;
    send_command(
        env,
        singleton.obj(),
        COMMAND_CHANGE_WINDOW_STYLE,
        style.obj(),
    )?;
    Ok(())
}

/// `ActivityInfo.SCREEN_ORIENTATION_*`.
const SCREEN_ORIENTATION_LANDSCAPE: jint = 0;
const SCREEN_ORIENTATION_PORTRAIT: jint = 1;
const SCREEN_ORIENTATION_SENSOR_LANDSCAPE: jint = 6;
const SCREEN_ORIENTATION_SENSOR_PORTRAIT: jint = 7;
const SCREEN_ORIENTATION_REVERSE_LANDSCAPE: jint = 8;
const SCREEN_ORIENTATION_REVERSE_PORTRAIT: jint = 9;
const SCREEN_ORIENTATION_USER_LANDSCAPE: jint = 11;
const SCREEN_ORIENTATION_USER_PORTRAIT: jint = 12;
const SCREEN_ORIENTATION_FULL_USER: jint = 13;

/// This method is called by SDL using JNI. This is a static method for JNI
/// convenience, it calls a non-static method (so that it can be
/// overridden, upstream). Translation of `setOrientation()`.
pub(crate) fn set_orientation<'a>(
    env: Env<'a>,
    w: jint,
    h: jint,
    resizable: bool,
    hint: &str,
) -> JniResult<'a, ()> {
    if singleton(env).is_some() {
        set_orientation_bis(env, w, h, resizable, hint)?;
    }
    Ok(())
}

/// The requested orientation, from the window's size, whether it's
/// resizable, and the orientations `hint` (`SDL_HINT_ORIENTATIONS`)
/// allows. Translation of `setOrientationBis()` (which an app's subclass
/// could override upstream: see the module's description).
fn set_orientation_bis<'a>(
    env: Env<'a>,
    w: jint,
    h: jint,
    resizable: bool,
    hint: &str,
) -> JniResult<'a, ()> {
    let mut orientation_landscape = -1;
    let mut orientation_portrait = -1;

    if w <= 1 || h <= 1 {
        // Invalid width/height, ignore this request
        return Ok(());
    }

    /* If set, hint "explicitly controls which UI orientations are allowed". */
    if hint.contains("LandscapeRight") && hint.contains("LandscapeLeft") {
        orientation_landscape = SCREEN_ORIENTATION_USER_LANDSCAPE;
    } else if hint.contains("LandscapeLeft") {
        orientation_landscape = SCREEN_ORIENTATION_LANDSCAPE;
    } else if hint.contains("LandscapeRight") {
        orientation_landscape = SCREEN_ORIENTATION_REVERSE_LANDSCAPE;
    }

    /* exact match to 'Portrait' to distinguish with PortraitUpsideDown */
    let contains_portrait = hint.contains("Portrait ") || hint.ends_with("Portrait");

    if contains_portrait && hint.contains("PortraitUpsideDown") {
        orientation_portrait = SCREEN_ORIENTATION_USER_PORTRAIT;
    } else if contains_portrait {
        orientation_portrait = SCREEN_ORIENTATION_PORTRAIT;
    } else if hint.contains("PortraitUpsideDown") {
        orientation_portrait = SCREEN_ORIENTATION_REVERSE_PORTRAIT;
    }

    let is_landscape_allowed = orientation_landscape != -1;
    let is_portrait_allowed = orientation_portrait != -1;
    let req; /* Requested orientation */

    /* No valid hint, nothing is explicitly allowed */
    if !is_portrait_allowed && !is_landscape_allowed {
        if resizable {
            /* All orientations are allowed, respecting user orientation lock setting */
            req = SCREEN_ORIENTATION_FULL_USER;
        } else {
            /* Fixed window and nothing specified. Get orientation from w/h of created window */
            req = if w > h {
                SCREEN_ORIENTATION_SENSOR_LANDSCAPE
            } else {
                SCREEN_ORIENTATION_SENSOR_PORTRAIT
            };
        }
    } else {
        /* At least one orientation is allowed */
        if resizable {
            if is_portrait_allowed && is_landscape_allowed {
                /* hint allows both landscape and portrait, promote to full user */
                req = SCREEN_ORIENTATION_FULL_USER;
            } else {
                /* Use the only one allowed "orientation" */
                req = if is_landscape_allowed {
                    orientation_landscape
                } else {
                    orientation_portrait
                };
            }
        } else {
            /* Fixed window and both orientations are allowed. Choose one. */
            if is_portrait_allowed && is_landscape_allowed {
                req = if w > h {
                    orientation_landscape
                } else {
                    orientation_portrait
                };
            } else {
                /* Use the only one allowed "orientation" */
                req = if is_landscape_allowed {
                    orientation_landscape
                } else {
                    orientation_portrait
                };
            }
        }
    }

    log(
        LogPriority::Verbose,
        &format!(
            "setOrientation() requestedOrientation={req} width={w} height={h} resizable={resizable} hint={hint}"
        ),
    );
    let singleton = singleton_or_throw(env)?;
    env.call_method(
        singleton.obj(),
        c"setRequestedOrientation",
        c"(I)V",
        &[Arg::Int(req)],
    )?;
    Ok(())
}

/// This method is called by SDL using JNI: the home screen started.
/// Translation of `minimizeWindow()`.
pub(crate) fn minimize_window<'a>(env: Env<'a>) -> JniResult<'a, ()> {
    let Some(singleton) = singleton(env) else {
        return Ok(());
    };

    const FLAG_ACTIVITY_NEW_TASK: jint = 0x10000000; // Intent.FLAG_ACTIVITY_NEW_TASK
    let intent_class = env.find_class(c"android/content/Intent")?;
    let action = env.new_string("android.intent.action.MAIN")?; // Intent.ACTION_MAIN
    let start_main = env.new_object(
        intent_class.obj(),
        c"(Ljava/lang/String;)V",
        &[Arg::Object(action.obj())],
    )?;
    let category = env.new_string("android.intent.category.HOME")?; // Intent.CATEGORY_HOME
    env.call_method(
        start_main.obj(),
        c"addCategory",
        c"(Ljava/lang/String;)Landroid/content/Intent;",
        &[Arg::Object(category.obj())],
    )?;
    env.call_method(
        start_main.obj(),
        c"setFlags",
        c"(I)Landroid/content/Intent;",
        &[Arg::Int(FLAG_ACTIVITY_NEW_TASK)],
    )?;
    env.call_method(
        singleton.obj(),
        c"startActivity",
        c"(Landroid/content/Intent;)V",
        &[Arg::Object(start_main.obj())],
    )?;
    Ok(())
}

/// This method is called by SDL using JNI. Translation of
/// `shouldMinimizeOnFocusLoss()`.
pub(crate) fn should_minimize_on_focus_loss(_env: Env<'_>) -> bool {
    false
}

/// This method is called by SDL using JNI. DeX mode in Samsung
/// Experience 9.0 and earlier doesn't support relative mice properly
/// under Android 7 APIs, and simply returns no data under Android 8 APIs.
/// Translation of `supportsRelativeMouse()`.
pub(crate) fn supports_relative_mouse<'a>(env: Env<'a>) -> JniResult<'a, bool> {
    // DeX mode in Samsung Experience 9.0 and earlier doesn't support relative mice properly under
    // Android 7 APIs, and simply returns no data under Android 8 APIs.
    //
    // This is fixed in Samsung Experience 9.5, which corresponds to Android 8.1.0, and
    // thus SDK version 27.  If we are in DeX mode and not API 27 or higher, as a result,
    // we should stick to relative mode.
    //
    if super::super::sdk_int(env) < 27 /* Android 8.1 (O_MR1) */ && is_dex_mode(env)? {
        return Ok(false);
    }

    let motion_listener = get_motion_listener(env)?;
    controller_manager::supports_relative_mouse(env, motion_listener.obj())
}

/// This method is called by SDL using JNI. Translation of
/// `setRelativeMouseEnabled()`.
pub(crate) fn set_relative_mouse_enabled<'a>(env: Env<'a>, enabled: bool) -> JniResult<'a, bool> {
    if enabled && !supports_relative_mouse(env)? {
        return Ok(false);
    }

    let motion_listener = get_motion_listener(env)?;
    controller_manager::set_relative_mouse_enabled(env, motion_listener.obj(), enabled)
}

/// This method is called by SDLControllerManager's API 26 Generic Motion
/// Handler. Translation of `getContentView()`.
pub(crate) fn get_content_view<'a>(env: Env<'a>) -> Option<Local<'a>> {
    let s = statics();
    s.layout.as_ref().and_then(|l| env.new_local_ref(l.obj()))
}

/// `mTextEdit`, as a local reference (`None`: null).
pub(crate) fn text_edit<'a>(env: Env<'a>) -> Option<Local<'a>> {
    let s = statics();
    s.text_edit
        .as_ref()
        .and_then(|t| env.new_local_ref(t.obj()))
}

/// `new RelativeLayout.LayoutParams(w, h)`.
fn new_relative_layout_params<'a>(env: Env<'a>, w: jint, h: jint) -> JniResult<'a, Local<'a>> {
    let class = env.find_class(c"android/widget/RelativeLayout$LayoutParams")?;
    env.new_object(class.obj(), c"(II)V", &[Arg::Int(w), Arg::Int(h)])
}

/// `(InputMethodManager) context.getSystemService(Context.INPUT_METHOD_SERVICE)`.
fn input_method_manager<'a>(env: Env<'a>, context: Obj<'_>) -> JniResult<'a, Local<'a>> {
    let name = env.new_string("input_method")?; // Context.INPUT_METHOD_SERVICE
    let imm = env
        .call_method(
            context,
            c"getSystemService",
            c"(Ljava/lang/String;)Ljava/lang/Object;",
            &[Arg::Object(name.obj())],
        )?
        .object();
    match imm {
        Some(imm) => Ok(imm),
        None => null_pointer(env, c"no InputMethodManager"),
    }
}

/// This is used to regulate the pan&scan method to have some offset from
/// the bottom edge of the input region and the top edge of an input method
/// (soft keyboard). `ShowTextInputTask.HEIGHT_PADDING`.
const HEIGHT_PADDING: jint = 15;

/// `new ShowTextInputTask(input_type, x, y, w, h)`: the DEX file's
/// `SDLActivity$ShowTextInputTask` (whose constructor is `Object`'s), its
/// fields set as upstream's constructor sets them.
fn new_show_text_input_task<'a>(
    env: Env<'a>,
    input_type: jint,
    x: jint,
    y: jint,
    mut w: jint,
    mut h: jint,
) -> JniResult<'a, Local<'a>> {
    let class = env::app_class(super::super::classes::SDL_SHOW_TEXT_INPUT_TASK)
        .expect("JNI_OnLoad() found ShowTextInputTask");
    let this = env.new_object(class.obj(), c"()V", &[])?;

    /* Minimum size of 1 pixel, so it takes focus. */
    if w <= 0 {
        w = 1;
    }
    if h + HEIGHT_PADDING <= 0 {
        h = 1 - HEIGHT_PADDING;
    }
    for (name, value) in [
        (c"input_type", input_type),
        (c"x", x),
        (c"y", y),
        (c"w", w),
        (c"h", h),
    ] {
        env.set_field(this.obj(), name, c"I", Arg::Int(value))?;
    }
    Ok(this)
}

/// `View.VISIBLE`.
const VISIBLE: jint = 0;

/// The text edit over the text input area, of the input type, focused, and
/// the input method's keyboard shown. Translation of
/// `ShowTextInputTask.run()`.
pub(crate) extern "system" fn show_text_input_task_run(env: *mut JNIEnv, this: jobject) {
    // SAFETY: a native method's environment and object.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    native(env, |env| {
        let field = |name: &CStr| env.get_field(this, name, c"I").map(env::Value::int);
        let (input_type, x, y, w, h) = (
            field(c"input_type")?,
            field(c"x")?,
            field(c"y")?,
            field(c"w")?,
            field(c"h")?,
        );
        let layout = {
            let s = statics();
            s.layout.as_ref().and_then(|l| env.new_local_ref(l.obj()))
        };
        let Some(layout) = layout else {
            return Ok(());
        };

        let params = new_relative_layout_params(env, w, h + HEIGHT_PADDING)?;
        env.set_field(params.obj(), c"leftMargin", c"I", Arg::Int(x))?;
        env.set_field(params.obj(), c"topMargin", c"I", Arg::Int(y))?;

        let text_edit = match text_edit(env) {
            None => {
                let context = context_or_throw(env)?;
                let text_edit = super::dummy_edit::new_sdl_dummy_edit(env, context.obj())?;
                statics().text_edit = Global::new(env, text_edit.obj());

                env.call_method(
                    layout.obj(),
                    c"addView",
                    c"(Landroid/view/View;Landroid/view/ViewGroup$LayoutParams;)V",
                    &[Arg::Object(text_edit.obj()), Arg::Object(params.obj())],
                )?;
                text_edit
            }
            Some(text_edit) => {
                env.call_method(
                    text_edit.obj(),
                    c"setLayoutParams",
                    c"(Landroid/view/ViewGroup$LayoutParams;)V",
                    &[Arg::Object(params.obj())],
                )?;
                text_edit
            }
        };
        super::dummy_edit::set_input_type(env, text_edit.obj(), input_type)?;

        env.call_method(
            text_edit.obj(),
            c"setVisibility",
            c"(I)V",
            &[Arg::Int(VISIBLE)],
        )?;
        env.call_method(text_edit.obj(), c"requestFocus", c"()Z", &[])?;

        let context = context_or_throw(env)?;
        let imm = input_method_manager(env, context.obj())?;
        env.call_method(
            imm.obj(),
            c"showSoftInput",
            c"(Landroid/view/View;I)Z",
            &[Arg::Object(text_edit.obj()), Arg::Int(0)],
        )?;

        if env
            .call_method(imm.obj(), c"isAcceptingText", c"()Z", &[])?
            .boolean()
        {
            super::super::on_screen_keyboard_shown();
        }
        Ok(())
    });
}

/// This method is called by SDL using JNI: the text edit shown on the main
/// thread, whether its task was posted. Translation of `showTextInput()`.
pub(crate) fn show_text_input<'a>(
    env: Env<'a>,
    input_type: jint,
    x: jint,
    y: jint,
    w: jint,
    h: jint,
) -> JniResult<'a, bool> {
    if statics().layout.is_none() {
        return Ok(false);
    }

    // Transfer the task to the main thread as a Runnable
    let task = new_show_text_input_task(env, input_type, x, y, w, h)?;
    let singleton = singleton_or_throw(env)?;
    let Some(command_handler) = env
        .get_field(singleton.obj(), c"commandHandler", c"Landroid/os/Handler;")?
        .object()
    else {
        return null_pointer(env, c"SDLActivity.commandHandler is null");
    };
    Ok(env
        .call_method(
            command_handler.obj(),
            c"post",
            c"(Ljava/lang/Runnable;)Z",
            &[Arg::Object(task.obj())],
        )?
        .boolean())
}

/// `KeyEvent.KEYCODE_SPACE`.
const KEYCODE_SPACE: jint = 62;

/// Whether a key event's character is text (`SDL_EVENT_TEXT_INPUT`).
/// Translation of `isTextInputEvent()`.
pub(crate) fn is_text_input_event<'a>(env: Env<'a>, event: Obj<'_>) -> JniResult<'a, bool> {
    // Key pressed with Ctrl should be sent as SDL_KEYDOWN/SDL_KEYUP and not SDL_TEXTINPUT
    if env
        .call_method(event, c"isCtrlPressed", c"()Z", &[])?
        .boolean()
    {
        return Ok(false);
    }

    Ok(env
        .call_method(event, c"isPrintingKey", c"()Z", &[])?
        .boolean()
        || env.call_method(event, c"getKeyCode", c"()I", &[])?.int() == KEYCODE_SPACE)
}

/// `KeyEvent.FLAG_FALLBACK`.
const FLAG_FALLBACK: jint = 0x400;
/// `InputDevice.SOURCE_UNKNOWN`, `SOURCE_MOUSE`.
const SOURCE_UNKNOWN: jint = 0;
const SOURCE_MOUSE: jint = 0x2002;
/// `KeyEvent.ACTION_DOWN`, `ACTION_UP`.
const ACTION_DOWN: jint = 0;
const ACTION_UP: jint = 1;
/// `KeyEvent.KEYCODE_FORWARD`.
const KEYCODE_FORWARD: jint = 125;

/// A view's key event (the surface's, the text edit's: `onKey()`), with
/// the text edit's input connection (`ic`, which commits a key's text; null
/// for the surface): whether it was handled. Translation of
/// `handleKeyEvent()`.
pub(crate) fn handle_key_event<'a>(
    env: Env<'a>,
    _v: Obj<'_>,
    key_code: jint,
    event: Obj<'_>,
    ic: Obj<'_>,
) -> JniResult<'a, bool> {
    let device_id = env.call_method(event, c"getDeviceId", c"()I", &[])?.int();
    let mut source = env.call_method(event, c"getSource", c"()I", &[])?.int();
    let input_device = env.find_class(c"android/view/InputDevice")?;
    let device = env
        .call_static_method(
            input_device.obj(),
            c"getDevice",
            c"(I)Landroid/view/InputDevice;",
            &[Arg::Int(device_id)],
        )?
        .object();

    if (env.call_method(event, c"getFlags", c"()I", &[])?.int() & FLAG_FALLBACK) != 0 {
        // If this is a fallback event -- e.g., the mouse is being turned into dpad navigational keys
        // just eat it, since that's almost never what we actually want to have happen. This will
        // happen with Logitech mice on Amazon Fire TV devices and some Samsung devices.
        return Ok(true);
    }

    if source == SOURCE_UNKNOWN {
        if let Some(device) = &device {
            source = env
                .call_method(device.obj(), c"getSources", c"()I", &[])?
                .int();
        }
    }

    // Dispatch the different events depending on where they come from
    // Some SOURCE_JOYSTICK, SOURCE_DPAD or SOURCE_GAMEPAD are also SOURCE_KEYBOARD
    // So, we try to process them as JOYSTICK/DPAD/GAMEPAD events first, if that fails we try them as KEYBOARD
    //
    // Furthermore, it's possible a game controller has SOURCE_KEYBOARD and
    // SOURCE_JOYSTICK, while its key events arrive from the keyboard source
    // So, retrieve the device itself and check all of its sources
    let action = env.call_method(event, c"getAction", c"()I", &[])?.int();
    if sdl::is_controller_manager_ready()
        && controller_manager::is_device_sdl_joystick(env, device.as_ref().map(Local::obj))?
    {
        // Note that we process events with specific key codes here
        let scan_code = env.call_method(event, c"getScanCode", c"()I", &[])?.int();
        if action == ACTION_DOWN {
            if controller_manager::on_native_pad_down(device_id, key_code, scan_code) {
                return Ok(true);
            }
        } else if action == ACTION_UP
            && controller_manager::on_native_pad_up(device_id, key_code, scan_code)
        {
            return Ok(true);
        }
    }

    if (source & SOURCE_MOUSE) == SOURCE_MOUSE {
        if is_vr_headset(env)? {
            // The Oculus Quest controller back button comes in as source mouse, so accept that
        } else {
            // on some devices key events are sent for mouse BUTTON_BACK/FORWARD presses
            // they are ignored here because sending them as mouse input to SDL is messy
            if (key_code == KEYCODE_BACK || key_code == KEYCODE_FORWARD)
                && (action == ACTION_DOWN || action == ACTION_UP)
            {
                // mark the event as handled or it will be handled by system
                // handling KEYCODE_BACK by system will call onBackPressed()
                return Ok(true);
            }
        }
    }

    if action == ACTION_DOWN {
        super::super::on_key_down(key_code);

        if is_text_input_event(env, event)? {
            // String.valueOf((char) event.getUnicodeChar())
            let unicode_char = env
                .call_method(event, c"getUnicodeChar", c"()I", &[])?
                .int() as jchar;
            if !ic.is_null() {
                let text = env.new_string_utf16(&[unicode_char])?;
                env.call_method(
                    ic,
                    c"commitText",
                    c"(Ljava/lang/CharSequence;I)Z",
                    &[Arg::Object(text.obj()), Arg::Int(1)],
                )?;
            } else {
                // SDLInputConnection.nativeCommitText(String.valueOf(...), 1)
                super::super::commit_text(&String::from_utf16_lossy(&[unicode_char]));
            }
        }
        return Ok(true);
    } else if action == ACTION_UP {
        super::super::on_key_up(key_code);
        return Ok(true);
    }

    Ok(false)
}

/// This method is called by SDL using JNI: the surface's `Surface` (null
/// without a surface). Translation of `getNativeSurface()`.
pub(crate) fn get_native_surface<'a>(env: Env<'a>) -> JniResult<'a, Option<Local<'a>>> {
    let surface = {
        let s = statics();
        s.surface.as_ref().and_then(|s| env.new_local_ref(s.obj()))
    };
    match surface {
        None => Ok(None),
        Some(surface) => surface::get_native_surface(env, surface.obj()),
    }
}

/// `rehideSystemUi`: an `SDLNativeListener`'s proxy of `Runnable`, which
/// hides the system bars again (Note: a proxy, which runs each time it's
/// posted, as upstream's anonymous `Runnable` does; and its
/// `SDLActivity.this` is the activity of the moment, `mSingleton`).
fn new_rehide_system_ui<'a>(env: Env<'a>) -> JniResult<'a, Local<'a>> {
    anonymous::listener(env, c"java/lang/Runnable", |env, method, _args| {
        if method == "run" {
            rehide_system_ui_run(env)?;
        }
        Ok(None)
    })
}

/// `rehideSystemUi.run()`.
fn rehide_system_ui_run<'a>(env: Env<'a>) -> JniResult<'a, ()> {
    let this = singleton_or_throw(env)?;
    let window = env
        .call_method(this.obj(), c"getWindow", c"()Landroid/view/Window;", &[])?
        .object()
        .expect("the activity's window");
    if super::super::sdk_int(env) >= 30
    /* Android 11 (R) */
    {
        let controller = env
            .call_method(
                window.obj(),
                c"getInsetsController",
                c"()Landroid/view/WindowInsetsController;",
                &[],
            )?
            .object();
        if let Some(controller) = controller {
            let types = env.find_class(c"android/view/WindowInsets$Type")?;
            let system_bars = env
                .call_static_method(types.obj(), c"systemBars", c"()I", &[])?
                .int();
            env.call_method(controller.obj(), c"hide", c"(I)V", &[Arg::Int(system_bars)])?;
        }
    } else {
        let flags = SYSTEM_UI_FLAG_FULLSCREEN
            | SYSTEM_UI_FLAG_HIDE_NAVIGATION
            | SYSTEM_UI_FLAG_IMMERSIVE_STICKY
            | SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
            | SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
            | SYSTEM_UI_FLAG_LAYOUT_STABLE
            | INVISIBLE;

        set_system_ui_visibility(env, window.obj(), flags)?;
    }
    Ok(())
}

/// The system UI shown while the activity is fullscreen: hidden again 2 s
/// later. Translation of `onSystemUiVisibilityChange()`.
pub(crate) extern "system" fn on_system_ui_visibility_change(
    env: *mut JNIEnv,
    this: jobject,
    visibility: jint,
) {
    // SAFETY: a native method's environment and object.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    native(env, |env| {
        if statics().fullscreen_mode_active
            && ((visibility & SYSTEM_UI_FLAG_FULLSCREEN) == 0
                || (visibility & SYSTEM_UI_FLAG_HIDE_NAVIGATION) == 0)
        {
            let window = env
                .call_method(this, c"getWindow", c"()Landroid/view/Window;", &[])?
                .object()
                .expect("the activity's window");
            let decor_view = env
                .call_method(window.obj(), c"getDecorView", c"()Landroid/view/View;", &[])?
                .object()
                .expect("a decor view");
            let handler = env
                .call_method(
                    decor_view.obj(),
                    c"getHandler",
                    c"()Landroid/os/Handler;",
                    &[],
                )?
                .object();
            if let Some(handler) = handler {
                let rehide_system_ui = env
                    .get_field(this, c"rehideSystemUi", c"Ljava/lang/Runnable;")?
                    .object();
                let rehide_system_ui = rehide_system_ui.as_ref().map_or(Obj::NULL, Local::obj);
                env.call_method(
                    handler.obj(),
                    c"removeCallbacks",
                    c"(Ljava/lang/Runnable;)V",
                    &[Arg::Object(rehide_system_ui)],
                )?; // Prevent a hide loop.
                env.call_method(
                    handler.obj(),
                    c"postDelayed",
                    c"(Ljava/lang/Runnable;J)Z",
                    &[Arg::Object(rehide_system_ui), Arg::Long(2000)],
                )?;
            }
        }
        Ok(())
    });
}

/// This method is called by SDL using JNI: a command and its parameter to
/// the command handler; false without an activity. Translation of
/// `sendMessage()`.
pub(crate) fn send_message<'a>(env: Env<'a>, command: jint, param: jint) -> JniResult<'a, bool> {
    let Some(singleton) = singleton(env) else {
        return Ok(false);
    };
    let param = new_integer(env, param)?;
    send_command(env, singleton.obj(), command, param.obj())
}

/// This method is called by SDL using JNI. Translation of `getContext()`.
pub(crate) fn get_context(env: Env<'_>) -> Option<Local<'_>> {
    sdl::get_context(env)
}

/// `getContext()`, or the `NullPointerException` of using it null.
fn context_or_throw<'a>(env: Env<'a>) -> JniResult<'a, Local<'a>> {
    match get_context(env) {
        Some(context) => Ok(context),
        None => null_pointer(env, c"SDLActivity.getContext() is null"),
    }
}

/// This method is called by SDL using JNI. Translation of `isAndroidTV()`.
pub(crate) fn is_android_tv<'a>(env: Env<'a>) -> JniResult<'a, bool> {
    let context = context_or_throw(env)?;
    let service = env.new_string("uimode")?; // UI_MODE_SERVICE
    let ui_mode_manager = env
        .call_method(
            context.obj(),
            c"getSystemService",
            c"(Ljava/lang/String;)Ljava/lang/Object;",
            &[Arg::Object(service.obj())],
        )?
        .object();
    let Some(ui_mode_manager) = ui_mode_manager else {
        return null_pointer(env, c"no UiModeManager");
    };
    const UI_MODE_TYPE_TELEVISION: jint = 4; // Configuration.UI_MODE_TYPE_TELEVISION
    if env
        .call_method(ui_mode_manager.obj(), c"getCurrentModeType", c"()I", &[])?
        .int()
        == UI_MODE_TYPE_TELEVISION
    {
        return Ok(true);
    }
    let manufacturer = build_field(env, c"MANUFACTURER")?;
    let model = build_field(env, c"MODEL")?.unwrap_or_default();
    if manufacturer.as_deref() == Some("MINIX") && model == "NEO-U1" {
        return Ok(true);
    }
    if manufacturer.as_deref() == Some("Amlogic")
        && (model.starts_with("TV") || model == "X96-W" || model == "A95X-R1")
    {
        return Ok(true);
    }
    Ok(false)
}

/// Translation of `isVRHeadset()`.
fn is_vr_headset<'a>(env: Env<'a>) -> JniResult<'a, bool> {
    let manufacturer = build_field(env, c"MANUFACTURER")?;
    let model = build_field(env, c"MODEL")?.unwrap_or_default();
    if manufacturer.as_deref() == Some("Oculus") && model.starts_with("Quest") {
        return Ok(true);
    }
    if manufacturer.as_deref() == Some("Pico") {
        return Ok(true);
    }
    Ok(false)
}

/// This method is called by SDL using JNI: `tv`, `headset`, `tablet` or
/// `phone`. Translation of `getDeviceFormFactor()`.
pub(crate) fn get_device_form_factor<'a>(env: Env<'a>) -> JniResult<'a, String> {
    // TODO: WearOS
    let form_factor = if is_android_tv(env)? {
        "tv"
    } else if is_vr_headset(env)? {
        "headset"
    } else if is_tablet(env)? {
        "tablet"
    //} else if (isAndroidAutomotive()) {
    //    return "car";
    } else {
        "phone"
    };
    Ok(form_factor.to_string())
}

/// The display's diagonal, in inches (0 without an activity). Translation
/// of `getDiagonal()`.
fn get_diagonal<'a>(env: Env<'a>) -> JniResult<'a, f64> {
    let metrics_class = env.find_class(c"android/util/DisplayMetrics")?;
    let metrics = env.new_object(metrics_class.obj(), c"()V", &[])?;
    let Some(activity) = get_context(env) else {
        return Ok(0.0);
    };
    let window_manager = env
        .call_method(
            activity.obj(),
            c"getWindowManager",
            c"()Landroid/view/WindowManager;",
            &[],
        )?
        .object()
        .expect("a window manager");
    let display = env
        .call_method(
            window_manager.obj(),
            c"getDefaultDisplay",
            c"()Landroid/view/Display;",
            &[],
        )?
        .object()
        .expect("a display");
    env.call_method(
        display.obj(),
        c"getMetrics",
        c"(Landroid/util/DisplayMetrics;)V",
        &[Arg::Object(metrics.obj())],
    )?;

    let width_pixels = env.get_field(metrics.obj(), c"widthPixels", c"I")?.int();
    let height_pixels = env.get_field(metrics.obj(), c"heightPixels", c"I")?.int();
    let xdpi = env.get_field(metrics.obj(), c"xdpi", c"F")?.float();
    let ydpi = env.get_field(metrics.obj(), c"ydpi", c"F")?.float();
    let d_width_inches = width_pixels as f64 / xdpi as f64;
    let d_height_inches = height_pixels as f64 / ydpi as f64;

    Ok(((d_width_inches * d_width_inches) + (d_height_inches * d_height_inches)).sqrt())
}

/// This method is called by SDL using JNI. Translation of `isTablet()`.
pub(crate) fn is_tablet<'a>(env: Env<'a>) -> JniResult<'a, bool> {
    // If our diagonal size is seven inches or greater, we consider ourselves a tablet.
    Ok(get_diagonal(env)? >= 7.0)
}

/// This method is called by SDL using JNI. Translation of
/// `isChromebook()`.
pub(crate) fn is_chromebook<'a>(env: Env<'a>) -> JniResult<'a, bool> {
    // https://stackoverflow.com/questions/39784415/how-to-detect-programmatically-if-android-app-is-running-in-chrome-book-or-in
    if let Some(context) = get_context(env) {
        let package_manager = env
            .call_method(
                context.obj(),
                c"getPackageManager",
                c"()Landroid/content/pm/PackageManager;",
                &[],
            )?
            .object()
            .expect("a package manager");
        for feature in ["org.chromium.arc", "org.chromium.arc.device_management"] {
            let feature = env.new_string(feature)?;
            if env
                .call_method(
                    package_manager.obj(),
                    c"hasSystemFeature",
                    c"(Ljava/lang/String;)Z",
                    &[Arg::Object(feature.obj())],
                )?
                .boolean()
            {
                return Ok(true);
            }
        }
    }

    // Running on AVD emulator
    let is_chromebook_emulator =
        build_field(env, c"MODEL")?.is_some_and(|model| model.starts_with("sdk_gpc_"));
    Ok(is_chromebook_emulator)
}

/// This method is called by SDL using JNI. Translation of `isDeXMode()`.
pub(crate) fn is_dex_mode<'a>(env: Env<'a>) -> JniResult<'a, bool> {
    if super::super::sdk_int(env) < 24
    /* Android 7.0 (N) */
    {
        return Ok(false);
    }
    match dex_mode(env) {
        Ok(dex) => Ok(dex),
        // catch(Exception ignored)
        Err(thrown) => {
            drop(thrown.catch_exception()?);
            Ok(false)
        }
    }
}

/// The `try` block of [`is_dex_mode`]: Samsung's configuration fields.
fn dex_mode<'a>(env: Env<'a>) -> JniResult<'a, bool> {
    let context = context_or_throw(env)?;
    let config = configuration(env, context.obj())?;
    let config_class = env.get_object_class(config.obj()).expect("a class");
    let get_int = |name: &str, of: Obj<'_>| -> JniResult<'a, jint> {
        let name = env.new_string(name)?;
        let field = env
            .call_method(
                config_class.obj(),
                c"getField",
                c"(Ljava/lang/String;)Ljava/lang/reflect/Field;",
                &[Arg::Object(name.obj())],
            )?
            .object()
            .expect("a field");
        Ok(env
            .call_method(
                field.obj(),
                c"getInt",
                c"(Ljava/lang/Object;)I",
                &[Arg::Object(of)],
            )?
            .int())
    };
    Ok(get_int("SEM_DESKTOP_MODE_ENABLED", config_class.obj())?
        == get_int("semDesktopModeEnabled", config.obj())?)
}

/// This method is called by SDL using JNI: the manifest's `SDL_ENV.NAME`
/// meta-data, set as the environment variables NAME (`nativeSetenv()`).
/// Translation of `getManifestEnvironmentVariables()`.
pub(crate) fn get_manifest_environment_variables(env: Env<'_>) -> bool {
    // (in a local frame: a Java method's references end with it)
    let result = env.with_local_frame(16, |env| match manifest_environment_variables(env) {
        Ok(set) => set,
        Err(thrown) => match thrown.catch_exception() {
            Ok(e) => {
                log(
                    LogPriority::Verbose,
                    &format!("exception {}", to_string(env, e.obj())),
                );
                false
            }
            // (an Error, which the method doesn't catch: pending, as
            // upstream's C caller leaves it)
            Err(thrown) => {
                drop(thrown);
                false
            }
        },
    });
    result.unwrap_or_else(|thrown| {
        drop(thrown);
        false
    })
}

/// The `try` block of `getManifestEnvironmentVariables()`.
fn manifest_environment_variables(env: Env<'_>) -> JniResult<'_, bool> {
    let Some(context) = sdl::get_context(env) else {
        return Ok(false);
    };

    const GET_META_DATA: jint = 0x80; // PackageManager.GET_META_DATA
    let package_manager = env
        .call_method(
            context.obj(),
            c"getPackageManager",
            c"()Landroid/content/pm/PackageManager;",
            &[],
        )?
        .object()
        .expect("a PackageManager");
    let package_name = env
        .call_method(
            context.obj(),
            c"getPackageName",
            c"()Ljava/lang/String;",
            &[],
        )?
        .object();
    let application_info = env
        .call_method(
            package_manager.obj(),
            c"getApplicationInfo",
            c"(Ljava/lang/String;I)Landroid/content/pm/ApplicationInfo;",
            &[
                Arg::Object(package_name.as_ref().map_or(Obj::NULL, Local::obj)),
                Arg::Int(GET_META_DATA),
            ],
        )?
        .object()
        .expect("an ApplicationInfo");
    let Some(bundle) = env
        .get_field(application_info.obj(), c"metaData", c"Landroid/os/Bundle;")?
        .object()
    else {
        return Ok(false);
    };
    let prefix = "SDL_ENV.";
    let keys = env
        .call_method(bundle.obj(), c"keySet", c"()Ljava/util/Set;", &[])?
        .object()
        .expect("a key set");
    let iterator = env
        .call_method(keys.obj(), c"iterator", c"()Ljava/util/Iterator;", &[])?
        .object()
        .expect("an iterator");
    while env
        .call_method(iterator.obj(), c"hasNext", c"()Z", &[])?
        .boolean()
    {
        let key = env
            .call_method(iterator.obj(), c"next", c"()Ljava/lang/Object;", &[])?
            .object()
            .expect("a key");
        let key_string = env.get_string(key.obj())?;
        if let Some(name) = key_string.strip_prefix(prefix) {
            let value = env
                .call_method(
                    bundle.obj(),
                    c"get",
                    c"(Ljava/lang/String;)Ljava/lang/Object;",
                    &[Arg::Object(key.obj())],
                )?
                .object();
            let Some(value) = value else {
                // (`.toString()` of null)
                return null_pointer(
                    env,
                    c"Attempt to invoke virtual method 'java.lang.String java.lang.Object.toString()' on a null object reference",
                );
            };
            let value = env
                .call_method(value.obj(), c"toString", c"()Ljava/lang/String;", &[])?
                .object();
            let name = env.new_string(name)?;
            if let Some(value) = value {
                super::super::setenv(env, name.obj(), value.obj());
            }
        }
    }
    /* environment variables set! */
    Ok(true)
}

// Input

/// `InputDevice.SOURCE_TOUCHSCREEN`.
const SOURCE_TOUCHSCREEN: jint = 0x1002;

/// This method is called by SDL using JNI: the touch screens, and the
/// virtual devices (they can send touch screen events), to
/// `nativeAddTouch()`. Translation of `initTouch()`.
pub(crate) fn init_touch<'a>(env: Env<'a>) -> JniResult<'a, ()> {
    let input_device = env.find_class(c"android/view/InputDevice")?;
    let ids = env
        .call_static_method(input_device.obj(), c"getDeviceIds", c"()[I", &[])?
        .object();
    let Some(ids) = ids else {
        return null_pointer(env, c"InputDevice.getDeviceIds() is null");
    };

    for index in 0..env.get_array_length(ids.obj()) {
        let id = env.get_int_array_element(ids.obj(), index)?;
        let device = env
            .call_static_method(
                input_device.obj(),
                c"getDevice",
                c"(I)Landroid/view/InputDevice;",
                &[Arg::Int(id)],
            )?
            .object();
        let Some(device) = device else {
            continue;
        };
        /* Allow SOURCE_TOUCHSCREEN and also Virtual InputDevices because they can send TOUCHSCREEN events */
        let sources = env
            .call_method(device.obj(), c"getSources", c"()I", &[])?
            .int();
        if (sources & SOURCE_TOUCHSCREEN) == SOURCE_TOUCHSCREEN
            || env
                .call_method(device.obj(), c"isVirtual", c"()Z", &[])?
                .boolean()
        {
            let device_id = env.call_method(device.obj(), c"getId", c"()I", &[])?.int();
            let name = env
                .call_method(device.obj(), c"getName", c"()Ljava/lang/String;", &[])?
                .object();
            let name = match name {
                Some(name) => env.get_string(name.obj())?,
                None => return null_pointer(env, c"InputDevice.getName() is null"),
            };
            super::super::add_touch(device_id, &name);
        }
    }
    Ok(())
}

/// `Bitmap.Config.ARGB_8888`.
fn bitmap_config_argb_8888<'a>(env: Env<'a>) -> JniResult<'a, Local<'a>> {
    let config = env.find_class(c"android/graphics/Bitmap$Config")?;
    env.get_static_field(
        config.obj(),
        c"ARGB_8888",
        c"Landroid/graphics/Bitmap$Config;",
    )?
    .object()
    .map_or_else(|| null_pointer(env, c"Bitmap.Config.ARGB_8888 is null"), Ok)
}

/// This method is called by SDL using JNI: a cursor of the ARGB pixels
/// (`PointerIcon.create()`, API 24), its ID (0 when it can't be made).
/// Translation of `createCustomCursor()`.
pub(crate) fn create_custom_cursor<'a>(
    env: Env<'a>,
    colors: &[jint],
    width: jint,
    height: jint,
    hot_spot_x: jint,
    hot_spot_y: jint,
) -> JniResult<'a, jint> {
    let colors = env.new_int_array(colors)?;
    let bitmap_class = env.find_class(c"android/graphics/Bitmap")?;
    let config = bitmap_config_argb_8888(env)?;
    let bitmap = env
        .call_static_method(
            bitmap_class.obj(),
            c"createBitmap",
            c"([IIILandroid/graphics/Bitmap$Config;)Landroid/graphics/Bitmap;",
            &[
                Arg::Object(colors.obj()),
                Arg::Int(width),
                Arg::Int(height),
                Arg::Object(config.obj()),
            ],
        )?
        .object();
    let id = {
        let mut s = statics();
        s.last_cursor_id += 1;
        s.last_cursor_id
    };

    if super::super::sdk_int(env) >= 24
    /* Android 7.0 (N) */
    {
        let pointer_icon = env.find_class(c"android/view/PointerIcon")?;
        let icon = env.call_static_method(
            pointer_icon.obj(),
            c"create",
            c"(Landroid/graphics/Bitmap;FF)Landroid/view/PointerIcon;",
            &[
                Arg::Object(bitmap.as_ref().map_or(Obj::NULL, Local::obj)),
                Arg::Float(hot_spot_x as f32),
                Arg::Float(hot_spot_y as f32),
            ],
        );
        match icon {
            Ok(icon) => {
                let icon = icon.object();
                let icon = icon.as_ref().and_then(|icon| Global::new(env, icon.obj()));
                // (Hashtable.put() of a null value throws)
                let Some(icon) = icon else {
                    return Ok(0);
                };
                statics().cursors.insert(id, icon);
            }
            Err(thrown) => {
                // catch (Exception e)
                drop(thrown.catch_exception()?);
                return Ok(0);
            }
        }
    } else {
        return Ok(0);
    }
    Ok(id)
}

/// This method is called by SDL using JNI. Translation of
/// `destroyCustomCursor()`.
pub(crate) fn destroy_custom_cursor(env: Env<'_>, cursor_id: jint) {
    if super::super::sdk_int(env) >= 24
    /* Android 7.0 (N) */
    {
        statics().cursors.remove(&cursor_id);
    }
}

/// `mSurface.setPointerIcon(icon)`, false if it throws (a null surface).
fn set_pointer_icon<'a>(env: Env<'a>, icon: Obj<'_>) -> JniResult<'a, bool> {
    let surface = {
        let s = statics();
        s.surface.as_ref().and_then(|s| env.new_local_ref(s.obj()))
    };
    let Some(surface) = surface else {
        // (a NullPointerException, caught)
        return Ok(false);
    };
    match env.call_method(
        surface.obj(),
        c"setPointerIcon",
        c"(Landroid/view/PointerIcon;)V",
        &[Arg::Object(icon)],
    ) {
        Ok(_) => Ok(true),
        Err(thrown) => {
            // catch (Exception e)
            drop(thrown.catch_exception()?);
            Ok(false)
        }
    }
}

/// This method is called by SDL using JNI: the surface's pointer icon the
/// custom cursor's (none for an ID without one). Translation of
/// `setCustomCursor()`.
pub(crate) fn set_custom_cursor<'a>(env: Env<'a>, cursor_id: jint) -> JniResult<'a, bool> {
    if super::super::sdk_int(env) >= 24
    /* Android 7.0 (N) */
    {
        let icon = {
            let s = statics();
            s.cursors
                .get(&cursor_id)
                .and_then(|icon| env.new_local_ref(icon.obj()))
        };
        set_pointer_icon(env, icon.as_ref().map_or(Obj::NULL, Local::obj))
    } else {
        Ok(false)
    }
}

/// `SDL_SystemCursor`'s values, upstream's `SDL_SYSTEM_CURSOR_*`.
const SDL_SYSTEM_CURSOR_DEFAULT: jint = 0;
const SDL_SYSTEM_CURSOR_TEXT: jint = 1;
const SDL_SYSTEM_CURSOR_WAIT: jint = 2;
const SDL_SYSTEM_CURSOR_CROSSHAIR: jint = 3;
const SDL_SYSTEM_CURSOR_PROGRESS: jint = 4;
const SDL_SYSTEM_CURSOR_NWSE_RESIZE: jint = 5;
const SDL_SYSTEM_CURSOR_NESW_RESIZE: jint = 6;
const SDL_SYSTEM_CURSOR_EW_RESIZE: jint = 7;
const SDL_SYSTEM_CURSOR_NS_RESIZE: jint = 8;
const SDL_SYSTEM_CURSOR_MOVE: jint = 9;
const SDL_SYSTEM_CURSOR_NOT_ALLOWED: jint = 10;
const SDL_SYSTEM_CURSOR_POINTER: jint = 11;
const SDL_SYSTEM_CURSOR_NW_RESIZE: jint = 12;
const SDL_SYSTEM_CURSOR_N_RESIZE: jint = 13;
const SDL_SYSTEM_CURSOR_NE_RESIZE: jint = 14;
const SDL_SYSTEM_CURSOR_E_RESIZE: jint = 15;
const SDL_SYSTEM_CURSOR_SE_RESIZE: jint = 16;
const SDL_SYSTEM_CURSOR_S_RESIZE: jint = 17;
const SDL_SYSTEM_CURSOR_SW_RESIZE: jint = 18;
const SDL_SYSTEM_CURSOR_W_RESIZE: jint = 19;
const SDL_SYSTEM_CURSOR_CONTEXT_MENU: jint = 20;
const SDL_SYSTEM_CURSOR_HELP: jint = 21;
const SDL_SYSTEM_CURSOR_CELL: jint = 22;
const SDL_SYSTEM_CURSOR_VERTICAL_TEXT: jint = 23;
const SDL_SYSTEM_CURSOR_ALIAS: jint = 24;
const SDL_SYSTEM_CURSOR_COPY: jint = 25;
const SDL_SYSTEM_CURSOR_NO_DROP: jint = 26;
const SDL_SYSTEM_CURSOR_GRAB: jint = 27;
const SDL_SYSTEM_CURSOR_GRABBING: jint = 28;
const SDL_SYSTEM_CURSOR_COL_RESIZE: jint = 29;
const SDL_SYSTEM_CURSOR_ROW_RESIZE: jint = 30;
const SDL_SYSTEM_CURSOR_ALL_SCROLL: jint = 31;
const SDL_SYSTEM_CURSOR_ZOOM_IN: jint = 32;
const SDL_SYSTEM_CURSOR_ZOOM_OUT: jint = 33;

/// This method is called by SDL using JNI: the surface's pointer icon the
/// system's icon for the cursor. Translation of `setSystemCursor()`.
pub(crate) fn set_system_cursor<'a>(env: Env<'a>, cursor_id: jint) -> JniResult<'a, bool> {
    let cursor_type = match cursor_id {
        SDL_SYSTEM_CURSOR_DEFAULT => 1000,       //PointerIcon.TYPE_ARROW;
        SDL_SYSTEM_CURSOR_TEXT => 1008,          //PointerIcon.TYPE_TEXT;
        SDL_SYSTEM_CURSOR_WAIT => 1004,          //PointerIcon.TYPE_WAIT;
        SDL_SYSTEM_CURSOR_CROSSHAIR => 1007,     //PointerIcon.TYPE_CROSSHAIR;
        SDL_SYSTEM_CURSOR_PROGRESS => 1004,      //PointerIcon.TYPE_WAIT;
        SDL_SYSTEM_CURSOR_NWSE_RESIZE => 1017,   //PointerIcon.TYPE_TOP_LEFT_DIAGONAL_DOUBLE_ARROW;
        SDL_SYSTEM_CURSOR_NESW_RESIZE => 1016,   //PointerIcon.TYPE_TOP_RIGHT_DIAGONAL_DOUBLE_ARROW;
        SDL_SYSTEM_CURSOR_EW_RESIZE => 1014,     //PointerIcon.TYPE_HORIZONTAL_DOUBLE_ARROW;
        SDL_SYSTEM_CURSOR_NS_RESIZE => 1015,     //PointerIcon.TYPE_VERTICAL_DOUBLE_ARROW;
        SDL_SYSTEM_CURSOR_MOVE => 1020,          //PointerIcon.TYPE_GRAB;
        SDL_SYSTEM_CURSOR_NOT_ALLOWED => 1012,   //PointerIcon.TYPE_NO_DROP;
        SDL_SYSTEM_CURSOR_POINTER => 1002,       //PointerIcon.TYPE_HAND;
        SDL_SYSTEM_CURSOR_NW_RESIZE => 1017,     //PointerIcon.TYPE_TOP_LEFT_DIAGONAL_DOUBLE_ARROW;
        SDL_SYSTEM_CURSOR_N_RESIZE => 1015,      //PointerIcon.TYPE_VERTICAL_DOUBLE_ARROW;
        SDL_SYSTEM_CURSOR_NE_RESIZE => 1016,     //PointerIcon.TYPE_TOP_RIGHT_DIAGONAL_DOUBLE_ARROW;
        SDL_SYSTEM_CURSOR_E_RESIZE => 1014,      //PointerIcon.TYPE_HORIZONTAL_DOUBLE_ARROW;
        SDL_SYSTEM_CURSOR_SE_RESIZE => 1017,     //PointerIcon.TYPE_TOP_LEFT_DIAGONAL_DOUBLE_ARROW;
        SDL_SYSTEM_CURSOR_S_RESIZE => 1015,      //PointerIcon.TYPE_VERTICAL_DOUBLE_ARROW;
        SDL_SYSTEM_CURSOR_SW_RESIZE => 1016,     //PointerIcon.TYPE_TOP_RIGHT_DIAGONAL_DOUBLE_ARROW;
        SDL_SYSTEM_CURSOR_W_RESIZE => 1014,      //PointerIcon.TYPE_HORIZONTAL_DOUBLE_ARROW;
        SDL_SYSTEM_CURSOR_CONTEXT_MENU => 1001,  //PointerIcon.TYPE_CONTEXT_MENU;
        SDL_SYSTEM_CURSOR_HELP => 1003,          //PointerIcon.TYPE_HELP;
        SDL_SYSTEM_CURSOR_CELL => 1006,          //PointerIcon.TYPE_CELL;
        SDL_SYSTEM_CURSOR_VERTICAL_TEXT => 1009, //PointerIcon.TYPE_VERTICAL_TEXT;
        SDL_SYSTEM_CURSOR_ALIAS => 1010,         //PointerIcon.TYPE_ALIAS;
        SDL_SYSTEM_CURSOR_COPY => 1011,          //PointerIcon.TYPE_COPY;
        SDL_SYSTEM_CURSOR_NO_DROP => 1012,       //PointerIcon.TYPE_NO_DROP;
        SDL_SYSTEM_CURSOR_GRAB => 1020,          //PointerIcon.TYPE_GRAB;
        SDL_SYSTEM_CURSOR_GRABBING => 1021,      //PointerIcon.TYPE_GRABBING;
        SDL_SYSTEM_CURSOR_COL_RESIZE => 1014,    //PointerIcon.TYPE_HORIZONTAL_DOUBLE_ARROW;
        SDL_SYSTEM_CURSOR_ROW_RESIZE => 1015,    //PointerIcon.TYPE_VERTICAL_DOUBLE_ARROW;
        SDL_SYSTEM_CURSOR_ALL_SCROLL => 1013,    //PointerIcon.TYPE_ALL_SCROLL;
        SDL_SYSTEM_CURSOR_ZOOM_IN => 1018,       //PointerIcon.TYPE_ZOOM_IN;
        SDL_SYSTEM_CURSOR_ZOOM_OUT => 1019,      //PointerIcon.TYPE_ZOOM_OUT;
        _ => 0,                                  //PointerIcon.TYPE_NULL;
    };
    if super::super::sdk_int(env) >= 24
    /* Android 7.0 (N) */
    {
        let Some(context) = get_context(env) else {
            // (getSystemIcon() of a null context throws: caught)
            return Ok(false);
        };
        let pointer_icon = env.find_class(c"android/view/PointerIcon")?;
        let icon = match env.call_static_method(
            pointer_icon.obj(),
            c"getSystemIcon",
            c"(Landroid/content/Context;I)Landroid/view/PointerIcon;",
            &[Arg::Object(context.obj()), Arg::Int(cursor_type)],
        ) {
            Ok(icon) => icon.object(),
            Err(thrown) => {
                // catch (Exception e)
                drop(thrown.catch_exception()?);
                return Ok(false);
            }
        };
        if !set_pointer_icon(env, icon.as_ref().map_or(Obj::NULL, Local::obj))? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// `PackageManager.PERMISSION_GRANTED`.
const PERMISSION_GRANTED: jint = 0;

/// This method is called by SDL using JNI: ask for `permission`, whose
/// answer goes to `nativePermissionResult()` (at once when it's granted
/// already, else from `onRequestPermissionsResult()`, once the user has
/// answered the system's dialog). Translation of `requestPermission()`.
pub(crate) fn request_permission<'a>(
    env: Env<'a>,
    permission: &str,
    request_code: jint,
) -> JniResult<'a, ()> {
    if super::super::sdk_int(env) < 23
    /* Android 6.0 (M) */
    {
        super::super::permission_result(request_code, true);
        return Ok(());
    }

    let activity = context_or_throw(env)?;
    let permission = env.new_string(permission)?;
    let granted = env
        .call_method(
            activity.obj(),
            c"checkSelfPermission",
            c"(Ljava/lang/String;)I",
            &[Arg::Object(permission.obj())],
        )?
        .int();
    if granted != PERMISSION_GRANTED {
        let string = env.find_class(c"java/lang/String")?;
        let permissions = env.new_object_array(1, string.obj(), permission.obj())?;
        env.call_method(
            activity.obj(),
            c"requestPermissions",
            c"([Ljava/lang/String;I)V",
            &[Arg::Object(permissions.obj()), Arg::Int(request_code)],
        )?;
    } else {
        super::super::permission_result(request_code, true);
    }
    Ok(())
}

/// Translation of `onRequestPermissionsResult()`: the first permission's
/// answer to `nativePermissionResult()`.
pub(crate) extern "system" fn on_request_permissions_result(
    env: *mut JNIEnv,
    _this: jobject,
    request_code: jint,
    _permissions: jobject,
    grant_results: jobject,
) {
    // SAFETY: a native method's environment and arguments.
    let (env, grant_results) = unsafe { (Env::from_raw(env), Obj::from_raw(grant_results)) };
    native(env, |env| {
        let result = if grant_results.is_null() {
            // (grantResults.length of null)
            return null_pointer(env, c"Attempt to get length of null array");
        } else {
            env.get_array_length(grant_results) > 0
                && env.get_int_array_element(grant_results, 0)? == PERMISSION_GRANTED
        };
        super::super::permission_result(request_code, result);
        Ok(())
    });
}

/// `Intent.FLAG_ACTIVITY_NO_HISTORY`.
const FLAG_ACTIVITY_NO_HISTORY: jint = 0x40000000;
/// `Intent.FLAG_ACTIVITY_MULTIPLE_TASK`.
const FLAG_ACTIVITY_MULTIPLE_TASK: jint = 0x08000000;
/// `Intent.FLAG_ACTIVITY_NEW_DOCUMENT`.
const FLAG_ACTIVITY_NEW_DOCUMENT: jint = 0x00080000;

/// `Uri.parse(uri)`.
fn parse_uri<'a>(env: Env<'a>, uri: &str) -> JniResult<'a, Local<'a>> {
    let uri_class = env.find_class(c"android/net/Uri")?;
    let uri = env.new_string(uri)?;
    Ok(env
        .call_static_method(
            uri_class.obj(),
            c"parse",
            c"(Ljava/lang/String;)Landroid/net/Uri;",
            &[Arg::Object(uri.obj())],
        )?
        .object()
        .expect("Uri.parse() gives a Uri"))
}

/// This method is called by SDL using JNI: the URL viewed (`ACTION_VIEW`)
/// by the activity that answers it, false when none does (or something
/// else threw). Translation of `openURL()`.
pub(crate) fn open_url<'a>(env: Env<'a>, url: &str) -> JniResult<'a, bool> {
    let opened = (|| -> JniResult<'a, ()> {
        let intent_class = env.find_class(c"android/content/Intent")?;
        let action = env.new_string("android.intent.action.VIEW")?; // Intent.ACTION_VIEW
        let i = env.new_object(
            intent_class.obj(),
            c"(Ljava/lang/String;)V",
            &[Arg::Object(action.obj())],
        )?;
        let uri = parse_uri(env, url)?;
        env.call_method(
            i.obj(),
            c"setData",
            c"(Landroid/net/Uri;)Landroid/content/Intent;",
            &[Arg::Object(uri.obj())],
        )?;

        let flags =
            FLAG_ACTIVITY_NO_HISTORY | FLAG_ACTIVITY_MULTIPLE_TASK | FLAG_ACTIVITY_NEW_DOCUMENT;
        env.call_method(
            i.obj(),
            c"addFlags",
            c"(I)Landroid/content/Intent;",
            &[Arg::Int(flags)],
        )?;

        let singleton = singleton_or_throw(env)?;
        env.call_method(
            singleton.obj(),
            c"startActivity",
            c"(Landroid/content/Intent;)V",
            &[Arg::Object(i.obj())],
        )?;
        Ok(())
    })();
    match opened {
        Ok(()) => Ok(true),
        // catch (Exception ex)
        Err(thrown) => {
            drop(thrown.catch_exception()?);
            Ok(false)
        }
    }
}

/// This method is called by SDL using JNI: a toast of `message` for
/// `duration` (0: short, 1: long), at `gravity` (when it's 0 or more) and
/// the offsets, shown on the main thread; false without an activity.
/// Translation of `showToast()`, whose `OneShotTask` (a local class that
/// implements `Runnable`) is an `SDLNativeRunnable` here (Note: another
/// name).
pub(crate) fn show_toast<'a>(
    env: Env<'a>,
    message: &str,
    duration: jint,
    gravity: jint,
    x_offset: jint,
    y_offset: jint,
) -> JniResult<'a, bool> {
    let Some(singleton) = singleton(env) else {
        return Ok(false);
    };

    let shown = (|| -> JniResult<'a, ()> {
        let message = message.to_owned();
        let task = anonymous::runnable(env, move |env| {
            // (OneShotTask.run())
            if let Err(thrown) =
                one_shot_task_run(env, &message, duration, gravity, x_offset, y_offset)
            {
                // catch(Exception ex): Log.e(TAG, ex.getMessage()) (Note
                // (upstream): Log.e() of a null message throws; here it's
                // logged as "null")
                match thrown.catch_exception() {
                    Ok(ex) => match message_of(env, ex.obj()) {
                        Ok((_, text)) => log(LogPriority::Error, &text),
                        Err(thrown) => drop(thrown),
                    },
                    // (an Error: pending, for the looper)
                    Err(thrown) => drop(thrown),
                }
            }
        })?;
        env.call_method(
            singleton.obj(),
            c"runOnUiThread",
            c"(Ljava/lang/Runnable;)V",
            &[Arg::Object(task.obj())],
        )?;
        Ok(())
    })();
    match shown {
        Ok(()) => Ok(true),
        // catch(Exception ex)
        Err(thrown) => {
            drop(thrown.catch_exception()?);
            Ok(false)
        }
    }
}

/// `OneShotTask.run()`'s `try` block: the toast, made and shown.
fn one_shot_task_run<'a>(
    env: Env<'a>,
    message: &str,
    duration: jint,
    gravity: jint,
    x_offset: jint,
    y_offset: jint,
) -> JniResult<'a, ()> {
    let singleton = singleton_or_throw(env)?;
    let toast_class = env.find_class(c"android/widget/Toast")?;
    let text = env.new_string(message)?;
    let toast = env
        .call_static_method(
            toast_class.obj(),
            c"makeText",
            c"(Landroid/content/Context;Ljava/lang/CharSequence;I)Landroid/widget/Toast;",
            &[
                Arg::Object(singleton.obj()),
                Arg::Object(text.obj()),
                Arg::Int(duration),
            ],
        )?
        .object()
        .expect("Toast.makeText() gives a toast");
    if gravity >= 0 {
        env.call_method(
            toast.obj(),
            c"setGravity",
            c"(III)V",
            &[Arg::Int(gravity), Arg::Int(x_offset), Arg::Int(y_offset)],
        )?;
    }
    env.call_method(toast.obj(), c"show", c"()V", &[])?;
    Ok(())
}

/// This method is called by SDL using JNI: a file descriptor of the
/// content `uri` opened in `mode` (the content resolver's: `r`, `w`, `wt`,
/// `wa`, `rw` or `rwt`), which the caller owns, or -1 (no activity, or no
/// such content: `FileNotFoundException`, whose stack trace is printed);
/// the other exceptions are thrown. Translation of `openFileDescriptor()`.
pub(crate) fn open_file_descriptor<'a>(env: Env<'a>, uri: &str, mode: &str) -> JniResult<'a, jint> {
    let Some(singleton) = singleton(env) else {
        return Ok(-1);
    };

    let opened = (|| -> JniResult<'a, jint> {
        let resolver = env
            .call_method(
                singleton.obj(),
                c"getContentResolver",
                c"()Landroid/content/ContentResolver;",
                &[],
            )?
            .object()
            .expect("a content resolver");
        let uri = parse_uri(env, uri)?;
        let mode = env.new_string(mode)?;
        let pfd = env
            .call_method(
                resolver.obj(),
                c"openFileDescriptor",
                c"(Landroid/net/Uri;Ljava/lang/String;)Landroid/os/ParcelFileDescriptor;",
                &[Arg::Object(uri.obj()), Arg::Object(mode.obj())],
            )?
            .object();
        match pfd {
            Some(pfd) => Ok(env.call_method(pfd.obj(), c"detachFd", c"()I", &[])?.int()),
            None => Ok(-1),
        }
    })();
    match opened {
        Ok(fd) => Ok(fd),
        Err(thrown) => {
            // catch (FileNotFoundException e)
            let e = thrown.catch_any(&[c"java/io/FileNotFoundException"])?;
            env.call_method(e.obj(), c"printStackTrace", c"()V", &[])?;
            Ok(-1)
        }
    }
}

/// This method is called by SDL using JNI: the user's locales, most
/// preferred first, as `ll_CC` separated by commas (`LocaleList`'s
/// adjusted default from API 24, else the activity's locale).
/// Translation of `getPreferredLocales()`.
pub(crate) fn get_preferred_locales<'a>(env: Env<'a>) -> JniResult<'a, String> {
    let mut result = String::new();
    if super::super::sdk_int(env) >= 24
    /* Android 7 (N) */
    {
        let locale_list = env.find_class(c"android/os/LocaleList")?;
        let locales = env
            .call_static_method(
                locale_list.obj(),
                c"getAdjustedDefault",
                c"()Landroid/os/LocaleList;",
                &[],
            )?
            .object()
            .expect("a locale list");
        let size = env.call_method(locales.obj(), c"size", c"()I", &[])?.int();
        for i in 0..size {
            if i != 0 {
                result += ",";
            }
            let locale = env
                .call_method(
                    locales.obj(),
                    c"get",
                    c"(I)Ljava/util/Locale;",
                    &[Arg::Int(i)],
                )?
                .object();
            result += &format_locale(env, locale.as_ref().map_or(Obj::NULL, Local::obj))?;
        }
    } else {
        let current = {
            let s = statics();
            s.current_locale
                .as_ref()
                .and_then(|l| env.new_local_ref(l.obj()))
        };
        if let Some(current) = current {
            result = format_locale(env, current.obj())?;
        }
    }
    Ok(result)
}

/// `a == "literal"` of Java: whether `a` is the string literal's object
/// (literals are interned: `"literal".intern()`).
fn is_literal<'a>(env: Env<'a>, a: Obj<'_>, literal: &str) -> JniResult<'a, bool> {
    let s = env.new_string(literal)?;
    let interned = env
        .call_method(s.obj(), c"intern", c"()Ljava/lang/String;", &[])?
        .object();
    Ok(env.is_same_object(a, interned.as_ref().map_or(Obj::NULL, Local::obj)))
}

/// A locale as `ll` or `ll_CC`. Translation of `formatLocale()`, whose
/// comparisons of strings are Java's `==` (the same object: the locale's
/// strings are interned, so it is their equality there).
pub(crate) fn format_locale<'a>(env: Env<'a>, locale: Obj<'_>) -> JniResult<'a, String> {
    if locale.is_null() {
        return null_pointer(env, c"Attempt to invoke virtual method 'java.lang.String java.util.Locale.getLanguage()' on a null object reference");
    }
    let language = env
        .call_method(locale, c"getLanguage", c"()Ljava/lang/String;", &[])?
        .object();
    let language = language.as_ref().map_or(Obj::NULL, Local::obj);
    let lang = if is_literal(env, language, "in")? {
        // Indonesian is "id" according to ISO 639.2, but on Android is "in" because of Java backwards compatibility
        "id".to_string()
    } else if is_literal(env, language, "")? {
        // Make sure language is never empty
        "und".to_string()
    } else if language.is_null() {
        "null".to_string()
    } else {
        env.get_string(language)?
    };

    let country = env
        .call_method(locale, c"getCountry", c"()Ljava/lang/String;", &[])?
        .object();
    let country = country.as_ref().map_or(Obj::NULL, Local::obj);
    let result = if is_literal(env, country, "")? {
        lang
    } else if country.is_null() {
        format!("{lang}_null")
    } else {
        format!("{lang}_{}", env.get_string(country)?)
    };
    Ok(result)
}

/// Simple runnable to start the SDL application. Translation of
/// `SDLMain.run()`.
pub(crate) extern "system" fn sdl_main_run(env: *mut JNIEnv, _this: jobject) {
    // SAFETY: a native method's environment.
    let env = unsafe { Env::from_raw(env) };
    native(env, |env| {
        // Runs SDLActivity.main()

        let process = env.find_class(c"android/os/Process")?;
        const THREAD_PRIORITY_DISPLAY: jint = -4; // android.os.Process.THREAD_PRIORITY_DISPLAY
        if let Err(thrown) = env.call_static_method(
            process.obj(),
            c"setThreadPriority",
            c"(I)V",
            &[Arg::Int(THREAD_PRIORITY_DISPLAY)],
        ) {
            let e = thrown.catch_exception()?;
            log(
                LogPriority::Verbose,
                &format!(
                    "modify thread properties failed {}",
                    to_string(env, e.obj())
                ),
            );
        }

        super::super::init_main_thread(env);
        main(env)?;
        super::super::cleanup_main_thread();

        let singleton = {
            let s = statics();
            s.singleton
                .as_ref()
                .and_then(|a| env.new_local_ref(a.obj()))
        };
        if let Some(singleton) = singleton {
            if !env
                .call_method(singleton.obj(), c"isFinishing", c"()Z", &[])?
                .boolean()
            {
                // Let's finish the Activity
                {
                    let mut s = statics();
                    s.sdl_thread = None;
                    s.sdl_main_finished = true;
                }
                env.call_method(singleton.obj(), c"finish", c"()V", &[])?;
            } // else: Activity is already being destroyed
        }
        Ok(())
    });
}

/// The activity (`mSingleton`), as a local reference of `env`.
pub(crate) fn singleton<'a>(env: Env<'a>) -> Option<Local<'a>> {
    statics()
        .singleton
        .as_ref()
        .and_then(|a| env.new_local_ref(a.obj()))
}
