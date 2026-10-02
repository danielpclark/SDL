// Rust translation of src/stdlib/SDL_crc32.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Public domain CRC implementation adapted from:
   http://home.thep.lu.se/~bjorn/crc/crc32_simple.c

   This algorithm is compatible with the 32-bit CRC described here:
   https://www.lammertbies.nl/comm/info/crc-calculation
*/

/* NOTE: DO NOT CHANGE THIS ALGORITHM
   There is code that relies on this in the joystick code
*/

fn crc32_for_byte(mut r: u32) -> u32 {
    for _ in 0..8 {
        r = (if r & 1 != 0 { 0 } else { 0xEDB88320u32 }) ^ (r >> 1);
    }
    r ^ 0xFF000000u32
}

/// Calculate a CRC-32 value.
///
/// This function can be called multiple times, to stream data to be
/// checksummed in blocks. Each call must provide the previous CRC-32 return
/// value to be updated with the next block.
///
/// Translation of `SDL_crc32()`.
pub fn crc32(mut crc: u32, data: &[u8]) -> u32 {
    // As an optimization we can precalculate a 256 entry table for each byte
    for &b in data {
        crc = crc32_for_byte(((crc as u8) ^ b) as u32) ^ (crc >> 8);
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector() {
        // Standard CRC-32 of "123456789" is 0xCBF43926.
        assert_eq!(crc32(0, b"123456789"), 0xCBF43926);
        let a = crc32(0, b"12345");
        assert_eq!(crc32(a, b"6789"), 0xCBF43926);
    }
}
