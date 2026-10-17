// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/
// (Nes_Vrc7_Apu.h and Nes_Vrc7_Apu.cpp carry no license notice of their
// own; they are part of Game_Music_Emu, under the GNU LGPL 2.1 or later:
// see COPYING.)
// Modified 2026-10-17: translated into Rust from SDL_mixer 3.3.0's Game_Music_Emu 0.6.5 (see NOTICE).

//! Konami VRC7 sound chip emulator (`Nes_Vrc7_Apu.h`, `Nes_Vrc7_Apu.cpp`),
//! over [`crate::emu2413`] (the snapshot functions, which nothing calls,
//! are left out).

use crate::blargg_common::BlarggErr;
use crate::blip_buffer::{BlipBuffer, BlipEq, BlipSynth, BlipTime, BufId, BLIP_MED_QUALITY};
use crate::emu2413::Opll;

pub const OSC_COUNT: i32 = 6;

const PERIOD: i32 = 36; // NES CPU clocks per FM clock

#[derive(Clone, Copy, Debug, Default)]
struct Vrc7Osc {
    regs: [u8; 3],
    output: Option<BufId>,
    last_amp: i32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Mono {
    output: Option<BufId>,
    last_amp: i32,
}

#[derive(Debug)]
pub struct NesVrc7Apu {
    oscs: [Vrc7Osc; 6],
    // (kon is never used)
    inst: [u8; 8],
    opll: Option<Box<Opll>>,
    addr: i32,
    next_time: BlipTime,
    mono: Mono,
    synth: BlipSynth,
}

impl Default for NesVrc7Apu {
    fn default() -> Self {
        Self::new()
    }
}

impl NesVrc7Apu {
    pub fn new() -> Self {
        NesVrc7Apu {
            oscs: [Vrc7Osc::default(); 6],
            inst: [0; 8],
            opll: None,
            addr: 0,
            next_time: 0,
            mono: Mono::default(),
            synth: BlipSynth::new(BLIP_MED_QUALITY, 1),
        }
    }

    pub fn init(&mut self) -> BlarggErr {
        let mut opll = Opll::new(3579545, 3579545 / 72);
        opll.set_chip_type(1);
        opll.reset_patch(1);
        self.opll = Some(opll);

        self.set_output(None);
        self.volume(1.0);
        self.reset();
        Ok(())
    }

    // DB2LIN_AMP_BITS == 11, * 2
    pub fn volume(&mut self, v: f64) {
        self.synth.volume(1.0 / 3.0 / 4096.0 * v);
    }

    pub fn treble_eq(&mut self, eq: &BlipEq) {
        self.synth.treble_eq(eq);
    }

    pub fn set_output(&mut self, buf: Option<BufId>) {
        for osc in self.oscs.iter_mut() {
            osc.output = buf;
        }
        self.output_changed();
    }

    pub fn osc_output(&mut self, i: i32, buf: Option<BufId>) {
        // assert( (unsigned) i < osc_count );
        self.oscs[i as usize].output = buf;
        self.output_changed();
    }

    fn output_changed(&mut self) {
        self.mono.output = self.oscs[0].output;
        for i in (1..OSC_COUNT as usize).rev() {
            if self.mono.output != self.oscs[i].output {
                self.mono.output = None;
                break;
            }
        }

        if self.mono.output.is_some() {
            for i in (1..OSC_COUNT as usize).rev() {
                self.mono.last_amp += self.oscs[i].last_amp;
                self.oscs[i].last_amp = 0;
            }
        }
    }

    pub fn reset(&mut self) {
        self.addr = 0;
        self.next_time = 0;
        self.mono.last_amp = 0;

        for osc in self.oscs.iter_mut().rev() {
            osc.last_amp = 0;
            osc.regs = [0; 3];
        }

        if let Some(o) = self.opll.as_mut() {
            o.reset();
        }
    }

    pub fn write_reg(&mut self, data: i32) {
        self.addr = data;
    }

    pub fn write_data(&mut self, bufs: &mut [BlipBuffer], time: BlipTime, data: i32) {
        let type_ = (self.addr >> 4) - 1;
        let chan = self.addr & 15;
        if (type_ as u32) < 3 && chan < OSC_COUNT {
            self.oscs[chan as usize].regs[type_ as usize] = data as u8;
        }

        if self.addr < 0x08 {
            // (the address is a register number from 0 to 255)
            if let Some(i) = self.inst.get_mut(self.addr as usize) {
                *i = data as u8;
            }
        }

        if time > self.next_time {
            self.run_until(bufs, time);
        }

        let opll = self.opll.as_mut().unwrap();
        opll.write_io(0, self.addr as u8);
        opll.write_io(1, data as u8);
    }

    pub fn end_frame(&mut self, bufs: &mut [BlipBuffer], time: BlipTime) {
        if time > self.next_time {
            self.run_until(bufs, time);
        }

        self.next_time -= time;
        // assert( next_time >= 0 );

        for osc in self.oscs.iter().rev() {
            if let Some(o) = osc.output {
                bufs[o].set_modified();
            }
        }
    }

    fn run_until(&mut self, bufs: &mut [BlipBuffer], end_time: BlipTime) {
        // require( end_time > next_time );

        let mut time = self.next_time;
        let opll = self.opll.as_mut().unwrap(); // cache
        let mono_output = self.mono.output;
        let mut buffer = [0i32; 2];

        if let Some(mo) = mono_output {
            // optimal case
            let out = &mut bufs[mo];
            loop {
                opll.calc_stereo(&mut buffer);
                let amp = buffer[0].wrapping_add(buffer[1]);
                let delta = amp.wrapping_sub(self.mono.last_amp);
                if delta != 0 {
                    self.mono.last_amp = amp;
                    self.synth.offset_inline(time, delta, out);
                }
                time += PERIOD;
                if time >= end_time {
                    break;
                }
            }
        } else {
            self.mono.last_amp = 0;
            loop {
                opll.calc_stereo(&mut buffer);
                for i in 0..OSC_COUNT as usize {
                    let osc = &mut self.oscs[i];
                    if let Some(o) = osc.output {
                        let amp = opll.ch_out[i] as i32;
                        let delta = amp - osc.last_amp;
                        if delta != 0 {
                            osc.last_amp = amp;
                            self.synth.offset(time, delta, &mut bufs[o]);
                        }
                    }
                }
                time += PERIOD;
                if time >= end_time {
                    break;
                }
            }
        }

        self.next_time = time;
    }
}
