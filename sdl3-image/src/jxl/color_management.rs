// Rust translation of lib/jxl/color_management.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The ICC profiles made from color encoding fields (`MaybeCreateProfile()`,
//! which the decoder calls when it reads a color encoding: its failures are
//! the decoder's).

use super::base::{jxl_failure, PaddedBytes, Status};
use super::color_encoding_internal::{
    adapt_to_xyz_d50, description, primaries_to_xyz_d50, CIExy, ColorEncoding, ColorSpace,
    TransferFunction,
};
use super::transfer_functions::{TfHlg, TfPq};

/// Translation of `ExtraTF` (the two the table curves are for).
#[derive(Clone, Copy, PartialEq, Eq)]
enum ExtraTF {
    Pq,
    Hlg,
}

// NOTE: this is only used to provide a reasonable ICC profile that other
// software can read. Our own transforms use ExtraTF instead because that is
// more precise and supports unbounded mode.
/// Translation of `CreateTableCurve()`.
fn create_table_curve(n: u32, tf: ExtraTF) -> Vec<u16> {
    debug_assert!(n <= 4096); // ICC MFT2 only allows 4K entries
                              // No point using float - LCMS converts to 16-bit for A2B/MFT.
    let mut table = vec![0u16; n as usize];
    for i in 0..n {
        let x = i as f32 / (n - 1) as f32; // 1.0 at index N - 1.
        let dx = x as f64;
        // LCMS requires EOTF (e.g. 2.4 exponent).
        let mut y = if tf == ExtraTF::Hlg {
            TfHlg.display_from_encoded(dx)
        } else {
            TfPq.display_from_encoded(dx)
        };
        debug_assert!(y >= 0.0);
        // Clamp to table range - necessary for HLG.
        if y > 1.0 {
            y = 1.0;
        }
        // 1.0 corresponds to table value 0xFFFF.
        table[i as usize] = super::math::roundf((y * 65535.0) as f32) as u16;
    }
    table
}

/// Translation of `CIEXYZFromWhiteCIExy()`.
fn ciexyz_from_white_ciexy(xy: &CIExy, xyz: &mut [f32; 3]) -> Status {
    // Target Y = 1.
    if xy.y.abs() < 1e-12 {
        return jxl_failure!("Y value is too small");
    }
    let factor = (1.0 / xy.y) as f32;
    xyz[0] = (xy.x * factor as f64) as f32;
    xyz[1] = 1.0;
    xyz[2] = ((1.0 - xy.x - xy.y) * factor as f64) as f32;
    Ok(())
}

/// Translation of `ICCComputeMD5()`.
fn icc_compute_md5(data: &PaddedBytes, sum: &mut [u8; 16]) {
    let mut data64 = data.clone();
    data64.push(128);
    // Add bytes such that ((size + 8) & 63) == 0.
    let extra = (64 - ((data64.len() + 8) & 63)) & 63;
    data64.resize(data64.len() + extra, 0);
    let mut i: u64 = 0;
    while i < 64 {
        data64.push(((data.len() as u64) << 3u64 >> i) as u8);
        i += 8;
    }

    static SINEPARTS: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];
    static SHIFT: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];

    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;

    let mut i = 0;
    while i < data64.len() {
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for j in 0..64 {
            let mut f: u32;
            let g: usize;
            if j < 16 {
                f = (b & c) | ((!b) & d);
                g = j;
            } else if j < 32 {
                f = (d & b) | ((!d) & c);
                g = (5 * j + 1) & 0xf;
            } else if j < 48 {
                f = b ^ c ^ d;
                g = (3 * j + 5) & 0xf;
            } else {
                f = c ^ (b | (!d));
                g = (7 * j) & 0xf;
            }
            let dg0 = data64[i + g * 4] as u32;
            let dg1 = data64[i + g * 4 + 1] as u32;
            let dg2 = data64[i + g * 4 + 2] as u32;
            let dg3 = data64[i + g * 4 + 3] as u32;
            let u = dg0 | (dg1 << 8) | (dg2 << 16) | (dg3 << 24);
            f = f.wrapping_add(a).wrapping_add(SINEPARTS[j]).wrapping_add(u);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(SHIFT[j]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
        i += 64;
    }
    sum[0..4].copy_from_slice(&a0.to_le_bytes());
    sum[4..8].copy_from_slice(&b0.to_le_bytes());
    sum[8..12].copy_from_slice(&c0.to_le_bytes());
    sum[12..16].copy_from_slice(&d0.to_le_bytes());
}

/// Translation of `CreateICCChadMatrix()`.
fn create_icc_chad_matrix(w: CIExy, result: &mut [f32; 9]) -> Status {
    let mut m = [0f32; 9];
    if w.y == 0.0 {
        // WhitePoint can not be pitch-black.
        return jxl_failure!("Invalid WhitePoint");
    }
    adapt_to_xyz_d50(w.x as f32, w.y as f32, &mut m)?;
    *result = m;
    Ok(())
}

/// Creates RGB to XYZ matrix given RGB primaries and whitepoint in xy.
/// Translation of `CreateICCRGBMatrix()`.
fn create_icc_rgb_matrix(r: CIExy, g: CIExy, b: CIExy, w: CIExy, result: &mut [f32; 9]) -> Status {
    let mut m = [0f32; 9];
    primaries_to_xyz_d50(
        r.x as f32, r.y as f32, g.x as f32, g.y as f32, b.x as f32, b.y as f32, w.x as f32,
        w.y as f32, &mut m,
    )?;
    *result = m;
    Ok(())
}

fn write_icc_uint32(value: u32, pos: usize, icc: &mut PaddedBytes) {
    if icc.len() < pos + 4 {
        icc.resize(pos + 4, 0);
    }
    icc[pos..pos + 4].copy_from_slice(&value.to_be_bytes());
}

fn write_icc_uint16(value: u16, pos: usize, icc: &mut PaddedBytes) {
    if icc.len() < pos + 2 {
        icc.resize(pos + 2, 0);
    }
    icc[pos..pos + 2].copy_from_slice(&value.to_be_bytes());
}

// Writes a 4-character tag
fn write_icc_tag(value: &[u8; 4], pos: usize, icc: &mut PaddedBytes) {
    if icc.len() < pos + 4 {
        icc.resize(pos + 4, 0);
    }
    icc[pos..pos + 4].copy_from_slice(value);
}

fn write_icc_s15_fixed16(value: f32, pos: usize, icc: &mut PaddedBytes) -> Status {
    // "nextafterf" for 32768.0f towards zero are:
    // 32767.998046875, 32767.99609375, 32767.994140625
    // Even the first value works well,...
    let ok = (-32767.995f32..=32767.995f32).contains(&value);
    if !ok {
        return jxl_failure!("ICC value is out of range / NaN");
    }
    let i = (value * 65536.0f32 + 0.5f32) as i32;
    // Use two's complement
    let u = i as u32;
    write_icc_uint32(u, pos, icc);
    Ok(())
}

fn create_icc_header(c: &ColorEncoding, header: &mut PaddedBytes) -> Status {
    // TODO(lode): choose color management engine name, e.g. "skia" if
    // integrated in skia.
    let k_cmm = b"jxl ";

    header.resize(128, 0);

    write_icc_uint32(0, 0, header); // size, correct value filled in at end
    write_icc_tag(k_cmm, 4, header);
    write_icc_uint32(0x04300000, 8, header);
    write_icc_tag(b"mntr", 12, header);
    write_icc_tag(if c.is_gray() { b"GRAY" } else { b"RGB " }, 16, header);
    write_icc_tag(b"XYZ ", 20, header);

    // Three uint32_t's date/time encoding.
    // TODO(lode): encode actual date and time, this is a placeholder
    let (year, month, day) = (2019u16, 12u16, 1u16);
    let (hour, minute, second) = (0u16, 0u16, 0u16);
    write_icc_uint16(year, 24, header);
    write_icc_uint16(month, 26, header);
    write_icc_uint16(day, 28, header);
    write_icc_uint16(hour, 30, header);
    write_icc_uint16(minute, 32, header);
    write_icc_uint16(second, 34, header);

    write_icc_tag(b"acsp", 36, header);
    write_icc_tag(b"APPL", 40, header);
    write_icc_uint32(0, 44, header); // flags
    write_icc_uint32(0, 48, header); // device manufacturer
    write_icc_uint32(0, 52, header); // device model
    write_icc_uint32(0, 56, header); // device attributes
    write_icc_uint32(0, 60, header); // device attributes
    write_icc_uint32(c.rendering_intent as u32, 64, header);

    // Mandatory D50 white point of profile connection space
    write_icc_uint32(0x0000f6d6, 68, header);
    write_icc_uint32(0x00010000, 72, header);
    write_icc_uint32(0x0000d32d, 76, header);

    write_icc_tag(k_cmm, 80, header);

    Ok(())
}

fn add_to_icc_tag_table(
    tag: &[u8; 4],
    offset: usize,
    size: usize,
    tagtable: &mut PaddedBytes,
    offsets: &mut Vec<usize>,
) {
    let pos = tagtable.len();
    write_icc_tag(tag, pos, tagtable);
    // writing true offset deferred to later
    let pos = tagtable.len();
    write_icc_uint32(0, pos, tagtable);
    offsets.push(offset);
    let pos = tagtable.len();
    write_icc_uint32(size as u32, pos, tagtable);
}

fn finalize_icc_tag(tags: &mut PaddedBytes, offset: &mut usize, size: &mut usize) {
    while (tags.len() & 3) != 0 {
        tags.push(0);
    }
    *offset += *size;
    *size = tags.len() - *offset;
}

// The input text must be ASCII, writing other characters to UTF-16 is not
// implemented.
fn create_icc_mluc_tag(text: &str, tags: &mut PaddedBytes) {
    let pos = tags.len();
    write_icc_tag(b"mluc", pos, tags);
    let pos = tags.len();
    write_icc_uint32(0, pos, tags);
    let pos = tags.len();
    write_icc_uint32(1, pos, tags);
    let pos = tags.len();
    write_icc_uint32(12, pos, tags);
    let pos = tags.len();
    write_icc_tag(b"enUS", pos, tags);
    let pos = tags.len();
    write_icc_uint32(text.len() as u32 * 2, pos, tags);
    let pos = tags.len();
    write_icc_uint32(28, pos, tags);
    for &ch in text.as_bytes() {
        tags.push(0); // prepend 0 for UTF-16
        tags.push(ch);
    }
}

fn create_icc_xyz_tag(xyz: &[f32; 3], tags: &mut PaddedBytes) -> Status {
    let pos = tags.len();
    write_icc_tag(b"XYZ ", pos, tags);
    let pos = tags.len();
    write_icc_uint32(0, pos, tags);
    for &v in xyz {
        let pos = tags.len();
        write_icc_s15_fixed16(v, pos, tags)?;
    }
    Ok(())
}

fn create_icc_chad_tag(chad: &[f32; 9], tags: &mut PaddedBytes) -> Status {
    let pos = tags.len();
    write_icc_tag(b"sf32", pos, tags);
    let pos = tags.len();
    write_icc_uint32(0, pos, tags);
    for &v in chad {
        let pos = tags.len();
        write_icc_s15_fixed16(v, pos, tags)?;
    }
    Ok(())
}

fn create_icc_curv_curv_tag(curve: &[u16], tags: &mut PaddedBytes) {
    let pos = tags.len();
    tags.resize(tags.len() + 12 + curve.len() * 2, 0);
    write_icc_tag(b"curv", pos, tags);
    write_icc_uint32(0, pos + 4, tags);
    write_icc_uint32(curve.len() as u32, pos + 8, tags);
    for (i, &v) in curve.iter().enumerate() {
        write_icc_uint16(v, pos + 12 + i * 2, tags);
    }
}

fn create_icc_curv_para_tag(params: &[f32], curve_type: usize, tags: &mut PaddedBytes) -> Status {
    let pos = tags.len();
    write_icc_tag(b"para", pos, tags);
    let pos = tags.len();
    write_icc_uint32(0, pos, tags);
    let pos = tags.len();
    write_icc_uint16(curve_type as u16, pos, tags);
    let pos = tags.len();
    write_icc_uint16(0, pos, tags);
    for &p in params {
        let pos = tags.len();
        write_icc_s15_fixed16(p, pos, tags)?;
    }
    Ok(())
}

/// Translation of `MaybeCreateProfile()` (`Err` for its "not an error"
/// false too: the caller only tells success from failure).
pub(crate) fn maybe_create_profile(c: &ColorEncoding, icc: &mut PaddedBytes) -> Status {
    let mut header = PaddedBytes::new();
    let mut tagtable = PaddedBytes::new();
    let mut tags = PaddedBytes::new();

    if c.get_color_space() == ColorSpace::Unknown || c.tf.is_unknown() {
        return jxl_failure!("Not an error");
    }

    match c.get_color_space() {
        ColorSpace::Rgb | ColorSpace::Gray => {} // OK
        ColorSpace::Xyb => return jxl_failure!("XYB ICC not yet implemented"),
        _ => return jxl_failure!("Invalid CS"),
    }

    create_icc_header(c, &mut header)?;

    let mut offsets: Vec<usize> = Vec::new();
    // tag count, deferred to later
    let pos = tagtable.len();
    write_icc_uint32(0, pos, &mut tagtable);

    let mut tag_offset = 0usize;
    let mut tag_size = 0usize;

    create_icc_mluc_tag(&description(c), &mut tags);
    finalize_icc_tag(&mut tags, &mut tag_offset, &mut tag_size);
    add_to_icc_tag_table(b"desc", tag_offset, tag_size, &mut tagtable, &mut offsets);

    let copyright = "Copyright 2019 Google LLC, CC-BY-SA 3.0 Unported \
                     license(https://creativecommons.org/licenses/by-sa/3.0/legalcode)";
    create_icc_mluc_tag(copyright, &mut tags);
    finalize_icc_tag(&mut tags, &mut tag_offset, &mut tag_size);
    add_to_icc_tag_table(b"cprt", tag_offset, tag_size, &mut tagtable, &mut offsets);

    // TODO(eustas): isn't it the other way round: gray image has d50 WhitePoint?
    if c.is_gray() {
        let mut wtpt = [0f32; 3];
        ciexyz_from_white_ciexy(&c.get_white_point(), &mut wtpt)?;
        create_icc_xyz_tag(&wtpt, &mut tags)?;
    } else {
        let d50 = [0.964203f32, 1.0, 0.824905];
        create_icc_xyz_tag(&d50, &mut tags)?;
    }
    finalize_icc_tag(&mut tags, &mut tag_offset, &mut tag_size);
    add_to_icc_tag_table(b"wtpt", tag_offset, tag_size, &mut tagtable, &mut offsets);

    if !c.is_gray() {
        // Chromatic adaptation matrix
        let mut chad = [0f32; 9];
        create_icc_chad_matrix(c.get_white_point(), &mut chad)?;

        let primaries = c.get_primaries();
        let mut m = [0f32; 9];
        create_icc_rgb_matrix(
            primaries.r,
            primaries.g,
            primaries.b,
            c.get_white_point(),
            &mut m,
        )?;
        let r = [m[0], m[3], m[6]];
        let g = [m[1], m[4], m[7]];
        let b = [m[2], m[5], m[8]];

        create_icc_chad_tag(&chad, &mut tags)?;
        finalize_icc_tag(&mut tags, &mut tag_offset, &mut tag_size);
        add_to_icc_tag_table(b"chad", tag_offset, tag_size, &mut tagtable, &mut offsets);

        create_icc_xyz_tag(&r, &mut tags)?;
        finalize_icc_tag(&mut tags, &mut tag_offset, &mut tag_size);
        add_to_icc_tag_table(b"rXYZ", tag_offset, tag_size, &mut tagtable, &mut offsets);

        create_icc_xyz_tag(&g, &mut tags)?;
        finalize_icc_tag(&mut tags, &mut tag_offset, &mut tag_size);
        add_to_icc_tag_table(b"gXYZ", tag_offset, tag_size, &mut tagtable, &mut offsets);

        create_icc_xyz_tag(&b, &mut tags)?;
        finalize_icc_tag(&mut tags, &mut tag_offset, &mut tag_size);
        add_to_icc_tag_table(b"bXYZ", tag_offset, tag_size, &mut tagtable, &mut offsets);
    }

    if c.tf.is_gamma() {
        let gamma = (1.0 / c.tf.get_gamma()) as f32;
        create_icc_curv_para_tag(&[gamma, 1.0, 0.0, 1.0, 0.0], 3, &mut tags)?;
    } else {
        match c.tf.get_transfer_function() {
            TransferFunction::Hlg => {
                create_icc_curv_curv_tag(&create_table_curve(4096, ExtraTF::Hlg), &mut tags);
            }
            TransferFunction::Pq => {
                create_icc_curv_curv_tag(&create_table_curve(4096, ExtraTF::Pq), &mut tags);
            }
            TransferFunction::Srgb => {
                create_icc_curv_para_tag(
                    &[
                        2.4,
                        (1.0 / 1.055) as f32,
                        (0.055 / 1.055) as f32,
                        (1.0 / 12.92) as f32,
                        0.04045,
                    ],
                    3,
                    &mut tags,
                )?;
            }
            TransferFunction::T709 => {
                create_icc_curv_para_tag(
                    &[
                        (1.0 / 0.45) as f32,
                        (1.0 / 1.099) as f32,
                        (0.099 / 1.099) as f32,
                        (1.0 / 4.5) as f32,
                        0.081,
                    ],
                    3,
                    &mut tags,
                )?;
            }
            TransferFunction::Linear => {
                create_icc_curv_para_tag(&[1.0, 1.0, 0.0, 1.0, 0.0], 3, &mut tags)?;
            }
            TransferFunction::Dci => {
                create_icc_curv_para_tag(&[2.6, 1.0, 0.0, 1.0, 0.0], 3, &mut tags)?;
            }
            TransferFunction::Unknown => {
                // JXL_ABORT("Unknown TF %u", ...) (excluded above)
                return jxl_failure!("Unknown TF");
            }
        }
    }
    finalize_icc_tag(&mut tags, &mut tag_offset, &mut tag_size);
    if c.is_gray() {
        add_to_icc_tag_table(b"kTRC", tag_offset, tag_size, &mut tagtable, &mut offsets);
    } else {
        add_to_icc_tag_table(b"rTRC", tag_offset, tag_size, &mut tagtable, &mut offsets);
        add_to_icc_tag_table(b"gTRC", tag_offset, tag_size, &mut tagtable, &mut offsets);
        add_to_icc_tag_table(b"bTRC", tag_offset, tag_size, &mut tagtable, &mut offsets);
    }

    // Tag count
    write_icc_uint32(offsets.len() as u32, 0, &mut tagtable);
    for (i, &off) in offsets.iter().enumerate() {
        let v = (off + header.len() + tagtable.len()) as u32;
        write_icc_uint32(v, 4 + 12 * i + 4, &mut tagtable);
    }

    // ICC profile size
    let total = (header.len() + tagtable.len() + tags.len()) as u32;
    write_icc_uint32(total, 0, &mut header);

    *icc = header;
    icc.extend_from_slice(&tagtable);
    icc.extend_from_slice(&tags);

    // The MD5 checksum must be computed on the profile with profile flags,
    // rendering intent, and region of the checksum itself, set to 0.
    // TODO(lode): manually verify with a reliable tool that this creates correct
    // signature (profile id) for ICC profiles.
    let mut icc_sum = icc.clone();
    if icc_sum.len() >= 64 + 4 {
        icc_sum[44..48].fill(0);
        icc_sum[64..68].fill(0);
    }
    let mut checksum = [0u8; 16];
    icc_compute_md5(&icc_sum, &mut checksum);

    icc[84..100].copy_from_slice(&checksum);

    Ok(())
}
