// Rust translation of src/psnames/psmodule.c and the PS_Unicodes types of
// include/freetype/internal/services/svpscmap.h from FreeType (2.13.2, as
// SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! psnames module implementation (body).
//!
//! The file `pstables.h' with its arrays and its function
//! `ft_get_adobe_glyph_index' is useful for other projects also (for
//! example, `pdfium' is using it).
//!
//! `FT_CONFIG_OPTION_POSTSCRIPT_NAMES` and
//! `FT_CONFIG_OPTION_ADOBE_GLYPH_LIST` are defined, so the whole service is
//! here; its functions are called directly (C's `pscmaps_interface`).

use super::super::base::ftmemory::ft_new_array;
use super::super::base::ftobjs::{FtModuleClass, FtModuleInterface};
use super::super::fttypes::*;
use super::pstables::*;

/// `PS_UniMap`
#[derive(Debug, Clone, Copy, Default)]
pub struct PsUniMap {
    pub unicode: FtUInt32, /* bit 31 set: is glyph variant */
    pub glyph_index: FtUInt,
}

/// `PS_UnicodesRec` (without its `FT_CMapRec` root)
#[derive(Debug, Clone, Default)]
pub struct PsUnicodesRec {
    pub num_maps: FtUInt,
    pub maps: Vec<PsUniMap>,
}

const VARIANT_BIT: FtUInt32 = 0x80000000;

/// `BASE_GLYPH`
const fn base_glyph(code: FtUInt32) -> FtUInt32 {
    code & !VARIANT_BIT
}

/// `ps_unicode_value`: Return the Unicode value corresponding to a given
/// glyph.  Note that we do deal with glyph variants by detecting a
/// non-initial dot in the name, as in `A.swash' or `e.final'; in this
/// case, the VARIANT_BIT is set in the return value.
///
/// `glyph_name` is the name without its terminating NUL.
pub fn ps_unicode_value(glyph_name: &[u8]) -> FtUInt32 {
    let at = |i: usize| -> u8 { glyph_name.get(i).copied().unwrap_or(0) };

    /* If the name begins with `uni', then the glyph name may be a */
    /* hard-coded unicode character code.                          */
    if at(0) == b'u' && at(1) == b'n' && at(2) == b'i' {
        /* determine whether the next four characters following are */
        /* hexadecimal.                                             */

        /* XXX: Add code to deal with ligatures, i.e. glyph names like */
        /*      `uniXXXXYYYYZZZZ'...                                   */

        let mut count = 4;
        let mut value: FtUInt32 = 0;
        let mut p = 3;

        while count > 0 {
            let c = at(p);
            let mut d = (c as u32).wrapping_sub(b'0' as u32);
            if d >= 10 {
                d = (c as u32).wrapping_sub(b'A' as u32);
                if d >= 6 {
                    d = 16;
                } else {
                    d += 10;
                }
            }

            /* Exit if a non-uppercase hexadecimal character was found   */
            /* -- this also catches character codes below `0' since such */
            /* negative numbers cast to `unsigned int' are far too big.  */
            if d >= 16 {
                break;
            }

            value = (value << 4) + d;
            count -= 1;
            p += 1;
        }

        /* there must be exactly four hex digits */
        if count == 0 {
            if at(p) == b'\0' {
                return value;
            }
            if at(p) == b'.' {
                return value | VARIANT_BIT;
            }
        }
    }

    /* If the name begins with `u', followed by four to six uppercase */
    /* hexadecimal digits, it is a hard-coded unicode character code. */
    if at(0) == b'u' {
        let mut count = 6;
        let mut value: FtUInt32 = 0;
        let mut p = 1;

        while count > 0 {
            let c = at(p);
            let mut d = (c as u32).wrapping_sub(b'0' as u32);
            if d >= 10 {
                d = (c as u32).wrapping_sub(b'A' as u32);
                if d >= 6 {
                    d = 16;
                } else {
                    d += 10;
                }
            }

            if d >= 16 {
                break;
            }

            value = (value << 4) + d;
            count -= 1;
            p += 1;
        }

        if count <= 2 {
            if at(p) == b'\0' {
                return value;
            }
            if at(p) == b'.' {
                return value | VARIANT_BIT;
            }
        }
    }

    /* Look for a non-initial dot in the glyph name in order to */
    /* find variants like `A.swash', `e.final', etc.            */
    {
        let mut value: FtUInt32 = 0;
        let mut p = 0;

        while at(p) != 0 && at(p) != b'.' {
            p += 1;
        }

        /* now look up the glyph in the Adobe Glyph List;      */
        /* `.notdef', `.null' and the empty name are short cut */
        if p > 0 {
            value = ft_get_adobe_glyph_index(&glyph_name[..p]) as FtUInt32;

            if at(p) == b'.' {
                value |= VARIANT_BIT;
            }
        }

        value
    }
}

/// `compare_uni_maps`: ft_qsort callback to sort the unicode map
fn compare_uni_maps(map1: &PsUniMap, map2: &PsUniMap) -> std::cmp::Ordering {
    let unicode1 = base_glyph(map1.unicode);
    let unicode2 = base_glyph(map2.unicode);

    /* sort base glyphs before glyph variants */
    if unicode1 == unicode2 {
        map1.unicode.cmp(&map2.unicode)
    } else {
        unicode1.cmp(&unicode2)
    }
}

/* support for extra glyphs not handled (well) in AGL; */
/* we add extra mappings for them if necessary         */

const EXTRA_GLYPH_LIST_SIZE: usize = 10;

static FT_EXTRA_GLYPH_UNICODES: [FtUInt32; EXTRA_GLYPH_LIST_SIZE] = [
    /* WGL 4 */
    0x0394, 0x03A9, 0x2215, 0x00AD, 0x02C9, 0x03BC, 0x2219, 0x00A0, /* Romanian */
    0x021A, 0x021B,
];

static FT_EXTRA_GLYPH_NAMES: [&[u8]; EXTRA_GLYPH_LIST_SIZE] = [
    b"Delta",
    b"Omega",
    b"fraction",
    b"hyphen",
    b"macron",
    b"mu",
    b"periodcentered",
    b"space",
    b"Tcommaaccent",
    b"tcommaaccent",
];

/// `ps_check_extra_glyph_name`
fn ps_check_extra_glyph_name(
    gname: &[u8],
    glyph: FtUInt,
    extra_glyphs: &mut [FtUInt; EXTRA_GLYPH_LIST_SIZE],
    states: &mut [FtUInt; EXTRA_GLYPH_LIST_SIZE],
) {
    for n in 0..EXTRA_GLYPH_LIST_SIZE {
        if FT_EXTRA_GLYPH_NAMES[n] == gname {
            if states[n] == 0 {
                /* mark this extra glyph as a candidate for the cmap */
                states[n] = 1;
                extra_glyphs[n] = glyph;
            }

            return;
        }
    }
}

/// `ps_check_extra_glyph_unicode`
fn ps_check_extra_glyph_unicode(uni_char: FtUInt32, states: &mut [FtUInt; EXTRA_GLYPH_LIST_SIZE]) {
    for n in 0..EXTRA_GLYPH_LIST_SIZE {
        if uni_char == FT_EXTRA_GLYPH_UNICODES[n] {
            /* disable this extra glyph from being added to the cmap */
            states[n] = 2;

            return;
        }
    }
}

/// `ps_unicodes_init`: Build a table that maps Unicode values to glyph
/// indices. `get_glyph_name` gives the name of a glyph (without its NUL),
/// if any.
pub fn ps_unicodes_init(
    table: &mut PsUnicodesRec,
    num_glyphs: FtUInt,
    get_glyph_name: &mut dyn FnMut(FtUInt) -> Option<Vec<u8>>,
) -> FtResult<()> {
    let mut extra_glyph_list_states = [0u32; EXTRA_GLYPH_LIST_SIZE];
    let mut extra_glyphs = [0u32; EXTRA_GLYPH_LIST_SIZE];

    /* we first allocate the table */
    table.num_maps = 0;

    let mut maps: Vec<PsUniMap> =
        ft_new_array(num_glyphs as FtLong + EXTRA_GLYPH_LIST_SIZE as FtLong)?;
    let mut map = 0;

    for n in 0..num_glyphs {
        if let Some(gname) = get_glyph_name(n) {
            if !gname.is_empty() {
                ps_check_extra_glyph_name(
                    &gname,
                    n,
                    &mut extra_glyphs,
                    &mut extra_glyph_list_states,
                );
                let uni_char = ps_unicode_value(&gname);

                if base_glyph(uni_char) != 0 {
                    ps_check_extra_glyph_unicode(uni_char, &mut extra_glyph_list_states);
                    maps[map].unicode = uni_char;
                    maps[map].glyph_index = n;
                    map += 1;
                }
            }
        }
    }

    for n in 0..EXTRA_GLYPH_LIST_SIZE {
        if extra_glyph_list_states[n] == 1 {
            /* This glyph name has an additional representation. */
            /* Add it to the cmap.                               */

            maps[map].unicode = FT_EXTRA_GLYPH_UNICODES[n];
            maps[map].glyph_index = extra_glyphs[n];
            map += 1;
        }
    }

    /* now compress the table a bit */
    let count = map as FtUInt;

    if count == 0 {
        /* No unicode chars here! */
        table.maps = Vec::new();
        table.num_maps = count;
        return Err(FT_ERR_NO_UNICODE_GLYPH_NAME);
    }

    /* Reallocate if the number of used entries is much smaller. */
    maps.truncate(count as usize);

    /* Sort the table in increasing order of unicode values, */
    /* taking care of glyph variants.                        */
    /* (`ft_qsort`; glibc's is a merge sort, which is stable) */
    maps.sort_by(compare_uni_maps);

    table.maps = maps;
    table.num_maps = count;

    Ok(())
}

/// `ps_unicodes_char_index`
pub fn ps_unicodes_char_index(table: &PsUnicodesRec, unicode: FtUInt32) -> FtUInt {
    let maps = &table.maps;
    let mut result: Option<usize> = None;
    let mut min: usize = 0;
    let mut max: usize = table.num_maps as usize;
    let mut mid: usize = min + ((max - min) >> 1);

    /* Perform a binary search on the table. */
    while min < max {
        if maps[mid].unicode == unicode {
            result = Some(mid);
            break;
        }

        let base_glyph = base_glyph(maps[mid].unicode);

        if base_glyph == unicode {
            result = Some(mid); /* remember match but continue search for base glyph */
        }

        if base_glyph < unicode {
            min = mid + 1;
        } else {
            max = mid;
        }

        /* reasonable prediction in a continuous block */
        mid = mid.wrapping_add(unicode.wrapping_sub(base_glyph) as usize);
        if mid >= max || mid < min {
            mid = min + ((max - min) >> 1);
        }
    }

    match result {
        Some(r) => maps[r].glyph_index,
        None => 0,
    }
}

/// `ps_unicodes_char_next`
pub fn ps_unicodes_char_next(table: &PsUnicodesRec, unicode: &mut FtUInt32) -> FtUInt {
    let mut result: FtUInt = 0;
    let mut char_code = unicode.wrapping_add(1);

    'exit: {
        let mut min: FtUInt = 0;
        let mut max: FtUInt = table.num_maps;
        let mut mid: FtUInt = min + ((max - min) >> 1);

        while min < max {
            let map = &table.maps[mid as usize];

            if map.unicode == char_code {
                result = map.glyph_index;
                break 'exit;
            }

            let base_glyph = base_glyph(map.unicode);

            if base_glyph == char_code {
                result = map.glyph_index;
            }

            if base_glyph < char_code {
                min = mid + 1;
            } else {
                max = mid;
            }

            /* reasonable prediction in a continuous block */
            mid = mid.wrapping_add(char_code.wrapping_sub(base_glyph));
            if mid >= max || mid < min {
                mid = min + (max - min) / 2;
            }
        }

        if result != 0 {
            break 'exit; /* we have a variant glyph */
        }

        /* we didn't find it; check whether we have a map just above it */
        char_code = 0;

        if min < table.num_maps {
            let map = &table.maps[min as usize];
            result = map.glyph_index;
            char_code = base_glyph(map.unicode);
        }
    }

    /* Exit: */
    *unicode = char_code;

    result
}

/// `ps_get_macintosh_name`: the name (without its NUL)
pub fn ps_get_macintosh_name(mut name_index: FtUInt) -> &'static [u8] {
    if name_index >= FT_NUM_MAC_NAMES {
        name_index = 0;
    }

    c_str_at(FT_MAC_NAMES[name_index as usize] as usize)
}

/// `ps_get_standard_strings`: the string (without its NUL)
pub fn ps_get_standard_strings(sid: FtUInt) -> Option<&'static [u8]> {
    if sid >= FT_NUM_SID_NAMES {
        return None;
    }

    Some(c_str_at(FT_SID_NAMES[sid as usize] as usize))
}

/// The NUL-terminated name at `offset` in `ft_standard_glyph_names`.
fn c_str_at(offset: usize) -> &'static [u8] {
    let s = &FT_STANDARD_GLYPH_NAMES[offset..];
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    &s[..end]
}

/// `psnames_module_class`
pub static PSNAMES_MODULE_CLASS: FtModuleClass = FtModuleClass {
    module_flags: 0,          /* this is not a font driver, nor a renderer */
    module_name: "psnames",   /* driver name                         */
    module_version: 0x10000,  /* driver version                      */
    module_requires: 0x20000, /* driver requires FreeType 2 or above */

    module_interface: FtModuleInterface::None, /* (called directly) */
    module_init: None,                         /* FT_Module_Constructor module_init   */
    module_done: None,                         /* FT_Module_Destructor  module_done   */
    get_interface: None, /* (the `postscript-cmaps' service, called directly) */
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_names_table() {
        assert_eq!(FT_STANDARD_GLYPH_NAMES.len(), 3696);
        assert_eq!(ps_get_macintosh_name(0), b".notdef");
        assert_eq!(ps_get_macintosh_name(36), b"A");
        assert_eq!(ps_get_standard_strings(1), Some(&b"space"[..]));
    }

    #[test]
    fn unicode_values() {
        assert_eq!(ps_unicode_value(b"A"), 0x41);
        assert_eq!(ps_unicode_value(b"A.swash"), 0x41 | VARIANT_BIT);
        assert_eq!(ps_unicode_value(b"uni20AC"), 0x20AC);
        assert_eq!(ps_unicode_value(b"u1F600"), 0x1F600);
        assert_eq!(ps_unicode_value(b"Euro"), 0x20AC);
        assert_eq!(ps_unicode_value(b".notdef"), 0);
    }
}
