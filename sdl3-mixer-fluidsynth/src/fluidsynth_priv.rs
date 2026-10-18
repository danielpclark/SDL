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
// Modified 2026-10-17: translated into Rust from fluidsynth_priv.h in
// SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE).

//! lightweight part of fluid_sys.h, containing forward declarations of
//! fluidsynth's private types and private macros

use std::sync::Arc;

use crate::sfloader::sfont::Sample;
use crate::FluidReal;

/// Translation of the `ptr` member of `fluid_rvoice_param_t`: the
/// pointers rvoice events carry.
#[derive(Clone, Default)]
pub(crate) enum RvoicePtr {
    #[default]
    Null,
    /// An rvoice (its index in the mixer's rvoice pool).
    Rvoice(usize),
    /// A sample.
    Sample(Arc<Sample>),
}

/// Translation of `fluid_rvoice_param_t` (a union in C; each event
/// function reads the member its caller set).
#[derive(Clone, Default)]
pub(crate) struct RvoiceParam {
    pub ptr: RvoicePtr,
    pub i: i32,
    pub real: FluidReal,
}

impl RvoiceParam {
    /// A param with its `i` member set.
    #[allow(dead_code)]
    pub(crate) fn i(i: i32) -> Self {
        RvoiceParam {
            i,
            ..Default::default()
        }
    }
    /// A param with its `real` member set.
    #[allow(dead_code)]
    pub(crate) fn real(real: FluidReal) -> Self {
        RvoiceParam {
            real,
            ..Default::default()
        }
    }
}

/// Maximum number of #fluid_rvoice_param_t to be passed to an
/// #fluid_rvoice_function_t (`MAX_EVENT_PARAMS`)
pub(crate) const MAX_EVENT_PARAMS: usize = 7;

/// A fresh `fluid_rvoice_param_t param[MAX_EVENT_PARAMS]`.
pub(crate) fn params() -> [RvoiceParam; MAX_EVENT_PARAMS] {
    Default::default()
}

/***************************************************************
 *
 *                      CONSTANTS
 */

/// FluidSynth internal buffer size (in samples)
pub(crate) const FLUID_BUFSIZE: usize = 64;
/// Number of buffers that can be processed in one rendering run
pub(crate) const FLUID_MIXER_MAX_BUFFERS_DEFAULT: usize = 8192 / FLUID_BUFSIZE;
/// Maximum number of modulators in a voice
pub(crate) const FLUID_NUM_MOD: usize = 64;

/* Math constants */
/// C's conversion of a `double` to `int` as x86-64 does it (`cvttsd2si`):
/// truncation toward zero, and `INT_MIN` for NaN or a value out of
/// range, where C's conversion is undefined (Note (upstream) at the
/// places that use this).
#[inline]
pub(crate) fn c_double_to_int(x: f64) -> i32 {
    if x > -2147483649.0 && x < 2147483648.0 {
        x as i32
    } else {
        i32::MIN
    }
}

pub(crate) const M_PI: f64 = 3.1415926535897932384626433832795;
pub(crate) const M_LN10: f64 = 2.3025850929940456840179914546844;

pub(crate) const FLUID_M_PI: FluidReal = M_PI;
pub(crate) const FLUID_M_LN10: FluidReal = M_LN10;

/// Translation of `fluid_clip()`.
#[inline]
pub(crate) fn fluid_clip<T: PartialOrd>(val: T, min: T, max: T) -> T {
    if val < min {
        min
    } else if val > max {
        max
    } else {
        val
    }
}
