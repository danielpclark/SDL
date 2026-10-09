// Rust translation of src/filmgrain_tmpl.c and src/filmgrain.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins), the plain-C functions.
// Copyright © 2018, Niklas Haas
// Copyright © 2018, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Film grain synthesis: the grain LUT generators and the 32x32 block
//! noise application for luma and chroma.

use super::bitdepth::{bitdepth_from_max, iclip_pixel, Pixel};
use super::headers::Dav1dFilmGrainData;
use super::intops::{iclip, imin};
use super::tables::GAUSSIAN_SEQUENCE;

pub(crate) const GRAIN_WIDTH: usize = 82;
pub(crate) const GRAIN_HEIGHT: usize = 73;
pub(crate) const BLOCK_SIZE: usize = 32;

/// One plane's grain LUT (`entry [GRAIN_HEIGHT + 1][GRAIN_WIDTH]`).
pub(crate) type GrainLut<E> = [[E; GRAIN_WIDTH]; GRAIN_HEIGHT + 1];

const SUB_GRAIN_WIDTH: usize = 44;
const SUB_GRAIN_HEIGHT: usize = 38;

#[inline]
fn get_random_number(bits: i32, state: &mut u32) -> i32 {
    let r = *state as i32;
    let bit = (((r >> 0) ^ (r >> 1) ^ (r >> 3) ^ (r >> 12)) & 1) as u32;
    *state = ((r >> 1) as u32) | (bit << 15);

    ((*state >> (16 - bits)) & ((1 << bits) - 1)) as i32
}

#[inline]
fn round2(x: i32, shift: u64) -> i32 {
    (x + ((1 << shift) >> 1)) >> shift
}

/// `generate_grain_y_c`
pub(crate) fn generate_grain_y<P: Pixel>(
    buf: &mut GrainLut<P::Entry>,
    data: &Dav1dFilmGrainData,
    bitdepth_max: i32,
) {
    let bitdepth_min_8 = bitdepth_from_max(bitdepth_max) - 8;
    let mut seed = data.seed;
    let shift = (4 - bitdepth_min_8 + data.grain_scale_shift) as u64;
    let grain_ctr = 128 << bitdepth_min_8;
    let (grain_min, grain_max) = (-grain_ctr, grain_ctr - 1);

    for row in buf.iter_mut().take(GRAIN_HEIGHT) {
        for v in row.iter_mut() {
            let value = get_random_number(11, &mut seed);
            *v = P::entry_from_i32(round2(GAUSSIAN_SEQUENCE[value as usize] as i32, shift));
        }
    }

    let ar_pad = 3;
    let ar_lag = data.ar_coeff_lag as isize;

    for y in ar_pad..GRAIN_HEIGHT {
        for x in ar_pad..GRAIN_WIDTH - ar_pad {
            let mut coeff = 0;
            let mut sum = 0;
            'outer: for dy in -ar_lag..=0 {
                for dx in -ar_lag..=ar_lag {
                    if dx == 0 && dy == 0 {
                        break 'outer;
                    }
                    sum += data.ar_coeffs_y[coeff] as i32
                        * P::entry_to_i32(
                            buf[(y as isize + dy) as usize][(x as isize + dx) as usize],
                        );
                    coeff += 1;
                }
            }

            let grain = P::entry_to_i32(buf[y][x]) + round2(sum, data.ar_coeff_shift);
            buf[y][x] = P::entry_from_i32(iclip(grain, grain_min, grain_max));
        }
    }
}

/// `generate_grain_uv_c` (`dsp->generate_grain_uv[layout - 1]` are `subx`,
/// `suby` (1, 1) for 4:2:0, (1, 0) for 4:2:2 and (0, 0) for 4:4:4)
pub(crate) fn generate_grain_uv<P: Pixel>(
    buf: &mut GrainLut<P::Entry>,
    buf_y: &GrainLut<P::Entry>,
    data: &Dav1dFilmGrainData,
    uv: usize,
    subx: bool,
    suby: bool,
    bitdepth_max: i32,
) {
    let bitdepth_min_8 = bitdepth_from_max(bitdepth_max) - 8;
    let mut seed = data.seed ^ if uv != 0 { 0x49d8 } else { 0xb524 };
    let shift = (4 - bitdepth_min_8 + data.grain_scale_shift) as u64;
    let grain_ctr = 128 << bitdepth_min_8;
    let (grain_min, grain_max) = (-grain_ctr, grain_ctr - 1);

    let chroma_w = if subx { SUB_GRAIN_WIDTH } else { GRAIN_WIDTH };
    let chroma_h = if suby { SUB_GRAIN_HEIGHT } else { GRAIN_HEIGHT };

    for row in buf.iter_mut().take(chroma_h) {
        for v in row.iter_mut().take(chroma_w) {
            let value = get_random_number(11, &mut seed);
            *v = P::entry_from_i32(round2(GAUSSIAN_SEQUENCE[value as usize] as i32, shift));
        }
    }

    let ar_pad = 3;
    let ar_lag = data.ar_coeff_lag as isize;
    let (subx, suby) = (subx as usize, suby as usize);

    for y in ar_pad..chroma_h {
        for x in ar_pad..chroma_w - ar_pad {
            let coeffs = &data.ar_coeffs_uv[uv];
            let mut coeff = 0;
            let mut sum = 0;
            'outer: for dy in -ar_lag..=0 {
                for dx in -ar_lag..=ar_lag {
                    // For the final (current) pixel, we need to add in the
                    // contribution from the luma grain texture
                    if dx == 0 && dy == 0 {
                        if data.num_y_points == 0 {
                            break 'outer;
                        }
                        let mut luma = 0;
                        let luma_x = ((x - ar_pad) << subx) + ar_pad;
                        let luma_y = ((y - ar_pad) << suby) + ar_pad;
                        for i in 0..=suby {
                            for j in 0..=subx {
                                luma += P::entry_to_i32(buf_y[luma_y + i][luma_x + j]);
                            }
                        }
                        luma = round2(luma, (subx + suby) as u64);
                        sum += luma * coeffs[coeff] as i32;
                        break 'outer;
                    }

                    sum += coeffs[coeff] as i32
                        * P::entry_to_i32(
                            buf[(y as isize + dy) as usize][(x as isize + dx) as usize],
                        );
                    coeff += 1;
                }
            }

            let grain = P::entry_to_i32(buf[y][x]) + round2(sum, data.ar_coeff_shift);
            buf[y][x] = P::entry_from_i32(iclip(grain, grain_min, grain_max));
        }
    }
}

// samples from the correct block of a grain LUT, while taking into account the
// offsets provided by the offsets cache
#[inline]
fn sample_lut<P: Pixel>(
    grain_lut: &GrainLut<P::Entry>,
    offsets: &[[i32; 2]; 2],
    subx: usize,
    suby: usize,
    bx: usize,
    by: usize,
    x: usize,
    y: usize,
) -> i32 {
    let randval = offsets[bx][by];
    let offx = 3 + (2 >> subx) * (3 + (randval >> 4)) as usize;
    let offy = 3 + (2 >> suby) * (3 + (randval & 0xF)) as usize;
    P::entry_to_i32(
        grain_lut[offy + y + (BLOCK_SIZE >> suby) * by][offx + x + (BLOCK_SIZE >> subx) * bx],
    )
}

/// `fgy_32x32xn_c`: `dst_row`/`src_row` are offsets of the row in `dst` and
/// `src`.
pub(crate) fn fgy_32x32xn<P: Pixel>(
    dst: &mut [P],
    dst_row: usize,
    src: &[P],
    src_row: usize,
    stride: usize,
    data: &Dav1dFilmGrainData,
    pw: usize,
    scaling: &[u8],
    grain_lut: &GrainLut<P::Entry>,
    bh: i32,
    row_num: i32,
    bitdepth_max: i32,
) {
    let rows = 1 + (data.overlap_flag != 0 && row_num > 0) as usize;
    let bitdepth_min_8 = bitdepth_from_max(bitdepth_max) - 8;
    let grain_ctr = 128 << bitdepth_min_8;
    let (grain_min, grain_max) = (-grain_ctr, grain_ctr - 1);

    let (min_value, max_value);
    if data.clip_to_restricted_range != 0 {
        min_value = 16 << bitdepth_min_8;
        max_value = 235 << bitdepth_min_8;
    } else {
        min_value = 0;
        max_value = bitdepth_max;
    }

    // seed[0] contains the current row, seed[1] contains the previous
    let mut seed = [0u32; 2];
    for (i, s) in seed.iter_mut().enumerate().take(rows) {
        let i = i as i32;
        *s = data.seed;
        *s ^= ((((row_num - i) * 37 + 178) & 0xFF) << 8) as u32;
        *s ^= (((row_num - i) * 173 + 105) & 0xFF) as u32;
    }

    debug_assert!(stride % BLOCK_SIZE == 0);

    let mut offsets = [[0i32; 2 /* row offset */]; 2 /* col offset */];

    const W: [[i32; 2]; 2] = [[27, 17], [17, 27]];

    // process this row in BLOCK_SIZE^2 blocks
    let mut bx = 0;
    while bx < pw {
        let bw = imin(BLOCK_SIZE as i32, pw as i32 - bx as i32) as usize;

        if data.overlap_flag != 0 && bx != 0 {
            // shift previous offsets left
            for i in 0..rows {
                offsets[1][i] = offsets[0][i];
            }
        }

        // update current offsets
        for i in 0..rows {
            offsets[0][i] = get_random_number(8, &mut seed[i]);
        }

        // x/y block offsets to compensate for overlapped regions
        let ystart = if data.overlap_flag != 0 && row_num != 0 {
            imin(2, bh) as usize
        } else {
            0
        };
        let xstart = if data.overlap_flag != 0 && bx != 0 {
            imin(2, bw as i32) as usize
        } else {
            0
        };

        let mut add_noise_y = |x: usize, y: usize, grain: i32| {
            let s = src[src_row + y * stride + x + bx];
            let noise = round2(
                scaling[s.to_i32() as usize] as i32 * grain,
                data.scaling_shift as u64,
            );
            dst[dst_row + y * stride + x + bx] =
                P::from_i32(iclip(s.to_i32() + noise, min_value, max_value));
        };

        for y in ystart..bh as usize {
            // Non-overlapped image region (straightforward)
            for x in xstart..bw {
                let grain = sample_lut::<P>(grain_lut, &offsets, 0, 0, 0, 0, x, y);
                add_noise_y(x, y, grain);
            }

            // Special case for overlapped column
            for x in 0..xstart {
                let mut grain = sample_lut::<P>(grain_lut, &offsets, 0, 0, 0, 0, x, y);
                let old = sample_lut::<P>(grain_lut, &offsets, 0, 0, 1, 0, x, y);
                grain = round2(old * W[x][0] + grain * W[x][1], 5);
                grain = iclip(grain, grain_min, grain_max);
                add_noise_y(x, y, grain);
            }
        }

        for y in 0..ystart {
            // Special case for overlapped row (sans corner)
            for x in xstart..bw {
                let mut grain = sample_lut::<P>(grain_lut, &offsets, 0, 0, 0, 0, x, y);
                let old = sample_lut::<P>(grain_lut, &offsets, 0, 0, 0, 1, x, y);
                grain = round2(old * W[y][0] + grain * W[y][1], 5);
                grain = iclip(grain, grain_min, grain_max);
                add_noise_y(x, y, grain);
            }

            // Special case for doubly-overlapped corner
            for x in 0..xstart {
                // Blend the top pixel with the top left block
                let mut top = sample_lut::<P>(grain_lut, &offsets, 0, 0, 0, 1, x, y);
                let mut old = sample_lut::<P>(grain_lut, &offsets, 0, 0, 1, 1, x, y);
                top = round2(old * W[x][0] + top * W[x][1], 5);
                top = iclip(top, grain_min, grain_max);

                // Blend the current pixel with the left block
                let mut grain = sample_lut::<P>(grain_lut, &offsets, 0, 0, 0, 0, x, y);
                old = sample_lut::<P>(grain_lut, &offsets, 0, 0, 1, 0, x, y);
                grain = round2(old * W[x][0] + grain * W[x][1], 5);
                grain = iclip(grain, grain_min, grain_max);

                // Mix the row rows together and apply grain
                grain = round2(top * W[y][0] + grain * W[y][1], 5);
                grain = iclip(grain, grain_min, grain_max);
                add_noise_y(x, y, grain);
            }
        }
        bx += BLOCK_SIZE;
    }
}

/// `fguv_32x32xn_c` (`dsp->fguv_32x32xn[layout - 1]` are `sx`, `sy` as for
/// [`generate_grain_uv`]); `luma_w` is the luma width (see below).
///
/// Note (upstream): `dav1d_apply_grain_row()` extends the luma plane of
/// the input picture by one pixel on the right (`ptr[out->p.w] =
/// ptr[out->p.w - 1]`) before calling this for odd widths with horizontal
/// subsampling; here the input stays immutable and the read of that
/// column is clamped to the last one instead, which gives the same value.
pub(crate) fn fguv_32x32xn<P: Pixel>(
    dst: &mut [P],
    dst_row: usize,
    src: &[P],
    src_row: usize,
    stride: usize,
    data: &Dav1dFilmGrainData,
    pw: usize,
    scaling: &[u8],
    grain_lut: &GrainLut<P::Entry>,
    bh: i32,
    row_num: i32,
    luma: &[P],
    luma_row: usize,
    luma_stride: usize,
    luma_w: usize,
    uv: usize,
    is_id: bool,
    sx: bool,
    sy: bool,
    bitdepth_max: i32,
) {
    let rows = 1 + (data.overlap_flag != 0 && row_num > 0) as usize;
    let bitdepth_min_8 = bitdepth_from_max(bitdepth_max) - 8;
    let grain_ctr = 128 << bitdepth_min_8;
    let (grain_min, grain_max) = (-grain_ctr, grain_ctr - 1);
    let (sx, sy) = (sx as usize, sy as usize);

    let (min_value, max_value);
    if data.clip_to_restricted_range != 0 {
        min_value = 16 << bitdepth_min_8;
        max_value = (if is_id { 235 } else { 240 }) << bitdepth_min_8;
    } else {
        min_value = 0;
        max_value = bitdepth_max;
    }

    // seed[0] contains the current row, seed[1] contains the previous
    let mut seed = [0u32; 2];
    for (i, s) in seed.iter_mut().enumerate().take(rows) {
        let i = i as i32;
        *s = data.seed;
        *s ^= ((((row_num - i) * 37 + 178) & 0xFF) << 8) as u32;
        *s ^= (((row_num - i) * 173 + 105) & 0xFF) as u32;
    }

    debug_assert!(stride % BLOCK_SIZE == 0);

    let mut offsets = [[0i32; 2 /* row offset */]; 2 /* col offset */];

    const W: [[[i32; 2]; 2] /* off */; 2 /* sub */] = [[[27, 17], [17, 27]], [[23, 22], [0, 0]]];

    // process this row in BLOCK_SIZE^2 blocks (subsampled)
    let mut bx = 0;
    while bx < pw {
        let bw = imin((BLOCK_SIZE >> sx) as i32, (pw - bx) as i32) as usize;
        if data.overlap_flag != 0 && bx != 0 {
            // shift previous offsets left
            for i in 0..rows {
                offsets[1][i] = offsets[0][i];
            }
        }

        // update current offsets
        for i in 0..rows {
            offsets[0][i] = get_random_number(8, &mut seed[i]);
        }

        // x/y block offsets to compensate for overlapped regions
        let ystart = if data.overlap_flag != 0 && row_num != 0 {
            imin(2 >> sy, bh) as usize
        } else {
            0
        };
        let xstart = if data.overlap_flag != 0 && bx != 0 {
            imin(2 >> sx, bw as i32) as usize
        } else {
            0
        };

        let mut add_noise_uv = |x: usize, y: usize, grain: i32| {
            let lx = (bx + x) << sx;
            let ly = y << sy;
            let l = luma_row + ly * luma_stride;
            let mut avg = luma[l + lx].to_i32();
            if sx != 0 {
                avg = (avg + luma[l + (lx + 1).min(luma_w - 1)].to_i32() + 1) >> 1;
            }
            let si = src_row + y * stride + (bx + x);
            let s = src[si].to_i32();
            let mut val = avg;
            if data.chroma_scaling_from_luma == 0 {
                let combined = avg * data.uv_luma_mult[uv] + s * data.uv_mult[uv];
                val = iclip_pixel::<P>(
                    (combined >> 6) + (data.uv_offset[uv] * (1 << bitdepth_min_8)),
                    bitdepth_max,
                )
                .to_i32();
            }
            let noise = round2(
                scaling[val as usize] as i32 * grain,
                data.scaling_shift as u64,
            );
            dst[dst_row + y * stride + (bx + x)] =
                P::from_i32(iclip(s + noise, min_value, max_value));
        };

        for y in ystart..bh as usize {
            // Non-overlapped image region (straightforward)
            for x in xstart..bw {
                let grain = sample_lut::<P>(grain_lut, &offsets, sx, sy, 0, 0, x, y);
                add_noise_uv(x, y, grain);
            }

            // Special case for overlapped column
            for x in 0..xstart {
                let mut grain = sample_lut::<P>(grain_lut, &offsets, sx, sy, 0, 0, x, y);
                let old = sample_lut::<P>(grain_lut, &offsets, sx, sy, 1, 0, x, y);
                grain = round2(old * W[sx][x][0] + grain * W[sx][x][1], 5);
                grain = iclip(grain, grain_min, grain_max);
                add_noise_uv(x, y, grain);
            }
        }

        for y in 0..ystart {
            // Special case for overlapped row (sans corner)
            for x in xstart..bw {
                let mut grain = sample_lut::<P>(grain_lut, &offsets, sx, sy, 0, 0, x, y);
                let old = sample_lut::<P>(grain_lut, &offsets, sx, sy, 0, 1, x, y);
                grain = round2(old * W[sy][y][0] + grain * W[sy][y][1], 5);
                grain = iclip(grain, grain_min, grain_max);
                add_noise_uv(x, y, grain);
            }

            // Special case for doubly-overlapped corner
            for x in 0..xstart {
                // Blend the top pixel with the top left block
                let mut top = sample_lut::<P>(grain_lut, &offsets, sx, sy, 0, 1, x, y);
                let mut old = sample_lut::<P>(grain_lut, &offsets, sx, sy, 1, 1, x, y);
                top = round2(old * W[sx][x][0] + top * W[sx][x][1], 5);
                top = iclip(top, grain_min, grain_max);

                // Blend the current pixel with the left block
                let mut grain = sample_lut::<P>(grain_lut, &offsets, sx, sy, 0, 0, x, y);
                old = sample_lut::<P>(grain_lut, &offsets, sx, sy, 1, 0, x, y);
                grain = round2(old * W[sx][x][0] + grain * W[sx][x][1], 5);
                grain = iclip(grain, grain_min, grain_max);

                // Mix the row rows together and apply to image
                grain = round2(top * W[sy][y][0] + grain * W[sy][y][1], 5);
                grain = iclip(grain, grain_min, grain_max);
                add_noise_uv(x, y, grain);
            }
        }
        bx += BLOCK_SIZE >> sx;
    }
}
