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
// Modified 2026-10-17: translated into Rust from fluid_phase.h in SDL_mixer
// 3.3.0's FluidSynth 2.5.7 (see NOTICE).

//! phase

pub(crate) const FLUID_INTERP_BITS_MASK: u32 = 0xff000000;
pub(crate) const FLUID_INTERP_BITS_SHIFT: u32 = 24;

pub(crate) const FLUID_FRACT_MAX: f64 = 4294967296.0;

/* fluid_phase_t
 * Purpose:
 * Playing pointer for voice playback
 *
 * When a sample is played back at a different pitch, the playing pointer in the
 * source sample will not advance exactly one sample per output sample.
 * This playing pointer is implemented using fluid_phase_t.
 * It is a 64 bit number. The higher 32 bits contain the 'index' (number of
 * the current sample), the lower 32 bits the fractional part.
 */
/// Translation of `fluid_phase_t`.
pub(crate) type FluidPhase = u64;

/// Translation of `fluid_phase_set_int()`.
#[inline]
pub(crate) fn fluid_phase_set_int(b: i32) -> FluidPhase {
    (b as i64 as u64) << 32
}

/// Translation of `fluid_phase_set_float()`.
///
/// Sets the phase a to a phase increment given in b.
/// For example, assume b is 0.9. After setting a to it, adding a to
/// the playing pointer will advance it by 0.9 samples.
#[inline]
pub(crate) fn fluid_phase_set_float(b: f64) -> FluidPhase {
    // (C's conversions of a double out of range are undefined; these
    // saturate.)
    ((b as u64) << 32) | (((b - (b as i32) as f64) * FLUID_FRACT_MAX) as u32) as u64
}

/// Translation of `fluid_phase_index()`.
#[inline]
pub(crate) fn fluid_phase_index(x: FluidPhase) -> u32 {
    (x >> 32) as u32
}

/// Translation of `fluid_phase_fract()`.
#[inline]
pub(crate) fn fluid_phase_fract(x: FluidPhase) -> u32 {
    (x & 0xFFFFFFFF) as u32
}

/// Translation of `fluid_phase_index_round()`: the phase index with
/// fractional rounding.
#[inline]
pub(crate) fn fluid_phase_index_round(x: FluidPhase) -> u32 {
    (x.wrapping_add(0x80000000) >> 32) as u32
}

/// Translation of `fluid_phase_fract_to_tablerow()`.
///
/// Takes the fractional part of the argument phase and
/// calculates the corresponding position in the interpolation table.
/// The fractional position of the playing pointer is calculated with a quite high
/// resolution (32 bits). It would be unpractical to keep a set of interpolation
/// coefficients for each possible fractional part...
#[inline]
pub(crate) fn fluid_phase_fract_to_tablerow(x: FluidPhase) -> u32 {
    (fluid_phase_fract(x) & FLUID_INTERP_BITS_MASK) >> FLUID_INTERP_BITS_SHIFT
}

/// Translation of `fluid_phase_sub_int()`: subtract b samples from a.
#[inline]
pub(crate) fn fluid_phase_sub_int(a: &mut FluidPhase, b: i32) {
    *a = a.wrapping_sub((b as i64 as u64) << 32);
}
