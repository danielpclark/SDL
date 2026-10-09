// Rust translation of the parts of src/colr.c from libavif
// (https://github.com/AOMediaCodec/libavif, at the revision SDL_image's
// external/libavif pins: libavif 1.1.1 with SDL's patches) that the YUV
// to RGB conversion uses.
// Copyright 2019 Joe Drago. All rights reserved.
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Color primaries and matrix coefficients: the YUV coefficients of an
//! image's CICP values. The transfer functions and the color space
//! conversion matrices (used by the gain map code) are not translated.

use super::avif::{
    AvifColorPrimaries, AvifImage, AVIF_COLOR_PRIMARIES_BT2020, AVIF_COLOR_PRIMARIES_BT470BG,
    AVIF_COLOR_PRIMARIES_BT470M, AVIF_COLOR_PRIMARIES_BT601, AVIF_COLOR_PRIMARIES_BT709,
    AVIF_COLOR_PRIMARIES_EBU3213, AVIF_COLOR_PRIMARIES_GENERIC_FILM, AVIF_COLOR_PRIMARIES_SMPTE240,
    AVIF_COLOR_PRIMARIES_SMPTE431, AVIF_COLOR_PRIMARIES_SMPTE432, AVIF_COLOR_PRIMARIES_XYZ,
    AVIF_MATRIX_COEFFICIENTS_BT2020_NCL, AVIF_MATRIX_COEFFICIENTS_BT470BG,
    AVIF_MATRIX_COEFFICIENTS_BT601, AVIF_MATRIX_COEFFICIENTS_BT709,
    AVIF_MATRIX_COEFFICIENTS_CHROMA_DERIVED_NCL, AVIF_MATRIX_COEFFICIENTS_FCC,
    AVIF_MATRIX_COEFFICIENTS_SMPTE240,
};

/// Translation of `struct avifColorPrimariesTable`.
struct AvifColorPrimariesTable {
    color_primaries_enum: AvifColorPrimaries,
    #[allow(dead_code)]
    name: &'static str,
    /// rX, rY, gX, gY, bX, bY, wX, wY
    primaries: [f32; 8],
}

static AVIF_COLOR_PRIMARIES_TABLES: &[AvifColorPrimariesTable] = &[
    AvifColorPrimariesTable {
        color_primaries_enum: AVIF_COLOR_PRIMARIES_BT709,
        name: "BT.709",
        primaries: [0.64, 0.33, 0.3, 0.6, 0.15, 0.06, 0.3127, 0.329],
    },
    AvifColorPrimariesTable {
        color_primaries_enum: AVIF_COLOR_PRIMARIES_BT470M,
        name: "BT.470-6 System M",
        primaries: [0.67, 0.33, 0.21, 0.71, 0.14, 0.08, 0.310, 0.316],
    },
    AvifColorPrimariesTable {
        color_primaries_enum: AVIF_COLOR_PRIMARIES_BT470BG,
        name: "BT.470-6 System BG",
        primaries: [0.64, 0.33, 0.29, 0.60, 0.15, 0.06, 0.3127, 0.3290],
    },
    AvifColorPrimariesTable {
        color_primaries_enum: AVIF_COLOR_PRIMARIES_BT601,
        name: "BT.601",
        primaries: [0.630, 0.340, 0.310, 0.595, 0.155, 0.070, 0.3127, 0.3290],
    },
    AvifColorPrimariesTable {
        color_primaries_enum: AVIF_COLOR_PRIMARIES_SMPTE240,
        name: "SMPTE 240M",
        primaries: [0.630, 0.340, 0.310, 0.595, 0.155, 0.070, 0.3127, 0.3290],
    },
    AvifColorPrimariesTable {
        color_primaries_enum: AVIF_COLOR_PRIMARIES_GENERIC_FILM,
        name: "Generic film",
        primaries: [0.681, 0.319, 0.243, 0.692, 0.145, 0.049, 0.310, 0.316],
    },
    AvifColorPrimariesTable {
        color_primaries_enum: AVIF_COLOR_PRIMARIES_BT2020,
        name: "BT.2020",
        primaries: [0.708, 0.292, 0.170, 0.797, 0.131, 0.046, 0.3127, 0.3290],
    },
    AvifColorPrimariesTable {
        color_primaries_enum: AVIF_COLOR_PRIMARIES_XYZ,
        name: "XYZ",
        primaries: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.3333, 0.3333],
    },
    AvifColorPrimariesTable {
        color_primaries_enum: AVIF_COLOR_PRIMARIES_SMPTE431,
        name: "SMPTE RP 431-2",
        primaries: [0.680, 0.320, 0.265, 0.690, 0.150, 0.060, 0.314, 0.351],
    },
    AvifColorPrimariesTable {
        color_primaries_enum: AVIF_COLOR_PRIMARIES_SMPTE432,
        name: "SMPTE EG 432-1 (DCI P3)",
        primaries: [0.680, 0.320, 0.265, 0.690, 0.150, 0.060, 0.3127, 0.3290],
    },
    AvifColorPrimariesTable {
        color_primaries_enum: AVIF_COLOR_PRIMARIES_EBU3213,
        name: "EBU Tech. 3213-E",
        primaries: [0.630, 0.340, 0.295, 0.605, 0.155, 0.077, 0.3127, 0.3290],
    },
];

/// outPrimaries: rX, rY, gX, gY, bX, bY, wX, wY. Translation of
/// `avifColorPrimariesGetValues()`.
pub(crate) fn avif_color_primaries_get_values(acp: AvifColorPrimaries) -> [f32; 8] {
    for table in AVIF_COLOR_PRIMARIES_TABLES {
        if table.color_primaries_enum == acp {
            return table.primaries;
        }
    }

    // if we get here, the color primaries are unknown. Just return a reasonable default.
    AVIF_COLOR_PRIMARIES_TABLES[0].primaries
}

/// Translation of `struct avifMatrixCoefficientsTable`.
struct AvifMatrixCoefficientsTable {
    matrix_coefficients_enum: u16,
    #[allow(dead_code)]
    name: &'static str,
    kr: f32,
    kb: f32,
}

// https://www.itu.int/rec/T-REC-H.273-201612-I/en
static MATRIX_COEFFICIENTS_TABLES: &[AvifMatrixCoefficientsTable] = &[
    //{ AVIF_MATRIX_COEFFICIENTS_IDENTITY, "Identity", 0.0f, 0.0f, }, // Handled elsewhere
    AvifMatrixCoefficientsTable {
        matrix_coefficients_enum: AVIF_MATRIX_COEFFICIENTS_BT709,
        name: "BT.709",
        kr: 0.2126,
        kb: 0.0722,
    },
    AvifMatrixCoefficientsTable {
        matrix_coefficients_enum: AVIF_MATRIX_COEFFICIENTS_FCC,
        name: "FCC USFC 73.682",
        kr: 0.30,
        kb: 0.11,
    },
    AvifMatrixCoefficientsTable {
        matrix_coefficients_enum: AVIF_MATRIX_COEFFICIENTS_BT470BG,
        name: "BT.470-6 System BG",
        kr: 0.299,
        kb: 0.114,
    },
    AvifMatrixCoefficientsTable {
        matrix_coefficients_enum: AVIF_MATRIX_COEFFICIENTS_BT601,
        name: "BT.601",
        kr: 0.299,
        kb: 0.114,
    },
    AvifMatrixCoefficientsTable {
        matrix_coefficients_enum: AVIF_MATRIX_COEFFICIENTS_SMPTE240,
        name: "SMPTE ST 240",
        kr: 0.212,
        kb: 0.087,
    },
    //{ AVIF_MATRIX_COEFFICIENTS_YCGCO, "YCgCo", 0.0f, 0.0f, }, // Handled elsewhere
    AvifMatrixCoefficientsTable {
        matrix_coefficients_enum: AVIF_MATRIX_COEFFICIENTS_BT2020_NCL,
        name: "BT.2020 (non-constant luminance)",
        kr: 0.2627,
        kb: 0.0593,
    },
    //{ AVIF_MATRIX_COEFFICIENTS_BT2020_CL, "BT.2020 (constant luminance)", 0.2627f, 0.0593f }, // FIXME: It is not an linear transformation.
    //{ AVIF_MATRIX_COEFFICIENTS_SMPTE2085, "ST 2085", 0.0f, 0.0f }, // FIXME: ST2085 can't represent using Kr and Kb.
    //{ AVIF_MATRIX_COEFFICIENTS_CHROMA_DERIVED_CL, "Chromaticity-derived constant luminance system", 0.0f, 0.0f } // FIXME: It is not an linear transformation.
    //{ AVIF_MATRIX_COEFFICIENTS_ICTCP, "BT.2100-0 ICtCp", 0.0f, 0.0f }, // FIXME: This can't represent using Kr and Kb.
];

/// Translation of `calcYUVInfoFromCICP()`.
fn calc_yuv_info_from_cicp(image: &AvifImage, coeffs: &mut [f32; 3]) -> bool {
    if image.matrix_coefficients == AVIF_MATRIX_COEFFICIENTS_CHROMA_DERIVED_NCL {
        *coeffs = avif_color_primaries_compute_y_coeffs(image.color_primaries);
        return true;
    } else {
        for table in MATRIX_COEFFICIENTS_TABLES {
            if table.matrix_coefficients_enum == image.matrix_coefficients {
                coeffs[0] = table.kr;
                coeffs[2] = table.kb;
                coeffs[1] = 1.0f32 - coeffs[0] - coeffs[2];
                return true;
            }
        }
    }
    false
}

/// Translation of `avifCalcYUVCoefficients()`: (kr, kg, kb).
pub(crate) fn avif_calc_yuv_coefficients(image: &AvifImage) -> (f32, f32, f32) {
    // (As of ISO/IEC 23000-22:2019 Amendment 2)
    // MIAF Section 7.3.6.4 "Colour information property":
    //
    // If a coded image has no associated colour property, the default property is defined as having
    // colour_type equal to 'nclx' with properties as follows:
    // -   colour_primaries equal to 1,
    // -   transfer_characteristics equal to 13,
    // -   matrix_coefficients equal to 5 or 6 (which are functionally identical), and
    // -   full_range_flag equal to 1.
    // Only if the colour information property of the image matches these default values, the colour
    // property may be omitted; all other images shall have an explicitly declared colour space via
    // association with a property of this type.
    //
    // See here for the discussion: https://github.com/AOMediaCodec/av1-avif/issues/77#issuecomment-676526097

    // matrix_coefficients of [5,6] == BT.601:
    let mut kr = 0.299f32;
    let mut kb = 0.114f32;
    let mut kg = 1.0f32 - kr - kb;

    let mut coeffs = [0f32; 3];
    if calc_yuv_info_from_cicp(image, &mut coeffs) {
        kr = coeffs[0];
        kg = coeffs[1];
        kb = coeffs[2];
    }

    (kr, kg, kb)
}

/// Computes the RGB->YUV conversion coefficients kr, kg, kb, such that
/// Y=kr*R+kg*G+kb*B. Translation of `avifColorPrimariesComputeYCoeffs()`.
pub(crate) fn avif_color_primaries_compute_y_coeffs(
    color_primaries: AvifColorPrimaries,
) -> [f32; 3] {
    let primaries = avif_color_primaries_get_values(color_primaries);
    let r_x = primaries[0];
    let r_y = primaries[1];
    let g_x = primaries[2];
    let g_y = primaries[3];
    let b_x = primaries[4];
    let b_y = primaries[5];
    let w_x = primaries[6];
    let w_y = primaries[7];
    let r_z = 1.0f32 - (r_x + r_y); // (Eq. 34)
    let g_z = 1.0f32 - (g_x + g_y); // (Eq. 35)
    let b_z = 1.0f32 - (b_x + b_y); // (Eq. 36)
    let w_z = 1.0f32 - (w_x + w_y); // (Eq. 37)
    let kr = (r_y
        * (w_x * (g_y * b_z - b_y * g_z)
            + w_y * (b_x * g_z - g_x * b_z)
            + w_z * (g_x * b_y - b_x * g_y)))
        / (w_y
            * (r_x * (g_y * b_z - b_y * g_z)
                + g_x * (b_y * r_z - r_y * b_z)
                + b_x * (r_y * g_z - g_y * r_z)));
    // (Eq. 32)
    let kb = (b_y
        * (w_x * (r_y * g_z - g_y * r_z)
            + w_y * (g_x * r_z - r_x * g_z)
            + w_z * (r_x * g_y - g_x * r_y)))
        / (w_y
            * (r_x * (g_y * b_z - b_y * g_z)
                + g_x * (b_y * r_z - r_y * b_z)
                + b_x * (r_y * g_z - g_y * r_z)));
    // (Eq. 33)
    [kr, 1.0f32 - kr - kb, kb]
}
