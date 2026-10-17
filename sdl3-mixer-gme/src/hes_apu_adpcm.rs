// Game_Music_Emu $vers. http://www.slack.net/~ant/

/* Copyright (C) 2006-2008 Shay Green. This module is free software; you
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

//! Turbo Grafx 16 (PC Engine) ADPCM sound chip emulator
//! (`Hes_Apu_Adpcm.h`, `Hes_Apu_Adpcm.cpp`).

use crate::blip_buffer::{BlipBuffer, BlipEq, BlipSynth, BlipTime, BufId, BLIP_MED_QUALITY};

// More features

/// Same as set_output(), but for a particular channel
pub const OSC_COUNT: i32 = 1; // 0 <= chan < osc_count

/// Registers are at io_addr to io_addr+io_size-1
pub const IO_ADDR: u32 = 0x1800;
pub const IO_SIZE: u32 = 0x400;

const AMP_RANGE: i32 = 2048;

#[derive(Clone, Debug)]
struct State {
    pcmbuf: Box<[u8; 0x10000]>,
    port: [u8; 0x10],
    ad_sample: i32,
    ad_ref_index: i32,
    ad_low_nibble: bool,
    freq: i32,
    addr: u16,
    writeptr: u16,
    readptr: u16,
    playptr: u16,
    playflag: u8,
    repeatflag: u8,
    length: i32,
    playlength: i32,
    playedsamplecount: i32,
    volume: i32,
    fadetimer: i32,
    fadecount: i32,
}

#[derive(Clone, Debug)]
pub struct HesApuAdpcm {
    state: State,
    synth: BlipSynth,

    output: Option<BufId>,
    last_time: BlipTime,
    next_timer: f64,
    last_amp: i32,
}

impl Default for HesApuAdpcm {
    fn default() -> Self {
        Self::new()
    }
}

static STEPSIZE: [i16; 49] = [
    16, 17, 19, 21, 23, 25, 28, //
    31, 34, 37, 41, 45, 50, 55, //
    60, 66, 73, 80, 88, 97, 107, //
    118, 130, 143, 157, 173, 190, 209, //
    230, 253, 279, 307, 337, 371, 408, //
    449, 494, 544, 598, 658, 724, 796, //
    876, 963, 1060, 1166, 1282, 1411, 1552,
];

impl HesApuAdpcm {
    pub fn new() -> Self {
        let mut apu = HesApuAdpcm {
            // memset( &state, 0, sizeof( state ) );
            state: State {
                pcmbuf: Box::new([0; 0x10000]),
                port: [0; 0x10],
                ad_sample: 0,
                ad_ref_index: 0,
                ad_low_nibble: false,
                freq: 0,
                addr: 0,
                writeptr: 0,
                readptr: 0,
                playptr: 0,
                playflag: 0,
                repeatflag: 0,
                length: 0,
                playlength: 0,
                playedsamplecount: 0,
                volume: 0,
                fadetimer: 0,
                fadecount: 0,
            },
            synth: BlipSynth::new(BLIP_MED_QUALITY, 1),
            output: None,
            last_time: 0,
            next_timer: 0.0,
            last_amp: 0,
        };

        apu.reset();
        apu
    }

    // Basics

    /// Sets buffer(s) to generate sound into, or 0 to mute. If only center is not 0,
    /// output is mono.
    pub fn set_output(&mut self, c: Option<BufId>, l: Option<BufId>, r: Option<BufId>) {
        self.set_output_chan(0, c, l, r);
    }

    /// Sets treble equalization
    pub fn treble_eq(&mut self, eq: &BlipEq) {
        self.synth.treble_eq(eq);
    }

    /// Sets overall volume, where 1.0 is normal
    pub fn volume(&mut self, v: f64) {
        self.synth
            .volume(0.6 / OSC_COUNT as f64 / AMP_RANGE as f64 * v);
    }

    /// Resets sound chip
    pub fn reset(&mut self) {
        self.last_time = 0;
        self.next_timer = 0.0;
        self.last_amp = 0;

        self.state.pcmbuf.fill(0);
        self.state.port.fill(0);

        self.state.ad_sample = 0;
        self.state.ad_ref_index = 0;

        self.state.addr = 0;
        self.state.freq = 0;
        self.state.writeptr = 0;
        self.state.readptr = 0;
        self.state.playflag = 0;
        self.state.repeatflag = 0;
        self.state.length = 0;
        self.state.volume = 0xFF;
        self.state.fadetimer = 0;
        self.state.fadecount = 0;
    }

    pub fn set_output_chan(
        &mut self,
        _i: i32,
        center: Option<BufId>,
        left: Option<BufId>,
        right: Option<BufId>,
    ) {
        // Must be silent (all NULL), mono (left and right NULL), or stereo (none NULL)
        // require( !center || (center && !left && !right) || (center && left && right) );
        // require( (unsigned) i < osc_count ); // fails if you pass invalid osc index

        // (left and right are unused)
        let _ = (left, right);

        self.output = center;
    }

    fn run_until(&mut self, bufs: &mut [BlipBuffer], end_time: BlipTime) {
        let mut volume = self.state.volume;
        let fadetimer = self.state.fadetimer;
        let mut fadecount = self.state.fadecount;
        let mut last_time = self.last_time;
        let mut next_timer = self.next_timer;
        let mut last_amp = self.last_amp;

        let output = self.output; // cache often-used values

        while self.state.playflag != 0 && last_time < end_time {
            while last_time as f64 >= next_timer {
                if fadetimer != 0 {
                    if fadecount > 0 {
                        fadecount -= 1;
                        volume = 0xFF * fadecount / fadetimer;
                    } else if fadecount < 0 {
                        fadecount += 1;
                        volume = 0xFF - (0xFF * fadecount / fadetimer);
                    }
                }

                next_timer += 7159.091;
            }

            let mut amp;
            if self.state.ad_low_nibble {
                let code = self.state.pcmbuf[self.state.playptr as usize] & 0x0F;
                amp = self.adpcm_decode(code as i32);
                self.state.ad_low_nibble = false;
                self.state.playptr = self.state.playptr.wrapping_add(1);
                self.state.playedsamplecount = self.state.playedsamplecount.wrapping_add(1);
                if self.state.playedsamplecount == self.state.playlength {
                    self.state.playflag = 0;
                }
            } else {
                let code = self.state.pcmbuf[self.state.playptr as usize] >> 4;
                amp = self.adpcm_decode(code as i32);
                self.state.ad_low_nibble = true;
            }

            amp = amp * volume / 0xFF;
            let delta = amp - last_amp;

            if let (Some(out), true) = (output, delta != 0) {
                last_amp = amp;
                self.synth.offset(last_time, delta, &mut bufs[out]);
            }

            last_time = last_time.wrapping_add(self.state.freq);
        }

        if self.state.playflag == 0 {
            while next_timer <= end_time as f64 {
                next_timer += 7159.091;
            }

            last_time = end_time;
        }

        self.last_time = last_time;
        self.next_timer = next_timer;
        self.last_amp = last_amp;
        self.state.volume = volume;
        self.state.fadetimer = fadetimer;
        self.state.fadecount = fadecount;
    }

    /// Emulates to time t, then writes data to addr
    pub fn write_data(&mut self, bufs: &mut [BlipBuffer], time: BlipTime, addr: i32, data: i32) {
        if time > self.last_time {
            self.run_until(bufs, time);
        }

        let data = data & 0xFF;
        let state = &mut self.state;
        state.port[(addr & 15) as usize] = data as u8;
        match addr & 15 {
            8 => {
                state.addr &= 0xFF00;
                state.addr |= data as u16;
            }
            9 => {
                state.addr &= 0xFF;
                state.addr |= (data << 8) as u16;
            }
            10 => {
                state.pcmbuf[state.writeptr as usize] = data as u8;
                state.writeptr = state.writeptr.wrapping_add(1);
                state.playlength = state.playlength.wrapping_add(1);
            }
            11 => {
                // debug_printf("ADPCM DMA 0x%02X", data);
            }
            13 => {
                if data & 0x80 != 0 {
                    state.addr = 0;
                    state.freq = 0;
                    state.writeptr = 0;
                    state.readptr = 0;
                    state.playflag = 0;
                    state.repeatflag = 0;
                    state.length = 0;
                    state.volume = 0xFF;
                }

                if (data & 3) == 3 {
                    state.writeptr = state.addr;
                }

                if data & 8 != 0 {
                    state.readptr = if state.addr != 0 {
                        state.addr - 1
                    } else {
                        state.addr
                    };
                }

                if data & 0x10 != 0 {
                    state.length = state.addr as i32;
                }

                state.repeatflag = (data & 0x20) as u8;
                state.playflag = (data & 0x40) as u8;

                if state.playflag != 0 {
                    state.playptr = state.readptr;
                    state.playlength = state.length + 1;
                    state.playedsamplecount = 0;
                    state.ad_sample = 0;
                    state.ad_low_nibble = false;
                }
            }
            14 => {
                state.freq = 7159091 / (32000 / (16 - (data & 15)));
            }
            15 => match data & 15 {
                0 | 8 | 12 => {
                    state.fadetimer = -100;
                    state.fadecount = state.fadetimer;
                }
                10 => {
                    state.fadetimer = 5000;
                    state.fadecount = state.fadetimer;
                }
                14 => {
                    state.fadetimer = 1500;
                    state.fadecount = state.fadetimer;
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// Emulates to time t, then reads from addr
    pub fn read_data(&mut self, bufs: &mut [BlipBuffer], time: BlipTime, addr: i32) -> i32 {
        if time > self.last_time {
            self.run_until(bufs, time);
        }

        let state = &mut self.state;
        match addr & 15 {
            10 => {
                let v = state.pcmbuf[state.readptr as usize];
                state.readptr = state.readptr.wrapping_add(1);
                return v as i32;
            }
            11 => return (state.port[11] & !1) as i32,
            12 => {
                if state.playflag == 0 {
                    state.port[12] |= 1;
                    state.port[12] &= !8;
                } else {
                    state.port[12] &= !1;
                    state.port[12] |= 8;
                }
                return state.port[12] as i32;
            }
            13 => return state.port[13] as i32,
            _ => {}
        }

        0xFF
    }

    /// Emulates to time t, then subtracts t from the current time.
    /// OK if previous write call had time slightly after t.
    pub fn end_frame(&mut self, bufs: &mut [BlipBuffer], end_time: BlipTime) {
        self.run_until(bufs, end_time);
        self.last_time -= end_time;
        self.next_timer -= end_time as f64;
        // check( last_time >= 0 );
        if let Some(out) = self.output {
            bufs[out].set_modified();
        }
    }

    fn adpcm_decode(&mut self, code: i32) -> i32 {
        let mut step = STEPSIZE[self.state.ad_ref_index as usize] as i32;
        let mut delta;
        let c = code & 7;

        delta = 0;
        if c & 4 != 0 {
            delta += step;
        }
        step >>= 1;
        if c & 2 != 0 {
            delta += step;
        }
        step >>= 1;
        if c & 1 != 0 {
            delta += step;
        }
        step >>= 1;
        delta += step;
        // (#else: delta = ( ( c + c + 1 ) * step ) / 8; // maybe faster, but introduces rounding)

        if c != code {
            self.state.ad_sample -= delta;
            if self.state.ad_sample < -2048 {
                self.state.ad_sample = -2048;
            }
        } else {
            self.state.ad_sample += delta;
            if self.state.ad_sample > 2047 {
                self.state.ad_sample = 2047;
            }
        }

        static STEPS: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

        self.state.ad_ref_index += STEPS[c as usize];

        if self.state.ad_ref_index < 0 {
            self.state.ad_ref_index = 0;
        } else if self.state.ad_ref_index > 48 {
            self.state.ad_ref_index = 48;
        }

        self.state.ad_sample
    }
}
