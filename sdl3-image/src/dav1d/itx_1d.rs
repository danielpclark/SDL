// Rust translation of src/itx_1d.c and src/itx_1d.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins).
// Copyright © 2018-2019, VideoLAN and dav1d authors
// Copyright © 2018-2019, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The one-dimensional inverse transforms: `c` is the buffer, `o` the
//! offset of the first element and `s` the stride (`int32_t *c` and
//! `ptrdiff_t stride` upstream).

#![allow(clippy::identity_op, clippy::erasing_op, clippy::too_many_arguments)]

use super::bitdepth::ix;
use super::intops::iclip;

/// `CLIP()`.
#[inline(always)]
fn clip(min: i32, max: i32, a: i32) -> i32 {
    iclip(a, min, max)
}

/*
 * In some places, we use the pattern like this:
 * t2 = ((in1 *  1567         - in3 * (3784 - 4096) + 2048) >> 12) - in3;
 * even though the reference code might use something like:
 * t2 =  (in1 *  1567         - in3 *  3784         + 2048) >> 12;
 *
 * The reason for this is that for 12 bits/component bitstreams (corrupt/
 * invalid ones, but they are codable nonetheless), each coefficient or
 * input can be 19(+sign) bits, and therefore if the combination of the
 * two multipliers (each 12 bits) is >= 4096, the result of the add/sub
 * after the pair of multiplies will exceed the 31+sign bit range. Signed
 * integer overflows are UB in C, and we'd like to prevent that.
 *
 * To workaround this, we invert one of the two coefficients (or, if both are
 * multiples of 2, we reduce their magnitude by one bit). It should be noted
 * that SIMD implementations do not have to follow this exact behaviour. The
 * AV1 spec clearly states that the result of the multiply/add pairs should
 * fit in 31+sign bit intermediates, and that streams violating this convention
 * are not AV1-compliant. So, as long as we don't trigger UB (which some people
 * would consider a security vulnerability), we're fine. So, SIMD can simply
 * use the faster implementation, even if that might in some cases result in
 * integer overflows, since these are not considered valid AV1 anyway, and in
 * e.g. x86 assembly, integer overflows are not considered UB, but they merely
 * wrap around.
 */

#[inline(never)]
fn inv_dct4_1d_internal(c: &mut [i32], o: usize, s: usize, min: i32, max: i32, tx64: bool) {
    let in0 = c[o + 0 * s];
    let in1 = c[o + 1 * s];
    let t0;
    let t1;
    let t2;
    let t3;
    if tx64 {
        t1 = (in0 * 181 + 128) >> 8;
        t0 = t1;
        t2 = (in1 * 1567 + 2048) >> 12;
        t3 = (in1 * 3784 + 2048) >> 12;
    } else {
        let in2 = c[o + 2 * s];
        let in3 = c[o + 3 * s];
        t0 = ((in0 + in2) * 181 + 128) >> 8;
        t1 = ((in0 - in2) * 181 + 128) >> 8;
        t2 = ((in1 * 1567 - in3 * (3784 - 4096) + 2048) >> 12) - in3;
        t3 = ((in1 * (3784 - 4096) + in3 * 1567 + 2048) >> 12) + in1;
    }
    c[o + 0 * s] = clip(min, max, t0 + t3);
    c[o + 1 * s] = clip(min, max, t1 + t2);
    c[o + 2 * s] = clip(min, max, t1 - t2);
    c[o + 3 * s] = clip(min, max, t0 - t3);
}

/// Translation of `dav1d_inv_dct4_1d_c()`.
pub(crate) fn inv_dct4_1d(c: &mut [i32], o: usize, s: usize, min: i32, max: i32) {
    inv_dct4_1d_internal(c, o, s, min, max, false);
}

#[inline(never)]
fn inv_dct8_1d_internal(c: &mut [i32], o: usize, s: usize, min: i32, max: i32, tx64: bool) {
    inv_dct4_1d_internal(c, o, s << 1, min, max, tx64);
    let in1 = c[o + 1 * s];
    let in3 = c[o + 3 * s];
    let t4a;
    let mut t5a;
    let mut t6a;
    let t7a;
    if tx64 {
        t4a = (in1 * 799 + 2048) >> 12;
        t5a = (in3 * -2276 + 2048) >> 12;
        t6a = (in3 * 3406 + 2048) >> 12;
        t7a = (in1 * 4017 + 2048) >> 12;
    } else {
        let in5 = c[o + 5 * s];
        let in7 = c[o + 7 * s];
        t4a = ((in1 * 799 - in7 * (4017 - 4096) + 2048) >> 12) - in7;
        t5a = (in5 * 1703 - in3 * 1138 + 1024) >> 11;
        t6a = (in5 * 1138 + in3 * 1703 + 1024) >> 11;
        t7a = ((in1 * (4017 - 4096) + in7 * 799 + 2048) >> 12) + in1;
    }
    let t4 = clip(min, max, t4a + t5a);
    t5a = clip(min, max, t4a - t5a);
    let t7 = clip(min, max, t7a + t6a);
    t6a = clip(min, max, t7a - t6a);
    let t5 = ((t6a - t5a) * 181 + 128) >> 8;
    let t6 = ((t6a + t5a) * 181 + 128) >> 8;
    let t0 = c[o + 0 * s];
    let t1 = c[o + 2 * s];
    let t2 = c[o + 4 * s];
    let t3 = c[o + 6 * s];
    c[o + 0 * s] = clip(min, max, t0 + t7);
    c[o + 1 * s] = clip(min, max, t1 + t6);
    c[o + 2 * s] = clip(min, max, t2 + t5);
    c[o + 3 * s] = clip(min, max, t3 + t4);
    c[o + 4 * s] = clip(min, max, t3 - t4);
    c[o + 5 * s] = clip(min, max, t2 - t5);
    c[o + 6 * s] = clip(min, max, t1 - t6);
    c[o + 7 * s] = clip(min, max, t0 - t7);
}

/// Translation of `dav1d_inv_dct8_1d_c()`.
pub(crate) fn inv_dct8_1d(c: &mut [i32], o: usize, s: usize, min: i32, max: i32) {
    inv_dct8_1d_internal(c, o, s, min, max, false);
}

#[inline(never)]
fn inv_dct16_1d_internal(c: &mut [i32], o: usize, s: usize, min: i32, max: i32, tx64: bool) {
    inv_dct8_1d_internal(c, o, s << 1, min, max, tx64);
    let in1 = c[o + 1 * s];
    let in3 = c[o + 3 * s];
    let in5 = c[o + 5 * s];
    let in7 = c[o + 7 * s];
    let mut t8a;
    let mut t9a;
    let mut t10a;
    let mut t11a;
    let mut t12a;
    let mut t13a;
    let mut t14a;
    let mut t15a;
    if tx64 {
        t8a = (in1 * 401 + 2048) >> 12;
        t9a = (in7 * -2598 + 2048) >> 12;
        t10a = (in5 * 1931 + 2048) >> 12;
        t11a = (in3 * -1189 + 2048) >> 12;
        t12a = (in3 * 3920 + 2048) >> 12;
        t13a = (in5 * 3612 + 2048) >> 12;
        t14a = (in7 * 3166 + 2048) >> 12;
        t15a = (in1 * 4076 + 2048) >> 12;
    } else {
        let in9 = c[o + 9 * s];
        let in11 = c[o + 11 * s];
        let in13 = c[o + 13 * s];
        let in15 = c[o + 15 * s];
        t8a = ((in1 * 401 - in15 * (4076 - 4096) + 2048) >> 12) - in15;
        t9a = (in9 * 1583 - in7 * 1299 + 1024) >> 11;
        t10a = ((in5 * 1931 - in11 * (3612 - 4096) + 2048) >> 12) - in11;
        t11a = ((in13 * (3920 - 4096) - in3 * 1189 + 2048) >> 12) + in13;
        t12a = ((in13 * 1189 + in3 * (3920 - 4096) + 2048) >> 12) + in3;
        t13a = ((in5 * (3612 - 4096) + in11 * 1931 + 2048) >> 12) + in5;
        t14a = (in9 * 1299 + in7 * 1583 + 1024) >> 11;
        t15a = ((in1 * (4076 - 4096) + in15 * 401 + 2048) >> 12) + in1;
    }
    let t8 = clip(min, max, t8a + t9a);
    let mut t9 = clip(min, max, t8a - t9a);
    let mut t10 = clip(min, max, t11a - t10a);
    let mut t11 = clip(min, max, t11a + t10a);
    let mut t12 = clip(min, max, t12a + t13a);
    let mut t13 = clip(min, max, t12a - t13a);
    let mut t14 = clip(min, max, t15a - t14a);
    let t15 = clip(min, max, t15a + t14a);
    t9a = ((t14 * 1567 - t9 * (3784 - 4096) + 2048) >> 12) - t9;
    t14a = ((t14 * (3784 - 4096) + t9 * 1567 + 2048) >> 12) + t14;
    t10a = ((-(t13 * (3784 - 4096) + t10 * 1567) + 2048) >> 12) - t13;
    t13a = ((t13 * 1567 - t10 * (3784 - 4096) + 2048) >> 12) - t10;
    t8a = clip(min, max, t8 + t11);
    t9 = clip(min, max, t9a + t10a);
    t10 = clip(min, max, t9a - t10a);
    t11a = clip(min, max, t8 - t11);
    t12a = clip(min, max, t15 - t12);
    t13 = clip(min, max, t14a - t13a);
    t14 = clip(min, max, t14a + t13a);
    t15a = clip(min, max, t15 + t12);
    t10a = ((t13 - t10) * 181 + 128) >> 8;
    t13a = ((t13 + t10) * 181 + 128) >> 8;
    t11 = ((t12a - t11a) * 181 + 128) >> 8;
    t12 = ((t12a + t11a) * 181 + 128) >> 8;
    let t0 = c[o + 0 * s];
    let t1 = c[o + 2 * s];
    let t2 = c[o + 4 * s];
    let t3 = c[o + 6 * s];
    let t4 = c[o + 8 * s];
    let t5 = c[o + 10 * s];
    let t6 = c[o + 12 * s];
    let t7 = c[o + 14 * s];
    c[o + 0 * s] = clip(min, max, t0 + t15a);
    c[o + 1 * s] = clip(min, max, t1 + t14);
    c[o + 2 * s] = clip(min, max, t2 + t13a);
    c[o + 3 * s] = clip(min, max, t3 + t12);
    c[o + 4 * s] = clip(min, max, t4 + t11);
    c[o + 5 * s] = clip(min, max, t5 + t10a);
    c[o + 6 * s] = clip(min, max, t6 + t9);
    c[o + 7 * s] = clip(min, max, t7 + t8a);
    c[o + 8 * s] = clip(min, max, t7 - t8a);
    c[o + 9 * s] = clip(min, max, t6 - t9);
    c[o + 10 * s] = clip(min, max, t5 - t10a);
    c[o + 11 * s] = clip(min, max, t4 - t11);
    c[o + 12 * s] = clip(min, max, t3 - t12);
    c[o + 13 * s] = clip(min, max, t2 - t13a);
    c[o + 14 * s] = clip(min, max, t1 - t14);
    c[o + 15 * s] = clip(min, max, t0 - t15a);
}

/// Translation of `dav1d_inv_dct16_1d_c()`.
pub(crate) fn inv_dct16_1d(c: &mut [i32], o: usize, s: usize, min: i32, max: i32) {
    inv_dct16_1d_internal(c, o, s, min, max, false);
}

#[inline(never)]
fn inv_dct32_1d_internal(c: &mut [i32], o: usize, s: usize, min: i32, max: i32, tx64: bool) {
    inv_dct16_1d_internal(c, o, s << 1, min, max, tx64);
    let in1 = c[o + 1 * s];
    let in3 = c[o + 3 * s];
    let in5 = c[o + 5 * s];
    let in7 = c[o + 7 * s];
    let in9 = c[o + 9 * s];
    let in11 = c[o + 11 * s];
    let in13 = c[o + 13 * s];
    let in15 = c[o + 15 * s];
    let mut t16a;
    let mut t17a;
    let mut t18a;
    let mut t19a;
    let mut t20a;
    let mut t21a;
    let mut t22a;
    let mut t23a;
    let mut t24a;
    let mut t25a;
    let mut t26a;
    let mut t27a;
    let mut t28a;
    let mut t29a;
    let mut t30a;
    let mut t31a;
    if tx64 {
        t16a = (in1 * 201 + 2048) >> 12;
        t17a = (in15 * -2751 + 2048) >> 12;
        t18a = (in9 * 1751 + 2048) >> 12;
        t19a = (in7 * -1380 + 2048) >> 12;
        t20a = (in5 * 995 + 2048) >> 12;
        t21a = (in11 * -2106 + 2048) >> 12;
        t22a = (in13 * 2440 + 2048) >> 12;
        t23a = (in3 * -601 + 2048) >> 12;
        t24a = (in3 * 4052 + 2048) >> 12;
        t25a = (in13 * 3290 + 2048) >> 12;
        t26a = (in11 * 3513 + 2048) >> 12;
        t27a = (in5 * 3973 + 2048) >> 12;
        t28a = (in7 * 3857 + 2048) >> 12;
        t29a = (in9 * 3703 + 2048) >> 12;
        t30a = (in15 * 3035 + 2048) >> 12;
        t31a = (in1 * 4091 + 2048) >> 12;
    } else {
        let in17 = c[o + 17 * s];
        let in19 = c[o + 19 * s];
        let in21 = c[o + 21 * s];
        let in23 = c[o + 23 * s];
        let in25 = c[o + 25 * s];
        let in27 = c[o + 27 * s];
        let in29 = c[o + 29 * s];
        let in31 = c[o + 31 * s];
        t16a = ((in1 * 201 - in31 * (4091 - 4096) + 2048) >> 12) - in31;
        t17a = ((in17 * (3035 - 4096) - in15 * 2751 + 2048) >> 12) + in17;
        t18a = ((in9 * 1751 - in23 * (3703 - 4096) + 2048) >> 12) - in23;
        t19a = ((in25 * (3857 - 4096) - in7 * 1380 + 2048) >> 12) + in25;
        t20a = ((in5 * 995 - in27 * (3973 - 4096) + 2048) >> 12) - in27;
        t21a = ((in21 * (3513 - 4096) - in11 * 2106 + 2048) >> 12) + in21;
        t22a = (in13 * 1220 - in19 * 1645 + 1024) >> 11;
        t23a = ((in29 * (4052 - 4096) - in3 * 601 + 2048) >> 12) + in29;
        t24a = ((in29 * 601 + in3 * (4052 - 4096) + 2048) >> 12) + in3;
        t25a = (in13 * 1645 + in19 * 1220 + 1024) >> 11;
        t26a = ((in21 * 2106 + in11 * (3513 - 4096) + 2048) >> 12) + in11;
        t27a = ((in5 * (3973 - 4096) + in27 * 995 + 2048) >> 12) + in5;
        t28a = ((in25 * 1380 + in7 * (3857 - 4096) + 2048) >> 12) + in7;
        t29a = ((in9 * (3703 - 4096) + in23 * 1751 + 2048) >> 12) + in9;
        t30a = ((in17 * 2751 + in15 * (3035 - 4096) + 2048) >> 12) + in15;
        t31a = ((in1 * (4091 - 4096) + in31 * 201 + 2048) >> 12) + in1;
    }
    let mut t16 = clip(min, max, t16a + t17a);
    let mut t17 = clip(min, max, t16a - t17a);
    let mut t18 = clip(min, max, t19a - t18a);
    let mut t19 = clip(min, max, t19a + t18a);
    let mut t20 = clip(min, max, t20a + t21a);
    let mut t21 = clip(min, max, t20a - t21a);
    let mut t22 = clip(min, max, t23a - t22a);
    let mut t23 = clip(min, max, t23a + t22a);
    let mut t24 = clip(min, max, t24a + t25a);
    let mut t25 = clip(min, max, t24a - t25a);
    let mut t26 = clip(min, max, t27a - t26a);
    let mut t27 = clip(min, max, t27a + t26a);
    let mut t28 = clip(min, max, t28a + t29a);
    let mut t29 = clip(min, max, t28a - t29a);
    let mut t30 = clip(min, max, t31a - t30a);
    let mut t31 = clip(min, max, t31a + t30a);
    t17a = ((t30 * 799 - t17 * (4017 - 4096) + 2048) >> 12) - t17;
    t30a = ((t30 * (4017 - 4096) + t17 * 799 + 2048) >> 12) + t30;
    t18a = ((-(t29 * (4017 - 4096) + t18 * 799) + 2048) >> 12) - t29;
    t29a = ((t29 * 799 - t18 * (4017 - 4096) + 2048) >> 12) - t18;
    t21a = (t26 * 1703 - t21 * 1138 + 1024) >> 11;
    t26a = (t26 * 1138 + t21 * 1703 + 1024) >> 11;
    t22a = (-(t25 * 1138 + t22 * 1703) + 1024) >> 11;
    t25a = (t25 * 1703 - t22 * 1138 + 1024) >> 11;
    t16a = clip(min, max, t16 + t19);
    t17 = clip(min, max, t17a + t18a);
    t18 = clip(min, max, t17a - t18a);
    t19a = clip(min, max, t16 - t19);
    t20a = clip(min, max, t23 - t20);
    t21 = clip(min, max, t22a - t21a);
    t22 = clip(min, max, t22a + t21a);
    t23a = clip(min, max, t23 + t20);
    t24a = clip(min, max, t24 + t27);
    t25 = clip(min, max, t25a + t26a);
    t26 = clip(min, max, t25a - t26a);
    t27a = clip(min, max, t24 - t27);
    t28a = clip(min, max, t31 - t28);
    t29 = clip(min, max, t30a - t29a);
    t30 = clip(min, max, t30a + t29a);
    t31a = clip(min, max, t31 + t28);
    t18a = ((t29 * 1567 - t18 * (3784 - 4096) + 2048) >> 12) - t18;
    t29a = ((t29 * (3784 - 4096) + t18 * 1567 + 2048) >> 12) + t29;
    t19 = ((t28a * 1567 - t19a * (3784 - 4096) + 2048) >> 12) - t19a;
    t28 = ((t28a * (3784 - 4096) + t19a * 1567 + 2048) >> 12) + t28a;
    t20 = ((-(t27a * (3784 - 4096) + t20a * 1567) + 2048) >> 12) - t27a;
    t27 = ((t27a * 1567 - t20a * (3784 - 4096) + 2048) >> 12) - t20a;
    t21a = ((-(t26 * (3784 - 4096) + t21 * 1567) + 2048) >> 12) - t26;
    t26a = ((t26 * 1567 - t21 * (3784 - 4096) + 2048) >> 12) - t21;
    t16 = clip(min, max, t16a + t23a);
    t17a = clip(min, max, t17 + t22);
    t18 = clip(min, max, t18a + t21a);
    t19a = clip(min, max, t19 + t20);
    t20a = clip(min, max, t19 - t20);
    t21 = clip(min, max, t18a - t21a);
    t22a = clip(min, max, t17 - t22);
    t23 = clip(min, max, t16a - t23a);
    t24 = clip(min, max, t31a - t24a);
    t25a = clip(min, max, t30 - t25);
    t26 = clip(min, max, t29a - t26a);
    t27a = clip(min, max, t28 - t27);
    t28a = clip(min, max, t28 + t27);
    t29 = clip(min, max, t29a + t26a);
    t30a = clip(min, max, t30 + t25);
    t31 = clip(min, max, t31a + t24a);
    t20 = ((t27a - t20a) * 181 + 128) >> 8;
    t27 = ((t27a + t20a) * 181 + 128) >> 8;
    t21a = ((t26 - t21) * 181 + 128) >> 8;
    t26a = ((t26 + t21) * 181 + 128) >> 8;
    t22 = ((t25a - t22a) * 181 + 128) >> 8;
    t25 = ((t25a + t22a) * 181 + 128) >> 8;
    t23a = ((t24 - t23) * 181 + 128) >> 8;
    t24a = ((t24 + t23) * 181 + 128) >> 8;
    let t0 = c[o + 0 * s];
    let t1 = c[o + 2 * s];
    let t2 = c[o + 4 * s];
    let t3 = c[o + 6 * s];
    let t4 = c[o + 8 * s];
    let t5 = c[o + 10 * s];
    let t6 = c[o + 12 * s];
    let t7 = c[o + 14 * s];
    let t8 = c[o + 16 * s];
    let t9 = c[o + 18 * s];
    let t10 = c[o + 20 * s];
    let t11 = c[o + 22 * s];
    let t12 = c[o + 24 * s];
    let t13 = c[o + 26 * s];
    let t14 = c[o + 28 * s];
    let t15 = c[o + 30 * s];
    c[o + 0 * s] = clip(min, max, t0 + t31);
    c[o + 1 * s] = clip(min, max, t1 + t30a);
    c[o + 2 * s] = clip(min, max, t2 + t29);
    c[o + 3 * s] = clip(min, max, t3 + t28a);
    c[o + 4 * s] = clip(min, max, t4 + t27);
    c[o + 5 * s] = clip(min, max, t5 + t26a);
    c[o + 6 * s] = clip(min, max, t6 + t25);
    c[o + 7 * s] = clip(min, max, t7 + t24a);
    c[o + 8 * s] = clip(min, max, t8 + t23a);
    c[o + 9 * s] = clip(min, max, t9 + t22);
    c[o + 10 * s] = clip(min, max, t10 + t21a);
    c[o + 11 * s] = clip(min, max, t11 + t20);
    c[o + 12 * s] = clip(min, max, t12 + t19a);
    c[o + 13 * s] = clip(min, max, t13 + t18);
    c[o + 14 * s] = clip(min, max, t14 + t17a);
    c[o + 15 * s] = clip(min, max, t15 + t16);
    c[o + 16 * s] = clip(min, max, t15 - t16);
    c[o + 17 * s] = clip(min, max, t14 - t17a);
    c[o + 18 * s] = clip(min, max, t13 - t18);
    c[o + 19 * s] = clip(min, max, t12 - t19a);
    c[o + 20 * s] = clip(min, max, t11 - t20);
    c[o + 21 * s] = clip(min, max, t10 - t21a);
    c[o + 22 * s] = clip(min, max, t9 - t22);
    c[o + 23 * s] = clip(min, max, t8 - t23a);
    c[o + 24 * s] = clip(min, max, t7 - t24a);
    c[o + 25 * s] = clip(min, max, t6 - t25);
    c[o + 26 * s] = clip(min, max, t5 - t26a);
    c[o + 27 * s] = clip(min, max, t4 - t27);
    c[o + 28 * s] = clip(min, max, t3 - t28a);
    c[o + 29 * s] = clip(min, max, t2 - t29);
    c[o + 30 * s] = clip(min, max, t1 - t30a);
    c[o + 31 * s] = clip(min, max, t0 - t31);
}

/// Translation of `dav1d_inv_dct32_1d_c()`.
pub(crate) fn inv_dct32_1d(c: &mut [i32], o: usize, s: usize, min: i32, max: i32) {
    inv_dct32_1d_internal(c, o, s, min, max, false);
}

/// Translation of `dav1d_inv_dct64_1d_c()`.
pub(crate) fn inv_dct64_1d(c: &mut [i32], o: usize, s: usize, min: i32, max: i32) {
    inv_dct32_1d_internal(c, o, s << 1, min, max, true);
    let in1 = c[o + 1 * s];
    let in3 = c[o + 3 * s];
    let in5 = c[o + 5 * s];
    let in7 = c[o + 7 * s];
    let in9 = c[o + 9 * s];
    let in11 = c[o + 11 * s];
    let in13 = c[o + 13 * s];
    let in15 = c[o + 15 * s];
    let in17 = c[o + 17 * s];
    let in19 = c[o + 19 * s];
    let in21 = c[o + 21 * s];
    let in23 = c[o + 23 * s];
    let in25 = c[o + 25 * s];
    let in27 = c[o + 27 * s];
    let in29 = c[o + 29 * s];
    let in31 = c[o + 31 * s];
    let mut t32a = (in1 * 101 + 2048) >> 12;
    let mut t33a = (in31 * -2824 + 2048) >> 12;
    let mut t34a = (in17 * 1660 + 2048) >> 12;
    let mut t35a = (in15 * -1474 + 2048) >> 12;
    let mut t36a = (in9 * 897 + 2048) >> 12;
    let mut t37a = (in23 * -2191 + 2048) >> 12;
    let mut t38a = (in25 * 2359 + 2048) >> 12;
    let mut t39a = (in7 * -700 + 2048) >> 12;
    let mut t40a = (in5 * 501 + 2048) >> 12;
    let mut t41a = (in27 * -2520 + 2048) >> 12;
    let mut t42a = (in21 * 2019 + 2048) >> 12;
    let mut t43a = (in11 * -1092 + 2048) >> 12;
    let mut t44a = (in13 * 1285 + 2048) >> 12;
    let mut t45a = (in19 * -1842 + 2048) >> 12;
    let mut t46a = (in29 * 2675 + 2048) >> 12;
    let mut t47a = (in3 * -301 + 2048) >> 12;
    let mut t48a = (in3 * 4085 + 2048) >> 12;
    let mut t49a = (in29 * 3102 + 2048) >> 12;
    let mut t50a = (in19 * 3659 + 2048) >> 12;
    let mut t51a = (in13 * 3889 + 2048) >> 12;
    let mut t52a = (in11 * 3948 + 2048) >> 12;
    let mut t53a = (in21 * 3564 + 2048) >> 12;
    let mut t54a = (in27 * 3229 + 2048) >> 12;
    let mut t55a = (in5 * 4065 + 2048) >> 12;
    let mut t56a = (in7 * 4036 + 2048) >> 12;
    let mut t57a = (in25 * 3349 + 2048) >> 12;
    let mut t58a = (in23 * 3461 + 2048) >> 12;
    let mut t59a = (in9 * 3996 + 2048) >> 12;
    let mut t60a = (in15 * 3822 + 2048) >> 12;
    let mut t61a = (in17 * 3745 + 2048) >> 12;
    let mut t62a = (in31 * 2967 + 2048) >> 12;
    let mut t63a = (in1 * 4095 + 2048) >> 12;
    let mut t32 = clip(min, max, t32a + t33a);
    let mut t33 = clip(min, max, t32a - t33a);
    let mut t34 = clip(min, max, t35a - t34a);
    let mut t35 = clip(min, max, t35a + t34a);
    let mut t36 = clip(min, max, t36a + t37a);
    let mut t37 = clip(min, max, t36a - t37a);
    let mut t38 = clip(min, max, t39a - t38a);
    let mut t39 = clip(min, max, t39a + t38a);
    let mut t40 = clip(min, max, t40a + t41a);
    let mut t41 = clip(min, max, t40a - t41a);
    let mut t42 = clip(min, max, t43a - t42a);
    let mut t43 = clip(min, max, t43a + t42a);
    let mut t44 = clip(min, max, t44a + t45a);
    let mut t45 = clip(min, max, t44a - t45a);
    let mut t46 = clip(min, max, t47a - t46a);
    let mut t47 = clip(min, max, t47a + t46a);
    let mut t48 = clip(min, max, t48a + t49a);
    let mut t49 = clip(min, max, t48a - t49a);
    let mut t50 = clip(min, max, t51a - t50a);
    let mut t51 = clip(min, max, t51a + t50a);
    let mut t52 = clip(min, max, t52a + t53a);
    let mut t53 = clip(min, max, t52a - t53a);
    let mut t54 = clip(min, max, t55a - t54a);
    let mut t55 = clip(min, max, t55a + t54a);
    let mut t56 = clip(min, max, t56a + t57a);
    let mut t57 = clip(min, max, t56a - t57a);
    let mut t58 = clip(min, max, t59a - t58a);
    let mut t59 = clip(min, max, t59a + t58a);
    let mut t60 = clip(min, max, t60a + t61a);
    let mut t61 = clip(min, max, t60a - t61a);
    let mut t62 = clip(min, max, t63a - t62a);
    let mut t63 = clip(min, max, t63a + t62a);
    t33a = ((t33 * (4096 - 4076) + t62 * 401 + 2048) >> 12) - t33;
    t34a = ((t34 * -401 + t61 * (4096 - 4076) + 2048) >> 12) - t61;
    t37a = (t37 * -1299 + t58 * 1583 + 1024) >> 11;
    t38a = (t38 * -1583 + t57 * -1299 + 1024) >> 11;
    t41a = ((t41 * (4096 - 3612) + t54 * 1931 + 2048) >> 12) - t41;
    t42a = ((t42 * -1931 + t53 * (4096 - 3612) + 2048) >> 12) - t53;
    t45a = ((t45 * -1189 + t50 * (3920 - 4096) + 2048) >> 12) + t50;
    t46a = ((t46 * (4096 - 3920) + t49 * -1189 + 2048) >> 12) - t46;
    t49a = ((t46 * -1189 + t49 * (3920 - 4096) + 2048) >> 12) + t49;
    t50a = ((t45 * (3920 - 4096) + t50 * 1189 + 2048) >> 12) + t45;
    t53a = ((t42 * (4096 - 3612) + t53 * 1931 + 2048) >> 12) - t42;
    t54a = ((t41 * 1931 + t54 * (3612 - 4096) + 2048) >> 12) + t54;
    t57a = (t38 * -1299 + t57 * 1583 + 1024) >> 11;
    t58a = (t37 * 1583 + t58 * 1299 + 1024) >> 11;
    t61a = ((t34 * (4096 - 4076) + t61 * 401 + 2048) >> 12) - t34;
    t62a = ((t33 * 401 + t62 * (4076 - 4096) + 2048) >> 12) + t62;
    t32a = clip(min, max, t32 + t35);
    t33 = clip(min, max, t33a + t34a);
    t34 = clip(min, max, t33a - t34a);
    t35a = clip(min, max, t32 - t35);
    t36a = clip(min, max, t39 - t36);
    t37 = clip(min, max, t38a - t37a);
    t38 = clip(min, max, t38a + t37a);
    t39a = clip(min, max, t39 + t36);
    t40a = clip(min, max, t40 + t43);
    t41 = clip(min, max, t41a + t42a);
    t42 = clip(min, max, t41a - t42a);
    t43a = clip(min, max, t40 - t43);
    t44a = clip(min, max, t47 - t44);
    t45 = clip(min, max, t46a - t45a);
    t46 = clip(min, max, t46a + t45a);
    t47a = clip(min, max, t47 + t44);
    t48a = clip(min, max, t48 + t51);
    t49 = clip(min, max, t49a + t50a);
    t50 = clip(min, max, t49a - t50a);
    t51a = clip(min, max, t48 - t51);
    t52a = clip(min, max, t55 - t52);
    t53 = clip(min, max, t54a - t53a);
    t54 = clip(min, max, t54a + t53a);
    t55a = clip(min, max, t55 + t52);
    t56a = clip(min, max, t56 + t59);
    t57 = clip(min, max, t57a + t58a);
    t58 = clip(min, max, t57a - t58a);
    t59a = clip(min, max, t56 - t59);
    t60a = clip(min, max, t63 - t60);
    t61 = clip(min, max, t62a - t61a);
    t62 = clip(min, max, t62a + t61a);
    t63a = clip(min, max, t63 + t60);
    t34a = ((t34 * (4096 - 4017) + t61 * 799 + 2048) >> 12) - t34;
    t35 = ((t35a * (4096 - 4017) + t60a * 799 + 2048) >> 12) - t35a;
    t36 = ((t36a * -799 + t59a * (4096 - 4017) + 2048) >> 12) - t59a;
    t37a = ((t37 * -799 + t58 * (4096 - 4017) + 2048) >> 12) - t58;
    t42a = (t42 * -1138 + t53 * 1703 + 1024) >> 11;
    t43 = (t43a * -1138 + t52a * 1703 + 1024) >> 11;
    t44 = (t44a * -1703 + t51a * -1138 + 1024) >> 11;
    t45a = (t45 * -1703 + t50 * -1138 + 1024) >> 11;
    t50a = (t45 * -1138 + t50 * 1703 + 1024) >> 11;
    t51 = (t44a * -1138 + t51a * 1703 + 1024) >> 11;
    t52 = (t43a * 1703 + t52a * 1138 + 1024) >> 11;
    t53a = (t42 * 1703 + t53 * 1138 + 1024) >> 11;
    t58a = ((t37 * (4096 - 4017) + t58 * 799 + 2048) >> 12) - t37;
    t59 = ((t36a * (4096 - 4017) + t59a * 799 + 2048) >> 12) - t36a;
    t60 = ((t35a * 799 + t60a * (4017 - 4096) + 2048) >> 12) + t60a;
    t61a = ((t34 * 799 + t61 * (4017 - 4096) + 2048) >> 12) + t61;
    t32 = clip(min, max, t32a + t39a);
    t33a = clip(min, max, t33 + t38);
    t34 = clip(min, max, t34a + t37a);
    t35a = clip(min, max, t35 + t36);
    t36a = clip(min, max, t35 - t36);
    t37 = clip(min, max, t34a - t37a);
    t38a = clip(min, max, t33 - t38);
    t39 = clip(min, max, t32a - t39a);
    t40 = clip(min, max, t47a - t40a);
    t41a = clip(min, max, t46 - t41);
    t42 = clip(min, max, t45a - t42a);
    t43a = clip(min, max, t44 - t43);
    t44a = clip(min, max, t44 + t43);
    t45 = clip(min, max, t45a + t42a);
    t46a = clip(min, max, t46 + t41);
    t47 = clip(min, max, t47a + t40a);
    t48 = clip(min, max, t48a + t55a);
    t49a = clip(min, max, t49 + t54);
    t50 = clip(min, max, t50a + t53a);
    t51a = clip(min, max, t51 + t52);
    t52a = clip(min, max, t51 - t52);
    t53 = clip(min, max, t50a - t53a);
    t54a = clip(min, max, t49 - t54);
    t55 = clip(min, max, t48a - t55a);
    t56 = clip(min, max, t63a - t56a);
    t57a = clip(min, max, t62 - t57);
    t58 = clip(min, max, t61a - t58a);
    t59a = clip(min, max, t60 - t59);
    t60a = clip(min, max, t60 + t59);
    t61 = clip(min, max, t61a + t58a);
    t62a = clip(min, max, t62 + t57);
    t63 = clip(min, max, t63a + t56a);
    t36 = ((t36a * (4096 - 3784) + t59a * 1567 + 2048) >> 12) - t36a;
    t37a = ((t37 * (4096 - 3784) + t58 * 1567 + 2048) >> 12) - t37;
    t38 = ((t38a * (4096 - 3784) + t57a * 1567 + 2048) >> 12) - t38a;
    t39a = ((t39 * (4096 - 3784) + t56 * 1567 + 2048) >> 12) - t39;
    t40a = ((t40 * -1567 + t55 * (4096 - 3784) + 2048) >> 12) - t55;
    t41 = ((t41a * -1567 + t54a * (4096 - 3784) + 2048) >> 12) - t54a;
    t42a = ((t42 * -1567 + t53 * (4096 - 3784) + 2048) >> 12) - t53;
    t43 = ((t43a * -1567 + t52a * (4096 - 3784) + 2048) >> 12) - t52a;
    t52 = ((t43a * (4096 - 3784) + t52a * 1567 + 2048) >> 12) - t43a;
    t53a = ((t42 * (4096 - 3784) + t53 * 1567 + 2048) >> 12) - t42;
    t54 = ((t41a * (4096 - 3784) + t54a * 1567 + 2048) >> 12) - t41a;
    t55a = ((t40 * (4096 - 3784) + t55 * 1567 + 2048) >> 12) - t40;
    t56a = ((t39 * 1567 + t56 * (3784 - 4096) + 2048) >> 12) + t56;
    t57 = ((t38a * 1567 + t57a * (3784 - 4096) + 2048) >> 12) + t57a;
    t58a = ((t37 * 1567 + t58 * (3784 - 4096) + 2048) >> 12) + t58;
    t59 = ((t36a * 1567 + t59a * (3784 - 4096) + 2048) >> 12) + t59a;
    t32a = clip(min, max, t32 + t47);
    t33 = clip(min, max, t33a + t46a);
    t34a = clip(min, max, t34 + t45);
    t35 = clip(min, max, t35a + t44a);
    t36a = clip(min, max, t36 + t43);
    t37 = clip(min, max, t37a + t42a);
    t38a = clip(min, max, t38 + t41);
    t39 = clip(min, max, t39a + t40a);
    t40 = clip(min, max, t39a - t40a);
    t41a = clip(min, max, t38 - t41);
    t42 = clip(min, max, t37a - t42a);
    t43a = clip(min, max, t36 - t43);
    t44 = clip(min, max, t35a - t44a);
    t45a = clip(min, max, t34 - t45);
    t46 = clip(min, max, t33a - t46a);
    t47a = clip(min, max, t32 - t47);
    t48a = clip(min, max, t63 - t48);
    t49 = clip(min, max, t62a - t49a);
    t50a = clip(min, max, t61 - t50);
    t51 = clip(min, max, t60a - t51a);
    t52a = clip(min, max, t59 - t52);
    t53 = clip(min, max, t58a - t53a);
    t54a = clip(min, max, t57 - t54);
    t55 = clip(min, max, t56a - t55a);
    t56 = clip(min, max, t56a + t55a);
    t57a = clip(min, max, t57 + t54);
    t58 = clip(min, max, t58a + t53a);
    t59a = clip(min, max, t59 + t52);
    t60 = clip(min, max, t60a + t51a);
    t61a = clip(min, max, t61 + t50);
    t62 = clip(min, max, t62a + t49a);
    t63a = clip(min, max, t63 + t48);
    t40a = ((t55 - t40) * 181 + 128) >> 8;
    t41 = ((t54a - t41a) * 181 + 128) >> 8;
    t42a = ((t53 - t42) * 181 + 128) >> 8;
    t43 = ((t52a - t43a) * 181 + 128) >> 8;
    t44a = ((t51 - t44) * 181 + 128) >> 8;
    t45 = ((t50a - t45a) * 181 + 128) >> 8;
    t46a = ((t49 - t46) * 181 + 128) >> 8;
    t47 = ((t48a - t47a) * 181 + 128) >> 8;
    t48 = ((t47a + t48a) * 181 + 128) >> 8;
    t49a = ((t46 + t49) * 181 + 128) >> 8;
    t50 = ((t45a + t50a) * 181 + 128) >> 8;
    t51a = ((t44 + t51) * 181 + 128) >> 8;
    t52 = ((t43a + t52a) * 181 + 128) >> 8;
    t53a = ((t42 + t53) * 181 + 128) >> 8;
    t54 = ((t41a + t54a) * 181 + 128) >> 8;
    t55a = ((t40 + t55) * 181 + 128) >> 8;
    let t0 = c[o + 0 * s];
    let t1 = c[o + 2 * s];
    let t2 = c[o + 4 * s];
    let t3 = c[o + 6 * s];
    let t4 = c[o + 8 * s];
    let t5 = c[o + 10 * s];
    let t6 = c[o + 12 * s];
    let t7 = c[o + 14 * s];
    let t8 = c[o + 16 * s];
    let t9 = c[o + 18 * s];
    let t10 = c[o + 20 * s];
    let t11 = c[o + 22 * s];
    let t12 = c[o + 24 * s];
    let t13 = c[o + 26 * s];
    let t14 = c[o + 28 * s];
    let t15 = c[o + 30 * s];
    let t16 = c[o + 32 * s];
    let t17 = c[o + 34 * s];
    let t18 = c[o + 36 * s];
    let t19 = c[o + 38 * s];
    let t20 = c[o + 40 * s];
    let t21 = c[o + 42 * s];
    let t22 = c[o + 44 * s];
    let t23 = c[o + 46 * s];
    let t24 = c[o + 48 * s];
    let t25 = c[o + 50 * s];
    let t26 = c[o + 52 * s];
    let t27 = c[o + 54 * s];
    let t28 = c[o + 56 * s];
    let t29 = c[o + 58 * s];
    let t30 = c[o + 60 * s];
    let t31 = c[o + 62 * s];
    c[o + 0 * s] = clip(min, max, t0 + t63a);
    c[o + 1 * s] = clip(min, max, t1 + t62);
    c[o + 2 * s] = clip(min, max, t2 + t61a);
    c[o + 3 * s] = clip(min, max, t3 + t60);
    c[o + 4 * s] = clip(min, max, t4 + t59a);
    c[o + 5 * s] = clip(min, max, t5 + t58);
    c[o + 6 * s] = clip(min, max, t6 + t57a);
    c[o + 7 * s] = clip(min, max, t7 + t56);
    c[o + 8 * s] = clip(min, max, t8 + t55a);
    c[o + 9 * s] = clip(min, max, t9 + t54);
    c[o + 10 * s] = clip(min, max, t10 + t53a);
    c[o + 11 * s] = clip(min, max, t11 + t52);
    c[o + 12 * s] = clip(min, max, t12 + t51a);
    c[o + 13 * s] = clip(min, max, t13 + t50);
    c[o + 14 * s] = clip(min, max, t14 + t49a);
    c[o + 15 * s] = clip(min, max, t15 + t48);
    c[o + 16 * s] = clip(min, max, t16 + t47);
    c[o + 17 * s] = clip(min, max, t17 + t46a);
    c[o + 18 * s] = clip(min, max, t18 + t45);
    c[o + 19 * s] = clip(min, max, t19 + t44a);
    c[o + 20 * s] = clip(min, max, t20 + t43);
    c[o + 21 * s] = clip(min, max, t21 + t42a);
    c[o + 22 * s] = clip(min, max, t22 + t41);
    c[o + 23 * s] = clip(min, max, t23 + t40a);
    c[o + 24 * s] = clip(min, max, t24 + t39);
    c[o + 25 * s] = clip(min, max, t25 + t38a);
    c[o + 26 * s] = clip(min, max, t26 + t37);
    c[o + 27 * s] = clip(min, max, t27 + t36a);
    c[o + 28 * s] = clip(min, max, t28 + t35);
    c[o + 29 * s] = clip(min, max, t29 + t34a);
    c[o + 30 * s] = clip(min, max, t30 + t33);
    c[o + 31 * s] = clip(min, max, t31 + t32a);
    c[o + 32 * s] = clip(min, max, t31 - t32a);
    c[o + 33 * s] = clip(min, max, t30 - t33);
    c[o + 34 * s] = clip(min, max, t29 - t34a);
    c[o + 35 * s] = clip(min, max, t28 - t35);
    c[o + 36 * s] = clip(min, max, t27 - t36a);
    c[o + 37 * s] = clip(min, max, t26 - t37);
    c[o + 38 * s] = clip(min, max, t25 - t38a);
    c[o + 39 * s] = clip(min, max, t24 - t39);
    c[o + 40 * s] = clip(min, max, t23 - t40a);
    c[o + 41 * s] = clip(min, max, t22 - t41);
    c[o + 42 * s] = clip(min, max, t21 - t42a);
    c[o + 43 * s] = clip(min, max, t20 - t43);
    c[o + 44 * s] = clip(min, max, t19 - t44a);
    c[o + 45 * s] = clip(min, max, t18 - t45);
    c[o + 46 * s] = clip(min, max, t17 - t46a);
    c[o + 47 * s] = clip(min, max, t16 - t47);
    c[o + 48 * s] = clip(min, max, t15 - t48);
    c[o + 49 * s] = clip(min, max, t14 - t49a);
    c[o + 50 * s] = clip(min, max, t13 - t50);
    c[o + 51 * s] = clip(min, max, t12 - t51a);
    c[o + 52 * s] = clip(min, max, t11 - t52);
    c[o + 53 * s] = clip(min, max, t10 - t53a);
    c[o + 54 * s] = clip(min, max, t9 - t54);
    c[o + 55 * s] = clip(min, max, t8 - t55a);
    c[o + 56 * s] = clip(min, max, t7 - t56);
    c[o + 57 * s] = clip(min, max, t6 - t57a);
    c[o + 58 * s] = clip(min, max, t5 - t58);
    c[o + 59 * s] = clip(min, max, t4 - t59a);
    c[o + 60 * s] = clip(min, max, t3 - t60);
    c[o + 61 * s] = clip(min, max, t2 - t61a);
    c[o + 62 * s] = clip(min, max, t1 - t62);
    c[o + 63 * s] = clip(min, max, t0 - t63a);
}

#[inline(never)]
fn inv_adst4_1d_internal(
    inp: &[i32],
    io: usize,
    is: usize,
    min: i32,
    max: i32,
    out: &mut [i32],
    oo: usize,
    os: isize,
) {
    let _ = (min, max);
    let in0 = inp[io + 0 * is];
    let in1 = inp[io + 1 * is];
    let in2 = inp[io + 2 * is];
    let in3 = inp[io + 3 * is];
    out[ix(oo, 0 * os)] =
        ((1321 * in0 + (3803 - 4096) * in2 + (2482 - 4096) * in3 + (3344 - 4096) * in1 + 2048)
            >> 12)
            + in2
            + in3
            + in1;
    out[ix(oo, 1 * os)] =
        (((2482 - 4096) * in0 - 1321 * in2 - (3803 - 4096) * in3 + (3344 - 4096) * in1 + 2048)
            >> 12)
            + in0
            - in3
            + in1;
    out[ix(oo, 2 * os)] = (209 * (in0 - in2 + in3) + 128) >> 8;
    out[ix(oo, 3 * os)] =
        (((3803 - 4096) * in0 + (2482 - 4096) * in2 - 1321 * in3 - (3344 - 4096) * in1 + 2048)
            >> 12)
            + in0
            + in2
            - in1;
}

#[inline(never)]
fn inv_adst8_1d_internal(
    inp: &[i32],
    io: usize,
    is: usize,
    min: i32,
    max: i32,
    out: &mut [i32],
    oo: usize,
    os: isize,
) {
    let in0 = inp[io + 0 * is];
    let in1 = inp[io + 1 * is];
    let in2 = inp[io + 2 * is];
    let in3 = inp[io + 3 * is];
    let in4 = inp[io + 4 * is];
    let in5 = inp[io + 5 * is];
    let in6 = inp[io + 6 * is];
    let in7 = inp[io + 7 * is];
    let t0a = (((4076 - 4096) * in7 + 401 * in0 + 2048) >> 12) + in7;
    let t1a = ((401 * in7 - (4076 - 4096) * in0 + 2048) >> 12) - in0;
    let t2a = (((3612 - 4096) * in5 + 1931 * in2 + 2048) >> 12) + in5;
    let t3a = ((1931 * in5 - (3612 - 4096) * in2 + 2048) >> 12) - in2;
    let mut t4a = (1299 * in3 + 1583 * in4 + 1024) >> 11;
    let mut t5a = (1583 * in3 - 1299 * in4 + 1024) >> 11;
    let mut t6a = ((1189 * in1 + (3920 - 4096) * in6 + 2048) >> 12) + in6;
    let mut t7a = (((3920 - 4096) * in1 - 1189 * in6 + 2048) >> 12) + in1;
    let t0 = clip(min, max, t0a + t4a);
    let t1 = clip(min, max, t1a + t5a);
    let mut t2 = clip(min, max, t2a + t6a);
    let mut t3 = clip(min, max, t3a + t7a);
    let t4 = clip(min, max, t0a - t4a);
    let t5 = clip(min, max, t1a - t5a);
    let mut t6 = clip(min, max, t2a - t6a);
    let mut t7 = clip(min, max, t3a - t7a);
    t4a = (((3784 - 4096) * t4 + 1567 * t5 + 2048) >> 12) + t4;
    t5a = ((1567 * t4 - (3784 - 4096) * t5 + 2048) >> 12) - t5;
    t6a = (((3784 - 4096) * t7 - 1567 * t6 + 2048) >> 12) + t7;
    t7a = ((1567 * t7 + (3784 - 4096) * t6 + 2048) >> 12) + t6;
    out[ix(oo, 0 * os)] = clip(min, max, t0 + t2);
    out[ix(oo, 7 * os)] = -clip(min, max, t1 + t3);
    t2 = clip(min, max, t0 - t2);
    t3 = clip(min, max, t1 - t3);
    out[ix(oo, 1 * os)] = -clip(min, max, t4a + t6a);
    out[ix(oo, 6 * os)] = clip(min, max, t5a + t7a);
    t6 = clip(min, max, t4a - t6a);
    t7 = clip(min, max, t5a - t7a);
    out[ix(oo, 3 * os)] = -(((t2 + t3) * 181 + 128) >> 8);
    out[ix(oo, 4 * os)] = ((t2 - t3) * 181 + 128) >> 8;
    out[ix(oo, 2 * os)] = ((t6 + t7) * 181 + 128) >> 8;
    out[ix(oo, 5 * os)] = -(((t6 - t7) * 181 + 128) >> 8);
}

#[inline(never)]
fn inv_adst16_1d_internal(
    inp: &[i32],
    io: usize,
    is: usize,
    min: i32,
    max: i32,
    out: &mut [i32],
    oo: usize,
    os: isize,
) {
    let in0 = inp[io + 0 * is];
    let in1 = inp[io + 1 * is];
    let in2 = inp[io + 2 * is];
    let in3 = inp[io + 3 * is];
    let in4 = inp[io + 4 * is];
    let in5 = inp[io + 5 * is];
    let in6 = inp[io + 6 * is];
    let in7 = inp[io + 7 * is];
    let in8 = inp[io + 8 * is];
    let in9 = inp[io + 9 * is];
    let in10 = inp[io + 10 * is];
    let in11 = inp[io + 11 * is];
    let in12 = inp[io + 12 * is];
    let in13 = inp[io + 13 * is];
    let in14 = inp[io + 14 * is];
    let in15 = inp[io + 15 * is];
    let mut t0 = ((in15 * (4091 - 4096) + in0 * 201 + 2048) >> 12) + in15;
    let mut t1 = ((in15 * 201 - in0 * (4091 - 4096) + 2048) >> 12) - in0;
    let mut t2 = ((in13 * (3973 - 4096) + in2 * 995 + 2048) >> 12) + in13;
    let mut t3 = ((in13 * 995 - in2 * (3973 - 4096) + 2048) >> 12) - in2;
    let mut t4 = ((in11 * (3703 - 4096) + in4 * 1751 + 2048) >> 12) + in11;
    let mut t5 = ((in11 * 1751 - in4 * (3703 - 4096) + 2048) >> 12) - in4;
    let mut t6 = (in9 * 1645 + in6 * 1220 + 1024) >> 11;
    let mut t7 = (in9 * 1220 - in6 * 1645 + 1024) >> 11;
    let mut t8 = ((in7 * 2751 + in8 * (3035 - 4096) + 2048) >> 12) + in8;
    let mut t9 = ((in7 * (3035 - 4096) - in8 * 2751 + 2048) >> 12) + in7;
    let mut t10 = ((in5 * 2106 + in10 * (3513 - 4096) + 2048) >> 12) + in10;
    let mut t11 = ((in5 * (3513 - 4096) - in10 * 2106 + 2048) >> 12) + in5;
    let mut t12 = ((in3 * 1380 + in12 * (3857 - 4096) + 2048) >> 12) + in12;
    let mut t13 = ((in3 * (3857 - 4096) - in12 * 1380 + 2048) >> 12) + in3;
    let mut t14 = ((in1 * 601 + in14 * (4052 - 4096) + 2048) >> 12) + in14;
    let mut t15 = ((in1 * (4052 - 4096) - in14 * 601 + 2048) >> 12) + in1;
    let t0a = clip(min, max, t0 + t8);
    let t1a = clip(min, max, t1 + t9);
    let mut t2a = clip(min, max, t2 + t10);
    let mut t3a = clip(min, max, t3 + t11);
    let mut t4a = clip(min, max, t4 + t12);
    let mut t5a = clip(min, max, t5 + t13);
    let mut t6a = clip(min, max, t6 + t14);
    let mut t7a = clip(min, max, t7 + t15);
    let mut t8a = clip(min, max, t0 - t8);
    let mut t9a = clip(min, max, t1 - t9);
    let mut t10a = clip(min, max, t2 - t10);
    let mut t11a = clip(min, max, t3 - t11);
    let mut t12a = clip(min, max, t4 - t12);
    let mut t13a = clip(min, max, t5 - t13);
    let mut t14a = clip(min, max, t6 - t14);
    let mut t15a = clip(min, max, t7 - t15);
    t8 = ((t8a * (4017 - 4096) + t9a * 799 + 2048) >> 12) + t8a;
    t9 = ((t8a * 799 - t9a * (4017 - 4096) + 2048) >> 12) - t9a;
    t10 = ((t10a * 2276 + t11a * (3406 - 4096) + 2048) >> 12) + t11a;
    t11 = ((t10a * (3406 - 4096) - t11a * 2276 + 2048) >> 12) + t10a;
    t12 = ((t13a * (4017 - 4096) - t12a * 799 + 2048) >> 12) + t13a;
    t13 = ((t13a * 799 + t12a * (4017 - 4096) + 2048) >> 12) + t12a;
    t14 = ((t15a * 2276 - t14a * (3406 - 4096) + 2048) >> 12) - t14a;
    t15 = ((t15a * (3406 - 4096) + t14a * 2276 + 2048) >> 12) + t15a;
    t0 = clip(min, max, t0a + t4a);
    t1 = clip(min, max, t1a + t5a);
    t2 = clip(min, max, t2a + t6a);
    t3 = clip(min, max, t3a + t7a);
    t4 = clip(min, max, t0a - t4a);
    t5 = clip(min, max, t1a - t5a);
    t6 = clip(min, max, t2a - t6a);
    t7 = clip(min, max, t3a - t7a);
    t8a = clip(min, max, t8 + t12);
    t9a = clip(min, max, t9 + t13);
    t10a = clip(min, max, t10 + t14);
    t11a = clip(min, max, t11 + t15);
    t12a = clip(min, max, t8 - t12);
    t13a = clip(min, max, t9 - t13);
    t14a = clip(min, max, t10 - t14);
    t15a = clip(min, max, t11 - t15);
    t4a = ((t4 * (3784 - 4096) + t5 * 1567 + 2048) >> 12) + t4;
    t5a = ((t4 * 1567 - t5 * (3784 - 4096) + 2048) >> 12) - t5;
    t6a = ((t7 * (3784 - 4096) - t6 * 1567 + 2048) >> 12) + t7;
    t7a = ((t7 * 1567 + t6 * (3784 - 4096) + 2048) >> 12) + t6;
    t12 = ((t12a * (3784 - 4096) + t13a * 1567 + 2048) >> 12) + t12a;
    t13 = ((t12a * 1567 - t13a * (3784 - 4096) + 2048) >> 12) - t13a;
    t14 = ((t15a * (3784 - 4096) - t14a * 1567 + 2048) >> 12) + t15a;
    t15 = ((t15a * 1567 + t14a * (3784 - 4096) + 2048) >> 12) + t14a;
    out[ix(oo, 0 * os)] = clip(min, max, t0 + t2);
    out[ix(oo, 15 * os)] = -clip(min, max, t1 + t3);
    t2a = clip(min, max, t0 - t2);
    t3a = clip(min, max, t1 - t3);
    out[ix(oo, 3 * os)] = -clip(min, max, t4a + t6a);
    out[ix(oo, 12 * os)] = clip(min, max, t5a + t7a);
    t6 = clip(min, max, t4a - t6a);
    t7 = clip(min, max, t5a - t7a);
    out[ix(oo, 1 * os)] = -clip(min, max, t8a + t10a);
    out[ix(oo, 14 * os)] = clip(min, max, t9a + t11a);
    t10 = clip(min, max, t8a - t10a);
    t11 = clip(min, max, t9a - t11a);
    out[ix(oo, 2 * os)] = clip(min, max, t12 + t14);
    out[ix(oo, 13 * os)] = -clip(min, max, t13 + t15);
    t14a = clip(min, max, t12 - t14);
    t15a = clip(min, max, t13 - t15);
    out[ix(oo, 7 * os)] = -(((t2a + t3a) * 181 + 128) >> 8);
    out[ix(oo, 8 * os)] = ((t2a - t3a) * 181 + 128) >> 8;
    out[ix(oo, 4 * os)] = ((t6 + t7) * 181 + 128) >> 8;
    out[ix(oo, 11 * os)] = -(((t6 - t7) * 181 + 128) >> 8);
    out[ix(oo, 6 * os)] = ((t10 + t11) * 181 + 128) >> 8;
    out[ix(oo, 9 * os)] = -(((t10 - t11) * 181 + 128) >> 8);
    out[ix(oo, 5 * os)] = -(((t14a + t15a) * 181 + 128) >> 8);
    out[ix(oo, 10 * os)] = ((t14a - t15a) * 181 + 128) >> 8;
}

// #define inv_adst_1d(sz): the input and the output are the same buffer, so
// the input is copied first (the internal functions read all of it before
// writing).
macro_rules! inv_adst_1d {
    ($sz:literal, $adst:ident, $flipadst:ident, $internal:ident) => {
        #[doc = concat!("Translation of `dav1d_inv_adst", $sz, "_1d_c()`.")]
        pub(crate) fn $adst(c: &mut [i32], o: usize, s: usize, min: i32, max: i32) {
            let mut inp = [0i32; $sz];
            for k in 0..$sz {
                inp[k] = c[o + k * s];
            }
            $internal(&inp, 0, 1, min, max, c, o, s as isize);
        }
        #[doc = concat!("Translation of `dav1d_inv_flipadst", $sz, "_1d_c()`.")]
        pub(crate) fn $flipadst(c: &mut [i32], o: usize, s: usize, min: i32, max: i32) {
            let mut inp = [0i32; $sz];
            for k in 0..$sz {
                inp[k] = c[o + k * s];
            }
            $internal(&inp, 0, 1, min, max, c, o + ($sz - 1) * s, -(s as isize));
        }
    };
}

inv_adst_1d!(4, inv_adst4_1d, inv_flipadst4_1d, inv_adst4_1d_internal);
inv_adst_1d!(8, inv_adst8_1d, inv_flipadst8_1d, inv_adst8_1d_internal);
inv_adst_1d!(16, inv_adst16_1d, inv_flipadst16_1d, inv_adst16_1d_internal);

/// Translation of `dav1d_inv_identity4_1d_c()`.
pub(crate) fn inv_identity4_1d(c: &mut [i32], o: usize, s: usize, _min: i32, _max: i32) {
    for i in 0..4 {
        let r#in = c[o + s * i];
        c[o + s * i] = r#in + ((r#in * 1697 + 2048) >> 12);
    }
}

/// Translation of `dav1d_inv_identity8_1d_c()`.
pub(crate) fn inv_identity8_1d(c: &mut [i32], o: usize, s: usize, _min: i32, _max: i32) {
    for i in 0..8 {
        c[o + s * i] *= 2;
    }
}

/// Translation of `dav1d_inv_identity16_1d_c()`.
pub(crate) fn inv_identity16_1d(c: &mut [i32], o: usize, s: usize, _min: i32, _max: i32) {
    for i in 0..16 {
        let r#in = c[o + s * i];
        c[o + s * i] = 2 * r#in + ((r#in * 1697 + 1024) >> 11);
    }
}

/// Translation of `dav1d_inv_identity32_1d_c()`.
pub(crate) fn inv_identity32_1d(c: &mut [i32], o: usize, s: usize, _min: i32, _max: i32) {
    for i in 0..32 {
        c[o + s * i] *= 4;
    }
}

/// Translation of `dav1d_inv_wht4_1d_c()`.
pub(crate) fn inv_wht4_1d(c: &mut [i32], o: usize, s: usize) {
    let in0 = c[o + 0 * s];
    let in1 = c[o + 1 * s];
    let in2 = c[o + 2 * s];
    let in3 = c[o + 3 * s];
    let t0 = in0 + in1;
    let t2 = in2 - in3;
    let t4 = (t0 - t2) >> 1;
    let t3 = t4 - in3;
    let t1 = t4 - in1;
    c[o + 0 * s] = t0 - t3;
    c[o + 1 * s] = t3;
    c[o + 2 * s] = t1;
    c[o + 3 * s] = t2 + t1;
}
