// Rust translation of src/autofit/afranges.c and afranges.h from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it), with
// `AF_CONFIG_OPTION_CJK` and `AF_CONFIG_OPTION_INDIC` defined, as FreeType
// builds it.
// Copyright (C) 2013-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Auto-fitter Unicode script ranges.

#![allow(clippy::unusual_byte_groupings)]

use super::aftypes::AfScriptUniRangeRec;

/// `AF_UNIRANGE_REC`
const fn af_unirange_rec(a: u32, b: u32) -> AfScriptUniRangeRec {
    AfScriptUniRangeRec { first: a, last: b }
}

/*
 * The algorithm for assigning properties and styles to the `glyph_styles'
 * array is as follows (cf. the implementation in
 * `af_face_globals_compute_style_coverage').
 *
 *   Walk over all scripts (as listed in `afscript.h').
 *
 *   For a given script, walk over all styles (as listed in `afstyles.h').
 *   The order of styles is important and should be as follows.
 *
 *   - First come styles based on OpenType features (small caps, for
 *     example).  Since features rely on glyph indices, thus completely
 *     bypassing character codes, no properties are assigned.
 *
 *   - Next comes the default style, using the character ranges as defined
 *     below.  This also assigns properties.
 *
 *   Note that there also exist fallback scripts, mainly covering
 *   superscript and subscript glyphs of a script that are not present as
 *   OpenType features.  Fallback scripts are defined below, also
 *   assigning properties; they are applied after the corresponding
 *   script.
 *
 */

/* XXX Check base character ranges again:                        */
/*     Right now, they are quickly derived by visual inspection. */
/*     I can imagine that fine-tuning is necessary.              */

/* for the auto-hinter, a `non-base character' is something that should */
/* not be affected by blue zones, regardless of whether this is a       */
/* spacing or no-spacing glyph                                          */

/* the `af_xxxx_nonbase_uniranges' ranges must be strict subsets */
/* of the corresponding `af_xxxx_uniranges' ranges               */

#[rustfmt::skip]
pub static AF_ADLM_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x1E900, 0x1E95F), /* Adlam */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_ADLM_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x1D944, 0x1E94A),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_ARAB_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0600, 0x06FF), /* Arabic                                 */
    af_unirange_rec(0x0750, 0x07FF), /* Arabic Supplement                      */
    af_unirange_rec(0x08A0, 0x08FF), /* Arabic Extended-A                      */
    af_unirange_rec(0xFB50, 0xFDFF), /* Arabic Presentation Forms-A            */
    af_unirange_rec(0xFE70, 0xFEFF), /* Arabic Presentation Forms-B            */
    af_unirange_rec(0x1EE00, 0x1EEFF), /* Arabic Mathematical Alphabetic Symbols */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_ARAB_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0600, 0x0605),
    af_unirange_rec(0x0610, 0x061A),
    af_unirange_rec(0x064B, 0x065F),
    af_unirange_rec(0x0670, 0x0670),
    af_unirange_rec(0x06D6, 0x06DC),
    af_unirange_rec(0x06DF, 0x06E4),
    af_unirange_rec(0x06E7, 0x06E8),
    af_unirange_rec(0x06EA, 0x06ED),
    af_unirange_rec(0x08D4, 0x08E1),
    af_unirange_rec(0x08D3, 0x08FF),
    af_unirange_rec(0xFBB2, 0xFBC1),
    af_unirange_rec(0xFE70, 0xFE70),
    af_unirange_rec(0xFE72, 0xFE72),
    af_unirange_rec(0xFE74, 0xFE74),
    af_unirange_rec(0xFE76, 0xFE76),
    af_unirange_rec(0xFE78, 0xFE78),
    af_unirange_rec(0xFE7A, 0xFE7A),
    af_unirange_rec(0xFE7C, 0xFE7C),
    af_unirange_rec(0xFE7E, 0xFE7E),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_ARMN_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0530, 0x058F), /* Armenian                          */
    af_unirange_rec(0xFB13, 0xFB17), /* Alphab. Present. Forms (Armenian) */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_ARMN_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0559, 0x055F),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_AVST_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x10B00, 0x10B3F), /* Avestan */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_AVST_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x10B39, 0x10B3F),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_BAMU_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0xA6A0, 0xA6FF), /* Bamum */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_BAMU_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0xA6F0, 0xA6F1),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_BENG_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0980, 0x09FF), /* Bengali */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_BENG_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0981, 0x0981),
    af_unirange_rec(0x09BC, 0x09BC),
    af_unirange_rec(0x09C1, 0x09C4),
    af_unirange_rec(0x09CD, 0x09CD),
    af_unirange_rec(0x09E2, 0x09E3),
    af_unirange_rec(0x09FE, 0x09FE),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_BUHD_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x1740, 0x175F), /* Buhid */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_BUHD_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x1752, 0x1753),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_CAKM_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x11100, 0x1114F), /* Chakma */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_CAKM_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x11100, 0x11102),
    af_unirange_rec(0x11127, 0x11134),
    af_unirange_rec(0x11146, 0x11146),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_CANS_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x1400, 0x167F), /* Unified Canadian Aboriginal Syllabics          */
    af_unirange_rec(0x18B0, 0x18FF), /* Unified Canadian Aboriginal Syllabics Extended */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_CANS_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_CARI_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x102A0, 0x102DF), /* Carian */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_CARI_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_CHER_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x13A0, 0x13FF), /* Cherokee            */
    af_unirange_rec(0xAB70, 0xABBF), /* Cherokee Supplement */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_CHER_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_COPT_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x2C80, 0x2CFF), /* Coptic */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_COPT_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x2CEF, 0x2CF1),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_CPRT_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x10800, 0x1083F), /* Cypriot */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_CPRT_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_CYRL_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0400, 0x04FF), /* Cyrillic            */
    af_unirange_rec(0x0500, 0x052F), /* Cyrillic Supplement */
    af_unirange_rec(0x2DE0, 0x2DFF), /* Cyrillic Extended-A */
    af_unirange_rec(0xA640, 0xA69F), /* Cyrillic Extended-B */
    af_unirange_rec(0x1C80, 0x1C8F), /* Cyrillic Extended-C */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_CYRL_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0483, 0x0489),
    af_unirange_rec(0x2DE0, 0x2DFF),
    af_unirange_rec(0xA66F, 0xA67F),
    af_unirange_rec(0xA69E, 0xA69F),
    af_unirange_rec(0, 0),
];

/* There are some characters in the Devanagari Unicode block that are    */
/* generic to Indic scripts; we omit them so that their presence doesn't */
/* trigger Devanagari.                                                   */

#[rustfmt::skip]
pub static AF_DEVA_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0900, 0x093B), /* Devanagari          */
    /* omitting U+093C nukta */
    af_unirange_rec(0x093D, 0x0950), /* ... continued       */
    /* omitting U+0951 udatta, U+0952 anudatta */
    af_unirange_rec(0x0953, 0x0963), /* ... continued       */
    /* omitting U+0964 danda, U+0965 double danda */
    af_unirange_rec(0x0966, 0x097F), /* ... continued       */
    af_unirange_rec(0x20B9, 0x20B9), /* (new) Rupee sign    */
    af_unirange_rec(0xA8E0, 0xA8FF), /* Devanagari Extended */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_DEVA_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0900, 0x0902),
    af_unirange_rec(0x093A, 0x093A),
    af_unirange_rec(0x0941, 0x0948),
    af_unirange_rec(0x094D, 0x094D),
    af_unirange_rec(0x0953, 0x0957),
    af_unirange_rec(0x0962, 0x0963),
    af_unirange_rec(0xA8E0, 0xA8F1),
    af_unirange_rec(0xA8FF, 0xA8FF),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_DSRT_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x10400, 0x1044F), /* Deseret */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_DSRT_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_ETHI_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x1200, 0x137F), /* Ethiopic            */
    af_unirange_rec(0x1380, 0x139F), /* Ethiopic Supplement */
    af_unirange_rec(0x2D80, 0x2DDF), /* Ethiopic Extended   */
    af_unirange_rec(0xAB00, 0xAB2F), /* Ethiopic Extended-A */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_ETHI_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x135D, 0x135F),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_GEOR_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x10D0, 0x10FF), /* Georgian (Mkhedruli)          */
    af_unirange_rec(0x1C90, 0x1CBF), /* Georgian Extended (Mtavruli)  */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_GEOR_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_GEOK_UNIRANGES: &[AfScriptUniRangeRec] = &[
    /* Khutsuri */
    af_unirange_rec(0x10A0, 0x10CD), /* Georgian (Asomtavruli)         */
    af_unirange_rec(0x2D00, 0x2D2D), /* Georgian Supplement (Nuskhuri) */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_GEOK_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_GLAG_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x2C00, 0x2C5F), /* Glagolitic */
    af_unirange_rec(0x1E000, 0x1E02F), /* Glagolitic Supplement */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_GLAG_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x1E000, 0x1E02F),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_GOTH_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x10330, 0x1034F), /* Gothic */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_GOTH_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_GREK_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0370, 0x03FF), /* Greek and Coptic */
    af_unirange_rec(0x1F00, 0x1FFF), /* Greek Extended   */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_GREK_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x037A, 0x037A),
    af_unirange_rec(0x0384, 0x0385),
    af_unirange_rec(0x1FBD, 0x1FC1),
    af_unirange_rec(0x1FCD, 0x1FCF),
    af_unirange_rec(0x1FDD, 0x1FDF),
    af_unirange_rec(0x1FED, 0x1FEF),
    af_unirange_rec(0x1FFD, 0x1FFE),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_GUJR_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0A80, 0x0AFF), /* Gujarati */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_GUJR_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0A81, 0x0A82),
    af_unirange_rec(0x0ABC, 0x0ABC),
    af_unirange_rec(0x0AC1, 0x0AC8),
    af_unirange_rec(0x0ACD, 0x0ACD),
    af_unirange_rec(0x0AE2, 0x0AE3),
    af_unirange_rec(0x0AFA, 0x0AFF),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_GURU_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0A00, 0x0A7F), /* Gurmukhi */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_GURU_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0A01, 0x0A02),
    af_unirange_rec(0x0A3C, 0x0A3C),
    af_unirange_rec(0x0A41, 0x0A51),
    af_unirange_rec(0x0A70, 0x0A71),
    af_unirange_rec(0x0A75, 0x0A75),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_HEBR_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0590, 0x05FF), /* Hebrew                          */
    af_unirange_rec(0xFB1D, 0xFB4F), /* Alphab. Present. Forms (Hebrew) */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_HEBR_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0591, 0x05BF),
    af_unirange_rec(0x05C1, 0x05C2),
    af_unirange_rec(0x05C4, 0x05C5),
    af_unirange_rec(0x05C7, 0x05C7),
    af_unirange_rec(0xFB1E, 0xFB1E),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_KALI_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0xA900, 0xA92F), /* Kayah Li */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_KALI_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0xA926, 0xA92D),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_KNDA_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0C80, 0x0CFF), /* Kannada */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_KNDA_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0C81, 0x0C81),
    af_unirange_rec(0x0CBC, 0x0CBC),
    af_unirange_rec(0x0CBF, 0x0CBF),
    af_unirange_rec(0x0CC6, 0x0CC6),
    af_unirange_rec(0x0CCC, 0x0CCD),
    af_unirange_rec(0x0CE2, 0x0CE3),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_KHMR_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x1780, 0x17FF), /* Khmer */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_KHMR_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x17B7, 0x17BD),
    af_unirange_rec(0x17C6, 0x17C6),
    af_unirange_rec(0x17C9, 0x17D3),
    af_unirange_rec(0x17DD, 0x17DD),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_KHMS_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x19E0, 0x19FF), /* Khmer Symbols */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_KHMS_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_LAO_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0E80, 0x0EFF), /* Lao */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_LAO_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0EB1, 0x0EB1),
    af_unirange_rec(0x0EB4, 0x0EBC),
    af_unirange_rec(0x0EC8, 0x0ECD),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_LATN_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0020, 0x007F), /* Basic Latin (no control chars)         */
    af_unirange_rec(0x00A0, 0x00A9), /* Latin-1 Supplement (no control chars)  */
    af_unirange_rec(0x00AB, 0x00B1), /* ... continued                          */
    af_unirange_rec(0x00B4, 0x00B8), /* ... continued                          */
    af_unirange_rec(0x00BB, 0x00FF), /* ... continued                          */
    af_unirange_rec(0x0100, 0x017F), /* Latin Extended-A                       */
    af_unirange_rec(0x0180, 0x024F), /* Latin Extended-B                       */
    af_unirange_rec(0x0250, 0x02AF), /* IPA Extensions                         */
    af_unirange_rec(0x02B9, 0x02DF), /* Spacing Modifier Letters               */
    af_unirange_rec(0x02E5, 0x02FF), /* ... continued                          */
    af_unirange_rec(0x0300, 0x036F), /* Combining Diacritical Marks            */
    af_unirange_rec(0x1AB0, 0x1ABE), /* Combining Diacritical Marks Extended   */
    af_unirange_rec(0x1D00, 0x1D2B), /* Phonetic Extensions                    */
    af_unirange_rec(0x1D6B, 0x1D77), /* ... continued                          */
    af_unirange_rec(0x1D79, 0x1D7F), /* ... continued                          */
    af_unirange_rec(0x1D80, 0x1D9A), /* Phonetic Extensions Supplement         */
    af_unirange_rec(0x1DC0, 0x1DFF), /* Combining Diacritical Marks Supplement */
    af_unirange_rec(0x1E00, 0x1EFF), /* Latin Extended Additional              */
    af_unirange_rec(0x2000, 0x206F), /* General Punctuation                    */
    af_unirange_rec(0x20A0, 0x20B8), /* Currency Symbols ...                   */
    af_unirange_rec(0x20BA, 0x20CF), /* ... except new Rupee sign              */
    af_unirange_rec(0x2150, 0x218F), /* Number Forms                           */
    af_unirange_rec(0x2C60, 0x2C7B), /* Latin Extended-C                       */
    af_unirange_rec(0x2C7E, 0x2C7F), /* ... continued                          */
    af_unirange_rec(0x2E00, 0x2E7F), /* Supplemental Punctuation               */
    af_unirange_rec(0xA720, 0xA76F), /* Latin Extended-D                       */
    af_unirange_rec(0xA771, 0xA7F7), /* ... continued                          */
    af_unirange_rec(0xA7FA, 0xA7FF), /* ... continued                          */
    af_unirange_rec(0xAB30, 0xAB5B), /* Latin Extended-E                       */
    af_unirange_rec(0xAB60, 0xAB6F), /* ... continued                          */
    af_unirange_rec(0xFB00, 0xFB06), /* Alphab. Present. Forms (Latin Ligs)    */
    af_unirange_rec(0x1D400, 0x1D7FF), /* Mathematical Alphanumeric Symbols      */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_LATN_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x005E, 0x0060),
    af_unirange_rec(0x007E, 0x007E),
    af_unirange_rec(0x00A8, 0x00A9),
    af_unirange_rec(0x00AE, 0x00B0),
    af_unirange_rec(0x00B4, 0x00B4),
    af_unirange_rec(0x00B8, 0x00B8),
    af_unirange_rec(0x00BC, 0x00BE),
    af_unirange_rec(0x02B9, 0x02DF),
    af_unirange_rec(0x02E5, 0x02FF),
    af_unirange_rec(0x0300, 0x036F),
    af_unirange_rec(0x1AB0, 0x1ABE),
    af_unirange_rec(0x1DC0, 0x1DFF),
    af_unirange_rec(0x2017, 0x2017),
    af_unirange_rec(0x203E, 0x203E),
    af_unirange_rec(0xA788, 0xA788),
    af_unirange_rec(0xA7F8, 0xA7FA),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_LATB_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x1D62, 0x1D6A), /* some small subscript letters   */
    af_unirange_rec(0x2080, 0x209C), /* subscript digits and letters   */
    af_unirange_rec(0x2C7C, 0x2C7C), /* latin subscript small letter j */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_LATB_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_LATP_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x00AA, 0x00AA), /* feminine ordinal indicator          */
    af_unirange_rec(0x00B2, 0x00B3), /* superscript two and three           */
    af_unirange_rec(0x00B9, 0x00BA), /* superscript one, masc. ord. indic.  */
    af_unirange_rec(0x02B0, 0x02B8), /* some latin superscript mod. letters */
    af_unirange_rec(0x02E0, 0x02E4), /* some IPA modifier letters           */
    af_unirange_rec(0x1D2C, 0x1D61), /* latin superscript modifier letters  */
    af_unirange_rec(0x1D78, 0x1D78), /* modifier letter cyrillic en         */
    af_unirange_rec(0x1D9B, 0x1DBF), /* more modifier letters               */
    af_unirange_rec(0x2070, 0x207F), /* superscript digits and letters      */
    af_unirange_rec(0x2C7D, 0x2C7D), /* modifier letter capital v           */
    af_unirange_rec(0xA770, 0xA770), /* modifier letter us                  */
    af_unirange_rec(0xA7F8, 0xA7F9), /* more modifier letters               */
    af_unirange_rec(0xAB5C, 0xAB5F), /* more modifier letters               */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_LATP_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_LISU_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0xA4D0, 0xA4FF), /* Lisu */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_LISU_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_MLYM_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0D00, 0x0D7F), /* Malayalam */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_MLYM_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0D00, 0x0D01),
    af_unirange_rec(0x0D3B, 0x0D3C),
    af_unirange_rec(0x0D4D, 0x0D4E),
    af_unirange_rec(0x0D62, 0x0D63),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_MEDF_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x16E40, 0x16E9F), /* Medefaidrin */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_MEDF_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_MONG_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x1800, 0x18AF), /* Mongolian            */
    af_unirange_rec(0x11660, 0x1167F), /* Mongolian Supplement */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_MONG_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x1885, 0x1886),
    af_unirange_rec(0x18A9, 0x18A9),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_MYMR_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x1000, 0x109F), /* Myanmar            */
    af_unirange_rec(0xA9E0, 0xA9FF), /* Myanmar Extended-B */
    af_unirange_rec(0xAA60, 0xAA7F), /* Myanmar Extended-A */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_MYMR_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x102D, 0x1030),
    af_unirange_rec(0x1032, 0x1037),
    af_unirange_rec(0x103A, 0x103A),
    af_unirange_rec(0x103D, 0x103E),
    af_unirange_rec(0x1058, 0x1059),
    af_unirange_rec(0x105E, 0x1060),
    af_unirange_rec(0x1071, 0x1074),
    af_unirange_rec(0x1082, 0x1082),
    af_unirange_rec(0x1085, 0x1086),
    af_unirange_rec(0x108D, 0x108D),
    af_unirange_rec(0xA9E5, 0xA9E5),
    af_unirange_rec(0xAA7C, 0xAA7C),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_NKOO_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x07C0, 0x07FF), /* N'Ko */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_NKOO_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x07EB, 0x07F5),
    af_unirange_rec(0x07FD, 0x07FD),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_NONE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_NONE_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_OLCK_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x1C50, 0x1C7F), /* Ol Chiki */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_OLCK_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_ORKH_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x10C00, 0x10C4F), /* Old Turkic */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_ORKH_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_OSGE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x104B0, 0x104FF), /* Osage */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_OSGE_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_OSMA_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x10480, 0x104AF), /* Osmanya */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_OSMA_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_ROHG_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x10D00, 0x10D3F), /* Hanifi Rohingya */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_ROHG_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_SAUR_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0xA880, 0xA8DF), /* Saurashtra */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_SAUR_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0xA880, 0xA881),
    af_unirange_rec(0xA8B4, 0xA8C5),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_SHAW_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x10450, 0x1047F), /* Shavian */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_SHAW_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_SINH_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0D80, 0x0DFF), /* Sinhala */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_SINH_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0DCA, 0x0DCA),
    af_unirange_rec(0x0DD2, 0x0DD6),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_SUND_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x1B80, 0x1BBF), /* Sundanese            */
    af_unirange_rec(0x1CC0, 0x1CCF), /* Sundanese Supplement */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_SUND_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x1B80, 0x1B82),
    af_unirange_rec(0x1BA1, 0x1BAD),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_TAML_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0B80, 0x0BFF), /* Tamil */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_TAML_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0B82, 0x0B82),
    af_unirange_rec(0x0BC0, 0x0BC2),
    af_unirange_rec(0x0BCD, 0x0BCD),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_TAVT_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0xAA80, 0xAADF), /* Tai Viet */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_TAVT_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0xAAB0, 0xAAB0),
    af_unirange_rec(0xAAB2, 0xAAB4),
    af_unirange_rec(0xAAB7, 0xAAB8),
    af_unirange_rec(0xAABE, 0xAABF),
    af_unirange_rec(0xAAC1, 0xAAC1),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_TELU_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0C00, 0x0C7F), /* Telugu */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_TELU_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0C00, 0x0C00),
    af_unirange_rec(0x0C04, 0x0C04),
    af_unirange_rec(0x0C3E, 0x0C40),
    af_unirange_rec(0x0C46, 0x0C56),
    af_unirange_rec(0x0C62, 0x0C63),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_THAI_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0E00, 0x0E7F), /* Thai */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_THAI_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0E31, 0x0E31),
    af_unirange_rec(0x0E34, 0x0E3A),
    af_unirange_rec(0x0E47, 0x0E4E),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_TFNG_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x2D30, 0x2D7F), /* Tifinagh */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_TFNG_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_VAII_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0xA500, 0xA63F), /* Vai */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_VAII_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_LIMB_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x1900, 0x194F), /* Limbu */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_LIMB_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x1920, 0x1922),
    af_unirange_rec(0x1927, 0x1934),
    af_unirange_rec(0x1937, 0x193B),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_ORYA_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0B00, 0x0B7F), /* Oriya */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_ORYA_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0B01, 0x0B02),
    af_unirange_rec(0x0B3C, 0x0B3C),
    af_unirange_rec(0x0B3F, 0x0B3F),
    af_unirange_rec(0x0B41, 0x0B44),
    af_unirange_rec(0x0B4D, 0x0B56),
    af_unirange_rec(0x0B62, 0x0B63),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_SYLO_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0xA800, 0xA82F), /* Syloti Nagri */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_SYLO_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0xA802, 0xA802),
    af_unirange_rec(0xA806, 0xA806),
    af_unirange_rec(0xA80B, 0xA80B),
    af_unirange_rec(0xA825, 0xA826),
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_TIBT_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0F00, 0x0FFF), /* Tibetan */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_TIBT_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x0F18, 0x0F19),
    af_unirange_rec(0x0F35, 0x0F35),
    af_unirange_rec(0x0F37, 0x0F37),
    af_unirange_rec(0x0F39, 0x0F39),
    af_unirange_rec(0x0F3E, 0x0F3F),
    af_unirange_rec(0x0F71, 0x0F7E),
    af_unirange_rec(0x0F80, 0x0F84),
    af_unirange_rec(0x0F86, 0x0F87),
    af_unirange_rec(0x0F8D, 0x0FBC),
    af_unirange_rec(0, 0),
];

/* this corresponds to Unicode 6.0 */

#[rustfmt::skip]
pub static AF_HANI_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x1100, 0x11FF), /* Hangul Jamo                             */
    af_unirange_rec(0x2E80, 0x2EFF), /* CJK Radicals Supplement                 */
    af_unirange_rec(0x2F00, 0x2FDF), /* Kangxi Radicals                         */
    af_unirange_rec(0x2FF0, 0x2FFF), /* Ideographic Description Characters      */
    af_unirange_rec(0x3000, 0x303F), /* CJK Symbols and Punctuation             */
    af_unirange_rec(0x3040, 0x309F), /* Hiragana                                */
    af_unirange_rec(0x30A0, 0x30FF), /* Katakana                                */
    af_unirange_rec(0x3100, 0x312F), /* Bopomofo                                */
    af_unirange_rec(0x3130, 0x318F), /* Hangul Compatibility Jamo               */
    af_unirange_rec(0x3190, 0x319F), /* Kanbun                                  */
    af_unirange_rec(0x31A0, 0x31BF), /* Bopomofo Extended                       */
    af_unirange_rec(0x31C0, 0x31EF), /* CJK Strokes                             */
    af_unirange_rec(0x31F0, 0x31FF), /* Katakana Phonetic Extensions            */
    af_unirange_rec(0x3300, 0x33FF), /* CJK Compatibility                       */
    af_unirange_rec(0x3400, 0x4DBF), /* CJK Unified Ideographs Extension A      */
    af_unirange_rec(0x4DC0, 0x4DFF), /* Yijing Hexagram Symbols                 */
    af_unirange_rec(0x4E00, 0x9FFF), /* CJK Unified Ideographs                  */
    af_unirange_rec(0xA960, 0xA97F), /* Hangul Jamo Extended-A                  */
    af_unirange_rec(0xAC00, 0xD7AF), /* Hangul Syllables                        */
    af_unirange_rec(0xD7B0, 0xD7FF), /* Hangul Jamo Extended-B                  */
    af_unirange_rec(0xF900, 0xFAFF), /* CJK Compatibility Ideographs            */
    af_unirange_rec(0xFE10, 0xFE1F), /* Vertical forms                          */
    af_unirange_rec(0xFE30, 0xFE4F), /* CJK Compatibility Forms                 */
    af_unirange_rec(0xFF00, 0xFFEF), /* Halfwidth and Fullwidth Forms           */
    af_unirange_rec(0x1B000, 0x1B0FF), /* Kana Supplement                         */
    af_unirange_rec(0x1B100, 0x1B12F), /* Kana Extended-A                         */
    af_unirange_rec(0x1D300, 0x1D35F), /* Tai Xuan Hing Symbols                   */
    af_unirange_rec(0x20000, 0x2A6DF), /* CJK Unified Ideographs Extension B      */
    af_unirange_rec(0x2A700, 0x2B73F), /* CJK Unified Ideographs Extension C      */
    af_unirange_rec(0x2B740, 0x2B81F), /* CJK Unified Ideographs Extension D      */
    af_unirange_rec(0x2B820, 0x2CEAF), /* CJK Unified Ideographs Extension E      */
    af_unirange_rec(0x2CEB0, 0x2EBEF), /* CJK Unified Ideographs Extension F      */
    af_unirange_rec(0x2F800, 0x2FA1F), /* CJK Compatibility Ideographs Supplement */
    af_unirange_rec(0, 0),
];

#[rustfmt::skip]
pub static AF_HANI_NONBASE_UNIRANGES: &[AfScriptUniRangeRec] = &[
    af_unirange_rec(0x302A, 0x302F),
    af_unirange_rec(0x3190, 0x319F),
    af_unirange_rec(0, 0),
];

/* END */
