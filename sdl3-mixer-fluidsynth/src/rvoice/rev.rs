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
// Modified 2026-10-17: translated into Rust from fluid_rev.c and fluid_rev.h
// in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE).

/******************************************************************************
 *
 *
 *                           FDN REVERB
 *
 * Freeverb used by fluidsynth (v.1.1.10 and previous) is based on
 * Schroeder-Moorer reverberator:
 * https://ccrma.stanford.edu/~jos/pasp/Freeverb.html
 *
 * This FDN reverberation is based on jot FDN reverberator.
 * https://ccrma.stanford.edu/~jos/Reverb/FDN_Late_Reverberation.html
 * Like Freeverb it is a late reverb which is convenient for Fluidsynth.
 *
 * It takes a monophonic input and produces a stereo output.
 *
 * The parameters are the same than for Freeverb.
 * Also the default response of these parameters are the same than for Freeverb:
 *  - roomsize (0 to 1): control the reverb time from 0.7 to 12.5 s.
 *    This reverberation time is ofen called T60DC.
 *
 *  - damp (0 to 1): controls the reverb time frequency dependency.
 *    This controls the reverb time for the frequency sample rate/2
 *
 *    When 0, the reverb time for high frequencies is the same as
 *    for DC frequency.
 *    When > 0, high frequencies have less reverb time than lower frequencies.
 *
 *  - width (0 to 100): controls the left/right output separation.
 *    When 0, there are no separation and the signal on left and right.
 *    output is the same. This sounds like a monophonic signal.
 *    When 1, the separation between left and right is maximum.
 *    When 100 the perception of this separation is further "exaggerated".
 *
 *  - level (0 to 1), controls the output level reverberation.
 *
 * (Upstream's comment goes on about the reverb's quality, CPU load and
 * memory use with 8 and 12 delay lines; this build uses 8, the default,
 * the Freeverb-like roomsize response and DENORMALISING, as upstream's
 * defaults.)
 */

use sdl3::stdlib::math::{cos, log, pow, sin, sqrt};

use crate::fluidsynth_priv::{fluid_clip, FLUID_BUFSIZE, FLUID_M_LN10, FLUID_M_PI};
use crate::utils::sys::{fluid_log, FLUID_INFO, FLUID_WARN};
use crate::FluidReal;

/* enum describing each reverb parameter */
pub(crate) const FLUID_REVERB_ROOMSIZE: usize = 0; /*< reverb time */
pub(crate) const FLUID_REVERB_DAMP: usize = 1; /*< high frequency damping */
pub(crate) const FLUID_REVERB_WIDTH: usize = 2; /*< stereo width */
pub(crate) const FLUID_REVERB_LEVEL: usize = 3; /*< output level */
pub(crate) const FLUID_REVERB_PARAM_LAST: usize = 4; /* number of enum fluid_reverb_param */

/* return a bit flag from param: 2^param */
pub(crate) const fn fluid_revparam_to_setflag(param: usize) -> i32 {
    1 << param
}

/* Flags for fluid_revmodel_set() */
pub(crate) const FLUID_REVMODEL_SET_ROOMSIZE: i32 =
    fluid_revparam_to_setflag(FLUID_REVERB_ROOMSIZE);
pub(crate) const FLUID_REVMODEL_SET_DAMPING: i32 = fluid_revparam_to_setflag(FLUID_REVERB_DAMP);
pub(crate) const FLUID_REVMODEL_SET_WIDTH: i32 = fluid_revparam_to_setflag(FLUID_REVERB_WIDTH);
pub(crate) const FLUID_REVMODEL_SET_LEVEL: i32 = fluid_revparam_to_setflag(FLUID_REVERB_LEVEL);
/** Value for fluid_revmodel_set() which sets all reverb parameters. */
pub(crate) const FLUID_REVMODEL_SET_ALL: i32 = FLUID_REVMODEL_SET_LEVEL
    | FLUID_REVMODEL_SET_WIDTH
    | FLUID_REVMODEL_SET_DAMPING
    | FLUID_REVMODEL_SET_ROOMSIZE;

/*----------------------------------------------------------------------------
                        Configuration macros at compiler time.
-----------------------------------------------------------------------------*/

/* Number of delay lines (must be only 8 or 12)
  8 is the default.
 12 produces a better quality but is +50% cpu expensive.
*/
const NBR_DELAYS: usize = 8; /* default*/

/* DENORMALISING enable denormalising handling */
const DC_OFFSET: f32 = 1e-8;

/*----------------------------------------------------------------------------
 Initial internal reverb settings (at reverb creation time)
-----------------------------------------------------------------------------*/
/* SCALE_WET_WIDTH is a compensation weight factor to get an output
  amplitude (wet) rather independent of the width setting.
   0: the output amplitude is fully dependent on the width setting.
  >0: the output amplitude is less dependent on the width setting.
  With a SCALE_WET_WIDTH of 0.2 the output amplitude is rather
  independent of width setting (see fluid_revmodel_update()).
*/
const SCALE_WET_WIDTH: f32 = 0.2;

/* It is best to inject the input signal less ofen. This contributes to obtain
a flatter response on comb filter. So the input gain is set to 0.1 rather 1.0. */
const FIXED_GAIN: f32 = 0.1; /* input gain */

/* SCALE_WET is adjusted to 5.0 to get internal output level equivalent to freeverb */
const SCALE_WET: f32 = 5.0; /* scale output gain */

/*----------------------------------------------------------------------------
 Internal FDN late reverb settings
-----------------------------------------------------------------------------*/

/*-- Reverberation time settings ----------------------------------
 MIN_DC_REV_TIME est defined equal to the minimum value of freeverb:
 MAX_DC_REV_TIME est defined equal to the maximum value of freeverb:
 T60DC is computed from gi and the longest delay line in freeverb: L8 = 1617
 T60 = -3 * Li * T / log10(gi)
 T60 = -3 * Li *  / (log10(gi) * sr)

  - Li: length of comb filter delay line.
  - sr: sample rate.
  - gi: the feedback gain.

 The minimum value for freeverb correspond to gi = 0.7.
 with Mi = 1617, sr at 44100 Hz, and gi = 0.7 => MIN_DC_REV_TIME = 0.7 s

 The maximum value for freeverb correspond to gi = 0.98.
 with Mi = 1617, sr at 44100 Hz, and gi = 0.98 => MAX_DC_REV_TIME = 12.5 s
*/

const MIN_DC_REV_TIME: f32 = 0.7; /* minimum T60DC reverb time: seconds */
const MAX_DC_REV_TIME: f32 = 12.5; /* maximumm T60DC time in seconds */

/*-- Modulation related settings ----------------------------------*/
/* For many instruments, the range for MOD_FREQ and MOD_DEPTH should be:

 MOD_DEPTH: [3..6] (in samples).
 MOD_FREQ: [0.5 ..2.0] (in Hz).

 Values below the lower limits are often not sufficient to cancel unwanted
 "ringing"(resonant frequency).
 Values above upper limits augment the unwanted "chorus".

 With NBR_DELAYS to 8:
  MOD_DEPTH must be >= 4 to cancel the unwanted "ringing".[4..6].
 With NBR_DELAYS to 12:
  MOD_DEPTH to 3 is sufficient to cancel the unwanted "ringing".[3..6]
*/
const MOD_DEPTH: i32 = 4; /* modulation depth (samples)*/
const MOD_RATE: i32 = 50; /* modulation rate  (samples)*/
const MOD_FREQ: f32 = 1.0; /* modulation frequency (Hz) */
/*
 Number of samples to add to the desired length of a delay line. This
 allow to take account of modulation interpolation.
 1 is sufficient with MOD_DEPTH equal to 4.
*/
const INTERP_SAMPLES_NBR: i32 = 1;

/* phase offset between modulators waveform */
const MOD_PHASE: f32 = 360.0 / NBR_DELAYS as f32;

/*---------------------------------------------------------------------------*/
/* The FDN late feed back matrix: A
                            T
  A   = P  -  2 / N * u  * u
   N     N             N    N

  N: the matrix dimension (i.e NBR_DELAYS).
  P: permutation matrix.
  u: is a column vector of 1.

*/
const FDN_MATRIX_FACTOR: FluidReal = -2.0 / NBR_DELAYS as FluidReal;

/*----------------------------------------------------------------------------
             Internal FDN late structures and static functions
-----------------------------------------------------------------------------*/

/*-----------------------------------------------------------------------------
 Delay absorbent low pass filter
-----------------------------------------------------------------------------*/
/// Translation of `fdn_delay_lpf`.
#[derive(Clone, Copy, Default, Debug)]
struct FdnDelayLpf {
    buffer: FluidReal,
    b0: FluidReal,
    a1: FluidReal, /* filter coefficients */
}

/// Translation of `set_fdn_delay_lpf()`: sets coefficients for delay
/// absorbent low pass filter.
fn set_fdn_delay_lpf(lpf: &mut FdnDelayLpf, b0: FluidReal, a1: FluidReal) {
    lpf.b0 = b0;
    lpf.a1 = a1;
}

/*-----------------------------------------------------------------------------
 Delay line :
 The delay line is composed of the line plus an absorbent low pass filter
 to get frequency dependent reverb time.
-----------------------------------------------------------------------------*/
/// Translation of `delay_line`.
#[derive(Clone, Default, Debug)]
struct DelayLine {
    line: Vec<FluidReal>, /* buffer line */
    size: i32,            /* effective internal size (in samples) */
    /*-------------*/
    line_in: i32,  /* line in position */
    line_out: i32, /* line out position */
    /*-------------*/
    damping: FdnDelayLpf, /* damping low pass filter */
}

impl DelayLine {
    /// `dl->line[i]` (Note (upstream): nothing but the parameter ranges
    /// keeps the positions inside the line; outside, this reads 0).
    #[inline]
    fn at(&self, i: i32) -> FluidReal {
        self.line.get(i as usize).copied().unwrap_or(0.0)
    }
}

/// Translation of `clear_delay_line()`: clears a delay line to DC_OFFSET
/// float value.
fn clear_delay_line(dl: &mut DelayLine) {
    for l in dl.line.iter_mut() {
        *l = DC_OFFSET as FluidReal;
    }
}

/// Translation of `push_in_delay_line()`: push a sample val into the delay
/// line.
#[inline]
fn push_in_delay_line(dl: &mut DelayLine, val: FluidReal) {
    if let Some(l) = dl.line.get_mut(dl.line_in as usize) {
        *l = val;
    }
    /* Incrementation and circular motion if necessary */
    dl.line_in += 1;
    if dl.line_in >= dl.size {
        dl.line_in -= dl.size;
    }
}

/*-----------------------------------------------------------------------------
 Modulator for modulated delay line
-----------------------------------------------------------------------------*/

/*-----------------------------------------------------------------------------
 Sinusoidal modulator
-----------------------------------------------------------------------------*/
/* modulator are integrated in modulated delay line */
/// Translation of `sinus_modulator`.
#[derive(Clone, Copy, Default, Debug)]
struct SinusModulator {
    a1: FluidReal,            /* Coefficient: a1 = 2 * cos(w) */
    buffer1: FluidReal,       /* buffer1 */
    buffer2: FluidReal,       /* buffer2 */
    reset_buffer2: FluidReal, /* reset value of buffer2 */
}

/// Translation of `set_mod_frequency()`: sets the frequency of sinus
/// oscillator.
///
/// @param mod pointer on modulator structure.
/// @param freq frequency of the oscillator in Hz.
/// @param sample_rate sample rate on audio output in Hz.
/// @param phase initial phase of the oscillator in degree (0 to 360).
fn set_mod_frequency(mod_: &mut SinusModulator, freq: f32, sample_rate: f32, phase: f32) {
    let w = 2.0 * FLUID_M_PI * freq as FluidReal / sample_rate as FluidReal; /* initial angle */

    mod_.a1 = 2.0 * cos(w);

    let a = (2.0 * FLUID_M_PI / 360.0) * phase as FluidReal;

    mod_.buffer2 = sin(a - w); /* y(n-1) = sin(-initial angle) */
    mod_.buffer1 = sin(a); /* y(n) = sin(initial phase) */
    mod_.reset_buffer2 = sin(FLUID_M_PI / 2.0 - w); /* reset value for PI/2 */
}

/// Translation of `get_mod_sinus()`: gets current value of sinus
/// modulator:
///   y(n) = a1 . y(n-1)  -  y(n-2)
///   out = a1 . buffer1  -  buffer2
#[inline]
fn get_mod_sinus(mod_: &mut SinusModulator) -> FluidReal {
    let mut out = mod_.a1 * mod_.buffer1 - mod_.buffer2;
    mod_.buffer2 = mod_.buffer1;

    if out >= 1.0 {
        /* reset in case of instability near PI/2 */
        out = 1.0; /* forces output to the right value */
        mod_.buffer2 = mod_.reset_buffer2;
    }

    if out <= -1.0 {
        /* reset in case of instability near -PI/2 */
        out = -1.0; /* forces output to the right value */
        mod_.buffer2 = -mod_.reset_buffer2;
    }

    mod_.buffer1 = out;
    out
}

/*-----------------------------------------------------------------------------
 Modulated delay line. The line is composed of:
 - the delay line with its damping low pass filter.
 - the sinusoidal modulator.
 - center output position modulated by the modulator.
 - variable rate control of center output position.
 - first order All-Pass interpolator.
-----------------------------------------------------------------------------*/
/// Translation of `mod_delay_line`.
#[derive(Clone, Default, Debug)]
struct ModDelayLine {
    /* delay line with damping low pass filter member */
    dl: DelayLine, /* delayed line */
    /*---------------------------*/
    /* Sinusoidal modulator member */
    mod_: SinusModulator, /* sinus modulator */
    /*-------------------------*/
    /* center output position members */
    center_pos_mod: FluidReal, /* center output position modulated by modulator */
    mod_depth: i32,            /* modulation depth (in samples) */
    /*-------------------------*/
    /* variable rate control of center output position */
    index_rate: i32, /* index rate to know when to update center_pos_mod */
    mod_rate: i32,   /* rate at which center_pos_mod is updated */
    /*-------------------------*/
    /* first order All-Pass interpolator members */
    frac_pos_mod: FluidReal, /* fractional position part between samples) */
    /* previous value used when interpolating using fractional */
    buffer: FluidReal,
}

/// Translation of `get_mod_delay_line_length()`: return norminal delay
/// length.
fn get_mod_delay_line_length(mdl: &ModDelayLine) -> i32 {
    mdl.dl.size - mdl.mod_depth - INTERP_SAMPLES_NBR
}

/// Translation of `get_mod_delay()`: reads the sample value out of the
/// modulated delay line.
#[inline]
fn get_mod_delay(mdl: &mut ModDelayLine) -> FluidReal {
    let out_index: FluidReal; /* new modulated index position */
    let int_out_index: i32; /* integer part of out_index */
    let mut out: FluidReal; /* value to return */

    /* Checks if the modulator must be updated (every mod_rate samples). */
    /* Important: center_pos_mod must be used immediately for the
    first sample. So, mdl->index_rate must be initialized
    to mdl->mod_rate (set_mod_delay_line())  */

    mdl.index_rate += 1;
    if mdl.index_rate >= mdl.mod_rate {
        mdl.index_rate = 0;

        /* out_index = center position (center_pos_mod) + sinus waweform */
        out_index = mdl.center_pos_mod + get_mod_sinus(&mut mdl.mod_) * mdl.mod_depth as FluidReal;

        /* extracts integer part in int_out_index */
        if out_index >= 0.0 {
            int_out_index = out_index as i32; /* current integer part */

            /* forces read index (line_out)  with integer modulation value  */
            /* Boundary check and circular motion as needed */
            mdl.dl.line_out = int_out_index;
            if mdl.dl.line_out >= mdl.dl.size {
                mdl.dl.line_out -= mdl.dl.size;
            }
        } else {
            /* negative */
            int_out_index = (out_index - 1.0) as i32; /* previous integer part */
            /* forces read index (line_out) with integer modulation value  */
            /* circular motion as needed */
            mdl.dl.line_out = int_out_index.wrapping_add(mdl.dl.size);
        }

        /* extracts fractionnal part. (it will be used when interpolating
        between line_out and line_out +1) and memorize it.
        Memorizing is necessary for modulation rate above 1 */
        mdl.frac_pos_mod = out_index - int_out_index as FluidReal;

        /* updates center position (center_pos_mod) to the next position
        specified by modulation rate */
        mdl.center_pos_mod += mdl.mod_rate as FluidReal;
        if mdl.center_pos_mod >= mdl.dl.size as FluidReal {
            mdl.center_pos_mod -= mdl.dl.size as FluidReal;
        }
    }

    /*  First order all-pass interpolation ----------------------------------*/
    /* https://ccrma.stanford.edu/~jos/pasp/First_Order_Allpass_Interpolation.html */
    /*  begins interpolation: read current sample */
    out = mdl.dl.at(mdl.dl.line_out);

    /* updates line_out to the next sample.
    Boundary check and circular motion as needed */
    mdl.dl.line_out += 1;
    if mdl.dl.line_out >= mdl.dl.size {
        mdl.dl.line_out -= mdl.dl.size;
    }

    /* Fractional interpolation between next sample (at next position) and
    previous output added to current sample.
    */
    out += mdl.frac_pos_mod * (mdl.dl.at(mdl.dl.line_out) - mdl.buffer);
    mdl.buffer = out; /* memorizes current output */
    out
}

/*-----------------------------------------------------------------------------
 Late structure
-----------------------------------------------------------------------------*/
/// Translation of `fluid_late`.
#[derive(Clone, Default, Debug)]
struct FluidLate {
    samplerate: FluidReal,      /* sample rate */
    sample_rate_max: FluidReal, /* sample rate maximum */
    /*----- High pass tone corrector -------------------------------------*/
    tone_buffer: FluidReal,
    b1: FluidReal,
    b2: FluidReal,
    /*----- Modulated delay lines lines ----------------------------------*/
    mod_delay_lines: [ModDelayLine; NBR_DELAYS],
    /*-----------------------------------------------------------------------*/
    /* Output coefficients for separate Left and right stereo outputs */
    out_left_gain: [FluidReal; NBR_DELAYS], /* Left delay lines' output gains */
    out_right_gain: [FluidReal; NBR_DELAYS], /* Right delay lines' output gains*/
}

/*-----------------------------------------------------------------------------
 fluidsynth reverb structure
-----------------------------------------------------------------------------*/
/// Translation of `fluid_revmodel_t`.
#[derive(Clone, Debug)]
pub(crate) struct FluidRevmodel {
    /* reverb parameters */
    roomsize: FluidReal, /* acting on reverb time */
    damp: FluidReal,     /* acting on frequency dependent reverb time */
    level: FluidReal,
    wet1: FluidReal,
    wet2: FluidReal,  /* output level */
    width: FluidReal, /* width stereo separation */

    /* fdn reverberation structure */
    late: FluidLate,
}

/// Translation of `update_rev_time_damping()`: updates Reverb time and
/// absorbent filters coefficients from parameters.
///
/// @param late pointer on late structure.
/// @param roomsize (0 to 1): acting on reverb time.
/// @param damping (0 to 1): acting on absorbent damping filter.
///
/// Design formulas:
/// https://ccrma.stanford.edu/~jos/Reverb/First_Order_Delay_Filter_Design.html
/// https://ccrma.stanford.edu/~jos/Reverb/Tonal_Correction_Filter.html
fn update_rev_time_damping(late: &mut FluidLate, roomsize: FluidReal, damp: FluidReal) {
    let sample_period = 1.0 / late.samplerate; /* Sampling period */
    let mut delay_length: i32; /* delay length */
    let dc_rev_time: FluidReal; /* Reverb time at 0 Hz (in seconds) */

    let alpha: FluidReal;
    let alpha2: FluidReal;

    /*--------------------------------------------
         Computes dc_rev_time and alpha
    ----------------------------------------------*/
    {
        let gi_tmp: FluidReal;
        let ai_tmp: FluidReal;
        /*   roomsize parameters have the same response that Freeverb, that is:
         *   - roomsize (0 to 1) controls concave reverb time (0.7 to 10 s).
         */
        {
            /*-----------------------------------------
             Computes dc_rev_time
            ------------------------------------------*/
            /* values gi_min et gi_max are computed using E2 for the line with
            maximum delay */
            delay_length = get_mod_delay_line_length(&late.mod_delay_lines[NBR_DELAYS - 1]);
            let gi_max = pow(
                10.0,
                ((-3 * delay_length) as f32 / MAX_DC_REV_TIME) as FluidReal * sample_period,
            ); /* E2 */
            let gi_min = pow(
                10.0,
                ((-3 * delay_length) as f32 / MIN_DC_REV_TIME) as FluidReal * sample_period,
            ); /* E2 */
            /* gi = f(roomsize, gi_max, gi_min) */
            gi_tmp = gi_min + roomsize * (gi_max - gi_min);
            /* Computes T60DC from gi using inverse of relation E2.*/
            dc_rev_time =
                -3.0 * FLUID_M_LN10 * delay_length as FluidReal * sample_period / log(gi_tmp);
        }
        /*--------------------------------------------
            Computes alpha
        ----------------------------------------------*/
        /* Computes alpha from damp,ai_tmp,gi_tmp using relation R */
        /* - damp (0 to 1) controls concave reverb time for fs/2 frequency (T60DC to 0) */
        ai_tmp = 1.0 * damp;

        /* Preserve the square of R */
        alpha2 = 1.0 / (1.0 - ai_tmp / ((20.0f32 / 80.0f32) as FluidReal * log(gi_tmp)));

        alpha = sqrt(alpha2); /* R */
    }

    /* updates tone corrector coefficients b1,b2 from alpha */
    {
        /*
         Beta = (1 - alpha)  / (1 + alpha)
         b1 = 1/(1-beta)
         b2 = beta * b1
        */
        let beta = (1.0 - alpha) / (1.0 + alpha);
        late.b1 = 1.0 / (1.0 - beta);
        late.b2 = beta * late.b1;
        late.tone_buffer = 0.0;
    }

    /* updates damping  coefficients of all lines (gi , ai) from dc_rev_time, alpha */
    for i in 0..NBR_DELAYS {
        /* delay length */
        delay_length = get_mod_delay_line_length(&late.mod_delay_lines[i]);

        /* iir low pass filter gain */
        let gi = pow(
            10.0,
            (-3 * delay_length) as FluidReal * sample_period / dc_rev_time,
        );

        /* iir low pass filter feedback gain */
        let ai = (20.0f32 / 80.0f32) as FluidReal * log(gi) * (1.0 - 1.0 / alpha2);

        /* b0 = gi * (1 - ai),  a1 = - ai */
        set_fdn_delay_lpf(
            &mut late.mod_delay_lines[i].dl.damping,
            gi * (1.0 - ai),
            -ai,
        );
    }
}

/// Translation of `update_stereo_coefficient()`: updates stereo
/// coefficients.
///
/// @param late pointer on late structure
/// @param wet level integrated in stereo coefficients.
fn update_stereo_coefficient(late: &mut FluidLate, wet1: FluidReal) {
    for i in 0..NBR_DELAYS {
        /*  delay lines output gains vectors Left and Right

                           L    R
                       0 | 1    1|
                       1 |-1    1|
                       2 | 1   -1|
                       3 |-1   -1|

                       4 | 1    1|
                       5 |-1    1|
         stereo gain = 6 | 1   -1|
                       7 |-1   -1|

                       8 | 1    1|
                       9 |-1    1|
                       10| 1   -1|
                       11|-1   -1|
        */

        /* for left line: 00,  ,02,  ,04,  ,06,  ,08,  ,10,  ,12,... left_gain = +1 */
        /* for left line:   ,01,  ,03,  ,05,  ,07,  ,09,  ,11,...    left_gain = -1 */
        let mut wet = wet1;
        if i & 1 != 0 {
            wet = -wet1;
        }
        late.out_left_gain[i] = wet;

        /* for right line: 00,01,      ,04,05,     ,08,09,     ,12,13  right_gain = +1 */
        /* for right line:      ,02 ,03,     ,06,07,     ,10,11,...    right_gain = -1 */
        wet = wet1;
        if i & 2 != 0 {
            wet = -wet1;
        }
        late.out_right_gain[i] = wet;
    }
}

/* Nominal delay lines length table (in samples) */
const NOM_DELAY_LENGTH: [i32; NBR_DELAYS] = [601, 691, 773, 839, 919, 997, 1061, 1129];

/*
 1)"modal density" is one property that contributes to the quality of the reverb tail.
   The more is the modal density, the less are unwanted resonant frequencies
   build during the decay time: modal density = total delay / sample rate.

   Delay line's length given by static table delay_length[] are nominal
   to get minimum modal density of 0.15 at sample rate 44100Hz.
   Here we set length_factor to 2 to multiply this nominal modal
   density by 2. This leads to a default modal density of 0.15 * 2 = 0.3 for
   sample rate <= 44100.

   For sample rate > 44100, length_factor is multiplied by
   sample_rate / 44100. This ensures that the default modal density keeps inchanged.
   (Without this compensation, the default modal density would be diminished for
   new sample rate change above 44100Hz).

 2)Modulated delay line contributes to diminish resonnant frequencies (often called "ringing").
   Modulation depth (mod_depth) is set to nominal value of MOD_DEPTH at sample rate 44100Hz.
   For sample rate > 44100, mod_depth is multiplied by sample_rate / 44100. This ensures
   that the effect of modulated delay line remains inchanged.
*/
/// Translation of `compensate_from_sample_rate()`: returns `(mod_depth,
/// length_factor)`.
fn compensate_from_sample_rate(sample_rate: FluidReal) -> (FluidReal, FluidReal) {
    let mut mod_depth = MOD_DEPTH as FluidReal;
    let mut length_factor: FluidReal = 2.0;
    if sample_rate > 44100.0 {
        let sample_rate_factor = sample_rate / 44100.0;
        length_factor *= sample_rate_factor;
        mod_depth *= sample_rate_factor;
    }
    (mod_depth, length_factor)
}

/// Translation of `create_mod_delay_lines()`: creates all modulated lines.
///
/// @param late, pointer on the fnd late reverb to initialize.
/// @param sample_rate_max, the maximum audio sample rate expected.
/// @return FLUID_OK if success, FLUID_FAILED otherwise.
fn create_mod_delay_lines(late: &mut FluidLate, sample_rate_max: FluidReal) -> bool {
    /* compute mod_depth, length factor */
    let (mut mod_depth, length_factor) = compensate_from_sample_rate(sample_rate_max);

    late.sample_rate_max = sample_rate_max;

    for i in 0..NBR_DELAYS {
        /* for each delay line */
        let delay_length = (NOM_DELAY_LENGTH[i] as FluidReal * length_factor) as i32;
        let mdl = &mut late.mod_delay_lines[i];

        /*-------------------------------------------------------------------*/
        /* checks parameter */
        if delay_length < 1 {
            return false;
        }

        /* limits mod_depth to the requested delay length */
        if mod_depth >= delay_length as FluidReal {
            fluid_log(FLUID_INFO, "fdn reverb: modulation depth has been limited");
            mod_depth = (delay_length - 1) as FluidReal;
        }

        /*---------------------------------------------------------------------
         allocates delay lines
        */

        /* real size of the line in use (in samples):
        size = INTERP_SAMPLES_NBR + mod_depth + delay_length */
        mdl.dl.size =
            (delay_length as FluidReal + mod_depth + INTERP_SAMPLES_NBR as FluidReal) as i32;
        // (C's FLUID_ARRAY leaves the line uninitialized until it is cleared)
        mdl.dl.line = vec![0.0; mdl.dl.size.max(0) as usize];
    }
    true
}

/// Translation of `initialize_mod_delay_lines()`: initialize all modulated
/// lines.
///
/// @param late, pointer on the fnd late reverb to initialize.
/// @param sample_rate, the audio sample rate.
fn initialize_mod_delay_lines(late: &mut FluidLate, sample_rate: FluidReal) {
    /* update delay line parameter dependent of sample rate */
    late.samplerate = sample_rate;

    /* compute mod_depth, length factor */
    let (mut mod_depth, length_factor) = compensate_from_sample_rate(sample_rate);

    for i in 0..NBR_DELAYS {
        /* for each delay line */
        let mdl = &mut late.mod_delay_lines[i];
        let delay_length = (NOM_DELAY_LENGTH[i] as FluidReal * length_factor) as i32;

        /* limits mod_depth to the requested delay length */
        if mod_depth >= delay_length as FluidReal {
            mod_depth = (delay_length - 1) as FluidReal;
        }

        mdl.mod_depth = mod_depth as i32;

        clear_delay_line(&mut mdl.dl); /* clears the buffer */

        /* Initializes line_in to the start of the buffer */
        mdl.dl.line_in = 0;

        /* Initializes line_out index INTERP_SAMPLES_NBR samples after
        line_in so that the delay between line_out and line_in is:
        mod_depth + delay_length
        */
        mdl.dl.line_out = mdl.dl.line_in + INTERP_SAMPLES_NBR;

        /* Damping low pass filter ------------------------------------------*/
        mdl.dl.damping.buffer = 0.0;

        /*---------------------------------------------------------------------
         Initializes modulation members:
         - modulated center position: center_pos_mod
         - modulation rate (the speed at which center_pos_mod is modulated: mod_rate
         - index rate to know when to update center_pos_mod:index_rate
         - interpolator member: buffer, frac_pos_mod
        ---------------------------------------------------------------------*/
        /* Initializes the modulated center position (center_pos_mod) so that:
           - the delay between line_out and center_pos_mod is mod_depth.
           - the delay between center_pos_mod and line_in is delay_length.
        */
        mdl.center_pos_mod = INTERP_SAMPLES_NBR as FluidReal + mod_depth;

        /* Sets the modulation rate. This rate defines how often
        the  center position (center_pos_mod ) is modulated .
        The value is expressed in samples. The default value is 1 that means that
        center_pos_mod is updated at every sample.
        For example with a value of 2, the center position position will be
        updated only one time every 2 samples only.
        */
        if MOD_RATE < 1 || MOD_RATE > mdl.dl.size {
            fluid_log(FLUID_INFO, "fdn reverb: modulation rate is out of range");
            mdl.mod_rate = 1; /* default modulation rate: every one sample */
        } else {
            mdl.mod_rate = MOD_RATE;
        }

        /* index rate to control when to update center_pos_mod.
        Important: must be set to get center_pos_mod immediately used for
        the reading of first sample (see get_mod_delay())
        */
        mdl.index_rate = mdl.mod_rate;

        /* initializes first order All-Pass interpolator members */
        mdl.buffer = 0.0; /* previous delay sample value */
        mdl.frac_pos_mod = 0.0; /* frac. position (between consecutives sample) */

        /* Sets local Modulators parameters: frequency and phase.
        Each modulateur are shifted of MOD_PHASE degree
        */
        set_mod_frequency(
            &mut mdl.mod_,
            MOD_FREQ * MOD_RATE as f32,
            sample_rate as f32,
            MOD_PHASE * i as f32,
        );
    }
}

impl FluidRevmodel {
    /// Translation of `fluid_revmodel_init()`: clears the delay lines.
    fn init(&mut self) {
        /* clears all the delay lines */
        for i in 0..NBR_DELAYS {
            clear_delay_line(&mut self.late.mod_delay_lines[i].dl);
        }
    }

    /// Translation of `fluid_revmodel_update()`: updates internal
    /// parameters.
    fn update(&mut self) {
        /* Recalculate internal values after parameters change */

        /* The stereo amplitude equation (wet1 and wet2 below) have a
        tendency to produce high amplitude with high width values ( 1 < width < 100).
        This results in an unwanted noisy output clipped by the audio card.
        To avoid this dependency, we divide by (1 + rev->width * SCALE_WET_WIDTH)
        Actually, with a SCALE_WET_WIDTH of 0.2, (regardless of level setting),
        the output amplitude (wet) seems rather independent of width setting */
        let wet = (self.level * SCALE_WET as FluidReal)
            / (1.0 + self.width * SCALE_WET_WIDTH as FluidReal);

        /* wet1 and wet2 are used by the stereo effect controlled by the width setting
        for producing a stereo ouptput from a monophonic reverb signal.
        Please see the note above about a side effect tendency */

        self.wet1 = wet * (self.width / 2.0 + 0.5);
        self.wet2 = wet * ((1.0 - self.width) / 2.0);

        /* integrates wet1 in stereo coefficient (this will save one multiply) */
        update_stereo_coefficient(&mut self.late, self.wet1);

        if self.wet1 > 0.0 {
            self.wet2 /= self.wet1;
        }

        /* Reverberation time and damping */
        update_rev_time_damping(&mut self.late, self.roomsize, self.damp);
    }

    /*----------------------------------------------------------------------------
                                Reverb API
    -----------------------------------------------------------------------------*/
    /// Translation of `new_fluid_revmodel()`.
    ///
    /// Creates a reverb. Once created the reverb have no parameters set, so
    /// fluid_revmodel_set() must be called at least one time after calling
    /// new_fluid_revmodel().
    ///
    /// @param sample_rate_max maximum sample rate expected in Hz.
    ///
    /// @param sample_rate actual sample rate needed in Hz.
    /// @return pointer on the new reverb or NULL if memory error.
    pub(crate) fn new(
        mut sample_rate_max: FluidReal,
        sample_rate: FluidReal,
    ) -> Option<FluidRevmodel> {
        if sample_rate <= 0.0 {
            return None;
        }

        // Note (upstream): only the late reverb is cleared; the parameters
        // stay uninitialized until fluid_revmodel_set(), whose event the
        // synth only dispatches after its first API call, so the first
        // blocks a synth renders read them uninitialized. Here they are 0.
        let mut rev = FluidRevmodel {
            roomsize: 0.0,
            damp: 0.0,
            level: 0.0,
            wet1: 0.0,
            wet2: 0.0,
            width: 0.0,
            late: FluidLate::default(),
        };

        /*--------------------------------------------------------------------------
          Create fdn late reverb.
        */

        /* update minimum value for sample_rate_max */
        if sample_rate > sample_rate_max {
            sample_rate_max = sample_rate;
        }

        /*--------------------------------------------------------------------------
          Allocate the modulated delay lines
        */
        if !create_mod_delay_lines(&mut rev.late, sample_rate_max) {
            return None;
        }

        /*--------------------------------------------------------------------------
          Initialize the fdn reverb
        */
        /* Initialize all modulated lines. */
        initialize_mod_delay_lines(&mut rev.late, sample_rate);

        Some(rev)
    }

    /// Translation of `fluid_revmodel_set()`.
    ///
    /// Sets one or more reverb parameters. Note this must be called at least one
    /// time after calling new_fluid_revmodel() and before any call to
    /// fluid_revmodel_processXXX() and fluid_revmodel_samplerate_change().
    pub(crate) fn set(
        &mut self,
        set: i32,
        mut roomsize: FluidReal,
        mut damping: FluidReal,
        width: FluidReal,
        mut level: FluidReal,
    ) {
        /*-----------------------------------*/
        if set & FLUID_REVMODEL_SET_ROOMSIZE != 0 {
            roomsize = fluid_clip(roomsize, 0.0, 1.0);
            self.roomsize = roomsize;
        }

        /*-----------------------------------*/
        if set & FLUID_REVMODEL_SET_DAMPING != 0 {
            damping = fluid_clip(damping, 0.0, 1.0);
            self.damp = damping;
        }

        /*-----------------------------------*/
        if set & FLUID_REVMODEL_SET_WIDTH != 0 {
            self.width = width;
        }

        /*-----------------------------------*/
        if set & FLUID_REVMODEL_SET_LEVEL != 0 {
            level = fluid_clip(level, 0.0, 1.0);
            self.level = level;
        }

        /* updates internal parameters */
        self.update();
    }

    /// Translation of `fluid_revmodel_samplerate_change()`: applies a
    /// sample rate change on the reverb.
    pub(crate) fn samplerate_change(&mut self, mut sample_rate: FluidReal) -> bool {
        let mut status = true;

        if sample_rate > self.late.sample_rate_max {
            fluid_log(
                FLUID_WARN,
                &format!(
                    "fdn reverb: sample rate {:.0} Hz is deduced to {:.0} Hz\n",
                    sample_rate, self.late.sample_rate_max
                ),
            );

            /* Reduce sample rate to the maximum value set at creation time.
            The reverb will continue to work with possible lost of quality.
            */
            sample_rate = self.late.sample_rate_max;
            status = false;
        }

        /* Initialize all modulated lines according to sample rate change. */
        initialize_mod_delay_lines(&mut self.late, sample_rate);

        /* updates damping filter coefficients according to sample rate change */
        update_rev_time_damping(&mut self.late, self.roomsize, self.damp);

        status
    }

    /// Translation of `fluid_revmodel_reset()`: damps the reverb by clearing
    /// the delay lines.
    pub(crate) fn reset(&mut self) {
        self.init();
    }

    /// One sample of `fluid_revmodel_processmix()` and
    /// `fluid_revmodel_processreplace()`: the output.
    #[inline]
    fn process_sample(&mut self, input: FluidReal) -> (FluidReal, FluidReal) {
        let mut delay_out: [FluidReal; NBR_DELAYS] = [0.0; NBR_DELAYS]; /* Line output + damper output */

        /* stereo output */
        let mut out_left: FluidReal = 0.0;
        let mut out_right: FluidReal = 0.0;

        /* Input is adjusted by DC_OFFSET. */
        let mut xn = input * FIXED_GAIN as FluidReal + DC_OFFSET as FluidReal; /* mono input x(n) */

        /*--------------------------------------------------------------------
         tone correction.
        */
        let out_tone_filter = xn * self.late.b1 - self.late.b2 * self.late.tone_buffer; /* tone corrector output */
        self.late.tone_buffer = xn;
        xn = out_tone_filter;
        /*--------------------------------------------------------------------
         process  feedback delayed network:
          - xn is the input signal.
          - before inserting in the line input we first we get the delay lines
            output, filter them and compute output in delay_out[].
          - also matrix_factor is computed (to simplify further matrix product)
        ---------------------------------------------------------------------*/
        /* We begin with the modulated output delay line + damping filter */
        let mut matrix_factor: FluidReal = 0.0; /* partial matrix computation */

        for i in 0..NBR_DELAYS {
            let mdl = &mut self.late.mod_delay_lines[i];
            /* get current modulated output */
            let mut delay_out_s = get_mod_delay(mdl); /* sample */

            /* process low pass damping filter
            (input:delay_out_s, output:delay_out_s) */
            delay_out_s =
                delay_out_s * mdl.dl.damping.b0 - mdl.dl.damping.buffer * mdl.dl.damping.a1;
            mdl.dl.damping.buffer = delay_out_s;

            /* Result in delay_out[], and matrix_factor.
            These will be of use later during input line process */
            delay_out[i] = delay_out_s; /* result in delay_out[] */
            matrix_factor += delay_out_s; /* result in matrix_factor */

            /* Process stereo output */
            /* stereo left = left + out_left_gain * delay_out */
            out_left += self.late.out_left_gain[i] * delay_out_s;
            /* stereo right= right+ out_right_gain * delay_out */
            out_right += self.late.out_right_gain[i] * delay_out_s;
        }

        /* now we process the input delay line.Each input is a combination of
           - xn: input signal
           - delay_out[] the output of a delay line given by a permutation matrix P
           - and matrix_factor.
          This computes: in_delay_line = xn + (delay_out[] * matrix A) with
          an algorithm equivalent but faster than using a product with matrix A.
        */
        /* matrix_factor = output sum * (-2.0)/N  */
        matrix_factor *= FDN_MATRIX_FACTOR;
        matrix_factor += xn; /* adds reverb input signal */

        for i in 1..NBR_DELAYS {
            /* delay_in[i-1] = delay_out[i] + matrix_factor */
            let dl = &mut self.late.mod_delay_lines[i - 1].dl;
            push_in_delay_line(dl, delay_out[i] + matrix_factor);
        }

        /* last line input (NB_DELAY-1) */
        /* delay_in[0] = delay_out[NB_DELAY -1] + matrix_factor */
        {
            let dl = &mut self.late.mod_delay_lines[NBR_DELAYS - 1].dl;
            push_in_delay_line(dl, delay_out[0] + matrix_factor);
        }

        /*-------------------------------------------------------------------*/
        /* Removes the DC offset */
        out_left -= DC_OFFSET as FluidReal;
        out_right -= DC_OFFSET as FluidReal;

        /*
            left_out[k]  = out_left * rev->wet1 + out_right * rev->wet2;
            right_out[k] = out_right * rev->wet1 + out_left * rev->wet2;

            As wet1 is integrated in stereo coefficient wet 1 is now
            integrated in out_left and out_right, so we simplify previous
            relation by suppression of one multiply as this:

            left_out[k]  = out_left  + out_right * rev->wet2;
            right_out[k] = out_right + out_left * rev->wet2;
        */
        (
            out_left + out_right * self.wet2,
            out_right + out_left * self.wet2,
        )
    }

    /// Translation of `fluid_revmodel_processreplace()`: fdn reverb process
    /// replace.
    ///
    /// The processed reverb is replacing anything there in out.
    pub(crate) fn processreplace(
        &mut self,
        in_: &[FluidReal],
        left_out: &mut [FluidReal],
        right_out: &mut [FluidReal],
    ) {
        for k in 0..FLUID_BUFSIZE {
            /* Calculates stereo output REPLACING anything already there: */
            let (l, r) = self.process_sample(in_[k]);
            left_out[k] = l;
            right_out[k] = r;
        }
    }

    /// Translation of `fluid_revmodel_processmix()`: fdn reverb process mix.
    ///
    /// The processed reverb is mixed in out with samples already there in out.
    pub(crate) fn processmix(
        &mut self,
        in_: &[FluidReal],
        left_out: &mut [FluidReal],
        right_out: &mut [FluidReal],
    ) {
        for k in 0..FLUID_BUFSIZE {
            /* Calculates stereo output MIXING anything already there: */
            let (l, r) = self.process_sample(in_[k]);
            left_out[k] += l;
            right_out[k] += r;
        }
    }
}
