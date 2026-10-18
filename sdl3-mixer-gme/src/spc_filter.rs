// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/

/* Copyright (C) 2007 Shay Green. This module is free software; you
can redistribute it and/or modify it under the terms of the GNU Lesser
General Public License as published by the Free Software Foundation; either
version 2.1 of the License, or (at your option) any later version. This
module is distributed in the hope that it will be useful, but WITHOUT ANY
WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
FOR A PARTICULAR PURPOSE. See the GNU Lesser General Public License for more
details. You should have received a copy of the GNU Lesser General Public
License along with this module; if not, write to the Free Software Foundation,
Inc., 51 Franklin Street, Fifth Floor, Boston, MA 02110-1301 USA */
// Modified 2026-10-17: translated into Rust from SDL_mixer 3.3.0's Game_Music_Emu 0.6.5 (see NOTICE).

//! Simple low-pass and high-pass filter to better match sound output of a SNES
//! (`Spc_Filter.h`, `Spc_Filter.cpp`).

pub type Sample = i16;

/// Sets gain (volume), where gain_unit is normal. Gains greater than gain_unit
/// are fine, since output is clamped to 16-bit sample range.
pub const GAIN_UNIT: u32 = 0x100;

/// Sets amount of bass (logarithmic scale)
pub const BASS_NONE: u32 = 0;
pub const BASS_NORM: u32 = 8; // normal amount
pub const BASS_MAX: u32 = 31;

const GAIN_BITS: u32 = 8;

#[derive(Clone, Copy, Debug, Default)]
struct Chan {
    p1: i32,
    pp1: i32,
    sum: i32,
}

#[derive(Clone, Debug)]
pub struct SpcFilter {
    gain: i32,
    bass: i32,
    enabled: bool,
    ch: [Chan; 2],
}

impl Default for SpcFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl SpcFilter {
    pub fn new() -> Self {
        let mut f = SpcFilter {
            enabled: true,
            gain: GAIN_UNIT as i32,
            bass: BASS_NORM as i32,
            ch: [Chan::default(); 2],
        };
        f.clear();
        f
    }

    // Optional features

    /// Clears filter to silence
    pub fn clear(&mut self) {
        self.ch = [Chan::default(); 2];
    }

    pub fn set_gain(&mut self, g: i32) {
        self.gain = g;
    }

    /// Enables/disables filtering (when disabled, gain is still applied)
    pub fn enable(&mut self, b: bool) {
        self.enabled = b;
    }

    pub fn set_bass(&mut self, b: i32) {
        self.bass = b;
    }

    /// Filters count samples of stereo sound in place. Count must be a multiple of 2.
    pub fn run(&mut self, io: &mut [Sample], count: i32) {
        // require( (count & 1) == 0 ); // must be even

        let gain = self.gain;
        if self.enabled {
            let bass = self.bass;
            let mut c = 2;
            let mut io_at = 0;
            loop {
                // cache in registers
                c -= 1;
                let ch = &mut self.ch[c];
                let mut sum = ch.sum;
                let mut pp1 = ch.pp1;
                let mut p1 = ch.p1;

                let mut i = 0;
                while i < count as usize {
                    // Low-pass filter (two point FIR with coeffs 0.25, 0.75)
                    let f = io[io_at + i] as i32 + p1;
                    p1 = io[io_at + i] as i32 * 3;

                    // High-pass filter ("leaky integrator")
                    let delta = f - pp1;
                    pp1 = f;
                    let mut s = sum >> (GAIN_BITS + 2);
                    sum = sum
                        .wrapping_add(delta.wrapping_mul(gain))
                        .wrapping_sub(sum >> bass);

                    // Clamp to 16 bits
                    if s as i16 as i32 != s {
                        s = (s >> 31) ^ 0x7FFF;
                    }

                    io[io_at + i] = s as Sample;
                    i += 2;
                }

                ch.p1 = p1;
                ch.pp1 = pp1;
                ch.sum = sum;
                io_at += 1;
                if c == 0 {
                    break;
                }
            }
        } else if gain != GAIN_UNIT as i32 {
            for v in io[..count as usize].iter_mut() {
                let mut s = (*v as i32 * gain) >> GAIN_BITS;
                if s as i16 as i32 != s {
                    s = (s >> 31) ^ 0x7FFF;
                }
                *v = s as Sample;
            }
        }
    }
}
