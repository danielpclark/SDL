// Rust translation of the parts of av1/encoder/mcomp.c, mcomp.h and
// mcomp_structs.h from aom (https://aomedia.googlesource.com/aom, at the
// revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's patches)
// the real-time encoder reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Motion search.

use super::block::{with_source, Macroblock, MvCosts, K_MED_SAD};
use super::cost::AV1_PROB_COST_SHIFT;
use super::encodemv::av1_get_mv_joint;
use super::encoder::Av1Comp;
use super::rd::{av1_get_scaled_ref_frame, RDDIV_BITS, RD_EPB_SHIFT};
use super::speed_features::{
    PRUNE_MESH_SEARCH_LVL_2, SUBPEL_TREE, SUBPEL_TREE_PRUNED, SUBPEL_TREE_PRUNED_MORE,
};
use crate::aom::aom_dsp::avg::{aom_int_pro_col, aom_int_pro_row, aom_vector_var};
use crate::aom::aom_dsp::variance::AomVarianceFnPtr;
use crate::aom::av1::common::av1_common_int::{Av1Common, CommonModeInfoParams};
use crate::aom::av1::common::blockd::{is_intrabc_block, Buf2d, MacroBlockD, MAX_MB_PLANE};
use crate::aom::av1::common::entropymv::{MV_JOINTS, MV_LOW, MV_MAX, MV_UPP};
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::mv::*;
use crate::aom::av1::common::quant_common::QINDEX_BITS;
use crate::aom::av1::common::reconinter::{av1_setup_pre_planes, AOM_INTERP_EXTEND};
use crate::aom::av1::common::scale::av1_is_scaled;

/// `MAX_MVSEARCH_STEPS`
pub(crate) const MAX_MVSEARCH_STEPS: i32 = 11;
// Enable the use of motion vector in range [-1023, 1023].
/// `MAX_FULL_PEL_VAL`
pub(crate) const MAX_FULL_PEL_VAL: i32 = (1 << (MAX_MVSEARCH_STEPS - 1)) - 1;
// Maximum size of the first step in full pel units
/// `MAX_FIRST_STEP`
pub(crate) const MAX_FIRST_STEP: i32 = 1 << (MAX_MVSEARCH_STEPS - 1);

pub(crate) const SEARCH_RANGE_8P: i32 = 3;
pub(crate) const SEARCH_GRID_STRIDE_8P: i32 = 2 * SEARCH_RANGE_8P + 1;
pub(crate) const SEARCH_GRID_CENTER_8P: i32 =
    SEARCH_RANGE_8P * SEARCH_GRID_STRIDE_8P + SEARCH_RANGE_8P;

// motion search site
/// `search_site`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SearchSite {
    pub(crate) mv: FullpelMv,
    pub(crate) offset: i32,
}

/// `search_site_config`
#[derive(Debug, Clone)]
pub(crate) struct SearchSiteConfig {
    pub(crate) site: [[SearchSite; 16 + 1]; MAX_MVSEARCH_STEPS as usize * 2],
    // Number of search steps.
    pub(crate) num_search_steps: i32,
    pub(crate) searches_per_step: [i32; MAX_MVSEARCH_STEPS as usize * 2],
    pub(crate) radius: [i32; MAX_MVSEARCH_STEPS as usize * 2],
    pub(crate) stride: i32,
}

impl Default for SearchSiteConfig {
    fn default() -> Self {
        SearchSiteConfig {
            site: [[SearchSite::default(); 17]; MAX_MVSEARCH_STEPS as usize * 2],
            num_search_steps: 0,
            searches_per_step: [0; MAX_MVSEARCH_STEPS as usize * 2],
            radius: [0; MAX_MVSEARCH_STEPS as usize * 2],
            stride: 0,
        }
    }
}

/// `SS_CFG_SRC`
pub(crate) const SS_CFG_SRC: usize = 0;
/// `SS_CFG_LOOKAHEAD`
pub(crate) const SS_CFG_LOOKAHEAD: usize = 1;
/// `SS_CFG_FPF`
pub(crate) const SS_CFG_FPF: usize = 2;
/// `SS_CFG_TOTAL`
pub(crate) const SS_CFG_TOTAL: usize = 3;

// (SEARCH_METHODS)
// Search 8-points in the radius grid around center, up to 11 search stages.
pub(crate) const DIAMOND: i32 = 0;
// Search 12-points in the radius/tan_radius grid around center,
// up to 15 search stages.
pub(crate) const NSTEP: i32 = 1;
// Search 8-points in the radius grid around center, up to 16 search stages.
pub(crate) const NSTEP_8PT: i32 = 2;
// Search 8-points in the radius grid around center, upto 11 search stages
// with clamping of search radius.
pub(crate) const CLAMPED_DIAMOND: i32 = 3;
// Search maximum 8-points in the radius grid around center,
// up to 11 search stages. First stage consists of 8 search points
// and the rest with 6 search points each in hex shape.
pub(crate) const HEX: i32 = 4;
// Search maximum 8-points in the radius grid around center,
// up to 11 search stages. First stage consists of 4 search
// points and the rest with 8 search points each.
pub(crate) const BIGDIA: i32 = 5;
// Search 8-points in the square grid around center, up to 11 search stages.
pub(crate) const SQUARE: i32 = 6;
// HEX search with up to 2 stages.
pub(crate) const FAST_HEX: i32 = 7;
// BIGDIA search with up to 2 stages.
pub(crate) const FAST_DIAMOND: i32 = 8;
// BIGDIA search with up to 3 stages.
pub(crate) const FAST_BIGDIA: i32 = 9;
// BIGDIA search with up to 1 stage.
pub(crate) const VFAST_DIAMOND: i32 = 10;
// Total number of search methods.
pub(crate) const NUM_SEARCH_METHODS: i32 = 11;
// Number of distinct search methods.
pub(crate) const NUM_DISTINCT_SEARCH_METHODS: i32 = SQUARE + 1;

// =============================================================================
//  Subpixel Motion Search
// =============================================================================
// (SUBPEL_FORCE_STOP)
pub(crate) const EIGHTH_PEL: i32 = 0;
pub(crate) const QUARTER_PEL: i32 = 1;
pub(crate) const HALF_PEL: i32 = 2;
pub(crate) const FULL_PEL: i32 = 3;

pub(crate) const MAX_PATTERN_SCALES: usize = 11;
pub(crate) const MAX_PATTERN_CANDIDATES: usize = 8; // max number of candidates per scale
pub(crate) const PATTERN_CANDIDATES_REF: i32 = 3; // number of refinement candidates

/// Translation of `get_offset_from_fullmv()` (mcomp.h).
#[inline]
pub(crate) fn get_offset_from_fullmv(mv: &FullpelMv, stride: i32) -> i32 {
    mv.row as i32 * stride + mv.col as i32
}

#[inline]
const fn fm(row: i32, col: i32) -> FullpelMv {
    FullpelMv {
        row: row as i16,
        col: col as i16,
    }
}

// Search site initialization for DIAMOND / CLAMPED_DIAMOND search methods.
// level = 0: DIAMOND, level = 1: CLAMPED_DIAMOND.
/// Translation of `av1_init_dsmotion_compensation()`.
pub(crate) fn av1_init_dsmotion_compensation(cfg: &mut SearchSiteConfig, stride: i32, level: i32) {
    let mut num_search_steps = 0;
    let mut stage_index = MAX_MVSEARCH_STEPS as isize - 1;
    cfg.site[stage_index as usize][0].mv = fm(0, 0);
    cfg.site[stage_index as usize][0].offset = 0;
    cfg.stride = stride;

    // Choose the initial step size depending on level.
    let first_step = if level > 0 {
        MAX_FIRST_STEP / 4
    } else {
        MAX_FIRST_STEP
    };

    let mut radius = first_step;
    while radius > 0 {
        let num_search_pts = 8;

        let search_site_mvs: [FullpelMv; 9] = [
            fm(0, 0),
            fm(-radius, 0),
            fm(radius, 0),
            fm(0, -radius),
            fm(0, radius),
            fm(-radius, -radius),
            fm(radius, radius),
            fm(-radius, radius),
            fm(radius, -radius),
        ];

        let si = stage_index as usize;
        for i in 0..=num_search_pts {
            let site = &mut cfg.site[si][i];
            site.mv = search_site_mvs[i];
            site.offset = get_offset_from_fullmv(&site.mv, stride);
        }
        cfg.searches_per_step[si] = num_search_pts as i32;
        cfg.radius[si] = radius;
        // Update the search radius based on level.
        if level == 0 || (stage_index < 9 && level != 0) {
            radius /= 2;
        }
        stage_index -= 1;
        num_search_steps += 1;
    }
    cfg.num_search_steps = num_search_steps;
}

/// Translation of `av1_init_motion_fpf()`.
pub(crate) fn av1_init_motion_fpf(cfg: &mut SearchSiteConfig, stride: i32) {
    let mut num_search_steps = 0;
    let mut stage_index = MAX_MVSEARCH_STEPS as isize - 1;

    cfg.site[stage_index as usize][0].mv = fm(0, 0);
    cfg.site[stage_index as usize][0].offset = 0;
    cfg.stride = stride;

    let mut radius = MAX_FIRST_STEP;
    while radius > 0 {
        // Generate offsets for 8 search sites per step.
        let tan_radius = ((0.41 * radius as f64) as i32).max(1);
        let mut num_search_pts = 12;
        if radius == 1 {
            num_search_pts = 8;
        }

        let search_site_mvs: [FullpelMv; 13] = [
            fm(0, 0),
            fm(-radius, 0),
            fm(radius, 0),
            fm(0, -radius),
            fm(0, radius),
            fm(-radius, -tan_radius),
            fm(radius, tan_radius),
            fm(-tan_radius, radius),
            fm(tan_radius, -radius),
            fm(-radius, tan_radius),
            fm(radius, -tan_radius),
            fm(tan_radius, radius),
            fm(-tan_radius, -radius),
        ];

        let si = stage_index as usize;
        for i in 0..=num_search_pts {
            let site = &mut cfg.site[si][i];
            site.mv = search_site_mvs[i];
            site.offset = get_offset_from_fullmv(&site.mv, stride);
        }
        cfg.searches_per_step[si] = num_search_pts as i32;
        cfg.radius[si] = radius;
        stage_index -= 1;
        num_search_steps += 1;
        radius /= 2;
    }
    cfg.num_search_steps = num_search_steps;
}

// Search site initialization for NSTEP / NSTEP_8PT search methods.
// level = 0: NSTEP, level = 1: NSTEP_8PT.
/// Translation of `av1_init_motion_compensation_nstep()`.
pub(crate) fn av1_init_motion_compensation_nstep(
    cfg: &mut SearchSiteConfig,
    stride: i32,
    level: i32,
) {
    let mut num_search_steps = 0;
    cfg.stride = stride;
    let mut radius = 1;
    let num_stages = if level > 0 { 16 } else { 15 };
    for stage_index in 0..num_stages {
        let mut tan_radius = ((0.41 * radius as f64) as i32).max(1);
        let mut num_search_pts = 12;
        if radius <= 5 || level > 0 {
            tan_radius = radius;
            num_search_pts = 8;
        }
        let search_site_mvs: [FullpelMv; 13] = [
            fm(0, 0),
            fm(-radius, 0),
            fm(radius, 0),
            fm(0, -radius),
            fm(0, radius),
            fm(-radius, -tan_radius),
            fm(radius, tan_radius),
            fm(-tan_radius, radius),
            fm(tan_radius, -radius),
            fm(-radius, tan_radius),
            fm(radius, -tan_radius),
            fm(tan_radius, radius),
            fm(-tan_radius, -radius),
        ];
        for i in 0..=num_search_pts {
            let site = &mut cfg.site[stage_index][i];
            site.mv = search_site_mvs[i];
            site.offset = get_offset_from_fullmv(&site.mv, stride);
        }
        cfg.searches_per_step[stage_index] = num_search_pts as i32;
        cfg.radius[stage_index] = radius;
        num_search_steps += 1;
        if stage_index < 12 {
            radius = (radius as f64 * 1.5 + 0.5).max((radius + 1) as f64) as i32;
        }
    }
    cfg.num_search_steps = num_search_steps;
}

/// Fills a pattern search's sites (`av1_init_motion_compensation_bigdia()`,
/// `_square()` and `_hex()`, which differ in their candidates and in
/// filling all `MAX_PATTERN_CANDIDATES` or only the scale's candidates).
fn init_pattern_sites(
    cfg: &mut SearchSiteConfig,
    stride: i32,
    num_candidates: &[i32; MAX_PATTERN_SCALES],
    candidates: &[[FullpelMv; MAX_PATTERN_CANDIDATES]; MAX_PATTERN_SCALES],
    all_candidates: bool,
) {
    cfg.stride = stride;
    let mut radius = 1;
    for i in 0..MAX_PATTERN_SCALES {
        cfg.searches_per_step[i] = num_candidates[i];
        cfg.radius[i] = radius;
        let n = if all_candidates {
            MAX_PATTERN_CANDIDATES
        } else {
            num_candidates[i] as usize
        };
        for j in 0..n {
            let site = &mut cfg.site[i][j];
            site.mv = candidates[i][j];
            site.offset = get_offset_from_fullmv(&site.mv, stride);
        }
        radius *= 2;
    }
    cfg.num_search_steps = MAX_PATTERN_SCALES as i32;
}

/// The diamond pattern's candidates at a scale of `s`.
const fn bigdia_scale(s: i32) -> [FullpelMv; MAX_PATTERN_CANDIDATES] {
    [
        fm(-s, -s),
        fm(0, -2 * s),
        fm(s, -s),
        fm(2 * s, 0),
        fm(s, s),
        fm(0, 2 * s),
        fm(-s, s),
        fm(-2 * s, 0),
    ]
}

/// The square pattern's candidates at a scale of `s`.
const fn square_scale(s: i32) -> [FullpelMv; MAX_PATTERN_CANDIDATES] {
    [
        fm(-s, -s),
        fm(0, -s),
        fm(s, -s),
        fm(s, 0),
        fm(s, s),
        fm(0, s),
        fm(-s, s),
        fm(-s, 0),
    ]
}

/// The hex pattern's candidates at a scale of `s`.
const fn hex_scale(s: i32) -> [FullpelMv; MAX_PATTERN_CANDIDATES] {
    [
        fm(-s, -2 * s),
        fm(s, -2 * s),
        fm(2 * s, 0),
        fm(s, 2 * s),
        fm(-s, 2 * s),
        fm(-2 * s, 0),
        fm(0, 0),
        fm(0, 0),
    ]
}

// Search site initialization for BIGDIA / FAST_BIGDIA / FAST_DIAMOND
// search methods.
/// Translation of `av1_init_motion_compensation_bigdia()`.
pub(crate) fn av1_init_motion_compensation_bigdia(
    cfg: &mut SearchSiteConfig,
    stride: i32,
    _level: i32,
) {
    // First scale has 4-closest points, the rest have 8 points in diamond
    // shape at increasing scales
    const BIGDIA_NUM_CANDIDATES: [i32; MAX_PATTERN_SCALES] = [4, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8];
    // BIGDIA search method candidates.
    // Note that the largest candidate step at each scale is 2^scale
    const SITE_CANDIDATES: [[FullpelMv; MAX_PATTERN_CANDIDATES]; MAX_PATTERN_SCALES] = [
        [
            fm(0, -1),
            fm(1, 0),
            fm(0, 1),
            fm(-1, 0),
            fm(0, 0),
            fm(0, 0),
            fm(0, 0),
            fm(0, 0),
        ],
        bigdia_scale(1),
        bigdia_scale(2),
        bigdia_scale(4),
        bigdia_scale(8),
        bigdia_scale(16),
        bigdia_scale(32),
        bigdia_scale(64),
        bigdia_scale(128),
        bigdia_scale(256),
        bigdia_scale(512),
    ];
    init_pattern_sites(cfg, stride, &BIGDIA_NUM_CANDIDATES, &SITE_CANDIDATES, true);
}

// Search site initialization for SQUARE search method.
/// Translation of `av1_init_motion_compensation_square()`.
pub(crate) fn av1_init_motion_compensation_square(
    cfg: &mut SearchSiteConfig,
    stride: i32,
    _level: i32,
) {
    // All scales have 8 closest points in square shape.
    const SQUARE_NUM_CANDIDATES: [i32; MAX_PATTERN_SCALES] = [8; MAX_PATTERN_SCALES];
    // Square search method candidates.
    // Note that the largest candidate step at each scale is 2^scale.
    const SQUARE_CANDIDATES: [[FullpelMv; MAX_PATTERN_CANDIDATES]; MAX_PATTERN_SCALES] = [
        square_scale(1),
        square_scale(2),
        square_scale(4),
        square_scale(8),
        square_scale(16),
        square_scale(32),
        square_scale(64),
        square_scale(128),
        square_scale(256),
        square_scale(512),
        square_scale(1024),
    ];
    init_pattern_sites(
        cfg,
        stride,
        &SQUARE_NUM_CANDIDATES,
        &SQUARE_CANDIDATES,
        true,
    );
}

// Search site initialization for HEX / FAST_HEX search methods.
/// Translation of `av1_init_motion_compensation_hex()`.
pub(crate) fn av1_init_motion_compensation_hex(
    cfg: &mut SearchSiteConfig,
    stride: i32,
    _level: i32,
) {
    // First scale has 8-closest points, the rest have 6 points in hex shape
    // at increasing scales.
    const HEX_NUM_CANDIDATES: [i32; MAX_PATTERN_SCALES] = [8, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6];
    // Note that the largest candidate step at each scale is 2^scale.
    const HEX_CANDIDATES: [[FullpelMv; MAX_PATTERN_CANDIDATES]; MAX_PATTERN_SCALES] = [
        square_scale(1),
        hex_scale(1),
        hex_scale(2),
        hex_scale(4),
        hex_scale(8),
        hex_scale(16),
        hex_scale(32),
        hex_scale(64),
        hex_scale(128),
        hex_scale(256),
        hex_scale(512),
    ];
    init_pattern_sites(cfg, stride, &HEX_NUM_CANDIDATES, &HEX_CANDIDATES, false);
}

/// `av1_init_motion_compensation[]`: the search site initialization of
/// search method `method`.
pub(crate) fn av1_init_motion_compensation(
    method: i32,
    cfg: &mut SearchSiteConfig,
    stride: i32,
    level: i32,
) {
    match method {
        DIAMOND | CLAMPED_DIAMOND => av1_init_dsmotion_compensation(cfg, stride, level),
        NSTEP | NSTEP_8PT => av1_init_motion_compensation_nstep(cfg, stride, level),
        HEX => av1_init_motion_compensation_hex(cfg, stride, level),
        BIGDIA => av1_init_motion_compensation_bigdia(cfg, stride, level),
        _ => av1_init_motion_compensation_square(cfg, stride, level),
    }
}

// =============================================================================
//  Cost functions
// =============================================================================
// (MV_COST_TYPE)
pub(crate) const MV_COST_ENTROPY: i32 = 0; // Use the entropy rate of the mv as the cost
pub(crate) const MV_COST_L1_LOWRES: i32 = 1; // Use the l1 norm of the mv as the cost (<480p)
pub(crate) const MV_COST_L1_MIDRES: i32 = 2; // Use the l1 norm of the mv as the cost (>=480p)
pub(crate) const MV_COST_L1_HDRES: i32 = 3; // Use the l1 norm of the mv as the cost (>=720p)
pub(crate) const MV_COST_NONE: i32 = 4; // Use 0 as as cost irrespective of the current mv

/// The cost tables of a motion vector component: indexed by `MV_MAX + v`
/// (upstream's pointer to the middle of the table). A component outside
/// the table (which the search ranges rule out) costs as much as the
/// table's end.
#[inline]
fn comp_cost(table: &[i32], v: i32) -> i32 {
    let i = (MV_MAX + v.clamp(-MV_MAX, MV_MAX)) as usize;
    table.get(i).copied().unwrap_or(0)
}

/// `MV_COST_PARAMS`: the tables are `None` when `x->mv_costs` is not
/// allocated (all-intra encoding).
#[derive(Clone, Copy)]
pub(crate) struct MvCostParams<'a> {
    // The reference mv used to compute the mv cost
    pub(crate) ref_mv: Mv,
    pub(crate) full_ref_mv: FullpelMv,
    pub(crate) mv_cost_type: i32,
    pub(crate) mvjcost: Option<&'a [i32; MV_JOINTS as usize]>,
    pub(crate) mvcost: Option<[&'a [i32]; 2]>,
    pub(crate) error_per_bit: i32,
    // A multiplier used to convert rate to sad cost
    pub(crate) sad_per_bit: i32,
}

#[inline]
fn init_mv_cost_params<'a>(
    mv_costs: Option<&'a MvCosts>,
    ref_mv: &Mv,
    errorperbit: i32,
    sadperbit: i32,
) -> MvCostParams<'a> {
    // For allintra encoding mode, 'mv_costs' is not allocated. Hence, the
    // population of mvjcost and mvcost are avoided. In case of IntraBC, these
    // values are populated from 'dv_costs' in av1_set_ms_to_intra_mode().
    MvCostParams {
        ref_mv: *ref_mv,
        full_ref_mv: get_fullmv_from_mv(ref_mv),
        mv_cost_type: MV_COST_ENTROPY,
        error_per_bit: errorperbit,
        sad_per_bit: sadperbit,
        mvjcost: mv_costs.map(|c| &c.nmv_joint_cost),
        mvcost: mv_costs.map(|c| [c.mv_cost_stack(0), c.mv_cost_stack(1)]),
    }
}

// Returns the rate of encoding the current motion vector based on the
// joint_cost and comp_cost. joint_costs covers the cost of transmitting
// JOINT_MV, and comp_cost covers the cost of transmitting the actual motion
// vector.
#[inline]
fn mv_cost(mv: &Mv, joint_cost: &[i32; MV_JOINTS as usize], comp_costs: &[&[i32]; 2]) -> i32 {
    joint_cost[av1_get_mv_joint(mv) as usize]
        + comp_cost(comp_costs[0], mv.row as i32)
        + comp_cost(comp_costs[1], mv.col as i32)
}

// Returns the cost of encoding the motion vector diff := *mv - *ref. The cost
// is defined as the rate required to encode diff * weight, rounded to the
// nearest 2 ** 7.
// This is NOT used during motion compensation.
pub(crate) fn av1_mv_bit_cost(
    mv: &Mv,
    ref_mv: &Mv,
    mvjcost: &[i32; MV_JOINTS as usize],
    mvcost: &[&[i32]; 2],
    weight: i32,
) -> i32 {
    let diff = Mv {
        row: mv.row.wrapping_sub(ref_mv.row),
        col: mv.col.wrapping_sub(ref_mv.col),
    };
    round_power_of_two(mv_cost(&diff, mvjcost, mvcost).wrapping_mul(weight), 7)
}

#[inline]
fn round_power_of_two(value: i32, n: i32) -> i32 {
    (value + ((1 << n) >> 1)) >> n
}

// Returns the cost of using the current mv during the motion search. This is
// used when var is used as the error metric.
const PIXEL_TRANSFORM_ERROR_SCALE: i32 = 4;
// TODO(any): Adaptively adjust the regularization strength based on image size
// and motion activity instead of using hard-coded values. It seems like we
// roughly half the lambda for each increase in resolution
// These are multiplier used to perform regularization in motion compensation
// when x->mv_cost_type is set to MV_COST_L1.
// LOWRES
const SSE_LAMBDA_LOWRES: i32 = 2; // Used by mv_cost_err_fn
const SAD_LAMBDA_LOWRES: i32 = 32; // Used by mvsad_err_cost during full pixel search
                                   // MIDRES
const SSE_LAMBDA_MIDRES: i32 = 0; // Used by mv_cost_err_fn
const SAD_LAMBDA_MIDRES: i32 = 15; // Used by mvsad_err_cost during full pixel search
                                   // HDRES
const SSE_LAMBDA_HDRES: i32 = 1; // Used by mv_cost_err_fn
const SAD_LAMBDA_HDRES: i32 = 8; // Used by mvsad_err_cost during full pixel search

#[inline]
fn mv_err_cost(
    mv: &Mv,
    ref_mv: &Mv,
    mvjcost: Option<&[i32; MV_JOINTS as usize]>,
    mvcost: Option<&[&[i32]; 2]>,
    error_per_bit: i32,
    mv_cost_type: i32,
) -> i32 {
    let diff = Mv {
        row: mv.row.wrapping_sub(ref_mv.row),
        col: mv.col.wrapping_sub(ref_mv.col),
    };
    let abs_diff = ((diff.row as i32).abs(), (diff.col as i32).abs());

    match mv_cost_type {
        MV_COST_ENTROPY => {
            if let (Some(mvjcost), Some(mvcost)) = (mvjcost, mvcost) {
                let n =
                    RDDIV_BITS + AV1_PROB_COST_SHIFT - RD_EPB_SHIFT + PIXEL_TRANSFORM_ERROR_SCALE;
                return ((mv_cost(&diff, mvjcost, mvcost) as i64 * error_per_bit as i64
                    + ((1i64 << n) >> 1))
                    >> n) as i32;
            }
            0
        }
        MV_COST_L1_LOWRES => (SSE_LAMBDA_LOWRES * (abs_diff.0 + abs_diff.1)) >> 3,
        MV_COST_L1_MIDRES => (SSE_LAMBDA_MIDRES * (abs_diff.0 + abs_diff.1)) >> 3,
        MV_COST_L1_HDRES => (SSE_LAMBDA_HDRES * (abs_diff.0 + abs_diff.1)) >> 3,
        _ => 0,
    }
}

#[inline]
fn mv_err_cost_(mv: &Mv, mv_cost_params: &MvCostParams) -> i32 {
    if mv_cost_params.mv_cost_type == MV_COST_NONE {
        return 0;
    }
    mv_err_cost(
        mv,
        &mv_cost_params.ref_mv,
        mv_cost_params.mvjcost,
        mv_cost_params.mvcost.as_ref(),
        mv_cost_params.error_per_bit,
        mv_cost_params.mv_cost_type,
    )
}

// Returns the cost of using the current mv during the motion search. This is
// only used during full pixel motion search when sad is used as the error
// metric
#[inline]
fn mvsad_err_cost(
    mv: &FullpelMv,
    ref_mv: &FullpelMv,
    mvjcost: Option<&[i32; MV_JOINTS as usize]>,
    mvcost: Option<&[&[i32]; 2]>,
    sad_per_bit: i32,
    mv_cost_type: i32,
) -> i32 {
    let diff = Mv {
        row: get_mv_subpel(mv.row as i32 - ref_mv.row as i32) as i16,
        col: get_mv_subpel(mv.col as i32 - ref_mv.col as i32) as i16,
    };

    match mv_cost_type {
        MV_COST_ENTROPY => {
            let (Some(mvjcost), Some(mvcost)) = (mvjcost, mvcost) else {
                return 0;
            };
            let c = (mv_cost(&diff, mvjcost, mvcost) as u32).wrapping_mul(sad_per_bit as u32);
            (c.wrapping_add((1u32 << AV1_PROB_COST_SHIFT) >> 1) >> AV1_PROB_COST_SHIFT) as i32
        }
        MV_COST_L1_LOWRES => {
            (SAD_LAMBDA_LOWRES * ((diff.row as i32).abs() + (diff.col as i32).abs())) >> 3
        }
        MV_COST_L1_MIDRES => {
            (SAD_LAMBDA_MIDRES * ((diff.row as i32).abs() + (diff.col as i32).abs())) >> 3
        }
        MV_COST_L1_HDRES => {
            (SAD_LAMBDA_HDRES * ((diff.row as i32).abs() + (diff.col as i32).abs())) >> 3
        }
        _ => 0,
    }
}

#[inline]
fn mvsad_err_cost_(mv: &FullpelMv, mv_cost_params: &MvCostParams) -> i32 {
    mvsad_err_cost(
        mv,
        &mv_cost_params.full_ref_mv,
        mv_cost_params.mvjcost,
        mv_cost_params.mvcost.as_ref(),
        mv_cost_params.sad_per_bit,
        mv_cost_params.mv_cost_type,
    )
}

// =============================================================================
//  Motion Search
// =============================================================================
/// `MSBuffers`: the reference and source planes, with the frame buffers
/// they point into. (The compound predictions and masks, and the OBMC
/// buffers, are never set by the real-time encoder: `second_pred` and
/// `mask` are NULL.)
#[derive(Clone, Copy)]
pub(crate) struct MsBuffers<'a> {
    // The reference buffer
    pub(crate) ref_: Buf2d,
    pub(crate) ref_buf: &'a [u16],
    // The source and predictors/mask used by translational search
    pub(crate) src: Buf2d,
    pub(crate) src_buf: &'a [u16],
}

/// Translation of `init_ms_buffers()`: the reference is the frame of
/// `x->e_mbd.plane[0].pre[0]`, the source `cpi->source`.
#[inline]
fn init_ms_buffers<'a>(cpi: &'a Av1Comp, x: &'a Macroblock) -> MsBuffers<'a> {
    let ref_ = x.e_mbd.plane[0].pre[0];
    MsBuffers {
        ref_,
        ref_buf: &cpi.common.buffer_pool.frame_bufs[ref_.frame_buf]
            .buf
            .buffer_alloc,
        src: x.plane[0].src,
        src_buf: &x.source_alloc,
    }
}

/// The offset of `buf` moved by `delta` samples.
#[inline]
fn off(buf: usize, delta: i32) -> usize {
    (buf as i64 + delta as i64) as usize
}

// =============================================================================
//  Fullpixel Motion Search
// =============================================================================
// This struct holds fullpixel motion search parameters that should be constant
// during the search
/// `FULLPEL_MOTION_SEARCH_PARAMS` (`sdf`, `sdx4df` and `sdx3df` are the
/// table's plain or downsampled sums of absolute differences, as
/// `use_downsampled_sad` says; the mesh patterns are not kept: the mesh
/// search is never run)
#[derive(Clone, Copy)]
pub(crate) struct FullpelMotionSearchParams<'a> {
    pub(crate) bsize: BlockSize,
    // A function pointer to the simd function for fast computation
    pub(crate) vfp: &'a AomVarianceFnPtr,

    pub(crate) ms_buffers: MsBuffers<'a>,

    // WARNING: search_method should be regarded as a private variable and should
    // not be modified directly so it is in sync with search_sites. To modify it,
    // use av1_set_mv_search_method.
    pub(crate) search_method: i32,
    pub(crate) search_sites: &'a SearchSiteConfig,
    pub(crate) mv_limits: FullMvLimits,

    // Sets mesh search unless it got pruned by prune_mesh_search.
    pub(crate) run_mesh_search: i32,
    // Disables mesh search if the best_mv after a normal search if close to the
    // start_mv.
    pub(crate) prune_mesh_search: i32,
    // mv diff threshold to enable prune_mesh_search
    pub(crate) mesh_search_mv_diff_threshold: i32,
    // Forces mesh search if the residue variance is higher than the threshold.
    pub(crate) force_mesh_thresh: i32,

    // Use maximum search interval of 4 if true. This helps motion search to find
    // the best motion vector for screen content types.
    pub(crate) fine_search_interval: i32,

    pub(crate) is_intra_mode: i32,

    pub(crate) fast_obmc_search: i32,

    // For calculating mv cost
    pub(crate) mv_cost_params: MvCostParams<'a>,

    // Stores the function used to compute the sad. This can be different from the
    // sdf in vfp (e.g. downsampled sad and not sad) to allow speed up.
    pub(crate) use_downsampled_sad: bool,
}

impl FullpelMotionSearchParams<'_> {
    /// `ms_params->sdf(src, src_stride, ref, ref_stride)` on the source
    /// block and the reference at offset `ref_off`.
    #[inline]
    fn sdf(&self, ref_off: usize) -> u32 {
        let b = &self.ms_buffers;
        let f = if self.use_downsampled_sad {
            AomVarianceFnPtr::sdsf
        } else {
            AomVarianceFnPtr::sdf
        };
        f(
            self.vfp,
            self.bsize,
            b.src_buf,
            b.src.buf,
            b.src.stride as usize,
            b.ref_buf,
            ref_off,
            b.ref_.stride as usize,
        )
    }

    /// `ms_params->sdx4df()` (and `sdx3df()`, the same four sums).
    #[inline]
    fn sdx4df(&self, ref_offs: &[usize; 4], sads: &mut [u32; 4]) {
        let b = &self.ms_buffers;
        let f = if self.use_downsampled_sad {
            AomVarianceFnPtr::sdsx4df
        } else {
            AomVarianceFnPtr::sdx4df
        };
        f(
            self.vfp,
            self.bsize,
            b.src_buf,
            b.src.buf,
            b.src.stride as usize,
            b.ref_buf,
            ref_offs,
            b.ref_.stride as usize,
            sads,
        );
    }
}

/// `get_faster_search_method()`
fn get_faster_search_method(search_method: i32) -> i32 {
    // Note on search method's accuracy:
    //  1. NSTEP
    //  2. DIAMOND
    //  3. BIGDIA \approx SQUARE
    //  4. HEX.
    //  5. FAST_HEX \approx FAST_DIAMOND
    match search_method {
        NSTEP => DIAMOND,
        NSTEP_8PT => DIAMOND,
        DIAMOND => BIGDIA,
        CLAMPED_DIAMOND => BIGDIA,
        BIGDIA => HEX,
        SQUARE => HEX,
        HEX => FAST_HEX,
        FAST_HEX => FAST_HEX,
        FAST_DIAMOND => VFAST_DIAMOND,
        FAST_BIGDIA => FAST_BIGDIA,
        VFAST_DIAMOND => VFAST_DIAMOND,
        _ => {
            debug_assert!(false, "Invalid search method!");
            DIAMOND
        }
    }
}

// Array to inform which all search methods are having
// same candidates and different in number of search steps.
const SEARCH_METHOD_LOOKUP: [i32; NUM_SEARCH_METHODS as usize] = [
    DIAMOND,         // DIAMOND
    NSTEP,           // NSTEP
    NSTEP_8PT,       // NSTEP_8PT
    CLAMPED_DIAMOND, // CLAMPED_DIAMOND
    HEX,             // HEX
    BIGDIA,          // BIGDIA
    SQUARE,          // SQUARE
    HEX,             // FAST_HEX
    BIGDIA,          // FAST_DIAMOND
    BIGDIA,          // FAST_BIGDIA
    BIGDIA,          // VFAST_DIAMOND
];

/// Translation of `av1_make_default_fullpel_ms_params()`. `Err` for the
/// block size dependent faster search methods whose search sites have a
/// different stride (`x->search_site_cfg_buf` is never refreshed by the
/// real-time speed features).
pub(crate) fn av1_make_default_fullpel_ms_params<'a>(
    cpi: &'a Av1Comp,
    x: &'a Macroblock,
    bsize: BlockSize,
    ref_mv: &Mv,
    search_sites: &'a [SearchSiteConfig; NUM_DISTINCT_SEARCH_METHODS as usize],
    fine_search_interval: i32,
) -> Result<FullpelMotionSearchParams<'a>, ()> {
    let mv_sf = &cpi.sf.mv_sf;

    // High level params
    let ms_buffers = init_ms_buffers(cpi, x);

    let mut search_method = mv_sf.search_method;
    let sf_blk_search_method = mv_sf.use_bsize_dependent_search_method;
    let min_dim = BLOCK_SIZE_WIDE[bsize as usize].min(BLOCK_SIZE_HIGH[bsize as usize]) as i32;
    let qband = x.qindex >> (QINDEX_BITS - 2);
    let use_faster_search_method = (sf_blk_search_method == 1 && min_dim >= 32)
        || (sf_blk_search_method >= 2
            && min_dim >= 16
            && x.content_state_sb.source_sad_nonrd <= K_MED_SAD
            && qband < 3);

    if use_faster_search_method {
        search_method = get_faster_search_method(search_method);

        // We might need to update the search site config since search_method
        // is changed here.
        let ref_stride = ms_buffers.ref_.stride;
        if ref_stride != search_sites[search_method as usize].stride {
            // (av1_refresh_search_site_config() into x->search_site_cfg_buf)
            return Err(());
        }
    }

    // (av1_set_mv_search_method())
    let search_sites = &search_sites[SEARCH_METHOD_LOOKUP[search_method as usize] as usize];

    let use_downsampled_sad =
        mv_sf.use_downsampled_sad != 0 && BLOCK_SIZE_HIGH[bsize as usize] >= 16;

    let mut mv_limits = x.mv_limits;
    av1_set_mv_search_range(&mut mv_limits, ref_mv);

    Ok(FullpelMotionSearchParams {
        bsize,
        vfp: &cpi.ppi.fn_ptr,
        ms_buffers,
        search_method,
        search_sites,
        mv_limits,
        run_mesh_search: 0,
        prune_mesh_search: (mv_sf.prune_mesh_search == PRUNE_MESH_SEARCH_LVL_2) as i32,
        mesh_search_mv_diff_threshold: 4,
        force_mesh_thresh: mv_sf.exhaustive_searches_thresh,
        fine_search_interval,
        is_intra_mode: 0,
        fast_obmc_search: mv_sf.obmc_full_pixel_search_level,
        // Mvcost params
        mv_cost_params: init_mv_cost_params(
            x.mv_costs.as_deref(),
            ref_mv,
            x.errorperbit,
            x.sadperbit,
        ),
        use_downsampled_sad,
    })
}

/// Translation of `av1_set_mv_search_range()`.
pub(crate) fn av1_set_mv_search_range(mv_limits: &mut FullMvLimits, mv: &Mv) {
    // Calculate the outermost full-pixel MVs which are inside the limits set by
    // av1_set_subpel_mv_search_range().
    //
    // The subpel limits are simply mv->col +/- 8*MAX_FULL_PEL_VAL, and similar
    // for mv->row. We can then divide by 8 to find the fullpel MV limits. But
    // we have to be careful about the rounding. We want these bounds to be
    // at least as tight as the subpel limits, which means that we must round
    // the minimum values up and the maximum values down when dividing.
    let mut col_min = ((mv.col as i32 + 7) >> 3) - MAX_FULL_PEL_VAL;
    let mut row_min = ((mv.row as i32 + 7) >> 3) - MAX_FULL_PEL_VAL;
    let mut col_max = (mv.col as i32 >> 3) + MAX_FULL_PEL_VAL;
    let mut row_max = (mv.row as i32 >> 3) + MAX_FULL_PEL_VAL;

    col_min = col_min.max((MV_LOW >> 3) + 1);
    row_min = row_min.max((MV_LOW >> 3) + 1);
    col_max = col_max.min((MV_UPP >> 3) - 1);
    row_max = row_max.min((MV_UPP >> 3) - 1);

    // Get intersection of UMV window and valid MV window to reduce # of checks
    // in diamond search.
    if mv_limits.col_min < col_min {
        mv_limits.col_min = col_min;
    }
    if mv_limits.col_max > col_max {
        mv_limits.col_max = col_max;
    }
    if mv_limits.row_min < row_min {
        mv_limits.row_min = row_min;
    }
    if mv_limits.row_max > row_max {
        mv_limits.row_max = row_max;
    }
}

// Set up limit values for MV components.
// Mv beyond the range do not produce new/different prediction block.
#[inline]
fn av1_set_mv_row_limits(
    mi_params: &CommonModeInfoParams,
    mv_limits: &mut FullMvLimits,
    mi_row: i32,
    mi_height: i32,
    border: i32,
) {
    let min1 = -(mi_row * MI_SIZE + border - 2 * AOM_INTERP_EXTEND);
    let min2 = -(((mi_row + mi_height) * MI_SIZE) + 2 * AOM_INTERP_EXTEND);
    mv_limits.row_min = min1.max(min2);
    let max1 = (mi_params.mi_rows - mi_row - mi_height) * MI_SIZE + border - 2 * AOM_INTERP_EXTEND;
    let max2 = (mi_params.mi_rows - mi_row) * MI_SIZE + 2 * AOM_INTERP_EXTEND;
    mv_limits.row_max = max1.min(max2);
}

#[inline]
fn av1_set_mv_col_limits(
    mi_params: &CommonModeInfoParams,
    mv_limits: &mut FullMvLimits,
    mi_col: i32,
    mi_width: i32,
    border: i32,
) {
    let min1 = -(mi_col * MI_SIZE + border - 2 * AOM_INTERP_EXTEND);
    let min2 = -(((mi_col + mi_width) * MI_SIZE) + 2 * AOM_INTERP_EXTEND);
    mv_limits.col_min = min1.max(min2);
    let max1 = (mi_params.mi_cols - mi_col - mi_width) * MI_SIZE + border - 2 * AOM_INTERP_EXTEND;
    let max2 = (mi_params.mi_cols - mi_col) * MI_SIZE + 2 * AOM_INTERP_EXTEND;
    mv_limits.col_max = max1.min(max2);
}

/// Translation of `av1_set_mv_limits()` (mcomp.h).
#[inline]
pub(crate) fn av1_set_mv_limits(
    mi_params: &CommonModeInfoParams,
    mv_limits: &mut FullMvLimits,
    mi_row: i32,
    mi_col: i32,
    mi_height: i32,
    mi_width: i32,
    border: i32,
) {
    av1_set_mv_row_limits(mi_params, mv_limits, mi_row, mi_height, border);
    av1_set_mv_col_limits(mi_params, mv_limits, mi_col, mi_width, border);
}

#[inline]
fn av1_is_fullmv_in_range(mv_limits: &FullMvLimits, mv: FullpelMv) -> bool {
    (mv.col as i32 >= mv_limits.col_min)
        && (mv.col as i32 <= mv_limits.col_max)
        && (mv.row as i32 >= mv_limits.row_min)
        && (mv.row as i32 <= mv_limits.row_max)
}

/// Translation of `clamp_fullmv()` (mv.h).
#[inline]
pub(crate) fn clamp_fullmv(mv: &mut FullpelMv, mv_limits: &FullMvLimits) {
    mv.col = (mv.col as i32).clamp(mv_limits.col_min, mv_limits.col_max) as i16;
    mv.row = (mv.row as i32).clamp(mv_limits.row_min, mv_limits.row_max) as i16;
}

/// `get_buf_from_fullmv()`: the offset of the reference at `mv`.
#[inline]
fn get_buf_from_fullmv(buf: &Buf2d, mv: &FullpelMv) -> usize {
    off(buf.buf, get_offset_from_fullmv(mv, buf.stride))
}

// Checks whether the mv is within range of the mv_limits
#[inline]
fn check_bounds(mv_limits: &FullMvLimits, row: i32, col: i32, range: i32) -> bool {
    ((row - range) >= mv_limits.row_min)
        & ((row + range) <= mv_limits.row_max)
        & ((col - range) >= mv_limits.col_min)
        & ((col + range) <= mv_limits.col_max)
}

#[inline]
fn get_mvpred_var_cost(ms_params: &FullpelMotionSearchParams, this_mv: &FullpelMv) -> i32 {
    let vfp = ms_params.vfp;
    let sub_this_mv = get_mv_from_fullmv(this_mv);
    let b = &ms_params.ms_buffers;
    let mut unused = 0u32;

    let mut bestsme = vfp.vf(
        ms_params.bsize,
        b.src_buf,
        b.src.buf,
        b.src.stride as usize,
        b.ref_buf,
        get_buf_from_fullmv(&b.ref_, this_mv),
        b.ref_.stride as usize,
        &mut unused,
    ) as i32;

    bestsme = bestsme.wrapping_add(mv_err_cost_(&sub_this_mv, &ms_params.mv_cost_params));

    bestsme
}

#[inline]
fn get_mvpred_sad(ms_params: &FullpelMotionSearchParams, ref_address: usize) -> u32 {
    ms_params.sdf(ref_address)
}

// Calculates and returns a sad+mvcost list around an integer best pel during
// fullpixel motion search. The resulting list can be used to speed up subpel
// motion search later.
// (USE_SAD_COSTLIST is 1: calc_int_cost_list() is not used)

const NEIGHBORS: [FullpelMv; 4] = [fm(0, -1), fm(1, 0), fm(0, 1), fm(-1, 0)];

// calc_int_sad_list uses sad to populate the costlist, which is less accurate
// than var but faster.
#[inline]
fn calc_int_sad_list(
    best_mv: FullpelMv,
    ms_params: &FullpelMotionSearchParams,
    cost_list: &mut [i32; 5],
    costlist_has_sad: bool,
) {
    let ref_ = &ms_params.ms_buffers.ref_;
    let br = best_mv.row as i32;
    let bc = best_mv.col as i32;

    debug_assert!(av1_is_fullmv_in_range(&ms_params.mv_limits, best_mv));

    // Refresh the costlist it does not contain valid sad
    if !costlist_has_sad {
        cost_list[0] = get_mvpred_sad(ms_params, get_buf_from_fullmv(ref_, &best_mv)) as i32;

        if check_bounds(&ms_params.mv_limits, br, bc, 1) {
            for i in 0..4 {
                let this_mv = fm(br + NEIGHBORS[i].row as i32, bc + NEIGHBORS[i].col as i32);
                cost_list[i + 1] =
                    get_mvpred_sad(ms_params, get_buf_from_fullmv(ref_, &this_mv)) as i32;
            }
        } else {
            for i in 0..4 {
                let this_mv = fm(br + NEIGHBORS[i].row as i32, bc + NEIGHBORS[i].col as i32);
                if !av1_is_fullmv_in_range(&ms_params.mv_limits, this_mv) {
                    cost_list[i + 1] = i32::MAX;
                } else {
                    cost_list[i + 1] =
                        get_mvpred_sad(ms_params, get_buf_from_fullmv(ref_, &this_mv)) as i32;
                }
            }
        }
    }

    let mv_cost_params = &ms_params.mv_cost_params;
    cost_list[0] = cost_list[0].wrapping_add(mvsad_err_cost_(&best_mv, mv_cost_params));

    for idx in 0..4 {
        if cost_list[idx + 1] != i32::MAX {
            let this_mv = fm(
                br + NEIGHBORS[idx].row as i32,
                bc + NEIGHBORS[idx].col as i32,
            );
            cost_list[idx + 1] =
                cost_list[idx + 1].wrapping_add(mvsad_err_cost_(&this_mv, mv_cost_params));
        }
    }
}

// Computes motion vector cost and adds to the sad cost.
// Then updates the best sad and motion vectors.
// Inputs:
//   this_sad: the sad to be evaluated.
//   mv: the current motion vector.
//   mv_cost_params: a structure containing information to compute mv cost.
//   best_sad: the current best sad.
//   raw_best_sad (optional): the current best sad without calculating mv cost.
//   best_mv: the current best motion vector.
//   second_best_mv (optional): the second best motion vector up to now.
// Modifies:
//   best_sad, raw_best_sad, best_mv, second_best_mv
//   If the current sad is lower than the current best sad.
// Returns:
//   Whether the input sad (mv) is better than the current best.
#[inline]
fn update_mvs_and_sad(
    this_sad: u32,
    mv: &FullpelMv,
    mv_cost_params: &MvCostParams,
    best_sad: &mut u32,
    raw_best_sad: Option<&mut u32>,
    best_mv: &mut FullpelMv,
    second_best_mv: Option<&mut FullpelMv>,
) -> bool {
    if this_sad >= *best_sad {
        return false;
    }

    // Add the motion vector cost.
    let sad = this_sad.wrapping_add(mvsad_err_cost_(mv, mv_cost_params) as u32);
    if sad < *best_sad {
        if let Some(r) = raw_best_sad {
            *r = this_sad;
        }
        *best_sad = sad;
        if let Some(s) = second_best_mv {
            *s = *best_mv;
        }
        *best_mv = *mv;
        return true;
    }
    false
}

/// The running state of a pattern search: the best sums and motion
/// vector (upstream's `bestsad`, `raw_bestsad` and `best_mv` pointers).
struct PatternBest<'b> {
    bestsad: u32,
    raw_bestsad: u32,
    best_mv: &'b mut FullpelMv,
}

// Calculate sad4 and update the bestmv information
// in FAST_DIAMOND search method.
/// (`cost_list`: whether the four sums are stored in the cost list's
/// entries 1 to 4, upstream's `sads = cost_list + 1`)
#[inline]
fn calc_sad4_update_bestmv(
    ms_params: &FullpelMotionSearchParams,
    mv_cost_params: &MvCostParams,
    best: &mut PatternBest,
    center_mv: FullpelMv,
    center_address: usize,
    search_step: i32,
    best_site: &mut i32,
    cand_start: i32,
    cost_list: Option<&mut [i32; 5]>,
) {
    let site = &ms_params.search_sites.site[search_step as usize];

    let mut block_offset = [0usize; 4];
    let mut sads = [0u32; 4];
    // Loop over number of candidates.
    for (j, o) in block_offset.iter_mut().enumerate() {
        *o = off(center_address, site[cand_start as usize + j].offset);
    }

    // 4-point sad calculation.
    ms_params.sdx4df(&block_offset, &mut sads);

    for j in 0..4 {
        let s = &site[cand_start as usize + j];
        let this_mv = fm(
            center_mv.row as i32 + s.mv.row as i32,
            center_mv.col as i32 + s.mv.col as i32,
        );
        let found_better_mv = update_mvs_and_sad(
            sads[j],
            &this_mv,
            mv_cost_params,
            &mut best.bestsad,
            Some(&mut best.raw_bestsad),
            best.best_mv,
            /*second_best_mv=*/ None,
        );
        if found_better_mv {
            *best_site = cand_start + j as i32;
        }
    }
    if let Some(cost_list) = cost_list {
        for j in 0..4 {
            cost_list[j + 1] = sads[j] as i32;
        }
    }
}

#[inline]
fn calc_sad3_update_bestmv(
    ms_params: &FullpelMotionSearchParams,
    mv_cost_params: &MvCostParams,
    best: &mut PatternBest,
    center_mv: FullpelMv,
    center_address: usize,
    search_step: i32,
    best_site: &mut i32,
    chkpts_indices: &[i32; PATTERN_CANDIDATES_REF as usize],
    cost_list: Option<&mut [i32; 5]>,
) {
    let site = &ms_params.search_sites.site[search_step as usize];
    let block_offset = [
        off(center_address, site[chkpts_indices[0] as usize].offset),
        off(center_address, site[chkpts_indices[1] as usize].offset),
        off(center_address, site[chkpts_indices[2] as usize].offset),
        center_address,
    ];
    let mut sads = [0u32; 4];
    ms_params.sdx4df(&block_offset, &mut sads);
    for j in 0..3 {
        let index = chkpts_indices[j] as usize;
        let this_mv = fm(
            center_mv.row as i32 + site[index].mv.row as i32,
            center_mv.col as i32 + site[index].mv.col as i32,
        );
        let found_better_mv = update_mvs_and_sad(
            sads[j],
            &this_mv,
            mv_cost_params,
            &mut best.bestsad,
            Some(&mut best.raw_bestsad),
            best.best_mv,
            /*second_best_mv=*/ None,
        );
        if found_better_mv {
            *best_site = j as i32;
        }
    }
    if let Some(cost_list) = cost_list {
        for j in 0..3 {
            let index = chkpts_indices[j] as usize;
            cost_list[index + 1] = sads[j] as i32;
        }
    }
}

// Calculate sad and update the bestmv information
// in FAST_DIAMOND search method.
#[inline]
fn calc_sad_update_bestmv(
    ms_params: &FullpelMotionSearchParams,
    mv_cost_params: &MvCostParams,
    best: &mut PatternBest,
    center_mv: FullpelMv,
    center_address: usize,
    search_step: i32,
    best_site: &mut i32,
    num_candidates: i32,
    cand_start: i32,
    mut cost_list: Option<&mut [i32; 5]>,
) {
    let site = &ms_params.search_sites.site[search_step as usize];
    // Loop over number of candidates.
    for i in cand_start..num_candidates {
        let s = &site[i as usize];
        let this_mv = fm(
            center_mv.row as i32 + s.mv.row as i32,
            center_mv.col as i32 + s.mv.col as i32,
        );
        if !av1_is_fullmv_in_range(&ms_params.mv_limits, this_mv) {
            continue;
        }
        let thissad = get_mvpred_sad(ms_params, off(center_address, s.offset));
        if let Some(c) = cost_list.as_deref_mut() {
            c[i as usize + 1] = thissad as i32;
        }
        let found_better_mv = update_mvs_and_sad(
            thissad,
            &this_mv,
            mv_cost_params,
            &mut best.bestsad,
            Some(&mut best.raw_bestsad),
            best.best_mv,
            /*second_best_mv=*/ None,
        );
        if found_better_mv {
            *best_site = i;
        }
    }
}

#[inline]
fn calc_sad_update_bestmv_with_indices(
    ms_params: &FullpelMotionSearchParams,
    mv_cost_params: &MvCostParams,
    best: &mut PatternBest,
    center_mv: FullpelMv,
    center_address: usize,
    search_step: i32,
    best_site: &mut i32,
    num_candidates: i32,
    chkpts_indices: &[i32; PATTERN_CANDIDATES_REF as usize],
    mut cost_list: Option<&mut [i32; 5]>,
) {
    let site = &ms_params.search_sites.site[search_step as usize];
    // Loop over number of candidates.
    for i in 0..num_candidates as usize {
        let index = chkpts_indices[i] as usize;
        let this_mv = fm(
            center_mv.row as i32 + site[index].mv.row as i32,
            center_mv.col as i32 + site[index].mv.col as i32,
        );
        if !av1_is_fullmv_in_range(&ms_params.mv_limits, this_mv) {
            if let Some(c) = cost_list.as_deref_mut() {
                c[index + 1] = i32::MAX;
            }
            continue;
        }
        let thissad = get_mvpred_sad(ms_params, off(center_address, site[index].offset));
        if let Some(c) = cost_list.as_deref_mut() {
            c[index + 1] = thissad as i32;
        }
        let found_better_mv = update_mvs_and_sad(
            thissad,
            &this_mv,
            mv_cost_params,
            &mut best.bestsad,
            Some(&mut best.raw_bestsad),
            best.best_mv,
            /*second_best_mv=*/ None,
        );
        if found_better_mv {
            *best_site = i as i32;
        }
    }
}

/// The candidates around candidate `k` of `n` (`next_chkpts_indices`).
#[inline]
fn next_chkpts(k: i32, n: i32) -> [i32; PATTERN_CANDIDATES_REF as usize] {
    [
        if k == 0 { n - 1 } else { k - 1 },
        k,
        if k == n - 1 { 0 } else { k + 1 },
    ]
}

// Generic pattern search function that searches over multiple scales.
// Each scale can have a different number of candidates and shape of
// candidates as indicated in the num_candidates and candidates arrays
// passed into this function
fn pattern_search(
    mut start_mv: FullpelMv,
    ms_params: &FullpelMotionSearchParams,
    search_step: i32,
    do_init_search: bool,
    mut cost_list: Option<&mut [i32; 5]>,
    best_mv: &mut FullpelMv,
) -> i32 {
    const SEARCH_STEPS: [i32; MAX_MVSEARCH_STEPS as usize] = [10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0];
    let mut s: i32;

    let ref_ = &ms_params.ms_buffers.ref_;
    let search_sites = ms_params.search_sites;
    let num_candidates = &search_sites.searches_per_step;
    let last_is_4 = num_candidates[0] == 4;
    let mut br: i32;
    let mut bc: i32;
    let mut k: i32 = -1;
    let mv_cost_params = &ms_params.mv_cost_params;
    let search_step = search_step.min(MAX_MVSEARCH_STEPS - 1);
    debug_assert!(search_step >= 0);
    let mut best_init_s = SEARCH_STEPS[search_step as usize];
    // adjust ref_mv to make sure it is within MV range
    clamp_fullmv(&mut start_mv, &ms_params.mv_limits);
    br = start_mv.row as i32;
    bc = start_mv.col as i32;
    if let Some(c) = cost_list.as_deref_mut() {
        *c = [i32::MAX; 5];
    }
    let mut costlist_has_sad = false;

    // Work out the start point for the search
    let raw_bestsad = get_mvpred_sad(ms_params, get_buf_from_fullmv(ref_, &start_mv));
    let mut best = PatternBest {
        raw_bestsad,
        bestsad: raw_bestsad.wrapping_add(mvsad_err_cost_(&start_mv, mv_cost_params) as u32),
        best_mv,
    };

    // Search all possible scales up to the search param around the center point
    // pick the scale of the point that is best as the starting scale of
    // further steps around it.
    let mut center_address = get_buf_from_fullmv(ref_, &start_mv);
    if do_init_search {
        s = best_init_s;
        best_init_s = -1;
        for t in 0..=s {
            let mut best_site = -1;
            let center_mv = fm(br, bc);
            if check_bounds(&ms_params.mv_limits, br, bc, 1 << t) {
                // Call 4-point sad for multiples of 4 candidates.
                let no_of_4_cand_loops = num_candidates[t as usize] >> 2;
                for i in 0..no_of_4_cand_loops {
                    calc_sad4_update_bestmv(
                        ms_params,
                        mv_cost_params,
                        &mut best,
                        center_mv,
                        center_address,
                        t,
                        &mut best_site,
                        i * 4,
                        /*cost_list=*/ None,
                    );
                }
                // Rest of the candidates
                let remaining_cand = num_candidates[t as usize] % 4;
                calc_sad_update_bestmv(
                    ms_params,
                    mv_cost_params,
                    &mut best,
                    center_mv,
                    center_address,
                    t,
                    &mut best_site,
                    remaining_cand,
                    no_of_4_cand_loops * 4,
                    None,
                );
            } else {
                calc_sad_update_bestmv(
                    ms_params,
                    mv_cost_params,
                    &mut best,
                    center_mv,
                    center_address,
                    t,
                    &mut best_site,
                    num_candidates[t as usize],
                    0,
                    None,
                );
            }
            if best_site == -1 {
                continue;
            } else {
                best_init_s = t;
                k = best_site;
            }
        }
        if best_init_s != -1 {
            let site = &search_sites.site[best_init_s as usize][k as usize];
            br += site.mv.row as i32;
            bc += site.mv.col as i32;
            center_address = off(center_address, site.offset);
        }
    }

    // If the center point is still the best, just skip this and move to
    // the refinement step.
    if best_init_s != -1 {
        let last_s = (last_is_4 && cost_list.is_some()) as i32;
        let mut best_site = -1;
        s = best_init_s;

        while s >= last_s {
            // No need to search all points the 1st time if initial search was used
            if !do_init_search || s != best_init_s {
                let center_mv = fm(br, bc);
                if check_bounds(&ms_params.mv_limits, br, bc, 1 << s) {
                    // Call 4-point sad for multiples of 4 candidates.
                    let no_of_4_cand_loops = num_candidates[s as usize] >> 2;
                    for i in 0..no_of_4_cand_loops {
                        calc_sad4_update_bestmv(
                            ms_params,
                            mv_cost_params,
                            &mut best,
                            center_mv,
                            center_address,
                            s,
                            &mut best_site,
                            i * 4,
                            /*cost_list=*/ None,
                        );
                    }
                    // Rest of the candidates
                    let remaining_cand = num_candidates[s as usize] % 4;
                    calc_sad_update_bestmv(
                        ms_params,
                        mv_cost_params,
                        &mut best,
                        center_mv,
                        center_address,
                        s,
                        &mut best_site,
                        remaining_cand,
                        no_of_4_cand_loops * 4,
                        None,
                    );
                } else {
                    calc_sad_update_bestmv(
                        ms_params,
                        mv_cost_params,
                        &mut best,
                        center_mv,
                        center_address,
                        s,
                        &mut best_site,
                        num_candidates[s as usize],
                        0,
                        None,
                    );
                }

                if best_site == -1 {
                    s -= 1;
                    continue;
                } else {
                    let site = &search_sites.site[s as usize][best_site as usize];
                    br += site.mv.row as i32;
                    bc += site.mv.col as i32;
                    center_address = off(center_address, site.offset);
                    k = best_site;
                }
            }

            loop {
                best_site = -1;
                let next_chkpts_indices = next_chkpts(k, num_candidates[s as usize]);

                let center_mv = fm(br, bc);
                if check_bounds(&ms_params.mv_limits, br, bc, 1 << s) {
                    calc_sad3_update_bestmv(
                        ms_params,
                        mv_cost_params,
                        &mut best,
                        center_mv,
                        center_address,
                        s,
                        &mut best_site,
                        &next_chkpts_indices,
                        None,
                    );
                } else {
                    calc_sad_update_bestmv_with_indices(
                        ms_params,
                        mv_cost_params,
                        &mut best,
                        center_mv,
                        center_address,
                        s,
                        &mut best_site,
                        PATTERN_CANDIDATES_REF,
                        &next_chkpts_indices,
                        None,
                    );
                }

                if best_site != -1 {
                    k = next_chkpts_indices[best_site as usize];
                    let site = &search_sites.site[s as usize][k as usize];
                    br += site.mv.row as i32;
                    bc += site.mv.col as i32;
                    center_address = off(center_address, site.offset);
                }
                if best_site == -1 {
                    break;
                }
            }
            s -= 1;
        }
        // Note: If we enter the if below, then cost_list must be non-NULL.
        if s == 0 {
            if let Some(cost_list) = cost_list.as_deref_mut() {
                cost_list[0] = best.raw_bestsad as i32;
                costlist_has_sad = true;
                debug_assert!(num_candidates[s as usize] == 4);
                if !do_init_search || s != best_init_s {
                    let center_mv = fm(br, bc);
                    if check_bounds(&ms_params.mv_limits, br, bc, 1 << s) {
                        calc_sad4_update_bestmv(
                            ms_params,
                            mv_cost_params,
                            &mut best,
                            center_mv,
                            center_address,
                            s,
                            &mut best_site,
                            0,
                            Some(cost_list),
                        );
                    } else {
                        calc_sad_update_bestmv(
                            ms_params,
                            mv_cost_params,
                            &mut best,
                            center_mv,
                            center_address,
                            s,
                            &mut best_site,
                            /*num_candidates=*/ 4,
                            /*cand_start=*/ 0,
                            Some(cost_list),
                        );
                    }

                    if best_site != -1 {
                        let site = &search_sites.site[s as usize][best_site as usize];
                        br += site.mv.row as i32;
                        bc += site.mv.col as i32;
                        center_address = off(center_address, site.offset);
                        k = best_site;
                    }
                }
                while best_site != -1 {
                    best_site = -1;
                    let next_chkpts_indices = next_chkpts(k, num_candidates[s as usize]);
                    cost_list[1] = i32::MAX;
                    cost_list[2] = i32::MAX;
                    cost_list[3] = i32::MAX;
                    cost_list[4] = i32::MAX;
                    cost_list[(((k + 2) % 4) + 1) as usize] = cost_list[0];
                    cost_list[0] = best.raw_bestsad as i32;

                    let center_mv = fm(br, bc);
                    if check_bounds(&ms_params.mv_limits, br, bc, 1 << s) {
                        // (assert(PATTERN_CANDIDATES_REF == 3): a constant)
                        calc_sad3_update_bestmv(
                            ms_params,
                            mv_cost_params,
                            &mut best,
                            center_mv,
                            center_address,
                            s,
                            &mut best_site,
                            &next_chkpts_indices,
                            Some(cost_list),
                        );
                    } else {
                        calc_sad_update_bestmv_with_indices(
                            ms_params,
                            mv_cost_params,
                            &mut best,
                            center_mv,
                            center_address,
                            s,
                            &mut best_site,
                            PATTERN_CANDIDATES_REF,
                            &next_chkpts_indices,
                            Some(cost_list),
                        );
                    }

                    if best_site != -1 {
                        k = next_chkpts_indices[best_site as usize];
                        let site = &search_sites.site[s as usize][k as usize];
                        br += site.mv.row as i32;
                        bc += site.mv.col as i32;
                        center_address = off(center_address, site.offset);
                    }
                }
            }
        }
    }
    let best_mv = best.best_mv;
    best_mv.row = br as i16;
    best_mv.col = bc as i16;

    debug_assert!(
        center_address == get_buf_from_fullmv(ref_, best_mv),
        "center address is out of sync with best_mv!\n"
    );

    // Returns the one-away integer pel cost/sad around the best as follows:
    // cost_list[0]: cost/sad at the best integer pel
    // cost_list[1]: cost/sad at delta {0, -1} (left)   from the best integer pel
    // cost_list[2]: cost/sad at delta { 1, 0} (bottom) from the best integer pel
    // cost_list[3]: cost/sad at delta { 0, 1} (right)  from the best integer pel
    // cost_list[4]: cost/sad at delta {-1, 0} (top)    from the best integer pel
    if let Some(cost_list) = cost_list {
        // (USE_SAD_COSTLIST)
        calc_int_sad_list(*best_mv, ms_params, cost_list, costlist_has_sad);
    }

    get_mvpred_var_cost(ms_params, best_mv)
}

// For the following foo_search, the input arguments are:
// start_mv: where we are starting our motion search
// ms_params: a collection of motion search parameters
// search_step: how many steps to skip in our motion search. For example,
//   a value 3 suggests that 3 search steps have already taken place prior to
//   this function call, so we jump directly to step 4 of the search process
// do_init_search: if on, do an initial search of all possible scales around the
//   start_mv, and then pick the best scale.
// cond_list: used to hold the cost around the best full mv so we can use it to
//   speed up subpel search later.
// best_mv: the best mv found in the motion search
// (hex_search(), bigdia_search() and square_search() are pattern_search())

/// Translation of `av1_full_pixel_search()`: `Err` for the diamond
/// (`NSTEP`, `DIAMOND`, ...) searches and the exhaustive mesh search, which
/// the real-time speed features never use.
pub(crate) fn av1_full_pixel_search(
    start_mv: FullpelMv,
    ms_params: &FullpelMotionSearchParams,
    step_param: i32,
    mut cost_list: Option<&mut [i32; 5]>,
    best_mv: &mut FullpelMv,
    second_best_mv: Option<&mut FullpelMv>,
) -> Result<i32, ()> {
    let bsize = ms_params.bsize;
    let search_method = ms_params.search_method;
    let is_intra_mode = ms_params.is_intra_mode;
    let mut run_mesh_search = ms_params.run_mesh_search;

    // (MARK_MV_INVALID())
    best_mv.row = INVALID_MV_ROW_COL as i16;
    best_mv.col = INVALID_MV_ROW_COL as i16;
    if let Some(s) = second_best_mv {
        s.row = INVALID_MV_ROW_COL as i16;
        s.col = INVALID_MV_ROW_COL as i16;
    }

    if let Some(c) = cost_list.as_deref_mut() {
        *c = [i32::MAX; 5];
    }

    debug_assert!(ms_params.ms_buffers.ref_.stride == ms_params.search_sites.stride);

    let (step, init) = match search_method {
        FAST_BIGDIA => ((MAX_MVSEARCH_STEPS - 3).max(step_param), false),
        VFAST_DIAMOND => ((MAX_MVSEARCH_STEPS - 1).max(step_param), false),
        FAST_DIAMOND => ((MAX_MVSEARCH_STEPS - 2).max(step_param), false),
        FAST_HEX => ((MAX_MVSEARCH_STEPS - 2).max(step_param), false),
        HEX | SQUARE | BIGDIA => (step_param, true),
        // (full_pixel_diamond(): the diamond searches)
        _ => return Err(()),
    };
    let var = pattern_search(
        start_mv,
        ms_params,
        step,
        init,
        cost_list.as_deref_mut(),
        best_mv,
    );

    // Should we allow a follow on exhaustive search?
    if run_mesh_search == 0 && (search_method == NSTEP || search_method == NSTEP_8PT) {
        let mut exhaustive_thr = ms_params.force_mesh_thresh;
        exhaustive_thr >>=
            10 - (MI_SIZE_WIDE_LOG2[bsize as usize] + MI_SIZE_HIGH_LOG2[bsize as usize]) as i32;
        // Threshold variance for an exhaustive full search.
        if var > exhaustive_thr {
            run_mesh_search = 1;
        }
    }

    // TODO(yunqing): the following is used to reduce mesh search in temporal
    // filtering. Can extend it to intrabc.
    if is_intra_mode == 0 && ms_params.prune_mesh_search != 0 {
        let full_pel_mv_diff = (start_mv.row as i32 - best_mv.row as i32)
            .abs()
            .max((start_mv.col as i32 - best_mv.col as i32).abs());
        if full_pel_mv_diff <= ms_params.mesh_search_mv_diff_threshold {
            run_mesh_search = 0;
        }
    }

    if ms_params.use_downsampled_sad {
        // If we are skipping rows when we perform the motion search, we need to
        // check the quality of skipping. If it's bad, then we run mesh search with
        // skip row features off.
        // TODO(chiyotsai@google.com): Handle the case where we have a vertical
        // offset of 1 before we hit this statement to avoid having to redo
        // motion search.
        let b = &ms_params.ms_buffers;
        let best_address = get_buf_from_fullmv(&b.ref_, best_mv);
        let sad = ms_params.vfp.sdf(
            bsize,
            b.src_buf,
            b.src.buf,
            b.src.stride as usize,
            b.ref_buf,
            best_address,
            b.ref_.stride as usize,
        ) as i32;
        let skip_sad = ms_params.vfp.sdsf(
            bsize,
            b.src_buf,
            b.src.buf,
            b.src.stride as usize,
            b.ref_buf,
            best_address,
            b.ref_.stride as usize,
        ) as i32;
        // We will keep the result of skipping rows if it's good enough. Here, good
        // enough means the error is less than 1 per pixel.
        let k_sad_thresh =
            1 << (MI_SIZE_WIDE_LOG2[bsize as usize] + MI_SIZE_HIGH_LOG2[bsize as usize]);
        if sad > k_sad_thresh && (skip_sad - sad).abs() * 10 >= sad.max(1) * 9 {
            // There is a large discrepancy between skipping and not skipping, so we
            // need to redo the motion search.
            let mut new_ms_params = *ms_params;
            new_ms_params.use_downsampled_sad = false;

            return av1_full_pixel_search(
                start_mv,
                &new_ms_params,
                step_param,
                cost_list,
                best_mv,
                None,
            );
        }
    }

    if run_mesh_search != 0 {
        // (full_pixel_exhaustive(): the mesh search)
        return Err(());
    }

    Ok(var)
}

fn vector_match(ref_: &[i16], src: &[i16], bwl: i32) -> i32 {
    let mut best_sad = i32::MAX;
    let mut this_sad;
    let mut center;
    let mut offset = 0;
    let bw = 4 << bwl; // redundant variable, to be changed in the experiments.
    let mut d = 0;
    while d <= bw {
        this_sad = aom_vector_var(&ref_[d as usize..], src, bwl);
        if this_sad < best_sad {
            best_sad = this_sad;
            offset = d;
        }
        d += 16;
    }
    center = offset;

    // (the four refinements, of 8, 4, 2 and 1 samples)
    for (step, span) in [(16, 8), (8, 4), (4, 2), (2, 1)] {
        let mut d = -span;
        while d <= span {
            let this_pos = offset + d;
            // check limit
            if !(this_pos < 0 || this_pos > bw) {
                this_sad = aom_vector_var(&ref_[this_pos as usize..], src, bwl);
                if this_sad < best_sad {
                    best_sad = this_sad;
                    center = this_pos;
                }
            }
            d += step;
        }
        offset = center;
    }

    center - (bw >> 1)
}

// A special fast version of motion search used in rt mode
/// Translation of `av1_int_pro_motion_estimation()`.
pub(crate) fn av1_int_pro_motion_estimation(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    bsize: BlockSize,
    mi_row: i32,
    mi_col: i32,
    ref_mv: &Mv,
) -> Result<u32, ()> {
    with_source(x, |x, src| {
        int_pro_motion_estimation(cpi, x, src, bsize, mi_row, mi_col, ref_mv)
    })
}

/// `av1_int_pro_motion_estimation()` on the source samples `src`.
fn int_pro_motion_estimation(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    src: &[u16],
    bsize: BlockSize,
    mi_row: i32,
    mi_col: i32,
    ref_mv: &Mv,
) -> Result<u32, ()> {
    let mut backup_yv12 = [Buf2d::default(); MAX_MB_PLANE];
    let mut hbuf = [0i16; 256];
    let mut vbuf = [0i16; 256];
    let mut src_hbuf = [0i16; 128];
    let mut src_vbuf = [0i16; 128];
    let bw = 4 << MI_SIZE_WIDE_LOG2[bsize as usize];
    let bh = 4 << MI_SIZE_HIGH_LOG2[bsize as usize];
    let search_width = bw << 1;
    let search_height = bh << 1;
    let src_stride = x.plane[0].src.stride;
    let ref_stride = x.e_mbd.plane[0].pre[0].stride;
    let mut best_sad: u32;
    let tmp_sad: u32;
    let mut this_sad = [0u32; 4];
    let row_norm_factor = MI_SIZE_HIGH_LOG2[bsize as usize] as i32 + 1;
    let col_norm_factor = 3 + (bw >> 5);
    let scaled_ref_frame = av1_get_scaled_ref_frame(cpi, x.e_mbd.mbmi().ref_frame[0]);
    const SEARCH_POS: [Mv; 4] = [
        Mv { row: -1, col: 0 },
        Mv { row: 0, col: -1 },
        Mv { row: 0, col: 1 },
        Mv { row: 1, col: 0 },
    ];

    let cm = &cpi.common;
    if let Some(scaled) = scaled_ref_frame {
        // Swap out the reference frame for a version that's been scaled to
        // match the resolution of the current frame, allowing the existing
        // motion search code to be used without additional modifications.
        for (b, pd) in backup_yv12.iter_mut().zip(x.e_mbd.plane.iter()) {
            *b = pd.pre[0];
        }
        av1_setup_pre_planes(
            &mut x.e_mbd,
            0,
            Some((&cm.buffer_pool.frame_bufs[scaled].buf, scaled)),
            mi_row,
            mi_col,
            None,
            MAX_MB_PLANE,
        );
    }

    let pre = x.e_mbd.plane[0].pre[0];
    let ref_frame = &cm.buffer_pool.frame_bufs[pre.frame_buf].buf.buffer_alloc;
    let fn_ptr = &cpi.ppi.fn_ptr;
    let src_buf = x.plane[0].src.buf;
    let restore = |x: &mut Macroblock| {
        if scaled_ref_frame.is_some() {
            for (pd, b) in x.e_mbd.plane.iter_mut().zip(backup_yv12.iter()) {
                pd.pre[0] = *b;
            }
        }
    };

    if x.e_mbd.bd != 8 {
        x.e_mbd.mbmi_mut().mv[0].set_fullmv(K_ZERO_FULL_MV);
        let sad = fn_ptr.sdf(
            bsize,
            src,
            src_buf,
            src_stride as usize,
            ref_frame,
            pre.buf,
            ref_stride as usize,
        );

        restore(x);
        return Ok(sad);
    }

    // Set up prediction 1-D reference set
    let ref_buf = off(pre.buf, -(bw >> 1));
    aom_int_pro_row(
        &mut hbuf,
        ref_frame,
        ref_buf,
        ref_stride as usize,
        search_width,
        bh,
        row_norm_factor,
    );

    let ref_buf = off(pre.buf, -(bh >> 1) * ref_stride);
    aom_int_pro_col(
        &mut vbuf,
        ref_frame,
        ref_buf,
        ref_stride as usize,
        bw,
        search_height,
        col_norm_factor,
    );

    // Set up src 1-D reference set
    aom_int_pro_row(
        &mut src_hbuf,
        src,
        src_buf,
        src_stride as usize,
        bw,
        bh,
        row_norm_factor,
    );
    aom_int_pro_col(
        &mut src_vbuf,
        src,
        src_buf,
        src_stride as usize,
        bw,
        bh,
        col_norm_factor,
    );

    // Find the best match per 1-D search
    let mut best_int_mv = FullpelMv {
        col: vector_match(&hbuf, &src_hbuf, MI_SIZE_WIDE_LOG2[bsize as usize] as i32) as i16,
        row: vector_match(&vbuf, &src_vbuf, MI_SIZE_HIGH_LOG2[bsize as usize] as i32) as i16,
    };

    let mut this_mv = best_int_mv;
    let ref_buf = get_buf_from_fullmv(&pre, &this_mv);
    best_sad = fn_ptr.sdf(
        bsize,
        src,
        src_buf,
        src_stride as usize,
        ref_frame,
        ref_buf,
        ref_stride as usize,
    );

    {
        let pos = [
            off(ref_buf, -ref_stride),
            off(ref_buf, -1),
            off(ref_buf, 1),
            off(ref_buf, ref_stride),
        ];

        fn_ptr.sdx4df(
            bsize,
            src,
            src_buf,
            src_stride as usize,
            ref_frame,
            &pos,
            ref_stride as usize,
            &mut this_sad,
        );
    }

    for idx in 0..4 {
        if this_sad[idx] < best_sad {
            best_sad = this_sad[idx];
            best_int_mv.row = SEARCH_POS[idx].row + this_mv.row;
            best_int_mv.col = SEARCH_POS[idx].col + this_mv.col;
        }
    }

    if this_sad[0] < this_sad[3] {
        this_mv.row -= 1;
    } else {
        this_mv.row += 1;
    }

    if this_sad[1] < this_sad[2] {
        this_mv.col -= 1;
    } else {
        this_mv.col += 1;
    }

    let ref_buf = get_buf_from_fullmv(&pre, &this_mv);

    tmp_sad = fn_ptr.sdf(
        bsize,
        src,
        src_buf,
        src_stride as usize,
        ref_frame,
        ref_buf,
        ref_stride as usize,
    );
    if best_sad > tmp_sad {
        best_int_mv = this_mv;
        best_sad = tmp_sad;
    }

    let mut mv_limits = x.mv_limits;
    av1_set_mv_search_range(&mut mv_limits, ref_mv);
    clamp_fullmv(&mut best_int_mv, &mv_limits);

    let mut best = IntMv::from_fullmv(best_int_mv);
    convert_fullmv_to_mv(&mut best);
    x.e_mbd.mbmi_mut().mv[0] = best;

    restore(x);

    Ok(best_sad)
}

// =============================================================================
//  Subpixel Motion Search: Translational
// =============================================================================
const INIT_SUBPEL_STEP_SIZE: i32 = 4;

/// `SUBPEL_SEARCH_VAR_PARAMS` (with the block size `vfp` is the function
/// table entry of)
#[derive(Clone, Copy)]
pub(crate) struct SubpelSearchVarParams<'a> {
    pub(crate) vfp: &'a AomVarianceFnPtr,
    pub(crate) subpel_search_type: i32,
    // Source and reference buffers
    pub(crate) ms_buffers: MsBuffers<'a>,
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) bsize: BlockSize,
}

// This struct holds subpixel motion search parameters that should be constant
// during the search
/// `SUBPEL_MOTION_SEARCH_PARAMS`
#[derive(Clone, Copy)]
pub(crate) struct SubpelMotionSearchParams<'a> {
    // High level motion search settings
    pub(crate) allow_hp: bool,
    pub(crate) cost_list: Option<[i32; 5]>,
    pub(crate) forced_stop: i32,
    pub(crate) iters_per_step: i32,
    pub(crate) mv_limits: SubpelMvLimits,

    // For calculating mv cost
    pub(crate) mv_cost_params: MvCostParams<'a>,

    // Distortion calculation params
    pub(crate) var_params: SubpelSearchVarParams<'a>,
}

/// Translation of `cond_cost_list_const()` (encoder.h).
#[inline]
pub(crate) fn cond_cost_list_const(cpi: &Av1Comp, cost_list: &[i32; 5]) -> Option<[i32; 5]> {
    if cond_cost_list(cpi) {
        Some(*cost_list)
    } else {
        None
    }
}

/// Translation of `cond_cost_list()` (encoder.h): whether the search is
/// given the cost list.
#[inline]
pub(crate) fn cond_cost_list(cpi: &Av1Comp) -> bool {
    cpi.sf.mv_sf.subpel_search_method != SUBPEL_TREE && cpi.sf.mv_sf.use_fullpel_costlist != 0
}

/// Translation of `av1_make_default_subpel_ms_params()`.
pub(crate) fn av1_make_default_subpel_ms_params<'a>(
    cpi: &'a Av1Comp,
    x: &'a Macroblock,
    bsize: BlockSize,
    ref_mv: &Mv,
    cost_list: &[i32; 5],
) -> SubpelMotionSearchParams<'a> {
    let cm = &cpi.common;
    let mut mv_limits = SubpelMvLimits::default();
    av1_set_subpel_mv_search_range(&mut mv_limits, &x.mv_limits, ref_mv);
    SubpelMotionSearchParams {
        // High level params
        allow_hp: cm.features.allow_high_precision_mv,
        forced_stop: cpi.sf.mv_sf.subpel_force_stop,
        iters_per_step: cpi.sf.mv_sf.subpel_iters_per_step,
        cost_list: cond_cost_list_const(cpi, cost_list),
        mv_limits,
        // Mvcost params
        mv_cost_params: init_mv_cost_params(
            x.mv_costs.as_deref(),
            ref_mv,
            x.errorperbit,
            x.sadperbit,
        ),
        // Subpel variance params
        var_params: SubpelSearchVarParams {
            vfp: &cpi.ppi.fn_ptr,
            subpel_search_type: cpi.sf.mv_sf.use_accurate_subpel_search,
            w: BLOCK_SIZE_WIDE[bsize as usize] as i32,
            h: BLOCK_SIZE_HIGH[bsize as usize] as i32,
            bsize,
            // Ref and src buffers
            ms_buffers: init_ms_buffers(cpi, x),
        },
    }
}

/// Translation of `av1_set_subpel_mv_search_range()` (mcomp.h).
#[inline]
pub(crate) fn av1_set_subpel_mv_search_range(
    subpel_limits: &mut SubpelMvLimits,
    mv_limits: &FullMvLimits,
    ref_mv: &Mv,
) {
    let max_mv = get_mv_subpel(MAX_FULL_PEL_VAL);
    let minc = get_mv_subpel(mv_limits.col_min).max(ref_mv.col as i32 - max_mv);
    let maxc = get_mv_subpel(mv_limits.col_max).min(ref_mv.col as i32 + max_mv);
    let minr = get_mv_subpel(mv_limits.row_min).max(ref_mv.row as i32 - max_mv);
    let maxr = get_mv_subpel(mv_limits.row_max).min(ref_mv.row as i32 + max_mv);

    subpel_limits.col_min = (MV_LOW + 1).max(minc);
    subpel_limits.col_max = (MV_UPP - 1).min(maxc);
    subpel_limits.row_min = (MV_LOW + 1).max(minr);
    subpel_limits.row_max = (MV_UPP - 1).min(maxr);
}

#[inline]
fn av1_is_subpelmv_in_range(mv_limits: &SubpelMvLimits, mv: Mv) -> bool {
    (mv.col as i32 >= mv_limits.col_min)
        && (mv.col as i32 <= mv_limits.col_max)
        && (mv.row as i32 >= mv_limits.row_min)
        && (mv.row as i32 <= mv_limits.row_max)
}

// Returns the subpel offset used by various subpel variance functions [m]sv[a]f
#[inline]
fn get_subpel_part(x: i32) -> i32 {
    x & 7
}

// Gets the address of the ref buffer at subpel location (r, c), rounded to the
// nearest fullpel precision toward - \infty
#[inline]
fn get_buf_from_mv(buf: &Buf2d, mv: Mv) -> usize {
    let offset = (mv.row as i32 >> 3) * buf.stride + (mv.col as i32 >> 3);
    off(buf.buf, offset)
}

// Estimates the variance of prediction residue using bilinear filter for fast
// search.
#[inline]
fn estimated_pref_error(this_mv: &Mv, var_params: &SubpelSearchVarParams, sse: &mut u32) -> i32 {
    let vfp = var_params.vfp;

    let ms_buffers = &var_params.ms_buffers;
    let r = get_buf_from_mv(&ms_buffers.ref_, *this_mv);

    let subpel_x_q3 = get_subpel_part(this_mv.col as i32);
    let subpel_y_q3 = get_subpel_part(this_mv.row as i32);

    // (second_pred is NULL)
    vfp.svf(
        var_params.bsize,
        ms_buffers.ref_buf,
        r,
        ms_buffers.ref_.stride as usize,
        subpel_x_q3,
        subpel_y_q3,
        ms_buffers.src_buf,
        ms_buffers.src.buf,
        ms_buffers.src.stride as usize,
        sse,
    ) as i32
}

/// The state the subpel checks update (upstream's `besterr`, `sse1` and
/// `distortion` pointers).
struct SubpelBest {
    besterr: u32,
    sse1: u32,
    distortion: i32,
}

/// The constant inputs of the subpel checks.
struct SubpelCtx<'p, 'a> {
    mv_limits: &'p SubpelMvLimits,
    var_params: &'p SubpelSearchVarParams<'a>,
    mv_cost_params: &'p MvCostParams<'a>,
    is_scaled: bool,
}

// Estimates whether this_mv is better than best_mv. This function incorporates
// both prediction error and residue into account. It is suffixed "fast" because
// it uses bilinear filter to estimate the prediction.
/// (`Err` for the scaled references' upsampled prediction error)
#[inline]
fn check_better_fast(
    c: &SubpelCtx,
    this_mv: &Mv,
    best_mv: &mut Mv,
    best: &mut SubpelBest,
    has_better_mv: &mut i32,
) -> Result<u32, ()> {
    let mut cost: u32;
    if av1_is_subpelmv_in_range(c.mv_limits, *this_mv) {
        let mut sse = 0u32;
        if c.is_scaled {
            // (upsampled_pref_error())
            return Err(());
        }
        let thismse = estimated_pref_error(this_mv, c.var_params, &mut sse);
        cost = mv_err_cost_(this_mv, c.mv_cost_params) as u32;
        cost = cost.wrapping_add(thismse as u32);

        if cost < best.besterr {
            best.besterr = cost;
            *best_mv = *this_mv;
            best.distortion = thismse;
            best.sse1 = sse;
            *has_better_mv |= 1;
        }
    } else {
        cost = i32::MAX as u32;
    }
    Ok(cost)
}

#[inline]
fn get_best_diag_step(
    step_size: i32,
    left_cost: u32,
    right_cost: u32,
    up_cost: u32,
    down_cost: u32,
) -> Mv {
    Mv {
        row: (if up_cost <= down_cost {
            -step_size
        } else {
            step_size
        }) as i16,
        col: (if left_cost <= right_cost {
            -step_size
        } else {
            step_size
        }) as i16,
    }
}

#[inline]
fn mvp(row: i32, col: i32) -> Mv {
    Mv {
        row: row as i16,
        col: col as i16,
    }
}

// Searches the four cardinal direction for a better mv, then follows up with a
// search in the best quadrant. This uses bilinear filter to speed up the
// calculation.
#[inline]
fn first_level_check_fast(
    c: &SubpelCtx,
    this_mv: Mv,
    best_mv: &mut Mv,
    hstep: i32,
    best: &mut SubpelBest,
) -> Result<Mv, ()> {
    let (r, col) = (this_mv.row as i32, this_mv.col as i32);
    // Check the four cardinal directions
    let left_mv = mvp(r, col - hstep);
    let mut dummy = 0;
    let left = check_better_fast(c, &left_mv, best_mv, best, &mut dummy)?;

    let right_mv = mvp(r, col + hstep);
    let right = check_better_fast(c, &right_mv, best_mv, best, &mut dummy)?;

    let top_mv = mvp(r - hstep, col);
    let up = check_better_fast(c, &top_mv, best_mv, best, &mut dummy)?;

    let bottom_mv = mvp(r + hstep, col);
    let down = check_better_fast(c, &bottom_mv, best_mv, best, &mut dummy)?;

    let diag_step = get_best_diag_step(hstep, left, right, up, down);
    let diag_mv = mvp(r + diag_step.row as i32, col + diag_step.col as i32);

    // Check the diagonal direction with the best mv
    check_better_fast(c, &diag_mv, best_mv, best, &mut dummy)?;

    Ok(diag_step)
}

// Performs a following up search after first_level_check_fast is called. This
// performs two extra chess pattern searches in the best quadrant.
#[inline]
fn second_level_check_fast(
    c: &SubpelCtx,
    this_mv: Mv,
    diag_step: Mv,
    best_mv: &mut Mv,
    hstep: i32,
    best: &mut SubpelBest,
) -> Result<(), ()> {
    debug_assert!(diag_step.row as i32 == hstep || diag_step.row as i32 == -hstep);
    debug_assert!(diag_step.col as i32 == hstep || diag_step.col as i32 == -hstep);
    let tr = this_mv.row as i32;
    let tc = this_mv.col as i32;
    let br = best_mv.row as i32;
    let bc = best_mv.col as i32;
    let (dr, dc) = (diag_step.row as i32, diag_step.col as i32);
    let mut dummy = 0;
    if tr != br && tc != bc {
        debug_assert!(dc == bc - tc);
        debug_assert!(dr == br - tr);
        let chess_mv_1 = mvp(br, bc + dc);
        let chess_mv_2 = mvp(br + dr, bc);
        check_better_fast(c, &chess_mv_1, best_mv, best, &mut dummy)?;

        check_better_fast(c, &chess_mv_2, best_mv, best, &mut dummy)?;
    } else if tr == br && tc != bc {
        debug_assert!(dc == bc - tc);
        // Continue searching in the best direction
        let bottom_long_mv = mvp(br + hstep, bc + dc);
        let top_long_mv = mvp(br - hstep, bc + dc);
        check_better_fast(c, &bottom_long_mv, best_mv, best, &mut dummy)?;
        check_better_fast(c, &top_long_mv, best_mv, best, &mut dummy)?;

        // Search in the direction opposite of the best quadrant
        let rev_mv = mvp(br - dr, bc);
        check_better_fast(c, &rev_mv, best_mv, best, &mut dummy)?;
    } else if tr != br && tc == bc {
        debug_assert!(dr == br - tr);
        // Continue searching in the best direction
        let right_long_mv = mvp(br + dr, bc + hstep);
        let left_long_mv = mvp(br + dr, bc - hstep);
        check_better_fast(c, &right_long_mv, best_mv, best, &mut dummy)?;
        check_better_fast(c, &left_long_mv, best_mv, best, &mut dummy)?;

        // Search in the direction opposite of the best quadrant
        let rev_mv = mvp(br, bc - dc);
        check_better_fast(c, &rev_mv, best_mv, best, &mut dummy)?;
    }
    Ok(())
}

// Combines first level check and second level check when applicable. This first
// searches the four cardinal directions, and perform several
// diagonal/chess-pattern searches in the best quadrant.
#[inline]
fn two_level_checks_fast(
    c: &SubpelCtx,
    this_mv: Mv,
    best_mv: &mut Mv,
    hstep: i32,
    best: &mut SubpelBest,
    iters: i32,
) -> Result<(), ()> {
    let diag_step = first_level_check_fast(c, this_mv, best_mv, hstep, best)?;
    if iters > 1 {
        second_level_check_fast(c, this_mv, diag_step, best_mv, hstep, best)?;
    }
    Ok(())
}

// Gets the error at the beginning when the mv has fullpel precision
fn setup_center_error(
    bestmv: &Mv,
    var_params: &SubpelSearchVarParams,
    mv_cost_params: &MvCostParams,
    sse1: &mut u32,
    distortion: &mut i32,
) -> u32 {
    let vfp = var_params.vfp;

    let ms_buffers = &var_params.ms_buffers;
    let y = get_buf_from_mv(&ms_buffers.ref_, *bestmv);

    // (second_pred is NULL)
    let mut besterr = vfp.vf(
        var_params.bsize,
        ms_buffers.ref_buf,
        y,
        ms_buffers.ref_.stride as usize,
        ms_buffers.src_buf,
        ms_buffers.src.buf,
        ms_buffers.src.stride as usize,
        sse1,
    );
    *distortion = besterr as i32;
    besterr = besterr.wrapping_add(mv_err_cost_(bestmv, mv_cost_params) as u32);
    besterr
}

#[inline]
fn divide_and_round(n: i32, d: i32) -> i32 {
    if (n < 0) ^ (d < 0) {
        (n - d / 2) / d
    } else {
        (n + d / 2) / d
    }
}

#[inline]
fn is_cost_list_wellbehaved(cost_list: &[i32; 5]) -> bool {
    cost_list[0] < cost_list[1]
        && cost_list[0] < cost_list[2]
        && cost_list[0] < cost_list[3]
        && cost_list[0] < cost_list[4]
}

// Returns surface minima estimate at given precision in 1/2^n bits.
// Assume a model for the cost surface: S = A(x - x0)^2 + B(y - y0)^2 + C
// For a given set of costs S0, S1, S2, S3, S4 at points
// (y, x) = (0, 0), (0, -1), (1, 0), (0, 1) and (-1, 0) respectively,
// the solution for the location of the minima (x0, y0) is given by:
// x0 = 1/2 (S1 - S3)/(S1 + S3 - 2*S0),
// y0 = 1/2 (S4 - S2)/(S4 + S2 - 2*S0).
// The code below is an integerized version of that.
#[inline]
fn get_cost_surf_min(cost_list: &[i32; 5], ir: &mut i32, ic: &mut i32, bits: i32) {
    *ic = divide_and_round(
        cost_list[1]
            .wrapping_sub(cost_list[3])
            .wrapping_mul(1 << (bits - 1)),
        cost_list[1]
            .wrapping_sub(cost_list[0].wrapping_mul(2))
            .wrapping_add(cost_list[3]),
    );
    *ir = divide_and_round(
        cost_list[4]
            .wrapping_sub(cost_list[2])
            .wrapping_mul(1 << (bits - 1)),
        cost_list[4]
            .wrapping_sub(cost_list[0].wrapping_mul(2))
            .wrapping_add(cost_list[2]),
    );
}

/// The scale factors the subpel searches check (`is_scaled`).
#[inline]
fn subpel_is_scaled(cm: &Av1Common, xd: &MacroBlockD) -> bool {
    let sf = if is_intrabc_block(xd.mbmi()) {
        &cm.sf_identity
    } else {
        &xd.block_ref_scale_factors[0]
    };
    av1_is_scaled(sf)
}

/// The start of both pruned subpel searches: the center error
/// (`setup_center_error_facade()`; `Err` for the scaled references'
/// upsampled error).
#[inline]
fn subpel_center<'p, 'a>(
    cm: &Av1Common,
    xd: &MacroBlockD,
    ms_params: &'p SubpelMotionSearchParams<'a>,
    bestmv: &Mv,
) -> Result<(SubpelCtx<'p, 'a>, SubpelBest), ()> {
    let c = SubpelCtx {
        mv_limits: &ms_params.mv_limits,
        var_params: &ms_params.var_params,
        mv_cost_params: &ms_params.mv_cost_params,
        is_scaled: subpel_is_scaled(cm, xd),
    };
    let mut best = SubpelBest {
        besterr: 0,
        sse1: 0,
        distortion: 0,
    };
    if c.is_scaled {
        // (upsampled_setup_center_error())
        return Err(());
    }
    best.besterr = setup_center_error(
        bestmv,
        c.var_params,
        c.mv_cost_params,
        &mut best.sse1,
        &mut best.distortion,
    );
    Ok((c, best))
}

/// Translation of `av1_find_best_sub_pixel_tree_pruned_more()`
/// (`last_mv_search_list` is always NULL in the real-time encoder).
pub(crate) fn av1_find_best_sub_pixel_tree_pruned_more(
    xd: &MacroBlockD,
    cm: &Av1Common,
    ms_params: &SubpelMotionSearchParams,
    mut start_mv: Mv,
    bestmv: &mut Mv,
    distortion: &mut i32,
    sse1: &mut u32,
) -> Result<i32, ()> {
    let allow_hp = ms_params.allow_hp;
    let forced_stop = ms_params.forced_stop;
    let iters_per_step = ms_params.iters_per_step;
    let cost_list = ms_params.cost_list.as_ref();

    // The iteration we are current searching for. Iter 0 corresponds to fullpel
    // mv, iter 1 to half pel, and so on
    let mut hstep = INIT_SUBPEL_STEP_SIZE; // Step size, initialized to 4/8=1/2 pel
    *bestmv = start_mv;

    let (c, mut best) = subpel_center(cm, xd, ms_params, bestmv)?;
    let finish = |best: &SubpelBest, sse1: &mut u32, distortion: &mut i32| {
        *sse1 = best.sse1;
        *distortion = best.distortion;
        best.besterr as i32
    };

    // If forced_stop is FULL_PEL, return.
    if forced_stop == FULL_PEL {
        return Ok(finish(&best, sse1, distortion));
    }

    // (check_repeated_mv_and_update(): last_mv_search_list is NULL)

    if let Some(cost_list) = cost_list.filter(|cl| {
        cl[0] != i32::MAX
            && cl[1] != i32::MAX
            && cl[2] != i32::MAX
            && cl[3] != i32::MAX
            && cl[4] != i32::MAX
            && is_cost_list_wellbehaved(cl)
    }) {
        let mut ir = 0;
        let mut ic = 0;
        get_cost_surf_min(cost_list, &mut ir, &mut ic, 1);
        if ir != 0 || ic != 0 {
            let this_mv = mvp(
                start_mv.row as i32 + ir * hstep,
                start_mv.col as i32 + ic * hstep,
            );
            let mut dummy = 0;
            check_better_fast(&c, &this_mv, bestmv, &mut best, &mut dummy)?;
        }
    } else {
        two_level_checks_fast(&c, start_mv, bestmv, hstep, &mut best, iters_per_step)?;
    }

    // Each subsequent iteration checks at least one point in common with
    // the last iteration could be 2 ( if diag selected) 1/4 pel
    if forced_stop < HALF_PEL {
        hstep >>= 1;
        start_mv = *bestmv;
        two_level_checks_fast(&c, start_mv, bestmv, hstep, &mut best, iters_per_step)?;
    }

    if allow_hp && forced_stop == EIGHTH_PEL {
        hstep >>= 1;
        start_mv = *bestmv;
        two_level_checks_fast(&c, start_mv, bestmv, hstep, &mut best, iters_per_step)?;
    }

    Ok(finish(&best, sse1, distortion))
}

/// Translation of `av1_find_best_sub_pixel_tree_pruned()`
/// (`last_mv_search_list` is always NULL in the real-time encoder).
pub(crate) fn av1_find_best_sub_pixel_tree_pruned(
    xd: &MacroBlockD,
    cm: &Av1Common,
    ms_params: &SubpelMotionSearchParams,
    mut start_mv: Mv,
    bestmv: &mut Mv,
    distortion: &mut i32,
    sse1: &mut u32,
) -> Result<i32, ()> {
    let allow_hp = ms_params.allow_hp;
    let forced_stop = ms_params.forced_stop;
    let iters_per_step = ms_params.iters_per_step;
    let cost_list = ms_params.cost_list.as_ref();

    // The iteration we are current searching for. Iter 0 corresponds to fullpel
    // mv, iter 1 to half pel, and so on
    let mut hstep = INIT_SUBPEL_STEP_SIZE; // Step size, initialized to 4/8=1/2 pel
    *bestmv = start_mv;

    let (c, mut best) = subpel_center(cm, xd, ms_params, bestmv)?;
    let finish = |best: &SubpelBest, sse1: &mut u32, distortion: &mut i32| {
        *sse1 = best.sse1;
        *distortion = best.distortion;
        best.besterr as i32
    };

    // If forced_stop is FULL_PEL, return.
    if forced_stop == FULL_PEL {
        return Ok(finish(&best, sse1, distortion));
    }

    // (check_repeated_mv_and_update(): last_mv_search_list is NULL)

    if let Some(cost_list) = cost_list.filter(|cl| {
        cl[0] != i32::MAX
            && cl[1] != i32::MAX
            && cl[2] != i32::MAX
            && cl[3] != i32::MAX
            && cl[4] != i32::MAX
    }) {
        let whichdir = (if cost_list[1] < cost_list[3] { 0 } else { 1 })
            + (if cost_list[2] < cost_list[4] { 0 } else { 2 });

        let (r, col) = (start_mv.row as i32, start_mv.col as i32);
        let left_mv = mvp(r, col - hstep);
        let right_mv = mvp(r, col + hstep);
        let bottom_mv = mvp(r + hstep, col);
        let top_mv = mvp(r - hstep, col);

        let bottom_left_mv = mvp(r + hstep, col - hstep);
        let bottom_right_mv = mvp(r + hstep, col + hstep);
        let top_left_mv = mvp(r - hstep, col - hstep);
        let top_right_mv = mvp(r - hstep, col + hstep);

        let mut dummy = 0;

        let mvs = match whichdir {
            0 => [left_mv, bottom_mv, bottom_left_mv], // bottom left quadrant
            1 => [right_mv, bottom_mv, bottom_right_mv], // bottom right quadrant
            2 => [left_mv, top_mv, top_left_mv],       // top left quadrant
            _ => [right_mv, top_mv, top_right_mv],     // top right quadrant
        };
        for mv in mvs {
            check_better_fast(&c, &mv, bestmv, &mut best, &mut dummy)?;
        }
    } else {
        two_level_checks_fast(&c, start_mv, bestmv, hstep, &mut best, iters_per_step)?;
    }

    // Each subsequent iteration checks at least one point in common with
    // the last iteration could be 2 ( if diag selected) 1/4 pel
    if forced_stop < HALF_PEL {
        hstep >>= 1;
        start_mv = *bestmv;
        two_level_checks_fast(&c, start_mv, bestmv, hstep, &mut best, iters_per_step)?;
    }

    if allow_hp && forced_stop == EIGHTH_PEL {
        hstep >>= 1;
        start_mv = *bestmv;
        two_level_checks_fast(&c, start_mv, bestmv, hstep, &mut best, iters_per_step)?;
    }

    Ok(finish(&best, sse1, distortion))
}

/// `cpi->mv_search_params.find_fractional_mv_step`: the subpel search of
/// `SUBPEL_SEARCH_METHODS` value `method`. `Err` for the other searches
/// (`SUBPEL_TREE` and the like), which the real-time speed features never
/// use.
pub(crate) fn find_fractional_mv_step(
    method: i32,
    xd: &MacroBlockD,
    cm: &Av1Common,
    ms_params: &SubpelMotionSearchParams,
    start_mv: Mv,
    bestmv: &mut Mv,
    distortion: &mut i32,
    sse1: &mut u32,
) -> Result<i32, ()> {
    match method {
        SUBPEL_TREE_PRUNED => av1_find_best_sub_pixel_tree_pruned(
            xd, cm, ms_params, start_mv, bestmv, distortion, sse1,
        ),
        SUBPEL_TREE_PRUNED_MORE => av1_find_best_sub_pixel_tree_pruned_more(
            xd, cm, ms_params, start_mv, bestmv, distortion, sse1,
        ),
        _ => Err(()),
    }
}
