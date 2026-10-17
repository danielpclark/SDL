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
// Modified 2026-10-17: translated into Rust from fluid_tuning.c and
// fluid_tuning.h in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE). The
// reference count is the `Arc`s the synth and its channels hold.

/*

  More information about micro tuning can be found at:

  https://www.midi.org/about-midi/tuning.htm
  https://www.midi.org/about-midi/tuning-scale.htm
  https://www.midi.org/about-midi/tuning_extens.htm

*/

/// Translation of `fluid_tuning_t`.
#[derive(Clone, Debug)]
pub(crate) struct FluidTuning {
    pub name: Option<String>,
    #[allow(dead_code)]
    pub bank: i32,
    #[allow(dead_code)]
    pub prog: i32,
    pub pitch: [f64; 128], /* the pitch of every key, in cents */
}

impl FluidTuning {
    /// Translation of `new_fluid_tuning()`.
    pub(crate) fn new(name: Option<&str>, bank: i32, prog: i32) -> FluidTuning {
        let mut tuning = FluidTuning {
            name: None,
            bank,
            prog,
            pitch: [0.0; 128],
        };

        tuning.set_name(name);

        for i in 0..128 {
            tuning.pitch[i] = i as f64 * 100.0;
        }

        tuning
    }

    /// Translation of `fluid_tuning_duplicate()`: duplicate a tuning.
    pub(crate) fn duplicate(&self) -> FluidTuning {
        self.clone()
    }

    /// Translation of `fluid_tuning_set_name()`.
    pub(crate) fn set_name(&mut self, name: Option<&str>) {
        self.name = name.map(str::to_string);
    }

    /// Translation of `fluid_tuning_get_name()`.
    #[allow(dead_code)]
    pub(crate) fn get_name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// `fluid_tuning_get_bank()`
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn get_bank(&self) -> i32 {
        self.bank
    }

    /// `fluid_tuning_get_prog()`
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn get_prog(&self) -> i32 {
        self.prog
    }

    /// `fluid_tuning_get_pitch()`
    #[inline]
    pub(crate) fn get_pitch(&self, key: i32) -> f64 {
        // (keys outside 0-127 index past the array upstream; the callers
        // pass MIDI keys)
        self.pitch[(key as usize) & 127]
    }

    /// Translation of `fluid_tuning_set_octave()`.
    pub(crate) fn set_octave(&mut self, pitch_deriv: &[f64; 12]) {
        for i in 0..128 {
            self.pitch[i] = i as f64 * 100.0 + pitch_deriv[i % 12];
        }
    }

    /// Translation of `fluid_tuning_set_all()`.
    #[allow(dead_code)]
    pub(crate) fn set_all(&mut self, pitch: &[f64; 128]) {
        self.pitch = *pitch;
    }

    /// Translation of `fluid_tuning_set_pitch()`.
    pub(crate) fn set_pitch(&mut self, key: i32, pitch: f64) {
        if (0..128).contains(&key) {
            self.pitch[key as usize] = pitch;
        }
    }
}
