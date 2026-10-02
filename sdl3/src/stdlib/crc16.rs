// Rust translation of src/stdlib/SDL_crc16.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Public domain CRC implementation adapted from:
   http://home.thep.lu.se/~bjorn/crc/crc32_simple.c

   This algorithm is compatible with the 16-bit CRC described here:
   https://www.lammertbies.nl/comm/info/crc-calculation
*/

/* NOTE: DO NOT CHANGE THIS ALGORITHM
   There is code that relies on this in the joystick code
*/

fn crc16_for_byte(mut r: u8) -> u16 {
    let mut crc: u16 = 0;
    for _ in 0..8 {
        crc = (if (crc ^ r as u16) & 1 != 0 { 0xA001 } else { 0 }) ^ (crc >> 1);
        r >>= 1;
    }
    crc
}

/// Calculate a CRC-16 value.
///
/// This function can be called multiple times, to stream data to be
/// checksummed in blocks. Each call must provide the previous CRC-16 return
/// value to be updated with the next block.
///
/// Translation of `SDL_crc16()`.
pub fn crc16(mut crc: u16, data: &[u8]) -> u16 {
    // As an optimization we can precalculate a 256 entry table for each byte
    for &b in data {
        crc = crc16_for_byte((crc as u8) ^ b) ^ (crc >> 8);
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector() {
        // CRC-16/ARC ("CRC-16/IBM") of "123456789" is 0xBB3D.
        assert_eq!(crc16(0, b"123456789"), 0xBB3D);
        // Streaming must equal one-shot.
        let a = crc16(0, b"1234");
        assert_eq!(crc16(a, b"56789"), 0xBB3D);
    }
}
