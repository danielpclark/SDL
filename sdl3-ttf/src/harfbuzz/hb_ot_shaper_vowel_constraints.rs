// Rust translation of src/hb-ot-shaper-vowel-constraints.hh and
// src/hb-ot-shaper-vowel-constraints.cc from HarfBuzz (8.5.0, as SDL_ttf's
// external/harfbuzz pins it).
// Copyright © 2018  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).

//! The dotted circles of vowel sequences that look like another vowel.

use super::hb_buffer::*;
use super::hb_font::HbFont;
use super::hb_ot_layout::_hb_glyph_info_reset_continuation;
use super::hb_ot_shape::HbOtShapePlan;
use super::hb_ot_shaper_vowel_constraints_table::*;

/// `_output_dotted_circle`
fn _output_dotted_circle(buffer: &mut HbBuffer) {
    let _ = buffer.output_glyph(0x25CC);
    _hb_glyph_info_reset_continuation(buffer.prev_mut());
}

/// `_output_with_dotted_circle`
fn _output_with_dotted_circle(buffer: &mut HbBuffer) {
    _output_dotted_circle(buffer);
    let _ = buffer.next_glyph();
}

/// `_hb_preprocess_text_vowel_constraints`
pub(crate) fn _hb_preprocess_text_vowel_constraints(
    _plan: &HbOtShapePlan,
    buffer: &mut HbBuffer,
    _font: &mut HbFont,
) {
    if buffer.flags & HB_BUFFER_FLAG_DO_NOT_INSERT_DOTTED_CIRCLE != 0 {
        return;
    }

    /* UGLY UGLY UGLY business of adding dotted-circle in the middle of
     * vowel-sequences that look like another vowel.  Data for each script
     * collected from the USE script development spec.
     *
     * https://github.com/harfbuzz/harfbuzz/issues/1019
     */
    buffer.clear_output();
    let count = buffer.len;
    if let Some((_, rules)) = VOWEL_CONSTRAINTS
        .iter()
        .find(|(s, _)| *s == buffer.props.script)
    {
        buffer.idx = 0;
        while buffer.idx + 1 < count && buffer.successful {
            let mut matched = false;
            let cur = buffer.cur(0).codepoint;
            if let Some((_, rule)) = rules.iter().find(|(first, _)| *first == cur) {
                match *rule {
                    VowelRule::Second(seconds) => {
                        matched = seconds.contains(&buffer.cur(1).codepoint)
                    }
                    VowelRule::Third(second, third) => {
                        if second == buffer.cur(1).codepoint
                            && buffer.idx + 2 < count
                            && third == buffer.cur(2).codepoint
                        {
                            let _ = buffer.next_glyph();
                            matched = true;
                        }
                    }
                }
            }
            let _ = buffer.next_glyph();
            if matched {
                _output_with_dotted_circle(buffer);
            }
        }
    }
    let _ = buffer.sync();
}
