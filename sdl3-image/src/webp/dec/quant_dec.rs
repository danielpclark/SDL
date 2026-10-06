// Rust translation of src/dec/quant_dec.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2010 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Quantizer initialization

use crate::webp::dec::vp8_dec::VP8Decoder;
use crate::webp::dec::NUM_MB_SEGMENTS;

/// Translation of `clip()`.
fn clip(v: i32, m: i32) -> usize {
    (if v < 0 {
        0
    } else if v > m {
        m
    } else {
        v
    }) as usize
}

// Paragraph 14.1
static K_DC_TABLE: [u8; 128] = [ 4, 5, 6, 7, 8, 9, 10, 10, 11, 12, 13, 14, 15, 16, 17, 17, 18, 19, 20, 20, 21, 21, 22, 22, 23, 23, 24, 25, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 91, 93, 95, 96, 98, 100, 101, 102, 104, 106, 108, 110, 112, 114, 116, 118, 122, 124, 126, 128, 130, 132, 134, 136, 138, 140, 143, 145, 148, 151, 154, 157 ];

static K_AC_TABLE: [u16; 128] = [ 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 60, 62, 64, 66, 68, 70, 72, 74, 76, 78, 80, 82, 84, 86, 88, 90, 92, 94, 96, 98, 100, 102, 104, 106, 108, 110, 112, 114, 116, 119, 122, 125, 128, 131, 134, 137, 140, 143, 146, 149, 152, 155, 158, 161, 164, 167, 170, 173, 177, 181, 185, 189, 193, 197, 201, 205, 209, 213, 217, 221, 225, 229, 234, 239, 245, 249, 254, 259, 264, 269, 274, 279, 284 ];

//------------------------------------------------------------------------------
// Paragraph 9.6

/// Translation of `VP8ParseQuant()`.
pub(crate) fn vp8_parse_quant(dec: &mut VP8Decoder<'_>) {
    let br = &mut dec.br;
    let base_q0 = br.get_value(7) as i32;
    let mut delta = || {
        if br.get() != 0 {
            br.get_signed_value(4)
        } else {
            0
        }
    };
    let dqy1_dc = delta();
    let dqy2_dc = delta();
    let dqy2_ac = delta();
    let dquv_dc = delta();
    let dquv_ac = delta();

    let hdr = &dec.segment_hdr;

    for i in 0..NUM_MB_SEGMENTS {
        let mut q;
        if hdr.use_segment {
            q = hdr.quantizer[i] as i32;
            if !hdr.absolute_delta {
                q += base_q0;
            }
        } else if i > 0 {
            dec.dqm[i] = dec.dqm[0];
            continue;
        } else {
            q = base_q0;
        }
        {
            let m = &mut dec.dqm[i];
            m.y1_mat[0] = K_DC_TABLE[clip(q + dqy1_dc, 127)] as i32;
            m.y1_mat[1] = K_AC_TABLE[clip(q, 127)] as i32;

            m.y2_mat[0] = K_DC_TABLE[clip(q + dqy2_dc, 127)] as i32 * 2;
            // For all x in [0..284], x*155/100 is bitwise equal to (x*101581) >> 16.
            // The smallest precision for that is '(x*6349) >> 12' but 16 is a good
            // word size.
            m.y2_mat[1] = (K_AC_TABLE[clip(q + dqy2_ac, 127)] as i32 * 101581) >> 16;
            if m.y2_mat[1] < 8 {
                m.y2_mat[1] = 8;
            }

            m.uv_mat[0] = K_DC_TABLE[clip(q + dquv_dc, 117)] as i32;
            m.uv_mat[1] = K_AC_TABLE[clip(q + dquv_ac, 127)] as i32;

            m.uv_quant = q + dquv_ac; // for dithering strength evaluation
        }
    }
}
