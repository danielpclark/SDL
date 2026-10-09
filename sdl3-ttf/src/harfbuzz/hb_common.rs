// Rust translation of src/hb-common.h and src/hb-common.cc from HarfBuzz
// (8.5.0, as SDL_ttf's external/harfbuzz pins it).
// Copyright © 2009,2010  Red Hat, Inc.
// Copyright © 2011,2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Red Hat Author(s): Behdad Esfahbod
// Google Author(s): Behdad Esfahbod

//! Common data types: tags, directions, languages, scripts, features and
//! the library version.
//!
//! Translation notes: `hb_language_t` is an interned, canonicalized
//! string (as in C, every distinct language is kept until exit), compared
//! by value, which is equivalent to C's pointer comparison of interned
//! strings. The feature and variation string parsers, and the
//! `HB_OPTIONS` environment variable (whose only option affects the
//! Uniscribe shaper, which is never selected by default) are not
//! translated: SDL_ttf does not use them.

use std::sync::Mutex;

/// `hb_bool_t`
pub type HbBool = bool;
/// `hb_codepoint_t`: a Unicode character or a glyph index
pub type HbCodepoint = u32;
/// `hb_position_t`: a position in font units (or 26.6, with hb-ft)
pub type HbPosition = i32;
/// `hb_mask_t`
pub type HbMask = u32;
/// `hb_tag_t`
pub type HbTag = u32;

/// `HB_TAG`
pub const fn hb_tag(c1: u8, c2: u8, c3: u8, c4: u8) -> HbTag {
    ((c1 as u32) << 24) | ((c2 as u32) << 16) | ((c3 as u32) << 8) | (c4 as u32)
}

/// `HB_TAG_NONE`
pub const HB_TAG_NONE: HbTag = hb_tag(0, 0, 0, 0);
/// `HB_TAG_MAX`
pub const HB_TAG_MAX: HbTag = hb_tag(0xff, 0xff, 0xff, 0xff);
/// `HB_TAG_MAX_SIGNED`
pub const HB_TAG_MAX_SIGNED: HbTag = hb_tag(0x7f, 0xff, 0xff, 0xff);

/// `HB_CODEPOINT_INVALID`
pub const HB_CODEPOINT_INVALID: HbCodepoint = u32::MAX;

/* hb_tag_t */

/// Converts a string into an `hb_tag_t`. Valid tags are four characters.
/// Shorter input strings will be padded with spaces. Longer input strings
/// will be truncated. (`hb_tag_from_string`, with `len` the slice length)
pub fn hb_tag_from_string(s: &[u8]) -> HbTag {
    let mut tag = [0u8; 4];
    let mut i = 0;

    if s.is_empty() || s[0] == 0 {
        return HB_TAG_NONE;
    }

    let len = s.len().min(4);
    while i < len && s[i] != 0 {
        tag[i] = s[i];
        i += 1;
    }
    while i < 4 {
        tag[i] = b' ';
        i += 1;
    }

    hb_tag(tag[0], tag[1], tag[2], tag[3])
}

/// Converts an `hb_tag_t` to a string (`hb_tag_to_string`).
pub fn hb_tag_to_string(tag: HbTag) -> [u8; 4] {
    [
        (tag >> 24) as u8,
        (tag >> 16) as u8,
        (tag >> 8) as u8,
        tag as u8,
    ]
}

/* hb_direction_t */

/// `hb_direction_t`: the direction of a text segment or buffer.
pub type HbDirection = u32;
/// Initial, unset direction.
pub const HB_DIRECTION_INVALID: HbDirection = 0;
/// Text is set horizontally from left to right.
pub const HB_DIRECTION_LTR: HbDirection = 4;
/// Text is set horizontally from right to left.
pub const HB_DIRECTION_RTL: HbDirection = 5;
/// Text is set vertically from top to bottom.
pub const HB_DIRECTION_TTB: HbDirection = 6;
/// Text is set vertically from bottom to top.
pub const HB_DIRECTION_BTT: HbDirection = 7;

/// `HB_DIRECTION_IS_VALID`
#[inline]
pub const fn hb_direction_is_valid(dir: HbDirection) -> bool {
    (dir & !3) == 4
}
/* Direction must be valid for the following */
/// `HB_DIRECTION_IS_HORIZONTAL`
#[inline]
pub const fn hb_direction_is_horizontal(dir: HbDirection) -> bool {
    (dir & !1) == 4
}
/// `HB_DIRECTION_IS_VERTICAL`
#[inline]
pub const fn hb_direction_is_vertical(dir: HbDirection) -> bool {
    (dir & !1) == 6
}
/// `HB_DIRECTION_IS_FORWARD`
#[inline]
pub const fn hb_direction_is_forward(dir: HbDirection) -> bool {
    (dir & !2) == 4
}
/// `HB_DIRECTION_IS_BACKWARD`
#[inline]
pub const fn hb_direction_is_backward(dir: HbDirection) -> bool {
    (dir & !2) == 5
}
/// `HB_DIRECTION_REVERSE`
#[inline]
pub const fn hb_direction_reverse(dir: HbDirection) -> HbDirection {
    dir ^ 1
}

static DIRECTION_STRINGS: [&str; 4] = ["ltr", "rtl", "ttb", "btt"];

/// Converts a string to an `hb_direction_t` (`hb_direction_from_string`).
pub fn hb_direction_from_string(s: &[u8]) -> HbDirection {
    if s.is_empty() || s[0] == 0 {
        return HB_DIRECTION_INVALID;
    }

    /* Lets match loosely: just match the first letter, such that
     * all of "ltr", "left-to-right", etc work!
     */
    let c = s[0].to_ascii_lowercase();
    for (i, d) in DIRECTION_STRINGS.iter().enumerate() {
        if c == d.as_bytes()[0] {
            return HB_DIRECTION_LTR + i as u32;
        }
    }

    HB_DIRECTION_INVALID
}

/// Converts an `hb_direction_t` to a string (`hb_direction_to_string`).
pub fn hb_direction_to_string(direction: HbDirection) -> &'static str {
    let i = direction.wrapping_sub(HB_DIRECTION_LTR) as usize;
    if i < DIRECTION_STRINGS.len() {
        return DIRECTION_STRINGS[i];
    }

    "invalid"
}

/* hb_language_t */

/// `hb_language_t`: a language (a canonicalized BCP 47 tag; `None` is
/// `HB_LANGUAGE_INVALID`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct HbLanguage(pub(crate) Option<&'static str>);

impl std::fmt::Debug for HbLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(s) => write!(f, "HbLanguage({s:?})"),
            None => write!(f, "HB_LANGUAGE_INVALID"),
        }
    }
}

/// `HB_LANGUAGE_INVALID`: an unset language.
pub const HB_LANGUAGE_INVALID: HbLanguage = HbLanguage(None);

#[rustfmt::skip]
static CANON_MAP: [u8; 256] = {
    let mut m = [0u8; 256];
    let rows: [[u8; 16]; 8] = [
        [0; 16],
        [0; 16],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, b'-', 0, 0],
        [b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', 0, 0, 0, 0, 0, 0],
        [0, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l', b'm', b'n', b'o'],
        [b'p', b'q', b'r', b's', b't', b'u', b'v', b'w', b'x', b'y', b'z', 0, 0, 0, 0, b'-'],
        [0, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l', b'm', b'n', b'o'],
        [b'p', b'q', b'r', b's', b't', b'u', b'v', b'w', b'x', b'y', b'z', 0, 0, 0, 0, 0],
    ];
    let mut r = 0;
    while r < 8 {
        let mut c = 0;
        while c < 16 {
            m[r * 16 + c] = rows[r][c];
            c += 1;
        }
        r += 1;
    }
    m
};

/// The interned languages (`langs`, C's lock-free list; the newest
/// first).
static LANGS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

/// `lang_equal`: whether the canonical `v1` matches the string `v2`.
fn lang_equal(v1: &str, v2: &[u8]) -> bool {
    let p1 = v1.as_bytes();
    let mut i = 0;
    loop {
        let c1 = p1.get(i).copied().unwrap_or(0);
        let c2 = CANON_MAP[v2.get(i).copied().unwrap_or(0) as usize];
        if c1 == 0 || c1 != c2 {
            return c1 == c2;
        }
        i += 1;
    }
}

/// `lang_find_or_insert`
fn lang_find_or_insert(key: &[u8]) -> HbLanguage {
    let mut langs = LANGS.lock().unwrap_or_else(|e| e.into_inner());

    for lang in langs.iter().rev() {
        if lang_equal(lang, key) {
            return HbLanguage(Some(lang));
        }
    }

    /* Not found; allocate one. */
    /* (the copy is canonicalized up to its first character that maps
     * to NUL, which ends the C string) */
    let mut s = String::new();
    for &c in key {
        let m = CANON_MAP[c as usize];
        if m == 0 {
            break;
        }
        s.push(m as char);
    }
    let lang: &'static str = Box::leak(s.into_boxed_str());
    langs.push(lang);
    HbLanguage(Some(lang))
}

/// Converts `str` representing a BCP 47 language tag to the corresponding
/// `hb_language_t` (`hb_language_from_string`; the slice is the string up
/// to `len` bytes or its NUL).
pub fn hb_language_from_string(s: &[u8]) -> HbLanguage {
    if s.is_empty() || s[0] == 0 {
        return HB_LANGUAGE_INVALID;
    }

    /* NUL-terminate it. */
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    /* (a length given in C is capped to the 63 bytes of its buffer; the
     * whole string, when given as NUL-terminated, is not: SDL_ttf passes
     * -1) */
    lang_find_or_insert(&s[..end])
}

/// Converts an `hb_language_t` to a string (`hb_language_to_string`).
pub fn hb_language_to_string(language: HbLanguage) -> Option<&'static str> {
    language.0
}

/// Fetches the default language from the current locale
/// (`hb_language_get_default`).
///
/// C reads `setlocale (LC_CTYPE, NULL)`, which is "C" in a program that
/// has not called `setlocale`; Rust programs do not set the locale, so
/// the default language here is the one of the "C" locale: "c".
pub fn hb_language_get_default() -> HbLanguage {
    static DEFAULT_LANGUAGE: Mutex<Option<HbLanguage>> = Mutex::new(None);
    let mut default_language = DEFAULT_LANGUAGE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(language) = *default_language {
        return language;
    }
    let language = hb_language_from_string(b"C");
    *default_language = Some(language);
    language
}

/// Check whether a second language tag is the same or a more specific
/// version of the provided language tag (`hb_language_matches`).
pub fn hb_language_matches(language: HbLanguage, specific: HbLanguage) -> bool {
    if language == specific {
        return true;
    }
    let (Some(l), Some(s)) = (language.0, specific.0) else {
        return false;
    };

    let ll = l.len();
    let sl = s.len();

    if ll > sl {
        return false;
    }

    s.as_bytes().starts_with(l.as_bytes()) && (sl == ll || s.as_bytes()[ll] == b'-')
}

/* hb_script_t */

/// `hb_script_t`: an ISO 15924 script tag.
pub type HbScript = u32;

// https://docs.google.com/spreadsheets/d/1Y90M0Ie3MUJ6UVCRDOypOtijlMDLNNyyLk36T6iMu0o
#[doc = "`HB_SCRIPT_COMMON`"]
pub const HB_SCRIPT_COMMON: HbScript = hb_tag(b'Z', b'y', b'y', b'y'); // 1.1
#[doc = "`HB_SCRIPT_INHERITED`"]
pub const HB_SCRIPT_INHERITED: HbScript = hb_tag(b'Z', b'i', b'n', b'h'); // 1.1
#[doc = "`HB_SCRIPT_UNKNOWN`"]
pub const HB_SCRIPT_UNKNOWN: HbScript = hb_tag(b'Z', b'z', b'z', b'z'); // 5.0

#[doc = "`HB_SCRIPT_ARABIC`"]
pub const HB_SCRIPT_ARABIC: HbScript = hb_tag(b'A', b'r', b'a', b'b'); // 1.1
#[doc = "`HB_SCRIPT_ARMENIAN`"]
pub const HB_SCRIPT_ARMENIAN: HbScript = hb_tag(b'A', b'r', b'm', b'n'); // 1.1
#[doc = "`HB_SCRIPT_BENGALI`"]
pub const HB_SCRIPT_BENGALI: HbScript = hb_tag(b'B', b'e', b'n', b'g'); // 1.1
#[doc = "`HB_SCRIPT_CYRILLIC`"]
pub const HB_SCRIPT_CYRILLIC: HbScript = hb_tag(b'C', b'y', b'r', b'l'); // 1.1
#[doc = "`HB_SCRIPT_DEVANAGARI`"]
pub const HB_SCRIPT_DEVANAGARI: HbScript = hb_tag(b'D', b'e', b'v', b'a'); // 1.1
#[doc = "`HB_SCRIPT_GEORGIAN`"]
pub const HB_SCRIPT_GEORGIAN: HbScript = hb_tag(b'G', b'e', b'o', b'r'); // 1.1
#[doc = "`HB_SCRIPT_GREEK`"]
pub const HB_SCRIPT_GREEK: HbScript = hb_tag(b'G', b'r', b'e', b'k'); // 1.1
#[doc = "`HB_SCRIPT_GUJARATI`"]
pub const HB_SCRIPT_GUJARATI: HbScript = hb_tag(b'G', b'u', b'j', b'r'); // 1.1
#[doc = "`HB_SCRIPT_GURMUKHI`"]
pub const HB_SCRIPT_GURMUKHI: HbScript = hb_tag(b'G', b'u', b'r', b'u'); // 1.1
#[doc = "`HB_SCRIPT_HANGUL`"]
pub const HB_SCRIPT_HANGUL: HbScript = hb_tag(b'H', b'a', b'n', b'g'); // 1.1
#[doc = "`HB_SCRIPT_HAN`"]
pub const HB_SCRIPT_HAN: HbScript = hb_tag(b'H', b'a', b'n', b'i'); // 1.1
#[doc = "`HB_SCRIPT_HEBREW`"]
pub const HB_SCRIPT_HEBREW: HbScript = hb_tag(b'H', b'e', b'b', b'r'); // 1.1
#[doc = "`HB_SCRIPT_HIRAGANA`"]
pub const HB_SCRIPT_HIRAGANA: HbScript = hb_tag(b'H', b'i', b'r', b'a'); // 1.1
#[doc = "`HB_SCRIPT_KANNADA`"]
pub const HB_SCRIPT_KANNADA: HbScript = hb_tag(b'K', b'n', b'd', b'a'); // 1.1
#[doc = "`HB_SCRIPT_KATAKANA`"]
pub const HB_SCRIPT_KATAKANA: HbScript = hb_tag(b'K', b'a', b'n', b'a'); // 1.1
#[doc = "`HB_SCRIPT_LAO`"]
pub const HB_SCRIPT_LAO: HbScript = hb_tag(b'L', b'a', b'o', b'o'); // 1.1
#[doc = "`HB_SCRIPT_LATIN`"]
pub const HB_SCRIPT_LATIN: HbScript = hb_tag(b'L', b'a', b't', b'n'); // 1.1
#[doc = "`HB_SCRIPT_MALAYALAM`"]
pub const HB_SCRIPT_MALAYALAM: HbScript = hb_tag(b'M', b'l', b'y', b'm'); // 1.1
#[doc = "`HB_SCRIPT_ORIYA`"]
pub const HB_SCRIPT_ORIYA: HbScript = hb_tag(b'O', b'r', b'y', b'a'); // 1.1
#[doc = "`HB_SCRIPT_TAMIL`"]
pub const HB_SCRIPT_TAMIL: HbScript = hb_tag(b'T', b'a', b'm', b'l'); // 1.1
#[doc = "`HB_SCRIPT_TELUGU`"]
pub const HB_SCRIPT_TELUGU: HbScript = hb_tag(b'T', b'e', b'l', b'u'); // 1.1
#[doc = "`HB_SCRIPT_THAI`"]
pub const HB_SCRIPT_THAI: HbScript = hb_tag(b'T', b'h', b'a', b'i'); // 1.1

#[doc = "`HB_SCRIPT_TIBETAN`"]
pub const HB_SCRIPT_TIBETAN: HbScript = hb_tag(b'T', b'i', b'b', b't'); // 2.0

#[doc = "`HB_SCRIPT_BOPOMOFO`"]
pub const HB_SCRIPT_BOPOMOFO: HbScript = hb_tag(b'B', b'o', b'p', b'o'); // 3.0
#[doc = "`HB_SCRIPT_BRAILLE`"]
pub const HB_SCRIPT_BRAILLE: HbScript = hb_tag(b'B', b'r', b'a', b'i'); // 3.0
#[doc = "`HB_SCRIPT_CANADIAN_SYLLABICS`"]
pub const HB_SCRIPT_CANADIAN_SYLLABICS: HbScript = hb_tag(b'C', b'a', b'n', b's'); // 3.0
#[doc = "`HB_SCRIPT_CHEROKEE`"]
pub const HB_SCRIPT_CHEROKEE: HbScript = hb_tag(b'C', b'h', b'e', b'r'); // 3.0
#[doc = "`HB_SCRIPT_ETHIOPIC`"]
pub const HB_SCRIPT_ETHIOPIC: HbScript = hb_tag(b'E', b't', b'h', b'i'); // 3.0
#[doc = "`HB_SCRIPT_KHMER`"]
pub const HB_SCRIPT_KHMER: HbScript = hb_tag(b'K', b'h', b'm', b'r'); // 3.0
#[doc = "`HB_SCRIPT_MONGOLIAN`"]
pub const HB_SCRIPT_MONGOLIAN: HbScript = hb_tag(b'M', b'o', b'n', b'g'); // 3.0
#[doc = "`HB_SCRIPT_MYANMAR`"]
pub const HB_SCRIPT_MYANMAR: HbScript = hb_tag(b'M', b'y', b'm', b'r'); // 3.0
#[doc = "`HB_SCRIPT_OGHAM`"]
pub const HB_SCRIPT_OGHAM: HbScript = hb_tag(b'O', b'g', b'a', b'm'); // 3.0
#[doc = "`HB_SCRIPT_RUNIC`"]
pub const HB_SCRIPT_RUNIC: HbScript = hb_tag(b'R', b'u', b'n', b'r'); // 3.0
#[doc = "`HB_SCRIPT_SINHALA`"]
pub const HB_SCRIPT_SINHALA: HbScript = hb_tag(b'S', b'i', b'n', b'h'); // 3.0
#[doc = "`HB_SCRIPT_SYRIAC`"]
pub const HB_SCRIPT_SYRIAC: HbScript = hb_tag(b'S', b'y', b'r', b'c'); // 3.0
#[doc = "`HB_SCRIPT_THAANA`"]
pub const HB_SCRIPT_THAANA: HbScript = hb_tag(b'T', b'h', b'a', b'a'); // 3.0
#[doc = "`HB_SCRIPT_YI`"]
pub const HB_SCRIPT_YI: HbScript = hb_tag(b'Y', b'i', b'i', b'i'); // 3.0

#[doc = "`HB_SCRIPT_DESERET`"]
pub const HB_SCRIPT_DESERET: HbScript = hb_tag(b'D', b's', b'r', b't'); // 3.1
#[doc = "`HB_SCRIPT_GOTHIC`"]
pub const HB_SCRIPT_GOTHIC: HbScript = hb_tag(b'G', b'o', b't', b'h'); // 3.1
#[doc = "`HB_SCRIPT_OLD_ITALIC`"]
pub const HB_SCRIPT_OLD_ITALIC: HbScript = hb_tag(b'I', b't', b'a', b'l'); // 3.1

#[doc = "`HB_SCRIPT_BUHID`"]
pub const HB_SCRIPT_BUHID: HbScript = hb_tag(b'B', b'u', b'h', b'd'); // 3.2
#[doc = "`HB_SCRIPT_HANUNOO`"]
pub const HB_SCRIPT_HANUNOO: HbScript = hb_tag(b'H', b'a', b'n', b'o'); // 3.2
#[doc = "`HB_SCRIPT_TAGALOG`"]
pub const HB_SCRIPT_TAGALOG: HbScript = hb_tag(b'T', b'g', b'l', b'g'); // 3.2
#[doc = "`HB_SCRIPT_TAGBANWA`"]
pub const HB_SCRIPT_TAGBANWA: HbScript = hb_tag(b'T', b'a', b'g', b'b'); // 3.2

#[doc = "`HB_SCRIPT_CYPRIOT`"]
pub const HB_SCRIPT_CYPRIOT: HbScript = hb_tag(b'C', b'p', b'r', b't'); // 4.0
#[doc = "`HB_SCRIPT_LIMBU`"]
pub const HB_SCRIPT_LIMBU: HbScript = hb_tag(b'L', b'i', b'm', b'b'); // 4.0
#[doc = "`HB_SCRIPT_LINEAR_B`"]
pub const HB_SCRIPT_LINEAR_B: HbScript = hb_tag(b'L', b'i', b'n', b'b'); // 4.0
#[doc = "`HB_SCRIPT_OSMANYA`"]
pub const HB_SCRIPT_OSMANYA: HbScript = hb_tag(b'O', b's', b'm', b'a'); // 4.0
#[doc = "`HB_SCRIPT_SHAVIAN`"]
pub const HB_SCRIPT_SHAVIAN: HbScript = hb_tag(b'S', b'h', b'a', b'w'); // 4.0
#[doc = "`HB_SCRIPT_TAI_LE`"]
pub const HB_SCRIPT_TAI_LE: HbScript = hb_tag(b'T', b'a', b'l', b'e'); // 4.0
#[doc = "`HB_SCRIPT_UGARITIC`"]
pub const HB_SCRIPT_UGARITIC: HbScript = hb_tag(b'U', b'g', b'a', b'r'); // 4.0

#[doc = "`HB_SCRIPT_BUGINESE`"]
pub const HB_SCRIPT_BUGINESE: HbScript = hb_tag(b'B', b'u', b'g', b'i'); // 4.1
#[doc = "`HB_SCRIPT_COPTIC`"]
pub const HB_SCRIPT_COPTIC: HbScript = hb_tag(b'C', b'o', b'p', b't'); // 4.1
#[doc = "`HB_SCRIPT_GLAGOLITIC`"]
pub const HB_SCRIPT_GLAGOLITIC: HbScript = hb_tag(b'G', b'l', b'a', b'g'); // 4.1
#[doc = "`HB_SCRIPT_KHAROSHTHI`"]
pub const HB_SCRIPT_KHAROSHTHI: HbScript = hb_tag(b'K', b'h', b'a', b'r'); // 4.1
#[doc = "`HB_SCRIPT_NEW_TAI_LUE`"]
pub const HB_SCRIPT_NEW_TAI_LUE: HbScript = hb_tag(b'T', b'a', b'l', b'u'); // 4.1
#[doc = "`HB_SCRIPT_OLD_PERSIAN`"]
pub const HB_SCRIPT_OLD_PERSIAN: HbScript = hb_tag(b'X', b'p', b'e', b'o'); // 4.1
#[doc = "`HB_SCRIPT_SYLOTI_NAGRI`"]
pub const HB_SCRIPT_SYLOTI_NAGRI: HbScript = hb_tag(b'S', b'y', b'l', b'o'); // 4.1
#[doc = "`HB_SCRIPT_TIFINAGH`"]
pub const HB_SCRIPT_TIFINAGH: HbScript = hb_tag(b'T', b'f', b'n', b'g'); // 4.1

#[doc = "`HB_SCRIPT_BALINESE`"]
pub const HB_SCRIPT_BALINESE: HbScript = hb_tag(b'B', b'a', b'l', b'i'); // 5.0
#[doc = "`HB_SCRIPT_CUNEIFORM`"]
pub const HB_SCRIPT_CUNEIFORM: HbScript = hb_tag(b'X', b's', b'u', b'x'); // 5.0
#[doc = "`HB_SCRIPT_NKO`"]
pub const HB_SCRIPT_NKO: HbScript = hb_tag(b'N', b'k', b'o', b'o'); // 5.0
#[doc = "`HB_SCRIPT_PHAGS_PA`"]
pub const HB_SCRIPT_PHAGS_PA: HbScript = hb_tag(b'P', b'h', b'a', b'g'); // 5.0
#[doc = "`HB_SCRIPT_PHOENICIAN`"]
pub const HB_SCRIPT_PHOENICIAN: HbScript = hb_tag(b'P', b'h', b'n', b'x'); // 5.0

#[doc = "`HB_SCRIPT_CARIAN`"]
pub const HB_SCRIPT_CARIAN: HbScript = hb_tag(b'C', b'a', b'r', b'i'); // 5.1
#[doc = "`HB_SCRIPT_CHAM`"]
pub const HB_SCRIPT_CHAM: HbScript = hb_tag(b'C', b'h', b'a', b'm'); // 5.1
#[doc = "`HB_SCRIPT_KAYAH_LI`"]
pub const HB_SCRIPT_KAYAH_LI: HbScript = hb_tag(b'K', b'a', b'l', b'i'); // 5.1
#[doc = "`HB_SCRIPT_LEPCHA`"]
pub const HB_SCRIPT_LEPCHA: HbScript = hb_tag(b'L', b'e', b'p', b'c'); // 5.1
#[doc = "`HB_SCRIPT_LYCIAN`"]
pub const HB_SCRIPT_LYCIAN: HbScript = hb_tag(b'L', b'y', b'c', b'i'); // 5.1
#[doc = "`HB_SCRIPT_LYDIAN`"]
pub const HB_SCRIPT_LYDIAN: HbScript = hb_tag(b'L', b'y', b'd', b'i'); // 5.1
#[doc = "`HB_SCRIPT_OL_CHIKI`"]
pub const HB_SCRIPT_OL_CHIKI: HbScript = hb_tag(b'O', b'l', b'c', b'k'); // 5.1
#[doc = "`HB_SCRIPT_REJANG`"]
pub const HB_SCRIPT_REJANG: HbScript = hb_tag(b'R', b'j', b'n', b'g'); // 5.1
#[doc = "`HB_SCRIPT_SAURASHTRA`"]
pub const HB_SCRIPT_SAURASHTRA: HbScript = hb_tag(b'S', b'a', b'u', b'r'); // 5.1
#[doc = "`HB_SCRIPT_SUNDANESE`"]
pub const HB_SCRIPT_SUNDANESE: HbScript = hb_tag(b'S', b'u', b'n', b'd'); // 5.1
#[doc = "`HB_SCRIPT_VAI`"]
pub const HB_SCRIPT_VAI: HbScript = hb_tag(b'V', b'a', b'i', b'i'); // 5.1

#[doc = "`HB_SCRIPT_AVESTAN`"]
pub const HB_SCRIPT_AVESTAN: HbScript = hb_tag(b'A', b'v', b's', b't'); // 5.2
#[doc = "`HB_SCRIPT_BAMUM`"]
pub const HB_SCRIPT_BAMUM: HbScript = hb_tag(b'B', b'a', b'm', b'u'); // 5.2
#[doc = "`HB_SCRIPT_EGYPTIAN_HIEROGLYPHS`"]
pub const HB_SCRIPT_EGYPTIAN_HIEROGLYPHS: HbScript = hb_tag(b'E', b'g', b'y', b'p'); // 5.2
#[doc = "`HB_SCRIPT_IMPERIAL_ARAMAIC`"]
pub const HB_SCRIPT_IMPERIAL_ARAMAIC: HbScript = hb_tag(b'A', b'r', b'm', b'i'); // 5.2
#[doc = "`HB_SCRIPT_INSCRIPTIONAL_PAHLAVI`"]
pub const HB_SCRIPT_INSCRIPTIONAL_PAHLAVI: HbScript = hb_tag(b'P', b'h', b'l', b'i'); // 5.2
#[doc = "`HB_SCRIPT_INSCRIPTIONAL_PARTHIAN`"]
pub const HB_SCRIPT_INSCRIPTIONAL_PARTHIAN: HbScript = hb_tag(b'P', b'r', b't', b'i'); // 5.2
#[doc = "`HB_SCRIPT_JAVANESE`"]
pub const HB_SCRIPT_JAVANESE: HbScript = hb_tag(b'J', b'a', b'v', b'a'); // 5.2
#[doc = "`HB_SCRIPT_KAITHI`"]
pub const HB_SCRIPT_KAITHI: HbScript = hb_tag(b'K', b't', b'h', b'i'); // 5.2
#[doc = "`HB_SCRIPT_LISU`"]
pub const HB_SCRIPT_LISU: HbScript = hb_tag(b'L', b'i', b's', b'u'); // 5.2
#[doc = "`HB_SCRIPT_MEETEI_MAYEK`"]
pub const HB_SCRIPT_MEETEI_MAYEK: HbScript = hb_tag(b'M', b't', b'e', b'i'); // 5.2
#[doc = "`HB_SCRIPT_OLD_SOUTH_ARABIAN`"]
pub const HB_SCRIPT_OLD_SOUTH_ARABIAN: HbScript = hb_tag(b'S', b'a', b'r', b'b'); // 5.2
#[doc = "`HB_SCRIPT_OLD_TURKIC`"]
pub const HB_SCRIPT_OLD_TURKIC: HbScript = hb_tag(b'O', b'r', b'k', b'h'); // 5.2
#[doc = "`HB_SCRIPT_SAMARITAN`"]
pub const HB_SCRIPT_SAMARITAN: HbScript = hb_tag(b'S', b'a', b'm', b'r'); // 5.2
#[doc = "`HB_SCRIPT_TAI_THAM`"]
pub const HB_SCRIPT_TAI_THAM: HbScript = hb_tag(b'L', b'a', b'n', b'a'); // 5.2
#[doc = "`HB_SCRIPT_TAI_VIET`"]
pub const HB_SCRIPT_TAI_VIET: HbScript = hb_tag(b'T', b'a', b'v', b't'); // 5.2

#[doc = "`HB_SCRIPT_BATAK`"]
pub const HB_SCRIPT_BATAK: HbScript = hb_tag(b'B', b'a', b't', b'k'); // 6.0
#[doc = "`HB_SCRIPT_BRAHMI`"]
pub const HB_SCRIPT_BRAHMI: HbScript = hb_tag(b'B', b'r', b'a', b'h'); // 6.0
#[doc = "`HB_SCRIPT_MANDAIC`"]
pub const HB_SCRIPT_MANDAIC: HbScript = hb_tag(b'M', b'a', b'n', b'd'); // 6.0

#[doc = "`HB_SCRIPT_CHAKMA`"]
pub const HB_SCRIPT_CHAKMA: HbScript = hb_tag(b'C', b'a', b'k', b'm'); // 6.1
#[doc = "`HB_SCRIPT_MEROITIC_CURSIVE`"]
pub const HB_SCRIPT_MEROITIC_CURSIVE: HbScript = hb_tag(b'M', b'e', b'r', b'c'); // 6.1
#[doc = "`HB_SCRIPT_MEROITIC_HIEROGLYPHS`"]
pub const HB_SCRIPT_MEROITIC_HIEROGLYPHS: HbScript = hb_tag(b'M', b'e', b'r', b'o'); // 6.1
#[doc = "`HB_SCRIPT_MIAO`"]
pub const HB_SCRIPT_MIAO: HbScript = hb_tag(b'P', b'l', b'r', b'd'); // 6.1
#[doc = "`HB_SCRIPT_SHARADA`"]
pub const HB_SCRIPT_SHARADA: HbScript = hb_tag(b'S', b'h', b'r', b'd'); // 6.1
#[doc = "`HB_SCRIPT_SORA_SOMPENG`"]
pub const HB_SCRIPT_SORA_SOMPENG: HbScript = hb_tag(b'S', b'o', b'r', b'a'); // 6.1
#[doc = "`HB_SCRIPT_TAKRI`"]
pub const HB_SCRIPT_TAKRI: HbScript = hb_tag(b'T', b'a', b'k', b'r'); // 6.1

// Since: 0.9.30
#[doc = "`HB_SCRIPT_BASSA_VAH`"]
pub const HB_SCRIPT_BASSA_VAH: HbScript = hb_tag(b'B', b'a', b's', b's'); // 7.0
#[doc = "`HB_SCRIPT_CAUCASIAN_ALBANIAN`"]
pub const HB_SCRIPT_CAUCASIAN_ALBANIAN: HbScript = hb_tag(b'A', b'g', b'h', b'b'); // 7.0
#[doc = "`HB_SCRIPT_DUPLOYAN`"]
pub const HB_SCRIPT_DUPLOYAN: HbScript = hb_tag(b'D', b'u', b'p', b'l'); // 7.0
#[doc = "`HB_SCRIPT_ELBASAN`"]
pub const HB_SCRIPT_ELBASAN: HbScript = hb_tag(b'E', b'l', b'b', b'a'); // 7.0
#[doc = "`HB_SCRIPT_GRANTHA`"]
pub const HB_SCRIPT_GRANTHA: HbScript = hb_tag(b'G', b'r', b'a', b'n'); // 7.0
#[doc = "`HB_SCRIPT_KHOJKI`"]
pub const HB_SCRIPT_KHOJKI: HbScript = hb_tag(b'K', b'h', b'o', b'j'); // 7.0
#[doc = "`HB_SCRIPT_KHUDAWADI`"]
pub const HB_SCRIPT_KHUDAWADI: HbScript = hb_tag(b'S', b'i', b'n', b'd'); // 7.0
#[doc = "`HB_SCRIPT_LINEAR_A`"]
pub const HB_SCRIPT_LINEAR_A: HbScript = hb_tag(b'L', b'i', b'n', b'a'); // 7.0
#[doc = "`HB_SCRIPT_MAHAJANI`"]
pub const HB_SCRIPT_MAHAJANI: HbScript = hb_tag(b'M', b'a', b'h', b'j'); // 7.0
#[doc = "`HB_SCRIPT_MANICHAEAN`"]
pub const HB_SCRIPT_MANICHAEAN: HbScript = hb_tag(b'M', b'a', b'n', b'i'); // 7.0
#[doc = "`HB_SCRIPT_MENDE_KIKAKUI`"]
pub const HB_SCRIPT_MENDE_KIKAKUI: HbScript = hb_tag(b'M', b'e', b'n', b'd'); // 7.0
#[doc = "`HB_SCRIPT_MODI`"]
pub const HB_SCRIPT_MODI: HbScript = hb_tag(b'M', b'o', b'd', b'i'); // 7.0
#[doc = "`HB_SCRIPT_MRO`"]
pub const HB_SCRIPT_MRO: HbScript = hb_tag(b'M', b'r', b'o', b'o'); // 7.0
#[doc = "`HB_SCRIPT_NABATAEAN`"]
pub const HB_SCRIPT_NABATAEAN: HbScript = hb_tag(b'N', b'b', b'a', b't'); // 7.0
#[doc = "`HB_SCRIPT_OLD_NORTH_ARABIAN`"]
pub const HB_SCRIPT_OLD_NORTH_ARABIAN: HbScript = hb_tag(b'N', b'a', b'r', b'b'); // 7.0
#[doc = "`HB_SCRIPT_OLD_PERMIC`"]
pub const HB_SCRIPT_OLD_PERMIC: HbScript = hb_tag(b'P', b'e', b'r', b'm'); // 7.0
#[doc = "`HB_SCRIPT_PAHAWH_HMONG`"]
pub const HB_SCRIPT_PAHAWH_HMONG: HbScript = hb_tag(b'H', b'm', b'n', b'g'); // 7.0
#[doc = "`HB_SCRIPT_PALMYRENE`"]
pub const HB_SCRIPT_PALMYRENE: HbScript = hb_tag(b'P', b'a', b'l', b'm'); // 7.0
#[doc = "`HB_SCRIPT_PAU_CIN_HAU`"]
pub const HB_SCRIPT_PAU_CIN_HAU: HbScript = hb_tag(b'P', b'a', b'u', b'c'); // 7.0
#[doc = "`HB_SCRIPT_PSALTER_PAHLAVI`"]
pub const HB_SCRIPT_PSALTER_PAHLAVI: HbScript = hb_tag(b'P', b'h', b'l', b'p'); // 7.0
#[doc = "`HB_SCRIPT_SIDDHAM`"]
pub const HB_SCRIPT_SIDDHAM: HbScript = hb_tag(b'S', b'i', b'd', b'd'); // 7.0
#[doc = "`HB_SCRIPT_TIRHUTA`"]
pub const HB_SCRIPT_TIRHUTA: HbScript = hb_tag(b'T', b'i', b'r', b'h'); // 7.0
#[doc = "`HB_SCRIPT_WARANG_CITI`"]
pub const HB_SCRIPT_WARANG_CITI: HbScript = hb_tag(b'W', b'a', b'r', b'a'); // 7.0

#[doc = "`HB_SCRIPT_AHOM`"]
pub const HB_SCRIPT_AHOM: HbScript = hb_tag(b'A', b'h', b'o', b'm'); // 8.0
#[doc = "`HB_SCRIPT_ANATOLIAN_HIEROGLYPHS`"]
pub const HB_SCRIPT_ANATOLIAN_HIEROGLYPHS: HbScript = hb_tag(b'H', b'l', b'u', b'w'); // 8.0
#[doc = "`HB_SCRIPT_HATRAN`"]
pub const HB_SCRIPT_HATRAN: HbScript = hb_tag(b'H', b'a', b't', b'r'); // 8.0
#[doc = "`HB_SCRIPT_MULTANI`"]
pub const HB_SCRIPT_MULTANI: HbScript = hb_tag(b'M', b'u', b'l', b't'); // 8.0
#[doc = "`HB_SCRIPT_OLD_HUNGARIAN`"]
pub const HB_SCRIPT_OLD_HUNGARIAN: HbScript = hb_tag(b'H', b'u', b'n', b'g'); // 8.0
#[doc = "`HB_SCRIPT_SIGNWRITING`"]
pub const HB_SCRIPT_SIGNWRITING: HbScript = hb_tag(b'S', b'g', b'n', b'w'); // 8.0

// Since 1.3.0
#[doc = "`HB_SCRIPT_ADLAM`"]
pub const HB_SCRIPT_ADLAM: HbScript = hb_tag(b'A', b'd', b'l', b'm'); // 9.0
#[doc = "`HB_SCRIPT_BHAIKSUKI`"]
pub const HB_SCRIPT_BHAIKSUKI: HbScript = hb_tag(b'B', b'h', b'k', b's'); // 9.0
#[doc = "`HB_SCRIPT_MARCHEN`"]
pub const HB_SCRIPT_MARCHEN: HbScript = hb_tag(b'M', b'a', b'r', b'c'); // 9.0
#[doc = "`HB_SCRIPT_OSAGE`"]
pub const HB_SCRIPT_OSAGE: HbScript = hb_tag(b'O', b's', b'g', b'e'); // 9.0
#[doc = "`HB_SCRIPT_TANGUT`"]
pub const HB_SCRIPT_TANGUT: HbScript = hb_tag(b'T', b'a', b'n', b'g'); // 9.0
#[doc = "`HB_SCRIPT_NEWA`"]
pub const HB_SCRIPT_NEWA: HbScript = hb_tag(b'N', b'e', b'w', b'a'); // 9.0

// Since 1.6.0
#[doc = "`HB_SCRIPT_MASARAM_GONDI`"]
pub const HB_SCRIPT_MASARAM_GONDI: HbScript = hb_tag(b'G', b'o', b'n', b'm'); // 10.0
#[doc = "`HB_SCRIPT_NUSHU`"]
pub const HB_SCRIPT_NUSHU: HbScript = hb_tag(b'N', b's', b'h', b'u'); // 10.0
#[doc = "`HB_SCRIPT_SOYOMBO`"]
pub const HB_SCRIPT_SOYOMBO: HbScript = hb_tag(b'S', b'o', b'y', b'o'); // 10.0
#[doc = "`HB_SCRIPT_ZANABAZAR_SQUARE`"]
pub const HB_SCRIPT_ZANABAZAR_SQUARE: HbScript = hb_tag(b'Z', b'a', b'n', b'b'); // 10.0

// Since 1.8.0
#[doc = "`HB_SCRIPT_DOGRA`"]
pub const HB_SCRIPT_DOGRA: HbScript = hb_tag(b'D', b'o', b'g', b'r'); // 11.0
#[doc = "`HB_SCRIPT_GUNJALA_GONDI`"]
pub const HB_SCRIPT_GUNJALA_GONDI: HbScript = hb_tag(b'G', b'o', b'n', b'g'); // 11.0
#[doc = "`HB_SCRIPT_HANIFI_ROHINGYA`"]
pub const HB_SCRIPT_HANIFI_ROHINGYA: HbScript = hb_tag(b'R', b'o', b'h', b'g'); // 11.0
#[doc = "`HB_SCRIPT_MAKASAR`"]
pub const HB_SCRIPT_MAKASAR: HbScript = hb_tag(b'M', b'a', b'k', b'a'); // 11.0
#[doc = "`HB_SCRIPT_MEDEFAIDRIN`"]
pub const HB_SCRIPT_MEDEFAIDRIN: HbScript = hb_tag(b'M', b'e', b'd', b'f'); // 11.0
#[doc = "`HB_SCRIPT_OLD_SOGDIAN`"]
pub const HB_SCRIPT_OLD_SOGDIAN: HbScript = hb_tag(b'S', b'o', b'g', b'o'); // 11.0
#[doc = "`HB_SCRIPT_SOGDIAN`"]
pub const HB_SCRIPT_SOGDIAN: HbScript = hb_tag(b'S', b'o', b'g', b'd'); // 11.0

// Since 2.4.0
#[doc = "`HB_SCRIPT_ELYMAIC`"]
pub const HB_SCRIPT_ELYMAIC: HbScript = hb_tag(b'E', b'l', b'y', b'm'); // 12.0
#[doc = "`HB_SCRIPT_NANDINAGARI`"]
pub const HB_SCRIPT_NANDINAGARI: HbScript = hb_tag(b'N', b'a', b'n', b'd'); // 12.0
#[doc = "`HB_SCRIPT_NYIAKENG_PUACHUE_HMONG`"]
pub const HB_SCRIPT_NYIAKENG_PUACHUE_HMONG: HbScript = hb_tag(b'H', b'm', b'n', b'p'); // 12.0
#[doc = "`HB_SCRIPT_WANCHO`"]
pub const HB_SCRIPT_WANCHO: HbScript = hb_tag(b'W', b'c', b'h', b'o'); // 12.0

// Since 2.6.7
#[doc = "`HB_SCRIPT_CHORASMIAN`"]
pub const HB_SCRIPT_CHORASMIAN: HbScript = hb_tag(b'C', b'h', b'r', b's'); // 13.0
#[doc = "`HB_SCRIPT_DIVES_AKURU`"]
pub const HB_SCRIPT_DIVES_AKURU: HbScript = hb_tag(b'D', b'i', b'a', b'k'); // 13.0
#[doc = "`HB_SCRIPT_KHITAN_SMALL_SCRIPT`"]
pub const HB_SCRIPT_KHITAN_SMALL_SCRIPT: HbScript = hb_tag(b'K', b'i', b't', b's'); // 13.0
#[doc = "`HB_SCRIPT_YEZIDI`"]
pub const HB_SCRIPT_YEZIDI: HbScript = hb_tag(b'Y', b'e', b'z', b'i'); // 13.0

// Since 3.0.0
#[doc = "`HB_SCRIPT_CYPRO_MINOAN`"]
pub const HB_SCRIPT_CYPRO_MINOAN: HbScript = hb_tag(b'C', b'p', b'm', b'n'); // 14.0
#[doc = "`HB_SCRIPT_OLD_UYGHUR`"]
pub const HB_SCRIPT_OLD_UYGHUR: HbScript = hb_tag(b'O', b'u', b'g', b'r'); // 14.0
#[doc = "`HB_SCRIPT_TANGSA`"]
pub const HB_SCRIPT_TANGSA: HbScript = hb_tag(b'T', b'n', b's', b'a'); // 14.0
#[doc = "`HB_SCRIPT_TOTO`"]
pub const HB_SCRIPT_TOTO: HbScript = hb_tag(b'T', b'o', b't', b'o'); // 14.0
#[doc = "`HB_SCRIPT_VITHKUQI`"]
pub const HB_SCRIPT_VITHKUQI: HbScript = hb_tag(b'V', b'i', b't', b'h'); // 14.0

// Since 3.4.0
#[doc = "`HB_SCRIPT_MATH`"]
pub const HB_SCRIPT_MATH: HbScript = hb_tag(b'Z', b'm', b't', b'h');

// Since 5.2.0
#[doc = "`HB_SCRIPT_KAWI`"]
pub const HB_SCRIPT_KAWI: HbScript = hb_tag(b'K', b'a', b'w', b'i'); // 15.0
#[doc = "`HB_SCRIPT_NAG_MUNDARI`"]
pub const HB_SCRIPT_NAG_MUNDARI: HbScript = hb_tag(b'N', b'a', b'g', b'm'); // 15.0

// No script set.

/// No script set.
pub const HB_SCRIPT_INVALID: HbScript = HB_TAG_NONE;

/// Converts an ISO 15924 script tag to a corresponding `hb_script_t`
/// (`hb_script_from_iso15924_tag`).
pub fn hb_script_from_iso15924_tag(mut tag: HbTag) -> HbScript {
    if tag == HB_TAG_NONE {
        return HB_SCRIPT_INVALID;
    }

    /* Be lenient, adjust case (one capital letter followed by three small letters) */
    tag = (tag & 0xDFDFDFDF) | 0x00202020;

    const QAAI: HbTag = hb_tag(b'Q', b'a', b'a', b'i');
    const QAAC: HbTag = hb_tag(b'Q', b'a', b'a', b'c');
    const ARAN: HbTag = hb_tag(b'A', b'r', b'a', b'n');
    const CYRS: HbTag = hb_tag(b'C', b'y', b'r', b's');
    const GEOK: HbTag = hb_tag(b'G', b'e', b'o', b'k');
    const HANS: HbTag = hb_tag(b'H', b'a', b'n', b's');
    const HANT: HbTag = hb_tag(b'H', b'a', b'n', b't');
    const JAMO: HbTag = hb_tag(b'J', b'a', b'm', b'o');
    const LATF: HbTag = hb_tag(b'L', b'a', b't', b'f');
    const LATG: HbTag = hb_tag(b'L', b'a', b't', b'g');
    const SYRE: HbTag = hb_tag(b'S', b'y', b'r', b'e');
    const SYRJ: HbTag = hb_tag(b'S', b'y', b'r', b'j');
    const SYRN: HbTag = hb_tag(b'S', b'y', b'r', b'n');
    match tag {
        /* These graduated from the 'Q' private-area codes, but
         * the old code is still aliased by Unicode, and the Qaai
         * one in use by ICU. */
        QAAI => return HB_SCRIPT_INHERITED,
        QAAC => return HB_SCRIPT_COPTIC,

        /* Script variants from https://unicode.org/iso15924/ */
        ARAN => return HB_SCRIPT_ARABIC,
        CYRS => return HB_SCRIPT_CYRILLIC,
        GEOK => return HB_SCRIPT_GEORGIAN,
        HANS => return HB_SCRIPT_HAN,
        HANT => return HB_SCRIPT_HAN,
        JAMO => return HB_SCRIPT_HANGUL,
        LATF => return HB_SCRIPT_LATIN,
        LATG => return HB_SCRIPT_LATIN,
        SYRE => return HB_SCRIPT_SYRIAC,
        SYRJ => return HB_SCRIPT_SYRIAC,
        SYRN => return HB_SCRIPT_SYRIAC,
        _ => {}
    }

    /* If it looks right, just use the tag as a script */
    if (tag & 0xE0E0E0E0) == 0x40606060 {
        return tag;
    }

    /* Otherwise, return unknown */
    HB_SCRIPT_UNKNOWN
}

/// Converts a string to an `hb_script_t` (`hb_script_from_string`).
pub fn hb_script_from_string(s: &[u8]) -> HbScript {
    hb_script_from_iso15924_tag(hb_tag_from_string(s))
}

/// Converts an `hb_script_t` to a corresponding ISO 15924 script tag
/// (`hb_script_to_iso15924_tag`).
pub fn hb_script_to_iso15924_tag(script: HbScript) -> HbTag {
    script
}

/// Fetches the `hb_direction_t` of a script when it is set horizontally
/// (`hb_script_get_horizontal_direction`).
pub fn hb_script_get_horizontal_direction(script: HbScript) -> HbDirection {
    /* https://docs.google.com/spreadsheets/d/1Y90M0Ie3MUJ6UVCRDOypOtijlMDLNNyyLk36T6iMu0o */
    match script {
        /* Unicode-1.1 additions */
        HB_SCRIPT_ARABIC
        | HB_SCRIPT_HEBREW

        /* Unicode-3.0 additions */
        | HB_SCRIPT_SYRIAC
        | HB_SCRIPT_THAANA

        /* Unicode-4.0 additions */
        | HB_SCRIPT_CYPRIOT

        /* Unicode-4.1 additions */
        | HB_SCRIPT_KHAROSHTHI

        /* Unicode-5.0 additions */
        | HB_SCRIPT_PHOENICIAN
        | HB_SCRIPT_NKO

        /* Unicode-5.1 additions */
        | HB_SCRIPT_LYDIAN

        /* Unicode-5.2 additions */
        | HB_SCRIPT_AVESTAN
        | HB_SCRIPT_IMPERIAL_ARAMAIC
        | HB_SCRIPT_INSCRIPTIONAL_PAHLAVI
        | HB_SCRIPT_INSCRIPTIONAL_PARTHIAN
        | HB_SCRIPT_OLD_SOUTH_ARABIAN
        | HB_SCRIPT_OLD_TURKIC
        | HB_SCRIPT_SAMARITAN

        /* Unicode-6.0 additions */
        | HB_SCRIPT_MANDAIC

        /* Unicode-6.1 additions */
        | HB_SCRIPT_MEROITIC_CURSIVE
        | HB_SCRIPT_MEROITIC_HIEROGLYPHS

        /* Unicode-7.0 additions */
        | HB_SCRIPT_MANICHAEAN
        | HB_SCRIPT_MENDE_KIKAKUI
        | HB_SCRIPT_NABATAEAN
        | HB_SCRIPT_OLD_NORTH_ARABIAN
        | HB_SCRIPT_PALMYRENE
        | HB_SCRIPT_PSALTER_PAHLAVI

        /* Unicode-8.0 additions */
        | HB_SCRIPT_HATRAN

        /* Unicode-9.0 additions */
        | HB_SCRIPT_ADLAM

        /* Unicode-11.0 additions */
        | HB_SCRIPT_HANIFI_ROHINGYA
        | HB_SCRIPT_OLD_SOGDIAN
        | HB_SCRIPT_SOGDIAN

        /* Unicode-12.0 additions */
        | HB_SCRIPT_ELYMAIC

        /* Unicode-13.0 additions */
        | HB_SCRIPT_CHORASMIAN
        | HB_SCRIPT_YEZIDI

        /* Unicode-14.0 additions */
        | HB_SCRIPT_OLD_UYGHUR => HB_DIRECTION_RTL,

        /* https://github.com/harfbuzz/harfbuzz/issues/1000 */
        HB_SCRIPT_OLD_HUNGARIAN
        | HB_SCRIPT_OLD_ITALIC
        | HB_SCRIPT_RUNIC
        | HB_SCRIPT_TIFINAGH => HB_DIRECTION_INVALID,

        _ => HB_DIRECTION_LTR,
    }
}

/* hb_version */

/// `HB_VERSION_MAJOR`
pub const HB_VERSION_MAJOR: u32 = 8;
/// `HB_VERSION_MINOR`
pub const HB_VERSION_MINOR: u32 = 5;
/// `HB_VERSION_MICRO`
pub const HB_VERSION_MICRO: u32 = 0;
/// `HB_VERSION_STRING`
pub const HB_VERSION_STRING: &str = "8.5.0";

/// Returns library version as three integer components (`hb_version`).
pub fn hb_version() -> (u32, u32, u32) {
    (HB_VERSION_MAJOR, HB_VERSION_MINOR, HB_VERSION_MICRO)
}

/// Returns library version as a string (`hb_version_string`).
pub fn hb_version_string() -> &'static str {
    HB_VERSION_STRING
}

/* hb_feature_t and hb_variation_t */

/// `HB_FEATURE_GLOBAL_START`
pub const HB_FEATURE_GLOBAL_START: u32 = 0;
/// `HB_FEATURE_GLOBAL_END`
pub const HB_FEATURE_GLOBAL_END: u32 = u32::MAX;

/// `hb_feature_t`: an OpenType feature to apply during shaping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HbFeature {
    /// The tag of the feature.
    pub tag: HbTag,
    /// The value of the feature (0 disables it, 1 enables it, larger
    /// values select alternates).
    pub value: u32,
    /// The cluster to start applying this feature setting (inclusive).
    pub start: u32,
    /// The cluster to end applying this feature setting (exclusive).
    pub end: u32,
}

/// `hb_variation_t`: a font variation axis setting.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HbVariation {
    /// The tag of the variation-axis name.
    pub tag: HbTag,
    /// The value of the variation axis.
    pub value: f32,
}

/// `hb_segment_properties_t`: the properties of a text segment
/// (direction, script and language).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HbSegmentProperties {
    /// the direction of the text
    pub direction: HbDirection,
    /// the script of the text
    pub script: HbScript,
    /// the language of the text
    pub language: HbLanguage,
}

/// `HB_SEGMENT_PROPERTIES_DEFAULT`
pub const HB_SEGMENT_PROPERTIES_DEFAULT: HbSegmentProperties = HbSegmentProperties {
    direction: HB_DIRECTION_INVALID,
    script: HB_SCRIPT_INVALID,
    language: HB_LANGUAGE_INVALID,
};

/// `hb_segment_properties_equal`
pub fn hb_segment_properties_equal(a: &HbSegmentProperties, b: &HbSegmentProperties) -> bool {
    a.direction == b.direction && a.script == b.script && a.language == b.language
}

/// `hb_segment_properties_overlay`
pub fn hb_segment_properties_overlay(p: &mut HbSegmentProperties, src: &HbSegmentProperties) {
    if p.direction == HB_DIRECTION_INVALID {
        p.direction = src.direction;
    }

    if p.direction != src.direction {
        return;
    }

    if p.script == HB_SCRIPT_INVALID {
        p.script = src.script;
    }

    if p.script != src.script {
        return;
    }

    if p.language == HB_LANGUAGE_INVALID {
        p.language = src.language;
    }
}
