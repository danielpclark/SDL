// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/

//#include "nes_cpu_log.h"

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

//! Atari 6502 CPU emulator (`Sap_Cpu.h`, `Sap_Cpu.cpp`).
//!
//! [`run`] works on the emulator's 64K memory and calls its write handler
//! (`sap_cpu_io.h`) through [`SapCpuHost`]. Upstream runs on a local copy
//! of its state, which the handler sees (through its `state` pointer) and
//! which it copies back when it stops; this runs on the state itself.
//! Reads are plain memory reads: `CPU_READ()` is `READ_LOW()` in the build
//! this matches, which defines NDEBUG.

pub type SapTime = i32; // clock cycle count
pub type SapAddr = u32; // 16-bit address
pub const FUTURE_SAP_TIME: SapTime = i32::MAX / 2 + 1;

pub const IDLE_ADDR: u32 = 0xFEFF;
pub const IRQ_INHIBIT: u8 = 0x04;

/// Registers are not updated until run() returns (except I flag in status)
#[derive(Clone, Copy, Debug, Default)]
pub struct Registers {
    pub pc: u16,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub status: u8,
    pub sp: u8,
}

const ST_N: u8 = 0x80;
const ST_V: u8 = 0x40;
const ST_R: u8 = 0x20;
const ST_B: u8 = 0x10;
const ST_D: u8 = 0x08;
const ST_I: u8 = 0x04;
const ST_Z: u8 = 0x02;
const ST_C: u8 = 0x01;

#[derive(Clone, Debug, Default)]
pub struct SapCpu {
    pub r: Registers,
    // state_t
    base: SapTime,
    time: SapTime,
    irq_time_: SapTime,
    end_time_: SapTime,
}

/// The emulator running the CPU: its memory and its write handler.
pub trait SapCpuHost {
    fn cpu(&mut self) -> &mut SapCpu;
    /// The 64K memory (`mem`), with the bytes past it that reads and
    /// writes past 0xFFFF reach.
    fn mem(&mut self) -> &mut [u8];
    /// `CPU_WRITE()`
    fn cpu_write(&mut self, addr: SapAddr, data: i32);
}

impl SapCpu {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear all registers (the 64K memory is the host's)
    pub fn reset(&mut self) {
        // check( state == &state_ );
        // state = &state_;
        self.r.status = ST_I;
        self.r.sp = 0xFF;
        self.r.pc = 0;
        self.r.a = 0;
        self.r.x = 0;
        self.r.y = 0;
        self.time = 0;
        self.base = 0;
        self.irq_time_ = FUTURE_SAP_TIME;
        self.end_time_ = FUTURE_SAP_TIME;
    }

    /// Time of beginning of next instruction to be executed
    pub fn time(&self) -> SapTime {
        self.time.wrapping_add(self.base)
    }
    pub fn set_time(&mut self, t: SapTime) {
        self.time = t.wrapping_sub(self.base);
    }
    pub fn adjust_time(&mut self, delta: i32) {
        self.time = self.time.wrapping_add(delta);
    }

    pub fn irq_time(&self) -> SapTime {
        self.irq_time_
    }
    pub fn set_irq_time(&mut self, t: SapTime) {
        self.irq_time_ = t;
        let d = self.update_end_time(self.end_time_, t);
        self.time = self.time.wrapping_add(d);
    }

    pub fn end_time(&self) -> SapTime {
        self.end_time_
    }
    pub fn set_end_time(&mut self, t: SapTime) {
        self.end_time_ = t;
        let d = self.update_end_time(t, self.irq_time_);
        self.time = self.time.wrapping_add(d);
    }

    #[inline]
    fn update_end_time(&mut self, mut t: SapTime, irq: SapTime) -> SapTime {
        if irq < t && self.r.status & IRQ_INHIBIT == 0 {
            t = irq;
        }
        let delta = self.base.wrapping_sub(t);
        self.base = t;
        delta
    }
}

static CLOCK_TABLE: [u8; 256] = [
    // 0 1 2 3 4 5 6 7 8 9 A B C D E F
    0, 6, 2, 8, 3, 3, 5, 5, 3, 2, 2, 2, 4, 4, 6, 6, // 0
    3, 5, 2, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7, // 1
    6, 6, 2, 8, 3, 3, 5, 5, 4, 2, 2, 2, 4, 4, 6, 6, // 2
    3, 5, 2, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7, // 3
    6, 6, 2, 8, 3, 3, 5, 5, 3, 2, 2, 2, 3, 4, 6, 6, // 4
    3, 5, 2, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7, // 5
    6, 6, 2, 8, 3, 3, 5, 5, 4, 2, 2, 2, 5, 4, 6, 6, // 6
    3, 5, 2, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7, // 7
    2, 6, 2, 6, 3, 3, 3, 3, 2, 2, 2, 2, 4, 4, 4, 4, // 8
    3, 6, 2, 6, 4, 4, 4, 4, 2, 5, 2, 5, 5, 5, 5, 5, // 9
    2, 6, 2, 6, 3, 3, 3, 3, 2, 2, 2, 2, 4, 4, 4, 4, // A
    3, 5, 2, 5, 4, 4, 4, 4, 2, 4, 2, 4, 4, 4, 4, 4, // B
    2, 6, 2, 8, 3, 3, 5, 5, 2, 2, 2, 2, 4, 4, 6, 6, // C
    3, 5, 2, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7, // D
    2, 6, 2, 8, 3, 3, 5, 5, 2, 2, 2, 2, 4, 4, 6, 6, // E
    3, 5, 2, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7, // F
]; // 0x00 was 7

/// Where an instruction continues, for upstream's `goto`s.
enum Next {
    Loop,
    DecClockLoop,
    Brk,
    Cli,
    Sei,
    Stop,
}

/// Run until specified time is reached. Returns true if suspicious/unsupported
/// instruction was encountered at any point during run.
pub fn run<H: SapCpuHost + ?Sized>(h: &mut H, end_time: SapTime) -> bool {
    let mut illegal_encountered = false;
    h.cpu().set_end_time(end_time);
    let mut s_time: i32 = h.cpu().time;

    // registers
    let r = h.cpu().r;
    let mut pc: u16 = r.pc;
    let mut a: u8 = r.a;
    let mut x: u8 = r.x;
    let mut y: u8 = r.y;
    let mut sp: u16 = ((r.sp as u16) + 1) | 0x100;

    // status flags
    let mut status: u8;
    let mut c: u16; // carry set if (c & 0x100) != 0
    let mut nz: u16; // Z set if (nz & 0xFF) == 0, N set if (nz & 0x8080) != 0

    macro_rules! set_status {
        ($in:expr) => {{
            let in_: u8 = $in;
            status = in_ & (ST_V | ST_D | ST_I);
            nz = (in_ as u16) << 8;
            c = nz;
            nz |= (!in_ & ST_Z) as u16;
        }};
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
    macro_rules! is_neg {
        () => {
            nz & 0x8080 != 0
        };
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
    macro_rules! read_low {
        ($addr:expr) => {{
            let i = ($addr) as usize;
            h.mem()[i]
        }};
    }
    macro_rules! write_low {
        ($addr:expr, $data:expr) => {{
            let i = ($addr) as usize;
            let d = ($data) as u8;
            h.mem()[i] = d;
        }};
    }
    // (READ() and READ_PROG() are READ_LOW())
    macro_rules! read {
        ($addr:expr) => {
            read_low!($addr) as i32
        };
    }
    macro_rules! write {
        ($addr:expr, $data:expr) => {
            h.cpu_write(($addr) as SapAddr, ($data) as i32)
        };
    }
    macro_rules! push {
        ($v:expr) => {{
            sp = sp.wrapping_sub(1) | 0x100;
            write_low!(sp, $v);
        }};
    }
    macro_rules! handle_page_crossing {
        ($lsb:expr) => {
            s_time = s_time.wrapping_add((($lsb) >> 8) as i32)
        };
    }

    {
        let temp = r.status;
        set_status!(temp);
    }

    let mut next = Next::Loop;
    loop {
        match next {
            Next::Loop => {}
            Next::DecClockLoop => {
                s_time = s_time.wrapping_sub(1);
            }
            Next::Brk => {
                // handle_brk:
                if pc as i32 - 1 >= IDLE_ADDR as i32 {
                    // idle_done:
                    //s_time = 0;
                    pc = pc.wrapping_sub(1);
                    break;
                }
                pc = pc.wrapping_add(1);
                let result_ = 4usize;
                // debug_printf( "BRK executed\n" );

                // interrupt:
                s_time = s_time.wrapping_add(7);

                write_low!(0x100 | sp.wrapping_sub(1), pc >> 8);
                write_low!(0x100 | sp.wrapping_sub(2), pc);
                pc = read_low!(0xFFFA + result_) as u16
                    | (read_low!(0xFFFA + result_ + 1) as u16) << 8;

                sp = sp.wrapping_sub(3) | 0x100;
                let mut temp = calc_status!();
                temp |= ST_R;
                if result_ != 0 {
                    temp |= ST_B; // TODO: incorrectly sets B flag for IRQ
                }
                write_low!(sp, temp);

                status &= !ST_D;
                status |= ST_I;
                let cpu = h.cpu();
                cpu.r.status = status; // update externally-visible I flag

                let delta = cpu.base.wrapping_sub(cpu.end_time_);
                cpu.base = cpu.end_time_;
                s_time = s_time.wrapping_add(delta);
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
                // debug_printf( "Delayed CLI not emulated\n" );
            }
            Next::Sei => {
                // handle_sei:
                let cpu = h.cpu();
                cpu.r.status = status; // update externally-visible I flag
                let delta = cpu.base.wrapping_sub(cpu.end_time_);
                cpu.base = cpu.end_time_;
                s_time = s_time.wrapping_add(delta);
                // if ( s_time < 0 ) goto loop;
                // debug_printf( "Delayed SEI not emulated\n" );
            }
            Next::Stop => break,
        }
        next = Next::Loop;

        // loop:

        // check( (unsigned) GET_SP() < 0x100 );

        let opcode = read_low!(pc);
        pc = pc.wrapping_add(1);
        let instr = pc as usize;

        let mut data: u16 = CLOCK_TABLE[opcode as usize] as u16;
        s_time = s_time.wrapping_add(data as i32);
        if s_time >= 0 {
            // possibly_out_of_time:
            if s_time >= data as i32 {
                s_time = s_time.wrapping_sub(data as i32);
                // out_of_time:
                pc = pc.wrapping_sub(1);
                flush_time!();
                // CPU_DONE( this, TIME, result_ ) -> result_ = -1
                cache_time!();
                if s_time < 0 {
                    continue;
                }
                break; // stop
            }
        }
        // almost_out_of_time:

        data = read_low!(instr) as u16;

        macro_rules! get_msb {
            () => {
                read_low!(instr + 1) as u16
            };
        }
        macro_rules! add_page {
            () => {{
                pc = pc.wrapping_add(1);
                data = data.wrapping_add(0x100u16.wrapping_mul(get_msb!()));
            }};
        }
        macro_rules! get_addr {
            () => {
                (read_low!(instr) as u16) | (read_low!(instr + 1) as u16) << 8
            };
        }
        macro_rules! inc_dec_xy {
            ($reg:ident, $n:expr) => {{
                nz = ($reg as i32 + $n) as u16;
                $reg = nz as u8;
            }};
        }
        macro_rules! ind_x {
            () => {{
                let temp = data.wrapping_add(x as u16);
                0x100u16
                    .wrapping_mul(read_low!(temp.wrapping_add(1) as u8) as u16)
                    .wrapping_add(read_low!(temp as u8) as u16)
            }};
        }
        // TODO: more efficient way to handle negative branch that wraps PC around
        macro_rules! branch {
            ($cond:expr) => {{
                let offset = data as u8 as i8 as i16;
                pc = pc.wrapping_add(1);
                let extra_clock = ((pc & 0xFF) as i32 + offset as i32) as u16;
                if !($cond) {
                    next = Next::DecClockLoop;
                } else {
                    pc = pc.wrapping_add(offset as u16);
                    s_time = s_time.wrapping_add((extra_clock >> 8 & 1) as i32);
                }
            }};
        }

        // ARITH_ADDR_MODES: the operand of the arithmetic instructions
        // (ORA, AND, EOR, ADC, CMP, SBC), by addressing mode
        let aaa = opcode >> 5;
        if opcode & 3 == 1 && aaa != 4 && aaa != 5 {
            match (opcode >> 2) & 7 {
                0 => {
                    // (ind,x)
                    data = ind_x!();
                    // ptr:
                    flush_time!();
                    data = read!(data) as u16;
                    cache_time!();
                }
                4 => {
                    // (ind),y
                    let temp = (read_low!(data) as u16).wrapping_add(y as u16);
                    data = temp.wrapping_add(
                        0x100u16.wrapping_mul(read_low!(data.wrapping_add(1) as u8) as u16),
                    );
                    handle_page_crossing!(temp);
                    // ptr:
                    flush_time!();
                    data = read!(data) as u16;
                    cache_time!();
                }
                5 | 1 => {
                    if (opcode >> 2) & 7 == 5 {
                        // zp,X
                        data = data.wrapping_add(x as u16) as u8 as u16;
                    }
                    // zp
                    data = read_low!(data) as u16;
                }
                6 | 7 | 3 => {
                    let m = (opcode >> 2) & 7;
                    if m == 6 {
                        // abs,Y
                        data = data.wrapping_add(y as u16);
                    } else if m == 7 {
                        // abs,X
                        data = data.wrapping_add(x as u16);
                    }
                    if m != 3 {
                        // ind:
                        handle_page_crossing!(data);
                    }
                    // abs
                    add_page!();
                    // ptr:
                    flush_time!();
                    data = read!(data) as u16;
                    cache_time!();
                }
                _ => {
                    // imm
                }
            }
            match aaa {
                6 => {
                    // CMP
                    nz = (a as u16).wrapping_sub(data);
                    pc = pc.wrapping_add(1);
                    c = !nz;
                    nz &= 0xFF;
                }
                1 => {
                    // AND
                    a &= data as u8;
                    nz = a as u16;
                    pc = pc.wrapping_add(1);
                }
                2 => {
                    // EOR
                    a ^= data as u8;
                    nz = a as u16;
                    pc = pc.wrapping_add(1);
                }
                0 => {
                    // ORA
                    a |= data as u8;
                    nz = a as u16;
                    pc = pc.wrapping_add(1);
                }
                _ => {
                    // SBC, ADC
                    if aaa == 7 {
                        data ^= 0xFF;
                    }
                    // adc_imm:
                    // check( !(status & st_d) );
                    let carry = (c >> 8 & 1) as i16;
                    let ov = ((a ^ 0x80) as i16)
                        .wrapping_add(carry)
                        .wrapping_add(data as u8 as i8 as i16); // sign-extend
                    status &= !ST_V;
                    status |= ((ov as i32) >> 2 & 0x40) as u8;
                    nz = (a as u16).wrapping_add(data).wrapping_add(carry as u16);
                    c = nz;
                    pc = pc.wrapping_add(1);
                    a = nz as u8;
                }
            }
            continue;
        }

        match opcode {
            // Often-Used
            0xB5 => {
                // LDA zp,x
                nz = read_low!(data.wrapping_add(x as u16) as u8) as u16;
                a = nz as u8;
                pc = pc.wrapping_add(1);
            }

            0xA5 => {
                // LDA zp
                nz = read_low!(data) as u16;
                a = nz as u8;
                pc = pc.wrapping_add(1);
            }

            0xD0 => branch!(nz as u8 != 0), // BNE

            0x20 => {
                // JSR
                let temp = pc.wrapping_add(1);
                pc = get_addr!();
                write_low!(0x100 | sp.wrapping_sub(1), temp >> 8);
                sp = sp.wrapping_sub(2) | 0x100;
                write_low!(sp, temp);
            }

            0x4C => {
                // JMP abs
                pc = get_addr!();
            }

            0xE8 => inc_dec_xy!(x, 1), // INX

            0x10 => branch!(!is_neg!()), // BPL

            0x30 => branch!(is_neg!()), // BMI

            0xF0 => branch!(nz as u8 == 0), // BEQ

            0x95 | 0x85 => {
                if opcode == 0x95 {
                    // STA zp,x
                    data = data.wrapping_add(x as u16) as u8 as u16;
                }
                // STA zp
                pc = pc.wrapping_add(1);
                write_low!(data, a);
            }

            0xC8 => inc_dec_xy!(y, 1), // INY

            0xA8 => {
                // TAY
                y = a;
                nz = a as u16;
            }

            0x98 => {
                // TYA
                a = y;
                nz = y as u16;
            }

            0xAD => {
                // LDA abs
                let addr = get_addr!() as u32;
                pc = pc.wrapping_add(2);
                nz = read!(addr) as u16;
                a = nz as u8;
            }

            0x60 => {
                // RTS
                pc = 1 + read_low!(sp) as u16;
                pc = pc.wrapping_add(
                    0x100u16.wrapping_mul(read_low!(0x100 | sp.wrapping_sub(0xFF)) as u16),
                );
                sp = sp.wrapping_sub(0xFE) | 0x100;
            }

            0x99 | 0x8D | 0x9D | 0x91 | 0x81 => {
                let addr: u16 = match opcode {
                    0x99 => {
                        // STA abs,Y
                        let addr = (y as u16).wrapping_add(get_addr!());
                        pc = pc.wrapping_add(2);
                        addr
                    }
                    0x8D => {
                        // STA abs
                        let addr = get_addr!();
                        pc = pc.wrapping_add(2);
                        addr
                    }
                    0x9D => {
                        // STA abs,X (slightly more common than STA abs)
                        let addr = (x as u16).wrapping_add(get_addr!());
                        pc = pc.wrapping_add(2);
                        addr
                    }
                    0x91 => {
                        // STA (ind),Y
                        let temp = (read_low!(data) as u16).wrapping_add(y as u16);
                        let addr = temp.wrapping_add(
                            0x100u16.wrapping_mul(read_low!(data.wrapping_add(1) as u8) as u16),
                        );
                        pc = pc.wrapping_add(1);
                        addr
                    }
                    _ => {
                        // STA (ind,X)
                        let addr = ind_x!();
                        pc = pc.wrapping_add(1);
                        addr
                    }
                };
                if (opcode == 0x99 || opcode == 0x8D || opcode == 0x9D) && addr <= 0x7FF {
                    write_low!(addr, a);
                } else {
                    // sta_ptr:
                    flush_time!();
                    write!(addr, a);
                    cache_time!();
                }
            }

            0xA9 => {
                // LDA #imm
                pc = pc.wrapping_add(1);
                a = data as u8;
                nz = data;
            }

            // common read instructions
            0xA1 => {
                // LDA (ind,X)
                let addr = ind_x!();
                pc = pc.wrapping_add(1);
                // a_nz_read_addr:
                flush_time!();
                nz = read!(addr) as u16;
                a = nz as u8;
                cache_time!();
            }

            0xB1 | 0xB9 | 0xBD => {
                let addr: u16 = match opcode {
                    0xB1 => {
                        // LDA (ind),Y
                        let mut addr = (read_low!(data) as u16).wrapping_add(y as u16);
                        handle_page_crossing!(addr);
                        addr = addr.wrapping_add(
                            0x100u16.wrapping_mul(read_low!(data.wrapping_add(1) as u8) as u16),
                        );
                        pc = pc.wrapping_add(1);
                        addr
                    }
                    0xB9 => {
                        // LDA abs,Y
                        handle_page_crossing!(data + y as u16);
                        let addr = get_addr!().wrapping_add(y as u16);
                        pc = pc.wrapping_add(2);
                        addr
                    }
                    _ => {
                        // LDA abs,X
                        handle_page_crossing!(data + x as u16);
                        let addr = get_addr!().wrapping_add(x as u16);
                        pc = pc.wrapping_add(2);
                        addr
                    }
                };
                nz = read_low!(addr) as u16;
                a = nz as u8;
                if (addr ^ 0x8000) > 0x9FFF {
                    // a_nz_read_addr:
                    flush_time!();
                    nz = read!(addr) as u16;
                    a = nz as u8;
                    cache_time!();
                }
            }

            // Branch
            0x50 => branch!(status & ST_V == 0), // BVC

            0x70 => branch!(status & ST_V != 0), // BVS

            0xB0 => branch!(c & 0x100 != 0), // BCS

            0x90 => branch!(c & 0x100 == 0), // BCC

            // Load/store
            0x94 | 0x84 => {
                if opcode == 0x94 {
                    // STY zp,x
                    data = data.wrapping_add(x as u16) as u8 as u16;
                }
                // STY zp
                pc = pc.wrapping_add(1);
                write_low!(data, y);
            }

            0x96 | 0x86 => {
                if opcode == 0x96 {
                    // STX zp,y
                    data = data.wrapping_add(y as u16) as u8 as u16;
                }
                // STX zp
                pc = pc.wrapping_add(1);
                write_low!(data, x);
            }

            0xB6 | 0xA6 | 0xA2 => {
                if opcode == 0xB6 {
                    // LDX zp,y
                    data = data.wrapping_add(y as u16) as u8 as u16;
                }
                if opcode != 0xA2 {
                    // LDX zp
                    data = read_low!(data) as u16;
                }
                // LDX #imm
                pc = pc.wrapping_add(1);
                x = data as u8;
                nz = data;
            }

            0xB4 | 0xA4 | 0xA0 => {
                if opcode == 0xB4 {
                    // LDY zp,x
                    data = data.wrapping_add(x as u16) as u8 as u16;
                }
                if opcode != 0xA0 {
                    // LDY zp
                    data = read_low!(data) as u16;
                }
                // LDY #imm
                pc = pc.wrapping_add(1);
                y = data as u8;
                nz = data;
            }

            0xBC | 0xAC => {
                if opcode == 0xBC {
                    // LDY abs,X
                    data = data.wrapping_add(x as u16);
                    handle_page_crossing!(data);
                }
                // LDY abs
                let addr = data as u32 + 0x100 * get_msb!() as u32;
                pc = pc.wrapping_add(2);
                flush_time!();
                nz = read!(addr) as u16;
                y = nz as u8;
                cache_time!();
            }

            0xBE | 0xAE => {
                if opcode == 0xBE {
                    // LDX abs,y
                    data = data.wrapping_add(y as u16);
                    handle_page_crossing!(data);
                }
                // LDX abs
                let addr = data as u32 + 0x100 * get_msb!() as u32;
                pc = pc.wrapping_add(2);
                flush_time!();
                nz = read!(addr) as u16;
                x = nz as u8;
                cache_time!();
            }

            0x8C | 0x8E => {
                // STY abs, STX abs
                let temp = if opcode == 0x8C { y } else { x };
                // store_abs:
                let addr = get_addr!() as u32;
                pc = pc.wrapping_add(2);
                if addr <= 0x7FF {
                    write_low!(addr, temp);
                } else {
                    flush_time!();
                    write!(addr, temp);
                    cache_time!();
                }
            }

            // Compare
            0xEC | 0xE4 | 0xE0 => {
                if opcode == 0xEC {
                    // CPX abs
                    let addr = get_addr!() as u32;
                    pc = pc.wrapping_add(1);
                    flush_time!();
                    data = read!(addr) as u16;
                    cache_time!();
                } else if opcode == 0xE4 {
                    // CPX zp
                    data = read_low!(data) as u16;
                }
                // CPX #imm
                // cpx_data:
                nz = (x as u16).wrapping_sub(data);
                pc = pc.wrapping_add(1);
                c = !nz;
                nz &= 0xFF;
            }

            0xCC | 0xC4 | 0xC0 => {
                if opcode == 0xCC {
                    // CPY abs
                    let addr = get_addr!() as u32;
                    pc = pc.wrapping_add(1);
                    flush_time!();
                    data = read!(addr) as u16;
                    cache_time!();
                } else if opcode == 0xC4 {
                    // CPY zp
                    data = read_low!(data) as u16;
                }
                // CPY #imm
                // cpy_data:
                nz = (y as u16).wrapping_sub(data);
                pc = pc.wrapping_add(1);
                c = !nz;
                nz &= 0xFF;
            }

            // Logical
            0x2C => {
                // BIT abs
                let addr = get_addr!() as u32;
                pc = pc.wrapping_add(2);
                status &= !ST_V;
                nz = read!(addr) as u16;
                status |= nz as u8 & ST_V;
                if a as u16 & nz == 0 {
                    nz <<= 8; // result must be zero, even if N bit is set
                }
            }

            0x24 => {
                // BIT zp
                nz = read_low!(data) as u16;
                pc = pc.wrapping_add(1);
                status &= !ST_V;
                status |= nz as u8 & ST_V;
                if a as u16 & nz == 0 {
                    nz <<= 8; // result must be zero, even if N bit is set
                }
            }

            // Add/subtract
            0xEB => {
                // unofficial equivalent of SBC #imm
                data ^= 0xFF;
                // adc_imm:
                // check( !(status & st_d) );
                let carry = (c >> 8 & 1) as i16;
                let ov = ((a ^ 0x80) as i16)
                    .wrapping_add(carry)
                    .wrapping_add(data as u8 as i8 as i16); // sign-extend
                status &= !ST_V;
                status |= ((ov as i32) >> 2 & 0x40) as u8;
                nz = (a as u16).wrapping_add(data).wrapping_add(carry as u16);
                c = nz;
                pc = pc.wrapping_add(1);
                a = nz as u8;
            }

            // Shift/rotate
            0x4A | 0x6A => {
                if opcode == 0x4A {
                    // LSR A
                    c = 0;
                }
                // ROR A
                nz = c >> 1 & 0x80;
                c = (a as u16) << 8;
                nz |= (a >> 1) as u16;
                a = nz as u8;
            }

            0x0A => {
                // ASL A
                nz = (a as u16) << 1;
                c = nz;
                a = nz as u8;
            }

            0x2A => {
                // ROL A
                nz = (a as u16) << 1;
                let temp = c >> 8 & 1;
                c = nz;
                nz |= temp;
                a = nz as u8;
            }

            0x5E | 0x4E | 0x6E | 0x7E => {
                if opcode == 0x5E || opcode == 0x7E {
                    // LSR abs,X / ROR abs,X
                    data = data.wrapping_add(x as u16);
                }
                if opcode == 0x5E || opcode == 0x4E {
                    // LSR abs
                    c = 0;
                }
                // ROR abs
                // ror_abs:
                add_page!();
                flush_time!();
                let temp = read!(data);
                nz = ((c >> 1 & 0x80) as i32 | (temp >> 1)) as u16;
                c = (temp << 8) as u16;
                // rotate_common:
                pc = pc.wrapping_add(1);
                write!(data, nz as u8);
                cache_time!();
            }

            0x3E | 0x1E | 0x0E | 0x2E => {
                if opcode == 0x3E || opcode == 0x1E {
                    // ROL abs,X / ASL abs,X
                    data = data.wrapping_add(x as u16);
                }
                if opcode == 0x1E || opcode == 0x0E {
                    // ASL abs
                    c = 0;
                }
                // ROL abs
                // rol_abs:
                add_page!();
                nz = c >> 8 & 1;
                flush_time!();
                c = (read!(data) << 1) as u16;
                nz |= c;
                // rotate_common:
                pc = pc.wrapping_add(1);
                write!(data, nz as u8);
                cache_time!();
            }

            0x76 | 0x56 | 0x46 | 0x66 => {
                if opcode == 0x76 || opcode == 0x56 {
                    // ROR zp,x / LSR zp,x
                    data = data.wrapping_add(x as u16) as u8 as u16;
                }
                if opcode == 0x56 || opcode == 0x46 {
                    // LSR zp
                    c = 0;
                }
                // ROR zp
                // ror_zp:
                let temp = read_low!(data) as u16;
                nz = (c >> 1 & 0x80) | (temp >> 1);
                c = temp << 8;
                // write_nz_zp:
                pc = pc.wrapping_add(1);
                write_low!(data, nz);
            }

            0x36 | 0x16 | 0x06 | 0x26 => {
                if opcode == 0x36 || opcode == 0x16 {
                    // ROL zp,x / ASL zp,x
                    data = data.wrapping_add(x as u16) as u8 as u16;
                }
                if opcode == 0x16 || opcode == 0x06 {
                    // ASL zp
                    c = 0;
                }
                // ROL zp
                // rol_zp:
                nz = c >> 8 & 1;
                c = (read_low!(data) as u16) << 1;
                nz |= c;
                // write_nz_zp:
                pc = pc.wrapping_add(1);
                write_low!(data, nz);
            }

            // Increment/decrement
            0xCA => inc_dec_xy!(x, -1), // DEX

            0x88 => inc_dec_xy!(y, -1), // DEY

            0xF6 | 0xE6 | 0xD6 | 0xC6 => {
                if opcode == 0xF6 || opcode == 0xD6 {
                    // INC zp,x / DEC zp,x
                    data = data.wrapping_add(x as u16) as u8 as u16;
                }
                nz = if opcode == 0xF6 || opcode == 0xE6 {
                    1
                } else {
                    u16::MAX
                };
                // add_nz_zp:
                nz = nz.wrapping_add(read_low!(data) as u16);
                // write_nz_zp:
                pc = pc.wrapping_add(1);
                write_low!(data, nz);
            }

            0xFE | 0xEE | 0xDE | 0xCE => {
                data = match opcode {
                    0xFE | 0xDE => (x as u16).wrapping_add(get_addr!()), // INC abs,x / DEC abs,x
                    _ => get_addr!(),                                    // INC abs / DEC abs
                };
                // inc_ptr: / dec_ptr:
                nz = if opcode == 0xFE || opcode == 0xEE {
                    1
                } else {
                    u16::MAX
                };
                // inc_common:
                flush_time!();
                nz = nz.wrapping_add(read!(data) as u16);
                pc = pc.wrapping_add(2);
                write!(data, nz as u8);
                cache_time!();
            }

            // Transfer
            0xAA => {
                // TAX
                x = a;
                nz = a as u16;
            }

            0x8A => {
                // TXA
                a = x;
                nz = x as u16;
            }

            0x9A => {
                // TXS
                sp = ((x as u16) + 1) | 0x100; // verified (no flag change)
            }

            0xBA => {
                // TSX
                x = (sp.wrapping_sub(1) & 0xFF) as u8;
                nz = x as u16;
            }

            // Stack
            0x48 => push!(a), // PHA (verified)

            0x68 => {
                // PLA
                nz = read_low!(sp) as u16;
                a = nz as u8;
                sp = sp.wrapping_sub(0xFF) | 0x100;
            }

            0x40 => {
                // RTI
                let temp = read_low!(sp);
                pc = read_low!(0x100 | sp.wrapping_sub(0xFF)) as u16;
                pc |= (read_low!(0x100 | sp.wrapping_sub(0xFE)) as u16) * 0x100;
                sp = sp.wrapping_sub(0xFD) | 0x100;
                let data = status;
                set_status!(temp);
                let cpu = h.cpu();
                cpu.r.status = status; // update externally-visible I flag
                if (data ^ status) & ST_I != 0 {
                    let mut new_time = cpu.end_time_;
                    if status & ST_I == 0 && new_time > cpu.irq_time_ {
                        new_time = cpu.irq_time_;
                    }
                    let delta = cpu.base.wrapping_sub(new_time);
                    cpu.base = new_time;
                    s_time = s_time.wrapping_add(delta);
                }
            }

            0x28 => {
                // PLP
                let temp = read_low!(sp);
                sp = sp.wrapping_sub(0xFF) | 0x100;
                let changed = status ^ temp;
                set_status!(temp);
                if changed & ST_I != 0 {
                    next = if status & ST_I != 0 {
                        Next::Sei
                    } else {
                        Next::Cli
                    };
                }
                // (I flag didn't change)
            }

            0x08 => {
                // PHP
                let temp = calc_status!();
                push!(temp | (ST_B | ST_R));
            }

            0x6C => {
                // JMP (ind)
                data = get_addr!();
                pc = read_low!(data) as u16;
                data = (data & 0xFF00) | (data.wrapping_add(1) & 0xFF);
                pc |= 0x100 * read_low!(data) as u16;
            }

            0x00 => next = Next::Brk, // BRK

            // Flags
            0x38 => c = !0, // SEC

            0x18 => c = 0, // CLC

            0xB8 => status &= !ST_V, // CLV

            0xD8 => status &= !ST_D, // CLD

            0xF8 => status |= ST_D, // SED

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

            // Unofficial

            // SKW - Skip word
            0x1C | 0x3C | 0x5C | 0x7C | 0xDC | 0xFC | 0x0C | 0x74 | 0x04 | 0x14 | 0x34 | 0x44
            | 0x54 | 0x64 | 0x80 | 0x82 | 0x89 | 0xC2 | 0xD4 | 0xE2 | 0xF4 => {
                if matches!(opcode, 0x1C | 0x3C | 0x5C | 0x7C | 0xDC | 0xFC) {
                    handle_page_crossing!(data + x as u16);
                }
                if matches!(opcode, 0x1C | 0x3C | 0x5C | 0x7C | 0xDC | 0xFC | 0x0C) {
                    pc = pc.wrapping_add(1);
                }
                // SKB - Skip byte
                pc = pc.wrapping_add(1);
            }

            // NOP
            0xEA | 0x1A | 0x3A | 0x5A | 0x7A | 0xDA | 0xFA => {}

            // Unimplemented

            // halt
            //case 0x02: case 0x12: case 0x22: case 0x32: case 0x42: case 0x52:
            //case 0x62: case 0x72: case 0x92: case 0xB2: case 0xD2: case 0xF2:
            _ => {
                illegal_encountered = true;
                pc = pc.wrapping_sub(1);
                next = Next::Stop;
            }
        }
    }

    // stop:
    let cpu = h.cpu();
    cpu.time = s_time;

    cpu.r.pc = pc;
    cpu.r.sp = (sp.wrapping_sub(1) & 0xFF) as u8;
    cpu.r.a = a;
    cpu.r.x = x;
    cpu.r.y = y;

    cpu.r.status = calc_status!();

    illegal_encountered
}
