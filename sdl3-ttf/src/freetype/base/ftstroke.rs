// Rust translation of src/base/ftstroke.c (and include/freetype/ftstroke.h)
// from FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2002-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType path stroker (body).
//!
//! Translation note: coordinate sums wrap (as they do in C on the
//! targets FreeType supports) instead of trapping in debug builds, since
//! hinted outlines of broken fonts can hold arbitrary coordinates.

use super::super::fttypes::*;
use super::ftcalc::{ft_div_fix, ft_mul_div, ft_mul_fix};
use super::ftglyph::{ft_done_glyph, ft_glyph_copy, FtGlyph};
use super::ftmemory::ft_renew_array;
use super::ftobjs::FtLibrary;
use super::ftoutln::{ft_outline_get_orientation, ft_outline_new, FT_ORIENTATION_TRUETYPE};
use super::fttrigon::*;

/* ftstroke.h */

/// `FT_Stroker_LineJoin`
pub type FtStrokerLineJoin = u32;
pub const FT_STROKER_LINEJOIN_ROUND: FtStrokerLineJoin = 0;
pub const FT_STROKER_LINEJOIN_BEVEL: FtStrokerLineJoin = 1;
pub const FT_STROKER_LINEJOIN_MITER_VARIABLE: FtStrokerLineJoin = 2;
pub const FT_STROKER_LINEJOIN_MITER: FtStrokerLineJoin = FT_STROKER_LINEJOIN_MITER_VARIABLE;
pub const FT_STROKER_LINEJOIN_MITER_FIXED: FtStrokerLineJoin = 3;

/// `FT_Stroker_LineCap`
pub type FtStrokerLineCap = u32;
pub const FT_STROKER_LINECAP_BUTT: FtStrokerLineCap = 0;
pub const FT_STROKER_LINECAP_ROUND: FtStrokerLineCap = 1;
pub const FT_STROKER_LINECAP_SQUARE: FtStrokerLineCap = 2;

/// `FT_StrokerBorder`
pub type FtStrokerBorder = u32;
pub const FT_STROKER_BORDER_LEFT: FtStrokerBorder = 0;
pub const FT_STROKER_BORDER_RIGHT: FtStrokerBorder = 1;

/// `FT_Outline_GetInsideBorder`
pub fn ft_outline_get_inside_border(outline: &FtOutline) -> FtStrokerBorder {
    let o = ft_outline_get_orientation(outline);

    if o == FT_ORIENTATION_TRUETYPE {
        FT_STROKER_BORDER_RIGHT
    } else {
        FT_STROKER_BORDER_LEFT
    }
}

/// `FT_Outline_GetOutsideBorder`
pub fn ft_outline_get_outside_border(outline: &FtOutline) -> FtStrokerBorder {
    let o = ft_outline_get_orientation(outline);

    if o == FT_ORIENTATION_TRUETYPE {
        FT_STROKER_BORDER_LEFT
    } else {
        FT_STROKER_BORDER_RIGHT
    }
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                      BEZIER COMPUTATIONS                      *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

const FT_SMALL_CONIC_THRESHOLD: FtAngle = FT_ANGLE_PI / 6;
const FT_SMALL_CUBIC_THRESHOLD: FtAngle = FT_ANGLE_PI / 8;
const FT_EPSILON: FtPos = 2;

/// `FT_IS_SMALL`
#[inline]
fn ft_is_small(x: FtPos) -> bool {
    x > -FT_EPSILON && x < FT_EPSILON
}

/// `ft_pos_abs`
#[inline]
fn ft_pos_abs(x: FtPos) -> FtPos {
    if x >= 0 {
        x
    } else {
        x.wrapping_neg()
    }
}

/// The difference of two vectors (wrapping).
#[inline]
fn vsub(a: FtVector, b: FtVector) -> FtVector {
    FtVector {
        x: a.x.wrapping_sub(b.x),
        y: a.y.wrapping_sub(b.y),
    }
}

/// The sum of two vectors (wrapping).
#[inline]
fn vadd(a: FtVector, b: FtVector) -> FtVector {
    FtVector {
        x: a.x.wrapping_add(b.x),
        y: a.y.wrapping_add(b.y),
    }
}

/// `FT_Vector_From_Polar` returning the vector
#[inline]
fn from_polar(length: FtFixed, angle: FtAngle) -> FtVector {
    let mut v = FtVector::default();
    ft_vector_from_polar(&mut v, length, angle);
    v
}

/// `ft_conic_split`
fn ft_conic_split(base: &mut [FtVector]) {
    base[4].x = base[2].x;
    let a = base[0].x.wrapping_add(base[1].x);
    let b = base[1].x.wrapping_add(base[2].x);
    base[3].x = b >> 1;
    base[2].x = a.wrapping_add(b) >> 2;
    base[1].x = a >> 1;

    base[4].y = base[2].y;
    let a = base[0].y.wrapping_add(base[1].y);
    let b = base[1].y.wrapping_add(base[2].y);
    base[3].y = b >> 1;
    base[2].y = a.wrapping_add(b) >> 2;
    base[1].y = a >> 1;
}

/// `ft_conic_is_small_enough`
fn ft_conic_is_small_enough(
    base: &[FtVector],
    angle_in: &mut FtAngle,
    angle_out: &mut FtAngle,
) -> bool {
    let d1 = vsub(base[1], base[2]);
    let d2 = vsub(base[0], base[1]);

    let close1 = ft_is_small(d1.x) && ft_is_small(d1.y);
    let close2 = ft_is_small(d2.x) && ft_is_small(d2.y);

    if close1 {
        if close2 {
            /* basically a point;                      */
            /* do nothing to retain original direction */
        } else {
            *angle_in = ft_atan2(d2.x, d2.y);
            *angle_out = *angle_in;
        }
    } else {
        /* !close1 */
        if close2 {
            *angle_in = ft_atan2(d1.x, d1.y);
            *angle_out = *angle_in;
        } else {
            *angle_in = ft_atan2(d1.x, d1.y);
            *angle_out = ft_atan2(d2.x, d2.y);
        }
    }

    let theta = ft_pos_abs(ft_angle_diff(*angle_in, *angle_out));

    theta < FT_SMALL_CONIC_THRESHOLD
}

/// `ft_cubic_split`
fn ft_cubic_split(base: &mut [FtVector]) {
    base[6].x = base[3].x;
    let mut a = base[0].x.wrapping_add(base[1].x);
    let b = base[1].x.wrapping_add(base[2].x);
    let mut c = base[2].x.wrapping_add(base[3].x);
    base[5].x = c >> 1;
    c = c.wrapping_add(b);
    base[4].x = c >> 2;
    base[1].x = a >> 1;
    a = a.wrapping_add(b);
    base[2].x = a >> 2;
    base[3].x = a.wrapping_add(c) >> 3;

    base[6].y = base[3].y;
    let mut a = base[0].y.wrapping_add(base[1].y);
    let b = base[1].y.wrapping_add(base[2].y);
    let mut c = base[2].y.wrapping_add(base[3].y);
    base[5].y = c >> 1;
    c = c.wrapping_add(b);
    base[4].y = c >> 2;
    base[1].y = a >> 1;
    a = a.wrapping_add(b);
    base[2].y = a >> 2;
    base[3].y = a.wrapping_add(c) >> 3;
}

/// `ft_angle_mean`: Return the average of `angle1' and `angle2'.  This
/// gives correct result even if `angle1' and `angle2' have opposite signs.
fn ft_angle_mean(angle1: FtAngle, angle2: FtAngle) -> FtAngle {
    angle1 + ft_angle_diff(angle1, angle2) / 2
}

/// `ft_cubic_is_small_enough`
fn ft_cubic_is_small_enough(
    base: &[FtVector],
    angle_in: &mut FtAngle,
    angle_mid: &mut FtAngle,
    angle_out: &mut FtAngle,
) -> bool {
    let d1 = vsub(base[2], base[3]);
    let d2 = vsub(base[1], base[2]);
    let d3 = vsub(base[0], base[1]);

    let close1 = ft_is_small(d1.x) && ft_is_small(d1.y);
    let close2 = ft_is_small(d2.x) && ft_is_small(d2.y);
    let close3 = ft_is_small(d3.x) && ft_is_small(d3.y);

    if close1 {
        if close2 {
            if close3 {
                /* basically a point;                      */
                /* do nothing to retain original direction */
            } else {
                /* !close3 */
                *angle_in = ft_atan2(d3.x, d3.y);
                *angle_mid = *angle_in;
                *angle_out = *angle_in;
            }
        } else {
            /* !close2 */
            if close3 {
                *angle_in = ft_atan2(d2.x, d2.y);
                *angle_mid = *angle_in;
                *angle_out = *angle_in;
            } else {
                /* !close3 */
                *angle_in = ft_atan2(d2.x, d2.y);
                *angle_mid = *angle_in;
                *angle_out = ft_atan2(d3.x, d3.y);
            }
        }
    } else {
        /* !close1 */
        if close2 {
            if close3 {
                *angle_in = ft_atan2(d1.x, d1.y);
                *angle_mid = *angle_in;
                *angle_out = *angle_in;
            } else {
                /* !close3 */
                *angle_in = ft_atan2(d1.x, d1.y);
                *angle_out = ft_atan2(d3.x, d3.y);
                *angle_mid = ft_angle_mean(*angle_in, *angle_out);
            }
        } else {
            /* !close2 */
            if close3 {
                *angle_in = ft_atan2(d1.x, d1.y);
                *angle_mid = ft_atan2(d2.x, d2.y);
                *angle_out = *angle_mid;
            } else {
                /* !close3 */
                *angle_in = ft_atan2(d1.x, d1.y);
                *angle_mid = ft_atan2(d2.x, d2.y);
                *angle_out = ft_atan2(d3.x, d3.y);
            }
        }
    }

    let theta1 = ft_pos_abs(ft_angle_diff(*angle_in, *angle_mid));
    let theta2 = ft_pos_abs(ft_angle_diff(*angle_mid, *angle_out));

    theta1 < FT_SMALL_CUBIC_THRESHOLD && theta2 < FT_SMALL_CUBIC_THRESHOLD
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                       STROKE BORDERS                          *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/* FT_StrokeTags */
const FT_STROKE_TAG_ON: u8 = 1; /* on-curve point  */
const FT_STROKE_TAG_CUBIC: u8 = 2; /* cubic off-point */
const FT_STROKE_TAG_BEGIN: u8 = 4; /* sub-path start  */
const FT_STROKE_TAG_END: u8 = 8; /* sub-path end    */

const FT_STROKE_TAG_BEGIN_END: u8 = FT_STROKE_TAG_BEGIN | FT_STROKE_TAG_END;

/// `FT_StrokeBorderRec`
#[derive(Debug, Clone, Default)]
struct FtStrokeBorderRec {
    num_points: FtUInt,
    max_points: FtUInt,
    points: Vec<FtVector>,
    tags: Vec<FtByte>,
    movable: bool, /* TRUE for ends of lineto borders */
    start: FtInt,  /* index of current sub-path start point */
    valid: bool,
}

/// `ft_stroke_border_grow`
fn ft_stroke_border_grow(border: &mut FtStrokeBorderRec, new_points: FtUInt) -> FtResult<()> {
    let old_max = border.max_points;
    let new_max = border.num_points.wrapping_add(new_points);

    if new_max > old_max {
        let mut cur_max = old_max;

        while cur_max < new_max {
            cur_max = cur_max.wrapping_add((cur_max >> 1) + 16);
        }

        ft_renew_array(&mut border.points, cur_max as FtLong)?;
        ft_renew_array(&mut border.tags, cur_max as FtLong)?;

        border.max_points = cur_max;
    }

    Ok(())
}

/// `ft_stroke_border_close`
fn ft_stroke_border_close(border: &mut FtStrokeBorderRec, reverse: bool) {
    let start = border.start as FtUInt;
    let mut count = border.num_points;

    /* don't record empty paths! */
    if count <= start + 1 {
        border.num_points = start;
    } else {
        /* copy the last point to the start of this sub-path, since */
        /* it contains the `adjusted' starting coordinates          */
        count -= 1;
        border.num_points = count;
        border.points[start as usize] = border.points[count as usize];
        border.tags[start as usize] = border.tags[count as usize];

        if reverse {
            /* reverse the points */
            border.points[start as usize + 1..count as usize].reverse();

            /* then the tags */
            border.tags[start as usize + 1..count as usize].reverse();
        }

        border.tags[start as usize] |= FT_STROKE_TAG_BEGIN;
        border.tags[count as usize - 1] |= FT_STROKE_TAG_END;
    }

    border.start = -1;
    border.movable = false;
}

/// `ft_stroke_border_lineto`
fn ft_stroke_border_lineto(
    border: &mut FtStrokeBorderRec,
    to: &FtVector,
    movable: bool,
) -> FtResult<()> {
    if border.movable {
        /* move last point */
        border.points[border.num_points as usize - 1] = *to;
    } else {
        /* don't add zero-length lineto, but always add moveto */
        if border.num_points > border.start as FtUInt
            && ft_is_small(
                border.points[border.num_points as usize - 1]
                    .x
                    .wrapping_sub(to.x),
            )
            && ft_is_small(
                border.points[border.num_points as usize - 1]
                    .y
                    .wrapping_sub(to.y),
            )
        {
            return Ok(());
        }

        /* add one point */
        let error = ft_stroke_border_grow(border, 1);
        if error.is_ok() {
            let n = border.num_points as usize;
            border.points[n] = *to;
            border.tags[n] = FT_STROKE_TAG_ON;

            border.num_points += 1;
        } else {
            border.movable = movable;
            return error;
        }
    }

    border.movable = movable;
    Ok(())
}

/// `ft_stroke_border_conicto`
fn ft_stroke_border_conicto(
    border: &mut FtStrokeBorderRec,
    control: &FtVector,
    to: &FtVector,
) -> FtResult<()> {
    let error = ft_stroke_border_grow(border, 2);
    if error.is_ok() {
        let n = border.num_points as usize;

        border.points[n] = *control;
        border.points[n + 1] = *to;

        border.tags[n] = 0;
        border.tags[n + 1] = FT_STROKE_TAG_ON;

        border.num_points += 2;
    }

    border.movable = false;

    error
}

/// `ft_stroke_border_cubicto`
fn ft_stroke_border_cubicto(
    border: &mut FtStrokeBorderRec,
    control1: &FtVector,
    control2: &FtVector,
    to: &FtVector,
) -> FtResult<()> {
    let error = ft_stroke_border_grow(border, 3);
    if error.is_ok() {
        let n = border.num_points as usize;

        border.points[n] = *control1;
        border.points[n + 1] = *control2;
        border.points[n + 2] = *to;

        border.tags[n] = FT_STROKE_TAG_CUBIC;
        border.tags[n + 1] = FT_STROKE_TAG_CUBIC;
        border.tags[n + 2] = FT_STROKE_TAG_ON;

        border.num_points += 3;
    }

    border.movable = false;

    error
}

const FT_ARC_CUBIC_ANGLE: FtAngle = FT_ANGLE_PI / 2;

/// `ft_stroke_border_arcto`
fn ft_stroke_border_arcto(
    border: &mut FtStrokeBorderRec,
    center: &FtVector,
    radius: FtFixed,
    angle_start: FtAngle,
    angle_diff: FtAngle,
) -> FtResult<()> {
    let mut arcs: FtInt = 1;

    /* number of cubic arcs to draw */
    while angle_diff > FT_ARC_CUBIC_ANGLE * arcs as FtAngle
        || -angle_diff > FT_ARC_CUBIC_ANGLE * arcs as FtAngle
    {
        arcs += 1;
    }

    /* control tangents */
    let mut coef = ft_tan(angle_diff / (4 * arcs as FtAngle));
    coef += coef / 3;

    /* compute start and first control point */
    let mut a0 = from_polar(radius, angle_start);
    let mut a1 = FtVector {
        x: ft_mul_fix(-a0.y, coef),
        y: ft_mul_fix(a0.x, coef),
    };

    a0 = vadd(a0, *center);
    a1 = vadd(a1, a0);

    let mut error = Ok(());
    for i in 1..=arcs {
        /* compute end and second control point */
        let mut a3 = from_polar(
            radius,
            angle_start + i as FtAngle * angle_diff / arcs as FtAngle,
        );
        let mut a2 = FtVector {
            x: ft_mul_fix(a3.y, coef),
            y: ft_mul_fix(-a3.x, coef),
        };

        a3 = vadd(a3, *center);
        a2 = vadd(a2, a3);

        /* add cubic arc */
        error = ft_stroke_border_cubicto(border, &a1, &a2, &a3);
        if error.is_err() {
            break;
        }

        /* a0 = a3; */
        a1.x = a3.x.wrapping_sub(a2.x).wrapping_add(a3.x);
        a1.y = a3.y.wrapping_sub(a2.y).wrapping_add(a3.y);
    }

    error
}

/// `ft_stroke_border_moveto`
fn ft_stroke_border_moveto(border: &mut FtStrokeBorderRec, to: &FtVector) -> FtResult<()> {
    /* close current open path if any ? */
    if border.start >= 0 {
        ft_stroke_border_close(border, false);
    }

    border.start = border.num_points as FtInt;
    border.movable = false;

    ft_stroke_border_lineto(border, to, false)
}

/// `ft_stroke_border_init`
fn ft_stroke_border_init(border: &mut FtStrokeBorderRec) {
    border.points = Vec::new();
    border.tags = Vec::new();

    border.num_points = 0;
    border.max_points = 0;
    border.start = -1;
    border.valid = false;
}

/// `ft_stroke_border_reset`
fn ft_stroke_border_reset(border: &mut FtStrokeBorderRec) {
    border.num_points = 0;
    border.start = -1;
    border.valid = false;
}

/// `ft_stroke_border_done`
fn ft_stroke_border_done(border: &mut FtStrokeBorderRec) {
    border.points = Vec::new();
    border.tags = Vec::new();

    border.num_points = 0;
    border.max_points = 0;
    border.start = -1;
    border.valid = false;
}

/// `ft_stroke_border_get_counts`
fn ft_stroke_border_get_counts(border: &mut FtStrokeBorderRec) -> (FtUInt, FtUInt) {
    let mut num_points: FtUInt = 0;
    let mut num_contours: FtUInt = 0;

    let mut in_contour = 0;

    for &tag in &border.tags[..border.num_points as usize] {
        if tag & FT_STROKE_TAG_BEGIN != 0 {
            if in_contour != 0 {
                /* Fail: */
                return (0, 0);
            }

            in_contour = 1;
        } else if in_contour == 0 {
            /* Fail: */
            return (0, 0);
        }

        if tag & FT_STROKE_TAG_END != 0 {
            in_contour = 0;
            num_contours += 1;
        }

        num_points += 1;
    }

    if in_contour != 0 {
        /* Fail: */
        return (0, 0);
    }

    border.valid = true;

    /* Exit: */
    (num_points, num_contours)
}

/// `ft_stroke_border_export`
fn ft_stroke_border_export(border: &FtStrokeBorderRec, outline: &mut FtOutline) {
    let n = border.num_points as usize;
    let base = outline.n_points as usize;

    /* copy point locations */
    if n != 0 {
        outline.points[base..base + n].copy_from_slice(&border.points[..n]);
    }

    /* copy tags */
    for (write, &read) in outline.tags[base..base + n]
        .iter_mut()
        .zip(&border.tags[..n])
    {
        if read & FT_STROKE_TAG_ON != 0 {
            *write = FT_CURVE_TAG_ON;
        } else if read & FT_STROKE_TAG_CUBIC != 0 {
            *write = FT_CURVE_TAG_CUBIC;
        } else {
            *write = FT_CURVE_TAG_CONIC;
        }
    }

    /* copy contours */
    {
        let mut idx = outline.n_points;

        for &tag in &border.tags[..n] {
            if tag & FT_STROKE_TAG_END != 0 {
                outline.contours[outline.n_contours as usize] = idx;
                outline.n_contours += 1;
            }
            idx = idx.wrapping_add(1);
        }
    }

    outline.n_points = outline.n_points.wrapping_add(border.num_points as i16);
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                           STROKER                             *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `FT_SIDE_TO_ROTATE`
#[inline]
fn ft_side_to_rotate(s: usize) -> FtAngle {
    FT_ANGLE_PI2 - s as FtAngle * FT_ANGLE_PI
}

/// `FT_StrokerRec`
#[derive(Debug, Clone, Default)]
pub struct FtStrokerRec {
    angle_in: FtAngle,            /* direction into curr join */
    angle_out: FtAngle,           /* direction out of join  */
    center: FtVector,             /* current position */
    line_length: FtFixed,         /* length of last lineto */
    first_point: bool,            /* is this the start? */
    subpath_open: bool,           /* is the subpath open? */
    subpath_angle: FtAngle,       /* subpath start direction */
    subpath_start: FtVector,      /* subpath start position */
    subpath_line_length: FtFixed, /* subpath start lineto len */
    handle_wide_strokes: bool,    /* use wide strokes logic? */

    line_cap: FtStrokerLineCap,
    line_join: FtStrokerLineJoin,
    line_join_saved: FtStrokerLineJoin,
    miter_limit: FtFixed,
    radius: FtFixed,

    borders: [FtStrokeBorderRec; 2],
}

/// `FT_Stroker`
pub type FtStroker = Box<FtStrokerRec>;

/// `FT_Stroker_New`
pub fn ft_stroker_new(library: &FtLibrary) -> FtResult<FtStroker> {
    let _ = library;

    let mut stroker = Box::<FtStrokerRec>::default();

    ft_stroke_border_init(&mut stroker.borders[0]);
    ft_stroke_border_init(&mut stroker.borders[1]);

    Ok(stroker)
}

/// `FT_Stroker_Set`
pub fn ft_stroker_set(
    stroker: &mut FtStrokerRec,
    radius: FtFixed,
    line_cap: FtStrokerLineCap,
    line_join: FtStrokerLineJoin,
    miter_limit: FtFixed,
) {
    stroker.radius = radius;
    stroker.line_cap = line_cap;
    stroker.line_join = line_join;
    stroker.miter_limit = miter_limit;

    /* ensure miter limit has sensible value */
    if stroker.miter_limit < 0x10000 {
        stroker.miter_limit = 0x10000;
    }

    /* save line join style:                                           */
    /* line join style can be temporarily changed when stroking curves */
    stroker.line_join_saved = line_join;

    ft_stroker_rewind(stroker);
}

/// `FT_Stroker_Rewind`
pub fn ft_stroker_rewind(stroker: &mut FtStrokerRec) {
    ft_stroke_border_reset(&mut stroker.borders[0]);
    ft_stroke_border_reset(&mut stroker.borders[1]);
}

/// `FT_Stroker_Done`
pub fn ft_stroker_done(mut stroker: FtStroker) {
    ft_stroke_border_done(&mut stroker.borders[0]);
    ft_stroke_border_done(&mut stroker.borders[1]);
}

/// `ft_stroker_arcto`: create a circular arc at a corner or cap
fn ft_stroker_arcto(stroker: &mut FtStrokerRec, side: usize) -> FtResult<()> {
    let radius = stroker.radius;

    let rotate = ft_side_to_rotate(side);

    let mut total = ft_angle_diff(stroker.angle_in, stroker.angle_out);
    if total == FT_ANGLE_PI {
        total = -rotate * 2;
    }

    let center = stroker.center;
    let angle_in = stroker.angle_in;
    let border = &mut stroker.borders[side];
    let error = ft_stroke_border_arcto(border, &center, radius, angle_in + rotate, total);
    border.movable = false;
    error
}

/// `ft_stroker_cap`: add a cap at the end of an opened path
fn ft_stroker_cap(stroker: &mut FtStrokerRec, angle: FtAngle, side: usize) -> FtResult<()> {
    if stroker.line_cap == FT_STROKER_LINECAP_ROUND {
        /* add a round cap */
        stroker.angle_in = angle;
        stroker.angle_out = angle + FT_ANGLE_PI;

        ft_stroker_arcto(stroker, side)
    } else {
        /* add a square or butt cap */
        let radius = stroker.radius;

        /* compute middle point and first angle point */
        let mut middle = from_polar(radius, angle);
        let mut delta = FtVector {
            x: if side != 0 {
                middle.y
            } else {
                middle.y.wrapping_neg()
            },
            y: if side != 0 {
                middle.x.wrapping_neg()
            } else {
                middle.x
            },
        };

        if stroker.line_cap == FT_STROKER_LINECAP_SQUARE {
            middle = vadd(middle, stroker.center);
        } else {
            /* FT_STROKER_LINECAP_BUTT */
            middle = stroker.center;
        }

        delta = vadd(delta, middle);

        let border = &mut stroker.borders[side];
        ft_stroke_border_lineto(border, &delta, false)?;

        /* compute second angle point */
        delta.x = middle.x.wrapping_sub(delta.x).wrapping_add(middle.x);
        delta.y = middle.y.wrapping_sub(delta.y).wrapping_add(middle.y);

        ft_stroke_border_lineto(border, &delta, false)
    }
}

/// `ft_stroker_inside`: process an inside corner, i.e. compute
/// intersection
fn ft_stroker_inside(
    stroker: &mut FtStrokerRec,
    side: usize,
    line_length: FtFixed,
) -> FtResult<()> {
    let mut sigma = FtVector { x: 0, y: 0 };
    let delta;

    let rotate = ft_side_to_rotate(side);

    let theta = ft_angle_diff(stroker.angle_in, stroker.angle_out) / 2;

    /* Only intersect borders if between two lineto's and both */
    /* lines are long enough (line_length is zero for curves). */
    /* Also avoid U-turns of nearly 180 degree.                */
    let intersect = if !stroker.borders[side].movable
        || line_length == 0
        || theta > 0x59C000
        || theta < -0x59C000
    {
        false
    } else {
        /* compute minimum required length of lines */
        ft_vector_unit(&mut sigma, theta);
        let min_length = ft_pos_abs(ft_mul_div(stroker.radius, sigma.y, sigma.x));

        min_length != 0 && stroker.line_length >= min_length && line_length >= min_length
    };

    if !intersect {
        delta = vadd(
            from_polar(stroker.radius, stroker.angle_out + rotate),
            stroker.center,
        );

        stroker.borders[side].movable = false;
    } else {
        /* compute median angle */
        let phi = stroker.angle_in + theta + rotate;

        let length = ft_div_fix(stroker.radius, sigma.x);

        delta = vadd(from_polar(length, phi), stroker.center);
    }

    ft_stroke_border_lineto(&mut stroker.borders[side], &delta, false)
}

/// `ft_stroker_outside`: process an outside corner, i.e. compute
/// bevel/miter/round
fn ft_stroker_outside(
    stroker: &mut FtStrokerRec,
    side: usize,
    line_length: FtFixed,
) -> FtResult<()> {
    if stroker.line_join == FT_STROKER_LINEJOIN_ROUND {
        return ft_stroker_arcto(stroker, side);
    }

    /* this is a mitered (pointed) or beveled (truncated) corner */
    let radius = stroker.radius;
    let mut sigma = FtVector { x: 0, y: 0 };
    let mut theta: FtAngle = 0;
    let mut phi: FtAngle = 0;

    let rotate = ft_side_to_rotate(side);

    let mut bevel = stroker.line_join == FT_STROKER_LINEJOIN_BEVEL;
    let fixed_bevel = stroker.line_join != FT_STROKER_LINEJOIN_MITER_VARIABLE;

    /* check miter limit first */
    if !bevel {
        theta = ft_angle_diff(stroker.angle_in, stroker.angle_out) / 2;

        if theta == FT_ANGLE_PI2 {
            theta = -rotate;
        }

        phi = stroker.angle_in + theta + rotate;

        sigma = from_polar(stroker.miter_limit, theta);

        /* is miter limit exceeded? */
        if sigma.x < 0x10000 {
            /* don't create variable bevels for very small deviations; */
            /* FT_Sin(x) = 0 for x <= 57                               */
            if fixed_bevel || ft_pos_abs(theta) > 57 {
                bevel = true;
            }
        }
    }

    let center = stroker.center;
    let angle_out = stroker.angle_out;
    let miter_limit = stroker.miter_limit;
    let border = &mut stroker.borders[side];

    if bevel {
        /* this is a bevel (broken angle) */
        if fixed_bevel {
            /* the outer corners are simply joined together */

            /* add bevel */
            let delta = vadd(from_polar(radius, angle_out + rotate), center);

            border.movable = false;
            ft_stroke_border_lineto(border, &delta, false)
        } else {
            /* variable bevel or clipped miter */

            /* the miter is truncated */

            /* compute middle point and first angle point */
            let mut middle = from_polar(ft_mul_fix(radius, miter_limit), phi);

            let coef = ft_div_fix(0x10000 - sigma.x, sigma.y);
            let mut delta = FtVector {
                x: ft_mul_fix(middle.y, coef),
                y: ft_mul_fix(-middle.x, coef),
            };

            middle = vadd(middle, center);
            delta = vadd(delta, middle);

            ft_stroke_border_lineto(border, &delta, false)?;

            /* compute second angle point */
            delta.x = middle.x.wrapping_sub(delta.x).wrapping_add(middle.x);
            delta.y = middle.y.wrapping_sub(delta.y).wrapping_add(middle.y);

            ft_stroke_border_lineto(border, &delta, false)?;

            /* finally, add an end point; only needed if not lineto */
            /* (line_length is zero for curves)                     */
            if line_length == 0 {
                let delta = vadd(from_polar(radius, angle_out + rotate), center);

                ft_stroke_border_lineto(border, &delta, false)
            } else {
                Ok(())
            }
        }
    } else {
        /* this is a miter (intersection) */
        let length = ft_mul_div(radius, miter_limit, sigma.x);

        let delta = vadd(from_polar(length, phi), center);

        ft_stroke_border_lineto(border, &delta, false)?;

        /* now add an end point; only needed if not lineto */
        /* (line_length is zero for curves)                */
        if line_length == 0 {
            let delta = vadd(from_polar(radius, angle_out + rotate), center);

            ft_stroke_border_lineto(border, &delta, false)
        } else {
            Ok(())
        }
    }
}

/// `ft_stroker_process_corner`
fn ft_stroker_process_corner(stroker: &mut FtStrokerRec, line_length: FtFixed) -> FtResult<()> {
    let turn = ft_angle_diff(stroker.angle_in, stroker.angle_out);

    /* no specific corner processing is required if the turn is 0 */
    if turn == 0 {
        return Ok(());
    }

    /* when we turn to the right, the inside side is 0 */
    /* otherwise, the inside side is 1 */
    let inside_side = (turn < 0) as usize;

    /* process the inside side */
    ft_stroker_inside(stroker, inside_side, line_length)?;

    /* process the outside side */
    ft_stroker_outside(stroker, 1 - inside_side, line_length)
}

/// `ft_stroker_subpath_start`: add two points to the left and right
/// borders corresponding to the start of the subpath
fn ft_stroker_subpath_start(
    stroker: &mut FtStrokerRec,
    start_angle: FtAngle,
    line_length: FtFixed,
) -> FtResult<()> {
    let delta = from_polar(stroker.radius, start_angle + FT_ANGLE_PI2);

    let point = vadd(stroker.center, delta);

    ft_stroke_border_moveto(&mut stroker.borders[0], &point)?;

    let point = vsub(stroker.center, delta);

    let error = ft_stroke_border_moveto(&mut stroker.borders[1], &point);

    /* save angle, position, and line length for last join */
    /* (line_length is zero for curves)                    */
    stroker.subpath_angle = start_angle;
    stroker.first_point = false;
    stroker.subpath_line_length = line_length;

    error
}

/// `FT_Stroker_LineTo`
pub fn ft_stroker_line_to(stroker: &mut FtStrokerRec, to: &FtVector) -> FtResult<()> {
    let mut delta = vsub(*to, stroker.center);

    /* a zero-length lineto is a no-op; avoid creating a spurious corner */
    if delta.x == 0 && delta.y == 0 {
        return Ok(());
    }

    /* compute length of line */
    let line_length = ft_vector_length(&delta);

    let angle = ft_atan2(delta.x, delta.y);
    delta = from_polar(stroker.radius, angle + FT_ANGLE_PI2);

    /* process corner if necessary */
    if stroker.first_point {
        /* This is the first segment of a subpath.  We need to     */
        /* add a point to each border at their respective starting */
        /* point locations.                                        */
        ft_stroker_subpath_start(stroker, angle, line_length)?;
    } else {
        /* process the current corner */
        stroker.angle_out = angle;
        ft_stroker_process_corner(stroker, line_length)?;
    }

    /* now add a line segment to both the `inside' and `outside' paths */
    for border in stroker.borders.iter_mut() {
        let point = vadd(*to, delta);

        /* the ends of lineto borders are movable */
        ft_stroke_border_lineto(border, &point, true)?;

        delta.x = delta.x.wrapping_neg();
        delta.y = delta.y.wrapping_neg();
    }

    stroker.angle_in = angle;
    stroker.center = *to;
    stroker.line_length = line_length;

    Ok(())
}

/// `FT_Stroker_ConicTo`
pub fn ft_stroker_conic_to(
    stroker: &mut FtStrokerRec,
    control: &FtVector,
    to: &FtVector,
) -> FtResult<()> {
    let mut bez_stack = [FtVector::default(); 34];
    let limit: isize = 30;
    let mut first_arc = true;

    /* if all control points are coincident, this is a no-op; */
    /* avoid creating a spurious corner                       */
    if ft_is_small(stroker.center.x.wrapping_sub(control.x))
        && ft_is_small(stroker.center.y.wrapping_sub(control.y))
        && ft_is_small(control.x.wrapping_sub(to.x))
        && ft_is_small(control.y.wrapping_sub(to.y))
    {
        stroker.center = *to;
        return Ok(());
    }

    let mut arc: isize = 0;
    bez_stack[0] = *to;
    bez_stack[1] = *control;
    bez_stack[2] = stroker.center;

    while arc >= 0 {
        let a = arc as usize;

        /* initialize with current direction */
        let mut angle_in = stroker.angle_in;
        let mut angle_out = stroker.angle_in;

        if arc < limit && !ft_conic_is_small_enough(&bez_stack[a..], &mut angle_in, &mut angle_out)
        {
            if stroker.first_point {
                stroker.angle_in = angle_in;
            }

            ft_conic_split(&mut bez_stack[a..]);
            arc += 2;
            continue;
        }

        if first_arc {
            first_arc = false;

            /* process corner if necessary */
            if stroker.first_point {
                ft_stroker_subpath_start(stroker, angle_in, 0)?;
            } else {
                stroker.angle_out = angle_in;
                ft_stroker_process_corner(stroker, 0)?;
            }
        } else if ft_pos_abs(ft_angle_diff(stroker.angle_in, angle_in))
            > FT_SMALL_CONIC_THRESHOLD / 4
        {
            /* if the deviation from one arc to the next is too great, */
            /* add a round corner                                      */
            stroker.center = bez_stack[a + 2];
            stroker.angle_out = angle_in;
            stroker.line_join = FT_STROKER_LINEJOIN_ROUND;

            let error = ft_stroker_process_corner(stroker, 0);

            /* reinstate line join style */
            stroker.line_join = stroker.line_join_saved;

            error?;
        }

        /* the arc's angle is small enough; we can add it directly to each */
        /* border                                                          */
        {
            let mut alpha0: FtAngle = 0;

            let theta = ft_angle_diff(angle_in, angle_out) / 2;
            let phi = angle_in + theta;
            let length = ft_div_fix(stroker.radius, ft_cos(theta));

            let arc0 = bez_stack[a];
            let arc1 = bez_stack[a + 1];
            let arc2 = bez_stack[a + 2];

            /* compute direction of original arc */
            if stroker.handle_wide_strokes {
                alpha0 = ft_atan2(arc0.x.wrapping_sub(arc2.x), arc0.y.wrapping_sub(arc2.y));
            }

            let radius = stroker.radius;
            let handle_wide_strokes = stroker.handle_wide_strokes;

            for (side, border) in stroker.borders.iter_mut().enumerate() {
                let rotate = ft_side_to_rotate(side);

                /* compute control point */
                let ctrl = vadd(from_polar(length, phi + rotate), arc1);

                /* compute end point */
                let end = vadd(from_polar(radius, angle_out + rotate), arc0);

                if handle_wide_strokes {
                    /* determine whether the border radius is greater than the */
                    /* radius of curvature of the original arc                 */
                    let start = border.points[border.num_points as usize - 1];

                    let alpha1 = ft_atan2(end.x.wrapping_sub(start.x), end.y.wrapping_sub(start.y));

                    /* is the direction of the border arc opposite to */
                    /* that of the original arc? */
                    if ft_pos_abs(ft_angle_diff(alpha0, alpha1)) > FT_ANGLE_PI / 2 {
                        /* use the sine rule to find the intersection point */
                        let beta =
                            ft_atan2(arc2.x.wrapping_sub(start.x), arc2.y.wrapping_sub(start.y));
                        let gamma =
                            ft_atan2(arc0.x.wrapping_sub(end.x), arc0.y.wrapping_sub(end.y));

                        let bvec = vsub(end, start);

                        let blen = ft_vector_length(&bvec);

                        let sin_a = ft_pos_abs(ft_sin(alpha1 - gamma));
                        let sin_b = ft_pos_abs(ft_sin(beta - gamma));

                        let alen = ft_mul_div(blen, sin_a, sin_b);

                        let delta = vadd(from_polar(alen, beta), start);

                        /* circumnavigate the negative sector backwards */
                        border.movable = false;
                        ft_stroke_border_lineto(border, &delta, false)?;
                        ft_stroke_border_lineto(border, &end, false)?;
                        ft_stroke_border_conicto(border, &ctrl, &start)?;
                        /* and then move to the endpoint */
                        ft_stroke_border_lineto(border, &end, false)?;

                        continue;
                    }

                    /* else fall through */
                }

                /* simply add an arc */
                ft_stroke_border_conicto(border, &ctrl, &end)?;
            }
        }

        arc -= 2;

        stroker.angle_in = angle_out;
    }

    stroker.center = *to;
    stroker.line_length = 0;

    Ok(())
}

/// `FT_Stroker_CubicTo`
pub fn ft_stroker_cubic_to(
    stroker: &mut FtStrokerRec,
    control1: &FtVector,
    control2: &FtVector,
    to: &FtVector,
) -> FtResult<()> {
    let mut bez_stack = [FtVector::default(); 37];
    let limit: isize = 32;
    let mut first_arc = true;

    /* if all control points are coincident, this is a no-op; */
    /* avoid creating a spurious corner */
    if ft_is_small(stroker.center.x.wrapping_sub(control1.x))
        && ft_is_small(stroker.center.y.wrapping_sub(control1.y))
        && ft_is_small(control1.x.wrapping_sub(control2.x))
        && ft_is_small(control1.y.wrapping_sub(control2.y))
        && ft_is_small(control2.x.wrapping_sub(to.x))
        && ft_is_small(control2.y.wrapping_sub(to.y))
    {
        stroker.center = *to;
        return Ok(());
    }

    let mut arc: isize = 0;
    bez_stack[0] = *to;
    bez_stack[1] = *control2;
    bez_stack[2] = *control1;
    bez_stack[3] = stroker.center;

    while arc >= 0 {
        let a = arc as usize;

        /* initialize with current direction */
        let mut angle_in = stroker.angle_in;
        let mut angle_out = stroker.angle_in;
        let mut angle_mid = stroker.angle_in;

        if arc < limit
            && !ft_cubic_is_small_enough(
                &bez_stack[a..],
                &mut angle_in,
                &mut angle_mid,
                &mut angle_out,
            )
        {
            if stroker.first_point {
                stroker.angle_in = angle_in;
            }

            ft_cubic_split(&mut bez_stack[a..]);
            arc += 3;
            continue;
        }

        if first_arc {
            first_arc = false;

            /* process corner if necessary */
            if stroker.first_point {
                ft_stroker_subpath_start(stroker, angle_in, 0)?;
            } else {
                stroker.angle_out = angle_in;
                ft_stroker_process_corner(stroker, 0)?;
            }
        } else if ft_pos_abs(ft_angle_diff(stroker.angle_in, angle_in))
            > FT_SMALL_CUBIC_THRESHOLD / 4
        {
            /* if the deviation from one arc to the next is too great, */
            /* add a round corner                                      */
            stroker.center = bez_stack[a + 3];
            stroker.angle_out = angle_in;
            stroker.line_join = FT_STROKER_LINEJOIN_ROUND;

            let error = ft_stroker_process_corner(stroker, 0);

            /* reinstate line join style */
            stroker.line_join = stroker.line_join_saved;

            error?;
        }

        /* the arc's angle is small enough; we can add it directly to each */
        /* border                                                          */
        {
            let mut alpha0: FtAngle = 0;

            let theta1 = ft_angle_diff(angle_in, angle_mid) / 2;
            let theta2 = ft_angle_diff(angle_mid, angle_out) / 2;
            let phi1 = ft_angle_mean(angle_in, angle_mid);
            let phi2 = ft_angle_mean(angle_mid, angle_out);
            let length1 = ft_div_fix(stroker.radius, ft_cos(theta1));
            let length2 = ft_div_fix(stroker.radius, ft_cos(theta2));

            let arc0 = bez_stack[a];
            let arc1 = bez_stack[a + 1];
            let arc2 = bez_stack[a + 2];
            let arc3 = bez_stack[a + 3];

            /* compute direction of original arc */
            if stroker.handle_wide_strokes {
                alpha0 = ft_atan2(arc0.x.wrapping_sub(arc3.x), arc0.y.wrapping_sub(arc3.y));
            }

            let radius = stroker.radius;
            let handle_wide_strokes = stroker.handle_wide_strokes;

            for (side, border) in stroker.borders.iter_mut().enumerate() {
                let rotate = ft_side_to_rotate(side);

                /* compute control points */
                let ctrl1 = vadd(from_polar(length1, phi1 + rotate), arc2);

                let ctrl2 = vadd(from_polar(length2, phi2 + rotate), arc1);

                /* compute end point */
                let end = vadd(from_polar(radius, angle_out + rotate), arc0);

                if handle_wide_strokes {
                    /* determine whether the border radius is greater than the */
                    /* radius of curvature of the original arc                 */
                    let start = border.points[border.num_points as usize - 1];

                    let alpha1 = ft_atan2(end.x.wrapping_sub(start.x), end.y.wrapping_sub(start.y));

                    /* is the direction of the border arc opposite to */
                    /* that of the original arc? */
                    if ft_pos_abs(ft_angle_diff(alpha0, alpha1)) > FT_ANGLE_PI / 2 {
                        /* use the sine rule to find the intersection point */
                        let beta =
                            ft_atan2(arc3.x.wrapping_sub(start.x), arc3.y.wrapping_sub(start.y));
                        let gamma =
                            ft_atan2(arc0.x.wrapping_sub(end.x), arc0.y.wrapping_sub(end.y));

                        let bvec = vsub(end, start);

                        let blen = ft_vector_length(&bvec);

                        let sin_a = ft_pos_abs(ft_sin(alpha1 - gamma));
                        let sin_b = ft_pos_abs(ft_sin(beta - gamma));

                        let alen = ft_mul_div(blen, sin_a, sin_b);

                        let delta = vadd(from_polar(alen, beta), start);

                        /* circumnavigate the negative sector backwards */
                        border.movable = false;
                        ft_stroke_border_lineto(border, &delta, false)?;
                        ft_stroke_border_lineto(border, &end, false)?;
                        ft_stroke_border_cubicto(border, &ctrl2, &ctrl1, &start)?;
                        /* and then move to the endpoint */
                        ft_stroke_border_lineto(border, &end, false)?;

                        continue;
                    }

                    /* else fall through */
                }

                /* simply add an arc */
                ft_stroke_border_cubicto(border, &ctrl1, &ctrl2, &end)?;
            }
        }

        arc -= 3;

        stroker.angle_in = angle_out;
    }

    stroker.center = *to;
    stroker.line_length = 0;

    Ok(())
}

/// `FT_Stroker_BeginSubPath`
pub fn ft_stroker_begin_sub_path(
    stroker: &mut FtStrokerRec,
    to: &FtVector,
    open: bool,
) -> FtResult<()> {
    /* We cannot process the first point, because there is not enough      */
    /* information regarding its corner/cap.  The latter will be processed */
    /* in the `FT_Stroker_EndSubPath' routine.                             */
    /*                                                                     */
    stroker.first_point = true;
    stroker.center = *to;
    stroker.subpath_open = open;

    /* Determine if we need to check whether the border radius is greater */
    /* than the radius of curvature of a curve, to handle this case       */
    /* specially.  This is only required if bevel joins or butt caps may  */
    /* be created, because round & miter joins and round & square caps    */
    /* cover the negative sector created with wide strokes.               */
    stroker.handle_wide_strokes = stroker.line_join != FT_STROKER_LINEJOIN_ROUND
        || (stroker.subpath_open && stroker.line_cap == FT_STROKER_LINECAP_BUTT);

    /* record the subpath start point for each border */
    stroker.subpath_start = *to;

    stroker.angle_in = 0;

    Ok(())
}

/// `ft_stroker_add_reverse_left`
fn ft_stroker_add_reverse_left(stroker: &mut FtStrokerRec, open: bool) -> FtResult<()> {
    let [right, left] = &mut stroker.borders;

    let new_points = left.num_points as FtInt - left.start;
    if new_points > 0 {
        ft_stroke_border_grow(right, new_points as FtUInt)?;

        {
            let mut dst = right.num_points as usize;
            let mut src = left.num_points as isize - 1;

            while src >= left.start as isize {
                right.points[dst] = left.points[src as usize];
                right.tags[dst] = left.tags[src as usize];

                if open {
                    right.tags[dst] &= !FT_STROKE_TAG_BEGIN_END;
                } else {
                    let ttag = right.tags[dst] & FT_STROKE_TAG_BEGIN_END;

                    /* switch begin/end tags if necessary */
                    if ttag == FT_STROKE_TAG_BEGIN || ttag == FT_STROKE_TAG_END {
                        right.tags[dst] ^= FT_STROKE_TAG_BEGIN_END;
                    }
                }

                src -= 1;
                dst += 1;
            }
        }

        left.num_points = left.start as FtUInt;
        right.num_points += new_points as FtUInt;

        right.movable = false;
        left.movable = false;
    }

    Ok(())
}

/// `FT_Stroker_EndSubPath`: there's a lot of magic in this function!
pub fn ft_stroker_end_sub_path(stroker: &mut FtStrokerRec) -> FtResult<()> {
    if stroker.subpath_open {
        /* All right, this is an opened path, we need to add a cap between */
        /* right & left, add the reverse of left, then add a final cap     */
        /* between left & right.                                           */
        ft_stroker_cap(stroker, stroker.angle_in, 0)?;

        /* add reversed points from `left' to `right' */
        ft_stroker_add_reverse_left(stroker, true)?;

        /* now add the final cap */
        stroker.center = stroker.subpath_start;
        ft_stroker_cap(stroker, stroker.subpath_angle + FT_ANGLE_PI, 0)?;

        /* Now end the right subpath accordingly.  The left one is */
        /* rewind and doesn't need further processing.             */
        ft_stroke_border_close(&mut stroker.borders[0], false);
    } else {
        /* close the path if needed */
        if !ft_is_small(stroker.center.x.wrapping_sub(stroker.subpath_start.x))
            || !ft_is_small(stroker.center.y.wrapping_sub(stroker.subpath_start.y))
        {
            let start = stroker.subpath_start;
            ft_stroker_line_to(stroker, &start)?;
        }

        /* process the corner */
        stroker.angle_out = stroker.subpath_angle;
        ft_stroker_process_corner(stroker, stroker.subpath_line_length)?;

        /* then end our two subpaths */
        ft_stroke_border_close(&mut stroker.borders[0], false);
        ft_stroke_border_close(&mut stroker.borders[1], true);
    }

    Ok(())
}

/// `FT_Stroker_GetBorderCounts`: returns `(num_points, num_contours)`
pub fn ft_stroker_get_border_counts(
    stroker: &mut FtStrokerRec,
    border: FtStrokerBorder,
) -> FtResult<(FtUInt, FtUInt)> {
    if border > 1 {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    Ok(ft_stroke_border_get_counts(
        &mut stroker.borders[border as usize],
    ))
}

/// `FT_Stroker_GetCounts`: returns `(num_points, num_contours)`
pub fn ft_stroker_get_counts(stroker: &mut FtStrokerRec) -> FtResult<(FtUInt, FtUInt)> {
    let (count1, count2) = ft_stroke_border_get_counts(&mut stroker.borders[0]);

    let (count3, count4) = ft_stroke_border_get_counts(&mut stroker.borders[1]);

    let num_points = count1 + count3;
    let num_contours = count2 + count4;

    Ok((num_points, num_contours))
}

/// `FT_Stroker_ExportBorder`
pub fn ft_stroker_export_border(
    stroker: &FtStrokerRec,
    border: FtStrokerBorder,
    outline: &mut FtOutline,
) {
    if border == FT_STROKER_BORDER_LEFT || border == FT_STROKER_BORDER_RIGHT {
        let sborder = &stroker.borders[border as usize];

        if sborder.valid {
            ft_stroke_border_export(sborder, outline);
        }
    }
}

/// `FT_Stroker_Export`
pub fn ft_stroker_export(stroker: &FtStrokerRec, outline: &mut FtOutline) {
    ft_stroker_export_border(stroker, FT_STROKER_BORDER_LEFT, outline);
    ft_stroker_export_border(stroker, FT_STROKER_BORDER_RIGHT, outline);
}

/// `FT_Stroker_ParseOutline`: The following is very similar to
/// FT_Outline_Decompose, except that we do support opened paths, and do
/// not scale the outline.
pub fn ft_stroker_parse_outline(
    stroker: &mut FtStrokerRec,
    outline: &FtOutline,
    opened: bool,
) -> FtResult<()> {
    ft_stroker_rewind(stroker);

    let points = &outline.points;
    let tags = &outline.tags;

    let mut last: FtInt = -1;
    for n in 0..outline.n_contours as usize {
        let first = last + 1;
        last = outline.contours[n] as FtInt;

        /* skip empty points; we don't stroke these */
        if last <= first {
            continue;
        }

        let mut limit = last as isize;

        let mut v_start = points[first as usize];
        let v_last = points[last as usize];

        let mut v_control = v_start;

        let mut point = first as isize;

        let mut tag = ft_curve_tag(tags[first as usize]);

        /* A contour cannot start with a cubic control point! */
        if tag == FT_CURVE_TAG_CUBIC {
            return Err(FT_ERR_INVALID_OUTLINE);
        }

        /* check first point to determine origin */
        if tag == FT_CURVE_TAG_CONIC {
            /* First point is conic control.  Yes, this happens. */
            if ft_curve_tag(tags[last as usize]) == FT_CURVE_TAG_ON {
                /* start at last point if it is on the curve */
                v_start = v_last;
                limit -= 1;
            } else {
                /* if both first and last points are conic, */
                /* start at their middle                    */
                v_start.x = (v_start.x.wrapping_add(v_last.x)) / 2;
                v_start.y = (v_start.y.wrapping_add(v_last.y)) / 2;
            }
            point -= 1;
        }

        ft_stroker_begin_sub_path(stroker, &v_start, opened)?;

        let mut closed = false;
        while point < limit {
            point += 1;

            tag = ft_curve_tag(tags[point as usize]);
            match tag {
                FT_CURVE_TAG_ON => {
                    /* emit a single line_to */
                    let vec = points[point as usize];

                    ft_stroker_line_to(stroker, &vec)?;
                    continue;
                }

                FT_CURVE_TAG_CONIC => {
                    /* consume conic arcs */
                    v_control = points[point as usize];

                    /* Do_Conic: */
                    loop {
                        if point < limit {
                            point += 1;
                            tag = ft_curve_tag(tags[point as usize]);

                            let vec = points[point as usize];

                            if tag == FT_CURVE_TAG_ON {
                                ft_stroker_conic_to(stroker, &v_control, &vec)?;
                                break;
                            }

                            if tag != FT_CURVE_TAG_CONIC {
                                return Err(FT_ERR_INVALID_OUTLINE);
                            }

                            let v_middle = FtVector {
                                x: (v_control.x.wrapping_add(vec.x)) / 2,
                                y: (v_control.y.wrapping_add(vec.y)) / 2,
                            };

                            ft_stroker_conic_to(stroker, &v_control, &v_middle)?;

                            v_control = vec;
                            continue;
                        }

                        ft_stroker_conic_to(stroker, &v_control, &v_start)?;
                        closed = true;
                        break;
                    }

                    if closed {
                        break;
                    }
                    continue;
                }

                _ => {
                    /* FT_CURVE_TAG_CUBIC */
                    if point + 1 > limit
                        || ft_curve_tag(tags[point as usize + 1]) != FT_CURVE_TAG_CUBIC
                    {
                        return Err(FT_ERR_INVALID_OUTLINE);
                    }

                    point += 2;

                    let vec1 = points[point as usize - 2];
                    let vec2 = points[point as usize - 1];

                    if point <= limit {
                        let vec = points[point as usize];

                        ft_stroker_cubic_to(stroker, &vec1, &vec2, &vec)?;
                        continue;
                    }

                    ft_stroker_cubic_to(stroker, &vec1, &vec2, &v_start)?;
                    break;
                }
            }
        }

        /* Close: */

        /* don't try to end the path if no segments have been generated */
        if !stroker.first_point {
            ft_stroker_end_sub_path(stroker)?;
        }
    }

    Ok(())
}

/// `FT_Glyph_Stroke`: on success `pglyph` holds the stroked glyph, and
/// the old one is returned unless `destroy` is set; on failure `pglyph`
/// is unchanged unless `destroy` is not set (C then sets it to NULL:
/// `None` here).
pub fn ft_glyph_stroke(
    pglyph: &mut Option<FtGlyph>,
    stroker: &mut FtStrokerRec,
    destroy: bool,
) -> FtResult<Option<FtGlyph>> {
    let Some(FtGlyph::Outline(_)) = pglyph.as_ref() else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };

    let mut glyph = ft_glyph_copy(pglyph.as_ref().unwrap())?;

    let error = (|| -> FtResult<()> {
        let FtGlyph::Outline(oglyph) = &mut glyph else {
            unreachable!()
        };
        let outline = &mut oglyph.outline;

        ft_stroker_parse_outline(stroker, outline, false)?;

        let (num_points, num_contours) = ft_stroker_get_counts(stroker).unwrap_or((0, 0));

        *outline = FtOutline::default();

        *outline = ft_outline_new(num_points, num_contours as FtInt)?;

        outline.n_points = 0;
        outline.n_contours = 0;

        ft_stroker_export(stroker, outline);

        Ok(())
    })();

    match error {
        Ok(()) => {
            let old = pglyph.replace(glyph);
            if destroy {
                if let Some(old) = old {
                    ft_done_glyph(old);
                }
                return Ok(None);
            }

            Ok(old)
        }
        Err(e) => {
            /* Fail: */
            ft_done_glyph(glyph);

            if !destroy {
                *pglyph = None;
            }

            Err(e)
        }
    }
}

/// `FT_Glyph_StrokeBorder`: as [`ft_glyph_stroke`].
pub fn ft_glyph_stroke_border(
    pglyph: &mut Option<FtGlyph>,
    stroker: &mut FtStrokerRec,
    inside: bool,
    destroy: bool,
) -> FtResult<Option<FtGlyph>> {
    let Some(FtGlyph::Outline(_)) = pglyph.as_ref() else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };

    let mut glyph = ft_glyph_copy(pglyph.as_ref().unwrap())?;

    let error = (|| -> FtResult<()> {
        let FtGlyph::Outline(oglyph) = &mut glyph else {
            unreachable!()
        };
        let outline = &mut oglyph.outline;

        let mut border = ft_outline_get_outside_border(outline);
        if inside {
            if border == FT_STROKER_BORDER_LEFT {
                border = FT_STROKER_BORDER_RIGHT;
            } else {
                border = FT_STROKER_BORDER_LEFT;
            }
        }

        ft_stroker_parse_outline(stroker, outline, false)?;

        let (num_points, num_contours) =
            ft_stroker_get_border_counts(stroker, border).unwrap_or((0, 0));

        *outline = FtOutline::default();

        *outline = ft_outline_new(num_points, num_contours as FtInt)?;

        outline.n_points = 0;
        outline.n_contours = 0;

        ft_stroker_export_border(stroker, border, outline);

        Ok(())
    })();

    match error {
        Ok(()) => {
            let old = pglyph.replace(glyph);
            if destroy {
                if let Some(old) = old {
                    ft_done_glyph(old);
                }
                return Ok(None);
            }

            Ok(old)
        }
        Err(e) => {
            /* Fail: */
            ft_done_glyph(glyph);

            if !destroy {
                *pglyph = None;
            }

            Err(e)
        }
    }
}
