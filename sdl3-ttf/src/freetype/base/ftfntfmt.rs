// Rust translation of src/base/ftfntfmt.c from FreeType (2.13.2, as
// SDL_ttf's external/freetype pins it).
// Copyright (C) 2002-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType utility file for font formats (body).

use super::ftobjs::*;

/// `FT_Get_Font_Format`
pub fn ft_get_font_format(face: &FtFaceRec) -> Option<&'static str> {
    match ft_face_find_service(face, FT_SERVICE_ID_FONT_FORMAT) {
        Some(FtService::FontFormat(result)) => Some(result),
        _ => None,
    }
}

/// `FT_Get_X11_Font_Format`: deprecated function name; retained for ABI
/// compatibility
pub fn ft_get_x11_font_format(face: &FtFaceRec) -> Option<&'static str> {
    ft_get_font_format(face)
}
