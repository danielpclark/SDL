// Rust translation of src/hb-ot-shaper-hebrew.cc from HarfBuzz (8.5.0, as
// SDL_ttf's external/harfbuzz pins it).
// Copyright © 2010,2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod

//! The Hebrew shaper.

use super::hb_buffer::HbBuffer;
use super::hb_common::*;
use super::hb_ot_layout::info_cc;
use super::hb_ot_shape::HbOtShapePlan;
use super::hb_ot_shape_normalize::*;
use super::hb_ot_shaper::*;
use super::hb_unicode::*;

/* Hebrew presentation-form shaping.
 * https://bugzilla.mozilla.org/show_bug.cgi?id=728866
 * Hebrew presentation forms with dagesh, for characters U+05D0..05EA;
 * Note that some letters do not have a dagesh presForm encoded.
 */
static S_DAGESH_FORMS: [HbCodepoint; 0x05EA - 0x05D0 + 1] = [
    0xFB30, /* ALEF */
    0xFB31, /* BET */
    0xFB32, /* GIMEL */
    0xFB33, /* DALET */
    0xFB34, /* HE */
    0xFB35, /* VAV */
    0xFB36, /* ZAYIN */
    0x0000, /* HET */
    0xFB38, /* TET */
    0xFB39, /* YOD */
    0xFB3A, /* FINAL KAF */
    0xFB3B, /* KAF */
    0xFB3C, /* LAMED */
    0x0000, /* FINAL MEM */
    0xFB3E, /* MEM */
    0x0000, /* FINAL NUN */
    0xFB40, /* NUN */
    0xFB41, /* SAMEKH */
    0x0000, /* AYIN */
    0xFB43, /* FINAL PE */
    0xFB44, /* PE */
    0x0000, /* FINAL TSADI */
    0xFB46, /* TSADI */
    0xFB47, /* QOF */
    0xFB48, /* RESH */
    0xFB49, /* SHIN */
    0xFB4A, /* TAV */
];

/// `compose_hebrew`
fn compose_hebrew(
    c: &HbOtShapeNormalizeContext,
    a: HbCodepoint,
    b: HbCodepoint,
    ab: &mut HbCodepoint,
) -> bool {
    let mut found = c.unicode.compose(a, b, ab);

    if !found && !c.plan.has_gpos_mark {
        /* Special-case Hebrew presentation forms that are excluded from
         * standard normalization, but wanted for old fonts. */
        match b {
            0x05B4 => {
                /* HIRIQ */
                if a == 0x05D9 {
                    /* YOD */
                    *ab = 0xFB1D;
                    found = true;
                }
            }
            0x05B7 => {
                /* PATAH */
                if a == 0x05F2 {
                    /* YIDDISH YOD YOD */
                    *ab = 0xFB1F;
                    found = true;
                } else if a == 0x05D0 {
                    /* ALEF */
                    *ab = 0xFB2E;
                    found = true;
                }
            }
            0x05B8 => {
                /* QAMATS */
                if a == 0x05D0 {
                    /* ALEF */
                    *ab = 0xFB2F;
                    found = true;
                }
            }
            0x05B9 => {
                /* HOLAM */
                if a == 0x05D5 {
                    /* VAV */
                    *ab = 0xFB4B;
                    found = true;
                }
            }
            0x05BC => {
                /* DAGESH */
                if (0x05D0..=0x05EA).contains(&a) {
                    *ab = S_DAGESH_FORMS[(a - 0x05D0) as usize];
                    found = *ab != 0;
                } else if a == 0xFB2A {
                    /* SHIN WITH SHIN DOT */
                    *ab = 0xFB2C;
                    found = true;
                } else if a == 0xFB2B {
                    /* SHIN WITH SIN DOT */
                    *ab = 0xFB2D;
                    found = true;
                }
            }
            0x05BF => {
                /* RAFE */
                match a {
                    0x05D1 => {
                        /* BET */
                        *ab = 0xFB4C;
                        found = true;
                    }
                    0x05DB => {
                        /* KAF */
                        *ab = 0xFB4D;
                        found = true;
                    }
                    0x05E4 => {
                        /* PE */
                        *ab = 0xFB4E;
                        found = true;
                    }
                    _ => {}
                }
            }
            0x05C1 => {
                /* SHIN DOT */
                if a == 0x05E9 {
                    /* SHIN */
                    *ab = 0xFB2A;
                    found = true;
                } else if a == 0xFB49 {
                    /* SHIN WITH DAGESH */
                    *ab = 0xFB2C;
                    found = true;
                }
            }
            0x05C2 => {
                /* SIN DOT */
                if a == 0x05E9 {
                    /* SHIN */
                    *ab = 0xFB2B;
                    found = true;
                } else if a == 0xFB49 {
                    /* SHIN WITH DAGESH */
                    *ab = 0xFB2D;
                    found = true;
                }
            }
            _ => {}
        }
    }

    found
}

/// `reorder_marks_hebrew`
fn reorder_marks_hebrew(_plan: &HbOtShapePlan, buffer: &mut HbBuffer, start: u32, end: u32) {
    let mut i = start + 2;
    while i < end {
        let c0 = info_cc(&buffer.info[(i - 2) as usize]);
        let c1 = info_cc(&buffer.info[(i - 1) as usize]);
        let c2 = info_cc(&buffer.info[i as usize]);

        if (c0 == HB_MODIFIED_COMBINING_CLASS_CCC17 || c0 == HB_MODIFIED_COMBINING_CLASS_CCC18) /* patach or qamats */ &&
            (c1 == HB_MODIFIED_COMBINING_CLASS_CCC10 || c1 == HB_MODIFIED_COMBINING_CLASS_CCC14) /* sheva or hiriq */ &&
            (c2 == HB_MODIFIED_COMBINING_CLASS_CCC22 || c2 == HB_UNICODE_COMBINING_CLASS_BELOW)
        /* meteg or below */
        {
            buffer.merge_clusters(i - 1, i + 1);
            buffer.info.swap((i - 1) as usize, i as usize);
            break;
        }
        i += 1;
    }
}

/// `_hb_ot_shaper_hebrew`
pub(crate) static _hb_ot_shaper_hebrew: HbOtShaper = HbOtShaper {
    collect_features: None,
    override_features: None,
    data_create: None,
    preprocess_text: None,
    postprocess_glyphs: None,
    decompose: None,
    compose: Some(compose_hebrew),
    setup_masks: None,
    reorder_marks: Some(reorder_marks_hebrew),
    gpos_tag: hb_tag(b'h', b'e', b'b', b'r'), /* gpos_tag. https://github.com/harfbuzz/harfbuzz/issues/347#issuecomment-267838368 */
    normalization_preference: HB_OT_SHAPE_NORMALIZATION_MODE_DEFAULT,
    zero_width_marks: HB_OT_SHAPE_ZERO_WIDTH_MARKS_BY_GDEF_LATE,
    fallback_position: true,
};
