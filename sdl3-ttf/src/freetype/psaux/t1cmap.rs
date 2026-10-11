// Rust translation of src/psaux/t1cmap.c and src/psaux/t1cmap.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2002-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Type 1 character map support (body).
//!
//! The charmaps keep what they read of the font (C's pointers into its
//! face): the standard and expert ones its glyph names, the custom one
//! its encoding's glyph indices; the init functions take the Type 1 font
//! record of the face (a Type 1 or Type 42 one).

use std::sync::Arc;

use super::super::base::ftobjs::*;
use super::super::fttypes::*;
use super::super::psnames::psmodule::*;
use super::super::psnames::pstables::{T1_EXPERT_ENCODING, T1_STANDARD_ENCODING};
use super::super::t1types::T1FontRec;
use super::psobjs::PsTableData;

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****          TYPE1 STANDARD (AND EXPERT) ENCODING CMAPS           *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `T1_CMapStdRec`: standard (and expert) encoding cmaps (`sid_to_string`
/// is `psnames`' `ps_get_standard_strings`)
#[derive(Debug, Clone)]
pub struct T1CMapStdRec {
    pub code_to_sid: &'static [FtUShort; 256],
    pub num_glyphs: FtUInt,
    pub glyph_names: PsTableData,
}

/// `t1_cmap_std_init`
fn t1_cmap_std_init(type1: &T1FontRec, is_expert: bool) -> FtCMapData {
    FtCMapData::T1Std(Box::new(T1CMapStdRec {
        num_glyphs: type1.num_glyphs as FtUInt,
        glyph_names: type1.glyph_names.clone(),
        code_to_sid: if is_expert {
            &T1_EXPERT_ENCODING
        } else {
            &T1_STANDARD_ENCODING
        },
    }))
}

/// `t1_cmap_std_done`
pub fn t1_cmap_std_done(cmap: &mut FtCMapRec) {
    cmap.data = FtCMapData::None;
}

/// `t1_cmap_std_char_index`
fn t1_cmap_std_char_index(cmap: &FtCMapRec, char_code: FtUInt32) -> FtUInt {
    let FtCMapData::T1Std(t1cmap) = &cmap.data else {
        return 0;
    };
    let mut result: FtUInt = 0;

    if char_code < 256 {
        /* convert character code to Adobe SID string */
        let code = t1cmap.code_to_sid[char_code as usize] as FtUInt;
        let glyph_name = ps_get_standard_strings(code).unwrap_or(b"");

        /* look for the corresponding glyph name */
        for n in 0..t1cmap.num_glyphs as usize {
            if let Some(gname) = t1cmap.glyph_names.name(n) {
                if gname.first() == glyph_name.first() && gname == glyph_name {
                    result = n as FtUInt;
                    break;
                }
            }
        }
    }

    result
}

/// `t1_cmap_std_char_next`
fn t1_cmap_std_char_next(cmap: &mut FtCMapRec, pchar_code: &mut FtUInt32) -> FtUInt {
    let mut result: FtUInt = 0;
    let mut char_code: FtUInt32 = pchar_code.wrapping_add(1);

    'exit: {
        while char_code < 256 {
            result = t1_cmap_std_char_index(cmap, char_code);
            if result != 0 {
                break 'exit;
            }

            char_code += 1;
        }
        char_code = 0;
    }

    /* Exit: */
    *pchar_code = char_code;
    result
}

/// `t1_cmap_standard_init`
pub fn t1_cmap_standard_init(type1: &T1FontRec) -> FtResult<FtCMapData> {
    Ok(t1_cmap_std_init(type1, false))
}

/// `t1_cmap_standard_class_rec`
pub static T1_CMAP_STANDARD_CLASS_REC: FtCMapClassRec = FtCMapClassRec {
    char_index: t1_cmap_std_char_index, /* char_index */
    char_next: t1_cmap_std_char_next,   /* char_next  */

    char_var_index: None,   /* char_var_index   */
    char_var_default: None, /* char_var_default */
    variant_list: None,     /* variant_list     */
    charvariant_list: None, /* charvariant_list */
    variantchar_list: None, /* variantchar_list */
};

/// `t1_cmap_expert_init`
pub fn t1_cmap_expert_init(type1: &T1FontRec) -> FtResult<FtCMapData> {
    Ok(t1_cmap_std_init(type1, true))
}

/// `t1_cmap_expert_class_rec`
pub static T1_CMAP_EXPERT_CLASS_REC: FtCMapClassRec = FtCMapClassRec {
    char_index: t1_cmap_std_char_index, /* char_index */
    char_next: t1_cmap_std_char_next,   /* char_next  */

    char_var_index: None,   /* char_var_index   */
    char_var_default: None, /* char_var_default */
    variant_list: None,     /* variant_list     */
    charvariant_list: None, /* charvariant_list */
    variantchar_list: None, /* variantchar_list */
};

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                    TYPE1 CUSTOM ENCODING CMAP                 *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `T1_CMapCustomRec`
#[derive(Debug, Clone)]
pub struct T1CMapCustomRec {
    pub first: FtUInt,
    pub count: FtUInt,
    pub indices: Arc<[FtUShort]>,
}

/// `t1_cmap_custom_init`
pub fn t1_cmap_custom_init(type1: &T1FontRec) -> FtResult<FtCMapData> {
    let encoding = &type1.encoding;

    let first = encoding.code_first as FtUInt;
    Ok(FtCMapData::T1Custom(Box::new(T1CMapCustomRec {
        first,
        count: (encoding.code_last as FtUInt).wrapping_sub(first),
        indices: Arc::from(&encoding.char_index[..]),
    })))
}

/// `t1_cmap_custom_done`
pub fn t1_cmap_custom_done(cmap: &mut FtCMapRec) {
    cmap.data = FtCMapData::None;
}

/// `t1_cmap_custom_char_index`
fn t1_cmap_custom_char_index(cmap: &FtCMapRec, char_code: FtUInt32) -> FtUInt {
    let FtCMapData::T1Custom(t1cmap) = &cmap.data else {
        return 0;
    };
    let mut result: FtUInt = 0;

    if char_code >= t1cmap.first && char_code < t1cmap.first.wrapping_add(t1cmap.count) {
        result = t1cmap.indices.get(char_code as usize).copied().unwrap_or(0) as FtUInt;
    }

    result
}

/// `t1_cmap_custom_char_next`
fn t1_cmap_custom_char_next(cmap: &mut FtCMapRec, pchar_code: &mut FtUInt32) -> FtUInt {
    let FtCMapData::T1Custom(t1cmap) = &cmap.data else {
        return 0;
    };
    let mut result: FtUInt = 0;
    let mut char_code: FtUInt32 = *pchar_code;

    char_code = char_code.wrapping_add(1);

    if char_code < t1cmap.first {
        char_code = t1cmap.first;
    }

    'exit: {
        while char_code < t1cmap.first.wrapping_add(t1cmap.count) {
            result = t1cmap.indices.get(char_code as usize).copied().unwrap_or(0) as FtUInt;
            if result != 0 {
                break 'exit;
            }
            char_code += 1;
        }

        char_code = 0;
    }

    /* Exit: */
    *pchar_code = char_code;
    result
}

/// `t1_cmap_custom_class_rec`
pub static T1_CMAP_CUSTOM_CLASS_REC: FtCMapClassRec = FtCMapClassRec {
    char_index: t1_cmap_custom_char_index, /* char_index */
    char_next: t1_cmap_custom_char_next,   /* char_next  */

    char_var_index: None,   /* char_var_index   */
    char_var_default: None, /* char_var_default */
    variant_list: None,     /* variant_list     */
    charvariant_list: None, /* charvariant_list */
    variantchar_list: None, /* variantchar_list */
};

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****            TYPE1 SYNTHETIC UNICODE ENCODING CMAP              *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `psaux_get_glyph_name`
fn psaux_get_glyph_name(type1: &T1FontRec, idx: FtUInt) -> Option<Vec<u8>> {
    type1.glyph_names.name(idx as usize).map(|n| n.to_vec())
}

/// `t1_cmap_unicode_init` (the `psnames' module's `unicodes_init' is
/// called directly)
pub fn t1_cmap_unicode_init(type1: &T1FontRec) -> FtResult<FtCMapData> {
    let mut unicodes = PsUnicodesRec::default();

    let mut get_name = |idx: FtUInt| psaux_get_glyph_name(type1, idx);
    ps_unicodes_init(&mut unicodes, type1.num_glyphs as FtUInt, &mut get_name)?;

    Ok(FtCMapData::PsUnicodes(unicodes))
}

/// `t1_cmap_unicode_done`
pub fn t1_cmap_unicode_done(cmap: &mut FtCMapRec) {
    cmap.data = FtCMapData::None;
}

/// `t1_cmap_unicode_char_index`
fn t1_cmap_unicode_char_index(cmap: &FtCMapRec, char_code: FtUInt32) -> FtUInt {
    match &cmap.data {
        FtCMapData::PsUnicodes(u) => ps_unicodes_char_index(u, char_code),
        _ => 0,
    }
}

/// `t1_cmap_unicode_char_next`
fn t1_cmap_unicode_char_next(cmap: &mut FtCMapRec, pchar_code: &mut FtUInt32) -> FtUInt {
    match &cmap.data {
        FtCMapData::PsUnicodes(u) => ps_unicodes_char_next(u, pchar_code),
        _ => 0,
    }
}

/// `t1_cmap_unicode_class_rec`
pub static T1_CMAP_UNICODE_CLASS_REC: FtCMapClassRec = FtCMapClassRec {
    char_index: t1_cmap_unicode_char_index, /* char_index */
    char_next: t1_cmap_unicode_char_next,   /* char_next  */

    char_var_index: None,   /* char_var_index   */
    char_var_default: None, /* char_var_default */
    variant_list: None,     /* variant_list     */
    charvariant_list: None, /* charvariant_list */
    variantchar_list: None, /* variantchar_list */
};
