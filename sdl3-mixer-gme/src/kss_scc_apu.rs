// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/

/* Copyright (C) 2006 Shay Green. This module is free software; you
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

//! Konami SCC sound chip emulator (`Kss_Scc_Apu.h`, `Kss_Scc_Apu.cpp`).

use crate::blip_buffer::{BlipBuffer, BlipEq, BlipSynth, BlipTime, BufId, BLIP_MED_QUALITY};

/// Write to register at specified time
pub const REG_COUNT: i32 = 0x90;

/// Set sound output of specific oscillator to buffer, where index is
/// 0 to 4. If buffer is NULL, the specified oscillator is muted.
pub const OSC_COUNT: i32 = 5;

const AMP_RANGE: u32 = 0x8000;

#[derive(Clone, Copy, Debug, Default)]
struct Osc {
    delay: i32,
    phase: i32,
    last_amp: i32,
    output: Option<BufId>,
}

#[derive(Clone, Debug)]
pub struct SccApu {
    oscs: [Osc; OSC_COUNT as usize],
    last_time: BlipTime,
    regs: [u8; REG_COUNT as usize],
    synth: BlipSynth,
}

impl Default for SccApu {
    fn default() -> Self {
        Self::new()
    }
}

// Tones above this frequency are treated as disabled tone at half volume.
// Power of two is more efficient (avoids division).
const INAUDIBLE_FREQ: u32 = 16384;

const WAVE_SIZE: i32 = 0x20;

impl SccApu {
    pub fn new() -> Self {
        let mut apu = SccApu {
            oscs: [Osc::default(); OSC_COUNT as usize],
            // (upstream leaves these uninitialized: reset() sets them)
            last_time: 0,
            regs: [0; REG_COUNT as usize],
            synth: BlipSynth::new(BLIP_MED_QUALITY, 1),
        };
        apu.output(None);
        apu
    }

    /// Set overall volume (default is 1.0)
    pub fn volume(&mut self, v: f64) {
        self.synth
            .volume(0.43 / OSC_COUNT as f64 / AMP_RANGE as f64 * v);
    }

    /// Set treble equalization (see documentation)
    pub fn treble_eq(&mut self, eq: &BlipEq) {
        self.synth.treble_eq(eq);
    }

    pub fn osc_output(&mut self, index: i32, b: Option<BufId>) {
        // assert( (unsigned) index < osc_count );
        self.oscs[index as usize].output = b;
    }

    /// Write to register at specified time
    pub fn write(&mut self, bufs: &mut [BlipBuffer], time: BlipTime, addr: i32, data: i32) {
        // assert( (unsigned) addr < reg_count );
        self.run_until(bufs, time);
        self.regs[addr as usize] = data as u8;
    }

    /// Run sound to specified time, end current time frame, then start a new
    /// time frame at time 0. Time frames have no effect on emulation and each
    /// can be whatever length is convenient.
    pub fn end_frame(&mut self, bufs: &mut [BlipBuffer], end_time: BlipTime) {
        if end_time > self.last_time {
            self.run_until(bufs, end_time);
        }
        self.last_time -= end_time;
        // assert( last_time >= 0 );
    }

    /// Set buffer to generate all sound into, or disable sound if NULL
    pub fn output(&mut self, buf: Option<BufId>) {
        for i in 0..OSC_COUNT as usize {
            self.oscs[i].output = buf;
        }
    }

    /// Reset sound chip
    pub fn reset(&mut self) {
        self.last_time = 0;

        for osc in self.oscs.iter_mut() {
            osc.delay = 0;
            osc.phase = 0;
            osc.last_amp = 0;
        }
        self.regs.fill(0);
    }

    /// `wave [i]`, the oscillator's wave starting at register `base`
    #[inline]
    fn wave(&self, base: i32, i: i32) -> i32 {
        self.regs[(base + i) as usize] as i8 as i32
    }

    fn run_until(&mut self, bufs: &mut [BlipBuffer], end_time: BlipTime) {
        for index in 0..OSC_COUNT {
            let Some(out) = self.oscs[index as usize].output else {
                continue;
            };
            let output = &mut bufs[out];
            output.set_modified();

            let period: BlipTime = (self.regs[(0x80 + index * 2 + 1) as usize] & 0x0F) as BlipTime
                * 0x100
                + self.regs[(0x80 + index * 2) as usize] as BlipTime
                + 1;

            let mut volume = 0;
            if self.regs[0x8F] & (1 << index) != 0 {
                let inaudible_period: BlipTime =
                    ((output.clock_rate() as i64 + (INAUDIBLE_FREQ * 32) as i64) as u32
                        / (INAUDIBLE_FREQ * 16)) as BlipTime;
                if period > inaudible_period {
                    volume = (self.regs[(0x8A + index) as usize] & 0x0F) as i32
                        * (AMP_RANGE / 256 / 15) as i32;
                }
            }

            let mut wave = index * WAVE_SIZE;
            if index == OSC_COUNT - 1 {
                wave -= WAVE_SIZE; // last two oscs share wave
            }

            let mut osc = self.oscs[index as usize];
            {
                let amp = self.wave(wave, osc.phase) * volume;
                let delta = amp - osc.last_amp;
                if delta != 0 {
                    osc.last_amp = amp;
                    self.synth.offset(self.last_time, delta, output);
                }
            }

            let mut time: BlipTime = self.last_time + osc.delay;
            if time < end_time {
                if volume == 0 {
                    // maintain phase
                    let count: i32 = (end_time - time + period - 1) / period;
                    osc.phase = (osc.phase + count) & (WAVE_SIZE - 1);
                    time += count * period;
                } else {
                    let mut phase = osc.phase;
                    let mut last_wave = self.wave(wave, phase);
                    phase = (phase + 1) & (WAVE_SIZE - 1); // pre-advance for optimal inner loop
                    loop {
                        let amp = self.wave(wave, phase);
                        phase = (phase + 1) & (WAVE_SIZE - 1);
                        let delta = amp - last_wave;
                        if delta != 0 {
                            last_wave = amp;
                            self.synth.offset(time, delta * volume, output);
                        }
                        time += period;
                        if time >= end_time {
                            break;
                        }
                    }

                    phase = (phase - 1) & (WAVE_SIZE - 1); // undo pre-advance
                    osc.phase = phase;
                    osc.last_amp = self.wave(wave, phase) * volume;
                }
            }
            osc.delay = time - end_time;
            self.oscs[index as usize] = osc;
        }
        self.last_time = end_time;
    }
}
