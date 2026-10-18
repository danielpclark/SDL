// SPC emulation support: init, sample buffering, reset, SPC loading

// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/

/* Copyright (C) 2004-2007 Shay Green. This module is free software; you
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

//! SNES SPC-700 APU emulator (`Snes_Spc.h`, `Snes_Spc.cpp`; the core,
//! `Spc_Cpu.cpp` and `Spc_Cpu.h`, is in `spc_cpu`).
//!
//! The DSP is the fast one (`Spc_Dsp.h` defines SPC_LESS_ACCURATE and
//! SPC_NO_COPY_STATE_FUNCS). Upstream's output pointer points into the
//! caller's buffer; the samples are made here in [`SnesSpc`]'s own buffer
//! and copied to the caller's when `play()` returns.

use crate::blargg_common::BlarggErr;
use crate::spc_dsp::{self, Sample, SpcDsp};

/// Sample pairs generated per second
pub const SAMPLE_RATE: i32 = 32000;

// Emulator use

/// Sets IPL ROM data. Library does not include ROM data. Most SPC music files
/// don't need ROM, but a full emulator must provide this.
pub const ROM_SIZE: usize = 0x40;

/// 1024000 SPC clocks per second, sample pair every 32 clocks
pub type TimeT = i32;
pub const CLOCK_RATE: i32 = 1024000;
pub const CLOCKS_PER_SAMPLE: i32 = 32;

/// Emulated port read/write at specified time
pub const PORT_COUNT: usize = 4;

/// Mutes voices corresponding to non-zero bits in mask (issues repeated KOFF events).
/// Reduces emulation accuracy.
pub const VOICE_COUNT: usize = 8;

/// Sets tempo, where tempo_unit = normal, tempo_unit / 2 = half speed, etc.
pub const TEMPO_UNIT: u32 = 0x100;

// SPC music files

/// Loads SPC data into emulator
pub const SPC_MIN_FILE_SIZE: i64 = 0x10180;
pub const SPC_FILE_SIZE: i64 = 0x10200;

/// TODO: document
#[derive(Clone, Copy, Debug, Default)]
pub struct Regs {
    pub pc: u16,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub psw: u8,
    pub sp: u8,
}

/// Time relative to m_spc_time. Speeds up code a bit by eliminating need to
/// constantly add m_spc_time to time from CPU. CPU uses time that ends at
/// 0 to eliminate reloading end time every instruction. It pays off.
pub type RelTime = i32;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Timer {
    pub(crate) next_time: RelTime, // time of next event
    pub(crate) prescaler: i32,
    pub(crate) period: i32,
    pub(crate) divider: i32,
    pub(crate) enabled: i32,
    pub(crate) counter: i32,
}

pub(crate) const REG_COUNT: usize = 0x10;
pub(crate) const TIMER_COUNT: usize = 3;
pub(crate) const EXTRA_SIZE: usize = spc_dsp::EXTRA_SIZE;

const SIGNATURE_SIZE: usize = 35;

pub(crate) const ROM_ADDR: usize = 0xFFC0;

pub(crate) const SKIPPING_TIME: i32 = 127;

/// Value that padding should be filled with
const CPU_PAD_FILL: u8 = 0xFF;

pub const R_TEST: usize = 0x0;
pub const R_CONTROL: usize = 0x1;
pub const R_DSPADDR: usize = 0x2;
pub const R_DSPDATA: usize = 0x3;
pub const R_CPUIO0: usize = 0x4;
pub const R_CPUIO1: usize = 0x5;
pub const R_CPUIO2: usize = 0x6;
pub const R_CPUIO3: usize = 0x7;
pub const R_F8: usize = 0x8;
pub const R_F9: usize = 0x9;
pub const R_T0TARGET: usize = 0xA;
pub const R_T1TARGET: usize = 0xB;
pub const R_T2TARGET: usize = 0xC;
pub const R_T0OUT: usize = 0xD;
pub const R_T1OUT: usize = 0xE;
pub const R_T2OUT: usize = 0xF;

/// The size of the RAM (`m.ram.ram`): the 64K and the padding after it
/// (upstream's 0x100 bytes, and 0x300 more, see [`SnesSpc::ram`]).
/// (Upstream also has 0x100 bytes of padding before it, which nothing
/// reads.)
pub(crate) const RAM_SIZE: usize = 0x10000 + 0x100 + 0x300;

// spc_file_t
mod spc_file {
    pub const SIGNATURE: usize = 0; // [signature_size]
    pub const PCL: usize = 37;
    pub const PCH: usize = 38;
    pub const A: usize = 39;
    pub const X: usize = 40;
    pub const Y: usize = 41;
    pub const PSW: usize = 42;
    pub const SP: usize = 43;
    pub const RAM: usize = 0x100; // [0x10000]
    pub const DSP: usize = 0x10100; // [128]
}

pub struct SnesSpc {
    pub(crate) dsp: SpcDsp,

    pub(crate) reg_times: [i8; 256],

    // state_t
    pub(crate) timers: [Timer; TIMER_COUNT],

    /// `smp_regs [0]` (REGS) and `smp_regs [1]` (REGS_IN)
    pub(crate) smp_regs: [[u8; REG_COUNT]; 2],

    pub(crate) cpu_regs: Regs,

    pub(crate) dsp_time: RelTime,
    pub(crate) spc_time: TimeT,
    pub(crate) echo_accessed: bool,

    pub(crate) tempo: i32,
    pub(crate) skipped_kon: i32,
    pub(crate) skipped_koff: i32,
    pub(crate) cpu_error: Option<&'static str>,

    pub(crate) extra_clocks: i32,
    /// (`buf_begin` set: whether the output buffer, [`SnesSpc::main`],
    /// is in use; `buf_end` is its size)
    pub(crate) buf_begin: bool,
    pub(crate) buf_end: usize,
    pub(crate) extra_pos: usize,
    pub(crate) extra_buf: [Sample; EXTRA_SIZE],

    pub(crate) rom_enabled: i32,
    pub(crate) rom: [u8; ROM_SIZE],
    pub(crate) hi_ram: [u8; ROM_SIZE],

    pub(crate) cycle_table: [u8; 256],

    /// The 64K RAM shared by the SPC-700 and the DSP, and the padding
    /// after it ("padding to neutralize address overflow -- but this is
    /// still undefined behavior!"). Note (upstream): the DSP reads the
    /// sample directory past even the padding (with the directory in the
    /// last page and a source number past 0xC0); this has 0x300 more bytes
    /// of padding, zeros, and upstream's build for the tests is patched
    /// the same.
    pub(crate) ram: Box<[u8]>,

    /// The output buffer (the caller's in upstream)
    pub(crate) main: Vec<Sample>,
}

impl std::fmt::Debug for SnesSpc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnesSpc").finish_non_exhaustive()
    }
}

impl Default for SnesSpc {
    fn default() -> Self {
        Self::new()
    }
}

// (n ? n : 256)
#[inline]
pub(crate) fn if_0_then_256(n: i32) -> i32 {
    (n - 1) as u8 as i32 + 1
}

static CYCLE_TABLE: [u8; 128] = [
    //   01   23   45   67   89   AB   CD   EF
    0x28, 0x47, 0x34, 0x36, 0x26, 0x54, 0x54, 0x68, // 0
    0x48, 0x47, 0x45, 0x56, 0x55, 0x65, 0x22, 0x46, // 1
    0x28, 0x47, 0x34, 0x36, 0x26, 0x54, 0x54, 0x74, // 2
    0x48, 0x47, 0x45, 0x56, 0x55, 0x65, 0x22, 0x38, // 3
    0x28, 0x47, 0x34, 0x36, 0x26, 0x44, 0x54, 0x66, // 4
    0x48, 0x47, 0x45, 0x56, 0x55, 0x45, 0x22, 0x43, // 5
    0x28, 0x47, 0x34, 0x36, 0x26, 0x44, 0x54, 0x75, // 6
    0x48, 0x47, 0x45, 0x56, 0x55, 0x55, 0x22, 0x36, // 7
    0x28, 0x47, 0x34, 0x36, 0x26, 0x54, 0x52, 0x45, // 8
    0x48, 0x47, 0x45, 0x56, 0x55, 0x55, 0x22, 0xC5, // 9
    0x38, 0x47, 0x34, 0x36, 0x26, 0x44, 0x52, 0x44, // A
    0x48, 0x47, 0x45, 0x56, 0x55, 0x55, 0x22, 0x34, // B
    0x38, 0x47, 0x45, 0x47, 0x25, 0x64, 0x52, 0x49, // C
    0x48, 0x47, 0x56, 0x67, 0x45, 0x55, 0x22, 0x83, // D
    0x28, 0x47, 0x34, 0x36, 0x24, 0x53, 0x43, 0x40, // E
    0x48, 0x47, 0x45, 0x56, 0x34, 0x54, 0x22, 0x60, // F
];

const SIGNATURE: &[u8; SIGNATURE_SIZE + 1] = b"SNES-SPC700 Sound File Data v0.30\x1A\x1A\0";

impl SnesSpc {
    pub fn new() -> Self {
        SnesSpc {
            dsp: SpcDsp::new(),
            reg_times: [0; 256],
            timers: [Timer::default(); TIMER_COUNT],
            smp_regs: [[0; REG_COUNT]; 2],
            cpu_regs: Regs::default(),
            dsp_time: 0,
            spc_time: 0,
            echo_accessed: false,
            tempo: 0,
            skipped_kon: 0,
            skipped_koff: 0,
            cpu_error: None,
            extra_clocks: 0,
            buf_begin: false,
            buf_end: 0,
            extra_pos: 0,
            extra_buf: [0; EXTRA_SIZE],
            rom_enabled: 0,
            rom: [0; ROM_SIZE],
            hi_ram: [0; ROM_SIZE],
            cycle_table: [0; 256],
            ram: Box::new([]),
            main: Vec::new(),
        }
    }

    // Init

    /// Must be called once before using
    pub fn init(&mut self) -> BlarggErr {
        // memset( &m, 0, sizeof m );
        let dsp = std::mem::take(&mut self.dsp);
        let main = std::mem::take(&mut self.main);
        let ram = std::mem::take(&mut self.ram);
        *self = SnesSpc {
            dsp,
            reg_times: self.reg_times,
            ..SnesSpc::new()
        };
        self.main = main;
        self.ram = ram;
        if self.ram.len() != RAM_SIZE {
            self.ram = crate::blargg_common::try_vec(RAM_SIZE)?.into_boxed_slice();
        } else {
            self.ram.fill(0);
        }
        self.dsp.init();

        self.tempo = TEMPO_UNIT as i32;

        // Most SPC music doesn't need ROM, and almost all the rest only rely
        // on these two bytes
        self.rom[0x3E] = 0xFF;
        self.rom[0x3F] = 0xC0;

        // unpack cycle table
        for i in 0..128 {
            let n = CYCLE_TABLE[i];
            self.cycle_table[i * 2] = n >> 4;
            self.cycle_table[i * 2 + 1] = n & 0x0F;
        }

        self.reg_times = crate::spc_cpu::REG_TIMES_;

        self.reset();
        Ok(())
    }

    pub fn init_rom(&mut self, input: &[u8; ROM_SIZE]) {
        self.rom.copy_from_slice(input);
    }

    pub fn set_tempo(&mut self, mut t: i32) {
        self.tempo = t;
        const TIMER2_SHIFT: i32 = 4; // 64 kHz
        const OTHER_SHIFT: i32 = 3; //  8 kHz

        // (#if SPC_DISABLE_TEMPO: the fixed prescalers)
        if t == 0 {
            t = 1;
        }
        const TIMER2_RATE: i32 = 1 << TIMER2_SHIFT;
        let mut rate = (TIMER2_RATE * TEMPO_UNIT as i32 + (t >> 1)) / t;
        if rate < TIMER2_RATE / 4 {
            rate = TIMER2_RATE / 4; // max 4x tempo
        }
        self.timers[2].prescaler = rate;
        self.timers[1].prescaler = rate << OTHER_SHIFT;
        self.timers[0].prescaler = rate << OTHER_SHIFT;
    }

    /// Timer registers have been loaded. Applies these to the timers. Does not
    /// reset timer prescalers or dividers.
    fn timers_loaded(&mut self) {
        for i in 0..TIMER_COUNT {
            let t = &mut self.timers[i];
            t.period = if_0_then_256(self.smp_regs[0][R_T0TARGET + i] as i32);
            t.enabled = (self.smp_regs[0][R_CONTROL] >> i & 1) as i32;
            t.counter = (self.smp_regs[1][R_T0OUT + i] & 0x0F) as i32;
        }

        let tempo = self.tempo;
        self.set_tempo(tempo);
    }

    /// Loads registers from unified 16-byte format
    fn load_regs(&mut self, input: &[u8]) {
        self.smp_regs[0].copy_from_slice(&input[..REG_COUNT]);
        self.smp_regs[1] = self.smp_regs[0];

        // These always read back as 0
        self.smp_regs[1][R_TEST] = 0;
        self.smp_regs[1][R_CONTROL] = 0;
        self.smp_regs[1][R_T0TARGET] = 0;
        self.smp_regs[1][R_T1TARGET] = 0;
        self.smp_regs[1][R_T2TARGET] = 0;
    }

    /// RAM was just loaded from SPC, with $F0-$FF containing SMP registers
    /// and timer counts. Copies these to proper registers.
    fn ram_loaded(&mut self) {
        self.rom_enabled = 0;
        let mut regs = [0u8; REG_COUNT];
        regs.copy_from_slice(&self.ram[0xF0..0x100]);
        self.load_regs(&regs);

        // Put STOP instruction around memory to catch PC underflow/overflow
        // (memset( m.ram.padding1, cpu_pad_fill, sizeof m.ram.padding1 ))
        self.ram[0x10000..0x10100].fill(CPU_PAD_FILL);
    }

    /// Registers were just loaded. Applies these new values.
    fn regs_loaded(&mut self) {
        let control = self.smp_regs[0][R_CONTROL] & 0x80;
        self.enable_rom(control as i32);
        self.timers_loaded();
    }

    fn reset_time_regs(&mut self) {
        self.cpu_error = None;
        self.echo_accessed = false;
        self.spc_time = 0;
        self.dsp_time = 0;
        // (#if SPC_LESS_ACCURATE)
        self.dsp_time = CLOCKS_PER_SAMPLE + 1;

        for t in self.timers.iter_mut() {
            t.next_time = 1;
            t.divider = 0;
        }

        self.regs_loaded();

        self.extra_clocks = 0;
        self.reset_buf();
    }

    fn reset_common(&mut self, timer_counter_init: i32) {
        for i in 0..TIMER_COUNT {
            self.smp_regs[1][R_T0OUT + i] = timer_counter_init as u8;
        }

        // Run IPL ROM
        self.cpu_regs = Regs::default();
        self.cpu_regs.pc = ROM_ADDR as u16;

        self.smp_regs[0][R_TEST] = 0x0A;
        self.smp_regs[0][R_CONTROL] = 0xB0; // ROM enabled, clear ports
        for i in 0..PORT_COUNT {
            self.smp_regs[1][R_CPUIO0 + i] = 0;
        }

        self.reset_time_regs();
    }

    /// Emulates pressing reset switch on SNES. This resets your output buffer, so
    /// you must call set_output() after this.
    pub fn soft_reset(&mut self) {
        self.reset_common(0);
        self.dsp.soft_reset();
    }

    /// Resets SPC to power-on state. This resets your output buffer, so you must
    /// call set_output() after this.
    pub fn reset(&mut self) {
        self.ram[..0x10000].fill(0xFF);
        self.ram_loaded();
        self.reset_common(0x0F);
        self.dsp.reset();
    }

    pub fn load_spc(&mut self, data: &[u8]) -> BlarggErr {
        let spc = data;
        let size = data.len();

        // be sure compiler didn't insert any padding into fle_t
        // blaarg_static_assert( sizeof (spc_file_t) == spc_min_file_size + 0x80, ... );

        // Check signature and file size
        if size < SIGNATURE_SIZE || spc[spc_file::SIGNATURE..27] != SIGNATURE[..27] {
            return Err("Not an SPC file");
        }

        if (size as i64) < SPC_MIN_FILE_SIZE {
            return Err("Corrupt SPC file");
        }

        // CPU registers
        self.cpu_regs.pc = spc[spc_file::PCH] as u16 * 0x100 + spc[spc_file::PCL] as u16;
        self.cpu_regs.a = spc[spc_file::A];
        self.cpu_regs.x = spc[spc_file::X];
        self.cpu_regs.y = spc[spc_file::Y];
        self.cpu_regs.psw = spc[spc_file::PSW];
        self.cpu_regs.sp = spc[spc_file::SP];

        // RAM and registers
        self.ram[..0x10000].copy_from_slice(&spc[spc_file::RAM..spc_file::RAM + 0x10000]);
        self.ram_loaded();

        // DSP registers
        self.dsp
            .load(&spc[spc_file::DSP..spc_file::DSP + spc_dsp::REGISTER_COUNT]);

        self.reset_time_regs();

        Ok(())
    }

    /// Clears echo region. Useful after loading an SPC as many have garbage in echo.
    pub fn clear_echo(&mut self) {
        // Allows playback of dodgy Super Mario World mod SPCs
        // (#ifndef SPC_ISOLATED_ECHO_BUFFER)
        if self.dsp.read(spc_dsp::R_FLG) & 0x20 == 0 {
            let addr = 0x100 * self.dsp.read(spc_dsp::R_ESA);
            let mut end = addr + 0x800 * (self.dsp.read(spc_dsp::R_EDL) & 0x0F);
            if end > 0x10000 {
                end = 0x10000;
            }
            self.ram[addr as usize..end as usize].fill(0xFF);
        }
    }

    // Sample output

    fn reset_buf(&mut self) {
        // Start with half extra buffer of silence
        let mut out = 0;
        while out < EXTRA_SIZE / 2 {
            self.extra_buf[out] = 0;
            out += 1;
        }

        self.extra_pos = out;
        self.buf_begin = false;

        self.dsp.set_output(None);
    }

    /// Sets destination for output samples (`Some(size)`: `main`,
    /// resized to it)
    pub fn set_output(&mut self, out: Option<usize>) {
        // require( (size & 1) == 0 ); // size must be even

        self.extra_clocks &= CLOCKS_PER_SAMPLE - 1;
        if let Some(size) = out {
            self.main.clear();
            self.main.resize(size, 0);
            let out_end = size;
            self.buf_begin = true;
            self.buf_end = out_end;

            // Copy extra to output
            let mut out = 0;
            let mut input = 0;
            while input < self.extra_pos && out < out_end {
                self.main[out] = self.extra_buf[input];
                out += 1;
                input += 1;
            }

            // Handle output being full already
            if out >= out_end {
                // Have DSP write to remaining extra space
                let mut out = 0;

                // Copy any remaining extra samples as if DSP wrote them
                while input < self.extra_pos {
                    self.dsp.extra[out] = self.extra_buf[input];
                    out += 1;
                    input += 1;
                }
                // assert( out <= out_end );

                self.dsp.set_output_extra(out, EXTRA_SIZE - out);
            } else {
                self.dsp.set_output(Some((out, out_end - out)));
            }
        } else {
            self.reset_buf();
        }
    }

    /// Number of samples written to output since last set
    pub fn sample_count(&self) -> i32 {
        (self.extra_clocks >> 5) * 2
    }

    pub(crate) fn save_extra(&mut self) {
        // Get end pointers
        let mut main_end = self.buf_end; // end of data written to buf
        let dsp_out = self.dsp.out_pos(); // end of data written to dsp.extra()
        let mut dsp_end = dsp_out.pos;
        if !dsp_out.extra && dsp_end <= main_end {
            main_end = dsp_end;
            dsp_end = 0; // nothing in DSP's extra
        }

        // Copy any extra samples at these ends into extra_buf
        let mut out = 0;
        let mut input = self.sample_count() as usize;
        while input < main_end {
            self.extra_buf[out] = self.main[input];
            out += 1;
            input += 1;
        }
        for input in 0..dsp_end {
            self.extra_buf[out] = self.dsp.extra[input];
            out += 1;
        }

        self.extra_pos = out;
        // assert( out <= &m.extra_buf [extra_size] );
    }

    /// Plays for count samples and write samples to out. Discards samples if out
    /// is NULL. Count must be a multiple of 2 since output is stereo.
    pub fn play(&mut self, count: i32, out: Option<&mut [Sample]>) -> BlarggErr {
        // require( (count & 1) == 0 ); // must be even
        if count != 0 {
            let has_out = out.is_some();
            self.set_output(if has_out { Some(count as usize) } else { None });
            self.end_frame(count * (CLOCKS_PER_SAMPLE / 2));
            if let Some(out) = out {
                out[..count as usize].copy_from_slice(&self.main[..count as usize]);
            }
        }

        let err = self.cpu_error.take();
        match err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Skips count samples. Several times faster than play() when using fast DSP.
    pub fn skip(&mut self, mut count: i32) -> BlarggErr {
        // (#if SPC_LESS_ACCURATE)
        if count > 2 * SAMPLE_RATE * 2 {
            self.set_output(None);

            // Skip a multiple of 4 samples
            let mut end: TimeT = count;
            count = (count & 3) + SAMPLE_RATE * 2;
            end = (end - count) * (CLOCKS_PER_SAMPLE / 2);

            self.skipped_kon = 0;
            self.skipped_koff = 0;

            // Preserve DSP and timer synchronization
            // TODO: verify that this really preserves it
            let old_dsp_time = self.dsp_time + self.spc_time;
            self.dsp_time = end - self.spc_time + SKIPPING_TIME;
            self.end_frame(end);
            self.dsp_time = self.dsp_time - SKIPPING_TIME + old_dsp_time;

            let koff = self.skipped_koff & !self.skipped_kon;
            self.dsp.write(spc_dsp::R_KOFF, koff);
            let kon = self.skipped_kon;
            self.dsp.write(spc_dsp::R_KON, kon);
            self.clear_echo();
        }

        self.play(count, None)
    }

    // Sound control

    pub fn mute_voices(&mut self, mask: i32) {
        self.dsp.mute_voices(mask);
    }

    /// If true, prevents channels and global volumes from being phase-negated.
    /// Only supported by fast DSP.
    pub fn disable_surround(&mut self, disable: bool) {
        self.dsp.disable_surround(disable);
    }

    pub fn disable_echo(&mut self, disable: bool) {
        self.dsp.disable_echo(disable);
    }

    pub fn smp_regs(&mut self) -> &mut Regs {
        &mut self.cpu_regs
    }

    pub fn smp_ram(&mut self) -> &mut [u8] {
        &mut self.ram[..0x10000]
    }

    pub fn run_until(&mut self, t: TimeT) {
        self.run_until_(t);
    }

    pub fn read_port(&mut self, t: TimeT, port: usize) -> i32 {
        // assert( (unsigned) port < port_count );
        self.run_until_(t);
        self.smp_regs[0][R_CPUIO0 + port] as i32
    }

    pub fn write_port(&mut self, t: TimeT, port: usize, data: i32) {
        // assert( (unsigned) port < port_count );
        self.run_until_(t);
        // (run_until_( t ) [0x10 + port]: REGS_IN's port)
        self.smp_regs[1][R_CPUIO0 + port] = data as u8;
    }
}
