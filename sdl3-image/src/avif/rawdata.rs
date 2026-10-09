// Rust translation of src/rawdata.c from libavif
// (https://github.com/AOMediaCodec/libavif, at the revision SDL_image's
// external/libavif pins: libavif 1.1.1 with SDL's patches).
// Copyright 2019 Joe Drago. All rights reserved.
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `avifRWData`, which is a `Vec<u8>` here (`avifRWDataFree()` is clearing
//! it). Allocations are fallible, as upstream's: a size that can't be
//! reserved is `AVIF_RESULT_OUT_OF_MEMORY`.

use super::avif::AvifResult;

/// The avifRWData input must be zero-initialized before being manipulated with these functions.
/// If AVIF_RESULT_OUT_OF_MEMORY is returned, raw is left unchanged.
///
/// Translation of `avifRWDataRealloc()`: the new bytes are zeros
/// (upstream's are uninitialized).
pub(crate) fn avif_rw_data_realloc(raw: &mut Vec<u8>, new_size: usize) -> AvifResult {
    if raw.len() != new_size {
        if new_size == 0 {
            // avifAlloc(0) returns NULL, so handle the shrink-to-zero case by freeing the buffer.
            *raw = Vec::new();
            return AvifResult::Ok;
        }
        let mut new_data = Vec::new();
        if new_data.try_reserve_exact(new_size).is_err() {
            return AvifResult::OutOfMemory;
        }
        let keep = raw.len().min(new_size);
        new_data.extend_from_slice(&raw[..keep]);
        new_data.resize(new_size, 0);
        *raw = new_data;
    }
    AvifResult::Ok
}

/// Translation of `avifRWDataSet()`.
pub(crate) fn avif_rw_data_set(raw: &mut Vec<u8>, data: &[u8]) -> AvifResult {
    if !data.is_empty() {
        if raw.len() != data.len() {
            let mut new_data = Vec::new();
            if new_data.try_reserve_exact(data.len()).is_err() {
                return AvifResult::OutOfMemory;
            }
            *raw = new_data;
        } else {
            raw.clear();
        }
        raw.extend_from_slice(data);
    } else {
        *raw = Vec::new();
    }
    AvifResult::Ok
}
