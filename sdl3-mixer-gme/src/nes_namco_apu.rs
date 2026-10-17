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

//! Namco 106 sound chip emulator (`Nes_Namco_Apu.h`, `Nes_Namco_Apu.cpp`).

use crate::blip_buffer::{
    BlipBuffer, BlipEq, BlipResampledTime, BlipSynth, BlipTime, BufId, BLIP_GOOD_QUALITY,
};

pub const OSC_COUNT: i32 = 8;

/// Read/write data register is at 0x4800
pub const DATA_REG_ADDR: u32 = 0x4800;
/// Write-only address register is at 0xF800
pub const ADDR_REG_ADDR: u32 = 0xF800;

const REG_COUNT: usize = 0x80;

#[derive(Clone, Copy, Debug, Default)]
struct NamcoOsc {
    delay: i32,
    output: Option<BufId>,
    last_amp: i16,
    wave_pos: i16,
}

#[derive(Clone, Debug)]
pub struct NesNamcoApu {
    oscs: [NamcoOsc; 8],
    last_time: BlipTime,
    addr_reg: i32,
    reg: [u8; REG_COUNT],
    synth: BlipSynth,
}

impl Default for NesNamcoApu {
    fn default() -> Self {
        Self::new()
    }
}

impl NesNamcoApu {
    pub fn new() -> Self {
        let mut a = NesNamcoApu {
            oscs: [NamcoOsc::default(); 8],
            last_time: 0,
            addr_reg: 0,
            reg: [0; REG_COUNT],
            synth: BlipSynth::new(BLIP_GOOD_QUALITY, 15),
        };
        a.output(None);
        a.volume(1.0);
        a.reset();
        a
    }

    pub fn reset(&mut self) {
        self.last_time = 0;
        self.addr_reg = 0;

        self.reg = [0; REG_COUNT];

        for osc in self.oscs.iter_mut() {
            osc.delay = 0;
            osc.last_amp = 0;
            osc.wave_pos = 0;
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
        self.synth.volume(0.10 / OSC_COUNT as f64 * v);
    }

    pub fn treble_eq(&mut self, eq: &BlipEq) {
        self.synth.treble_eq(eq);
    }

    /// `access()`: the index of the register the address register selects
    #[inline]
    fn access(&mut self) -> usize {
        let addr = self.addr_reg & 0x7F;
        if self.addr_reg & 0x80 != 0 {
            self.addr_reg = (addr + 1) | 0x80;
        }
        addr as usize
    }

    pub fn write_addr(&mut self, v: i32) {
        self.addr_reg = v;
    }

    pub fn read_data(&mut self) -> i32 {
        let i = self.access();
        self.reg[i] as i32
    }

    pub fn write_data(&mut self, bufs: &mut [BlipBuffer], time: BlipTime, data: i32) {
        self.run_until(bufs, time);
        let i = self.access();
        self.reg[i] = data as u8;
    }

    pub fn end_frame(&mut self, bufs: &mut [BlipBuffer], time: BlipTime) {
        if time > self.last_time {
            self.run_until(bufs, time);
        }

        // assert( last_time >= time );
        self.last_time -= time;
    }

    fn run_until(&mut self, bufs: &mut [BlipBuffer], nes_end_time: BlipTime) {
        let active_oscs = ((self.reg[0x7F] >> 4) & 7) as i32 + 1;
        for i in (OSC_COUNT - active_oscs) as usize..OSC_COUNT as usize {
            let Some(out) = self.oscs[i].output else {
                continue;
            };
            let output = &mut bufs[out];
            output.set_modified();

            let osc = &mut self.oscs[i];
            let mut time: BlipResampledTime = output
                .resampled_time(self.last_time)
                .wrapping_add(osc.delay as u32);
            let end_time: BlipResampledTime = output.resampled_time(nes_end_time);
            osc.delay = 0;
            if time < end_time {
                let osc_reg = &self.reg[i * 8 + 0x40..i * 8 + 0x48];
                if osc_reg[4] & 0xE0 == 0 {
                    continue;
                }

                let volume = osc_reg[7] as i32 & 15;
                if volume == 0 {
                    continue;
                }

                let freq: i32 = (osc_reg[4] as i32 & 3) * 0x10000
                    + osc_reg[2] as i32 * 0x100
                    + osc_reg[0] as i32;
                if freq < 64 * active_oscs {
                    continue; // prevent low frequencies from excessively delaying freq changes
                }
                let period: BlipResampledTime = (output.resampled_duration(983040) / freq as u32)
                    .wrapping_mul(active_oscs as u32);

                let wave_size = 32 - (osc_reg[4] as i32 >> 2 & 7) * 4;
                if wave_size == 0 {
                    continue;
                }

                let mut last_amp = osc.last_amp as i32;
                let mut wave_pos = osc.wave_pos as i32;

                loop {
                    // read wave sample
                    // FIXME (upstream): the address can be past the 128
                    // registers (upstream reads past them, into the synth);
                    // it wraps at 256 samples here, as it does in the chip,
                    // and in upstream's patched build for the tests.
                    let addr = (wave_pos + osc_reg[6] as i32) & 0xFF;
                    let mut sample = (self.reg[(addr >> 1) as usize] as i32) >> (addr << 2 & 4);
                    wave_pos += 1;
                    sample = (sample & 15) * volume;

                    // output impulse if amplitude changed
                    let delta = sample - last_amp;
                    if delta != 0 {
                        last_amp = sample;
                        self.synth.offset_resampled(time, delta, output);
                    }

                    // next sample
                    time = time.wrapping_add(period);
                    if wave_pos >= wave_size {
                        wave_pos = 0;
                    }
                    if time >= end_time {
                        break;
                    }
                }

                osc.wave_pos = wave_pos as i16;
                osc.last_amp = last_amp as i16;
            }
            osc.delay = time.wrapping_sub(end_time) as i32;
        }

        self.last_time = nes_end_time;
    }
}
