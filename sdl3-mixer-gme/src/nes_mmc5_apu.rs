// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/
// (Nes_Mmc5_Apu.h carries no license notice of its own; it is part of
// Game_Music_Emu, under the GNU LGPL 2.1 or later: see COPYING.)
// Modified 2026-10-17: translated into Rust from SDL_mixer 3.3.0's Game_Music_Emu 0.6.5 (see NOTICE).

//! NES MMC5 sound chip emulator (`Nes_Mmc5_Apu.h`): an [`NesApu`] whose
//! two squares and DAC the MMC5's registers drive.

use crate::blip_buffer::{BlipBuffer, BlipEq, BlipTime, BufId};
use crate::nes_apu::{DmcReader, NesApu};

pub const REGS_ADDR: u32 = 0x5000;
pub const REGS_SIZE: u32 = 0x16;

pub const OSC_COUNT: i32 = 3;

pub const EXRAM_SIZE: usize = 1024;

/// (the MMC5 never starts its DMC, so nothing reads through this)
const NO_READER: DmcReader = &|_| 0;

#[derive(Clone, Debug)]
pub struct NesMmc5Apu {
    pub apu: NesApu,
    pub exram: [u8; EXRAM_SIZE],
}

impl Default for NesMmc5Apu {
    fn default() -> Self {
        Self::new()
    }
}

impl NesMmc5Apu {
    pub fn new() -> Self {
        NesMmc5Apu {
            apu: NesApu::new(),
            // (upstream leaves it uninitialized until a track starts)
            exram: [0; EXRAM_SIZE],
        }
    }

    pub fn osc_output(&mut self, mut i: i32, b: Option<BufId>) {
        // in: square 1, square 2, PCM
        // out: square 1, square 2, skipped, skipped, PCM
        // assert( (unsigned) i < osc_count );
        if i > 1 {
            i += 2;
        }
        self.apu.osc_output(i, b);
    }

    pub fn write_register(
        &mut self,
        bufs: &mut [BlipBuffer],
        time: BlipTime,
        addr: u32,
        mut data: i32,
    ) {
        match addr {
            0x5015 | 0x5000 | 0x5002 | 0x5003 | 0x5004 | 0x5006 | 0x5007 | 0x5011 => {
                if addr == 0x5015 {
                    // channel enables
                    data &= 0x03; // enable the square waves only
                }
                // fall through
                // Square 1, Square 2, DAC
                self.apu
                    .write_register(bufs, NO_READER, time, addr - 0x1000, data);
            }

            0x5010 => {} // some things write to this for some reason

            _ => {}
        }
    }

    pub fn volume(&mut self, v: f64) {
        self.apu.volume(v);
    }

    pub fn treble_eq(&mut self, eq: &BlipEq) {
        self.apu.treble_eq(eq);
    }

    pub fn reset(&mut self) {
        self.apu.reset(false, 0);
    }

    pub fn end_frame(&mut self, bufs: &mut [BlipBuffer], time: BlipTime) {
        self.apu.end_frame(bufs, NO_READER, time);
    }
}
