// Rust translation of the allocation helpers of src/mem.c and src/mem.h
// from dav1d (https://code.videolan.org/videolan/dav1d, at the revision
// SDL_image's external/dav1d pins).
// Copyright © 2018, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Fallible allocation. Upstream's buffer pools (`Dav1dMemPool`) and
//! aligned allocations are plain vectors here; what upstream does on
//! `malloc()` failure (return `ENOMEM`) is what happens when a vector
//! can't be reserved, so sizes that come from the bitstream never abort.

/// Allocate `n` copies of `val`, failing instead of aborting when the
/// memory can't be reserved (`dav1d_alloc_aligned()`/`malloc()` returning
/// `NULL`).
pub(crate) fn try_vec<T: Clone>(val: T, n: usize) -> Result<Vec<T>, ()> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| ())?;
    v.resize(n, val);
    Ok(v)
}
