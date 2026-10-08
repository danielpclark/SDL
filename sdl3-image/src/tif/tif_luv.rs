// Rust translation of libtiff/tif_luv.c and libtiff/uvcode.h from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1997 Greg Ward Larson
// Copyright (c) 1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! TIFF Library.
//! LogLuv compression support for high dynamic range images.
//!
//! Contributed by Greg Larson.
//!
//! LogLuv image support uses the TIFF library to store 16 or 10-bit
//! log luminance values with 8 bits each of u and v or a 14-bit index.
//!
//! The codec can take as input and produce as output 32-bit IEEE float values
//! as well as 16-bit integer values.  A 16-bit luminance is interpreted
//! as a sign bit followed by a 15-bit integer that is converted
//! to and from a linear magnitude using the transformation:
//!
//! ```text
//!     L = 2^( (Le+.5)/256 - 64 )          # real from 15-bit
//!
//!     Le = floor( 256*(log2(L) + 64) )    # 15-bit from real
//! ```
//!
//! The actual conversion to world luminance units in candelas per sq. meter
//! requires an additional multiplier, which is stored in the TIFFTAG_STONITS.
//! This value is usually set such that a reasonable exposure comes from
//! clamping decoded luminances above 1 to 1 in the displayed image.
//!
//! The 16-bit values for u and v may be converted to real values by dividing
//! each by 32768.  (This allows for negative values, which aren't useful as
//! far as we know, but are left in case of future improvements in human
//! color vision.)
//!
//! Conversion from (u,v), which is actually the CIE (u',v') system for
//! you color scientists, is accomplished by the following transformation:
//!
//! ```text
//!     u = 4*x / (-2*x + 12*y + 3)
//!     v = 9*y / (-2*x + 12*y + 3)
//!
//!     x = 9*u / (6*u - 16*v + 12)
//!     y = 4*v / (6*u - 16*v + 12)
//! ```
//!
//! This process is greatly simplified by passing 32-bit IEEE floats
//! for each of three CIE XYZ coordinates.  The codec then takes care
//! of conversion to and from LogLuv, though the application is still
//! responsible for interpreting the TIFFTAG_STONITS calibration factor.
//!
//! By definition, a CIE XYZ vector of [1 1 1] corresponds to a neutral white
//! point of (x,y)=(1/3,1/3).  However, most color systems assume some other
//! white point, such as D65, and an absolute color conversion to XYZ then
//! to another color space with a different white point may introduce an
//! unwanted color cast to the image.  It is often desirable, therefore, to
//! perform a white point conversion that maps the input white to [1 1 1]
//! in XYZ, then record the original white point using the TIFFTAG_WHITEPOINT
//! tag value.  A decoder that demands absolute color calibration may use
//! this white point tag to get back the original colors, but usually it
//! will be ignored and the new white point will be used instead that
//! matches the output color space.
//!
//! Pixel information is compressed into one of two basic encodings, depending
//! on the setting of the compression tag, which is one of COMPRESSION_SGILOG
//! or COMPRESSION_SGILOG24.  For COMPRESSION_SGILOG, greyscale data is
//! stored as:
//!
//! ```text
//!      1       15
//!     |-+---------------|
//! ```
//!
//! COMPRESSION_SGILOG color data is stored as:
//!
//! ```text
//!      1       15           8        8
//!     |-+---------------|--------+--------|
//!      S       Le           ue       ve
//! ```
//!
//! For the 24-bit COMPRESSION_SGILOG24 color format, the data is stored as:
//!
//! ```text
//!          10           14
//!     |----------|--------------|
//!          Le'          Ce
//! ```
//!
//! There is no sign bit in the 24-bit case, and the (u,v) chromaticity is
//! encoded as an index for optimal color resolution.  The 10 log bits are
//! defined by the following conversions:
//!
//! ```text
//!     L = 2^((Le'+.5)/64 - 12)            # real from 10-bit
//!
//!     Le' = floor( 64*(log2(L) + 12) )    # 10-bit from real
//! ```
//!
//! The 10 bits of the smaller format may be converted into the 15 bits of
//! the larger format by multiplying by 4 and adding 13314.  Obviously,
//! a smaller range of magnitudes is covered (about 5 orders of magnitude
//! instead of 38), and the lack of a sign bit means that negative luminances
//! are not allowed.  (Well, they aren't allowed in the real world, either,
//! but they are useful for certain types of image processing.)
//!
//! The desired user format is controlled by the setting the internal
//! pseudo tag TIFFTAG_SGILOGDATAFMT to one of:
//!  SGILOGDATAFMT_FLOAT       = IEEE 32-bit float XYZ values
//!  SGILOGDATAFMT_16BIT       = 16-bit integer encodings of logL, u and v
//! Raw data i/o is also possible using:
//!  SGILOGDATAFMT_RAW         = 32-bit unsigned integer with encoded pixel
//! In addition, the following decoding is provided for ease of display:
//!  SGILOGDATAFMT_8BIT        = 8-bit default RGB gamma-corrected values
//!
//! For grayscale images, we provide the following data formats:
//!  SGILOGDATAFMT_FLOAT       = IEEE 32-bit float Y values
//!  SGILOGDATAFMT_16BIT       = 16-bit integer w/ encoded luminance
//!  SGILOGDATAFMT_8BIT        = 8-bit gray monitor values
//!
//! Note that the COMPRESSION_SGILOG applies a simple run-length encoding
//! scheme by separating the logL, u and v bytes for each row and applying
//! a PackBits type of compression.  Since the 24-bit encoding is not
//! adaptive, the 32-bit color format takes less space in many cases.
//!
//! Further control is provided over the conversion from higher-resolution
//! formats to final encoded values through the pseudo tag
//! TIFFTAG_SGILOGENCODE:
//!  SGILOGENCODE_NODITHER     = do not dither encoded values
//!  SGILOGENCODE_RANDITHER    = apply random dithering during encoding
//!
//! The default value of this tag is SGILOGENCODE_NODITHER for
//! COMPRESSION_SGILOG to maximize run-length encoding and
//! SGILOGENCODE_RANDITHER for COMPRESSION_SGILOG24 to turn
//! quantization errors into noise.
//!
//! The decoders (the encoders, and the conversions only they use, are left
//! out). The translation buffer is a vector of 16-bit (LogL) or 32-bit
//! (LogLuv) values; where the C decodes straight into the caller's buffer
//! (16-bit LogL and raw LogLuv data) the values are decoded into a
//! vector and then stored into the buffer in host byte order.

use std::f64::consts::LN_2;

use super::tif_aux::_tiff_multiply_ssize;
use super::tif_compress::_tiff_set_default_compression_state;
use super::tif_dir::{
    tiff_set_field, TIFFVGetMethod, TIFFVSetMethod, Va, VaList, Gv, FIELD_PSEUDO,
    TIFF_SETGET_INT,
};
use super::tif_dirinfo::_tiff_merge_fields;
use super::tif_error::tiff_error_ext_r;
use super::tif_strip::tiff_scanline_size;
use super::tif_tile::{tiff_tile_row_size, tiff_tile_size};
use super::tiff::*;
use super::tiffio::{field, TIFFField};
use super::tiffiop::{try_vec, TIFFPostMethod, TifData, Tiff, TmSize};

/// `void (*tfunc)(LogLuvState *, uint8_t *, tmsize_t)`: the conversion of
/// the translation buffer into the user's data format.
type TFunc = fn(&LogLuvState, &mut [u8], TmSize);

/// State block for each open TIFF
/// file using LogLuv compression/decompression.
pub(crate) struct LogLuvState {
    encoder_state: i32, /* 1 if encoder correctly initialized */
    user_datafmt: i32,  /* user data format */
    encode_meth: i32,   /* encoding method */
    pixel_size: i32,    /* bytes per pixel */

    /// translation buffer (of 16-bit LogL values)
    tbuf16: Vec<i16>,
    /// translation buffer (of 32-bit or 24-bit LogLuv values)
    tbuf32: Vec<u32>,
    tbuflen: TmSize, /* buffer length */
    tfunc: TFunc,

    vgetparent: TIFFVGetMethod, /* super-class method */
    vsetparent: TIFFVSetMethod, /* super-class method */
}

/// Translation of `DecoderState()`.
fn decoder_state(data: &mut TifData) -> Option<&mut LogLuvState> {
    match data {
        TifData::LogLuv(sp) => Some(sp),
        _ => None,
    }
}

const SGILOGDATAFMT_UNKNOWN: i32 = -1;

/// The byte at `bp` of the raw data (0 past its end, which the C's
/// counts keep it from reading).
fn byte(raw: &[u8], bp: usize) -> u8 {
    raw.get(bp).copied().unwrap_or(0)
}

/// Stores `values` into `op` in host byte order (what the C's casts of
/// the caller's buffer do).
fn store_ne<T: Copy, const N: usize>(op: &mut [u8], values: &[T], f: impl Fn(T) -> [u8; N]) {
    for (dst, &v) in op.chunks_exact_mut(N).zip(values) {
        dst.copy_from_slice(&f(v));
    }
}

/// Decode a string of 16-bit gray pixels.
/// Translation of `LogL16Decode()`.
fn log_l16_decode(tif: &mut Tiff<'_>, op: &mut [u8], occ: TmSize, _s: u16) -> i32 {
    const MODULE: &str = "LogL16Decode";
    let Some(sp) = decoder_state(&mut tif.tif_data) else {
        return 0;
    };
    if sp.pixel_size <= 0 {
        return 0;
    }

    let npixels = occ / sp.pixel_size as TmSize;

    let mut own: Vec<i16>;
    let tp: &mut [i16] = if sp.user_datafmt == SGILOGDATAFMT_16BIT as i32 {
        own = match try_vec(npixels.max(0) as usize) {
            Some(v) => v,
            None => return 0,
        };
        &mut own
    } else {
        if sp.tbuflen < npixels {
            tiff_error_ext_r!(MODULE, "Translation buffer too short");
            return 0;
        }
        let n = (npixels.max(0) as usize).min(sp.tbuf16.len());
        &mut sp.tbuf16[..n]
    };
    tp.fill(0);
    let npixels = tp.len() as TmSize;

    let raw = &tif.tif_rawdata;
    let mut bp = tif.tif_rawcp;
    let mut cc = tif.tif_rawcc;
    let mut ok = true;
    /* get each byte string */
    for shft in [8, 0] {
        let mut i: TmSize = 0;
        while i < npixels && cc > 0 {
            if byte(raw, bp) >= 128 {
                /* run */
                if cc < 2 {
                    break;
                }
                let mut rc = byte(raw, bp) as i32 + (2 - 128);
                bp += 1;
                let b = ((byte(raw, bp) as i32) << shft) as i16;
                bp += 1;
                cc -= 2;
                while rc != 0 && i < npixels {
                    rc -= 1;
                    tp[i as usize] |= b;
                    i += 1;
                }
            } else {
                /* non-run */
                let mut rc = byte(raw, bp) as i32; /* nul is noop */
                bp += 1;
                loop {
                    cc -= 1;
                    if cc == 0 || rc == 0 {
                        break;
                    }
                    rc -= 1;
                    if i >= npixels {
                        break;
                    }
                    tp[i as usize] |= ((byte(raw, bp) as i32) << shft) as i16;
                    i += 1;
                    bp += 1;
                }
            }
        }
        if i != npixels {
            tiff_error_ext_r!(
                MODULE,
                "Not enough data at row {} (short {} pixels)",
                tif.tif_dir.td_row,
                npixels - i
            );
            ok = false;
            break;
        }
    }
    if sp.user_datafmt == SGILOGDATAFMT_16BIT as i32 {
        store_ne(op, tp, i16::to_ne_bytes);
    }
    tif.tif_rawcp = bp;
    tif.tif_rawcc = cc;
    if !ok {
        return 0;
    }
    (sp.tfunc)(sp, op, npixels);
    1
}

/// Decode a string of 24-bit pixels.
/// Translation of `LogLuvDecode24()`.
fn log_luv_decode24(tif: &mut Tiff<'_>, op: &mut [u8], occ: TmSize, _s: u16) -> i32 {
    const MODULE: &str = "LogLuvDecode24";
    let Some(sp) = decoder_state(&mut tif.tif_data) else {
        return 0;
    };
    if sp.pixel_size <= 0 {
        return 0;
    }

    let npixels = occ / sp.pixel_size as TmSize;

    let mut own: Vec<u32>;
    let tp: &mut [u32] = if sp.user_datafmt == SGILOGDATAFMT_RAW as i32 {
        own = match try_vec(npixels.max(0) as usize) {
            Some(v) => v,
            None => return 0,
        };
        &mut own
    } else {
        if sp.tbuflen < npixels {
            tiff_error_ext_r!(MODULE, "Translation buffer too short");
            return 0;
        }
        let n = (npixels.max(0) as usize).min(sp.tbuf32.len());
        &mut sp.tbuf32[..n]
    };
    let npixels = tp.len() as TmSize;
    /* copy to array of uint32_t */
    let raw = &tif.tif_rawdata;
    let mut bp = tif.tif_rawcp;
    let mut cc = tif.tif_rawcc;
    let mut i: TmSize = 0;
    while i < npixels && cc >= 3 {
        tp[i as usize] = (byte(raw, bp) as u32) << 16
            | (byte(raw, bp + 1) as u32) << 8
            | byte(raw, bp + 2) as u32;
        bp += 3;
        cc -= 3;
        i += 1;
    }
    if sp.user_datafmt == SGILOGDATAFMT_RAW as i32 {
        store_ne(op, &tp[..i as usize], u32::to_ne_bytes);
    }
    tif.tif_rawcp = bp;
    tif.tif_rawcc = cc;
    if i != npixels {
        tiff_error_ext_r!(
            MODULE,
            "Not enough data at row {} (short {} pixels)",
            tif.tif_dir.td_row,
            npixels - i
        );
        return 0;
    }
    (sp.tfunc)(sp, op, npixels);
    1
}

/// Decode a string of 32-bit pixels.
/// Translation of `LogLuvDecode32()`.
fn log_luv_decode32(tif: &mut Tiff<'_>, op: &mut [u8], occ: TmSize, _s: u16) -> i32 {
    const MODULE: &str = "LogLuvDecode32";
    let Some(sp) = decoder_state(&mut tif.tif_data) else {
        return 0;
    };
    if sp.pixel_size <= 0 {
        return 0;
    }

    let npixels = occ / sp.pixel_size as TmSize;

    let mut own: Vec<u32>;
    let tp: &mut [u32] = if sp.user_datafmt == SGILOGDATAFMT_RAW as i32 {
        own = match try_vec(npixels.max(0) as usize) {
            Some(v) => v,
            None => return 0,
        };
        &mut own
    } else {
        if sp.tbuflen < npixels {
            tiff_error_ext_r!(MODULE, "Translation buffer too short");
            return 0;
        }
        let n = (npixels.max(0) as usize).min(sp.tbuf32.len());
        &mut sp.tbuf32[..n]
    };
    tp.fill(0);
    let npixels = tp.len() as TmSize;

    let raw = &tif.tif_rawdata;
    let mut bp = tif.tif_rawcp;
    let mut cc = tif.tif_rawcc;
    let mut ok = true;
    /* get each byte string */
    for shft in [24, 16, 8, 0] {
        let mut i: TmSize = 0;
        while i < npixels && cc > 0 {
            if byte(raw, bp) >= 128 {
                /* run */
                if cc < 2 {
                    break;
                }
                let mut rc = byte(raw, bp) as i32 + (2 - 128);
                bp += 1;
                let b = (byte(raw, bp) as u32) << shft;
                bp += 1;
                cc -= 2;
                while rc != 0 && i < npixels {
                    rc -= 1;
                    tp[i as usize] |= b;
                    i += 1;
                }
            } else {
                /* non-run */
                let mut rc = byte(raw, bp) as i32; /* nul is noop */
                bp += 1;
                loop {
                    cc -= 1;
                    if cc == 0 || rc == 0 {
                        break;
                    }
                    rc -= 1;
                    if i >= npixels {
                        break;
                    }
                    tp[i as usize] |= (byte(raw, bp) as u32) << shft;
                    i += 1;
                    bp += 1;
                }
            }
        }
        if i != npixels {
            tiff_error_ext_r!(
                MODULE,
                "Not enough data at row {} (short {} pixels)",
                tif.tif_dir.td_row,
                npixels - i
            );
            ok = false;
            break;
        }
    }
    if sp.user_datafmt == SGILOGDATAFMT_RAW as i32 {
        store_ne(op, tp, u32::to_ne_bytes);
    }
    tif.tif_rawcp = bp;
    tif.tif_rawcc = cc;
    if !ok {
        return 0;
    }
    (sp.tfunc)(sp, op, npixels);
    1
}

/// Decode a strip of pixels.  We break it into rows to
/// maintain synchrony with the encode algorithm, which
/// is row by row.
/// Translation of `LogLuvDecodeStrip()`.
fn log_luv_decode_strip(tif: &mut Tiff<'_>, bp: &mut [u8], mut cc: TmSize, s: u16) -> i32 {
    let rowlen = tiff_scanline_size(tif);

    if rowlen == 0 {
        return 0;
    }

    // (the C asserts cc % rowlen == 0, and with a remainder decodes a
    // whole row past the buffer's end: the loop stops at the remainder)
    let mut off = 0usize;
    while cc > 0 {
        let decoderow = tif.tif_decoderow;
        let row = bp.get_mut(off..).unwrap_or(&mut []);
        if decoderow(tif, row, rowlen, s) == 0 {
            break;
        }
        off += rowlen as usize;
        cc -= rowlen;
    }
    (cc == 0) as i32
}

/// Decode a tile of pixels.  We break it into rows to
/// maintain synchrony with the encode algorithm, which
/// is row by row.
/// Translation of `LogLuvDecodeTile()`.
fn log_luv_decode_tile(tif: &mut Tiff<'_>, bp: &mut [u8], mut cc: TmSize, s: u16) -> i32 {
    let rowlen = tiff_tile_row_size(tif);

    if rowlen == 0 {
        return 0;
    }

    // (as in log_luv_decode_strip())
    let mut off = 0usize;
    while cc > 0 {
        let decoderow = tif.tif_decoderow;
        let row = bp.get_mut(off..).unwrap_or(&mut []);
        if decoderow(tif, row, rowlen, s) == 0 {
            break;
        }
        off += rowlen as usize;
        cc -= rowlen;
    }
    (cc == 0) as i32
}

/*
 * Encoding/Decoding Tables
 */

/* Version 1.0 generated April 7, 1997 by Greg Ward Larson, SGI */
const UV_SQSIZ: f32 = 0.003500;
const UV_NDIVS: i32 = 16289;
const UV_VSTART: f32 = 0.016940;
const UV_NVS: u32 = 163;

/// A row of `uv_row`.
struct UvRow {
    ustart: f32,
    nus: i16,
    ncum: i16,
}

/// A `uv_row` entry.
const fn r(ustart: f32, nus: i16, ncum: i16) -> UvRow {
    UvRow { ustart, nus, ncum }
}

const U_NEU: f64 = 0.210526316;
const V_NEU: f64 = 0.473684211;
const UVSCALE: f64 = 410.;

/// Translation of `LogL16toY()`: compute luminance from 16-bit LogL
fn log_l16_to_y(p16: i32) -> f64 {
    let le = p16 & 0x7fff;

    if le == 0 {
        return 0.;
    }
    let y = (LN_2 / 256. * (le as f64 + 0.5) - LN_2 * 64.).exp();
    if (p16 & 0x8000) == 0 {
        y
    } else {
        -y
    }
}

/// `(uint8_t)((v <= 0.) ? 0 : (v >= 1.) ? 255 : (int)(256. * sqrt(v)))`
fn gamma8(v: f64) -> u8 {
    if v <= 0. {
        0
    } else if v >= 1. {
        255
    } else {
        (256. * v.sqrt()) as i32 as u8
    }
}

/// Translation of `L16toY()`.
fn l16_to_y(sp: &LogLuvState, op: &mut [u8], n: TmSize) {
    let n = n.max(0) as usize;
    for (yp, &l16) in op.chunks_exact_mut(4).zip(&sp.tbuf16).take(n) {
        yp.copy_from_slice(&(log_l16_to_y(l16 as i32) as f32).to_ne_bytes());
    }
}

/// Translation of `L16toGry()`.
fn l16_to_gry(sp: &LogLuvState, op: &mut [u8], n: TmSize) {
    let n = n.max(0) as usize;
    for (gp, &l16) in op.iter_mut().zip(&sp.tbuf16).take(n) {
        let y = log_l16_to_y(l16 as i32);
        *gp = gamma8(y);
    }
}

/// Translation of `XYZtoRGB24()`.
fn xyz_to_rgb24(xyz: &[f32; 3], rgb: &mut [u8]) {
    /* assume CCIR-709 primaries */
    let r = 2.690 * xyz[0] as f64 + -1.276 * xyz[1] as f64 + -0.414 * xyz[2] as f64;
    let g = -1.022 * xyz[0] as f64 + 1.978 * xyz[1] as f64 + 0.044 * xyz[2] as f64;
    let b = 0.061 * xyz[0] as f64 + -0.224 * xyz[1] as f64 + 1.163 * xyz[2] as f64;
    /* assume 2.0 gamma for speed */
    /* could use integer sqrt approx., but this is probably faster */
    rgb[0] = gamma8(r);
    rgb[1] = gamma8(g);
    rgb[2] = gamma8(b);
}

/// Translation of `LogL10toY()`: compute luminance from 10-bit LogL
fn log_l10_to_y(p10: i32) -> f64 {
    if p10 == 0 {
        return 0.;
    }
    (LN_2 / 64. * (p10 as f64 + 0.5) - LN_2 * 12.).exp()
}

/// Translation of `uv_decode()`: decode (u',v') index
fn uv_decode(up: &mut f64, vp: &mut f64, c: i32) -> i32 {
    if !(0..UV_NDIVS).contains(&c) {
        return -1;
    }
    let mut lower: u32 = 0; /* binary search */
    let mut upper: u32 = UV_NVS;
    while upper - lower > 1 {
        let vi = (lower + upper) >> 1;
        let ui = c - UV_ROW[vi as usize].ncum as i32;
        if ui > 0 {
            lower = vi;
        } else if ui < 0 {
            upper = vi;
        } else {
            lower = vi;
            break;
        }
    }
    let vi = lower;
    let ui = c - UV_ROW[vi as usize].ncum as i32;
    *up = UV_ROW[vi as usize].ustart as f64 + (ui as f64 + 0.5) * UV_SQSIZ as f64;
    *vp = UV_VSTART as f64 + (vi as f64 + 0.5) * UV_SQSIZ as f64;
    0
}

/// Translation of `LogLuv24toXYZ()`.
fn log_luv24_to_xyz(p: u32, xyz: &mut [f32; 3]) {
    /* decode luminance */
    let l = log_l10_to_y((p >> 14 & 0x3ff) as i32);
    if l <= 0. {
        *xyz = [0.; 3];
        return;
    }
    /* decode color */
    let ce = (p & 0x3fff) as i32;
    let (mut u, mut v) = (0., 0.);
    if uv_decode(&mut u, &mut v, ce) < 0 {
        u = U_NEU;
        v = V_NEU;
    }
    let s = 1. / (6. * u - 16. * v + 12.);
    let x = 9. * u * s;
    let y = 4. * v * s;
    /* convert to XYZ */
    xyz[0] = (x / y * l) as f32;
    xyz[1] = l as f32;
    xyz[2] = ((1. - x - y) / y * l) as f32;
}

/// Translation of `Luv24toXYZ()`.
fn luv24_to_xyz(sp: &LogLuvState, op: &mut [u8], n: TmSize) {
    let n = n.max(0) as usize;
    for (out, &luv) in op.chunks_exact_mut(12).zip(&sp.tbuf32).take(n) {
        let mut xyz = [0f32; 3];
        log_luv24_to_xyz(luv, &mut xyz);
        store_ne(out, &xyz, f32::to_ne_bytes);
    }
}

/// Translation of `Luv24toLuv48()`.
fn luv24_to_luv48(sp: &LogLuvState, op: &mut [u8], n: TmSize) {
    let n = n.max(0) as usize;
    for (out, &luv) in op.chunks_exact_mut(6).zip(&sp.tbuf32).take(n) {
        let (mut u, mut v) = (0., 0.);

        let l = ((luv >> 12 & 0xffd) + 13314) as i16;
        if uv_decode(&mut u, &mut v, (luv & 0x3fff) as i32) < 0 {
            u = U_NEU;
            v = V_NEU;
        }
        let luv3 = [l, (u * (1 << 15) as f64) as i16, (v * (1 << 15) as f64) as i16];
        store_ne(out, &luv3, i16::to_ne_bytes);
    }
}

/// Translation of `Luv24toRGB()`.
fn luv24_to_rgb(sp: &LogLuvState, op: &mut [u8], n: TmSize) {
    let n = n.max(0) as usize;
    for (rgb, &luv) in op.chunks_exact_mut(3).zip(&sp.tbuf32).take(n) {
        let mut xyz = [0f32; 3];

        log_luv24_to_xyz(luv, &mut xyz);
        xyz_to_rgb24(&xyz, rgb);
    }
}

/// Translation of `LogLuv32toXYZ()`.
fn log_luv32_to_xyz(p: u32, xyz: &mut [f32; 3]) {
    /* decode luminance */
    let l = log_l16_to_y((p as i32) >> 16);
    if l <= 0. {
        *xyz = [0.; 3];
        return;
    }
    /* decode color */
    let u = 1. / UVSCALE * ((p >> 8 & 0xff) as f64 + 0.5);
    let v = 1. / UVSCALE * ((p & 0xff) as f64 + 0.5);
    let s = 1. / (6. * u - 16. * v + 12.);
    let x = 9. * u * s;
    let y = 4. * v * s;
    /* convert to XYZ */
    xyz[0] = (x / y * l) as f32;
    xyz[1] = l as f32;
    xyz[2] = ((1. - x - y) / y * l) as f32;
}

/// Translation of `Luv32toXYZ()`.
fn luv32_to_xyz(sp: &LogLuvState, op: &mut [u8], n: TmSize) {
    let n = n.max(0) as usize;
    for (out, &luv) in op.chunks_exact_mut(12).zip(&sp.tbuf32).take(n) {
        let mut xyz = [0f32; 3];
        log_luv32_to_xyz(luv, &mut xyz);
        store_ne(out, &xyz, f32::to_ne_bytes);
    }
}

/// Translation of `Luv32toLuv48()`.
fn luv32_to_luv48(sp: &LogLuvState, op: &mut [u8], n: TmSize) {
    let n = n.max(0) as usize;
    for (out, &luv) in op.chunks_exact_mut(6).zip(&sp.tbuf32).take(n) {
        let l = (luv >> 16) as i16;
        let u = 1. / UVSCALE * ((luv >> 8 & 0xff) as f64 + 0.5);
        let v = 1. / UVSCALE * ((luv & 0xff) as f64 + 0.5);
        let luv3 = [l, (u * (1 << 15) as f64) as i16, (v * (1 << 15) as f64) as i16];
        store_ne(out, &luv3, i16::to_ne_bytes);
    }
}

/// Translation of `Luv32toRGB()`.
fn luv32_to_rgb(sp: &LogLuvState, op: &mut [u8], n: TmSize) {
    let n = n.max(0) as usize;
    for (rgb, &luv) in op.chunks_exact_mut(3).zip(&sp.tbuf32).take(n) {
        let mut xyz = [0f32; 3];

        log_luv32_to_xyz(luv, &mut xyz);
        xyz_to_rgb24(&xyz, rgb);
    }
}

/// Translation of `_logLuvNop()`.
fn _log_luv_nop(_sp: &LogLuvState, _op: &mut [u8], _n: TmSize) {}

/// Translation of `LogL16GuessDataFmt()`.
fn log_l16_guess_data_fmt(tif: &Tiff<'_>) -> i32 {
    let td = &tif.tif_dir;
    let pack = |s: u32, b: u32, f: u32| (b << 6) | (s << 3) | f;
    let v = pack(
        td.td_samplesperpixel as u32,
        td.td_bitspersample as u32,
        td.td_sampleformat as u32,
    );
    if v == pack(1, 32, SAMPLEFORMAT_IEEEFP as u32) {
        return SGILOGDATAFMT_FLOAT as i32;
    }
    if v == pack(1, 16, SAMPLEFORMAT_VOID as u32)
        || v == pack(1, 16, SAMPLEFORMAT_INT as u32)
        || v == pack(1, 16, SAMPLEFORMAT_UINT as u32)
    {
        return SGILOGDATAFMT_16BIT as i32;
    }
    if v == pack(1, 8, SAMPLEFORMAT_VOID as u32) || v == pack(1, 8, SAMPLEFORMAT_UINT as u32) {
        return SGILOGDATAFMT_8BIT as i32;
    }
    SGILOGDATAFMT_UNKNOWN
}

/// Translation of `multiply_ms()`.
fn multiply_ms(m1: TmSize, m2: TmSize) -> TmSize {
    _tiff_multiply_ssize(None, m1, m2, None)
}

/// The translation buffer's length (in pixels) for the strips or tiles.
fn tbuflen(tif: &Tiff<'_>) -> TmSize {
    let td = &tif.tif_dir;
    if tif.is_tiled() {
        multiply_ms(td.td_tilewidth as TmSize, td.td_tilelength as TmSize)
    } else if td.td_rowsperstrip < td.td_imagelength {
        multiply_ms(td.td_imagewidth as TmSize, td.td_rowsperstrip as TmSize)
    } else {
        multiply_ms(td.td_imagewidth as TmSize, td.td_imagelength as TmSize)
    }
}

/// Translation of `LogL16InitState()`.
fn log_l16_init_state(tif: &mut Tiff<'_>) -> i32 {
    const MODULE: &str = "LogL16InitState";

    if tif.tif_dir.td_samplesperpixel != 1 {
        tiff_error_ext_r!(
            MODULE,
            "Sorry, can not handle LogL image with {}={}",
            "Samples/pixel",
            tif.tif_dir.td_samplesperpixel
        );
        return 0;
    }

    let guess = log_l16_guess_data_fmt(tif);
    let len = tbuflen(tif);
    let Some(sp) = decoder_state(&mut tif.tif_data) else {
        return 0;
    };
    /* for some reason, we can't do this in TIFFInitLogL16 */
    if sp.user_datafmt == SGILOGDATAFMT_UNKNOWN {
        sp.user_datafmt = guess;
    }
    match sp.user_datafmt {
        x if x == SGILOGDATAFMT_FLOAT as i32 => sp.pixel_size = 4,
        x if x == SGILOGDATAFMT_16BIT as i32 => sp.pixel_size = 2,
        x if x == SGILOGDATAFMT_8BIT as i32 => sp.pixel_size = 1,
        _ => {
            tiff_error_ext_r!(MODULE, "No support for converting user data format to LogL");
            return 0;
        }
    }
    sp.tbuflen = len;
    let buf = if multiply_ms(sp.tbuflen, 2) == 0 {
        None
    } else {
        try_vec::<i16>(sp.tbuflen as usize)
    };
    match buf {
        Some(b) => sp.tbuf16 = b,
        None => {
            tiff_error_ext_r!(MODULE, "No space for SGILog translation buffer");
            return 0;
        }
    }
    1
}

/// Translation of `LogLuvGuessDataFmt()`.
fn log_luv_guess_data_fmt(tif: &Tiff<'_>) -> i32 {
    let td = &tif.tif_dir;

    /*
     * If the user didn't tell us their datafmt,
     * take our best guess from the bitspersample.
     */
    let pack = |a: u32, b: u32| (a << 3) | b;
    let v = pack(td.td_bitspersample as u32, td.td_sampleformat as u32);
    let mut guess = if v == pack(32, SAMPLEFORMAT_IEEEFP as u32) {
        SGILOGDATAFMT_FLOAT as i32
    } else if v == pack(32, SAMPLEFORMAT_VOID as u32)
        || v == pack(32, SAMPLEFORMAT_UINT as u32)
        || v == pack(32, SAMPLEFORMAT_INT as u32)
    {
        SGILOGDATAFMT_RAW as i32
    } else if v == pack(16, SAMPLEFORMAT_VOID as u32)
        || v == pack(16, SAMPLEFORMAT_INT as u32)
        || v == pack(16, SAMPLEFORMAT_UINT as u32)
    {
        SGILOGDATAFMT_16BIT as i32
    } else if v == pack(8, SAMPLEFORMAT_VOID as u32) || v == pack(8, SAMPLEFORMAT_UINT as u32) {
        SGILOGDATAFMT_8BIT as i32
    } else {
        SGILOGDATAFMT_UNKNOWN
    };
    /*
     * Double-check samples per pixel.
     */
    match td.td_samplesperpixel {
        1 => {
            if guess != SGILOGDATAFMT_RAW as i32 {
                guess = SGILOGDATAFMT_UNKNOWN;
            }
        }
        3 => {
            if guess == SGILOGDATAFMT_RAW as i32 {
                guess = SGILOGDATAFMT_UNKNOWN;
            }
        }
        _ => guess = SGILOGDATAFMT_UNKNOWN,
    }
    guess
}

/// Translation of `LogLuvInitState()`.
fn log_luv_init_state(tif: &mut Tiff<'_>) -> i32 {
    const MODULE: &str = "LogLuvInitState";

    /* for some reason, we can't do this in TIFFInitLogLuv */
    if tif.tif_dir.td_planarconfig != PLANARCONFIG_CONTIG {
        tiff_error_ext_r!(MODULE, "SGILog compression cannot handle non-contiguous data");
        return 0;
    }
    let guess = log_luv_guess_data_fmt(tif);
    let len = tbuflen(tif);
    let Some(sp) = decoder_state(&mut tif.tif_data) else {
        return 0;
    };
    if sp.user_datafmt == SGILOGDATAFMT_UNKNOWN {
        sp.user_datafmt = guess;
    }
    match sp.user_datafmt {
        x if x == SGILOGDATAFMT_FLOAT as i32 => sp.pixel_size = 3 * 4,
        x if x == SGILOGDATAFMT_16BIT as i32 => sp.pixel_size = 3 * 2,
        x if x == SGILOGDATAFMT_RAW as i32 => sp.pixel_size = 4,
        x if x == SGILOGDATAFMT_8BIT as i32 => sp.pixel_size = 3,
        _ => {
            tiff_error_ext_r!(
                MODULE,
                "No support for converting user data format to LogLuv"
            );
            return 0;
        }
    }
    sp.tbuflen = len;
    let buf = if multiply_ms(sp.tbuflen, 4) == 0 {
        None
    } else {
        try_vec::<u32>(sp.tbuflen as usize)
    };
    match buf {
        Some(b) => sp.tbuf32 = b,
        None => {
            tiff_error_ext_r!(MODULE, "No space for SGILog translation buffer");
            return 0;
        }
    }
    1
}

/// Translation of `LogLuvFixupTags()`.
fn log_luv_fixup_tags(_tif: &mut Tiff<'_>) -> i32 {
    1
}

/// Translation of `LogLuvSetupDecode()`.
fn log_luv_setup_decode(tif: &mut Tiff<'_>) -> i32 {
    const MODULE: &str = "LogLuvSetupDecode";

    tif.tif_postdecode = TIFFPostMethod::NoPostDecode;
    match tif.tif_dir.td_photometric {
        PHOTOMETRIC_LOGLUV => {
            if log_luv_init_state(tif) != 0 {
                let sgilog24 = tif.tif_dir.td_compression == COMPRESSION_SGILOG24;
                tif.tif_decoderow = if sgilog24 {
                    log_luv_decode24
                } else {
                    log_luv_decode32
                };
                let Some(sp) = decoder_state(&mut tif.tif_data) else {
                    return 0;
                };
                let fmt = sp.user_datafmt;
                let tfunc: Option<TFunc> = if sgilog24 {
                    match fmt {
                        x if x == SGILOGDATAFMT_FLOAT as i32 => Some(luv24_to_xyz),
                        x if x == SGILOGDATAFMT_16BIT as i32 => Some(luv24_to_luv48),
                        x if x == SGILOGDATAFMT_8BIT as i32 => Some(luv24_to_rgb),
                        _ => None,
                    }
                } else {
                    match fmt {
                        x if x == SGILOGDATAFMT_FLOAT as i32 => Some(luv32_to_xyz),
                        x if x == SGILOGDATAFMT_16BIT as i32 => Some(luv32_to_luv48),
                        x if x == SGILOGDATAFMT_8BIT as i32 => Some(luv32_to_rgb),
                        _ => None,
                    }
                };
                if let Some(f) = tfunc {
                    sp.tfunc = f;
                }
                return 1;
            }
        }
        PHOTOMETRIC_LOGL => {
            if log_l16_init_state(tif) != 0 {
                tif.tif_decoderow = log_l16_decode;
                let Some(sp) = decoder_state(&mut tif.tif_data) else {
                    return 0;
                };
                match sp.user_datafmt {
                    x if x == SGILOGDATAFMT_FLOAT as i32 => sp.tfunc = l16_to_y,
                    x if x == SGILOGDATAFMT_8BIT as i32 => sp.tfunc = l16_to_gry,
                    _ => {}
                }
                return 1;
            }
        }
        _ => {
            tiff_error_ext_r!(
                MODULE,
                "Inappropriate photometric interpretation {} for SGILog compression; {}",
                tif.tif_dir.td_photometric,
                "must be either LogLUV or LogL"
            );
        }
    }
    0
}

/// Translation of `LogLuvClose()`.
fn log_luv_close(tif: &mut Tiff<'_>) {
    let Some(sp) = decoder_state(&mut tif.tif_data) else {
        return;
    };
    /*
     * For consistency, we always want to write out the same
     * bitspersample and sampleformat for our TIFF file,
     * regardless of the data format being used by the application.
     * Since this routine is called after tags have been set but
     * before they have been recorded in the file, we reset them here.
     * Note: this is really a nasty approach. See PixarLogClose
     */
    if sp.encoder_state != 0 {
        /* See PixarLogClose. Might avoid issues with tags whose size depends
         * on those below, but not completely sure this is enough. */
        let td = &mut tif.tif_dir;
        td.td_samplesperpixel = if td.td_photometric == PHOTOMETRIC_LOGL {
            1
        } else {
            3
        };
        td.td_bitspersample = 16;
        td.td_sampleformat = SAMPLEFORMAT_INT;
    }
}

/// Translation of `LogLuvCleanup()`.
fn log_luv_cleanup(tif: &mut Tiff<'_>) {
    if let Some(sp) = decoder_state(&mut tif.tif_data) {
        let (vget, vset) = (sp.vgetparent, sp.vsetparent);
        tif.tif_tagmethods.vgetfield = vget;
        tif.tif_tagmethods.vsetfield = vset;
    }

    tif.tif_data = TifData::None;

    _tiff_set_default_compression_state(tif);
}

/// Translation of `LogLuvVSetField()`.
fn log_luv_vset_field(tif: &mut Tiff<'_>, tag: u32, ap: &mut VaList<'_>) -> i32 {
    const MODULE: &str = "LogLuvVSetField";
    let Some(sp) = decoder_state(&mut tif.tif_data) else {
        return 0;
    };

    match tag {
        TIFFTAG_SGILOGDATAFMT => {
            sp.user_datafmt = ap.int() as i32;
            let user_datafmt = sp.user_datafmt;
            /*
             * Tweak the TIFF header so that the rest of libtiff knows what
             * size of data will be passed between app and library, and
             * assume that the app knows what it is doing and is not
             * confused by these header manipulations...
             */
            let (bps, fmt) = match user_datafmt {
                x if x == SGILOGDATAFMT_FLOAT as i32 => (32, SAMPLEFORMAT_IEEEFP),
                x if x == SGILOGDATAFMT_16BIT as i32 => (16, SAMPLEFORMAT_INT),
                x if x == SGILOGDATAFMT_RAW as i32 => {
                    tiff_set_field(tif, TIFFTAG_SAMPLESPERPIXEL, &[Va::Int(1)]);
                    (32, SAMPLEFORMAT_UINT)
                }
                x if x == SGILOGDATAFMT_8BIT as i32 => (8, SAMPLEFORMAT_UINT),
                _ => {
                    tiff_error_ext_r!(
                        &tif.tif_name,
                        "Unknown data format {} for LogLuv compression",
                        user_datafmt
                    );
                    return 0;
                }
            };
            tiff_set_field(tif, TIFFTAG_BITSPERSAMPLE, &[Va::Int(bps)]);
            tiff_set_field(tif, TIFFTAG_SAMPLEFORMAT, &[Va::Int(fmt as i64)]);
            /*
             * Must recalculate sizes should bits/sample change.
             */
            tif.tif_dir.td_tilesize = if tif.is_tiled() {
                tiff_tile_size(tif)
            } else {
                -1
            };
            tif.tif_dir.td_scanlinesize = tiff_scanline_size(tif);
            1
        }
        TIFFTAG_SGILOGENCODE => {
            sp.encode_meth = ap.int() as i32;
            if sp.encode_meth != SGILOGENCODE_NODITHER as i32
                && sp.encode_meth != SGILOGENCODE_RANDITHER as i32
            {
                tiff_error_ext_r!(
                    MODULE,
                    "Unknown encoding {} for LogLuv compression",
                    sp.encode_meth
                );
                return 0;
            }
            1
        }
        _ => {
            let parent = sp.vsetparent;
            parent(tif, tag, ap)
        }
    }
}

/// Translation of `LogLuvVGetField()`.
fn log_luv_vget_field(tif: &mut Tiff<'_>, tag: u32, ap: &mut Vec<Gv>) -> i32 {
    let Some(sp) = decoder_state(&mut tif.tif_data) else {
        return 0;
    };

    match tag {
        TIFFTAG_SGILOGDATAFMT => {
            ap.push(Gv::I32(sp.user_datafmt));
            1
        }
        _ => {
            let parent = sp.vgetparent;
            parent(tif, tag, ap)
        }
    }
}

static LOG_LUV_FIELDS: [TIFFField; 2] = [
    field(
        TIFFTAG_SGILOGDATAFMT,
        0,
        0,
        TIFF_SHORT,
        0,
        TIFF_SETGET_INT,
        FIELD_PSEUDO,
        1,
        0,
        "SGILogDataFmt",
    ),
    field(
        TIFFTAG_SGILOGENCODE,
        0,
        0,
        TIFF_SHORT,
        0,
        TIFF_SETGET_INT,
        FIELD_PSEUDO,
        1,
        0,
        "SGILogEncode",
    ),
];

/// Translation of `TIFFInitSGILog()`.
pub(crate) fn tiff_init_sg_log(tif: &mut Tiff<'_>, scheme: i32) -> i32 {
    const MODULE: &str = "TIFFInitSGILog";

    /*
     * Merge codec-specific tag information.
     */
    if _tiff_merge_fields(tif, &LOG_LUV_FIELDS) == 0 {
        tiff_error_ext_r!(MODULE, "Merging SGILog codec-specific tags failed");
        return 0;
    }

    /*
     * Allocate state block so tag methods have storage to record values.
     */
    let sp = LogLuvState {
        encoder_state: 0,
        user_datafmt: SGILOGDATAFMT_UNKNOWN,
        encode_meth: if scheme == COMPRESSION_SGILOG24 as i32 {
            SGILOGENCODE_RANDITHER as i32
        } else {
            SGILOGENCODE_NODITHER as i32
        },
        pixel_size: 0,
        tbuf16: Vec::new(),
        tbuf32: Vec::new(),
        tbuflen: 0,
        tfunc: _log_luv_nop,
        /*
         * Override parent get/set field methods.
         */
        vgetparent: tif.tif_tagmethods.vgetfield,
        vsetparent: tif.tif_tagmethods.vsetfield,
    };
    tif.tif_data = TifData::LogLuv(Box::new(sp));

    /*
     * Install codec methods.
     * NB: tif_decoderow & tif_encoderow are filled
     *     in at setup time.
     */
    tif.tif_fixuptags = log_luv_fixup_tags;
    tif.tif_setupdecode = log_luv_setup_decode;
    tif.tif_decodestrip = log_luv_decode_strip;
    tif.tif_decodetile = log_luv_decode_tile;
    tif.tif_close = log_luv_close;
    tif.tif_cleanup = log_luv_cleanup;

    tif.tif_tagmethods.vgetfield = log_luv_vget_field; /* hook for codec tags */
    tif.tif_tagmethods.vsetfield = log_luv_vset_field; /* hook for codec tags */

    1
}


/// `uv_row`: the rows of the (u',v') grid.
#[rustfmt::skip]
static UV_ROW: [UvRow; UV_NVS as usize] = [
    r(0.247663, 4, 0), r(0.243779, 6, 4), r(0.241684, 7, 10), r(0.237874, 9, 17),
    r(0.235906, 10, 26), r(0.232153, 12, 36), r(0.228352, 14, 48), r(0.226259, 15, 62),
    r(0.222371, 17, 77), r(0.220410, 18, 94), r(0.214710, 21, 112), r(0.212714, 22, 133),
    r(0.210721, 23, 155), r(0.204976, 26, 178), r(0.202986, 27, 204), r(0.199245, 29, 231),
    r(0.195525, 31, 260), r(0.193560, 32, 291), r(0.189878, 34, 323), r(0.186216, 36, 357),
    r(0.186216, 36, 393), r(0.182592, 38, 429), r(0.179003, 40, 467), r(0.175466, 42, 507),
    r(0.172001, 44, 549), r(0.172001, 44, 593), r(0.168612, 46, 637), r(0.168612, 46, 683),
    r(0.163575, 49, 729), r(0.158642, 52, 778), r(0.158642, 52, 830), r(0.158642, 52, 882),
    r(0.153815, 55, 934), r(0.153815, 55, 989), r(0.149097, 58, 1044), r(0.149097, 58, 1102),
    r(0.142746, 62, 1160), r(0.142746, 62, 1222), r(0.142746, 62, 1284), r(0.138270, 65, 1346),
    r(0.138270, 65, 1411), r(0.138270, 65, 1476), r(0.132166, 69, 1541), r(0.132166, 69, 1610),
    r(0.126204, 73, 1679), r(0.126204, 73, 1752), r(0.126204, 73, 1825), r(0.120381, 77, 1898),
    r(0.120381, 77, 1975), r(0.120381, 77, 2052), r(0.120381, 77, 2129), r(0.112962, 82, 2206),
    r(0.112962, 82, 2288), r(0.112962, 82, 2370), r(0.107450, 86, 2452), r(0.107450, 86, 2538),
    r(0.107450, 86, 2624), r(0.107450, 86, 2710), r(0.100343, 91, 2796), r(0.100343, 91, 2887),
    r(0.100343, 91, 2978), r(0.095126, 95, 3069), r(0.095126, 95, 3164), r(0.095126, 95, 3259),
    r(0.095126, 95, 3354), r(0.088276, 100, 3449), r(0.088276, 100, 3549), r(0.088276, 100, 3649),
    r(0.088276, 100, 3749), r(0.081523, 105, 3849), r(0.081523, 105, 3954), r(0.081523, 105, 4059),
    r(0.081523, 105, 4164), r(0.074861, 110, 4269), r(0.074861, 110, 4379), r(0.074861, 110, 4489),
    r(0.074861, 110, 4599), r(0.068290, 115, 4709), r(0.068290, 115, 4824), r(0.068290, 115, 4939),
    r(0.068290, 115, 5054), r(0.063573, 119, 5169), r(0.063573, 119, 5288), r(0.063573, 119, 5407),
    r(0.063573, 119, 5526), r(0.057219, 124, 5645), r(0.057219, 124, 5769), r(0.057219, 124, 5893),
    r(0.057219, 124, 6017), r(0.050985, 129, 6141), r(0.050985, 129, 6270), r(0.050985, 129, 6399),
    r(0.050985, 129, 6528), r(0.050985, 129, 6657), r(0.044859, 134, 6786), r(0.044859, 134, 6920),
    r(0.044859, 134, 7054), r(0.044859, 134, 7188), r(0.040571, 138, 7322), r(0.040571, 138, 7460),
    r(0.040571, 138, 7598), r(0.040571, 138, 7736), r(0.036339, 142, 7874), r(0.036339, 142, 8016),
    r(0.036339, 142, 8158), r(0.036339, 142, 8300), r(0.032139, 146, 8442), r(0.032139, 146, 8588),
    r(0.032139, 146, 8734), r(0.032139, 146, 8880), r(0.027947, 150, 9026), r(0.027947, 150, 9176),
    r(0.027947, 150, 9326), r(0.023739, 154, 9476), r(0.023739, 154, 9630), r(0.023739, 154, 9784),
    r(0.023739, 154, 9938), r(0.019504, 158, 10092), r(0.019504, 158, 10250), r(0.019504, 158, 10408),
    r(0.016976, 161, 10566), r(0.016976, 161, 10727), r(0.016976, 161, 10888), r(0.016976, 161, 11049),
    r(0.012639, 165, 11210), r(0.012639, 165, 11375), r(0.012639, 165, 11540), r(0.009991, 168, 11705),
    r(0.009991, 168, 11873), r(0.009991, 168, 12041), r(0.009016, 170, 12209), r(0.009016, 170, 12379),
    r(0.009016, 170, 12549), r(0.006217, 173, 12719), r(0.006217, 173, 12892), r(0.005097, 175, 13065),
    r(0.005097, 175, 13240), r(0.005097, 175, 13415), r(0.003909, 177, 13590), r(0.003909, 177, 13767),
    r(0.002340, 177, 13944), r(0.002389, 170, 14121), r(0.001068, 164, 14291), r(0.001653, 157, 14455),
    r(0.000717, 150, 14612), r(0.001614, 143, 14762), r(0.000270, 136, 14905), r(0.000484, 129, 15041),
    r(0.001103, 123, 15170), r(0.001242, 115, 15293), r(0.001188, 109, 15408), r(0.001011, 103, 15517),
    r(0.000709, 97, 15620), r(0.000301, 89, 15717), r(0.002416, 82, 15806), r(0.003251, 76, 15888),
    r(0.003246, 69, 15964), r(0.004141, 62, 16033), r(0.005963, 55, 16095), r(0.008839, 47, 16150),
    r(0.010490, 40, 16197), r(0.016994, 31, 16237), r(0.023659, 21, 16268),
];
