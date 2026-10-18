// Rust translation of the PNM loader of stb_image.h v2.30 (Sean Barrett and
// contributors, http://nothings.org/stb; PGM/PPM by Ken Miller, 16-bit PNM
// by Simon Breuss), from the copy PlutoVG bundles
// (source/plutovg-stb-image.h in SDL_ttf's external/plutovg), which SDL's
// own copy compiles out (`STBI_ONLY_PNG`, `STBI_ONLY_JPEG`).
// stb_image is available under the MIT License or in the public domain
// (see LICENSE.txt).
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Binary PGM (P5) and PPM (P6), 8 and 16 bits per sample, with header
//! comments. No ASCII image data (P2 and P3).

use super::stb_image::{
    convert_format16, convert_format8, err, mad4sizes_valid, Context, Image, Pixels,
    STBI_MAX_DIMENSIONS,
};
use crate::error::Result;

/// Translation of `stbi__pnm_test()`.
pub(crate) fn pnm_test(s: &mut Context<'_>) -> bool {
    let p = s.get8();
    let t = s.get8();
    if p != b'P' || (t != b'5' && t != b'6') {
        s.rewind();
        return false;
    }
    true
}

/// Translation of `stbi__pnm_load()`.
pub(crate) fn pnm_load(s: &mut Context<'_>, req_comp: i32) -> Result<Image<Pixels>> {
    let (x, y, n, bits_per_channel) = pnm_info(s)?;
    s.img_x = x as u32;
    s.img_y = y as u32;
    s.img_n = n;

    if s.img_y > STBI_MAX_DIMENSIONS {
        return err("Very large image (corrupt?)");
    }
    if s.img_x > STBI_MAX_DIMENSIONS {
        return err("Very large image (corrupt?)");
    }

    let bytes = bits_per_channel / 8;
    if !mad4sizes_valid(s.img_n, s.img_x as i32, s.img_y as i32, bytes, 0) {
        return err("PNM too large");
    }

    let mut out = vec![0u8; (s.img_n * s.img_x as i32 * s.img_y as i32 * bytes) as usize];
    if !s.getn(&mut out) {
        return err("PNM file truncated");
    }

    let (img_n, img_x, img_y) = (s.img_n, s.img_x, s.img_y);
    let data = if bits_per_channel == 16 {
        // (the samples, big-endian in the file, are read in the host's
        // byte order, as the C reads them)
        let out16: Vec<u16> = out
            .chunks_exact(2)
            .map(|b| u16::from_ne_bytes([b[0], b[1]]))
            .collect();
        Pixels::Bits16(if req_comp != 0 && req_comp != img_n {
            convert_format16(out16, img_n, req_comp, img_x, img_y)?
        } else {
            out16
        })
    } else {
        Pixels::Bits8(if req_comp != 0 && req_comp != img_n {
            convert_format8(out, img_n, req_comp, img_x, img_y)?
        } else {
            out
        })
    };
    Ok(Image {
        x: img_x as i32,
        y: img_y as i32,
        comp: img_n,
        data,
    })
}

/// Translation of `stbi__pnm_isspace()` (on a `char`, signed here).
fn pnm_isspace(c: i8) -> bool {
    c == b' ' as i8
        || c == b'\t' as i8
        || c == b'\n' as i8
        || c == 0x0b
        || c == 0x0c
        || c == b'\r' as i8
}

/// Translation of `stbi__pnm_skip_whitespace()`.
fn pnm_skip_whitespace(s: &mut Context<'_>, c: &mut i8) {
    loop {
        while !s.at_eof() && pnm_isspace(*c) {
            *c = s.get8() as i8;
        }

        if s.at_eof() || *c != b'#' as i8 {
            break;
        }

        while !s.at_eof() && *c != b'\n' as i8 && *c != b'\r' as i8 {
            *c = s.get8() as i8;
        }
    }
}

/// Translation of `stbi__pnm_isdigit()`.
fn pnm_isdigit(c: i8) -> bool {
    c >= b'0' as i8 && c <= b'9' as i8
}

/// Translation of `stbi__pnm_getinteger()`: 0 on overflow (an error).
fn pnm_getinteger(s: &mut Context<'_>, c: &mut i8) -> Result<i32> {
    let mut value: i32 = 0;

    while !s.at_eof() && pnm_isdigit(*c) {
        value = value * 10 + (*c as i32 - b'0' as i32);
        *c = s.get8() as i8;
        if (value > 214748364) || (value == 214748364 && *c > b'7' as i8) {
            return err("Parsing an integer in the PPM header overflowed a 32-bit int");
        }
    }

    Ok(value)
}

/// Translation of `stbi__pnm_info()`: width, height, components and bits
/// per channel (where the C returns 0, an error, and it sets one for all
/// but the identifier's).
fn pnm_info(s: &mut Context<'_>) -> Result<(i32, i32, i32, i32)> {
    s.rewind();

    // Get identifier
    let p = s.get8();
    let t = s.get8();
    if p != b'P' || (t != b'5' && t != b'6') {
        s.rewind();
        return err("Corrupt PNM");
    }

    let comp = if t == b'6' { 3 } else { 1 }; // '5' is 1-component .pgm; '6' is 3-component .ppm

    let mut c = s.get8() as i8;
    pnm_skip_whitespace(s, &mut c);

    // (stbi__err() returns 0, which the checks after take for the value)
    let x = pnm_getinteger(s, &mut c).unwrap_or(0); // read width
    if x == 0 {
        return err("PPM image header had zero or overflowing width");
    }
    pnm_skip_whitespace(s, &mut c);

    let y = pnm_getinteger(s, &mut c).unwrap_or(0); // read height
    if y == 0 {
        return err("PPM image header had zero or overflowing width");
    }
    pnm_skip_whitespace(s, &mut c);

    let maxv = pnm_getinteger(s, &mut c).unwrap_or(0); // read max value
    if maxv > 65535 {
        err("PPM image supports only 8-bit and 16-bit images")
    } else if maxv > 255 {
        Ok((x, y, comp, 16))
    } else {
        Ok((x, y, comp, 8))
    }
}
