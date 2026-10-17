// Rust translation of src/internal.c and the helpers of src/internal.h from
// opusfile 0.12 (https://opus-codec.org, at the revision SDL_mixer's
// external/opusfile pins).
// THE libopusfile SOURCE CODE IS (C) COPYRIGHT 2012-2020
// by the Xiph.Org Foundation and contributors https://xiph.org/
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! opusfile's internal helpers. `OP_ASSERT()` is off in SDL_mixer's build
//! (`OP_ENABLE_ASSERTIONS` isn't defined); it is `debug_assert!()` here.

pub(crate) const OP_INT64_MAX: i64 = i64::MAX;
pub(crate) const OP_INT64_MIN: i64 = -OP_INT64_MAX - 1;
pub(crate) const OP_INT32_MAX: i32 = i32::MAX;

/*Advance a file offset by the given amount, clamping against OP_INT64_MAX.
This is used to advance a known offset by things like OP_CHUNK_SIZE or
OP_PAGE_SIZE_MAX, while making sure to avoid signed overflow.
It assumes that both _offset and _amount are non-negative.*/
/// Translation of `OP_ADV_OFFSET()`.
pub(crate) fn op_adv_offset(offset: i64, amount: i64) -> i64 {
    offset.min(OP_INT64_MAX - amount) + amount
}

/*The maximum channel count for any mapping we'll actually decode.*/
pub(crate) const OP_NCHANNELS_MAX: usize = 8;

/*Initial state.*/
pub(crate) const OP_NOTOPEN: i32 = 0;
/*We've found the first Opus stream in the first link.*/
pub(crate) const OP_PARTOPEN: i32 = 1;
pub(crate) const OP_OPENED: i32 = 2;
/*We've found the first Opus stream in the current link.*/
pub(crate) const OP_STREAMSET: i32 = 3;
/*We've initialized the decoder for the chosen Opus stream in the current
link.*/
pub(crate) const OP_INITSET: i32 = 4;

/*A version of strncasecmp() that is guaranteed to only ignore the case of
ASCII characters.*/
/// Translation of `op_strncasecmp()`. Past their ends, `a` and `b` read as
/// NULs (the terminators of C's strings).
pub(crate) fn op_strncasecmp(a: &[u8], b: &[u8], n: i32) -> i32 {
    for i in 0..n.max(0) as usize {
        let mut a = a.get(i).copied().unwrap_or(0) as i8 as i32;
        let mut b = b.get(i).copied().unwrap_or(0) as i8 as i32;
        if (b'a' as i32..=b'z' as i32).contains(&a) {
            a -= (b'a' - b'A') as i32;
        }
        if (b'a' as i32..=b'z' as i32).contains(&b) {
            b -= (b'a' - b'A') as i32;
        }
        let d = a - b;
        if d != 0 {
            return d;
        }
    }
    0
}
