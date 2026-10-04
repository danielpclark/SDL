// Rust translation of src/audio from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Audio functionality. Translation of `SDL_audio.h`.
//!
//! All audio in SDL3 revolves around [`AudioStream`]. Whether you want to
//! play or record audio, convert it, stream it, buffer it, or mix it,
//! you're going to be passing it through an audio stream.
//!
//! Audio streams are quite flexible; they can accept any amount of data at
//! a time, in any supported format, and output it as needed in any other
//! format, even if the data format changes on either side halfway through.
//!
//! An app opens an audio device and binds any number of audio streams to
//! it, feeding more data to the streams as available. When the device needs
//! more data, it will pull it from all bound streams and mix them together
//! for playback.
//!
//! Audio streams can also use an app-provided callback to supply data
//! on-demand, which maps pretty closely to the SDL2 audio model.
//!
//! SDL also provides a simple .WAV loader in [`load_wav`] (and
//! [`load_wav_io`] if you aren't reading from a file) as a basic means to
//! load sound data into your program.
//!
//! ## Logical audio devices
//!
//! In SDL3, opening a physical device (like a SoundBlaster 16 Pro) gives
//! you a logical device ID that you can bind audio streams to. In almost
//! all cases, logical devices can be used anywhere in the API that a
//! physical device is normally used. However, since each device opening
//! generates a new logical device, different parts of the program (say, a
//! VoIP library, or text-to-speech framework, or maybe some other sort of
//! mixer on top of SDL) can have their own device opens that do not
//! interfere with each other; each logical device will mix its separate
//! audio down to a single buffer, fed to the physical device, behind the
//! scenes. As many logical devices as you like can come and go; SDL will
//! only have to open the physical device at the OS level once, and will
//! manage all the logical devices on top of it internally.
//!
//! One other benefit of logical devices: if you don't open a specific
//! physical device, instead opting for the default, SDL can automatically
//! migrate those logical devices to different hardware as circumstances
//! change: a user plugged in headphones? The system default changed? SDL
//! can transparently migrate the logical devices to the correct physical
//! device seamlessly and keep playing; the app doesn't even have to know it
//! happened if it doesn't want to.
//!
//! ## Drivers
//!
//! The platform drivers are tried in upstream's order ([`audio_driver`]
//! lists them): on Linux "pulseaudio" and "alsa". Each loads its system library at run
//! time, so a missing library or sound server just moves on to the next
//! driver. "disk" and "dummy" are only used when requested with the
//! [`AUDIO_DRIVER`](crate::hints::AUDIO_DRIVER) hint (which may name
//! several drivers, separated by commas, to try in turn).

mod channel_converters;
mod convert;
pub(crate) mod device;
mod drivers;
mod format;
mod mixer;
mod queue;
mod resample;
mod stream;
mod typecvt;
mod wave;

pub use crate::events::AudioDeviceID;
pub use device::{
    audio_device_channel_map, audio_device_format, audio_device_gain, audio_device_name,
    audio_device_paused, audio_device_properties, audio_driver, bind_audio_stream,
    bind_audio_streams, close_audio_device, current_audio_driver, is_audio_device_physical,
    is_audio_device_playback, num_audio_drivers, open_audio_device, open_audio_device_stream,
    pause_audio_device, playback_devices, recording_devices, resume_audio_device,
    set_audio_device_gain, set_audio_postmix_callback, unbind_audio_stream, unbind_audio_streams,
    AudioDevice, AudioPostmixCallback, AUDIO_DEVICE_DEFAULT_PLAYBACK,
    AUDIO_DEVICE_DEFAULT_RECORDING, PROP_AUDIO_DEVICE_UNIQUE_ID_STRING,
};
pub use format::{
    define_audio_format, AudioFormat, AudioSpec, AUDIO_MASK_BIG_ENDIAN, AUDIO_MASK_BITSIZE,
    AUDIO_MASK_FLOAT, AUDIO_MASK_SIGNED,
};
pub use mixer::mix_audio;
pub use stream::{
    convert_audio_samples, AudioStream, AudioStreamCallback, AudioStreamLock,
    PROP_AUDIOSTREAM_AUTO_CLEANUP_BOOLEAN,
};
pub use wave::{load_wav, load_wav_io};

pub(crate) use device::{init_audio, quit_audio, update_audio};

#[cfg(test)]
mod tests;
