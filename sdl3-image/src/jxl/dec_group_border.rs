// Rust translation of lib/jxl/dec_group_border.h and
// lib/jxl/dec_group_border.cc from libjxl (https://github.com/libjxl/libjxl,
// at the revision SDL_image's external/libjxl pins: libjxl 0.7.3 with SDL's
// patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Assigns the areas between groups to the group that completes them.

use super::base::{FrameDimensions, K_BLOCK_DIM};
use super::image::Rect;

/// Translation of `GroupBorderAssigner` (single-threaded: the counters are
/// plain bytes).
#[derive(Clone, Debug, Default)]
pub(crate) struct GroupBorderAssigner {
    frame_dim: FrameDimensions,
    counters: Vec<u8>,
}

// Constants to identify group positions relative to the corners.
const K_TOP_LEFT: u8 = 0x01;
const K_TOP_RIGHT: u8 = 0x02;
const K_BOTTOM_RIGHT: u8 = 0x04;
const K_BOTTOM_LEFT: u8 = 0x08;

impl GroupBorderAssigner {
    pub(crate) const K_MAX_TO_FINALIZE: usize = 3;

    /// Prepare the GroupBorderAssigner to handle a given frame.
    pub(crate) fn init(&mut self, frame_dim: &FrameDimensions) {
        self.frame_dim = frame_dim.clone();
        let num_corners = (self.frame_dim.xsize_groups + 1) * (self.frame_dim.ysize_groups + 1);
        self.counters = vec![0; num_corners];
        // Initialize counters.
        for y in 0..self.frame_dim.ysize_groups + 1 {
            for x in 0..self.frame_dim.xsize_groups + 1 {
                // Counters at image borders don't have anything on the other side, we
                // pre-fill their value to have more uniform handling afterwards.
                let mut init_value: u8 = 0;
                if x == 0 {
                    init_value |= K_TOP_LEFT | K_BOTTOM_LEFT;
                }
                if x == self.frame_dim.xsize_groups {
                    init_value |= K_TOP_RIGHT | K_BOTTOM_RIGHT;
                }
                if y == 0 {
                    init_value |= K_TOP_LEFT | K_TOP_RIGHT;
                }
                if y == self.frame_dim.ysize_groups {
                    init_value |= K_BOTTOM_LEFT | K_BOTTOM_RIGHT;
                }
                self.counters[y * (self.frame_dim.xsize_groups + 1) + x] = init_value;
            }
        }
    }

    /// Marks a group as not-done, for running re-paints.
    pub(crate) fn clear_done(&mut self, group_id: usize) {
        let x = group_id % self.frame_dim.xsize_groups;
        let y = group_id / self.frame_dim.xsize_groups;
        let top_left_idx = y * (self.frame_dim.xsize_groups + 1) + x;
        let top_right_idx = y * (self.frame_dim.xsize_groups + 1) + x + 1;
        let bottom_right_idx = (y + 1) * (self.frame_dim.xsize_groups + 1) + x + 1;
        let bottom_left_idx = (y + 1) * (self.frame_dim.xsize_groups + 1) + x;
        self.counters[top_left_idx] &= !K_BOTTOM_RIGHT;
        self.counters[top_right_idx] &= !K_BOTTOM_LEFT;
        self.counters[bottom_left_idx] &= !K_TOP_RIGHT;
        self.counters[bottom_right_idx] &= !K_TOP_LEFT;
    }

    // Looking at each corner between groups, we can guarantee that the four
    // involved groups will agree between each other regarding the order in which
    // each of the four groups terminated. Thus, the last of the four groups
    // gets the responsibility of handling the corner. For borders, every border
    // is assigned to its top corner (for vertical borders) or to its left corner
    // (for horizontal borders): the order as seen on those corners will decide who
    // handles that border.

    /// Marks a group as done, and returns the (at most 3) rects to run
    /// FinalizeImageRect on. Translation of `GroupDone()`.
    pub(crate) fn group_done(&mut self, group_id: usize, padx: usize, pady: usize) -> Vec<Rect> {
        let fd = &self.frame_dim;
        let x = group_id % fd.xsize_groups;
        let y = group_id / fd.xsize_groups;
        let block_rect = Rect::new_clamped(
            x * fd.group_dim / K_BLOCK_DIM,
            y * fd.group_dim / K_BLOCK_DIM,
            fd.group_dim / K_BLOCK_DIM,
            fd.group_dim / K_BLOCK_DIM,
            fd.xsize_blocks,
            fd.ysize_blocks,
        );

        let top_left_idx = y * (fd.xsize_groups + 1) + x;
        let top_right_idx = y * (fd.xsize_groups + 1) + x + 1;
        let bottom_right_idx = (y + 1) * (fd.xsize_groups + 1) + x + 1;
        let bottom_left_idx = (y + 1) * (fd.xsize_groups + 1) + x;

        let counters = &mut self.counters;
        let mut fetch_status = |idx: usize, bit: u8| -> u8 {
            let status = counters[idx];
            counters[idx] |= bit;
            debug_assert!((bit & status) == 0);
            bit | status
        };

        let top_left_status = fetch_status(top_left_idx, K_BOTTOM_RIGHT);
        let top_right_status = fetch_status(top_right_idx, K_BOTTOM_LEFT);
        let bottom_right_status = fetch_status(bottom_right_idx, K_TOP_LEFT);
        let bottom_left_status = fetch_status(bottom_left_idx, K_TOP_RIGHT);

        let fd = &self.frame_dim;
        let x1 = block_rect.x0() + block_rect.xsize();
        let y1 = block_rect.y0() + block_rect.ysize();

        let is_last_group_x = fd.xsize_groups == x + 1;
        let is_last_group_y = fd.ysize_groups == y + 1;

        // Start of border of neighbouring group, end of border of this group, start
        // of border of this group (on the other side), end of border of next group.
        let xpos: [usize; 4] = [
            if block_rect.x0() == 0 {
                0
            } else {
                (block_rect.x0() * K_BLOCK_DIM).wrapping_sub(padx)
            },
            if block_rect.x0() == 0 {
                0
            } else {
                fd.xsize.min(block_rect.x0() * K_BLOCK_DIM + padx)
            },
            if is_last_group_x {
                fd.xsize
            } else {
                (x1 * K_BLOCK_DIM).wrapping_sub(padx)
            },
            fd.xsize.min(x1 * K_BLOCK_DIM + padx),
        ];
        let ypos: [usize; 4] = [
            if block_rect.y0() == 0 {
                0
            } else {
                (block_rect.y0() * K_BLOCK_DIM).wrapping_sub(pady)
            },
            if block_rect.y0() == 0 {
                0
            } else {
                fd.ysize.min(block_rect.y0() * K_BLOCK_DIM + pady)
            },
            if is_last_group_y {
                fd.ysize
            } else {
                (y1 * K_BLOCK_DIM).wrapping_sub(pady)
            },
            fd.ysize.min(y1 * K_BLOCK_DIM + pady),
        ];

        let mut rects_to_finalize = Vec::with_capacity(Self::K_MAX_TO_FINALIZE);
        let mut append_rect = |x0: usize, x1: usize, y0: usize, y1: usize| {
            let rect = Rect::new(
                xpos[x0],
                ypos[y0],
                xpos[x1].wrapping_sub(xpos[x0]),
                ypos[y1].wrapping_sub(ypos[y0]),
            );
            if rect.xsize() == 0 || rect.ysize() == 0 {
                return;
            }
            debug_assert!(rects_to_finalize.len() < Self::K_MAX_TO_FINALIZE);
            rects_to_finalize.push(rect);
        };

        // Because of how group borders are assigned, it is impossible that we need to
        // process the left and right side of some area but not the center area. Thus,
        // we compute the first/last part to process in every horizontal strip and
        // merge them together. We first collect a mask of what parts should be
        // processed.
        // We do this horizontally rather than vertically because horizontal borders
        // are larger.
        let mut available_parts_mask = [[false; 3]; 3]; // [x][y]
        // Center
        available_parts_mask[1][1] = true;
        // Corners
        if top_left_status == 0xF {
            available_parts_mask[0][0] = true;
        }
        if top_right_status == 0xF {
            available_parts_mask[2][0] = true;
        }
        if bottom_right_status == 0xF {
            available_parts_mask[2][2] = true;
        }
        if bottom_left_status == 0xF {
            available_parts_mask[0][2] = true;
        }
        // Other borders
        if top_left_status & K_TOP_RIGHT != 0 {
            available_parts_mask[1][0] = true;
        }
        if top_left_status & K_BOTTOM_LEFT != 0 {
            available_parts_mask[0][1] = true;
        }
        if top_right_status & K_BOTTOM_RIGHT != 0 {
            available_parts_mask[2][1] = true;
        }
        if bottom_left_status & K_BOTTOM_RIGHT != 0 {
            available_parts_mask[1][2] = true;
        }

        // Collect horizontal ranges.
        const K_NO_SEGMENT: usize = 3;
        let mut horizontal_segments = [(K_NO_SEGMENT, K_NO_SEGMENT); 3];
        for y in 0..3 {
            for x in 0..3 {
                if !available_parts_mask[x][y] {
                    continue;
                }
                debug_assert!(horizontal_segments[y].1 == K_NO_SEGMENT || horizontal_segments[y].1 == x);
                if horizontal_segments[y].0 == K_NO_SEGMENT {
                    horizontal_segments[y].0 = x;
                }
                horizontal_segments[y].1 = x + 1;
            }
        }
        if horizontal_segments[0] == horizontal_segments[1] && horizontal_segments[0] == horizontal_segments[2] {
            append_rect(horizontal_segments[0].0, horizontal_segments[0].1, 0, 3);
        } else if horizontal_segments[0] == horizontal_segments[1] {
            append_rect(horizontal_segments[0].0, horizontal_segments[0].1, 0, 2);
            append_rect(horizontal_segments[2].0, horizontal_segments[2].1, 2, 3);
        } else if horizontal_segments[1] == horizontal_segments[2] {
            append_rect(horizontal_segments[0].0, horizontal_segments[0].1, 0, 1);
            append_rect(horizontal_segments[1].0, horizontal_segments[1].1, 1, 3);
        } else {
            append_rect(horizontal_segments[0].0, horizontal_segments[0].1, 0, 1);
            append_rect(horizontal_segments[1].0, horizontal_segments[1].1, 1, 2);
            append_rect(horizontal_segments[2].0, horizontal_segments[2].1, 2, 3);
        }
        rects_to_finalize
    }
}
