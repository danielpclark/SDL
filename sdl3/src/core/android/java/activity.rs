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
//! Not translated yet, and so skipped where upstream does them: the
//! activity's layout and surface (`SDLSurface`, whose state machine calls
//! are [`super::surface`]'s, `SDLDummyEdit`), the system UI's visibility
//! listener, the controller, HID, clipboard and audio managers, the motion
//! listener, the file dialogs' results (`onActivityResult()`), and what
//! the activity offers SDL's C side for the video driver (text input,
//! cursors, orientation, message boxes, ...), permissions, URLs and
//! toasts. They come with the backends that use them. With no surface,
//! the state machine starts the SDL thread as soon as the activity is
//! resumed, as upstream's does without one.
//!
//! Note: the activity's methods that upstream's subclasses may override
//! (`getLibraries()`, `getArguments()`, `onUnhandledMessage()`, ...)
//! can't be overridden here: an app's Java can't extend this
//! `SDLActivity`.

use super::super::env::{self, Arg, Env, Global, JniResult, Local, Obj};
use super::super::jni::*;
use super::super::ndk::LogPriority;
use super::{anonymous, sdl, surface};
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
    /// `mSurface`: none until `SDLSurface` is translated.
    surface: Option<Global>,

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
    #[allow(dead_code)] // (the system UI's visibility listener reads it)
    fullscreen_mode_active: bool,
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
    sdl_thread: None,
    sdl_main_finished: false,
    activity_created: false,
    dispatching_key_event: false,
    back_button_callback: None,
    back_key_trap_enabled: false,
    fullscreen_mode_active: false,
});

fn statics() -> std::sync::MutexGuard<'static, Statics> {
    STATICS.lock().unwrap_or_else(|e| e.into_inner())
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
fn call_super<'a>(
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
    // (mTextEdit, mLayout, mClipboardHandler, mCursors and mLastCursorID
    // come with the video driver)
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
/// Translation of `onCreate()`. Note: the activity's `commandHandler`,
/// which upstream's instance initializer makes, is made here first (on
/// the same thread: the constructor has no code of its own here).
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
        sdl::initialize();

        // So we can call stuff from static callbacks
        statics().singleton = Global::new(env, this);
        sdl::set_context(sdl::global(env, this));

        // (SDL.isControllerManagerReady():
        // SDLControllerManager.initializeDeviceListener(), the clipboard
        // handler when video is compiled, and the HID device manager when
        // nativeIsHIDAPIEnabled(): those managers come with their drivers)

        // Set up the surface
        // (SDL.isSubsystemInitialized(SDL.SDL_INIT_VIDEO): the surface,
        // createSDLSurface(), in its layout, a RelativeLayout, and the
        // orientation passed down, nativeSetNaturalOrientation() and
        // onNativeRotationChanged(): with SDLSurface, so mSurface stays
        // null)

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

        // (mLayout != null: setContentView(mLayout) and
        // setWindowStyle(false), with the layout; and the decor view's
        // setOnSystemUiVisibilityChangeListener(this), with the system
        // UI's visibility: both with the surface)

        // Get filename from "Open with" of another application
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
            // (getMotionListener().reclaimRelativeMouseModeIfNeeded(): the
            // motion listener comes with the mouse)

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

        // (HIDDeviceManager.release(mHIDDeviceManager), see on_pause(),
        // and SDLAudioManager.release(this), with the audio manager)

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
#[allow(dead_code)] // (the keyboard's input connection asks)
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
                // (mTextEdit != null: the text edit, with the keyboard;
                // mTextEdit is null without it)
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
#[allow(dead_code)] // (the video driver calls it)
pub(crate) fn set_activity_title<'a>(env: Env<'a>, title: &str) -> JniResult<'a, bool> {
    // Called from SDLMain() thread and can't directly affect the view
    let singleton = singleton_or_throw(env)?;
    let title = env.new_string(title)?;
    send_command(env, singleton.obj(), COMMAND_CHANGE_TITLE, title.obj())
}

/// This method is called by SDL using JNI: fullscreen or not (from the
/// SDLMain thread, on the main one). Translation of `setWindowStyle()`.
#[allow(dead_code)] // (the video driver calls it)
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
