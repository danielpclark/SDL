// Rust translation of av1/common/filter.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The interpolation filters of inter prediction.

pub(crate) const MAX_FILTER_TAP: usize = 12;

// (InterpFilter)
pub(crate) type InterpFilter = i32;
pub(crate) const EIGHTTAP_REGULAR: InterpFilter = 0;
pub(crate) const EIGHTTAP_SMOOTH: InterpFilter = 1;
pub(crate) const MULTITAP_SHARP: InterpFilter = 2;
pub(crate) const BILINEAR: InterpFilter = 3;
// Encoder side only filters
pub(crate) const MULTITAP_SHARP2: InterpFilter = 4;

pub(crate) const INTERP_FILTERS_ALL: InterpFilter = 5;
pub(crate) const SWITCHABLE_FILTERS: InterpFilter = BILINEAR;
pub(crate) const SWITCHABLE: InterpFilter = SWITCHABLE_FILTERS + 1; /* the last switchable one */
pub(crate) const EXTRA_FILTERS: InterpFilter = INTERP_FILTERS_ALL - SWITCHABLE_FILTERS;
pub(crate) const INTERP_INVALID: InterpFilter = 0xff;

// (SUBPEL_SEARCH_TYPE)
pub(crate) const USE_2_TAPS_ORIG: i32 = 0; // This is used in temporal filtering.
pub(crate) const USE_2_TAPS: i32 = 1;
pub(crate) const USE_4_TAPS: i32 = 2;
pub(crate) const USE_8_TAPS: i32 = 3;

// (INTERP_EVAL_PLANE)
pub(crate) const INTERP_EVAL_LUMA_EVAL_CHROMA: i32 = 0;
pub(crate) const INTERP_SKIP_LUMA_EVAL_CHROMA: i32 = 1;
pub(crate) const INTERP_EVAL_INVALID: i32 = 2;
pub(crate) const INTERP_SKIP_LUMA_SKIP_CHROMA: i32 = 3;

// (INTERP_PRED_TYPE)
pub(crate) const INTERP_HORZ_NEQ_VERT_NEQ: i32 = 0;
pub(crate) const INTERP_HORZ_EQ_VERT_NEQ: i32 = 1;
pub(crate) const INTERP_HORZ_NEQ_VERT_EQ: i32 = 2;
pub(crate) const INTERP_HORZ_EQ_VERT_EQ: i32 = 3;
pub(crate) const INTERP_PRED_TYPE_ALL: i32 = 4;

// Pack two InterpFilter's into a uint32_t: since there are at most 10 filters,
// we can use 16 bits for each and have more than enough space. This reduces
// argument passing and unifies the operation of setting a (pair of) filters.
// (int_interpfilters: as_int, with as_filters.y_filter in the low 16 bits and
// as_filters.x_filter in the high 16 bits, the layout of the union on the
// little-endian machines SDL_image runs on)

/// Translation of `av1_extract_interp_filter()`.
#[inline]
pub(crate) fn av1_extract_interp_filter(filters: u32, dir: i32) -> InterpFilter {
    if dir != 0 {
        (filters >> 16) as InterpFilter
    } else {
        (filters & 0xffff) as InterpFilter
    }
}

/// Translation of `av1_broadcast_interp_filter()`.
#[inline]
pub(crate) fn av1_broadcast_interp_filter(filter: InterpFilter) -> u32 {
    let f = filter as u32 & 0xffff;
    f | (f << 16)
}

/// `int_interpfilters` with `as_filters.y_filter` and `as_filters.x_filter`.
#[inline]
pub(crate) fn interp_filters_of(y_filter: InterpFilter, x_filter: InterpFilter) -> u32 {
    (y_filter as u32 & 0xffff) | ((x_filter as u32 & 0xffff) << 16)
}

/// Translation of `av1_unswitchable_filter()`.
#[inline]
pub(crate) fn av1_unswitchable_filter(filter: InterpFilter) -> InterpFilter {
    if filter == SWITCHABLE {
        EIGHTTAP_REGULAR
    } else {
        filter
    }
}

/* (1 << LOG_SWITCHABLE_FILTERS) > SWITCHABLE_FILTERS */
pub(crate) const LOG_SWITCHABLE_FILTERS: i32 = 2;

pub(crate) const SWITCHABLE_FILTER_CONTEXTS: usize = ((SWITCHABLE_FILTERS + 1) * 4) as usize;
pub(crate) const INTER_FILTER_COMP_OFFSET: i32 = SWITCHABLE_FILTERS + 1;
pub(crate) const INTER_FILTER_DIR_OFFSET: i32 = (SWITCHABLE_FILTERS + 1) * 2;
pub(crate) const ALLOW_ALL_INTERP_FILT_MASK: u16 = 0x01ff;

pub(crate) const SUBPEL_BITS: i32 = 4;
pub(crate) const SUBPEL_MASK: i32 = (1 << SUBPEL_BITS) - 1;
pub(crate) const SUBPEL_SHIFTS: usize = 1 << SUBPEL_BITS;
pub(crate) const SUBPEL_TAPS: u16 = 8;

/// `InterpKernel`
pub(crate) type InterpKernel = [i16; SUBPEL_TAPS as usize];

/// Translation of `InterpFilterParams`: `filter_ptr` is the kernel table.
#[derive(Debug, Clone, Copy)]
pub(crate) struct InterpFilterParams {
    pub(crate) filter_ptr: &'static [i16],
    pub(crate) taps: u16,
    pub(crate) interp_filter: InterpFilter,
}

static AV1_BILINEAR_FILTERS: [InterpKernel; SUBPEL_SHIFTS] = [
    [0, 0, 0, 128, 0, 0, 0, 0],
    [0, 0, 0, 120, 8, 0, 0, 0],
    [0, 0, 0, 112, 16, 0, 0, 0],
    [0, 0, 0, 104, 24, 0, 0, 0],
    [0, 0, 0, 96, 32, 0, 0, 0],
    [0, 0, 0, 88, 40, 0, 0, 0],
    [0, 0, 0, 80, 48, 0, 0, 0],
    [0, 0, 0, 72, 56, 0, 0, 0],
    [0, 0, 0, 64, 64, 0, 0, 0],
    [0, 0, 0, 56, 72, 0, 0, 0],
    [0, 0, 0, 48, 80, 0, 0, 0],
    [0, 0, 0, 40, 88, 0, 0, 0],
    [0, 0, 0, 32, 96, 0, 0, 0],
    [0, 0, 0, 24, 104, 0, 0, 0],
    [0, 0, 0, 16, 112, 0, 0, 0],
    [0, 0, 0, 8, 120, 0, 0, 0],
];

static AV1_SUB_PEL_FILTERS_8: [InterpKernel; SUBPEL_SHIFTS] = [
    [0, 0, 0, 128, 0, 0, 0, 0],
    [0, 2, -6, 126, 8, -2, 0, 0],
    [0, 2, -10, 122, 18, -4, 0, 0],
    [0, 2, -12, 116, 28, -8, 2, 0],
    [0, 2, -14, 110, 38, -10, 2, 0],
    [0, 2, -14, 102, 48, -12, 2, 0],
    [0, 2, -16, 94, 58, -12, 2, 0],
    [0, 2, -14, 84, 66, -12, 2, 0],
    [0, 2, -14, 76, 76, -14, 2, 0],
    [0, 2, -12, 66, 84, -14, 2, 0],
    [0, 2, -12, 58, 94, -16, 2, 0],
    [0, 2, -12, 48, 102, -14, 2, 0],
    [0, 2, -10, 38, 110, -14, 2, 0],
    [0, 2, -8, 28, 116, -12, 2, 0],
    [0, 0, -4, 18, 122, -10, 2, 0],
    [0, 0, -2, 8, 126, -6, 2, 0],
];

static AV1_SUB_PEL_FILTERS_8SHARP: [InterpKernel; SUBPEL_SHIFTS] = [
    [0, 0, 0, 128, 0, 0, 0, 0],
    [-2, 2, -6, 126, 8, -2, 2, 0],
    [-2, 6, -12, 124, 16, -6, 4, -2],
    [-2, 8, -18, 120, 26, -10, 6, -2],
    [-4, 10, -22, 116, 38, -14, 6, -2],
    [-4, 10, -22, 108, 48, -18, 8, -2],
    [-4, 10, -24, 100, 60, -20, 8, -2],
    [-4, 10, -24, 90, 70, -22, 10, -2],
    [-4, 12, -24, 80, 80, -24, 12, -4],
    [-2, 10, -22, 70, 90, -24, 10, -4],
    [-2, 8, -20, 60, 100, -24, 10, -4],
    [-2, 8, -18, 48, 108, -22, 10, -4],
    [-2, 6, -14, 38, 116, -22, 10, -4],
    [-2, 6, -10, 26, 120, -18, 8, -2],
    [-2, 4, -6, 16, 124, -12, 6, -2],
    [0, 2, -2, 8, 126, -6, 2, -2],
];

static AV1_SUB_PEL_FILTERS_8SMOOTH: [InterpKernel; SUBPEL_SHIFTS] = [
    [0, 0, 0, 128, 0, 0, 0, 0],
    [0, 2, 28, 62, 34, 2, 0, 0],
    [0, 0, 26, 62, 36, 4, 0, 0],
    [0, 0, 22, 62, 40, 4, 0, 0],
    [0, 0, 20, 60, 42, 6, 0, 0],
    [0, 0, 18, 58, 44, 8, 0, 0],
    [0, 0, 16, 56, 46, 10, 0, 0],
    [0, -2, 16, 54, 48, 12, 0, 0],
    [0, -2, 14, 52, 52, 14, -2, 0],
    [0, 0, 12, 48, 54, 16, -2, 0],
    [0, 0, 10, 46, 56, 16, 0, 0],
    [0, 0, 8, 44, 58, 18, 0, 0],
    [0, 0, 6, 42, 60, 20, 0, 0],
    [0, 0, 4, 40, 62, 22, 0, 0],
    [0, 0, 4, 36, 62, 26, 0, 0],
    [0, 0, 2, 34, 62, 28, 2, 0],
];

static AV1_SUB_PEL_FILTERS_12SHARP: [[i16; 12]; SUBPEL_SHIFTS] = [
    [0, 0, 0, 0, 0, 128, 0, 0, 0, 0, 0, 0],
    [0, 1, -2, 3, -7, 127, 8, -4, 2, -1, 1, 0],
    [-1, 2, -3, 6, -13, 124, 18, -8, 4, -2, 2, -1],
    [-1, 3, -4, 8, -18, 120, 28, -12, 7, -4, 2, -1],
    [-1, 3, -6, 10, -21, 115, 38, -15, 8, -5, 3, -1],
    [-2, 4, -6, 12, -24, 108, 49, -18, 10, -6, 3, -2],
    [-2, 4, -7, 13, -25, 100, 60, -21, 11, -7, 4, -2],
    [-2, 4, -7, 13, -26, 91, 71, -24, 13, -7, 4, -2],
    [-2, 4, -7, 13, -25, 81, 81, -25, 13, -7, 4, -2],
    [-2, 4, -7, 13, -24, 71, 91, -26, 13, -7, 4, -2],
    [-2, 4, -7, 11, -21, 60, 100, -25, 13, -7, 4, -2],
    [-2, 3, -6, 10, -18, 49, 108, -24, 12, -6, 4, -2],
    [-1, 3, -5, 8, -15, 38, 115, -21, 10, -6, 3, -1],
    [-1, 2, -4, 7, -12, 28, 120, -18, 8, -4, 3, -1],
    [-1, 2, -2, 4, -8, 18, 124, -13, 6, -3, 2, -1],
    [0, 1, -1, 2, -4, 8, 127, -7, 3, -2, 1, 0],
];

/// The kernels of a table, as one slice (the C code's `(const int16_t *)`
/// cast of the table).
const fn flat<const N: usize, const T: usize>(table: &'static [[i16; T]; N]) -> &'static [i16] {
    // SAFETY-free flattening: a [[i16; T]; N] is laid out as N * T i16s.
    table.as_flattened()
}

pub(crate) static AV1_INTERP_FILTER_PARAMS_LIST: [InterpFilterParams; INTERP_FILTERS_ALL as usize] = [
    InterpFilterParams {
        filter_ptr: flat(&AV1_SUB_PEL_FILTERS_8),
        taps: SUBPEL_TAPS,
        interp_filter: EIGHTTAP_REGULAR,
    },
    InterpFilterParams {
        filter_ptr: flat(&AV1_SUB_PEL_FILTERS_8SMOOTH),
        taps: SUBPEL_TAPS,
        interp_filter: EIGHTTAP_SMOOTH,
    },
    InterpFilterParams {
        filter_ptr: flat(&AV1_SUB_PEL_FILTERS_8SHARP),
        taps: SUBPEL_TAPS,
        interp_filter: MULTITAP_SHARP,
    },
    InterpFilterParams {
        filter_ptr: flat(&AV1_BILINEAR_FILTERS),
        taps: SUBPEL_TAPS,
        interp_filter: BILINEAR,
    },
    // The following filters are for encoder only, and now they are used in
    // temporal filtering. The predictor block size >= 16 in temporal filter.
    InterpFilterParams {
        filter_ptr: flat(&AV1_SUB_PEL_FILTERS_12SHARP),
        taps: 12,
        interp_filter: MULTITAP_SHARP2,
    },
];

// A special 2-tap bilinear filter for IntraBC chroma. IntraBC uses full pixel
// MV for luma. If sub-sampling exists, chroma may possibly use half-pel MV.
static AV1_INTRABC_BILINEAR_FILTER: [i16; 2 * SUBPEL_SHIFTS] = [
    128, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, //
    64, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

pub(crate) static AV1_INTRABC_FILTER_PARAMS: InterpFilterParams = InterpFilterParams {
    filter_ptr: &AV1_INTRABC_BILINEAR_FILTER,
    taps: 2,
    interp_filter: BILINEAR,
};

static AV1_SUB_PEL_FILTERS_4: [InterpKernel; SUBPEL_SHIFTS] = [
    [0, 0, 0, 128, 0, 0, 0, 0],
    [0, 0, -4, 126, 8, -2, 0, 0],
    [0, 0, -8, 122, 18, -4, 0, 0],
    [0, 0, -10, 116, 28, -6, 0, 0],
    [0, 0, -12, 110, 38, -8, 0, 0],
    [0, 0, -12, 102, 48, -10, 0, 0],
    [0, 0, -14, 94, 58, -10, 0, 0],
    [0, 0, -12, 84, 66, -10, 0, 0],
    [0, 0, -12, 76, 76, -12, 0, 0],
    [0, 0, -10, 66, 84, -12, 0, 0],
    [0, 0, -10, 58, 94, -14, 0, 0],
    [0, 0, -10, 48, 102, -12, 0, 0],
    [0, 0, -8, 38, 110, -12, 0, 0],
    [0, 0, -6, 28, 116, -10, 0, 0],
    [0, 0, -4, 18, 122, -8, 0, 0],
    [0, 0, -2, 8, 126, -4, 0, 0],
];
static AV1_SUB_PEL_FILTERS_4SMOOTH: [InterpKernel; SUBPEL_SHIFTS] = [
    [0, 0, 0, 128, 0, 0, 0, 0],
    [0, 0, 30, 62, 34, 2, 0, 0],
    [0, 0, 26, 62, 36, 4, 0, 0],
    [0, 0, 22, 62, 40, 4, 0, 0],
    [0, 0, 20, 60, 42, 6, 0, 0],
    [0, 0, 18, 58, 44, 8, 0, 0],
    [0, 0, 16, 56, 46, 10, 0, 0],
    [0, 0, 14, 54, 48, 12, 0, 0],
    [0, 0, 12, 52, 52, 12, 0, 0],
    [0, 0, 12, 48, 54, 14, 0, 0],
    [0, 0, 10, 46, 56, 16, 0, 0],
    [0, 0, 8, 44, 58, 18, 0, 0],
    [0, 0, 6, 42, 60, 20, 0, 0],
    [0, 0, 4, 40, 62, 22, 0, 0],
    [0, 0, 4, 36, 62, 26, 0, 0],
    [0, 0, 2, 34, 62, 30, 0, 0],
];

// (DUAL_FILTER_TYPE)
pub(crate) const REG_REG: i32 = 0;
pub(crate) const REG_SMOOTH: i32 = 1; // filter[1]: SMOOTH, filter[0]: REGULAR (x, y)
pub(crate) const REG_SHARP: i32 = 2;
pub(crate) const SMOOTH_REG: i32 = 3;
pub(crate) const SMOOTH_SMOOTH: i32 = 4;
pub(crate) const SMOOTH_SHARP: i32 = 5;
pub(crate) const SHARP_REG: i32 = 6;
pub(crate) const SHARP_SMOOTH: i32 = 7;
pub(crate) const SHARP_SHARP: i32 = 8;

pub(crate) static AV1_INTERP_DUAL_FILT_MASK: [[u16; SWITCHABLE_FILTERS as usize];
    (INTERP_PRED_TYPE_ALL - 2) as usize] = [
    [
        (1 << REG_REG) | (1 << SMOOTH_REG) | (1 << SHARP_REG),
        (1 << REG_SMOOTH) | (1 << SMOOTH_SMOOTH) | (1 << SHARP_SMOOTH),
        (1 << REG_SHARP) | (1 << SMOOTH_SHARP) | (1 << SHARP_SHARP),
    ],
    [
        (1 << REG_REG) | (1 << REG_SMOOTH) | (1 << REG_SHARP),
        (1 << SMOOTH_REG) | (1 << SMOOTH_SMOOTH) | (1 << SMOOTH_SHARP),
        (1 << SHARP_REG) | (1 << SHARP_SMOOTH) | (1 << SHARP_SHARP),
    ],
];

// For w<=4, MULTITAP_SHARP is the same as EIGHTTAP_REGULAR
pub(crate) static AV1_INTERP_4TAP: [InterpFilterParams; SWITCHABLE_FILTERS as usize + 1] = [
    InterpFilterParams {
        filter_ptr: flat(&AV1_SUB_PEL_FILTERS_4),
        taps: SUBPEL_TAPS,
        interp_filter: EIGHTTAP_REGULAR,
    },
    InterpFilterParams {
        filter_ptr: flat(&AV1_SUB_PEL_FILTERS_4SMOOTH),
        taps: SUBPEL_TAPS,
        interp_filter: EIGHTTAP_SMOOTH,
    },
    InterpFilterParams {
        filter_ptr: flat(&AV1_SUB_PEL_FILTERS_4),
        taps: SUBPEL_TAPS,
        interp_filter: EIGHTTAP_REGULAR,
    },
    InterpFilterParams {
        filter_ptr: flat(&AV1_BILINEAR_FILTERS),
        taps: SUBPEL_TAPS,
        interp_filter: BILINEAR,
    },
];

/// Translation of `av1_get_interp_filter_params_with_block_size()`.
#[inline]
pub(crate) fn av1_get_interp_filter_params_with_block_size(
    interp_filter: InterpFilter,
    w: i32,
) -> &'static InterpFilterParams {
    if w <= 4 && interp_filter != MULTITAP_SHARP2 {
        return &AV1_INTERP_4TAP[interp_filter as usize];
    }
    &AV1_INTERP_FILTER_PARAMS_LIST[interp_filter as usize]
}

/// Translation of `av1_get_interp_filter_kernel()`.
#[inline]
pub(crate) fn av1_get_interp_filter_kernel(
    interp_filter: InterpFilter,
    subpel_search: i32,
) -> &'static [i16] {
    debug_assert!(subpel_search >= USE_2_TAPS);
    if subpel_search == USE_2_TAPS {
        AV1_INTERP_4TAP[BILINEAR as usize].filter_ptr
    } else if subpel_search == USE_4_TAPS {
        AV1_INTERP_4TAP[interp_filter as usize].filter_ptr
    } else {
        AV1_INTERP_FILTER_PARAMS_LIST[interp_filter as usize].filter_ptr
    }
}

/// Translation of `av1_get_interp_filter_subpel_kernel()`.
#[inline]
pub(crate) fn av1_get_interp_filter_subpel_kernel(
    filter_params: &InterpFilterParams,
    subpel: i32,
) -> &'static [i16] {
    let taps = filter_params.taps as usize;
    let start = taps * subpel as usize;
    &filter_params.filter_ptr[start..start + taps]
}

/// Translation of `av1_get_filter()`.
#[inline]
pub(crate) fn av1_get_filter(subpel_search: i32) -> Option<&'static InterpFilterParams> {
    debug_assert!(subpel_search >= USE_2_TAPS);

    match subpel_search {
        USE_2_TAPS => Some(&AV1_INTERP_4TAP[BILINEAR as usize]),
        USE_4_TAPS => Some(&AV1_INTERP_4TAP[EIGHTTAP_REGULAR as usize]),
        USE_8_TAPS => Some(&AV1_INTERP_FILTER_PARAMS_LIST[EIGHTTAP_REGULAR as usize]),
        _ => {
            debug_assert!(false);
            None
        }
    }
}

/// Translation of `reset_interp_filter_allowed_mask()`.
#[inline]
pub(crate) fn reset_interp_filter_allowed_mask(allow_interp_mask: &mut u16, filt_type: i32) {
    let tmp = (!(1u32 << filt_type) & 0xffff) as u16;
    *allow_interp_mask &= tmp & ALLOW_ALL_INTERP_FILT_MASK;
}

/// Translation of `set_interp_filter_allowed_mask()`.
#[inline]
pub(crate) fn set_interp_filter_allowed_mask(allow_interp_mask: &mut u16, filt_type: i32) {
    *allow_interp_mask |= 1 << filt_type;
}

/// Translation of `get_interp_filter_allowed_mask()`.
#[inline]
pub(crate) fn get_interp_filter_allowed_mask(allow_interp_mask: u16, filt_type: i32) -> u8 {
    ((allow_interp_mask >> filt_type) & 1) as u8
}

/// Translation of `get_filter_tap()`.
#[inline]
pub(crate) fn get_filter_tap(filter_params: &InterpFilterParams, subpel_qn: i32) -> i32 {
    let filter = av1_get_interp_filter_subpel_kernel(filter_params, subpel_qn & SUBPEL_MASK);
    if filter_params.taps == 12 {
        return 12;
    }
    if (filter[0] | filter[7]) != 0 {
        return 8;
    }
    if (filter[1] | filter[6]) != 0 {
        return 6;
    }
    if (filter[2] | filter[5]) != 0 {
        return 4;
    }
    2
}
