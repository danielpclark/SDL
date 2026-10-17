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

//! MSX computer KSS music file emulator (`Kss_Emu.h`, `Kss_Emu.cpp`).

use crate::ay_apu::{self, AyApu};
use crate::ay_cpu::{self, Instr, Z80Host, RA};
use crate::blargg_common::{get_le16, BlarggErr};
use crate::blip_buffer::{BlipEq, BlipTime, BufId};
use crate::classic_emu::{
    classic_emu_overrides, classic_start_track_, setup_buffer, ClassicEmu, ClassicEmuState,
    RomData, WAVE_TYPE,
};
use crate::data_reader::{DataReader, EOF_ERROR};
use crate::gme_file::{copy_field_, GmeTypeT, TrackInfo, GME_WRONG_FILE_TYPE};
use crate::kss_cpu::{self, CpuTime, KssCpu, Page, IDLE_ADDR, Z80};
use crate::kss_scc_apu::{self, SccApu};
use crate::music_emu::{gme_info_overrides, music_emu_unload, MusicEmu, MusicEmuState};
use crate::sms_apu::{self, SmsApu};

const CLOCK_RATE: i64 = 3579545;
const OSC_COUNT: i32 = ay_apu::OSC_COUNT + kss_scc_apu::OSC_COUNT;

/// KSS file header
pub const HEADER_SIZE: usize = 0x10;
pub const EXT_HEADER_SIZE: usize = 0x10;

/// The fields of the KSS `composite_header_t` (`header_t`, then
/// `ext_header_t`), as offsets into its bytes.
pub mod header {
    pub const TAG: usize = 0; // [4]
    pub const LOAD_ADDR: usize = 4; // [2]
    pub const LOAD_SIZE: usize = 6; // [2]
    pub const INIT_ADDR: usize = 8; // [2]
    pub const PLAY_ADDR: usize = 10; // [2]
    pub const FIRST_BANK: usize = 12;
    pub const BANK_MODE: usize = 13;
    pub const EXTRA_HEADER: usize = 14;
    pub const DEVICE_FLAGS: usize = 15;
    // ext_header_t
    pub const DATA_SIZE: usize = 16; // [4]
    pub const UNUSED: usize = 20; // [4]
    pub const FIRST_TRACK: usize = 24; // [2]
    pub const LAST_TACK: usize = 26; // [2]
    pub const PSG_VOL: usize = 28;
    pub const SCC_VOL: usize = 29;
    pub const MSX_MUSIC_VOL: usize = 30;
    pub const MSX_AUDIO_VOL: usize = 31;
}

const MEM_SIZE: usize = 0x10000;

// Memory regions of the page maps
const MEM_RAM: u8 = 0;
const MEM_ROM: u8 = 1;
const MEM_UNMAPPED_READ: u8 = 2;
const MEM_UNMAPPED_WRITE: u8 = 3;

static VOICE_NAMES: [&str; OSC_COUNT as usize] = [
    "Square 1", "Square 2", "Square 3", "Wave 1", "Wave 2", "Wave 3", "Wave 4", "Wave 5",
];

static VOICE_TYPES: [i32; OSC_COUNT as usize] = [
    WAVE_TYPE,
    WAVE_TYPE | 1,
    WAVE_TYPE | 2,
    WAVE_TYPE | 3,
    WAVE_TYPE | 4,
    WAVE_TYPE | 5,
    WAVE_TYPE | 6,
    WAVE_TYPE | 7,
];

pub struct KssEmu {
    pub(crate) state: MusicEmuState,
    pub(crate) classic: ClassicEmuState,
    cpu: KssCpu,

    rom: RomData,
    header_: [u8; HEADER_SIZE + EXT_HEADER_SIZE],

    scc_accessed: bool,
    gain_updated: bool,
    scc_enabled: u32, // 0 or 0xC000
    bank_count: i32,

    play_period: BlipTime,
    next_play: BlipTime,
    ay_latch: i32,

    // large items
    ram: Box<[u8]>,
    ay: AyApu,
    scc: SccApu,
    sn: Option<Box<SmsApu>>,
    unmapped_read: [u8; 0x100],
    unmapped_write: Box<[u8]>,
}

impl std::fmt::Debug for KssEmu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KssEmu").finish_non_exhaustive()
    }
}

impl Default for KssEmu {
    fn default() -> Self {
        Self::new()
    }
}

// Track info

fn copy_kss_fields(h: &[u8], out: &mut TrackInfo) {
    let mut system: &[u8] = b"MSX";
    if h[header::DEVICE_FLAGS] & 0x02 != 0 {
        system = b"Sega Master System";
        if h[header::DEVICE_FLAGS] & 0x04 != 0 {
            system = b"Game Gear";
        }

        if h[header::DEVICE_FLAGS] & 0x01 != 0 {
            system = b"Sega Mega Drive";
        }
    }
    copy_field_(&mut out.system, system);
}

fn check_kss_header(header: &[u8]) -> BlarggErr {
    if &header[..4] != b"KSCC" && &header[..4] != b"KSSX" {
        return Err(GME_WRONG_FILE_TYPE);
    }
    Ok(())
}

/// `Kss_File`: the info-only KSS reader
pub(crate) struct KssFile {
    state: MusicEmuState,
    header_: [u8; HEADER_SIZE],
}

impl KssFile {
    fn new() -> Self {
        let mut state = MusicEmuState::new();
        state.set_type(&GME_KSS_TYPE_);
        KssFile {
            state,
            header_: [0; HEADER_SIZE],
        }
    }
}

impl MusicEmu for KssFile {
    gme_info_overrides!();

    fn load_(&mut self, input: &mut dyn DataReader) -> BlarggErr {
        if let Err(err) = input.read(&mut self.header_) {
            return Err(if err == EOF_ERROR {
                GME_WRONG_FILE_TYPE
            } else {
                err
            });
        }
        check_kss_header(&self.header_)
    }

    fn track_info_(&self, out: &mut TrackInfo, _track: i32) -> BlarggErr {
        copy_kss_fields(&self.header_, out);
        Ok(())
    }
}

fn new_kss_emu() -> Box<dyn MusicEmu> {
    Box::new(KssEmu::new())
}
fn new_kss_file() -> Box<dyn MusicEmu> {
    Box::new(KssFile::new())
}

pub(crate) static GME_KSS_TYPE_: GmeTypeT = GmeTypeT {
    system: "MSX",
    track_count: 256,
    new_emu: new_kss_emu,
    new_info: new_kss_file,
    extension_: "KSS",
    flags_: 0x03,
};

// copy driver code to lo RAM
static BIOS: [u8; 13] = [
    0xD3, 0xA0, 0xF5, 0x7B, 0xD3, 0xA1, 0xF1, 0xC9, // $0001: WRTPSG
    0xD3, 0xA0, 0xDB, 0xA2, 0xC9, // $0009: RDPSG
];
static VECTORS: [u8; 6] = [
    0xC3, 0x01, 0x00, // $0093: WRTPSG vector
    0xC3, 0x09, 0x00, // $0096: RDPSG vector
];

impl KssEmu {
    pub fn new() -> Self {
        let mut state = MusicEmuState::new();
        state.set_type(&GME_KSS_TYPE_);
        state.set_silence_lookahead(6);
        state.set_voice_names(&VOICE_NAMES);
        let mut classic = ClassicEmuState::new();
        classic.set_voice_types(&VOICE_TYPES);
        KssEmu {
            state,
            classic,
            cpu: KssCpu::new(),
            rom: RomData::new(kss_cpu::PAGE_SIZE as i32),
            header_: [0; HEADER_SIZE + EXT_HEADER_SIZE],
            scc_accessed: false,
            gain_updated: false,
            scc_enabled: 0,
            bank_count: 0,
            play_period: 0,
            next_play: 0,
            ay_latch: 0,
            ram: vec![0u8; MEM_SIZE + kss_cpu::CPU_PADDING].into_boxed_slice(),
            ay: AyApu::new(),
            scc: SccApu::new(),
            sn: None,
            unmapped_read: [0xFF; 0x100],
            unmapped_write: vec![0u8; kss_cpu::PAGE_SIZE as usize + 1].into_boxed_slice(),
        }
    }

    /// Header for currently loaded file
    pub fn header(&self) -> &[u8; HEADER_SIZE + EXT_HEADER_SIZE] {
        &self.header_
    }

    fn bank_size(&self) -> i32 {
        (16 * 1024) >> (self.header_[header::BANK_MODE] >> 7 & 1)
    }

    // Setup

    fn update_gain(&mut self) {
        let mut g = self.state.gain() * 1.4;
        if self.scc_accessed {
            g *= 1.5;
        }
        self.ay.volume(g);
        self.scc.volume(g);
        if let Some(sn) = self.sn.as_mut() {
            sn.volume(g);
        }
    }

    fn kss_load_(&mut self, input: &mut dyn DataReader) -> BlarggErr {
        self.header_.fill(0);
        let mut h = [0u8; HEADER_SIZE];
        let r = self.rom.load(input, HEADER_SIZE as i32, &mut h, 0);
        self.header_[..HEADER_SIZE].copy_from_slice(&h);
        r?;
        check_kss_header(&self.header_)?;

        if self.header_[3] == b'C' {
            if self.header_[header::EXTRA_HEADER] != 0 {
                self.header_[header::EXTRA_HEADER] = 0;
                self.state.set_warning("Unknown data in header");
            }
            if self.header_[header::DEVICE_FLAGS] & !0x0F != 0 {
                self.header_[header::DEVICE_FLAGS] &= 0x0F;
                self.state.set_warning("Unknown data in header");
            }
        } else {
            let n = EXT_HEADER_SIZE.min(self.header_[header::EXTRA_HEADER] as usize);
            let b = self.rom.begin();
            self.header_[HEADER_SIZE..HEADER_SIZE + n].copy_from_slice(&self.rom.rom[b..b + n]);
            if self.header_[header::EXTRA_HEADER] > 0x10 {
                self.state.set_warning("Unknown data in header");
            }
        }

        if self.header_[header::DEVICE_FLAGS] & 0x09 != 0 {
            self.state.set_warning("FM sound not supported");
        }

        self.scc_enabled = 0xC000;
        if self.header_[header::DEVICE_FLAGS] & 0x04 != 0 {
            self.scc_enabled = 0;
        }

        if self.header_[header::DEVICE_FLAGS] & 0x02 != 0 && self.sn.is_none() {
            self.sn = Some(Box::new(SmsApu::new()));
        }

        self.state.set_voice_count(OSC_COUNT);

        setup_buffer(self, CLOCK_RATE as u32)
    }

    fn kss_start_track_(&mut self, track: i32) -> BlarggErr {
        classic_start_track_(self, track)?;

        self.ram[..0x4000].fill(0xC9);
        self.ram[0x4000..].fill(0);

        self.ram[0x01..0x01 + BIOS.len()].copy_from_slice(&BIOS);
        self.ram[0x93..0x93 + VECTORS.len()].copy_from_slice(&VECTORS);

        // copy non-banked data into RAM
        let load_addr = get_le16(&self.header_[header::LOAD_ADDR..]) as i64;
        let orig_load_size = get_le16(&self.header_[header::LOAD_SIZE..]) as i64;
        let mut load_size = orig_load_size.min(self.rom.file_size());
        load_size = load_size.min(MEM_SIZE as i64 - load_addr);
        if load_size != orig_load_size {
            self.state.set_warning("Excessive data size");
        }
        let extra = self.header_[header::EXTRA_HEADER] as usize;
        let src = self.rom.begin() + extra;
        let (a, n) = (load_addr as usize, load_size as usize);
        self.ram[a..a + n].copy_from_slice(&self.rom.rom[src..src + n]);

        self.rom.set_addr(-load_size - extra as i64);

        // check available bank data
        let bank_size = self.bank_size();
        let max_banks =
            ((self.rom.file_size() - load_size + bank_size as i64 - 1) / bank_size as i64) as i32;
        self.bank_count = (self.header_[header::BANK_MODE] & 0x7F) as i32;
        if self.bank_count > max_banks {
            self.bank_count = max_banks;
            self.state.set_warning("Bank data missing");
        }
        //debug_printf( "load_size : $%X\n", load_size );
        //debug_printf( "bank_size : $%X\n", bank_size );
        //debug_printf( "bank_count: %d (%d claimed)\n", bank_count, header_.bank_mode & 0x7F );

        self.ram[IDLE_ADDR as usize] = 0xFF;
        self.cpu.reset(
            Page {
                mem: MEM_UNMAPPED_WRITE,
                offset: 0,
            },
            Page {
                mem: MEM_UNMAPPED_READ,
                offset: 0,
            },
        );
        let ram = Page {
            mem: MEM_RAM,
            offset: 0,
        };
        self.cpu.map_mem(0, MEM_SIZE as u32, ram, ram);

        self.ay.reset();
        self.scc.reset();
        if let Some(sn) = self.sn.as_mut() {
            sn.reset(0, 0);
        }
        let r = &mut self.cpu.z80.r;
        r.sp = 0xF380;
        r.sp -= 1;
        self.ram[r.sp as usize] = (IDLE_ADDR >> 8) as u8;
        r.sp -= 1;
        self.ram[r.sp as usize] = (IDLE_ADDR & 0xFF) as u8;
        r.b.0[RA] = track as u8;
        r.pc = get_le16(&self.header_[header::INIT_ADDR..]) as u16;
        self.next_play = self.play_period;
        self.scc_accessed = false;
        self.gain_updated = false;
        self.update_gain();
        self.ay_latch = 0;

        Ok(())
    }

    fn set_bank(&mut self, logical: i32, mut physical: i32) {
        let bank_size = self.bank_size() as u32;

        let mut addr: u32 = 0x8000;
        if logical != 0 && bank_size == 8 * 1024 {
            addr = 0xA000;
        }

        physical -= self.header_[header::FIRST_BANK] as i32;
        if physical as u32 >= self.bank_count as u32 {
            let data = Page {
                mem: MEM_RAM,
                offset: addr as usize,
            };
            self.cpu.map_mem(addr, bank_size, data, data);
        } else {
            let phys: i64 = physical as i64 * bank_size as i64;
            let mut offset = 0;
            while offset < bank_size {
                let at = self.rom.at_addr((phys + offset as i64) as i32);
                self.cpu.map_mem(
                    addr + offset,
                    kss_cpu::PAGE_SIZE,
                    Page {
                        mem: MEM_UNMAPPED_WRITE,
                        offset: 0,
                    },
                    Page {
                        mem: MEM_ROM,
                        offset: at,
                    },
                );
                offset += kss_cpu::PAGE_SIZE;
            }
        }
    }

    fn cpu_write(&mut self, addr: u32, mut data: i32) {
        data &= 0xFF;
        match addr {
            0x9000 => {
                self.set_bank(0, data);
                return;
            }

            0xB000 => {
                self.set_bank(1, data);
                return;
            }
            _ => {}
        }

        let scc_addr = ((addr & 0xDFFF) ^ 0x9800) as i32;
        if scc_addr < kss_scc_apu::REG_COUNT {
            self.scc_accessed = true;
            let t = self.cpu.z80.time();
            self.scc.write(self.classic.bufs(), t, scc_addr, data);
            return;
        }

        // debug_printf( "LD ($%04X),$%02X\n", addr, data );
    }

    #[inline]
    fn byte(&self, p: Page) -> u8 {
        match p.mem {
            MEM_RAM => self.ram[p.offset],
            MEM_ROM => self.rom.rom[p.offset],
            MEM_UNMAPPED_READ => self.unmapped_read[p.offset],
            _ => self.unmapped_write[p.offset],
        }
    }

    #[inline]
    fn set_byte(&mut self, p: Page, data: u8) {
        match p.mem {
            MEM_RAM => self.ram[p.offset] = data,
            // (the write maps are only ever the RAM and the unmapped page)
            _ => self.unmapped_write[p.offset] = data,
        }
    }
}

impl Z80Host for KssEmu {
    const PC_16: bool = false;
    const INSTR_OPERANDS: bool = true;

    fn cpu(&mut self) -> &mut Z80 {
        &mut self.cpu.z80
    }

    #[inline]
    fn read(&mut self, addr: u32) -> u8 {
        let p = self.cpu.read(addr);
        self.byte(p)
    }

    #[inline]
    fn read_word(&mut self, addr: u32) -> u16 {
        let p = self.cpu.read(addr);
        let hi = Page {
            mem: p.mem,
            offset: p.offset + 1,
        };
        self.byte(p) as u16 | (self.byte(hi) as u16) << 8
    }

    /// `kss_cpu_write()`
    fn write(&mut self, addr: u32, data: u8) {
        let p = self.cpu.write(addr);
        self.set_byte(p, data);
        if (addr & self.scc_enabled) == 0x8000 {
            self.cpu_write(addr, data as i32);
        }
    }

    #[inline]
    fn write_word(&mut self, addr: u32, data: u16) {
        let p = self.cpu.write(addr);
        self.set_byte(p, data as u8);
        self.set_byte(
            Page {
                mem: p.mem,
                offset: p.offset + 1,
            },
            (data >> 8) as u8,
        );
    }

    #[inline]
    fn instr(&mut self, pc: u32) -> Instr {
        let p = self.cpu.read(pc);
        Instr {
            mem: p.mem,
            offset: p.offset,
        }
    }

    #[inline]
    fn instr_byte(&mut self, instr: Instr, k: usize) -> u8 {
        self.byte(Page {
            mem: instr.mem,
            offset: instr.offset + k,
        })
    }

    /// `kss_cpu_out()`
    fn cpu_out(&mut self, time: CpuTime, addr: u32, mut data: i32) {
        data &= 0xFF;
        match addr & 0xFF {
            0xA0 => {
                self.ay_latch = data & 0x0F;
                return;
            }

            0xA1 => {
                // GME_APU_HOOK( &emu, emu.ay_latch, data );
                let latch = self.ay_latch;
                self.ay.write(self.classic.bufs(), time, latch, data);
                return;
            }

            0x06 => {
                if self.header_[header::DEVICE_FLAGS] & 0x04 != 0 {
                    if let Some(sn) = self.sn.as_mut() {
                        sn.write_ggstereo(self.classic.bufs(), time, data);
                        return;
                    }
                }
            }

            0x7E | 0x7F => {
                if let Some(sn) = self.sn.as_mut() {
                    // GME_APU_HOOK( &emu, 16, data );
                    sn.write_data(self.classic.bufs(), time, data);
                    return;
                }
            }

            0xFE => {
                self.set_bank(0, data);
                return;
            }
            // (the 0xF1, 0xF0 and 0xA8 cases are only in debug builds)
            _ => {}
        }

        // debug_printf( "OUT $%04X,$%02X\n", addr, data );
    }

    /// `kss_cpu_in()`
    fn cpu_in(&mut self, _time: CpuTime, _addr: u32) -> i32 {
        // debug_printf( "IN $%04X\n", addr );
        0
    }
}

impl ClassicEmu for KssEmu {
    fn classic(&self) -> &ClassicEmuState {
        &self.classic
    }
    fn classic_mut(&mut self) -> &mut ClassicEmuState {
        &mut self.classic
    }

    fn update_eq(&mut self, eq: &BlipEq) {
        self.ay.treble_eq(eq);
        self.scc.treble_eq(eq);
        if let Some(sn) = self.sn.as_mut() {
            sn.treble_eq(eq);
        }
    }

    fn set_voice(
        &mut self,
        i: i32,
        center: Option<BufId>,
        left: Option<BufId>,
        right: Option<BufId>,
    ) {
        let i2 = i - ay_apu::OSC_COUNT;
        if i2 >= 0 {
            self.scc.osc_output(i2, center);
        } else {
            self.ay.osc_output(i, center);
        }
        if let Some(sn) = self.sn.as_mut() {
            if i < sms_apu::OSC_COUNT {
                sn.osc_output(i, center, left, right);
            }
        }
    }

    // Emulation

    fn run_clocks(&mut self, duration: &mut BlipTime, _msec: i32) -> BlarggErr {
        while self.cpu.z80.time() < *duration {
            let end: BlipTime = (*duration).min(self.next_play);
            ay_cpu::run(self, (*duration).min(self.next_play));
            if self.cpu.z80.r.pc as u32 == IDLE_ADDR {
                self.cpu.z80.set_time(end);
            }

            if self.cpu.z80.time() >= self.next_play {
                self.next_play += self.play_period;
                if self.cpu.z80.r.pc as u32 == IDLE_ADDR {
                    if !self.gain_updated {
                        self.gain_updated = true;
                        if self.scc_accessed {
                            self.update_gain();
                        }
                    }

                    let r = &mut self.cpu.z80.r;
                    r.sp = r.sp.wrapping_sub(1);
                    self.ram[r.sp as usize] = (IDLE_ADDR >> 8) as u8;
                    r.sp = r.sp.wrapping_sub(1);
                    self.ram[r.sp as usize] = (IDLE_ADDR & 0xFF) as u8;
                    r.pc = get_le16(&self.header_[header::PLAY_ADDR..]) as u16;
                    // GME_FRAME_HOOK( this );
                }
            }
        }

        *duration = self.cpu.z80.time();
        self.next_play -= *duration;
        // check( next_play >= 0 );
        self.cpu.z80.adjust_time(-*duration);
        let d = *duration;
        let bufs = self.classic.bufs();
        self.ay.end_frame(bufs, d);
        self.scc.end_frame(bufs, d);
        if let Some(sn) = self.sn.as_mut() {
            sn.end_frame(bufs, d);
        }

        Ok(())
    }
}

impl MusicEmu for KssEmu {
    classic_emu_overrides!();

    fn unload(&mut self) {
        self.sn = None;
        music_emu_unload(self);
    }

    fn track_info_(&self, out: &mut TrackInfo, _track: i32) -> BlarggErr {
        copy_kss_fields(&self.header_, out);
        Ok(())
    }

    fn load_(&mut self, input: &mut dyn DataReader) -> BlarggErr {
        self.kss_load_(input)
    }

    fn start_track_(&mut self, track: i32) -> BlarggErr {
        self.kss_start_track_(track)
    }

    fn set_tempo_(&mut self, t: f64) {
        let period: BlipTime = if self.header_[header::DEVICE_FLAGS] & 0x40 != 0 {
            (CLOCK_RATE / 50) as BlipTime
        } else {
            (CLOCK_RATE / 60) as BlipTime
        };
        self.play_period = (period as f64 / t) as BlipTime;
    }
}
