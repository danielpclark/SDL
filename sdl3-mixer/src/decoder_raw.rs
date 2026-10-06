// Rust translation of src/decoder_raw.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Raw PCM data, in a format the app names.
//!
//! (this decoder is always enabled, as it is used internally.)
//!
//! Upstream reads straight from the memory behind a memory stream
//! (`MIX_GetConstIOBuffer()`), pushing it without a copy; a stream here
//! doesn't expose its memory, so this always reads through the stream.
//! The audio pushed is the same: Note (upstream): only a memory stream that
//! wasn't at its start when the track began differs, as upstream plays it
//! from the start of the memory regardless.

use std::sync::Arc;

use sdl3::audio::{AudioFormat, AudioSpec, AudioStream};
use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;
use sdl3::stdlib::string::strcasecmp;

use crate::internal::{
    io_size, AudioData, Decoder, TrackData, PROP_DECODER_CHANNELS_NUMBER,
    PROP_DECODER_FORMAT_NUMBER, PROP_DECODER_FREQ_NUMBER,
};
use crate::{DURATION_UNKNOWN, PROP_AUDIO_DECODER_STRING};

// !!! FIXME: change track interface to provide the stream when seeking, and a means to see if we're backed by a memory SDL_IOStream, then we could use SDL_AudioStreamPutDataNoCopy to push the whole buffer upfront for free, and clear/push a subset when seeking.

/// Translation of `RAW_TrackData`.
struct RawTrackData<'a> {
    io: IoStream<'a>,
    framesize: usize,
    position: i64,
}

/// The RAW decoder has no audio state (`audio_userdata` is `NULL`).
struct RawAudioData;

/// The RAW decoder's (empty) audio state, for predecoded audio.
pub(crate) fn raw_audio_data() -> Arc<dyn AudioData> {
    Arc::new(RawAudioData)
}

/// Translation of `RAW_init_audio()`.
fn raw_init_audio(
    io: Option<&mut IoStream<'_>>,
    spec: &mut AudioSpec,
    props: &Properties,
    duration_frames: &mut i64,
) -> Result<Arc<dyn AudioData>> {
    let decoder_name = props.get_string(PROP_AUDIO_DECODER_STRING);
    if decoder_name
        .as_deref()
        .is_none_or(|name| strcasecmp(name, "raw") != std::cmp::Ordering::Equal)
    {
        return Err(Error::new("Not RAW"));
    }

    let si64fmt = props.get_number(PROP_DECODER_FORMAT_NUMBER).unwrap_or(-1);
    let si64channels = props.get_number(PROP_DECODER_CHANNELS_NUMBER).unwrap_or(-1);
    let si64freq = props.get_number(PROP_DECODER_FREQ_NUMBER).unwrap_or(-1);

    if (si64fmt <= 0) || (si64channels <= 0) || (si64freq <= 0) {
        return Err(Error::new(
            "Requested RAW decoder but didn't provide PCM format information",
        ));
    }

    spec.format = AudioFormat(si64fmt as u32);
    spec.channels = si64channels as i32;
    spec.freq = si64freq as i32;

    // we don't have to inspect the data, we treat anything as valid.

    let framesize = spec.frame_size() as i64;
    let iolen = match io {
        Some(io) => io_size(io),
        None => -1,
    };

    // (an unknown audio format has no frame size; upstream divides by zero.)
    *duration_frames = if iolen >= 0 && framesize > 0 {
        iolen / framesize
    } else {
        DURATION_UNKNOWN
    };

    Ok(Arc::new(RawAudioData)) // no state.
}

impl AudioData for RawAudioData {
    /// Translation of `RAW_init_track()`.
    fn init_track<'a>(
        self: Arc<Self>,
        io: Option<IoStream<'a>>,
        spec: &AudioSpec,
        _props: &Properties,
    ) -> Result<Box<dyn TrackData + 'a>> {
        let io = io.ok_or_else(|| Error::invalid_param("io"))?;
        Ok(Box::new(RawTrackData {
            io,
            framesize: spec.frame_size().max(1),
            position: 0,
        }))
    }
}

impl TrackData for RawTrackData<'_> {
    /// Translation of `RAW_decode()`.
    fn decode(&mut self, stream: &AudioStream) -> bool {
        let mut buffer = [0u8; 256];

        let readlen = buffer.len() - (buffer.len() % self.framesize);
        let mut br = self.io.read(&mut buffer[..readlen]);
        br -= br % self.framesize;
        if br == 0 {
            return false; // eof or error, can't supply more data.
        }
        let _ = stream.put_data(&buffer[..br]);
        self.position += br as i64;

        true
    }

    /// Translation of `RAW_seek()`.
    fn seek(&mut self, frame: u64) -> Result<()> {
        let offset = frame.wrapping_mul(self.framesize as u64) as i64;
        if self.io.seek(offset, IoWhence::Set)? != offset {
            return Err(Error::new("Seek past end of data"));
        }
        self.position = offset;
        Ok(())
    }
}

/// Translation of `MIX_Decoder_RAW`.
pub(crate) static DECODER: Decoder = Decoder {
    name: "RAW",
    init: None,
    init_audio: raw_init_audio,
    has_jump_to_order: false,
    quit: None,
};
