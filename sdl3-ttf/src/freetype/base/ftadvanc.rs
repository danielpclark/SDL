// Rust translation of src/base/ftadvanc.c (and include/freetype/ftadvanc.h)
// from FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2008-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Quick computation of advance widths (body).

use super::super::fttypes::*;
use super::ftcalc::ft_mul_div;
use super::ftobjs::*;

/// `FT_ADVANCE_FLAG_FAST_ONLY`: A bit-flag to be OR-ed with the `flags`
/// parameter of the @FT_Get_Advance and @FT_Get_Advances functions.
///
/// If set, it indicates that you want these functions to fail if the
/// corresponding hinting mode or font driver doesn't allow for very quick
/// advance computation.
pub const FT_ADVANCE_FLAG_FAST_ONLY: FtInt32 = 0x20000000;

/// `ft_face_scale_advances_`
fn ft_face_scale_advances_(
    face: &FtFace,
    advances: &mut [FtFixed],
    count: FtUInt,
    flags: FtInt32,
) -> FtResult<()> {
    if flags & FT_LOAD_NO_SCALE != 0 {
        return Ok(());
    }

    if !face.has_slot_and_size {
        return Err(FT_ERR_INVALID_SIZE_HANDLE);
    }

    let scale = if flags & FT_LOAD_VERTICAL_LAYOUT != 0 {
        face.size.metrics.y_scale
    } else {
        face.size.metrics.x_scale
    };

    /* this must be the same scaling as to get linear{Hori,Vert}Advance */
    /* (see `FT_Load_Glyph' implementation in src/base/ftobjs.c)        */

    for nn in 0..count as usize {
        advances[nn] = ft_mul_div(advances[nn], scale, 64);
    }

    Ok(())
}

/* at the moment, we can perform fast advance retrieval only in */
/* the following cases:                                         */
/*                                                              */
/*  - unscaled load                                             */
/*  - unhinted load                                             */
/*  - light-hinted load                                         */
/*  - if a variations font, it must have an `HVAR' or `VVAR'    */
/*    table (thus the old MM or GX fonts don't qualify; this    */
/*    gets checked by the driver-specific functions)            */

/// `LOAD_ADVANCE_FAST_CHECK`
fn load_advance_fast_check(flags: FtInt32) -> bool {
    flags & (FT_LOAD_NO_SCALE | FT_LOAD_NO_HINTING) != 0
        || ft_load_target_mode(flags) == FT_RENDER_MODE_LIGHT
}

/* documentation is in ftadvanc.h */

/// `FT_Get_Advance`
pub fn ft_get_advance(face: &mut FtFace, gindex: FtUInt, flags: FtInt32) -> FtResult<FtFixed> {
    let mut padvance: [FtFixed; 1] = [0];

    if gindex >= face.num_glyphs as FtUInt {
        return Err(FT_ERR_INVALID_GLYPH_INDEX);
    }

    let func = face.driver_class().get_advances;
    if let Some(func) = func {
        if load_advance_fast_check(flags) {
            match func(face, gindex, 1, flags, &mut padvance) {
                Ok(()) => {
                    ft_face_scale_advances_(face, &mut padvance, 1, flags)?;
                    return Ok(padvance[0]);
                }
                Err(error) => {
                    if error != FT_ERR_UNIMPLEMENTED_FEATURE {
                        return Err(error);
                    }
                }
            }
        }
    }

    ft_get_advances(face, gindex, 1, flags, &mut padvance)?;
    Ok(padvance[0])
}

/* documentation is in ftadvanc.h */

/// `FT_Get_Advances`
pub fn ft_get_advances(
    face: &mut FtFace,
    start: FtUInt,
    count: FtUInt,
    mut flags: FtInt32,
    padvances: &mut [FtFixed],
) -> FtResult<()> {
    let num = face.num_glyphs as FtUInt;
    let end = start.wrapping_add(count);
    if start >= num || end < start || end > num {
        return Err(FT_ERR_INVALID_GLYPH_INDEX);
    }

    if count == 0 {
        return Ok(());
    }

    let func = face.driver_class().get_advances;
    if let Some(func) = func {
        if load_advance_fast_check(flags) {
            match func(face, start, count, flags, padvances) {
                Ok(()) => return ft_face_scale_advances_(face, padvances, count, flags),
                Err(error) => {
                    if error != FT_ERR_UNIMPLEMENTED_FEATURE {
                        return Err(error);
                    }
                }
            }
        }
    }

    if flags & FT_ADVANCE_FLAG_FAST_ONLY != 0 {
        return Err(FT_ERR_UNIMPLEMENTED_FEATURE);
    }

    flags |= FT_LOAD_ADVANCE_ONLY;
    let factor: FtFixed = if flags & FT_LOAD_NO_SCALE != 0 {
        1
    } else {
        1024
    };
    for nn in 0..count {
        ft_load_glyph(face, start + nn, flags)?;

        /* scale from 26.6 to 16.16, unless NO_SCALE was requested */
        padvances[nn as usize] = if flags & FT_LOAD_VERTICAL_LAYOUT != 0 {
            face.glyph.advance.y.wrapping_mul(factor)
        } else {
            face.glyph.advance.x.wrapping_mul(factor)
        };
    }

    Ok(())
}
