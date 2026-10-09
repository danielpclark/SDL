// Rust translation of lib/jxl/modular/transform/rct.h and
// lib/jxl/modular/transform/rct.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The inverse reversible color transforms (as highway's scalar target runs
//! the vector loop: one lane at a time, with wrapping arithmetic).

use super::super::super::base::{jxl_failure, Status};
use super::super::modular_image::{Image, PixelType};
use super::transform::{check_equal_channels, pixel_add};

/// Translation of `InvRCTRow<transform_type>()` on the pixel values.
#[inline]
fn inv_rct_pixel(transform_type: usize, in0: PixelType, in1: PixelType, in2: PixelType) -> (PixelType, PixelType, PixelType) {
    let second = transform_type >> 1;
    let third = transform_type & 1;
    if transform_type == 6 {
        let mut y = in0;
        let co = in1;
        let cg = in2;
        y = y.wrapping_sub(cg >> 1);
        let g = cg.wrapping_add(y);
        y = y.wrapping_sub(co >> 1);
        let r = y.wrapping_add(co);
        (r, g, y)
    } else {
        let first = in0;
        let mut second_v = in1;
        let mut third_v = in2;
        if third != 0 {
            third_v = pixel_add(third_v, first);
        }
        if second == 1 {
            second_v = pixel_add(second_v, first);
        } else if second == 2 {
            second_v = pixel_add(second_v, pixel_add(first, third_v) >> 1);
        }
        (first, second_v, third_v)
    }
}

/// Translation of `InvRCT()`.
pub(crate) fn inv_rct(input: &mut Image, begin_c: usize, rct_type: usize) -> Status {
    check_equal_channels(input, begin_c as u32, begin_c.wrapping_add(2) as u32)?;
    let m = begin_c;
    let w = input.channel[m].w;
    let h = input.channel[m].h;
    if rct_type == 0 {
        // noop
        return Ok(());
    }
    // Permutation: 0=RGB, 1=GBR, 2=BRG, 3=RBG, 4=GRB, 5=BGR
    let permutation = rct_type / 7;
    if permutation >= 6 {
        // JXL_CHECK(permutation < 6)
        return jxl_failure!("invalid permutation");
    }
    // 0-5 values have the low bit corresponding to Third and the high bits
    // corresponding to Second. 6 corresponds to YCoCg.
    //
    // Second: 0=nop, 1=SubtractFirst, 2=SubtractAvgFirstThird
    //
    // Third: 0=nop, 1=SubtractFirst
    let custom = rct_type % 7;
    let o0 = m + (permutation % 3);
    let o1 = m + ((permutation + 1 + permutation / 3) % 3);
    let o2 = m + ((permutation + 2 - permutation / 3) % 3);
    // Special case: permute-only. Swap channels around.
    if custom == 0 {
        let ch0 = std::mem::take(&mut input.channel[m]);
        let ch1 = std::mem::take(&mut input.channel[m + 1]);
        let ch2 = std::mem::take(&mut input.channel[m + 2]);
        input.channel[o0] = ch0;
        input.channel[o1] = ch1;
        input.channel[o2] = ch2;
        return Ok(());
    }
    let mut row0 = vec![0 as PixelType; w];
    let mut row1 = vec![0 as PixelType; w];
    let mut row2 = vec![0 as PixelType; w];
    for y in 0..h {
        row0.copy_from_slice(&input.channel[m].row(y)[..w]);
        row1.copy_from_slice(&input.channel[m + 1].row(y)[..w]);
        row2.copy_from_slice(&input.channel[m + 2].row(y)[..w]);
        for x in 0..w {
            let (a, b, c) = inv_rct_pixel(custom, row0[x], row1[x], row2[x]);
            row0[x] = a;
            row1[x] = b;
            row2[x] = c;
        }
        input.channel[o0].row_mut(y)[..w].copy_from_slice(&row0);
        input.channel[o1].row_mut(y)[..w].copy_from_slice(&row1);
        input.channel[o2].row_mut(y)[..w].copy_from_slice(&row2);
    }
    Ok(())
}
