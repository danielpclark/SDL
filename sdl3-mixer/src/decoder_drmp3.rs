// Rust translation of src/decoder_drmp3.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! MP3 decoder, using dr_mp3 (see `dr_mp3.rs`).

// !!! FIXME: we need a DRMP3_NO_PARSE_METADATA_TAGS option to remove the ID3/APE checks, since we filtered them elsewhere.

use std::sync::Arc;

use sdl3::audio::{AudioFormat, AudioSpec, AudioStream};
use sdl3::error::{Error, Result};
use sdl3::io::IoStream;
use sdl3::properties::Properties;

use crate::dr_mp3::{Drmp3, Drmp3SeekPoint};
use crate::internal::{borrow_io, put_f32, try_alloc, AudioData, Decoder, TrackData};

/// Translation of `DRMP3_AudioData`.
struct Drmp3AudioData {
    framesize: usize,
    seek_points: Arc<[Drmp3SeekPoint]>,
}

/// Translation of `DRMP3_TrackData`.
struct Drmp3TrackData<'a> {
    adata: Arc<Drmp3AudioData>,
    decoder: Drmp3<'a>,
}

/// Translation of `DRMP3_init_audio()`.
fn drmp3_init_audio(
    io: Option<&mut IoStream<'_>>,
    spec: &mut AudioSpec,
    _props: &Properties,
    duration_frames: &mut i64,
) -> Result<Arc<dyn AudioData>> {
    let io = io.ok_or_else(|| Error::invalid_param("io"))?;

    // do an initial load from the IOStream to get metadata.
    // (upstream doesn't set an error message here.)
    let Some(mut decoder) = Drmp3::init(borrow_io(io)) else {
        return Err(Error::new("DRMP3: not an MP3 file")); // probably not an MP3 file.
    };

    // I don't know if this is a great idea, as this is allegedly inefficient, but let's precalculate a seek table at load time, so each track can reuse it.
    // (If any of this fails, we go on without it.)
    let mut seek_points: Arc<[Drmp3SeekPoint]> = Vec::new().into();
    let mut num_pcm_frames: u64 = 0;
    if let Some((num_mp3_frames, pcm_frames)) = decoder.get_mp3_and_pcm_frame_count() {
        num_pcm_frames = pcm_frames;
        let mut num_seek_points = num_mp3_frames as u32;
        if let Ok(mut points) = try_alloc::<Drmp3SeekPoint>(num_mp3_frames as usize) {
            if decoder.calculate_seek_points(&mut num_seek_points, &mut points) {
                // shrink the array if possible.
                points.truncate(num_seek_points as usize);
                seek_points = points.into();
            } // else failed, oh well. Live without.
        }
    }

    spec.format = AudioFormat::F32;
    spec.channels = decoder.channels as i32;
    spec.freq = decoder.sample_rate as i32;

    drop(decoder);

    let framesize = spec.format.bytesize() as usize * spec.channels as usize;

    *duration_frames = num_pcm_frames as i64;

    Ok(Arc::new(Drmp3AudioData {
        framesize,
        seek_points,
    }))
}

impl AudioData for Drmp3AudioData {
    /// Translation of `DRMP3_init_track()`.
    fn init_track<'a>(
        self: Arc<Self>,
        io: Option<IoStream<'a>>,
        _spec: &AudioSpec,
        _props: &Properties,
    ) -> Result<Box<dyn TrackData + 'a>> {
        let io = io.ok_or_else(|| Error::invalid_param("io"))?;
        let Some(mut decoder) = Drmp3::init(io) else {
            return Err(Error::new("DRMP3: not an MP3 file"));
        };

        if !self.seek_points.is_empty() {
            decoder.bind_seek_table(self.seek_points.clone());
        }

        Ok(Box::new(Drmp3TrackData {
            adata: self,
            decoder,
        }))
    }
}

impl TrackData for Drmp3TrackData<'_> {
    /// Translation of `DRMP3_decode()`.
    fn decode(&mut self, stream: &AudioStream) -> bool {
        let framesize = self.adata.framesize;
        let mut samples = [0.0f32; 256];
        let rc = self.decoder.read_pcm_frames_f32(
            (std::mem::size_of_val(&samples) / framesize) as u64,
            Some(&mut samples),
        );
        if rc == 0 {
            return false; // done decoding.
        }
        let _ = put_f32(stream, &samples[..rc as usize * framesize / 4]);
        true
    }

    /// Translation of `DRMP3_seek()`.
    fn seek(&mut self, frame: u64) -> Result<()> {
        if self.decoder.seek_to_pcm_frame(frame) {
            Ok(())
        } else {
            Err(Error::new("DRMP3: seek failed")) // (upstream doesn't set an error message.)
        }
    }
}

/// Translation of `MIX_Decoder_DRMP3`.
pub(crate) static DECODER: Decoder = Decoder {
    name: "DRMP3",
    init: None,
    init_audio: drmp3_init_audio,
    has_jump_to_order: false,
    quit: None,
};
