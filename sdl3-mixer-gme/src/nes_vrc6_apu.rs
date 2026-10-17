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

//! Konami VRC6 sound chip emulator (`Nes_Vrc6_Apu.h`, `Nes_Vrc6_Apu.cpp`;
//! the state saving functions, which nothing calls, are left out).

use crate::blip_buffer::{
    BlipBuffer, BlipEq, BlipSynth, BlipTime, BufId, BLIP_GOOD_QUALITY, BLIP_MED_QUALITY,
};

pub const OSC_COUNT: i32 = 3;
pub const REG_COUNT: i32 = 3;
pub const BASE_ADDR: u32 = 0x9000;
pub const ADDR_STEP: u32 = 0x1000;

#[derive(Clone, Debug, Default)]
struct Vrc6Osc {
    regs: [u8; 3],
    output: Option<BufId>,
    delay: i32,
    last_amp: i32,
    phase: i32,
    /// only used by saw
    amp: i32,
}

impl Vrc6Osc {
    fn period(&self) -> i32 {
        ((self.regs[2] as i64 & 0x0F) * 0x100 + self.regs[1] as i64 + 1) as i32
    }
}

#[derive(Clone, Debug)]
pub struct NesVrc6Apu {
    oscs: [Vrc6Osc; 3],
    last_time: BlipTime,
    saw_synth: BlipSynth,
    square_synth: BlipSynth,
}

impl Default for NesVrc6Apu {
    fn default() -> Self {
        Self::new()
    }
}

impl NesVrc6Apu {
    pub fn new() -> Self {
        let mut a = NesVrc6Apu {
            oscs: Default::default(),
            last_time: 0,
            saw_synth: BlipSynth::new(BLIP_MED_QUALITY, 1),
            square_synth: BlipSynth::new(BLIP_GOOD_QUALITY, 1),
        };
        a.output(None);
        a.volume(1.0);
        a.reset();
        a
    }

    pub fn reset(&mut self) {
        self.last_time = 0;
        for osc in self.oscs.iter_mut() {
            osc.regs = [0; 3];
            osc.delay = 0;
            osc.last_amp = 0;
            osc.phase = 1;
            osc.amp = 0;
        }
    }

    pub fn output(&mut self, buf: Option<BufId>) {
        for i in 0..OSC_COUNT {
            self.osc_output(i, buf);
        }
    }

    pub fn osc_output(&mut self, i: i32, buf: Option<BufId>) {
        // assert( (unsigned) i < osc_count );
        self.oscs[i as usize].output = buf;
    }

    pub fn volume(&mut self, v: f64) {
        let factor = 0.0967 * 2.0;
        self.saw_synth.volume(factor / 31.0 * v);
        self.square_synth.volume(factor * 0.5 / 15.0 * v);
    }

    pub fn treble_eq(&mut self, eq: &BlipEq) {
        self.saw_synth.treble_eq(eq);
        self.square_synth.treble_eq(eq);
    }

    fn run_until(&mut self, bufs: &mut [BlipBuffer], time: BlipTime) {
        // require( time >= last_time );
        self.run_square(0, bufs, time);
        self.run_square(1, bufs, time);
        self.run_saw(bufs, time);
        self.last_time = time;
    }

    pub fn write_osc(
        &mut self,
        bufs: &mut [BlipBuffer],
        time: BlipTime,
        osc_index: i32,
        reg: i32,
        data: i32,
    ) {
        // require( (unsigned) osc_index < osc_count );
        // require( (unsigned) reg < reg_count );

        self.run_until(bufs, time);
        self.oscs[osc_index as usize].regs[reg as usize] = data as u8;
    }

    pub fn end_frame(&mut self, bufs: &mut [BlipBuffer], time: BlipTime) {
        if time > self.last_time {
            self.run_until(bufs, time);
        }

        // assert( last_time >= time );
        self.last_time -= time;
    }

    fn run_square(&mut self, i: usize, bufs: &mut [BlipBuffer], end_time: BlipTime) {
        let synth = &self.square_synth;
        let osc = &mut self.oscs[i];
        let Some(out) = osc.output else {
            return;
        };
        let output = &mut bufs[out];
        output.set_modified();

        let mut volume = osc.regs[0] as i32 & 15;
        if osc.regs[2] & 0x80 == 0 {
            volume = 0;
        }

        let gate = osc.regs[0] as i32 & 0x80;
        let duty = ((osc.regs[0] as i32 >> 4) & 7) + 1;
        let delta = (if gate != 0 || osc.phase < duty {
            volume
        } else {
            0
        }) - osc.last_amp;
        let mut time = self.last_time;
        if delta != 0 {
            osc.last_amp += delta;
            synth.offset(time, delta, output);
        }

        time += osc.delay;
        osc.delay = 0;
        let period = osc.period();
        if volume != 0 && gate == 0 && period > 4 {
            if time < end_time {
                let mut phase = osc.phase;

                loop {
                    phase += 1;
                    if phase == 16 {
                        phase = 0;
                        osc.last_amp = volume;
                        synth.offset(time, volume, output);
                    }
                    if phase == duty {
                        osc.last_amp = 0;
                        synth.offset(time, -volume, output);
                    }
                    time += period;
                    if time >= end_time {
                        break;
                    }
                }

                osc.phase = phase;
            }
            osc.delay = time - end_time;
        }
    }

    fn run_saw(&mut self, bufs: &mut [BlipBuffer], end_time: BlipTime) {
        let synth = &self.saw_synth;
        let osc = &mut self.oscs[2];
        let Some(out) = osc.output else {
            return;
        };
        let output = &mut bufs[out];
        output.set_modified();

        let mut amp = osc.amp;
        let amp_step = osc.regs[0] as i32 & 0x3F;
        let mut time = self.last_time;
        let mut last_amp = osc.last_amp;
        if osc.regs[2] & 0x80 == 0 || (amp_step | amp) == 0 {
            osc.delay = 0;
            let delta = (amp >> 3) - last_amp;
            last_amp = amp >> 3;
            synth.offset(time, delta, output);
        } else {
            time += osc.delay;
            if time < end_time {
                let period = osc.period() * 2;
                let mut phase = osc.phase;

                loop {
                    phase -= 1;
                    if phase == 0 {
                        phase = 7;
                        amp = 0;
                    }

                    let delta = (amp >> 3) - last_amp;
                    if delta != 0 {
                        last_amp = amp >> 3;
                        synth.offset(time, delta, output);
                    }

                    time += period;
                    amp = (amp + amp_step) & 0xFF;
                    if time >= end_time {
                        break;
                    }
                }

                osc.phase = phase;
                osc.amp = amp;
            }

            osc.delay = time - end_time;
        }

        osc.last_amp = last_amp;
    }
}
