// Rust translation of lib/jxl/modular/encoding/encoding.h and
// lib/jxl/modular/encoding/encoding.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The modular image decoder: the group header, the tree filtering and the
//! channel decoding with its fast paths.

use std::collections::VecDeque;

use super::super::super::base::{
    div_ceil, jxl_failure, unpack_signed, Status, StatusCode, K_BLOCK_DIM,
};
use super::super::super::dec_ans::{decode_histograms, AnsCode, AnsSymbolReader};
use super::super::super::dec_bit_reader::BitReader;
use super::super::super::fields::{bits_offset, bundle_init, bundle_read, val, Fields, Visitor};
use super::super::super::image::zero_fill_image;
use super::super::modular_image::{Channel, Image, PixelType, PixelTypeW};
use super::super::options::{ModularOptions, Predictor, Properties, K_NUM_STATIC_PROPERTIES};
use super::super::transform::transform::Transform;
use super::context_predict::{
    clamped_gradient, init_props_row, precompute_references, predict_no_tree_no_wp,
    predict_no_tree_wp, predict_tree_no_wp, predict_tree_no_wp_nec, predict_tree_wp, weighted,
    FlatDecisionNode, FlatTree, MaTreeLookup, K_EXTRA_PROPS_PER_CHANNEL, K_GRADIENT_PROP,
    K_NUM_NONREF_PROPERTIES, K_WP_PROP,
};
use super::dec_ma::{decode_tree, Tree};

// Valid range of properties for using lookup tables instead of trees.
pub(crate) const K_PROP_RANGE_FAST: i32 = 512;

/// Translation of `GroupHeader`.
#[derive(Clone, Debug)]
pub(crate) struct GroupHeader {
    pub use_global_tree: bool,
    pub wp_header: weighted::Header,

    pub transforms: Vec<Transform>,
}

impl GroupHeader {
    pub(crate) fn new() -> Self {
        let mut s = GroupHeader {
            use_global_tree: false,
            wp_header: weighted::Header::new(),
            transforms: Vec::new(),
        };
        bundle_init(&mut s);
        s
    }
}

impl Fields for GroupHeader {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        visitor.bool_(false, &mut self.use_global_tree)?;
        visitor.visit_nested(&mut self.wp_header)?;
        let mut num_transforms = self.transforms.len() as u32;
        visitor.u32d(
            val(0),
            val(1),
            bits_offset(4, 2),
            bits_offset(8, 18),
            0,
            &mut num_transforms,
        )?;
        if visitor.is_reading() {
            self.transforms
                .resize_with(num_transforms as usize, Transform::default);
        }
        for i in 0..num_transforms as usize {
            visitor.visit_nested(&mut self.transforms[i])?;
        }
        Ok(())
    }
}

/// Removes all nodes that use a static property (i.e. channel or group ID)
/// from the tree and collapses each node on even levels with its two
/// children to produce a flatter tree. Also computes whether the resulting
/// tree requires using the weighted predictor. Translation of
/// `FilterTree()`.
pub(crate) fn filter_tree(
    global_tree: &Tree,
    static_props: &[PixelType; K_NUM_STATIC_PROPERTIES],
    num_props: &mut usize,
    use_wp: &mut bool,
    wp_only: &mut bool,
    gradient_only: &mut bool,
) -> FlatTree {
    *num_props = 0;
    let mut has_wp = false;
    let mut has_non_wp = false;
    *gradient_only = true;
    let mut mark_property = |p: i32, has_wp: &mut bool, has_non_wp: &mut bool| {
        if p == K_WP_PROP as i32 {
            *has_wp = true;
        } else if p >= K_NUM_STATIC_PROPERTIES as i32 {
            *has_non_wp = true;
        }
        if p >= K_NUM_STATIC_PROPERTIES as i32 && p != K_GRADIENT_PROP as i32 {
            *gradient_only = false;
        }
    };
    let mut output: FlatTree = Vec::new();
    let mut nodes: VecDeque<usize> = VecDeque::new();
    nodes.push_back(0);
    // Produces a trimmed and flattened tree by doing a BFS visit of the original
    // tree, ignoring branches that are known to be false and proceeding two
    // levels at a time to collapse nodes in a flatter tree; if an inner parent
    // node has a leaf as a child, the leaf is duplicated and an implicit fake
    // node is added. This allows to reduce the number of branches when traversing
    // the resulting flat tree.
    let skip = |mut cur: usize| -> usize {
        // Skip nodes that we can decide now, by jumping directly to their children.
        while (global_tree[cur].property as i32) < K_NUM_STATIC_PROPERTIES as i32
            && global_tree[cur].property != -1
        {
            if static_props[global_tree[cur].property as usize] > global_tree[cur].splitval {
                cur = global_tree[cur].lchild as usize;
            } else {
                cur = global_tree[cur].rchild as usize;
            }
        }
        cur
    };
    let mut leaf_gradient_only = true;
    while let Some(cur) = nodes.pop_front() {
        let cur = skip(cur);
        let mut flat = FlatDecisionNode::default();
        if global_tree[cur].property == -1 {
            flat.property0 = -1;
            flat.child_id = global_tree[cur].lchild;
            flat.predictor = global_tree[cur].predictor;
            flat.predictor_offset = global_tree[cur].predictor_offset;
            flat.multiplier = global_tree[cur].multiplier as i32;
            leaf_gradient_only &= flat.predictor == Predictor::Gradient;
            has_wp |= flat.predictor == Predictor::Weighted;
            has_non_wp |= flat.predictor != Predictor::Weighted;
            output.push(flat);
            continue;
        }
        flat.child_id = (output.len() + nodes.len() + 1) as u32;

        flat.property0 = global_tree[cur].property as i32;
        *num_props = (*num_props).max(flat.property0 as usize + 1);
        flat.splitval0 = global_tree[cur].splitval;

        for i in 0..2 {
            let cur_child = skip(if i == 0 {
                global_tree[cur].lchild as usize
            } else {
                global_tree[cur].rchild as usize
            });
            // We ended up in a leaf, add a dummy decision and two copies of the leaf.
            if global_tree[cur_child].property == -1 {
                flat.properties[i] = 0;
                flat.splitvals[i] = 0;
                nodes.push_back(cur_child);
                nodes.push_back(cur_child);
            } else {
                flat.properties[i] = global_tree[cur_child].property as i32;
                flat.splitvals[i] = global_tree[cur_child].splitval;
                nodes.push_back(global_tree[cur_child].lchild as usize);
                nodes.push_back(global_tree[cur_child].rchild as usize);
                *num_props = (*num_props).max(flat.properties[i] as usize + 1);
            }
        }

        for j in 0..2 {
            mark_property(flat.properties[j], &mut has_wp, &mut has_non_wp);
        }
        mark_property(flat.property0, &mut has_wp, &mut has_non_wp);
        output.push(flat);
    }
    *gradient_only &= leaf_gradient_only;
    if *num_props > K_NUM_NONREF_PROPERTIES {
        *num_props = div_ceil(
            *num_props - K_NUM_NONREF_PROPERTIES,
            K_EXTRA_PROPS_PER_CHANNEL,
        ) * K_EXTRA_PROPS_PER_CHANNEL
            + K_NUM_NONREF_PROPERTIES;
    } else {
        *num_props = K_NUM_NONREF_PROPERTIES;
    }
    *use_wp = has_wp;
    *wp_only = has_wp && !has_non_wp;

    output
}

/// Translation of `TreeToLookupTable()`.
pub(crate) fn tree_to_lookup_table(
    tree: &FlatTree,
    context_lookup: &mut [u8; 2 * K_PROP_RANGE_FAST as usize],
    offsets: &mut [i8; 2 * K_PROP_RANGE_FAST as usize],
    mut multipliers: Option<&mut [i8; 2 * K_PROP_RANGE_FAST as usize]>,
) -> bool {
    struct TreeRange {
        // Begin *excluded*, end *included*. This works best with > vs <= decision
        // nodes.
        begin: i32,
        end: i32,
        pos: usize,
    }
    let mut ranges: Vec<TreeRange> = vec![TreeRange {
        begin: -K_PROP_RANGE_FAST - 1,
        end: K_PROP_RANGE_FAST - 1,
        pos: 0,
    }];
    while let Some(cur) = ranges.pop() {
        if cur.begin < -K_PROP_RANGE_FAST - 1
            || cur.begin >= K_PROP_RANGE_FAST - 1
            || cur.end > K_PROP_RANGE_FAST - 1
        {
            // Tree is outside the allowed range, exit.
            return false;
        }
        let node = &tree[cur.pos];
        // Leaf.
        if node.property0 == -1 {
            if node.predictor_offset < i8::MIN as i64 || node.predictor_offset > i8::MAX as i64 {
                return false;
            }
            if node.multiplier < i8::MIN as i32 || node.multiplier > i8::MAX as i32 {
                return false;
            }
            if multipliers.is_none() && node.multiplier != 1 {
                return false;
            }
            for i in cur.begin + 1..cur.end + 1 {
                let k = (i + K_PROP_RANGE_FAST) as usize;
                context_lookup[k] = node.child_id as u8;
                if let Some(m) = multipliers.as_mut() {
                    m[k] = node.multiplier as i8;
                }
                offsets[k] = node.predictor_offset as i8;
            }
            continue;
        }
        let child = node.child_id as usize;
        // > side of top node.
        if node.properties[0] >= K_NUM_STATIC_PROPERTIES as i32 {
            ranges.push(TreeRange {
                begin: node.splitvals[0],
                end: cur.end,
                pos: child,
            });
            ranges.push(TreeRange {
                begin: node.splitval0,
                end: node.splitvals[0],
                pos: child + 1,
            });
        } else {
            ranges.push(TreeRange {
                begin: node.splitval0,
                end: cur.end,
                pos: child,
            });
        }
        // <= side
        if node.properties[1] >= K_NUM_STATIC_PROPERTIES as i32 {
            ranges.push(TreeRange {
                begin: node.splitvals[1],
                end: node.splitval0,
                pos: child + 2,
            });
            ranges.push(TreeRange {
                begin: cur.begin,
                end: node.splitvals[1],
                pos: child + 3,
            });
        } else {
            ranges.push(TreeRange {
                begin: cur.begin,
                end: node.splitval0,
                pos: child + 2,
            });
        }
    }
    true
}

/// Translation of the `make_pixel` lambda.
#[inline]
fn make_pixel(v: u64, multiplier: PixelType, offset: PixelTypeW) -> PixelType {
    let val: PixelTypeW = unpack_signed(v as usize) as i64;
    // if it overflows, it overflows, and we have a problem anyway
    val.wrapping_mul(multiplier as i64).wrapping_add(offset) as PixelType
}

/// Translation of `DecodeModularChannelMAANS()`.
#[allow(clippy::too_many_arguments)]
fn decode_modular_channel_maans(
    br: &mut BitReader<'_>,
    reader: &mut AnsSymbolReader<'_>,
    context_map: &[u8],
    global_tree: &Tree,
    wp_header: &weighted::Header,
    chan: PixelType,
    group_id: usize,
    image: &mut Image,
    fl_run: &mut u32,
    fl_v: &mut u32,
) -> Status {
    let static_props: [PixelType; K_NUM_STATIC_PROPERTIES] = [chan, group_id as i32];
    // TODO(veluca): filter the tree according to static_props.

    let (cw, chh) = (
        image.channel[chan as usize].w,
        image.channel[chan as usize].h,
    );
    // zero pixel channel? could happen
    if cw == 0 || chh == 0 {
        return Ok(());
    }

    let mut tree_has_wp_prop_or_pred = false;
    let mut is_wp_only = false;
    let mut is_gradient_only = false;
    let mut num_props = 0usize;
    let mut tree = filter_tree(
        global_tree,
        &static_props,
        &mut num_props,
        &mut tree_has_wp_prop_or_pred,
        &mut is_wp_only,
        &mut is_gradient_only,
    );

    // From here on, tree lookup returns a *clustered* context ID.
    // This avoids an extra memory lookup after tree traversal.
    for node in tree.iter_mut() {
        if node.property0 == -1 {
            node.child_id = context_map[node.child_id as usize] as u32;
        }
    }

    // MAANS decode

    if tree.len() == 1 {
        // special optimized case: no meta-adaptation, so no need
        // to compute properties.
        let predictor = tree[0].predictor;
        let offset: i64 = tree[0].predictor_offset;
        let multiplier: i32 = tree[0].multiplier;
        let ctx_id = tree[0].child_id as usize;
        let channel = &mut image.channel[chan as usize];
        let onerow = channel.plane.pixels_per_row();
        if predictor == Predictor::Zero {
            let mut value = 0u32;
            if reader.is_single_value_and_advance(ctx_id, &mut value, channel.w * channel.h) {
                // Special-case: histogram has a single symbol, with no extra bits, and
                // we use ANS mode.
                let v = make_pixel(value as u64, multiplier, offset);
                for y in 0..channel.h {
                    let w = channel.w;
                    channel.row_mut(y)[..w].fill(v);
                }
            } else if multiplier == 1 && offset == 0 {
                for y in 0..channel.h {
                    let w = channel.w;
                    let r = channel.row_mut(y);
                    for x in 0..w {
                        let v = reader.read_hybrid_uint_clustered(ctx_id, br) as u32;
                        r[x] = unpack_signed(v as usize) as PixelType;
                    }
                }
            } else {
                for y in 0..channel.h {
                    let w = channel.w;
                    let r = channel.row_mut(y);
                    for x in 0..w {
                        let v = reader.read_hybrid_uint_clustered(ctx_id, br) as u32;
                        r[x] = make_pixel(v as u64, multiplier, offset);
                    }
                }
            }
        } else if predictor == Predictor::Gradient
            && offset == 0
            && multiplier == 1
            && reader.huff_rle_only()
        {
            // Gradient RLE (fjxl) very fast track.
            let mut sv: PixelTypeW = unpack_signed(*fl_v as usize) as i64;
            let w = channel.w;
            let data = channel.plane.data_mut();
            for y in 0..chh {
                let r = y * onerow;
                // (rtop and rtopleft are the row above, and the pixel left of
                // the current one on the first row)
                let guess: PixelTypeW = if y != 0 { data[r - onerow] as i64 } else { 0 };
                if *fl_run == 0 {
                    reader.read_hybrid_uint_clustered_huff_rle_only(ctx_id, br, fl_v, fl_run);
                    sv = unpack_signed(*fl_v as usize) as i64;
                } else {
                    *fl_run -= 1;
                }
                data[r] = sv.wrapping_add(guess) as PixelType;
                for x in 1..w {
                    let left = data[r + x - 1];
                    let top = if y != 0 {
                        data[r - onerow + x]
                    } else {
                        data[r + x - 1]
                    };
                    let topleft = if y != 0 {
                        data[r - onerow + x - 1]
                    } else {
                        data[r + x - 1]
                    };
                    let guess: PixelTypeW = clamped_gradient(top, left, topleft) as i64;
                    if *fl_run == 0 {
                        reader.read_hybrid_uint_clustered_huff_rle_only(ctx_id, br, fl_v, fl_run);
                        sv = unpack_signed(*fl_v as usize) as i64;
                    } else {
                        *fl_run -= 1;
                    }
                    data[r + x] = sv.wrapping_add(guess) as PixelType;
                }
            }
        } else if predictor == Predictor::Gradient && offset == 0 && multiplier == 1 {
            // Gradient very fast track.
            let w = channel.w;
            let data = channel.plane.data_mut();
            for y in 0..chh {
                let r = y * onerow;
                for x in 0..w {
                    let left: PixelType = if x != 0 {
                        data[r + x - 1]
                    } else if y != 0 {
                        data[r + x - onerow]
                    } else {
                        0
                    };
                    let top: PixelType = if y != 0 { data[r + x - onerow] } else { left };
                    let topleft: PixelType = if x != 0 && y != 0 {
                        data[r + x - 1 - onerow]
                    } else {
                        left
                    };
                    let guess = clamped_gradient(top, left, topleft);
                    let v = reader.read_hybrid_uint_clustered(ctx_id, br) as u64;
                    data[r + x] = make_pixel(v, 1, guess as i64);
                }
            }
        } else if predictor != Predictor::Weighted {
            // special optimized case: no wp
            let w = channel.w;
            let data = channel.plane.data_mut();
            for y in 0..chh {
                for x in 0..w {
                    let pos = y * onerow + x;
                    let pred = predict_no_tree_no_wp(w, data, pos, onerow, x, y, predictor);
                    let g: PixelTypeW = pred.guess.wrapping_add(offset);
                    let v = reader.read_hybrid_uint_clustered(ctx_id, br) as u64;
                    // NOTE: pred.multiplier is unset.
                    data[pos] = make_pixel(v, multiplier, g);
                }
            }
        } else {
            let w = channel.w;
            let mut wp_state = weighted::State::new(wp_header, w, chh);
            let data = channel.plane.data_mut();
            for y in 0..chh {
                for x in 0..w {
                    let pos = y * onerow + x;
                    let g: PixelTypeW =
                        predict_no_tree_wp(w, data, pos, onerow, x, y, predictor, &mut wp_state)
                            .guess
                            .wrapping_add(offset);
                    let v = reader.read_hybrid_uint_clustered(ctx_id, br) as u64;
                    data[pos] = make_pixel(v, multiplier, g);
                    wp_state.update_errors(data[pos] as i64, x, y, w);
                }
            }
        }
        return Ok(());
    }

    // Check if this tree is a WP-only tree with a small enough property value
    // range.
    // Initialized to avoid clang-tidy complaining.
    let mut context_lookup = [0u8; 2 * K_PROP_RANGE_FAST as usize];
    let mut multipliers = [0i8; 2 * K_PROP_RANGE_FAST as usize];
    let mut offsets = [0i8; 2 * K_PROP_RANGE_FAST as usize];
    if is_wp_only {
        is_wp_only = tree_to_lookup_table(
            &tree,
            &mut context_lookup,
            &mut offsets,
            Some(&mut multipliers),
        );
    }
    if is_gradient_only {
        is_gradient_only = tree_to_lookup_table(
            &tree,
            &mut context_lookup,
            &mut offsets,
            Some(&mut multipliers),
        );
    }

    if is_gradient_only {
        // Gradient fast track.
        let channel = &mut image.channel[chan as usize];
        let onerow = channel.plane.pixels_per_row();
        let w = channel.w;
        let data = channel.plane.data_mut();
        for y in 0..chh {
            let r = y * onerow;
            for x in 0..w {
                let left: PixelTypeW = if x != 0 {
                    data[r + x - 1] as i64
                } else if y != 0 {
                    data[r + x - onerow] as i64
                } else {
                    0
                };
                let top: PixelTypeW = if y != 0 {
                    data[r + x - onerow] as i64
                } else {
                    left
                };
                let topleft: PixelTypeW = if x != 0 && y != 0 {
                    data[r + x - 1 - onerow] as i64
                } else {
                    left
                };
                let guess: i32 = clamped_gradient(top as i32, left as i32, topleft as i32);
                let pos = (K_PROP_RANGE_FAST as i64
                    + ((-K_PROP_RANGE_FAST as i64)
                        .max(top.wrapping_add(left).wrapping_sub(topleft)))
                    .min(K_PROP_RANGE_FAST as i64 - 1)) as usize;
                let ctx_id = context_lookup[pos] as usize;
                let v = reader.read_hybrid_uint_clustered(ctx_id, br) as u64;
                data[r + x] = make_pixel(
                    v,
                    multipliers[pos] as i32,
                    (offsets[pos] as i64).wrapping_add(guess as i64),
                );
            }
        }
    } else if is_wp_only {
        // WP fast track.
        let channel = &mut image.channel[chan as usize];
        let onerow = channel.plane.pixels_per_row();
        let w = channel.w;
        let mut wp_state = weighted::State::new(wp_header, w, chh);
        let mut properties: Properties = vec![0; 1];
        let data = channel.plane.data_mut();
        for y in 0..chh {
            let r = y * onerow;
            for x in 0..w {
                let offset = 0usize;
                let left: PixelTypeW = if x != 0 {
                    data[r + x - 1] as i64
                } else if y != 0 {
                    data[r + x - onerow] as i64
                } else {
                    0
                };
                let top: PixelTypeW = if y != 0 {
                    data[r + x - onerow] as i64
                } else {
                    left
                };
                let topleft: PixelTypeW = if x != 0 && y != 0 {
                    data[r + x - 1 - onerow] as i64
                } else {
                    left
                };
                let topright: PixelTypeW = if x + 1 < w && y != 0 {
                    data[r + x + 1 - onerow] as i64
                } else {
                    top
                };
                let toptop: PixelTypeW = if y > 1 {
                    data[r + x - onerow - onerow] as i64
                } else {
                    top
                };
                let guess = wp_state.predict::<true>(
                    x,
                    y,
                    w,
                    top,
                    left,
                    topright,
                    topleft,
                    toptop,
                    &mut properties,
                    offset,
                ) as i32;
                let pos = (K_PROP_RANGE_FAST
                    + (-K_PROP_RANGE_FAST)
                        .max(properties[0])
                        .min(K_PROP_RANGE_FAST - 1)) as usize;
                let ctx_id = context_lookup[pos] as usize;
                let v = reader.read_hybrid_uint_clustered(ctx_id, br) as u64;
                data[r + x] = make_pixel(
                    v,
                    multipliers[pos] as i32,
                    (offsets[pos] as i64).wrapping_add(guess as i64),
                );
                wp_state.update_errors(data[r + x] as i64, x, y, w);
            }
        }
    } else if !tree_has_wp_prop_or_pred {
        // special optimized case: the weighted predictor and its properties are not
        // used, so no need to compute weights and properties.
        let tree_lookup = MaTreeLookup::new(&tree);
        let mut properties: Properties = vec![0; num_props];
        let mut references = Channel::new(properties.len() - K_NUM_NONREF_PROPERTIES, cw, 0, 0)?;
        let onerow = image.channel[chan as usize].plane.pixels_per_row();
        let w = cw;
        for y in 0..chh {
            {
                let ch = &image.channel[chan as usize];
                precompute_references(ch, y, image, chan as u32, &mut references);
            }
            init_props_row(&mut properties, &static_props, y as i32);
            let data = image.channel[chan as usize].plane.data_mut();
            let p = y * onerow;
            if y > 1 && w > 8 && references.w == 0 {
                for x in 0..2 {
                    let res = predict_tree_no_wp(
                        &mut properties,
                        w,
                        data,
                        p + x,
                        onerow,
                        x,
                        y,
                        &tree_lookup,
                        &references,
                    );
                    let v = reader.read_hybrid_uint_clustered(res.context as usize, br) as u64;
                    data[p + x] = make_pixel(v, res.multiplier, res.guess);
                }
                for x in 2..w - 2 {
                    let res = predict_tree_no_wp_nec(
                        &mut properties,
                        w,
                        data,
                        p + x,
                        onerow,
                        x,
                        y,
                        &tree_lookup,
                        &references,
                    );
                    let v = reader.read_hybrid_uint_clustered(res.context as usize, br) as u64;
                    data[p + x] = make_pixel(v, res.multiplier, res.guess);
                }
                for x in w - 2..w {
                    let res = predict_tree_no_wp(
                        &mut properties,
                        w,
                        data,
                        p + x,
                        onerow,
                        x,
                        y,
                        &tree_lookup,
                        &references,
                    );
                    let v = reader.read_hybrid_uint_clustered(res.context as usize, br) as u64;
                    data[p + x] = make_pixel(v, res.multiplier, res.guess);
                }
            } else {
                for x in 0..w {
                    let res = predict_tree_no_wp(
                        &mut properties,
                        w,
                        data,
                        p + x,
                        onerow,
                        x,
                        y,
                        &tree_lookup,
                        &references,
                    );
                    let v = reader.read_hybrid_uint_clustered(res.context as usize, br) as u64;
                    data[p + x] = make_pixel(v, res.multiplier, res.guess);
                }
            }
        }
    } else {
        // Slowest track.
        let tree_lookup = MaTreeLookup::new(&tree);
        let mut properties: Properties = vec![0; num_props];
        let mut references = Channel::new(properties.len() - K_NUM_NONREF_PROPERTIES, cw, 0, 0)?;
        let onerow = image.channel[chan as usize].plane.pixels_per_row();
        let w = cw;
        let mut wp_state = weighted::State::new(wp_header, w, chh);
        for y in 0..chh {
            init_props_row(&mut properties, &static_props, y as i32);
            {
                let ch = &image.channel[chan as usize];
                precompute_references(ch, y, image, chan as u32, &mut references);
            }
            let data = image.channel[chan as usize].plane.data_mut();
            let p = y * onerow;
            for x in 0..w {
                let res = predict_tree_wp(
                    &mut properties,
                    w,
                    data,
                    p + x,
                    onerow,
                    x,
                    y,
                    &tree_lookup,
                    &references,
                    &mut wp_state,
                );
                let v = reader.read_hybrid_uint_clustered(res.context as usize, br) as u64;
                data[p + x] = make_pixel(v, res.multiplier, res.guess);
                wp_state.update_errors(data[p + x] as i64, x, y, w);
            }
        }
    }
    Ok(())
}

/// Translation of `ValidateChannelDimensions()`.
pub(crate) fn validate_channel_dimensions(image: &Image, options: &ModularOptions) -> Status {
    let nb_channels = image.channel.len();
    for is_dc in [true, false] {
        let group_dim = options.group_dim * if is_dc { K_BLOCK_DIM } else { 1 };
        let mut c = image.nb_meta_channels;
        while c < nb_channels {
            let ch = &image.channel[c];
            if ch.w > options.group_dim || ch.h > options.group_dim {
                break;
            }
            c += 1;
        }
        while c < nb_channels {
            let ch = &image.channel[c];
            c += 1;
            if ch.w == 0 || ch.h == 0 {
                continue; // skip empty
            }
            let is_dc_channel = ch.hshift.min(ch.vshift) >= 3;
            if is_dc_channel != is_dc {
                continue;
            }
            let shift = ch.hshift.max(ch.vshift);
            let tile_dim = if (0..64).contains(&shift) {
                group_dim >> shift
            } else {
                // (an out of range shift)
                0
            };
            if tile_dim == 0 {
                return jxl_failure!("Inconsistent transforms");
            }
        }
    }
    Ok(())
}

/// Translation of `ModularDecode()`.
#[allow(clippy::too_many_arguments)]
fn modular_decode(
    br: &mut BitReader<'_>,
    image: &mut Image,
    header: &mut GroupHeader,
    group_id: usize,
    options: &ModularOptions,
    global_tree: Option<&Tree>,
    global_code: Option<&AnsCode>,
    global_ctx_map: Option<&Vec<u8>>,
    allow_truncated_group: bool,
) -> Status {
    if image.channel.is_empty() {
        return Ok(());
    }

    // decode transforms
    let status = bundle_read(br, header);
    if !allow_truncated_group {
        status?;
    }
    if status == Err(StatusCode::GenericError) {
        return status;
    }
    if !br.all_reads_within_bounds() {
        // Don't do/undo transforms if header is incomplete.
        header.transforms.clear();
        image.transform = header.transforms.clone();
        for c in 0..image.channel.len() {
            zero_fill_image(&mut image.channel[c].plane);
        }
        return Err(StatusCode::NotEnoughBytes);
    }

    image.transform = header.transforms.clone();
    for i in 0..image.transform.len() {
        let mut t = image.transform[i].clone();
        let r = t.meta_apply(image);
        image.transform[i] = t;
        r?;
    }
    if image.error {
        return jxl_failure!("Corrupt file. Aborting.");
    }
    validate_channel_dimensions(image, options)?;

    let nb_channels = image.channel.len();

    let mut num_chans = 0usize;
    let mut distance_multiplier = 0usize;
    for i in 0..nb_channels {
        let channel = &image.channel[i];
        if channel.w == 0 || channel.h == 0 {
            continue; // skip empty channels
        }
        if i >= image.nb_meta_channels
            && (channel.w > options.max_chan_size || channel.h > options.max_chan_size)
        {
            break;
        }
        if channel.w > distance_multiplier {
            distance_multiplier = channel.w;
        }
        num_chans += 1;
    }
    if num_chans == 0 {
        return Ok(());
    }

    let mut next_channel = 0usize;
    // (the scope guard: zero-fills the channels not decoded on failure, if
    // truncated groups are allowed)
    let result = (|| -> Status {
        // Read tree.
        let mut tree_storage: Tree = Vec::new();
        let mut context_map_storage: Vec<u8> = Vec::new();
        let mut code_storage = AnsCode::default();
        let tree: &Tree;
        let code: &AnsCode;
        let context_map: &Vec<u8>;
        if !header.use_global_tree {
            let mut max_tree_size: usize = 1024;
            for i in 0..nb_channels {
                let channel = &image.channel[i];
                if channel.w == 0 || channel.h == 0 {
                    continue; // skip empty channels
                }
                if i >= image.nb_meta_channels
                    && (channel.w > options.max_chan_size || channel.h > options.max_chan_size)
                {
                    break;
                }
                let pixels = channel.w.wrapping_mul(channel.h);
                if pixels / channel.w != channel.h {
                    return jxl_failure!("Tree size overflow");
                }
                max_tree_size = max_tree_size.wrapping_add(pixels);
                if max_tree_size < pixels {
                    return jxl_failure!("Tree size overflow");
                }
            }
            max_tree_size = (1usize << 20).min(max_tree_size);
            decode_tree(br, &mut tree_storage, max_tree_size)?;
            decode_histograms(
                br,
                tree_storage.len().div_ceil(2),
                &mut code_storage,
                &mut context_map_storage,
                false,
            )?;
            tree = &tree_storage;
            code = &code_storage;
            context_map = &context_map_storage;
        } else {
            match (global_tree, global_code, global_ctx_map) {
                (Some(t), Some(c), Some(m)) if !t.is_empty() => {
                    tree = t;
                    code = c;
                    context_map = m;
                }
                _ => return jxl_failure!("No global tree available but one was requested"),
            }
        }

        // Read channels
        let mut reader = AnsSymbolReader::new(code, br, distance_multiplier);
        let mut fl_run: u32 = 0;
        let mut fl_v: u32 = 0;
        while next_channel < nb_channels {
            let channel = &image.channel[next_channel];
            if channel.w == 0 || channel.h == 0 {
                next_channel += 1;
                continue; // skip empty channels
            }
            if next_channel >= image.nb_meta_channels
                && (channel.w > options.max_chan_size || channel.h > options.max_chan_size)
            {
                break;
            }
            decode_modular_channel_maans(
                br,
                &mut reader,
                context_map,
                tree,
                &header.wp_header,
                next_channel as PixelType,
                group_id,
                image,
                &mut fl_run,
                &mut fl_v,
            )?;
            // Truncated group.
            if !br.all_reads_within_bounds() {
                if !allow_truncated_group {
                    return jxl_failure!("Truncated input");
                }
                return Err(StatusCode::NotEnoughBytes);
            }
            next_channel += 1;
        }

        // Make sure no zero-filling happens even if next_channel < nb_channels.
        next_channel = nb_channels;

        if !reader.check_ans_final_state() {
            return jxl_failure!("ANS decode final state failed");
        }
        Ok(())
    })();
    if result.is_err() && allow_truncated_group {
        // (the scope guard, not disarmed)
        for c in next_channel..nb_channels {
            zero_fill_image(&mut image.channel[c].plane);
        }
    }
    result
}

/// Translation of `ModularGenericDecompress()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn modular_generic_decompress(
    br: &mut BitReader<'_>,
    image: &mut Image,
    header: Option<&mut GroupHeader>,
    group_id: usize,
    options: &ModularOptions,
    undo_transforms: bool,
    tree: Option<&Tree>,
    code: Option<&AnsCode>,
    ctx_map: Option<&Vec<u8>>,
    allow_truncated_group: bool,
) -> Status {
    let req_sizes: Vec<(usize, usize)> = image.channel.iter().map(|c| (c.w, c.h)).collect();
    let mut local_header = GroupHeader::new();
    let header = match header {
        Some(h) => h,
        None => &mut local_header,
    };
    let dec_status = modular_decode(
        br,
        image,
        header,
        group_id,
        options,
        tree,
        code,
        ctx_map,
        allow_truncated_group,
    );
    if !allow_truncated_group {
        dec_status?;
    }
    if dec_status == Err(StatusCode::GenericError) {
        return dec_status;
    }
    if undo_transforms {
        image.undo_transforms(&header.wp_header);
    }
    if image.error {
        return jxl_failure!("Corrupt file. Aborting.");
    }
    // Check that after applying all transforms we are back to the requested image
    // sizes, otherwise there's a programming error with the transformations.
    if undo_transforms {
        if image.channel.len() != req_sizes.len() {
            // JXL_ASSERT
            return jxl_failure!("channel count changed");
        }
        for (c, &(w, h)) in req_sizes.iter().enumerate() {
            if w != image.channel[c].w || h != image.channel[c].h {
                // JXL_ASSERT
                return jxl_failure!("channel size changed");
            }
        }
    }
    dec_status
}
