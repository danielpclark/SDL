// Rust translation of src/decoder_aiff.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

/*
  This is the source needed to decode an AIFF file into a waveform.
  It's pretty straightforward once you get going.

  This file by Torbjörn Andersson (torbjorn.andersson@eurotime.se)
  8SVX file support added by Marc Le Douarain (mavati@club-internet.fr)
  in december 2002.
*/

//! AIFF and AIFF-C files: PCM, mu-law, a-law and float.

// (This code is originally from SDL2_mixer, heavily modified here.)

use std::sync::Arc;

use sdl3::audio::{AudioFormat, AudioSpec, AudioStream};
use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;

use crate::internal::{borrow_io, try_alloc, AudioData, Decoder, IoClamp, TrackData};
use crate::metadata_tags::{cstring, read_metadata_tags};
use crate::mixer::{ALAW_TO_FLOAT, ULAW_TO_FLOAT};
use crate::{
    PROP_METADATA_ARTIST_STRING, PROP_METADATA_COPYRIGHT_STRING, PROP_METADATA_TITLE_STRING,
};

/*********************************************/
/* Define values for AIFF (IFF audio) format */
/*********************************************/
const FORM: u32 = 0x4d524f46; /* "FORM" */
const AIFF: u32 = 0x46464941; /* "AIFF" */
const AIFC: u32 = 0x43464941; /* "AIFC" */
const FVER: u32 = 0x52455646; /* "FVER" */
const SSND: u32 = 0x444e5353; /* "SSND" */
const COMM: u32 = 0x4d4d4f43; /* "COMM" */
const AIFF_ID3_: u32 = 0x20334449; /* "ID3 " */
#[allow(dead_code)]
const MARK: u32 = 0x4B52414D; /* "MARK" */
#[allow(dead_code)]
const INST: u32 = 0x54534E49; /* "INST" */
const AUTH: u32 = 0x48545541; /* "AUTH" */
const NAME: u32 = 0x454D414E; /* "NAME" */
const C___: u32 = 0x20296328; /* "(c) " */
const ANNO: u32 = 0x4F4E4E41; /* "ANNO" */

/* Supported compression types */
const NONE: u32 = 0x454E4F4E; /* "NONE" */
const SOWT: u32 = 0x74776F73; /* "sowt" */
const RAW_: u32 = 0x20776172; /* "raw " */
const ULAW_LOWER: u32 = 0x77616C75; /* "ulaw" */
const ALAW_LOWER: u32 = 0x77616C61; /* "alaw" */
const ULAW_UPPER: u32 = 0x57414C55; /* "ULAW" */
const ALAW_UPPER: u32 = 0x57414C41; /* "ALAW" */
const FL32_LOWER: u32 = 0x32336C66; /* "fl32" */
const FL64_LOWER: u32 = 0x34366C66; /* "fl64" */
const FL32_UPPER: u32 = 0x32334C46; /* "FL32" */

/// Translation of `AIFF_FetchFn`'s choices.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Fetch {
    #[default]
    Pcm,
    ULaw,
    ALaw,
    Pcm24Le,
    Pcm24Be,
    Float64Be,
}

/// Translation of `AIFF_AudioData`.
#[derive(Clone, Copy, Default)]
struct AiffAudioData {
    start: i64,
    stop: i64,
    framesize: i32,
    decoded_framesize: i32,
    fetch: Fetch,
    num_pcm_frames: i64,
}

/// Translation of `AIFF_TrackData`.
struct AiffTrackData<'a> {
    adata: Arc<AiffAudioData>,
    io: IoStream<'a>,
}

impl AiffTrackData<'_> {
    /// Translation of `FetchXLaw()`.
    fn fetch_xlaw(&mut self, buffer: &mut [u8], lut: &[f32; 256]) -> i32 {
        let framesize = self.adata.framesize.max(1) as usize;
        let n = buffer.len() / 4;
        let mut length = self.io.read(&mut buffer[..n]);
        if length % framesize != 0 {
            length -= length % framesize;
        }
        // FIXME (upstream): this writes through a pointer to the last float
        // (`&buffer[(length - 1) * 4]`) indexed by `i` again, which writes
        // past the buffer; it's meant to be the buffer itself, as here (the
        // C reference harness is patched the same way).
        for i in (0..length).rev() {
            let v = lut[buffer[i] as usize];
            buffer[i * 4..i * 4 + 4].copy_from_slice(&v.to_ne_bytes());
        }
        (length * 4) as i32
    }

    /// Translation of `FetchPCM()`.
    fn fetch_pcm(&mut self, buffer: &mut [u8]) -> i32 {
        self.io.read(buffer) as i32
    }

    /// Translation of `FetchPCM24LE()` and `FetchPCM24BE()`.
    fn fetch_pcm24(&mut self, buffer: &mut [u8], big_endian: bool) -> i32 {
        let framesize = self.adata.framesize.max(1) as usize;
        let n = (buffer.len() / 4) * 3;
        let mut length = self.io.read(&mut buffer[..n]);
        if (length % framesize) != 0 {
            length -= length % framesize;
        }
        let mut i = length as isize - 3;
        let mut o = ((length as isize - 3) / 3) * 4;
        while i >= 0 {
            let x = &buffer[i as usize..i as usize + 3];
            let inp = if big_endian {
                ((x[0] as i8 as i32) << 16) | ((x[1] as i32) << 8) | x[2] as i32
            } else {
                ((x[2] as i8 as i32) << 16) | ((x[1] as i32) << 8) | x[0] as i32
            };
            let out = (inp as f32) / 8388608.0;
            buffer[o as usize..o as usize + 4].copy_from_slice(&out.to_ne_bytes());
            i -= 3;
            o -= 4;
        }
        ((length / 3) * 4) as i32
    }

    /// Translation of `FetchFloat64BE()`.
    fn fetch_float64be(&mut self, buffer: &mut [u8]) -> i32 {
        let framesize = self.adata.framesize.max(1) as usize;
        let mut length = self.io.read(buffer);
        if length % framesize != 0 {
            length -= length % framesize;
        }
        let mut o = 0;
        for i in (0..length / 8 * 8).step_by(8) {
            let mut d = [0u8; 8];
            d.copy_from_slice(&buffer[i..i + 8]);
            let out = f64::from_be_bytes(d) as f32;
            buffer[o * 4..o * 4 + 4].copy_from_slice(&out.to_ne_bytes());
            o += 1;
        }
        (length / 2) as i32
    }

    fn fetch(&mut self, buffer: &mut [u8]) -> i32 {
        match self.adata.fetch {
            Fetch::Pcm => self.fetch_pcm(buffer),
            Fetch::ULaw => self.fetch_xlaw(buffer, &ULAW_TO_FLOAT),
            Fetch::ALaw => self.fetch_xlaw(buffer, &ALAW_TO_FLOAT),
            Fetch::Pcm24Le => self.fetch_pcm24(buffer, false),
            Fetch::Pcm24Be => self.fetch_pcm24(buffer, true),
            Fetch::Float64Be => self.fetch_float64be(buffer),
        }
    }
}

// I couldn't get SANE_to_double() to work, so I stole this from libsndfile.
// I don't pretend to fully understand it.
/// Translation of `SANE_to_Uint32()`.
fn sane_to_uint32(sanebuf: &[u8; 10]) -> u32 {
    let sb0 = sanebuf[0];
    let sb1 = sanebuf[1];
    if sb0 & 0x80 != 0 {
        // Negative number?
        return 0;
    } else if sb0 <= 0x3F {
        // Less than 1?
        return 1;
    } else if sb0 > 0x40 {
        // Way too big?
        return 0x4000000;
    } else if (sb0 == 0x40) && (sb1 > 0x1C) {
        // Still too big?
        return 800000000;
    }
    let v = ((sanebuf[2] as i32).wrapping_shl(23))
        | ((sanebuf[3] as i32) << 15)
        | ((sanebuf[4] as i32) << 7)
        | ((sanebuf[5] as i32) >> 1);
    (v >> (29 - sb1 as i32)) as u32
}

/// Translation of `ParseAIFFID3()`.
fn parse_aiff_id3(io: &mut IoStream<'_>, props: &Properties, chunk_length: u32) -> Result<()> {
    let mut clamp = IoClamp::open(borrow_io(io))?;
    clamp.length = chunk_length as i64;
    read_metadata_tags(&mut clamp, props);
    Ok(())
}

/// Translation of `CheckAIFFMetadataField()`.
fn check_aiff_metadata_field(
    propname: Option<&str>,
    io: &mut IoStream<'_>,
    props: &Properties,
    chunk_type: u32,
    chunk_length: u32,
    dupcount: i32,
) -> Result<()> {
    let mut chunk_buffer = try_alloc::<u8>(chunk_length as usize + 1)?;
    if io.read(&mut chunk_buffer[..chunk_length as usize]) != chunk_length as usize {
        return Err(Error::new("AIFF: Couldn't read metadata chunk"));
    }

    chunk_buffer[chunk_length as usize] = b'\0';

    let dupstr = if dupcount >= 0 {
        format!("{dupcount}")
    } else {
        String::new()
    };
    let mut key = b"SDL_mixer.metadata.aiff.".to_vec();
    key.extend_from_slice(&chunk_type.to_le_bytes());
    key.extend_from_slice(dupstr.as_bytes());
    let value = cstring(&chunk_buffer);
    let _ = props.set(&cstring(&key), value.clone());
    if let Some(propname) = propname {
        if !props.contains(propname) {
            let _ = props.set(propname, value);
        }
    }

    Ok(())
}

/// Translation of `AIFF_init_audio_internal()`.
fn aiff_init_audio_internal(
    adata: &mut AiffAudioData,
    io: &mut IoStream<'_>,
    spec: &mut AudioSpec,
    props: &Properties,
) -> Result<()> {
    let mut channels: u16 = 0;
    let mut numsamples: u32 = 0;
    let mut samplesize: u16 = 0;
    let mut frequency: u32 = 0;
    let mut compression_type: u32 = 0;
    let mut anno_count = 0;

    let flen = io.size().unwrap_or(-1);

    // Check the magic header
    let mut chunk_type = io.read_u32_le()?;
    let chunk_length = io.read_u32_be()?;
    if chunk_type != FORM {
        return Err(Error::new("AIFF: Unrecognized file type (not FORM chunk)"));
    } else if chunk_length as i64 > (flen - 8) {
        return Err(Error::new(
            "AIFF: Corrupt file (primary chunk larger than file)",
        ));
    }
    chunk_type = io.read_u32_le()?;
    if (chunk_type != AIFF) && (chunk_type != AIFC) {
        return Err(Error::new(
            "AIFF: Unrecognized file type (not AIFF or AIFC)",
        ));
    }

    let is_aifc = chunk_type == AIFC;

    // From what I understand of the specification, chunks may appear in
    // any order, and we should just ignore unknown ones.
    //
    // TODO: Better sanity-checking. E.g. what happens if the AIFF file contains compressed sound data?

    let mut found_ssnd = false;
    let mut found_comm = false;
    let mut found_fver = false;

    loop {
        let chunk_type = io.read_u32_le()?;
        let chunk_length = io.read_u32_be()?;

        let chunk_start_position = io.tell().unwrap_or(-1);
        let mut next_chunk = chunk_start_position + chunk_length as i64;
        if chunk_start_position < 0 {
            return Err(Error::new("Error seeking in datastream"));
        } else if next_chunk > flen {
            return Err(Error::new("AIFF: Corrupt AIFF file (chunk goes past EOF)"));
        }

        if chunk_length % 2 != 0 {
            next_chunk += 1; // pad to 16-bit word size.
        }

        match chunk_type {
            SSND => {
                found_ssnd = true;
                let offset = io.read_u32_be()?;
                let _blocksize = io.read_u32_be()?; // unused
                adata.start = io.tell().unwrap_or(-1) + offset as i64;
            }

            FVER => {
                found_fver = true;
                let _aifc_version1 = io.read_u32_be()?; // unused
            }

            AIFF_ID3_ => parse_aiff_id3(io, props, chunk_length)?,

            NAME => check_aiff_metadata_field(
                Some(PROP_METADATA_TITLE_STRING),
                io,
                props,
                chunk_type,
                chunk_length,
                -1,
            )?,

            AUTH => check_aiff_metadata_field(
                Some(PROP_METADATA_ARTIST_STRING),
                io,
                props,
                chunk_type,
                chunk_length,
                -1,
            )?,

            C___ => check_aiff_metadata_field(
                Some(PROP_METADATA_COPYRIGHT_STRING),
                io,
                props,
                chunk_type,
                chunk_length,
                -1,
            )?,

            ANNO => {
                check_aiff_metadata_field(None, io, props, chunk_type, chunk_length, anno_count)?;
                anno_count += 1;
            }

            COMM => {
                found_comm = true;

                // Read the audio data format chunk
                channels = io.read_u16_be()?;
                numsamples = io.read_u32_be()?;
                samplesize = io.read_u16_be()?;
                let mut sane_freq = [0u8; 10];
                if io.read(&mut sane_freq) != sane_freq.len() {
                    return Err(Error::new("AIFF: Couldn't read COMM chunk"));
                }
                frequency = sane_to_uint32(&sane_freq);
                if is_aifc {
                    compression_type = io.read_u32_le()?;
                    // here must be a "compressionName" which is a padded string
                }
            }

            _ => {} // Unknown/unsupported chunk: we just skip over it.
        }

        if !((next_chunk < flen) && io.seek(next_chunk, IoWhence::Set).is_ok()) {
            break;
        }
    }

    if !found_ssnd {
        return Err(Error::new("AIFF: Bad AIFF/AIFF-C file (no SSND chunk)"));
    } else if !found_comm {
        return Err(Error::new("AIFF: Bad AIFF/AIFF-C file (no COMM chunk)"));
    } else if is_aifc && !found_fver {
        return Err(Error::new("AIFF: Bad AIFF-C file (no FVER chunk)"));
    }

    adata.framesize = channels as i32 * (samplesize as i32 / 8);
    adata.stop = adata.start
        + ((channels as u32)
            .wrapping_mul(numsamples)
            .wrapping_mul((samplesize / 8) as u32)) as i64;
    adata.fetch = Fetch::Pcm;
    adata.num_pcm_frames = numsamples as i64;

    // Decode the audio data format
    spec.freq = frequency as i32;
    let mut unsupported_format = false;
    match samplesize {
        8 => {
            if !is_aifc {
                spec.format = AudioFormat::S8;
            } else {
                match compression_type {
                    RAW_ => spec.format = AudioFormat::U8,
                    SOWT => spec.format = AudioFormat::S8,
                    ULAW_LOWER => {
                        spec.format = AudioFormat::F32;
                        adata.fetch = Fetch::ULaw;
                    }
                    ALAW_LOWER => {
                        spec.format = AudioFormat::F32;
                        adata.fetch = Fetch::ALaw;
                    }
                    _ => unsupported_format = true,
                }
            }
        }
        16 => {
            if !is_aifc {
                spec.format = AudioFormat::S16BE;
            } else {
                match compression_type {
                    SOWT => spec.format = AudioFormat::S16LE,
                    NONE => spec.format = AudioFormat::S16BE,
                    ULAW_UPPER => {
                        spec.format = AudioFormat::F32;
                        adata.fetch = Fetch::ULaw;
                    }
                    ALAW_UPPER => {
                        spec.format = AudioFormat::F32;
                        adata.fetch = Fetch::ALaw;
                    }
                    _ => unsupported_format = true,
                }
            }
        }
        24 => {
            adata.fetch = Fetch::Pcm24Be;
            spec.format = AudioFormat::F32;
            if is_aifc {
                match compression_type {
                    SOWT => adata.fetch = Fetch::Pcm24Le,
                    NONE => {}
                    _ => unsupported_format = true,
                }
            }
        }
        32 => {
            if !is_aifc {
                spec.format = AudioFormat::S32BE;
            } else {
                match compression_type {
                    SOWT => spec.format = AudioFormat::S32LE,
                    NONE => spec.format = AudioFormat::S32BE,
                    FL32_LOWER | FL32_UPPER => spec.format = AudioFormat::F32BE,
                    _ => unsupported_format = true,
                }
            }
        }
        64 => {
            adata.fetch = Fetch::Float64Be;
            if !is_aifc {
                spec.format = AudioFormat::F32;
            } else {
                match compression_type {
                    FL64_LOWER => spec.format = AudioFormat::F32,
                    _ => unsupported_format = true,
                }
            }
        }
        _ => unsupported_format = true,
    }

    if unsupported_format {
        return Err(Error::new("AIFF: unsupported data format"));
    }

    // FIXME (upstream): zero channels (also 256, as the count is stored in
    // a byte) make a zero frame size, which upstream divides by (and
    // crashes) later.
    if channels as u8 == 0 {
        return Err(Error::new("AIFF: Invalid number of channels"));
    }

    spec.channels = channels as u8 as i32;
    adata.decoded_framesize = spec.frame_size() as i32;

    Ok(())
}

/// Translation of `AIFF_init_audio()`.
fn aiff_init_audio(
    io: Option<&mut IoStream<'_>>,
    spec: &mut AudioSpec,
    props: &Properties,
    duration_frames: &mut i64,
) -> Result<Arc<dyn AudioData>> {
    let io = io.ok_or_else(|| Error::invalid_param("io"))?;

    // quick rejection before we allocate anything.
    let mut aiffmagic = io.read_u32_le()?;
    if aiffmagic != FORM {
        return Err(Error::new("AIFF: Not an AIFF file"));
    }
    let _ = io.read_u32_be()?; // we don't care about the length of the FORM field here.
    aiffmagic = io.read_u32_le()?;
    if (aiffmagic != AIFF) && (aiffmagic != AIFC) {
        return Err(Error::new("AIFF: Not an AIFF file"));
    }
    io.seek(0, IoWhence::Set)?;

    let mut adata = AiffAudioData::default();

    aiff_init_audio_internal(&mut adata, io, spec, props)?;

    *duration_frames = adata.num_pcm_frames;

    Ok(Arc::new(adata))
}

impl AudioData for AiffAudioData {
    /// Translation of `AIFF_init_track()`.
    fn init_track<'a>(
        self: Arc<Self>,
        io: Option<IoStream<'a>>,
        _spec: &AudioSpec,
        _props: &Properties,
    ) -> Result<Box<dyn TrackData + 'a>> {
        let io = io.ok_or_else(|| Error::invalid_param("io"))?;
        Ok(Box::new(AiffTrackData { adata: self, io }))
    }
}

impl TrackData for AiffTrackData<'_> {
    /// Translation of `AIFF_decode()`.
    fn decode(&mut self, stream: &AudioStream) -> bool {
        let mut buffer = [0u8; 1024];
        let mut buflen = buffer.len() as i32;
        let m = buflen % self.adata.decoded_framesize.max(1);
        if m != 0 {
            buflen -= m;
        }
        let br = self.fetch(&mut buffer[..buflen as usize]); // this will deal with different formats that might need decompression or conversion.
        if br <= 0 {
            return false;
        }

        let _ = stream.put_data(&buffer[..br as usize]);
        true
    }

    /// Translation of `AIFF_seek()`.
    fn seek(&mut self, frame: u64) -> Result<()> {
        let adata = &self.adata;
        let dest_offset = (frame as i64).wrapping_mul(adata.framesize as i64);
        let destpos = adata.start.wrapping_add(dest_offset);
        if destpos > adata.stop {
            return Err(Error::new("AIFF: seek past end of data"));
        }
        self.io.seek(destpos, IoWhence::Set)?;

        Ok(())
    }
}

/// Translation of `MIX_Decoder_AIFF`.
pub(crate) static DECODER: Decoder = Decoder {
    name: "AIFF",
    init: None,
    init_audio: aiff_init_audio,
    has_jump_to_order: false,
    quit: None,
};
