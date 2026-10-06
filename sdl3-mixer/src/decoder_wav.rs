// Rust translation of src/decoder_wav.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! WAVE files: PCM, IEEE float, mu-law and a-law, MS and IMA ADPCM, with
//! `smpl` loops and `LIST`/`id3 ` metadata.

// this is originally SDL2_mixer's music_wav.c, which was probably
// SDL's SDL_wave.c at some point. It's been heavily modified for this.

// !!! FIXME: some of this is duplicated in decoder_aiff.c; if we end up
// !!! FIXME: supporting more formats that need it, we should generalize it.

/*
    Taken with permission from SDL_wave.h, part of the SDL library,
    available at: http://www.libsdl.org/
    and placed under the same license as this mixer library.
*/

use std::sync::Arc;

use sdl3::audio::{AudioFormat, AudioSpec, AudioStream};
use sdl3::error::{Error, Result};
use sdl3::io::{IoStatus, IoStream, IoWhence};
use sdl3::properties::Properties;

use crate::internal::{borrow_io, try_alloc, AudioData, Decoder, IoClamp, TrackData};
use crate::metadata_tags::{cstr, cstring, read_metadata_tags};
use crate::mixer::{ALAW_TO_FLOAT, ULAW_TO_FLOAT};
use crate::{
    DURATION_INFINITE, PROP_AUDIO_LOAD_IGNORE_LOOPS_BOOLEAN, PROP_METADATA_ALBUM_STRING,
    PROP_METADATA_ARTIST_STRING, PROP_METADATA_COPYRIGHT_STRING, PROP_METADATA_TITLE_STRING,
};

/* WAVE files are little-endian */

/*******************************************/
/* Define values for Microsoft WAVE format */
/*******************************************/
const RIFF: u32 = 0x46464952; /* "RIFF" */
const WAVE: u32 = 0x45564157; /* "WAVE" */
const FMT: u32 = 0x20746D66; /* "fmt " */
const DATA: u32 = 0x61746164; /* "data" */
const SMPL: u32 = 0x6c706d73; /* "smpl" */
const LIST: u32 = 0x5453494c; /* "LIST" */
const ID3X_LOWER: u32 = 0x20336469; /* "id3 " */
const ID3X_UPPER: u32 = 0x20334449; /* "ID3 " */
#[allow(dead_code)]
const UNKNOWN_CODE: u16 = 0x0000;
const PCM_CODE: u16 = 0x0001; /* WAVE_FORMAT_PCM */
const MS_ADPCM_CODE: u16 = 0x0002; /* WAVE_FORMAT_ADPCM */
const IEEE_FLOAT_CODE: u16 = 0x0003; /* WAVE_FORMAT_IEEE_FLOAT */
const ALAW_CODE: u16 = 0x0006; /* WAVE_FORMAT_ALAW */
const MULAW_CODE: u16 = 0x0007; /* WAVE_FORMAT_MULAW */
const IMA_ADPCM_CODE: u16 = 0x0011;
#[allow(dead_code)]
const MPEG_CODE: u16 = 0x0050;
#[allow(dead_code)]
const MPEGLAYER3_CODE: u16 = 0x0055;
const EXTENSIBLE_CODE: u16 = 0xFFFE;

// channel mask bits in WAV file
const WAV_SPEAKER_FRONT_LEFT: u32 = 1 << 0;
const WAV_SPEAKER_FRONT_RIGHT: u32 = 1 << 1;
const WAV_SPEAKER_FRONT_CENTER: u32 = 1 << 2;
const WAV_SPEAKER_LOW_FREQUENCY: u32 = 1 << 3;
const WAV_SPEAKER_BACK_LEFT: u32 = 1 << 4;
const WAV_SPEAKER_BACK_RIGHT: u32 = 1 << 5;
const WAV_SPEAKER_FRONT_LEFT_OF_CENTER: u32 = 1 << 6;
const WAV_SPEAKER_FRONT_RIGHT_OF_CENTER: u32 = 1 << 7;
const WAV_SPEAKER_BACK_CENTER: u32 = 1 << 8;
const WAV_SPEAKER_SIDE_LEFT: u32 = 1 << 9;
const WAV_SPEAKER_SIDE_RIGHT: u32 = 1 << 10;

/// The packed `WaveFMT` (16 bytes) and `WaveFMTEx` (40 bytes), read from
/// a chunk's bytes.
#[derive(Clone, Copy, Default)]
struct WaveFmtEx {
    // Not saved in the chunk we read:
    //Uint32  chunkID;
    //Uint32  chunkLen;
    encoding: u16,
    channels: u16,  // 1 = mono, 2 = stereo
    frequency: u32, // One of 11025, 22050, or 44100 Hz
    #[allow(dead_code)]
    byterate: u32, // Average bytes per second
    blockalign: u16, // Bytes per sample block
    bitspersample: u16, // One of 8, 12, 16, or 4 for ADPCM
    cb_size: u16,
    samples: u16, // validbitspersample / samplesperblock / reserved
    channelsmask: u32,
    // GUID subFormat 16 bytes
    subencoding: u32,
}

const SIZEOF_WAVEFMT: usize = 16;
const SIZEOF_WAVEFMTEX: usize = 40;

/// Little-endian reads from a chunk; bytes past its end read as zero
/// (Note (upstream): the ADPCM setup reads the extended header fields
/// before checking the chunk is long enough to have them).
fn le16(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([
        d.get(o).copied().unwrap_or(0),
        d.get(o + 1).copied().unwrap_or(0),
    ])
}

fn le32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([
        d.get(o).copied().unwrap_or(0),
        d.get(o + 1).copied().unwrap_or(0),
        d.get(o + 2).copied().unwrap_or(0),
        d.get(o + 3).copied().unwrap_or(0),
    ])
}

impl WaveFmtEx {
    /// The struct as `SDL_memcpy(&fmt, chunk, size)` fills it (the rest zeroed).
    fn from_bytes(d: &[u8]) -> WaveFmtEx {
        WaveFmtEx {
            encoding: le16(d, 0),
            channels: le16(d, 2),
            frequency: le32(d, 4),
            byterate: le32(d, 8),
            blockalign: le16(d, 12),
            bitspersample: le16(d, 14),
            cb_size: le16(d, 16),
            samples: le16(d, 18),
            channelsmask: le32(d, 20),
            subencoding: le32(d, 24),
        }
    }
}

/// Translation of `ADPCM_DecoderInfo`.
#[derive(Clone, Default)]
struct AdpcmDecoderInfo {
    channels: u32,          // Number of channels.
    blocksize: usize,       // Size of an ADPCM block in bytes.
    blockheadersize: usize, // Size of an ADPCM block header in bytes.
    samplesperblock: usize, // Number of samples per channel in an ADPCM block.
    ddata: Vec<i16>,        // Decoder data from initialization (the MS ADPCM coefficient pairs).
}

/// Translation of `MS_ADPCM_ChannelState`.
#[derive(Clone, Copy, Default)]
struct MsAdpcmChannelState {
    delta: u16,
    coeff1: i16,
    coeff2: i16,
}

/// Translation of `ADPCM_DecoderState`.
#[derive(Default)]
struct AdpcmDecoderState {
    ms_cstate: Vec<MsAdpcmChannelState>, // Decoding state for each channel (MS ADPCM).
    ima_cstate: Vec<i8>,                 // Decoding state for each channel (IMA ADPCM).

    // Current ADPCM block
    block_data: Vec<u8>,
    block_size: usize,
    block_pos: usize,

    // Decoded 16-bit PCM data.
    output_data: Vec<i16>,
    #[allow(dead_code)]
    output_size: usize,
    output_pos: usize,
    output_read: usize,
}

/// Translation of `WAVLoopPoint`.
#[derive(Clone, Copy)]
struct WavLoopPoint {
    start: u32,
    stop: u32,
    iterations: u32,
}

// precalc some positional things to make seeking and looping easier.
/// Translation of `WAVSeekBlock`.
#[derive(Clone, Copy, Default, Debug)]
struct WavSeekBlock {
    frame_start: i64,
    num_frames: i64,
    iterations: i64,
    seek_position: i64, // for compressed formats, this is the start of the block. For uncompressed, it's the start of the sample frame.
}

/// Translation of `WAV_FetchFn`'s choices.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Fetch {
    #[default]
    Pcm,
    ULaw,
    ALaw,
    MsAdpcm,
    ImaAdpcm,
    Pcm24Le,
    Float64Le,
}

/// Translation of `WAV_AudioData`.
#[derive(Default)]
struct WavAudioData {
    adpcm_info: AdpcmDecoderInfo,
    encoding: u16,
    start: i64,
    stop: i64,
    framesize: i32,
    decoded_framesize: i32,
    fetch: Fetch,
    loops: Vec<WavLoopPoint>,
    seekblocks: Vec<WavSeekBlock>,
    channelmask: u32,
}

/// Translation of `WAV_TrackData`.
struct WavTrackData<'a> {
    adata: Arc<WavAudioData>,
    io: IoStream<'a>,
    adpcm_state: AdpcmDecoderState,
    seekblock: usize,              // current seekblock we're decoding.
    current_iteration: u32,        // current loop iteration in seekblock
    current_iteration_frames: u32, // current framecount into seekblock.
    channels: i32,
    must_set_channel_map: bool,
}

/// Translation of `IsADPCM()`.
fn is_adpcm(encoding: u16) -> bool {
    (encoding == MS_ADPCM_CODE) || (encoding == IMA_ADPCM_CODE)
}

/// Translation of `MS_ADPCM_Init()`.
fn ms_adpcm_init(info: &mut AdpcmDecoderInfo, chunk_data: &[u8], chunk_length: u32) -> Result<()> {
    let fmt = WaveFmtEx::from_bytes(chunk_data);
    let channels = fmt.channels;
    let blockalign = fmt.blockalign;
    let bitspersample = fmt.bitspersample;
    let blockheadersize = channels as usize * 7;
    let blockdatasize = (blockalign as usize).wrapping_sub(blockheadersize);
    let blockframebitsize = bitspersample as usize * channels as usize;
    // FIXME (upstream): this divides by zero (and crashes) for zero
    // channels or bits per sample, before the checks below reject them.
    if blockframebitsize == 0 {
        return Err(Error::new("WAV: Invalid MS ADPCM format header"));
    }
    let blockdatasamples = (blockdatasize.wrapping_mul(8)) / blockframebitsize;

    // While it's clear how IMA ADPCM handles more than two channels, the nibble
    // order of MS ADPCM makes it awkward. The Standards Update does not talk
    // about supporting more than stereo anyway.
    if channels > 2 {
        return Err(Error::new("WAV: ADPCM Invalid number of channels"));
    } else if bitspersample != 4 {
        return Err(Error::new(format!(
            "WAV: Invalid MS ADPCM bits per sample of {}",
            bitspersample as u32
        )));
    } else if (blockalign as usize) < blockheadersize {
        // The block size must be big enough to contain the block header.
        return Err(Error::new("WAV: Invalid MS ADPCM block size (nBlockAlign)"));
    }

    let cb_ext_size = fmt.cb_size;
    let mut samplesperblock = fmt.samples;

    // Number of coefficient pairs. A pair has two 16-bit integers.
    let mut coeffcount = le16(chunk_data, 20) as usize;

    // bPredictor, the integer offset into the coefficients array, is only
    // 8 bits. It can only address the first 256 coefficients. Let's limit
    // the count number here.
    if coeffcount > 256 {
        coeffcount = 256;
    }

    // There are wSamplesPerBlock, wNumCoef, and at least 7 coefficient pairs in the extended part of the header.
    if (chunk_length as usize) < 22 + coeffcount * 4 {
        return Err(Error::new(
            "WAV: Chunk size too small for MS ADPCM format header",
        ));
    } else if (cb_ext_size as usize) < 4 + coeffcount * 4 {
        return Err(Error::new(
            "WAV: Invalid MS ADPCM format header (too small)",
        ));
    } else if coeffcount < 7 {
        return Err(Error::new(
            "Missing required coefficients in MS ADPCM format header",
        ));
    }

    // Technically, wSamplesPerBlock is required, but we have all the
    // information in the other fields to calculate it, if it's zero.
    if samplesperblock == 0 {
        /* Let's be nice to the encoders that didn't know how to fill this.
         * The Standards Update calculates it this way:
         *
         *   x = Block size (in bits) minus header size (in bits)
         *   y = Bit depth multiplied by channel count
         *   z = Number of samples per channel in block header
         *   wSamplesPerBlock = x / y + z
         */
        samplesperblock = (blockdatasamples + 2) as u16;
    }

    /* nBlockAlign can be in conflict with wSamplesPerBlock. For example, if
     * the number of samples doesn't fit into the block. The Standards Update
     * also describes wSamplesPerBlock with a formula that makes it necessary to
     * always fill the block with the maximum amount of samples, but this is not
     * enforced here as there are no compatibility issues.
     * A truncated block header with just one sample is not supported.
     */
    if samplesperblock == 1 || blockdatasamples < (samplesperblock as usize).wrapping_sub(2) {
        return Err(Error::new(
            "WAV: Invalid number of samples per MS ADPCM block (wSamplesPerBlock)",
        ));
    }

    let mut coeff = try_alloc::<i16>(coeffcount * 2)?;

    // Copy the 16-bit pairs.
    static PRESETCOEFFS: [i16; 14] = [
        256, 0, 512, -256, 0, 0, 192, 64, 240, 0, 460, -208, 392, -232,
    ];
    for (i, c_out) in coeff.iter_mut().enumerate() {
        let mut c = chunk_data[22 + i * 2] as i32 | ((chunk_data[23 + i * 2] as i32) << 8);
        if c >= 0x8000 {
            c -= 0x10000;
        }
        if (i < 14) && (c != PRESETCOEFFS[i] as i32) {
            return Err(Error::new(
                "WAV: Wrong preset coefficients in MS ADPCM format header",
            ));
        }
        *c_out = c as i16;
    }
    info.ddata = coeff; // Freed in cleanup.

    info.blocksize = blockalign as usize;
    info.channels = channels as u32;
    info.blockheadersize = blockheadersize;
    info.samplesperblock = samplesperblock as usize;

    Ok(())
}

/// Translation of `MS_ADPCM_ProcessNibble()`.
fn ms_adpcm_process_nibble(
    cstate: &mut MsAdpcmChannelState,
    sample1: i32,
    sample2: i32,
    nybble: u8,
) -> i16 {
    const MAX_AUDIOVAL: i32 = 32767;
    const MIN_AUDIOVAL: i32 = -32768;
    const MAX_DELTAVAL: u32 = 65535;
    static ADAPTIVE: [u16; 16] = [
        230, 230, 230, 230, 307, 409, 512, 614, 768, 614, 512, 409, 307, 230, 230, 230,
    ];

    let mut new_sample = (sample1 * cstate.coeff1 as i32 + sample2 * cstate.coeff2 as i32) / 256;
    // The nibble is a signed 4-bit error delta.
    let errordelta = nybble as i32 - if nybble >= 0x08 { 0x10 } else { 0 };
    let mut delta = cstate.delta as u32;
    new_sample += delta as i32 * errordelta;
    if new_sample < MIN_AUDIOVAL {
        new_sample = MIN_AUDIOVAL;
    } else if new_sample > MAX_AUDIOVAL {
        new_sample = MAX_AUDIOVAL;
    }
    delta = (delta * ADAPTIVE[nybble as usize] as u32) / 256;
    if delta < 16 {
        delta = 16;
    } else if delta > MAX_DELTAVAL {
        // This issue is not described in the Standards Update and therefore
        // undefined. It seems sensible to prevent overflows with a limit.
        delta = MAX_DELTAVAL;
    }

    cstate.delta = delta as u16;
    new_sample as i16
}

/// Translation of `MS_ADPCM_DecodeBlockHeader()`.
fn ms_adpcm_decode_block_header(
    info: &AdpcmDecoderInfo,
    state: &mut AdpcmDecoderState,
) -> Result<()> {
    let channels = info.channels as usize;
    let coeffcount = info.ddata.len() / 2;

    if state.block_size < info.blockheadersize {
        return Err(Error::new("Invalid ADPCM header"));
    }

    for c in 0..channels {
        let mut o = c;

        // Load the coefficient pair into the channel state.
        let coeffindex = state.block_data[o] as usize;
        // FIXME (upstream): this checks `coeffindex > coeffcount`, which
        // lets an index equal to the count read past the coefficients.
        if coeffindex >= coeffcount {
            return Err(Error::new(
                "Invalid MS ADPCM coefficient index in block header",
            ));
        }
        let cstate = &mut state.ms_cstate[c];
        cstate.coeff1 = info.ddata[coeffindex * 2];
        cstate.coeff2 = info.ddata[coeffindex * 2 + 1];

        // Initial delta value.
        o = channels + c * 2;
        cstate.delta = state.block_data[o] as u16 | ((state.block_data[o + 1] as u16) << 8);

        // Load the samples from the header. Interestingly, the sample later in
        //the output stream comes first.
        o = channels * 3 + c * 2;
        let mut sample = state.block_data[o] as i32 | ((state.block_data[o + 1] as i32) << 8);
        if sample >= 0x8000 {
            sample -= 0x10000;
        }
        state.output_data[state.output_pos + channels] = sample as i16;

        o = channels * 5 + c * 2;
        sample = state.block_data[o] as i32 | ((state.block_data[o + 1] as i32) << 8);
        if sample >= 0x8000 {
            sample -= 0x10000;
        }
        state.output_data[state.output_pos] = sample as i16;

        state.output_pos += 1;
    }

    state.block_pos += info.blockheadersize;

    // Skip second sample frame that came from the header.
    state.output_pos += channels;

    Ok(())
}

// Decodes the data of the MS ADPCM block. Decoding will stop if a block is too
// short, returning with none or partially decoded data. The partial data
// will always contain full sample frames (same sample count for each channel).
// Incomplete sample frames are discarded.
/// Translation of `MS_ADPCM_DecodeBlockData()`.
fn ms_adpcm_decode_block_data(
    info: &AdpcmDecoderInfo,
    state: &mut AdpcmDecoderState,
) -> Result<()> {
    let channels = info.channels as usize;
    let mut nybble: u16 = 0;
    let mut blockpos = state.block_pos;
    let blocksize = state.block_size;
    let mut outpos = state.output_pos;
    let mut blockframesleft = info.samplesperblock - 2;

    while blockframesleft > 0 {
        for c in 0..channels {
            if nybble & 0x4000 != 0 {
                nybble <<= 4;
            } else if blockpos < blocksize {
                nybble = state.block_data[blockpos] as u16 | 0x4000;
                blockpos += 1;
            } else {
                // Out of input data. Drop the incomplete frame and return.
                state.output_pos = outpos - c;
                return Err(Error::new("Truncated MS ADPCM block"));
            }

            // Load previous samples which may come from the block header.
            let sample1 = state.output_data[outpos - channels];
            let sample2 = state.output_data[outpos - channels * 2];

            let sample1 = ms_adpcm_process_nibble(
                &mut state.ms_cstate[c],
                sample1 as i32,
                sample2 as i32,
                ((nybble >> 4) & 0x0f) as u8,
            );
            state.output_data[outpos] = sample1;
            outpos += 1;
        }

        blockframesleft -= 1;
    }

    state.output_pos = outpos;

    Ok(())
}

/// Translation of `IMA_ADPCM_Init()`.
fn ima_adpcm_init(info: &mut AdpcmDecoderInfo, chunk_data: &[u8], chunk_length: u32) -> Result<()> {
    let fmt = WaveFmtEx::from_bytes(chunk_data);
    let formattag = fmt.encoding;
    let channels = fmt.channels;
    let blockalign = fmt.blockalign;
    let bitspersample = fmt.bitspersample;
    let blockheadersize = channels as usize * 4;
    let blockdatasize = (blockalign as usize).wrapping_sub(blockheadersize);
    let blockframebitsize = bitspersample as usize * channels as usize;
    // FIXME (upstream): this divides by zero (and crashes) for zero
    // channels or bits per sample, before the checks below reject them.
    if blockframebitsize == 0 {
        return Err(Error::new("WAV: Invalid IMA ADPCM format header"));
    }
    let blockdatasamples = (blockdatasize.wrapping_mul(8)) / blockframebitsize;
    let mut samplesperblock: u16 = 0;

    // IMA ADPCM can also have 3-bit samples, but it's not supported by SDL at this time.
    if bitspersample == 3 {
        return Err(Error::new("WAV: 3-bit IMA ADPCM currently not supported"));
    } else if bitspersample != 4 {
        return Err(Error::new(format!(
            "WAV: Invalid IMA ADPCM bits per sample of {}",
            bitspersample as u32
        )));
    } else if ((blockalign as usize) < blockheadersize) || (blockalign % 4) != 0 {
        // The block size is required to be a multiple of 4 and it must be able to hold a block header.
        return Err(Error::new(
            "WAV: Invalid IMA ADPCM block size (nBlockAlign)",
        ));
    }

    if formattag == EXTENSIBLE_CODE {
        // There's no specification for this, but it's basically the same
        // format because the extensible header has wSampePerBlocks too.
    } else if chunk_length >= 20 {
        let cb_ext_size = fmt.cb_size;
        if cb_ext_size >= 2 {
            samplesperblock = fmt.samples;
        }
    }

    if samplesperblock == 0 {
        /* Field zero? No problem. We just assume the encoder packed the block.
         * The specification calculates it this way:
         *
         *   x = Block size (in bits) minus header size (in bits)
         *   y = Bit depth multiplied by channel count
         *   z = Number of samples per channel in header
         *   wSamplesPerBlock = x / y + z
         */
        samplesperblock = (blockdatasamples + 1) as u16;
    }

    /* nBlockAlign can be in conflict with wSamplesPerBlock. For example, if
     * the number of samples doesn't fit into the block. The Standards Update
     * also describes wSamplesPerBlock with a formula that makes it necessary
     * to always fill the block with the maximum amount of samples, but this is
     * not enforced here as there are no compatibility issues.
     */
    if blockdatasamples < (samplesperblock as usize).wrapping_sub(1) {
        return Err(Error::new(
            "WAV: Invalid number of samples per IMA ADPCM block (wSamplesPerBlock)",
        ));
    }

    info.blocksize = blockalign as usize;
    info.channels = channels as u32;
    info.blockheadersize = blockheadersize;
    info.samplesperblock = samplesperblock as usize;

    Ok(())
}

/// Translation of `IMA_ADPCM_ProcessNibble()`.
fn ima_adpcm_process_nibble(cindex: &mut i8, lastsample: i16, nybble: u8) -> i16 {
    const MAX_AUDIOVAL: i32 = 32767;
    const MIN_AUDIOVAL: i32 = -32768;
    static INDEX_TABLE_4B: [i8; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];
    static STEP_TABLE: [u16; 89] = [
        7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60,
        66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371,
        408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878,
        2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845,
        8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086,
        29794, 32767,
    ];

    let mut index = *cindex;

    // Clamp index into valid range.
    if index > 88 {
        index = 88;
    } else if index < 0 {
        index = 0;
    }

    // explicit cast to avoid gcc warning about using 'char' as array index
    let step = STEP_TABLE[index as usize] as u32;

    // Update index value
    *cindex = index + INDEX_TABLE_4B[nybble as usize];

    /* This calculation uses shifts and additions because multiplications were
     * much slower back then. Sadly, this can't just be replaced with an actual
     * multiplication now as the old algorithm drops some bits. The closest
     * approximation I could find is something like this:
     * (nybble & 0x8 ? -1 : 1) * ((nybble & 0x7) * step / 4 + step / 8)
     */
    let mut delta = (step >> 3) as i32;
    if nybble & 0x04 != 0 {
        delta += step as i32;
    }
    if nybble & 0x02 != 0 {
        delta += (step >> 1) as i32;
    }
    if nybble & 0x01 != 0 {
        delta += (step >> 2) as i32;
    }
    if nybble & 0x08 != 0 {
        delta = -delta;
    }

    let mut sample = lastsample as i32 + delta;

    // Clamp output sample
    if sample > MAX_AUDIOVAL {
        sample = MAX_AUDIOVAL;
    } else if sample < MIN_AUDIOVAL {
        sample = MIN_AUDIOVAL;
    }

    sample as i16
}

/// Translation of `IMA_ADPCM_DecodeBlockHeader()`.
fn ima_adpcm_decode_block_header(
    info: &AdpcmDecoderInfo,
    state: &mut AdpcmDecoderState,
) -> Result<()> {
    if state.block_size < info.blockheadersize {
        return Err(Error::new("Invalid ADPCM header"));
    }

    for c in 0..info.channels as usize {
        let o = state.block_pos + c * 4;

        // Extract the sample from the header.
        let mut sample = state.block_data[o] as i32 | ((state.block_data[o + 1] as i32) << 8);
        if sample >= 0x8000 {
            sample -= 0x10000;
        }
        state.output_data[state.output_pos] = sample as i16;
        state.output_pos += 1;

        // Channel step index.
        let step = state.block_data[o + 2] as i16;
        state.ima_cstate[c] = (if step > 0x80 { step - 0x100 } else { step }) as i8;

        // Reserved byte in block header, should be 0.
        if state.block_data[o + 3] != 0 {
            // Uh oh, corrupt data?  Buggy code?
        }
    }

    state.block_pos += info.blockheadersize;

    Ok(())
}

// Decodes the data of the IMA ADPCM block. Decoding will stop if a block is too
// short, returning with none or partially decoded data. The partial data always
// contains full sample frames (same sample count for each channel).
// Incomplete sample frames are discarded.
/// Translation of `IMA_ADPCM_DecodeBlockData()`.
fn ima_adpcm_decode_block_data(
    info: &AdpcmDecoderInfo,
    state: &mut AdpcmDecoderState,
) -> Result<()> {
    let mut retval = Ok(());
    let channels = info.channels as usize;
    let subblockframesize = channels * 4;
    let mut blockpos = state.block_pos;
    let blocksize = state.block_size;
    let blockleft = blocksize - blockpos;
    let mut outpos = state.output_pos;
    let mut blockframesleft = info.samplesperblock as i64 - 1;
    let bytesrequired = ((blockframesleft + 7) / 8) as u64 * subblockframesize as u64;

    if (blockleft as u64) < bytesrequired {
        // Data truncated. Calculate how many samples we can get out if it.
        let guaranteedframes = blockleft / subblockframesize;
        let remainingbytes = blockleft % subblockframesize;
        blockframesleft = guaranteedframes as i64;
        if remainingbytes > subblockframesize - 4 {
            blockframesleft += (remainingbytes % 4) as i64 * 2;
        }
        // Signal the truncation.
        retval = Err(Error::new("Truncated IMA ADPCM block"));
        // FIXME (upstream): for a block of few samples, the frames counted
        // in a truncated block can be more than the block holds, which
        // writes past the output buffer; they're clamped here (the C
        // reference harness is patched the same way).
        if blockframesleft > info.samplesperblock as i64 - 1 {
            blockframesleft = info.samplesperblock as i64 - 1;
        }
    }

    /* Each channel has their nibbles packed into 32-bit blocks. These blocks
     * are interleaved and make up the data part of the ADPCM block. This loop
     * decodes the samples as they come from the input data and puts them at
     * the appropriate places in the output data.
     */
    while blockframesleft > 0 {
        let subblocksamples = if blockframesleft < 8 {
            blockframesleft as usize
        } else {
            8
        };

        for c in 0..channels {
            let mut nybble: u8 = 0;
            // Load previous sample which may come from the block header.
            let mut sample = state.output_data[outpos + c - channels];

            for i in 0..subblocksamples {
                if i & 1 != 0 {
                    nybble >>= 4;
                } else {
                    nybble = state.block_data.get(blockpos).copied().unwrap_or(0);
                    blockpos += 1;
                }

                sample = ima_adpcm_process_nibble(&mut state.ima_cstate[c], sample, nybble & 0x0f);
                state.output_data[outpos + c + i * channels] = sample;
            }
        }

        outpos += channels * subblocksamples;
        blockframesleft -= subblocksamples as i64;
    }

    state.block_pos = blockpos;
    state.output_pos = outpos;

    retval
}

/// Translation of `StandardSDLWavChannelMask()`.
fn standard_sdl_wav_channel_mask(channels: i32) -> u32 {
    match channels {
        1 => WAV_SPEAKER_FRONT_CENTER,
        2 => WAV_SPEAKER_FRONT_LEFT | WAV_SPEAKER_FRONT_RIGHT,
        3 => WAV_SPEAKER_FRONT_LEFT | WAV_SPEAKER_FRONT_RIGHT | WAV_SPEAKER_LOW_FREQUENCY,
        4 => {
            WAV_SPEAKER_FRONT_LEFT
                | WAV_SPEAKER_FRONT_RIGHT
                | WAV_SPEAKER_BACK_LEFT
                | WAV_SPEAKER_BACK_RIGHT
        }
        5 => {
            WAV_SPEAKER_FRONT_LEFT
                | WAV_SPEAKER_FRONT_RIGHT
                | WAV_SPEAKER_BACK_LEFT
                | WAV_SPEAKER_BACK_RIGHT
                | WAV_SPEAKER_LOW_FREQUENCY
        }
        6 => {
            WAV_SPEAKER_FRONT_LEFT
                | WAV_SPEAKER_FRONT_RIGHT
                | WAV_SPEAKER_FRONT_CENTER
                | WAV_SPEAKER_BACK_LEFT
                | WAV_SPEAKER_BACK_RIGHT
                | WAV_SPEAKER_LOW_FREQUENCY
        }
        7 => {
            WAV_SPEAKER_FRONT_LEFT
                | WAV_SPEAKER_FRONT_RIGHT
                | WAV_SPEAKER_FRONT_CENTER
                | WAV_SPEAKER_BACK_CENTER
                | WAV_SPEAKER_SIDE_LEFT
                | WAV_SPEAKER_SIDE_RIGHT
                | WAV_SPEAKER_LOW_FREQUENCY
        }
        8 => {
            WAV_SPEAKER_FRONT_LEFT
                | WAV_SPEAKER_FRONT_RIGHT
                | WAV_SPEAKER_FRONT_CENTER
                | WAV_SPEAKER_BACK_LEFT
                | WAV_SPEAKER_BACK_RIGHT
                | WAV_SPEAKER_SIDE_LEFT
                | WAV_SPEAKER_SIDE_RIGHT
                | WAV_SPEAKER_LOW_FREQUENCY
        }
        _ => {
            if channels < 32 {
                ((1i64 << channels) - 1) as u32
            } else {
                0xFFFFFFFF // mark all available channels as used by default.
            }
        }
    }
}

/// Translation of `ParseFMT()`.
fn parse_fmt(
    adata: &mut WavAudioData,
    io: &mut IoStream<'_>,
    spec: &mut AudioSpec,
    chunk_length: u32,
) -> Result<()> {
    if (chunk_length as usize) < SIZEOF_WAVEFMT {
        return Err(Error::new("Wave format chunk too small"));
    }

    let mut chunk = try_alloc::<u8>(chunk_length as usize)?;

    if io.read(&mut chunk) != chunk_length as usize {
        return Err(Error::new(format!(
            "Couldn't read {} bytes from WAV file",
            chunk_length
        )));
    }

    let size = if chunk_length as usize >= SIZEOF_WAVEFMTEX {
        SIZEOF_WAVEFMTEX
    } else {
        SIZEOF_WAVEFMT
    };
    let fmt = WaveFmtEx::from_bytes(&chunk[..size]);

    adata.encoding = fmt.encoding;

    if adata.encoding == EXTENSIBLE_CODE {
        if size < SIZEOF_WAVEFMTEX {
            return Err(Error::new("Wave format chunk too small"));
        }
        adata.encoding = fmt.subencoding as u16;
        adata.channelmask = fmt.channelsmask;
    } else {
        adata.channelmask = standard_sdl_wav_channel_mask(fmt.channels as i32);
    }

    // FIXME (upstream): zero channels (also 256, as the count is stored in
    // a byte) make a zero frame size, which upstream divides by (and
    // crashes) later.
    if fmt.channels as u8 == 0 {
        return Err(Error::new("WAV: Invalid number of channels"));
    }

    // Decode the audio data format
    match adata.encoding {
        PCM_CODE | IEEE_FLOAT_CODE => adata.fetch = Fetch::Pcm,
        MULAW_CODE => adata.fetch = Fetch::ULaw,
        ALAW_CODE => adata.fetch = Fetch::ALaw,
        MS_ADPCM_CODE => {
            adata.fetch = Fetch::MsAdpcm;
            ms_adpcm_init(&mut adata.adpcm_info, &chunk, chunk_length)?;
        }
        IMA_ADPCM_CODE => {
            adata.fetch = Fetch::ImaAdpcm;
            ima_adpcm_init(&mut adata.adpcm_info, &chunk, chunk_length)?;
        }

        // !!! FIXME: pass off embedded MP3 data to drmp3/mpg123?
        _ => return Err(Error::new("Unknown WAVE data format")),
    }

    spec.freq = fmt.frequency as i32;
    let bits = fmt.bitspersample as i32;
    let mut unknown_bits = false;
    match bits {
        4 => match adata.encoding {
            MS_ADPCM_CODE => spec.format = AudioFormat::S16,
            IMA_ADPCM_CODE => spec.format = AudioFormat::S16,
            _ => unknown_bits = true,
        },
        8 => match adata.encoding {
            PCM_CODE => spec.format = AudioFormat::U8,
            ALAW_CODE => spec.format = AudioFormat::F32,
            MULAW_CODE => spec.format = AudioFormat::F32,
            _ => unknown_bits = true,
        },
        16 => match adata.encoding {
            PCM_CODE => spec.format = AudioFormat::S16LE,
            _ => unknown_bits = true,
        },
        24 => match adata.encoding {
            PCM_CODE => {
                adata.fetch = Fetch::Pcm24Le;
                spec.format = AudioFormat::F32;
            }
            _ => unknown_bits = true,
        },
        32 => match adata.encoding {
            PCM_CODE => spec.format = AudioFormat::S32LE,
            IEEE_FLOAT_CODE => spec.format = AudioFormat::F32LE,
            _ => unknown_bits = true,
        },
        64 => match adata.encoding {
            IEEE_FLOAT_CODE => {
                adata.fetch = Fetch::Float64Le;
                spec.format = AudioFormat::F32;
            }
            _ => unknown_bits = true,
        },
        _ => unknown_bits = true,
    }

    if unknown_bits {
        return Err(Error::new(format!(
            "WAV: Unknown PCM format with {} bits",
            bits
        )));
    }

    spec.channels = fmt.channels as u8 as i32;
    adata.framesize = spec.channels * (bits / 8);
    adata.decoded_framesize = spec.frame_size() as i32;

    Ok(())
}

/// Translation of `ParseDATA()`.
fn parse_data(adata: &mut WavAudioData, io: &mut IoStream<'_>, chunk_length: u32) -> Result<()> {
    adata.start = io.tell().unwrap_or(-1);
    adata.stop = adata.start + chunk_length as i64;
    Ok(())
}

/// Translation of `AddLoopPoint()`.
fn add_loop_point(adata: &mut WavAudioData, play_count: u32, start: u32, stop: u32) -> bool {
    // ignore the loop if it's bogus but carry on.
    if start >= stop {
        return true;
    }

    if adata.loops.try_reserve(1).is_err() {
        return false;
    }

    //SDL_Log("LOOP: count=%d start=%d stop=%d", (int) play_count, (int) start, (int) stop);

    adata.loops.push(WavLoopPoint {
        start,
        stop,
        iterations: play_count,
    });

    true
}

/// Translation of `ParseSMPL()`.
fn parse_smpl(adata: &mut WavAudioData, io: &mut IoStream<'_>, chunk_length: u32) -> Result<()> {
    let mut data = try_alloc::<u8>(chunk_length as usize)?;
    if io.read(&mut data) != chunk_length as usize {
        return Err(Error::new(format!(
            "Couldn't read {} bytes from WAV file",
            chunk_length
        )));
    }

    // SamplerChunk: nine Uint32s, then the SampleLoops (six Uint32s each).
    // FIXME (upstream): the loop count isn't checked against the chunk's
    // length, so the loops are read past its end; here the loops (and the
    // count) that aren't in the chunk aren't read.
    let sample_loops = if data.len() >= 32 { le32(&data, 28) } else { 0 };
    for i in 0..sample_loops as usize {
        let at = 36 + i * 24;
        if at + 24 > data.len() {
            break;
        }
        const LOOP_TYPE_FORWARD: u32 = 0;
        let loop_type = le32(&data, at + 4);
        if loop_type == LOOP_TYPE_FORWARD {
            add_loop_point(
                adata,
                le32(&data, at + 20),
                le32(&data, at + 8),
                le32(&data, at + 12).wrapping_add(1),
            ); // +1 because the end field is inclusive.
        }
    }

    data.clear();

    // !!! FIXME: sort loops so they go from start to finish.
    // !!! FIXME: eliminate loops that overlap.
    // !!! FIXME: eliminate loops that go past EOF.

    Ok(())
}

/// Translation of `Swap32LEUnaligned()`.
fn swap32le_unaligned(data: &[u8]) -> u32 {
    // read as bytes, swap ourselves.
    (data[0] as u32) | ((data[1] as u32) << 8) | ((data[2] as u32) << 16) | ((data[3] as u32) << 24)
}

/// How many zero bytes `ParseLIST()`'s buffer gets past the chunk: Note
/// (upstream): the field parser reads up to 3 bytes past the chunk (and a
/// string with no NUL in the chunk, up to one), which read as zero here
/// (the C reference harness pads its copy the same way).
const LIST_PADDING: usize = 16;

/// Translation of `CheckWAVMetadataField()`.
fn check_wav_metadata_field(
    wantedtag: &[u8; 4],
    propname: &str,
    props: &Properties,
    i: &mut usize,
    chunk_length: u32,
    data: &[u8],
) -> bool {
    let tag = &data[*i..*i + 4];
    if tag != wantedtag {
        return false;
    }

    *i += 4;

    let len = swap32le_unaligned(&data[*i..]);

    if len > chunk_length {
        *i -= 4; // move back so we can resync.
        return false; // Do nothing due to broken length
    }
    *i += 4;

    // SDL_strlcpy(field, data + *i, len): at most len-1 bytes, to a NUL.
    let src = cstr(&data[(*i).min(data.len())..]);
    let field = cstring(&src[..src.len().min((len as usize).saturating_sub(1))]);
    *i += len as usize;

    let key = format!(
        "SDL_mixer.metadata.wavLIST.{}",
        String::from_utf8_lossy(wantedtag)
    );
    let _ = props.set(&key, field.clone());
    if !props.contains(propname) {
        let _ = props.set(propname, field);
    }

    true
}

/// Translation of `ParseLIST()`.
fn parse_list(io: &mut IoStream<'_>, props: &Properties, chunk_length: u32) -> Result<()> {
    let mut data = try_alloc::<u8>(chunk_length as usize + LIST_PADDING)?;

    if io.read(&mut data[..chunk_length as usize]) != chunk_length as usize {
        return Err(Error::new(format!(
            "Couldn't read {} bytes from WAV file",
            chunk_length
        )));
    }

    if &data[..4] == b"INFO" {
        let mut i = 4usize;
        while i < (chunk_length as usize).wrapping_sub(4) && i + 8 <= data.len() {
            if check_wav_metadata_field(
                b"INAM",
                PROP_METADATA_TITLE_STRING,
                props,
                &mut i,
                chunk_length,
                &data,
            ) || check_wav_metadata_field(
                b"IART",
                PROP_METADATA_ARTIST_STRING,
                props,
                &mut i,
                chunk_length,
                &data,
            ) || check_wav_metadata_field(
                b"IALB",
                PROP_METADATA_ALBUM_STRING,
                props,
                &mut i,
                chunk_length,
                &data,
            ) || check_wav_metadata_field(
                b"BCPR",
                PROP_METADATA_COPYRIGHT_STRING,
                props,
                &mut i,
                chunk_length,
                &data,
            ) {
                continue;
            }
            i += 1;
        }
    }

    Ok(())
}

/// Translation of `ParseWAVID3()`.
fn parse_wav_id3(io: &mut IoStream<'_>, props: &Properties, chunk_length: u32) -> Result<()> {
    let mut clamp = IoClamp::open(borrow_io(io))?;
    clamp.length = chunk_length as i64;
    read_metadata_tags(&mut clamp, props);
    Ok(())
}

/// Translation of `CalcSeekBlockSeek()`.
fn calc_seek_block_seek(adata: &WavAudioData, actual_frame: u32, seekblock: &mut WavSeekBlock) {
    if is_adpcm(adata.encoding) {
        seekblock.seek_position = adata.start
            + ((actual_frame as usize / adata.adpcm_info.samplesperblock)
                * adata.adpcm_info.blocksize) as i64;
    } else {
        seekblock.seek_position = adata.start + (actual_frame as i64 * adata.framesize as i64);
    }
}

/// Translation of `BuildSeekBlocks()`.
fn build_seek_blocks(adata: &mut WavAudioData) -> Result<()> {
    let all_bytes_in_file = adata.stop - adata.start;

    let numloops = adata.loops.len();
    if numloops == 0 {
        let mut seekblock = WavSeekBlock {
            frame_start: 0,
            iterations: 1,
            seek_position: adata.start,
            ..Default::default()
        };

        if is_adpcm(adata.encoding) {
            // if for some reason the final block isn't completely present, this number might be wrong; we'll presumably decode to the real EOF later.
            seekblock.num_frames = (all_bytes_in_file / adata.adpcm_info.blocksize as i64)
                * adata.adpcm_info.samplesperblock as i64;
        } else {
            seekblock.num_frames = all_bytes_in_file / adata.framesize as i64;
        }
        adata.seekblocks = vec![seekblock];
    } else {
        let num_seekblocks = (numloops * 2) + 1;
        let mut seekblocks = try_alloc::<WavSeekBlock>(num_seekblocks)?;

        let mut current_frame: i64 = 0;
        let mut s = 0;

        // first seekblock is start of audio data, before any loop.
        seekblocks[s].frame_start = current_frame;
        seekblocks[s].num_frames = adata.loops[0].start as i64;
        seekblocks[s].iterations = 1;
        calc_seek_block_seek(adata, 0, &mut seekblocks[s]);

        current_frame += seekblocks[s].num_frames;
        s += 1;

        for i in 0..numloops {
            // space covered by a loop...
            let lp = adata.loops[i];
            seekblocks[s].frame_start = current_frame;
            seekblocks[s].num_frames = lp.stop as i64 - lp.start as i64;
            calc_seek_block_seek(adata, lp.start, &mut seekblocks[s]);

            if lp.iterations == 0 {
                // it's an infinite loop!
                seekblocks[s].iterations = -1;
                current_frame += seekblocks[s].num_frames;
            } else {
                seekblocks[s].iterations = lp.iterations as i64;
                current_frame += seekblocks[s].num_frames * lp.iterations as i64;
            }

            // space covered between loops (or after last loop to EOF)...
            s += 1;
            seekblocks[s].frame_start = current_frame;
            if i < (numloops - 1) {
                seekblocks[s].num_frames = adata.loops[i + 1].start.wrapping_sub(lp.stop) as i64;
            } else if is_adpcm(adata.encoding) {
                // this is kinda wordy, but I wanted to reason through all the math.
                let loop_stop_frame = lp.stop as i64;
                let frames_per_block = adata.adpcm_info.samplesperblock as i64;
                let stop_frame_block = loop_stop_frame / frames_per_block;
                let offset_into_stop_block = loop_stop_frame % frames_per_block;
                let frames_left_in_stop_block = frames_per_block - offset_into_stop_block;
                // FIXME (upstream): this divides the data's size in bytes by
                // the frames per block, where it means the block size.
                let all_blocks_in_file = (adata.stop - adata.start) / frames_per_block;
                let blocks_left_after_stop_block = all_blocks_in_file - stop_frame_block;
                seekblocks[s].num_frames =
                    frames_left_in_stop_block + (blocks_left_after_stop_block * frames_per_block);
            } else {
                let all_frames_in_file = all_bytes_in_file / adata.framesize as i64;
                seekblocks[s].num_frames = all_frames_in_file - (lp.stop as i64);
            }

            seekblocks[s].iterations = 1;
            calc_seek_block_seek(adata, lp.stop, &mut seekblocks[s]);

            current_frame += seekblocks[s].num_frames;
            s += 1;
        }
        adata.seekblocks = seekblocks;
    }

    Ok(())
}

/// Translation of `WAV_init_audio_internal()`.
fn wav_init_audio_internal(
    adata: &mut WavAudioData,
    io: &mut IoStream<'_>,
    spec: &mut AudioSpec,
    props: &Properties,
) -> Result<()> {
    let ignore_loops = props
        .get_bool(PROP_AUDIO_LOAD_IGNORE_LOOPS_BOOLEAN)
        .unwrap_or(false);
    let mut flen = io.size().unwrap_or(-1);
    let mut found_fmt = false;
    let mut found_data = false;

    // Check the magic header
    let mut wavemagic = io.read_u32_le()?;
    let wavelen = io.read_u32_le()?;
    if (wavelen as i64) > flen {
        return Err(Error::new("Corrupt WAV file (wavlength goes past EOF)"));
    }

    flen = wavelen as i64; // clamp the believed file length, in case there's something appended to the WAV file.

    if wavemagic == RIFF {
        // there's a 4-byte "form type" that must be WAVE, and then the first chunk follows directly after.
        wavemagic = io.read_u32_le()?;
    }

    if wavemagic != WAVE {
        return Err(Error::new("Not a WAV file"));
    }

    // Read the chunks
    loop {
        let (chunk_type, chunk_length) =
            match io.read_u32_le().and_then(|t| Ok((t, io.read_u32_le()?))) {
                Ok(tl) => tl,
                Err(e) => {
                    if io.status() == IoStatus::Eof {
                        break;
                    }
                    return Err(e);
                }
            };

        let chunk_start_position = io.tell()?;
        if chunk_start_position < 0 {
            return Err(Error::new("Error seeking in datastream"));
        }

        if chunk_length == 0 {
            break;
        } else if ((chunk_start_position + chunk_length as i64) - 8) > flen {
            return Err(Error::new("Corrupt WAV file (chunk goes past EOF)"));
        }

        match chunk_type {
            FMT => {
                found_fmt = true;
                parse_fmt(adata, io, spec, chunk_length)?;
            }
            DATA => {
                found_data = true;
                parse_data(adata, io, chunk_length)?;
            }
            SMPL => {
                if !ignore_loops {
                    parse_smpl(adata, io, chunk_length)?;
                }
            }
            LIST => parse_list(io, props, chunk_length)?,
            ID3X_LOWER | ID3X_UPPER => parse_wav_id3(io, props, chunk_length)?,
            _ => {
                // unknown or unsupported chunk, we'll just skip it.
            }
        }

        // move to start of next chunk.
        let mut next_chunk = chunk_start_position + chunk_length as i64;
        // RIFF chunks have a 2-byte alignment. Skip padding byte.
        if chunk_length & 1 != 0 {
            next_chunk += 1;
        }

        if next_chunk >= flen {
            break; // we're done.
        }
        io.seek(next_chunk, IoWhence::Set)?;
    }

    if !found_fmt {
        return Err(Error::new("Bad WAV file (no FMT chunk)"));
    } else if !found_data {
        return Err(Error::new("Bad WAV file (no DATA chunk)"));
    }

    build_seek_blocks(adata)
}

/// Translation of `WAV_init_audio()`.
fn wav_init_audio(
    io: Option<&mut IoStream<'_>>,
    spec: &mut AudioSpec,
    props: &Properties,
    duration_frames: &mut i64,
) -> Result<Arc<dyn AudioData>> {
    let io = io.ok_or_else(|| Error::invalid_param("io"))?;

    // Quick rejection before we allocate anything.
    let wavemagic = io.read_u32_le()?;
    if (wavemagic != WAVE) && (wavemagic != RIFF) {
        return Err(Error::new("WAV: not a wav file"));
    }
    io.seek(0, IoWhence::Set)?;

    let mut adata = WavAudioData::default();

    wav_init_audio_internal(&mut adata, io, spec, props)?;

    let mut num_frames: i64 = 0;

    // figure out if loops increase play length...
    for seekblock in &adata.seekblocks {
        if seekblock.iterations < 0 {
            // infinite loop
            num_frames = DURATION_INFINITE;
            break;
        }
        num_frames =
            num_frames.wrapping_add(seekblock.num_frames.wrapping_mul(seekblock.iterations));
    }

    *duration_frames = num_frames;

    Ok(Arc::new(adata))
}

/// Translation of `SpeakerBitToSDLChannelIndex()`.
fn speaker_bit_to_sdl_channel_index(channels: i32, newbit: u32) -> i32 {
    match channels {
        1 => 0, // always speaker 0, I dunno.

        2 => match newbit {
            WAV_SPEAKER_FRONT_LEFT => 0,
            WAV_SPEAKER_FRONT_RIGHT => 1,
            _ => -1,
        },

        3 => match newbit {
            WAV_SPEAKER_FRONT_LEFT => 0,
            WAV_SPEAKER_FRONT_RIGHT => 1,
            WAV_SPEAKER_LOW_FREQUENCY => 2,
            _ => -1,
        },

        4 => match newbit {
            WAV_SPEAKER_FRONT_LEFT => 0,
            WAV_SPEAKER_FRONT_RIGHT => 1,
            WAV_SPEAKER_BACK_LEFT => 2,
            WAV_SPEAKER_BACK_RIGHT => 3,
            _ => -1,
        },

        5 => match newbit {
            WAV_SPEAKER_FRONT_LEFT => 0,
            WAV_SPEAKER_FRONT_RIGHT => 1,
            WAV_SPEAKER_LOW_FREQUENCY => 2,
            WAV_SPEAKER_BACK_LEFT => 3,
            WAV_SPEAKER_BACK_RIGHT => 4,
            _ => -1,
        },

        6 => match newbit {
            WAV_SPEAKER_FRONT_LEFT => 0,
            WAV_SPEAKER_FRONT_RIGHT => 1,
            WAV_SPEAKER_FRONT_CENTER => 2,
            WAV_SPEAKER_LOW_FREQUENCY => 3,
            WAV_SPEAKER_BACK_LEFT => 4,
            WAV_SPEAKER_BACK_RIGHT => 5,
            _ => -1,
        },

        7 => match newbit {
            WAV_SPEAKER_FRONT_LEFT => 0,
            WAV_SPEAKER_FRONT_RIGHT => 1,
            WAV_SPEAKER_FRONT_CENTER => 2,
            WAV_SPEAKER_LOW_FREQUENCY => 3,
            WAV_SPEAKER_BACK_CENTER => 4,
            WAV_SPEAKER_SIDE_LEFT => 5,
            WAV_SPEAKER_SIDE_RIGHT => 6,
            _ => -1,
        },

        8 => match newbit {
            WAV_SPEAKER_FRONT_LEFT => 0,
            WAV_SPEAKER_FRONT_RIGHT => 1,
            WAV_SPEAKER_FRONT_CENTER => 2,
            WAV_SPEAKER_LOW_FREQUENCY => 3,
            WAV_SPEAKER_BACK_LEFT => 4,
            WAV_SPEAKER_BACK_RIGHT => 5,
            WAV_SPEAKER_SIDE_LEFT => 6,
            WAV_SPEAKER_SIDE_RIGHT => 7,
            _ => -1,
        },

        _ => -1,
    }
}

/// Translation of `SetAudioStreamChannelMapForWav()`.
fn set_audio_stream_channel_map_for_wav(stream: &AudioStream, mut channels: i32, channelmask: u32) {
    // WAV files can provide whatever channels they want, setting the provided channels in a bitmask,
    // but they have to provide the data for those channels a specific order.
    // Generally this lines up with SDL, but we need to make sure that unexpected channels in the
    // bitmask are handled.

    if channels == 1 {
        return; // don't remap mono stream (what would you remap it to!?), in case something reports front-center vs front-left or whatever.
    }

    let mut standardmap = standard_sdl_wav_channel_mask(channels);
    if channelmask == standardmap {
        return; // no remapping needed!
    }

    let mut chmap = [0i32; 32];
    channels = channels.min(chmap.len() as i32);

    let mut current_channel = 0;
    for i in 0..32 {
        let channelbit = channelmask & (1u32 << i);

        if channelbit != 0 {
            // wav file uses this speaker?
            let mut remapping = -1; // drop it by default.
            if standardmap & (1u32 << i) != 0 {
                // SDL offers this same speaker.
                remapping = speaker_bit_to_sdl_channel_index(channels, channelbit);
            } else {
                // see if we can bump this channel into something unused that we _do_ have (like, side_left can become back_left, better than nothing).
                let mut newbit = 0; // no remapping by default.
                match channelbit {
                    WAV_SPEAKER_BACK_LEFT => {
                        if standardmap & WAV_SPEAKER_SIDE_LEFT == 0 {
                            newbit = WAV_SPEAKER_SIDE_LEFT;
                        }
                    }
                    WAV_SPEAKER_BACK_RIGHT => {
                        if standardmap & WAV_SPEAKER_SIDE_RIGHT == 0 {
                            newbit = WAV_SPEAKER_SIDE_RIGHT;
                        }
                    }
                    WAV_SPEAKER_FRONT_LEFT_OF_CENTER => {
                        if standardmap & WAV_SPEAKER_FRONT_LEFT == 0 {
                            newbit = WAV_SPEAKER_FRONT_LEFT;
                        }
                    }
                    WAV_SPEAKER_FRONT_RIGHT_OF_CENTER => {
                        if standardmap & WAV_SPEAKER_FRONT_RIGHT == 0 {
                            newbit = WAV_SPEAKER_FRONT_RIGHT;
                        }
                    }
                    WAV_SPEAKER_SIDE_LEFT => {
                        if standardmap & WAV_SPEAKER_BACK_LEFT == 0 {
                            newbit = WAV_SPEAKER_BACK_LEFT;
                        }
                    }
                    WAV_SPEAKER_SIDE_RIGHT => {
                        if standardmap & WAV_SPEAKER_BACK_RIGHT == 0 {
                            newbit = WAV_SPEAKER_BACK_RIGHT;
                        }
                    }
                    _ => {}
                }
                if newbit != 0 {
                    remapping = speaker_bit_to_sdl_channel_index(channels, newbit);
                    standardmap |= newbit; // mark it as used so we don't try to use it for a later missing speaker.
                }
            }

            chmap[current_channel] = remapping;
            current_channel += 1;
            if current_channel >= channels as usize {
                break; // we got them all.
            }
        }
    }

    while current_channel < channels.max(0) as usize {
        chmap[current_channel] = -1; // dump anything that wasn't set up for some reason.
        current_channel += 1;
    }

    let _ = stream.set_input_channel_map(Some(&chmap[..channels.max(0) as usize]));
}

impl AudioData for WavAudioData {
    /// Translation of `WAV_init_track()`.
    fn init_track<'a>(
        self: Arc<Self>,
        io: Option<IoStream<'a>>,
        spec: &AudioSpec,
        _props: &Properties,
    ) -> Result<Box<dyn TrackData + 'a>> {
        let io = io.ok_or_else(|| Error::invalid_param("io"))?;

        let mut state = AdpcmDecoderState::default();
        if is_adpcm(self.encoding) {
            let info = &self.adpcm_info;
            if self.encoding == MS_ADPCM_CODE {
                state.ms_cstate = try_alloc(info.channels as usize)?;
            } else if self.encoding == IMA_ADPCM_CODE {
                state.ima_cstate = try_alloc(info.channels as usize)?;
            } else {
                debug_assert!(false, "WAV: Unexpected ADPCM encoding");
            }

            state.block_size = info.blocksize;
            state.block_data = try_alloc(state.block_size)?;

            state.output_size = info.samplesperblock * info.channels as usize;
            state.output_data = try_alloc(state.output_size)?;
        }

        Ok(Box::new(WavTrackData {
            channels: spec.channels, // !!! FIXME: why aren't we passing the AudioStream in during init_track, so we don't have to do this check?
            adata: self,
            io,
            adpcm_state: state,
            seekblock: 0,
            current_iteration: 0,
            current_iteration_frames: 0,
            must_set_channel_map: true, // !!! FIXME: why aren't we passing the AudioStream in during init_track, so we don't have to do this check?
        }))
    }
}

impl WavTrackData<'_> {
    /// Translation of `FetchADPCM()`.
    fn fetch_adpcm(&mut self, buffer: &mut [u8], ms: bool) -> i32 {
        let info = &self.adata.adpcm_info;
        let state = &mut self.adpcm_state;
        let buflen = buffer.len();
        let mut left = buflen;
        let mut dst = 0usize;

        while left > 0 {
            if state.output_read == state.output_pos {
                let bytesread = self.io.read(&mut state.block_data[..info.blocksize]);
                if bytesread == 0 {
                    return (buflen - left) as i32;
                }

                state.block_size = if bytesread < info.blocksize {
                    bytesread
                } else {
                    info.blocksize
                };
                state.block_pos = 0;
                state.output_pos = 0;
                state.output_read = 0;

                let ok = if ms {
                    ms_adpcm_decode_block_header(info, state)
                        .and_then(|_| ms_adpcm_decode_block_data(info, state))
                } else {
                    ima_adpcm_decode_block_header(info, state)
                        .and_then(|_| ima_adpcm_decode_block_data(info, state))
                };
                if ok.is_err() {
                    return -1;
                }
            }
            let len = left.min((state.output_pos - state.output_read) * 2);
            for (d, s) in buffer[dst..dst + len]
                .chunks_exact_mut(2)
                .zip(&state.output_data[state.output_read..])
            {
                d.copy_from_slice(&s.to_ne_bytes());
            }
            state.output_read += len / 2;
            dst += len;
            left -= len;
            if len == 0 {
                // (an odd buffer length leaves one byte that can't take a
                // sample; upstream copies nothing more and spins.)
                break;
            }
        }

        buflen as i32
    }

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

    /// Translation of `FetchPCM24LE()`.
    fn fetch_pcm24le(&mut self, buffer: &mut [u8]) -> i32 {
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
            let inp = ((x[2] as i8 as i32) << 16) | ((x[1] as i32) << 8) | x[0] as i32;
            let out = (inp as f32) / 8388608.0;
            buffer[o as usize..o as usize + 4].copy_from_slice(&out.to_ne_bytes());
            i -= 3;
            o -= 4;
        }
        ((length / 3) * 4) as i32
    }

    /// Translation of `FetchFloat64LE()`.
    fn fetch_float64le(&mut self, buffer: &mut [u8]) -> i32 {
        let framesize = self.adata.framesize.max(1) as usize;
        let mut length = self.io.read(buffer);
        if length % framesize != 0 {
            length -= length % framesize;
        }
        let mut o = 0;
        for i in (0..length / 8 * 8).step_by(8) {
            let mut d = [0u8; 8];
            d.copy_from_slice(&buffer[i..i + 8]);
            let out = f64::from_le_bytes(d) as f32;
            buffer[o * 4..o * 4 + 4].copy_from_slice(&out.to_ne_bytes());
            o += 1;
        }
        (length / 2) as i32
    }

    /// Call the format's fetch function (`adata->fetch`).
    fn fetch(&mut self, buffer: &mut [u8]) -> i32 {
        match self.adata.fetch {
            Fetch::Pcm => self.fetch_pcm(buffer),
            Fetch::ULaw => self.fetch_xlaw(buffer, &ULAW_TO_FLOAT),
            Fetch::ALaw => self.fetch_xlaw(buffer, &ALAW_TO_FLOAT),
            Fetch::MsAdpcm => self.fetch_adpcm(buffer, true),
            Fetch::ImaAdpcm => self.fetch_adpcm(buffer, false),
            Fetch::Pcm24Le => self.fetch_pcm24le(buffer),
            Fetch::Float64Le => self.fetch_float64le(buffer),
        }
    }
}

/// Translation of `FindWAVSeekBlock()`.
fn find_wav_seek_block(seekblocks: &[WavSeekBlock], ui64frame: u64) -> Result<usize> {
    let frame = ui64frame as i64;
    for (i, seekblock) in seekblocks.iter().enumerate() {
        let frame_start = seekblock.frame_start;
        let num_frames = seekblock.num_frames;
        if num_frames < 0 {
            // infinite loop?
            return Ok(i); // it's in here.
        } else if (frame >= frame_start) && (frame < (frame_start + num_frames)) {
            //  is the target!
            return Ok(i); // it's in here.
        }
    }

    Err(Error::new("Seek past end of file"))
}

impl TrackData for WavTrackData<'_> {
    /// Translation of `WAV_decode()`.
    fn decode(&mut self, stream: &AudioStream) -> bool {
        // !!! FIXME: why aren't we passing the AudioStream in during init_track, so we don't have to do this check?
        if self.must_set_channel_map {
            set_audio_stream_channel_map_for_wav(stream, self.channels, self.adata.channelmask);
            self.must_set_channel_map = false;
        }

        // see if we are at the end of a loop, etc.
        let mut seekblock = self.seekblock;
        while (self.current_iteration_frames as i64) == self.adata.seekblocks[seekblock].num_frames
        {
            //SDL_Log("Decoded to the end of a seekblock! (iteration %d of %d)", (int) tdata->current_iteration, (int) seekblock->iterations);
            let sb = self.adata.seekblocks[seekblock];

            let mut should_loop = false;
            if sb.iterations < 0 {
                // negative==infinite loop
                self.current_iteration = 0;
                should_loop = true;
            } else {
                self.current_iteration = self.current_iteration.wrapping_add(1);
                debug_assert!(self.current_iteration as i64 <= sb.iterations);
                if (self.current_iteration as i64) < sb.iterations {
                    should_loop = true;
                }
            }

            if should_loop {
                let nextframe = (sb.frame_start as u64).wrapping_add(
                    (sb.num_frames as u64).wrapping_mul(self.current_iteration as u64),
                );
                //SDL_Log("Moving back to the start of seekblock for next iteration!");
                if self.seek(nextframe).is_err() {
                    //SDL_Log("SEEK FAILED");
                    return false;
                }
                debug_assert!(self.seekblock == seekblock); // should not have changed.
                debug_assert!(self.current_iteration_frames == 0); // should be at start of loop.
            } else {
                //SDL_Log("That was the last iteration, moving to next seekblock!");
                seekblock += 1;
                if seekblock >= self.adata.seekblocks.len() {
                    // ran out of blocks! EOF!!
                    //SDL_Log("That was the last seekblock, too!");
                    self.current_iteration = self.current_iteration.wrapping_sub(1);
                    return false;
                }
                self.seekblock = seekblock;
                self.current_iteration = 0;
            }
            self.current_iteration_frames = 0;
        }

        let sb = self.adata.seekblocks[seekblock];
        let decoded_framesize = self.adata.decoded_framesize;
        let available_bytes = ((sb.num_frames - self.current_iteration_frames as i64) as u64)
            .wrapping_mul(decoded_framesize as u64);

        // !!! FIXME: looping.
        let mut buffer = [0u8; 1024];
        let mut buflen = buffer.len() as i32;
        let m = buflen % decoded_framesize.max(1);
        if m != 0 {
            buflen -= m;
        }

        buflen = (buflen as i64).min(available_bytes as i64) as i32;
        debug_assert!(buflen > 0); // we should have caught this in the seekblock code.
                                   // FIXME (upstream): a seekblock with a negative length (loops that
                                   // overlap or end past the data) gets here with a negative buffer
                                   // length, which upstream reads into its stack buffer as a huge
                                   // size; that's the end of the data here (and in the patched C
                                   // reference harness).
        if buflen <= 0 {
            return false;
        }

        let br = self.fetch(&mut buffer[..buflen.max(0) as usize]); // this will deal with different formats that might need decompression or conversion.
                                                                    //SDL_Log("Requested %d bytes, read %d bytes (%d frames)!", buflen, br, br / decoded_framesize);
        if br <= 0 {
            return false;
        }

        // update framecount, but we'll actually move to the next seekblock if necessary on the next decode, since we definitely have data to return now.
        self.current_iteration_frames = self
            .current_iteration_frames
            .wrapping_add((br / decoded_framesize.max(1)) as u32);

        let _ = stream.put_data(&buffer[..br as usize]);
        true
    }

    /// Translation of `WAV_seek()`.
    fn seek(&mut self, frame: u64) -> Result<()> {
        let adata = self.adata.clone();

        // figure out if loops change final position...
        let index = find_wav_seek_block(&adata.seekblocks, frame)?;
        let seekblock = adata.seekblocks[index];

        // make frame relative to the start of this seekblock.
        let frame = frame.wrapping_sub(seekblock.frame_start as u64);

        let num_frames = seekblock.num_frames as u64;
        let current_iteration = if seekblock.iterations < 0 || num_frames == 0 {
            0
        } else {
            frame / num_frames
        };
        let current_iteration_frames = if num_frames == 0 {
            frame
        } else {
            frame % num_frames
        };

        // Deal with loop iterations, offset by the modulus of total frames in the loop; frame_start should have already
        //  dealt with iterations, so this shouldn't matter how many iterations there are or if it's an infinite loop.
        let frame = current_iteration_frames;

        if is_adpcm(adata.encoding) {
            let samplesperblock = adata.adpcm_info.samplesperblock as u64;
            let dest_offset =
                ((frame / samplesperblock) as i64).wrapping_mul(adata.adpcm_info.blocksize as i64); // the start of the correct ADPCM block within this seekblock.
            let destpos = seekblock.seek_position.wrapping_add(dest_offset); // seek_position is aligned to the start of an ADPCM block.
            self.io.seek(destpos, IoWhence::Set)?;

            self.adpcm_state.output_pos = 0;
            self.adpcm_state.output_read = 0; // reset this for the new block.

            // We're at the start of the right ADPCM block now; decode and throw away until we hit the exact frame we want to seek to.
            let mut remainder =
                ((frame % samplesperblock) as i32).wrapping_mul(adata.decoded_framesize);
            while remainder > 0 {
                let mut buffer = [0u8; 1024];
                let n = (remainder as usize).min(buffer.len());
                let br = self.fetch(&mut buffer[..n]);
                if br <= 0 {
                    return Err(Error::new("WAV: seek failed"));
                }
                remainder -= br;
            }
        } else {
            let dest_offset = (frame as i64).wrapping_mul(adata.framesize as i64);
            let destpos = seekblock.seek_position.wrapping_add(dest_offset); // seek_position is aligned to start of a sample frame.
            self.io.seek(destpos, IoWhence::Set)?;
        }

        self.seekblock = index;
        self.current_iteration = current_iteration as u32;
        self.current_iteration_frames = current_iteration_frames as u32;

        Ok(())
    }
}

/// Translation of `MIX_Decoder_WAV`.
pub(crate) static DECODER: Decoder = Decoder {
    name: "WAV",
    init: None,
    init_audio: wav_init_audio,
    has_jump_to_order: false,
    quit: None,
};
