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

//! NES FDS sound chip emulator (`Nes_Fds_Apu.h`, `Nes_Fds_Apu.cpp`).

use crate::blip_buffer::{BlipBuffer, BlipEq, BlipSynth, BlipTime, BufId, BLIP_MED_QUALITY};

pub const OSC_COUNT: i32 = 1;
pub const IO_ADDR: u32 = 0x4040;
pub const IO_SIZE: u32 = 0x53;

const WAVE_SIZE: u32 = 0x40;
const MASTER_VOL_MAX: u32 = 10;
const VOL_MAX: i32 = 0x20;
const WAVE_SAMPLE_MAX: u32 = 0x3F;
const LFO_BASE_TEMPO: u32 = 8;

const FRACT_RANGE: i32 = 65536;

#[derive(Clone, Debug)]
pub struct NesFdsApu {
    /// last written value to registers
    regs_: [u8; IO_SIZE as usize],
    /// normally 8; adjusted by set_tempo()
    lfo_tempo: i32,
    env_delay: i32,
    env_speed: i32,
    env_gain: i32,
    sweep_delay: i32,
    sweep_speed: i32,
    sweep_gain: i32,
    wave_pos: i32,
    last_amp: i32,
    wave_fract: BlipTime,
    mod_fract: i32,
    mod_pos: i32,
    mod_write_pos: i32,
    mod_wave: [u8; WAVE_SIZE as usize],

    // synthesis
    last_time: BlipTime,
    output_: Option<BufId>,
    synth: BlipSynth,
}

impl Default for NesFdsApu {
    fn default() -> Self {
        Self::new()
    }
}

impl NesFdsApu {
    pub fn new() -> Self {
        let mut a = NesFdsApu {
            regs_: [0; IO_SIZE as usize],
            lfo_tempo: LFO_BASE_TEMPO as i32,
            env_delay: 0,
            // (upstream leaves the speeds and gains uninitialized; reset()'s
            // register writes set them)
            env_speed: 0,
            env_gain: 0,
            sweep_delay: 0,
            sweep_speed: 0,
            sweep_gain: 0,
            wave_pos: 0,
            last_amp: 0,
            wave_fract: 0,
            mod_fract: 0,
            mod_pos: 0,
            mod_write_pos: 0,
            mod_wave: [0; WAVE_SIZE as usize],
            last_time: 0,
            output_: None,
            synth: BlipSynth::new(BLIP_MED_QUALITY, 1),
        };
        a.osc_output(0, None);
        a.volume(1.0);
        a.reset();
        a
    }

    pub fn volume(&mut self, v: f64) {
        self.synth
            .volume(0.14 / MASTER_VOL_MAX as f64 / VOL_MAX as f64 / WAVE_SAMPLE_MAX as f64 * v);
    }

    pub fn treble_eq(&mut self, eq: &BlipEq) {
        self.synth.treble_eq(eq);
    }

    pub fn osc_output(&mut self, _i: i32, buf: Option<BufId>) {
        // assert( (unsigned) i < osc_count );
        self.output_ = buf;
    }

    /// allow access to registers by absolute address (i.e. 0x4080)
    #[inline]
    fn regs(&self, addr: u32) -> u8 {
        self.regs_[(addr - IO_ADDR) as usize]
    }
    #[inline]
    fn set_regs(&mut self, addr: u32, v: u8) {
        self.regs_[(addr - IO_ADDR) as usize] = v;
    }

    pub fn end_frame(&mut self, bufs: &mut [BlipBuffer], end_time: BlipTime) {
        if end_time > self.last_time {
            self.run_until(bufs, end_time);
        }
        self.last_time -= end_time;
        // assert( last_time >= 0 );
    }

    pub fn write(&mut self, bufs: &mut [BlipBuffer], time: BlipTime, addr: u32, data: i32) {
        self.run_until(bufs, time);
        self.write_(addr, data);
    }

    pub fn read(&mut self, bufs: &mut [BlipBuffer], time: BlipTime, addr: u32) -> i32 {
        self.run_until(bufs, time);
        let mut result = 0xFF;
        match addr {
            0x4090 => result = self.env_gain,
            0x4092 => result = self.sweep_gain,
            _ => {
                let i = addr.wrapping_sub(IO_ADDR);
                if i < WAVE_SIZE {
                    result = self.regs_[i as usize] as i32;
                }
            }
        }
        result | 0x40
    }

    pub fn reset(&mut self) {
        self.regs_ = [0; IO_SIZE as usize];
        self.mod_wave = [0; WAVE_SIZE as usize];

        self.last_time = 0;
        self.env_delay = 0;
        self.sweep_delay = 0;
        self.wave_pos = 0;
        self.last_amp = 0;
        self.wave_fract = FRACT_RANGE;
        self.mod_fract = FRACT_RANGE;
        self.mod_pos = 0;
        self.mod_write_pos = 0;

        static INITIAL_REGS: [u8; 0x0B] = [
            0x80, // disable envelope
            0, 0, 0xC0, // disable wave and lfo
            0x80, // disable sweep
            0, 0, 0x80, // disable modulation
            0, 0, 0xFF, // LFO period // TODO: use 0xE8 as FDS ROM does?
        ];
        for (i, &r) in INITIAL_REGS.iter().enumerate() {
            // two writes to set both gain and period for envelope registers
            self.write_(IO_ADDR + WAVE_SIZE + i as u32, 0);
            self.write_(IO_ADDR + WAVE_SIZE + i as u32, r as i32);
        }
    }

    pub fn write_(&mut self, addr: u32, mut data: i32) {
        let reg = addr.wrapping_sub(IO_ADDR);
        if reg < IO_SIZE {
            if reg < WAVE_SIZE {
                if self.regs(0x4089) & 0x80 != 0 {
                    self.regs_[reg as usize] = (data as u32 & WAVE_SAMPLE_MAX) as u8;
                }
            } else {
                self.regs_[reg as usize] = data as u8;
                match addr {
                    0x4080 => {
                        if data & 0x80 != 0 {
                            self.env_gain = data & 0x3F;
                        } else {
                            self.env_speed = (data & 0x3F) + 1;
                        }
                    }

                    0x4084 => {
                        if data & 0x80 != 0 {
                            self.sweep_gain = data & 0x3F;
                        } else {
                            self.sweep_speed = (data & 0x3F) + 1;
                        }
                    }

                    0x4085 => {
                        self.mod_pos = self.mod_write_pos;
                        self.set_regs(0x4085, (data & 0x7F) as u8);
                    }

                    0x4088 => {
                        if self.regs(0x4087) & 0x80 != 0 {
                            let pos = self.mod_write_pos;
                            data &= 0x07;
                            self.mod_wave[pos as usize] = data as u8;
                            self.mod_wave[pos as usize + 1] = data as u8;
                            self.mod_write_pos = (pos + 2) & (WAVE_SIZE as i32 - 1);
                            self.mod_pos = (self.mod_pos + 2) & (WAVE_SIZE as i32 - 1);
                        }
                    }

                    _ => {}
                }
            }
        }
    }

    pub fn set_tempo(&mut self, t: f64) {
        self.lfo_tempo = LFO_BASE_TEMPO as i32;
        if t != 1.0 {
            self.lfo_tempo = (LFO_BASE_TEMPO as f64 / t + 0.5) as i32;
            if self.lfo_tempo <= 0 {
                self.lfo_tempo = 1;
            }
        }
    }

    fn run_until(&mut self, bufs: &mut [BlipBuffer], final_end_time: BlipTime) {
        let wave_freq = (self.regs(0x4083) as i32 & 0x0F) * 0x100 + self.regs(0x4082) as i32;
        if let (true, Some(out)) = (
            wave_freq != 0 && (self.regs(0x4089) | self.regs(0x4083)) & 0x80 == 0,
            self.output_,
        ) {
            let output_ = &mut bufs[out];
            output_.set_modified();

            // master_volume
            const fn mvol_entry(percent: u32) -> u8 {
                ((MASTER_VOL_MAX * percent + 50) / 100) as u8
            }
            static MASTER_VOLUMES: [u8; 4] = [
                mvol_entry(100),
                mvol_entry(67),
                mvol_entry(50),
                mvol_entry(40),
            ];
            let master_volume = MASTER_VOLUMES[(self.regs(0x4089) & 0x03) as usize] as i32;

            // lfo_period
            let mut lfo_period: BlipTime = self.regs(0x408A) as i32 * self.lfo_tempo;
            if self.regs(0x4083) & 0x40 != 0 {
                lfo_period = 0;
            }

            // sweep setup
            let mut sweep_time: BlipTime = self.last_time + self.sweep_delay;
            let sweep_period: BlipTime = lfo_period * self.sweep_speed;
            if sweep_period == 0 || self.regs(0x4084) & 0x80 != 0 {
                sweep_time = final_end_time;
            }

            // envelope setup
            let mut env_time: BlipTime = self.last_time + self.env_delay;
            let env_period: BlipTime = lfo_period * self.env_speed;
            if env_period == 0 || self.regs(0x4080) & 0x80 != 0 {
                env_time = final_end_time;
            }

            // modulation
            let mut mod_freq = 0;
            if self.regs(0x4087) & 0x80 == 0 {
                mod_freq = (self.regs(0x4087) as i32 & 0x0F) * 0x100 + self.regs(0x4086) as i32;
            }

            let mut end_time: BlipTime = self.last_time;
            loop {
                // sweep
                if sweep_time <= end_time {
                    sweep_time += sweep_period;
                    let mode = (self.regs(0x4084) as i32) >> 5 & 2;
                    let new_sweep_gain = self.sweep_gain + mode - 1;
                    if new_sweep_gain as u32 <= 0x80u32 >> mode {
                        self.sweep_gain = new_sweep_gain;
                    } else {
                        let r = self.regs(0x4084) | 0x80;
                        self.set_regs(0x4084, r); // optimization only
                    }
                }

                // envelope
                if env_time <= end_time {
                    env_time += env_period;
                    let mode = (self.regs(0x4080) as i32) >> 5 & 2;
                    let new_env_gain = self.env_gain + mode - 1;
                    if new_env_gain as u32 <= 0x80u32 >> mode {
                        self.env_gain = new_env_gain;
                    } else {
                        let r = self.regs(0x4080) | 0x80;
                        self.set_regs(0x4080, r); // optimization only
                    }
                }

                // new end_time
                let start_time = end_time;
                end_time = final_end_time;
                if end_time > env_time {
                    end_time = env_time;
                }
                if end_time > sweep_time {
                    end_time = sweep_time;
                }

                // frequency modulation
                let mut freq = wave_freq;
                if mod_freq != 0 {
                    // time of next modulation clock
                    let mod_time = start_time + (self.mod_fract + mod_freq - 1) / mod_freq;
                    if end_time > mod_time {
                        end_time = mod_time;
                    }

                    // run modulator up to next clock and save old sweep_bias
                    let mut sweep_bias = self.regs(0x4085) as i32;
                    self.mod_fract -= (end_time - start_time) * mod_freq;
                    if self.mod_fract <= 0 {
                        self.mod_fract += FRACT_RANGE;
                        // check( (unsigned) mod_fract <= fract_range );

                        static MOD_TABLE: [i16; 8] = [0, 1, 2, 4, 0, -4, -2, -1];
                        let m = self.mod_wave[self.mod_pos as usize] as i32;
                        self.mod_pos = (self.mod_pos + 1) & (WAVE_SIZE as i32 - 1);
                        let mut new_sweep_bias = (sweep_bias + MOD_TABLE[m as usize] as i32) & 0x7F;
                        if m == 4 {
                            new_sweep_bias = 0;
                        }
                        self.set_regs(0x4085, new_sweep_bias as u8);
                    }

                    // apply frequency modulation
                    sweep_bias = (sweep_bias ^ 0x40) - 0x40;
                    let mut factor = sweep_bias * self.sweep_gain;
                    let extra = factor & 0x0F;
                    factor >>= 4;
                    if extra != 0 {
                        factor -= 1;
                        if sweep_bias >= 0 {
                            factor += 3;
                        }
                    }
                    if factor > 193 {
                        factor -= 258;
                    }
                    if factor < -64 {
                        factor += 256;
                    }
                    freq += (freq * factor) >> 6;
                    if freq <= 0 {
                        if end_time < final_end_time {
                            continue;
                        }
                        break;
                    }
                }

                // wave
                let mut wave_fract = self.wave_fract;
                let mut delay: BlipTime = (wave_fract + freq - 1) / freq;
                let mut time: BlipTime = start_time + delay;

                if time <= end_time {
                    // at least one wave clock within start_time...end_time

                    let min_delay: BlipTime = FRACT_RANGE / freq;
                    let mut wave_pos = self.wave_pos;

                    let mut volume = self.env_gain;
                    if volume > VOL_MAX {
                        volume = VOL_MAX;
                    }
                    volume *= master_volume;

                    let min_fract = min_delay * freq;

                    loop {
                        // clock wave
                        let amp = self.regs_[wave_pos as usize] as i32 * volume;
                        wave_pos = (wave_pos + 1) & (WAVE_SIZE as i32 - 1);
                        let delta = amp - self.last_amp;
                        if delta != 0 {
                            self.last_amp = amp;
                            self.synth.offset_inline(time, delta, output_);
                        }

                        wave_fract += FRACT_RANGE - delay * freq;
                        // check( unsigned (fract_range - wave_fract) < freq );

                        // delay until next clock
                        delay = min_delay;
                        if wave_fract > min_fract {
                            delay += 1;
                        }
                        // check( delay && delay == (wave_fract + freq - 1) / freq );

                        time += delay;
                        if time > end_time {
                            break; // TODO: using < breaks things, but <= is wrong
                        }
                    }

                    self.wave_pos = wave_pos;
                }
                self.wave_fract = wave_fract - (end_time - (time - delay)) * freq;
                // check( this->wave_fract > 0 );
                if end_time >= final_end_time {
                    break;
                }
            }

            self.env_delay = env_time - final_end_time; // check( env_delay >= 0 );
            self.sweep_delay = sweep_time - final_end_time; // check( sweep_delay >= 0 );
        }
        self.last_time = final_end_time;
    }
}
