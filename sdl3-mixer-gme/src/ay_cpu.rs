// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/

/*
Last validated with zexall 2006.11.21 5:26 PM
* Doesn't implement the R register or immediate interrupt after EI.
* Address wrap-around isn't completely correct, but is prevented from crashing emulator.
*/

//#include "z80_cpu_log.h"

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

//! Z80 CPU emulator (`Ay_Cpu.h`, `Ay_Cpu.cpp`), for the AY files, and
//! with the KSS files' memory (`Kss_Cpu.cpp`, the same code over paged
//! memory, see `kss_cpu`).
//!
//! [`run`] reaches memory and the ports through [`Z80Host`]: the AY
//! emulator's flat 64K of memory (and the padding past it) with its
//! `ay_cpu_out()` and `ay_cpu_in()`, or the KSS emulator's pages with its
//! `kss_cpu_write()`, `kss_cpu_out()` and `kss_cpu_in()`. The two upstream
//! copies differ only in what the host says: the memory access, the
//! program counter (16 bits in `Ay_Cpu`, wider in `Kss_Cpu`, which stops
//! at its idle address past the end of memory), and where an
//! instruction's address operand is read from. Upstream runs on a local
//! copy of its time state, which it copies back when it stops; this does
//! the same (and, as `Kss_Cpu`'s `CPU_WRITE()` does, stores the time
//! before a write that goes to the emulator).

pub type CpuTime = i32;

/// can read this far past end of memory
pub const CPU_PADDING: usize = 0x100;

// (the registers, in upstream's little-endian `regs_t` layout: c, b, e, d,
// l, h, a, flags; the pairs `pairs_t` are bc, de, hl and fa over them)
pub const RC: usize = 0;
pub const RB: usize = 1;
pub const RE: usize = 2;
pub const RD: usize = 3;
pub const RL: usize = 4;
pub const RH: usize = 5;
pub const RA: usize = 6;
pub const RFLAGS: usize = 7;

/// `regs_t` and `pairs_t` (a union upstream)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Regs(pub [u8; 8]);

impl Regs {
    /// pair `i` (`w.bc`, `w.de`, `w.hl`, `w.fa`)
    #[inline]
    pub fn w(&self, i: usize) -> u16 {
        self.0[i * 2] as u16 | (self.0[i * 2 + 1] as u16) << 8
    }
    #[inline]
    pub fn set_w(&mut self, i: usize, v: u16) {
        self.0[i * 2] = v as u8;
        self.0[i * 2 + 1] = (v >> 8) as u8;
    }
    pub fn hl(&self) -> u16 {
        self.w(2)
    }
}

/// Registers are not updated until run() returns
#[derive(Clone, Copy, Debug, Default)]
pub struct Registers {
    pub pc: u16,
    pub sp: u16,
    pub ix: u16,
    pub iy: u16,
    pub b: Regs,
    pub alt: Regs,
    pub iff1: u8,
    pub iff2: u8,
    pub r: u8,
    pub i: u8,
    pub im: u8,
}

#[derive(Clone, Debug)]
pub struct AyCpu {
    szpc: [u8; 0x200],
    end_time_: CpuTime,
    // state_t
    base: CpuTime,
    time: CpuTime,
    pub r: Registers,
}

/// The place of an instruction's opcode in memory (`instr`): a memory
/// region of the host's and the offset in it.
#[derive(Clone, Copy, Debug)]
pub struct Instr {
    pub mem: u8,
    pub offset: usize,
}

/// The emulator running the CPU: its memory and ports.
pub trait Z80Host {
    /// The program counter is 16 bits (`Ay_Cpu`), rather than wider and
    /// stopping at the idle address past 0xFFFF (`Kss_Cpu`).
    const PC_16: bool;
    /// An instruction's address operand is read from the memory after its
    /// opcode (`Kss_Cpu`'s `instr`), rather than at the program counter.
    const INSTR_OPERANDS: bool;

    fn cpu(&mut self) -> &mut AyCpu;
    /// `READ_PROG()`, `READ()`
    fn read(&mut self, addr: u32) -> u8;
    /// `READ_WORD()`
    fn read_word(&mut self, addr: u32) -> u16;
    /// `WRITE()` (`CPU_WRITE()`)
    fn write(&mut self, addr: u32, data: u8);
    /// `WRITE_WORD()`
    fn write_word(&mut self, addr: u32, data: u16);
    /// `instr` for the opcode at `pc`
    fn instr(&mut self, pc: u32) -> Instr;
    /// `instr [k]`
    fn instr_byte(&mut self, instr: Instr, k: usize) -> u8;
    /// `CPU_OUT()`
    fn cpu_out(&mut self, time: CpuTime, addr: u32, data: i32);
    /// `CPU_IN()`
    fn cpu_in(&mut self, time: CpuTime, addr: u32) -> i32;
}

// flags, named with hex value for clarity
const S80: i32 = 0x80;
const Z40: i32 = 0x40;
const F20: i32 = 0x20;
const H10: i32 = 0x10;
const F08: i32 = 0x08;
const V04: i32 = 0x04;
const P04: i32 = 0x04;
const N02: i32 = 0x02;
const C01: i32 = 0x01;

impl Default for AyCpu {
    fn default() -> Self {
        Self::new()
    }
}

impl AyCpu {
    pub fn new() -> Self {
        let mut szpc = [0u8; 0x200];
        let mut i = 0x100;
        while i > 0 {
            i -= 1;
            let mut even = 1;
            let mut p = i;
            while p != 0 {
                even ^= p;
                p >>= 1;
            }
            let n = (i as i32 & (S80 | F20 | F08)) | ((even & 1) as i32 * P04);
            szpc[i] = n as u8;
            szpc[i + 0x100] = (n | C01) as u8;
        }
        szpc[0x000] |= Z40 as u8;
        szpc[0x100] |= Z40 as u8;
        AyCpu {
            szpc,
            end_time_: 0,
            base: 0,
            time: 0,
            r: Registers::default(),
        }
    }

    /// Clear all registers (the memory is the emulator's)
    pub fn reset(&mut self) {
        self.time = 0;
        self.base = 0;
        self.end_time_ = 0;

        self.r = Registers::default();
    }

    /// Time of beginning of next instruction
    pub fn time(&self) -> CpuTime {
        self.time.wrapping_add(self.base)
    }

    /// Alter current time. Not supported during run() call.
    pub fn set_time(&mut self, t: CpuTime) {
        self.time = t.wrapping_sub(self.base);
    }
    pub fn adjust_time(&mut self, delta: i32) {
        self.time = self.time.wrapping_add(delta);
    }

    fn set_end_time(&mut self, t: CpuTime) {
        let delta = self.base.wrapping_sub(t);
        self.base = t;
        self.time = self.time.wrapping_add(delta);
    }
}

/// How a run stops (upstream's labels).
enum Stop {
    OutOfTime,
    Halt,
    HitIdleAddr,
}

// high four bits are $ED time - 8, low four bits are $DD/$FD time - 8
static ED_DD_TIMING: [u8; 0x100] = [
    //0    1    2    3    4    5    6    7    8    9    A    B    C    D    E    F
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x06, 0x0C, 0x02, 0x00, 0x00, 0x03, 0x00, 0x00, 0x07, 0x0C, 0x02, 0x00, 0x00, 0x03, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x0F, 0x0F, 0x0B, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x40, 0x40, 0x70, 0xC0, 0x00, 0x60, 0x0B, 0x10, 0x40, 0x40, 0x70, 0xC0, 0x00, 0x60, 0x0B, 0x10,
    0x40, 0x40, 0x70, 0xC0, 0x00, 0x60, 0x0B, 0x10, 0x40, 0x40, 0x70, 0xC0, 0x00, 0x60, 0x0B, 0x10,
    0x40, 0x40, 0x70, 0xC0, 0x00, 0x60, 0x0B, 0xA0, 0x40, 0x40, 0x70, 0xC0, 0x00, 0x60, 0x0B, 0xA0,
    0x4B, 0x4B, 0x7B, 0xCB, 0x0B, 0x6B, 0x00, 0x0B, 0x40, 0x40, 0x70, 0xC0, 0x00, 0x60, 0x0B, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0B, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0B, 0x00,
    0x80, 0x80, 0x80, 0x80, 0x00, 0x00, 0x0B, 0x00, 0x80, 0x80, 0x80, 0x80, 0x00, 0x00, 0x0B, 0x00,
    0xD0, 0xD0, 0xD0, 0xD0, 0x00, 0x00, 0x0B, 0x00, 0xD0, 0xD0, 0xD0, 0xD0, 0x00, 0x00, 0x0B, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x06, 0x00, 0x0F, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

static BASE_TIMING: [u8; 0x100] = [
    //   0  1  2  3  4  5  6  7  8  9  A  B  C  D  E  F
    4, 10, 7, 6, 4, 4, 7, 4, 4, 11, 7, 6, 4, 4, 7, 4, // 0
    13, 10, 7, 6, 4, 4, 7, 4, 12, 11, 7, 6, 4, 4, 7, 4, // 1
    12, 10, 16, 6, 4, 4, 7, 4, 12, 11, 16, 6, 4, 4, 7, 4, // 2
    12, 10, 13, 6, 11, 11, 10, 4, 12, 11, 13, 6, 4, 4, 7, 4, // 3
    4, 4, 4, 4, 4, 4, 7, 4, 4, 4, 4, 4, 4, 4, 7, 4, // 4
    4, 4, 4, 4, 4, 4, 7, 4, 4, 4, 4, 4, 4, 4, 7, 4, // 5
    4, 4, 4, 4, 4, 4, 7, 4, 4, 4, 4, 4, 4, 4, 7, 4, // 6
    7, 7, 7, 7, 7, 7, 4, 7, 4, 4, 4, 4, 4, 4, 7, 4, // 7
    4, 4, 4, 4, 4, 4, 7, 4, 4, 4, 4, 4, 4, 4, 7, 4, // 8
    4, 4, 4, 4, 4, 4, 7, 4, 4, 4, 4, 4, 4, 4, 7, 4, // 9
    4, 4, 4, 4, 4, 4, 7, 4, 4, 4, 4, 4, 4, 4, 7, 4, // A
    4, 4, 4, 4, 4, 4, 7, 4, 4, 4, 4, 4, 4, 4, 7, 4, // B
    11, 10, 10, 10, 17, 11, 7, 11, 11, 10, 10, 8, 17, 17, 7, 11, // C
    11, 10, 10, 11, 17, 11, 7, 11, 11, 4, 10, 11, 17, 8, 7, 11, // D
    11, 10, 10, 19, 17, 11, 7, 11, 11, 4, 10, 4, 17, 8, 7, 11, // E
    11, 10, 10, 4, 17, 11, 7, 11, 11, 6, 10, 4, 17, 8, 7, 11, // F
];

/// Run until specified time is reached. Returns true if suspicious/unsupported
/// instruction was encountered at any point during run.
pub fn run<H: Z80Host + ?Sized>(h: &mut H, end_time: CpuTime) -> bool {
    h.cpu().set_end_time(end_time);
    let s_base = h.cpu().base;
    let mut warning = false;

    let mut rg: [u8; 8] = h.cpu().r.b.0;

    let mut s_time: CpuTime = h.cpu().time;
    let r = h.cpu().r;
    let mut pc: u32 = r.pc as u32;
    let mut sp: u16 = r.sp;
    let mut ix: u16 = r.ix; // TODO: keep in memory for direct access?
    let mut iy: u16 = r.iy;
    let mut flags: i32 = r.b.0[RFLAGS] as i32;

    // R8( n, offset ): ((r8_ - offset) [(n) ^ 1]) on a little-endian machine
    macro_rules! r8 {
        ($n:expr, $offset:expr) => {
            rg[((($n) as usize) ^ 1) - ($offset) as usize]
        };
    }
    // R16( n, shift, offset ): the pair at byte (n >> (shift - 1)) - (offset >> (shift - 1))
    macro_rules! r16_index {
        ($n:expr, $shift:expr, $offset:expr) => {
            ((($n) as usize >> ($shift - 1)) - (($offset) as usize >> ($shift - 1))) / 2
        };
    }
    macro_rules! r16 {
        ($n:expr, $shift:expr, $offset:expr) => {{
            let i = r16_index!($n, $shift, $offset);
            rg[i * 2] as u16 | (rg[i * 2 + 1] as u16) << 8
        }};
    }
    macro_rules! set_r16 {
        ($n:expr, $shift:expr, $offset:expr, $v:expr) => {{
            let i = r16_index!($n, $shift, $offset);
            let v = ($v) as u16;
            rg[i * 2] = v as u8;
            rg[i * 2 + 1] = (v >> 8) as u8;
        }};
    }
    macro_rules! bc {
        () => {
            (rg[RC] as u16 | (rg[RB] as u16) << 8)
        };
    }
    macro_rules! de {
        () => {
            (rg[RE] as u16 | (rg[RD] as u16) << 8)
        };
    }
    macro_rules! hl {
        () => {
            (rg[RL] as u16 | (rg[RH] as u16) << 8)
        };
    }
    macro_rules! set_pair {
        ($lo:expr, $hi:expr, $v:expr) => {{
            let v = ($v) as u16;
            rg[$lo] = v as u8;
            rg[$hi] = (v >> 8) as u8;
        }};
    }
    macro_rules! szpc {
        ($n:expr) => {
            h.cpu().szpc[($n) as usize] as i32
        };
    }
    macro_rules! sz28p {
        ($n:expr) => {
            szpc!($n)
        };
    }
    macro_rules! sz28c {
        ($n:expr) => {
            (szpc!($n) & !P04)
        };
    }
    macro_rules! sz28 {
        ($n:expr) => {
            sz28c!($n)
        };
    }
    macro_rules! time {
        () => {
            s_time.wrapping_add(s_base)
        };
    }
    macro_rules! read_prog {
        ($addr:expr) => {
            h.read(($addr) as u32)
        };
    }
    macro_rules! read {
        ($addr:expr) => {
            read_prog!($addr)
        };
    }
    macro_rules! write {
        ($addr:expr, $data:expr) => {{
            let a = ($addr) as u32;
            let d = ($data) as u8;
            h.cpu().time = s_time; // (SYNC_TIME())
            h.write(a, d);
        }};
    }
    macro_rules! read_word {
        ($addr:expr) => {
            h.read_word(($addr) as u32)
        };
    }
    macro_rules! write_word {
        ($addr:expr, $data:expr) => {{
            let a = ($addr) as u32;
            let d = ($data) as u16;
            h.write_word(a, d);
        }};
    }
    // (the program counter, added to: 16 bits in Ay_Cpu)
    macro_rules! pc_add {
        ($n:expr) => {{
            pc = (pc as i64 + ($n) as i64) as u32;
            if H::PC_16 {
                pc &= 0xFFFF;
            }
        }};
    }
    macro_rules! cpu_in {
        ($addr:expr) => {{
            let t = time!();
            h.cpu_in(t, ($addr) as u32)
        }};
    }
    macro_rules! cpu_out {
        ($addr:expr, $data:expr) => {{
            let t = time!();
            h.cpu_out(t, ($addr) as u32, ($data) as i32)
        }};
    }

    // (how the run ends)
    let stop: Stop = 'run: loop {
        // loop:

        // check( (unsigned long) pc < 0x10000 ); ...

        let instr = h.instr(pc);
        let mut opcode: u8 = h.instr_byte(instr, 0);
        pc_add!(1);

        // GET_ADDR(): the operand after the opcode (and, from the ED and
        // DD/FD prefixes on, after the second opcode byte)
        macro_rules! get_addr_at {
            ($k:expr) => {{
                if H::INSTR_OPERANDS {
                    h.instr_byte(instr, $k) as u16 | (h.instr_byte(instr, $k + 1) as u16) << 8
                } else {
                    read_word!(pc)
                }
            }};
        }
        macro_rules! get_addr {
            () => {
                get_addr_at!(1)
            };
        }

        let mut data: u16 = BASE_TIMING[opcode as usize] as u16;
        s_time = s_time.wrapping_add(data as i32);
        if s_time >= 0 {
            // possibly_out_of_time:
            if s_time >= data as i32 {
                s_time -= data as i32;
                break 'run Stop::OutOfTime;
            }
        }
        // almost_out_of_time:

        data = read_prog!(pc) as u16;

        macro_rules! jr_not_taken {
            () => {{
                s_time -= 5;
                continue 'run;
            }};
        }
        macro_rules! jp_not_taken {
            () => {{
                pc_add!(2);
                continue 'run;
            }};
        }
        macro_rules! call_not_taken {
            () => {{
                s_time -= 7;
                jp_not_taken!();
            }};
        }
        // JR
        macro_rules! jr {
            ($cond:expr) => {{
                let disp = data as u8 as i8 as i32;
                pc_add!(1);
                if !($cond) {
                    jr_not_taken!();
                }
                pc = (pc as i32).wrapping_add(disp) as u16 as u32;
                continue 'run;
            }};
        }
        // JP
        macro_rules! jp {
            ($cond:expr) => {{
                if !($cond) {
                    jp_not_taken!();
                }
                pc = get_addr!() as u32;
                continue 'run;
            }};
        }
        macro_rules! ret_taken {
            () => {{
                pc = read_word!(sp) as u32;
                sp = sp.wrapping_add(2);
                continue 'run;
            }};
        }
        // RET
        macro_rules! ret {
            ($cond:expr) => {{
                if $cond {
                    ret_taken!();
                }
                s_time -= 6;
                continue 'run;
            }};
        }
        macro_rules! call_taken {
            () => {{
                let addr = (pc + 2) as u16;
                pc = get_addr!() as u32;
                sp = sp.wrapping_sub(2);
                write_word!(sp, addr);
                continue 'run;
            }};
        }
        // CALL
        macro_rules! call {
            ($cond:expr) => {{
                if $cond {
                    call_taken!();
                }
                call_not_taken!();
            }};
        }
        macro_rules! push_data {
            () => {{
                sp = sp.wrapping_sub(2);
                write_word!(sp, data);
                continue 'run;
            }};
        }
        macro_rules! adc_data {
            () => {{
                let mut result: i32 = data as i32 + (flags & C01);
                data ^= rg[RA] as u16;
                flags = (opcode >> 3) as i32 & N02; // bit 4 is set in subtract opcodes
                if flags != 0 {
                    result = -result;
                }
                result += rg[RA] as i32;
                data ^= result as u16;
                flags |= (data as i32 & H10)
                    | ((data as i32 - -0x80) >> 6 & V04)
                    | sz28c!(result & 0x1FF);
                rg[RA] = result as u8;
                continue 'run;
            }};
        }
        macro_rules! cp_data {
            () => {{
                let result: i32 = rg[RA] as i32 - data as i32;
                flags = N02 | (data as i32 & (F20 | F08)) | (result >> 8 & C01);
                data ^= rg[RA] as u16;
                flags |= (((result ^ rg[RA] as i32) & data as i32) >> 5 & V04)
                    | (((data as i32 & H10) ^ result) & (S80 | H10));
                if result as u8 != 0 {
                    continue 'run;
                }
                flags |= Z40;
                continue 'run;
            }};
        }
        macro_rules! add_hl_data {
            () => {{
                let sum: u32 = hl!() as u32 + data as u32;
                data ^= hl!();
                set_pair!(RL, RH, sum);
                flags = (flags & (S80 | Z40 | V04))
                    | (sum >> 16) as i32
                    | (sum >> 8) as i32 & (F20 | F08)
                    | ((data as u32 ^ sum) >> 8) as i32 & H10;
                continue 'run;
            }};
        }
        macro_rules! inc_set_flags {
            () => {{
                flags = (flags & C01) | (((data as i32 & 0x0F) - 1) & H10) | sz28!(data as u8);
                if data != 0x80 {
                    continue 'run;
                }
                flags |= V04;
                continue 'run;
            }};
        }
        macro_rules! dec_set_flags {
            () => {{
                flags =
                    (flags & C01) | N02 | (((data as i32 & 0x0F) + 1) & H10) | sz28!(data as u8);
                if data != 0x7F {
                    continue 'run;
                }
                flags |= V04;
                continue 'run;
            }};
        }
        macro_rules! and_data {
            () => {{
                rg[RA] &= data as u8;
                flags = sz28p!(rg[RA]) | H10;
                continue 'run;
            }};
        }
        macro_rules! or_data {
            () => {{
                rg[RA] |= data as u8;
                flags = sz28p!(rg[RA]);
                continue 'run;
            }};
        }
        macro_rules! xor_data {
            () => {{
                rg[RA] ^= data as u8;
                flags = sz28p!(rg[RA]);
                continue 'run;
            }};
        }

        // Conditional
        let zero = flags & Z40 != 0;
        let carry = flags & C01 != 0;
        let even = flags & P04 != 0;
        let minus = flags & S80 != 0;

        match opcode {
            // Common
            0x00 | 0x40 | 0x49 | 0x52 | 0x5B | 0x64 | 0x6D | 0x7F => {
                // NOP, LD B,B etc.
                continue 'run;
            }

            0x08 => {
                // EX AF,AF'
                let cpu = h.cpu();
                let temp = cpu.r.alt.0[RA];
                cpu.r.alt.0[RA] = rg[RA];
                rg[RA] = temp;

                let temp = cpu.r.alt.0[RFLAGS] as i32;
                cpu.r.alt.0[RFLAGS] = flags as u8;
                flags = temp;
                continue 'run;
            }

            0xD3 => {
                // OUT (imm),A
                pc_add!(1);
                cpu_out!(data as u32 + rg[RA] as u32 * 0x100, rg[RA]);
                continue 'run;
            }

            0x2E => {
                // LD L,imm
                pc_add!(1);
                rg[RL] = data as u8;
                continue 'run;
            }

            0x3E => {
                // LD A,imm
                pc_add!(1);
                rg[RA] = data as u8;
                continue 'run;
            }

            0x3A => {
                // LD A,(addr)
                let addr: u16 = get_addr!();
                pc_add!(2);
                rg[RA] = read!(addr);
                continue 'run;
            }

            // Conditional
            0x20 => jr!(!zero),  // JR NZ,disp
            0x28 => jr!(zero),   // JR Z,disp
            0x30 => jr!(!carry), // JR NC,disp
            0x38 => jr!(carry),  // JR C,disp
            0x18 => jr!(true),   // JR disp

            0x10 => {
                // DJNZ disp
                let temp: i32 = rg[RB] as i32 - 1;
                rg[RB] = temp as u8;
                jr!(temp != 0)
            }

            0xC2 => jp!(!zero),  // JP NZ,addr
            0xCA => jp!(zero),   // JP Z,addr
            0xD2 => jp!(!carry), // JP NC,addr
            0xDA => jp!(carry),  // JP C,addr
            0xE2 => jp!(!even),  // JP PO,addr
            0xEA => jp!(even),   // JP PE,addr
            0xF2 => jp!(!minus), // JP P,addr
            0xFA => jp!(minus),  // JP M,addr

            0xC3 => {
                // JP addr
                pc = get_addr!() as u32;
                continue 'run;
            }

            0xE9 => {
                // JP HL
                pc = hl!() as u32;
                continue 'run;
            }

            0xC0 => ret!(!zero),  // RET NZ
            0xC8 => ret!(zero),   // RET Z
            0xD0 => ret!(!carry), // RET NC
            0xD8 => ret!(carry),  // RET C
            0xE0 => ret!(!even),  // RET PO
            0xE8 => ret!(even),   // RET PE
            0xF0 => ret!(!minus), // RET P
            0xF8 => ret!(minus),  // RET M

            0xC9 => {
                // RET
                ret_taken!();
            }

            0xC4 => call!(!zero),  // CALL NZ,addr
            0xCC => call!(zero),   // CALL Z,addr
            0xD4 => call!(!carry), // CALL NC,addr
            0xDC => call!(carry),  // CALL C,addr
            0xE4 => call!(!even),  // CALL PO,addr
            0xEC => call!(even),   // CALL PE,addr
            0xF4 => call!(!minus), // CALL P,addr
            0xFC => call!(minus),  // CALL M,addr

            0xCD => {
                // CALL addr
                call_taken!();
            }

            0xFF | 0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 => {
                // RST
                // (Ay_Cpu: if ( (pc - 1) > 0xFFFF ), which its 16-bit pc
                // never is; Kss_Cpu: the idle address)
                if opcode == 0xFF && !H::PC_16 && pc > 0xFFFF {
                    break 'run Stop::HitIdleAddr;
                }
                data = pc as u16;
                pc = (opcode & 0x38) as u32;
                push_data!();
            }

            // PUSH/POP
            0xF5 => {
                // PUSH AF
                data = (rg[RA] as u32 * 0x100 + flags as u32) as u16;
                push_data!();
            }

            0xC5 | 0xD5 | 0xE5 => {
                // PUSH BC, DE, HL
                data = r16!(opcode, 4, 0xC5);
                push_data!();
            }

            0xF1 => {
                // POP AF
                flags = read!(sp) as i32;
                rg[RA] = read!(sp as u32 + 1);
                sp = sp.wrapping_add(2);
                continue 'run;
            }

            0xC1 | 0xD1 | 0xE1 => {
                // POP BC, DE, HL
                let v = read_word!(sp);
                set_r16!(opcode, 4, 0xC1, v);
                sp = sp.wrapping_add(2);
                continue 'run;
            }

            // ADC/ADD/SBC/SUB
            0x96 | 0x86 | 0x9E | 0x8E => {
                if opcode == 0x96 || opcode == 0x86 {
                    // SUB (HL), ADD (HL)
                    flags &= !C01;
                }
                // SBC (HL), ADC (HL)
                data = read!(hl!()) as u16;
                adc_data!();
            }

            0xD6 | 0xC6 | 0xDE | 0xCE => {
                if opcode == 0xD6 || opcode == 0xC6 {
                    // SUB A,imm, ADD imm
                    flags &= !C01;
                }
                // SBC A,imm, ADC imm
                pc_add!(1);
                adc_data!();
            }

            0x90..=0x95 | 0x97 | 0x80..=0x85 | 0x87 | 0x98..=0x9D | 0x9F | 0x88..=0x8D | 0x8F => {
                if opcode < 0x88 || (0x90..0x98).contains(&opcode) {
                    // SUB r, ADD r
                    flags &= !C01;
                }
                // SBC r, ADC r
                data = r8!(opcode & 7, 0) as u16;
                adc_data!();
            }

            // CP
            0xBE => {
                // CP (HL)
                data = read!(hl!()) as u16;
                cp_data!();
            }

            0xFE => {
                // CP imm
                pc_add!(1);
                cp_data!();
            }

            0xB8..=0xBD | 0xBF => {
                // CP r
                data = r8!(opcode, 0xB8) as u16;
                cp_data!();
            }

            // ADD HL,rp
            0x39 => {
                // ADD HL,SP
                data = sp;
                add_hl_data!();
            }

            0x09 | 0x19 | 0x29 => {
                // ADD HL,BC, DE, HL
                data = r16!(opcode, 4, 0x09);
                add_hl_data!();
            }

            0x27 => {
                // DAA
                let mut a: i32 = rg[RA] as i32;
                if a > 0x99 {
                    flags |= C01;
                }

                let mut adjust: i32 = 0x60 & -(flags & C01);

                if flags & H10 != 0 || (a & 0x0F) > 9 {
                    adjust |= 0x06;
                }

                if flags & N02 != 0 {
                    adjust = -adjust;
                }
                a += adjust;

                flags = (flags & (C01 | N02)) | ((rg[RA] as i32 ^ a) & H10) | sz28p!(a as u8);
                rg[RA] = a as u8;
                continue 'run;
            }

            // INC/DEC
            0x34 => {
                // INC (HL)
                data = read!(hl!()) as u16 + 1;
                write!(hl!(), data);
                inc_set_flags!();
            }

            0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x3C => {
                // INC r
                let v = r8!(opcode >> 3, 0).wrapping_add(1);
                r8!(opcode >> 3, 0) = v;
                data = v as u16;
                inc_set_flags!();
            }

            0x35 => {
                // DEC (HL)
                data = (read!(hl!()) as u16).wrapping_sub(1);
                write!(hl!(), data);
                dec_set_flags!();
            }

            0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x3D => {
                // DEC r
                let v = r8!(opcode >> 3, 0).wrapping_sub(1);
                r8!(opcode >> 3, 0) = v;
                data = v as u16;
                dec_set_flags!();
            }

            0x03 | 0x13 | 0x23 => {
                // INC BC, DE, HL
                let v = r16!(opcode, 4, 0x03).wrapping_add(1);
                set_r16!(opcode, 4, 0x03, v);
                continue 'run;
            }

            0x33 => {
                // INC SP
                sp = sp.wrapping_add(1);
                continue 'run;
            }

            0x0B | 0x1B | 0x2B => {
                // DEC BC, DE, HL
                let v = r16!(opcode, 4, 0x0B).wrapping_sub(1);
                set_r16!(opcode, 4, 0x0B, v);
                continue 'run;
            }

            0x3B => {
                // DEC SP
                sp = sp.wrapping_sub(1);
                continue 'run;
            }

            // AND
            0xA6 => {
                // AND (HL)
                data = read!(hl!()) as u16;
                and_data!();
            }

            0xE6 => {
                // AND imm
                pc_add!(1);
                and_data!();
            }

            0xA0..=0xA5 | 0xA7 => {
                // AND r
                data = r8!(opcode, 0xA0) as u16;
                and_data!();
            }

            // OR
            0xB6 => {
                // OR (HL)
                data = read!(hl!()) as u16;
                or_data!();
            }

            0xF6 => {
                // OR imm
                pc_add!(1);
                or_data!();
            }

            0xB0..=0xB5 | 0xB7 => {
                // OR r
                data = r8!(opcode, 0xB0) as u16;
                or_data!();
            }

            // XOR
            0xAE => {
                // XOR (HL)
                data = read!(hl!()) as u16;
                xor_data!();
            }

            0xEE => {
                // XOR imm
                pc_add!(1);
                xor_data!();
            }

            0xA8..=0xAD | 0xAF => {
                // XOR r
                data = r8!(opcode, 0xA8) as u16;
                xor_data!();
            }

            // LD
            0x70..=0x75 | 0x77 => {
                // LD (HL),r
                let v = r8!(opcode, 0x70);
                write!(hl!(), v);
                continue 'run;
            }

            0x41..=0x45 | 0x47 // LD B,r
            | 0x48 | 0x4A..=0x4D | 0x4F // LD C,r
            | 0x50 | 0x51 | 0x53..=0x55 | 0x57 // LD D,r
            | 0x58..=0x5A | 0x5C | 0x5D | 0x5F // LD E,r
            | 0x60..=0x63 | 0x65 | 0x67 // LD H,r
            | 0x68..=0x6C | 0x6F // LD L,r
            | 0x78..=0x7D => {
                // LD A,r
                r8!(opcode >> 3 & 7, 0) = r8!(opcode & 7, 0);
                continue 'run;
            }

            0x06 | 0x0E | 0x16 | 0x1E | 0x26 => {
                // LD r,imm
                r8!(opcode >> 3, 0) = data as u8;
                pc_add!(1);
                continue 'run;
            }

            0x36 => {
                // LD (HL),imm
                pc_add!(1);
                write!(hl!(), data);
                continue 'run;
            }

            0x46 | 0x4E | 0x56 | 0x5E | 0x66 | 0x6E | 0x7E => {
                // LD r,(HL)
                let v = read!(hl!());
                r8!(opcode >> 3, 8) = v;
                continue 'run;
            }

            0x01 | 0x11 | 0x21 => {
                // LD rp,imm
                let v = get_addr!();
                set_r16!(opcode, 4, 0x01, v);
                pc_add!(2);
                continue 'run;
            }

            0x31 => {
                // LD sp,imm
                sp = get_addr!();
                pc_add!(2);
                continue 'run;
            }

            0x2A => {
                // LD HL,(addr)
                let addr: u16 = get_addr!();
                pc_add!(2);
                let v = read_word!(addr);
                set_pair!(RL, RH, v);
                continue 'run;
            }

            0x32 => {
                // LD (addr),A
                let addr: u16 = get_addr!();
                pc_add!(2);
                write!(addr, rg[RA]);
                continue 'run;
            }

            0x22 => {
                // LD (addr),HL
                let addr: u16 = get_addr!();
                pc_add!(2);
                write_word!(addr, hl!());
                continue 'run;
            }

            0x02 | 0x12 => {
                // LD (BC),A, LD (DE),A
                write!(r16!(opcode, 4, 0x02), rg[RA]);
                continue 'run;
            }

            0x0A | 0x1A => {
                // LD A,(BC), LD A,(DE)
                rg[RA] = read!(r16!(opcode, 4, 0x0A));
                continue 'run;
            }

            0xF9 => {
                // LD SP,HL
                sp = hl!();
                continue 'run;
            }

            // Rotate
            0x07 => {
                // RLCA
                let mut temp: u16 = rg[RA] as u16;
                temp = (temp << 1) | (temp >> 7);
                flags = (flags & (S80 | Z40 | P04)) | (temp as i32 & (F20 | F08 | C01));
                rg[RA] = temp as u8;
                continue 'run;
            }

            0x0F => {
                // RRCA
                let mut temp: u16 = rg[RA] as u16;
                flags = (flags & (S80 | Z40 | P04)) | (temp as i32 & C01);
                temp = (temp << 7) | (temp >> 1);
                flags |= temp as i32 & (F20 | F08);
                rg[RA] = temp as u8;
                continue 'run;
            }

            0x17 => {
                // RLA
                let temp: u32 = ((rg[RA] as u32) << 1) | (flags & C01) as u32;
                flags = (flags & (S80 | Z40 | P04)) | (temp as i32 & (F20 | F08)) | (temp >> 8) as i32;
                rg[RA] = temp as u8;
                continue 'run;
            }

            0x1F => {
                // RRA
                let temp: u16 = ((flags << 7) | (rg[RA] as i32 >> 1)) as u16;
                flags = (flags & (S80 | Z40 | P04))
                    | (temp as i32 & (F20 | F08))
                    | (rg[RA] as i32 & C01);
                rg[RA] = temp as u8;
                continue 'run;
            }

            // Misc
            0x2F => {
                // CPL
                let temp: u16 = !(rg[RA] as i32) as u16;
                flags = (flags & (S80 | Z40 | P04 | C01)) | (temp as i32 & (F20 | F08)) | (H10 | N02);
                rg[RA] = temp as u8;
                continue 'run;
            }

            0x3F => {
                // CCF
                flags = ((flags & (S80 | Z40 | P04 | C01)) ^ C01)
                    | (flags << 4 & H10)
                    | (rg[RA] as i32 & (F20 | F08));
                continue 'run;
            }

            0x37 => {
                // SCF
                flags = (flags & (S80 | Z40 | P04)) | C01 | (rg[RA] as i32 & (F20 | F08));
                continue 'run;
            }

            0xDB => {
                // IN A,(imm)
                pc_add!(1);
                rg[RA] = cpu_in!(data as u32 + rg[RA] as u32 * 0x100) as u8;
                continue 'run;
            }

            0xE3 => {
                // EX (SP),HL
                let temp: u16 = read_word!(sp);
                write_word!(sp, hl!());
                set_pair!(RL, RH, temp);
                continue 'run;
            }

            0xEB => {
                // EX DE,HL
                let temp: u16 = hl!();
                let de = de!();
                set_pair!(RL, RH, de);
                set_pair!(RE, RD, temp);
                continue 'run;
            }

            0xD9 => {
                // EXX DE,HL
                let cpu = h.cpu();
                for i in 0..3 {
                    let temp = cpu.r.alt.w(i);
                    let cur = rg[i * 2] as u16 | (rg[i * 2 + 1] as u16) << 8;
                    cpu.r.alt.set_w(i, cur);
                    rg[i * 2] = temp as u8;
                    rg[i * 2 + 1] = (temp >> 8) as u8;
                }
                continue 'run;
            }

            0xF3 => {
                // DI
                let cpu = h.cpu();
                cpu.r.iff1 = 0;
                cpu.r.iff2 = 0;
                continue 'run;
            }

            0xFB => {
                // EI
                let cpu = h.cpu();
                cpu.r.iff1 = 1;
                cpu.r.iff2 = 1;
                // TODO: delayed effect
                continue 'run;
            }

            0x76 => {
                // HALT
                break 'run Stop::Halt;
            }

            //////////////////////////////////////// CB prefix
            0xCB => {
                // (data2 = INSTR( 1 ), which goes unused: TODO is this the
                // same as data in all cases?)
                pc_add!(1);
                let (addr, reg): (u16, Option<usize>) = match data {
                    0x06 | 0x16 | 0x26 | 0x36 | 0x0E | 0x1E | 0x2E | 0x3E => {
                        s_time += 7;
                        (hl!(), None)
                    }
                    0x00..=0x3F => (0, Some(((data as usize & 7) ^ 1) as usize)),
                    0x46 | 0x4E | 0x56 | 0x5E | 0x66 | 0x6E | 0x76 | 0x7E => {
                        // BIT b,(HL)
                        s_time += 4;
                        let temp = read!(hl!()) as i32;
                        flags &= C01;
                        // bit_temp:
                        let masked: i32 = temp & 1 << (data >> 3 & 7);
                        flags |= (masked & S80) | H10 | ((masked - 1) >> 8 & (Z40 | P04));
                        continue 'run;
                    }
                    0x40..=0x7F => {
                        // BIT b,r
                        let temp = r8!(data & 7, 0) as i32;
                        flags = (flags & C01) | (temp & (F20 | F08));
                        // bit_temp:
                        let masked: i32 = temp & 1 << (data >> 3 & 7);
                        flags |= (masked & S80) | H10 | ((masked - 1) >> 8 & (Z40 | P04));
                        continue 'run;
                    }
                    0x86 | 0x8E | 0x96 | 0x9E | 0xA6 | 0xAE | 0xB6 | 0xBE | 0xC6 | 0xCE | 0xD6
                    | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE => {
                        // RES b,(HL), SET b,(HL)
                        s_time += 7;
                        let mut temp = read!(hl!()) as i32;
                        let bit = 1 << (data >> 3 & 7);
                        temp |= bit; // SET
                        if data & 0x40 == 0 {
                            temp ^= bit; // RES
                        }
                        write!(hl!(), temp);
                        continue 'run;
                    }
                    0xC0..=0xFF => {
                        // SET b,r
                        r8!(data & 7, 0) |= (1 << (data >> 3 & 7)) as u8;
                        continue 'run;
                    }
                    _ => {
                        // RES b,r
                        r8!(data & 7, 0) &= !((1 << (data >> 3 & 7)) as u8);
                        continue 'run;
                    }
                };
                // the rotates and shifts, on a register or on (HL) (or,
                // from the DD/FD CB prefix, (IXY+disp)): ..._data_addr
                let read_v: u8 = match reg {
                    Some(i) => rg[i],
                    None => read!(addr),
                };
                let result: u8 = cb_shift(&mut flags, data as u8 >> 3, read_v, &h.cpu().szpc);
                match reg {
                    Some(i) => rg[i] = result,
                    None => write!(addr, result),
                }
                continue 'run;
            }

            //////////////////////////////////////// ED prefix
            0xED => {
                pc_add!(1);
                s_time += (ED_DD_TIMING[data as usize] >> 4) as i32;
                match data {
                    0x72 | 0x7A | 0x42 | 0x52 | 0x62 | 0x4A | 0x5A | 0x6A => {
                        // SBC/ADC HL,SP, BC, DE, HL
                        let mut temp: u32 = if data == 0x72 || data == 0x7A {
                            sp as u32
                        } else {
                            r16!(data >> 3 & 6, 1, 0) as u32
                        };
                        let mut sum: u32 = temp.wrapping_add((flags & C01) as u32);
                        flags = (!(data as i32)) >> 2 & N02;
                        if flags != 0 {
                            sum = sum.wrapping_neg();
                        }
                        sum = sum.wrapping_add(hl!() as u32);
                        temp ^= hl!() as u32;
                        temp ^= sum;
                        flags |= (sum >> 16) as i32 & C01
                            | (temp >> 8) as i32 & H10
                            | (sum >> 8) as i32 & (S80 | F20 | F08)
                            | (temp.wrapping_add(0x8000) >> 14) as i32 & V04;
                        set_pair!(RL, RH, sum);
                        if sum as u16 != 0 {
                            continue 'run;
                        }
                        flags |= Z40;
                        continue 'run;
                    }

                    0x40 | 0x48 | 0x50 | 0x58 | 0x60 | 0x68 | 0x70 | 0x78 => {
                        // IN r,(C)
                        let temp = cpu_in!(bc!());
                        r8!(data >> 3, 8) = temp as u8;
                        flags = (flags & C01) | sz28p!(temp as u8);
                        continue 'run;
                    }

                    0x71 | 0x41 | 0x49 | 0x51 | 0x59 | 0x61 | 0x69 | 0x79 => {
                        if data == 0x71 {
                            // OUT (C),0
                            rg[RFLAGS] = 0;
                        }
                        // OUT (C),r
                        let v = r8!(data >> 3, 8);
                        cpu_out!(bc!(), v);
                        continue 'run;
                    }

                    0x73 | 0x43 | 0x53 => {
                        // LD (ADDR),SP, BC, DE
                        let temp: u32 = if data == 0x73 {
                            sp as u32
                        } else {
                            r16!(data, 4, 0x43) as u32
                        };
                        let addr: u16 = get_addr_at!(2);
                        pc_add!(2);
                        write_word!(addr, temp);
                        continue 'run;
                    }

                    0x4B | 0x5B => {
                        // LD BC,(ADDR), LD DE,(ADDR)
                        let addr: u16 = get_addr_at!(2);
                        pc_add!(2);
                        let v = read_word!(addr);
                        set_r16!(data, 4, 0x4B, v);
                        continue 'run;
                    }

                    0x7B => {
                        // LD SP,(ADDR)
                        let addr: u16 = get_addr_at!(2);
                        pc_add!(2);
                        sp = read_word!(addr);
                        continue 'run;
                    }

                    0x67 => {
                        // RRD
                        let mut temp: u8 = read!(hl!());
                        write!(hl!(), ((rg[RA] as i32) << 4) | (temp as i32 >> 4));
                        temp = (rg[RA] & 0xF0) | (temp & 0x0F);
                        flags = (flags & C01) | sz28p!(temp);
                        rg[RA] = temp;
                        continue 'run;
                    }

                    0x6F => {
                        // RLD
                        let mut temp: u8 = read!(hl!());
                        write!(hl!(), ((temp as i32) << 4) | (rg[RA] as i32 & 0x0F));
                        temp = (rg[RA] & 0xF0) | (temp >> 4);
                        flags = (flags & C01) | sz28p!(temp);
                        rg[RA] = temp;
                        continue 'run;
                    }

                    0x44 | 0x4C | 0x54 | 0x5C | 0x64 | 0x6C | 0x74 | 0x7C => {
                        // NEG
                        opcode = 0x10; // flag to do SBC instead of ADC
                        flags &= !C01;
                        data = rg[RA] as u16;
                        rg[RA] = 0;
                        adc_data!();
                    }

                    0xA9 | 0xB9 | 0xA1 | 0xB1 => {
                        // CPD, CPDR, CPI, CPIR
                        let inc: i32 = if data == 0xA9 || data == 0xB9 { -1 } else { 1 };
                        let addr: u16 = hl!();
                        set_pair!(RL, RH, (addr as i32 + inc) as u16);
                        let temp = read!(addr) as i32;

                        let mut result: i32 = rg[RA] as i32 - temp;
                        flags = (flags & C01) | N02 | ((((temp ^ rg[RA] as i32) & H10) ^ result) & (S80 | H10));

                        if result as u8 == 0 {
                            flags |= Z40;
                        }
                        result -= (flags & H10) >> 4;
                        flags |= result & F08;
                        flags |= result << 4 & F20;
                        let b = bc!().wrapping_sub(1);
                        set_pair!(RC, RB, b);
                        if b == 0 {
                            continue 'run;
                        }

                        flags |= V04;
                        if flags & Z40 != 0 || data < 0xB0 {
                            continue 'run;
                        }

                        pc_add!(-2);
                        s_time += 5;
                        continue 'run;
                    }

                    0xA8 | 0xB8 | 0xA0 | 0xB0 => {
                        // LDD, LDDR, LDI, LDIR
                        let inc: i32 = if data == 0xA8 || data == 0xB8 { -1 } else { 1 };
                        let mut addr: u16 = hl!();
                        set_pair!(RL, RH, (addr as i32 + inc) as u16);
                        let mut temp = read!(addr) as i32;

                        addr = de!();
                        set_pair!(RE, RD, (addr as i32 + inc) as u16);
                        write!(addr, temp);

                        temp += rg[RA] as i32;
                        flags = (flags & (S80 | Z40 | C01)) | (temp & F08) | (temp << 4 & F20);
                        let b = bc!().wrapping_sub(1);
                        set_pair!(RC, RB, b);
                        if b == 0 {
                            continue 'run;
                        }

                        flags |= V04;
                        if data < 0xB0 {
                            continue 'run;
                        }

                        pc_add!(-2);
                        s_time += 5;
                        continue 'run;
                    }

                    0xAB | 0xBB | 0xA3 | 0xB3 => {
                        // OUTD, OTDR, OUTI, OTIR
                        let inc: i32 = if data == 0xAB || data == 0xBB { -1 } else { 1 };
                        let addr: u16 = hl!();
                        set_pair!(RL, RH, (addr as i32 + inc) as u16);
                        let temp = read!(addr) as i32;

                        rg[RB] = rg[RB].wrapping_sub(1);
                        let b = rg[RB] as i32;
                        flags = (temp >> 6 & N02) | sz28!(b);
                        if b != 0 && data >= 0xB0 {
                            pc_add!(-2);
                            s_time += 5;
                        }

                        cpu_out!(bc!(), temp);
                        continue 'run;
                    }

                    0xAA | 0xBA | 0xA2 | 0xB2 => {
                        // IND, INDR, INI, INIR
                        let inc: i32 = if data == 0xAA || data == 0xBA { -1 } else { 1 };

                        let addr: u16 = hl!();
                        set_pair!(RL, RH, (addr as i32 + inc) as u16);

                        let temp = cpu_in!(bc!());

                        rg[RB] = rg[RB].wrapping_sub(1);
                        let b = rg[RB] as i32;
                        flags = (temp >> 6 & N02) | sz28!(b);
                        if b != 0 && data >= 0xB0 {
                            pc_add!(-2);
                            s_time += 5;
                        }

                        write!(addr, temp);
                        continue 'run;
                    }

                    0x47 => {
                        // LD I,A
                        h.cpu().r.i = rg[RA];
                        continue 'run;
                    }

                    0x4F => {
                        // LD R,A
                        h.cpu().r.r = rg[RA];
                        // debug_printf( "LD R,A not supported\n" );
                        warning = true;
                        continue 'run;
                    }

                    0x57 | 0x5F => {
                        if data == 0x57 {
                            // LD A,I
                            rg[RA] = h.cpu().r.i;
                        } else {
                            // LD A,R
                            rg[RA] = h.cpu().r.r;
                            // debug_printf( "LD A,R not supported\n" );
                            warning = true;
                        }
                        // ld_ai_common:
                        let iff2 = h.cpu().r.iff2 as i32;
                        flags = (flags & C01) | sz28!(rg[RA]) | (iff2 << 2 & V04);
                        continue 'run;
                    }

                    0x45 | 0x4D | 0x55 | 0x5D | 0x65 | 0x6D | 0x75 | 0x7D => {
                        // RETI/RETN
                        let cpu = h.cpu();
                        cpu.r.iff1 = cpu.r.iff2;
                        ret_taken!();
                    }

                    0x46 | 0x4E | 0x66 | 0x6E => {
                        // IM 0
                        h.cpu().r.im = 0;
                        continue 'run;
                    }

                    0x56 | 0x76 => {
                        // IM 1
                        h.cpu().r.im = 1;
                        continue 'run;
                    }

                    0x5E | 0x7E => {
                        // IM 2
                        h.cpu().r.im = 2;
                        continue 'run;
                    }

                    _ => {
                        // debug_printf( "Opcode $ED $%02X not supported\n", data );
                        warning = true;
                        continue 'run;
                    }
                }
            }

            //////////////////////////////////////// DD/FD prefix
            0xDD | 0xFD => {
                let mut ixy: u16 = if opcode == 0xDD { ix } else { iy };
                // ix_prefix:
                pc_add!(1);
                let mut data2: u32 = read_prog!(pc) as u32;
                s_time += (ED_DD_TIMING[data as usize] & 0x0F) as i32;

                // TODO: more efficient way of avoid negative address
                macro_rules! ixy_disp {
                    ($disp:expr) => {
                        (ixy as i32 + ($disp) as u8 as i8 as i32) as u16
                    };
                }
                macro_rules! set_ixy {
                    () => {{
                        if opcode == 0xDD {
                            ix = ixy;
                            continue 'run;
                        }
                        iy = ixy;
                        continue 'run;
                    }};
                }

                match data {
                    // ADD/ADC/SUB/SBC
                    0x96 | 0x86 | 0x9E | 0x8E => {
                        if data == 0x96 || data == 0x86 {
                            // SUB (IXY+disp), ADD (IXY+disp)
                            flags &= !C01;
                        }
                        // SBC (IXY+disp), ADC (IXY+disp)
                        pc_add!(1);
                        opcode = data as u8;
                        data = read!(ixy_disp!(data2)) as u16;
                        adc_data!();
                    }

                    0x94 | 0x84 | 0x9C | 0x8C => {
                        if data == 0x94 || data == 0x84 {
                            // SUB HXY, ADD HXY
                            flags &= !C01;
                        }
                        // SBC HXY, ADC HXY
                        opcode = data as u8;
                        data = ixy >> 8;
                        adc_data!();
                    }

                    0x95 | 0x85 | 0x9D | 0x8D => {
                        if data == 0x95 || data == 0x85 {
                            // SUB LXY, ADD LXY
                            flags &= !C01;
                        }
                        // SBC LXY, ADC LXY
                        opcode = data as u8;
                        data = ixy as u8 as u16;
                        adc_data!();
                    }

                    0x39 | 0x29 | 0x09 | 0x19 => {
                        let mut temp: u32 = match data {
                            0x39 => sp as u32,  // ADD IXY,SP
                            0x29 => ixy as u32, // ADD IXY,HL
                            _ => r16!(data, 4, 0x09) as u32, // ADD IXY,BC, ADD IXY,DE
                        };
                        // add_ixy_data:
                        let sum: u32 = ixy as u32 + temp;
                        temp ^= ixy as u32;
                        ixy = sum as u16;
                        flags = (flags & (S80 | Z40 | V04))
                            | (sum >> 16) as i32
                            | (sum >> 8) as i32 & (F20 | F08)
                            | ((temp ^ sum) >> 8) as i32 & H10;
                        set_ixy!();
                    }

                    // AND
                    0xA6 => {
                        // AND (IXY+disp)
                        pc_add!(1);
                        data = read!(ixy_disp!(data2)) as u16;
                        and_data!();
                    }

                    0xA4 => {
                        // AND HXY
                        data = ixy >> 8;
                        and_data!();
                    }

                    0xA5 => {
                        // AND LXY
                        data = ixy as u8 as u16;
                        and_data!();
                    }

                    // OR
                    0xB6 => {
                        // OR (IXY+disp)
                        pc_add!(1);
                        data = read!(ixy_disp!(data2)) as u16;
                        or_data!();
                    }

                    0xB4 => {
                        // OR HXY
                        data = ixy >> 8;
                        or_data!();
                    }

                    0xB5 => {
                        // OR LXY
                        data = ixy as u8 as u16;
                        or_data!();
                    }

                    // XOR
                    0xAE => {
                        // XOR (IXY+disp)
                        pc_add!(1);
                        data = read!(ixy_disp!(data2)) as u16;
                        xor_data!();
                    }

                    0xAC => {
                        // XOR HXY
                        data = ixy >> 8;
                        xor_data!();
                    }

                    0xAD => {
                        // XOR LXY
                        data = ixy as u8 as u16;
                        xor_data!();
                    }

                    // CP
                    0xBE => {
                        // CP (IXY+disp)
                        pc_add!(1);
                        data = read!(ixy_disp!(data2)) as u16;
                        cp_data!();
                    }

                    0xBC => {
                        // CP HXY
                        data = ixy >> 8;
                        cp_data!();
                    }

                    0xBD => {
                        // CP LXY
                        data = ixy as u8 as u16;
                        cp_data!();
                    }

                    // LD
                    0x70..=0x75 | 0x77 | 0x36 => {
                        if data == 0x36 {
                            // LD (IXY+disp),imm
                            pc_add!(1);
                            data = read_prog!(pc) as u16;
                        } else {
                            // LD (IXY+disp),r
                            data = r8!(data, 0x70) as u16;
                        }
                        pc_add!(1);
                        write!(ixy_disp!(data2), data);
                        continue 'run;
                    }

                    0x44 | 0x4C | 0x54 | 0x5C | 0x7C => {
                        // LD r,HXY
                        r8!(data >> 3, 8) = (ixy >> 8) as u8;
                        continue 'run;
                    }

                    0x64 | 0x6D => {
                        // LD HXY,HXY, LD LXY,LXY
                        continue 'run;
                    }

                    0x45 | 0x4D | 0x55 | 0x5D | 0x7D => {
                        // LD r,LXY
                        r8!(data >> 3, 8) = ixy as u8;
                        continue 'run;
                    }

                    0x46 | 0x4E | 0x56 | 0x5E | 0x66 | 0x6E | 0x7E => {
                        // LD r,(IXY+disp)
                        pc_add!(1);
                        let v = read!(ixy_disp!(data2));
                        r8!(data >> 3, 8) = v;
                        continue 'run;
                    }

                    0x26 | 0x65 | 0x60..=0x63 | 0x67 => {
                        match data {
                            0x26 => {
                                // LD HXY,imm
                                pc_add!(1);
                            }
                            0x65 => {
                                // LD HXY,LXY
                                data2 = ixy as u8 as u32;
                            }
                            _ => {
                                // LD HXY,r
                                data2 = r8!(data, 0x60) as u32;
                            }
                        }
                        // ld_hxy_data:
                        ixy = (ixy as u8 as u32 | (data2 << 8)) as u16;
                        set_ixy!();
                    }

                    0x2E | 0x6C | 0x68..=0x6B | 0x6F => {
                        match data {
                            0x2E => {
                                // LD LXY,imm
                                pc_add!(1);
                            }
                            0x6C => {
                                // LD LXY,HXY
                                data2 = (ixy >> 8) as u32;
                            }
                            _ => {
                                // LD LXY,r
                                data2 = r8!(data, 0x68) as u32;
                            }
                        }
                        // ld_lxy_data:
                        ixy = ((ixy as u32 & 0xFF00) | data2) as u16;
                        set_ixy!();
                    }

                    0xF9 => {
                        // LD SP,IXY
                        sp = ixy;
                        continue 'run;
                    }

                    0x22 => {
                        // LD (ADDR),IXY
                        let addr: u16 = get_addr_at!(2);
                        pc_add!(2);
                        write_word!(addr, ixy);
                        continue 'run;
                    }

                    0x21 => {
                        // LD IXY,imm
                        ixy = get_addr_at!(2);
                        pc_add!(2);
                        set_ixy!();
                    }

                    0x2A => {
                        // LD IXY,(addr)
                        let addr: u16 = get_addr_at!(2);
                        ixy = read_word!(addr);
                        pc_add!(2);
                        set_ixy!();
                    }

                    // DD/FD CB prefix
                    0xCB => {
                        data = ixy_disp!(data2);
                        pc_add!(1);
                        data2 = read_prog!(pc) as u32;
                        pc_add!(1);
                        match data2 {
                            0x06 | 0x16 | 0x26 | 0x36 | 0x0E | 0x1E | 0x2E | 0x3E => {
                                // RLC, RL, SLA, SLL, RRC, RR, SRA, SRL (IXY)
                                // (rlc_data_addr ... srl_data_addr)
                                let v = read!(data);
                                let result =
                                    cb_shift(&mut flags, (data2 >> 3) as u8, v, &h.cpu().szpc);
                                write!(data, result);
                                continue 'run;
                            }

                            0x46 | 0x4E | 0x56 | 0x5E | 0x66 | 0x6E | 0x76 | 0x7E => {
                                // BIT b,(IXY+disp)
                                let temp: u8 = read!(data);
                                let masked: i32 = temp as i32 & 1 << (data2 >> 3 & 7);
                                flags = (flags & C01)
                                    | H10
                                    | (masked & S80)
                                    | ((masked - 1) >> 8 & (Z40 | P04));
                                continue 'run;
                            }

                            0x86 | 0x8E | 0x96 | 0x9E | 0xA6 | 0xAE | 0xB6 | 0xBE | 0xC6
                            | 0xCE | 0xD6 | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE => {
                                // RES b,(IXY+disp), SET b,(IXY+disp)
                                let mut temp = read!(data) as i32;
                                let bit = 1 << (data2 >> 3 & 7);
                                temp |= bit; // SET
                                if data2 & 0x40 == 0 {
                                    temp ^= bit; // RES
                                }
                                write!(data, temp);
                                continue 'run;
                            }

                            _ => {
                                // debug_printf( "Opcode $%02X $CB $%02X not supported\n", opcode, data2 );
                                warning = true;
                                continue 'run;
                            }
                        }
                    }

                    // INC/DEC
                    0x23 => {
                        // INC IXY
                        ixy = ixy.wrapping_add(1);
                        set_ixy!();
                    }

                    0x2B => {
                        // DEC IXY
                        ixy = ixy.wrapping_sub(1);
                        set_ixy!();
                    }

                    0x34 => {
                        // INC (IXY+disp)
                        ixy = ixy_disp!(data2);
                        pc_add!(1);
                        data = read!(ixy) as u16 + 1;
                        write!(ixy, data);
                        inc_set_flags!();
                    }

                    0x35 => {
                        // DEC (IXY+disp)
                        ixy = ixy_disp!(data2);
                        pc_add!(1);
                        data = (read!(ixy) as u16).wrapping_sub(1);
                        write!(ixy, data);
                        dec_set_flags!();
                    }

                    0x24 | 0x2C => {
                        if data == 0x24 {
                            // INC HXY
                            ixy = ixy.wrapping_add(0x100);
                            data = ixy >> 8;
                        } else {
                            // INC LXY
                            data = ixy.wrapping_add(1) as u8 as u16;
                            ixy = (ixy & 0xFF00) | data;
                        }
                        // inc_xy_common:
                        if opcode == 0xDD {
                            ix = ixy;
                            inc_set_flags!();
                        }
                        iy = ixy;
                        inc_set_flags!();
                    }

                    0x25 | 0x2D => {
                        if data == 0x25 {
                            // DEC HXY
                            ixy = ixy.wrapping_sub(0x100);
                            data = ixy >> 8;
                        } else {
                            // DEC LXY
                            data = ixy.wrapping_sub(1) as u8 as u16;
                            ixy = (ixy & 0xFF00) | data;
                        }
                        // dec_xy_common:
                        if opcode == 0xDD {
                            ix = ixy;
                            dec_set_flags!();
                        }
                        iy = ixy;
                        dec_set_flags!();
                    }

                    // PUSH/POP
                    0xE5 => {
                        // PUSH IXY
                        data = ixy;
                        push_data!();
                    }

                    0xE1 => {
                        // POP IXY
                        ixy = read_word!(sp);
                        sp = sp.wrapping_add(2);
                        set_ixy!();
                    }

                    // Misc
                    0xE9 => {
                        // JP (IXY)
                        pc = ixy as u32;
                        continue 'run;
                    }

                    0xE3 => {
                        // EX (SP),IXY
                        let temp: u16 = read_word!(sp);
                        write_word!(sp, ixy);
                        ixy = temp;
                        set_ixy!();
                    }

                    _ => {
                        // debug_printf( "Unnecessary DD/FD prefix encountered\n" );
                        warning = true;
                        pc_add!(-1);
                        continue 'run;
                    }
                }
            }
        }
    };

    match stop {
        Stop::HitIdleAddr => s_time -= 11,
        Stop::Halt => s_time &= 3, // increment by multiple of 4
        Stop::OutOfTime => {}
    }
    // out_of_time:
    pc_add!(-1);

    let cpu = h.cpu();
    cpu.time = s_time;
    rg[RFLAGS] = flags as u8;
    cpu.r.ix = ix;
    cpu.r.iy = iy;
    cpu.r.sp = sp;
    cpu.r.pc = pc as u16;
    cpu.r.b = Regs(rg);
    cpu.base = s_base;

    warning
}

/// The CB prefix's rotates and shifts (`RLC()`, `RL()`, `SLA()`, `RRC()`,
/// `RR()`, `SRA()`, `SRL()`, by bits 3-5 of the opcode), on `read`,
/// setting `flags`; returns the result to write.
#[inline]
fn cb_shift(flags: &mut i32, op: u8, read: u8, szpc: &[u8; 0x200]) -> u8 {
    let sz28p = |n: u32| szpc[n as usize] as i32;
    match op & 7 {
        0 => {
            // RLC
            let mut result: u8 = read;
            result = (result << 1) | (result >> 7);
            *flags = sz28p(result as u32) | (result as i32 & C01);
            result
        }
        1 => {
            // RRC
            let mut result: u8 = read;
            *flags = result as i32 & C01;
            result = (result << 7) | (result >> 1);
            *flags |= sz28p(result as u32);
            result
        }
        2 => {
            // RL
            let result: u16 = ((read as u16) << 1) | (*flags & C01) as u16;
            *flags = sz28p(result as u32); // SZ28PC
            result as u8
        }
        3 => {
            // RR
            let mut result: u8 = read;
            let temp: u8 = result & C01 as u8;
            result = ((*flags << 7) as u8) | (result >> 1);
            *flags = sz28p(result as u32) | temp as i32;
            result
        }
        4 | 6 => {
            // SLA, SLL
            let add: u16 = (op & 7 == 6) as u16;
            let result: u16 = ((read as u16) << 1) | add;
            *flags = sz28p(result as u32); // SZ28PC
            result as u8
        }
        5 => {
            // SRA
            let mut result: u8 = read;
            *flags = result as i32 & C01;
            result = (result & 0x80) | (result >> 1);
            *flags |= sz28p(result as u32);
            result
        }
        _ => {
            // SRL
            let mut result: u8 = read;
            *flags = result as i32 & C01;
            result >>= 1;
            *flags |= sz28p(result as u32);
            result
        }
    }
}
