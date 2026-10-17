// Rust translation of aom_dsp/loopfilter.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The deblocking filters (`aom_lpf_*_c()` and `aom_highbd_lpf_*_c()`).
//!
//! The low bit depth filters compute what the high bit depth ones compute
//! for `bd` 8: their `int8_t` arithmetic (`^ 0x80`, `signed_char_clamp()`)
//! is the high bit depth one's `- (0x80 << 0)` and
//! `signed_char_clamp_high(.., 8)` on 8-bit samples, and their masks are
//! the high bit depth masks with no shift. So the samples (16-bit here)
//! are filtered by the high bit depth code, with `bd` 8 for the low bit
//! depth functions. Each filter handles four sample positions along the
//! edge; the dual and quad versions are two and four such calls on
//! consecutive positions.

/// Translation of `signed_char_clamp_high()` (and, for `bd` 8, of
/// `signed_char_clamp()`).
#[inline]
fn signed_char_clamp_high(t: i32, bd: i32) -> i16 {
    match bd {
        10 => t.clamp(-128 * 4, 128 * 4 - 1) as i16,
        12 => t.clamp(-128 * 16, 128 * 16 - 1) as i16,
        _ => t.clamp(-128, 128 - 1) as i16,
    }
}

#[inline]
fn m(c: bool) -> i8 {
    (c as i8).wrapping_neg()
}

// Should we apply any filter at all: 11111111 yes, 00000000 no ?
#[inline]
fn highbd_filter_mask2(limit: u8, blimit: u8, p1: i32, p0: i32, q0: i32, q1: i32, bd: i32) -> i8 {
    let mut mask: i8 = 0;
    let limit16 = ((limit as u16) << (bd - 8)) as i16 as i32;
    let blimit16 = ((blimit as u16) << (bd - 8)) as i16 as i32;
    mask |= m((p1 - p0).abs() > limit16);
    mask |= m((q1 - q0).abs() > limit16);
    mask |= m((p0 - q0).abs() * 2 + (p1 - q1).abs() / 2 > blimit16);
    !mask
}

// Should we apply any filter at all: 11111111 yes, 00000000 no ?
#[inline]
fn highbd_filter_mask(
    limit: u8,
    blimit: u8,
    p3: i32,
    p2: i32,
    p1: i32,
    p0: i32,
    q0: i32,
    q1: i32,
    q2: i32,
    q3: i32,
    bd: i32,
) -> i8 {
    let mut mask: i8 = 0;
    let limit16 = ((limit as u16) << (bd - 8)) as i16 as i32;
    let blimit16 = ((blimit as u16) << (bd - 8)) as i16 as i32;
    mask |= m((p3 - p2).abs() > limit16);
    mask |= m((p2 - p1).abs() > limit16);
    mask |= m((p1 - p0).abs() > limit16);
    mask |= m((q1 - q0).abs() > limit16);
    mask |= m((q2 - q1).abs() > limit16);
    mask |= m((q3 - q2).abs() > limit16);
    mask |= m((p0 - q0).abs() * 2 + (p1 - q1).abs() / 2 > blimit16);
    !mask
}

#[inline]
fn highbd_filter_mask3_chroma(
    limit: u8,
    blimit: u8,
    p2: i32,
    p1: i32,
    p0: i32,
    q0: i32,
    q1: i32,
    q2: i32,
    bd: i32,
) -> i8 {
    let mut mask: i8 = 0;
    let limit16 = ((limit as u16) << (bd - 8)) as i16 as i32;
    let blimit16 = ((blimit as u16) << (bd - 8)) as i16 as i32;
    mask |= m((p2 - p1).abs() > limit16);
    mask |= m((p1 - p0).abs() > limit16);
    mask |= m((q1 - q0).abs() > limit16);
    mask |= m((q2 - q1).abs() > limit16);
    mask |= m((p0 - q0).abs() * 2 + (p1 - q1).abs() / 2 > blimit16);
    !mask
}

#[inline]
fn highbd_flat_mask3_chroma(
    thresh: u8,
    p2: i32,
    p1: i32,
    p0: i32,
    q0: i32,
    q1: i32,
    q2: i32,
    bd: i32,
) -> i8 {
    let mut mask: i8 = 0;
    let thresh16 = ((thresh as u16) << (bd - 8)) as i16 as i32;
    mask |= m((p1 - p0).abs() > thresh16);
    mask |= m((q1 - q0).abs() > thresh16);
    mask |= m((p2 - p0).abs() > thresh16);
    mask |= m((q2 - q0).abs() > thresh16);
    !mask
}

#[inline]
fn highbd_flat_mask4(
    thresh: u8,
    p3: i32,
    p2: i32,
    p1: i32,
    p0: i32,
    q0: i32,
    q1: i32,
    q2: i32,
    q3: i32,
    bd: i32,
) -> i8 {
    let mut mask: i8 = 0;
    let thresh16 = ((thresh as u16) << (bd - 8)) as i16 as i32;
    mask |= m((p1 - p0).abs() > thresh16);
    mask |= m((q1 - q0).abs() > thresh16);
    mask |= m((p2 - p0).abs() > thresh16);
    mask |= m((q2 - q0).abs() > thresh16);
    mask |= m((p3 - p0).abs() > thresh16);
    mask |= m((q3 - q0).abs() > thresh16);
    !mask
}

// Is there high edge variance internal edge:
// 11111111_11111111 yes, 00000000_00000000 no ?
#[inline]
fn highbd_hev_mask(thresh: u8, p1: i32, p0: i32, q0: i32, q1: i32, bd: i32) -> i16 {
    let mut hev: i16 = 0;
    let thresh16 = ((thresh as u16) << (bd - 8)) as i16 as i32;
    hev |= ((p1 - p0).abs() > thresh16) as i16 * -1;
    hev |= ((q1 - q0).abs() > thresh16) as i16 * -1;
    hev
}

#[inline]
fn rpot(v: i32, n: i32) -> u16 {
    ((v + ((1 << n) >> 1)) >> n) as u16
}

/// Translation of `highbd_filter4()`: `o` holds the offsets of
/// `op1, op0, oq0, oq1`.
#[inline]
fn highbd_filter4(mask: i8, thresh: u8, s: &mut [u16], o: [usize; 4], bd: i32) {
    // ^0x80 equivalent to subtracting 0x80 from the values to turn them
    // into -128 to +127 instead of 0 to 255.
    let shift = bd - 8;
    let ps1 = (s[o[0]] as i16).wrapping_sub(0x80 << shift) as i32;
    let ps0 = (s[o[1]] as i16).wrapping_sub(0x80 << shift) as i32;
    let qs0 = (s[o[2]] as i16).wrapping_sub(0x80 << shift) as i32;
    let qs1 = (s[o[3]] as i16).wrapping_sub(0x80 << shift) as i32;
    let hev = highbd_hev_mask(
        thresh,
        s[o[0]] as i32,
        s[o[1]] as i32,
        s[o[2]] as i32,
        s[o[3]] as i32,
        bd,
    );

    // Add outer taps if we have high edge variance.
    let mut filter = signed_char_clamp_high(ps1 - qs1, bd) & hev;

    // Inner taps.
    filter = signed_char_clamp_high(filter as i32 + 3 * (qs0 - ps0), bd) & (mask as i16);

    // Save bottom 3 bits so that we round one side +4 and the other +3
    // if it equals 4 we'll set to adjust by -1 to account for the fact
    // we'd round 3 the other way.
    let filter1 = signed_char_clamp_high(filter as i32 + 4, bd) >> 3;
    let filter2 = signed_char_clamp_high(filter as i32 + 3, bd) >> 3;

    s[o[2]] = (signed_char_clamp_high(qs0 - filter1 as i32, bd) as i32 + (0x80 << shift)) as u16;
    s[o[1]] = (signed_char_clamp_high(ps0 + filter2 as i32, bd) as i32 + (0x80 << shift)) as u16;

    // Outer tap adjustments.
    filter = (((filter1 as i32 + 1) >> 1) as i16) & !hev;

    s[o[3]] = (signed_char_clamp_high(qs1 - filter as i32, bd) as i32 + (0x80 << shift)) as u16;
    s[o[0]] = (signed_char_clamp_high(ps1 + filter as i32, bd) as i32 + (0x80 << shift)) as u16;
}

/// Translation of `highbd_filter6()`: `o` holds the offsets of
/// `op2, op1, op0, oq0, oq1, oq2`.
#[inline]
fn highbd_filter6(mask: i8, thresh: u8, flat: i8, s: &mut [u16], o: [usize; 6], bd: i32) {
    if flat != 0 && mask != 0 {
        let (p2, p1, p0) = (s[o[0]] as i32, s[o[1]] as i32, s[o[2]] as i32);
        let (q0, q1, q2) = (s[o[3]] as i32, s[o[4]] as i32, s[o[5]] as i32);

        // 5-tap filter [1, 2, 2, 2, 1]
        s[o[1]] = rpot(p2 * 3 + p1 * 2 + p0 * 2 + q0, 3);
        s[o[2]] = rpot(p2 + p1 * 2 + p0 * 2 + q0 * 2 + q1, 3);
        s[o[3]] = rpot(p1 + p0 * 2 + q0 * 2 + q1 * 2 + q2, 3);
        s[o[4]] = rpot(p0 + q0 * 2 + q1 * 2 + q2 * 3, 3);
    } else {
        highbd_filter4(mask, thresh, s, [o[1], o[2], o[3], o[4]], bd);
    }
}

/// Translation of `highbd_filter8()`: `o` holds the offsets of
/// `op3 .. oq3`.
#[inline]
fn highbd_filter8(mask: i8, thresh: u8, flat: i8, s: &mut [u16], o: [usize; 8], bd: i32) {
    if flat != 0 && mask != 0 {
        let (p3, p2, p1, p0) = (
            s[o[0]] as i32,
            s[o[1]] as i32,
            s[o[2]] as i32,
            s[o[3]] as i32,
        );
        let (q0, q1, q2, q3) = (
            s[o[4]] as i32,
            s[o[5]] as i32,
            s[o[6]] as i32,
            s[o[7]] as i32,
        );

        // 7-tap filter [1, 1, 1, 2, 1, 1, 1]
        s[o[1]] = rpot(p3 + p3 + p3 + 2 * p2 + p1 + p0 + q0, 3);
        s[o[2]] = rpot(p3 + p3 + p2 + 2 * p1 + p0 + q0 + q1, 3);
        s[o[3]] = rpot(p3 + p2 + p1 + 2 * p0 + q0 + q1 + q2, 3);
        s[o[4]] = rpot(p2 + p1 + p0 + 2 * q0 + q1 + q2 + q3, 3);
        s[o[5]] = rpot(p1 + p0 + q0 + 2 * q1 + q2 + q3 + q3, 3);
        s[o[6]] = rpot(p0 + q0 + q1 + 2 * q2 + q3 + q3 + q3, 3);
    } else {
        highbd_filter4(mask, thresh, s, [o[2], o[3], o[4], o[5]], bd);
    }
}

/// Translation of `highbd_filter14()`: `o` holds the offsets of
/// `op6 .. oq6`.
#[inline]
fn highbd_filter14(
    mask: i8,
    thresh: u8,
    flat: i8,
    flat2: i8,
    s: &mut [u16],
    o: [usize; 14],
    bd: i32,
) {
    if flat2 != 0 && flat != 0 && mask != 0 {
        let p6 = s[o[0]] as i32;
        let p5 = s[o[1]] as i32;
        let p4 = s[o[2]] as i32;
        let p3 = s[o[3]] as i32;
        let p2 = s[o[4]] as i32;
        let p1 = s[o[5]] as i32;
        let p0 = s[o[6]] as i32;
        let q0 = s[o[7]] as i32;
        let q1 = s[o[8]] as i32;
        let q2 = s[o[9]] as i32;
        let q3 = s[o[10]] as i32;
        let q4 = s[o[11]] as i32;
        let q5 = s[o[12]] as i32;
        let q6 = s[o[13]] as i32;

        // 13-tap filter [1, 1, 1, 1, 1, 2, 2, 2, 1, 1, 1, 1, 1]
        s[o[1]] = rpot(p6 * 7 + p5 * 2 + p4 * 2 + p3 + p2 + p1 + p0 + q0, 4);
        s[o[2]] = rpot(
            p6 * 5 + p5 * 2 + p4 * 2 + p3 * 2 + p2 + p1 + p0 + q0 + q1,
            4,
        );
        s[o[3]] = rpot(
            p6 * 4 + p5 + p4 * 2 + p3 * 2 + p2 * 2 + p1 + p0 + q0 + q1 + q2,
            4,
        );
        s[o[4]] = rpot(
            p6 * 3 + p5 + p4 + p3 * 2 + p2 * 2 + p1 * 2 + p0 + q0 + q1 + q2 + q3,
            4,
        );
        s[o[5]] = rpot(
            p6 * 2 + p5 + p4 + p3 + p2 * 2 + p1 * 2 + p0 * 2 + q0 + q1 + q2 + q3 + q4,
            4,
        );
        s[o[6]] = rpot(
            p6 + p5 + p4 + p3 + p2 + p1 * 2 + p0 * 2 + q0 * 2 + q1 + q2 + q3 + q4 + q5,
            4,
        );
        s[o[7]] = rpot(
            p5 + p4 + p3 + p2 + p1 + p0 * 2 + q0 * 2 + q1 * 2 + q2 + q3 + q4 + q5 + q6,
            4,
        );
        s[o[8]] = rpot(
            p4 + p3 + p2 + p1 + p0 + q0 * 2 + q1 * 2 + q2 * 2 + q3 + q4 + q5 + q6 * 2,
            4,
        );
        s[o[9]] = rpot(
            p3 + p2 + p1 + p0 + q0 + q1 * 2 + q2 * 2 + q3 * 2 + q4 + q5 + q6 * 3,
            4,
        );
        s[o[10]] = rpot(
            p2 + p1 + p0 + q0 + q1 + q2 * 2 + q3 * 2 + q4 * 2 + q5 + q6 * 4,
            4,
        );
        s[o[11]] = rpot(
            p1 + p0 + q0 + q1 + q2 + q3 * 2 + q4 * 2 + q5 * 2 + q6 * 5,
            4,
        );
        s[o[12]] = rpot(p0 + q0 + q1 + q2 + q3 + q4 * 2 + q5 * 2 + q6 * 7, 4);
    } else {
        highbd_filter8(
            mask,
            thresh,
            flat,
            s,
            [o[3], o[4], o[5], o[6], o[7], o[8], o[9], o[10]],
            bd,
        );
    }
}

/// The edge geometry of a filter call: `s` is the offset of the first
/// sample across the edge (q0), `tap` the step across the edge and
/// `along` the step to the next position along it.
#[derive(Debug, Clone, Copy)]
struct Edge {
    s: isize,
    tap: isize,
    along: isize,
}

impl Edge {
    #[inline]
    fn at(&self, i: isize, k: isize) -> usize {
        (self.s + i * self.along + k * self.tap) as usize
    }
}

/// `aom_highbd_lpf_{horizontal,vertical}_4_c()`
fn lpf_4(s: &mut [u16], e: Edge, blimit: u8, limit: u8, thresh: u8, bd: i32) {
    // loop filter designed to work using chars so that we can make maximum use
    // of 8 bit simd instructions.
    for i in 0..4 {
        let o = [e.at(i, -2), e.at(i, -1), e.at(i, 0), e.at(i, 1)];
        let (p1, p0, q0, q1) = (
            s[o[0]] as i32,
            s[o[1]] as i32,
            s[o[2]] as i32,
            s[o[3]] as i32,
        );
        let mask = highbd_filter_mask2(limit, blimit, p1, p0, q0, q1, bd);
        highbd_filter4(mask, thresh, s, o, bd);
    }
}

/// `aom_highbd_lpf_{horizontal,vertical}_6_c()`
fn lpf_6(s: &mut [u16], e: Edge, blimit: u8, limit: u8, thresh: u8, bd: i32) {
    for i in 0..4 {
        let o = [
            e.at(i, -3),
            e.at(i, -2),
            e.at(i, -1),
            e.at(i, 0),
            e.at(i, 1),
            e.at(i, 2),
        ];
        let mut v = [0i32; 6];
        for k in 0..6 {
            v[k] = s[o[k]] as i32;
        }
        let mask =
            highbd_filter_mask3_chroma(limit, blimit, v[0], v[1], v[2], v[3], v[4], v[5], bd);
        let flat = highbd_flat_mask3_chroma(1, v[0], v[1], v[2], v[3], v[4], v[5], bd);
        highbd_filter6(mask, thresh, flat, s, o, bd);
    }
}

/// `aom_highbd_lpf_{horizontal,vertical}_8_c()`
fn lpf_8(s: &mut [u16], e: Edge, blimit: u8, limit: u8, thresh: u8, bd: i32) {
    for i in 0..4 {
        let o = [
            e.at(i, -4),
            e.at(i, -3),
            e.at(i, -2),
            e.at(i, -1),
            e.at(i, 0),
            e.at(i, 1),
            e.at(i, 2),
            e.at(i, 3),
        ];
        let mut v = [0i32; 8];
        for k in 0..8 {
            v[k] = s[o[k]] as i32;
        }
        let mask = highbd_filter_mask(
            limit, blimit, v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7], bd,
        );
        let flat = highbd_flat_mask4(1, v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7], bd);
        highbd_filter8(mask, thresh, flat, s, o, bd);
    }
}

/// `highbd_mb_lpf_{horizontal,vertical}_edge_w()` over 4 positions
fn lpf_14(s: &mut [u16], e: Edge, blimit: u8, limit: u8, thresh: u8, bd: i32) {
    for i in 0..4 {
        let mut o = [0usize; 14];
        for k in 0..14 {
            o[k] = e.at(i, k as isize - 7);
        }
        let mut v = [0i32; 14];
        for k in 0..14 {
            v[k] = s[o[k]] as i32;
        }
        // (p3 .. q3 are v[3] .. v[10])
        let mask = highbd_filter_mask(
            limit, blimit, v[3], v[4], v[5], v[6], v[7], v[8], v[9], v[10], bd,
        );
        let flat = highbd_flat_mask4(1, v[3], v[4], v[5], v[6], v[7], v[8], v[9], v[10], bd);
        let flat2 = highbd_flat_mask4(1, v[0], v[1], v[2], v[6], v[7], v[11], v[12], v[13], bd);
        highbd_filter14(mask, thresh, flat, flat2, s, o, bd);
    }
}

/// Filters `n` consecutive groups of four positions along an edge with
/// the `filter_length`-tap filter (4, 6, 8 or 14; others filter nothing):
/// the single (`n` 1), dual (2) and quad (4) versions of
/// `aom_lpf_horizontal_*()` (`vertical` false: the edge is horizontal, the
/// taps are a row apart) and `aom_lpf_vertical_*()`, low (`bd` 8) or high
/// bit depth.
pub(crate) fn aom_lpf(
    s: &mut [u16],
    s_off: usize,
    pitch: i32,
    vertical: bool,
    filter_length: i32,
    n: i32,
    blimit: u8,
    limit: u8,
    thresh: u8,
    bd: i32,
) {
    let pitch = pitch as isize;
    let (tap, along) = if vertical { (1, pitch) } else { (pitch, 1) };
    for g in 0..n as isize {
        let e = Edge {
            s: s_off as isize + 4 * g * along,
            tap,
            along,
        };
        match filter_length {
            4 => lpf_4(s, e, blimit, limit, thresh, bd),
            6 => lpf_6(s, e, blimit, limit, thresh, bd),
            8 => lpf_8(s, e, blimit, limit, thresh, bd),
            14 => lpf_14(s, e, blimit, limit, thresh, bd),
            _ => {}
        }
    }
}
