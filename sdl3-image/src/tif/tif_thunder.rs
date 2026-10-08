// Rust translation of libtiff/tif_thunder.c from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1988-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! ThunderScan 4-bit Compression Algorithm Support
//!
//! ThunderScan uses an encoding scheme designed for
//! 4-bit pixel values.  Data is encoded in bytes, with
//! each byte split into a 2-bit code word and a 6-bit
//! data value.  The encoding gives raw data, runs of
//! pixels, or pixel values encoded as a delta from the
//! previous pixel value.  For the latter, either 2-bit
//! or 3-bit delta values are used, with the deltas packed
//! into a single byte.

use super::tif_error::tiff_error_ext_r;
use super::tiffiop::{Tiff, TmSize};

// #define THUNDER_DATA 0x3f /* mask for 6-bit data */
const THUNDER_CODE: i32 = 0xc0; /* mask for 2-bit code word */
/* code values */
const THUNDER_RUN: i32 = 0x00; /* run of pixels w/ encoded count */
const THUNDER_2BITDELTAS: i32 = 0x40; /* 3 pixels w/ encoded 2-bit deltas */
const DELTA2_SKIP: i32 = 2; /* skip code for 2-bit deltas */
const THUNDER_3BITDELTAS: i32 = 0x80; /* 2 pixels w/ encoded 3-bit deltas */
const DELTA3_SKIP: i32 = 4; /* skip code for 3-bit deltas */
const THUNDER_RAW: i32 = 0xc0; /* raw data encoded */

static TWOBITDELTAS: [i32; 4] = [0, 1, 0, -1];
static THREEBITDELTAS: [i32; 8] = [0, 1, 2, 3, 0, -3, -2, -1];

/// Translation of `ThunderSetupDecode()`.
fn thunder_setup_decode(tif: &mut Tiff<'_>) -> i32 {
    const MODULE: &str = "ThunderSetupDecode";

    if tif.tif_dir.td_bitspersample != 4 {
        tiff_error_ext_r!(
            MODULE,
            "Wrong bitspersample value ({}), Thunder decoder only supports 4bits per sample.",
            tif.tif_dir.td_bitspersample as i32
        );
        return 0;
    }

    1
}

/// The state of `ThunderDecode()`'s `SETPIXEL()`.
struct Pixels<'b> {
    row: &'b mut [u8],
    op: usize,
    lastpixel: u32,
    npixels: TmSize,
    maxpixels: TmSize,
}

impl Pixels<'_> {
    /// Translation of `SETPIXEL()`.
    fn setpixel(&mut self, v: u32) {
        self.lastpixel = v & 0xf;
        if self.npixels < self.maxpixels {
            let npixels = self.npixels;
            self.npixels += 1;
            if npixels & 1 != 0 {
                if let Some(b) = self.row.get_mut(self.op) {
                    *b |= self.lastpixel as u8;
                }
                self.op += 1;
            } else if let Some(b) = self.row.get_mut(self.op) {
                *b = (self.lastpixel << 4) as u8;
            }
        }
    }
}

/// Translation of `ThunderDecode()`.
fn thunder_decode(tif: &mut Tiff<'_>, op0: &mut [u8], maxpixels: TmSize) -> i32 {
    const MODULE: &str = "ThunderDecode";

    let mut bp = tif.tif_rawcp;
    let mut cc = tif.tif_rawcc;
    let mut p = Pixels {
        row: op0,
        op: 0,
        lastpixel: 0,
        npixels: 0,
        maxpixels,
    };
    while cc > 0 && p.npixels < maxpixels {
        let mut n = tif.tif_rawdata.get(bp).copied().unwrap_or(0) as i32;
        bp += 1;
        cc -= 1;
        match n & THUNDER_CODE {
            THUNDER_RUN => {
                /* pixel run */
                /*
                 * Replicate the last pixel n times,
                 * where n is the lower-order 6 bits.
                 */
                if n == 0 {
                    continue;
                }
                if p.npixels & 1 != 0 {
                    if let Some(b) = p.row.get_mut(p.op) {
                        *b |= p.lastpixel as u8;
                        p.lastpixel = *b as u32;
                    }
                    p.op += 1;
                    p.npixels += 1;
                    n -= 1;
                } else {
                    p.lastpixel |= p.lastpixel << 4;
                }
                p.npixels += n as TmSize;
                if p.npixels > maxpixels {
                    continue;
                }
                while n > 0 {
                    if let Some(b) = p.row.get_mut(p.op) {
                        *b = p.lastpixel as u8;
                    }
                    p.op += 1;
                    n -= 2;
                }
                if n == -1 {
                    p.op -= 1;
                    if let Some(b) = p.row.get_mut(p.op) {
                        *b &= 0xf0;
                    }
                }
                p.lastpixel &= 0xf;
            }
            THUNDER_2BITDELTAS => {
                /* 2-bit deltas */
                let delta = (n >> 4) & 3;
                if delta != DELTA2_SKIP {
                    p.setpixel((p.lastpixel as i32 + TWOBITDELTAS[delta as usize]) as u32);
                }
                let delta = (n >> 2) & 3;
                if delta != DELTA2_SKIP {
                    p.setpixel((p.lastpixel as i32 + TWOBITDELTAS[delta as usize]) as u32);
                }
                let delta = n & 3;
                if delta != DELTA2_SKIP {
                    p.setpixel((p.lastpixel as i32 + TWOBITDELTAS[delta as usize]) as u32);
                }
            }
            THUNDER_3BITDELTAS => {
                /* 3-bit deltas */
                let delta = (n >> 3) & 7;
                if delta != DELTA3_SKIP {
                    p.setpixel((p.lastpixel as i32 + THREEBITDELTAS[delta as usize]) as u32);
                }
                let delta = n & 7;
                if delta != DELTA3_SKIP {
                    p.setpixel((p.lastpixel as i32 + THREEBITDELTAS[delta as usize]) as u32);
                }
            }
            THUNDER_RAW => {
                /* raw data */
                p.setpixel(n as u32);
            }
            _ => {}
        }
    }
    tif.tif_rawcp = bp;
    tif.tif_rawcc = cc;
    if p.npixels != maxpixels {
        let op_end = ((maxpixels + 1) / 2).max(0) as usize;
        let end = op_end.min(p.row.len());
        if p.op < end {
            p.row[p.op..end].fill(0);
        }
        tiff_error_ext_r!(
            MODULE,
            "{} data at scanline {} ({} != {})",
            if p.npixels < maxpixels {
                "Not enough"
            } else {
                "Too much"
            },
            tif.tif_dir.td_row,
            p.npixels as u64,
            maxpixels as u64
        );
        return 0;
    }

    1
}

/// Translation of `ThunderDecodeRow()`.
fn thunder_decode_row(tif: &mut Tiff<'_>, buf: &mut [u8], mut occ: TmSize, _s: u16) -> i32 {
    const MODULE: &str = "ThunderDecodeRow";
    let mut row = 0usize;

    let scanline = tif.tif_dir.td_scanlinesize;
    if scanline == 0 || occ % scanline != 0 {
        tiff_error_ext_r!(MODULE, "Fractional scanlines cannot be read");
        return 0;
    }
    while occ > 0 {
        let width = tif.tif_dir.td_imagewidth as TmSize;
        let rowbuf = buf.get_mut(row..).unwrap_or(&mut []);
        if thunder_decode(tif, rowbuf, width) == 0 {
            return 0;
        }
        occ -= scanline;
        row += scanline as usize;
    }
    1
}

/// Translation of `TIFFInitThunderScan()`.
pub(crate) fn tiff_init_thunder_scan(tif: &mut Tiff<'_>, _scheme: i32) -> i32 {
    tif.tif_setupdecode = thunder_setup_decode;
    tif.tif_decoderow = thunder_decode_row;
    tif.tif_decodestrip = thunder_decode_row;
    1
}
