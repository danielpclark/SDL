// Rust translation of the parts of src/base/ftmm.c that HarfBuzz's hb-ft
// uses (FT_Get_MM_Var, FT_Get_Var_Blend_Coordinates) from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Multiple Master font support (body).
//!
//! The multiple masters service of the faces SDL_ttf opens is the
//! TrueType driver's (the CFF driver's forwards to it), so the service
//! calls are those of `ttgxvar`.

use super::ftobjs::*;
use crate::freetype::fttypes::*;
use crate::freetype::sfnt::ttload::with_stream;
use crate::freetype::truetype::ttgxvar::{tt_get_mm_blend, tt_get_mm_var, FtMMVar};

/// `ft_face_get_mm_service`: whether the face has the multiple masters
/// service.
fn ft_face_get_mm_service(face: &FtFace) -> FtResult<()> {
    if ft_has_multiple_masters(face) {
        return Ok(());
    }

    Err(FT_ERR_INVALID_ARGUMENT)
}

/// `FT_Get_MM_Var`
pub fn ft_get_mm_var(face: &mut FtFace) -> FtResult<FtMMVar> {
    /* check of `face' delayed to `ft_face_get_mm_service' */

    ft_face_get_mm_service(face)?;

    let Some(tt) = face.tt_mut() else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };
    let master = with_stream(tt, |tt, stream| tt_get_mm_var(tt, stream, true))?;
    master.ok_or(FT_ERR_INVALID_ARGUMENT)
}

/// `FT_Get_Var_Blend_Coordinates` (`coords.len ()` is `num_coords`)
pub fn ft_get_var_blend_coordinates(face: &mut FtFace, coords: &mut [FtFixed]) -> FtResult<()> {
    /* check of `face' delayed to `ft_face_get_mm_service' */

    ft_face_get_mm_service(face)?;

    let Some(tt) = face.tt_mut() else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };
    with_stream(tt, |tt, stream| tt_get_mm_blend(tt, stream, coords))
}
