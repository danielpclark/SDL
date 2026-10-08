// Rust translation of libtiff/tif_color.c from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1988-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! CIE L*a*b* to CIE XYZ and CIE XYZ to RGB conversion routines are taken
//! from the VIPS library (<http://www.vips.ecs.soton.ac.uk>) with
//! the permission of John Cupitt, the VIPS author.
//!
//! TIFF Library.
//!
//! Color space conversion routines.

use super::tiffio::{TIFFCIELabToRGB, TIFFDisplay, TIFFYCbCrToRGB, CIELABTORGB_TABLE_RANGE};
use super::tiffiop::c_f32_to_i32;

/// Convert color value from the CIE L*a*b* 1976 space to CIE XYZ.
/// Translation of `TIFFCIELabToXYZ()`.
pub(crate) fn tiff_cie_lab_to_xyz(
    cielab: &TIFFCIELabToRGB,
    l: u32,
    a: i32,
    b: i32,
) -> (f32, f32, f32) {
    tiff_cie_lab16_to_xyz(
        cielab,
        l.wrapping_mul(257),
        a.wrapping_mul(256),
        b.wrapping_mul(256),
    )
}

/// For CIELab encoded in 16 bits, L is an unsigned integer range [0,65535].
/// The a* and b* components are signed integers range [-32768,32767]. The 16
/// bit chrominance values are encoded as 256 times the 1976 CIE a* and b*
/// values
/// Translation of `TIFFCIELab16ToXYZ()`: the result is (X, Y, Z).
pub(crate) fn tiff_cie_lab16_to_xyz(
    cielab: &TIFFCIELabToRGB,
    l: u32,
    a: i32,
    b: i32,
) -> (f32, f32, f32) {
    let l = l as f32 * 100.0f32 / 65535.0f32;
    let y;
    let cby;

    if l < 8.856f32 {
        y = (l * cielab.Y0) / 903.292f32;
        cby = 7.787f32 * (y / cielab.Y0) + 16.0f32 / 116.0f32;
    } else {
        cby = (l + 16.0f32) / 116.0f32;
        y = cielab.Y0 * cby * cby * cby;
    }

    let mut tmp = a as f32 / 256.0f32 / 500.0f32 + cby;
    let x = if tmp < 0.2069f32 {
        cielab.X0 * (tmp - 0.13793f32) / 7.787f32
    } else {
        cielab.X0 * tmp * tmp * tmp
    };

    tmp = cby - b as f32 / 256.0f32 / 200.0f32;
    let z = if tmp < 0.2069f32 {
        cielab.Z0 * (tmp - 0.13793f32) / 7.787f32
    } else {
        cielab.Z0 * tmp * tmp * tmp
    };
    (x, y, z)
}

/// `RINT()`
fn rint(r: f32) -> u32 {
    if r > 0.0 {
        (r + 0.5f32) as u32
    } else {
        (r - 0.5f32) as u32
    }
}

/// `TIFFmax()` (for floats: a NaN `a` gives `b`)
fn tiff_max(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

/// `TIFFmin()`
fn tiff_min(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}

/// `(size_t)f` (x86-64: out of range and NaN give 2^63, which the
/// callers clamp to the table's range)
fn c_f32_to_usize(f: f32) -> usize {
    if f.is_nan() || f >= 18446744073709551616.0 || f <= -1.0 {
        1usize << 63
    } else {
        f as usize
    }
}

/// Convert color value from the XYZ space to RGB.
/// Translation of `TIFFXYZToRGB()`: the result is (r, g, b).
pub(crate) fn tiff_xyz_to_rgb(cielab: &TIFFCIELabToRGB, x: f32, y: f32, z: f32) -> (u32, u32, u32) {
    let matrix = &cielab.display.d_mat;

    /* Multiply through the matrix to get luminosity values. */
    let mut yr = matrix[0][0] * x + matrix[0][1] * y + matrix[0][2] * z;
    let mut yg = matrix[1][0] * x + matrix[1][1] * y + matrix[1][2] * z;
    let mut yb = matrix[2][0] * x + matrix[2][1] * y + matrix[2][2] * z;

    /* Clip input */
    yr = tiff_max(yr, cielab.display.d_Y0R);
    yg = tiff_max(yg, cielab.display.d_Y0G);
    yb = tiff_max(yb, cielab.display.d_Y0B);

    /* Avoid overflow in case of wrong input values */
    yr = tiff_min(yr, cielab.display.d_YCR);
    yg = tiff_min(yg, cielab.display.d_YCG);
    yb = tiff_min(yb, cielab.display.d_YCB);

    let range = cielab.range.max(0) as usize;

    /* Turn luminosity to colour value. */
    let mut i = c_f32_to_usize((yr - cielab.display.d_Y0R) / cielab.rstep);
    i = range.min(i);
    let mut r = rint(cielab.Yr2r.get(i).copied().unwrap_or(0.0));

    i = c_f32_to_usize((yg - cielab.display.d_Y0G) / cielab.gstep);
    i = range.min(i);
    let mut g = rint(cielab.Yg2g.get(i).copied().unwrap_or(0.0));

    i = c_f32_to_usize((yb - cielab.display.d_Y0B) / cielab.bstep);
    i = range.min(i);
    let mut b = rint(cielab.Yb2b.get(i).copied().unwrap_or(0.0));

    /* Clip output. */
    r = r.min(cielab.display.d_Vrwr);
    g = g.min(cielab.display.d_Vrwg);
    b = b.min(cielab.display.d_Vrwb);
    (r, g, b)
}

/// Allocate conversion state structures and make look_up tables for
/// the Yr,Yb,Yg <=> r,g,b conversions.
/// Translation of `TIFFCIELabToRGBInit()`.
pub(crate) fn tiff_cie_lab_to_rgb_init(
    cielab: &mut TIFFCIELabToRGB,
    display: &TIFFDisplay,
    ref_white: &[f32; 3],
) -> i32 {
    cielab.range = CIELABTORGB_TABLE_RANGE as i32;

    cielab.display = *display;

    /* Red */
    let mut df_gamma = 1.0 / cielab.display.d_gammaR as f64;
    cielab.rstep = (cielab.display.d_YCR - cielab.display.d_Y0R) / cielab.range as f32;
    for i in 0..=CIELABTORGB_TABLE_RANGE {
        cielab.Yr2r[i] =
            cielab.display.d_Vrwr as f32 * ((i as f64 / cielab.range as f64).powf(df_gamma) as f32);
    }

    // (sic: the green and blue steps are the red channel's, as upstream)
    /* Green */
    df_gamma = 1.0 / cielab.display.d_gammaG as f64;
    cielab.gstep = (cielab.display.d_YCR - cielab.display.d_Y0R) / cielab.range as f32;
    for i in 0..=CIELABTORGB_TABLE_RANGE {
        cielab.Yg2g[i] =
            cielab.display.d_Vrwg as f32 * ((i as f64 / cielab.range as f64).powf(df_gamma) as f32);
    }

    /* Blue */
    df_gamma = 1.0 / cielab.display.d_gammaB as f64;
    cielab.bstep = (cielab.display.d_YCR - cielab.display.d_Y0R) / cielab.range as f32;
    for i in 0..=CIELABTORGB_TABLE_RANGE {
        cielab.Yb2b[i] =
            cielab.display.d_Vrwb as f32 * ((i as f64 / cielab.range as f64).powf(df_gamma) as f32);
    }

    /* Init reference white point */
    cielab.X0 = ref_white[0];
    cielab.Y0 = ref_white[1];
    cielab.Z0 = ref_white[2];

    0
}

/*
 * Convert color value from the YCbCr space to RGB.
 * The colorspace conversion algorithm comes from the IJG v5a code;
 * see below for more information on how it works.
 */
const SHIFT: i32 = 16;
/// `FIX()`
fn fix(x: f32) -> i32 {
    (x as f64 * (1i64 << SHIFT) as f64 + 0.5) as i32
}
const ONE_HALF: i32 = 1 << (SHIFT - 1);
/// `TIFF_FLOAT_EQ()`
pub(crate) fn tiff_float_eq(x: f32, y: f32) -> bool {
    !((x - y).abs() > 0.0f32)
}
/// `Code2V()`
fn code2v(c: i32, rb: f32, rw: f32, cr: i32) -> f32 {
    // ((int32_t)(RB) is out of range for a huge reference black, and the
    // subtraction then overflows: as x86-64 does it)
    (c.wrapping_sub(c_f32_to_i32(rb)) as f32 * cr as f32)
        / if !tiff_float_eq(rw, rb) {
            rw - rb
        } else {
            1.0f32
        }
}
/// `CLAMP()` for floats (`!((f)>=(min))` written that way to deal with
/// NaN)
fn clamp_f(f: f32, min: f32, max: f32) -> f32 {
    if !(f >= min) {
        min
    } else if f > max {
        max
    } else {
        f
    }
}
/// `CLAMP()` for integers
fn clamp_i(f: i32, min: i32, max: i32) -> i32 {
    if f < min {
        min
    } else if f > max {
        max
    } else {
        f
    }
}

/// Translation of `TIFFYCbCrtoRGB()`: the result is (r, g, b).
pub(crate) fn tiff_ycbcr_to_rgb(
    ycbcr: &TIFFYCbCrToRGB,
    mut y: u32,
    cb: i32,
    cr: i32,
) -> (u32, u32, u32) {
    /* XXX: Only 8-bit YCbCr input supported for now */
    if y > 255 {
        y = 255;
    }
    let cb = clamp_i(cb, 0, 255) as usize;
    let cr = clamp_i(cr, 0, 255) as usize;
    let y = y as usize;
    let tab = |t: &Vec<i32>, i: usize| t.get(i).copied().unwrap_or(0);

    let mut i = tab(&ycbcr.Y_tab, y).wrapping_add(tab(&ycbcr.Cr_r_tab, cr));
    let r = clamp_i(i, 0, 255) as u32;
    i = tab(&ycbcr.Y_tab, y)
        .wrapping_add(tab(&ycbcr.Cb_g_tab, cb).wrapping_add(tab(&ycbcr.Cr_g_tab, cr)) >> SHIFT);
    let g = clamp_i(i, 0, 255) as u32;
    i = tab(&ycbcr.Y_tab, y).wrapping_add(tab(&ycbcr.Cb_b_tab, cb));
    let b = clamp_i(i, 0, 255) as u32;
    (r, g, b)
}

/// Clamp function for sanitization purposes. Normally clamping should not
/// occur for well behaved chroma and refBlackWhite coefficients
/// Translation of `CLAMPw()`.
fn clampw(v: f32, vmin: f32, vmax: f32) -> f32 {
    if v < vmin {
        /* printf("%f clamped to %f\n", v, vmin); */
        return vmin;
    }
    if v > vmax {
        /* printf("%f clamped to %f\n", v, vmax); */
        return vmax;
    }
    v
}

/// Initialize the YCbCr->RGB conversion tables.  The conversion
/// is done according to the 6.0 spec:
///
/// ```text
///    R = Y + Cr*(2 - 2*LumaRed)
///    B = Y + Cb*(2 - 2*LumaBlue)
///    G =   Y
///        - LumaBlue*Cb*(2-2*LumaBlue)/LumaGreen
///        - LumaRed*Cr*(2-2*LumaRed)/LumaGreen
/// ```
///
/// To avoid floating point arithmetic the fractional constants that
/// come out of the equations are represented as fixed point values
/// in the range 0...2^16.  We also eliminate multiplications by
/// pre-calculating possible values indexed by Cb and Cr (this code
/// assumes conversion is being done for 8-bit samples).
/// Translation of `TIFFYCbCrToRGBInit()`.
pub(crate) fn tiff_ycbcr_to_rgb_init(
    ycbcr: &mut TIFFYCbCrToRGB,
    luma: &[f32; 3],
    ref_black_white: &[f32; 6],
) -> i32 {
    let mut clamptab = vec![0u8; 4 * 256]; /* v < 0 => 0 */
    for i in 0..256 {
        clamptab[256 + i] = i as u8;
    }
    clamptab[512..].fill(255); /* v > 255 => 255 */
    ycbcr.clamptab = clamptab;
    ycbcr.Cr_r_tab = vec![0; 256];
    ycbcr.Cb_b_tab = vec![0; 256];
    ycbcr.Cr_g_tab = vec![0; 256];
    ycbcr.Cb_g_tab = vec![0; 256];
    ycbcr.Y_tab = vec![0; 256];

    let luma_red = luma[0];
    let luma_green = luma[1];
    let luma_blue = luma[2];
    {
        let f1 = 2.0 - 2.0 * luma_red;
        let d1 = fix(clamp_f(f1, 0.0f32, 2.0f32));
        let f2 = luma_red * f1 / luma_green;
        let d2 = -fix(clamp_f(f2, 0.0f32, 2.0f32));
        let f3 = 2.0 - 2.0 * luma_blue;
        let d3 = fix(clamp_f(f3, 0.0f32, 2.0f32));
        let f4 = luma_blue * f3 / luma_green;
        let d4 = -fix(clamp_f(f4, 0.0f32, 2.0f32));

        /*
         * i is the actual input pixel value in the range 0..255
         * Cb and Cr values are in the range -128..127 (actually
         * they are in a range defined by the ReferenceBlackWhite
         * tag) so there is some range shifting to do here when
         * constructing tables indexed by the raw pixel data.
         */
        for (i, x) in (0..256usize).zip(-128i32..) {
            let cr = c_f32_to_i32(clampw(
                code2v(
                    x,
                    ref_black_white[4] - 128.0f32,
                    ref_black_white[5] - 128.0f32,
                    127,
                ),
                -128.0f32 * 32.0,
                128.0f32 * 32.0,
            ));
            let cb = c_f32_to_i32(clampw(
                code2v(
                    x,
                    ref_black_white[2] - 128.0f32,
                    ref_black_white[3] - 128.0f32,
                    127,
                ),
                -128.0f32 * 32.0,
                128.0f32 * 32.0,
            ));

            ycbcr.Cr_r_tab[i] = d1.wrapping_mul(cr).wrapping_add(ONE_HALF) >> SHIFT;
            ycbcr.Cb_b_tab[i] = d3.wrapping_mul(cb).wrapping_add(ONE_HALF) >> SHIFT;
            ycbcr.Cr_g_tab[i] = d2.wrapping_mul(cr);
            ycbcr.Cb_g_tab[i] = d4.wrapping_mul(cb).wrapping_add(ONE_HALF);
            ycbcr.Y_tab[i] = c_f32_to_i32(clampw(
                code2v(x + 128, ref_black_white[0], ref_black_white[1], 255),
                -128.0f32 * 32.0,
                128.0f32 * 32.0,
            ));
        }
    }

    0
}
