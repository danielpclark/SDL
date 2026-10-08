// Rust translation of libtiff/tif_next.c from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1988-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! NeXT 2-bit Grey Scale Compression Algorithm Support

use super::tif_error::tiff_error_ext_r;
use super::tiffiop::{Tiff, TmSize};

const LITERALROW: TmSize = 0x00;
const LITERALSPAN: TmSize = 0x40;

/// Translation of `NeXTDecode()`.
fn next_decode(tif: &mut Tiff<'_>, buf: &mut [u8], mut occ: TmSize, _s: u16) -> i32 {
    const MODULE: &str = "NeXTDecode";

    /*
     * Each scanline is assumed to start off as all
     * white (we assume a PhotometricInterpretation
     * of ``min-is-black'').
     */
    let len = (occ.max(0) as usize).min(buf.len());
    buf[..len].fill(0xff);

    let mut bp = tif.tif_rawcp;
    let mut cc = tif.tif_rawcc;
    let scanline = tif.tif_dir.td_scanlinesize;
    if scanline == 0 || occ % scanline != 0 {
        tiff_error_ext_r!(MODULE, "Fractional scanlines cannot be read");
        return 0;
    }
    let raw = |i: usize| tif.tif_rawdata.get(i).copied().unwrap_or(0) as TmSize;
    let mut row = 0usize;
    let bad = 'rows: {
        while cc > 0 && occ > 0 {
            let mut n = raw(bp);
            bp += 1;
            cc -= 1;
            match n {
                LITERALROW => {
                    /*
                     * The entire scanline is given as literal values.
                     */
                    if cc < scanline {
                        break 'rows true;
                    }
                    copy(buf, row, &tif.tif_rawdata, bp, scanline as usize);
                    bp += scanline as usize;
                    cc -= scanline;
                }
                LITERALSPAN => {
                    /*
                     * The scanline has a literal span that begins at some
                     * offset.
                     */
                    if cc < 4 {
                        break 'rows true;
                    }
                    let off = (raw(bp) * 256) + raw(bp + 1);
                    n = (raw(bp + 2) * 256) + raw(bp + 3);
                    if cc < 4 + n || off + n > scanline {
                        break 'rows true;
                    }
                    copy(
                        buf,
                        row + off as usize,
                        &tif.tif_rawdata,
                        bp + 4,
                        n as usize,
                    );
                    bp += 4 + n as usize;
                    cc -= 4 + n;
                }
                _ => {
                    let mut npixels: u32 = 0;
                    let mut op_offset: TmSize = 0;
                    let mut imagewidth = tif.tif_dir.td_imagewidth;
                    if tif.is_tiled() {
                        imagewidth = tif.tif_dir.td_tilewidth;
                    }

                    /*
                     * The scanline is composed of a sequence of constant
                     * color ``runs''.  We shift into ``run mode'' and
                     * interpret bytes as codes of the form
                     * <color><npixels> until we've filled the scanline.
                     */
                    let mut op = row;
                    loop {
                        let grey = ((n >> 6) & 0x3) as u8;
                        n &= 0x3f;
                        /*
                         * Ensure the run does not exceed the scanline
                         * bounds, potentially resulting in a security
                         * issue.
                         */
                        while n > 0 && npixels < imagewidth && op_offset < scanline {
                            n -= 1;
                            /* SETPIXEL(op, grey) */
                            let p = npixels & 3;
                            npixels += 1;
                            if let Some(b) = buf.get_mut(op) {
                                match p {
                                    0 => *b = grey << 6,
                                    1 => *b |= grey << 4,
                                    2 => *b |= grey << 2,
                                    _ => *b |= grey,
                                }
                            }
                            if p == 3 {
                                op += 1;
                                op_offset += 1;
                            }
                        }
                        if npixels >= imagewidth {
                            break;
                        }
                        if op_offset >= scanline {
                            tiff_error_ext_r!(
                                MODULE,
                                "Invalid data for scanline {}",
                                tif.tif_dir.td_row
                            );
                            return 0;
                        }
                        if cc == 0 {
                            break 'rows true;
                        }
                        n = raw(bp);
                        bp += 1;
                        cc -= 1;
                    }
                }
            }
            occ -= scanline;
            row += scanline as usize;
        }
        false
    };
    if bad {
        tiff_error_ext_r!(
            MODULE,
            "Not enough data for scanline {}",
            tif.tif_dir.td_row
        );
        return 0;
    }
    tif.tif_rawcp = bp;
    tif.tif_rawcc = cc;
    1
}

/// `_TIFFmemcpy(buf + at, raw + from, n)`, as far as both go.
fn copy(buf: &mut [u8], at: usize, raw: &[u8], from: usize, n: usize) {
    let src = raw.get(from..).unwrap_or(&[]);
    let dst = buf.get_mut(at..).unwrap_or(&mut []);
    let n = n.min(src.len()).min(dst.len());
    dst[..n].copy_from_slice(&src[..n]);
}

/// Translation of `NeXTPreDecode()`.
fn next_pre_decode(tif: &mut Tiff<'_>, _s: u16) -> i32 {
    const MODULE: &str = "NeXTPreDecode";

    if tif.tif_dir.td_bitspersample != 2 {
        tiff_error_ext_r!(
            MODULE,
            "Unsupported BitsPerSample = {}",
            tif.tif_dir.td_bitspersample
        );
        return 0;
    }
    1
}

/// Translation of `TIFFInitNeXT()`.
pub(crate) fn tiff_init_next(tif: &mut Tiff<'_>, _scheme: i32) -> i32 {
    tif.tif_predecode = next_pre_decode;
    tif.tif_decoderow = next_decode;
    tif.tif_decodestrip = next_decode;
    tif.tif_decodetile = next_decode;
    1
}
