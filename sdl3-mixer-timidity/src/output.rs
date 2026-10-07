/*
    TiMidity -- Experimental MIDI to WAVE converter
    Copyright (C) 1995 Tuukka Toivonen <toivonen@clinet.fi>

    This program is free software; you can redistribute it and/or modify
    it under the terms of the Perl Artistic License, available in COPYING.

    output.c

    Audio output (to file / device) functions.
*/
// Modified 2026-10-07: translated into Rust from output.c and output.h in SDL_mixer 3.3.0's TiMidity (see NOTICE).

//! Conversion of the 32-bit mix to the output format. Translation of
//! `output.c` (and `output.h`).
//!
//! Upstream picks the byte-swapping variants by the machine's byte order
//! (`timi_s32tos16l` is `timi_s32tos16` on a little-endian machine, and
//! so on); here each function writes the byte order its name says.
//!
//! The output buffer is bytes: these write as many samples as fit in it
//! (`dp` in upstream must hold all of them).

use crate::options::GUARD_BITS;
use crate::WriteFn;

/* Data format encoding bits */

pub(crate) const PE_MONO: i32 = 0x01; /* versus stereo */
pub(crate) const PE_SIGNED: i32 = 0x02; /* versus unsigned */
pub(crate) const PE_16BIT: i32 = 0x04; /* versus 8-bit */
pub(crate) const PE_32BIT: i32 = 0x08; /* versus 8-bit or 16-bit */

/*****************************************************************/
/* Some functions to convert signed 32-bit data to other formats */

/// Write `c` samples of `lp` to `dp`, `N` bytes each.
fn write_each<const N: usize>(dp: &mut [u8], lp: &[i32], c: i32, f: impl Fn(i32) -> [u8; N]) {
    let c = c.max(0) as usize;
    for (out, &l) in dp.chunks_exact_mut(N).zip(lp).take(c) {
        out.copy_from_slice(&f(l));
    }
}

/// Translation of `timi_s32tos8()`.
pub(crate) fn timi_s32tos8(dp: &mut [u8], lp: &[i32], c: i32) {
    write_each(dp, lp, c, |l| {
        let mut l = l >> (32 - 8 - GUARD_BITS);
        if l > 127 {
            l = 127;
        } else if l < -128 {
            l = -128;
        }
        [(l as i8) as u8]
    });
}

/// Translation of `timi_s32tou8()`.
pub(crate) fn timi_s32tou8(dp: &mut [u8], lp: &[i32], c: i32) {
    write_each(dp, lp, c, |l| {
        let mut l = l >> (32 - 8 - GUARD_BITS);
        if l > 127 {
            l = 127;
        } else if l < -128 {
            l = -128;
        }
        [0x80 ^ (l as u8)]
    });
}

fn s32tos16(l: i32) -> i16 {
    let mut l = l >> (32 - 16 - GUARD_BITS);
    if l > 32767 {
        l = 32767;
    } else if l < -32768 {
        l = -32768;
    }
    l as i16
}

/// Translation of `timi_s32tos16l` (`timi_s32tos16()` on a little-endian
/// machine).
pub(crate) fn timi_s32tos16l(dp: &mut [u8], lp: &[i32], c: i32) {
    write_each(dp, lp, c, |l| s32tos16(l).to_le_bytes());
}

/// Translation of `timi_s32tos16b` (`timi_s32tos16x()` on a little-endian
/// machine).
pub(crate) fn timi_s32tos16b(dp: &mut [u8], lp: &[i32], c: i32) {
    write_each(dp, lp, c, |l| s32tos16(l).to_be_bytes());
}

fn s32tof32(l: i32) -> f32 {
    l as f32 / (1i32 << (32 - GUARD_BITS - 1)) as f32
}

/// Translation of `timi_s32tof32l` (`timi_s32tof32()` on a little-endian
/// machine).
pub(crate) fn timi_s32tof32l(dp: &mut [u8], lp: &[i32], c: i32) {
    write_each(dp, lp, c, |l| s32tof32(l).to_le_bytes());
}

/// Translation of `timi_s32tof32b` (`timi_s32tof32x()` on a little-endian
/// machine).
pub(crate) fn timi_s32tof32b(dp: &mut [u8], lp: &[i32], c: i32) {
    write_each(dp, lp, c, |l| s32tof32(l).to_be_bytes());
}

/// Translation of `timi_s32tos32l` (`timi_s32tos32()` on a little-endian
/// machine).
pub(crate) fn timi_s32tos32l(dp: &mut [u8], lp: &[i32], c: i32) {
    write_each(dp, lp, c, |l| (l << GUARD_BITS).to_le_bytes());
}

/// Translation of `timi_s32tos32b` (`timi_s32tos32x()` on a little-endian
/// machine).
pub(crate) fn timi_s32tos32b(dp: &mut [u8], lp: &[i32], c: i32) {
    write_each(dp, lp, c, |l| (l << GUARD_BITS).to_be_bytes());
}

/// `song->write(dp, lp, c)`.
pub(crate) fn write(f: WriteFn, dp: &mut [u8], lp: &[i32], c: i32) {
    match f {
        WriteFn::S8 => timi_s32tos8(dp, lp, c),
        WriteFn::U8 => timi_s32tou8(dp, lp, c),
        WriteFn::S16L => timi_s32tos16l(dp, lp, c),
        WriteFn::S16B => timi_s32tos16b(dp, lp, c),
        WriteFn::S32L => timi_s32tos32l(dp, lp, c),
        WriteFn::S32B => timi_s32tos32b(dp, lp, c),
        WriteFn::F32L => timi_s32tof32l(dp, lp, c),
        WriteFn::F32B => timi_s32tof32b(dp, lp, c),
    }
}
