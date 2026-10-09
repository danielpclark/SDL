// Rust translation of lib/jxl/quantizer.h and lib/jxl/quantizer.cc from
// libjxl (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Quantizer: the global and per-block quantization scales.

// This follows from computing the distribution of the quantization bias, which
// can be approximated fairly well by <constant>/x when |x| is at least two.
#[allow(dead_code)]
pub(crate) const K_BIAS_NUMERATOR: f32 = 0.145;

pub(crate) const K_DEFAULT_QUANT_BIAS: [f32; 4] = [
    1.0 - 0.05465007330715401,
    1.0 - 0.07005449891748593,
    1.0 - 0.049935103337343655,
    0.145,
];
