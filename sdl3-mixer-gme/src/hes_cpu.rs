// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/

//#include "hes_cpu_log.h"

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

//! PC Engine CPU emulator for use with HES music files (`Hes_Cpu.h`,
//! `Hes_Cpu.cpp`).
//!
//! The code map holds [`CodePage`]s, places in the memory regions of the
//! emulator using the CPU (upstream holds pointers), and [`run`] calls the
//! emulator's memory handlers (`hes_cpu_io.h`) through [`HesCpuHost`].
//! Upstream runs on a local copy of its state, which the memory handlers
//! see (through its `state` pointer) and which it copies back when it
//! stops; this runs on the state itself.
//!
//! Upstream's registers in `run()` are `uint_fast16_t` and `uint_fast8_t`,
//! 64 and 8 bits wide in the build this matches (glibc's); they are `u64`
//! and `u8` here.

pub type HesTime = i32; // clock cycle count
pub type HesAddr = u32; // 16-bit address
pub const FUTURE_HES_TIME: HesTime = i32::MAX / 2 + 1;

pub const PAGE_SIZE: usize = 0x2000;
pub const PAGE_SHIFT: u32 = 13;
pub const PAGE_COUNT: usize = 8;

/// Attempt to execute instruction here results in CPU advancing time to
/// lesser of irq_time() and end_time() (or end_time() if IRQs are
/// disabled)
pub const IDLE_ADDR: u64 = 0x1FFF;

/// Can read this many bytes past end of a page
pub const CPU_PADDING: usize = 8;

pub const IRQ_INHIBIT: u8 = 0x04;

/// A place in one of the emulator's memory regions: the region and the
/// offset in it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CodePage {
    pub mem: u8,
    pub offset: usize,
}

/// The 8KB of RAM (`ram`), as a memory region.
pub const MEM_RAM: u8 = 0;

/// not kept updated during a call to run()
#[derive(Clone, Copy, Debug, Default)]
pub struct Registers {
    pub pc: u16,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub status: u8,
    pub sp: u8,
}

#[derive(Clone, Debug)]
pub struct HesCpu {
    /// Note (upstream): an instruction at the end of the RAM page has its
    /// operands read past it (upstream reads the registers after it); the
    /// eight bytes after it are kept zero here, and upstream's build for
    /// the tests has the same padding.
    pub ram: [u8; PAGE_SIZE + CPU_PADDING],

    pub r: Registers,

    /// page mapping registers
    pub mmr: [u8; PAGE_COUNT + 1],

    // state_t
    code_map: [CodePage; PAGE_COUNT + 1],
    base: HesTime,
    time: i32,
    irq_time_: HesTime,
    end_time_: HesTime,
}

impl Default for HesCpu {
    fn default() -> Self {
        HesCpu {
            ram: [0; PAGE_SIZE + CPU_PADDING],
            r: Registers::default(),
            mmr: [0; PAGE_COUNT + 1],
            code_map: [CodePage::default(); PAGE_COUNT + 1],
            base: 0,
            time: 0,
            irq_time_: 0,
            end_time_: 0,
        }
    }
}

/// The memory handlers of the emulator running the CPU.
pub trait HesCpuHost {
    fn cpu(&mut self) -> &mut HesCpu;
    /// The byte at `offset` in memory region `mem`.
    fn code_byte(&self, mem: u8, offset: usize) -> u8;
    /// `CPU_READ()` (and `CPU_READ_FAST()`, which reads the same)
    fn cpu_read(&mut self, addr: HesAddr) -> i32;
    /// `CPU_WRITE()` (and `CPU_WRITE_FAST()`, which writes the same)
    fn cpu_write(&mut self, addr: HesAddr, data: i32);
    /// `CPU_WRITE_VDP()`
    fn cpu_write_vdp(&mut self, addr: i32, data: i32);
    /// `CPU_SET_MMR()`: the page the bank maps
    fn cpu_set_mmr(&mut self, page: i32, bank: i32) -> CodePage;
    /// `CPU_DONE()`
    fn cpu_done(&mut self) -> i32;
}

// status flags
const ST_N: u8 = 0x80;
const ST_V: u8 = 0x40;
#[allow(dead_code)]
const ST_T: u8 = 0x20;
const ST_B: u8 = 0x10;
const ST_D: u8 = 0x08;
const ST_I: u8 = 0x04;
const ST_Z: u8 = 0x02;
const ST_C: u8 = 0x01;

// all zero-page should really use whatever is at page 1, but that would
// reduce efficiency quite a bit
const RAM_ADDR: u64 = 0x2000;

/// The code map entry for an address. Note (upstream): the program
/// counter isn't kept to 16 bits while running (only when the CPU stops),
/// so code running on past 0xFFFF (through the unmapped page after it) goes
/// on to index past the code map; this maps the addresses past it to that
/// unmapped page too (upstream's build for the tests is patched the same).
#[inline]
fn page_index(addr: u64) -> usize {
    ((addr >> PAGE_SHIFT) as usize).min(PAGE_COUNT)
}

impl HesCpu {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        // check( state == &state_ );
        // state = &state_;

        self.time = 0;
        self.base = 0;
        self.irq_time_ = FUTURE_HES_TIME;
        self.end_time_ = FUTURE_HES_TIME;

        self.r.status = ST_I;
        self.r.sp = 0;
        self.r.pc = 0;
        self.r.a = 0;
        self.r.x = 0;
        self.r.y = 0;
    }

    /// `get_code()`: the region and offset of the byte at `addr`
    pub fn get_code(&self, addr: HesAddr) -> CodePage {
        let p = self.code_map[(addr >> PAGE_SHIFT) as usize];
        CodePage {
            mem: p.mem,
            offset: p.offset + (addr % PAGE_SIZE as u32) as usize,
        }
    }

    /// Time of beginning of next instruction to be executed
    pub fn time(&self) -> HesTime {
        self.time.wrapping_add(self.base)
    }
    pub fn set_time(&mut self, t: HesTime) {
        self.time = t.wrapping_sub(self.base);
    }
    pub fn adjust_time(&mut self, delta: i32) {
        self.time = self.time.wrapping_add(delta);
    }

    pub fn irq_time(&self) -> HesTime {
        self.irq_time_
    }
    pub fn set_irq_time(&mut self, t: HesTime) {
        self.irq_time_ = t;
        let d = self.update_end_time(self.end_time_, t);
        self.time = self.time.wrapping_add(d);
    }

    pub fn end_time(&self) -> HesTime {
        self.end_time_
    }
    pub fn set_end_time(&mut self, t: HesTime) {
        self.end_time_ = t;
        let d = self.update_end_time(t, self.irq_time_);
        self.time = self.time.wrapping_add(d);
    }

    pub fn end_frame(&mut self, t: HesTime) {
        // assert( state == &state_ );
        self.base = self.base.wrapping_sub(t);
        if self.irq_time_ < FUTURE_HES_TIME {
            self.irq_time_ = self.irq_time_.wrapping_sub(t);
        }
        if self.end_time_ < FUTURE_HES_TIME {
            self.end_time_ = self.end_time_.wrapping_sub(t);
        }
    }

    #[inline]
    fn update_end_time(&mut self, mut t: HesTime, irq: HesTime) -> i32 {
        if irq < t && self.r.status & IRQ_INHIBIT == 0 {
            t = irq;
        }
        let delta = self.base.wrapping_sub(t);
        self.base = t;
        delta
    }
}

pub fn set_mmr<H: HesCpuHost + ?Sized>(h: &mut H, reg: i32, bank: i32) {
    // assert( (unsigned) reg <= page_count ); // allow page past end to be set
    // assert( (unsigned) bank < 0x100 );
    h.cpu().mmr[reg as usize] = bank as u8;
    let code = h.cpu_set_mmr(reg, bank);
    // (minus PAGE_OFFSET( reg << page_shift ), which is 0)
    h.cpu().code_map[reg as usize] = code;
}

// TODO: each reference lists slightly different timing values, ugh
static CLOCK_TABLE: [u8; 256] = [
    // 0 1 2 3 4 5 6 7 8 9 A B C D E F
    1, 7, 3, 4, 6, 4, 6, 7, 3, 2, 2, 2, 7, 5, 7, 6, // 0
    4, 7, 7, 4, 6, 4, 6, 7, 2, 5, 2, 2, 7, 5, 7, 6, // 1
    7, 7, 3, 4, 4, 4, 6, 7, 4, 2, 2, 2, 5, 5, 7, 6, // 2
    4, 7, 7, 2, 4, 4, 6, 7, 2, 5, 2, 2, 5, 5, 7, 6, // 3
    7, 7, 3, 4, 8, 4, 6, 7, 3, 2, 2, 2, 4, 5, 7, 6, // 4
    4, 7, 7, 5, 2, 4, 6, 7, 2, 5, 3, 2, 2, 5, 7, 6, // 5
    7, 7, 2, 2, 4, 4, 6, 7, 4, 2, 2, 2, 7, 5, 7, 6, // 6
    4, 7, 7, 17, 4, 4, 6, 7, 2, 5, 4, 2, 7, 5, 7, 6, // 7
    4, 7, 2, 7, 4, 4, 4, 7, 2, 2, 2, 2, 5, 5, 5, 6, // 8
    4, 7, 7, 8, 4, 4, 4, 7, 2, 5, 2, 2, 5, 5, 5, 6, // 9
    2, 7, 2, 7, 4, 4, 4, 7, 2, 2, 2, 2, 5, 5, 5, 6, // A
    4, 7, 7, 8, 4, 4, 4, 7, 2, 5, 2, 2, 5, 5, 5, 6, // B
    2, 7, 2, 17, 4, 4, 6, 7, 2, 2, 2, 2, 5, 5, 7, 6, // C
    4, 7, 7, 17, 2, 4, 6, 7, 2, 5, 3, 2, 2, 5, 7, 6, // D
    2, 7, 2, 17, 4, 4, 6, 7, 2, 2, 2, 2, 5, 5, 7, 6, // E
    4, 7, 7, 17, 2, 4, 6, 7, 2, 5, 4, 2, 2, 5, 7, 6, // F
]; // 0x00 was 8

/// Where an instruction continues, for upstream's `goto`s.
#[derive(Clone, Copy)]
enum Next {
    Loop,
    BranchNotTaken,
    Brk,
    OutOfTime,
    Cli,
    Sei,
}

/// Set end_time and run CPU from current time. Returns true if any illegal
/// instructions were encountered.
pub fn run<H: HesCpuHost + ?Sized>(h: &mut H, end_time: HesTime) -> bool {
    let mut illegal_encountered = false;
    h.cpu().set_end_time(end_time);
    // even on x86, using s.time in place of s_time was slower
    let mut s_time: i32 = h.cpu().time;

    // registers
    let r = h.cpu().r;
    let mut pc: u64 = r.pc as u64;
    let mut a: u8 = r.a;
    let mut x: u8 = r.x;
    let mut y: u8 = r.y;
    let mut sp: u64;

    macro_rules! set_sp {
        ($v:expr) => {
            sp = (($v) as u64 + 1) | 0x100
        };
    }
    macro_rules! get_sp {
        () => {
            (sp.wrapping_sub(1) & 0xFF) as u8
        };
    }
    set_sp!(r.sp);

    let mut status: u8;
    let mut c: u64; // carry set if (c & 0x100) != 0
    let mut nz: u64; // Z set if (nz & 0xFF) == 0, N set if (nz & 0x8080) != 0

    macro_rules! is_neg {
        () => {
            nz & 0x8080 != 0
        };
    }
    macro_rules! calc_status {
        () => {{
            let mut out = status & (ST_V | ST_D | ST_I);
            out |= (((nz >> 8) | nz) as u8) & ST_N;
            out |= (c >> 8) as u8 & ST_C;
            if nz & 0xFF == 0 {
                out |= ST_Z;
            }
            out
        }};
    }
    macro_rules! set_status {
        ($in:expr) => {{
            let in_: u8 = $in;
            status = in_ & (ST_V | ST_D | ST_I);
            nz = (in_ as u64) << 8;
            c = nz;
            nz |= (!in_ & ST_Z) as u64;
        }};
    }
    macro_rules! flush_time {
        () => {
            h.cpu().time = s_time
        };
    }
    macro_rules! cache_time {
        () => {
            s_time = h.cpu().time
        };
    }
    macro_rules! read {
        ($addr:expr) => {
            h.cpu_read(($addr) as HesAddr) as i64 as u64
        };
    }
    macro_rules! write {
        ($addr:expr, $data:expr) => {
            h.cpu_write(($addr) as HesAddr, ($data) as i32)
        };
    }
    // (CPU_READ_FAST() reads the code byte, then the I/O page's register
    // with the time flushed, as cpu_read() reads; flushing and caching the
    // time around a read that doesn't change it changes nothing)
    macro_rules! read_fast {
        ($addr:expr) => {{
            flush_time!();
            let v = read!($addr);
            cache_time!();
            v
        }};
    }
    // (and CPU_WRITE_FAST() writes as cpu_write() does)
    macro_rules! write_fast {
        ($addr:expr, $data:expr) => {{
            flush_time!();
            write!($addr, $data);
            cache_time!();
        }};
    }
    macro_rules! read_low {
        ($addr:expr) => {{
            let i = ($addr) as i32 as usize;
            h.cpu().ram[i]
        }};
    }
    macro_rules! write_low {
        ($addr:expr, $data:expr) => {{
            let i = ($addr) as i32 as usize;
            let d = ($data) as u8;
            h.cpu().ram[i] = d;
        }};
    }
    macro_rules! read_prog_le16 {
        ($addr:expr) => {{
            let addr: u64 = $addr;
            let p = h.cpu().code_map[page_index(addr)];
            let off = p.offset + (addr & (PAGE_SIZE as u64 - 1)) as usize;
            h.code_byte(p.mem, off) as u64 | (h.code_byte(p.mem, off + 1) as u64) << 8
        }};
    }
    macro_rules! push {
        ($v:expr) => {{
            sp = sp.wrapping_sub(1) | 0x100;
            write_low!(sp, $v);
        }};
    }
    macro_rules! pop {
        () => {{
            let v = read_low!(sp);
            sp = sp.wrapping_sub(0xFF) | 0x100;
            v
        }};
    }
    macro_rules! interrupt {
        ($result:expr) => {{
            let result_: usize = $result;
            s_time = s_time.wrapping_add(7);

            write_low!(0x100 | sp.wrapping_sub(1), pc >> 8);
            write_low!(0x100 | sp.wrapping_sub(2), pc);
            pc = {
                let p = h.cpu().code_map[page_index(0xFFF0)];
                let off = p.offset + (0xFFF0 & (PAGE_SIZE - 1)) + result_;
                h.code_byte(p.mem, off) as u64 | (h.code_byte(p.mem, off + 1) as u64) << 8
            };

            sp = sp.wrapping_sub(3) | 0x100;
            let mut temp = calc_status!();
            if result_ == 6 {
                temp |= ST_B;
            }
            write_low!(sp, temp);

            status &= !ST_D;
            status |= ST_I;
            let cpu = h.cpu();
            cpu.r.status = status; // update externally-visible I flag

            let delta = cpu.base.wrapping_sub(cpu.end_time_);
            cpu.base = cpu.end_time_;
            s_time = s_time.wrapping_add(delta);
        }};
    }

    {
        let temp = r.status;
        set_status!(temp);
    }

    let mut next = Next::Loop;
    'stop: loop {
        match next {
            Next::Loop => {}
            Next::BranchNotTaken => {
                s_time = s_time.wrapping_sub(2);
            }
            Next::Brk => {
                // handle_brk:
                pc = pc.wrapping_add(1);
                interrupt!(6);
            }
            Next::OutOfTime => {
                // out_of_time:
                pc = pc.wrapping_sub(1);
                flush_time!();
                let result_ = h.cpu_done();
                cache_time!();
                if result_ > 0 {
                    interrupt!(result_ as usize);
                } else if s_time >= 0 {
                    break 'stop;
                }
            }
            Next::Cli => {
                // handle_cli:
                let cpu = h.cpu();
                cpu.r.status = status; // update externally-visible I flag
                let delta = cpu.base.wrapping_sub(cpu.irq_time_);
                if delta <= 0 {
                    // if ( TIME < irq_time_ ) goto loop;
                    // goto delayed_cli;
                } else {
                    cpu.base = cpu.irq_time_;
                    s_time = s_time.wrapping_add(delta);
                    if s_time >= 0 && delta >= s_time.wrapping_add(1) {
                        // delayed irq until after next instruction
                        cpu.base = cpu.base.wrapping_add(s_time.wrapping_add(1));
                        s_time = -1;
                        cpu.irq_time_ = cpu.base; // TODO: remove, as only to satisfy debug check in loop
                    }
                }
                // (delayed_cli:)
                // debug_printf( "Delayed CLI not supported\n" ); // TODO: implement
            }
            Next::Sei => {
                // handle_sei:
                let cpu = h.cpu();
                cpu.r.status = status; // update externally-visible I flag
                let delta = cpu.base.wrapping_sub(cpu.end_time_);
                cpu.base = cpu.end_time_;
                s_time = s_time.wrapping_add(delta);
                // if ( s_time < 0 ) goto loop;
                // debug_printf( "Delayed SEI not supported\n" ); // TODO: implement
            }
        }
        next = Next::Loop;

        // loop:

        // check( (unsigned) GET_SP() < 0x100 );

        let page = h.cpu().code_map[page_index(pc)];

        // TODO: eliminate this special case
        let instr = page.offset + (pc & (PAGE_SIZE as u64 - 1)) as usize;
        let opcode: u8 = h.code_byte(page.mem, instr);
        let instr = instr + 1;
        pc = pc.wrapping_add(1);

        let mut data: u64 = CLOCK_TABLE[opcode as usize] as u64;
        s_time = s_time.wrapping_add(data as i32);
        if s_time >= 0 {
            // possibly_out_of_time:
            if s_time >= data as i32 {
                s_time = s_time.wrapping_sub(data as i32);
                next = Next::OutOfTime;
                continue 'stop;
            }
        }
        // almost_out_of_time:

        data = h.code_byte(page.mem, instr) as u64;

        // Macros

        macro_rules! instr_byte {
            ($i:expr) => {
                h.code_byte(page.mem, instr + $i) as u64
            };
        }
        macro_rules! get_msb {
            () => {
                instr_byte!(1)
            };
        }
        macro_rules! add_page {
            () => {{
                pc = pc.wrapping_add(1);
                data.wrapping_add(0x100u64.wrapping_mul(get_msb!()))
            }};
        }
        macro_rules! get_addr {
            () => {
                instr_byte!(0) | instr_byte!(1) << 8
            };
        }

        // TODO: is the penalty really always added? the original 6502 was much better
        //#define PAGE_CROSS_PENALTY( lsb ) (void) (s_time += (lsb) >> 8)

        // Branch

        // TODO: more efficient way to handle negative branch that wraps PC around
        macro_rules! branch {
            ($cond:expr) => {{
                let offset = data as u8 as i8 as i64;
                pc = pc.wrapping_add(1);
                if !($cond) {
                    next = Next::BranchNotTaken;
                } else {
                    pc = pc.wrapping_add(offset as u64) as u16 as u64;
                }
            }};
        }

        // ARITH_ADDR_MODES( op ): the operand of the arithmetic
        // instructions (CMP, AND, EOR, ORA, SBC, ADC), by addressing mode
        let hi = opcode & 0xE0;
        let lo = opcode & 0x1F;
        if matches!(hi, 0x00 | 0x20 | 0x40 | 0x60 | 0xC0 | 0xE0)
            && matches!(
                lo,
                0x01 | 0x12 | 0x11 | 0x15 | 0x05 | 0x19 | 0x1D | 0x0D | 0x09
            )
        {
            match lo {
                0x01 | 0x12 => {
                    if lo == 0x01 {
                        // (ind,x)
                        data = data.wrapping_add(x as u64) as u8 as u64;
                    }
                    // (ind)
                    data = 0x100u64
                        .wrapping_mul(read_low!(data.wrapping_add(1) as u8) as u64)
                        .wrapping_add(read_low!(data) as u64);
                    // ptr:
                    flush_time!();
                    data = read!(data);
                    cache_time!();
                }
                0x11 => {
                    // (ind),y
                    let temp = (read_low!(data) as u64).wrapping_add(y as u64);
                    data = temp.wrapping_add(
                        0x100u64.wrapping_mul(read_low!(data.wrapping_add(1) as u8) as u64),
                    );
                    // ptr:
                    flush_time!();
                    data = read!(data);
                    cache_time!();
                }
                0x15 | 0x05 => {
                    if lo == 0x15 {
                        // zp,X
                        data = data.wrapping_add(x as u64) as u8 as u64;
                    }
                    // zp
                    data = read_low!(data) as u64;
                }
                0x19 | 0x1D | 0x0D => {
                    if lo == 0x19 {
                        // abs,Y
                        data = data.wrapping_add(y as u64);
                    } else if lo == 0x1D {
                        // abs,X
                        data = data.wrapping_add(x as u64);
                    }
                    // abs
                    data = add_page!();
                    // ptr:
                    flush_time!();
                    data = read!(data);
                    cache_time!();
                }
                _ => {
                    // imm
                }
            }
            match hi {
                0xC0 => {
                    // CMP
                    nz = (a as u64).wrapping_sub(data);
                    pc = pc.wrapping_add(1);
                    c = !nz;
                    nz &= 0xFF;
                }
                0x20 => {
                    // AND
                    a &= data as u8;
                    nz = a as u64;
                    pc = pc.wrapping_add(1);
                }
                0x40 => {
                    // EOR
                    a ^= data as u8;
                    nz = a as u64;
                    pc = pc.wrapping_add(1);
                }
                0x00 => {
                    // ORA
                    a |= data as u8;
                    nz = a as u64;
                    pc = pc.wrapping_add(1);
                }
                _ => {
                    // SBC, ADC
                    if hi == 0xE0 {
                        data ^= 0xFF;
                    }
                    // adc_imm:
                    // if ( status & st_d )
                    //  debug_printf( "Decimal mode not supported\n" );
                    let carry = (c >> 8 & 1) as i64;
                    let ov = ((a ^ 0x80) as i64) + carry + (data as u8 as i8 as i64); // sign-extend
                    status &= !ST_V;
                    status |= (ov >> 2 & 0x40) as u8;
                    nz = (a as u64).wrapping_add(data).wrapping_add(carry as u64);
                    c = nz;
                    pc = pc.wrapping_add(1);
                    a = nz as u8;
                }
            }
            continue 'stop;
        }

        match opcode {
            0xF0 => branch!(nz as u8 == 0), // BEQ

            0xD0 => branch!(nz as u8 != 0), // BNE

            0x10 => branch!(!is_neg!()), // BPL

            0x90 => branch!(c & 0x100 == 0), // BCC

            0x30 => branch!(is_neg!()), // BMI

            0x50 => branch!(status & ST_V == 0), // BVC

            0x70 => branch!(status & ST_V != 0), // BVS

            0xB0 => branch!(c & 0x100 != 0), // BCS

            0x80 => branch!(true), // BRA

            0xFF if pc == IDLE_ADDR + 1 => {
                // idle_done:
                s_time = 0;
                next = Next::OutOfTime;
            }
            0x0F | 0x1F | 0x2F | 0x3F | 0x4F | 0x5F | 0x6F | 0x7F // BBRn
            | 0x8F | 0x9F | 0xAF | 0xBF | 0xCF | 0xDF | 0xEF | 0xFF => {
                // BBSn
                let mut t: u64 = 0x101 * read_low!(data) as u64;
                t ^= 0xFF;
                pc = pc.wrapping_add(1);
                data = get_msb!();
                branch!(t & (1 << (opcode >> 4)) != 0)
            }

            0x4C => {
                // JMP abs
                pc = get_addr!();
            }

            0x7C | 0x6C => {
                if opcode == 0x7C {
                    // JMP (ind+X)
                    data = data.wrapping_add(x as u64);
                }
                // JMP (ind)
                data = data.wrapping_add(0x100 * get_msb!());
                pc = read_prog_le16!(data);
            }

            // Subroutine
            0x44 => {
                // BSR
                write_low!(0x100 | sp.wrapping_sub(1), pc >> 8);
                sp = sp.wrapping_sub(2) | 0x100;
                write_low!(sp, pc);
                // branch_taken:
                branch!(true)
            }

            0x20 => {
                // JSR
                let temp = pc.wrapping_add(1);
                pc = get_addr!();
                write_low!(0x100 | sp.wrapping_sub(1), temp >> 8);
                sp = sp.wrapping_sub(2) | 0x100;
                write_low!(sp, temp);
            }

            0x60 => {
                // RTS
                pc = 0x100 * read_low!(0x100 | sp.wrapping_sub(0xFF)) as u64;
                pc = pc.wrapping_add(1 + read_low!(sp) as u64);
                sp = sp.wrapping_sub(0xFE) | 0x100;
            }

            0x00 => {
                // BRK
                next = Next::Brk;
            }

            // Common
            0xBD => {
                // LDA abs,X
                let addr = get_addr!().wrapping_add(x as u64);
                pc = pc.wrapping_add(2);
                nz = read_fast!(addr);
                a = nz as u8;
            }

            0x9D => {
                // STA abs,X
                let addr = get_addr!().wrapping_add(x as u64);
                pc = pc.wrapping_add(2);
                write_fast!(addr, a);
            }

            0x95 | 0x85 => {
                if opcode == 0x95 {
                    // STA zp,x
                    data = data.wrapping_add(x as u64) as u8 as u64;
                }
                // STA zp
                pc = pc.wrapping_add(1);
                write_low!(data, a);
            }

            0xAE => {
                // LDX abs
                let addr = get_addr!();
                pc = pc.wrapping_add(2);
                nz = read_fast!(addr);
                x = nz as u8;
            }

            0xA5 => {
                // LDA zp
                nz = read_low!(data) as u64;
                a = nz as u8;
                pc = pc.wrapping_add(1);
            }

            // Load/store
            0x91 | 0x81 | 0x92 | 0x99 | 0x8D => {
                let addr: u64;
                match opcode {
                    0x91 => {
                        // STA (ind),Y
                        let hi = 0x100 * read_low!(data.wrapping_add(1) as u8) as u64;
                        addr = hi
                            .wrapping_add(read_low!(data) as u64)
                            .wrapping_add(y as u64);
                        pc = pc.wrapping_add(1);
                    }
                    0x81 | 0x92 => {
                        if opcode == 0x81 {
                            // STA (ind,X)
                            data = data.wrapping_add(x as u64) as u8 as u64;
                        }
                        // STA (ind)
                        let hi = 0x100 * read_low!(data.wrapping_add(1) as u8) as u64;
                        addr = hi.wrapping_add(read_low!(data) as u64);
                        pc = pc.wrapping_add(1);
                    }
                    _ => {
                        if opcode == 0x99 {
                            // STA abs,Y
                            data = data.wrapping_add(y as u64);
                        }
                        // STA abs
                        addr = data.wrapping_add(0x100 * get_msb!());
                        pc = pc.wrapping_add(2);
                    }
                }
                // sta_ptr:
                write_fast!(addr, a);
            }

            0xA1 | 0xB2 | 0xB1 | 0xB9 | 0xAD => {
                let addr: u64;
                match opcode {
                    0xA1 | 0xB2 => {
                        if opcode == 0xA1 {
                            // LDA (ind,X)
                            data = data.wrapping_add(x as u64) as u8 as u64;
                        }
                        // LDA (ind)
                        let hi = 0x100 * read_low!(data.wrapping_add(1) as u8) as u64;
                        addr = hi.wrapping_add(read_low!(data) as u64);
                        pc = pc.wrapping_add(1);
                    }
                    0xB1 => {
                        // LDA (ind),Y
                        let lo = (read_low!(data) as u64).wrapping_add(y as u64);
                        addr = lo
                            .wrapping_add(0x100 * read_low!(data.wrapping_add(1) as u8) as u64);
                        pc = pc.wrapping_add(1);
                    }
                    _ => {
                        if opcode == 0xB9 {
                            // LDA abs,Y
                            data = data.wrapping_add(y as u64);
                        }
                        // LDA abs
                        addr = data.wrapping_add(0x100 * get_msb!());
                        pc = pc.wrapping_add(2);
                    }
                }
                // a_nz_read_addr:
                nz = read_fast!(addr);
                a = nz as u8;
            }

            0xBE => {
                // LDX abs,y
                let addr = get_addr!().wrapping_add(y as u64);
                pc = pc.wrapping_add(2);
                flush_time!();
                nz = read!(addr);
                x = nz as u8;
                cache_time!();
            }

            0xB5 => {
                // LDA zp,x
                nz = read_low!(data.wrapping_add(x as u64) as u8) as u64;
                a = nz as u8;
                pc = pc.wrapping_add(1);
            }

            0xA9 => {
                // LDA #imm
                pc = pc.wrapping_add(1);
                a = data as u8;
                nz = data;
            }

            // Bit operations
            0x3C | 0x2C | 0x34 | 0x24 | 0x89 => {
                match opcode {
                    0x3C | 0x2C => {
                        if opcode == 0x3C {
                            // BIT abs,x
                            data = data.wrapping_add(x as u64);
                        }
                        // BIT abs
                        let addr = add_page!();
                        flush_time!();
                        nz = read!(addr);
                        cache_time!();
                    }
                    _ => {
                        if opcode == 0x34 {
                            // BIT zp,x
                            data = data.wrapping_add(x as u64) as u8 as u64;
                        }
                        if opcode != 0x89 {
                            // BIT zp
                            data = read_low!(data) as u64;
                        }
                        // BIT imm
                        nz = data;
                    }
                }
                // bit_common:
                pc = pc.wrapping_add(1);
                status &= !ST_V;
                status |= nz as u8 & ST_V;
                if nz & a as u64 == 0 {
                    // (otherwise Z should be clear, and nz must be non-zero
                    // if nz & a is)
                    nz <<= 8; // set Z flag without affecting N flag
                }
            }

            0xB3 | 0x93 | 0xA3 | 0x83 => {
                match opcode {
                    0xB3 | 0x93 => {
                        let mut addr: u64 = if opcode == 0xB3 {
                            // TST abs,x
                            get_msb!().wrapping_add(x as u64)
                        } else {
                            // TST abs
                            get_msb!()
                        };
                        // tst_abs:
                        addr = addr.wrapping_add(0x100 * instr_byte!(2));
                        pc = pc.wrapping_add(1);
                        flush_time!();
                        nz = read!(addr);
                        cache_time!();
                    }
                    0xA3 => {
                        // TST zp,x
                        nz = read_low!(get_msb!().wrapping_add(x as u64) as u8) as u64;
                    }
                    _ => {
                        // TST zp
                        nz = read_low!(get_msb!()) as u64;
                    }
                }
                // tst_common:
                pc = pc.wrapping_add(2);
                status &= !ST_V;
                status |= nz as u8 & ST_V;
                if nz & data == 0 {
                    // (otherwise Z should be clear, and nz must be non-zero
                    // if nz & data is)
                    nz <<= 8; // set Z flag without affecting N flag
                }
            }

            0x0C | 0x1C | 0x04 | 0x14 => {
                let addr: u64 = if opcode == 0x0C || opcode == 0x1C {
                    // TSB abs / TRB abs
                    let addr = get_addr!();
                    pc = pc.wrapping_add(1);
                    addr
                } else {
                    // TODO: everyone lists different behaviors for the status flags, ugh
                    // TSB zp / TRB zp
                    data + RAM_ADDR
                };
                // txb_addr:
                flush_time!();
                nz = a as u64 | read!(addr);
                if opcode & 0x10 != 0 {
                    nz ^= a as u64; // bits from a will already be set, so this clears them
                }
                status &= !ST_V;
                status |= nz as u8 & ST_V;
                pc = pc.wrapping_add(1);
                write!(addr, nz);
                cache_time!();
            }

            0x07 | 0x17 | 0x27 | 0x37 | 0x47 | 0x57 | 0x67 | 0x77 => {
                // RMBn
                pc = pc.wrapping_add(1);
                h.cpu().ram[data as usize] &= !(1u8 << (opcode >> 4));
            }

            0x87 | 0x97 | 0xA7 | 0xB7 | 0xC7 | 0xD7 | 0xE7 | 0xF7 => {
                // SMBn
                pc = pc.wrapping_add(1);
                h.cpu().ram[data as usize] |= 1u8 << ((opcode >> 4) - 8);
            }

            // Load/store
            0x9E | 0x9C => {
                if opcode == 0x9E {
                    // STZ abs,x
                    data = data.wrapping_add(x as u64);
                }
                // STZ abs
                data = add_page!();
                pc = pc.wrapping_add(1);
                flush_time!();
                write!(data, 0);
                cache_time!();
            }

            0x74 | 0x64 => {
                if opcode == 0x74 {
                    // STZ zp,x
                    data = data.wrapping_add(x as u64) as u8 as u64;
                }
                // STZ zp
                pc = pc.wrapping_add(1);
                write_low!(data, 0);
            }

            0x94 | 0x84 => {
                if opcode == 0x94 {
                    // STY zp,x
                    data = data.wrapping_add(x as u64) as u8 as u64;
                }
                // STY zp
                pc = pc.wrapping_add(1);
                write_low!(data, y);
            }

            0x96 | 0x86 => {
                if opcode == 0x96 {
                    // STX zp,y
                    data = data.wrapping_add(y as u64) as u8 as u64;
                }
                // STX zp
                pc = pc.wrapping_add(1);
                write_low!(data, x);
            }

            0xB6 | 0xA6 | 0xA2 => {
                if opcode == 0xB6 {
                    // LDX zp,y
                    data = data.wrapping_add(y as u64) as u8 as u64;
                }
                if opcode != 0xA2 {
                    // LDX zp
                    data = read_low!(data) as u64;
                }
                // LDX #imm
                pc = pc.wrapping_add(1);
                x = data as u8;
                nz = data;
            }

            0xB4 | 0xA4 | 0xA0 => {
                if opcode == 0xB4 {
                    // LDY zp,x
                    data = data.wrapping_add(x as u64) as u8 as u64;
                }
                if opcode != 0xA0 {
                    // LDY zp
                    data = read_low!(data) as u64;
                }
                // LDY #imm
                pc = pc.wrapping_add(1);
                y = data as u8;
                nz = data;
            }

            0xBC | 0xAC => {
                if opcode == 0xBC {
                    // LDY abs,X
                    data = data.wrapping_add(x as u64);
                }
                // LDY abs
                let addr = data.wrapping_add(0x100 * get_msb!());
                pc = pc.wrapping_add(2);
                flush_time!();
                nz = read!(addr);
                y = nz as u8;
                cache_time!();
            }

            0x8C | 0x8E => {
                let temp: u8 = if opcode == 0x8C {
                    // STY abs
                    y
                } else {
                    // STX abs
                    x
                };
                // store_abs:
                let addr = get_addr!();
                pc = pc.wrapping_add(2);
                flush_time!();
                write!(addr, temp);
                cache_time!();
            }

            // Compare
            0xEC | 0xE4 | 0xE0 => {
                match opcode {
                    0xEC => {
                        // CPX abs
                        let addr = get_addr!();
                        pc = pc.wrapping_add(1);
                        flush_time!();
                        data = read!(addr);
                        cache_time!();
                    }
                    0xE4 => {
                        // CPX zp
                        data = read_low!(data) as u64;
                    }
                    _ => {} // CPX #imm
                }
                // cpx_data:
                nz = (x as u64).wrapping_sub(data);
                pc = pc.wrapping_add(1);
                c = !nz;
                nz &= 0xFF;
            }

            0xCC | 0xC4 | 0xC0 => {
                match opcode {
                    0xCC => {
                        // CPY abs
                        let addr = get_addr!();
                        pc = pc.wrapping_add(1);
                        flush_time!();
                        data = read!(addr);
                        cache_time!();
                    }
                    0xC4 => {
                        // CPY zp
                        data = read_low!(data) as u64;
                    }
                    _ => {} // CPY #imm
                }
                // cpy_data:
                nz = (y as u64).wrapping_sub(data);
                pc = pc.wrapping_add(1);
                c = !nz;
                nz &= 0xFF;
            }

            // (Logical, Add/subtract: above)

            // Shift/rotate
            0x4A | 0x6A => {
                if opcode == 0x4A {
                    // LSR A
                    c = 0;
                }
                // ROR A
                nz = c >> 1 & 0x80;
                c = (a as u64) << 8;
                nz |= (a >> 1) as u64;
                a = nz as u8;
            }

            0x0A => {
                // ASL A
                nz = (a as u64) << 1;
                c = nz;
                a = nz as u8;
            }

            0x2A => {
                // ROL A
                nz = (a as u64) << 1;
                let temp = c >> 8 & 1;
                c = nz;
                nz |= temp;
                a = nz as u8;
            }

            0x5E | 0x4E | 0x6E | 0x7E => {
                if opcode == 0x5E || opcode == 0x7E {
                    // LSR abs,X / ROR abs,X
                    data = data.wrapping_add(x as u64);
                }
                if opcode == 0x5E || opcode == 0x4E {
                    // LSR abs
                    c = 0;
                }
                // ROR abs
                // ror_abs:
                data = add_page!();
                flush_time!();
                let temp = h.cpu_read(data as HesAddr);
                nz = (c >> 1 & 0x80) | (temp >> 1) as i64 as u64;
                c = ((temp as u32) << 8) as i32 as i64 as u64;
                // rotate_common:
                pc = pc.wrapping_add(1);
                write!(data, nz as u8);
                cache_time!();
            }

            0x3E | 0x1E | 0x0E | 0x2E => {
                if opcode == 0x3E || opcode == 0x1E {
                    // ROL abs,X / ASL abs,X
                    data = data.wrapping_add(x as u64);
                }
                if opcode == 0x1E || opcode == 0x0E {
                    // ASL abs
                    c = 0;
                }
                // ROL abs
                // rol_abs:
                data = add_page!();
                nz = c >> 8 & 1;
                flush_time!();
                c = ((h.cpu_read(data as HesAddr) as u32) << 1) as i32 as i64 as u64;
                nz |= c;
                // rotate_common:
                pc = pc.wrapping_add(1);
                write!(data, nz as u8);
                cache_time!();
            }

            0x76 | 0x56 | 0x46 | 0x66 => {
                if opcode == 0x76 || opcode == 0x56 {
                    // ROR zp,x / LSR zp,x
                    data = data.wrapping_add(x as u64) as u8 as u64;
                }
                if opcode == 0x56 || opcode == 0x46 {
                    // LSR zp
                    c = 0;
                }
                // ROR zp
                // ror_zp:
                let temp = read_low!(data) as u64;
                nz = (c >> 1 & 0x80) | (temp >> 1);
                c = temp << 8;
                // write_nz_zp:
                pc = pc.wrapping_add(1);
                write_low!(data, nz);
            }

            0x36 | 0x16 | 0x06 | 0x26 => {
                if opcode == 0x36 || opcode == 0x16 {
                    // ROL zp,x / ASL zp,x
                    data = data.wrapping_add(x as u64) as u8 as u64;
                }
                if opcode == 0x16 || opcode == 0x06 {
                    // ASL zp
                    c = 0;
                }
                // ROL zp
                // rol_zp:
                nz = c >> 8 & 1;
                c = (read_low!(data) as u64) << 1;
                nz |= c;
                // write_nz_zp:
                pc = pc.wrapping_add(1);
                write_low!(data, nz);
            }

            // Increment/decrement
            0x1A => {
                // INA
                nz = (a as u64).wrapping_add(1);
                a = nz as u8;
            }

            0xE8 => {
                // INX
                nz = (x as u64).wrapping_add(1);
                x = nz as u8;
            }

            0xC8 => {
                // INY
                nz = (y as u64).wrapping_add(1);
                y = nz as u8;
            }

            0x3A => {
                // DEA
                nz = (a as i64 - 1) as u64;
                a = nz as u8;
            }

            0xCA => {
                // DEX
                nz = (x as i64 - 1) as u64;
                x = nz as u8;
            }

            0x88 => {
                // DEY
                nz = (y as i64 - 1) as u64;
                y = nz as u8;
            }

            0xF6 | 0xE6 | 0xD6 | 0xC6 => {
                if opcode == 0xF6 || opcode == 0xD6 {
                    // INC zp,x / DEC zp,x
                    data = data.wrapping_add(x as u64) as u8 as u64;
                }
                nz = if opcode == 0xF6 || opcode == 0xE6 {
                    // INC zp
                    1
                } else {
                    // DEC zp
                    u64::MAX
                };
                // add_nz_zp:
                nz = nz.wrapping_add(read_low!(data) as u64);
                // write_nz_zp:
                pc = pc.wrapping_add(1);
                write_low!(data, nz);
            }

            0xFE | 0xEE | 0xDE | 0xCE => {
                data = match opcode {
                    // INC abs,x / DEC abs,x
                    0xFE | 0xDE => (x as u64).wrapping_add(get_addr!()),
                    // INC abs / DEC abs
                    _ => get_addr!(),
                };
                nz = if opcode == 0xFE || opcode == 0xEE {
                    // inc_ptr:
                    1
                } else {
                    // dec_ptr:
                    u64::MAX
                };
                // inc_common:
                flush_time!();
                nz = nz.wrapping_add(read!(data));
                pc = pc.wrapping_add(2);
                write!(data, nz as u8);
                cache_time!();
            }

            // Transfer
            0xA8 => {
                // TAY
                y = a;
                nz = a as u64;
            }

            0x98 => {
                // TYA
                a = y;
                nz = y as u64;
            }

            0xAA => {
                // TAX
                x = a;
                nz = a as u64;
            }

            0x8A => {
                // TXA
                a = x;
                nz = x as u64;
            }

            0x9A => {
                // TXS
                set_sp!(x); // verified (no flag change)
            }

            0xBA => {
                // TSX
                x = get_sp!();
                nz = x as u64;
            }

            0x02 => {
                // SXY
                std::mem::swap(&mut x, &mut y);
            }

            0x22 => {
                // SAX
                std::mem::swap(&mut a, &mut x);
            }

            0x42 => {
                // SAY
                std::mem::swap(&mut a, &mut y);
            }

            0x62 => {
                // CLA
                a = 0;
            }

            0x82 => {
                // CLX
                x = 0;
            }

            0xC2 => {
                // CLY
                y = 0;
            }

            // Stack
            0x48 => push!(a), // PHA

            0xDA => push!(x), // PHX

            0x5A => push!(y), // PHY

            0x40 => {
                // RTI
                let temp = read_low!(sp);
                pc = read_low!(0x100 | sp.wrapping_sub(0xFF)) as u64;
                pc |= read_low!(0x100 | sp.wrapping_sub(0xFE)) as u64 * 0x100;
                sp = sp.wrapping_sub(0xFD) | 0x100;
                data = status as u64;
                set_status!(temp);
                let cpu = h.cpu();
                cpu.r.status = status; // update externally-visible I flag
                if (data ^ status as u64) & ST_I as u64 != 0 {
                    let mut new_time = cpu.end_time_;
                    if status & ST_I == 0 && new_time > cpu.irq_time_ {
                        new_time = cpu.irq_time_;
                    }
                    let delta = cpu.base.wrapping_sub(new_time);
                    cpu.base = new_time;
                    s_time = s_time.wrapping_add(delta);
                }
            }

            0x68 => {
                // PLA
                a = pop!();
                nz = a as u64;
            }

            0xFA => {
                // PLX
                x = pop!();
                nz = x as u64;
            }

            0x7A => {
                // PLY
                y = pop!();
                nz = y as u64;
            }

            0x28 => {
                // PLP
                let temp = pop!();
                let changed = status ^ temp;
                set_status!(temp);
                if changed & ST_I != 0 {
                    // (otherwise the I flag didn't change)
                    next = if status & ST_I != 0 {
                        Next::Sei
                    } else {
                        Next::Cli
                    };
                }
            }

            0x08 => {
                // PHP
                let temp = calc_status!();
                push!(temp | ST_B);
            }

            // Flags
            0x38 => {
                // SEC
                c = !0;
            }

            0x18 => {
                // CLC
                c = 0;
            }

            0xB8 => {
                // CLV
                status &= !ST_V;
            }

            0xD8 => {
                // CLD
                status &= !ST_D;
            }

            0xF8 => {
                // SED
                status |= ST_D;
            }

            0x58 => {
                // CLI
                if status & ST_I != 0 {
                    status &= !ST_I;
                    next = Next::Cli;
                }
            }

            0x78 => {
                // SEI
                if status & ST_I == 0 {
                    status |= ST_I;
                    next = Next::Sei;
                }
            }

            // Special
            0x53 => {
                // TAM
                let bits = data as u8; // avoid using data across function call
                pc = pc.wrapping_add(1);
                for i in 0..8 {
                    if bits & (1 << i) != 0 {
                        set_mmr(h, i, a as i32);
                    }
                }
            }

            0x43 => {
                // TMA
                pc = pc.wrapping_add(1);
                let mut i = 0;
                loop {
                    if data & 1 != 0 {
                        a = h.cpu().mmr[i];
                    }
                    i += 1;
                    data >>= 1;
                    if data == 0 {
                        break;
                    }
                }
            }

            0x03 | 0x13 | 0x23 => {
                // ST0 / ST1 / ST2
                let mut addr = (opcode >> 4) as i32;
                if addr != 0 {
                    addr += 1;
                }
                pc = pc.wrapping_add(1);
                flush_time!();
                h.cpu_write_vdp(addr, data as i32);
                cache_time!();
            }

            0xEA => {} // NOP

            0x54 => {
                // CSL
                // debug_printf( "CSL not supported\n" );
                illegal_encountered = true;
            }

            0xD4 => {} // CSH

            0xF4 => {
                // SET
                //fuint16 operand = GET_MSB();
                // debug_printf( "SET not handled\n" );
                //switch ( data )
                //{
                //}
                illegal_encountered = true;
            }

            // Block transfer
            0xE3 | 0xF3 | 0xD3 | 0xC3 | 0x73 => {
                let in_alt: u64;
                let mut in_inc: i64;
                let out_alt: u64;
                let mut out_inc: i64;
                match opcode {
                    0xE3 | 0xF3 => {
                        // TIA / TAI
                        // FIXME (upstream): the increment meant to alternate
                        // (the destination's for TIA, the source's for TAI)
                        // starts at 0, so negating it never alternates: TIA
                        // writes every byte to its first address and TAI reads
                        // every byte from its first. Kept, as upstream plays it
                        // (danielpclark/SDL#143).
                        in_alt = (opcode == 0xF3) as u64;
                        // bxfer_alt:
                        in_inc = (in_alt ^ 1) as i64;
                        out_alt = in_inc as u64;
                        out_inc = in_alt as i64;
                    }
                    _ => {
                        (in_inc, out_inc) = match opcode {
                            0xD3 => (1, 0),   // TIN
                            0xC3 => (-1, -1), // TDD
                            _ => (1, 1),      // TII
                        };
                        // bxfer_no_alt:
                        in_alt = 0;
                        out_alt = 0;
                    }
                }
                // bxfer:
                let mut in_ = instr_byte!(0) | instr_byte!(1) << 8;
                let mut out = instr_byte!(2) | instr_byte!(3) << 8;
                let mut count = (instr_byte!(4) | instr_byte!(5) << 8) as i32;
                if count == 0 {
                    count = 0x10000;
                }
                pc = pc.wrapping_add(6);
                write_low!(0x100 | sp.wrapping_sub(1), y);
                write_low!(0x100 | sp.wrapping_sub(2), a);
                write_low!(0x100 | sp.wrapping_sub(3), x);
                flush_time!();
                loop {
                    // TODO: reads from $0800-$1400 in I/O page return 0 and don't access I/O
                    let t = read!(in_) as u8;
                    in_ = in_.wrapping_add(in_inc as u64);
                    in_ &= 0xFFFF;
                    {
                        let cpu = h.cpu();
                        cpu.time = cpu.time.wrapping_add(6);
                    }
                    if in_alt != 0 {
                        in_inc = -in_inc;
                    }
                    write!(out, t);
                    out = out.wrapping_add(out_inc as u64);
                    out &= 0xFFFF;
                    if out_alt != 0 {
                        out_inc = -out_inc;
                    }
                    count -= 1;
                    if count == 0 {
                        break;
                    }
                }
                cache_time!();
            }

            // Illegal
            _ => {
                // debug_printf( "Illegal opcode $%02X at $%04X\n", (int) opcode, (int) pc - 1 );
                illegal_encountered = true;
            }
        }
    }

    let cpu = h.cpu();
    cpu.time = s_time;

    cpu.r.pc = pc as u16;
    cpu.r.sp = get_sp!();
    cpu.r.a = a;
    cpu.r.x = x;
    cpu.r.y = y;

    {
        let temp = calc_status!();
        h.cpu().r.status = temp;
    }

    illegal_encountered
}
