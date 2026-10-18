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

//! Atari POKEY sound chip emulator (`Sap_Apu.h`, `Sap_Apu.cpp`).
//!
//! The `Sap_Apu_Impl` the two POKEYs of a stereo rip share is passed to the
//! functions that use it (upstream keeps a pointer to it).

use crate::blip_buffer::{BlipBuffer, BlipSynth, BlipTime, BufId, BLIP_GOOD_QUALITY};

pub const OSC_COUNT: i32 = 4;

pub const START_ADDR: u32 = 0xD200;
pub const END_ADDR: u32 = 0xD209;

const POLY4_LEN: i32 = (1 << 4) - 1;
const POLY9_LEN: i32 = (1 << 9) - 1;
const POLY17_LEN: i32 = (1 << 17) - 1;

#[derive(Clone, Copy, Debug, Default)]
struct Osc {
    regs: [u8; 2],
    phase: u8,
    invert: u8,
    last_amp: i32,
    delay: BlipTime,
    period: BlipTime, // always recalculated before use; here for convenience
    output: Option<BufId>,
}

#[derive(Clone, Debug, Default)]
pub struct SapApu {
    oscs: [Osc; OSC_COUNT as usize],
    last_time: BlipTime,
    poly5_pos: i32,
    poly4_pos: i32,
    polym_pos: i32,
    control: i32,
}

/// Common tables and Blip_Synth that can be shared among multiple Sap_Apu objects
#[derive(Clone, Debug)]
pub struct SapApuImpl {
    pub synth: BlipSynth,
    poly4: [u8; (POLY4_LEN / 8 + 1) as usize],
    poly9: [u8; (POLY9_LEN / 8 + 1) as usize],
    poly17: Box<[u8; (POLY17_LEN / 8 + 1) as usize]>,
}

static MAX_FREQUENCY: i32 = 12000; // pure waves above this frequency are silenced

fn gen_poly(mask: u32, out: &mut [u8]) {
    let mut n: u32 = 1;
    for o in out.iter_mut() {
        let mut bits = 0u32;
        for b in 0..8 {
            // implemented using "Galios configuration"
            bits |= (n & 1) << b;
            n = (n >> 1) ^ (mask & (n & 1).wrapping_neg());
        }
        *o = bits as u8;
    }
}

// poly5
const POLY5_LEN: i32 = (1 << 5) - 1;
const POLY5_MASK: u32 = ((1u64 << POLY5_LEN) - 1) as u32;
const POLY5: u32 = 0x167C6EA1;

#[inline]
fn run_poly5(input: u32, shift: i32) -> u32 {
    (input << shift & POLY5_MASK) | (input >> (POLY5_LEN - shift))
}

const fn poly_mask(width: u32, tap1: u32, tap2: u32) -> u32 {
    (1 << (width - 1 - tap1)) | (1 << (width - 1 - tap2))
}

impl Default for SapApuImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl SapApuImpl {
    pub fn new() -> Self {
        let mut imp = SapApuImpl {
            synth: BlipSynth::new(BLIP_GOOD_QUALITY, 1),
            poly4: [0; (POLY4_LEN / 8 + 1) as usize],
            poly9: [0; (POLY9_LEN / 8 + 1) as usize],
            poly17: Box::new([0; (POLY17_LEN / 8 + 1) as usize]),
        };
        gen_poly(poly_mask(4, 1, 0), &mut imp.poly4);
        gen_poly(poly_mask(9, 5, 0), &mut imp.poly9);
        gen_poly(poly_mask(17, 5, 0), &mut imp.poly17[..]);

        // (if ( 0 ): the recalculation of the poly5 constant)
        imp
    }

    pub fn volume(&mut self, d: f64) {
        self.synth.volume(1.0 / OSC_COUNT as f64 / 30.0 * d);
    }
}

impl SapApu {
    pub fn new() -> Self {
        let mut apu = Self::default();
        for i in 0..OSC_COUNT {
            apu.osc_output(i, None);
        }
        apu
    }

    pub fn osc_output(&mut self, i: i32, b: Option<BufId>) {
        // assert( (unsigned) i < osc_count );
        self.oscs[i as usize].output = b;
    }

    pub fn reset(&mut self) {
        self.last_time = 0;
        self.poly5_pos = 0;
        self.poly4_pos = 0;
        self.polym_pos = 0;
        self.control = 0;

        for osc in self.oscs.iter_mut() {
            // memset( &oscs [i], 0, offsetof (osc_t,output) );
            let output = osc.output;
            *osc = Osc {
                output,
                ..Osc::default()
            };
        }
    }

    #[inline]
    fn calc_periods(&mut self) {
        // 15/64 kHz clock
        let mut divider = 28;
        if self.control & 1 != 0 {
            divider = 114;
        }

        for i in 0..OSC_COUNT as usize {
            let osc_reload = self.oscs[i].regs[0] as i32; // cache
            let mut period: i32 = (osc_reload + 1) * divider;
            static FAST_BITS: [u8; OSC_COUNT as usize] = [1 << 6, 1 << 4, 1 << 5, 1 << 3];
            if self.control & FAST_BITS[i] as i32 != 0 {
                period = osc_reload + 4;
                if i & 1 != 0 {
                    period = osc_reload * 0x100 + self.oscs[i - 1].regs[0] as i32 + 7;
                    if self.control & FAST_BITS[i - 1] as i32 == 0 {
                        period = (period - 6) * divider;
                    }

                    // if ( (osc [-1].regs [1] & 0x1F) > 0x10 )
                    //  debug_printf( "Use of slave channel in 16-bit mode not supported\n" );
                }
            }
            self.oscs[i].period = period;
        }
    }

    fn run_until(&mut self, imp: &SapApuImpl, bufs: &mut [BlipBuffer], end_time: BlipTime) {
        self.calc_periods();

        // 17/9-bit poly selection
        let mut polym: &[u8] = &imp.poly17[..];
        let mut polym_len = POLY17_LEN;
        if self.control & 0x80 != 0 {
            polym_len = POLY9_LEN;
            polym = &imp.poly9;
        }
        self.polym_pos %= polym_len;

        let last_time = self.last_time;
        for i in 0..OSC_COUNT as usize {
            let mut time: BlipTime = last_time + self.oscs[i].delay;
            let period: BlipTime = self.oscs[i].period;

            // output
            if let Some(out) = self.oscs[i].output {
                let output = &mut bufs[out];
                output.set_modified();

                let osc_control = self.oscs[i].regs[1] as i32; // cache
                let mut volume = (osc_control & 0x0F) * 2;
                if volume == 0
                    || osc_control & 0x10 != 0 // silent, DAC mode, or inaudible frequency
                    || ((osc_control & 0xA0) == 0xA0 && period < 1789773 / 2 / MAX_FREQUENCY)
                {
                    if osc_control & 0x10 == 0 {
                        volume >>= 1; // inaudible frequency = half volume
                    }

                    let osc = &mut self.oscs[i];
                    let delta = volume - osc.last_amp;
                    if delta != 0 {
                        osc.last_amp = volume;
                        imp.synth.offset(last_time, delta, output);
                    }

                    // TODO: doesn't maintain high pass flip-flop (very minor issue)
                } else {
                    // high pass
                    static HIPASS_BITS: [u8; OSC_COUNT as usize] = [1 << 2, 1 << 1, 0, 0];
                    let mut period2: BlipTime = 0; // unused if no high pass
                    let mut time2: BlipTime = end_time;
                    if self.control & HIPASS_BITS[i] as i32 != 0 {
                        period2 = self.oscs[i + 2].period;
                        time2 = last_time + self.oscs[i + 2].delay;
                        if self.oscs[i].invert != 0 {
                            // trick inner wave loop into inverting output
                            self.oscs[i].last_amp -= volume;
                            volume = -volume;
                        }
                    }

                    let osc = &mut self.oscs[i];
                    if time < end_time || time2 < end_time {
                        // poly source
                        static POLY1: [u8; 2] = [0x55, 0x55]; // square wave
                        let mut poly: &[u8] = &POLY1;
                        let mut poly_len: i32 = 8 * POLY1.len() as i32; // can be just 2 bits, but this is faster
                        let mut poly_pos: i32 = (osc.phase & 1) as i32;
                        let mut poly_inc: i32 = 1;
                        if osc_control & 0x20 == 0 {
                            poly = polym;
                            poly_len = polym_len;
                            poly_pos = self.polym_pos;
                            if osc_control & 0x40 != 0 {
                                poly = &imp.poly4;
                                poly_len = POLY4_LEN;
                                poly_pos = self.poly4_pos;
                            }
                            poly_inc = period % poly_len;
                            poly_pos = (poly_pos + osc.delay) % poly_len;
                        }
                        poly_inc -= poly_len; // allows more optimized inner loop below

                        // square/poly5 wave
                        let mut wave: u32 = POLY5;
                        // check( poly5 & 1 ); // low bit is set for pure wave
                        let mut poly5_inc = 0;
                        if osc_control & 0x80 == 0 {
                            wave = run_poly5(wave, (osc.delay + self.poly5_pos) % POLY5_LEN);
                            poly5_inc = period % POLY5_LEN;
                        }

                        // Run wave and high pass interleved with each catching up to the other.
                        // Disabled high pass has no performance effect since inner wave loop
                        // makes no compromise for high pass, and only runs once in that case.
                        let mut osc_last_amp = osc.last_amp;
                        loop {
                            // run high pass
                            if time2 < time {
                                let mut delta = -osc_last_amp;
                                if volume < 0 {
                                    delta += volume;
                                }
                                if delta != 0 {
                                    osc_last_amp += delta - volume;
                                    volume = -volume;
                                    imp.synth.offset(time2, delta, output);
                                }
                            }
                            while time2 <= time {
                                // must advance *past* time to avoid hang
                                time2 += period2;
                            }

                            // run wave
                            let mut end: BlipTime = end_time;
                            if end > time2 {
                                end = time2;
                            }
                            while time < end {
                                if wave & 1 != 0 {
                                    let amp = volume
                                        & -((poly[(poly_pos >> 3) as usize] >> (poly_pos & 7) & 1)
                                            as i32);
                                    poly_pos += poly_inc;
                                    if poly_pos < 0 {
                                        poly_pos += poly_len;
                                    }
                                    let delta = amp - osc_last_amp;
                                    if delta != 0 {
                                        osc_last_amp = amp;
                                        imp.synth.offset(time, delta, output);
                                    }
                                }
                                wave = run_poly5(wave, poly5_inc);
                                time += period;
                            }
                            if !(time < end_time || time2 < end_time) {
                                break;
                            }
                        }

                        osc.phase = poly_pos as u8;
                        osc.last_amp = osc_last_amp;
                    }

                    osc.invert = 0;
                    if volume < 0 {
                        // undo inversion trickery
                        osc.last_amp -= volume;
                        osc.invert = 1;
                    }
                }
            }

            // maintain divider
            let osc = &mut self.oscs[i];
            let remain: BlipTime = end_time - time;
            if remain > 0 {
                let count: i32 = (remain + period - 1) / period;
                osc.phase ^= count as u8;
                time += count * period;
            }
            osc.delay = time - end_time;
        }

        // advance polies
        let duration: BlipTime = end_time - last_time;
        self.last_time = end_time;
        self.poly4_pos = (self.poly4_pos + duration) % POLY4_LEN;
        self.poly5_pos = (self.poly5_pos + duration) % POLY5_LEN;
        self.polym_pos += duration; // will get %'d on next call
    }

    pub fn write_data(
        &mut self,
        imp: &SapApuImpl,
        bufs: &mut [BlipBuffer],
        time: BlipTime,
        addr: u32,
        data: i32,
    ) {
        self.run_until(imp, bufs, time);
        let i = ((addr ^ 0xD200) >> 1) as i32;
        if i < OSC_COUNT {
            self.oscs[i as usize].regs[(addr & 1) as usize] = data as u8;
        } else if addr == 0xD208 {
            self.control = data;
        } else if addr == 0xD209 {
            self.oscs[0].delay = 0;
            self.oscs[1].delay = 0;
            self.oscs[2].delay = 0;
            self.oscs[3].delay = 0;
        }
        /*
        // TODO: are polynomials reset in this case?
        else if ( addr == 0xD20F )
        {
            if ( (data & 3) == 0 )
                polym_pos = 0;
        }
        */
    }

    pub fn end_frame(&mut self, imp: &SapApuImpl, bufs: &mut [BlipBuffer], end_time: BlipTime) {
        if end_time > self.last_time {
            self.run_until(imp, bufs, end_time);
        }

        self.last_time -= end_time;
    }
}
