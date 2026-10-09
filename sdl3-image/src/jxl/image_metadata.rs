// Rust translation of lib/jxl/image_metadata.h and lib/jxl/image_metadata.cc
// from libjxl (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Main codestream header bundles, the metadata that applies to all frames.
//! Enums must align with the C API definitions in codestream_header.h.
//! (The debug strings are not translated.)

use super::base::{jxl_failure, Status, K_DEFAULT_INTENSITY_TARGET};
use super::color_encoding_internal::ColorEncoding;
use super::fields::{bits, bits_offset, bundle_init, val, visit_enum, Fields, Visitor};
use super::headers::{AnimationHeader, PreviewHeader, SizeHeader};
use super::opsin_params::{
    K_DEFAULT_INVERSE_OPSIN_ABSORBANCE_MATRIX, K_NEG_OPSIN_ABSORBANCE_BIAS_RGB,
};
use super::quantizer::K_DEFAULT_QUANT_BIAS;

/// EXIF orientation of the image. This field overrides any field present in
/// actual EXIF metadata. The value tells which transformation the decoder
/// must apply after decoding to display the image with the correct
/// orientation. Translation of `Orientation`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub(crate) enum Orientation {
    // Values 1..8 match the EXIF definitions.
    Identity = 1,
    FlipHorizontal = 2,
    Rotate180 = 3,
    FlipVertical = 4,
    Transpose = 5,
    Rotate90 = 6,
    AntiTranspose = 7,
    Rotate270 = 8,
}
// Don't need an EnumBits because Orientation is not read via Enum().

impl Orientation {
    fn from_u32(v: u32) -> Orientation {
        match v {
            2 => Orientation::FlipHorizontal,
            3 => Orientation::Rotate180,
            4 => Orientation::FlipVertical,
            5 => Orientation::Transpose,
            6 => Orientation::Rotate90,
            7 => Orientation::AntiTranspose,
            8 => Orientation::Rotate270,
            _ => Orientation::Identity,
        }
    }
}

/// Defines a field enum: the variants with their values, and the
/// `EnumName()`/`EnumBits()` of the `Visitor::Enum()` fields.
macro_rules! jxl_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident { $($(#[$vmeta:meta])* $variant:ident = $value:expr,)* }
        bits: [$($bit:ident),*]
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        #[repr(u32)]
        $vis enum $name { $($(#[$vmeta])* $variant = $value,)* }

        impl $crate::jxl::fields::JxlEnum for $name {
            const NAME: &'static str = stringify!($name);
            fn enum_bits() -> u64 {
                0 $(| $crate::jxl::fields::make_bit($name::$bit as u32))*
            }
            fn to_u32(self) -> u32 {
                self as u32
            }
            fn from_u32(v: u32) -> Self {
                match v {
                    $(x if x == $value => $name::$variant,)*
                    // (EnumValid() checked the value)
                    _ => $crate::jxl::image_metadata::unreachable_enum(),
                }
            }
        }
    };
}
pub(crate) use jxl_enum;

/// The value of an enum read through `Visitor::Enum()` after
/// `EnumValid()` accepted it: unreachable.
#[cold]
pub(crate) fn unreachable_enum<T>() -> T {
    unreachable!("EnumValid() accepted the value")
}

jxl_enum! {
    /// Translation of `ExtraChannel`.
    pub(crate) enum ExtraChannel {
        // First two enumerators (most common) are cheaper to encode
        Alpha = 0,
        Depth = 1,

        SpotColor = 2,
        SelectionMask = 3,
        Black = 4, // for CMYK
        Cfa = 5,   // Bayer channel
        Thermal = 6,
        Reserved0 = 7,
        Reserved1 = 8,
        Reserved2 = 9,
        Reserved3 = 10,
        Reserved4 = 11,
        Reserved5 = 12,
        Reserved6 = 13,
        Reserved7 = 14,
        // disambiguated via name string, raise warning if unsupported
        Unknown = 15,
        // like kUnknown but can silently be ignored
        Optional = 16,
    }
    bits: [Alpha, Depth, SpotColor, SelectionMask, Black, Cfa, Thermal, Unknown, Optional]
}

/// Used in ImageMetadata and ExtraChannelInfo. Translation of `BitDepth`.
#[derive(Clone, Default, Debug)]
pub(crate) struct BitDepth {
    // Whether the original (uncompressed) samples are floating point or
    // unsigned integer.
    pub floating_point_sample: bool,

    // Bit depth of the original (uncompressed) image samples. Must be in the
    // range [1, 32].
    pub bits_per_sample: u32,

    // Floating point exponent bits of the original (uncompressed) image samples,
    // only used if floating_point_sample is true.
    // If used, the samples are floating point with:
    // - 1 sign bit
    // - exponent_bits_per_sample exponent bits
    // - (bits_per_sample - exponent_bits_per_sample - 1) mantissa bits
    // If used, exponent_bits_per_sample must be in the range
    // [2, 8] and amount of mantissa bits must be in the range [2, 23].
    // NOTE: exponent_bits_per_sample is 8 for single precision binary32
    // point, 5 for half precision binary16, 7 for fp24.
    pub exponent_bits_per_sample: u32,
}

impl BitDepth {
    pub(crate) fn new() -> Self {
        let mut s = BitDepth::default();
        bundle_init(&mut s);
        s
    }
}

impl Fields for BitDepth {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        visitor.bool_(false, &mut self.floating_point_sample)?;
        // The same fields (bits_per_sample and exponent_bits_per_sample) are read
        // in a different way depending on floating_point_sample's value. It's still
        // default-initialized correctly so using visitor->Conditional is not
        // required.
        if !self.floating_point_sample {
            visitor.u32d(
                val(8),
                val(10),
                val(12),
                bits_offset(6, 1),
                8,
                &mut self.bits_per_sample,
            )?;
            self.exponent_bits_per_sample = 0;
        } else {
            visitor.u32d(
                val(32),
                val(16),
                val(24),
                bits_offset(6, 1),
                32,
                &mut self.bits_per_sample,
            )?;
            // The encoded value is exponent_bits_per_sample - 1, encoded in 3 bits
            // so the value can be in range [1, 8].
            let offset: u32 = 1;
            self.exponent_bits_per_sample = self.exponent_bits_per_sample.wrapping_sub(offset);
            visitor.bits(4, 8 - offset, &mut self.exponent_bits_per_sample)?;
            self.exponent_bits_per_sample = self.exponent_bits_per_sample.wrapping_add(offset);
        }

        // Error-checking for floating point ranges.
        if self.floating_point_sample {
            if self.exponent_bits_per_sample < 2 || self.exponent_bits_per_sample > 8 {
                return jxl_failure!(
                    "Invalid exponent_bits_per_sample: {}",
                    self.exponent_bits_per_sample
                );
            }
            let mantissa_bits =
                self.bits_per_sample as i32 - self.exponent_bits_per_sample as i32 - 1;
            if !(2..=23).contains(&mantissa_bits) {
                return jxl_failure!("Invalid bits_per_sample: {}", self.bits_per_sample);
            }
        } else if self.bits_per_sample > 31 {
            return jxl_failure!("Invalid bits_per_sample: {}", self.bits_per_sample);
        }
        Ok(())
    }
}

/// Also used by extra channel names. Translation of `VisitNameString()`
/// (from frame_header.h; the names are bytes).
pub(crate) fn visit_name_string(visitor: &mut dyn Visitor, name: &mut Vec<u8>) -> Status {
    let mut name_length = name.len() as u32;
    // Allows layer name lengths up to 1071 bytes
    visitor.u32d(
        val(0),
        bits(4),
        bits_offset(5, 16),
        bits_offset(10, 48),
        0,
        &mut name_length,
    )?;
    if visitor.is_reading() {
        name.resize(name_length as usize, 0);
    }
    for i in 0..name_length as usize {
        let mut c = name[i] as u32;
        visitor.bits(8, 0, &mut c)?;
        name[i] = c as u8;
    }
    Ok(())
}

/// Describes one extra channel. Translation of `ExtraChannelInfo`.
#[derive(Clone, Debug)]
pub(crate) struct ExtraChannelInfo {
    pub all_default: bool,

    pub type_: ExtraChannel,
    pub bit_depth: BitDepth,
    pub dim_shift: u32, // downsampled by 2^dim_shift on each axis

    pub name: Vec<u8>, // UTF-8

    // Conditional:
    pub alpha_associated: bool, // i.e. premultiplied
    pub spot_color: [f32; 4],   // spot color in linear RGBA
    pub cfa_channel: u32,
}

impl ExtraChannelInfo {
    pub(crate) fn new() -> Self {
        let mut s = ExtraChannelInfo {
            all_default: false,
            type_: ExtraChannel::Alpha,
            bit_depth: BitDepth::new(),
            dim_shift: 0,
            name: Vec::new(),
            alpha_associated: false,
            spot_color: [0.0; 4],
            cfa_channel: 0,
        };
        bundle_init(&mut s);
        s
    }
}

impl Fields for ExtraChannelInfo {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        if visitor.all_default(&mut self.all_default) {
            // Overwrite all serialized fields, but not any nonserialized_*.
            visitor.set_default(self);
            return Ok(());
        }

        // General
        visit_enum(visitor, ExtraChannel::Alpha, &mut self.type_)?;

        visitor.visit_nested(&mut self.bit_depth)?;

        visitor.u32d(
            val(0),
            val(3),
            val(4),
            bits_offset(3, 1),
            0,
            &mut self.dim_shift,
        )?;
        if self.dim_shift >= 32 || (1u32 << self.dim_shift) > 8 {
            return jxl_failure!("dim_shift {} too large", self.dim_shift);
        }

        visit_name_string(visitor, &mut self.name)?;

        // Conditional
        if visitor.conditional(self.type_ == ExtraChannel::Alpha) {
            visitor.bool_(false, &mut self.alpha_associated)?;
        }
        if visitor.conditional(self.type_ == ExtraChannel::SpotColor) {
            for c in self.spot_color.iter_mut() {
                visitor.f16(0.0, c)?;
            }
        }
        if visitor.conditional(self.type_ == ExtraChannel::Cfa) {
            visitor.u32d(
                val(1),
                bits(2),
                bits_offset(4, 3),
                bits_offset(8, 19),
                1,
                &mut self.cfa_channel,
            )?;
        }

        if self.type_ == ExtraChannel::Unknown
            || (ExtraChannel::Reserved0 as u32 <= self.type_ as u32
                && self.type_ as u32 <= ExtraChannel::Reserved7 as u32)
        {
            return jxl_failure!(
                "Unknown extra channel (bits {}, shift {}, name '{:?}')\n",
                self.bit_depth.bits_per_sample,
                self.dim_shift,
                self.name
            );
        }
        Ok(())
    }
}

/// Translation of `OpsinInverseMatrix`.
#[derive(Clone, Default, Debug)]
pub(crate) struct OpsinInverseMatrix {
    pub all_default: bool,

    pub inverse_matrix: [f32; 9],
    pub opsin_biases: [f32; 3],
    pub quant_biases: [f32; 4],
}

impl OpsinInverseMatrix {
    pub(crate) fn new() -> Self {
        let mut s = OpsinInverseMatrix::default();
        bundle_init(&mut s);
        s
    }
}

impl Fields for OpsinInverseMatrix {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        if visitor.all_default(&mut self.all_default) {
            // Overwrite all serialized fields, but not any nonserialized_*.
            visitor.set_default(self);
            return Ok(());
        }
        for i in 0..9 {
            visitor.f16(
                K_DEFAULT_INVERSE_OPSIN_ABSORBANCE_MATRIX[i],
                &mut self.inverse_matrix[i],
            )?;
        }
        for i in 0..3 {
            visitor.f16(
                K_NEG_OPSIN_ABSORBANCE_BIAS_RGB[i],
                &mut self.opsin_biases[i],
            )?;
        }
        for i in 0..4 {
            visitor.f16(K_DEFAULT_QUANT_BIAS[i], &mut self.quant_biases[i])?;
        }
        Ok(())
    }
}

/// Information useful for mapping HDR images to lower dynamic range
/// displays. Translation of `ToneMapping`.
#[derive(Clone, Default, Debug)]
pub(crate) struct ToneMapping {
    pub all_default: bool,

    // Upper bound on the intensity level present in the image. For unsigned
    // integer pixel encodings, this is the brightness of the largest
    // representable value. The image does not necessarily contain a pixel
    // actually this bright. An encoder is allowed to set 255 for SDR images
    // without computing a histogram.
    pub intensity_target: f32, // [nits]

    // Lower bound on the intensity level present in the image. This may be
    // loose, i.e. lower than the actual darkest pixel. When tone mapping, a
    // decoder will map [min_nits, intensity_target] to the display range.
    pub min_nits: f32,

    pub relative_to_max_display: bool, // see below
    // The tone mapping will leave unchanged (linear mapping) any pixels whose
    // brightness is strictly below this. The interpretation depends on
    // relative_to_max_display. If true, this is a ratio [0, 1] of the maximum
    // display brightness [nits], otherwise an absolute brightness [nits].
    pub linear_below: f32,
}

impl ToneMapping {
    pub(crate) fn new() -> Self {
        let mut s = ToneMapping::default();
        bundle_init(&mut s);
        s
    }
}

impl Fields for ToneMapping {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        if visitor.all_default(&mut self.all_default) {
            // Overwrite all serialized fields, but not any nonserialized_*.
            visitor.set_default(self);
            return Ok(());
        }

        visitor.f16(K_DEFAULT_INTENSITY_TARGET, &mut self.intensity_target)?;
        if self.intensity_target <= 0.0 {
            return jxl_failure!("invalid intensity target");
        }

        visitor.f16(0.0, &mut self.min_nits)?;
        if self.min_nits < 0.0 || self.min_nits > self.intensity_target {
            return jxl_failure!(
                "invalid min {} vs max {}",
                self.min_nits,
                self.intensity_target
            );
        }

        visitor.bool_(false, &mut self.relative_to_max_display)?;

        visitor.f16(0.0, &mut self.linear_below)?;
        if self.linear_below < 0.0 || (self.relative_to_max_display && self.linear_below > 1.0) {
            return jxl_failure!("invalid linear_below {}", self.linear_below);
        }

        Ok(())
    }
}

/// Contains weights to customize some trasnforms - in particular, XYB and
/// upsampling. Translation of `CustomTransformData`.
#[derive(Clone, Debug)]
pub(crate) struct CustomTransformData {
    // Must be set before calling VisitFields. Must equal xyb_encoded of
    // ImageMetadata, should be set by ImageMetadata during VisitFields.
    pub nonserialized_xyb_encoded: bool,

    pub all_default: bool,

    pub opsin_inverse_matrix: OpsinInverseMatrix,

    pub custom_weights_mask: u32,
    pub upsampling2_weights: [f32; 15],
    pub upsampling4_weights: [f32; 55],
    pub upsampling8_weights: [f32; 210],
}

impl CustomTransformData {
    pub(crate) fn new() -> Self {
        let mut s = CustomTransformData {
            nonserialized_xyb_encoded: false,
            all_default: false,
            opsin_inverse_matrix: OpsinInverseMatrix::new(),
            custom_weights_mask: 0,
            upsampling2_weights: [0.0; 15],
            upsampling4_weights: [0.0; 55],
            upsampling8_weights: [0.0; 210],
        };
        bundle_init(&mut s);
        s
    }
}

// 4 5x5 kernels, but all of them can be obtained by symmetry from one,
// which is symmetric along its main diagonal. The top-left kernel is
// defined by
//
// 0  1  2  3  4
// 1  5  6  7  8
// 2  6  9 10 11
// 3  7 10 12 13
// 4  8 11 13 14
const K_WEIGHTS2: [f32; 15] = [
    -0.01716200,
    -0.03452303,
    -0.04022174,
    -0.02921014,
    -0.00624645,
    0.14111091,
    0.28896755,
    0.00278718,
    -0.01610267,
    0.56661550,
    0.03777607,
    -0.01986694,
    -0.03144731,
    -0.01185068,
    -0.00213539,
];

// 16 5x5 kernels, but all of them can be obtained by symmetry from
// three, two of which are symmetric along their main diagonals. The top
// left 4 kernels are defined by
//
// 0  1  2  3  4   5  6  7  8  9
// 1 10 11 12 13  14 15 16 17 18
// 2 11 19 20 21  22 23 24 25 26
// 3 12 20 27 28  29 30 31 32 33
// 4 13 21 28 34  35 36 37 38 39
//
// 5 14 22 29 35  40 41 42 43 44
// 6 15 23 30 36  41 45 46 47 48
// 7 16 24 31 37  42 46 49 50 51
// 8 17 25 32 38  43 47 50 52 53
// 9 18 26 33 39  44 48 51 53 54
const K_WEIGHTS4: [f32; 55] = [
    -0.02419067,
    -0.03491987,
    -0.03693351,
    -0.03094285,
    -0.00529785,
    -0.01663432,
    -0.03556863,
    -0.03888905,
    -0.03516850,
    -0.00989469,
    0.23651958,
    0.33392945,
    -0.01073543,
    -0.01313181,
    -0.03556694,
    0.13048175,
    0.40103025,
    0.03951150,
    -0.02077584,
    0.46914198,
    -0.00209270,
    -0.01484589,
    -0.04064806,
    0.18942530,
    0.56279892,
    0.06674400,
    -0.02335494,
    -0.03551682,
    -0.00754830,
    -0.02267919,
    -0.02363578,
    0.00315804,
    -0.03399098,
    -0.01359519,
    -0.00091653,
    -0.00335467,
    -0.01163294,
    -0.01610294,
    -0.00974088,
    -0.00191622,
    -0.01095446,
    -0.03198464,
    -0.04455121,
    -0.02799790,
    -0.00645912,
    0.06390599,
    0.22963888,
    0.00630981,
    -0.01897349,
    0.67537268,
    0.08483369,
    -0.02534994,
    -0.02205197,
    -0.01667999,
    -0.00384443,
];

// 64 5x5 kernels, all of them can be obtained by symmetry from
// 10, 4 of which are symmetric along their main diagonals. The top
// left 16 kernels are defined by
//  0  1  2  3  4   5  6  7  8  9   a  b  c  d  e   f 10 11 12 13
//  1 14 15 16 17  18 19 1a 1b 1c  1d 1e 1f 20 21  22 23 24 25 26
//  2 15 27 28 29  2a 2b 2c 2d 2e  2f 30 31 32 33  34 35 36 37 38
//  3 16 28 39 3a  3b 3c 3d 3e 3f  40 41 42 43 44  45 46 47 48 49
//  4 17 29 3a 4a  4b 4c 4d 4e 4f  50 51 52 53 54  55 56 57 58 59

//  5 18 2a 3b 4b  5a 5b 5c 5d 5e  5f 60 61 62 63  64 65 66 67 68
//  6 19 2b 3c 4c  5b 69 6a 6b 6c  6d 6e 6f 70 71  72 73 74 75 76
//  7 1a 2c 3d 4d  5c 6a 77 78 79  7a 7b 7c 7d 7e  7f 80 81 82 83
//  8 1b 2d 3e 4e  5d 6b 78 84 85  86 87 88 89 8a  8b 8c 8d 8e 8f
//  9 1c 2e 3f 4f  5e 6c 79 85 90  91 92 93 94 95  96 97 98 99 9a

//  a 1d 2f 40 50  5f 6d 7a 86 91  9b 9c 9d 9e 9f  a0 a1 a2 a3 a4
//  b 1e 30 41 51  60 6e 7b 87 92  9c a5 a6 a7 a8  a9 aa ab ac ad
//  c 1f 31 42 52  61 6f 7c 88 93  9d a6 ae af b0  b1 b2 b3 b4 b5
//  d 20 32 43 53  62 70 7d 89 94  9e a7 af b6 b7  b8 b9 ba bb bc
//  e 21 33 44 54  63 71 7e 8a 95  9f a8 b0 b7 bd  be bf c0 c1 c2

//  f 22 34 45 55  64 72 7f 8b 96  a0 a9 b1 b8 be  c3 c4 c5 c6 c7
// 10 23 35 46 56  65 73 80 8c 97  a1 aa b2 b9 bf  c4 c8 c9 ca cb
// 11 24 36 47 57  66 74 81 8d 98  a2 ab b3 ba c0  c5 c9 cc cd ce
// 12 25 37 48 58  67 75 82 8e 99  a3 ac b4 bb c1  c6 ca cd cf d0
// 13 26 38 49 59  68 76 83 8f 9a  a4 ad b5 bc c2  c7 cb ce d0 d1
const K_WEIGHTS8: [f32; 210] = [
    -0.02928613,
    -0.03706353,
    -0.03783812,
    -0.03324558,
    -0.00447632,
    -0.02519406,
    -0.03752601,
    -0.03901508,
    -0.03663285,
    -0.00646649,
    -0.02066407,
    -0.03838633,
    -0.04002101,
    -0.03900035,
    -0.00901973,
    -0.01626393,
    -0.03954148,
    -0.04046620,
    -0.03979621,
    -0.01224485,
    0.29895328,
    0.35757708,
    -0.02447552,
    -0.01081748,
    -0.04314594,
    0.23903219,
    0.41119301,
    -0.00573046,
    -0.01450239,
    -0.04246845,
    0.17567618,
    0.45220643,
    0.02287757,
    -0.01936783,
    -0.03583255,
    0.11572472,
    0.47416733,
    0.06284440,
    -0.02685066,
    0.42720050,
    -0.02248939,
    -0.01155273,
    -0.04562755,
    0.28689496,
    0.49093869,
    -0.00007891,
    -0.01545926,
    -0.04562659,
    0.21238920,
    0.53980934,
    0.03369474,
    -0.02070211,
    -0.03866988,
    0.14229550,
    0.56593398,
    0.08045181,
    -0.02888298,
    -0.03680918,
    -0.00542229,
    -0.02920477,
    -0.02788574,
    -0.02118180,
    -0.03942402,
    -0.00775547,
    -0.02433614,
    -0.03193943,
    -0.02030828,
    -0.04044014,
    -0.01074016,
    -0.01930822,
    -0.03620399,
    -0.01974125,
    -0.03919545,
    -0.01456093,
    -0.00045072,
    -0.00360110,
    -0.01020207,
    -0.01231907,
    -0.00638988,
    -0.00071592,
    -0.00279122,
    -0.00957115,
    -0.01288327,
    -0.00730937,
    -0.00107783,
    -0.00210156,
    -0.00890705,
    -0.01317668,
    -0.00813895,
    -0.00153491,
    -0.02128481,
    -0.04173044,
    -0.04831487,
    -0.03293190,
    -0.00525260,
    -0.01720322,
    -0.04052736,
    -0.05045706,
    -0.03607317,
    -0.00738030,
    -0.01341764,
    -0.03965629,
    -0.05151616,
    -0.03814886,
    -0.01005819,
    0.18968273,
    0.33063684,
    -0.01300105,
    -0.01372950,
    -0.04017465,
    0.13727832,
    0.36402234,
    0.01027890,
    -0.01832107,
    -0.03365072,
    0.08734506,
    0.38194295,
    0.04338228,
    -0.02525993,
    0.56408126,
    0.00458352,
    -0.01648227,
    -0.04887868,
    0.24585519,
    0.62026135,
    0.04314807,
    -0.02213737,
    -0.04158014,
    0.16637289,
    0.65027023,
    0.09621636,
    -0.03101388,
    -0.04082742,
    -0.00904519,
    -0.02790922,
    -0.02117818,
    0.00798662,
    -0.03995711,
    -0.01243427,
    -0.02231705,
    -0.02946266,
    0.00992055,
    -0.03600283,
    -0.01684920,
    -0.00111684,
    -0.00411204,
    -0.01297130,
    -0.01723725,
    -0.01022545,
    -0.00165306,
    -0.00313110,
    -0.01218016,
    -0.01763266,
    -0.01125620,
    -0.00231663,
    -0.01374149,
    -0.03797620,
    -0.05142937,
    -0.03117307,
    -0.00581914,
    -0.01064003,
    -0.03608089,
    -0.05272168,
    -0.03375670,
    -0.00795586,
    0.09628104,
    0.27129991,
    -0.00353779,
    -0.01734151,
    -0.03153981,
    0.05686230,
    0.28500998,
    0.02230594,
    -0.02374955,
    0.68214326,
    0.05018048,
    -0.02320852,
    -0.04383616,
    0.18459474,
    0.71517975,
    0.10805613,
    -0.03263677,
    -0.03637639,
    -0.01394373,
    -0.02511203,
    -0.01728636,
    0.05407331,
    -0.02867568,
    -0.01893131,
    -0.00240854,
    -0.00446511,
    -0.01636187,
    -0.02377053,
    -0.01522848,
    -0.00333334,
    -0.00819975,
    -0.02964169,
    -0.04499287,
    -0.02745350,
    -0.00612408,
    0.02727416,
    0.19446600,
    0.00159832,
    -0.02232473,
    0.74982506,
    0.11452620,
    -0.03348048,
    -0.01605681,
    -0.02070339,
    -0.00458223,
];

impl Fields for CustomTransformData {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        if visitor.all_default(&mut self.all_default) {
            // Overwrite all serialized fields, but not any nonserialized_*.
            visitor.set_default(self);
            return Ok(());
        }
        if visitor.conditional(self.nonserialized_xyb_encoded) {
            visitor.visit_nested(&mut self.opsin_inverse_matrix)?;
        }
        visitor.bits(3, 0, &mut self.custom_weights_mask)?;
        if visitor.conditional((self.custom_weights_mask & 0x1) != 0) {
            for i in 0..15 {
                visitor.f16(K_WEIGHTS2[i], &mut self.upsampling2_weights[i])?;
            }
        }
        if visitor.conditional((self.custom_weights_mask & 0x2) != 0) {
            for i in 0..55 {
                visitor.f16(K_WEIGHTS4[i], &mut self.upsampling4_weights[i])?;
            }
        }
        if visitor.conditional((self.custom_weights_mask & 0x4) != 0) {
            for i in 0..210 {
                visitor.f16(K_WEIGHTS8[i], &mut self.upsampling8_weights[i])?;
            }
        }
        Ok(())
    }
}

/// Properties of the original image bundle. This enables Encode(Decode())
/// to re-create an equivalent image without user input. Translation of
/// `ImageMetadata`.
#[derive(Clone, Debug)]
pub(crate) struct ImageMetadata {
    pub all_default: bool,

    pub bit_depth: BitDepth,
    pub modular_16_bit_buffer_sufficient: bool, // otherwise 32 is.

    // Whether the colors values of the pixels of frames are encoded in the
    // codestream using the absolute XYB color space, or the using values that
    // follow the color space defined by the ColorEncoding or ICC profile.
    // (See upstream's image_metadata.h for the full discussion.)
    pub xyb_encoded: bool,

    pub color_encoding: ColorEncoding,

    // These values are initialized to defaults such that the 'extra_fields'
    // condition in VisitFields uses correctly initialized values.
    pub orientation: u32,
    pub have_preview: bool,
    pub have_animation: bool,
    pub have_intrinsic_size: bool,

    // If present, the stored image has the dimensions of the first SizeHeader,
    // but decoders are advised to resample or display per `intrinsic_size`.
    pub intrinsic_size: SizeHeader, // only if have_intrinsic_size

    pub tone_mapping: ToneMapping,

    // When reading: deserialized. When writing: automatically set from vector.
    pub num_extra_channels: u32,
    pub extra_channel_info: Vec<ExtraChannelInfo>,

    // Only present if m.have_preview.
    pub preview_size: PreviewHeader,
    // Only present if m.have_animation.
    pub animation: AnimationHeader,

    pub extensions: u64,

    // Option to stop parsing after basic info, and treat as if the later
    // fields do not participate. Use to parse only basic image information
    // excluding the final larger or variable sized data.
    pub nonserialized_only_parse_basic_info: bool,
}

impl ImageMetadata {
    pub(crate) fn new() -> Self {
        let mut s = ImageMetadata {
            all_default: false,
            bit_depth: BitDepth::new(),
            modular_16_bit_buffer_sufficient: false,
            xyb_encoded: false,
            color_encoding: ColorEncoding::new(),
            orientation: 1,
            have_preview: false,
            have_animation: false,
            have_intrinsic_size: false,
            intrinsic_size: SizeHeader::new(),
            tone_mapping: ToneMapping::new(),
            num_extra_channels: 0,
            extra_channel_info: Vec::new(),
            preview_size: PreviewHeader::new(),
            animation: AnimationHeader::new(),
            extensions: 0,
            nonserialized_only_parse_basic_info: false,
        };
        bundle_init(&mut s);
        s
    }

    /// Returns bit depth of the JPEG XL compressed alpha channel, or 0 if no
    /// alpha channel present. In the theoretical case that there are
    /// multiple alpha channels, returns the bit depht of the first.
    /// Translation of `GetAlphaBits()`.
    pub(crate) fn get_alpha_bits(&self) -> u32 {
        match self.find(ExtraChannel::Alpha) {
            None => 0,
            Some(alpha) => alpha.bit_depth.bits_per_sample,
        }
    }

    /// Translation of `HasAlpha()`.
    pub(crate) fn has_alpha(&self) -> bool {
        self.get_alpha_bits() != 0
    }

    /// Translation of `IntensityTarget()`.
    pub(crate) fn intensity_target(&self) -> f32 {
        debug_assert!(self.tone_mapping.intensity_target != 0.0);
        self.tone_mapping.intensity_target
    }

    /// Returns first ExtraChannelInfo of the given type, or nullptr if none.
    /// Translation of `Find()`.
    pub(crate) fn find(&self, type_: ExtraChannel) -> Option<&ExtraChannelInfo> {
        self.extra_channel_info
            .iter()
            .find(|eci| eci.type_ == type_)
    }

    /// Translation of `GetOrientation()`.
    pub(crate) fn get_orientation(&self) -> Orientation {
        Orientation::from_u32(self.orientation)
    }
}

impl Fields for ImageMetadata {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        if visitor.all_default(&mut self.all_default) {
            // Overwrite all serialized fields, but not any nonserialized_*.
            visitor.set_default(self);
            return Ok(());
        }

        // Bundle::AllDefault does not allow usage when reading (it may abort the
        // program when a codestream has invalid values), but when reading we
        // overwrite the extra_fields value, so do not need to call AllDefault.
        let tone_mapping_default = if visitor.is_reading() {
            false
        } else {
            super::fields::bundle_all_default(&mut self.tone_mapping.clone())
        };

        let mut extra_fields = self.orientation != 1
            || self.have_preview
            || self.have_animation
            || self.have_intrinsic_size
            || !tone_mapping_default;
        visitor.bool_(false, &mut extra_fields)?;
        if visitor.conditional(extra_fields) {
            self.orientation = self.orientation.wrapping_sub(1);
            visitor.bits(3, 0, &mut self.orientation)?;
            self.orientation = self.orientation.wrapping_add(1);
            // (No need for bounds checking because we read exactly 3 bits)

            visitor.bool_(false, &mut self.have_intrinsic_size)?;
            if visitor.conditional(self.have_intrinsic_size) {
                visitor.visit_nested(&mut self.intrinsic_size)?;
            }
            visitor.bool_(false, &mut self.have_preview)?;
            if visitor.conditional(self.have_preview) {
                visitor.visit_nested(&mut self.preview_size)?;
            }
            visitor.bool_(false, &mut self.have_animation)?;
            if visitor.conditional(self.have_animation) {
                visitor.visit_nested(&mut self.animation)?;
            }
        } else {
            self.orientation = 1; // identity
            self.have_intrinsic_size = false;
            self.have_preview = false;
            self.have_animation = false;
        }

        visitor.visit_nested(&mut self.bit_depth)?;
        visitor.bool_(true, &mut self.modular_16_bit_buffer_sufficient)?;

        self.num_extra_channels = self.extra_channel_info.len() as u32;
        visitor.u32d(
            val(0),
            val(1),
            bits_offset(4, 2),
            bits_offset(12, 1),
            0,
            &mut self.num_extra_channels,
        )?;

        if visitor.conditional(self.num_extra_channels != 0) {
            if visitor.is_reading() {
                self.extra_channel_info
                    .resize_with(self.num_extra_channels as usize, ExtraChannelInfo::new);
            }
            for eci in self.extra_channel_info.iter_mut() {
                visitor.visit_nested(eci)?;
            }
        }

        visitor.bool_(true, &mut self.xyb_encoded)?;
        visitor.visit_nested(&mut self.color_encoding)?;
        if visitor.conditional(extra_fields) {
            visitor.visit_nested(&mut self.tone_mapping)?;
        }

        // Treat as if only the fields up to extra channels exist.
        if visitor.is_reading() && self.nonserialized_only_parse_basic_info {
            return Ok(());
        }

        visitor.begin_extensions(&mut self.extensions)?;
        // Extensions: in chronological order of being added to the format.
        visitor.end_extensions()
    }
}

/// All metadata applicable to the entire codestream (dimensions, extra
/// channels, ...). Translation of `CodecMetadata`.
#[derive(Clone, Debug)]
pub(crate) struct CodecMetadata {
    // TODO(lode): use the preview and animation fields too, in place of the
    // nonserialized_ ones in ImageMetadata.
    pub m: ImageMetadata,
    // The size of the codestream: this is the nominal size applicable to all
    // frames, although some frames can have a different effective size through
    // crop, dc_level or representing a the preview.
    pub size: SizeHeader,
    // Often default.
    pub transform_data: CustomTransformData,
}

impl CodecMetadata {
    pub(crate) fn new() -> Self {
        CodecMetadata {
            m: ImageMetadata::new(),
            size: SizeHeader::new(),
            transform_data: CustomTransformData::new(),
        }
    }

    pub(crate) fn xsize(&self) -> usize {
        self.size.xsize()
    }
    pub(crate) fn ysize(&self) -> usize {
        self.size.ysize()
    }
    pub(crate) fn oriented_xsize(&self, keep_orientation: bool) -> usize {
        if self.m.get_orientation() as u32 > 4 && !keep_orientation {
            self.ysize()
        } else {
            self.xsize()
        }
    }
    pub(crate) fn oriented_preview_xsize(&self, keep_orientation: bool) -> usize {
        if self.m.get_orientation() as u32 > 4 && !keep_orientation {
            self.m.preview_size.ysize()
        } else {
            self.m.preview_size.xsize()
        }
    }
    pub(crate) fn oriented_ysize(&self, keep_orientation: bool) -> usize {
        if self.m.get_orientation() as u32 > 4 && !keep_orientation {
            self.xsize()
        } else {
            self.ysize()
        }
    }
    pub(crate) fn oriented_preview_ysize(&self, keep_orientation: bool) -> usize {
        if self.m.get_orientation() as u32 > 4 && !keep_orientation {
            self.m.preview_size.xsize()
        } else {
            self.m.preview_size.ysize()
        }
    }
}

// (SetAlphaBits(), SetUintSamples() and the other setters are the
// encoder's.)
