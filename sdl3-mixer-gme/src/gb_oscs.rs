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

//! Private oscillators used by Gb_Apu (`Gb_Oscs.h`, `Gb_Oscs.cpp`).
//!
//! Upstream's oscillators point at their five registers in the APU's
//! register array and at their synth; here the APU passes them in.

use crate::blip_buffer::{BlipBuffer, BlipSynth, BlipTime, BufId};

pub const TRIGGER: i32 = 0x80;
pub const LEN_ENABLED_MASK: u8 = 0x40;

/// `Gb_Osc`
#[derive(Clone, Debug, Default)]
pub struct GbOsc {
    pub outputs: [Option<BufId>; 4], // NULL, right, left, center
    pub output: Option<BufId>,
    pub output_select: i32,

    pub delay: i32,
    pub last_amp: i32,
    pub volume: i32,
    pub length: i32,
    pub enabled: i32,
}

impl GbOsc {
    pub fn frequency(regs: &[u8]) -> i32 {
        (regs[4] & 7) as i32 * 0x100 + regs[3] as i32
    }

    // Gb_Osc

    pub fn reset(&mut self) {
        self.delay = 0;
        self.last_amp = 0;
        self.length = 0;
        self.output_select = 3;
        self.output = self.outputs[self.output_select as usize];
    }

    pub fn clock_length(&mut self, regs: &[u8]) {
        if (regs[4] & LEN_ENABLED_MASK) != 0 && self.length != 0 {
            self.length -= 1;
        }
    }
}

/// `Gb_Env`
#[derive(Clone, Debug, Default)]
pub struct GbEnv {
    pub osc: GbOsc,
    pub env_delay: i32,
}

impl GbEnv {
    pub fn reset(&mut self) {
        self.env_delay = 0;
        self.osc.reset();
    }

    // Gb_Env

    pub fn clock_envelope(&mut self, regs: &[u8]) {
        if self.env_delay != 0 && {
            self.env_delay -= 1;
            self.env_delay == 0
        } {
            self.env_delay = (regs[2] & 7) as i32;
            let v = self.osc.volume - 1 + (regs[2] >> 2 & 2) as i32;
            if (v as u32) < 15 {
                self.osc.volume = v;
            }
        }
    }

    pub fn write_register(&mut self, regs: &[u8], reg: i32, data: i32) -> bool {
        match reg {
            1 => {
                self.osc.length = 64 - (regs[1] & 0x3F) as i32;
            }

            2 => {
                if data >> 4 == 0 {
                    self.osc.enabled = false as i32;
                }
            }

            4 => {
                if data & TRIGGER != 0 {
                    self.env_delay = (regs[2] & 7) as i32;
                    self.osc.volume = (regs[2] >> 4) as i32;
                    self.osc.enabled = true as i32;
                    if self.osc.length == 0 {
                        self.osc.length = 64;
                    }
                    return true;
                }
            }
            _ => {}
        }
        false
    }
}

pub const PERIOD_MASK: u8 = 0x70;
pub const SHIFT_MASK: u8 = 0x07;

/// `Gb_Square` (its synth is `Blip_Synth<blip_good_quality,1>`)
#[derive(Clone, Debug, Default)]
pub struct GbSquare {
    pub env: GbEnv,
    pub sweep_delay: i32,
    pub sweep_freq: i32,
    pub phase: i32,
}

impl GbSquare {
    // Gb_Square

    pub fn reset(&mut self) {
        self.phase = 0;
        self.sweep_freq = 0;
        self.sweep_delay = 0;
        self.env.reset();
    }

    pub fn clock_sweep(&mut self, regs: &mut [u8]) {
        let sweep_period = ((regs[0] & PERIOD_MASK) >> 4) as i32;
        if sweep_period != 0 && self.sweep_delay != 0 && {
            self.sweep_delay -= 1;
            self.sweep_delay == 0
        } {
            self.sweep_delay = sweep_period;
            regs[3] = (self.sweep_freq & 0xFF) as u8;
            regs[4] = (regs[4] & !0x07) | (self.sweep_freq >> 8 & 0x07) as u8;

            let mut offset = self.sweep_freq >> (regs[0] & SHIFT_MASK);
            if regs[0] & 0x08 != 0 {
                offset = -offset;
            }
            self.sweep_freq += offset;

            if self.sweep_freq < 0 {
                self.sweep_freq = 0;
            } else if self.sweep_freq >= 2048 {
                self.sweep_delay = 0; // don't modify channel frequency any further
                self.sweep_freq = 2048; // silence sound immediately
            }
        }
    }

    pub fn run(
        &mut self,
        synth: &BlipSynth,
        bufs: &mut [BlipBuffer],
        regs: &[u8],
        mut time: BlipTime,
        end_time: BlipTime,
        mut playing: i32,
    ) {
        if self.sweep_freq == 2048 {
            playing = false as i32;
        }

        static TABLE: [u8; 4] = [1, 2, 4, 6];
        let duty = TABLE[(regs[1] >> 6) as usize] as i32;
        let osc = &mut self.env.osc;
        let mut amp = osc.volume & playing;
        if self.phase >= duty {
            amp = -amp;
        }

        let frequency = GbOsc::frequency(regs);
        if (frequency - 1) as u32 > 2040 {
            // frequency < 1 || frequency > 2041
            // really high frequency results in DC at half volume
            amp = osc.volume >> 1;
            playing = false as i32;
        }

        let out = osc.output.expect("output");
        {
            let delta = amp - osc.last_amp;
            if delta != 0 {
                osc.last_amp = amp;
                synth.offset(time, delta, &mut bufs[out]);
            }
        }

        time += osc.delay;
        if playing == 0 {
            time = end_time;
        }

        if time < end_time {
            let period = (2048 - frequency) * 4;
            let output = &mut bufs[out];
            let mut phase = self.phase;
            let mut delta = amp * 2;
            loop {
                phase = (phase + 1) & 7;
                if phase == 0 || phase == duty {
                    delta = -delta;
                    synth.offset_inline(time, delta, output);
                }
                time += period;
                if time >= end_time {
                    break;
                }
            }

            self.phase = phase;
            osc.last_amp = delta >> 1;
        }
        osc.delay = time - end_time;
    }
}

/// `Gb_Noise` (its synth is `Blip_Synth<blip_med_quality,1>`)
#[derive(Clone, Debug, Default)]
pub struct GbNoise {
    pub env: GbEnv,
    pub bits: u32,
}

impl GbNoise {
    // Gb_Noise

    pub fn run(
        &mut self,
        synth: &BlipSynth,
        bufs: &mut [BlipBuffer],
        regs: &[u8],
        mut time: BlipTime,
        end_time: BlipTime,
        playing: i32,
    ) {
        let osc = &mut self.env.osc;
        let mut amp = osc.volume & playing;
        let tap = 13 - (regs[3] & 8) as u32;
        if self.bits >> tap & 2 != 0 {
            amp = -amp;
        }

        let out = osc.output.expect("output");
        {
            let delta = amp - osc.last_amp;
            if delta != 0 {
                osc.last_amp = amp;
                synth.offset(time, delta, &mut bufs[out]);
            }
        }

        time += osc.delay;
        if playing == 0 {
            time = end_time;
        }

        if time < end_time {
            static TABLE: [u8; 8] = [8, 16, 32, 48, 64, 80, 96, 112];
            let period = (TABLE[(regs[3] & 7) as usize] as i32).wrapping_shl((regs[3] >> 4) as u32);

            // keep parallel resampled time to eliminate time conversion in the loop
            let output = &mut bufs[out];
            let resampled_period = output.resampled_duration(period);
            let mut resampled_time = output.resampled_time(time);
            let mut bits = self.bits;
            let mut delta = amp * 2;

            loop {
                let changed = (bits >> tap).wrapping_add(1);
                time = time.wrapping_add(period);
                bits <<= 1;
                if changed & 2 != 0 {
                    delta = -delta;
                    bits |= 1;
                    synth.offset_resampled(resampled_time, delta, output);
                }
                resampled_time = resampled_time.wrapping_add(resampled_period);
                if time >= end_time {
                    break;
                }
            }

            self.bits = bits;
            osc.last_amp = delta >> 1;
        }
        osc.delay = time.wrapping_sub(end_time);
    }
}

pub const WAVE_SIZE: usize = 32;

/// `Gb_Wave` (its synth is `Blip_Synth<blip_med_quality,1>`)
#[derive(Clone, Debug, Default)]
pub struct GbWave {
    pub osc: GbOsc,
    pub wave_pos: i32,
    pub wave: [u8; WAVE_SIZE],
}

impl GbWave {
    // Gb_Wave

    #[inline]
    pub fn write_register(&mut self, regs: &[u8], reg: i32, data: i32) {
        match reg {
            0 => {
                if data & 0x80 == 0 {
                    self.osc.enabled = false as i32;
                }
            }

            1 => {
                self.osc.length = 256 - regs[1] as i32;
            }

            2 => {
                self.osc.volume = data >> 5 & 3;
            }

            4 => {
                if data & TRIGGER & regs[0] as i32 != 0 {
                    self.wave_pos = 0;
                    self.osc.enabled = true as i32;
                    if self.osc.length == 0 {
                        self.osc.length = 256;
                    }
                }
            }
            _ => {}
        }
    }

    pub fn run(
        &mut self,
        synth: &BlipSynth,
        bufs: &mut [BlipBuffer],
        regs: &[u8],
        mut time: BlipTime,
        end_time: BlipTime,
        mut playing: i32,
    ) {
        let osc = &mut self.osc;
        let volume_shift = ((osc.volume - 1) & 7) as u32; // volume = 0 causes shift = 7
        let frequency;
        let out = osc.output.expect("output");
        {
            let mut amp =
                ((self.wave[self.wave_pos as usize] >> volume_shift) as i32 & playing) * 2;

            frequency = GbOsc::frequency(regs);
            if (frequency - 1) as u32 > 2044 {
                // frequency < 1 || frequency > 2045
                amp = 30 >> volume_shift & playing;
                playing = false as i32;
            }

            let delta = amp - osc.last_amp;
            if delta != 0 {
                osc.last_amp = amp;
                synth.offset(time, delta, &mut bufs[out]);
            }
        }

        time += osc.delay;
        if playing == 0 {
            time = end_time;
        }

        if time < end_time {
            let output = &mut bufs[out];
            let period = (2048 - frequency) * 2;
            let mut wave_pos = (self.wave_pos + 1) & (WAVE_SIZE as i32 - 1);

            loop {
                let amp = (self.wave[wave_pos as usize] >> volume_shift) as i32 * 2;
                wave_pos = (wave_pos + 1) & (WAVE_SIZE as i32 - 1);
                let delta = amp - osc.last_amp;
                if delta != 0 {
                    osc.last_amp = amp;
                    synth.offset_inline(time, delta, output);
                }
                time += period;
                if time >= end_time {
                    break;
                }
            }

            self.wave_pos = (wave_pos - 1) & (WAVE_SIZE as i32 - 1);
        }
        osc.delay = time - end_time;
    }
}
