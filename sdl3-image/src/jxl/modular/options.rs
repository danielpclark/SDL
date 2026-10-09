// Rust translation of lib/jxl/modular/options.h from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Modular mode options (the decoder's: the encoder's are not translated).

pub(crate) type PropertyVal = i32;
pub(crate) type Properties = Vec<PropertyVal>;

/// Translation of `Predictor` (the decoder's predictors; 14 and 15 are the
/// encoder's).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(u32)]
pub(crate) enum Predictor {
    #[default]
    Zero = 0,
    Left = 1,
    Top = 2,
    Average0 = 3,
    Select = 4,
    Gradient = 5,
    Weighted = 6,
    TopRight = 7,
    TopLeft = 8,
    LeftLeft = 9,
    Average1 = 10,
    Average2 = 11,
    Average3 = 12,
    Average4 = 13,
}

impl Predictor {
    /// The predictor with the value `v`, for the values checked to be below
    /// `kNumModularPredictors`.
    pub(crate) fn from_u32(v: u32) -> Predictor {
        match v {
            1 => Predictor::Left,
            2 => Predictor::Top,
            3 => Predictor::Average0,
            4 => Predictor::Select,
            5 => Predictor::Gradient,
            6 => Predictor::Weighted,
            7 => Predictor::TopRight,
            8 => Predictor::TopLeft,
            9 => Predictor::LeftLeft,
            10 => Predictor::Average1,
            11 => Predictor::Average2,
            12 => Predictor::Average3,
            13 => Predictor::Average4,
            _ => Predictor::Zero,
        }
    }
}

pub(crate) const K_NUM_MODULAR_PREDICTORS: usize = Predictor::Average4 as usize + 1;
// (Predictor::Best, the first of the encoder's)
pub(crate) const K_PREDICTOR_BEST: u32 = 14;

pub(crate) const K_NUM_STATIC_PROPERTIES: usize = 2; // channel, group_id.

/// Translation of `ModularOptions` (its decoding fields).
#[derive(Clone, Debug)]
pub(crate) struct ModularOptions {
    // Stop encoding/decoding when reaching a (non-meta) channel that has a
    // dimension bigger than max_chan_size.
    pub max_chan_size: usize,

    // Used during decoding for validation of transforms (sqeeezing) scheme.
    pub group_dim: usize,
}

impl Default for ModularOptions {
    fn default() -> Self {
        ModularOptions {
            max_chan_size: 0xFFFFFF,
            group_dim: 0x1FFFFFFF,
        }
    }
}
