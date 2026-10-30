// The Android-specific entry points of SDL_system.h, from
// src/core/android/SDL_android.c of Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! What SDL offers an Android app of its platform: the Java VM's
//! environment of a thread and the activity (for an app's own JNI calls),
//! the device's API level and form, the back button, and messages to the
//! activity. They need the app's Java VM: in a program that isn't an app
//! (one started from the shell) there's none, and they give `None`, false
//! or nothing. (The device's form factor, TV or tablet, is
//! [`init::device_form_factor`](crate::init::device_form_factor).)

use std::ffi::c_void;
use std::ptr::NonNull;

use crate::core::android as core_android;
use crate::error::Result;

/// The Java Native Interface environment (`JNIEnv *`) of the current
/// thread, which is attached to the Java VM if it isn't yet (and detached
/// when it ends); `None` without a Java VM, or when the thread can't be
/// attached. Translation of `SDL_GetAndroidJNIEnv()`.
///
/// It is safe to call this function from any thread.
pub fn jni_env() -> Option<NonNull<c_void>> {
    core_android::get_android_jni_env()
}

/// The app's activity (a `jobject`: `SDLActivity`'s instance), a local
/// reference of the current thread's environment ([`jni_env`]), which the
/// caller must delete (JNI's `DeleteLocalRef()`, or a local frame's
/// `PopLocalFrame()`); `None` without one. Translation of
/// `SDL_GetAndroidActivity()`.
///
/// It is safe to call this function from any thread.
pub fn activity() -> Option<NonNull<c_void>> {
    core_android::get_android_activity()
}

/// The device's Android API level (35 for Android 15, 34 for Android 14,
/// ..., 21 for Android 5.0), or 0 when it can't be read. Translation of
/// `SDL_GetAndroidSDKVersion()`.
///
/// It is safe to call this function from any thread.
pub fn sdk_version() -> i32 {
    core_android::get_android_sdk_version()
}

/// Whether the app runs on a Chromebook. Translation of
/// `SDL_IsChromebook()`.
///
/// It is safe to call this function from any thread.
pub fn is_chromebook() -> bool {
    core_android::is_chromebook()
}

/// Whether the app runs on a Samsung DeX docking station. Translation of
/// `SDL_IsDeXMode()`.
///
/// It is safe to call this function from any thread.
pub fn is_dex_mode() -> bool {
    core_android::is_dex_mode()
}

/// Trigger the Android system back button behavior (on the activity's
/// main thread). Translation of `SDL_SendAndroidBackButton()`.
///
/// It is safe to call this function from any thread.
pub fn send_back_button() {
    core_android::send_android_back_button();
}

/// Send a user command to `SDLActivity`, which its command handler gets on
/// the main thread (upstream's apps override its `onUnhandledMessage()`
/// to handle it, which an app can't do here: the activity logs that it
/// didn't handle it). `command` must be 0x8000 or more; an error too when
/// the message wasn't sent (no activity yet, or its looper is quitting).
/// Translation of `SDL_SendAndroidMessage()`.
///
/// It is safe to call this function from any thread.
pub fn send_message(command: u32, param: i32) -> Result<()> {
    core_android::send_android_message(command, param)
}
