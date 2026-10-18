// Sms_Snd_Emu 0.1.4. http://www.slack.net/~ant/

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

//! Sega Master System SN76489 PSG sound chip emulator (`Sms_Apu.h`,
//! `Sms_Apu.cpp`, with its private oscillators, `Sms_Oscs.h`). The state
//! saving functions, which nothing calls, are left out.

use crate::blip_buffer::{
    BlipBuffer, BlipEq, BlipSynth, BlipTime, BufId, BLIP_GOOD_QUALITY, BLIP_MED_QUALITY,
};

pub const OSC_COUNT: i32 = 4;

/// `Sms_Osc`
#[derive(Clone, Debug, Default)]
pub struct SmsOsc {
    pub outputs: [Option<BufId>; 4], // NULL, right, left, center
    pub output: Option<BufId>,
    pub output_select: i32,

    pub delay: i32,
    pub last_amp: i32,
    pub volume: i32,
}

impl SmsOsc {
    // Sms_Osc

    pub fn reset(&mut self) {
        self.delay = 0;
        self.last_amp = 0;
        self.volume = 0;
        self.output_select = 3;
        self.output = self.outputs[3];
    }
}

/// `Sms_Square` (its synth is `Blip_Synth<blip_good_quality,1>`, shared)
#[derive(Clone, Debug, Default)]
pub struct SmsSquare {
    pub osc: SmsOsc,
    pub period: i32,
    pub phase: i32,
}

impl SmsSquare {
    // Sms_Square

    #[inline]
    pub fn reset(&mut self) {
        self.period = 0;
        self.phase = 0;
        self.osc.reset();
    }

    pub fn run(
        &mut self,
        synth: &BlipSynth,
        bufs: &mut [BlipBuffer],
        mut time: BlipTime,
        end_time: BlipTime,
    ) {
        let osc = &mut self.osc;
        let out = osc.output.expect("output");
        if osc.volume == 0 || self.period <= 128 {
            // ignore 16kHz and higher
            if osc.last_amp != 0 {
                synth.offset(time, -osc.last_amp, &mut bufs[out]);
                osc.last_amp = 0;
            }
            time += osc.delay;
            if self.period == 0 {
                time = end_time;
            } else if time < end_time {
                // keep calculating phase
                let count = (end_time - time + self.period - 1) / self.period;
                self.phase = (self.phase + count) & 1;
                time += count * self.period;
            }
        } else {
            let amp = if self.phase != 0 {
                osc.volume
            } else {
                -osc.volume
            };
            {
                let delta = amp - osc.last_amp;
                if delta != 0 {
                    osc.last_amp = amp;
                    synth.offset(time, delta, &mut bufs[out]);
                }
            }

            time += osc.delay;
            if time < end_time {
                let output = &mut bufs[out];
                let mut delta = amp * 2;
                loop {
                    delta = -delta;
                    synth.offset_inline(time, delta, output);
                    time += self.period;
                    self.phase ^= 1;
                    if time >= end_time {
                        break;
                    }
                }
                osc.last_amp = if self.phase != 0 {
                    osc.volume
                } else {
                    -osc.volume
                };
            }
        }
        osc.delay = time - end_time;
    }
}

/// Where the noise's period comes from (`period`, a pointer upstream):
/// one of the three fixed periods, or square 3's.
#[derive(Clone, Copy, Debug)]
pub enum NoisePeriod {
    Fixed(usize),
    Square3,
}

impl Default for NoisePeriod {
    fn default() -> Self {
        NoisePeriod::Fixed(0)
    }
}

// Sms_Noise

static NOISE_PERIODS: [i32; 3] = [0x100, 0x200, 0x400];

/// `Sms_Noise` (its synth is `Blip_Synth<blip_med_quality,1>`)
#[derive(Clone, Debug)]
pub struct SmsNoise {
    pub osc: SmsOsc,
    pub period: NoisePeriod,
    pub shifter: u32,
    pub feedback: u32,
    pub synth: BlipSynth,
}

impl SmsNoise {
    #[inline]
    pub fn reset(&mut self) {
        self.period = NoisePeriod::Fixed(0);
        self.shifter = 0x8000;
        self.feedback = 0x9000;
        self.osc.reset();
    }

    pub fn run(
        &mut self,
        bufs: &mut [BlipBuffer],
        square3_period: i32,
        mut time: BlipTime,
        end_time: BlipTime,
    ) {
        let osc = &mut self.osc;
        let out = osc.output.expect("output");
        let mut amp = osc.volume;
        if self.shifter & 1 != 0 {
            amp = -amp;
        }

        {
            let delta = amp - osc.last_amp;
            if delta != 0 {
                osc.last_amp = amp;
                self.synth.offset(time, delta, &mut bufs[out]);
            }
        }

        time += osc.delay;
        if osc.volume == 0 {
            time = end_time;
        }

        if time < end_time {
            let output = &mut bufs[out];
            let mut shifter = self.shifter;
            let mut delta = amp * 2;
            let p = match self.period {
                NoisePeriod::Fixed(i) => NOISE_PERIODS[i],
                NoisePeriod::Square3 => square3_period,
            };
            let mut period = p * 2;
            if period == 0 {
                period = 16;
            }

            loop {
                let changed = shifter.wrapping_add(1);
                shifter = (self.feedback & (shifter & 1).wrapping_neg()) ^ (shifter >> 1);
                if changed & 2 != 0 {
                    // true if bits 0 and 1 differ
                    delta = -delta;
                    self.synth.offset_inline(time, delta, output);
                }
                time += period;
                if time >= end_time {
                    break;
                }
            }

            self.shifter = shifter;
            osc.last_amp = delta >> 1;
        }
        osc.delay = time - end_time;
    }
}

#[derive(Clone, Debug)]
pub struct SmsApu {
    squares: [SmsSquare; 3],
    square_synth: BlipSynth, // used by squares
    last_time: BlipTime,
    latch: i32,
    noise: SmsNoise,
    noise_feedback: u32,
    looped_feedback: u32,
}

impl Default for SmsApu {
    fn default() -> Self {
        Self::new()
    }
}

// volumes [i] = 64 * pow( 1.26, 15 - i ) / pow( 1.26, 15 )
static VOLUMES: [u8; 16] = [64, 50, 39, 31, 24, 19, 15, 12, 9, 7, 5, 4, 3, 2, 1, 0];

impl SmsApu {
    // Sms_Apu

    pub fn new() -> Self {
        let mut apu = SmsApu {
            squares: Default::default(),
            square_synth: BlipSynth::new(BLIP_GOOD_QUALITY, 1),
            last_time: 0,
            latch: 0,
            noise: SmsNoise {
                osc: SmsOsc::default(),
                period: NoisePeriod::default(),
                shifter: 0,
                feedback: 0,
                synth: BlipSynth::new(BLIP_MED_QUALITY, 1),
            },
            noise_feedback: 0,
            looped_feedback: 0,
        };
        apu.volume(1.0);
        apu.reset(0, 0);
        apu
    }

    /// `oscs [i]`
    fn osc(&mut self, i: i32) -> &mut SmsOsc {
        if i < 3 {
            &mut self.squares[i as usize].osc
        } else {
            &mut self.noise.osc
        }
    }

    /// Set overall volume of all oscillators, where 1.0 is full volume
    pub fn volume(&mut self, mut vol: f64) {
        vol *= 0.85 / (OSC_COUNT * 64 * 2) as f64;
        self.square_synth.volume(vol);
        self.noise.synth.volume(vol);
    }

    /// Set treble equalization
    pub fn treble_eq(&mut self, eq: &BlipEq) {
        self.square_synth.treble_eq(eq);
        self.noise.synth.treble_eq(eq);
    }

    /// Assign single oscillator output to buffer(s). Valid indicies are 0 to 3,
    /// which refer to Square 1, Square 2, Square 3, and Noise. If buffer is NULL,
    /// silences oscillator.
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

    /// Reset oscillators and internal state
    pub fn reset(&mut self, mut feedback: u32, mut noise_width: i32) {
        self.last_time = 0;
        self.latch = 0;

        if feedback == 0 || noise_width == 0 {
            feedback = 0x0009;
            noise_width = 16;
        }
        // convert to "Galios configuration"
        // FIXME (upstream): a noise width past 32 (from a VGM header)
        // shifts past the feedback's bits; the shift is kept to 5 bits, as
        // x86 keeps it, here and in upstream's build for the tests.
        self.looped_feedback = 1u32 << ((noise_width - 1) & 31);
        self.noise_feedback = 0;
        while noise_width != 0 {
            noise_width -= 1;
            self.noise_feedback = (self.noise_feedback << 1) | (feedback & 1);
            feedback >>= 1;
        }

        self.squares[0].reset();
        self.squares[1].reset();
        self.squares[2].reset();
        self.noise.reset();
    }

    fn run_until(&mut self, bufs: &mut [BlipBuffer], end_time: BlipTime) {
        // require( end_time >= last_time ); // end_time must not be before previous time

        if end_time > self.last_time {
            // run oscillators
            for i in 0..OSC_COUNT {
                if let Some(out) = self.osc(i).output {
                    bufs[out].set_modified();
                    if i < 3 {
                        self.squares[i as usize].run(
                            &self.square_synth,
                            bufs,
                            self.last_time,
                            end_time,
                        );
                    } else {
                        let p3 = self.squares[2].period;
                        self.noise.run(bufs, p3, self.last_time, end_time);
                    }
                }
            }

            self.last_time = end_time;
        }
    }

    /// Run all oscillators up to specified time, end current frame, then
    /// start a new frame at time 0.
    pub fn end_frame(&mut self, bufs: &mut [BlipBuffer], end_time: BlipTime) {
        if end_time > self.last_time {
            self.run_until(bufs, end_time);
        }

        // assert( last_time >= end_time );
        self.last_time -= end_time;
    }

    /// Write GameGear left/right assignment byte
    pub fn write_ggstereo(&mut self, bufs: &mut [BlipBuffer], time: BlipTime, data: i32) {
        // require( (unsigned) data <= 0xFF );

        self.run_until(bufs, time);

        for i in 0..OSC_COUNT {
            let osc = self.osc(i);
            let flags = data >> i;
            let old_output = osc.output;
            osc.output_select = (flags >> 3 & 2) | (flags & 1);
            osc.output = osc.outputs[osc.output_select as usize];
            if osc.output != old_output && osc.last_amp != 0 {
                let last_amp = osc.last_amp;
                if let Some(old) = old_output {
                    bufs[old].set_modified();
                    self.square_synth.offset(time, -last_amp, &mut bufs[old]);
                }
                self.osc(i).last_amp = 0;
            }
        }
    }

    /// Write to data port
    pub fn write_data(&mut self, bufs: &mut [BlipBuffer], time: BlipTime, data: i32) {
        // require( (unsigned) data <= 0xFF );

        self.run_until(bufs, time);

        if data & 0x80 != 0 {
            self.latch = data;
        }

        let index = (self.latch >> 5) & 3;
        if self.latch & 0x10 != 0 {
            self.osc(index).volume = VOLUMES[(data & 15) as usize] as i32;
        } else if index < 3 {
            let sq = &mut self.squares[index as usize];
            if data & 0x80 != 0 {
                sq.period = (sq.period & 0xFF00) | (data << 4 & 0x00FF);
            } else {
                sq.period = (sq.period & 0x00FF) | (data << 8 & 0x3F00);
            }
        } else {
            let select = (data & 3) as usize;
            if select < 3 {
                self.noise.period = NoisePeriod::Fixed(select);
            } else {
                self.noise.period = NoisePeriod::Square3;
            }

            self.noise.feedback = if data & 0x04 != 0 {
                self.noise_feedback
            } else {
                self.looped_feedback
            };
            self.noise.shifter = 0x8000;
        }
    }
}
