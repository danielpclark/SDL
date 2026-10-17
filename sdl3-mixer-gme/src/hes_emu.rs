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

//! TurboGrafx-16/PC Engine HES music file emulator (`Hes_Emu.h`,
//! `Hes_Emu.cpp`, `hes_cpu_io.h`).

use crate::blargg_common::{get_le16, get_le32, BlarggErr};
use crate::blip_buffer::{BlipEq, BlipTime, BufId};
use crate::classic_emu::{
    classic_emu_overrides, classic_start_track_, setup_buffer, ClassicEmu, ClassicEmuState,
    RomData, MIXED_TYPE, WAVE_TYPE,
};
use crate::data_reader::{DataReader, EOF_ERROR};
use crate::gme_file::{copy_field_n, GmeTypeT, TrackInfo, GME_WRONG_FILE_TYPE};
use crate::hes_apu::{self, HesApu};
use crate::hes_apu_adpcm::{self, HesApuAdpcm};
use crate::hes_cpu::{
    self, CodePage, HesAddr, HesCpu, HesCpuHost, HesTime, CPU_PADDING, FUTURE_HES_TIME, IDLE_ADDR,
    MEM_RAM, PAGE_COUNT, PAGE_SHIFT, PAGE_SIZE,
};
use crate::music_emu::{gme_info_overrides, music_emu_unload, MusicEmu, MusicEmuState};

/// HES file header
pub const HEADER_SIZE: usize = 0x20;

/// The fields of the HES `header_t`, as offsets into its bytes.
pub mod header {
    pub const TAG: usize = 0; // [4]
    pub const VERS: usize = 4;
    pub const FIRST_TRACK: usize = 5;
    pub const INIT_ADDR: usize = 6; // [2]
    pub const BANKS: usize = 8; // [8]
    pub const DATA_TAG: usize = 16; // [4]
    pub const SIZE: usize = 20; // [4]
    pub const ADDR: usize = 24; // [4]
    pub const UNUSED: usize = 28; // [4]
}

// Memory regions of the code map (and MEM_RAM, the CPU's RAM)
const MEM_ROM: u8 = 1;
const MEM_SGX: u8 = 2;

const TIMER_MASK: u8 = 0x04;
const VDP_MASK: u8 = 0x02;
const I_FLAG_MASK: u8 = 0x04;
const UNMAPPED: i32 = 0xFF;

const PERIOD_60HZ: i64 = 262 * 455; // scanlines * clocks per scanline

static VOICE_NAMES: [&str; hes_apu::OSC_COUNT as usize] =
    ["Wave 1", "Wave 2", "Wave 3", "Wave 4", "Multi 1", "Multi 2"];

static VOICE_TYPES: [i32; hes_apu::OSC_COUNT as usize] = [
    WAVE_TYPE,
    WAVE_TYPE | 1,
    WAVE_TYPE | 2,
    WAVE_TYPE | 3,
    MIXED_TYPE,
    MIXED_TYPE | 1,
];

#[derive(Clone, Copy, Debug, Default)]
struct Timer {
    last_time: HesTime,
    count: i32,
    load: i32,
    raw_load: i32,
    enabled: u8,
    fired: u8,
}

#[derive(Clone, Copy, Debug, Default)]
struct Vdp {
    next_vbl: HesTime,
    latch: u8,
    control: u8,
}

#[derive(Clone, Copy, Debug, Default)]
struct Irq {
    timer: HesTime,
    vdp: HesTime,
    disables: u8,
}

pub struct HesEmu {
    pub(crate) state: MusicEmuState,
    pub(crate) classic: ClassicEmuState,
    cpu: HesCpu,

    write_pages: [Option<CodePage>; PAGE_COUNT + 1], // 0 if unmapped or I/O space

    rom: RomData,
    header_: [u8; HEADER_SIZE],
    play_period: HesTime,
    last_frame_hook: HesTime,
    timer_base: i32,

    timer: Timer,
    vdp: Vdp,
    irq: Irq,

    // large items
    apu: HesApu,
    adpcm: Box<HesApuAdpcm>,
    sgx: Box<[u8; 3 * PAGE_SIZE + CPU_PADDING]>,
}

impl std::fmt::Debug for HesEmu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HesEmu").finish_non_exhaustive()
    }
}

impl Default for HesEmu {
    fn default() -> Self {
        Self::new()
    }
}

impl HesEmu {
    pub fn new() -> Self {
        let mut state = MusicEmuState::new();
        state.set_type(&GME_HES_TYPE_);
        state.set_voice_names(&VOICE_NAMES);
        let mut classic = ClassicEmuState::new();
        classic.set_voice_types(&VOICE_TYPES);
        state.set_silence_lookahead(6);
        state.set_gain(1.11);
        HesEmu {
            state,
            classic,
            cpu: HesCpu::new(),
            write_pages: [None; PAGE_COUNT + 1],
            rom: RomData::new(PAGE_SIZE as i32),
            header_: [0; HEADER_SIZE],
            // (upstream leaves these uninitialized until set_tempo_() and
            // start_track_() set them)
            play_period: 0,
            last_frame_hook: 0,
            timer_base: 0,
            timer: Timer::default(),
            vdp: Vdp::default(),
            irq: Irq::default(),
            apu: HesApu::new(),
            adpcm: Box::new(HesApuAdpcm::new()),
            sgx: Box::new([0; 3 * PAGE_SIZE + CPU_PADDING]),
        }
    }

    /// Header for currently loaded file
    pub fn header(&self) -> &[u8; HEADER_SIZE] {
        &self.header_
    }
}

// Track info

fn copy_field<'a>(input: Option<&'a [u8]>, out: &mut [u8]) -> Option<&'a [u8]> {
    if let Some(inp) = input {
        let mut len = 0x20;
        if inp[0x1F] != 0 && inp[0x2F] == 0 {
            len = 0x30; // fields are sometimes 16 bytes longer (ugh)
        }

        // since text fields are where any data could be, detect non-text
        // and fields with data after zero byte terminator

        let mut i = 0;
        while i < len && inp[i] != 0 {
            if ((inp[i] as i32 + 1) & 0xFF) < b' ' as i32 + 1 {
                // also treat 0xFF as non-text
                return None; // non-ASCII found
            }
            i += 1;
        }

        while i < len {
            if inp[i] != 0 {
                return None; // data after terminator
            }
            i += 1;
        }

        copy_field_n(out, inp, len as i32);
        return Some(&inp[len..]);
    }
    input
}

fn copy_hes_fields(input: &[u8], out: &mut TrackInfo) {
    if input[0] >= b' ' {
        let input = copy_field(Some(input), &mut out.game);
        let input = copy_field(input, &mut out.author);
        copy_field(input, &mut out.copyright);
    }
}

fn check_hes_header(header: &[u8]) -> BlarggErr {
    if &header[..4] != b"HESM" {
        return Err(GME_WRONG_FILE_TYPE);
    }
    Ok(())
}

/// `Hes_File`: the info-only HES reader
pub(crate) struct HesFile {
    state: MusicEmuState,
    /// `header_t`: the header, 0x20 unused bytes, then the fields
    h: [u8; HEADER_SIZE + 0x20 + 0x30 * 3],
}

impl HesFile {
    fn new() -> Self {
        let mut state = MusicEmuState::new();
        state.set_type(&GME_HES_TYPE_);
        HesFile {
            state,
            h: [0; HEADER_SIZE + 0x20 + 0x30 * 3],
        }
    }
}

impl MusicEmu for HesFile {
    gme_info_overrides!();

    fn load_(&mut self, input: &mut dyn DataReader) -> BlarggErr {
        // blaarg_static_assert( offsetof (header_t,fields) == Hes_Emu::header_size + 0x20, ... );
        if let Err(err) = input.read(&mut self.h) {
            return Err(if err == EOF_ERROR {
                GME_WRONG_FILE_TYPE
            } else {
                err
            });
        }
        check_hes_header(&self.h)
    }

    fn track_info_(&self, out: &mut TrackInfo, _track: i32) -> BlarggErr {
        copy_hes_fields(&self.h[HEADER_SIZE + 0x20..], out);
        Ok(())
    }
}

fn new_hes_emu() -> Box<dyn MusicEmu> {
    Box::new(HesEmu::new())
}
fn new_hes_file() -> Box<dyn MusicEmu> {
    Box::new(HesFile::new())
}

pub(crate) static GME_HES_TYPE_: GmeTypeT = GmeTypeT {
    system: "PC Engine",
    track_count: 256,
    new_emu: new_hes_emu,
    new_info: new_hes_file,
    extension_: "HES",
    flags_: 1,
};

impl HesEmu {
    // Setup

    fn hes_load_(&mut self, input: &mut dyn DataReader) -> BlarggErr {
        // blaarg_static_assert( offsetof (header_t,unused [4]) == header_size, ... );
        self.rom
            .load(input, HEADER_SIZE as i32, &mut self.header_, UNMAPPED as u8)?;

        check_hes_header(&self.header_)?;

        if self.header_[header::VERS] != 0 {
            self.state.set_warning("Unknown file version");
        }

        if &self.header_[header::DATA_TAG..header::DATA_TAG + 4] != b"DATA" {
            self.state.set_warning("Data header missing");
        }

        if self.header_[header::UNUSED..header::UNUSED + 4] != [0; 4] {
            self.state.set_warning("Unknown header data");
        }

        // File spec supports multiple blocks, but I haven't found any, and
        // many files have bad sizes in the only block, so it's simpler to
        // just try to load the damn data as best as possible.

        let mut addr = get_le32(&self.header_[header::ADDR..]) as i64;
        let size = get_le32(&self.header_[header::SIZE..]) as i64;
        const ROM_MAX: i64 = 0x100000;
        if addr & !(ROM_MAX - 1) != 0 {
            self.state.set_warning("Invalid address");
            addr &= ROM_MAX - 1;
        }
        if (addr + size) as u64 > ROM_MAX as u64 {
            self.state.set_warning("Invalid size");
        }

        if size != self.rom.file_size() {
            let b = self.rom.begin();
            if size <= self.rom.file_size() - 4
                && &self.rom.rom[b + size as usize..b + size as usize + 4] == b"DATA"
            {
                self.state.set_warning("Multiple DATA not supported");
            } else if size < self.rom.file_size() {
                self.state.set_warning("Extra file data");
            } else {
                self.state.set_warning("Missing file data");
            }
        }

        self.rom.set_addr(addr);

        self.state.set_voice_count(hes_apu::OSC_COUNT);

        let gain = self.state.gain();
        self.apu.volume(gain);

        setup_buffer(self, 7159091)
    }

    // Emulation

    fn recalc_timer_load(&mut self) {
        self.timer.load = self
            .timer
            .raw_load
            .wrapping_mul(self.timer_base)
            .wrapping_add(1);
    }

    fn hes_start_track_(&mut self, track: i32) -> BlarggErr {
        classic_start_track_(self, track)?;

        self.cpu.ram.fill(0); // some HES music relies on zero fill
        self.sgx.fill(0);

        self.apu.reset();
        self.adpcm.reset();
        self.cpu.reset();

        for i in 0..8 {
            let bank = self.header_[header::BANKS + i] as i32;
            hes_cpu::set_mmr(self, i as i32, bank);
        }
        hes_cpu::set_mmr(self, PAGE_COUNT as i32, 0xFF); // unmapped beyond end of address space

        self.irq.disables = TIMER_MASK | VDP_MASK;
        self.irq.timer = FUTURE_HES_TIME;
        self.irq.vdp = FUTURE_HES_TIME;

        self.timer.enabled = 0;
        self.timer.raw_load = 0x80;
        self.timer.count = self.timer.load;
        self.timer.fired = 0;
        self.timer.last_time = 0;

        self.vdp.latch = 0;
        self.vdp.control = 0;
        self.vdp.next_vbl = 0;

        self.cpu.ram[0x1FF] = ((IDLE_ADDR - 1) >> 8) as u8;
        self.cpu.ram[0x1FE] = ((IDLE_ADDR - 1) & 0xFF) as u8;
        self.cpu.r.sp = 0xFD;
        self.cpu.r.pc = get_le16(&self.header_[header::INIT_ADDR..]) as u16;
        self.cpu.r.a = track as u8;

        self.recalc_timer_load();
        self.last_frame_hook = 0;

        Ok(())
    }

    // Hardware

    fn time(&self) -> HesTime {
        self.cpu.time()
    }

    fn cpu_write_vdp_(&mut self, addr: i32, data: i32) {
        match addr {
            0 => {
                self.vdp.latch = (data & 0x1F) as u8;
            }

            2 => {
                if self.vdp.latch == 5 {
                    if data & 0x04 != 0 {
                        self.state.set_warning("Scanline interrupt unsupported");
                    }
                    self.run_until(self.time());
                    self.vdp.control = data as u8;
                    self.irq_changed();
                } else {
                    // debug_printf( "VDP not supported: $%02X <- $%02X\n", vdp.latch, data );
                }
            }

            3 => {
                // debug_printf( "VDP MSB not supported: $%02X <- $%02X\n", vdp.latch, data );
            }
            _ => {}
        }
    }

    fn cpu_write_(&mut self, addr: HesAddr, mut data: i32) {
        if addr.wrapping_sub(hes_apu::START_ADDR) <= hes_apu::END_ADDR - hes_apu::START_ADDR {
            // GME_APU_HOOK( this, addr - apu.start_addr, data );
            // avoid going way past end when a long block xfer is writing to I/O space
            let t = self.time().min(self.cpu.end_time().wrapping_add(8));
            self.apu
                .write_data(self.classic.bufs(), t, addr as i32, data);
            return;
        }

        if addr.wrapping_sub(hes_apu_adpcm::IO_ADDR) < hes_apu_adpcm::IO_SIZE {
            let t = self.time().min(self.cpu.end_time().wrapping_add(6));
            self.adpcm
                .write_data(self.classic.bufs(), t, addr as i32, data);
            return;
        }

        let time = self.time();
        match addr {
            0x0000 | 0x0002 | 0x0003 => {
                self.cpu_write_vdp_(addr as i32, data);
                return;
            }

            0x0C00 => {
                self.run_until(time);
                self.timer.raw_load = (data & 0x7F) + 1;
                self.recalc_timer_load();
                self.timer.count = self.timer.load;
            }

            0x0C01 => {
                data &= 1;
                if self.timer.enabled as i32 == data {
                    return;
                }
                self.run_until(time);
                self.timer.enabled = data as u8;
                if data != 0 {
                    self.timer.count = self.timer.load;
                }
            }

            0x1402 => {
                self.run_until(time);
                self.irq.disables = data as u8;
                // if ( (data & 0xF8) && (data & 0xF8) != 0xF8 ) // flag questionable values
                //  debug_printf( "Int mask: $%02X\n", data );
            }

            0x1403 => {
                self.run_until(time);
                if self.timer.enabled != 0 {
                    self.timer.count = self.timer.load;
                }
                self.timer.fired = 0;
            }

            // (#ifndef NDEBUG: the I/O port, the palette and unmapped
            // writes return here; the build this matches defines NDEBUG)
            _ => {}
        }

        self.irq_changed();
    }

    fn cpu_read_(&mut self, addr: HesAddr) -> i32 {
        let time = self.time();
        let addr = addr & (PAGE_SIZE as u32 - 1);
        match addr {
            0x0000 => {
                if self.irq.vdp > time {
                    return 0;
                }
                self.irq.vdp = FUTURE_HES_TIME;
                self.run_until(time);
                self.irq_changed();
                return 0x20;
            }

            0x0002 | 0x0003 => {
                // debug_printf( "VDP read not supported: %d\n", addr );
                return 0;
            }

            0x0C01 | 0x0C00 => {
                //return timer.enabled; // TODO: remove?
                self.run_until(time);
                // debug_printf( "Timer count read\n" );
                return ((self.timer.count.wrapping_sub(1) as u32) / self.timer_base as u32) as i32;
            }

            0x1402 => return self.irq.disables as i32,

            0x1403 => {
                let mut status = 0;
                if self.irq.timer <= time {
                    status |= TIMER_MASK as i32;
                }
                if self.irq.vdp <= time {
                    status |= VDP_MASK as i32;
                }
                return status;
            }

            0x180A..=0x180D => {
                return self.adpcm.read_data(self.classic.bufs(), time, addr as i32);
            }

            // (#ifndef NDEBUG: the I/O port and unmapped reads; the build
            // this matches defines NDEBUG)
            _ => {}
        }

        UNMAPPED
    }

    // see hes_cpu_io.h for core read/write functions

    // Emulation

    fn run_until(&mut self, present: HesTime) {
        while self.vdp.next_vbl < present {
            self.vdp.next_vbl = self.vdp.next_vbl.wrapping_add(self.play_period);
        }

        let elapsed = present.wrapping_sub(self.timer.last_time);
        if elapsed > 0 {
            if self.timer.enabled != 0 {
                self.timer.count = self.timer.count.wrapping_sub(elapsed);
                if self.timer.count <= 0 {
                    self.timer.count = self.timer.count.wrapping_add(self.timer.load);
                }
            }
            self.timer.last_time = present;
        }
    }

    fn irq_changed(&mut self) {
        let present = self.time();

        if self.irq.timer > present {
            self.irq.timer = FUTURE_HES_TIME;
            if self.timer.enabled != 0 && self.timer.fired == 0 {
                self.irq.timer = present.wrapping_add(self.timer.count);
            }
        }

        if self.irq.vdp > present {
            self.irq.vdp = FUTURE_HES_TIME;
            if self.vdp.control & 0x08 != 0 {
                self.irq.vdp = self.vdp.next_vbl;
            }
        }

        let mut time = FUTURE_HES_TIME;
        if self.irq.disables & TIMER_MASK == 0 {
            time = self.irq.timer;
        }
        if self.irq.disables & VDP_MASK == 0 {
            time = time.min(self.irq.vdp);
        }

        self.cpu.set_irq_time(time);
    }
}

// hes_cpu_io.h

impl HesCpuHost for HesEmu {
    fn cpu(&mut self) -> &mut HesCpu {
        &mut self.cpu
    }

    #[inline]
    fn code_byte(&self, mem: u8, offset: usize) -> u8 {
        // (the pages, and the bytes the CPU reads past their ends, are all
        // within the ROM data's padding, the RAM's or the SGX RAM's)
        match mem {
            MEM_RAM => self.cpu.ram[offset],
            MEM_SGX => self.sgx[offset],
            _ => self.rom.rom[offset],
        }
    }

    fn cpu_read(&mut self, addr: HesAddr) -> i32 {
        // check( addr <= 0xFFFF );
        let p = self.cpu.get_code(addr);
        let mut result = self.code_byte(p.mem, p.offset) as i32;
        if self.cpu.mmr[(addr >> PAGE_SHIFT) as usize] == 0xFF {
            result = self.cpu_read_(addr);
        }
        result
    }

    fn cpu_write(&mut self, addr: HesAddr, data: i32) {
        // check( addr <= 0xFFFF );
        let out = self.write_pages[(addr >> PAGE_SHIFT) as usize];
        let addr = addr & (PAGE_SIZE as u32 - 1);
        if let Some(p) = out {
            let i = p.offset + addr as usize;
            match p.mem {
                MEM_RAM => self.cpu.ram[i] = data as u8,
                _ => self.sgx[i] = data as u8,
            }
        } else if self.cpu.mmr[(addr >> PAGE_SHIFT) as usize] == 0xFF {
            // FIXME (upstream): the address is already masked to the page
            // here, so this checks the first page's mapping (normally the
            // I/O page) rather than the page written to
            self.cpu_write_(addr, data);
        }
    }

    fn cpu_write_vdp(&mut self, addr: i32, data: i32) {
        self.cpu_write_vdp_(addr, data);
    }

    fn cpu_set_mmr(&mut self, page: i32, bank: i32) -> CodePage {
        self.write_pages[page as usize] = None;
        if bank < 0x80 {
            return CodePage {
                mem: MEM_ROM,
                offset: self.rom.at_addr(bank * PAGE_SIZE as i32),
            };
        }

        let data = match bank {
            0xF8 => CodePage {
                mem: MEM_RAM,
                offset: 0,
            },

            0xF9..=0xFB => CodePage {
                mem: MEM_SGX,
                offset: (bank - 0xF9) as usize * PAGE_SIZE,
            },

            _ => {
                // if ( bank != 0xFF )
                //  debug_printf( "Unmapped bank $%02X\n", bank );
                return CodePage {
                    mem: MEM_ROM,
                    offset: self.rom.unmapped(),
                };
            }
        };

        self.write_pages[page as usize] = Some(data);
        data
    }

    fn cpu_done(&mut self) -> i32 {
        // check( time() >= end_time() ||
        //      (!(r.status & i_flag_mask) && time() >= irq_time()) );

        if self.cpu.r.status & I_FLAG_MASK == 0 {
            let present = self.time();

            if self.irq.timer <= present && self.irq.disables & TIMER_MASK == 0 {
                self.timer.fired = 1;
                self.irq.timer = FUTURE_HES_TIME;
                self.irq_changed(); // overkill, but not worth writing custom code
                                    // (#if GME_FRAME_HOOK_DEFINED: the frame hook)
                return 0x0A;
            }

            if self.irq.vdp <= present && self.irq.disables & VDP_MASK == 0 {
                // work around for bugs with music not acknowledging VDP
                //run_until( present );
                //irq.vdp = future_hes_time;
                //irq_changed();
                // (#if GME_FRAME_HOOK_DEFINED: the frame hook)
                return 0x08;
            }
        }
        0
    }
}

fn adjust_time(time: &mut i32, delta: HesTime) {
    if *time < FUTURE_HES_TIME {
        *time = time.wrapping_sub(delta);
        if *time < 0 {
            *time = 0;
        }
    }
}

impl ClassicEmu for HesEmu {
    fn classic(&self) -> &ClassicEmuState {
        &self.classic
    }
    fn classic_mut(&mut self) -> &mut ClassicEmuState {
        &mut self.classic
    }

    fn update_eq(&mut self, eq: &BlipEq) {
        self.apu.treble_eq(eq);
    }

    fn set_voice(
        &mut self,
        i: i32,
        center: Option<BufId>,
        left: Option<BufId>,
        right: Option<BufId>,
    ) {
        self.apu.osc_output(i, center, left, right);
    }

    fn run_clocks(&mut self, duration_: &mut BlipTime, _msec: i32) -> BlarggErr {
        let duration: BlipTime = *duration_; // cache

        if hes_cpu::run(self, duration) {
            self.state
                .set_warning("Emulation error (illegal instruction)");
        }

        // check( time() >= duration );
        //check( time() - duration < 20 ); // Txx instruction could cause going way over

        self.run_until(duration);

        // end time frame
        self.timer.last_time = self.timer.last_time.wrapping_sub(duration);
        self.vdp.next_vbl = self.vdp.next_vbl.wrapping_sub(duration);
        // (#if GME_FRAME_HOOK_DEFINED: last_frame_hook -= duration;)
        self.cpu.end_frame(duration);
        adjust_time(&mut self.irq.timer, duration);
        adjust_time(&mut self.irq.vdp, duration);
        let bufs = self.classic.bufs();
        self.apu.end_frame(bufs, duration);
        self.adpcm.end_frame(bufs, duration);

        Ok(())
    }
}

impl MusicEmu for HesEmu {
    classic_emu_overrides!();

    fn unload(&mut self) {
        self.rom.clear();
        music_emu_unload(self);
    }

    fn track_info_(&self, out: &mut TrackInfo, _track: i32) -> BlarggErr {
        copy_hes_fields(&self.rom.rom[self.rom.begin() + 0x20..], out);
        Ok(())
    }

    fn load_(&mut self, input: &mut dyn DataReader) -> BlarggErr {
        self.hes_load_(input)
    }

    fn start_track_(&mut self, track: i32) -> BlarggErr {
        self.hes_start_track_(track)
    }

    fn set_tempo_(&mut self, t: f64) {
        self.play_period = (PERIOD_60HZ as f64 / t) as HesTime;
        self.timer_base = (1024.0 / t) as i32;
        self.recalc_timer_load();
    }
}
