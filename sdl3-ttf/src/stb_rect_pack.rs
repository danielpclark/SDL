// Rust translation of stb_rect_pack.h v1.01 (Sean Barrett), as SDL_ttf
// bundles it in src/ and builds it (with STBRP_SORT = SDL_qsort).
// stb_rect_pack.h - public domain (or the MIT license; see LICENSE.txt).
// This is an altered (translated) version of the original software.

//! stb_rect_pack.h - v1.01 - public domain - rectangle packing
//! Sean Barrett 2014
//!
//! Useful for e.g. packing rectangular textures into an atlas.
//! Does not do rotation.
//!
//! Not necessarily the awesomest packing method, but better than
//! the totally naive one in stb_truetype (which is primarily what
//! this is meant to replace).
//!
//! This library currently uses the Skyline Bottom-Left algorithm.
//!
//! Translation notes: the nodes (the user's array and the context's two
//! extra nodes) are one array, and the C pointers into it are indices;
//! a link (`stbrp_node **`) is the context's head or a node's `next`.

#![allow(dead_code)]

use crate::qsort::sdl_qsort;

/// `stbrp_coord`
pub(crate) type StbrpCoord = i32;

/// `STBRP__MAXVAL`: Mostly for internal use, but this is the maximum
/// supported coordinate value.
pub(crate) const STBRP__MAXVAL: i32 = 0x7fffffff;

/// `stbrp_rect`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct StbrpRect {
    // reserved for your use:
    pub id: i32,

    // input:
    pub w: StbrpCoord,
    pub h: StbrpCoord,

    // output:
    pub x: StbrpCoord,
    pub y: StbrpCoord,
    pub was_packed: i32, // non-zero if valid packing
} // 16 bytes, nominally

pub(crate) const STBRP_HEURISTIC_SKYLINE_DEFAULT: i32 = 0;
pub(crate) const STBRP_HEURISTIC_SKYLINE_BL_SORT_HEIGHT: i32 = STBRP_HEURISTIC_SKYLINE_DEFAULT;
pub(crate) const STBRP_HEURISTIC_SKYLINE_BF_SORT_HEIGHT: i32 = 1;

/// `stbrp_node`
#[derive(Debug, Clone, Copy, Default)]
struct StbrpNode {
    x: StbrpCoord,
    y: StbrpCoord,
    next: Option<usize>,
}

/// `stbrp_context`
#[derive(Debug, Clone, Default)]
pub(crate) struct StbrpContext {
    width: i32,
    height: i32,
    align: i32,
    init_mode: i32,
    heuristic: i32,
    num_nodes: i32,
    active_head: Option<usize>,
    free_head: Option<usize>,
    /// the user's nodes, then the two `extra` nodes (we allocate two extra
    /// nodes so optimal user-node-count is 'width' not 'width+2')
    nodes: Vec<StbrpNode>,
}

const STBRP__INIT_SKYLINE: i32 = 1;

/// A `stbrp_node **`: the link to a node
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Link {
    /// `&context->active_head`
    ActiveHead,
    /// `&node->next`
    Next(usize),
}

impl StbrpContext {
    fn link(&self, l: Link) -> Option<usize> {
        match l {
            Link::ActiveHead => self.active_head,
            Link::Next(n) => self.nodes[n].next,
        }
    }

    fn set_link(&mut self, l: Link, v: Option<usize>) {
        match l {
            Link::ActiveHead => self.active_head = v,
            Link::Next(n) => self.nodes[n].next = v,
        }
    }

    fn node(&self, n: usize) -> &StbrpNode {
        &self.nodes[n]
    }

    fn next(&self, n: usize) -> usize {
        self.nodes[n].next.expect("skyline node")
    }
}

/// `stbrp_setup_heuristic`
pub(crate) fn stbrp_setup_heuristic(context: &mut StbrpContext, heuristic: i32) {
    match context.init_mode {
        STBRP__INIT_SKYLINE => {
            debug_assert!(
                heuristic == STBRP_HEURISTIC_SKYLINE_BL_SORT_HEIGHT
                    || heuristic == STBRP_HEURISTIC_SKYLINE_BF_SORT_HEIGHT
            );
            context.heuristic = heuristic;
        }
        _ => debug_assert!(false),
    }
}

/// `stbrp_setup_allow_out_of_mem`
pub(crate) fn stbrp_setup_allow_out_of_mem(context: &mut StbrpContext, allow_out_of_mem: bool) {
    if allow_out_of_mem {
        // if it's ok to run out of memory, then don't bother aligning them;
        // this gives better packing, but may fail due to OOM (even though
        // the rectangles easily fit). @TODO a smarter approach would be to only
        // quantize once we've hit OOM, then we could get rid of this parameter.
        context.align = 1;
    } else {
        // if it's not ok to run out of memory, then quantize the widths
        // so that num_nodes is always enough nodes.
        //
        // I.e. num_nodes * align >= width
        //                  align >= width / num_nodes
        //                  align = ceil(width/num_nodes)

        context.align = (context.width + context.num_nodes - 1) / context.num_nodes;
    }
}

/// `stbrp_init_target` (with `num_nodes` nodes, which this allocates; an
/// error if that fails)
pub(crate) fn stbrp_init_target(
    context: &mut StbrpContext,
    width: i32,
    height: i32,
    num_nodes: i32,
) -> Result<(), std::collections::TryReserveError> {
    let n = num_nodes as usize;
    let mut nodes = Vec::new();
    nodes.try_reserve_exact(n + 2)?;
    nodes.resize(n + 2, StbrpNode::default());
    context.nodes = nodes;

    let mut i = 0;
    while (i as i32) < num_nodes - 1 {
        context.nodes[i].next = Some(i + 1);
        i += 1;
    }
    context.nodes[i].next = None;
    context.init_mode = STBRP__INIT_SKYLINE;
    context.heuristic = STBRP_HEURISTIC_SKYLINE_DEFAULT;
    context.free_head = Some(0);
    context.active_head = Some(n);
    context.width = width;
    context.height = height;
    context.num_nodes = num_nodes;
    stbrp_setup_allow_out_of_mem(context, false);

    // node 0 is the full width, node 1 is the sentinel (lets us not store width explicitly)
    context.nodes[n].x = 0;
    context.nodes[n].y = 0;
    context.nodes[n].next = Some(n + 1);
    context.nodes[n + 1].x = width as StbrpCoord;
    context.nodes[n + 1].y = 1 << 30;
    context.nodes[n + 1].next = None;
    Ok(())
}

/// `stbrp__skyline_find_min_y`: find minimum y position if it starts at x1
fn stbrp_skyline_find_min_y(
    c: &StbrpContext,
    first: usize,
    x0: i32,
    width: i32,
    pwaste: &mut i32,
) -> i32 {
    let mut node = first;
    let x1 = x0 + width;

    debug_assert!(c.node(first).x <= x0);

    debug_assert!(c.node(c.next(node)).x > x0); // we ended up handling this in the caller for efficiency

    debug_assert!(c.node(node).x <= x0);

    let mut min_y = 0;
    let mut waste_area = 0;
    let mut visited_width = 0;
    while c.node(node).x < x1 {
        let nx = c.node(c.next(node)).x;
        if c.node(node).y > min_y {
            // raise min_y higher.
            // we've accounted for all waste up to min_y,
            // but we'll now add more waste for everything we've visted
            waste_area += visited_width * (c.node(node).y - min_y);
            min_y = c.node(node).y;
            // the first time through, visited_width might be reduced
            if c.node(node).x < x0 {
                visited_width += nx - x0;
            } else {
                visited_width += nx - c.node(node).x;
            }
        } else {
            // add waste area
            let mut under_width = nx - c.node(node).x;
            if under_width + visited_width > width {
                under_width = width - visited_width;
            }
            waste_area += under_width * (min_y - c.node(node).y);
            visited_width += under_width;
        }
        node = c.next(node);
    }

    *pwaste = waste_area;
    min_y
}

/// `stbrp__findresult`
#[derive(Debug, Clone, Copy)]
struct StbrpFindResult {
    x: i32,
    y: i32,
    prev_link: Option<Link>,
}

/// `stbrp__skyline_find_best_pos`
fn stbrp_skyline_find_best_pos(c: &StbrpContext, width: i32, height: i32) -> StbrpFindResult {
    let mut best_waste = 1 << 30;
    let mut best_x;
    let mut best_y = 1 << 30;
    let mut best: Option<Link> = None;
    let mut width = width;

    // align to multiple of c->align
    width += c.align - 1;
    width -= width % c.align;
    debug_assert!(width % c.align == 0);

    // if it can't possibly fit, bail immediately
    if width > c.width || height > c.height {
        return StbrpFindResult {
            prev_link: None,
            x: 0,
            y: 0,
        };
    }

    let mut node = c.active_head.expect("skyline head");
    let mut prev = Link::ActiveHead;
    while c.node(node).x + width <= c.width {
        let mut waste = 0;
        let y = stbrp_skyline_find_min_y(c, node, c.node(node).x, width, &mut waste);
        if c.heuristic == STBRP_HEURISTIC_SKYLINE_BL_SORT_HEIGHT {
            // actually just want to test BL
            // bottom left
            if y < best_y {
                best_y = y;
                best = Some(prev);
            }
        } else {
            // best-fit
            if y + height <= c.height {
                // can only use it if it first vertically
                if y < best_y || (y == best_y && waste < best_waste) {
                    best_y = y;
                    best_waste = waste;
                    best = Some(prev);
                }
            }
        }
        prev = Link::Next(node);
        node = c.next(node);
    }

    best_x = match best {
        None => 0,
        Some(b) => c.node(c.link(b).expect("skyline node")).x,
    };

    // if doing best-fit (BF), we also have to try aligning right edge to each node position
    //
    // e.g, if fitting
    //
    //     ____________________
    //    |____________________|
    //
    //            into
    //
    //   |                         |
    //   |             ____________|
    //   |____________|
    //
    // then right-aligned reduces waste, but bottom-left BL is always chooses left-aligned
    //
    // This makes BF take about 2x the time

    if c.heuristic == STBRP_HEURISTIC_SKYLINE_BF_SORT_HEIGHT {
        let mut tail = c.active_head;
        let mut node = c.active_head.expect("skyline head");
        let mut prev = Link::ActiveHead;
        // find first node that's admissible
        while let Some(t) = tail {
            if c.node(t).x >= width {
                break;
            }
            tail = c.node(t).next;
        }
        while let Some(t) = tail {
            let xpos = c.node(t).x - width;
            let mut waste = 0;
            debug_assert!(xpos >= 0);
            // find the left position that matches this
            while c.node(c.next(node)).x <= xpos {
                prev = Link::Next(node);
                node = c.next(node);
            }
            debug_assert!(c.node(c.next(node)).x > xpos && c.node(node).x <= xpos);
            let y = stbrp_skyline_find_min_y(c, node, xpos, width, &mut waste);
            if y + height <= c.height
                && y <= best_y
                && (y < best_y || waste < best_waste || (waste == best_waste && xpos < best_x))
            {
                best_x = xpos;
                debug_assert!(y <= best_y);
                best_y = y;
                best_waste = waste;
                best = Some(prev);
            }
            tail = c.node(t).next;
        }
    }

    StbrpFindResult {
        prev_link: best,
        x: best_x,
        y: best_y,
    }
}

/// `stbrp__skyline_pack_rectangle`
fn stbrp_skyline_pack_rectangle(
    context: &mut StbrpContext,
    width: i32,
    height: i32,
) -> StbrpFindResult {
    // find best position according to heuristic
    let mut res = stbrp_skyline_find_best_pos(context, width, height);

    // bail if:
    //    1. it failed
    //    2. the best node doesn't fit (we don't always check this)
    //    3. we're out of memory
    let Some(prev_link) = res.prev_link else {
        res.prev_link = None;
        return res;
    };
    if res.y + height > context.height || context.free_head.is_none() {
        res.prev_link = None;
        return res;
    }

    // on success, create new node
    let node = context.free_head.unwrap();
    context.nodes[node].x = res.x as StbrpCoord;
    context.nodes[node].y = (res.y + height) as StbrpCoord;

    context.free_head = context.nodes[node].next;

    // insert the new node into the right starting point, and
    // let 'cur' point to the remaining nodes needing to be
    // stiched back in

    let mut cur = context.link(prev_link).expect("skyline node");
    if context.nodes[cur].x < res.x {
        // preserve the existing one, so start testing with the next one
        let next = context.next(cur);
        context.nodes[cur].next = Some(node);
        cur = next;
    } else {
        context.set_link(prev_link, Some(node));
    }

    // from here, traverse cur and free the nodes, until we get to one
    // that shouldn't be freed
    while let Some(next) = context.nodes[cur].next {
        if context.nodes[next].x > res.x + width {
            break;
        }
        // move the current node to the free list
        context.nodes[cur].next = context.free_head;
        context.free_head = Some(cur);
        cur = next;
    }

    // stitch the list back in
    context.nodes[node].next = Some(cur);

    if context.nodes[cur].x < res.x + width {
        context.nodes[cur].x = (res.x + width) as StbrpCoord;
    }

    res
}

/// `rect_height_compare`
fn rect_height_compare(p: &StbrpRect, q: &StbrpRect) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    if p.h > q.h {
        return Less;
    }
    if p.h < q.h {
        return Greater;
    }
    if p.w > q.w {
        return Less;
    }
    if p.w < q.w {
        return Greater;
    }
    if p.was_packed < q.was_packed {
        Less
    } else if p.was_packed > q.was_packed {
        Greater
    } else {
        Equal
    }
}

/// `rect_original_order`
fn rect_original_order(p: &StbrpRect, q: &StbrpRect) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    if p.was_packed < q.was_packed {
        Less
    } else if p.was_packed > q.was_packed {
        Greater
    } else {
        Equal
    }
}

/// `stbrp_pack_rects`: Assign packed locations to rectangles. Returns 1
/// if all of the rectangles were successfully packed and 0 otherwise.
pub(crate) fn stbrp_pack_rects(context: &mut StbrpContext, rects: &mut [StbrpRect]) -> i32 {
    let mut all_rects_packed = 1;

    // we use the 'was_packed' field internally to allow sorting/unsorting
    for (i, r) in rects.iter_mut().enumerate() {
        r.was_packed = i as i32;
    }

    // sort according to heuristic
    sdl_qsort(rects, rect_height_compare);

    for r in rects.iter_mut() {
        if r.w == 0 || r.h == 0 {
            r.x = 0; // empty rect needs no space
            r.y = 0;
        } else {
            let fr = stbrp_skyline_pack_rectangle(context, r.w, r.h);
            if fr.prev_link.is_some() {
                r.x = fr.x as StbrpCoord;
                r.y = fr.y as StbrpCoord;
            } else {
                r.x = STBRP__MAXVAL;
                r.y = STBRP__MAXVAL;
            }
        }
    }

    // unsort
    sdl_qsort(rects, rect_original_order);

    // set was_packed flags and all_rects_packed status
    for r in rects.iter_mut() {
        r.was_packed = !(r.x == STBRP__MAXVAL && r.y == STBRP__MAXVAL) as i32;
        if r.was_packed == 0 {
            all_rects_packed = 0;
        }
    }

    // return the all_rects_packed status
    all_rects_packed
}

/* (not in stb_rect_pack.h) */

/// Whether a `w` x `h` rectangle can be packed into an empty
/// `size` x `size` target with `size / 4` nodes, as the text engines make
/// them: the widths are rounded up to a multiple of the alignment
/// (`stbrp_setup_allow_out_of_mem(context, 0)`).
pub(crate) fn stbrp_fits_empty_target(w: i32, h: i32, size: i32) -> bool {
    let num_nodes = size / 4;
    if num_nodes == 0 {
        return true; // (CreateAtlas() fails)
    }
    let align = (size + num_nodes - 1) / num_nodes;
    let aligned = (w as i64 + align as i64 - 1) / align as i64 * align as i64;
    aligned <= size as i64 && h <= size
}
