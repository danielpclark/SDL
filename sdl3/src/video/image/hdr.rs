// Rust translation of the Radiance RGBE HDR loader of stb_image.h v2.30
// (Sean Barrett and contributors, http://nothings.org/stb; "originally by
// Nicolas Schulz") and of its conversion to 8 bits (`stbi__hdr_to_ldr()`),
// from the copy PlutoVG bundles (source/plutovg-stb-image.h in SDL_ttf's
// external/plutovg), which SDL's own copy compiles out (`STBI_ONLY_PNG`,
// `STBI_ONLY_JPEG`).
// stb_image is available under the MIT License or in the public domain
// (see LICENSE.txt).
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Radiance RGBE HDR: flat or run-length encoded scanlines, decoded to
//! floats and converted to 8 bits with stb_image's default gamma (2.2) and
//! scale (1).
//!
//! The conversion's `pow()` is SDL's ([`crate::stdlib::math::pow`]): for
//! every pixel value an RGBE file can hold it gives the bytes glibc's
//! `pow()` gives (see the test below).

use super::stb_image::{
    err, mad2sizes_valid, mad3sizes_valid, mad4sizes_valid, Context, STBI_MAX_DIMENSIONS,
};
use crate::error::Result;

/// Translation of `stbi__hdr_test_core()`.
fn hdr_test_core(s: &mut Context<'_>, signature: &[u8]) -> bool {
    for &c in signature {
        if s.get8() != c {
            return false;
        }
    }
    s.rewind();
    true
}

/// Translation of `stbi__hdr_test()`.
pub(crate) fn hdr_test(s: &mut Context<'_>) -> bool {
    let mut r = hdr_test_core(s, b"#?RADIANCE\n");
    s.rewind();
    if !r {
        r = hdr_test_core(s, b"#?RGBE\n");
        s.rewind();
    }
    r
}

/// `STBI__HDR_BUFLEN`
const HDR_BUFLEN: usize = 1024;

/// Translation of `stbi__hdr_gettoken()`: the token as the C string it
/// reads (up to its first NUL).
fn hdr_gettoken(z: &mut Context<'_>) -> Vec<u8> {
    let mut buffer = Vec::new();

    let mut c = z.get8();

    while !z.at_eof() && c != b'\n' {
        buffer.push(c);
        if buffer.len() == HDR_BUFLEN - 1 {
            // flush to end of line
            while !z.at_eof() && z.get8() != b'\n' {}
            break;
        }
        c = z.get8();
    }

    if let Some(nul) = buffer.iter().position(|&b| b == 0) {
        buffer.truncate(nul);
    }
    buffer
}

/// `strtol(nptr, &endptr, 10)` of glibc (whose `long` has 64 bits; the C
/// casts the result to `int`): the value and the index of `endptr`.
fn strtol(nptr: &[u8]) -> (i64, usize) {
    let mut i = 0;
    while i < nptr.len() && matches!(nptr[i], b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
        i += 1;
    }
    let mut negative = false;
    if i < nptr.len() && (nptr[i] == b'+' || nptr[i] == b'-') {
        negative = nptr[i] == b'-';
        i += 1;
    }
    let start = i;
    let mut value: i64 = 0;
    let mut overflow = false;
    while i < nptr.len() && nptr[i].is_ascii_digit() {
        let d = (nptr[i] - b'0') as i64;
        // accumulate negatively, as LONG_MIN has no positive counterpart
        match value.checked_mul(10).and_then(|v| v.checked_sub(d)) {
            Some(v) => value = v,
            None => overflow = true,
        }
        i += 1;
    }
    if i == start {
        // no digits: endptr is nptr
        return (0, 0);
    }
    let value = if overflow {
        if negative {
            i64::MIN
        } else {
            i64::MAX
        }
    } else if negative {
        value
    } else {
        value.checked_neg().unwrap_or(i64::MAX)
    };
    (value, i)
}

/// `ldexp(1.0f, e)` as a float, for the exponents of RGBE pixels
/// (-135 to 119, all exact).
fn exp2f(e: i32) -> f32 {
    f64::from_bits(((e + 1023) as u64) << 52) as f32
}

/// Translation of `stbi__hdr_convert()`.
fn hdr_convert(output: &mut [f32], input: &[u8; 4], req_comp: i32) {
    if input[3] != 0 {
        // Exponent
        let f1 = exp2f(input[3] as i32 - (128 + 8));
        if req_comp <= 2 {
            output[0] = (input[0] as i32 + input[1] as i32 + input[2] as i32) as f32 * f1 / 3.0;
        } else {
            output[0] = input[0] as f32 * f1;
            output[1] = input[1] as f32 * f1;
            output[2] = input[2] as f32 * f1;
        }
        if req_comp == 2 {
            output[1] = 1.0;
        }
        if req_comp == 4 {
            output[3] = 1.0;
        }
    } else {
        match req_comp {
            4 => {
                output[3] = 1.0;
                output[..3].fill(0.0);
            }
            3 => output[..3].fill(0.0),
            2 => {
                output[1] = 1.0;
                output[0] = 0.0;
            }
            1 => output[0] = 0.0,
            _ => {}
        }
    }
}

/// A decoded HDR image: `x` by `y` pixels of `comp` floats.
pub(crate) struct HdrImage {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) data: Vec<f32>,
}

/// Translation of `stbi__hdr_load()`: the image, and the number of
/// components in the file (always 3).
pub(crate) fn hdr_load(s: &mut Context<'_>, req_comp: i32) -> Result<HdrImage> {
    // Check identifier
    let header_token = hdr_gettoken(s);
    if header_token != b"#?RADIANCE" && header_token != b"#?RGBE" {
        return err("Corrupt HDR image");
    }

    // Parse header
    let mut valid = false;
    loop {
        let token = hdr_gettoken(s);
        if token.is_empty() {
            break;
        }
        if token == b"FORMAT=32-bit_rle_rgbe" {
            valid = true;
        }
    }

    if !valid {
        return err("Unsupported HDR format");
    }

    // Parse width and height
    // can't use sscanf() if we're not using stdio!
    let token = hdr_gettoken(s);
    if !token.starts_with(b"-Y ") {
        return err("Unsupported HDR format");
    }
    let mut t = &token[3..];
    let (height, end) = strtol(t);
    let height = height as i32;
    t = &t[end..];
    while t.first() == Some(&b' ') {
        t = &t[1..];
    }
    if !t.starts_with(b"+X ") {
        return err("Unsupported HDR format");
    }
    t = &t[3..];
    let width = strtol(t).0 as i32;

    if height > STBI_MAX_DIMENSIONS as i32 {
        return err("Very large image (corrupt?)");
    }
    if width > STBI_MAX_DIMENSIONS as i32 {
        return err("Very large image (corrupt?)");
    }

    let req_comp = if req_comp == 0 { 3 } else { req_comp };

    if !mad4sizes_valid(width, height, req_comp, 4, 0) {
        return err("HDR image is too large");
    }

    // Read data
    // (malloc'd uninitialized in C; every value is written below)
    let mut hdr_data = vec![0f32; (width * height * req_comp) as usize];
    let rc = req_comp as usize;
    let w = width as usize;

    /* FIXME (upstream): the C's rgbe array is uninitialized, and a pixel
    that stbi__getn() can't read whole keeps what it held (in practice the
    previous pixel's bytes); it starts zeroed here and keeps the previous
    pixel's. */
    let mut rgbe = [0u8; 4];
    let mut flat = |s: &mut Context<'_>, hdr_data: &mut [f32], j0: usize, i0: usize| {
        for j in j0..height as usize {
            for i in if j == j0 { i0 } else { 0 }..w {
                s.getn(&mut rgbe);
                hdr_convert(&mut hdr_data[(j * w + i) * rc..], &rgbe, req_comp);
            }
        }
    };

    // Load image data
    // image data is stored as some number of sca
    if !(8..32768).contains(&width) {
        // Read flat data
        flat(s, &mut hdr_data, 0, 0);
    } else {
        // Read RLE-encoded data
        let mut scanline: Vec<u8> = Vec::new();

        for j in 0..height as usize {
            let c1 = s.get8();
            let c2 = s.get8();
            let mut len = s.get8() as i32;
            if c1 != 2 || c2 != 2 || (len & 0x80) != 0 {
                // not run-length encoded, so we have to actually use THIS data as a decoded
                // pixel (note this can't be a valid pixel--one of RGB must be >= 128)
                let first = [c1, c2, len as u8, s.get8()];
                hdr_convert(&mut hdr_data, &first, req_comp);
                // (goto main_decode_loop with i = 1, j = 0: yes, this makes no sense)
                flat(s, &mut hdr_data, 0, 1);
                return Ok(HdrImage {
                    x: width,
                    y: height,
                    data: hdr_data,
                });
            }
            len <<= 8;
            len |= s.get8() as i32;
            if len != width {
                return err("corrupt HDR");
            }
            if scanline.is_empty() {
                if !mad2sizes_valid(width, 4, 0) {
                    return err("Out of memory");
                }
                // (malloc'd uninitialized in C; every row is written whole)
                scanline = vec![0; w * 4];
            }

            for k in 0..4 {
                let mut i = 0usize;
                loop {
                    let nleft = w as i32 - i as i32;
                    if nleft <= 0 {
                        break;
                    }
                    let mut count = s.get8();
                    if count > 128 {
                        // Run
                        let value = s.get8();
                        count -= 128;
                        if count == 0 || count as i32 > nleft {
                            return err("bad RLE data in HDR");
                        }
                        for _ in 0..count {
                            scanline[i * 4 + k] = value;
                            i += 1;
                        }
                    } else {
                        // Dump
                        if count == 0 || count as i32 > nleft {
                            return err("bad RLE data in HDR");
                        }
                        for _ in 0..count {
                            scanline[i * 4 + k] = s.get8();
                            i += 1;
                        }
                    }
                }
            }
            for i in 0..w {
                let px = [
                    scanline[i * 4],
                    scanline[i * 4 + 1],
                    scanline[i * 4 + 2],
                    scanline[i * 4 + 3],
                ];
                hdr_convert(&mut hdr_data[(j * w + i) * rc..], &px, req_comp);
            }
        }
    }

    Ok(HdrImage {
        x: width,
        y: height,
        data: hdr_data,
    })
}

/// `stbi__h2l_gamma_i`
const H2L_GAMMA_I: f32 = 1.0 / 2.2;
/// `stbi__h2l_scale_i`
const H2L_SCALE_I: f32 = 1.0;

/// Translation of `stbi__hdr_to_ldr()`.
pub(crate) fn hdr_to_ldr(data: &[f32], x: i32, y: i32, comp: i32) -> Result<Vec<u8>> {
    if !mad3sizes_valid(x, y, comp, 0) {
        return err("Out of memory");
    }
    let mut output = vec![0u8; (x * y * comp) as usize];
    // compute number of non-alpha components
    let n = if comp & 1 != 0 { comp } else { comp - 1 } as usize;
    let comp = comp as usize;
    for (out, px) in output.chunks_exact_mut(comp).zip(data.chunks_exact(comp)) {
        for k in 0..n {
            let mut z = crate::stdlib::math::pow((px[k] * H2L_SCALE_I) as f64, H2L_GAMMA_I as f64)
                as f32
                * 255.0
                + 0.5;
            z = z.clamp(0.0, 255.0);
            out[k] = z as i32 as u8; // stbi__float2int
        }
        if n < comp {
            let mut z = px[n] * 255.0 + 0.5;
            z = z.clamp(0.0, 255.0);
            out[n] = z as i32 as u8;
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every value of an RGBE pixel (mantissa and exponent) converted to
    /// 8 bits: the hash of the bytes a C program gives with glibc's
    /// `pow()` (2.39, x86-64).
    #[test]
    fn hdr_to_ldr_matches_glibc_pow() {
        let mut h: u64 = 14695981039346656037;
        for e in 0..256 {
            for m in 0..256 {
                let mut v = [0f32; 4];
                hdr_convert(&mut v, &[m as u8, 0, 0, e as u8], 4);
                let b = hdr_to_ldr(&v, 1, 1, 4).unwrap()[0];
                h ^= b as u64;
                h = h.wrapping_mul(1099511628211);
            }
        }
        assert_eq!(h, 0x0ddbda265e79e843);
    }

    #[test]
    fn strtol_as_glibc() {
        assert_eq!(strtol(b"  12 +X"), (12, 4));
        assert_eq!(strtol(b"-7"), (-7, 2));
        assert_eq!(strtol(b"+x"), (0, 0));
        assert_eq!(strtol(b"99999999999999999999"), (i64::MAX, 20));
        assert_eq!(strtol(b"-9223372036854775808"), (i64::MIN, 20));
        assert_eq!(strtol(b"-99999999999999999999"), (i64::MIN, 21));
    }
}
