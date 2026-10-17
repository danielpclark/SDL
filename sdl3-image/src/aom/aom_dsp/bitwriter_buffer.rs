// Rust translation of aom_dsp/bitwriter_buffer.c and
// aom_dsp/bitwriter_buffer.h from aom (https://aomedia.googlesource.com/aom,
// at the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The raw bit writer of the OBU headers (the uvlc and subexponential
//! writers are not reached: no timing info or global motion parameters
//! are written).

const CHAR_BIT: u32 = 8;

/// Translation of `struct aom_write_bit_buffer`, over the bytes from its
/// `bit_buffer` on.
#[derive(Debug)]
pub(crate) struct AomWriteBitBuffer<'a> {
    pub(crate) bit_buffer: &'a mut [u8],
    pub(crate) bit_offset: u32,
}

pub(crate) fn aom_wb_is_byte_aligned(wb: &AomWriteBitBuffer<'_>) -> bool {
    wb.bit_offset % CHAR_BIT == 0
}

pub(crate) fn aom_wb_bytes_written(wb: &AomWriteBitBuffer<'_>) -> u32 {
    wb.bit_offset / CHAR_BIT + (wb.bit_offset % CHAR_BIT > 0) as u32
}

pub(crate) fn aom_wb_write_bit(wb: &mut AomWriteBitBuffer<'_>, bit: i32) {
    let off = wb.bit_offset as i32;
    let p = (off / CHAR_BIT as i32) as usize;
    let q = CHAR_BIT as i32 - 1 - off % CHAR_BIT as i32;
    if q == CHAR_BIT as i32 - 1 {
        // Zero next char and write bit
        wb.bit_buffer[p] = (bit << q) as u8;
    } else {
        wb.bit_buffer[p] &= !(1u8 << q);
        wb.bit_buffer[p] |= (bit << q) as u8;
    }
    wb.bit_offset = (off + 1) as u32;
}

pub(crate) fn aom_wb_overwrite_bit(wb: &mut AomWriteBitBuffer<'_>, bit: i32) {
    // Do not zero bytes but overwrite exisiting values
    let off = wb.bit_offset as i32;
    let p = (off / CHAR_BIT as i32) as usize;
    let q = CHAR_BIT as i32 - 1 - off % CHAR_BIT as i32;
    wb.bit_buffer[p] &= !(1u8 << q);
    wb.bit_buffer[p] |= (bit << q) as u8;
    wb.bit_offset = (off + 1) as u32;
}

pub(crate) fn aom_wb_write_literal(wb: &mut AomWriteBitBuffer<'_>, data: i32, bits: i32) {
    debug_assert!(bits <= 31);
    let mut bit = bits - 1;
    while bit >= 0 {
        aom_wb_write_bit(wb, (data >> bit) & 1);
        bit -= 1;
    }
}

pub(crate) fn aom_wb_overwrite_literal(wb: &mut AomWriteBitBuffer<'_>, data: i32, bits: i32) {
    let mut bit = bits - 1;
    while bit >= 0 {
        aom_wb_overwrite_bit(wb, (data >> bit) & 1);
        bit -= 1;
    }
}
