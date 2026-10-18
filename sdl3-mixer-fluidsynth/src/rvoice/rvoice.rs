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
// Modified 2026-10-17: translated into Rust from fluid_rvoice.c and
// fluid_rvoice.h in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE).

use std::sync::Arc;

use sdl3::stdlib::math::log;

use crate::fluidsynth_h::FLUID_IIR_BEANLAND;
use crate::fluidsynth_priv::{
    fluid_clip, RvoiceParam, RvoicePtr, FLUID_BUFSIZE, FLUID_M_LN10, MAX_EVENT_PARAMS,
};
use crate::rvoice::adsr_env::*;
use crate::rvoice::iir_filter::{
    fluid_iir_filter_apply, fluid_iir_filter_calc, fluid_iir_filter_reset, FluidIirFilter,
};
use crate::rvoice::lfo::{fluid_lfo_calc, fluid_lfo_get_val, fluid_lfo_reset, FluidLfo};
use crate::rvoice::phase::{fluid_phase_index, fluid_phase_set_int, FluidPhase};
use crate::rvoice::rvoice_dsp::{fluid_rvoice_dsp_interpolate, fluid_rvoice_dsp_silence};
use crate::sfloader::sfont::Sample;
use crate::utils::conv::{fluid_cb2amp, fluid_convex, fluid_ct2hz_real};
use crate::utils::conv_tables::FLUID_PEAK_ATTENUATION;
use crate::FluidReal;

/* Smallest amplitude that can be perceived (full scale is +/- 0.5)
 * 16 bits => 96+4=100 dB dynamic range => 0.00001
 * 24 bits => 144-4 = 140 dB dynamic range => 1.e-7
 * 1.e-7 * 2 == 2.e-7 :)
 */
pub(crate) const FLUID_NOISE_FLOOR: FluidReal = 2.0e-7;

// enum fluid_loop
pub(crate) const FLUID_UNLOOPED: i32 = 0;
pub(crate) const FLUID_LOOP_DURING_RELEASE: i32 = 1;
pub(crate) const FLUID_START_ON_RELEASE: i32 = 2; /* this is a looping mode introduced by Polyphone, see #1398 for more info */
pub(crate) const FLUID_LOOP_UNTIL_RELEASE: i32 = 3;

/// Translation of `fluid_rvoice_envlfo_t`: rvoice ticks-based parameters.
/// These parameters must be updated even if the voice is currently quiet.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct FluidRvoiceEnvlfo {
    /* Note-off minimum length */
    pub ticks: u32,
    pub noteoff_ticks: u32,

    /* vol env */
    pub volenv: FluidAdsrEnv,

    /* mod env */
    pub modenv: FluidAdsrEnv,
    pub modenv_to_fc: FluidReal,
    pub modenv_to_pitch: FluidReal,

    /* mod lfo */
    pub modlfo: FluidLfo,
    pub modlfo_to_fc: FluidReal,
    pub modlfo_to_pitch: FluidReal,
    pub modlfo_to_vol: FluidReal,

    /* vib lfo */
    pub viblfo: FluidLfo,
    pub viblfo_to_pitch: FluidReal,
}

/// Translation of `fluid_rvoice_dsp_t`: rvoice parameters needed for dsp
/// interpolation.
#[derive(Clone, Default)]
pub(crate) struct FluidRvoiceDsp {
    /* interpolation method, as in fluid_interp in fluidsynth.h */
    pub interp_method: i32,
    pub samplemode: i32,

    /* Flag that is set as soon as the first loop is completed. */
    pub has_looped: bool,

    /* Flag that initiates, that sample-related parameters have to be checked. */
    pub check_sample_sanity_flag: u8,

    pub sample: Option<Arc<Sample>>,

    /* sample and loop start and end points (offset in sample memory).  */
    pub start: i32,
    pub end: i32,
    pub loopstart: i32,
    pub loopend: i32, /* Note: first point following the loop (superimposed on loopstart) */

    /* Stuff needed for portamento calculations */
    pub pitchoffset: FluidReal, /* the portamento range in midicents */
    pub pitchinc: FluidReal,    /* the portamento increment in midicents */

    /* Stuff needed for phase calculations */
    pub pitch: FluidReal,         /* the pitch in midicents */
    pub root_pitch_hz: FluidReal, /* the base note of the note in hz */
    pub output_rate: FluidReal,

    /* Stuff needed for amplitude calculations */
    pub attenuation: FluidReal, /* the attenuation in centibels */
    pub prev_attenuation: FluidReal, /* the previous attenuation in centibels
                                used by fluid_rvoice_multi_retrigger_attack() */
    pub min_attenuation_cb: FluidReal, /* Estimate on the smallest possible attenuation
                                        * during the lifetime of the voice */
    pub amplitude_that_reaches_noise_floor_nonloop: FluidReal,
    pub amplitude_that_reaches_noise_floor_loop: FluidReal,
    pub synth_gain: FluidReal, /* master gain */

    /* Dynamic input to the interpolator below */
    pub phase: FluidPhase, /* the phase (current sample offset) of the sample wave */
    pub phase_incr: FluidReal, /* the phase increment for the next FLUID_BUFSIZE samples */
}

/* Currently left, right, reverb, chorus. To be changed if we
ever add surround positioning, or stereo reverb/chorus */
pub(crate) const FLUID_RVOICE_MAX_BUFS: usize = 4;

/// One record of `fluid_rvoice_buffers_t::bufs`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct FluidRvoiceBuf {
    /* the actual, linearly interpolated amplitude with which the dsp sample should be mixed into the buf */
    pub current_amp: FluidReal,

    /* the desired amplitude [...] mixed into the buf (directly set by e.g. rapidly changing PAN events) */
    pub target_amp: FluidReal,

    /* Mapping to mixdown buffer index */
    pub mapping: i32,
}

/// Translation of `fluid_rvoice_buffers_t`: rvoice mixer-related
/// parameters.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct FluidRvoiceBuffers {
    pub count: u32, /* Number of records in "bufs" */
    pub bufs: [FluidRvoiceBuf; FLUID_RVOICE_MAX_BUFS],
}

/// Translation of `fluid_rvoice_t`: hard realtime parameters needed to
/// synthesize a voice.
#[derive(Clone, Default)]
pub(crate) struct FluidRvoice {
    pub envlfo: FluidRvoiceEnvlfo,
    pub dsp: FluidRvoiceDsp,
    pub resonant_filter: FluidIirFilter, /* IIR resonant dsp filter */
    pub resonant_custom_filter: FluidIirFilter, /* optional custom/general-purpose IIR resonant filter */
    pub buffers: FluidRvoiceBuffers,
}

/// Translation of `fluid_rvoice_get_sample24()`.
///
/// Combines the most significant 16 bit part of a sample with a potentially present
/// least sig. 8 bit part in order to create a 24 bit sample.
#[inline]
pub(crate) fn fluid_rvoice_get_sample24(dsp_msb: &[i16], dsp_lsb: &[u8], idx: u32) -> i32 {
    /* cast sample to unsigned type, so we can safely shift and bitwise or
     * without relying on undefined behaviour (should never happen anyway ofc...) */
    // Note (upstream): nothing keeps the index inside the sample data;
    // past it, this reads 0.
    let msb = dsp_msb.get(idx as usize).copied().unwrap_or(0) as i32 as u32;
    let lsb = dsp_lsb.get(idx as usize).copied().unwrap_or(0);
    ((msb << 8) | lsb as u32) as i32
}

/// Translation of `fluid_rvoice_get_sample16()`.
#[inline]
pub(crate) fn fluid_rvoice_get_sample16(dsp_msb: &[i16], idx: u32) -> i32 {
    /* cast sample to unsigned type, so we can safely shift and bitwise or
     * without relying on undefined behaviour (should never happen anyway ofc...) */
    // Note (upstream): nothing keeps the index inside the sample data;
    // past it, this reads 0.
    let msb = dsp_msb.get(idx as usize).copied().unwrap_or(0) as i32 as u32;
    ((msb << 8) | 0) as i32
}

/// Translation of `fluid_rvoice_get_sample()`.
#[inline]
pub(crate) fn fluid_rvoice_get_sample(dsp_msb: &[i16], dsp_lsb: Option<&[u8]>, idx: u32) -> i32 {
    if let Some(dsp_lsb) = dsp_lsb {
        fluid_rvoice_get_sample24(dsp_msb, dsp_lsb, idx)
    } else {
        fluid_rvoice_get_sample16(dsp_msb, idx)
    }
}

/// Translation of `fluid_rvoice_calc_amp()`.
///
/// @return -1 if voice is quiet, 0 if voice has finished, 1 otherwise
#[inline]
fn fluid_rvoice_calc_amp(voice: &mut FluidRvoice) -> i32 {
    let target_amp: FluidReal; /* target amplitude */

    if fluid_adsr_env_get_section(&voice.envlfo.volenv) == FLUID_VOICE_ENVDELAY {
        return -1; /* The volume amplitude is in hold phase. No sound is produced. */
    }

    if fluid_adsr_env_get_section(&voice.envlfo.volenv) == FLUID_VOICE_ENVATTACK {
        /* the envelope is in the attack section: ramp linearly to max value.
         * A positive modlfo_to_vol should increase volume (negative attenuation).
         */
        target_amp = fluid_cb2amp(voice.dsp.attenuation)
            * fluid_cb2amp(fluid_lfo_get_val(&voice.envlfo.modlfo) * -voice.envlfo.modlfo_to_vol)
            * fluid_adsr_env_get_val(&voice.envlfo.volenv);
    } else {
        let amplitude_that_reaches_noise_floor: FluidReal;
        let amp_max: FluidReal;

        target_amp = fluid_cb2amp(voice.dsp.attenuation)
            * fluid_cb2amp(
                FLUID_PEAK_ATTENUATION as FluidReal
                    * (1.0 - fluid_adsr_env_get_val(&voice.envlfo.volenv))
                    + fluid_lfo_get_val(&voice.envlfo.modlfo) * -voice.envlfo.modlfo_to_vol,
            );

        /* We turn off a voice, if the volume has dropped low enough. */

        /* A voice can be turned off, when an estimate for the volume
         * (upper bound) falls below that volume, that will drop the
         * sample below the noise floor.
         */

        /* If the loop amplitude is known, we can use it if the voice loop is within
         * the sample loop
         */

        /* Is the playing pointer already in the loop? */
        if voice.dsp.has_looped {
            amplitude_that_reaches_noise_floor = voice.dsp.amplitude_that_reaches_noise_floor_loop;
        } else {
            amplitude_that_reaches_noise_floor =
                voice.dsp.amplitude_that_reaches_noise_floor_nonloop;
        }

        /* voice->attenuation_min is a lower boundary for the attenuation
         * now and in the future (possibly 0 in the worst case).  Now the
         * amplitude of sample and volenv cannot exceed amp_max (since
         * volenv_val can only drop):
         */

        amp_max = fluid_cb2amp(voice.dsp.min_attenuation_cb)
            * fluid_adsr_env_get_val(&voice.envlfo.volenv);

        /* And if amp_max is already smaller than the known amplitude,
         * which will attenuate the sample below the noise floor, then we
         * can safely turn off the voice. Duh. */
        if amp_max < amplitude_that_reaches_noise_floor {
            return 0;
        }
    }

    /* Volume increment to go from voice->amp to target_amp in FLUID_BUFSIZE steps */
    voice.resonant_filter.amp_incr =
        (target_amp - voice.resonant_filter.amp) / FLUID_BUFSIZE as FluidReal;

    /* no volume and not changing? - No need to process */
    if (voice.resonant_filter.amp == 0.0) && (voice.resonant_filter.amp_incr == 0.0) {
        return -1;
    }

    1
}

/* these should be the absolute minimum that FluidSynth can deal with */
const FLUID_MIN_LOOP_SIZE: i32 = 2;
const FLUID_MIN_LOOP_PAD: i32 = 0;

const FLUID_SAMPLESANITY_CHECK: u8 = 1 << 0;
const FLUID_SAMPLESANITY_STARTUP: u8 = 1 << 1;

/// Translation of `fluid_rvoice_check_sample_sanity()`.
///
/// Make sure, that sample start / end point and loop points are in
/// proper order. When starting up, calculate the initial phase.
/// TODO: Investigate whether this can be moved from rvoice to voice.
fn fluid_rvoice_check_sample_sanity(voice: &mut FluidRvoice) {
    let Some(sample) = voice.dsp.sample.clone() else {
        return;
    };
    let min_index_nonloop = sample.start as i32;
    let max_index_nonloop = sample.end as i32;

    /* make sure we have enough samples surrounding the loop */
    let min_index_loop = sample.start as i32 + FLUID_MIN_LOOP_PAD;
    let max_index_loop = (sample.end as i32 - FLUID_MIN_LOOP_PAD).wrapping_add(1); /* 'end' is last valid sample, loopend can be + 1 */

    /* Keep the start point within the sample data */
    if voice.dsp.start < min_index_nonloop {
        voice.dsp.start = min_index_nonloop;
    } else if voice.dsp.start > max_index_nonloop {
        voice.dsp.start = max_index_nonloop;
    }

    /* Keep the end point within the sample data */
    if voice.dsp.end < min_index_nonloop {
        voice.dsp.end = min_index_nonloop;
    } else if voice.dsp.end > max_index_nonloop {
        voice.dsp.end = max_index_nonloop;
    }

    /* Keep start and end point in the right order */
    if voice.dsp.start > voice.dsp.end {
        std::mem::swap(&mut voice.dsp.start, &mut voice.dsp.end);
        /*FLUID_LOG(FLUID_DBG, "Loop / sample sanity check: Changing order of start / end points!"); */
    }

    /* Zero length? */
    if voice.dsp.start == voice.dsp.end {
        fluid_rvoice_voiceoff(voice, &Default::default());
        return;
    }

    if (voice.dsp.samplemode == FLUID_LOOP_UNTIL_RELEASE)
        || (voice.dsp.samplemode == FLUID_LOOP_DURING_RELEASE)
    {
        /* Keep the loop start point within the sample data */
        if voice.dsp.loopstart < min_index_loop {
            voice.dsp.loopstart = min_index_loop;
        } else if voice.dsp.loopstart > max_index_loop {
            voice.dsp.loopstart = max_index_loop;
        }

        /* Keep the loop end point within the sample data */
        if voice.dsp.loopend < min_index_loop {
            voice.dsp.loopend = min_index_loop;
        } else if voice.dsp.loopend > max_index_loop {
            voice.dsp.loopend = max_index_loop;
        }

        /* Keep loop start and end point in the right order */
        if voice.dsp.loopstart > voice.dsp.loopend {
            std::mem::swap(&mut voice.dsp.loopstart, &mut voice.dsp.loopend);
            /*FLUID_LOG(FLUID_DBG, "Loop / sample sanity check: Changing order of loop points!"); */
        }

        /* Loop too short? Then don't loop. */
        if voice.dsp.loopend < voice.dsp.loopstart.wrapping_add(FLUID_MIN_LOOP_SIZE) {
            voice.dsp.samplemode = FLUID_UNLOOPED;
        }

        /* The loop points may have changed. Obtain a new estimate for the loop volume. */
        /* Is the voice loop within the sample loop? */
        if voice.dsp.loopstart >= sample.loopstart as i32
            && voice.dsp.loopend <= sample.loopend as i32
        {
            /* Is there a valid peak amplitude available for the loop, and can we use it? */
            if sample.amplitude_that_reaches_noise_floor_is_valid
                && voice.dsp.samplemode == FLUID_LOOP_DURING_RELEASE
            {
                voice.dsp.amplitude_that_reaches_noise_floor_loop =
                    sample.amplitude_that_reaches_noise_floor / voice.dsp.synth_gain;
            } else {
                /* Worst case */
                voice.dsp.amplitude_that_reaches_noise_floor_loop =
                    voice.dsp.amplitude_that_reaches_noise_floor_nonloop;
            };
        };
    } /* if sample mode is looped */

    /* Run startup specific code (only once, when the voice is started) */
    if voice.dsp.check_sample_sanity_flag & FLUID_SAMPLESANITY_STARTUP != 0 {
        if max_index_loop.wrapping_sub(min_index_loop) < FLUID_MIN_LOOP_SIZE {
            if (voice.dsp.samplemode == FLUID_LOOP_UNTIL_RELEASE)
                || (voice.dsp.samplemode == FLUID_LOOP_DURING_RELEASE)
            {
                voice.dsp.samplemode = FLUID_UNLOOPED;
            }
        }

        /* Set the initial phase of the voice (using the result from the
        start offset modulators). */
        voice.dsp.phase = fluid_phase_set_int(voice.dsp.start);
    } /* if startup */

    /* Is this voice run in loop mode, or does it run straight to the
    end of the waveform data? */
    if ((voice.dsp.samplemode == FLUID_LOOP_UNTIL_RELEASE)
        && (fluid_adsr_env_get_section(&voice.envlfo.volenv) < FLUID_VOICE_ENVRELEASE))
        || (voice.dsp.samplemode == FLUID_LOOP_DURING_RELEASE)
    {
        /* Yes, it will loop as soon as it reaches the loop point.  In
         * this case we must prevent, that the playback pointer (phase)
         * happens to end up beyond the 2nd loop point, because the
         * point has moved.  The DSP algorithm is unable to cope with
         * that situation.  So if the phase is beyond the 2nd loop
         * point, set it to the start of the loop. No way to avoid some
         * noise here.  Note: If the sample pointer ends up -before the
         * first loop point- instead, then the DSP loop will just play
         * the sample, enter the loop and proceed as expected => no
         * actions required.
         */
        let index_in_sample = fluid_phase_index(voice.dsp.phase) as i32;

        if index_in_sample >= voice.dsp.loopend {
            /* FLUID_LOG(FLUID_DBG, "Loop / sample sanity check: Phase after 2nd loop point!"); */
            voice.dsp.phase = fluid_phase_set_int(voice.dsp.loopstart);
        }
    }

    /*    FLUID_LOG(FLUID_DBG, "Loop / sample sanity check: Sample from %i to %i, loop from %i to %i", voice->dsp.start, voice->dsp.end, voice->dsp.loopstart, voice->dsp.loopend); */

    /* Sample sanity has been assured. Don't check again, until some
    sample parameter is changed by modulation. */
    voice.dsp.check_sample_sanity_flag = 0;
}

/// Translation of `fluid_rvoice_write()`: synthesize a voice to a buffer.
///
/// @param voice rvoice to synthesize
/// @param dsp_buf Audio buffer to synthesize to (#FLUID_BUFSIZE in length)
/// @return Count of samples written to dsp_buf. (-1 means voice is currently
/// quiet, 0 .. #FLUID_BUFSIZE-1 means voice finished.)
///
/// Panning, reverb and chorus are processed separately. The dsp interpolation
/// routine is in (fluid_rvoice_dsp.c).
pub(crate) fn fluid_rvoice_write(voice: &mut FluidRvoice, dsp_buf: &mut [FluidReal]) -> i32 {
    let ticks = voice.envlfo.ticks as i32;
    let mut count: i32;
    let modenv_val: FluidReal;
    let mut fmod: FluidReal;

    /******************* sample sanity check **********/

    let Some(sample) = voice.dsp.sample.clone() else {
        return 0;
    };

    if voice.dsp.check_sample_sanity_flag != 0 {
        fluid_rvoice_check_sample_sanity(voice);
    }

    /******************* noteoff check ****************/

    if voice.envlfo.noteoff_ticks != 0 && voice.envlfo.ticks >= voice.envlfo.noteoff_ticks {
        fluid_rvoice_noteoff_local(voice, 0);
    }

    voice.envlfo.ticks = voice.envlfo.ticks.wrapping_add(FLUID_BUFSIZE as u32);

    /******************* vol env **********************/

    fluid_adsr_env_calc(&mut voice.envlfo.volenv);

    if fluid_adsr_env_get_section(&voice.envlfo.volenv) == FLUID_VOICE_ENVFINISHED {
        return 0;
    }

    /******************* mod env **********************/

    fluid_adsr_env_calc(&mut voice.envlfo.modenv);

    /******************* lfo **********************/

    fluid_lfo_calc(&mut voice.envlfo.modlfo, ticks as u32);
    fluid_lfo_calc(&mut voice.envlfo.viblfo, ticks as u32);

    /******************* amplitude **********************/

    count = fluid_rvoice_calc_amp(voice);
    if count == 0 {
        // Voice has finished, remove from dsp loop
        return 0;
    }
    // else if count is negative, still process the voice

    /******************* phase **********************/

    /* SF2.04 section 8.1.2 #26:
     * attack of modEnv is convex ?!?
     */
    modenv_val = if fluid_adsr_env_get_section(&voice.envlfo.modenv) == FLUID_VOICE_ENVATTACK {
        fluid_convex(127.0 * fluid_adsr_env_get_val(&voice.envlfo.modenv))
    } else {
        fluid_adsr_env_get_val(&voice.envlfo.modenv)
    };
    /* Calculate the number of samples, that the DSP loop advances
     * through the original waveform with each step in the output
     * buffer. It is the ratio between the frequencies of original
     * waveform and output waveform.*/
    voice.dsp.phase_incr = fluid_ct2hz_real(
        voice.dsp.pitch
            + voice.dsp.pitchoffset
            + fluid_lfo_get_val(&voice.envlfo.modlfo) * voice.envlfo.modlfo_to_pitch
            + fluid_lfo_get_val(&voice.envlfo.viblfo) * voice.envlfo.viblfo_to_pitch
            + modenv_val * voice.envlfo.modenv_to_pitch,
    ) / voice.dsp.root_pitch_hz;

    /******************* portamento ****************/
    /* pitchoffset is updated if enabled.
    Pitchoffset will be added to dsp pitch at next phase calculation time */

    /* In most cases portamento will be disabled. Thus first verify that portamento is
     * enabled before updating pitchoffset and before disabling portamento when necessary,
     * in order to keep the performance loss at minimum.
     * If the algorithm would first update pitchoffset and then verify if portamento
     * needs to be disabled, there would be a significant performance drop on a x87 FPU
     */
    if voice.dsp.pitchinc > 0.0 {
        /* portamento is enabled, so update pitchoffset */
        voice.dsp.pitchoffset += voice.dsp.pitchinc;

        /* when pitchoffset reaches 0.0f, portamento is disabled */
        if voice.dsp.pitchoffset > 0.0 {
            voice.dsp.pitchinc = 0.0;
            voice.dsp.pitchoffset = 0.0;
        }
    } else if voice.dsp.pitchinc < 0.0 {
        /* portamento is enabled, so update pitchoffset */
        voice.dsp.pitchoffset += voice.dsp.pitchinc;

        /* when pitchoffset reaches 0.0f, portamento is disabled */
        if voice.dsp.pitchoffset < 0.0 {
            voice.dsp.pitchinc = 0.0;
            voice.dsp.pitchoffset = 0.0;
        }
    }

    /* if phase_incr is not advancing, set it to the minimum fraction value (prevent stuckage) */
    if voice.dsp.phase_incr == 0.0 {
        voice.dsp.phase_incr = 1.0;
    }

    /* loop mode release? if not in release, the voice is silent
     * note: this intentionally processes the volenv before returning silence,
     * since that's what polyphone does (PR #1400) */
    if voice.dsp.samplemode == FLUID_START_ON_RELEASE
        && fluid_adsr_env_get_section(&voice.envlfo.volenv) < FLUID_VOICE_ENVRELEASE
    {
        return -1;
    }

    /* voice is currently looping? */
    let is_looping = voice.dsp.samplemode == FLUID_LOOP_DURING_RELEASE
        || (voice.dsp.samplemode == FLUID_LOOP_UNTIL_RELEASE
            && fluid_adsr_env_get_section(&voice.envlfo.volenv) < FLUID_VOICE_ENVRELEASE);

    /*************** resonant filter ******************/
    // Only "prepare" the filter here, the filter itself will be applied in the dsp_interpolation routines below.
    // This is to satisfy SF2 Section 9.1.8, particularly, the filtered output must be gain-adjusted by the volEnv.
    // Applying the filter after applying the gain from the volEnv might cause audible clicks for when turning off
    // voices that are filtered by a high Q, see https://github.com/FluidSynth/fluidsynth/issues/1427
    //
    // Note that at this point we are using voice->dsp.output_rate which is set to the synth's output rate, because
    // the filter will receive the interpolated waveform.

    fmod = fluid_lfo_get_val(&voice.envlfo.modlfo) * voice.envlfo.modlfo_to_fc
        + modenv_val * voice.envlfo.modenv_to_fc;
    fluid_iir_filter_calc(&mut voice.resonant_filter, voice.dsp.output_rate, fmod);

    /* additional custom filter */
    fmod = if voice.resonant_custom_filter.flags & FLUID_IIR_BEANLAND != 0 {
        (voice.dsp.pitch + voice.dsp.pitchoffset)
            - (sample.origpitch * 100 + sample.pitchadj) as FluidReal
    } else {
        0.0
    };
    fluid_iir_filter_calc(
        &mut voice.resonant_custom_filter,
        voice.dsp.output_rate,
        fmod,
    );

    /*********************** run the dsp chain ************************
     * The sample is mixed with the output buffer.
     * The buffer has to be filled from 0 to FLUID_BUFSIZE-1.
     * Depending on the position in the loop and the loop size, this
     * may require several runs. */

    if count < 0 {
        // The voice is quite, i.e. either in delay phase or zero volume.
        // We need to update the rvoice's dsp phase, as the delay phase shall not "postpone" the sound, rather
        // it should be played silently, see https://github.com/FluidSynth/fluidsynth/issues/1312
        return fluid_rvoice_dsp_silence(voice, dsp_buf, is_looping);
    }

    count = fluid_rvoice_dsp_interpolate(voice, dsp_buf, is_looping);

    if count == 0 {
        // voice has finished
        return count;
    }

    fluid_iir_filter_apply(
        &mut voice.resonant_filter,
        &mut voice.resonant_custom_filter,
        dsp_buf,
        count as usize,
    );

    count
}

/// Translation of `fluid_rvoice_buffers_check_bufnum()`: initialize
/// buffers up to (and including) bufnum.
fn fluid_rvoice_buffers_check_bufnum(buffers: &mut FluidRvoiceBuffers, bufnum: u32) -> bool {
    if bufnum < buffers.count {
        return true;
    }

    if bufnum >= FLUID_RVOICE_MAX_BUFS as u32 {
        return false;
    }

    for i in buffers.count..=bufnum {
        buffers.bufs[i as usize].target_amp = 0.0;
        buffers.bufs[i as usize].current_amp = 0.0;
    }

    buffers.count = bufnum + 1;
    true
}

/// Translation of `fluid_rvoice_buffers_set_amp()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_buffers_set_amp(
    buffers: &mut FluidRvoiceBuffers,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    let bufnum = param[0].i as u32;
    let value = param[1].real;

    if !fluid_rvoice_buffers_check_bufnum(buffers, bufnum) {
        return;
    }

    buffers.bufs[bufnum as usize].target_amp = value;
}

/// Translation of `fluid_rvoice_buffers_set_mapping()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_buffers_set_mapping(
    buffers: &mut FluidRvoiceBuffers,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    let bufnum = param[0].i as u32;
    let mapping = param[1].i;

    if !fluid_rvoice_buffers_check_bufnum(buffers, bufnum) {
        return;
    }

    buffers.bufs[bufnum as usize].mapping = mapping;
}

/// Translation of `fluid_rvoice_reset()` (an rvoice event function).
pub(crate) fn fluid_rvoice_reset(
    voice: &mut FluidRvoice,
    _param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    voice.dsp.has_looped = false;
    voice.envlfo.ticks = 0;
    voice.envlfo.noteoff_ticks = 0;

    /* legato initialization */
    voice.dsp.pitchoffset = 0.0; /* portamento initialization */
    voice.dsp.pitchinc = 0.0;

    /* mod env initialization*/
    fluid_adsr_env_reset(&mut voice.envlfo.modenv);

    /* vol env initialization */
    fluid_adsr_env_reset(&mut voice.envlfo.volenv);

    /* Fixme: Retrieve from any other existing
    voice on this channel to keep LFOs in
    unison? */
    fluid_lfo_reset(&mut voice.envlfo.viblfo);
    fluid_lfo_reset(&mut voice.envlfo.modlfo);

    /* Clear sample history in filter */
    fluid_iir_filter_reset(&mut voice.resonant_filter);
    fluid_iir_filter_reset(&mut voice.resonant_custom_filter);

    /* Force setting of the phase at the first DSP loop run
     * This cannot be done earlier, because it depends on modulators.
    [DH] Is that comment really true? */
    voice.dsp.check_sample_sanity_flag |= FLUID_SAMPLESANITY_STARTUP;
}

/// Translation of `fluid_rvoice_noteoff()` (an rvoice event function).
pub(crate) fn fluid_rvoice_noteoff(
    rvoice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    let min_ticks = param[0].i as u32;

    fluid_rvoice_noteoff_local(rvoice, min_ticks);
}

/// Translation of `fluid_rvoice_noteoff_LOCAL()`.
fn fluid_rvoice_noteoff_local(voice: &mut FluidRvoice, min_ticks: u32) {
    if min_ticks > voice.envlfo.ticks {
        /* Delay noteoff */
        voice.envlfo.noteoff_ticks = min_ticks;
        return;
    }

    voice.envlfo.noteoff_ticks = 0;

    if fluid_adsr_env_get_section(&voice.envlfo.volenv) == FLUID_VOICE_ENVATTACK {
        /* A voice is turned off during the attack section of the volume
         * envelope.  The attack section ramps up linearly with
         * amplitude. The other sections use logarithmic scaling. Calculate new
         * volenv_val to achieve equivalent amplitude during the release phase
         * for seamless volume transition.
         */
        if fluid_adsr_env_get_val(&voice.envlfo.volenv) > 0.0 {
            let lfo = fluid_lfo_get_val(&voice.envlfo.modlfo) * -voice.envlfo.modlfo_to_vol;
            let amp = fluid_adsr_env_get_val(&voice.envlfo.volenv) * fluid_cb2amp(lfo);
            let mut env_value = -(((-200.0 / FLUID_M_LN10) * log(amp) - lfo)
                / FLUID_PEAK_ATTENUATION as FluidReal
                - 1.0);
            env_value = fluid_clip(env_value, 0.0, 1.0);
            fluid_adsr_env_set_val(&mut voice.envlfo.volenv, env_value);
        }
    }

    if fluid_adsr_env_get_section(&voice.envlfo.modenv) == FLUID_VOICE_ENVATTACK {
        /* A voice is turned off during the attack section of the modulation
         * envelope. The attack section use convex scaling with pitch and filter
         * frequency cutoff (see fluid_rvoice_write(): modenv_val = fluid_convex(127 * modenv.val)
         * The other sections use linear scaling: modenv_val = modenv.val
         *
         * Calculate new modenv.val to achieve equivalent modenv_val during the release phase
         * for seamless pitch and filter frequency cutoff transition.
         */
        if fluid_adsr_env_get_val(&voice.envlfo.modenv) > 0.0 {
            let mut env_value = fluid_convex(127.0 * fluid_adsr_env_get_val(&voice.envlfo.modenv));
            env_value = fluid_clip(env_value, 0.0, 1.0);
            fluid_adsr_env_set_val(&mut voice.envlfo.modenv, env_value);
        }
    }

    fluid_adsr_env_set_section(&mut voice.envlfo.volenv, FLUID_VOICE_ENVRELEASE);
    fluid_adsr_env_set_section(&mut voice.envlfo.modenv, FLUID_VOICE_ENVRELEASE);
}

/// Translation of `fluid_rvoice_local_retrigger_attack()`.
///
/// skips to Attack section
///
/// Updates vol and attack data
/// Correction on volume val to achieve equivalent amplitude at noteOn legato
#[inline]
fn fluid_rvoice_local_retrigger_attack(voice: &mut FluidRvoice) {
    /* skips to Attack section */
    /* Once in Attack section, current count must be reset, to be sure
    that the section will be not be prematurely finished. */
    fluid_adsr_env_set_section(&mut voice.envlfo.volenv, FLUID_VOICE_ENVATTACK);
    {
        /* Correction on volume val to achieve equivalent amplitude at noteOn legato */
        let peak = fluid_cb2amp(voice.dsp.attenuation);
        let prev_peak = fluid_cb2amp(voice.dsp.prev_attenuation);
        voice.envlfo.volenv.val = (voice.envlfo.volenv.val * prev_peak) / peak;
        /* Correction on slope direction for Attack section */
        let val = voice.envlfo.volenv.val;
        let env_data = &mut voice.envlfo.volenv.data[FLUID_VOICE_ENVATTACK as usize];

        if val <= 1.0 {
            /* slope attack for legato note needs to be positive from val  up to 1 */
            env_data.increment = (1.0f32 / env_data.count as f32) as FluidReal;
            env_data.min = -1.0;
            env_data.max = 1.0;
        } else {
            /* slope attack for legato note needs to be negative: from val  down to 1 */
            env_data.increment = -val / env_data.count as FluidReal;
            env_data.min = 1.0;
            env_data.max = val;
        }
    }
}

/// Translation of `fluid_rvoice_multi_retrigger_attack()` (an rvoice event
/// function).
///
/// Used by legato Mode : multi_retrigger
///  see fluid_synth_noteon_mono_legato_multi_retrigger()
pub(crate) fn fluid_rvoice_multi_retrigger_attack(
    voice: &mut FluidRvoice,
    _param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    /*-------------------------------------------------------------------------
     Section skip for volume envelope
    --------------------------------------------------------------------------*/
    let mut section = fluid_adsr_env_get_section(&voice.envlfo.volenv); /* volume or modulation section */
    if section >= FLUID_VOICE_ENVHOLD {
        /* DECAY, SUSTAIN,RELEASE section use logarithmic scaling. Calculates new
        volenv_val to achieve equivalent amplitude during the attack phase
        for seamless volume transition. */
        let amp_cb = FLUID_PEAK_ATTENUATION as FluidReal
            * (1.0 - fluid_adsr_env_get_val(&voice.envlfo.volenv));
        let mut env_value = fluid_cb2amp(amp_cb); /* a bit of optimization */
        env_value = fluid_clip(env_value, 0.0, 1.0);
        fluid_adsr_env_set_val(&mut voice.envlfo.volenv, env_value);
        /* next, skips to Attack section */
    }

    /* skips to Attack section from any section */
    /* Update vol and  attack data */
    fluid_rvoice_local_retrigger_attack(voice);

    /*-------------------------------------------------------------------------
     Section skip for modulation envelope
    --------------------------------------------------------------------------*/
    section = fluid_adsr_env_get_section(&voice.envlfo.modenv);
    if section >= FLUID_VOICE_ENVHOLD {
        /* DECAY, SUSTAIN,RELEASE section use linear scaling.
        Since v 2.1 , as recommended by soundfont 2.01/2.4 spec, ATTACK section
        uses convex shape (see fluid_rvoice_write() - fluid_convex()).
        Calculate new modenv value (new_value) for seamless attack transition.
        Here we need the inverse of fluid_convex() function defined as:
        new_value = pow(10, (1 - current_val) . FLUID_PEAK_ATTENUATION / -200 . 2.0)
        For performance reason we use fluid_cb2amp(Val) = pow(10, val/-200) with
        val = (1 - current_val) . FLUID_PEAK_ATTENUATION / 2.0
        */
        let mut new_value = fluid_cb2amp(
            (1.0 - fluid_adsr_env_get_val(&voice.envlfo.modenv))
                * FLUID_PEAK_ATTENUATION as FluidReal
                / 2.0,
        ); /* new modenv value */
        new_value = fluid_clip(new_value, 0.0, 1.0);
        fluid_adsr_env_set_val(&mut voice.envlfo.modenv, new_value);
    }
    /* Skips from any section to ATTACK section */
    fluid_adsr_env_set_section(&mut voice.envlfo.modenv, FLUID_VOICE_ENVATTACK);
}

/// Translation of `fluid_rvoice_set_portamento()` (an rvoice event
/// function).
///
/// sets the portamento dsp parameters: dsp.pitchoffset, dsp.pitchinc
/// @param voice rvoice to set portamento.
/// @param countinc increment count number.
/// @param pitchoffset pitch offset to apply to voice dsp.pitch.
///
/// Notes:
/// 1) To get continuous portamento between consecutive noteOn (n1,n2,n3...),
///    pitchoffset is accumulated in current dsp pitchoffset.
/// 2) And to get constant portamento duration, dsp pitch increment is updated.
pub(crate) fn fluid_rvoice_set_portamento(
    voice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    let countinc = param[0].i as u32;
    let pitchoffset = param[1].real;

    if countinc != 0 {
        voice.dsp.pitchoffset += pitchoffset;
        voice.dsp.pitchinc = -voice.dsp.pitchoffset / countinc as FluidReal;
    }

    /* Then during the voice processing (in fluid_rvoice_write()),
    dsp.pitchoffset will be incremented by dsp pitchinc. */
}

/// Translation of `fluid_rvoice_set_output_rate()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_set_output_rate(
    voice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    voice.dsp.output_rate = param[0].real;
}

/// Translation of `fluid_rvoice_set_interp_method()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_set_interp_method(
    voice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    voice.dsp.interp_method = param[0].i;
}

/// Translation of `fluid_rvoice_set_root_pitch_hz()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_set_root_pitch_hz(
    voice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    voice.dsp.root_pitch_hz = param[0].real;
}

/// Translation of `fluid_rvoice_set_pitch()` (an rvoice event function).
pub(crate) fn fluid_rvoice_set_pitch(
    voice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    voice.dsp.pitch = param[0].real;
}

/// Translation of `fluid_rvoice_set_attenuation()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_set_attenuation(
    voice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    let value = param[0].real;

    voice.dsp.prev_attenuation = voice.dsp.attenuation;
    voice.dsp.attenuation = value;
}

/// Translation of `fluid_rvoice_set_min_attenuation_cB()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_set_min_attenuation_cb(
    voice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    voice.dsp.min_attenuation_cb = param[0].real;
}

/// Translation of `fluid_rvoice_set_viblfo_to_pitch()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_set_viblfo_to_pitch(
    voice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    voice.envlfo.viblfo_to_pitch = param[0].real;
}

/// Translation of `fluid_rvoice_set_modlfo_to_pitch()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_set_modlfo_to_pitch(
    voice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    voice.envlfo.modlfo_to_pitch = param[0].real;
}

/// Translation of `fluid_rvoice_set_modlfo_to_vol()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_set_modlfo_to_vol(
    voice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    voice.envlfo.modlfo_to_vol = param[0].real;
}

/// Translation of `fluid_rvoice_set_modlfo_to_fc()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_set_modlfo_to_fc(
    voice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    voice.envlfo.modlfo_to_fc = param[0].real;
}

/// Translation of `fluid_rvoice_set_modenv_to_fc()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_set_modenv_to_fc(
    voice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    voice.envlfo.modenv_to_fc = param[0].real;
}

/// Translation of `fluid_rvoice_set_modenv_to_pitch()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_set_modenv_to_pitch(
    voice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    voice.envlfo.modenv_to_pitch = param[0].real;
}

/// Translation of `fluid_rvoice_set_synth_gain()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_set_synth_gain(
    voice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    let value = param[0].real;

    voice.dsp.synth_gain = value;

    /* For a looped sample, this value will be overwritten as soon as the
     * loop parameters are initialized (they may depend on modulators).
     * This value can be kept, it is a worst-case estimate.
     */
    voice.dsp.amplitude_that_reaches_noise_floor_nonloop = FLUID_NOISE_FLOOR / value;
    voice.dsp.amplitude_that_reaches_noise_floor_loop = FLUID_NOISE_FLOOR / value;
    voice.dsp.check_sample_sanity_flag |= FLUID_SAMPLESANITY_CHECK;
}

/// Translation of `fluid_rvoice_set_start()` (an rvoice event function).
pub(crate) fn fluid_rvoice_set_start(
    voice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    voice.dsp.start = param[0].i;
    voice.dsp.check_sample_sanity_flag |= FLUID_SAMPLESANITY_CHECK;
}

/// Translation of `fluid_rvoice_set_end()` (an rvoice event function).
pub(crate) fn fluid_rvoice_set_end(
    voice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    voice.dsp.end = param[0].i;
    voice.dsp.check_sample_sanity_flag |= FLUID_SAMPLESANITY_CHECK;
}

/// Translation of `fluid_rvoice_set_loopstart()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_set_loopstart(
    voice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    voice.dsp.loopstart = param[0].i;
    voice.dsp.check_sample_sanity_flag |= FLUID_SAMPLESANITY_CHECK;
}

/// Translation of `fluid_rvoice_set_loopend()` (an rvoice event function).
pub(crate) fn fluid_rvoice_set_loopend(
    voice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    voice.dsp.loopend = param[0].i;
    voice.dsp.check_sample_sanity_flag |= FLUID_SAMPLESANITY_CHECK;
}

/// Translation of `fluid_rvoice_set_samplemode()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_set_samplemode(
    voice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    voice.dsp.samplemode = param[0].i;
    voice.dsp.check_sample_sanity_flag |= FLUID_SAMPLESANITY_CHECK;
}

/// Translation of `fluid_rvoice_set_sample()` (an rvoice event function).
pub(crate) fn fluid_rvoice_set_sample(
    voice: &mut FluidRvoice,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    let value = match &param[0].ptr {
        RvoicePtr::Sample(s) => Some(s.clone()),
        _ => None,
    };

    let set = value.is_some();
    voice.dsp.sample = value;

    if set {
        voice.dsp.check_sample_sanity_flag |= FLUID_SAMPLESANITY_STARTUP;
    }
}

/// Translation of `fluid_rvoice_voiceoff()` (an rvoice event function).
pub(crate) fn fluid_rvoice_voiceoff(
    voice: &mut FluidRvoice,
    _param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    fluid_adsr_env_set_section(&mut voice.envlfo.volenv, FLUID_VOICE_ENVFINISHED);
    fluid_adsr_env_set_section(&mut voice.envlfo.modenv, FLUID_VOICE_ENVFINISHED);
}
