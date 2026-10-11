// Rust translation of src/pfr/pfrgload.c and src/pfr/pfrgload.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2002-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType PFR glyph loader (body).
//!
//! `loader->current.outline` is the glyph loader's current outline (its
//! `current_*` slices and counts).

use super::super::base::ftcalc::ft_mul_fix;
use super::super::base::ftgloadr::FtGlyphLoaderRec;
use super::super::base::ftmemory::ft_renew_array;
use super::super::base::ftstream::FtStreamRec;
use super::super::fttypes::*;
use super::pfrload::*;
use super::pfrtypes::*;

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                      PFR GLYPH BUILDER                        *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `pfr_glyph_init`
pub fn pfr_glyph_init(glyph: &mut PfrGlyphRec, loader: FtGlyphLoaderRec) {
    *glyph = PfrGlyphRec::default();

    glyph.loader = loader;

    glyph.loader.rewind();
}

/// `pfr_glyph_done`
pub fn pfr_glyph_done(glyph: &mut PfrGlyphRec) {
    glyph.x_control = Vec::new();
    glyph.y_control = 0;

    glyph.max_xy_control = 0;
    /* (the `#if 0'ed num_x_control and num_y_control) */

    glyph.subs = Vec::new();

    glyph.max_subs = 0;
    glyph.num_subs = 0;

    glyph.loader = FtGlyphLoaderRec::default();
    glyph.path_begun = false;
}

/// `pfr_glyph_close_contour`: close current contour, if any
fn pfr_glyph_close_contour(glyph: &mut PfrGlyphRec) {
    let loader = &mut glyph.loader;

    if !glyph.path_begun {
        return;
    }

    /* compute first and last point indices in current glyph outline */
    let n_contours = loader.current.n_contours as i32;
    let mut last: FtInt = loader.current.n_points as i32 - 1;
    let mut first: FtInt = 0;

    /* FIXME (upstream): this is the last point of the previous contour, */
    /* not the first point of this one (so only the first contour loses  */
    /* a last point on its first one)                                    */
    if n_contours > 0 {
        first = loader.current_contours()[n_contours as usize - 1] as FtInt;
    }

    /* if the last point falls on the same location as the first one */
    /* we need to delete it                                          */
    if last > first {
        let points = loader.current_points();
        let p1 = points[first as usize];
        let p2 = points[last as usize];

        if p1.x == p2.x && p1.y == p2.y {
            loader.current.n_points -= 1;
            last -= 1;
        }
    }

    /* don't add empty contours */
    if last >= first {
        loader.current_contours()[n_contours as usize] = last as i16;
        loader.current.n_contours += 1;
    }

    glyph.path_begun = false;
}

/// `pfr_glyph_start`: reset glyph to start the loading of a new glyph
fn pfr_glyph_start(glyph: &mut PfrGlyphRec) {
    glyph.path_begun = false;
}

/// `pfr_glyph_line_to`
fn pfr_glyph_line_to(glyph: &mut PfrGlyphRec, to: &FtVector) -> FtResult<()> {
    let loader = &mut glyph.loader;

    /* check that we have begun a new path */
    if !glyph.path_begun {
        return Err(FT_ERR_INVALID_TABLE);
    }

    loader.check_points_macro(1, 0)?;

    let n = loader.current.n_points as u16 as usize;

    loader.current_points()[n] = *to;
    loader.current_tags()[n] = FT_CURVE_TAG_ON;

    loader.current.n_points += 1;

    /* Exit: */
    Ok(())
}

/// `pfr_glyph_curve_to`
fn pfr_glyph_curve_to(
    glyph: &mut PfrGlyphRec,
    control1: &FtVector,
    control2: &FtVector,
    to: &FtVector,
) -> FtResult<()> {
    let loader = &mut glyph.loader;

    /* check that we have begun a new path */
    if !glyph.path_begun {
        return Err(FT_ERR_INVALID_TABLE);
    }

    loader.check_points_macro(3, 0)?;

    let n = loader.current.n_points as u16 as usize;

    let vec = &mut loader.current_points()[n..n + 3];
    vec[0] = *control1;
    vec[1] = *control2;
    vec[2] = *to;

    let tag = &mut loader.current_tags()[n..n + 3];
    tag[0] = FT_CURVE_TAG_CUBIC;
    tag[1] = FT_CURVE_TAG_CUBIC;
    tag[2] = FT_CURVE_TAG_ON;

    loader.current.n_points = (loader.current.n_points as i32 + 3) as i16;

    /* Exit: */
    Ok(())
}

/// `pfr_glyph_move_to`
fn pfr_glyph_move_to(glyph: &mut PfrGlyphRec, to: &FtVector) -> FtResult<()> {
    /* close current contour if any */
    pfr_glyph_close_contour(glyph);

    /* indicate that a new contour has started */
    glyph.path_begun = true;

    /* check that there is space for a new contour and a new point */
    glyph.loader.check_points_macro(1, 1)?;

    /* add new start point */
    pfr_glyph_line_to(glyph, to)
}

/// `pfr_glyph_end`
fn pfr_glyph_end(glyph: &mut PfrGlyphRec) {
    /* close current contour if any */
    pfr_glyph_close_contour(glyph);

    /* merge the current glyph into the stack */
    glyph.loader.add();
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                      PFR GLYPH LOADER                         *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `pfr_glyph_load_simple`: load a simple glyph
fn pfr_glyph_load_simple(
    glyph: &mut PfrGlyphRec,
    base: &[u8],
    mut p: usize,
    limit: usize,
) -> FtResult<()> {
    macro_rules! check {
        ($x:expr) => {
            if !pfr_check(p, ($x) as u64, limit) {
                /* Failure: Too_Short: */
                return Err(FT_ERR_INVALID_TABLE);
            }
        };
    }

    check!(1);
    let flags = pfr_next_byte(base, &mut p);

    /* test for composite glyphs */
    if flags & PFR_GLYPH_IS_COMPOUND != 0 {
        return Err(FT_ERR_INVALID_TABLE);
    }

    let mut x_count: FtUInt = 0;
    let mut y_count: FtUInt = 0;

    if flags & PFR_GLYPH_1BYTE_XYCOUNT != 0 {
        check!(1);
        let count = pfr_next_byte(base, &mut p);
        x_count = count & 15;
        y_count = count >> 4;
    } else {
        if flags & PFR_GLYPH_XCOUNT != 0 {
            check!(1);
            x_count = pfr_next_byte(base, &mut p);
        }

        if flags & PFR_GLYPH_YCOUNT != 0 {
            check!(1);
            y_count = pfr_next_byte(base, &mut p);
        }
    }

    let count = x_count + y_count;

    /* re-allocate array when necessary */
    if count > glyph.max_xy_control {
        let new_max = ft_pad_ceil(count as i64, 8) as FtUInt;

        ft_renew_array(&mut glyph.x_control, new_max as FtLong)?;

        glyph.max_xy_control = new_max;
    }

    glyph.y_control = x_count as usize;

    let mut mask: FtUInt = 0;
    let mut x: FtInt = 0;

    for i in 0..count {
        if (i & 7) == 0 {
            check!(1);
            mask = pfr_next_byte(base, &mut p);
        }

        if mask & 1 != 0 {
            check!(2);
            x = pfr_next_short(base, &mut p);
        } else {
            check!(1);
            x = x.wrapping_add(pfr_next_byte(base, &mut p) as FtInt);
        }

        glyph.x_control[i as usize] = x as FtPos;

        mask >>= 1;
    }

    /* XXX: we ignore the secondary stroke and edge definitions */
    /*      since we don't support native PFR hinting           */
    /*                                                          */
    if flags & PFR_GLYPH_SINGLE_EXTRA_ITEMS != 0 {
        pfr_extra_items_skip(base, &mut p, limit)?;
    }

    pfr_glyph_start(glyph);

    /* now load a simple glyph */
    {
        let mut pos = [FtVector::default(); 4];

        pos[0].x = 0;
        pos[0].y = 0;
        pos[3] = pos[0];

        loop {
            let mut args_format: FtUInt = 0;
            let mut args_count: FtUInt;

            /****************************************************************
             * read instruction
             */
            check!(1);
            let format = pfr_next_byte(base, &mut p);
            let format_low = format & 15;

            match format >> 4 {
                0 => {
                    /* end glyph */
                    args_count = 0;
                }

                1 | 4 | 5 => {
                    /* general line operation */
                    /* move to inside contour  */
                    /* move to outside contour */
                    /* Line1: */
                    args_format = format_low;
                    args_count = 1;
                }

                2 => {
                    /* horizontal line to */
                    if format_low >= x_count {
                        return Err(FT_ERR_INVALID_TABLE);
                    }
                    pos[0].x = glyph.x_control[format_low as usize];
                    pos[0].y = pos[3].y;
                    pos[3] = pos[0];
                    args_count = 0;
                }

                3 => {
                    /* vertical line to */
                    if format_low >= y_count {
                        return Err(FT_ERR_INVALID_TABLE);
                    }
                    pos[0].x = pos[3].x;
                    pos[0].y = glyph.x_control[glyph.y_control + format_low as usize];
                    pos[3] = pos[0];
                    args_count = 0;
                }

                6 => {
                    /* horizontal to vertical curve */
                    args_format = 0xB8E;
                    args_count = 3;
                }

                7 => {
                    /* vertical to horizontal curve */
                    args_format = 0xE2B;
                    args_count = 3;
                }

                _ => {
                    /* general curve to */
                    args_count = 4;
                    args_format = format_low;
                }
            }

            /************************************************************
             * now read arguments
             */
            let mut cur = 0usize;
            let mut n: FtUInt = 0;
            while n < args_count {
                /* read the X argument */
                match args_format & 3 {
                    0 => {
                        /* 8-bit index */
                        check!(1);
                        let idx = pfr_next_byte(base, &mut p);
                        if idx >= x_count {
                            return Err(FT_ERR_INVALID_TABLE);
                        }
                        pos[cur].x = glyph.x_control[idx as usize];
                    }

                    1 => {
                        /* 16-bit absolute value */
                        check!(2);
                        pos[cur].x = pfr_next_short(base, &mut p) as FtPos;
                    }

                    2 => {
                        /* 8-bit delta */
                        check!(1);
                        let delta = pfr_next_int8(base, &mut p);
                        pos[cur].x = pos[3].x + delta as FtPos;
                    }

                    _ => {
                        pos[cur].x = pos[3].x;
                    }
                }

                /* read the Y argument */
                match (args_format >> 2) & 3 {
                    0 => {
                        /* 8-bit index */
                        check!(1);
                        let idx = pfr_next_byte(base, &mut p);
                        if idx >= y_count {
                            return Err(FT_ERR_INVALID_TABLE);
                        }
                        pos[cur].y = glyph.x_control[glyph.y_control + idx as usize];
                    }

                    1 => {
                        /* 16-bit absolute value */
                        check!(2);
                        pos[cur].y = pfr_next_short(base, &mut p) as FtPos;
                    }

                    2 => {
                        /* 8-bit delta */
                        check!(1);
                        let delta = pfr_next_int8(base, &mut p);
                        pos[cur].y = pos[3].y + delta as FtPos;
                    }

                    _ => {
                        pos[cur].y = pos[3].y;
                    }
                }

                /* read the additional format flag for the general curve */
                if n == 0 && args_count == 4 {
                    check!(1);
                    args_format = pfr_next_byte(base, &mut p);
                    args_count -= 1;
                } else {
                    args_format >>= 4;
                }

                /* save the previous point */
                pos[3] = pos[cur];
                cur += 1;
                n += 1;
            }

            /************************************************************
             * finally, execute instruction
             */
            match format >> 4 {
                0 => {
                    /* end glyph => EXIT */
                    pfr_glyph_end(glyph);
                    return Ok(());
                }

                1..=3 => {
                    /* line operations */
                    pfr_glyph_line_to(glyph, &pos[0])?;
                }

                4 | 5 => {
                    /* move to inside contour  */
                    /* move to outside contour */
                    pfr_glyph_move_to(glyph, &pos[0])?;
                }

                _ => {
                    /* curve operations */
                    let (c1, c2, to) = (pos[0], pos[1], pos[2]);
                    pfr_glyph_curve_to(glyph, &c1, &c2, &to)?;
                }
            }
        } /* for (;;) */
    }
}

/// `pfr_glyph_load_compound`: load a composite/compound glyph
fn pfr_glyph_load_compound(
    glyph: &mut PfrGlyphRec,
    base: &[u8],
    mut p: usize,
    limit: usize,
) -> FtResult<()> {
    macro_rules! check {
        ($x:expr) => {
            if !pfr_check(p, ($x) as u64, limit) {
                /* Failure: Too_Short: */
                return Err(FT_ERR_INVALID_TABLE);
            }
        };
    }

    check!(1);
    let flags = pfr_next_byte(base, &mut p);

    /* test for composite glyphs */
    if flags & PFR_GLYPH_IS_COMPOUND == 0 {
        return Err(FT_ERR_INVALID_TABLE);
    }

    let count = flags & 0x3F;

    /* ignore extra items when present */
    /*                                 */
    if flags & PFR_GLYPH_COMPOUND_EXTRA_ITEMS != 0 {
        pfr_extra_items_skip(base, &mut p, limit)?;
    }

    /* we can't rely on the FT_GlyphLoader to load sub-glyphs, because   */
    /* the PFR format is dumb, using direct file offsets to point to the */
    /* sub-glyphs (instead of glyph indices).  Sigh.                     */
    /*                                                                   */
    /* For now, we load the list of sub-glyphs into a different array    */
    /* but this will prevent us from using the auto-hinter at its best   */
    /* quality.                                                          */
    /*                                                                   */
    let org_count = glyph.num_subs;

    if org_count + count > glyph.max_subs {
        let new_max = (org_count + count + 3) & (-4i32 as FtUInt);

        /* we arbitrarily limit the number of subglyphs */
        /* to avoid endless recursion                   */
        if new_max > 64 {
            return Err(FT_ERR_INVALID_TABLE);
        }

        ft_renew_array(&mut glyph.subs, new_max as FtLong)?;

        glyph.max_subs = new_max;
    }

    for i in 0..count {
        let subglyph = &mut glyph.subs[(org_count + i) as usize];
        let mut x_pos: FtInt = 0;
        let mut y_pos: FtInt = 0;

        check!(1);
        let format = pfr_next_byte(base, &mut p);

        /* read scale when available */
        subglyph.x_scale = 0x10000;
        if format & PFR_SUBGLYPH_XSCALE != 0 {
            check!(2);
            subglyph.x_scale = pfr_next_short(base, &mut p) as FtFixed * 16;
        }

        subglyph.y_scale = 0x10000;
        if format & PFR_SUBGLYPH_YSCALE != 0 {
            check!(2);
            subglyph.y_scale = pfr_next_short(base, &mut p) as FtFixed * 16;
        }

        /* read offset */
        match format & 3 {
            1 => {
                check!(2);
                x_pos = pfr_next_short(base, &mut p);
            }

            2 => {
                check!(1);
                x_pos += pfr_next_int8(base, &mut p);
            }

            _ => {}
        }

        match (format >> 2) & 3 {
            1 => {
                check!(2);
                y_pos = pfr_next_short(base, &mut p);
            }

            2 => {
                check!(1);
                y_pos += pfr_next_int8(base, &mut p);
            }

            _ => {}
        }

        subglyph.x_delta = x_pos;
        subglyph.y_delta = y_pos;

        /* read glyph position and size now */
        if format & PFR_SUBGLYPH_2BYTE_SIZE != 0 {
            check!(2);
            subglyph.gps_size = pfr_next_ushort(base, &mut p);
        } else {
            check!(1);
            subglyph.gps_size = pfr_next_byte(base, &mut p);
        }

        if format & PFR_SUBGLYPH_3BYTE_OFFSET != 0 {
            check!(3);
            subglyph.gps_offset = pfr_next_ulong(base, &mut p);
        } else {
            check!(2);
            subglyph.gps_offset = pfr_next_ushort(base, &mut p);
        }

        glyph.num_subs += 1;
    }

    /* Exit: */
    Ok(())
}

/// `pfr_glyph_load_rec`
fn pfr_glyph_load_rec(
    glyph: &mut PfrGlyphRec,
    stream: &mut FtStreamRec,
    gps_offset: FtULong,
    offset: FtULong,
    size: FtULong,
) -> FtResult<()> {
    stream.seek(gps_offset.wrapping_add(offset))?;
    stream.enter_frame(size)?;

    let limit = size as usize;
    let compound = {
        let base = stream.frame_data();
        size > 0 && base[0] as FtUInt & PFR_GLYPH_IS_COMPOUND != 0
    };

    if compound {
        let old_count = glyph.num_subs;

        /* this is a compound glyph - load it */
        let error = pfr_glyph_load_compound(glyph, stream.frame_data(), 0, limit);

        stream.exit_frame();

        error?;

        let count = glyph.num_subs - old_count;

        /* now, load each individual glyph */
        let mut error = Ok(());
        for n in 0..count {
            let subglyph = glyph.subs[(old_count + n) as usize];
            let old_points = glyph.loader.base.outline.n_points as i32;

            error = pfr_glyph_load_rec(
                glyph,
                stream,
                gps_offset,
                subglyph.gps_offset as FtULong,
                subglyph.gps_size as FtULong,
            );
            if error.is_err() {
                break;
            }

            /* note that `glyph->subs' might have been re-allocated */
            let subglyph = glyph.subs[(old_count + n) as usize];
            let base = &mut glyph.loader.base.outline;
            let num_points = base.n_points as i32 - old_points;

            /* translate and eventually scale the new glyph points */
            let vecs = &mut base.points[old_points as usize..(old_points + num_points) as usize];
            if subglyph.x_scale != 0x10000 || subglyph.y_scale != 0x10000 {
                for vec in vecs {
                    vec.x = ft_mul_fix(vec.x, subglyph.x_scale) + subglyph.x_delta as FtPos;
                    vec.y = ft_mul_fix(vec.y, subglyph.y_scale) + subglyph.y_delta as FtPos;
                }
            } else {
                for vec in vecs {
                    vec.x += subglyph.x_delta as FtPos;
                    vec.y += subglyph.y_delta as FtPos;
                }
            }

            /* proceed to next sub-glyph */
        }

        error
    } else {
        /* load a simple glyph */
        let error = pfr_glyph_load_simple(glyph, stream.frame_data(), 0, limit);

        stream.exit_frame();

        error
    }

    /* Exit: */
}

/// `pfr_glyph_load`
pub fn pfr_glyph_load(
    glyph: &mut PfrGlyphRec,
    stream: &mut FtStreamRec,
    gps_offset: FtULong,
    offset: FtULong,
    size: FtULong,
) -> FtResult<()> {
    /* initialize glyph loader */
    glyph.loader.rewind();

    glyph.num_subs = 0;

    /* load the glyph, recursively when needed */
    pfr_glyph_load_rec(glyph, stream, gps_offset, offset, size)
}
