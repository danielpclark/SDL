// Rust translation of the JavaScript in src/audio/dummy/SDL_dummyaudio.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The JavaScript of the dummy audio driver on Emscripten
//! (`SDL_dummyaudio.c`'s `SDL_PLATFORM_EMSCRIPTEN` parts): without threads
//! a repeating timer per device (`Module['SDL3'].dummy_audio.timers`, 0 for
//! playback and 1 for recording) consumes or produces the audio, calling
//! back `iterate(device)` (`dynCall('vi', iterate, [device])`; upstream's
//! `SDL_PlaybackAudioThreadIterate` or `SDL_RecordingAudioThreadIterate`
//! and its `SDL_AudioDevice *`, here a Rust function of that type and the
//! device's handle).

use core::ffi::c_void;

// DUMMYAUDIO_Init()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Makes `Module['SDL3'].dummy_audio`, with no timers.
    pub fn sdl3_dummy_audio_init(),
    r#"{
        Module['SDL3'].dummy_audio = {};
        Module['SDL3'].dummy_audio.timers = [];
        Module['SDL3'].dummy_audio.timers[0] = undefined;
        Module['SDL3'].dummy_audio.timers[1] = undefined;
    }"#
);

// DUMMYAUDIO_OpenDevice()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Starts timer `recording` (0 or 1), replacing the one before: every
    /// `sample_frames` frames at `freq` Hz it calls `iterate(device)`.
    pub fn sdl3_dummy_audio_open(
        recording: i32,
        sample_frames: i32,
        freq: i32,
        iterate: *const c_void,
        device: usize
    ),
    r#"{
        var a = Module['SDL3'].dummy_audio;
        if (a.timers[$0] !== undefined) { clearInterval(a.timers[$0]); }
        a.timers[$0] = setInterval(function() { dynCall('vi', $3, [$4]); }, ($1 / $2) * 1000);
    }"#
);

// DUMMYAUDIO_CloseDevice()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Stops timer `recording` (0 or 1).
    pub fn sdl3_dummy_audio_close(recording: i32),
    r#"{
            var a = Module['SDL3'].dummy_audio;
            if (a.timers[$0] !== undefined) { clearInterval(a.timers[$0]); }
            a.timers[$0] = undefined;
        }"#
);
