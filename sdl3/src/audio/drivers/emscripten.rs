// Rust translation of src/audio/emscripten/SDL_emscriptenaudio.c and
// src/audio/emscripten/SDL_emscriptenaudio.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Emscripten audio driver: Web Audio. One `AudioContext` for every
//! device (at its own sample rate; Web Audio only takes floats), a default
//! playback device that's a `ScriptProcessorNode` connected to the
//! context's destination, and a default recording device that's the
//! microphone (`getUserMedia()`) into another, once the page may use it.
//!
//! As upstream's build has no threads, the driver provides its own
//! callback "thread" (`ProvidesOwnCallbackThread`): the nodes'
//! `onaudioprocess` events, in the browser's event loop, run the device's
//! iterations (`SDL_PlaybackAudioThreadIterate()` and
//! `SDL_RecordingAudioThreadIterate()`), which call back into
//! `PlayDevice`/`RecordDevice` to copy the samples between the device's
//! buffer and the node's `AudioBuffer`. Until those events come (a
//! suspended context: the browser's autoplay policy blocks it until the
//! user interacts with the page, and the microphone until the user allows
//! it) a timer runs the iterations with a silent buffer, so the
//! application keeps going.
//!
//! The JavaScript is upstream's, in the `sdl3-emscripten` crate
//! (`sdl3_emscripten::audio`). Where upstream hands it the iteration
//! functions and the `SDL_AudioDevice *` to call them with, this hands it
//! [`playback_iterate`] or [`recording_iterate`] and the device's handle,
//! which they look the device up by (as the CoreAudio and PipeWire drivers'
//! callbacks do).
//!
//! Note (upstream), three things the JavaScript does, kept as they are:
//!
//! * opening either device makes both devices' tables
//!   (`audio_playback`, `audio_recording`), and the context is closed only
//!   once both are gone, so after a device of one kind alone is closed (or
//!   SDL quits) the context stays open, and the next device reuses it;
//! * closing the recording device removes the microphone's track from its
//!   stream without stopping it, so the browser goes on recording (and
//!   showing that it does);
//! * an iteration that comes after its device closed (an `onaudioprocess`
//!   already queued when the node was disconnected, run once a device of
//!   either kind has made the tables again) dereferences upstream's NULL
//!   `device->hidden`; here the core finds no backend and throws the audio
//!   away, as for a lost device.
//!
//! Without Web Audio (under node: no `AudioContext`) the driver is
//! unavailable, as upstream's is, and audio initialization goes on to the
//! next driver: none (disk and dummy are demand-only) unless the
//! [`AUDIO_DRIVER`](crate::hints::AUDIO_DRIVER) hint asks for one. Note
//! (upstream): its error is the core's "Audio target '' failed to
//! initialize" (upstream keeps the driver's "No audio context available",
//! as for every driver that fails; see `init_audio()`).

use std::ffi::c_void;
use std::sync::Arc;

use sdl3_emscripten::audio as js;

use crate::audio::device::{
    audio_thread_finalize, find_physical_audio_device_by_handle,
    get_default_sample_frames_from_freq, playback_audio_thread_iterate,
    recording_audio_thread_iterate, updated_audio_device_format, AudioBootStrap, AudioDriverImpl,
    DeviceBackend, DriverFlags, PhysState, PhysicalDevice,
};
use crate::audio::AudioFormat;
use crate::error::{Error, Result};

/// The driver (`EMSCRIPTENAUDIO_Init()`'s implementation).
struct EmscriptenAudio {
    recording_available: bool,
}

/// An opened device. Translation of `struct SDL_PrivateAudioData` (the
/// core owns the mix buffer, `mixbuf`); what `PlayDevice` and
/// `RecordDevice` read of the device.
struct EmscriptenDevice {
    /// The device's frame size (`SDL_AUDIO_FRAMESIZE(device->spec)`).
    frame_size: usize,
    /// `device->spec.channels`.
    channels: i32,
}

/// `SDL_PlaybackAudioThreadIterate(device)`, which the JavaScript calls
/// (`dynCall('ip', …)`) with the device's handle.
extern "C-unwind" fn playback_iterate(handle: usize) -> bool {
    match find_physical_audio_device_by_handle(handle) {
        Some(device) => playback_audio_thread_iterate(&device),
        None => false,
    }
}

/// `SDL_RecordingAudioThreadIterate(device)`, which the JavaScript calls
/// (`dynCall('ip', …)`) with the device's handle.
extern "C-unwind" fn recording_iterate(handle: usize) -> bool {
    match find_physical_audio_device_by_handle(handle) {
        Some(device) => recording_audio_thread_iterate(&device),
        None => false,
    }
}

impl DeviceBackend for EmscriptenDevice {
    /// Translation of `EMSCRIPTENAUDIO_GetDeviceBuf()`.
    fn get_device_buf(&self, _device: &PhysicalDevice, buffer_size: usize) -> Option<usize> {
        Some(buffer_size)
    }

    /// Translation of `EMSCRIPTENAUDIO_PlayDevice()`.
    fn play_device(&self, _device: &PhysicalDevice, buffer: &[u8]) -> bool {
        let framelen = self.frame_size.max(1);
        // SAFETY: the JavaScript reads the buffer's `float`s, as many frames
        // as it holds.
        unsafe { js::sdl3_audio_play_device(buffer.as_ptr(), (buffer.len() / framelen) as i32) };
        true
    }

    /// Translation of `EMSCRIPTENAUDIO_FlushRecording()`.
    fn flush_recording(&self, _device: &PhysicalDevice) {
        // Do nothing, the new data will just be dropped.
    }

    /// Translation of `EMSCRIPTENAUDIO_RecordDevice()`.
    fn record_device(&self, _device: &PhysicalDevice, buffer: &mut [u8]) -> Result<usize> {
        let buflen = buffer.len();
        let frames = (buflen / std::mem::size_of::<f32>()) as i32 / self.channels.max(1);
        // SAFETY: the JavaScript writes `frames` frames of `channels`
        // `float`s, which the buffer holds.
        unsafe { js::sdl3_audio_record_device(buffer.as_mut_ptr(), frames) };

        Ok(buflen)
    }

    /// Translation of `EMSCRIPTENAUDIO_CloseDevice()`.
    fn close_device(&self, device: &PhysicalDevice) {
        // (`device->hidden` is there: the core calls this for an opened
        // device only)
        // SAFETY: plain JavaScript.
        unsafe { js::sdl3_audio_close_device(device.recording as i32) };

        // (the core frees the mix buffer)

        audio_thread_finalize(device);
    }
}

impl AudioDriverImpl for EmscriptenAudio {
    fn flags(&self) -> DriverFlags {
        DriverFlags {
            only_has_default_playback_device: true,

            // technically, this is just runs in idle time in the main thread, but it's close enough to a "thread" for our purposes.
            provides_own_callback_thread: true,

            has_recording_support: self.recording_available,
            only_has_default_recording_device: self.recording_available,
        }
    }

    /// Translation of `EMSCRIPTENAUDIO_OpenDevice()`.
    fn open_device(
        &self,
        device: &PhysicalDevice,
        state: &mut PhysState,
    ) -> Result<Arc<dyn DeviceBackend>> {
        // based on parts of library_sdl.js

        // create context
        // SAFETY: plain JavaScript.
        let result = unsafe { js::sdl3_audio_create_context() } != 0;

        if !result {
            return Err(Error::new("Web Audio API is not available!"));
        }

        state.spec.format = AudioFormat::F32; // web audio only supports floats

        // (`device->hidden`, below)

        // limit to native freq
        // SAFETY: plain JavaScript; the context is there.
        state.spec.freq = unsafe { js::sdl3_audio_sample_rate() };
        state.sample_frames = get_default_sample_frames_from_freq(state.spec.freq) * 2; // double the buffer size, some browsers need more, and we'll just have to live with the latency.

        updated_audio_device_format(state);

        // (the core allocates the playback device's mix buffer, silenced)

        if device.recording {
            /* The idea is to take the recording media stream, hook it up to an
            audio graph where we can pass it through a ScriptProcessorNode
            to access the raw PCM samples and push them to the SDL app's
            callback. From there, we "process" the audio data into silence
            and forget about it.

            This should, strictly speaking, use MediaRecorder for recording, but
            this API is cleaner to use and better supported, and fires a
            callback whenever there's enough data to fire down into the app.
            The downside is that we are spending CPU time silencing a buffer
            that the audiocontext uselessly mixes into any playback. On the
            upside, both of those things are not only run in native code in
            the browser, they're probably SIMD code, too. MediaRecorder
            feels like it's a pretty inefficient tapdance in similar ways,
            to be honest. */

            // SAFETY: the JavaScript calls back an extern "C-unwind" fn(usize)
            // -> bool, which lives as long as the program, with the handle.
            unsafe {
                js::sdl3_audio_open_recording(
                    state.spec.channels,
                    state.sample_frames,
                    recording_iterate as *const c_void,
                    device.handle,
                )
            };
        } else {
            // setup a ScriptProcessorNode
            // SAFETY: as above.
            unsafe {
                js::sdl3_audio_open_playback(
                    state.spec.channels,
                    state.sample_frames,
                    playback_iterate as *const c_void,
                    device.handle,
                )
            };
        }

        Ok(Arc::new(EmscriptenDevice {
            frame_size: state.spec.frame_size(),
            channels: state.spec.channels,
        }))
    }
}

/// Translation of `EMSCRIPTENAUDIO_Init()`.
fn emscriptenaudio_init() -> Option<Arc<dyn AudioDriverImpl>> {
    // check availability
    // SAFETY: plain JavaScript.
    let available = unsafe { js::sdl3_audio_available() } != 0;

    if !available {
        // ("No audio context available": see the module's documentation)
        return None;
    }

    // SAFETY: plain JavaScript.
    let recording_available = available && unsafe { js::sdl3_audio_recording_available() } != 0;

    Some(Arc::new(EmscriptenAudio {
        recording_available,
    }))
}

/// Translation of `EMSCRIPTENAUDIO_bootstrap`.
pub(crate) static EMSCRIPTENAUDIO_BOOTSTRAP: AudioBootStrap = AudioBootStrap {
    name: "emscripten",
    desc: "SDL emscripten audio driver",
    init: emscriptenaudio_init,
    demand_only: false,
    is_preferred: false,
};

#[cfg(test)]
mod tests;
