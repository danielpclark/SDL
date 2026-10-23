// Rust translation of the JavaScript in src/audio/emscripten/SDL_emscriptenaudio.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The JavaScript of the Web Audio driver, `SDL_emscriptenaudio.c` (and
//! [`dummy`] for the Emscripten parts of `SDL_dummyaudio.c`).
//!
//! Each function is upstream's `MAIN_THREAD_EM_ASM` body, verbatim (in the
//! braces emscripten puts around an `EM_ASM`'s code): without threads the
//! main thread is the only one. The state is in `Module['SDL3']`:
//! `audioContext` (one `AudioContext` for every device), and
//! `audio_playback` and `audio_recording`, each device's
//! `ScriptProcessorNode`, its silence timer and buffer, and the buffer the
//! device's next `PlayDevice`/`RecordDevice` reads or writes.
//!
//! Where upstream passes `SDL_PlaybackAudioThreadIterate` (or
//! `SDL_RecordingAudioThreadIterate`) and its `SDL_AudioDevice *` for the
//! JavaScript to call back with `dynCall('ip', iterate, [device])`, the
//! callers pass a Rust function of that type (`extern "C-unwind" fn(usize)
//! -> bool`) and the device's handle.

pub mod dummy;

use core::ffi::c_void;

// EMSCRIPTENAUDIO_PlayDevice()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Copies `frames` frames of interleaved `float` samples at `buf` into
    /// the channels of `Module['SDL3'].audio_playback.currentPlaybackBuffer`
    /// (an `AudioBuffer` with `frames` frames); throws if its length
    /// differs.
    pub fn sdl3_audio_play_device(buf: *const u8, frames: i32),
    r#"{
        /* Convert incoming buf pointer to a HEAPF32 offset. */
        var SDL3 = Module['SDL3'];
        var buf = SDL3.CPtrToHeap32Index($0);
        var numChannels = SDL3.audio_playback.currentPlaybackBuffer['numberOfChannels'];
        for (var c = 0; c < numChannels; ++c) {
            var channelData = SDL3.audio_playback.currentPlaybackBuffer['getChannelData'](c);
            if (channelData.length != $1) {
                throw 'Web Audio playback buffer length mismatch! Destination size: ' + channelData.length + ' samples vs expected ' + $1 + ' samples!';
            }

            for (var j = 0; j < $1; ++j) {
                channelData[j] = HEAPF32[buf + (j * numChannels + c)];
            }
        }
    }"#
);

// EMSCRIPTENAUDIO_RecordDevice()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Copies the channels of
    /// `Module['SDL3'].audio_recording.currentRecordingBuffer` (`frames`
    /// frames) into `buf`, interleaved `float` samples; throws if its length
    /// differs.
    pub fn sdl3_audio_record_device(buf: *mut u8, frames: i32),
    r#"{
        var SDL3 = Module['SDL3'];
        var numChannels = SDL3.audio_recording.currentRecordingBuffer.numberOfChannels;
        for (var c = 0; c < numChannels; ++c) {
            var channelData = SDL3.audio_recording.currentRecordingBuffer.getChannelData(c);
            if (channelData.length != $1) {
                throw 'Web Audio recording buffer length mismatch! Destination size: ' + channelData.length + ' samples vs expected ' + $1 + ' samples!';
            }

            if (numChannels == 1) {  // fastpath this a little for the common (mono) case.
                for (var j = 0; j < $1; ++j) {
                    setValue($0 + (j * 4), channelData[j], 'float');
                }
            } else {
                for (var j = 0; j < $1; ++j) {
                    setValue($0 + (((j * numChannels) + c) * 4), channelData[j], 'float');
                }
            }
        }
    }"#
);

// EMSCRIPTENAUDIO_CloseDevice()'s MAIN_THREAD_EM_ASM, verbatim.
crate::em_js_dollar!(
    /// Takes the recording device (`recording` nonzero) or the playback one
    /// apart: its silence timer, its nodes (and the microphone's stream's
    /// tracks), then `audio_recording` or `audio_playback` itself; and once
    /// both are gone, closes the `AudioContext`.
    pub fn sdl3_audio_close_device(recording: i32),
    r#"{
        var SDL3 = Module['SDL3'];
        if ($0) {
            if (SDL3.audio_recording.silenceTimer !== undefined) {
                clearInterval(SDL3.audio_recording.silenceTimer);
            }
            if (SDL3.audio_recording.stream !== undefined) {
                var tracks = SDL3.audio_recording.stream.getAudioTracks();
                for (var i = 0; i < tracks.length; i++) {
                    SDL3.audio_recording.stream.removeTrack(tracks[i]);
                }
            }
            if (SDL3.audio_recording.scriptProcessorNode !== undefined) {
                SDL3.audio_recording.scriptProcessorNode.onaudioprocess = function(audioProcessingEvent) {};
                SDL3.audio_recording.scriptProcessorNode.disconnect();
            }
            if (SDL3.audio_recording.mediaStreamNode !== undefined) {
                SDL3.audio_recording.mediaStreamNode.disconnect();
            }
            SDL3.audio_recording = undefined;
        } else {
            if (SDL3.audio_playback.scriptProcessorNode != undefined) {
                SDL3.audio_playback.scriptProcessorNode.disconnect();
            }
            if (SDL3.audio_playback.silenceTimer !== undefined) {
                clearInterval(SDL3.audio_playback.silenceTimer);
            }
            SDL3.audio_playback = undefined;
        }
        if ((SDL3.audioContext !== undefined) && (SDL3.audio_playback === undefined) && (SDL3.audio_recording === undefined)) {
            SDL3.audioContext.close();
            SDL3.audioContext = undefined;
        }
    }"#
);

// The first MAIN_THREAD_EM_ASM_INT of EMSCRIPTENAUDIO_OpenDevice()
// ("create context"), verbatim. (`autoResumeAudioContext` is the dependency
// table's.)
crate::em_js_dollar!(
    /// Makes `audio_playback` and `audio_recording` (empty) where they
    /// aren't, and the `AudioContext` where there's none (resumed on the
    /// page's first key press, mouse button or touch where the browser has
    /// no `navigator.userActivation`). Whether there's one now (1).
    pub fn sdl3_audio_create_context() -> i32,
    r#"{
        var SDL3 = Module['SDL3'];
        if (typeof(SDL3.audio_playback) === 'undefined') {
            SDL3.audio_playback = {};
        }
        if (typeof(SDL3.audio_recording) === 'undefined') {
            SDL3.audio_recording = {};
        }

        if (!SDL3.audioContext) {
            if (typeof(AudioContext) !== 'undefined') {
                SDL3.audioContext = new AudioContext();
            } else if (typeof(webkitAudioContext) !== 'undefined') {
                SDL3.audioContext = new webkitAudioContext();
            }
            if (SDL3.audioContext) {
                if ((typeof navigator.userActivation) === 'undefined') {
                    autoResumeAudioContext(SDL3.audioContext);
                }
            }
        }
        return (SDL3.audioContext !== undefined);
    }"#
);

// EMSCRIPTENAUDIO_OpenDevice()'s MAIN_THREAD_EM_ASM_INT for the frequency
// ("limit to native freq"), verbatim.
crate::em_js_dollar!(
    /// The `AudioContext`'s `sampleRate`.
    pub fn sdl3_audio_sample_rate() -> i32,
    r#"{ return Module['SDL3'].audioContext.sampleRate; }"#
);

// EMSCRIPTENAUDIO_OpenDevice()'s MAIN_THREAD_EM_ASM for a recording device,
// verbatim.
crate::em_js_dollar!(
    /// Sets up recording: a silence timer that calls `iterate(device)` with
    /// a silent `currentRecordingBuffer` every `frames` frames until the
    /// microphone (`getUserMedia()`) answers; then the microphone's
    /// `MediaStreamAudioSourceNode` into a `ScriptProcessorNode` of
    /// `frames` frames and `channels` channels (connected to the
    /// destination, writing silence), whose `onaudioprocess` calls
    /// `iterate(device)` with its input as `currentRecordingBuffer`.
    pub fn sdl3_audio_open_recording(
        channels: i32,
        frames: i32,
        iterate: *const c_void,
        device: usize
    ),
    r#"{
            var SDL3 = Module['SDL3'];
            var have_microphone = function(stream) {
                //console.log('SDL audio recording: we have a microphone! Replacing silence callback.');
                if (SDL3.audio_recording.silenceTimer !== undefined) {
                    clearInterval(SDL3.audio_recording.silenceTimer);
                    SDL3.audio_recording.silenceTimer = undefined;
                    SDL3.audio_recording.silenceBuffer = undefined
                }
                SDL3.audio_recording.mediaStreamNode = SDL3.audioContext.createMediaStreamSource(stream);
                SDL3.audio_recording.scriptProcessorNode = SDL3.audioContext.createScriptProcessor($1, $0, 1);
                SDL3.audio_recording.scriptProcessorNode.onaudioprocess = function(audioProcessingEvent) {
                    if ((SDL3 === undefined) || (SDL3.audio_recording === undefined)) { return; }
                    audioProcessingEvent.outputBuffer.getChannelData(0).fill(0.0);
                    SDL3.audio_recording.currentRecordingBuffer = audioProcessingEvent.inputBuffer;
                    dynCall('ip', $2, [$3]);
                };
                SDL3.audio_recording.mediaStreamNode.connect(SDL3.audio_recording.scriptProcessorNode);
                SDL3.audio_recording.scriptProcessorNode.connect(SDL3.audioContext.destination);
                SDL3.audio_recording.stream = stream;
            };

            var no_microphone = function(error) {
                //console.log('SDL audio recording: we DO NOT have a microphone! (' + error.name + ')...leaving silence callback running.');
            };

            // we write silence to the audio callback until the microphone is available (user approves use, etc).
            SDL3.audio_recording.silenceBuffer = SDL3.audioContext.createBuffer($0, $1, SDL3.audioContext.sampleRate);
            SDL3.audio_recording.silenceBuffer.getChannelData(0).fill(0.0);
            var silence_callback = function() {
                SDL3.audio_recording.currentRecordingBuffer = SDL3.audio_recording.silenceBuffer;
                dynCall('ip', $2, [$3]);
            };

            SDL3.audio_recording.silenceTimer = setInterval(silence_callback, ($1 / SDL3.audioContext.sampleRate) * 1000);

            if ((navigator.mediaDevices !== undefined) && (navigator.mediaDevices.getUserMedia !== undefined)) {
                navigator.mediaDevices.getUserMedia({ audio: true, video: false }).then(have_microphone).catch(no_microphone);
            }
        }"#
);

// EMSCRIPTENAUDIO_OpenDevice()'s MAIN_THREAD_EM_ASM for a playback device
// ("setup a ScriptProcessorNode"), verbatim.
crate::em_js_dollar!(
    /// Sets up playback: a `ScriptProcessorNode` of `frames` frames and
    /// `channels` output channels, connected to the destination, whose
    /// `onaudioprocess` calls `iterate(device)` with its output as
    /// `currentPlaybackBuffer`. While the `AudioContext` is suspended
    /// (autoplay is blocked), a silence timer calls `iterate(device)` every
    /// `frames` frames into a buffer that's thrown away (and resumes the
    /// context once the page has had a user gesture); the node's first
    /// `onaudioprocess` stops it.
    pub fn sdl3_audio_open_playback(
        channels: i32,
        frames: i32,
        iterate: *const c_void,
        device: usize
    ),
    r#"{
            var SDL3 = Module['SDL3'];
            SDL3.audio_playback.scriptProcessorNode = SDL3.audioContext['createScriptProcessor']($1, 0, $0);
            SDL3.audio_playback.scriptProcessorNode['onaudioprocess'] = function (e) {
                if ((SDL3 === undefined) || (SDL3.audio_playback === undefined)) { return; }
                // if we're actually running the node, we don't need the fake callback anymore, so kill it.
                if (SDL3.audio_playback.silenceTimer !== undefined) {
                    clearInterval(SDL3.audio_playback.silenceTimer);
                    SDL3.audio_playback.silenceTimer = undefined;
                    SDL3.audio_playback.silenceBuffer = undefined;
                }
                SDL3.audio_playback.currentPlaybackBuffer = e['outputBuffer'];
                dynCall('ip', $2, [$3]);
            };

            SDL3.audio_playback.scriptProcessorNode['connect'](SDL3.audioContext['destination']);

            if (SDL3.audioContext.state === 'suspended') {  // uhoh, autoplay is blocked.
                SDL3.audio_playback.silenceBuffer = SDL3.audioContext.createBuffer($0, $1, SDL3.audioContext.sampleRate);
                SDL3.audio_playback.silenceBuffer.getChannelData(0).fill(0.0);
                var silence_callback = function() {
                    if ((typeof navigator.userActivation) !== 'undefined') {
                        if (navigator.userActivation.hasBeenActive) {
                            SDL3.audioContext.resume();
                        }
                    }

                    // the buffer that gets filled here just gets ignored, so the app can make progress
                    //  and/or avoid flooding audio queues until we can actually play audio.
                    SDL3.audio_playback.currentPlaybackBuffer = SDL3.audio_playback.silenceBuffer;
                    dynCall('ip', $2, [$3]);
                    SDL3.audio_playback.currentPlaybackBuffer = undefined;
                };

                SDL3.audio_playback.silenceTimer = setInterval(silence_callback, ($1 / SDL3.audioContext.sampleRate) * 1000);
            }
        }"#
);

// EMSCRIPTENAUDIO_Init()'s first MAIN_THREAD_EM_ASM_INT ("check
// availability"), verbatim.
crate::em_js_dollar!(
    /// Whether there's an `AudioContext` (or `webkitAudioContext`) to make
    /// (1): not under node.
    pub fn sdl3_audio_available() -> i32,
    r#"{
        if (typeof(AudioContext) !== 'undefined') {
            return true;
        } else if (typeof(webkitAudioContext) !== 'undefined') {
            return true;
        }
        return false;
    }"#
);

// EMSCRIPTENAUDIO_Init()'s second MAIN_THREAD_EM_ASM_INT, verbatim.
crate::em_js_dollar!(
    /// Whether the page can ask for the microphone,
    /// `navigator.mediaDevices.getUserMedia()` (1).
    pub fn sdl3_audio_recording_available() -> i32,
    r#"{
        if ((typeof(navigator.mediaDevices) !== 'undefined') && (typeof(navigator.mediaDevices.getUserMedia) !== 'undefined')) {
            return true;
        }
        return false;
    }"#
);
