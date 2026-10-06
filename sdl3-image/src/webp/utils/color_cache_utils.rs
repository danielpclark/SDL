// Rust translation of src/utils/color_cache_utils.c and
// color_cache_utils.h from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2012 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Color Cache for WebP Lossless.

use crate::webp::utils::safe_alloc;

/// Main color cache struct. Translation of `VP8LColorCache`.
#[derive(Default)]
pub(crate) struct VP8LColorCache {
    /// color entries
    pub(crate) colors: Vec<u32>,
    /// Hash shift: 32 - hash_bits_.
    pub(crate) hash_shift: i32,
    pub(crate) hash_bits: i32,
}

const K_HASH_MUL: u32 = 0x1e35a7bd;

/// Translation of `VP8LHashPix()`.
fn vp8l_hash_pix(argb: u32, shift: i32) -> usize {
    (argb.wrapping_mul(K_HASH_MUL) >> shift) as usize
}

impl VP8LColorCache {
    /// Translation of `VP8LColorCacheLookup()`.
    pub(crate) fn lookup(&self, key: u32) -> u32 {
        debug_assert!((key >> self.hash_bits) == 0);
        self.colors[key as usize]
    }

    /// Translation of `VP8LColorCacheInsert()`.
    pub(crate) fn insert(&mut self, argb: u32) {
        let key = vp8l_hash_pix(argb, self.hash_shift);
        self.colors[key] = argb;
    }
}

/// Initializes the color cache with 'hash_bits' bits for the keys.
/// Returns false in case of memory error. Translation of
/// `VP8LColorCacheInit()`.
pub(crate) fn vp8l_color_cache_init(color_cache: &mut VP8LColorCache, hash_bits: i32) -> bool {
    let hash_size = 1 << hash_bits;
    debug_assert!(hash_bits > 0);
    let Some(colors) = safe_alloc(hash_size as u64, 0u32) else {
        return false;
    };
    color_cache.colors = colors;
    color_cache.hash_shift = 32 - hash_bits;
    color_cache.hash_bits = hash_bits;
    true
}

/// Delete the memory associated to color cache. Translation of
/// `VP8LColorCacheClear()`.
pub(crate) fn vp8l_color_cache_clear(color_cache: &mut VP8LColorCache) {
    color_cache.colors = Vec::new();
}
