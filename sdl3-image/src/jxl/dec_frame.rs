// Rust translation of lib/jxl/dec_frame.h and lib/jxl/dec_frame.cc from
// libjxl (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Decodes a frame. (Single-threaded; JPEG reconstruction is not
//! translated, so `decoded->IsJPEG()` is always false. Progressive pauses
//! are only requested through `JXL_DEC_FRAME_PROGRESSION`, which SDL_image
//! does not subscribe to, so the progressive detail is always `kFrames`.)

use super::ac_context::{
    decode_block_ctx_map, K_ZERO_DENSITY_CONTEXT_COUNT, K_ZERO_DENSITY_CONTEXT_LIMIT,
};
use super::ac_strategy::CoeffOrderT;
use super::base::{
    ceil_log2_nonzero_u64, jxl_failure, FrameDimensions, Status, StatusCode, K_GROUP_DIM,
    K_MAX_NUM_PASSES,
};
use super::coeff_order::decode_coeff_orders;
use super::compressed_dc::adaptive_dc_smoothing;
use super::dec_ans::decode_histograms;
use super::dec_bit_reader::BitReader;
use super::dec_cache::{AcImage, GroupDecCache, PassesDecoderState, PipelineOptions};
use super::dec_group::decode_group;
use super::dec_modular::{ModularFrameDecoder, ModularStreamId};
use super::dec_noise::{decode_noise, random_3_planes};
use super::epf::K_INV_SIGMA_NUM;
use super::fields::{bundle_read, u32_coder_read};
use super::frame_header::{
    BlendMode, FrameEncoding, FrameHeader, FrameType, K_NOISE, K_ORDER_ENC, K_PATCHES,
    K_SKIP_ADAPTIVE_DC_SMOOTHING, K_SPLINES, K_USE_DC_FRAME,
};
use super::image::{fill_image, Rect};
use super::image_bundle::ImageBundle;
use super::image_metadata::CodecMetadata;
use super::passes_state::initialize_passes_shared_state;
use super::quant_weights::DequantMatrices;
use super::toc::{num_toc_entries, read_toc};

/// Translation of `FrameDecoder::SectionInfo`.
pub(crate) struct SectionInfo<'a> {
    pub br: BitReader<'a>,
    pub id: usize,
}

/// Translation of `FrameDecoder::TocEntry`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TocEntry {
    pub size: usize,
    pub id: usize,
}

/// Translation of `FrameDecoder::SectionStatus`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SectionStatus {
    // Processed correctly.
    Done = 0,
    // Skipped because other required sections were not yet processed.
    Skipped = 1,
    // Skipped because the section was already processed.
    Duplicate = 2,
    // Only partially decoded: the section will need to be processed again.
    Partial = 3,
}

/// Translation of `FrameDecoder`. The output image bundle (`decoded`) is
/// owned by the frame decoder while the frame decodes.
pub(crate) struct FrameDecoder {
    toc: Vec<TocEntry>,
    section_sizes_sum: u64,
    // TODO(veluca): figure out the duplication between these and dec_state_.
    frame_header: FrameHeader,
    frame_dim: FrameDimensions,
    pub decoded: ImageBundle,
    modular_frame_decoder: ModularFrameDecoder,
    render_spotcolors: bool,
    coalescing: bool,

    processed_section: Vec<u8>,
    decoded_passes_per_ac_group: Vec<u8>,
    decoded_dc_groups: Vec<u8>,
    decoded_dc_global: bool,
    decoded_ac_global: bool,
    finalized_dc: bool,
    num_sections_done: usize,
    is_finalized: bool,
    allocated: bool,

    group_dec_caches: Vec<GroupDecCache>,
}

impl FrameDecoder {
    /// Translation of `FrameDecoder(dec_state, metadata, pool, ...)`.
    pub(crate) fn new(metadata: std::rc::Rc<CodecMetadata>) -> Self {
        FrameDecoder {
            toc: Vec::new(),
            section_sizes_sum: 0,
            frame_header: FrameHeader::new(Some(metadata)),
            frame_dim: FrameDimensions::default(),
            decoded: ImageBundle::new(None),
            modular_frame_decoder: ModularFrameDecoder::default(),
            render_spotcolors: true,
            coalescing: true,
            processed_section: Vec::new(),
            decoded_passes_per_ac_group: Vec::new(),
            decoded_dc_groups: Vec::new(),
            decoded_dc_global: false,
            decoded_ac_global: false,
            finalized_dc: true,
            num_sections_done: 0,
            is_finalized: true,
            allocated: false,
            group_dec_caches: Vec::new(),
        }
    }

    pub(crate) fn set_render_spotcolors(&mut self, rsc: bool) {
        self.render_spotcolors = rsc;
    }
    pub(crate) fn set_coalescing(&mut self, c: bool) {
        self.coalescing = c;
    }

    pub(crate) fn sum_section_sizes(&self) -> u64 {
        self.section_sizes_sum
    }
    pub(crate) fn toc(&self) -> &Vec<TocEntry> {
        &self.toc
    }

    pub(crate) fn get_frame_header(&self) -> &FrameHeader {
        &self.frame_header
    }

    /// Returns whether a DC image has been decoded, accessible at low
    /// resolution at passes.shared_storage.dc_storage. Translation of
    /// `HasDecodedDC()`.
    #[allow(dead_code)]
    pub(crate) fn has_decoded_dc(&self) -> bool {
        self.finalized_dc
    }
    pub(crate) fn has_decoded_all(&self) -> bool {
        self.toc.len() == self.num_sections_done
    }

    /// Read FrameHeader and table of contents from the given BitReader.
    /// Also checks frame dimensions for their limits, and sets the output
    /// image buffer. Translation of `InitFrame()`.
    pub(crate) fn init_frame(
        &mut self,
        br: &mut BitReader<'_>,
        decoded: ImageBundle,
        dec_state: &mut PassesDecoderState,
        is_preview: bool,
        output_needed: bool,
    ) -> Status {
        self.decoded = decoded;
        debug_assert!(self.is_finalized);

        // Reset the dequantization matrices to their default values.
        dec_state.shared_storage.matrices = DequantMatrices::new();

        self.frame_header.nonserialized_is_preview = is_preview;
        debug_assert!(self.frame_header.nonserialized_metadata.is_some());
        bundle_read(br, &mut self.frame_header)?;
        self.frame_dim = self.frame_header.to_frame_dimensions();

        let num_passes = self.frame_header.passes.num_passes as usize;
        let num_groups = self.frame_dim.num_groups;

        // If the previous frame was not a kRegularFrame, `decoded` may have different
        // dimensions; must reset to avoid errors.
        self.decoded.remove_color();
        self.decoded.clear_extra_channels();

        self.decoded.duration = self.frame_header.animation_frame.duration;

        if !self.frame_header.nonserialized_is_preview
            && (self.frame_header.is_last || self.frame_header.animation_frame.duration > 0)
            && (self.frame_header.frame_type == FrameType::RegularFrame
                || self.frame_header.frame_type == FrameType::SkipProgressive)
        {
            dec_state.visible_frame_index += 1;
            dec_state.nonvisible_frame_index = 0;
        } else {
            dec_state.nonvisible_frame_index += 1;
        }

        // Read TOC.
        let has_ac_global = true;
        let toc_entries = num_toc_entries(
            num_groups,
            self.frame_dim.num_dc_groups,
            num_passes,
            has_ac_global,
        );
        let mut sizes: Vec<u32> = Vec::new();
        let mut permutation: Vec<CoeffOrderT> = Vec::new();
        read_toc(toc_entries, br, &mut sizes, &mut permutation)?;
        let have_permutation = !permutation.is_empty();
        self.toc.clear();
        self.toc.resize(toc_entries, TocEntry::default());
        self.section_sizes_sum = 0;
        for i in 0..toc_entries {
            self.toc[i].size = sizes[i] as usize;
            let index = if have_permutation {
                permutation[i] as usize
            } else {
                i
            };
            self.toc[index].id = i;
            if self.section_sizes_sum.wrapping_add(self.toc[i].size as u64) < self.section_sizes_sum
            {
                return jxl_failure!("group offset overflow");
            }
            self.section_sizes_sum += self.toc[i].size as u64;
        }

        debug_assert!((br.total_bits_consumed() % 8) == 0);
        let group_codes_begin = (br.total_bits_consumed() / 8) as u64;
        debug_assert!(!self.toc.is_empty());

        // Overflow check.
        if group_codes_begin.wrapping_add(self.section_sizes_sum) < group_codes_begin {
            return jxl_failure!("Invalid group codes");
        }

        if !self.frame_header.chroma_subsampling.is_444()
            && (self.frame_header.flags & K_SKIP_ADAPTIVE_DC_SMOOTHING) == 0
            && self.frame_header.encoding == FrameEncoding::VarDct
        {
            return jxl_failure!(
                "Non-444 chroma subsampling is not allowed when adaptive DC smoothing is enabled"
            );
        }

        if !output_needed {
            return Ok(());
        }
        initialize_passes_shared_state(&self.frame_header, &mut dec_state.shared_storage)?;
        dec_state.init()?;
        self.modular_frame_decoder = ModularFrameDecoder::default();
        self.modular_frame_decoder.init(&self.frame_dim);

        // Clear the state.
        self.decoded_dc_global = false;
        self.decoded_ac_global = false;
        self.is_finalized = false;
        self.finalized_dc = false;
        self.num_sections_done = 0;
        self.decoded_dc_groups.clear();
        self.decoded_dc_groups
            .resize(self.frame_dim.num_dc_groups, 0);
        self.decoded_passes_per_ac_group.clear();
        self.decoded_passes_per_ac_group
            .resize(self.frame_dim.num_groups, 0);
        self.processed_section.clear();
        self.processed_section.resize(self.toc.len(), 0);
        self.allocated = false;
        Ok(())
    }

    /// Translation of `ProcessDCGlobal()`.
    fn process_dc_global(
        &mut self,
        br: &mut BitReader<'_>,
        dec_state: &mut PassesDecoderState,
    ) -> Status {
        let shared = &mut dec_state.shared_storage;
        if shared.frame_header.flags & K_PATCHES != 0 {
            let mut uses_extra_channels = false;
            let md = shared.metadata.clone();
            let (num_ec, eci) = match md.as_ref() {
                Some(m) => (
                    m.m.num_extra_channels as usize,
                    m.m.extra_channel_info.as_slice(),
                ),
                None => (0, &[][..]),
            };
            shared.image_features.patches.decode(
                br,
                self.frame_dim.xsize_padded,
                self.frame_dim.ysize_padded,
                &mut uses_extra_channels,
                num_ec,
                eci,
                &shared.reference_frames,
            )?;
            if uses_extra_channels && self.frame_header.upsampling != 1 {
                for &ecups in &self.frame_header.extra_channel_upsampling {
                    if ecups != self.frame_header.upsampling {
                        return jxl_failure!(
                            "Cannot use extra channels in patches if color channels are subsampled differently from extra channels"
                        );
                    }
                }
            }
        } else {
            shared.image_features.patches.clear();
        }
        shared.image_features.splines.clear();
        if shared.frame_header.flags & K_SPLINES != 0 {
            shared
                .image_features
                .splines
                .decode(br, self.frame_dim.xsize.wrapping_mul(self.frame_dim.ysize))?;
        }
        if shared.frame_header.flags & K_NOISE != 0 {
            decode_noise(br, &mut shared.image_features.noise_params)?;
        }
        shared.matrices.decode_dc(br)?;

        if self.frame_header.encoding == FrameEncoding::VarDct {
            // (DecodeGlobalDCInfo)
            shared.quantizer.decode(br, &shared.matrices)?;

            decode_block_ctx_map(br, &mut shared.block_ctx_map)?;

            shared.cmap.decode_dc(br)?;

            // Pre-compute info for decoding a group.
            // (decoded->IsJPEG() is false: the DC stays dequantized.)

            shared.ac_strategy.fill_invalid();
        }
        // Splines' draw cache uses the color correlation map.
        if shared.frame_header.flags & K_SPLINES != 0 {
            let ytox = shared.cmap.y_to_x_ratio(0);
            let ytob = shared.cmap.y_to_b_ratio(0);
            shared.image_features.splines.initialize_draw_cache(
                self.frame_dim.xsize_upsampled,
                self.frame_dim.ysize_upsampled,
                ytox,
                ytob,
            )?;
        }
        let dec_status = self.modular_frame_decoder.decode_global_info(
            br,
            &self.frame_header,
            /*allow_truncated_group=*/ false,
        );
        if dec_status == Err(StatusCode::GenericError) {
            return dec_status;
        }
        if dec_status.is_ok() {
            self.decoded_dc_global = true;
        }
        dec_status
    }

    /// Translation of `ProcessDCGroup()`.
    fn process_dc_group(
        &mut self,
        dc_group_id: usize,
        br: &mut BitReader<'_>,
        dec_state: &mut PassesDecoderState,
    ) -> Status {
        let gx = dc_group_id % self.frame_dim.xsize_dc_groups;
        let gy = dc_group_id / self.frame_dim.xsize_dc_groups;
        if self.frame_header.encoding == FrameEncoding::VarDct
            && (self.frame_header.flags & K_USE_DC_FRAME) == 0
        {
            self.modular_frame_decoder
                .decode_var_dct_dc(dc_group_id, br, dec_state)?;
        }
        let mrect = Rect::new(
            gx * self.frame_dim.dc_group_dim,
            gy * self.frame_dim.dc_group_dim,
            self.frame_dim.dc_group_dim,
            self.frame_dim.dc_group_dim,
        );
        self.modular_frame_decoder.decode_group(
            &mrect,
            Some(br),
            3,
            1000,
            &ModularStreamId::modular_dc(dc_group_id),
            /*zerofill=*/ false,
            None,
            None,
            /*allow_truncated=*/ false,
            None,
        )?;
        if self.frame_header.encoding == FrameEncoding::VarDct {
            self.modular_frame_decoder
                .decode_ac_metadata(dc_group_id, br, dec_state)?;
        } else {
            let lf = &dec_state.shared_storage.frame_header.loop_filter;
            if lf.epf_iters > 0 {
                let v = K_INV_SIGMA_NUM / lf.epf_sigma_for_modular;
                fill_image(v, &mut dec_state.sigma);
            }
        }
        self.decoded_dc_groups[dc_group_id] = 1;
        Ok(())
    }

    /// Translation of `FinalizeDC()`.
    fn finalize_dc(&mut self, dec_state: &mut PassesDecoderState) -> Status {
        // Do Adaptive DC smoothing if enabled. This *must* happen between all the
        // ProcessDCGroup and ProcessACGroup.
        if self.frame_header.encoding == FrameEncoding::VarDct
            && (self.frame_header.flags & K_SKIP_ADAPTIVE_DC_SMOOTHING) == 0
            && (self.frame_header.flags & K_USE_DC_FRAME) == 0
        {
            let shared = &mut dec_state.shared_storage;
            let mul_dc = *shared.quantizer.mul_dc();
            adaptive_dc_smoothing(&mul_dc, &mut shared.dc_storage)?;
        }

        self.finalized_dc = true;
        Ok(())
    }

    /// Translation of `AllocateOutput()`.
    fn allocate_output(&mut self, dec_state: &mut PassesDecoderState) -> Status {
        if self.allocated {
            return Ok(());
        }
        self.modular_frame_decoder.maybe_drop_full_image();
        self.decoded.origin = dec_state.shared_storage.frame_header.frame_origin;
        dec_state.init_for_ac()?;
        self.allocated = true;
        Ok(())
    }

    /// Translation of `ProcessACGlobal()`.
    fn process_ac_global(
        &mut self,
        br: &mut BitReader<'_>,
        dec_state: &mut PassesDecoderState,
    ) -> Status {
        if !self.finalized_dc {
            // JXL_CHECK(finalized_dc_)
            return Err(StatusCode::GenericError);
        }

        // Decode AC group.
        if self.frame_header.encoding == FrameEncoding::VarDct {
            dec_state
                .shared_storage
                .matrices
                .decode(br, Some(&self.modular_frame_decoder))?;
            let used_acs = dec_state.used_acs;
            dec_state
                .shared_storage
                .matrices
                .ensure_computed(used_acs)?;

            let num_histo_bits =
                ceil_log2_nonzero_u64(dec_state.shared_storage.frame_dim.num_groups as u64);
            dec_state.shared_storage.num_histograms = 1 + br.read_bits(num_histo_bits) as usize;

            dec_state
                .code
                .resize_with(K_MAX_NUM_PASSES, Default::default);
            dec_state
                .context_map
                .resize_with(K_MAX_NUM_PASSES, Vec::new);
            // Read coefficient orders and histograms.
            let mut max_num_bits_ac = 0usize;
            let num_passes = dec_state.shared_storage.frame_header.passes.num_passes as usize;
            for i in 0..num_passes {
                let used_orders = u32_coder_read(K_ORDER_ENC, br) as u16;
                let size = dec_state.shared_storage.coeff_order_size;
                let used_acs = dec_state.used_acs;
                decode_coeff_orders(
                    used_orders,
                    used_acs,
                    &mut dec_state.shared_storage.coeff_orders[i * size..],
                    br,
                )?;
                let num_contexts = dec_state.shared_storage.num_histograms
                    * dec_state.shared_storage.block_ctx_map.num_ac_contexts() as usize;
                decode_histograms(
                    br,
                    num_contexts,
                    &mut dec_state.code[i],
                    &mut dec_state.context_map[i],
                    false,
                )?;
                // Add extra values to enable the cheat in hot loop of DecodeACVarBlock.
                dec_state.context_map[i].resize(
                    num_contexts + K_ZERO_DENSITY_CONTEXT_LIMIT - K_ZERO_DENSITY_CONTEXT_COUNT,
                    0,
                );
                max_num_bits_ac = max_num_bits_ac.max(dec_state.code[i].max_num_bits);
            }
            max_num_bits_ac += ceil_log2_nonzero_u64(num_passes as u64);
            // 16-bit buffer for decoding to JPEG are not implemented.
            // TODO(veluca): figure out the exact limit - 16 should still work with
            // 16-bit buffers, but we are excluding it for safety.
            let use_16_bit = max_num_bits_ac < 16;
            let store = self.frame_header.passes.num_passes > 1;
            let xs = if store { K_GROUP_DIM * K_GROUP_DIM } else { 0 };
            let ys = if store { self.frame_dim.num_groups } else { 0 };
            dec_state.coefficients = if use_16_bit {
                AcImage::new16(xs, ys)?
            } else {
                AcImage::new32(xs, ys)?
            };
            if store {
                dec_state.coefficients.zero_fill();
            }
        }

        // (No JPEG decoding data to set.)
        self.decoded_ac_global = true;
        Ok(())
    }

    /// Translation of `ProcessACGroup()`.
    #[allow(clippy::too_many_arguments)]
    fn process_ac_group(
        &mut self,
        ac_group_id: usize,
        br: &mut [&mut BitReader<'_>],
        num_passes: usize,
        thread: usize,
        force_draw: bool,
        dc_only: bool,
        dec_state: &mut PassesDecoderState,
    ) -> Status {
        let group_dim = self.frame_dim.group_dim;
        let gx = ac_group_id % self.frame_dim.xsize_groups;
        let gy = ac_group_id / self.frame_dim.xsize_groups;
        let x = gx * group_dim;
        let y = gy * group_dim;

        let render_pipeline_input = match dec_state.render_pipeline.as_ref() {
            Some(p) => p.get_input_buffers(ac_group_id, thread),
            None => return Err(StatusCode::GenericError),
        };

        let mut should_run_pipeline = true;

        if self.frame_header.encoding == FrameEncoding::VarDct {
            if self.group_dec_caches.len() <= thread {
                self.group_dec_caches
                    .resize_with(thread + 1, GroupDecCache::default);
            }
            self.group_dec_caches[thread].init_once(
                self.frame_header.passes.num_passes as usize,
                dec_state.used_acs,
            )?;
            decode_group(
                br,
                num_passes,
                ac_group_id,
                dec_state,
                &mut self.group_dec_caches[thread],
                thread,
                &render_pipeline_input,
                self.decoded_passes_per_ac_group[ac_group_id] as usize,
                force_draw,
                dc_only,
                Some(&mut should_run_pipeline),
            )?;
        }

        // don't limit to image dimensions here (is done in DecodeGroup)
        let mrect = Rect::new(x, y, group_dim, group_dim);
        let mut modular_ready = false;
        let pass0 = self.decoded_passes_per_ac_group[ac_group_id] as usize;
        let pass1 = if force_draw {
            self.frame_header.passes.num_passes as usize
        } else {
            pass0 + num_passes
        };
        for i in pass0..pass1 {
            let mut min_shift = 0i32;
            let mut max_shift = 0i32;
            self.frame_header
                .passes
                .get_downsampling_bracket(i, &mut min_shift, &mut max_shift);
            let mut modular_pass_ready = true;
            if i < pass0 + num_passes {
                self.modular_frame_decoder.decode_group(
                    &mrect,
                    Some(&mut *br[i - pass0]),
                    min_shift,
                    max_shift,
                    &ModularStreamId::modular_ac(ac_group_id, i),
                    /*zerofill=*/ false,
                    Some(dec_state),
                    Some(&render_pipeline_input),
                    /*allow_truncated=*/ false,
                    Some(&mut modular_pass_ready),
                )?;
            } else {
                self.modular_frame_decoder.decode_group(
                    &mrect,
                    None,
                    min_shift,
                    max_shift,
                    &ModularStreamId::modular_ac(ac_group_id, i),
                    /*zerofill=*/ true,
                    Some(dec_state),
                    Some(&render_pipeline_input),
                    /*allow_truncated=*/ false,
                    Some(&mut modular_pass_ready),
                )?;
            }
            if modular_pass_ready {
                modular_ready = true;
            }
        }
        self.decoded_passes_per_ac_group[ac_group_id] =
            self.decoded_passes_per_ac_group[ac_group_id].wrapping_add(num_passes as u8);

        if (self.frame_header.flags & K_NOISE) != 0 {
            let num_ec = self
                .frame_header
                .nonserialized_metadata
                .as_ref()
                .map_or(0, |m| m.m.num_extra_channels as usize);
            let noise_c_start = 3 + num_ec;
            // When the color channels are downsampled, we need to generate more noise
            // input for the current group than just the group dimensions.
            let upsampling = self.frame_header.upsampling as usize;
            for iy in 0..upsampling {
                for ix in 0..upsampling {
                    let mut rects = [(0usize, Rect::default()); 3];
                    for (c, rc) in rects.iter_mut().enumerate() {
                        let (buf, r) = render_pipeline_input.get_buffer(noise_c_start + c)?;
                        let x1 = r.x0() + r.xsize();
                        let y1 = r.y0() + r.ysize();
                        *rc = (
                            buf,
                            Rect::new_clamped(
                                r.x0() + ix * group_dim,
                                r.y0() + iy * group_dim,
                                group_dim,
                                group_dim,
                                x1,
                                y1,
                            ),
                        );
                    }
                    let visible = dec_state.visible_frame_index;
                    let nonvisible = dec_state.nonvisible_frame_index;
                    let Some(pipeline) = dec_state.render_pipeline.as_mut() else {
                        return Err(StatusCode::GenericError);
                    };
                    let mut planes = pipeline.buffers_mut3([rects[0].0, rects[1].0, rects[2].0])?;
                    let [p0, p1, p2] = &mut planes;
                    random_3_planes(
                        visible,
                        nonvisible,
                        (gx * upsampling + ix) * group_dim,
                        (gy * upsampling + iy) * group_dim,
                        [
                            (&mut **p0, rects[0].1),
                            (&mut **p1, rects[1].1),
                            (&mut **p2, rects[2].1),
                        ],
                    );
                }
            }
        }

        if !self.modular_frame_decoder.uses_full_image() {
            if should_run_pipeline && modular_ready {
                dec_state.input_done(&render_pipeline_input, &mut self.decoded);
            } else if force_draw {
                return jxl_failure!("Modular group decoding failed.");
            }
        }
        Ok(())
    }

    /// Translation of `MarkSections()`.
    fn mark_sections(&mut self, sections: &[SectionInfo<'_>], section_status: &[SectionStatus]) {
        self.num_sections_done += sections.len();
        for (i, s) in sections.iter().enumerate() {
            if section_status[i] != SectionStatus::Done {
                self.processed_section[s.id] = 0;
                self.num_sections_done -= 1;
            }
        }
    }

    /// Processes `num` sections; each SectionInfo contains the index of the
    /// section and a BitReader that only contains the data of the section.
    /// `section_status` should point to `num` elements, and will be filled
    /// with information about whether each section was processed or not. A
    /// section is a part of the encoded file that is indexed by the TOC.
    /// Translation of `ProcessSections()`.
    pub(crate) fn process_sections(
        &mut self,
        sections: &mut [SectionInfo<'_>],
        section_status: &mut [SectionStatus],
        dec_state: &mut PassesDecoderState,
    ) -> Status {
        let num = sections.len();
        if num == 0 {
            return Ok(()); // Nothing to process
        }
        section_status.fill(SectionStatus::Skipped);
        let mut dc_global_sec = num;
        let mut ac_global_sec = num;
        let mut dc_group_sec: Vec<usize> = vec![num; self.frame_dim.num_dc_groups];
        let mut ac_group_sec: Vec<Vec<usize>> =
            vec![
                vec![num; self.frame_header.passes.num_passes as usize];
                self.frame_dim.num_groups
            ];
        // This keeps track of the number of ac passes we want to process during this
        // call of ProcessSections.
        let mut desired_num_ac_passes: Vec<usize> = vec![0; self.frame_dim.num_groups];
        let single_section =
            self.frame_dim.num_groups == 1 && self.frame_header.passes.num_passes == 1;
        if single_section {
            if !(num == 1 && sections[0].id == 0) {
                // JXL_ASSERT(num == 1); JXL_ASSERT(sections[0].id == 0);
                return Err(StatusCode::GenericError);
            }
            if self.processed_section[0] == 0 {
                self.processed_section[0] = 1;
                ac_group_sec[0].resize(1, num);
                dc_global_sec = 0;
                ac_global_sec = 0;
                dc_group_sec[0] = 0;
                ac_group_sec[0][0] = 0;
                desired_num_ac_passes[0] = 1;
            } else {
                section_status[0] = SectionStatus::Duplicate;
            }
        } else {
            let ac_global_index = self.frame_dim.num_dc_groups + 1;
            for i in 0..num {
                if sections[i].id >= self.processed_section.len() {
                    // JXL_ASSERT(sections[i].id < processed_section_.size())
                    return Err(StatusCode::GenericError);
                }
                if self.processed_section[sections[i].id] != 0 {
                    section_status[i] = SectionStatus::Duplicate;
                    continue;
                }
                if sections[i].id == 0 {
                    dc_global_sec = i;
                } else if sections[i].id < ac_global_index {
                    dc_group_sec[sections[i].id - 1] = i;
                } else if sections[i].id == ac_global_index {
                    ac_global_sec = i;
                } else {
                    let ac_idx = sections[i].id - ac_global_index - 1;
                    let acg = ac_idx % self.frame_dim.num_groups;
                    let acp = ac_idx / self.frame_dim.num_groups;
                    if acp >= self.frame_header.passes.num_passes as usize {
                        return jxl_failure!("Invalid section ID");
                    }
                    ac_group_sec[acg][acp] = i;
                }
                self.processed_section[sections[i].id] = 1;
            }
            // Count number of new passes per group.
            for g in 0..ac_group_sec.len() {
                let mut j = 0usize;
                while j + (self.decoded_passes_per_ac_group[g] as usize)
                    < self.frame_header.passes.num_passes as usize
                {
                    if ac_group_sec[g][j + self.decoded_passes_per_ac_group[g] as usize] == num {
                        break;
                    }
                    j += 1;
                }
                desired_num_ac_passes[g] = j;
            }
        }
        if dc_global_sec != num {
            let dc_global_status =
                self.process_dc_global(&mut sections[dc_global_sec].br, dec_state);
            if dc_global_status == Err(StatusCode::GenericError) {
                return dc_global_status;
            }
            if dc_global_status.is_ok() {
                section_status[dc_global_sec] = SectionStatus::Done;
            } else {
                section_status[dc_global_sec] = SectionStatus::Partial;
            }
        }

        let mut has_error = false;
        if self.decoded_dc_global {
            for i in 0..dc_group_sec.len() {
                if dc_group_sec[i] != num {
                    if self
                        .process_dc_group(i, &mut sections[dc_group_sec[i]].br, dec_state)
                        .is_err()
                    {
                        has_error = true;
                    } else {
                        section_status[dc_group_sec[i]] = SectionStatus::Done;
                    }
                }
            }
        }
        if has_error {
            return jxl_failure!("Error in DC group");
        }

        if self.decoded_dc_groups.iter().copied().min().unwrap_or(0) != 0 && !self.finalized_dc {
            let pipeline_options = PipelineOptions {
                coalescing: self.coalescing,
                render_spotcolors: self.render_spotcolors,
            };
            dec_state.prepare_pipeline(&mut self.decoded, pipeline_options)?;
            self.finalize_dc(dec_state)?;
            self.allocate_output(dec_state)?;
            // (progressive_detail_ is kFrames: no pause at the DC.)
        }

        if self.finalized_dc && ac_global_sec != num && !self.decoded_ac_global {
            self.process_ac_global(&mut sections[ac_global_sec].br, dec_state)?;
            section_status[ac_global_sec] = SectionStatus::Done;
        }

        if self.decoded_ac_global {
            // Mark all the AC groups that we received as not complete yet.
            for i in 0..ac_group_sec.len() {
                if desired_num_ac_passes[i] != 0 {
                    if let Some(p) = dec_state.render_pipeline.as_mut() {
                        p.clear_done(i);
                    }
                }
            }

            self.prepare_storage(1, self.decoded_passes_per_ac_group.len(), dec_state)?;
            for g in 0..ac_group_sec.len() {
                if desired_num_ac_passes[g] == 0 {
                    // no new AC pass, nothing to do
                    continue;
                }
                let first_pass = self.decoded_passes_per_ac_group[g] as usize;
                let mut idxs: Vec<usize> = Vec::with_capacity(desired_num_ac_passes[g]);
                for i in 0..desired_num_ac_passes[g] {
                    debug_assert!(ac_group_sec[g][first_pass + i] != num);
                    idxs.push(ac_group_sec[g][first_pass + i]);
                }
                let result = {
                    let mut readers = take_readers(sections, &idxs);
                    let mut refs: Vec<&mut BitReader<'_>> =
                        readers.iter_mut().map(|r| &mut r.1).collect();
                    let r = self.process_ac_group(
                        g,
                        &mut refs,
                        desired_num_ac_passes[g],
                        0,
                        false,
                        false,
                        dec_state,
                    );
                    drop(refs);
                    put_back_readers(sections, readers);
                    r
                };
                if result.is_err() {
                    has_error = true;
                } else {
                    for i in 0..desired_num_ac_passes[g] {
                        section_status[ac_group_sec[g][first_pass + i]] = SectionStatus::Done;
                    }
                }
            }
        }
        if has_error {
            return jxl_failure!("Error in AC group");
        }

        self.mark_sections(sections, section_status);
        Ok(())
    }

    /// Translation of `PrepareStorage()` (one thread).
    fn prepare_storage(
        &mut self,
        num_threads: usize,
        num_tasks: usize,
        dec_state: &mut PassesDecoderState,
    ) -> Status {
        let storage_size = num_threads.min(num_tasks);
        if storage_size > self.group_dec_caches.len() {
            self.group_dec_caches
                .resize_with(storage_size, GroupDecCache::default);
        }
        let use_group_ids = self.modular_frame_decoder.uses_full_image()
            && (self.frame_header.encoding == FrameEncoding::VarDct
                || (self.frame_header.flags & K_NOISE) != 0);
        if let Some(p) = dec_state.render_pipeline.as_mut() {
            p.prepare_for_threads(storage_size, use_group_ids)?;
        }
        Ok(())
    }

    /// Flushes all the data decoded so far to pixels. Translation of
    /// `Flush()`.
    fn flush(&mut self, dec_state: &mut PassesDecoderState) -> Status {
        let mut has_blending = self.frame_header.blending_info.mode != BlendMode::Replace
            || self.frame_header.custom_size_or_origin;
        for blending_info_ec in &self.frame_header.extra_channel_blending_info {
            if blending_info_ec.mode != BlendMode::Replace {
                has_blending = true;
            }
        }
        // No early Flush() if blending is enabled.
        if has_blending && !self.is_finalized {
            return Err(StatusCode::GenericError);
        }
        // No early Flush() - nothing to do - if the frame is a kSkipProgressive
        // frame.
        if self.frame_header.frame_type == FrameType::SkipProgressive && !self.is_finalized {
            return Ok(());
        }
        self.allocate_output(dec_state)?;

        let completely_decoded_ac_pass = self
            .decoded_passes_per_ac_group
            .iter()
            .copied()
            .min()
            .unwrap_or(0) as u32;
        if completely_decoded_ac_pass < self.frame_header.passes.num_passes {
            // We don't have all AC yet: force a draw of all the missing areas.
            // Mark all sections as not complete.
            for i in 0..self.decoded_passes_per_ac_group.len() {
                if (self.decoded_passes_per_ac_group[i] as u32)
                    < self.frame_header.passes.num_passes
                {
                    if let Some(p) = dec_state.render_pipeline.as_mut() {
                        p.clear_done(i);
                    }
                }
            }
            self.prepare_storage(1, self.decoded_passes_per_ac_group.len(), dec_state)?;
            let mut has_error = false;
            for g in 0..self.decoded_passes_per_ac_group.len() {
                if self.decoded_passes_per_ac_group[g] as u32 == self.frame_header.passes.num_passes
                {
                    // This group was drawn already, nothing to do.
                    continue;
                }
                let mut readers: Vec<&mut BitReader<'_>> = Vec::new();
                let dc_only = !self.decoded_ac_global;
                if self
                    .process_ac_group(
                        g,
                        &mut readers,
                        /*num_passes=*/ 0,
                        0,
                        /*force_draw=*/ true,
                        dc_only,
                        dec_state,
                    )
                    .is_err()
                {
                    has_error = true;
                }
            }
            if has_error {
                return jxl_failure!("Drawing groups failed");
            }
        }

        // undo global modular transforms and copy int pixel buffers to float ones
        let inplace = self.is_finalized;
        self.modular_frame_decoder
            .finalize_decoding(dec_state, &mut self.decoded, inplace)?;

        Ok(())
    }

    /// Returns reference id of storage location where this frame is stored
    /// as a bit flag, or 0 if not stored. Translation of `SavedAs()`.
    pub(crate) fn saved_as(header: &FrameHeader) -> i32 {
        if header.frame_type == FrameType::DcFrame {
            // bits 16, 32, 64, 128 for DC level
            16 << (header.dc_level as i32 - 1)
        } else if header.can_be_referenced() {
            // bits 1, 2, 4 and 8 for the references
            1 << header.save_as_reference
        } else {
            0
        }
    }

    /// Translation of `HasEverything()`.
    fn has_everything(&self) -> bool {
        if !self.decoded_dc_global {
            return false;
        }
        if !self.decoded_ac_global {
            return false;
        }
        for &have_dc_group in &self.decoded_dc_groups {
            if have_dc_group == 0 {
                return false;
            }
        }
        for &nb_passes in &self.decoded_passes_per_ac_group {
            if (nb_passes as u32) < self.frame_header.passes.num_passes {
                return false;
            }
        }
        true
    }

    /// Returns dependencies of this frame on reference ids as a bit mask:
    /// bits 0-3 indicate reference frame 0-3 for patches and blending, bits
    /// 4-7 indicate DC frames this frame depends on. Translation of
    /// `References()`.
    pub(crate) fn references(&self, dec_state: &PassesDecoderState) -> i32 {
        if self.is_finalized {
            return 0;
        }
        if !self.has_everything() {
            return 0;
        }

        let mut result = 0i32;

        // Blending
        if self.frame_header.frame_type == FrameType::RegularFrame
            || self.frame_header.frame_type == FrameType::SkipProgressive
        {
            let cropped = self.frame_header.custom_size_or_origin;
            if cropped || self.frame_header.blending_info.mode != BlendMode::Replace {
                result |= 1 << self.frame_header.blending_info.source;
            }
            for extra in &self.frame_header.extra_channel_blending_info {
                if cropped || extra.mode != BlendMode::Replace {
                    result |= 1 << extra.source;
                }
            }
        }

        // Patches
        if self.frame_header.flags & K_PATCHES != 0 {
            result |= dec_state
                .shared_storage
                .image_features
                .patches
                .get_references();
        }

        // DC Level
        if self.frame_header.flags & K_USE_DC_FRAME != 0 {
            // Reads from the next dc level
            let dc_level = self.frame_header.dc_level as i32 + 1;
            // bits 16, 32, 64, 128 for DC level
            result |= 16 << (dc_level - 1);
        }

        result
    }

    /// Runs final operations once a frame data is decoded. Must be called
    /// exactly once per frame, after all calls to ProcessSections.
    /// Translation of `FinalizeFrame()`.
    pub(crate) fn finalize_frame(&mut self, dec_state: &mut PassesDecoderState) -> Status {
        if self.is_finalized {
            return jxl_failure!("FinalizeFrame called multiple times");
        }
        self.is_finalized = true;
        if !self.finalized_dc {
            // We don't have all of DC, and render pipeline is not created yet, so we
            // can not call Flush() yet.
            return jxl_failure!("FinalizeFrame called before DC was fully decoded");
        }

        self.flush(dec_state)?;

        if self.frame_header.can_be_referenced() {
            let info = &mut dec_state.shared_storage.reference_frames
                [self.frame_header.save_as_reference as usize];
            info.frame = std::mem::replace(
                &mut dec_state.frame_storage_for_referencing,
                ImageBundle::new(None),
            );
            info.ib_is_in_xyb = self.frame_header.save_before_color_transform;
        }
        Ok(())
    }

    /// Sets the buffer to which uint8 sRGB pixels will be decoded. This is
    /// not supported for all images. If it succeeds, HasRGBBuffer() will
    /// return true. Translation of `MaybeSetRGB8OutputBuffer()`: returns the
    /// buffer back if it was not taken.
    pub(crate) fn maybe_set_rgb8_output_buffer(
        &self,
        rgb_output: Vec<u8>,
        stride: usize,
        is_rgba: bool,
        undo_orientation: bool,
        dec_state: &mut PassesDecoderState,
    ) -> Option<Vec<u8>> {
        if !self.can_do_low_memory_path(undo_orientation) || dec_state.unpremul_alpha {
            return Some(rgb_output);
        }
        dec_state.rgb_output = Some(rgb_output);
        dec_state.rgb_output_is_rgba = is_rgba;
        dec_state.rgb_stride = stride;
        // (JXL_HIGH_PRECISION: no fast XYB to sRGB8 conversion.)
        None
    }

    /// Translation of `MaybeSetUnpremultiplyAlpha()`.
    pub(crate) fn maybe_set_unpremultiply_alpha(
        &self,
        unpremul_alpha: bool,
        dec_state: &mut PassesDecoderState,
    ) {
        let alpha = self
            .decoded
            .metadata()
            .and_then(|m| m.find(super::image_metadata::ExtraChannel::Alpha));
        if let Some(alpha) = alpha {
            if alpha.alpha_associated && unpremul_alpha {
                dec_state.unpremul_alpha = true;
            }
        }
    }

    /// Returns true if the rgb output buffer passed by
    /// MaybeSetRGB8OutputBuffer has been/will be populated by Flush() /
    /// FinalizeFrame(). Translation of `HasRGBBuffer()`.
    pub(crate) fn has_rgb_buffer(dec_state: &PassesDecoderState) -> bool {
        dec_state.rgb_output.is_some()
    }

    /// If the image has default exif orientation (or has an orientation but
    /// should not be undone) and no blending, the current frame cannot be
    /// referenced by future frames, there are no spot colors to be rendered,
    /// and alpha is not premultiplied, then low memory options can be used
    /// (uint8 output buffer or float pixel callback). Translation of
    /// `CanDoLowMemoryPath()`.
    fn can_do_low_memory_path(&self, undo_orientation: bool) -> bool {
        !(undo_orientation
            && self.decoded.metadata().is_some_and(|m| {
                m.get_orientation() != super::image_metadata::Orientation::Identity
            }))
    }
}

/// Takes the bit readers of the given sections out (to hand them to a group
/// decoder together).
fn take_readers<'a>(
    sections: &mut [SectionInfo<'a>],
    idxs: &[usize],
) -> Vec<(usize, BitReader<'a>)> {
    idxs.iter()
        .map(|&i| {
            (
                i,
                std::mem::replace(&mut sections[i].br, BitReader::new(&[])),
            )
        })
        .collect()
}

/// Puts back the bit readers taken by `take_readers`.
fn put_back_readers<'a>(sections: &mut [SectionInfo<'a>], readers: Vec<(usize, BitReader<'a>)>) {
    for (i, br) in readers {
        sections[i].br = br;
    }
}
