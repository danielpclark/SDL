// Rust translation of src/hb-ot-kern-table.hh, src/hb-kern.hh and
// hb_ot_layout_kern (src/hb-ot-layout.cc's kern section) from HarfBuzz
// (8.5.0, as SDL_ttf's external/harfbuzz pins it).
// Copyright © 2017  Google, Inc.
// Copyright © 2018  Ebrahim Byagowi
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod

//! kern -- Kerning
//! <https://docs.microsoft.com/en-us/typography/opentype/spec/kern>
//! <https://developer.apple.com/fonts/TrueType-Reference-Manual/RM06/Chap6kern.html>
//!
//! Translation notes: the OpenType (`KernOT`) and the Apple (`KernAAT`)
//! versions of the table are `KerxTable`s
//! ([`super::hb_aat_layout_kerx_table`], with `KerxKind::KernOt` and
//! `KerxKind::KernAat`); their subtables of formats 0, 1 (the state
//! machine) and 2 are `kerx`'s, and format 3 is here.

use super::hb_aat_layout_common::HbAatApplyContext;
use super::hb_aat_layout_kerx_table::*;
use super::hb_buffer::*;
use super::hb_common::*;
use super::hb_face::*;
use super::hb_font::HbFont;
use super::hb_open_type::*;
use super::hb_ot_layout_common::LOOKUP_FLAG_IGNORE_MARKS;
use super::hb_ot_layout_gsubgpos::{GsubGposAccel, GsubGposKind, HbOtApplyContext, Iter};
use super::hb_sanitize::{hb_sanitize_blob, HbSanitizeContext};
use super::hb_set_digest::HbSetDigest;

/* KernSubTableFormat3 */

/// `KernSubTableFormat3::get_kerning`
fn format3_get_kerning(
    d: &[u8],
    st: usize,
    kind: KerxKind,
    left: HbCodepoint,
    right: HbCodepoint,
) -> i32 {
    let p = st + kind.header_size();
    let glyph_count = u16_at(d, p) as usize;
    let kern_value_count = u8_at(d, p + 2) as usize;
    let left_class_count = u8_at(d, p + 3) as u32;
    let right_class_count = u8_at(d, p + 4) as u32;
    let kern_value = p + 6;
    let left_class = kern_value + kern_value_count * 2;
    let right_class = left_class + glyph_count;
    let kern_index = right_class + glyph_count;
    let kern_index_len = (left_class_count * right_class_count) as usize;

    /* (hb_array_t's operator[] gives Null past the end) */
    let left_c = if (left as usize) < glyph_count {
        u8_at(d, left_class + left as usize) as u32
    } else {
        0
    };
    let right_c = if (right as usize) < glyph_count {
        u8_at(d, right_class + right as usize) as u32
    } else {
        0
    };
    if left_c >= left_class_count || right_c >= right_class_count {
        return 0;
    }
    let i = (left_c * right_class_count + right_c) as usize;
    let ki = if i < kern_index_len {
        u8_at(d, kern_index + i) as usize
    } else {
        0
    };
    if ki < kern_value_count {
        i16_at(d, kern_value + ki * 2) as i32
    } else {
        0
    }
}

/// `KernSubTableFormat3::apply`
fn format3_apply(c: &mut HbAatApplyContext, d: &[u8], st: usize, kind: KerxKind) -> bool {
    if !c.plan.requested_kerning {
        return false;
    }

    if kind.coverage(d, st) & kind.backwards() != 0 {
        return false;
    }

    let cross_stream = kind.coverage(d, st) & kind.cross_stream() != 0;
    let mut driver = |_font: &mut HbFont, left: HbCodepoint, right: HbCodepoint| -> i32 {
        format3_get_kerning(d, st, kind, left, right)
    };
    let kern_mask = c.plan.kern_mask;
    hb_kern_machine_kern(
        &mut driver,
        cross_stream,
        c.face,
        c.font,
        c.buffer,
        kern_mask,
        true,
    );

    true
}

/// `KernSubTableFormat3::sanitize`
fn format3_sanitize(c: &mut HbSanitizeContext, st: usize, kind: KerxKind) -> bool {
    let p = st + kind.header_size();
    c.check_struct(st, kind.header_size() + 6) && {
        let glyph_count = c.u16(p) as u32;
        let kern_value_count = c.u8(p + 2) as u32;
        let left_class_count = c.u8(p + 3) as u32;
        let right_class_count = c.u8(p + 4) as u32;
        c.check_range(
            p + 6,
            kern_value_count * 2 + glyph_count * 2 + left_class_count * right_class_count,
        )
    }
}

/// `KernSubTableFormat3::collect_glyphs`
fn format3_collect_glyphs(
    d: &[u8],
    st: usize,
    kind: KerxKind,
    left_set: &mut HbSetDigest,
    right_set: &mut HbSetDigest,
) {
    let glyph_count = u16_at(d, st + kind.header_size()) as u32;
    let mut set = HbSetDigest::default();
    if glyph_count != 0 {
        set.add_range(0, glyph_count - 1);
    }
    left_set.union_(&set);
    right_set.union_(&set);
}

/* KernSubTable */

/// `KernSubTable::dispatch (hb_aat_apply_context_t)`
pub(crate) fn kern_subtable_apply(
    c: &mut HbAatApplyContext,
    d: &[u8],
    st: usize,
    kind: KerxKind,
) -> bool {
    match kind.subtable_type(d, st) {
        0 => format0_apply(c, d, st, kind),
        1 => format1_apply(c, d, st, kind),
        2 => format2_apply(c, d, st, kind),
        3 => format3_apply(c, d, st, kind),
        _ => false,
    }
}

/// `KernSubTable::collect_glyphs`
pub(crate) fn kern_subtable_collect_glyphs(
    d: &[u8],
    st: usize,
    kind: KerxKind,
    left_set: &mut HbSetDigest,
    right_set: &mut HbSetDigest,
    num_glyphs: u32,
) {
    match kind.subtable_type(d, st) {
        0 => format0_collect_glyphs(d, st, kind, left_set, right_set),
        1 => format1_collect_glyphs(d, st, kind, left_set, right_set, num_glyphs),
        2 => format2_collect_glyphs(d, st, kind, left_set, right_set, num_glyphs),
        3 => format3_collect_glyphs(d, st, kind, left_set, right_set),
        _ => {}
    }
}

/// `KernSubTable::sanitize`
pub(crate) fn kern_subtable_sanitize(c: &mut HbSanitizeContext, st: usize, kind: KerxKind) -> bool {
    if !(c.check_struct(st, kind.header_size())
        && kind.length(&c.data, st) >= kind.header_size() as u32
        && c.check_range(st, kind.length(&c.data, st)))
    {
        return false;
    }

    match kind.subtable_type(&c.data, st) {
        0 => format0_sanitize(c, st, kind),
        1 => format1_sanitize(c, st, kind),
        2 => format2_sanitize(c, st, kind),
        3 => format3_sanitize(c, st, kind),
        _ => true,
    }
}

/* KernOTSubTableHeader: versionZ (HBUINT16, unused), length (HBUINT16,
 * length of the subtable (including this header)), format (HBUINT8,
 * subtable format), coverage (HBUINT8, coverage bits): 6 bytes.
 *
 * KernAATSubTableHeader: length (HBUINT32), coverage (HBUINT8), format
 * (HBUINT8), tupleIndex (HBUINT16, the tuple index (used for variations
 * fonts); not implemented): 8 bytes.
 *
 * See KerxKind. */

/* kern */

/// `kern::sanitize`
fn kern_sanitize(c: &mut HbSanitizeContext) -> bool {
    if !c.check_struct(0, 4) {
        return false;
    }
    match c.u16(0) {
        0 => kerx_table_sanitize(c, KerxKind::KernOt),
        1 => kerx_table_sanitize(c, KerxKind::KernAat),
        _ => true,
    }
}

/// `kern::accelerator_t`
#[derive(Debug)]
pub(crate) struct KernAccel {
    pub(crate) table: Vec<u8>,
    /// `kern_accelerator_data_t`: the left and right glyph digests of
    /// each subtable
    accel_data: KernAcceleratorData,
}

impl KernAccel {
    pub(crate) fn new(face: &HbFace) -> KernAccel {
        let num_glyphs = face.get_num_glyphs();
        let table = hb_sanitize_blob(
            face.reference_table(HB_OT_TAG_KERN),
            Some(num_glyphs),
            kern_sanitize,
        );
        let mut k = KernAccel {
            table,
            accel_data: Vec::new(),
        };
        k.accel_data = k.create_accelerator_data(num_glyphs);
        k
    }

    /// The table's kind (`get_type`), if it is a known one.
    fn kind(&self) -> Option<KerxKind> {
        match u16_at(&self.table, 0) {
            0 => Some(KerxKind::KernOt),
            1 => Some(KerxKind::KernAat),
            _ => None,
        }
    }

    /// `get_blob`
    pub(crate) fn get_blob(&self) -> &[u8] {
        &self.table
    }

    /// `create_accelerator_data`
    fn create_accelerator_data(&self, num_glyphs: u32) -> KernAcceleratorData {
        match self.kind() {
            Some(kind) => kerx_table_create_accelerator_data(&self.table, kind, num_glyphs),
            None => Vec::new(),
        }
    }

    /// `has_data`
    pub(crate) fn has_data(&self) -> bool {
        u32_at(&self.table, 0) != 0
    }

    /// `has_state_machine`
    pub(crate) fn has_state_machine(&self) -> bool {
        match self.kind() {
            Some(kind) => kerx_table_has_state_machine(&self.table, kind),
            None => false,
        }
    }

    /// `has_cross_stream`
    pub(crate) fn has_cross_stream(&self) -> bool {
        match self.kind() {
            Some(kind) => kerx_table_has_cross_stream(&self.table, kind),
            None => false,
        }
    }

    /// `apply` (`kern::apply`: the dispatch to the table's kind)
    pub(crate) fn apply(&self, c: &mut HbAatApplyContext) -> bool {
        match self.kind() {
            Some(kind) => kerx_table_apply(c, &self.table, kind, Some(&self.accel_data)),
            None => false,
        }
    }
}

/// `hb_kern_machine_t::kern`
pub(crate) fn hb_kern_machine_kern(
    driver: &mut dyn FnMut(&mut HbFont, HbCodepoint, HbCodepoint) -> i32,
    cross_stream: bool,
    face: &HbFace,
    font: &mut HbFont,
    buffer: &mut HbBuffer,
    kern_mask: HbMask,
    scale: bool,
) {
    buffer.unsafe_to_concat(0, u32::MAX);
    /* (on the empty blob) */
    let empty = GsubGposAccel::empty(GsubGposKind::Gpos);
    let mut c = HbOtApplyContext::new(1, font, face, buffer, &empty);
    c.set_lookup_mask(kern_mask, true);
    c.set_lookup_props(LOOKUP_FLAG_IGNORE_MARKS);

    let horizontal = hb_direction_is_horizontal(c.buffer.props.direction);
    let count = c.buffer.len;
    let mut idx = 0;
    while idx < count {
        if c.buffer.info[idx as usize].mask & kern_mask == 0 {
            idx += 1;
            continue;
        }

        c.iter_reset(Iter::Input, idx);
        let mut unsafe_to = 0;
        if !c.iter_next(Iter::Input, Some(&mut unsafe_to)) {
            idx += 1;
            continue;
        }

        let i = idx as usize;
        let j = c.iter_input.idx as usize;

        let (left, right) = (c.buffer.info[i].codepoint, c.buffer.info[j].codepoint);
        let mut kern = driver(c.font, left, right);

        'skip: {
            if kern == 0 {
                break 'skip;
            }

            let buffer = &mut *c.buffer;
            if horizontal {
                if scale {
                    kern = c.font.em_scale_x(kern as i16);
                }
                if cross_stream {
                    buffer.pos[j].y_offset = kern;
                    buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT;
                } else {
                    let kern1 = kern >> 1;
                    let kern2 = kern.wrapping_sub(kern1);
                    buffer.pos[i].x_advance = buffer.pos[i].x_advance.wrapping_add(kern1);
                    buffer.pos[j].x_advance = buffer.pos[j].x_advance.wrapping_add(kern2);
                    buffer.pos[j].x_offset = buffer.pos[j].x_offset.wrapping_add(kern2);
                }
            } else {
                if scale {
                    kern = c.font.em_scale_y(kern as i16);
                }
                if cross_stream {
                    buffer.pos[j].x_offset = kern;
                    buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT;
                } else {
                    let kern1 = kern >> 1;
                    let kern2 = kern.wrapping_sub(kern1);
                    buffer.pos[i].y_advance = buffer.pos[i].y_advance.wrapping_add(kern1);
                    buffer.pos[j].y_advance = buffer.pos[j].y_advance.wrapping_add(kern2);
                    buffer.pos[j].y_offset = buffer.pos[j].y_offset.wrapping_add(kern2);
                }
            }

            buffer.unsafe_to_break(i as u32, j as u32 + 1);
        }

        /* skip: */
        idx = c.iter_input.idx;
    }
}
