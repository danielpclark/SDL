// Rust translation of av1/common/seg_common.c and av1/common/seg_common.h
// from aom (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The segmentation parameters. The saver never enables segmentation, so
//! only the queries and the resets it makes are here.

pub(crate) const MAX_SEGMENTS: usize = 8;
pub(crate) const SEG_TREE_PROBS: usize = MAX_SEGMENTS - 1;

pub(crate) const SEG_TEMPORAL_PRED_CTXS: usize = 3;
pub(crate) const SPATIAL_PREDICTION_PROBS: usize = 3;

// (SEG_LVL_FEATURES)
pub(crate) type SegLvlFeatures = i32;
pub(crate) const SEG_LVL_ALT_Q: SegLvlFeatures = 0; // Use alternate Quantizer ....
pub(crate) const SEG_LVL_ALT_LF_Y_V: SegLvlFeatures = 1; // Use alternate loop filter value on y plane vertical
pub(crate) const SEG_LVL_ALT_LF_Y_H: SegLvlFeatures = 2; // Use alternate loop filter value on y plane horizontal
pub(crate) const SEG_LVL_ALT_LF_U: SegLvlFeatures = 3; // Use alternate loop filter value on u plane
pub(crate) const SEG_LVL_ALT_LF_V: SegLvlFeatures = 4; // Use alternate loop filter value on v plane
pub(crate) const SEG_LVL_REF_FRAME: SegLvlFeatures = 5; // Optional Segment reference frame
pub(crate) const SEG_LVL_SKIP: SegLvlFeatures = 6; // Optional Segment (0,0) + skip mode
pub(crate) const SEG_LVL_GLOBALMV: SegLvlFeatures = 7;
pub(crate) const SEG_LVL_MAX: usize = 8;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Segmentation {
    pub(crate) enabled: u8,
    pub(crate) update_map: u8,
    pub(crate) update_data: u8,
    pub(crate) temporal_update: u8,

    pub(crate) feature_data: [[i16; SEG_LVL_MAX]; MAX_SEGMENTS],
    pub(crate) feature_mask: [u32; MAX_SEGMENTS],
    pub(crate) last_active_segid: i32, // The highest numbered segment id that has some
    // enabled feature.
    pub(crate) segid_preskip: u8, // Whether the segment id will be read before the
                                  // skip syntax element.
                                  // 1: the segment id will be read first.
                                  // 0: the skip syntax element will be read first.
}

#[inline]
pub(crate) fn segfeature_active(
    seg: &Segmentation,
    segment_id: u8,
    feature_id: SegLvlFeatures,
) -> bool {
    seg.enabled != 0 && (seg.feature_mask[segment_id as usize] & (1 << feature_id)) != 0
}

#[inline]
pub(crate) fn segfeatures_copy(dst: &mut Segmentation, src: &Segmentation) {
    for i in 0..MAX_SEGMENTS {
        dst.feature_mask[i] = src.feature_mask[i];
        for j in 0..SEG_LVL_MAX {
            dst.feature_data[i][j] = src.feature_data[i][j];
        }
    }
    dst.segid_preskip = src.segid_preskip;
    dst.last_active_segid = src.last_active_segid;
}

#[inline]
pub(crate) fn get_segdata(seg: &Segmentation, segment_id: i32, feature_id: SegLvlFeatures) -> i32 {
    seg.feature_data[segment_id as usize][feature_id as usize] as i32
}

// (seg_common.c)

// These functions provide access to new segment level features.
// Eventually these function may be "optimized out" but for the moment,
// the coding mechanism is still subject to change so these provide a
// convenient single point of change.

pub(crate) fn av1_clearall_segfeatures(seg: &mut Segmentation) {
    seg.feature_data = [[0; SEG_LVL_MAX]; MAX_SEGMENTS];
    seg.feature_mask = [0; MAX_SEGMENTS];
}
