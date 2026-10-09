// Rust translation of lib/jxl/dec_patch_dictionary.h,
// lib/jxl/dec_patch_dictionary.cc and lib/jxl/patch_dictionary_internal.h
// from libjxl (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Chooses reference patches, and avoids encoding them once per occurrence.
//! (Upstream, the dictionary points to the shared state; here the parts of
//! it the dictionary reads are passed to `decode` and `add_one_row`.)

use super::base::{jxl_failure, unpack_signed, Status, K_MAX_NUM_REFERENCE_FRAMES};
use super::blending::perform_blending;
use super::dec_ans::{decode_histograms, AnsCode, AnsSymbolReader};
use super::dec_bit_reader::BitReader;
use super::image_metadata::ExtraChannelInfo;
use super::passes_state::ReferenceFrame;

// --- patch_dictionary_internal.h ---

const K_NUM_REF_PATCH_CONTEXT: usize = 0;
const K_REFERENCE_FRAME_CONTEXT: usize = 1;
const K_PATCH_SIZE_CONTEXT: usize = 2;
const K_PATCH_REFERENCE_POSITION_CONTEXT: usize = 3;
const K_PATCH_POSITION_CONTEXT: usize = 4;
const K_PATCH_BLEND_MODE_CONTEXT: usize = 5;
const K_PATCH_OFFSET_CONTEXT: usize = 6;
const K_PATCH_COUNT_CONTEXT: usize = 7;
const K_PATCH_ALPHA_CHANNEL_CONTEXT: usize = 8;
const K_PATCH_CLAMP_CONTEXT: usize = 9;
const K_NUM_PATCH_DICTIONARY_CONTEXTS: usize = 10;

// --- dec_patch_dictionary.h ---

/// Translation of `PatchBlendMode`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PatchBlendMode {
    None = 0,
    Replace = 1,
    Add = 2,
    Mul = 3,
    BlendAbove = 4,
    BlendBelow = 5,
    AlphaWeightedAddAbove = 6,
    AlphaWeightedAddBelow = 7,
}

const K_NUM_BLEND_MODES: u32 = 8;

impl PatchBlendMode {
    fn from_u32(v: u32) -> PatchBlendMode {
        match v {
            0 => PatchBlendMode::None,
            1 => PatchBlendMode::Replace,
            2 => PatchBlendMode::Add,
            3 => PatchBlendMode::Mul,
            4 => PatchBlendMode::BlendAbove,
            5 => PatchBlendMode::BlendBelow,
            6 => PatchBlendMode::AlphaWeightedAddAbove,
            _ => PatchBlendMode::AlphaWeightedAddBelow,
        }
    }
}

/// Translation of `UsesAlpha()`.
#[inline]
pub(crate) fn uses_alpha(mode: PatchBlendMode) -> bool {
    mode == PatchBlendMode::BlendAbove
        || mode == PatchBlendMode::BlendBelow
        || mode == PatchBlendMode::AlphaWeightedAddAbove
        || mode == PatchBlendMode::AlphaWeightedAddBelow
}

/// Translation of `UsesClamp()`.
#[inline]
pub(crate) fn uses_clamp(mode: PatchBlendMode) -> bool {
    uses_alpha(mode) || mode == PatchBlendMode::Mul
}

/// Translation of `PatchBlending`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PatchBlending {
    pub mode: PatchBlendMode,
    pub alpha_channel: u32,
    pub clamp: bool,
}

/// Translation of `PatchReferencePosition`.
#[derive(Clone, Copy, Debug, Default)]
struct PatchReferencePosition {
    ref_: usize,
    x0: usize,
    y0: usize,
    xsize: usize,
    ysize: usize,
}

/// Translation of `PatchPosition`.
#[derive(Clone, Copy, Debug, Default)]
struct PatchPosition {
    x: usize,
    y: usize,
    ref_pos_idx: usize,
}

#[derive(Clone, Copy, Debug, Default)]
struct PatchTreeNode {
    left_child: isize,
    right_child: isize,
    y_center: usize,
    start: usize,
    num: usize,
}

/// Translation of `PatchDictionary`.
#[derive(Clone, Debug, Default)]
pub(crate) struct PatchDictionary {
    positions: Vec<PatchPosition>,
    ref_positions: Vec<PatchReferencePosition>,
    blendings: Vec<PatchBlending>,

    patch_tree: Vec<PatchTreeNode>,
    num_patches: Vec<usize>,
    sorted_patches_y0: Vec<(usize, usize)>,
    sorted_patches_y1: Vec<(usize, usize)>,
}

#[derive(Clone, Copy)]
struct PatchInterval {
    idx: usize,
    y0: usize,
    y1: usize,
}

impl PatchDictionary {
    #[allow(dead_code)]
    pub(crate) fn has_any(&self) -> bool {
        !self.positions.is_empty()
    }

    /// Translation of `Clear()`.
    pub(crate) fn clear(&mut self) {
        self.positions.clear();
        self.compute_patch_tree();
    }

    /// Translation of `Decode()`.
    pub(crate) fn decode(
        &mut self,
        br: &mut BitReader<'_>,
        xsize: usize,
        ysize: usize,
        uses_extra_channels: &mut bool,
        num_extra_channels: usize,
        extra_channel_info: &[ExtraChannelInfo],
        reference_frames: &[ReferenceFrame; 4],
    ) -> Status {
        self.positions.clear();
        let mut context_map: Vec<u8> = Vec::new();
        let mut code = AnsCode::default();
        decode_histograms(
            br,
            K_NUM_PATCH_DICTIONARY_CONTEXTS,
            &mut code,
            &mut context_map,
            false,
        )?;
        let mut decoder = AnsSymbolReader::new(&code, br, 0);

        let mut read_num = |context: usize, br: &mut BitReader<'_>| -> usize {
            decoder.read_hybrid_uint(context, br, &context_map)
        };

        let num_ref_patch = read_num(K_NUM_REF_PATCH_CONTEXT, br);
        // Limit max memory usage of patches to about 66 bytes per pixel (assuming 8
        // bytes per size_t)
        let num_pixels = xsize.wrapping_mul(ysize);
        let max_ref_patches = 1024 + num_pixels / 4;
        let max_patches = max_ref_patches * 4;
        let max_blending_infos = max_patches * 4;
        if num_ref_patch > max_ref_patches {
            return jxl_failure!("Too many patches in dictionary");
        }
        let num_ec = num_extra_channels;

        let mut total_patches: usize = 0;
        let mut next_size: usize = 1;

        for _id in 0..num_ref_patch {
            let mut ref_pos = PatchReferencePosition {
                ref_: read_num(K_REFERENCE_FRAME_CONTEXT, br),
                ..Default::default()
            };
            if ref_pos.ref_ >= K_MAX_NUM_REFERENCE_FRAMES
                || reference_frames[ref_pos.ref_].frame.xsize() == 0
            {
                return jxl_failure!("Invalid reference frame ID");
            }
            if !reference_frames[ref_pos.ref_].ib_is_in_xyb {
                return jxl_failure!("Patches cannot use frames saved post color transforms");
            }
            let ib = &reference_frames[ref_pos.ref_].frame;
            ref_pos.x0 = read_num(K_PATCH_REFERENCE_POSITION_CONTEXT, br);
            ref_pos.y0 = read_num(K_PATCH_REFERENCE_POSITION_CONTEXT, br);
            ref_pos.xsize = read_num(K_PATCH_SIZE_CONTEXT, br).wrapping_add(1);
            ref_pos.ysize = read_num(K_PATCH_SIZE_CONTEXT, br).wrapping_add(1);
            if ref_pos.x0.wrapping_add(ref_pos.xsize) > ib.xsize() {
                return jxl_failure!("Invalid position specified in reference frame");
            }
            if ref_pos.y0.wrapping_add(ref_pos.ysize) > ib.ysize() {
                return jxl_failure!("Invalid position specified in reference frame");
            }
            let id_count = read_num(K_PATCH_COUNT_CONTEXT, br).wrapping_add(1);
            total_patches = total_patches.wrapping_add(id_count);
            if total_patches > max_patches {
                return jxl_failure!("Too many patches in dictionary");
            }
            if next_size < total_patches {
                next_size *= 2;
                next_size = next_size.min(max_patches);
            }
            if next_size * (num_ec + 1) > max_blending_infos {
                return jxl_failure!("Too many patches in dictionary");
            }
            if self
                .positions
                .try_reserve(next_size.saturating_sub(self.positions.len()))
                .is_err()
            {
                return jxl_failure!("out of memory");
            }
            let want = next_size * (num_ec + 1);
            if self
                .blendings
                .try_reserve(want.saturating_sub(self.blendings.len()))
                .is_err()
            {
                return jxl_failure!("out of memory");
            }
            for i in 0..id_count {
                let mut pos = PatchPosition {
                    ref_pos_idx: self.ref_positions.len(),
                    ..Default::default()
                };
                if i == 0 {
                    pos.x = read_num(K_PATCH_POSITION_CONTEXT, br);
                    pos.y = read_num(K_PATCH_POSITION_CONTEXT, br);
                } else {
                    let back = *self.positions.last().unwrap_or(&PatchPosition::default());
                    let deltax = unpack_signed(read_num(K_PATCH_OFFSET_CONTEXT, br));
                    if deltax < 0 && deltax.unsigned_abs() > back.x {
                        return jxl_failure!("Invalid patch: negative x coordinate");
                    }
                    pos.x = back.x.wrapping_add(deltax as usize);
                    let deltay = unpack_signed(read_num(K_PATCH_OFFSET_CONTEXT, br));
                    if deltay < 0 && deltay.unsigned_abs() > back.y {
                        return jxl_failure!("Invalid patch: negative y coordinate");
                    }
                    pos.y = back.y.wrapping_add(deltay as usize);
                }
                if pos.x.wrapping_add(ref_pos.xsize) > xsize {
                    return jxl_failure!("Invalid patch x");
                }
                if pos.y.wrapping_add(ref_pos.ysize) > ysize {
                    return jxl_failure!("Invalid patch y");
                }
                for j in 0..num_ec + 1 {
                    let blend_mode = read_num(K_PATCH_BLEND_MODE_CONTEXT, br) as u32;
                    if blend_mode >= K_NUM_BLEND_MODES {
                        return jxl_failure!("Invalid patch blend mode");
                    }
                    let mut info = PatchBlending {
                        mode: PatchBlendMode::from_u32(blend_mode),
                        alpha_channel: 0,
                        clamp: false,
                    };
                    if uses_alpha(info.mode) {
                        *uses_extra_channels = true;
                    }
                    if info.mode != PatchBlendMode::None && j > 0 {
                        *uses_extra_channels = true;
                    }
                    if uses_alpha(info.mode) && extra_channel_info.len() > 1 {
                        info.alpha_channel = read_num(K_PATCH_ALPHA_CHANNEL_CONTEXT, br) as u32;
                        if info.alpha_channel as usize >= extra_channel_info.len() {
                            return jxl_failure!("Invalid alpha channel for blending");
                        }
                    } else {
                        info.alpha_channel = 0;
                    }
                    if uses_clamp(info.mode) {
                        info.clamp = read_num(K_PATCH_CLAMP_CONTEXT, br) != 0;
                    } else {
                        info.clamp = false;
                    }
                    self.blendings.push(info);
                }
                self.positions.push(pos);
            }
            self.ref_positions.push(ref_pos);
        }
        self.positions.shrink_to_fit();

        if !decoder.check_ans_final_state() {
            return jxl_failure!("ANS checksum failure.");
        }

        self.compute_patch_tree();
        Ok(())
    }

    /// Translation of `GetReferences()`.
    pub(crate) fn get_references(&self) -> i32 {
        let mut result = 0i32;
        for rp in &self.ref_positions {
            result |= 1 << rp.ref_ as i32;
        }
        result
    }

    /// Translation of `ComputePatchTree()`.
    fn compute_patch_tree(&mut self) {
        self.patch_tree.clear();
        self.num_patches.clear();
        self.sorted_patches_y0.clear();
        self.sorted_patches_y1.clear();
        if self.positions.is_empty() {
            return;
        }
        // Create a y-interval for each patch.
        let mut intervals: Vec<PatchInterval> = self
            .positions
            .iter()
            .enumerate()
            .map(|(i, pos)| PatchInterval {
                idx: i,
                y0: pos.y,
                y1: pos.y + self.ref_positions[pos.ref_pos_idx].ysize,
            })
            .collect();
        let sort_by_y0 = |intervals: &mut Vec<PatchInterval>, start: usize, end: usize| {
            intervals[start..end].sort_by_key(|i0| i0.y0);
        };
        let sort_by_y1 = |intervals: &mut Vec<PatchInterval>, start: usize, end: usize| {
            intervals[start..end].sort_by_key(|i0| i0.y1);
        };
        // Count the number of patches for each row.
        let n = intervals.len();
        sort_by_y1(&mut intervals, 0, n);
        self.num_patches.resize(intervals[n - 1].y1, 0);
        for iv in &intervals {
            for y in iv.y0..iv.y1 {
                self.num_patches[y] += 1;
            }
        }
        self.patch_tree.push(PatchTreeNode {
            start: 0,
            num: n,
            ..Default::default()
        });
        let mut next = 0usize;
        while next < self.patch_tree.len() {
            let start = self.patch_tree[next].start;
            let end = start + self.patch_tree[next].num;
            // Choose the y_center for this node to be the median of interval starts.
            sort_by_y0(&mut intervals, start, end);
            let middle_idx = start + self.patch_tree[next].num / 2;
            let y_center = intervals[middle_idx].y0;
            self.patch_tree[next].y_center = y_center;
            // Divide the intervals in [start, end) into three groups:
            //   * those completely to the right of y_center: [right_start, end)
            //   * those overlapping y_center: [left_end, right_start)
            //   * those completely to the left of y_center: [start, left_end)
            let mut right_start = middle_idx;
            while right_start < end && intervals[right_start].y0 == y_center {
                right_start += 1;
            }
            sort_by_y1(&mut intervals, start, right_start);
            let mut left_end = right_start;
            while left_end > start && intervals[left_end - 1].y1 > y_center {
                left_end -= 1;
            }
            // Fill in sorted_patches_y0_ and sorted_patches_y1_ for the current node.
            self.patch_tree[next].num = right_start - left_end;
            self.patch_tree[next].start = self.sorted_patches_y0.len();
            for i in (left_end..right_start).rev() {
                self.sorted_patches_y1
                    .push((intervals[i].y1, intervals[i].idx));
            }
            sort_by_y0(&mut intervals, left_end, right_start);
            for iv in &intervals[left_end..right_start] {
                self.sorted_patches_y0.push((iv.y0, iv.idx));
            }
            // Create the left and right nodes (if not empty).
            self.patch_tree[next].left_child = -1;
            self.patch_tree[next].right_child = -1;
            if left_end > start {
                let left = PatchTreeNode {
                    start,
                    num: left_end - start,
                    ..Default::default()
                };
                self.patch_tree[next].left_child = self.patch_tree.len() as isize;
                self.patch_tree.push(left);
            }
            if right_start < end {
                let right = PatchTreeNode {
                    start: right_start,
                    num: end - right_start,
                    ..Default::default()
                };
                self.patch_tree[next].right_child = self.patch_tree.len() as isize;
                self.patch_tree.push(right);
            }
            next += 1;
        }
    }

    /// Translation of `GetPatchesForRow()`.
    pub(crate) fn get_patches_for_row(&self, y: usize) -> Vec<usize> {
        let mut result = Vec::new();
        if y < self.num_patches.len() && self.num_patches[y] > 0 {
            result.reserve(self.num_patches[y]);
            let mut tree_idx: isize = 0;
            while tree_idx != -1 {
                debug_assert!(tree_idx < self.patch_tree.len() as isize);
                let node = &self.patch_tree[tree_idx as usize];
                if y <= node.y_center {
                    for i in 0..node.num {
                        let p = self.sorted_patches_y0[node.start + i];
                        if y < p.0 {
                            break;
                        }
                        result.push(p.1);
                    }
                    tree_idx = if y < node.y_center {
                        node.left_child
                    } else {
                        -1
                    };
                } else {
                    for i in 0..node.num {
                        let p = self.sorted_patches_y1[node.start + i];
                        if y >= p.0 {
                            break;
                        }
                        result.push(p.1);
                    }
                    tree_idx = node.right_child;
                }
            }
            // Ensure that he relative order of patches that affect the same pixels is
            // preserved. This is important for patches that have a blend mode
            // different from kAdd.
            result.sort_unstable();
        }
        result
    }

    /// Adds patches to a segment of `xsize` pixels, starting at `inout`,
    /// assumed to be located at position (x0, y) in the frame. Translation of
    /// `AddOneRow()` (`inout[c]` holds the `xsize` pixels of channel `c`).
    pub(crate) fn add_one_row(
        &self,
        inout: &mut [Vec<f32>],
        y: usize,
        x0: usize,
        xsize: usize,
        num_extra_channels: usize,
        extra_channel_info: &[ExtraChannelInfo],
        reference_frames: &[ReferenceFrame; 4],
    ) {
        let num_ec = num_extra_channels;
        for pos_idx in self.get_patches_for_row(y) {
            let blending_idx = pos_idx * (num_ec + 1);
            let pos = &self.positions[pos_idx];
            let ref_pos = &self.ref_positions[pos.ref_pos_idx];
            let by = pos.y;
            let bx = pos.x;
            let patch_xsize = ref_pos.xsize;
            debug_assert!(y >= by);
            debug_assert!(y < by + ref_pos.ysize);
            let iy = y - by;
            let ref_ = ref_pos.ref_;
            if bx >= x0 + xsize {
                continue;
            }
            if bx + patch_xsize < x0 {
                continue;
            }
            let patch_x0 = bx.max(x0);
            let patch_x1 = (bx + patch_xsize).min(x0 + xsize);
            let n = patch_x1 - patch_x0;
            // (fg_ptrs[c] + patch_x0 - x0 is the reference pixel at
            // ref_pos.x0 + patch_x0 - bx.)
            let fx = ref_pos.x0 + patch_x0 - bx;
            let frame = &reference_frames[ref_].frame;
            let mut fg: Vec<&[f32]> = Vec::with_capacity(3 + num_ec);
            for c in 0..3 {
                fg.push(&frame.color().const_plane_row(c, ref_pos.y0 + iy)[fx..fx + n]);
            }
            for i in 0..num_ec {
                fg.push(&frame.extra_channels()[i].row(ref_pos.y0 + iy)[fx..fx + n]);
            }
            let ox = patch_x0 - x0;
            let out = {
                let bg: Vec<&[f32]> = inout.iter().map(|r| &r[ox..ox + n]).collect();
                perform_blending(
                    &bg,
                    &fg,
                    n,
                    &self.blendings[blending_idx],
                    &self.blendings[blending_idx + 1..],
                    extra_channel_info,
                )
            };
            for (i, o) in out.iter().enumerate() {
                inout[i][ox..ox + n].copy_from_slice(o);
            }
        }
    }
}
