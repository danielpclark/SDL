// Rust translation of src/truetype/ttgload.c from FreeType (2.13.2, as
// SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! TrueType Glyph Loader (body).
//!
//! The loader takes the glyph slot's glyph loader and the size's execution
//! context out of the face for the duration of a load and puts them back
//! afterwards.  C's glyph zone (`loader->zone`) points into the glyph
//! loader's arrays; here `tt_hint_glyph` copies the zone's points into
//! the execution context and back.  The `face->access_glyph_frame` & co.
//! function pointers (`TT_Init_Glyph_Loading`) are called directly.

use std::sync::Arc;

use super::super::base::ftcalc::*;
use super::super::base::ftgloadr::{FtGlyphLoaderRec, FtSubGlyphRec};
use super::super::base::ftobjs::*;
use super::super::base::ftoutln::{
    ft_outline_get_cbox, ft_outline_transform, ft_outline_translate,
};
use super::super::base::ftstream::*;
use super::super::fttypes::*;
use super::super::sfnt::ttload::with_stream;
use super::super::sfnt::ttmtx::tt_face_get_metrics;
use super::super::sfnt::ttsbit::tt_face_load_sbit_image;
use super::super::sfnt::ttsvg::tt_face_load_svg_doc;
use super::super::tttypes::*;
use super::ttgxvar::{tt_vary_apply_glyph_deltas, GxOutlineView, GxVaryPhantoms};
use super::ttinterp::*;
use super::ttobjs::*;
use super::ttpload::tt_face_get_location;

/*
 *
 * Simple glyph flags.
 */
const ON_CURVE_POINT: u8 = 0x01; /* same value as FT_CURVE_TAG_ON            */
const X_SHORT_VECTOR: u8 = 0x02;
const Y_SHORT_VECTOR: u8 = 0x04;
const REPEAT_FLAG: u8 = 0x08;
const X_POSITIVE: u8 = 0x10; /* two meanings depending on X_SHORT_VECTOR */
const SAME_X: u8 = 0x10;
const Y_POSITIVE: u8 = 0x20; /* two meanings depending on Y_SHORT_VECTOR */
const SAME_Y: u8 = 0x20;
const OVERLAP_SIMPLE: u8 = 0x40; /* retained as FT_OUTLINE_OVERLAP           */

/*
 *
 * Composite glyph flags.
 */
const ARGS_ARE_WORDS: FtUShort = 0x0001;
const ARGS_ARE_XY_VALUES: FtUShort = 0x0002;
const ROUND_XY_TO_GRID: FtUShort = 0x0004;
const WE_HAVE_A_SCALE: FtUShort = 0x0008;
/* reserved                        0x0010 */
const MORE_COMPONENTS: FtUShort = 0x0020;
const WE_HAVE_AN_XY_SCALE: FtUShort = 0x0040;
const WE_HAVE_A_2X2: FtUShort = 0x0080;
const WE_HAVE_INSTR: FtUShort = 0x0100;
const USE_MY_METRICS: FtUShort = 0x0200;
const OVERLAP_COMPOUND: FtUShort = 0x0400; /* retained as FT_OUTLINE_OVERLAP */
const SCALED_COMPONENT_OFFSET: FtUShort = 0x0800;
#[allow(dead_code)]
const UNSCALED_COMPONENT_OFFSET: FtUShort = 0x1000;

/// `IS_DEFAULT_INSTANCE`
fn is_default_instance(face: &FtFaceRec) -> bool {
    !(ft_is_named_instance(face) || ft_is_variation(face))
}

/// `IS_HINTED`
#[inline]
fn is_hinted(flags: FtULong) -> bool {
    (flags & FT_LOAD_NO_HINTING as FtULong) == 0
}

/// `TT_LoaderRec` (tttypes.h)
#[derive(Debug, Default)]
pub struct TtLoaderRec {
    /// `gloader` (taken from the glyph slot)
    pub gloader: FtGlyphLoaderRec,

    pub load_flags: FtULong,
    pub glyph_index: FtUInt,

    pub byte_len: FtUInt,

    pub n_contours: FtShort,
    pub bbox: FtBBox,
    pub left_bearing: FtInt,
    pub advance: FtInt,
    pub linear: FtInt,
    pub linear_def: bool,
    pub pp1: FtVector,
    pub pp2: FtVector,

    /* the zone where we load our glyphs */
    /* (see `tt_hint_glyph') */
    /// `exec` (taken from the size)
    pub exec: Option<Box<TtExecContextRec>>,
    /// whether `exec` holds the size's arrays (see `tt_load_context`)
    exec_loaded: bool,

    /// `cursor` (an offset into the stream's frame)
    pub cursor: usize,
    /// `limit` (an offset into the stream's frame)
    pub limit: usize,

    /* since version 2.1.8 */
    pub top_bearing: FtInt,
    pub vadvance: FtInt,
    pub pp3: FtVector,
    pub pp4: FtVector,

    /* since version 2.2.1 */
    /// `widthp` (an offset into the `hdmx' table)
    pub widthp: Option<usize>,

    pub ins_pos: FtULong,

    /* since version 2.6.2 */
    /// `composites`: the glyph indices of the composites being loaded, by
    /// nesting level (`None` is C's `(void*)-1`)
    pub composites: Vec<Option<FtUInt>>,

    /* the driver's interpreter version */
    interpreter_version: FtUInt,
}

/// `TT_Get_HMetrics`: Return the horizontal metrics in font units for a
/// given glyph.
pub fn tt_get_hmetrics(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    idx: FtUInt,
) -> (FtShort, FtUShort) {
    tt_face_get_metrics(face, stream, false, idx)
}

/// `TT_Get_VMetrics`: Return the vertical metrics in font units for a
/// given glyph.  See function `tt_loader_set_pp' below for explanations.
pub fn tt_get_vmetrics(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    idx: FtUInt,
    y_max: FtPos,
) -> (FtShort, FtUShort) {
    if face.vertical_info {
        tt_face_get_metrics(face, stream, true, idx)
    } else if face.os2.version != 0xFFFF {
        (
            (face.os2.sTypoAscender as FtPos).wrapping_sub(y_max) as FtShort,
            ft_abs(face.os2.sTypoAscender as FtLong - face.os2.sTypoDescender as FtLong)
                as FtUShort,
        )
    } else {
        (
            (face.horizontal.Ascender as FtPos).wrapping_sub(y_max) as FtShort,
            ft_abs(face.horizontal.Ascender as FtLong - face.horizontal.Descender as FtLong)
                as FtUShort,
        )
    }
}

/// `tt_get_metrics`
fn tt_get_metrics(
    loader: &mut TtLoaderRec,
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    glyph_index: FtUInt,
) -> FtResult<()> {
    /* we must preserve the stream position          */
    /* (which gets altered by the metrics functions) */
    let pos = stream.pos();

    let (left_bearing, advance_width) = tt_get_hmetrics(face, stream, glyph_index);
    let (top_bearing, advance_height) =
        tt_get_vmetrics(face, stream, glyph_index, loader.bbox.yMax);

    stream.seek(pos)?;

    loader.left_bearing = left_bearing as FtInt;
    loader.advance = advance_width as FtInt;
    loader.top_bearing = top_bearing as FtInt;
    loader.vadvance = advance_height as FtInt;

    /* (FT_CONFIG_OPTION_INCREMENTAL: no incremental interface is set) */
    if !loader.linear_def {
        loader.linear_def = true;
        loader.linear = advance_width as FtInt;
    }

    Ok(())
}

/*
 *
 * The following functions are used by default with TrueType fonts.
 * However, they can be replaced by alternatives if we need to support
 * TrueType-compressed formats (like MicroType) in the future.
 *
 */

/// `TT_Access_Glyph_Frame`
fn tt_access_glyph_frame(
    loader: &mut TtLoaderRec,
    stream: &mut FtStreamRec,
    _glyph_index: FtUInt,
    offset: FtULong,
    byte_count: FtUInt,
) -> FtResult<()> {
    /* the following line sets the `error' variable through macros! */
    stream.seek(offset)?;
    stream.enter_frame(byte_count as FtULong)?;

    loader.cursor = 0;
    loader.limit = stream.frame_data().len();

    Ok(())
}

/// `TT_Forget_Glyph_Frame`
fn tt_forget_glyph_frame(stream: &mut FtStreamRec) {
    stream.exit_frame();
}

/// `TT_Load_Glyph_Header`
fn tt_load_glyph_header(loader: &mut TtLoaderRec, stream: &FtStreamRec) -> FtResult<()> {
    let t = stream.frame_data();
    let mut p = loader.cursor;
    let limit = loader.limit;

    if p + 10 > limit {
        return Err(FT_ERR_INVALID_OUTLINE);
    }

    loader.n_contours = ft_next_short(t, &mut p);

    loader.bbox.xMin = ft_next_short(t, &mut p) as FtPos;
    loader.bbox.yMin = ft_next_short(t, &mut p) as FtPos;
    loader.bbox.xMax = ft_next_short(t, &mut p) as FtPos;
    loader.bbox.yMax = ft_next_short(t, &mut p) as FtPos;

    loader.cursor = p;

    Ok(())
}

/// `TT_Load_Simple_Glyph`
fn tt_load_simple_glyph(load: &mut TtLoaderRec, stream: &FtStreamRec) -> FtResult<()> {
    let t = stream.frame_data();
    let mut p = load.cursor;
    let limit = load.limit;
    let n_contours = load.n_contours as FtInt;

    /* check that we can add the contours to the glyph */
    load.gloader.check_points_macro(0, n_contours as FtUInt)?;

    /* check space for contours array + instructions count */
    if n_contours >= 0xFFF || p + 2 * n_contours as usize + 2 > limit {
        return Err(FT_ERR_INVALID_OUTLINE);
    }

    /* reading the contours' endpoints & number of points */
    let mut last: FtShort = -1;
    {
        let cont = load.gloader.current_contours();
        for c in cont.iter_mut().take(n_contours as usize) {
            *c = ft_next_short(t, &mut p);

            if *c <= last {
                return Err(FT_ERR_INVALID_OUTLINE);
            }

            last = *c;
        }
    }

    let n_points = last as FtInt + 1;

    /* note that we will add four phantom points later */
    load.gloader
        .check_points_macro((n_points + 4) as FtUInt, 0)?;

    /* space checked above */
    let n_ins = ft_next_ushort(t, &mut p);

    /* check instructions size */
    if p + n_ins as usize > limit {
        return Err(FT_ERR_TOO_MANY_HINTS);
    }

    /* TT_USE_BYTECODE_INTERPRETER */
    if is_hinted(load.load_flags) {
        let exec = load
            .exec
            .as_mut()
            .expect("hinting without an execution context");

        exec.glyph_ins = Vec::new();
        exec.glyph_size = 0;

        /* we don't trust `maxSizeOfInstructions' in the `maxp' table */
        /* and thus allocate the bytecode array size by ourselves     */
        if n_ins != 0 {
            let mut v = Vec::new();
            if v.try_reserve_exact(n_ins as usize).is_err() {
                return Err(FT_ERR_OUT_OF_MEMORY);
            }
            v.extend_from_slice(&t[p..p + n_ins as usize]);
            exec.glyph_ins = v;
            exec.glyph_size = n_ins as FtUInt;
        }
    }

    p += n_ins as usize;

    /* reading the point tags */
    {
        let flags = load.gloader.current_tags();
        let flag_limit = n_points as usize;
        let mut flag = 0usize;

        while flag < flag_limit {
            if p + 1 > limit {
                return Err(FT_ERR_INVALID_OUTLINE);
            }

            let c = ft_next_byte(t, &mut p);
            flags[flag] = c;
            flag += 1;
            if c & REPEAT_FLAG != 0 {
                if p + 1 > limit {
                    return Err(FT_ERR_INVALID_OUTLINE);
                }

                let count = ft_next_byte(t, &mut p);
                if flag + count as usize > flag_limit {
                    return Err(FT_ERR_INVALID_OUTLINE);
                }

                for _ in 0..count {
                    flags[flag] = c;
                    flag += 1;
                }
            }
        }
    }

    /* retain the overlap flag */
    if n_points != 0 && load.gloader.current_tags()[0] & OVERLAP_SIMPLE != 0 {
        load.gloader.base.outline.flags |= FT_OUTLINE_OVERLAP;
    }

    {
        let start = load.gloader.current_point_start();
        let outline = &mut load.gloader.base.outline;
        let (points, tags) = (&mut outline.points[start..], &mut outline.tags[start..]);

        /* reading the X coordinates */

        let mut x: FtPos = 0;

        for (vec, &f) in points.iter_mut().zip(tags.iter()).take(n_points as usize) {
            let mut delta: FtPos = 0;

            if f & X_SHORT_VECTOR != 0 {
                if p + 1 > limit {
                    return Err(FT_ERR_INVALID_OUTLINE);
                }

                delta = ft_next_byte(t, &mut p) as FtPos;
                if f & X_POSITIVE == 0 {
                    delta = -delta;
                }
            } else if f & SAME_X == 0 {
                if p + 2 > limit {
                    return Err(FT_ERR_INVALID_OUTLINE);
                }

                delta = ft_next_short(t, &mut p) as FtPos;
            }

            x += delta;
            vec.x = x;
        }

        /* reading the Y coordinates */

        let mut y: FtPos = 0;

        for (vec, flag) in points
            .iter_mut()
            .zip(tags.iter_mut())
            .take(n_points as usize)
        {
            let mut delta: FtPos = 0;
            let f = *flag;

            if f & Y_SHORT_VECTOR != 0 {
                if p + 1 > limit {
                    return Err(FT_ERR_INVALID_OUTLINE);
                }

                delta = ft_next_byte(t, &mut p) as FtPos;
                if f & Y_POSITIVE == 0 {
                    delta = -delta;
                }
            } else if f & SAME_Y == 0 {
                if p + 2 > limit {
                    return Err(FT_ERR_INVALID_OUTLINE);
                }

                delta = ft_next_short(t, &mut p) as FtPos;
            }

            y += delta;
            vec.y = y;

            /* the cast is for stupid compilers */
            *flag = f & ON_CURVE_POINT;
        }
    }

    load.gloader.current.n_points = n_points as FtShort;
    load.gloader.current.n_contours = n_contours as FtShort;

    load.cursor = p;

    Ok(())
}

/// `TT_Load_Composite_Glyph`
fn tt_load_composite_glyph(
    loader: &mut TtLoaderRec,
    face: &TtFaceRec,
    stream: &FtStreamRec,
) -> FtResult<()> {
    let t = stream.frame_data();
    let mut p = loader.cursor;
    let limit = loader.limit;
    let num_glyphs = face.root.num_glyphs;

    let mut num_subglyphs: FtUInt = 0;

    loop {
        /* check that we can load a new subglyph */
        loader.gloader.check_subglyphs(num_subglyphs + 1)?;

        /* check space */
        if p + 4 > limit {
            return Err(FT_ERR_INVALID_COMPOSITE);
        }

        let mut subglyph = FtSubGlyphRec {
            arg1: 0,
            arg2: 0,
            ..Default::default()
        };

        subglyph.flags = ft_next_ushort(t, &mut p);
        subglyph.index = ft_next_ushort(t, &mut p) as FtInt;

        /* we reject composites that have components */
        /* with invalid glyph indices                */
        if subglyph.index as FtLong >= num_glyphs {
            return Err(FT_ERR_INVALID_COMPOSITE);
        }

        /* check space */
        let mut count = 2;
        if subglyph.flags & ARGS_ARE_WORDS != 0 {
            count += 2;
        }
        if subglyph.flags & WE_HAVE_A_SCALE != 0 {
            count += 2;
        } else if subglyph.flags & WE_HAVE_AN_XY_SCALE != 0 {
            count += 4;
        } else if subglyph.flags & WE_HAVE_A_2X2 != 0 {
            count += 8;
        }

        if p + count > limit {
            return Err(FT_ERR_INVALID_COMPOSITE);
        }

        /* read arguments */
        if subglyph.flags & ARGS_ARE_XY_VALUES != 0 {
            if subglyph.flags & ARGS_ARE_WORDS != 0 {
                subglyph.arg1 = ft_next_short(t, &mut p) as FtInt;
                subglyph.arg2 = ft_next_short(t, &mut p) as FtInt;
            } else {
                subglyph.arg1 = ft_next_char(t, &mut p) as FtInt;
                subglyph.arg2 = ft_next_char(t, &mut p) as FtInt;
            }
        } else if subglyph.flags & ARGS_ARE_WORDS != 0 {
            subglyph.arg1 = ft_next_ushort(t, &mut p) as FtInt;
            subglyph.arg2 = ft_next_ushort(t, &mut p) as FtInt;
        } else {
            subglyph.arg1 = ft_next_byte(t, &mut p) as FtInt;
            subglyph.arg2 = ft_next_byte(t, &mut p) as FtInt;
        }

        /* read transform */
        let mut xx: FtFixed = 0x10000;
        let mut yy: FtFixed = 0x10000;
        let mut xy: FtFixed = 0;
        let mut yx: FtFixed = 0;

        if subglyph.flags & WE_HAVE_A_SCALE != 0 {
            xx = ft_next_short(t, &mut p) as FtFixed * 4;
            yy = xx;
        } else if subglyph.flags & WE_HAVE_AN_XY_SCALE != 0 {
            xx = ft_next_short(t, &mut p) as FtFixed * 4;
            yy = ft_next_short(t, &mut p) as FtFixed * 4;
        } else if subglyph.flags & WE_HAVE_A_2X2 != 0 {
            xx = ft_next_short(t, &mut p) as FtFixed * 4;
            yx = ft_next_short(t, &mut p) as FtFixed * 4;
            xy = ft_next_short(t, &mut p) as FtFixed * 4;
            yy = ft_next_short(t, &mut p) as FtFixed * 4;
        }

        subglyph.transform.xx = xx;
        subglyph.transform.xy = xy;
        subglyph.transform.yx = yx;
        subglyph.transform.yy = yy;

        let flags = subglyph.flags;
        loader.gloader.current_subglyphs()[num_subglyphs as usize] = subglyph;

        num_subglyphs += 1;

        if flags & MORE_COMPONENTS == 0 {
            break;
        }
    }

    loader.gloader.current.num_subglyphs = num_subglyphs;

    /* TT_USE_BYTECODE_INTERPRETER */
    {
        /* we must undo the FT_FRAME_ENTER in order to point */
        /* to the composite instructions, if we find some.   */
        /* We will process them later.                       */
        /*                                                   */
        loader.ins_pos = (stream.pos() as i64 + p as i64 - limit as i64) as FtULong;
    }

    loader.cursor = p;

    Ok(())
}

/// The part of the glyph loader's arrays a glyph zone covers
/// (`tt_prepare_zone`'s arguments): the zone starts at point
/// `start_point` and contour `start_contour` of the base arrays and has
/// `n_points` points (including the four phantom points) and `n_contours`
/// contours; `first_point` is the number of the zone's first point.
#[derive(Debug, Clone, Copy)]
struct TtZoneSpec {
    start_point: usize,
    start_contour: usize,
    n_points: FtUShort,
    n_contours: FtShort,
    first_point: FtUShort,
}

/// `tt_prepare_zone` (for `loader->gloader->current`, at point `start`
/// of the base arrays, or the base itself)
fn tt_prepare_zone(
    gloader: &FtGlyphLoaderRec,
    current: bool,
    start_point: FtUInt,
    start_contour: FtUInt,
) -> TtZoneSpec {
    if current {
        TtZoneSpec {
            start_point: gloader.current_point_start() + start_point as usize,
            start_contour: gloader.current_contour_start() + start_contour as usize,
            n_points: (gloader.current.n_points as FtUShort)
                .wrapping_add(4)
                .wrapping_sub(start_point as FtUShort),
            n_contours: gloader
                .current
                .n_contours
                .wrapping_sub(start_contour as FtShort),
            first_point: start_point as FtUShort,
        }
    } else {
        TtZoneSpec {
            start_point: start_point as usize,
            start_contour: start_contour as usize,
            n_points: (gloader.base.outline.n_points as FtUShort)
                .wrapping_add(4)
                .wrapping_sub(start_point as FtUShort),
            n_contours: gloader
                .base
                .outline
                .n_contours
                .wrapping_sub(start_contour as FtShort),
            first_point: start_point as FtUShort,
        }
    }
}

/// Copies the zone's points out of the glyph loader (C's zone points into
/// the loader's arrays).
fn zone_from_gloader(gloader: &FtGlyphLoaderRec, spec: &TtZoneSpec) -> FtResult<TtGlyphZoneRec> {
    let s = spec.start_point;
    let n = spec.n_points as usize;
    let m = gloader.max_points as usize;
    let cs = spec.start_contour;
    let nc = spec.n_contours.max(0) as usize;

    fn copy<T: Clone>(src: &[T]) -> FtResult<Vec<T>> {
        let mut v = Vec::new();
        if v.try_reserve_exact(src.len()).is_err() {
            return Err(FT_ERR_OUT_OF_MEMORY);
        }
        v.extend_from_slice(src);
        Ok(v)
    }

    let outline = &gloader.base.outline;
    let mut contours = Vec::new();
    if contours.try_reserve_exact(nc).is_err() {
        return Err(FT_ERR_OUT_OF_MEMORY);
    }
    contours.extend(outline.contours[cs..cs + nc].iter().map(|&c| c as FtUShort));

    Ok(TtGlyphZoneRec {
        max_points: spec.n_points,
        max_contours: spec.n_contours,
        n_points: spec.n_points,
        n_contours: spec.n_contours,
        org: copy(&gloader.base.extra_points[s..s + n])?,
        cur: copy(&outline.points[s..s + n])?,
        orus: copy(&gloader.base.extra_points[m + s..m + s + n])?,
        tags: copy(&outline.tags[s..s + n])?,
        contours,
        first_point: spec.first_point,
    })
}

/// Copies the zone's points back into the glyph loader.
fn zone_to_gloader(gloader: &mut FtGlyphLoaderRec, spec: &TtZoneSpec, zone: &TtGlyphZoneRec) {
    let s = spec.start_point;
    let n = spec.n_points as usize;
    let m = gloader.max_points as usize;

    gloader.base.extra_points[s..s + n].copy_from_slice(&zone.org[..n]);
    gloader.base.outline.points[s..s + n].copy_from_slice(&zone.cur[..n]);
    gloader.base.extra_points[m + s..m + s + n].copy_from_slice(&zone.orus[..n]);
    gloader.base.outline.tags[s..s + n].copy_from_slice(&zone.tags[..n]);
}

/// `TT_Hint_Glyph`: Hint the glyph using the zone prepared by the caller.
/// Note that the zone is supposed to include four phantom points.
fn tt_hint_glyph(
    loader: &mut TtLoaderRec,
    face: &TtFaceRec,
    spec: TtZoneSpec,
    is_composite: bool,
) -> FtResult<()> {
    let mut zone = zone_from_gloader(&loader.gloader, &spec)?;
    let np = zone.n_points as usize;

    let exec = loader
        .exec
        .as_mut()
        .expect("hinting without an execution context");
    let n_ins = exec.glyph_size as FtLong;

    /* TT_USE_BYTECODE_INTERPRETER */
    /* save original point positions in `org' array */
    if n_ins > 0 {
        zone.org[..np].copy_from_slice(&zone.cur[..np]);
    }

    /* Reset graphics state. */
    exec.gs = face.size.gs;

    /* XXX: UNDOCUMENTED! Hinting instructions of a composite glyph */
    /*      completely refer to the (already) hinted subglyphs.     */
    if is_composite {
        exec.metrics.x_scale = 1 << 16;
        exec.metrics.y_scale = 1 << 16;

        zone.orus[..np].copy_from_slice(&zone.cur[..np]);
    } else {
        exec.metrics.x_scale = face.size_metrics().x_scale;
        exec.metrics.y_scale = face.size_metrics().y_scale;
    }

    /* round phantom points */
    zone.cur[np - 4].x = ft_pix_round(zone.cur[np - 4].x);
    zone.cur[np - 3].x = ft_pix_round(zone.cur[np - 3].x);
    zone.cur[np - 2].y = ft_pix_round(zone.cur[np - 2].y);
    zone.cur[np - 1].y = ft_pix_round(zone.cur[np - 1].y);

    /* TT_USE_BYTECODE_INTERPRETER */
    if n_ins > 0 {
        let ins: Arc<[u8]> = Arc::from(&exec.glyph_ins[..]);
        tt_set_code_range(exec, TT_CODERANGE_GLYPH, ins, n_ins);

        exec.is_composite = is_composite;
        exec.pts = zone;

        let error = tt_run_context(exec);

        zone = std::mem::take(&mut exec.pts);

        if let Err(e) = error {
            if exec.pedantic_hinting {
                zone_to_gloader(&mut loader.gloader, &spec, &zone);
                return Err(e);
            }
        }

        /* store drop-out mode in bits 5-7; set bit 2 also as a marker */
        let st = loader.gloader.current_point_start();
        zone_to_gloader(&mut loader.gloader, &spec, &zone);
        let exec = loader.exec.as_ref().unwrap();
        loader.gloader.base.outline.tags[st] |=
            ((exec.gs.scan_type << 5) as u8) | FT_CURVE_TAG_HAS_SCANMODE;
    } else {
        zone_to_gloader(&mut loader.gloader, &spec, &zone);
    }

    /* Save possibly modified glyph phantom points unless in v40 backward  */
    /* compatibility mode, where no movement on the x axis means no reason */
    /* to change bearings or advance widths.                               */

    /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
    if loader.interpreter_version == TT_INTERPRETER_VERSION_40
        && loader.exec.as_ref().unwrap().backward_compatibility
    {
        return Ok(());
    }

    loader.pp1 = zone.cur[np - 4];
    loader.pp2 = zone.cur[np - 3];
    loader.pp3 = zone.cur[np - 2];
    loader.pp4 = zone.cur[np - 1];

    Ok(())
}

/// The variation phantom points of the loader.
fn loader_phantoms(loader: &TtLoaderRec) -> GxVaryPhantoms {
    GxVaryPhantoms {
        pp1: loader.pp1,
        pp2: loader.pp2,
        pp3: loader.pp3,
        pp4: loader.pp4,
        linear: loader.linear,
        vadvance: loader.vadvance,
    }
}

fn set_loader_phantoms(loader: &mut TtLoaderRec, p: &GxVaryPhantoms) {
    loader.pp1 = p.pp1;
    loader.pp2 = p.pp2;
    loader.pp3 = p.pp3;
    loader.pp4 = p.pp4;
    loader.linear = p.linear;
    loader.vadvance = p.vadvance;
}

/// `TT_Process_Simple_Glyph`: Once a simple glyph has been loaded, it
/// needs to be processed.  Usually, this means scaling and hinting through
/// bytecode interpretation.
fn tt_process_simple_glyph(
    loader: &mut TtLoaderRec,
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
) -> FtResult<()> {
    let mut n_points = loader.gloader.current.n_points as usize;

    /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
    let mut unrounded: Vec<FtVector> = Vec::new();

    /* set phantom points */
    {
        let points = loader.gloader.current_points();
        points[n_points] = loader.pp1;
        points[n_points + 1] = loader.pp2;
        points[n_points + 2] = loader.pp3;
        points[n_points + 3] = loader.pp4;
    }

    n_points += 4;

    /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
    if !is_default_instance(&face.root) {
        unrounded = super::super::base::ftmemory::ft_new_array(n_points as FtLong)?;

        /* Deltas apply to the unscaled data. */
        let mut phantoms = loader_phantoms(loader);
        let n_contours = loader.gloader.current.n_contours;
        let n_pts = loader.gloader.current.n_points;
        let ps = loader.gloader.current_point_start();
        let cs = loader.gloader.current_contour_start();
        let outline = &mut loader.gloader.base.outline;
        let (points, tags, contours) = (
            &mut outline.points[ps..],
            &outline.tags[ps..],
            &outline.contours[cs..],
        );
        let mut view = GxOutlineView {
            n_points: n_pts,
            n_contours,
            points,
            tags,
            contours,
        };
        tt_vary_apply_glyph_deltas(
            face,
            stream,
            loader.glyph_index,
            &mut view,
            &mut unrounded,
            &mut phantoms,
        )?;
        set_loader_phantoms(loader, &phantoms);
    }

    let mut zone_spec = None;
    if is_hinted(loader.load_flags) {
        let spec = tt_prepare_zone(&loader.gloader, true, 0, 0);

        /* FT_ARRAY_COPY( loader->zone.orus, loader->zone.cur, n_points ) */
        let s = spec.start_point;
        let n = spec.n_points as usize;
        let m = loader.gloader.max_points as usize;
        let (pts, extra) = (
            &loader.gloader.base.outline.points,
            &mut loader.gloader.base.extra_points,
        );
        extra[m + s..m + s + n].copy_from_slice(&pts[s..s + n]);

        zone_spec = Some(spec);
    }

    {
        let mut x_scale: FtFixed = 0; /* pacify compiler */
        let mut y_scale: FtFixed = 0;

        let mut do_scale = false;

        {
            /* scale the glyph */
            if (loader.load_flags & FT_LOAD_NO_SCALE as FtULong) == 0 {
                x_scale = face.size_metrics().x_scale;
                y_scale = face.size_metrics().y_scale;

                do_scale = true;
            }
        }

        let default_instance = is_default_instance(&face.root);
        let points = &mut loader.gloader.current_points()[..n_points];

        if do_scale {
            /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
            if !default_instance {
                for (vec, u) in points.iter_mut().zip(unrounded.iter()) {
                    vec.x = add_long(ft_mul_fix(u.x, x_scale), 32) >> 6;
                    vec.y = add_long(ft_mul_fix(u.y, y_scale), 32) >> 6;
                }
            } else {
                for vec in points.iter_mut() {
                    vec.x = ft_mul_fix(vec.x, x_scale);
                    vec.y = ft_mul_fix(vec.y, y_scale);
                }
            }
        }

        let points = &loader.gloader.current_points()[..n_points];
        let (o4, o3, o2, o1) = (
            points[n_points - 4],
            points[n_points - 3],
            points[n_points - 2],
            points[n_points - 1],
        );

        /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
        /* if we have a HVAR table, `pp1' and/or `pp2' */
        /* are already adjusted but unscaled           */
        if (face.variation_support & TT_FACE_FLAG_VAR_HADVANCE) != 0 && is_hinted(loader.load_flags)
        {
            loader.pp1.x = ft_mul_fix(loader.pp1.x, x_scale);
            loader.pp2.x = ft_mul_fix(loader.pp2.x, x_scale);
            /* pp1.y and pp2.y are always zero */
        } else {
            loader.pp1 = o4;
            loader.pp2 = o3;
        }

        /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
        /* if we have a VVAR table, `pp3' and/or `pp4' */
        /* are already adjusted but unscaled           */
        if (face.variation_support & TT_FACE_FLAG_VAR_VADVANCE) != 0 && is_hinted(loader.load_flags)
        {
            loader.pp3.x = ft_mul_fix(loader.pp3.x, x_scale);
            loader.pp3.y = ft_mul_fix(loader.pp3.y, y_scale);
            loader.pp4.x = ft_mul_fix(loader.pp4.x, x_scale);
            loader.pp4.y = ft_mul_fix(loader.pp4.y, y_scale);
        } else {
            loader.pp3 = o2;
            loader.pp4 = o1;
        }
    }

    if let Some(spec) = zone_spec {
        tt_hint_glyph(loader, face, spec, false)?;
    }

    Ok(())
}

/// `TT_Process_Composite_Component`: Once a composite component has been
/// loaded, it needs to be processed.  Usually, this means transforming and
/// translating.
fn tt_process_composite_component(
    loader: &mut TtLoaderRec,
    face: &TtFaceRec,
    subglyph: &FtSubGlyphRec,
    start_point: FtUInt,
    num_base_points: FtUInt,
) -> FtResult<()> {
    let gloader = &mut loader.gloader;
    let mut x: FtPos;
    let mut y: FtPos;

    /* `current' is a view of the base outline from `num_base_points' */
    let cur_start = num_base_points as usize;
    let cur_n =
        (gloader.base.outline.n_points as i32 - num_base_points as i16 as i32).max(0) as usize;

    let have_scale = subglyph.flags & (WE_HAVE_A_SCALE | WE_HAVE_AN_XY_SCALE | WE_HAVE_A_2X2) != 0;

    /* perform the transform required for this subglyph */
    if have_scale {
        let mut current = FtOutline {
            n_points: cur_n as i16,
            points: gloader.base.outline.points[cur_start..cur_start + cur_n].to_vec(),
            ..Default::default()
        };
        ft_outline_transform(&mut current, &subglyph.transform);
        gloader.base.outline.points[cur_start..cur_start + cur_n].copy_from_slice(&current.points);
    }

    /* get offset */
    if subglyph.flags & ARGS_ARE_XY_VALUES == 0 {
        let num_points = gloader.base.outline.n_points as FtUInt;
        let mut k = subglyph.arg1 as FtUInt;
        let mut l = subglyph.arg2 as FtUInt;

        /* match l-th point of the newly loaded component to the k-th point */
        /* of the previously loaded components.                             */

        /* change to the point numbers used by our outline */
        k = k.wrapping_add(start_point);
        l = l.wrapping_add(num_base_points);
        if k >= num_base_points || l >= num_points {
            return Err(FT_ERR_INVALID_COMPOSITE);
        }

        let p1 = gloader.base.outline.points[k as usize];
        let p2 = gloader.base.outline.points[l as usize];

        x = sub_long(p1.x, p2.x);
        y = sub_long(p1.y, p2.y);
    } else {
        x = subglyph.arg1 as FtPos;
        y = subglyph.arg2 as FtPos;

        if x == 0 && y == 0 {
            return Ok(());
        }

        /* Use a default value dependent on                                  */
        /* TT_CONFIG_OPTION_COMPONENT_OFFSET_SCALED.  This is useful for old */
        /* TT fonts which don't set the xxx_COMPONENT_OFFSET bit.            */

        if have_scale && (subglyph.flags & SCALED_COMPONENT_OFFSET) != 0 {
            /*
             *
             * This algorithm is a guess and works much better than the above.
             *
             */
            let mac_xscale = ft_hypot_fixed(subglyph.transform.xx, subglyph.transform.xy);
            let mac_yscale = ft_hypot_fixed(subglyph.transform.yy, subglyph.transform.yx);

            x = ft_mul_fix(x, mac_xscale);
            y = ft_mul_fix(y, mac_yscale);
        }

        if loader.load_flags & FT_LOAD_NO_SCALE as FtULong == 0 {
            let x_scale = face.size_metrics().x_scale;
            let y_scale = face.size_metrics().y_scale;

            x = ft_mul_fix(x, x_scale);
            y = ft_mul_fix(y, y_scale);

            if subglyph.flags & ROUND_XY_TO_GRID != 0 && is_hinted(loader.load_flags) {
                /*
                 * We round the horizontal offset only if there is hinting along
                 * the x axis; this corresponds to integer advance width values.
                 *
                 * Theoretically, a glyph's bytecode can toggle ClearType's
                 * `backward compatibility' mode, which would allow modification
                 * of the advance width.  In reality, however, applications
                 * neither allow nor expect modified advance widths if subpixel
                 * rendering is active.
                 *
                 */
                if loader.interpreter_version == TT_INTERPRETER_VERSION_35 {
                    x = ft_pix_round(x);
                }

                y = ft_pix_round(y);
            }
        }
    }

    if x != 0 || y != 0 {
        let pts = &mut loader.gloader.base.outline.points[cur_start..cur_start + cur_n];
        for v in pts.iter_mut() {
            v.x = add_long(v.x, x);
            v.y = add_long(v.y, y);
        }
    }

    Ok(())
}

/// `TT_Process_Composite_Glyph`: This is slightly different from
/// TT_Process_Simple_Glyph, in that its sole purpose is to hint the glyph.
/// Thus this function is only available when bytecode interpreter is
/// enabled.
fn tt_process_composite_glyph(
    loader: &mut TtLoaderRec,
    face: &TtFaceRec,
    stream: &mut FtStreamRec,
    start_point: FtUInt,
    start_contour: FtUInt,
) -> FtResult<()> {
    /* make room for phantom points */
    let np = loader.gloader.base.outline.n_points as FtUInt;
    loader.gloader.check_points_macro(np + 4, 0)?;

    {
        let n = loader.gloader.base.outline.n_points as usize;
        let points = &mut loader.gloader.base.outline.points;
        points[n] = loader.pp1;
        points[n + 1] = loader.pp2;
        points[n + 2] = loader.pp3;
        points[n + 3] = loader.pp4;
    }

    /* TT_USE_BYTECODE_INTERPRETER */
    {
        let exec = loader
            .exec
            .as_mut()
            .expect("hinting without an execution context");

        exec.glyph_ins = Vec::new();
        exec.glyph_size = 0;

        /* TT_Load_Composite_Glyph only gives us the offset of instructions */
        /* so we read them here                                             */
        stream.seek(loader.ins_pos)?;
        let n_ins = stream.read_ushort()?;

        if n_ins == 0 {
            return Ok(());
        }

        /* don't trust `maxSizeOfInstructions'; */
        /* only do a rough safety check         */
        if n_ins as FtUInt > loader.byte_len {
            return Err(FT_ERR_TOO_MANY_HINTS);
        }

        let mut ins = super::super::base::ftmemory::ft_qalloc(n_ins as FtLong)?;
        stream.read(&mut ins)?;
        exec.glyph_ins = ins;

        exec.glyph_size = n_ins as FtUInt;
    }

    let spec = tt_prepare_zone(&loader.gloader, false, start_point, start_contour);

    /* Some points are likely touched during execution of  */
    /* instructions on components.  So let's untouch them. */
    {
        let s = spec.start_point;
        let tags = &mut loader.gloader.base.outline.tags;
        for i in 0..(spec.n_points as FtUInt).wrapping_sub(4) as usize {
            tags[s + i] &= !FT_CURVE_TAG_TOUCH_BOTH;
        }
    }

    tt_hint_glyph(loader, face, spec, true)
}

/*
 * Calculate the phantom points
 *
 * Defining the right side bearing (rsb) as
 *
 *   rsb = aw - (lsb + xmax - xmin)
 *
 * (with `aw' the advance width, `lsb' the left side bearing, and `xmin'
 * and `xmax' the glyph's minimum and maximum x value), the OpenType
 * specification defines the initial position of horizontal phantom points
 * as
 *
 *   pp1 = (round(xmin - lsb), 0)      ,
 *   pp2 = (round(pp1 + aw), 0)        .
 *
 * Note that the rounding to the grid (in the device space) is not
 * documented currently in the specification.
 *
 * However, the specification lacks the precise definition of vertical
 * phantom points.  Greg Hitchcock provided the following explanation.
 *
 * - a `vmtx' table is present
 *
 *   For any glyph, the minimum and maximum y values (`ymin' and `ymax')
 *   are given in the `glyf' table, the top side bearing (tsb) and advance
 *   height (ah) are given in the `vmtx' table.  The bottom side bearing
 *   (bsb) is then calculated as
 *
 *     bsb = ah - (tsb + ymax - ymin)       ,
 *
 *   and the initial position of vertical phantom points is
 *
 *     pp3 = (x, round(ymax + tsb))       ,
 *     pp4 = (x, round(pp3 - ah))         .
 *
 *   See below for value `x'.
 *
 * - no `vmtx' table in the font
 *
 *   If there is an `OS/2' table, we set
 *
 *     DefaultAscender = sTypoAscender       ,
 *     DefaultDescender = sTypoDescender     ,
 *
 *   otherwise we use data from the `hhea' table:
 *
 *     DefaultAscender = Ascender         ,
 *     DefaultDescender = Descender       .
 *
 *   With these two variables we can now set
 *
 *     ah = DefaultAscender - sDefaultDescender    ,
 *     tsb = DefaultAscender - yMax                ,
 *
 *   and proceed as if a `vmtx' table was present.
 *
 * Usually we have
 *
 *   x = aw / 2      ,                                                (1)
 *
 * but there is one compatibility case where it can be set to
 *
 *   x = -DefaultDescender -
 *         ((DefaultAscender - DefaultDescender - aw) / 2)     .      (2)
 *
 * and another one with
 *
 *   x = 0     .                                                      (3)
 *
 * In Windows, the history of those values is quite complicated,
 * depending on the hinting engine (that is, the graphics framework).
 *
 *   framework        from                 to       formula
 *  ----------------------------------------------------------
 *    GDI       Windows 98               current      (1)
 *              (Windows 2000 for NT)
 *    GDI+      Windows XP               Windows 7    (2)
 *    GDI+      Windows 8                current      (3)
 *    DWrite    Windows 7                current      (3)
 *
 * For simplicity, FreeType uses (1) for grayscale subpixel hinting and
 * (3) for everything else.
 *
 */
/// `tt_loader_set_pp`
fn tt_loader_set_pp(loader: &mut TtLoaderRec) {
    loader.pp1.x = loader.bbox.xMin - loader.left_bearing as FtPos;
    loader.pp1.y = 0;
    loader.pp2.x = loader.pp1.x + loader.advance as FtPos;
    loader.pp2.y = 0;

    loader.pp3.x = 0;
    loader.pp3.y = loader.bbox.yMax + loader.top_bearing as FtPos;
    loader.pp4.x = 0;
    loader.pp4.y = loader.pp3.y - loader.vadvance as FtPos;

    /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
    if loader.interpreter_version == TT_INTERPRETER_VERSION_40 {
        if let Some(exec) = &loader.exec {
            if exec.subpixel_hinting_lean && exec.grayscale_cleartype {
                loader.pp3.x = loader.advance as FtPos / 2;
                loader.pp4.x = loader.advance as FtPos / 2;
            }
        }
    }
}

/// `load_truetype_glyph`: Loads a given truetype glyph.  Handles
/// composites and uses a TT_Loader object.
fn load_truetype_glyph(
    loader: &mut TtLoaderRec,
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    glyph_index: FtUInt,
    recurse_count: FtUInt,
    header_only: bool,
) -> FtResult<()> {
    let mut opened_frame = false;

    let r = load_truetype_glyph_inner(
        loader,
        face,
        stream,
        glyph_index,
        recurse_count,
        header_only,
        &mut opened_frame,
    );

    /* Exit: */
    if opened_frame {
        tt_forget_glyph_frame(stream);
    }

    r
}

fn load_truetype_glyph_inner(
    loader: &mut TtLoaderRec,
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    glyph_index: FtUInt,
    recurse_count: FtUInt,
    header_only: bool,
    opened_frame: &mut bool,
) -> FtResult<()> {
    /* some fonts have an incorrect value of `maxComponentDepth' */
    if recurse_count > face.max_profile.maxComponentDepth as FtUInt {
        face.max_profile.maxComponentDepth = recurse_count as FtUShort;
    }

    /* check glyph index */
    if glyph_index >= face.root.num_glyphs as FtUInt {
        return Err(FT_ERR_INVALID_GLYPH_INDEX);
    }

    loader.glyph_index = glyph_index;

    let (x_scale, y_scale) = if loader.load_flags & FT_LOAD_NO_SCALE as FtULong != 0 {
        (0x10000, 0x10000)
    } else {
        (face.size_metrics().x_scale, face.size_metrics().y_scale)
    };

    /* Set `offset' to the start of the glyph relative to the start of */
    /* the `glyf' table, and `byte_len' to the length of the glyph in  */
    /* bytes.                                                          */

    let offset;
    {
        let (o, len) = tt_face_get_location(face, glyph_index);
        offset = o;
        loader.byte_len = len as FtUInt;
    }

    if loader.byte_len > 0 {
        if face.glyf_offset == 0 {
            return Err(FT_ERR_INVALID_TABLE);
        }

        tt_access_glyph_frame(
            loader,
            stream,
            glyph_index,
            face.glyf_offset + offset,
            loader.byte_len,
        )?;

        /* read glyph header first */
        let error = tt_load_glyph_header(loader, stream);

        tt_forget_glyph_frame(stream);

        error?;
    }

    /* a space glyph */
    if loader.byte_len == 0 || loader.n_contours == 0 {
        loader.bbox.xMin = 0;
        loader.bbox.xMax = 0;
        loader.bbox.yMin = 0;
        loader.bbox.yMax = 0;
    }

    /* the metrics must be computed after loading the glyph header */
    /* since we need the glyph's `yMax' value in case the vertical */
    /* metrics must be emulated                                    */
    tt_get_metrics(loader, face, stream, glyph_index)?;

    if header_only {
        return Ok(());
    }

    if loader.byte_len == 0 || loader.n_contours == 0 {
        tt_loader_set_pp(loader);

        /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
        if ft_is_named_instance(&face.root) || ft_is_variation(&face.root) {
            /* a small outline structure with four elements for */
            /* communication with `TT_Vary_Apply_Glyph_Deltas'  */
            let mut points = [loader.pp1, loader.pp2, loader.pp3, loader.pp4];

            /* unrounded values */
            let mut unrounded = [FtVector::default(); 4];

            let mut view = GxOutlineView {
                n_points: 0,
                n_contours: 0,
                points: &mut points,
                tags: &[],
                contours: &[],
            };

            /* this must be done before scaling */
            let mut phantoms = loader_phantoms(loader);
            tt_vary_apply_glyph_deltas(
                face,
                stream,
                loader.glyph_index,
                &mut view,
                &mut unrounded,
                &mut phantoms,
            )?;
            set_loader_phantoms(loader, &phantoms);
        }

        /* scale phantom points, if necessary; */
        /* they get rounded in `TT_Hint_Glyph' */
        if (loader.load_flags & FT_LOAD_NO_SCALE as FtULong) == 0 {
            loader.pp1.x = ft_mul_fix(loader.pp1.x, x_scale);
            loader.pp2.x = ft_mul_fix(loader.pp2.x, x_scale);
            /* pp1.y and pp2.y are always zero */

            loader.pp3.x = ft_mul_fix(loader.pp3.x, x_scale);
            loader.pp3.y = ft_mul_fix(loader.pp3.y, y_scale);
            loader.pp4.x = ft_mul_fix(loader.pp4.x, x_scale);
            loader.pp4.y = ft_mul_fix(loader.pp4.y, y_scale);
        }

        return Ok(());
    }

    tt_loader_set_pp(loader);

    /* we now open a frame again, right after the glyph header */
    /* (which consists of 10 bytes)                            */
    tt_access_glyph_frame(
        loader,
        stream,
        glyph_index,
        face.glyf_offset + offset + 10,
        loader.byte_len - 10,
    )?;

    *opened_frame = true;

    /* if it is a simple glyph, load it */

    if loader.n_contours > 0 {
        tt_load_simple_glyph(loader, stream)?;

        /* all data have been read */
        tt_forget_glyph_frame(stream);
        *opened_frame = false;

        tt_process_simple_glyph(loader, face, stream)?;

        loader.gloader.add();
    }
    /* otherwise, load a composite! */
    else if loader.n_contours < 0 {
        /* normalize the `n_contours' value */
        loader.n_contours = -1;

        /*
         * We store the glyph index directly in the `node->data' pointer,
         * following the glib solution (cf. macro `GUINT_TO_POINTER') with a
         * double cast to make this portable.  Note, however, that this needs
         * pointers with a width of at least 32 bits.
         */

        /* clear the nodes filled by sibling chains */
        let node = recurse_count as usize;
        for node2 in loader.composites.iter_mut().skip(node) {
            *node2 = None;
        }

        /* check whether we already have a composite glyph with this index */
        if loader.composites.contains(&Some(glyph_index)) {
            return Err(FT_ERR_INVALID_COMPOSITE);
        } else if node < loader.composites.len() {
            loader.composites[node] = Some(glyph_index);
        } else {
            if loader.composites.try_reserve(1).is_err() {
                return Err(FT_ERR_OUT_OF_MEMORY);
            }
            loader.composites.push(Some(glyph_index));
        }

        let start_point = loader.gloader.base.outline.n_points as FtUInt;
        let start_contour = loader.gloader.base.outline.n_contours as FtUInt;

        /* for each subglyph, read composite header */
        tt_load_composite_glyph(loader, face, stream)?;

        /* store the offset of instructions */
        let ins_pos = loader.ins_pos;

        /* all data we need are read */
        tt_forget_glyph_frame(stream);
        *opened_frame = false;

        /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
        if ft_is_named_instance(&face.root) || ft_is_variation(&face.root) {
            let limit = loader.gloader.current.num_subglyphs as usize;

            /* construct an outline structure for              */
            /* communication with `TT_Vary_Apply_Glyph_Deltas' */
            use super::super::base::ftmemory::ft_new_array;
            let mut points: Vec<FtVector> = ft_new_array((limit + 4) as FtLong)?;
            let mut tags: Vec<u8> = ft_new_array(limit as FtLong)?;
            let mut contours: Vec<i16> = ft_new_array(limit as FtLong)?;
            let mut unrounded: Vec<FtVector> = ft_new_array((limit + 4) as FtLong)?;

            {
                let subglyphs = loader.gloader.current_subglyphs();
                for i in 0..limit {
                    /* applying deltas for anchor points doesn't make sense, */
                    /* but we don't have to specially check this since       */
                    /* unused delta values are zero anyways                  */
                    points[i].x = subglyphs[i].arg1 as FtPos;
                    points[i].y = subglyphs[i].arg2 as FtPos;
                    tags[i] = ON_CURVE_POINT;
                    contours[i] = i as i16;
                }
            }

            points[limit] = loader.pp1;
            points[limit + 1] = loader.pp2;
            points[limit + 2] = loader.pp3;
            points[limit + 3] = loader.pp4;

            /* this call provides additional offsets */
            /* for each component's translation      */
            let mut phantoms = loader_phantoms(loader);
            {
                let mut view = GxOutlineView {
                    n_points: limit as i16,
                    n_contours: limit as i16,
                    points: &mut points,
                    tags: &tags,
                    contours: &contours,
                };
                tt_vary_apply_glyph_deltas(
                    face,
                    stream,
                    loader.glyph_index,
                    &mut view,
                    &mut unrounded,
                    &mut phantoms,
                )?;
            }
            set_loader_phantoms(loader, &phantoms);

            let subglyphs = loader.gloader.current_subglyphs();
            for i in 0..limit {
                if subglyphs[i].flags & ARGS_ARE_XY_VALUES != 0 {
                    subglyphs[i].arg1 = points[i].x as i16 as FtInt;
                    subglyphs[i].arg2 = points[i].y as i16 as FtInt;
                }
            }
            tags.clear();
            contours.clear();
        }

        /* scale phantom points, if necessary; */
        /* they get rounded in `TT_Hint_Glyph' */
        if (loader.load_flags & FT_LOAD_NO_SCALE as FtULong) == 0 {
            loader.pp1.x = ft_mul_fix(loader.pp1.x, x_scale);
            loader.pp2.x = ft_mul_fix(loader.pp2.x, x_scale);
            /* pp1.y and pp2.y are always zero */

            loader.pp3.x = ft_mul_fix(loader.pp3.x, x_scale);
            loader.pp3.y = ft_mul_fix(loader.pp3.y, y_scale);
            loader.pp4.x = ft_mul_fix(loader.pp4.x, x_scale);
            loader.pp4.y = ft_mul_fix(loader.pp4.y, y_scale);
        }

        /* if the flag FT_LOAD_NO_RECURSE is set, we return the subglyph */
        /* `as is' in the glyph slot (the client application will be     */
        /* responsible for interpreting these data)...                   */
        if loader.load_flags & FT_LOAD_NO_RECURSE as FtULong != 0 {
            loader.gloader.add();
            face.root.glyph.format = FT_GLYPH_FORMAT_COMPOSITE;

            return Ok(());
        }

        /*********************************************************************/
        /*********************************************************************/
        /*********************************************************************/

        {
            let mut subglyph: Option<FtSubGlyphRec> = None;

            let mut num_points = start_point;
            let num_subglyphs = loader.gloader.current.num_subglyphs;
            let num_base_subgs = loader.gloader.base.num_subglyphs;

            let old_byte_len = loader.byte_len;

            loader.gloader.add();

            /* read each subglyph independently */
            for n in 0..num_subglyphs {
                /* Each time we call `load_truetype_glyph' in this loop, the */
                /* value of `gloader.base.subglyphs' can change due to table */
                /* reallocations.  We thus need to recompute the subglyph    */
                /* pointer on each iteration.                                */
                let sg = loader.gloader.base.subglyphs[(num_base_subgs + n) as usize];

                let pp = [loader.pp1, loader.pp2, loader.pp3, loader.pp4];

                let linear_hadvance = loader.linear;
                let linear_vadvance = loader.vadvance;

                let num_base_points = loader.gloader.base.outline.n_points as FtUInt;

                load_truetype_glyph(
                    loader,
                    face,
                    stream,
                    sg.index as FtUInt,
                    recurse_count + 1,
                    false,
                )?;

                /* restore subglyph pointer */
                let sg = loader.gloader.base.subglyphs[(num_base_subgs + n) as usize];
                subglyph = Some(sg);

                /* restore phantom points if necessary */
                if sg.flags & USE_MY_METRICS == 0 {
                    loader.pp1 = pp[0];
                    loader.pp2 = pp[1];
                    loader.pp3 = pp[2];
                    loader.pp4 = pp[3];

                    loader.linear = linear_hadvance;
                    loader.vadvance = linear_vadvance;
                }

                num_points = loader.gloader.base.outline.n_points as FtUInt;

                if num_points == num_base_points {
                    continue;
                }

                /* gloader->base.outline consists of three parts:           */
                /*                                                          */
                /* 0 ----> start_point ----> num_base_points ----> n_points */
                /*    (1)               (2)                   (3)           */
                /*                                                          */
                /* (1) points that exist from the beginning                 */
                /* (2) component points that have been loaded so far        */
                /* (3) points of the newly loaded component                 */
                tt_process_composite_component(loader, face, &sg, start_point, num_base_points)?;
            }

            loader.byte_len = old_byte_len;

            /* process the glyph */
            loader.ins_pos = ins_pos;
            if is_hinted(loader.load_flags)
                && subglyph.is_some_and(|sg| sg.flags & WE_HAVE_INSTR != 0)
                && num_points > start_point
            {
                tt_process_composite_glyph(loader, face, stream, start_point, start_contour)?;
            }
        }

        /* retain the overlap flag */
        if loader.gloader.base.num_subglyphs != 0
            && loader.gloader.base.subglyphs[0].flags & OVERLAP_COMPOUND != 0
        {
            loader.gloader.base.outline.flags |= FT_OUTLINE_OVERLAP;
        }
    }

    /*********************************************************************/
    /*********************************************************************/
    /*********************************************************************/

    Ok(())
}

/// `compute_glyph_metrics`
fn compute_glyph_metrics(
    loader: &TtLoaderRec,
    face: &mut TtFaceRec,
    glyph_index: FtUInt,
) -> FtResult<()> {
    let mut y_scale: FtFixed = 0x10000;
    if (loader.load_flags & FT_LOAD_NO_SCALE as FtULong) == 0 {
        y_scale = face.size_metrics().y_scale;
    }

    let bbox = if face.root.glyph.format != FT_GLYPH_FORMAT_COMPOSITE {
        ft_outline_get_cbox(&face.root.glyph.outline)
    } else {
        loader.bbox
    };

    let vertical_info = face.vertical_info;
    let number_of_vmetrics = face.vertical.number_Of_VMetrics;
    let os2_version = face.os2.version;
    let (typo_asc, typo_desc) = (face.os2.sTypoAscender, face.os2.sTypoDescender);
    let (hasc, hdesc) = (face.horizontal.Ascender, face.horizontal.Descender);
    let widthp = loader
        .widthp
        .map(|w| face.hdmx_table[w + glyph_index as usize]);
    let glyph = &mut face.root.glyph;

    /* get the device-independent horizontal advance; it is scaled later */
    /* by the base layer.                                                */
    glyph.linearHoriAdvance = loader.linear as FtFixed;

    glyph.metrics.horiBearingX = bbox.xMin;
    glyph.metrics.horiBearingY = bbox.yMax;
    if let Some(w) = widthp {
        glyph.metrics.horiAdvance = w as FtPos * 64;
    } else {
        glyph.metrics.horiAdvance = sub_long(loader.pp2.x, loader.pp1.x);
    }

    /* set glyph dimensions */
    glyph.metrics.width = sub_long(bbox.xMax, bbox.xMin);
    glyph.metrics.height = sub_long(bbox.yMax, bbox.yMin);

    /* Now take care of vertical metrics.  In the case where there is */
    /* no vertical information within the font (relatively common),   */
    /* create some metrics manually                                   */
    {
        let mut top: FtPos; /* scaled vertical top side bearing  */
        let mut advance: FtPos; /* scaled vertical advance height    */

        /* Get the unscaled top bearing and advance height. */
        if vertical_info && number_of_vmetrics > 0 {
            top = ft_div_fix(sub_long(loader.pp3.y, bbox.yMax), y_scale) as FtShort as FtPos;

            if loader.pp3.y <= loader.pp4.y {
                advance = 0;
            } else {
                advance =
                    ft_div_fix(sub_long(loader.pp3.y, loader.pp4.y), y_scale) as FtUShort as FtPos;
            }
        } else {
            /* XXX Compute top side bearing and advance height in  */
            /*     Get_VMetrics instead of here.                   */

            /* NOTE: The OS/2 values are the only `portable' ones, */
            /*       which is why we use them, if there is an OS/2 */
            /*       table in the font.  Otherwise, we use the     */
            /*       values defined in the horizontal header.      */

            let height = ft_div_fix(sub_long(bbox.yMax, bbox.yMin), y_scale) as FtShort as FtPos;
            if os2_version != 0xFFFF {
                advance = typo_asc as FtPos - typo_desc as FtPos;
            } else {
                advance = hasc as FtPos - hdesc as FtPos;
            }

            top = (advance - height) / 2;
        }

        glyph.linearVertAdvance = advance;

        /* scale the metrics */
        if loader.load_flags & FT_LOAD_NO_SCALE as FtULong == 0 {
            top = ft_mul_fix(top, y_scale);
            advance = ft_mul_fix(advance, y_scale);
        }

        /* XXX: for now, we have no better algorithm for the lsb, but it */
        /*      should work fine.                                        */
        /*                                                               */
        glyph.metrics.vertBearingX =
            sub_long(glyph.metrics.horiBearingX, glyph.metrics.horiAdvance / 2);
        glyph.metrics.vertBearingY = top;
        glyph.metrics.vertAdvance = advance;
    }

    Ok(())
}

/// `load_sbit_image` (TT_CONFIG_OPTION_EMBEDDED_BITMAPS)
fn load_sbit_image(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    glyph_index: FtUInt,
    load_flags: FtInt32,
) -> FtResult<()> {
    let mut sbit_metrics = TtSBitMetricsRec::default();

    let strike_index = face.size.strike_index;
    let error = tt_face_load_sbit_image(
        face,
        stream,
        strike_index,
        glyph_index,
        load_flags as FtUInt,
        &mut sbit_metrics,
    );
    if error.is_ok() {
        let glyph = &mut face.root.glyph;

        glyph.outline.n_points = 0;
        glyph.outline.n_contours = 0;

        glyph.metrics.width = sbit_metrics.width as FtPos * 64;
        glyph.metrics.height = sbit_metrics.height as FtPos * 64;

        glyph.metrics.horiBearingX = sbit_metrics.horiBearingX as FtPos * 64;
        glyph.metrics.horiBearingY = sbit_metrics.horiBearingY as FtPos * 64;
        glyph.metrics.horiAdvance = sbit_metrics.horiAdvance as FtPos * 64;

        glyph.metrics.vertBearingX = sbit_metrics.vertBearingX as FtPos * 64;
        glyph.metrics.vertBearingY = sbit_metrics.vertBearingY as FtPos * 64;
        glyph.metrics.vertAdvance = sbit_metrics.vertAdvance as FtPos * 64;

        glyph.format = FT_GLYPH_FORMAT_BITMAP;

        if load_flags & FT_LOAD_VERTICAL_LAYOUT != 0 {
            glyph.bitmap_left = sbit_metrics.vertBearingX as FtInt;
            glyph.bitmap_top = sbit_metrics.vertBearingY as FtInt;
        } else {
            glyph.bitmap_left = sbit_metrics.horiBearingX as FtInt;
            glyph.bitmap_top = sbit_metrics.horiBearingY as FtInt;
        }
    }

    error
}

/// `tt_loader_init`
fn tt_loader_init(
    loader: &mut TtLoaderRec,
    face: &mut TtFaceRec,
    mut load_flags: FtInt32,
    glyf_table_only: bool,
) -> FtResult<()> {
    /* TT_USE_BYTECODE_INTERPRETER */
    let pedantic = load_flags & FT_LOAD_PEDANTIC != 0;

    /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
    let interpreter_version = tt_driver_interpreter_version(face);

    *loader = TtLoaderRec::default();
    loader.interpreter_version = interpreter_version;

    /* TT_USE_BYTECODE_INTERPRETER */

    /* load execution context */
    if is_hinted(load_flags as FtULong) && !glyf_table_only {
        let mut grayscale;

        /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
        let subpixel_hinting_lean: bool;
        let grayscale_cleartype: bool;

        let mut reexecute = false;

        if face.size.bytecode_ready < 0 || face.size.cvt_ready < 0 {
            tt_size_ready_bytecode(face, pedantic)?;
        } else if face.size.bytecode_ready != 0 {
            return Err(face.size.bytecode_ready);
        } else if face.size.cvt_ready != 0 {
            return Err(face.size.cvt_ready);
        }

        /* query new execution context */
        let mut exec = match face.size.context.take() {
            Some(e) => e,
            None => return Err(FT_ERR_COULD_NOT_FIND_CONTEXT),
        };

        grayscale = ft_load_target_mode(load_flags) != FT_RENDER_MODE_MONO;

        /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
        if interpreter_version == TT_INTERPRETER_VERSION_40 {
            subpixel_hinting_lean = ft_load_target_mode(load_flags) != FT_RENDER_MODE_MONO;
            grayscale_cleartype = subpixel_hinting_lean
                && !((load_flags & FT_LOAD_TARGET_LCD) != 0
                    || (load_flags & FT_LOAD_TARGET_LCD_V) != 0);
            exec.vertical_lcd_lean =
                subpixel_hinting_lean && (load_flags & FT_LOAD_TARGET_LCD_V) != 0;
            grayscale = grayscale && !subpixel_hinting_lean;
        } else {
            subpixel_hinting_lean = false;
            grayscale_cleartype = false;
            exec.vertical_lcd_lean = false;
        }

        if let Err(e) = tt_load_context(&mut exec, face, interpreter_version) {
            tt_unload_context(&mut exec, &mut face.size);
            face.size.context = Some(exec);
            return Err(e);
        }

        {
            /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
            if interpreter_version == TT_INTERPRETER_VERSION_40 {
                /* a change from mono to subpixel rendering (and vice versa) */
                /* requires a re-execution of the CVT program                */
                if subpixel_hinting_lean != exec.subpixel_hinting_lean {
                    exec.subpixel_hinting_lean = subpixel_hinting_lean;
                    reexecute = true;
                }

                /* a change from colored to grayscale subpixel rendering (and */
                /* vice versa) requires a re-execution of the CVT program     */
                if grayscale_cleartype != exec.grayscale_cleartype {
                    exec.grayscale_cleartype = grayscale_cleartype;
                    reexecute = true;
                }
            }

            /* a change from mono to grayscale rendering (and vice versa) */
            /* requires a re-execution of the CVT program                 */
            if grayscale != exec.grayscale {
                exec.grayscale = grayscale;
                reexecute = true;
            }
        }

        if reexecute {
            /* (`tt_size_run_prep' loads the context itself) */
            tt_unload_context(&mut exec, &mut face.size);
            face.size.context = Some(exec);

            tt_size_run_prep(face, pedantic)?;

            exec = match face.size.context.take() {
                Some(e) => e,
                None => return Err(FT_ERR_COULD_NOT_FIND_CONTEXT),
            };

            if let Err(e) = tt_load_context(&mut exec, face, interpreter_version) {
                tt_unload_context(&mut exec, &mut face.size);
                face.size.context = Some(exec);
                return Err(e);
            }
        }

        /* check whether the cvt program has disabled hinting */
        if exec.gs.instruct_control & 1 != 0 {
            load_flags |= FT_LOAD_NO_HINTING;
        }

        /* load default graphics state -- if needed */
        if exec.gs.instruct_control & 2 != 0 {
            exec.gs = TT_DEFAULT_GRAPHICS_STATE;
        }

        /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
        /*
         * Toggle backward compatibility according to what font wants, except
         * when
         *
         * 1) we have a `tricky' font that heavily relies on the interpreter to
         *    render glyphs correctly, for example DFKai-SB, or
         * 2) FT_RENDER_MODE_MONO (i.e, monochome rendering) is requested.
         *
         * In those cases, backward compatibility needs to be turned off to get
         * correct rendering.  The rendering is then completely up to the
         * font's programming.
         *
         */
        if interpreter_version == TT_INTERPRETER_VERSION_40
            && subpixel_hinting_lean
            && !ft_is_tricky(&face.root)
        {
            exec.backward_compatibility = (exec.gs.instruct_control & 4) == 0;
        } else {
            exec.backward_compatibility = false;
        }

        exec.pedantic_hinting = load_flags & FT_LOAD_PEDANTIC != 0;
        let backward_compatibility = exec.backward_compatibility;
        loader.exec = Some(exec);
        loader.exec_loaded = true;
        /* (loader->instructions = exec->glyphIns: unused) */

        /* Use the hdmx table if any unless FT_LOAD_COMPUTE_METRICS */
        /* is set or backward compatibility mode of the v38 or v40  */
        /* interpreters is active.  See `ttinterp.h' for details on */
        /* backward compatibility mode.                             */
        /* FIXME (upstream): `loader->load_flags' is still zero here (the  */
        /* loader was just cleared with FT_ZERO), so neither the hinting   */
        /* flag nor FT_LOAD_COMPUTE_METRICS is honored by this test.       */
        if is_hinted(loader.load_flags)
            && (loader.load_flags & FT_LOAD_COMPUTE_METRICS as FtULong) == 0
            && !(interpreter_version == TT_INTERPRETER_VERSION_40 && backward_compatibility)
            && face.postscript.isFixedPitch == 0
        {
            loader.widthp = face.size.widthp;
        } else {
            loader.widthp = None;
        }
    }

    /* get face's glyph loader */
    if !glyf_table_only {
        let mut gloader = face.root.glyph.internal.loader.take().unwrap_or_default();

        gloader.rewind();
        loader.gloader = gloader;
    }

    loader.load_flags = load_flags as FtULong;

    loader.composites = Vec::new();

    Ok(())
}

/// `tt_loader_done` (also puts the glyph loader and the execution context
/// back)
fn tt_loader_done(loader: &mut TtLoaderRec, face: &mut TtFaceRec, glyf_table_only: bool) {
    loader.composites = Vec::new();

    if !glyf_table_only {
        face.root.glyph.internal.loader = Some(std::mem::take(&mut loader.gloader));
    }

    if let Some(mut exec) = loader.exec.take() {
        if loader.exec_loaded {
            tt_unload_context(&mut exec, &mut face.size);
            loader.exec_loaded = false;
        }
        face.size.context = Some(exec);
    }
}

/// `TT_Load_Glyph`: A function used to load a single glyph within a given
/// glyph slot, for a given size (the face's glyph slot and size).
pub fn tt_load_glyph(
    face: &mut TtFaceRec,
    glyph_index: FtUInt,
    load_flags: FtInt32,
) -> FtResult<()> {
    with_stream(face, |face, stream| {
        tt_load_glyph_stream(face, stream, glyph_index, load_flags)
    })
}

fn tt_load_glyph_stream(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    glyph_index: FtUInt,
    load_flags: FtInt32,
) -> FtResult<()> {
    let mut loader = TtLoaderRec::default();

    /* TT_CONFIG_OPTION_EMBEDDED_BITMAPS */
    /* try to load embedded bitmap (if any) */
    if face.size.strike_index != 0xFFFFFFFF
        && (load_flags & FT_LOAD_NO_BITMAP) == 0
        && is_default_instance(&face.root)
    {
        let x_scale = face.root.size.metrics.x_scale;
        let y_scale = face.root.size.metrics.y_scale;

        let error = load_sbit_image(face, stream, glyph_index, load_flags);
        match error {
            Err(e) if ft_err_eq(e, FT_ERR_MISSING_BITMAP) => {
                /* the bitmap strike is incomplete and misses the requested glyph; */
                /* if we have a bitmap-only font, return an empty glyph            */
                if !ft_is_scalable(&face.root) {
                    /* to return an empty glyph, however, we need metrics data   */
                    /* from the `hmtx' (or `vmtx') table; the assumption is that */
                    /* empty glyphs are missing intentionally, representing      */
                    /* whitespace - not having at least horizontal metrics is    */
                    /* thus considered an error                                  */
                    if face.horz_metrics_size == 0 {
                        return Err(e);
                    }

                    /* we now construct an empty bitmap glyph */
                    let (left_bearing, advance_width) = tt_get_hmetrics(face, stream, glyph_index);
                    let (top_bearing, advance_height) =
                        tt_get_vmetrics(face, stream, glyph_index, 0);

                    let glyph = &mut face.root.glyph;

                    glyph.outline.n_points = 0;
                    glyph.outline.n_contours = 0;

                    glyph.metrics.width = 0;
                    glyph.metrics.height = 0;

                    glyph.metrics.horiBearingX = ft_mul_fix(left_bearing as FtLong, x_scale);
                    glyph.metrics.horiBearingY = 0;
                    glyph.metrics.horiAdvance = ft_mul_fix(advance_width as FtLong, x_scale);

                    glyph.metrics.vertBearingX = 0;
                    glyph.metrics.vertBearingY = ft_mul_fix(top_bearing as FtLong, y_scale);
                    glyph.metrics.vertAdvance = ft_mul_fix(advance_height as FtLong, y_scale);

                    glyph.format = FT_GLYPH_FORMAT_BITMAP;
                    glyph.bitmap.pixel_mode = FT_PIXEL_MODE_MONO;

                    glyph.bitmap_left = 0;
                    glyph.bitmap_top = 0;

                    return Ok(());
                }
            }
            Err(e) => {
                /* return error if font is not scalable */
                if !ft_is_scalable(&face.root) {
                    return Err(e);
                }
            }
            Ok(()) => {
                if ft_is_scalable(&face.root) || ft_has_sbix(&face.root) {
                    /* for the bbox we need the header only */
                    let _ = tt_loader_init(&mut loader, face, load_flags, true);
                    let _ = load_truetype_glyph(&mut loader, face, stream, glyph_index, 0, true);
                    tt_loader_done(&mut loader, face, true);
                    face.root.glyph.linearHoriAdvance = loader.linear as FtFixed;
                    face.root.glyph.linearVertAdvance = loader.vadvance as FtFixed;

                    /* Bitmaps from the 'sbix' table need special treatment:  */
                    /* if there is a glyph contour, the bitmap origin must be */
                    /* shifted to be relative to the lower left corner of the */
                    /* glyph bounding box, also taking the left-side bearing  */
                    /* (or top bearing) into account.                         */
                    if face.sbit_table_type == TT_SBIT_TABLE_TYPE_SBIX && loader.n_contours > 0 {
                        let (bitmap_left, bitmap_top): (FtInt, FtInt);

                        if load_flags & FT_LOAD_VERTICAL_LAYOUT != 0 {
                            /* This is a guess, since Apple's CoreText engine doesn't */
                            /* really do vertical typesetting.                        */
                            bitmap_left = loader.bbox.xMin as FtInt;
                            bitmap_top = loader.top_bearing;
                        } else {
                            bitmap_left = loader.left_bearing;
                            bitmap_top = loader.bbox.yMin as FtInt;
                        }

                        let glyph = &mut face.root.glyph;
                        glyph.bitmap_left +=
                            (ft_mul_fix(bitmap_left as FtLong, x_scale) >> 6) as FtInt;
                        glyph.bitmap_top +=
                            (ft_mul_fix(bitmap_top as FtLong, y_scale) >> 6) as FtInt;
                    }

                    /* sanity checks: if `xxxAdvance' in the sbit metric */
                    /* structure isn't set, use `linearXXXAdvance'      */
                    let glyph = &mut face.root.glyph;
                    if glyph.metrics.horiAdvance == 0 && glyph.linearHoriAdvance != 0 {
                        glyph.metrics.horiAdvance = ft_mul_fix(glyph.linearHoriAdvance, x_scale);
                    }
                    if glyph.metrics.vertAdvance == 0 && glyph.linearVertAdvance != 0 {
                        glyph.metrics.vertAdvance = ft_mul_fix(glyph.linearVertAdvance, y_scale);
                    }
                }

                return Ok(());
            }
        }
    }

    if load_flags & FT_LOAD_SBITS_ONLY != 0 {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    /* if FT_LOAD_NO_SCALE is not set, `ttmetrics' must be valid */
    if (load_flags & FT_LOAD_NO_SCALE) == 0 && !face.size.ttmetrics.valid {
        return Err(FT_ERR_INVALID_SIZE_HANDLE);
    }

    /* FT_CONFIG_OPTION_SVG */

    /* check for OT-SVG */
    if (load_flags & FT_LOAD_NO_SVG) == 0 && (load_flags & FT_LOAD_COLOR) != 0 && face.svg.is_some()
    {
        if tt_face_load_svg_doc(face, glyph_index).is_ok() {
            let x_scale = face.root.size.metrics.x_scale;
            let y_scale = face.root.size.metrics.y_scale;

            face.root.glyph.format = FT_GLYPH_FORMAT_SVG;

            let (_left_bearing, advance_x) = tt_face_get_metrics(face, stream, false, glyph_index);
            let (_top_bearing, advance_y) = tt_face_get_metrics(face, stream, true, glyph_index);

            let glyph = &mut face.root.glyph;
            glyph.linearHoriAdvance = advance_x as FtFixed;
            glyph.linearVertAdvance = advance_y as FtFixed;

            glyph.metrics.horiAdvance = ft_mul_fix(advance_x as FtLong, x_scale);
            glyph.metrics.vertAdvance = ft_mul_fix(advance_y as FtLong, y_scale);

            return Ok(());
        }
    }

    /* return immediately if we only want SVG glyphs */
    if load_flags & FT_LOAD_SVG_ONLY != 0 {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    tt_loader_init(&mut loader, face, load_flags, false)?;

    let error = tt_load_glyph_body(&mut loader, face, stream, glyph_index, load_flags);

    /* Done: */
    tt_loader_done(&mut loader, face, false);

    /* Exit: */
    error
}

fn tt_load_glyph_body(
    loader: &mut TtLoaderRec,
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    glyph_index: FtUInt,
    load_flags: FtInt32,
) -> FtResult<()> {
    /* done if we are only interested in the `hdmx` advance */
    if load_flags & FT_LOAD_ADVANCE_ONLY != 0 && (load_flags & FT_LOAD_VERTICAL_LAYOUT) == 0 {
        if let Some(w) = loader.widthp {
            face.root.glyph.metrics.horiAdvance =
                face.hdmx_table[w + glyph_index as usize] as FtPos * 64;
            return Ok(());
        }
    }

    face.root.glyph.format = FT_GLYPH_FORMAT_OUTLINE;
    face.root.glyph.num_subglyphs = 0;
    face.root.glyph.outline.flags = 0;

    /* main loading loop */
    let mut error = load_truetype_glyph(loader, face, stream, glyph_index, 0, false);
    if error.is_ok() {
        if face.root.glyph.format == FT_GLYPH_FORMAT_COMPOSITE {
            let n = loader.gloader.base.num_subglyphs;
            face.root.glyph.num_subglyphs = n;
            face.root.glyph.subglyphs = loader.gloader.base.subglyphs[..n as usize].to_vec();
        } else {
            let base = &loader.gloader.base.outline;
            let (n, nc) = (
                base.n_points.max(0) as usize,
                base.n_contours.max(0) as usize,
            );
            let glyph = &mut face.root.glyph;
            glyph.outline = FtOutline {
                n_contours: base.n_contours,
                n_points: base.n_points,
                points: base.points[..n].to_vec(),
                tags: base.tags[..n].to_vec(),
                contours: base.contours[..nc].to_vec(),
                flags: base.flags,
            };
            glyph.outline.flags &= !FT_OUTLINE_SINGLE_PASS;

            /* Translate array so that (0,0) is the glyph's origin.  Note  */
            /* that this behaviour is independent on the value of bit 1 of */
            /* the `flags' field in the `head' table -- at least major     */
            /* applications like Acroread indicate that.                   */
            if loader.pp1.x != 0 {
                ft_outline_translate(&mut glyph.outline, -loader.pp1.x, 0);
            }
        }

        /* TT_USE_BYTECODE_INTERPRETER */
        if is_hinted(load_flags as FtULong) {
            let exec = loader
                .exec
                .as_ref()
                .expect("hinting without an execution context");
            let glyph = &mut face.root.glyph;

            glyph.control_data = exec.glyph_ins.clone();
            glyph.control_len = exec.glyph_size as i64;

            if exec.gs.scan_control {
                /* convert scan conversion mode to FT_OUTLINE_XXX flags */
                match exec.gs.scan_type {
                    0 => {
                        /* simple drop-outs including stubs */
                        glyph.outline.flags |= FT_OUTLINE_INCLUDE_STUBS;
                    }
                    1 => {
                        /* simple drop-outs excluding stubs */
                        /* nothing; it's the default rendering mode */
                    }
                    4 => {
                        /* smart drop-outs including stubs */
                        glyph.outline.flags |= FT_OUTLINE_SMART_DROPOUTS | FT_OUTLINE_INCLUDE_STUBS;
                    }
                    5 => {
                        /* smart drop-outs excluding stubs  */
                        glyph.outline.flags |= FT_OUTLINE_SMART_DROPOUTS;
                    }
                    _ => {
                        /* no drop-out control */
                        glyph.outline.flags |= FT_OUTLINE_IGNORE_DROPOUTS;
                    }
                }
            } else {
                glyph.outline.flags |= FT_OUTLINE_IGNORE_DROPOUTS;
            }
        }

        error = compute_glyph_metrics(loader, face, glyph_index);
    }

    /* Set the `high precision' bit flag.                           */
    /* This is _critical_ to get correct output for monochrome      */
    /* TrueType glyphs at all sizes using the bytecode interpreter. */
    /*                                                              */
    if (load_flags & FT_LOAD_NO_SCALE) == 0 && face.size_metrics().y_ppem < 24 {
        face.root.glyph.outline.flags |= FT_OUTLINE_HIGH_PRECISION;
    }

    error
}
