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

//! Nintendo NES/Famicom NSF music file emulator (`Nsf_Emu.h`,
//! `Nsf_Emu.cpp`, `nes_cpu_io.h`), with all of its sound chips (libgme is
//! built without `NSF_EMU_APU_ONLY` or `NSF_EMU_EXTRA_FLAGS`).

use crate::blargg_common::{get_le16, BlarggErr};
use crate::blip_buffer::{BlipEq, BlipTime, BufId};
use crate::classic_emu::{
    classic_emu_overrides, classic_start_track_, setup_buffer, ClassicEmu, ClassicEmuState,
    RomData, MIXED_TYPE, NOISE_TYPE, WAVE_TYPE,
};
use crate::data_reader::{DataReader, EOF_ERROR};
use crate::gme_file::{copy_field_, gme_copy_field, GmeTypeT, TrackInfo, GME_WRONG_FILE_TYPE};
use crate::music_emu::{
    gme_info_overrides, make_equalizer, music_emu_unload, Equalizer, MusicEmu, MusicEmuState,
};
use crate::nes_apu::{self, NesApu};
use crate::nes_cpu::{
    self, CodePage, NesAddr, NesCpu, NesCpuHost, NesTime, Registers, BAD_OPCODE, MEM_LOW,
};
use crate::nes_fds_apu::{self, NesFdsApu};
use crate::nes_fme7_apu::{self, NesFme7Apu};
use crate::nes_mmc5_apu::{self, NesMmc5Apu};
use crate::nes_namco_apu::{self, NesNamcoApu};
use crate::nes_vrc6_apu::{self, NesVrc6Apu};
use crate::nes_vrc7_apu::{self, NesVrc7Apu};

const VRC6_FLAG: u8 = 0x01;
const VRC7_FLAG: u8 = 0x02;
const FDS_FLAG: u8 = 0x04;
const MMC5_FLAG: u8 = 0x08;
const NAMCO_FLAG: u8 = 0x10;
const FME7_FLAG: u8 = 0x20;

const CLOCK_DIVISOR: i64 = 12;

/// Equalizer profiles for US NES and Japanese Famicom
pub const NES_EQ: Equalizer = make_equalizer(-1.0, 80.0);
pub const FAMICOM_EQ: Equalizer = make_equalizer(-15.0, 80.0);

/// NSF file header
pub const HEADER_SIZE: usize = 0x80;

/// The fields of the NSF `header_t`, as offsets into its bytes.
pub mod header {
    pub const TAG: usize = 0; // [5]
    pub const VERS: usize = 5;
    pub const TRACK_COUNT: usize = 6;
    pub const FIRST_TRACK: usize = 7;
    pub const LOAD_ADDR: usize = 8; // [2]
    pub const INIT_ADDR: usize = 10; // [2]
    pub const PLAY_ADDR: usize = 12; // [2]
    pub const GAME: usize = 14; // [32]
    pub const AUTHOR: usize = 46; // [32]
    pub const COPYRIGHT: usize = 78; // [32]
    pub const NTSC_SPEED: usize = 110; // [2]
    pub const BANKS: usize = 112; // [8]
    pub const PAL_SPEED: usize = 120; // [2]
    pub const SPEED_FLAGS: usize = 122;
    pub const CHIP_FLAGS: usize = 123;
}

const ROM_BEGIN: u32 = 0x8000;
const BANK_SELECT_ADDR: u32 = 0x5FF8;
const BANK_SIZE: i32 = 0x1000;
const BANK_COUNT: usize = 8;
const BADOP_ADDR: u32 = BANK_SELECT_ADDR;

const SRAM_ADDR: u32 = 0x6000;
const SRAM_SIZE: usize = 0x2000;

// Memory regions of the code map
const MEM_SRAM: u8 = 1;
const MEM_ROM: u8 = 2;
const MEM_UNMAPPED: u8 = 3;

static VOICE_TYPES: [i32; 16] = [
    WAVE_TYPE | 1,
    WAVE_TYPE | 2,
    WAVE_TYPE,
    NOISE_TYPE,
    MIXED_TYPE | 1,
    WAVE_TYPE | 3,
    WAVE_TYPE | 4,
    WAVE_TYPE | 5,
    WAVE_TYPE | 6,
    WAVE_TYPE | 7,
    WAVE_TYPE | 8,
    WAVE_TYPE | 9,
    WAVE_TYPE | 10,
    WAVE_TYPE | 11,
    WAVE_TYPE | 12,
    WAVE_TYPE | 13,
];

pub struct NsfEmu {
    pub(crate) state: MusicEmuState,
    pub(crate) classic: ClassicEmuState,
    cpu: NesCpu,

    initial_banks: [u8; BANK_COUNT],
    init_addr: NesAddr,
    play_addr: NesAddr,
    clock_rate_: f64,
    pal_only: bool,

    // timing
    saved_state: Registers,
    next_play: NesTime,
    play_period: NesTime,
    play_extra: i32,
    play_ready: i32,

    rom: RomData,

    mmc5_mul: [u8; 2],

    namco: Option<Box<NesNamcoApu>>,
    vrc6: Option<Box<NesVrc6Apu>>,
    fme7: Option<Box<NesFme7Apu>>,
    fds: Option<Box<NesFdsApu>>,
    mmc5: Option<Box<NesMmc5Apu>>,
    vrc7: Option<Box<NesVrc7Apu>>,
    apu: NesApu,
    apu_names: Vec<&'static str>,

    pub(crate) header_: [u8; HEADER_SIZE],

    /// `sram` and, right after it, `unmapped_code`, as upstream lays them
    /// out (code running at the end of the SRAM reads its operands from
    /// the unmapped page after it)
    mem: [u8; SRAM_SIZE + nes_cpu::PAGE_SIZE as usize + 8],
}

impl std::fmt::Debug for NsfEmu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NsfEmu").finish_non_exhaustive()
    }
}

fn copy_nsf_fields(h: &[u8; HEADER_SIZE], out: &mut TrackInfo) {
    gme_copy_field(&mut out.game, &h[header::GAME..header::GAME + 32]);
    gme_copy_field(&mut out.author, &h[header::AUTHOR..header::AUTHOR + 32]);
    gme_copy_field(
        &mut out.copyright,
        &h[header::COPYRIGHT..header::COPYRIGHT + 32],
    );
    if h[header::CHIP_FLAGS] != 0 {
        copy_field_(&mut out.system, b"Famicom");
    }
}

fn check_nsf_header(header: &[u8]) -> BlarggErr {
    if &header[..5] != b"NESM\x1A" {
        return Err(GME_WRONG_FILE_TYPE);
    }
    Ok(())
}

/// `Nsf_File`: the info-only NSF reader
pub(crate) struct NsfFile {
    state: MusicEmuState,
    h: [u8; HEADER_SIZE],
}

impl NsfFile {
    fn new() -> Self {
        let mut state = MusicEmuState::new();
        state.set_type(&GME_NSF_TYPE_);
        NsfFile {
            state,
            h: [0; HEADER_SIZE],
        }
    }
}

impl MusicEmu for NsfFile {
    gme_info_overrides!();

    fn load_(&mut self, input: &mut dyn DataReader) -> BlarggErr {
        if let Err(err) = input.read(&mut self.h) {
            return Err(if err == EOF_ERROR {
                GME_WRONG_FILE_TYPE
            } else {
                err
            });
        }

        if self.h[header::CHIP_FLAGS]
            & !(NAMCO_FLAG | VRC6_FLAG | FME7_FLAG | FDS_FLAG | MMC5_FLAG | VRC7_FLAG)
            != 0
        {
            self.state
                .set_warning("Uses unsupported audio expansion hardware");
        }

        self.state
            .set_track_count(self.h[header::TRACK_COUNT] as i32);
        check_nsf_header(&self.h)
    }

    fn track_info_(&self, out: &mut TrackInfo, _track: i32) -> BlarggErr {
        copy_nsf_fields(&self.h, out);
        Ok(())
    }
}

fn new_nsf_emu() -> Box<dyn MusicEmu> {
    Box::new(NsfEmu::new())
}
fn new_nsf_file() -> Box<dyn MusicEmu> {
    Box::new(NsfFile::new())
}

pub(crate) static GME_NSF_TYPE_: GmeTypeT = GmeTypeT {
    system: "Nintendo NES",
    track_count: 0,
    new_emu: new_nsf_emu,
    new_info: new_nsf_file,
    extension_: "NSF",
    flags_: 1,
};

/// `pcm_read()`: the DMC's memory reader, `*get_code( addr )`
fn pcm_read(cpu: &NesCpu, rom: &RomData, mem: &[u8], addr: NesAddr) -> i32 {
    let p = cpu.get_code(addr);
    code_byte(cpu, rom, mem, p.mem, p.offset) as i32
}

#[inline]
fn code_byte(cpu: &NesCpu, rom: &RomData, mem: &[u8], region: u8, offset: usize) -> u8 {
    match region {
        MEM_LOW => cpu.low_mem[offset],
        MEM_SRAM => mem[offset],
        MEM_ROM => rom.rom[offset],
        _ => mem[SRAM_SIZE + offset],
    }
}

impl Default for NsfEmu {
    fn default() -> Self {
        Self::new()
    }
}

impl NsfEmu {
    pub fn new() -> Self {
        let mut state = MusicEmuState::new();
        state.set_type(&GME_NSF_TYPE_);
        state.set_silence_lookahead(6);
        let mut e = NsfEmu {
            state,
            classic: ClassicEmuState::new(),
            cpu: NesCpu::new(),
            initial_banks: [0; BANK_COUNT],
            init_addr: 0,
            play_addr: 0,
            clock_rate_: 0.0,
            pal_only: false,
            saved_state: Registers::default(),
            next_play: 0,
            play_period: 0,
            play_extra: 0,
            play_ready: 0,
            rom: RomData::new(BANK_SIZE),
            mmc5_mul: [0; 2],
            namco: None,
            vrc6: None,
            fme7: None,
            fds: None,
            mmc5: None,
            vrc7: None,
            apu: NesApu::new(),
            apu_names: Vec::new(),
            header_: [0; HEADER_SIZE],
            mem: [0; SRAM_SIZE + nes_cpu::PAGE_SIZE as usize + 8],
        };
        // apu.dmc_reader( pcm_read, this );
        e.set_equalizer(&NES_EQ);
        e.state.set_gain(1.4);
        e.mem[SRAM_SIZE..].fill(BAD_OPCODE);
        e
    }

    /// Header for currently loaded file
    pub fn header(&self) -> &[u8; HEADER_SIZE] {
        &self.header_
    }

    /// `Nsf_Emu::unload()`
    pub(crate) fn nsf_unload(&mut self) {
        self.vrc6 = None;
        self.namco = None;
        self.fme7 = None;
        self.fds = None;
        self.mmc5 = None;
        self.vrc7 = None;

        self.rom.clear();
    }

    fn init_sound(&mut self) -> BlarggErr {
        if self.header_[header::CHIP_FLAGS]
            & !(NAMCO_FLAG | VRC6_FLAG | FME7_FLAG | FDS_FLAG | MMC5_FLAG | VRC7_FLAG)
            != 0
        {
            self.state
                .set_warning("Uses unsupported audio expansion hardware");
        }

        let count_total = nes_apu::OSC_COUNT
            + nes_namco_apu::OSC_COUNT
            + nes_vrc6_apu::OSC_COUNT
            + nes_fme7_apu::OSC_COUNT
            + nes_fds_apu::OSC_COUNT
            + nes_mmc5_apu::OSC_COUNT
            + nes_vrc7_apu::OSC_COUNT;

        self.apu_names = vec![""; count_total as usize];

        let mut count = 0usize;

        {
            let n = &mut self.apu_names;
            n[count] = "Square 1";
            n[count + 1] = "Square 2";
            n[count + 2] = "Triangle";
            n[count + 3] = "Noise";
            n[count + 4] = "DMC";
            n[count + 5] = "FM";
            count += nes_apu::OSC_COUNT as usize;
        }

        self.classic.set_voice_types(&VOICE_TYPES); // common to all sound chip configurations

        let mut adjusted_gain = self.state.gain();

        let flags = self.header_[header::CHIP_FLAGS];
        {
            if flags & VRC6_FLAG != 0 {
                self.vrc6 = Some(Box::new(NesVrc6Apu::new()));
                adjusted_gain *= 0.75;

                self.apu_names[count..count + 3]
                    .copy_from_slice(&["Saw Wave", "Square 3", "Square 4"]);

                count += nes_vrc6_apu::OSC_COUNT as usize;
            }

            if flags & NAMCO_FLAG != 0 {
                self.namco = Some(Box::new(NesNamcoApu::new()));
                adjusted_gain *= 0.75;

                self.apu_names[count..count + 8].copy_from_slice(&[
                    "Wave 1", "Wave 2", "Wave 3", "Wave 4", "Wave 5", "Wave 6", "Wave 7", "Wave 8",
                ]);

                count += nes_namco_apu::OSC_COUNT as usize;
            }

            if flags & FME7_FLAG != 0 {
                self.fme7 = Some(Box::new(NesFme7Apu::new()));
                adjusted_gain *= 0.75;

                self.apu_names[count..count + 3]
                    .copy_from_slice(&["Square 3", "Square 4", "Square 5"]);

                count += nes_fme7_apu::OSC_COUNT as usize;
            }

            if flags & FDS_FLAG != 0 {
                self.fds = Some(Box::new(NesFdsApu::new()));
                adjusted_gain *= 0.75;

                self.apu_names[count] = "Wave";

                count += nes_fds_apu::OSC_COUNT as usize;
            }

            if flags & MMC5_FLAG != 0 {
                self.mmc5 = Some(Box::new(NesMmc5Apu::new()));
                adjusted_gain *= 0.75;

                self.apu_names[count..count + 3].copy_from_slice(&["Square 3", "Square 4", "PCM"]);

                count += nes_mmc5_apu::OSC_COUNT as usize;
            }

            if flags & VRC7_FLAG != 0 {
                let mut vrc7 = Box::new(NesVrc7Apu::new());
                vrc7.init()?;
                self.vrc7 = Some(vrc7);
                adjusted_gain *= 0.75;

                self.apu_names[count..count + 6]
                    .copy_from_slice(&["FM 1", "FM 2", "FM 3", "FM 4", "FM 5", "FM 6"]);

                count += nes_vrc7_apu::OSC_COUNT as usize;
            }

            if let Some(c) = self.namco.as_mut() {
                c.volume(adjusted_gain);
            }
            if let Some(c) = self.vrc6.as_mut() {
                c.volume(adjusted_gain);
            }
            if let Some(c) = self.fme7.as_mut() {
                c.volume(adjusted_gain);
            }
            if let Some(c) = self.fds.as_mut() {
                c.volume(adjusted_gain);
            }
            if let Some(c) = self.mmc5.as_mut() {
                c.volume(adjusted_gain);
            }
            if let Some(c) = self.vrc7.as_mut() {
                c.volume(adjusted_gain);
            }
        }

        self.state.set_voice_count(count as i32);
        let names = self.apu_names.clone();
        self.state.set_voice_names(&names);

        self.apu.volume(adjusted_gain);

        Ok(())
    }

    /// `Nsf_Emu::load_()`
    pub(crate) fn nsf_load_(&mut self, input: &mut dyn DataReader) -> BlarggErr {
        self.rom
            .load(input, HEADER_SIZE as i32, &mut self.header_, 0)?;

        self.state
            .set_track_count(self.header_[header::TRACK_COUNT] as i32);
        check_nsf_header(&self.header_)?;

        if self.header_[header::VERS] != 1 {
            self.state.set_warning("Unknown file version");
        }

        // sound and memory
        self.init_sound()?;

        // set up data
        let mut load_addr = get_le16(&self.header_[header::LOAD_ADDR..]);
        self.init_addr = get_le16(&self.header_[header::INIT_ADDR..]);
        self.play_addr = get_le16(&self.header_[header::PLAY_ADDR..]);
        if load_addr == 0 {
            load_addr = ROM_BEGIN;
        }
        if self.init_addr == 0 {
            self.init_addr = ROM_BEGIN;
        }
        if self.play_addr == 0 {
            self.play_addr = ROM_BEGIN;
        }
        if load_addr < ROM_BEGIN || self.init_addr < ROM_BEGIN {
            let w = self.warning();
            return Err(w.unwrap_or("Corrupt file (invalid load/init/play address)"));
        }

        self.rom.set_addr((load_addr % BANK_SIZE as u32) as i64);
        let total_banks = (self.rom.size() / BANK_SIZE as i64) as i32;

        // bank switching
        let first_bank = ((load_addr - ROM_BEGIN) / BANK_SIZE as u32) as i32;
        for i in 0..BANK_COUNT {
            let mut bank = (i as i32).wrapping_sub(first_bank) as u32;
            if bank >= total_banks as u32 {
                bank = 0;
            }
            self.initial_banks[i] = bank as u8;

            if self.header_[header::BANKS + i] != 0 {
                // bank-switched
                self.initial_banks
                    .copy_from_slice(&self.header_[header::BANKS..header::BANKS + BANK_COUNT]);
                break;
            }
        }

        self.pal_only = (self.header_[header::SPEED_FLAGS] & 3) == 1;

        // #if !NSF_EMU_EXTRA_FLAGS
        self.header_[header::SPEED_FLAGS] = 0;

        let t = self.state.tempo();
        self.set_tempo(t);

        setup_buffer(self, (self.clock_rate_ + 0.5) as i64 as u32)
    }

    /// `Nsf_Emu::start_track_()`
    pub(crate) fn nsf_start_track_(&mut self, track: i32) -> BlarggErr {
        classic_start_track_(self, track)?;

        self.cpu.low_mem[..0x800].fill(0);
        self.mem[..SRAM_SIZE].fill(0);

        self.cpu.reset(CodePage {
            mem: MEM_UNMAPPED,
            offset: 0,
        }); // also maps low_mem
        self.cpu.map_code(
            SRAM_ADDR,
            SRAM_SIZE as u32,
            CodePage {
                mem: MEM_SRAM,
                offset: 0,
            },
            false,
        );
        for i in 0..BANK_COUNT {
            let b = self.initial_banks[i] as i32;
            self.cpu_write(BANK_SELECT_ADDR + i as u32, b);
        }

        let speed_flags = self.header_[header::SPEED_FLAGS];
        self.apu.reset(
            self.pal_only,
            if speed_flags & 0x20 != 0 { 0x3F } else { 0 },
        );
        {
            let (cpu, rom, mem) = (&self.cpu, &self.rom, &self.mem);
            let reader = |a| pcm_read(cpu, rom, mem, a);
            let bufs = self.classic.bufs();
            self.apu.write_register(bufs, &reader, 0, 0x4015, 0x0F);
            self.apu.write_register(
                bufs,
                &reader,
                0,
                0x4017,
                if speed_flags & 0x10 != 0 { 0x80 } else { 0 },
            );
        }
        if let Some(mmc5) = self.mmc5.as_mut() {
            self.mmc5_mul = [0; 2];
            mmc5.exram.fill(0);
        }

        {
            if let Some(c) = self.namco.as_mut() {
                c.reset();
            }
            if let Some(c) = self.vrc6.as_mut() {
                c.reset();
            }
            if let Some(c) = self.fme7.as_mut() {
                c.reset();
            }
            if let Some(c) = self.fds.as_mut() {
                c.reset();
            }
            if let Some(c) = self.mmc5.as_mut() {
                c.reset();
            }
            if let Some(c) = self.vrc7.as_mut() {
                c.reset();
            }
        }

        self.play_ready = 4;
        self.play_extra = 0;
        self.next_play = (self.play_period as i64 / CLOCK_DIVISOR) as NesTime;

        self.saved_state.pc = BADOP_ADDR as u16;
        self.cpu.low_mem[0x1FF] = ((BADOP_ADDR - 1) >> 8) as u8;
        self.cpu.low_mem[0x1FE] = ((BADOP_ADDR - 1) & 0xFF) as u8;
        self.cpu.r.sp = 0xFD;
        self.cpu.r.pc = self.init_addr as u16;
        self.cpu.r.a = track as u8;
        self.cpu.r.x = self.pal_only as u8;

        Ok(())
    }

    /// `Nsf_Emu::cpu_write_misc()`
    fn cpu_write_misc(&mut self, addr: NesAddr, data: i32) {
        let time = self.cpu.time();
        let bufs = self.classic.bufs();
        {
            if let Some(fds) = self.fds.as_mut() {
                if addr.wrapping_sub(nes_fds_apu::IO_ADDR) < nes_fds_apu::IO_SIZE {
                    fds.write(bufs, time, addr, data);
                    return;
                }
            }

            if let Some(namco) = self.namco.as_mut() {
                match addr {
                    nes_namco_apu::DATA_REG_ADDR => {
                        namco.write_data(bufs, time, data);
                        return;
                    }

                    nes_namco_apu::ADDR_REG_ADDR => {
                        namco.write_addr(data);
                        return;
                    }
                    _ => {}
                }
            }

            if addr >= nes_fme7_apu::LATCH_ADDR {
                if let Some(fme7) = self.fme7.as_mut() {
                    match addr & nes_fme7_apu::ADDR_MASK {
                        nes_fme7_apu::LATCH_ADDR => {
                            fme7.write_latch(data);
                            return;
                        }

                        nes_fme7_apu::DATA_ADDR => {
                            fme7.write_data(bufs, time, data);
                            return;
                        }
                        _ => {}
                    }
                }
            }

            if let Some(vrc6) = self.vrc6.as_mut() {
                let reg = addr & (nes_vrc6_apu::ADDR_STEP - 1);
                let osc = addr.wrapping_sub(nes_vrc6_apu::BASE_ADDR) / nes_vrc6_apu::ADDR_STEP;
                if osc < nes_vrc6_apu::OSC_COUNT as u32 && reg < nes_vrc6_apu::REG_COUNT as u32 {
                    vrc6.write_osc(bufs, time, osc as i32, reg as i32, data);
                    return;
                }
            }

            if let Some(mmc5) = self.mmc5.as_mut() {
                if addr.wrapping_sub(nes_mmc5_apu::REGS_ADDR) < nes_mmc5_apu::REGS_SIZE {
                    mmc5.write_register(bufs, time, addr, data);
                    return;
                }

                let m = addr.wrapping_sub(0x5205);
                if m < 2 {
                    self.mmc5_mul[m as usize] = data as u8;
                    return;
                }

                let i = addr.wrapping_sub(0x5C00);
                if (i as usize) < nes_mmc5_apu::EXRAM_SIZE {
                    mmc5.exram[i as usize] = data as u8;
                    return;
                }
            }

            if let Some(vrc7) = self.vrc7.as_mut() {
                if addr == 0x9010 {
                    vrc7.write_reg(data);
                    return;
                }

                if addr.wrapping_sub(0x9028) <= 0x08 {
                    vrc7.write_data(bufs, time, data);
                    return;
                }
            }
        }

        // unmapped write
        // (debug_printf in debug builds)
    }
}

impl NesCpuHost for NsfEmu {
    fn cpu(&mut self) -> &mut NesCpu {
        &mut self.cpu
    }

    #[inline]
    fn code_byte(&self, mem: u8, offset: usize) -> u8 {
        code_byte(&self.cpu, &self.rom, &self.mem, mem, offset)
    }

    /// `Nsf_Emu::cpu_read()`
    fn cpu_read(&mut self, addr: NesAddr) -> i32 {
        let mut result = self.cpu.low_mem[(addr & 0x7FF) as usize] as i32;
        if addr & 0xE000 == 0 {
            return result;
        }

        let p = self.cpu.get_code(addr);
        result = self.code_byte(p.mem, p.offset) as i32;
        if addr > 0x7FFF {
            return result;
        }

        result = self.mem[(addr as usize) & (SRAM_SIZE - 1)] as i32;
        if addr > 0x5FFF {
            return result;
        }

        if addr == nes_apu::STATUS_ADDR {
            let time = self.cpu.time();
            let (cpu, rom, mem) = (&self.cpu, &self.rom, &self.mem);
            let reader = |a| pcm_read(cpu, rom, mem, a);
            return self.apu.read_status(self.classic.bufs(), &reader, time);
        }

        if addr == nes_namco_apu::DATA_REG_ADDR {
            if let Some(namco) = self.namco.as_mut() {
                return namco.read_data();
            }
        }

        if addr.wrapping_sub(nes_fds_apu::IO_ADDR) < nes_fds_apu::IO_SIZE {
            if let Some(fds) = self.fds.as_mut() {
                let time = self.cpu.time();
                return fds.read(self.classic.bufs(), time, addr);
            }
        }

        let i = addr.wrapping_sub(0x5C00);
        if (i as usize) < nes_mmc5_apu::EXRAM_SIZE {
            if let Some(mmc5) = self.mmc5.as_ref() {
                return mmc5.exram[i as usize] as i32;
            }
        }

        let i = addr.wrapping_sub(0x5205);
        if i < 2 && self.mmc5.is_some() {
            return ((self.mmc5_mul[0] as i32 * self.mmc5_mul[1] as i32) >> (i * 8)) & 0xFF;
        }

        result = (addr >> 8) as i32; // simulate open bus

        // if ( addr != 0x2002 ) debug_printf( "Read unmapped $%.4X\n", (unsigned) addr );

        result
    }

    /// `Nsf_Emu::cpu_write()`
    fn cpu_write(&mut self, addr: NesAddr, data: i32) {
        {
            let offset = addr ^ SRAM_ADDR;
            if (offset as usize) < SRAM_SIZE {
                self.mem[offset as usize] = data as u8;
                return;
            }
        }
        {
            let temp = addr & 0x7FF;
            if addr & 0xE000 == 0 {
                self.cpu.low_mem[temp as usize] = data as u8;
                return;
            }
        }

        if addr.wrapping_sub(nes_apu::START_ADDR) <= nes_apu::END_ADDR - nes_apu::START_ADDR {
            // GME_APU_HOOK( this, addr - Nes_Apu::start_addr, data );
            let time = self.cpu.time();
            let (cpu, rom, mem) = (&self.cpu, &self.rom, &self.mem);
            let reader = |a| pcm_read(cpu, rom, mem, a);
            self.apu
                .write_register(self.classic.bufs(), &reader, time, addr, data);
            return;
        }

        let bank = addr.wrapping_sub(BANK_SELECT_ADDR);
        if (bank as usize) < BANK_COUNT {
            let offset = self.rom.mask_addr(data.wrapping_mul(BANK_SIZE));
            if offset as i64 >= self.rom.size() {
                self.state.set_warning("Invalid bank");
            }
            let at = self.rom.at_addr(offset);
            self.cpu.map_code(
                (bank + 8) * BANK_SIZE as u32,
                BANK_SIZE as u32,
                CodePage {
                    mem: MEM_ROM,
                    offset: at,
                },
                false,
            );
            return;
        }

        self.cpu_write_misc(addr, data);
    }
}

impl ClassicEmu for NsfEmu {
    fn classic(&self) -> &ClassicEmuState {
        &self.classic
    }
    fn classic_mut(&mut self) -> &mut ClassicEmuState {
        &mut self.classic
    }

    fn update_eq(&mut self, eq: &BlipEq) {
        self.apu.treble_eq(eq);

        {
            if let Some(c) = self.namco.as_mut() {
                c.treble_eq(eq);
            }
            if let Some(c) = self.vrc6.as_mut() {
                c.treble_eq(eq);
            }
            if let Some(c) = self.fme7.as_mut() {
                c.treble_eq(eq);
            }
            if let Some(c) = self.fds.as_mut() {
                c.treble_eq(eq);
            }
            if let Some(c) = self.mmc5.as_mut() {
                c.treble_eq(eq);
            }
            if let Some(c) = self.vrc7.as_mut() {
                c.treble_eq(eq);
            }
        }
    }

    #[allow(unused_assignments)]
    fn set_voice(&mut self, mut i: i32, buf: Option<BufId>, _l: Option<BufId>, _r: Option<BufId>) {
        if i < nes_apu::OSC_COUNT {
            self.apu.osc_output(i, buf);
            return;
        }
        i -= nes_apu::OSC_COUNT;

        if let Some(vrc6) = self.vrc6.as_mut() {
            if i < nes_vrc6_apu::OSC_COUNT {
                // put saw first
                i -= 1;
                if i < 0 {
                    i = 2;
                }
                vrc6.osc_output(i, buf);
                return;
            }
            i -= nes_vrc6_apu::OSC_COUNT;
        }
        macro_rules! handle_chip {
            ($obj:ident, $m:ident) => {
                if let Some(o) = self.$obj.as_mut() {
                    if i < $m::OSC_COUNT {
                        o.osc_output(i, buf);
                        return;
                    }
                    i -= $m::OSC_COUNT;
                }
            };
        }
        handle_chip!(namco, nes_namco_apu);
        handle_chip!(fme7, nes_fme7_apu);
        handle_chip!(fds, nes_fds_apu);
        handle_chip!(mmc5, nes_mmc5_apu);
        handle_chip!(vrc7, nes_vrc7_apu);
    }

    // Emulation

    // see nes_cpu_io.h for read/write functions

    fn run_clocks(&mut self, duration: &mut BlipTime, _msec: i32) -> BlarggErr {
        self.cpu.set_time(0);
        while self.cpu.time() < *duration {
            let mut end: NesTime = self.next_play.min(*duration);
            end = end.min(self.cpu.time() + 32767); // allows CPU to use 16-bit time delta
            if nes_cpu::run(self, end) {
                if self.cpu.r.pc as u32 != BADOP_ADDR {
                    self.state
                        .set_warning("Emulation error (illegal instruction)");
                    self.cpu.r.pc = self.cpu.r.pc.wrapping_add(1);
                } else {
                    self.play_ready = 1;
                    if self.saved_state.pc as u32 != BADOP_ADDR {
                        self.cpu.r = self.saved_state;
                        self.saved_state.pc = BADOP_ADDR as u16;
                    } else {
                        self.cpu.set_time(end);
                    }
                }
            }

            if self.cpu.time() >= self.next_play {
                let period =
                    ((self.play_period as i64 + self.play_extra as i64) / CLOCK_DIVISOR) as NesTime;
                self.play_extra = (self.play_period as i64 - period as i64 * CLOCK_DIVISOR) as i32;
                self.next_play += period;
                if self.play_ready != 0 {
                    self.play_ready -= 1;
                    if self.play_ready == 0 {
                        // check( saved_state.pc == badop_addr );
                        if self.cpu.r.pc as u32 != BADOP_ADDR {
                            self.saved_state = self.cpu.r;
                        }

                        self.cpu.r.pc = self.play_addr as u16;
                        let sp = self.cpu.r.sp;
                        self.cpu.low_mem[0x100 + sp as usize] = ((BADOP_ADDR - 1) >> 8) as u8;
                        let sp = sp.wrapping_sub(1);
                        self.cpu.low_mem[0x100 + sp as usize] = ((BADOP_ADDR - 1) & 0xFF) as u8;
                        self.cpu.r.sp = sp.wrapping_sub(1);
                        // GME_FRAME_HOOK( this );
                    }
                }
            }
        }

        if self.cpu.error_count() != 0 {
            self.cpu.clear_error_count();
            self.state
                .set_warning("Emulation error (illegal instruction)");
        }

        *duration = self.cpu.time();
        self.next_play -= *duration;
        // check( next_play >= 0 );
        if self.next_play < 0 {
            self.next_play = 0;
        }

        let d = *duration;
        let bufs = self.classic.bufs();
        {
            let (cpu, rom, mem) = (&self.cpu, &self.rom, &self.mem);
            let reader = |a| pcm_read(cpu, rom, mem, a);
            self.apu.end_frame(bufs, &reader, d);
        }

        {
            if let Some(c) = self.namco.as_mut() {
                c.end_frame(bufs, d);
            }
            if let Some(c) = self.vrc6.as_mut() {
                c.end_frame(bufs, d);
            }
            if let Some(c) = self.fme7.as_mut() {
                c.end_frame(bufs, d);
            }
            if let Some(c) = self.fds.as_mut() {
                c.end_frame(bufs, d);
            }
            if let Some(c) = self.mmc5.as_mut() {
                c.end_frame(bufs, d);
            }
            if let Some(c) = self.vrc7.as_mut() {
                c.end_frame(bufs, d);
            }
        }

        Ok(())
    }
}

impl MusicEmu for NsfEmu {
    classic_emu_overrides!();

    fn unload(&mut self) {
        self.nsf_unload();
        music_emu_unload(self);
    }

    // Track info

    fn track_info_(&self, out: &mut TrackInfo, _track: i32) -> BlarggErr {
        copy_nsf_fields(&self.header_, out);
        Ok(())
    }

    fn load_(&mut self, input: &mut dyn DataReader) -> BlarggErr {
        self.nsf_load_(input)
    }

    fn start_track_(&mut self, track: i32) -> BlarggErr {
        self.nsf_start_track_(track)
    }

    // Setup

    fn set_tempo_(&mut self, t: f64) {
        let mut playback_rate = get_le16(&self.header_[header::NTSC_SPEED..]);
        let mut standard_rate = 0x411Au32;
        self.clock_rate_ = 1789772.72727;
        self.play_period = (262 * 341i64 * 4 - 2) as NesTime; // two fewer PPU clocks every four frames

        if self.pal_only {
            self.play_period = (33247 * CLOCK_DIVISOR) as NesTime;
            self.clock_rate_ = 1662607.125;
            standard_rate = 0x4E20;
            playback_rate = get_le16(&self.header_[header::PAL_SPEED..]);
        }

        if playback_rate == 0 {
            playback_rate = standard_rate;
        }

        if playback_rate != standard_rate || t != 1.0 {
            self.play_period = (playback_rate as f64 * self.clock_rate_
                / (1000000.0 / CLOCK_DIVISOR as f64 * t)) as i64
                as NesTime;
        }

        self.apu.set_tempo(t);
    }
}
