// Rust translation of src/hb-ot-shaper-thai.cc from HarfBuzz (8.5.0, as
// SDL_ttf's external/harfbuzz pins it).
// Copyright © 2010,2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod

//! The Thai / Lao shaper.

use super::hb_buffer::*;
use super::hb_common::*;
use super::hb_font::HbFont;
use super::hb_ot_layout::*;
use super::hb_ot_shape::HbOtShapePlan;
use super::hb_ot_shape_normalize::*;
use super::hb_ot_shaper::*;
use super::hb_unicode::*;

/* Thai / Lao shaper */

/* PUA shaping */

/// `thai_consonant_type_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThaiConsonantType {
    NC,
    AC,
    RC,
    DC,
    NotConsonant,
}
const NUM_CONSONANT_TYPES: usize = ThaiConsonantType::NotConsonant as usize;

/// `get_consonant_type`
fn get_consonant_type(u: HbCodepoint) -> ThaiConsonantType {
    if u == 0x0E1B || u == 0x0E1D || u == 0x0E1F
    /* || u == 0x0E2Cu*/
    {
        return ThaiConsonantType::AC;
    }
    if u == 0x0E0D || u == 0x0E10 {
        return ThaiConsonantType::RC;
    }
    if u == 0x0E0E || u == 0x0E0F {
        return ThaiConsonantType::DC;
    }
    if (0x0E01..=0x0E2E).contains(&u) {
        return ThaiConsonantType::NC;
    }
    ThaiConsonantType::NotConsonant
}

/// `thai_mark_type_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThaiMarkType {
    AV,
    BV,
    T,
    NotMark,
}
const NUM_MARK_TYPES: usize = ThaiMarkType::NotMark as usize;

/// `get_mark_type`
fn get_mark_type(u: HbCodepoint) -> ThaiMarkType {
    if u == 0x0E31
        || (0x0E34..=0x0E37).contains(&u)
        || u == 0x0E47
        || (0x0E4D..=0x0E4E).contains(&u)
    {
        return ThaiMarkType::AV;
    }
    if (0x0E38..=0x0E3A).contains(&u) {
        return ThaiMarkType::BV;
    }
    if (0x0E48..=0x0E4C).contains(&u) {
        return ThaiMarkType::T;
    }
    ThaiMarkType::NotMark
}

/// `thai_action_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThaiAction {
    NOP,
    SD,  /* Shift combining-mark down */
    SL,  /* Shift combining-mark left */
    SDL, /* Shift combining-mark down-left */
    RD,  /* Remove descender from base */
}

/// `thai_pua_mapping_t`
struct ThaiPuaMapping {
    u: u16,
    win_pua: u16,
    mac_pua: u16,
}

const fn m(u: u16, win_pua: u16, mac_pua: u16) -> ThaiPuaMapping {
    ThaiPuaMapping {
        u,
        win_pua,
        mac_pua,
    }
}

static SD_MAPPINGS: [ThaiPuaMapping; 9] = [
    m(0x0E48, 0xF70A, 0xF88B), /* MAI EK */
    m(0x0E49, 0xF70B, 0xF88E), /* MAI THO */
    m(0x0E4A, 0xF70C, 0xF891), /* MAI TRI */
    m(0x0E4B, 0xF70D, 0xF894), /* MAI CHATTAWA */
    m(0x0E4C, 0xF70E, 0xF897), /* THANTHAKHAT */
    m(0x0E38, 0xF718, 0xF89B), /* SARA U */
    m(0x0E39, 0xF719, 0xF89C), /* SARA UU */
    m(0x0E3A, 0xF71A, 0xF89D), /* PHINTHU */
    m(0x0000, 0x0000, 0x0000),
];
static SDL_MAPPINGS: [ThaiPuaMapping; 6] = [
    m(0x0E48, 0xF705, 0xF88C), /* MAI EK */
    m(0x0E49, 0xF706, 0xF88F), /* MAI THO */
    m(0x0E4A, 0xF707, 0xF892), /* MAI TRI */
    m(0x0E4B, 0xF708, 0xF895), /* MAI CHATTAWA */
    m(0x0E4C, 0xF709, 0xF898), /* THANTHAKHAT */
    m(0x0000, 0x0000, 0x0000),
];
static SL_MAPPINGS: [ThaiPuaMapping; 13] = [
    m(0x0E48, 0xF713, 0xF88A), /* MAI EK */
    m(0x0E49, 0xF714, 0xF88D), /* MAI THO */
    m(0x0E4A, 0xF715, 0xF890), /* MAI TRI */
    m(0x0E4B, 0xF716, 0xF893), /* MAI CHATTAWA */
    m(0x0E4C, 0xF717, 0xF896), /* THANTHAKHAT */
    m(0x0E31, 0xF710, 0xF884), /* MAI HAN-AKAT */
    m(0x0E34, 0xF701, 0xF885), /* SARA I */
    m(0x0E35, 0xF702, 0xF886), /* SARA II */
    m(0x0E36, 0xF703, 0xF887), /* SARA UE */
    m(0x0E37, 0xF704, 0xF888), /* SARA UEE */
    m(0x0E47, 0xF712, 0xF889), /* MAITAIKHU */
    m(0x0E4D, 0xF711, 0xF899), /* NIKHAHIT */
    m(0x0000, 0x0000, 0x0000),
];
static RD_MAPPINGS: [ThaiPuaMapping; 3] = [
    m(0x0E0D, 0xF70F, 0xF89A), /* YO YING */
    m(0x0E10, 0xF700, 0xF89E), /* THO THAN */
    m(0x0000, 0x0000, 0x0000),
];

/// `thai_pua_shape`
fn thai_pua_shape(u: HbCodepoint, action: ThaiAction, font: &mut HbFont) -> HbCodepoint {
    let pua_mappings: &[ThaiPuaMapping] = match action {
        ThaiAction::NOP => return u,
        ThaiAction::SD => &SD_MAPPINGS,
        ThaiAction::SDL => &SDL_MAPPINGS,
        ThaiAction::SL => &SL_MAPPINGS,
        ThaiAction::RD => &RD_MAPPINGS,
    };
    for mapping in pua_mappings.iter().take_while(|p| p.u != 0) {
        if mapping.u as HbCodepoint == u {
            let mut glyph = 0;
            if font.get_nominal_glyph(mapping.win_pua as HbCodepoint, &mut glyph, 0) {
                return mapping.win_pua as HbCodepoint;
            }
            if font.get_nominal_glyph(mapping.mac_pua as HbCodepoint, &mut glyph, 0) {
                return mapping.mac_pua as HbCodepoint;
            }
            break;
        }
    }
    u
}

/// `thai_above_state_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThaiAboveState {
    /* Cluster above looks like: */
    T0, /*  ⣤                      */
    T1, /*     ⣼                   */
    T2, /*        ⣾                */
    T3, /*           ⣿             */
}
const NUM_ABOVE_STATES: usize = 4;

static THAI_ABOVE_START_STATE: [ThaiAboveState; NUM_CONSONANT_TYPES + 1 /* For NOT_CONSONANT */] = [
    ThaiAboveState::T0, /* NC */
    ThaiAboveState::T1, /* AC */
    ThaiAboveState::T0, /* RC */
    ThaiAboveState::T0, /* DC */
    ThaiAboveState::T3, /* NOT_CONSONANT */
];

/// `thai_above_state_machine_edge_t`
struct ThaiAboveStateMachineEdge {
    action: ThaiAction,
    next_state: ThaiAboveState,
}

const fn ae(action: ThaiAction, next_state: ThaiAboveState) -> ThaiAboveStateMachineEdge {
    ThaiAboveStateMachineEdge { action, next_state }
}

#[rustfmt::skip]
static THAI_ABOVE_STATE_MACHINE: [[ThaiAboveStateMachineEdge; NUM_MARK_TYPES]; NUM_ABOVE_STATES] = {
    use ThaiAction::*;
    use ThaiAboveState::*;
    [   /*AV*/          /*BV*/          /*T*/
/*T0*/ [ae(NOP, T3), ae(NOP, T0), ae(SD, T3)],
/*T1*/ [ae(SL, T2), ae(NOP, T1), ae(SDL, T2)],
/*T2*/ [ae(NOP, T3), ae(NOP, T2), ae(SL, T3)],
/*T3*/ [ae(NOP, T3), ae(NOP, T3), ae(NOP, T3)],
    ]
};

/// `thai_below_state_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThaiBelowState {
    B0, /* No descender */
    B1, /* Removable descender */
    B2, /* Strict descender */
}
const NUM_BELOW_STATES: usize = 3;

static THAI_BELOW_START_STATE: [ThaiBelowState; NUM_CONSONANT_TYPES + 1 /* For NOT_CONSONANT */] = [
    ThaiBelowState::B0, /* NC */
    ThaiBelowState::B0, /* AC */
    ThaiBelowState::B1, /* RC */
    ThaiBelowState::B2, /* DC */
    ThaiBelowState::B2, /* NOT_CONSONANT */
];

/// `thai_below_state_machine_edge_t`
struct ThaiBelowStateMachineEdge {
    action: ThaiAction,
    next_state: ThaiBelowState,
}

const fn be(action: ThaiAction, next_state: ThaiBelowState) -> ThaiBelowStateMachineEdge {
    ThaiBelowStateMachineEdge { action, next_state }
}

#[rustfmt::skip]
static THAI_BELOW_STATE_MACHINE: [[ThaiBelowStateMachineEdge; NUM_MARK_TYPES]; NUM_BELOW_STATES] = {
    use ThaiAction::*;
    use ThaiBelowState::*;
    [   /*AV*/          /*BV*/          /*T*/
/*B0*/ [be(NOP, B0), be(NOP, B2), be(NOP, B0)],
/*B1*/ [be(NOP, B1), be(RD, B2), be(NOP, B1)],
/*B2*/ [be(NOP, B2), be(SD, B2), be(NOP, B2)],
    ]
};

/// `do_thai_pua_shaping`
fn do_thai_pua_shaping(_plan: &HbOtShapePlan, buffer: &mut HbBuffer, font: &mut HbFont) {
    let mut above_state = THAI_ABOVE_START_STATE[ThaiConsonantType::NotConsonant as usize];
    let mut below_state = THAI_BELOW_START_STATE[ThaiConsonantType::NotConsonant as usize];
    let mut base = 0;

    let count = buffer.len;
    for i in 0..count {
        let mt = get_mark_type(buffer.info[i as usize].codepoint);

        if mt == ThaiMarkType::NotMark {
            let ct = get_consonant_type(buffer.info[i as usize].codepoint);
            above_state = THAI_ABOVE_START_STATE[ct as usize];
            below_state = THAI_BELOW_START_STATE[ct as usize];
            base = i;
            continue;
        }

        let above_edge = &THAI_ABOVE_STATE_MACHINE[above_state as usize][mt as usize];
        let below_edge = &THAI_BELOW_STATE_MACHINE[below_state as usize][mt as usize];
        above_state = above_edge.next_state;
        below_state = below_edge.next_state;

        /* At least one of the above/below actions is NOP. */
        let action = if above_edge.action != ThaiAction::NOP {
            above_edge.action
        } else {
            below_edge.action
        };

        buffer.unsafe_to_break(base, i);
        if action == ThaiAction::RD {
            buffer.info[base as usize].codepoint =
                thai_pua_shape(buffer.info[base as usize].codepoint, action, font);
        } else {
            buffer.info[i as usize].codepoint =
                thai_pua_shape(buffer.info[i as usize].codepoint, action, font);
        }
    }
}

/* We only get one script at a time, so a script-agnostic implementation
 * is adequate here. */
#[inline]
fn is_sara_am(x: HbCodepoint) -> bool {
    (x & !0x0080) == 0x0E33
}
#[inline]
fn nikhahit_from_sara_am(x: HbCodepoint) -> HbCodepoint {
    x - 0x0E33 + 0x0E4D
}
#[inline]
fn sara_aa_from_sara_am(x: HbCodepoint) -> HbCodepoint {
    x - 1
}
#[inline]
fn is_above_base_mark(x: HbCodepoint) -> bool {
    let x = x & !0x0080;
    (0x0E34..=0x0E37).contains(&x) || (0x0E47..=0x0E4E).contains(&x) || x == 0x0E31 || x == 0x0E3B
}

/// `preprocess_text_thai`
fn preprocess_text_thai(plan: &HbOtShapePlan, buffer: &mut HbBuffer, font: &mut HbFont) {
    /* This function implements the shaping logic documented here:
     *
     *   https://linux.thai.net/~thep/th-otf/shaping.html
     *
     * The first shaping rule listed there is needed even if the font has Thai
     * OpenType tables.  The rest do fallback positioning based on PUA codepoints.
     * We implement that only if there exist no Thai GSUB in the font.
     */

    /* The following is NOT specified in the MS OT Thai spec, however, it seems
     * to be what Uniscribe and other engines implement.  According to Eric Muller:
     *
     * When you have a SARA AM, decompose it in NIKHAHIT + SARA AA, *and* move the
     * NIKHAHIT backwards over any above-base marks.
     *
     * <0E14, 0E4B, 0E33> -> <0E14, 0E4D, 0E4B, 0E32>
     *
     * This reordering is legit only when the NIKHAHIT comes from a SARA AM, not
     * when it's there to start with. The string <0E14, 0E4B, 0E4D> is probably
     * not what a user wanted, but the rendering is nevertheless nikhahit above
     * chattawa.
     *
     * Same for Lao.
     *
     * Note:
     *
     * Uniscribe also does some below-marks reordering.  Namely, it positions U+0E3A
     * after U+0E38 and U+0E39.  We do that by modifying the ccc for U+0E3A.
     * See unicode->modified_combining_class ().  Lao does NOT have a U+0E3A
     * equivalent.
     */

    /*
     * Here are the characters of significance:
     *
     *			Thai	Lao
     * SARA AM:		U+0E33	U+0EB3
     * SARA AA:		U+0E32	U+0EB2
     * Nikhahit:		U+0E4D	U+0ECD
     *
     * Testing shows that Uniscribe reorder the following marks:
     * Thai:	<0E31,0E34..0E37,     0E47..0E4E>
     * Lao:	<0EB1,0EB4..0EB7,0EBB,0EC8..0ECD>
     *
     * Note how the Lao versions are the same as Thai + 0x80.
     */

    buffer.clear_output();
    let count = buffer.len;
    buffer.idx = 0;
    while buffer.idx < count
    /* No need for: && buffer->successful */
    {
        let u = buffer.cur(0).codepoint;
        if !is_sara_am(u) {
            if !buffer.next_glyph() {
                break;
            }
            continue;
        }

        /* Is SARA AM. Decompose and reorder. */
        let _ = buffer.output_glyph(nikhahit_from_sara_am(u));
        _hb_glyph_info_set_continuation(buffer.prev_mut());
        if !buffer.replace_glyph(sara_aa_from_sara_am(u)) {
            break;
        }

        /* Make Nikhahit be recognized as a ccc=0 mark when zeroing widths. */
        let end = buffer.out_len;
        _hb_glyph_info_set_general_category(
            buffer.out_info_mut(end - 2),
            HB_UNICODE_GENERAL_CATEGORY_NON_SPACING_MARK,
        );

        /* Ok, let's see... */
        let mut start = end - 2;
        while start > 0 && is_above_base_mark(buffer.out_info(start - 1).codepoint) {
            start -= 1;
        }

        if start + 2 < end {
            /* Move Nikhahit (end-2) to the beginning */
            buffer.merge_out_clusters(start, end);
            let out = buffer.out_infos_mut();
            let t = out[(end - 2) as usize];
            out.copy_within(start as usize..(end - 2) as usize, (start + 1) as usize);
            out[start as usize] = t;
        } else {
            /* Since we decomposed, and NIKHAHIT is combining, merge clusters with the
             * previous cluster. */
            if start != 0 && buffer.cluster_level == HB_BUFFER_CLUSTER_LEVEL_MONOTONE_GRAPHEMES {
                buffer.merge_out_clusters(start - 1, end);
            }
        }
    }
    buffer.sync();

    /* If font has Thai GSUB, we are done. */
    if plan.props.script == HB_SCRIPT_THAI && !plan.map.found_script[0] {
        do_thai_pua_shaping(plan, buffer, font);
    }
}

/// `_hb_ot_shaper_thai`
pub(crate) static _hb_ot_shaper_thai: HbOtShaper = HbOtShaper {
    collect_features: None,
    override_features: None,
    data_create: None,
    preprocess_text: Some(preprocess_text_thai),
    postprocess_glyphs: None,
    decompose: None,
    compose: None,
    setup_masks: None,
    reorder_marks: None,
    gpos_tag: HB_TAG_NONE,
    normalization_preference: HB_OT_SHAPE_NORMALIZATION_MODE_DEFAULT,
    zero_width_marks: HB_OT_SHAPE_ZERO_WIDTH_MARKS_BY_GDEF_LATE,
    fallback_position: false, /* fallback_position */
};
