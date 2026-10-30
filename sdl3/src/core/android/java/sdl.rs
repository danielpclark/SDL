// Rust translation of android-project/app/src/main/java/org/libsdl/app/
// SDL.java from Simple DirectMedia Layer (part).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `class SDL` (SDL library initialization): the subsystems the app's Java sets up, and the context
//! (the current activity). Its library loading is `SDLActivity`'s static
//! initializer's (see `super::super::classes`).

use super::super::env::{Env, Global, Obj};
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

/// `mInitializedSubsystems`.
static INITIALIZED_SUBSYSTEMS: AtomicI32 = AtomicI32::new(SDL_INIT_EVERYTHING);
/// `mCompiledSubsystems`.
static COMPILED_SUBSYSTEMS: AtomicI32 = AtomicI32::new(SDL_INIT_EVERYTHING);

/// `mContext`.
static CONTEXT: Mutex<Option<Global>> = Mutex::new(None);

/// Translation of `setupJNI(int subsystems)`. (The audio and controller
/// managers' `nativeSetupJNI()` come with those classes.)
pub(crate) fn setup_jni(env: Env<'_>, subsystems: i32) {
    if let Some(class) = super::super::env::app_class(super::super::classes::SDL_ACTIVITY) {
        super::super::setup_jni(env, class.obj());
    }

    let compiled = super::super::get_compiled_subsystems();
    COMPILED_SUBSYSTEMS.store(compiled, Ordering::Release);
    INITIALIZED_SUBSYSTEMS.store(subsystems & compiled, Ordering::Release);
}

/// Translation of `initialize(int subsystems)` (`initialize()`). (The
/// audio and controller managers' `initialize()` come with them.)
pub(crate) fn initialize() {
    set_context(None);

    activity::initialize();
}

/// Translation of `isSubsystemInitialized()`.
pub(crate) fn is_subsystem_initialized(subsystem: i32) -> bool {
    (INITIALIZED_SUBSYSTEMS.load(Ordering::Acquire) & subsystem) != 0
}

/// Translation of `isSubsystemCompiled()`.
pub(crate) fn is_subsystem_compiled(subsystem: i32) -> bool {
    (COMPILED_SUBSYSTEMS.load(Ordering::Acquire) & subsystem) != 0
}

/// This function stores the current activity (SDL or not). Translation of
/// `setContext()`. (The audio manager's context comes with it.)
pub(crate) fn set_context(context: Option<Global>) {
    *CONTEXT.lock().unwrap_or_else(|e| e.into_inner()) = context;
}

/// The current activity, as a local reference of `env`. Translation of
/// `getContext()`.
pub(crate) fn get_context<'a>(env: Env<'a>) -> Option<super::super::env::Local<'a>> {
    let context = CONTEXT.lock().unwrap_or_else(|e| e.into_inner());
    context.as_ref().and_then(|c| env.new_local_ref(c.obj()))
}

/// Whether there's a context (`getContext() == null`).
pub(crate) fn has_context() -> bool {
    CONTEXT.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// A global reference to `obj`, for [`set_context`].
pub(crate) fn global(env: Env<'_>, obj: Obj<'_>) -> Option<Global> {
    Global::new(env, obj)
}
