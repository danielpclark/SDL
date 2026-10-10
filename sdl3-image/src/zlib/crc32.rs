// Rust translation of crc32.c from zlib 1.3.1.
// Copyright (C) 1995-2022 Mark Adler
// This is an altered (translated) version of the original software; zlib is
// used under the zlib license (see zlib.h and LICENSE.txt).

//! crc32.c -- compute the CRC-32 of a data stream (with the byte-wise
//! table loop, which computes the same CRC as zlib's braided one)

/// The CRC-32 table (`crc_table`, polynomial 0xedb88320, reflected).
const fn make_crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut p = i as u32;
        let mut k = 0;
        while k < 8 {
            p = if p & 1 != 0 {
                (p >> 1) ^ 0xedb88320
            } else {
                p >> 1
            };
            k += 1;
        }
        table[i] = p;
        i += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = make_crc_table();

/// `crc32_z` (and `crc32`): `None` is C's `Z_NULL` buffer.
pub(crate) fn crc32(crc: u64, buf: Option<&[u8]>) -> u64 {
    /* Return initial CRC, if requested. */
    let Some(buf) = buf else {
        return 0;
    };

    /* Pre-condition the CRC */
    let mut crc = (crc as u32) ^ 0xffffffff;

    /* Complete the computation of the CRC on any remaining bytes. */
    for &b in buf {
        crc = (crc >> 8) ^ CRC_TABLE[((crc ^ b as u32) & 0xff) as usize];
    }

    /* Return the CRC, post-conditioned. */
    (crc ^ 0xffffffff) as u64
}
