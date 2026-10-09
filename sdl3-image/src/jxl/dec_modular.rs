// Rust translation of lib/jxl/dec_modular.h and lib/jxl/dec_modular.cc from
// libjxl (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Decoding of the modular parts of a frame.

use super::ac_strategy::AcStrategy;
use super::base::{
    ceil_log2_nonzero_u64, div_ceil, jxl_failure, FrameDimensions, Status, StatusCode, K_BITS_PER_BYTE,
    K_GROUP_DIM_IN_BLOCKS,
};
use super::chroma_from_luma::K_COLOR_TILE_DIM_IN_BLOCKS;
use super::compressed_dc::dequant_dc;
use super::dec_ans::{decode_histograms, AnsCode};
use super::dec_bit_reader::BitReader;
use super::dec_cache::PassesDecoderState;
use super::epf::compute_sigma;
use super::fields::f16_coder_read;
use super::frame_header::{ColorTransform, FrameEncoding, FrameHeader, K_NOISE};
use super::image::{copy_image_to_rect, zero_fill_image, Rect};
use super::image_bundle::ImageBundle;
use super::loop_filter::K_EPF_SHARP_ENTRIES;
use super::modular::encoding::dec_ma::{decode_tree, Tree};
use super::modular::encoding::encoding::{modular_generic_decompress, GroupHeader};
use super::modular::modular_image::{Channel, Image, PixelType};
use super::modular::options::ModularOptions;
use super::modular::transform::transform::{Transform, TransformId};
use super::quant_weights::{quant_table, QuantEncoding};
use super::quantizer::Quantizer;
use super::render_pipeline::RenderPipelineInput;

/// Translation of `ModularStreamId::Kind`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ModularStreamKind {
    GlobalData,
    VarDctDc,
    ModularDc,
    AcMetadata,
    QuantTable,
    ModularAc,
}

/// Translation of `ModularStreamId`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ModularStreamId {
    pub kind: ModularStreamKind,
    pub quant_table_id: usize,
    pub group_id: usize, // DC or AC group id.
    pub pass_id: usize,  // Only for kModularAC.
}

impl ModularStreamId {
    pub(crate) fn id(&self, frame_dim: &FrameDimensions) -> usize {
        match self.kind {
            ModularStreamKind::GlobalData => 0,
            ModularStreamKind::VarDctDc => 1 + self.group_id,
            ModularStreamKind::ModularDc => 1 + frame_dim.num_dc_groups + self.group_id,
            ModularStreamKind::AcMetadata => 1 + 2 * frame_dim.num_dc_groups + self.group_id,
            ModularStreamKind::QuantTable => 1 + 3 * frame_dim.num_dc_groups + self.quant_table_id,
            ModularStreamKind::ModularAc => {
                1 + 3 * frame_dim.num_dc_groups
                    + quant_table::K_NUM
                    + frame_dim.num_groups * self.pass_id
                    + self.group_id
            }
        }
    }
    pub(crate) fn global() -> Self {
        Self::make(ModularStreamKind::GlobalData, 0, 0, 0)
    }
    pub(crate) fn var_dct_dc(group_id: usize) -> Self {
        Self::make(ModularStreamKind::VarDctDc, 0, group_id, 0)
    }
    pub(crate) fn modular_dc(group_id: usize) -> Self {
        Self::make(ModularStreamKind::ModularDc, 0, group_id, 0)
    }
    pub(crate) fn ac_metadata(group_id: usize) -> Self {
        Self::make(ModularStreamKind::AcMetadata, 0, group_id, 0)
    }
    pub(crate) fn quant_table(quant_table_id: usize) -> Self {
        debug_assert!(quant_table_id < quant_table::K_NUM);
        Self::make(ModularStreamKind::QuantTable, quant_table_id, 0, 0)
    }
    pub(crate) fn modular_ac(group_id: usize, pass_id: usize) -> Self {
        Self::make(ModularStreamKind::ModularAc, 0, group_id, pass_id)
    }
    fn make(kind: ModularStreamKind, quant_table_id: usize, group_id: usize, pass_id: usize) -> Self {
        ModularStreamId {
            kind,
            quant_table_id,
            group_id,
            pass_id,
        }
    }
}

/// Translation of `MultiplySum()` (one lane).
fn multiply_sum(xsize: usize, row_in: &[PixelType], row_in_y: &[PixelType], factor: f32, row_out: &mut [f32]) {
    for x in 0..xsize {
        let inp = row_in[x].wrapping_add(row_in_y[x]);
        row_out[x] = inp as f32 * factor;
    }
}

/// Translation of `SingleFromSingle()` (one lane).
fn single_from_single(xsize: usize, row_in: &[PixelType], factor: f32, row_out: &mut [f32]) {
    for x in 0..xsize {
        row_out[x] = row_in[x] as f32 * factor;
    }
}

// Slow conversion using double precision multiplication, only
// needed when the bit depth is too high for single precision
/// Translation of `SingleFromSingleAccurate()`.
fn single_from_single_accurate(xsize: usize, row_in: &[PixelType], factor: f64, row_out: &mut [f32]) {
    for x in 0..xsize {
        row_out[x] = (row_in[x] as f64 * factor) as f32;
    }
}

// convert custom [bits]-bit float (with [exp_bits] exponent bits) stored as int
// back to binary32 float
/// Translation of `int_to_float()`.
fn int_to_float(row_in: &[PixelType], row_out: &mut [f32], xsize: usize, bits: i32, exp_bits: i32) -> Status {
    if bits == 32 {
        // JXL_ASSERT(exp_bits == 8)
        if exp_bits != 8 {
            return Err(StatusCode::GenericError);
        }
        for x in 0..xsize {
            row_out[x] = f32::from_bits(row_in[x] as u32);
        }
        return Ok(());
    }
    let exp_bias = (1i32 << (exp_bits - 1)) - 1;
    let sign_shift = bits - 1;
    let mant_bits = bits - exp_bits - 1;
    let mant_shift = 23 - mant_bits;
    for x in 0..xsize {
        let mut f = row_in[x] as u32;
        let signbit = (f >> sign_shift) as i32;
        f &= (1u32 << sign_shift).wrapping_sub(1);
        if f == 0 {
            row_out[x] = if signbit != 0 { -0.0f32 } else { 0.0f32 };
            continue;
        }
        let mut exp = (f >> mant_bits) as i32;
        let mut mantissa = (f & ((1u32 << mant_bits).wrapping_sub(1))) as i32;
        mantissa = mantissa.wrapping_shl(mant_shift as u32);
        // Try to normalize only if there is space for maneuver.
        if exp == 0 && exp_bits < 8 {
            // subnormal number
            while (mantissa & 0x800000) == 0 {
                mantissa = mantissa.wrapping_shl(1);
                exp -= 1;
            }
            exp += 1;
            // remove leading 1 because it is implicit now
            mantissa &= 0x7fffff;
        }
        exp -= exp_bias;
        // broke up the arbitrary float into its parts, now reassemble into
        // binary32
        exp += 127;
        // JXL_ASSERT(exp >= 0)
        if exp < 0 {
            return Err(StatusCode::GenericError);
        }
        f = if signbit != 0 { 0x80000000 } else { 0 };
        f |= (exp as u32) << 23;
        f |= mantissa as u32;
        row_out[x] = f32::from_bits(f);
    }
    Ok(())
}

/// Translation of `ModularFrameDecoder`.
#[derive(Debug)]
pub(crate) struct ModularFrameDecoder {
    full_image: Image,
    global_transform: Vec<Transform>,
    frame_dim: FrameDimensions,
    do_color: bool,
    have_something: bool,
    use_full_image: bool,
    all_same_shift: bool,
    tree: Tree,
    code: AnsCode,
    context_map: Vec<u8>,
    global_header: GroupHeader,
}

impl Default for ModularFrameDecoder {
    fn default() -> Self {
        ModularFrameDecoder {
            full_image: Image::default(),
            global_transform: Vec::new(),
            frame_dim: FrameDimensions::default(),
            do_color: false,
            have_something: false,
            use_full_image: true,
            all_same_shift: false,
            tree: Tree::new(),
            code: AnsCode::default(),
            context_map: Vec::new(),
            global_header: GroupHeader::new(),
        }
    }
}

impl ModularFrameDecoder {
    pub(crate) fn init(&mut self, frame_dim: &FrameDimensions) {
        self.frame_dim = frame_dim.clone();
    }

    #[allow(dead_code)]
    pub(crate) fn have_dc(&self) -> bool {
        self.have_something
    }

    pub(crate) fn uses_full_image(&self) -> bool {
        self.use_full_image
    }

    /// Translation of `DecodeGlobalInfo()`.
    pub(crate) fn decode_global_info(
        &mut self,
        reader: &mut BitReader<'_>,
        frame_header: &FrameHeader,
        allow_truncated_group: bool,
    ) -> Status {
        let decode_color = frame_header.encoding == FrameEncoding::Modular;
        let Some(md) = frame_header.nonserialized_metadata.as_ref() else {
            return Err(StatusCode::GenericError);
        };
        let metadata = &md.m;
        let is_gray = metadata.color_encoding.is_gray();
        let mut nb_chans: usize = 3;
        if is_gray && frame_header.color_transform == ColorTransform::None {
            nb_chans = 1;
        }
        self.do_color = decode_color;
        let nb_extra = metadata.extra_channel_info.len();
        let has_tree = reader.read_bits(1) != 0;
        if !allow_truncated_group || reader.total_bits_consumed() < reader.total_bytes() * K_BITS_PER_BYTE {
            if has_tree {
                let tree_size_limit = (1usize << 22).min(
                    1024 + self
                        .frame_dim
                        .xsize
                        .wrapping_mul(self.frame_dim.ysize)
                        .wrapping_mul(nb_chans + nb_extra)
                        / 16,
                );
                decode_tree(reader, &mut self.tree, tree_size_limit)?;
                decode_histograms(reader, (self.tree.len() + 1) / 2, &mut self.code, &mut self.context_map, false)?;
            }
        }
        if !self.do_color {
            nb_chans = 0;
        }

        let fp = metadata.bit_depth.floating_point_sample;

        // bits_per_sample is just metadata for XYB images.
        if metadata.bit_depth.bits_per_sample >= 32
            && self.do_color
            && frame_header.color_transform != ColorTransform::Xyb
        {
            if metadata.bit_depth.bits_per_sample == 32 && !fp {
                return jxl_failure!("uint32_t not supported in dec_modular");
            } else if metadata.bit_depth.bits_per_sample > 32 {
                return jxl_failure!("bits_per_sample > 32 not supported");
            }
        }

        let mut gi = Image::new(
            self.frame_dim.xsize,
            self.frame_dim.ysize,
            metadata.bit_depth.bits_per_sample as i32,
            (nb_chans + nb_extra) as i32,
        )?;

        self.all_same_shift = true;
        if frame_header.color_transform == ColorTransform::YCbCr {
            for c in 0..nb_chans {
                gi.channel[c].hshift = frame_header.chroma_subsampling.h_shift(c) as i32;
                gi.channel[c].vshift = frame_header.chroma_subsampling.v_shift(c) as i32;
                let xsize_shifted = div_ceil(self.frame_dim.xsize, 1 << gi.channel[c].hshift);
                let ysize_shifted = div_ceil(self.frame_dim.ysize, 1 << gi.channel[c].vshift);
                gi.channel[c].shrink_to(xsize_shifted, ysize_shifted)?;
                if gi.channel[c].hshift != gi.channel[0].hshift || gi.channel[c].vshift != gi.channel[0].vshift {
                    self.all_same_shift = false;
                }
            }
        }

        for ec in 0..nb_extra {
            let c = nb_chans + ec;
            let ecups = frame_header.extra_channel_upsampling[ec] as usize;
            gi.channel[c].shrink_to(
                div_ceil(self.frame_dim.xsize_upsampled, ecups),
                div_ceil(self.frame_dim.ysize_upsampled, ecups),
            )?;
            let s = ceil_log2_nonzero_u64(ecups as u64) as i32
                - ceil_log2_nonzero_u64(frame_header.upsampling as u64) as i32;
            gi.channel[c].hshift = s;
            gi.channel[c].vshift = s;
            if gi.channel[c].hshift != gi.channel[0].hshift || gi.channel[c].vshift != gi.channel[0].vshift {
                self.all_same_shift = false;
            }
        }

        let options = ModularOptions {
            max_chan_size: self.frame_dim.group_dim,
            group_dim: self.frame_dim.group_dim,
        };
        let dec_status = modular_generic_decompress(
            reader,
            &mut gi,
            Some(&mut self.global_header),
            ModularStreamId::global().id(&self.frame_dim),
            &options,
            /*undo_transforms=*/ false,
            Some(&self.tree),
            Some(&self.code),
            Some(&self.context_map),
            allow_truncated_group,
        );
        if !allow_truncated_group {
            dec_status?;
        }
        if dec_status == Err(StatusCode::GenericError) {
            return jxl_failure!("Failed to decode global modular info");
        }

        // TODO(eustas): are we sure this can be done after partial decode?
        self.have_something = false;
        for c in 0..gi.channel.len() {
            let gic = &gi.channel[c];
            if c >= gi.nb_meta_channels && gic.w <= self.frame_dim.group_dim && gic.h <= self.frame_dim.group_dim {
                self.have_something = true;
            }
        }
        // move global transforms to groups if possible
        if !self.have_something
            && self.all_same_shift
            && gi.transform.len() == 1
            && gi.transform[0].id == TransformId::Rct
        {
            self.global_transform = std::mem::take(&mut gi.transform);
            // TODO(jon): also move no-delta-palette out (trickier though)
        }
        self.full_image = gi;
        dec_status
    }

    /// Translation of `MaybeDropFullImage()`.
    pub(crate) fn maybe_drop_full_image(&mut self) {
        if self.full_image.transform.is_empty() && !self.have_something && self.all_same_shift {
            self.use_full_image = false;
            for ch in self.full_image.channel.iter_mut() {
                // keep metadata on channels around, but dealloc their planes
                ch.plane = super::image::Plane::empty();
            }
        }
    }

    /// Translation of `DecodeGroup()`. `dec_state`/`render_pipeline_input`
    /// are `None` for DC groups; `decoded` is the frame's output (for the
    /// render pipeline).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decode_group(
        &mut self,
        rect: &Rect,
        reader: Option<&mut BitReader<'_>>,
        min_shift: i32,
        max_shift: i32,
        stream: &ModularStreamId,
        zerofill: bool,
        dec_state: Option<&mut PassesDecoderState>,
        render_pipeline_input: Option<&RenderPipelineInput>,
        allow_truncated: bool,
        should_run_pipeline: Option<&mut bool>,
    ) -> Status {
        debug_assert!(stream.kind == ModularStreamKind::ModularDc || stream.kind == ModularStreamKind::ModularAc);
        let xsize = rect.xsize();
        let ysize = rect.ysize();
        let mut gi = Image::new(xsize, ysize, self.full_image.bitdepth, 0)?;
        // start at the first bigger-than-groupsize non-metachannel
        let mut c = self.full_image.nb_meta_channels;
        while c < self.full_image.channel.len() {
            let fc = &self.full_image.channel[c];
            if fc.w > self.frame_dim.group_dim || fc.h > self.frame_dim.group_dim {
                break;
            }
            c += 1;
        }
        let beginc = c;
        while c < self.full_image.channel.len() {
            let use_full_image = self.use_full_image;
            let fc = &mut self.full_image.channel[c];
            let shift = fc.hshift.min(fc.vshift);
            if shift > max_shift || shift < min_shift {
                c += 1;
                continue;
            }
            let r = Rect::new_clamped(
                rect.x0() >> fc.hshift,
                rect.y0() >> fc.vshift,
                rect.xsize() >> fc.hshift,
                rect.ysize() >> fc.vshift,
                fc.w,
                fc.h,
            );
            if r.xsize() == 0 || r.ysize() == 0 {
                c += 1;
                continue;
            }
            if zerofill && use_full_image {
                for y in 0..r.ysize() {
                    let n = r.xsize();
                    r.row(&mut fc.plane, y)[..n].fill(0);
                }
            } else {
                let mut gc = Channel::new(r.xsize(), r.ysize(), 0, 0)?;
                if zerofill {
                    zero_fill_image(&mut gc.plane);
                }
                gc.hshift = fc.hshift;
                gc.vshift = fc.vshift;
                gi.channel.push(gc);
            }
            c += 1;
        }
        if zerofill && self.use_full_image {
            return Ok(());
        }
        // Return early if there's nothing to decode. Otherwise there might be
        // problems later (in ModularImageToDecodedRect).
        if gi.channel.is_empty() {
            if let (Some(dec_state), Some(should_run_pipeline)) = (dec_state.as_deref(), should_run_pipeline) {
                let frame_header = &dec_state.shared_storage.frame_header;
                let num_ec = frame_header
                    .nonserialized_metadata
                    .as_ref()
                    .map_or(0, |m| m.m.num_extra_channels);
                if self.do_color || num_ec > 0 {
                    // Signal to FrameDecoder that we do not have some of the required input
                    // for the render pipeline.
                    *should_run_pipeline = false;
                }
            }
            return Ok(());
        }
        let options = ModularOptions::default();
        if !zerofill {
            let Some(reader) = reader else {
                return Err(StatusCode::GenericError);
            };
            let status = modular_generic_decompress(
                reader,
                &mut gi,
                /*header=*/ None,
                stream.id(&self.frame_dim),
                &options,
                /*undo_transforms=*/ true,
                Some(&self.tree),
                Some(&self.code),
                Some(&self.context_map),
                allow_truncated,
            );
            if !allow_truncated {
                status?;
            }
            if status == Err(StatusCode::GenericError) {
                return status;
            }
        }
        // Undo global transforms that have been pushed to the group level
        if !self.use_full_image {
            let (Some(dec_state), Some(render_pipeline_input)) = (dec_state, render_pipeline_input) else {
                // JXL_ASSERT(render_pipeline_input)
                return Err(StatusCode::GenericError);
            };
            for t in &self.global_transform {
                t.inverse(&mut gi, &self.global_header.wp_header)?;
            }
            let (w, h) = (gi.w, gi.h);
            self.modular_image_to_decoded_rect(&mut gi, dec_state, render_pipeline_input, Rect::new(0, 0, w, h))?;
            return Ok(());
        }
        let mut gic = 0usize;
        for c in beginc..self.full_image.channel.len() {
            let fc = &mut self.full_image.channel[c];
            let shift = fc.hshift.min(fc.vshift);
            if shift > max_shift || shift < min_shift {
                continue;
            }
            let r = Rect::new_clamped(
                rect.x0() >> fc.hshift,
                rect.y0() >> fc.vshift,
                rect.xsize() >> fc.hshift,
                rect.ysize() >> fc.vshift,
                fc.w,
                fc.h,
            );
            if r.xsize() == 0 || r.ysize() == 0 {
                continue;
            }
            copy_image_to_rect(
                /*rect_from=*/ &Rect::new(0, 0, r.xsize(), r.ysize()),
                /*from=*/ &gi.channel[gic].plane,
                /*rect_to=*/ &r,
                /*to=*/ &mut fc.plane,
            );
            gic += 1;
        }
        Ok(())
    }

    /// Decodes a VarDCT DC group (`group_id`) from the given `reader`.
    /// Translation of `DecodeVarDCTDC()`.
    pub(crate) fn decode_var_dct_dc(
        &mut self,
        group_id: usize,
        reader: &mut BitReader<'_>,
        dec_state: &mut PassesDecoderState,
    ) -> Status {
        let r = dec_state.shared_storage.dc_group_rect(group_id);
        // TODO(eustas): investigate if we could reduce the impact of
        //               EvalRationalPolynomial; generally speaking, the limit is
        //               2**(128/(3*magic)), where 128 comes from IEEE 754 exponent,
        //               3 comes from XybToRgb that cubes the values, and "magic" is
        //               the sum of all other contributions. 2**18 is known to lead
        //               to NaN on input found by fuzzing (see commit message).
        let mut image = Image::new(r.xsize(), r.ysize(), self.full_image.bitdepth, 3)?;
        let stream_id = ModularStreamId::var_dct_dc(group_id).id(&self.frame_dim);
        reader.refill();
        let extra_precision = reader.read_fixed_bits::<2>() as usize;
        let mul = 1.0f32 / (1 << extra_precision) as f32;
        let options = ModularOptions::default();
        let cs = dec_state.shared_storage.frame_header.chroma_subsampling;
        for c in 0..3 {
            let ch = &mut image.channel[if c < 2 { c ^ 1 } else { c }];
            ch.w >>= cs.h_shift(c);
            ch.h >>= cs.v_shift(c);
            ch.shrink()?;
        }
        if modular_generic_decompress(
            reader,
            &mut image,
            /*header=*/ None,
            stream_id,
            &options,
            /*undo_transforms=*/ true,
            Some(&self.tree),
            Some(&self.code),
            Some(&self.context_map),
            false,
        )
        .is_err()
        {
            return jxl_failure!("Failed to decode modular DC group");
        }
        let shared = &mut dec_state.shared_storage;
        let mul_dc = *shared.quantizer.mul_dc();
        let dc_factors = *shared.cmap.dc_factors();
        dequant_dc(
            &r,
            &mut shared.dc_storage,
            &mut shared.quant_dc,
            &image,
            &mul_dc,
            mul,
            &dc_factors,
            &cs,
            &shared.block_ctx_map,
        );
        Ok(())
    }

    /// Decodes a VarDCT AC Metadata group (`group_id`) from the given
    /// `reader`. Translation of `DecodeAcMetadata()`.
    pub(crate) fn decode_ac_metadata(
        &mut self,
        group_id: usize,
        reader: &mut BitReader<'_>,
        dec_state: &mut PassesDecoderState,
    ) -> Status {
        let r = dec_state.shared_storage.dc_group_rect(group_id);
        let upper_bound = r.xsize() * r.ysize();
        reader.refill();
        let count = reader.read_bits(ceil_log2_nonzero_u64(upper_bound as u64)) as usize + 1;
        let stream_id = ModularStreamId::ac_metadata(group_id).id(&self.frame_dim);
        // YToX, YToB, ACS + QF, EPF
        let mut image = Image::new(r.xsize(), r.ysize(), self.full_image.bitdepth, 4)?;
        debug_assert!(K_COLOR_TILE_DIM_IN_BLOCKS == 8);
        let cr = Rect::new(r.x0() >> 3, r.y0() >> 3, (r.xsize() + 7) >> 3, (r.ysize() + 7) >> 3);
        image.channel[0] = Channel::new(cr.xsize(), cr.ysize(), 3, 3)?;
        image.channel[1] = Channel::new(cr.xsize(), cr.ysize(), 3, 3)?;
        image.channel[2] = Channel::new(count, 2, 0, 0)?;
        let options = ModularOptions::default();
        if modular_generic_decompress(
            reader,
            &mut image,
            /*header=*/ None,
            stream_id,
            &options,
            /*undo_transforms=*/ true,
            Some(&self.tree),
            Some(&self.code),
            Some(&self.context_map),
            false,
        )
        .is_err()
        {
            return jxl_failure!("Failed to decode AC metadata");
        }
        let shared = &mut dec_state.shared_storage;
        convert_plane_and_clamp(&image.channel[0], &cr, &mut shared.cmap.ytox_map);
        convert_plane_and_clamp(&image.channel[1], &cr, &mut shared.cmap.ytob_map);
        let mut num = 0usize;
        let is444 = shared.frame_header.chroma_subsampling.is_444();
        let xlim = shared.ac_strategy.xsize().min(r.x0() + r.xsize());
        let ylim = shared.ac_strategy.ysize().min(r.y0() + r.ysize());
        let mut local_used_acs: u32 = 0;
        for iy in 0..r.ysize() {
            let y = r.y0() + iy;
            let row_in_1 = image.channel[2].plane.row(0);
            let row_in_2 = image.channel[2].plane.row(1);
            let row_in_3 = image.channel[3].plane.row(iy);
            for ix in 0..r.xsize() {
                let x = r.x0() + ix;
                let sharpness = row_in_3[ix];
                if sharpness < 0 || sharpness >= K_EPF_SHARP_ENTRIES as i32 {
                    return jxl_failure!("Corrupted sharpness field");
                }
                r.row(&mut shared.epf_sharpness, iy)[ix] = sharpness as u8;
                if shared.ac_strategy.is_valid(x, y) {
                    continue;
                }

                if num >= count {
                    return jxl_failure!("Corrupted stream");
                }

                if !AcStrategy::is_raw_strategy_valid(row_in_1[num]) {
                    return jxl_failure!("Invalid AC strategy");
                }
                local_used_acs |= 1u32 << row_in_1[num];
                let acs = AcStrategy::from_raw_strategy(row_in_1[num] as u8);
                if (acs.covered_blocks_x() > 1 || acs.covered_blocks_y() > 1) && !is444 {
                    return jxl_failure!("AC strategy not compatible with chroma subsampling");
                }
                // Ensure that blocks do not overflow *AC* groups.
                let next_x_ac_block = (x / K_GROUP_DIM_IN_BLOCKS + 1) * K_GROUP_DIM_IN_BLOCKS;
                let next_y_ac_block = (y / K_GROUP_DIM_IN_BLOCKS + 1) * K_GROUP_DIM_IN_BLOCKS;
                let next_x_dct_block = x + acs.covered_blocks_x();
                let next_y_dct_block = y + acs.covered_blocks_y();
                if next_x_dct_block > next_x_ac_block || next_x_dct_block > xlim {
                    return jxl_failure!("Invalid AC strategy, x overflow");
                }
                if next_y_dct_block > next_y_ac_block || next_y_dct_block > ylim {
                    return jxl_failure!("Invalid AC strategy, y overflow");
                }
                shared.ac_strategy.set_no_bounds_check(x, y, row_in_1[num] as u8, true)?;
                r.row(&mut shared.raw_quant_field, iy)[ix] = 1 + 0.max((Quantizer::K_QUANT_MAX - 1).min(row_in_2[num]));
                num += 1;
            }
        }
        dec_state.used_acs |= local_used_acs;
        if dec_state.shared_storage.frame_header.loop_filter.epf_iters > 0 {
            compute_sigma(&r, dec_state);
        }
        Ok(())
    }

    /// Translation of `ModularImageToDecodedRect()`.
    fn modular_image_to_decoded_rect(
        &self,
        gi: &mut Image,
        dec_state: &mut PassesDecoderState,
        render_pipeline_input: &RenderPipelineInput,
        modular_rect: Rect,
    ) -> Status {
        let frame_header = &dec_state.shared_storage.frame_header;
        let Some(md) = frame_header.nonserialized_metadata.as_ref() else {
            return Err(StatusCode::GenericError);
        };
        let metadata = &md.m;
        if !gi.transform.is_empty() {
            // JXL_CHECK(gi.transform.empty())
            return Err(StatusCode::GenericError);
        }
        let Some(pipeline) = dec_state.render_pipeline.as_mut() else {
            return Err(StatusCode::GenericError);
        };
        let color_transform = frame_header.color_transform;
        let dc_quants = *dec_state.shared_storage.matrices.dc_quants();

        let mut c = 0usize;
        if self.do_color {
            let rgb_from_gray = metadata.color_encoding.is_gray() && color_transform == ColorTransform::None;
            let fp = metadata.bit_depth.floating_point_sample && color_transform != ColorTransform::Xyb;
            while c < 3 {
                let mut factor: f64 = if self.full_image.bitdepth < 32 {
                    1.0 / ((1u32 << self.full_image.bitdepth) - 1) as f64
                } else {
                    0.0
                };
                let mut c_in = c;
                if color_transform == ColorTransform::Xyb {
                    factor = dc_quants[c] as f64;
                    // XYB is encoded as YX(B-Y)
                    if c < 2 {
                        c_in = 1 - c;
                    }
                } else if rgb_from_gray {
                    c_in = 0;
                }
                if c_in >= gi.channel.len() {
                    // JXL_ASSERT(c_in < gi.channel.size())
                    return Err(StatusCode::GenericError);
                }
                let ch_in = &gi.channel[c_in];
                // TODO(eustas): could we detect it on earlier stage?
                if ch_in.w == 0 || ch_in.h == 0 {
                    return jxl_failure!("Empty image");
                }
                if !(ch_in.hshift <= 3 && ch_in.vshift <= 3) {
                    return Err(StatusCode::GenericError);
                }
                let (buf_c, r) = render_pipeline_input.get_buffer(c)?;
                let mr = Rect::new(
                    modular_rect.x0() >> ch_in.hshift,
                    modular_rect.y0() >> ch_in.vshift,
                    div_ceil(modular_rect.xsize(), 1 << ch_in.hshift),
                    div_ceil(modular_rect.ysize(), 1 << ch_in.vshift),
                )
                .crop_plane(&ch_in.plane);
                let xsize_shifted = r.xsize();
                let ysize_shifted = r.ysize();
                if r.ysize() != mr.ysize() || r.xsize() != mr.xsize() {
                    return jxl_failure!("Dimension mismatch");
                }
                if color_transform == ColorTransform::Xyb && c == 2 {
                    debug_assert!(!fp);
                    for y in 0..ysize_shifted {
                        let row_in = mr.const_row(&ch_in.plane, y);
                        let row_in_y = mr.const_row(&gi.channel[0].plane, y);
                        let row_out = r.row(pipeline.buffer_mut(buf_c), y);
                        multiply_sum(xsize_shifted, row_in, row_in_y, factor as f32, row_out);
                    }
                } else if fp {
                    let bits = metadata.bit_depth.bits_per_sample as i32;
                    let exp_bits = metadata.bit_depth.exponent_bits_per_sample as i32;
                    for y in 0..ysize_shifted {
                        let row_in = mr.const_row(&ch_in.plane, y);
                        if rgb_from_gray {
                            for cc in 0..3 {
                                let (b, rr) = render_pipeline_input.get_buffer(cc)?;
                                let row_out = rr.row(pipeline.buffer_mut(b), y);
                                int_to_float(row_in, row_out, xsize_shifted, bits, exp_bits)?;
                            }
                        } else {
                            let row_out = r.row(pipeline.buffer_mut(buf_c), y);
                            int_to_float(row_in, row_out, xsize_shifted, bits, exp_bits)?;
                        }
                    }
                } else {
                    for y in 0..ysize_shifted {
                        let row_in = mr.const_row(&ch_in.plane, y);
                        if rgb_from_gray {
                            for cc in 0..3 {
                                let (b, rr) = render_pipeline_input.get_buffer(cc)?;
                                let row_out = rr.row(pipeline.buffer_mut(b), y);
                                if self.full_image.bitdepth < 23 {
                                    // (RgbFromSingle)
                                    single_from_single(xsize_shifted, row_in, factor as f32, row_out);
                                } else {
                                    single_from_single_accurate(xsize_shifted, row_in, factor, row_out);
                                }
                            }
                        } else {
                            let row_out = r.row(pipeline.buffer_mut(buf_c), y);
                            if self.full_image.bitdepth < 23 {
                                single_from_single(xsize_shifted, row_in, factor as f32, row_out);
                            } else {
                                single_from_single_accurate(xsize_shifted, row_in, factor, row_out);
                            }
                        }
                    }
                }
                if rgb_from_gray {
                    break;
                }
                c += 1;
            }
            if rgb_from_gray {
                c = 1;
            }
        }
        let num_extra_channels = metadata.num_extra_channels as usize;
        for ec in 0..num_extra_channels {
            let eci = &metadata.extra_channel_info[ec];
            let bits = eci.bit_depth.bits_per_sample as i32;
            let exp_bits = eci.bit_depth.exponent_bits_per_sample as i32;
            let fp = eci.bit_depth.floating_point_sample;
            if !(fp || bits < 32) {
                // JXL_ASSERT(fp || bits < 32)
                return Err(StatusCode::GenericError);
            }
            let factor: f64 = if fp { 0.0 } else { 1.0 / ((1u32 << bits) - 1) as f64 };
            if c >= gi.channel.len() {
                // JXL_ASSERT(c < gi.channel.size())
                return Err(StatusCode::GenericError);
            }
            let ch_in = &gi.channel[c];
            let (buf, r) = render_pipeline_input.get_buffer(3 + ec)?;
            let mr = Rect::new(
                modular_rect.x0() >> ch_in.hshift,
                modular_rect.y0() >> ch_in.vshift,
                div_ceil(modular_rect.xsize(), 1 << ch_in.hshift),
                div_ceil(modular_rect.ysize(), 1 << ch_in.vshift),
            )
            .crop_plane(&ch_in.plane);
            if r.ysize() != mr.ysize() || r.xsize() != mr.xsize() {
                return jxl_failure!("Dimension mismatch");
            }
            for y in 0..r.ysize() {
                let row_out = r.row(pipeline.buffer_mut(buf), y);
                let row_in = mr.const_row(&ch_in.plane, y);
                if fp {
                    int_to_float(row_in, row_out, r.xsize(), bits, exp_bits)?;
                } else if self.full_image.bitdepth < 23 {
                    single_from_single(r.xsize(), row_in, factor as f32, row_out);
                } else {
                    single_from_single_accurate(r.xsize(), row_in, factor, row_out);
                }
            }
            c += 1;
        }
        Ok(())
    }

    /// If inplace is true, this can only be called once. If it is false, it
    /// can be called multiple times (e.g. for progressive steps).
    /// Translation of `FinalizeDecoding()`.
    pub(crate) fn finalize_decoding(
        &mut self,
        dec_state: &mut PassesDecoderState,
        decoded: &mut ImageBundle,
        inplace: bool,
    ) -> Status {
        if !self.use_full_image {
            return Ok(());
        }
        let mut gi = if inplace {
            let gi = std::mem::take(&mut self.full_image);
            // (A moved-from Image keeps its scalar members: the bit depth
            // ModularImageToDecodedRect() reads from full_image below.)
            self.full_image.w = gi.w;
            self.full_image.h = gi.h;
            self.full_image.bitdepth = gi.bitdepth;
            self.full_image.nb_meta_channels = gi.nb_meta_channels;
            self.full_image.error = gi.error;
            gi
        } else {
            self.full_image.clone_image()?
        };

        // Undo the global transforms
        gi.undo_transforms(&self.global_header.wp_header);
        debug_assert!(self.global_transform.is_empty());
        if gi.error {
            return jxl_failure!("Undoing transforms failed");
        }

        let num_groups = dec_state.shared_storage.frame_dim.num_groups;
        let Some(pipeline) = dec_state.render_pipeline.as_mut() else {
            return Err(StatusCode::GenericError);
        };
        for i in 0..num_groups {
            pipeline.clear_done(i);
        }
        let frame_header = &dec_state.shared_storage.frame_header;
        let use_group_ids = frame_header.encoding == FrameEncoding::VarDct || (frame_header.flags & K_NOISE) != 0;
        pipeline.prepare_for_threads(1, use_group_ids)?;
        for group in 0..num_groups {
            let input = match dec_state.render_pipeline.as_ref() {
                Some(p) => p.get_input_buffers(group, 0),
                None => return Err(StatusCode::GenericError),
            };
            let group_rect = dec_state.shared_storage.group_rect(group);
            if self.modular_image_to_decoded_rect(&mut gi, dec_state, &input, group_rect).is_err() {
                return jxl_failure!("Error producing input to render pipeline");
            }
            dec_state.input_done(&input, decoded);
        }
        Ok(())
    }

    /// Decodes a RAW quant table from `br` into the given `encoding`, of size
    /// `required_size_x x required_size_y`. If `modular_frame_decoder` is
    /// passed, its global tree is used, otherwise no global tree is used.
    /// Translation of `DecodeQuantTable()`.
    pub(crate) fn decode_quant_table(
        required_size_x: usize,
        required_size_y: usize,
        br: &mut BitReader<'_>,
        encoding: &mut QuantEncoding,
        idx: usize,
        modular_frame_decoder: Option<&ModularFrameDecoder>,
    ) -> Status {
        const K_ALMOST_ZERO: f32 = 1e-8f32;
        f16_coder_read(br, &mut encoding.qraw_qtable_den)?;
        if encoding.qraw_qtable_den < K_ALMOST_ZERO {
            // qtable[] values are already checked for <= 0 so the denominator may not
            // be negative.
            return jxl_failure!("Invalid qtable_den: value too small");
        }
        let mut image = Image::new(required_size_x, required_size_y, 8, 3)?;
        let options = ModularOptions::default();
        match modular_frame_decoder {
            Some(mfd) => {
                modular_generic_decompress(
                    br,
                    &mut image,
                    /*header=*/ None,
                    ModularStreamId::quant_table(idx).id(&mfd.frame_dim),
                    &options,
                    /*undo_transforms=*/ true,
                    Some(&mfd.tree),
                    Some(&mfd.code),
                    Some(&mfd.context_map),
                    false,
                )?;
            }
            None => {
                modular_generic_decompress(
                    br,
                    &mut image,
                    /*header=*/ None,
                    0,
                    &options,
                    /*undo_transforms=*/ true,
                    None,
                    None,
                    None,
                    false,
                )?;
            }
        }
        let n = required_size_x * required_size_y * 3;
        let qtable = encoding.qraw_qtable.get_or_insert_with(Vec::new);
        if qtable.try_reserve(n.saturating_sub(qtable.len())).is_err() {
            return Err(StatusCode::GenericError);
        }
        qtable.resize(n, 0);
        for c in 0..3 {
            for y in 0..required_size_y {
                let row = image.channel[c].row(y);
                for x in 0..required_size_x {
                    qtable[c * required_size_x * required_size_y + y * required_size_x + x] = row[x];
                    if row[x] <= 0 {
                        return jxl_failure!("Invalid raw quantization table");
                    }
                }
            }
        }
        Ok(())
    }
}

/// Translation of `ConvertPlaneAndClamp()` (from image_ops.h):
/// `to:rect_to` = clamp(`from`) as int8.
fn convert_plane_and_clamp(from: &Channel, rect_to: &Rect, to: &mut super::image::ImageSB) {
    let rect_from = Rect::from_plane(&from.plane);
    debug_assert!(rect_from.xsize() == rect_to.xsize() && rect_from.ysize() == rect_to.ysize());
    for y in 0..rect_to.ysize() {
        let row_from = rect_from.const_row(&from.plane, y);
        let row_to = rect_to.row(to, y);
        for x in 0..rect_to.xsize() {
            row_to[x] = row_from[x].clamp(i8::MIN as i32, i8::MAX as i32) as i8;
        }
    }
}
