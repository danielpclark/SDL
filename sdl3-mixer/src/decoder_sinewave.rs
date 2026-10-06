// Rust translation of src/decoder_sinewave.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A sine wave generator.
//!
//! (this decoder is always enabled, since an external API uses it.)

// !!! FIXME:
// change track interface to provide the stream when seeking, then we could:
//   - Generate one whole iteration of the waveform upfront
//   - Push the same buffer twice with SDL_AudioStreamPutDataNoCopy
//   - decode just needs to see if one of the buffers is complete and push it again.
//   - clear/push a subset when seeking.

use std::sync::Arc;

use sdl3::audio::{AudioFormat, AudioSpec, AudioStream};
use sdl3::error::{Error, Result};
use sdl3::io::IoStream;
use sdl3::properties::Properties;
use sdl3::stdlib::math::{sinf, PI_F};
use sdl3::stdlib::string::strcasecmp;

use crate::internal::{
    put_f32, AudioData, Decoder, TrackData, PROP_DECODER_SINEWAVE_AMPLITUDE_FLOAT,
    PROP_DECODER_SINEWAVE_HZ_NUMBER, PROP_DECODER_SINEWAVE_MS_NUMBER,
};
use crate::mixer::ms_to_frames;
use crate::{DURATION_INFINITE, PROP_AUDIO_DECODER_STRING};

/// Translation of `SINEWAVE_AudioData`.
struct SinewaveAudioData {
    hz: i32,
    amplitude: f32,
    sample_rate: i32,
    total_frames: i64,
}

/// Translation of `SINEWAVE_TrackData`.
struct SinewaveTrackData {
    adata: Arc<SinewaveAudioData>,
    current_sine_sample: i32,
    position: i64,
}

/// Translation of `SINEWAVE_init_audio()`.
fn sinewave_init_audio(
    _io: Option<&mut IoStream<'_>>,
    spec: &mut AudioSpec,
    props: &Properties,
    duration_frames: &mut i64,
) -> Result<Arc<dyn AudioData>> {
    let decoder_name = props.get_string(PROP_AUDIO_DECODER_STRING);
    if decoder_name
        .as_deref()
        .is_none_or(|name| strcasecmp(name, "sinewave") != std::cmp::Ordering::Equal)
    {
        return Err(Error::new("Not SINEWAVE"));
    }

    let si64hz = props
        .get_number(PROP_DECODER_SINEWAVE_HZ_NUMBER)
        .unwrap_or(-1);
    let famp = props
        .get_float(PROP_DECODER_SINEWAVE_AMPLITUDE_FLOAT)
        .unwrap_or(-1.0);
    let ms = props
        .get_number(PROP_DECODER_SINEWAVE_MS_NUMBER)
        .unwrap_or(-1);

    if (si64hz <= 0) || (famp <= 0.0) {
        return Err(Error::new("Invalid sine wave"));
    }

    spec.format = AudioFormat::F32;
    spec.channels = 1;
    // we use the existing spec->freq to match the device sample rate, avoiding unnecessary resampling.

    let total_frames = if ms < 0 {
        DURATION_INFINITE
    } else {
        ms_to_frames(spec.freq, ms).unwrap_or(-1)
    };
    let adata = SinewaveAudioData {
        hz: si64hz as i32,
        amplitude: famp,
        sample_rate: spec.freq,
        total_frames,
    };

    *duration_frames = adata.total_frames;

    Ok(Arc::new(adata))
}

impl AudioData for SinewaveAudioData {
    /// Translation of `SINEWAVE_init_track()`.
    fn init_track<'a>(
        self: Arc<Self>,
        _io: Option<IoStream<'a>>,
        _spec: &AudioSpec,
        _props: &Properties,
    ) -> Result<Box<dyn TrackData + 'a>> {
        Ok(Box::new(SinewaveTrackData {
            adata: self,
            current_sine_sample: 0,
            position: 0,
        }))
    }
}

impl TrackData for SinewaveTrackData {
    /// Translation of `SINEWAVE_decode()`.
    fn decode(&mut self, stream: &AudioStream) -> bool {
        let adata = &self.adata;
        let sample_rate = adata.sample_rate;
        let fsample_rate = sample_rate as f32;
        let hz = adata.hz;
        let amplitude = adata.amplitude;
        let mut current_sine_sample = self.current_sine_sample;
        let mut samples = [0.0f32; 256];
        let infinite_sine = adata.total_frames < 0;
        let total_frames: i64 = if infinite_sine {
            samples.len() as i64
        } else {
            (adata.total_frames - self.position).min(samples.len() as i64)
        };

        if total_frames <= 0 {
            return false;
        }

        for sample in samples.iter_mut().take(total_frames as usize) {
            let phase = current_sine_sample.wrapping_mul(hz) as f32 / fsample_rate;
            *sample = sinf(phase * 2.0 * PI_F) * amplitude;
            current_sine_sample += 1;
        }

        // wrapping around to avoid floating-point errors
        self.current_sine_sample = if sample_rate != 0 {
            current_sine_sample % sample_rate
        } else {
            current_sine_sample
        };

        if !infinite_sine {
            self.position += total_frames;
        }

        // FIXME (upstream): this pushes the whole buffer, past the last
        // frame of a finite wave too; upstream's samples there are
        // uninitialized, here they're silence.
        let _ = put_f32(stream, &samples);

        true
    }

    /// Translation of `SINEWAVE_seek()`.
    fn seek(&mut self, frame: u64) -> Result<()> {
        let adata = &self.adata;
        if adata.total_frames >= 0 {
            if frame > adata.total_frames as u64 {
                return Err(Error::new("Past end of sinewave"));
            }
            self.position = frame as i64;
        }
        self.current_sine_sample = if adata.sample_rate > 0 {
            (frame % adata.sample_rate as u64) as i32
        } else {
            0
        };
        Ok(())
    }
}

/// Translation of `MIX_Decoder_SINEWAVE`.
pub(crate) static DECODER: Decoder = Decoder {
    name: "SINEWAVE",
    init: None,
    init_audio: sinewave_init_audio,
    has_jump_to_order: false,
    quit: None,
};
