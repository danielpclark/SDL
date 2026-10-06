// Rust translation of src/utils/utils.c and src/utils/utils.h from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it), the parts the decoder needs.
// Copyright 2012 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Misc. common utility functions: the size-checked allocations and the
//! little-endian readers. (The pixel copies and the color palette counter
//! are the encoder's.)

pub(crate) mod bit_reader_utils;
pub(crate) mod color_cache_utils;
pub(crate) mod huffman_utils;

//------------------------------------------------------------------------------
// Memory allocation

/// This is the maximum memory amount that libwebp will ever try to
/// allocate. Translation of `WEBP_MAX_ALLOCABLE_MEMORY`.
pub(crate) const WEBP_MAX_ALLOCABLE_MEMORY: u64 = if usize::MAX as u64 > (1u64 << 34) {
    1u64 << 34
} else {
    // For 32-bit targets keep this below INT_MAX to avoid valgrind warnings.
    (1u64 << 31) - (1 << 16)
};

/// Whether `size` fits a `size_t`. Translation of `CheckSizeOverflow()`.
pub(crate) fn check_size_overflow(size: u64) -> bool {
    size == size as usize as u64
}

/// Returns false in case of overflow of nmemb * size.
/// Translation of `CheckSizeArgumentsOverflow()`.
fn check_size_arguments_overflow(nmemb: u64, size: usize) -> bool {
    let total_size = nmemb.wrapping_mul(size as u64);
    if nmemb == 0 {
        return true;
    }
    if size as u64 > WEBP_MAX_ALLOCABLE_MEMORY / nmemb {
        return false;
    }
    if !check_size_overflow(total_size) {
        return false;
    }
    true
}

/// size-checking safe malloc/calloc: verify that the requested size is not
/// too large, or return `None`. Translation of `WebPSafeMalloc()` and
/// `WebPSafeCalloc()`: the elements are `value` (calloc's zeroes, where
/// malloc's would be uninitialized), and a failed allocation is `None`
/// rather than an abort.
pub(crate) fn safe_alloc<T: Clone>(nmemb: u64, value: T) -> Option<Vec<T>> {
    if !check_size_arguments_overflow(nmemb, std::mem::size_of::<T>()) {
        return None;
    }
    let mut v = Vec::new();
    v.try_reserve_exact(nmemb as usize).ok()?;
    v.resize(nmemb as usize, value);
    Some(v)
}

//------------------------------------------------------------------------------
// Reading/writing data.

/// Read 16 bits stored in little-endian order. Translation of `GetLE16()`.
pub(crate) fn get_le16(data: &[u8]) -> i32 {
    (data[0] as i32) | ((data[1] as i32) << 8)
}

/// Read 24 bits stored in little-endian order. Translation of `GetLE24()`.
pub(crate) fn get_le24(data: &[u8]) -> i32 {
    get_le16(data) | ((data[2] as i32) << 16)
}

/// Read 32 bits stored in little-endian order. Translation of `GetLE32()`.
pub(crate) fn get_le32(data: &[u8]) -> u32 {
    get_le16(data) as u32 | ((get_le16(&data[2..]) as u32) << 16)
}

/// Returns (int)floor(log2(n)). n must be > 0. Translation of
/// `BitsLog2Floor()`.
pub(crate) fn bits_log2_floor(n: u32) -> i32 {
    31 ^ n.leading_zeros() as i32
}
