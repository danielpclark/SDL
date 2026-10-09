// Rust translation of lib/jxl/render_pipeline/stage_upsampling.h and
// lib/jxl/render_pipeline/stage_upsampling.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Upsamples the given channel by the given factor.

use super::super::image_metadata::CustomTransformData;
use super::super::math::{hwy_clamp, hwy_max, hwy_min};
use super::{RenderPipelineChannelMode, RenderPipelineStage, Settings, StageCtx, StageRows};

/// Translation of `UpsamplingStage`.
pub(crate) struct UpsamplingStage {
    shift: usize,
    c: usize,
    kernel: [[[[f32; 5]; 5]; 4]; 4],
}

impl UpsamplingStage {
    fn new(ups_factors: &CustomTransformData, c: usize, shift: usize) -> Self {
        let weights: &[f32] = if shift == 1 {
            &ups_factors.upsampling2_weights
        } else if shift == 2 {
            &ups_factors.upsampling4_weights
        } else {
            &ups_factors.upsampling8_weights
        };
        let mut kernel = [[[[0f32; 5]; 5]; 4]; 4];
        let n = 1usize << (shift - 1);
        for i in 0..5 * n {
            for j in 0..5 * n {
                let y = i.min(j);
                let x = i.max(j);
                kernel[j / 5][i / 5][j % 5][i % 5] = weights[5 * n * y - y * (y.wrapping_sub(1)) / 2 + x - y];
            }
        }
        UpsamplingStage { shift, c, kernel }
    }

    #[inline]
    fn kernel_n(&self, n: usize, x: usize, y: usize, ix: isize, iy: isize) -> f32 {
        let ix = (ix + 2) as usize;
        let iy = (iy + 2) as usize;
        if n == 2 {
            return self.kernel[0][0][if y % 2 != 0 { 4 - iy } else { iy }][if x % 2 != 0 { 4 - ix } else { ix }];
        }
        if n == 4 {
            return self.kernel[if y % 4 < 2 { y % 2 } else { 1 - y % 2 }][if x % 4 < 2 { x % 2 } else { 1 - x % 2 }]
                [if y % 4 < 2 { iy } else { 4 - iy }][if x % 4 < 2 { ix } else { 4 - ix }];
        }
        // N == 8
        self.kernel[if y % 8 < 4 { y % 4 } else { 3 - y % 4 }][if x % 8 < 4 { x % 4 } else { 3 - x % 4 }]
            [if y % 8 < 4 { iy } else { 4 - iy }][if x % 8 < 4 { ix } else { 4 - ix }]
    }
}

impl RenderPipelineStage for UpsamplingStage {
    fn settings(&self) -> Settings {
        Settings::symmetric(/*shift=*/ self.shift, /*border=*/ 2)
    }

    fn process_row(
        &self,
        rows: &mut StageRows<'_>,
        xextra: usize,
        xsize: usize,
        _xpos: usize,
        _ypos: usize,
        _thread_id: usize,
        _ctx: &mut StageCtx<'_>,
    ) {
        let n = 1usize << self.shift;
        debug_assert!(xextra == 0);
        let _ = xextra;
        // (ProcessRowImpl<N>)
        let input: Vec<Vec<f32>> = (-2..=2)
            .map(|iy| rows.load(rows.get_input_row(self.c, iy), -2, xsize + 4))
            .collect();
        let mut ups = [0f32; 8];
        for oy in 0..n {
            let mut dst = vec![0f32; xsize * n];
            for x in 0..xsize {
                for (ox, up) in ups.iter_mut().enumerate().take(n) {
                    let mut result = 0.0f32;
                    let mut min = input[2][x + 2];
                    let mut max = min;
                    for iy in -2isize..=2 {
                        for ix in -2isize..=2 {
                            let v = input[(iy + 2) as usize][(x as isize + ix + 2) as usize];
                            result = self.kernel_n(n, ox, oy, ix, iy) * v + result;
                            min = hwy_min(v, min);
                            max = hwy_max(v, max);
                        }
                    }
                    // Avoid overshooting.
                    *up = hwy_clamp(result, min, max);
                }
                dst[x * n..x * n + n].copy_from_slice(&ups[..n]);
            }
            let po = rows.get_output_row(self.c, oy);
            rows.store(po, 0, &dst);
        }
    }

    fn get_channel_mode(&self, c: usize) -> RenderPipelineChannelMode {
        if c == self.c {
            RenderPipelineChannelMode::InOut
        } else {
            RenderPipelineChannelMode::Ignored
        }
    }

    fn get_name(&self) -> &'static str {
        "Upsample"
    }
}

/// Translation of `GetUpsamplingStage()`.
pub(crate) fn get_upsampling_stage(
    ups_factors: &CustomTransformData,
    c: usize,
    shift: usize,
) -> Box<dyn RenderPipelineStage> {
    debug_assert!(shift != 0);
    debug_assert!(shift <= 3);
    Box::new(UpsamplingStage::new(ups_factors, c, shift))
}
