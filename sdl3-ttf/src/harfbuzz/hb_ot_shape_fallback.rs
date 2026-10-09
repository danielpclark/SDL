// Rust translation of src/hb-ot-shape-fallback.cc from HarfBuzz (8.5.0,
// as SDL_ttf's external/harfbuzz pins it).
// Copyright © 2011,2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod

//! Fallback positioning: of marks (by their combining classes), kerning
//! (by the font's kerning function) and spaces.

use super::hb_buffer::*;
use super::hb_common::*;
use super::hb_font::{HbFont, HbGlyphExtents};
use super::hb_ot_kern_table::hb_kern_machine_kern;
use super::hb_ot_layout::*;
use super::hb_ot_shape::HbOtShapePlan;
use super::hb_unicode::*;

/// `recategorize_combining_class`
fn recategorize_combining_class(u: HbCodepoint, mut klass: u32) -> u32 {
    if klass >= 200 {
        return klass;
    }

    /* Thai / Lao need some per-character work. */
    if (u & !0xFF) == 0x0E00 {
        if klass == 0 {
            match u {
                0x0E31 | 0x0E34 | 0x0E35 | 0x0E36 | 0x0E37 | 0x0E47 | 0x0E4C | 0x0E4D | 0x0E4E => {
                    klass = HB_UNICODE_COMBINING_CLASS_ABOVE_RIGHT;
                }

                0x0EB1 | 0x0EB4 | 0x0EB5 | 0x0EB6 | 0x0EB7 | 0x0EBB | 0x0ECC | 0x0ECD => {
                    klass = HB_UNICODE_COMBINING_CLASS_ABOVE;
                }

                0x0EBC => {
                    klass = HB_UNICODE_COMBINING_CLASS_BELOW;
                }
                _ => {}
            }
        } else {
            /* Thai virama is below-right */
            if u == 0x0E3A {
                klass = HB_UNICODE_COMBINING_CLASS_BELOW_RIGHT;
            }
        }
    }

    match klass {
        /* Hebrew */
        HB_MODIFIED_COMBINING_CLASS_CCC10 /* sheva */
        | HB_MODIFIED_COMBINING_CLASS_CCC11 /* hataf segol */
        | HB_MODIFIED_COMBINING_CLASS_CCC12 /* hataf patah */
        | HB_MODIFIED_COMBINING_CLASS_CCC13 /* hataf qamats */
        | HB_MODIFIED_COMBINING_CLASS_CCC14 /* hiriq */
        | HB_MODIFIED_COMBINING_CLASS_CCC15 /* tsere */
        | HB_MODIFIED_COMBINING_CLASS_CCC16 /* segol */
        | HB_MODIFIED_COMBINING_CLASS_CCC17 /* patah */
        | HB_MODIFIED_COMBINING_CLASS_CCC18 /* qamats & qamats qatan */
        | HB_MODIFIED_COMBINING_CLASS_CCC20 /* qubuts */
        | HB_MODIFIED_COMBINING_CLASS_CCC22 /* meteg */ => HB_UNICODE_COMBINING_CLASS_BELOW,

        HB_MODIFIED_COMBINING_CLASS_CCC23 /* rafe */ => HB_UNICODE_COMBINING_CLASS_ATTACHED_ABOVE,

        HB_MODIFIED_COMBINING_CLASS_CCC24 /* shin dot */ => HB_UNICODE_COMBINING_CLASS_ABOVE_RIGHT,

        HB_MODIFIED_COMBINING_CLASS_CCC25 /* sin dot */
        | HB_MODIFIED_COMBINING_CLASS_CCC19 /* holam & holam haser for vav */ => HB_UNICODE_COMBINING_CLASS_ABOVE_LEFT,

        HB_MODIFIED_COMBINING_CLASS_CCC26 /* point varika */ => HB_UNICODE_COMBINING_CLASS_ABOVE,

        HB_MODIFIED_COMBINING_CLASS_CCC21 /* dagesh */ => klass,

        /* Arabic and Syriac */
        HB_MODIFIED_COMBINING_CLASS_CCC27 /* fathatan */
        | HB_MODIFIED_COMBINING_CLASS_CCC28 /* dammatan */
        | HB_MODIFIED_COMBINING_CLASS_CCC30 /* fatha */
        | HB_MODIFIED_COMBINING_CLASS_CCC31 /* damma */
        | HB_MODIFIED_COMBINING_CLASS_CCC33 /* shadda */
        | HB_MODIFIED_COMBINING_CLASS_CCC34 /* sukun */
        | HB_MODIFIED_COMBINING_CLASS_CCC35 /* superscript alef */
        | HB_MODIFIED_COMBINING_CLASS_CCC36 /* superscript alaph */ => HB_UNICODE_COMBINING_CLASS_ABOVE,

        HB_MODIFIED_COMBINING_CLASS_CCC29 /* kasratan */
        | HB_MODIFIED_COMBINING_CLASS_CCC32 /* kasra */ => HB_UNICODE_COMBINING_CLASS_BELOW,

        /* Thai */
        HB_MODIFIED_COMBINING_CLASS_CCC103 /* sara u / sara uu */ => HB_UNICODE_COMBINING_CLASS_BELOW_RIGHT,

        HB_MODIFIED_COMBINING_CLASS_CCC107 /* mai */ => HB_UNICODE_COMBINING_CLASS_ABOVE_RIGHT,

        /* Lao */
        HB_MODIFIED_COMBINING_CLASS_CCC118 /* sign u / sign uu */ => HB_UNICODE_COMBINING_CLASS_BELOW,

        HB_MODIFIED_COMBINING_CLASS_CCC122 /* mai */ => HB_UNICODE_COMBINING_CLASS_ABOVE,

        /* Tibetan */
        HB_MODIFIED_COMBINING_CLASS_CCC129 /* sign aa */ => HB_UNICODE_COMBINING_CLASS_BELOW,

        HB_MODIFIED_COMBINING_CLASS_CCC130 /* sign i*/ => HB_UNICODE_COMBINING_CLASS_ABOVE,

        HB_MODIFIED_COMBINING_CLASS_CCC132 /* sign u */ => HB_UNICODE_COMBINING_CLASS_BELOW,

        _ => klass,
    }
}

/// `_hb_ot_shape_fallback_mark_position_recategorize_marks`
pub(crate) fn _hb_ot_shape_fallback_mark_position_recategorize_marks(buffer: &mut HbBuffer) {
    let count = buffer.len as usize;
    for info in &mut buffer.info[..count] {
        if _hb_glyph_info_get_general_category(info) == HB_UNICODE_GENERAL_CATEGORY_NON_SPACING_MARK
        {
            let mut combining_class = _hb_glyph_info_get_modified_combining_class(info);
            combining_class = recategorize_combining_class(info.codepoint, combining_class);
            _hb_glyph_info_set_modified_combining_class(info, combining_class);
        }
    }
}

/// `zero_mark_advances`
fn zero_mark_advances(
    buffer: &mut HbBuffer,
    start: u32,
    end: u32,
    adjust_offsets_when_zeroing: bool,
) {
    for i in start as usize..end as usize {
        if _hb_glyph_info_get_general_category(&buffer.info[i])
            == HB_UNICODE_GENERAL_CATEGORY_NON_SPACING_MARK
        {
            let pos = &mut buffer.pos[i];
            if adjust_offsets_when_zeroing {
                pos.x_offset = pos.x_offset.wrapping_sub(pos.x_advance);
                pos.y_offset = pos.y_offset.wrapping_sub(pos.y_advance);
            }
            pos.x_advance = 0;
            pos.y_advance = 0;
        }
    }
}

/// `position_mark`
fn position_mark(
    font: &mut HbFont,
    buffer: &mut HbBuffer,
    base_extents: &mut HbGlyphExtents,
    i: usize,
    combining_class: u32,
) {
    let mut mark_extents = HbGlyphExtents::default();
    if !font.get_glyph_extents(buffer.info[i].codepoint, &mut mark_extents) {
        return;
    }

    let y_gap: HbPosition = font.p.y_scale / 16;

    let direction = buffer.props.direction;
    let pos = &mut buffer.pos[i];
    pos.x_offset = 0;
    pos.y_offset = 0;

    /* We don't position LEFT and RIGHT marks. */

    /* X positioning */
    let center = |pos: &mut HbGlyphPosition, base_extents: &HbGlyphExtents| {
        /* Center align. */
        pos.x_offset += base_extents.x_bearing + (base_extents.width - mark_extents.width) / 2
            - mark_extents.x_bearing;
    };
    match combining_class {
        HB_UNICODE_COMBINING_CLASS_DOUBLE_BELOW | HB_UNICODE_COMBINING_CLASS_DOUBLE_ABOVE => {
            if direction == HB_DIRECTION_LTR {
                pos.x_offset += base_extents.x_bearing + base_extents.width
                    - mark_extents.width / 2
                    - mark_extents.x_bearing;
            } else if direction == HB_DIRECTION_RTL {
                pos.x_offset +=
                    base_extents.x_bearing - mark_extents.width / 2 - mark_extents.x_bearing;
            } else {
                center(pos, base_extents);
            }
        }

        HB_UNICODE_COMBINING_CLASS_ATTACHED_BELOW_LEFT
        | HB_UNICODE_COMBINING_CLASS_BELOW_LEFT
        | HB_UNICODE_COMBINING_CLASS_ABOVE_LEFT => {
            /* Left align. */
            pos.x_offset += base_extents.x_bearing - mark_extents.x_bearing;
        }

        HB_UNICODE_COMBINING_CLASS_ATTACHED_ABOVE_RIGHT
        | HB_UNICODE_COMBINING_CLASS_BELOW_RIGHT
        | HB_UNICODE_COMBINING_CLASS_ABOVE_RIGHT => {
            /* Right align. */
            pos.x_offset += base_extents.x_bearing + base_extents.width
                - mark_extents.width
                - mark_extents.x_bearing;
        }

        /* default, ATTACHED_BELOW, ATTACHED_ABOVE, BELOW, ABOVE */
        _ => center(pos, base_extents),
    }

    /* Y positioning */
    let below = |pos: &mut HbGlyphPosition, base_extents: &mut HbGlyphExtents| {
        pos.y_offset = base_extents.y_bearing + base_extents.height - mark_extents.y_bearing;
        /* Never shift up "below" marks. */
        if (y_gap > 0) == (pos.y_offset > 0) {
            base_extents.height -= pos.y_offset;
            pos.y_offset = 0;
        }
        base_extents.height += mark_extents.height;
    };
    let above = |pos: &mut HbGlyphPosition, base_extents: &mut HbGlyphExtents| {
        pos.y_offset = base_extents.y_bearing - (mark_extents.y_bearing + mark_extents.height);
        /* Don't shift down "above" marks too much. */
        if (y_gap > 0) != (pos.y_offset > 0) {
            let correction = -pos.y_offset / 2;
            base_extents.y_bearing += correction;
            base_extents.height -= correction;
            pos.y_offset += correction;
        }
        base_extents.y_bearing -= mark_extents.height;
        base_extents.height += mark_extents.height;
    };
    match combining_class {
        HB_UNICODE_COMBINING_CLASS_DOUBLE_BELOW
        | HB_UNICODE_COMBINING_CLASS_BELOW_LEFT
        | HB_UNICODE_COMBINING_CLASS_BELOW
        | HB_UNICODE_COMBINING_CLASS_BELOW_RIGHT => {
            /* Add gap, fall-through. */
            base_extents.height -= y_gap;
            below(pos, base_extents);
        }

        HB_UNICODE_COMBINING_CLASS_ATTACHED_BELOW_LEFT
        | HB_UNICODE_COMBINING_CLASS_ATTACHED_BELOW => {
            below(pos, base_extents);
        }

        HB_UNICODE_COMBINING_CLASS_DOUBLE_ABOVE
        | HB_UNICODE_COMBINING_CLASS_ABOVE_LEFT
        | HB_UNICODE_COMBINING_CLASS_ABOVE
        | HB_UNICODE_COMBINING_CLASS_ABOVE_RIGHT => {
            /* Add gap, fall-through. */
            base_extents.y_bearing += y_gap;
            base_extents.height -= y_gap;
            above(pos, base_extents);
        }

        HB_UNICODE_COMBINING_CLASS_ATTACHED_ABOVE
        | HB_UNICODE_COMBINING_CLASS_ATTACHED_ABOVE_RIGHT => {
            above(pos, base_extents);
        }
        _ => {}
    }
}

/// `position_around_base`
fn position_around_base(
    plan: &HbOtShapePlan,
    font: &mut HbFont,
    buffer: &mut HbBuffer,
    base: u32,
    end: u32,
    adjust_offsets_when_zeroing: bool,
) {
    let mut horiz_dir = HB_DIRECTION_INVALID;

    buffer.unsafe_to_break(base, end);

    let base_u = base as usize;
    let mut base_extents = HbGlyphExtents::default();
    if !font.get_glyph_extents(buffer.info[base_u].codepoint, &mut base_extents) {
        /* If extents don't work, zero marks and go home. */
        zero_mark_advances(buffer, base + 1, end, adjust_offsets_when_zeroing);
        return;
    }
    base_extents.y_bearing += buffer.pos[base_u].y_offset;
    /* Use horizontal advance for horizontal positioning.
     * Generally a better idea.  Also works for zero-ink glyphs.  See:
     * https://github.com/harfbuzz/harfbuzz/issues/1532 */
    base_extents.x_bearing = 0;
    base_extents.width = font.get_glyph_h_advance(buffer.info[base_u].codepoint);

    let lig_id = _hb_glyph_info_get_lig_id(&buffer.info[base_u]);
    /* Use integer for num_lig_components such that it doesn't convert to unsigned
     * when we divide or multiply by it. */
    let num_lig_components = _hb_glyph_info_get_lig_num_comps(&buffer.info[base_u]) as i32;

    let mut x_offset: HbPosition = 0;
    let mut y_offset: HbPosition = 0;
    if hb_direction_is_forward(buffer.props.direction) {
        x_offset -= buffer.pos[base_u].x_advance;
        y_offset -= buffer.pos[base_u].y_advance;
    }

    let mut component_extents = base_extents;
    let mut last_lig_component: i32 = -1;
    let mut last_combining_class: u32 = 255;
    let mut cluster_extents = base_extents; /* Initialization is just to shut gcc up. */
    for i in (base + 1) as usize..end as usize {
        if _hb_glyph_info_get_modified_combining_class(&buffer.info[i]) != 0 {
            if num_lig_components > 1 {
                let this_lig_id = _hb_glyph_info_get_lig_id(&buffer.info[i]);
                let mut this_lig_component =
                    _hb_glyph_info_get_lig_comp(&buffer.info[i]) as i32 - 1;
                /* Conditions for attaching to the last component. */
                if lig_id == 0 || lig_id != this_lig_id || this_lig_component >= num_lig_components
                {
                    this_lig_component = num_lig_components - 1;
                }
                if last_lig_component != this_lig_component {
                    last_lig_component = this_lig_component;
                    last_combining_class = 255;
                    component_extents = base_extents;
                    if horiz_dir == HB_DIRECTION_INVALID {
                        if hb_direction_is_horizontal(plan.props.direction) {
                            horiz_dir = plan.props.direction;
                        } else {
                            horiz_dir = hb_script_get_horizontal_direction(plan.props.script);
                        }
                    }
                    if horiz_dir == HB_DIRECTION_LTR {
                        component_extents.x_bearing +=
                            (this_lig_component * component_extents.width) / num_lig_components;
                    } else {
                        component_extents.x_bearing +=
                            ((num_lig_components - 1 - this_lig_component)
                                * component_extents.width)
                                / num_lig_components;
                    }
                    component_extents.width /= num_lig_components;
                }
            }

            let this_combining_class = _hb_glyph_info_get_modified_combining_class(&buffer.info[i]);
            if last_combining_class != this_combining_class {
                last_combining_class = this_combining_class;
                cluster_extents = component_extents;
            }

            position_mark(font, buffer, &mut cluster_extents, i, this_combining_class);

            buffer.pos[i].x_advance = 0;
            buffer.pos[i].y_advance = 0;
            buffer.pos[i].x_offset += x_offset;
            buffer.pos[i].y_offset += y_offset;
        } else if hb_direction_is_forward(buffer.props.direction) {
            x_offset -= buffer.pos[i].x_advance;
            y_offset -= buffer.pos[i].y_advance;
        } else {
            x_offset += buffer.pos[i].x_advance;
            y_offset += buffer.pos[i].y_advance;
        }
    }
}

/// `position_cluster`
fn position_cluster(
    plan: &HbOtShapePlan,
    font: &mut HbFont,
    buffer: &mut HbBuffer,
    start: u32,
    end: u32,
    adjust_offsets_when_zeroing: bool,
) {
    if end - start < 2 {
        return;
    }

    /* Find the base glyph */
    let mut i = start;
    while i < end {
        if !_hb_glyph_info_is_unicode_mark(&buffer.info[i as usize]) {
            /* Find mark glyphs */
            let mut j = i + 1;
            while j < end {
                if !_hb_glyph_info_is_unicode_mark(&buffer.info[j as usize]) {
                    break;
                }
                j += 1;
            }

            position_around_base(plan, font, buffer, i, j, adjust_offsets_when_zeroing);

            i = j - 1;
        }
        i += 1;
    }
}

/// `_hb_ot_shape_fallback_mark_position`
pub(crate) fn _hb_ot_shape_fallback_mark_position(
    plan: &HbOtShapePlan,
    font: &mut HbFont,
    buffer: &mut HbBuffer,
    adjust_offsets_when_zeroing: bool,
) {
    let mut start = 0;
    let count = buffer.len;
    for i in 1..count {
        if !_hb_glyph_info_is_unicode_mark(&buffer.info[i as usize]) {
            position_cluster(plan, font, buffer, start, i, adjust_offsets_when_zeroing);
            start = i;
        }
    }
    position_cluster(
        plan,
        font,
        buffer,
        start,
        count,
        adjust_offsets_when_zeroing,
    );
}

/// `_hb_ot_shape_fallback_kern`: performs font-assisted kerning.
pub(crate) fn _hb_ot_shape_fallback_kern(
    plan: &HbOtShapePlan,
    font: &mut HbFont,
    buffer: &mut HbBuffer,
) {
    if if hb_direction_is_horizontal(buffer.props.direction) {
        !font.has_glyph_h_kerning_func()
    } else {
        !font.has_glyph_v_kerning_func()
    } {
        return;
    }

    let reverse = hb_direction_is_backward(buffer.props.direction);

    if reverse {
        buffer.reverse();
    }

    /* hb_ot_shape_fallback_kern_driver_t */
    let direction = buffer.props.direction;
    let mut driver = |font: &mut HbFont, first: HbCodepoint, second: HbCodepoint| -> HbPosition {
        let mut kern = 0;
        let mut kern_y = 0;
        font.get_glyph_kerning_for_direction(first, second, direction, &mut kern, &mut kern_y);
        /* (C passes &kern for both: the y kerning wins) */
        if hb_direction_is_horizontal(direction) {
            kern
        } else {
            kern_y
        }
    };
    let face = font.p.face.clone();
    hb_kern_machine_kern(
        &mut driver,
        false,
        &face,
        font,
        buffer,
        plan.kern_mask,
        false,
    );

    if reverse {
        buffer.reverse();
    }
}

/// `_hb_ot_shape_fallback_spaces`: adjusts width of various spaces.
pub(crate) fn _hb_ot_shape_fallback_spaces(font: &mut HbFont, buffer: &mut HbBuffer) {
    let horizontal = hb_direction_is_horizontal(buffer.props.direction);
    let count = buffer.len as usize;
    for i in 0..count {
        if _hb_glyph_info_is_unicode_space(&buffer.info[i])
            && !_hb_glyph_info_ligated(&buffer.info[i])
        {
            /* If font had no ASCII space and we used the invisible glyph, give it a 1/4 EM default advance. */
            if buffer.invisible != 0 && buffer.info[i].codepoint == buffer.invisible {
                if horizontal {
                    buffer.pos[i].x_advance = font.p.x_scale / 4;
                } else {
                    buffer.pos[i].y_advance = -font.p.y_scale / 4;
                }
            }

            let space_type = _hb_glyph_info_get_unicode_space_fallback_type(&buffer.info[i]);
            let mut glyph = 0;
            match space_type as SpaceType {
                NOT_SPACE /* Shouldn't happen. */ | SPACE => {}

                SPACE_EM | SPACE_EM_2 | SPACE_EM_3 | SPACE_EM_4 | SPACE_EM_5 | SPACE_EM_6 | SPACE_EM_16 => {
                    let st = space_type as i32;
                    if horizontal {
                        buffer.pos[i].x_advance = (font.p.x_scale + st / 2) / st;
                    } else {
                        buffer.pos[i].y_advance = -(font.p.y_scale + st / 2) / st;
                    }
                }

                SPACE_4_EM_18 => {
                    if horizontal {
                        buffer.pos[i].x_advance = (font.p.x_scale as i64 * 4 / 18) as HbPosition;
                    } else {
                        buffer.pos[i].y_advance = (-(font.p.y_scale as i64) * 4 / 18) as HbPosition;
                    }
                }

                SPACE_FIGURE => {
                    for u in b'0'..=b'9' {
                        if font.get_nominal_glyph(u as u32, &mut glyph, 0) {
                            if horizontal {
                                buffer.pos[i].x_advance = font.get_glyph_h_advance(glyph);
                            } else {
                                buffer.pos[i].y_advance = font.get_glyph_v_advance(glyph);
                            }
                            break;
                        }
                    }
                }

                SPACE_PUNCTUATION => {
                    if font.get_nominal_glyph(b'.' as u32, &mut glyph, 0)
                        || font.get_nominal_glyph(b',' as u32, &mut glyph, 0)
                    {
                        if horizontal {
                            buffer.pos[i].x_advance = font.get_glyph_h_advance(glyph);
                        } else {
                            buffer.pos[i].y_advance = font.get_glyph_v_advance(glyph);
                        }
                    }
                }

                SPACE_NARROW => {
                    /* Half-space?
                     * Unicode doc https://unicode.org/charts/PDF/U2000.pdf says ~1/4 or 1/5 of EM.
                     * However, in my testing, many fonts have their regular space being about that
                     * size.  To me, a percentage of the space width makes more sense.  Half is as
                     * good as any. */
                    if horizontal {
                        buffer.pos[i].x_advance /= 2;
                    } else {
                        buffer.pos[i].y_advance /= 2;
                    }
                }
                _ => {}
            }
        }
    }
}
