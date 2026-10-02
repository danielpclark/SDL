// Rust translation of src/libm/ and the math functions of src/stdlib/SDL_stdlib.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.
//
// The libm routines are from fdlibm (via uClibc) and carry Sun's notice:
//
// ====================================================
// Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.
//
// Developed at SunPro, a Sun Microsystems, Inc. business.
// Permission to use, copy, modify, and distribute this
// software is freely granted, provided that this notice
// is preserved.
// ====================================================

//! SDL's portable math functions (`SDL_sin`, `SDL_pow`, ...).
//!
//! Upstream calls the platform C library when it has the function and falls
//! back to its bundled fdlibm (`src/libm/`) otherwise. This translation
//! always uses the fdlibm code, so results are **bit-identical on every
//! platform** and identical to a C SDL built without a system libm. The
//! `f32` variants compute in `f64` and round, exactly like upstream's
//! fallbacks (`(float)SDL_sin((double)x)`).
//!
//! For everyday arithmetic Rust's inherent float methods are fine; use these
//! when reproducibility across platforms matters (simulations, replays,
//! lock-step networking) or when matching C SDL output.

// fdlibm's constants are written with more digits than f64 holds; they are
// kept exactly as upstream wrote them (the bit-identity test checks them).
#![allow(clippy::excessive_precision)]
// fdlibm idioms kept verbatim: `(x-x)/(x-x)` to manufacture NaN, its own
// copies of ln2/log10(e)/..., declare-then-assign variables, and the ASCII
// formula layout of its comments.
#![allow(
    clippy::approx_constant,
    clippy::eq_op,
    clippy::needless_late_init,
    clippy::doc_overindented_list_items,
    clippy::doc_lazy_continuation,
    clippy::collapsible_else_if,
    clippy::needless_return,
    clippy::neg_multiply,
    clippy::zero_divided_by_zero,
    clippy::misrefactored_assign_op,
    clippy::explicit_counter_loop
)]

mod exp_log;
mod kernels;
mod misc;
mod rem_pio2;
mod trig;
mod words;

/// The value of Pi, as a double-precision floating point literal. Translation of `SDL_PI_D`.
pub const PI_D: f64 = 3.141592653589793238462643383279502884;
/// The value of Pi, as a single-precision floating point literal. Translation of `SDL_PI_F`.
pub const PI_F: f32 = 3.141592653589793238462643383279502884;

/// Arc tangent of `x`, in radians. Translation of `SDL_atan()`.
pub fn atan(x: f64) -> f64 {
    trig::atan(x)
}

/// Translation of `SDL_atanf()`.
pub fn atanf(x: f32) -> f32 {
    atan(x as f64) as f32
}

/// Arc tangent of `y / x`, using the signs to pick the quadrant. Translation of `SDL_atan2()`.
pub fn atan2(y: f64, x: f64) -> f64 {
    trig::atan2(y, x)
}

/// Translation of `SDL_atan2f()`.
pub fn atan2f(y: f32, x: f32) -> f32 {
    atan2(y as f64, x as f64) as f32
}

/// Arc cosine of `val`, in radians. Translation of `SDL_acos()`.
pub fn acos(val: f64) -> f64 {
    let mut result;
    if val == -1.0 {
        result = PI_D;
    } else {
        result = atan(sqrt(1.0 - val * val) / val);
        if result < 0.0 {
            result += PI_D;
        }
    }
    result
}

/// Translation of `SDL_acosf()`.
pub fn acosf(val: f32) -> f32 {
    acos(val as f64) as f32
}

/// Arc sine of `val`, in radians. Translation of `SDL_asin()`.
pub fn asin(val: f64) -> f64 {
    if val == -1.0 {
        -(PI_D / 2.0)
    } else {
        (PI_D / 2.0) - acos(val)
    }
}

/// Translation of `SDL_asinf()`.
pub fn asinf(val: f32) -> f32 {
    asin(val as f64) as f32
}

/// Smallest integral value not less than `x`. Translation of `SDL_ceil()`.
pub fn ceil(x: f64) -> f64 {
    let mut integer = floor(x);
    let fraction = x - integer;
    if fraction > 0.0 {
        integer += 1.0;
    }
    integer
}

/// Translation of `SDL_ceilf()`.
pub fn ceilf(x: f32) -> f32 {
    ceil(x as f64) as f32
}

/// `x` with the sign of `y`. Translation of `SDL_copysign()`.
pub fn copysign(x: f64, y: f64) -> f64 {
    misc::copysign(x, y)
}

/// Translation of `SDL_copysignf()`.
pub fn copysignf(x: f32, y: f32) -> f32 {
    copysign(x as f64, y as f64) as f32
}

/// Cosine of `x` radians. Translation of `SDL_cos()`.
pub fn cos(x: f64) -> f64 {
    trig::cos(x)
}

/// Translation of `SDL_cosf()`.
pub fn cosf(x: f32) -> f32 {
    cos(x as f64) as f32
}

/// e raised to `x`. Translation of `SDL_exp()`.
pub fn exp(x: f64) -> f64 {
    exp_log::exp(x)
}

/// Translation of `SDL_expf()`.
pub fn expf(x: f32) -> f32 {
    exp(x as f64) as f32
}

/// Absolute value. Translation of `SDL_fabs()`.
pub fn fabs(x: f64) -> f64 {
    misc::fabs(x)
}

/// Translation of `SDL_fabsf()`.
pub fn fabsf(x: f32) -> f32 {
    fabs(x as f64) as f32
}

/// Largest integral value not greater than `x`. Translation of `SDL_floor()`.
pub fn floor(x: f64) -> f64 {
    misc::floor(x)
}

/// Translation of `SDL_floorf()`.
pub fn floorf(x: f32) -> f32 {
    floor(x as f64) as f32
}

/// `x` rounded toward zero. Translation of `SDL_trunc()`.
pub fn trunc(x: f64) -> f64 {
    if x >= 0.0 {
        floor(x)
    } else {
        ceil(x)
    }
}

/// Translation of `SDL_truncf()`.
pub fn truncf(x: f32) -> f32 {
    trunc(x as f64) as f32
}

/// Floating-point remainder of `x / y`, with the sign of `x`. Translation of `SDL_fmod()`.
pub fn fmod(x: f64, y: f64) -> f64 {
    misc::fmod(x, y)
}

/// Translation of `SDL_fmodf()`.
pub fn fmodf(x: f32, y: f32) -> f32 {
    fmod(x as f64, y as f64) as f32
}

/// Whether `x` is positive or negative infinity. Translation of `SDL_isinf()`.
pub fn isinf(x: f64) -> bool {
    misc::isinf(x) != 0
}

/// Translation of `SDL_isinff()`.
pub fn isinff(x: f32) -> bool {
    misc::isinff(x) != 0
}

/// Whether `x` is a NaN. Translation of `SDL_isnan()`.
pub fn isnan(x: f64) -> bool {
    misc::isnan(x) != 0
}

/// Translation of `SDL_isnanf()`.
pub fn isnanf(x: f32) -> bool {
    misc::isnanf(x) != 0
}

/// Natural logarithm. Translation of `SDL_log()`.
pub fn log(x: f64) -> f64 {
    exp_log::log(x)
}

/// Translation of `SDL_logf()`.
pub fn logf(x: f32) -> f32 {
    log(x as f64) as f32
}

/// Base-10 logarithm. Translation of `SDL_log10()`.
pub fn log10(x: f64) -> f64 {
    exp_log::log10(x)
}

/// Translation of `SDL_log10f()`.
pub fn log10f(x: f32) -> f32 {
    log10(x as f64) as f32
}

/// Split `x` into its fractional and integral parts, both with the sign of
/// `x`: returns `(fraction, integral)`. Translation of `SDL_modf()`.
pub fn modf(x: f64) -> (f64, f64) {
    misc::modf(x)
}

/// Translation of `SDL_modff()`.
pub fn modff(x: f32) -> (f32, f32) {
    let (double_result, double_y) = modf(x as f64);
    (double_result as f32, double_y as f32)
}

/// `x` raised to the power `y`. Translation of `SDL_pow()`.
pub fn pow(x: f64, y: f64) -> f64 {
    exp_log::pow(x, y)
}

/// Translation of `SDL_powf()`.
pub fn powf(x: f32, y: f32) -> f32 {
    pow(x as f64, y as f64) as f32
}

/// Round to the nearest integral value, halfway cases away from zero. Translation of `SDL_round()`.
pub fn round(arg: f64) -> f64 {
    if arg >= 0.0 {
        floor(arg + 0.5)
    } else {
        ceil(arg - 0.5)
    }
}

/// Translation of `SDL_roundf()`.
pub fn roundf(arg: f32) -> f32 {
    round(arg as f64) as f32
}

/// [`round`] converted to an integer (saturating where C would be undefined).
/// Translation of `SDL_lround()`.
pub fn lround(arg: f64) -> i64 {
    round(arg) as i64
}

/// Translation of `SDL_lroundf()`.
pub fn lroundf(arg: f32) -> i64 {
    round(arg as f64) as i64
}

/// `x * 2^n`. Translation of `SDL_scalbn()`.
pub fn scalbn(x: f64, n: i32) -> f64 {
    misc::scalbn(x, n)
}

/// Translation of `SDL_scalbnf()`.
pub fn scalbnf(x: f32, n: i32) -> f32 {
    scalbn(x as f64, n) as f32
}

/// Sine of `x` radians. Translation of `SDL_sin()`.
pub fn sin(x: f64) -> f64 {
    trig::sin(x)
}

/// Translation of `SDL_sinf()`.
pub fn sinf(x: f32) -> f32 {
    sin(x as f64) as f32
}

/// Square root (correctly rounded). Translation of `SDL_sqrt()`.
pub fn sqrt(x: f64) -> f64 {
    misc::sqrt(x)
}

/// Translation of `SDL_sqrtf()`.
pub fn sqrtf(x: f32) -> f32 {
    sqrt(x as f64) as f32
}

/// Tangent of `x` radians. Translation of `SDL_tan()`.
pub fn tan(x: f64) -> f64 {
    trig::tan(x)
}

/// Translation of `SDL_tanf()`.
pub fn tanf(x: f32) -> f32 {
    tan(x as f64) as f32
}

#[cfg(test)]
mod tests;
