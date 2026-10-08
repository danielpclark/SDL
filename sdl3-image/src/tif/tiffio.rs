// Rust translation of libtiff/tiffio.h from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1988-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! TIFF I/O Library Definitions: the parts the reading path uses (the
//! field descriptor, the colorimetry structures and their constants).

#![allow(dead_code)] // (definitions the reading path doesn't all use)

use std::borrow::Cow;

use super::tif_dir::TIFFSetGetFieldType;
use super::tiff::TIFFDataType;

/*
 * Flags to pass to TIFFPrintDirectory to control
 * printing of data structures that are potentially
 * very large.   Bit-or these flags to enable printing
 * multiple items.
 */

/*
 * Colour conversion stuff
 */

/* reference white */
pub(crate) const D65_X0: f32 = 95.0470;
pub(crate) const D65_Y0: f32 = 100.0;
pub(crate) const D65_Z0: f32 = 108.8827;

pub(crate) const D50_X0: f32 = 96.4250;
pub(crate) const D50_Y0: f32 = 100.0;
pub(crate) const D50_Z0: f32 = 82.4680;

/* Structure for holding information about a display device. */

/// `TIFFRGBValue`: 8-bit samples
pub(crate) type TIFFRGBValue = u8;

/// Translation of `TIFFDisplay`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TIFFDisplay {
    pub(crate) d_mat: [[f32; 3]; 3], /* XYZ -> luminance matrix */
    pub(crate) d_YCR: f32,           /* Light o/p for reference white */
    pub(crate) d_YCG: f32,
    pub(crate) d_YCB: f32,
    pub(crate) d_Vrwr: u32, /* Pixel values for ref. white */
    pub(crate) d_Vrwg: u32,
    pub(crate) d_Vrwb: u32,
    pub(crate) d_Y0R: f32, /* Residual light for black pixel */
    pub(crate) d_Y0G: f32,
    pub(crate) d_Y0B: f32,
    pub(crate) d_gammaR: f32, /* Gamma values for the three guns */
    pub(crate) d_gammaG: f32,
    pub(crate) d_gammaB: f32,
}

/// Translation of `TIFFYCbCrToRGB`: YCbCr->RGB support. The tables are
/// one allocation in the C; here they are vectors (`clamptab` holding the
/// 256 entries before the C's pointer too, so its index 256 is the C's 0).
#[derive(Clone, Debug, Default)]
pub(crate) struct TIFFYCbCrToRGB {
    /* range clamping table */
    pub(crate) clamptab: Vec<TIFFRGBValue>,
    pub(crate) Cr_r_tab: Vec<i32>,
    pub(crate) Cb_b_tab: Vec<i32>,
    pub(crate) Cr_g_tab: Vec<i32>,
    pub(crate) Cb_g_tab: Vec<i32>,
    pub(crate) Y_tab: Vec<i32>,
}

pub(crate) const CIELABTORGB_TABLE_RANGE: usize = 1500;

/// Translation of `TIFFCIELabToRGB`: CIE Lab 1976->RGB support
#[derive(Clone, Debug)]
pub(crate) struct TIFFCIELabToRGB {
    pub(crate) range: i32, /* Size of conversion table */
    pub(crate) rstep: f32,
    pub(crate) gstep: f32,
    pub(crate) bstep: f32,
    pub(crate) X0: f32, /* Reference white point */
    pub(crate) Y0: f32,
    pub(crate) Z0: f32,
    pub(crate) display: TIFFDisplay,
    pub(crate) Yr2r: [f32; CIELABTORGB_TABLE_RANGE + 1], /* Conversion of Yr to r */
    pub(crate) Yg2g: [f32; CIELABTORGB_TABLE_RANGE + 1], /* Conversion of Yg to g */
    pub(crate) Yb2b: [f32; CIELABTORGB_TABLE_RANGE + 1], /* Conversion of Yb to b */
}

impl Default for TIFFCIELabToRGB {
    fn default() -> Self {
        TIFFCIELabToRGB {
            range: 0,
            rstep: 0.0,
            gstep: 0.0,
            bstep: 0.0,
            X0: 0.0,
            Y0: 0.0,
            Z0: 0.0,
            display: TIFFDisplay::default(),
            Yr2r: [0.0; CIELABTORGB_TABLE_RANGE + 1],
            Yg2g: [0.0; CIELABTORGB_TABLE_RANGE + 1],
            Yb2b: [0.0; CIELABTORGB_TABLE_RANGE + 1],
        }
    }
}

/*
 * TIFF field descriptor: tag, counts, type, the setter/getter type, the
 * bit in the fieldsset bit vector, and the ASCII name.
 */
pub(crate) const TIFF_ANY: TIFFDataType = super::tiff::TIFF_NOTYPE; /* for field descriptor searching */
pub(crate) const TIFF_VARIABLE: i16 = -1; /* marker for variable length tags */
pub(crate) const TIFF_SPP: i16 = -2; /* marker for SamplesPerPixel tags */
pub(crate) const TIFF_VARIABLE2: i16 = -3; /* marker for uint32_t var-length tags */

pub(crate) const FIELD_CUSTOM: u16 = 65;

/// Translation of `TIFFField` (`struct _TIFFField`). The child IFD
/// definitions (`field_subfields`, for the EXIF and GPS directory
/// pointers) are left out: those directories are not read.
#[derive(Clone, Debug)]
pub(crate) struct TIFFField {
    pub(crate) field_tag: u32,           /* field's tag */
    pub(crate) field_readcount: i16,     /* read count/TIFF_VARIABLE/TIFF_SPP */
    pub(crate) field_writecount: i16,    /* write count/TIFF_VARIABLE */
    pub(crate) field_type: TIFFDataType, /* type of associated data */
    pub(crate) field_anonymous: u32,     /* if true, this is a unknown /
                                         anonymous tag */
    pub(crate) set_get_field_type: TIFFSetGetFieldType, /* type to be passed to
                                                        TIFFSetField, TIFFGetField */
    pub(crate) field_bit: u16,       /* bit in fieldsset bit vector */
    pub(crate) field_oktochange: u8, /* if true, can change while writing */
    pub(crate) field_passcount: u8,  /* if true, pass dir count on set */
    pub(crate) field_name: Cow<'static, str>, /* ASCII name */
}

/// A `TIFFField` initializer, in the C's field order (the child IFD
/// definitions, always `NULL` here, left out).
#[allow(clippy::too_many_arguments)]
pub(crate) const fn field(
    field_tag: u32,
    field_readcount: i16,
    field_writecount: i16,
    field_type: TIFFDataType,
    field_anonymous: u32,
    set_get_field_type: TIFFSetGetFieldType,
    field_bit: u16,
    field_oktochange: u8,
    field_passcount: u8,
    field_name: &'static str,
) -> TIFFField {
    TIFFField {
        field_tag,
        field_readcount,
        field_writecount,
        field_type,
        field_anonymous,
        set_get_field_type,
        field_bit,
        field_oktochange,
        field_passcount,
        field_name: Cow::Borrowed(field_name),
    }
}
