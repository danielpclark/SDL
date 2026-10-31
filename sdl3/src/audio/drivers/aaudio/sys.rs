// The AAudio declarations the AAudio driver uses, from the NDK's
// <aaudio/AAudio.h> (API 26 and later, the functions SDL_aaudiofuncs.h
// lists), loaded at run time from libaaudio.so as upstream does.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

#![allow(non_camel_case_types, non_snake_case)]

use std::ffi::{c_char, c_void};

use crate::error::{Error, Result};
use crate::loadso::SharedObject;

/// `AAudioStream` (opaque).
#[repr(C)]
pub(super) struct AAudioStream {
    _private: [u8; 0],
}

/// `AAudioStreamBuilder` (opaque).
#[repr(C)]
pub(super) struct AAudioStreamBuilder {
    _private: [u8; 0],
}

pub(super) type aaudio_result_t = i32;
pub(super) type aaudio_stream_state_t = i32;
pub(super) type aaudio_direction_t = i32;
pub(super) type aaudio_format_t = i32;
pub(super) type aaudio_performance_mode_t = i32;
pub(super) type aaudio_usage_t = i32;
pub(super) type aaudio_input_preset_t = i32;
pub(super) type aaudio_data_callback_result_t = i32;
/// Bionic's `clockid_t`.
pub(super) type clockid_t = i32;

/// `AAudioStream_dataCallback`.
pub(super) type AAudioStream_dataCallback = unsafe extern "C" fn(
    stream: *mut AAudioStream,
    user_data: *mut c_void,
    audio_data: *mut c_void,
    num_frames: i32,
) -> aaudio_data_callback_result_t;

/// `AAudioStream_errorCallback`.
pub(super) type AAudioStream_errorCallback =
    unsafe extern "C" fn(stream: *mut AAudioStream, user_data: *mut c_void, error: aaudio_result_t);

pub(super) const AAUDIO_OK: aaudio_result_t = 0;
pub(super) const AAUDIO_UNSPECIFIED: i32 = 0;

pub(super) const AAUDIO_DIRECTION_OUTPUT: aaudio_direction_t = 0;
pub(super) const AAUDIO_DIRECTION_INPUT: aaudio_direction_t = 1;

pub(super) const AAUDIO_FORMAT_PCM_I16: aaudio_format_t = 1;
pub(super) const AAUDIO_FORMAT_PCM_FLOAT: aaudio_format_t = 2;
// (`#if __ANDROID_API__ < 31`, as upstream defines it)
pub(super) const AAUDIO_FORMAT_PCM_I32: aaudio_format_t = 4;

pub(super) const AAUDIO_CALLBACK_RESULT_CONTINUE: aaudio_data_callback_result_t = 0;

pub(super) const AAUDIO_PERFORMANCE_MODE_LOW_LATENCY: aaudio_performance_mode_t = 12;

pub(super) const AAUDIO_USAGE_MEDIA: aaudio_usage_t = 1;
pub(super) const AAUDIO_USAGE_VOICE_COMMUNICATION: aaudio_usage_t = 2;
pub(super) const AAUDIO_USAGE_GAME: aaudio_usage_t = 14;

macro_rules! aaudio_procs {
    (
        required { $($name:ident: fn($($arg:ty),*) $(-> $ret:ty)?;)* }
        optional { $($opt:ident: fn($($oarg:ty),*) $(-> $oret:ty)?;)* }
    ) => {
        /// `AAUDIO_Data` (`ctx`): libaaudio and its functions
        /// (SDL_aaudiofuncs.h's `SDL_PROC`s, then its `SDL_PROC_OPTIONAL`s,
        /// which the system may lack; not its `SDL_PROC_UNUSED`s).
        #[allow(dead_code)] // (all are loaded, as upstream does, though not every one is called)
        pub(super) struct AaudioLib {
            $(pub(super) $name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
            $(pub(super) $opt: Option<unsafe extern "C" fn($($oarg),*) $(-> $oret)?>,)*
            _handle: SharedObject,
        }

        impl AaudioLib {
            /// Translation of `AAUDIO_LoadFunctions()`.
            pub(super) fn load_functions(handle: SharedObject) -> Result<AaudioLib> {
                Ok(AaudioLib {
                    $(
                        // SAFETY: the declared type is the function's C
                        // signature from <aaudio/AAudio.h>, and it is only
                        // called while `_handle` keeps the library loaded.
                        $name: unsafe { handle.function(stringify!($name)) }.map_err(|e| {
                            Error::new(format!(
                                "Couldn't load AAUDIO function {}: {}",
                                stringify!($name),
                                e.message()
                            ))
                        })?,
                    )*
                    $(
                        // if it fails, okay. (SAFETY: as above.)
                        $opt: unsafe { handle.function(stringify!($opt)) }.ok(),
                    )*
                    _handle: handle,
                })
            }
        }
    };
}

aaudio_procs! {
    required {
    AAudio_convertResultToText: fn(aaudio_result_t) -> *const c_char;
    AAudio_convertStreamStateToText: fn(aaudio_stream_state_t) -> *const c_char;
    AAudio_createStreamBuilder: fn(*mut *mut AAudioStreamBuilder) -> aaudio_result_t;
    AAudioStreamBuilder_setDeviceId: fn(*mut AAudioStreamBuilder, i32);
    AAudioStreamBuilder_setSampleRate: fn(*mut AAudioStreamBuilder, i32);
    AAudioStreamBuilder_setChannelCount: fn(*mut AAudioStreamBuilder, i32);
    AAudioStreamBuilder_setFormat: fn(*mut AAudioStreamBuilder, aaudio_format_t);
    AAudioStreamBuilder_setDirection: fn(*mut AAudioStreamBuilder, aaudio_direction_t);
    AAudioStreamBuilder_setBufferCapacityInFrames: fn(*mut AAudioStreamBuilder, i32);
    AAudioStreamBuilder_setPerformanceMode: fn(*mut AAudioStreamBuilder, aaudio_performance_mode_t);
    AAudioStreamBuilder_setDataCallback: fn(*mut AAudioStreamBuilder, AAudioStream_dataCallback, *mut c_void);
    AAudioStreamBuilder_setFramesPerDataCallback: fn(*mut AAudioStreamBuilder, i32);
    AAudioStreamBuilder_setErrorCallback: fn(*mut AAudioStreamBuilder, AAudioStream_errorCallback, *mut c_void);
    AAudioStreamBuilder_openStream: fn(*mut AAudioStreamBuilder, *mut *mut AAudioStream) -> aaudio_result_t;
    AAudioStreamBuilder_delete: fn(*mut AAudioStreamBuilder) -> aaudio_result_t;
    AAudioStream_close: fn(*mut AAudioStream) -> aaudio_result_t;
    AAudioStream_requestStart: fn(*mut AAudioStream) -> aaudio_result_t;
    AAudioStream_requestPause: fn(*mut AAudioStream) -> aaudio_result_t;
    AAudioStream_requestStop: fn(*mut AAudioStream) -> aaudio_result_t;
    AAudioStream_getState: fn(*mut AAudioStream) -> aaudio_stream_state_t;
    AAudioStream_getBufferCapacityInFrames: fn(*mut AAudioStream) -> i32;
    AAudioStream_getFramesPerDataCallback: fn(*mut AAudioStream) -> i32;
    AAudioStream_getSampleRate: fn(*mut AAudioStream) -> i32;
    AAudioStream_getChannelCount: fn(*mut AAudioStream) -> i32;
    AAudioStream_getFormat: fn(*mut AAudioStream) -> aaudio_format_t;
    AAudioStream_getTimestamp: fn(*mut AAudioStream, clockid_t, *mut i64, *mut i64) -> aaudio_result_t;
    }
    optional {
    // API 28
    AAudioStreamBuilder_setUsage: fn(*mut AAudioStreamBuilder, aaudio_usage_t);
    // API 28
    AAudioStreamBuilder_setInputPreset: fn(*mut AAudioStreamBuilder, aaudio_input_preset_t);
    }
}

impl AaudioLib {
    /// `ctx.AAudio_convertResultToText(res)` as a `String`.
    pub(super) fn result_text(&self, res: aaudio_result_t) -> String {
        // SAFETY: AAudio_convertResultToText() takes any value and returns
        // a static string.
        let text = unsafe { (self.AAudio_convertResultToText)(res) };
        if text.is_null() {
            return String::new();
        }
        // SAFETY: a NUL-terminated static string.
        unsafe { std::ffi::CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    }
}
