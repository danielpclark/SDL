// Rust translation of src/libFLAC/crc.c (and include/private/crc.h) from
// libFLAC 1.3.4 (https://xiph.org/flac/, at the revision SDL_mixer's
// external/flac pins).
// Copyright (C) 2000-2009  Josh Coalson
// Copyright (C) 2011-2018  Xiph.Org Foundation
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The CRC-8 of frame headers and the CRC-16 of frames. The tables are
//! computed at compile time (as the `#if 0` `FLAC__crc16_init_table()`
//! does) rather than spelled out; a test checks them against upstream's.

/// CRC-8, poly = x^8 + x^2 + x^1 + x^0, init = 0
pub(crate) static FLAC__crc8_table: [u8; 256] = crc8_init_table();

const fn crc8_init_table() -> [u8; 256] {
    let mut table = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u8;
        let mut j = 0;
        while j < 8 {
            crc = (crc << 1) ^ if crc & 0x80 != 0 { 0x07 } else { 0 };
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

/// CRC-16, poly = x^16 + x^15 + x^2 + x^0, init = 0
pub(crate) static FLAC__crc16_table: [[u16; 256]; 8] = crc16_init_table();

/// `FLAC__crc16_init_table()` (`#if 0` upstream, which spells the tables
/// out).
const fn crc16_init_table() -> [[u16; 256]; 8] {
    let mut table = [[0u16; 256]; 8];
    let polynomial: u16 = 0x8005;

    let mut i = 0;
    while i <= 0xFF {
        let mut crc = (i as u16) << 8;

        let mut j = 0;
        while j < 8 {
            crc = (crc << 1) ^ if crc & (1 << 15) != 0 { polynomial } else { 0 };
            j += 1;
        }

        table[0][i] = crc;
        i += 1;
    }

    let mut i = 0;
    while i <= 0xFF {
        let mut j = 1;
        while j < 8 {
            table[j][i] = table[0][(table[j - 1][i] >> 8) as usize] ^ (table[j - 1][i] << 8);
            j += 1;
        }
        i += 1;
    }
    table
}

/// `FLAC__CRC16_UPDATE(data, crc)`.
#[inline]
pub(crate) fn FLAC__CRC16_UPDATE(data: u32, crc: u32) -> u32 {
    ((crc << 8) & 0xffff) ^ FLAC__crc16_table[0][((crc >> 8) ^ data) as usize] as u32
}

/// 8 bit CRC generator, MSB shifted first
/// polynomial = x^8 + x^2 + x^1 + x^0
/// init = 0
pub(crate) fn FLAC__crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0;

    for &b in data {
        crc = FLAC__crc8_table[(crc ^ b) as usize];
    }

    crc
}

/// `FLAC__crc16_update_words32()`.
pub(crate) fn FLAC__crc16_update_words32(words: &[u32], mut crc: u16) -> u16 {
    let t = &FLAC__crc16_table;
    let mut words = words;
    while words.len() >= 2 {
        crc ^= (words[0] >> 16) as u16;

        crc = t[7][(crc >> 8) as usize]
            ^ t[6][(crc & 0xFF) as usize]
            ^ t[5][((words[0] >> 8) & 0xFF) as usize]
            ^ t[4][(words[0] & 0xFF) as usize]
            ^ t[3][(words[1] >> 24) as usize]
            ^ t[2][((words[1] >> 16) & 0xFF) as usize]
            ^ t[1][((words[1] >> 8) & 0xFF) as usize]
            ^ t[0][(words[1] & 0xFF) as usize];

        words = &words[2..];
    }

    if !words.is_empty() {
        crc ^= (words[0] >> 16) as u16;

        crc = t[3][(crc >> 8) as usize]
            ^ t[2][(crc & 0xFF) as usize]
            ^ t[1][((words[0] >> 8) & 0xFF) as usize]
            ^ t[0][(words[0] & 0xFF) as usize];
    }

    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fnv(vals: impl Iterator<Item = u64>) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for v in vals {
            h ^= v;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    /// The tables are upstream's (FNV-1a over crc.c's spelled-out values).
    #[test]
    fn tables_match_upstream() {
        assert_eq!(
            fnv(FLAC__crc8_table.iter().map(|&v| v as u64)),
            0x5c91_7c4d_34dc_5c65
        );
        assert_eq!(
            fnv(FLAC__crc16_table.iter().flatten().map(|&v| v as u64)),
            0xa5a2_f977_f4dd_2865
        );
    }

    #[test]
    fn word_update_matches_bytes() {
        let bytes = [0x12u8, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 1, 2, 3, 4];
        let mut crc = 0u32;
        for &b in &bytes {
            crc = FLAC__CRC16_UPDATE(b as u32, crc);
        }
        let words: Vec<u32> = bytes
            .chunks(4)
            .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        assert_eq!(FLAC__crc16_update_words32(&words, 0) as u32, crc);
    }
}
