// Rust translation of source/plutovg-ft-stroker.c and
// plutovg-ft-stroker.h from PlutoVG (0.0.12, as SDL_ttf's external/plutovg
// pins it): PlutoVG's copy of FreeType's ftstroke.c and ftstroke.h.
// Copyright 2002-2006, 2008-2011, 2013 by David Turner, Robert Wilhelm,
// and Werner Lemberg (ftstroke.h: Copyright 2002-2006, 2008, 2009,
// 2011-2012).
// This file is part of the FreeType project, and may only be used,
// modified, and distributed under the terms of the FreeType project
// license, FTL.TXT. This is an altered (translated) version of the
// original software; it is used under the FreeType License (see
// sdl3-ttf/FTL.TXT and LICENSE.txt).

//! FreeType path stroker (body).
//!
//! Translation notes: the borders' point and tag arrays are vectors (the
//! number of points is their length); the `assert`s are left out, as
//! SDL_ttf's (release) build compiles them out.

use super::ft_math::*;
use super::ft_raster::*;

/// `PVG_FT_Stroker_LineJoin`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PvgFtStrokerLineJoin {
    Round = 0,
    Bevel = 1,
    MiterVariable = 2,
    MiterFixed = 3,
}

/// `PVG_FT_STROKER_LINEJOIN_MITER` (an alias of `MiterVariable`)
pub const PVG_FT_STROKER_LINEJOIN_MITER: PvgFtStrokerLineJoin = PvgFtStrokerLineJoin::MiterVariable;

/// `PVG_FT_Stroker_LineCap`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PvgFtStrokerLineCap {
    Butt = 0,
    Round,
    Square,
}

/// `PVG_FT_StrokerBorder`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PvgFtStrokerBorder {
    Left = 0,
    Right,
}

/***************************************************************************/
/*****                      BEZIER COMPUTATIONS                      *****/
/***************************************************************************/

const PVG_FT_SMALL_CONIC_THRESHOLD: PvgFtAngle = PVG_FT_ANGLE_PI / 6;
const PVG_FT_SMALL_CUBIC_THRESHOLD: PvgFtAngle = PVG_FT_ANGLE_PI / 8;

const PVG_FT_EPSILON: i64 = 2;

/// `PVG_FT_IS_SMALL`
#[inline]
fn pvg_ft_is_small(x: i64) -> bool {
    x > -PVG_FT_EPSILON && x < PVG_FT_EPSILON
}

/// `ft_pos_abs`
#[inline]
fn ft_pos_abs(x: PvgFtPos) -> PvgFtPos {
    if x >= 0 {
        x
    } else {
        x.wrapping_neg()
    }
}

/// `ft_conic_split`
fn ft_conic_split(base: &mut [PvgFtVector]) {
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
    base: &[PvgFtVector],
    angle_in: &mut PvgFtAngle,
    angle_out: &mut PvgFtAngle,
) -> bool {
    let d1 = PvgFtVector {
        x: base[1].x.wrapping_sub(base[2].x),
        y: base[1].y.wrapping_sub(base[2].y),
    };
    let d2 = PvgFtVector {
        x: base[0].x.wrapping_sub(base[1].x),
        y: base[0].y.wrapping_sub(base[1].y),
    };

    let close1 = pvg_ft_is_small(d1.x) && pvg_ft_is_small(d1.y);
    let close2 = pvg_ft_is_small(d2.x) && pvg_ft_is_small(d2.y);

    if close1 {
        if close2 {
            /* basically a point;                      */
            /* do nothing to retain original direction */
        } else {
            *angle_in = pvg_ft_atan2(d2.x, d2.y);
            *angle_out = *angle_in;
        }
    } else
    /* !close1 */
    {
        if close2 {
            *angle_in = pvg_ft_atan2(d1.x, d1.y);
            *angle_out = *angle_in;
        } else {
            *angle_in = pvg_ft_atan2(d1.x, d1.y);
            *angle_out = pvg_ft_atan2(d2.x, d2.y);
        }
    }

    let theta = ft_pos_abs(pvg_ft_angle_diff(*angle_in, *angle_out));

    theta < PVG_FT_SMALL_CONIC_THRESHOLD
}

/// `ft_cubic_split`
fn ft_cubic_split(base: &mut [PvgFtVector]) {
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

/// `ft_angle_mean`: return the average of `angle1' and `angle2'. This
/// gives correct result even if `angle1' and `angle2' have opposite
/// signs.
fn ft_angle_mean(angle1: PvgFtAngle, angle2: PvgFtAngle) -> PvgFtAngle {
    angle1.wrapping_add(pvg_ft_angle_diff(angle1, angle2) / 2)
}

/// `ft_cubic_is_small_enough`
fn ft_cubic_is_small_enough(
    base: &[PvgFtVector],
    angle_in: &mut PvgFtAngle,
    angle_mid: &mut PvgFtAngle,
    angle_out: &mut PvgFtAngle,
) -> bool {
    let d1 = PvgFtVector {
        x: base[2].x.wrapping_sub(base[3].x),
        y: base[2].y.wrapping_sub(base[3].y),
    };
    let d2 = PvgFtVector {
        x: base[1].x.wrapping_sub(base[2].x),
        y: base[1].y.wrapping_sub(base[2].y),
    };
    let d3 = PvgFtVector {
        x: base[0].x.wrapping_sub(base[1].x),
        y: base[0].y.wrapping_sub(base[1].y),
    };

    let close1 = pvg_ft_is_small(d1.x) && pvg_ft_is_small(d1.y);
    let close2 = pvg_ft_is_small(d2.x) && pvg_ft_is_small(d2.y);
    let close3 = pvg_ft_is_small(d3.x) && pvg_ft_is_small(d3.y);

    if close1 {
        if close2 {
            if close3 {
                /* basically a point;                      */
                /* do nothing to retain original direction */
            } else
            /* !close3 */
            {
                *angle_in = pvg_ft_atan2(d3.x, d3.y);
                *angle_mid = *angle_in;
                *angle_out = *angle_in;
            }
        } else
        /* !close2 */
        {
            if close3 {
                *angle_in = pvg_ft_atan2(d2.x, d2.y);
                *angle_mid = *angle_in;
                *angle_out = *angle_in;
            } else
            /* !close3 */
            {
                *angle_in = pvg_ft_atan2(d2.x, d2.y);
                *angle_mid = *angle_in;
                *angle_out = pvg_ft_atan2(d3.x, d3.y);
            }
        }
    } else
    /* !close1 */
    {
        if close2 {
            if close3 {
                *angle_in = pvg_ft_atan2(d1.x, d1.y);
                *angle_mid = *angle_in;
                *angle_out = *angle_in;
            } else
            /* !close3 */
            {
                *angle_in = pvg_ft_atan2(d1.x, d1.y);
                *angle_out = pvg_ft_atan2(d3.x, d3.y);
                *angle_mid = ft_angle_mean(*angle_in, *angle_out);
            }
        } else
        /* !close2 */
        {
            if close3 {
                *angle_in = pvg_ft_atan2(d1.x, d1.y);
                *angle_mid = pvg_ft_atan2(d2.x, d2.y);
                *angle_out = *angle_mid;
            } else
            /* !close3 */
            {
                *angle_in = pvg_ft_atan2(d1.x, d1.y);
                *angle_mid = pvg_ft_atan2(d2.x, d2.y);
                *angle_out = pvg_ft_atan2(d3.x, d3.y);
            }
        }
    }

    let theta1 = ft_pos_abs(pvg_ft_angle_diff(*angle_in, *angle_mid));
    let theta2 = ft_pos_abs(pvg_ft_angle_diff(*angle_mid, *angle_out));

    theta1 < PVG_FT_SMALL_CUBIC_THRESHOLD && theta2 < PVG_FT_SMALL_CUBIC_THRESHOLD
}

/***************************************************************************/
/*****                       STROKE BORDERS                          *****/
/***************************************************************************/

/* PVG_FT_StrokeTags */
const PVG_FT_STROKE_TAG_ON: u8 = 1; /* on-curve point  */
const PVG_FT_STROKE_TAG_CUBIC: u8 = 2; /* cubic off-point */
const PVG_FT_STROKE_TAG_BEGIN: u8 = 4; /* sub-path start  */
const PVG_FT_STROKE_TAG_END: u8 = 8; /* sub-path end    */

const PVG_FT_STROKE_TAG_BEGIN_END: u8 = PVG_FT_STROKE_TAG_BEGIN | PVG_FT_STROKE_TAG_END;

/// `PVG_FT_StrokeBorderRec` (`num_points` is the length of `points` and
/// `tags`)
#[derive(Debug, Clone, Default)]
struct StrokeBorder {
    points: Vec<PvgFtVector>,
    tags: Vec<u8>,
    movable: bool, /* TRUE for ends of lineto borders */
    start: i32,    /* index of current sub-path start point */
    valid: bool,
}

impl StrokeBorder {
    #[inline]
    fn num_points(&self) -> u32 {
        self.points.len() as u32
    }

    /// `border->num_points = n` (for `n` at most the current count)
    #[inline]
    fn set_num_points(&mut self, n: u32) {
        self.points.truncate(n as usize);
        self.tags.truncate(n as usize);
    }

    #[inline]
    fn last_point(&self) -> PvgFtVector {
        self.points.last().copied().unwrap_or_default()
    }
}

/// `ft_stroke_border_grow` (for the reservation; errors are not
/// reported, as C's)
fn ft_stroke_border_grow(border: &mut StrokeBorder, new_points: u32) -> PvgFtError {
    let _ = border.points.try_reserve(new_points as usize);
    let _ = border.tags.try_reserve(new_points as usize);
    0
}

/// `ft_stroke_border_close`
fn ft_stroke_border_close(border: &mut StrokeBorder, reverse: bool) {
    let start = border.start as u32;
    let mut count = border.num_points();

    /* assert(border->start >= 0); */

    /* don't record empty paths! */
    if count <= start.wrapping_add(1) {
        border.set_num_points(start);
    } else {
        /* copy the last point to the start of this sub-path, since */
        /* it contains the `adjusted' starting coordinates          */
        count -= 1;
        let (s, c) = (start as usize, count as usize);
        border.points[s] = border.points[c];
        border.tags[s] = border.tags[c];
        border.set_num_points(count);

        if reverse {
            /* reverse the points */
            if c >= s + 2 {
                border.points[s + 1..c].reverse();
                /* then the tags */
                border.tags[s + 1..c].reverse();
            }
        }

        border.tags[s] |= PVG_FT_STROKE_TAG_BEGIN;
        border.tags[c - 1] |= PVG_FT_STROKE_TAG_END;
    }

    border.start = -1;
    border.movable = false;
}

/// `ft_stroke_border_lineto`
fn ft_stroke_border_lineto(
    border: &mut StrokeBorder,
    to: &PvgFtVector,
    movable: bool,
) -> PvgFtError {
    let mut error = 0;

    /* assert(border->start >= 0); */

    if border.movable {
        /* move last point */
        if let Some(p) = border.points.last_mut() {
            *p = *to;
        }
    } else {
        /* don't add zero-length lineto, but always add moveto */
        if border.num_points() > border.start as u32
            && pvg_ft_is_small(border.last_point().x.wrapping_sub(to.x))
            && pvg_ft_is_small(border.last_point().y.wrapping_sub(to.y))
        {
            return error;
        }

        /* add one point */
        error = ft_stroke_border_grow(border, 1);
        if error == 0 {
            border.points.push(*to);
            border.tags.push(PVG_FT_STROKE_TAG_ON);
        }
    }
    border.movable = movable;
    error
}

/// `ft_stroke_border_conicto`
fn ft_stroke_border_conicto(
    border: &mut StrokeBorder,
    control: &PvgFtVector,
    to: &PvgFtVector,
) -> PvgFtError {
    /* assert(border->start >= 0); */

    let error = ft_stroke_border_grow(border, 2);
    if error == 0 {
        border.points.push(*control);
        border.points.push(*to);

        border.tags.push(0);
        border.tags.push(PVG_FT_STROKE_TAG_ON);
    }

    border.movable = false;

    error
}

/// `ft_stroke_border_cubicto`
fn ft_stroke_border_cubicto(
    border: &mut StrokeBorder,
    control1: &PvgFtVector,
    control2: &PvgFtVector,
    to: &PvgFtVector,
) -> PvgFtError {
    /* assert(border->start >= 0); */

    let error = ft_stroke_border_grow(border, 3);
    if error == 0 {
        border.points.push(*control1);
        border.points.push(*control2);
        border.points.push(*to);

        border.tags.push(PVG_FT_STROKE_TAG_CUBIC);
        border.tags.push(PVG_FT_STROKE_TAG_CUBIC);
        border.tags.push(PVG_FT_STROKE_TAG_ON);
    }

    border.movable = false;

    error
}

const PVG_FT_ARC_CUBIC_ANGLE: PvgFtAngle = PVG_FT_ANGLE_PI / 2;

/// `ft_stroke_border_arcto`
fn ft_stroke_border_arcto(
    border: &mut StrokeBorder,
    center: &PvgFtVector,
    radius: PvgFtFixed,
    angle_start: PvgFtAngle,
    angle_diff: PvgFtAngle,
) -> PvgFtError {
    let mut a0 = PvgFtVector::default();
    let mut a1 = PvgFtVector::default();
    let mut a2 = PvgFtVector::default();
    let mut a3 = PvgFtVector::default();
    let mut arcs: PvgFtInt = 1;
    let mut error = 0;

    /* number of cubic arcs to draw */
    while angle_diff > PVG_FT_ARC_CUBIC_ANGLE * arcs as i64
        || -angle_diff > PVG_FT_ARC_CUBIC_ANGLE * arcs as i64
    {
        arcs += 1;
    }

    /* control tangents */
    let mut coef = pvg_ft_tan(angle_diff / (4 * arcs as i64));
    coef = coef.wrapping_add(coef / 3);

    /* compute start and first control point */
    pvg_ft_vector_from_polar(&mut a0, radius, angle_start);
    a1.x = pvg_ft_mul_fix(a0.y.wrapping_neg(), coef);
    a1.y = pvg_ft_mul_fix(a0.x, coef);

    a0.x = a0.x.wrapping_add(center.x);
    a0.y = a0.y.wrapping_add(center.y);
    a1.x = a1.x.wrapping_add(a0.x);
    a1.y = a1.y.wrapping_add(a0.y);

    for i in 1..=arcs {
        /* compute end and second control point */
        pvg_ft_vector_from_polar(
            &mut a3,
            radius,
            angle_start.wrapping_add(i as i64 * angle_diff / arcs as i64),
        );
        a2.x = pvg_ft_mul_fix(a3.y, coef);
        a2.y = pvg_ft_mul_fix(a3.x.wrapping_neg(), coef);

        a3.x = a3.x.wrapping_add(center.x);
        a3.y = a3.y.wrapping_add(center.y);
        a2.x = a2.x.wrapping_add(a3.x);
        a2.y = a2.y.wrapping_add(a3.y);

        /* add cubic arc */
        error = ft_stroke_border_cubicto(border, &a1, &a2, &a3);
        if error != 0 {
            break;
        }

        /* a0 = a3; */
        a1.x = a3.x.wrapping_sub(a2.x).wrapping_add(a3.x);
        a1.y = a3.y.wrapping_sub(a2.y).wrapping_add(a3.y);
    }

    error
}

/// `ft_stroke_border_moveto`
fn ft_stroke_border_moveto(border: &mut StrokeBorder, to: &PvgFtVector) -> PvgFtError {
    /* close current open path if any ? */
    if border.start >= 0 {
        ft_stroke_border_close(border, false);
    }

    border.start = border.num_points() as i32;
    border.movable = false;

    ft_stroke_border_lineto(border, to, false)
}

/// `ft_stroke_border_init`
fn ft_stroke_border_init(border: &mut StrokeBorder) {
    border.points = Vec::new();
    border.tags = Vec::new();

    border.start = -1;
    border.valid = false;
}

/// `ft_stroke_border_reset`
fn ft_stroke_border_reset(border: &mut StrokeBorder) {
    border.set_num_points(0);
    border.start = -1;
    border.valid = false;
}

/// `ft_stroke_border_get_counts`
fn ft_stroke_border_get_counts(
    border: &mut StrokeBorder,
    anum_points: &mut PvgFtUInt,
    anum_contours: &mut PvgFtUInt,
) -> PvgFtError {
    let error = 0;
    let mut num_points: PvgFtUInt = 0;
    let mut num_contours: PvgFtUInt = 0;

    let mut in_contour = 0;
    let mut fail = false;

    for &tag in &border.tags {
        if tag & PVG_FT_STROKE_TAG_BEGIN != 0 {
            if in_contour != 0 {
                fail = true;
                break;
            }

            in_contour = 1;
        } else if in_contour == 0 {
            fail = true;
            break;
        }

        if tag & PVG_FT_STROKE_TAG_END != 0 {
            in_contour = 0;
            num_contours += 1;
        }
        num_points += 1;
    }

    if !fail && in_contour != 0 {
        fail = true;
    }

    if fail {
        num_points = 0;
        num_contours = 0;
    } else {
        border.valid = true;
    }

    *anum_points = num_points;
    *anum_contours = num_contours;
    error
}

/// `ft_stroke_border_export`
fn ft_stroke_border_export(border: &StrokeBorder, outline: &mut PvgFtOutline) {
    let base = outline.n_points.max(0) as usize;
    let count = border.points.len();

    /* copy point locations */
    /* copy tags */
    for i in 0..count {
        let read = border.tags[i];
        let write = if read & PVG_FT_STROKE_TAG_ON != 0 {
            PVG_FT_CURVE_TAG_ON
        } else if read & PVG_FT_STROKE_TAG_CUBIC != 0 {
            PVG_FT_CURVE_TAG_CUBIC
        } else {
            PVG_FT_CURVE_TAG_CONIC
        };
        if let Some(p) = outline.points.get_mut(base + i) {
            *p = border.points[i];
        }
        if let Some(t) = outline.tags.get_mut(base + i) {
            *t = write;
        }
    }

    /* copy contours */
    {
        let mut idx = outline.n_points;

        for &tag in &border.tags {
            if tag & PVG_FT_STROKE_TAG_END != 0 {
                if let Some(c) = outline.contours.get_mut(outline.n_contours.max(0) as usize) {
                    *c = idx;
                }
                outline.n_contours += 1;
            }
            idx = idx.wrapping_add(1);
        }
    }

    outline.n_points = outline.n_points.wrapping_add(count as i32);

    /* assert(PVG_FT_Outline_Check(outline) == 0); */
}

/***************************************************************************/
/*****                           STROKER                             *****/
/***************************************************************************/

/// `PVG_FT_SIDE_TO_ROTATE`
#[inline]
fn pvg_ft_side_to_rotate(s: PvgFtInt) -> PvgFtAngle {
    PVG_FT_ANGLE_PI2 - s as i64 * PVG_FT_ANGLE_PI
}

/// `PVG_FT_StrokerRec`
#[derive(Debug, Clone)]
pub struct PvgFtStroker {
    angle_in: PvgFtAngle,            /* direction into curr join */
    angle_out: PvgFtAngle,           /* direction out of join  */
    center: PvgFtVector,             /* current position */
    line_length: PvgFtFixed,         /* length of last lineto */
    first_point: bool,               /* is this the start? */
    subpath_open: bool,              /* is the subpath open? */
    subpath_angle: PvgFtAngle,       /* subpath start direction */
    subpath_start: PvgFtVector,      /* subpath start position */
    subpath_line_length: PvgFtFixed, /* subpath start lineto len */
    handle_wide_strokes: bool,       /* use wide strokes logic? */

    line_cap: PvgFtStrokerLineCap,
    line_join: PvgFtStrokerLineJoin,
    line_join_saved: PvgFtStrokerLineJoin,
    miter_limit: PvgFtFixed,
    radius: PvgFtFixed,

    borders: [StrokeBorder; 2],
}

/// `PVG_FT_Stroker_New`
pub fn pvg_ft_stroker_new() -> PvgFtStroker {
    let mut stroker = PvgFtStroker {
        angle_in: 0,
        angle_out: 0,
        center: PvgFtVector::default(),
        line_length: 0,
        first_point: false,
        subpath_open: false,
        subpath_angle: 0,
        subpath_start: PvgFtVector::default(),
        subpath_line_length: 0,
        handle_wide_strokes: false,
        line_cap: PvgFtStrokerLineCap::Butt,
        line_join: PvgFtStrokerLineJoin::Round,
        line_join_saved: PvgFtStrokerLineJoin::Round,
        miter_limit: 0,
        radius: 0,
        borders: [StrokeBorder::default(), StrokeBorder::default()],
    };
    ft_stroke_border_init(&mut stroker.borders[0]);
    ft_stroke_border_init(&mut stroker.borders[1]);
    stroker
}

/// `PVG_FT_Stroker_Rewind`
pub fn pvg_ft_stroker_rewind(stroker: &mut PvgFtStroker) {
    ft_stroke_border_reset(&mut stroker.borders[0]);
    ft_stroke_border_reset(&mut stroker.borders[1]);
}

/// `PVG_FT_Stroker_Set`
pub fn pvg_ft_stroker_set(
    stroker: &mut PvgFtStroker,
    radius: PvgFtFixed,
    line_cap: PvgFtStrokerLineCap,
    line_join: PvgFtStrokerLineJoin,
    miter_limit: PvgFtFixed,
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

    pvg_ft_stroker_rewind(stroker);
}

/// `ft_stroker_arcto`: create a circular arc at a corner or cap
fn ft_stroker_arcto(stroker: &mut PvgFtStroker, side: PvgFtInt) -> PvgFtError {
    let radius = stroker.radius;

    let rotate = pvg_ft_side_to_rotate(side);

    let mut total = pvg_ft_angle_diff(stroker.angle_in, stroker.angle_out);
    if total == PVG_FT_ANGLE_PI {
        total = -rotate * 2;
    }

    let center = stroker.center;
    let angle_in = stroker.angle_in;
    let border = &mut stroker.borders[side as usize];
    let error = ft_stroke_border_arcto(
        border,
        &center,
        radius,
        angle_in.wrapping_add(rotate),
        total,
    );
    border.movable = false;
    error
}

/// `ft_stroker_cap`: add a cap at the end of an opened path
fn ft_stroker_cap(stroker: &mut PvgFtStroker, angle: PvgFtAngle, side: PvgFtInt) -> PvgFtError {
    let mut error;

    if stroker.line_cap == PvgFtStrokerLineCap::Round {
        /* add a round cap */
        stroker.angle_in = angle;
        stroker.angle_out = angle.wrapping_add(PVG_FT_ANGLE_PI);

        error = ft_stroker_arcto(stroker, side);
    } else {
        /* add a square or butt cap */
        let mut middle = PvgFtVector::default();
        let mut delta = PvgFtVector::default();
        let radius = stroker.radius;

        /* compute middle point and first angle point */
        pvg_ft_vector_from_polar(&mut middle, radius, angle);
        delta.x = if side != 0 {
            middle.y
        } else {
            middle.y.wrapping_neg()
        };
        delta.y = if side != 0 {
            middle.x.wrapping_neg()
        } else {
            middle.x
        };

        if stroker.line_cap == PvgFtStrokerLineCap::Square {
            middle.x = middle.x.wrapping_add(stroker.center.x);
            middle.y = middle.y.wrapping_add(stroker.center.y);
        } else
        /* PVG_FT_STROKER_LINECAP_BUTT */
        {
            middle.x = stroker.center.x;
            middle.y = stroker.center.y;
        }

        delta.x = delta.x.wrapping_add(middle.x);
        delta.y = delta.y.wrapping_add(middle.y);

        let border = &mut stroker.borders[side as usize];
        error = ft_stroke_border_lineto(border, &delta, false);
        if error == 0 {
            /* compute second angle point */
            delta.x = middle.x.wrapping_sub(delta.x).wrapping_add(middle.x);
            delta.y = middle.y.wrapping_sub(delta.y).wrapping_add(middle.y);

            error = ft_stroke_border_lineto(border, &delta, false);
        }
    }

    error
}

/// `ft_stroker_inside`: process an inside corner, i.e. compute
/// intersection
fn ft_stroker_inside(
    stroker: &mut PvgFtStroker,
    side: PvgFtInt,
    line_length: PvgFtFixed,
) -> PvgFtError {
    let mut sigma = PvgFtVector { x: 0, y: 0 };
    let mut delta = PvgFtVector::default();
    let intersect: bool; /* use intersection of lines? */

    let rotate = pvg_ft_side_to_rotate(side);

    let theta = pvg_ft_angle_diff(stroker.angle_in, stroker.angle_out) / 2;

    /* Only intersect borders if between two lineto's and both */
    /* lines are long enough (line_length is zero for curves). */
    if !stroker.borders[side as usize].movable
        || line_length == 0
        || !(-0x59C000..=0x59C000).contains(&theta)
    {
        intersect = false;
    } else {
        /* compute minimum required length of lines */
        pvg_ft_vector_unit(&mut sigma, theta);
        let min_length = ft_pos_abs(pvg_ft_mul_div(stroker.radius, sigma.y, sigma.x));

        intersect =
            min_length != 0 && stroker.line_length >= min_length && line_length >= min_length;
    }

    if !intersect {
        pvg_ft_vector_from_polar(
            &mut delta,
            stroker.radius,
            stroker.angle_out.wrapping_add(rotate),
        );
        delta.x = delta.x.wrapping_add(stroker.center.x);
        delta.y = delta.y.wrapping_add(stroker.center.y);

        stroker.borders[side as usize].movable = false;
    } else {
        /* compute median angle */
        let phi = stroker.angle_in.wrapping_add(theta).wrapping_add(rotate);

        let length = pvg_ft_div_fix(stroker.radius, sigma.x);

        pvg_ft_vector_from_polar(&mut delta, length, phi);
        delta.x = delta.x.wrapping_add(stroker.center.x);
        delta.y = delta.y.wrapping_add(stroker.center.y);
    }

    ft_stroke_border_lineto(&mut stroker.borders[side as usize], &delta, false)
}

/// `ft_stroker_outside`: process an outside corner, i.e. compute
/// bevel/miter/round
fn ft_stroker_outside(
    stroker: &mut PvgFtStroker,
    side: PvgFtInt,
    line_length: PvgFtFixed,
) -> PvgFtError {
    let error;

    if stroker.line_join == PvgFtStrokerLineJoin::Round {
        error = ft_stroker_arcto(stroker, side);
    } else {
        /* this is a mitered (pointed) or beveled (truncated) corner */
        let radius = stroker.radius;
        let mut sigma = PvgFtVector { x: 0, y: 0 };
        let mut phi: PvgFtAngle = 0;

        let rotate = pvg_ft_side_to_rotate(side);

        let mut bevel = stroker.line_join == PvgFtStrokerLineJoin::Bevel;

        let fixed_bevel = stroker.line_join != PvgFtStrokerLineJoin::MiterVariable;

        /* check miter limit first */
        if !bevel {
            let mut theta = pvg_ft_angle_diff(stroker.angle_in, stroker.angle_out) / 2;

            if theta == PVG_FT_ANGLE_PI2 {
                theta = -rotate;
            }

            phi = stroker.angle_in.wrapping_add(theta).wrapping_add(rotate);

            pvg_ft_vector_from_polar(&mut sigma, stroker.miter_limit, theta);

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
        let border = &mut stroker.borders[side as usize];

        if bevel {
            /* this is a bevel (broken angle) */
            if fixed_bevel {
                /* the outer corners are simply joined together */
                let mut delta = PvgFtVector::default();

                /* add bevel */
                pvg_ft_vector_from_polar(&mut delta, radius, angle_out.wrapping_add(rotate));
                delta.x = delta.x.wrapping_add(center.x);
                delta.y = delta.y.wrapping_add(center.y);

                border.movable = false;
                error = ft_stroke_border_lineto(border, &delta, false);
            } else
            /* variable bevel or clipped miter */
            {
                /* the miter is truncated */
                let mut middle = PvgFtVector::default();
                let mut delta = PvgFtVector::default();

                /* compute middle point and first angle point */
                pvg_ft_vector_from_polar(&mut middle, pvg_ft_mul_fix(radius, miter_limit), phi);

                let coef = pvg_ft_div_fix(0x10000i64.wrapping_sub(sigma.x), sigma.y);
                delta.x = pvg_ft_mul_fix(middle.y, coef);
                delta.y = pvg_ft_mul_fix(middle.x.wrapping_neg(), coef);

                middle.x = middle.x.wrapping_add(center.x);
                middle.y = middle.y.wrapping_add(center.y);
                delta.x = delta.x.wrapping_add(middle.x);
                delta.y = delta.y.wrapping_add(middle.y);

                let mut e = ft_stroke_border_lineto(border, &delta, false);
                if e == 0 {
                    /* compute second angle point */
                    delta.x = middle.x.wrapping_sub(delta.x).wrapping_add(middle.x);
                    delta.y = middle.y.wrapping_sub(delta.y).wrapping_add(middle.y);

                    e = ft_stroke_border_lineto(border, &delta, false);
                    if e == 0 {
                        /* finally, add an end point; only needed if not lineto */
                        /* (line_length is zero for curves)                     */
                        if line_length == 0 {
                            pvg_ft_vector_from_polar(
                                &mut delta,
                                radius,
                                angle_out.wrapping_add(rotate),
                            );

                            delta.x = delta.x.wrapping_add(center.x);
                            delta.y = delta.y.wrapping_add(center.y);

                            e = ft_stroke_border_lineto(border, &delta, false);
                        }
                    }
                }
                error = e;
            }
        } else
        /* this is a miter (intersection) */
        {
            let mut delta = PvgFtVector::default();

            let length = pvg_ft_mul_div(radius, miter_limit, sigma.x);

            pvg_ft_vector_from_polar(&mut delta, length, phi);
            delta.x = delta.x.wrapping_add(center.x);
            delta.y = delta.y.wrapping_add(center.y);

            let mut e = ft_stroke_border_lineto(border, &delta, false);
            if e == 0 {
                /* now add an end point; only needed if not lineto */
                /* (line_length is zero for curves)                */
                if line_length == 0 {
                    pvg_ft_vector_from_polar(&mut delta, radius, angle_out.wrapping_add(rotate));
                    delta.x = delta.x.wrapping_add(center.x);
                    delta.y = delta.y.wrapping_add(center.y);

                    e = ft_stroke_border_lineto(border, &delta, false);
                }
            }
            error = e;
        }
    }

    error
}

/// `ft_stroker_process_corner`
fn ft_stroker_process_corner(stroker: &mut PvgFtStroker, line_length: PvgFtFixed) -> PvgFtError {
    let turn = pvg_ft_angle_diff(stroker.angle_in, stroker.angle_out);

    /* no specific corner processing is required if the turn is 0 */
    if turn == 0 {
        return 0;
    }

    /* when we turn to the right, the inside side is 0 */
    let mut inside_side = 0;

    /* otherwise, the inside side is 1 */
    if turn < 0 {
        inside_side = 1;
    }

    /* process the inside side */
    let error = ft_stroker_inside(stroker, inside_side, line_length);
    if error != 0 {
        return error;
    }

    /* process the outside side */
    ft_stroker_outside(stroker, 1 - inside_side, line_length)
}

/// `ft_stroker_subpath_start`: add two points to the left and right
/// borders corresponding to the start of the subpath
fn ft_stroker_subpath_start(
    stroker: &mut PvgFtStroker,
    start_angle: PvgFtAngle,
    line_length: PvgFtFixed,
) -> PvgFtError {
    let mut delta = PvgFtVector::default();

    pvg_ft_vector_from_polar(
        &mut delta,
        stroker.radius,
        start_angle.wrapping_add(PVG_FT_ANGLE_PI2),
    );

    let mut point = PvgFtVector {
        x: stroker.center.x.wrapping_add(delta.x),
        y: stroker.center.y.wrapping_add(delta.y),
    };

    let mut error = ft_stroke_border_moveto(&mut stroker.borders[0], &point);
    if error != 0 {
        return error;
    }

    point.x = stroker.center.x.wrapping_sub(delta.x);
    point.y = stroker.center.y.wrapping_sub(delta.y);

    error = ft_stroke_border_moveto(&mut stroker.borders[1], &point);

    /* save angle, position, and line length for last join */
    /* (line_length is zero for curves)                    */
    stroker.subpath_angle = start_angle;
    stroker.first_point = false;
    stroker.subpath_line_length = line_length;

    error
}

/// `PVG_FT_Stroker_LineTo`
pub fn pvg_ft_stroker_line_to(stroker: &mut PvgFtStroker, to: &PvgFtVector) -> PvgFtError {
    let mut error;
    let mut delta = PvgFtVector {
        x: to.x.wrapping_sub(stroker.center.x),
        y: to.y.wrapping_sub(stroker.center.y),
    };

    /* a zero-length lineto is a no-op; avoid creating a spurious corner */
    if delta.x == 0 && delta.y == 0 {
        return 0;
    }

    /* compute length of line */
    let line_length = pvg_ft_vector_length(&delta);

    let angle = pvg_ft_atan2(delta.x, delta.y);
    pvg_ft_vector_from_polar(
        &mut delta,
        stroker.radius,
        angle.wrapping_add(PVG_FT_ANGLE_PI2),
    );

    /* process corner if necessary */
    if stroker.first_point {
        /* This is the first segment of a subpath.  We need to     */
        /* add a point to each border at their respective starting */
        /* point locations.                                        */
        error = ft_stroker_subpath_start(stroker, angle, line_length);
        if error != 0 {
            return error;
        }
    } else {
        /* process the current corner */
        stroker.angle_out = angle;
        error = ft_stroker_process_corner(stroker, line_length);
        if error != 0 {
            return error;
        }
    }

    /* now add a line segment to both the `inside' and `outside' paths */
    for border in stroker.borders.iter_mut() {
        let point = PvgFtVector {
            x: to.x.wrapping_add(delta.x),
            y: to.y.wrapping_add(delta.y),
        };

        /* the ends of lineto borders are movable */
        error = ft_stroke_border_lineto(border, &point, true);
        if error != 0 {
            return error;
        }

        delta.x = delta.x.wrapping_neg();
        delta.y = delta.y.wrapping_neg();
    }

    stroker.angle_in = angle;
    stroker.center = *to;
    stroker.line_length = line_length;

    0
}

/// `PVG_FT_Stroker_ConicTo`
pub fn pvg_ft_stroker_conic_to(
    stroker: &mut PvgFtStroker,
    control: &PvgFtVector,
    to: &PvgFtVector,
) -> PvgFtError {
    let mut error = 0;
    let mut bez_stack = [PvgFtVector::default(); 34];
    let limit = 30usize;
    let mut first_arc = true;

    /* if all control points are coincident, this is a no-op; */
    /* avoid creating a spurious corner                       */
    if pvg_ft_is_small(stroker.center.x.wrapping_sub(control.x))
        && pvg_ft_is_small(stroker.center.y.wrapping_sub(control.y))
        && pvg_ft_is_small(control.x.wrapping_sub(to.x))
        && pvg_ft_is_small(control.y.wrapping_sub(to.y))
    {
        stroker.center = *to;
        return 0;
    }

    let mut arc: isize = 0;
    bez_stack[0] = *to;
    bez_stack[1] = *control;
    bez_stack[2] = stroker.center;

    while arc >= 0 {
        let a = arc as usize;
        let mut angle_in;
        let mut angle_out;

        /* initialize with current direction */
        angle_in = stroker.angle_in;
        angle_out = stroker.angle_in;

        if a < limit && !ft_conic_is_small_enough(&bez_stack[a..], &mut angle_in, &mut angle_out) {
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
                error = ft_stroker_subpath_start(stroker, angle_in, 0);
            } else {
                stroker.angle_out = angle_in;
                error = ft_stroker_process_corner(stroker, 0);
            }
        } else if ft_pos_abs(pvg_ft_angle_diff(stroker.angle_in, angle_in))
            > PVG_FT_SMALL_CONIC_THRESHOLD / 4
        {
            /* if the deviation from one arc to the next is too great, */
            /* add a round corner                                      */
            stroker.center = bez_stack[a + 2];
            stroker.angle_out = angle_in;
            stroker.line_join = PvgFtStrokerLineJoin::Round;

            error = ft_stroker_process_corner(stroker, 0);

            /* reinstate line join style */
            stroker.line_join = stroker.line_join_saved;
        }

        if error != 0 {
            return error;
        }

        /* the arc's angle is small enough; we can add it directly to each */
        /* border                                                          */
        {
            let mut ctrl = PvgFtVector::default();
            let mut end = PvgFtVector::default();
            let mut alpha0: PvgFtAngle = 0;

            let theta = pvg_ft_angle_diff(angle_in, angle_out) / 2;
            let phi = angle_in.wrapping_add(theta);
            let length = pvg_ft_div_fix(stroker.radius, pvg_ft_cos(theta));

            /* compute direction of original arc */
            if stroker.handle_wide_strokes {
                alpha0 = pvg_ft_atan2(
                    bez_stack[a].x.wrapping_sub(bez_stack[a + 2].x),
                    bez_stack[a].y.wrapping_sub(bez_stack[a + 2].y),
                );
            }

            for side in 0..=1 {
                let rotate = pvg_ft_side_to_rotate(side);

                /* compute control point */
                pvg_ft_vector_from_polar(&mut ctrl, length, phi.wrapping_add(rotate));
                ctrl.x = ctrl.x.wrapping_add(bez_stack[a + 1].x);
                ctrl.y = ctrl.y.wrapping_add(bez_stack[a + 1].y);

                /* compute end point */
                pvg_ft_vector_from_polar(&mut end, stroker.radius, angle_out.wrapping_add(rotate));
                end.x = end.x.wrapping_add(bez_stack[a].x);
                end.y = end.y.wrapping_add(bez_stack[a].y);

                let border = &mut stroker.borders[side as usize];

                if stroker.handle_wide_strokes {
                    /* determine whether the border radius is greater than the
                     */
                    /* radius of curvature of the original arc */
                    let start = border.last_point();

                    let alpha1 =
                        pvg_ft_atan2(end.x.wrapping_sub(start.x), end.y.wrapping_sub(start.y));

                    /* is the direction of the border arc opposite to */
                    /* that of the original arc? */
                    if ft_pos_abs(pvg_ft_angle_diff(alpha0, alpha1)) > PVG_FT_ANGLE_PI / 2 {
                        let mut delta = PvgFtVector::default();

                        /* use the sine rule to find the intersection point */
                        let beta = pvg_ft_atan2(
                            bez_stack[a + 2].x.wrapping_sub(start.x),
                            bez_stack[a + 2].y.wrapping_sub(start.y),
                        );
                        let gamma = pvg_ft_atan2(
                            bez_stack[a].x.wrapping_sub(end.x),
                            bez_stack[a].y.wrapping_sub(end.y),
                        );

                        let bvec = PvgFtVector {
                            x: end.x.wrapping_sub(start.x),
                            y: end.y.wrapping_sub(start.y),
                        };

                        let blen = pvg_ft_vector_length(&bvec);

                        let sin_a = ft_pos_abs(pvg_ft_sin(alpha1.wrapping_sub(gamma)));
                        let sin_b = ft_pos_abs(pvg_ft_sin(beta.wrapping_sub(gamma)));

                        let alen = pvg_ft_mul_div(blen, sin_a, sin_b);

                        pvg_ft_vector_from_polar(&mut delta, alen, beta);
                        delta.x = delta.x.wrapping_add(start.x);
                        delta.y = delta.y.wrapping_add(start.y);

                        /* circumnavigate the negative sector backwards */
                        border.movable = false;
                        error = ft_stroke_border_lineto(border, &delta, false);
                        if error != 0 {
                            return error;
                        }
                        error = ft_stroke_border_lineto(border, &end, false);
                        if error != 0 {
                            return error;
                        }
                        error = ft_stroke_border_conicto(border, &ctrl, &start);
                        if error != 0 {
                            return error;
                        }
                        /* and then move to the endpoint */
                        error = ft_stroke_border_lineto(border, &end, false);
                        if error != 0 {
                            return error;
                        }

                        continue;
                    }

                    /* else fall through */
                }
                /* simply add an arc */
                error = ft_stroke_border_conicto(border, &ctrl, &end);
                if error != 0 {
                    return error;
                }
            }
        }

        arc -= 2;

        stroker.angle_in = angle_out;
    }

    stroker.center = *to;
    stroker.line_length = 0;

    error
}

/// `PVG_FT_Stroker_CubicTo`
pub fn pvg_ft_stroker_cubic_to(
    stroker: &mut PvgFtStroker,
    control1: &PvgFtVector,
    control2: &PvgFtVector,
    to: &PvgFtVector,
) -> PvgFtError {
    let mut error = 0;
    let mut bez_stack = [PvgFtVector::default(); 37];
    let limit = 32usize;
    let mut first_arc = true;

    /* if all control points are coincident, this is a no-op; */
    /* avoid creating a spurious corner */
    if pvg_ft_is_small(stroker.center.x.wrapping_sub(control1.x))
        && pvg_ft_is_small(stroker.center.y.wrapping_sub(control1.y))
        && pvg_ft_is_small(control1.x.wrapping_sub(control2.x))
        && pvg_ft_is_small(control1.y.wrapping_sub(control2.y))
        && pvg_ft_is_small(control2.x.wrapping_sub(to.x))
        && pvg_ft_is_small(control2.y.wrapping_sub(to.y))
    {
        stroker.center = *to;
        return 0;
    }

    let mut arc: isize = 0;
    bez_stack[0] = *to;
    bez_stack[1] = *control2;
    bez_stack[2] = *control1;
    bez_stack[3] = stroker.center;

    while arc >= 0 {
        let a = arc as usize;
        let mut angle_in;
        let mut angle_mid;
        let mut angle_out;

        /* initialize with current direction */
        angle_in = stroker.angle_in;
        angle_out = stroker.angle_in;
        angle_mid = stroker.angle_in;

        if a < limit
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
                error = ft_stroker_subpath_start(stroker, angle_in, 0);
            } else {
                stroker.angle_out = angle_in;
                error = ft_stroker_process_corner(stroker, 0);
            }
        } else if ft_pos_abs(pvg_ft_angle_diff(stroker.angle_in, angle_in))
            > PVG_FT_SMALL_CUBIC_THRESHOLD / 4
        {
            /* if the deviation from one arc to the next is too great, */
            /* add a round corner                                      */
            stroker.center = bez_stack[a + 3];
            stroker.angle_out = angle_in;
            stroker.line_join = PvgFtStrokerLineJoin::Round;

            error = ft_stroker_process_corner(stroker, 0);

            /* reinstate line join style */
            stroker.line_join = stroker.line_join_saved;
        }

        if error != 0 {
            return error;
        }

        /* the arc's angle is small enough; we can add it directly to each */
        /* border                                                          */
        {
            let mut ctrl1 = PvgFtVector::default();
            let mut ctrl2 = PvgFtVector::default();
            let mut end = PvgFtVector::default();
            let mut alpha0: PvgFtAngle = 0;

            let theta1 = pvg_ft_angle_diff(angle_in, angle_mid) / 2;
            let theta2 = pvg_ft_angle_diff(angle_mid, angle_out) / 2;
            let phi1 = ft_angle_mean(angle_in, angle_mid);
            let phi2 = ft_angle_mean(angle_mid, angle_out);
            let length1 = pvg_ft_div_fix(stroker.radius, pvg_ft_cos(theta1));
            let length2 = pvg_ft_div_fix(stroker.radius, pvg_ft_cos(theta2));

            /* compute direction of original arc */
            if stroker.handle_wide_strokes {
                alpha0 = pvg_ft_atan2(
                    bez_stack[a].x.wrapping_sub(bez_stack[a + 3].x),
                    bez_stack[a].y.wrapping_sub(bez_stack[a + 3].y),
                );
            }

            for side in 0..=1 {
                let rotate = pvg_ft_side_to_rotate(side);

                /* compute control points */
                pvg_ft_vector_from_polar(&mut ctrl1, length1, phi1.wrapping_add(rotate));
                ctrl1.x = ctrl1.x.wrapping_add(bez_stack[a + 2].x);
                ctrl1.y = ctrl1.y.wrapping_add(bez_stack[a + 2].y);

                pvg_ft_vector_from_polar(&mut ctrl2, length2, phi2.wrapping_add(rotate));
                ctrl2.x = ctrl2.x.wrapping_add(bez_stack[a + 1].x);
                ctrl2.y = ctrl2.y.wrapping_add(bez_stack[a + 1].y);

                /* compute end point */
                pvg_ft_vector_from_polar(&mut end, stroker.radius, angle_out.wrapping_add(rotate));
                end.x = end.x.wrapping_add(bez_stack[a].x);
                end.y = end.y.wrapping_add(bez_stack[a].y);

                let border = &mut stroker.borders[side as usize];

                if stroker.handle_wide_strokes {
                    /* determine whether the border radius is greater than the
                     */
                    /* radius of curvature of the original arc */
                    let start = border.last_point();

                    let alpha1 =
                        pvg_ft_atan2(end.x.wrapping_sub(start.x), end.y.wrapping_sub(start.y));

                    /* is the direction of the border arc opposite to */
                    /* that of the original arc? */
                    if ft_pos_abs(pvg_ft_angle_diff(alpha0, alpha1)) > PVG_FT_ANGLE_PI / 2 {
                        let mut delta = PvgFtVector::default();

                        /* use the sine rule to find the intersection point */
                        let beta = pvg_ft_atan2(
                            bez_stack[a + 3].x.wrapping_sub(start.x),
                            bez_stack[a + 3].y.wrapping_sub(start.y),
                        );
                        let gamma = pvg_ft_atan2(
                            bez_stack[a].x.wrapping_sub(end.x),
                            bez_stack[a].y.wrapping_sub(end.y),
                        );

                        let bvec = PvgFtVector {
                            x: end.x.wrapping_sub(start.x),
                            y: end.y.wrapping_sub(start.y),
                        };

                        let blen = pvg_ft_vector_length(&bvec);

                        let sin_a = ft_pos_abs(pvg_ft_sin(alpha1.wrapping_sub(gamma)));
                        let sin_b = ft_pos_abs(pvg_ft_sin(beta.wrapping_sub(gamma)));

                        let alen = pvg_ft_mul_div(blen, sin_a, sin_b);

                        pvg_ft_vector_from_polar(&mut delta, alen, beta);
                        delta.x = delta.x.wrapping_add(start.x);
                        delta.y = delta.y.wrapping_add(start.y);

                        /* circumnavigate the negative sector backwards */
                        border.movable = false;
                        error = ft_stroke_border_lineto(border, &delta, false);
                        if error != 0 {
                            return error;
                        }
                        error = ft_stroke_border_lineto(border, &end, false);
                        if error != 0 {
                            return error;
                        }
                        error = ft_stroke_border_cubicto(border, &ctrl2, &ctrl1, &start);
                        if error != 0 {
                            return error;
                        }
                        /* and then move to the endpoint */
                        error = ft_stroke_border_lineto(border, &end, false);
                        if error != 0 {
                            return error;
                        }

                        continue;
                    }

                    /* else fall through */
                }

                /* simply add an arc */
                error = ft_stroke_border_cubicto(border, &ctrl1, &ctrl2, &end);
                if error != 0 {
                    return error;
                }
            }
        }

        arc -= 3;

        stroker.angle_in = angle_out;
    }

    stroker.center = *to;
    stroker.line_length = 0;

    error
}

/// `PVG_FT_Stroker_BeginSubPath`
pub fn pvg_ft_stroker_begin_sub_path(
    stroker: &mut PvgFtStroker,
    to: &PvgFtVector,
    open: bool,
) -> PvgFtError {
    /* We cannot process the first point, because there is not enough      */
    /* information regarding its corner/cap.  The latter will be processed */
    /* in the `PVG_FT_Stroker_EndSubPath' routine.                             */
    /*                                                                     */
    stroker.first_point = true;
    stroker.center = *to;
    stroker.subpath_open = open;

    /* Determine if we need to check whether the border radius is greater */
    /* than the radius of curvature of a curve, to handle this case       */
    /* specially.  This is only required if bevel joins or butt caps may  */
    /* be created, because round & miter joins and round & square caps    */
    /* cover the negative sector created with wide strokes.               */
    stroker.handle_wide_strokes = stroker.line_join != PvgFtStrokerLineJoin::Round
        || (stroker.subpath_open && stroker.line_cap == PvgFtStrokerLineCap::Butt);

    /* record the subpath start point for each border */
    stroker.subpath_start = *to;

    stroker.angle_in = 0;

    0
}

/// `ft_stroker_add_reverse_left`
fn ft_stroker_add_reverse_left(stroker: &mut PvgFtStroker, open: bool) -> PvgFtError {
    let [right, left] = &mut stroker.borders;
    let mut error = 0;

    /* assert(left->start >= 0); */

    let start = left.start.max(0) as usize;
    let new_points = left.num_points() as i64 - left.start as i64;
    if new_points > 0 {
        error = ft_stroke_border_grow(right, new_points as u32);
        if error == 0 {
            for i in (start..left.points.len()).rev() {
                let mut tag = left.tags[i];

                if open {
                    tag &= !PVG_FT_STROKE_TAG_BEGIN_END;
                } else {
                    let ttag = tag & PVG_FT_STROKE_TAG_BEGIN_END;

                    /* switch begin/end tags if necessary */
                    if ttag == PVG_FT_STROKE_TAG_BEGIN || ttag == PVG_FT_STROKE_TAG_END {
                        tag ^= PVG_FT_STROKE_TAG_BEGIN_END;
                    }
                }

                right.points.push(left.points[i]);
                right.tags.push(tag);
            }

            left.set_num_points(start as u32);

            right.movable = false;
            left.movable = false;
        }
    }

    error
}

/// `PVG_FT_Stroker_EndSubPath`: there's a lot of magic in this function!
pub fn pvg_ft_stroker_end_sub_path(stroker: &mut PvgFtStroker) -> PvgFtError {
    let mut error;

    if stroker.subpath_open {
        /* All right, this is an opened path, we need to add a cap between */
        /* right & left, add the reverse of left, then add a final cap     */
        /* between left & right.                                           */
        error = ft_stroker_cap(stroker, stroker.angle_in, 0);
        if error != 0 {
            return error;
        }

        /* add reversed points from `left' to `right' */
        error = ft_stroker_add_reverse_left(stroker, true);
        if error != 0 {
            return error;
        }

        /* now add the final cap */
        stroker.center = stroker.subpath_start;
        error = ft_stroker_cap(
            stroker,
            stroker.subpath_angle.wrapping_add(PVG_FT_ANGLE_PI),
            0,
        );
        if error != 0 {
            return error;
        }

        /* Now end the right subpath accordingly.  The left one is */
        /* rewind and doesn't need further processing.             */
        ft_stroke_border_close(&mut stroker.borders[0], false);
    } else {
        /* close the path if needed */
        if stroker.center.x != stroker.subpath_start.x
            || stroker.center.y != stroker.subpath_start.y
        {
            let start = stroker.subpath_start;
            error = pvg_ft_stroker_line_to(stroker, &start);
            if error != 0 {
                return error;
            }
        }

        /* process the corner */
        stroker.angle_out = stroker.subpath_angle;
        let turn = pvg_ft_angle_diff(stroker.angle_in, stroker.angle_out);

        /* no specific corner processing is required if the turn is 0 */
        if turn != 0 {
            /* when we turn to the right, the inside side is 0 */
            let mut inside_side = 0;

            /* otherwise, the inside side is 1 */
            if turn < 0 {
                inside_side = 1;
            }

            error = ft_stroker_inside(stroker, inside_side, stroker.subpath_line_length);
            if error != 0 {
                return error;
            }

            /* process the outside side */
            error = ft_stroker_outside(stroker, 1 - inside_side, stroker.subpath_line_length);
            if error != 0 {
                return error;
            }
        }

        /* then end our two subpaths */
        ft_stroke_border_close(&mut stroker.borders[0], false);
        ft_stroke_border_close(&mut stroker.borders[1], true);
    }

    0
}

/// `PVG_FT_Stroker_GetBorderCounts`
pub fn pvg_ft_stroker_get_border_counts(
    stroker: &mut PvgFtStroker,
    border: PvgFtStrokerBorder,
    anum_points: &mut PvgFtUInt,
    anum_contours: &mut PvgFtUInt,
) -> PvgFtError {
    ft_stroke_border_get_counts(
        &mut stroker.borders[border as usize],
        anum_points,
        anum_contours,
    )
}

/// `PVG_FT_Stroker_GetCounts`
pub fn pvg_ft_stroker_get_counts(
    stroker: &mut PvgFtStroker,
    anum_points: &mut PvgFtUInt,
    anum_contours: &mut PvgFtUInt,
) -> PvgFtError {
    let mut count1 = 0;
    let mut count2 = 0;
    let mut count3 = 0;
    let mut count4 = 0;
    let mut num_points = 0;
    let mut num_contours = 0;

    let mut error = ft_stroke_border_get_counts(&mut stroker.borders[0], &mut count1, &mut count2);
    if error == 0 {
        error = ft_stroke_border_get_counts(&mut stroker.borders[1], &mut count3, &mut count4);
        if error == 0 {
            num_points = count1 + count3;
            num_contours = count2 + count4;
        }
    }

    *anum_points = num_points;
    *anum_contours = num_contours;
    error
}

/// `PVG_FT_Stroker_ExportBorder`
pub fn pvg_ft_stroker_export_border(
    stroker: &PvgFtStroker,
    border: PvgFtStrokerBorder,
    outline: &mut PvgFtOutline,
) {
    let sborder = &stroker.borders[border as usize];

    if sborder.valid {
        ft_stroke_border_export(sborder, outline);
    }
}

/// `PVG_FT_Stroker_Export`
pub fn pvg_ft_stroker_export(stroker: &PvgFtStroker, outline: &mut PvgFtOutline) {
    pvg_ft_stroker_export_border(stroker, PvgFtStrokerBorder::Left, outline);
    pvg_ft_stroker_export_border(stroker, PvgFtStrokerBorder::Right, outline);
}

/// `PVG_FT_Stroker_ParseOutline`: very similar to
/// `PVG_FT_Outline_Decompose`, except that we do support opened paths,
/// and do not scale the outline.
pub fn pvg_ft_stroker_parse_outline(
    stroker: &mut PvgFtStroker,
    outline: &PvgFtOutline,
) -> PvgFtError {
    let points = &outline.points;
    let tags = &outline.tags;
    let pt = |i: i64| points.get(i as usize).copied().unwrap_or_default();
    let tg = |i: i64| tags.get(i as usize).copied().unwrap_or(0);

    pvg_ft_stroker_rewind(stroker);

    let mut first: u32 = 0;

    for n in 0..outline.n_contours.max(0) as usize {
        let last = outline.contours.get(n).copied().unwrap_or(0) as u32; /* index of last point in contour */
        let mut limit = last as i64;

        /* skip empty points; we don't stroke these */
        if last <= first {
            first = last.wrapping_add(1);
            continue;
        }

        let mut v_start = pt(first as i64);
        let v_last = pt(last as i64);

        let mut v_control;

        let mut point = first as i64;
        let mut tagi = first as i64;
        let mut tag = pvg_ft_curve_tag(tg(tagi));

        /* A contour cannot start with a cubic control point! */
        if tag == PVG_FT_CURVE_TAG_CUBIC {
            return -2; /* PVG_FT_THROW( Invalid_Outline ) */
        }

        /* check first point to determine origin */
        if tag == PVG_FT_CURVE_TAG_CONIC {
            /* First point is conic control.  Yes, this happens. */
            if pvg_ft_curve_tag(tg(last as i64)) == PVG_FT_CURVE_TAG_ON {
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
            tagi -= 1;
        }

        let open = outline.contours_flag.get(n).copied().unwrap_or(0) != 0;
        let mut error = pvg_ft_stroker_begin_sub_path(stroker, &v_start, open);
        if error != 0 {
            return error;
        }

        let mut closed = false;
        'contour: while point < limit {
            point += 1;
            tagi += 1;

            tag = pvg_ft_curve_tag(tg(tagi));
            match tag {
                PVG_FT_CURVE_TAG_ON => {
                    /* emit a single line_to */
                    let vec = pt(point);

                    error = pvg_ft_stroker_line_to(stroker, &vec);
                    if error != 0 {
                        return error;
                    }
                    continue;
                }

                PVG_FT_CURVE_TAG_CONIC => {
                    /* consume conic arcs */
                    v_control = pt(point);

                    /* Do_Conic: */
                    loop {
                        if point < limit {
                            point += 1;
                            tagi += 1;
                            tag = pvg_ft_curve_tag(tg(tagi));

                            let vec = pt(point);

                            if tag == PVG_FT_CURVE_TAG_ON {
                                error = pvg_ft_stroker_conic_to(stroker, &v_control, &vec);
                                if error != 0 {
                                    return error;
                                }
                                continue 'contour;
                            }

                            if tag != PVG_FT_CURVE_TAG_CONIC {
                                return -2; /* PVG_FT_THROW( Invalid_Outline ) */
                            }

                            let v_middle = PvgFtVector {
                                x: (v_control.x.wrapping_add(vec.x)) / 2,
                                y: (v_control.y.wrapping_add(vec.y)) / 2,
                            };

                            error = pvg_ft_stroker_conic_to(stroker, &v_control, &v_middle);
                            if error != 0 {
                                return error;
                            }

                            v_control = vec;
                            continue;
                        }

                        error = pvg_ft_stroker_conic_to(stroker, &v_control, &v_start);
                        closed = true;
                        break 'contour;
                    }
                }

                _ => {
                    /* PVG_FT_CURVE_TAG_CUBIC */
                    if point + 1 > limit || pvg_ft_curve_tag(tg(tagi + 1)) != PVG_FT_CURVE_TAG_CUBIC
                    {
                        return -2; /* PVG_FT_THROW( Invalid_Outline ) */
                    }

                    point += 2;
                    tagi += 2;

                    let vec1 = pt(point - 2);
                    let vec2 = pt(point - 1);

                    if point <= limit {
                        let vec = pt(point);

                        error = pvg_ft_stroker_cubic_to(stroker, &vec1, &vec2, &vec);
                        if error != 0 {
                            return error;
                        }
                        continue;
                    }

                    error = pvg_ft_stroker_cubic_to(stroker, &vec1, &vec2, &v_start);
                    closed = true;
                    break 'contour;
                }
            }
        }
        let _ = closed;

        /* Close: */
        if error != 0 {
            return error;
        }

        if stroker.first_point {
            stroker.subpath_open = true;
            error = ft_stroker_subpath_start(stroker, 0, 0);
            if error != 0 {
                return error;
            }
        }

        error = pvg_ft_stroker_end_sub_path(stroker);
        if error != 0 {
            return error;
        }

        first = last.wrapping_add(1);
    }

    0
}
