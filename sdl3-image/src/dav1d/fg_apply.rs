// Rust translation of src/fg_apply_tmpl.c and src/fg_apply.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins).
// Copyright © 2018, Niklas Haas
// Copyright © 2018, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Applies film grain to a whole picture: `out` is a copy of `in`'s
//! properties with its own pixels (both have the same strides).

use super::bitdepth::Pixel;
use super::filmgrain::{
    fguv_32x32xn, fgy_32x32xn, generate_grain_uv, generate_grain_y, GrainLut, BLOCK_SIZE,
    GRAIN_HEIGHT, GRAIN_WIDTH,
};
use super::headers::{
    Dav1dFilmGrainData, DAV1D_PIXEL_LAYOUT_I400, DAV1D_PIXEL_LAYOUT_I420, DAV1D_PIXEL_LAYOUT_I444,
};
use super::intops::imin;
use super::picture::{Dav1dPictureParameters, PictureData};

fn generate_scaling<P: Pixel>(bitdepth: i32, points: &[[u8; 2]], num: usize, scaling: &mut [u8]) {
    let (shift_x, scaling_size) = if P::BPC8 {
        (0, P::SCALING_SIZE)
    } else {
        debug_assert!(bitdepth > 8);
        (bitdepth - 8, 1usize << bitdepth)
    };

    if num == 0 {
        scaling[..scaling_size].fill(0);
        return;
    }

    // Fill up the preceding entries with the initial value
    scaling[..(points[0][0] as usize) << shift_x].fill(points[0][1]);

    // Linearly interpolate the values in the middle
    for i in 0..num - 1 {
        let bx = points[i][0] as i32;
        let by = points[i][1] as i32;
        let ex = points[i + 1][0] as i32;
        let ey = points[i + 1][1] as i32;
        let dx = ex - bx;
        let dy = ey - by;
        debug_assert!(dx > 0);
        let delta = dy * ((0x10000 + (dx >> 1)) / dx);
        let mut d = 0x8000;
        for x in 0..dx {
            scaling[((bx + x) << shift_x) as usize] = (by + (d >> 16)) as u8;
            d += delta;
        }
    }

    // Fill up the remaining entries with the final value
    let n = (points[num - 1][0] as usize) << shift_x;
    scaling[n..scaling_size].fill(points[num - 1][1]);

    if !P::BPC8 {
        let pad = 1usize << shift_x;
        let rnd = (pad >> 1) as i32;
        for i in 0..num - 1 {
            let bx = (points[i][0] as usize) << shift_x;
            let ex = (points[i + 1][0] as usize) << shift_x;
            let dx = ex - bx;
            let mut x = 0;
            while x < dx {
                let range = scaling[bx + x + pad] as i32 - scaling[bx + x] as i32;
                let mut r = rnd;
                for n in 1..pad {
                    r += range;
                    scaling[bx + x + n] = (scaling[bx + x] as i32 + (r >> shift_x)) as u8;
                }
                x += pad;
            }
        }
    }
}

/// The grain and scaling LUTs `dav1d_prep_grain()` fills.
struct GrainTables<P: Pixel> {
    scaling: [Vec<u8>; 3],
    grain_lut: Box<[GrainLut<P::Entry>; 3]>,
}

/// `dav1d_prep_grain()`
fn prep_grain<P: Pixel>(
    out: &mut PictureData,
    in_: &PictureData,
    p: &Dav1dPictureParameters,
    data: &Dav1dFilmGrainData,
) -> GrainTables<P> {
    let bitdepth_max = (1 << p.bpc) - 1;
    let mut t = GrainTables::<P> {
        scaling: [
            vec![0; P::SCALING_SIZE],
            vec![0; P::SCALING_SIZE],
            vec![0; P::SCALING_SIZE],
        ],
        grain_lut: Box::new([[[P::Entry::default(); GRAIN_WIDTH]; GRAIN_HEIGHT + 1]; 3]),
    };
    let ss_x = p.layout != DAV1D_PIXEL_LAYOUT_I444;
    let ss_y = p.layout == DAV1D_PIXEL_LAYOUT_I420;

    // Generate grain LUTs as needed
    let [lut_y, lut_u, lut_v] = &mut *t.grain_lut;
    generate_grain_y::<P>(lut_y, data, bitdepth_max); // always needed
    if data.num_uv_points[0] != 0 || data.chroma_scaling_from_luma != 0 {
        generate_grain_uv::<P>(lut_u, lut_y, data, 0, ss_x, ss_y, bitdepth_max);
    }
    if data.num_uv_points[1] != 0 || data.chroma_scaling_from_luma != 0 {
        generate_grain_uv::<P>(lut_v, lut_y, data, 1, ss_x, ss_y, bitdepth_max);
    }

    // Generate scaling LUTs as needed
    if data.num_y_points != 0 || data.chroma_scaling_from_luma != 0 {
        generate_scaling::<P>(
            p.bpc,
            &data.y_points,
            data.num_y_points as usize,
            &mut t.scaling[0],
        );
    }
    if data.num_uv_points[0] != 0 {
        generate_scaling::<P>(
            p.bpc,
            &data.uv_points[0],
            data.num_uv_points[0] as usize,
            &mut t.scaling[1],
        );
    }
    if data.num_uv_points[1] != 0 {
        generate_scaling::<P>(
            p.bpc,
            &data.uv_points[1],
            data.num_uv_points[1] as usize,
            &mut t.scaling[2],
        );
    }

    // Copy over the non-modified planes
    debug_assert!(out.stride == in_.stride);
    let src = P::planes(in_);
    let dst = P::planes_mut(out);
    if data.num_y_points == 0 {
        let sz = p.h as usize * in_.stride[0];
        dst[0][..sz].copy_from_slice(&src[0][..sz]);
    }

    if p.layout != DAV1D_PIXEL_LAYOUT_I400 && data.chroma_scaling_from_luma == 0 {
        let ss_ver = ss_y as i32;
        let sz = ((p.h + ss_ver) >> ss_ver) as usize * in_.stride[1];
        if data.num_uv_points[0] == 0 {
            dst[1][..sz].copy_from_slice(&src[1][..sz]);
        }
        if data.num_uv_points[1] == 0 {
            dst[2][..sz].copy_from_slice(&src[2][..sz]);
        }
    }
    t
}

/// `dav1d_apply_grain_row()`
fn apply_grain_row<P: Pixel>(
    out: &mut PictureData,
    in_: &PictureData,
    p: &Dav1dPictureParameters,
    data: &Dav1dFilmGrainData,
    is_id: bool,
    t: &GrainTables<P>,
    row: i32,
) {
    // Synthesize grain for the affected planes
    let ss_y = p.layout == DAV1D_PIXEL_LAYOUT_I420;
    let ss_x = p.layout != DAV1D_PIXEL_LAYOUT_I444;
    let cpw = ((p.w + ss_x as i32) >> ss_x as i32) as usize;
    let bitdepth_max = (1 << p.bpc) - 1;
    let stride = in_.stride;
    let src = P::planes(in_);
    let dst = P::planes_mut(out);
    let luma_src = row as usize * BLOCK_SIZE * stride[0];

    if data.num_y_points != 0 {
        let bh = imin(p.h - row * BLOCK_SIZE as i32, BLOCK_SIZE as i32);
        fgy_32x32xn::<P>(
            &mut dst[0],
            luma_src,
            &src[0],
            luma_src,
            stride[0],
            data,
            p.w as usize,
            &t.scaling[0],
            &t.grain_lut[0],
            bh,
            row,
            bitdepth_max,
        );
    }

    if data.num_uv_points[0] == 0
        && data.num_uv_points[1] == 0
        && data.chroma_scaling_from_luma == 0
    {
        return;
    }

    let bh = (imin(p.h - row * BLOCK_SIZE as i32, BLOCK_SIZE as i32) + ss_y as i32) >> ss_y as i32;

    // extend padding pixels
    // (fguv_32x32xn() clamps the read of the column at out->p.w instead;
    // see there)

    let uv_off = (row as usize * BLOCK_SIZE * stride[1]) >> ss_y as usize;
    for pl in 0..2 {
        let scaling = if data.chroma_scaling_from_luma != 0 {
            &t.scaling[0]
        } else if data.num_uv_points[pl] != 0 {
            &t.scaling[1 + pl]
        } else {
            continue;
        };
        fguv_32x32xn::<P>(
            &mut dst[1 + pl],
            uv_off,
            &src[1 + pl],
            uv_off,
            stride[1],
            data,
            cpw,
            scaling,
            &t.grain_lut[1 + pl],
            bh,
            row,
            &src[0],
            luma_src,
            stride[0],
            p.w as usize,
            pl,
            is_id,
            ss_x,
            ss_y,
            bitdepth_max,
        );
    }
}

/// `dav1d_apply_grain()`
pub(crate) fn apply_grain<P: Pixel>(
    out: &mut PictureData,
    in_: &PictureData,
    p: &Dav1dPictureParameters,
    data: &Dav1dFilmGrainData,
    is_id: bool,
) {
    let rows = (p.h + 31) >> 5;

    let t = prep_grain::<P>(out, in_, p, data);
    for row in 0..rows {
        apply_grain_row::<P>(out, in_, p, data, is_id, &t, row);
    }
}
