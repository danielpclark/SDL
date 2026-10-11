// Rust translation of src/pfr/pfrcmap.c and src/pfr/pfrcmap.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2002-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType PFR cmap handling (body).
//!
//! The charmap keeps a copy of the physical font's character codes (C's
//! `chars` points to its characters).

use super::super::base::ftmemory::ft_new_array;
use super::super::base::ftobjs::*;
use super::super::fttypes::*;
use super::pfrobjs::PfrFaceRec;

/// `PFR_CMapRec`
#[derive(Debug, Clone, Default)]
pub struct PfrCMapRec {
    pub num_chars: FtUInt,
    /// the codes of `chars`
    pub chars: Vec<FtUInt>,
}

/// `pfr_cmap_init`
pub fn pfr_cmap_init(face: &PfrFaceRec) -> FtResult<FtCMapData> {
    let mut pfrcmap = PfrCMapRec {
        num_chars: face.phy_font.num_chars,
        chars: ft_new_array(face.phy_font.num_chars as FtLong)?,
    };
    for (code, c) in pfrcmap.chars.iter_mut().zip(&face.phy_font.chars) {
        *code = c.char_code;
    }

    /* just for safety, check that the character entries are correctly */
    /* sorted in increasing character code order                       */
    {
        for n in 1..pfrcmap.num_chars as usize {
            if pfrcmap.chars[n - 1] >= pfrcmap.chars[n] {
                return Err(FT_ERR_INVALID_TABLE);
            }
        }
    }

    /* Exit: */
    Ok(FtCMapData::Pfr(pfrcmap))
}

/// `pfr_cmap_done`
pub fn pfr_cmap_done(cmap: &mut FtCMapRec) {
    cmap.data = FtCMapData::None;
}

fn pfrcmap(cmap: &FtCMapRec) -> Option<&PfrCMapRec> {
    match &cmap.data {
        FtCMapData::Pfr(c) => Some(c),
        _ => None,
    }
}

/// `pfr_cmap_char_index`
fn pfr_cmap_char_index(cmap: &FtCMapRec, char_code: FtUInt32) -> FtUInt {
    let Some(pfrcmap) = pfrcmap(cmap) else {
        return 0;
    };
    let mut min: FtUInt = 0;
    let mut max: FtUInt = pfrcmap.num_chars;
    let mut mid: FtUInt = min + (max - min) / 2;

    while min < max {
        let gchar = pfrcmap.chars[mid as usize];

        if gchar == char_code {
            return mid + 1;
        }

        if gchar < char_code {
            min = mid + 1;
        } else {
            max = mid;
        }

        /* reasonable prediction in a continuous block */
        mid = mid.wrapping_add(char_code.wrapping_sub(gchar));
        if mid >= max || mid < min {
            mid = min + (max - min) / 2;
        }
    }
    0
}

/// `pfr_cmap_char_next`
fn pfr_cmap_char_next(cmap: &mut FtCMapRec, pchar_code: &mut FtUInt32) -> FtUInt {
    let Some(pfrcmap) = pfrcmap(cmap) else {
        return 0;
    };
    let mut result: FtUInt = 0;
    let mut char_code: FtUInt32 = pchar_code.wrapping_add(1);

    'exit: loop {
        /* Restart: */
        let mut min: FtUInt = 0;
        let mut max: FtUInt = pfrcmap.num_chars;
        let mut mid: FtUInt = min + (max - min) / 2;

        while min < max {
            let gchar = pfrcmap.chars[mid as usize];

            if gchar == char_code {
                /* FIXME (upstream): the first character (`mid' 0, glyph  */
                /* index 1) is taken for none and skipped, here and below */
                result = mid;
                if result != 0 {
                    result += 1;
                    break 'exit;
                }

                char_code = char_code.wrapping_add(1);
                continue 'exit;
            }

            if gchar < char_code {
                min = mid + 1;
            } else {
                max = mid;
            }

            /* reasonable prediction in a continuous block */
            mid = mid.wrapping_add(char_code.wrapping_sub(gchar));
            if mid >= max || mid < min {
                mid = min + (max - min) / 2;
            }
        }

        /* we didn't find it, but we have a pair just above it */
        char_code = 0;

        if min < pfrcmap.num_chars {
            let gchar = pfrcmap.chars[min as usize];
            result = min;
            if result != 0 {
                result += 1;
                char_code = gchar;
            }
        }
        break 'exit;
    }

    /* Exit: */
    *pchar_code = char_code;
    result
}

/// `pfr_cmap_class_rec`
pub static PFR_CMAP_CLASS_REC: FtCMapClassRec = FtCMapClassRec {
    char_index: pfr_cmap_char_index, /* char_index */
    char_next: pfr_cmap_char_next,   /* char_next  */

    char_var_index: None,   /* char_var_index   */
    char_var_default: None, /* char_var_default */
    variant_list: None,     /* variant_list     */
    charvariant_list: None, /* charvariant_list */
    variantchar_list: None, /* variantchar_list */
};
