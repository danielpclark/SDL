// Rust translation of src/autofit/afshaper.c and afshaper.h from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it), without HarfBuzz
// (`FT_CONFIG_OPTION_USE_HARFBUZZ` is undefined), as SDL_ttf builds it;
// with afblue.h's `GET_UTF8_CHAR`.
// Copyright (C) 2013-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! HarfBuzz interface for accessing OpenType features (here: its
//! replacement for builds without HarfBuzz, which uses the character map
//! only).

use super::super::base::ftadvanc::ft_get_advance;
use super::super::base::ftobjs::*;
use super::super::fttypes::*;
use super::afglobal::AfFaceGlobalsRec;
use super::aftypes::AfStyleClassRec;

/// `GET_UTF8_CHAR`: an auxiliary macro to decode a UTF-8 character --
/// since we only use hard-coded, self-converted data, no error checking
/// is performed
pub fn get_utf8_char(s: &[u8], p: &mut usize) -> FtULong {
    let mut ch = s[*p] as FtULong;
    *p += 1;
    if ch >= 0x80 {
        let mut len_: FtUInt;

        if ch < 0xE0 {
            len_ = 1;
            ch &= 0x1F;
        } else if ch < 0xF0 {
            len_ = 2;
            ch &= 0x0F;
        } else {
            len_ = 3;
            ch &= 0x07;
        }

        while len_ > 0 {
            ch = (ch << 6) | (s[*p] as FtULong & 0x3F);
            *p += 1;
            len_ -= 1;
        }
    }
    ch
}

/* !FT_CONFIG_OPTION_USE_HARFBUZZ */

/// `af_shaper_get_coverage`
pub fn af_shaper_get_coverage(
    _globals: &mut AfFaceGlobalsRec,
    _style_class: &AfStyleClassRec,
    _gstyles: &mut [FtUShort],
    _default_script: bool,
) -> FtResult<()> {
    Ok(())
}

/// `af_shaper_buf_create`
pub fn af_shaper_buf_create(_face: &FtFace) {}

/// `af_shaper_buf_destroy`
pub fn af_shaper_buf_destroy(_face: &FtFace, _buf: &mut FtULong) {}

/// `af_shaper_get_cluster`: returns the new position in `s` and the
/// number of glyphs of the cluster
pub fn af_shaper_get_cluster(
    s: &[u8],
    mut p: usize,
    face: &FtFace,
    buf: &mut FtULong,
) -> (usize, FtUInt) {
    let mut dummy: FtULong = 0;
    let count: FtUInt;

    while s[p] == b' ' {
        p += 1;
    }

    let ch = get_utf8_char(s, &mut p);

    /* since we don't have an engine to handle clusters, */
    /* we scan the characters but return zero            */
    while !(s[p] == b' ' || s[p] == 0) {
        dummy = get_utf8_char(s, &mut p);
    }

    if dummy != 0 {
        *buf = 0;
        count = 0;
    } else {
        *buf = ft_get_char_index(face, ch) as FtULong;
        count = 1;
    }

    (p, count)
}

/// `af_shaper_get_elem`
pub fn af_shaper_get_elem(
    face: &mut FtFace,
    buf: FtULong,
    _idx: FtUInt,
    advance: Option<&mut FtLong>,
    y_offset: Option<&mut FtLong>,
) -> FtULong {
    let glyph_index = buf;

    if let Some(advance) = advance {
        /* (on an error, the advance is left as it is) */
        if let Ok(a) = ft_get_advance(
            face,
            glyph_index as FtUInt,
            FT_LOAD_NO_SCALE | FT_LOAD_NO_HINTING | FT_LOAD_IGNORE_TRANSFORM,
        ) {
            *advance = a;
        }
    }

    if let Some(y_offset) = y_offset {
        *y_offset = 0;
    }

    glyph_index
}
