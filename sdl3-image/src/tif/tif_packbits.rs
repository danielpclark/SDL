// Rust translation of libtiff/tif_packbits.c from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1988-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! PackBits Compression Algorithm Support (the decoder; the encoder is
//! left out).

use super::tif_error::{tiff_error_ext_r, tiff_warning_ext_r};
use super::tiffiop::{Tiff, TmSize};

/// Translation of `PackBitsDecode()`.
fn pack_bits_decode(tif: &mut Tiff<'_>, op: &mut [u8], mut occ: TmSize, _s: u16) -> i32 {
    const MODULE: &str = "PackBitsDecode";

    let mut bp = tif.tif_rawcp;
    let mut cc = tif.tif_rawcc;
    let mut o = 0usize;
    let raw = |i: usize| tif.tif_rawdata.get(i).copied().unwrap_or(0);
    while cc > 0 && occ > 0 {
        let mut n = raw(bp) as i8 as i64;
        bp += 1;
        cc -= 1;
        if n < 0 {
            /* replicate next byte -n+1 times */
            if n == -128 {
                /* nop */
                continue;
            }
            n = -n + 1;
            if occ < n as TmSize {
                tiff_warning_ext_r!(
                    MODULE,
                    "Discarding {} bytes to avoid buffer overrun",
                    n as TmSize - occ
                );
                n = occ as i64;
            }
            if cc == 0 {
                tiff_warning_ext_r!(MODULE, "Terminating PackBitsDecode due to lack of data.");
                break;
            }
            occ -= n as TmSize;
            let b = raw(bp);
            bp += 1;
            cc -= 1;
            let end = (o + n as usize).min(op.len());
            if o < end {
                op[o..end].fill(b);
            }
            o += n as usize;
        } else {
            /* copy next n+1 bytes literally */
            if occ < (n + 1) as TmSize {
                tiff_warning_ext_r!(
                    MODULE,
                    "Discarding {} bytes to avoid buffer overrun",
                    n as TmSize - occ + 1
                );
                n = occ as i64 - 1;
            }
            if cc < (n + 1) as TmSize {
                tiff_warning_ext_r!(MODULE, "Terminating PackBitsDecode due to lack of data.");
                break;
            }
            n += 1;
            let src = tif.tif_rawdata.get(bp..).unwrap_or(&[]);
            let dst = op.get_mut(o..).unwrap_or(&mut []);
            let k = (n as usize).min(src.len()).min(dst.len());
            dst[..k].copy_from_slice(&src[..k]);
            o += n as usize;
            occ -= n as TmSize;
            bp += n as usize;
            cc -= n as TmSize;
        }
    }
    tif.tif_rawcp = bp;
    tif.tif_rawcc = cc;
    if occ > 0 {
        let end = (o + occ as usize).min(op.len());
        if o < end {
            op[o..end].fill(0);
        }
        tiff_error_ext_r!(
            MODULE,
            "Not enough data for scanline {}",
            tif.tif_dir.td_row
        );
        return 0;
    }
    1
}

/// Translation of `PackBitsGetMaxCompressionRatio()`.
fn pack_bits_get_max_compression_ratio(_tif: &Tiff<'_>) -> u64 {
    /* See README_for_libtiff_developpers.md for raw data used to estimate
     * the maximum compression rate. */

    64
}

/// Translation of `TIFFInitPackBits()`.
pub(crate) fn tiff_init_pack_bits(tif: &mut Tiff<'_>, _scheme: i32) -> i32 {
    tif.tif_decoderow = pack_bits_decode;
    tif.tif_decodestrip = pack_bits_decode;
    tif.tif_decodetile = pack_bits_decode;
    tif.tif_getmaxcompressionratio = pack_bits_get_max_compression_ratio;

    1
}
