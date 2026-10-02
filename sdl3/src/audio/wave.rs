// Rust translation of src/audio/SDL_wave.c and src/audio/SDL_wave.h from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Microsoft WAVE file loading routines.
//!
//! (The `SDL_WAVE_DEBUG_LOG_FORMAT`/`SDL_WAVE_DEBUG_DUMP_FORMAT` debug
//! dumps, compiled out upstream, are not translated. Upstream also has an
//! optional lookup-table A-law/µ-law decoder, `SDL_WAVE_LAW_LUT`, that is
//! off by default; this translates the default computed decoder.)

use super::format::{AudioFormat, AudioSpec};
use crate::error::{Error, Result};
use crate::hints;
use crate::io::{IoStream, IoWhence};

// RIFF WAVE files are little-endian

/*******************************************/
// Define values for Microsoft WAVE format
/*******************************************/
// FOURCC
const RIFF: u32 = 0x4646_4952; // "RIFF"
const WAVE: u32 = 0x4556_4157; // "WAVE"
const FACT: u32 = 0x7463_6166; // "fact"
#[allow(dead_code)]
const LIST: u32 = 0x5453_494c; // "LIST"
#[allow(dead_code)]
const BEXT: u32 = 0x7478_6562; // "bext"
#[allow(dead_code)]
const JUNK: u32 = 0x4B4E_554A; // "JUNK"
const FMT: u32 = 0x2074_6D66; // "fmt "
const DATA: u32 = 0x6174_6164; // "data"
                               // Format tags
const UNKNOWN_CODE: u16 = 0x0000;
const PCM_CODE: u16 = 0x0001;
const MS_ADPCM_CODE: u16 = 0x0002;
const IEEE_FLOAT_CODE: u16 = 0x0003;
const ALAW_CODE: u16 = 0x0006;
const MULAW_CODE: u16 = 0x0007;
const IMA_ADPCM_CODE: u16 = 0x0011;
const MPEG_CODE: u16 = 0x0050;
const MPEGLAYER3_CODE: u16 = 0x0055;
const EXTENSIBLE_CODE: u16 = 0xFFFE;

/// Stores the WAVE format information. Translation of `WaveFormat`.
#[derive(Clone, Copy, Debug, Default)]
struct WaveFormat {
    /// Raw value of the first field in the fmt chunk data.
    formattag: u16,
    /// Actual encoding, possibly from the extensible header.
    encoding: u16,
    /// Number of channels.
    channels: u16,
    /// Sampling rate in Hz.
    frequency: u32,
    /// Average bytes per second.
    byterate: u32,
    /// Bytes per block.
    blockalign: u16,
    /// Currently supported are 8, 16, 24, 32, and 4 for ADPCM.
    bitspersample: u16,

    /// Extra information size. Number of extra bytes starting at byte 18 in the
    /// fmt chunk data. This is at least 22 for the extensible header.
    extsize: u16,

    // Extensible WAVE header fields
    validsamplebits: u16,
    /// For compressed formats. Can be zero. Actually 16 bits in the header.
    samplesperblock: u32,
    channelmask: u32,
    /// A format GUID.
    subformat: [u8; 16],
}

/// Stores information on the fact chunk. Translation of `WaveFact`.
#[derive(Clone, Copy, Debug, Default)]
struct WaveFact {
    /// Represents the state of the fact chunk in the WAVE file.
    /// Set to -1 if the fact chunk is invalid.
    /// Set to 0 if the fact chunk is not present
    /// Set to 1 if the fact chunk is present and valid.
    /// Set to 2 if samplelength is going to be used as the number of sample frames.
    status: i32,

    /// Version 1 of the RIFF specification calls the field in the fact chunk
    /// dwFileSize. The Standards Update then calls it dwSampleLength and specifies
    /// that it is 'the length of the data in samples'. WAVE files from Windows
    /// with this chunk have it set to the samples per channel (sample frames).
    /// This is useful to truncate compressed audio to a specific sample count
    /// because a compressed block is usually decoded to a fixed number of
    /// sample frames.
    samplelength: u32, // Raw sample length value from the fact chunk.
}

/// Generic struct for the chunks in the WAVE file. Translation of `WaveChunk`.
#[derive(Clone, Debug, Default)]
struct WaveChunk {
    /// FOURCC of the chunk.
    fourcc: u32,
    /// Size of the chunk data.
    length: u32,
    /// Position of the data in the stream.
    position: i64,
    /// When allocated, this holds the chunk data; `data.len()` is the number
    /// of bytes that could be read from the stream (`size`), which can be
    /// smaller than length.
    data: Vec<u8>,
}

/// Controls how the size of the RIFF chunk affects the loading of a WAVE
/// file. Translation of `WaveRiffSizeHint`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum WaveRiffSizeHint {
    #[default]
    NoHint,
    Force,
    IgnoreZero,
    Ignore,
    Maximum,
}

/// Controls how a truncated WAVE file is handled. Translation of `WaveTruncationHint`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum WaveTruncationHint {
    #[default]
    NoHint,
    VeryStrict,
    Strict,
    DropFrame,
    DropBlock,
}

/// Controls how the fact chunk affects the loading of a WAVE file.
/// Translation of `WaveFactChunkHint`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum WaveFactChunkHint {
    #[default]
    NoHint,
    Truncate,
    Strict,
    IgnoreZero,
    Ignore,
}

/// Translation of `WaveFile`.
#[derive(Debug, Default)]
struct WaveFile {
    chunk: WaveChunk,
    format: WaveFormat,
    fact: WaveFact,

    /// Number of sample frames that will be decoded. Calculated either with the
    /// size of the data chunk or, if the appropriate hint is enabled, with the
    /// sample length value from the fact chunk.
    sampleframes: i64,

    /// Some decoders require extra data for a state (the MS ADPCM
    /// coefficients, `MS_ADPCM_CoeffData`).
    decoderdata: Vec<i16>,

    riffhint: WaveRiffSizeHint,
    trunchint: WaveTruncationHint,
    facthint: WaveFactChunkHint,
}

impl WaveFile {
    fn strict(&self) -> bool {
        self.trunchint == WaveTruncationHint::VeryStrict
            || self.trunchint == WaveTruncationHint::Strict
    }
}

/// Multiplies `f1` by `f2`, failing if the multiplication overflows (in
/// which case `f1` is not modified). Translation of `SafeMult()`.
fn safe_mult(f1: &mut usize, f2: usize) -> Result<()> {
    if *f1 > 0 && usize::MAX / *f1 <= f2 {
        return Err(Error::new("WAVE file too big"));
    }
    *f1 *= f2;
    Ok(())
}

/// Translation of `ADPCM_DecoderState` (the input/block/output cursors).
struct AdpcmDecoderState<'a> {
    /// Number of channels.
    channels: u32,
    /// Size of an ADPCM block in bytes.
    blocksize: usize,
    /// Size of an ADPCM block header in bytes.
    blockheadersize: usize,
    /// Number of samples per channel in an ADPCM block.
    samplesperblock: usize,
    /// Total number of sample frames.
    #[allow(dead_code)]
    framestotal: i64,
    /// Number of sample frames still to be decoded.
    framesleft: i64,

    // ADPCM data.
    input: &'a [u8],
    input_pos: usize,

    // Current ADPCM block in the ADPCM data above.
    block_start: usize,
    block_size: usize,
    block_pos: usize,

    // Decoded 16-bit PCM data.
    output: Vec<i16>,
    output_pos: usize,
}

impl AdpcmDecoderState<'_> {
    fn block(&self, i: usize) -> u8 {
        self.input[self.block_start + i]
    }
}

/// Translation of `MS_ADPCM_ChannelState`.
#[derive(Clone, Copy, Debug, Default)]
struct MsAdpcmChannelState {
    delta: u16,
    coeff1: i16,
    coeff2: i16,
}

/// Translation of `WaveAdjustToFactValue()`.
fn wave_adjust_to_fact_value(file: &WaveFile, sampleframes: i64) -> Result<i64> {
    if file.fact.status == 2 {
        if file.facthint == WaveFactChunkHint::Strict
            && sampleframes < file.fact.samplelength as i64
        {
            return Err(Error::new(
                "Invalid number of sample frames in WAVE fact chunk (too many)",
            ));
        } else if sampleframes > file.fact.samplelength as i64 {
            return Ok(file.fact.samplelength as i64);
        }
    }

    Ok(sampleframes)
}

/// Translation of `MS_ADPCM_CalculateSampleFrames()`.
fn ms_adpcm_calculate_sample_frames(file: &mut WaveFile, datalength: usize) -> Result<()> {
    let format = file.format;
    let blockheadersize = format.channels as usize * 7;
    let availableblocks = datalength / format.blockalign as usize;
    let blockframebitsize = format.bitspersample as usize * format.channels as usize;
    let trailingdata = datalength % format.blockalign as usize;

    if file.strict() {
        // The size of the data chunk must be a multiple of the block size.
        if datalength < blockheadersize || trailingdata > 0 {
            return Err(Error::new("Truncated MS ADPCM block"));
        }
    }

    // Calculate number of sample frames that will be decoded.
    file.sampleframes = availableblocks as i64 * format.samplesperblock as i64;
    if trailingdata > 0 {
        // The last block is truncated. Check if we can get any samples out of it.
        if file.trunchint == WaveTruncationHint::DropFrame {
            // Drop incomplete sample frame.
            if trailingdata >= blockheadersize {
                let mut trailingsamples =
                    2 + (trailingdata - blockheadersize) * 8 / blockframebitsize;
                if trailingsamples > format.samplesperblock as usize {
                    trailingsamples = format.samplesperblock as usize;
                }
                file.sampleframes += trailingsamples as i64;
            }
        }
    }

    file.sampleframes = wave_adjust_to_fact_value(file, file.sampleframes)?;
    Ok(())
}

/// Translation of `MS_ADPCM_Init()`.
fn ms_adpcm_init(file: &mut WaveFile, datalength: usize) -> Result<()> {
    let format = file.format;
    let chunk = &file.chunk;
    let blockheadersize = format.channels as usize * 7;
    let blockdatasize = (format.blockalign as usize).wrapping_sub(blockheadersize);
    let blockframebitsize = format.bitspersample as usize * format.channels as usize;
    let presetcoeffs: [i16; 14] = [
        256, 0, 512, -256, 0, 0, 192, 64, 240, 0, 460, -208, 392, -232,
    ];

    // Sanity checks.

    /* While it's clear how IMA ADPCM handles more than two channels, the nibble
     * order of MS ADPCM makes it awkward. The Standards Update does not talk
     * about supporting more than stereo anyway.
     */
    if format.channels > 2 {
        return Err(Error::new("Invalid number of channels"));
    }

    if format.bitspersample != 4 {
        return Err(Error::new(format!(
            "Invalid MS ADPCM bits per sample of {}",
            format.bitspersample
        )));
    }

    // The block size must be big enough to contain the block header.
    if (format.blockalign as usize) < blockheadersize {
        return Err(Error::new("Invalid MS ADPCM block size (nBlockAlign)"));
    }
    let blockdatasamples = (blockdatasize * 8) / blockframebitsize;

    if format.formattag == EXTENSIBLE_CODE {
        /* Does have a GUID (like all format tags), but there's no specification
         * for how the data is packed into the extensible header. Making
         * assumptions here could lead to new formats nobody wants to support.
         */
        return Err(Error::new(
            "MS ADPCM with the extensible header is not supported",
        ));
    }

    /* There are wSamplesPerBlock, wNumCoef, and at least 7 coefficient pairs in
     * the extended part of the header.
     */
    if chunk.data.len() < 22 {
        return Err(Error::new("Could not read MS ADPCM format header"));
    }

    let d = &chunk.data;
    file.format.samplesperblock = d[18] as u32 | ((d[19] as u32) << 8);
    // Number of coefficient pairs. A pair has two 16-bit integers.
    let mut coeffcount = d[20] as usize | ((d[21] as usize) << 8);
    /* bPredictor, the integer offset into the coefficients array, is only
     * 8 bits. It can only address the first 256 coefficients. Let's limit
     * the count number here.
     */
    if coeffcount > 256 {
        coeffcount = 256;
    }

    if d.len() < 22 + coeffcount * 4 {
        return Err(Error::new(
            "Could not read custom coefficients in MS ADPCM format header",
        ));
    } else if (format.extsize as usize) < 4 + coeffcount * 4 {
        return Err(Error::new("Invalid MS ADPCM format header (too small)"));
    } else if coeffcount < 7 {
        return Err(Error::new(
            "Missing required coefficients in MS ADPCM format header",
        ));
    }

    // (MS_ADPCM_CoeffData: the pairs, plus one zeroed pair standing in for the
    // struct padding that the block header check's off-by-one can reach.)
    let mut coeff = vec![0i16; coeffcount * 2 + 2];

    // Copy the 16-bit pairs.
    for (i, c_out) in coeff.iter_mut().take(coeffcount * 2).enumerate() {
        let mut c = d[22 + i * 2] as i32 | ((d[23 + i * 2] as i32) << 8);
        if c >= 0x8000 {
            c -= 0x10000;
        }
        if i < 14 && c != presetcoeffs[i] as i32 {
            return Err(Error::new(
                "Wrong preset coefficients in MS ADPCM format header",
            ));
        }
        *c_out = c as i16;
    }
    file.decoderdata = coeff; // Freed in cleanup.

    /* Technically, wSamplesPerBlock is required, but we have all the
     * information in the other fields to calculate it, if it's zero.
     */
    if file.format.samplesperblock == 0 {
        /* Let's be nice to the encoders that didn't know how to fill this.
         * The Standards Update calculates it this way:
         *
         *   x = Block size (in bits) minus header size (in bits)
         *   y = Bit depth multiplied by channel count
         *   z = Number of samples per channel in block header
         *   wSamplesPerBlock = x / y + z
         */
        file.format.samplesperblock = blockdatasamples as u32 + 2;
    }

    /* nBlockAlign can be in conflict with wSamplesPerBlock. For example, if
     * the number of samples doesn't fit into the block. The Standards Update
     * also describes wSamplesPerBlock with a formula that makes it necessary to
     * always fill the block with the maximum amount of samples, but this is not
     * enforced here as there are no compatibility issues.
     * A truncated block header with just one sample is not supported.
     */
    if file.format.samplesperblock == 1
        || blockdatasamples < (file.format.samplesperblock as usize).wrapping_sub(2)
    {
        return Err(Error::new(
            "Invalid number of samples per MS ADPCM block (wSamplesPerBlock)",
        ));
    }

    ms_adpcm_calculate_sample_frames(file, datalength)
}

/// Translation of `MS_ADPCM_ProcessNibble()`.
fn ms_adpcm_process_nibble(
    cstate: &mut MsAdpcmChannelState,
    sample1: i32,
    sample2: i32,
    nybble: u8,
) -> i16 {
    let max_audioval: i32 = 32767;
    let min_audioval: i32 = -32768;
    let max_deltaval: u32 = 65535;
    const ADAPTIVE: [u16; 16] = [
        230, 230, 230, 230, 307, 409, 512, 614, 768, 614, 512, 409, 307, 230, 230, 230,
    ];
    let mut delta = cstate.delta as u32;

    let mut new_sample = (sample1 * cstate.coeff1 as i32 + sample2 * cstate.coeff2 as i32) / 256;
    // The nibble is a signed 4-bit error delta.
    let errordelta = nybble as i32 - if nybble >= 0x08 { 0x10 } else { 0 };
    new_sample += delta as i32 * errordelta;
    new_sample = new_sample.clamp(min_audioval, max_audioval);
    delta = (delta * ADAPTIVE[nybble as usize] as u32) / 256;
    if delta < 16 {
        delta = 16;
    } else if delta > max_deltaval {
        /* This issue is not described in the Standards Update and therefore
         * undefined. It seems sensible to prevent overflows with a limit.
         */
        delta = max_deltaval;
    }

    cstate.delta = delta as u16;
    new_sample as i16
}

/// Translation of `MS_ADPCM_DecodeBlockHeader()`.
fn ms_adpcm_decode_block_header(
    state: &mut AdpcmDecoderState<'_>,
    cstate: &mut [MsAdpcmChannelState; 2],
    ddata: &[i16],
) -> Result<()> {
    let channels = state.channels as usize;
    let coeffcount = (ddata.len() - 2) / 2;

    for (c, cs) in cstate.iter_mut().enumerate().take(channels) {
        let mut o = c;

        // Load the coefficient pair into the channel state.
        let coeffindex = state.block(o) as usize;
        // FIXME (upstream): `>` should be `>=`; an index equal to the count
        // reads the zeroed pad pair after the coefficients.
        if coeffindex > coeffcount {
            return Err(Error::new(
                "Invalid MS ADPCM coefficient index in block header",
            ));
        }
        cs.coeff1 = ddata[coeffindex * 2];
        cs.coeff2 = ddata[coeffindex * 2 + 1];

        // Initial delta value.
        o = channels + c * 2;
        cs.delta = state.block(o) as u16 | ((state.block(o + 1) as u16) << 8);

        /* Load the samples from the header. Interestingly, the sample later in
         * the output stream comes first.
         */
        o = channels * 3 + c * 2;
        let mut sample = state.block(o) as i32 | ((state.block(o + 1) as i32) << 8);
        if sample >= 0x8000 {
            sample -= 0x10000;
        }
        let p = state.output_pos;
        state.output[p + channels] = sample as i16;

        o = channels * 5 + c * 2;
        sample = state.block(o) as i32 | ((state.block(o + 1) as i32) << 8);
        if sample >= 0x8000 {
            sample -= 0x10000;
        }
        state.output[p] = sample as i16;

        state.output_pos += 1;
    }

    state.block_pos += state.blockheadersize;

    // Skip second sample frame that came from the header.
    state.output_pos += channels;

    // Header provided two sample frames.
    state.framesleft -= 2;

    Ok(())
}

/// Decodes the data of the MS ADPCM block. Decoding will stop if a block is
/// too short, returning with none or partially decoded data. The partial
/// data will always contain full sample frames (same sample count for each
/// channel). Incomplete sample frames are discarded.
/// Translation of `MS_ADPCM_DecodeBlockData()`.
fn ms_adpcm_decode_block_data(
    state: &mut AdpcmDecoderState<'_>,
    cstate: &mut [MsAdpcmChannelState; 2],
) -> bool {
    let mut nybble: u16 = 0;
    let channels = state.channels as usize;

    let mut blockpos = state.block_pos;
    let blocksize = state.block_size;

    let mut outpos = state.output_pos;

    let mut blockframesleft = state.samplesperblock as i64 - 2;
    if blockframesleft > state.framesleft {
        blockframesleft = state.framesleft;
    }

    while blockframesleft > 0 {
        for (c, cs) in cstate.iter_mut().enumerate().take(channels) {
            if nybble & 0x4000 != 0 {
                nybble <<= 4;
            } else if blockpos < blocksize {
                nybble = state.block(blockpos) as u16 | 0x4000;
                blockpos += 1;
            } else {
                // Out of input data. Drop the incomplete frame and return.
                state.output_pos = outpos - c;
                return false;
            }

            // Load previous samples which may come from the block header.
            let sample1 = state.output[outpos - channels];
            let sample2 = state.output[outpos - channels * 2];

            let sample1 = ms_adpcm_process_nibble(
                cs,
                sample1 as i32,
                sample2 as i32,
                ((nybble >> 4) & 0x0f) as u8,
            );
            state.output[outpos] = sample1;
            outpos += 1;
        }

        state.framesleft -= 1;
        blockframesleft -= 1;
    }

    state.output_pos = outpos;

    true
}

/// Translation of `MS_ADPCM_Decode()`.
fn ms_adpcm_decode(file: &mut WaveFile) -> Result<Vec<u8>> {
    if file.chunk.data.len() != file.chunk.length as usize {
        // Could not read everything. Recalculate number of sample frames.
        ms_adpcm_calculate_sample_frames(file, file.chunk.data.len())?;
    }

    // Nothing to decode, nothing to return.
    if file.sampleframes == 0 {
        return Ok(Vec::new());
    }

    let channels = file.format.channels as u32;
    let framesize = channels as usize * std::mem::size_of::<i16>();

    // The output size in bytes. May get modified if data is truncated.
    let mut outputsize = file.sampleframes as usize;
    safe_mult(&mut outputsize, framesize)?;
    if outputsize > u32::MAX as usize || file.sampleframes as u64 > usize::MAX as u64 {
        return Err(Error::new("WAVE file too big"));
    }

    let mut state = AdpcmDecoderState {
        channels,
        blocksize: file.format.blockalign as usize,
        blockheadersize: channels as usize * 7,
        samplesperblock: file.format.samplesperblock as usize,
        framestotal: file.sampleframes,
        framesleft: file.sampleframes,
        input: &file.chunk.data,
        input_pos: 0,
        block_start: 0,
        block_size: 0,
        block_pos: 0,
        output: vec![0i16; outputsize / std::mem::size_of::<i16>()],
        output_pos: 0,
    };
    let mut cstate = [MsAdpcmChannelState::default(); 2];

    // Decode block by block. A truncated block will stop the decoding.
    let mut bytesleft = state.input.len() - state.input_pos;
    while state.framesleft > 0 && bytesleft >= state.blockheadersize {
        state.block_start = state.input_pos;
        state.block_size = bytesleft.min(state.blocksize);
        state.block_pos = 0;

        if ((state.output.len() - state.output_pos) as u64)
            < state.framesleft as u64 * state.channels as u64
        {
            // Somehow didn't allocate enough space for the output.
            return Err(Error::new("Unexpected overflow in MS ADPCM decoder"));
        }

        // Initialize decoder with the values from the block header.
        ms_adpcm_decode_block_header(&mut state, &mut cstate, &file.decoderdata)?;

        // Decode the block data. It stores the samples directly in the output.
        if !ms_adpcm_decode_block_data(&mut state, &mut cstate) {
            // Unexpected end. Stop decoding and return partial data if necessary.
            if file.strict() {
                return Err(Error::new("Truncated data chunk"));
            } else if file.trunchint != WaveTruncationHint::DropFrame {
                state.output_pos -=
                    state.output_pos % (state.samplesperblock * state.channels as usize);
            }
            outputsize = state.output_pos * std::mem::size_of::<i16>(); // Can't overflow, is always smaller.
            break;
        }

        state.input_pos += state.block_size;
        bytesleft = state.input.len() - state.input_pos;
    }

    Ok(samples_to_bytes(&state.output, outputsize))
}

/// The first `len` bytes of native-endian 16-bit samples.
fn samples_to_bytes(samples: &[i16], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    for s in samples {
        out.extend_from_slice(&s.to_ne_bytes());
    }
    out.truncate(len);
    out
}

/// Translation of `IMA_ADPCM_CalculateSampleFrames()`.
fn ima_adpcm_calculate_sample_frames(file: &mut WaveFile, datalength: usize) -> Result<()> {
    let format = file.format;
    let blockheadersize = format.channels as usize * 4;
    let subblockframesize = format.channels as usize * 4;
    let availableblocks = datalength / format.blockalign as usize;
    let trailingdata = datalength % format.blockalign as usize;

    if file.strict() {
        // The size of the data chunk must be a multiple of the block size.
        if datalength < blockheadersize || trailingdata > 0 {
            return Err(Error::new("Truncated IMA ADPCM block"));
        }
    }

    // Calculate number of sample frames that will be decoded.
    file.sampleframes = (availableblocks as u64 * format.samplesperblock as u64) as i64;
    if trailingdata > 0 {
        // The last block is truncated. Check if we can get any samples out of it.
        if file.trunchint == WaveTruncationHint::DropFrame && trailingdata > blockheadersize - 2 {
            /* The sample frame in the header of the truncated block is present.
             * Drop incomplete sample frames.
             */
            let mut trailingsamples = 1usize;

            if trailingdata > blockheadersize {
                // More data following after the header.
                let trailingblockdata = trailingdata - blockheadersize;
                let trailingsubblockdata = trailingblockdata % subblockframesize;
                trailingsamples += (trailingblockdata / subblockframesize) * 8;
                /* Due to the interleaved sub-blocks, the last 4 bytes determine
                 * how many samples of the truncated sub-block are lost.
                 */
                if trailingsubblockdata > subblockframesize - 4 {
                    trailingsamples += (trailingsubblockdata % 4) * 2;
                }
            }

            if trailingsamples > format.samplesperblock as usize {
                trailingsamples = format.samplesperblock as usize;
            }
            file.sampleframes += trailingsamples as i64;
        }
    }

    file.sampleframes = wave_adjust_to_fact_value(file, file.sampleframes)?;
    Ok(())
}

/// Translation of `IMA_ADPCM_Init()`.
fn ima_adpcm_init(file: &mut WaveFile, datalength: usize) -> Result<()> {
    let format = file.format;
    let blockheadersize = format.channels as usize * 4;
    let blockdatasize = (format.blockalign as usize).wrapping_sub(blockheadersize);
    let blockframebitsize = format.bitspersample as usize * format.channels as usize;

    // Sanity checks.

    // IMA ADPCM can also have 3-bit samples, but it's not supported by SDL at this time.
    if format.bitspersample == 3 {
        return Err(Error::new("3-bit IMA ADPCM currently not supported"));
    } else if format.bitspersample != 4 {
        return Err(Error::new(format!(
            "Invalid IMA ADPCM bits per sample of {}",
            format.bitspersample
        )));
    }

    /* The block size is required to be a multiple of 4 and it must be able to
     * hold a block header.
     */
    if (format.blockalign as usize) < blockheadersize || format.blockalign % 4 != 0 {
        return Err(Error::new("Invalid IMA ADPCM block size (nBlockAlign)"));
    }
    let blockdatasamples = (blockdatasize * 8) / blockframebitsize;

    if format.formattag == EXTENSIBLE_CODE {
        /* There's no specification for this, but it's basically the same
         * format because the extensible header has wSamplePerBlocks too.
         */
    } else {
        // The Standards Update says there 'should' be 2 bytes for wSamplesPerBlock.
        let d = &file.chunk.data;
        if d.len() >= 20 && format.extsize >= 2 {
            file.format.samplesperblock = d[18] as u32 | ((d[19] as u32) << 8);
        }
    }

    if file.format.samplesperblock == 0 {
        /* Field zero? No problem. We just assume the encoder packed the block.
         * The specification calculates it this way:
         *
         *   x = Block size (in bits) minus header size (in bits)
         *   y = Bit depth multiplied by channel count
         *   z = Number of samples per channel in header
         *   wSamplesPerBlock = x / y + z
         */
        file.format.samplesperblock = blockdatasamples as u32 + 1;
    }

    /* nBlockAlign can be in conflict with wSamplesPerBlock. For example, if
     * the number of samples doesn't fit into the block. The Standards Update
     * also describes wSamplesPerBlock with a formula that makes it necessary
     * to always fill the block with the maximum amount of samples, but this is
     * not enforced here as there are no compatibility issues.
     */
    if blockdatasamples < (file.format.samplesperblock as usize).wrapping_sub(1) {
        return Err(Error::new(
            "Invalid number of samples per IMA ADPCM block (wSamplesPerBlock)",
        ));
    }

    ima_adpcm_calculate_sample_frames(file, datalength)
}

/// Translation of `IMA_ADPCM_ProcessNibble()`.
fn ima_adpcm_process_nibble(cindex: &mut i8, lastsample: i16, nybble: u8) -> i16 {
    let max_audioval: i32 = 32767;
    let min_audioval: i32 = -32768;
    const INDEX_TABLE_4B: [i8; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];
    const STEP_TABLE: [u16; 89] = [
        7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60,
        66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371,
        408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878,
        2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845,
        8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086,
        29794, 32767,
    ];
    let mut index = *cindex;

    // Clamp index into valid range.
    index = index.clamp(0, 88);

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

    let sample = lastsample as i32 + delta;

    // Clamp output sample
    sample.clamp(min_audioval, max_audioval) as i16
}

/// Translation of `IMA_ADPCM_DecodeBlockHeader()`.
fn ima_adpcm_decode_block_header(state: &mut AdpcmDecoderState<'_>, cstate: &mut [i8]) {
    for (c, cs) in cstate.iter_mut().enumerate().take(state.channels as usize) {
        let o = state.block_pos + c * 4;

        // Extract the sample from the header.
        let mut sample = state.block(o) as i32 | ((state.block(o + 1) as i32) << 8);
        if sample >= 0x8000 {
            sample -= 0x10000;
        }
        state.output[state.output_pos] = sample as i16;
        state.output_pos += 1;

        // Channel step index.
        let step = state.block(o + 2) as i16;
        *cs = (if step > 0x80 { step - 0x100 } else { step }) as i8;

        // Reserved byte in block header, should be 0.
        if state.block(o + 3) != 0 { /* Uh oh, corrupt data?  Buggy code? */ }
    }

    state.block_pos += state.blockheadersize;

    // Header provided one sample frame.
    state.framesleft -= 1;
}

/// Decodes the data of the IMA ADPCM block. Decoding will stop if a block is
/// too short, returning with none or partially decoded data. The partial
/// data always contains full sample frames (same sample count for each
/// channel). Incomplete sample frames are discarded.
/// Translation of `IMA_ADPCM_DecodeBlockData()`.
fn ima_adpcm_decode_block_data(state: &mut AdpcmDecoderState<'_>, cstate: &mut [i8]) -> bool {
    let channels = state.channels as usize;
    let subblockframesize = channels * 4;
    let mut result = true;

    let mut blockpos = state.block_pos;
    let blocksize = state.block_size;
    let blockleft = blocksize - blockpos;

    let mut outpos = state.output_pos;

    let mut blockframesleft = state.samplesperblock as i64 - 1;
    if blockframesleft > state.framesleft {
        blockframesleft = state.framesleft;
    }

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
        result = false;
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

        for (c, cs) in cstate.iter_mut().enumerate().take(channels) {
            let mut nybble: u8 = 0;
            // Load previous sample which may come from the block header.
            let mut sample = state.output[outpos + c - channels];

            for i in 0..subblocksamples {
                if i & 1 != 0 {
                    nybble >>= 4;
                } else {
                    nybble = state.block(blockpos);
                    blockpos += 1;
                }

                sample = ima_adpcm_process_nibble(cs, sample, nybble & 0x0f);
                state.output[outpos + c + i * channels] = sample;
            }
        }

        outpos += channels * subblocksamples;
        state.framesleft -= subblocksamples as i64;
        blockframesleft -= subblocksamples as i64;
    }

    state.block_pos = blockpos;
    state.output_pos = outpos;

    result
}

/// Translation of `IMA_ADPCM_Decode()`.
fn ima_adpcm_decode(file: &mut WaveFile) -> Result<Vec<u8>> {
    if file.chunk.data.len() != file.chunk.length as usize {
        // Could not read everything. Recalculate number of sample frames.
        ima_adpcm_calculate_sample_frames(file, file.chunk.data.len())?;
    }

    // Nothing to decode, nothing to return.
    if file.sampleframes == 0 {
        return Ok(Vec::new());
    }

    let channels = file.format.channels as u32;
    let framesize = channels as usize * std::mem::size_of::<i16>();

    // The output size in bytes. May get modified if data is truncated.
    let mut outputsize = file.sampleframes as usize;
    safe_mult(&mut outputsize, framesize)?;
    if outputsize > u32::MAX as usize || file.sampleframes as u64 > usize::MAX as u64 {
        return Err(Error::new("WAVE file too big"));
    }

    let mut state = AdpcmDecoderState {
        channels,
        blocksize: file.format.blockalign as usize,
        blockheadersize: channels as usize * 4,
        samplesperblock: file.format.samplesperblock as usize,
        framestotal: file.sampleframes,
        framesleft: file.sampleframes,
        input: &file.chunk.data,
        input_pos: 0,
        block_start: 0,
        block_size: 0,
        block_pos: 0,
        output: vec![0i16; outputsize / std::mem::size_of::<i16>()],
        output_pos: 0,
    };
    let mut cstate = vec![0i8; channels as usize];

    // Decode block by block. A truncated block will stop the decoding.
    let mut bytesleft = state.input.len() - state.input_pos;
    while state.framesleft > 0 && bytesleft >= state.blockheadersize {
        state.block_start = state.input_pos;
        state.block_size = bytesleft.min(state.blocksize);
        state.block_pos = 0;

        if ((state.output.len() - state.output_pos) as u64)
            < state.framesleft as u64 * state.channels as u64
        {
            // Somehow didn't allocate enough space for the output.
            return Err(Error::new("Unexpected overflow in IMA ADPCM decoder"));
        }

        // Initialize decoder with the values from the block header.
        ima_adpcm_decode_block_header(&mut state, &mut cstate);
        // Decode the block data. It stores the samples directly in the output.
        let result = ima_adpcm_decode_block_data(&mut state, &mut cstate);

        if !result {
            // Unexpected end. Stop decoding and return partial data if necessary.
            if file.strict() {
                return Err(Error::new("Truncated data chunk"));
            } else if file.trunchint != WaveTruncationHint::DropFrame {
                state.output_pos -=
                    state.output_pos % (state.samplesperblock * state.channels as usize);
            }
            outputsize = state.output_pos * std::mem::size_of::<i16>(); // Can't overflow, is always smaller.
            break;
        }

        state.input_pos += state.block_size;
        bytesleft = state.input.len() - state.input_pos;
    }

    Ok(samples_to_bytes(&state.output, outputsize))
}

/// Translation of `LAW_Init()`.
fn law_init(file: &mut WaveFile, datalength: usize) -> Result<()> {
    let format = file.format;

    // Standards Update requires this to be 8.
    if format.bitspersample != 8 {
        return Err(Error::new(format!(
            "Invalid companded bits per sample of {}",
            format.bitspersample
        )));
    }

    // Not going to bother with weird padding.
    if format.blockalign != format.channels {
        return Err(Error::new("Unsupported block alignment"));
    }

    if file.strict() && format.blockalign > 1 && datalength % format.blockalign as usize != 0 {
        return Err(Error::new("Truncated data chunk in WAVE file"));
    }

    file.sampleframes =
        wave_adjust_to_fact_value(file, (datalength / format.blockalign as usize) as i64)?;
    Ok(())
}

/// Translation of `LAW_Decode()` (the computed decoder; see the module docs).
fn law_decode(file: &mut WaveFile) -> Result<Vec<u8>> {
    let format = file.format;

    if file.chunk.length as usize != file.chunk.data.len() {
        file.sampleframes = wave_adjust_to_fact_value(
            file,
            (file.chunk.data.len() / format.blockalign as usize) as i64,
        )?;
    }

    // Nothing to decode, nothing to return.
    if file.sampleframes == 0 {
        return Ok(Vec::new());
    }

    let mut sample_count = file.sampleframes as usize;
    safe_mult(&mut sample_count, format.channels as usize)?;

    let mut expanded_len = sample_count;
    safe_mult(&mut expanded_len, std::mem::size_of::<i16>())?;
    if expanded_len > u32::MAX as usize || file.sampleframes as u64 > usize::MAX as u64 {
        return Err(Error::new("WAVE file too big"));
    }

    let mut src = std::mem::take(&mut file.chunk.data);
    src.resize(expanded_len.max(1), 0); // 1 to avoid allocating zero bytes, to keep static analysis happy.

    /* Work backwards, since we're expanding in-place. `format` will
     * inform the caller about the byte order.
     */
    let put = |src: &mut Vec<u8>, i: usize, v: i16| {
        src[i * 2..i * 2 + 2].copy_from_slice(&v.to_ne_bytes())
    };
    match format.encoding {
        ALAW_CODE => {
            for i in (0..sample_count).rev() {
                let nibble = src[i];
                let mut exponent = (nibble & 0x7f) ^ 0x55;
                let mut mantissa = (exponent & 0xf) as i16;

                exponent >>= 4;
                if exponent > 0 {
                    mantissa |= 0x10;
                }
                mantissa = (mantissa << 4) | 0x8;
                if exponent > 1 {
                    mantissa <<= exponent - 1;
                }

                put(
                    &mut src,
                    i,
                    if nibble & 0x80 != 0 {
                        mantissa
                    } else {
                        -mantissa
                    },
                );
            }
        }
        MULAW_CODE => {
            for i in (0..sample_count).rev() {
                let nibble = !src[i];
                let mut mantissa = (nibble & 0xf) as i16;
                let exponent = (nibble >> 4) & 0x7;
                let step = (4i32 << (exponent + 1)) as i16;

                mantissa = ((0x80i32 << exponent) + step as i32 * mantissa as i32 + step as i32 / 2
                    - 132) as i16;

                put(
                    &mut src,
                    i,
                    if nibble & 0x80 != 0 {
                        -mantissa
                    } else {
                        mantissa
                    },
                );
            }
        }
        _ => return Err(Error::new("Unknown companded encoding")),
    }

    src.truncate(expanded_len);
    Ok(src)
}

/// Translation of `PCM_Init()`.
fn pcm_init(file: &mut WaveFile, datalength: usize) -> Result<()> {
    let format = file.format;

    if format.encoding == PCM_CODE {
        match format.bitspersample {
            8 | 16 | 24 | 32 => {} // These are supported.
            bits => return Err(Error::new(format!("{bits}-bit PCM format not supported"))),
        }
    } else if format.encoding == IEEE_FLOAT_CODE && format.bitspersample != 32 {
        return Err(Error::new(format!(
            "{}-bit IEEE floating-point format not supported",
            format.bitspersample
        )));
    }

    /* It wouldn't be that hard to support more exotic block sizes, but
     * the most common formats should do for now.
     */
    // Make sure we're a multiple of the blockalign, at least.
    if (format.channels as u32 * format.bitspersample as u32) % (format.blockalign as u32 * 8) != 0
    {
        return Err(Error::new("Unsupported block alignment"));
    }

    if file.strict() && format.blockalign > 1 && datalength % format.blockalign as usize != 0 {
        return Err(Error::new("Truncated data chunk in WAVE file"));
    }

    file.sampleframes =
        wave_adjust_to_fact_value(file, (datalength / format.blockalign as usize) as i64)?;
    Ok(())
}

/// Translation of `PCM_ConvertSint24ToSint32()`.
fn pcm_convert_sint24_to_sint32(file: &mut WaveFile) -> Result<Vec<u8>> {
    let format = file.format;

    let mut sample_count = file.sampleframes as usize;
    safe_mult(&mut sample_count, format.channels as usize)?;

    let mut expanded_len = sample_count;
    safe_mult(&mut expanded_len, std::mem::size_of::<i32>())?;
    if expanded_len > u32::MAX as usize || file.sampleframes as u64 > usize::MAX as u64 {
        return Err(Error::new("WAVE file too big"));
    }

    let mut ptr = std::mem::take(&mut file.chunk.data);
    ptr.resize(expanded_len.max(1), 0); // 1 to avoid allocating zero bytes, to keep static analysis happy.

    // work from end to start, since we're expanding in-place.
    for o in (0..sample_count).rev() {
        let b = [0, ptr[o * 3], ptr[o * 3 + 1], ptr[o * 3 + 2]];
        ptr[o * 4..o * 4 + 4].copy_from_slice(&b);
    }

    ptr.truncate(expanded_len);
    Ok(ptr)
}

/// Translation of `PCM_Decode()`.
fn pcm_decode(file: &mut WaveFile) -> Result<Vec<u8>> {
    let format = file.format;

    if file.chunk.length as usize != file.chunk.data.len() {
        file.sampleframes = wave_adjust_to_fact_value(
            file,
            (file.chunk.data.len() / format.blockalign as usize) as i64,
        )?;
    }

    // Nothing to decode, nothing to return.
    if file.sampleframes == 0 {
        return Ok(Vec::new());
    }

    // 24-bit samples get shifted to 32 bits.
    if format.encoding == PCM_CODE && format.bitspersample == 24 {
        return pcm_convert_sint24_to_sint32(file);
    }

    let mut outputsize = file.sampleframes as usize;
    safe_mult(&mut outputsize, format.blockalign as usize)?;
    if outputsize > u32::MAX as usize || file.sampleframes as u64 > usize::MAX as u64 {
        return Err(Error::new("WAVE file too big"));
    }

    // This buffer is going to be returned to the caller. Prevent free in cleanup.
    let mut data = std::mem::take(&mut file.chunk.data);
    // (the returned length can be shorter than the buffer; C trims nothing
    // but reports `outputsize`.)
    data.truncate(outputsize);
    Ok(data)
}

/// Translation of `WaveGetRiffSizeHint()`.
fn wave_get_riff_size_hint() -> WaveRiffSizeHint {
    match hints::get(hints::WAVE_RIFF_CHUNK_SIZE).as_deref() {
        Some("force") => WaveRiffSizeHint::Force,
        Some("ignore") => WaveRiffSizeHint::Ignore,
        Some("ignorezero") => WaveRiffSizeHint::IgnoreZero,
        Some("maximum") => WaveRiffSizeHint::Maximum,
        _ => WaveRiffSizeHint::NoHint,
    }
}

/// Translation of `WaveGetTruncationHint()`.
fn wave_get_truncation_hint() -> WaveTruncationHint {
    match hints::get(hints::WAVE_TRUNCATION).as_deref() {
        Some("verystrict") => WaveTruncationHint::VeryStrict,
        Some("strict") => WaveTruncationHint::Strict,
        Some("dropframe") => WaveTruncationHint::DropFrame,
        Some("dropblock") => WaveTruncationHint::DropBlock,
        _ => WaveTruncationHint::NoHint,
    }
}

/// Translation of `WaveGetFactChunkHint()`.
fn wave_get_fact_chunk_hint() -> WaveFactChunkHint {
    match hints::get(hints::WAVE_FACT_CHUNK).as_deref() {
        Some("truncate") => WaveFactChunkHint::Truncate,
        Some("strict") => WaveFactChunkHint::Strict,
        Some("ignorezero") => WaveFactChunkHint::IgnoreZero,
        Some("ignore") => WaveFactChunkHint::Ignore,
        _ => WaveFactChunkHint::NoHint,
    }
}

/// Why [`wave_next_chunk`] / [`wave_read_partial_chunk_data`] failed
/// (their -1 and -2 results).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ChunkError {
    /// -1: overflow, short read or allocation failure.
    Read,
    /// -2: Not sure how we ended up here. Just abort.
    Seek,
}

/// Translation of `WaveNextChunk()`.
fn wave_next_chunk(
    src: &mut IoStream<'_>,
    chunk: &mut WaveChunk,
) -> std::result::Result<(), ChunkError> {
    let mut nextposition = chunk.position.wrapping_add(chunk.length as i64);

    // Data is no longer valid after this function returns.
    chunk.data = Vec::new(); // WaveFreeChunkData()

    // Error on overflows.
    if i64::MAX - (chunk.length as i64) < chunk.position || i64::MAX - 8 < nextposition {
        return Err(ChunkError::Read);
    }

    // RIFF chunks have a 2-byte alignment. Skip padding byte.
    if chunk.length & 1 != 0 {
        nextposition += 1;
    }

    if src.seek(nextposition, IoWhence::Set).ok() != Some(nextposition) {
        // Not sure how we ended up here. Just abort.
        return Err(ChunkError::Seek);
    }
    let mut chunkheader = [0u8; 8];
    if src.read(&mut chunkheader) != 8 {
        return Err(ChunkError::Read);
    }

    chunk.fourcc = u32::from_le_bytes([
        chunkheader[0],
        chunkheader[1],
        chunkheader[2],
        chunkheader[3],
    ]);
    chunk.length = u32::from_le_bytes([
        chunkheader[4],
        chunkheader[5],
        chunkheader[6],
        chunkheader[7],
    ]);
    chunk.position = nextposition + 8;

    Ok(())
}

/// Translation of `WaveReadPartialChunkData()`.
fn wave_read_partial_chunk_data(
    src: &mut IoStream<'_>,
    chunk: &mut WaveChunk,
    length: usize,
) -> std::result::Result<(), ChunkError> {
    chunk.data = Vec::new(); // WaveFreeChunkData()

    let length = length.min(chunk.length as usize);

    if length > 0 {
        let mut data = vec![0u8; length];

        if src.seek(chunk.position, IoWhence::Set).ok() != Some(chunk.position) {
            // Not sure how we ended up here. Just abort.
            return Err(ChunkError::Seek);
        }

        let size = src.read(&mut data);
        // (a short read is expected to be handled by the caller.)
        data.truncate(size);
        chunk.data = data;
    }

    Ok(())
}

/// Translation of `WaveReadChunkData()`.
fn wave_read_chunk_data(
    src: &mut IoStream<'_>,
    chunk: &mut WaveChunk,
) -> std::result::Result<(), ChunkError> {
    let len = chunk.length as usize;
    wave_read_partial_chunk_data(src, chunk, len)
}

/// Some of the GUIDs that are used by WAVEFORMATEXTENSIBLE.
/// Translation of `WAVE_FORMATTAG_GUID()`.
const fn wave_formattag_guid(tag: u16) -> [u8; 16] {
    [
        (tag & 0xff) as u8,
        (tag >> 8) as u8,
        0,
        0,
        0,
        0,
        16,
        0,
        128,
        0,
        0,
        170,
        0,
        56,
        155,
        113,
    ]
}

/// Translation of `extensible_guids`.
static EXTENSIBLE_GUIDS: [(u16, [u8; 16]); 6] = [
    (PCM_CODE, wave_formattag_guid(PCM_CODE)),
    (MS_ADPCM_CODE, wave_formattag_guid(MS_ADPCM_CODE)),
    (IEEE_FLOAT_CODE, wave_formattag_guid(IEEE_FLOAT_CODE)),
    (ALAW_CODE, wave_formattag_guid(ALAW_CODE)),
    (MULAW_CODE, wave_formattag_guid(MULAW_CODE)),
    (IMA_ADPCM_CODE, wave_formattag_guid(IMA_ADPCM_CODE)),
];

/// Translation of `WaveGetFormatGUIDEncoding()`.
fn wave_get_format_guid_encoding(format: &WaveFormat) -> u16 {
    EXTENSIBLE_GUIDS
        .iter()
        .find(|(_, guid)| *guid == format.subformat)
        .map_or(UNKNOWN_CODE, |&(encoding, _)| encoding)
}

/// Translation of `WaveReadFormat()`.
fn wave_read_format(file: &mut WaveFile) -> Result<()> {
    let fmtlen = file.chunk.data.len();

    if fmtlen > i32::MAX as usize {
        // Limit given by SDL_IOFromConstMem.
        return Err(Error::new("Data of WAVE fmt chunk too big"));
    }
    let mut fmtsrc = IoStream::from_const_mem(&file.chunk.data);
    let format = &mut file.format;

    format.formattag = fmtsrc.read_u16_le()?;
    format.channels = fmtsrc.read_u16_le()?;
    format.frequency = fmtsrc.read_u32_le()?;
    format.byterate = fmtsrc.read_u32_le()?;
    format.blockalign = fmtsrc.read_u16_le()?;
    format.encoding = format.formattag;

    // This is PCM specific in the first version of the specification.
    if fmtlen >= 16 {
        format.bitspersample = fmtsrc.read_u16_le()?;
    } else if format.encoding == PCM_CODE {
        return Err(Error::new("Missing wBitsPerSample field in WAVE fmt chunk"));
    }

    // The earlier versions also don't have this field.
    if fmtlen >= 18 {
        format.extsize = fmtsrc.read_u16_le()?;
    }

    if format.formattag == EXTENSIBLE_CODE {
        /* note that this ignores channel masks, smaller valid bit counts
         * inside a larger container, and most subtypes. This is just enough
         * to get things that didn't really _need_ WAVE_FORMAT_EXTENSIBLE
         * to be useful working when they use this format flag.
         */

        // Extensible header must be at least 22 bytes.
        if fmtlen < 40 || format.extsize < 22 {
            return Err(Error::new("Extensible WAVE header too small"));
        }

        // (read failures here are ignored, as upstream)
        if let Ok(v) = fmtsrc.read_u16_le() {
            format.validsamplebits = v;
            if let Ok(m) = fmtsrc.read_u32_le() {
                format.channelmask = m;
                let _ = fmtsrc.read(&mut format.subformat);
            }
        }
        format.samplesperblock = format.validsamplebits as u32;
        format.encoding = wave_get_format_guid_encoding(format);
    }

    Ok(())
}

/// Translation of `WaveCheckFormat()`.
fn wave_check_format(file: &mut WaveFile, datalength: usize) -> Result<()> {
    let format = &mut file.format;

    // Check for some obvious issues.

    if format.channels == 0 {
        return Err(Error::new("Invalid number of channels"));
    }

    if format.frequency == 0 {
        return Err(Error::new("Invalid sample rate"));
    } else if format.frequency > i32::MAX as u32 {
        return Err(Error::new(format!(
            "Sample rate exceeds limit of {}",
            i32::MAX
        )));
    }

    // Reject invalid fact chunks in strict mode.
    if file.facthint == WaveFactChunkHint::Strict && file.fact.status == -1 {
        return Err(Error::new("Invalid fact chunk in WAVE file"));
    }

    /* Check for issues common to all encodings. Some unsupported formats set
     * the bits per sample to zero. These fall through to the 'unsupported
     * format' error.
     */
    match format.encoding {
        IEEE_FLOAT_CODE | ALAW_CODE | MULAW_CODE | MS_ADPCM_CODE | IMA_ADPCM_CODE | PCM_CODE => {
            if format.encoding != PCM_CODE {
                // These formats require a fact chunk.
                if file.facthint == WaveFactChunkHint::Strict && file.fact.status <= 0 {
                    return Err(Error::new("Missing fact chunk in WAVE file"));
                }
            }
            // All supported formats require a non-zero bit depth.
            if file.chunk.data.len() < 16 {
                return Err(Error::new("Missing wBitsPerSample field in WAVE fmt chunk"));
            } else if format.bitspersample == 0 {
                return Err(Error::new("Invalid bits per sample"));
            }

            // All supported formats must have a proper block size.
            if format.blockalign == 0 {
                format.blockalign = 1; // force it to 1 if it was unset.
            }

            /* If the fact chunk is valid and the appropriate hint is set, the
             * decoders will use the number of sample frames from the fact chunk.
             */
            if file.fact.status == 1 {
                let hint = file.facthint;
                let samples = file.fact.samplelength;
                if hint == WaveFactChunkHint::Truncate
                    || hint == WaveFactChunkHint::Strict
                    || (hint == WaveFactChunkHint::IgnoreZero && samples > 0)
                {
                    file.fact.status = 2;
                }
            }
        }
        _ => {}
    }

    // Check the format for encoding specific issues and initialize decoders.
    match file.format.encoding {
        PCM_CODE | IEEE_FLOAT_CODE => pcm_init(file, datalength),
        ALAW_CODE | MULAW_CODE => law_init(file, datalength),
        MS_ADPCM_CODE => ms_adpcm_init(file, datalength),
        IMA_ADPCM_CODE => ima_adpcm_init(file, datalength),
        MPEG_CODE | MPEGLAYER3_CODE => Err(Error::new("MPEG formats not supported")),
        encoding => {
            let format = &file.format;
            if format.formattag == EXTENSIBLE_CODE {
                let g = &format.subformat;
                let g1 = g[0] as u32
                    | ((g[1] as u32) << 8)
                    | ((g[2] as u32) << 16)
                    | ((g[3] as u32) << 24);
                let g2 = g[4] as u32 | ((g[5] as u32) << 8);
                let g3 = g[6] as u32 | ((g[7] as u32) << 8);
                return Err(Error::new(format!(
                    "Unknown WAVE format GUID: {g1:08x}-{g2:04x}-{g3:04x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                    g[8], g[9], g[10], g[11], g[12], g[13], g[14], g[15]
                )));
            }
            Err(Error::new(format!(
                "Unknown WAVE format tag: 0x{encoding:04x}"
            )))
        }
    }
}

/// Translation of `WaveLoad()`: the spec and data, or an error. The end
/// position for the cleanup code is left in `file.chunk.position`.
fn wave_load(src: &mut IoStream<'_>, file: &mut WaveFile) -> Result<(AudioSpec, Vec<u8>)> {
    let mut chunkcount: u32 = 0;
    let mut chunkcountlimit: u32 = 10000;
    let flen = src.size().unwrap_or(-1); // this might be -1 if the IOStream can't determine the total size.
    let mut riff_length_known = false;
    let mut riffchunk = WaveChunk::default();
    let mut fmtchunk = WaveChunk::default();
    let mut datachunk = WaveChunk::default();

    if let Some(hint) = hints::get(hints::WAVE_CHUNK_LIMIT) {
        // (SDL_sscanf "%u")
        let (count, used) = crate::stdlib::string::strtoul(hint.trim_start(), 10);
        if used > 0 && !hint.trim_start().starts_with('-') {
            chunkcountlimit = count.min(u32::MAX as u64) as u32;
        }
    }

    let riffstart = src
        .tell()
        .map_err(|_| Error::new("Could not seek in file"))?;
    if riffstart < 0 {
        return Err(Error::new("Could not seek in file"));
    }

    riffchunk.position = riffstart;
    if wave_next_chunk(src, &mut riffchunk).is_err() {
        return Err(Error::new("Could not read RIFF header"));
    }

    // Check main WAVE file identifiers.
    if riffchunk.fourcc == RIFF {
        // Read the form type. "WAVE" expected.
        match src.read_u32_le() {
            Err(_) => return Err(Error::new("Could not read RIFF form type")),
            Ok(formtype) if formtype != WAVE => {
                return Err(Error::new(
                    "RIFF form type is not WAVE (not a Waveform file)",
                ));
            }
            Ok(_) => {}
        }
    } else if riffchunk.fourcc == WAVE {
        // RIFF chunk missing or skipped. Length unknown.
        riffchunk.position = 0;
        riffchunk.length = 0;
    } else {
        return Err(Error::new(
            "Could not find RIFF or WAVE identifiers (not a Waveform file)",
        ));
    }

    // The 4-byte form type is immediately followed by the first chunk.
    file.chunk.position = riffchunk.position + 4;

    /* Use the RIFF chunk size to limit the search for the chunks. This is not
     * always reliable and the hint can be used to tune the behavior. By
     * default, it will never search past 4 GiB.
     */
    let riffend: i64 = match file.riffhint {
        WaveRiffSizeHint::Ignore => riffchunk.position + u32::MAX as i64,
        WaveRiffSizeHint::Maximum => i64::MAX,
        WaveRiffSizeHint::IgnoreZero | WaveRiffSizeHint::NoHint if riffchunk.length == 0 => {
            riffchunk.position + u32::MAX as i64
        }
        // (RiffSizeIgnoreZero with a non-zero length, the default, and RiffSizeForce)
        _ => {
            riff_length_known = true;
            riffchunk.position + riffchunk.length as i64
        }
    };

    /* Step through all chunks and save information on the fmt, data, and fact
     * chunks. Ignore the chunks we don't know as per specification. This
     * currently also ignores cue, list, and inst chunks.
     */
    while (riffend as u64)
        > (file.chunk.position as u64)
            .wrapping_add(file.chunk.length as u64)
            .wrapping_add((file.chunk.length & 1) as u64)
    {
        // Abort after too many chunks or else corrupt files may waste time.
        if chunkcount >= chunkcountlimit {
            return Err(Error::new(format!(
                "Chunk count in WAVE file exceeds limit of {chunkcountlimit}"
            )));
        }
        chunkcount += 1;

        if wave_next_chunk(src, &mut file.chunk).is_err() {
            // Unexpected EOF. Corrupt file or I/O issues.
            if file.trunchint == WaveTruncationHint::VeryStrict {
                return Err(Error::new("Unexpected end of WAVE file"));
            }
            // Let the checks after this loop sort this issue out.
            // (upstream's separate `result == -2` "Could not seek to WAVE
            // chunk header" branch is unreachable, as -2 < 0.)
            break;
        }

        let chunk = &mut file.chunk;
        if chunk.fourcc == FMT {
            if fmtchunk.fourcc == FMT {
                // Multiple fmt chunks. Ignore or error?
            } else {
                // The fmt chunk must occur before the data chunk.
                if datachunk.fourcc == DATA {
                    return Err(Error::new("fmt chunk after data chunk in WAVE file"));
                }
                fmtchunk = chunk.clone();
            }
        } else if chunk.fourcc == DATA {
            /* If the data chunk is bigger than the file, it might be corrupt
            or the file is truncated. Try to recover by clamping the file
            size. This also means a malicious file can't allocate 4 gigabytes
            for the chunks without actually supplying a 4 gigabyte file. */
            if flen > 0 && (chunk.position + chunk.length as i64) > flen {
                chunk.length = (flen - chunk.position) as u32;
            }

            /* Only use the first data chunk. Handling the wavl list madness
             * may require a different approach.
             */
            if datachunk.fourcc != DATA {
                datachunk = chunk.clone();
            }
        } else if chunk.fourcc == FACT {
            /* The fact chunk data must be at least 4 bytes for the
             * dwSampleLength field. Ignore all fact chunks after the first one.
             */
            if file.fact.status == 0 {
                if chunk.length < 4 {
                    file.fact.status = -1;
                } else {
                    // Let's use src directly, it's just too convenient.
                    let position = src.seek(chunk.position, IoWhence::Set).ok();
                    match (position == Some(chunk.position)).then(|| src.read_u32_le()) {
                        Some(Ok(samplelength)) => {
                            file.fact.samplelength = samplelength;
                            file.fact.status = 1;
                        }
                        _ => file.fact.status = -1,
                    }
                }
            }
        }

        /* Go through all chunks in verystrict mode or stop the search early if
         * all required chunks were found.
         */
        let chunk = &file.chunk;
        if file.trunchint == WaveTruncationHint::VeryStrict {
            if (riffend as u64) < (chunk.position as u64).wrapping_add(chunk.length as u64) {
                return Err(Error::new("RIFF size truncates chunk"));
            }
        } else if fmtchunk.fourcc == FMT
            && datachunk.fourcc == DATA
            && (file.fact.status == 1
                || file.facthint == WaveFactChunkHint::Ignore
                || file.facthint == WaveFactChunkHint::NoHint)
        {
            break;
        }
    }

    /* Save the position after the last chunk. This position will be used if the
     * RIFF length is unknown.
     */
    let lastchunkpos = file.chunk.position + file.chunk.length as i64;

    // The fmt chunk is mandatory.
    if fmtchunk.fourcc != FMT {
        return Err(Error::new("Missing fmt chunk in WAVE file"));
    }
    // A data chunk must be present.
    if datachunk.fourcc != DATA {
        return Err(Error::new("Missing data chunk in WAVE file"));
    }
    // Check if the last chunk has all of its data in verystrict mode.
    if file.trunchint == WaveTruncationHint::VeryStrict {
        // data chunk is handled later.
        let chunk = &file.chunk;
        if chunk.fourcc != DATA && chunk.length > 0 {
            let position = chunk.position as u64 + chunk.length as u64 - 1;
            if position > i64::MAX as u64
                || src.seek(position as i64, IoWhence::Set).ok() != Some(position as i64)
            {
                return Err(Error::new("Could not seek to WAVE chunk data"));
            } else if src.read_u8().is_err() {
                return Err(Error::new("RIFF size truncates chunk"));
            }
        }
    }

    // Process fmt chunk.
    file.chunk = fmtchunk.clone();

    /* No need to read more than 1046 bytes of the fmt chunk data with the
     * formats that are currently supported. (1046 because of MS ADPCM coefficients)
     */
    if wave_read_partial_chunk_data(src, &mut file.chunk, 1046).is_err() {
        return Err(Error::new("Could not read data of WAVE fmt chunk"));
    }

    /* The fmt chunk data must be at least 14 bytes to include all common fields.
     * It usually is 16 and larger depending on the header and encoding.
     */
    if file.chunk.length < 14 {
        return Err(Error::new("Invalid WAVE fmt chunk length (too small)"));
    } else if file.chunk.data.len() < 14 {
        return Err(Error::new("Could not read data of WAVE fmt chunk"));
    }
    wave_read_format(file)?;
    wave_check_format(file, datachunk.length as usize)?;

    // Process data chunk.
    file.chunk = datachunk;

    if file.chunk.length > 0 {
        match wave_read_chunk_data(src, &mut file.chunk) {
            Ok(()) => {}
            // (upstream checks `result < 0` first, so -2 never reaches its
            // "Could not seek data of WAVE data chunk" message either)
            Err(_) => return Err(Error::new("Could not read data of WAVE data chunk")),
        }
    }

    if file.chunk.length as usize != file.chunk.data.len() {
        // I/O issues or corrupt file.
        if file.strict() {
            return Err(Error::new("Could not read data of WAVE data chunk"));
        }
        // The decoders handle this truncation.
    }

    // Decode or convert the data if necessary.
    let audio = match file.format.encoding {
        PCM_CODE | IEEE_FLOAT_CODE => pcm_decode(file)?,
        ALAW_CODE | MULAW_CODE => law_decode(file)?,
        MS_ADPCM_CODE => ms_adpcm_decode(file)?,
        IMA_ADPCM_CODE => ima_adpcm_decode(file)?,
        _ => Vec::new(),
    };

    /* Setting up the specs. All unsupported formats were filtered out
     * by checks earlier in this function.
     */
    let format = &file.format;
    let spec_format = match format.encoding {
        // These can be easily stored in the byte order of the system.
        MS_ADPCM_CODE | IMA_ADPCM_CODE | ALAW_CODE | MULAW_CODE => AudioFormat::S16,
        IEEE_FLOAT_CODE => AudioFormat::F32LE,
        PCM_CODE => match format.bitspersample {
            8 => AudioFormat::U8,
            16 => AudioFormat::S16LE,
            24 | 32 => AudioFormat::S32LE, // (24 has been shifted to 32 bits.)
            // Just in case something unexpected happened in the checks.
            bits => return Err(Error::new(format!("Unexpected {bits}-bit PCM data format"))),
        },
        _ => return Err(Error::new("Unexpected data format")),
    };
    let spec = AudioSpec {
        freq: format.frequency as i32,
        channels: format.channels as u8 as i32,
        format: spec_format,
    };

    // Report the end position back to the cleanup code.
    file.chunk.position = if riff_length_known {
        riffend
    } else {
        lastchunkpos
    };

    Ok((spec, audio))
}

/// Load the audio data of a WAVE file into memory.
/// Translation of `SDL_LoadWAV_IO()`.
///
/// Loading a WAVE file requires `src` to point to a valid [`IoStream`] at
/// the start of the RIFF data; it is left positioned after the WAVE data
/// (the C function's `closeio == false` behavior; drop the stream to close
/// it). Returns the format of the audio and the decoded samples.
///
/// The entire data portion of the file is then loaded into memory and
/// decoded if necessary.
///
/// Supported formats are RIFF WAVE files with the formats PCM (8, 16, 24,
/// and 32 bits), IEEE Float (32 bits), Microsoft ADPCM and IMA ADPCM (4
/// bits), and A-law and mu-law (8 bits). Other formats are currently
/// unsupported and cause an error.
///
/// Because of the underspecification of the .WAV format, there are many
/// problematic files in the wild that cause issues with strict decoders. To
/// provide compatibility with these files, this decoder is lenient in
/// regards to the truncation of the file, the fact chunk, and the size of
/// the RIFF chunk. The hints [`WAVE_RIFF_CHUNK_SIZE`](hints::WAVE_RIFF_CHUNK_SIZE),
/// [`WAVE_TRUNCATION`](hints::WAVE_TRUNCATION), and
/// [`WAVE_FACT_CHUNK`](hints::WAVE_FACT_CHUNK) can be used to tune the
/// behavior of the loading process.
///
/// Any file that is invalid (due to truncation, corruption, or wrong values
/// in the headers), too big, or unsupported causes an error. Additionally,
/// any critical I/O error from the data source will terminate the loading
/// process with an error.
///
/// It is required that the data source supports seeking.
pub fn load_wav_io(src: &mut IoStream<'_>) -> Result<(AudioSpec, Vec<u8>)> {
    let mut file = WaveFile {
        riffhint: wave_get_riff_size_hint(),
        trunchint: wave_get_truncation_hint(),
        facthint: wave_get_fact_chunk_hint(),
        ..WaveFile::default()
    };

    let result = wave_load(src, &mut file);

    // Cleanup
    let _ = src.seek(file.chunk.position, IoWhence::Set);

    result
}

/// Loads a WAV from a file path. Translation of `SDL_LoadWAV()`.
///
/// This is a convenience function that is effectively the same as opening
/// the file and calling [`load_wav_io`].
pub fn load_wav(path: impl AsRef<std::path::Path>) -> Result<(AudioSpec, Vec<u8>)> {
    let mut stream = IoStream::from_file(path, "rb")?;
    load_wav_io(&mut stream)
}
