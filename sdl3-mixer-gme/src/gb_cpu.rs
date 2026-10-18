// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/

//#include "gb_cpu_log.h"

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

//! Nintendo Game Boy CPU emulator (`Gb_Cpu.h`, `Gb_Cpu.cpp`).
//! Treats every instruction as taking 4 cycles.
//!
//! The code map holds [`CodePage`]s, places in the memory regions of the
//! emulator using the CPU (upstream holds pointers), and [`run`] calls the
//! emulator's memory handlers (`gb_cpu_io.h`) through [`GbCpuHost`].
//! Upstream runs on a local copy of its state, which the memory handlers
//! see (through its `state` pointer) and which it copies back when it
//! stops; this runs on the state itself.

pub type GbAddr = u32; // 16-bit CPU address

const CLOCKS_PER_INSTR: i32 = 4;

/// Map code memory (memory accessed via the program counter). Start and size
/// must be multiple of page_size.
pub const PAGE_SIZE: u32 = 0x2000;
pub const PAGE_SHIFT: u32 = 13;
pub const PAGE_COUNT: usize = 0x10000 >> PAGE_SHIFT;

/// If CPU executes opcode 0xFF at this address, it treats as illegal instruction
pub const IDLE_ADDR: u32 = 0xF00D;

/// Can read this many bytes past end of a page
pub const CPU_PADDING: usize = 8;

/// A place in one of the emulator's memory regions: the region and the
/// offset in it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CodePage {
    pub mem: u8,
    pub offset: usize,
}

// (the core registers, in upstream's little-endian layout: c, b, e, d, l,
// h, a, flags, which its R8() and register pair views index)
const RC: usize = 0;
const RB: usize = 1;
const RE: usize = 2;
const RD: usize = 3;
const RL: usize = 4;
const RH: usize = 5;
const RA: usize = 6;
const RFLAGS: usize = 7;

/// Game Boy Z80 registers. *Not* kept updated during a call to run().
#[derive(Clone, Copy, Debug, Default)]
pub struct Registers {
    /// `core_regs_t`: c, b, e, d, l, h, a, flags
    pub core: [u8; 8],
    pub pc: i64, // more than 16 bits to allow overflow detection
    pub sp: u16,
}

impl Registers {
    pub fn a(&self) -> u8 {
        self.core[RA]
    }
    pub fn set_a(&mut self, v: u8) {
        self.core[RA] = v;
    }
    pub fn flags(&self) -> u8 {
        self.core[RFLAGS]
    }
}

#[derive(Clone, Debug)]
pub struct GbCpu {
    pub r: Registers,

    // Interrupt enable flag set by EI and cleared by DI
    //bool interrupts_enabled; // unused
    /// Base address for RST vectors (normally 0)
    pub rst_base: GbAddr,

    // state_t
    code_map: [CodePage; PAGE_COUNT + 1],
    remain: i32,
}

impl Default for GbCpu {
    fn default() -> Self {
        GbCpu {
            r: Registers::default(),
            rst_base: 0,
            code_map: [CodePage::default(); PAGE_COUNT + 1],
            remain: 0,
        }
    }
}

/// The memory handlers of the emulator running the CPU.
pub trait GbCpuHost {
    fn cpu(&mut self) -> &mut GbCpu;
    /// The byte at `offset` in memory region `mem`.
    fn code_byte(&self, mem: u8, offset: usize) -> u8;
    /// `CPU_READ()`
    fn cpu_read(&mut self, addr: GbAddr) -> i32;
    /// `CPU_WRITE()`
    fn cpu_write(&mut self, addr: GbAddr, data: i32);
}

impl GbCpu {
    pub fn new() -> Self {
        Self::default()
    }

    fn set_code_page(&mut self, i: usize, p: CodePage) {
        // (minus PAGE_OFFSET( i * page_size ), which is 0)
        self.code_map[i] = p;
    }

    /// Clear registers and map all pages to unmapped
    pub fn reset(&mut self, unmapped: CodePage) {
        self.remain = 0;

        for i in 0..PAGE_COUNT + 1 {
            self.set_code_page(i, unmapped);
        }

        self.r = Registers::default();
        //interrupts_enabled = false;
    }

    /// Map code memory (memory accessed via the program counter). Start and size
    /// must be multiple of page_size.
    pub fn map_code(&mut self, start: GbAddr, size: u32, data: CodePage) {
        // address range must begin and end on page boundaries
        // require( start % page_size == 0 );
        // require( size % page_size == 0 );

        let first_page = (start / PAGE_SIZE) as usize;
        let mut i = size / PAGE_SIZE;
        while i > 0 {
            i -= 1;
            self.set_code_page(
                first_page + i as usize,
                CodePage {
                    mem: data.mem,
                    offset: data.offset + (i * PAGE_SIZE) as usize,
                },
            );
        }
    }

    /// `get_code()`: the region and offset of the byte at `addr`
    pub fn get_code(&self, addr: GbAddr) -> CodePage {
        let p = self.code_map[page_index(addr)];
        CodePage {
            mem: p.mem,
            offset: p.offset + (addr % PAGE_SIZE) as usize,
        }
    }

    /// Number of clock cycles remaining for most recent run() call
    pub fn remain(&self) -> i32 {
        self.remain.wrapping_mul(CLOCKS_PER_INSTR)
    }
}

/// The code map entry for an address. Note (upstream): the program
/// counter isn't kept to 16 bits while running (only when the CPU stops),
/// so code running on past 0xFFFF (through the unmapped page after it) goes
/// on to index past the code map; this maps the addresses past it to that
/// unmapped page too (upstream's build for the tests is patched the same).
#[inline]
fn page_index(addr: u32) -> usize {
    ((addr >> PAGE_SHIFT) as usize).min(PAGE_COUNT)
}

const Z_FLAG: u32 = 0x80;
const N_FLAG: u32 = 0x40;
const H_FLAG: u32 = 0x20;
const C_FLAG: u32 = 0x10;

/// Run CPU for at least 'count' cycles and return false, or return true if
/// illegal instruction is encountered.
pub fn run<H: GbCpuHost + ?Sized>(h: &mut H, cycle_count: i32) -> bool {
    h.cpu().remain = ((cycle_count + CLOCKS_PER_INSTR) as u32 / CLOCKS_PER_INSTR as u32) as i32;

    // (the union of the individual registers, the pairs and the indexed
    // registers)
    let r = h.cpu().r;
    let mut rg: [u8; 8] = r.core;
    let mut pc: u32 = r.pc as u32;
    let mut sp: u32 = r.sp as u32;
    let mut flags: u32 = r.core[RFLAGS] as u32;

    // R8( n ): (r8_ [(n) ^ 1]) on a little-endian machine
    macro_rules! r8 {
        ($n:expr) => {
            rg[(($n) as usize) ^ 1]
        };
    }
    macro_rules! r16 {
        ($i:expr) => {{
            let i = ($i) as usize * 2;
            rg[i] as u32 | (rg[i + 1] as u32) << 8
        }};
    }
    macro_rules! set_r16 {
        ($i:expr, $v:expr) => {{
            let i = ($i) as usize * 2;
            let v = ($v) as u32;
            rg[i] = v as u8;
            rg[i + 1] = (v >> 8) as u8;
        }};
    }
    macro_rules! read {
        ($addr:expr) => {
            h.cpu_read(($addr) as GbAddr) as u32
        };
    }
    macro_rules! write {
        ($addr:expr, $data:expr) => {
            h.cpu_write(($addr) as GbAddr, ($data) as i32)
        };
    }
    // (CPU_READ_FAST: the code byte, or the APU's register, which is what
    // cpu_read() returns)
    macro_rules! read_fast {
        ($addr:expr) => {
            h.cpu_read(($addr) as GbAddr) as u8
        };
    }

    'stop: loop {
        // loop:

        // check( (unsigned long) pc < 0x10000 );
        // check( (unsigned long) sp < 0x10000 );
        // check( (flags & ~0xF0) == 0 );

        let page = h.cpu().code_map[page_index(pc)];
        let mut op: u32;

        // TODO: eliminate this special case
        let instr = page.offset + (pc & (PAGE_SIZE - 1)) as usize;
        op = h.code_byte(page.mem, instr) as u32;
        let instr = instr + 1;
        pc = pc.wrapping_add(1);

        macro_rules! get_addr {
            () => {
                h.code_byte(page.mem, instr) as u32 | (h.code_byte(page.mem, instr + 1) as u32) << 8
            };
        }

        {
            let cpu = h.cpu();
            cpu.remain -= 1;
            if cpu.remain == 0 {
                break 'stop;
            }
        }

        let mut data: u32 = h.code_byte(page.mem, instr) as u32;

        // TODO: more efficient way to handle negative branch that wraps PC around
        macro_rules! branch {
            ($cond:expr) => {{
                pc = pc.wrapping_add(1);
                let offset = data as u8 as i8 as i32;
                if !($cond) {
                    continue 'stop;
                }
                pc = (pc as i32).wrapping_add(offset) as u16 as u32;
                continue 'stop;
            }};
        }
        macro_rules! sub_set_flags {
            () => {{
                flags = (op & 15).wrapping_sub(data & 15) & H_FLAG;
                flags |= (data >> 4) & C_FLAG;
                flags |= N_FLAG;
                if data & 0xFF != 0 {
                    continue 'stop;
                }
                flags |= Z_FLAG;
                continue 'stop;
            }};
        }
        macro_rules! cmp_comm {
            () => {{
                op = rg[RA] as u32;
                data = op.wrapping_sub(data);
                sub_set_flags!();
            }};
        }
        macro_rules! push {
            () => {{
                sp = sp.wrapping_sub(1) & 0xFFFF;
                write!(sp, data >> 8);
                sp = sp.wrapping_sub(1) & 0xFFFF;
                write!(sp, data & 0xFF);
                continue 'stop;
            }};
        }
        macro_rules! call {
            () => {{
                pc = pc.wrapping_sub(2);
                // case 0xCD:
                data = pc.wrapping_add(2);
                pc = get_addr!();
                push!();
            }};
        }
        macro_rules! ret {
            () => {{
                pc = read!(sp);
                pc = pc.wrapping_add(0x100u32.wrapping_mul(read!(sp + 1)));
                sp = (sp + 2) & 0xFFFF;
                continue 'stop;
            }};
        }
        macro_rules! write_hl_op_ff {
            () => {{
                write!(r16!(2), op & 0xFF);
                continue 'stop;
            }};
        }
        macro_rules! shift_comm {
            () => {{
                data &= 7;
                if op & 0xFF == 0 {
                    flags |= Z_FLAG;
                }
                if data == 6 {
                    write_hl_op_ff!();
                }
                r8!(data) = op as u8;
                continue 'stop;
            }};
        }
        macro_rules! rl_comm {
            () => {{
                op <<= 1;
                op |= ((data & flags) >> 4) & 1; // RL and carry is set
                flags = (op >> 4) & C_FLAG; // C = bit shifted out
                if data < 0x10 {
                    // RLC
                    op |= op >> 8;
                }
                // SLA doesn't fill lower bit
                shift_comm!();
            }};
        }
        macro_rules! rr_comm {
            () => {{
                op |= (data & flags) << 4; // RR and carry is set
                flags = (op << 4) & C_FLAG; // C = bit shifted out
                if data < 0x10 {
                    // RRC
                    op |= op << 8;
                }
                op >>= 1;
                if data & 0x20 != 0 {
                    // SRA propagates sign bit
                    op |= (op << 1) & 0x80;
                }
                shift_comm!();
            }};
        }
        macro_rules! ld_a_ind_comm {
            ($temp:expr) => {{
                let temp = $temp;
                rg[RA] = read_fast!(temp);
                continue 'stop;
            }};
        }
        macro_rules! write_data_rg_a {
            ($temp:expr) => {{
                let temp = $temp;
                write!(temp, rg[RA]);
                continue 'stop;
            }};
        }
        macro_rules! inc_comm {
            () => {{
                flags = (flags & C_FLAG)
                    | ((data & 15).wrapping_sub(1) & H_FLAG)
                    | ((data >> 1) & Z_FLAG);
                continue 'stop;
            }};
        }
        macro_rules! dec_comm {
            () => {{
                flags = (flags & C_FLAG) | N_FLAG | (((data & 15) + 0x31) & H_FLAG);
                if data & 0xFF != 0 {
                    continue 'stop;
                }
                flags |= Z_FLAG;
                continue 'stop;
            }};
        }
        macro_rules! add_16_comm {
            ($temp:expr, $prev:expr) => {{
                let temp: u32 = $temp;
                let prev: u32 = $prev;
                flags |= (temp >> 12) & C_FLAG;
                flags |= ((temp & 0x0FFF).wrapping_sub(prev & 0x0FFF) >> 7) & H_FLAG;
                continue 'stop;
            }};
        }
        macro_rules! add_16_hl {
            ($temp:expr, $prev:expr) => {{
                let temp: u32 = $temp;
                set_r16!(2, temp);
                add_16_comm!(temp, $prev);
            }};
        }
        macro_rules! add_hl_comm {
            ($temp:expr) => {{
                let mut temp: u32 = $temp;
                let prev = r16!(2);
                temp = temp.wrapping_add(prev);
                flags &= Z_FLAG;
                add_16_hl!(temp, prev);
            }};
        }
        macro_rules! add_comm {
            () => {{
                flags = rg[RA] as u32;
                data = data.wrapping_add(flags);
                flags = (data & 15).wrapping_sub(flags & 15) & H_FLAG;
                flags |= (data >> 4) & C_FLAG;
                rg[RA] = data as u8;
                if data & 0xFF != 0 {
                    continue 'stop;
                }
                flags |= Z_FLAG;
                continue 'stop;
            }};
        }
        macro_rules! adc_comm {
            () => {{
                data = data.wrapping_add((flags >> 4) & 1);
                data &= 0xFF; // to do: does carry get set when sum + carry = 0x100?
                add_comm!();
            }};
        }
        macro_rules! sub_comm {
            () => {{
                op = rg[RA] as u32;
                data = op.wrapping_sub(data);
                rg[RA] = data as u8;
                sub_set_flags!();
            }};
        }
        macro_rules! sbc_comm {
            () => {{
                data = data.wrapping_add((flags >> 4) & 1);
                data &= 0xFF; // to do: does carry get set when sum + carry = 0x100?
                sub_comm!();
            }};
        }
        macro_rules! and_a {
            () => {{
                // case 0xA7: // AND A
                flags = H_FLAG | ((((rg[RA] as i32) - 1) >> 1) as u32 & Z_FLAG);
                continue 'stop;
            }};
        }
        macro_rules! and_comm {
            () => {{
                rg[RA] &= data as u8; // FALLTHRU
                and_a!();
            }};
        }
        macro_rules! or_a {
            () => {{
                // case 0xB7: // OR A
                flags = (((rg[RA] as i32) - 1) >> 1) as u32 & Z_FLAG;
                continue 'stop;
            }};
        }
        macro_rules! or_comm {
            () => {{
                rg[RA] |= data as u8; // FALLTHRU
                or_a!();
            }};
        }
        macro_rules! xor_comm {
            () => {{
                data ^= rg[RA] as u32;
                rg[RA] = data as u8;
                data = data.wrapping_sub(1);
                flags = (data >> 1) & Z_FLAG;
                continue 'stop;
            }};
        }
        macro_rules! jp_taken {
            () => {{
                // (pc -= 2, which the address read from instr overrides)
                pc = get_addr!();
                continue 'stop;
            }};
        }

        match op {
            // Most Common
            0x20 => {
                // JR NZ
                branch!(flags & Z_FLAG == 0)
            }

            0x21 => {
                // LD HL,IMM (common)
                set_r16!(2, get_addr!());
                pc = pc.wrapping_add(2);
                continue 'stop;
            }

            0x28 => {
                // JR Z
                branch!(flags & Z_FLAG != 0)
            }

            0xF0 => {
                // LD A,(0xFF00+imm)
                let temp = data | 0xFF00;
                pc = pc.wrapping_add(1);
                ld_a_ind_comm!(temp);
            }

            0xF2 => {
                // LD A,(0xFF00+C)
                ld_a_ind_comm!(rg[RC] as u32 | 0xFF00);
            }

            0x0A => {
                // LD A,(BC)
                ld_a_ind_comm!(r16!(0));
            }

            0x3A => {
                // LD A,(HL-)
                let temp = r16!(2);
                set_r16!(2, temp.wrapping_sub(1));
                ld_a_ind_comm!(temp);
            }

            0x1A => {
                // LD A,(DE)
                ld_a_ind_comm!(r16!(1));
            }

            0x2A => {
                // LD A,(HL+) (common)
                let temp = r16!(2);
                set_r16!(2, temp + 1);
                ld_a_ind_comm!(temp);
            }

            0xFA => {
                // LD A,IND16 (common)
                let temp = get_addr!();
                pc = pc.wrapping_add(2);
                ld_a_ind_comm!(temp);
            }

            0xBE => {
                // CMP (HL)
                data = read!(r16!(2));
                cmp_comm!();
            }

            0xB8..=0xBD => {
                // CMP B, C, D, E, H, L
                data = r8!(op & 7) as u32;
                cmp_comm!();
            }

            0xFE => {
                // CMP IMM
                pc = pc.wrapping_add(1);
                cmp_comm!();
            }

            0x46 | 0x4E | 0x56 | 0x5E | 0x66 | 0x6E | 0x7E => {
                // LD B,(HL) ... LD L,(HL), LD A,(HL)
                let addr = r16!(2);
                let v = read_fast!(addr);
                r8!((op >> 3) & 7) = v;
                continue 'stop;
            }

            0xC4 => {
                // CNZ (next-most-common)
                pc = pc.wrapping_add(2);
                if flags & Z_FLAG != 0 {
                    continue 'stop;
                }
                call!();
            }
            0xCD => {
                // CALL (most-common)
                data = pc.wrapping_add(2);
                pc = get_addr!();
                push!();
            }

            0xC8 => {
                // RNZ (next-most-common)
                if flags & Z_FLAG == 0 {
                    continue 'stop;
                }
                // FALLTHRU
                ret!();
            }
            0xC9 => {
                // RET (most common)
                ret!();
            }

            0x00 | 0x40 | 0x49 | 0x52 | 0x5B | 0x64 | 0x6D | 0x7F => {
                // NOP, LD B,B ... LD A,A
                continue 'stop;
            }

            // CB Instructions
            0xCB => {
                pc = pc.wrapping_add(1);
                // now data is the opcode
                match data {
                    0x40..=0x7F => {
                        // BIT b,(HL) (0x46, 0x4E ... 0x7E) / BIT b,r
                        let temp: i32 = if data & 7 == 6 {
                            let addr = r16!(2);
                            read_fast!(addr) as i32
                        } else {
                            r8!(data & 7) as i32
                        };
                        // bit_comm:
                        let bit = (!data >> 3) & 7;
                        flags &= !N_FLAG;
                        flags |= H_FLAG | Z_FLAG;
                        flags ^= (temp << bit) as u32 & Z_FLAG;
                        continue 'stop;
                    }

                    0x86 | 0x8E | 0x96 | 0x9E | 0xA6 | 0xAE | 0xB6 | 0xBE | 0xC6 | 0xCE
                    | 0xD6 | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE => {
                        // RES b,(HL) / SET b,(HL)
                        let mut temp = read!(r16!(2)) as i32;
                        let mut bit: i32 = 1 << ((data >> 3) & 7);
                        temp &= !bit;
                        if data & 0x40 == 0 {
                            bit = 0;
                        }
                        write!(r16!(2), temp | bit);
                        continue 'stop;
                    }

                    0xC0..=0xFF => {
                        // SET b,r
                        r8!(data & 7) |= (1u32 << ((data >> 3) & 7)) as u8;
                        continue 'stop;
                    }

                    0x80..=0xBF => {
                        // RES b,r
                        r8!(data & 7) &= !((1u32 << ((data >> 3) & 7)) as u8);
                        continue 'stop;
                    }

                    0x30..=0x37 => {
                        // SWAP (HL) / SWAP r
                        let temp: u32 = if data == 0x36 {
                            read!(r16!(2))
                        } else {
                            r8!(data & 7) as u32
                        };
                        // swap_comm:
                        op = (temp >> 4) | (temp << 4);
                        flags = 0;
                        shift_comm!();
                    }

                    // Shift/Rotate
                    0x06 | 0x16 | 0x26 => {
                        // RLC (HL), RL (HL), SLA (HL)
                        op = read!(r16!(2));
                        rl_comm!();
                    }

                    0x20..=0x27 | 0x00..=0x07 | 0x10..=0x17 => {
                        // SLA r, RLC r, RL r
                        op = r8!(data & 7) as u32;
                        rl_comm!();
                    }

                    0x3E | 0x1E | 0x0E | 0x2E => {
                        if data == 0x3E {
                            // SRL (HL)
                            data += 0x10; // bump up to 0x4n to avoid preserving sign bit
                        }
                        // RR (HL), RRC (HL), SRA (HL)
                        op = read!(r16!(2));
                        rr_comm!();
                    }

                    0x38..=0x3F | 0x18..=0x1F | 0x08..=0x0F | 0x28..=0x2F => {
                        if (0x38..=0x3F).contains(&data) {
                            // SRL r
                            data += 0x10; // bump up to 0x4n
                        }
                        // RR r, RRC r, SRA r
                        op = r8!(data & 7) as u32;
                        rr_comm!();
                    }

                    _ => {
                        // assert( false ); // unhandled CB op
                        // (data is a byte: every CB op is handled)
                        continue 'stop;
                    }
                } // CB op
            }

            0x07 | 0x17 => {
                // RLCA, RLA
                data = op;
                op = rg[RA] as u32;
                rl_comm!();
            }

            0x0F | 0x1F => {
                // RRCA, RRA
                data = op;
                op = rg[RA] as u32;
                rr_comm!();
            }

            // Load
            0x70..=0x75 | 0x77 => {
                // LD (HL),B ... LD (HL),L, LD (HL),A
                op = r8!(op & 7) as u32;
                write_hl_op_ff!();
            }

            0x41..=0x45
            | 0x47
            | 0x48
            | 0x4A..=0x4D
            | 0x4F
            | 0x50
            | 0x51
            | 0x53..=0x55
            | 0x57
            | 0x58..=0x5A
            | 0x5C
            | 0x5D
            | 0x5F
            | 0x60..=0x63
            | 0x65
            | 0x67
            | 0x68..=0x6C
            | 0x6F
            | 0x78..=0x7D => {
                // LD r,r
                r8!((op >> 3) & 7) = r8!(op & 7);
                continue 'stop;
            }

            0x08 => {
                // LD IND16,SP
                data = get_addr!();
                pc = pc.wrapping_add(2);
                write!(data, sp & 0xFF);
                data += 1;
                write!(data, sp >> 8);
                continue 'stop;
            }

            0xF9 => {
                // LD SP,HL
                sp = r16!(2);
                continue 'stop;
            }

            0x31 => {
                // LD SP,IMM
                sp = get_addr!();
                pc = pc.wrapping_add(2);
                continue 'stop;
            }

            0x01 | 0x11 => {
                // LD BC,IMM, LD DE,IMM
                set_r16!(op >> 4, get_addr!());
                pc = pc.wrapping_add(2);
                continue 'stop;
            }

            0xE0 => {
                // LD (0xFF00+imm),A
                let temp = data | 0xFF00;
                pc = pc.wrapping_add(1);
                write_data_rg_a!(temp);
            }

            0xE2 => {
                // LD (0xFF00+C),A
                write_data_rg_a!(rg[RC] as u32 | 0xFF00);
            }

            0x32 => {
                // LD (HL-),A
                let temp = r16!(2);
                set_r16!(2, temp.wrapping_sub(1));
                write_data_rg_a!(temp);
            }

            0x02 => {
                // LD (BC),A
                write_data_rg_a!(r16!(0));
            }

            0x12 => {
                // LD (DE),A
                write_data_rg_a!(r16!(1));
            }

            0x22 => {
                // LD (HL+),A
                let temp = r16!(2);
                set_r16!(2, temp + 1);
                write_data_rg_a!(temp);
            }

            0xEA => {
                // LD IND16,A (common)
                let temp = get_addr!();
                pc = pc.wrapping_add(2);
                write_data_rg_a!(temp);
            }

            0x06 => {
                // LD B,IMM
                rg[RB] = data as u8;
                pc = pc.wrapping_add(1);
                continue 'stop;
            }

            0x0E => {
                // LD C,IMM
                rg[RC] = data as u8;
                pc = pc.wrapping_add(1);
                continue 'stop;
            }

            0x16 => {
                // LD D,IMM
                rg[RD] = data as u8;
                pc = pc.wrapping_add(1);
                continue 'stop;
            }

            0x1E => {
                // LD E,IMM
                rg[RE] = data as u8;
                pc = pc.wrapping_add(1);
                continue 'stop;
            }

            0x26 => {
                // LD H,IMM
                rg[RH] = data as u8;
                pc = pc.wrapping_add(1);
                continue 'stop;
            }

            0x2E => {
                // LD L,IMM
                rg[RL] = data as u8;
                pc = pc.wrapping_add(1);
                continue 'stop;
            }

            0x36 => {
                // LD (HL),IMM
                write!(r16!(2), data);
                pc = pc.wrapping_add(1);
                continue 'stop;
            }

            0x3E => {
                // LD A,IMM
                rg[RA] = data as u8;
                pc = pc.wrapping_add(1);
                continue 'stop;
            }

            // Increment/Decrement
            0x03 | 0x13 | 0x23 => {
                // INC BC, INC DE, INC HL
                set_r16!(op >> 4, r16!(op >> 4).wrapping_add(1));
                continue 'stop;
            }

            0x33 => {
                // INC SP
                sp = (sp + 1) & 0xFFFF;
                continue 'stop;
            }

            0x0B | 0x1B | 0x2B => {
                // DEC BC, DEC DE, DEC HL
                set_r16!(op >> 4, r16!(op >> 4).wrapping_sub(1));
                continue 'stop;
            }

            0x3B => {
                // DEC SP
                sp = sp.wrapping_sub(1) & 0xFFFF;
                continue 'stop;
            }

            0x34 => {
                // INC (HL)
                op = r16!(2);
                data = read!(op);
                data = data.wrapping_add(1);
                write!(op, data & 0xFF);
                inc_comm!();
            }

            0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x3C => {
                // INC B, INC C (common), INC D, INC E, INC H, INC L, INC A
                op = (op >> 3) & 7;
                data = (r8!(op) as u32).wrapping_add(1);
                r8!(op) = data as u8;
                inc_comm!();
            }

            0x35 => {
                // DEC (HL)
                op = r16!(2);
                data = read!(op);
                data = data.wrapping_sub(1);
                write!(op, data & 0xFF);
                dec_comm!();
            }

            0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x3D => {
                // DEC B ... DEC L, DEC A
                op = (op >> 3) & 7;
                data = (r8!(op) as u32).wrapping_sub(1);
                r8!(op) = data as u8;
                dec_comm!();
            }

            // Add 16-bit
            0xF8 => {
                // LD HL,SP+imm
                let mut temp = data as u8 as i8 as i32 as u32; // sign-extend to 16 bits
                pc = pc.wrapping_add(1);
                flags = 0;
                temp = temp.wrapping_add(sp);
                let prev = sp;
                add_16_hl!(temp, prev);
            }

            0xE8 => {
                // ADD SP,IMM
                let mut temp = data as u8 as i8 as i32 as u32; // sign-extend to 16 bits
                pc = pc.wrapping_add(1);
                flags = 0;
                temp = temp.wrapping_add(sp);
                let prev = sp;
                sp = temp & 0xFFFF;
                add_16_comm!(temp, prev);
            }

            0x39 => {
                // ADD HL,SP
                add_hl_comm!(sp);
            }

            0x09 | 0x19 | 0x29 => {
                // ADD HL,BC, ADD HL,DE, ADD HL,HL
                add_hl_comm!(r16!(op >> 4));
            }

            0x86 => {
                // ADD (HL)
                data = read!(r16!(2));
                add_comm!();
            }

            0x80..=0x85 | 0x87 => {
                // ADD B ... ADD L, ADD A
                data = r8!(op & 7) as u32;
                add_comm!();
            }

            0xC6 => {
                // ADD IMM
                pc = pc.wrapping_add(1);
                add_comm!();
            }

            // Add/Subtract
            0x8E => {
                // ADC (HL)
                data = read!(r16!(2));
                adc_comm!();
            }

            0x88..=0x8D | 0x8F => {
                // ADC B ... ADC L, ADC A
                data = r8!(op & 7) as u32;
                adc_comm!();
            }

            0xCE => {
                // ADC IMM
                pc = pc.wrapping_add(1);
                adc_comm!();
            }

            0x96 => {
                // SUB (HL)
                data = read!(r16!(2));
                sub_comm!();
            }

            0x90..=0x95 | 0x97 => {
                // SUB B ... SUB L, SUB A
                data = r8!(op & 7) as u32;
                sub_comm!();
            }

            0xD6 => {
                // SUB IMM
                pc = pc.wrapping_add(1);
                sub_comm!();
            }

            0x9E => {
                // SBC (HL)
                data = read!(r16!(2));
                sbc_comm!();
            }

            0x98..=0x9D | 0x9F => {
                // SBC B ... SBC L, SBC A
                data = r8!(op & 7) as u32;
                sbc_comm!();
            }

            0xDE => {
                // SBC IMM
                pc = pc.wrapping_add(1);
                sbc_comm!();
            }

            // Logical
            0xA0..=0xA5 => {
                // AND B ... AND L
                data = r8!(op & 7) as u32;
                and_comm!();
            }

            0xA6 => {
                // AND (HL)
                data = read!(r16!(2));
                pc = pc.wrapping_sub(1); // FALLTHRU
                pc = pc.wrapping_add(1);
                and_comm!();
            }
            0xE6 => {
                // AND IMM
                pc = pc.wrapping_add(1);
                and_comm!();
            }
            0xA7 => {
                // AND A
                and_a!();
            }

            0xB0..=0xB5 => {
                // OR B ... OR L
                data = r8!(op & 7) as u32;
                or_comm!();
            }

            0xB6 => {
                // OR (HL)
                data = read!(r16!(2));
                pc = pc.wrapping_sub(1); // FALLTHRU
                pc = pc.wrapping_add(1);
                or_comm!();
            }
            0xF6 => {
                // OR IMM
                pc = pc.wrapping_add(1);
                or_comm!();
            }
            0xB7 => {
                // OR A
                or_a!();
            }

            0xA8..=0xAD => {
                // XOR B ... XOR L
                data = r8!(op & 7) as u32;
                xor_comm!();
            }

            0xAE => {
                // XOR (HL)
                data = read!(r16!(2));
                pc = pc.wrapping_sub(1); // FALLTHRU
                pc = pc.wrapping_add(1);
                xor_comm!();
            }
            0xEE => {
                // XOR IMM
                pc = pc.wrapping_add(1);
                xor_comm!();
            }

            0xAF => {
                // XOR A
                rg[RA] = 0;
                flags = Z_FLAG;
                continue 'stop;
            }

            // Stack
            0xF1 | 0xC1 | 0xD1 | 0xE1 => {
                // POP AF, POP BC, POP DE, POP HL (common)
                data = read!(sp);
                let hi = read!(sp + 1);
                set_r16!((op >> 4) & 3, data.wrapping_add(0x100u32.wrapping_mul(hi)));
                sp = (sp + 2) & 0xFFFF;
                if op != 0xF1 {
                    continue 'stop;
                }
                flags = rg[RA] as u32 & 0xF0;
                rg[RA] = rg[RFLAGS];
                continue 'stop;
            }

            0xC5 => {
                // PUSH BC
                data = r16!(0);
                push!();
            }

            0xD5 => {
                // PUSH DE
                data = r16!(1);
                push!();
            }

            0xE5 => {
                // PUSH HL
                data = r16!(2);
                push!();
            }

            0xF5 => {
                // PUSH AF
                data = ((rg[RA] as u32) << 8) | flags;
                push!();
            }

            // Flow control
            0xFF | 0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 => {
                if op == 0xFF && pc == IDLE_ADDR + 1 {
                    break 'stop;
                }
                // FALLTHRU
                // RST
                data = pc;
                pc = (op & 0x38).wrapping_add(h.cpu().rst_base);
                push!();
            }

            0xCC => {
                // CZ
                pc = pc.wrapping_add(2);
                if flags & Z_FLAG != 0 {
                    call!();
                }
                continue 'stop;
            }

            0xD4 => {
                // CNC
                pc = pc.wrapping_add(2);
                if flags & C_FLAG == 0 {
                    call!();
                }
                continue 'stop;
            }

            0xDC => {
                // CC
                pc = pc.wrapping_add(2);
                if flags & C_FLAG != 0 {
                    call!();
                }
                continue 'stop;
            }

            0xD9 => {
                // RETI
                //interrupts_enabled = 1;
                ret!();
            }

            0xC0 => {
                // RZ
                if flags & Z_FLAG == 0 {
                    ret!();
                }
                continue 'stop;
            }

            0xD0 => {
                // RNC
                if flags & C_FLAG == 0 {
                    ret!();
                }
                continue 'stop;
            }

            0xD8 => {
                // RC
                if flags & C_FLAG != 0 {
                    ret!();
                }
                continue 'stop;
            }

            0x18 => {
                // JR
                branch!(true)
            }

            0x30 => {
                // JR NC
                branch!(flags & C_FLAG == 0)
            }

            0x38 => {
                // JR C
                branch!(flags & C_FLAG != 0)
            }

            0xE9 => {
                // JP_HL
                pc = r16!(2);
                continue 'stop;
            }

            0xC3 => {
                // JP (next-most-common)
                pc = get_addr!();
                continue 'stop;
            }

            0xC2 => {
                // JP NZ
                pc = pc.wrapping_add(2);
                if flags & Z_FLAG == 0 {
                    jp_taken!();
                }
                continue 'stop;
            }

            0xCA => {
                // JP Z (most common)
                pc = pc.wrapping_add(2);
                if flags & Z_FLAG == 0 {
                    continue 'stop;
                }
                jp_taken!();
            }

            0xD2 => {
                // JP NC
                pc = pc.wrapping_add(2);
                if flags & C_FLAG == 0 {
                    jp_taken!();
                }
                continue 'stop;
            }

            0xDA => {
                // JP C
                pc = pc.wrapping_add(2);
                if flags & C_FLAG != 0 {
                    jp_taken!();
                }
                continue 'stop;
            }

            // Flags
            0x2F => {
                // CPL
                rg[RA] = !rg[RA];
                flags |= N_FLAG | H_FLAG;
                continue 'stop;
            }

            0x3F => {
                // CCF
                flags = (flags ^ C_FLAG) & !(N_FLAG | H_FLAG);
                continue 'stop;
            }

            0x37 => {
                // SCF
                flags = (flags | C_FLAG) & !(N_FLAG | H_FLAG);
                continue 'stop;
            }

            0xF3 => {
                // DI
                //interrupts_enabled = 0;
                continue 'stop;
            }

            0xFB => {
                // EI
                //interrupts_enabled = 1;
                continue 'stop;
            }

            // Special
            0xDD | 0xD3 | 0xDB | 0xE3 | 0xE4 // ?
            | 0xEB | 0xEC | 0xF4 | 0xFD | 0xFC
            | 0x10 // STOP
            | 0x27 // DAA (I'll have to implement this eventually...)
            | 0xBF
            | 0xED // Z80 prefix
            | 0x76 => {
                // HALT
                h.cpu().remain += 1;
                break 'stop;
            }

            _ => {
                // If this fails then the case above is missing an opcode
                // assert( false );
                break 'stop;
            }
        }
    }

    // stop:
    pc = pc.wrapping_sub(1);

    // copy state back
    let cpu = h.cpu();
    cpu.r.core = rg;
    cpu.r.pc = pc as i64;
    cpu.r.sp = sp as u16;
    cpu.r.core[RFLAGS] = flags as u8;

    cpu.remain > 0
}
