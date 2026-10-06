// Rust translation of src/test/SDL_test_crc32.c and include/SDL3/SDL_test_crc32.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! CRC32 functions of SDL test framework.
//!
//! Implements CRC32 calculations (default output is Perl String::CRC32
//! compatible).
//!
//! Used by the test execution component.
//! Original source code contributed by A. Schiffler for GSOC project.
//!
//! Upstream can also be built with `ORIGINAL_METHOD`, a table for the
//! AUTODIN II, Ethernet, & FDDI polynomial (`0x04c11db7`) shifted the other
//! way; like upstream's default build, this translates the Perl-compatible
//! method only.

/* ------------ Definitions --------- */

/// Translation of `CRC32_POLY` (Perl String::CRC32 compatible).
pub const CRC32_POLY: u32 = 0xEDB88320;

/// Data structure for CRC32 (checksum) computation: the CRC table.
/// Translation of `SDLTest_Crc32Context`; there is nothing to clean up
/// (`SDLTest_Crc32Done()` only checks its argument).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Crc32Context {
    /// CRC table
    crc32_table: [u32; 256],
}

impl Default for Crc32Context {
    fn default() -> Self {
        Crc32Context::new()
    }
}

impl Crc32Context {
    /// Initialize the CRC context: the function initializes the crc table
    /// required for all crc calculations. Translation of `SDLTest_Crc32Init()`.
    pub const fn new() -> Crc32Context {
        let mut crc32_table = [0u32; 256];

        /*
         * Build auxiliary table for parallel byte-at-a-time CRC-32
         */
        let mut i = 0;
        while i < 256 {
            let mut c = i as u32;
            let mut j = 8;
            while j > 0 {
                if c & 1 != 0 {
                    c = (c >> 1) ^ CRC32_POLY;
                } else {
                    c >>= 1;
                }
                j -= 1;
            }
            crc32_table[i] = c;
            i += 1;
        }

        Crc32Context { crc32_table }
    }

    /// The CRC table.
    pub const fn table(&self) -> &[u32; 256] {
        &self.crc32_table
    }

    /// Complete CRC32 calculation on a memory block. Translation of
    /// `SDLTest_Crc32Calc()`.
    pub fn calc(&self, in_buf: &[u8]) -> u32 {
        let crc32 = self.calc_start();
        let crc32 = self.calc_buffer(in_buf, crc32);
        self.calc_end(crc32)
    }

    /// Start crc calculation: the value to pass to the first
    /// [`calc_buffer`](Self::calc_buffer). Translation of
    /// `SDLTest_Crc32CalcStart()`.
    pub fn calc_start(&self) -> u32 {
        /*
         * Preload shift register, per CRC-32 spec
         */
        0xffffffff
    }

    /// Finish crc calculation. Translation of `SDLTest_Crc32CalcEnd()`.
    pub fn calc_end(&self, crc32: u32) -> u32 {
        /*
         * Return complement, per CRC-32 spec
         */
        !crc32
    }

    /// Include memory block in crc. Translation of `SDLTest_Crc32CalcBuffer()`.
    pub fn calc_buffer(&self, in_buf: &[u8], crc32: u32) -> u32 {
        /*
         * Calculate CRC from data
         */
        let mut crc = crc32;
        for &p in in_buf {
            crc = ((crc >> 8) & 0x00FFFFFF) ^ self.crc32_table[((crc ^ p as u32) & 0xFF) as usize];
        }
        crc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table() {
        let context = Crc32Context::new();
        // The first entries of the reflected CRC-32 table.
        assert_eq!(
            context.table()[..8],
            [
                0x00000000, 0x77073096, 0xEE0E612C, 0x990951BA, 0x076DC419, 0x706AF48F, 0xE963A535,
                0x9E6495A3
            ]
        );
        assert_eq!(context.table()[255], 0x2D02EF8D);
    }

    #[test]
    fn known_vectors() {
        let context = Crc32Context::new();
        // The CRC-32 check value, and String::CRC32's documented vectors.
        assert_eq!(context.calc(b"123456789"), 0xCBF43926);
        assert_eq!(context.calc(b""), 0);
        assert_eq!(context.calc(b"a"), 0xE8B7BE43);
        assert_eq!(
            context.calc(b"The quick brown fox jumps over the lazy dog"),
            0x414FA339
        );
        // From upstream's C.
        assert_eq!(context.calc(b"ABCDEFGHIJKLMNOPQRSTUVWXYZ"), 0xABF77822);

        // In pieces.
        let crc = context.calc_start();
        let crc = context.calc_buffer(b"12345", crc);
        let crc = context.calc_buffer(b"6789", crc);
        assert_eq!(context.calc_end(crc), 0xCBF43926);

        // The same as sdl3's own SDL_crc32().
        assert_eq!(
            context.calc(b"123456789"),
            sdl3::stdlib::crc32(0, b"123456789")
        );
    }
}
