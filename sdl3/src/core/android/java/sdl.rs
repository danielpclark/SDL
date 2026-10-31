// Rust translation of android-project/app/src/main/java/org/libsdl/app/
// SDL.java from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `class SDL` (SDL library initialization): the subsystems the app's Java
//! sets up, the context (the current activity), and the libraries'
//! loading (through ReLinker when the app has it), with the audio manager
//! (`SDLAudioManager`), which `setupJNI()`, `initialize()` and
//! `setContext()` set up too. The controller manager
//! (`SDLControllerManager`) comes with the joystick drivers.

use super::super::env::{self, Arg, Env, Global, JniResult, Local, Obj};
use super::activity;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;

// SDL_INIT_* values, mirrored so embedders can request a subset and skip stub classes for unused subsystems.
pub(crate) const SDL_INIT_AUDIO: i32 = 0x00000010;
pub(crate) const SDL_INIT_VIDEO: i32 = 0x00000020;
pub(crate) const SDL_INIT_JOYSTICK: i32 = 0x00000200;
pub(crate) const SDL_INIT_HAPTIC: i32 = 0x00001000;
pub(crate) const SDL_INIT_GAMEPAD: i32 = 0x00002000;
pub(crate) const SDL_INIT_SENSOR: i32 = 0x00008000;
pub(crate) const SDL_INIT_CAMERA: i32 = 0x00010000;

pub(crate) const SDL_INIT_EVERYTHING: i32 = SDL_INIT_AUDIO
    | SDL_INIT_VIDEO
    | SDL_INIT_JOYSTICK
    | SDL_INIT_HAPTIC
    | SDL_INIT_GAMEPAD
    | SDL_INIT_SENSOR
    | SDL_INIT_CAMERA;

// SDLControllerManager backs all three of these, so it is set up when any are present.
const SDL_INIT_CONTROLLER: i32 = SDL_INIT_JOYSTICK | SDL_INIT_GAMEPAD | SDL_INIT_HAPTIC;

/// `mInitializedSubsystems`.
static INITIALIZED_SUBSYSTEMS: AtomicI32 = AtomicI32::new(SDL_INIT_EVERYTHING);
/// `mCompiledSubsystems`.
static COMPILED_SUBSYSTEMS: AtomicI32 = AtomicI32::new(SDL_INIT_EVERYTHING);

/// `mContext`.
static CONTEXT: Mutex<Option<Global>> = Mutex::new(None);

/// Translation of `setupJNI(int subsystems)`.
pub(crate) fn setup_jni(env: Env<'_>, subsystems: i32) {
    if let Some(class) = env::app_class(super::super::classes::SDL_ACTIVITY) {
        super::super::setup_jni(env, class.obj());
    }

    let compiled = super::super::get_compiled_subsystems();
    COMPILED_SUBSYSTEMS.store(compiled, Ordering::Release);
    INITIALIZED_SUBSYSTEMS.store(subsystems & compiled, Ordering::Release);

    if is_subsystem_compiled(SDL_INIT_AUDIO) {
        if let Some(class) = env::app_class(super::super::classes::SDL_AUDIO_MANAGER) {
            super::super::audio_setup_jni(env, class.obj());
        }
    }

    // (isSubsystemCompiled(SDL_INIT_CONTROLLER):
    // SDLControllerManager.nativeSetupJNI(), comes with that manager)
}

/// Translation of `initialize()`, `initialize(mInitializedSubsystems)`.
pub(crate) fn initialize<'a>(env: Env<'a>) -> JniResult<'a, ()> {
    set_context(env, None);

    activity::initialize();

    if is_subsystem_compiled(SDL_INIT_AUDIO) {
        super::audio_manager::initialize(env)?;
    }

    // (isSubsystemCompiled(SDL_INIT_CONTROLLER):
    // SDLControllerManager.initialize(): see setup_jni())
    Ok(())
}

/// Translation of `isSubsystemInitialized()`.
pub(crate) fn is_subsystem_initialized(subsystem: i32) -> bool {
    (INITIALIZED_SUBSYSTEMS.load(Ordering::Acquire) & subsystem) != 0
}

/// Translation of `isSubsystemCompiled()`.
pub(crate) fn is_subsystem_compiled(subsystem: i32) -> bool {
    (COMPILED_SUBSYSTEMS.load(Ordering::Acquire) & subsystem) != 0
}

/// Translation of `isControllerManagerReady()`.
pub(crate) fn is_controller_manager_ready() -> bool {
    is_subsystem_initialized(SDL_INIT_CONTROLLER)
}

/// This function stores the current activity (SDL or not). Translation of
/// `setContext()`.
pub(crate) fn set_context(env: Env<'_>, context: Option<Obj<'_>>) {
    if is_subsystem_compiled(SDL_INIT_AUDIO) {
        super::audio_manager::set_context(env, context);
    }
    *CONTEXT.lock().unwrap_or_else(|e| e.into_inner()) = context.and_then(|c| Global::new(env, c));
}

/// The current activity, as a local reference of `env`. Translation of
/// `getContext()`.
pub(crate) fn get_context<'a>(env: Env<'a>) -> Option<Local<'a>> {
    let context = CONTEXT.lock().unwrap_or_else(|e| e.into_inner());
    context.as_ref().and_then(|c| env.new_local_ref(c.obj()))
}

/// Whether there's a context (`getContext() == null`).
pub(crate) fn has_context() -> bool {
    CONTEXT.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// Load the library `libraryName` (`lib<libraryName>.so`) of `context`'s
/// app: through ReLinker (`com.getkeepsafe.relinker.ReLinker`, found by
/// reflection) when the app has it, else `System.loadLibrary()`; the
/// `UnsatisfiedLinkError` (or `SecurityException`) thrown when it can't be
/// loaded. Translation of `loadLibrary(String libraryName, Context
/// context)`.
pub(crate) fn load_library<'a>(
    env: Env<'a>,
    library_name: &str,
    context: Obj<'_>,
) -> JniResult<'a, ()> {
    // (libraryName == null: a NullPointerException; there's a name here)

    // Let's see if we have ReLinker available in the project.  This is necessary for
    // some projects that have huge numbers of local libraries bundled, and thus may
    // trip a bug in Android's native library loader which ReLinker works around.  (If
    // loadLibrary works properly, ReLinker will simply use the normal Android method
    // internally.)
    //
    // To use ReLinker, just add it as a dependency.  For more information, see
    // https://github.com/KeepSafe/ReLinker for ReLinker's repository.
    //
    // (in a local frame: a Java method's references end with it)
    let relinked = env.with_local_frame(16, |env| match relink(env, library_name, context) {
        Ok(()) => true,
        // catch (final Throwable e)
        Err(thrown) => {
            drop(thrown.catch());
            false
        }
    });
    match relinked {
        Ok(true) => Ok(()),
        relinked => {
            // Fall back
            if let Err(thrown) = relinked {
                drop(thrown.catch());
            }
            let system = env.find_class(c"java/lang/System")?;
            let name = env.new_string(library_name)?;
            env.call_static_method(
                system.obj(),
                c"loadLibrary",
                c"(Ljava/lang/String;)V",
                &[Arg::Object(name.obj())],
            )?;
            Ok(())
        }
    }
}

/// The `try` block of [`load_library`]: ReLinker's `force()` instance's
/// `loadLibrary(context, libraryName, null, null)`.
fn relink<'a>(env: Env<'a>, library_name: &str, context: Obj<'_>) -> JniResult<'a, ()> {
    let loader = env
        .call_method(
            context,
            c"getClassLoader",
            c"()Ljava/lang/ClassLoader;",
            &[],
        )?
        .object()
        .expect("a class loader");
    let load_class = |name: &str| -> JniResult<'a, Local<'a>> {
        let name = env.new_string(name)?;
        Ok(env
            .call_method(
                loader.obj(),
                c"loadClass",
                c"(Ljava/lang/String;)Ljava/lang/Class;",
                &[Arg::Object(name.obj())],
            )?
            .object()
            .expect("a class"))
    };
    let relink_class = load_class("com.getkeepsafe.relinker.ReLinker")?;
    let relink_listener_class = load_class("com.getkeepsafe.relinker.ReLinker$LoadListener")?;
    let context_class = load_class("android.content.Context")?;
    let string_class = load_class("java.lang.String")?;
    let class_class = env.find_class(c"java/lang/Class")?;
    let object_class = env.find_class(c"java/lang/Object")?;

    // Get a 'force' instance of the ReLinker, so we can ensure libraries are reinstalled if
    // they've changed during updates.
    let no_classes = env.new_object_array(0, class_class.obj(), Obj::NULL)?;
    let force = env.new_string("force")?;
    let force_method = env
        .call_method(
            relink_class.obj(),
            c"getDeclaredMethod",
            c"(Ljava/lang/String;[Ljava/lang/Class;)Ljava/lang/reflect/Method;",
            &[Arg::Object(force.obj()), Arg::Object(no_classes.obj())],
        )?
        .object()
        .expect("a method");
    let no_args = env.new_object_array(0, object_class.obj(), Obj::NULL)?;
    let relink_instance = env
        .call_method(
            force_method.obj(),
            c"invoke",
            c"(Ljava/lang/Object;[Ljava/lang/Object;)Ljava/lang/Object;",
            &[Arg::Object(Obj::NULL), Arg::Object(no_args.obj())],
        )?
        .object()
        .expect("a ReLinker instance");
    let relink_instance_class = env
        .get_object_class(relink_instance.obj())
        .expect("an object's class");

    // Actually load the library!
    let parameter_types = env.new_object_array(4, class_class.obj(), Obj::NULL)?;
    for (i, class) in [
        &context_class,
        &string_class,
        &string_class,
        &relink_listener_class,
    ]
    .into_iter()
    .enumerate()
    {
        env.set_object_array_element(parameter_types.obj(), i as i32, class.obj())?;
    }
    let load_library = env.new_string("loadLibrary")?;
    let load_method = env
        .call_method(
            relink_instance_class.obj(),
            c"getDeclaredMethod",
            c"(Ljava/lang/String;[Ljava/lang/Class;)Ljava/lang/reflect/Method;",
            &[
                Arg::Object(load_library.obj()),
                Arg::Object(parameter_types.obj()),
            ],
        )?
        .object()
        .expect("a method");
    let name = env.new_string(library_name)?;
    let args = env.new_object_array(4, object_class.obj(), Obj::NULL)?;
    env.set_object_array_element(args.obj(), 0, context)?;
    env.set_object_array_element(args.obj(), 1, name.obj())?;
    env.call_method(
        load_method.obj(),
        c"invoke",
        c"(Ljava/lang/Object;[Ljava/lang/Object;)Ljava/lang/Object;",
        &[Arg::Object(relink_instance.obj()), Arg::Object(args.obj())],
    )?;
    Ok(())
}
