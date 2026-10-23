// Rust translation of src/audio/dummy/SDL_dummyaudio.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

// Output audio to nowhere...
//
// On Emscripten, without threads, a repeating timer per device (the
// browser's or node's setInterval(), in its event loop) runs the device's
// iterations, so the driver provides its own callback "thread"; the
// JavaScript is upstream's, in `sdl3_emscripten::audio::dummy`.

use std::sync::Arc;
use std::time::Duration;

use crate::audio::device::{
    AudioBootStrap, AudioDriverImpl, DeviceBackend, DriverFlags, PhysState, PhysicalDevice,
};
use crate::error::Result;

/// The dummy driver (`DUMMYAUDIO_Init()`).
struct DummyAudio;

/// Translation of `struct SDL_PrivateAudioData` (the core owns the mix buffer).
struct DummyDevice {
    io_delay: u32,
}

impl DeviceBackend for DummyDevice {
    /// Translation of `DUMMYAUDIO_WaitDevice()`.
    fn wait_device(&self, _device: &PhysicalDevice) -> bool {
        crate::timer::delay(Duration::from_millis(self.io_delay as u64));
        true
    }

    /// Translation of `DUMMYAUDIO_GetDeviceBuf()`.
    fn get_device_buf(&self, _device: &PhysicalDevice, buffer_size: usize) -> Option<usize> {
        Some(buffer_size)
    }

    /// Translation of `DUMMYAUDIO_WaitDevice()` (also used for recording).
    fn wait_recording_device(&self, device: &PhysicalDevice) -> bool {
        self.wait_device(device)
    }

    /// Translation of `DUMMYAUDIO_RecordDevice()`.
    fn record_device(&self, device: &PhysicalDevice, buffer: &mut [u8]) -> Result<usize> {
        // always return a full buffer of silence.
        buffer.fill(crate::audio::device::silence_value_of(device));
        Ok(buffer.len())
    }

    /// Translation of `DUMMYAUDIO_CloseDevice()` (the core frees the mix
    /// buffer).
    #[cfg(target_os = "emscripten")]
    fn close_device(&self, device: &PhysicalDevice) {
        // on Emscripten, we just fire a repeating timer to consume audio.
        // SAFETY: plain JavaScript.
        unsafe { sdl3_emscripten::audio::dummy::sdl3_dummy_audio_close(device.recording as i32) };
    }
}

/// `SDL_PlaybackAudioThreadIterate(device)` or
/// `SDL_RecordingAudioThreadIterate(device)`, which the timer calls
/// (`dynCall('vi', …)`) with the device's handle (the dummy devices are the
/// default ones, 0x1 and 0x2).
#[cfg(target_os = "emscripten")]
extern "C-unwind" fn iterate(handle: usize) -> bool {
    use crate::audio::device::{
        find_physical_audio_device_by_handle, playback_audio_thread_iterate,
        recording_audio_thread_iterate,
    };
    match find_physical_audio_device_by_handle(handle) {
        Some(device) if device.recording => recording_audio_thread_iterate(&device),
        Some(device) => playback_audio_thread_iterate(&device),
        None => false,
    }
}

impl AudioDriverImpl for DummyAudio {
    fn flags(&self) -> DriverFlags {
        DriverFlags {
            only_has_default_playback_device: true,
            only_has_default_recording_device: true,
            has_recording_support: true,
            // on Emscripten, we just fire a repeating timer to consume audio.
            provides_own_callback_thread: cfg!(target_os = "emscripten"),
        }
    }

    /// Translation of `DUMMYAUDIO_OpenDevice()`.
    fn open_device(
        &self,
        device: &PhysicalDevice,
        state: &mut PhysState,
    ) -> Result<Arc<dyn DeviceBackend>> {
        let io_delay = super::io_delay(
            state.sample_frames,
            state.spec.freq,
            crate::hints::AUDIO_DUMMY_TIMESCALE,
        );

        // on Emscripten, we just fire a repeating timer to consume audio.
        #[cfg(target_os = "emscripten")]
        {
            // SAFETY: the timer calls back an extern "C-unwind" fn(usize) ->
            // bool, which lives as long as the program, with the handle.
            unsafe {
                sdl3_emscripten::audio::dummy::sdl3_dummy_audio_open(
                    device.recording as i32,
                    state.sample_frames,
                    state.spec.freq,
                    iterate as *const std::ffi::c_void,
                    device.handle,
                )
            };
        }
        #[cfg(not(target_os = "emscripten"))]
        let _ = device;

        Ok(Arc::new(DummyDevice { io_delay })) // we're good; don't change reported device format.
    }
}

/// Translation of `DUMMYAUDIO_Init()`.
fn dummyaudio_init() -> Option<Arc<dyn AudioDriverImpl>> {
    // on Emscripten, we just fire a repeating timer to consume audio.
    #[cfg(target_os = "emscripten")]
    {
        // SAFETY: plain JavaScript (`Module['SDL3']` is made by
        // SDL_InitSubSystem()).
        unsafe { sdl3_emscripten::audio::dummy::sdl3_dummy_audio_init() };
    }

    Some(Arc::new(DummyAudio))
}

/// Translation of `DUMMYAUDIO_bootstrap`.
pub(crate) static DUMMYAUDIO_BOOTSTRAP: AudioBootStrap = AudioBootStrap {
    name: "dummy",
    desc: "SDL dummy audio driver",
    init: dummyaudio_init,
    demand_only: true,
    is_preferred: false,
};
