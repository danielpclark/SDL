// Rust translation of src/dec/vp8_dec.c, src/dec/vp8_dec.h and
// src/dec/vp8i_dec.h from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2010 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! main entry for the decoder: the VP8 (lossy) decoder object, its headers
//! and the residual decoding.
//!
//! The decoder runs single-threaded (`mt_method_` 0, as upstream without
//! the `use_threads` option): the worker is the direct call it makes then.
//! The `VP8Io` hooks are the default ones of io_dec.c (`WebPInitCustomIo()`),
//! called directly with the `WebPDecParams` that is their `opaque` handle
//! upstream; the sample pointers of `VP8Io` are offsets in the decoder's
//! cache and alpha plane. Dithering (a decoding option) is not translated.

use crate::webp::dec::alpha_dec::{webp_deallocate_alpha_memory, ALPHDecoder};
use crate::webp::dec::frame_dec::{
    vp8_enter_critical, vp8_exit_critical, vp8_init_frame, vp8_process_row,
};
use crate::webp::dec::io_dec::WebPDecParams;
use crate::webp::dec::quant_dec::vp8_parse_quant;
use crate::webp::dec::tree_dec::{vp8_parse_intra_mode_row, vp8_parse_proba, vp8_reset_proba};
use crate::webp::dec::{
    B_DC_PRED, MAX_NUM_PARTITIONS, MB_FEATURE_TREE_PROBS, NUM_BANDS, NUM_CTX, NUM_MB_SEGMENTS,
    NUM_MODE_LF_DELTAS, NUM_PROBAS, NUM_REF_LF_DELTAS, NUM_TYPES,
};
use crate::webp::decode::VP8StatusCode;
use crate::webp::dsp::dec::transform_wht;
use crate::webp::utils::bit_reader_utils::VP8BitReader;

//------------------------------------------------------------------------------
// Various defines and enums

// version numbers
pub(crate) const DEC_MAJ_VERSION: i32 = 1;
pub(crate) const DEC_MIN_VERSION: i32 = 3;
pub(crate) const DEC_REV_VERSION: i32 = 2;

// YUV-cache parameters. Cache is 32-bytes wide (= one cacheline).
// Constraints are: We need to store one 16x16 block of luma samples (y),
// and two 8x8 chroma blocks (u/v). These are better be 16-bytes aligned,
// in order to be SIMD-friendly. We also need to store the top, left and
// top-left samples (from previously decoded blocks), along with four
// extra top-right samples for luma (intra4x4 prediction only).
// One possible layout is, using 32 * (17 + 9) bytes:
//
//   .+------   <- only 1 pixel high
//   .|yyyyt.
//   .|yyyyt.
//   .|yyyyt.
//   .|yyyy..
//   .+--.+--   <- only 1 pixel high
//   .|uu.|vv
//   .|uu.|vv
//
// Every character is a 4x4 block, with legend:
//  '.' = unused
//  'y' = y-samples   'u' = u-samples     'v' = u-samples
//  '|' = left sample,   '-' = top sample,    '+' = top-left sample
//  't' = extra top-right sample for 4x4 modes
use crate::webp::dsp::BPS;
pub(crate) const YUV_SIZE: usize = BPS * 17 + BPS * 9;
pub(crate) const Y_OFF: usize = BPS + 8;
pub(crate) const U_OFF: usize = Y_OFF + BPS * 16 + BPS;
pub(crate) const V_OFF: usize = U_OFF + 16;

//------------------------------------------------------------------------------
// Input / Output (vp8_dec.h)

/// Translation of `VP8Io`: the decoding parameters and the rows handed to
/// the output (`put()`).
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct VP8Io<'d> {
    // set by VP8GetHeaders()
    /// picture dimensions, in pixels (invariable). These are the original,
    /// uncropped dimensions. The actual area passed to put() is stored in
    /// mb_w / mb_h fields.
    pub(crate) width: i32,
    pub(crate) height: i32,

    // set before calling put()
    /// position of the current rows (in pixels)
    pub(crate) mb_y: i32,
    /// number of columns in the sample
    pub(crate) mb_w: i32,
    /// number of rows in the sample
    pub(crate) mb_h: i32,
    /// rows to copy (in yuv420 format): offsets in the decoder's cache
    pub(crate) y: usize,
    pub(crate) u: usize,
    pub(crate) v: usize,
    /// row stride for luma
    pub(crate) y_stride: i32,
    /// row stride for chroma
    pub(crate) uv_stride: i32,

    /// this is a recommendation for the user-side yuv->rgb converter. This
    /// flag is set when calling setup() hook and can be overwritten by it.
    /// It then can be taken into consideration during the put() method.
    pub(crate) fancy_upsampling: bool,

    /// Input buffer.
    pub(crate) data: &'d [u8],

    /// If true, in-loop filtering will not be performed even if present in
    /// the bitstream. Switching off filtering may speed up decoding at the
    /// expense of more visible blocking. Note that output will also be
    /// non-compliant with the VP8 specifications.
    pub(crate) bypass_filtering: bool,

    // Cropping parameters.
    pub(crate) use_cropping: bool,
    pub(crate) crop_left: i32,
    pub(crate) crop_right: i32,
    pub(crate) crop_top: i32,
    pub(crate) crop_bottom: i32,

    // Scaling parameters.
    pub(crate) use_scaling: bool,
    pub(crate) scaled_width: i32,
    pub(crate) scaled_height: i32,

    /// If non NULL, pointer to the alpha data (if present) corresponding to
    /// the start of the current row (That is: it is pre-offset by mb_y and
    /// takes cropping into account): an offset in the alpha plane.
    pub(crate) a: Option<usize>,
}

//------------------------------------------------------------------------------
// Headers

/// Translation of `VP8FrameHeader`.
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8FrameHeader {
    pub(crate) key_frame: bool,
    pub(crate) profile: u8,
    pub(crate) show: bool,
    pub(crate) partition_length: u32,
}

/// Translation of `VP8PictureHeader`.
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8PictureHeader {
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) xscale: u8,
    pub(crate) yscale: u8,
    /// 0 = YCbCr
    pub(crate) colorspace: u8,
    pub(crate) clamp_type: u8,
}

/// segment features. Translation of `VP8SegmentHeader`.
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8SegmentHeader {
    pub(crate) use_segment: bool,
    /// whether to update the segment map or not
    pub(crate) update_map: bool,
    /// absolute or delta values for quantizer and filter
    pub(crate) absolute_delta: bool,
    /// quantization changes
    pub(crate) quantizer: [i8; NUM_MB_SEGMENTS],
    /// filter strength for segments
    pub(crate) filter_strength: [i8; NUM_MB_SEGMENTS],
}

/// probas associated to one of the contexts. Translation of
/// `VP8ProbaArray`.
pub(crate) type VP8ProbaArray = [u8; NUM_PROBAS];

/// all the probas associated to one band. Translation of `VP8BandProbas`.
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8BandProbas {
    pub(crate) probas: [VP8ProbaArray; NUM_CTX],
}

/// Struct collecting all frame-persistent probabilities. Translation of
/// `VP8Proba` (`bands_ptr_[t][b]`, `&bands_[t][kBands[b]]`, is the band
/// index `kBands[b]` here).
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8Proba {
    pub(crate) segments: [u8; MB_FEATURE_TREE_PROBS],
    // Type: 0:Intra16-AC  1:Intra16-DC   2:Chroma   3:Intra4
    pub(crate) bands: [[VP8BandProbas; NUM_BANDS]; NUM_TYPES],
    pub(crate) bands_ptr: [[u8; 16 + 1]; NUM_TYPES],
}

/// Filter parameters. Translation of `VP8FilterHeader`.
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8FilterHeader {
    /// 0=complex, 1=simple
    pub(crate) simple: bool,
    /// [0..63]
    pub(crate) level: i32,
    /// [0..7]
    pub(crate) sharpness: i32,
    pub(crate) use_lf_delta: bool,
    pub(crate) ref_lf_delta: [i32; NUM_REF_LF_DELTAS],
    pub(crate) mode_lf_delta: [i32; NUM_MODE_LF_DELTAS],
}

//------------------------------------------------------------------------------
// Informations about the macroblocks.

/// filter specs. Translation of `VP8FInfo`.
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8FInfo {
    /// filter limit in [3..189], or 0 if no filtering
    pub(crate) f_limit: u8,
    /// inner limit in [1..63]
    pub(crate) f_ilevel: u8,
    /// do inner filtering?
    pub(crate) f_inner: u8,
    /// high edge variance threshold in [0..2]
    pub(crate) hev_thresh: u8,
}

/// Top/Left Contexts used for syntax-parsing. Translation of `VP8MB`.
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8MB {
    /// non-zero AC/DC coeffs (4bit for luma + 4bit for chroma)
    pub(crate) nz: u8,
    /// non-zero DC coeff (1bit)
    pub(crate) nz_dc: u8,
}

/// Dequantization matrices: [DC / AC]. Translation of `quant_t`.
pub(crate) type QuantT = [i32; 2];

/// Translation of `VP8QuantMatrix`.
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8QuantMatrix {
    pub(crate) y1_mat: QuantT,
    pub(crate) y2_mat: QuantT,
    pub(crate) uv_mat: QuantT,

    /// U/V quantizer value
    pub(crate) uv_quant: i32,
    /// dithering amplitude (0 = off, max=255)
    pub(crate) dither: i32,
}

/// Data needed to reconstruct a macroblock. Translation of `VP8MBData`.
#[derive(Clone, Copy)]
pub(crate) struct VP8MBData {
    /// 384 coeffs = (16+4+4) * 4*4
    pub(crate) coeffs: [i16; 384],
    /// true if intra4x4
    pub(crate) is_i4x4: bool,
    /// one 16x16 mode (#0) or sixteen 4x4 modes
    pub(crate) imodes: [u8; 16],
    /// chroma prediction mode
    pub(crate) uvmode: u8,
    // bit-wise info about the content of each sub-4x4 blocks (in decoding order).
    // Each of the 4x4 blocks for y/u/v is associated with a 2b code according to:
    //   code=0 -> no coefficient
    //   code=1 -> only DC
    //   code=2 -> first three coefficients are non-zero
    //   code=3 -> more than three coefficients are non-zero
    // This allows to call specialized transform functions.
    pub(crate) non_zero_y: u32,
    pub(crate) non_zero_uv: u32,
    /// local dithering strength (deduced from non_zero_*)
    pub(crate) dither: u8,
    pub(crate) skip: bool,
    pub(crate) segment: u8,
}

impl Default for VP8MBData {
    fn default() -> Self {
        VP8MBData {
            coeffs: [0; 384],
            is_i4x4: false,
            imodes: [0; 16],
            uvmode: 0,
            non_zero_y: 0,
            non_zero_uv: 0,
            dither: 0,
            skip: false,
            segment: 0,
        }
    }
}

/// Persistent information needed by the parallel processing. Translation
/// of `VP8ThreadContext` (single-threaded, its `f_info_` and `mb_data_`
/// are the decoder's, and the `io_` copy isn't needed).
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8ThreadContext {
    /// cache row to process (in [0..2])
    pub(crate) id: i32,
    /// macroblock position of the row
    pub(crate) mb_y: i32,
    /// true if row-filtering is needed
    pub(crate) filter_row: bool,
}

/// `sizeof(VP8TopSamples)`.
pub(crate) const VP8_TOP_SAMPLES_SIZE: usize = 32;

/// Saved top samples, per macroblock. Fits into a cache-line.
/// Translation of `VP8TopSamples`.
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8TopSamples {
    pub(crate) y: [u8; 16],
    pub(crate) u: [u8; 8],
    pub(crate) v: [u8; 8],
}

//------------------------------------------------------------------------------
// VP8Decoder: the main opaque structure handed over to user

/// Translation of `struct VP8Decoder`. The persistent buffers of upstream's
/// single memory chunk (`mem_`) are vectors; `mb_info` has the left
/// context first (upstream's `mb_info_[-1]`), and the three cache planes
/// are in `cache`, at the offsets `cache_y`, `cache_u` and `cache_v`.
pub(crate) struct VP8Decoder<'a> {
    pub(crate) status: VP8StatusCode,
    /// true if ready to decode a picture with VP8Decode()
    pub(crate) ready: bool,
    /// set when status_ is not OK.
    pub(crate) error_msg: &'static str,

    /// Main data source
    pub(crate) br: VP8BitReader<'a>,

    // headers
    pub(crate) frm_hdr: VP8FrameHeader,
    pub(crate) pic_hdr: VP8PictureHeader,
    pub(crate) filter_hdr: VP8FilterHeader,
    pub(crate) segment_hdr: VP8SegmentHeader,

    // Worker
    /// multi-thread method: 0=off, 1=[parse+recon][filter]
    /// 2=[parse][recon+filter]
    pub(crate) mt_method: i32,
    /// current cache row
    pub(crate) cache_id: i32,
    /// number of cached rows of 16 pixels (1, 2 or 3)
    pub(crate) num_caches: i32,
    /// Thread context
    pub(crate) thread_ctx: VP8ThreadContext,

    // dimension, in macroblock units.
    pub(crate) mb_w: i32,
    pub(crate) mb_h: i32,

    // Macroblock to process/filter, depending on cropping and filter_type.
    /// top-left MB that must be in-loop filtered
    pub(crate) tl_mb_x: i32,
    pub(crate) tl_mb_y: i32,
    /// last bottom-right MB that must be decoded
    pub(crate) br_mb_x: i32,
    pub(crate) br_mb_y: i32,

    /// number of partitions minus one.
    pub(crate) num_parts_minus_one: u32,
    /// per-partition boolean decoders.
    pub(crate) parts: [VP8BitReader<'a>; MAX_NUM_PARTITIONS],

    /// Dithering strength, deduced from decoding options: whether to use
    /// dithering or not
    pub(crate) dither: bool,

    /// dequantization (one set of DC/AC dequant factor per segment)
    pub(crate) dqm: [VP8QuantMatrix; NUM_MB_SEGMENTS],

    // probabilities
    pub(crate) proba: VP8Proba,
    pub(crate) use_skip_proba: bool,
    pub(crate) skip_p: u8,

    // Boundary data cache and persistent buffers.
    /// top intra modes values: 4 * mb_w_
    pub(crate) intra_t: Vec<u8>,
    /// left intra modes values
    pub(crate) intra_l: [u8; 4],

    /// top y/u/v samples
    pub(crate) yuv_t: Vec<VP8TopSamples>,

    /// contextual macroblock info (mb_w_ + 1)
    pub(crate) mb_info: Vec<VP8MB>,
    /// filter strength info
    pub(crate) f_info: Vec<VP8FInfo>,
    /// main block for Y/U/V (size = YUV_SIZE)
    pub(crate) yuv_b: Vec<u8>,

    /// macroblock row for storing unfiltered samples
    pub(crate) cache: Vec<u8>,
    pub(crate) cache_y: usize,
    pub(crate) cache_u: usize,
    pub(crate) cache_v: usize,
    pub(crate) cache_y_stride: i32,
    pub(crate) cache_uv_stride: i32,

    // Per macroblock non-persistent infos.
    /// current position, in macroblock units
    pub(crate) mb_x: i32,
    pub(crate) mb_y: i32,
    /// parsed reconstruction data
    pub(crate) mb_data: Vec<VP8MBData>,

    // Filtering side-info
    /// 0=off, 1=simple, 2=complex
    pub(crate) filter_type: i32,
    /// precalculated per-segment/type
    pub(crate) fstrengths: [[VP8FInfo; 2]; NUM_MB_SEGMENTS],

    // Alpha
    /// alpha-plane decoder object
    pub(crate) alph_dec: Option<Box<ALPHDecoder<'a>>>,
    /// compressed alpha data (if present)
    pub(crate) alpha_data: Option<&'a [u8]>,
    /// true if alpha_data_ is decoded in alpha_plane_
    pub(crate) is_alpha_decoded: bool,
    /// output. Persistent, contains the whole data.
    pub(crate) alpha_plane: Vec<u8>,
    /// last decoded alpha row (or NULL): an offset in alpha_plane
    pub(crate) alpha_prev_line: Option<usize>,
    /// derived from decoding options (0=off, 100=full)
    pub(crate) alpha_dithering: i32,
}

//------------------------------------------------------------------------------

/// Translation of `WebPGetDecoderVersion()`.
#[allow(dead_code)]
pub(crate) fn webp_get_decoder_version() -> i32 {
    (DEC_MAJ_VERSION << 16) | (DEC_MIN_VERSION << 8) | DEC_REV_VERSION
}

//------------------------------------------------------------------------------
// VP8Decoder

/// Translation of `SetOk()`.
fn set_ok(dec: &mut VP8Decoder<'_>) {
    dec.status = VP8StatusCode::Ok;
    dec.error_msg = "OK";
}

/// Create a new decoder object. Translation of `VP8New()`.
pub(crate) fn vp8_new<'a>() -> VP8Decoder<'a> {
    let mut dec = VP8Decoder {
        status: VP8StatusCode::Ok,
        ready: false,
        error_msg: "OK",
        br: VP8BitReader::default(),
        frm_hdr: VP8FrameHeader::default(),
        pic_hdr: VP8PictureHeader::default(),
        filter_hdr: VP8FilterHeader::default(),
        segment_hdr: VP8SegmentHeader::default(),
        mt_method: 0,
        cache_id: 0,
        num_caches: 0,
        thread_ctx: VP8ThreadContext::default(),
        mb_w: 0,
        mb_h: 0,
        tl_mb_x: 0,
        tl_mb_y: 0,
        br_mb_x: 0,
        br_mb_y: 0,
        num_parts_minus_one: 0,
        parts: [VP8BitReader::default(); MAX_NUM_PARTITIONS],
        dither: false,
        dqm: [VP8QuantMatrix::default(); NUM_MB_SEGMENTS],
        proba: VP8Proba::default(),
        use_skip_proba: false,
        skip_p: 0,
        intra_t: Vec::new(),
        intra_l: [0; 4],
        yuv_t: Vec::new(),
        mb_info: Vec::new(),
        f_info: Vec::new(),
        yuv_b: Vec::new(),
        cache: Vec::new(),
        cache_y: 0,
        cache_u: 0,
        cache_v: 0,
        cache_y_stride: 0,
        cache_uv_stride: 0,
        mb_x: 0,
        mb_y: 0,
        mb_data: Vec::new(),
        filter_type: 0,
        fstrengths: [[VP8FInfo::default(); 2]; NUM_MB_SEGMENTS],
        alph_dec: None,
        alpha_data: None,
        is_alpha_decoded: false,
        alpha_plane: Vec::new(),
        alpha_prev_line: None,
        alpha_dithering: 0,
    };
    set_ok(&mut dec);
    dec.ready = false;
    dec.num_parts_minus_one = 0;
    dec
}

/// Translation of `VP8SetError()`: returns false.
pub(crate) fn vp8_set_error(
    dec: &mut VP8Decoder<'_>,
    error: VP8StatusCode,
    msg: &'static str,
) -> bool {
    // The oldest error reported takes precedence over the new one.
    if dec.status == VP8StatusCode::Ok {
        dec.status = error;
        dec.error_msg = msg;
        dec.ready = false;
    }
    false
}

//------------------------------------------------------------------------------

/// Returns true if the next 3 bytes in data contain the VP8 signature.
/// Translation of `VP8CheckSignature()`.
pub(crate) fn vp8_check_signature(data: &[u8]) -> bool {
    data.len() >= 3 && data[0] == 0x9d && data[1] == 0x01 && data[2] == 0x2a
}

/// Validates the VP8 data-header and retrieves basic header information
/// viz width and height. Returns `None` in case of formatting error.
/// Translation of `VP8GetInfo()` (`data_size` is `data.len()`).
pub(crate) fn vp8_get_info(data: &[u8], chunk_size: usize) -> Option<(i32, i32)> {
    if data.len() < crate::webp::decode::VP8_FRAME_HEADER_SIZE {
        return None; // not enough data
    }
    // check signature
    if !vp8_check_signature(&data[3..]) {
        return None; // Wrong signature.
    }
    let bits = data[0] as u32 | ((data[1] as u32) << 8) | ((data[2] as u32) << 16);
    let key_frame = bits & 1 == 0;
    let w = (((data[7] as i32) << 8) | data[6] as i32) & 0x3fff;
    let h = (((data[9] as i32) << 8) | data[8] as i32) & 0x3fff;

    if !key_frame {
        // Not a keyframe.
        return None;
    }

    if ((bits >> 1) & 7) > 3 {
        return None; // unknown profile
    }
    if (bits >> 4) & 1 == 0 {
        return None; // first frame is invisible!
    }
    if (bits >> 5) as usize >= chunk_size {
        // partition_length
        return None; // inconsistent size information.
    }
    if w == 0 || h == 0 {
        return None; // We don't support both width and height to be zero.
    }

    Some((w, h))
}

//------------------------------------------------------------------------------
// Header parsing

/// Translation of `ResetSegmentHeader()`.
fn reset_segment_header(hdr: &mut VP8SegmentHeader) {
    hdr.use_segment = false;
    hdr.update_map = false;
    hdr.absolute_delta = true;
    hdr.quantizer = [0; NUM_MB_SEGMENTS];
    hdr.filter_strength = [0; NUM_MB_SEGMENTS];
}

/// Paragraph 9.3. Translation of `ParseSegmentHeader()`.
fn parse_segment_header(
    br: &mut VP8BitReader<'_>,
    hdr: &mut VP8SegmentHeader,
    proba: &mut VP8Proba,
) -> bool {
    hdr.use_segment = br.get() != 0;
    if hdr.use_segment {
        hdr.update_map = br.get() != 0;
        if br.get() != 0 {
            // update data
            hdr.absolute_delta = br.get() != 0;
            for s in 0..NUM_MB_SEGMENTS {
                hdr.quantizer[s] = if br.get() != 0 {
                    br.get_signed_value(7) as i8
                } else {
                    0
                };
            }
            for s in 0..NUM_MB_SEGMENTS {
                hdr.filter_strength[s] = if br.get() != 0 {
                    br.get_signed_value(6) as i8
                } else {
                    0
                };
            }
        }
        if hdr.update_map {
            for s in 0..MB_FEATURE_TREE_PROBS {
                proba.segments[s] = if br.get() != 0 {
                    br.get_value(8) as u8
                } else {
                    255
                };
            }
        }
    } else {
        hdr.update_map = false;
    }
    !br.eof
}

/// Paragraph 9.5. This function returns VP8_STATUS_SUSPENDED if we don't
/// have all the necessary data in 'buf'. This case is not necessarily an
/// error (for incremental decoding). Still, no bitreader is ever
/// initialized to make it possible to read unavailable memory. If we
/// don't even have the partitions' sizes, than VP8_STATUS_NOT_ENOUGH_DATA
/// is returned, and this is an unrecoverable error. If the partitions were
/// positioned ok, VP8_STATUS_OK is returned. Translation of
/// `ParsePartitions()`.
fn parse_partitions<'a>(dec: &mut VP8Decoder<'a>, buf: &'a [u8]) -> VP8StatusCode {
    let size = buf.len();
    let mut size_left = size;

    dec.num_parts_minus_one = (1 << dec.br.get_value(2)) - 1;
    let last_part = dec.num_parts_minus_one as usize;
    if size < 3 * last_part {
        // we can't even read the sizes with sz[]! That's a failure.
        return VP8StatusCode::NotEnoughData;
    }
    let mut part_start = last_part * 3;
    size_left -= last_part * 3;
    for p in 0..last_part {
        let sz = &buf[3 * p..];
        let mut psize = sz[0] as usize | ((sz[1] as usize) << 8) | ((sz[2] as usize) << 16);
        if psize > size_left {
            psize = size_left;
        }
        dec.parts[p] = VP8BitReader::new(&buf[part_start..part_start + psize]);
        part_start += psize;
        size_left -= psize;
    }
    dec.parts[last_part] = VP8BitReader::new(&buf[part_start..part_start + size_left]);
    if part_start < size {
        VP8StatusCode::Ok
    } else {
        VP8StatusCode::Suspended // Init is ok, but there's not enough data
    }
}

/// Paragraph 9.4. Translation of `ParseFilterHeader()`.
fn parse_filter_header(dec: &mut VP8Decoder<'_>) -> bool {
    let br = &mut dec.br;
    let hdr = &mut dec.filter_hdr;
    hdr.simple = br.get() != 0;
    hdr.level = br.get_value(6) as i32;
    hdr.sharpness = br.get_value(3) as i32;
    hdr.use_lf_delta = br.get() != 0;
    if hdr.use_lf_delta {
        if br.get() != 0 {
            // update lf-delta?
            for i in 0..NUM_REF_LF_DELTAS {
                if br.get() != 0 {
                    hdr.ref_lf_delta[i] = br.get_signed_value(6);
                }
            }
            for i in 0..NUM_MODE_LF_DELTAS {
                if br.get() != 0 {
                    hdr.mode_lf_delta[i] = br.get_signed_value(6);
                }
            }
        }
    }
    dec.filter_type = if hdr.level == 0 {
        0
    } else if hdr.simple {
        1
    } else {
        2
    };
    !br.eof
}

/// Decode the VP8 frame header. Returns true if ok. Note: 'io->data' must
/// be pointing to the start of the VP8 frame header. Translation of
/// `VP8GetHeaders()`.
pub(crate) fn vp8_get_headers<'a>(dec: &mut VP8Decoder<'a>, io: &mut VP8Io<'a>) -> bool {
    set_ok(dec);
    let mut buf = io.data;
    if buf.len() < 4 {
        return vp8_set_error(dec, VP8StatusCode::NotEnoughData, "Truncated header.");
    }

    // Paragraph 9.1
    {
        let bits = buf[0] as u32 | ((buf[1] as u32) << 8) | ((buf[2] as u32) << 16);
        let frm_hdr = &mut dec.frm_hdr;
        frm_hdr.key_frame = bits & 1 == 0;
        frm_hdr.profile = ((bits >> 1) & 7) as u8;
        frm_hdr.show = (bits >> 4) & 1 != 0;
        frm_hdr.partition_length = bits >> 5;
        if frm_hdr.profile > 3 {
            return vp8_set_error(
                dec,
                VP8StatusCode::BitstreamError,
                "Incorrect keyframe parameters.",
            );
        }
        if !frm_hdr.show {
            return vp8_set_error(
                dec,
                VP8StatusCode::UnsupportedFeature,
                "Frame not displayable.",
            );
        }
        buf = &buf[3..];
    }

    if dec.frm_hdr.key_frame {
        // Paragraph 9.2
        if buf.len() < 7 {
            return vp8_set_error(
                dec,
                VP8StatusCode::NotEnoughData,
                "cannot parse picture header",
            );
        }
        if !vp8_check_signature(buf) {
            return vp8_set_error(dec, VP8StatusCode::BitstreamError, "Bad code word");
        }
        let pic_hdr = &mut dec.pic_hdr;
        pic_hdr.width = ((((buf[4] as u32) << 8) | buf[3] as u32) & 0x3fff) as u16;
        pic_hdr.xscale = buf[4] >> 6; // ratio: 1, 5/4 5/3 or 2
        pic_hdr.height = ((((buf[6] as u32) << 8) | buf[5] as u32) & 0x3fff) as u16;
        pic_hdr.yscale = buf[6] >> 6;
        buf = &buf[7..];

        dec.mb_w = (dec.pic_hdr.width as i32 + 15) >> 4;
        dec.mb_h = (dec.pic_hdr.height as i32 + 15) >> 4;

        // Setup default output area (can be later modified during io->setup())
        io.width = dec.pic_hdr.width as i32;
        io.height = dec.pic_hdr.height as i32;
        // IMPORTANT! use some sane dimensions in crop_* and scaled_* fields.
        // So they can be used interchangeably without always testing for
        // 'use_cropping'.
        io.use_cropping = false;
        io.crop_top = 0;
        io.crop_left = 0;
        io.crop_right = io.width;
        io.crop_bottom = io.height;
        io.use_scaling = false;
        io.scaled_width = io.width;
        io.scaled_height = io.height;

        io.mb_w = io.width; // for soundness
        io.mb_h = io.height; // ditto

        vp8_reset_proba(&mut dec.proba);
        reset_segment_header(&mut dec.segment_hdr);
    }

    // Check if we have all the partition #0 available, and initialize dec->br_
    // to read this partition (and this partition only).
    if dec.frm_hdr.partition_length as usize > buf.len() {
        return vp8_set_error(dec, VP8StatusCode::NotEnoughData, "bad partition length");
    }

    let partition_length = dec.frm_hdr.partition_length as usize;
    dec.br = VP8BitReader::new(&buf[..partition_length]);
    buf = &buf[partition_length..];

    if dec.frm_hdr.key_frame {
        dec.pic_hdr.colorspace = dec.br.get() as u8;
        dec.pic_hdr.clamp_type = dec.br.get() as u8;
    }
    if !parse_segment_header(&mut dec.br, &mut dec.segment_hdr, &mut dec.proba) {
        return vp8_set_error(
            dec,
            VP8StatusCode::BitstreamError,
            "cannot parse segment header",
        );
    }
    // Filter specs
    if !parse_filter_header(dec) {
        return vp8_set_error(
            dec,
            VP8StatusCode::BitstreamError,
            "cannot parse filter header",
        );
    }
    let status = parse_partitions(dec, buf);
    if status != VP8StatusCode::Ok {
        return vp8_set_error(dec, status, "cannot parse partitions");
    }

    // quantizer change
    vp8_parse_quant(dec);

    // Frame buffer marking
    if !dec.frm_hdr.key_frame {
        return vp8_set_error(dec, VP8StatusCode::UnsupportedFeature, "Not a key frame.");
    }

    dec.br.get(); // ignore the value of update_proba_

    vp8_parse_proba(dec);

    // sanitized state
    dec.ready = true;
    true
}

//------------------------------------------------------------------------------
// Residual decoding (Paragraph 13.2 / 13.3)

static K_CAT3: [u8; 4] = [173, 148, 140, 0];
static K_CAT4: [u8; 5] = [176, 155, 140, 135, 0];
static K_CAT5: [u8; 6] = [180, 157, 141, 134, 130, 0];
static K_CAT6: [u8; 12] = [254, 254, 243, 230, 196, 177, 153, 140, 133, 130, 129, 0];
static K_CAT3456: [&[u8]; 4] = [&K_CAT3, &K_CAT4, &K_CAT5, &K_CAT6];
static K_ZIGZAG: [u8; 16] = [0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15];

/// See section 13-2: https://datatracker.ietf.org/doc/html/rfc6386#section-13.2
/// Translation of `GetLargeValue()`.
fn get_large_value(br: &mut VP8BitReader<'_>, p: &[u8]) -> i32 {
    let mut v;
    if br.get_bit(p[3] as i32) == 0 {
        if br.get_bit(p[4] as i32) == 0 {
            v = 2;
        } else {
            v = 3 + br.get_bit(p[5] as i32);
        }
    } else if br.get_bit(p[6] as i32) == 0 {
        if br.get_bit(p[7] as i32) == 0 {
            v = 5 + br.get_bit(159);
        } else {
            v = 7 + 2 * br.get_bit(165);
            v += br.get_bit(145);
        }
    } else {
        let bit1 = br.get_bit(p[8] as i32);
        let bit0 = br.get_bit(p[9 + bit1 as usize] as i32);
        let cat = 2 * bit1 + bit0;
        v = 0;
        for &tab in K_CAT3456[cat as usize] {
            if tab == 0 {
                break;
            }
            v += v + br.get_bit(tab as i32);
        }
        v += 3 + (8 << cat);
    }
    v
}

/// The coefficient probabilities of a block type: `prob[n]` upstream
/// (`bands_ptr_[t][n]`).
#[derive(Clone, Copy)]
pub(crate) struct BandsPtr<'p> {
    bands: &'p [VP8BandProbas; NUM_BANDS],
    map: &'p [u8; 16 + 1],
}

impl<'p> BandsPtr<'p> {
    fn get(&self, n: usize) -> &'p VP8BandProbas {
        &self.bands[self.map[n] as usize]
    }
}

/// Returns the position of the last non-zero coeff plus one. Translation
/// of `GetCoeffsFast()` (upstream's `GetCoeffs()` on CPUs without slow
/// SSSE3; `GetCoeffsAlt()` decodes the same coefficients).
fn get_coeffs(
    br: &mut VP8BitReader<'_>,
    prob: BandsPtr<'_>,
    ctx: usize,
    dq: &QuantT,
    mut n: usize,
    out: &mut [i16],
) -> i32 {
    let mut p: &VP8ProbaArray = &prob.get(n).probas[ctx];
    while n < 16 {
        if br.get_bit(p[0] as i32) == 0 {
            return n as i32; // previous coeff was last non-zero coeff
        }
        while br.get_bit(p[1] as i32) == 0 {
            // sequence of zero coeffs
            n += 1;
            p = &prob.get(n).probas[0];
            if n == 16 {
                return 16;
            }
        }
        {
            // non zero coeff
            let p_ctx = &prob.get(n + 1).probas;
            let v;
            if br.get_bit(p[2] as i32) == 0 {
                v = 1;
                p = &p_ctx[1];
            } else {
                v = get_large_value(br, p);
                p = &p_ctx[2];
            }
            out[K_ZIGZAG[n] as usize] = (br.get_signed(v) * dq[(n > 0) as usize]) as i16;
        }
        n += 1;
    }
    16
}

/// Translation of `NzCodeBits()`.
fn nz_code_bits(mut nz_coeffs: u32, nz: i32, dc_nz: bool) -> u32 {
    nz_coeffs <<= 2;
    nz_coeffs |= if nz > 3 {
        3
    } else if nz > 1 {
        2
    } else {
        dc_nz as u32
    };
    nz_coeffs
}

/// Translation of `ParseResiduals()`: `mb` is the index of the current
/// macroblock's context in `mb_info` (the left context is entry 0).
fn parse_residuals(dec: &mut VP8Decoder<'_>, mb: usize, token_br: &mut VP8BitReader<'_>) -> bool {
    let mb_x = dec.mb_x as usize;
    let bands_ptr = &dec.proba.bands_ptr;
    let bands_of = |t: usize| BandsPtr {
        bands: &dec.proba.bands[t],
        map: &bands_ptr[t],
    };
    let block = &mut dec.mb_data[mb_x];
    let q = &dec.dqm[block.segment as usize];
    let dst = &mut block.coeffs;
    let mut non_zero_y: u32 = 0;
    let mut non_zero_uv: u32 = 0;
    let first;
    let ac_proba;

    // (the left context is dec->mb_info_ - 1, entry 0 here)
    let (left_part, rest) = dec.mb_info.split_at_mut(1);
    let left_mb = &mut left_part[0];
    let mb = &mut rest[mb - 1];

    dst.fill(0);
    if !block.is_i4x4 {
        // parse DC
        let mut dc = [0i16; 16];
        let ctx = (mb.nz_dc + left_mb.nz_dc) as usize;
        let nz = get_coeffs(token_br, bands_of(1), ctx, &q.y2_mat, 0, &mut dc);
        mb.nz_dc = (nz > 0) as u8;
        left_mb.nz_dc = mb.nz_dc;
        if nz > 1 {
            // more than just the DC -> perform the full transform
            transform_wht(&dc, dst);
        } else {
            // only DC is non-zero -> inlined simplified transform
            let dc0 = ((dc[0] as i32 + 3) >> 3) as i16;
            for i in (0..16 * 16).step_by(16) {
                dst[i] = dc0;
            }
        }
        first = 1;
        ac_proba = bands_of(0);
    } else {
        first = 0;
        ac_proba = bands_of(3);
    }

    let mut tnz: u8 = mb.nz & 0x0f;
    let mut lnz: u8 = left_mb.nz & 0x0f;
    let mut d = 0usize;
    for _y in 0..4 {
        let mut l = lnz & 1;
        let mut nz_coeffs: u32 = 0;
        for _x in 0..4 {
            let ctx = (l + (tnz & 1)) as usize;
            let nz = get_coeffs(token_br, ac_proba, ctx, &q.y1_mat, first, &mut dst[d..]);
            l = (nz > first as i32) as u8;
            tnz = (tnz >> 1) | (l << 7);
            nz_coeffs = nz_code_bits(nz_coeffs, nz, dst[d] != 0);
            d += 16;
        }
        tnz >>= 4;
        lnz = (lnz >> 1) | (l << 7);
        non_zero_y = (non_zero_y << 8) | nz_coeffs;
    }
    let mut out_t_nz: u32 = tnz as u32;
    let mut out_l_nz: u32 = (lnz >> 4) as u32;

    for ch in (0..4).step_by(2) {
        let mut nz_coeffs: u32 = 0;
        tnz = mb.nz >> (4 + ch);
        lnz = left_mb.nz >> (4 + ch);
        for _y in 0..2 {
            let mut l = lnz & 1;
            for _x in 0..2 {
                let ctx = (l + (tnz & 1)) as usize;
                let nz = get_coeffs(token_br, bands_of(2), ctx, &q.uv_mat, 0, &mut dst[d..]);
                l = (nz > 0) as u8;
                tnz = (tnz >> 1) | (l << 3);
                nz_coeffs = nz_code_bits(nz_coeffs, nz, dst[d] != 0);
                d += 16;
            }
            tnz >>= 2;
            lnz = (lnz >> 1) | (l << 5);
        }
        // Note: we don't really need the per-4x4 details for U/V blocks.
        non_zero_uv |= nz_coeffs << (4 * ch);
        out_t_nz |= ((tnz as u32) << 4) << ch;
        out_l_nz |= ((lnz & 0xf0) as u32) << ch;
    }
    mb.nz = out_t_nz as u8;
    left_mb.nz = out_l_nz as u8;

    block.non_zero_y = non_zero_y;
    block.non_zero_uv = non_zero_uv;

    // We look at the mode-code of each block and check if some blocks have less
    // than three non-zero coeffs (code < 2). This is to avoid dithering flat and
    // empty blocks.
    block.dither = if non_zero_uv & 0xaaaa != 0 {
        0
    } else {
        q.dither as u8
    };

    (non_zero_y | non_zero_uv) == 0 // will be used for further optimization
}

//------------------------------------------------------------------------------
// Main loop

/// Decode one macroblock. Returns false if there is not enough data.
/// Translation of `VP8DecodeMB()`.
pub(crate) fn vp8_decode_mb(dec: &mut VP8Decoder<'_>, token_br: &mut VP8BitReader<'_>) -> bool {
    let mb_x = dec.mb_x as usize;
    let mb = mb_x + 1; // (dec->mb_info_ + dec->mb_x_)
    let mut skip = if dec.use_skip_proba {
        dec.mb_data[mb_x].skip
    } else {
        false
    };

    if !skip {
        skip = parse_residuals(dec, mb, token_br);
    } else {
        dec.mb_info[0].nz = 0;
        dec.mb_info[mb].nz = 0;
        if !dec.mb_data[mb_x].is_i4x4 {
            dec.mb_info[0].nz_dc = 0;
            dec.mb_info[mb].nz_dc = 0;
        }
        let block = &mut dec.mb_data[mb_x];
        block.non_zero_y = 0;
        block.non_zero_uv = 0;
        block.dither = 0;
    }

    if dec.filter_type > 0 {
        // store filter info
        let block = &dec.mb_data[mb_x];
        let mut finfo = dec.fstrengths[block.segment as usize][block.is_i4x4 as usize];
        finfo.f_inner |= !skip as u8;
        dec.f_info[mb_x] = finfo;
    }

    !token_br.eof
}

/// To be called at the start of a new scanline, to initialize predictors.
/// Translation of `VP8InitScanline()`.
pub(crate) fn vp8_init_scanline(dec: &mut VP8Decoder<'_>) {
    let left = &mut dec.mb_info[0];
    left.nz = 0;
    left.nz_dc = 0;
    dec.intra_l = [B_DC_PRED as u8; 4];
    dec.mb_x = 0;
}

/// Translation of `ParseFrame()`.
fn parse_frame(
    dec: &mut VP8Decoder<'_>,
    io: &mut VP8Io<'_>,
    params: &mut WebPDecParams<'_>,
) -> bool {
    dec.mb_y = 0;
    while dec.mb_y < dec.br_mb_y {
        // Parse bitstream for this row.
        let part = (dec.mb_y as u32 & dec.num_parts_minus_one) as usize;
        if !vp8_parse_intra_mode_row(dec) {
            return vp8_set_error(
                dec,
                VP8StatusCode::NotEnoughData,
                "Premature end-of-partition0 encountered.",
            );
        }
        let mut token_br = dec.parts[part];
        while dec.mb_x < dec.mb_w {
            if !vp8_decode_mb(dec, &mut token_br) {
                dec.parts[part] = token_br;
                return vp8_set_error(
                    dec,
                    VP8StatusCode::NotEnoughData,
                    "Premature end-of-file encountered.",
                );
            }
            dec.mb_x += 1;
        }
        dec.parts[part] = token_br;
        vp8_init_scanline(dec); // Prepare for next scanline

        // Reconstruct, filter and emit the row.
        if !vp8_process_row(dec, io, params) {
            return vp8_set_error(dec, VP8StatusCode::UserAbort, "Output aborted.");
        }
        dec.mb_y += 1;
    }

    true
}

/// Decode a picture. Will call VP8GetHeaders() if it wasn't done already.
/// Returns false in case of error. Translation of `VP8Decode()` (`params`
/// is the I/O hooks' `opaque` handle).
pub(crate) fn vp8_decode<'a>(
    dec: &mut VP8Decoder<'a>,
    io: &mut VP8Io<'a>,
    params: &mut WebPDecParams<'_>,
) -> bool {
    if !dec.ready && !vp8_get_headers(dec, io) {
        return false;
    }
    debug_assert!(dec.ready);

    // Finish setting up the decoding parameter. Will call io->setup().
    let mut ok = vp8_enter_critical(dec, io, params) == VP8StatusCode::Ok;
    if ok {
        // good to go.
        // Will allocate memory and prepare everything.
        if ok {
            ok = vp8_init_frame(dec, io);
        }

        // Main decoding loop
        if ok {
            ok = parse_frame(dec, io, params);
        }

        // Exit.
        ok &= vp8_exit_critical(dec, io, params);
    }

    if !ok {
        vp8_clear(dec);
        return false;
    }

    dec.ready = false;
    ok
}

/// Resets the decoder in its initial state, reclaiming memory. Not a
/// mandatory call between calls to VP8Decode(). Translation of
/// `VP8Clear()`.
pub(crate) fn vp8_clear(dec: &mut VP8Decoder<'_>) {
    webp_deallocate_alpha_memory(dec);
    dec.intra_t = Vec::new();
    dec.yuv_t = Vec::new();
    dec.mb_info = Vec::new();
    dec.f_info = Vec::new();
    dec.yuv_b = Vec::new();
    dec.cache = Vec::new();
    dec.mb_data = Vec::new();
    dec.br = VP8BitReader::default();
    dec.ready = false;
}
