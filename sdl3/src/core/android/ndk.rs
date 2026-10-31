// The Android NDK's C libraries the Android layer calls, declared by hand.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! What upstream includes from the NDK's headers (layout facts of the
//! NDK's Apache-2.0 headers: see docs/LICENSING.md), linked as upstream
//! links them (`-llog`, `-landroid`, ...): `<android/log.h>`'s log,
//! `<android/asset_manager.h>`'s assets (the app package's files),
//! `<android/native_window.h>`'s windows (the activity's surface's), and
//! `<android/looper.h>`'s loopers and `<android/sensor.h>`'s sensors (the
//! sensor driver's).

use std::ffi::{c_char, c_int, c_void, CString};

/// `android_LogPriority`.
#[allow(dead_code)]
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LogPriority {
    Unknown = 0,
    Default = 1,
    Verbose = 2,
    Debug = 3,
    Info = 4,
    Warn = 5,
    Error = 6,
    Fatal = 7,
    Silent = 8,
}

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

/// `__android_log_write()` (a NUL in the text ends it there, as in C).
pub(crate) fn log_write(priority: LogPriority, tag: &str, text: &str) {
    let cut = |s: &str| CString::new(s.split('\0').next().unwrap_or("")).unwrap_or_default();
    let (tag, text) = (cut(tag), cut(text));
    // SAFETY: C strings.
    unsafe { __android_log_write(priority as c_int, tag.as_ptr(), text.as_ptr()) };
}

/// `AAssetManager` (`<android/asset_manager.h>`): opaque.
#[repr(C)]
pub(crate) struct AAssetManager {
    _private: [u8; 0],
}

/// `AAsset`: opaque.
#[repr(C)]
pub(crate) struct AAsset {
    _private: [u8; 0],
}

/// `AASSET_MODE_UNKNOWN`: no specific information about how data will be
/// accessed.
pub(crate) const AASSET_MODE_UNKNOWN: c_int = 0;

#[link(name = "android")]
unsafe extern "C" {
    /// `AAssetManager_fromJava()` (`<android/asset_manager_jni.h>`).
    pub(crate) fn AAssetManager_fromJava(
        env: *mut super::jni::JNIEnv,
        asset_manager: super::jni::jobject,
    ) -> *mut AAssetManager;
    /// `AAssetManager_open()`.
    pub(crate) fn AAssetManager_open(
        mgr: *mut AAssetManager,
        filename: *const c_char,
        mode: c_int,
    ) -> *mut AAsset;
    /// `AAsset_read()`.
    pub(crate) fn AAsset_read(asset: *mut AAsset, buf: *mut c_void, count: usize) -> c_int;
    /// `AAsset_seek64()` (API 13).
    pub(crate) fn AAsset_seek64(asset: *mut AAsset, offset: i64, whence: c_int) -> i64;
    /// `AAsset_close()`.
    pub(crate) fn AAsset_close(asset: *mut AAsset);
    /// `AAsset_getLength64()` (API 13).
    pub(crate) fn AAsset_getLength64(asset: *mut AAsset) -> i64;
}

/// `ANativeWindow` (`<android/native_window.h>`): opaque.
#[repr(C)]
pub(crate) struct ANativeWindow {
    _private: [u8; 0],
}

#[link(name = "android")]
unsafe extern "C" {
    /// `ANativeWindow_fromSurface()` (`<android/native_window_jni.h>`):
    /// the surface's window, with a reference the caller releases.
    pub(crate) fn ANativeWindow_fromSurface(
        env: *mut super::jni::JNIEnv,
        surface: super::jni::jobject,
    ) -> *mut ANativeWindow;
    /// `ANativeWindow_release()`.
    pub(crate) fn ANativeWindow_release(window: *mut ANativeWindow);
    /// `ANativeWindow_getWidth()`.
    pub(crate) fn ANativeWindow_getWidth(window: *mut ANativeWindow) -> i32;
    /// `ANativeWindow_getHeight()`.
    pub(crate) fn ANativeWindow_getHeight(window: *mut ANativeWindow) -> i32;
    /// `ANativeWindow_getFormat()`.
    pub(crate) fn ANativeWindow_getFormat(window: *mut ANativeWindow) -> i32;
    /// `ANativeWindow_setBuffersGeometry()`.
    pub(crate) fn ANativeWindow_setBuffersGeometry(
        window: *mut ANativeWindow,
        width: i32,
        height: i32,
        format: i32,
    ) -> i32;
}

/// `ALooper` (`<android/looper.h>`): opaque.
#[repr(C)]
pub(crate) struct ALooper {
    _private: [u8; 0],
}

/// `ALOOPER_PREPARE_ALLOW_NON_CALLBACKS`: the looper's `pollOnce()` may
/// return the identifier of a file descriptor added without a callback.
pub(crate) const ALOOPER_PREPARE_ALLOW_NON_CALLBACKS: c_int = 1 << 0;

/// `ALooper_callbackFunc`.
pub(crate) type ALooperCallbackFunc =
    Option<unsafe extern "C" fn(fd: c_int, events: c_int, data: *mut c_void) -> c_int>;

#[link(name = "android")]
unsafe extern "C" {
    /// `ALooper_prepare()`: the calling thread's looper, made if it has
    /// none.
    pub(crate) fn ALooper_prepare(opts: c_int) -> *mut ALooper;
    /// `ALooper_pollOnce()`.
    pub(crate) fn ALooper_pollOnce(
        timeout_millis: c_int,
        out_fd: *mut c_int,
        out_events: *mut c_int,
        out_data: *mut *mut c_void,
    ) -> c_int;
    /// `ALooper_wake()`.
    pub(crate) fn ALooper_wake(looper: *mut ALooper);
}

/// `ASensorManager` (`<android/sensor.h>`): opaque.
#[repr(C)]
pub(crate) struct ASensorManager {
    _private: [u8; 0],
}

/// `ASensor`: opaque.
#[repr(C)]
pub(crate) struct ASensor {
    _private: [u8; 0],
}

/// `ASensorEventQueue`: opaque.
#[repr(C)]
pub(crate) struct ASensorEventQueue {
    _private: [u8; 0],
}

/// `ASensorRef`.
pub(crate) type ASensorRef = *const ASensor;

/// `ASensorList`.
pub(crate) type ASensorList = *const ASensorRef;

/// `ASensorEvent` (104 bytes), its union after the timestamp read as
/// `float data[16]`, the member SDL reads.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ASensorEvent {
    /// `sizeof(struct ASensorEvent)`
    pub(crate) version: i32,
    pub(crate) sensor: i32,
    pub(crate) type_: i32,
    pub(crate) reserved0: i32,
    /// time in nanoseconds
    pub(crate) timestamp: i64,
    pub(crate) data: [f32; 16],
    pub(crate) flags: u32,
    pub(crate) reserved1: [i32; 3],
}

#[link(name = "android")]
unsafe extern "C" {
    /// `ASensorManager_getInstance()` (deprecated in API 26 for
    /// `ASensorManager_getInstanceForPackage()`, which upstream doesn't
    /// use).
    pub(crate) fn ASensorManager_getInstance() -> *mut ASensorManager;
    /// `ASensorManager_getSensorList()`: the number of sensors.
    pub(crate) fn ASensorManager_getSensorList(
        manager: *mut ASensorManager,
        list: *mut ASensorList,
    ) -> c_int;
    /// `ASensorManager_createEventQueue()`.
    pub(crate) fn ASensorManager_createEventQueue(
        manager: *mut ASensorManager,
        looper: *mut ALooper,
        ident: c_int,
        callback: ALooperCallbackFunc,
        data: *mut c_void,
    ) -> *mut ASensorEventQueue;
    /// `ASensorManager_destroyEventQueue()`.
    pub(crate) fn ASensorManager_destroyEventQueue(
        manager: *mut ASensorManager,
        queue: *mut ASensorEventQueue,
    ) -> c_int;
    /// `ASensorEventQueue_enableSensor()`.
    pub(crate) fn ASensorEventQueue_enableSensor(
        queue: *mut ASensorEventQueue,
        sensor: ASensorRef,
    ) -> c_int;
    /// `ASensorEventQueue_disableSensor()`.
    pub(crate) fn ASensorEventQueue_disableSensor(
        queue: *mut ASensorEventQueue,
        sensor: ASensorRef,
    ) -> c_int;
    /// `ASensorEventQueue_setEventRate()`.
    pub(crate) fn ASensorEventQueue_setEventRate(
        queue: *mut ASensorEventQueue,
        sensor: ASensorRef,
        usec: i32,
    ) -> c_int;
    /// `ASensorEventQueue_getEvents()`: the number of events read, or a
    /// negative error.
    pub(crate) fn ASensorEventQueue_getEvents(
        queue: *mut ASensorEventQueue,
        events: *mut ASensorEvent,
        count: usize,
    ) -> isize;
    /// `ASensor_getName()`.
    pub(crate) fn ASensor_getName(sensor: ASensorRef) -> *const c_char;
    /// `ASensor_getType()`.
    pub(crate) fn ASensor_getType(sensor: ASensorRef) -> c_int;
    /// `ASensor_getMinDelay()`: the minimum delay between events in
    /// microseconds, 0 for a sensor that reports on changes only.
    pub(crate) fn ASensor_getMinDelay(sensor: ASensorRef) -> c_int;
}
