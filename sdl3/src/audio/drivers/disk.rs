// Rust translation of src/audio/disk/SDL_diskaudio.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

// Output raw audio data to a file.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::audio::device::{
    add_audio_device, AudioBootStrap, AudioDriverImpl, DeviceBackend, DriverFlags, PhysState,
    PhysicalDevice, DEFAULT_PLAYBACK_DEVNAME, DEFAULT_RECORDING_DEVNAME,
};
use crate::error::Result;
use crate::io::IoStream;

const DISKDEFAULT_OUTFILE: &str = "sdlaudio.raw";
const DISKDEFAULT_INFILE: &str = "sdlaudio-in.raw";

/// The disk driver (`DISKAUDIO_Init()`).
struct DiskAudio;

/// Translation of `struct SDL_PrivateAudioData` (the core owns the mix buffer).
struct DiskDevice {
    io: Mutex<Option<IoStream<'static>>>,
    io_delay: u32,
}

impl DeviceBackend for DiskDevice {
    /// Translation of `DISKAUDIO_WaitDevice()`.
    fn wait_device(&self, _device: &PhysicalDevice) -> bool {
        crate::timer::delay(Duration::from_millis(self.io_delay as u64));
        true
    }

    /// Translation of `DISKAUDIO_WaitDevice()` (also used for recording).
    fn wait_recording_device(&self, device: &PhysicalDevice) -> bool {
        self.wait_device(device)
    }

    /// Translation of `DISKAUDIO_PlayDevice()`.
    fn play_device(&self, _device: &PhysicalDevice, buffer: &[u8]) -> bool {
        let mut io = self.io.lock().unwrap_or_else(|e| e.into_inner());
        let written = io.as_mut().map_or(0, |io| io.write(buffer));
        // If we couldn't write, assume fatal error for now
        written == buffer.len()
    }

    /// Translation of `DISKAUDIO_GetDeviceBuf()`.
    fn get_device_buf(&self, _device: &PhysicalDevice, buffer_size: usize) -> Option<usize> {
        Some(buffer_size)
    }

    /// Translation of `DISKAUDIO_RecordDevice()`.
    fn record_device(&self, device: &PhysicalDevice, buffer: &mut [u8]) -> Result<usize> {
        let origbuflen = buffer.len();
        let mut br = 0;

        let mut io = self.io.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(stream) = io.as_mut() {
            br = stream.read(buffer);
            if br < origbuflen {
                // EOF (or error, but whatever).
                *io = None; // (SDL_CloseIO)
            }
        }

        // if we ran out of file, just write silence.
        buffer[br..].fill(crate::audio::device::silence_value_of(device));

        Ok(origbuflen)
    }

    /// Translation of `DISKAUDIO_FlushRecording()`.
    fn flush_recording(&self, _device: &PhysicalDevice) {
        // no op...we don't advance the file pointer or anything.
    }

    /// Translation of `DISKAUDIO_CloseDevice()`.
    fn close_device(&self, _device: &PhysicalDevice) {
        let io = self.io.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(io) = io {
            let _ = io.close();
        }
    }
}

/// Translation of `get_filename()`.
fn get_filename(recording: bool) -> String {
    crate::hints::get(if recording {
        crate::hints::AUDIO_DISK_INPUT_FILE
    } else {
        crate::hints::AUDIO_DISK_OUTPUT_FILE
    })
    .unwrap_or_else(|| {
        (if recording {
            DISKDEFAULT_INFILE
        } else {
            DISKDEFAULT_OUTFILE
        })
        .to_owned()
    })
}

impl AudioDriverImpl for DiskAudio {
    fn flags(&self) -> DriverFlags {
        DriverFlags {
            has_recording_support: true,
            ..DriverFlags::default()
        }
    }

    /// Translation of `DISKAUDIO_DetectDevices()`.
    fn detect_devices(&self) -> (Option<Arc<PhysicalDevice>>, Option<Arc<PhysicalDevice>>) {
        (
            add_audio_device(false, DEFAULT_PLAYBACK_DEVNAME, None, None, 0x1),
            add_audio_device(true, DEFAULT_RECORDING_DEVNAME, None, None, 0x2),
        )
    }

    /// Translation of `DISKAUDIO_OpenDevice()`.
    fn open_device(
        &self,
        device: &PhysicalDevice,
        state: &mut PhysState,
    ) -> Result<Arc<dyn DeviceBackend>> {
        let recording = device.recording;
        let fname = get_filename(recording);

        let io_delay = super::io_delay(
            state.sample_frames,
            state.spec.freq,
            crate::hints::AUDIO_DISK_TIMESCALE,
        );

        // Open the "audio device"
        let io = IoStream::from_file(&fname, if recording { "rb" } else { "wb" })?;

        // (the core allocates the silenced mixing buffer)

        crate::critical!(
            crate::log::Category::Audio,
            "You are using the SDL disk i/o audio driver!"
        );
        crate::critical!(
            crate::log::Category::Audio,
            " {} file [{}], format={} channels={} freq={}.",
            if recording {
                "Reading from"
            } else {
                "Writing to"
            },
            fname,
            state.spec.format.short_name(), // AudioFormatString()
            state.spec.channels,
            state.spec.freq
        );

        Ok(Arc::new(DiskDevice {
            io: Mutex::new(Some(io)),
            io_delay,
        })) // We're ready to rock and roll. :-)
    }
}

/// Translation of `DISKAUDIO_Init()`.
fn diskaudio_init() -> Option<Arc<dyn AudioDriverImpl>> {
    Some(Arc::new(DiskAudio))
}

/// Translation of `DISKAUDIO_bootstrap`.
pub(crate) static DISKAUDIO_BOOTSTRAP: AudioBootStrap = AudioBootStrap {
    name: "disk",
    desc: "direct-to-disk audio",
    init: diskaudio_init,
    demand_only: true,
    is_preferred: false,
};
