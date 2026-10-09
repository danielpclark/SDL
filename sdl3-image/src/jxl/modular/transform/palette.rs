// Rust translation of lib/jxl/modular/transform/palette.h from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The palette transform's inverse (with the delta palette and the implicit
//! color cubes).

use super::super::super::base::{clamp1, jxl_failure, Status};
use super::super::super::image::copy_image;
use super::super::encoding::context_predict::{predict_no_tree_no_wp, predict_no_tree_wp, weighted};
use super::super::modular_image::{Channel, Image, PixelType, PixelTypeW};
use super::super::options::Predictor;
use super::transform::check_equal_channels;

pub(crate) mod palette_internal {
    use super::PixelType;

    #[allow(dead_code)]
    pub(crate) const K_MAX_PALETTE_LOOKUP_TABLE_SIZE: i32 = 1 << 16;

    pub(crate) const K_RGB_CHANNELS: usize = 3;

    // 5x5x5 color cube for the larger cube.
    pub(crate) const K_LARGE_CUBE: i32 = 5;

    // Smaller interleaved color cube to fill the holes of the larger cube.
    pub(crate) const K_SMALL_CUBE: i32 = 4;
    pub(crate) const K_SMALL_CUBE_BITS: i32 = 2;
    // kSmallCube ** 3
    pub(crate) const K_LARGE_CUBE_OFFSET: i32 = K_SMALL_CUBE * K_SMALL_CUBE * K_SMALL_CUBE;

    /// Translation of `Scale()`.
    #[inline]
    pub(crate) fn scale(value: u64, bit_depth: u64, denom: u64) -> PixelType {
        // return (value * ((static_cast<pixel_type_w>(1) << bit_depth) - 1)) / denom;
        // We only call this function with kSmallCube or kLargeCube - 1 as denom,
        // allowing us to avoid a division here.
        debug_assert!(denom == 4);
        (value.wrapping_mul((1u64 << bit_depth).wrapping_sub(1)) >> 2) as PixelType
    }

    static K_DELTA_PALETTE: [[PixelType; 3]; 72] = [
        [0, 0, 0],
        [4, 4, 4],
        [11, 0, 0],
        [0, 0, -13],
        [0, -12, 0],
        [-10, -10, -10],
        [-18, -18, -18],
        [-27, -27, -27],
        [-18, -18, 0],
        [0, 0, -32],
        [-32, 0, 0],
        [-37, -37, -37],
        [0, -32, -32],
        [24, 24, 45],
        [50, 50, 50],
        [-45, -24, -24],
        [-24, -45, -45],
        [0, -24, -24],
        [-34, -34, 0],
        [-24, 0, -24],
        [-45, -45, -24],
        [64, 64, 64],
        [-32, 0, -32],
        [0, -32, 0],
        [-32, 0, 32],
        [-24, -45, -24],
        [45, 24, 45],
        [24, -24, -45],
        [-45, -24, 24],
        [80, 80, 80],
        [64, 0, 0],
        [0, 0, -64],
        [0, -64, -64],
        [-24, -24, 45],
        [96, 96, 96],
        [64, 64, 0],
        [45, -24, -24],
        [34, -34, 0],
        [112, 112, 112],
        [24, -45, -45],
        [45, 45, -24],
        [0, -32, 32],
        [24, -24, 45],
        [0, 96, 96],
        [45, -24, 24],
        [24, -45, -24],
        [-24, -45, 24],
        [0, -64, 0],
        [96, 0, 0],
        [128, 128, 128],
        [64, 0, 64],
        [144, 144, 144],
        [96, 96, 0],
        [-36, -36, 36],
        [45, -24, -45],
        [45, -45, -24],
        [0, 0, -96],
        [0, 128, 128],
        [0, 96, 0],
        [45, 24, -45],
        [-128, 0, 0],
        [24, -45, 24],
        [-45, 24, -45],
        [64, 0, -64],
        [64, -64, -64],
        [96, 0, 96],
        [45, -45, 24],
        [24, 45, -45],
        [64, 64, -64],
        [128, 128, 0],
        [0, 0, -128],
        [-24, 45, -45],
    ];

    /// The purpose of this function is solely to extend the interpretation
    /// of palette indices to implicit values. If index < nb_deltas,
    /// indicating that the result is a delta palette entry, it is the
    /// responsibility of the caller to treat it as such. Translation of
    /// `GetPaletteValue()` (`palette` is the palette channel's storage).
    #[inline]
    pub(crate) fn get_palette_value(
        palette: &[PixelType],
        mut index: i32,
        c: usize,
        palette_size: i32,
        onerow: i32,
        bit_depth: i32,
    ) -> PixelType {
        if index < 0 {
            if c >= K_RGB_CHANNELS {
                return 0;
            }
            // Do not open the brackets, otherwise INT32_MIN negation could overflow.
            index = -(index + 1);
            index %= 1 + 2 * (K_DELTA_PALETTE.len() as i32 - 1);
            const K_MULTIPLIER: [i32; 2] = [-1, 1];
            let mut result = K_DELTA_PALETTE[((index + 1) >> 1) as usize][c] * K_MULTIPLIER[(index & 1) as usize];
            if bit_depth > 8 {
                result = result.wrapping_mul(1i32.wrapping_shl((bit_depth - 8) as u32));
            }
            result
        } else if palette_size <= index && index < palette_size.wrapping_add(K_LARGE_CUBE_OFFSET) {
            if c >= K_RGB_CHANNELS {
                return 0;
            }
            index -= palette_size;
            index >>= c as i32 * K_SMALL_CUBE_BITS;
            scale((index % K_SMALL_CUBE) as u64, bit_depth as u64, K_SMALL_CUBE as u64)
                .wrapping_add(1i32.wrapping_shl(0.max(bit_depth - 3) as u32))
        } else if palette_size.wrapping_add(K_LARGE_CUBE_OFFSET) <= index {
            if c >= K_RGB_CHANNELS {
                return 0;
            }
            index -= palette_size + K_LARGE_CUBE_OFFSET;
            // TODO(eustas): should we take care of ambiguity created by
            //               index >= kLargeCube ** 3 ?
            match c {
                0 => {}
                1 => index /= K_LARGE_CUBE,
                2 => index /= K_LARGE_CUBE * K_LARGE_CUBE,
                _ => {}
            }
            scale((index % K_LARGE_CUBE) as u64, bit_depth as u64, (K_LARGE_CUBE - 1) as u64)
        } else {
            palette[c * onerow as usize + index as usize]
        }
    }
}

/// Translation of `InvPalette()`.
pub(crate) fn inv_palette(
    input: &mut Image,
    begin_c: u32,
    nb_colors: u32,
    nb_deltas: u32,
    predictor: Predictor,
    wp_header: &weighted::Header,
) -> Status {
    let _ = nb_colors;
    if input.nb_meta_channels < 1 {
        return jxl_failure!("Error: Palette transform without palette.");
    }
    let nb = input.channel[0].h as i32;
    let c0 = begin_c.wrapping_add(1) as usize;
    if c0 >= input.channel.len() {
        return jxl_failure!("Channel is out of range.");
    }
    let w = input.channel[c0].w;
    let h = input.channel[c0].h;
    if nb < 1 {
        return jxl_failure!("Corrupted transforms");
    }
    for _ in 1..nb {
        let ch = Channel::new(w, h, input.channel[c0].hshift, input.channel[c0].vshift)?;
        input.channel.insert(c0 + 1, ch);
    }
    let bit_depth = input.bitdepth.min(24);

    if w == 0 {
        // Nothing to do.
        // Avoid touching "empty" channels with non-zero height.
    } else if nb_deltas == 0 && predictor == Predictor::Zero {
        let (head, tail) = input.channel.split_at_mut(1);
        let palette = &head[0];
        let p_palette = palette.plane.data();
        let onerow = palette.plane.pixels_per_row() as i32;
        let palette_w = palette.w as i32;
        if nb == 1 {
            let ch = &mut tail[c0 - 1];
            for y in 0..h {
                let p = ch.row_mut(y);
                for x in 0..w {
                    let index = clamp1::<i32>(p[x], 0, palette_w - 1);
                    p[x] = palette_internal::get_palette_value(p_palette, index, /*c=*/ 0, palette_w, onerow, bit_depth);
                }
            }
        } else {
            let mut p_index = vec![0 as PixelType; w];
            for y in 0..h {
                p_index.copy_from_slice(&tail[c0 - 1].row(y)[..w]);
                for c in 0..nb as usize {
                    let p_out = tail[c0 - 1 + c].row_mut(y);
                    for x in 0..w {
                        let index = p_index[x];
                        p_out[x] = palette_internal::get_palette_value(p_palette, index, c, palette_w, onerow, bit_depth);
                    }
                }
            }
        }
    } else {
        // Parallelized per channel.
        let indices = copy_image(&input.channel[c0].plane)?;
        let onerow_image = input.channel[c0].plane.pixels_per_row();
        let (head, tail) = input.channel.split_at_mut(1);
        let palette = &head[0];
        let p_palette = palette.plane.data();
        let onerow = palette.plane.pixels_per_row() as i32;
        let palette_w = palette.w as i32;
        for c in 0..nb as usize {
            let channel = &mut tail[c0 - 1 + c];
            let cw = channel.w;
            let ch = channel.h;
            let mut wp_state = if predictor == Predictor::Weighted {
                Some(weighted::State::new(wp_header, cw, ch))
            } else {
                None
            };
            let data = channel.plane.data_mut();
            for y in 0..ch {
                let idx = indices.row(y);
                for x in 0..cw {
                    let index = idx[x];
                    let palette_entry = palette_internal::get_palette_value(p_palette, index, c, palette_w, onerow, bit_depth);
                    let pos = y * onerow_image + x;
                    let val: PixelTypeW = if index < nb_deltas as i32 {
                        let pred = match wp_state.as_mut() {
                            Some(wp_state) => predict_no_tree_wp(cw, data, pos, onerow_image, x, y, predictor, wp_state),
                            None => predict_no_tree_no_wp(cw, data, pos, onerow_image, x, y, predictor),
                        };
                        pred.guess.wrapping_add(palette_entry as i64)
                    } else {
                        palette_entry as i64
                    };
                    data[pos] = val as PixelType;
                    if let Some(wp_state) = wp_state.as_mut() {
                        wp_state.update_errors(data[pos] as i64, x, y, cw);
                    }
                }
            }
        }
    }
    if c0 >= input.nb_meta_channels {
        // Palette was done on normal channels
        input.nb_meta_channels -= 1;
    } else {
        // Palette was done on metachannels
        if (input.nb_meta_channels as i64) < 2 - nb as i64 {
            // JXL_ASSERT
            return jxl_failure!("invalid meta channels");
        }
        input.nb_meta_channels = (input.nb_meta_channels as i64 - (2 - nb as i64)) as usize;
        if (begin_c as i64 + nb as i64 - 1) >= input.nb_meta_channels as i64 {
            // JXL_ASSERT
            return jxl_failure!("invalid meta channels");
        }
    }
    input.channel.remove(0);
    Ok(())
}

/// Translation of `MetaPalette()`.
pub(crate) fn meta_palette(
    input: &mut Image,
    begin_c: u32,
    end_c: u32,
    nb_colors: u32,
    nb_deltas: u32,
    _lossy: bool,
) -> Status {
    check_equal_channels(input, begin_c, end_c)?;

    let nb = (end_c - begin_c + 1) as usize;
    if (begin_c as usize) >= input.nb_meta_channels {
        // Palette was done on normal channels
        input.nb_meta_channels += 1;
    } else {
        // Palette was done on metachannels
        if end_c as usize >= input.nb_meta_channels {
            // JXL_ASSERT(end_c < input.nb_meta_channels);
            return jxl_failure!("invalid meta channels");
        }
        // we remove nb-1 metachannels and add one
        input.nb_meta_channels = (input.nb_meta_channels + 2).wrapping_sub(nb);
    }
    input
        .channel
        .drain(begin_c as usize + 1..end_c as usize + 1);
    let mut pch = Channel::new(nb_colors.wrapping_add(nb_deltas) as usize, nb, 0, 0)?;
    pch.hshift = -1;
    input.channel.insert(0, pch);
    Ok(())
}
