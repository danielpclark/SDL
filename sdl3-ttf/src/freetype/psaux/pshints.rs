// Rust translation of src/psaux/pshints.c and src/psaux/pshints.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright 2007-2014 Adobe Systems Incorporated.
//
// This software, and all works of authorship, whether in source or
// object code form as indicated by the copyright notice(s) included
// herein (collectively, the "Work") is made available, and may only be
// used, modified, and distributed under the FreeType Project License,
// LICENSE.TXT.  Additionally, subject to the terms and conditions of the
// FreeType Project License, each contributor to the Work hereby grants
// to any individual or legal entity exercising permissions granted by
// the FreeType Project License and this section (hereafter, "You" or
// "Your") a perpetual, worldwide, non-exclusive, no-charge,
// royalty-free, irrevocable (except as stated in this section) patent
// license to make, have made, use, offer to sell, sell, import, and
// otherwise transfer the Work, where such license applies only to those
// patent claims licensable by such contributor that are necessarily
// infringed by their contribution(s) alone or by combination of their
// contribution(s) with the Work to which such contribution(s) was
// submitted.  If You institute patent litigation against any entity
// (including a cross-claim or counterclaim in a lawsuit) alleging that
// the Work or a contribution incorporated within the Work constitutes
// direct or contributory patent infringement, then any patent licenses
// granted to You under this License for that Work shall terminate as of
// the date such litigation is filed.
//
// By using, modifying, or distributing the Work you indicate that you
// have read and understood the terms and conditions of the
// FreeType Project License as well as those provided in this section,
// and you accept them fully.
//
// This is an altered (translated) version of the original software; the
// FreeType Project License is in FTL.TXT (see also LICENSE.txt).

//! Adobe's code for handling CFF hints (body).
//!
//! A hint map's `font`, `initialHintMap` and `hintMoves` pointers are
//! given to the functions that use them (the initial map is `None` when
//! the initial map itself is built: C points it at itself, and it is not
//! valid until the build ends). A glyph path's font, outline callbacks,
//! stem hint arrays and hint mask are given likewise ([`Cf2Ctx`],
//! [`Cf2Hints`]); its `blues` is the font's.

use super::super::base::ftcalc::*;
use super::super::fttypes::*;
use super::psarrst::*;
use super::psblues::*;
use super::psfixed::*;
use super::psfont::Cf2FontRec;
use super::psft::{cf2_builder_cube_to, cf2_builder_line_to, cf2_builder_move_to};
use super::psglue::*;
use super::psintrp::{
    cf2_hintmask_get_mask_ptr, cf2_hintmask_init, cf2_hintmask_is_new, cf2_hintmask_is_valid,
    cf2_hintmask_set_all, cf2_hintmask_set_new,
};
use super::psobjs::PsDecoder;

pub const CF2_MAX_HINTS: usize = 96; /* maximum # of hints */

/*
 * A HintMask object stores a bit mask that specifies which hints in the
 * charstring are active at a given time.  Hints in CFF must be declared
 * at the start, before any drawing operators, with horizontal hints
 * preceding vertical hints.  The HintMask is ordered the same way, with
 * horizontal hints immediately followed by vertical hints.  Clients are
 * responsible for knowing how many of each type are present.
 *
 * The maximum total number of hints is 96, as specified by the CFF
 * specification.
 *
 * A HintMask is built 0 or more times while interpreting a charstring, by
 * the HintMask operator.  There is only one HintMask, but it is built or
 * rebuilt each time there is a hint substitution (HintMask operator) in
 * the charstring.  A default HintMask with all bits set is built if there
 * has been no HintMask operator prior to the first drawing operator.
 *
 */

/// `CF2_HintMaskRec` (its `error` is the font instance's)
#[derive(Debug, Clone, Copy, Default)]
pub struct Cf2HintMaskRec {
    pub isValid: bool,
    pub isNew: bool,

    pub bitCount: usize,
    pub byteCount: usize,

    pub mask: [FtByte; CF2_MAX_HINTS.div_ceil(8)],
}

/// `CF2_StemHintRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct Cf2StemHintRec {
    pub used: bool, /* DS positions are valid         */

    pub min: Cf2Fixed, /* original character space value */
    pub max: Cf2Fixed,

    pub minDS: Cf2Fixed, /* DS position after first use    */
    pub maxDS: Cf2Fixed,
}

/*
 * A HintMap object stores a piecewise linear function for mapping
 * y-coordinates from character space to device space, providing
 * appropriate pixel alignment to stem edges.
 *
 * The map is implemented as an array of `CF2_Hint' elements, each
 * representing an edge.  When edges are paired, as from stem hints, the
 * bottom edge must immediately precede the top edge in the array.
 * Element character space AND device space positions must both increase
 * monotonically in the array.  `CF2_Hint' elements are also used as
 * parameters to `cf2_blues_capture'.
 *
 * The `cf2_hintmap_build' method must be called before any drawing
 * operation (beginning with a Move operator) and at each hint
 * substitution (HintMask operator).
 *
 * The `cf2_hintmap_map' method is called to transform y-coordinates at
 * each drawing operation (move, line, curve).
 *
 */

/* TODO: make this a CF2_ArrStack and add a deep copy method */
pub const CF2_MAX_HINT_EDGES: usize = CF2_MAX_HINTS * 2;

/// `CF2_HintMapRec`
#[derive(Debug, Clone, Copy)]
pub struct Cf2HintMapRec {
    pub isValid: bool,
    pub hinted: bool,

    pub scale: Cf2Fixed,
    pub count: Cf2UInt,

    /* start search from this index */
    pub lastIndex: Cf2UInt,

    pub edge: [Cf2HintRec; CF2_MAX_HINT_EDGES], /* 192 */
}

impl Default for Cf2HintMapRec {
    fn default() -> Self {
        Cf2HintMapRec {
            isValid: false,
            hinted: false,
            scale: 0,
            count: 0,
            lastIndex: 0,
            edge: [Cf2HintRec::default(); CF2_MAX_HINT_EDGES],
        }
    }
}

/*
 * GlyphPath is a wrapper for drawing operations that scales the
 * coordinates according to the render matrix and HintMap.  It also tracks
 * open paths to control ClosePath and to insert MoveTo for broken fonts.
 *
 */

/// `CF2_GlyphPathRec`
#[derive(Debug, Clone, Default)]
pub struct Cf2GlyphPathRec {
    /* TODO: gather some of these into a hinting context */
    pub hintMap: Cf2HintMapRec,        /* current hint map            */
    pub firstHintMap: Cf2HintMapRec,   /* saved copy                  */
    pub initialHintMap: Cf2HintMapRec, /* based on all captured hints */

    pub hintMoves: Cf2ArrStackRec<Cf2HintMoveRec>, /* list of hint moves for 2nd pass */

    pub scaleX: Cf2Fixed, /* matrix a */
    pub scaleC: Cf2Fixed, /* matrix c */
    pub scaleY: Cf2Fixed, /* matrix d */

    pub fractionalTranslation: FtVector, /* including deviceXScale */

    pub pathIsOpen: bool,    /* true after MoveTo                     */
    pub pathIsClosing: bool, /* true when synthesizing closepath line */
    pub darken: bool,        /* true if stem darkening                */
    pub moveIsPending: bool, /* true between MoveTo and offset MoveTo */

    /* references used to call `cf2_hintmap_build', if necessary */
    pub hintOriginY: Cf2Fixed, /* copy of current origin  */

    pub xOffset: Cf2Fixed, /* character space offsets */
    pub yOffset: Cf2Fixed,

    /* character space miter limit threshold */
    pub miterLimit: Cf2Fixed,
    /* vertical/horizontal snap distance in character space */
    pub snapThreshold: Cf2Fixed,

    pub offsetStart0: FtVector, /* first and second points of first */
    pub offsetStart1: FtVector, /* element with offset applied      */

    /* current point, character space, before offset */
    pub currentCS: FtVector,
    /* current point, device space */
    pub currentDS: FtVector,
    /* start point of subpath, character space */
    pub start: FtVector,

    /* the following members constitute the `queue' of one element */
    pub elemIsQueued: bool,
    pub prevElemOp: Cf2Int,

    pub prevElemP0: FtVector,
    pub prevElemP1: FtVector,
    pub prevElemP2: FtVector,
    pub prevElemP3: FtVector,
}

/// The font instance and the decoder the glyph path draws with (C's
/// `glyphpath->font` and the outline callbacks' `decoder`).
#[derive(Debug)]
pub struct Cf2Ctx<'x, 'a, 'b> {
    pub font: &'x mut Cf2FontRec,
    pub decoder: &'x mut PsDecoder<'a, 'b>,
}

/// The interpreter's hint objects the glyph path refers to (C's
/// `hStemHintArray`, `vStemHintArray` and `hintMask` pointers).
#[derive(Debug, Clone, Default)]
pub struct Cf2Hints {
    pub hStemHintArray: Cf2ArrStackRec<Cf2StemHintRec>,
    pub vStemHintArray: Cf2ArrStackRec<Cf2StemHintRec>,
    pub hintMask: Cf2HintMaskRec,
}

/// `CF2_HintMoveRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct Cf2HintMoveRec {
    pub j: usize,         /* index of upper hint map edge   */
    pub moveUp: Cf2Fixed, /* adjustment to optimum position */
}

/* Compute angular momentum for winding order detection.  It is called */
/* for all lines and curves, but not necessarily in element order.     */
fn cf2_get_winding_momentum(x1: Cf2Fixed, y1: Cf2Fixed, x2: Cf2Fixed, y2: Cf2Fixed) -> Cf2Int {
    /* cross product of pt1 position from origin with pt2 position from  */
    /* pt1; we reduce the precision so that the result fits into 32 bits */
    (x1 >> 16)
        .wrapping_mul(sub_int32(y2, y1) >> 16)
        .wrapping_sub((y1 >> 16).wrapping_mul(sub_int32(x2, x1) >> 16))
}

/*
 * Construct from a StemHint; this is used as a parameter to
 * `cf2_blues_capture'.
 * `hintOrigin' is the character space displacement of a seac accent.
 * Adjust stem hint for darkening here.
 *
 */
#[allow(clippy::too_many_arguments)]
fn cf2_hint_init(
    hint: &mut Cf2HintRec,
    stem_hint_array: &Cf2ArrStackRec<Cf2StemHintRec>,
    index_stem_hint: usize,
    font: &mut Cf2FontRec,
    hint_origin: Cf2Fixed,
    scale: Cf2Fixed,
    bottom: bool,
) {
    *hint = Cf2HintRec::default();

    let i = cf2_arrstack_get_pointer(stem_hint_array, index_stem_hint, &mut font.error);
    let stem_hint: Cf2StemHintRec = stem_hint_array.items.get(i).copied().unwrap_or_default();

    let width: Cf2Fixed = sub_int32(stem_hint.max, stem_hint.min);

    if width == cf2_int_to_fixed(-21) {
        /* ghost bottom */

        if bottom {
            hint.csCoord = stem_hint.max;
            hint.flags = CF2_GHOST_BOTTOM;
        } else {
            hint.flags = 0;
        }
    } else if width == cf2_int_to_fixed(-20) {
        /* ghost top */

        if bottom {
            hint.flags = 0;
        } else {
            hint.csCoord = stem_hint.min;
            hint.flags = CF2_GHOST_TOP;
        }
    } else if width < 0 {
        /* inverted pair */

        /*
         * Hints with negative widths were produced by an early version of a
         * non-Adobe font tool.  The Type 2 spec allows edge (ghost) hints
         * with negative widths, but says
         *
         *   All other negative widths have undefined meaning.
         *
         * CoolType has a silent workaround that negates the hint width; for
         * permissive mode, we do the same here.
         *
         * Note: Such fonts cannot use ghost hints, but should otherwise work.
         * Note: Some poor hints in our faux fonts can produce negative
         *       widths at some blends.  For example, see a light weight of
         *       `u' in ASerifMM.
         *
         */
        if bottom {
            hint.csCoord = stem_hint.max;
            hint.flags = CF2_PAIR_BOTTOM;
        } else {
            hint.csCoord = stem_hint.min;
            hint.flags = CF2_PAIR_TOP;
        }
    } else {
        /* normal pair */

        if bottom {
            hint.csCoord = stem_hint.min;
            hint.flags = CF2_PAIR_BOTTOM;
        } else {
            hint.csCoord = stem_hint.max;
            hint.flags = CF2_PAIR_TOP;
        }
    }

    /* Now that ghost hints have been detected, adjust this edge for      */
    /* darkening.  Bottoms are not changed; tops are incremented by twice */
    /* `darkenY'.                                                         */
    if cf2_hint_is_top(hint) {
        hint.csCoord = add_int32(hint.csCoord, font.darkenY.wrapping_mul(2));
    }

    hint.csCoord = add_int32(hint.csCoord, hint_origin);
    hint.scale = scale;
    hint.index = index_stem_hint; /* index in original stem hint array */

    /* if original stem hint has been used, use the same position */
    if hint.flags != 0 && stem_hint.used {
        if cf2_hint_is_top(hint) {
            hint.dsCoord = stem_hint.maxDS;
        } else {
            hint.dsCoord = stem_hint.minDS;
        }

        cf2_hint_lock(hint);
    } else {
        hint.dsCoord = ft_mul_fix(hint.csCoord as FtLong, scale as FtLong) as Cf2Fixed;
    }
}

/* initialize an invalid hint map element */
fn cf2_hint_init_zero(hint: &mut Cf2HintRec) {
    *hint = Cf2HintRec::default();
}

/// `cf2_hint_isValid`
pub fn cf2_hint_is_valid(hint: &Cf2HintRec) -> bool {
    hint.flags != 0
}

fn cf2_hint_is_pair(hint: &Cf2HintRec) -> bool {
    hint.flags & (CF2_PAIR_BOTTOM | CF2_PAIR_TOP) != 0
}

fn cf2_hint_is_pair_top(hint: &Cf2HintRec) -> bool {
    hint.flags & CF2_PAIR_TOP != 0
}

/// `cf2_hint_isTop`
pub fn cf2_hint_is_top(hint: &Cf2HintRec) -> bool {
    hint.flags & (CF2_PAIR_TOP | CF2_GHOST_TOP) != 0
}

/// `cf2_hint_isBottom`
pub fn cf2_hint_is_bottom(hint: &Cf2HintRec) -> bool {
    hint.flags & (CF2_PAIR_BOTTOM | CF2_GHOST_BOTTOM) != 0
}

fn cf2_hint_is_locked(hint: &Cf2HintRec) -> bool {
    hint.flags & CF2_LOCKED != 0
}

fn cf2_hint_is_synthetic(hint: &Cf2HintRec) -> bool {
    hint.flags & CF2_SYNTHETIC != 0
}

/// `cf2_hint_lock`
pub fn cf2_hint_lock(hint: &mut Cf2HintRec) {
    hint.flags |= CF2_LOCKED;
}

/// `cf2_hintmap_init` (the font, initial map and hint moves are given to
/// the functions using them)
pub fn cf2_hintmap_init(hintmap: &mut Cf2HintMapRec, font: &Cf2FontRec, scale: Cf2Fixed) {
    *hintmap = Cf2HintMapRec::default();

    /* copy parameters from font instance */
    hintmap.hinted = font.hinted;
    hintmap.scale = scale;
}

fn cf2_hintmap_is_valid(hintmap: Option<&Cf2HintMapRec>) -> bool {
    hintmap.is_some_and(|h| h.isValid)
}

fn cf2_hintmap_dump(_hintmap: &Cf2HintMapRec) {
    /* (FT_DEBUG_LEVEL_TRACE is undefined) */
}

/* transform character space coordinate to device space using hint map */
fn cf2_hintmap_map(hintmap: &mut Cf2HintMapRec, cs_coord: Cf2Fixed) -> Cf2Fixed {
    if hintmap.count == 0 || !hintmap.hinted {
        /* there are no hints; use uniform scale and zero offset */
        ft_mul_fix(cs_coord as FtLong, hintmap.scale as FtLong) as Cf2Fixed
    } else {
        /* start linear search from last hit */
        let mut i: usize = hintmap.lastIndex as usize;

        /* search up */
        while i < hintmap.count as usize - 1 && cs_coord >= hintmap.edge[i + 1].csCoord {
            i += 1;
        }

        /* search down */
        while i > 0 && cs_coord < hintmap.edge[i].csCoord {
            i -= 1;
        }

        hintmap.lastIndex = i as Cf2UInt;

        if i == 0 && cs_coord < hintmap.edge[0].csCoord {
            /* special case for points below first edge: use uniform scale */
            add_int32(
                ft_mul_fix(
                    sub_int32(cs_coord, hintmap.edge[0].csCoord) as FtLong,
                    hintmap.scale as FtLong,
                ) as Cf2Fixed,
                hintmap.edge[0].dsCoord,
            )
        } else {
            /*
             * Note: entries with duplicate csCoord are allowed.
             * Use edge[i], the highest entry where csCoord >= entry[i].csCoord
             */
            add_int32(
                ft_mul_fix(
                    sub_int32(cs_coord, hintmap.edge[i].csCoord) as FtLong,
                    hintmap.edge[i].scale as FtLong,
                ) as Cf2Fixed,
                hintmap.edge[i].dsCoord,
            )
        }
    }
}

/*
 * This hinting policy moves a hint pair in device space so that one of
 * its two edges is on a device pixel boundary (its fractional part is
 * zero).  `cf2_hintmap_insertHint' guarantees no overlap in CS
 * space.  Ensure here that there is no overlap in DS.
 *
 * In the first pass, edges are adjusted relative to adjacent hints.
 * Those that are below have already been adjusted.  Those that are
 * above have not yet been adjusted.  If a hint above blocks an
 * adjustment to an optimal position, we will try again in a second
 * pass.  The second pass is top-down.
 *
 */

fn cf2_hintmap_adjust_hints(
    hintmap: &mut Cf2HintMapRec,
    hint_moves: &mut Cf2ArrStackRec<Cf2HintMoveRec>,
    error: &mut FtError,
) {
    cf2_arrstack_clear(hint_moves); /* working storage */

    /*
     * First pass is bottom-up (font hint order) without look-ahead.
     * Locked edges are already adjusted.
     * Unlocked edges begin with dsCoord from `initialHintMap'.
     * Save edges that are not optimally adjusted in `hintMoves' array,
     * and process them in second pass.
     */

    let count = hintmap.count as usize;
    let mut i: usize = 0;
    while i < count {
        let is_pair = cf2_hint_is_pair(&hintmap.edge[i]);

        /* final amount to move edge or edge pair */
        let mut move_: Cf2Fixed = 0;

        /* index of upper edge (same value for ghost hint) */
        let j: usize = if is_pair { i + 1 } else { i };

        let ds_coord_i: Cf2Fixed = hintmap.edge[i].dsCoord;
        let ds_coord_j: Cf2Fixed = hintmap.edge[j].dsCoord;

        if !cf2_hint_is_locked(&hintmap.edge[i]) {
            /* hint edge is not locked, we can adjust it */
            let frac_down: Cf2Fixed = cf2_fixed_fraction(ds_coord_i);
            let frac_up: Cf2Fixed = cf2_fixed_fraction(ds_coord_j);

            /* calculate all four possibilities; moves down are negative */
            let down_move_down: Cf2Fixed = 0i32.wrapping_sub(frac_down);
            let up_move_down: Cf2Fixed = 0i32.wrapping_sub(frac_up);
            let down_move_up: Cf2Fixed = if frac_down == 0 {
                0
            } else {
                cf2_int_to_fixed(1).wrapping_sub(frac_down)
            };
            let up_move_up: Cf2Fixed = if frac_up == 0 {
                0
            } else {
                cf2_int_to_fixed(1).wrapping_sub(frac_up)
            };

            /* smallest move up */
            let move_up: Cf2Fixed = down_move_up.min(up_move_up);
            /* smallest move down */
            let move_down: Cf2Fixed = down_move_down.max(up_move_down);

            let down_min_counter: Cf2Fixed = CF2_MIN_COUNTER;
            let up_min_counter: Cf2Fixed = CF2_MIN_COUNTER;
            let mut save_edge = false;

            /* minimum counter constraint doesn't apply when adjacent edges */
            /* are synthetic                                                */
            /* TODO: doesn't seem a big effect; for now, reduce the code    */

            /* is there room to move up?                                    */
            /* there is if we are at top of array or the next edge is at or */
            /* beyond proposed move up?                                     */
            if j >= count - 1
                || hintmap.edge[j + 1].dsCoord
                    >= add_int32(ds_coord_j, move_up.wrapping_add(up_min_counter))
            {
                /* there is room to move up; is there also room to move down? */
                if i == 0
                    || hintmap.edge[i - 1].dsCoord
                        <= add_int32(ds_coord_i, move_down.wrapping_sub(down_min_counter))
                {
                    /* move smaller absolute amount */
                    move_ = if move_down.wrapping_neg() < move_up {
                        move_down
                    } else {
                        move_up
                    }; /* optimum */
                } else {
                    move_ = move_up;
                }
            } else {
                /* is there room to move down? */
                if i == 0
                    || hintmap.edge[i - 1].dsCoord
                        <= add_int32(ds_coord_i, move_down.wrapping_sub(down_min_counter))
                {
                    move_ = move_down;
                    /* true if non-optimum move */
                    save_edge = move_up < move_down.wrapping_neg();
                } else {
                    /* no room to move either way without overlapping or reducing */
                    /* the counter too much                                       */
                    move_ = 0;
                    save_edge = true;
                }
            }

            /* Identify non-moves and moves down that aren't optimal, and save */
            /* them for second pass.                                           */
            /* Do this only if there is an unlocked edge above (which could    */
            /* possibly move).                                                 */
            if save_edge && j < count - 1 && !cf2_hint_is_locked(&hintmap.edge[j + 1]) {
                let saved_move = Cf2HintMoveRec {
                    j,
                    /* desired adjustment in second pass */
                    moveUp: move_up.wrapping_sub(move_),
                };

                cf2_arrstack_push(hint_moves, &saved_move, error);
            }

            /* move the edge(s) */
            hintmap.edge[i].dsCoord = add_int32(ds_coord_i, move_);
            if is_pair {
                hintmap.edge[j].dsCoord = add_int32(ds_coord_j, move_);
            }
        }

        /* assert there are no overlaps in device space;     */
        /* ignore tests if there was overflow (that is, if   */
        /* operands have the same sign but the sum does not) */

        /* adjust the scales, avoiding divide by zero */
        if i > 0 {
            if hintmap.edge[i].csCoord != hintmap.edge[i - 1].csCoord {
                hintmap.edge[i - 1].scale = ft_div_fix(
                    sub_int32(hintmap.edge[i].dsCoord, hintmap.edge[i - 1].dsCoord) as FtLong,
                    sub_int32(hintmap.edge[i].csCoord, hintmap.edge[i - 1].csCoord) as FtLong,
                ) as Cf2Fixed;
            }
        }

        if is_pair {
            if hintmap.edge[j].csCoord != hintmap.edge[j - 1].csCoord {
                hintmap.edge[j - 1].scale = ft_div_fix(
                    sub_int32(hintmap.edge[j].dsCoord, hintmap.edge[j - 1].dsCoord) as FtLong,
                    sub_int32(hintmap.edge[j].csCoord, hintmap.edge[j - 1].csCoord) as FtLong,
                ) as Cf2Fixed;
            }

            i += 1; /* skip upper edge on next loop */
        }

        i += 1;
    }

    /* second pass tries to move non-optimal hints up, in case there is */
    /* room now                                                         */
    let mut i = cf2_arrstack_size(hint_moves);
    while i > 0 {
        let k = cf2_arrstack_get_pointer(hint_moves, i - 1, error);
        let hint_move = hint_moves.items.get(k).copied().unwrap_or_default();

        let j = hint_move.j;

        /* this was tested before the push, above */

        /* is there room to move up? */
        if hintmap.edge[j + 1].dsCoord
            >= add_int32(
                hintmap.edge[j].dsCoord,
                hint_move.moveUp.wrapping_add(CF2_MIN_COUNTER),
            )
        {
            /* there is more room now, move edge up */
            hintmap.edge[j].dsCoord = add_int32(hintmap.edge[j].dsCoord, hint_move.moveUp);

            if cf2_hint_is_pair(&hintmap.edge[j]) {
                hintmap.edge[j - 1].dsCoord =
                    add_int32(hintmap.edge[j - 1].dsCoord, hint_move.moveUp);
            }
        }
        i -= 1;
    }
}

/* insert hint edges into map, sorted by csCoord */
fn cf2_hintmap_insert_hint(
    hintmap: &mut Cf2HintMapRec,
    initial_hint_map: Option<&mut Cf2HintMapRec>,
    bottom_hint_edge: &mut Cf2HintRec,
    top_hint_edge: &mut Cf2HintRec,
) {
    let mut index_insert: Cf2UInt;

    /* set default values, then check for edge hints */
    let mut is_pair = true;
    /* `firstHintEdge' and `secondHintEdge' point to the bottom and top */
    /* edges, or both to the top edge (`first_is_top')                  */
    let mut first_is_top = false;

    /* one or none of the input params may be invalid when dealing with */
    /* edge hints; at least one edge must be valid                      */

    /* determine how many and which edges to insert */
    if !cf2_hint_is_valid(bottom_hint_edge) {
        /* insert only the top edge */
        first_is_top = true;
        is_pair = false;
    } else if !cf2_hint_is_valid(top_hint_edge) {
        /* insert only the bottom edge */
        is_pair = false;
    }

    /* paired edges must be in proper order */
    if is_pair && top_hint_edge.csCoord < bottom_hint_edge.csCoord {
        return;
    }

    let first_cs = if first_is_top {
        top_hint_edge.csCoord
    } else {
        bottom_hint_edge.csCoord
    };

    /* linear search to find index value of insertion point */
    index_insert = 0;
    while index_insert < hintmap.count {
        if hintmap.edge[index_insert as usize].csCoord >= first_cs {
            break;
        }
        index_insert += 1;
    }

    /*
     * Discard any hints that overlap in character space.  Most often, this
     * is while building the initial map, where captured hints from all
     * zones are combined.  Define overlap to include hints that `touch'
     * (overlap zero).  Hiragino Sans/Gothic fonts have numerous hints that
     * touch.  Some fonts have non-ideographic glyphs that overlap our
     * synthetic hints.
     *
     * Overlap also occurs when darkening stem hints that are close.
     *
     */
    if index_insert < hintmap.count {
        let ins = &hintmap.edge[index_insert as usize];

        /* we are inserting before an existing edge:    */
        /* verify that an existing edge is not the same */
        if ins.csCoord == first_cs {
            return; /* ignore overlapping stem hint */
        }

        /* verify that a new pair does not straddle the next edge */
        if is_pair && ins.csCoord <= top_hint_edge.csCoord {
            return; /* ignore overlapping stem hint */
        }

        /* verify that we are not inserting between paired edges */
        if cf2_hint_is_pair_top(ins) {
            return; /* ignore overlapping stem hint */
        }
    }

    /* recompute device space locations using initial hint map */
    let first_locked = if first_is_top {
        cf2_hint_is_locked(top_hint_edge)
    } else {
        cf2_hint_is_locked(bottom_hint_edge)
    };
    if let Some(initial) = initial_hint_map {
        if initial.isValid && !first_locked {
            if is_pair {
                /* Use hint map to position the center of stem, and nominal scale */
                /* to position the two edges.  This preserves the stem width.     */
                let first = bottom_hint_edge.csCoord;
                let second = top_hint_edge.csCoord;
                let midpoint: Cf2Fixed =
                    cf2_hintmap_map(initial, add_int32(first, sub_int32(second, first) / 2));
                let half_width: Cf2Fixed = ft_mul_fix(
                    (sub_int32(second, first) / 2) as FtLong,
                    hintmap.scale as FtLong,
                ) as Cf2Fixed;

                bottom_hint_edge.dsCoord = sub_int32(midpoint, half_width);
                top_hint_edge.dsCoord = add_int32(midpoint, half_width);
            } else if first_is_top {
                top_hint_edge.dsCoord = cf2_hintmap_map(initial, top_hint_edge.csCoord);
            } else {
                bottom_hint_edge.dsCoord = cf2_hintmap_map(initial, bottom_hint_edge.csCoord);
            }
        }
    }

    let first_hint_edge: Cf2HintRec = if first_is_top {
        *top_hint_edge
    } else {
        *bottom_hint_edge
    };
    let second_hint_edge: Cf2HintRec = *top_hint_edge;

    /*
     * Discard any hints that overlap in device space; this can occur
     * because locked hints have been moved to align with blue zones.
     *
     * TODO: Although we might correct this later during adjustment, we
     * don't currently have a way to delete a conflicting hint once it has
     * been inserted.  See v2.030 MinionPro-Regular, 12 ppem darkened,
     * initial hint map for second path, glyph 945 (the perispomeni (tilde)
     * in U+1F6E, Greek omega with psili and perispomeni).  Darkening is
     * 25.  Pair 667,747 initially conflicts in design space with top edge
     * 660.  This is because 667 maps to 7.87, and the top edge was
     * captured by a zone at 8.0.  The pair is later successfully inserted
     * in a zone without the top edge.  In this zone it is adjusted to 8.0,
     * and no longer conflicts with the top edge in design space.  This
     * means it can be included in yet a later zone which does have the top
     * edge hint.  This produces a small mismatch between the first and
     * last points of this path, even though the hint masks are the same.
     * The density map difference is tiny (1/256).
     *
     */

    if index_insert > 0 {
        /* we are inserting after an existing edge */
        if first_hint_edge.dsCoord < hintmap.edge[index_insert as usize - 1].dsCoord {
            return;
        }
    }

    if index_insert < hintmap.count {
        /* we are inserting before an existing edge */
        if is_pair {
            if second_hint_edge.dsCoord > hintmap.edge[index_insert as usize].dsCoord {
                return;
            }
        } else if first_hint_edge.dsCoord > hintmap.edge[index_insert as usize].dsCoord {
            return;
        }
    }

    /* make room to insert */
    {
        let mut i_src: Cf2UInt = hintmap.count.wrapping_sub(1);
        let mut i_dst: Cf2UInt = if is_pair {
            hintmap.count + 1
        } else {
            hintmap.count
        };

        let mut count: Cf2UInt = hintmap.count - index_insert;

        if i_dst as usize >= CF2_MAX_HINT_EDGES {
            return;
        }

        while count > 0 {
            count -= 1;
            hintmap.edge[i_dst as usize] = hintmap.edge[i_src as usize];
            i_dst = i_dst.wrapping_sub(1);
            i_src = i_src.wrapping_sub(1);
        }

        /* insert first edge */
        hintmap.edge[index_insert as usize] = first_hint_edge; /* copy struct */
        hintmap.count += 1;

        if is_pair {
            /* insert second edge */
            hintmap.edge[index_insert as usize + 1] = second_hint_edge; /* copy struct */
            hintmap.count += 1;
        }
    }
}

/*
 * Build a map from hints and mask.
 *
 * This function may recur one level if `hintmap->initialHintMap' is not yet
 * valid.
 * If `initialMap' is true, simply build initial map.
 *
 * Synthetic hints are used in two ways.  A hint at zero is inserted, if
 * needed, in the initial hint map, to prevent translations from
 * propagating across the origin.  If synthetic em box hints are enabled
 * for ideographic dictionaries, then they are inserted in all hint
 * maps, including the initial one.
 *
 */

/// `cf2_hintmap_build` (`initial_hint_map` is `hintmap->initialHintMap`,
/// `None` when the initial map itself is built)
#[allow(clippy::too_many_arguments)]
pub fn cf2_hintmap_build(
    hintmap: &mut Cf2HintMapRec,
    mut initial_hint_map: Option<&mut Cf2HintMapRec>,
    hint_moves: &mut Cf2ArrStackRec<Cf2HintMoveRec>,
    font: &mut Cf2FontRec,
    h_stem_hint_array: &mut Cf2ArrStackRec<Cf2StemHintRec>,
    v_stem_hint_array: &Cf2ArrStackRec<Cf2StemHintRec>,
    hint_mask: &mut Cf2HintMaskRec,
    hint_origin: Cf2Fixed,
    initial_map: bool,
) {
    let mut temp_hint_mask = Cf2HintMaskRec::default();
    let mut mask_byte: FtByte;

    /* check whether initial map is constructed */
    if !initial_map && !cf2_hintmap_is_valid(initial_hint_map.as_deref()) {
        /* make recursive call with initialHintMap and temporary mask; */
        /* temporary mask will get all bits set, below */
        cf2_hintmask_init(&mut temp_hint_mask);
        if let Some(initial) = initial_hint_map.as_deref_mut() {
            cf2_hintmap_build(
                initial,
                None,
                hint_moves,
                font,
                h_stem_hint_array,
                v_stem_hint_array,
                &mut temp_hint_mask,
                hint_origin,
                true,
            );
        }
    }

    if !cf2_hintmask_is_valid(hint_mask) {
        /* without a hint mask, assume all hints are active */
        cf2_hintmask_set_all(
            hint_mask,
            cf2_arrstack_size(h_stem_hint_array) + cf2_arrstack_size(v_stem_hint_array),
            &mut font.error,
        );
        if !cf2_hintmask_is_valid(hint_mask) {
            if font.isT1 {
                /* no error, just continue unhinted */
                font.error = FT_ERR_OK;
                hintmap.hinted = false;
            }
            return; /* too many stem hints */
        }
    }

    /* begin by clearing the map */
    hintmap.count = 0;
    hintmap.lastIndex = 0;

    /* make a copy of the hint mask so we can modify it */
    temp_hint_mask = *hint_mask;
    let mut mask_ptr: usize = cf2_hintmask_get_mask_ptr(&temp_hint_mask);

    /* use the hStem hints only, which are first in the mask */
    let bit_count: usize = cf2_arrstack_size(h_stem_hint_array);

    /* Defense-in-depth.  Should never return here. */
    if bit_count > hint_mask.bitCount {
        return;
    }

    /* synthetic embox hints get highest priority */
    if font.blues.doEmBoxHints {
        let mut dummy = Cf2HintRec::default();

        cf2_hint_init_zero(&mut dummy); /* invalid hint map element */

        /* ghost bottom */
        let mut bottom = font.blues.emBoxBottomEdge;
        cf2_hintmap_insert_hint(
            hintmap,
            initial_hint_map.as_deref_mut(),
            &mut bottom,
            &mut dummy,
        );
        font.blues.emBoxBottomEdge = bottom;

        /* ghost top */
        let mut top = font.blues.emBoxTopEdge;
        cf2_hintmap_insert_hint(
            hintmap,
            initial_hint_map.as_deref_mut(),
            &mut dummy,
            &mut top,
        );
        font.blues.emBoxTopEdge = top;
    }

    /* insert hints captured by a blue zone or already locked (higher */
    /* priority)                                                      */
    mask_byte = 0x80;
    for i in 0..bit_count {
        if mask_byte & temp_hint_mask.mask[mask_ptr] != 0 {
            /* expand StemHint into two `CF2_Hint' elements */
            let mut bottom_hint_edge = Cf2HintRec::default();
            let mut top_hint_edge = Cf2HintRec::default();

            let scale = hintmap.scale;
            cf2_hint_init(
                &mut bottom_hint_edge,
                h_stem_hint_array,
                i,
                font,
                hint_origin,
                scale,
                true, /* bottom */
            );
            cf2_hint_init(
                &mut top_hint_edge,
                h_stem_hint_array,
                i,
                font,
                hint_origin,
                scale,
                false, /* top */
            );

            if cf2_hint_is_locked(&bottom_hint_edge)
                || cf2_hint_is_locked(&top_hint_edge)
                || cf2_blues_capture(&font.blues, &mut bottom_hint_edge, &mut top_hint_edge)
            {
                /* insert captured hint into map */
                cf2_hintmap_insert_hint(
                    hintmap,
                    initial_hint_map.as_deref_mut(),
                    &mut bottom_hint_edge,
                    &mut top_hint_edge,
                );

                temp_hint_mask.mask[mask_ptr] &= !mask_byte; /* turn off the bit for this hint */
            }
        }

        if (i & 7) == 7 {
            /* move to next mask byte */
            mask_ptr += 1;
            mask_byte = 0x80;
        } else {
            mask_byte >>= 1;
        }
    }

    /* initial hint map includes only captured hints plus maybe one at 0 */

    /*
     * TODO: There is a problem here because we are trying to build a
     *       single hint map containing all captured hints.  It is
     *       possible for there to be conflicts between captured hints,
     *       either because of darkening or because the hints are in
     *       separate hint zones (we are ignoring hint zones for the
     *       initial map).  An example of the latter is MinionPro-Regular
     *       v2.030 glyph 883 (Greek Capital Alpha with Psili) at 15ppem.
     *       A stem hint for the psili conflicts with the top edge hint
     *       for the base character.  The stem hint gets priority because
     *       of its sort order.  In glyph 884 (Greek Capital Alpha with
     *       Psili and Oxia), the top of the base character gets a stem
     *       hint, and the psili does not.  This creates different initial
     *       maps for the two glyphs resulting in different renderings of
     *       the base character.  Will probably defer this either as not
     *       worth the cost or as a font bug.  I don't think there is any
     *       good reason for an accent to be captured by an alignment
     *       zone.  -darnold 2/12/10
     */

    if initial_map {
        /* Apply a heuristic that inserts a point for (0,0), unless it's     */
        /* already covered by a mapping.  This locks the baseline for glyphs */
        /* that have no baseline hints.                                      */

        if hintmap.count == 0
            || hintmap.edge[0].csCoord > 0
            || hintmap.edge[hintmap.count as usize - 1].csCoord < 0
        {
            /* all edges are above 0 or all edges are below 0; */
            /* construct a locked edge hint at 0               */

            let mut edge = Cf2HintRec::default();
            let mut invalid = Cf2HintRec::default();

            cf2_hint_init_zero(&mut edge);

            edge.flags = CF2_GHOST_BOTTOM | CF2_LOCKED | CF2_SYNTHETIC;
            edge.scale = hintmap.scale;

            cf2_hint_init_zero(&mut invalid);
            cf2_hintmap_insert_hint(
                hintmap,
                initial_hint_map.as_deref_mut(),
                &mut edge,
                &mut invalid,
            );
        }
    } else {
        /* insert remaining hints */

        mask_ptr = cf2_hintmask_get_mask_ptr(&temp_hint_mask);

        mask_byte = 0x80;
        for i in 0..bit_count {
            if mask_byte & temp_hint_mask.mask[mask_ptr] != 0 {
                let mut bottom_hint_edge = Cf2HintRec::default();
                let mut top_hint_edge = Cf2HintRec::default();

                let scale = hintmap.scale;
                cf2_hint_init(
                    &mut bottom_hint_edge,
                    h_stem_hint_array,
                    i,
                    font,
                    hint_origin,
                    scale,
                    true, /* bottom */
                );
                cf2_hint_init(
                    &mut top_hint_edge,
                    h_stem_hint_array,
                    i,
                    font,
                    hint_origin,
                    scale,
                    false, /* top */
                );

                cf2_hintmap_insert_hint(
                    hintmap,
                    initial_hint_map.as_deref_mut(),
                    &mut bottom_hint_edge,
                    &mut top_hint_edge,
                );
            }

            if (i & 7) == 7 {
                /* move to next mask byte */
                mask_ptr += 1;
                mask_byte = 0x80;
            } else {
                mask_byte >>= 1;
            }
        }
    }

    cf2_hintmap_dump(hintmap);

    /*
     * Note: The following line is a convenient place to break when
     *       debugging hinting.  Examine `hintmap->edge' for the list of
     *       enabled hints, then step over the call to see the effect of
     *       adjustment.  We stop here first on the recursive call that
     *       creates the initial map, and then on each counter group and
     *       hint zone.
     */

    /* adjust positions of hint edges that are not locked to blue zones */
    cf2_hintmap_adjust_hints(hintmap, hint_moves, &mut font.error);

    cf2_hintmap_dump(hintmap);

    /* save the position of all hints that were used in this hint map; */
    /* if we use them again, we'll locate them in the same position    */
    if !initial_map {
        for i in 0..hintmap.count as usize {
            if !cf2_hint_is_synthetic(&hintmap.edge[i]) {
                /* Note: include both valid and invalid edges            */
                /* Note: top and bottom edges are copied back separately */
                let k = cf2_arrstack_get_pointer(
                    h_stem_hint_array,
                    hintmap.edge[i].index,
                    &mut font.error,
                );

                if let Some(stemhint) = h_stem_hint_array.items.get_mut(k) {
                    if cf2_hint_is_top(&hintmap.edge[i]) {
                        stemhint.maxDS = hintmap.edge[i].dsCoord;
                    } else {
                        stemhint.minDS = hintmap.edge[i].dsCoord;
                    }

                    stemhint.used = true;
                }
            }
        }
    }

    /* hint map is ready to use */
    hintmap.isValid = true;

    /* remember this mask has been used */
    cf2_hintmask_set_new(hint_mask, false);
}

/// `cf2_glyphpath_init` (the font, callbacks, stem hint arrays and hint
/// mask are given to the functions using them; `blues` is the font's)
pub fn cf2_glyphpath_init(
    glyphpath: &mut Cf2GlyphPathRec,
    font: &Cf2FontRec,
    scale_y: Cf2Fixed,
    /* CF2_Fixed hShift, */
    hint_origin_y: Cf2Fixed,
    fractional_translation: &FtVector,
) {
    *glyphpath = Cf2GlyphPathRec::default();

    cf2_arrstack_init(&mut glyphpath.hintMoves);

    cf2_hintmap_init(&mut glyphpath.initialHintMap, font, scale_y);
    cf2_hintmap_init(&mut glyphpath.firstHintMap, font, scale_y);
    cf2_hintmap_init(&mut glyphpath.hintMap, font, scale_y);

    glyphpath.scaleX = font.innerTransform.a;
    glyphpath.scaleC = font.innerTransform.c;
    glyphpath.scaleY = font.innerTransform.d;

    glyphpath.fractionalTranslation = *fractional_translation;

    glyphpath.hintOriginY = hint_origin_y;
    glyphpath.darken = font.darkened; /* TODO: should we make copies? */
    glyphpath.xOffset = font.darkenX;
    glyphpath.yOffset = font.darkenY;
    glyphpath.miterLimit =
        2i32.wrapping_mul(cf2_fixed_abs(glyphpath.xOffset).max(cf2_fixed_abs(glyphpath.yOffset)));

    /* .1 character space unit */
    glyphpath.snapThreshold = cf2_double_to_fixed(0.1);

    glyphpath.moveIsPending = true;
    glyphpath.pathIsOpen = false;
    glyphpath.pathIsClosing = false;
    glyphpath.elemIsQueued = false;
}

/// `cf2_glyphpath_finalize`
pub fn cf2_glyphpath_finalize(glyphpath: &mut Cf2GlyphPathRec) {
    cf2_arrstack_finalize(&mut glyphpath.hintMoves);
}

/*
 * Hint point in y-direction and apply outerTransform.
 * Input `current' hint map (which is actually delayed by one element).
 * Input x,y point in Character Space.
 * Output x,y point in Device Space, including translation.
 */
fn cf2_glyphpath_hint_point(
    glyphpath: &GlyphPathScale,
    font: &Cf2FontRec,
    hintmap: &mut Cf2HintMapRec,
    ppt: &mut FtVector,
    x: Cf2Fixed,
    y: Cf2Fixed,
) {
    /* hinted point in upright DS */
    let pt = FtVector {
        x: add_int32(
            ft_mul_fix(glyphpath.scaleX as FtLong, x as FtLong) as Cf2Fixed,
            ft_mul_fix(glyphpath.scaleC as FtLong, y as FtLong) as Cf2Fixed,
        ) as FtPos,
        y: cf2_hintmap_map(hintmap, y) as FtPos,
    };

    ppt.x = add_int32(
        ft_mul_fix(font.outerTransform.a as FtLong, pt.x) as Cf2Fixed,
        add_int32(
            ft_mul_fix(font.outerTransform.c as FtLong, pt.y) as Cf2Fixed,
            glyphpath.fractionalTranslation.x as Cf2Fixed,
        ),
    ) as FtPos;
    ppt.y = add_int32(
        ft_mul_fix(font.outerTransform.b as FtLong, pt.x) as Cf2Fixed,
        add_int32(
            ft_mul_fix(font.outerTransform.d as FtLong, pt.y) as Cf2Fixed,
            glyphpath.fractionalTranslation.y as Cf2Fixed,
        ),
    ) as FtPos;
}

/// The members of a glyph path `cf2_glyphpath_hintPoint` reads.
#[derive(Debug, Clone, Copy)]
struct GlyphPathScale {
    scaleX: Cf2Fixed,
    scaleC: Cf2Fixed,
    fractionalTranslation: FtVector,
}

impl Cf2GlyphPathRec {
    fn scale(&self) -> GlyphPathScale {
        GlyphPathScale {
            scaleX: self.scaleX,
            scaleC: self.scaleC,
            fractionalTranslation: self.fractionalTranslation,
        }
    }
}

/*
 * From two line segments, (u1,u2) and (v1,v2), compute a point of
 * intersection on the corresponding lines.
 * Return false if no intersection is found, or if the intersection is
 * too far away from the ends of the line segments, u2 and v1.
 *
 */
fn cf2_glyphpath_compute_intersection(
    glyphpath: &Cf2GlyphPathRec,
    u1: &FtVector,
    u2: &FtVector,
    v1: &FtVector,
    v2: &FtVector,
    intersection: &mut FtVector,
) -> bool {
    /*
     * Let `u' be a zero-based vector from the first segment, `v' from the
     * second segment.
     * Let `w 'be the zero-based vector from `u1' to `v1'.
     * `perp' is the `perpendicular dot product'; see
     * https://mathworld.wolfram.com/PerpDotProduct.html.
     * `s' is the parameter for the parametric line for the first segment
     * (`u').
     *
     * See notation in
     * http://geomalgorithms.com/a05-_intersect-1.html.
     * Calculations are done in 16.16, but must handle the squaring of
     * line lengths in character space.  We scale all vectors by 1/32 to
     * avoid overflow.  This allows values up to 4095 to be squared.  The
     * scale factor cancels in the divide.
     *
     * TODO: the scale factor could be computed from UnitsPerEm.
     *
     */

    fn cf2_perp(a: &FtVector, b: &FtVector) -> FtLong {
        ft_mul_fix(a.x, b.y) - ft_mul_fix(a.y, b.x)
    }

    /* round and divide by 32 (of a 32-bit difference, in `int') */
    fn cf2_cs_scale(x: FtLong) -> FtLong {
        ((x as i32).wrapping_add(0x10) >> 5) as FtLong
    }

    let mut u = FtVector::default(); /* scaled vectors */
    let mut v = FtVector::default();
    let mut w = FtVector::default();

    let sub = |a: FtPos, b: FtPos| sub_int32(a as i32, b as i32) as FtLong;

    u.x = cf2_cs_scale(sub(u2.x, u1.x));
    u.y = cf2_cs_scale(sub(u2.y, u1.y));
    v.x = cf2_cs_scale(sub(v2.x, v1.x));
    v.y = cf2_cs_scale(sub(v2.y, v1.y));
    w.x = cf2_cs_scale(sub(v1.x, u1.x));
    w.y = cf2_cs_scale(sub(v1.y, u1.y));

    let denominator: Cf2Fixed = cf2_perp(&u, &v) as Cf2Fixed;

    if denominator == 0 {
        return false; /* parallel or coincident lines */
    }

    let s: Cf2Fixed = ft_div_fix(cf2_perp(&w, &v), denominator as FtLong) as Cf2Fixed;

    intersection.x = add_int32(
        u1.x as i32,
        ft_mul_fix(s as FtLong, sub(u2.x, u1.x)) as Cf2Fixed,
    ) as FtPos;
    intersection.y = add_int32(
        u1.y as i32,
        ft_mul_fix(s as FtLong, sub(u2.y, u1.y)) as Cf2Fixed,
    ) as FtPos;

    /*
     * Special case snapping for horizontal and vertical lines.
     * This cleans up intersections and reduces problems with winding
     * order detection.
     * Sample case is sbc cd KozGoPr6N-Medium.otf 20 16685.
     * Note: these calculations are in character space.
     *
     */

    let snap =
        |a: FtPos, b: FtPos| cf2_fixed_abs(sub_int32(a as i32, b as i32)) < glyphpath.snapThreshold;

    if u1.x == u2.x && snap(intersection.x, u1.x) {
        intersection.x = u1.x;
    }
    if u1.y == u2.y && snap(intersection.y, u1.y) {
        intersection.y = u1.y;
    }

    if v1.x == v2.x && snap(intersection.x, v1.x) {
        intersection.x = v1.x;
    }
    if v1.y == v2.y && snap(intersection.y, v1.y) {
        intersection.y = v1.y;
    }

    /* limit the intersection distance from midpoint of u2 and v1 */
    let far = |i: FtPos, a: FtPos, b: FtPos| {
        let d = i - (add_int32(a as i32, b as i32) / 2) as FtLong;
        /* `cf2_fixedAbs' of the (64-bit) difference: its `NEG_INT32' */
        /* gives a 32-bit value for a negative difference             */
        let abs = if d < 0 {
            neg_int32(d as i32) as FtLong
        } else {
            d
        };
        abs > glyphpath.miterLimit as FtLong
    };
    if far(intersection.x, u2.x, v1.x) || far(intersection.y, u2.y, v1.y) {
        return false;
    }

    true
}

/// Calls the outline callback for `params.op` (`callbacks->lineTo` or
/// `callbacks->cubeTo`).
fn cf2_glyphpath_callback(ctx: &mut Cf2Ctx<'_, '_, '_>, params: &Cf2CallbackParamsRec) {
    match params.op {
        CF2_PATH_OP_MOVE_TO => cf2_builder_move_to(ctx.decoder, params),
        CF2_PATH_OP_LINE_TO => cf2_builder_line_to(ctx.decoder, &mut ctx.font.error, params),
        _ => cf2_builder_cube_to(ctx.decoder, &mut ctx.font.error, params),
    }
}

/*
 * Push the cached element (glyphpath->prevElem*) to the outline
 * consumer.  When a darkening offset is used, the end point of the
 * cached element may be adjusted to an intersection point or we may
 * synthesize a connecting line to the current element.  If we are
 * closing a subpath, we may also generate a connecting line to the start
 * point.
 *
 * This is where Character Space (CS) is converted to Device Space (DS)
 * using a hint map.  This calculation must use a HintMap that was valid
 * at the time the element was saved.  For the first point in a subpath,
 * that is a saved HintMap.  For most elements, it just means the caller
 * has delayed building a HintMap from the current HintMask.
 *
 * Transform each point with outerTransform and call the outline
 * callbacks.  This is a general 3x3 transform:
 *
 *   x' = a*x + c*y + tx, y' = b*x + d*y + ty
 *
 * but it uses 4 elements from CF2_Font and the translation part
 * from CF2_GlyphPath.
 *
 */

/// `cf2_glyphpath_pushPrevElem` (the hint map is `glyphpath->hintMap`;
/// `next_p0` is in or out of `glyphpath`, as `next_p0_start` says: the
/// close case passes `&glyphpath->offsetStart0`)
fn cf2_glyphpath_push_prev_elem(
    glyphpath: &mut Cf2GlyphPathRec,
    ctx: &mut Cf2Ctx<'_, '_, '_>,
    next_p0: &mut FtVector,
    next_p1: FtVector,
    close: bool,
) {
    let mut params = Cf2CallbackParamsRec::default();

    let mut intersection = FtVector { x: 0, y: 0 };
    let mut use_intersection = false;

    let (prev_p0, prev_p1) = if glyphpath.prevElemOp == CF2_PATH_OP_LINE_TO {
        (glyphpath.prevElemP0, glyphpath.prevElemP1)
    } else {
        (glyphpath.prevElemP2, glyphpath.prevElemP3)
    };

    /* optimization: if previous and next elements are offset by the same */
    /* amount, then there will be no gap, and no need to compute an       */
    /* intersection.                                                      */
    if prev_p1.x != next_p0.x || prev_p1.y != next_p0.y {
        /* previous element does not join next element:             */
        /* adjust end point of previous element to the intersection */
        use_intersection = cf2_glyphpath_compute_intersection(
            glyphpath,
            &prev_p0,
            &prev_p1,
            next_p0,
            &next_p1,
            &mut intersection,
        );
        if use_intersection {
            /* modify the last point of the cached element (either line or */
            /* curve)                                                      */
            if glyphpath.prevElemOp == CF2_PATH_OP_LINE_TO {
                glyphpath.prevElemP1 = intersection;
            } else {
                glyphpath.prevElemP3 = intersection;
            }
        }
    }

    params.pt0 = glyphpath.currentDS;

    match glyphpath.prevElemOp {
        CF2_PATH_OP_LINE_TO => {
            params.op = CF2_PATH_OP_LINE_TO;

            /* note: pt2 and pt3 are unused */

            let (x, y) = (
                glyphpath.prevElemP1.x as Cf2Fixed,
                glyphpath.prevElemP1.y as Cf2Fixed,
            );
            if close {
                /* use first hint map if closing */
                let gs = glyphpath.scale();
                cf2_glyphpath_hint_point(
                    &gs,
                    ctx.font,
                    &mut glyphpath.firstHintMap,
                    &mut params.pt1,
                    x,
                    y,
                );
            } else {
                let gs = glyphpath.scale();
                cf2_glyphpath_hint_point(
                    &gs,
                    ctx.font,
                    &mut glyphpath.hintMap,
                    &mut params.pt1,
                    x,
                    y,
                );
            }

            /* output only non-zero length lines */
            if params.pt0.x != params.pt1.x || params.pt0.y != params.pt1.y {
                cf2_glyphpath_callback(ctx, &params);

                glyphpath.currentDS = params.pt1;
            }
        }

        CF2_PATH_OP_CUBE_TO => {
            params.op = CF2_PATH_OP_CUBE_TO;

            /* TODO: should we intersect the interior joins (p1-p2 and p2-p3)? */
            let gs = glyphpath.scale();
            let (p1, p2, p3) = (
                glyphpath.prevElemP1,
                glyphpath.prevElemP2,
                glyphpath.prevElemP3,
            );
            cf2_glyphpath_hint_point(
                &gs,
                ctx.font,
                &mut glyphpath.hintMap,
                &mut params.pt1,
                p1.x as Cf2Fixed,
                p1.y as Cf2Fixed,
            );
            cf2_glyphpath_hint_point(
                &gs,
                ctx.font,
                &mut glyphpath.hintMap,
                &mut params.pt2,
                p2.x as Cf2Fixed,
                p2.y as Cf2Fixed,
            );
            cf2_glyphpath_hint_point(
                &gs,
                ctx.font,
                &mut glyphpath.hintMap,
                &mut params.pt3,
                p3.x as Cf2Fixed,
                p3.y as Cf2Fixed,
            );

            cf2_glyphpath_callback(ctx, &params);

            glyphpath.currentDS = params.pt3;
        }

        _ => {}
    }

    if !use_intersection || close {
        /* insert connecting line between end of previous element and start */
        /* of current one                                                   */
        /* note: at the end of a subpath, we might do both, so use `nextP0' */
        /* before we change it, below                                       */

        if close {
            /* if we are closing the subpath, then nextP0 is in the first     */
            /* hint zone                                                      */
            let gs = glyphpath.scale();
            cf2_glyphpath_hint_point(
                &gs,
                ctx.font,
                &mut glyphpath.firstHintMap,
                &mut params.pt1,
                next_p0.x as Cf2Fixed,
                next_p0.y as Cf2Fixed,
            );
        } else {
            let gs = glyphpath.scale();
            cf2_glyphpath_hint_point(
                &gs,
                ctx.font,
                &mut glyphpath.hintMap,
                &mut params.pt1,
                next_p0.x as Cf2Fixed,
                next_p0.y as Cf2Fixed,
            );
        }

        if params.pt1.x != glyphpath.currentDS.x || params.pt1.y != glyphpath.currentDS.y {
            /* length is nonzero */
            params.op = CF2_PATH_OP_LINE_TO;
            params.pt0 = glyphpath.currentDS;

            /* note: pt2 and pt3 are unused */
            cf2_glyphpath_callback(ctx, &params);

            glyphpath.currentDS = params.pt1;
        }
    }

    if use_intersection {
        /* return intersection point to caller */
        *next_p0 = intersection;
    }
}

/* push a MoveTo element based on current point and offset of current */
/* element                                                            */
fn cf2_glyphpath_push_move(
    glyphpath: &mut Cf2GlyphPathRec,
    ctx: &mut Cf2Ctx<'_, '_, '_>,
    hints: &mut Cf2Hints,
    start: FtVector,
) {
    let mut params = Cf2CallbackParamsRec {
        op: CF2_PATH_OP_MOVE_TO,
        pt0: glyphpath.currentDS,
        ..Default::default()
    };

    /* Test if move has really happened yet; it would have called */
    /* `cf2_hintmap_build' to set `isValid'.                   */
    if !glyphpath.hintMap.isValid {
        /* we are here iff first subpath is missing a moveto operator: */
        /* synthesize first moveTo to finish initialization of hintMap */
        let (x, y) = (glyphpath.start.x as Cf2Fixed, glyphpath.start.y as Cf2Fixed);
        cf2_glyphpath_move_to(glyphpath, ctx, hints, x, y);
    }

    let gs = glyphpath.scale();
    cf2_glyphpath_hint_point(
        &gs,
        ctx.font,
        &mut glyphpath.hintMap,
        &mut params.pt1,
        start.x as Cf2Fixed,
        start.y as Cf2Fixed,
    );

    /* note: pt2 and pt3 are unused */
    cf2_glyphpath_callback(ctx, &params);

    glyphpath.currentDS = params.pt1;
    glyphpath.offsetStart0 = start;
}

/*
 * All coordinates are in character space.
 * On input, (x1, y1) and (x2, y2) give line segment.
 * On output, (x, y) give offset vector.
 * We use a piecewise approximation to trig functions.
 *
 * TODO: Offset true perpendicular and proper length
 *       supply the y-translation for hinting here, too,
 *       that adds yOffset unconditionally to *y.
 */
#[allow(clippy::too_many_arguments)]
fn cf2_glyphpath_compute_offset(
    glyphpath: &Cf2GlyphPathRec,
    ctx: &mut Cf2Ctx<'_, '_, '_>,
    x1: Cf2Fixed,
    y1: Cf2Fixed,
    x2: Cf2Fixed,
    y2: Cf2Fixed,
    x: &mut Cf2Fixed,
    y: &mut Cf2Fixed,
) {
    let mut dx: Cf2Fixed = sub_int32(x2, x1);
    let mut dy: Cf2Fixed = sub_int32(y2, y1);

    /* note: negative offsets don't work here; negate deltas to change */
    /* quadrants, below                                                */
    if ctx.font.reverseWinding {
        dx = neg_int32(dx);
        dy = neg_int32(dy);
    }

    *x = 0;
    *y = 0;

    if !glyphpath.darken {
        return;
    }

    /* add momentum for this path element */
    ctx.font.outline.windingMomentum = add_int32(
        ctx.font.outline.windingMomentum,
        cf2_get_winding_momentum(x1, y1, x2, y2),
    );

    let mul = |a: Cf2Fixed, b: Cf2Fixed| ft_mul_fix(a as FtLong, b as FtLong) as Cf2Fixed;

    /* note: allow mixed integer and fixed multiplication here */
    if dx >= 0 {
        if dy >= 0 {
            /* first quadrant, +x +y */

            if dx > mul_int32(2, dy) {
                /* +x */
                *x = 0;
                *y = 0;
            } else if dy > mul_int32(2, dx) {
                /* +y */
                *x = glyphpath.xOffset;
                *y = glyphpath.yOffset;
            } else {
                /* +x +y */
                *x = mul(cf2_double_to_fixed(0.7), glyphpath.xOffset);
                *y = mul(cf2_double_to_fixed(1.0 - 0.7), glyphpath.yOffset);
            }
        } else {
            /* fourth quadrant, +x -y */

            if dx > mul_int32(-2, dy) {
                /* +x */
                *x = 0;
                *y = 0;
            } else if neg_int32(dy) > mul_int32(2, dx) {
                /* -y */
                *x = neg_int32(glyphpath.xOffset);
                *y = glyphpath.yOffset;
            } else {
                /* +x -y */
                *x = mul(cf2_double_to_fixed(-0.7), glyphpath.xOffset);
                *y = mul(cf2_double_to_fixed(1.0 - 0.7), glyphpath.yOffset);
            }
        }
    } else if dy >= 0 {
        /* second quadrant, -x +y */

        if neg_int32(dx) > mul_int32(2, dy) {
            /* -x */
            *x = 0;
            *y = mul_int32(2, glyphpath.yOffset);
        } else if dy > mul_int32(-2, dx) {
            /* +y */
            *x = glyphpath.xOffset;
            *y = glyphpath.yOffset;
        } else {
            /* -x +y */
            *x = mul(cf2_double_to_fixed(0.7), glyphpath.xOffset);
            *y = mul(cf2_double_to_fixed(1.0 + 0.7), glyphpath.yOffset);
        }
    } else {
        /* third quadrant, -x -y */

        if neg_int32(dx) > mul_int32(-2, dy) {
            /* -x */
            *x = 0;
            *y = mul_int32(2, glyphpath.yOffset);
        } else if neg_int32(dy) > mul_int32(-2, dx) {
            /* -y */
            *x = neg_int32(glyphpath.xOffset);
            *y = glyphpath.yOffset;
        } else {
            /* -x -y */
            *x = mul(cf2_double_to_fixed(-0.7), glyphpath.xOffset);
            *y = mul(cf2_double_to_fixed(1.0 + 0.7), glyphpath.yOffset);
        }
    }
}

/// Builds `glyphpath->hintMap` (`cf2_hintmap_build` with the glyph path's
/// references).
fn cf2_glyphpath_build_hint_map(
    glyphpath: &mut Cf2GlyphPathRec,
    font: &mut Cf2FontRec,
    hints: &mut Cf2Hints,
) {
    let Cf2GlyphPathRec {
        hintMap,
        initialHintMap,
        hintMoves,
        hintOriginY,
        ..
    } = glyphpath;
    cf2_hintmap_build(
        hintMap,
        Some(initialHintMap),
        hintMoves,
        font,
        &mut hints.hStemHintArray,
        &hints.vStemHintArray,
        &mut hints.hintMask,
        *hintOriginY,
        false,
    );
}

/*
 * The functions cf2_glyphpath_{moveTo,lineTo,curveTo,closeOpenPath} are
 * called by the interpreter with Character Space (CS) coordinates.  Each
 * path element is placed into a queue of length one to await the
 * calculation of the following element.  At that time, the darkening
 * offset of the following element is known and joins can be computed,
 * including possible modification of this element, before mapping to
 * Device Space (DS) and passing it on to the outline consumer.
 *
 */

/// `cf2_glyphpath_moveTo`
pub fn cf2_glyphpath_move_to(
    glyphpath: &mut Cf2GlyphPathRec,
    ctx: &mut Cf2Ctx<'_, '_, '_>,
    hints: &mut Cf2Hints,
    x: Cf2Fixed,
    y: Cf2Fixed,
) {
    cf2_glyphpath_close_open_path(glyphpath, ctx, hints);

    /* save the parameters of the move for later, when we'll know how to */
    /* offset it;                                                        */
    /* also save last move point */
    glyphpath.start.x = x as FtPos;
    glyphpath.currentCS.x = x as FtPos;
    glyphpath.start.y = y as FtPos;
    glyphpath.currentCS.y = y as FtPos;

    glyphpath.moveIsPending = true;

    /* ensure we have a valid map with current mask */
    if !glyphpath.hintMap.isValid || cf2_hintmask_is_new(&hints.hintMask) {
        cf2_glyphpath_build_hint_map(glyphpath, ctx.font, hints);
    }

    /* save a copy of current HintMap to use when drawing initial point */
    glyphpath.firstHintMap = glyphpath.hintMap; /* structure copy */
}

/// `cf2_glyphpath_lineTo`
pub fn cf2_glyphpath_line_to(
    glyphpath: &mut Cf2GlyphPathRec,
    ctx: &mut Cf2Ctx<'_, '_, '_>,
    hints: &mut Cf2Hints,
    x: Cf2Fixed,
    y: Cf2Fixed,
) {
    let mut x_offset: Cf2Fixed = 0;
    let mut y_offset: Cf2Fixed = 0;
    let mut p0 = FtVector::default();
    let mut p1 = FtVector::default();

    /*
     * New hints will be applied after cf2_glyphpath_pushPrevElem has run.
     * In case this is a synthesized closing line, any new hints should be
     * delayed until this path is closed (`cf2_hintmask_isNew' will be
     * called again before the next line or curve).
     */

    /* true if new hint map not on close */
    let new_hint_map = cf2_hintmask_is_new(&hints.hintMask) && !glyphpath.pathIsClosing;

    /*
     * Zero-length lines may occur in the charstring.  Because we cannot
     * compute darkening offsets or intersections from zero-length lines,
     * it is best to remove them and avoid artifacts.  However, zero-length
     * lines in CS at the start of a new hint map can generate non-zero
     * lines in DS due to hint substitution.  We detect a change in hint
     * map here and pass those zero-length lines along.
     */

    /*
     * Note: Find explicitly closed paths here with a conditional
     *       breakpoint using
     *
     *         !gp->pathIsClosing && gp->start.x == x && gp->start.y == y
     *
     */

    if glyphpath.currentCS.x == x as FtPos && glyphpath.currentCS.y == y as FtPos && !new_hint_map {
        /*
         * Ignore zero-length lines in CS where the hint map is the same
         * because the line in DS will also be zero length.
         *
         * Ignore zero-length lines when we synthesize a closing line because
         * the close will be handled in cf2_glyphPath_pushPrevElem.
         */
        return;
    }

    let (cx, cy) = (
        glyphpath.currentCS.x as Cf2Fixed,
        glyphpath.currentCS.y as Cf2Fixed,
    );
    cf2_glyphpath_compute_offset(glyphpath, ctx, cx, cy, x, y, &mut x_offset, &mut y_offset);

    /* construct offset points */
    p0.x = add_int32(cx, x_offset) as FtPos;
    p0.y = add_int32(cy, y_offset) as FtPos;
    p1.x = add_int32(x, x_offset) as FtPos;
    p1.y = add_int32(y, y_offset) as FtPos;

    if glyphpath.moveIsPending {
        /* emit offset 1st point as MoveTo */
        cf2_glyphpath_push_move(glyphpath, ctx, hints, p0);

        glyphpath.moveIsPending = false; /* adjust state machine */
        glyphpath.pathIsOpen = true;

        glyphpath.offsetStart1 = p1; /* record second point */
    }

    if glyphpath.elemIsQueued {
        cf2_glyphpath_push_prev_elem(glyphpath, ctx, &mut p0, p1, false);
    }

    /* queue the current element with offset points */
    glyphpath.elemIsQueued = true;
    glyphpath.prevElemOp = CF2_PATH_OP_LINE_TO;
    glyphpath.prevElemP0 = p0;
    glyphpath.prevElemP1 = p1;

    /* update current map */
    if new_hint_map {
        cf2_glyphpath_build_hint_map(glyphpath, ctx.font, hints);
    }

    glyphpath.currentCS.x = x as FtPos; /* pre-offset current point */
    glyphpath.currentCS.y = y as FtPos;
}

/// `cf2_glyphpath_curveTo`
#[allow(clippy::too_many_arguments)]
pub fn cf2_glyphpath_curve_to(
    glyphpath: &mut Cf2GlyphPathRec,
    ctx: &mut Cf2Ctx<'_, '_, '_>,
    hints: &mut Cf2Hints,
    x1: Cf2Fixed,
    y1: Cf2Fixed,
    x2: Cf2Fixed,
    y2: Cf2Fixed,
    x3: Cf2Fixed,
    y3: Cf2Fixed,
) {
    let mut x_offset1: Cf2Fixed = 0;
    let mut y_offset1: Cf2Fixed = 0;
    let mut x_offset3: Cf2Fixed = 0;
    let mut y_offset3: Cf2Fixed = 0;
    let mut p0 = FtVector::default();
    let mut p1 = FtVector::default();
    let mut p2 = FtVector::default();
    let mut p3 = FtVector::default();

    /* TODO: ignore zero length portions of curve?? */
    let (cx, cy) = (
        glyphpath.currentCS.x as Cf2Fixed,
        glyphpath.currentCS.y as Cf2Fixed,
    );
    cf2_glyphpath_compute_offset(
        glyphpath,
        ctx,
        cx,
        cy,
        x1,
        y1,
        &mut x_offset1,
        &mut y_offset1,
    );
    cf2_glyphpath_compute_offset(
        glyphpath,
        ctx,
        x2,
        y2,
        x3,
        y3,
        &mut x_offset3,
        &mut y_offset3,
    );

    /* add momentum from the middle segment */
    ctx.font.outline.windingMomentum = add_int32(
        ctx.font.outline.windingMomentum,
        cf2_get_winding_momentum(x1, y1, x2, y2),
    );

    /* construct offset points */
    p0.x = add_int32(cx, x_offset1) as FtPos;
    p0.y = add_int32(cy, y_offset1) as FtPos;
    p1.x = add_int32(x1, x_offset1) as FtPos;
    p1.y = add_int32(y1, y_offset1) as FtPos;
    /* note: preserve angle of final segment by using offset3 at both ends */
    p2.x = add_int32(x2, x_offset3) as FtPos;
    p2.y = add_int32(y2, y_offset3) as FtPos;
    p3.x = add_int32(x3, x_offset3) as FtPos;
    p3.y = add_int32(y3, y_offset3) as FtPos;

    if glyphpath.moveIsPending {
        /* emit offset 1st point as MoveTo */
        cf2_glyphpath_push_move(glyphpath, ctx, hints, p0);

        glyphpath.moveIsPending = false;
        glyphpath.pathIsOpen = true;

        glyphpath.offsetStart1 = p1; /* record second point */
    }

    if glyphpath.elemIsQueued {
        cf2_glyphpath_push_prev_elem(glyphpath, ctx, &mut p0, p1, false);
    }

    /* queue the current element with offset points */
    glyphpath.elemIsQueued = true;
    glyphpath.prevElemOp = CF2_PATH_OP_CUBE_TO;
    glyphpath.prevElemP0 = p0;
    glyphpath.prevElemP1 = p1;
    glyphpath.prevElemP2 = p2;
    glyphpath.prevElemP3 = p3;

    /* update current map */
    if cf2_hintmask_is_new(&hints.hintMask) {
        cf2_glyphpath_build_hint_map(glyphpath, ctx.font, hints);
    }

    glyphpath.currentCS.x = x3 as FtPos; /* pre-offset current point */
    glyphpath.currentCS.y = y3 as FtPos;
}

/// `cf2_glyphpath_closeOpenPath`
pub fn cf2_glyphpath_close_open_path(
    glyphpath: &mut Cf2GlyphPathRec,
    ctx: &mut Cf2Ctx<'_, '_, '_>,
    hints: &mut Cf2Hints,
) {
    if glyphpath.pathIsOpen {
        /*
         * A closing line in Character Space line is always generated below
         * with `cf2_glyphPath_lineTo'.  It may be ignored later if it turns
         * out to be zero length in Device Space.
         */
        glyphpath.pathIsClosing = true;

        let (sx, sy) = (glyphpath.start.x as Cf2Fixed, glyphpath.start.y as Cf2Fixed);
        cf2_glyphpath_line_to(glyphpath, ctx, hints, sx, sy);

        /* empty the final element from the queue and close the path */
        if glyphpath.elemIsQueued {
            let mut start0 = glyphpath.offsetStart0;
            let start1 = glyphpath.offsetStart1;
            cf2_glyphpath_push_prev_elem(glyphpath, ctx, &mut start0, start1, true);
            glyphpath.offsetStart0 = start0;
        }

        /* reset state machine */
        glyphpath.moveIsPending = true;
        glyphpath.pathIsOpen = false;
        glyphpath.pathIsClosing = false;
        glyphpath.elemIsQueued = false;
    }
}
