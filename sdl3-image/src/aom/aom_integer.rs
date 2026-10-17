// Rust translation of aom/src/aom_integer.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The unsigned LEB128 helpers of the OBU writer.

const K_MAXIMUM_LEB128_SIZE: usize = 8;
const K_LEB128_BYTE_MASK: u8 = 0x7f; // Binary: 01111111

// Disallow values larger than 32-bits to ensure consistent behavior on 32 and
// 64 bit targets: value is typically used to determine buffer allocation size
// when decoded.
const K_MAXIMUM_LEB128_VALUE: u64 = u32::MAX as u64;

pub(crate) fn aom_uleb_size_in_bytes(mut value: u64) -> usize {
    let mut size = 0;
    loop {
        size += 1;
        value >>= 7;
        if value == 0 {
            break;
        }
    }
    size
}

/// Translation of `aom_uleb_decode()`: `Some((value, length))` for 0.
pub(crate) fn aom_uleb_decode(buffer: &[u8]) -> Option<(u64, usize)> {
    let mut value: u64 = 0;
    let mut i = 0;
    while i < K_MAXIMUM_LEB128_SIZE && i < buffer.len() {
        let decoded_byte = buffer[i] & K_LEB128_BYTE_MASK;
        value |= (decoded_byte as u64) << (i * 7);
        if (buffer[i] >> 7) == 0 {
            // Fail on values larger than 32-bits to ensure consistent behavior on
            // 32 and 64 bit targets: value is typically used to determine buffer
            // allocation size.
            if value > u32::MAX as u64 {
                return None;
            }

            return Some((value, i + 1));
        }
        i += 1;
    }

    // If we get here, either the buffer/value pointers were invalid,
    // or we ran over the available space
    None
}

/// Translation of `aom_uleb_encode()`: the coded size for 0, `None` for -1.
pub(crate) fn aom_uleb_encode(mut value: u64, coded_value: &mut [u8]) -> Option<usize> {
    let leb_size = aom_uleb_size_in_bytes(value);
    if value > K_MAXIMUM_LEB128_VALUE
        || leb_size > K_MAXIMUM_LEB128_SIZE
        || leb_size > coded_value.len()
    {
        return None;
    }

    for byte_out in coded_value.iter_mut().take(leb_size) {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;

        if value != 0 {
            byte |= 0x80; // Signal that more bytes follow.
        }

        *byte_out = byte;
    }

    Some(leb_size)
}
