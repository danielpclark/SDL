// Rust translation of src/autofit/aflatin.c and aflatin.h from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2003-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Auto-fitter hinting routines for latin writing system.

use super::super::base::ftcalc::{ft_div_fix, ft_mul_div, ft_mul_fix};
use super::super::base::ftobjs::*;
use super::super::fttypes::*;
use super::afblue::*;
use super::afglobal::*;
use super::afhints::*;
use super::afscript::AF_SCRIPT_CLASSES;
use super::afshaper::*;
use super::aftypes::*;

/* the `latin' writing system */

/* constants are given with units_per_em == 2048 in mind */
/* (`AF_LATIN_CONSTANT' is `af_latin_constant' in aftypes.rs) */

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****            L A T I N   G L O B A L   M E T R I C S            *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/*
 * The following declarations could be embedded in the file `aflatin.c';
 * they have been made semi-public to allow alternate writing system
 * hinters to re-use some of them.
 */

/// `AF_LATIN_IS_TOP_BLUE`
fn af_latin_is_top_blue(b: &AfBlueStringRec) -> bool {
    b.properties & AF_BLUE_PROPERTY_LATIN_TOP != 0
}
/// `AF_LATIN_IS_SUB_TOP_BLUE`
fn af_latin_is_sub_top_blue(b: &AfBlueStringRec) -> bool {
    b.properties & AF_BLUE_PROPERTY_LATIN_SUB_TOP != 0
}
/// `AF_LATIN_IS_NEUTRAL_BLUE`
fn af_latin_is_neutral_blue(b: &AfBlueStringRec) -> bool {
    b.properties & AF_BLUE_PROPERTY_LATIN_NEUTRAL != 0
}
/// `AF_LATIN_IS_X_HEIGHT_BLUE`
fn af_latin_is_x_height_blue(b: &AfBlueStringRec) -> bool {
    b.properties & AF_BLUE_PROPERTY_LATIN_X_HEIGHT != 0
}
/// `AF_LATIN_IS_LONG_BLUE`
fn af_latin_is_long_blue(b: &AfBlueStringRec) -> bool {
    b.properties & AF_BLUE_PROPERTY_LATIN_LONG != 0
}

pub const AF_LATIN_MAX_WIDTHS: usize = 16;

pub const AF_LATIN_BLUE_ACTIVE: FtUInt = 1 << 0; /* zone height is <= 3/4px   */
pub const AF_LATIN_BLUE_TOP: FtUInt = 1 << 1; /* we have a top blue zone   */
pub const AF_LATIN_BLUE_SUB_TOP: FtUInt = 1 << 2; /* we have a subscript top   */
/* blue zone                 */
pub const AF_LATIN_BLUE_NEUTRAL: FtUInt = 1 << 3; /* we have neutral blue zone */
pub const AF_LATIN_BLUE_ADJUSTMENT: FtUInt = 1 << 4; /* used for scale adjustment */
/* optimization              */

/// `AF_LatinBlueRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct AfLatinBlueRec {
    pub ref_: AfWidthRec,
    pub shoot: AfWidthRec,
    pub ascender: FtPos,
    pub descender: FtPos,
    pub flags: FtUInt,
}

/// `AF_LatinAxisRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct AfLatinAxisRec {
    pub scale: FtFixed,
    pub delta: FtPos,

    pub width_count: FtUInt,                       /* number of used widths */
    pub widths: [AfWidthRec; AF_LATIN_MAX_WIDTHS], /* widths array          */
    pub edge_distance_threshold: FtPos,            /* used for creating edges */
    pub standard_width: FtPos,                     /* the default stem thickness */
    pub extra_light: bool,                         /* is standard width very light? */

    /* ignored for horizontal metrics */
    pub blue_count: FtUInt,
    pub blues: [AfLatinBlueRec; AF_BLUE_STRINGSET_MAX_LEN],

    pub org_scale: FtFixed,
    pub org_delta: FtPos,
}

/// `AF_LatinMetricsRec` (without its `root`, which is the
/// [`AfStyleMetricsRec`] holding it)
#[derive(Debug, Clone, Copy, Default)]
pub struct AfLatinMetricsRec {
    pub units_per_em: FtUInt,
    pub axis: [AfLatinAxisRec; AF_DIMENSION_MAX],
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****           L A T I N   G L Y P H   A N A L Y S I S             *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

pub const AF_LATIN_HINTS_HORZ_SNAP: FtUInt32 = 1 << 0; /* stem width snapping  */
pub const AF_LATIN_HINTS_VERT_SNAP: FtUInt32 = 1 << 1; /* stem height snapping */
pub const AF_LATIN_HINTS_STEM_ADJUST: FtUInt32 = 1 << 2; /* stem width/height    */
/* adjustment           */
pub const AF_LATIN_HINTS_MONO: FtUInt32 = 1 << 3; /* monochrome rendering */

/// `AF_LATIN_HINTS_DO_HORZ_SNAP`
pub fn af_latin_hints_do_horz_snap(other_flags: FtUInt32) -> bool {
    other_flags & AF_LATIN_HINTS_HORZ_SNAP != 0
}
/// `AF_LATIN_HINTS_DO_VERT_SNAP`
pub fn af_latin_hints_do_vert_snap(other_flags: FtUInt32) -> bool {
    other_flags & AF_LATIN_HINTS_VERT_SNAP != 0
}
/// `AF_LATIN_HINTS_DO_STEM_ADJUST`
pub fn af_latin_hints_do_stem_adjust(other_flags: FtUInt32) -> bool {
    other_flags & AF_LATIN_HINTS_STEM_ADJUST != 0
}
/// `AF_LATIN_HINTS_DO_MONO`
pub fn af_latin_hints_do_mono(other_flags: FtUInt32) -> bool {
    other_flags & AF_LATIN_HINTS_MONO != 0
}

/* aflatin.c */

/* needed for computation of round vs. flat segments */

/// `FLAT_THRESHOLD`
fn flat_threshold(x: FtPos) -> FtPos {
    x / 14
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****            L A T I N   G L O B A L   M E T R I C S            *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/* Find segments and links, compute all stem widths, and initialize */
/* standard width and height for the glyph with given charcode.     */

/// `af_latin_metrics_init_widths`
pub fn af_latin_metrics_init_widths(metrics: &mut AfStyleMetricsRec, face: &mut FtFace) {
    /* scan the array of segments in each direction */
    let mut hints = af_glyph_hints_init();

    metrics.latin_mut().axis[AF_DIMENSION_HORZ].width_count = 0;
    metrics.latin_mut().axis[AF_DIMENSION_VERT].width_count = 0;

    {
        let style_class = metrics.style_class;
        let script_class = &AF_SCRIPT_CLASSES[style_class.script as usize];

        /* If HarfBuzz is not available, we need a pointer to a single */
        /* unsigned long value.                                        */
        let mut shaper_buf: FtULong = 0;

        let p_str = script_class.standard_charstring;
        let mut p: usize = 0;

        /*
         * We check a list of standard characters to catch features like
         * `c2sc' (small caps from caps) that don't contain lowercase letters
         * by definition, or other features that mainly operate on numerals.
         * The first match wins.
         */

        let mut glyph_index: FtULong = 0;
        while p_str[p] != 0 {
            while p_str[p] == b' ' {
                p += 1;
            }

            /* reject input that maps to more than a single glyph */
            let num_idx;
            (p, num_idx) = af_shaper_get_cluster(p_str, p, face, &mut shaper_buf);
            if num_idx > 1 {
                continue;
            }

            /* otherwise exit loop if we have a result */
            glyph_index = af_shaper_get_elem(face, shaper_buf, 0, None, None);
            if glyph_index != 0 {
                break;
            }
        }

        af_shaper_buf_destroy(face, &mut shaper_buf);

        'exit: {
            if glyph_index == 0 {
                break 'exit;
            }

            let error = ft_load_glyph(face, glyph_index as FtUInt, FT_LOAD_NO_SCALE);
            if error.is_err() || face.glyph.outline.n_points <= 0 {
                break 'exit;
            }

            let units_per_em = metrics.latin().units_per_em;
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
                ws: AfWritingSystemMetrics::Latin(Box::new(AfLatinMetricsRec {
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

                let axis = &mut metrics.latin_mut().axis[dim];
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

                            if (num_widths as usize) < AF_LATIN_MAX_WIDTHS {
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
            let axis = &mut metrics.latin_mut().axis[dim];

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

/// `af_latin_sort_blue`
fn af_latin_sort_blue(count: FtUInt, table: &mut [usize], blues: &[AfLatinBlueRec]) {
    /* we sort from bottom to top */
    for i in 1..count as usize {
        let mut j = i;
        while j > 0 {
            let a = if blues[table[j - 1]].flags & (AF_LATIN_BLUE_TOP | AF_LATIN_BLUE_SUB_TOP) != 0
            {
                blues[table[j - 1]].ref_.org
            } else {
                blues[table[j - 1]].shoot.org
            };

            let b = if blues[table[j]].flags & (AF_LATIN_BLUE_TOP | AF_LATIN_BLUE_SUB_TOP) != 0 {
                blues[table[j]].ref_.org
            } else {
                blues[table[j]].shoot.org
            };

            if b >= a {
                break;
            }

            table.swap(j, j - 1);
            j -= 1;
        }
    }
}

/* Find all blue zones.  Flat segments give the reference points, */
/* round segments the overshoot positions.                        */

/// `af_latin_metrics_init_blues`
fn af_latin_metrics_init_blues(
    metrics: &mut AfStyleMetricsRec,
    face: &mut FtFace,
    globals: &mut AfFaceGlobalsRec,
) -> i32 {
    let mut flats: [FtPos; AF_BLUE_STRING_MAX_LEN] = [0; AF_BLUE_STRING_MAX_LEN];
    let mut rounds: [FtPos; AF_BLUE_STRING_MAX_LEN] = [0; AF_BLUE_STRING_MAX_LEN];

    let sc = metrics.style_class;

    let bss = sc.blue_stringset;
    let mut bs = bss as usize;

    let units_per_em = metrics.latin().units_per_em;
    let flat_threshold_ = flat_threshold(units_per_em as FtPos);

    /* If HarfBuzz is not available, we need a pointer to a single */
    /* unsigned long value.                                        */
    let mut shaper_buf: FtULong = 0;

    /* we walk over the blue character strings as specified in the */
    /* style's entry in the `af_blue_stringset' array              */

    while AF_BLUE_STRINGSETS[bs].string != AF_BLUE_STRING_MAX {
        let bsr = AF_BLUE_STRINGSETS[bs];
        let p_str = &AF_BLUE_STRINGS[..];
        let mut p = bsr.string as usize;

        let mut num_flats: FtUInt = 0;
        let mut num_rounds: FtUInt = 0;
        let mut ascender: FtPos = 0;
        let mut descender: FtPos = 0;

        while p_str[p] != 0 {
            let mut y_offset: FtLong = 0;
            let mut best_y_extremum: FtPos; /* same as points.y */
            let mut best_round = false;

            while p_str[p] == b' ' {
                p += 1;
            }

            let num_idx;
            (p, num_idx) = af_shaper_get_cluster(p_str, p, face, &mut shaper_buf);

            if num_idx == 0 {
                continue;
            }

            if af_latin_is_top_blue(&bsr) {
                best_y_extremum = i32::MIN as FtPos;
            } else {
                best_y_extremum = i32::MAX as FtPos;
            }

            /* iterate over all glyph elements of the character cluster */
            /* and get the data of the `biggest' one                    */
            for i in 0..num_idx {
                let mut best_y: FtPos;
                let mut round = false;

                /* load the character in the face -- skip unknown or empty ones */
                let glyph_index =
                    af_shaper_get_elem(face, shaper_buf, i, None, Some(&mut y_offset));
                if glyph_index == 0 {
                    continue;
                }

                let error = ft_load_glyph(face, glyph_index as FtUInt, FT_LOAD_NO_SCALE);
                let outline = &face.glyph.outline;
                /* reject glyphs that don't produce any rendering */
                if error.is_err() || outline.n_points <= 2 {
                    continue;
                }

                /* now compute min or max point indices and coordinates */
                let points = &outline.points;
                let mut best_point: FtInt = -1;
                let mut best_contour_first: FtInt = -1;
                let mut best_contour_last: FtInt = -1;
                best_y = 0; /* make compiler happy */

                {
                    let mut last: FtInt = -1;
                    for nn in 0..outline.n_contours as usize {
                        let first = last + 1;
                        last = outline.contours[nn] as FtInt;

                        /* Avoid single-point contours since they are never      */
                        /* rasterized.  In some fonts, they correspond to mark   */
                        /* attachment points that are way outside of the glyph's */
                        /* real outline.                                         */
                        if last <= first {
                            continue;
                        }

                        if af_latin_is_top_blue(&bsr) || af_latin_is_sub_top_blue(&bsr) {
                            for pp in first..=last {
                                let py = points[pp as usize].y;
                                if best_point < 0 || py > best_y {
                                    best_point = pp;
                                    best_y = py;
                                    ascender = ascender.max(best_y + y_offset);
                                } else {
                                    descender = descender.min(py + y_offset);
                                }
                            }
                        } else {
                            for pp in first..=last {
                                let py = points[pp as usize].y;
                                if best_point < 0 || py < best_y {
                                    best_point = pp;
                                    best_y = py;
                                    descender = descender.min(best_y + y_offset);
                                } else {
                                    ascender = ascender.max(py + y_offset);
                                }
                            }
                        }

                        if best_point > best_contour_last {
                            best_contour_first = first;
                            best_contour_last = last;
                        }
                    }
                }

                /* now check whether the point belongs to a straight or round   */
                /* segment; we first need to find in which contour the extremum */
                /* lies, then inspect its previous and next points              */
                if best_point >= 0 {
                    let pt = |i: FtInt| points[i as usize];
                    let tag = |i: FtInt| ft_curve_tag(outline.tags[i as usize]);
                    let best_x = pt(best_point).x;
                    let mut prev: FtInt;
                    let mut next: FtInt;
                    let mut best_segment_first: FtInt;
                    let mut best_segment_last: FtInt;
                    let mut best_on_point_first: FtInt;
                    let mut best_on_point_last: FtInt;
                    let mut dist: FtPos;

                    best_segment_first = best_point;
                    best_segment_last = best_point;

                    if tag(best_point) == FT_CURVE_TAG_ON {
                        best_on_point_first = best_point;
                        best_on_point_last = best_point;
                    } else {
                        best_on_point_first = -1;
                        best_on_point_last = -1;
                    }

                    /* look for the previous and next points on the contour  */
                    /* that are not on the same Y coordinate, then threshold */
                    /* the `closeness'...                                    */
                    prev = best_point;
                    next = prev;

                    loop {
                        if prev > best_contour_first {
                            prev -= 1;
                        } else {
                            prev = best_contour_last;
                        }

                        dist = ft_abs(pt(prev).y - best_y);
                        /* accept a small distance or a small angle (both values are */
                        /* heuristic; value 20 corresponds to approx. 2.9 degrees)   */
                        if dist > 5 && ft_abs(pt(prev).x - best_x) <= 20 * dist {
                            break;
                        }

                        best_segment_first = prev;

                        if tag(prev) == FT_CURVE_TAG_ON {
                            best_on_point_first = prev;
                            if best_on_point_last < 0 {
                                best_on_point_last = prev;
                            }
                        }

                        if prev == best_point {
                            break;
                        }
                    }

                    loop {
                        if next < best_contour_last {
                            next += 1;
                        } else {
                            next = best_contour_first;
                        }

                        dist = ft_abs(pt(next).y - best_y);
                        if dist > 5 && ft_abs(pt(next).x - best_x) <= 20 * dist {
                            break;
                        }

                        best_segment_last = next;

                        if tag(next) == FT_CURVE_TAG_ON {
                            best_on_point_last = next;
                            if best_on_point_first < 0 {
                                best_on_point_first = next;
                            }
                        }

                        if next == best_point {
                            break;
                        }
                    }

                    if af_latin_is_long_blue(&bsr) {
                        /* If this flag is set, we have an additional constraint to  */
                        /* get the blue zone distance: Find a segment of the topmost */
                        /* (or bottommost) contour that is longer than a heuristic   */
                        /* threshold.  This ensures that small bumps in the outline  */
                        /* are ignored (for example, the `vertical serifs' found in  */
                        /* many Hebrew glyph designs).                               */

                        /* If this segment is long enough, we are done.  Otherwise,  */
                        /* search the segment next to the extremum that is long      */
                        /* enough, has the same direction, and a not too large       */
                        /* vertical distance from the extremum.  Note that the       */
                        /* algorithm doesn't check whether the found segment is      */
                        /* actually the one (vertically) nearest to the extremum.    */

                        /* heuristic threshold value */
                        let length_threshold = units_per_em as FtPos / 25;

                        dist = ft_abs(pt(best_segment_last).x - pt(best_segment_first).x);

                        if dist < length_threshold
                            && best_segment_last - best_segment_first + 2
                                <= best_contour_last - best_contour_first
                        {
                            /* heuristic threshold value */
                            let height_threshold = units_per_em as FtPos / 4;

                            let mut first: FtInt;
                            let mut last: FtInt;
                            let mut hit: bool;

                            /* we intentionally declare these two variables        */
                            /* outside of the loop since various compilers emit    */
                            /* incorrect warning messages otherwise, talking about */
                            /* `possibly uninitialized variables'                  */
                            let mut p_first: FtInt = 0; /* make compiler happy */
                            let mut p_last: FtInt = 0;

                            /* compute direction */
                            prev = best_point;

                            loop {
                                if prev > best_contour_first {
                                    prev -= 1;
                                } else {
                                    prev = best_contour_last;
                                }

                                if pt(prev).x != best_x {
                                    break;
                                }

                                if prev == best_point {
                                    break;
                                }
                            }

                            /* skip glyph for the degenerate case */
                            if prev == best_point {
                                continue;
                            }

                            let left2right = pt(prev).x < pt(best_point).x;

                            first = best_segment_last;
                            last = first;
                            hit = false;

                            'search: loop {
                                'cont: {
                                    if !hit {
                                        /* no hit; adjust first point */
                                        first = last;

                                        /* also adjust first and last on point */
                                        if tag(first) == FT_CURVE_TAG_ON {
                                            p_first = first;
                                            p_last = first;
                                        } else {
                                            p_first = -1;
                                            p_last = -1;
                                        }

                                        hit = true;
                                    }

                                    if last < best_contour_last {
                                        last += 1;
                                    } else {
                                        last = best_contour_first;
                                    }

                                    if ft_abs(best_y - pt(first).y) > height_threshold {
                                        /* vertical distance too large */
                                        hit = false;
                                        break 'cont;
                                    }

                                    /* same test as above */
                                    dist = ft_abs(pt(last).y - pt(first).y);
                                    if dist > 5 && ft_abs(pt(last).x - pt(first).x) <= 20 * dist {
                                        hit = false;
                                        break 'cont;
                                    }

                                    if tag(last) == FT_CURVE_TAG_ON {
                                        p_last = last;
                                        if p_first < 0 {
                                            p_first = last;
                                        }
                                    }

                                    let l2r = pt(first).x < pt(last).x;
                                    let mut d = ft_abs(pt(last).x - pt(first).x);

                                    if l2r == left2right && d >= length_threshold {
                                        /* all constraints are met; update segment after */
                                        /* finding its end                               */
                                        loop {
                                            if last < best_contour_last {
                                                last += 1;
                                            } else {
                                                last = best_contour_first;
                                            }

                                            d = ft_abs(pt(last).y - pt(first).y);
                                            /* (upstream compares the `next' point and `dist' */
                                            /* here, not `last' and `d')                      */
                                            if d > 5
                                                && ft_abs(pt(next).x - pt(first).x) <= 20 * dist
                                            {
                                                if last > best_contour_first {
                                                    last -= 1;
                                                } else {
                                                    last = best_contour_last;
                                                }
                                                break;
                                            }

                                            p_last = last;

                                            if tag(last) == FT_CURVE_TAG_ON {
                                                p_last = last;
                                                if p_first < 0 {
                                                    p_first = last;
                                                }
                                            }

                                            if last == best_segment_first {
                                                break;
                                            }
                                        }

                                        best_y = pt(first).y;

                                        best_segment_first = first;
                                        best_segment_last = last;

                                        best_on_point_first = p_first;
                                        best_on_point_last = p_last;

                                        break 'search;
                                    }
                                }

                                if last == best_segment_first {
                                    break;
                                }
                            }
                        }
                    }

                    /* for computing blue zones, we add the y offset as returned */
                    /* by the currently used OpenType feature -- for example,    */
                    /* superscript glyphs might be identical to subscript glyphs */
                    /* with a vertical shift                                     */
                    best_y += y_offset;

                    /* now set the `round' flag depending on the segment's kind: */
                    /*                                                           */
                    /* - if the horizontal distance between the first and last   */
                    /*   `on' point is larger than a heuristic threshold         */
                    /*   we have a flat segment                                  */
                    /* - if either the first or the last point of the segment is */
                    /*   an `off' point, the segment is round, otherwise it is   */
                    /*   flat                                                    */
                    if best_on_point_first >= 0
                        && best_on_point_last >= 0
                        && ft_abs(pt(best_on_point_last).x - pt(best_on_point_first).x)
                            > flat_threshold_
                    {
                        round = false;
                    } else {
                        round = tag(best_segment_first) != FT_CURVE_TAG_ON
                            || tag(best_segment_last) != FT_CURVE_TAG_ON;
                    }

                    if round && af_latin_is_neutral_blue(&bsr) {
                        /* only use flat segments for a neutral blue zone */
                        continue;
                    }
                }

                if af_latin_is_top_blue(&bsr) {
                    if best_y > best_y_extremum {
                        best_y_extremum = best_y;
                        best_round = round;
                    }
                } else if best_y < best_y_extremum {
                    best_y_extremum = best_y;
                    best_round = round;
                }
            } /* end for loop */

            if !(best_y_extremum == i32::MIN as FtPos || best_y_extremum == i32::MAX as FtPos) {
                if best_round {
                    rounds[num_rounds as usize] = best_y_extremum;
                    num_rounds += 1;
                } else {
                    flats[num_flats as usize] = best_y_extremum;
                    num_flats += 1;
                }
            }
        } /* end while loop */

        if num_flats == 0 && num_rounds == 0 {
            /*
             * we couldn't find a single glyph to compute this blue zone,
             * we will simply ignore it then
             */
            bs += 1;
            continue;
        }

        /* we have computed the contents of the `rounds' and `flats' tables, */
        /* now determine the reference and overshoot position of the blue -- */
        /* we simply take the median value after a simple sort               */
        af_sort_pos(num_rounds, &mut rounds);
        af_sort_pos(num_flats, &mut flats);

        let axis = &mut metrics.latin_mut().axis[AF_DIMENSION_VERT];
        let blue = &mut axis.blues[axis.blue_count as usize];
        axis.blue_count += 1;

        if num_flats == 0 {
            blue.ref_.org = rounds[num_rounds as usize / 2];
            blue.shoot.org = blue.ref_.org;
        } else if num_rounds == 0 {
            blue.ref_.org = flats[num_flats as usize / 2];
            blue.shoot.org = blue.ref_.org;
        } else {
            blue.ref_.org = flats[num_flats as usize / 2];
            blue.shoot.org = rounds[num_rounds as usize / 2];
        }

        /* there are sometimes problems: if the overshoot position of top     */
        /* zones is under its reference position, or the opposite for bottom  */
        /* zones.  We must thus check everything there and correct the errors */
        if blue.shoot.org != blue.ref_.org {
            let ref_ = blue.ref_.org;
            let shoot = blue.shoot.org;
            let over_ref = shoot > ref_;

            if (af_latin_is_top_blue(&bsr) || af_latin_is_sub_top_blue(&bsr)) ^ over_ref {
                blue.ref_.org = (shoot + ref_) / 2;
                blue.shoot.org = blue.ref_.org;
            }
        }

        blue.ascender = ascender;
        blue.descender = descender;

        blue.flags = 0;
        if af_latin_is_top_blue(&bsr) {
            blue.flags |= AF_LATIN_BLUE_TOP;
        }
        if af_latin_is_sub_top_blue(&bsr) {
            blue.flags |= AF_LATIN_BLUE_SUB_TOP;
        }
        if af_latin_is_neutral_blue(&bsr) {
            blue.flags |= AF_LATIN_BLUE_NEUTRAL;
        }

        /*
         * The following flag is used later to adjust the y and x scales
         * in order to optimize the pixel grid alignment of the top of small
         * letters.
         */
        if af_latin_is_x_height_blue(&bsr) {
            blue.flags |= AF_LATIN_BLUE_ADJUSTMENT;
        }

        bs += 1;
    } /* end for loop */

    af_shaper_buf_destroy(face, &mut shaper_buf);

    let axis = &mut metrics.latin_mut().axis[AF_DIMENSION_VERT];
    if axis.blue_count != 0 {
        /* we finally check whether blue zones are ordered;            */
        /* `ref' and `shoot' values of two blue zones must not overlap */

        let mut blue_sorted: [usize; AF_BLUE_STRINGSET_MAX_LEN] = [0; AF_BLUE_STRINGSET_MAX_LEN];

        for i in 0..axis.blue_count as usize {
            blue_sorted[i] = i;
        }

        /* sort bottoms of blue zones... */
        af_latin_sort_blue(axis.blue_count, &mut blue_sorted, &axis.blues);

        /* ...and adjust top values if necessary */
        for i in 0..axis.blue_count as usize - 1 {
            let bi = blue_sorted[i];
            let bj = blue_sorted[i + 1];

            let a_is_top = axis.blues[bi].flags & (AF_LATIN_BLUE_TOP | AF_LATIN_BLUE_SUB_TOP) != 0;
            let b_is_top = axis.blues[bj].flags & (AF_LATIN_BLUE_TOP | AF_LATIN_BLUE_SUB_TOP) != 0;

            let b = if b_is_top {
                axis.blues[bj].shoot.org
            } else {
                axis.blues[bj].ref_.org
            };
            let a = if a_is_top {
                &mut axis.blues[bi].shoot.org
            } else {
                &mut axis.blues[bi].ref_.org
            };

            if *a > b {
                *a = b;
            }
        }

        0
    } else {
        /* disable hinting for the current style if there are no blue zones */

        for i in 0..globals.glyph_count as usize {
            if (globals.glyph_styles[i] & AF_STYLE_MASK) as AfStyle == sc.style {
                globals.glyph_styles[i] = super::afstyles::AF_STYLE_NONE_DFLT as FtUShort;
            }
        }

        1
    }
}

/* Check whether all ASCII digits have the same advance width. */

/// `af_latin_metrics_check_digits`
pub fn af_latin_metrics_check_digits(metrics: &mut AfStyleMetricsRec, face: &mut FtFace) {
    let mut started = false;
    let mut same_width = true;
    let mut advance: FtLong = 0;
    let mut old_advance: FtLong = 0;

    /* If HarfBuzz is not available, we need a pointer to a single */
    /* unsigned long value.                                        */
    let mut shaper_buf: FtULong = 0;

    /* in all supported charmaps, digits have character codes 0x30-0x39 */
    let digits: &[u8] = b"0 1 2 3 4 5 6 7 8 9\0";
    let mut p: usize = 0;

    while digits[p] != 0 {
        /* reject input that maps to more than a single glyph */
        let num_idx;
        (p, num_idx) = af_shaper_get_cluster(digits, p, face, &mut shaper_buf);
        if num_idx > 1 {
            continue;
        }

        let glyph_index = af_shaper_get_elem(face, shaper_buf, 0, Some(&mut advance), None);
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

    af_shaper_buf_destroy(face, &mut shaper_buf);

    metrics.digits_have_same_width = same_width;
}

/* Initialize global metrics. */

/// `af_latin_metrics_init`
pub fn af_latin_metrics_init(
    metrics: &mut AfStyleMetricsRec, /* AF_LatinMetrics */
    face: &mut FtFace,
    globals: &mut AfFaceGlobalsRec,
) -> FtResult<()> {
    let mut error: FtResult<()> = Ok(());

    let oldmap = face.charmap;

    metrics.latin_mut().units_per_em = face.units_per_EM as FtUInt;

    if ft_select_charmap(face, FT_ENCODING_UNICODE).is_ok() {
        af_latin_metrics_init_widths(metrics, face);
        if af_latin_metrics_init_blues(metrics, face, globals) != 0 {
            /* use internal error code to indicate missing blue zones */
            error = Err(-1);
        } else {
            af_latin_metrics_check_digits(metrics, face);
        }
    }

    /* Exit: */
    face.charmap = oldmap;
    error
}

/* Adjust scaling value, then scale and shift widths   */
/* and blue zones (if applicable) for given dimension. */

/// `af_latin_metrics_scale_dim`
fn af_latin_metrics_scale_dim(
    metrics: &mut AfStyleMetricsRec,
    scaler: &AfScalerRec,
    dim: AfDimension,
    globals: &AfFaceGlobalsRec,
) {
    let mut scale: FtFixed;
    let delta: FtPos;

    if dim == AF_DIMENSION_HORZ {
        scale = scaler.x_scale;
        delta = scaler.x_delta;
    } else {
        scale = scaler.y_scale;
        delta = scaler.y_delta;
    }

    let ppem = metrics.scaler.face.x_ppem as FtUInt;
    let latin = metrics.latin_mut();
    let units_per_em = latin.units_per_em;

    {
        let axis = &mut latin.axis[dim];

        if axis.org_scale == scale && axis.org_delta == delta {
            return;
        }

        axis.org_scale = scale;
        axis.org_delta = delta;
    }

    /*
     * correct X and Y scale to optimize the alignment of the top of small
     * letters to the pixel grid
     */
    {
        let axis_v = &latin.axis[AF_DIMENSION_VERT];
        let mut blue: Option<&AfLatinBlueRec> = None;

        for nn in 0..axis_v.blue_count as usize {
            if axis_v.blues[nn].flags & AF_LATIN_BLUE_ADJUSTMENT != 0 {
                blue = Some(&axis_v.blues[nn]);
                break;
            }
        }

        if let Some(blue) = blue {
            let scaled = ft_mul_fix(blue.shoot.org, scale);
            let limit = globals.increase_x_height;
            let mut threshold: FtPos = 40;

            /* if the `increase-x-height' property is active, */
            /* we round up much more often                    */
            if limit != 0 && ppem <= limit && ppem >= AF_PROP_INCREASE_X_HEIGHT_MIN {
                threshold = 52;
            }

            let fitted = (scaled + threshold) & !63;

            if scaled != fitted && dim == AF_DIMENSION_VERT {
                let new_scale = ft_mul_div(scale, fitted, scaled);

                /* the scaling should not change the result by more than two pixels */
                let mut max_height = units_per_em as FtPos;

                for nn in 0..axis_v.blue_count as usize {
                    max_height = max_height.max(axis_v.blues[nn].ascender);
                    max_height = max_height.max(-axis_v.blues[nn].descender);
                }

                let mut dist = ft_abs(ft_mul_fix(max_height, new_scale - scale));
                dist &= !127;

                if dist == 0 {
                    scale = new_scale;
                }
            }
        }
    }

    let axis = &mut latin.axis[dim];

    axis.scale = scale;
    axis.delta = delta;

    if dim == AF_DIMENSION_HORZ {
        metrics.scaler.x_scale = scale;
        metrics.scaler.x_delta = delta;
    } else {
        metrics.scaler.y_scale = scale;
        metrics.scaler.y_delta = delta;
    }

    let axis = &mut metrics.latin_mut().axis[dim];

    /* scale the widths */
    for nn in 0..axis.width_count as usize {
        let width = &mut axis.widths[nn];

        width.cur = ft_mul_fix(width.org, scale);
        width.fit = width.cur;
    }

    /* an extra-light axis corresponds to a standard width that is */
    /* smaller than 5/8 pixels                                     */
    axis.extra_light = ft_mul_fix(axis.standard_width, scale) < 32 + 8;

    if dim == AF_DIMENSION_VERT {
        /* scale the blue zones */
        for nn in 0..axis.blue_count as usize {
            let blue = &mut axis.blues[nn];

            blue.ref_.cur = ft_mul_fix(blue.ref_.org, scale) + delta;
            blue.ref_.fit = blue.ref_.cur;
            blue.shoot.cur = ft_mul_fix(blue.shoot.org, scale) + delta;
            blue.shoot.fit = blue.shoot.cur;
            blue.flags &= !AF_LATIN_BLUE_ACTIVE;

            /* a blue zone is only active if it is less than 3/4 pixels tall */
            let dist = ft_mul_fix(blue.ref_.org - blue.shoot.org, scale);
            if (-48..=48).contains(&dist) {
                /* use discrete values for blue zone widths */

                /* simplified version due to abs(dist) <= 48 */
                let mut delta2 = dist;
                if dist < 0 {
                    delta2 = -delta2;
                }

                if delta2 < 32 {
                    delta2 = 0;
                } else if delta2 < 48 {
                    delta2 = 32;
                } else {
                    delta2 = 64;
                }

                if dist < 0 {
                    delta2 = -delta2;
                }

                blue.ref_.fit = ft_pix_round(blue.ref_.cur);
                blue.shoot.fit = blue.ref_.fit - delta2;

                blue.flags |= AF_LATIN_BLUE_ACTIVE;
            }
        }

        /* use sub-top blue zone only if it doesn't overlap with */
        /* another (non-sup-top) blue zone; otherwise, the       */
        /* effect would be similar to a neutral blue zone, which */
        /* is not desired here                                   */
        for nn in 0..axis.blue_count as usize {
            if axis.blues[nn].flags & AF_LATIN_BLUE_SUB_TOP == 0 {
                continue;
            }
            if axis.blues[nn].flags & AF_LATIN_BLUE_ACTIVE == 0 {
                continue;
            }

            for i in 0..axis.blue_count as usize {
                let b = axis.blues[i];

                if b.flags & AF_LATIN_BLUE_SUB_TOP != 0 {
                    continue;
                }
                if b.flags & AF_LATIN_BLUE_ACTIVE == 0 {
                    continue;
                }

                if b.ref_.fit <= axis.blues[nn].shoot.fit && b.shoot.fit >= axis.blues[nn].ref_.fit
                {
                    axis.blues[nn].flags &= !AF_LATIN_BLUE_ACTIVE;
                    break;
                }
            }
        }
    }
}

/* Scale global values in both directions. */

/// `af_latin_metrics_scale`
pub fn af_latin_metrics_scale(
    metrics: &mut AfStyleMetricsRec, /* AF_LatinMetrics */
    scaler: &AfScalerRec,
    globals: &AfFaceGlobalsRec,
) {
    metrics.scaler.render_mode = scaler.render_mode;
    metrics.scaler.face = scaler.face;
    metrics.scaler.flags = scaler.flags;

    af_latin_metrics_scale_dim(metrics, scaler, AF_DIMENSION_HORZ, globals);
    af_latin_metrics_scale_dim(metrics, scaler, AF_DIMENSION_VERT, globals);
}

/* Extract standard_width from writing system/script specific */
/* metrics class.                                             */

/// `af_latin_get_standard_widths`
fn af_latin_get_standard_widths(
    metrics: &AfStyleMetricsRec, /* AF_LatinMetrics */
) -> (FtPos, FtPos) {
    let latin = metrics.latin();
    (
        latin.axis[AF_DIMENSION_VERT].standard_width,
        latin.axis[AF_DIMENSION_HORZ].standard_width,
    )
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****           L A T I N   G L Y P H   A N A L Y S I S             *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/* Walk over all contours and compute its segments. */

/// `af_latin_hints_compute_segments`
pub fn af_latin_hints_compute_segments(
    hints: &mut AfGlyphHintsRec,
    metrics: &AfStyleMetricsRec,
    dim: AfDimension,
) -> FtResult<()> {
    let flat_threshold_ = flat_threshold(metrics.units_per_em() as FtPos);

    let seg0 = AfSegmentRec {
        score: 32000,
        flags: AF_EDGE_NORMAL,
        ..Default::default()
    };

    let num_points = hints.num_points as usize;
    let points = &mut hints.points;
    let axis = &mut hints.axis[dim];

    let major_dir: AfDirection = axis.major_dir.abs();
    let mut segment_dir: AfDirection = major_dir;

    axis.num_segments = 0;

    /* set up (u,v) in each point */
    if dim == AF_DIMENSION_HORZ {
        for point in &mut points[..num_points] {
            point.u = point.fx as FtPos;
            point.v = point.fy as FtPos;
        }
    } else {
        for point in &mut points[..num_points] {
            point.u = point.fy as FtPos;
            point.v = point.fx as FtPos;
        }
    }

    /* do each contour separately */
    for c in 0..hints.num_contours as usize {
        let mut point = hints.contours[c];
        let mut last = points[point].prev;

        let mut on_edge = false;

        /* we call values measured along a segment (point->v)    */
        /* `coordinates', and values orthogonal to it (point->u) */
        /* `positions'                                           */
        let mut min_pos: FtPos = 32000;
        let mut max_pos: FtPos = -32000;
        let mut min_coord: FtPos = 32000;
        let mut max_coord: FtPos = -32000;
        let mut min_flags: FtUShort = AF_FLAG_NONE;
        let mut max_flags: FtUShort = AF_FLAG_NONE;
        let mut min_on_coord: FtPos = 32000;
        let mut max_on_coord: FtPos = -32000;

        let mut passed: bool;

        let mut segment: Option<usize> = None;
        let mut prev_segment: Option<usize> = None;

        let mut prev_min_pos = min_pos;
        let mut prev_max_pos = max_pos;
        let mut prev_min_coord = min_coord;
        let mut prev_max_coord = max_coord;
        let mut prev_min_flags = min_flags;
        let mut prev_max_flags = max_flags;
        let mut prev_min_on_coord = min_on_coord;
        let mut prev_max_on_coord = max_on_coord;

        let abs_dir = |d: i8| (d as AfDirection).abs();

        if abs_dir(points[last].out_dir) == major_dir && abs_dir(points[point].out_dir) == major_dir
        {
            /* we are already on an edge, try to locate its start */
            last = point;

            loop {
                point = points[point].prev;
                if abs_dir(points[point].out_dir) != major_dir {
                    point = points[point].next;
                    break;
                }
                if point == last {
                    break;
                }
            }
        }

        last = point;
        passed = false;

        loop {
            if on_edge {
                let seg = segment.expect("segment while on an edge");

                /* get minimum and maximum position */
                let u = points[point].u;
                if u < min_pos {
                    min_pos = u;
                }
                if u > max_pos {
                    max_pos = u;
                }

                /* get minimum and maximum coordinate together with flags */
                let v = points[point].v;
                if v < min_coord {
                    min_coord = v;
                    min_flags = points[point].flags;
                }
                if v > max_coord {
                    max_coord = v;
                    max_flags = points[point].flags;
                }

                /* get minimum and maximum coordinate of `on' points */
                if points[point].flags & AF_FLAG_CONTROL == 0 {
                    let v = points[point].v;
                    if v < min_on_coord {
                        min_on_coord = v;
                    }
                    if v > max_on_coord {
                        max_on_coord = v;
                    }
                }

                if points[point].out_dir as AfDirection != segment_dir || point == last {
                    let segments = &mut axis.segments;

                    /* check whether the new segment's start point is identical to */
                    /* the previous segment's end point; for example, this might   */
                    /* happen for spikes                                           */

                    if prev_segment.is_none()
                        || segments[seg].first != segments[prev_segment.unwrap()].last
                    {
                        /* points are different: we are just leaving an edge, thus */
                        /* record a new segment                                    */

                        let s = &mut segments[seg];
                        s.last = point;
                        s.pos = ((min_pos + max_pos) >> 1) as FtShort;
                        s.delta = ((max_pos - min_pos) >> 1) as FtShort;

                        /* a segment is round if either its first or last point */
                        /* is a control point, and the length of the on points  */
                        /* inbetween doesn't exceed a heuristic limit           */
                        if (min_flags | max_flags) & AF_FLAG_CONTROL != 0
                            && (max_on_coord - min_on_coord) < flat_threshold_
                        {
                            s.flags |= AF_EDGE_ROUND;
                        }

                        s.min_coord = min_coord as FtShort;
                        s.max_coord = max_coord as FtShort;
                        s.height = s.max_coord.wrapping_sub(s.min_coord);

                        prev_segment = Some(seg);
                        prev_min_pos = min_pos;
                        prev_max_pos = max_pos;
                        prev_min_coord = min_coord;
                        prev_max_coord = max_coord;
                        prev_min_flags = min_flags;
                        prev_max_flags = max_flags;
                        prev_min_on_coord = min_on_coord;
                        prev_max_on_coord = max_on_coord;
                    } else {
                        let ps = prev_segment.unwrap();

                        /* points are the same: we don't create a new segment but */
                        /* merge the current segment with the previous one        */

                        if points[segments[ps].last].in_dir == points[point].in_dir {
                            /* we have identical directions (this can happen for       */
                            /* degenerate outlines that move zig-zag along the main    */
                            /* axis without changing the coordinate value of the other */
                            /* axis, and where the segments have just been merged):    */
                            /* unify segments                                          */

                            /* update constraints */

                            if prev_min_pos < min_pos {
                                min_pos = prev_min_pos;
                            }
                            if prev_max_pos > max_pos {
                                max_pos = prev_max_pos;
                            }

                            if prev_min_coord < min_coord {
                                min_coord = prev_min_coord;
                                min_flags = prev_min_flags;
                            }
                            if prev_max_coord > max_coord {
                                max_coord = prev_max_coord;
                                max_flags = prev_max_flags;
                            }

                            if prev_min_on_coord < min_on_coord {
                                min_on_coord = prev_min_on_coord;
                            }
                            if prev_max_on_coord > max_on_coord {
                                max_on_coord = prev_max_on_coord;
                            }

                            let p = &mut segments[ps];
                            p.last = point;
                            p.pos = ((min_pos + max_pos) >> 1) as FtShort;
                            p.delta = ((max_pos - min_pos) >> 1) as FtShort;

                            if (min_flags | max_flags) & AF_FLAG_CONTROL != 0
                                && (max_on_coord - min_on_coord) < flat_threshold_
                            {
                                p.flags |= AF_EDGE_ROUND;
                            } else {
                                p.flags &= !AF_EDGE_ROUND;
                            }

                            p.min_coord = min_coord as FtShort;
                            p.max_coord = max_coord as FtShort;
                            p.height = p.max_coord.wrapping_sub(p.min_coord);
                        } else {
                            /* we have different directions; use the properties of the */
                            /* longer segment and discard the other one                */

                            if ft_abs(prev_max_coord - prev_min_coord)
                                > ft_abs(max_coord - min_coord)
                            {
                                /* discard current segment */

                                if min_pos < prev_min_pos {
                                    prev_min_pos = min_pos;
                                }
                                if max_pos > prev_max_pos {
                                    prev_max_pos = max_pos;
                                }

                                let p = &mut segments[ps];
                                p.last = point;
                                p.pos = ((prev_min_pos + prev_max_pos) >> 1) as FtShort;
                                p.delta = ((prev_max_pos - prev_min_pos) >> 1) as FtShort;
                            } else {
                                /* discard previous segment */

                                if prev_min_pos < min_pos {
                                    min_pos = prev_min_pos;
                                }
                                if prev_max_pos > max_pos {
                                    max_pos = prev_max_pos;
                                }

                                let s = &mut segments[seg];
                                s.last = point;
                                s.pos = ((min_pos + max_pos) >> 1) as FtShort;
                                s.delta = ((max_pos - min_pos) >> 1) as FtShort;

                                if (min_flags | max_flags) & AF_FLAG_CONTROL != 0
                                    && (max_on_coord - min_on_coord) < flat_threshold_
                                {
                                    s.flags |= AF_EDGE_ROUND;
                                }

                                s.min_coord = min_coord as FtShort;
                                s.max_coord = max_coord as FtShort;
                                s.height = s.max_coord.wrapping_sub(s.min_coord);

                                segments[ps] = segments[seg];

                                prev_min_pos = min_pos;
                                prev_max_pos = max_pos;
                                prev_min_coord = min_coord;
                                prev_max_coord = max_coord;
                                prev_min_flags = min_flags;
                                prev_max_flags = max_flags;
                                prev_min_on_coord = min_on_coord;
                                prev_max_on_coord = max_on_coord;
                            }
                        }

                        axis.num_segments -= 1;
                    }

                    on_edge = false;
                    segment = None;

                    /* fall through */
                }
            }

            /* now exit if we are at the start/end point */
            if point == last {
                if passed {
                    break;
                }
                passed = true;
            }

            /* if we are not on an edge, check whether the major direction */
            /* coincides with the current point's `out' direction, or      */
            /* whether we have a single-point contour                      */
            if !on_edge
                && (abs_dir(points[point].out_dir) == major_dir || point == points[point].prev)
            {
                /*
                 * For efficiency, we restrict the number of segments to 1000,
                 * which is a heuristic value: it is very unlikely that a glyph
                 * with so many segments can be hinted in a sensible way.
                 * Reasons:
                 *
                 * - The glyph has really 1000 segments; this implies that it has
                 *   at least 2000 outline points.  Assuming 'normal' fonts that
                 *   have superfluous points optimized away, viewing such a glyph
                 *   only makes sense at large magnifications where hinting
                 *   isn't applied anyway.
                 *
                 * - We have a broken glyph.  Hinting doesn't make sense in this
                 *   case either.
                 */
                if axis.num_segments > 1000 {
                    axis.num_segments = 0;
                    return Ok(());
                }

                /* this is the start of a new segment! */
                segment_dir = points[point].out_dir as AfDirection;

                let seg = af_axis_hints_new_segment(axis)?;
                segment = Some(seg);

                /* clear all segment fields */
                axis.segments[seg] = seg0;

                axis.segments[seg].dir = segment_dir as i8;
                axis.segments[seg].first = point;
                axis.segments[seg].last = point;

                /* `af_axis_hints_new_segment' reallocates memory,    */
                /* thus we have to refresh the `prev_segment' pointer */
                if prev_segment.is_some() {
                    prev_segment = Some(seg.wrapping_sub(1));
                }

                min_pos = points[point].u;
                max_pos = min_pos;
                min_coord = points[point].v;
                max_coord = min_coord;
                min_flags = points[point].flags;
                max_flags = min_flags;

                if points[point].flags & AF_FLAG_CONTROL != 0 {
                    min_on_coord = 32000;
                    max_on_coord = -32000;
                } else {
                    min_on_coord = points[point].v;
                    max_on_coord = min_on_coord;
                }

                on_edge = true;

                if point == points[point].prev {
                    /* we have a one-point segment: this is a one-point */
                    /* contour with `in' and `out' direction set to     */
                    /* AF_DIR_NONE                                      */
                    let s = &mut axis.segments[seg];
                    s.pos = min_pos as FtShort;

                    if points[point].flags & AF_FLAG_CONTROL != 0 {
                        s.flags |= AF_EDGE_ROUND;
                    }

                    s.min_coord = points[point].v as FtShort;
                    s.max_coord = points[point].v as FtShort;
                    s.height = 0;

                    on_edge = false;
                    segment = None;
                }
            }

            point = points[point].next;
        }
    } /* contours */

    /* now slightly increase the height of segments if this makes */
    /* sense -- this is used to better detect and ignore serifs   */
    {
        for s in 0..axis.num_segments as usize {
            let segment = &mut axis.segments[s];
            let first = segment.first;
            let last = segment.last;
            let first_v = points[first].v;
            let last_v = points[last].v;

            if first_v < last_v {
                let p = points[first].prev;
                if points[p].v < first_v {
                    segment.height =
                        (segment.height as FtPos + ((first_v - points[p].v) >> 1)) as FtShort;
                }

                let p = points[last].next;
                if points[p].v > last_v {
                    segment.height =
                        (segment.height as FtPos + ((points[p].v - last_v) >> 1)) as FtShort;
                }
            } else {
                let p = points[first].prev;
                if points[p].v > first_v {
                    segment.height =
                        (segment.height as FtPos + ((points[p].v - first_v) >> 1)) as FtShort;
                }

                let p = points[last].next;
                if points[p].v < last_v {
                    segment.height =
                        (segment.height as FtPos + ((last_v - points[p].v) >> 1)) as FtShort;
                }
            }
        }
    }

    Ok(())
}

/* Link segments to form stems and serifs.  If `width_count' and      */
/* `widths' are non-zero, use them to fine-tune the scoring function. */

/// `af_latin_hints_link_segments`
pub fn af_latin_hints_link_segments(
    hints: &mut AfGlyphHintsRec,
    metrics: &AfStyleMetricsRec,
    width_count: FtUInt,
    widths: &[AfWidthRec],
    dim: AfDimension,
) {
    let axis = &mut hints.axis[dim];
    let major_dir = axis.major_dir;
    let segment_limit = axis.num_segments as usize;
    let segments = &mut axis.segments;

    let max_width: FtPos = if width_count != 0 {
        widths[width_count as usize - 1].org
    } else {
        0
    };

    /* a heuristic value to set up a minimum value for overlapping */
    let mut len_threshold = af_latin_constant(metrics, 8);
    if len_threshold == 0 {
        len_threshold = 1;
    }

    /* a heuristic value to weight lengths */
    let len_score = af_latin_constant(metrics, 6000);

    /* a heuristic value to weight distances (no call to    */
    /* AF_LATIN_CONSTANT needed, since we work on multiples */
    /* of the stem width)                                   */
    let dist_score: FtPos = 3000;

    /* now compare each segment to the others */
    for seg1 in 0..segment_limit {
        if segments[seg1].dir as AfDirection != major_dir {
            continue;
        }

        /* search for stems having opposite directions, */
        /* with seg1 to the `left' of seg2              */
        for seg2 in 0..segment_limit {
            let pos1 = segments[seg1].pos as FtPos;
            let pos2 = segments[seg2].pos as FtPos;

            if segments[seg1].dir as i32 + segments[seg2].dir as i32 == 0 && pos2 > pos1 {
                /* compute distance between the two segments */
                let mut min = segments[seg1].min_coord as FtPos;
                let mut max = segments[seg1].max_coord as FtPos;

                if min < segments[seg2].min_coord as FtPos {
                    min = segments[seg2].min_coord as FtPos;
                }

                if max > segments[seg2].max_coord as FtPos {
                    max = segments[seg2].max_coord as FtPos;
                }

                /* compute maximum coordinate difference of the two segments */
                /* (that is, how much they overlap)                          */
                let len = max - min;
                if len >= len_threshold {
                    /*
                     * The score is the sum of two demerits indicating the
                     * `badness' of a fit, measured along the segments' main axis
                     * and orthogonal to it, respectively.
                     *
                     * - The less overlapping along the main axis, the worse it
                     *   is, causing a larger demerit.
                     *
                     * - The nearer the orthogonal distance to a stem width, the
                     *   better it is, causing a smaller demerit.  For simplicity,
                     *   however, we only increase the demerit for values that
                     *   exceed the largest stem width.
                     */

                    let dist = pos2 - pos1;

                    let dist_demerit: FtPos = if max_width != 0 {
                        /* distance demerits are based on multiples of `max_width'; */
                        /* we scale by 1024 for getting more precision              */
                        let delta = (dist << 10) / max_width - (1 << 10);

                        if delta > 10000 {
                            32000
                        } else if delta > 0 {
                            delta * delta / dist_score
                        } else {
                            0
                        }
                    } else {
                        dist /* default if no widths available */
                    };

                    let score = dist_demerit + len_score / len;

                    /* and we search for the smallest score */
                    if score < segments[seg1].score {
                        segments[seg1].score = score;
                        segments[seg1].link = Some(seg2);
                    }

                    if score < segments[seg2].score {
                        segments[seg2].score = score;
                        segments[seg2].link = Some(seg1);
                    }
                }
            }
        }
    }

    /* now compute the `serif' segments, cf. explanations in `afhints.h' */
    for seg1 in 0..segment_limit {
        let seg2 = segments[seg1].link;

        if let Some(seg2) = seg2 {
            if segments[seg2].link != Some(seg1) {
                segments[seg1].link = None;
                segments[seg1].serif = segments[seg2].link;
            }
        }
    }
}

/* Link segments to edges, using feature analysis for selection. */

/// `af_latin_hints_compute_edges`
pub fn af_latin_hints_compute_edges(
    hints: &mut AfGlyphHintsRec,
    metrics: &AfStyleMetricsRec,
    dim: AfDimension,
) -> FtResult<()> {
    let laxis = &metrics.latin().axis[dim];

    let style_class = metrics.style_class;
    let script_class = &AF_SCRIPT_CLASSES[style_class.script as usize];

    let mut top_to_bottom_hinting = false;

    let scale: FtFixed = if dim == AF_DIMENSION_HORZ {
        hints.x_scale
    } else {
        hints.y_scale
    };
    let y_scale = hints.y_scale;

    let axis = &mut hints.axis[dim];
    let segment_limit = axis.num_segments as usize;

    axis.num_edges = 0;

    if dim == AF_DIMENSION_VERT {
        top_to_bottom_hinting = script_class.top_to_bottom_hinting;
    }

    /*
     * We ignore all segments that are less than 1 pixel in length
     * to avoid many problems with serif fonts.  We compute the
     * corresponding threshold in font units.
     */
    let segment_length_threshold: FtPos = if dim == AF_DIMENSION_HORZ {
        ft_div_fix(64, y_scale)
    } else {
        0
    };

    /*
     * Similarly, we ignore segments that have a width delta
     * larger than 0.5px (i.e., a width larger than 1px).
     */
    let segment_width_threshold = ft_div_fix(32, scale);

    /**********************************************************************
     *
     * We begin by generating a sorted table of edges for the current
     * direction.  To do so, we simply scan each segment and try to find
     * an edge in our table that corresponds to its position.
     *
     * If no edge is found, we create and insert a new edge in the
     * sorted table.  Otherwise, we simply add the segment to the edge's
     * list which gets processed in the second step to compute the
     * edge's properties.
     *
     * Note that the table of edges is sorted along the segment/edge
     * position.
     *
     */

    /* assure that edge distance threshold is at most 0.25px */
    let mut edge_distance_threshold = ft_mul_fix(laxis.edge_distance_threshold, scale);
    if edge_distance_threshold > 64 / 4 {
        edge_distance_threshold = 64 / 4;
    }

    edge_distance_threshold = ft_div_fix(edge_distance_threshold, scale);

    for seg in 0..segment_limit {
        let mut found: Option<usize> = None;

        /* ignore too short segments, too wide ones, and, in this loop, */
        /* one-point segments without a direction                       */
        let s = axis.segments[seg];
        if (s.height as FtPos) < segment_length_threshold
            || s.delta as FtPos > segment_width_threshold
            || s.dir as AfDirection == AF_DIR_NONE
        {
            continue;
        }

        /* A special case for serif edges: If they are smaller than */
        /* 1.5 pixels we ignore them.                               */
        if s.serif.is_some() && 2 * (s.height as FtPos) < 3 * segment_length_threshold {
            continue;
        }

        /* look for an edge corresponding to the segment */
        for ee in 0..axis.num_edges as usize {
            let edge = &axis.edges[ee];

            let mut dist = s.pos as FtPos - edge.fpos as FtPos;
            if dist < 0 {
                dist = -dist;
            }

            if dist < edge_distance_threshold && edge.dir == s.dir {
                found = Some(ee);
                break;
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
            let edge = af_axis_hints_new_edge(
                axis,
                s.pos as FtInt,
                s.dir as AfDirection,
                top_to_bottom_hinting,
            )?;

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

    /* we loop again over all segments to catch one-point segments   */
    /* without a direction: if possible, link them to existing edges */
    for seg in 0..segment_limit {
        let mut found: Option<usize> = None;

        if axis.segments[seg].dir as AfDirection != AF_DIR_NONE {
            continue;
        }

        /* look for an edge corresponding to the segment */
        for ee in 0..axis.num_edges as usize {
            let edge = &axis.edges[ee];

            let mut dist = axis.segments[seg].pos as FtPos - edge.fpos as FtPos;
            if dist < 0 {
                dist = -dist;
            }

            if dist < edge_distance_threshold {
                found = Some(ee);
                break;
            }
        }

        /* one-point segments without a match are ignored */
        if let Some(found) = found {
            axis.segments[seg].edge_next = axis.edges[found].first;
            let found_last = axis.edges[found].last.expect("edge with segments");
            axis.segments[found_last].edge_next = Some(seg);
            axis.edges[found].last = Some(seg);
        }
    }

    /******************************************************************
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

            let first = edges[edge].first.expect("edge with segments");
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
                    Some(serif) => matches!(segments[serif].edge, Some(e) if e != edge),
                    None => false,
                };

                let link_has_edge =
                    matches!(segments[seg].link, Some(l) if segments[l].edge.is_some());
                if link_has_edge || is_serif {
                    let mut edge2 = edges[edge].link;
                    let mut seg2 = segments[seg].link;

                    if is_serif {
                        seg2 = segments[seg].serif;
                        edge2 = edges[edge].serif;
                    }

                    let seg2 = seg2.expect("linked segment");

                    if let Some(e2) = edge2 {
                        let mut edge_delta = edges[edge].fpos as FtPos - edges[e2].fpos as FtPos;
                        if edge_delta < 0 {
                            edge_delta = -edge_delta;
                        }

                        let mut seg_delta =
                            segments[seg].pos as FtPos - segments[seg2].pos as FtPos;
                        if seg_delta < 0 {
                            seg_delta = -seg_delta;
                        }

                        if seg_delta < edge_delta {
                            edge2 = segments[seg2].edge;
                        }
                    } else {
                        edge2 = segments[seg2].edge;
                    }

                    if is_serif {
                        edges[edge].serif = edge2;
                        let e2 = edge2.expect("serif edge");
                        edges[e2].flags |= AF_EDGE_SERIF;
                    } else {
                        edges[edge].link = edge2;
                    }
                }

                seg = segments[seg].edge_next.expect("segment in an edge");
                if seg == first {
                    break;
                }
            }

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

/// `af_latin_hints_detect_features`
pub fn af_latin_hints_detect_features(
    hints: &mut AfGlyphHintsRec,
    metrics: &AfStyleMetricsRec,
    width_count: FtUInt,
    widths: &[AfWidthRec],
    dim: AfDimension,
) -> FtResult<()> {
    af_latin_hints_compute_segments(hints, metrics, dim)?;
    af_latin_hints_link_segments(hints, metrics, width_count, widths, dim);
    af_latin_hints_compute_edges(hints, metrics, dim)
}

/* Compute all edges which lie within blue zones. */

/// `af_latin_hints_compute_blue_edges`
fn af_latin_hints_compute_blue_edges(hints: &mut AfGlyphHintsRec, metrics: &AfStyleMetricsRec) {
    let lmetrics = metrics.latin();
    let axis = &mut hints.axis[AF_DIMENSION_VERT];
    let major_dir = axis.major_dir;
    let edge_limit = axis.num_edges as usize;
    let latin = &lmetrics.axis[AF_DIMENSION_VERT];
    let scale = latin.scale;

    /* compute which blue zones are active, i.e. have their scaled */
    /* size < 3/4 pixels                                           */

    /* for each horizontal edge search the blue zone which is closest */
    for edge in &mut axis.edges[..edge_limit] {
        let mut best_blue: Option<AfWidthRec> = None;
        let mut best_blue_is_neutral = false;

        /* compute the initial threshold as a fraction of the EM size */
        /* (the value 40 is heuristic)                                */
        let mut best_dist = ft_mul_fix(lmetrics.units_per_em as FtPos / 40, scale); /* initial threshold */

        /* assure a minimum distance of 0.5px */
        if best_dist > 64 / 2 {
            best_dist = 64 / 2;
        }

        for bb in 0..latin.blue_count as usize {
            let blue = &latin.blues[bb];

            /* skip inactive blue zones (i.e., those that are too large) */
            if blue.flags & AF_LATIN_BLUE_ACTIVE == 0 {
                continue;
            }

            /* if it is a top zone, check for right edges (against the major */
            /* direction); if it is a bottom zone, check for left edges (in  */
            /* the major direction) -- this assumes the TrueType convention  */
            /* for the orientation of contours                               */
            let is_top_blue = (blue.flags & (AF_LATIN_BLUE_TOP | AF_LATIN_BLUE_SUB_TOP)) != 0;
            let is_neutral_blue = (blue.flags & AF_LATIN_BLUE_NEUTRAL) != 0;
            let is_major_dir = edge.dir as AfDirection == major_dir;

            /* neutral blue zones are handled for both directions */
            if is_top_blue ^ is_major_dir || is_neutral_blue {
                /* first of all, compare it to the reference position */
                let mut dist = edge.fpos as FtPos - blue.ref_.org;
                if dist < 0 {
                    dist = -dist;
                }

                dist = ft_mul_fix(dist, scale);
                if dist < best_dist {
                    best_dist = dist;
                    best_blue = Some(blue.ref_);
                    best_blue_is_neutral = is_neutral_blue;
                }

                /* now compare it to the overshoot position and check whether */
                /* the edge is rounded, and whether the edge is over the      */
                /* reference position of a top zone, or under the reference   */
                /* position of a bottom zone (provided we don't have a        */
                /* neutral blue zone)                                         */
                if edge.flags & AF_EDGE_ROUND != 0 && dist != 0 && !is_neutral_blue {
                    let is_under_ref = (edge.fpos as FtPos) < blue.ref_.org;

                    if is_top_blue ^ is_under_ref {
                        dist = edge.fpos as FtPos - blue.shoot.org;
                        if dist < 0 {
                            dist = -dist;
                        }

                        dist = ft_mul_fix(dist, scale);
                        if dist < best_dist {
                            best_dist = dist;
                            best_blue = Some(blue.shoot);
                            best_blue_is_neutral = is_neutral_blue;
                        }
                    }
                }
            }
        }

        if best_blue.is_some() {
            edge.blue_edge = best_blue;
            if best_blue_is_neutral {
                edge.flags |= AF_EDGE_NEUTRAL;
            }
        }
    }
}

/* Initalize hinting engine. */

/// `af_latin_hints_init`
fn af_latin_hints_init(
    hints: &mut AfGlyphHintsRec,
    metrics: &AfStyleMetricsRec, /* AF_LatinMetrics */
) -> FtResult<()> {
    let face = &metrics.scaler.face;
    let latin = metrics.latin();

    af_glyph_hints_rescale(hints, metrics);

    /*
     * correct x_scale and y_scale if needed, since they may have
     * been modified by `af_latin_metrics_scale_dim' above
     */
    hints.x_scale = latin.axis[AF_DIMENSION_HORZ].scale;
    hints.x_delta = latin.axis[AF_DIMENSION_HORZ].delta;
    hints.y_scale = latin.axis[AF_DIMENSION_VERT].scale;
    hints.y_delta = latin.axis[AF_DIMENSION_VERT].delta;

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

    /*
     * In `light' or `lcd' mode we disable horizontal hinting completely.
     * We also do it if the face is italic.
     *
     * However, if warping is enabled (which only works in `light' hinting
     * mode), advance widths get adjusted, too.
     */
    if mode == FT_RENDER_MODE_LIGHT
        || mode == FT_RENDER_MODE_LCD
        || (face.style_flags & FT_STYLE_FLAG_ITALIC) != 0
    {
        scaler_flags |= AF_SCALER_FLAG_NO_HORIZONTAL;
    }

    hints.scaler_flags = scaler_flags;
    hints.other_flags = other_flags;

    Ok(())
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****        L A T I N   G L Y P H   G R I D - F I T T I N G        *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/* Snap a given width in scaled coordinates to one of the */
/* current standard widths.                               */

/// `af_latin_snap_width`
fn af_latin_snap_width(widths: &[AfWidthRec], count: FtUInt, mut width: FtPos) -> FtPos {
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

/* Compute the snapped width of a given stem, ignoring very thin ones. */
/* There is a lot of voodoo in this function; changing the hard-coded  */
/* parameters influence the whole hinting process.                     */

/// `af_latin_compute_stem_width` (`hints` is its `other_flags`)
fn af_latin_compute_stem_width(
    other_flags: FtUInt32,
    metrics: &AfStyleMetricsRec,
    dim: AfDimension,
    width: FtPos,
    base_delta: FtPos,
    base_flags: FtUInt,
    stem_flags: FtUInt,
) -> FtPos {
    let axis = &metrics.latin().axis[dim];
    let mut dist = width;
    let mut sign = false;
    let vertical = dim == AF_DIMENSION_VERT;

    if !af_latin_hints_do_stem_adjust(other_flags) || axis.extra_light {
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

            /* leave the widths of serifs alone */
            if (stem_flags & AF_EDGE_SERIF as FtUInt) != 0 && vertical && (dist < 3 * 64) {
                break 'done;
            } else if base_flags & AF_EDGE_ROUND as FtUInt != 0 {
                if dist < 80 {
                    dist = 64;
                }
            } else if dist < 56 {
                dist = 56;
            }

            if axis.width_count > 0 {
                /* compare to standard width */
                let mut delta = dist - axis.widths[0].cur;

                if delta < 0 {
                    delta = -delta;
                }

                if delta < 40 {
                    dist = axis.widths[0].cur;
                    if dist < 48 {
                        dist = 48;
                    }

                    break 'done;
                }

                if dist < 3 * 64 {
                    delta = dist & 63;
                    dist &= -64;

                    if delta < 10 {
                        dist += delta;
                    } else if delta < 32 {
                        dist += 10;
                    } else if delta < 54 {
                        dist += 54;
                    } else {
                        dist += delta;
                    }
                } else {
                    /* A stem's end position depends on two values: the start        */
                    /* position and the stem length.  The former gets usually        */
                    /* rounded to the grid, while the latter gets rounded also if it */
                    /* exceeds a certain length (see below in this function).  This  */
                    /* `double rounding' can lead to a great difference to the       */
                    /* original, unhinted position; this normally doesn't matter for */
                    /* large PPEM values, but for small sizes it can easily make     */
                    /* outlines collide.  For this reason, we adjust the stem length */
                    /* by a small amount depending on the PPEM value in case the     */
                    /* former and latter rounding both point into the same           */
                    /* direction.                                                    */

                    let mut bdelta: FtPos = 0;

                    if ((width > 0) && (base_delta > 0)) || ((width < 0) && (base_delta < 0)) {
                        let ppem = metrics.scaler.face.x_ppem as FtUInt;

                        if ppem < 10 {
                            bdelta = base_delta;
                        } else if ppem < 30 {
                            bdelta = (base_delta * (30 - ppem) as FtPos) / 20;
                        }

                        if bdelta < 0 {
                            bdelta = -bdelta;
                        }
                    }

                    dist = (dist - bdelta + 32) & !63;
                }
            }
        } else {
            /* strong hinting process: snap the stem width to integer pixels */

            let org_dist = dist;

            dist = af_latin_snap_width(&axis.widths, axis.width_count, dist);

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
                    /* We only round to an integer width if the corresponding */
                    /* distortion is less than 1/4 pixel.  Otherwise this     */
                    /* makes everything worse since the diagonals, which are  */
                    /* not hinted, appear a lot bolder or thinner than the    */
                    /* vertical stems.                                        */

                    dist = (dist + 22) & !63;
                    let mut delta = dist - org_dist;
                    if delta < 0 {
                        delta = -delta;
                    }

                    if delta >= 16 {
                        dist = org_dist;
                        if dist < 48 {
                            dist = (dist + 64) >> 1;
                        }
                    }
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

/// `af_latin_align_linked_edge`
fn af_latin_align_linked_edge(
    other_flags: FtUInt32,
    metrics: &AfStyleMetricsRec,
    dim: AfDimension,
    edges: &mut [AfEdgeRec],
    base_edge: usize,
    stem_edge: usize,
) {
    let dist = edges[stem_edge].opos - edges[base_edge].opos;

    let base_delta = edges[base_edge].pos - edges[base_edge].opos;

    let fitted_width = af_latin_compute_stem_width(
        other_flags,
        metrics,
        dim,
        dist,
        base_delta,
        edges[base_edge].flags as FtUInt,
        edges[stem_edge].flags as FtUInt,
    );

    edges[stem_edge].pos = edges[base_edge].pos + fitted_width;
}

/* Shift the coordinates of the `serif' edge by the same amount */
/* as the corresponding `base' edge has been moved already.     */

/// `af_latin_align_serif_edge`
fn af_latin_align_serif_edge(edges: &mut [AfEdgeRec], base: usize, serif: usize) {
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

/* The main grid-fitting routine. */

/// `af_latin_hint_edges`
fn af_latin_hint_edges(hints: &mut AfGlyphHintsRec, metrics: &AfStyleMetricsRec, dim: AfDimension) {
    let other_flags = hints.other_flags;
    let do_blues = af_hints_do_blues(hints);
    let axis = &mut hints.axis[dim];
    let edge_limit = axis.num_edges as usize;
    let edges = &mut axis.edges[..];
    let mut anchor: Option<usize> = None;
    let mut has_serifs: FtInt = 0;

    let style_class = metrics.style_class;
    let script_class = &AF_SCRIPT_CLASSES[style_class.script as usize];

    let mut top_to_bottom_hinting = false;

    if dim == AF_DIMENSION_VERT {
        top_to_bottom_hinting = script_class.top_to_bottom_hinting;
    }

    /* we begin by aligning all stems relative to the blue zone */
    /* if needed -- that's only for horizontal edges            */

    if dim == AF_DIMENSION_VERT && do_blues {
        for edge in 0..edge_limit {
            let blue: Option<AfWidthRec>;
            let mut edge1: Option<usize>; /* these edges form the stem to check */
            let mut edge2: Option<usize>;

            if edges[edge].flags & AF_EDGE_DONE != 0 {
                continue;
            }

            edge1 = None;
            edge2 = edges[edge].link;

            /*
             * If a stem contains both a neutral and a non-neutral blue zone,
             * skip the neutral one.  Otherwise, outlines with different
             * directions might be incorrectly aligned at the same vertical
             * position.
             *
             * If we have two neutral blue zones, skip one of them.
             *
             */
            if let Some(e2) = edge2 {
                if edges[edge].blue_edge.is_some() && edges[e2].blue_edge.is_some() {
                    let neutral = edges[edge].flags & AF_EDGE_NEUTRAL;
                    let neutral2 = edges[e2].flags & AF_EDGE_NEUTRAL;

                    if neutral2 != 0 {
                        edges[e2].blue_edge = None;
                        edges[e2].flags &= !AF_EDGE_NEUTRAL;
                    } else if neutral != 0 {
                        edges[edge].blue_edge = None;
                        edges[edge].flags &= !AF_EDGE_NEUTRAL;
                    }
                }
            }

            let mut b = edges[edge].blue_edge;
            if b.is_some() {
                edge1 = Some(edge);
            }
            /* flip edges if the other edge is aligned to a blue zone */
            else if let Some(e2) = edge2 {
                if edges[e2].blue_edge.is_some() {
                    b = edges[e2].blue_edge;
                    edge1 = Some(e2);
                    edge2 = Some(edge);
                }
            }
            blue = b;

            let Some(edge1) = edge1 else {
                continue;
            };

            edges[edge1].pos = blue.expect("blue edge").fit;
            edges[edge1].flags |= AF_EDGE_DONE;

            if let Some(e2) = edge2 {
                if edges[e2].blue_edge.is_none() {
                    af_latin_align_linked_edge(other_flags, metrics, dim, edges, edge1, e2);
                    edges[e2].flags |= AF_EDGE_DONE;
                }
            }

            if anchor.is_none() {
                anchor = Some(edge);
            }
        }
    }

    /* now we align all other stem edges, trying to maintain the */
    /* relative order of stems in the glyph                      */
    for edge in 0..edge_limit {
        if edges[edge].flags & AF_EDGE_DONE != 0 {
            continue;
        }

        /* skip all non-stem edges */
        let Some(edge2) = edges[edge].link else {
            has_serifs += 1;
            continue;
        };

        /* now align the stem */

        /* this should not happen, but it's better to be safe */
        if edges[edge2].blue_edge.is_some() {
            af_latin_align_linked_edge(other_flags, metrics, dim, edges, edge2, edge);
            edges[edge].flags |= AF_EDGE_DONE;
            continue;
        }

        if let Some(anchor_) = anchor {
            let mut org_pos = edges[anchor_].pos + (edges[edge].opos - edges[anchor_].opos);
            let mut org_len = edges[edge2].opos - edges[edge].opos;
            let mut org_center = org_pos + (org_len >> 1);

            let mut cur_len = af_latin_compute_stem_width(
                other_flags,
                metrics,
                dim,
                org_len,
                0,
                edges[edge].flags as FtUInt,
                edges[edge2].flags as FtUInt,
            );

            if edges[edge2].flags & AF_EDGE_DONE != 0 {
                edges[edge].pos = edges[edge2].pos - cur_len;
            } else if cur_len < 96 {
                let u_off: FtPos;
                let d_off: FtPos;

                let mut cur_pos1 = ft_pix_round(org_center);

                if cur_len <= 64 {
                    u_off = 32;
                    d_off = 32;
                } else {
                    u_off = 38;
                    d_off = 26;
                }

                let mut delta1 = org_center - (cur_pos1 - u_off);
                if delta1 < 0 {
                    delta1 = -delta1;
                }

                let mut delta2 = org_center - (cur_pos1 + d_off);
                if delta2 < 0 {
                    delta2 = -delta2;
                }

                if delta1 < delta2 {
                    cur_pos1 -= u_off;
                } else {
                    cur_pos1 += d_off;
                }

                edges[edge].pos = cur_pos1 - cur_len / 2;
                edges[edge2].pos = cur_pos1 + cur_len / 2;
            } else {
                org_pos = edges[anchor_].pos + (edges[edge].opos - edges[anchor_].opos);
                org_len = edges[edge2].opos - edges[edge].opos;
                org_center = org_pos + (org_len >> 1);

                cur_len = af_latin_compute_stem_width(
                    other_flags,
                    metrics,
                    dim,
                    org_len,
                    0,
                    edges[edge].flags as FtUInt,
                    edges[edge2].flags as FtUInt,
                );

                let cur_pos1 = ft_pix_round(org_pos);
                let mut delta1 = cur_pos1 + (cur_len >> 1) - org_center;
                if delta1 < 0 {
                    delta1 = -delta1;
                }

                let cur_pos2 = ft_pix_round(org_pos + org_len) - cur_len;
                let mut delta2 = cur_pos2 + (cur_len >> 1) - org_center;
                if delta2 < 0 {
                    delta2 = -delta2;
                }

                edges[edge].pos = if delta1 < delta2 { cur_pos1 } else { cur_pos2 };
                edges[edge2].pos = edges[edge].pos + cur_len;
            }

            edges[edge].flags |= AF_EDGE_DONE;
            edges[edge2].flags |= AF_EDGE_DONE;

            if edge > 0
                && (if top_to_bottom_hinting {
                    edges[edge].pos > edges[edge - 1].pos
                } else {
                    edges[edge].pos < edges[edge - 1].pos
                })
            {
                /* don't move if stem would (almost) disappear otherwise; */
                /* the ad-hoc value 16 corresponds to 1/4px               */
                if let Some(link) = edges[edge].link {
                    if ft_abs(edges[link].pos - edges[edge - 1].pos) > 16 {
                        edges[edge].pos = edges[edge - 1].pos;
                    }
                }
            }
        } else {
            /* if we reach this if clause, no stem has been aligned yet */

            let org_len = edges[edge2].opos - edges[edge].opos;
            let cur_len = af_latin_compute_stem_width(
                other_flags,
                metrics,
                dim,
                org_len,
                0,
                edges[edge].flags as FtUInt,
                edges[edge2].flags as FtUInt,
            );

            /* some voodoo to specially round edges for small stem widths; */
            /* the idea is to align the center of a stem, then shifting    */
            /* the stem edges to suitable positions                        */
            let u_off: FtPos;
            let d_off: FtPos;
            if cur_len <= 64 {
                /* width <= 1px */
                u_off = 32;
                d_off = 32;
            } else {
                /* 1px < width < 1.5px */
                u_off = 38;
                d_off = 26;
            }

            if cur_len < 96 {
                let org_center = edges[edge].opos + (org_len >> 1);
                let mut cur_pos1 = ft_pix_round(org_center);

                let mut error1 = org_center - (cur_pos1 - u_off);
                if error1 < 0 {
                    error1 = -error1;
                }

                let mut error2 = org_center - (cur_pos1 + d_off);
                if error2 < 0 {
                    error2 = -error2;
                }

                if error1 < error2 {
                    cur_pos1 -= u_off;
                } else {
                    cur_pos1 += d_off;
                }

                edges[edge].pos = cur_pos1 - cur_len / 2;
                edges[edge2].pos = edges[edge].pos + cur_len;
            } else {
                edges[edge].pos = ft_pix_round(edges[edge].opos);
            }

            anchor = Some(edge);
            edges[edge].flags |= AF_EDGE_DONE;

            af_latin_align_linked_edge(other_flags, metrics, dim, edges, edge, edge2);
        }
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

        if span < 8 {
            let delta = edges[edge3].pos - (2 * edges[edge2].pos - edges[edge1].pos);
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

    if has_serifs != 0 || anchor.is_none() {
        /*
         * now hint the remaining edges (serifs and single) in order
         * to complete our processing
         */
        for edge in 0..edge_limit {
            if edges[edge].flags & AF_EDGE_DONE != 0 {
                continue;
            }

            let mut delta: FtPos = 1000;

            if let Some(serif) = edges[edge].serif {
                delta = edges[serif].opos - edges[edge].opos;
                if delta < 0 {
                    delta = -delta;
                }
            }

            if delta < 64 + 16 {
                let serif = edges[edge].serif.expect("serif edge");
                af_latin_align_serif_edge(edges, serif, edge);
            } else if anchor.is_none() {
                edges[edge].pos = ft_pix_round(edges[edge].opos);
                anchor = Some(edge);
            } else {
                let anchor_ = anchor.unwrap();
                let mut before = edge as isize - 1;
                while before >= 0 {
                    if edges[before as usize].flags & AF_EDGE_DONE != 0 {
                        break;
                    }
                    before -= 1;
                }

                let mut after = edge + 1;
                while after < edge_limit {
                    if edges[after].flags & AF_EDGE_DONE != 0 {
                        break;
                    }
                    after += 1;
                }

                if before >= 0 && (before as usize) < edge && after < edge_limit && after > edge {
                    let before = before as usize;
                    if edges[after].opos == edges[before].opos {
                        edges[edge].pos = edges[before].pos;
                    } else {
                        edges[edge].pos = edges[before].pos
                            + ft_mul_div(
                                edges[edge].opos - edges[before].opos,
                                edges[after].pos - edges[before].pos,
                                edges[after].opos - edges[before].opos,
                            );
                    }
                } else {
                    edges[edge].pos =
                        edges[anchor_].pos + ((edges[edge].opos - edges[anchor_].opos + 16) & !31);
                }
            }

            edges[edge].flags |= AF_EDGE_DONE;

            if edge > 0
                && (if top_to_bottom_hinting {
                    edges[edge].pos > edges[edge - 1].pos
                } else {
                    edges[edge].pos < edges[edge - 1].pos
                })
            {
                /* don't move if stem would (almost) disappear otherwise; */
                /* the ad-hoc value 16 corresponds to 1/4px               */
                if let Some(link) = edges[edge].link {
                    if ft_abs(edges[link].pos - edges[edge - 1].pos) > 16 {
                        edges[edge].pos = edges[edge - 1].pos;
                    }
                }
            }

            if edge + 1 < edge_limit
                && edges[edge + 1].flags & AF_EDGE_DONE != 0
                && (if top_to_bottom_hinting {
                    edges[edge].pos < edges[edge + 1].pos
                } else {
                    edges[edge].pos > edges[edge + 1].pos
                })
            {
                /* don't move if stem would (almost) disappear otherwise; */
                /* the ad-hoc value 16 corresponds to 1/4px               */
                /* FIXME (upstream): this compares with `edge[-1]', which */
                /* is before the edges array for the first edge; the     */
                /* edges hinted here have no link, though, so it isn't   */
                /* reached                                               */
                if let Some(link) = edges[edge].link {
                    if edge > 0 && ft_abs(edges[link].pos - edges[edge - 1].pos) > 16 {
                        edges[edge].pos = edges[edge + 1].pos;
                    }
                }
            }
        }
    }
}

/* Apply the complete hinting algorithm to a latin glyph. */

/// `af_latin_hints_apply`
fn af_latin_hints_apply(
    glyph_index: FtUInt,
    hints: &mut AfGlyphHintsRec,
    outline: &mut FtOutline,
    metrics: &AfStyleMetricsRec, /* AF_LatinMetrics */
    globals: &AfFaceGlobalsRec,
) -> FtResult<()> {
    let latin = metrics.latin();

    af_glyph_hints_reload(hints, outline)?;

    /* analyze glyph outline */
    if af_hints_do_horizontal(hints) {
        let axis = &latin.axis[AF_DIMENSION_HORZ];
        af_latin_hints_detect_features(
            hints,
            metrics,
            axis.width_count,
            &axis.widths,
            AF_DIMENSION_HORZ,
        )?;
    }

    if af_hints_do_vertical(hints) {
        let axis = &latin.axis[AF_DIMENSION_VERT];
        af_latin_hints_detect_features(
            hints,
            metrics,
            axis.width_count,
            &axis.widths,
            AF_DIMENSION_VERT,
        )?;

        /* apply blue zones to base characters only */
        if globals.glyph_styles[glyph_index as usize] & AF_NONBASE == 0 {
            af_latin_hints_compute_blue_edges(hints, metrics);
        }
    }

    /* grid-fit the outline */
    for dim in 0..AF_DIMENSION_MAX {
        if (dim == AF_DIMENSION_HORZ && af_hints_do_horizontal(hints))
            || (dim == AF_DIMENSION_VERT && af_hints_do_vertical(hints))
        {
            af_latin_hint_edges(hints, metrics, dim);
            af_glyph_hints_align_edge_points(hints, dim);
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
/*****              L A T I N   S C R I P T   C L A S S              *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `af_latin_writing_system_class`
pub static AF_LATIN_WRITING_SYSTEM_CLASS: AfWritingSystemClassRec = AfWritingSystemClassRec {
    writing_system: AF_WRITING_SYSTEM_LATIN,

    /* sizeof ( AF_LatinMetricsRec ) */
    style_metrics_init: Some(af_latin_metrics_init), /* style_metrics_init    */
    style_metrics_scale: Some(af_latin_metrics_scale), /* style_metrics_scale   */
    style_metrics_done: None,                        /* style_metrics_done    */
    style_metrics_getstdw: Some(af_latin_get_standard_widths), /* style_metrics_getstdw */

    style_hints_init: Some(af_latin_hints_init), /* style_hints_init      */
    style_hints_apply: Some(af_latin_hints_apply), /* style_hints_apply     */
};
