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

//! AY-3-8910 sound chip emulator (`Ay_Apu.h`, `Ay_Apu.cpp`).

use crate::blip_buffer::{BlipBuffer, BlipEq, BlipSynth, BlipTime, BufId, BLIP_GOOD_QUALITY};

// Emulation inaccuracies:
// * Noise isn't run when not in use
// * Changes to envelope and noise periods are delayed until next reload
// * Super-sonic tone should attenuate output to about 60%, not 50%

// Tones above this frequency are treated as disabled tone at half volume.
// Power of two is more efficient (avoids division).
const INAUDIBLE_FREQ: u32 = 16384;

const PERIOD_FACTOR: i32 = 16;

pub const REG_COUNT: usize = 16;
pub const OSC_COUNT: i32 = 3;
pub const AMP_RANGE: i32 = 255;

const fn entry(n: f64) -> u8 {
    (n * AMP_RANGE as f64 + 0.5) as u8
}

static AMP_TABLE: [u8; 16] = [
    // With channels tied together and 1K resistor to ground (as datasheet recommends),
    // output nearly matches logarithmic curve as claimed. Approx. 1.5 dB per step.
    entry(0.000000),
    entry(0.007813),
    entry(0.011049),
    entry(0.015625),
    entry(0.022097),
    entry(0.031250),
    entry(0.044194),
    entry(0.062500),
    entry(0.088388),
    entry(0.125000),
    entry(0.176777),
    entry(0.250000),
    entry(0.353553),
    entry(0.500000),
    entry(0.707107),
    entry(1.000000),
    /*
    // Measured from an AY-3-8910A chip with date code 8611.
    // Direct voltages without any load (very linear)
    ENTRY(0.000000),ENTRY(0.046237),ENTRY(0.064516),ENTRY(0.089785),
    ENTRY(0.124731),ENTRY(0.173118),ENTRY(0.225806),ENTRY(0.329032),
    ENTRY(0.360215),ENTRY(0.494624),ENTRY(0.594624),ENTRY(0.672043),
    ENTRY(0.766129),ENTRY(0.841935),ENTRY(0.926882),ENTRY(1.000000),
    // With only some load
    ENTRY(0.000000),ENTRY(0.011940),ENTRY(0.017413),ENTRY(0.024876),
    ENTRY(0.036318),ENTRY(0.054229),ENTRY(0.072637),ENTRY(0.122388),
    ENTRY(0.174129),ENTRY(0.239303),ENTRY(0.323881),ENTRY(0.410945),
    ENTRY(0.527363),ENTRY(0.651741),ENTRY(0.832338),ENTRY(1.000000),
    */
];

const fn mode(a0: u8, a1: u8, b0: u8, b1: u8, c0: u8, c1: u8) -> u8 {
    a0 | a1 << 1 | b0 << 2 | b1 << 3 | c0 << 4 | c1 << 5
}

static MODES: [u8; 8] = [
    mode(1, 0, 1, 0, 1, 0),
    mode(1, 0, 0, 0, 0, 0),
    mode(1, 0, 0, 1, 1, 0),
    mode(1, 0, 1, 1, 1, 1),
    mode(0, 1, 0, 1, 0, 1),
    mode(0, 1, 1, 1, 1, 1),
    mode(0, 1, 1, 0, 0, 1),
    mode(0, 1, 0, 0, 0, 0),
];

#[derive(Clone, Copy, Debug, Default)]
struct Osc {
    period: BlipTime,
    delay: BlipTime,
    last_amp: i16,
    phase: i16,
    output: Option<BufId>,
}

#[derive(Clone, Debug)]
struct Env {
    delay: BlipTime,
    /// `wave`: the index in `modes` that the envelope positions (-48 to
    /// -1) are relative to
    wave: usize,
    pos: i32,
    /// `modes [8] [48]`: values already passed through volume table
    modes: [u8; 8 * 48],
}

#[derive(Clone, Debug)]
pub struct AyApu {
    oscs: [Osc; OSC_COUNT as usize],
    last_time: BlipTime,
    regs: [u8; REG_COUNT],

    noise_delay: BlipTime,
    noise_lfsr: u32,

    env: Env,

    pub synth_: BlipSynth,
}

impl Default for AyApu {
    fn default() -> Self {
        Self::new()
    }
}

const NOISE_OFF: i32 = 0x08;
const TONE_OFF: i32 = 0x01;

impl AyApu {
    pub fn new() -> Self {
        let mut apu = AyApu {
            oscs: [Osc::default(); OSC_COUNT as usize],
            last_time: 0,
            regs: [0; REG_COUNT],
            noise_delay: 0,
            noise_lfsr: 0,
            env: Env {
                delay: 0,
                wave: 0,
                pos: 0,
                modes: [0; 8 * 48],
            },
            synth_: BlipSynth::new(BLIP_GOOD_QUALITY, 1),
        };

        // build full table of the upper 8 envelope waveforms
        let mut m = 8;
        while m > 0 {
            m -= 1;
            let mut out = m * 48;
            let mut flags = MODES[m] as i32;
            let mut x = 3;
            while {
                x -= 1;
                x >= 0
            } {
                let mut amp = flags & 1;
                let end = flags >> 1 & 1;
                let step = end - amp;
                amp *= 15;
                let mut y = 16;
                while {
                    y -= 1;
                    y >= 0
                } {
                    apu.env.modes[out] = AMP_TABLE[amp as usize];
                    out += 1;
                    amp += step;
                }
                flags >>= 2;
            }
        }

        apu.output(None);
        apu.volume(1.0);
        apu.reset();
        apu
    }

    /// Set overall volume (default is 1.0)
    pub fn volume(&mut self, v: f64) {
        self.synth_
            .volume(0.7 / OSC_COUNT as f64 / AMP_RANGE as f64 * v);
    }

    /// Set treble equalization (see documentation)
    pub fn treble_eq(&mut self, eq: &BlipEq) {
        self.synth_.treble_eq(eq);
    }

    /// Write to register at specified time
    pub fn write(&mut self, bufs: &mut [BlipBuffer], time: BlipTime, addr: i32, data: i32) {
        self.run_until(bufs, time);
        self.write_data_(addr, data);
    }

    /// Set sound output of specific oscillator to buffer, where index is
    /// 0, 1, or 2. If buffer is NULL, the specified oscillator is muted.
    pub fn osc_output(&mut self, i: i32, buf: Option<BufId>) {
        // assert( (unsigned) i < osc_count );
        self.oscs[i as usize].output = buf;
    }

    /// Set buffer to generate all sound into, or disable sound if NULL
    pub fn output(&mut self, buf: Option<BufId>) {
        self.osc_output(0, buf);
        self.osc_output(1, buf);
        self.osc_output(2, buf);
    }

    /// Run sound to specified time, end current time frame, then start a new
    /// time frame at time 0. Time frames have no effect on emulation and each
    /// can be whatever length is convenient.
    pub fn end_frame(&mut self, bufs: &mut [BlipBuffer], time: BlipTime) {
        if time > self.last_time {
            self.run_until(bufs, time);
        }

        // assert( last_time >= time );
        self.last_time -= time;
    }

    /// Reset sound chip
    pub fn reset(&mut self) {
        self.last_time = 0;
        self.noise_delay = 0;
        self.noise_lfsr = 1;

        for osc in self.oscs.iter_mut().rev() {
            osc.period = PERIOD_FACTOR;
            osc.delay = 0;
            osc.last_amp = 0;
            osc.phase = 0;
        }

        self.regs.fill(0);
        self.regs[7] = 0xFF;
        self.write_data_(13, 0);
    }

    fn write_data_(&mut self, addr: i32, mut data: i32) {
        // assert( (unsigned) addr < reg_count );

        // envelope mode
        if addr == 13 {
            if data & 8 == 0 {
                // convert modes 0-7 to proper equivalents
                data = if data & 4 != 0 { 15 } else { 9 };
            }
            // Note (upstream): the envelope positions are -48 to -1 from
            // `wave`, so they index the row before `modes [(data - 7) & 7]`;
            // for mode 15 that is row 0, and they read the 48 bytes before
            // the table (the fields before it); this uses row 8, so mode 15
            // gets the last row (its attack-then-off waveform), as its
            // table entry is built for, and upstream's build for the tests
            // is patched the same.
            let mut row = ((data - 7) & 0x07) as usize;
            if row == 0 {
                row = 8;
            }
            self.env.wave = row * 48;
            self.env.pos = -48;
            self.env.delay = 0; // will get set to envelope period in run_until()
        }
        self.regs[addr as usize] = data as u8;

        // handle period changes accurately
        let i = addr >> 1;
        if i < OSC_COUNT {
            let mut period: BlipTime = (self.regs[(i * 2 + 1) as usize] & 0x0F) as BlipTime
                * (0x100 * PERIOD_FACTOR)
                + self.regs[(i * 2) as usize] as BlipTime * PERIOD_FACTOR;
            if period == 0 {
                period = PERIOD_FACTOR;
            }

            // adjust time of next timer expiration based on change in period
            let osc = &mut self.oscs[i as usize];
            osc.delay += period - osc.period;
            if osc.delay < 0 {
                osc.delay = 0;
            }
            osc.period = period;
        }

        // TODO: same as above for envelope timer, and it also has a divide by two after it
    }

    /// `env.wave [pos]`
    #[inline]
    fn env_wave(&self, pos: i32) -> i32 {
        self.env.modes[(self.env.wave as i32 + pos) as usize] as i32
    }

    fn run_until(&mut self, bufs: &mut [BlipBuffer], final_end_time: BlipTime) {
        // require( final_end_time >= last_time );

        // noise period and initial values
        let noise_period_factor: BlipTime = PERIOD_FACTOR * 2; // verified
        let mut noise_period: BlipTime = (self.regs[6] & 0x1F) as BlipTime * noise_period_factor;
        if noise_period == 0 {
            noise_period = noise_period_factor;
        }
        let old_noise_delay: BlipTime = self.noise_delay;
        let old_noise_lfsr: u32 = self.noise_lfsr;

        // envelope period
        let env_period_factor: BlipTime = PERIOD_FACTOR * 2; // verified
        let mut env_period: BlipTime =
            (self.regs[12] as BlipTime * 0x100 + self.regs[11] as BlipTime) * env_period_factor;
        if env_period == 0 {
            env_period = env_period_factor; // same as period 1 on my AY chip
        }
        if self.env.delay == 0 {
            self.env.delay = env_period;
        }

        // run each osc separately
        for index in 0..OSC_COUNT as usize {
            let mut osc_mode = (self.regs[7] >> index) as i32;

            // output
            let Some(out) = self.oscs[index].output else {
                continue;
            };
            let osc_output = &mut bufs[out];
            osc_output.set_modified();

            // period
            let mut half_vol = 0;
            let inaudible_period: BlipTime = ((osc_output.clock_rate() as i64
                + INAUDIBLE_FREQ as i64) as u32
                / (INAUDIBLE_FREQ * 2)) as BlipTime;
            if self.oscs[index].period <= inaudible_period && (osc_mode & TONE_OFF) == 0 {
                half_vol = 1; // Actually around 60%, but 50% is close enough
                osc_mode |= TONE_OFF;
            }

            // envelope
            let mut start_time: BlipTime = self.last_time;
            let mut end_time: BlipTime = final_end_time;
            let vol_mode = self.regs[0x08 + index] as i32;
            let mut volume = (AMP_TABLE[(vol_mode & 0x0F) as usize] as i32) >> half_vol;
            let mut osc_env_pos = self.env.pos;
            if vol_mode & 0x10 != 0 {
                volume = self.env_wave(osc_env_pos) >> half_vol;
                // use envelope only if it's a repeating wave or a ramp that hasn't finished
                if (self.regs[13] & 1) == 0 || osc_env_pos < -32 {
                    end_time = start_time + self.env.delay;
                    if end_time >= final_end_time {
                        end_time = final_end_time;
                    }

                    //if ( !(regs [12] | regs [11]) )
                    //  debug_printf( "Used envelope period 0\n" );
                } else if volume == 0 {
                    osc_mode = NOISE_OFF | TONE_OFF;
                }
            } else if volume == 0 {
                osc_mode = NOISE_OFF | TONE_OFF;
            }

            let osc = &mut self.oscs[index];

            // tone time
            let period: BlipTime = osc.period;
            let mut time: BlipTime = start_time + osc.delay;
            if osc_mode & TONE_OFF != 0 {
                // maintain tone's phase when off
                let count: i32 = (final_end_time - time + period - 1) / period;
                time += count * period;
                osc.phase ^= (count & 1) as i16;
            }

            // noise time
            let mut ntime: BlipTime = final_end_time;
            let mut noise_lfsr: u32 = 1;
            if osc_mode & NOISE_OFF == 0 {
                ntime = start_time + old_noise_delay;
                noise_lfsr = old_noise_lfsr;
                //if ( (regs [6] & 0x1F) == 0 )
                //  debug_printf( "Used noise period 0\n" );
            }

            // The following efficiently handles several cases (least demanding first):
            // * Tone, noise, and envelope disabled, where channel acts as 4-bit DAC
            // * Just tone or just noise, envelope disabled
            // * Envelope controlling tone and/or noise
            // * Tone and noise disabled, envelope enabled with high frequency
            // * Tone and noise together
            // * Tone and noise together with envelope

            // This loop only runs one iteration if envelope is disabled. If envelope
            // is being used as a waveform (tone and noise disabled), this loop will
            // still be reasonably efficient since the bulk of it will be skipped.
            loop {
                // current amplitude
                let mut amp = 0;
                if ((osc_mode | osc.phase as i32) & 1 & (osc_mode >> 3 | noise_lfsr as i32)) != 0 {
                    amp = volume;
                }
                {
                    let delta = amp - osc.last_amp as i32;
                    if delta != 0 {
                        osc.last_amp = amp as i16;
                        self.synth_.offset(start_time, delta, osc_output);
                    }
                }

                // Run wave and noise interleved with each catching up to the other.
                // If one or both are disabled, their "current time" will be past end time,
                // so there will be no significant performance hit.
                if ntime < end_time || time < end_time {
                    // Since amplitude was updated above, delta will always be +/- volume,
                    // so we can avoid using last_amp every time to calculate the delta.
                    let mut delta = amp * 2 - volume;
                    let delta_non_zero = (delta != 0) as i32;
                    let mut phase = osc.phase as i32 | (osc_mode & TONE_OFF); // assert( tone_off == 0x01 );
                    loop {
                        // run noise
                        let mut end = end_time;
                        if end_time > time {
                            end = time;
                        }
                        if phase & delta_non_zero != 0 {
                            while ntime <= end {
                                // must advance *past* time to avoid hang
                                let changed = noise_lfsr.wrapping_add(1);
                                noise_lfsr =
                                    ((noise_lfsr & 1).wrapping_neg() & 0x12000) ^ (noise_lfsr >> 1);
                                if changed & 2 != 0 {
                                    delta = -delta;
                                    self.synth_.offset(ntime, delta, osc_output);
                                }
                                ntime += noise_period;
                            }
                        } else {
                            // 20 or more noise periods on average for some music
                            let remain: i32 = end - ntime;
                            let count: i32 = remain / noise_period;
                            if remain >= 0 {
                                ntime += noise_period + count * noise_period;
                            }
                        }

                        // run tone
                        end = end_time;
                        if end_time > ntime {
                            end = ntime;
                        }
                        if noise_lfsr as i32 & delta_non_zero != 0 {
                            while time < end {
                                delta = -delta;
                                self.synth_.offset(time, delta, osc_output);
                                time += period;
                                //phase ^= 1;
                            }
                            //assert( phase == (delta > 0) );
                            phase = ((delta.wrapping_neg()) as u32 >> (u32::BITS - 1)) as i32;
                            // (delta > 0)
                        } else {
                            // loop usually runs less than once
                            //SUB_CASE_COUNTER( (time < end) * (end - time + period - 1) / period );

                            while time < end {
                                time += period;
                                phase ^= 1;
                            }
                        }
                        if !(time < end_time || ntime < end_time) {
                            break;
                        }
                    }

                    osc.last_amp = ((delta + volume) >> 1) as i16;
                    if osc_mode & TONE_OFF == 0 {
                        osc.phase = phase as i16;
                    }
                }

                if end_time >= final_end_time {
                    break; // breaks first time when envelope is disabled
                }

                // next envelope step
                osc_env_pos += 1;
                if osc_env_pos >= 0 {
                    osc_env_pos -= 32;
                }
                volume = (self.env.modes[(self.env.wave as i32 + osc_env_pos) as usize] as i32)
                    >> half_vol;

                start_time = end_time;
                end_time += env_period;
                if end_time > final_end_time {
                    end_time = final_end_time;
                }
            }
            osc.delay = time - final_end_time;

            if osc_mode & NOISE_OFF == 0 {
                self.noise_delay = ntime - final_end_time;
                self.noise_lfsr = noise_lfsr;
            }
        }

        // TODO: optimized saw wave envelope?

        // maintain envelope phase
        let mut remain: BlipTime = final_end_time - self.last_time - self.env.delay;
        if remain >= 0 {
            let count: i32 = (remain + env_period) / env_period;
            self.env.pos += count;
            if self.env.pos >= 0 {
                self.env.pos = (self.env.pos & 31) - 32;
            }
            remain -= count * env_period;
            // assert( -remain <= env_period );
        }
        self.env.delay = -remain;
        // assert( env.delay > 0 );
        // assert( env.pos < 0 );

        self.last_time = final_end_time;
    }
}
