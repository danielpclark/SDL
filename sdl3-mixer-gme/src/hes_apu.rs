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

//! Turbo Grafx 16 (PC Engine) PSG sound chip emulator (`Hes_Apu.h`,
//! `Hes_Apu.cpp`).

use crate::blip_buffer::{BlipBuffer, BlipEq, BlipSynth, BlipTime, BufId, BLIP_MED_QUALITY};

#[derive(Clone, Copy, Debug, Default)]
struct HesOsc {
    wave: [u8; 32],
    volume: [i16; 2],
    last_amp: [i32; 2],
    delay: i32,
    period: i32,
    noise: u8,
    phase: u8,
    balance: u8,
    dac: u8,
    last_time: BlipTime,

    outputs: [Option<BufId>; 2],
    chans: [Option<BufId>; 3],
    noise_lfsr: u32,
    control: u8,
}

const AMP_RANGE: u32 = 0x8000;

impl HesOsc {
    fn run_until(&mut self, synth_: &BlipSynth, bufs: &mut [BlipBuffer], end_time: BlipTime) {
        // cache often-used values
        if let (Some(osc_outputs_0), true) = (self.outputs[0], self.control & 0x80 != 0) {
            let mut dac = self.dac as i32;

            let volume_0 = self.volume[0] as i32;
            {
                let delta = dac * volume_0 - self.last_amp[0];
                if delta != 0 {
                    synth_.offset(self.last_time, delta, &mut bufs[osc_outputs_0]);
                }
                bufs[osc_outputs_0].set_modified();
            }

            let osc_outputs_1 = self.outputs[1];
            let volume_1 = self.volume[1] as i32;
            if let Some(o1) = osc_outputs_1 {
                let delta = dac * volume_1 - self.last_amp[1];
                if delta != 0 {
                    synth_.offset(self.last_time, delta, &mut bufs[o1]);
                }
                bufs[o1].set_modified();
            }

            let mut time: BlipTime = self.last_time + self.delay;
            if time < end_time {
                if self.noise & 0x80 != 0 {
                    if volume_0 | volume_1 != 0 {
                        // noise
                        let period = (32 - (self.noise & 0x1F) as i32) * 64; // TODO: correct?
                        let mut noise_lfsr = self.noise_lfsr;
                        loop {
                            let new_dac = (0x1F & (noise_lfsr >> 1 & 1).wrapping_neg()) as i32;
                            // Implemented using "Galios configuration"
                            // TODO: find correct LFSR algorithm
                            noise_lfsr =
                                (noise_lfsr >> 1) ^ (0xE008 & (noise_lfsr & 1).wrapping_neg());
                            //noise_lfsr = (noise_lfsr >> 1) ^ (0x6000 & -(noise_lfsr & 1));
                            let delta = new_dac - dac;
                            if delta != 0 {
                                dac = new_dac;
                                synth_.offset(time, delta * volume_0, &mut bufs[osc_outputs_0]);
                                if let Some(o1) = osc_outputs_1 {
                                    synth_.offset(time, delta * volume_1, &mut bufs[o1]);
                                }
                            }
                            time += period;
                            if time >= end_time {
                                break;
                            }
                        }

                        self.noise_lfsr = noise_lfsr;
                        // assert( noise_lfsr );
                    }
                } else if self.control & 0x40 == 0 {
                    // wave
                    let mut phase = (self.phase as i32 + 1) & 0x1F; // pre-advance for optimal inner loop
                    let mut period = self.period.wrapping_mul(2);
                    if period >= 14 && (volume_0 | volume_1) != 0 {
                        loop {
                            let new_dac = self.wave[phase as usize] as i32;
                            phase = (phase + 1) & 0x1F;
                            let delta = new_dac - dac;
                            if delta != 0 {
                                dac = new_dac;
                                synth_.offset(time, delta * volume_0, &mut bufs[osc_outputs_0]);
                                if let Some(o1) = osc_outputs_1 {
                                    synth_.offset(time, delta * volume_1, &mut bufs[o1]);
                                }
                            }
                            time += period;
                            if time >= end_time {
                                break;
                            }
                        }
                    } else {
                        if period == 0 {
                            // TODO: Gekisha Boy assumes that period = 0 silences wave
                            //period = 0x1000 * 2;
                            period = 1;
                            //if ( !(volume_0 | volume_1) )
                            //  debug_printf( "Used period 0\n" );
                        }
                        // maintain phase when silent
                        let count: i32 = end_time
                            .wrapping_sub(time)
                            .wrapping_add(period)
                            .wrapping_sub(1)
                            .wrapping_div(period);
                        phase = phase.wrapping_add(count); // phase will be masked below
                        time = time.wrapping_add(count.wrapping_mul(period));
                    }
                    self.phase = (phase.wrapping_sub(1) & 0x1F) as u8; // undo pre-advance
                }
            }
            time -= end_time;
            if time < 0 {
                time = 0;
            }
            self.delay = time;

            self.dac = dac as u8;
            self.last_amp[0] = dac * volume_0;
            self.last_amp[1] = dac * volume_1;
        }
        self.last_time = end_time;
    }
}

pub const OSC_COUNT: i32 = 6;

pub const START_ADDR: u32 = 0x0800;
pub const END_ADDR: u32 = 0x0809;

#[derive(Clone, Debug)]
pub struct HesApu {
    oscs: [HesOsc; OSC_COUNT as usize],
    latch: i32,
    balance: i32,
    synth: BlipSynth,
}

impl Default for HesApu {
    fn default() -> Self {
        Self::new()
    }
}

static CENTER_WAVES: bool = true; // reduces asymmetry and clamping when starting notes

// ~1.5 db per step
macro_rules! entry {
    ($factor:expr) => {
        ($factor * AMP_RANGE as f64 / 31.0 + 0.5) as i16
    };
}
static LOG_TABLE: [i16; 32] = [
    entry!(0.000000),
    entry!(0.005524),
    entry!(0.006570),
    entry!(0.007813),
    entry!(0.009291),
    entry!(0.011049),
    entry!(0.013139),
    entry!(0.015625),
    entry!(0.018581),
    entry!(0.022097),
    entry!(0.026278),
    entry!(0.031250),
    entry!(0.037163),
    entry!(0.044194),
    entry!(0.052556),
    entry!(0.062500),
    entry!(0.074325),
    entry!(0.088388),
    entry!(0.105112),
    entry!(0.125000),
    entry!(0.148651),
    entry!(0.176777),
    entry!(0.210224),
    entry!(0.250000),
    entry!(0.297302),
    entry!(0.353553),
    entry!(0.420448),
    entry!(0.500000),
    entry!(0.594604),
    entry!(0.707107),
    entry!(0.840896),
    entry!(1.000000),
];

impl HesApu {
    pub fn new() -> Self {
        let mut apu = HesApu {
            oscs: [HesOsc::default(); OSC_COUNT as usize],
            latch: 0,
            balance: 0,
            synth: BlipSynth::new(BLIP_MED_QUALITY, 1),
        };
        for osc in apu.oscs.iter_mut().rev() {
            osc.outputs = [None; 2];
            osc.chans = [None; 3];
        }

        apu.reset();
        apu
    }

    pub fn treble_eq(&mut self, eq: &BlipEq) {
        self.synth.treble_eq(eq);
    }

    pub fn volume(&mut self, v: f64) {
        self.synth
            .volume(1.8 / OSC_COUNT as f64 / AMP_RANGE as f64 * v);
    }

    pub fn reset(&mut self) {
        self.latch = 0;
        self.balance = 0xFF;

        for osc in self.oscs.iter_mut().rev() {
            // memset( osc, 0, offsetof (Hes_Osc,outputs) );
            osc.wave = [0; 32];
            osc.volume = [0; 2];
            osc.last_amp = [0; 2];
            osc.delay = 0;
            osc.period = 0;
            osc.noise = 0;
            osc.phase = 0;
            osc.balance = 0;
            osc.dac = 0;
            osc.last_time = 0;
            osc.noise_lfsr = 1;
            osc.control = 0x40;
            osc.balance = 0xFF;
        }
    }

    pub fn osc_output(
        &mut self,
        index: i32,
        center: Option<BufId>,
        left: Option<BufId>,
        right: Option<BufId>,
    ) {
        // require( (unsigned) index < osc_count );
        let osc = &mut self.oscs[index as usize];
        osc.chans[0] = center;
        osc.chans[1] = left;
        osc.chans[2] = right;

        for i in (0..OSC_COUNT as usize).rev() {
            self.balance_changed(i);
        }
    }

    fn balance_changed(&mut self, i: usize) {
        let balance = self.balance;
        let osc = &mut self.oscs[i];
        let vol = (osc.control & 0x1F) as i32 - 0x1E * 2;

        let mut left = vol + (osc.balance as i32 >> 3 & 0x1E) + (balance >> 3 & 0x1E);
        if left < 0 {
            left = 0;
        }

        let mut right = vol + ((osc.balance as i32) << 1 & 0x1E) + (balance << 1 & 0x1E);
        if right < 0 {
            right = 0;
        }

        let left = LOG_TABLE[left as usize] as i32;
        let right = LOG_TABLE[right as usize] as i32;

        // optimizing for the common case of being centered also allows easy
        // panning using Effects_Buffer
        osc.outputs[0] = osc.chans[0]; // center
        osc.outputs[1] = None;
        if left != right {
            osc.outputs[0] = osc.chans[1]; // left
            osc.outputs[1] = osc.chans[2]; // right
        }

        if CENTER_WAVES {
            osc.last_amp[0] = osc.last_amp[0].wrapping_add((left - osc.volume[0] as i32) * 16);
            osc.last_amp[1] = osc.last_amp[1].wrapping_add((right - osc.volume[1] as i32) * 16);
        }

        osc.volume[0] = left as i16;
        osc.volume[1] = right as i16;
    }

    pub fn write_data(&mut self, bufs: &mut [BlipBuffer], time: BlipTime, addr: i32, data: i32) {
        if addr == 0x800 {
            self.latch = data & 7;
        } else if addr == 0x801 {
            if self.balance != data {
                self.balance = data;

                for i in (0..OSC_COUNT as usize).rev() {
                    self.oscs[i].run_until(&self.synth, bufs, time);
                    // FIXME (upstream): this updates the first oscillator's
                    // balance every time, not this one's
                    self.balance_changed(0);
                }
            }
        } else if self.latch < OSC_COUNT {
            let latch = self.latch as usize;
            self.oscs[latch].run_until(&self.synth, bufs, time);
            let osc = &mut self.oscs[latch];
            let mut data = data;
            match addr {
                0x802 => {
                    osc.period = (osc.period & 0xF00) | data;
                }

                0x803 => {
                    osc.period = (osc.period & 0x0FF) | ((data & 0x0F) << 8);
                }

                0x804 => {
                    if osc.control as i32 & 0x40 & !data != 0 {
                        osc.phase = 0;
                    }
                    osc.control = data as u8;
                    self.balance_changed(latch);
                }

                0x805 => {
                    osc.balance = data as u8;
                    self.balance_changed(latch);
                }

                0x806 => {
                    data &= 0x1F;
                    if osc.control & 0x40 == 0 {
                        osc.wave[osc.phase as usize] = data as u8;
                        osc.phase = (osc.phase + 1) & 0x1F;
                    } else if osc.control & 0x80 != 0 {
                        osc.dac = data as u8;
                    }
                }

                0x807 => {
                    if latch >= 4 {
                        osc.noise = data as u8;
                    }
                }

                0x809 => {
                    // if ( !(data & 0x80) && (data & 0x03) != 0 )
                    //  debug_printf( "HES LFO not supported\n" );
                }
                _ => {}
            }
        }
    }

    pub fn end_frame(&mut self, bufs: &mut [BlipBuffer], end_time: BlipTime) {
        for osc in self.oscs.iter_mut().rev() {
            if end_time > osc.last_time {
                osc.run_until(&self.synth, bufs, end_time);
            }
            // assert( osc->last_time >= end_time );
            osc.last_time -= end_time;
        }
    }
}
