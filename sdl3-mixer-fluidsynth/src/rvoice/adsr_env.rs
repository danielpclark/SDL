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
// Modified 2026-10-17: translated into Rust from fluid_adsr_env.c and
// fluid_adsr_env.h in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE).

use crate::fluidsynth_priv::{RvoiceParam, MAX_EVENT_PARAMS};
use crate::FluidReal;

/// Translation of `fluid_env_data_t`: envelope data.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct FluidEnvData {
    pub count: u32,
    pub coeff: FluidReal,
    pub increment: FluidReal,
    pub min: FluidReal,
    pub max: FluidReal,
}

/* Indices for envelope tables */
// (enum fluid_voice_envelope_index, fluid_adsr_env_section_t)
pub(crate) const FLUID_VOICE_ENVDELAY: u32 = 0;
pub(crate) const FLUID_VOICE_ENVATTACK: u32 = 1;
pub(crate) const FLUID_VOICE_ENVHOLD: u32 = 2;
pub(crate) const FLUID_VOICE_ENVDECAY: u32 = 3;
pub(crate) const FLUID_VOICE_ENVSUSTAIN: u32 = 4;
pub(crate) const FLUID_VOICE_ENVRELEASE: u32 = 5;
pub(crate) const FLUID_VOICE_ENVFINISHED: u32 = 6;
pub(crate) const FLUID_VOICE_ENVLAST: usize = 7;

/// Translation of `fluid_adsr_env_t`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct FluidAdsrEnv {
    pub data: [FluidEnvData; FLUID_VOICE_ENVLAST],
    pub section: u32, // type fluid_adsr_env_section_t, but declare it unsigned to make C++ happy
    pub count: u32,
    pub val: FluidReal, /* the current value of the envelope */
}

/* For performance, all functions are inlined */
/// Translation of `fluid_adsr_env_calc()`.
#[inline]
pub(crate) fn fluid_adsr_env_calc(env: &mut FluidAdsrEnv) {
    let mut x: FluidReal;

    // Note (upstream): the section runs past FINISHED (whose count is
    // 0xffffffff, so it stays there in practice); a section beyond the
    // table reads the FINISHED data here.
    let sec = |s: u32| (s as usize).min(FLUID_VOICE_ENVLAST - 1);
    let mut env_data = env.data[sec(env.section)];

    /* skip to the next section of the envelope if necessary */
    while env.count >= env_data.count {
        // If we're switching envelope stages from decay to sustain, force the value to be the end value of the previous stage
        // Hmm, should this only apply to volenv? It was so before refactoring, so keep it for now. [DH]
        // No, must apply to both, otherwise some voices may sound detuned. [TM] (https://github.com/FluidSynth/fluidsynth/issues/1059)
        if env.section == FLUID_VOICE_ENVDECAY {
            env.val = env_data.min * env_data.coeff;
        }

        env.section += 1;
        env_data = env.data[sec(env.section)];
        env.count = 0;
    }

    /* calculate the envelope value and check for valid range */
    x = env_data.coeff * env.val + env_data.increment;

    if x < env_data.min {
        x = env_data.min;
        env.section += 1;
        env.count = 0;
    } else if x > env_data.max {
        x = env_data.max;
        env.section += 1;
        env.count = 0;
    } else {
        env.count += 1;
    }

    env.val = x;
}

/// Translation of `fluid_adsr_env_set_data()` (an rvoice event function).
pub(crate) fn fluid_adsr_env_set_data(
    env: &mut FluidAdsrEnv,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    let section = param[0].i;
    let count = param[1].i as u32;
    let coeff = param[2].real;
    let increment = param[3].real;
    let min = param[4].real;
    let max = param[5].real;

    // (sections come from the voice code, always in range)
    let Some(data) = env.data.get_mut(section as usize) else {
        return;
    };
    data.count = count;
    data.coeff = coeff;
    data.increment = increment;
    data.min = min;
    data.max = max;
}

/// Translation of `fluid_adsr_env_reset()`.
#[inline]
pub(crate) fn fluid_adsr_env_reset(env: &mut FluidAdsrEnv) {
    env.count = 0;
    env.section = FLUID_VOICE_ENVDELAY;
    env.val = 0.0;
}

/// Translation of `fluid_adsr_env_get_val()`.
#[inline]
pub(crate) fn fluid_adsr_env_get_val(env: &FluidAdsrEnv) -> FluidReal {
    env.val
}

/// Translation of `fluid_adsr_env_set_val()`.
#[inline]
pub(crate) fn fluid_adsr_env_set_val(env: &mut FluidAdsrEnv, val: FluidReal) {
    env.val = val;
}

/// Translation of `fluid_adsr_env_get_section()`.
#[inline]
pub(crate) fn fluid_adsr_env_get_section(env: &FluidAdsrEnv) -> u32 {
    env.section
}

/// Translation of `fluid_adsr_env_set_section()`.
#[inline]
pub(crate) fn fluid_adsr_env_set_section(env: &mut FluidAdsrEnv, section: u32) {
    env.section = section;
    env.count = 0;
}

/// Translation of `fluid_adsr_env_get_max_val()`.
///
/// Used for determining which voice to kill.
/// Returns max amplitude from now, and forward in time.
#[allow(dead_code)]
#[inline]
pub(crate) fn fluid_adsr_env_get_max_val(env: &FluidAdsrEnv) -> FluidReal {
    if env.section > FLUID_VOICE_ENVATTACK {
        env.val * 1000.0
    } else {
        env.data[FLUID_VOICE_ENVATTACK as usize].max
    }
}
