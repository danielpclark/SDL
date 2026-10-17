// Gb_Snd_Emu 0.1.5. http://www.slack.net/~ant/

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

//! Nintendo Game Boy PAPU sound chip emulator (`Gb_Apu.h`, `Gb_Apu.cpp`,
//! with `Gb_Apu::write_osc()` from `Gb_Oscs.cpp`).

use crate::blip_buffer::{
    BlipBuffer, BlipEq, BlipSynth, BlipTime, BufId, BLIP_GOOD_QUALITY, BLIP_MED_QUALITY,
};
use crate::gb_oscs::*;

// Assign single oscillator output to buffer(s). Valid indicies are 0 to 3,
// which refer to Square 1, Square 2, Wave, and Noise. If buffer is NULL,
// silences oscillator.
pub const OSC_COUNT: i32 = 4;

// Reads and writes at addr must satisfy start_addr <= addr <= end_addr
pub const START_ADDR: u32 = 0xFF10;
pub const END_ADDR: u32 = 0xFF3F;
pub const REGISTER_COUNT: u32 = END_ADDR - START_ADDR + 1;

const VOL_REG: u32 = 0xFF24;
const STATUS_REG: u32 = 0xFF26;

#[derive(Clone, Debug)]
pub struct GbApu {
    next_frame_time: BlipTime,
    last_time: BlipTime,
    frame_period: BlipTime,
    volume_unit: f64,
    frame_count: i32,

    pub square1: GbSquare,
    pub square2: GbSquare,
    pub wave: GbWave,
    pub noise: GbNoise,
    regs: [u8; REGISTER_COUNT as usize],
    square_synth: BlipSynth, // used by squares
    other_synth: BlipSynth,  // used by wave and noise
}

impl Default for GbApu {
    fn default() -> Self {
        Self::new()
    }
}

static POWERUP_REGS: [u8; 0x20] = [
    0x80, 0x3F, 0x00, 0xFF, 0xBF, // square 1
    0xFF, 0x3F, 0x00, 0xFF, 0xBF, // square 2
    0x7F, 0xFF, 0x9F, 0xFF, 0xBF, // wave
    0xFF, 0xFF, 0x00, 0x00, 0xBF, // noise
    0x00, // left/right enables
    0x77, // master volume
    0x80, // power
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
];

impl GbApu {
    pub fn new() -> Self {
        let mut apu = GbApu {
            next_frame_time: 0,
            last_time: 0,
            frame_period: 0,
            volume_unit: 0.0,
            frame_count: 0,
            // (the oscillators' outputs are cleared; upstream leaves the
            // rest of their state, which reset() and the register writes
            // set before use, uninitialized)
            square1: GbSquare::default(),
            square2: GbSquare::default(),
            wave: GbWave::default(),
            noise: GbNoise::default(),
            regs: [0; REGISTER_COUNT as usize],
            square_synth: BlipSynth::new(BLIP_GOOD_QUALITY, 1),
            other_synth: BlipSynth::new(BLIP_MED_QUALITY, 1),
        };

        apu.set_tempo(1.0);
        apu.volume(1.0);
        // (no outputs yet, so no buffers)
        apu.reset(&mut []);
        apu
    }

    /// `oscs [i]`
    fn osc(&mut self, i: i32) -> &mut GbOsc {
        match i {
            0 => &mut self.square1.env.osc,
            1 => &mut self.square2.env.osc,
            2 => &mut self.wave.osc,
            _ => &mut self.noise.env.osc,
        }
    }

    /// Set overall volume of all oscillators, where 1.0 is full volume
    pub fn volume(&mut self, vol: f64) {
        self.volume_unit =
            0.60 / OSC_COUNT as f64 / 15.0 /*steps*/ / 2.0 /*?*/ / 8.0 /*master vol range*/ * vol;
        self.update_volume();
    }

    /// Set treble equalization
    pub fn treble_eq(&mut self, eq: &BlipEq) {
        self.square_synth.treble_eq(eq);
        self.other_synth.treble_eq(eq);
    }

    /// Assign single oscillator output to buffer(s).
    pub fn osc_output(
        &mut self,
        index: i32,
        center: Option<BufId>,
        left: Option<BufId>,
        right: Option<BufId>,
    ) {
        // require( (unsigned) index < osc_count );
        // require( (center && left && right) || (!center && !left && !right) );
        let osc = self.osc(index);
        osc.outputs[1] = right;
        osc.outputs[2] = left;
        osc.outputs[3] = center;
        osc.output = osc.outputs[osc.output_select as usize];
    }

    /// Assign all oscillator outputs to specified buffer(s). If buffer
    /// is NULL, silences all oscillators.
    pub fn output(&mut self, center: Option<BufId>, left: Option<BufId>, right: Option<BufId>) {
        for i in 0..OSC_COUNT {
            self.osc_output(i, center, left, right);
        }
    }

    fn update_volume(&mut self) {
        // TODO: doesn't handle differing left/right global volume (support would
        // require modification to all oscillator code)
        let data = self.regs[(VOL_REG - START_ADDR) as usize] as i32;
        let vol = ((data & 7).max(data >> 4 & 7) + 1) as f64 * self.volume_unit;
        self.square_synth.volume(vol);
        self.other_synth.volume(vol);
    }

    pub fn set_tempo(&mut self, t: f64) {
        self.frame_period = 4194304 / 256; // 256 Hz
        if t != 1.0 {
            self.frame_period = (self.frame_period as f64 / t) as BlipTime;
        }
    }

    /// Reset oscillators and internal state
    pub fn reset(&mut self, bufs: &mut [BlipBuffer]) {
        self.next_frame_time = 0;
        self.last_time = 0;
        self.frame_count = 0;

        self.square1.reset();
        self.square2.reset();
        self.wave.osc.reset();
        self.noise.env.reset();
        self.noise.bits = 1;
        self.wave.wave_pos = 0;

        // avoid click at beginning
        self.regs[(VOL_REG - START_ADDR) as usize] = 0x77;
        self.update_volume();

        self.regs[(STATUS_REG - START_ADDR) as usize] = 0x01; // force power
        self.write_register(bufs, 0, STATUS_REG, 0x00);

        static INITIAL_WAVE: [u8; 16] = [
            0x84, 0x40, 0x43, 0xAA, 0x2D, 0x78, 0x92, 0x3C, // wave table
            0x60, 0x59, 0x59, 0xB0, 0x34, 0xB8, 0x2E, 0xDA,
        ];
        self.wave.wave[..INITIAL_WAVE.len()].copy_from_slice(&INITIAL_WAVE);
    }

    fn run_until(&mut self, bufs: &mut [BlipBuffer], end_time: BlipTime) {
        // require( end_time >= last_time ); // end_time must not be before previous time
        if end_time == self.last_time {
            return;
        }

        loop {
            let mut time = self.next_frame_time;
            if time > end_time {
                time = end_time;
            }

            // run oscillators
            for i in 0..OSC_COUNT {
                let regs = &self.regs[(i * 5) as usize..(i * 5 + 5) as usize];
                let osc = match i {
                    0 => &self.square1.env.osc,
                    1 => &self.square2.env.osc,
                    2 => &self.wave.osc,
                    _ => &self.noise.env.osc,
                };
                if let Some(out) = osc.output {
                    bufs[out].set_modified(); // TODO: misses optimization opportunities?
                    let mut playing = false as i32;
                    if osc.enabled != 0
                        && osc.volume != 0
                        && ((regs[4] & LEN_ENABLED_MASK) == 0 || osc.length != 0)
                    {
                        playing = -1;
                    }
                    let last_time = self.last_time;
                    match i {
                        0 => self.square1.run(
                            &self.square_synth,
                            bufs,
                            regs,
                            last_time,
                            time,
                            playing,
                        ),
                        1 => self.square2.run(
                            &self.square_synth,
                            bufs,
                            regs,
                            last_time,
                            time,
                            playing,
                        ),
                        2 => self
                            .wave
                            .run(&self.other_synth, bufs, regs, last_time, time, playing),
                        _ => {
                            self.noise
                                .run(&self.other_synth, bufs, regs, last_time, time, playing)
                        }
                    }
                }
            }
            self.last_time = time;

            if time == end_time {
                break;
            }

            self.next_frame_time += self.frame_period;

            // 256 Hz actions
            self.square1.env.osc.clock_length(&self.regs[0..5]);
            self.square2.env.osc.clock_length(&self.regs[5..10]);
            self.wave.osc.clock_length(&self.regs[10..15]);
            self.noise.env.osc.clock_length(&self.regs[15..20]);

            self.frame_count = (self.frame_count + 1) & 3;
            if self.frame_count == 0 {
                // 64 Hz actions
                self.square1.env.clock_envelope(&self.regs[0..5]);
                self.square2.env.clock_envelope(&self.regs[5..10]);
                self.noise.env.clock_envelope(&self.regs[15..20]);
            }

            if self.frame_count & 1 != 0 {
                self.square1.clock_sweep(&mut self.regs[0..5]); // 128 Hz action
            }
        }
    }

    /// Run all oscillators up to specified time, end current time frame, then
    /// start a new frame at time 0.
    pub fn end_frame(&mut self, bufs: &mut [BlipBuffer], end_time: BlipTime) {
        if end_time > self.last_time {
            self.run_until(bufs, end_time);
        }

        // assert( next_frame_time >= end_time );
        self.next_frame_time -= end_time;

        // assert( last_time >= end_time );
        self.last_time -= end_time;
    }

    /// Write 'data' to address at specified time
    pub fn write_register(
        &mut self,
        bufs: &mut [BlipBuffer],
        time: BlipTime,
        addr: u32,
        data: i32,
    ) {
        // require( (unsigned) data < 0x100 );

        let reg = addr.wrapping_sub(START_ADDR) as i32;
        if reg as u32 >= REGISTER_COUNT {
            return;
        }

        self.run_until(bufs, time);

        let old_reg = self.regs[reg as usize] as i32;
        self.regs[reg as usize] = data as u8;

        if addr < VOL_REG {
            self.write_osc(bufs, reg / 5, reg, data);
        } else if addr == VOL_REG && data != old_reg {
            // global volume
            // return all oscs to 0
            for i in 0..OSC_COUNT {
                let osc = self.osc(i);
                let amp = osc.last_amp;
                osc.last_amp = 0;
                if amp != 0 && osc.enabled != 0 {
                    if let Some(out) = osc.output {
                        self.other_synth.offset(time, -amp, &mut bufs[out]);
                    }
                }
            }

            if let Some(out) = self.wave.osc.outputs[3] {
                self.other_synth.offset(time, 30, &mut bufs[out]);
            }

            self.update_volume();

            if let Some(out) = self.wave.osc.outputs[3] {
                self.other_synth.offset(time, -30, &mut bufs[out]);
            }

            // oscs will update with new amplitude when next run
        } else if addr == 0xFF25 || addr == STATUS_REG {
            let mask: i32 = if self.regs[(STATUS_REG - START_ADDR) as usize] & 0x80 != 0 {
                !0
            } else {
                0
            };
            let flags = self.regs[(0xFF25 - START_ADDR) as usize] as i32 & mask;

            // left/right assignments
            for i in 0..OSC_COUNT {
                let osc = self.osc(i);
                osc.enabled &= mask;
                let bits = flags >> i;
                let old_output = osc.output;
                osc.output_select = (bits >> 3 & 2) | (bits & 1);
                osc.output = osc.outputs[osc.output_select as usize];
                if osc.output != old_output {
                    let amp = osc.last_amp;
                    osc.last_amp = 0;
                    if amp != 0 {
                        if let Some(old) = old_output {
                            self.other_synth.offset(time, -amp, &mut bufs[old]);
                        }
                    }
                }
            }

            if addr == STATUS_REG && data != old_reg {
                if data & 0x80 == 0 {
                    for i in 0..POWERUP_REGS.len() as u32 {
                        if i != STATUS_REG - START_ADDR {
                            self.write_register(
                                bufs,
                                time,
                                i + START_ADDR,
                                POWERUP_REGS[i as usize] as i32,
                            );
                        }
                    }
                } else {
                    //debug_printf( "APU powered on\n" );
                }
            }
        } else if addr >= 0xFF30 {
            let index = ((addr & 0x0F) * 2) as usize;
            self.wave.wave[index] = (data >> 4) as u8;
            self.wave.wave[index + 1] = (data & 0x0F) as u8;
        }
    }

    /// Read from address at specified time
    pub fn read_register(&mut self, bufs: &mut [BlipBuffer], time: BlipTime, addr: u32) -> i32 {
        self.run_until(bufs, time);

        let index = addr.wrapping_sub(START_ADDR) as i32;
        // require( (unsigned) index < register_count );
        let mut data = self.regs[index as usize] as i32;

        if addr == STATUS_REG {
            data = (data & 0x80) | 0x70;
            for i in 0..OSC_COUNT {
                let regs4 = self.regs[(i * 5 + 4) as usize];
                let osc = self.osc(i);
                if osc.enabled != 0 && (osc.length != 0 || (regs4 & LEN_ENABLED_MASK) == 0) {
                    data |= 1 << i;
                }
            }
        }

        data
    }

    // Gb_Apu::write_osc

    fn write_osc(&mut self, bufs: &mut [BlipBuffer], index: i32, mut reg: i32, data: i32) {
        let _ = bufs;
        reg -= index * 5;
        let regs = &self.regs[(index * 5) as usize..(index * 5 + 5) as usize];
        match index {
            0 | 1 => {
                let sq = if index == 0 {
                    &mut self.square1
                } else {
                    &mut self.square2
                };
                if sq.env.write_register(regs, reg, data) && index == 0 {
                    self.square1.sweep_freq = GbOsc::frequency(&self.regs[0..5]);
                    if (self.regs[0] & PERIOD_MASK) != 0 && (self.regs[0] & SHIFT_MASK) != 0 {
                        self.square1.sweep_delay = 1; // cause sweep to recalculate now
                        self.square1.clock_sweep(&mut self.regs[0..5]);
                    }
                }
            }

            2 => {
                self.wave.write_register(regs, reg, data);
            }

            _ => {
                if self.noise.env.write_register(regs, reg, data) {
                    self.noise.bits = 0x7FFF;
                }
            }
        }
    }
}
