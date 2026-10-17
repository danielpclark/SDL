// Nes_Snd_Emu 0.1.8. http://www.slack.net/~ant/

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

//! Private oscillators used by Nes_Apu (`Nes_Oscs.h`, `Nes_Oscs.cpp`).
//! The DMC's functions that use the APU it belongs to are the APU's
//! (`dmc_*` in [`crate::nes_apu`]).

use crate::blip_buffer::{BlipBuffer, BlipSynth, BufId, BLIP_GOOD_QUALITY, BLIP_MED_QUALITY};
use crate::nes_cpu::NesTime;

#[derive(Clone, Debug, Default)]
pub struct NesOsc {
    pub regs: [u8; 4],
    pub reg_written: [bool; 4],
    pub output: Option<BufId>,
    /// length counter (0 if unused by oscillator)
    pub length_counter: i32,
    /// delay until next (potential) transition
    pub delay: i32,
    /// last amplitude oscillator was outputting
    pub last_amp: i32,
}

impl NesOsc {
    pub fn clock_length(&mut self, halt_mask: i32) {
        if self.length_counter != 0 && (self.regs[0] as i32 & halt_mask) == 0 {
            self.length_counter -= 1;
        }
    }

    pub fn period(&self) -> i32 {
        (self.regs[3] as i32 & 7) * 0x100 + (self.regs[2] as i32 & 0xFF)
    }

    pub fn reset(&mut self) {
        self.delay = 0;
        self.last_amp = 0;
    }

    pub fn update_amp(&mut self, amp: i32) -> i32 {
        let delta = amp - self.last_amp;
        self.last_amp = amp;
        delta
    }
}

#[derive(Clone, Debug, Default)]
pub struct NesEnvelope {
    pub osc: NesOsc,
    pub envelope: i32,
    pub env_delay: i32,
}

impl NesEnvelope {
    pub fn clock_envelope(&mut self) {
        let period = self.osc.regs[0] as i32 & 15;
        if self.osc.reg_written[3] {
            self.osc.reg_written[3] = false;
            self.env_delay = period;
            self.envelope = 15;
        } else {
            self.env_delay -= 1;
            if self.env_delay < 0 {
                self.env_delay = period;
                if (self.envelope | (self.osc.regs[0] as i32 & 0x20)) != 0 {
                    self.envelope = (self.envelope - 1) & 15;
                }
            }
        }
    }

    pub fn volume(&self) -> i32 {
        if self.osc.length_counter == 0 {
            0
        } else if self.osc.regs[0] & 0x10 != 0 {
            self.osc.regs[0] as i32 & 15
        } else {
            self.envelope
        }
    }

    pub fn reset(&mut self) {
        self.envelope = 0;
        self.env_delay = 0;
        self.osc.reset();
    }
}

// Nes_Square

pub const SQUARE_NEGATE_FLAG: i32 = 0x08;
pub const SQUARE_SHIFT_MASK: i32 = 0x07;
pub const SQUARE_PHASE_RANGE: i32 = 8;

/// `Nes_Square::Synth`, shared between squares
pub fn new_square_synth() -> BlipSynth {
    BlipSynth::new(BLIP_GOOD_QUALITY, 1)
}

#[derive(Clone, Debug, Default)]
pub struct NesSquare {
    pub env: NesEnvelope,
    pub phase: i32,
    pub sweep_delay: i32,
}

impl NesSquare {
    pub fn reset(&mut self) {
        self.sweep_delay = 0;
        self.env.reset();
    }

    pub fn clock_sweep(&mut self, negative_adjust: i32) {
        let osc = &mut self.env.osc;
        let sweep = osc.regs[1] as i32;

        self.sweep_delay -= 1;
        if self.sweep_delay < 0 {
            osc.reg_written[1] = true;

            let mut period = osc.period();
            let shift = sweep & SQUARE_SHIFT_MASK;
            if shift != 0 && (sweep & 0x80) != 0 && period >= 8 {
                let mut offset = period >> shift;

                if sweep & SQUARE_NEGATE_FLAG != 0 {
                    offset = negative_adjust - offset;
                }

                if period + offset < 0x800 {
                    period += offset;
                    // rewrite period
                    osc.regs[2] = (period & 0xFF) as u8;
                    osc.regs[3] = ((osc.regs[3] as i32 & !7) | ((period >> 8) & 7)) as u8;
                }
            }
        }

        if osc.reg_written[1] {
            osc.reg_written[1] = false;
            self.sweep_delay = (sweep >> 4) & 7;
        }
    }

    // TODO: clean up
    #[inline]
    fn maintain_phase(
        &mut self,
        mut time: NesTime,
        end_time: NesTime,
        timer_period: NesTime,
    ) -> NesTime {
        let remain = end_time - time;
        if remain > 0 {
            let count = (remain + timer_period - 1) / timer_period;
            self.phase = (self.phase + count) & (SQUARE_PHASE_RANGE - 1);
            time = time.wrapping_add(count.wrapping_mul(timer_period));
        }
        time
    }

    pub fn run(
        &mut self,
        synth: &BlipSynth,
        bufs: &mut [BlipBuffer],
        mut time: NesTime,
        end_time: NesTime,
    ) {
        let period = self.env.osc.period();
        let timer_period = (period + 1) * 2;

        let Some(out) = self.env.osc.output else {
            let t = self.maintain_phase(time + self.env.osc.delay, end_time, timer_period);
            self.env.osc.delay = t - end_time;
            return;
        };
        let output = &mut bufs[out];

        output.set_modified();

        let mut offset = period >> (self.env.osc.regs[1] as i32 & SQUARE_SHIFT_MASK);
        if self.env.osc.regs[1] as i32 & SQUARE_NEGATE_FLAG != 0 {
            offset = 0;
        }

        let volume = self.env.volume();
        if volume == 0 || period < 8 || (period + offset) >= 0x800 {
            if self.env.osc.last_amp != 0 {
                synth.offset(time, -self.env.osc.last_amp, output);
                self.env.osc.last_amp = 0;
            }

            time += self.env.osc.delay;
            time = self.maintain_phase(time, end_time, timer_period);
        } else {
            // handle duty select
            let duty_select = (self.env.osc.regs[0] as i32 >> 6) & 3;
            let mut duty = 1 << duty_select; // 1, 2, 4, 2
            let mut amp = 0;
            if duty_select == 3 {
                duty = 2; // negated 25%
                amp = volume;
            }
            if self.phase < duty {
                amp ^= volume;
            }

            {
                let delta = self.env.osc.update_amp(amp);
                if delta != 0 {
                    synth.offset(time, delta, output);
                }
            }

            time += self.env.osc.delay;
            if time < end_time {
                let mut delta = amp * 2 - volume;
                let mut phase = self.phase;

                loop {
                    phase = (phase + 1) & (SQUARE_PHASE_RANGE - 1);
                    if phase == 0 || phase == duty {
                        delta = -delta;
                        synth.offset_inline(time, delta, output);
                    }
                    time += timer_period;
                    if time >= end_time {
                        break;
                    }
                }

                self.env.osc.last_amp = (delta + volume) >> 1;
                self.phase = phase;
            }
        }

        self.env.osc.delay = time - end_time;
    }
}

// Nes_Triangle

pub const TRIANGLE_PHASE_RANGE: i32 = 16;

#[derive(Clone, Debug)]
pub struct NesTriangle {
    pub osc: NesOsc,
    pub phase: i32,
    pub linear_counter: i32,
    pub synth: BlipSynth,
}

impl Default for NesTriangle {
    fn default() -> Self {
        NesTriangle {
            osc: NesOsc::default(),
            phase: 0,
            linear_counter: 0,
            synth: BlipSynth::new(BLIP_MED_QUALITY, 1),
        }
    }
}

impl NesTriangle {
    pub fn reset(&mut self) {
        self.linear_counter = 0;
        self.phase = 1;
        self.osc.reset();
    }

    pub fn clock_linear_counter(&mut self) {
        if self.osc.reg_written[3] {
            self.linear_counter = self.osc.regs[0] as i32 & 0x7F;
        } else if self.linear_counter != 0 {
            self.linear_counter -= 1;
        }

        if self.osc.regs[0] & 0x80 == 0 {
            self.osc.reg_written[3] = false;
        }
    }

    #[inline]
    fn calc_amp(&self) -> i32 {
        let mut amp = TRIANGLE_PHASE_RANGE - self.phase;
        if amp < 0 {
            amp = self.phase - (TRIANGLE_PHASE_RANGE + 1);
        }
        amp
    }

    // TODO: clean up
    #[inline]
    fn maintain_phase(
        &mut self,
        mut time: NesTime,
        end_time: NesTime,
        timer_period: NesTime,
    ) -> NesTime {
        let remain = end_time - time;
        if remain > 0 {
            let count = (remain + timer_period - 1) / timer_period;
            self.phase = ((self.phase as u32)
                .wrapping_add(1)
                .wrapping_sub(count as u32)
                & (TRIANGLE_PHASE_RANGE as u32 * 2 - 1)) as i32;
            self.phase += 1;
            time = time.wrapping_add(count.wrapping_mul(timer_period));
        }
        time
    }

    pub fn run(&mut self, bufs: &mut [BlipBuffer], mut time: NesTime, end_time: NesTime) {
        let timer_period = self.osc.period() + 1;
        let Some(out) = self.osc.output else {
            time += self.osc.delay;
            self.osc.delay = 0;
            if self.osc.length_counter != 0 && self.linear_counter != 0 && timer_period >= 3 {
                self.osc.delay = self.maintain_phase(time, end_time, timer_period) - end_time;
            }
            return;
        };
        let output = &mut bufs[out];

        output.set_modified();

        // to do: track phase when period < 3
        // to do: Output 7.5 on dac when period < 2? More accurate, but results in more clicks.

        let amp = self.calc_amp();
        let delta = self.osc.update_amp(amp);
        if delta != 0 {
            self.synth.offset(time, delta, output);
        }

        time += self.osc.delay;
        if self.osc.length_counter == 0 || self.linear_counter == 0 || timer_period < 3 {
            time = end_time;
        } else if time < end_time {
            let mut phase = self.phase;
            let mut volume = 1;
            if phase > TRIANGLE_PHASE_RANGE {
                phase -= TRIANGLE_PHASE_RANGE;
                volume = -volume;
            }

            loop {
                phase -= 1;
                if phase == 0 {
                    phase = TRIANGLE_PHASE_RANGE;
                    volume = -volume;
                } else {
                    self.synth.offset_inline(time, volume, output);
                }

                time += timer_period;
                if time >= end_time {
                    break;
                }
            }

            if volume < 0 {
                phase += TRIANGLE_PHASE_RANGE;
            }
            self.phase = phase;
            self.osc.last_amp = self.calc_amp();
        }
        self.osc.delay = time - end_time;
    }
}

// Nes_Noise

#[derive(Clone, Debug)]
pub struct NesNoise {
    pub env: NesEnvelope,
    pub noise: i32,
    pub synth: BlipSynth,
}

impl Default for NesNoise {
    fn default() -> Self {
        NesNoise {
            env: NesEnvelope::default(),
            noise: 0,
            synth: BlipSynth::new(BLIP_MED_QUALITY, 1),
        }
    }
}

static NOISE_PERIOD_TABLE: [i16; 16] = [
    0x004, 0x008, 0x010, 0x020, 0x040, 0x060, 0x080, 0x0A0, 0x0CA, 0x0FE, 0x17C, 0x1FC, 0x2FA,
    0x3F8, 0x7F2, 0xFE4,
];

impl NesNoise {
    pub fn reset(&mut self) {
        self.noise = 1 << 14;
        self.env.reset();
    }

    pub fn run(&mut self, bufs: &mut [BlipBuffer], mut time: NesTime, end_time: NesTime) {
        let period = NOISE_PERIOD_TABLE[(self.env.osc.regs[2] & 15) as usize] as i32;

        let Some(out) = self.env.osc.output else {
            // TODO: clean up
            time += self.env.osc.delay;
            self.env.osc.delay = time + (end_time - time + period - 1) / period * period - end_time;
            return;
        };
        let output = &mut bufs[out];

        output.set_modified();

        let volume = self.env.volume();
        let amp = if self.noise & 1 != 0 { volume } else { 0 };
        {
            let delta = self.env.osc.update_amp(amp);
            if delta != 0 {
                self.synth.offset(time, delta, output);
            }
        }

        time += self.env.osc.delay;
        if time < end_time {
            let mode_flag = 0x80;

            if volume == 0 {
                // round to next multiple of period
                time += (end_time - time + period - 1) / period * period;

                // approximate noise cycling while muted, by shuffling up noise register
                // to do: precise muted noise cycling?
                if self.env.osc.regs[2] & mode_flag == 0 {
                    let feedback = (self.noise << 13) ^ (self.noise << 14);
                    self.noise = (feedback & 0x4000) | (self.noise >> 1);
                }
            } else {
                // using resampled time avoids conversion in synth.offset()
                let rperiod = output.resampled_duration(period);
                let mut rtime = output.resampled_time(time);

                let mut noise = self.noise;
                let mut delta = amp * 2 - volume;
                let tap = if self.env.osc.regs[2] & mode_flag != 0 {
                    8
                } else {
                    13
                };

                loop {
                    let feedback = (noise << tap) ^ (noise << 14);
                    time += period;

                    if (noise + 1) & 2 != 0 {
                        // bits 0 and 1 of noise differ
                        delta = -delta;
                        self.synth.offset_resampled(rtime, delta, output);
                    }

                    rtime = rtime.wrapping_add(rperiod);
                    noise = (feedback & 0x4000) | (noise >> 1);
                    if time >= end_time {
                        break;
                    }
                }

                self.env.osc.last_amp = (delta + volume) >> 1;
                self.noise = noise;
            }
        }

        self.env.osc.delay = time - end_time;
    }
}

// Nes_Dmc

pub const DMC_LOOP_FLAG: u8 = 0x40;

#[derive(Clone, Debug)]
pub struct NesDmc {
    pub osc: NesOsc,
    /// address of next byte to read
    pub address: i32,
    pub period: i32,
    //int length_counter; // bytes remaining to play (already defined in Nes_Osc)
    pub buf: i32,
    pub bits_remain: i32,
    pub bits: i32,
    pub buf_full: bool,
    pub silence: bool,
    pub dac: i32,
    pub next_irq: NesTime,
    pub irq_enabled: bool,
    pub irq_flag: bool,
    pub pal_mode: bool,
    pub nonlinear: bool,
    pub synth: BlipSynth,
}

impl Default for NesDmc {
    fn default() -> Self {
        NesDmc {
            osc: NesOsc::default(),
            address: 0,
            period: 0,
            buf: 0,
            bits_remain: 0,
            bits: 0,
            buf_full: false,
            silence: false,
            dac: 0,
            next_irq: 0,
            irq_enabled: false,
            irq_flag: false,
            pal_mode: false,
            nonlinear: false,
            synth: BlipSynth::new(BLIP_MED_QUALITY, 1),
        }
    }
}

pub static DMC_PERIOD_TABLE: [[i16; 16]; 2] = [
    [
        428, 380, 340, 320, 286, 254, 226, 214, // NTSC
        190, 160, 142, 128, 106, 84, 72, 54,
    ],
    [
        398, 354, 316, 298, 276, 236, 210, 198, // PAL
        176, 148, 132, 118, 98, 78, 66, 50,
    ],
];

pub static DAC_TABLE: [u8; 128] = [
    0, 1, 2, 3, 4, 5, 6, 7, 7, 8, 9, 10, 11, 12, 13, 14, 15, 15, 16, 17, 18, 19, 20, 20, 21, 22,
    23, 24, 24, 25, 26, 27, 27, 28, 29, 30, 31, 31, 32, 33, 33, 34, 35, 36, 36, 37, 38, 38, 39, 40,
    41, 41, 42, 43, 43, 44, 45, 45, 46, 47, 47, 48, 48, 49, 50, 50, 51, 52, 52, 53, 53, 54, 55, 55,
    56, 56, 57, 58, 58, 59, 59, 60, 60, 61, 61, 62, 63, 63, 64, 64, 65, 65, 66, 66, 67, 67, 68, 68,
    69, 70, 70, 71, 71, 72, 72, 73, 73, 74, 74, 75, 75, 75, 76, 76, 77, 77, 78, 78, 79, 79, 80, 80,
    81, 81, 82, 82, 82, 83,
];

impl NesDmc {
    #[inline]
    pub fn reload_sample(&mut self) {
        self.address = 0x4000 + self.osc.regs[2] as i32 * 0x40;
        self.osc.length_counter = self.osc.regs[3] as i32 * 0x10 + 1;
    }
}
