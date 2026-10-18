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

//! Atari XL/XE SAP music file emulator (`Sap_Emu.h`, `Sap_Emu.cpp`,
//! `sap_cpu_io.h`).

use crate::blargg_common::{get_le16, BlarggErr};
use crate::blip_buffer::{BlipEq, BlipTime, BufId};
use crate::classic_emu::{
    classic_emu_overrides, classic_start_track_, setup_buffer, ClassicEmu, ClassicEmuState,
    WAVE_TYPE,
};
use crate::gme_file::{copy_field_, GmeTypeT, TrackInfo, GME_WRONG_FILE_TYPE};
use crate::music_emu::{gme_info_overrides, MusicEmu, MusicEmuState};
use crate::sap_apu::{self, SapApu, SapApuImpl};
use crate::sap_cpu::{self, SapAddr, SapCpu, SapCpuHost, SapTime, IDLE_ADDR};

const BASE_SCANLINE_PERIOD: i64 = 114;

static VOICE_NAMES: [&str; sap_apu::OSC_COUNT as usize * 2] = [
    "Wave 1", "Wave 2", "Wave 3", "Wave 4", "Wave 5", "Wave 6", "Wave 7", "Wave 8",
];

static VOICE_TYPES: [i32; sap_apu::OSC_COUNT as usize * 2] = [
    WAVE_TYPE | 1,
    WAVE_TYPE | 2,
    WAVE_TYPE | 3,
    WAVE_TYPE,
    WAVE_TYPE | 5,
    WAVE_TYPE | 6,
    WAVE_TYPE | 7,
    WAVE_TYPE | 4,
];

/// The rip's header (`info_t`); `rom_data` is an index into the file data.
#[derive(Clone, Debug)]
pub struct Info {
    pub rom_data: usize,
    pub warning: Option<&'static str>,
    pub init_addr: i64,
    pub play_addr: i64,
    pub music_addr: i64,
    pub type_: i32,
    pub track_count: i32,
    pub fastplay: i32,
    pub stereo: bool,
    pub author: [u8; 256],
    pub name: [u8; 256],
    pub copyright: [u8; 32],
}

impl Default for Info {
    fn default() -> Self {
        Info {
            rom_data: 0,
            warning: None,
            init_addr: 0,
            play_addr: 0,
            music_addr: 0,
            type_: 0,
            track_count: 0,
            fastplay: 0,
            stereo: false,
            author: [0; 256],
            name: [0; 256],
            copyright: [0; 32],
        }
    }
}

/// The size of `mem.ram`: the 64K memory and the bytes past it that reads
/// and writes past 0xFFFF reach (upstream's `mem` also has 0x100 bytes of
/// padding before it, which nothing reaches).
const RAM_SIZE: usize = 0x10000 + 0x100;

pub struct SapEmu {
    pub(crate) state: MusicEmuState,
    pub(crate) classic: ClassicEmuState,
    cpu: SapCpu,

    info: Info,

    file_end: usize,
    scanline_period: SapTime,
    next_play: SapTime,
    time_mask: SapTime,
    apu: SapApu,
    apu2: SapApu,

    // large items
    ram: Box<[u8]>,
    apu_impl: Box<SapApuImpl>,
}

impl std::fmt::Debug for SapEmu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SapEmu").finish_non_exhaustive()
    }
}

impl Default for SapEmu {
    fn default() -> Self {
        Self::new()
    }
}

impl SapEmu {
    pub fn new() -> Self {
        let mut state = MusicEmuState::new();
        state.set_type(&GME_SAP_TYPE_);
        state.set_voice_names(&VOICE_NAMES);
        let mut classic = ClassicEmuState::new();
        classic.set_voice_types(&VOICE_TYPES);
        state.set_silence_lookahead(6);
        SapEmu {
            state,
            classic,
            cpu: SapCpu::new(),
            info: Info::default(),
            file_end: 0,
            // (upstream leaves these uninitialized until set_tempo_() and
            // start_track_() set them)
            scanline_period: 0,
            next_play: 0,
            time_mask: 0,
            apu: SapApu::new(),
            apu2: SapApu::new(),
            ram: vec![0u8; RAM_SIZE].into_boxed_slice(),
            apu_impl: Box::new(SapApuImpl::new()),
        }
    }
}

// Track info

/// Returns 16 or greater if not hex
#[inline]
fn from_hex_char(mut h: i32) -> i32 {
    h -= 0x30;
    if h as u32 > 9 {
        h = ((h - 0x11) & 0xDF) + 10;
    }
    h
}

fn from_hex(input: &[u8]) -> i64 {
    let mut result: u32 = 0;
    for &b in &input[..4] {
        let h = from_hex_char(b as i32);
        if h > 15 {
            return -1;
        }
        result = result.wrapping_mul(0x10).wrapping_add(h as u32);
    }
    result as i64
}

fn from_dec(data: &[u8], mut input: usize, end: usize) -> i32 {
    if input >= end {
        return -1;
    }

    let mut n: i32 = 0;
    while input < end {
        let dig = data[input] as i32 - b'0' as i32;
        input += 1;
        if dig as u32 > 9 {
            return -1;
        }
        n = n.wrapping_mul(10).wrapping_add(dig);
    }
    n
}

fn parse_string(data: &[u8], mut input: usize, end: usize, len: i32, out: &mut [u8]) {
    let mut start = input;
    let first = data[input];
    input += 1;
    if first == b'"' {
        start += 1;
        while input < end && data[input] != b'"' {
            input += 1;
        }
    } else {
        input = end;
    }
    let len = (len - 1).min(input as i32 - start as i32) as usize;
    out[len] = 0;
    out[..len].copy_from_slice(&data[start..start + len]);
}

/// `!strncmp( lit, tag, tag_len )`
fn tag_is(lit: &[u8], tag: &[u8]) -> bool {
    for (k, &b) in tag.iter().enumerate() {
        let a = lit.get(k).copied().unwrap_or(0);
        if a != b {
            return false;
        }
        if a == 0 {
            return true;
        }
    }
    true
}

fn parse_info(data: &[u8], begin: usize, size: usize, out: &mut Info) -> BlarggErr {
    out.track_count = 1;
    out.author[0] = 0;
    out.name[0] = 0;
    out.copyright[0] = 0;

    if size < 16 || &data[begin..begin + 5] != b"SAP\x0D\x0A" {
        return Err(GME_WRONG_FILE_TYPE);
    }

    let file_end = begin + size - 5;
    let mut input = begin + 5;
    while input < file_end && (data[input] != 0xFF || data[input + 1] != 0xFF) {
        let mut line_end = input;
        while line_end < file_end && data[line_end] != 0x0D {
            line_end += 1;
        }

        let tag = input;
        while input < line_end && data[input] > b' ' {
            input += 1;
        }
        let tag_len = input - tag;
        let tag = &data[tag..tag + tag_len];

        while input < line_end && data[input] <= b' ' {
            input += 1;
        }

        if tag_len == 0 {
            // skip line
        } else if tag_is(b"INIT", tag) {
            out.init_addr = from_hex(&data[input..]);
            if out.init_addr as u64 > 0xFFFF {
                return Err("Invalid init address");
            }
        } else if tag_is(b"PLAYER", tag) {
            out.play_addr = from_hex(&data[input..]);
            if out.play_addr as u64 > 0xFFFF {
                return Err("Invalid play address");
            }
        } else if tag_is(b"MUSIC", tag) {
            out.music_addr = from_hex(&data[input..]);
            if out.music_addr as u64 > 0xFFFF {
                return Err("Invalid music address");
            }
        } else if tag_is(b"SONGS", tag) {
            out.track_count = from_dec(data, input, line_end);
            if out.track_count <= 0 {
                return Err("Invalid track count");
            }
        } else if tag_is(b"TYPE", tag) {
            out.type_ = data[input] as i32;
            match data[input] {
                b'C' | b'B' => {}

                b'D' => return Err("Digimusic not supported"),

                _ => return Err("Unsupported player type"),
            }
        } else if tag_is(b"STEREO", tag) {
            out.stereo = true;
        } else if tag_is(b"FASTPLAY", tag) {
            out.fastplay = from_dec(data, input, line_end);
            if out.fastplay <= 0 {
                return Err("Invalid fastplay value");
            }
        } else if tag_is(b"AUTHOR", tag) {
            let n = out.author.len() as i32;
            parse_string(data, input, line_end, n, &mut out.author);
        } else if tag_is(b"NAME", tag) {
            let n = out.name.len() as i32;
            parse_string(data, input, line_end, n, &mut out.name);
        } else if tag_is(b"DATE", tag) {
            let n = out.copyright.len() as i32;
            parse_string(data, input, line_end, n, &mut out.copyright);
        }

        input = line_end + 2;
    }

    if data[input] != 0xFF || data[input + 1] != 0xFF {
        return Err("ROM data missing");
    }
    out.rom_data = input + 2;

    Ok(())
}

fn copy_sap_fields(input: &Info, out: &mut TrackInfo) {
    copy_field_(&mut out.game, &input.name);
    copy_field_(&mut out.author, &input.author);
    copy_field_(&mut out.copyright, &input.copyright);
}

/// `Sap_File`: the info-only SAP reader
pub(crate) struct SapFile {
    state: MusicEmuState,
    info: Info,
}

impl SapFile {
    fn new() -> Self {
        let mut state = MusicEmuState::new();
        state.set_type(&GME_SAP_TYPE_);
        SapFile {
            state,
            info: Info::default(),
        }
    }
}

impl MusicEmu for SapFile {
    gme_info_overrides!();

    fn load_mem_(&mut self, begin: usize, size: usize) -> BlarggErr {
        parse_info(&self.state.file.file_data, begin, size, &mut self.info)?;
        self.state.set_track_count(self.info.track_count);
        Ok(())
    }

    fn track_info_(&self, out: &mut TrackInfo, _track: i32) -> BlarggErr {
        copy_sap_fields(&self.info, out);
        Ok(())
    }
}

fn new_sap_emu() -> Box<dyn MusicEmu> {
    Box::new(SapEmu::new())
}
fn new_sap_file() -> Box<dyn MusicEmu> {
    Box::new(SapFile::new())
}

pub(crate) static GME_SAP_TYPE_: GmeTypeT = GmeTypeT {
    system: "Atari XL",
    track_count: 0,
    new_emu: new_sap_emu,
    new_info: new_sap_file,
    extension_: "SAP",
    flags_: 1,
};

impl SapEmu {
    // Setup

    fn sap_load_mem_(&mut self, begin: usize, size: usize) -> BlarggErr {
        self.file_end = begin + size;

        self.info.warning = None;
        self.info.type_ = b'B' as i32;
        self.info.stereo = false;
        self.info.init_addr = -1;
        self.info.play_addr = -1;
        self.info.music_addr = -1;
        self.info.fastplay = 312;
        parse_info(&self.state.file.file_data, begin, size, &mut self.info)?;

        // set_warning( info.warning ) (which is null)
        self.state.file.warning_ = self.info.warning;
        self.state.set_track_count(self.info.track_count);
        self.state
            .set_voice_count(sap_apu::OSC_COUNT << self.info.stereo as i32);
        let gain = self.state.gain();
        self.apu_impl.volume(gain);

        setup_buffer(self, 1773447)
    }

    // Emulation

    #[inline]
    fn play_period(&self) -> SapTime {
        self.info.fastplay.wrapping_mul(self.scanline_period)
    }

    fn cpu_jsr(&mut self, addr: SapAddr) {
        // check( r.sp >= 0xFE ); // catch anything trying to leave data on stack
        let r = &mut self.cpu.r;
        r.pc = addr as u16;
        let high_byte = ((IDLE_ADDR - 1) >> 8) as u8;
        if r.sp == 0xFE && self.ram[0x1FF] == high_byte {
            r.sp = 0xFF; // pop extra byte off
        }
        self.ram[0x100 + r.sp as usize] = high_byte; // some routines use RTI to return
        r.sp = r.sp.wrapping_sub(1);
        self.ram[0x100 + r.sp as usize] = high_byte;
        r.sp = r.sp.wrapping_sub(1);
        self.ram[0x100 + r.sp as usize] = ((IDLE_ADDR - 1) & 0xFF) as u8;
        r.sp = r.sp.wrapping_sub(1);
    }

    fn run_routine(&mut self, addr: SapAddr) {
        self.cpu_jsr(addr);
        sap_cpu::run(self, (312 * BASE_SCANLINE_PERIOD * 60) as SapTime);
        // check( r.pc == idle_addr );
    }

    #[inline]
    fn call_init(&mut self, track: i32) {
        match self.info.type_ as u8 {
            b'B' => {
                self.cpu.r.a = track as u8;
                self.run_routine(self.info.init_addr as SapAddr);
            }

            b'C' => {
                self.cpu.r.a = 0x70;
                self.cpu.r.x = (self.info.music_addr & 0xFF) as u8;
                self.cpu.r.y = (self.info.music_addr >> 8) as u8;
                self.run_routine((self.info.play_addr + 3) as SapAddr);
                self.cpu.r.a = 0;
                self.cpu.r.x = track as u8;
                self.run_routine((self.info.play_addr + 3) as SapAddr);
            }
            _ => {}
        }
    }

    fn sap_start_track_(&mut self, track: i32) -> BlarggErr {
        classic_start_track_(self, track)?;

        self.ram.fill(0);

        let file_end = self.file_end;
        let mut input = self.info.rom_data;
        while file_end as isize - input as isize >= 5 {
            let start = get_le16(&self.state.file.file_data[input..]) as usize;
            let end = get_le16(&self.state.file.file_data[input + 2..]) as usize;
            //debug_printf( "Block $%04X-$%04X\n", start, end );
            input += 4;
            if end < start {
                self.state.set_warning("Invalid file data block");
                break;
            }
            let len = end - start + 1;
            if len as isize > file_end as isize - input as isize {
                self.state.set_warning("Invalid file data block");
                break;
            }

            let fd = &self.state.file.file_data;
            self.ram[start..start + len].copy_from_slice(&fd[input..input + len]);
            input += len;
            if file_end as isize - input as isize >= 2 && fd[input] == 0xFF && fd[input + 1] == 0xFF
            {
                input += 2;
            }
        }

        self.apu.reset();
        self.apu2.reset();
        self.cpu.reset();
        self.time_mask = 0; // disables sound during init
        self.call_init(track);
        self.time_mask = -1;

        self.next_play = self.play_period();

        Ok(())
    }

    // see sap_cpu_io.h for read/write functions

    fn cpu_write_(&mut self, addr: SapAddr, data: i32) {
        if (addr ^ sap_apu::START_ADDR) <= (sap_apu::END_ADDR - sap_apu::START_ADDR) {
            // GME_APU_HOOK( this, addr - Sap_Apu::start_addr, data );
            let t = self.cpu.time() & self.time_mask;
            self.apu
                .write_data(&self.apu_impl, self.classic.bufs(), t, addr, data);
            return;
        }

        if (addr ^ (sap_apu::START_ADDR + 0x10)) <= (sap_apu::END_ADDR - sap_apu::START_ADDR)
            && self.info.stereo
        {
            // GME_APU_HOOK( this, addr - 0x10 - Sap_Apu::start_addr + 10, data );
            let t = self.cpu.time() & self.time_mask;
            self.apu2
                .write_data(&self.apu_impl, self.classic.bufs(), t, addr ^ 0x10, data);
            return;
        }

        // if ( (addr & ~0x0010) != 0xD20F || data != 0x03 )
        //  debug_printf( "Unmapped write $%04X <- $%02X\n", addr, data );
    }

    #[inline]
    fn call_play(&mut self) {
        match self.info.type_ as u8 {
            b'B' => self.cpu_jsr(self.info.play_addr as SapAddr),

            b'C' => self.cpu_jsr((self.info.play_addr + 6) as SapAddr),
            _ => {}
        }
    }
}

// sap_cpu_io.h

impl SapCpuHost for SapEmu {
    fn cpu(&mut self) -> &mut SapCpu {
        &mut self.cpu
    }

    fn mem(&mut self) -> &mut [u8] {
        &mut self.ram
    }

    fn cpu_write(&mut self, addr: SapAddr, data: i32) {
        self.ram[addr as usize] = data as u8;
        if (addr >> 8) == 0xD2 {
            self.cpu_write_(addr, data);
        }
    }
}

impl ClassicEmu for SapEmu {
    fn classic(&self) -> &ClassicEmuState {
        &self.classic
    }
    fn classic_mut(&mut self) -> &mut ClassicEmuState {
        &mut self.classic
    }

    fn update_eq(&mut self, eq: &BlipEq) {
        self.apu_impl.synth.treble_eq(eq);
    }

    fn set_voice(
        &mut self,
        i: i32,
        center: Option<BufId>,
        left: Option<BufId>,
        right: Option<BufId>,
    ) {
        let i2 = i - sap_apu::OSC_COUNT;
        if i2 >= 0 {
            self.apu2.osc_output(i2, right);
        } else {
            self.apu
                .osc_output(i, if self.info.stereo { left } else { center });
        }
    }

    fn run_clocks(&mut self, duration: &mut BlipTime, _msec: i32) -> BlarggErr {
        self.cpu.set_time(0);
        while self.cpu.time() < *duration {
            if sap_cpu::run(self, *duration) || self.cpu.r.pc as u32 > IDLE_ADDR {
                return Err("Emulation error (illegal instruction)");
            }

            if self.cpu.r.pc as u32 == IDLE_ADDR {
                if self.next_play <= *duration {
                    self.cpu.set_time(self.next_play);
                    self.next_play = self.next_play.wrapping_add(self.play_period());
                    self.call_play();
                    // GME_FRAME_HOOK( this );
                } else {
                    self.cpu.set_time(*duration);
                }
            }
        }

        *duration = self.cpu.time();
        self.next_play -= *duration;
        // check( next_play >= 0 );
        if self.next_play < 0 {
            self.next_play = 0;
        }
        let d = *duration;
        let bufs = self.classic.bufs();
        self.apu.end_frame(&self.apu_impl, bufs, d);
        if self.info.stereo {
            self.apu2.end_frame(&self.apu_impl, bufs, d);
        }

        Ok(())
    }
}

impl MusicEmu for SapEmu {
    classic_emu_overrides!();

    fn track_info_(&self, out: &mut TrackInfo, _track: i32) -> BlarggErr {
        copy_sap_fields(&self.info, out);
        Ok(())
    }

    fn load_mem_(&mut self, begin: usize, size: usize) -> BlarggErr {
        self.sap_load_mem_(begin, size)
    }

    fn start_track_(&mut self, track: i32) -> BlarggErr {
        self.sap_start_track_(track)
    }

    fn set_tempo_(&mut self, t: f64) {
        self.scanline_period = (BASE_SCANLINE_PERIOD as f64 / t) as SapTime;
    }
}
