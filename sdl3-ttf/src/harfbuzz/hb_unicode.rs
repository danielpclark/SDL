// Rust translation of src/hb-unicode.h, src/hb-unicode.hh and
// src/hb-unicode.cc from HarfBuzz (8.5.0, as SDL_ttf's external/harfbuzz
// pins it).
// Copyright © 2009  Red Hat, Inc.
// Copyright © 2011  Codethink Limited
// Copyright © 2010,2011,2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Red Hat Author(s): Behdad Esfahbod
// Codethink Author(s): Ryan Lortie
// Google Author(s): Behdad Esfahbod

//! Unicode functions.
//!
//! Translation notes: HarfBuzz lets the Unicode functions be replaced by
//! user callbacks (or GLib's and ICU's); SDL_ttf never does, and builds
//! HarfBuzz without GLib and ICU, so `hb_unicode_funcs_t` here always has
//! the functions of `hb-ucd.cc`, HarfBuzz's own Unicode Character Database
//! tables.

use super::hb_common::*;
use super::hb_ucd::*;
use super::hb_unicode_emoji_table::_hb_emoji_is_Extended_Pictographic;

/* hb_unicode_general_category_t */

/// `hb_unicode_general_category_t`: the Unicode General Category values.
pub type HbUnicodeGeneralCategory = u32;
/// Cc
pub const HB_UNICODE_GENERAL_CATEGORY_CONTROL: u32 = 0;
/// Cf
pub const HB_UNICODE_GENERAL_CATEGORY_FORMAT: u32 = 1;
/// Cn
pub const HB_UNICODE_GENERAL_CATEGORY_UNASSIGNED: u32 = 2;
/// Co
pub const HB_UNICODE_GENERAL_CATEGORY_PRIVATE_USE: u32 = 3;
/// Cs
pub const HB_UNICODE_GENERAL_CATEGORY_SURROGATE: u32 = 4;
/// Ll
pub const HB_UNICODE_GENERAL_CATEGORY_LOWERCASE_LETTER: u32 = 5;
/// Lm
pub const HB_UNICODE_GENERAL_CATEGORY_MODIFIER_LETTER: u32 = 6;
/// Lo
pub const HB_UNICODE_GENERAL_CATEGORY_OTHER_LETTER: u32 = 7;
/// Lt
pub const HB_UNICODE_GENERAL_CATEGORY_TITLECASE_LETTER: u32 = 8;
/// Lu
pub const HB_UNICODE_GENERAL_CATEGORY_UPPERCASE_LETTER: u32 = 9;
/// Mc
pub const HB_UNICODE_GENERAL_CATEGORY_SPACING_MARK: u32 = 10;
/// Me
pub const HB_UNICODE_GENERAL_CATEGORY_ENCLOSING_MARK: u32 = 11;
/// Mn
pub const HB_UNICODE_GENERAL_CATEGORY_NON_SPACING_MARK: u32 = 12;
/// Nd
pub const HB_UNICODE_GENERAL_CATEGORY_DECIMAL_NUMBER: u32 = 13;
/// Nl
pub const HB_UNICODE_GENERAL_CATEGORY_LETTER_NUMBER: u32 = 14;
/// No
pub const HB_UNICODE_GENERAL_CATEGORY_OTHER_NUMBER: u32 = 15;
/// Pc
pub const HB_UNICODE_GENERAL_CATEGORY_CONNECT_PUNCTUATION: u32 = 16;
/// Pd
pub const HB_UNICODE_GENERAL_CATEGORY_DASH_PUNCTUATION: u32 = 17;
/// Pe
pub const HB_UNICODE_GENERAL_CATEGORY_CLOSE_PUNCTUATION: u32 = 18;
/// Pf
pub const HB_UNICODE_GENERAL_CATEGORY_FINAL_PUNCTUATION: u32 = 19;
/// Pi
pub const HB_UNICODE_GENERAL_CATEGORY_INITIAL_PUNCTUATION: u32 = 20;
/// Po
pub const HB_UNICODE_GENERAL_CATEGORY_OTHER_PUNCTUATION: u32 = 21;
/// Ps
pub const HB_UNICODE_GENERAL_CATEGORY_OPEN_PUNCTUATION: u32 = 22;
/// Sc
pub const HB_UNICODE_GENERAL_CATEGORY_CURRENCY_SYMBOL: u32 = 23;
/// Sk
pub const HB_UNICODE_GENERAL_CATEGORY_MODIFIER_SYMBOL: u32 = 24;
/// Sm
pub const HB_UNICODE_GENERAL_CATEGORY_MATH_SYMBOL: u32 = 25;
/// So
pub const HB_UNICODE_GENERAL_CATEGORY_OTHER_SYMBOL: u32 = 26;
/// Zl
pub const HB_UNICODE_GENERAL_CATEGORY_LINE_SEPARATOR: u32 = 27;
/// Zp
pub const HB_UNICODE_GENERAL_CATEGORY_PARAGRAPH_SEPARATOR: u32 = 28;
/// Zs
pub const HB_UNICODE_GENERAL_CATEGORY_SPACE_SEPARATOR: u32 = 29;

/* hb_unicode_combining_class_t (the named ones) */

/// `HB_UNICODE_COMBINING_CLASS_NOT_REORDERED`
pub const HB_UNICODE_COMBINING_CLASS_NOT_REORDERED: u32 = 0;
/// `HB_UNICODE_COMBINING_CLASS_OVERLAY`
pub const HB_UNICODE_COMBINING_CLASS_OVERLAY: u32 = 1;
/// `HB_UNICODE_COMBINING_CLASS_NUKTA`
pub const HB_UNICODE_COMBINING_CLASS_NUKTA: u32 = 7;
/// `HB_UNICODE_COMBINING_CLASS_KANA_VOICING`
pub const HB_UNICODE_COMBINING_CLASS_KANA_VOICING: u32 = 8;
/// `HB_UNICODE_COMBINING_CLASS_VIRAMA`
pub const HB_UNICODE_COMBINING_CLASS_VIRAMA: u32 = 9;
/// `HB_UNICODE_COMBINING_CLASS_ATTACHED_BELOW_LEFT`
pub const HB_UNICODE_COMBINING_CLASS_ATTACHED_BELOW_LEFT: u32 = 200;
/// `HB_UNICODE_COMBINING_CLASS_ATTACHED_BELOW`
pub const HB_UNICODE_COMBINING_CLASS_ATTACHED_BELOW: u32 = 202;
/// `HB_UNICODE_COMBINING_CLASS_ATTACHED_ABOVE`
pub const HB_UNICODE_COMBINING_CLASS_ATTACHED_ABOVE: u32 = 214;
/// `HB_UNICODE_COMBINING_CLASS_ATTACHED_ABOVE_RIGHT`
pub const HB_UNICODE_COMBINING_CLASS_ATTACHED_ABOVE_RIGHT: u32 = 216;
/// `HB_UNICODE_COMBINING_CLASS_BELOW_LEFT`
pub const HB_UNICODE_COMBINING_CLASS_BELOW_LEFT: u32 = 218;
/// `HB_UNICODE_COMBINING_CLASS_BELOW`
pub const HB_UNICODE_COMBINING_CLASS_BELOW: u32 = 220;
/// `HB_UNICODE_COMBINING_CLASS_BELOW_RIGHT`
pub const HB_UNICODE_COMBINING_CLASS_BELOW_RIGHT: u32 = 222;
/// `HB_UNICODE_COMBINING_CLASS_LEFT`
pub const HB_UNICODE_COMBINING_CLASS_LEFT: u32 = 224;
/// `HB_UNICODE_COMBINING_CLASS_RIGHT`
pub const HB_UNICODE_COMBINING_CLASS_RIGHT: u32 = 226;
/// `HB_UNICODE_COMBINING_CLASS_ABOVE_LEFT`
pub const HB_UNICODE_COMBINING_CLASS_ABOVE_LEFT: u32 = 228;
/// `HB_UNICODE_COMBINING_CLASS_ABOVE`
pub const HB_UNICODE_COMBINING_CLASS_ABOVE: u32 = 230;
/// `HB_UNICODE_COMBINING_CLASS_ABOVE_RIGHT`
pub const HB_UNICODE_COMBINING_CLASS_ABOVE_RIGHT: u32 = 232;
/// `HB_UNICODE_COMBINING_CLASS_DOUBLE_BELOW`
pub const HB_UNICODE_COMBINING_CLASS_DOUBLE_BELOW: u32 = 233;
/// `HB_UNICODE_COMBINING_CLASS_DOUBLE_ABOVE`
pub const HB_UNICODE_COMBINING_CLASS_DOUBLE_ABOVE: u32 = 234;
/// `HB_UNICODE_COMBINING_CLASS_IOTA_SUBSCRIPT`
pub const HB_UNICODE_COMBINING_CLASS_IOTA_SUBSCRIPT: u32 = 240;
/// `HB_UNICODE_COMBINING_CLASS_INVALID`
pub const HB_UNICODE_COMBINING_CLASS_INVALID: u32 = 255;

/// `FLAG`
#[inline]
pub(crate) const fn flag(x: u32) -> u32 {
    1u32 << x
}
/// `FLAG_UNSAFE`
#[inline]
pub(crate) const fn flag_unsafe(x: u32) -> u32 {
    if x < 32 {
        1u32 << x
    } else {
        0
    }
}
/// `FLAG_RANGE`
#[inline]
pub(crate) const fn flag_range(x: u32, y: u32) -> u32 {
    flag(y + 1) - flag(x)
}
/// `FLAG64`
#[inline]
pub(crate) const fn flag64(x: u32) -> u64 {
    1u64 << x
}
/// `FLAG64_UNSAFE`
#[inline]
pub(crate) const fn flag64_unsafe(x: u32) -> u64 {
    if x < 64 {
        1u64 << x
    } else {
        0
    }
}

/// `space_t`: space estimates based on:
/// <https://unicode.org/charts/PDF/U2000.pdf> and
/// <https://docs.microsoft.com/en-us/typography/develop/character-design-standards/whitespace>
pub(crate) type SpaceType = u8;
pub(crate) const NOT_SPACE: SpaceType = 0;
pub(crate) const SPACE_EM: SpaceType = 1;
pub(crate) const SPACE_EM_2: SpaceType = 2;
pub(crate) const SPACE_EM_3: SpaceType = 3;
pub(crate) const SPACE_EM_4: SpaceType = 4;
pub(crate) const SPACE_EM_5: SpaceType = 5;
pub(crate) const SPACE_EM_6: SpaceType = 6;
pub(crate) const SPACE_EM_16: SpaceType = 16;
pub(crate) const SPACE_4_EM_18: SpaceType = 17; /* 4/18th of an EM! */
pub(crate) const SPACE: SpaceType = 18;
pub(crate) const SPACE_FIGURE: SpaceType = 19;
pub(crate) const SPACE_PUNCTUATION: SpaceType = 20;
pub(crate) const SPACE_NARROW: SpaceType = 21;

/// `hb_unicode_funcs_t`: the Unicode functions (always HarfBuzz's own
/// UCD functions; see the module documentation).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HbUnicodeFuncs;

impl HbUnicodeFuncs {
    /// `combining_class`
    #[inline]
    pub fn combining_class(&self, unicode: HbCodepoint) -> u32 {
        hb_ucd_combining_class(unicode)
    }
    /// `general_category`
    #[inline]
    pub fn general_category(&self, unicode: HbCodepoint) -> HbUnicodeGeneralCategory {
        hb_ucd_general_category(unicode)
    }
    /// `mirroring`
    #[inline]
    pub fn mirroring(&self, unicode: HbCodepoint) -> HbCodepoint {
        hb_ucd_mirroring(unicode)
    }
    /// `script`
    #[inline]
    pub fn script(&self, unicode: HbCodepoint) -> HbScript {
        hb_ucd_script(unicode)
    }

    /// `compose`
    pub fn compose(&self, a: HbCodepoint, b: HbCodepoint, ab: &mut HbCodepoint) -> bool {
        *ab = 0;
        if a == 0 || b == 0 {
            return false;
        }
        hb_ucd_compose(a, b, ab)
    }

    /// `decompose`
    pub fn decompose(&self, ab: HbCodepoint, a: &mut HbCodepoint, b: &mut HbCodepoint) -> bool {
        *a = ab;
        *b = 0;
        hb_ucd_decompose(ab, a, b)
    }

    /// `modified_combining_class`
    pub fn modified_combining_class(&self, u: HbCodepoint) -> u32 {
        /* Reorder SAKOT to ensure it comes after any tone marks. */
        if u == 0x1A60 {
            return 254;
        }
        /* Reorder PADMA to ensure it comes after any vowel marks. */
        if u == 0x0FC6 {
            return 254;
        }
        /* Reorder TSA -PHRU to reorder before U+0F74 */
        if u == 0x0F39 {
            return 127;
        }

        _HB_MODIFIED_COMBINING_CLASS[self.combining_class(u) as usize] as u32
    }

    /// `is_variation_selector`
    #[inline]
    pub fn is_variation_selector(unicode: HbCodepoint) -> bool {
        /* U+180B..180D, U+180F MONGOLIAN FREE VARIATION SELECTORs are handled in the
         * Arabic shaper.  No need to match them here. */
        (0xFE00..=0xFE0F).contains(&unicode) /* VARIATION SELECTOR-1..16 */
            || (0xE0100..=0xE01EF).contains(&unicode) /* VARIATION SELECTOR-17..256 */
    }

    /// `is_default_ignorable`: Default_Ignorable codepoints.
    ///
    /// Note: While U+115F, U+1160, U+3164 and U+FFA0 are Default_Ignorable,
    /// we do NOT want to hide them, as the way Uniscribe has implemented them
    /// is with regular spacing glyphs, and that's the way fonts are made to work.
    /// As such, we make exceptions for those four.
    /// Also ignoring U+1BCA0..1BCA3. <https://github.com/harfbuzz/harfbuzz/issues/503>
    ///
    /// (See hb-unicode.hh for the list of Unicode 14.0.)
    pub fn is_default_ignorable(ch: HbCodepoint) -> bool {
        let plane = ch >> 16;
        if plane == 0 {
            /* BMP */
            let page = ch >> 8;
            match page {
                0x00 => ch == 0x00AD,
                0x03 => ch == 0x034F,
                0x06 => ch == 0x061C,
                0x17 => (0x17B4..=0x17B5).contains(&ch),
                0x18 => (0x180B..=0x180E).contains(&ch),
                0x20 => {
                    (0x200B..=0x200F).contains(&ch)
                        || (0x202A..=0x202E).contains(&ch)
                        || (0x2060..=0x206F).contains(&ch)
                }
                0xFE => (0xFE00..=0xFE0F).contains(&ch) || ch == 0xFEFF,
                0xFF => (0xFFF0..=0xFFF8).contains(&ch),
                _ => false,
            }
        } else {
            /* Other planes */
            match plane {
                0x01 => (0x1D173..=0x1D17A).contains(&ch),
                0x0E => (0xE0000..=0xE0FFF).contains(&ch),
                _ => false,
            }
        }
    }

    /// `space_fallback_type`
    pub fn space_fallback_type(u: HbCodepoint) -> SpaceType {
        match u {
            /* All GC=Zs chars that can use a fallback. */
            0x0020 => SPACE,             /* U+0020 SPACE */
            0x00A0 => SPACE,             /* U+00A0 NO-BREAK SPACE */
            0x2000 => SPACE_EM_2,        /* U+2000 EN QUAD */
            0x2001 => SPACE_EM,          /* U+2001 EM QUAD */
            0x2002 => SPACE_EM_2,        /* U+2002 EN SPACE */
            0x2003 => SPACE_EM,          /* U+2003 EM SPACE */
            0x2004 => SPACE_EM_3,        /* U+2004 THREE-PER-EM SPACE */
            0x2005 => SPACE_EM_4,        /* U+2005 FOUR-PER-EM SPACE */
            0x2006 => SPACE_EM_6,        /* U+2006 SIX-PER-EM SPACE */
            0x2007 => SPACE_FIGURE,      /* U+2007 FIGURE SPACE */
            0x2008 => SPACE_PUNCTUATION, /* U+2008 PUNCTUATION SPACE */
            0x2009 => SPACE_EM_5,        /* U+2009 THIN SPACE */
            0x200A => SPACE_EM_16,       /* U+200A HAIR SPACE */
            0x202F => SPACE_NARROW,      /* U+202F NARROW NO-BREAK SPACE */
            0x205F => SPACE_4_EM_18,     /* U+205F MEDIUM MATHEMATICAL SPACE */
            0x3000 => SPACE_EM,          /* U+3000 IDEOGRAPHIC SPACE */
            _ => NOT_SPACE,              /* U+1680 OGHAM SPACE MARK */
        }
    }
}

/// `hb_unicode_funcs_get_default`
pub fn hb_unicode_funcs_get_default() -> HbUnicodeFuncs {
    HbUnicodeFuncs
}

/// `hb_unicode_script`: retrieves the script of a code point.
pub fn hb_unicode_script(ufuncs: &HbUnicodeFuncs, unicode: HbCodepoint) -> HbScript {
    ufuncs.script(unicode)
}

/// `HB_UNICODE_GENERAL_CATEGORY_IS_MARK`
#[inline]
pub(crate) fn hb_unicode_general_category_is_mark(gen_cat: u32) -> bool {
    flag_unsafe(gen_cat)
        & (flag(HB_UNICODE_GENERAL_CATEGORY_SPACING_MARK)
            | flag(HB_UNICODE_GENERAL_CATEGORY_ENCLOSING_MARK)
            | flag(HB_UNICODE_GENERAL_CATEGORY_NON_SPACING_MARK))
        != 0
}

/// `HB_UNICODE_GENERAL_CATEGORY_IS_LETTER`
#[inline]
pub(crate) fn hb_unicode_general_category_is_letter(gen_cat: u32) -> bool {
    flag_unsafe(gen_cat)
        & (flag(HB_UNICODE_GENERAL_CATEGORY_LOWERCASE_LETTER)
            | flag(HB_UNICODE_GENERAL_CATEGORY_MODIFIER_LETTER)
            | flag(HB_UNICODE_GENERAL_CATEGORY_OTHER_LETTER)
            | flag(HB_UNICODE_GENERAL_CATEGORY_TITLECASE_LETTER)
            | flag(HB_UNICODE_GENERAL_CATEGORY_UPPERCASE_LETTER))
        != 0
}

/* Modified combining marks */

/* Hebrew
 *
 * We permute the "fixed-position" classes 10-26 into the order
 * described in the SBL Hebrew manual:
 *
 * https://www.sbl-site.org/Fonts/SBLHebrewUserManual1.5x.pdf
 *
 * (as recommended by:
 *  https://forum.fontlab.com/archive-old-microsoft-volt-group/vista-and-diacritic-ordering/msg22823/)
 *
 * More details here:
 * https://bugzilla.mozilla.org/show_bug.cgi?id=662055
 */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC10: u32 = 22; /* sheva */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC11: u32 = 15; /* hataf segol */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC12: u32 = 16; /* hataf patah */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC13: u32 = 17; /* hataf qamats */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC14: u32 = 23; /* hiriq */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC15: u32 = 18; /* tsere */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC16: u32 = 19; /* segol */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC17: u32 = 20; /* patah */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC18: u32 = 21; /* qamats & qamats qatan */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC19: u32 = 14; /* holam & holam haser for vav*/
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC20: u32 = 24; /* qubuts */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC21: u32 = 12; /* dagesh */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC22: u32 = 25; /* meteg */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC23: u32 = 13; /* rafe */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC24: u32 = 10; /* shin dot */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC25: u32 = 11; /* sin dot */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC26: u32 = 26; /* point varika */

/*
 * Arabic
 *
 * Modify to move Shadda (ccc=33) before other marks.  See:
 * https://unicode.org/faq/normalization.html#8
 * https://unicode.org/faq/normalization.html#9
 */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC27: u32 = 28; /* fathatan */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC28: u32 = 29; /* dammatan */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC29: u32 = 30; /* kasratan */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC30: u32 = 31; /* fatha */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC31: u32 = 32; /* damma */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC32: u32 = 33; /* kasra */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC33: u32 = 27; /* shadda */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC34: u32 = 34; /* sukun */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC35: u32 = 35; /* superscript alef */

/* Syriac */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC36: u32 = 36; /* superscript alaph */

/* Telugu
 *
 * Modify Telugu length marks (ccc=84, ccc=91).
 * These are the only matras in the main Indic scripts range that have
 * a non-zero ccc.  That makes them reorder with the Halant (ccc=9).
 * Assign 4 and 5, which are otherwise unassigned.
 */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC84: u32 = 4; /* length mark */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC91: u32 = 5; /* ai length mark */

/* Thai
 *
 * Modify U+0E38 and U+0E39 (ccc=103) to be reordered before U+0E3A (ccc=9).
 * Assign 3, which is unassigned otherwise.
 * Uniscribe does this reordering too.
 */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC103: u32 = 3; /* sara u / sara uu */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC107: u32 = 107; /* mai * */

/* Lao */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC118: u32 = 118; /* sign u / sign uu */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC122: u32 = 122; /* mai * */

/* Tibetan
 *
 * In case of multiple vowel-signs, use u first (but after achung)
 * this allows Dzongkha multi-vowel shortcuts to render correctly
 */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC129: u32 = 129; /* sign aa */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC130: u32 = 132; /* sign i */
pub(crate) const HB_MODIFIED_COMBINING_CLASS_CCC132: u32 = 131; /* sign u */

/// `_hb_modified_combining_class`: permuted combining classes (see the
/// comments above).
#[rustfmt::skip]
pub(crate) static _HB_MODIFIED_COMBINING_CLASS: [u8; 256] = [
    0, /* HB_UNICODE_COMBINING_CLASS_NOT_REORDERED */
    1, /* HB_UNICODE_COMBINING_CLASS_OVERLAY */
    2, 3, 4, 5, 6,
    7, /* HB_UNICODE_COMBINING_CLASS_NUKTA */
    8, /* HB_UNICODE_COMBINING_CLASS_KANA_VOICING */
    9, /* HB_UNICODE_COMBINING_CLASS_VIRAMA */

    /* Hebrew */
    HB_MODIFIED_COMBINING_CLASS_CCC10 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC11 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC12 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC13 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC14 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC15 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC16 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC17 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC18 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC19 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC20 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC21 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC22 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC23 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC24 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC25 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC26 as u8,

    /* Arabic */
    HB_MODIFIED_COMBINING_CLASS_CCC27 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC28 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC29 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC30 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC31 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC32 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC33 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC34 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC35 as u8,

    /* Syriac */
    HB_MODIFIED_COMBINING_CLASS_CCC36 as u8,

    37, 38, 39,
    40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59,
    60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79,
    80, 81, 82, 83,

    /* Telugu */
    HB_MODIFIED_COMBINING_CLASS_CCC84 as u8,
    85, 86, 87, 88, 89, 90,
    HB_MODIFIED_COMBINING_CLASS_CCC91 as u8,
    92, 93, 94, 95, 96, 97, 98, 99, 100, 101, 102,

    /* Thai */
    HB_MODIFIED_COMBINING_CLASS_CCC103 as u8,
    104, 105, 106,
    HB_MODIFIED_COMBINING_CLASS_CCC107 as u8,
    108, 109, 110, 111, 112, 113, 114, 115, 116, 117,

    /* Lao */
    HB_MODIFIED_COMBINING_CLASS_CCC118 as u8,
    119, 120, 121,
    HB_MODIFIED_COMBINING_CLASS_CCC122 as u8,
    123, 124, 125, 126, 127, 128,

    /* Tibetan */
    HB_MODIFIED_COMBINING_CLASS_CCC129 as u8,
    HB_MODIFIED_COMBINING_CLASS_CCC130 as u8,
    131,
    HB_MODIFIED_COMBINING_CLASS_CCC132 as u8,
    133, 134, 135, 136, 137, 138, 139,

    140, 141, 142, 143, 144, 145, 146, 147, 148, 149,
    150, 151, 152, 153, 154, 155, 156, 157, 158, 159,
    160, 161, 162, 163, 164, 165, 166, 167, 168, 169,
    170, 171, 172, 173, 174, 175, 176, 177, 178, 179,
    180, 181, 182, 183, 184, 185, 186, 187, 188, 189,
    190, 191, 192, 193, 194, 195, 196, 197, 198, 199,

    200, /* HB_UNICODE_COMBINING_CLASS_ATTACHED_BELOW_LEFT */
    201,
    202, /* HB_UNICODE_COMBINING_CLASS_ATTACHED_BELOW */
    203, 204, 205, 206, 207, 208, 209, 210, 211, 212, 213,
    214, /* HB_UNICODE_COMBINING_CLASS_ATTACHED_ABOVE */
    215,
    216, /* HB_UNICODE_COMBINING_CLASS_ATTACHED_ABOVE_RIGHT */
    217,
    218, /* HB_UNICODE_COMBINING_CLASS_BELOW_LEFT */
    219,
    220, /* HB_UNICODE_COMBINING_CLASS_BELOW */
    221,
    222, /* HB_UNICODE_COMBINING_CLASS_BELOW_RIGHT */
    223,
    224, /* HB_UNICODE_COMBINING_CLASS_LEFT */
    225,
    226, /* HB_UNICODE_COMBINING_CLASS_RIGHT */
    227,
    228, /* HB_UNICODE_COMBINING_CLASS_ABOVE_LEFT */
    229,
    230, /* HB_UNICODE_COMBINING_CLASS_ABOVE */
    231,
    232, /* HB_UNICODE_COMBINING_CLASS_ABOVE_RIGHT */
    233, /* HB_UNICODE_COMBINING_CLASS_DOUBLE_BELOW */
    234, /* HB_UNICODE_COMBINING_CLASS_DOUBLE_ABOVE */
    235, 236, 237, 238, 239,
    240, /* HB_UNICODE_COMBINING_CLASS_IOTA_SUBSCRIPT */
    241, 242, 243, 244, 245, 246, 247, 248, 249, 250, 251, 252, 253, 254,
    255, /* HB_UNICODE_COMBINING_CLASS_INVALID */
];

/* Emoji */

/// `_hb_unicode_is_emoji_Extended_Pictographic`
#[allow(non_snake_case)]
pub(crate) fn _hb_unicode_is_emoji_Extended_Pictographic(cp: HbCodepoint) -> bool {
    _hb_emoji_is_Extended_Pictographic(cp) != 0
}
