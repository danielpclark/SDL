// The Android NDK's C libraries the Android layer calls, declared by hand.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! What upstream includes from the NDK's headers (layout facts of the
//! NDK's Apache-2.0 headers: see docs/LICENSING.md), linked as upstream
//! links them (`-llog`, `-landroid`, ...): `<android/log.h>`'s log, and
//! `<android/asset_manager.h>`'s assets (the app package's files).

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
