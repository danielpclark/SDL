// The Android-specific entry points of SDL_system.h, from
// src/core/android/SDL_android.c of Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! What SDL offers an Android app of its platform: the Java VM's
//! environment of a thread and the activity (for an app's own JNI calls),
//! the device's API level and form, the back button, messages to the
//! activity, the app's storage paths, runtime permissions and toasts. They
//! need the app's Java VM: in a program that isn't an app (one started
//! from the shell) there's none, and they give `None`, false, nothing or
//! an error. (The device's form factor, TV or tablet, is
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

/// [`external_storage_state`]'s flag: external storage can be read.
/// `SDL_ANDROID_EXTERNAL_STORAGE_READ`. See the official Android developer
/// guide for more information:
/// <http://developer.android.com/guide/topics/data/data-storage.html>
pub const EXTERNAL_STORAGE_READ: u32 = core_android::ANDROID_EXTERNAL_STORAGE_READ;

/// [`external_storage_state`]'s flag: external storage can be written.
/// `SDL_ANDROID_EXTERNAL_STORAGE_WRITE`.
pub const EXTERNAL_STORAGE_WRITE: u32 = core_android::ANDROID_EXTERNAL_STORAGE_WRITE;

/// The path used for internal storage for this Android application, which
/// is unique to your application and cannot be written to by other
/// applications: typically `/data/data/your.app.package/files`
/// (`android.content.Context.getFilesDir()`). Translation of
/// `SDL_GetAndroidInternalStoragePath()`.
pub fn internal_storage_path() -> Result<String> {
    core_android::get_android_internal_storage_path()
}

/// The current state of external storage for this Android application: a
/// bitmask of [`EXTERNAL_STORAGE_READ`] and [`EXTERNAL_STORAGE_WRITE`], 0
/// when external storage is currently unavailable. Translation of
/// `SDL_GetAndroidExternalStorageState()`.
pub fn external_storage_state() -> u32 {
    core_android::get_android_external_storage_state()
}

/// The path used for external storage for this Android application, which
/// is unique to your application, but is public and can be written to by
/// other applications: typically
/// `/storage/sdcard0/Android/data/your.app.package/files`
/// (`android.content.Context.getExternalFilesDir()`). Translation of
/// `SDL_GetAndroidExternalStoragePath()`.
pub fn external_storage_path() -> Result<String> {
    core_android::get_android_external_storage_path()
}

/// The path used for caching data for this Android application, which is
/// unique to your application, but is public and can be written to by other
/// applications: typically `/data/data/your.app.package/cache/`
/// (`android.content.Context.getCacheDir()`). Translation of
/// `SDL_GetAndroidCachePath()`.
pub fn cache_path() -> Result<String> {
    core_android::get_android_cache_path()
}

/// Request a permission at runtime, asynchronously: `callback` gets the
/// permission (an Android permission's name, from
/// <https://developer.android.com/reference/android/Manifest.permission>)
/// and whether it is granted, once a decision has been made. Translation of
/// `SDL_RequestAndroidPermission()`.
///
/// You do not need to call this for built-in functionality of SDL;
/// recording from a microphone or reading images from a camera, using
/// standard SDL APIs, will manage permission requests for you.
///
/// This function never blocks. The callback may be called on a different
/// thread (the activity's main thread), and possibly much later, as it
/// might wait on a user to respond to a system dialog. If permission has
/// already been granted for a specific entitlement, the callback will still
/// fire, on the current thread and before this function returns. The result
/// of the request is only ever reported through the callback; an error
/// means the request wasn't submitted (no app).
///
/// It is safe to call this function from any thread.
pub fn request_permission(
    permission: &str,
    callback: impl FnOnce(&str, bool) + Send + 'static,
) -> Result<()> {
    core_android::request_android_permission(permission, Box::new(callback))
}

/// Show an Android toast notification (a lightweight notification unique to
/// Android: <https://developer.android.com/guide/topics/ui/notifiers/toasts>),
/// on the UI thread: `message` for `duration` (0: short, 1: long), at
/// `gravity` (a value of
/// <https://developer.android.com/reference/android/view/Gravity>, or -1 for
/// no preference) offset by `xoffset` and `yoffset` (used when `gravity` is
/// 0 or more). Translation of `SDL_ShowAndroidToast()`.
///
/// It is safe to call this function from any thread.
pub fn show_toast(
    message: &str,
    duration: i32,
    gravity: i32,
    xoffset: i32,
    yoffset: i32,
) -> Result<()> {
    core_android::show_android_toast(message, duration, gravity, xoffset, yoffset)
}
