// Rust translation of src/autofit/afcjk.c and afcjk.h from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it), with
// `AF_CONFIG_OPTION_CJK` defined (and `AF_CONFIG_OPTION_CJK_BLUE_HANI_VERT`
// not).
// Copyright (C) 2006-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Auto-fitter hinting routines for CJK writing system.
//!
//! The algorithm is based on akito's autohint patch, archived at
//!
//! <https://web.archive.org/web/20051219160454/http://www.kde.gr.jp:80/~akito/patch/freetype2/2.1.7/>

use super::super::base::ftcalc::{ft_div_fix, ft_mul_div, ft_mul_fix};
use super::super::base::ftobjs::*;
use super::super::fttypes::*;
use super::afblue::*;
use super::afglobal::*;
use super::afhints::*;
use super::aflatin::*;
use super::afscript::AF_SCRIPT_CLASSES;
use super::afshaper::*;
use super::aftypes::*;

/* the CJK-specific writing system */

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****              C J K   G L O B A L   M E T R I C S              *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/*
 * CJK glyphs tend to fill the square.  So we have both vertical and
 * horizontal blue zones.  But some glyphs have flat bounding strokes that
 * leave some space between neighbour glyphs.
 */

/// `AF_CJK_IS_TOP_BLUE`
fn af_cjk_is_top_blue(b: &AfBlueStringRec) -> bool {
    b.properties & AF_BLUE_PROPERTY_CJK_TOP != 0
}
/// `AF_CJK_IS_HORIZ_BLUE`
fn af_cjk_is_horiz_blue(b: &AfBlueStringRec) -> bool {
    b.properties & AF_BLUE_PROPERTY_CJK_HORIZ != 0
}
/// `AF_CJK_IS_RIGHT_BLUE`
fn af_cjk_is_right_blue(b: &AfBlueStringRec) -> bool {
    af_cjk_is_top_blue(b)
}

pub const AF_CJK_MAX_WIDTHS: usize = 16;

pub const AF_CJK_BLUE_ACTIVE: FtUInt = 1 << 0; /* zone height is <= 3/4px      */
pub const AF_CJK_BLUE_TOP: FtUInt = 1 << 1; /* result of AF_CJK_IS_TOP_BLUE */
pub const AF_CJK_BLUE_ADJUSTMENT: FtUInt = 1 << 2; /* used for scale adjustment    */
/* optimization                 */

/// `AF_CJKBlueRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct AfCjkBlueRec {
    pub ref_: AfWidthRec,
    pub shoot: AfWidthRec, /* undershoot */
    pub flags: FtUInt,
}

/// `AF_CJKAxisRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct AfCjkAxisRec {
    pub scale: FtFixed,
    pub delta: FtPos,

    pub width_count: FtUInt,                     /* number of used widths */
    pub widths: [AfWidthRec; AF_CJK_MAX_WIDTHS], /* widths array          */
    pub edge_distance_threshold: FtPos,          /* used for creating edges */
    pub standard_width: FtPos,                   /* the default stem thickness */
    pub extra_light: bool,                       /* is standard width very light? */

    /* used for horizontal metrics too for CJK */
    pub control_overshoot: bool,
    pub blue_count: FtUInt,
    pub blues: [AfCjkBlueRec; AF_BLUE_STRINGSET_MAX_LEN],

    pub org_scale: FtFixed,
    pub org_delta: FtPos,
}

/// `AF_CJKMetricsRec` (without its `root`, which is the
/// [`AfStyleMetricsRec`] holding it)
#[derive(Debug, Clone, Copy, Default)]
pub struct AfCjkMetricsRec {
    pub units_per_em: FtUInt,
    pub axis: [AfCjkAxisRec; AF_DIMENSION_MAX],
}

/* afcjk.c */

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****              C J K   G L O B A L   M E T R I C S              *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/* Basically the Latin version with AF_CJKMetrics */
/* to replace AF_LatinMetrics.                    */

/// `af_cjk_metrics_init_widths`
pub fn af_cjk_metrics_init_widths(
    metrics: &mut AfStyleMetricsRec,
    face: &mut FtFace,
    globals: &mut AfFaceGlobalsRec,
) {
    /* scan the array of segments in each direction */
    let mut hints = af_glyph_hints_init();

    metrics.cjk_mut().axis[AF_DIMENSION_HORZ].width_count = 0;
    metrics.cjk_mut().axis[AF_DIMENSION_VERT].width_count = 0;

    {
        let style_class = metrics.style_class;
        let script_class = &AF_SCRIPT_CLASSES[style_class.script as usize];

        let mut shaper_buf = af_shaper_buf_create(face);

        let p_str = script_class.standard_charstring;
        let mut p: usize = 0;

        /* We check a list of standard characters.  The first match wins. */

        let mut glyph_index: FtULong = 0;
        while p_str[p] != 0 {
            while p_str[p] == b' ' {
                p += 1;
            }

            /* reject input that maps to more than a single glyph */
            let num_idx;
            (p, num_idx) = af_shaper_get_cluster(
                p_str,
                p,
                metrics.style_class,
                face.units_per_EM,
                globals,
                &mut shaper_buf,
            );
            if num_idx > 1 {
                continue;
            }

            /* otherwise exit loop if we have a result */
            glyph_index = af_shaper_get_elem(&mut shaper_buf, 0, None, None);
            if glyph_index != 0 {
                break;
            }
        }

        af_shaper_buf_destroy(face, shaper_buf);

        'exit: {
            if glyph_index == 0 {
                break 'exit;
            }

            if glyph_index == 0 {
                break 'exit;
            }

            let error = ft_load_glyph(face, glyph_index as FtUInt, FT_LOAD_NO_SCALE);
            if error.is_err() || face.glyph.outline.n_points <= 0 {
                break 'exit;
            }

            let units_per_em = metrics.cjk().units_per_em;
            let dummy = AfStyleMetricsRec {
                style_class: metrics.style_class,
                scaler: AfScalerRec {
                    x_scale: 0x10000,
                    y_scale: 0x10000,
                    x_delta: 0,
                    y_delta: 0,
                    face: AfScalerFace::of(face),
                    render_mode: FT_RENDER_MODE_NORMAL,
                    flags: 0,
                },
                digits_have_same_width: false,
                ws: AfWritingSystemMetrics::Cjk(Box::new(AfCjkMetricsRec {
                    units_per_em,
                    ..Default::default()
                })),
            };

            af_glyph_hints_rescale(&mut hints, &dummy);

            if af_glyph_hints_reload(&mut hints, &face.glyph.outline).is_err() {
                break 'exit;
            }

            for dim in 0..AF_DIMENSION_MAX {
                let mut num_widths: FtUInt = 0;

                if af_latin_hints_compute_segments(&mut hints, &dummy, dim).is_err() {
                    break 'exit;
                }

                /*
                 * We assume that the glyphs selected for the stem width
                 * computation are `featureless' enough so that the linking
                 * algorithm works fine without adjustments of its scoring
                 * function.
                 */
                af_latin_hints_link_segments(&mut hints, &dummy, 0, &[], dim);

                let axis = &mut metrics.cjk_mut().axis[dim];
                let axhints = &hints.axis[dim];
                for seg in 0..axhints.num_segments as usize {
                    let link = axhints.segments[seg].link;

                    /* we only consider stem segments there! */
                    if let Some(link) = link {
                        if axhints.segments[link].link == Some(seg) && link > seg {
                            let mut dist = axhints.segments[seg].pos as FtPos
                                - axhints.segments[link].pos as FtPos;
                            if dist < 0 {
                                dist = -dist;
                            }

                            if (num_widths as usize) < AF_CJK_MAX_WIDTHS {
                                axis.widths[num_widths as usize].org = dist;
                                num_widths += 1;
                            }
                        }
                    }
                }

                /* this also replaces multiple almost identical stem widths */
                /* with a single one (the value 100 is heuristic)           */
                af_sort_and_quantize_widths(
                    &mut num_widths,
                    &mut axis.widths,
                    units_per_em as FtPos / 100,
                );
                axis.width_count = num_widths;
            }
        }

        /* Exit: */
        let constant = af_latin_constant(metrics, 50);
        for dim in 0..AF_DIMENSION_MAX {
            let axis = &mut metrics.cjk_mut().axis[dim];

            let stdw = if axis.width_count > 0 {
                axis.widths[0].org
            } else {
                constant
            };

            /* let's try 20% of the smallest width */
            axis.edge_distance_threshold = stdw / 5;
            axis.standard_width = stdw;
            axis.extra_light = false;
        }
    }

    af_glyph_hints_done(&mut hints);
}

/* Find all blue zones. */

/// `af_cjk_metrics_init_blues`
fn af_cjk_metrics_init_blues(
    metrics: &mut AfStyleMetricsRec,
    face: &mut FtFace,
    globals: &mut AfFaceGlobalsRec,
) {
    let mut fills: [FtPos; AF_BLUE_STRING_MAX_LEN] = [0; AF_BLUE_STRING_MAX_LEN];
    let mut flats: [FtPos; AF_BLUE_STRING_MAX_LEN] = [0; AF_BLUE_STRING_MAX_LEN];

    let sc = metrics.style_class;

    let bss = sc.blue_stringset;
    let mut bs = bss as usize;

    let mut shaper_buf = af_shaper_buf_create(face);

    /* we walk over the blue character strings as specified in the   */
    /* style's entry in the `af_blue_stringset' array, computing its */
    /* extremum points (depending on the string properties)          */

    while AF_BLUE_STRINGSETS[bs].string != AF_BLUE_STRING_MAX {
        let bsr = AF_BLUE_STRINGSETS[bs];
        let p_str = &AF_BLUE_STRINGS[..];
        let mut p = bsr.string as usize;

        let dim = if af_cjk_is_horiz_blue(&bsr) {
            AF_DIMENSION_HORZ
        } else {
            AF_DIMENSION_VERT
        };

        let mut num_fills: FtUInt = 0;
        let mut num_flats: FtUInt = 0;

        let mut fill = true; /* start with characters that define fill values */

        while p_str[p] != 0 {
            while p_str[p] == b' ' {
                p += 1;
            }

            /* switch to characters that define flat values */
            if p_str[p] == b'|' {
                fill = false;
                p += 1;
                continue;
            }

            /* reject input that maps to more than a single glyph */
            let num_idx;
            (p, num_idx) = af_shaper_get_cluster(
                p_str,
                p,
                metrics.style_class,
                face.units_per_EM,
                globals,
                &mut shaper_buf,
            );
            if num_idx > 1 {
                continue;
            }

            /* load the character in the face -- skip unknown or empty ones */
            let glyph_index = af_shaper_get_elem(&mut shaper_buf, 0, None, None);

            if glyph_index == 0 {
                continue;
            }

            let error = ft_load_glyph(face, glyph_index as FtUInt, FT_LOAD_NO_SCALE);
            let outline = &face.glyph.outline;
            if error.is_err() || outline.n_points <= 2 {
                continue;
            }

            /* now compute min or max point indices and coordinates */
            let points = &outline.points;
            let mut best_point: FtInt = -1;
            let mut best_pos: FtPos = 0; /* make compiler happy */

            {
                let mut last: FtInt = -1;
                for nn in 0..outline.n_contours as usize {
                    let first = last + 1;
                    last = outline.contours[nn] as FtInt;

                    /* Avoid single-point contours since they are never rasterized. */
                    /* In some fonts, they correspond to mark attachment points     */
                    /* which are way outside of the glyph's real outline.           */
                    if last <= first {
                        continue;
                    }

                    if af_cjk_is_horiz_blue(&bsr) {
                        if af_cjk_is_right_blue(&bsr) {
                            for pp in first..=last {
                                if best_point < 0 || points[pp as usize].x > best_pos {
                                    best_point = pp;
                                    best_pos = points[pp as usize].x;
                                }
                            }
                        } else {
                            for pp in first..=last {
                                if best_point < 0 || points[pp as usize].x < best_pos {
                                    best_point = pp;
                                    best_pos = points[pp as usize].x;
                                }
                            }
                        }
                    } else if af_cjk_is_top_blue(&bsr) {
                        for pp in first..=last {
                            if best_point < 0 || points[pp as usize].y > best_pos {
                                best_point = pp;
                                best_pos = points[pp as usize].y;
                            }
                        }
                    } else {
                        for pp in first..=last {
                            if best_point < 0 || points[pp as usize].y < best_pos {
                                best_point = pp;
                                best_pos = points[pp as usize].y;
                            }
                        }
                    }
                }
            }

            if fill {
                fills[num_fills as usize] = best_pos;
                num_fills += 1;
            } else {
                flats[num_flats as usize] = best_pos;
                num_flats += 1;
            }
        } /* end while loop */

        if num_flats == 0 && num_fills == 0 {
            /*
             * we couldn't find a single glyph to compute this blue zone,
             * we will simply ignore it then
             */
            bs += 1;
            continue;
        }

        /* we have computed the contents of the `fill' and `flats' tables,   */
        /* now determine the reference and overshoot position of the blue -- */
        /* we simply take the median value after a simple sort               */
        af_sort_pos(num_fills, &mut fills);
        af_sort_pos(num_flats, &mut flats);

        let axis = &mut metrics.cjk_mut().axis[dim];
        let blue = &mut axis.blues[axis.blue_count as usize];
        axis.blue_count += 1;

        if num_flats == 0 {
            blue.ref_.org = fills[num_fills as usize / 2];
            blue.shoot.org = blue.ref_.org;
        } else if num_fills == 0 {
            blue.ref_.org = flats[num_flats as usize / 2];
            blue.shoot.org = blue.ref_.org;
        } else {
            blue.ref_.org = fills[num_fills as usize / 2];
            blue.shoot.org = flats[num_flats as usize / 2];
        }

        /* make sure blue_ref >= blue_shoot for top/right or */
        /* vice versa for bottom/left                        */
        if blue.shoot.org != blue.ref_.org {
            let ref_ = blue.ref_.org;
            let shoot = blue.shoot.org;
            let under_ref = shoot < ref_;

            /* AF_CJK_IS_TOP_BLUE covers `right' and `top' */
            if af_cjk_is_top_blue(&bsr) ^ under_ref {
                blue.ref_.org = (shoot + ref_) / 2;
                blue.shoot.org = blue.ref_.org;
            }
        }

        blue.flags = 0;
        if af_cjk_is_top_blue(&bsr) {
            blue.flags |= AF_CJK_BLUE_TOP;
        }

        bs += 1;
    } /* end for loop */

    af_shaper_buf_destroy(face, shaper_buf);
}

/* Basically the Latin version with type AF_CJKMetrics for metrics. */

/// `af_cjk_metrics_check_digits`
pub fn af_cjk_metrics_check_digits(
    metrics: &mut AfStyleMetricsRec,
    face: &mut FtFace,
    globals: &mut AfFaceGlobalsRec,
) {
    let mut started = false;
    let mut same_width = true;
    let mut advance: FtLong = 0;
    let mut old_advance: FtLong = 0;

    let mut shaper_buf = af_shaper_buf_create(face);

    /* in all supported charmaps, digits have character codes 0x30-0x39 */
    let digits: &[u8] = b"0 1 2 3 4 5 6 7 8 9\0";
    let mut p: usize = 0;

    while digits[p] != 0 {
        /* reject input that maps to more than a single glyph */
        let num_idx;
        (p, num_idx) = af_shaper_get_cluster(
            digits,
            p,
            metrics.style_class,
            face.units_per_EM,
            globals,
            &mut shaper_buf,
        );
        if num_idx > 1 {
            continue;
        }

        let glyph_index = af_shaper_get_elem(&mut shaper_buf, 0, Some(&mut advance), None);
        if glyph_index == 0 {
            continue;
        }

        if started {
            if advance != old_advance {
                same_width = false;
                break;
            }
        } else {
            old_advance = advance;
            started = true;
        }
    }

    af_shaper_buf_destroy(face, shaper_buf);

    metrics.digits_have_same_width = same_width;
}

/* Initialize global metrics. */

/// `af_cjk_metrics_init`
pub fn af_cjk_metrics_init(
    metrics: &mut AfStyleMetricsRec, /* AF_CJKMetrics */
    face: &mut FtFace,
    globals: &mut AfFaceGlobalsRec,
) -> FtResult<()> {
    let oldmap = face.charmap;

    metrics.cjk_mut().units_per_em = face.units_per_EM as FtUInt;

    if ft_select_charmap(face, FT_ENCODING_UNICODE).is_ok() {
        af_cjk_metrics_init_widths(metrics, face, globals);
        af_cjk_metrics_init_blues(metrics, face, globals);
        af_cjk_metrics_check_digits(metrics, face, globals);
    }

    face.charmap = oldmap;
    Ok(())
}

/* Adjust scaling value, then scale and shift widths   */
/* and blue zones (if applicable) for given dimension. */

/// `af_cjk_metrics_scale_dim`
fn af_cjk_metrics_scale_dim(
    metrics: &mut AfStyleMetricsRec,
    scaler: &AfScalerRec,
    dim: AfDimension,
) {
    let scale: FtFixed;
    let delta: FtPos;

    if dim == AF_DIMENSION_HORZ {
        scale = scaler.x_scale;
        delta = scaler.x_delta;
    } else {
        scale = scaler.y_scale;
        delta = scaler.y_delta;
    }

    let axis = &mut metrics.cjk_mut().axis[dim];

    if axis.org_scale == scale && axis.org_delta == delta {
        return;
    }

    axis.org_scale = scale;
    axis.org_delta = delta;

    axis.scale = scale;
    axis.delta = delta;

    /* scale the blue zones */
    for nn in 0..axis.blue_count as usize {
        let blue = &mut axis.blues[nn];

        blue.ref_.cur = ft_mul_fix(blue.ref_.org, scale) + delta;
        blue.ref_.fit = blue.ref_.cur;
        blue.shoot.cur = ft_mul_fix(blue.shoot.org, scale) + delta;
        blue.shoot.fit = blue.shoot.cur;
        blue.flags &= !AF_CJK_BLUE_ACTIVE;

        /* a blue zone is only active if it is less than 3/4 pixels tall */
        let dist = ft_mul_fix(blue.ref_.org - blue.shoot.org, scale);
        if (-48..=48).contains(&dist) {
            blue.ref_.fit = ft_pix_round(blue.ref_.cur);

            /* shoot is under shoot for cjk */
            let delta1 = ft_div_fix(blue.ref_.fit, scale) - blue.shoot.org;
            let mut delta2 = delta1;
            if delta1 < 0 {
                delta2 = -delta2;
            }

            delta2 = ft_mul_fix(delta2, scale);

            if delta2 < 32 {
                delta2 = 0;
            } else {
                delta2 = ft_pix_round(delta2);
            }

            if delta1 < 0 {
                delta2 = -delta2;
            }

            blue.shoot.fit = blue.ref_.fit - delta2;

            blue.flags |= AF_CJK_BLUE_ACTIVE;
        }
    }
}

/* Scale global values in both directions. */

/// `af_cjk_metrics_scale`
pub fn af_cjk_metrics_scale(
    metrics: &mut AfStyleMetricsRec, /* AF_CJKMetrics */
    scaler: &AfScalerRec,
    _globals: &AfFaceGlobalsRec,
) {
    /* we copy the whole structure since the x and y scaling values */
    /* are not modified, contrary to e.g. the `latin' auto-hinter   */
    metrics.scaler = *scaler;

    af_cjk_metrics_scale_dim(metrics, scaler, AF_DIMENSION_HORZ);
    af_cjk_metrics_scale_dim(metrics, scaler, AF_DIMENSION_VERT);
}

/* Extract standard_width from writing system/script specific */
/* metrics class.                                             */

/// `af_cjk_get_standard_widths`
fn af_cjk_get_standard_widths(metrics: &AfStyleMetricsRec, /* AF_CJKMetrics */) -> (FtPos, FtPos) {
    let cjk = metrics.cjk();
    (
        cjk.axis[AF_DIMENSION_VERT].standard_width,
        cjk.axis[AF_DIMENSION_HORZ].standard_width,
    )
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****              C J K   G L Y P H   A N A L Y S I S              *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/* Walk over all contours and compute its segments. */

/// `af_cjk_hints_compute_segments`
fn af_cjk_hints_compute_segments(
    hints: &mut AfGlyphHintsRec,
    metrics: &AfStyleMetricsRec,
    dim: AfDimension,
) -> FtResult<()> {
    /* FIXME (upstream): the segment limit is taken before the segments */
    /* are computed (when there are none: `af_glyph_hints_reload' resets */
    /* them), so the loop below never runs                              */
    let segment_limit = hints.axis[dim].num_segments as usize;

    af_latin_hints_compute_segments(hints, metrics, dim)?;

    /* a segment is round if it doesn't have successive */
    /* on-curve points.                                 */
    let points = &hints.points;
    let axis = &mut hints.axis[dim];
    for seg in &mut axis.segments[..segment_limit] {
        let mut pt = seg.first;
        let last = seg.last;
        let mut f0 = points[pt].flags & AF_FLAG_CONTROL;
        let mut f1: FtUShort;

        seg.flags &= !AF_EDGE_ROUND;

        while pt != last {
            pt = points[pt].next;
            f1 = points[pt].flags & AF_FLAG_CONTROL;

            if f0 == 0 && f1 == 0 {
                break;
            }

            if pt == last {
                seg.flags |= AF_EDGE_ROUND;
            }

            f0 = f1;
        }
    }

    Ok(())
}

/// `af_cjk_hints_link_segments`
fn af_cjk_hints_link_segments(
    hints: &mut AfGlyphHintsRec,
    metrics: &AfStyleMetricsRec,
    dim: AfDimension,
) {
    let x_scale = hints.x_scale;
    let y_scale = hints.y_scale;
    let axis = &mut hints.axis[dim];
    let segment_limit = axis.num_segments as usize;
    let major_dir = axis.major_dir;
    let segments = &mut axis.segments;

    let len_threshold = af_latin_constant(metrics, 8);

    let mut dist_threshold = if dim == AF_DIMENSION_HORZ {
        x_scale
    } else {
        y_scale
    };
    dist_threshold = ft_div_fix(64 * 3, dist_threshold);

    /* now compare each segment to the others */
    for seg1 in 0..segment_limit {
        if segments[seg1].dir as AfDirection != major_dir {
            continue;
        }

        for seg2 in 0..segment_limit {
            if seg2 != seg1 && segments[seg1].dir as i32 + segments[seg2].dir as i32 == 0 {
                let dist = segments[seg2].pos as FtPos - segments[seg1].pos as FtPos;

                if dist < 0 {
                    continue;
                }

                {
                    let mut min = segments[seg1].min_coord as FtPos;
                    let mut max = segments[seg1].max_coord as FtPos;

                    if min < segments[seg2].min_coord as FtPos {
                        min = segments[seg2].min_coord as FtPos;
                    }

                    if max > segments[seg2].max_coord as FtPos {
                        max = segments[seg2].max_coord as FtPos;
                    }

                    let len = max - min;
                    if len >= len_threshold {
                        if dist * 8 < segments[seg1].score * 9
                            && (dist * 8 < segments[seg1].score * 7 || segments[seg1].len < len)
                        {
                            segments[seg1].score = dist;
                            segments[seg1].len = len;
                            segments[seg1].link = Some(seg2);
                        }

                        if dist * 8 < segments[seg2].score * 9
                            && (dist * 8 < segments[seg2].score * 7 || segments[seg2].len < len)
                        {
                            segments[seg2].score = dist;
                            segments[seg2].len = len;
                            segments[seg2].link = Some(seg1);
                        }
                    }
                }
            }
        }
    }

    /*
     * now compute the `serif' segments
     *
     * In Hanzi, some strokes are wider on one or both of the ends.
     * We either identify the stems on the ends as serifs or remove
     * the linkage, depending on the length of the stems.
     *
     */

    {
        for seg1 in 0..segment_limit {
            let Some(link1) = segments[seg1].link else {
                continue;
            };
            if segments[link1].link != Some(seg1) || segments[link1].pos <= segments[seg1].pos {
                continue;
            }

            if segments[seg1].score >= dist_threshold {
                continue;
            }

            for seg2 in 0..segment_limit {
                if segments[seg2].pos > segments[seg1].pos || seg1 == seg2 {
                    continue;
                }

                let Some(link2) = segments[seg2].link else {
                    continue;
                };
                if segments[link2].link != Some(seg2) || segments[link2].pos < segments[link1].pos {
                    continue;
                }

                if segments[seg1].pos == segments[seg2].pos
                    && segments[link1].pos == segments[link2].pos
                {
                    continue;
                }

                if segments[seg2].score <= segments[seg1].score
                    || segments[seg1].score * 4 <= segments[seg2].score
                {
                    continue;
                }

                /* seg2 < seg1 < link1 < link2 */

                if segments[seg1].len >= segments[seg2].len * 3 {
                    for seg in 0..segment_limit {
                        let link = segments[seg].link;

                        if link == Some(seg2) {
                            segments[seg].link = None;
                            segments[seg].serif = Some(link1);
                        } else if link == Some(link2) {
                            segments[seg].link = None;
                            segments[seg].serif = Some(seg1);
                        }
                    }
                } else {
                    segments[seg1].link = None;
                    segments[link1].link = None;
                    break;
                }
            }
        }
    }

    for seg1 in 0..segment_limit {
        let seg2 = segments[seg1].link;

        if let Some(seg2) = seg2 {
            if segments[seg2].link != Some(seg1) {
                segments[seg1].link = None;

                if segments[seg2].score < dist_threshold
                    || segments[seg1].score < segments[seg2].score * 4
                {
                    segments[seg1].serif = segments[seg2].link;
                }
            }
        }
    }
}

/// `af_cjk_hints_compute_edges`
fn af_cjk_hints_compute_edges(
    hints: &mut AfGlyphHintsRec,
    metrics: &AfStyleMetricsRec,
    dim: AfDimension,
) -> FtResult<()> {
    let laxis = &metrics.cjk().axis[dim];

    let scale: FtFixed = if dim == AF_DIMENSION_HORZ {
        hints.x_scale
    } else {
        hints.y_scale
    };

    let axis = &mut hints.axis[dim];
    let segment_limit = axis.num_segments as usize;

    axis.num_edges = 0;

    /**********************************************************************
     *
     * We begin by generating a sorted table of edges for the current
     * direction.  To do so, we simply scan each segment and try to find
     * an edge in our table that corresponds to its position.
     *
     * If no edge is found, we create and insert a new edge in the
     * sorted table.  Otherwise, we simply add the segment to the edge's
     * list which is then processed in the second step to compute the
     * edge's properties.
     *
     * Note that the edges table is sorted along the segment/edge
     * position.
     *
     */

    let mut edge_distance_threshold = ft_mul_fix(laxis.edge_distance_threshold, scale);
    if edge_distance_threshold > 64 / 4 {
        edge_distance_threshold = ft_div_fix(64 / 4, scale);
    } else {
        edge_distance_threshold = laxis.edge_distance_threshold;
    }

    for seg in 0..segment_limit {
        let mut found: Option<usize> = None;
        let mut best: FtPos = 0xFFFF;

        /* look for an edge corresponding to the segment */
        for ee in 0..axis.num_edges as usize {
            let edge = axis.edges[ee];

            if edge.dir != axis.segments[seg].dir {
                continue;
            }

            let mut dist = axis.segments[seg].pos as FtPos - edge.fpos as FtPos;
            if dist < 0 {
                dist = -dist;
            }

            if dist < edge_distance_threshold && dist < best {
                let link = axis.segments[seg].link;

                /* check whether all linked segments of the candidate edge */
                /* can make a single edge.                                 */
                if let Some(link) = link {
                    let first = edge.first.expect("edge with segments");
                    let mut seg1 = first;
                    let mut dist2: FtPos = 0;

                    loop {
                        let link1 = axis.segments[seg1].link;

                        if let Some(link1) = link1 {
                            dist2 = af_segment_dist(&axis.segments[link], &axis.segments[link1]);
                            if dist2 >= edge_distance_threshold {
                                break;
                            }
                        }

                        seg1 = axis.segments[seg1].edge_next.expect("segment in an edge");
                        if seg1 == first {
                            break;
                        }
                    }

                    if dist2 >= edge_distance_threshold {
                        continue;
                    }
                }

                best = dist;
                found = Some(ee);
            }
        }

        if let Some(found) = found {
            /* if an edge was found, simply add the segment to the edge's */
            /* list                                                       */
            axis.segments[seg].edge_next = axis.edges[found].first;
            let found_last = axis.edges[found].last.expect("edge with segments");
            axis.segments[found_last].edge_next = Some(seg);
            axis.edges[found].last = Some(seg);
        } else {
            /* insert a new edge in the list and */
            /* sort according to the position    */
            let s = axis.segments[seg];
            let edge = af_axis_hints_new_edge(axis, s.pos as FtInt, s.dir as AfDirection, false)?;

            /* add the segment to the new edge's list */
            let e = &mut axis.edges[edge];
            *e = AfEdgeRec::default();
            e.first = Some(seg);
            e.last = Some(seg);
            e.dir = s.dir;
            e.fpos = s.pos;
            e.opos = ft_mul_fix(s.pos as FtPos, scale);
            e.pos = e.opos;
            axis.segments[seg].edge_next = Some(seg);
        }
    }

    /*********************************************************************
     *
     * Good, we now compute each edge's properties according to the
     * segments found on its position.  Basically, these are
     *
     * - the edge's main direction
     * - stem edge, serif edge or both (which defaults to stem then)
     * - rounded edge, straight or both (which defaults to straight)
     * - link for edge
     *
     */

    /* first of all, set the `edge' field in each segment -- this is */
    /* required in order to compute edge links                       */
    /*
     * Note that removing this loop and setting the `edge' field of each
     * segment directly in the code above slows down execution speed for
     * some reasons on platforms like the Sun.
     */
    {
        let edge_limit = axis.num_edges as usize;
        let edges = &mut axis.edges;
        let segments = &mut axis.segments;

        for edge in 0..edge_limit {
            if let Some(first) = edges[edge].first {
                let mut seg = first;
                loop {
                    segments[seg].edge = Some(edge);
                    seg = segments[seg].edge_next.expect("segment in an edge");
                    if seg == first {
                        break;
                    }
                }
            }
        }

        /* now compute each edge properties */
        for edge in 0..edge_limit {
            let mut is_round: FtInt = 0; /* does it contain round segments?    */
            let mut is_straight: FtInt = 0; /* does it contain straight segments? */

            if let Some(first) = edges[edge].first {
                let mut seg = first;

                loop {
                    /* check for roundness of segment */
                    if segments[seg].flags & AF_EDGE_ROUND != 0 {
                        is_round += 1;
                    } else {
                        is_straight += 1;
                    }

                    /* check for links -- if seg->serif is set, then seg->link must */
                    /* be ignored                                                   */
                    let is_serif = match segments[seg].serif {
                        Some(serif) => segments[serif].edge != Some(edge),
                        None => false,
                    };

                    if segments[seg].link.is_some() || is_serif {
                        let mut edge2 = edges[edge].link;
                        let mut seg2 = segments[seg].link;

                        if is_serif {
                            seg2 = segments[seg].serif;
                            edge2 = edges[edge].serif;
                        }

                        let seg2 = seg2.expect("linked segment");

                        if let Some(e2) = edge2 {
                            let mut edge_delta =
                                edges[edge].fpos as FtPos - edges[e2].fpos as FtPos;
                            if edge_delta < 0 {
                                edge_delta = -edge_delta;
                            }

                            let seg_delta = af_segment_dist(&segments[seg], &segments[seg2]);
                            if seg_delta < edge_delta {
                                edge2 = segments[seg2].edge;
                            }
                        } else {
                            edge2 = segments[seg2].edge;
                        }

                        if is_serif {
                            edges[edge].serif = edge2;
                            /* (every segment has an edge here, so edge2 does) */
                            if let Some(e2) = edge2 {
                                edges[e2].flags |= AF_EDGE_SERIF;
                            }
                        } else {
                            edges[edge].link = edge2;
                        }
                    }

                    seg = segments[seg].edge_next.expect("segment in an edge");
                    if seg == first {
                        break;
                    }
                }
            }

            /* Skip_Loop: */
            /* set the round/straight flags */
            edges[edge].flags = AF_EDGE_NORMAL;

            if is_round > 0 && is_round >= is_straight {
                edges[edge].flags |= AF_EDGE_ROUND;
            }

            /* get rid of serifs if link is set                 */
            /* XXX: This gets rid of many unpleasant artefacts! */
            /*      Example: the `c' in cour.pfa at size 13     */

            if edges[edge].serif.is_some() && edges[edge].link.is_some() {
                edges[edge].serif = None;
            }
        }
    }

    Ok(())
}

/* Detect segments and edges for given dimension. */

/// `af_cjk_hints_detect_features`
fn af_cjk_hints_detect_features(
    hints: &mut AfGlyphHintsRec,
    metrics: &AfStyleMetricsRec,
    dim: AfDimension,
) -> FtResult<()> {
    af_cjk_hints_compute_segments(hints, metrics, dim)?;
    af_cjk_hints_link_segments(hints, metrics, dim);
    af_cjk_hints_compute_edges(hints, metrics, dim)
}

/* Compute all edges which lie within blue zones. */

/// `af_cjk_hints_compute_blue_edges`
fn af_cjk_hints_compute_blue_edges(
    hints: &mut AfGlyphHintsRec,
    metrics: &AfStyleMetricsRec,
    dim: AfDimension,
) {
    let cmetrics = metrics.cjk();
    let axis = &mut hints.axis[dim];
    let major_dir = axis.major_dir;
    let edge_limit = axis.num_edges as usize;
    let cjk = &cmetrics.axis[dim];
    let scale = cjk.scale;

    /* compute the initial threshold as a fraction of the EM size */
    let mut best_dist0 = ft_mul_fix(cmetrics.units_per_em as FtPos / 40, scale); /* initial threshold */

    if best_dist0 > 64 / 2 {
        /* maximum 1/2 pixel */
        best_dist0 = 64 / 2;
    }

    /* compute which blue zones are active, i.e. have their scaled */
    /* size < 3/4 pixels                                           */

    /* If the distant between an edge and a blue zone is shorter than */
    /* best_dist0, set the blue zone for the edge.  Then search for   */
    /* the blue zone with the smallest best_dist to the edge.         */

    for edge in &mut axis.edges[..edge_limit] {
        let mut best_blue: Option<AfWidthRec> = None;
        let mut best_dist = best_dist0;

        for bb in 0..cjk.blue_count as usize {
            let blue = &cjk.blues[bb];

            /* skip inactive blue zones (i.e., those that are too small) */
            if blue.flags & AF_CJK_BLUE_ACTIVE == 0 {
                continue;
            }

            /* if it is a top zone, check for right edges -- if it is a bottom */
            /* zone, check for left edges                                      */
            /*                                                                 */
            /* of course, that's for TrueType                                  */
            let is_top_right_blue = (blue.flags & AF_CJK_BLUE_TOP) != 0;
            let is_major_dir = edge.dir as AfDirection == major_dir;

            /* if it is a top zone, the edge must be against the major    */
            /* direction; if it is a bottom zone, it must be in the major */
            /* direction                                                  */
            if is_top_right_blue ^ is_major_dir {
                /* Compare the edge to the closest blue zone type */
                let compare = if ft_abs(edge.fpos as FtPos - blue.ref_.org)
                    > ft_abs(edge.fpos as FtPos - blue.shoot.org)
                {
                    &blue.shoot
                } else {
                    &blue.ref_
                };

                let mut dist = edge.fpos as FtPos - compare.org;
                if dist < 0 {
                    dist = -dist;
                }

                dist = ft_mul_fix(dist, scale);
                if dist < best_dist {
                    best_dist = dist;
                    best_blue = Some(*compare);
                }
            }
        }

        if best_blue.is_some() {
            edge.blue_edge = best_blue;
        }
    }
}

/* Initalize hinting engine. */

/// `af_cjk_hints_init`
pub fn af_cjk_hints_init(
    hints: &mut AfGlyphHintsRec,
    metrics: &AfStyleMetricsRec, /* AF_CJKMetrics */
) -> FtResult<()> {
    let cjk = metrics.cjk();

    af_glyph_hints_rescale(hints, metrics);

    /*
     * correct x_scale and y_scale when needed, since they may have
     * been modified af_cjk_scale_dim above
     */
    hints.x_scale = cjk.axis[AF_DIMENSION_HORZ].scale;
    hints.x_delta = cjk.axis[AF_DIMENSION_HORZ].delta;
    hints.y_scale = cjk.axis[AF_DIMENSION_VERT].scale;
    hints.y_delta = cjk.axis[AF_DIMENSION_VERT].delta;

    /* compute flags depending on render mode, etc. */
    let mode = metrics.scaler.render_mode;

    let mut scaler_flags = hints.scaler_flags;
    let mut other_flags: FtUInt32 = 0;

    /*
     * We snap the width of vertical stems for the monochrome and
     * horizontal LCD rendering targets only.
     */
    if mode == FT_RENDER_MODE_MONO || mode == FT_RENDER_MODE_LCD {
        other_flags |= AF_LATIN_HINTS_HORZ_SNAP;
    }

    /*
     * We snap the width of horizontal stems for the monochrome and
     * vertical LCD rendering targets only.
     */
    if mode == FT_RENDER_MODE_MONO || mode == FT_RENDER_MODE_LCD_V {
        other_flags |= AF_LATIN_HINTS_VERT_SNAP;
    }

    /*
     * We adjust stems to full pixels unless in `light' or `lcd' mode.
     */
    if mode != FT_RENDER_MODE_LIGHT && mode != FT_RENDER_MODE_LCD {
        other_flags |= AF_LATIN_HINTS_STEM_ADJUST;
    }

    if mode == FT_RENDER_MODE_MONO {
        other_flags |= AF_LATIN_HINTS_MONO;
    }

    scaler_flags |= AF_SCALER_FLAG_NO_ADVANCE;

    hints.scaler_flags = scaler_flags;
    hints.other_flags = other_flags;

    Ok(())
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****          C J K   G L Y P H   G R I D - F I T T I N G          *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/* Snap a given width in scaled coordinates to one of the */
/* current standard widths.                               */

/// `af_cjk_snap_width`
fn af_cjk_snap_width(widths: &[AfWidthRec], count: FtUInt, mut width: FtPos) -> FtPos {
    let mut best: FtPos = 64 + 32 + 2;
    let mut reference = width;

    for n in 0..count as usize {
        let w = widths[n].cur;
        let mut dist = width - w;
        if dist < 0 {
            dist = -dist;
        }
        if dist < best {
            best = dist;
            reference = w;
        }
    }

    let scaled = ft_pix_round(reference);

    if width >= reference {
        if width < scaled + 48 {
            width = reference;
        }
    } else if width > scaled - 48 {
        width = reference;
    }

    width
}

/* Compute the snapped width of a given stem.                          */
/* There is a lot of voodoo in this function; changing the hard-coded  */
/* parameters influence the whole hinting process.                     */

/// `af_cjk_compute_stem_width` (`hints` is its `other_flags`)
fn af_cjk_compute_stem_width(
    other_flags: FtUInt32,
    metrics: &AfStyleMetricsRec,
    dim: AfDimension,
    width: FtPos,
    _base_flags: FtUInt,
    _stem_flags: FtUInt,
) -> FtPos {
    let axis = &metrics.cjk().axis[dim];
    let mut dist = width;
    let mut sign = false;
    let vertical = dim == AF_DIMENSION_VERT;

    if !af_latin_hints_do_stem_adjust(other_flags) {
        return width;
    }

    if dist < 0 {
        dist = -width;
        sign = true;
    }

    'done: {
        if (vertical && !af_latin_hints_do_vert_snap(other_flags))
            || (!vertical && !af_latin_hints_do_horz_snap(other_flags))
        {
            /* smooth hinting process: very lightly quantize the stem width */

            if axis.width_count > 0 && ft_abs(dist - axis.widths[0].cur) < 40 {
                dist = axis.widths[0].cur;
                if dist < 48 {
                    dist = 48;
                }

                break 'done;
            }

            if dist < 54 {
                dist += (54 - dist) / 2;
            } else if dist < 3 * 64 {
                let delta = dist & 63;
                dist &= -64;

                if delta < 10 {
                    dist += delta;
                } else if delta < 22 {
                    dist += 10;
                } else if delta < 42 {
                    dist += delta;
                } else if delta < 54 {
                    dist += 54;
                } else {
                    dist += delta;
                }
            }
        } else {
            /* strong hinting process: snap the stem width to integer pixels */

            dist = af_cjk_snap_width(&axis.widths, axis.width_count, dist);

            if vertical {
                /* in the case of vertical hinting, always round */
                /* the stem heights to integer pixels            */

                if dist >= 64 {
                    dist = (dist + 16) & !63;
                } else {
                    dist = 64;
                }
            } else if af_latin_hints_do_mono(other_flags) {
                /* monochrome horizontal hinting: snap widths to integer pixels */
                /* with a different threshold                                   */

                if dist < 64 {
                    dist = 64;
                } else {
                    dist = (dist + 32) & !63;
                }
            } else {
                /* for horizontal anti-aliased hinting, we adopt a more subtle */
                /* approach: we strengthen small stems, round stems whose size */
                /* is between 1 and 2 pixels to an integer, otherwise nothing  */

                if dist < 48 {
                    dist = (dist + 64) >> 1;
                } else if dist < 128 {
                    dist = (dist + 22) & !63;
                } else {
                    /* round otherwise to prevent color fringes in LCD mode */
                    dist = (dist + 32) & !63;
                }
            }
        }
    }

    /* Done_Width: */
    if sign {
        dist = -dist;
    }

    dist
}

/* Align one stem edge relative to the previous stem edge. */

/// `af_cjk_align_linked_edge`
fn af_cjk_align_linked_edge(
    other_flags: FtUInt32,
    metrics: &AfStyleMetricsRec,
    dim: AfDimension,
    edges: &mut [AfEdgeRec],
    base_edge: usize,
    stem_edge: usize,
) {
    let dist = edges[stem_edge].opos - edges[base_edge].opos;

    let fitted_width = af_cjk_compute_stem_width(
        other_flags,
        metrics,
        dim,
        dist,
        edges[base_edge].flags as FtUInt,
        edges[stem_edge].flags as FtUInt,
    );

    edges[stem_edge].pos = edges[base_edge].pos + fitted_width;
}

/* Shift the coordinates of the `serif' edge by the same amount */
/* as the corresponding `base' edge has been moved already.     */

/// `af_cjk_align_serif_edge`
fn af_cjk_align_serif_edge(edges: &mut [AfEdgeRec], base: usize, serif: usize) {
    edges[serif].pos = edges[base].pos + (edges[serif].opos - edges[base].opos);
}

/*************************************************************************/
/*************************************************************************/
/*************************************************************************/
/****                                                                 ****/
/****                    E D G E   H I N T I N G                      ****/
/****                                                                 ****/
/*************************************************************************/
/*************************************************************************/
/*************************************************************************/

const AF_LIGHT_MODE_MAX_HORZ_GAP: FtPos = 9;
const AF_LIGHT_MODE_MAX_VERT_GAP: FtPos = 15;
const AF_LIGHT_MODE_MAX_DELTA_ABS: FtPos = 14;

/// `af_hint_normal_stem`
fn af_hint_normal_stem(
    other_flags: FtUInt32,
    metrics: &AfStyleMetricsRec,
    edges: &mut [AfEdgeRec],
    edge: usize,
    edge2: usize,
    anchor: FtPos,
    dim: AfDimension,
) -> FtPos {
    let mut threshold: FtPos = 64;

    if !af_latin_hints_do_stem_adjust(other_flags) {
        if (edges[edge].flags & AF_EDGE_ROUND) != 0 && (edges[edge2].flags & AF_EDGE_ROUND) != 0 {
            if dim == AF_DIMENSION_VERT {
                threshold = 64 - AF_LIGHT_MODE_MAX_HORZ_GAP;
            } else {
                threshold = 64 - AF_LIGHT_MODE_MAX_VERT_GAP;
            }
        } else if dim == AF_DIMENSION_VERT {
            threshold = 64 - AF_LIGHT_MODE_MAX_HORZ_GAP / 3;
        } else {
            threshold = 64 - AF_LIGHT_MODE_MAX_VERT_GAP / 3;
        }
    }

    let org_len = edges[edge2].opos - edges[edge].opos;
    let cur_len = af_cjk_compute_stem_width(
        other_flags,
        metrics,
        dim,
        org_len,
        edges[edge].flags as FtUInt,
        edges[edge2].flags as FtUInt,
    );

    let org_center = (edges[edge].opos + edges[edge2].opos) / 2 + anchor;
    let mut cur_pos1 = org_center - cur_len / 2;
    let cur_pos2 = cur_pos1 + cur_len;
    let mut d_off1 = cur_pos1 - ft_pix_floor(cur_pos1);
    let mut d_off2 = cur_pos2 - ft_pix_floor(cur_pos2);
    let mut u_off1 = 64 - d_off1;
    let mut u_off2 = 64 - d_off2;
    let mut delta: FtPos = 0;

    'exit: {
        if d_off1 == 0 || d_off2 == 0 {
            break 'exit;
        }

        if cur_len <= threshold {
            if d_off2 < cur_len {
                if u_off1 <= d_off2 {
                    delta = u_off1;
                } else {
                    delta = -d_off2;
                }
            }

            break 'exit;
        }

        if threshold < 64
            && (d_off1 >= threshold
                || u_off1 >= threshold
                || d_off2 >= threshold
                || u_off2 >= threshold)
        {
            break 'exit;
        }

        let mut offset = cur_len & 63;

        if offset < 32 {
            if u_off1 <= offset || d_off2 <= offset {
                break 'exit;
            }
        } else {
            offset = 64 - threshold;
        }

        d_off1 = threshold - u_off1;
        u_off1 -= offset;
        u_off2 = threshold - d_off2;
        d_off2 -= offset;

        if d_off1 <= u_off1 {
            u_off1 = -d_off1;
        }

        if d_off2 <= u_off2 {
            u_off2 = -d_off2;
        }

        if ft_abs(u_off1) <= ft_abs(u_off2) {
            delta = u_off1;
        } else {
            delta = u_off2;
        }
    }

    /* Exit: */

    if !af_latin_hints_do_stem_adjust(other_flags) {
        if delta > AF_LIGHT_MODE_MAX_DELTA_ABS {
            delta = AF_LIGHT_MODE_MAX_DELTA_ABS;
        } else if delta < -AF_LIGHT_MODE_MAX_DELTA_ABS {
            delta = -AF_LIGHT_MODE_MAX_DELTA_ABS;
        }
    }

    cur_pos1 += delta;

    if edges[edge].opos < edges[edge2].opos {
        edges[edge].pos = cur_pos1;
        edges[edge2].pos = cur_pos1 + cur_len;
    } else {
        edges[edge].pos = cur_pos1 + cur_len;
        edges[edge2].pos = cur_pos1;
    }

    delta
}

/* The main grid-fitting routine. */

/// `af_cjk_hint_edges`
fn af_cjk_hint_edges(hints: &mut AfGlyphHintsRec, metrics: &AfStyleMetricsRec, dim: AfDimension) {
    let other_flags = hints.other_flags;
    let do_blues = af_hints_do_blues(hints);
    let axis = &mut hints.axis[dim];
    let edge_limit = axis.num_edges as usize;
    let edges = &mut axis.edges[..];
    let mut anchor: Option<usize> = None;
    let mut delta: FtPos = 0;
    let mut skipped: FtInt = 0;
    let mut has_last_stem = false;
    let mut last_stem_pos: FtPos = 0;

    'exit: {
        /* we begin by aligning all stems relative to the blue zone */

        if do_blues {
            for edge in 0..edge_limit {
                if edges[edge].flags & AF_EDGE_DONE != 0 {
                    continue;
                }

                let mut blue = edges[edge].blue_edge;
                let mut edge1: Option<usize> = None;
                let mut edge2 = edges[edge].link;

                if blue.is_some() {
                    edge1 = Some(edge);
                } else if let Some(e2) = edge2 {
                    if edges[e2].blue_edge.is_some() {
                        blue = edges[e2].blue_edge;
                        edge1 = Some(e2);
                        edge2 = Some(edge);
                    }
                }

                let Some(edge1) = edge1 else {
                    continue;
                };

                edges[edge1].pos = blue.expect("blue edge").fit;
                edges[edge1].flags |= AF_EDGE_DONE;

                if let Some(e2) = edge2 {
                    if edges[e2].blue_edge.is_none() {
                        af_cjk_align_linked_edge(other_flags, metrics, dim, edges, edge1, e2);
                        edges[e2].flags |= AF_EDGE_DONE;
                    }
                }

                if anchor.is_none() {
                    anchor = Some(edge);
                }
            }
        }

        /* now we align all stem edges. */
        for edge in 0..edge_limit {
            if edges[edge].flags & AF_EDGE_DONE != 0 {
                continue;
            }

            /* skip all non-stem edges */
            let Some(edge2) = edges[edge].link else {
                skipped += 1;
                continue;
            };

            /* Some CJK characters have so many stems that
             * the hinter is likely to merge two adjacent ones.
             * To solve this problem, if either edge of a stem
             * is too close to the previous one, we avoid
             * aligning the two edges, but rather interpolate
             * their locations at the end of this function in
             * order to preserve the space between the stems.
             */
            if has_last_stem
                && (edges[edge].pos < last_stem_pos + 64 || edges[edge2].pos < last_stem_pos + 64)
            {
                skipped += 1;
                continue;
            }

            /* now align the stem */

            /* this should not happen, but it's better to be safe */
            if edges[edge2].blue_edge.is_some() {
                af_cjk_align_linked_edge(other_flags, metrics, dim, edges, edge2, edge);
                edges[edge].flags |= AF_EDGE_DONE;
                continue;
            }

            if edge2 < edge {
                af_cjk_align_linked_edge(other_flags, metrics, dim, edges, edge2, edge);
                edges[edge].flags |= AF_EDGE_DONE;
                /* We rarely reaches here it seems;
                 * usually the two edges belonging
                 * to one stem are marked as DONE together
                 */
                has_last_stem = true;
                last_stem_pos = edges[edge].pos;
                continue;
            }

            if dim != AF_DIMENSION_VERT && anchor.is_none() {
                delta = af_hint_normal_stem(
                    other_flags,
                    metrics,
                    edges,
                    edge,
                    edge2,
                    0,
                    AF_DIMENSION_HORZ,
                );
            } else {
                af_hint_normal_stem(other_flags, metrics, edges, edge, edge2, delta, dim);
            }

            anchor = Some(edge);
            edges[edge].flags |= AF_EDGE_DONE;
            edges[edge2].flags |= AF_EDGE_DONE;
            has_last_stem = true;
            last_stem_pos = edges[edge2].pos;
        }

        /* make sure that lowercase m's maintain their symmetry */

        /* In general, lowercase m's have six vertical edges if they are sans */
        /* serif, or twelve if they are with serifs.  This implementation is  */
        /* based on that assumption, and seems to work very well with most    */
        /* faces.  However, if for a certain face this assumption is not      */
        /* true, the m is just rendered like before.  In addition, any stem   */
        /* correction will only be applied to symmetrical glyphs (even if the */
        /* glyph is not an m), so the potential for unwanted distortion is    */
        /* relatively low.                                                    */

        /* We don't handle horizontal edges since we can't easily assure that */
        /* the third (lowest) stem aligns with the base line; it might end up */
        /* one pixel higher or lower.                                         */

        let n_edges = edge_limit;
        if dim == AF_DIMENSION_HORZ && (n_edges == 6 || n_edges == 12) {
            let (edge1, edge2, edge3) = if n_edges == 6 { (0, 2, 4) } else { (1, 5, 9) };

            let dist1 = edges[edge2].opos - edges[edge1].opos;
            let dist2 = edges[edge3].opos - edges[edge2].opos;

            let mut span = dist1 - dist2;
            if span < 0 {
                span = -span;
            }

            if edges[edge1].link == Some(edge1 + 1)
                && edges[edge2].link == Some(edge2 + 1)
                && edges[edge3].link == Some(edge3 + 1)
                && span < 8
            {
                delta = edges[edge3].pos - (2 * edges[edge2].pos - edges[edge1].pos);
                edges[edge3].pos -= delta;
                if let Some(link) = edges[edge3].link {
                    edges[link].pos -= delta;
                }

                /* move the serifs along with the stem */
                if n_edges == 12 {
                    edges[8].pos -= delta;
                    edges[11].pos -= delta;
                }

                edges[edge3].flags |= AF_EDGE_DONE;
                if let Some(link) = edges[edge3].link {
                    edges[link].flags |= AF_EDGE_DONE;
                }
            }
        }

        if skipped == 0 {
            break 'exit;
        }

        /*
         * now hint the remaining edges (serifs and single) in order
         * to complete our processing
         */
        for edge in 0..edge_limit {
            if edges[edge].flags & AF_EDGE_DONE != 0 {
                continue;
            }

            if let Some(serif) = edges[edge].serif {
                af_cjk_align_serif_edge(edges, serif, edge);
                edges[edge].flags |= AF_EDGE_DONE;
                skipped -= 1;
            }
        }

        if skipped == 0 {
            break 'exit;
        }

        for edge in 0..edge_limit {
            if edges[edge].flags & AF_EDGE_DONE != 0 {
                continue;
            }

            let mut before = edge as isize;
            let mut after = edge;

            loop {
                before -= 1;
                if before < 0 {
                    break;
                }
                if edges[before as usize].flags & AF_EDGE_DONE != 0 {
                    break;
                }
            }

            loop {
                after += 1;
                if after >= edge_limit {
                    break;
                }
                if edges[after].flags & AF_EDGE_DONE != 0 {
                    break;
                }
            }

            if before >= 0 || after < edge_limit {
                if before < 0 {
                    af_cjk_align_serif_edge(edges, after, edge);
                } else if after >= edge_limit {
                    af_cjk_align_serif_edge(edges, before as usize, edge);
                } else {
                    let before = before as usize;
                    if edges[after].fpos == edges[before].fpos {
                        edges[edge].pos = edges[before].pos;
                    } else {
                        edges[edge].pos = edges[before].pos
                            + ft_mul_div(
                                edges[edge].fpos as FtLong - edges[before].fpos as FtLong,
                                edges[after].pos - edges[before].pos,
                                edges[after].fpos as FtLong - edges[before].fpos as FtLong,
                            );
                    }
                }
            }
        }
    }

    /* Exit: */
}

/// `af_cjk_align_edge_points`
fn af_cjk_align_edge_points(hints: &mut AfGlyphHintsRec, dim: AfDimension) {
    let snapping = (dim == AF_DIMENSION_HORZ && af_latin_hints_do_horz_snap(hints.other_flags))
        || (dim == AF_DIMENSION_VERT && af_latin_hints_do_vert_snap(hints.other_flags));

    let axis = &hints.axis[dim];
    let points = &mut hints.points;

    for edge in &axis.edges[..axis.num_edges as usize] {
        /* move the points of each segment     */
        /* in each edge to the edge's position */
        let first = edge.first.expect("edge with segments");
        let mut seg = first;

        if snapping {
            loop {
                let s = &axis.segments[seg];
                let mut point = s.first;

                loop {
                    if dim == AF_DIMENSION_HORZ {
                        points[point].x = edge.pos;
                        points[point].flags |= AF_FLAG_TOUCH_X;
                    } else {
                        points[point].y = edge.pos;
                        points[point].flags |= AF_FLAG_TOUCH_Y;
                    }

                    if point == s.last {
                        break;
                    }

                    point = points[point].next;
                }

                seg = s.edge_next.expect("segment in an edge");
                if seg == first {
                    break;
                }
            }
        } else {
            let delta = edge.pos - edge.opos;

            loop {
                let s = &axis.segments[seg];
                let mut point = s.first;

                loop {
                    if dim == AF_DIMENSION_HORZ {
                        points[point].x += delta;
                        points[point].flags |= AF_FLAG_TOUCH_X;
                    } else {
                        points[point].y += delta;
                        points[point].flags |= AF_FLAG_TOUCH_Y;
                    }

                    if point == s.last {
                        break;
                    }

                    point = points[point].next;
                }

                seg = s.edge_next.expect("segment in an edge");
                if seg == first {
                    break;
                }
            }
        }
    }
}

/* Apply the complete hinting algorithm to a CJK glyph. */

/// `af_cjk_hints_apply`
pub fn af_cjk_hints_apply(
    _glyph_index: FtUInt,
    hints: &mut AfGlyphHintsRec,
    outline: &mut FtOutline,
    metrics: &AfStyleMetricsRec, /* AF_CJKMetrics */
    _globals: &AfFaceGlobalsRec,
) -> FtResult<()> {
    af_glyph_hints_reload(hints, outline)?;

    /* analyze glyph outline */
    if af_hints_do_horizontal(hints) {
        af_cjk_hints_detect_features(hints, metrics, AF_DIMENSION_HORZ)?;

        af_cjk_hints_compute_blue_edges(hints, metrics, AF_DIMENSION_HORZ);
    }

    if af_hints_do_vertical(hints) {
        af_cjk_hints_detect_features(hints, metrics, AF_DIMENSION_VERT)?;

        af_cjk_hints_compute_blue_edges(hints, metrics, AF_DIMENSION_VERT);
    }

    /* grid-fit the outline */
    for dim in 0..AF_DIMENSION_MAX {
        if (dim == AF_DIMENSION_HORZ && af_hints_do_horizontal(hints))
            || (dim == AF_DIMENSION_VERT && af_hints_do_vertical(hints))
        {
            af_cjk_hint_edges(hints, metrics, dim);
            af_cjk_align_edge_points(hints, dim);
            af_glyph_hints_align_strong_points(hints, dim);
            af_glyph_hints_align_weak_points(hints, dim);
        }
    }

    af_glyph_hints_save(hints, outline);

    Ok(())
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                C J K   S C R I P T   C L A S S                *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `af_cjk_writing_system_class`
pub static AF_CJK_WRITING_SYSTEM_CLASS: AfWritingSystemClassRec = AfWritingSystemClassRec {
    writing_system: AF_WRITING_SYSTEM_CJK,

    /* sizeof ( AF_CJKMetricsRec ) */
    style_metrics_init: Some(af_cjk_metrics_init), /* style_metrics_init    */
    style_metrics_scale: Some(af_cjk_metrics_scale), /* style_metrics_scale   */
    style_metrics_done: None,                      /* style_metrics_done    */
    style_metrics_getstdw: Some(af_cjk_get_standard_widths), /* style_metrics_getstdw */

    style_hints_init: Some(af_cjk_hints_init), /* style_hints_init      */
    style_hints_apply: Some(af_cjk_hints_apply), /* style_hints_apply     */
};
