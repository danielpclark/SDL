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

//! NES 2A03 APU sound chip emulator (`Nes_Apu.h`, `Nes_Apu.cpp`, and the
//! DMC functions of `Nes_Oscs.cpp` that use the APU).
//!
//! The functions that run the oscillators take the buffers they write to,
//! and the DMC's memory reader (upstream's `dmc_reader()` callback). The
//! IRQ notifier, which nothing in the library sets, and the state saving
//! functions, which nothing calls, are left out.

use crate::blip_buffer::{BlipBuffer, BlipEq, BlipSynth, BufId};
use crate::nes_cpu::{NesAddr, NesTime};
use crate::nes_oscs::*;

const AMP_RANGE: i32 = 15;

pub const START_ADDR: u32 = 0x4000;
pub const END_ADDR: u32 = 0x4017;
/// Read from status register at 0x4015
pub const STATUS_ADDR: u32 = 0x4015;
pub const OSC_COUNT: i32 = 5;
pub const NO_IRQ: NesTime = (i32::MAX / 2 + 1) as NesTime;
pub const IRQ_WAITING: NesTime = 0;

/// The DMC's memory reader (`prg_reader`).
pub type DmcReader<'a> = &'a dyn Fn(NesAddr) -> i32;

#[derive(Clone, Debug)]
pub struct NesApu {
    pub square1: NesSquare,
    pub square2: NesSquare,
    pub noise: NesNoise,
    pub triangle: NesTriangle,
    pub dmc: NesDmc,

    tempo_: f64,
    /// has been run until this time in current frame
    last_time: NesTime,
    last_dmc_time: NesTime,
    earliest_irq_: NesTime,
    next_irq: NesTime,
    frame_period: i32,
    /// cycles until frame counter runs next
    frame_delay: i32,
    /// current frame (0-3)
    frame: i32,
    osc_enables: i32,
    frame_mode: i32,
    irq_flag: bool,

    /// shared by squares
    square_synth: BlipSynth,
}

static LENGTH_TABLE: [u8; 0x20] = [
    0x0A, 0xFE, 0x14, 0x02, 0x28, 0x04, 0x50, 0x06, 0xA0, 0x08, 0x3C, 0x0A, 0x0E, 0x0C, 0x1A, 0x0E,
    0x0C, 0x10, 0x18, 0x12, 0x30, 0x14, 0x60, 0x16, 0xC0, 0x18, 0x48, 0x1A, 0x10, 0x1C, 0x20, 0x1E,
];

impl Default for NesApu {
    fn default() -> Self {
        Self::new()
    }
}

impl NesApu {
    pub fn new() -> Self {
        let mut apu = NesApu {
            square1: NesSquare::default(),
            square2: NesSquare::default(),
            noise: NesNoise::default(),
            triangle: NesTriangle::default(),
            dmc: NesDmc::default(),
            tempo_: 1.0,
            last_time: 0,
            last_dmc_time: 0,
            earliest_irq_: 0,
            next_irq: 0,
            frame_period: 0,
            frame_delay: 0,
            frame: 0,
            osc_enables: 0,
            // (upstream leaves it uninitialized; reset() writes $4017, which sets it)
            frame_mode: 0,
            irq_flag: false,
            square_synth: new_square_synth(),
        };

        apu.output(None);
        apu.volume(1.0);
        apu.reset(false, 0);
        apu
    }

    fn osc(&mut self, i: i32) -> &mut NesOsc {
        match i {
            0 => &mut self.square1.env.osc,
            1 => &mut self.square2.env.osc,
            2 => &mut self.triangle.osc,
            3 => &mut self.noise.env.osc,
            _ => &mut self.dmc.osc,
        }
    }

    /// Set treble equalization (see notes.txt)
    pub fn treble_eq(&mut self, eq: &BlipEq) {
        self.square_synth.treble_eq(eq);
        self.triangle.synth.treble_eq(eq);
        self.noise.synth.treble_eq(eq);
        self.dmc.synth.treble_eq(eq);
    }

    /// Set overall volume (default is 1.0)
    pub fn volume(&mut self, v: f64) {
        self.dmc.nonlinear = false;
        self.square_synth.volume(0.1128 / AMP_RANGE as f64 * v);
        self.triangle.synth.volume(0.12765 / AMP_RANGE as f64 * v);
        self.noise.synth.volume(0.0741 / AMP_RANGE as f64 * v);
        self.dmc.synth.volume(0.42545 / 127.0 * v);
    }

    /// Set buffer to generate all sound into, or disable sound if NULL
    pub fn output(&mut self, buffer: Option<BufId>) {
        for i in 0..OSC_COUNT {
            self.osc_output(i, buffer);
        }
    }

    /// Set sound output of specific oscillator to buffer. If buffer is NULL,
    /// the specified oscillator is muted and emulation accuracy is reduced.
    /// The oscillators are indexed as follows: 0) Square 1, 1) Square 2,
    /// 2) Triangle, 3) Noise, 4) DMC.
    pub fn osc_output(&mut self, osc: i32, buf: Option<BufId>) {
        // assert( (unsigned) osc < osc_count );
        self.osc(osc).output = buf;
    }

    /// Adjust frame period
    pub fn set_tempo(&mut self, t: f64) {
        self.tempo_ = t;
        self.frame_period = if self.dmc.pal_mode { 8314 } else { 7458 };
        if t != 1.0 {
            self.frame_period = ((self.frame_period as f64 / t) as i32) & !1; // must be even
        }
    }

    /// Reset internal frame counter, registers, and all oscillators.
    /// Use PAL timing if pal_timing is true, otherwise use NTSC timing.
    /// Set the DMC oscillator's initial DAC value to initial_dmc_dac without
    /// any audible click.
    pub fn reset(&mut self, pal_mode: bool, initial_dmc_dac: i32) {
        self.dmc.pal_mode = pal_mode;
        let t = self.tempo_;
        self.set_tempo(t);

        self.square1.reset();
        self.square2.reset();
        self.triangle.reset();
        self.noise.reset();
        self.dmc_reset();

        self.last_time = 0;
        self.last_dmc_time = 0;
        self.osc_enables = 0;
        self.irq_flag = false;
        self.earliest_irq_ = NO_IRQ;
        self.frame_delay = 1;
        // (no DMC reads can happen here, with the DMC disabled)
        let no_reader: DmcReader = &|_| 0;
        let mut no_bufs: [BlipBuffer; 0] = [];
        self.write_register(&mut no_bufs, no_reader, 0, 0x4017, 0x00);
        self.write_register(&mut no_bufs, no_reader, 0, 0x4015, 0x00);

        for addr in START_ADDR..=0x4013 {
            self.write_register(
                &mut no_bufs,
                no_reader,
                0,
                addr,
                if addr & 3 != 0 { 0x00 } else { 0x10 },
            );
        }

        self.dmc.dac = initial_dmc_dac;
        if !self.dmc.nonlinear {
            self.triangle.osc.last_amp = 15;
        }
        if !self.dmc.nonlinear {
            // TODO: remove?
            self.dmc.osc.last_amp = initial_dmc_dac; // prevent output transition
        }
    }

    fn irq_changed(&mut self) {
        let mut new_irq = self.dmc.next_irq;
        if self.dmc.irq_flag | self.irq_flag {
            new_irq = 0;
        } else if new_irq > self.next_irq {
            new_irq = self.next_irq;
        }

        if new_irq != self.earliest_irq_ {
            self.earliest_irq_ = new_irq;
            // (no irq_notifier_)
        }
    }

    /// Get time that APU-generated IRQ will occur if no further register reads
    /// or writes occur. If IRQ is already pending, returns irq_waiting. If no
    /// IRQ will occur, returns no_irq.
    pub fn earliest_irq(&self, _t: NesTime) -> NesTime {
        self.earliest_irq_
    }

    // frames

    /// Run DMC until specified time, so that any DMC memory reads can be
    /// accounted for (i.e. inserting CPU wait states).
    pub fn run_until(&mut self, bufs: &mut [BlipBuffer], reader: DmcReader, end_time: NesTime) {
        // require( end_time >= last_dmc_time );
        if end_time > self.next_dmc_read_time() {
            let start = self.last_dmc_time;
            self.last_dmc_time = end_time;
            self.dmc_run(bufs, reader, start, end_time);
        }
    }

    fn run_until_(&mut self, bufs: &mut [BlipBuffer], reader: DmcReader, end_time: NesTime) {
        // require( end_time >= last_time );

        if end_time == self.last_time {
            return;
        }

        if self.last_dmc_time < end_time {
            let start = self.last_dmc_time;
            self.last_dmc_time = end_time;
            self.dmc_run(bufs, reader, start, end_time);
        }

        loop {
            // earlier of next frame time or end time
            let mut time = self.last_time + self.frame_delay;
            if time > end_time {
                time = end_time;
            }
            self.frame_delay -= time - self.last_time;

            // run oscs to present
            self.square1
                .run(&self.square_synth, bufs, self.last_time, time);
            self.square2
                .run(&self.square_synth, bufs, self.last_time, time);
            self.triangle.run(bufs, self.last_time, time);
            self.noise.run(bufs, self.last_time, time);
            self.last_time = time;

            if time == end_time {
                break; // no more frames to run
            }

            // take frame-specific actions
            self.frame_delay = self.frame_period;
            let frame = self.frame;
            self.frame += 1;
            match frame {
                0 | 2 => {
                    if frame == 0 && self.frame_mode & 0xC0 == 0 {
                        self.next_irq = time + self.frame_period * 4 + 2;
                        self.irq_flag = true;
                    }
                    // fall through
                    // clock length and sweep on frames 0 and 2
                    self.square1.env.osc.clock_length(0x20);
                    self.square2.env.osc.clock_length(0x20);
                    self.noise.env.osc.clock_length(0x20);
                    self.triangle.osc.clock_length(0x80); // different bit for halt flag on triangle

                    self.square1.clock_sweep(-1);
                    self.square2.clock_sweep(0);

                    // frame 2 is slightly shorter in mode 1
                    if self.dmc.pal_mode && self.frame == 3 {
                        self.frame_delay -= 2;
                    }
                }

                1 => {
                    // frame 1 is slightly shorter in mode 0
                    if !self.dmc.pal_mode {
                        self.frame_delay -= 2;
                    }
                }

                3 => {
                    self.frame = 0;

                    // frame 3 is almost twice as long in mode 1
                    if self.frame_mode & 0x80 != 0 {
                        self.frame_delay +=
                            self.frame_period - if self.dmc.pal_mode { 2 } else { 6 };
                    }
                }

                _ => {}
            }

            // clock envelopes and linear counter every frame
            self.triangle.clock_linear_counter();
            self.square1.env.clock_envelope();
            self.square2.env.clock_envelope();
            self.noise.env.clock_envelope();
        }
    }

    /// Run all oscillators up to specified time, end current time frame, then
    /// start a new time frame at time 0. Time frames have no effect on emulation
    /// and each can be whatever length is convenient.
    pub fn end_frame(&mut self, bufs: &mut [BlipBuffer], reader: DmcReader, end_time: NesTime) {
        if end_time > self.last_time {
            self.run_until_(bufs, reader, end_time);
        }

        if self.dmc.nonlinear {
            let t = self.last_time;
            zero_apu_osc(&mut self.square1.env.osc, &self.square_synth, bufs, t);
            zero_apu_osc(&mut self.square2.env.osc, &self.square_synth, bufs, t);
            zero_apu_osc(&mut self.triangle.osc, &self.triangle.synth, bufs, t);
            zero_apu_osc(&mut self.noise.env.osc, &self.noise.synth, bufs, t);
            zero_apu_osc(&mut self.dmc.osc, &self.dmc.synth, bufs, t);
        }

        // make times relative to new frame
        self.last_time -= end_time;
        // require( last_time >= 0 );

        self.last_dmc_time -= end_time;
        // require( last_dmc_time >= 0 );

        if self.next_irq != NO_IRQ {
            self.next_irq -= end_time;
            // check( next_irq >= 0 );
        }
        if self.dmc.next_irq != NO_IRQ {
            self.dmc.next_irq -= end_time;
            // check( dmc.next_irq >= 0 );
        }
        if self.earliest_irq_ != NO_IRQ {
            self.earliest_irq_ -= end_time;
            if self.earliest_irq_ < 0 {
                self.earliest_irq_ = 0;
            }
        }
    }

    // registers

    /// Write to register (0x4000-0x4017, except 0x4014 and 0x4016)
    pub fn write_register(
        &mut self,
        bufs: &mut [BlipBuffer],
        reader: DmcReader,
        time: NesTime,
        addr: NesAddr,
        data: i32,
    ) {
        // require( addr > 0x20 ); // addr must be actual address (i.e. 0x40xx)
        // require( (unsigned) data <= 0xFF );

        // Ignore addresses outside range
        if addr.wrapping_sub(START_ADDR) > END_ADDR - START_ADDR {
            return;
        }

        self.run_until_(bufs, reader, time);

        if addr < 0x4014 {
            // Write to channel
            let osc_index = ((addr - START_ADDR) >> 2) as i32;

            let reg = (addr & 3) as usize;
            {
                let osc = self.osc(osc_index);
                osc.regs[reg] = data as u8;
                osc.reg_written[reg] = true;
            }

            if osc_index == 4 {
                // handle DMC specially
                self.dmc_write_register(reg as i32, data);
            } else if reg == 3 {
                // load length counter
                if (self.osc_enables >> osc_index) & 1 != 0 {
                    self.osc(osc_index).length_counter =
                        LENGTH_TABLE[((data >> 3) & 0x1F) as usize] as i32;
                }

                // reset square phase
                if osc_index == 0 {
                    self.square1.phase = SQUARE_PHASE_RANGE - 1;
                } else if osc_index == 1 {
                    self.square2.phase = SQUARE_PHASE_RANGE - 1;
                }
            }
        } else if addr == 0x4015 {
            // Channel enables
            let mut i = OSC_COUNT;
            while i > 0 {
                i -= 1;
                if (data >> i) & 1 == 0 {
                    self.osc(i).length_counter = 0;
                }
            }

            let mut recalc_irq = self.dmc.irq_flag;
            self.dmc.irq_flag = false;

            let old_enables = self.osc_enables;
            self.osc_enables = data;
            if data & 0x10 == 0 {
                self.dmc.next_irq = NO_IRQ;
                recalc_irq = true;
            } else if old_enables & 0x10 == 0 {
                self.dmc_start(reader); // dmc just enabled
            }

            if recalc_irq {
                self.irq_changed();
            }
        } else if addr == 0x4017 {
            // Frame mode
            self.frame_mode = data;

            let irq_enabled = data & 0x40 == 0;
            self.irq_flag &= irq_enabled;
            self.next_irq = NO_IRQ;

            // mode 1
            self.frame_delay &= 1;
            self.frame = 0;

            if data & 0x80 == 0 {
                // mode 0
                self.frame = 1;
                self.frame_delay += self.frame_period;
                if irq_enabled {
                    self.next_irq = time + self.frame_delay + self.frame_period * 3 + 1;
                }
            }

            self.irq_changed();
        }
    }

    /// Read from status register at 0x4015
    pub fn read_status(
        &mut self,
        bufs: &mut [BlipBuffer],
        reader: DmcReader,
        time: NesTime,
    ) -> i32 {
        self.run_until_(bufs, reader, time - 1);

        let mut result = ((self.dmc.irq_flag as i32) << 7) | ((self.irq_flag as i32) << 6);

        for i in 0..OSC_COUNT {
            if self.osc(i).length_counter != 0 {
                result |= 1 << i;
            }
        }

        self.run_until_(bufs, reader, time);

        if self.irq_flag {
            result |= 0x40;
            self.irq_flag = false;
            self.irq_changed();
        }

        //debug_printf( "%6d/%d Read $4015->$%02X\n", frame_delay, frame, result );

        result
    }

    /// Time when next DMC memory read will occur
    pub fn next_dmc_read_time(&self) -> NesTime {
        if self.dmc.osc.length_counter == 0 {
            return NO_IRQ; // not reading
        }

        self.last_dmc_time + self.dmc.osc.delay + (self.dmc.bits_remain - 1) * self.dmc.period
    }

    // Nes_Dmc

    fn dmc_reset(&mut self) {
        let d = &mut self.dmc;
        d.address = 0;
        d.dac = 0;
        d.buf = 0;
        d.bits_remain = 1;
        d.bits = 0;
        d.buf_full = false;
        d.silence = true;
        d.next_irq = NO_IRQ;
        d.irq_flag = false;
        d.irq_enabled = false;

        d.osc.reset();
        d.period = 0x1AC;
    }

    fn dmc_recalc_irq(&mut self) {
        let mut irq = NO_IRQ;
        let d = &self.dmc;
        if d.irq_enabled && d.osc.length_counter != 0 {
            irq = self.last_dmc_time
                + d.osc.delay
                + ((d.osc.length_counter - 1) * 8 + d.bits_remain - 1) * d.period
                + 1;
        }
        if irq != self.dmc.next_irq {
            self.dmc.next_irq = irq;
            self.irq_changed();
        }
    }

    fn dmc_write_register(&mut self, addr: i32, data: i32) {
        if addr == 0 {
            let d = &mut self.dmc;
            d.period = DMC_PERIOD_TABLE[d.pal_mode as usize][(data & 15) as usize] as i32;
            d.irq_enabled = (data & 0xC0) == 0x80; // enabled only if loop disabled
            d.irq_flag &= d.irq_enabled;
            self.dmc_recalc_irq();
        } else if addr == 1 {
            let d = &mut self.dmc;
            let old_dac = d.dac;
            d.dac = data & 0x7F;

            // adjust last_amp so that "pop" amplitude will be properly non-linear
            // with respect to change in dac
            let faked_nonlinear =
                d.dac - (DAC_TABLE[d.dac as usize] as i32 - DAC_TABLE[old_dac as usize] as i32);
            if !d.nonlinear {
                d.osc.last_amp = faked_nonlinear;
            }
        }
    }

    fn dmc_start(&mut self, reader: DmcReader) {
        self.dmc.reload_sample();
        self.dmc_fill_buffer(reader);
        self.dmc_recalc_irq();
    }

    fn dmc_fill_buffer(&mut self, reader: DmcReader) {
        if !self.dmc.buf_full && self.dmc.osc.length_counter != 0 {
            // require( prg_reader ); // prg_reader must be set
            let d = &mut self.dmc;
            d.buf = reader(0x8000u32.wrapping_add(d.address as u32));
            d.address = (d.address + 1) & 0x7FFF;
            d.buf_full = true;
            d.osc.length_counter -= 1;
            if d.osc.length_counter == 0 {
                if d.osc.regs[0] & DMC_LOOP_FLAG != 0 {
                    d.reload_sample();
                } else {
                    self.osc_enables &= !0x10;
                    self.dmc.irq_flag = self.dmc.irq_enabled;
                    self.dmc.next_irq = NO_IRQ;
                    self.irq_changed();
                }
            }
        }
    }

    fn dmc_run(
        &mut self,
        bufs: &mut [BlipBuffer],
        reader: DmcReader,
        mut time: NesTime,
        end_time: NesTime,
    ) {
        let dac = self.dmc.dac;
        let delta = self.dmc.osc.update_amp(dac);
        match self.dmc.osc.output {
            None => {
                self.dmc.silence = true;
            }
            Some(out) => {
                let output = &mut bufs[out];
                output.set_modified();
                if delta != 0 {
                    self.dmc.synth.offset(time, delta, output);
                }
            }
        }

        time += self.dmc.osc.delay;
        if time < end_time {
            let mut bits_remain = self.dmc.bits_remain;
            if self.dmc.silence && !self.dmc.buf_full {
                let period = self.dmc.period;
                let count = (end_time - time + period - 1) / period;
                bits_remain = (bits_remain - 1 + 8 - (count % 8)) % 8 + 1;
                time += count * period;
            } else {
                let output = self.dmc.osc.output;
                let period = self.dmc.period;
                let mut bits = self.dmc.bits;
                let mut dac = self.dmc.dac;

                loop {
                    if !self.dmc.silence {
                        let step = (bits & 1) * 4 - 2;
                        bits >>= 1;
                        if (dac + step) as u32 <= 0x7F {
                            dac += step;
                            if let Some(out) = output {
                                self.dmc.synth.offset_inline(time, step, &mut bufs[out]);
                            }
                        }
                    }

                    time += period;

                    bits_remain -= 1;
                    if bits_remain == 0 {
                        bits_remain = 8;
                        if !self.dmc.buf_full {
                            self.dmc.silence = true;
                        } else {
                            self.dmc.silence = false;
                            bits = self.dmc.buf;
                            self.dmc.buf_full = false;
                            if output.is_none() {
                                self.dmc.silence = true;
                            }
                            self.dmc_fill_buffer(reader);
                        }
                    }
                    if time >= end_time {
                        break;
                    }
                }

                self.dmc.dac = dac;
                self.dmc.osc.last_amp = dac;
                self.dmc.bits = bits;
            }
            self.dmc.bits_remain = bits_remain;
        }
        self.dmc.osc.delay = time - end_time;
    }
}

#[inline]
fn zero_apu_osc(osc: &mut NesOsc, synth: &BlipSynth, bufs: &mut [BlipBuffer], time: NesTime) {
    let output = osc.output;
    let last_amp = osc.last_amp;
    osc.last_amp = 0;
    if let Some(o) = output {
        if last_amp != 0 {
            synth.offset(time, -last_amp, &mut bufs[o]);
        }
    }
}
