// Rust translation of src/base/ftgloadr.c and
// include/freetype/internal/ftgloadr.h from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2002-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The FreeType glyph loader (body).
//!
//! `loader.current` is a view of the base arrays starting after the base
//! outline's points, contours and subglyphs; here it is kept as its
//! counts, and the `current_*` accessors give the slices C's
//! `FT_GlyphLoader_Adjust_Points()` points it at. `extra_points2` is the
//! second half of `extra_points`, from `max_points` on.

use super::super::fttypes::*;
use super::ftmemory::ft_renew_array;

/// `FT_SubGlyphRec`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FtSubGlyphRec {
    pub index: FtInt,
    pub flags: FtUShort,
    pub arg1: FtInt,
    pub arg2: FtInt,
    pub transform: FtMatrix,
}

/// `FT_GlyphLoadRec` (the base): the outline arrays have `max_points`
/// and `max_contours` entries.
#[derive(Debug, Clone, Default)]
pub struct FtGlyphLoadRec {
    pub outline: FtOutline,
    /// `extra_points` (`max_points` entries) followed by `extra_points2`
    pub extra_points: Vec<FtVector>,
    pub num_subglyphs: FtUInt,
    pub subglyphs: Vec<FtSubGlyphRec>,
}

/// The counts of `loader.current` (`FT_GlyphLoadRec`), whose arrays start
/// where the base's end.
#[derive(Debug, Clone, Copy, Default)]
pub struct FtGlyphLoadCurrent {
    pub n_points: i16,
    pub n_contours: i16,
    pub flags: i32,
    pub num_subglyphs: FtUInt,
}

/// `FT_GlyphLoaderRec`
#[derive(Debug, Clone, Default)]
pub struct FtGlyphLoaderRec {
    pub max_points: FtUInt,
    pub max_contours: FtUInt,
    pub max_subglyphs: FtUInt,
    pub use_extra: bool,

    pub base: FtGlyphLoadRec,
    pub current: FtGlyphLoadCurrent,
}

/*************************************************************************/
/*                                                                       */
/*                    G L Y P H   L O A D E R                            */
/*                                                                       */
/*************************************************************************/

/*
 * The glyph loader is a simple object which is used to load a set of
 * glyphs easily.  It is critical for the correct loading of composites.
 *
 * Ideally, one can see it as a stack of abstract `glyph' objects.
 *
 *   loader.base     Is really the bottom of the stack.  It describes a
 *                   single glyph image made of the juxtaposition of
 *                   several glyphs (those `in the stack').
 *
 *   loader.current  Describes the top of the stack, on which a new
 *                   glyph can be loaded.
 *
 *   Rewind          Clears the stack.
 *   Prepare         Set up `loader.current' for addition of a new glyph
 *                   image.
 *   Add             Add the `current' glyph image to the `base' one,
 *                   and prepare for another one.
 *
 * The glyph loader is now a base object.  Each driver used to
 * re-implement it in one way or the other, which wasted code and
 * energy.
 *
 */

impl FtGlyphLoaderRec {
    /// `FT_GlyphLoader_New`: create a new glyph loader
    pub fn new() -> FtGlyphLoaderRec {
        FtGlyphLoaderRec::default()
    }

    /// `FT_GlyphLoader_Rewind`: rewind the glyph loader - reset counters
    /// to 0
    pub fn rewind(&mut self) {
        let base = &mut self.base;

        base.outline.n_points = 0;
        base.outline.n_contours = 0;
        base.outline.flags = 0;
        base.num_subglyphs = 0;

        self.current = FtGlyphLoadCurrent {
            n_points: 0,
            n_contours: 0,
            flags: 0,
            num_subglyphs: 0,
        };
    }

    /// `FT_GlyphLoader_Reset`: reset glyph loader, free all allocated
    /// tables, and start from zero
    pub fn reset(&mut self) {
        self.base.outline.points = Vec::new();
        self.base.outline.tags = Vec::new();
        self.base.outline.contours = Vec::new();
        self.base.extra_points = Vec::new();
        self.base.subglyphs = Vec::new();

        self.max_points = 0;
        self.max_contours = 0;
        self.max_subglyphs = 0;

        self.rewind();
    }

    /// `FT_GlyphLoader_CreateExtra`
    pub fn create_extra(&mut self) -> FtResult<()> {
        if self.max_points == 0 || !self.base.extra_points.is_empty() {
            return Ok(());
        }

        ft_renew_array(&mut self.base.extra_points, 2 * self.max_points as FtLong)?;
        self.use_extra = true;
        Ok(())
    }

    /// `FT_GlyphLoader_CheckPoints`: Ensure that we can add `n_points'
    /// and `n_contours' to our glyph.  This function reallocates its
    /// outline tables if necessary.  Note that it DOESN'T change the
    /// number of points within the loader!
    pub fn check_points(&mut self, n_points: FtUInt, n_contours: FtUInt) -> FtResult<()> {
        let r = self.check_points_inner(n_points, n_contours);
        if r.is_err() {
            self.reset();
        }
        r
    }

    fn check_points_inner(&mut self, n_points: FtUInt, n_contours: FtUInt) -> FtResult<()> {
        self.create_extra()?;

        /* check points & tags */
        let mut new_max = (self.base.outline.n_points as u16 as FtUInt)
            .wrapping_add(self.current.n_points as u16 as FtUInt)
            .wrapping_add(n_points);
        let mut old_max = self.max_points;

        if new_max > old_max {
            if new_max > FT_OUTLINE_POINTS_MAX as FtUInt {
                return Err(FT_ERR_ARRAY_TOO_LARGE);
            }

            let min_new_max = old_max + (old_max >> 1);
            if new_max < min_new_max {
                new_max = min_new_max;
            }
            new_max = ft_pad_ceil(new_max as i64, 8) as FtUInt;
            if new_max > FT_OUTLINE_POINTS_MAX as FtUInt {
                new_max = FT_OUTLINE_POINTS_MAX as FtUInt;
            }

            ft_renew_array(&mut self.base.outline.points, new_max as FtLong)?;
            ft_renew_array(&mut self.base.outline.tags, new_max as FtLong)?;

            if self.use_extra {
                ft_renew_array(&mut self.base.extra_points, (new_max * 2) as FtLong)?;

                let (o, n) = (old_max as usize, new_max as usize);
                self.base.extra_points.copy_within(o..o + o, n);
            }

            self.max_points = new_max;
        }

        self.create_extra()?;

        /* check contours */
        old_max = self.max_contours;
        new_max = (self.base.outline.n_contours as u16 as FtUInt)
            .wrapping_add(self.current.n_contours as u16 as FtUInt)
            .wrapping_add(n_contours);
        if new_max > old_max {
            if new_max > FT_OUTLINE_CONTOURS_MAX as FtUInt {
                return Err(FT_ERR_ARRAY_TOO_LARGE);
            }

            let min_new_max = old_max + (old_max >> 1);
            if new_max < min_new_max {
                new_max = min_new_max;
            }
            new_max = ft_pad_ceil(new_max as i64, 4) as FtUInt;
            if new_max > FT_OUTLINE_CONTOURS_MAX as FtUInt {
                new_max = FT_OUTLINE_CONTOURS_MAX as FtUInt;
            }

            ft_renew_array(&mut self.base.outline.contours, new_max as FtLong)?;

            self.max_contours = new_max;
        }

        Ok(())
    }

    /// `FT_GLYPHLOADER_CHECK_POINTS`
    pub fn check_points_macro(&mut self, n_points: FtUInt, n_contours: FtUInt) -> FtResult<()> {
        let p_ok = n_points == 0
            || (self.base.outline.n_points as u16 as FtUInt)
                .wrapping_add(self.current.n_points as u16 as FtUInt)
                .wrapping_add(n_points)
                <= self.max_points;
        let c_ok = n_contours == 0
            || (self.base.outline.n_contours as u16 as FtUInt)
                .wrapping_add(self.current.n_contours as u16 as FtUInt)
                .wrapping_add(n_contours)
                <= self.max_contours;
        if p_ok && c_ok {
            Ok(())
        } else {
            self.check_points(n_points, n_contours)
        }
    }

    /// `FT_GlyphLoader_CheckSubGlyphs`: Ensure that we can add `n_subs'
    /// to our glyph. this function reallocates its subglyphs table if
    /// necessary.  Note that it DOES NOT change the number of subglyphs
    /// within the loader!
    pub fn check_subglyphs(&mut self, n_subs: FtUInt) -> FtResult<()> {
        let mut new_max = self
            .base
            .num_subglyphs
            .wrapping_add(self.current.num_subglyphs)
            .wrapping_add(n_subs);
        let old_max = self.max_subglyphs;
        if new_max > old_max {
            new_max = ft_pad_ceil(new_max as i64, 2) as FtUInt;
            ft_renew_array(&mut self.base.subglyphs, new_max as FtLong)?;

            self.max_subglyphs = new_max;
        }

        Ok(())
    }

    /// `FT_GlyphLoader_Prepare`: prepare loader for the addition of a new
    /// glyph on top of the base one
    pub fn prepare(&mut self) {
        let current = &mut self.current;

        current.n_points = 0;
        current.n_contours = 0;
        current.num_subglyphs = 0;
    }

    /// `FT_GlyphLoader_Add`: add current glyph to the base image -- and
    /// prepare for another
    pub fn add(&mut self) {
        let n_curr_contours = self.current.n_contours as i32;
        let n_base_points = self.base.outline.n_points as i32;
        let n_base_contours = self.base.outline.n_contours as usize;

        self.base.outline.n_points =
            (self.base.outline.n_points as i32 + self.current.n_points as i32) as i16;
        self.base.outline.n_contours =
            (self.base.outline.n_contours as i32 + self.current.n_contours as i32) as i16;

        self.base.num_subglyphs += self.current.num_subglyphs;

        /* adjust contours count in newest outline */
        for n in 0..n_curr_contours.max(0) as usize {
            let c = &mut self.base.outline.contours[n_base_contours + n];
            *c = (*c as i32 + n_base_points) as i16;
        }

        /* prepare for another new glyph image */
        self.prepare();
    }

    /// The first point of `current` in the base arrays.
    pub fn current_point_start(&self) -> usize {
        self.base.outline.n_points as u16 as usize
    }

    /// The first contour of `current` in the base arrays.
    pub fn current_contour_start(&self) -> usize {
        self.base.outline.n_contours as u16 as usize
    }

    /// `current.outline.points`
    pub fn current_points(&mut self) -> &mut [FtVector] {
        let s = self.current_point_start();
        &mut self.base.outline.points[s..]
    }

    /// `current.outline.tags`
    pub fn current_tags(&mut self) -> &mut [u8] {
        let s = self.current_point_start();
        &mut self.base.outline.tags[s..]
    }

    /// `current.outline.contours`
    pub fn current_contours(&mut self) -> &mut [i16] {
        let s = self.current_contour_start();
        &mut self.base.outline.contours[s..]
    }

    /// `current.extra_points`
    pub fn current_extra_points(&mut self) -> &mut [FtVector] {
        let s = self.current_point_start();
        let m = self.max_points as usize;
        &mut self.base.extra_points[s..m]
    }

    /// `current.extra_points2`
    pub fn current_extra_points2(&mut self) -> &mut [FtVector] {
        let s = self.current_point_start();
        let m = self.max_points as usize;
        &mut self.base.extra_points[m + s..]
    }

    /// `current.subglyphs`
    pub fn current_subglyphs(&mut self) -> &mut [FtSubGlyphRec] {
        let s = self.base.num_subglyphs as usize;
        &mut self.base.subglyphs[s..]
    }
}
