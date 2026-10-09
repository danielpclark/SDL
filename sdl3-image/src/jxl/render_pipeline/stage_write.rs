// Rust translation of lib/jxl/render_pipeline/stage_write.h and
// lib/jxl/render_pipeline/stage_write.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The stages that write the pipeline's output: to an RGB(A)8 buffer, to an
//! ImageBundle or to an Image3F. (The pixel callback stage is not
//! translated: SDL_image does not use output callbacks.)

use super::super::base::Status;
use super::super::color_encoding_internal::ColorEncoding;
use super::super::image::Image3F;
use super::super::math::{hwy_clamp, hwy_nearest_int};
use super::{RenderPipelineChannelMode, RenderPipelineStage, Settings, StageCtx, StageRows};

/// Translation of `StoreRGBA()` (scalar target).
#[inline]
fn store_rgba(r: u8, g: u8, b: u8, a: u8, alpha: bool, buf: &mut [u8]) {
    buf[0] = r;
    buf[1] = g;
    buf[2] = b;
    if alpha {
        buf[3] = a;
    }
}

/// `U8FromU32(BitCast(du, NearestInt(v)))` on the scalar target, which
/// demotes with saturation.
#[inline]
fn u8_from_nearest(v: f32) -> u8 {
    let i = hwy_nearest_int(v);
    i.clamp(0, 255) as u8
}

/// Translation of `WriteToU8Stage`.
struct WriteToU8Stage {
    stride: usize,
    height: usize,
    rgba: bool,
    has_alpha: bool,
    alpha_c: usize,
}

impl RenderPipelineStage for WriteToU8Stage {
    fn settings(&self) -> Settings {
        Settings::default()
    }

    fn process_row(
        &self,
        rows: &mut StageRows<'_>,
        xextra: usize,
        xsize: usize,
        xpos: usize,
        ypos: usize,
        _thread_id: usize,
        ctx: &mut StageCtx<'_>,
    ) {
        if ypos >= self.height {
            return;
        }
        debug_assert!(xextra == 0);
        let bytes = if self.rgba { 4 } else { 3 };
        let row_in_r = rows.row(rows.get_input_row(0, 0), 0, xsize);
        let row_in_g = rows.row(rows.get_input_row(1, 0), 0, xsize);
        let row_in_b = rows.row(rows.get_input_row(2, 0), 0, xsize);
        let row_in_a = if self.has_alpha {
            Some(rows.row(rows.get_input_row(self.alpha_c, 0), 0, xsize))
        } else {
            None
        };
        let Some(rgb) = ctx.rgb_output.as_mut() else {
            return;
        };
        let base_ptr = ypos * self.stride + bytes * (xpos - xextra);
        let zero = 0.0f32;
        let one = 1.0f32;
        let mul = 255.0f32;

        for x in 0..xsize {
            let rf = hwy_clamp(zero, row_in_r[x], one) * mul;
            let gf = hwy_clamp(zero, row_in_g[x], one) * mul;
            let bf = hwy_clamp(zero, row_in_b[x], one) * mul;
            let af = match row_in_a {
                Some(a) => hwy_clamp(zero, a[x], one) * mul,
                None => 255.0f32,
            };
            let r8 = u8_from_nearest(rf);
            let g8 = u8_from_nearest(gf);
            let b8 = u8_from_nearest(bf);
            let a8 = u8_from_nearest(af);
            let o = base_ptr + bytes * x;
            store_rgba(r8, g8, b8, a8, self.rgba, &mut rgb[o..o + bytes]);
        }
    }

    fn get_channel_mode(&self, c: usize) -> RenderPipelineChannelMode {
        if c < 3 || (self.has_alpha && c == self.alpha_c) {
            RenderPipelineChannelMode::Input
        } else {
            RenderPipelineChannelMode::Ignored
        }
    }

    fn get_name(&self) -> &'static str {
        "WriteToU8"
    }
}

/// Translation of `GetWriteToU8Stage()`.
pub(crate) fn get_write_to_u8_stage(
    stride: usize,
    height: usize,
    rgba: bool,
    has_alpha: bool,
    alpha_c: usize,
) -> Box<dyn RenderPipelineStage> {
    Box::new(WriteToU8Stage {
        stride,
        height,
        rgba,
        has_alpha,
        alpha_c,
    })
}

/// Which image bundle a `WriteToImageBundleStage` writes to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ImageBundleTarget {
    /// The output image bundle (`decoded`).
    Decoded,
    /// `PassesDecoderState::frame_storage_for_referencing`.
    FrameStorageForReferencing,
}

/// Translation of `WriteToImageBundleStage`.
struct WriteToImageBundleStage {
    target: ImageBundleTarget,
    color_encoding: ColorEncoding,
}

impl RenderPipelineStage for WriteToImageBundleStage {
    fn settings(&self) -> Settings {
        Settings::default()
    }

    fn set_input_sizes(
        &mut self,
        input_sizes: &[(usize, usize)],
        ctx: &mut StageCtx<'_>,
    ) -> Status {
        debug_assert!(input_sizes.len() >= 3);
        for c in 1..input_sizes.len() {
            debug_assert!(input_sizes[c].0 == input_sizes[0].0);
            debug_assert!(input_sizes[c].1 == input_sizes[0].1);
        }
        let image_bundle = match self.target {
            ImageBundleTarget::Decoded => &mut *ctx.decoded,
            ImageBundleTarget::FrameStorageForReferencing => {
                &mut *ctx.frame_storage_for_referencing
            }
        };
        // TODO(eustas): what should we do in the case of "want only ECs"?
        image_bundle.set_from_image(
            Image3F::new(input_sizes[0].0, input_sizes[0].1)?,
            &self.color_encoding,
        )?;
        // TODO(veluca): consider not reallocating ECs if not needed.
        image_bundle.extra_channels_mut().clear();
        for size in &input_sizes[3..] {
            let plane = super::super::image::ImageF::new(size.0, size.1)?;
            image_bundle.extra_channels_mut().push(plane);
        }
        Ok(())
    }

    fn process_row(
        &self,
        rows: &mut StageRows<'_>,
        xextra: usize,
        xsize: usize,
        xpos: usize,
        ypos: usize,
        _thread_id: usize,
        ctx: &mut StageCtx<'_>,
    ) {
        let image_bundle = match self.target {
            ImageBundleTarget::Decoded => &mut *ctx.decoded,
            ImageBundleTarget::FrameStorageForReferencing => {
                &mut *ctx.frame_storage_for_referencing
            }
        };
        let n = xsize + 2 * xextra;
        let x0 = -(xextra as isize);
        let (color, extra_channels) = image_bundle.color_and_extra_channels_mut();
        for c in 0..3 {
            let src = rows.row(rows.get_input_row(c, 0), x0, n);
            let dst = &mut color.plane_row(c, ypos)[xpos - xextra..xpos - xextra + n];
            dst.copy_from_slice(src);
        }
        for (ec, plane) in extra_channels.iter_mut().enumerate() {
            debug_assert!(plane.xsize() >= xpos + xsize + xextra);
            let src = rows.row(rows.get_input_row(3 + ec, 0), x0, n);
            let dst = &mut plane.row_mut(ypos)[xpos - xextra..xpos - xextra + n];
            dst.copy_from_slice(src);
        }
    }

    fn get_channel_mode(&self, _c: usize) -> RenderPipelineChannelMode {
        RenderPipelineChannelMode::Input
    }

    fn get_name(&self) -> &'static str {
        "WriteIB"
    }
}

/// Translation of `WriteToImage3FStage` (writing to `dc_frames[index]`).
struct WriteToImage3FStage {
    index: usize,
}

impl RenderPipelineStage for WriteToImage3FStage {
    fn settings(&self) -> Settings {
        Settings::default()
    }

    fn set_input_sizes(
        &mut self,
        input_sizes: &[(usize, usize)],
        ctx: &mut StageCtx<'_>,
    ) -> Status {
        debug_assert!(input_sizes.len() >= 3);
        ctx.dc_frames[self.index] = Image3F::new(input_sizes[0].0, input_sizes[0].1)?;
        Ok(())
    }

    fn process_row(
        &self,
        rows: &mut StageRows<'_>,
        xextra: usize,
        xsize: usize,
        xpos: usize,
        ypos: usize,
        _thread_id: usize,
        ctx: &mut StageCtx<'_>,
    ) {
        let n = xsize + 2 * xextra;
        let x0 = -(xextra as isize);
        let image = &mut ctx.dc_frames[self.index];
        for c in 0..3 {
            let src = rows.row(rows.get_input_row(c, 0), x0, n);
            let dst = &mut image.plane_row(c, ypos)[xpos - xextra..xpos - xextra + n];
            dst.copy_from_slice(src);
        }
    }

    fn get_channel_mode(&self, c: usize) -> RenderPipelineChannelMode {
        if c < 3 {
            RenderPipelineChannelMode::Input
        } else {
            RenderPipelineChannelMode::Ignored
        }
    }

    fn get_name(&self) -> &'static str {
        "WriteI3F"
    }
}

/// Translation of `GetWriteToImageBundleStage()`.
pub(crate) fn get_write_to_image_bundle_stage(
    target: ImageBundleTarget,
    color_encoding: ColorEncoding,
) -> Box<dyn RenderPipelineStage> {
    Box::new(WriteToImageBundleStage {
        target,
        color_encoding,
    })
}

/// Translation of `GetWriteToImage3FStage()` (for `dc_frames[index]`).
pub(crate) fn get_write_to_image3f_stage(index: usize) -> Box<dyn RenderPipelineStage> {
    Box::new(WriteToImage3FStage { index })
}
