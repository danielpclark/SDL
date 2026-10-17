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
// Modified 2026-10-17: translated into Rust from fluid_lfo.h and fluid_lfo.c
// (which carries no notice of its own) in SDL_mixer 3.3.0's FluidSynth
// 2.5.7 (see NOTICE).

use crate::fluidsynth_priv::{RvoiceParam, MAX_EVENT_PARAMS};
use crate::FluidReal;

/// Translation of `fluid_lfo_t`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct FluidLfo {
    pub val: FluidReal,       /* the current value of the LFO */
    pub delay: u32,           /* the delay of the lfo in samples */
    pub increment: FluidReal, /* the lfo frequency is converted to a per-buffer increment */
}

/// Translation of `fluid_lfo_reset()`.
#[inline]
pub(crate) fn fluid_lfo_reset(lfo: &mut FluidLfo) {
    lfo.val = 0.0;
}

// These two cannot be inlined since they're used by event_dispatch
/// Translation of `fluid_lfo_set_incr()` (an rvoice event function).
pub(crate) fn fluid_lfo_set_incr(lfo: &mut FluidLfo, param: &[RvoiceParam; MAX_EVENT_PARAMS]) {
    let increment = param[0].real;

    lfo.increment = increment;
}

/// Translation of `fluid_lfo_set_delay()` (an rvoice event function).
pub(crate) fn fluid_lfo_set_delay(lfo: &mut FluidLfo, param: &[RvoiceParam; MAX_EVENT_PARAMS]) {
    let delay = param[0].i as u32;

    lfo.delay = delay;
}

/// Translation of `fluid_lfo_get_val()`.
#[inline]
pub(crate) fn fluid_lfo_get_val(lfo: &FluidLfo) -> FluidReal {
    lfo.val
}

/// Translation of `fluid_lfo_calc()`.
#[inline]
pub(crate) fn fluid_lfo_calc(lfo: &mut FluidLfo, cur_delay: u32) {
    if cur_delay < lfo.delay {
        return;
    }

    lfo.val += lfo.increment;

    if lfo.val > 1.0 {
        lfo.increment = -lfo.increment;
        lfo.val = 2.0 - lfo.val;
    } else if lfo.val < -1.0 {
        lfo.increment = -lfo.increment;
        lfo.val = -2.0 - lfo.val;
    }
}
