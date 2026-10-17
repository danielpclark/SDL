// Rust translation of the parts of av1/encoder/mcomp.c, mcomp.h and
// mcomp_structs.h from aom (https://aomedia.googlesource.com/aom, at the
// revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's patches)
// the real-time encoder reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Motion search.

use crate::aom::av1::common::mv::FullpelMv;

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
