// Rust translation of src/cff/cffcmap.c and src/cff/cffcmap.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2002-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! CFF character mapping table (cmap) support (body).
//!
//! A charmap's `init` builds its data (`FtCMapData`) before `FT_CMap_New`
//! adds it, as in the `sfnt` module.

use super::super::base::ftobjs::*;
use super::super::cfftypes::*;
use super::super::fttypes::*;
use super::super::psnames::psmodule::*;
use super::cffload::cff_index_get_sid_string;

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****           CFF STANDARD (AND EXPERT) ENCODING CMAPS            *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `cff_cmap_encoding_init`: the charmap's `gids` (the font encoding's
/// `codes`)
pub fn cff_cmap_encoding_init(cff: &CffFontRec) -> FtCMapData {
    let encoding = &cff.encoding;

    FtCMapData::CffEncoding(Box::new(encoding.codes))
}

/// `cff_cmap_encoding_done`
pub fn cff_cmap_encoding_done(cmap: &mut FtCMapRec) {
    cmap.data = FtCMapData::None;
}

fn gids(cmap: &FtCMapRec) -> Option<&[FtUShort; 256]> {
    match &cmap.data {
        FtCMapData::CffEncoding(g) => Some(g),
        _ => None,
    }
}

fn cff_cmap_encoding_char_index(cmap: &FtCMapRec, char_code: FtUInt32) -> FtUInt {
    let mut result: FtUInt = 0;

    if char_code < 256 {
        if let Some(gids) = gids(cmap) {
            result = gids[char_code as usize] as FtUInt;
        }
    }

    result
}

fn cff_cmap_encoding_char_next(cmap: &mut FtCMapRec, pchar_code: &mut FtUInt32) -> FtUInt {
    let mut result: FtUInt = 0;
    let mut char_code: FtUInt32 = *pchar_code;

    let Some(gids) = gids(cmap) else {
        return 0;
    };

    while char_code < 255 {
        char_code += 1;
        result = gids[char_code as usize] as FtUInt;
        if result != 0 {
            *pchar_code = char_code;
            break;
        }
    }

    result
}

/// `cff_cmap_encoding_class_rec`
pub static CFF_CMAP_ENCODING_CLASS_REC: FtCMapClassRec = FtCMapClassRec {
    char_index: cff_cmap_encoding_char_index, /* char_index */
    char_next: cff_cmap_encoding_char_next,   /* char_next  */

    char_var_index: None,   /* char_var_index   */
    char_var_default: None, /* char_var_default */
    variant_list: None,     /* variant_list     */
    charvariant_list: None, /* charvariant_list */
    variantchar_list: None, /* variantchar_list */
};

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****              CFF SYNTHETIC UNICODE ENCODING CMAP              *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

fn cff_sid_to_glyph_name(cff: &CffFontRec, idx: FtUInt) -> Option<Vec<u8>> {
    let charset = &cff.charset;
    let sid: FtUInt = charset
        .sids
        .as_ref()?
        .get(idx as usize)
        .copied()
        .unwrap_or(0) as FtUInt;

    cff_index_get_sid_string(cff, sid).map(|s| s.to_vec())
}

/// `cff_cmap_unicode_init`: builds the charmap's table (`PS_Unicodes`)
pub fn cff_cmap_unicode_init(cff: &CffFontRec) -> FtResult<PsUnicodesRec> {
    let charset = &cff.charset;
    let mut unicodes = PsUnicodesRec::default();

    /* can't build Unicode map for CID-keyed font */
    /* because we don't know glyph names.         */
    if charset.sids.is_none() {
        return Err(FT_ERR_NO_UNICODE_GLYPH_NAME);
    }

    /* (the `psnames' module's `unicodes_init' is called directly) */
    let mut get_name = |idx: FtUInt| cff_sid_to_glyph_name(cff, idx);
    ps_unicodes_init(&mut unicodes, cff.num_glyphs, &mut get_name)?;

    Ok(unicodes)
}

/// `cff_cmap_unicode_done`
pub fn cff_cmap_unicode_done(cmap: &mut FtCMapRec) {
    cmap.data = FtCMapData::None;
}

fn cff_cmap_unicode_char_index(cmap: &FtCMapRec, char_code: FtUInt32) -> FtUInt {
    match &cmap.data {
        FtCMapData::PsUnicodes(u) => ps_unicodes_char_index(u, char_code),
        _ => 0,
    }
}

fn cff_cmap_unicode_char_next(cmap: &mut FtCMapRec, pchar_code: &mut FtUInt32) -> FtUInt {
    match &cmap.data {
        FtCMapData::PsUnicodes(u) => ps_unicodes_char_next(u, pchar_code),
        _ => 0,
    }
}

/// `cff_cmap_unicode_class_rec`
pub static CFF_CMAP_UNICODE_CLASS_REC: FtCMapClassRec = FtCMapClassRec {
    char_index: cff_cmap_unicode_char_index, /* char_index */
    char_next: cff_cmap_unicode_char_next,   /* char_next  */

    char_var_index: None,   /* char_var_index   */
    char_var_default: None, /* char_var_default */
    variant_list: None,     /* variant_list     */
    charvariant_list: None, /* charvariant_list */
    variantchar_list: None, /* variantchar_list */
};
