// Rust translation of src/hb-ot-shaper-syllabic.hh and
// src/hb-ot-shaper-syllabic.cc from HarfBuzz (8.5.0, as SDL_ttf's
// external/harfbuzz pins it).
// Copyright © 2021  Behdad Esfahbod.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).

//! What the syllabic shapers share: the dotted circles of broken
//! syllables.

use super::hb_buffer::*;
use super::hb_font::HbFont;
use super::hb_ot_layout::VAR_SYLLABLE;
use super::hb_ot_shape::HbOtShapePlan;

/// `hb_syllabic_insert_dotted_circles` (`repha_category` and
/// `dottedcircle_position` -1: none)
pub(crate) fn hb_syllabic_insert_dotted_circles(
    font: &mut HbFont,
    buffer: &mut HbBuffer,
    broken_syllable_type: u32,
    dottedcircle_category: u32,
    repha_category: i32,
    dottedcircle_position: i32,
) -> bool {
    if buffer.flags & HB_BUFFER_FLAG_DO_NOT_INSERT_DOTTED_CIRCLE != 0 {
        return false;
    }
    if buffer.scratch_flags & HB_BUFFER_SCRATCH_FLAG_HAS_BROKEN_SYLLABLE == 0 {
        return false;
    }

    let mut dottedcircle_glyph = 0;
    if !font.get_nominal_glyph(0x25CC, &mut dottedcircle_glyph, 0) {
        return false;
    }

    let mut dottedcircle = HbGlyphInfo {
        codepoint: 0x25CC,
        ..Default::default()
    };
    dottedcircle.set_complex_var_u8_category(dottedcircle_category as u8);
    if dottedcircle_position != -1 {
        dottedcircle.set_complex_var_u8_auxiliary(dottedcircle_position as u8);
    }
    dottedcircle.codepoint = dottedcircle_glyph;

    buffer.clear_output();

    buffer.idx = 0;
    let mut last_syllable = 0;
    while buffer.idx < buffer.len && buffer.successful {
        let syllable = buffer.cur(0).syllable() as u32;
        if last_syllable != syllable && (syllable & 0x0F) == broken_syllable_type {
            last_syllable = syllable;

            let mut ginfo = dottedcircle;
            ginfo.cluster = buffer.cur(0).cluster;
            ginfo.mask = buffer.cur(0).mask;
            ginfo.set_syllable(buffer.cur(0).syllable());

            /* Insert dottedcircle after possible Repha. */
            if repha_category != -1 {
                while buffer.idx < buffer.len
                    && buffer.successful
                    && last_syllable == buffer.cur(0).syllable() as u32
                    && buffer.cur(0).complex_var_u8_category() as i32 == repha_category
                {
                    let _ = buffer.next_glyph();
                }
            }

            let _ = buffer.output_info(ginfo);
        } else {
            let _ = buffer.next_glyph();
        }
    }
    let _ = buffer.sync();
    true
}

/// `hb_syllabic_clear_var` (a pause function)
pub(crate) fn hb_syllabic_clear_var(
    _plan: &HbOtShapePlan,
    _font: &mut HbFont,
    buffer: &mut HbBuffer,
) -> bool {
    buffer.deallocate_var(VAR_SYLLABLE.0, VAR_SYLLABLE.1);
    false
}
