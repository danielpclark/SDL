// The Android NDK's C libraries the Android layer calls, declared by hand.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! What upstream includes from the NDK's headers (layout facts of the
//! NDK's Apache-2.0 headers: see docs/LICENSING.md), linked as upstream
//! links them (`-llog`, ...): `<android/log.h>`'s log so far.

use std::ffi::{c_char, c_int, CString};

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
