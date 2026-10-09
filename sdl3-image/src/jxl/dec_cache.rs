// Rust translation of lib/jxl/dec_cache.h, lib/jxl/dec_cache.cc and
// lib/jxl/dct_util.h from libjxl (https://github.com/libjxl/libjxl, at the
// revision SDL_image's external/libjxl pins: libjxl 0.7.3 with SDL's
// patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Per-frame decoder state. (The pixel callback is not translated: SDL_image
//! does not use output callbacks.)

use super::ac_strategy::AcStrategy;
use super::base::{ceil_log2_nonzero_u64, Status, StatusCode, K_DCT_BLOCK_SIZE, K_GROUP_DIM_IN_BLOCKS, K_MAX_NUM_PASSES};
use super::blending::needs_blending;
use super::coeff_order::{K_COEFF_ORDER_OFFSET, K_STRATEGY_ORDER};
use super::dec_ans::AnsCode;
use super::dec_xyb::OutputEncodingInfo;
use super::frame_header::{ColorTransform, K_NOISE, K_PATCHES, K_SPLINES};
use super::image::{zero_fill_image3, Image3, ImageF};
use super::image_bundle::ImageBundle;
use super::image_metadata::ExtraChannel;
use super::math::powf;
use super::passes_state::PassesSharedState;
use super::render_pipeline::stage_blending::get_blending_stage;
use super::render_pipeline::stage_chroma_upsampling::get_chroma_upsampling_stage;
use super::render_pipeline::stage_epf::get_epf_stage;
use super::render_pipeline::stage_from_linear::get_from_linear_stage;
use super::render_pipeline::stage_gaborish::get_gaborish_stage;
use super::render_pipeline::stage_noise::{get_add_noise_stage, get_convolve_noise_stage};
use super::render_pipeline::stage_patches::get_patches_stage;
use super::render_pipeline::stage_splines::get_spline_stage;
use super::render_pipeline::stage_spot::get_spot_color_stage;
use super::render_pipeline::stage_to_linear::get_to_linear_stage;
use super::render_pipeline::stage_tone_mapping::get_tone_mapping_stage;
use super::render_pipeline::stage_upsampling::get_upsampling_stage;
use super::render_pipeline::stage_write::{
    get_write_to_image3f_stage, get_write_to_image_bundle_stage, get_write_to_u8_stage, ImageBundleTarget,
};
use super::render_pipeline::stage_xyb::get_xyb_stage;
use super::render_pipeline::stage_ycbcr::get_ycbcr_stage;
use super::render_pipeline::{Builder, RenderPipeline, RenderPipelineInput, StageCtx};

pub(crate) const K_SIGMA_BORDER: usize = 1;
pub(crate) const K_SIGMA_PADDING: usize = 2;

// --- dct_util.h ---

/// Storage for AC coefficients. Translation of `ACImage` (`ACImageT<T>`).
#[derive(Debug)]
pub(crate) enum AcImage {
    I16(Image3<i16>),
    I32(Image3<i32>),
}

impl AcImage {
    pub(crate) fn new32(xsize: usize, ysize: usize) -> Result<AcImage, StatusCode> {
        Ok(AcImage::I32(Image3::new(xsize, ysize)?))
    }
    pub(crate) fn new16(xsize: usize, ysize: usize) -> Result<AcImage, StatusCode> {
        Ok(AcImage::I16(Image3::new(xsize, ysize)?))
    }
    pub(crate) fn zero_fill(&mut self) {
        match self {
            AcImage::I16(i) => zero_fill_image3(i),
            AcImage::I32(i) => zero_fill_image3(i),
        }
    }
    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        match self {
            AcImage::I16(i) => i.xsize() == 0 || i.ysize() == 0,
            AcImage::I32(i) => i.xsize() == 0 || i.ysize() == 0,
        }
    }
}

/// Options of `PreparePipeline()`. Translation of
/// `PassesDecoderState::PipelineOptions`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PipelineOptions {
    pub coalescing: bool,
    pub render_spotcolors: bool,
}

/// Per-frame decoder state. All the images here should be accessed through
/// a group rect (either with block units or pixel units). Translation of
/// `PassesDecoderState`.
pub(crate) struct PassesDecoderState {
    pub shared_storage: PassesSharedState,

    // For ANS decoding.
    pub code: Vec<AnsCode>,
    pub context_map: Vec<Vec<u8>>,

    // Multiplier to be applied to the quant matrices of the x channel.
    pub x_dm_multiplier: f32,
    pub b_dm_multiplier: f32,

    // Sigma values for EPF.
    pub sigma: ImageF,

    // RGB8 output buffer. If set, image data will be written to this
    // buffer instead of being written to the output ImageBundle. The image data
    // is assumed to have the stride given by `rgb_stride`, hence row `i` starts
    // at position `i * rgb_stride`. (Upstream, a pointer to the user's buffer;
    // here the buffer is moved in while the frame decodes.)
    pub rgb_output: Option<Vec<u8>>,
    pub rgb_stride: usize,

    // Whether to use int16 float-XYB-to-uint8-srgb conversion.
    pub fast_xyb_srgb8_conversion: bool,

    // If true, rgb_output or callback output is RGBA using 4 instead of 3 bytes
    // per pixel.
    pub rgb_output_is_rgba: bool,
    // If true, the RGBA output will be unpremultiplied before writing to the
    // output callback (the output buffer case is handled in ConvertToExternal).
    pub unpremul_alpha: bool,

    // Used for seeding noise.
    pub visible_frame_index: usize,
    pub nonvisible_frame_index: usize,

    // Keep track of the transform types used.
    pub used_acs: u32,

    // Storage for coefficients if in "accumulate" mode.
    pub coefficients: AcImage,

    // Rendering pipeline.
    pub render_pipeline: Option<RenderPipeline>,

    // Storage for the current frame if it can be referenced by future frames.
    pub frame_storage_for_referencing: ImageBundle,

    // Information for colour conversions.
    pub output_encoding_info: OutputEncodingInfo,
}

impl PassesDecoderState {
    pub(crate) fn new() -> Self {
        PassesDecoderState {
            shared_storage: PassesSharedState::new(),
            code: Vec::new(),
            context_map: Vec::new(),
            x_dm_multiplier: 0.0,
            b_dm_multiplier: 0.0,
            sigma: ImageF::empty(),
            rgb_output: None,
            rgb_stride: 0,
            fast_xyb_srgb8_conversion: false,
            rgb_output_is_rgba: false,
            unpremul_alpha: false,
            visible_frame_index: 0,
            nonvisible_frame_index: 0,
            used_acs: 0,
            coefficients: AcImage::I32(Image3::empty()),
            render_pipeline: None,
            frame_storage_for_referencing: ImageBundle::new(None),
            output_encoding_info: OutputEncodingInfo::default(),
        }
    }

    /// Initializes decoder-specific structures using information from
    /// *shared. Translation of `Init()`. (The 8x upsampler for DC is only used
    /// when rendering the DC progressively, which SDL_image never asks for.)
    pub(crate) fn init(&mut self) -> Status {
        let fh = &self.shared_storage.frame_header;
        self.x_dm_multiplier = powf(1.0 / 1.25f32, fh.x_qm_scale as f32 - 2.0f32);
        self.b_dm_multiplier = powf(1.0 / 1.25f32, fh.b_qm_scale as f32 - 2.0f32);

        self.rgb_output = None;
        self.rgb_output_is_rgba = false;
        self.unpremul_alpha = false;
        self.fast_xyb_srgb8_conversion = false;
        self.used_acs = 0;

        if fh.loop_filter.epf_iters > 0 {
            let fd = &self.shared_storage.frame_dim;
            self.sigma = ImageF::new(fd.xsize_blocks + 2 * K_SIGMA_PADDING, fd.ysize_blocks + 2 * K_SIGMA_PADDING)?;
        }
        Ok(())
    }

    /// Initialize the decoder state after all of DC is decoded. Translation
    /// of `InitForAC()`.
    pub(crate) fn init_for_ac(&mut self) -> Status {
        let s = &mut self.shared_storage;
        s.coeff_order_size = 0;
        for o in 0..AcStrategy::K_NUM_VALID_STRATEGIES {
            if ((1u32 << o) & self.used_acs) == 0 {
                continue;
            }
            let ord = K_STRATEGY_ORDER[o as usize] as usize;
            s.coeff_order_size = s.coeff_order_size.max(K_COEFF_ORDER_OFFSET[3 * (ord + 1)] * K_DCT_BLOCK_SIZE);
        }
        let sz = s.frame_header.passes.num_passes as usize * s.coeff_order_size;
        if sz > s.coeff_orders.len() {
            if s.coeff_orders.try_reserve(sz - s.coeff_orders.len()).is_err() {
                return Err(StatusCode::GenericError);
            }
            s.coeff_orders.resize(sz, 0);
        }
        Ok(())
    }

    /// The context the render pipeline's stages need, besides `decoded`.
    pub(crate) fn stage_ctx<'a>(&'a mut self, decoded: &'a mut ImageBundle) -> (Option<&'a mut RenderPipeline>, StageCtx<'a>) {
        let PassesDecoderState {
            shared_storage,
            sigma,
            rgb_output,
            render_pipeline,
            frame_storage_for_referencing,
            ..
        } = self;
        let PassesSharedState {
            dc_frames,
            reference_frames,
            image_features,
            ..
        } = shared_storage;
        (
            render_pipeline.as_mut(),
            StageCtx {
                decoded,
                frame_storage_for_referencing,
                dc_frames,
                rgb_output,
                reference_frames,
                image_features,
                sigma,
            },
        )
    }

    /// Translation of `RenderPipelineInput::Done()` for the current pipeline.
    pub(crate) fn input_done(&mut self, input: &RenderPipelineInput, decoded: &mut ImageBundle) {
        let (pipeline, mut ctx) = self.stage_ctx(decoded);
        if let Some(p) = pipeline {
            p.input_ready(input, &mut ctx);
        }
    }

    /// Translation of `PreparePipeline()`.
    pub(crate) fn prepare_pipeline(&mut self, decoded: &mut ImageBundle, options: PipelineOptions) -> Status {
        let frame_header = self.shared_storage.frame_header.clone();
        let Some(md) = frame_header.nonserialized_metadata.clone() else {
            return Err(StatusCode::GenericError);
        };
        let mut num_c = 3 + md.m.num_extra_channels as usize;
        if (frame_header.flags & K_NOISE) != 0 {
            num_c += 3;
        }

        if frame_header.can_be_referenced() {
            // Necessary so that SetInputSizes() can allocate output buffers as needed.
            self.frame_storage_for_referencing = ImageBundle::new(decoded.metadata_rc());
        }

        let mut builder = Builder::new(num_c);

        if !frame_header.chroma_subsampling.is_444() {
            for c in 0..3 {
                if frame_header.chroma_subsampling.h_shift(c) != 0 {
                    builder.add_stage(get_chroma_upsampling_stage(c, /*horizontal=*/ true));
                }
                if frame_header.chroma_subsampling.v_shift(c) != 0 {
                    builder.add_stage(get_chroma_upsampling_stage(c, /*horizontal=*/ false));
                }
            }
        }

        if frame_header.loop_filter.gab {
            builder.add_stage(get_gaborish_stage(&frame_header.loop_filter));
        }

        {
            let lf = &frame_header.loop_filter;
            if lf.epf_iters >= 3 {
                builder.add_stage(get_epf_stage(lf, 0)?);
            }
            if lf.epf_iters >= 1 {
                builder.add_stage(get_epf_stage(lf, 1)?);
            }
            if lf.epf_iters >= 2 {
                builder.add_stage(get_epf_stage(lf, 2)?);
            }
        }

        let mut late_ec_upsample = frame_header.upsampling != 1;
        for &ecups in &frame_header.extra_channel_upsampling {
            if ecups != frame_header.upsampling {
                // If patches are applied, either frame_header.upsampling == 1 or
                // late_ec_upsample is true.
                late_ec_upsample = false;
            }
        }

        if !late_ec_upsample {
            for (ec, &ecups) in frame_header.extra_channel_upsampling.iter().enumerate() {
                if ecups != 1 {
                    builder.add_stage(get_upsampling_stage(
                        &md.transform_data,
                        3 + ec,
                        ceil_log2_nonzero_u64(ecups as u64),
                    ));
                }
            }
        }

        if (frame_header.flags & K_PATCHES) != 0 {
            builder.add_stage(get_patches_stage(
                3 + md.m.num_extra_channels as usize,
                md.m.num_extra_channels as usize,
                &md.m.extra_channel_info,
            ));
        }
        if (frame_header.flags & K_SPLINES) != 0 {
            builder.add_stage(get_spline_stage());
        }

        if frame_header.upsampling != 1 {
            let nb_channels = 3 + if late_ec_upsample {
                frame_header.extra_channel_upsampling.len()
            } else {
                0
            };
            for c in 0..nb_channels {
                builder.add_stage(get_upsampling_stage(
                    &md.transform_data,
                    c,
                    ceil_log2_nonzero_u64(frame_header.upsampling as u64),
                ));
            }
        }

        if (frame_header.flags & K_NOISE) != 0 {
            builder.add_stage(get_convolve_noise_stage(num_c - 3));
            let cmap = &self.shared_storage.cmap;
            builder.add_stage(get_add_noise_stage(
                &self.shared_storage.image_features.noise_params,
                cmap.y_to_x_ratio(0),
                cmap.y_to_b_ratio(0),
                num_c - 3,
            ));
        }
        if frame_header.dc_level != 0 {
            builder.add_stage(get_write_to_image3f_stage(frame_header.dc_level as usize - 1));
        }

        if frame_header.can_be_referenced() && frame_header.save_before_color_transform {
            builder.add_stage(get_write_to_image_bundle_stage(
                ImageBundleTarget::FrameStorageForReferencing,
                self.output_encoding_info.color_encoding.clone(),
            ));
        }

        let mut has_alpha = false;
        let mut alpha_c = 0usize;
        if let Some(dm) = decoded.metadata() {
            for (i, eci) in dm.extra_channel_info.iter().enumerate() {
                if eci.type_ == ExtraChannel::Alpha {
                    has_alpha = true;
                    alpha_c = 3 + i;
                    break;
                }
            }
        }

        let height = if options.coalescing {
            md.ysize()
        } else {
            self.shared_storage.frame_dim.ysize_upsampled
        };

        if self.fast_xyb_srgb8_conversion {
            // (HasFastXYBTosRGB8() is false on the scalar target.)
            return Err(StatusCode::GenericError);
        } else {
            let mut linear = false;
            if frame_header.color_transform == ColorTransform::YCbCr {
                builder.add_stage(get_ycbcr_stage());
            } else if frame_header.color_transform == ColorTransform::Xyb {
                builder.add_stage(get_xyb_stage(&self.output_encoding_info.opsin_params));
                linear = true;
            } // Nothing to do for kNone.

            if options.coalescing && needs_blending(&self.shared_storage.frame_header) {
                if linear {
                    builder.add_stage(get_from_linear_stage(&self.output_encoding_info)?);
                    linear = false;
                }
                builder.add_stage(get_blending_stage(
                    &self.shared_storage.frame_header,
                    &self.shared_storage.reference_frames,
                    md.m.xyb_encoded,
                    self.output_encoding_info.color_encoding_is_original,
                ));
            }

            if options.coalescing && frame_header.can_be_referenced() && !frame_header.save_before_color_transform {
                if linear {
                    builder.add_stage(get_from_linear_stage(&self.output_encoding_info)?);
                    linear = false;
                }
                builder.add_stage(get_write_to_image_bundle_stage(
                    ImageBundleTarget::FrameStorageForReferencing,
                    self.output_encoding_info.color_encoding.clone(),
                ));
            }

            if options.render_spotcolors && md.m.find(ExtraChannel::SpotColor).is_some() {
                if let Some(dm) = decoded.metadata() {
                    for (i, eci) in dm.extra_channel_info.iter().enumerate() {
                        // Don't use Find() because there may be multiple spot color channels.
                        if eci.type_ == ExtraChannel::SpotColor {
                            builder.add_stage(get_spot_color_stage(3 + i, eci.spot_color));
                        }
                    }
                }
            }

            if let Some(tone_mapping_stage) = get_tone_mapping_stage(&self.output_encoding_info)? {
                if !linear {
                    let to_linear_stage = get_to_linear_stage(&self.output_encoding_info);
                    builder.add_stage(to_linear_stage);
                    linear = true;
                }
                builder.add_stage(tone_mapping_stage);
            }

            if linear {
                builder.add_stage(get_from_linear_stage(&self.output_encoding_info)?);
            }

            if self.rgb_output.is_some() {
                builder.add_stage(get_write_to_u8_stage(
                    self.rgb_stride,
                    height,
                    self.rgb_output_is_rgba,
                    has_alpha,
                    alpha_c,
                ));
            } else {
                builder.add_stage(get_write_to_image_bundle_stage(
                    ImageBundleTarget::Decoded,
                    self.output_encoding_info.color_encoding.clone(),
                ));
            }
        }
        let frame_dim = self.shared_storage.frame_dim.clone();
        self.render_pipeline = None;
        let pipeline = {
            let (_, mut ctx) = self.stage_ctx(decoded);
            builder.finalize(frame_dim, &mut ctx)?
        };
        let status = pipeline.is_initialized();
        self.render_pipeline = Some(pipeline);
        status
    }
}

/// Temp images required for decoding a single group. Reduces memory
/// allocations for large images because we only initialize min(#threads,
/// #groups) instances. Translation of `GroupDecCache`.
#[derive(Debug, Default)]
pub(crate) struct GroupDecCache {
    // Scratch space used by DecGroupImpl().
    pub dec_group_block: Vec<f32>,
    pub dec_group_qblock: Vec<i32>,
    pub dec_group_qblock16: Vec<i16>,

    // For TransformToPixels.
    pub scratch_space: Vec<f32>,
    // Note that scratch_space is never used at the same time as dec_group_qblock.
    // Moreover, only one of dec_group_qblock16 is ever used.
    // TODO(veluca): figure out if we can save allocations.

    // AC decoding
    pub num_nzeroes: Vec<Image3<i32>>,

    max_block_area: usize,
}

impl GroupDecCache {
    /// Translation of `InitOnce()`.
    pub(crate) fn init_once(&mut self, num_passes: usize, used_acs: u32) -> Status {
        if self.num_nzeroes.len() < K_MAX_NUM_PASSES {
            self.num_nzeroes.resize_with(K_MAX_NUM_PASSES, Image3::empty);
        }
        for i in 0..num_passes {
            if self.num_nzeroes[i].xsize() == 0 {
                // Allocate enough for a whole group - partial groups on the
                // right/bottom border just use a subset. The valid size is passed via
                // Rect.

                self.num_nzeroes[i] = Image3::new(K_GROUP_DIM_IN_BLOCKS, K_GROUP_DIM_IN_BLOCKS)?;
            }
        }
        let mut max_block_area = 0usize;

        for o in 0..AcStrategy::K_NUM_VALID_STRATEGIES {
            let acs = AcStrategy::from_raw_strategy(o);
            if (used_acs & (1 << o)) == 0 {
                continue;
            }
            let area = acs.covered_blocks_x() * acs.covered_blocks_y() * K_DCT_BLOCK_SIZE;
            max_block_area = max_block_area.max(area);
        }

        if max_block_area > self.max_block_area {
            self.max_block_area = max_block_area;
            // We need 3x float blocks for dequantized coefficients and 1x for scratch
            // space for transforms.
            self.dec_group_block = vec![0f32; self.max_block_area * 3];
            self.scratch_space = vec![0f32; self.max_block_area];
            // We need 3x int32 or int16 blocks for quantized coefficients.
            self.dec_group_qblock = vec![0i32; self.max_block_area * 3];
            self.dec_group_qblock16 = vec![0i16; self.max_block_area * 3];
        }
        Ok(())
    }
}
