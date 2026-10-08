// Rust translation of src/base/ftoutln.c and include/freetype/ftoutln.h
// from FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType outline management (body).
//!
//! The `FT_Outline_Funcs` interface with its `user` pointer becomes the
//! [`FtOutlineFuncs`] trait, implemented by the callers' state.

use super::super::fttypes::*;
use super::ftcalc::*;
use super::ftobjs::{ft_lookup_renderer, FtLibraryRec};
use crate::freetype::ftimage::FtRasterParams;

/// `FT_Outline_Funcs` (without `shift` and `delta`, which are passed to
/// [`ft_outline_decompose`])
pub trait FtOutlineFuncs {
    /// `move_to`
    fn move_to(&mut self, to: &FtVector) -> FtError;
    /// `line_to`
    fn line_to(&mut self, to: &FtVector) -> FtError;
    /// `conic_to`
    fn conic_to(&mut self, control: &FtVector, to: &FtVector) -> FtError;
    /// `cubic_to`
    fn cubic_to(&mut self, control1: &FtVector, control2: &FtVector, to: &FtVector) -> FtError;
}

/// `FT_Orientation`
pub type FtOrientation = u32;
pub const FT_ORIENTATION_TRUETYPE: FtOrientation = 0;
pub const FT_ORIENTATION_POSTSCRIPT: FtOrientation = 1;
pub const FT_ORIENTATION_FILL_RIGHT: FtOrientation = FT_ORIENTATION_TRUETYPE;
pub const FT_ORIENTATION_FILL_LEFT: FtOrientation = FT_ORIENTATION_POSTSCRIPT;
pub const FT_ORIENTATION_NONE: FtOrientation = 2;

/// `FT_Outline_Decompose`
pub fn ft_outline_decompose(
    outline: &FtOutline,
    func_interface: &mut dyn FtOutlineFuncs,
    shift: i32,
    delta: FtPos,
) -> FtResult<()> {
    let scaled = |x: FtPos| -> FtPos { x.wrapping_mul(1i64 << shift).wrapping_sub(delta) };

    let mut v_last: FtVector;
    let mut v_control: FtVector;
    let mut v_start: FtVector;

    let mut error: FtError;

    let mut first: i32; /* index of first point in contour */
    let mut last: i32; /* index of last point in contour  */

    let mut tag: u8; /* current point's state           */

    let points = &outline.points;
    let tags = &outline.tags;

    macro_rules! invalid_outline {
        () => {
            return Err(FT_ERR_INVALID_OUTLINE)
        };
    }
    macro_rules! check {
        ($e:expr) => {{
            error = $e;
            if error != 0 {
                return Err(error);
            }
        }};
    }

    last = -1;
    for n in 0..outline.n_contours.max(0) as usize {
        first = last + 1;
        last = outline.contours[n] as i32;
        if last < first {
            invalid_outline!();
        }

        let mut limit = last as isize;

        v_start = points[first as usize];
        v_start.x = scaled(v_start.x);
        v_start.y = scaled(v_start.y);

        v_last = points[last as usize];
        v_last.x = scaled(v_last.x);
        v_last.y = scaled(v_last.y);

        v_control = v_start;

        let mut point = first as isize;
        tag = ft_curve_tag(tags[first as usize]);

        /* A contour cannot start with a cubic control point! */
        if tag == FT_CURVE_TAG_CUBIC {
            invalid_outline!();
        }

        /* check first point to determine origin */
        if tag == FT_CURVE_TAG_CONIC {
            /* first point is conic control.  Yes, this happens. */
            if ft_curve_tag(tags[last as usize]) == FT_CURVE_TAG_ON {
                /* start at last point if it is on the curve */
                v_start = v_last;
                limit -= 1;
            } else {
                /* if both first and last points are conic,         */
                /* start at their middle and record its position    */
                /* for closure                                      */
                v_start.x = (v_start.x.wrapping_add(v_last.x)) / 2;
                v_start.y = (v_start.y.wrapping_add(v_last.y)) / 2;

                /* v_last = v_start; */
            }
            point -= 1;
        }

        check!(func_interface.move_to(&v_start));

        let mut closed = false;
        'contour: while point < limit {
            point += 1;
            tag = ft_curve_tag(tags[point as usize]);
            match tag {
                FT_CURVE_TAG_ON => {
                    /* emit a single line_to */
                    let vec = FtVector {
                        x: scaled(points[point as usize].x),
                        y: scaled(points[point as usize].y),
                    };

                    check!(func_interface.line_to(&vec));
                    continue;
                }

                FT_CURVE_TAG_CONIC => {
                    /* consume conic arcs */
                    v_control.x = scaled(points[point as usize].x);
                    v_control.y = scaled(points[point as usize].y);

                    loop {
                        /* Do_Conic: */
                        if point < limit {
                            point += 1;
                            tag = ft_curve_tag(tags[point as usize]);

                            let vec = FtVector {
                                x: scaled(points[point as usize].x),
                                y: scaled(points[point as usize].y),
                            };

                            if tag == FT_CURVE_TAG_ON {
                                check!(func_interface.conic_to(&v_control, &vec));
                                continue 'contour;
                            }

                            if tag != FT_CURVE_TAG_CONIC {
                                invalid_outline!();
                            }

                            let v_middle = FtVector {
                                x: (v_control.x.wrapping_add(vec.x)) / 2,
                                y: (v_control.y.wrapping_add(vec.y)) / 2,
                            };

                            check!(func_interface.conic_to(&v_control, &v_middle));

                            v_control = vec;
                            continue;
                        }

                        error = func_interface.conic_to(&v_control, &v_start);
                        closed = true;
                        break 'contour;
                    }
                }

                _ => {
                    /* FT_CURVE_TAG_CUBIC */
                    if point + 1 > limit
                        || ft_curve_tag(tags[(point + 1) as usize]) != FT_CURVE_TAG_CUBIC
                    {
                        invalid_outline!();
                    }

                    point += 2;

                    let vec1 = FtVector {
                        x: scaled(points[(point - 2) as usize].x),
                        y: scaled(points[(point - 2) as usize].y),
                    };
                    let vec2 = FtVector {
                        x: scaled(points[(point - 1) as usize].x),
                        y: scaled(points[(point - 1) as usize].y),
                    };

                    if point <= limit {
                        let vec = FtVector {
                            x: scaled(points[point as usize].x),
                            y: scaled(points[point as usize].y),
                        };

                        check!(func_interface.cubic_to(&vec1, &vec2, &vec));
                        continue;
                    }

                    error = func_interface.cubic_to(&vec1, &vec2, &v_start);
                    closed = true;
                    break 'contour;
                }
            }
        }

        if !closed {
            /* close the contour with a line segment */
            error = func_interface.line_to(&v_start);
        }

        /* Close: */
        if error != 0 {
            return Err(error);
        }
    }

    Ok(())
}

/// `FT_Outline_New`
pub fn ft_outline_new(num_points: FtUInt, num_contours: FtInt) -> FtResult<FtOutline> {
    if num_contours < 0 || num_contours as FtUInt > num_points {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    if num_points > FT_OUTLINE_POINTS_MAX as FtUInt {
        return Err(FT_ERR_ARRAY_TOO_LARGE);
    }

    use super::ftmemory::ft_new_array;
    let points = ft_new_array(num_points as FtLong)?;
    let tags = ft_new_array(num_points as FtLong)?;
    let contours = ft_new_array(num_contours as FtLong)?;

    Ok(FtOutline {
        n_points: num_points as i16,
        n_contours: num_contours as i16,
        points,
        tags,
        contours,
        flags: FT_OUTLINE_OWNER,
    })
}

/// `FT_Outline_Check`
pub fn ft_outline_check(outline: &FtOutline) -> FtResult<()> {
    let n_points = outline.n_points as i32;
    let n_contours = outline.n_contours as i32;

    /* empty glyph? */
    if n_points == 0 && n_contours == 0 {
        return Ok(());
    }

    /* check point and contour counts */
    if n_points <= 0 || n_contours <= 0 {
        return Err(FT_ERR_INVALID_OUTLINE);
    }

    let mut end0 = -1;
    for n in 0..n_contours as usize {
        let end = outline.contours[n] as i32;

        /* note that we don't accept empty contours */
        if end <= end0 || end >= n_points {
            return Err(FT_ERR_INVALID_OUTLINE);
        }

        end0 = end;
    }

    if end0 != n_points - 1 {
        return Err(FT_ERR_INVALID_OUTLINE);
    }

    /* XXX: check the tags array */
    Ok(())
}

/// `FT_Outline_Copy`
pub fn ft_outline_copy(source: &FtOutline, target: &mut FtOutline) -> FtResult<()> {
    if source.n_points != target.n_points || source.n_contours != target.n_contours {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    let np = source.n_points as u16 as usize;
    let nc = source.n_contours as u16 as usize;
    if np != 0 {
        target.points[..np].copy_from_slice(&source.points[..np]);
        target.tags[..np].copy_from_slice(&source.tags[..np]);
    }

    if nc != 0 {
        target.contours[..nc].copy_from_slice(&source.contours[..nc]);
    }

    /* copy all flags, except the `FT_OUTLINE_OWNER' one */
    let is_owner = target.flags & FT_OUTLINE_OWNER;
    target.flags = source.flags;

    target.flags &= !FT_OUTLINE_OWNER;
    target.flags |= is_owner;

    Ok(())
}

/// `FT_Outline_Done`
pub fn ft_outline_done(outline: &mut FtOutline) {
    *outline = FtOutline::default();
}

/// `FT_Outline_Get_CBox`
pub fn ft_outline_get_cbox(outline: &FtOutline) -> FtBBox {
    let (x_min, y_min, x_max, y_max);

    if outline.n_points == 0 {
        x_min = 0;
        y_min = 0;
        x_max = 0;
        y_max = 0;
    } else {
        let n = outline.n_points as u16 as usize;
        let pts = &outline.points[..n];
        let mut xmin = pts[0].x;
        let mut xmax = xmin;
        let mut ymin = pts[0].y;
        let mut ymax = ymin;

        for vec in &pts[1..] {
            let x = vec.x;
            if x < xmin {
                xmin = x;
            }
            if x > xmax {
                xmax = x;
            }

            let y = vec.y;
            if y < ymin {
                ymin = y;
            }
            if y > ymax {
                ymax = y;
            }
        }
        x_min = xmin;
        y_min = ymin;
        x_max = xmax;
        y_max = ymax;
    }

    FtBBox {
        xMin: x_min,
        yMin: y_min,
        xMax: x_max,
        yMax: y_max,
    }
}

/// `FT_Outline_Translate`
pub fn ft_outline_translate(outline: &mut FtOutline, x_offset: FtPos, y_offset: FtPos) {
    let n = outline.n_points as u16 as usize;
    for vec in &mut outline.points[..n] {
        vec.x = add_long(vec.x, x_offset);
        vec.y = add_long(vec.y, y_offset);
    }
}

/// `FT_Outline_Reverse`
pub fn ft_outline_reverse(outline: &mut FtOutline) {
    let mut last: i32 = -1;
    for n in 0..outline.n_contours as u16 as usize {
        /* keep the first contour point as is and swap points around it */
        /* to guarantee that the cubic arches stay valid after reverse  */
        let first = last + 2;
        last = outline.contours[n] as i32;

        /* reverse point table */
        {
            let mut p = first;
            let mut q = last;

            while p < q {
                outline.points.swap(p as usize, q as usize);
                p += 1;
                q -= 1;
            }
        }

        /* reverse tags table */
        {
            let mut p = first;
            let mut q = last;

            while p < q {
                outline.tags.swap(p as usize, q as usize);
                p += 1;
                q -= 1;
            }
        }
    }

    outline.flags ^= FT_OUTLINE_REVERSE_FILL;
}

/// `FT_Outline_Render`
pub fn ft_outline_render(
    library: &FtLibraryRec,
    outline: &FtOutline,
    params: &mut FtRasterParams,
) -> FtResult<()> {
    let cbox = ft_outline_get_cbox(outline);
    if cbox.xMin < -0x1000000
        || cbox.yMin < -0x1000000
        || cbox.xMax > 0x1000000
        || cbox.yMax > 0x1000000
    {
        return Err(FT_ERR_INVALID_OUTLINE);
    }

    let mut renderer = library.cur_renderer;
    let mut node: Option<usize> = None;

    /* preset clip_box for direct mode */
    if params.flags & FT_RASTER_FLAG_DIRECT != 0 && params.flags & FT_RASTER_FLAG_CLIP == 0 {
        params.clip_box.xMin = cbox.xMin >> 6;
        params.clip_box.yMin = cbox.yMin >> 6;
        params.clip_box.xMax = (cbox.xMax + 63) >> 6;
        params.clip_box.yMax = (cbox.yMax + 63) >> 6;
    }

    let mut error = FT_ERR_CANNOT_RENDER_GLYPH;
    while let Some(r) = renderer {
        error = match library.renderer_raster_render(r, outline, params) {
            Ok(()) => 0,
            Err(e) => e,
        };
        if error == 0 || ft_err_neq(error, FT_ERR_CANNOT_RENDER_GLYPH) {
            break;
        }

        /* FT_Err_Cannot_Render_Glyph is returned if the render mode   */
        /* is unsupported by the current renderer for this glyph image */
        /* format                                                      */

        /* now, look for another renderer that supports the same */
        /* format                                                */
        renderer = ft_lookup_renderer(library, FT_GLYPH_FORMAT_OUTLINE, &mut node);
    }

    if error == 0 {
        Ok(())
    } else {
        Err(error)
    }
}

/// `FT_Outline_Get_Bitmap`
pub fn ft_outline_get_bitmap(
    library: &FtLibraryRec,
    outline: &FtOutline,
    abitmap: &mut FtBitmap,
) -> FtResult<()> {
    /* other checks are delayed to `FT_Outline_Render' */

    let mut flags = 0;

    if abitmap.pixel_mode == FT_PIXEL_MODE_GRAY
        || abitmap.pixel_mode == FT_PIXEL_MODE_LCD
        || abitmap.pixel_mode == FT_PIXEL_MODE_LCD_V
    {
        flags |= FT_RASTER_FLAG_AA;
    }

    let mut params = FtRasterParams::new(Some(abitmap), flags);
    ft_outline_render(library, outline, &mut params)
}

/// `FT_Vector_Transform` (documentation is in freetype.h)
pub fn ft_vector_transform(vector: &mut FtVector, matrix: &FtMatrix) {
    let xz = ft_mul_fix(vector.x, matrix.xx).wrapping_add(ft_mul_fix(vector.y, matrix.xy));

    let yz = ft_mul_fix(vector.x, matrix.yx).wrapping_add(ft_mul_fix(vector.y, matrix.yy));

    vector.x = xz;
    vector.y = yz;
}

/// `FT_Outline_Transform`
pub fn ft_outline_transform(outline: &mut FtOutline, matrix: &FtMatrix) {
    let n = outline.n_points as u16 as usize;
    for vec in &mut outline.points[..n] {
        ft_vector_transform(vec, matrix);
    }
}

/// `FT_Outline_Embolden`
pub fn ft_outline_embolden(outline: &mut FtOutline, strength: FtPos) -> FtResult<()> {
    ft_outline_embolden_xy(outline, strength, strength)
}

/// `FT_Outline_EmboldenXY`
pub fn ft_outline_embolden_xy(
    outline: &mut FtOutline,
    mut xstrength: FtPos,
    mut ystrength: FtPos,
) -> FtResult<()> {
    xstrength /= 2;
    ystrength /= 2;
    if xstrength == 0 && ystrength == 0 {
        return Ok(());
    }

    let orientation = ft_outline_get_orientation(outline);
    if orientation == FT_ORIENTATION_NONE {
        if outline.n_contours != 0 {
            return Err(FT_ERR_INVALID_ARGUMENT);
        } else {
            return Ok(());
        }
    }

    let points = &mut outline.points;

    let mut last: i32 = -1;
    for c in 0..outline.n_contours as u16 as usize {
        let mut in_ = FtVector::default();
        let mut out = FtVector::default();
        let mut anchor = FtVector::default();
        let mut shift = FtVector::default();
        let mut l_in: FtFixed;
        let mut l_out: FtFixed;
        let mut l_anchor: FtFixed = 0;
        let mut l: FtFixed;
        let mut q: FtFixed;
        let mut d: FtFixed;

        let first = last + 1;
        last = outline.contours[c] as i32;
        l_in = 0;

        /* Counter j cycles though the points; counter i advances only  */
        /* when points are moved; anchor k marks the first moved point. */
        let mut i = last;
        let mut j = first;
        let mut k: i32 = -1;
        while j != i && i != k {
            let mut skip = false;
            if j != k {
                out.x = points[j as usize].x.wrapping_sub(points[i as usize].x);
                out.y = points[j as usize].y.wrapping_sub(points[i as usize].y);
                l_out = ft_vector_norm_len(&mut out) as FtFixed;

                if l_out == 0 {
                    skip = true;
                }
            } else {
                out = anchor;
                l_out = l_anchor;
            }

            if !skip {
                if l_in != 0 {
                    if k < 0 {
                        k = i;
                        anchor = in_;
                        l_anchor = l_in;
                    }

                    d = ft_mul_fix(in_.x, out.x).wrapping_add(ft_mul_fix(in_.y, out.y));

                    /* shift only if turn is less than ~160 degrees */
                    if d > -0xF000 {
                        d += 0x10000;

                        /* shift components along lateral bisector in proper orientation */
                        shift.x = in_.y.wrapping_add(out.y);
                        shift.y = in_.x.wrapping_add(out.x);

                        if orientation == FT_ORIENTATION_TRUETYPE {
                            shift.x = shift.x.wrapping_neg();
                        } else {
                            shift.y = shift.y.wrapping_neg();
                        }

                        /* restrict shift magnitude to better handle collapsing segments */
                        q = ft_mul_fix(out.x, in_.y).wrapping_sub(ft_mul_fix(out.y, in_.x));
                        if orientation == FT_ORIENTATION_TRUETYPE {
                            q = q.wrapping_neg();
                        }

                        l = l_in.min(l_out);

                        /* non-strict inequalities avoid divide-by-zero when q == l == 0 */
                        if ft_mul_fix(xstrength, q) <= ft_mul_fix(l, d) {
                            shift.x = ft_mul_div(shift.x, xstrength, d);
                        } else {
                            shift.x = ft_mul_div(shift.x, l, q);
                        }

                        if ft_mul_fix(ystrength, q) <= ft_mul_fix(l, d) {
                            shift.y = ft_mul_div(shift.y, ystrength, d);
                        } else {
                            shift.y = ft_mul_div(shift.y, l, q);
                        }
                    } else {
                        shift.x = 0;
                        shift.y = 0;
                    }

                    while i != j {
                        points[i as usize].x = points[i as usize]
                            .x
                            .wrapping_add(xstrength.wrapping_add(shift.x));
                        points[i as usize].y = points[i as usize]
                            .y
                            .wrapping_add(ystrength.wrapping_add(shift.y));
                        i = if i < last { i + 1 } else { first };
                    }
                } else {
                    i = j;
                }

                in_ = out;
                l_in = l_out;
            }

            j = if j < last { j + 1 } else { first };
        }
    }

    Ok(())
}

/// `FT_Outline_Get_Orientation`
pub fn ft_outline_get_orientation(outline: &FtOutline) -> FtOrientation {
    let mut area: FtPos = 0;

    if outline.n_points <= 0 {
        return FT_ORIENTATION_TRUETYPE;
    }

    /* We use the nonzero winding rule to find the orientation.       */
    /* Since glyph outlines behave much more `regular' than arbitrary */
    /* cubic or quadratic curves, this test deals with the polygon    */
    /* only that is spanned up by the control points.                 */

    let cbox = ft_outline_get_cbox(outline);

    /* Handle collapsed outlines to avoid undefined FT_MSB. */
    if cbox.xMin == cbox.xMax || cbox.yMin == cbox.yMax {
        return FT_ORIENTATION_NONE;
    }

    /* Reject values large outlines. */
    if cbox.xMin < -0x1000000
        || cbox.yMin < -0x1000000
        || cbox.xMax > 0x1000000
        || cbox.yMax > 0x1000000
    {
        return FT_ORIENTATION_NONE;
    }

    let mut xshift = ft_msb((ft_abs(cbox.xMax) | ft_abs(cbox.xMin)) as u32) - 14;
    xshift = xshift.max(0);

    let mut yshift = ft_msb((cbox.yMax - cbox.yMin) as u32) - 14;
    yshift = yshift.max(0);

    let points = &outline.points;

    let mut last: i32 = -1;
    for c in 0..outline.n_contours.max(0) as usize {
        let first = last + 1;
        last = outline.contours[c] as i32;

        let mut v_prev = FtVector {
            x: points[last as usize].x >> xshift,
            y: points[last as usize].y >> yshift,
        };

        for n in first..=last {
            let v_cur = FtVector {
                x: points[n as usize].x >> xshift,
                y: points[n as usize].y >> yshift,
            };

            area = add_long(area, mul_long(v_cur.y - v_prev.y, v_cur.x + v_prev.x));

            v_prev = v_cur;
        }
    }

    if area > 0 {
        FT_ORIENTATION_POSTSCRIPT
    } else if area < 0 {
        FT_ORIENTATION_TRUETYPE
    } else {
        FT_ORIENTATION_NONE
    }
}
