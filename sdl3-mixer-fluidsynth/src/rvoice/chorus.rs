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
// Modified 2026-10-17: translated into Rust from fluid_chorus.c and
// fluid_chorus.h in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE).

/*
 based on a chorus implementation made by Juergen Mueller And Sundry Contributors in 1998

 CHANGES

 - Adapted for fluidsynth, Peter Hanappe, March 2002

 - Variable delay line implementation using bandlimited
   interpolation, code reorganization: Markus Nentwig May 2002

 - Complete rewrite using lfo computed on the fly, first order all-pass
   interpolator and adding stereo unit: Jean-Jacques Ceresa, Jul 2019
*/

/*
 * 	Chorus effect.
 *
 * Flow diagram scheme for n delays ( 1 <= n <= MAX_CHORUS ):
 *
 *                                                       ________
 *                  direct signal (not implemented) >-->|        |
 *                 _________                            |        |
 * mono           |         |                           |        |
 * input ---+---->| delay 1 |-------------------------->| Stereo |---> right
 *          |     |_________|                           |        |     output
 *          |        /|\                                | Unit   |
 *          :         |                                 |        |
 *          : +-----------------+                       |(width) |
 *          : | Delay control 1 |<-+                    |        |
 *          : +-----------------+  |                    |        |---> left
 *          |      _________       |                    |        |     output
 *          |     |         |      |                    |        |
 *          +---->| delay n |-------------------------->|        |
 *                |_________|      |                    |        |
 *                   /|\           |                    |________|
 *                    |            |  +--------------+      /|\
 *            +-----------------+  |  |mod depth (ms)|       |
 *            | Delay control n |<-*--|lfo speed (Hz)|     gain-out
 *            +-----------------+     +--------------+
 *
 *
 * The delay i is controlled by a sine or triangle modulation i ( 1 <= i <= n).
 *
 * The chorus unit process a monophonic input signal and produces stereo output
 * controlled by WIDTH macro.
 * Actually WIDTH is fixed to maximum value. But in the future, we could add a
 * setting (e.g "synth.chorus.width") allowing the user to get a gradually stereo
 * effect from minimum (monophonic) to maximum stereo effect.
 *
 * Delays lines are implemented using only one line for all chorus blocks.
 * Each chorus block has it own lfo (sinus/triangle). Each lfo are out of phase
 * to produce uncorrelated signal at the output of the delay line (this simulates
 * the presence of individual line for each block). Each lfo modulates the length
 * of the line using a depth modulation value and lfo frequency value common to
 * all lfos.
 *
 * LFO modulators are computed on the fly, instead of using lfo lookup table.
 * The advantages are:
 * - Avoiding a lost of 608272 memory bytes when lfo speed is low (0.3Hz).
 * - Allows to diminish the lfo speed lower limit to 0.1Hz instead of 0.3Hz.
 *   A speed of 0.1 is interesting for chorus. Using a lookuptable for 0.1Hz
 *   would require too much memory (1824816 bytes).
 * - Interpolation make use of first order all-pass interpolator instead of
 *   bandlimited interpolation.
 * - Although lfo modulator is computed on the fly, cpu load is lower than
 *   using lfo lookup table with bandlimited interpolator.
 */

use sdl3::stdlib::math::{cos, sin};

use crate::fluidsynth_h::{FLUID_CHORUS_MOD_SINE, FLUID_CHORUS_MOD_TRIANGLE};
use crate::fluidsynth_priv::{FLUID_BUFSIZE, M_PI};
use crate::utils::sys::{fluid_log, FLUID_WARN};
use crate::FluidReal;

/* enum describing each chorus parameter */
pub(crate) const FLUID_CHORUS_NR: usize = 0; /*< number of delay line */
pub(crate) const FLUID_CHORUS_LEVEL: usize = 1; /*< output level */
pub(crate) const FLUID_CHORUS_SPEED: usize = 2; /*< lfo frequency */
pub(crate) const FLUID_CHORUS_DEPTH: usize = 3; /*< modulation depth */
pub(crate) const FLUID_CHORUS_TYPE: usize = 4; /*< type of waveform */
pub(crate) const FLUID_CHORUS_PARAM_LAST: usize = 5; /* number of enum fluid_chorus_param */

/* return a bit flag from param: 2^param */
pub(crate) const fn fluid_chorparam_to_setflag(param: usize) -> i32 {
    1 << param
}

/* Flags for fluid_chorus_set() */
pub(crate) const FLUID_CHORUS_SET_NR: i32 = fluid_chorparam_to_setflag(FLUID_CHORUS_NR);
pub(crate) const FLUID_CHORUS_SET_LEVEL: i32 = fluid_chorparam_to_setflag(FLUID_CHORUS_LEVEL);
pub(crate) const FLUID_CHORUS_SET_SPEED: i32 = fluid_chorparam_to_setflag(FLUID_CHORUS_SPEED);
pub(crate) const FLUID_CHORUS_SET_DEPTH: i32 = fluid_chorparam_to_setflag(FLUID_CHORUS_DEPTH);
pub(crate) const FLUID_CHORUS_SET_TYPE: i32 = fluid_chorparam_to_setflag(FLUID_CHORUS_TYPE);
/** Value for fluid_chorus_set() which sets all chorus parameters. */
pub(crate) const FLUID_CHORUS_SET_ALL: i32 = FLUID_CHORUS_SET_NR
    | FLUID_CHORUS_SET_LEVEL
    | FLUID_CHORUS_SET_SPEED
    | FLUID_CHORUS_SET_DEPTH
    | FLUID_CHORUS_SET_TYPE;

/*-------------------------------------------------------------------------------------
  Private
--------------------------------------------------------------------------------------*/
const MAX_CHORUS: i32 = 99; /* number maximum of block */
const MAX_LEVEL: FluidReal = 10.0; /* max output level */
const MIN_SPEED_HZ: FluidReal = 0.1; /* min lfo frequency (Hz) */
const MAX_SPEED_HZ: FluidReal = 5.0; /* max lfo frequency (Hz) */

/* WIDTH [0..10] value define a stereo separation between left and right.
 When 0, the output is monophonic. When > 0 , the output is stereophonic.
 Actually WIDTH is fixed to maximum value. But in the future we could add a setting to
 allow a gradually stereo effect from minimum (monophonic) to maximum stereo effect.
*/
const WIDTH: FluidReal = 10.0;

/* SCALE_WET_WIDTH is a compensation weight factor to get an output
  amplitude (wet) rather independent of the width setting.
   0: the output amplitude is fully dependent on the width setting.
  >0: the output amplitude is less dependent on the width setting.
  With a SCALE_WET_WIDTH of 0.2 the output amplitude is rather
  independent of width setting (see fluid_chorus_set()).
*/
const SCALE_WET_WIDTH: f32 = 0.2;
const SCALE_WET: f32 = 1.0;

const MAX_SAMPLES: i32 = 2048; /* delay length in sample (46.4 ms at sample rate: 44100Hz).*/
const LOW_MOD_DEPTH: i32 = 176; /* low mod_depth/2 in samples */
const HIGH_MOD_DEPTH: i32 = MAX_SAMPLES / 2; /* high mod_depth in sample */
const RANGE_MOD_DEPTH: i32 = HIGH_MOD_DEPTH - LOW_MOD_DEPTH;

/* Important min max values for MOD_RATE */
/* mod rate define the rate at which the modulator is updated. Examples
   50: the modulator is updated every 50 samples (less cpu cycles expensive).
   1: the modulator is updated every sample (more cpu cycles expensive).
*/
/* MOD_RATE acceptable for max lfo speed (5Hz) and max modulation depth (46.6 ms) */
const LOW_MOD_RATE: i32 = 5; /* MOD_RATE acceptable for low modulation depth (8 ms) */
const HIGH_MOD_RATE: i32 = 4; /* MOD_RATE acceptable for max modulation depth (46.6 ms) */
/* and max lfo speed (5 Hz) */
const RANGE_MOD_RATE: i32 = HIGH_MOD_RATE - LOW_MOD_RATE;

/*
 Number of samples to add to the desired length of the delay line. This
 allows to take account of rounding error interpolation when using large
 modulation depth.
 1 is sufficient for max modulation depth (46.6 ms) and max lfo speed (5 Hz).
*/
//#define INTERP_SAMPLES_NBR 0
const INTERP_SAMPLES_NBR: i32 = 1;

/*-----------------------------------------------------------------------------
 Sinusoidal modulator
-----------------------------------------------------------------------------*/
/// Translation of `sinus_modulator`.
#[derive(Clone, Copy, Default, Debug)]
struct SinusModulator {
    // for sufficient precision members MUST be double! See https://github.com/FluidSynth/fluidsynth/issues/1331
    a1: f64,            /* Coefficient: a1 = 2 * cos(w) */
    buffer1: f64,       /* buffer1 */
    buffer2: f64,       /* buffer2 */
    reset_buffer2: f64, /* reset value of buffer2 */
}

/*-----------------------------------------------------------------------------
 Triangle modulator
-----------------------------------------------------------------------------*/
/// Translation of `triang_modulator`.
#[derive(Clone, Copy, Default, Debug)]
struct TriangModulator {
    freq: FluidReal, /* Osc. Frequency (in Hertz) */
    val: FluidReal,  /* internal current value */
    inc: FluidReal,  /* increment value */
}

/*-----------------------------------------------------------------------------
 modulator
-----------------------------------------------------------------------------*/
/// Translation of `modulator`.
#[derive(Clone, Copy, Default, Debug)]
struct Modulator {
    /*-------------*/
    line_out: i32, /* current line out position for this modulator */
    /*-------------*/
    sinus: SinusModulator,   /* sinus lfo */
    triang: TriangModulator, /* triangle lfo */
    /*-------------------------*/
    /* first order All-Pass interpolator members */
    frac_pos_mod: FluidReal, /* fractional position part between samples */
    /* previous value used when interpolating using fractional */
    buffer: FluidReal,
}

/// Translation of `fluid_chorus_t`.
#[derive(Clone, Debug)]
pub(crate) struct FluidChorus {
    type_: i32,
    depth_ms: FluidReal,
    level: FluidReal,
    speed_hz: FluidReal,
    number_blocks: i32,
    sample_rate: FluidReal,

    /* width control: 0 monophonic, > 0 more stereophonic */
    width: FluidReal,
    wet1: FluidReal,
    wet2: FluidReal,

    line: Vec<FluidReal>, /* buffer line */
    size: i32,            /* effective internal size (in samples) */

    line_in: i32, /* line in position */

    /* center output position members */
    center_pos_mod: FluidReal, /* center output position modulated by modulator */
    mod_depth: i32,            /* modulation depth (in samples) */

    /* variable rate control of center output position */
    index_rate: i32, /* index rate to know when to update center_pos_mod */
    mod_rate: i32,   /* rate at which center_pos_mod is updated */

    /* modulator member */
    mod_: [Modulator; MAX_CHORUS as usize], /* sinus/triangle modulator */
}

/// Translation of `set_sinus_frequency()`: sets the frequency of sinus
/// oscillator.
///
/// For sufficient precision use double precision in set_sinus_frequency() computation !.
/// Never use: fluid_real_t , cosf(), sinf(), FLUID_COS(), FLUID_SIN(), FLUID_M_PI.
/// See https://github.com/FluidSynth/fluidsynth/issues/1331
///
/// @param mod pointer on modulator structure.
/// @param freq frequency of the oscillator in Hz.
/// @param sample_rate sample rate on audio output in Hz.
/// @param phase initial phase of the oscillator in degree (0 to 360).
fn set_sinus_frequency(mod_: &mut SinusModulator, freq: f32, sample_rate: f32, phase: f32) {
    let w = (2.0 * M_PI) * freq as f64 / sample_rate as f64; /* step phase between each sinus wave sample (in radian) */

    // DO NOT use potentially single precision cosf or FLUID_COS here! See https://github.com/FluidSynth/fluidsynth/issues/1331
    mod_.a1 = 2.0 * cos(w);

    let a = (2.0 * M_PI / 360.0) * phase as f64; /* initial phase at which the sinus wave must begin (in radian) */

    mod_.buffer2 = sin(a - w); /* y(n-1) = sin(-initial angle) */
    mod_.buffer1 = sin(a); /* y(n) = sin(initial phase) */
    mod_.reset_buffer2 = sin((M_PI / 2.0) - w); /* reset value for PI/2 */
}

/// Translation of `get_mod_sinus()`: gets current value of sinus modulator:
///   y(n) = a1 . y(n-1)  -  y(n-2)
///   out = a1 . buffer1  -  buffer2
#[inline]
fn get_mod_sinus(mod_: &mut SinusModulator) -> f64 {
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

/// Translation of `set_triangle_frequency()`: set the frequency of
/// triangular oscillator.
///
/// The frequency is converted in a slope value.
/// The initial value is set according to frac_phase which is a position
/// in the period relative to the beginning of the period.
/// For example: 0 is the beginning of the period, 1/4 is at 1/4 of the period
/// relative to the beginning.
fn set_triangle_frequency(
    mod_: &mut TriangModulator,
    mut freq: f32,
    sample_rate: f32,
    frac_phase: f32,
) {
    if freq <= 0.0 {
        freq = 0.5;
    }

    mod_.freq = freq as FluidReal;

    let ns_period = (sample_rate / freq) as FluidReal; /* period in numbers of sample */

    /* the slope of a triangular osc (0 up to +1 down to -1 up to 0....) is equivalent
    to the slope of a saw osc (0 -> +4) */
    mod_.inc = 4.0 / ns_period; /* positive slope */

    /* The initial value and the sign of the slope depend of initial phase:
      initial value = = (ns_period * frac_phase) * slope
    */
    mod_.val = ns_period * frac_phase as FluidReal * mod_.inc;

    if 1.0 <= mod_.val && mod_.val < 3.0 {
        mod_.val = 2.0 - mod_.val; /*  1.0 down to -1.0 */
        mod_.inc = -mod_.inc; /* negative slope */
    } else if 3.0 <= mod_.val {
        mod_.val -= 4.0; /*  -1.0 up to +1.0. */
    }

    /* else val < 1.0 */
}

/// Translation of `get_mod_triang()`: get current value of triangular
/// oscillator
///       y(n) = y(n-1) + dy
#[inline]
fn get_mod_triang(mod_: &mut TriangModulator) -> FluidReal {
    mod_.val += mod_.inc;

    if mod_.val >= 1.0 {
        mod_.inc = -mod_.inc;
        return 1.0;
    }

    if mod_.val <= -1.0 {
        mod_.inc = -mod_.inc;
        return -1.0;
    }

    mod_.val
}

impl FluidChorus {
    /// `chorus->line[i]` (FIXME (upstream): nothing but the parameter
    /// ranges keeps the read position inside the line; outside, this reads
    /// 0).
    #[inline]
    fn line_at(&self, i: i32) -> FluidReal {
        self.line.get(i as usize).copied().unwrap_or(0.0)
    }

    /// Translation of `get_mod_delay()`: reads the sample value out of the
    /// modulated delay line.
    #[inline]
    fn get_mod_delay(&mut self, m: usize) -> FluidReal {
        let out_index: FluidReal; /* new modulated index position */
        let int_out_index: i32; /* integer part of out_index */
        let mut out: FluidReal; /* value to return */

        /* Checks if the modulator must be updated (every mod_rate samples). */
        /* Important: center_pos_mod must be used immediately for the
        first sample. So, mdl->index_rate must be initialized
        to mdl->mod_rate (new_mod_delay_line())  */

        if self.index_rate >= self.mod_rate {
            /* out_index = center position (center_pos_mod) + sinus waweform */
            if self.type_ == FLUID_CHORUS_MOD_SINE {
                out_index = self.center_pos_mod
                    + get_mod_sinus(&mut self.mod_[m].sinus) * self.mod_depth as f64;
            } else {
                out_index = self.center_pos_mod
                    + get_mod_triang(&mut self.mod_[m].triang) * self.mod_depth as FluidReal;
            }

            /* extracts integer part in int_out_index */
            if out_index >= 0.0 {
                int_out_index = out_index as i32; /* current integer part */

                /* forces read index (line_out)  with integer modulation value  */
                /* Boundary check and circular motion as needed */
                self.mod_[m].line_out = int_out_index;
                if self.mod_[m].line_out >= self.size {
                    self.mod_[m].line_out -= self.size;
                }
            } else {
                /* negative */
                int_out_index = (out_index - 1.0) as i32; /* previous integer part */
                /* forces read index (line_out) with integer modulation value  */
                /* circular motion as needed */
                self.mod_[m].line_out = int_out_index.wrapping_add(self.size);
            }

            /* extracts fractionnal part. (it will be used when interpolating
            between line_out and line_out +1) and memorize it.
            Memorizing is necessary for modulation rate above 1 */
            self.mod_[m].frac_pos_mod = out_index - int_out_index as FluidReal;
        }

        /*  First order all-pass interpolation ----------------------------------*/
        /* https://ccrma.stanford.edu/~jos/pasp/First_Order_Allpass_Interpolation.html */
        /*  begins interpolation: read current sample */
        out = self.line_at(self.mod_[m].line_out);

        /* updates line_out to the next sample.
        Boundary check and circular motion as needed */
        self.mod_[m].line_out += 1;
        if self.mod_[m].line_out >= self.size {
            self.mod_[m].line_out -= self.size;
        }

        /* Fractional interpolation between next sample (at next position) and
        previous output added to current sample.
        */
        out +=
            self.mod_[m].frac_pos_mod * (self.line_at(self.mod_[m].line_out) - self.mod_[m].buffer);
        self.mod_[m].buffer = out; /* memorizes current output */
        out
    }

    /// Translation of `push_in_delay_line()`: push a sample val into the
    /// delay line.
    #[inline]
    fn push_in_delay_line(&mut self, val: FluidReal) {
        if let Some(l) = self.line.get_mut(self.line_in as usize) {
            *l = val;
        }
        /* Incrementation and circular motion if necessary */
        self.line_in += 1;
        if self.line_in >= self.size {
            self.line_in -= self.size;
        }
    }

    /// Translation of `set_center_position()`: initialize : mod_rate,
    /// center_pos_mod,  and index rate.
    ///
    /// center_pos_mod is initialized so that the delay between center_pos_mod and
    /// line_in is: mod_depth + INTERP_SAMPLES_NBR.
    fn set_center_position(&mut self) {
        /* Sets the modulation rate. This rate defines how often
        the center position (center_pos_mod ) is modulated .
        The value is expressed in samples. The default value is 1 that means that
        center_pos_mod is updated at every sample.
        For example with a value of 2, the center position position will be
        updated only one time every 2 samples only.
        */
        self.mod_rate = LOW_MOD_RATE; /* default modulation rate */

        /* compensate mod rate for high modulation depth */
        if self.mod_depth > LOW_MOD_DEPTH {
            let delta_mod_depth = self.mod_depth - LOW_MOD_DEPTH;
            self.mod_rate += (delta_mod_depth * RANGE_MOD_RATE) / RANGE_MOD_DEPTH;
        }

        /* Initializes the modulated center position (center_pos_mod) so that:
            - the delay between center_pos_mod and line_in is:
              mod_depth + INTERP_SAMPLES_NBR.
        */
        let mut center = self.line_in - (INTERP_SAMPLES_NBR + self.mod_depth);

        if center < 0 {
            center += self.size;
        }

        self.center_pos_mod = center as FluidReal;

        /* index rate to control when to update center_pos_mod */
        /* Important: must be set to get center_pos_mod immediately used for the
        reading of first sample (see get_mod_delay()) */
        self.index_rate = self.mod_rate;
    }

    /// Translation of `update_parameters_from_sample_rate()`: update
    /// internal parameters dependent of sample rate.
    /// - mod_depth.
    /// - mod_rate, center_pos_mod, and index rate.
    /// - modulators frequency.
    fn update_parameters_from_sample_rate(&mut self) {
        /* initialize modulation depth (peak to peak) (in samples) */
        /* convert modulation depth in ms to sample number */
        self.mod_depth = (self.depth_ms / 1000.0 * self.sample_rate) as i32;

        /* the delay line is fixed. So we reduce mod_depth (if necessary) */
        if self.mod_depth > MAX_SAMPLES {
            fluid_log(
                FLUID_WARN,
                &format!(
                    "chorus: Too high depth. Setting it to max ({}).",
                    MAX_SAMPLES
                ),
            );
            self.mod_depth = MAX_SAMPLES;
            /* set depth_ms to maximum to avoid spamming console with above warning */
            self.depth_ms = (self.mod_depth * 1000) as FluidReal / self.sample_rate;
        }

        self.mod_depth /= 2; /* amplitude is peak to peek / 2 */

        /* Initializes the modulated center position:
        mod_rate, center_pos_mod, and index rate.
        */
        self.set_center_position(); /* must be called before set_xxxx_frequency() */

        /* initialize modulator frequency */
        for i in 0..self.number_blocks.max(0) as usize {
            set_sinus_frequency(
                &mut self.mod_[i].sinus,
                (self.speed_hz * self.mod_rate as FluidReal) as f32,
                self.sample_rate as f32,
                /* phase offset between modulators waveform */
                (360.0f32 / self.number_blocks as f32) * i as f32,
            );

            set_triangle_frequency(
                &mut self.mod_[i].triang,
                (self.speed_hz * self.mod_rate as FluidReal) as f32,
                self.sample_rate as f32,
                /* phase offset between modulators waveform */
                i as f32 / self.number_blocks as f32,
            );
        }
    }

    /// Translation of `new_mod_delay_line()`: modulated delay line
    /// initialization.
    ///
    /// Sets the length line ( alloc delay samples).
    /// Remark: the function sets the internal size according to the length delay_length.
    /// The size is augmented by INTERP_SAMPLES_NBR to take account of interpolation.
    fn new_mod_delay_line(&mut self, delay_length: i32) -> bool {
        /*-----------------------------------------------------------------------*/
        /* checks parameter */
        if delay_length < 1 {
            return false;
        }

        self.mod_depth = 0;
        /*-----------------------------------------------------------------------
         allocates delay_line and initialize members: - line, size, line_in...
        */
        /* total size of the line:  size = INTERP_SAMPLES_NBR + delay_length */
        self.size = delay_length + INTERP_SAMPLES_NBR;
        self.line = vec![0.0; self.size as usize];

        /* clears the buffer:
         - delay line
         - interpolator member: buffer, frac_pos_mod
        */
        self.reset();

        /* Initializes line_in to the start of the buffer */
        self.line_in = 0;
        /*------------------------------------------------------------------------
        Initializes modulation members:
        - modulation rate (the speed at which center_pos_mod is modulated: mod_rate
        - modulated center position: center_pos_mod
        - index rate to know when to update center_pos_mod:index_rate
        -------------------------------------------------------------------------*/
        /* Initializes the modulated center position:
        mod_rate, center_pos_mod,  and index rate
        */
        self.set_center_position();

        true
    }

    /*-----------------------------------------------------------------------------
      API
    ------------------------------------------------------------------------------*/
    /// Translation of `new_fluid_chorus()`.
    ///
    /// Create the chorus unit. Once created the chorus have no parameters set, so
    /// fluid_chorus_set() must be called at least one time after calling
    /// new_fluid_chorus().
    pub(crate) fn new(sample_rate: FluidReal) -> Option<FluidChorus> {
        let mut chorus = FluidChorus {
            type_: 0,
            depth_ms: 0.0,
            level: 0.0,
            speed_hz: 0.0,
            number_blocks: 0,
            sample_rate,
            width: 0.0,
            wet1: 0.0,
            wet2: 0.0,
            line: Vec::new(),
            size: 0,
            line_in: 0,
            center_pos_mod: 0.0,
            mod_depth: 0,
            index_rate: 0,
            mod_rate: 0,
            mod_: [Modulator::default(); MAX_CHORUS as usize],
        };

        if !chorus.new_mod_delay_line(MAX_SAMPLES) {
            return None;
        }

        Some(chorus)
    }

    /// Translation of `fluid_chorus_reset()`: clear the internal delay line
    /// and associate filter.
    pub(crate) fn reset(&mut self) {
        /* reset delay line */
        for l in self.line.iter_mut() {
            *l = 0.0;
        }

        /* reset modulators's allpass filter */
        for m in self.mod_.iter_mut() {
            /* initializes 1st order All-Pass interpolator members */
            m.buffer = 0.0; /* previous delay sample value */
            m.frac_pos_mod = 0.0; /* fractional position (between consecutives sample) */
        }
    }

    /// Translation of `fluid_chorus_set()`: set one or more chorus
    /// parameters.
    ///
    /// @param chorus Chorus instance.
    /// @param set Flags indicating which chorus parameters to set (#fluid_chorus_set_t).
    /// @param nr Chorus voice count (0-99, CPU time consumption proportional to
    ///   this value).
    /// @param level Chorus level (0.0-10.0).
    /// @param speed Chorus speed in Hz (0.1-5.0).
    /// @param depth_ms Chorus depth (max value depends on synth sample rate,
    ///   0.0-21.0 is safe for sample rate values up to 96KHz).
    /// @param type Chorus waveform type (#fluid_chorus_mod).
    pub(crate) fn set(
        &mut self,
        set: i32,
        nr: i32,
        level: FluidReal,
        speed: FluidReal,
        depth_ms: FluidReal,
        type_: i32,
    ) {
        if set & FLUID_CHORUS_SET_NR != 0 {
            /* number of block */
            self.number_blocks = nr;
        }

        if set & FLUID_CHORUS_SET_LEVEL != 0 {
            /* output level */
            self.level = level;
        }

        if set & FLUID_CHORUS_SET_SPEED != 0 {
            /* lfo frequency (in Hz) */
            self.speed_hz = speed;
        }

        if set & FLUID_CHORUS_SET_DEPTH != 0 {
            /* modulation depth (in ms) */
            self.depth_ms = depth_ms;
        }

        if set & FLUID_CHORUS_SET_TYPE != 0 {
            /* lfo shape (sinus, triangle) */
            self.type_ = type_;
        }

        /* check min , max parameters */
        if self.number_blocks < 0 {
            fluid_log(
                FLUID_WARN,
                "chorus: number blocks must be >=0! Setting value to 0.",
            );
            self.number_blocks = 0;
        } else if self.number_blocks > MAX_CHORUS {
            fluid_log(
                FLUID_WARN,
                &format!(
                    "chorus: number blocks larger than max. allowed! Setting value to {}.",
                    MAX_CHORUS
                ),
            );
            self.number_blocks = MAX_CHORUS;
        }

        if self.speed_hz < MIN_SPEED_HZ {
            fluid_log(
                FLUID_WARN,
                &format!(
                    "chorus: speed is too low (min {:.6})! Setting value to min.",
                    MIN_SPEED_HZ
                ),
            );
            self.speed_hz = MIN_SPEED_HZ;
        } else if self.speed_hz > MAX_SPEED_HZ {
            fluid_log(
                FLUID_WARN,
                &format!(
                    "chorus: speed must be below {:.6} Hz! Setting value to max.",
                    MAX_SPEED_HZ
                ),
            );
            self.speed_hz = MAX_SPEED_HZ;
        }

        if self.depth_ms < 0.0 {
            fluid_log(
                FLUID_WARN,
                "chorus: depth must be positive! Setting value to 0.",
            );
            self.depth_ms = 0.0;
        }

        if self.level < 0.0 {
            fluid_log(
                FLUID_WARN,
                "chorus: level must be positive! Setting value to 0.",
            );
            self.level = 0.0;
        } else if self.level > MAX_LEVEL {
            fluid_log(
                FLUID_WARN,
                "chorus: level must be < 10. A reasonable level is << 1! Setting it to 0.1.",
            );
            self.level = 0.1;
        }

        /* update parameters dependent of sample rate */
        self.update_parameters_from_sample_rate();

        /* Initialize the lfo waveform */
        if (self.type_ != FLUID_CHORUS_MOD_SINE) && (self.type_ != FLUID_CHORUS_MOD_TRIANGLE) {
            fluid_log(
                FLUID_WARN,
                "chorus: Unknown modulation type. Using sinewave.",
            );
            self.type_ = FLUID_CHORUS_MOD_SINE;
        }

        /* Recalculate internal values after parameters change */

        /*
         Note:
         Actually WIDTH is fixed to maximum value. But in the future we could add a setting
         "synth.chorus.width" to allow a gradually stereo effect from minimum (monophonic) to
         maximum stereo effect.
         If this setting will be added, remove the following instruction.
        */
        self.width = WIDTH;
        {
            /* The stereo amplitude equation (wet1 and wet2 below) have a
            tendency to produce high amplitude with high width values ( 1 < width < 10).
            This results in an unwanted noisy output clipped by the audio card.
            To avoid this dependency, we divide by (1 + chorus->width * SCALE_WET_WIDTH)
            Actually, with a SCALE_WET_WIDTH of 0.2, (regardless of level setting),
            the output amplitude (wet) seems rather independent of width setting */

            let mut wet = self.level * SCALE_WET as FluidReal;

            /* wet1 and wet2 are used by the stereo effect controlled by the width setting
            for producing a stereo ouptput from a monophonic chorus signal.
            Please see the note above about a side effect tendency */

            if self.number_blocks > 1 {
                wet /= 1.0 + self.width * SCALE_WET_WIDTH as FluidReal;
                self.wet1 = wet * (self.width / 2.0 + 0.5);
                self.wet2 = wet * ((1.0 - self.width) / 2.0);
            } else {
                /* only one chorus block */
                if self.width == 0.0 {
                    /* wet1 and wet2 should make stereo output monomophic */
                    self.wet1 = wet;
                    self.wet2 = wet;
                } else {
                    /* for width > 0, wet1 and wet2 should make stereo output stereo
                    with only one block. This will only possible by inverting
                    the unique signal on each left and right output.
                    Note however that with only one block, it isn't possible to
                    have a graduate width effect */
                    self.wet1 = wet;
                    self.wet2 = -wet; /* inversion */
                }
            }
        }
    }

    /// Translation of `fluid_chorus_samplerate_change()`: applies a sample
    /// rate change on the chorus.
    pub(crate) fn samplerate_change(&mut self, sample_rate: FluidReal) {
        self.sample_rate = sample_rate;

        /* update parameters dependent of sample rate */
        self.update_parameters_from_sample_rate();
    }

    /// One sample of `fluid_chorus_processmix()` and
    /// `fluid_chorus_processreplace()`: the stereo unit's output.
    #[inline]
    fn process_sample(&mut self, input: FluidReal) -> (FluidReal, FluidReal) {
        let mut d_out: [FluidReal; 2] = [0.0; 2]; /* output stereo Left and Right  */
        let mut out: FluidReal = 0.0; /* block output */
        let mut i: i32;

        self.index_rate += 1; /* modulator rate */

        /* foreach chorus block, process output sample */
        i = 0;
        while i < self.number_blocks {
            /* get sample from the output of modulated delay line */
            out = self.get_mod_delay(i as usize);

            /* accumulate out into stereo unit input */
            d_out[(i & 1) as usize] += out;
            i += 1;
        }

        /* update modulator index rate and output center position */
        if self.index_rate >= self.mod_rate {
            self.index_rate = 0; /* clear modulator index rate */

            /* updates center position (center_pos_mod) to the next position
            specified by modulation rate */
            self.center_pos_mod += self.mod_rate as FluidReal;
            if self.center_pos_mod >= self.size as FluidReal {
                self.center_pos_mod -= self.size as FluidReal;
            }
        }

        /* Adjust stereo input level in case of number_blocks odd:
        In those case, d_out[1] level is lower than d_out[0], so we need to
        add out value to d_out[1] to have d_out[0] and d_out[1] balanced.
        */
        if (i & 1) != 0 && i > 2 {
            // i = 3,5,7...
            d_out[1] += out;
        }

        /* Write the current input sample into the circular buffer.
         * Note that 'in' may be aliased with 'left_out'. Hence this must be done
         * before "processing stereo unit" (below). This ensures input buffer
         * not being overwritten by stereo unit output.
         */
        self.push_in_delay_line(input);

        /* process stereo unit */
        (
            d_out[0] * self.wet1 + d_out[1] * self.wet2,
            d_out[1] * self.wet1 + d_out[0] * self.wet2,
        )
    }

    /// Translation of `fluid_chorus_processmix()`: process chorus by mixing
    /// the result in output buffer.
    ///
    /// @param in, pointer on monophonic input buffer of FLUID_BUFSIZE samples.
    /// @param left_out, right_out, pointers on stereo output buffers of
    ///  FLUID_BUFSIZE samples.
    pub(crate) fn processmix(
        &mut self,
        in_: &[FluidReal],
        left_out: &mut [FluidReal],
        right_out: &mut [FluidReal],
    ) {
        /* foreach sample, process output sample then input sample */
        for sample_index in 0..FLUID_BUFSIZE {
            let (l, r) = self.process_sample(in_[sample_index]);
            /* Add the chorus stereo unit d_out to left and right output */
            left_out[sample_index] += l;
            right_out[sample_index] += r;
        }
    }

    /// Translation of `fluid_chorus_processreplace()`: process chorus by
    /// putting the result in output buffer (no mixing). (`in` aliases
    /// `left_out` upstream; here it is a copy of it.)
    pub(crate) fn processreplace(
        &mut self,
        in_: &[FluidReal],
        left_out: &mut [FluidReal],
        right_out: &mut [FluidReal],
    ) {
        /* foreach sample, process output sample then input sample */
        for sample_index in 0..FLUID_BUFSIZE {
            let (l, r) = self.process_sample(in_[sample_index]);
            /* store the chorus stereo unit d_out to left and right output */
            left_out[sample_index] = l;
            right_out[sample_index] = r;
        }
    }
}
