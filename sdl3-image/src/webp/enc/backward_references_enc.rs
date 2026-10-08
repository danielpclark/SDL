// Rust translation of src/enc/backward_references_enc.c and
// backward_references_enc.h from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2012 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The backward references (LZ77, RLE, the "box" LZ77 and the color
//! cache) of the lossless encoder, and its hash chains.
//!
//! `VP8LBackwardRefs` is a `Vec` of `PixOrCopy` here: upstream's chain of
//! fixed-size blocks (with a free list for recycling them) holds the same
//! sequence, and its cursor walks it in the same order. The low-effort
//! path (method 0) is not translated: SDL_image encodes with method 4.

use crate::webp::dsp::lossless_enc::{vp8l_prefix_encode, vp8l_vector_mismatch};
use crate::webp::enc::backward_references_cost_enc::vp8l_backward_references_trace_backwards;
use crate::webp::enc::histogram_enc::{
    vp8l_histogram_create, vp8l_histogram_estimate_bits, VP8LHistogram,
};
use crate::webp::utils::color_cache_utils::{
    vp8l_color_cache_clear, vp8l_color_cache_init, vp8l_hash_pix, VP8LColorCache,
};

/// The maximum allowed limit is 11.
pub(crate) const MAX_COLOR_CACHE_BITS: i32 = 10;

// -----------------------------------------------------------------------------
// PixOrCopy

/// Translation of `enum Mode`.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) enum Mode {
    #[default]
    Literal,
    CacheIdx,
    Copy,
}

/// Translation of `PixOrCopy`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct PixOrCopy {
    pub(crate) mode: Mode,
    pub(crate) len: u16,
    pub(crate) argb_or_distance: u32,
}

impl PixOrCopy {
    /// Translation of `PixOrCopyCreateCopy()`.
    pub(crate) fn create_copy(distance: u32, len: u16) -> PixOrCopy {
        PixOrCopy {
            mode: Mode::Copy,
            argb_or_distance: distance,
            len,
        }
    }

    /// Translation of `PixOrCopyCreateCacheIdx()`.
    pub(crate) fn create_cache_idx(idx: i32) -> PixOrCopy {
        debug_assert!(idx >= 0);
        debug_assert!(idx < (1 << MAX_COLOR_CACHE_BITS));
        PixOrCopy {
            mode: Mode::CacheIdx,
            argb_or_distance: idx as u32,
            len: 1,
        }
    }

    /// Translation of `PixOrCopyCreateLiteral()`.
    pub(crate) fn create_literal(argb: u32) -> PixOrCopy {
        PixOrCopy {
            mode: Mode::Literal,
            argb_or_distance: argb,
            len: 1,
        }
    }

    /// Translation of `PixOrCopyIsLiteral()`.
    pub(crate) fn is_literal(&self) -> bool {
        self.mode == Mode::Literal
    }

    /// Translation of `PixOrCopyIsCacheIdx()`.
    pub(crate) fn is_cache_idx(&self) -> bool {
        self.mode == Mode::CacheIdx
    }

    /// Translation of `PixOrCopyIsCopy()`.
    pub(crate) fn is_copy(&self) -> bool {
        self.mode == Mode::Copy
    }

    /// Translation of `PixOrCopyLiteral()`.
    pub(crate) fn literal(&self, component: i32) -> u32 {
        debug_assert!(self.mode == Mode::Literal);
        (self.argb_or_distance >> (component * 8)) & 0xff
    }

    /// Translation of `PixOrCopyLength()`.
    pub(crate) fn length(&self) -> u32 {
        self.len as u32
    }

    /// Translation of `PixOrCopyCacheIdx()`.
    pub(crate) fn cache_idx(&self) -> u32 {
        debug_assert!(self.mode == Mode::CacheIdx);
        debug_assert!(self.argb_or_distance < (1u32 << MAX_COLOR_CACHE_BITS));
        self.argb_or_distance
    }

    /// Translation of `PixOrCopyDistance()`.
    pub(crate) fn distance(&self) -> u32 {
        debug_assert!(self.mode == Mode::Copy);
        self.argb_or_distance
    }
}

// -----------------------------------------------------------------------------
// VP8LHashChain

const HASH_BITS: u32 = 18;
const HASH_SIZE: usize = 1 << HASH_BITS;

// If you change this, you need MAX_LENGTH_BITS + WINDOW_SIZE_BITS <= 32 as
// it is used in VP8LHashChain.
const MAX_LENGTH_BITS: u32 = 12;
const WINDOW_SIZE_BITS: u32 = 20;
/// We want the max value to be attainable and stored in MAX_LENGTH_BITS
/// bits.
pub(crate) const MAX_LENGTH: i32 = (1 << MAX_LENGTH_BITS) - 1;

/// Translation of `VP8LHashChain`.
#[derive(Default)]
pub(crate) struct VP8LHashChain {
    /// The 20 most significant bits contain the offset at which the best
    /// match is found. These 20 bits are the limit defined by
    /// GetWindowSizeForHashChain (through WINDOW_SIZE = 1<<20).
    /// The lower 12 bits contain the length of the match. The 12 bit limit
    /// is defined in MaxFindCopyLength with MAX_LENGTH=4096.
    pub(crate) offset_length: Vec<u32>,
    /// This is the maximum size of the hash_chain that can be constructed.
    /// Typically this is the pixel count (width x height) for a given image.
    pub(crate) size: i32,
}

impl VP8LHashChain {
    /// Translation of `VP8LHashChainFindOffset()`.
    pub(crate) fn find_offset(&self, base_position: usize) -> i32 {
        (self.offset_length[base_position] >> MAX_LENGTH_BITS) as i32
    }

    /// Translation of `VP8LHashChainFindLength()`.
    pub(crate) fn find_length(&self, base_position: usize) -> i32 {
        (self.offset_length[base_position] & ((1u32 << MAX_LENGTH_BITS) - 1)) as i32
    }

    /// Translation of `VP8LHashChainFindCopy()`: (offset, length).
    pub(crate) fn find_copy(&self, base_position: usize) -> (i32, i32) {
        (
            self.find_offset(base_position),
            self.find_length(base_position),
        )
    }
}

// -----------------------------------------------------------------------------
// VP8LBackwardRefs (block-based backward-references storage)

/// maximum number of reference blocks the image will be segmented into
pub(crate) const MAX_REFS_BLOCK_PER_IMAGE: i32 = 16;

/// Container for blocks chain. Translation of `VP8LBackwardRefs` (a
/// `Vec`: see the module documentation).
#[derive(Clone, Default)]
pub(crate) struct VP8LBackwardRefs {
    /// list of currently used blocks
    pub(crate) refs: Vec<PixOrCopy>,
    /// set to true if some memory error occurred
    pub(crate) error: bool,
}

// -----------------------------------------------------------------------------
// Main entry points

/// Translation of `kLZ77Standard`.
pub(crate) const K_LZ77_STANDARD: i32 = 1;
/// Translation of `kLZ77RLE`.
pub(crate) const K_LZ77_RLE: i32 = 2;
/// Translation of `kLZ77Box`.
pub(crate) const K_LZ77_BOX: i32 = 4;

/// 1M window (4M bytes) minus 120 special codes for short distances.
const WINDOW_SIZE: i32 = (1 << WINDOW_SIZE_BITS) - 120;

/// Minimum number of pixels for which it is cheaper to encode a
/// distance + length instead of each pixel as a literal.
const MIN_LENGTH: i32 = 4;

// -----------------------------------------------------------------------------

const PLANE_TO_CODE_LUT: [u8; 128] = [
    96, 73, 55, 39, 23, 13, 5, 1, 255, 255, 255, 255, 255, 255, 255, 255, //
    101, 78, 58, 42, 26, 16, 8, 2, 0, 3, 9, 17, 27, 43, 59, 79, //
    102, 86, 62, 46, 32, 20, 10, 6, 4, 7, 11, 21, 33, 47, 63, 87, //
    105, 90, 70, 52, 37, 28, 18, 14, 12, 15, 19, 29, 38, 53, 71, 91, //
    110, 99, 82, 66, 48, 35, 30, 24, 22, 25, 31, 36, 49, 67, 83, 100, //
    115, 108, 94, 76, 64, 50, 44, 40, 34, 41, 45, 51, 65, 77, 95, 109, //
    118, 113, 103, 92, 80, 68, 60, 56, 54, 57, 61, 69, 81, 93, 104, 114, //
    119, 116, 111, 106, 97, 88, 84, 74, 72, 75, 85, 89, 98, 107, 112, 117,
];

/// Translation of `VP8LDistanceToPlaneCode()`.
pub(crate) fn vp8l_distance_to_plane_code(xsize: i32, dist: i32) -> i32 {
    let yoffset = dist / xsize;
    let xoffset = dist - yoffset * xsize;
    if xoffset <= 8 && yoffset < 8 {
        return PLANE_TO_CODE_LUT[(yoffset * 16 + 8 - xoffset) as usize] as i32 + 1;
    } else if xoffset > xsize - 8 && yoffset < 7 {
        return PLANE_TO_CODE_LUT[((yoffset + 1) * 16 + 8 + (xsize - xoffset)) as usize] as i32 + 1;
    }
    dist + 120
}

/// Returns the exact index where array1 and array2 are different. For an
/// index inferior or equal to best_len_match, the return value just has to
/// be strictly inferior to best_len_match. The current behavior is to
/// return 0 if this index is best_len_match, and the index itself
/// otherwise. If no two elements are the same, it returns max_limit.
/// Translation of `FindMatchLength()` (the arrays are `argb` at `a1` and
/// `a2`).
fn find_match_length(
    argb: &[u32],
    a1: usize,
    a2: usize,
    best_len_match: usize,
    max_limit: usize,
) -> i32 {
    // Before 'expensive' linear match, check if the two arrays match at the
    // current best length index.
    if argb[a1 + best_len_match] != argb[a2 + best_len_match] {
        return 0;
    }

    vp8l_vector_mismatch(&argb[a1..], &argb[a2..], max_limit) as i32
}

// -----------------------------------------------------------------------------
//  VP8LBackwardRefs

/// Translation of `VP8LClearBackwardRefs()`.
pub(crate) fn vp8l_clear_backward_refs(refs: &mut VP8LBackwardRefs) {
    refs.refs.clear();
}

/// Release memory for backward references. Translation of
/// `VP8LBackwardRefsClear()`.
pub(crate) fn vp8l_backward_refs_clear(refs: &mut VP8LBackwardRefs) {
    refs.refs = Vec::new();
}

/// Swaps the content of two VP8LBackwardRefs. Translation of
/// `BackwardRefsSwap()`.
fn backward_refs_swap(refs1: &mut VP8LBackwardRefs, refs2: &mut VP8LBackwardRefs) {
    std::mem::swap(refs1, refs2);
}

/// Initialize the object. 'block_size' is the common block size to store
/// references (typically, width * height / MAX_REFS_BLOCK_PER_IMAGE).
/// Translation of `VP8LBackwardRefsInit()`.
pub(crate) fn vp8l_backward_refs_init(refs: &mut VP8LBackwardRefs, _block_size: i32) {
    *refs = VP8LBackwardRefs::default();
}

/// Return 1 on success, 0 on error. Translation of `BackwardRefsClone()`.
fn backward_refs_clone(from: &VP8LBackwardRefs, to: &mut VP8LBackwardRefs) -> bool {
    vp8l_clear_backward_refs(to);
    to.refs.extend_from_slice(&from.refs);
    true
}

/// Translation of `VP8LBackwardRefsCursorAdd()`.
pub(crate) fn vp8l_backward_refs_cursor_add(refs: &mut VP8LBackwardRefs, v: PixOrCopy) {
    refs.refs.push(v);
}

// -----------------------------------------------------------------------------
// Hash chains

/// Must be called first, to set size. Translation of
/// `VP8LHashChainInit()`.
pub(crate) fn vp8l_hash_chain_init(p: &mut VP8LHashChain, size: i32) -> bool {
    debug_assert!(p.size == 0);
    debug_assert!(p.offset_length.is_empty());
    debug_assert!(size > 0);
    p.offset_length = vec![0; size as usize];
    p.size = size;

    true
}

/// release memory. Translation of `VP8LHashChainClear()`.
pub(crate) fn vp8l_hash_chain_clear(p: &mut VP8LHashChain) {
    p.offset_length = Vec::new();
    p.size = 0;
}

// -----------------------------------------------------------------------------

const K_HASH_MULTIPLIER_HI: u32 = 0xc6a4a793;
const K_HASH_MULTIPLIER_LO: u32 = 0x5bd1e996;

/// Translation of `GetPixPairHash64()`.
fn get_pix_pair_hash64(argb: &[u32]) -> u32 {
    let mut key = argb[1].wrapping_mul(K_HASH_MULTIPLIER_HI);
    key = key.wrapping_add(argb[0].wrapping_mul(K_HASH_MULTIPLIER_LO));
    key >> (32 - HASH_BITS)
}

/// Returns the maximum number of hash chain lookups to do for a
/// given compression quality. Return value in range [8, 86].
/// Translation of `GetMaxItersForQuality()`.
fn get_max_iters_for_quality(quality: i32) -> i32 {
    8 + (quality * quality) / 128
}

/// Translation of `GetWindowSizeForHashChain()`.
fn get_window_size_for_hash_chain(quality: i32, xsize: i32) -> i32 {
    let max_window_size = if quality > 75 {
        WINDOW_SIZE
    } else if quality > 50 {
        xsize << 8
    } else if quality > 25 {
        xsize << 6
    } else {
        xsize << 4
    };
    debug_assert!(xsize > 0);
    if max_window_size > WINDOW_SIZE {
        WINDOW_SIZE
    } else {
        max_window_size
    }
}

/// Translation of `MaxFindCopyLength()`.
fn max_find_copy_length(len: i32) -> i32 {
    if len < MAX_LENGTH {
        len
    } else {
        MAX_LENGTH
    }
}

/// Pre-compute the best matches for argb. Translation of
/// `VP8LHashChainFill()` (the progress report is not translated).
pub(crate) fn vp8l_hash_chain_fill(
    p: &mut VP8LHashChain,
    quality: i32,
    argb: &[u32],
    xsize: i32,
    ysize: i32,
    low_effort: bool,
) -> bool {
    let size = xsize * ysize;
    let iter_max = get_max_iters_for_quality(quality);
    let window_size = get_window_size_for_hash_chain(quality, xsize) as u32;
    debug_assert!(size > 0);
    debug_assert!(p.size != 0);
    debug_assert!(!p.offset_length.is_empty());

    if size <= 2 {
        p.offset_length[0] = 0;
        p.offset_length[size as usize - 1] = 0;
        return true;
    }

    let mut hash_to_first_index = vec![-1i32; HASH_SIZE];

    // Temporarily use the p->offset_length_ as a hash chain.
    // (the int32_t chain is the same array, reinterpreted)
    let chain = &mut p.offset_length;
    let get_chain = |chain: &[u32], pos: usize| chain[pos] as i32;

    // Fill the chain linking pixels with the same hash.
    let mut argb_comp = argb[0] == argb[1];
    let mut pos: i32 = 0;
    while pos < size - 2 {
        let argb_comp_next = argb[pos as usize + 1] == argb[pos as usize + 2];
        if argb_comp && argb_comp_next {
            // Consecutive pixels with the same color will share the same hash.
            // We therefore use a different hash: the color and its repetition
            // length.
            let mut tmp = [0u32; 2];
            let mut len: u32 = 1;
            tmp[0] = argb[pos as usize];
            // Figure out how far the pixels are the same.
            // The last pixel has a different 64 bit hash, as its next pixel does
            // not have the same color, so we just need to get to the last pixel equal
            // to its follower.
            while pos + len as i32 + 2 < size
                && argb[(pos as u32 + len + 2) as usize] == argb[pos as usize]
            {
                len += 1;
            }
            if len > MAX_LENGTH as u32 {
                // Skip the pixels that match for distance=1 and length>MAX_LENGTH
                // because they are linked to their predecessor and we automatically
                // check that in the main for loop below. Skipping means setting no
                // predecessor in the chain, hence -1.
                let n = (len - MAX_LENGTH as u32) as usize;
                chain[pos as usize..pos as usize + n].fill(0xffffffff);
                pos += (len - MAX_LENGTH as u32) as i32;
                len = MAX_LENGTH as u32;
            }
            // Process the rest of the hash chain.
            while len != 0 {
                tmp[1] = len;
                len -= 1;
                let hash_code = get_pix_pair_hash64(&tmp) as usize;
                chain[pos as usize] = hash_to_first_index[hash_code] as u32;
                hash_to_first_index[hash_code] = pos;
                pos += 1;
            }
            argb_comp = false;
        } else {
            // Just move one pixel forward.
            let hash_code = get_pix_pair_hash64(&argb[pos as usize..]) as usize;
            chain[pos as usize] = hash_to_first_index[hash_code] as u32;
            hash_to_first_index[hash_code] = pos;
            pos += 1;
            argb_comp = argb_comp_next;
        }
    }
    // Process the penultimate pixel.
    chain[pos as usize] =
        hash_to_first_index[get_pix_pair_hash64(&argb[pos as usize..]) as usize] as u32;

    drop(hash_to_first_index);

    // Find the best match interval at each pixel, defined by an offset to the
    // pixel and a length. The right-most pixel cannot match anything to the right
    // (hence a best length of 0) and the left-most pixel nothing to the left
    // (hence an offset of 0).
    debug_assert!(size > 2);
    let offset_length = chain;
    offset_length[0] = 0;
    offset_length[size as usize - 1] = 0;
    let mut base_position = (size - 2) as u32;
    while base_position > 0 {
        let max_len = max_find_copy_length(size - 1 - base_position as i32);
        let argb_start = base_position as usize;
        let mut iter = iter_max;
        let mut best_length: i32 = 0;
        let mut best_distance: u32 = 0;
        let min_pos: i32 = if base_position > window_size {
            (base_position - window_size) as i32
        } else {
            0
        };
        let length_max = if max_len < 256 { max_len } else { 256 };

        let mut pos = get_chain(offset_length, base_position as usize);
        if !low_effort {
            // Heuristic: use the comparison with the above line as an initialization.
            if base_position >= xsize as u32 {
                let curr_length = find_match_length(
                    argb,
                    argb_start - xsize as usize,
                    argb_start,
                    best_length as usize,
                    max_len as usize,
                );
                if curr_length > best_length {
                    best_length = curr_length;
                    best_distance = xsize as u32;
                }
                iter -= 1;
            }
            // Heuristic: compare to the previous pixel.
            let curr_length = find_match_length(
                argb,
                argb_start - 1,
                argb_start,
                best_length as usize,
                max_len as usize,
            );
            if curr_length > best_length {
                best_length = curr_length;
                best_distance = 1;
            }
            iter -= 1;
            // Skip the for loop if we already have the maximum.
            if best_length == MAX_LENGTH {
                pos = min_pos - 1;
            }
        }
        let mut best_argb = argb[argb_start + best_length as usize];

        while pos >= min_pos && {
            iter -= 1;
            iter != 0
        } {
            debug_assert!(base_position > pos as u32);

            if argb[pos as usize + best_length as usize] != best_argb {
                pos = get_chain(offset_length, pos as usize);
                continue;
            }

            let curr_length =
                vp8l_vector_mismatch(&argb[pos as usize..], &argb[argb_start..], max_len as usize)
                    as i32;
            if best_length < curr_length {
                best_length = curr_length;
                best_distance = base_position - pos as u32;
                best_argb = argb[argb_start + best_length as usize];
                // Stop if we have reached a good enough length.
                if best_length >= length_max {
                    break;
                }
            }
            pos = get_chain(offset_length, pos as usize);
        }
        // We have the best match but in case the two intervals continue matching
        // to the left, we have the best matches for the left-extended pixels.
        let mut max_base_position = base_position;
        loop {
            debug_assert!(best_length <= MAX_LENGTH);
            debug_assert!(best_distance <= WINDOW_SIZE as u32);
            offset_length[base_position as usize] =
                (best_distance << MAX_LENGTH_BITS) | best_length as u32;
            base_position -= 1;
            // Stop if we don't have a match or if we are out of bounds.
            if best_distance == 0 || base_position == 0 {
                break;
            }
            // Stop if we cannot extend the matching intervals to the left.
            if base_position < best_distance
                || argb[(base_position - best_distance) as usize] != argb[base_position as usize]
            {
                break;
            }
            // Stop if we are matching at its limit because there could be a closer
            // matching interval with the same maximum length. Then again, if the
            // matching interval is as close as possible (best_distance == 1), we will
            // never find anything better so let's continue.
            if best_length == MAX_LENGTH
                && best_distance != 1
                && base_position + (MAX_LENGTH as u32) < max_base_position
            {
                break;
            }
            if best_length < MAX_LENGTH {
                best_length += 1;
                max_base_position = base_position;
            }
        }
    }

    true
}

/// Translation of `AddSingleLiteral()`.
fn add_single_literal(
    pixel: u32,
    use_color_cache: bool,
    hashers: &mut VP8LColorCache,
    refs: &mut VP8LBackwardRefs,
) {
    let v = if use_color_cache {
        let key = hashers.get_index(pixel);
        if hashers.lookup(key) == pixel {
            PixOrCopy::create_cache_idx(key as i32)
        } else {
            hashers.set(key, pixel);
            PixOrCopy::create_literal(pixel)
        }
    } else {
        PixOrCopy::create_literal(pixel)
    };
    vp8l_backward_refs_cursor_add(refs, v);
}

/// Translation of `BackwardReferencesRle()`.
fn backward_references_rle(
    xsize: i32,
    ysize: i32,
    argb: &[u32],
    cache_bits: i32,
    refs: &mut VP8LBackwardRefs,
) -> bool {
    let pix_count = (xsize * ysize) as usize;
    let use_color_cache = cache_bits > 0;
    let mut hashers = VP8LColorCache::default();

    if use_color_cache && !vp8l_color_cache_init(&mut hashers, cache_bits) {
        return false;
    }
    vp8l_clear_backward_refs(refs);
    // Add first pixel as literal.
    add_single_literal(argb[0], use_color_cache, &mut hashers, refs);
    let mut i = 1usize;
    while i < pix_count {
        let max_len = max_find_copy_length((pix_count - i) as i32) as usize;
        let rle_len = find_match_length(argb, i, i - 1, 0, max_len);
        let prev_row_len = if i < xsize as usize {
            0
        } else {
            find_match_length(argb, i, i - xsize as usize, 0, max_len)
        };
        if rle_len >= prev_row_len && rle_len >= MIN_LENGTH {
            vp8l_backward_refs_cursor_add(refs, PixOrCopy::create_copy(1, rle_len as u16));
            // We don't need to update the color cache here since it is always the
            // same pixel being copied, and that does not change the color cache
            // state.
            i += rle_len as usize;
        } else if prev_row_len >= MIN_LENGTH {
            vp8l_backward_refs_cursor_add(
                refs,
                PixOrCopy::create_copy(xsize as u32, prev_row_len as u16),
            );
            if use_color_cache {
                for k in 0..prev_row_len as usize {
                    hashers.insert(argb[i + k]);
                }
            }
            i += prev_row_len as usize;
        } else {
            add_single_literal(argb[i], use_color_cache, &mut hashers, refs);
            i += 1;
        }
    }
    if use_color_cache {
        vp8l_color_cache_clear(&mut hashers);
    }
    !refs.error
}

/// Translation of `BackwardReferencesLz77()`.
fn backward_references_lz77(
    xsize: i32,
    ysize: i32,
    argb: &[u32],
    cache_bits: i32,
    hash_chain: &VP8LHashChain,
    refs: &mut VP8LBackwardRefs,
) -> bool {
    let mut i_last_check: i32 = -1;
    let use_color_cache = cache_bits > 0;
    let pix_count = xsize * ysize;
    let mut hashers = VP8LColorCache::default();

    if use_color_cache && !vp8l_color_cache_init(&mut hashers, cache_bits) {
        return false;
    }
    vp8l_clear_backward_refs(refs);
    let mut i: i32 = 0;
    while i < pix_count {
        // Alternative#1: Code the pixels starting at 'i' using backward reference.
        let (offset, mut len) = hash_chain.find_copy(i as usize);
        if len >= MIN_LENGTH {
            let len_ini = len;
            let mut max_reach = 0;
            let j_max = if i + len_ini >= pix_count {
                pix_count - 1
            } else {
                i + len_ini
            };
            // Only start from what we have not checked already.
            i_last_check = if i > i_last_check { i } else { i_last_check };
            // We know the best match for the current pixel but we try to find the
            // best matches for the current pixel AND the next one combined.
            // The naive method would use the intervals:
            // [i,i+len) + [i+len, length of best match at i+len)
            // while we check if we can use:
            // [i,j) (where j<=i+len) + [j, length of best match at j)
            for j in i_last_check + 1..=j_max {
                let len_j = hash_chain.find_length(j as usize);
                let reach = j + if len_j >= MIN_LENGTH { len_j } else { 1 }; // 1 for single literal.
                if reach > max_reach {
                    len = j - i;
                    max_reach = reach;
                    if max_reach >= pix_count {
                        break;
                    }
                }
            }
        } else {
            len = 1;
        }
        // Go with literal or backward reference.
        debug_assert!(len > 0);
        if len == 1 {
            add_single_literal(argb[i as usize], use_color_cache, &mut hashers, refs);
        } else {
            vp8l_backward_refs_cursor_add(refs, PixOrCopy::create_copy(offset as u32, len as u16));
            if use_color_cache {
                for j in i..i + len {
                    hashers.insert(argb[j as usize]);
                }
            }
        }
        i += len;
    }

    let ok = !refs.error;
    if use_color_cache {
        vp8l_color_cache_clear(&mut hashers);
    }
    ok
}

/// Compute an LZ77 by forcing matches to happen within a given distance
/// cost. We therefore limit the algorithm to the lowest 32 values in the
/// PlaneCode definition.
const WINDOW_OFFSETS_SIZE_MAX: usize = 32;

/// Translation of `BackwardReferencesLz77Box()`.
fn backward_references_lz77_box(
    xsize: i32,
    ysize: i32,
    argb: &[u32],
    cache_bits: i32,
    hash_chain_best: &VP8LHashChain,
    hash_chain: &mut VP8LHashChain,
    refs: &mut VP8LBackwardRefs,
) -> bool {
    let pix_count = (xsize * ysize) as usize;
    let mut window_offsets = [0i32; WINDOW_OFFSETS_SIZE_MAX];
    let mut window_offsets_new = [0i32; WINDOW_OFFSETS_SIZE_MAX];
    let mut window_offsets_size = 0usize;
    let mut window_offsets_new_size = 0usize;
    let mut counts_ini = vec![0u16; pix_count];
    let mut best_offset_prev: i32 = -1;
    let mut best_length_prev: i32 = -1;

    // counts[i] counts how many times a pixel is repeated starting at position i.
    // (counts[1] past the last pixel is counts_ini[pix_count - 1])
    {
        let mut i = pix_count as isize - 2;
        counts_ini[pix_count - 1] = 1;
        while i >= 0 {
            let iu = i as usize;
            if argb[iu] == argb[iu + 1] {
                // Max out the counts to MAX_LENGTH.
                counts_ini[iu] =
                    counts_ini[iu + 1] + (counts_ini[iu + 1] != MAX_LENGTH as u16) as u16;
            } else {
                counts_ini[iu] = 1;
            }
            i -= 1;
        }
    }

    // Figure out the window offsets around a pixel. They are stored in a
    // spiraling order around the pixel as defined by VP8LDistanceToPlaneCode.
    {
        for y in 0..=6 {
            for x in -6..=6 {
                let offset = y * xsize + x;
                // Ignore offsets that bring us after the pixel.
                if offset <= 0 {
                    continue;
                }
                let plane_code = vp8l_distance_to_plane_code(xsize, offset) - 1;
                if plane_code >= WINDOW_OFFSETS_SIZE_MAX as i32 {
                    continue;
                }
                window_offsets[plane_code as usize] = offset;
            }
        }
        // For narrow images, not all plane codes are reached, so remove those.
        for i in 0..WINDOW_OFFSETS_SIZE_MAX {
            if window_offsets[i] == 0 {
                continue;
            }
            window_offsets[window_offsets_size] = window_offsets[i];
            window_offsets_size += 1;
        }
        // Given a pixel P, find the offsets that reach pixels unreachable from P-1
        // with any of the offsets in window_offsets[].
        for i in 0..window_offsets_size {
            let mut is_reachable = false;
            let mut j = 0;
            while j < window_offsets_size && !is_reachable {
                is_reachable |= window_offsets[i] == window_offsets[j] + 1;
                j += 1;
            }
            if !is_reachable {
                window_offsets_new[window_offsets_new_size] = window_offsets[i];
                window_offsets_new_size += 1;
            }
        }
    }

    hash_chain.offset_length[0] = 0;
    for i in 1..pix_count {
        let mut best_length = hash_chain_best.find_length(i);
        let mut best_offset = 0;
        let mut do_compute = true;

        if best_length >= MAX_LENGTH {
            // Do not recompute the best match if we already have a maximal one in the
            // window.
            best_offset = hash_chain_best.find_offset(i);
            for &w in &window_offsets[..window_offsets_size] {
                if best_offset == w {
                    do_compute = false;
                    break;
                }
            }
        }
        if do_compute {
            // Figure out if we should use the offset/length from the previous pixel
            // as an initial guess and therefore only inspect the offsets in
            // window_offsets_new[].
            let use_prev = best_length_prev > 1 && best_length_prev < MAX_LENGTH;
            let num_ind = if use_prev {
                window_offsets_new_size
            } else {
                window_offsets_size
            };
            best_length = if use_prev { best_length_prev - 1 } else { 0 };
            best_offset = if use_prev { best_offset_prev } else { 0 };
            // Find the longest match in a window around the pixel.
            for ind in 0..num_ind {
                let mut curr_length: i32 = 0;
                let mut j = i;
                let w = if use_prev {
                    window_offsets_new[ind]
                } else {
                    window_offsets[ind]
                };
                let mut j_offset = i as isize - w as isize;
                if j_offset < 0 || argb[j_offset as usize] != argb[i] {
                    continue;
                }
                // The longest match is the sum of how many times each pixel is
                // repeated.
                loop {
                    let counts_j_offset = counts_ini[j_offset as usize] as i32;
                    let counts_j = counts_ini[j] as i32;
                    if counts_j_offset != counts_j {
                        curr_length += if counts_j_offset < counts_j {
                            counts_j_offset
                        } else {
                            counts_j
                        };
                        break;
                    }
                    // The same color is repeated counts_pos times at j_offset and j.
                    curr_length += counts_j_offset;
                    j_offset += counts_j_offset as isize;
                    j += counts_j_offset as usize;
                    if !(curr_length <= MAX_LENGTH
                        && j < pix_count
                        && argb[j_offset as usize] == argb[j])
                    {
                        break;
                    }
                }
                if best_length < curr_length {
                    best_offset = w;
                    if curr_length >= MAX_LENGTH {
                        best_length = MAX_LENGTH;
                        break;
                    } else {
                        best_length = curr_length;
                    }
                }
            }
        }

        debug_assert!(i + best_length as usize <= pix_count);
        debug_assert!(best_length <= MAX_LENGTH);
        if best_length <= MIN_LENGTH {
            hash_chain.offset_length[i] = 0;
            best_offset_prev = 0;
            best_length_prev = 0;
        } else {
            hash_chain.offset_length[i] =
                ((best_offset as u32) << MAX_LENGTH_BITS) | best_length as u32;
            best_offset_prev = best_offset;
            best_length_prev = best_length;
        }
    }
    hash_chain.offset_length[0] = 0;
    drop(counts_ini);

    backward_references_lz77(xsize, ysize, argb, cache_bits, hash_chain, refs)
}

// -----------------------------------------------------------------------------

/// Translation of `BackwardReferences2DLocality()`.
fn backward_references_2d_locality(xsize: i32, refs: &mut VP8LBackwardRefs) {
    for v in refs.refs.iter_mut() {
        if v.is_copy() {
            let dist = v.argb_or_distance as i32;
            let transformed_dist = vp8l_distance_to_plane_code(xsize, dist);
            v.argb_or_distance = transformed_dist as u32;
        }
    }
}

const NUM_LITERAL_CODES: usize = 256;
const NUM_LENGTH_CODES: usize = 24;

/// Evaluate optimal cache bits for the local color cache.
/// The input *best_cache_bits sets the maximum cache bits to use (passing
/// 0 implies disabling the local color cache). The local color cache is
/// also disabled for the lower (<= 25) quality.
/// Returns 0 in case of memory error. Translation of
/// `CalculateBestCacheSize()`.
fn calculate_best_cache_size(
    argb: &[u32],
    quality: i32,
    refs: &VP8LBackwardRefs,
    best_cache_bits: &mut i32,
) -> bool {
    let cache_bits_max = if quality <= 25 { 0 } else { *best_cache_bits };
    let mut entropy_min = MAX_ENTROPY;
    let mut hashers: Vec<VP8LColorCache> = Vec::new();
    let mut histos: Vec<VP8LHistogram> = Vec::new();
    let mut argb_pos = 0usize;

    debug_assert!((0..=MAX_COLOR_CACHE_BITS).contains(&cache_bits_max));

    if cache_bits_max == 0 {
        *best_cache_bits = 0;
        // Local color cache is disabled.
        return true;
    }

    // Allocate data.
    for i in 0..=cache_bits_max {
        histos.push(VP8LHistogram::new(i));
        let mut cc = VP8LColorCache::default();
        if i != 0 && !vp8l_color_cache_init(&mut cc, i) {
            return false;
        }
        hashers.push(cc);
    }

    // Find the cache_bits giving the lowest entropy. The search is done in a
    // brute-force way as the function (entropy w.r.t cache_bits) can be
    // anything in practice.
    for v in &refs.refs {
        if v.is_literal() {
            let pix = argb[argb_pos];
            argb_pos += 1;
            let a = ((pix >> 24) & 0xff) as usize;
            let r = ((pix >> 16) & 0xff) as usize;
            let g = ((pix >> 8) & 0xff) as usize;
            let b = (pix & 0xff) as usize;
            // The keys of the caches can be derived from the longest one.
            let mut key = vp8l_hash_pix(pix, 32 - cache_bits_max);
            // Do not use the color cache for cache_bits = 0.
            histos[0].blue[b] += 1;
            histos[0].literal[g] += 1;
            histos[0].red[r] += 1;
            histos[0].alpha[a] += 1;
            // Deal with cache_bits > 0.
            let mut i = cache_bits_max as usize;
            while i >= 1 {
                if hashers[i].lookup(key as u32) == pix {
                    histos[i].literal[NUM_LITERAL_CODES + NUM_LENGTH_CODES + key] += 1;
                } else {
                    hashers[i].set(key as u32, pix);
                    histos[i].blue[b] += 1;
                    histos[i].literal[g] += 1;
                    histos[i].red[r] += 1;
                    histos[i].alpha[a] += 1;
                }
                i -= 1;
                key >>= 1;
            }
        } else {
            // We should compute the contribution of the (distance,length)
            // histograms but those are the same independently from the cache size.
            // As those constant contributions are in the end added to the other
            // histogram contributions, we can ignore them, except for the length
            // prefix that is part of the literal_ histogram.
            let mut len = v.length();
            let mut argb_prev = argb[argb_pos] ^ 0xffffffff;
            let (code, _, _) = vp8l_prefix_encode(len as i32);
            for h in histos.iter_mut() {
                h.literal[NUM_LITERAL_CODES + code as usize] += 1;
            }
            // Update the color caches.
            loop {
                if argb[argb_pos] != argb_prev {
                    // Efficiency: insert only if the color changes.
                    let mut key = vp8l_hash_pix(argb[argb_pos], 32 - cache_bits_max);
                    let mut i = cache_bits_max as usize;
                    while i >= 1 {
                        hashers[i].colors[key] = argb[argb_pos];
                        i -= 1;
                        key >>= 1;
                    }
                    argb_prev = argb[argb_pos];
                }
                argb_pos += 1;
                len -= 1;
                if len == 0 {
                    break;
                }
            }
        }
    }

    for (i, h) in histos.iter_mut().enumerate() {
        let entropy = vp8l_histogram_estimate_bits(h);
        if i == 0 || entropy < entropy_min {
            entropy_min = entropy;
            *best_cache_bits = i as i32;
        }
    }
    true
}

/// Update (in-place) backward references for specified cache_bits.
/// Translation of `BackwardRefsWithLocalCache()`.
fn backward_refs_with_local_cache(
    argb: &[u32],
    cache_bits: i32,
    refs: &mut VP8LBackwardRefs,
) -> bool {
    let mut pixel_index = 0usize;
    let mut hashers = VP8LColorCache::default();
    if !vp8l_color_cache_init(&mut hashers, cache_bits) {
        return false;
    }

    for v in refs.refs.iter_mut() {
        if v.is_literal() {
            let argb_literal = v.argb_or_distance;
            let ix = hashers.contains(argb_literal);
            if ix >= 0 {
                // hashers contains argb_literal
                *v = PixOrCopy::create_cache_idx(ix);
            } else {
                hashers.insert(argb_literal);
            }
            pixel_index += 1;
        } else {
            // refs was created without local cache, so it can not have cache indexes.
            debug_assert!(v.is_copy());
            for _ in 0..v.len {
                hashers.insert(argb[pixel_index]);
                pixel_index += 1;
            }
        }
    }
    vp8l_color_cache_clear(&mut hashers);
    true
}

const MAX_ENTROPY: f32 = 1e30;

/// Translation of `GetBackwardReferences()`: `refs` holds the best
/// references, the best ones without a cache if `do_no_cache`, and a
/// temporary.
#[allow(clippy::too_many_arguments)]
fn get_backward_references(
    width: i32,
    height: i32,
    argb: &[u32],
    quality: i32,
    mut lz77_types_to_try: i32,
    cache_bits_max: i32,
    do_no_cache: bool,
    hash_chain: &VP8LHashChain,
    refs: &mut [VP8LBackwardRefs],
    cache_bits_best: &mut i32,
) -> bool {
    // Index 0 is for a color cache, index 1 for no cache (if needed).
    let mut lz77_types_best = [0i32; 2];
    let mut bit_costs_best = [f32::MAX, f32::MAX];
    let mut hash_chain_box = VP8LHashChain::default();
    let tmp_index = if do_no_cache { 2 } else { 1 };
    let mut refs_tmp = std::mem::take(&mut refs[tmp_index]);

    let mut histo = VP8LHistogram::new(MAX_COLOR_CACHE_BITS);

    let status = (|| {
        let mut lz77_type = 1;
        while lz77_types_to_try != 0 {
            let mut bit_cost = 0.0f32;
            if (lz77_types_to_try & lz77_type) == 0 {
                lz77_types_to_try &= !lz77_type;
                lz77_type <<= 1;
                continue;
            }
            let res = match lz77_type {
                K_LZ77_RLE => backward_references_rle(width, height, argb, 0, &mut refs_tmp),
                K_LZ77_STANDARD => {
                    // Compute LZ77 with no cache (0 bits), as the ideal LZ77 with a color
                    // cache is not that different in practice.
                    backward_references_lz77(width, height, argb, 0, hash_chain, &mut refs_tmp)
                }
                K_LZ77_BOX => {
                    if !vp8l_hash_chain_init(&mut hash_chain_box, width * height) {
                        return false;
                    }
                    backward_references_lz77_box(
                        width,
                        height,
                        argb,
                        0,
                        hash_chain,
                        &mut hash_chain_box,
                        &mut refs_tmp,
                    )
                }
                _ => unreachable!(),
            };
            if !res {
                return false;
            }

            // Start with the no color cache case.
            for i in (0..=1).rev() {
                let mut cache_bits = if i == 1 { 0 } else { cache_bits_max };

                if i == 1 && !do_no_cache {
                    continue;
                }

                if i == 0 {
                    // Try with a color cache.
                    if !calculate_best_cache_size(argb, quality, &refs_tmp, &mut cache_bits) {
                        return false;
                    }
                    if cache_bits > 0
                        && !backward_refs_with_local_cache(argb, cache_bits, &mut refs_tmp)
                    {
                        return false;
                    }
                }

                if i == 0 && do_no_cache && cache_bits == 0 {
                    // No need to re-compute bit_cost as it was computed at i == 1.
                } else {
                    vp8l_histogram_create(&mut histo, &refs_tmp, cache_bits);
                    bit_cost = vp8l_histogram_estimate_bits(&mut histo);
                }

                if bit_cost < bit_costs_best[i] {
                    if i == 1 {
                        // Do not swap as the full cache analysis would have the wrong
                        // VP8LBackwardRefs to start with.
                        if !backward_refs_clone(&refs_tmp, &mut refs[1]) {
                            return false;
                        }
                    } else {
                        backward_refs_swap(&mut refs_tmp, &mut refs[0]);
                    }
                    bit_costs_best[i] = bit_cost;
                    lz77_types_best[i] = lz77_type;
                    if i == 0 {
                        *cache_bits_best = cache_bits;
                    }
                }
            }
            lz77_types_to_try &= !lz77_type;
            lz77_type <<= 1;
        }
        debug_assert!(lz77_types_best[0] > 0);
        debug_assert!(!do_no_cache || lz77_types_best[1] > 0);

        // Improve on simple LZ77 but only for high quality (TraceBackwards is
        // costly).
        for i in (0..=1).rev() {
            if i == 1 && !do_no_cache {
                continue;
            }
            if (lz77_types_best[i] == K_LZ77_STANDARD || lz77_types_best[i] == K_LZ77_BOX)
                && quality >= 25
            {
                let hash_chain_tmp = if lz77_types_best[i] == K_LZ77_STANDARD {
                    hash_chain
                } else {
                    &hash_chain_box
                };
                let cache_bits = if i == 1 { 0 } else { *cache_bits_best };
                if !vp8l_backward_references_trace_backwards(
                    width,
                    height,
                    argb,
                    cache_bits,
                    hash_chain_tmp,
                    &refs[i],
                    &mut refs_tmp,
                ) {
                    return false;
                }
                vp8l_histogram_create(&mut histo, &refs_tmp, cache_bits);
                let bit_cost_trace = vp8l_histogram_estimate_bits(&mut histo);
                if bit_cost_trace < bit_costs_best[i] {
                    backward_refs_swap(&mut refs_tmp, &mut refs[i]);
                }
            }

            backward_references_2d_locality(width, &mut refs[i]);

            if i == 1 && lz77_types_best[0] == lz77_types_best[1] && *cache_bits_best == 0 {
                // If the best cache size is 0 and we have the same best LZ77, just copy
                // the data over and stop here.
                let (r0, r1) = refs.split_at_mut(1);
                if !backward_refs_clone(&r1[0], &mut r0[0]) {
                    return false;
                }
                break;
            }
        }
        true
    })();

    refs[tmp_index] = refs_tmp;
    vp8l_hash_chain_clear(&mut hash_chain_box);
    status
}

/// Evaluates best possible backward references for specified quality.
/// The input cache_bits to 'VP8LGetBackwardReferences' sets the maximum
/// cache bits to use (passing 0 implies disabling the local color cache).
/// The optimal cache bits is evaluated and set for the *cache_bits_best
/// parameter with the matching refs_best.
/// If do_no_cache == 0, refs is an array of 2 values and the best
/// VP8LBackwardRefs is put in the first element.
/// If do_no_cache != 0, refs is an array of 3 values and the best
/// VP8LBackwardRefs is put in the first element, the best value with
/// no-cache in the second element.
/// In both cases, the last element is used as temporary internally.
/// Returns false in case of error. Translation of
/// `VP8LGetBackwardReferences()` (without its low-effort path and progress
/// report).
#[allow(clippy::too_many_arguments)]
pub(crate) fn vp8l_get_backward_references(
    width: i32,
    height: i32,
    argb: &[u32],
    quality: i32,
    low_effort: bool,
    lz77_types_to_try: i32,
    cache_bits_max: i32,
    do_no_cache: bool,
    hash_chain: &VP8LHashChain,
    refs: &mut [VP8LBackwardRefs],
    cache_bits_best: &mut i32,
) -> bool {
    debug_assert!(!low_effort, "the low-effort mode is not translated");
    get_backward_references(
        width,
        height,
        argb,
        quality,
        lz77_types_to_try,
        cache_bits_max,
        do_no_cache,
        hash_chain,
        refs,
        cache_bits_best,
    )
}
