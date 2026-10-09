// Rust translation of src/hb-ot-shaper-hangul.cc from HarfBuzz (8.5.0, as
// SDL_ttf's external/harfbuzz pins it).
// Copyright © 2013  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod

//! The Hangul shaper.

use super::hb_buffer::*;
use super::hb_common::*;
use super::hb_font::HbFont;
use super::hb_ot_layout::*;
use super::hb_ot_map::F_NONE;
use super::hb_ot_shape::{HbOtShapePlan, HbOtShapePlanner};
use super::hb_ot_shape_normalize::*;
use super::hb_ot_shaper::*;

/* Hangul shaper */

/* Same order as the feature array below */
const _JMO: u8 = 0;

const LJMO: u8 = 1;
const VJMO: u8 = 2;
const TJMO: u8 = 3;

const FIRST_HANGUL_FEATURE: usize = LJMO as usize;
const HANGUL_FEATURE_COUNT: usize = TJMO as usize + 1;

static HANGUL_FEATURES: [HbTag; HANGUL_FEATURE_COUNT] = [
    HB_TAG_NONE,
    hb_tag(b'l', b'j', b'm', b'o'),
    hb_tag(b'v', b'j', b'm', b'o'),
    hb_tag(b't', b'j', b'm', b'o'),
];

/// `collect_features_hangul`
fn collect_features_hangul(plan: &mut HbOtShapePlanner) {
    let map = &mut plan.map;

    for &feature in &HANGUL_FEATURES[FIRST_HANGUL_FEATURE..HANGUL_FEATURE_COUNT] {
        map.add_feature(feature, F_NONE, 1);
    }
}

/// `override_features_hangul`
fn override_features_hangul(plan: &mut HbOtShapePlanner) {
    /* Uniscribe does not apply 'calt' for Hangul, and certain fonts
     * (Noto Sans CJK, Source Sans Han, etc) apply all of jamo lookups
     * in calt, which is not desirable. */
    plan.map.disable_feature(hb_tag(b'c', b'a', b'l', b't'));
}

/// `hangul_shape_plan_t`
#[derive(Debug)]
pub(crate) struct HangulShapePlan {
    mask_array: [HbMask; HANGUL_FEATURE_COUNT],
}

/// `data_create_hangul`
fn data_create_hangul(plan: &HbOtShapePlan) -> Option<ShaperData> {
    let mut hangul_plan = HangulShapePlan {
        mask_array: [0; HANGUL_FEATURE_COUNT],
    };

    for i in 0..HANGUL_FEATURE_COUNT {
        hangul_plan.mask_array[i] = plan.map.get_1_mask(HANGUL_FEATURES[i]);
    }

    Some(ShaperData::Hangul(Box::new(hangul_plan)))
}

/* Constants for algorithmic hangul syllable [de]composition. */
const L_BASE: u32 = 0x1100;
const V_BASE: u32 = 0x1161;
const T_BASE: u32 = 0x11A7;
const L_COUNT: u32 = 19;
const V_COUNT: u32 = 21;
const T_COUNT: u32 = 28;
const S_BASE: u32 = 0xAC00;
const N_COUNT: u32 = V_COUNT * T_COUNT;
const S_COUNT: u32 = L_COUNT * N_COUNT;

#[inline]
fn is_combining_l(u: HbCodepoint) -> bool {
    (L_BASE..=L_BASE + L_COUNT - 1).contains(&u)
}
#[inline]
fn is_combining_v(u: HbCodepoint) -> bool {
    (V_BASE..=V_BASE + V_COUNT - 1).contains(&u)
}
#[inline]
fn is_combining_t(u: HbCodepoint) -> bool {
    (T_BASE + 1..=T_BASE + T_COUNT - 1).contains(&u)
}
#[inline]
fn is_combined_s(u: HbCodepoint) -> bool {
    (S_BASE..=S_BASE + S_COUNT - 1).contains(&u)
}

#[inline]
fn is_l(u: HbCodepoint) -> bool {
    (0x1100..=0x115F).contains(&u) || (0xA960..=0xA97C).contains(&u)
}
#[inline]
fn is_v(u: HbCodepoint) -> bool {
    (0x1160..=0x11A7).contains(&u) || (0xD7B0..=0xD7C6).contains(&u)
}
#[inline]
fn is_t(u: HbCodepoint) -> bool {
    (0x11A8..=0x11FF).contains(&u) || (0xD7CB..=0xD7FB).contains(&u)
}

#[inline]
fn is_hangul_tone(u: HbCodepoint) -> bool {
    (0x302E..=0x302F).contains(&u)
}

/* buffer var allocations */
/* hangul_shaping_feature() = ot_shaper_var_u8_auxiliary(): hangul jamo shaping feature */
#[inline]
fn set_hangul_shaping_feature(info: &mut HbGlyphInfo, v: u8) {
    info.set_complex_var_u8_auxiliary(v);
}

/// `is_zero_width_char`
fn is_zero_width_char(font: &mut HbFont, unicode: HbCodepoint) -> bool {
    let mut glyph = 0;
    font.get_nominal_glyph(unicode, &mut glyph, 0) && font.get_glyph_h_advance(glyph) == 0
}

/// `preprocess_text_hangul`
fn preprocess_text_hangul(_plan: &HbOtShapePlan, buffer: &mut HbBuffer, font: &mut HbFont) {
    buffer.allocate_var(VAR_COMPLEX_AUXILIARY.0, VAR_COMPLEX_AUXILIARY.1);

    /* Hangul syllables come in two shapes: LV, and LVT.  Of those:
     *
     *   - LV can be precomposed, or decomposed.  Lets call those
     *     <LV> and <L,V>,
     *   - LVT can be fully precomposed, partially precomposed, or
     *     fully decomposed.  Ie. <LVT>, <LV,T>, or <L,V,T>.
     *
     * The composition / decomposition is mechanical.  However, not
     * all <L,V> sequences compose, and not all <LV,T> sequences
     * compose.
     *
     * Here are the specifics:
     *
     *   - <L>: U+1100..115F, U+A960..A97F
     *   - <V>: U+1160..11A7, U+D7B0..D7C7
     *   - <T>: U+11A8..11FF, U+D7CB..D7FB
     *
     *   - Only the <L,V> sequences for some of the U+11xx ranges combine.
     *   - Only <LV,T> sequences for some of the Ts in U+11xx range combine.
     *
     * Here is what we want to accomplish in this shaper:
     *
     *   - If the whole syllable can be precomposed, do that,
     *   - Otherwise, fully decompose and apply ljmo/vjmo/tjmo features.
     *   - If a valid syllable is followed by a Hangul tone mark, reorder the tone
     *     mark to precede the whole syllable - unless it is a zero-width glyph, in
     *     which case we leave it untouched, assuming it's designed to overstrike.
     *
     * That is, of the different possible syllables:
     *
     *   <L>
     *   <L,V>
     *   <L,V,T>
     *   <LV>
     *   <LVT>
     *   <LV, T>
     *
     * - <L> needs no work.
     *
     * - <LV> and <LVT> can stay the way they are if the font supports them, otherwise we
     *   should fully decompose them if font supports.
     *
     * - <L,V> and <L,V,T> we should compose if the whole thing can be composed.
     *
     * - <LV,T> we should compose if the whole thing can be composed, otherwise we should
     *   decompose.
     */

    buffer.clear_output();
    /* Extent of most recently seen syllable;
     * valid only if start < end
     */
    let mut start: u32 = 0;
    let mut end: u32 = 0;
    let count = buffer.len;

    buffer.idx = 0;
    while buffer.idx < count && buffer.successful {
        let u = buffer.cur(0).codepoint;

        if is_hangul_tone(u) {
            /*
             * We could cache the width of the tone marks and the existence of dotted-circle,
             * but the use of the Hangul tone mark characters seems to be rare enough that
             * I didn't bother for now.
             */
            if start < end && end == buffer.out_len {
                /* Tone mark follows a valid syllable; move it in front, unless it's zero width. */
                let idx = buffer.idx;
                buffer.unsafe_to_break_from_outbuffer(start, idx);
                if !buffer.next_glyph() {
                    break;
                }
                if !is_zero_width_char(font, u) {
                    buffer.merge_out_clusters(start, end + 1);
                    let info = buffer.out_infos_mut();
                    let tone = info[end as usize];
                    info.copy_within(start as usize..end as usize, (start + 1) as usize);
                    info[start as usize] = tone;
                }
            } else {
                /* No valid syllable as base for tone mark; try to insert dotted circle. */
                if (buffer.flags & HB_BUFFER_FLAG_DO_NOT_INSERT_DOTTED_CIRCLE) == 0
                    && font.has_glyph(0x25CC)
                {
                    let chars: [HbCodepoint; 2] = if !is_zero_width_char(font, u) {
                        [u, 0x25CC]
                    } else {
                        [0x25CC, u]
                    };
                    let _ = buffer.replace_glyphs(1, 2, &chars);
                } else {
                    /* No dotted circle available in the font; just leave tone mark untouched. */
                    let _ = buffer.next_glyph();
                }
            }
            start = buffer.out_len;
            end = buffer.out_len;
            continue;
        }

        start = buffer.out_len; /* Remember current position as a potential syllable start;
                                 * will only be used if we set end to a later position.
                                 */

        if is_l(u) && buffer.idx + 1 < count {
            let l = u;
            let v = buffer.cur(1).codepoint;
            if is_v(v) {
                /* Have <L,V> or <L,V,T>. */
                let mut t: HbCodepoint = 0;
                let mut tindex: u32 = 0;
                if buffer.idx + 2 < count {
                    t = buffer.cur(2).codepoint;
                    if is_t(t) {
                        tindex = t - T_BASE; /* Only used if isCombiningT (t); otherwise invalid. */
                    } else {
                        t = 0; /* The next character was not a trailing jamo. */
                    }
                }
                let idx = buffer.idx;
                buffer.unsafe_to_break(idx, idx + if t != 0 { 3 } else { 2 });

                /* We've got a syllable <L,V,T?>; see if it can potentially be composed. */
                if is_combining_l(l) && is_combining_v(v) && (t == 0 || is_combining_t(t)) {
                    /* Try to compose; if this succeeds, end is set to start+1. */
                    let s = S_BASE + (l - L_BASE) * N_COUNT + (v - V_BASE) * T_COUNT + tindex;
                    if font.has_glyph(s) {
                        let _ = buffer.replace_glyphs(if t != 0 { 3 } else { 2 }, 1, &[s]);
                        end = start + 1;
                        continue;
                    }
                }

                /* We didn't compose, either because it's an Old Hangul syllable without a
                 * precomposed character in Unicode, or because the font didn't support the
                 * necessary precomposed glyph.
                 * Set jamo features on the individual glyphs, and advance past them.
                 */
                set_hangul_shaping_feature(buffer.cur_mut(0), LJMO);
                let _ = buffer.next_glyph();
                set_hangul_shaping_feature(buffer.cur_mut(0), VJMO);
                let _ = buffer.next_glyph();
                if t != 0 {
                    set_hangul_shaping_feature(buffer.cur_mut(0), TJMO);
                    let _ = buffer.next_glyph();
                    end = start + 3;
                } else {
                    end = start + 2;
                }
                if !buffer.successful {
                    break;
                }
                if buffer.cluster_level == HB_BUFFER_CLUSTER_LEVEL_MONOTONE_GRAPHEMES {
                    buffer.merge_out_clusters(start, end);
                }
                continue;
            }
        } else if is_combined_s(u) {
            /* Have <LV>, <LVT>, or <LV,T> */
            let s = u;
            let has_glyph = font.has_glyph(s);
            let lindex = (s - S_BASE) / N_COUNT;
            let nindex = (s - S_BASE) % N_COUNT;
            let vindex = nindex / T_COUNT;
            let tindex = nindex % T_COUNT;

            if tindex == 0 && buffer.idx + 1 < count && is_combining_t(buffer.cur(1).codepoint) {
                /* <LV,T>, try to combine. */
                let new_tindex = buffer.cur(1).codepoint - T_BASE;
                let new_s = s + new_tindex;
                if font.has_glyph(new_s) {
                    let _ = buffer.replace_glyphs(2, 1, &[new_s]);
                    end = start + 1;
                    continue;
                } else {
                    let idx = buffer.idx;
                    buffer.unsafe_to_break(idx, idx + 2); /* Mark unsafe between LV and T. */
                }
            }

            /* Otherwise, decompose if font doesn't support <LV> or <LVT>,
             * or if having non-combining <LV,T>.  Note that we already handled
             * combining <LV,T> above. */
            if !has_glyph
                || (tindex == 0 && buffer.idx + 1 < count && is_t(buffer.cur(1).codepoint))
            {
                let decomposed: [HbCodepoint; 3] =
                    [L_BASE + lindex, V_BASE + vindex, T_BASE + tindex];
                if font.has_glyph(decomposed[0])
                    && font.has_glyph(decomposed[1])
                    && (tindex == 0 || font.has_glyph(decomposed[2]))
                {
                    let mut s_len = if tindex != 0 { 3 } else { 2 };
                    let _ = buffer.replace_glyphs(1, s_len, &decomposed);

                    /* If we decomposed an LV because of a non-combining T following,
                     * we want to include this T in the syllable.
                     */
                    if has_glyph && tindex == 0 {
                        let _ = buffer.next_glyph();
                        s_len += 1;
                    }
                    if !buffer.successful {
                        break;
                    }

                    /* We decomposed S: apply jamo features to the individual glyphs
                     * that are now in buffer->out_info.
                     */
                    end = start + s_len;

                    let mut i = start;
                    set_hangul_shaping_feature(buffer.out_info_mut(i), LJMO);
                    i += 1;
                    set_hangul_shaping_feature(buffer.out_info_mut(i), VJMO);
                    i += 1;
                    if i < end {
                        set_hangul_shaping_feature(buffer.out_info_mut(i), TJMO);
                    }

                    if buffer.cluster_level == HB_BUFFER_CLUSTER_LEVEL_MONOTONE_GRAPHEMES {
                        buffer.merge_out_clusters(start, end);
                    }
                    continue;
                } else if tindex == 0 && buffer.idx + 1 < count && is_t(buffer.cur(1).codepoint) {
                    let idx = buffer.idx;
                    buffer.unsafe_to_break(idx, idx + 2); /* Mark unsafe between LV and T. */
                }
            }

            if has_glyph {
                /* We didn't decompose the S, so just advance past it and fall through. */
                end = start + 1;
            }
        }

        /* Didn't find a recognizable syllable, so we leave end <= start;
         * this will prevent tone-mark reordering happening.
         */
        let _ = buffer.next_glyph();
    }
    buffer.sync();
}

/// `setup_masks_hangul`
fn setup_masks_hangul(plan: &HbOtShapePlan, buffer: &mut HbBuffer, _font: &mut HbFont) {
    if let Some(ShaperData::Hangul(hangul_plan)) = &plan.data {
        let count = buffer.len as usize;
        for info in &mut buffer.info[..count] {
            info.mask |= hangul_plan.mask_array[info.complex_var_u8_auxiliary() as usize];
        }
    }

    buffer.deallocate_var(VAR_COMPLEX_AUXILIARY.0, VAR_COMPLEX_AUXILIARY.1);
}

/// `_hb_ot_shaper_hangul`
pub(crate) static _hb_ot_shaper_hangul: HbOtShaper = HbOtShaper {
    collect_features: Some(collect_features_hangul),
    override_features: Some(override_features_hangul),
    data_create: Some(data_create_hangul),
    preprocess_text: Some(preprocess_text_hangul),
    postprocess_glyphs: None,
    decompose: None,
    compose: None,
    setup_masks: Some(setup_masks_hangul),
    reorder_marks: None,
    gpos_tag: HB_TAG_NONE,
    normalization_preference: HB_OT_SHAPE_NORMALIZATION_MODE_NONE,
    zero_width_marks: HB_OT_SHAPE_ZERO_WIDTH_MARKS_NONE,
    fallback_position: false, /* fallback_position */
};
