// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/

/* Copyright (C) 2003-2006 Shay Green. This module is free software; you
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

//! Sunsoft FME-7 sound emulator (`Nes_Fme7_Apu.h`, `Nes_Fme7_Apu.cpp`;
//! the state saving functions, which nothing calls, are left out).

use crate::blip_buffer::{BlipBuffer, BlipEq, BlipSynth, BlipTime, BufId, BLIP_GOOD_QUALITY};

pub const OSC_COUNT: i32 = 3;
const REG_COUNT: usize = 14;

// Mask and addresses of registers
pub const ADDR_MASK: u32 = 0xE000;
pub const DATA_ADDR: u32 = 0xE000;
pub const LATCH_ADDR: u32 = 0xC000;

/// can be any value; this gives best error/quality tradeoff
const AMP_RANGE: u32 = 192;

const fn entry(n: f64) -> u8 {
    (n * AMP_RANGE as f64 + 0.5) as u8
}

static AMP_TABLE: [u8; 16] = [
    entry(0.0000),
    entry(0.0078),
    entry(0.0110),
    entry(0.0156),
    entry(0.0221),
    entry(0.0312),
    entry(0.0441),
    entry(0.0624),
    entry(0.0883),
    entry(0.1249),
    entry(0.1766),
    entry(0.2498),
    entry(0.3534),
    entry(0.4998),
    entry(0.7070),
    entry(1.0000),
];

#[derive(Clone, Copy, Debug, Default)]
struct Osc {
    output: Option<BufId>,
    last_amp: i32,
}

/// `Nes_Fme7_Apu` (with its `fme7_apu_state_t`)
#[derive(Clone, Debug)]
pub struct NesFme7Apu {
    // fme7_apu_state_t
    regs: [u8; REG_COUNT],
    phases: [u8; 3], // 0 or 1
    latch: u8,
    delays: [u16; 3], // a, b, c

    oscs: [Osc; 3],
    last_time: BlipTime,
    synth: BlipSynth,
}

impl Default for NesFme7Apu {
    fn default() -> Self {
        Self::new()
    }
}

impl NesFme7Apu {
    pub fn new() -> Self {
        let mut a = NesFme7Apu {
            regs: [0; REG_COUNT],
            phases: [0; 3],
            latch: 0,
            delays: [0; 3],
            oscs: [Osc::default(); 3],
            last_time: 0,
            synth: BlipSynth::new(BLIP_GOOD_QUALITY, 1),
        };
        a.output(None);
        a.volume(1.0);
        a.reset();
        a
    }

    pub fn reset(&mut self) {
        self.last_time = 0;

        for o in self.oscs.iter_mut() {
            o.last_amp = 0;
        }

        // memset( state, 0, sizeof *state );
        self.regs = [0; REG_COUNT];
        self.phases = [0; 3];
        self.latch = 0;
        self.delays = [0; 3];
    }

    pub fn volume(&mut self, v: f64) {
        self.synth.volume(0.38 / AMP_RANGE as f64 * v); // to do: fine-tune
    }

    pub fn treble_eq(&mut self, eq: &BlipEq) {
        self.synth.treble_eq(eq);
    }

    pub fn osc_output(&mut self, i: i32, buf: Option<BufId>) {
        // assert( (unsigned) i < osc_count );
        self.oscs[i as usize].output = buf;
    }

    pub fn output(&mut self, buf: Option<BufId>) {
        for i in 0..OSC_COUNT {
            self.osc_output(i, buf);
        }
    }

    /// (addr & addr_mask) == latch_addr
    pub fn write_latch(&mut self, data: i32) {
        self.latch = data as u8;
    }

    /// (addr & addr_mask) == data_addr
    pub fn write_data(&mut self, bufs: &mut [BlipBuffer], time: BlipTime, data: i32) {
        if self.latch as usize >= REG_COUNT {
            // debug_printf( "FME7 write to %02X (past end of sound registers)\n", (int) latch );
            return;
        }
        self.run_until(bufs, time);
        self.regs[self.latch as usize] = data as u8;
    }

    pub fn end_frame(&mut self, bufs: &mut [BlipBuffer], time: BlipTime) {
        if time > self.last_time {
            self.run_until(bufs, time);
        }

        // assert( last_time >= time );
        self.last_time -= time;
    }

    fn run_until(&mut self, bufs: &mut [BlipBuffer], end_time: BlipTime) {
        // require( end_time >= last_time );

        for index in 0..OSC_COUNT as usize {
            let mode = self.regs[7] as i32 >> index;
            let vol_mode = self.regs[0o10 + index] as i32;
            let mut volume = AMP_TABLE[(vol_mode & 0x0F) as usize] as i32;

            let Some(out) = self.oscs[index].output else {
                continue;
            };
            let osc_output = &mut bufs[out];
            osc_output.set_modified();

            // check for unsupported mode
            // (debug_printf in debug builds: FME7 used unimplemented sound mode)

            if (mode & 0o01) | (vol_mode & 0x10) != 0 {
                volume = 0; // noise and envelope aren't supported
            }

            // period
            let period_factor = 16u32;
            let mut period = (self.regs[index * 2 + 1] as u32 & 0x0F) * 0x100 * period_factor
                + self.regs[index * 2] as u32 * period_factor;
            if period < 50 {
                // around 22 kHz
                volume = 0;
                if period == 0 {
                    // on my AY-3-8910A, period doesn't have extra one added
                    period = period_factor;
                }
            }

            // current amplitude
            let mut amp = volume;
            if self.phases[index] == 0 {
                amp = 0;
            }
            {
                let delta = amp - self.oscs[index].last_amp;
                if delta != 0 {
                    self.oscs[index].last_amp = amp;
                    self.synth.offset(self.last_time, delta, osc_output);
                }
            }

            let mut time = self.last_time + self.delays[index] as i32;
            if time < end_time {
                let mut delta = amp * 2 - volume;
                if volume != 0 {
                    loop {
                        delta = -delta;
                        self.synth.offset_inline(time, delta, osc_output);
                        time = (time as u32).wrapping_add(period) as i32;
                        if time >= end_time {
                            break;
                        }
                    }

                    self.oscs[index].last_amp = (delta + volume) >> 1;
                    self.phases[index] = (delta > 0) as u8;
                } else {
                    // maintain phase when silent
                    let count = ((end_time - time) as u32 + period - 1) / period;
                    self.phases[index] ^= (count & 1) as u8;
                    time = (time as u32).wrapping_add(count.wrapping_mul(period)) as i32;
                }
            }

            self.delays[index] = (time - end_time) as u16;
        }

        self.last_time = end_time;
    }
}
