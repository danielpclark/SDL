// Core SPC emulation: CPU, timers, SMP registers, memory

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

//! The SPC-700's core (`Spc_Cpu.cpp`, `Spc_Cpu.h`): its timers, SMP
//! registers and memory, and the CPU, as [`SnesSpc`]'s methods. As in
//! the build this matches, SPC_MORE_ACCURACY is 0, SPC_LESS_ACCURATE (the
//! fast DSP's) is 1, and NDEBUG is defined.

use crate::snes_spc::{
    if_0_then_256, RelTime, SnesSpc, TimeT, CLOCKS_PER_SAMPLE, ROM_ADDR, ROM_SIZE, R_CONTROL,
    R_CPUIO0, R_CPUIO1, R_CPUIO2, R_CPUIO3, R_DSPADDR, R_DSPDATA, R_T0OUT, R_T0TARGET, R_T1OUT,
    R_T1TARGET, R_T2OUT, R_T2TARGET, R_TEST, SKIPPING_TIME, TIMER_COUNT,
};
use crate::spc_dsp;

// Timers

// (#if SPC_DISABLE_TEMPO: shifts)
#[inline]
fn timer_div(prescaler: i32, n: i32) -> i32 {
    n / prescaler
}
#[inline]
fn timer_mul(prescaler: i32, n: i32) -> i32 {
    n.wrapping_mul(prescaler)
}

// DSP

// (#if SPC_LESS_ACCURATE)
const MAX_REG_TIME: i32 = 29;

pub(crate) static REG_TIMES_: [i8; 256] = [
    -1, 0, -11, -10, -15, -11, -2, -2, 4, 3, 14, 14, 26, 26, 14, 22, //
    2, 3, 0, 1, -12, 0, 1, 1, 7, 6, 14, 14, 27, 14, 14, 23, //
    5, 6, 3, 4, -1, 3, 4, 4, 10, 9, 14, 14, 26, -5, 14, 23, //
    8, 9, 6, 7, 2, 6, 7, 7, 13, 12, 14, 14, 27, -4, 14, 24, //
    11, 12, 9, 10, 5, 9, 10, 10, 16, 15, 14, 14, -2, -4, 14, 24, //
    14, 15, 12, 13, 8, 12, 13, 13, 19, 18, 14, 14, -2, -36, 14, 24, //
    17, 18, 15, 16, 11, 15, 16, 16, 22, 21, 14, 14, 28, -3, 14, 25, //
    20, 21, 18, 19, 14, 18, 19, 19, 25, 24, 14, 14, 14, 29, 14, 25, //
    29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, //
    29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, //
    29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, //
    29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, //
    29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, //
    29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, //
    29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, //
    29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29,
];

// Read/write handlers are divided into multiple functions to keep rarely-used
// functionality separate so often-used functionality can be optimized better
// by compiler.

/// If write isn't preceded by read, data has this added to it
const NO_READ_BEFORE_WRITE: i32 = 0x2000;

// Status flag handling

// Hex value in name to clarify code and bit shifting.
// Flag stored in indicated variable during emulation
const N80: i32 = 0x80; // nz
const V40: i32 = 0x40; // psw
const P20: i32 = 0x20; // dp
const B10: i32 = 0x10; // psw
const H08: i32 = 0x08; // psw
const I04: i32 = 0x04; // psw
const Z02: i32 = 0x02; // nz
const C01: i32 = 0x01; // c

const NZ_NEG_MASK: i32 = 0x880; // either bit set indicates N flag set

/// Where the CPU continues, for upstream's `goto`s.
#[derive(Clone, Copy)]
enum Next {
    Loop,
    IncPcLoop,
    CbranchTakenLoop,
}

impl SnesSpc {
    // Timers

    pub(crate) fn run_timer_(&mut self, i: usize, time: RelTime) -> usize {
        let t = &mut self.timers[i];
        let elapsed = timer_div(t.prescaler, time.wrapping_sub(t.next_time)) + 1;
        t.next_time = t.next_time.wrapping_add(timer_mul(t.prescaler, elapsed));

        if t.enabled != 0 {
            let remain = if_0_then_256(t.period - t.divider);
            let mut divider = t.divider + elapsed;
            let over = elapsed - remain;
            if over >= 0 {
                let n = over / t.period;
                t.counter = (t.counter + 1 + n) & 0x0F;
                divider = over - n * t.period;
            }
            t.divider = divider as u8 as i32;
        }
        i
    }

    #[inline]
    pub(crate) fn run_timer(&mut self, i: usize, time: RelTime) -> usize {
        if time >= self.timers[i].next_time {
            return self.run_timer_(i, time);
        }
        i
    }

    // ROM

    pub(crate) fn enable_rom(&mut self, enable: i32) {
        if self.rom_enabled != enable {
            self.rom_enabled = enable;
            if enable != 0 {
                self.hi_ram
                    .copy_from_slice(&self.ram[ROM_ADDR..ROM_ADDR + ROM_SIZE]);
            }
            let src = if enable != 0 { self.rom } else { self.hi_ram };
            self.ram[ROM_ADDR..ROM_ADDR + ROM_SIZE].copy_from_slice(&src);
            // TODO: ROM can still get overwritten when DSP writes to echo buffer
        }
    }

    // DSP

    /// `RUN_DSP( time, offset )`: whether it ran (the macro's `if`, which
    /// dsp_write()'s `else` follows)
    #[inline]
    fn run_dsp(&mut self, time: i32, offset: i32) -> bool {
        let count = time - offset - self.dsp_time;
        if count >= 0 {
            let clock_count = (count & !(CLOCKS_PER_SAMPLE - 1)) + CLOCKS_PER_SAMPLE;
            self.dsp_time += clock_count;
            self.dsp.run(&mut self.ram, &mut self.main, clock_count);
            return true;
        }
        false
    }

    pub(crate) fn dsp_read(&mut self, time: RelTime) -> i32 {
        let off = self.reg_times[(self.smp_regs[0][R_DSPADDR] & 0x7F) as usize] as i32;
        self.run_dsp(time, off);

        // (#ifdef SPC_DSP_READ_HOOK)
        self.dsp.read((self.smp_regs[0][R_DSPADDR] & 0x7F) as usize)
    }

    #[inline]
    pub(crate) fn dsp_write(&mut self, data: i32, time: RelTime) {
        let off = self.reg_times[self.smp_regs[0][R_DSPADDR] as usize] as i32;
        if !self.run_dsp(time, off) && self.dsp_time == SKIPPING_TIME {
            // (#if SPC_LESS_ACCURATE)
            let r = self.smp_regs[0][R_DSPADDR] as usize;
            if r == spc_dsp::R_KON {
                self.skipped_kon |= data & !self.dsp.read(spc_dsp::R_KOFF);
            }

            if r == spc_dsp::R_KOFF {
                self.skipped_koff |= data;
                self.skipped_kon &= !data;
            }
        }

        // (#ifdef SPC_DSP_WRITE_HOOK)

        if self.smp_regs[0][R_DSPADDR] <= 0x7F {
            let addr = self.smp_regs[0][R_DSPADDR] as usize;
            self.dsp.write(addr, data);
        } else {
            // debug_printf( "SPC wrote to DSP register > $7F\n" );
        }
    }

    // Memory access extras

    // (MEM_ACCESS: nothing in the build this matches)

    // CPU write

    pub(crate) fn cpu_write_smp_reg_(&mut self, data: i32, time: RelTime, addr: u16) {
        match addr as usize {
            R_T0TARGET | R_T1TARGET | R_T2TARGET => {
                let ti = addr as usize - R_T0TARGET;
                let period = if_0_then_256(data);
                if self.timers[ti].period != period {
                    let t = self.run_timer(ti, time);
                    // (#if SPC_MORE_ACCURACY: the pathological target write)
                    self.timers[t].period = period;
                }
            }

            R_T0OUT | R_T1OUT | R_T2OUT => {
                // debug_printf( "SPC wrote to counter %d\n", (int) addr - r_t0out );

                if data < NO_READ_BEFORE_WRITE / 2 {
                    let t = self.run_timer(addr as usize - R_T0OUT, time - 1);
                    self.timers[t].counter = 0;
                }
            }

            // Registers that act like RAM
            0x8 | 0x9 => {
                self.smp_regs[1][addr as usize] = data as u8;
            }

            R_TEST => {
                // if ( (uint8_t) data != 0x0A )
                //  debug_printf( "SPC wrote to test register\n" );
            }

            R_CONTROL => {
                // port clears
                if data & 0x10 != 0 {
                    self.smp_regs[1][R_CPUIO0] = 0;
                    self.smp_regs[1][R_CPUIO1] = 0;
                }
                if data & 0x20 != 0 {
                    self.smp_regs[1][R_CPUIO2] = 0;
                    self.smp_regs[1][R_CPUIO3] = 0;
                }

                // timers
                for i in 0..TIMER_COUNT {
                    let enabled = data >> i & 1;
                    if self.timers[i].enabled != enabled {
                        let t = self.run_timer(i, time);
                        let t = &mut self.timers[t];
                        t.enabled = enabled;
                        if enabled != 0 {
                            t.divider = 0;
                            t.counter = 0;
                        }
                    }
                }
                self.enable_rom(data & 0x80);
            }
            _ => {}
        }
    }

    pub(crate) fn cpu_write_smp_reg(&mut self, data: i32, time: RelTime, addr: u16) {
        if addr as usize == R_DSPDATA {
            // 99%
            self.dsp_write(data, time);
        } else {
            self.cpu_write_smp_reg_(data, time, addr);
        }
    }

    pub(crate) fn cpu_write_high(&mut self, data: i32, i: u8) {
        // assert ( i < rom_size );
        self.hi_ram[i as usize] = data as u8;
        if self.rom_enabled != 0 {
            self.ram[i as usize + ROM_ADDR] = self.rom[i as usize]; // restore overwritten ROM
        }
    }

    pub(crate) fn cpu_write(&mut self, data: i32, addr: u16, time: RelTime) {
        // MEM_ACCESS( time, addr )

        // RAM
        self.ram[addr as usize] = data as u8;
        if addr >= 0xF0 {
            // 64%
            let reg = addr.wrapping_sub(0xF0);
            // $F0-$FF
            if (reg as usize) < crate::snes_spc::REG_COUNT {
                // 87%
                self.smp_regs[0][reg as usize] = data as u8;

                // Ports
                // (#ifdef SPC_PORT_WRITE_HOOK)

                // Registers other than $F2 and $F4-$F7
                if reg != 2 && (reg < 4 || reg > 7) {
                    // 36%
                    self.cpu_write_smp_reg(data, time, reg);
                }
            }
            // High mem/address wrap-around
            else if addr as usize >= ROM_ADDR {
                // 1% in IPL ROM area or address wrapped around
                self.cpu_write_high(data, (addr as usize - ROM_ADDR) as u8);
            }
        }
    }

    // CPU read

    #[inline]
    pub(crate) fn cpu_read_smp_reg(&mut self, reg: i32, time: RelTime) -> i32 {
        let mut result = self.smp_regs[1][reg as usize] as i32;
        let reg = reg - R_DSPADDR as i32;
        // DSP addr and data
        if reg as u32 <= 1 {
            // 4% 0xF2 and 0xF3
            result = self.smp_regs[0][R_DSPADDR] as i32;
            if reg as u32 == 1 {
                result = self.dsp_read(time); // 0xF3
            }
        }
        result
    }

    pub(crate) fn cpu_read(&mut self, addr: u16, time: RelTime) -> i32 {
        // MEM_ACCESS( time, addr )

        // RAM
        let mut result = self.ram[addr as usize] as i32;
        let mut reg = addr as i32 - 0xF0;
        if reg >= 0 {
            // 40%
            reg -= 0x10;
            if reg as u32 >= 0xFF00 {
                // 21%
                reg += 0x10 - R_T0OUT as i32;

                // Timers
                if (reg as u32) < TIMER_COUNT as u32 {
                    // 90%
                    let mut t = reg as usize;
                    if time >= self.timers[t].next_time {
                        t = self.run_timer_(t, time);
                    }
                    result = self.timers[t].counter;
                    self.timers[t].counter = 0;
                }
                // Other registers
                else if reg < 0 {
                    // 10%
                    result = self.cpu_read_smp_reg(reg + R_T0OUT as i32, time);
                } else {
                    // 1%
                    // assert( reg + (r_t0out + 0xF0 - 0x10000) < 0x100 );
                    result = self.cpu_read((reg + (R_T0OUT as i32 + 0xF0 - 0x10000)) as u16, time);
                }
            }
        }

        result
    }

    // Run

    pub(crate) fn end_frame(&mut self, end_time: TimeT) {
        // Catch CPU up to as close to end as possible. If final instruction
        // would exceed end, does NOT execute it and leaves m.spc_time < end.
        if end_time > self.spc_time {
            self.run_until_(end_time);
        }

        self.spc_time -= end_time;
        self.extra_clocks += end_time;

        // Greatest number of clocks early that emulation can stop early due to
        // not being able to execute current instruction without going over
        // allowed time.
        // assert( -cpu_lag_max <= m.spc_time && m.spc_time <= 0 );

        // Catch timers up to CPU
        for i in 0..TIMER_COUNT {
            self.run_timer(i, 0);
        }

        // Catch DSP up to CPU
        if self.dsp_time < 0 {
            self.run_dsp(0, MAX_REG_TIME);
        }

        // Save any extra samples beyond what should be generated
        if self.buf_begin {
            self.save_extra();
        }
    }

    // Spc_Cpu.h

    // Memory access

    #[inline]
    fn read_prog16(&self, addr: u32) -> u32 {
        self.ram[(addr & 0xffff) as usize] as u32
            | (self.ram[(addr.wrapping_add(1) & 0xffff) as usize] as u32) << 8
    }

    pub(crate) fn cpu_mem_bit(&mut self, pc: u16, rel_time: RelTime) -> u32 {
        let addr = self.read_prog16(pc as u32);
        let t = (self.cpu_read((addr & 0x1FFF) as u16, rel_time) as u32) >> (addr >> 13);
        t << 8 & 0x100
    }

    /// `run_until_()`: runs the CPU to `end_time` (upstream returns the
    /// ports, `&REGS [r_cpuio0]`)
    pub(crate) fn run_until_(&mut self, end_time: TimeT) {
        // SPC_CPU_RUN_FUNC
        let mut rel_time: RelTime = self.spc_time - end_time;
        // assert( rel_time <= 0 );
        self.spc_time = end_time;
        self.dsp_time += rel_time;
        self.timers[0].next_time += rel_time;
        self.timers[1].next_time += rel_time;
        self.timers[2].next_time += rel_time;

        let mut a: u8 = self.cpu_regs.a;
        let mut x: u8 = self.cpu_regs.x;
        let mut y: u8 = self.cpu_regs.y;
        let mut pc: u16;
        let mut sp: u8;
        let mut psw: i32;
        let mut c: i32;
        let mut nz: i32;
        let mut dp: i32;

        macro_rules! get_psw {
            () => {{
                let mut out = psw & !(N80 | P20 | Z02 | C01);
                out |= c >> 8 & C01;
                out |= dp >> 3 & P20;
                out |= ((nz >> 4) | nz) & N80;
                if nz as u8 == 0 {
                    out |= Z02;
                }
                out
            }};
        }
        macro_rules! set_psw {
            ($in:expr) => {{
                let in_: i32 = $in;
                psw = in_;
                c = in_ << 8;
                dp = in_ << 3 & 0x100;
                nz = (in_ << 4 & 0x800) | (!in_ & Z02);
            }};
        }
        macro_rules! ram {
            ($i:expr) => {
                self.ram[($i) as usize]
            };
        }
        // (READ_PC( pc ): the byte at pc, which can be past 0xFFFF, in the
        // padding)
        macro_rules! read_pc {
            ($pc:expr) => {
                self.ram[($pc) as usize] as u32
            };
        }
        macro_rules! read_pc16 {
            ($pc:expr) => {
                self.read_prog16(($pc) as u32)
            };
        }
        macro_rules! read {
            ($time:expr, $addr:expr) => {
                self.cpu_read(($addr) as u16, rel_time + ($time))
            };
        }
        macro_rules! write {
            ($time:expr, $addr:expr, $data:expr) => {
                self.cpu_write(($data) as i32, ($addr) as u16, rel_time + ($time))
            };
        }
        macro_rules! read_dp {
            ($time:expr, $addr:expr) => {
                read!($time, (dp as u32).wrapping_add(($addr) as u32))
            };
        }
        macro_rules! write_dp {
            ($time:expr, $addr:expr, $data:expr) => {
                write!($time, (dp as u32).wrapping_add(($addr) as u32), $data)
            };
        }
        // timers are by far the most common thing read from dp
        macro_rules! read_timer {
            ($time:expr, $addr:expr) => {{
                let adj_time: RelTime = rel_time + ($time);
                let dp_addr = ($addr) as i32;
                let ti = dp_addr - (R_T0OUT as i32 + 0xF0);
                if (ti as u32) < TIMER_COUNT as u32 {
                    let mut t = ti as usize;
                    if adj_time >= self.timers[t].next_time {
                        t = self.run_timer_(t, adj_time);
                    }
                    let out = self.timers[t].counter;
                    self.timers[t].counter = 0;
                    out
                } else {
                    let mut out = self.ram[dp_addr as usize] as i32;
                    let i = dp_addr - 0xF0;
                    if (i as u32) < 0x10 {
                        out = self.cpu_read_smp_reg(i, adj_time);
                    }
                    out
                }
            }};
        }
        macro_rules! read_dp_timer {
            ($time:expr, $addr:expr) => {
                read_timer!($time, dp + ($addr) as i32)
            };
        }
        macro_rules! push {
            ($data:expr) => {{
                self.ram[0x100 + sp as usize] = ($data) as u8;
                sp = sp.wrapping_sub(1);
            }};
        }
        macro_rules! push16 {
            ($data:expr) => {{
                let d = ($data) as i32;
                push!((d & 0xff00) >> 8);
                push!(d & 0xff);
            }};
        }
        macro_rules! pop {
            () => {{
                sp = sp.wrapping_add(1);
                self.ram[0x100 + sp as usize]
            }};
        }
        macro_rules! mem_bit {
            ($rel:expr) => {
                self.cpu_mem_bit(pc, rel_time + ($rel))
            };
        }

        pc = self.cpu_regs.pc;
        sp = self.cpu_regs.sp;
        set_psw!(self.cpu_regs.psw as i32);

        let mut next = Next::Loop;
        'stop: loop {
            match next {
                Next::Loop => {}
                Next::IncPcLoop => {
                    pc = pc.wrapping_add(1);
                }
                Next::CbranchTakenLoop => {
                    pc = pc.wrapping_add(ram!(pc) as i8 as u16);
                    pc = pc.wrapping_add(1);
                }
            }
            next = Next::Loop;

            // loop:
            let opcode: u32 = ram!(pc) as u32;
            let mut data: u32;
            rel_time += self.cycle_table[opcode as usize] as i32;
            if rel_time > 0 {
                // out_of_time:
                rel_time -= self.cycle_table[ram!(pc) as usize] as i32; // undo partial execution of opcode
                break 'stop;
            }

            // (#ifdef SPC_CPU_OPCODE_HOOK)

            // TODO: if PC is at end of memory, this will get wrong operand (very obscure)
            pc = pc.wrapping_add(1);
            data = ram!(pc) as u32;

            // Common instructions

            macro_rules! branch {
                ($cond:expr) => {{
                    pc = pc.wrapping_add(1);
                    pc = pc.wrapping_add(data as u8 as i8 as u16);
                    if !($cond) {
                        pc = pc.wrapping_sub(data as u8 as i8 as u16);
                        rel_time -= 2;
                    }
                    continue 'stop;
                }};
            }
            macro_rules! cbranch {
                ($cond:expr) => {{
                    pc = pc.wrapping_add(1);
                    if $cond {
                        next = Next::CbranchTakenLoop;
                        continue 'stop;
                    }
                    rel_time -= 2;
                    next = Next::IncPcLoop;
                    continue 'stop;
                }};
            }

            // ADDR_MODES_( op ), ADDR_MODES_NO_DP( op ) and ADDR_MODES( op ):
            // the operand's address for `opcode - op`, and whether it is one
            // of these modes
            macro_rules! addr_mode {
                ($m:expr) => {{
                    match $m {
                        -2 => {
                            // (X)
                            data = (x as i32 + dp) as u32;
                            pc = pc.wrapping_sub(1);
                            true
                        }
                        0x0F => {
                            // (dp)+Y
                            data = self
                                .read_prog16(data.wrapping_add(dp as u32))
                                .wrapping_add(y as u32);
                            true
                        }
                        -1 => {
                            // (dp+X)
                            data = self.read_prog16(
                                (data.wrapping_add(x as u32) as u8 as u32) + dp as u32,
                            );
                            true
                        }
                        0x0E | 0x0D | -3 => {
                            if $m == 0x0E {
                                // abs+Y
                                data = data.wrapping_add(y as u32);
                            } else if $m == 0x0D {
                                // abs+X
                                data = data.wrapping_add(x as u32);
                            }
                            // abs
                            pc = pc.wrapping_add(1);
                            data = data.wrapping_add(0x100 * read_pc!(pc));
                            true
                        }
                        0x0C | -4 => {
                            if $m == 0x0C {
                                // dp+X
                                data = data.wrapping_add(x as u32) as u8 as u32;
                            }
                            // dp
                            data = data.wrapping_add(dp as u32);
                            true
                        }
                        _ => false,
                    }
                }};
            }

            let op = opcode as i32;

            // 1. 8-bit Data Transmission Commands. Group I
            // (ADDR_MODES_NO_DP( 0xE8 ): MOV A,addr)
            if op != 0xE4 && addr_mode!(op - 0xE8) {
                a = read!(0, data) as u8;
                nz = a as i32;
                next = Next::IncPcLoop;
                continue 'stop;
            }
            // 2. 8-BIT DATA TRANSMISSION COMMANDS, GROUP 2
            // (ADDR_MODES_NO_DP( 0xC8 ): MOV addr,A)
            if op != 0xC4 && addr_mode!(op - 0xC8) {
                write!(0, data, a);
                next = Next::IncPcLoop;
                continue 'stop;
            }

            // 5. 8-BIT LOGIC OPERATION COMMANDS
            for base in [0x28, 0x08, 0x48] {
                let m = op - base;
                if matches!(
                    m,
                    -2 | 0x0F | -1 | 0x0E | 0x0D | -3 | 0x0C | -4 | 0 | 0x11 | 0x01 | 0x10
                ) {
                    macro_rules! func {
                        ($a:expr, $b:expr) => {
                            match base {
                                0x28 => ($a) & ($b), // AND
                                0x08 => ($a) | ($b), // OR
                                _ => ($a) ^ ($b),    // EOR
                            }
                        };
                    }
                    if m == 0 || addr_mode!(m) {
                        if m != 0 {
                            // addr
                            data = read!(0, data) as u32;
                        }
                        // imm
                        a = func!(a as u32, data) as u8;
                        nz = a as i32;
                        next = Next::IncPcLoop;
                        continue 'stop;
                    }
                    let addr: u32;
                    if m == 0x11 {
                        // X,Y
                        data = read_dp!(-2, y) as u32;
                        addr = (x as i32 + dp) as u32;
                    } else {
                        if m == 0x01 {
                            // dp,dp
                            data = read_dp!(-3, data) as u32;
                        }
                        // dp,imm
                        let addr2 = pc.wrapping_add(1);
                        pc = pc.wrapping_add(2);
                        addr = read_pc!(addr2) + dp as u32;
                    }
                    // addr_##op:
                    nz = func!(data, read!(-1, addr) as u32) as i32;
                    write!(0, addr, nz);
                    continue 'stop;
                }
            }

            // 4. 8-BIT ARITHMETIC OPERATION COMMANDS
            // (ADDR_MODES( 0x68 ): CMP addr)
            {
                let m = op - 0x68;
                if m == 0 || (m != 0x01 && m != 0x10 && m != 0x11 && addr_mode!(m)) {
                    if m != 0 {
                        data = read!(0, data) as u32;
                    }
                    // CMP imm
                    nz = (a as i32).wrapping_sub(data as i32);
                    c = !nz;
                    nz &= 0xFF;
                    next = Next::IncPcLoop;
                    continue 'stop;
                }
            }

            // ADC and SBC together, decoded later based on operand
            // (ADDR_MODES( 0x88 ) with CASE( n ): n and n + 0x20)
            {
                let base = if op & 0x20 != 0 { 0xA8 } else { 0x88 };
                let m = op - base;
                if matches!(op >> 4, 0x8..=0xB)
                    && matches!(
                        m,
                        -2 | 0x0F | -1 | 0x0E | 0x0D | -3 | 0x0C | -4 | 0 | 0x01 | 0x10 | 0x11
                    )
                {
                    let addr: i32;
                    if m == 0x11 || m == 0x01 || m == 0x10 {
                        if m == 0x11 {
                            // SBC (x),(y) / ADC (x),(y)
                            pc = pc.wrapping_sub(1); // compensate for inc later
                            data = read_dp!(-2, y) as u32;
                            addr = x as i32 + dp;
                        } else {
                            if m == 0x01 {
                                // SBC dp,dp / ADC dp,dp
                                data = read_dp!(-3, data) as u32;
                            }
                            // SBC dp,imm / ADC dp,imm
                            pc = pc.wrapping_add(1);
                            addr = read_pc!(pc) as i32 + dp;
                        }
                        // adc_addr:
                        nz = read!(-1, addr);
                    } else {
                        if m != 0 {
                            addr_mode!(m);
                            data = read!(0, data) as u32;
                        }
                        // SBC imm / ADC imm
                        addr = -1; // A
                        nz = a as i32;
                    }
                    // adc_data:
                    if opcode >= 0xA0 {
                        // SBC
                        data ^= 0xFF;
                    }

                    let mut flags = (data as i32) ^ nz;
                    nz = nz.wrapping_add((data as i32).wrapping_add(c >> 8 & 1));
                    flags ^= nz;

                    psw = (psw & !(V40 | H08))
                        | (flags >> 1 & H08)
                        | ((flags.wrapping_add(0x80)) >> 2 & V40);
                    c = nz;
                    if addr < 0 {
                        a = nz as u8;
                        next = Next::IncPcLoop;
                        continue 'stop;
                    }
                    write!(0, addr, /*(uint8_t)*/ nz);
                    next = Next::IncPcLoop;
                    continue 'stop;
                }
            }

            match opcode {
                0xF0 => branch!(nz as u8 == 0), // BEQ (89% taken)

                0xD0 => branch!(nz as u8 != 0), // BNE

                0x3F => {
                    // CALL
                    let old_addr = pc as i32 + 2;
                    pc = read_pc16!(pc) as u16;
                    push16!(old_addr);
                }

                0x6F => {
                    // RET
                    let l = pop!();
                    let h = pop!();
                    pc = l as u16 | (h as u16) << 8;
                }

                0xE4 => {
                    // MOV a,dp
                    pc = pc.wrapping_add(1);
                    // 80% from timer
                    nz = read_dp_timer!(0, data);
                    a = nz as u8;
                }

                0xFA | 0x8F => {
                    if opcode == 0xFA {
                        // MOV dp,dp
                        let temp = read_dp_timer!(-2, data);
                        data = (temp + NO_READ_BEFORE_WRITE) as u32;
                    }
                    // MOV dp,#imm
                    let temp = read_pc!(pc as u32 + 1) as i32;
                    pc = pc.wrapping_add(2);

                    // (#if !SPC_MORE_ACCURACY)
                    let mut i = dp + temp;
                    self.ram[i as usize] = data as u8;
                    i -= 0xF0;
                    if (i as u32) < 0x10 {
                        // 76%
                        self.smp_regs[0][i as usize] = data as u8;

                        // Registers other than $F2 and $F4-$F7
                        if i != 2 && (i < 4 || i > 7) {
                            // 12%
                            self.cpu_write_smp_reg(data as i32, rel_time, i as u16);
                        }
                    }
                }

                0xC4 => {
                    // MOV dp,a
                    pc = pc.wrapping_add(1);
                    // (#if !SPC_MORE_ACCURACY)
                    let mut i = dp + data as i32;
                    self.ram[i as usize] = a;
                    i -= 0xF0;
                    if (i as u32) < 0x10 {
                        // 39%
                        let sel = (i - 2) as u32;
                        self.smp_regs[0][i as usize] = a;

                        if sel == 1 {
                            // 51% $F3
                            self.dsp_write(a as i32, rel_time);
                        } else if sel > 1 {
                            // 1% not $F2 or $F3
                            self.cpu_write_smp_reg_(a as i32, rel_time, i as u16);
                        }
                    }
                }

                // 1. 8-bit Data Transmission Commands. Group I
                0xBF => {
                    // MOV A,(X)+
                    let temp = x as i32 + dp;
                    x = x.wrapping_add(1);
                    nz = read!(-1, temp);
                    a = nz as u8;
                }

                0xE8 => {
                    // MOV A,imm
                    a = data as u8;
                    nz = data as i32;
                    next = Next::IncPcLoop;
                }

                0xF9 | 0xF8 => {
                    if opcode == 0xF9 {
                        // MOV X,dp+Y
                        data = data.wrapping_add(y as u32) as u8 as u32;
                    }
                    // MOV X,dp
                    nz = read_dp_timer!(0, data);
                    x = nz as u8;
                    next = Next::IncPcLoop;
                }

                0xE9 | 0xCD => {
                    if opcode == 0xE9 {
                        // MOV X,abs
                        data = read_pc16!(pc);
                        pc = pc.wrapping_add(1);
                        data = read!(0, data) as u32;
                    }
                    // MOV X,imm
                    x = data as u8;
                    nz = data as i32;
                    next = Next::IncPcLoop;
                }

                0xFB | 0xEB => {
                    if opcode == 0xFB {
                        // MOV Y,dp+X
                        data = data.wrapping_add(x as u32) as u8 as u32;
                    }
                    // MOV Y,dp
                    // 70% from timer
                    pc = pc.wrapping_add(1);
                    nz = read_dp_timer!(0, data);
                    y = nz as u8;
                }

                0xEC => {
                    // MOV Y,abs
                    let temp = read_pc16!(pc);
                    pc = pc.wrapping_add(2);
                    nz = read_timer!(0, temp);
                    y = nz as u8;
                    //y = nz = READ( 0, temp );
                }

                0x8D => {
                    // MOV Y,imm
                    y = data as u8;
                    nz = data as i32;
                    next = Next::IncPcLoop;
                }

                // 2. 8-BIT DATA TRANSMISSION COMMANDS, GROUP 2
                0xCC | 0xC9 => {
                    // MOV abs,Y / MOV abs,X
                    let temp = if opcode == 0xCC { y } else { x };
                    // mov_abs_temp:
                    let addr = read_pc16!(pc);
                    write!(0, addr, temp);
                    pc = pc.wrapping_add(2);
                }

                0xD9 | 0xD8 => {
                    if opcode == 0xD9 {
                        // MOV dp+Y,X
                        data = data.wrapping_add(y as u32) as u8 as u32;
                    }
                    // MOV dp,X
                    write!(0, data.wrapping_add(dp as u32), x);
                    next = Next::IncPcLoop;
                }

                0xDB | 0xCB => {
                    if opcode == 0xDB {
                        // MOV dp+X,Y
                        data = data.wrapping_add(x as u32) as u8 as u32;
                    }
                    // MOV dp,Y
                    write!(0, data.wrapping_add(dp as u32), y);
                    next = Next::IncPcLoop;
                }

                // 3. 8-BIT DATA TRANSMISSIN COMMANDS, GROUP 3.
                0x7D => {
                    // MOV A,X
                    a = x;
                    nz = x as i32;
                }

                0xDD => {
                    // MOV A,Y
                    a = y;
                    nz = y as i32;
                }

                0x5D => {
                    // MOV X,A
                    x = a;
                    nz = a as i32;
                }

                0xFD => {
                    // MOV Y,A
                    y = a;
                    nz = a as i32;
                }

                0x9D => {
                    // MOV X,SP
                    x = sp;
                    nz = x as i32;
                }

                0xBD => {
                    // MOV SP,X
                    sp = x;
                }

                //case 0xC6: // MOV (X),A (handled by MOV addr,A in group 2)
                0xAF => {
                    // MOV (X)+,A
                    write_dp!(0, x, a as i32 + NO_READ_BEFORE_WRITE);
                    x = x.wrapping_add(1);
                }

                // 4. 8-BIT ARITHMETIC OPERATION COMMANDS
                0x79 => {
                    // CMP (X),(Y)
                    data = read_dp!(-2, y) as u32;
                    nz = read_dp!(-1, x).wrapping_sub(data as i32);
                    c = !nz;
                    nz &= 0xFF;
                }

                0x69 | 0x78 => {
                    if opcode == 0x69 {
                        // CMP dp,dp
                        data = read_dp!(-3, data) as u32;
                    }
                    // CMP dp,imm
                    pc = pc.wrapping_add(1);
                    let addr = read_pc!(pc);
                    nz = read_dp!(-1, addr).wrapping_sub(data as i32);
                    c = !nz;
                    nz &= 0xFF;
                    next = Next::IncPcLoop;
                }

                0x3E | 0x1E | 0xC8 => {
                    if opcode == 0x3E {
                        // CMP X,dp
                        data = data.wrapping_add(dp as u32);
                    } else if opcode == 0x1E {
                        // CMP X,abs
                        data = read_pc16!(pc);
                        pc = pc.wrapping_add(1);
                    }
                    if opcode != 0xC8 {
                        // cmp_x_addr:
                        data = read!(0, data) as u32;
                    }
                    // CMP X,imm
                    nz = (x as i32).wrapping_sub(data as i32);
                    c = !nz;
                    nz &= 0xFF;
                    next = Next::IncPcLoop;
                }

                0x7E | 0x5E | 0xAD => {
                    if opcode == 0x7E {
                        // CMP Y,dp
                        data = data.wrapping_add(dp as u32);
                    } else if opcode == 0x5E {
                        // CMP Y,abs
                        data = read_pc16!(pc);
                        pc = pc.wrapping_add(1);
                    }
                    if opcode != 0xAD {
                        // cmp_y_addr:
                        data = read!(0, data) as u32;
                    }
                    // CMP Y,imm
                    nz = (y as i32).wrapping_sub(data as i32);
                    c = !nz;
                    nz &= 0xFF;
                    next = Next::IncPcLoop;
                }

                // 6. ADDITION & SUBTRACTION COMMANDS
                0xBC => {
                    // INC A
                    nz = a as i32 + 1;
                    a = nz as u8;
                }
                0x3D => {
                    // INC X
                    nz = x as i32 + 1;
                    x = nz as u8;
                }
                0xFC => {
                    // INC Y
                    nz = y as i32 + 1;
                    y = nz as u8;
                }

                0x9C => {
                    // DEC A
                    nz = a as i32 - 1;
                    a = nz as u8;
                }
                0x1D => {
                    // DEC X
                    nz = x as i32 - 1;
                    x = nz as u8;
                }
                0xDC => {
                    // DEC Y
                    nz = y as i32 - 1;
                    y = nz as u8;
                }

                0x9B | 0xBB | 0x8B | 0xAB | 0x8C | 0xAC => {
                    match opcode {
                        0x9B | 0xBB | 0x8B | 0xAB => {
                            if opcode == 0x9B || opcode == 0xBB {
                                // DEC dp+X / INC dp+X
                                data = data.wrapping_add(x as u32) as u8 as u32;
                            }
                            // DEC dp / INC dp
                            data = data.wrapping_add(dp as u32);
                        }
                        _ => {
                            // DEC abs / INC abs
                            data = read_pc16!(pc);
                            pc = pc.wrapping_add(1);
                        }
                    }
                    // inc_abs:
                    nz = (opcode as i32 >> 4 & 2) - 1;
                    nz = nz.wrapping_add(read!(-1, data));
                    write!(0, data, /*(uint8_t)*/ nz);
                    next = Next::IncPcLoop;
                }

                // 7. SHIFT, ROTATION COMMANDS
                0x5C | 0x7C => {
                    if opcode == 0x5C {
                        // LSR A
                        c = 0;
                    }
                    // ROR A
                    nz = (c >> 1 & 0x80) | (a as i32 >> 1);
                    c = (a as i32) << 8;
                    a = nz as u8;
                }

                0x1C | 0x3C => {
                    if opcode == 0x1C {
                        // ASL A
                        c = 0;
                    }
                    // ROL A
                    let temp = c >> 8 & 1;
                    c = (a as i32) << 1;
                    nz = c | temp;
                    a = nz as u8;
                }

                0x0B | 0x1B | 0x3B | 0x2B | 0x0C | 0x2C => {
                    match opcode {
                        0x0B => {
                            // ASL dp
                            c = 0;
                            data = data.wrapping_add(dp as u32);
                        }
                        0x1B | 0x3B | 0x2B => {
                            if opcode == 0x1B {
                                // ASL dp+X
                                c = 0;
                            }
                            if opcode != 0x2B {
                                // ROL dp+X
                                data = data.wrapping_add(x as u32) as u8 as u32;
                            }
                            // ROL dp
                            data = data.wrapping_add(dp as u32);
                        }
                        _ => {
                            if opcode == 0x0C {
                                // ASL abs
                                c = 0;
                            }
                            // ROL abs
                            data = read_pc16!(pc);
                            pc = pc.wrapping_add(1);
                        }
                    }
                    // rol_mem:
                    nz = c >> 8 & 1;
                    c = read!(-1, data) << 1;
                    nz |= c;
                    write!(0, data, /*(uint8_t)*/ nz);
                    next = Next::IncPcLoop;
                }

                0x4B | 0x5B | 0x7B | 0x6B | 0x4C | 0x6C => {
                    match opcode {
                        0x4B => {
                            // LSR dp
                            c = 0;
                            data = data.wrapping_add(dp as u32);
                        }
                        0x5B | 0x7B | 0x6B => {
                            if opcode == 0x5B {
                                // LSR dp+X
                                c = 0;
                            }
                            if opcode != 0x6B {
                                // ROR dp+X
                                data = data.wrapping_add(x as u32) as u8 as u32;
                            }
                            // ROR dp
                            data = data.wrapping_add(dp as u32);
                        }
                        _ => {
                            if opcode == 0x4C {
                                // LSR abs
                                c = 0;
                            }
                            // ROR abs
                            data = read_pc16!(pc);
                            pc = pc.wrapping_add(1);
                        }
                    }
                    // ror_mem:
                    let temp = read!(-1, data);
                    nz = (c >> 1 & 0x80) | (temp >> 1);
                    c = temp << 8;
                    write!(0, data, nz);
                    next = Next::IncPcLoop;
                }

                0x9F => {
                    // XCN
                    a = (a >> 4) | (a << 4);
                    nz = a as i32;
                }

                // 8. 16-BIT TRANSMISION COMMANDS
                0xBA => {
                    // MOVW YA,dp
                    a = read_dp!(-2, data) as u8;
                    nz = (a as i32 & 0x7F) | (a as i32 >> 1);
                    y = read_dp!(0, data.wrapping_add(1) as u8) as u8;
                    nz |= y as i32;
                    next = Next::IncPcLoop;
                }

                0xDA => {
                    // MOVW dp,YA
                    write_dp!(-1, data, a);
                    write_dp!(
                        0,
                        data.wrapping_add(1) as u8,
                        y as i32 + NO_READ_BEFORE_WRITE
                    );
                    next = Next::IncPcLoop;
                }

                // 9. 16-BIT OPERATION COMMANDS
                0x3A | 0x1A => {
                    // INCW dp / DECW dp
                    // low byte
                    data = data.wrapping_add(dp as u32);
                    let mut temp = read!(-3, data);
                    temp += (opcode as i32 >> 4 & 2) - 1; // +1 for INCW, -1 for DECW
                    nz = ((temp >> 1) | temp) & 0x7F;
                    write!(-2, data, /*(uint8_t)*/ temp);

                    // high byte
                    data = (data.wrapping_add(1) as u8 as u32) + dp as u32;
                    temp = ((temp >> 8) + read!(-1, data)) as u8 as i32;
                    nz |= temp;
                    write!(0, data, temp);

                    next = Next::IncPcLoop;
                }

                0x7A | 0x9A => {
                    // ADDW YA,dp / SUBW YA,dp
                    let mut lo = read_dp!(-2, data);
                    let mut hi = read_dp!(0, data.wrapping_add(1) as u8);

                    if opcode == 0x9A {
                        // SUBW
                        lo = (lo ^ 0xFF) + 1;
                        hi ^= 0xFF;
                    }

                    lo += a as i32;
                    let mut result = y as i32 + hi + (lo >> 8);
                    let flags = hi ^ y as i32 ^ result;

                    psw = (psw & !(V40 | H08)) | (flags >> 1 & H08) | ((flags + 0x80) >> 2 & V40);
                    c = result;
                    a = lo as u8;
                    result = result as u8 as i32;
                    y = result as u8;
                    nz = (((lo >> 1) | lo) & 0x7F) | result;

                    next = Next::IncPcLoop;
                }

                0x5A => {
                    // CMPW YA,dp
                    let mut temp = a as i32 - read_dp!(-1, data);
                    nz = ((temp >> 1) | temp) & 0x7F;
                    temp = y as i32 + (temp >> 8);
                    temp -= read_dp!(0, data.wrapping_add(1) as u8);
                    nz |= temp;
                    c = !temp;
                    nz &= 0xFF;
                    next = Next::IncPcLoop;
                }

                // 10. MULTIPLICATION & DIVISON COMMANDS
                0xCF => {
                    // MUL YA
                    let temp: u32 = y as u32 * a as u32;
                    a = temp as u8;
                    nz = (((temp >> 1) | temp) & 0x7F) as i32;
                    y = (temp >> 8) as u8;
                    nz |= y as i32;
                }

                0x9E => {
                    // DIV YA,X
                    let ya: u32 = y as u32 * 0x100 + a as u32;

                    psw &= !(H08 | V40);

                    if y >= x {
                        psw |= V40;
                    }

                    if (y & 15) >= (x & 15) {
                        psw |= H08;
                    }

                    if (y as u32) < x as u32 * 2 {
                        a = (ya / x as u32) as u8;
                        y = ya.wrapping_sub(a as u32 * x as u32) as u8;
                    } else {
                        a = 255u32
                            .wrapping_sub(ya.wrapping_sub(x as u32 * 0x200) / (256 - x as u32))
                            as u8;
                        y = (x as u32 + ya.wrapping_sub(x as u32 * 0x200) % (256 - x as u32)) as u8;
                    }

                    nz = a as i32;
                }

                // 11. DECIMAL COMPENSATION COMMANDS
                0xDF => {
                    // DAA
                    // SUSPICIOUS_OPCODE( "DAA" );
                    if a > 0x99 || c & 0x100 != 0 {
                        a = a.wrapping_add(0x60);
                        c = 0x100;
                    }

                    if (a & 0x0F) > 9 || psw & H08 != 0 {
                        a = a.wrapping_add(0x06);
                    }

                    nz = a as i32;
                }

                0xBE => {
                    // DAS
                    // SUSPICIOUS_OPCODE( "DAS" );
                    if a > 0x99 || c & 0x100 == 0 {
                        a = a.wrapping_sub(0x60);
                        c = 0;
                    }

                    if (a & 0x0F) > 9 || psw & H08 == 0 {
                        a = a.wrapping_sub(0x06);
                    }

                    nz = a as i32;
                }

                // 12. BRANCHING COMMANDS
                0x2F => {
                    // BRA rel
                    pc = pc.wrapping_add(data as u8 as i8 as u16);
                    next = Next::IncPcLoop;
                }

                0x30 => branch!(nz & NZ_NEG_MASK != 0), // BMI

                0x10 => branch!(nz & NZ_NEG_MASK == 0), // BPL

                0xB0 => branch!(c & 0x100 != 0), // BCS

                0x90 => branch!(c & 0x100 == 0), // BCC

                0x70 => branch!(psw & V40 != 0), // BVS

                0x50 => branch!(psw & V40 == 0), // BVC

                0x03 | 0x23 | 0x43 | 0x63 | 0x83 | 0xA3 | 0xC3 | 0xE3 => {
                    // BBS dp.bit,rel
                    let v = read_dp!(-4, data);
                    cbranch!(v >> (opcode >> 5) & 1 != 0)
                }

                0x13 | 0x33 | 0x53 | 0x73 | 0x93 | 0xB3 | 0xD3 | 0xF3 => {
                    // BBC dp.bit,rel
                    let v = read_dp!(-4, data);
                    cbranch!(v >> (opcode >> 5) & 1 == 0)
                }

                0xDE | 0x2E => {
                    if opcode == 0xDE {
                        // CBNE dp+X,rel
                        data = data.wrapping_add(x as u32) as u8 as u32;
                    }
                    // CBNE dp,rel
                    // 61% from timer
                    let temp = read_dp_timer!(-4, data);
                    cbranch!(temp != a as i32)
                }

                0x6E => {
                    // DBNZ dp,rel
                    let temp = (read_dp!(-4, data) as u32).wrapping_sub(1);
                    write_dp!(
                        -3,
                        data as u8,
                        /*(uint8_t)*/ temp.wrapping_add(NO_READ_BEFORE_WRITE as u32)
                    );
                    cbranch!(temp != 0)
                }

                0xFE => {
                    // DBNZ Y,rel
                    y = y.wrapping_sub(1);
                    branch!(y != 0)
                }

                0x1F | 0x5F => {
                    if opcode == 0x1F {
                        // JMP [abs+X]
                        pc = (read_pc16!(pc) + x as u32) as u16;
                    }
                    // JMP abs
                    pc = read_pc16!(pc) as u16;
                }

                // 13. SUB-ROUTINE CALL RETURN COMMANDS
                0x0F => {
                    // BRK
                    let ret_addr = pc as i32;
                    // SUSPICIOUS_OPCODE( "BRK" );
                    pc = self.read_prog16(0xFFDE) as u16; // vector address verified
                    push16!(ret_addr);
                    let temp = get_psw!();
                    psw = (psw | B10) & !I04;
                    push!(temp);
                }

                0x4F => {
                    // PCALL offset
                    let ret_addr = pc as i32 + 1;
                    pc = (0xFF00 | data) as u16;
                    push16!(ret_addr);
                }

                0x01 | 0x11 | 0x21 | 0x31 | 0x41 | 0x51 | 0x61 | 0x71 | 0x81 | 0x91 | 0xA1
                | 0xB1 | 0xC1 | 0xD1 | 0xE1 | 0xF1 => {
                    // TCALL n
                    let ret_addr = pc as i32;
                    pc = self.read_prog16(0xFFDE - (opcode >> 3)) as u16;
                    push16!(ret_addr);
                }

                // 14. STACK OPERATION COMMANDS
                0x7F | 0x8E => {
                    let temp;
                    if opcode == 0x7F {
                        // RET1
                        temp = pop!();
                        let l = pop!();
                        let h = pop!();
                        pc = l as u16 | (h as u16) << 8;
                    } else {
                        // POP PSW
                        temp = pop!();
                    }
                    // set_psw:
                    set_psw!(temp as i32);
                }

                0x0D => {
                    // PUSH PSW
                    let temp = get_psw!();
                    push!(temp);
                }

                0x2D => push!(a), // PUSH A

                0x4D => push!(x), // PUSH X

                0x6D => push!(y), // PUSH Y

                0xAE => a = pop!(), // POP A

                0xCE => x = pop!(), // POP X

                0xEE => y = pop!(), // POP Y

                // 15. BIT OPERATION COMMANDS
                0x02 | 0x22 | 0x42 | 0x62 | 0x82 | 0xA2 | 0xC2 | 0xE2 | 0x12 | 0x32 | 0x52
                | 0x72 | 0x92 | 0xB2 | 0xD2 | 0xF2 => {
                    // SET1 / CLR1
                    let mut bit = 1 << (opcode >> 5);
                    let mask = !bit;
                    if opcode & 0x10 != 0 {
                        bit = 0;
                    }
                    data = data.wrapping_add(dp as u32);
                    let v = (read!(-1, data) & mask) | bit;
                    write!(0, data, v);
                    next = Next::IncPcLoop;
                }

                0x0E | 0x4E => {
                    // TSET1 abs / TCLR1 abs
                    data = read_pc16!(pc);
                    pc = pc.wrapping_add(2);
                    {
                        let mut temp = read!(-2, data) as u32;
                        nz = (a as u32).wrapping_sub(temp) as u8 as i32;
                        temp &= !(a as u32);
                        if opcode == 0x0E {
                            temp |= a as u32;
                        }
                        write!(0, data, temp);
                    }
                }

                0x4A => {
                    // AND1 C,mem.bit
                    c &= mem_bit!(0) as i32;
                    pc = pc.wrapping_add(2);
                }

                0x6A => {
                    // AND1 C,/mem.bit
                    c &= !mem_bit!(0) as i32;
                    pc = pc.wrapping_add(2);
                }

                0x0A => {
                    // OR1 C,mem.bit
                    c |= mem_bit!(-1) as i32;
                    pc = pc.wrapping_add(2);
                }

                0x2A => {
                    // OR1 C,/mem.bit
                    c |= !mem_bit!(-1) as i32;
                    pc = pc.wrapping_add(2);
                }

                0x8A => {
                    // EOR1 C,mem.bit
                    c ^= mem_bit!(-1) as i32;
                    pc = pc.wrapping_add(2);
                }

                0xEA => {
                    // NOT1 mem.bit
                    data = read_pc16!(pc);
                    pc = pc.wrapping_add(2);
                    {
                        let mut temp = read!(-1, data & 0x1FFF) as u32;
                        temp ^= 1 << (data >> 13);
                        write!(0, data & 0x1FFF, temp);
                    }
                }

                0xCA => {
                    // MOV1 mem.bit,C
                    data = read_pc16!(pc);
                    pc = pc.wrapping_add(2);
                    {
                        let mut temp = read!(-2, data & 0x1FFF) as u32;
                        let bit = data >> 13;
                        temp = (temp & !(1 << bit)) | (((c >> 8 & 1) as u32) << bit);
                        write!(
                            0,
                            data & 0x1FFF,
                            temp.wrapping_add(NO_READ_BEFORE_WRITE as u32)
                        );
                    }
                }

                0xAA => {
                    // MOV1 C,mem.bit
                    c = mem_bit!(0) as i32;
                    pc = pc.wrapping_add(2);
                }

                // 16. PROGRAM PSW FLAG OPERATION COMMANDS
                0x60 => c = 0, // CLRC

                0x80 => c = !0, // SETC

                0xED => c ^= 0x100, // NOTC

                0xE0 => psw &= !(V40 | H08), // CLRV

                0x20 => dp = 0, // CLRP

                0x40 => dp = 0x100, // SETP

                0xA0 => {
                    // EI
                    // SUSPICIOUS_OPCODE( "EI" );
                    psw |= I04;
                }

                0xC0 => {
                    // DI
                    // SUSPICIOUS_OPCODE( "DI" );
                    psw &= !I04;
                }

                // 17. OTHER COMMANDS
                0x00 => {} // NOP

                0xFF | 0xEF => {
                    // STOP
                    // handle PC wrap-around
                    if opcode == 0xFF && pc == 0x0000 {
                        // debug_printf( "SPC: PC wrapped around\n" );
                        continue 'stop;
                    }
                    // SLEEP
                    // SUSPICIOUS_OPCODE( "STOP/SLEEP" );
                    pc = pc.wrapping_sub(1);
                    rel_time = 0;
                    self.cpu_error = Some("SPC emulation error");
                    break 'stop;
                }

                _ => {
                    // (every other opcode is one of the groups above;
                    // upstream would go on to out_of_time)
                    rel_time -= self.cycle_table[ram!(pc) as usize] as i32;
                    break 'stop;
                }
            }
        }
        // stop:

        // Uncache registers
        self.cpu_regs.pc = pc;
        self.cpu_regs.sp = sp;
        self.cpu_regs.a = a;
        self.cpu_regs.x = x;
        self.cpu_regs.y = y;
        {
            let temp = get_psw!();
            self.cpu_regs.psw = temp as u8;
        }

        // SPC_CPU_RUN_FUNC_END
        self.spc_time += rel_time;
        self.dsp_time -= rel_time;
        self.timers[0].next_time -= rel_time;
        self.timers[1].next_time -= rel_time;
        self.timers[2].next_time -= rel_time;
        // assert( m.spc_time <= end_time );
    }
}
