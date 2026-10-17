// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/

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

//! Nintendo Game Boy GBS music file emulator (`Gbs_Emu.h`, `Gbs_Emu.cpp`,
//! `gb_cpu_io.h`).

use crate::blargg_common::{get_le16, BlarggErr};
use crate::blip_buffer::{BlipEq, BlipTime, BufId};
use crate::classic_emu::{
    classic_emu_overrides, classic_start_track_, setup_buffer, ClassicEmu, ClassicEmuState,
    RomData, MIXED_TYPE, WAVE_TYPE,
};
use crate::data_reader::{DataReader, EOF_ERROR};
use crate::gb_apu::{self, GbApu};
use crate::gb_cpu::{self, CodePage, GbAddr, GbCpu, GbCpuHost, IDLE_ADDR};
use crate::gme_file::{gme_copy_field, GmeTypeT, TrackInfo, GME_WRONG_FILE_TYPE};
use crate::music_emu::{
    gme_info_overrides, make_equalizer, music_emu_unload, Equalizer, MusicEmu, MusicEmuState,
};

/// Equalizer profiles for Game Boy Color speaker and headphones
pub const HANDHELD_EQ: Equalizer = make_equalizer(-47.0, 2000.0);
pub const HEADPHONES_EQ: Equalizer = make_equalizer(0.0, 300.0);

/// GBS file header
pub const HEADER_SIZE: usize = 112;

/// The fields of the GBS `header_t`, as offsets into its bytes.
pub mod header {
    pub const TAG: usize = 0; // [3]
    pub const VERS: usize = 3;
    pub const TRACK_COUNT: usize = 4;
    pub const FIRST_TRACK: usize = 5;
    pub const LOAD_ADDR: usize = 6; // [2]
    pub const INIT_ADDR: usize = 8; // [2]
    pub const PLAY_ADDR: usize = 10; // [2]
    pub const STACK_PTR: usize = 12; // [2]
    pub const TIMER_MODULO: usize = 14;
    pub const TIMER_MODE: usize = 15;
    pub const GAME: usize = 16; // [32]
    pub const AUTHOR: usize = 48; // [32]
    pub const COPYRIGHT: usize = 80; // [32]
}

// rom
const BANK_SIZE: i32 = 0x4000;

const JOYPAD_ADDR: u32 = 0xFF00;
const RAM_ADDR: u32 = 0xA000;
const HI_PAGE: usize = (0xFF00 - RAM_ADDR) as usize;
const RAM_SIZE: usize = 0x4000 + 0x2000 + gb_cpu::CPU_PADDING;

// Memory regions of the code map
const MEM_ROM: u8 = 0;
const MEM_RAM: u8 = 1;

static VOICE_NAMES: [&str; gb_apu::OSC_COUNT as usize] = ["Square 1", "Square 2", "Wave", "Noise"];

static VOICE_TYPES: [i32; gb_apu::OSC_COUNT as usize] =
    [WAVE_TYPE | 1, WAVE_TYPE | 2, WAVE_TYPE, MIXED_TYPE];

pub struct GbsEmu {
    pub(crate) state: MusicEmuState,
    pub(crate) classic: ClassicEmuState,
    cpu: GbCpu,

    // rom
    rom: RomData,

    // timer
    cpu_time: BlipTime,
    play_period: BlipTime,
    next_play: BlipTime,

    header_: [u8; HEADER_SIZE],

    ram: [u8; RAM_SIZE],
    apu: GbApu,
}

impl std::fmt::Debug for GbsEmu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GbsEmu").finish_non_exhaustive()
    }
}

// Track info

fn copy_gbs_fields(h: &[u8; HEADER_SIZE], out: &mut TrackInfo) {
    gme_copy_field(&mut out.game, &h[header::GAME..header::GAME + 32]);
    gme_copy_field(&mut out.author, &h[header::AUTHOR..header::AUTHOR + 32]);
    gme_copy_field(
        &mut out.copyright,
        &h[header::COPYRIGHT..header::COPYRIGHT + 32],
    );
}

fn check_gbs_header(header: &[u8]) -> BlarggErr {
    if &header[..3] != b"GBS" {
        return Err(GME_WRONG_FILE_TYPE);
    }
    Ok(())
}

/// `Gbs_File`: the info-only GBS reader
pub(crate) struct GbsFile {
    state: MusicEmuState,
    h: [u8; HEADER_SIZE],
}

impl GbsFile {
    fn new() -> Self {
        let mut state = MusicEmuState::new();
        state.set_type(&GME_GBS_TYPE_);
        GbsFile {
            state,
            h: [0; HEADER_SIZE],
        }
    }
}

impl MusicEmu for GbsFile {
    gme_info_overrides!();

    fn load_(&mut self, input: &mut dyn DataReader) -> BlarggErr {
        if let Err(err) = input.read(&mut self.h) {
            return Err(if err == EOF_ERROR {
                GME_WRONG_FILE_TYPE
            } else {
                err
            });
        }

        self.state
            .set_track_count(self.h[header::TRACK_COUNT] as i32);
        check_gbs_header(&self.h)
    }

    fn track_info_(&self, out: &mut TrackInfo, _track: i32) -> BlarggErr {
        copy_gbs_fields(&self.h, out);
        Ok(())
    }
}

fn new_gbs_emu() -> Box<dyn MusicEmu> {
    Box::new(GbsEmu::new())
}
fn new_gbs_file() -> Box<dyn MusicEmu> {
    Box::new(GbsFile::new())
}

pub(crate) static GME_GBS_TYPE_: GmeTypeT = GmeTypeT {
    system: "Game Boy",
    track_count: 0,
    new_emu: new_gbs_emu,
    new_info: new_gbs_file,
    extension_: "GBS",
    flags_: 1,
};

static SOUND_DATA: [u8; gb_apu::REGISTER_COUNT as usize] = [
    0x80, 0xBF, 0x00, 0x00, 0xBF, // square 1
    0x00, 0x3F, 0x00, 0x00, 0xBF, // square 2
    0x7F, 0xFF, 0x9F, 0x00, 0xBF, // wave
    0x00, 0xFF, 0x00, 0x00, 0xBF, // noise
    0x77, 0xF3, 0xF1, // vin/volume, status, power mode
    0, 0, 0, 0, 0, 0, 0, 0, 0, // unused
    0xAC, 0xDD, 0xDA, 0x48, 0x36, 0x02, 0xCF, 0x16, // waveform data
    0x2C, 0x04, 0xE5, 0x2C, 0xAC, 0xDD, 0xDA, 0x48,
];

impl Default for GbsEmu {
    fn default() -> Self {
        Self::new()
    }
}

impl GbsEmu {
    pub fn new() -> Self {
        let mut state = MusicEmuState::new();
        state.set_type(&GME_GBS_TYPE_);
        state.set_voice_names(&VOICE_NAMES);
        state.set_silence_lookahead(6);
        state.set_max_initial_silence(21);
        state.set_gain(1.2);
        let mut classic = ClassicEmuState::new();
        classic.set_voice_types(&VOICE_TYPES);
        let mut e = GbsEmu {
            state,
            classic,
            cpu: GbCpu::new(),
            rom: RomData::new(BANK_SIZE),
            cpu_time: 0,
            play_period: 0,
            next_play: 0,
            header_: [0; HEADER_SIZE],
            // (upstream leaves it uninitialized until a track starts:
            // update_timer() reads the timer registers in it at load, for
            // a period start_track_() recomputes)
            ram: [0; RAM_SIZE],
            apu: GbApu::new(),
        };

        e.set_equalizer(&make_equalizer(-1.0, 120.0));
        e
    }

    /// Header for currently loaded file
    pub fn header(&self) -> &[u8; HEADER_SIZE] {
        &self.header_
    }

    fn clock(&self) -> BlipTime {
        self.cpu_time - self.cpu.remain()
    }

    // Emulation

    // see gb_cpu_io.h for read/write functions

    fn set_bank(&mut self, mut n: i32) {
        // Only valid for MBC1 cartridges, but hopefully shouldn't hurt
        n &= 0x1f;
        if n == 0 {
            n = 1;
        }

        let addr = n * BANK_SIZE;
        if addr as i64 > self.rom.size() {
            return;
        }
        let at = self.rom.at_addr(self.rom.mask_addr(addr));
        self.cpu.map_code(
            BANK_SIZE as u32,
            BANK_SIZE as u32,
            CodePage {
                mem: MEM_ROM,
                offset: at,
            },
        );
    }

    fn update_timer(&mut self) {
        if self.header_[header::TIMER_MODE] & 0x04 != 0 {
            static RATES: [u8; 4] = [10, 4, 6, 8];
            let shift = RATES[(self.ram[HI_PAGE + 7] & 3) as usize] as i32
                - (self.header_[header::TIMER_MODE] >> 7) as i32;
            self.play_period = ((256i64 - self.ram[HI_PAGE + 6] as i64) << shift) as BlipTime;
        } else {
            self.play_period = 70224; // 59.73 Hz
        }
        if self.state.tempo() != 1.0 {
            self.play_period = (self.play_period as f64 / self.state.tempo()) as BlipTime;
        }
    }

    fn cpu_jsr(&mut self, addr: GbAddr) {
        // check( cpu::r.sp == get_le16( header_.stack_ptr ) );
        self.cpu.r.pc = addr as i64;
        self.cpu.r.sp = self.cpu.r.sp.wrapping_sub(1);
        let sp = self.cpu.r.sp;
        self.cpu_write(sp as GbAddr, (IDLE_ADDR >> 8) as i32);
        self.cpu.r.sp = self.cpu.r.sp.wrapping_sub(1);
        let sp = self.cpu.r.sp;
        self.cpu_write(sp as GbAddr, (IDLE_ADDR & 0xFF) as i32);
    }

    fn gbs_load_(&mut self, input: &mut dyn DataReader) -> BlarggErr {
        self.rom
            .load(input, HEADER_SIZE as i32, &mut self.header_, 0)?;

        self.state
            .set_track_count(self.header_[header::TRACK_COUNT] as i32);
        check_gbs_header(&self.header_)?;

        if self.header_[header::VERS] != 1 {
            self.state.set_warning("Unknown file version");
        }

        if self.header_[header::TIMER_MODE] & 0x78 != 0 {
            self.state.set_warning("Invalid timer mode");
        }

        let load_addr = get_le16(&self.header_[header::LOAD_ADDR..]);
        if (self.header_[header::LOAD_ADDR + 1]
            | self.header_[header::INIT_ADDR + 1]
            | self.header_[header::PLAY_ADDR + 1])
            > 0x7F
            || load_addr < 0x400
        {
            self.state.set_warning("Invalid load/init/play address");
        }

        self.state.set_voice_count(gb_apu::OSC_COUNT);

        let gain = self.state.gain();
        self.apu.volume(gain);

        setup_buffer(self, 4194304)
    }

    fn gbs_start_track_(&mut self, track: i32) -> BlarggErr {
        classic_start_track_(self, track)?;

        self.ram[..0x4000].fill(0);
        self.ram[0x4000..0x4000 + 0x1F00].fill(0xFF);
        self.ram[0x5F00..].fill(0);
        self.ram[HI_PAGE] = 0; // joypad reads back as 0

        let bufs = self.classic.bufs();
        self.apu.reset(bufs);
        for (i, &d) in SOUND_DATA.iter().enumerate() {
            self.apu
                .write_register(bufs, 0, i as u32 + gb_apu::START_ADDR, d as i32);
        }

        let load_addr = get_le16(&self.header_[header::LOAD_ADDR..]);
        self.rom.set_addr(load_addr as i64);
        self.cpu.rst_base = load_addr;

        self.cpu.reset(CodePage {
            mem: MEM_ROM,
            offset: self.rom.unmapped(),
        });

        self.cpu.map_code(
            RAM_ADDR,
            0x10000 - RAM_ADDR,
            CodePage {
                mem: MEM_RAM,
                offset: 0,
            },
        );
        let at = self.rom.at_addr(0);
        self.cpu.map_code(
            0,
            BANK_SIZE as u32,
            CodePage {
                mem: MEM_ROM,
                offset: at,
            },
        );
        let bank = (self.rom.size() > BANK_SIZE as i64) as i32;
        self.set_bank(bank);

        self.ram[HI_PAGE + 6] = self.header_[header::TIMER_MODULO];
        self.ram[HI_PAGE + 7] = self.header_[header::TIMER_MODE];
        self.update_timer();
        self.next_play = self.play_period;

        self.cpu.r.set_a(track as u8);
        self.cpu.r.pc = IDLE_ADDR as i64;
        self.cpu.r.sp = get_le16(&self.header_[header::STACK_PTR..]) as u16;
        self.cpu_time = 0;
        let init = get_le16(&self.header_[header::INIT_ADDR..]);
        self.cpu_jsr(init);

        Ok(())
    }
}

impl GbCpuHost for GbsEmu {
    fn cpu(&mut self) -> &mut GbCpu {
        &mut self.cpu
    }

    #[inline]
    fn code_byte(&self, mem: u8, offset: usize) -> u8 {
        // (the pages, and the bytes the CPU reads past their ends, are all
        // within the ROM data's padding or the RAM's)
        if mem == MEM_RAM {
            self.ram[offset]
        } else {
            self.rom.rom[offset]
        }
    }

    /// `Gbs_Emu::cpu_read()`
    fn cpu_read(&mut self, addr: GbAddr) -> i32 {
        let p = self.cpu.get_code(addr);
        let mut result = self.code_byte(p.mem, p.offset) as i32;
        if addr.wrapping_sub(gb_apu::START_ADDR) < gb_apu::REGISTER_COUNT {
            let time = self.clock();
            result = self.apu.read_register(self.classic.bufs(), time, addr);
        }
        result
    }

    /// `Gbs_Emu::cpu_write()`
    fn cpu_write(&mut self, addr: GbAddr, data: i32) {
        let offset = addr.wrapping_sub(RAM_ADDR);
        if offset <= 0xFFFF - RAM_ADDR {
            self.ram[offset as usize] = data as u8;
            if (addr ^ 0xE000) <= 0x1F80 - 1 {
                if addr.wrapping_sub(gb_apu::START_ADDR) < gb_apu::REGISTER_COUNT {
                    // GME_APU_HOOK( this, addr - Gb_Apu::start_addr, data );
                    let time = self.clock();
                    self.apu
                        .write_register(self.classic.bufs(), time, addr, data);
                } else if (addr ^ 0xFF06) < 2 {
                    self.update_timer();
                } else if addr == JOYPAD_ADDR {
                    self.ram[offset as usize] = 0; // keep joypad return value 0
                } else {
                    self.ram[offset as usize] = 0xFF;
                }

                //if ( addr == 0xFFFF )
                //  debug_printf( "Wrote interrupt mask\n" );
            }
        } else if (addr ^ 0x2000) <= 0x2000 - 1 {
            self.set_bank(data);
        }
    }
}

impl ClassicEmu for GbsEmu {
    fn classic(&self) -> &ClassicEmuState {
        &self.classic
    }
    fn classic_mut(&mut self) -> &mut ClassicEmuState {
        &mut self.classic
    }

    fn update_eq(&mut self, eq: &BlipEq) {
        self.apu.treble_eq(eq);
    }

    fn set_voice(&mut self, i: i32, c: Option<BufId>, l: Option<BufId>, r: Option<BufId>) {
        self.apu.osc_output(i, c, l, r);
    }

    fn run_clocks(&mut self, duration: &mut BlipTime, _msec: i32) -> BlarggErr {
        self.cpu_time = 0;
        while self.cpu_time < *duration {
            let count = *duration - self.cpu_time;
            self.cpu_time = *duration;
            let result = gb_cpu::run(self, count);
            self.cpu_time -= self.cpu.remain();

            if result {
                if self.cpu.r.pc == IDLE_ADDR as i64 {
                    if self.next_play > *duration {
                        self.cpu_time = *duration;
                        break;
                    }

                    if self.cpu_time < self.next_play {
                        self.cpu_time = self.next_play;
                    }
                    self.next_play += self.play_period;
                    let play = get_le16(&self.header_[header::PLAY_ADDR..]);
                    self.cpu_jsr(play);
                    // GME_FRAME_HOOK( this );
                    // TODO: handle timer rates different than 60 Hz
                } else if self.cpu.r.pc > 0xFFFF {
                    // debug_printf( "PC wrapped around\n" );
                    self.cpu.r.pc &= 0xFFFF;
                } else {
                    self.state
                        .set_warning("Emulation error (illegal/unsupported instruction)");
                    // debug_printf( "Bad opcode $%.2x at $%.4x\n", ... );
                    self.cpu.r.pc = (self.cpu.r.pc + 1) & 0xFFFF;
                    self.cpu_time += 6;
                }
            }
        }

        *duration = self.cpu_time;
        self.next_play -= self.cpu_time;
        if self.next_play < 0 {
            // could go negative if routine is taking too long to return
            self.next_play = 0;
        }
        let t = self.cpu_time;
        self.apu.end_frame(self.classic.bufs(), t);

        Ok(())
    }
}

impl MusicEmu for GbsEmu {
    classic_emu_overrides!();

    fn unload(&mut self) {
        self.rom.clear();
        music_emu_unload(self);
    }

    fn track_info_(&self, out: &mut TrackInfo, _track: i32) -> BlarggErr {
        copy_gbs_fields(&self.header_, out);
        Ok(())
    }

    // Setup

    fn load_(&mut self, input: &mut dyn DataReader) -> BlarggErr {
        self.gbs_load_(input)
    }

    fn start_track_(&mut self, track: i32) -> BlarggErr {
        self.gbs_start_track_(track)
    }

    fn set_tempo_(&mut self, t: f64) {
        self.apu.set_tempo(t);
        self.update_timer();
    }
}
