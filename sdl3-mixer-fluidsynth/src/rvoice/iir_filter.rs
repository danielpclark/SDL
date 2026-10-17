/* FluidSynth - A Software Synthesizer
 *
 * Copyright (C) 2003  Peter Hanappe and others.
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public License
 * as published by the Free Software Foundation; either version 2.1 of
 * the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful, but
 * WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see
 * <https://www.gnu.org/licenses/>.
 */
// Modified 2026-10-17: translated into Rust from fluid_iir_filter.c,
// fluid_iir_filter_impl.cpp and fluid_iir_filter.h in SDL_mixer 3.3.0's
// FluidSynth 2.5.7 (see NOTICE).

use std::sync::Arc;

use sdl3::stdlib::math::{cosf, pow, sinf, sqrtf};

use crate::fluidsynth_h::{
    FLUID_IIR_DISABLED, FLUID_IIR_HIGHPASS, FLUID_IIR_LOWPASS, FLUID_IIR_NO_GAIN_AMP,
    FLUID_IIR_Q_LINEAR, FLUID_IIR_Q_ZERO_OFF,
};
use crate::fluidsynth_priv::{fluid_clip, RvoiceParam, FLUID_BUFSIZE, MAX_EVENT_PARAMS, M_PI};
use crate::utils::conv::{fluid_ct2hz, fluid_hz2ct};
use crate::FluidReal;

const Q_MIN: FluidReal = 0.001;

// Calculate the filter coefficients with single precision
/// `IIR_COEFF_T`
type IirCoeff = f32;

/// Translation of `fluid_iir_sincos_t`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct FluidIirSincos {
    pub sin: IirCoeff,
    pub cos: IirCoeff,
}

/// Translation of `fluid_iir_filter_t`.
///
/// We can't do information hiding here, as fluid_voice_t includes the struct
/// without a pointer.
#[derive(Clone, Default, Debug)]
pub(crate) struct FluidIirFilter {
    pub type_: i32, /* specifies the type of this filter */
    pub flags: i32, /* additional flags to customize this filter */

    /* filter coefficients */
    /* The coefficients are normalized to a0. */
    /* b0 and b2 are identical => b02 */
    pub b02: IirCoeff, /* b0 / a0 */
    pub b1: IirCoeff,  /* b1 / a0 */
    pub a1: IirCoeff,  /* a0 / a0 */
    pub a2: IirCoeff,  /* a1 / a0 */

    pub hist1: FluidReal,
    pub hist2: FluidReal,    /* Sample history for the IIR filter */
    pub filter_startup: i32, /* Flag: If set, the filter parameters will be set directly. Else it changes smoothly. */

    pub fres: FluidReal, /* The desired resonance frequency, in absolute cents, this filter is currently set to */
    pub last_fres: FluidReal, /* The filter's current (smoothed out) resonance frequency in Hz, which will converge towards its target fres once fres_incr_count has become zero */
    pub fres_incr: FluidReal, /* The linear increment of fres each sample */
    pub fres_incr_count: i32, /* The number of samples left for the smoothed last_fres adjustment to complete */

    pub last_q: FluidReal, /* The filter's current (smoothed) Q-factor (or "bandwidth", or "resonance-friendlyness") on a linear scale. Just like fres, this will converge towards its target Q once q_incr_count has become zero. */
    pub q_incr: FluidReal, /* The linear increment of q each sample */
    pub q_incr_count: i32, /* The number of samples left for the smoothed Q adjustment to complete */

    // the final gain amplifier to be applied by the last filter in the chain, zero for all other filters
    pub amp: FluidReal,      /* current linear amplitude */
    pub amp_incr: FluidReal, /* amplitude increment value for the next FLUID_BUFSIZE samples */

    /// pointer to the precalculated sin and cos values, owned by the synth
    pub sincos_table: Option<Arc<[FluidIirSincos]>>,
}

pub(crate) const CENTS_STEP: i32 = 1; /* cents */
pub(crate) const FRES_MIN: i32 = 1500; /* cents */
pub(crate) const FRES_MAX: i32 = 13500; /* cents */
pub(crate) const SINCOS_TAB_SIZE: usize = ((FRES_MAX /* upper fc in cents */ - FRES_MIN/* lower fc in cents */)
    / CENTS_STEP) as usize
    + 1 /* add one because asking for FRES_MAX cents must yield a valid coefficient */;

/// Translation of `fluid_iir_filter_init()` (an rvoice event function).
pub(crate) fn fluid_iir_filter_init(
    iir_filter: &mut FluidIirFilter,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    let type_ = param[0].i;
    let flags = param[1].i;

    iir_filter.type_ = type_;
    iir_filter.flags = flags;

    if type_ != FLUID_IIR_DISABLED {
        fluid_iir_filter_reset(iir_filter);
    }
}

/// Translation of `fluid_iir_filter_reset()`.
pub(crate) fn fluid_iir_filter_reset(iir_filter: &mut FluidIirFilter) {
    iir_filter.hist1 = 0.0;
    iir_filter.hist2 = 0.0;
    iir_filter.last_fres = -1.0;
    iir_filter.last_q = 0.0;
    iir_filter.filter_startup = 1;
    iir_filter.amp = 0.0;
    iir_filter.amp_incr = 0.0;
}

/// Translation of `fluid_iir_filter_set_fres()` (an rvoice event
/// function).
pub(crate) fn fluid_iir_filter_set_fres(
    iir_filter: &mut FluidIirFilter,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    let fres = param[0].real;

    iir_filter.fres = fres;
}

/// Translation of `fluid_iir_filter_q_from_dB()`.
#[allow(non_snake_case)]
fn fluid_iir_filter_q_from_dB(mut q_dB: FluidReal) -> FluidReal {
    /* The generator contains 'centibels' (1/10 dB) => divide by 10 to
     * obtain dB */
    q_dB /= 10.0;

    /* Range: SF2.01 section 8.1.3 # 8 (convert from cB to dB => /10) */
    q_dB = fluid_clip(q_dB, 0.0, 96.0);

    /* Short version: Modify the Q definition in a way, that a Q of 0
     * dB leads to no resonance hump in the freq. response.
     *
     * Long version: From SF2.01, page 39, item 9 (initialFilterQ):
     * "The gain at the cutoff frequency may be less than zero when
     * zero is specified".  Assume q_dB=0 / q_lin=1: If we would leave
     * q as it is, then this results in a 3 dB hump slightly below
     * fc. At fc, the gain is exactly the DC gain (0 dB).  What is
     * (probably) meant here is that the filter does not show a
     * resonance hump for q_dB=0. In this case, the corresponding
     * q_lin is 1/sqrt(2)=0.707.  The filter should have 3 dB of
     * attenuation at fc now.  In this case Q_dB is the height of the
     * resonance peak not over the DC gain, but over the frequency
     * response of a non-resonant filter.  This idea is implemented as
     * follows: */
    q_dB -= 3.01f32 as FluidReal;

    /* The 'sound font' Q is defined in dB. The filter needs a linear
    q. Convert. */
    pow(10.0, q_dB / 20.0)
}

/// Translation of `fluid_iir_filter_set_q()` (an rvoice event function).
pub(crate) fn fluid_iir_filter_set_q(
    iir_filter: &mut FluidIirFilter,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    let mut q = param[0].real;
    let flags = iir_filter.flags;

    if flags & FLUID_IIR_Q_ZERO_OFF != 0 && q <= 0.0 {
        q = 0.0;
    } else if flags & FLUID_IIR_Q_LINEAR != 0 {
        /* q is linear (only for user-defined filter) and can be manipulated, scaled or transformed via FLUID_MOD_CUSTOM */
    } else {
        q = fluid_iir_filter_q_from_dB(q);
    }

    if iir_filter.filter_startup != 0 {
        iir_filter.last_q = q;
        iir_filter.q_incr_count = 0;
    } else {
        const Q_INCR_COUNT: FluidReal = FLUID_BUFSIZE as FluidReal;
        // Q must be at least Q_MIN, otherwise fluid_iir_filter_apply would never be entered
        if q >= Q_MIN && iir_filter.last_q < Q_MIN {
            iir_filter.last_q = Q_MIN;
        }
        iir_filter.q_incr = (q - iir_filter.last_q) / (Q_INCR_COUNT);
        iir_filter.q_incr_count = Q_INCR_COUNT as i32;
    }
}

// Calculating the sine and cosine coefficients for every possible cutoff frequency is too CPU expensive and can harm realtime playback.
// Therefore, we precalculate the coefficients with a precision of CENTS_STEP and store them in a table.
/// Translation of `fluid_iir_filter_init_table()`.
pub(crate) fn fluid_iir_filter_init_table(
    sincos_table: &mut [FluidIirSincos],
    sample_rate: FluidReal,
) {
    let period = (2.0 * M_PI / sample_rate) as IirCoeff;
    let mut fres_cents = FRES_MIN;
    let mut i = 0;
    while fres_cents <= FRES_MAX {
        let fres = fluid_ct2hz(fres_cents as FluidReal) as IirCoeff;
        let omega = period * fres;
        let sin_coeff = sinf(omega);
        let cos_coeff = cosf(omega);
        // i == (fres_cents - FRES_MIN) / CENTS_STEP;
        sincos_table[i].sin = sin_coeff;
        sincos_table[i].cos = cos_coeff;
        fres_cents += CENTS_STEP;
        i += 1;
    }
}

/// Translation of `fluid_iir_filter_calculate_coefficients<float,
/// GAIN_NORM, TYPE>()`: returns `(a1, a2, b02, b1)`, or `None` for a
/// disabled filter (which leaves the outputs as they were).
#[inline]
fn fluid_iir_filter_calculate_coefficients(
    gain_norm: bool,
    type_: i32,
    fres: IirCoeff,
    q: IirCoeff,
    sincos_table: &[FluidIirSincos],
) -> Option<(IirCoeff, IirCoeff, IirCoeff, IirCoeff)> {
    let mut filter_gain: IirCoeff = 1.0;

    /*
     * Those equations from Robert Bristow-Johnson's `Cookbook
     * formulae for audio EQ biquad filter coefficients', obtained
     * from Harmony-central.com / Computer / Programming. They are
     * the result of the bilinear transform on an analogue filter
     * prototype. To quote, `BLT frequency warping has been taken
     * into account for both significant frequency relocation and for
     * bandwidth readjustment'. */

    // (C's conversion of a float out of int's range is undefined; this
    // saturates.)
    let mut tab_idx = ((fres as i32).wrapping_sub(FRES_MIN)) / CENTS_STEP;
    tab_idx = fluid_clip(tab_idx, 0, SINCOS_TAB_SIZE as i32 - 1);
    let sin_coeff = sincos_table.get(tab_idx as usize).map_or(0.0, |s| s.sin);
    let cos_coeff = sincos_table.get(tab_idx as usize).map_or(0.0, |s| s.cos);
    let alpha_coeff = sin_coeff / (2.0 * q);
    let a0_inv = 1.0 / (1.0 + alpha_coeff);

    /* Calculate the filter coefficients. All coefficients are
     * normalized by a0. Think of `a1' as `a1/a0'.
     *
     * Here a couple of multiplications are saved by reusing common expressions.
     * The original equations should be:
     *  iir_filter->b0=(1.-cos_coeff)*a0_inv*0.5*filter_gain;
     *  iir_filter->b1=(1.-cos_coeff)*a0_inv*filter_gain;
     *  iir_filter->b2=(1.-cos_coeff)*a0_inv*0.5*filter_gain; */

    /* "a" coeffs are same for all 3 available filter types */
    let a1_temp = -2.0 * cos_coeff * a0_inv;
    let a2_temp = (1.0 - alpha_coeff) * a0_inv;
    let b02_temp: IirCoeff;
    let mut b1_temp: IirCoeff;

    if gain_norm {
        /* SF 2.01 page 59:
         *
         *  The SoundFont specs ask for a gain reduction equal to half the
         *  height of the resonance peak (Q).  For example, for a 10 dB
         *  resonance peak, the gain is reduced by 5 dB.  This is done by
         *  multiplying the total gain with sqrt(1/Q).  `Sqrt' divides dB
         *  by 2 (100 lin = 40 dB, 10 lin = 20 dB, 3.16 lin = 10 dB etc)
         *  The gain is later factored into the 'b' coefficients
         *  (numerator of the filter equation).  This gain factor depends
         *  only on Q, so this is the right place to calculate it.
         */
        filter_gain /= sqrtf(q);
    }

    match type_ {
        FLUID_IIR_HIGHPASS => {
            b1_temp = (1.0 + cos_coeff) * a0_inv * filter_gain;

            /* both b0 -and- b2 */
            b02_temp = b1_temp * 0.5;

            b1_temp *= -1.0;
        }

        FLUID_IIR_LOWPASS => {
            b1_temp = (1.0 - cos_coeff) * a0_inv * filter_gain;

            /* both b0 -and- b2 */
            b02_temp = b1_temp * 0.5;
        }

        _ => {
            /* filter disabled, should never get here */
            return None;
        }
    }

    Some((a1_temp, a2_temp, b02_temp, b1_temp))
}

/// Translation of `fluid_iir_filter_apply_local<GAIN_NORM, AMPLIFY,
/// TYPE>()`.
///
/// Applies a low- or high-pass filter with variable cutoff frequency and quality factor
/// for a given biquad transfer function:
///          b0 + b1*z^-1 + b2*z^-2
///  H(z) = ------------------------
///          a0 + a1*z^-1 + a2*z^-2
///
/// Also modifies filter state accordingly.
/*
 * Variable description:
 * - dsp_a1, dsp_a2: Filter coefficients for the the previously filtered output signal
 * - dsp_b0, dsp_b1, dsp_b2: Filter coefficients for input signal
 * - coefficients normalized to a0
 *
 * A couple of variables are used internally, their results are discarded:
 * - dsp_i: Index through the output buffer
 * - dsp_centernode: delay line for the IIR filter
 * - dsp_hist1: same
 * - dsp_hist2: same
 */
fn fluid_iir_filter_apply_local(
    gain_norm: bool,
    amplify: bool,
    type_: i32,
    iir_filter: &mut FluidIirFilter,
    dsp_buf: &mut [FluidReal],
    count: usize,
) {
    // FLUID_IIR_Q_LINEAR may switch the filter off by setting Q==0
    // Due to the linear smoothing, last_q may not exactly become zero.
    if iir_filter.type_ == FLUID_IIR_DISABLED || iir_filter.last_q < Q_MIN {
        return;
    }

    /* IIR filter sample history */
    let mut dsp_hist1 = iir_filter.hist1;
    let mut dsp_hist2 = iir_filter.hist2;

    /* IIR filter coefficients */
    let mut dsp_a1 = iir_filter.a1;
    let mut dsp_a2 = iir_filter.a2;
    let mut dsp_b02 = iir_filter.b02;
    let mut dsp_b1 = iir_filter.b1;

    let mut fres_incr_count = iir_filter.fres_incr_count;
    let mut q_incr_count = iir_filter.q_incr_count;

    let mut dsp_amp = iir_filter.amp;
    let dsp_amp_incr = iir_filter.amp_incr;
    let mut fres = iir_filter.last_fres as IirCoeff;
    let mut q = iir_filter.last_q as IirCoeff;

    let fres_incr = iir_filter.fres_incr as IirCoeff;
    let q_incr = iir_filter.q_incr as IirCoeff;

    let table = iir_filter
        .sincos_table
        .clone()
        .unwrap_or_else(|| Arc::from([]));

    /* filter (implement the voice filter according to SoundFont standard) */

    for dsp_i in 0..count {
        /* The filter is implemented in Direct-II form. */
        let dsp_centernode =
            dsp_buf[dsp_i] - dsp_a1 as FluidReal * dsp_hist1 - dsp_a2 as FluidReal * dsp_hist2;
        let sample =
            dsp_b02 as FluidReal * (dsp_centernode + dsp_hist2) + dsp_b1 as FluidReal * dsp_hist1;
        dsp_hist2 = dsp_hist1;
        dsp_hist1 = dsp_centernode;

        /* Alternatively, it could be implemented in Transposed Direct Form II */
        // fluid_real_t dsp_input = dsp_buf[dsp_i];
        // dsp_buf[dsp_i] = dsp_b02 * dsp_input + dsp_hist1;
        // dsp_hist1 = dsp_b1 * dsp_input - dsp_a1 * dsp_buf[dsp_i] + dsp_hist2;
        // dsp_hist2 = dsp_b02 * dsp_input - dsp_a2 * dsp_buf[dsp_i];

        if amplify {
            dsp_buf[dsp_i] = dsp_amp * sample;
            dsp_amp += dsp_amp_incr;
        } else {
            dsp_buf[dsp_i] = sample;
        }

        if fres_incr_count > 0 || q_incr_count > 0 {
            if fres_incr_count > 0 {
                fres_incr_count -= 1;
                fres += fres_incr;
            }
            if q_incr_count > 0 {
                q_incr_count -= 1;
                q += q_incr;
                if (q as FluidReal) < Q_MIN {
                    q_incr_count = 0;
                    q = Q_MIN as IirCoeff;
                }
            }

            if let Some((a1, a2, b02, b1)) =
                fluid_iir_filter_calculate_coefficients(gain_norm, type_, fres, q, &table)
            {
                dsp_a1 = a1;
                dsp_a2 = a2;
                dsp_b02 = b02;
                dsp_b1 = b1;
            }
        }
    }

    iir_filter.a1 = dsp_a1;
    iir_filter.a2 = dsp_a2;
    iir_filter.b02 = dsp_b02;
    iir_filter.b1 = dsp_b1;

    /* Check for denormal number (too close to zero). */
    if dsp_hist1.abs() < 1e-20f32 as FluidReal {
        dsp_hist1 = 0.0;
    }
    if dsp_hist2.abs() < 1e-20f32 as FluidReal {
        dsp_hist2 = 0.0;
    }
    iir_filter.hist1 = dsp_hist1;
    iir_filter.hist2 = dsp_hist2;

    iir_filter.last_fres = fres as FluidReal;
    iir_filter.fres_incr_count = fres_incr_count;
    iir_filter.last_q = q as FluidReal;
    iir_filter.q_incr_count = q_incr_count;
    iir_filter.amp = dsp_amp;
}

/// Translation of `fluid_iir_filter_apply()`.
pub(crate) fn fluid_iir_filter_apply(
    resonant_filter: &mut FluidIirFilter,
    resonant_custom_filter: &mut FluidIirFilter,
    dsp_buf: &mut [FluidReal],
    count: usize,
) {
    let custom_type = if resonant_custom_filter.type_ == FLUID_IIR_HIGHPASS {
        FLUID_IIR_HIGHPASS
    } else {
        FLUID_IIR_LOWPASS
    };
    if resonant_custom_filter.flags & FLUID_IIR_NO_GAIN_AMP != 0 {
        fluid_iir_filter_apply_local(
            false,
            false,
            custom_type,
            resonant_custom_filter,
            dsp_buf,
            count,
        );
    } else {
        fluid_iir_filter_apply_local(
            true,
            false,
            custom_type,
            resonant_custom_filter,
            dsp_buf,
            count,
        );
    }

    // This is the last filter in the chain - the default SF2 filter that always runs. This one must apply the final envelope gain.
    fluid_iir_filter_apply_local(
        true,
        true,
        FLUID_IIR_LOWPASS,
        resonant_filter,
        dsp_buf,
        count,
    );
}

/// Translation of `fluid_iir_filter_calc()`.
pub(crate) fn fluid_iir_filter_calc(
    iir_filter: &mut FluidIirFilter,
    output_rate: FluidReal,
    fres_mod: FluidReal,
) {
    let calc_coeff_flag: bool;
    let mut fres: FluidReal;

    if iir_filter.type_ == FLUID_IIR_DISABLED {
        return;
    }

    /* calculate the frequency of the resonant filter in Hz */
    fres = fluid_ct2hz(iir_filter.fres + fres_mod);

    /* I removed the optimization of turning the filter off when the
     * resonance frequency is above the maximum frequency. Instead, the
     * filter frequency is set to a maximum of 0.45 times the sampling
     * rate. For a 44100 kHz sampling rate, this amounts to 19845
     * Hz. The reason is that there were problems with anti-aliasing when the
     * synthesizer was run at lower sampling rates. Thanks to Stephan
     * Tassart for pointing me to this bug. By turning the filter on and
     * clipping the maximum filter frequency at 0.45*srate, the filter
     * is used as an anti-aliasing filter. */

    if fres > 0.45f32 as FluidReal * output_rate {
        fres = 0.45f32 as FluidReal * output_rate;
    } else if fres < 5.0 {
        fres = 5.0;
    }

    fres = fluid_hz2ct(fres);

    /* if filter enabled and there is a significant frequency change.. */
    let fres_diff = fres - iir_filter.last_fres;
    if iir_filter.filter_startup != 0 {
        // The filter was just starting up, make sure to calculate initial coefficients for the initial Q value, even though the fres may not have changed
        calc_coeff_flag = true;

        iir_filter.fres_incr_count = 0;
        iir_filter.last_fres = fres;
        iir_filter.filter_startup = (iir_filter.last_q < Q_MIN) as i32; // filter coefficients will not be initialized when Q is small
    } else if fres_diff.abs() > CENTS_STEP as FluidReal {
        // only smooth out fres when difference is "significant"
        let mut fres_incr_count = FLUID_BUFSIZE as FluidReal;
        let mut num_buffers = iir_filter.last_q;
        num_buffers = fluid_clip(num_buffers, 1.0, 5.0);
        // For high values of Q, the phase gets really steep. To prevent clicks when quickly modulating fres in this case, we need to smooth out "slower".
        // This is done by simply using Q times FLUID_BUFSIZE samples for the interpolation to complete, capped at 5.
        // 5 was chosen because the phase doesn't really get any steeper when continuing to increase Q.
        fres_incr_count *= num_buffers;
        iir_filter.fres_incr = fres_diff / (fres_incr_count);
        iir_filter.fres_incr_count = (fres_incr_count + 0.5) as i32;

        // The filter coefficients have to be recalculated (filter cutoff has changed).
        calc_coeff_flag = true;
    } else {
        let last_fres = iir_filter.last_fres;
        // difference in fres is small, so set it directly
        iir_filter.last_fres = fres;
        // disable fres interpolation
        iir_filter.fres_incr_count = 0;
        // Filter cutoff has changed, so don't forget to recalculate the coefficients! #1679
        calc_coeff_flag = last_fres != fres;
        // We do not account for any change of Q here - if it was changed q_incro_count will be non-zero and recalculating the coeffs
        // will be taken care of in fluid_iir_filter_apply().
    }

    let last_fres_f = iir_filter.last_fres as IirCoeff;
    let last_q_f = iir_filter.last_q as IirCoeff;
    if calc_coeff_flag && iir_filter.filter_startup == 0 {
        let gain_norm = iir_filter.flags & FLUID_IIR_NO_GAIN_AMP == 0;
        let type_ = if iir_filter.type_ == FLUID_IIR_HIGHPASS {
            FLUID_IIR_HIGHPASS
        } else {
            FLUID_IIR_LOWPASS
        };
        let table = iir_filter
            .sincos_table
            .clone()
            .unwrap_or_else(|| Arc::from([]));
        if let Some((a1, a2, b02, b1)) =
            fluid_iir_filter_calculate_coefficients(gain_norm, type_, last_fres_f, last_q_f, &table)
        {
            iir_filter.a1 = a1;
            iir_filter.a2 = a2;
            iir_filter.b02 = b02;
            iir_filter.b1 = b1;
        }
    }
}
