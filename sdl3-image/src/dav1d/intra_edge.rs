// Rust translation of src/intra_edge.c and src/intra_edge.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins).
// Copyright © 2018-2023, VideoLAN and dav1d authors
// Copyright © 2018-2023, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The tree of edge availability flags of each partition of a superblock.
//!
//! Upstream links the nodes with 16-bit byte offsets in one static block
//! of memory (`INTRA_EDGE_SPLIT()`); here the branches and the tips are
//! two arrays per superblock size and a split is an index into the next
//! level's array (the tips' when the parent is a 16x16 branch).

use std::sync::OnceLock;

use super::levels::*;

// enum EdgeFlags
pub(crate) const EDGE_I444_TOP_HAS_RIGHT: u8 = 1 << 0;
pub(crate) const EDGE_I422_TOP_HAS_RIGHT: u8 = 1 << 1;
pub(crate) const EDGE_I420_TOP_HAS_RIGHT: u8 = 1 << 2;
pub(crate) const EDGE_I444_LEFT_HAS_BOTTOM: u8 = 1 << 3;
pub(crate) const EDGE_I422_LEFT_HAS_BOTTOM: u8 = 1 << 4;
pub(crate) const EDGE_I420_LEFT_HAS_BOTTOM: u8 = 1 << 5;
pub(crate) const EDGE_ALL_TOP_HAS_RIGHT: u8 =
    EDGE_I444_TOP_HAS_RIGHT | EDGE_I422_TOP_HAS_RIGHT | EDGE_I420_TOP_HAS_RIGHT;
pub(crate) const EDGE_ALL_LEFT_HAS_BOTTOM: u8 =
    EDGE_I444_LEFT_HAS_BOTTOM | EDGE_I422_LEFT_HAS_BOTTOM | EDGE_I420_LEFT_HAS_BOTTOM;
pub(crate) const EDGE_ALL_TR_AND_BL: u8 = EDGE_ALL_TOP_HAS_RIGHT | EDGE_ALL_LEFT_HAS_BOTTOM;

/// Translation of `EdgeNode`, with the extra members of `EdgeTip`
/// (`split`) and `EdgeBranch` (`h4`, `v4`, `split_offset`, here the index
/// of each split child).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct EdgeNode {
    pub(crate) o: u8,
    pub(crate) h: [u8; 2],
    pub(crate) v: [u8; 2],
    // EdgeTip
    pub(crate) split: [u8; 3],
    // EdgeBranch
    pub(crate) h4: u8,
    pub(crate) v4: u8,
    pub(crate) split_offset: [u16; 4],
}

/// The node arrays of one superblock size: `levels[bl]` holds the nodes
/// of block level `bl` (the 8x8 level holds the tips).
pub(crate) struct EdgeTree {
    pub(crate) levels: [Vec<EdgeNode>; N_BL_LEVELS],
}

/// Translation of `INTRA_EDGE_SPLIT()`: the level and index of the `i`th
/// split child of node `idx` at level `bl`.
#[inline]
pub(crate) fn intra_edge_split(tree: &EdgeTree, bl: u8, idx: usize, i: usize) -> usize {
    tree.levels[bl as usize][idx].split_offset[i] as usize
}

fn init_edges(node: &mut EdgeNode, bl: u8, edge_flags: u8) {
    node.o = edge_flags;
    node.h[0] = edge_flags | EDGE_ALL_LEFT_HAS_BOTTOM;
    node.v[0] = edge_flags | EDGE_ALL_TOP_HAS_RIGHT;

    if bl == BL_8X8 {
        let nt = node;

        nt.h[1] = edge_flags & (EDGE_ALL_LEFT_HAS_BOTTOM | EDGE_I420_TOP_HAS_RIGHT);
        nt.v[1] = edge_flags
            & (EDGE_ALL_TOP_HAS_RIGHT | EDGE_I420_LEFT_HAS_BOTTOM | EDGE_I422_LEFT_HAS_BOTTOM);

        nt.split[0] = (edge_flags & EDGE_ALL_TOP_HAS_RIGHT) | EDGE_I422_LEFT_HAS_BOTTOM;
        nt.split[1] = edge_flags | EDGE_I444_TOP_HAS_RIGHT;
        nt.split[2] = edge_flags
            & (EDGE_I420_TOP_HAS_RIGHT | EDGE_I420_LEFT_HAS_BOTTOM | EDGE_I422_LEFT_HAS_BOTTOM);
    } else {
        let nwc = node;

        nwc.h[1] = edge_flags & EDGE_ALL_LEFT_HAS_BOTTOM;
        nwc.v[1] = edge_flags & EDGE_ALL_TOP_HAS_RIGHT;

        nwc.h4 = EDGE_ALL_LEFT_HAS_BOTTOM;
        nwc.v4 = EDGE_ALL_TOP_HAS_RIGHT;
        if bl == BL_16X16 {
            nwc.h4 |= edge_flags & EDGE_I420_TOP_HAS_RIGHT;
            nwc.v4 |= edge_flags & (EDGE_I420_LEFT_HAS_BOTTOM | EDGE_I422_LEFT_HAS_BOTTOM);
        }
    }
}

/// `struct ModeSelMem`: the next free node of each level.
struct ModeSelMem {
    nwc: [usize; 3],
    nt: usize,
}

fn init_mode_node(
    tree: &mut EdgeTree,
    idx: usize,
    bl: u8,
    mem: &mut ModeSelMem,
    top_has_right: bool,
    left_has_bottom: bool,
) {
    init_edges(
        &mut tree.levels[bl as usize][idx],
        bl,
        (if top_has_right {
            EDGE_ALL_TOP_HAS_RIGHT
        } else {
            0
        }) | (if left_has_bottom {
            EDGE_ALL_LEFT_HAS_BOTTOM
        } else {
            0
        }),
    );
    if bl == BL_16X16 {
        for n in 0..4 {
            let nt = mem.nt;
            mem.nt += 1;
            tree.levels[bl as usize][idx].split_offset[n] = nt as u16;
            init_edges(
                &mut tree.levels[(bl + 1) as usize][nt],
                bl + 1,
                (if n == 3 || (n == 1 && !top_has_right) {
                    0
                } else {
                    EDGE_ALL_TOP_HAS_RIGHT
                }) | (if !(n == 0 || (n == 2 && left_has_bottom)) {
                    0
                } else {
                    EDGE_ALL_LEFT_HAS_BOTTOM
                }),
            );
        }
    } else {
        for n in 0..4 {
            let nwc_child = mem.nwc[bl as usize];
            mem.nwc[bl as usize] += 1;
            tree.levels[bl as usize][idx].split_offset[n] = nwc_child as u16;
            init_mode_node(
                tree,
                nwc_child,
                bl + 1,
                mem,
                !(n == 3 || (n == 1 && !top_has_right)),
                n == 0 || (n == 2 && left_has_bottom),
            );
        }
    }
}

/// Translation of `dav1d_init_intra_edge_tree()`: the trees of both
/// superblock sizes (`dav1d_intra_edge_tree[BL_128X128]` and
/// `[BL_64X64]`).
fn init_intra_edge_tree() -> [EdgeTree; 2] {
    // The arrays of each level: branch_sb128[1 + 4 + 16 + 64] split by
    // level, tip_sb128[256]; branch_sb64[1 + 4 + 16], tip_sb64[64].
    let new = |sizes: [usize; N_BL_LEVELS]| EdgeTree {
        levels: sizes.map(|n| vec![EdgeNode::default(); n]),
    };
    let mut sb128 = new([1, 4, 16, 64, 256]);
    let mut sb64 = new([0, 1, 4, 16, 64]);

    let mut mem = ModeSelMem { nwc: [0; 3], nt: 0 };
    init_mode_node(&mut sb128, 0, BL_128X128, &mut mem, true, false);
    debug_assert!(mem.nwc == [4, 16, 64] && mem.nt == 256);

    let mut mem = ModeSelMem { nwc: [0; 3], nt: 0 };
    init_mode_node(&mut sb64, 0, BL_64X64, &mut mem, true, false);
    debug_assert!(mem.nwc == [0, 4, 16] && mem.nt == 64);

    [sb128, sb64]
}

static INTRA_EDGE_TREE: OnceLock<[EdgeTree; 2]> = OnceLock::new();

/// `dav1d_intra_edge_tree[root_bl]`.
pub(crate) fn intra_edge_tree(root_bl: u8) -> &'static EdgeTree {
    &INTRA_EDGE_TREE.get_or_init(init_intra_edge_tree)[root_bl as usize]
}
