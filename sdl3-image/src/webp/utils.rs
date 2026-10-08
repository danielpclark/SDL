// Rust translation of src/utils/utils.c and src/utils/utils.h from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it), the parts the decoder and encoder need.
// Copyright 2012 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Misc. common utility functions: the size-checked allocations, the
//! little-endian readers and writers, the pixel copies and the color
//! palette counter.

pub(crate) mod bit_reader_utils;
pub(crate) mod bit_writer_utils;
pub(crate) mod color_cache_utils;
pub(crate) mod filters_utils;
pub(crate) mod huffman_encode_utils;
pub(crate) mod huffman_utils;

use crate::webp::decode::MAX_PALETTE_SIZE;
use crate::webp::encode::WebPPicture;
use color_cache_utils::vp8l_hash_pix;

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

/// Store 16 bits in little-endian order. Translation of `PutLE16()`.
pub(crate) fn put_le16(data: &mut [u8], val: i32) {
    debug_assert!(val < (1 << 16));
    data[0] = val as u8;
    data[1] = (val >> 8) as u8;
}

/// Store 24 bits in little-endian order. Translation of `PutLE24()`.
pub(crate) fn put_le24(data: &mut [u8], val: i32) {
    debug_assert!(val < (1 << 24));
    put_le16(data, val & 0xffff);
    data[2] = (val >> 16) as u8;
}

/// Store 32 bits in little-endian order. Translation of `PutLE32()`.
pub(crate) fn put_le32(data: &mut [u8], val: u32) {
    put_le16(data, (val & 0xffff) as i32);
    put_le16(&mut data[2..], (val >> 16) as i32);
}

/// Returns (int)floor(log2(n)). n must be > 0. Translation of
/// `BitsLog2Floor()`.
pub(crate) fn bits_log2_floor(n: u32) -> i32 {
    31 ^ n.leading_zeros() as i32
}

//------------------------------------------------------------------------------
// Pixel copying.

/// Copy width x height pixels from 'src' to 'dst' honoring the strides.
/// Translation of `WebPCopyPlane()` (the planes start at the offsets).
#[allow(clippy::too_many_arguments)]
pub(crate) fn webp_copy_plane(
    src: &[u8],
    mut src_off: usize,
    src_stride: usize,
    dst: &mut [u8],
    mut dst_off: usize,
    dst_stride: usize,
    width: usize,
    height: usize,
) {
    for _ in 0..height {
        dst[dst_off..dst_off + width].copy_from_slice(&src[src_off..src_off + width]);
        src_off += src_stride;
        dst_off += dst_stride;
    }
}

/// Copy ARGB pixels from 'src' to 'dst' honoring strides. 'src' and 'dst'
/// are assumed to be already allocated and using ARGB data. Translation
/// of `WebPCopyPixels()`.
pub(crate) fn webp_copy_pixels(src: &WebPPicture, dst: &mut WebPPicture) {
    debug_assert!(src.width == dst.width && src.height == dst.height);
    debug_assert!(src.use_argb && dst.use_argb);
    let width = src.width as usize;
    let (src_stride, dst_stride) = (src.argb_stride as usize, dst.argb_stride as usize);
    for y in 0..src.height as usize {
        dst.argb[y * dst_stride..y * dst_stride + width]
            .copy_from_slice(&src.argb[y * src_stride..y * src_stride + width]);
    }
}

//------------------------------------------------------------------------------

const COLOR_HASH_SIZE: usize = MAX_PALETTE_SIZE * 4;
/// 32 - log2(COLOR_HASH_SIZE).
const COLOR_HASH_RIGHT_SHIFT: i32 = 22;

/// Returns count of unique colors in 'pic', assuming pic->use_argb is true.
/// If the unique color count is more than MAX_PALETTE_SIZE, returns
/// MAX_PALETTE_SIZE+1.
/// If 'palette' is not NULL and number of unique colors is less than or
/// equal to MAX_PALETTE_SIZE, also outputs the actual unique colors into
/// 'palette'.
/// Note: 'palette' is assumed to be an array already allocated with at
/// least MAX_PALETTE_SIZE elements. Translation of `WebPGetColorPalette()`.
pub(crate) fn webp_get_color_palette(pic: &WebPPicture, palette: Option<&mut [u32]>) -> i32 {
    let mut num_colors = 0;
    let mut in_use = [false; COLOR_HASH_SIZE];
    let mut colors = [0u32; COLOR_HASH_SIZE];
    let argb = &pic.argb;
    let width = pic.width as usize;
    let height = pic.height as usize;
    let mut last_pix = !argb[0]; // so we're sure that last_pix != argb[0]
    debug_assert!(pic.use_argb);

    let mut row = 0usize;
    for _ in 0..height {
        for &pix in &argb[row..row + width] {
            if pix == last_pix {
                continue;
            }
            last_pix = pix;
            let mut key = vp8l_hash_pix(last_pix, COLOR_HASH_RIGHT_SHIFT);
            loop {
                if !in_use[key] {
                    colors[key] = last_pix;
                    in_use[key] = true;
                    num_colors += 1;
                    if num_colors > MAX_PALETTE_SIZE as i32 {
                        return MAX_PALETTE_SIZE as i32 + 1; // Exact count not needed.
                    }
                    break;
                } else if colors[key] == last_pix {
                    break; // The color is already there.
                } else {
                    // Some other color sits here, so do linear conflict resolution.
                    key += 1;
                    key &= COLOR_HASH_SIZE - 1; // Key mask.
                }
            }
        }
        row += pic.argb_stride as usize;
    }

    if let Some(palette) = palette {
        // Fill the colors into palette.
        num_colors = 0;
        for i in 0..COLOR_HASH_SIZE {
            if in_use[i] {
                palette[num_colors as usize] = colors[i];
                num_colors += 1;
            }
        }
    }
    num_colors
}
