// Rust translation of src/autofit/afhints.c and afhints.h from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2003-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Auto-fitter hinting routines.
//!
//! Translation notes: the points, contours, segments and edges are
//! arrays, and the C pointers between them are indices into these. The
//! segment and edge arrays keep their elements when they are reset, as
//! C's do (a new segment or edge starts out with what was there before);
//! the embedded arrays and the allocation strategy otherwise only matter
//! for memory. The `metrics` of the hints (`hints->metrics`) are passed to
//! the functions that read them. The debugging dumps (`FT_DEBUG_AUTOFIT`)
//! aren't translated.

use super::super::base::ftcalc::{ft_corner_is_flat, ft_div_fix, ft_mul_fix};
use super::super::base::ftoutln::{ft_outline_get_orientation, FT_ORIENTATION_POSTSCRIPT};
use super::super::fttypes::*;
use super::aftypes::*;

/*
 * The definition of outline glyph hints.  These are shared by all
 * writing system analysis routines (until now).
 */

/// `AF_Dimension`
pub type AfDimension = usize;
pub const AF_DIMENSION_HORZ: AfDimension = 0; /* x coordinates,                    */
/* i.e., vertical segments & edges   */
pub const AF_DIMENSION_VERT: AfDimension = 1; /* y coordinates,                    */
/* i.e., horizontal segments & edges */
pub const AF_DIMENSION_MAX: AfDimension = 2; /* do not remove */

/* hint directions -- the values are computed so that two vectors are */
/* in opposite directions iff `dir1 + dir2 == 0'                      */

/// `AF_Direction`
pub type AfDirection = i32;
pub const AF_DIR_NONE: AfDirection = 4;
pub const AF_DIR_RIGHT: AfDirection = 1;
pub const AF_DIR_LEFT: AfDirection = -1;
pub const AF_DIR_UP: AfDirection = 2;
pub const AF_DIR_DOWN: AfDirection = -2;

/*
 * The following explanations are mostly taken from the article
 *
 *   Real-Time Grid Fitting of Typographic Outlines
 *
 * by David Turner and Werner Lemberg
 *
 *   https://www.tug.org/TUGboat/Articles/tb24-3/lemberg.pdf
 *
 * with appropriate updates.
 *
 *
 * Segments
 *
 *   `af_{cjk,latin,...}_hints_compute_segments' are the functions to
 *   find segments in an outline.
 *
 *   A segment is a series of at least two consecutive points that are
 *   approximately aligned along a coordinate axis.  The analysis to do
 *   so is specific to a writing system.
 *
 *
 * Edges
 *
 *   `af_{cjk,latin,...}_hints_compute_edges' are the functions to find
 *   edges.
 *
 *   As soon as segments are defined, the auto-hinter groups them into
 *   edges.  An edge corresponds to a single position on the main
 *   dimension that collects one or more segments (allowing for a small
 *   threshold).
 *
 *   As an example, the `latin' writing system first tries to grid-fit
 *   edges, then to align segments on the edges unless it detects that
 *   they form a serif.
 *
 *
 *                     A          H
 *                      |        |
 *                      |        |
 *                      |        |
 *                      |        |
 *        C             |        |             F
 *         +------<-----+        +-----<------+
 *         |             B      G             |
 *         |                                  |
 *         |                                  |
 *         +--------------->------------------+
 *        D                                    E
 *
 *
 * Stems
 *
 *   Stems are detected by `af_{cjk,latin,...}_hint_edges'.
 *
 *   Segments need to be `linked' to other ones in order to detect stems.
 *   A stem is made of two segments that face each other in opposite
 *   directions and that are sufficiently close to each other.  Using
 *   vocabulary from the TrueType specification, stem segments form a
 *   `black distance'.
 *
 *   In the above ASCII drawing, the horizontal segments are BC, DE, and
 *   FG; the vertical segments are AB, CD, EF, and GH.
 *
 *   Each segment has at most one `best' candidate to form a black
 *   distance, or no candidate at all.  Notice that two distinct segments
 *   can have the same candidate, which frequently means a serif.
 *
 *   A stem is recognized by the following condition:
 *
 *     best segment_1 = segment_2 && best segment_2 = segment_1
 *
 *   The best candidate is stored in field `link' in structure
 *   `AF_Segment'.
 *
 *   In the above ASCII drawing, the best candidate for both AB and CD is
 *   GH, while the best candidate for GH is AB.  Similarly, the best
 *   candidate for EF and GH is AB, while the best candidate for AB is
 *   GH.
 *
 *   The detection and handling of stems is dependent on the writing
 *   system.
 *
 *
 * Serifs
 *
 *   Serifs are detected by `af_{cjk,latin,...}_hint_edges'.
 *
 *   In comparison to a stem, a serif (as handled by the auto-hinter
 *   module that takes care of the `latin' writing system) has
 *
 *     best segment_1 = segment_2 && best segment_2 != segment_1
 *
 *   where segment_1 corresponds to the serif segment (CD and EF in the
 *   above ASCII drawing).
 *
 *   The best candidate is stored in field `serif' in structure
 *   `AF_Segment' (and `link' is set to NULL).
 *
 *
 * Touched points
 *
 *   A point is called `touched' if it has been processed somehow by the
 *   auto-hinter.  It basically means that it shouldn't be moved again
 *   (or moved only under certain constraints to preserve the already
 *   applied processing).
 *
 *
 * Flat and round segments
 *
 *   Segments are `round' or `flat', depending on the series of points
 *   that define them.  A segment is round if the next and previous point
 *   of an extremum (which can be either a single point or sequence of
 *   points) are both conic or cubic control points.  Otherwise, a
 *   segment with an extremum is flat.
 *
 *
 * Strong Points
 *
 *   Experience has shown that points not part of an edge need to be
 *   interpolated linearly between their two closest edges, even if these
 *   are not part of the contour of those particular points.  Typical
 *   candidates for this are
 *
 *   - angle points (i.e., points where the `in' and `out' direction
 *     differ greatly)
 *
 *   - inflection points (i.e., where the `in' and `out' angles are the
 *     same, but the curvature changes sign) [currently, such points
 *     aren't handled specially in the auto-hinter]
 *
 *   `af_glyph_hints_align_strong_points' is the function that takes
 *   care of such situations; it is equivalent to the TrueType `IP'
 *   hinting instruction.
 *
 *
 * Weak Points
 *
 *   Other points in the outline must be interpolated using the
 *   coordinates of their previous and next unfitted contour neighbours.
 *   These are called `weak points' and are touched by the function
 *   `af_glyph_hints_align_weak_points', equivalent to the TrueType `IUP'
 *   hinting instruction.  Typical candidates are control points and
 *   points on the contour without a major direction.
 *
 *   The major effect is to reduce possible distortion caused by
 *   alignment of edges and strong points, thus weak points are processed
 *   after strong points.
 */

/* point hint flags */
pub const AF_FLAG_NONE: FtUShort = 0;

/* point type flags */
pub const AF_FLAG_CONIC: FtUShort = 1 << 0;
pub const AF_FLAG_CUBIC: FtUShort = 1 << 1;
pub const AF_FLAG_CONTROL: FtUShort = AF_FLAG_CONIC | AF_FLAG_CUBIC;

/* point touch flags */
pub const AF_FLAG_TOUCH_X: FtUShort = 1 << 2;
pub const AF_FLAG_TOUCH_Y: FtUShort = 1 << 3;

/* candidates for weak interpolation have this flag set */
pub const AF_FLAG_WEAK_INTERPOLATION: FtUShort = 1 << 4;

/* the distance to the next point is very small */
pub const AF_FLAG_NEAR: FtUShort = 1 << 5;

/* edge hint flags */
pub const AF_EDGE_NORMAL: FtByte = 0;
pub const AF_EDGE_ROUND: FtByte = 1 << 0;
pub const AF_EDGE_SERIF: FtByte = 1 << 1;
pub const AF_EDGE_DONE: FtByte = 1 << 2;
pub const AF_EDGE_NEUTRAL: FtByte = 1 << 3; /* edge aligns to a neutral blue zone */

/// `AF_PointRec` (`AF_Point` is an index into `hints.points`)
#[derive(Debug, Clone, Copy, Default)]
pub struct AfPointRec {
    pub flags: FtUShort, /* point flags used by hinter   */
    pub in_dir: i8,      /* direction of inwards vector  */
    pub out_dir: i8,     /* direction of outwards vector */

    pub ox: FtPos, /* original, scaled position                   */
    pub oy: FtPos,
    pub fx: FtShort, /* original, unscaled position (in font units) */
    pub fy: FtShort,
    pub x: FtPos, /* current position                            */
    pub y: FtPos,
    pub u: FtPos, /* current (x,y) or (y,x) depending on context */
    pub v: FtPos,

    pub next: usize, /* next point in contour     */
    pub prev: usize, /* previous point in contour */
}

/// `AF_SegmentRec` (`AF_Segment` is an index into `axis.segments`)
#[derive(Debug, Clone, Copy, Default)]
pub struct AfSegmentRec {
    pub flags: FtByte,      /* edge/segment flags for this segment */
    pub dir: i8,            /* segment direction                   */
    pub pos: FtShort,       /* position of segment                 */
    pub delta: FtShort,     /* deviation from segment position     */
    pub min_coord: FtShort, /* minimum coordinate of segment       */
    pub max_coord: FtShort, /* maximum coordinate of segment       */
    pub height: FtShort,    /* the hinted segment height           */

    pub edge: Option<usize>,      /* the segment's parent edge           */
    pub edge_next: Option<usize>, /* link to next segment in parent edge */

    pub link: Option<usize>,  /* (stem) link segment        */
    pub serif: Option<usize>, /* primary segment for serifs */
    pub score: FtPos,         /* used during stem matching  */
    pub len: FtPos,           /* used during stem matching  */

    pub first: usize, /* first point in edge segment */
    pub last: usize,  /* last point in edge segment  */
}

/// `AF_EdgeRec` (`AF_Edge` is an index into `axis.edges`)
#[derive(Debug, Clone, Copy, Default)]
pub struct AfEdgeRec {
    pub fpos: FtShort, /* original, unscaled position (in font units) */
    pub opos: FtPos,   /* original, scaled position                   */
    pub pos: FtPos,    /* current position                            */

    pub flags: FtByte,  /* edge flags                                   */
    pub dir: i8,        /* edge direction                               */
    pub scale: FtFixed, /* used to speed up interpolation between edges */

    /// non-NULL if this is a blue edge (a copy of the blue zone's width
    /// record, which doesn't change while the glyph is hinted)
    pub blue_edge: Option<AfWidthRec>,
    pub link: Option<usize>,  /* link edge                       */
    pub serif: Option<usize>, /* primary edge for serifs         */
    pub score: FtInt,         /* used during stem matching       */

    pub first: Option<usize>, /* first segment in edge */
    pub last: Option<usize>,  /* last segment in edge  */
}

pub const AF_SEGMENTS_EMBEDDED: usize = 18; /* number of embedded segments   */
pub const AF_EDGES_EMBEDDED: usize = 12; /* number of embedded edges      */

/// `AF_AxisHintsRec`
#[derive(Debug, Clone, Default)]
pub struct AfAxisHintsRec {
    pub num_segments: FtUInt, /* number of used segments      */
    /// segments array (`max_segments` is its length)
    pub segments: Vec<AfSegmentRec>,

    pub num_edges: FtUInt, /* number of used edges      */
    /// edges array (`max_edges` is its length)
    pub edges: Vec<AfEdgeRec>,

    pub major_dir: AfDirection, /* either vertical or horizontal */
}

pub const AF_POINTS_EMBEDDED: usize = 96; /* number of embedded points   */
pub const AF_CONTOURS_EMBEDDED: usize = 8; /* number of embedded contours */

/// `AF_GlyphHintsRec`
#[derive(Debug, Clone, Default)]
pub struct AfGlyphHintsRec {
    pub x_scale: FtFixed,
    pub x_delta: FtPos,

    pub y_scale: FtFixed,
    pub y_delta: FtPos,

    pub num_points: FtInt,       /* number of used points      */
    pub points: Vec<AfPointRec>, /* points array               */

    pub num_contours: FtInt, /* number of used contours      */
    /// contours array (the first point of each contour)
    pub contours: Vec<usize>,

    pub axis: [AfAxisHintsRec; AF_DIMENSION_MAX],

    pub scaler_flags: FtUInt32, /* copy of scaler flags    */
    pub other_flags: FtUInt32,  /* free for style-specific */
    /* implementations         */
    /// `metrics->scaler.face->units_per_EM` (for `af_glyph_hints_reload`)
    pub units_per_EM: FtUShort,
}

/// `AF_HINTS_TEST_SCALER`
pub fn af_hints_test_scaler(h: &AfGlyphHintsRec, f: FtUInt32) -> bool {
    h.scaler_flags & f != 0
}
/// `AF_HINTS_TEST_OTHER`
pub fn af_hints_test_other(h: &AfGlyphHintsRec, f: FtUInt32) -> bool {
    h.other_flags & f != 0
}

/* !FT_DEBUG_AUTOFIT */

/// `AF_HINTS_DO_HORIZONTAL`
pub fn af_hints_do_horizontal(h: &AfGlyphHintsRec) -> bool {
    !af_hints_test_scaler(h, AF_SCALER_FLAG_NO_HORIZONTAL)
}
/// `AF_HINTS_DO_VERTICAL`
pub fn af_hints_do_vertical(h: &AfGlyphHintsRec) -> bool {
    !af_hints_test_scaler(h, AF_SCALER_FLAG_NO_VERTICAL)
}
/// `AF_HINTS_DO_BLUES`
pub fn af_hints_do_blues(_h: &AfGlyphHintsRec) -> bool {
    true
}

/// `AF_HINTS_DO_ADVANCE`
pub fn af_hints_do_advance(h: &AfGlyphHintsRec) -> bool {
    !af_hints_test_scaler(h, AF_SCALER_FLAG_NO_ADVANCE)
}

/// `AF_SEGMENT_LEN`
pub fn af_segment_len(seg: &AfSegmentRec) -> FtPos {
    seg.max_coord as FtPos - seg.min_coord as FtPos
}

/// `AF_SEGMENT_DIST`
pub fn af_segment_dist(seg1: &AfSegmentRec, seg2: &AfSegmentRec) -> FtPos {
    if seg1.pos > seg2.pos {
        seg1.pos as FtPos - seg2.pos as FtPos
    } else {
        seg2.pos as FtPos - seg1.pos as FtPos
    }
}

/* afhints.c */

/// `af_sort_pos`
pub fn af_sort_pos(count: FtUInt, table: &mut [FtPos]) {
    for i in 1..count as usize {
        let mut j = i;
        while j > 0 {
            if table[j] >= table[j - 1] {
                break;
            }

            table.swap(j, j - 1);
            j -= 1;
        }
    }
}

/// `af_sort_and_quantize_widths`
pub fn af_sort_and_quantize_widths(count: &mut FtUInt, table: &mut [AfWidthRec], threshold: FtPos) {
    if *count == 1 {
        return;
    }

    /* sort */
    for i in 1..*count as usize {
        let mut j = i;
        while j > 0 {
            if table[j].org >= table[j - 1].org {
                break;
            }

            table.swap(j, j - 1);
            j -= 1;
        }
    }

    let mut cur_idx: usize = 0;
    let mut cur_val = table[cur_idx].org;

    /* compute and use mean values for clusters not larger than  */
    /* `threshold'; this is very primitive and might not yield   */
    /* the best result, but normally, using reference character  */
    /* `o', `*count' is 2, so the code below is fully sufficient */
    let count_ = *count as usize;
    let mut i: usize = 1;
    while i < count_ {
        if table[i].org - cur_val > threshold || i == count_ - 1 {
            let mut sum: FtPos = 0;

            /* fix loop for end of array */
            if table[i].org - cur_val <= threshold && i == count_ - 1 {
                i += 1;
            }

            let mut j = cur_idx;
            while j < i {
                sum += table[j].org;
                table[j].org = 0;
                j += 1;
            }
            table[cur_idx].org = sum / j as FtPos;

            if i < count_ - 1 {
                cur_idx = i + 1;
                cur_val = table[cur_idx].org;
            }
        }
        i += 1;
    }

    cur_idx = 1;

    /* compress array to remove zero values */
    for i in 1..count_ {
        if table[i].org != 0 {
            table[cur_idx] = table[i];
            cur_idx += 1;
        }
    }

    *count = cur_idx as FtUInt;
}

/* Get new segment for given axis. */

/// `af_axis_hints_new_segment`
pub fn af_axis_hints_new_segment(axis: &mut AfAxisHintsRec) -> FtResult<usize> {
    let n = axis.num_segments as usize;
    if n >= axis.segments.len() {
        /* the array grows by C's steps (`new_max += ( new_max >> 2 ) + 4'), */
        /* and its new elements are zeroed                                    */
        let old_max = if axis.segments.is_empty() {
            0
        } else {
            axis.segments.len()
        };
        let new_max = if old_max == 0 {
            AF_SEGMENTS_EMBEDDED
        } else {
            old_max + (old_max >> 2) + 4
        };
        if axis.segments.try_reserve_exact(new_max - old_max).is_err() {
            return Err(FT_ERR_OUT_OF_MEMORY);
        }
        axis.segments.resize(new_max, AfSegmentRec::default());
    }

    axis.num_segments += 1;
    Ok(n)
}

/* Get new edge for given axis, direction, and position, */
/* without initializing the edge itself.                 */

/// `af_axis_hints_new_edge`
pub fn af_axis_hints_new_edge(
    axis: &mut AfAxisHintsRec,
    fpos: FtInt,
    dir: AfDirection,
    top_to_bottom_hinting: bool,
) -> FtResult<usize> {
    let n = axis.num_edges as usize;
    if n >= axis.edges.len() {
        let old_max = axis.edges.len();
        let new_max = if old_max == 0 {
            AF_EDGES_EMBEDDED
        } else {
            old_max + (old_max >> 2) + 4
        };
        if axis.edges.try_reserve_exact(new_max - old_max).is_err() {
            return Err(FT_ERR_OUT_OF_MEMORY);
        }
        axis.edges.resize(new_max, AfEdgeRec::default());
    }

    let edges = &mut axis.edges;
    let mut edge = n;

    while edge > 0 {
        if if top_to_bottom_hinting {
            edges[edge - 1].fpos as FtInt > fpos
        } else {
            (edges[edge - 1].fpos as FtInt) < fpos
        } {
            break;
        }

        /* we want the edge with same position and minor direction */
        /* to appear before those in the major one in the list     */
        if edges[edge - 1].fpos as FtInt == fpos && dir == axis.major_dir {
            break;
        }

        edges[edge] = edges[edge - 1];
        edge -= 1;
    }

    axis.num_edges += 1;

    Ok(edge)
}

/* (the debugging dumps of FT_DEBUG_AUTOFIT are not translated) */

/* Compute the direction value of a given vector. */

/// `af_direction_compute`
pub fn af_direction_compute(dx: FtPos, dy: FtPos) -> AfDirection {
    let ll: FtPos; /* long and short arm lengths */
    let ss: FtPos;
    let mut dir: AfDirection; /* candidate direction        */

    if dy >= dx {
        if dy >= dx.wrapping_neg() {
            dir = AF_DIR_UP;
            ll = dy;
            ss = dx;
        } else {
            dir = AF_DIR_LEFT;
            ll = dx.wrapping_neg();
            ss = dy;
        }
    } else
    /* dy < dx */
    {
        if dy >= dx.wrapping_neg() {
            dir = AF_DIR_RIGHT;
            ll = dx;
            ss = dy;
        } else {
            dir = AF_DIR_DOWN;
            ll = dy.wrapping_neg();
            ss = dx;
        }
    }

    /* return no direction if arm lengths do not differ enough       */
    /* (value 14 is heuristic, corresponding to approx. 4.1 degrees) */
    /* the long arm is never negative                                */
    if ll <= 14i64.wrapping_mul(ft_abs(ss)) {
        dir = AF_DIR_NONE;
    }

    dir
}

/// `af_glyph_hints_init`
pub fn af_glyph_hints_init() -> AfGlyphHintsRec {
    /* no need to initialize the embedded items */
    AfGlyphHintsRec::default()
}

/// `af_glyph_hints_done`
pub fn af_glyph_hints_done(hints: &mut AfGlyphHintsRec) {
    /*
     * note that we don't need to free the segment and edge
     * buffers since they are really within the hints->points array
     */
    for dim in 0..AF_DIMENSION_MAX {
        let axis = &mut hints.axis[dim];

        axis.num_segments = 0;
        axis.segments = Vec::new();

        axis.num_edges = 0;
        axis.edges = Vec::new();
    }

    hints.contours = Vec::new();
    hints.num_contours = 0;

    hints.points = Vec::new();
    hints.num_points = 0;
}

/* Reset metrics. */

/// `af_glyph_hints_rescale`
pub fn af_glyph_hints_rescale(hints: &mut AfGlyphHintsRec, metrics: &AfStyleMetricsRec) {
    /* hints->metrics = metrics; */
    hints.units_per_EM = metrics.scaler.face.units_per_EM;
    hints.scaler_flags = metrics.scaler.flags;
}

/* Recompute all AF_Point in AF_GlyphHints from the definitions */
/* in a source outline.                                         */

/// `af_glyph_hints_reload`
pub fn af_glyph_hints_reload(hints: &mut AfGlyphHintsRec, outline: &FtOutline) -> FtResult<()> {
    let x_scale = hints.x_scale;
    let y_scale = hints.y_scale;
    let x_delta = hints.x_delta;
    let y_delta = hints.y_delta;

    hints.num_points = 0;
    hints.num_contours = 0;

    hints.axis[0].num_segments = 0;
    hints.axis[0].num_edges = 0;
    hints.axis[1].num_segments = 0;
    hints.axis[1].num_edges = 0;

    /* first of all, reallocate the contours array if necessary */
    let new_max = outline.n_contours.max(0) as usize;
    if hints.contours.len() < new_max {
        if hints
            .contours
            .try_reserve_exact(new_max - hints.contours.len())
            .is_err()
        {
            return Err(FT_ERR_OUT_OF_MEMORY);
        }
        hints.contours.resize(new_max, 0);
    }

    /*
     * then reallocate the points arrays if necessary --
     * note that we reserve two additional point positions, used to
     * hint metrics appropriately
     */
    let new_max = outline.n_points.max(0) as usize + 2;
    if hints.points.len() < new_max {
        if hints
            .points
            .try_reserve_exact(new_max - hints.points.len())
            .is_err()
        {
            return Err(FT_ERR_OUT_OF_MEMORY);
        }
        hints.points.resize(new_max, AfPointRec::default());
    }

    hints.num_points = outline.n_points as FtInt;
    hints.num_contours = outline.n_contours as FtInt;

    /* We can't rely on the value of `FT_Outline.flags' to know the fill   */
    /* direction used for a glyph, given that some fonts are broken (e.g., */
    /* the Arphic ones).  We thus recompute it each time we need to.       */
    /*                                                                     */
    hints.axis[AF_DIMENSION_HORZ].major_dir = AF_DIR_UP;
    hints.axis[AF_DIMENSION_VERT].major_dir = AF_DIR_LEFT;

    if ft_outline_get_orientation(outline) == FT_ORIENTATION_POSTSCRIPT {
        hints.axis[AF_DIMENSION_HORZ].major_dir = AF_DIR_DOWN;
        hints.axis[AF_DIMENSION_VERT].major_dir = AF_DIR_RIGHT;
    }

    hints.x_scale = x_scale;
    hints.y_scale = y_scale;
    hints.x_delta = x_delta;
    hints.y_delta = y_delta;

    if hints.num_points == 0 {
        return Ok(());
    }

    let point_limit = hints.num_points as usize;

    /* value 20 in `near_limit' is heuristic */
    let units_per_em = hints.units_per_EM as FtUInt;
    let near_limit: FtInt = (20 * units_per_em / 2048) as FtInt;

    let points = &mut hints.points;

    /* compute coordinates & Bezier flags, next and prev */
    {
        let mut endpoint = outline.contours[0] as usize;
        let mut end = endpoint;
        let mut prev = end;
        let mut contour_index: FtInt = 0;

        for point in 0..point_limit {
            let vec = outline.points[point];
            let tag = outline.tags[point];

            points[point].in_dir = AF_DIR_NONE as i8;
            points[point].out_dir = AF_DIR_NONE as i8;

            points[point].fx = vec.x as FtShort;
            points[point].fy = vec.y as FtShort;
            let ox = ft_mul_fix(vec.x, x_scale).wrapping_add(x_delta);
            points[point].ox = ox;
            points[point].x = ox;
            let oy = ft_mul_fix(vec.y, y_scale).wrapping_add(y_delta);
            points[point].oy = oy;
            points[point].y = oy;

            points[end].fx = outline.points[endpoint].x as FtShort;
            points[end].fy = outline.points[endpoint].y as FtShort;

            points[point].flags = match ft_curve_tag(tag) {
                FT_CURVE_TAG_CONIC => AF_FLAG_CONIC,
                FT_CURVE_TAG_CUBIC => AF_FLAG_CUBIC,
                _ => AF_FLAG_NONE,
            };

            let out_x = points[point].fx as FtPos - points[prev].fx as FtPos;
            let out_y = points[point].fy as FtPos - points[prev].fy as FtPos;

            if out_x.abs() + out_y.abs() < near_limit as FtPos {
                points[prev].flags |= AF_FLAG_NEAR;
            }

            points[point].prev = prev;
            points[prev].next = point;
            prev = point;

            if point == end {
                contour_index += 1;
                if contour_index < outline.n_contours as FtInt {
                    endpoint = outline.contours[contour_index as usize] as usize;
                    end = endpoint;
                    prev = end;
                }
            }
        }
    }

    /* set up the contours array */
    {
        let mut idx: usize = 0;

        for contour in 0..hints.num_contours as usize {
            hints.contours[contour] = idx;
            idx = (outline.contours[contour] as i32 + 1) as i16 as usize;
        }
    }

    {
        /*
         * Compute directions of `in' and `out' vectors.
         *
         * Note that distances between points that are very near to each
         * other are accumulated.  In other words, the auto-hinter either
         * prepends the small vectors between near points to the first
         * non-near vector, or the sum of small vector lengths exceeds a
         * threshold, thus `grouping' the small vectors.  All intermediate
         * points are tagged as weak; the directions are adjusted also to
         * be equal to the accumulated one.
         */

        let near_limit2: FtInt = 2 * near_limit - 1;

        for c in 0..hints.num_contours as usize {
            let mut first = hints.contours[c];

            /* since the first point of a contour could be part of a */
            /* series of near points, go backwards to find the first */
            /* non-near point and adjust `first'                     */

            let mut point = first;
            let mut prev = points[first].prev;

            while prev != first {
                let out_x = points[point].fx as FtPos - points[prev].fx as FtPos;
                let out_y = points[point].fy as FtPos - points[prev].fy as FtPos;

                /*
                 * We use Taxicab metrics to measure the vector length.
                 *
                 * Note that the accumulated distances so far could have the
                 * opposite direction of the distance measured here.  For this
                 * reason we use `near_limit2' for the comparison to get a
                 * non-near point even in the worst case.
                 */
                if out_x.abs() + out_y.abs() >= near_limit2 as FtPos {
                    break;
                }

                point = prev;
                prev = points[prev].prev;
            }

            /* adjust first point */
            first = point;

            /* now loop over all points of the contour to get */
            /* `in' and `out' vector directions               */

            let mut curr = first;

            /*
             * We abuse the `u' and `v' fields to store index deltas to the
             * next and previous non-near point, respectively.
             *
             * To avoid problems with not having non-near points, we point to
             * `first' by default as the next non-near point.
             *
             */
            points[curr].u = first as FtPos - curr as FtPos;
            points[first].v = -points[curr].u;

            let mut out_x: FtPos = 0;
            let mut out_y: FtPos = 0;

            let mut next = first;
            loop {
                let point = next;
                next = points[point].next;

                out_x += points[next].fx as FtPos - points[point].fx as FtPos;
                out_y += points[next].fy as FtPos - points[point].fy as FtPos;

                if out_x.abs() + out_y.abs() < near_limit as FtPos {
                    points[next].flags |= AF_FLAG_WEAK_INTERPOLATION;
                    if next == first {
                        break;
                    }
                    continue;
                }

                points[curr].u = next as FtPos - curr as FtPos;
                points[next].v = -points[curr].u;

                let out_dir = af_direction_compute(out_x, out_y);

                /* adjust directions for all points inbetween; */
                /* the loop also updates position of `curr'    */
                points[curr].out_dir = out_dir as i8;
                curr = points[curr].next;
                while curr != next {
                    points[curr].in_dir = out_dir as i8;
                    points[curr].out_dir = out_dir as i8;
                    curr = points[curr].next;
                }
                points[next].in_dir = out_dir as i8;

                points[curr].u = first as FtPos - curr as FtPos;
                points[first].v = -points[curr].u;

                out_x = 0;
                out_y = 0;

                if next == first {
                    break;
                }
            }
        }

        /*
         * The next step is to `simplify' an outline's topology so that we
         * can identify local extrema more reliably: A series of
         * non-horizontal or non-vertical vectors pointing into the same
         * quadrant are handled as a single, long vector.  From a
         * topological point of the view, the intermediate points are of no
         * interest and thus tagged as weak.
         */

        for point in 0..point_limit {
            if points[point].flags & AF_FLAG_WEAK_INTERPOLATION != 0 {
                continue;
            }

            if points[point].in_dir as AfDirection == AF_DIR_NONE
                && points[point].out_dir as AfDirection == AF_DIR_NONE
            {
                /* check whether both vectors point into the same quadrant */

                let next_u = (point as FtPos + points[point].u) as usize;
                let prev_v = (point as FtPos + points[point].v) as usize;

                let in_x = points[point].fx as FtPos - points[prev_v].fx as FtPos;
                let in_y = points[point].fy as FtPos - points[prev_v].fy as FtPos;

                let out_x = points[next_u].fx as FtPos - points[point].fx as FtPos;
                let out_y = points[next_u].fy as FtPos - points[point].fy as FtPos;

                if (in_x ^ out_x) >= 0 && (in_y ^ out_y) >= 0 {
                    /* yes, so tag current point as weak */
                    /* and update index deltas           */

                    points[point].flags |= AF_FLAG_WEAK_INTERPOLATION;

                    points[prev_v].u = next_u as FtPos - prev_v as FtPos;
                    points[next_u].v = -points[prev_v].u;
                }
            }
        }

        /*
         * Finally, check for remaining weak points.  Everything else not
         * collected in edges so far is then implicitly classified as strong
         * points.
         */

        for point in 0..point_limit {
            if points[point].flags & AF_FLAG_WEAK_INTERPOLATION != 0 {
                continue;
            }

            let is_weak = if points[point].flags & AF_FLAG_CONTROL != 0 {
                /* control points are always weak */
                true
            } else if points[point].out_dir == points[point].in_dir {
                if points[point].out_dir as AfDirection != AF_DIR_NONE {
                    /* current point lies on a horizontal or          */
                    /* vertical segment (but doesn't start or end it) */
                    true
                } else {
                    let next_u = (point as FtPos + points[point].u) as usize;
                    let prev_v = (point as FtPos + points[point].v) as usize;

                    if ft_corner_is_flat(
                        points[point].fx as FtPos - points[prev_v].fx as FtPos,
                        points[point].fy as FtPos - points[prev_v].fy as FtPos,
                        points[next_u].fx as FtPos - points[point].fx as FtPos,
                        points[next_u].fy as FtPos - points[point].fy as FtPos,
                    ) {
                        /* either the `in' or the `out' vector is much more  */
                        /* dominant than the other one, so tag current point */
                        /* as weak and update index deltas                   */

                        points[prev_v].u = next_u as FtPos - prev_v as FtPos;
                        points[next_u].v = -points[prev_v].u;

                        true
                    } else {
                        false
                    }
                }
            } else {
                /* current point forms a spike */
                points[point].in_dir as AfDirection == -(points[point].out_dir as AfDirection)
            };

            if is_weak {
                /* Is_Weak_Point: */
                points[point].flags |= AF_FLAG_WEAK_INTERPOLATION;
            }
        }
    }

    Ok(())
}

/* Store the hinted outline in an FT_Outline structure. */

/// `af_glyph_hints_save`
pub fn af_glyph_hints_save(hints: &AfGlyphHintsRec, outline: &mut FtOutline) {
    for p in 0..hints.num_points as usize {
        let point = &hints.points[p];

        outline.points[p].x = point.x;
        outline.points[p].y = point.y;

        outline.tags[p] = if point.flags & AF_FLAG_CONIC != 0 {
            FT_CURVE_TAG_CONIC
        } else if point.flags & AF_FLAG_CUBIC != 0 {
            FT_CURVE_TAG_CUBIC
        } else {
            FT_CURVE_TAG_ON
        };
    }
}

/****************************************************************
 *
 *                     EDGE POINT GRID-FITTING
 *
 ****************************************************************/

/* Align all points of an edge to the same coordinate value, */
/* either horizontally or vertically.                        */

/// `af_glyph_hints_align_edge_points`
pub fn af_glyph_hints_align_edge_points(hints: &mut AfGlyphHintsRec, dim: AfDimension) {
    let axis = &hints.axis[dim];
    let points = &mut hints.points;

    for seg in &axis.segments[..axis.num_segments as usize] {
        let Some(edge) = seg.edge else {
            continue;
        };
        let pos = axis.edges[edge].pos;

        let first = seg.first;
        let last = seg.last;
        let mut point = first;
        loop {
            if dim == AF_DIMENSION_HORZ {
                points[point].x = pos;
                points[point].flags |= AF_FLAG_TOUCH_X;
            } else {
                points[point].y = pos;
                points[point].flags |= AF_FLAG_TOUCH_Y;
            }

            if point == last {
                break;
            }

            point = points[point].next;
        }
    }
}

/****************************************************************
 *
 *                    STRONG POINT INTERPOLATION
 *
 ****************************************************************/

/* Hint the strong points -- this is equivalent to the TrueType `IP' */
/* hinting instruction.                                              */

/// `af_glyph_hints_align_strong_points`
pub fn af_glyph_hints_align_strong_points(hints: &mut AfGlyphHintsRec, dim: AfDimension) {
    let point_limit = hints.num_points as usize;
    let axis = &mut hints.axis[dim];
    let edges = &mut axis.edges;
    let edge_limit = axis.num_edges as usize;
    let points = &mut hints.points;

    let touch_flag = if dim == AF_DIMENSION_HORZ {
        AF_FLAG_TOUCH_X
    } else {
        AF_FLAG_TOUCH_Y
    };

    if edge_limit > 0 {
        for point in 0..point_limit {
            if points[point].flags & touch_flag != 0 {
                continue;
            }

            /* if this point is candidate to weak interpolation, we       */
            /* interpolate it after all strong points have been processed */

            if points[point].flags & AF_FLAG_WEAK_INTERPOLATION != 0 {
                continue;
            }

            let (mut u, ou): (FtPos, FtPos) = if dim == AF_DIMENSION_VERT {
                (points[point].fy as FtPos, points[point].oy)
            } else {
                (points[point].fx as FtPos, points[point].ox)
            };

            let fu = u;

            'store: {
                /* is the point before the first edge? */
                let edge = 0;
                let delta = edges[edge].fpos as FtPos - u;
                if delta >= 0 {
                    u = edges[edge].pos - (edges[edge].opos - ou);
                    break 'store;
                }

                /* is the point after the last edge? */
                let edge = edge_limit - 1;
                let delta = u - edges[edge].fpos as FtPos;
                if delta >= 0 {
                    u = edges[edge].pos + (ou - edges[edge].opos);
                    break 'store;
                }

                {
                    /* find enclosing edges */
                    let mut min: usize = 0;
                    let mut max: usize = edge_limit;

                    /* for a small number of edges, a linear search is better */
                    if max <= 8 {
                        let mut nn = 0;
                        while nn < max {
                            if edges[nn].fpos as FtPos >= u {
                                break;
                            }
                            nn += 1;
                        }

                        if edges[nn].fpos as FtPos == u {
                            u = edges[nn].pos;
                            break 'store;
                        }
                        min = nn;
                    } else {
                        while min < max {
                            let mid = (max + min) >> 1;
                            let fpos = edges[mid].fpos as FtPos;

                            if u < fpos {
                                max = mid;
                            } else if u > fpos {
                                min = mid + 1;
                            } else {
                                /* we are on the edge */
                                u = edges[mid].pos;
                                break 'store;
                            }
                        }
                    }

                    /* point is not on an edge */
                    {
                        let before = min - 1;
                        let after = min;

                        /* assert( before && after && before != after ) */
                        if edges[before].scale == 0 {
                            edges[before].scale = ft_div_fix(
                                edges[after].pos - edges[before].pos,
                                edges[after].fpos as FtPos - edges[before].fpos as FtPos,
                            );
                        }

                        u = edges[before].pos
                            + ft_mul_fix(fu - edges[before].fpos as FtPos, edges[before].scale);
                    }
                }
            }

            /* Store_Point: */
            /* save the point position */
            if dim == AF_DIMENSION_HORZ {
                points[point].x = u;
            } else {
                points[point].y = u;
            }

            points[point].flags |= touch_flag;
        }
    }
}

/****************************************************************
 *
 *                    WEAK POINT INTERPOLATION
 *
 ****************************************************************/

/* Shift the original coordinates of all points between `p1' and */
/* `p2' to get hinted coordinates, using the same difference as  */
/* given by `ref'.                                               */

/// `af_iup_shift`
fn af_iup_shift(points: &mut [AfPointRec], p1: usize, p2: usize, ref_: usize) {
    let delta = points[ref_].u - points[ref_].v;

    if delta == 0 {
        return;
    }

    for p in p1..ref_ {
        points[p].u = points[p].v + delta;
    }

    for p in ref_ + 1..=p2 {
        points[p].u = points[p].v + delta;
    }
}

/* Interpolate the original coordinates of all points between `p1' and  */
/* `p2' to get hinted coordinates, using `ref1' and `ref2' as the       */
/* reference points.  The `u' and `v' members are the current and       */
/* original coordinate values, respectively.                            */
/*                                                                      */
/* Details can be found in the TrueType bytecode specification.         */

/// `af_iup_interp`
fn af_iup_interp(
    points: &mut [AfPointRec],
    p1: usize,
    p2: usize,
    mut ref1: usize,
    mut ref2: usize,
) {
    if p1 > p2 {
        return;
    }

    if points[ref1].v > points[ref2].v {
        std::mem::swap(&mut ref1, &mut ref2);
    }

    let v1 = points[ref1].v;
    let v2 = points[ref2].v;
    let u1 = points[ref1].u;
    let u2 = points[ref2].u;

    let d1 = u1 - v1;
    let d2 = u2 - v2;

    if u1 == u2 || v1 == v2 {
        for p in p1..=p2 {
            let mut u = points[p].v;

            if u <= v1 {
                u += d1;
            } else if u >= v2 {
                u += d2;
            } else {
                u = u1;
            }

            points[p].u = u;
        }
    } else {
        let scale = ft_div_fix(u2 - u1, v2 - v1);

        for p in p1..=p2 {
            let mut u = points[p].v;

            if u <= v1 {
                u += d1;
            } else if u >= v2 {
                u += d2;
            } else {
                u = u1 + ft_mul_fix(u - v1, scale);
            }

            points[p].u = u;
        }
    }
}

/* Hint the weak points -- this is equivalent to the TrueType `IUP' */
/* hinting instruction.                                             */

/// `af_glyph_hints_align_weak_points`
pub fn af_glyph_hints_align_weak_points(hints: &mut AfGlyphHintsRec, dim: AfDimension) {
    let point_limit = hints.num_points as usize;
    let points = &mut hints.points;
    let touch_flag;

    /* PASS 1: Move segment points to edge positions */

    if dim == AF_DIMENSION_HORZ {
        touch_flag = AF_FLAG_TOUCH_X;

        for point in &mut points[..point_limit] {
            point.u = point.x;
            point.v = point.ox;
        }
    } else {
        touch_flag = AF_FLAG_TOUCH_Y;

        for point in &mut points[..point_limit] {
            point.u = point.y;
            point.v = point.oy;
        }
    }

    for c in 0..hints.num_contours as usize {
        let mut point = hints.contours[c];
        let end_point = points[point].prev;
        let first_point = point;

        /* find first touched point */
        loop {
            if point > end_point {
                /* no touched point in contour */
                break;
            }

            if points[point].flags & touch_flag != 0 {
                break;
            }

            point += 1;
        }
        if point > end_point {
            /* NextContour: */
            continue;
        }

        let first_touched = point;
        let mut last_touched;

        loop {
            /* FT_ASSERT( point <= end_point                 && */
            /*            ( point->flags & touch_flag ) != 0 ); */

            /* skip any touched neighbours */
            while point < end_point && (points[point + 1].flags & touch_flag) != 0 {
                point += 1;
            }

            last_touched = point;

            /* find the next touched point, if any */
            point += 1;
            loop {
                if point > end_point {
                    break;
                }

                if (points[point].flags & touch_flag) != 0 {
                    break;
                }

                point += 1;
            }
            if point > end_point {
                break; /* EndContour */
            }

            /* interpolate between last_touched and point */
            af_iup_interp(points, last_touched + 1, point - 1, last_touched, point);
        }

        /* EndContour: */
        /* special case: only one point was touched */
        if last_touched == first_touched {
            af_iup_shift(points, first_point, end_point, first_touched);
        } else
        /* interpolate the last part */
        {
            if last_touched < end_point {
                af_iup_interp(
                    points,
                    last_touched + 1,
                    end_point,
                    last_touched,
                    first_touched,
                );
            }

            if first_touched > 0 {
                af_iup_interp(
                    points,
                    first_point,
                    first_touched.wrapping_sub(1),
                    last_touched,
                    first_touched,
                );
            }
        }
    }

    /* now save the interpolated values back to x/y */
    if dim == AF_DIMENSION_HORZ {
        for point in &mut points[..point_limit] {
            point.x = point.u;
        }
    } else {
        for point in &mut points[..point_limit] {
            point.y = point.u;
        }
    }
}
