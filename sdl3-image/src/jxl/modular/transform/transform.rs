// Rust translation of lib/jxl/modular/transform/transform.h and
// lib/jxl/modular/transform/transform.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The modular transforms' parameters, and their dispatch.

use super::super::super::base::{jxl_failure, Status};
use super::super::super::fields::{bits, bits_offset, bundle_init, val, Fields, Visitor};
use super::super::encoding::context_predict::weighted;
use super::super::modular_image::{Image, PixelType};
use super::super::options::{Predictor, K_PREDICTOR_BEST};
use super::palette::{inv_palette, meta_palette};
use super::rct::inv_rct;
use super::squeeze::{inv_squeeze, meta_squeeze};

/// Translation of `TransformId`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub(crate) enum TransformId {
    // G, R-G, B-G and variants (including YCoCg).
    Rct = 0,

    // Color palette. Parameters are: [begin_c] [end_c] [nb_colors]
    Palette = 1,

    // Squeezing (Haar-style)
    Squeeze = 2,

    // Invalid for now.
    Invalid = 3,
}

/// Translation of `SqueezeParams`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SqueezeParams {
    pub horizontal: bool,
    pub in_place: bool,
    pub begin_c: u32,
    pub num_c: u32,
}

impl SqueezeParams {
    pub(crate) fn new() -> Self {
        let mut s = SqueezeParams::default();
        bundle_init(&mut s);
        s
    }
}

impl Fields for SqueezeParams {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        visitor.bool_(false, &mut self.horizontal)?;
        visitor.bool_(false, &mut self.in_place)?;
        visitor.u32d(
            bits(3),
            bits_offset(6, 8),
            bits_offset(10, 72),
            bits_offset(13, 1096),
            0,
            &mut self.begin_c,
        )?;
        visitor.u32d(
            val(1),
            val(2),
            val(3),
            bits_offset(4, 4),
            2,
            &mut self.num_c,
        )?;
        Ok(())
    }
}

/// Translation of `Transform`.
#[derive(Clone, Debug)]
pub(crate) struct Transform {
    pub id: TransformId,
    // for Palette and RCT.
    pub begin_c: u32,
    // for RCT. 42 possible values starting from 0.
    pub rct_type: u32,
    // Only for Palette and NearLossless.
    pub num_c: u32,
    // Only for Palette.
    pub nb_colors: u32,
    pub nb_deltas: u32,
    // for Squeeze. Default squeeze if empty.
    pub squeezes: Vec<SqueezeParams>,
    // for NearLossless, not serialized.
    pub max_delta_error: i32,
    // Serialized for Palette.
    pub predictor: Predictor,
    // for Palette, not serialized.
    pub ordered_palette: bool,
    pub lossy_palette: bool,
}

impl Transform {
    pub(crate) fn new(id: TransformId) -> Self {
        let mut t = Transform {
            id,
            begin_c: 0,
            rct_type: 0,
            num_c: 0,
            nb_colors: 0,
            nb_deltas: 0,
            squeezes: Vec::new(),
            max_delta_error: 0,
            predictor: Predictor::Zero,
            ordered_palette: true,
            lossy_palette: false,
        };
        bundle_init(&mut t);
        t.id = id;
        t
    }

    /// Translation of `Inverse()`.
    pub(crate) fn inverse(&self, input: &mut Image, wp_header: &weighted::Header) -> Status {
        match self.id {
            TransformId::Rct => inv_rct(input, self.begin_c as usize, self.rct_type as usize),
            TransformId::Squeeze => inv_squeeze(input, &self.squeezes),
            TransformId::Palette => inv_palette(
                input,
                self.begin_c,
                self.nb_colors,
                self.nb_deltas,
                self.predictor,
                wp_header,
            ),
            _ => jxl_failure!("Unknown transformation"),
        }
    }

    /// Translation of `MetaApply()`.
    pub(crate) fn meta_apply(&mut self, input: &mut Image) -> Status {
        match self.id {
            TransformId::Rct => {
                check_equal_channels(input, self.begin_c, self.begin_c.wrapping_add(2))
            }
            TransformId::Squeeze => meta_squeeze(input, &mut self.squeezes),
            TransformId::Palette => meta_palette(
                input,
                self.begin_c,
                self.begin_c.wrapping_add(self.num_c).wrapping_sub(1),
                self.nb_colors,
                self.nb_deltas,
                self.lossy_palette,
            ),
            _ => jxl_failure!("Unknown transformation"),
        }
    }
}

impl Default for Transform {
    // default constructor for bundles.
    fn default() -> Self {
        Transform::new(TransformId::Invalid)
    }
}

impl Fields for Transform {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        let mut id = self.id as u32;
        visitor.u32d(
            val(TransformId::Rct as u32),
            val(TransformId::Palette as u32),
            val(TransformId::Squeeze as u32),
            val(TransformId::Invalid as u32),
            TransformId::Rct as u32,
            &mut id,
        )?;
        self.id = match id {
            0 => TransformId::Rct,
            1 => TransformId::Palette,
            2 => TransformId::Squeeze,
            _ => TransformId::Invalid,
        };
        if self.id == TransformId::Invalid {
            return jxl_failure!("Invalid transform ID");
        }
        if visitor.conditional(self.id == TransformId::Rct || self.id == TransformId::Palette) {
            visitor.u32d(
                bits(3),
                bits_offset(6, 8),
                bits_offset(10, 72),
                bits_offset(13, 1096),
                0,
                &mut self.begin_c,
            )?;
        }
        if visitor.conditional(self.id == TransformId::Rct) {
            // 0-41, default YCoCg.
            visitor.u32d(
                val(6),
                bits(2),
                bits_offset(4, 2),
                bits_offset(6, 10),
                6,
                &mut self.rct_type,
            )?;
            if self.rct_type >= 42 {
                return jxl_failure!("Invalid transform RCT type");
            }
        }
        if visitor.conditional(self.id == TransformId::Palette) {
            visitor.u32d(
                val(1),
                val(3),
                val(4),
                bits_offset(13, 1),
                3,
                &mut self.num_c,
            )?;
            visitor.u32d(
                bits_offset(8, 0),
                bits_offset(10, 256),
                bits_offset(12, 1280),
                bits_offset(16, 5376),
                256,
                &mut self.nb_colors,
            )?;
            visitor.u32d(
                val(0),
                bits_offset(8, 1),
                bits_offset(10, 257),
                bits_offset(16, 1281),
                0,
                &mut self.nb_deltas,
            )?;
            let mut predictor = self.predictor as u32;
            visitor.bits(4, Predictor::Zero as u32, &mut predictor)?;
            if predictor >= K_PREDICTOR_BEST {
                return jxl_failure!("Invalid predictor");
            }
            self.predictor = Predictor::from_u32(predictor);
        }

        if visitor.conditional(self.id == TransformId::Squeeze) {
            let mut num_squeezes = self.squeezes.len() as u32;
            visitor.u32d(
                val(0),
                bits_offset(4, 1),
                bits_offset(6, 9),
                bits_offset(8, 41),
                0,
                &mut num_squeezes,
            )?;
            if visitor.is_reading() {
                self.squeezes
                    .resize_with(num_squeezes as usize, SqueezeParams::new);
            }
            for i in 0..num_squeezes as usize {
                visitor.visit_nested(&mut self.squeezes[i])?;
            }
        }
        Ok(())
    }
}

/// Translation of `CheckEqualChannels()`.
pub(crate) fn check_equal_channels(image: &Image, c1: u32, c2: u32) -> Status {
    let (c1, c2) = (c1 as usize, c2 as usize);
    if c1 > image.channel.len() || c2 >= image.channel.len() || c2 < c1 {
        return jxl_failure!("Invalid channel range");
    }
    if c1 < image.nb_meta_channels && c2 >= image.nb_meta_channels {
        return jxl_failure!("Invalid: transforming mix of meta and nonmeta");
    }
    let ch1 = &image.channel[c1];
    for c in c1 + 1..=c2 {
        let ch2 = &image.channel[c];
        if ch1.w != ch2.w || ch1.h != ch2.h || ch1.hshift != ch2.hshift || ch1.vshift != ch2.vshift
        {
            return jxl_failure!("unequal channels");
        }
    }
    Ok(())
}

/// Translation of `PixelAdd()`.
#[inline]
pub(crate) fn pixel_add(a: PixelType, b: PixelType) -> PixelType {
    (a as u32).wrapping_add(b as u32) as PixelType
}
