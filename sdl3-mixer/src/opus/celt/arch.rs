// Rust translation of celt/arch.h and celt/os_support.h from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (C) 2003-2008 Jean-Marc Valin
// Copyright (C) 2007-2009 Xiph.Org Foundation
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Various architecture definitions for CELT, in the floating-point build
//! (`FIXED_POINT` undefined), which is what SDL_mixer's CMake builds.
//!
//! In the float build most of the fixed-point macros (`SHR32()`,
//! `EXTEND32()`, `PSHR32()`, `SATURATE()`, `QCONST16()`...) are the
//! identity; the translations leave them out and write the remaining
//! arithmetic macros (`MULT16_16()`, `MAC16_16()`, `HALF32()`...) as the
//! float expressions they expand to, in the same order of operations (and
//! in `f64` where a `double` constant promotes them). `MIN16()` and
//! friends are the functions below: C's `(a)<(b)?(a):(b)`, which treats
//! NaNs differently from `f32::min()`.
//!
//! `celt_assert()` is fatal in upstream's hardened build
//! (`ENABLE_HARDENING`, which SDL_mixer's CMake leaves on); the assertions
//! checked here hold for any input, so they are `debug_assert!()`s.
//! `OPUS_COPY()`, `OPUS_MOVE()` and `OPUS_CLEAR()` are slice copies and
//! fills.

/// `celt_sig`: `float` in the float build, as are `opus_val16`,
/// `opus_val32`, `celt_norm` and `celt_ener` (written `f32` here).
pub(crate) type CeltSig = f32;

pub(crate) const CELT_SIG_SCALE: f32 = 32768.0;

pub(crate) const Q15ONE: f32 = 1.0;

pub(crate) const EPSILON: f32 = 1e-15;
pub(crate) const VERY_SMALL: f32 = 1e-30;

/// Minimum 16-bit value. Translation of `MIN16()`.
#[inline(always)]
pub(crate) fn min16(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}

/// Maximum 16-bit value. Translation of `MAX16()`.
#[inline(always)]
pub(crate) fn max16(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

/// Minimum 32-bit value. Translation of `MIN32()`.
#[inline(always)]
pub(crate) fn min32(a: f32, b: f32) -> f32 {
    min16(a, b)
}

/// Maximum 32-bit value. Translation of `MAX32()`.
#[inline(always)]
pub(crate) fn max32(a: f32, b: f32) -> f32 {
    max16(a, b)
}

/// Minimum int value. Translation of `IMIN()`.
#[inline(always)]
pub(crate) fn imin(a: i32, b: i32) -> i32 {
    if a < b {
        a
    } else {
        b
    }
}

/// Maximum int value. Translation of `IMAX()`.
#[inline(always)]
pub(crate) fn imax(a: i32, b: i32) -> i32 {
    if a > b {
        a
    } else {
        b
    }
}

/// Translation of `SCALEOUT()`: `(a)*(1/CELT_SIG_SCALE)`.
#[inline(always)]
pub(crate) fn scaleout(a: f32) -> f32 {
    a * (1.0 / CELT_SIG_SCALE)
}

/// Translation of `celt_isnan()` (the non-`FLOAT_APPROX` version).
#[inline(always)]
#[allow(dead_code)]
pub(crate) fn celt_isnan(x: f32) -> bool {
    x.is_nan()
}
