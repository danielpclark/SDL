// Rust translation of src/enc/syntax_enc.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Header syntax writing

use crate::webp::dec::{MAX_NUM_PARTITIONS, NUM_MB_SEGMENTS};
use crate::webp::decode::{
    ALPHA_FLAG, CHUNK_HEADER_SIZE, MAX_CANVAS_SIZE, RIFF_HEADER_SIZE, TAG_SIZE, VP8X_CHUNK_SIZE,
    VP8_FRAME_HEADER_SIZE, VP8_MAX_PARTITION0_SIZE, VP8_MAX_PARTITION_SIZE, VP8_SIGNATURE,
};
use crate::webp::enc::picture_enc::{webp_encoding_set_error, webp_picture_write};
use crate::webp::enc::tree_enc::{vp8_code_intra_modes, vp8_write_probas};
use crate::webp::enc::vp8i_enc::{VP8EncFilterHeader, VP8Encoder};
use crate::webp::encode::{WebPEncodingError, WebPPicture};
use crate::webp::utils::bit_writer_utils::VP8BitWriter;
use crate::webp::utils::{put_le24, put_le32};

//------------------------------------------------------------------------------
// Helper functions

/// Translation of `IsVP8XNeeded()`.
fn is_vp8x_needed(enc: &VP8Encoder<'_>) -> bool {
    enc.has_alpha // Currently the only case when VP8X is needed.
                  // This could change in the future.
}

/// Translation of `PutPaddingByte()`.
fn put_padding_byte(pic: &WebPPicture) -> bool {
    let pad_byte = [0u8; 1];
    webp_picture_write(pic, &pad_byte)
}

//------------------------------------------------------------------------------
// Writers for header's various pieces (in order of appearance)

/// Translation of `PutRIFFHeader()`.
fn put_riff_header(enc: &VP8Encoder<'_>, riff_size: usize) -> WebPEncodingError {
    let pic = enc.pic;
    let mut riff: [u8; RIFF_HEADER_SIZE] = *b"RIFF\0\0\0\0WEBP";
    debug_assert!(riff_size == riff_size as u32 as usize);
    put_le32(&mut riff[TAG_SIZE..], riff_size as u32);
    if !webp_picture_write(pic, &riff) {
        return WebPEncodingError::BadWrite;
    }
    WebPEncodingError::Ok
}

/// Translation of `PutVP8XHeader()`.
fn put_vp8x_header(enc: &VP8Encoder<'_>) -> WebPEncodingError {
    let pic = enc.pic;
    let mut vp8x = [0u8; CHUNK_HEADER_SIZE + VP8X_CHUNK_SIZE as usize];
    vp8x[..4].copy_from_slice(b"VP8X");
    let mut flags: u32 = 0;

    debug_assert!(is_vp8x_needed(enc));
    debug_assert!(pic.width >= 1 && pic.height >= 1);
    debug_assert!(pic.width as u32 <= MAX_CANVAS_SIZE && pic.height as u32 <= MAX_CANVAS_SIZE);

    if enc.has_alpha {
        flags |= ALPHA_FLAG;
    }

    put_le32(&mut vp8x[TAG_SIZE..], VP8X_CHUNK_SIZE);
    put_le32(&mut vp8x[CHUNK_HEADER_SIZE..], flags);
    put_le24(&mut vp8x[CHUNK_HEADER_SIZE + 4..], pic.width - 1);
    put_le24(&mut vp8x[CHUNK_HEADER_SIZE + 7..], pic.height - 1);
    if !webp_picture_write(pic, &vp8x) {
        return WebPEncodingError::BadWrite;
    }
    WebPEncodingError::Ok
}

/// Translation of `PutAlphaChunk()`.
fn put_alpha_chunk(enc: &VP8Encoder<'_>) -> WebPEncodingError {
    let pic = enc.pic;
    let mut alpha_chunk_hdr = [0u8; CHUNK_HEADER_SIZE];
    alpha_chunk_hdr[..4].copy_from_slice(b"ALPH");

    debug_assert!(enc.has_alpha);

    // Alpha chunk header.
    let alpha_data_size = enc.alpha_data.len();
    put_le32(&mut alpha_chunk_hdr[TAG_SIZE..], alpha_data_size as u32);
    if !webp_picture_write(pic, &alpha_chunk_hdr) {
        return WebPEncodingError::BadWrite;
    }

    // Alpha chunk data.
    if !webp_picture_write(pic, &enc.alpha_data) {
        return WebPEncodingError::BadWrite;
    }

    // Padding.
    if (alpha_data_size & 1) != 0 && !put_padding_byte(pic) {
        return WebPEncodingError::BadWrite;
    }
    WebPEncodingError::Ok
}

/// Translation of `PutVP8Header()`.
fn put_vp8_header(pic: &WebPPicture, vp8_size: usize) -> WebPEncodingError {
    let mut vp8_chunk_hdr = [0u8; CHUNK_HEADER_SIZE];
    vp8_chunk_hdr[..4].copy_from_slice(b"VP8 ");
    debug_assert!(vp8_size == vp8_size as u32 as usize);
    put_le32(&mut vp8_chunk_hdr[TAG_SIZE..], vp8_size as u32);
    if !webp_picture_write(pic, &vp8_chunk_hdr) {
        return WebPEncodingError::BadWrite;
    }
    WebPEncodingError::Ok
}

/// Translation of `PutVP8FrameHeader()`.
fn put_vp8_frame_header(pic: &WebPPicture, profile: i32, size0: usize) -> WebPEncodingError {
    let mut vp8_frm_hdr = [0u8; VP8_FRAME_HEADER_SIZE];

    if size0 >= VP8_MAX_PARTITION0_SIZE as usize {
        // partition #0 is too big to fit
        return WebPEncodingError::Partition0Overflow;
    }

    // Paragraph 9.1.
    // keyframe (1b): 0
    let bits: u32 = ((profile as u32) << 1) // profile (3b)
        | (1 << 4) // visible (1b)
        | ((size0 as u32) << 5); // partition length (19b)
    vp8_frm_hdr[0] = (bits & 0xff) as u8;
    vp8_frm_hdr[1] = ((bits >> 8) & 0xff) as u8;
    vp8_frm_hdr[2] = ((bits >> 16) & 0xff) as u8;
    // signature
    vp8_frm_hdr[3] = ((VP8_SIGNATURE >> 16) & 0xff) as u8;
    vp8_frm_hdr[4] = ((VP8_SIGNATURE >> 8) & 0xff) as u8;
    vp8_frm_hdr[5] = (VP8_SIGNATURE & 0xff) as u8;
    // dimensions
    vp8_frm_hdr[6] = (pic.width & 0xff) as u8;
    vp8_frm_hdr[7] = (pic.width >> 8) as u8;
    vp8_frm_hdr[8] = (pic.height & 0xff) as u8;
    vp8_frm_hdr[9] = (pic.height >> 8) as u8;

    if !webp_picture_write(pic, &vp8_frm_hdr) {
        return WebPEncodingError::BadWrite;
    }
    WebPEncodingError::Ok
}

/// WebP Headers. Translation of `PutWebPHeaders()`.
fn put_webp_headers(enc: &VP8Encoder<'_>, size0: usize, vp8_size: usize, riff_size: usize) -> bool {
    let pic = enc.pic;
    let err = (|| {
        // RIFF header.
        let err = put_riff_header(enc, riff_size);
        if err != WebPEncodingError::Ok {
            return err;
        }

        // VP8X.
        if is_vp8x_needed(enc) {
            let err = put_vp8x_header(enc);
            if err != WebPEncodingError::Ok {
                return err;
            }
        }

        // Alpha.
        if enc.has_alpha {
            let err = put_alpha_chunk(enc);
            if err != WebPEncodingError::Ok {
                return err;
            }
        }

        // VP8 header.
        let err = put_vp8_header(pic, vp8_size);
        if err != WebPEncodingError::Ok {
            return err;
        }

        // VP8 frame header.
        put_vp8_frame_header(pic, enc.profile, size0)
    })();

    if err == WebPEncodingError::Ok {
        // All OK.
        return true;
    }

    // Error.
    webp_encoding_set_error(pic, err)
}

/// Segmentation header. Translation of `PutSegmentHeader()`.
fn put_segment_header(bw: &mut VP8BitWriter, enc: &VP8Encoder<'_>) {
    let hdr = &enc.segment_hdr;
    let proba = &enc.proba;
    if bw.put_bit_uniform(hdr.num_segments > 1) {
        // We always 'update' the quant and filter strength values
        let update_data = true;
        bw.put_bit_uniform(hdr.update_map);
        if bw.put_bit_uniform(update_data) {
            // we always use absolute values, not relative ones
            bw.put_bit_uniform(true); // (segment_feature_mode = 1. Paragraph 9.3.)
            for s in 0..NUM_MB_SEGMENTS {
                bw.put_signed_bits(enc.dqm[s].quant, 7);
            }
            for s in 0..NUM_MB_SEGMENTS {
                bw.put_signed_bits(enc.dqm[s].fstrength, 6);
            }
        }
        if hdr.update_map {
            for s in 0..3 {
                if bw.put_bit_uniform(proba.segments[s] != 255) {
                    bw.put_bits(proba.segments[s] as u32, 8);
                }
            }
        }
    }
}

/// Filtering parameters header. Translation of `PutFilterHeader()`.
fn put_filter_header(bw: &mut VP8BitWriter, hdr: &VP8EncFilterHeader) {
    let use_lf_delta = hdr.i4x4_lf_delta != 0;
    bw.put_bit_uniform(hdr.simple);
    bw.put_bits(hdr.level as u32, 6);
    bw.put_bits(hdr.sharpness as u32, 3);
    if bw.put_bit_uniform(use_lf_delta) {
        // '0' is the default value for i4x4_lf_delta_ at frame #0.
        let need_update = hdr.i4x4_lf_delta != 0;
        if bw.put_bit_uniform(need_update) {
            // we don't use ref_lf_delta => emit four 0 bits
            bw.put_bits(0, 4);
            // we use mode_lf_delta for i4x4
            bw.put_signed_bits(hdr.i4x4_lf_delta, 6);
            bw.put_bits(0, 3); // all others unused
        }
    }
}

/// Nominal quantization parameters. Translation of `PutQuant()`.
fn put_quant(bw: &mut VP8BitWriter, enc: &VP8Encoder<'_>) {
    bw.put_bits(enc.base_quant as u32, 7);
    bw.put_signed_bits(enc.dq_y1_dc, 4);
    bw.put_signed_bits(enc.dq_y2_dc, 4);
    bw.put_signed_bits(enc.dq_y2_ac, 4);
    bw.put_signed_bits(enc.dq_uv_dc, 4);
    bw.put_signed_bits(enc.dq_uv_ac, 4);
}

/// Partition sizes. Translation of `EmitPartitionsSize()`.
fn emit_partitions_size(enc: &VP8Encoder<'_>, pic: &WebPPicture) -> bool {
    let mut buf = [0u8; 3 * (MAX_NUM_PARTITIONS - 1)];
    let mut p = 0;
    while p < enc.num_parts as usize - 1 {
        let part_size = enc.parts[p].size();
        if part_size >= VP8_MAX_PARTITION_SIZE as usize {
            return webp_encoding_set_error(pic, WebPEncodingError::PartitionOverflow);
        }
        buf[3 * p] = (part_size & 0xff) as u8;
        buf[3 * p + 1] = ((part_size >> 8) & 0xff) as u8;
        buf[3 * p + 2] = ((part_size >> 16) & 0xff) as u8;
        p += 1;
    }
    if p != 0 && !webp_picture_write(pic, &buf[..3 * p]) {
        return webp_encoding_set_error(pic, WebPEncodingError::BadWrite);
    }
    true
}

//------------------------------------------------------------------------------

/// Translation of `GeneratePartition0()`.
fn generate_partition0(enc: &mut VP8Encoder<'_>) -> bool {
    let mb_size = enc.mb_w * enc.mb_h;

    enc.bw = VP8BitWriter::new((mb_size * 7 / 8) as usize); // ~7 bits per macroblock
    let mut bw = std::mem::take(&mut enc.bw);
    bw.put_bit_uniform(false); // colorspace
    bw.put_bit_uniform(false); // clamp type

    put_segment_header(&mut bw, enc);
    put_filter_header(&mut bw, &enc.filter_hdr);
    bw.put_bits(
        match enc.num_parts {
            8 => 3,
            4 => 2,
            2 => 1,
            _ => 0,
        },
        2,
    );
    put_quant(&mut bw, enc);
    bw.put_bit_uniform(false); // no proba update
    vp8_write_probas(&mut bw, &enc.proba);
    enc.bw = bw;
    vp8_code_intra_modes(enc);
    enc.bw.finish();

    if enc.bw.error {
        return webp_encoding_set_error(enc.pic, WebPEncodingError::OutOfMemory);
    }
    true
}

/// Release memory allocated for bit-writing in VP8EncLoop & seq.
/// Translation of `VP8EncFreeBitWriters()`.
pub(crate) fn vp8_enc_free_bit_writers(enc: &mut VP8Encoder<'_>) {
    enc.bw.wipe_out();
    for p in 0..enc.num_parts as usize {
        enc.parts[p].wipe_out();
    }
}

/// Generates the final bitstream by coding the partition0 and headers,
/// and appending an assembly of all the pre-coded token partitions.
/// Return true if everything is ok. Translation of `VP8EncWrite()`.
pub(crate) fn vp8_enc_write(enc: &mut VP8Encoder<'_>) -> bool {
    let pic = enc.pic;

    // Partition #0 with header and partition sizes
    let mut ok = generate_partition0(enc);
    if !ok {
        return false;
    }

    // Compute VP8 size
    let mut vp8_size = VP8_FRAME_HEADER_SIZE + enc.bw.size() + 3 * (enc.num_parts as usize - 1);
    for p in 0..enc.num_parts as usize {
        vp8_size += enc.parts[p].size();
    }
    let pad = vp8_size & 1;
    vp8_size += pad;

    // Compute RIFF size
    // At the minimum it is: "WEBPVP8 nnnn" + VP8 data size.
    let mut riff_size = TAG_SIZE + CHUNK_HEADER_SIZE + vp8_size;
    if is_vp8x_needed(enc) {
        // Add size for: VP8X header + data.
        riff_size += CHUNK_HEADER_SIZE + VP8X_CHUNK_SIZE as usize;
    }
    if enc.has_alpha {
        // Add size for: ALPH header + data.
        let padded_alpha_size = enc.alpha_data.len() + (enc.alpha_data.len() & 1);
        riff_size += CHUNK_HEADER_SIZE + padded_alpha_size;
    }
    // RIFF size should fit in 32-bits.
    if riff_size > 0xfffffffe {
        return webp_encoding_set_error(pic, WebPEncodingError::FileTooBig);
    }

    // Emit headers and partition #0
    {
        let size0 = enc.bw.size();
        ok = ok
            && put_webp_headers(enc, size0, vp8_size, riff_size)
            && webp_picture_write(pic, enc.bw.buf())
            && emit_partitions_size(enc, pic);
        enc.bw.wipe_out(); // will free the internal buffer.
    }

    // Token partitions
    for p in 0..enc.num_parts as usize {
        let size = enc.parts[p].size();
        if size != 0 {
            ok = ok && webp_picture_write(pic, enc.parts[p].buf());
        }
        enc.parts[p].wipe_out(); // will free the internal buffer.
    }

    // Padding byte
    if ok && pad != 0 {
        ok = put_padding_byte(pic);
    }

    if !ok {
        webp_encoding_set_error(pic, WebPEncodingError::BadWrite);
    }
    ok
}
