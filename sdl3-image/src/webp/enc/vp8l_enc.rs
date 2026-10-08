// Rust translation of src/enc/vp8l_enc.c and src/enc/vp8li_enc.h from
// libwebp (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2012 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! main entry for the lossless encoder.
//!
//! The encoder runs on the calling thread (`thread_level` 0, as SDL_image
//! configures it: upstream's side thread then never starts). The
//! transform buffer's three parts (the image, the prediction scratch rows
//! and the transform data) are separate `Vec`s. What SDL_image's settings
//! never reach is not translated: the low-effort mode (method 0), the
//! near-lossless preprocessing, the delta palette, and the brute-force
//! configurations of method 6 at quality 100 (with the modified Zeng
//! palette sorting); nor are the statistics and the progress report.

use crate::webp::decode::{
    VP8LImageTransformType, CHUNK_HEADER_SIZE, CODE_LENGTH_CODES, MAX_HUFFMAN_BITS,
    MAX_PALETTE_SIZE, MIN_HUFFMAN_BITS, NUM_DISTANCE_CODES, NUM_LENGTH_CODES, RIFF_HEADER_SIZE,
    TAG_SIZE, TRANSFORM_PRESENT, VP8L_IMAGE_SIZE_BITS, VP8L_MAGIC_BYTE, VP8L_SIGNATURE_SIZE,
    VP8L_VERSION, VP8L_VERSION_BITS,
};
use crate::webp::dsp::lossless_enc::{
    vp8l_bundle_color_map, vp8l_fast_log2, vp8l_prefix_encode, vp8l_sub_pixels,
    vp8l_sub_sample_size, vp8l_subtract_green_from_blue_and_red,
};
use crate::webp::enc::backward_references_enc::{
    vp8l_backward_refs_clear, vp8l_backward_refs_init, vp8l_get_backward_references,
    vp8l_hash_chain_clear, vp8l_hash_chain_fill, vp8l_hash_chain_init, VP8LBackwardRefs,
    VP8LHashChain, K_LZ77_BOX, K_LZ77_RLE, K_LZ77_STANDARD, MAX_COLOR_CACHE_BITS,
    MAX_REFS_BLOCK_PER_IMAGE,
};
use crate::webp::enc::histogram_enc::{
    vp8l_allocate_histogram_set, vp8l_bits_entropy, vp8l_get_histo_image_symbols,
    vp8l_histogram_num_codes, vp8l_histogram_set_clear, vp8l_histogram_store_refs, VP8LHistogram,
    VP8LHistogramSet,
};
use crate::webp::enc::picture_csp_enc::webp_picture_has_transparency;
use crate::webp::enc::picture_enc::{webp_encoding_set_error, webp_picture_write};
use crate::webp::enc::predictor_enc::{vp8l_color_space_transform, vp8l_residual_image};
use crate::webp::encode::{WebPConfig, WebPEncodingError, WebPImageHint, WebPPicture};
use crate::webp::utils::bit_writer_utils::VP8LBitWriter;
use crate::webp::utils::huffman_encode_utils::{
    vp8l_create_compressed_huffman_tree, vp8l_create_huffman_tree, HuffmanTree, HuffmanTreeCode,
    HuffmanTreeToken,
};
use crate::webp::utils::{bits_log2_floor, put_le32, webp_get_color_palette};

/// maximum value of transform_bits_ in VP8LEncoder.
pub(crate) const MAX_TRANSFORM_BITS: i32 = 6;

/// Translation of `VP8LEncoderARGBContent`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VP8LEncoderARGBContent {
    None,
    Argb,
    Palette,
}

/// Translation of `VP8LEncoder`.
struct VP8LEncoder<'p> {
    /// user configuration and parameters
    config: &'p WebPConfig,
    /// input picture.
    pic: &'p WebPPicture,

    /// Transformed argb image data.
    argb: Vec<u32>,
    /// Content type of the argb buffer.
    argb_content: VP8LEncoderARGBContent,
    /// Scratch memory for argb rows (used for prediction).
    argb_scratch: Vec<u32>,
    /// Scratch memory for transform data.
    transform_data: Vec<u32>,

    /// Corresponds to packed image width.
    current_width: i32,

    // Encoding parameters derived from quality parameter.
    histo_bits: i32,
    /// <= MAX_TRANSFORM_BITS.
    transform_bits: i32,
    /// If equal to 0, don't use color cache.
    cache_bits: i32,

    // Encoding parameters derived from image characteristics.
    use_cross_color: bool,
    use_subtract_green: bool,
    use_predict: bool,
    use_palette: bool,
    palette_size: i32,
    palette: [u32; MAX_PALETTE_SIZE],
    /// Sorted version of palette_ for cache purposes.
    palette_sorted: [u32; MAX_PALETTE_SIZE],

    // Some 'scratch' (potentially large) objects.
    /// Backward Refs array for temporaries.
    refs: [VP8LBackwardRefs; 4],
    /// HashChain data for constructing backward references.
    hash_chain: VP8LHashChain,
}

/// Maximum number of histogram images (sub-blocks).
const MAX_HUFF_IMAGE_SIZE: i32 = 2600;

// Palette reordering for smaller sum of deltas (and for smaller storage).

/// Translation of `PaletteComponentDistance()`.
fn palette_component_distance(v: u32) -> u32 {
    if v <= 128 {
        v
    } else {
        256 - v
    }
}

/// Computes a value that is related to the entropy created by the
/// palette entry diff.
///
/// Note that the last & 0xff is a no-operation in the next statement, but
/// removed by most compilers and is here only for regularity of the code.
/// Translation of `PaletteColorDistance()`.
fn palette_color_distance(col1: u32, col2: u32) -> u32 {
    let diff = vp8l_sub_pixels(col1, col2);
    let k_more_weight_for_rgb_than_for_alpha = 9;
    let mut score = palette_component_distance(diff & 0xff);
    score += palette_component_distance((diff >> 8) & 0xff);
    score += palette_component_distance((diff >> 16) & 0xff);
    score *= k_more_weight_for_rgb_than_for_alpha;
    score += palette_component_distance((diff >> 24) & 0xff);
    score
}

/// Translation of `SearchColorNoIdx()`.
fn search_color_no_idx(sorted: &[u32], color: u32, num_colors: usize) -> usize {
    let mut low = 0;
    let mut hi = num_colors;
    if sorted[low] == color {
        return low; // loop invariant: sorted[low] != color
    }
    loop {
        let mid = (low + hi) >> 1;
        if sorted[mid] == color {
            return mid;
        } else if sorted[mid] < color {
            low = mid;
        } else {
            hi = mid;
        }
    }
}

/// The palette has been sorted by alpha. This function checks if the other
/// components of the palette have a monotonic development with regards to
/// position in the palette. If all have monotonic development, there is
/// no benefit to re-organize them greedily. A monotonic development
/// would be spotted in green-only situations (like lossy alpha) or
/// gray-scale images. Translation of `PaletteHasNonMonotonousDeltas()`.
fn palette_has_non_monotonous_deltas(palette: &[u32], num_colors: usize) -> bool {
    let mut predict = 0x000000;
    let mut sign_found = 0x00u8;
    for &p in &palette[..num_colors] {
        let diff = vp8l_sub_pixels(p, predict);
        let rd = ((diff >> 16) & 0xff) as u8;
        let gd = ((diff >> 8) & 0xff) as u8;
        let bd = (diff & 0xff) as u8;
        if rd != 0x00 {
            sign_found |= if rd < 0x80 { 1 } else { 2 };
        }
        if gd != 0x00 {
            sign_found |= if gd < 0x80 { 8 } else { 16 };
        }
        if bd != 0x00 {
            sign_found |= if bd < 0x80 { 64 } else { 128 };
        }
        predict = p;
    }
    (sign_found & (sign_found << 1)) != 0 // two consequent signs.
}

/// Translation of `PaletteSortMinimizeDeltas()`.
fn palette_sort_minimize_deltas(palette_sorted: &[u32], num_colors: usize, palette: &mut [u32]) {
    let mut predict = 0x00000000;
    palette[..num_colors].copy_from_slice(&palette_sorted[..num_colors]);
    if !palette_has_non_monotonous_deltas(palette_sorted, num_colors) {
        return;
    }
    // Find greedily always the closest color of the predicted color to minimize
    // deltas in the palette. This reduces storage needs since the
    // palette is stored with delta encoding.
    for i in 0..num_colors {
        let mut best_ix = i;
        let mut best_score = !0u32;
        for k in i..num_colors {
            let cur_score = palette_color_distance(palette[k], predict);
            if best_score > cur_score {
                best_score = cur_score;
                best_ix = k;
            }
        }
        palette.swap(best_ix, i);
        predict = palette[i];
    }
}

/// Sort palette in increasing order and prepare an inverse mapping array.
/// Translation of `PrepareMapToPalette()`.
fn prepare_map_to_palette(
    palette: &[u32],
    num_colors: usize,
    sorted: &mut [u32],
    idx_map: &mut [u32],
) {
    sorted[..num_colors].copy_from_slice(&palette[..num_colors]);
    // (the colors are distinct: any sort gives qsort()'s order)
    sorted[..num_colors].sort_unstable();
    for (i, &p) in palette[..num_colors].iter().enumerate() {
        idx_map[search_color_no_idx(sorted, p, num_colors)] = i as u32;
    }
}

// -----------------------------------------------------------------------------
// Palette

/// These five modes are evaluated and their respective entropy is
/// computed. Translation of `EntropyIx`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EntropyIx {
    Direct = 0,
    Spatial = 1,
    SubGreen = 2,
    SpatialSubGreen = 3,
    Palette = 4,
    PaletteAndSpatial = 5,
}

const K_NUM_ENTROPY_IX: usize = 6;

/// Translation of `PaletteSorting`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PaletteSorting {
    SortedDefault = 0,
    MinimizeDelta = 1,
    UnusedPalette = 3,
}

// Translation of `HistoIx`.
const K_HISTO_ALPHA: usize = 0;
const K_HISTO_ALPHA_PRED: usize = 1;
const K_HISTO_GREEN: usize = 2;
const K_HISTO_GREEN_PRED: usize = 3;
const K_HISTO_RED: usize = 4;
const K_HISTO_RED_PRED: usize = 5;
const K_HISTO_BLUE: usize = 6;
const K_HISTO_BLUE_PRED: usize = 7;
const K_HISTO_RED_SUB_GREEN: usize = 8;
const K_HISTO_RED_PRED_SUB_GREEN: usize = 9;
const K_HISTO_BLUE_SUB_GREEN: usize = 10;
const K_HISTO_BLUE_PRED_SUB_GREEN: usize = 11;
const K_HISTO_PALETTE: usize = 12;
/// Must be last.
const K_HISTO_TOTAL: usize = 13;

/// Translation of `AddSingleSubGreen()`.
fn add_single_sub_green(p: u32, histo: &mut [u32], r: usize, b: usize) {
    let green = p as i32 >> 8; // The upper bits are masked away later.
    histo[r * 256 + (((p as i32 >> 16) - green) & 0xff) as usize] += 1;
    histo[b * 256 + ((p as i32 - green) & 0xff) as usize] += 1;
}

/// Translation of `AddSingle()`.
fn add_single(p: u32, histo: &mut [u32], a: usize, r: usize, g: usize, b: usize) {
    histo[a * 256 + ((p >> 24) & 0xff) as usize] += 1;
    histo[r * 256 + ((p >> 16) & 0xff) as usize] += 1;
    histo[g * 256 + ((p >> 8) & 0xff) as usize] += 1;
    histo[b * 256 + (p & 0xff) as usize] += 1;
}

/// Translation of `HashPix()`.
fn hash_pix(pix: u32) -> u32 {
    // Note that masking with 0xffffffffu is for preventing an
    // 'unsigned int overflow' warning. Doesn't impact the compiled code.
    (((pix as u64 + (pix >> 19) as u64).wrapping_mul(0x39c5fba7) & 0xffffffff) >> 24) as u32
}

/// Translation of `AnalyzeEntropy()`.
#[allow(clippy::too_many_arguments)]
fn analyze_entropy(
    argb: &[u32],
    width: i32,
    height: i32,
    argb_stride: i32,
    use_palette: bool,
    palette_size: i32,
    transform_bits: i32,
    min_entropy_ix: &mut EntropyIx,
    red_and_blue_always_zero: &mut bool,
) -> bool {
    if use_palette && palette_size <= 16 {
        // In the case of small palettes, we pack 2, 4 or 8 pixels together. In
        // practice, small palettes are better than any other transform.
        *min_entropy_ix = EntropyIx::Palette;
        *red_and_blue_always_zero = true;
        return true;
    }
    let mut histo = vec![0u32; K_HISTO_TOTAL * 256];
    {
        let mut prev_row: Option<usize> = None;
        let mut curr_row = 0usize;
        let mut pix_prev = argb[0]; // Skip the first pixel.
        for _y in 0..height {
            for x in 0..width as usize {
                let pix = argb[curr_row + x];
                let pix_diff = vp8l_sub_pixels(pix, pix_prev);
                pix_prev = pix;
                if pix_diff == 0 || prev_row.is_some_and(|prev| pix == argb[prev + x]) {
                    continue;
                }
                add_single(
                    pix,
                    &mut histo,
                    K_HISTO_ALPHA,
                    K_HISTO_RED,
                    K_HISTO_GREEN,
                    K_HISTO_BLUE,
                );
                add_single(
                    pix_diff,
                    &mut histo,
                    K_HISTO_ALPHA_PRED,
                    K_HISTO_RED_PRED,
                    K_HISTO_GREEN_PRED,
                    K_HISTO_BLUE_PRED,
                );
                add_single_sub_green(
                    pix,
                    &mut histo,
                    K_HISTO_RED_SUB_GREEN,
                    K_HISTO_BLUE_SUB_GREEN,
                );
                add_single_sub_green(
                    pix_diff,
                    &mut histo,
                    K_HISTO_RED_PRED_SUB_GREEN,
                    K_HISTO_BLUE_PRED_SUB_GREEN,
                );
                {
                    // Approximate the palette by the entropy of the multiplicative hash.
                    let hash = hash_pix(pix);
                    histo[K_HISTO_PALETTE * 256 + hash as usize] += 1;
                }
            }
            prev_row = Some(curr_row);
            curr_row += argb_stride as usize;
        }
        {
            let mut entropy_comp = [0.0f32; K_HISTO_TOTAL];
            let mut entropy = [0.0f32; K_NUM_ENTROPY_IX];
            let last_mode_to_analyze = if use_palette {
                EntropyIx::Palette
            } else {
                EntropyIx::SpatialSubGreen
            };
            // Let's add one zero to the predicted histograms. The zeros are removed
            // too efficiently by the pix_diff == 0 comparison, at least one of the
            // zeros is likely to exist.
            histo[K_HISTO_RED_PRED_SUB_GREEN * 256] += 1;
            histo[K_HISTO_BLUE_PRED_SUB_GREEN * 256] += 1;
            histo[K_HISTO_RED_PRED * 256] += 1;
            histo[K_HISTO_GREEN_PRED * 256] += 1;
            histo[K_HISTO_BLUE_PRED * 256] += 1;
            histo[K_HISTO_ALPHA_PRED * 256] += 1;

            for j in 0..K_HISTO_TOTAL {
                entropy_comp[j] = vp8l_bits_entropy(&histo[j * 256..], 256);
            }
            entropy[EntropyIx::Direct as usize] = entropy_comp[K_HISTO_ALPHA]
                + entropy_comp[K_HISTO_RED]
                + entropy_comp[K_HISTO_GREEN]
                + entropy_comp[K_HISTO_BLUE];
            entropy[EntropyIx::Spatial as usize] = entropy_comp[K_HISTO_ALPHA_PRED]
                + entropy_comp[K_HISTO_RED_PRED]
                + entropy_comp[K_HISTO_GREEN_PRED]
                + entropy_comp[K_HISTO_BLUE_PRED];
            entropy[EntropyIx::SubGreen as usize] = entropy_comp[K_HISTO_ALPHA]
                + entropy_comp[K_HISTO_RED_SUB_GREEN]
                + entropy_comp[K_HISTO_GREEN]
                + entropy_comp[K_HISTO_BLUE_SUB_GREEN];
            entropy[EntropyIx::SpatialSubGreen as usize] = entropy_comp[K_HISTO_ALPHA_PRED]
                + entropy_comp[K_HISTO_RED_PRED_SUB_GREEN]
                + entropy_comp[K_HISTO_GREEN_PRED]
                + entropy_comp[K_HISTO_BLUE_PRED_SUB_GREEN];
            entropy[EntropyIx::Palette as usize] = entropy_comp[K_HISTO_PALETTE];

            // When including transforms, there is an overhead in bits from
            // storing them. This overhead is small but matters for small images.
            // For spatial, there are 14 transformations.
            let transform_count = vp8l_sub_sample_size(width as u32, transform_bits as u32)
                .wrapping_mul(vp8l_sub_sample_size(height as u32, transform_bits as u32));
            entropy[EntropyIx::Spatial as usize] += transform_count as f32 * vp8l_fast_log2(14);
            // For color transforms: 24 as only 3 channels are considered in a
            // ColorTransformElement.
            entropy[EntropyIx::SpatialSubGreen as usize] +=
                transform_count as f32 * vp8l_fast_log2(24);
            // For palettes, add the cost of storing the palette.
            // We empirically estimate the cost of a compressed entry as 8 bits.
            // The palette is differential-coded when compressed hence a much
            // lower cost than sizeof(uint32_t)*8.
            entropy[EntropyIx::Palette as usize] += (palette_size * 8) as f32;

            *min_entropy_ix = EntropyIx::Direct;
            const MODES: [EntropyIx; 5] = [
                EntropyIx::Direct,
                EntropyIx::Spatial,
                EntropyIx::SubGreen,
                EntropyIx::SpatialSubGreen,
                EntropyIx::Palette,
            ];
            for k in EntropyIx::Direct as usize + 1..=last_mode_to_analyze as usize {
                if entropy[*min_entropy_ix as usize] > entropy[k] {
                    *min_entropy_ix = MODES[k];
                }
            }
            debug_assert!(*min_entropy_ix as usize <= last_mode_to_analyze as usize);
            *red_and_blue_always_zero = true;
            // Let's check if the histogram of the chosen entropy mode has
            // non-zero red and blue values. If all are zero, we can later skip
            // the cross color optimization.
            {
                const K_HISTO_PAIRS: [[usize; 2]; 5] = [
                    [K_HISTO_RED, K_HISTO_BLUE],
                    [K_HISTO_RED_PRED, K_HISTO_BLUE_PRED],
                    [K_HISTO_RED_SUB_GREEN, K_HISTO_BLUE_SUB_GREEN],
                    [K_HISTO_RED_PRED_SUB_GREEN, K_HISTO_BLUE_PRED_SUB_GREEN],
                    [K_HISTO_RED, K_HISTO_BLUE],
                ];
                let red_histo = 256 * K_HISTO_PAIRS[*min_entropy_ix as usize][0];
                let blue_histo = 256 * K_HISTO_PAIRS[*min_entropy_ix as usize][1];
                for i in 1..256 {
                    if (histo[red_histo + i] | histo[blue_histo + i]) != 0 {
                        *red_and_blue_always_zero = false;
                        break;
                    }
                }
            }
        }
        true
    }
}

/// Translation of `GetHistoBits()`.
fn get_histo_bits(method: i32, use_palette: bool, width: i32, height: i32) -> i32 {
    // Make tile size a function of encoding method (Range: 0 to 6).
    let mut histo_bits = (if use_palette { 9 } else { 7 }) - method;
    loop {
        let huff_image_size = vp8l_sub_sample_size(width as u32, histo_bits as u32)
            * vp8l_sub_sample_size(height as u32, histo_bits as u32);
        if huff_image_size as i32 <= MAX_HUFF_IMAGE_SIZE {
            break;
        }
        histo_bits += 1;
    }
    histo_bits.clamp(MIN_HUFFMAN_BITS, MAX_HUFFMAN_BITS)
}

/// Translation of `GetTransformBits()`.
fn get_transform_bits(method: i32, histo_bits: i32) -> i32 {
    let max_transform_bits = if method < 4 {
        6
    } else if method > 4 {
        4
    } else {
        5
    };
    let res = if histo_bits > max_transform_bits {
        max_transform_bits
    } else {
        histo_bits
    };
    debug_assert!(res <= MAX_TRANSFORM_BITS);
    res
}

/// Set of parameters to be used in each iteration of the cruncher.
const CRUNCH_SUBCONFIGS_MAX: usize = 2;

/// Translation of `CrunchSubConfig`.
#[derive(Clone, Copy, Default)]
struct CrunchSubConfig {
    lz77: i32,
    do_no_cache: bool,
}

/// Translation of `CrunchConfig`.
#[derive(Clone, Copy)]
struct CrunchConfig {
    entropy_idx: EntropyIx,
    palette_sorting_type: PaletteSorting,
    sub_configs: [CrunchSubConfig; CRUNCH_SUBCONFIGS_MAX],
    sub_configs_size: usize,
}

/// Translation of `EncoderAnalyze()` (for the methods 1 to 6, with one
/// configuration but at method 5 or for method 6 at quality 100, which are
/// not translated).
fn encoder_analyze(
    enc: &mut VP8LEncoder<'_>,
    crunch_configs: &mut Vec<CrunchConfig>,
    red_and_blue_always_zero: &mut bool,
) -> bool {
    let pic = enc.pic;
    let width = pic.width;
    let height = pic.height;
    let config = enc.config;
    let method = config.method;
    let low_effort = config.method == 0;
    debug_assert!(!pic.argb.is_empty());
    debug_assert!(!low_effort, "the low-effort mode is not translated");
    debug_assert!(
        method != 5 || config.quality < 75.0,
        "method 5's no-cache configurations are not translated"
    );
    debug_assert!(
        method != 6 || config.quality != 100.0,
        "method 6's brute force at quality 100 is not translated"
    );

    // Check whether a palette is possible.
    enc.palette_size = webp_get_color_palette(pic, Some(&mut enc.palette_sorted));
    let use_palette = enc.palette_size <= MAX_PALETTE_SIZE as i32;
    if !use_palette {
        enc.palette_size = 0;
    } else {
        // (the colors are distinct: any sort gives qsort()'s order)
        enc.palette_sorted[..enc.palette_size as usize].sort_unstable();
    }

    // Empirical bit sizes.
    enc.histo_bits = get_histo_bits(method, use_palette, pic.width, pic.height);
    enc.transform_bits = get_transform_bits(method, enc.histo_bits);

    // Try out multiple LZ77 on images with few colors.
    let n_lz77s = if enc.palette_size > 0 && enc.palette_size <= 16 {
        2
    } else {
        1
    };
    let mut min_entropy_ix = EntropyIx::Direct;
    if !analyze_entropy(
        &pic.argb,
        width,
        height,
        pic.argb_stride,
        use_palette,
        enc.palette_size,
        enc.transform_bits,
        &mut min_entropy_ix,
        red_and_blue_always_zero,
    ) {
        return false;
    }
    // Only choose the guessed best transform.
    crunch_configs.clear();
    crunch_configs.push(CrunchConfig {
        entropy_idx: min_entropy_ix,
        palette_sorting_type: if use_palette {
            PaletteSorting::MinimizeDelta
        } else {
            PaletteSorting::UnusedPalette
        },
        sub_configs: [CrunchSubConfig::default(); CRUNCH_SUBCONFIGS_MAX],
        sub_configs_size: 0,
    });
    // Fill in the different LZ77s.
    debug_assert!(n_lz77s <= CRUNCH_SUBCONFIGS_MAX);
    for c in crunch_configs.iter_mut() {
        for j in 0..n_lz77s {
            debug_assert!(j < CRUNCH_SUBCONFIGS_MAX);
            c.sub_configs[j].lz77 = if j == 0 {
                K_LZ77_STANDARD | K_LZ77_RLE
            } else {
                K_LZ77_BOX
            };
            c.sub_configs[j].do_no_cache = false;
        }
        c.sub_configs_size = n_lz77s;
    }
    true
}

/// Translation of `EncoderInit()`.
fn encoder_init(enc: &mut VP8LEncoder<'_>) -> bool {
    let pic = enc.pic;
    let width = pic.width;
    let height = pic.height;
    let pix_cnt = width * height;
    // we round the block size up, so we're guaranteed to have
    // at most MAX_REFS_BLOCK_PER_IMAGE blocks used:
    let refs_block_size = (pix_cnt - 1) / MAX_REFS_BLOCK_PER_IMAGE + 1;
    if !vp8l_hash_chain_init(&mut enc.hash_chain, pix_cnt) {
        return false;
    }

    for r in enc.refs.iter_mut() {
        vp8l_backward_refs_init(r, refs_block_size);
    }

    true
}

/// Returns false in case of memory error. Translation of
/// `GetHuffBitLengthsAndCodes()`.
fn get_huff_bit_lengths_and_codes(
    histogram_image: &mut VP8LHistogramSet,
    huffman_codes: &mut [HuffmanTreeCode],
) -> bool {
    let histogram_image_size = histogram_image.size as usize;
    let mut max_num_symbols = 0;

    // Iterate over all histograms and get the aggregate number of codes used.
    for i in 0..histogram_image_size {
        let histo = histogram_image.histograms[i].as_ref().unwrap();
        let codes = &mut huffman_codes[5 * i..5 * i + 5];
        for (k, code) in codes.iter_mut().enumerate() {
            let num_symbols = match k {
                0 => vp8l_histogram_num_codes(histo.palette_code_bits),
                4 => NUM_DISTANCE_CODES,
                _ => 256,
            };
            code.num_symbols = num_symbols;
        }
    }

    // Allocate and Set Huffman codes.
    for code in huffman_codes[..5 * histogram_image_size].iter_mut() {
        let bit_length = code.num_symbols as usize;
        code.codes = vec![0; bit_length];
        code.code_lengths = vec![0; bit_length];
        if max_num_symbols < bit_length {
            max_num_symbols = bit_length;
        }
    }

    let mut buf_rle = vec![0u8; max_num_symbols];
    let mut huff_tree = vec![HuffmanTree::default(); 3 * max_num_symbols];

    // Create Huffman trees.
    for i in 0..histogram_image_size {
        let codes = &mut huffman_codes[5 * i..5 * i + 5];
        let histo: &mut VP8LHistogram = histogram_image.histograms[i].as_mut().unwrap();
        vp8l_create_huffman_tree(
            &mut histo.literal,
            15,
            &mut buf_rle,
            &mut huff_tree,
            &mut codes[0],
        );
        vp8l_create_huffman_tree(
            &mut histo.red,
            15,
            &mut buf_rle,
            &mut huff_tree,
            &mut codes[1],
        );
        vp8l_create_huffman_tree(
            &mut histo.blue,
            15,
            &mut buf_rle,
            &mut huff_tree,
            &mut codes[2],
        );
        vp8l_create_huffman_tree(
            &mut histo.alpha,
            15,
            &mut buf_rle,
            &mut huff_tree,
            &mut codes[3],
        );
        vp8l_create_huffman_tree(
            &mut histo.distance,
            15,
            &mut buf_rle,
            &mut huff_tree,
            &mut codes[4],
        );
    }
    true
}

/// Translation of `StoreHuffmanTreeOfHuffmanTreeToBitMask()`.
fn store_huffman_tree_of_huffman_tree_to_bit_mask(
    bw: &mut VP8LBitWriter,
    code_length_bitdepth: &[u8],
) {
    // RFC 1951 will calm you down if you are worried about this funny sequence.
    // This sequence is tuned from that, but more weighted for lower symbol count,
    // and more spiking histograms.
    const K_STORAGE_ORDER: [u8; CODE_LENGTH_CODES as usize] = [
        17, 18, 0, 1, 2, 3, 4, 5, 16, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
    ];
    // Throw away trailing zeros:
    let mut codes_to_store = CODE_LENGTH_CODES as usize;
    while codes_to_store > 4 {
        if code_length_bitdepth[K_STORAGE_ORDER[codes_to_store - 1] as usize] != 0 {
            break;
        }
        codes_to_store -= 1;
    }
    bw.put_bits(codes_to_store as u32 - 4, 4);
    for &o in &K_STORAGE_ORDER[..codes_to_store] {
        bw.put_bits(code_length_bitdepth[o as usize] as u32, 3);
    }
}

/// Translation of `ClearHuffmanTreeIfOnlyOneSymbol()`.
fn clear_huffman_tree_if_only_one_symbol(huffman_code: &mut HuffmanTreeCode) {
    let mut count = 0;
    let n = huffman_code.num_symbols as usize;
    for k in 0..n {
        if huffman_code.code_lengths[k] != 0 {
            count += 1;
            if count > 1 {
                return;
            }
        }
    }
    huffman_code.code_lengths[..n].fill(0);
    huffman_code.codes[..n].fill(0);
}

/// Translation of `StoreHuffmanTreeToBitMask()`.
fn store_huffman_tree_to_bit_mask(
    bw: &mut VP8LBitWriter,
    tokens: &[HuffmanTreeToken],
    huffman_code: &HuffmanTreeCode,
) {
    for t in tokens {
        let ix = t.code as usize;
        let extra_bits = t.extra_bits as u32;
        bw.put_bits(
            huffman_code.codes[ix] as u32,
            huffman_code.code_lengths[ix] as i32,
        );
        match ix {
            16 => bw.put_bits(extra_bits, 2),
            17 => bw.put_bits(extra_bits, 3),
            18 => bw.put_bits(extra_bits, 7),
            _ => {}
        }
    }
}

/// 'huff_tree' and 'tokens' are pre-alloacted buffers. Translation of
/// `StoreFullHuffmanCode()`.
fn store_full_huffman_code(
    bw: &mut VP8LBitWriter,
    huff_tree: &mut [HuffmanTree],
    tokens: &mut [HuffmanTreeToken],
    tree: &HuffmanTreeCode,
) {
    let max_tokens = tree.num_symbols as usize;
    let mut huffman_code = HuffmanTreeCode {
        num_symbols: CODE_LENGTH_CODES,
        code_lengths: vec![0; CODE_LENGTH_CODES as usize],
        codes: vec![0; CODE_LENGTH_CODES as usize],
    };

    bw.put_bits(0, 1);
    let num_tokens = vp8l_create_compressed_huffman_tree(tree, tokens, max_tokens);
    {
        let mut histogram = [0u32; CODE_LENGTH_CODES as usize];
        let mut buf_rle = [0u8; CODE_LENGTH_CODES as usize];
        for t in &tokens[..num_tokens] {
            histogram[t.code as usize] += 1;
        }

        vp8l_create_huffman_tree(
            &mut histogram,
            7,
            &mut buf_rle,
            huff_tree,
            &mut huffman_code,
        );
    }

    store_huffman_tree_of_huffman_tree_to_bit_mask(bw, &huffman_code.code_lengths);
    clear_huffman_tree_if_only_one_symbol(&mut huffman_code);
    {
        let mut trailing_zero_bits = 0;
        let mut trimmed_length = num_tokens as i32;
        let mut i = num_tokens;
        while i > 0 {
            i -= 1;
            let ix = tokens[i].code as usize;
            if ix == 0 || ix == 17 || ix == 18 {
                trimmed_length -= 1; // discount trailing zeros
                trailing_zero_bits += huffman_code.code_lengths[ix] as i32;
                if ix == 17 {
                    trailing_zero_bits += 3;
                } else if ix == 18 {
                    trailing_zero_bits += 7;
                }
            } else {
                break;
            }
        }
        let write_trimmed_length = trimmed_length > 1 && trailing_zero_bits > 12;
        let length = if write_trimmed_length {
            trimmed_length as usize
        } else {
            num_tokens
        };
        bw.put_bits(write_trimmed_length as u32, 1);
        if write_trimmed_length {
            if trimmed_length == 2 {
                bw.put_bits(0, 3 + 2); // nbitpairs=1, trimmed_length=2
            } else {
                let nbits = bits_log2_floor((trimmed_length - 2) as u32);
                let nbitpairs = nbits / 2 + 1;
                debug_assert!(trimmed_length > 2);
                debug_assert!(nbitpairs - 1 < 8);
                bw.put_bits((nbitpairs - 1) as u32, 3);
                bw.put_bits((trimmed_length - 2) as u32, nbitpairs * 2);
            }
        }
        store_huffman_tree_to_bit_mask(bw, &tokens[..length], &huffman_code);
    }
}

/// 'huff_tree' and 'tokens' are pre-alloacted buffers. Translation of
/// `StoreHuffmanCode()`.
fn store_huffman_code(
    bw: &mut VP8LBitWriter,
    huff_tree: &mut [HuffmanTree],
    tokens: &mut [HuffmanTreeToken],
    huffman_code: &HuffmanTreeCode,
) {
    let mut count = 0;
    let mut symbols = [0i32; 2];
    let k_max_bits = 8;
    let k_max_symbol = 1 << k_max_bits;

    // Check whether it's a small tree.
    let mut i = 0;
    while i < huffman_code.num_symbols as usize && count < 3 {
        if huffman_code.code_lengths[i] != 0 {
            if count < 2 {
                symbols[count] = i as i32;
            }
            count += 1;
        }
        i += 1;
    }

    if count == 0 {
        // emit minimal tree for empty cases
        // bits: small tree marker: 1, count-1: 0, large 8-bit code: 0, code: 0
        bw.put_bits(0x01, 4);
    } else if count <= 2 && symbols[0] < k_max_symbol && symbols[1] < k_max_symbol {
        bw.put_bits(1, 1); // Small tree marker to encode 1 or 2 symbols.
        bw.put_bits(count as u32 - 1, 1);
        if symbols[0] <= 1 {
            bw.put_bits(0, 1); // Code bit for small (1 bit) symbol value.
            bw.put_bits(symbols[0] as u32, 1);
        } else {
            bw.put_bits(1, 1);
            bw.put_bits(symbols[0] as u32, 8);
        }
        if count == 2 {
            bw.put_bits(symbols[1] as u32, 8);
        }
    } else {
        store_full_huffman_code(bw, huff_tree, tokens, huffman_code);
    }
}

/// Translation of `WriteHuffmanCode()`.
fn write_huffman_code(bw: &mut VP8LBitWriter, code: &HuffmanTreeCode, code_index: usize) {
    let depth = code.code_lengths[code_index] as i32;
    let symbol = code.codes[code_index] as u32;
    bw.put_bits(symbol, depth);
}

/// Translation of `WriteHuffmanCodeWithExtraBits()`.
fn write_huffman_code_with_extra_bits(
    bw: &mut VP8LBitWriter,
    code: &HuffmanTreeCode,
    code_index: usize,
    bits: i32,
    n_bits: i32,
) {
    let depth = code.code_lengths[code_index] as i32;
    let symbol = code.codes[code_index] as u32;
    bw.put_bits(((bits as u32) << depth) | symbol, depth + n_bits);
}

/// Translation of `StoreImageToBitMask()`.
fn store_image_to_bit_mask(
    bw: &mut VP8LBitWriter,
    width: i32,
    histo_bits: i32,
    refs: &VP8LBackwardRefs,
    histogram_symbols: &[u16],
    huffman_codes: &[HuffmanTreeCode],
    pic: &WebPPicture,
) -> bool {
    let histo_xsize = if histo_bits != 0 {
        vp8l_sub_sample_size(width as u32, histo_bits as u32) as i32
    } else {
        1
    };
    let tile_mask = if histo_bits == 0 {
        0
    } else {
        -(1i32 << histo_bits)
    };
    // x and y trace the position in the image.
    let mut x = 0;
    let mut y = 0;
    let mut tile_x = x & tile_mask;
    let mut tile_y = y & tile_mask;
    let mut histogram_ix = histogram_symbols[0] as usize;
    let mut codes = &huffman_codes[5 * histogram_ix..];
    for v in &refs.refs {
        if (tile_x != (x & tile_mask)) || (tile_y != (y & tile_mask)) {
            tile_x = x & tile_mask;
            tile_y = y & tile_mask;
            histogram_ix = histogram_symbols
                [((y >> histo_bits) * histo_xsize + (x >> histo_bits)) as usize]
                as usize;
            codes = &huffman_codes[5 * histogram_ix..];
        }
        if v.is_literal() {
            const ORDER: [u8; 4] = [1, 2, 0, 3];
            for k in 0..4 {
                let code = v.literal(ORDER[k] as i32) as usize;
                write_huffman_code(bw, &codes[k], code);
            }
        } else if v.is_cache_idx() {
            let code = v.cache_idx() as usize;
            let literal_ix = 256 + NUM_LENGTH_CODES as usize + code;
            write_huffman_code(bw, &codes[0], literal_ix);
        } else {
            let distance = v.distance();
            let (code, n_bits, bits) = vp8l_prefix_encode(v.len as i32);
            write_huffman_code_with_extra_bits(bw, &codes[0], 256 + code as usize, bits, n_bits);

            // Don't write the distance with the extra bits code since
            // the distance can be up to 18 bits of extra bits, and the prefix
            // 15 bits, totaling to 33, and our PutBits only supports up to 32 bits.
            let (code, n_bits, bits) = vp8l_prefix_encode(distance as i32);
            write_huffman_code(bw, &codes[4], code as usize);
            bw.put_bits(bits as u32, n_bits);
        }
        x += v.length() as i32;
        while x >= width {
            x -= width;
            y += 1;
        }
    }
    if bw.error {
        return webp_encoding_set_error(pic, WebPEncodingError::OutOfMemory);
    }
    true
}

/// Special case of EncodeImageInternal() for cache-bits=0,
/// histo_bits=31. Translation of `EncodeImageNoHuffman()` (`refs_array`
/// holds the references and a temporary).
#[allow(clippy::too_many_arguments)]
fn encode_image_no_huffman(
    bw: &mut VP8LBitWriter,
    argb: &[u32],
    hash_chain: &mut VP8LHashChain,
    refs_array: &mut [VP8LBackwardRefs],
    width: i32,
    height: i32,
    quality: i32,
    low_effort: bool,
    pic: &WebPPicture,
) -> bool {
    let mut huffman_codes: [HuffmanTreeCode; 5] = Default::default();
    let histogram_symbols = [0u16; 1]; // only one tree, one symbol
    let mut cache_bits = 0;
    let mut huff_tree = vec![HuffmanTree::default(); 3 * CODE_LENGTH_CODES as usize];

    let ok = (|| {
        // Calculate backward references from ARGB image.
        if !vp8l_hash_chain_fill(hash_chain, quality, argb, width, height, low_effort) {
            return false;
        }
        if !vp8l_get_backward_references(
            width,
            height,
            argb,
            quality,
            /*low_effort=*/ false,
            K_LZ77_STANDARD | K_LZ77_RLE,
            cache_bits,
            /*do_no_cache=*/ false,
            hash_chain,
            refs_array,
            &mut cache_bits,
        ) {
            webp_encoding_set_error(pic, WebPEncodingError::OutOfMemory);
            return false;
        }
        let refs = &refs_array[0];
        let mut histogram_image = vp8l_allocate_histogram_set(1, cache_bits);
        vp8l_histogram_set_clear(&mut histogram_image);

        // Build histogram image and symbols from backward references.
        vp8l_histogram_store_refs(refs, histogram_image.histograms[0].as_mut().unwrap());

        // Create Huffman bit lengths and codes for each histogram image.
        debug_assert!(histogram_image.size == 1);
        if !get_huff_bit_lengths_and_codes(&mut histogram_image, &mut huffman_codes) {
            webp_encoding_set_error(pic, WebPEncodingError::OutOfMemory);
            return false;
        }

        // No color cache, no Huffman image.
        bw.put_bits(0, 1);

        // Find maximum number of symbols for the huffman tree-set.
        let mut max_tokens = 0;
        for codes in &huffman_codes {
            if max_tokens < codes.num_symbols as usize {
                max_tokens = codes.num_symbols as usize;
            }
        }

        let mut tokens = vec![HuffmanTreeToken::default(); max_tokens];

        // Store Huffman codes.
        for codes in huffman_codes.iter_mut() {
            store_huffman_code(bw, &mut huff_tree, &mut tokens, codes);
            clear_huffman_tree_if_only_one_symbol(codes);
        }

        // Store actual literals.
        store_image_to_bit_mask(bw, width, 0, refs, &histogram_symbols, &huffman_codes, pic)
    })();
    let _ = ok;

    pic.error_code.get() == WebPEncodingError::Ok
}

/// Translation of `EncodeImageInternal()` (without the progress report).
#[allow(clippy::too_many_arguments)]
fn encode_image_internal(
    bw: &mut VP8LBitWriter,
    argb: &[u32],
    hash_chain: &mut VP8LHashChain,
    refs_array: &mut [VP8LBackwardRefs; 4],
    width: i32,
    height: i32,
    quality: i32,
    low_effort: bool,
    use_cache: bool,
    config: &CrunchConfig,
    cache_bits: &mut i32,
    histogram_bits: i32,
    init_byte_position: usize,
    hdr_size: &mut i32,
    data_size: &mut i32,
    pic: &WebPPicture,
) -> bool {
    let histogram_image_xysize = vp8l_sub_sample_size(width as u32, histogram_bits as u32)
        * vp8l_sub_sample_size(height as u32, histogram_bits as u32);
    let mut huff_tree = vec![HuffmanTree::default(); 3 * CODE_LENGTH_CODES as usize];
    let mut histogram_symbols = vec![0u16; histogram_image_xysize as usize];
    let bw_init = bw.state();
    let mut bw_best = VP8LBitWriter::new(0);
    let mut hash_chain_histogram = VP8LHashChain::default(); // histogram image hash chain
    let mut bw_size_best = !0usize;
    debug_assert!(histogram_bits >= MIN_HUFFMAN_BITS);
    debug_assert!(histogram_bits <= MAX_HUFFMAN_BITS);

    let ok = (|| {
        // Make sure we can allocate the different objects.
        if !vp8l_hash_chain_init(&mut hash_chain_histogram, histogram_image_xysize as i32) {
            webp_encoding_set_error(pic, WebPEncodingError::OutOfMemory);
            return false;
        }

        if !vp8l_hash_chain_fill(hash_chain, quality, argb, width, height, low_effort) {
            return false;
        }

        let cache_bits_init = if use_cache {
            // If the value is different from zero, it has been set during the
            // palette analysis.
            if *cache_bits == 0 {
                MAX_COLOR_CACHE_BITS
            } else {
                *cache_bits
            }
        } else {
            0
        };
        // If several iterations will happen, clone into bw_best.
        if (config.sub_configs_size > 1 || config.sub_configs[0].do_no_cache)
            && !bw.clone_into(&mut bw_best)
        {
            webp_encoding_set_error(pic, WebPEncodingError::OutOfMemory);
            return false;
        }

        for sub_config in &config.sub_configs[..config.sub_configs_size] {
            let mut cache_bits_best = 0;

            if !vp8l_get_backward_references(
                width,
                height,
                argb,
                quality,
                low_effort,
                sub_config.lz77,
                cache_bits_init,
                sub_config.do_no_cache,
                hash_chain,
                &mut refs_array[..],
                &mut cache_bits_best,
            ) {
                webp_encoding_set_error(pic, WebPEncodingError::OutOfMemory);
                return false;
            }

            for i_cache in 0..if sub_config.do_no_cache { 2 } else { 1 } {
                let cache_bits_tmp = if i_cache == 0 { cache_bits_best } else { 0 };
                // Speed-up: no need to study the no-cache case if it was already studied
                // in i_cache == 0.
                if i_cache == 1 && cache_bits_best == 0 {
                    break;
                }

                // Reset the bit writer for this iteration.
                bw.reset(&bw_init);

                // Build histogram image and symbols from backward references.
                let mut histogram_image =
                    vp8l_allocate_histogram_set(histogram_image_xysize as i32, cache_bits_tmp);
                let mut tmp_histo = Box::new(VP8LHistogram::new(cache_bits_tmp));

                if !vp8l_get_histo_image_symbols(
                    width,
                    height,
                    &refs_array[i_cache],
                    quality,
                    low_effort,
                    histogram_bits,
                    cache_bits_tmp,
                    &mut histogram_image,
                    &mut tmp_histo,
                    &mut histogram_symbols,
                ) {
                    webp_encoding_set_error(pic, WebPEncodingError::OutOfMemory);
                    return false;
                }
                // Create Huffman bit lengths and codes for each histogram image.
                let mut histogram_image_size = histogram_image.size as usize;
                let bit_array_size = 5 * histogram_image_size;
                let mut huffman_codes = vec![HuffmanTreeCode::default(); bit_array_size];
                // Note: some histogram_image entries may point to tmp_histos[], so the
                // latter need to outlive the following call to
                // GetHuffBitLengthsAndCodes().
                if !get_huff_bit_lengths_and_codes(&mut histogram_image, &mut huffman_codes) {
                    webp_encoding_set_error(pic, WebPEncodingError::OutOfMemory);
                    return false;
                }
                // Free combined histograms.
                drop(histogram_image);

                // Free scratch histograms.
                drop(tmp_histo);

                // Color Cache parameters.
                if cache_bits_tmp > 0 {
                    bw.put_bits(1, 1);
                    bw.put_bits(cache_bits_tmp as u32, 4);
                } else {
                    bw.put_bits(0, 1);
                }

                // Huffman image + meta huffman.
                let write_histogram_image = histogram_image_size > 1;
                bw.put_bits(write_histogram_image as u32, 1);
                if write_histogram_image {
                    let mut histogram_argb = vec![0u32; histogram_image_xysize as usize];
                    let mut max_index = 0;
                    for (i, h) in histogram_argb.iter_mut().enumerate() {
                        let symbol_index = (histogram_symbols[i] & 0xffff) as usize;
                        *h = (symbol_index as u32) << 8;
                        if symbol_index >= max_index {
                            max_index = symbol_index + 1;
                        }
                    }
                    histogram_image_size = max_index;

                    bw.put_bits(histogram_bits as u32 - 2, 3);
                    if !encode_image_no_huffman(
                        bw,
                        &histogram_argb,
                        &mut hash_chain_histogram,
                        &mut refs_array[2..],
                        vp8l_sub_sample_size(width as u32, histogram_bits as u32) as i32,
                        vp8l_sub_sample_size(height as u32, histogram_bits as u32) as i32,
                        quality,
                        low_effort,
                        pic,
                    ) {
                        return false;
                    }
                }

                // Store Huffman codes.
                {
                    let mut max_tokens = 0;
                    // Find maximum number of symbols for the huffman tree-set.
                    for codes in &huffman_codes[..5 * histogram_image_size] {
                        if max_tokens < codes.num_symbols as usize {
                            max_tokens = codes.num_symbols as usize;
                        }
                    }
                    let mut tokens = vec![HuffmanTreeToken::default(); max_tokens];
                    for codes in huffman_codes[..5 * histogram_image_size].iter_mut() {
                        store_huffman_code(bw, &mut huff_tree, &mut tokens, codes);
                        clear_huffman_tree_if_only_one_symbol(codes);
                    }
                }
                // Store actual literals.
                let hdr_size_tmp = (bw.num_bytes() - init_byte_position) as i32;
                if !store_image_to_bit_mask(
                    bw,
                    width,
                    histogram_bits,
                    &refs_array[i_cache],
                    &histogram_symbols,
                    &huffman_codes,
                    pic,
                ) {
                    return false;
                }
                // Keep track of the smallest image so far.
                if bw.num_bytes() < bw_size_best {
                    bw_size_best = bw.num_bytes();
                    *cache_bits = cache_bits_tmp;
                    *hdr_size = hdr_size_tmp;
                    *data_size = (bw.num_bytes() - init_byte_position) as i32 - *hdr_size;
                    VP8LBitWriter::swap(bw, &mut bw_best);
                }
            }
        }
        VP8LBitWriter::swap(bw, &mut bw_best);
        true
    })();
    let _ = ok;

    vp8l_hash_chain_clear(&mut hash_chain_histogram);
    bw_best.wipe_out();
    pic.error_code.get() == WebPEncodingError::Ok
}

// -----------------------------------------------------------------------------
// Transforms

/// Translation of `ApplySubtractGreen()`.
fn apply_subtract_green(
    enc: &mut VP8LEncoder<'_>,
    width: i32,
    height: i32,
    bw: &mut VP8LBitWriter,
) {
    bw.put_bits(TRANSFORM_PRESENT, 1);
    bw.put_bits(VP8LImageTransformType::SubtractGreenTransform as u32, 2);
    vp8l_subtract_green_from_blue_and_red(&mut enc.argb[..(width * height) as usize]);
}

/// Translation of `ApplyPredictFilter()`.
fn apply_predict_filter(
    enc: &mut VP8LEncoder<'_>,
    width: i32,
    height: i32,
    quality: i32,
    low_effort: bool,
    used_subtract_green: bool,
    bw: &mut VP8LBitWriter,
) -> bool {
    let pred_bits = enc.transform_bits;
    let transform_width = vp8l_sub_sample_size(width as u32, pred_bits as u32) as i32;
    let transform_height = vp8l_sub_sample_size(height as u32, pred_bits as u32) as i32;
    // we disable near-lossless quantization if palette is used.
    let near_lossless_strength = if enc.use_palette {
        100
    } else {
        enc.config.near_lossless
    };

    if !vp8l_residual_image(
        width,
        height,
        pred_bits,
        low_effort,
        &mut enc.argb,
        &mut enc.argb_scratch,
        &mut enc.transform_data,
        near_lossless_strength,
        enc.config.exact != 0,
        used_subtract_green,
    ) {
        return false;
    }
    bw.put_bits(TRANSFORM_PRESENT, 1);
    bw.put_bits(VP8LImageTransformType::PredictorTransform as u32, 2);
    debug_assert!(pred_bits >= 2);
    bw.put_bits(pred_bits as u32 - 2, 3);
    encode_image_no_huffman(
        bw,
        &enc.transform_data,
        &mut enc.hash_chain,
        &mut enc.refs[..],
        transform_width,
        transform_height,
        quality,
        low_effort,
        enc.pic,
    )
}

/// Translation of `ApplyCrossColorFilter()`.
fn apply_cross_color_filter(
    enc: &mut VP8LEncoder<'_>,
    width: i32,
    height: i32,
    quality: i32,
    low_effort: bool,
    bw: &mut VP8LBitWriter,
) -> bool {
    let ccolor_transform_bits = enc.transform_bits;
    let transform_width = vp8l_sub_sample_size(width as u32, ccolor_transform_bits as u32) as i32;
    let transform_height = vp8l_sub_sample_size(height as u32, ccolor_transform_bits as u32) as i32;

    if !vp8l_color_space_transform(
        width,
        height,
        ccolor_transform_bits,
        quality,
        &mut enc.argb,
        &mut enc.transform_data,
    ) {
        return false;
    }
    bw.put_bits(TRANSFORM_PRESENT, 1);
    bw.put_bits(VP8LImageTransformType::CrossColorTransform as u32, 2);
    debug_assert!(ccolor_transform_bits >= 2);
    bw.put_bits(ccolor_transform_bits as u32 - 2, 3);
    encode_image_no_huffman(
        bw,
        &enc.transform_data,
        &mut enc.hash_chain,
        &mut enc.refs[..],
        transform_width,
        transform_height,
        quality,
        low_effort,
        enc.pic,
    )
}

// -----------------------------------------------------------------------------

/// Translation of `WriteRiffHeader()`.
fn write_riff_header(pic: &WebPPicture, riff_size: usize, vp8l_size: usize) -> bool {
    let mut riff: [u8; RIFF_HEADER_SIZE + CHUNK_HEADER_SIZE + VP8L_SIGNATURE_SIZE] = [
        b'R',
        b'I',
        b'F',
        b'F',
        0,
        0,
        0,
        0,
        b'W',
        b'E',
        b'B',
        b'P',
        b'V',
        b'P',
        b'8',
        b'L',
        0,
        0,
        0,
        0,
        VP8L_MAGIC_BYTE as u8,
    ];
    put_le32(&mut riff[TAG_SIZE..], riff_size as u32);
    put_le32(&mut riff[RIFF_HEADER_SIZE + TAG_SIZE..], vp8l_size as u32);
    webp_picture_write(pic, &riff)
}

/// Translation of `WriteImageSize()`.
fn write_image_size(pic: &WebPPicture, bw: &mut VP8LBitWriter) -> bool {
    let width = pic.width - 1;
    let height = pic.height - 1;
    debug_assert!(width < 16383 && height < 16383);

    bw.put_bits(width as u32, VP8L_IMAGE_SIZE_BITS);
    bw.put_bits(height as u32, VP8L_IMAGE_SIZE_BITS);
    !bw.error
}

/// Translation of `WriteRealAlphaAndVersion()`.
fn write_real_alpha_and_version(bw: &mut VP8LBitWriter, has_alpha: bool) -> bool {
    bw.put_bits(has_alpha as u32, 1);
    bw.put_bits(VP8L_VERSION, VP8L_VERSION_BITS);
    !bw.error
}

/// Translation of `WriteImage()`.
fn write_image(pic: &WebPPicture, bw: &mut VP8LBitWriter, coded_size: &mut usize) -> bool {
    let webpll_data = bw.finish().to_vec();
    let webpll_size = webpll_data.len();
    let vp8l_size = VP8L_SIGNATURE_SIZE + webpll_size;
    let pad = vp8l_size & 1;
    let riff_size = TAG_SIZE + CHUNK_HEADER_SIZE + vp8l_size + pad;
    *coded_size = 0;

    if bw.error {
        return webp_encoding_set_error(pic, WebPEncodingError::OutOfMemory);
    }

    if !write_riff_header(pic, riff_size, vp8l_size) || !webp_picture_write(pic, &webpll_data) {
        return webp_encoding_set_error(pic, WebPEncodingError::BadWrite);
    }

    if pad != 0 {
        let pad_byte = [0u8; 1];
        if !webp_picture_write(pic, &pad_byte) {
            return webp_encoding_set_error(pic, WebPEncodingError::BadWrite);
        }
    }
    *coded_size = CHUNK_HEADER_SIZE + riff_size;
    true
}

// -----------------------------------------------------------------------------

/// Translation of `ClearTransformBuffer()`.
fn clear_transform_buffer(enc: &mut VP8LEncoder<'_>) {
    enc.argb = Vec::new();
    enc.argb_scratch = Vec::new();
    enc.transform_data = Vec::new();
}

/// Allocates the memory for argb (W x H) buffer, 2 rows of context for
/// prediction and transform data.
/// Flags influencing the memory allocated:
///  enc->transform_bits_
///  enc->use_predict_, enc->use_cross_color_
/// Translation of `AllocateTransformBuffer()` (the buffers are kept when
/// big enough, as upstream keeps its block, which marks the image as gone
/// when it is reallocated).
fn allocate_transform_buffer(enc: &mut VP8LEncoder<'_>, width: i32, height: i32) -> bool {
    let image_size = width as u64 * height as u64;
    // VP8LResidualImage needs room for 2 scanlines of uint32 pixels with an extra
    // pixel in each, plus 2 regular scanlines of bytes.
    // TODO(skal): Clean up by using arithmetic in bytes instead of words.
    let argb_scratch_size: u64 = if enc.use_predict {
        (width as u64 + 1) * 2 + (width as u64 * 2 + 4 - 1) / 4
    } else {
        0
    };
    let transform_data_size: u64 = if enc.use_predict || enc.use_cross_color {
        vp8l_sub_sample_size(width as u32, enc.transform_bits as u32) as u64
            * vp8l_sub_sample_size(height as u32, enc.transform_bits as u32) as u64
    } else {
        0
    };
    if (enc.argb.len() as u64) < image_size
        || (enc.argb_scratch.len() as u64) < argb_scratch_size
        || (enc.transform_data.len() as u64) < transform_data_size
    {
        clear_transform_buffer(enc);
        enc.argb = vec![0; image_size as usize];
        enc.argb_scratch = vec![0; argb_scratch_size as usize];
        enc.transform_data = vec![0; transform_data_size as usize];
        enc.argb_content = VP8LEncoderARGBContent::None;
    }

    enc.current_width = width;
    true
}

/// Translation of `MakeInputImageCopy()`.
fn make_input_image_copy(enc: &mut VP8LEncoder<'_>) -> bool {
    let picture = enc.pic;
    let width = picture.width as usize;
    let height = picture.height as usize;

    if !allocate_transform_buffer(enc, width as i32, height as i32) {
        return false;
    }
    if enc.argb_content == VP8LEncoderARGBContent::Argb {
        return true;
    }

    {
        let stride = picture.argb_stride as usize;
        for y in 0..height {
            enc.argb[y * width..(y + 1) * width]
                .copy_from_slice(&picture.argb[y * stride..y * stride + width]);
        }
    }
    enc.argb_content = VP8LEncoderARGBContent::Argb;
    debug_assert!(enc.current_width == width as i32);
    true
}

// -----------------------------------------------------------------------------

const APPLY_PALETTE_GREEDY_MAX: usize = 4;

/// Translation of `SearchColorGreedy()`.
fn search_color_greedy(palette: &[u32], _palette_size: usize, color: u32) -> u32 {
    debug_assert!(_palette_size < APPLY_PALETTE_GREEDY_MAX);
    if color == palette[0] {
        return 0;
    }
    if color == palette[1] {
        return 1;
    }
    if color == palette[2] {
        return 2;
    }
    3
}

/// Translation of `ApplyPaletteHash0()`.
fn apply_palette_hash0(color: u32) -> u32 {
    // Focus on the green color.
    (color >> 8) & 0xff
}

const PALETTE_INV_SIZE_BITS: u32 = 11;
const PALETTE_INV_SIZE: usize = 1 << PALETTE_INV_SIZE_BITS;

/// Translation of `ApplyPaletteHash1()`.
fn apply_palette_hash1(color: u32) -> u32 {
    // Forget about alpha.
    (((color & 0x00ffffff) as u64).wrapping_mul(4222244071) as u32) >> (32 - PALETTE_INV_SIZE_BITS)
}

/// Translation of `ApplyPaletteHash2()`.
fn apply_palette_hash2(color: u32) -> u32 {
    // Forget about alpha.
    (((color & 0x00ffffff) as u64).wrapping_mul((1u64 << 31) - 1) as u32)
        >> (32 - PALETTE_INV_SIZE_BITS)
}

/// Remap argb values in src[] to packed palettes entries in dst[]
/// using 'row' as a temporary buffer of size 'width'.
/// We assume that all src[] values have a corresponding entry in the
/// palette. Translation of `ApplyPalette()` (with `APPLY_PALETTE_FOR()`;
/// `src` is never `dst` here, as the delta palette is not translated).
#[allow(clippy::too_many_arguments)]
fn apply_palette(
    src: &[u32],
    src_stride: usize,
    dst: &mut [u32],
    dst_stride: usize,
    palette: &[u32],
    palette_size: usize,
    width: usize,
    height: usize,
    xbits: i32,
) -> bool {
    // TODO(skal): this tmp buffer is not needed if VP8LBundleColorMap() can be
    // made to work in-place.
    let mut tmp_row = vec![0u8; width];

    // Use 1 pixel cache for ARGB pixels.
    let mut apply_palette_for = |color_index: &dyn Fn(u32) -> u32| {
        let mut prev_pix = palette[0];
        let mut prev_idx = 0u32;
        for y in 0..height {
            for x in 0..width {
                let pix = src[y * src_stride + x];
                if pix != prev_pix {
                    prev_idx = color_index(pix);
                    prev_pix = pix;
                }
                tmp_row[x] = prev_idx as u8;
            }
            vp8l_bundle_color_map(&tmp_row, width, xbits, &mut dst[y * dst_stride..]);
        }
    };

    if palette_size < APPLY_PALETTE_GREEDY_MAX {
        apply_palette_for(&|pix| search_color_greedy(palette, palette_size, pix));
    } else {
        let mut buffer = [0u16; PALETTE_INV_SIZE];
        let hash_functions: [fn(u32) -> u32; 3] = [
            apply_palette_hash0,
            apply_palette_hash1,
            apply_palette_hash2,
        ];

        // Try to find a perfect hash function able to go from a color to an index
        // within 1 << PALETTE_INV_SIZE_BITS in order to build a hash map to go
        // from color to index in palette.
        let mut i = 0;
        while i < 3 {
            let mut use_lut = true;
            // Set each element in buffer to max uint16_t.
            buffer.fill(0xffff);
            for (j, &p) in palette[..palette_size].iter().enumerate() {
                let ind = hash_functions[i](p) as usize;
                if buffer[ind] != 0xffff {
                    use_lut = false;
                    break;
                } else {
                    buffer[ind] = j as u16;
                }
            }
            if use_lut {
                break;
            }
            i += 1;
        }

        if i == 0 {
            apply_palette_for(&|pix| buffer[apply_palette_hash0(pix) as usize] as u32);
        } else if i == 1 {
            apply_palette_for(&|pix| buffer[apply_palette_hash1(pix) as usize] as u32);
        } else if i == 2 {
            apply_palette_for(&|pix| buffer[apply_palette_hash2(pix) as usize] as u32);
        } else {
            let mut idx_map = [0u32; MAX_PALETTE_SIZE];
            let mut palette_sorted = [0u32; MAX_PALETTE_SIZE];
            prepare_map_to_palette(palette, palette_size, &mut palette_sorted, &mut idx_map);
            apply_palette_for(&|pix| {
                idx_map[search_color_no_idx(&palette_sorted, pix, palette_size)]
            });
        }
    }
    true
}

/// Note: Expects "enc->palette_" to be set properly. Translation of
/// `MapImageFromPalette()`.
fn map_image_from_palette(enc: &mut VP8LEncoder<'_>, in_place: bool) -> bool {
    let pic = enc.pic;
    let width = pic.width;
    let height = pic.height;
    let palette_size = enc.palette_size;
    debug_assert!(!in_place, "the delta palette is not translated");

    // Replace each input pixel by corresponding palette index.
    // This is done line by line.
    let xbits = if palette_size <= 4 {
        if palette_size <= 2 {
            3
        } else {
            2
        }
    } else if palette_size <= 16 {
        1
    } else {
        0
    };

    if !allocate_transform_buffer(
        enc,
        vp8l_sub_sample_size(width as u32, xbits as u32) as i32,
        height,
    ) {
        return false;
    }
    let palette = enc.palette;
    let current_width = enc.current_width as usize;
    if !apply_palette(
        &pic.argb,
        pic.argb_stride as usize,
        &mut enc.argb,
        current_width,
        &palette,
        palette_size as usize,
        width as usize,
        height as usize,
        xbits,
    ) {
        return false;
    }
    enc.argb_content = VP8LEncoderARGBContent::Palette;
    true
}

/// Save palette_[] to bitstream. Translation of `EncodePalette()`.
fn encode_palette(bw: &mut VP8LBitWriter, low_effort: bool, enc: &mut VP8LEncoder<'_>) -> bool {
    let mut tmp_palette = [0u32; MAX_PALETTE_SIZE];
    let palette_size = enc.palette_size as usize;
    let palette = &enc.palette;
    bw.put_bits(TRANSFORM_PRESENT, 1);
    bw.put_bits(VP8LImageTransformType::ColorIndexingTransform as u32, 2);
    debug_assert!((1..=MAX_PALETTE_SIZE).contains(&palette_size));
    bw.put_bits(palette_size as u32 - 1, 8);
    for i in (1..palette_size).rev() {
        tmp_palette[i] = vp8l_sub_pixels(palette[i], palette[i - 1]);
    }
    tmp_palette[0] = palette[0];
    encode_image_no_huffman(
        bw,
        &tmp_palette[..palette_size],
        &mut enc.hash_chain,
        &mut enc.refs[..],
        palette_size as i32,
        1,
        /*quality=*/ 20,
        low_effort,
        enc.pic,
    )
}

// -----------------------------------------------------------------------------
// VP8LEncoder

/// Translation of `VP8LEncoderNew()`.
fn vp8l_encoder_new<'p>(config: &'p WebPConfig, picture: &'p WebPPicture) -> VP8LEncoder<'p> {
    VP8LEncoder {
        config,
        pic: picture,
        argb: Vec::new(),
        argb_content: VP8LEncoderARGBContent::None,
        argb_scratch: Vec::new(),
        transform_data: Vec::new(),
        current_width: 0,
        histo_bits: 0,
        transform_bits: 0,
        cache_bits: 0,
        use_cross_color: false,
        use_subtract_green: false,
        use_predict: false,
        use_palette: false,
        palette_size: 0,
        palette: [0; MAX_PALETTE_SIZE],
        palette_sorted: [0; MAX_PALETTE_SIZE],
        refs: Default::default(),
        hash_chain: VP8LHashChain::default(),
    }
}

/// Translation of `VP8LEncoderDelete()`.
fn vp8l_encoder_delete(enc: &mut VP8LEncoder<'_>) {
    vp8l_hash_chain_clear(&mut enc.hash_chain);
    for r in enc.refs.iter_mut() {
        vp8l_backward_refs_clear(r);
    }
    clear_transform_buffer(enc);
}

// -----------------------------------------------------------------------------
// Main call

/// Translation of `EncodeStreamHook()` (the worker of the main thread,
/// with its `StreamEncodeContext` as arguments).
#[allow(clippy::too_many_arguments)]
fn encode_stream_hook(
    config: &WebPConfig,
    picture: &WebPPicture,
    bw: &mut VP8LBitWriter,
    enc: &mut VP8LEncoder<'_>,
    use_cache: bool,
    crunch_configs: &[CrunchConfig],
    red_and_blue_always_zero: bool,
) -> bool {
    let num_crunch_configs = crunch_configs.len();
    let quality = config.quality as i32;
    let low_effort = config.method == 0;
    let height = picture.height;
    let byte_position = bw.num_bytes();
    let mut hdr_size = 0;
    let mut data_size = 0;
    let use_delta_palette = false;
    let mut best_size = !0usize;
    let bw_init = bw.state();
    let mut bw_best = VP8LBitWriter::new(0);

    let _ = (|| {
        if num_crunch_configs > 1 && !bw.clone_into(&mut bw_best) {
            webp_encoding_set_error(picture, WebPEncodingError::OutOfMemory);
            return false;
        }

        for crunch_config in crunch_configs {
            let entropy_idx = crunch_config.entropy_idx;
            enc.use_palette =
                entropy_idx == EntropyIx::Palette || entropy_idx == EntropyIx::PaletteAndSpatial;
            enc.use_subtract_green =
                entropy_idx == EntropyIx::SubGreen || entropy_idx == EntropyIx::SpatialSubGreen;
            enc.use_predict = entropy_idx == EntropyIx::Spatial
                || entropy_idx == EntropyIx::SpatialSubGreen
                || entropy_idx == EntropyIx::PaletteAndSpatial;
            // When using a palette, R/B==0, hence no need to test for cross-color.
            if low_effort || enc.use_palette {
                enc.use_cross_color = false;
            } else {
                enc.use_cross_color = if red_and_blue_always_zero {
                    false
                } else {
                    enc.use_predict
                };
            }
            // Reset any parameter in the encoder that is set in the previous iteration.
            enc.cache_bits = 0;
            vp8l_backward_refs_clear(&mut enc.refs[0]);
            vp8l_backward_refs_clear(&mut enc.refs[1]);

            // Apply near-lossless preprocessing.
            debug_assert!(
                config.near_lossless == 100 || enc.use_palette || enc.use_predict,
                "near-lossless is not translated"
            );
            enc.argb_content = VP8LEncoderARGBContent::None;

            // Encode palette
            if enc.use_palette {
                if crunch_config.palette_sorting_type == PaletteSorting::SortedDefault {
                    // Nothing to do, we have already sorted the palette.
                    let n = enc.palette_size as usize;
                    let sorted = enc.palette_sorted;
                    enc.palette[..n].copy_from_slice(&sorted[..n]);
                } else {
                    debug_assert!(
                        crunch_config.palette_sorting_type == PaletteSorting::MinimizeDelta
                    );
                    let sorted = enc.palette_sorted;
                    palette_sort_minimize_deltas(
                        &sorted,
                        enc.palette_size as usize,
                        &mut enc.palette,
                    );
                }
                if !encode_palette(bw, low_effort, enc) {
                    return false;
                }
                if !map_image_from_palette(enc, use_delta_palette) {
                    return false;
                }
                // If using a color cache, do not have it bigger than the number of
                // colors.
                if use_cache && enc.palette_size < (1 << MAX_COLOR_CACHE_BITS) {
                    enc.cache_bits = bits_log2_floor(enc.palette_size as u32) + 1;
                }
            }
            if !use_delta_palette {
                // In case image is not packed.
                if enc.argb_content != VP8LEncoderARGBContent::Palette
                    && !make_input_image_copy(enc)
                {
                    return false;
                }

                // -----------------------------------------------------------------------
                // Apply transforms and write transform data.

                if enc.use_subtract_green {
                    let w = enc.current_width;
                    apply_subtract_green(enc, w, height, bw);
                }

                if enc.use_predict {
                    let w = enc.current_width;
                    let used_subtract_green = enc.use_subtract_green;
                    if !apply_predict_filter(
                        enc,
                        w,
                        height,
                        quality,
                        low_effort,
                        used_subtract_green,
                        bw,
                    ) {
                        return false;
                    }
                }

                if enc.use_cross_color {
                    let w = enc.current_width;
                    if !apply_cross_color_filter(enc, w, height, quality, low_effort, bw) {
                        return false;
                    }
                }
            }

            bw.put_bits((TRANSFORM_PRESENT == 0) as u32, 1); // No more transforms.

            // -------------------------------------------------------------------------
            // Encode and write the transformed image.
            {
                let current_width = enc.current_width;
                let histo_bits = enc.histo_bits;
                let argb = std::mem::take(&mut enc.argb);
                let ok = encode_image_internal(
                    bw,
                    &argb,
                    &mut enc.hash_chain,
                    &mut enc.refs,
                    current_width,
                    height,
                    quality,
                    low_effort,
                    use_cache,
                    crunch_config,
                    &mut enc.cache_bits,
                    histo_bits,
                    byte_position,
                    &mut hdr_size,
                    &mut data_size,
                    picture,
                );
                enc.argb = argb;
                if !ok {
                    return false;
                }
            }

            // If we are better than what we already have.
            if bw.num_bytes() < best_size {
                best_size = bw.num_bytes();
                // Store the BitWriter.
                VP8LBitWriter::swap(bw, &mut bw_best);
            }
            // Reset the bit writer for the following iteration if any.
            if num_crunch_configs > 1 {
                bw.reset(&bw_init);
            }
        }
        VP8LBitWriter::swap(&mut bw_best, bw);
        true
    })();

    bw_best.wipe_out();
    // The hook should return false in case of error.
    picture.error_code.get() == WebPEncodingError::Ok
}

/// Encodes the main image stream using the supplied bit writer.
/// If 'use_cache' is false, disables the use of color cache.
/// Returns false in case of error (stored in picture->error_code).
/// Translation of `VP8LEncodeStream()` (on the calling thread:
/// `thread_level` 0).
pub(crate) fn vp8l_encode_stream(
    config: &WebPConfig,
    picture: &WebPPicture,
    bw_main: &mut VP8LBitWriter,
    use_cache: bool,
) -> bool {
    let mut enc_main = vp8l_encoder_new(config, picture);
    let mut crunch_configs = Vec::new();
    let mut red_and_blue_always_zero = false;
    debug_assert!(
        config.thread_level == 0,
        "the side thread is not translated"
    );

    // Analyze image (entropy, num_palettes etc)
    if !encoder_analyze(
        &mut enc_main,
        &mut crunch_configs,
        &mut red_and_blue_always_zero,
    ) || !encoder_init(&mut enc_main)
    {
        webp_encoding_set_error(picture, WebPEncodingError::OutOfMemory);
    } else {
        // Execute the main thread.
        encode_stream_hook(
            config,
            picture,
            bw_main,
            &mut enc_main,
            use_cache,
            &crunch_configs,
            red_and_blue_always_zero,
        );
    }

    vp8l_encoder_delete(&mut enc_main);
    picture.error_code.get() == WebPEncodingError::Ok
}

/// Encodes the picture. Returns 0 if config or picture is NULL or picture
/// doesn't have valid argb input. Translation of `VP8LEncodeImage()`
/// (without the statistics and progress report).
pub(crate) fn vp8l_encode_image(config: &WebPConfig, picture: &WebPPicture) -> bool {
    let mut coded_size = 0usize;

    if picture.argb.is_empty() {
        return webp_encoding_set_error(picture, WebPEncodingError::NullParameter);
    }

    let width = picture.width as usize;
    let height = picture.height as usize;
    // Initialize BitWriter with size corresponding to 16 bpp to photo images and
    // 8 bpp for graphical images.
    let initial_size = if config.image_hint == WebPImageHint::Graph {
        width * height
    } else {
        width * height * 2
    };
    let mut bw = VP8LBitWriter::new(initial_size);

    let _ = (|| {
        // Write image size.
        if !write_image_size(picture, &mut bw) {
            webp_encoding_set_error(picture, WebPEncodingError::OutOfMemory);
            return false;
        }

        let has_alpha = webp_picture_has_transparency(picture);
        // Write the non-trivial Alpha flag and lossless version.
        if !write_real_alpha_and_version(&mut bw, has_alpha) {
            webp_encoding_set_error(picture, WebPEncodingError::OutOfMemory);
            return false;
        }

        // Encode main image stream.
        if !vp8l_encode_stream(config, picture, &mut bw, true /*use_cache*/) {
            return false;
        }

        // Finish the RIFF chunk.
        if !write_image(picture, &mut bw, &mut coded_size) {
            return false;
        }
        true
    })();

    if bw.error {
        webp_encoding_set_error(picture, WebPEncodingError::OutOfMemory);
    }
    bw.wipe_out();
    picture.error_code.get() == WebPEncodingError::Ok
}
