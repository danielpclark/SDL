// Rust translation of lib/jxl/dec_group.h and lib/jxl/dec_group.cc from
// libjxl (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches), for Highway's
// scalar target; with AdjustQuantBias() from lib/jxl/quantizer-inl.h and
// PredictFromTopAndLeft() from lib/jxl/entropy_coder.h.
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Decoding of the VarDCT groups.
//!
//! (JPEG reconstruction is not translated: the decoder never holds JPEG
//! data, so `decoded->IsJPEG()` is always false. `GetBlockFromEncoder` and
//! `DecodeGroupForRoundtrip()` serve the encoder only.)

use super::ac_context::{zero_density_context, BlockCtxMap};
use super::ac_strategy::{AcStrategy, CoeffOrderT};
use super::base::{
    ceil_log2_nonzero_u64, div_ceil, Status, StatusCode, K_BLOCK_DIM, K_DCT_BLOCK_SIZE,
};
use super::chroma_from_luma::K_COLOR_TILE_DIM_IN_BLOCKS;
use super::coeff_order::{coeff_order_offset, K_STRATEGY_ORDER};
use super::dec_ans::AnsSymbolReader;
use super::dec_bit_reader::BitReader;
use super::dec_cache::{AcImage, GroupDecCache, PassesDecoderState};
use super::dec_transforms::{lowest_frequencies_from_dc, transform_to_pixels};
use super::image::{Image3, ImageB, ImageI, Rect};
use super::render_pipeline::RenderPipelineInput;

// ---- entropy_coder.h ----

/// Translation of `PredictFromTopAndLeft()`.
#[inline]
fn predict_from_top_and_left(
    row_top: Option<&[i32]>,
    row: &[i32],
    x: usize,
    default_val: i32,
) -> i32 {
    if x == 0 {
        return match row_top {
            None => default_val,
            Some(t) => t[x],
        };
    }
    match row_top {
        None => row[x - 1],
        Some(t) => (t[x].wrapping_add(row[x - 1]).wrapping_add(1)) / 2,
    }
}

// ---- quantizer-inl.h ----

/// Translation of `AdjustQuantBias()` (one lane).
#[inline(always)]
fn adjust_quant_bias(c: usize, quant_i: i32, biases: &[f32; 4]) -> f32 {
    let quant = quant_i as f32;

    // Compare |quant|, keep sign bit for negating result.
    let k_sign = i32::MIN as u32;
    let sign = quant.to_bits() & k_sign; // TODO(janwas): = abs ^ orig
    let abs_quant = f32::from_bits(!k_sign & quant.to_bits());

    // If |x| is 1, kZeroBias creates a different bias for each channel.
    // We're implementing the following:
    // if (quant == 0) return 0;
    // if (quant == 1) return biases[c];
    // if (quant == -1) return -biases[c];
    // return quant - biases[3] / quant;

    // Integer comparison is not helpful because Clang incurs bypass penalties
    // from unnecessarily mixing integer and float.
    let is_01 = abs_quant < 1.125f32;
    let not_0 = abs_quant > 0.0f32;

    // Bitwise logic is faster than quant * biases[c].
    let one_bias = if not_0 {
        f32::from_bits(biases[c].to_bits() ^ sign)
    } else {
        0.0
    };

    // About 2E-5 worse than ReciprocalNR or division.
    // (ApproximateReciprocal() on the scalar target: 1 / v, or 0 for 0.)
    let recip = if quant == 0.0 { 0.0 } else { 1.0f32 / quant };
    let bias = quant - biases[3] * recip;

    if is_01 {
        one_bias
    } else {
        bias
    }
}

// ---- dec_group.cc ----

// Controls whether DecodeGroupImpl renders to pixels or not.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DrawMode {
    // Render to pixels.
    Draw = 0,
    // Don't render to pixels.
    DontDraw = 1,
}

/// The quantized coefficients of a block's three channels: translation of
/// `ACPtr block[3]` (with the `ACType`).
enum AcPtr<'a> {
    K16([&'a mut [i16]; 3]),
    K32([&'a mut [i32]; 3]),
}

impl AcPtr<'_> {
    #[inline(always)]
    fn get(&self, c: usize, k: usize) -> i32 {
        match self {
            AcPtr::K16(b) => b[c][k] as i32,
            AcPtr::K32(b) => b[c][k],
        }
    }
}

// TODO(veluca): consider SIMDfying.
// (Transpose8x8InPlace() serves JPEG reconstruction only.)

/// Translation of `DequantLane<ac_type>()` (one lane).
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn dequant_lane(
    scaled_dequant_x: f32,
    scaled_dequant_y: f32,
    scaled_dequant_b: f32,
    dequant_matrices: &[f32],
    size: usize,
    k: usize,
    x_cc_mul: f32,
    b_cc_mul: f32,
    biases: &[f32; 4],
    qblock: &AcPtr<'_>,
    block: &mut [f32],
) {
    let x_mul = dequant_matrices[k] * scaled_dequant_x;
    let y_mul = dequant_matrices[size + k] * scaled_dequant_y;
    let b_mul = dequant_matrices[2 * size + k] * scaled_dequant_b;

    let quantized_x_int = qblock.get(0, k);
    let quantized_y_int = qblock.get(1, k);
    let quantized_b_int = qblock.get(2, k);

    let dequant_x_cc = adjust_quant_bias(0, quantized_x_int, biases) * x_mul;
    let dequant_y = adjust_quant_bias(1, quantized_y_int, biases) * y_mul;
    let dequant_b_cc = adjust_quant_bias(2, quantized_b_int, biases) * b_mul;

    let dequant_x = x_cc_mul * dequant_y + dequant_x_cc;
    let dequant_b = b_cc_mul * dequant_y + dequant_b_cc;
    block[k] = dequant_x;
    block[size + k] = dequant_y;
    block[2 * size + k] = dequant_b;
}

/// Translation of `DequantBlock<ac_type>()`.
#[allow(clippy::too_many_arguments)]
fn dequant_block(
    acs: &AcStrategy,
    inv_global_scale: f32,
    quant: i32,
    x_dm_multiplier: f32,
    b_dm_multiplier: f32,
    x_cc_mul: f32,
    b_cc_mul: f32,
    dequant_matrices: &[f32],
    size: usize,
    covered_blocks: usize,
    sbx: &[usize; 3],
    dc_row: &[&[f32]; 3],
    dc_stride: usize,
    biases: &[f32; 4],
    qblock: &AcPtr<'_>,
    block: &mut [f32],
) {
    let scaled_dequant_s = inv_global_scale / quant as f32;

    let scaled_dequant_x = scaled_dequant_s * x_dm_multiplier;
    let scaled_dequant_y = scaled_dequant_s;
    let scaled_dequant_b = scaled_dequant_s * b_dm_multiplier;

    for k in 0..covered_blocks * K_DCT_BLOCK_SIZE {
        dequant_lane(
            scaled_dequant_x,
            scaled_dequant_y,
            scaled_dequant_b,
            dequant_matrices,
            size,
            k,
            x_cc_mul,
            b_cc_mul,
            biases,
            qblock,
            block,
        );
    }
    for c in 0..3 {
        lowest_frequencies_from_dc(
            acs.strategy(),
            &dc_row[c][sbx[c]..],
            dc_stride,
            &mut block[c * size..],
        );
    }
}

/// Translation of `DecodeGroupImpl()` (with `GetBlockFromBitstream`, the
/// decoder's only `GetBlock`).
#[allow(clippy::too_many_arguments)]
fn decode_group_impl(
    get_block: &mut GetBlockFromBitstream<'_>,
    readers: &mut [&mut BitReader<'_>],
    group_dec_cache_block: &mut [f32],
    group_dec_cache_qblock: &mut [i32],
    group_dec_cache_qblock16: &mut [i16],
    group_dec_cache_scratch_space: &mut [f32],
    dec_state: DecStateParts<'_>,
    group_idx: usize,
    render_pipeline_input: &RenderPipelineInput,
    draw: DrawMode,
) -> Status {
    // TODO(veluca): investigate cache usage in this function.
    let shared = dec_state.shared;
    let block_rect = shared.block_group_rect(group_idx);
    let ac_strategy = &shared.ac_strategy;

    let xsize_blocks = block_rect.xsize();
    let ysize_blocks = block_rect.ysize();

    let dc = shared.dc();
    let dc_stride = dc.pixels_per_row();

    let inv_global_scale = shared.quantizer.inv_global_scale();

    let cs = &shared.frame_header.chroma_subsampling;

    let pipeline = dec_state.render_pipeline;
    let mut idct_buf = [0usize; 3];
    let mut idct_rect = [Rect::default(); 3];
    let mut idct_stride = [0usize; 3];
    for c in 0..3 {
        let (b, r) = render_pipeline_input.get_buffer(c)?;
        idct_buf[c] = b;
        idct_rect[c] = r;
        idct_stride[c] = pipeline.buffer_mut(b).pixels_per_row();
    }

    // (scaled_qtable serves JPEG reconstruction only.)

    let use_16 = matches!(dec_state.coefficients, AcImage::I16(_));
    // Whether or not coefficients should be stored for future usage, and/or read
    // from past usage.
    let accumulate = !dec_state.coefficients.is_empty();
    // Offset of the current block in the group.
    let mut offset = 0usize;

    // TODO(veluca): all of this should be done only once per image.
    // (if (decoded->IsJPEG()) { ... }: never, see the module comment.)

    let hshift = [cs.h_shift(0), cs.h_shift(1), cs.h_shift(2)];
    let vshift = [cs.v_shift(0), cs.v_shift(1), cs.v_shift(2)];
    let mut r = [Rect::default(); 3];
    for i in 0..3 {
        r[i] = Rect::new(
            block_rect.x0() >> hshift[i],
            block_rect.y0() >> vshift[i],
            block_rect.xsize() >> hshift[i],
            block_rect.ysize() >> vshift[i],
        );
        if !r[i].is_inside(&Rect::new(0, 0, dc.plane(i).xsize(), dc.plane(i).ysize())) {
            return Err(StatusCode::GenericError); // "Frame dimensions are too big for the image."
        }
    }

    let matrices = &shared.matrices;
    let biases = &dec_state.quant_biases;
    let coefficients = dec_state.coefficients;

    for by in 0..ysize_blocks {
        get_block.start_row(by);
        let sby = [by >> vshift[0], by >> vshift[1], by >> vshift[2]];

        let row_quant = block_rect.const_row(&shared.raw_quant_field, by);

        let dc_rows: [&[f32]; 3] = [
            r[0].const_plane_row(dc, 0, sby[0]),
            r[1].const_plane_row(dc, 1, sby[1]),
            r[2].const_plane_row(dc, 2, sby[2]),
        ];

        let ty = (block_rect.y0() + by) / K_COLOR_TILE_DIM_IN_BLOCKS;
        let acs_row = ac_strategy.const_row_rect(&block_rect, by);

        let row_cmap: [&[i8]; 3] = [
            shared.cmap.ytox_map.row(ty),
            &[],
            shared.cmap.ytob_map.row(ty),
        ];

        let mut bx = 0usize;
        for tx in 0..div_ceil(xsize_blocks, K_COLOR_TILE_DIM_IN_BLOCKS) {
            let abs_tx = tx + block_rect.x0() / K_COLOR_TILE_DIM_IN_BLOCKS;
            let x_cc_mul = shared.cmap.y_to_x_ratio(row_cmap[0][abs_tx] as i32);
            let b_cc_mul = shared.cmap.y_to_b_ratio(row_cmap[2][abs_tx] as i32);
            // Increment bx by llf_x because those iterations would otherwise
            // immediately continue (!IsFirstBlock). Reduces mispredictions.
            while bx < xsize_blocks && bx < (tx + 1) * K_COLOR_TILE_DIM_IN_BLOCKS {
                let sbx = [bx >> hshift[0], bx >> hshift[1], bx >> hshift[2]];
                let acs = acs_row.get(bx);
                let llf_x = acs.covered_blocks_x();

                // Can only happen in the second or lower rows of a varblock.
                if !acs.is_first_block() {
                    bx += llf_x;
                    continue;
                }
                let log2_covered_blocks = acs.log2_covered_blocks();

                let covered_blocks = 1usize << log2_covered_blocks;
                let size = covered_blocks * K_DCT_BLOCK_SIZE;

                let mut qblock = if accumulate {
                    match &mut *coefficients {
                        AcImage::I16(img) => {
                            let [p0, p1, p2] = img.planes_mut();
                            AcPtr::K16([
                                &mut p0.row_mut(group_idx)[offset..offset + size],
                                &mut p1.row_mut(group_idx)[offset..offset + size],
                                &mut p2.row_mut(group_idx)[offset..offset + size],
                            ])
                        }
                        AcImage::I32(img) => {
                            let [p0, p1, p2] = img.planes_mut();
                            AcPtr::K32([
                                &mut p0.row_mut(group_idx)[offset..offset + size],
                                &mut p1.row_mut(group_idx)[offset..offset + size],
                                &mut p2.row_mut(group_idx)[offset..offset + size],
                            ])
                        }
                    }
                } else {
                    // No point in reading from bitstream without accumulating and not
                    // drawing.
                    debug_assert!(draw == DrawMode::Draw);
                    if use_16 {
                        let q = &mut group_dec_cache_qblock16[..size * 3];
                        q.fill(0);
                        let (q0, rest) = q.split_at_mut(size);
                        let (q1, q2) = rest.split_at_mut(size);
                        AcPtr::K16([q0, q1, q2])
                    } else {
                        let q = &mut group_dec_cache_qblock[..size * 3];
                        q.fill(0);
                        let (q0, rest) = q.split_at_mut(size);
                        let (q1, q2) = rest.split_at_mut(size);
                        AcPtr::K32([q0, q1, q2])
                    }
                };
                get_block.load_block(
                    bx,
                    by,
                    &acs,
                    size,
                    log2_covered_blocks,
                    &mut qblock,
                    readers,
                )?;
                offset += size;
                if draw == DrawMode::DontDraw {
                    bx += llf_x;
                    continue;
                }

                // (if (JXL_UNLIKELY(decoded->IsJPEG())) { ... }: never.)
                {
                    let block = &mut *group_dec_cache_block;
                    // Dequantize and add predictions.
                    dequant_block(
                        &acs,
                        inv_global_scale,
                        row_quant[bx],
                        dec_state.x_dm_multiplier,
                        dec_state.b_dm_multiplier,
                        x_cc_mul,
                        b_cc_mul,
                        matrices.matrix(acs.raw_strategy() as usize, 0),
                        size,
                        acs.covered_blocks_y() * acs.covered_blocks_x(),
                        &sbx,
                        &dc_rows,
                        dc_stride,
                        biases,
                        &qblock,
                        block,
                    );

                    for c in [1usize, 0, 2] {
                        if (sbx[c] << hshift[c] != bx) || (sby[c] << vshift[c] != by) {
                            continue;
                        }
                        // IDCT
                        let img = pipeline.buffer_mut(idct_buf[c]);
                        let idct_pos = idct_rect[c].row_index(img, sby[c] * K_BLOCK_DIM)
                            + sbx[c] * K_BLOCK_DIM;
                        transform_to_pixels(
                            acs.strategy(),
                            &mut block[c * size..(c + 1) * size],
                            &mut img.data_mut()[idct_pos..],
                            idct_stride[c],
                            group_dec_cache_scratch_space,
                        );
                    }
                }
                bx += llf_x;
            }
        }
    }
    Ok(())
}

/// The parts of the `PassesDecoderState` that `DecodeGroupImpl()` uses
/// (borrowed apart from what the `GetBlockFromBitstream` holds).
struct DecStateParts<'a> {
    shared: &'a super::passes_state::PassesSharedState,
    x_dm_multiplier: f32,
    b_dm_multiplier: f32,
    quant_biases: [f32; 4],
    coefficients: &'a mut AcImage,
    render_pipeline: &'a mut super::render_pipeline::RenderPipeline,
}

/// Decode quantized AC coefficients of DCT blocks.
/// LLF components in the output block will not be modified.
/// Translation of `DecodeACVarBlock<ac_type>()`.
#[allow(clippy::too_many_arguments)]
fn decode_ac_var_block(
    ctx_offset: usize,
    log2_covered_blocks: usize,
    row_nzeros: &mut [i32],
    row_nzeros_top: Option<&[i32]>,
    nzeros_stride: usize,
    c: usize,
    bx: usize,
    by: usize,
    lbx: usize,
    acs: &AcStrategy,
    coeff_order: &[CoeffOrderT],
    br: &mut BitReader<'_>,
    decoder: &mut AnsSymbolReader<'_>,
    context_map: &[u8],
    qdc_row: &[u8],
    qf_row: &[i32],
    block_ctx_map: &BlockCtxMap,
    block: &mut AcPtr<'_>,
    shift: usize,
) -> Status {
    let _ = by;
    // Equal to number of LLF coefficients.
    let covered_blocks = 1usize << log2_covered_blocks;
    let size = covered_blocks * K_DCT_BLOCK_SIZE;
    let predicted_nzeros = predict_from_top_and_left(row_nzeros_top, row_nzeros, bx, 32);

    let ord = K_STRATEGY_ORDER[acs.raw_strategy() as usize] as usize;
    let order = &coeff_order[coeff_order_offset(ord, c)..];

    let block_ctx = block_ctx_map.context(qdc_row[lbx] as i32, qf_row[bx] as u32, ord, c);
    let nzero_ctx = (block_ctx_map.non_zero_context(predicted_nzeros as u32, block_ctx as u32)
        as usize)
        .wrapping_add(ctx_offset);

    let mut nzeros = decoder.read_hybrid_uint(nzero_ctx, br, context_map);
    if nzeros + covered_blocks > size {
        return Err(StatusCode::GenericError); // "Invalid AC: nzeros too large"
    }
    for y in 0..acs.covered_blocks_y() {
        for x in 0..acs.covered_blocks_x() {
            row_nzeros[bx + x + y * nzeros_stride] =
                ((nzeros + covered_blocks - 1) >> log2_covered_blocks) as i32;
        }
    }

    let histo_offset =
        ctx_offset + block_ctx_map.zero_density_contexts_offset(block_ctx as u32) as usize;

    // Skip LLF
    {
        let mut prev: usize = if nzeros > size / 16 { 0 } else { 1 };
        let mut k = covered_blocks;
        while k < size && nzeros != 0 {
            let ctx = histo_offset
                + zero_density_context(nzeros, k, covered_blocks, log2_covered_blocks, prev);
            let u_coeff = decoder.read_hybrid_uint(ctx, br, context_map);
            // Hand-rolled version of UnpackSigned, shifting before the conversion to
            // signed integer to avoid undefined behavior of shifting negative
            // numbers.
            let magnitude = u_coeff >> 1;
            let neg_sign = (!u_coeff) & 1;
            let coeff =
                ((magnitude ^ neg_sign.wrapping_sub(1)).wrapping_shl(shift as u32)) as isize;
            let idx = order[k] as usize;
            match block {
                AcPtr::K16(b) => {
                    let v = &mut b[c][idx];
                    *v = (*v as isize).wrapping_add(coeff) as i16;
                }
                AcPtr::K32(b) => {
                    let v = &mut b[c][idx];
                    *v = (*v as isize).wrapping_add(coeff) as i32;
                }
            }
            prev = (u_coeff != 0) as usize;
            nzeros -= prev;
            k += 1;
        }
        if nzeros != 0 {
            return Err(StatusCode::GenericError); // "Invalid AC: nzeros not 0. Block (bx, by), channel c"
        }
    }
    Ok(())
}

// Structs used by DecodeGroupImpl to get a quantized block.
// GetBlockFromBitstream uses ANS decoding (and thus keeps track of row
// pointers in row_nzeros), GetBlockFromEncoder simply reads the coefficient
// image provided by the encoder.

/// Translation of `GetBlockFromBitstream`.
struct GetBlockFromBitstream<'a> {
    shift_for_pass: &'a [u32], // not owned
    coeff_orders: &'a [CoeffOrderT],
    coeff_order_size: usize,
    context_map: &'a [Vec<u8>],
    decoders: Vec<AnsSymbolReader<'a>>,
    num_passes: usize,
    ctx_offset: [usize; super::base::K_MAX_NUM_PASSES],
    nzeros_stride: usize,
    // (row_nzeros and row_nzeros_top: rows `sby` and `sby - 1` of these)
    num_nzeroes: &'a mut [Image3<i32>],
    block_ctx_map: &'a BlockCtxMap,
    qf: &'a ImageI,
    quant_dc: &'a ImageB,
    by: usize,
    rect: Rect,
    hshift: [usize; 3],
    vshift: [usize; 3],
}

impl<'a> GetBlockFromBitstream<'a> {
    fn start_row(&mut self, by: usize) {
        // (qf_row, quant_dc_row and the row_nzeros pointers are taken from
        // `by` where they are used)
        self.by = by;
    }

    #[allow(clippy::too_many_arguments)]
    fn load_block(
        &mut self,
        bx: usize,
        by: usize,
        acs: &AcStrategy,
        size: usize,
        log2_covered_blocks: usize,
        block: &mut AcPtr<'_>,
        readers: &mut [&mut BitReader<'_>],
    ) -> Status {
        let _ = size;
        debug_assert_eq!(by, self.by);
        let qf_row = self.rect.const_row(self.qf, by);
        let quant_dc_row = &self.quant_dc.row(self.rect.y0() + by)[self.rect.x0()..];
        for c in [1usize, 0, 2] {
            let sbx = bx >> self.hshift[c];
            let sby = by >> self.vshift[c];
            if (sbx << self.hshift[c] != bx) || (sby << self.vshift[c] != by) {
                continue;
            }

            for pass in 0..self.num_passes {
                let plane = self.num_nzeroes[pass].plane_mut(c);
                let stride = plane.pixels_per_row();
                let (top, row) = plane.data_mut().split_at_mut(sby * stride);
                let row_nzeros_top = if sby == 0 {
                    None
                } else {
                    Some(&top[(sby - 1) * stride..])
                };
                decode_ac_var_block(
                    self.ctx_offset[pass],
                    log2_covered_blocks,
                    row,
                    row_nzeros_top,
                    self.nzeros_stride,
                    c,
                    sbx,
                    sby,
                    bx,
                    acs,
                    &self.coeff_orders[pass * self.coeff_order_size..],
                    readers[pass],
                    &mut self.decoders[pass],
                    &self.context_map[pass],
                    quant_dc_row,
                    qf_row,
                    self.block_ctx_map,
                    block,
                    self.shift_for_pass[pass] as usize,
                )?;
            }
        }
        Ok(())
    }

    /// Translation of `GetBlockFromBitstream::Init()`.
    #[allow(clippy::too_many_arguments)]
    fn init(
        readers: &mut [&mut BitReader<'_>],
        num_passes: usize,
        histo_selector_bits: usize,
        rect: Rect,
        num_nzeroes: &'a mut [Image3<i32>],
        dec_state_shared: &'a super::passes_state::PassesSharedState,
        dec_state_code: &'a [super::dec_ans::AnsCode],
        dec_state_context_map: &'a [Vec<u8>],
        first_pass: usize,
    ) -> Result<Self, StatusCode> {
        let mut hshift = [0usize; 3];
        let mut vshift = [0usize; 3];
        for i in 0..3 {
            hshift[i] = dec_state_shared.frame_header.chroma_subsampling.h_shift(i);
            vshift[i] = dec_state_shared.frame_header.chroma_subsampling.v_shift(i);
        }
        let coeff_order_size = dec_state_shared.coeff_order_size;
        let coeff_orders = &dec_state_shared.coeff_orders[first_pass * coeff_order_size..];
        let context_map = &dec_state_context_map[first_pass..];
        let shift_for_pass = &dec_state_shared.frame_header.passes.shift[first_pass..];
        let block_ctx_map = &dec_state_shared.block_ctx_map;
        let qf = &dec_state_shared.raw_quant_field;
        let quant_dc = &dec_state_shared.quant_dc;

        let mut ctx_offset = [0usize; super::base::K_MAX_NUM_PASSES];
        let mut decoders = Vec::with_capacity(num_passes);
        for pass in 0..num_passes {
            // Select which histogram set to use among those of the current pass.
            let mut cur_histogram = 0usize;
            if histo_selector_bits != 0 {
                cur_histogram = readers[pass].read_bits(histo_selector_bits) as usize;
            }
            if cur_histogram >= dec_state_shared.num_histograms {
                return Err(StatusCode::GenericError); // "Invalid histogram selector"
            }
            ctx_offset[pass] = cur_histogram * block_ctx_map.num_ac_contexts() as usize;

            decoders.push(AnsSymbolReader::new(
                &dec_state_code[pass + first_pass],
                readers[pass],
                0,
            ));
        }
        let nzeros_stride = num_nzeroes[0].pixels_per_row();
        for i in 0..num_passes {
            assert_eq!(nzeros_stride, num_nzeroes[i].pixels_per_row());
        }
        Ok(GetBlockFromBitstream {
            shift_for_pass,
            coeff_orders,
            coeff_order_size,
            context_map,
            decoders,
            num_passes,
            ctx_offset,
            nzeros_stride,
            num_nzeroes,
            block_ctx_map,
            qf,
            quant_dc,
            by: 0,
            rect,
            hshift,
            vshift,
        })
    }
}

/// Translation of `DecodeGroup()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_group(
    readers: &mut [&mut BitReader<'_>],
    num_passes: usize,
    group_idx: usize,
    dec_state: &mut PassesDecoderState,
    group_dec_cache: &mut GroupDecCache,
    thread: usize,
    render_pipeline_input: &RenderPipelineInput,
    first_pass: usize,
    force_draw: bool,
    dc_only: bool,
    should_run_pipeline: Option<&mut bool>,
) -> Status {
    let _ = thread;
    let draw = if (num_passes + first_pass
        == dec_state.shared_storage.frame_header.passes.num_passes as usize)
        || force_draw
    {
        DrawMode::Draw
    } else {
        DrawMode::DontDraw
    };

    if let Some(s) = should_run_pipeline {
        *s = draw != DrawMode::DontDraw;
    }

    if draw == DrawMode::Draw && num_passes == 0 && first_pass == 0 {
        // (The DC-only preview through the 8x upsampler of the DC image,
        // drawn only when a partial image is flushed, which SDL_image never
        // asks for; not translated.)
        return Err(StatusCode::GenericError);
    }

    let mut histo_selector_bits = 0usize;
    if dc_only {
        debug_assert_eq!(num_passes, 0);
    } else {
        debug_assert!(dec_state.shared_storage.num_histograms > 0);
        histo_selector_bits = ceil_log2_nonzero_u64(dec_state.shared_storage.num_histograms as u64);
    }

    let PassesDecoderState {
        shared_storage,
        code,
        context_map,
        x_dm_multiplier,
        b_dm_multiplier,
        coefficients,
        render_pipeline,
        output_encoding_info,
        ..
    } = dec_state;
    let Some(render_pipeline) = render_pipeline.as_mut() else {
        return Err(StatusCode::GenericError);
    };
    let GroupDecCache {
        dec_group_block,
        dec_group_qblock,
        dec_group_qblock16,
        scratch_space,
        num_nzeroes,
        ..
    } = group_dec_cache;

    let mut get_block = GetBlockFromBitstream::init(
        readers,
        num_passes,
        histo_selector_bits,
        shared_storage.block_group_rect(group_idx),
        num_nzeroes,
        shared_storage,
        code,
        context_map,
        first_pass,
    )?;

    let parts = DecStateParts {
        shared: shared_storage,
        x_dm_multiplier: *x_dm_multiplier,
        b_dm_multiplier: *b_dm_multiplier,
        quant_biases: output_encoding_info.opsin_params.quant_biases,
        coefficients,
        render_pipeline,
    };
    decode_group_impl(
        &mut get_block,
        readers,
        dec_group_block,
        dec_group_qblock,
        dec_group_qblock16,
        scratch_space,
        parts,
        group_idx,
        render_pipeline_input,
        draw,
    )?;

    for pass in 0..num_passes {
        if !get_block.decoders[pass].check_ans_final_state() {
            return Err(StatusCode::GenericError); // "ANS checksum failure."
        }
    }
    Ok(())
}
