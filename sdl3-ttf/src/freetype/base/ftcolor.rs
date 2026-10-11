// Rust translation of src/base/ftcolor.c from FreeType (2.13.2, as
// SDL_ttf's external/freetype pins it).
// Copyright (C) 2018-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType's glyph color management (body): the palette functions
//! PlutoSVG's FreeType hooks call (`FT_Palette_Data_Get`,
//! `FT_Palette_Select`; `TT_CONFIG_OPTION_COLOR_LAYERS` is defined).

use super::super::fttypes::*;
use super::super::sfnt::ttcpal::tt_face_palette_set;
use super::super::tttypes::{FtColor, FtPaletteData};
use super::ftobjs::*;

/// `FT_Palette_Data_Get` (`null_palette_data` for faces that are not
/// SFNT-based)
pub fn ft_palette_data_get(face: &FtFace) -> FtResult<FtPaletteData> {
    if ft_is_sfnt(face) {
        if let Some(ttface) = face.tt() {
            return Ok(ttface.palette_data.clone());
        }
    }
    Ok(FtPaletteData::default())
}

/// `FT_Palette_Select`: selects palette `palette_index` and returns the
/// face's palette (`None` for faces that are not SFNT-based).
pub fn ft_palette_select(
    face: &mut FtFace,
    palette_index: FtUShort,
) -> FtResult<Option<&[FtColor]>> {
    if !ft_is_sfnt(face) {
        return Ok(None);
    }

    let Some(ttface) = face.tt_mut() else {
        return Ok(None);
    };

    /* sfnt->set_palette */
    tt_face_palette_set(ttface, palette_index as FtUInt)?;

    ttface.palette_index = palette_index;

    Ok(Some(&ttface.palette))
}
