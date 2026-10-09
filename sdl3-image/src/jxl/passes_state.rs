// Rust translation of lib/jxl/passes_state.h and lib/jxl/passes_state.cc
// from libjxl (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Structures that hold the (en/de)coder state for a JPEG XL kVarDCT
//! (en/de)coder.

use std::rc::Rc;

use super::ac_context::BlockCtxMap;
use super::ac_strategy::{AcStrategyImage, CoeffOrderT};
use super::base::{jxl_failure, FrameDimensions, Status};
use super::chroma_from_luma::ColorCorrelationMap;
use super::coeff_order::K_COEFF_ORDER_MAX_SIZE;
use super::dec_noise::NoiseParams;
use super::dec_patch_dictionary::PatchDictionary;
use super::frame_header::{FrameHeader, K_USE_DC_FRAME};
use super::image::{zero_fill_image, Image3F, ImageB, ImageI, Rect};
use super::image_bundle::ImageBundle;
use super::image_metadata::CodecMetadata;
use super::quant_weights::DequantMatrices;
use super::quantizer::Quantizer;
use super::splines::Splines;

/// Translation of `ImageFeatures`.
#[derive(Clone, Debug, Default)]
pub(crate) struct ImageFeatures {
    pub noise_params: NoiseParams,
    pub patches: PatchDictionary,
    pub splines: Splines,
}

/// A reference frame slot of `PassesSharedState` (upstream, `frame` always
/// points to `storage` in the decoder).
#[derive(Clone, Debug)]
pub(crate) struct ReferenceFrame {
    pub frame: ImageBundle,
    // ImageBundle doesn't yet have a simple way to state it is in XYB.
    pub ib_is_in_xyb: bool,
}

impl Default for ReferenceFrame {
    fn default() -> Self {
        ReferenceFrame {
            frame: ImageBundle::new(None),
            ib_is_in_xyb: false,
        }
    }
}

/// State common to both encoder and decoder. Translation of
/// `PassesSharedState`.
#[derive(Debug)]
pub(crate) struct PassesSharedState {
    // Headers and metadata.
    pub metadata: Option<Rc<CodecMetadata>>,
    pub frame_header: FrameHeader,

    pub frame_dim: FrameDimensions,

    // Control fields and parameters.
    pub ac_strategy: AcStrategyImage,

    // Dequant matrices + quantizer.
    pub matrices: DequantMatrices,
    pub quantizer: Quantizer,
    pub raw_quant_field: ImageI,

    // Per-block side information for EPF detail preservation.
    pub epf_sharpness: ImageB,

    pub cmap: ColorCorrelationMap,

    pub image_features: ImageFeatures,

    // Memory area for storing coefficient orders.
    // `coeff_order_size` is the size used by *one* set of coefficient orders (at
    // most kMaxCoeffOrderSize). A set of coefficient orders is present for each
    // pass.
    pub coeff_order_size: usize,
    pub coeff_orders: Vec<CoeffOrderT>,

    // Decoder-side DC and quantized DC.
    pub quant_dc: ImageB,
    pub dc_storage: Image3F,
    /// Upstream's `dc` pointer: `None` for `dc_storage`, `Some(i)` for
    /// `dc_frames[i]`.
    pub dc_frame_index: Option<usize>,

    pub block_ctx_map: BlockCtxMap,

    pub dc_frames: [Image3F; 4],

    pub reference_frames: [ReferenceFrame; 4],

    // Number of pre-clustered set of histograms (with the same ctx map), per
    // pass. Encoded as num_histograms_ - 1.
    pub num_histograms: usize,
}

impl PassesSharedState {
    /// Translation of `PassesSharedState()`.
    pub(crate) fn new() -> Self {
        let matrices = DequantMatrices::new();
        let quantizer = Quantizer::new(&matrices);
        PassesSharedState {
            metadata: None,
            frame_header: FrameHeader::new(None),
            frame_dim: FrameDimensions::default(),
            ac_strategy: AcStrategyImage::default(),
            matrices,
            quantizer,
            raw_quant_field: ImageI::empty(),
            epf_sharpness: ImageB::empty(),
            cmap: ColorCorrelationMap::default(),
            image_features: ImageFeatures::default(),
            coeff_order_size: 0,
            coeff_orders: Vec::new(),
            quant_dc: ImageB::empty(),
            dc_storage: Image3F::empty(),
            dc_frame_index: None,
            block_ctx_map: BlockCtxMap::new(),
            dc_frames: [
                Image3F::empty(),
                Image3F::empty(),
                Image3F::empty(),
                Image3F::empty(),
            ],
            reference_frames: Default::default(),
            num_histograms: 0,
        }
    }

    /// The decoded DC image (upstream's `dc` pointer).
    pub(crate) fn dc(&self) -> &Image3F {
        match self.dc_frame_index {
            Some(i) => &self.dc_frames[i],
            None => &self.dc_storage,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn is_grayscale(&self) -> bool {
        self.metadata
            .as_ref()
            .is_some_and(|m| m.m.color_encoding.is_gray())
    }

    /// Translation of `GroupRect()`.
    pub(crate) fn group_rect(&self, group_index: usize) -> Rect {
        let fd = &self.frame_dim;
        let gx = group_index % fd.xsize_groups;
        let gy = group_index / fd.xsize_groups;
        Rect::new_clamped(
            gx * fd.group_dim,
            gy * fd.group_dim,
            fd.group_dim,
            fd.group_dim,
            fd.xsize,
            fd.ysize,
        )
    }

    /// Translation of `PaddedGroupRect()`.
    #[allow(dead_code)]
    pub(crate) fn padded_group_rect(&self, group_index: usize) -> Rect {
        let fd = &self.frame_dim;
        let gx = group_index % fd.xsize_groups;
        let gy = group_index / fd.xsize_groups;
        Rect::new_clamped(
            gx * fd.group_dim,
            gy * fd.group_dim,
            fd.group_dim,
            fd.group_dim,
            fd.xsize_padded,
            fd.ysize_padded,
        )
    }

    /// Translation of `BlockGroupRect()`.
    pub(crate) fn block_group_rect(&self, group_index: usize) -> Rect {
        let fd = &self.frame_dim;
        let gx = group_index % fd.xsize_groups;
        let gy = group_index / fd.xsize_groups;
        Rect::new_clamped(
            gx * (fd.group_dim >> 3),
            gy * (fd.group_dim >> 3),
            fd.group_dim >> 3,
            fd.group_dim >> 3,
            fd.xsize_blocks,
            fd.ysize_blocks,
        )
    }

    /// Translation of `DCGroupRect()`.
    pub(crate) fn dc_group_rect(&self, group_index: usize) -> Rect {
        let fd = &self.frame_dim;
        let gx = group_index % fd.xsize_dc_groups;
        let gy = group_index / fd.xsize_dc_groups;
        Rect::new_clamped(
            gx * fd.group_dim,
            gy * fd.group_dim,
            fd.group_dim,
            fd.group_dim,
            fd.xsize_blocks,
            fd.ysize_blocks,
        )
    }
}

/// Initialized the state information that is shared between encoder and
/// decoder. Translation of `InitializePassesSharedState()` (decoder side).
pub(crate) fn initialize_passes_shared_state(
    frame_header: &FrameHeader,
    shared: &mut PassesSharedState,
) -> Status {
    debug_assert!(frame_header.nonserialized_metadata.is_some());
    shared.frame_header = frame_header.clone();
    shared.metadata = frame_header.nonserialized_metadata.clone();
    shared.frame_dim = frame_header.to_frame_dimensions();

    let frame_dim = shared.frame_dim;

    shared.ac_strategy = AcStrategyImage::new(frame_dim.xsize_blocks, frame_dim.ysize_blocks)?;
    shared.raw_quant_field = ImageI::new(frame_dim.xsize_blocks, frame_dim.ysize_blocks)?;
    shared.epf_sharpness = ImageB::new(frame_dim.xsize_blocks, frame_dim.ysize_blocks)?;
    shared.cmap = ColorCorrelationMap::new(frame_dim.xsize, frame_dim.ysize, true)?;

    // In the decoder, we allocate coeff orders afterwards, when we know how many
    // we will actually need.
    shared.coeff_order_size = K_COEFF_ORDER_MAX_SIZE;

    shared.quant_dc = ImageB::new(frame_dim.xsize_blocks, frame_dim.ysize_blocks)?;

    let use_dc_frame = frame_header.flags & K_USE_DC_FRAME != 0;
    if use_dc_frame {
        if frame_header.dc_level == 4 {
            return jxl_failure!("Invalid DC level for kUseDcFrame");
        }
        shared.dc_storage = Image3F::empty();
        let level = frame_header.dc_level as usize;
        shared.dc_frame_index = Some(level);
        if shared.dc_frames[level].xsize() == 0 {
            return jxl_failure!(
                "kUseDcFrame specified for dc_level, but no frame was decoded with level + 1"
            );
        }
        zero_fill_image(&mut shared.quant_dc);
    } else {
        shared.dc_storage = Image3F::new(frame_dim.xsize_blocks, frame_dim.ysize_blocks)?;
        shared.dc_frame_index = None;
    }

    Ok(())
}
