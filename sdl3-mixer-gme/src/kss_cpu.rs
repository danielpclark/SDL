// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/

/*
Last validated with zexall 2006.11.14 2:19 PM
* Doesn't implement the R register or immediate interrupt after EI.
* Address wrap-around isn't completely correct, but is prevented from crashing emulator.
*/

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

//! Z80 CPU emulator for the KSS files (`Kss_Cpu.h`, `Kss_Cpu.cpp`): its
//! paged memory map. `Kss_Cpu::run()` is `Ay_Cpu::run()` over this map
//! (its own copy upstream, with a program counter wider than 16 bits that
//! stops at the idle address past the end of memory): it is
//! [`crate::ay_cpu::run`], with the KSS emulator as its host.

pub use crate::ay_cpu::{AyCpu as Z80, CpuTime};

/// Map memory. Start and size must be multiple of page_size.
pub const PAGE_SIZE: u32 = 0x2000;
pub const PAGE_SHIFT: u32 = 13;
pub const PAGE_COUNT: usize = 0x10000 >> PAGE_SHIFT;

pub const IDLE_ADDR: u32 = 0xFFFF;

/// can read this far past end of a page
pub const CPU_PADDING: usize = 0x100;

/// A page of memory: a memory region of the emulator's and the offset of
/// the page's start in it (upstream's pointers).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub mem: u8,
    pub offset: usize,
}

#[derive(Clone, Debug)]
pub struct KssCpu {
    pub z80: Z80,
    // state_t (with the Z80's time)
    read: [Page; PAGE_COUNT + 1],
    write: [Page; PAGE_COUNT + 1],
}

impl Default for KssCpu {
    fn default() -> Self {
        Self::new()
    }
}

impl KssCpu {
    pub fn new() -> Self {
        KssCpu {
            z80: Z80::new(),
            read: [Page::default(); PAGE_COUNT + 1],
            write: [Page::default(); PAGE_COUNT + 1],
        }
    }

    #[inline]
    fn set_page(&mut self, i: usize, write: Page, read: Page) {
        // (minus KSS_CPU_PAGE_OFFSET( i * page_size ), which is 0)
        self.write[i] = write;
        self.read[i] = read;
    }

    /// Clear registers and map all pages to unmapped
    pub fn reset(&mut self, unmapped_write: Page, unmapped_read: Page) {
        self.z80.reset();

        for i in 0..PAGE_COUNT + 1 {
            self.set_page(i, unmapped_write, unmapped_read);
        }
    }

    /// Map memory. Start and size must be multiple of page_size.
    pub fn map_mem(&mut self, addr: u32, size: u32, write: Page, read: Page) {
        // address range must begin and end on page boundaries
        // require( addr % page_size == 0 );
        // require( size % page_size == 0 );

        let first_page = (addr / PAGE_SIZE) as usize;
        let mut i = size / PAGE_SIZE;
        while i > 0 {
            i -= 1;
            let offset = (i * PAGE_SIZE) as usize;
            self.set_page(
                first_page + i as usize,
                Page {
                    mem: write.mem,
                    offset: write.offset + offset,
                },
                Page {
                    mem: read.mem,
                    offset: read.offset + offset,
                },
            );
        }
    }

    /// Map address to page (`write()`: the region and offset of the byte)
    #[inline]
    pub fn write(&self, addr: u32) -> Page {
        let p = self.write[(addr >> PAGE_SHIFT) as usize];
        Page {
            mem: p.mem,
            offset: p.offset + (addr & (PAGE_SIZE - 1)) as usize,
        }
    }

    /// `read()`: the region and offset of the byte
    #[inline]
    pub fn read(&self, addr: u32) -> Page {
        let p = self.read[(addr >> PAGE_SHIFT) as usize];
        Page {
            mem: p.mem,
            offset: p.offset + (addr & (PAGE_SIZE - 1)) as usize,
        }
    }
}
