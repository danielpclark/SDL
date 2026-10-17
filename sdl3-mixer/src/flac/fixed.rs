// Rust translation of the decoder's part of src/libFLAC/fixed.c from
// libFLAC 1.3.4 (https://xiph.org/flac/, at the revision SDL_mixer's
// external/flac pins).
// Copyright (C) 2000-2009  Josh Coalson
// Copyright (C) 2011-2016  Xiph.Org Foundation
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The fixed predictors' signal restoration (the predictor choice and
//! residual computation are the encoder's and aren't translated).

/// `FLAC__fixed_restore_signal()`: restores `data_len` samples of `data`
/// from `start` on (C's `data` pointer), the `order` before it being the
/// warm-up. Signed overflow (undefined in C) wraps, as GCC's x86-64 code
/// does.
pub(crate) fn FLAC__fixed_restore_signal(
    residual: &[i32],
    data_len: u32,
    order: u32,
    data: &mut [i32],
    start: usize,
) {
    let idata_len = data_len as usize;
    let d = |i: usize| start + i;

    match order {
        0 => {
            data[start..start + idata_len].copy_from_slice(&residual[..idata_len]);
        }
        1 => {
            for i in 0..idata_len {
                data[d(i)] = residual[i].wrapping_add(data[d(i) - 1]);
            }
        }
        2 => {
            for i in 0..idata_len {
                data[d(i)] = residual[i]
                    .wrapping_add(2i32.wrapping_mul(data[d(i) - 1]))
                    .wrapping_sub(data[d(i) - 2]);
            }
        }
        3 => {
            for i in 0..idata_len {
                data[d(i)] = residual[i]
                    .wrapping_add(3i32.wrapping_mul(data[d(i) - 1]))
                    .wrapping_sub(3i32.wrapping_mul(data[d(i) - 2]))
                    .wrapping_add(data[d(i) - 3]);
            }
        }
        4 => {
            for i in 0..idata_len {
                data[d(i)] = residual[i]
                    .wrapping_add(4i32.wrapping_mul(data[d(i) - 1]))
                    .wrapping_sub(6i32.wrapping_mul(data[d(i) - 2]))
                    .wrapping_add(4i32.wrapping_mul(data[d(i) - 3]))
                    .wrapping_sub(data[d(i) - 4]);
            }
        }
        _ => {
            debug_assert!(false);
        }
    }
}
