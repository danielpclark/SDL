// FluidSynth - A Software Synthesizer: src/gentables/ (which carries no
// notice of its own; it is under FluidSynth's license, the GNU Lesser
// General Public License version 2.1 or later, see COPYING).
// Modified 2026-10-17: translated into Rust from ConstExprArr.hpp,
// fluid_ct2hz.cpp, fluid_cb2amp.cpp, fluid_concave.cpp, fluid_convex.cpp,
// fluid_pan.cpp, fluid_interp_coeff.cpp, fluid_interp_coeff_linear.cpp and
// fluid_interp_coeff_sinc7.cpp in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see
// NOTICE).

//! The lookup tables C++ computes at compile time (`ConstExprArr<F, N>`:
//! `F::calc(i)` for `i` in `0..N`) with the gcem math functions, computed
//! here once, on first use, with the translation of those.

use std::sync::LazyLock;

use crate::gcem;
use crate::rvoice::dsp_tables::{
    CUBIC_INTERP_ORDER, FLUID_INTERP_MAX, LINEAR_INTERP_ORDER, SINC_INTERP_ORDER,
};
use crate::utils::conv_tables::{
    FLUID_CB_AMP_SIZE, FLUID_CENTS_HZ_SIZE, FLUID_PAN_SIZE, FLUID_PEAK_ATTENUATION,
    FLUID_VEL_CB_SIZE,
};
use crate::FluidReal;

/// Translation of `ConstExprArr<F, N>::value`.
fn const_expr_arr(n: usize, calc: impl Fn(i32) -> FluidReal) -> Box<[FluidReal]> {
    (0..n as i32).map(calc).collect()
}

/// `Ct2HzFunctor::calc()`
fn ct2hz_calc(i: i32) -> FluidReal {
    // 6,875 is just a factor that we already multiply into the lookup table to save
    // that multiplication in fluid_ct2hz_real()
    // 6.875 Hz because 440Hz / 2^6
    6.875 * gcem::pow(2.0, i as FluidReal / 1200.0)
}

/// `fluid_ct2hz_tab`
pub(crate) static FLUID_CT2HZ_TAB: LazyLock<Box<[FluidReal]>> =
    LazyLock::new(|| const_expr_arr(FLUID_CENTS_HZ_SIZE, ct2hz_calc));

/// `Cb2AmpFunctor::calc()`
fn cb2amp_calc(i: i32) -> FluidReal {
    /* centibels to amplitude conversion
     * Note: SF2.01 section 8.1.3: Initial attenuation range is
     * between 0 and 144 dB. Therefore a negative attenuation is
     * not allowed.
     */
    gcem::pow(10.0, i as FluidReal / -200.0)
}

/// `fluid_cb2amp_tab`
pub(crate) static FLUID_CB2AMP_TAB: LazyLock<Box<[FluidReal]>> =
    LazyLock::new(|| const_expr_arr(FLUID_CB_AMP_SIZE, cb2amp_calc));

/// `ConcaveFunctor::calc()`
fn concave_calc(i: i32) -> FluidReal {
    /* There seems to be an error in the specs. The equations are
    implemented according to the pictures on SF2.01 page 73. */
    if i == 0 {
        0.0
    } else if i == FLUID_VEL_CB_SIZE as i32 - 1 {
        1.0
    } else {
        (-200.0 * 2.0 / FLUID_PEAK_ATTENUATION as f64)
            * gcem::log(
                ((FLUID_VEL_CB_SIZE as i32 - 1) - i) as f64 / (FLUID_VEL_CB_SIZE as f64 - 1.0),
            )
            / gcem::GCEM_LOG_10
    }
}

/// `fluid_concave_tab`
pub(crate) static FLUID_CONCAVE_TAB: LazyLock<Box<[FluidReal]>> =
    LazyLock::new(|| const_expr_arr(FLUID_VEL_CB_SIZE, concave_calc));

/// `ConvexFunctor::calc()`
fn convex_calc(i: i32) -> FluidReal {
    /* There seems to be an error in the specs. The equations are
    implemented according to the pictures on SF2.01 page 73. */
    if i == 0 {
        0.0
    } else if i == FLUID_VEL_CB_SIZE as i32 - 1 {
        1.0
    } else {
        1.0 - ((-200.0 * 2.0 / FLUID_PEAK_ATTENUATION as f64)
            * gcem::log(i as f64 / (FLUID_VEL_CB_SIZE as f64 - 1.0))
            / gcem::GCEM_LOG_10)
    }
}

/// `fluid_convex_tab`
pub(crate) static FLUID_CONVEX_TAB: LazyLock<Box<[FluidReal]>> =
    LazyLock::new(|| const_expr_arr(FLUID_VEL_CB_SIZE, convex_calc));

/// `PanFunctor::calc()`
fn pan_calc(i: i32) -> FluidReal {
    /* initialize the pan conversion table */
    gcem::sin(i as f64 * (gcem::GCEM_HALF_PI / (FLUID_PAN_SIZE as f64 - 1.0)))
}

/// `fluid_pan_tab`
pub(crate) static FLUID_PAN_TAB: LazyLock<Box<[FluidReal]>> =
    LazyLock::new(|| const_expr_arr(FLUID_PAN_SIZE, pan_calc));

/// `InterpCubicFunctor::calc()`
fn interp_cubic_calc(i: i32) -> FluidReal {
    //for (i = 0; i < FLUID_INTERP_MAX; i++)
    //{
    //    x = (double)i / (double)FLUID_INTERP_MAX;
    //    interp_coeff[i][0] = (x * (-0.5 + x * (1 - 0.5 * x)));
    //    interp_coeff[i][1] = (1.0 + x * x * (1.5 * x - 2.5));
    //    interp_coeff[i][2] = (x * (0.5 + x * (2.0 - 1.5 * x)));
    //    interp_coeff[i][3] = (0.5 * x * x * (x - 1.0));
    //}
    let i_num = i / CUBIC_INTERP_ORDER as i32;
    let i_rem = i % CUBIC_INTERP_ORDER as i32;
    let x = i_num as FluidReal / FLUID_INTERP_MAX as FluidReal;
    match i_rem {
        0 => x * (-0.5 + x * (1.0 - 0.5 * x)),
        1 => 1.0 + x * x * (1.5 * x - 2.5),
        2 => x * (0.5 + x * (2.0 - 1.5 * x)),
        /* I_REM == 3 */ _ => 0.5 * x * x * (x - 1.0),
    }
}

/// `interp_coeff` (`[FLUID_INTERP_MAX][CUBIC_INTERP_ORDER]`, flattened)
pub(crate) static INTERP_COEFF: LazyLock<Box<[FluidReal]>> =
    LazyLock::new(|| const_expr_arr(FLUID_INTERP_MAX * CUBIC_INTERP_ORDER, interp_cubic_calc));

/// `InterpLinearFunctor::calc()`
fn interp_linear_calc(i: i32) -> FluidReal {
    //for (i = 0; i < FLUID_INTERP_MAX; i++)
    //{
    //    x = (double)i / (double)FLUID_INTERP_MAX;
    //    interp_coeff_linear[i][0] = (1.0 - x);
    //    interp_coeff_linear[i][1] = x;
    //}
    let i_num = i / LINEAR_INTERP_ORDER as i32;
    let i_rem = i % LINEAR_INTERP_ORDER as i32;
    let x = i_num as FluidReal / FLUID_INTERP_MAX as FluidReal;
    if i_rem == 0 {
        1.0 - x
    } else {
        x
    }
}

/// `interp_coeff_linear` (`[FLUID_INTERP_MAX][LINEAR_INTERP_ORDER]`,
/// flattened)
pub(crate) static INTERP_COEFF_LINEAR: LazyLock<Box<[FluidReal]>> =
    LazyLock::new(|| const_expr_arr(FLUID_INTERP_MAX * LINEAR_INTERP_ORDER, interp_linear_calc));

/// `InterpSincFunctor::calc()`
fn interp_sinc_calc(i: i32) -> FluidReal {
    // Below is the original code. Note how indexing of sinc_table7 does not happen sequentially in this version:
    //
    // /* i: Offset in terms of whole samples */
    //for (i = 0; i < SINC_INTERP_ORDER; i++)
    //{
    //    /* i2: Offset in terms of fractional samples ('subsamples') */
    //    for (i2 = 0; i2 < FLUID_INTERP_MAX; i2++)
    //    {
    //        /* center on middle of table */
    //        i_shifted = (double)i - ((double)SINC_INTERP_ORDER / 2.0) +
    //                    (double)i2 / (double)FLUID_INTERP_MAX;
    //        /* sinc(0) cannot be calculated straightforward (limit needed for 0/0) */
    //        if (fabs(i_shifted) > 0.000001)
    //        {
    //            double arg = M_PI * i_shifted;
    //            v = sin(arg) / (arg);
    //            /* Hanning window */
    //            v *= 0.5 * (1.0 + cos(2.0 * arg / (double)SINC_INTERP_ORDER));
    //        }
    //        else
    //        {
    //            v = 1.0;
    //        }
    //        sinc_table7[FLUID_INTERP_MAX - i2 - 1][i] = v;
    //    }
    //}
    //
    // To make the indexing happen sequentially, we transform it into this:
    //
    //for (i2 = 0; i2 < FLUID_INTERP_MAX; i2++)
    //{
    //    for (i = 0; i < SINC_INTERP_ORDER; i++)
    //    {
    //        /* Compute the shifted index as in the original code */
    //        double i_shifted = (double)i - ((double)SINC_INTERP_ORDER / 2.0) +
    //                           (double)(FLUID_INTERP_MAX - i2 - 1) / (double)FLUID_INTERP_MAX;
    //        if (fabs(i_shifted) > 0.000001)
    //        {
    //            double arg = M_PI * i_shifted;
    //            v = sin(arg) / arg;
    //            /* Hanning window */
    //            v *= 0.5 * (1.0 + cos(2.0 * arg / (double)SINC_INTERP_ORDER));
    //        }
    //        else
    //        {
    //            v = 1.0;
    //        }
    //        sinc_table7[i2][i] = v;
    //    }
    //}
    let i2 = i / SINC_INTERP_ORDER as i32;
    let ii = i % SINC_INTERP_ORDER as i32;
    let i_shifted = ii as FluidReal - (SINC_INTERP_ORDER as FluidReal / 2.0)
        + (FLUID_INTERP_MAX as i32 - i2 - 1) as FluidReal / FLUID_INTERP_MAX as FluidReal;
    let arg = gcem::GCEM_PI * i_shifted;
    if gcem::fabs(i_shifted) > 0.000001 {
        (gcem::sin(arg) / arg)
            * (0.5 * (1.0 + gcem::cos(2.0 * arg / SINC_INTERP_ORDER as FluidReal)))
    } else {
        1.0
    }
}

/// `sinc_table7` (`[FLUID_INTERP_MAX][SINC_INTERP_ORDER]`, flattened)
pub(crate) static SINC_TABLE7: LazyLock<Box<[FluidReal]>> =
    LazyLock::new(|| const_expr_arr(FLUID_INTERP_MAX * SINC_INTERP_ORDER, interp_sinc_calc));
