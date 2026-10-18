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

//! Sinclair Spectrum AY music file emulator (`Ay_Emu.h`, `Ay_Emu.cpp`).
//!
//! Upstream points into the file's data (`file_t`); this keeps offsets into
//! it, in the loaded file's data.

use crate::ay_apu::{self, AyApu};
use crate::ay_cpu::{self, AyCpu, CpuTime, Instr, Z80Host, RA, RB, RC, RD, RE, RFLAGS, RH, RL};
use crate::blargg_common::{get_be16, BlarggErr};
use crate::blip_buffer::{BlipEq, BlipTime, BufId};
use crate::classic_emu::{
    classic_emu_overrides, classic_start_track_, setup_buffer, ClassicEmu, ClassicEmuState,
    MIXED_TYPE, WAVE_TYPE,
};
use crate::gme_file::{copy_field_n, GmeTypeT, TrackInfo, GME_WRONG_FILE_TYPE};
use crate::music_emu::{gme_info_overrides, MusicEmu, MusicEmuState};

const SPECTRUM_CLOCK: u32 = 3546900;
const CPC_CLOCK: u32 = 2000000;

const RAM_START: u32 = 0x4000;
const OSC_COUNT: i32 = ay_apu::OSC_COUNT + 1;

/// AY file header
pub const HEADER_SIZE: usize = 0x14;

/// The fields of the AY `header_t`, as offsets into its bytes.
pub mod header {
    pub const TAG: usize = 0; // [8]
    pub const VERS: usize = 8;
    pub const PLAYER: usize = 9;
    pub const UNUSED: usize = 10; // [2]
    pub const AUTHOR: usize = 12; // [2]
    pub const COMMENT: usize = 14; // [2]
    pub const MAX_TRACK: usize = 16;
    pub const FIRST_TRACK: usize = 17;
    pub const TRACK_INFO: usize = 18; // [2]
}

/// `file_t`: the header (the start of the data), its end and the track
/// list, as offsets into the data.
#[derive(Clone, Copy, Debug, Default)]
pub struct FileT {
    pub header: usize,
    pub end: usize,
    pub tracks: usize,
}

static VOICE_NAMES: [&str; OSC_COUNT as usize] = ["Wave 1", "Wave 2", "Wave 3", "Beeper"];

static VOICE_TYPES: [i32; OSC_COUNT as usize] =
    [WAVE_TYPE, WAVE_TYPE | 1, WAVE_TYPE | 2, MIXED_TYPE];

const MEM_SIZE: usize = 0x10000 + 0x100;

pub struct AyEmu {
    pub(crate) state: MusicEmuState,
    pub(crate) classic: ClassicEmuState,
    cpu: AyCpu,

    file: FileT,
    play_period: CpuTime,
    next_play: CpuTime,
    beeper_output: Option<BufId>,
    beeper_delta: i32,
    last_beeper: i32,
    apu_addr: i32,
    cpc_latch: i32,
    spectrum_mode: bool,
    cpc_mode: bool,

    // large items
    /// `mem.ram` (`mem.padding1`, before it, is filled with 0xFF and never
    /// read)
    mem: Box<[u8]>,
    apu: AyApu,
}

impl std::fmt::Debug for AyEmu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AyEmu").finish_non_exhaustive()
    }
}

impl Default for AyEmu {
    fn default() -> Self {
        Self::new()
    }
}

// Track info

/// `get_data()`: the data `ptr`'s offset points to, if it has `min_size`
/// bytes in the file.
fn get_data(file: &FileT, data: &[u8], ptr: usize, min_size: i32) -> Option<usize> {
    let pos = ptr as i64 - file.header as i64;
    let file_size = file.end as i64 - file.header as i64;
    // assert( (unsigned long) pos <= (unsigned long) file_size - 2 );
    if pos as u64 > (file_size - 2) as u64 {
        return None; // Don't read farther than buffer allows
    }
    let offset = get_be16(&data[ptr..]) as i16 as i64;
    // Note (upstream): with a minimum size larger than the file, the
    // check below wraps around and passes any offset; that fails here
    // (upstream's build for the tests is patched the same).
    if offset == 0
        || file_size < min_size as i64
        || (pos + offset) as u32 > (file_size - min_size as i64) as u32
    {
        return None;
    }
    Some((ptr as i64 + offset) as usize)
}

fn parse_header(data: &[u8], begin: usize, size: usize, out: &mut FileT) -> BlarggErr {
    out.header = begin;
    out.end = begin + size;

    if size < HEADER_SIZE {
        return Err(GME_WRONG_FILE_TYPE);
    }

    let h = &data[begin..];
    if &h[header::TAG..header::TAG + 8] != b"ZXAYEMUL" {
        return Err(GME_WRONG_FILE_TYPE);
    }

    let max_track = h[header::MAX_TRACK] as i32;
    match get_data(out, data, begin + header::TRACK_INFO, (max_track + 1) * 4) {
        Some(t) => out.tracks = t,
        None => return Err("Missing track data"),
    }

    Ok(())
}

/// `Gme_File::copy_field_()` of a string in the file. Note (upstream): the
/// string can run on past the end of the file, which upstream reads past;
/// this stops at the end (upstream's build for the tests is patched the
/// same).
fn copy_ay_field(out: &mut [u8], file: &FileT, data: &[u8], at: Option<usize>) {
    if let Some(at) = at {
        copy_field_n(out, &data[at..file.end], (file.end - at) as i32);
    }
}

fn copy_ay_fields(file: &FileT, data: &[u8], out: &mut TrackInfo, track: i32) {
    let t = file.tracks + track as usize * 4;
    copy_ay_field(&mut out.song, file, data, get_data(file, data, t, 1));
    if let Some(track_info) = get_data(file, data, t + 2, 6) {
        out.length = get_be16(&data[track_info + 4..]) as i64 * (1000 / 50); // frames to msec
    }
    let h = file.header;
    copy_ay_field(
        &mut out.author,
        file,
        data,
        get_data(file, data, h + header::AUTHOR, 1),
    );
    copy_ay_field(
        &mut out.comment,
        file,
        data,
        get_data(file, data, h + header::COMMENT, 1),
    );
}

/// `Ay_File`: the info-only AY reader
pub(crate) struct AyFile {
    state: MusicEmuState,
    file: FileT,
}

impl AyFile {
    fn new() -> Self {
        let mut state = MusicEmuState::new();
        state.set_type(&GME_AY_TYPE_);
        AyFile {
            state,
            file: FileT::default(),
        }
    }
}

impl MusicEmu for AyFile {
    gme_info_overrides!();

    fn load_mem_(&mut self, begin: usize, size: usize) -> BlarggErr {
        parse_header(&self.state.file.file_data, begin, size, &mut self.file)?;
        let n = self.state.file.file_data[self.file.header + header::MAX_TRACK] as i32 + 1;
        self.state.set_track_count(n);
        Ok(())
    }

    fn track_info_(&self, out: &mut TrackInfo, track: i32) -> BlarggErr {
        copy_ay_fields(&self.file, &self.state.file.file_data, out, track);
        Ok(())
    }
}

fn new_ay_emu() -> Box<dyn MusicEmu> {
    Box::new(AyEmu::new())
}
fn new_ay_file() -> Box<dyn MusicEmu> {
    Box::new(AyFile::new())
}

pub(crate) static GME_AY_TYPE_: GmeTypeT = GmeTypeT {
    system: "ZX Spectrum",
    track_count: 0,
    new_emu: new_ay_emu,
    new_info: new_ay_file,
    extension_: "AY",
    flags_: 1,
};

// copy and configure driver
static PASSIVE: [u8; 10] = [
    0xF3, // DI
    0xCD, 0, 0, // CALL init
    0xED, 0x5E, // LOOP: IM 2
    0xFB, // EI
    0x76, // HALT
    0x18, 0xFA, // JR LOOP
];

static ACTIVE: [u8; 13] = [
    0xF3, // DI
    0xCD, 0, 0, // CALL init
    0xED, 0x56, // LOOP: IM 1
    0xFB, // EI
    0x76, // HALT
    0xCD, 0, 0, // CALL play
    0x18, 0xF7, // JR LOOP
];

impl AyEmu {
    pub fn new() -> Self {
        let mut state = MusicEmuState::new();
        state.set_type(&GME_AY_TYPE_);
        state.set_voice_names(&VOICE_NAMES);
        state.set_silence_lookahead(6);
        let mut classic = ClassicEmuState::new();
        classic.set_voice_types(&VOICE_TYPES);
        AyEmu {
            state,
            classic,
            cpu: AyCpu::new(),
            file: FileT::default(),
            play_period: 0,
            next_play: 0,
            beeper_output: None,
            beeper_delta: 0,
            last_beeper: 0,
            // Note (upstream): left uninitialized, and an AY register
            // write before the address is selected uses it as the register
            // index (out of the registers' bounds); it starts at 0 here
            // (upstream's build for the tests is patched the same).
            apu_addr: 0,
            cpc_latch: 0,
            spectrum_mode: false,
            cpc_mode: false,
            mem: vec![0u8; MEM_SIZE].into_boxed_slice(),
            apu: AyApu::new(),
        }
    }

    fn ay_load_mem_(&mut self, begin: usize, size: usize) -> BlarggErr {
        parse_header(&self.state.file.file_data, begin, size, &mut self.file)?;
        let h = &self.state.file.file_data[self.file.header..];
        let (max_track, vers) = (h[header::MAX_TRACK] as i32, h[header::VERS]);
        self.state.set_track_count(max_track + 1);

        if vers > 2 {
            self.state.set_warning("Unknown file version");
        }

        self.state.set_voice_count(OSC_COUNT);
        let gain = self.state.gain();
        self.apu.volume(gain);

        setup_buffer(self, SPECTRUM_CLOCK)
    }

    fn ay_start_track_(&mut self, track: i32) -> BlarggErr {
        classic_start_track_(self, track)?;

        self.mem[0x0000..0x0100].fill(0xC9); // fill RST vectors with RET
        self.mem[0x0100..0x4000].fill(0xFF);
        self.mem[RAM_START as usize..].fill(0x00);
        // (memset( mem.padding1, 0xFF, sizeof mem.padding1 ))
        self.mem[0x10000..].fill(0xFF);

        let file = self.file;
        // (a copy, as the memory and the warnings change as it is read)
        let fd = &self.state.file.file_data.clone();

        // locate data blocks
        let Some(data) = get_data(&file, fd, file.tracks + track as usize * 4 + 2, 14) else {
            return Err("File data missing");
        };

        let Some(more_data) = get_data(&file, fd, data + 10, 6) else {
            return Err("File data missing");
        };

        let Some(mut blocks) = get_data(&file, fd, data + 12, 8) else {
            return Err("File data missing");
        };

        // initial addresses
        self.cpu.reset();
        let r = &mut self.cpu.r;
        r.sp = get_be16(&fd[more_data..]) as u16;
        let (d8, d9) = (fd[data + 8], fd[data + 9]);
        for i in [RA, RB, RD, RH] {
            r.b.0[i] = d8;
        }
        for i in [RFLAGS, RC, RE, RL] {
            r.b.0[i] = d9;
        }
        r.alt = r.b;
        let hl = r.b.hl();
        r.ix = hl;
        r.iy = hl;

        let mut addr = get_be16(&fd[blocks..]);
        if addr == 0 {
            return Err("File data missing");
        }

        let mut init = get_be16(&fd[more_data + 2..]);
        if init == 0 {
            init = addr;
        }

        // copy blocks into memory
        loop {
            blocks += 2;
            let mut len = get_be16(&fd[blocks..]);
            blocks += 2;
            if addr + len > 0x10000 {
                self.state.set_warning("Bad data block size");
                len = 0x10000 - addr;
            }
            // check( len );
            let input = get_data(&file, fd, blocks, 0);
            blocks += 2;
            // Note (upstream): when the offset is bad, upstream copies from
            // a null pointer (and crashes); this copies nothing (upstream's
            // build for the tests is patched the same).
            match input {
                Some(input) => {
                    if len as usize > file.end - input {
                        self.state.set_warning("Missing file data");
                        len = (file.end - input) as u32;
                    }
                    //debug_printf( "addr: $%04X, len: $%04X\n", addr, len );
                    // if ( addr < ram_start && addr >= 0x400 ) // several tracks use low data
                    //     debug_printf( "Block addr in ROM\n" );
                    let a = addr as usize;
                    self.mem[a..a + len as usize].copy_from_slice(&fd[input..input + len as usize]);
                }
                None => {
                    self.state.set_warning("Missing file data");
                }
            }

            if (file.end as i64 - blocks as i64) < 8 {
                self.state.set_warning("Missing file data");
                break;
            }
            addr = get_be16(&fd[blocks..]);
            if addr == 0 {
                break;
            }
        }

        // copy and configure driver
        self.mem[..PASSIVE.len()].copy_from_slice(&PASSIVE);
        let play_addr = get_be16(&fd[more_data + 4..]);
        //debug_printf( "Play: $%04X\n", play_addr );
        if play_addr != 0 {
            self.mem[..ACTIVE.len()].copy_from_slice(&ACTIVE);
            self.mem[9] = play_addr as u8;
            self.mem[10] = (play_addr >> 8) as u8;
        }
        self.mem[2] = init as u8;
        self.mem[3] = (init >> 8) as u8;

        self.mem[0x38] = 0xFB; // Put EI at interrupt vector (followed by RET)

        self.mem.copy_within(0..0x80, 0x10000); // some code wraps around (ugh)

        self.beeper_delta = (ay_apu::AMP_RANGE as f64 * 0.65) as i32;
        self.last_beeper = 0;
        self.apu.reset();
        self.next_play = self.play_period;

        // start at spectrum speed
        self.classic.change_clock_rate(SPECTRUM_CLOCK);
        let t = self.state.tempo();
        self.set_tempo(t);

        self.spectrum_mode = false;
        self.cpc_mode = false;
        self.cpc_latch = 0;

        Ok(())
    }

    // Emulation

    fn cpu_out_misc(&mut self, time: CpuTime, addr: u32, data: i32) {
        'enable_cpc: {
            if !self.cpc_mode {
                match addr & 0xFEFF {
                    0xFEFD => {
                        self.spectrum_mode = true;
                        self.apu_addr = data & 0x0F;
                        return;
                    }

                    0xBEFD => {
                        self.spectrum_mode = true;
                        let a = self.apu_addr;
                        self.apu.write(self.classic.bufs(), time, a, data);
                        return;
                    }
                    _ => {}
                }
            }

            if !self.spectrum_mode {
                match addr >> 8 {
                    0xF6 => match data & 0xC0 {
                        0xC0 => {
                            self.apu_addr = self.cpc_latch & 0x0F;
                            break 'enable_cpc;
                        }

                        0x80 => {
                            let (a, d) = (self.apu_addr, self.cpc_latch);
                            self.apu.write(self.classic.bufs(), time, a, d);
                            break 'enable_cpc;
                        }
                        _ => {}
                    },

                    0xF4 => {
                        self.cpc_latch = data;
                        break 'enable_cpc;
                    }
                    _ => {}
                }
            }

            // debug_printf( "Unmapped OUT: $%04X <- $%02X\n", addr, data );
            return;
        }

        // enable_cpc:
        if !self.cpc_mode {
            self.cpc_mode = true;
            self.classic.change_clock_rate(CPC_CLOCK);
            let t = self.state.tempo();
            self.set_tempo(t);
        }
    }
}

impl Z80Host for AyEmu {
    const PC_16: bool = true;
    const INSTR_OPERANDS: bool = false;

    fn cpu(&mut self) -> &mut AyCpu {
        &mut self.cpu
    }

    // (Ay_Cpu reads and writes its memory directly: mem [addr])
    #[inline]
    fn read(&mut self, addr: u32) -> u8 {
        self.mem[addr as usize]
    }
    #[inline]
    fn read_word(&mut self, addr: u32) -> u16 {
        let a = addr as usize;
        self.mem[a] as u16 | (self.mem[a + 1] as u16) << 8
    }
    #[inline]
    fn write(&mut self, addr: u32, data: u8) {
        self.mem[addr as usize] = data;
    }
    #[inline]
    fn write_word(&mut self, addr: u32, data: u16) {
        let a = addr as usize;
        self.mem[a] = data as u8;
        self.mem[a + 1] = (data >> 8) as u8;
    }
    #[inline]
    fn instr(&mut self, pc: u32) -> Instr {
        Instr {
            mem: 0,
            offset: pc as usize,
        }
    }
    #[inline]
    fn instr_byte(&mut self, instr: Instr, k: usize) -> u8 {
        self.mem[instr.offset + k]
    }

    /// `ay_cpu_out()`
    fn cpu_out(&mut self, time: CpuTime, addr: u32, mut data: i32) {
        if (addr & 0xFF) == 0xFE && !self.cpc_mode {
            let delta = self.beeper_delta;
            data &= 0x10;
            if self.last_beeper != data {
                self.last_beeper = data;
                self.beeper_delta = -delta;
                self.spectrum_mode = true;
                if let Some(out) = self.beeper_output {
                    self.apu
                        .synth_
                        .offset(time, delta, &mut self.classic.bufs()[out]);
                }
            }
        } else {
            self.cpu_out_misc(time, addr, data);
        }
    }

    /// `ay_cpu_in()`
    fn cpu_in(&mut self, _time: CpuTime, addr: u32) -> i32 {
        // keyboard read and other things
        if (addr & 0xFF) == 0xFE {
            return 0xFF; // other values break some beeper tunes
        }

        // debug_printf( "Unmapped IN : $%04X\n", addr );
        0xFF
    }
}

impl ClassicEmu for AyEmu {
    fn classic(&self) -> &ClassicEmuState {
        &self.classic
    }
    fn classic_mut(&mut self) -> &mut ClassicEmuState {
        &mut self.classic
    }

    fn update_eq(&mut self, eq: &BlipEq) {
        self.apu.treble_eq(eq);
    }

    fn set_voice(&mut self, i: i32, center: Option<BufId>, _l: Option<BufId>, _r: Option<BufId>) {
        if i >= ay_apu::OSC_COUNT {
            self.beeper_output = center;
        } else {
            self.apu.osc_output(i, center);
        }
    }

    fn run_clocks(&mut self, duration: &mut BlipTime, _msec: i32) -> BlarggErr {
        self.cpu.set_time(0);
        if !(self.spectrum_mode | self.cpc_mode) {
            *duration /= 2; // until mode is set, leave room for halved clock rate
        }

        while self.cpu.time() < *duration {
            ay_cpu::run(self, (*duration).min(self.next_play));

            if self.cpu.time() >= self.next_play {
                self.next_play += self.play_period;

                if self.cpu.r.iff1 != 0 {
                    let r = &mut self.cpu.r;
                    if self.mem[r.pc as usize] == 0x76 {
                        r.pc = r.pc.wrapping_add(1);
                    }

                    r.iff1 = 0;
                    r.iff2 = 0;

                    r.sp = r.sp.wrapping_sub(1);
                    self.mem[r.sp as usize] = (r.pc >> 8) as u8;
                    r.sp = r.sp.wrapping_sub(1);
                    self.mem[r.sp as usize] = r.pc as u8;
                    r.pc = 0x38;
                    self.cpu.adjust_time(12);
                    if self.cpu.r.im == 2 {
                        self.cpu.adjust_time(6);
                        let addr = self.cpu.r.i as u32 * 0x100 + 0xFF;
                        self.cpu.r.pc = (self.mem[((addr + 1) & 0xFFFF) as usize] as u32 * 0x100
                            + self.mem[addr as usize] as u32)
                            as u16;
                    }
                }
            }
        }
        *duration = self.cpu.time();
        self.next_play -= *duration;
        // check( next_play >= 0 );
        self.cpu.adjust_time(-*duration);

        let d = *duration;
        self.apu.end_frame(self.classic.bufs(), d);

        Ok(())
    }
}

impl MusicEmu for AyEmu {
    classic_emu_overrides!();

    fn track_info_(&self, out: &mut TrackInfo, track: i32) -> BlarggErr {
        copy_ay_fields(&self.file, &self.state.file.file_data, out, track);
        Ok(())
    }

    // Setup

    fn load_mem_(&mut self, begin: usize, size: usize) -> BlarggErr {
        self.ay_load_mem_(begin, size)
    }

    fn start_track_(&mut self, track: i32) -> BlarggErr {
        self.ay_start_track_(track)
    }

    fn set_tempo_(&mut self, t: f64) {
        self.play_period = ((self.classic.clock_rate() / 50) as f64 / t) as BlipTime;
    }
}
