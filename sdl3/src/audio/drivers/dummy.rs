// Rust translation of src/audio/dummy/SDL_dummyaudio.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

// Output audio to nowhere...

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
}

impl AudioDriverImpl for DummyAudio {
    fn flags(&self) -> DriverFlags {
        DriverFlags {
            only_has_default_playback_device: true,
            only_has_default_recording_device: true,
            has_recording_support: true,
            provides_own_callback_thread: false,
        }
    }

    /// Translation of `DUMMYAUDIO_OpenDevice()`.
    fn open_device(
        &self,
        _device: &PhysicalDevice,
        state: &mut PhysState,
    ) -> Result<Arc<dyn DeviceBackend>> {
        let io_delay = super::io_delay(
            state.sample_frames,
            state.spec.freq,
            crate::hints::AUDIO_DUMMY_TIMESCALE,
        );
        Ok(Arc::new(DummyDevice { io_delay })) // we're good; don't change reported device format.
    }
}

/// Translation of `DUMMYAUDIO_Init()`.
fn dummyaudio_init() -> Option<Arc<dyn AudioDriverImpl>> {
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
