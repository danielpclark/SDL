// Rust translation of android-project/app/src/main/java/org/libsdl/app/
// SDLActivity.java from Simple DirectMedia Layer (part).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see
// LICENSE.txt.

//! `public class SDLActivity extends Activity` (the app's activity, which
//! the manifest names), and `class SDLMain implements Runnable` (the SDL
//! thread's code): the activity's lifecycle and the native state machine
//! (`handleNativeState()`), which starts the "SDLThread" thread, which
//! runs the app's `SDL_main` (`main()`, `nativeRunMain()`), and finishes
//! the activity when it returns; and the manifest's environment variables
//! (`getManifestEnvironmentVariables()`).
//!
//! Not translated yet, and so skipped where upstream does them: the
//! activity's layout (`SDLSurface`, `SDLDummyEdit`), the controller, HID,
//! clipboard and audio managers, the motion listener, the natives that
//! pass the lifecycle to SDL (`nativePause()`, `nativeResume()`,
//! `nativeFocusChanged()`, `nativeLowMemory()`, `nativeSendQuit()`,
//! `nativeQuit()`), the locale and dark mode, the command handler, and
//! what the activity offers SDL's C side (text input, cursors, dialogs,
//! permissions, ...). They come with the backends that use them. With no
//! surface, the state machine starts the SDL thread as soon as the
//! activity is resumed, as upstream's does without one.

use super::super::env::{self, Arg, Env, Global, JniResult, Local, Obj};
use super::super::jni::*;
use super::super::ndk::LogPriority;
use super::sdl;
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
    next_native_state: NativeState,
    current_native_state: NativeState,
    /// If shared libraries (e.g. SDL or the native application) could not
    /// be loaded.
    broken_libraries: bool,
    // Main components
    singleton: Option<Global>,
    // This is what SDL runs in. It invokes SDL_main(), eventually
    sdl_thread: Option<Global>,
    sdl_main_finished: bool,
    activity_created: bool,
    is_resumed_called: bool,
    has_focus: bool,
}

static STATICS: Mutex<Statics> = Mutex::new(Statics {
    next_native_state: NativeState::Init,
    current_native_state: NativeState::Init,
    broken_libraries: true,
    singleton: None,
    sdl_thread: None,
    sdl_main_finished: false,
    activity_created: false,
    is_resumed_called: false,
    has_focus: false,
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

/// A static `String` field of `android.os.Build` (`Build.MANUFACTURER`).
fn build_string(env: Env<'_>, field: &CStr) -> String {
    let value = env
        .find_class(c"android/os/Build")
        .and_then(|build| env.get_static_field(build.obj(), field, c"Ljava/lang/String;"));
    match value {
        Ok(value) => value
            .object()
            .and_then(|s| env.get_string(s.obj()).ok())
            .unwrap_or_else(|| "null".to_string()),
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
    s.sdl_thread = None;
    s.is_resumed_called = false;
    s.has_focus = true;
    s.next_native_state = NativeState::Init;
    s.current_native_state = NativeState::Init;
}

// Setup
/// Translation of `onCreate()`.
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
        // (loadLibraries(): the class's static initializer loaded them,
        // before the framework could call this method)
        let mut error_msg_broken_lib = String::new();
        statics().broken_libraries = false; /* success */

        let expected_version =
            format!("{SDL_MAJOR_VERSION}.{SDL_MINOR_VERSION}.{SDL_MICRO_VERSION}");
        let version = super::super::get_version(env)
            .and_then(|v| env.get_string(v.obj()).ok())
            .unwrap_or_default();
        if version != expected_version {
            statics().broken_libraries = true;
            error_msg_broken_lib =
                format!("SDL C/Java version mismatch (expected {expected_version}, got {version})");
        }

        if statics().broken_libraries {
            statics().singleton = Global::new(env, this);
            // (the AlertDialog "An error occurred while trying to start
            // the application. ..." comes with the message boxes)
            log(
                LogPriority::Error,
                &format!("Error: {error_msg_broken_lib}"),
            );
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

        // (the controller manager's device listener, the clipboard
        // handler, the HID device manager, the surface and its layout,
        // the orientation, the locale, the dark mode, the content view,
        // the system UI listener and the "Open with" file aren't
        // translated yet)
        Ok(())
    });
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

        // (the HID device manager isn't translated yet)

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

        // (the HID device manager isn't translated yet)

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

        // (the HID device manager isn't translated yet)

        let mut s = statics();
        if s.broken_libraries {
            return Ok(());
        }

        s.has_focus = has_focus;
        if has_focus {
            s.next_native_state = NativeState::Resumed;
            // (the motion listener's relative mouse mode isn't
            // translated yet)

            drop(s);
            handle_native_state(env)?;
            // (nativeFocusChanged(true) comes with the lifecycle events)
        } else {
            // (nativeFocusChanged(false) comes with the lifecycle events)
            drop(s);
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

        // (nativeLowMemory() comes with the lifecycle events)
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

        // (the locale and the dark mode come with the lifecycle events)
        Ok(())
    });
}

/// Translation of `onDestroy()`.
pub(crate) extern "system" fn on_destroy(env: *mut JNIEnv, this: jobject) {
    // SAFETY: a native method's environment and arguments.
    let (env, this) = unsafe { (Env::from_raw(env), Obj::from_raw(this)) };
    native(env, |env| {
        log(LogPriority::Verbose, "onDestroy()");

        // (the HID device manager and the audio manager aren't translated
        // yet)

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
            // (nativeSendQuit() comes with the lifecycle events)

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

        // (nativeQuit() comes with the lifecycle events)

        call_super(env, this, c"onDestroy", c"()V", &[])?;
        Ok(())
    });
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

    // Try a transition to paused state
    if s.next_native_state == NativeState::Paused {
        // (nativePause() comes with the lifecycle events, and the
        // surface's handlePause() with the surface)
        s.current_native_state = s.next_native_state;
        return Ok(());
    }

    // Try a transition to resumed state
    if s.next_native_state == NativeState::Resumed {
        // (mSurface == null: there's no surface yet)
        let ready_to_run = s.is_resumed_called;
        if ready_to_run {
            if s.sdl_thread.is_none() {
                // This is the entry point to the C app.
                // Start up the C app thread and enable sensor input for the first time
                // FIXME: Why aren't we enabling sensor input at start?

                let thread = new_sdl_thread(env)?;
                s.sdl_thread = Global::new(env, thread.obj());
                drop(s);
                env.call_method(thread.obj(), c"start", c"()V", &[])?;
                s = statics();

                // No nativeResume(), don't signal Android_ResumeSem
            } else {
                // (nativeResume() comes with the lifecycle events)
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

/// The arguments after the application name. Translation of
/// `getArguments()`: none (`new String[0]`).
fn get_arguments(env: Env<'_>) -> JniResult<'_, Local<'_>> {
    let string = env.find_class(c"java/lang/String")?;
    env.new_object_array(0, string.obj(), Obj::NULL)
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
                let npe = env.find_class(c"java/lang/NullPointerException")?;
                env.throw_new(
                    npe.obj(),
                    c"Attempt to invoke virtual method 'java.lang.String java.lang.Object.toString()' on a null object reference",
                );
                env.pending()?;
                return Ok(false);
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
