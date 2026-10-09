// Rust translation of lib/jxl/color_encoding_internal.h and
// lib/jxl/color_encoding_internal.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Metadata for color space conversions. (The conversions to and from the
//! API's `JxlColorEncoding` are not translated: SDL_image doesn't ask for
//! the color encoding.)

use super::base::{
    inv3x3_matrix, jxl_failure, mat_mul, pack_signed, unpack_signed, PaddedBytes, Status,
};
use super::color_management::maybe_create_profile;
use super::fields::{bits, bits_offset, bundle_init, visit_enum, Fields, Visitor};
use super::image_metadata::jxl_enum;

// (All CIE units are for the standard 1931 2 degree observer)

jxl_enum! {
    /// Color space the color pixel data is encoded in. The color pixel data is
    /// 3-channel in all cases except in case of kGray, where it uses only 1 channel.
    /// This also determines the amount of channels used in modular encoding.
    /// Translation of `ColorSpace`.
    pub(crate) enum ColorSpace {
        // Trichromatic color data. This also includes CMYK if a kBlack
        // ExtraChannelInfo is present. This implies, if there is an ICC profile, that
        // the ICC profile uses a 3-channel color space if no kBlack extra channel is
        // present, or uses color space 'CMYK' if a kBlack extra channel is present.
        Rgb = 0,
        // Single-channel data. This implies, if there is an ICC profile, that the ICC
        // profile also represents single-channel data and has the appropriate color
        // space ('GRAY').
        Gray = 1,
        // Like kRGB, but implies fixed values for primaries etc.
        Xyb = 2,
        // For non-RGB/gray data, e.g. from non-electro-optical sensors. Otherwise
        // the same conditions as kRGB apply.
        Unknown = 3,
    }
    bits: [Rgb, Gray, Xyb, Unknown]
}

jxl_enum! {
    /// Values from CICP ColourPrimaries. Translation of `WhitePoint`.
    pub(crate) enum WhitePoint {
        D65 = 1,     // sRGB/BT.709/Display P3/BT.2020
        Custom = 2,  // Actual values encoded in separate fields
        E = 10,      // XYZ
        Dci = 11,    // DCI-P3
    }
    bits: [D65, Custom, E, Dci]
}

jxl_enum! {
    /// Values from CICP ColourPrimaries. Translation of `Primaries`.
    pub(crate) enum Primaries {
        Srgb = 1,    // Same as BT.709
        Custom = 2,  // Actual values encoded in separate fields
        P2100 = 9,   // Same as BT.2020
        P3 = 11,
    }
    bits: [Srgb, Custom, P2100, P3]
}

jxl_enum! {
    /// Values from CICP TransferCharacteristics. Translation of
    /// `TransferFunction`.
    pub(crate) enum TransferFunction {
        T709 = 1,
        Unknown = 2,
        Linear = 8,
        Srgb = 13,
        Pq = 16,   // from BT.2100
        Dci = 17,  // from SMPTE RP 431-2 reference projector
        Hlg = 18,  // from BT.2100
    }
    bits: [T709, Linear, Srgb, Pq, Dci, Hlg, Unknown]
}

jxl_enum! {
    /// Translation of `RenderingIntent`.
    pub(crate) enum RenderingIntent {
        // Values match ICC sRGB encodings.
        Perceptual = 0,  // good for photos, requires a profile with LUT.
        Relative = 1,    // good for logos.
        Saturation = 2,  // perhaps useful for CG with fully saturated colors.
        Absolute = 3,    // leaves white point unchanged; good for proofing.
    }
    bits: [Perceptual, Relative, Saturation, Absolute]
}

/// Chromaticity (Y is omitted because it is 1 for primaries/white points).
/// Translation of `CIExy`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct CIExy {
    pub x: f64,
    pub y: f64,
}

/// Translation of `PrimariesCIExy`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct PrimariesCIExy {
    pub r: CIExy,
    pub g: CIExy,
    pub b: CIExy,
}

// Highest reasonable value for the gamma of a transfer curve.
const K_MAX_GAMMA: u32 = 8192;

// These strings are baked into Description - do not change.

fn color_space_to_string(color_space: ColorSpace) -> &'static str {
    match color_space {
        ColorSpace::Rgb => "RGB",
        ColorSpace::Gray => "Gra",
        ColorSpace::Xyb => "XYB",
        ColorSpace::Unknown => "CS?",
    }
}

fn white_point_to_string(white_point: WhitePoint) -> &'static str {
    match white_point {
        WhitePoint::D65 => "D65",
        WhitePoint::Custom => "Cst",
        WhitePoint::E => "EER",
        WhitePoint::Dci => "DCI",
    }
}

fn primaries_to_string(primaries: Primaries) -> &'static str {
    match primaries {
        Primaries::Srgb => "SRG",
        Primaries::P2100 => "202",
        Primaries::P3 => "DCI",
        Primaries::Custom => "Cst",
    }
}

fn transfer_function_to_string(transfer_function: TransferFunction) -> &'static str {
    match transfer_function {
        TransferFunction::Srgb => "SRG",
        TransferFunction::Linear => "Lin",
        TransferFunction::T709 => "709",
        TransferFunction::Pq => "PeQ",
        TransferFunction::Hlg => "HLG",
        TransferFunction::Dci => "DCI",
        TransferFunction::Unknown => "TF?",
    }
}

fn rendering_intent_to_string(rendering_intent: RenderingIntent) -> &'static str {
    match rendering_intent {
        RenderingIntent::Perceptual => "Per",
        RenderingIntent::Relative => "Rel",
        RenderingIntent::Saturation => "Sat",
        RenderingIntent::Absolute => "Abs",
    }
}

/// Translation of common.h's `ToString()` on a double (`"%g"`).
fn double_to_string(v: f64) -> String {
    if v == 0.0 {
        return "0".to_owned();
    }
    if !v.is_finite() {
        return format!("{v}");
    }
    let exp = v.abs().log10().floor() as i32;
    let p = 6;
    let s = if exp < -4 || exp >= p {
        let s = format!("{:.*e}", (p - 1) as usize, v);
        let (mantissa, e) = s.split_once('e').unwrap_or((&s, "0"));
        let mantissa = if mantissa.contains('.') {
            mantissa.trim_end_matches('0').trim_end_matches('.')
        } else {
            mantissa
        };
        let e: i32 = e.parse().unwrap_or(0);
        format!("{mantissa}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
    } else {
        let decimals = (p - 1 - exp).max(0) as usize;
        let s = format!("{:.*}", decimals, v);
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            s
        }
    };
    s
}

fn f64_from_customxy_i32(i: i32) -> f64 {
    i as f64 * 1E-6
}

/// Serializable form of CIExy. Translation of `Customxy`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Customxy {
    pub x: i32,
    pub y: i32,
}

impl Customxy {
    pub(crate) fn new() -> Self {
        let mut s = Customxy::default();
        bundle_init(&mut s);
        s
    }

    pub(crate) fn get(&self) -> CIExy {
        CIExy {
            x: f64_from_customxy_i32(self.x),
            y: f64_from_customxy_i32(self.y),
        }
    }
}

impl Fields for Customxy {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        let mut ux = pack_signed(self.x);
        visitor.u32d(
            bits(19),
            bits_offset(19, 524288),
            bits_offset(20, 1048576),
            bits_offset(21, 2097152),
            0,
            &mut ux,
        )?;
        self.x = unpack_signed(ux as usize) as i32;
        let mut uy = pack_signed(self.y);
        visitor.u32d(
            bits(19),
            bits_offset(19, 524288),
            bits_offset(20, 1048576),
            bits_offset(21, 2097152),
            0,
            &mut uy,
        )?;
        self.y = unpack_signed(uy as usize) as i32;
        Ok(())
    }
}

/// Translation of `CustomTransferFunction`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CustomTransferFunction {
    // Must be set before calling VisitFields!
    pub nonserialized_color_space: ColorSpace,

    have_gamma: bool,

    // OETF exponent to go from linear to gamma-compressed.
    gamma: u32, // Only used if have_gamma_.

    // Can be kUnknown.
    transfer_function: TransferFunction, // Only used if !have_gamma_.
}

impl CustomTransferFunction {
    const K_GAMMA_MUL: u32 = 10000000;

    pub(crate) fn new() -> Self {
        let mut s = CustomTransferFunction {
            nonserialized_color_space: ColorSpace::Rgb,
            have_gamma: false,
            gamma: 0,
            transfer_function: TransferFunction::Srgb,
        };
        bundle_init(&mut s);
        s
    }

    // Sets fields and returns true if nonserialized_color_space has an implicit
    // transfer function, otherwise leaves fields unchanged and returns false.
    pub(crate) fn set_implicit(&mut self) -> bool {
        if self.nonserialized_color_space == ColorSpace::Xyb {
            if self.set_gamma(1.0 / 3.0).is_err() {
                debug_assert!(false);
            }
            return true;
        }
        false
    }

    // Gamma: only used for PNG inputs
    pub(crate) fn is_gamma(&self) -> bool {
        self.have_gamma
    }
    pub(crate) fn get_gamma(&self) -> f64 {
        debug_assert!(self.is_gamma());
        self.gamma as f64 * 1E-7 // (0, 1)
    }

    pub(crate) fn set_gamma(&mut self, gamma: f64) -> Status {
        if gamma < (1.0f32 / K_MAX_GAMMA as f32) as f64 || gamma > 1.0 {
            return jxl_failure!("Invalid gamma {}", gamma);
        }

        self.have_gamma = false;
        if approx_eq(gamma, 1.0) {
            self.transfer_function = TransferFunction::Linear;
            return Ok(());
        }
        if approx_eq(gamma, 1.0 / 2.6) {
            self.transfer_function = TransferFunction::Dci;
            return Ok(());
        }
        // Don't translate 0.45.. to kSRGB nor k709 - that might change pixel
        // values because those curves also have a linear part.

        self.have_gamma = true;
        self.gamma = super::math::roundf((gamma * Self::K_GAMMA_MUL as f64) as f32) as u32;
        self.transfer_function = TransferFunction::Unknown;
        Ok(())
    }

    pub(crate) fn get_transfer_function(&self) -> TransferFunction {
        debug_assert!(!self.is_gamma());
        self.transfer_function
    }
    pub(crate) fn set_transfer_function(&mut self, tf: TransferFunction) {
        self.have_gamma = false;
        self.transfer_function = tf;
    }

    pub(crate) fn is_unknown(&self) -> bool {
        !self.have_gamma && (self.transfer_function == TransferFunction::Unknown)
    }
    pub(crate) fn is_srgb(&self) -> bool {
        !self.have_gamma && (self.transfer_function == TransferFunction::Srgb)
    }
    pub(crate) fn is_linear(&self) -> bool {
        !self.have_gamma && (self.transfer_function == TransferFunction::Linear)
    }
    pub(crate) fn is_pq(&self) -> bool {
        !self.have_gamma && (self.transfer_function == TransferFunction::Pq)
    }
    pub(crate) fn is_hlg(&self) -> bool {
        !self.have_gamma && (self.transfer_function == TransferFunction::Hlg)
    }
    pub(crate) fn is_709(&self) -> bool {
        !self.have_gamma && (self.transfer_function == TransferFunction::T709)
    }
    pub(crate) fn is_dci(&self) -> bool {
        !self.have_gamma && (self.transfer_function == TransferFunction::Dci)
    }
    pub(crate) fn is_same(&self, other: &CustomTransferFunction) -> bool {
        if self.have_gamma != other.have_gamma {
            return false;
        }
        if self.have_gamma {
            if self.gamma != other.gamma {
                return false;
            }
        } else if self.transfer_function != other.transfer_function {
            return false;
        }
        true
    }
}

impl Fields for CustomTransferFunction {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        let implicit = self.set_implicit();
        if visitor.conditional(!implicit) {
            visitor.bool_(false, &mut self.have_gamma)?;

            if visitor.conditional(self.have_gamma) {
                // Gamma is represented as a 24-bit int, the exponent used is
                // gamma_ / 1e7. Valid values are (0, 1]. On the low end side, we also
                // limit it to kMaxGamma/1e7.
                visitor.bits(24, Self::K_GAMMA_MUL, &mut self.gamma)?;
                if self.gamma > Self::K_GAMMA_MUL
                    || (self.gamma as u64) * (K_MAX_GAMMA as u64) < Self::K_GAMMA_MUL as u64
                {
                    return jxl_failure!("Invalid gamma {}", self.gamma);
                }
            }

            if visitor.conditional(!self.have_gamma) {
                visit_enum(visitor, TransferFunction::Srgb, &mut self.transfer_function)?;
            }
        }

        Ok(())
    }
}

/// Compact encoding of data required to interpret and translate pixels to a
/// known color space. Stored in Metadata. Thread-compatible. Translation of
/// `ColorEncoding`.
#[derive(Clone, Debug)]
pub(crate) struct ColorEncoding {
    pub all_default: bool,

    // Only valid if HaveFields()
    pub white_point: WhitePoint,
    pub primaries: Primaries, // Only valid if HasPrimaries()
    pub tf: CustomTransferFunction,
    pub rendering_intent: RenderingIntent,

    // If true, the codestream contains an ICC profile and we do not serialize
    // fields. Otherwise, fields are serialized and we create an ICC profile.
    want_icc: bool,

    // When false, fields such as white_point and tf are invalid and must not be
    // used. This occurs after setting a raw bytes-only ICC profile, only the
    // ICC bytes may be used. The color_space_ field is still valid.
    have_fields: bool,

    icc: PaddedBytes, // Valid ICC profile

    color_space: ColorSpace, // Can be kUnknown
    cmyk: bool,

    // Only used if white_point == kCustom.
    white: Customxy,

    // Only used if primaries == kCustom.
    red: Customxy,
    green: Customxy,
    blue: Customxy,
}

/// Returns whether the two inputs are approximately equal. Translation of
/// `ApproxEq()` (without skcms: `JPEGXL_ENABLE_SKCMS` is off).
pub(crate) fn approx_eq(a: f64, b: f64) -> bool {
    let max_l1 = 8E-5;
    // Threshold should be sufficient for ICC's 15-bit fixed-point numbers.
    // We have seen differences of 7.1E-5 with lcms2 and 1E-3 with skcms.
    (a - b).abs() <= max_l1
}

impl ColorEncoding {
    pub(crate) fn new() -> Self {
        let mut s = ColorEncoding {
            all_default: false,
            white_point: WhitePoint::D65,
            primaries: Primaries::Srgb,
            tf: CustomTransferFunction::new(),
            rendering_intent: RenderingIntent::Relative,
            want_icc: false,
            have_fields: true,
            icc: PaddedBytes::new(),
            color_space: ColorSpace::Rgb,
            cmyk: false,
            white: Customxy::new(),
            red: Customxy::new(),
            green: Customxy::new(),
            blue: Customxy::new(),
        };
        bundle_init(&mut s);
        s
    }

    /// Translation of the anonymous `CreateC2()` (one of its two).
    fn create_c2(pr: Primaries, tf: TransferFunction, is_gray: bool) -> ColorEncoding {
        let mut c = ColorEncoding::new();
        c.set_color_space(if is_gray {
            ColorSpace::Gray
        } else {
            ColorSpace::Rgb
        });
        c.white_point = WhitePoint::D65;
        c.primaries = pr;
        c.tf.set_transfer_function(tf);
        let ok = c.create_icc();
        debug_assert!(ok.is_ok());
        c
    }

    // Returns ready-to-use color encodings (initialized on-demand).
    // (Made on each call.)
    pub(crate) fn srgb(is_gray: bool) -> ColorEncoding {
        Self::create_c2(Primaries::Srgb, TransferFunction::Srgb, is_gray)
    }
    pub(crate) fn linear_srgb(is_gray: bool) -> ColorEncoding {
        Self::create_c2(Primaries::Srgb, TransferFunction::Linear, is_gray)
    }

    // Returns true if an ICC profile was successfully created from fields.
    // Must be called after modifying fields. Defined in color_management.cc.
    pub(crate) fn create_icc(&mut self) -> Status {
        self.internal_remove_icc();
        let mut icc = PaddedBytes::new();
        if maybe_create_profile(self, &mut icc).is_err() {
            return jxl_failure!("Failed to create profile from fields");
        }
        self.icc = icc;
        Ok(())
    }

    // Returns non-empty and valid ICC profile, unless:
    // - between calling InternalRemoveICC() and CreateICC() in tests;
    // - WantICC() == true and SetICC() was not yet called;
    // - after a failed call to SetSRGB(), SetICC(), or CreateICC().
    pub(crate) fn icc(&self) -> &PaddedBytes {
        &self.icc
    }

    // Internal only, do not call except from tests.
    pub(crate) fn internal_remove_icc(&mut self) {
        self.icc.clear();
    }

    // Sets the raw ICC profile bytes, without parsing the ICC, and without
    // updating the direct fields such as whitepoint, primaries and color
    // space. Functions to get and set fields, such as SetWhitePoint, cannot be
    // used anymore after this and functions such as IsSRGB return false no matter
    // what the contents of the icc profile.
    pub(crate) fn set_icc_raw(&mut self, icc: PaddedBytes) -> Status {
        if icc.is_empty() {
            return jxl_failure!("empty ICC");
        }
        self.icc = icc;

        self.want_icc = true;
        self.have_fields = false;
        Ok(())
    }

    // Returns whether to send the ICC profile in the codestream.
    pub(crate) fn want_icc(&self) -> bool {
        self.want_icc
    }

    // Return whether the direct fields are set, if false but ICC is set, only
    // raw ICC bytes are known.
    pub(crate) fn have_fields(&self) -> bool {
        self.have_fields
    }

    pub(crate) fn is_gray(&self) -> bool {
        self.color_space == ColorSpace::Gray
    }
    #[allow(dead_code)]
    pub(crate) fn is_cmyk(&self) -> bool {
        self.cmyk
    }
    pub(crate) fn channels(&self) -> usize {
        if self.is_gray() {
            1
        } else {
            3
        }
    }

    // Returns false if the field is invalid and unusable.
    pub(crate) fn has_primaries(&self) -> bool {
        !self.is_gray() && self.color_space != ColorSpace::Xyb
    }

    // Returns true after setting the field to a value defined by color_space,
    // otherwise false and leaves the field unchanged.
    pub(crate) fn implicit_white_point(&mut self) -> bool {
        if self.color_space == ColorSpace::Xyb {
            self.white_point = WhitePoint::D65;
            return true;
        }
        false
    }

    // Returns whether the color space is known to be sRGB. If a raw unparsed ICC
    // profile is set without the fields being set, this returns false, even if
    // the content of the ICC profile would match sRGB.
    pub(crate) fn is_srgb(&self) -> bool {
        if !self.have_fields {
            return false;
        }
        if !self.is_gray() && self.color_space != ColorSpace::Rgb {
            return false;
        }
        if self.white_point != WhitePoint::D65 {
            return false;
        }
        if self.primaries != Primaries::Srgb {
            return false;
        }
        if !self.tf.is_srgb() {
            return false;
        }
        true
    }

    // Returns whether the color space is known to be linear sRGB. If a raw unparsed ICC
    // profile is set without the fields being set, this returns false, even if
    // the content of the ICC profile would match linear sRGB.
    #[allow(dead_code)]
    pub(crate) fn is_linear_srgb(&self) -> bool {
        if !self.have_fields {
            return false;
        }
        if !self.is_gray() && self.color_space != ColorSpace::Rgb {
            return false;
        }
        if self.white_point != WhitePoint::D65 {
            return false;
        }
        if self.primaries != Primaries::Srgb {
            return false;
        }
        if !self.tf.is_linear() {
            return false;
        }
        true
    }

    // Accessors ensure tf.nonserialized_color_space is updated at the same time.
    pub(crate) fn get_color_space(&self) -> ColorSpace {
        self.color_space
    }
    pub(crate) fn set_color_space(&mut self, cs: ColorSpace) {
        self.color_space = cs;
        self.tf.nonserialized_color_space = cs;
    }

    pub(crate) fn get_white_point(&self) -> CIExy {
        debug_assert!(self.have_fields);
        match self.white_point {
            WhitePoint::Custom => self.white.get(),

            WhitePoint::D65 => CIExy {
                x: 0.3127,
                y: 0.3290,
            },

            WhitePoint::Dci => CIExy {
                // From https://ieeexplore.ieee.org/document/7290729 C.2 page 11
                x: 0.314,
                y: 0.351,
            },

            WhitePoint::E => CIExy {
                x: 1.0 / 3.0,
                y: 1.0 / 3.0,
            },
        }
    }

    pub(crate) fn get_primaries(&self) -> PrimariesCIExy {
        debug_assert!(self.have_fields);
        debug_assert!(self.has_primaries());
        match self.primaries {
            Primaries::Custom => PrimariesCIExy {
                r: self.red.get(),
                g: self.green.get(),
                b: self.blue.get(),
            },

            Primaries::Srgb => PrimariesCIExy {
                r: CIExy {
                    x: 0.639998686,
                    y: 0.330010138,
                },
                g: CIExy {
                    x: 0.300003784,
                    y: 0.600003357,
                },
                b: CIExy {
                    x: 0.150002046,
                    y: 0.059997204,
                },
            },

            Primaries::P2100 => PrimariesCIExy {
                r: CIExy { x: 0.708, y: 0.292 },
                g: CIExy { x: 0.170, y: 0.797 },
                b: CIExy { x: 0.131, y: 0.046 },
            },

            Primaries::P3 => PrimariesCIExy {
                r: CIExy { x: 0.680, y: 0.320 },
                g: CIExy { x: 0.265, y: 0.690 },
                b: CIExy { x: 0.150, y: 0.060 },
            },
        }
    }

    // Checks if the color spaces (including white point / primaries) are the
    // same, but ignores the transfer function, rendering intent and ICC bytes.
    pub(crate) fn same_color_space(&self, other: &ColorEncoding) -> bool {
        if self.color_space != other.color_space {
            return false;
        }

        if self.white_point != other.white_point {
            return false;
        }
        if self.white_point == WhitePoint::Custom
            && (self.white.x != other.white.x || self.white.y != other.white.y)
        {
            return false;
        }

        if self.has_primaries() != other.has_primaries() {
            return false;
        }
        if self.has_primaries() {
            if self.primaries != other.primaries {
                return false;
            }
            if self.primaries == Primaries::Custom {
                if self.red.x != other.red.x || self.red.y != other.red.y {
                    return false;
                }
                if self.green.x != other.green.x || self.green.y != other.green.y {
                    return false;
                }
                if self.blue.x != other.blue.x || self.blue.y != other.blue.y {
                    return false;
                }
            }
        }
        true
    }

    // Checks if the color space and transfer function are the same, ignoring
    // rendering intent and ICC bytes
    pub(crate) fn same_color_encoding(&self, other: &ColorEncoding) -> bool {
        self.same_color_space(other) && self.tf.is_same(&other.tf)
    }
}

impl Fields for ColorEncoding {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        if visitor.all_default(&mut self.all_default) {
            // Overwrite all serialized fields, but not any nonserialized_*.
            visitor.set_default(self);
            return Ok(());
        }

        visitor.bool_(false, &mut self.want_icc)?;

        // Always send even if want_icc_ because this affects decoding.
        // We can skip the white point/primaries because they do not.
        visit_enum(visitor, ColorSpace::Rgb, &mut self.color_space)?;

        if visitor.conditional(!self.want_icc()) {
            // Serialize enums. NOTE: we set the defaults to the most common values so
            // ImageMetadata.all_default is true in the common case.

            let implicit = self.implicit_white_point();
            if visitor.conditional(!implicit) {
                visit_enum(visitor, WhitePoint::D65, &mut self.white_point)?;
                if visitor.conditional(self.white_point == WhitePoint::Custom) {
                    visitor.visit_nested(&mut self.white)?;
                }
            }

            if visitor.conditional(self.has_primaries()) {
                visit_enum(visitor, Primaries::Srgb, &mut self.primaries)?;
                if visitor.conditional(self.primaries == Primaries::Custom) {
                    visitor.visit_nested(&mut self.red)?;
                    visitor.visit_nested(&mut self.green)?;
                    visitor.visit_nested(&mut self.blue)?;
                }
            }

            visitor.visit_nested(&mut self.tf)?;

            visit_enum(
                visitor,
                RenderingIntent::Relative,
                &mut self.rendering_intent,
            )?;

            // We didn't have ICC, so all fields should be known.
            if self.color_space == ColorSpace::Unknown || self.tf.is_unknown() {
                return jxl_failure!("No ICC but cs and tf unknown");
            }

            self.create_icc()?;
        }

        if self.want_icc() && visitor.is_reading() {
            // Haven't called SetICC() yet, do nothing.
        } else if self.icc().is_empty() {
            return jxl_failure!("Empty ICC");
        }

        Ok(())
    }
}

/// Returns a representation of the ColorEncoding fields (not icc).
/// Example description: "RGB_D65_SRG_Rel_Lin". Translation of
/// `Description()`.
pub(crate) fn description(c_in: &ColorEncoding) -> String {
    // Copy required for Implicit*
    let mut c = c_in.clone();

    let mut d = color_space_to_string(c.get_color_space()).to_owned();

    if !c.implicit_white_point() {
        d += "_";
        if c.white_point == WhitePoint::Custom {
            let wp = c.get_white_point();
            d += &(double_to_string(wp.x) + ";");
            d += &double_to_string(wp.y);
        } else {
            d += white_point_to_string(c.white_point);
        }
    }

    if c.has_primaries() {
        d += "_";
        if c.primaries == Primaries::Custom {
            let pr = c.get_primaries();
            d += &(double_to_string(pr.r.x) + ";");
            d += &(double_to_string(pr.r.y) + ";");
            d += &(double_to_string(pr.g.x) + ";");
            d += &(double_to_string(pr.g.y) + ";");
            d += &(double_to_string(pr.b.x) + ";");
            d += &double_to_string(pr.b.y);
        } else {
            d += primaries_to_string(c.primaries);
        }
    }

    d += "_";
    d += rendering_intent_to_string(c.rendering_intent);

    if !c.tf.set_implicit() {
        d += "_";
        if c.tf.is_gamma() {
            d += "g";
            d += &double_to_string(c.tf.get_gamma());
        } else {
            d += transfer_function_to_string(c.tf.get_transfer_function());
        }
    }

    d
}

/* Chromatic adaptation matrices*/
const K_BRADFORD: [f32; 9] = [
    0.8951, 0.2664, -0.1614, -0.7502, 1.7135, 0.0367, 0.0389, -0.0685, 1.0296,
];

const K_BRADFORD_INV: [f32; 9] = [
    0.9869929, -0.1470543, 0.1599627, 0.4323053, 0.5183603, 0.0492912, -0.0085287, 0.0400428,
    0.9684867,
];

/// Adapts whitepoint x, y to D50. Translation of `AdaptToXYZD50()`.
pub(crate) fn adapt_to_xyz_d50(wx: f32, wy: f32, matrix: &mut [f32; 9]) -> Status {
    if !(0.0..=1.0).contains(&wx) || wy <= 0.0 || wy > 1.0 {
        // Out of range values can cause division through zero
        // further down with the bradford adaptation too.
        return jxl_failure!("Invalid white point");
    }
    let w = [wx / wy, 1.0f32, (1.0f32 - wx - wy) / wy];
    // 1 / tiny float can still overflow
    if !(w[0].is_finite() && w[2].is_finite()) {
        return jxl_failure!("non-finite white point");
    }
    let w50 = [0.96422f32, 1.0f32, 0.82521f32];

    let mut lms = [0f32; 3];
    let mut lms50 = [0f32; 3];

    mat_mul(&K_BRADFORD, &w, 3, 3, 1, &mut lms);
    mat_mul(&K_BRADFORD, &w50, 3, 3, 1, &mut lms50);

    if lms[0] == 0.0 || lms[1] == 0.0 || lms[2] == 0.0 {
        return jxl_failure!("Invalid white point");
    }
    let a: [f32; 9] = [
        //       /----> 0, 1, 2, 3,          /----> 4, 5, 6, 7,          /----> 8,
        lms50[0] / lms[0],
        0.0,
        0.0,
        0.0,
        lms50[1] / lms[1],
        0.0,
        0.0,
        0.0,
        lms50[2] / lms[2],
    ];
    if !a[0].is_finite() || !a[4].is_finite() || !a[8].is_finite() {
        return jxl_failure!("Invalid white point");
    }

    let mut b = [0f32; 9];
    mat_mul(&a, &K_BRADFORD, 3, 3, 3, &mut b);
    mat_mul(&K_BRADFORD_INV, &b, 3, 3, 3, matrix);

    Ok(())
}

/// Translation of `PrimariesToXYZ()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn primaries_to_xyz(
    rx: f32,
    ry: f32,
    gx: f32,
    gy: f32,
    bx: f32,
    by: f32,
    wx: f32,
    wy: f32,
    matrix: &mut [f32; 9],
) -> Status {
    if !(0.0..=1.0).contains(&wx) || wy <= 0.0 || wy > 1.0 {
        return jxl_failure!("Invalid white point");
    }
    // TODO(lode): also require rx, ry, gx, gy, bx, to be in range 0-1? ICC
    // profiles in theory forbid negative XYZ values, but in practice the ACES P0
    // color space uses a negative y for the blue primary.
    let primaries: [f32; 9] = [
        rx,
        gx,
        bx,
        ry,
        gy,
        by,
        1.0f32 - rx - ry,
        1.0f32 - gx - gy,
        1.0f32 - bx - by,
    ];
    let mut primaries_inv = primaries;
    inv3x3_matrix(&mut primaries_inv)?;

    let w = [wx / wy, 1.0f32, (1.0f32 - wx - wy) / wy];
    // 1 / tiny float can still overflow
    if !(w[0].is_finite() && w[2].is_finite()) {
        return jxl_failure!("non-finite white point");
    }
    let mut xyz = [0f32; 3];
    mat_mul(&primaries_inv, &w, 3, 3, 1, &mut xyz);

    let a: [f32; 9] = [xyz[0], 0.0, 0.0, 0.0, xyz[1], 0.0, 0.0, 0.0, xyz[2]];

    mat_mul(&primaries, &a, 3, 3, 3, matrix);
    Ok(())
}

/// Translation of `PrimariesToXYZD50()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn primaries_to_xyz_d50(
    rx: f32,
    ry: f32,
    gx: f32,
    gy: f32,
    bx: f32,
    by: f32,
    wx: f32,
    wy: f32,
    matrix: &mut [f32; 9],
) -> Status {
    let mut to_xyz = [0f32; 9];
    primaries_to_xyz(rx, ry, gx, gy, bx, by, wx, wy, &mut to_xyz)?;
    let mut d50 = [0f32; 9];
    adapt_to_xyz_d50(wx, wy, &mut d50)?;

    mat_mul(&d50, &to_xyz, 3, 3, 3, matrix);
    Ok(())
}
