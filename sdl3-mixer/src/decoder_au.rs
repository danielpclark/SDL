// Rust translation of src/decoder_au.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

// This code originally came from SDL_sound, and was originally written by
// Mattias Engdegård. It's been heavily modified for SDL3_mixer, so don't
// bother Mattias about bugs, they're probably not his fault.  :)

//! Sun/NeXT .au decoder for SDL_sound.
//! Formats supported: 8 and 16 bit linear PCM, 8 bit mu-law.
//! Files without valid header are assumed to be 8 bit mu-law, 8kHz, mono.

use std::sync::Arc;

use sdl3::audio::{AudioFormat, AudioSpec, AudioStream};
use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;
use sdl3::stdlib::string::strcasecmp;

use crate::internal::{AudioData, Decoder, TrackData, PROP_AUDIO_LOAD_PATH_STRING};
use crate::mixer::ULAW_TO_FLOAT;
use crate::PROP_AUDIO_DECODER_STRING;

const AU_MAGIC: u32 = 0x2E736E64; // ".snd", in ASCII (bigendian number)

/// Translation of `AU_file_hdr`.
#[derive(Clone, Copy, Default)]
struct AuFileHdr {
    magic: u32,
    hdr_size: u32,
    data_size: u32,
    encoding: u32,
    sample_rate: u32,
    channels: u32,
}

// Translation of `AU_Encoding`.
const AU_ENC_ULAW_8: u32 = 1; // 8-bit ISDN mu-law
const AU_ENC_LINEAR_8: u32 = 2; // 8-bit linear PCM
const AU_ENC_LINEAR_16: u32 = 3; // 16-bit linear PCM
                                 // the rest are unsupported (I have never seen them in the wild)
                                 // AU_ENC_LINEAR_24 = 4, AU_ENC_LINEAR_32 = 5, AU_ENC_FLOAT = 6,
                                 // AU_ENC_DOUBLE = 7; more Sun formats, not supported either:
                                 // AU_ENC_ADPCM_G721 = 23, AU_ENC_ADPCM_G722 = 24,
                                 // AU_ENC_ADPCM_G723_3 = 25, AU_ENC_ADPCM_G723_5 = 26, AU_ENC_ALAW_8 = 27

/// Translation of `AU_AudioData`.
#[derive(Clone, Copy, Default)]
struct AuAudioData {
    start_offset: u32,
    framesize: i32, // encoded frame size! So ulaw will produce float32 samples, but the framesize is 1 byte * channels.
    encoding: u32,
}

/// Translation of `AU_TrackData`.
struct AuTrackData<'a> {
    adata: Arc<AuAudioData>,
    io: IoStream<'a>,
}

// Read in the AU header from disk. This makes this process safe
//  regardless of the processor's byte order or how the AU_file_hdr
//  structure is packed.
/// Translation of `ReadAUHeader()`.
fn read_au_header(io: &mut IoStream<'_>) -> Result<AuFileHdr> {
    Ok(AuFileHdr {
        magic: io.read_u32_be()?,
        hdr_size: io.read_u32_be()?,
        data_size: io.read_u32_be()?,
        encoding: io.read_u32_be()?,
        sample_rate: io.read_u32_be()?,
        channels: io.read_u32_be()?,
    })
}

/// Translation of `AU_init_audio()`.
fn au_init_audio(
    io: Option<&mut IoStream<'_>>,
    spec: &mut AudioSpec,
    props: &Properties,
    duration_frames: &mut i64,
) -> Result<Arc<dyn AudioData>> {
    let io = io.ok_or_else(|| Error::invalid_param("io"))?;
    let mut adata = AuAudioData::default();

    // ReadAUHeader() will do byte order swapping.
    let mut hdr = read_au_header(io).map_err(|_| Error::new("AU: bad header"))?;

    let mut total_frames: u32;
    if hdr.magic == AU_MAGIC {
        // valid magic?
        match hdr.encoding {
            AU_ENC_ULAW_8 => {
                spec.format = AudioFormat::F32; // might as well go straight to float...
                total_frames = hdr.data_size;
                adata.framesize = 1;
            }
            AU_ENC_LINEAR_8 => {
                spec.format = AudioFormat::S8;
                total_frames = hdr.data_size;
                adata.framesize = 1;
            }
            AU_ENC_LINEAR_16 => {
                spec.format = AudioFormat::S16BE;
                total_frames = hdr.data_size / 2;
                adata.framesize = 2;
            }
            _ => return Err(Error::new("AU: Unsupported .au encoding")),
        }

        // FIXME (upstream): zero channels divide by zero (and crash) here.
        if hdr.channels == 0 {
            return Err(Error::new("AU: Invalid number of channels"));
        }

        spec.freq = hdr.sample_rate as i32;
        spec.channels = hdr.channels as i32;

        adata.encoding = hdr.encoding;
        adata.framesize = adata.framesize.wrapping_mul(hdr.channels as i32);
        total_frames /= hdr.channels;
        adata.start_offset = hdr.hdr_size;
    } else {
        // A number of files in the wild have the .au extension but no valid
        // header; these are traditionally assumed to be 8kHz mu-law. Handle
        // them here only if the extension is recognized or this decoder was
        // explicitly requested.
        let mut assume_au_data = false;
        let decoder_name = props.get_string(PROP_AUDIO_DECODER_STRING);
        if decoder_name
            .as_deref()
            .is_some_and(|name| strcasecmp(name, "au") == std::cmp::Ordering::Equal)
        {
            assume_au_data = true;
        } else if let Some(origpath) = props.get_string(PROP_AUDIO_LOAD_PATH_STRING) {
            if let Some(dot) = origpath.rfind('.') {
                assume_au_data =
                    strcasecmp(&origpath[dot + 1..], "au") == std::cmp::Ordering::Equal;
            }
        }

        if !assume_au_data {
            return Err(Error::new("AU: Not .au audio"));
        }

        hdr = AuFileHdr::default();

        // FIXME (upstream): this says signed 16-bit, but the data is
        // decoded as mu-law to floats.
        spec.format = AudioFormat::S16;
        spec.freq = 8000;
        spec.channels = 1;

        total_frames = io.size().unwrap_or(-1) as u32;
        adata.start_offset = 0;
        adata.framesize = 1;
        adata.encoding = AU_ENC_ULAW_8;
    }

    // skip remaining part of header
    io.seek(hdr.hdr_size as i64, IoWhence::Set)?;

    *duration_frames = total_frames as i64;

    Ok(Arc::new(adata))
}

impl AudioData for AuAudioData {
    /// Translation of `AU_init_track()`.
    fn init_track<'a>(
        self: Arc<Self>,
        io: Option<IoStream<'a>>,
        _spec: &AudioSpec,
        _props: &Properties,
    ) -> Result<Box<dyn TrackData + 'a>> {
        let io = io.ok_or_else(|| Error::invalid_param("io"))?;
        Ok(Box::new(AuTrackData { adata: self, io }))
    }
}

impl TrackData for AuTrackData<'_> {
    /// Translation of `AU_decode()`.
    fn decode(&mut self, stream: &AudioStream) -> bool {
        const MAX_SAMPS: usize = 512;

        // (a frame size that overflowed with a huge channel count can't be
        // read.)
        let framesize = self.adata.framesize;
        if framesize <= 0 {
            return false;
        }
        let framesize = framesize as usize;

        match self.adata.encoding {
            AU_ENC_ULAW_8 => {
                let mut buffer = [0.0f32; MAX_SAMPS];
                let max_read = MAX_SAMPS - (MAX_SAMPS % framesize);
                let mut ulaw_buf = [0u8; MAX_SAMPS];
                let mut br = self.io.read(&mut ulaw_buf[..max_read]);
                br -= br % framesize;
                if br == 0 {
                    return false; // nothing else to read.
                }
                for i in 0..br {
                    buffer[i] = ULAW_TO_FLOAT[ulaw_buf[i] as usize];
                }
                let _ = crate::internal::put_f32(stream, &buffer[..br]);
                true
            }

            AU_ENC_LINEAR_8 | AU_ENC_LINEAR_16 => {
                let mut buffer = [0u8; MAX_SAMPS * 2];
                // (upstream computes `max_read` and then doesn't use it.)
                let mut br = self
                    .io
                    .read(&mut buffer[..(MAX_SAMPS * 2) - ((MAX_SAMPS * 2) % framesize)]);
                br -= br % framesize;
                if br == 0 {
                    return false; // nothing else to read.
                }
                let _ = stream.put_data(&buffer[..br]);
                true
            }

            _ => {
                debug_assert!(false, "Unexpected AU encoding!");
                false
            }
        }
    }

    /// Translation of `AU_seek()`.
    fn seek(&mut self, frame: u64) -> Result<()> {
        let adata = &self.adata;
        let pos = (adata.start_offset as u64)
            .wrapping_add(frame.wrapping_mul(adata.framesize as i64 as u64))
            as i64;
        self.io.seek(pos, IoWhence::Set)?;
        Ok(())
    }
}

/// Translation of `MIX_Decoder_AU`.
pub(crate) static DECODER: Decoder = Decoder {
    name: "AU",
    init: None,
    init_audio: au_init_audio,
    has_jump_to_order: false,
    quit: None,
};
