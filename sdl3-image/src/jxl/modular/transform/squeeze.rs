// Rust translation of lib/jxl/modular/transform/squeeze.h and
// lib/jxl/modular/transform/squeeze.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Haar-like transform: halves the resolution in one direction
//! A B   -> (A+B)>>1              in one channel (average)  -> same range as
//! original channel
//!          A-B - tendency        in a new channel ('residual' needed to make
//!          the transform reversible)
//!
//! Repeated application (alternating horizontal and vertical squeezes)
//! results in downscaling. (The SIMD unsqueeze is not used on highway's
//! scalar target.)

use super::super::super::base::{div_ceil, jxl_failure, Status};
use super::super::modular_image::{Channel, Image, PixelType, PixelTypeW};
use super::transform::SqueezeParams;

pub(crate) const JXL_MAX_FIRST_PREVIEW_SIZE: usize = 8;

//         |A B|C D|E F|
//           p   a   n             p=avg(A,B), a=avg(C,D), n=avg(E,F)
//
// Goal: estimate C-D (avoiding ringing artifacts)
// (ensuring that in smooth areas, a zero residual corresponds to a smooth
// gradient)

// best estimate for C: (B + 2*a)/3
// best estimate for D: (n + 3*a)/4
// best estimate for C-D:  4*B - 3*n - a /12

// avoid ringing by 1) only doing this if B <= a <= n  or  B >= a >= n
// (otherwise, this is not a smooth area and we cannot really estimate C-D)
//                  2) making sure that B <= C <= D <= n  or B >= C >= D >= n

/// Translation of `SmoothTendency()`.
#[inline]
pub(crate) fn smooth_tendency(b: PixelTypeW, a: PixelTypeW, n: PixelTypeW) -> PixelTypeW {
    let mut diff: PixelTypeW = 0;
    if b >= a && a >= n {
        diff = (4i64
            .wrapping_mul(b)
            .wrapping_sub(3i64.wrapping_mul(n))
            .wrapping_sub(a)
            .wrapping_add(6))
            / 12;
        //      2C = a<<1 + diff - diff&1 <= 2B  so diff - diff&1 <= 2B - 2a
        //      2D = a<<1 - diff - diff&1 >= 2n  so diff + diff&1 <= 2a - 2n
        if diff.wrapping_sub(diff & 1) > 2i64.wrapping_mul(b.wrapping_sub(a)) {
            diff = 2i64.wrapping_mul(b.wrapping_sub(a)).wrapping_add(1);
        }
        if diff.wrapping_add(diff & 1) > 2i64.wrapping_mul(a.wrapping_sub(n)) {
            diff = 2i64.wrapping_mul(a.wrapping_sub(n));
        }
    } else if b <= a && a <= n {
        diff = (4i64
            .wrapping_mul(b)
            .wrapping_sub(3i64.wrapping_mul(n))
            .wrapping_sub(a)
            .wrapping_sub(6))
            / 12;
        //      2C = a<<1 + diff + diff&1 >= 2B  so diff + diff&1 >= 2B - 2a
        //      2D = a<<1 - diff + diff&1 <= 2n  so diff - diff&1 >= 2a - 2n
        if diff.wrapping_add(diff & 1) < 2i64.wrapping_mul(b.wrapping_sub(a)) {
            diff = 2i64.wrapping_mul(b.wrapping_sub(a)).wrapping_sub(1);
        }
        if diff.wrapping_sub(diff & 1) < 2i64.wrapping_mul(a.wrapping_sub(n)) {
            diff = 2i64.wrapping_mul(a.wrapping_sub(n));
        }
    }
    diff
}

/// Translation of `InvHSqueeze()`.
fn inv_h_squeeze(input: &mut Image, c: usize, rc: usize) -> Status {
    if c >= input.channel.len() || rc >= input.channel.len() {
        return jxl_failure!("channel out of range");
    }
    let (cw, ch, chs, cvs) = {
        let chin = &input.channel[c];
        (chin.w, chin.h, chin.hshift, chin.vshift)
    };
    let (rw, rh) = (input.channel[rc].w, input.channel[rc].h);
    // These must be valid since we ran MetaApply already.
    if cw != div_ceil(cw + rw, 2) || ch != rh {
        // JXL_ASSERT
        return jxl_failure!("invalid squeeze");
    }

    if rw == 0 {
        // Short-circuit: output channel has same dimensions as input.
        input.channel[c].hshift -= 1;
        return Ok(());
    }

    // Note: chin.w >= chin_residual.w and at most 1 different.
    let mut chout = Channel::new(cw + rw, ch, chs - 1, cvs)?;

    if rh == 0 {
        // Short-circuit: channel with no pixels.
        input.channel[c] = chout;
        return Ok(());
    }
    {
        let chin = &input.channel[c];
        let chin_residual = &input.channel[rc];
        let out_w = chout.w;
        for y in 0..ch {
            let p_residual = chin_residual.row(y);
            let p_avg = chin.row(y);
            let p_out = chout.row_mut(y);
            for x in 0..rw {
                let diff_minus_tendency: PixelTypeW = p_residual[x] as i64;
                let avg: PixelTypeW = p_avg[x] as i64;
                let next_avg: PixelTypeW = if x + 1 < cw { p_avg[x + 1] as i64 } else { avg };
                let left: PixelTypeW = if x != 0 {
                    p_out[(x << 1) - 1] as i64
                } else {
                    avg
                };
                let tendency = smooth_tendency(left, avg, next_avg);
                let diff = diff_minus_tendency.wrapping_add(tendency);
                let a = avg.wrapping_add(diff / 2);
                p_out[x << 1] = a as PixelType;
                let b = a.wrapping_sub(diff);
                p_out[(x << 1) + 1] = b as PixelType;
            }
            if out_w & 1 != 0 {
                p_out[out_w - 1] = p_avg[cw - 1];
            }
        }
    }
    input.channel[c] = chout;
    Ok(())
}

/// Translation of `InvVSqueeze()`.
fn inv_v_squeeze(input: &mut Image, c: usize, rc: usize) -> Status {
    if c >= input.channel.len() || rc >= input.channel.len() {
        return jxl_failure!("channel out of range");
    }
    let (cw, ch, chs, cvs) = {
        let chin = &input.channel[c];
        (chin.w, chin.h, chin.hshift, chin.vshift)
    };
    let (rw, rh) = (input.channel[rc].w, input.channel[rc].h);
    // These must be valid since we ran MetaApply already.
    if ch != div_ceil(ch + rh, 2) || cw != rw {
        // JXL_ASSERT
        return jxl_failure!("invalid squeeze");
    }

    if rh == 0 {
        // Short-circuit: output channel has same dimensions as input.
        input.channel[c].vshift -= 1;
        return Ok(());
    }

    // Note: chin.h >= chin_residual.h and at most 1 different.
    let mut chout = Channel::new(cw, ch + rh, chs, cvs - 1)?;

    if rw == 0 {
        // Short-circuit: channel with no pixels.
        input.channel[c] = chout;
        return Ok(());
    }

    {
        let chin = &input.channel[c];
        let chin_residual = &input.channel[rc];
        // (The column slices of upstream's threads, in order.)
        // We only iterate up to std::min(chin_residual.h, chin.h) which is
        // always chin_residual.h.
        for y in 0..rh {
            let p_residual = chin_residual.row(y);
            let p_avg = chin.row(y);
            let p_navg = chin.row(if y + 1 < ch { y + 1 } else { y });
            for x in 0..cw {
                let avg: PixelTypeW = p_avg[x] as i64;
                let next_avg: PixelTypeW = p_navg[x] as i64;
                let top: PixelTypeW = if y > 0 {
                    chout.row((y << 1) - 1)[x] as i64
                } else {
                    p_avg[x] as i64
                };
                let tendency = smooth_tendency(top, avg, next_avg);
                let diff_minus_tendency: PixelTypeW = p_residual[x] as i64;
                let diff = diff_minus_tendency.wrapping_add(tendency);
                let out = avg.wrapping_add(diff / 2);
                chout.row_mut(y << 1)[x] = out as PixelType;
                // If the chin_residual.h == chin.h, the output has an even number
                // of rows so the next line is fine. Otherwise, this loop won't
                // write to the last output row which is handled separately.
                chout.row_mut((y << 1) + 1)[x] = out.wrapping_sub(diff) as PixelType;
            }
        }

        if chout.h & 1 != 0 {
            let y = ch - 1;
            let p_avg = chin.row(y);
            let p_out = chout.row_mut(y << 1);
            p_out[..cw].copy_from_slice(&p_avg[..cw]);
        }
    }
    input.channel[c] = chout;
    Ok(())
}

/// Translation of `InvSqueeze()`.
pub(crate) fn inv_squeeze(input: &mut Image, parameters: &[SqueezeParams]) -> Status {
    for i in (0..parameters.len()).rev() {
        check_meta_squeeze_params(&parameters[i], input.channel.len() as i32)?;
        let horizontal = parameters[i].horizontal;
        let in_place = parameters[i].in_place;
        let beginc = parameters[i].begin_c;
        let endc = parameters[i]
            .begin_c
            .wrapping_add(parameters[i].num_c)
            .wrapping_sub(1);
        let offset: u32 = if in_place {
            endc.wrapping_add(1)
        } else {
            (input.channel.len() as u32)
                .wrapping_add(beginc)
                .wrapping_sub(endc)
                .wrapping_sub(1)
        };
        if (beginc as usize) < input.nb_meta_channels {
            // This is checked in MetaSqueeze.
            if input.nb_meta_channels <= parameters[i].num_c as usize {
                // JXL_ASSERT
                return jxl_failure!("invalid meta channels");
            }
            input.nb_meta_channels -= parameters[i].num_c as usize;
        }

        for c in beginc..=endc {
            let rc = offset.wrapping_add(c).wrapping_sub(beginc) as usize;
            // MetaApply should imply that `rc` is within range, otherwise there's a
            // programming bug.
            if rc >= input.channel.len() {
                // JXL_ASSERT
                return jxl_failure!("rc out of range");
            }
            let c = c as usize;
            if (input.channel[c].w < input.channel[rc].w)
                || (input.channel[c].h < input.channel[rc].h)
            {
                return jxl_failure!("Corrupted squeeze transform");
            }
            if horizontal {
                inv_h_squeeze(input, c, rc)?;
            } else {
                inv_v_squeeze(input, c, rc)?;
            }
        }
        let start = offset as usize;
        let end = start + (endc - beginc + 1) as usize;
        if end > input.channel.len() {
            return jxl_failure!("rc out of range");
        }
        input.channel.drain(start..end);
    }
    Ok(())
}

/// Translation of `DefaultSqueezeParameters()`.
pub(crate) fn default_squeeze_parameters(
    parameters: &mut Vec<SqueezeParams>,
    image: &Image,
) -> Status {
    let nb_channels = image.channel.len() as i32 - image.nb_meta_channels as i32;

    parameters.clear();
    if image.nb_meta_channels >= image.channel.len() {
        // (upstream reads past the channels here)
        return jxl_failure!("no channels to squeeze");
    }
    let mut w = image.channel[image.nb_meta_channels].w;
    let mut h = image.channel[image.nb_meta_channels].h;

    // do horizontal first on wide images; vertical first on tall images
    let wide = w > h;

    if nb_channels > 2
        && image.channel[image.nb_meta_channels + 1].w == w
        && image.channel[image.nb_meta_channels + 1].h == h
    {
        // assume channels 1 and 2 are chroma, and can be squeezed first for 4:2:0
        // previews
        let mut params = SqueezeParams::new();
        // horizontal chroma squeeze
        params.horizontal = true;
        params.in_place = false;
        params.begin_c = image.nb_meta_channels as u32 + 1;
        params.num_c = 2;
        parameters.push(params);
        params.horizontal = false;
        // vertical chroma squeeze
        parameters.push(params);
    }
    let mut params = SqueezeParams::new();
    params.begin_c = image.nb_meta_channels as u32;
    params.num_c = nb_channels as u32;
    params.in_place = true;

    if !wide && h > JXL_MAX_FIRST_PREVIEW_SIZE {
        params.horizontal = false;
        parameters.push(params);
        h = h.div_ceil(2);
    }
    while w > JXL_MAX_FIRST_PREVIEW_SIZE || h > JXL_MAX_FIRST_PREVIEW_SIZE {
        if w > JXL_MAX_FIRST_PREVIEW_SIZE {
            params.horizontal = true;
            parameters.push(params);
            w = w.div_ceil(2);
        }
        if h > JXL_MAX_FIRST_PREVIEW_SIZE {
            params.horizontal = false;
            parameters.push(params);
            h = h.div_ceil(2);
        }
    }
    Ok(())
}

/// Translation of `CheckMetaSqueezeParams()`.
pub(crate) fn check_meta_squeeze_params(parameter: &SqueezeParams, num_channels: i32) -> Status {
    let c1 = parameter.begin_c as i32;
    let c2 = parameter
        .begin_c
        .wrapping_add(parameter.num_c)
        .wrapping_sub(1) as i32;
    if c1 < 0 || c1 >= num_channels || c2 < 0 || c2 >= num_channels || c2 < c1 {
        return jxl_failure!("Invalid channel range");
    }
    Ok(())
}

/// Translation of `MetaSqueeze()`.
pub(crate) fn meta_squeeze(image: &mut Image, parameters: &mut Vec<SqueezeParams>) -> Status {
    if parameters.is_empty() {
        default_squeeze_parameters(parameters, image)?;
    }

    for i in 0..parameters.len() {
        check_meta_squeeze_params(&parameters[i], image.channel.len() as i32)?;
        let horizontal = parameters[i].horizontal;
        let in_place = parameters[i].in_place;
        let beginc = parameters[i].begin_c;
        let endc = parameters[i].begin_c + parameters[i].num_c - 1;

        if (beginc as usize) < image.nb_meta_channels {
            if endc as usize >= image.nb_meta_channels {
                return jxl_failure!("Invalid squeeze: mix of meta and nonmeta channels");
            }
            if !in_place {
                return jxl_failure!("Invalid squeeze: meta channels require in-place residuals");
            }
            image.nb_meta_channels += parameters[i].num_c as usize;
        }
        let offset: u32 = if in_place {
            endc + 1
        } else {
            image.channel.len() as u32
        };
        for c in beginc..=endc {
            let cu = c as usize;
            if image.channel[cu].hshift > 30 || image.channel[cu].vshift > 30 {
                return jxl_failure!("Too many squeezes: shift > 30");
            }
            let mut w = image.channel[cu].w;
            let mut h = image.channel[cu].h;
            if horizontal {
                image.channel[cu].w = w.div_ceil(2);
                image.channel[cu].hshift += 1;
                w -= w.div_ceil(2);
            } else {
                image.channel[cu].h = h.div_ceil(2);
                image.channel[cu].vshift += 1;
                h -= h.div_ceil(2);
            }
            image.channel[cu].shrink()?;
            let mut dummy = Channel::new(w, h, 0, 0)?;
            dummy.hshift = image.channel[cu].hshift;
            dummy.vshift = image.channel[cu].vshift;

            image
                .channel
                .insert((offset + (c - beginc)) as usize, dummy);
        }
    }
    Ok(())
}
