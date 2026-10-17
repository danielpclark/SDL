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
// Modified 2026-10-17: translated into Rust from fluid_rvoice_dsp.cpp in
// SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE).

/* Purpose:
 *
 * Interpolates audio data (obtains values between the samples of the original
 * waveform data).
 *
 * Variables loaded from the voice structure (assigned in fluid_rvoice_write()):
 * - dsp_data: Pointer to the original waveform data
 * - dsp_phase: The position in the original waveform data.
 *              This has an integer and a fractional part (between samples).
 * - dsp_phase_incr: For each output sample, the position in the original
 *              waveform advances by dsp_phase_incr. This also has an integer
 *              part and a fractional part.
 *              If a sample is played at root pitch (no pitch change),
 *              dsp_phase_incr is integer=1 and fractional=0.
 * - dsp_amp: The current amplitude envelope value.
 * - dsp_amp_incr: The changing rate of the amplitude envelope.
 *
 * A couple of variables are used internally, their results are discarded:
 * - dsp_i: Index through the output buffer
 * - dsp_buf: Output buffer of floating point values (FLUID_BUFSIZE in length)
 */

use crate::fluidsynth_h::{FLUID_INTERP_7THORDER, FLUID_INTERP_LINEAR, FLUID_INTERP_NONE};
use crate::fluidsynth_priv::FLUID_BUFSIZE;
use crate::gentables::{INTERP_COEFF, INTERP_COEFF_LINEAR, SINC_TABLE7};
use crate::rvoice::dsp_tables::{CUBIC_INTERP_ORDER, LINEAR_INTERP_ORDER, SINC_INTERP_ORDER};
use crate::rvoice::phase::*;
use crate::rvoice::rvoice::{fluid_rvoice_get_sample16, fluid_rvoice_get_sample24, FluidRvoice};
use crate::FluidReal;

/* Interpolation (find a value between two samples of the original waveform) */

/// Translation of `fluid_rvoice_get_float_sample<IS_24BIT>()`.
#[inline(always)]
fn get<const IS_24BIT: bool>(dsp_msb: &[i16], dsp_lsb: &[u8], idx: u32) -> FluidReal {
    let sample = if IS_24BIT {
        fluid_rvoice_get_sample24(dsp_msb, dsp_lsb, idx)
    } else {
        fluid_rvoice_get_sample16(dsp_msb, idx)
    };

    sample as FluidReal
}

/// Translation of `fluid_rvoice_dsp_silence_local<LOOPING>()`: special case
/// of interpolate_none for rendering silent voices, i.e. in delay phase or
/// zero volume.
fn fluid_rvoice_dsp_silence_local<const LOOPING: bool>(
    rvoice: &mut FluidRvoice,
    dsp_buf: &mut [FluidReal],
) -> i32 {
    let voice = &mut rvoice.dsp;
    let mut dsp_phase = voice.phase;
    let mut dsp_i: usize = 0;
    let mut dsp_phase_index: u32;

    /* Convert playback "speed" floating point value to phase index/fract */
    let dsp_phase_incr = fluid_phase_set_float(voice.phase_incr);

    let end_index: u32 = if LOOPING {
        voice.loopend.wrapping_sub(1) as u32
    } else {
        voice.end as u32
    };

    loop {
        dsp_phase_index = fluid_phase_index_round(dsp_phase); /* round to nearest point */

        /* interpolate sequence of sample points */
        while dsp_i < FLUID_BUFSIZE && dsp_phase_index <= end_index {
            let sample = 0.0;
            dsp_buf[dsp_i] = sample;

            /* increment phase and amplitude */
            dsp_phase = dsp_phase.wrapping_add(dsp_phase_incr);
            dsp_i += 1;
        }

        /* break out if not looping (buffer may not be full) */
        if !LOOPING {
            break;
        }

        dsp_phase_index = fluid_phase_index_round(dsp_phase); /* round to nearest point */
        /* go back to loop start */
        if dsp_phase_index > end_index {
            fluid_phase_sub_int(&mut dsp_phase, voice.loopend.wrapping_sub(voice.loopstart));
            voice.has_looped = true;
        }

        /* break out if filled buffer */
        if dsp_i >= FLUID_BUFSIZE {
            break;
        }
    }

    voice.phase = dsp_phase;
    // Note, there is no need to update the amplitude here. When the voice becomes audible again, the amp will be updated anyway in fluid_rvoice_calc_amp().
    // voice->amp = dsp_amp;

    dsp_i as i32
}

/// Translation of `fluid_rvoice_dsp_interpolate_none_local<IS_24BIT,
/// LOOPING>()`.
///
/// No interpolation. Just take the sample, which is closest to
/// the playback pointer.  Questionable quality, but very
/// efficient.
fn fluid_rvoice_dsp_interpolate_none_local<const IS_24BIT: bool, const LOOPING: bool>(
    rvoice: &mut FluidRvoice,
    dsp_buf: &mut [FluidReal],
    dsp_data: &[i16],
    dsp_data24: &[u8],
) -> i32 {
    let voice = &mut rvoice.dsp;
    let mut dsp_phase = voice.phase;
    let mut dsp_i: usize = 0;
    let mut dsp_phase_index: u32;

    /* Convert playback "speed" floating point value to phase index/fract */
    let dsp_phase_incr = fluid_phase_set_float(voice.phase_incr);

    let end_index: u32 = if LOOPING {
        voice.loopend.wrapping_sub(1) as u32
    } else {
        voice.end as u32
    };

    loop {
        dsp_phase_index = fluid_phase_index_round(dsp_phase); /* round to nearest point */

        /* interpolate sequence of sample points */
        while dsp_i < FLUID_BUFSIZE && dsp_phase_index <= end_index {
            let sample = get::<IS_24BIT>(dsp_data, dsp_data24, dsp_phase_index);

            dsp_buf[dsp_i] = sample;

            /* increment phase and amplitude */
            dsp_phase = dsp_phase.wrapping_add(dsp_phase_incr);
            dsp_phase_index = fluid_phase_index_round(dsp_phase); /* round to nearest point */
            dsp_i += 1;
        }

        /* break out if not looping (buffer may not be full) */
        if !LOOPING {
            break;
        }

        /* go back to loop start */
        if dsp_phase_index > end_index {
            fluid_phase_sub_int(&mut dsp_phase, voice.loopend.wrapping_sub(voice.loopstart));
            voice.has_looped = true;
        }

        /* break out if filled buffer */
        if dsp_i >= FLUID_BUFSIZE {
            break;
        }
    }

    voice.phase = dsp_phase;

    dsp_i as i32
}

/// Translation of `fluid_rvoice_dsp_interpolate_linear_local<IS_24BIT,
/// LOOPING>()`: straight line interpolation.
///
/// Returns number of samples processed (usually FLUID_BUFSIZE but could be
/// smaller if end of sample occurs).
fn fluid_rvoice_dsp_interpolate_linear_local<const IS_24BIT: bool, const LOOPING: bool>(
    rvoice: &mut FluidRvoice,
    dsp_buf: &mut [FluidReal],
    dsp_data: &[i16],
    dsp_data24: &[u8],
) -> i32 {
    let voice = &mut rvoice.dsp;
    let mut dsp_phase = voice.phase;
    let mut dsp_i: usize = 0;
    let mut dsp_phase_index: u32;
    let interp_coeff_linear = &**INTERP_COEFF_LINEAR;

    /* Convert playback "speed" floating point value to phase index/fract */
    let dsp_phase_incr = fluid_phase_set_float(voice.phase_incr);

    /* last index before 2nd interpolation point must be specially handled */
    let mut end_index: u32 = (if LOOPING {
        voice.loopend.wrapping_sub(1) as u32
    } else {
        voice.end as u32
    })
    .wrapping_sub(1);

    /* 2nd interpolation point to use at end of loop or sample */
    let point = if LOOPING {
        get::<IS_24BIT>(dsp_data, dsp_data24, voice.loopstart as u32) /* loop start */
    } else {
        get::<IS_24BIT>(dsp_data, dsp_data24, voice.end as u32) /* duplicate end for samples no longer looping */
    };

    loop {
        dsp_phase_index = fluid_phase_index(dsp_phase);

        /* interpolate the sequence of sample points */
        while dsp_i < FLUID_BUFSIZE && dsp_phase_index <= end_index {
            let row = fluid_phase_fract_to_tablerow(dsp_phase) as usize * LINEAR_INTERP_ORDER;
            let coeffs = &interp_coeff_linear[row..row + LINEAR_INTERP_ORDER];

            let sample = coeffs[0] * get::<IS_24BIT>(dsp_data, dsp_data24, dsp_phase_index)
                + coeffs[1]
                    * get::<IS_24BIT>(dsp_data, dsp_data24, dsp_phase_index.wrapping_add(1));

            dsp_buf[dsp_i] = sample;

            /* increment phase and amplitude */
            dsp_phase = dsp_phase.wrapping_add(dsp_phase_incr);
            dsp_phase_index = fluid_phase_index(dsp_phase);
            dsp_i += 1;
        }

        /* break out if buffer filled */
        if dsp_i >= FLUID_BUFSIZE {
            break;
        }

        end_index = end_index.wrapping_add(1); /* we're now interpolating the last point */

        /* interpolate within last point */
        while dsp_phase_index <= end_index && dsp_i < FLUID_BUFSIZE {
            let row = fluid_phase_fract_to_tablerow(dsp_phase) as usize * LINEAR_INTERP_ORDER;
            let coeffs = &interp_coeff_linear[row..row + LINEAR_INTERP_ORDER];

            let sample = coeffs[0] * get::<IS_24BIT>(dsp_data, dsp_data24, dsp_phase_index)
                + coeffs[1] * point;

            dsp_buf[dsp_i] = sample;

            /* increment phase and amplitude */
            dsp_phase = dsp_phase.wrapping_add(dsp_phase_incr);
            dsp_phase_index = fluid_phase_index(dsp_phase);
            dsp_i += 1;
        }

        if !LOOPING {
            break; /* break out if not looping (end of sample) */
        }

        /* go back to loop start (if past */
        if dsp_phase_index > end_index {
            fluid_phase_sub_int(&mut dsp_phase, voice.loopend.wrapping_sub(voice.loopstart));
            voice.has_looped = true;
        }

        /* break out if filled buffer */
        if dsp_i >= FLUID_BUFSIZE {
            break;
        }

        end_index = end_index.wrapping_sub(1); /* set end back to second to last sample point */
    }

    voice.phase = dsp_phase;

    dsp_i as i32
}

/// Translation of `fluid_rvoice_dsp_interpolate_4th_order_local<IS_24BIT,
/// LOOPING>()`: 4th order (cubic) interpolation.
///
/// Returns number of samples processed (usually FLUID_BUFSIZE but could be
/// smaller if end of sample occurs).
fn fluid_rvoice_dsp_interpolate_4th_order_local<const IS_24BIT: bool, const LOOPING: bool>(
    rvoice: &mut FluidRvoice,
    dsp_buf: &mut [FluidReal],
    dsp_data: &[i16],
    dsp_data24: &[u8],
) -> i32 {
    let voice = &mut rvoice.dsp;
    let mut dsp_phase = voice.phase;
    let mut dsp_i: usize = 0;
    let mut dsp_phase_index: u32;
    let mut start_index: u32;
    let mut end_index: u32;
    let mut start_point: FluidReal;
    let end_point1: FluidReal;
    let end_point2: FluidReal;
    let interp_coeff = &**INTERP_COEFF;
    let g = |idx: u32| get::<IS_24BIT>(dsp_data, dsp_data24, idx);

    /* Convert playback "speed" floating point value to phase index/fract */
    let dsp_phase_incr = fluid_phase_set_float(voice.phase_incr);

    /* last index before 4th interpolation point must be specially handled */
    end_index = (if LOOPING {
        voice.loopend.wrapping_sub(1) as u32
    } else {
        voice.end as u32
    })
    .wrapping_sub(2);

    if voice.has_looped {
        /* set start_index and start point if looped or not */
        start_index = voice.loopstart as u32;
        start_point = g(voice.loopend.wrapping_sub(1) as u32); /* last point in loop (wrap around) */
    } else {
        start_index = voice.start as u32;
        start_point = g(voice.start as u32); /* just duplicate the point */
    }

    /* get points off the end (loop start if looping, duplicate point if end) */
    if LOOPING {
        end_point1 = g(voice.loopstart as u32);
        end_point2 = g(voice.loopstart.wrapping_add(1) as u32);
    } else {
        end_point1 = g(voice.end as u32);
        end_point2 = end_point1;
    }

    let coeffs_at = |phase: FluidPhase| {
        let row = fluid_phase_fract_to_tablerow(phase) as usize * CUBIC_INTERP_ORDER;
        &interp_coeff[row..row + CUBIC_INTERP_ORDER]
    };

    loop {
        dsp_phase_index = fluid_phase_index(dsp_phase);

        /* interpolate first sample point (start or loop start) if needed */
        while dsp_phase_index == start_index && dsp_i < FLUID_BUFSIZE {
            let coeffs = coeffs_at(dsp_phase);

            let sample = coeffs[0] * start_point
                + coeffs[1] * g(dsp_phase_index)
                + coeffs[2] * g(dsp_phase_index.wrapping_add(1))
                + coeffs[3] * g(dsp_phase_index.wrapping_add(2));

            dsp_buf[dsp_i] = sample;

            /* increment phase and amplitude */
            dsp_phase = dsp_phase.wrapping_add(dsp_phase_incr);
            dsp_phase_index = fluid_phase_index(dsp_phase);
            dsp_i += 1;
        }

        /* interpolate the sequence of sample points */
        while dsp_i < FLUID_BUFSIZE && dsp_phase_index <= end_index {
            let coeffs = coeffs_at(dsp_phase);

            let sample = coeffs[0] * g(dsp_phase_index.wrapping_sub(1))
                + coeffs[1] * g(dsp_phase_index)
                + coeffs[2] * g(dsp_phase_index.wrapping_add(1))
                + coeffs[3] * g(dsp_phase_index.wrapping_add(2));

            dsp_buf[dsp_i] = sample;

            /* increment phase and amplitude */
            dsp_phase = dsp_phase.wrapping_add(dsp_phase_incr);
            dsp_phase_index = fluid_phase_index(dsp_phase);
            dsp_i += 1;
        }

        /* break out if buffer filled */
        if dsp_i >= FLUID_BUFSIZE {
            break;
        }

        end_index = end_index.wrapping_add(1); /* we're now interpolating the 2nd to last point */

        /* interpolate within 2nd to last point */
        while dsp_phase_index <= end_index && dsp_i < FLUID_BUFSIZE {
            let coeffs = coeffs_at(dsp_phase);

            let sample = coeffs[0] * g(dsp_phase_index.wrapping_sub(1))
                + coeffs[1] * g(dsp_phase_index)
                + coeffs[2] * g(dsp_phase_index.wrapping_add(1))
                + coeffs[3] * end_point1;

            dsp_buf[dsp_i] = sample;

            /* increment phase and amplitude */
            dsp_phase = dsp_phase.wrapping_add(dsp_phase_incr);
            dsp_phase_index = fluid_phase_index(dsp_phase);
            dsp_i += 1;
        }

        end_index = end_index.wrapping_add(1); /* we're now interpolating the last point */

        /* interpolate within the last point */
        while dsp_phase_index <= end_index && dsp_i < FLUID_BUFSIZE {
            let coeffs = coeffs_at(dsp_phase);

            let sample = coeffs[0] * g(dsp_phase_index.wrapping_sub(1))
                + coeffs[1] * g(dsp_phase_index)
                + coeffs[2] * end_point1
                + coeffs[3] * end_point2;

            dsp_buf[dsp_i] = sample;

            /* increment phase and amplitude */
            dsp_phase = dsp_phase.wrapping_add(dsp_phase_incr);
            dsp_phase_index = fluid_phase_index(dsp_phase);
            dsp_i += 1;
        }

        if !LOOPING {
            break; /* break out if not looping (end of sample) */
        }

        /* go back to loop start */
        if dsp_phase_index > end_index {
            fluid_phase_sub_int(&mut dsp_phase, voice.loopend.wrapping_sub(voice.loopstart));

            if !voice.has_looped {
                voice.has_looped = true;
                start_index = voice.loopstart as u32;
                start_point = g(voice.loopend.wrapping_sub(1) as u32);
            }
        }

        /* break out if filled buffer */
        if dsp_i >= FLUID_BUFSIZE {
            break;
        }

        end_index = end_index.wrapping_sub(2); /* set end back to third to last sample point */
    }

    voice.phase = dsp_phase;

    dsp_i as i32
}

/// Translation of `fluid_rvoice_dsp_interpolate_7th_order_local<IS_24BIT,
/// LOOPING>()`: 7th order interpolation.
///
/// Returns number of samples processed (usually FLUID_BUFSIZE but could be
/// smaller if end of sample occurs).
fn fluid_rvoice_dsp_interpolate_7th_order_local<const IS_24BIT: bool, const LOOPING: bool>(
    rvoice: &mut FluidRvoice,
    dsp_buf: &mut [FluidReal],
    dsp_data: &[i16],
    dsp_data24: &[u8],
) -> i32 {
    let voice = &mut rvoice.dsp;
    let mut dsp_phase = voice.phase;
    let mut dsp_i: usize = 0;
    let mut dsp_phase_index: u32;
    let mut start_index: u32;
    let mut end_index: u32;
    let mut start_points: [FluidReal; 3] = [0.0; 3];
    let mut end_points: [FluidReal; 3] = [0.0; 3];
    let sinc_table7 = &**SINC_TABLE7;
    let g = |idx: u32| get::<IS_24BIT>(dsp_data, dsp_data24, idx);

    /* Convert playback "speed" floating point value to phase index/fract */
    let dsp_phase_incr = fluid_phase_set_float(voice.phase_incr);

    /* add 1/2 sample to dsp_phase since 7th order interpolation is centered on
     * the 4th sample point */
    dsp_phase = dsp_phase.wrapping_add(0x80000000);

    /* last index before 7th interpolation point must be specially handled */
    end_index = (if LOOPING {
        voice.loopend.wrapping_sub(1) as u32
    } else {
        voice.end as u32
    })
    .wrapping_sub(3);

    if voice.has_looped {
        /* set start_index and start point if looped or not */
        start_index = voice.loopstart as u32;
        start_points[0] = g(voice.loopend.wrapping_sub(1) as u32);
        start_points[1] = g(voice.loopend.wrapping_sub(2) as u32);
        start_points[2] = g(voice.loopend.wrapping_sub(3) as u32);
    } else {
        start_index = voice.start as u32;
        start_points[0] = g(voice.start as u32); /* just duplicate the start point */
        start_points[1] = start_points[0];
        start_points[2] = start_points[0];
    }

    /* get the 3 points off the end (loop start if looping, duplicate point if end) */
    if LOOPING {
        end_points[0] = g(voice.loopstart as u32);
        end_points[1] = g(voice.loopstart.wrapping_add(1) as u32);
        end_points[2] = g(voice.loopstart.wrapping_add(2) as u32);
    } else {
        end_points[0] = g(voice.end as u32);
        end_points[1] = end_points[0];
        end_points[2] = end_points[0];
    }

    let coeffs_at = |phase: FluidPhase| {
        let row = fluid_phase_fract_to_tablerow(phase) as usize * SINC_INTERP_ORDER;
        &sinc_table7[row..row + SINC_INTERP_ORDER]
    };

    loop {
        dsp_phase_index = fluid_phase_index(dsp_phase);

        /* interpolate first sample point (start or loop start) if needed */
        while dsp_phase_index == start_index && dsp_i < FLUID_BUFSIZE {
            let coeffs = coeffs_at(dsp_phase);

            let sample = coeffs[0] * start_points[2]
                + coeffs[1] * start_points[1]
                + coeffs[2] * start_points[0]
                + coeffs[3] * g(dsp_phase_index)
                + coeffs[4] * g(dsp_phase_index.wrapping_add(1))
                + coeffs[5] * g(dsp_phase_index.wrapping_add(2))
                + coeffs[6] * g(dsp_phase_index.wrapping_add(3));

            dsp_buf[dsp_i] = sample;

            /* increment phase and amplitude */
            dsp_phase = dsp_phase.wrapping_add(dsp_phase_incr);
            dsp_phase_index = fluid_phase_index(dsp_phase);
            dsp_i += 1;
        }

        start_index = start_index.wrapping_add(1);

        /* interpolate 2nd to first sample point (start or loop start) if needed */
        while dsp_phase_index == start_index && dsp_i < FLUID_BUFSIZE {
            let coeffs = coeffs_at(dsp_phase);

            let sample = coeffs[0] * start_points[1]
                + coeffs[1] * start_points[0]
                + coeffs[2] * g(dsp_phase_index.wrapping_sub(1))
                + coeffs[3] * g(dsp_phase_index)
                + coeffs[4] * g(dsp_phase_index.wrapping_add(1))
                + coeffs[5] * g(dsp_phase_index.wrapping_add(2))
                + coeffs[6] * g(dsp_phase_index.wrapping_add(3));

            dsp_buf[dsp_i] = sample;

            /* increment phase and amplitude */
            dsp_phase = dsp_phase.wrapping_add(dsp_phase_incr);
            dsp_phase_index = fluid_phase_index(dsp_phase);
            dsp_i += 1;
        }

        start_index = start_index.wrapping_add(1);

        /* interpolate 3rd to first sample point (start or loop start) if needed */
        while dsp_phase_index == start_index && dsp_i < FLUID_BUFSIZE {
            let coeffs = coeffs_at(dsp_phase);

            let sample = coeffs[0] * start_points[0]
                + coeffs[1] * g(dsp_phase_index.wrapping_sub(2))
                + coeffs[2] * g(dsp_phase_index.wrapping_sub(1))
                + coeffs[3] * g(dsp_phase_index)
                + coeffs[4] * g(dsp_phase_index.wrapping_add(1))
                + coeffs[5] * g(dsp_phase_index.wrapping_add(2))
                + coeffs[6] * g(dsp_phase_index.wrapping_add(3));

            dsp_buf[dsp_i] = sample;

            /* increment phase and amplitude */
            dsp_phase = dsp_phase.wrapping_add(dsp_phase_incr);
            dsp_phase_index = fluid_phase_index(dsp_phase);
            dsp_i += 1;
        }

        start_index = start_index.wrapping_sub(2); /* set back to original start index */

        /* interpolate the sequence of sample points */
        while dsp_i < FLUID_BUFSIZE && dsp_phase_index <= end_index {
            let coeffs = coeffs_at(dsp_phase);

            let sample = coeffs[0] * g(dsp_phase_index.wrapping_sub(3))
                + coeffs[1] * g(dsp_phase_index.wrapping_sub(2))
                + coeffs[2] * g(dsp_phase_index.wrapping_sub(1))
                + coeffs[3] * g(dsp_phase_index)
                + coeffs[4] * g(dsp_phase_index.wrapping_add(1))
                + coeffs[5] * g(dsp_phase_index.wrapping_add(2))
                + coeffs[6] * g(dsp_phase_index.wrapping_add(3));

            dsp_buf[dsp_i] = sample;

            /* increment phase and amplitude */
            dsp_phase = dsp_phase.wrapping_add(dsp_phase_incr);
            dsp_phase_index = fluid_phase_index(dsp_phase);
            dsp_i += 1;
        }

        /* break out if buffer filled */
        if dsp_i >= FLUID_BUFSIZE {
            break;
        }

        end_index = end_index.wrapping_add(1); /* we're now interpolating the 3rd to last point */

        /* interpolate within 3rd to last point */
        while dsp_phase_index <= end_index && dsp_i < FLUID_BUFSIZE {
            let coeffs = coeffs_at(dsp_phase);

            let sample = coeffs[0] * g(dsp_phase_index.wrapping_sub(3))
                + coeffs[1] * g(dsp_phase_index.wrapping_sub(2))
                + coeffs[2] * g(dsp_phase_index.wrapping_sub(1))
                + coeffs[3] * g(dsp_phase_index)
                + coeffs[4] * g(dsp_phase_index.wrapping_add(1))
                + coeffs[5] * g(dsp_phase_index.wrapping_add(2))
                + coeffs[6] * end_points[0];

            dsp_buf[dsp_i] = sample;

            /* increment phase and amplitude */
            dsp_phase = dsp_phase.wrapping_add(dsp_phase_incr);
            dsp_phase_index = fluid_phase_index(dsp_phase);
            dsp_i += 1;
        }

        end_index = end_index.wrapping_add(1); /* we're now interpolating the 2nd to last point */

        /* interpolate within 2nd to last point */
        while dsp_phase_index <= end_index && dsp_i < FLUID_BUFSIZE {
            let coeffs = coeffs_at(dsp_phase);

            let sample = coeffs[0] * g(dsp_phase_index.wrapping_sub(3))
                + coeffs[1] * g(dsp_phase_index.wrapping_sub(2))
                + coeffs[2] * g(dsp_phase_index.wrapping_sub(1))
                + coeffs[3] * g(dsp_phase_index)
                + coeffs[4] * g(dsp_phase_index.wrapping_add(1))
                + coeffs[5] * end_points[0]
                + coeffs[6] * end_points[1];

            dsp_buf[dsp_i] = sample;

            /* increment phase and amplitude */
            dsp_phase = dsp_phase.wrapping_add(dsp_phase_incr);
            dsp_phase_index = fluid_phase_index(dsp_phase);
            dsp_i += 1;
        }

        end_index = end_index.wrapping_add(1); /* we're now interpolating the last point */

        /* interpolate within last point */
        while dsp_phase_index <= end_index && dsp_i < FLUID_BUFSIZE {
            let coeffs = coeffs_at(dsp_phase);

            let sample = coeffs[0] * g(dsp_phase_index.wrapping_sub(3))
                + coeffs[1] * g(dsp_phase_index.wrapping_sub(2))
                + coeffs[2] * g(dsp_phase_index.wrapping_sub(1))
                + coeffs[3] * g(dsp_phase_index)
                + coeffs[4] * end_points[0]
                + coeffs[5] * end_points[1]
                + coeffs[6] * end_points[2];

            dsp_buf[dsp_i] = sample;

            /* increment phase and amplitude */
            dsp_phase = dsp_phase.wrapping_add(dsp_phase_incr);
            dsp_phase_index = fluid_phase_index(dsp_phase);
            dsp_i += 1;
        }

        if !LOOPING {
            break; /* break out if not looping (end of sample) */
        }

        /* go back to loop start */
        if dsp_phase_index > end_index {
            fluid_phase_sub_int(&mut dsp_phase, voice.loopend.wrapping_sub(voice.loopstart));

            if !voice.has_looped {
                voice.has_looped = true;
                start_index = voice.loopstart as u32;
                start_points[0] = g(voice.loopend.wrapping_sub(1) as u32);
                start_points[1] = g(voice.loopend.wrapping_sub(2) as u32);
                start_points[2] = g(voice.loopend.wrapping_sub(3) as u32);
            }
        }

        /* break out if filled buffer */
        if dsp_i >= FLUID_BUFSIZE {
            break;
        }

        end_index = end_index.wrapping_sub(3); /* set end back to 4th to last sample point */
    }

    /* sub 1/2 sample from dsp_phase since 7th order interpolation is centered on
     * the 4th sample point (correct back to real value) */
    dsp_phase = dsp_phase.wrapping_sub(0x80000000);

    voice.phase = dsp_phase;

    dsp_i as i32
}

/// The interpolation kinds (`ProcessSilence`, `InterpolateNone`,
/// `InterpolateLinear`, `Interpolate4thOrder`, `Interpolate7thOrder`).
#[derive(Clone, Copy)]
enum Func {
    Silence,
    None,
    Linear,
    Fourth,
    Seventh,
}

/// Translation of `dsp_invoker<T>()`.
fn dsp_invoker(
    func: Func,
    rvoice: &mut FluidRvoice,
    dsp_buf: &mut [FluidReal],
    looping: bool,
) -> i32 {
    let Some(sample) = rvoice.dsp.sample.clone() else {
        return 0;
    };
    let is_24bit = sample.data24.is_some();
    let data: &[i16] = &sample.data;
    let data24: &[u8] = sample.data24.as_deref().unwrap_or(&[]);

    macro_rules! call {
        ($b24:literal, $loop:literal) => {
            match func {
                Func::Silence => fluid_rvoice_dsp_silence_local::<$loop>(rvoice, dsp_buf),
                Func::None => fluid_rvoice_dsp_interpolate_none_local::<$b24, $loop>(
                    rvoice, dsp_buf, data, data24,
                ),
                Func::Linear => fluid_rvoice_dsp_interpolate_linear_local::<$b24, $loop>(
                    rvoice, dsp_buf, data, data24,
                ),
                Func::Fourth => fluid_rvoice_dsp_interpolate_4th_order_local::<$b24, $loop>(
                    rvoice, dsp_buf, data, data24,
                ),
                Func::Seventh => fluid_rvoice_dsp_interpolate_7th_order_local::<$b24, $loop>(
                    rvoice, dsp_buf, data, data24,
                ),
            }
        };
    }

    if is_24bit {
        if looping {
            call!(true, true)
        } else {
            call!(true, false)
        }
    } else {
        // This case is most common, thanks to templating it will also become the fastest one
        if looping {
            call!(false, true)
        } else {
            call!(false, false)
        }
    }
}

/// Translation of `fluid_rvoice_dsp_silence()`.
pub(crate) fn fluid_rvoice_dsp_silence(
    rvoice: &mut FluidRvoice,
    dsp_buf: &mut [FluidReal],
    looping: bool,
) -> i32 {
    dsp_invoker(Func::Silence, rvoice, dsp_buf, looping)
}

/// Translation of `fluid_rvoice_dsp_interpolate()`.
pub(crate) fn fluid_rvoice_dsp_interpolate(
    rvoice: &mut FluidRvoice,
    dsp_buf: &mut [FluidReal],
    looping: bool,
) -> i32 {
    match rvoice.dsp.interp_method {
        FLUID_INTERP_NONE => dsp_invoker(Func::None, rvoice, dsp_buf, looping),

        FLUID_INTERP_LINEAR => dsp_invoker(Func::Linear, rvoice, dsp_buf, looping),

        FLUID_INTERP_7THORDER => dsp_invoker(Func::Seventh, rvoice, dsp_buf, looping),

        // FLUID_INTERP_4THORDER and default
        _ => dsp_invoker(Func::Fourth, rvoice, dsp_buf, looping),
    }
}
