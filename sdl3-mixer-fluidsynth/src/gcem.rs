/*################################################################################
##
##   Copyright (C) 2016-2024 Keith O'Hara
##
##   This file is part of the GCE-Math C++ library.
##
##   Licensed under the Apache License, Version 2.0 (the "License");
##   you may not use this file except in compliance with the License.
##   You may obtain a copy of the License at
##
##       http://www.apache.org/licenses/LICENSE-2.0
##
##   Unless required by applicable law or agreed to in writing, software
##   distributed under the License is distributed on an "AS IS" BASIS,
##   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
##   See the License for the specific language governing permissions and
##   limitations under the License.
##
################################################################################*/
// Modified 2026-10-17: translated into Rust from gcem 1.18.0's abs, floor,
// sgn, is_odd, find_exponent, find_fraction, find_whole, mantissa,
// pow_integral, exp, log, pow, tan, cos and sin headers (gcem_incl/*.hpp, as
// SDL_mixer 3.3.0's FluidSynth vendors them in sdlorg_fork/gcem), for
// doubles only (see NOTICE and LICENSE.gcem).

//! The compile-time math functions FluidSynth's lookup tables are computed
//! with (`gcem::pow`, `log`, `sin`, `cos`, `fabs`), for `double`. C++
//! evaluates them at compile time in IEEE double arithmetic; the same
//! operations at run time give the same bits.

/// `GCEM_LOG_10`, as a double.
pub(crate) const GCEM_LOG_10: f64 = 2.302_585_092_994_045_684_017_991_454_684_364_207_601_1;
/// `GCEM_PI`, as a double.
pub(crate) const GCEM_PI: f64 = 3.141_592_653_589_793_238_462_643_383_279_502_884_197_2;
/// `GCEM_HALF_PI`, as a double.
pub(crate) const GCEM_HALF_PI: f64 = 1.570_796_326_794_896_619_231_321_691_639_751_442_098_6;
/// `GCEM_E`, as a double (`fluid_cb2amp.cpp` redefines it as one).
pub(crate) const GCEM_E: f64 = 2.718_281_828_459_045_235_360_287_471_352_662_497_757_2;

/// `GCEM_EXP_MAX_ITER_SMALL`
const GCEM_EXP_MAX_ITER_SMALL: i32 = 25;
/// `GCEM_LOG_MAX_ITER_SMALL`
const GCEM_LOG_MAX_ITER_SMALL: i32 = 25;

/// `GCLIM<double>::min()`
const DBL_MIN: f64 = f64::MIN_POSITIVE;

/// `gcem::abs()`
pub(crate) fn abs(x: f64) -> f64 {
    // deal with signed-zeros
    if x == 0.0 {
        0.0
    } else if x < 0.0 {
        -x
    } else {
        x
    }
}

/// `gcem::fabs()`
pub(crate) fn fabs(x: f64) -> f64 {
    abs(x)
}

/// `gcem::sgn()`
fn sgn(x: f64) -> i32 {
    if x > 0.0 {
        1
    } else if x < 0.0 {
        -1
    } else {
        0
    }
}

/// `internal::floor_resid()`
fn floor_resid(x: f64, x_whole: f64) -> i32 {
    ((x < 0.0) && (x < x_whole)) as i32
}

/// `internal::floor_int()`
fn floor_int(x: f64, x_whole: f64) -> f64 {
    x_whole - floor_resid(x, x_whole) as f64
}

/// `internal::floor_check_internal<double>()`
fn floor_check_internal(x: f64) -> f64 {
    if abs(x) >= 4503599627370496. {
        x
    } else {
        floor_int(x, (x as i64) as f64)
    }
}

/// `internal::floor_check()`
fn floor_check(x: f64) -> f64 {
    if x.is_nan() {
        f64::NAN
    } else if !x.is_finite() {
        // +/- infinite
        x
    } else if DBL_MIN > abs(x) {
        // signed-zero cases
        x
    } else {
        floor_check_internal(x)
    }
}

/// `internal::is_odd()`
fn is_odd(x: i64) -> bool {
    (x & 1) != 0
}

/// `internal::find_exponent()`
fn find_exponent(x: f64, exponent: i64) -> i64 {
    if x < 1e-03 {
        find_exponent(x * 1e+04, exponent - 4)
    } else if x < 1e-01 {
        find_exponent(x * 1e+02, exponent - 2)
    } else if x < 1.0 {
        find_exponent(x * 10.0, exponent - 1)
    } else if x > 10.0 {
        // > 10
        find_exponent(x / 10.0, exponent + 1)
    } else if x > 1e+02 {
        find_exponent(x / 1e+02, exponent + 2)
    } else if x > 1e+04 {
        find_exponent(x / 1e+04, exponent + 4)
    } else {
        exponent
    }
}

/// `internal::find_fraction()`
fn find_fraction(x: f64) -> f64 {
    if abs(x - floor_check(x)) >= 0.5 {
        x - floor_check(x) - sgn(x) as f64
    } else {
        x - floor_check(x)
    }
}

/// `internal::find_whole()`
fn find_whole(x: f64) -> i64 {
    if abs(x - floor_check(x)) >= 0.5 {
        (floor_check(x) + sgn(x) as f64) as i64
    } else {
        floor_check(x) as i64
    }
}

/// `internal::mantissa()`
fn mantissa(x: f64) -> f64 {
    if x < 1.0 {
        mantissa(x * 10.0)
    } else if x > 10.0 {
        mantissa(x / 10.0)
    } else {
        x
    }
}

/// `internal::pow_integral_compute_recur()`
fn pow_integral_compute_recur(base: f64, val: f64, exp_term: i64) -> f64 {
    if exp_term > 1 {
        if is_odd(exp_term) {
            pow_integral_compute_recur(base * base, val * base, exp_term / 2)
        } else {
            pow_integral_compute_recur(base * base, val, exp_term / 2)
        }
    } else if exp_term == 1 {
        val * base
    } else {
        val
    }
}

/// `internal::pow_integral_sgn_check()` (signed exponents)
fn pow_integral_sgn_check(base: f64, exp_term: i64) -> f64 {
    if exp_term < 0 {
        1.0 / pow_integral_compute(base, -exp_term)
    } else {
        pow_integral_compute_recur(base, 1.0, exp_term)
    }
}

/// `internal::pow_integral_compute()`
fn pow_integral_compute(base: f64, exp_term: i64) -> f64 {
    if exp_term == 3 {
        base * base * base
    } else if exp_term == 2 {
        base * base
    } else if exp_term == 1 {
        base
    } else if exp_term == 0 {
        1.0
    } else if exp_term == i64::MIN {
        // check for overflow
        0.0
    } else if exp_term == i64::MAX {
        f64::INFINITY
    } else {
        pow_integral_sgn_check(base, exp_term)
    }
}

/// `internal::pow_integral()`
fn pow_integral(base: f64, exp_term: i64) -> f64 {
    pow_integral_compute(base, exp_term)
}

// see https://en.wikipedia.org/wiki/Euler%27s_continued_fraction_formula
/// `internal::exp_cf_recur()` (the C++14 version)
fn exp_cf_recur(x: f64, depth_end: i32) -> f64 {
    let mut depth = GCEM_EXP_MAX_ITER_SMALL - 1;
    let mut res = 1.0;
    while depth > depth_end - 1 {
        res = 1.0 + x / (depth - 1) as f64 - x / depth as f64 / res;
        depth -= 1;
    }
    res
}

/// `internal::exp_cf()`
fn exp_cf(x: f64) -> f64 {
    1.0 / (1.0 - x / exp_cf_recur(x, 2))
}

/// `internal::exp_split()`
fn exp_split(x: f64) -> f64 {
    pow_integral(GCEM_E, find_whole(x)) * exp_cf(find_fraction(x))
}

/// `gcem::exp()` (`internal::exp_check()`)
pub(crate) fn exp(x: f64) -> f64 {
    if x.is_nan() {
        f64::NAN
    } else if x == f64::NEG_INFINITY {
        0.0
    } else if DBL_MIN > abs(x) {
        // indistinguishable from zero
        1.0
    } else if x == f64::INFINITY {
        f64::INFINITY
    } else if abs(x) < 2.0 {
        exp_cf(x)
    } else {
        exp_split(x)
    }
}

// continued fraction seems to be a better approximation for small x
// see http://functions.wolfram.com/ElementaryFunctions/Log/10/0005/
/// `internal::log_cf_main()` (the C++14 version)
fn log_cf_main(xx: f64, depth_end: i32) -> f64 {
    let mut depth = GCEM_LOG_MAX_ITER_SMALL - 1;
    let mut res = (2 * (depth + 1) - 1) as f64;
    while depth > depth_end - 1 {
        res = (2 * depth - 1) as f64 - (depth * depth) as f64 * xx / res;
        depth -= 1;
    }
    res
}

/// `internal::log_cf_begin()`
fn log_cf_begin(x: f64) -> f64 {
    2.0 * x / log_cf_main(x * x, 1)
}

/// `internal::log_main()`
fn log_main(x: f64) -> f64 {
    log_cf_begin((x - 1.0) / (x + 1.0))
}

/// `internal::log_mantissa_integer()` (its long doubles, as doubles)
fn log_mantissa_integer(x: i32) -> f64 {
    match x {
        2 => 0.693_147_180_559_945_309_417_232_121_458_176_568_075_5,
        3 => 1.098_612_288_668_109_691_395_245_236_922_525_704_647_5,
        4 => 1.386_294_361_119_890_618_834_464_242_916_353_136_151_0,
        5 => 1.609_437_912_434_100_374_600_759_333_226_187_639_525_6,
        6 => 1.791_759_469_228_055_000_812_477_358_380_702_272_723_0,
        7 => 1.945_910_149_055_313_305_105_352_743_443_179_729_637_1,
        8 => 2.079_441_541_679_835_928_251_696_364_374_529_704_226_5,
        9 => 2.197_224_577_336_219_382_790_490_473_845_051_409_295_0,
        10 => 2.302_585_092_994_045_684_017_991_454_684_364_207_601_1,
        _ => 0.0,
    }
}

/// `internal::log_mantissa()`
fn log_mantissa(x: f64) -> f64 {
    // divide by the integer part of x, which will be in [1,10], then adjust using tables
    log_main(x / ((x as i32) as f64)) + log_mantissa_integer(x as i32)
}

/// `internal::log_breakup()`
fn log_breakup(x: f64) -> f64 {
    // x = a*b, where b = 10^c
    log_mantissa(mantissa(x)) + GCEM_LOG_10 * find_exponent(x, 0) as f64
}

/// `gcem::log()` (`internal::log_check()`)
pub(crate) fn log(x: f64) -> f64 {
    if x.is_nan() {
        f64::NAN
    } else if x < 0.0 {
        // x < 0
        f64::NAN
    } else if DBL_MIN > x {
        // x ~= 0
        f64::NEG_INFINITY
    } else if DBL_MIN > abs(x - 1.0) {
        // indistinguishable from 1
        0.0
    } else if x == f64::INFINITY {
        f64::INFINITY
    } else if x < 0.5 || x > 1.5 {
        log_breakup(x)
    } else {
        log_main(x)
    }
}

/// `gcem::pow()` for a non-integral exponent (`internal::pow_check()`,
/// `internal::pow_dbl()`)
pub(crate) fn pow(base: f64, exp_term: f64) -> f64 {
    if base < 0.0 {
        f64::NAN
    } else {
        exp(exp_term * log(base))
    }
}

/// `internal::tan_series_exp_long()`
fn tan_series_exp_long(z: f64) -> f64 {
    // this is based on a fourth-order expansion of tan(z) using Bernoulli numbers
    -1.0 / z
        + (z / 3.0
            + (pow_integral(z, 3) / 45.0
                + (2.0 * pow_integral(z, 5) / 945.0 + pow_integral(z, 7) / 4725.0)))
}

/// `internal::tan_series_exp()`
fn tan_series_exp(x: f64) -> f64 {
    if DBL_MIN > abs(x - GCEM_HALF_PI) {
        // the value tan(pi/2) is somewhat of a convention;
        // technically the function is not defined at EXACTLY pi/2,
        // but this is floating point pi/2
        1.633124e+16
    } else {
        // otherwise we use an expansion around pi/2
        tan_series_exp_long(x - GCEM_HALF_PI)
    }
}

/// `internal::tan_cf_recur()` (its recursion unrolled from the deepest
/// level up)
fn tan_cf_recur(xx: f64, depth: i32, max_depth: i32) -> f64 {
    let mut res = (2 * max_depth - 1) as f64;
    let mut d = max_depth - 1;
    while d >= depth {
        res = (2 * d - 1) as f64 - xx / res;
        d -= 1;
    }
    res
}

/// `internal::tan_cf_main()`
fn tan_cf_main(x: f64) -> f64 {
    if x > 1.55 && x < 1.60 {
        // deals with a singularity at tan(pi/2)
        tan_series_exp(x)
    } else if x > 1.4 {
        x / tan_cf_recur(x * x, 1, 45)
    } else if x > 1.0 {
        x / tan_cf_recur(x * x, 1, 35)
    } else {
        x / tan_cf_recur(x * x, 1, 25)
    }
}

/// `internal::tan_begin()`
fn tan_begin(x: f64, count: i32) -> f64 {
    // tan(x) = tan(x + pi)
    if x > GCEM_PI {
        if count > 1 {
            // protect against undefined behavior
            f64::NAN
        } else {
            tan_begin(x - GCEM_PI * floor_check(x / GCEM_PI), count + 1)
        }
    } else {
        tan_cf_main(x)
    }
}

/// `gcem::tan()` (`internal::tan_check()`)
fn tan(x: f64) -> f64 {
    if x.is_nan() {
        f64::NAN
    } else if DBL_MIN > abs(x) {
        // indistinguishable from zero
        0.0
    } else if x < 0.0 {
        -tan_begin(-x, 0)
    } else {
        tan_begin(x, 0)
    }
}

/// `gcem::cos()` (`internal::cos_check()`, `internal::cos_compute()`)
pub(crate) fn cos(x: f64) -> f64 {
    if x.is_nan() {
        f64::NAN
    } else if DBL_MIN > abs(x) {
        // indistinguishable from 0
        1.0
    } else if DBL_MIN > abs(x - GCEM_HALF_PI) {
        // special cases: pi/2 and pi
        0.0
    } else if DBL_MIN > abs(x + GCEM_HALF_PI) {
        0.0
    } else if DBL_MIN > abs(x - GCEM_PI) {
        -1.0
    } else if DBL_MIN > abs(x + GCEM_PI) {
        -1.0
    } else {
        let x = tan(x / 2.0);
        (1.0 - x * x) / (1.0 + x * x)
    }
}

/// `gcem::sin()` (`internal::sin_check()`, `internal::sin_compute()`)
pub(crate) fn sin(x: f64) -> f64 {
    if x.is_nan() {
        f64::NAN
    } else if DBL_MIN > abs(x) {
        // indistinguishable from zero
        0.0
    } else if DBL_MIN > abs(x - GCEM_HALF_PI) {
        // special cases: pi/2 and pi
        1.0
    } else if DBL_MIN > abs(x + GCEM_HALF_PI) {
        -1.0
    } else if DBL_MIN > abs(x - GCEM_PI) {
        0.0
    } else if DBL_MIN > abs(x + GCEM_PI) {
        -0.0
    } else {
        let x = tan(x / 2.0);
        2.0 * x / (1.0 + x * x)
    }
}
