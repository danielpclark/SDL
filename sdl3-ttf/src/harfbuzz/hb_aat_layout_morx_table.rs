// Rust translation of src/hb-aat-layout-morx-table.hh from HarfBuzz
// (8.5.0, as SDL_ttf's external/harfbuzz pins it).
// Copyright © 2017  Google, Inc.
// Copyright © 2018  Ebrahim Byagowi
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod

//! morx -- Extended Glyph Metamorphosis
//! <https://developer.apple.com/fonts/TrueType-Reference-Manual/RM06/Chap6morx.html>
//! <https://developer.apple.com/fonts/TrueType-Reference-Manual/RM06/Chap6mort.html>
//!
//! Translation notes: `morx` (`ExtendedTypes`) and `mort`
//! (`ObsoleteTypes`) share the code, with the `extended` flag; the
//! subtables are their positions in the table, past their
//! `ChainSubtable` header.

use std::sync::OnceLock;

use super::hb_aat_layout_common::*;
use super::hb_aat_map::*;
use super::hb_buffer::HbBuffer;
use super::hb_common::*;
use super::hb_face::HbFace;
use super::hb_open_type::*;
use super::hb_ot_layout::{_hb_glyph_info_set_glyph_props, UPROPS_MASK_IGNORABLE};
use super::hb_ot_layout_gsubgpos::HB_MAX_CONTEXT_LENGTH;
use super::hb_sanitize::{hb_sanitize_blob, HbSanitizeContext};
use super::hb_set_digest::HbSetDigest;

/// `HB_AAT_TAG_morx`
pub(crate) const HB_AAT_TAG_MORX: HbTag = hb_tag(b'm', b'o', b'r', b'x');
/// `HB_AAT_TAG_mort`
pub(crate) const HB_AAT_TAG_MORT: HbTag = hb_tag(b'm', b'o', b'r', b't');

/// `sizeof (HBUINT)`
#[inline]
fn hbuint_size(extended: bool) -> usize {
    if extended {
        4
    } else {
        2
    }
}

/// An `HBUINT` field.
#[inline]
fn hbuint_at(d: &[u8], p: usize, extended: bool) -> u32 {
    if extended {
        u32_at(d, p)
    } else {
        u16_at(d, p) as u32
    }
}

/* RearrangementSubtable */

/// `RearrangementSubtable::driver_context_t`
struct RearrangementDriverContext {
    ret: bool,
    start: u32,
    end: u32,
}

/* Flags */
const REARRANGEMENT_MARK_FIRST: u16 = 0x8000; /* If set, make the current glyph the first
                                               * glyph to be rearranged. */
const REARRANGEMENT_DONT_ADVANCE: u16 = 0x4000; /* If set, don't advance to the next glyph
                                                 * before going to the new state. This means
                                                 * that the glyph index doesn't change, even
                                                 * if the glyph at that index has changed. */
const REARRANGEMENT_MARK_LAST: u16 = 0x2000; /* If set, make the current glyph the last
                                              * glyph to be rearranged. */
const REARRANGEMENT_RESERVED: u16 = 0x1FF0; /* These bits are reserved and should be set to 0. */
const REARRANGEMENT_VERB: u16 = 0x000F; /* The type of rearrangement specified. */

impl DriverContext for RearrangementDriverContext {
    const IN_PLACE: bool = true;
    const DONT_ADVANCE: u16 = REARRANGEMENT_DONT_ADVANCE;

    fn is_actionable(&self, _buffer: &HbBuffer, entry: &Entry) -> bool {
        (entry.flags & REARRANGEMENT_VERB) != 0 && self.start < self.end
    }

    fn transition(&mut self, c: &mut HbAatApplyContext, _num_glyphs: u32, entry: &Entry) {
        let buffer = &mut *c.buffer;
        let flags = entry.flags;

        if flags & REARRANGEMENT_MARK_FIRST != 0 {
            self.start = buffer.idx;
        }

        if flags & REARRANGEMENT_MARK_LAST != 0 {
            self.end = (buffer.idx + 1).min(buffer.len);
        }

        if (flags & REARRANGEMENT_VERB) != 0 && self.start < self.end {
            /* The following map has two nibbles, for start-side
             * and end-side. Values of 0,1,2 mean move that many
             * to the other side. Value of 3 means move 2 and
             * flip them. */
            const MAP: [u8; 16] = [
                0x00, /* 0	no change */
                0x10, /* 1	Ax => xA */
                0x01, /* 2	xD => Dx */
                0x11, /* 3	AxD => DxA */
                0x20, /* 4	ABx => xAB */
                0x30, /* 5	ABx => xBA */
                0x02, /* 6	xCD => CDx */
                0x03, /* 7	xCD => DCx */
                0x12, /* 8	AxCD => CDxA */
                0x13, /* 9	AxCD => DCxA */
                0x21, /* 10	ABxD => DxAB */
                0x31, /* 11	ABxD => DxBA */
                0x22, /* 12	ABxCD => CDxAB */
                0x32, /* 13	ABxCD => CDxBA */
                0x23, /* 14	ABxCD => DCxAB */
                0x33, /* 15	ABxCD => DCxBA */
            ];

            let m = MAP[(flags & REARRANGEMENT_VERB) as usize] as u32;
            let l = 2u32.min(m >> 4);
            let r = 2u32.min(m & 0x0F);
            let reverse_l = 3 == (m >> 4);
            let reverse_r = 3 == (m & 0x0F);

            let (start, end) = (self.start, self.end);
            if end - start >= l + r && end - start <= HB_MAX_CONTEXT_LENGTH {
                buffer.merge_clusters(start, (buffer.idx + 1).min(buffer.len));
                buffer.merge_clusters(start, end);

                let info = &mut buffer.info;
                let (start, end, l, r) = (start as usize, end as usize, l as usize, r as usize);
                let mut buf = [super::hb_buffer::HbGlyphInfo::default(); 4];

                buf[..l].copy_from_slice(&info[start..start + l]);
                buf[2..2 + r].copy_from_slice(&info[end - r..end]);

                if l != r {
                    info.copy_within(start + l..start + l + (end - start - l - r), start + r);
                }

                info[start..start + r].copy_from_slice(&buf[2..2 + r]);
                info[end - l..end].copy_from_slice(&buf[..l]);
                if reverse_l {
                    info.swap(end - 1, end - 2);
                }
                if reverse_r {
                    info.swap(start, start + 1);
                }
            }
        }
    }
}

/// `RearrangementSubtable::apply`: the subtable at `p`.
fn rearrangement_apply(c: &mut HbAatApplyContext, d: &[u8], p: usize, extended: bool) -> bool {
    let mut dc = RearrangementDriverContext {
        ret: false,
        start: 0,
        end: 0,
    };

    let machine = StateTable::new(d, p, extended, 0);
    let num_glyphs = c.face.get_num_glyphs();
    drive(&machine, num_glyphs, &mut dc, c);

    dc.ret
}

/// `RearrangementSubtable::sanitize`
fn rearrangement_sanitize(c: &mut HbSanitizeContext, p: usize, extended: bool) -> bool {
    StateTable::sanitize(c, p, extended, 0, None)
}

/* ContextualSubtable */

/* EntryData: markIndex (Index of the substitution table for the marked
 * glyph (use 0xFFFF for none)), currentIndex (Index of the substitution
 * table for the current glyph (use 0xFFFF for none)). */
const CONTEXTUAL_ENTRY_DATA_SIZE: usize = 4;

/* Flags */
const CONTEXTUAL_SET_MARK: u16 = 0x8000; /* If set, make the current glyph the marked glyph. */
const CONTEXTUAL_DONT_ADVANCE: u16 = 0x4000; /* If set, don't advance to the next glyph before
                                              * going to the new state. */
const CONTEXTUAL_RESERVED: u16 = 0x3FFF; /* These bits are reserved and should be set to 0. */

/// `ContextualSubtable::driver_context_t`
struct ContextualDriverContext<'a> {
    ret: bool,
    mark_set: bool,
    has_glyph_classes: bool,
    mark: u32,
    extended: bool,
    d: &'a [u8],
    /// `table`
    table: usize,
    /// `subs` (`table+table->substitutionTables`)
    subs: usize,
}

impl ContextualDriverContext<'_> {
    /// The replacement of `glyph` by the substitution table `index`
    /// (C's `replacement`).
    fn replacement(
        &self,
        c: &mut HbAatApplyContext,
        num_glyphs: u32,
        index: u16,
        glyph: HbCodepoint,
    ) -> Option<HbCodepoint> {
        let d = self.d;
        if self.extended {
            if index != 0xFFFF {
                /* subs[index]: UnsizedListOfOffset16To::operator [] */
                let p = self.subs + index as usize * 4;
                let lookup = Lookup::new(d, self.subs.wrapping_add(u32_at(d, p) as usize), 2);
                return lookup
                    .get_value(glyph, num_glyphs)
                    .map(|v| u16_at(d, v) as u32);
            }
            None
        } else {
            let offset = (index as u32).wrapping_add(glyph);
            let i = word_offset_to_index(false, offset, (self.subs - self.table) as u32, 2);
            let replacement = self.subs.wrapping_add(i as usize * 2);
            if !(c.sanitizer.check_struct(replacement, 2) && u16_at(d, replacement) != 0) {
                return None;
            }
            Some(u16_at(d, replacement) as u32)
        }
    }
}

impl DriverContext for ContextualDriverContext<'_> {
    const IN_PLACE: bool = true;
    const DONT_ADVANCE: u16 = CONTEXTUAL_DONT_ADVANCE;

    fn is_actionable(&self, buffer: &HbBuffer, entry: &Entry) -> bool {
        if buffer.idx == buffer.len && !self.mark_set {
            return false;
        }

        entry.data[0] != 0xFFFF || entry.data[1] != 0xFFFF
    }

    fn transition(&mut self, c: &mut HbAatApplyContext, num_glyphs: u32, entry: &Entry) {
        /* Looks like CoreText applies neither mark nor current substitution for
         * end-of-text if mark was not explicitly set. */
        if c.buffer.idx == c.buffer.len && !self.mark_set {
            return;
        }

        let (mark_index, current_index) = (entry.data[0], entry.data[1]);
        let gdef = c.gdef_table;

        let mark_glyph = c.buffer.info[self.mark as usize].codepoint;
        if let Some(replacement) = self.replacement(c, num_glyphs, mark_index, mark_glyph) {
            let buffer = &mut *c.buffer;
            buffer.unsafe_to_break(self.mark, (buffer.idx + 1).min(buffer.len));
            buffer.info[self.mark as usize].codepoint = replacement;
            if self.has_glyph_classes {
                _hb_glyph_info_set_glyph_props(
                    &mut buffer.info[self.mark as usize],
                    gdef.get_glyph_props(replacement),
                );
            }
            self.ret = true;
        }

        let idx = c.buffer.idx.min(c.buffer.len.wrapping_sub(1)) as usize;
        let glyph = c.buffer.info[idx].codepoint;
        if let Some(replacement) = self.replacement(c, num_glyphs, current_index, glyph) {
            let buffer = &mut *c.buffer;
            buffer.info[idx].codepoint = replacement;
            if self.has_glyph_classes {
                _hb_glyph_info_set_glyph_props(
                    &mut buffer.info[idx],
                    gdef.get_glyph_props(replacement),
                );
            }
            self.ret = true;
        }

        if entry.flags & CONTEXTUAL_SET_MARK != 0 {
            self.mark_set = true;
            self.mark = c.buffer.idx;
        }
    }
}

/// `ContextualSubtable::apply`
fn contextual_apply(c: &mut HbAatApplyContext, d: &[u8], p: usize, extended: bool) -> bool {
    let has_glyph_classes = c.gdef_table.has_glyph_classes();
    let subs_off = hbuint_at(d, p + StateTable::static_size(extended), extended) as usize;
    let mut dc = ContextualDriverContext {
        ret: false,
        mark_set: false,
        has_glyph_classes,
        mark: 0,
        extended,
        d,
        table: p,
        subs: p + subs_off,
    };

    let machine = StateTable::new(d, p, extended, CONTEXTUAL_ENTRY_DATA_SIZE);
    let num_glyphs = c.face.get_num_glyphs();
    drive(&machine, num_glyphs, &mut dc, c);

    dc.ret
}

/// `ContextualSubtable::sanitize`
fn contextual_sanitize(c: &mut HbSanitizeContext, p: usize, extended: bool) -> bool {
    let mut num_entries = 0;
    if !StateTable::sanitize(
        c,
        p,
        extended,
        CONTEXTUAL_ENTRY_DATA_SIZE,
        Some(&mut num_entries),
    ) {
        return false;
    }

    let field = p + StateTable::static_size(extended);
    let size = hbuint_size(extended);
    /* substitutionTables.sanitize (c, this, num_lookups): the
     * UnsizedListOfOffset16To's offsets, from the list itself */
    let substitution_tables_sanitize = |c: &mut HbSanitizeContext, num_lookups: u32| -> bool {
        if !c.check_struct(field, size) {
            return false;
        }
        let subs = p + hbuint_at(&c.data, field, extended) as usize;
        if !c.check_array(subs, num_lookups, size as u32) {
            return false;
        }
        for i in 0..num_lookups as usize {
            let q = subs + i * size;
            if !c.check_struct(q, size) {
                return false;
            }
            let off = hbuint_at(&c.data, q, extended) as usize;
            if !Lookup::sanitize(c, subs + off, 2) {
                return false;
            }
        }
        true
    };

    if !extended {
        return substitution_tables_sanitize(c, 0);
    }

    let mut num_lookups: u32 = 0;

    let machine = StateTable::new(&c.data, p, extended, CONTEXTUAL_ENTRY_DATA_SIZE);
    for i in 0..num_entries {
        let data = machine.get_entries(i).data;

        if data[0] != 0xFFFF {
            num_lookups = num_lookups.max(1 + data[0] as u32);
        }
        if data[1] != 0xFFFF {
            num_lookups = num_lookups.max(1 + data[1] as u32);
        }
    }

    substitution_tables_sanitize(c, num_lookups)
}

/* LigatureSubtable */

/* LigatureEntry<true>::Flags */
const LIGATURE_SET_COMPONENT: u16 = 0x8000; /* Push this glyph onto the component stack for
                                             * eventual processing. */
const LIGATURE_DONT_ADVANCE: u16 = 0x4000; /* Leave the glyph pointer at this glyph for the
                                           next iteration. */
const LIGATURE_PERFORM_ACTION: u16 = 0x2000; /* Use the ligActionIndex to process a ligature
                                              * group. */
const LIGATURE_RESERVED: u16 = 0x1FFF; /* These bits are reserved and should be set to 0. */
/* LigatureEntry<false>::Flags */
const LIGATURE_OFFSET: u16 = 0x3FFF; /* Byte offset from beginning of subtable to the
                                      * ligature action list. This value must be a
                                      * multiple of 4. */

/// `LigatureEntry<extended>::performAction`
#[inline]
fn ligature_perform_action(extended: bool, entry: &Entry) -> bool {
    if extended {
        entry.flags & LIGATURE_PERFORM_ACTION != 0
    } else {
        entry.flags & LIGATURE_OFFSET != 0
    }
}

/// `LigatureEntry<extended>::ligActionIndex`
#[inline]
fn ligature_lig_action_index(extended: bool, entry: &Entry) -> u32 {
    if extended {
        entry.data[0] as u32
    } else {
        (entry.flags & LIGATURE_OFFSET) as u32
    }
}

/* LigActionFlags */
const LIG_ACTION_LAST: u32 = 0x80000000; /* This is the last action in the list. This also
                                          * implies storage. */
const LIG_ACTION_STORE: u32 = 0x40000000; /* Store the ligature at the current cumulated index
                                           * in the ligature table in place of the marked
                                           * (i.e. currently-popped) glyph. */
const LIG_ACTION_OFFSET: u32 = 0x3FFFFFFF; /* A 30-bit value which is sign-extended to 32-bits
                                            * and added to the glyph ID, resulting in an index
                                            * into the component table. */

/// `LigatureSubtable::driver_context_t`
struct LigatureDriverContext<'a> {
    ret: bool,
    extended: bool,
    d: &'a [u8],
    table: usize,
    /// `ligAction` (`table+table->ligAction`)
    lig_action: usize,
    /// `component`
    component: usize,
    /// `ligature`
    ligature: usize,
    match_length: u32,
    match_positions: [u32; HB_MAX_CONTEXT_LENGTH as usize],
}

impl DriverContext for LigatureDriverContext<'_> {
    const IN_PLACE: bool = false;
    const DONT_ADVANCE: u16 = LIGATURE_DONT_ADVANCE;

    fn is_actionable(&self, _buffer: &HbBuffer, entry: &Entry) -> bool {
        ligature_perform_action(self.extended, entry)
    }

    fn transition(&mut self, c: &mut HbAatApplyContext, _num_glyphs: u32, entry: &Entry) {
        const N: u32 = HB_MAX_CONTEXT_LENGTH;
        let d = self.d;
        let extended = self.extended;
        let buffer = &mut *c.buffer;
        if entry.flags & LIGATURE_SET_COMPONENT != 0 {
            /* Never mark same index twice, in case DontAdvance was used... */
            if self.match_length != 0
                && self.match_positions[((self.match_length - 1) % N) as usize] == buffer.out_len
            {
                self.match_length -= 1;
            }

            self.match_positions[(self.match_length % N) as usize] = buffer.out_len;
            self.match_length = self.match_length.wrapping_add(1);
        }

        if ligature_perform_action(extended, entry) {
            let end = buffer.out_len;

            if self.match_length == 0 {
                return;
            }

            if buffer.idx >= buffer.len {
                return; /* TODO Work on previous instead? */
            }

            let mut cursor = self.match_length;

            let mut action_idx = ligature_lig_action_index(extended, entry);
            action_idx = offset_to_index(
                extended,
                action_idx,
                (self.lig_action - self.table) as u32,
                4,
            );
            let mut action_data = self.lig_action.wrapping_add(action_idx as usize * 4);

            let mut ligature_idx: u32 = 0;
            loop {
                if cursor == 0 {
                    /* Stack underflow.  Clear the stack. */
                    self.match_length = 0;
                    break;
                }

                cursor -= 1;
                if !buffer.move_to(self.match_positions[(cursor % N) as usize]) {
                    return;
                }

                if !c.sanitizer.check_struct(action_data, 4) {
                    break;
                }
                let action = u32_at(d, action_data);

                let mut uoffset = action & LIG_ACTION_OFFSET;
                if uoffset & 0x20000000 != 0 {
                    uoffset |= 0xC0000000; /* Sign-extend. */
                }
                let offset = uoffset as i32;
                let mut component_idx = buffer.cur(0).codepoint.wrapping_add(offset as u32);
                component_idx = word_offset_to_index(
                    extended,
                    component_idx,
                    (self.component - self.table) as u32,
                    2,
                );
                let component_data = self.component.wrapping_add(component_idx as usize * 2);
                if !c.sanitizer.check_struct(component_data, 2) {
                    break;
                }
                ligature_idx = ligature_idx.wrapping_add(u16_at(d, component_data) as u32);

                if action & (LIG_ACTION_STORE | LIG_ACTION_LAST) != 0 {
                    ligature_idx = offset_to_index(
                        extended,
                        ligature_idx,
                        (self.ligature - self.table) as u32,
                        2,
                    );
                    let ligature_data = self.ligature.wrapping_add(ligature_idx as usize * 2);
                    if !c.sanitizer.check_struct(ligature_data, 2) {
                        break;
                    }
                    let lig = u16_at(d, ligature_data) as u32;

                    if !buffer.replace_glyph(lig) {
                        return;
                    }

                    let lig_end = self.match_positions
                        [(self.match_length.wrapping_sub(1) % N) as usize]
                        .wrapping_add(1);
                    /* Now go and delete all subsequent components. */
                    while self.match_length.wrapping_sub(1) > cursor {
                        self.match_length -= 1;
                        if !buffer.move_to(self.match_positions[(self.match_length % N) as usize]) {
                            return;
                        }
                        let info = buffer.cur_mut(0);
                        info.set_unicode_props(info.unicode_props() | UPROPS_MASK_IGNORABLE);
                        if !buffer.replace_glyph(DELETED_GLYPH) {
                            return;
                        }
                    }

                    if !buffer.move_to(lig_end) {
                        return;
                    }
                    let out_len = buffer.out_len;
                    buffer.merge_out_clusters(self.match_positions[(cursor % N) as usize], out_len);
                }

                action_data = action_data.wrapping_add(4);

                if action & LIG_ACTION_LAST != 0 {
                    break;
                }
            }
            let _ = buffer.move_to(end);
        }
    }
}

/// `LigatureSubtable::apply`
fn ligature_apply(c: &mut HbAatApplyContext, d: &[u8], p: usize, extended: bool) -> bool {
    let fields = p + StateTable::static_size(extended);
    let size = hbuint_size(extended);
    let mut dc = LigatureDriverContext {
        ret: false,
        extended,
        d,
        table: p,
        lig_action: p + hbuint_at(d, fields, extended) as usize,
        component: p + hbuint_at(d, fields + size, extended) as usize,
        ligature: p + hbuint_at(d, fields + 2 * size, extended) as usize,
        match_length: 0,
        match_positions: [0; HB_MAX_CONTEXT_LENGTH as usize],
    };

    let machine = StateTable::new(d, p, extended, if extended { 2 } else { 0 });
    let num_glyphs = c.face.get_num_glyphs();
    drive(&machine, num_glyphs, &mut dc, c);

    dc.ret
}

/// `LigatureSubtable::sanitize`
fn ligature_sanitize(c: &mut HbSanitizeContext, p: usize, extended: bool) -> bool {
    /* The rest of array sanitizations are done at run-time. */
    let size = hbuint_size(extended);
    let fields = p + StateTable::static_size(extended);
    c.check_struct(p, StateTable::static_size(extended) + 3 * size)
        && StateTable::sanitize(c, p, extended, if extended { 2 } else { 0 }, None)
        && hbuint_at(&c.data, fields, extended) != 0
        && hbuint_at(&c.data, fields + size, extended) != 0
        && hbuint_at(&c.data, fields + 2 * size, extended) != 0
}

/* NoncontextualSubtable */

/// `NoncontextualSubtable::apply`
fn noncontextual_apply(c: &mut HbAatApplyContext, d: &[u8], p: usize) -> bool {
    let gdef = c.gdef_table;
    let has_glyph_classes = gdef.has_glyph_classes();

    let mut ret = false;
    let num_glyphs = c.face.get_num_glyphs();

    let substitute = Lookup::new(d, p, 2);
    let subtable_flags = c.subtable_flags;
    let range_flags = c.range_flags;
    let info = &mut c.buffer.info;
    let count = c.buffer.len as usize;
    // If there's only one range, we already checked the flag.
    let mut last_range: Option<usize> = match range_flags {
        Some(r) if r.len() > 1 => Some(0),
        _ => None,
    };
    for i in 0..count {
        /* This block copied from StateTableDriver::drive. Keep in sync. */
        if let (Some(lr), Some(rf)) = (last_range, range_flags) {
            let mut range = lr;
            {
                let cluster = info[i].cluster;
                while cluster < rf[range].cluster_first {
                    range -= 1;
                }
                while cluster > rf[range].cluster_last {
                    range += 1;
                }

                last_range = Some(range);
            }
            if rf[range].flags & subtable_flags == 0 {
                continue;
            }
        }

        if let Some(v) = substitute.get_value(info[i].codepoint, num_glyphs) {
            let replacement = u16_at(d, v) as u32;
            info[i].codepoint = replacement;
            if has_glyph_classes {
                _hb_glyph_info_set_glyph_props(&mut info[i], gdef.get_glyph_props(replacement));
            }
            ret = true;
        }
    }

    ret
}

/// `NoncontextualSubtable::sanitize`
fn noncontextual_sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
    Lookup::sanitize(c, p, 2)
}

/* InsertionSubtable */

/* EntryData: currentInsertIndex (Zero-based index into the insertion glyph table.
 * The number of glyphs to be inserted is contained
 * in the currentInsertCount field in the flags.
 * A value of 0xFFFF indicates no insertion is to
 * be done.), markedInsertIndex (Zero-based index into the insertion glyph table.
 * The number of glyphs to be inserted is contained
 * in the markedInsertCount field in the flags.
 * A value of 0xFFFF indicates no insertion is to
 * be done.) */
const INSERTION_ENTRY_DATA_SIZE: usize = 4;

/* Flags */
const INSERTION_SET_MARK: u16 = 0x8000; /* If set, mark the current glyph. */
const INSERTION_DONT_ADVANCE: u16 = 0x4000; /* If set, don't advance to the next glyph before
                                             * going to the new state.  This does not mean
                                             * that the glyph pointed to is the same one as
                                             * before. If you've made insertions immediately
                                             * downstream of the current glyph, the next glyph
                                             * processed would in fact be the first one
                                             * inserted. */
const INSERTION_CURRENT_IS_KASHIDA_LIKE: u16 = 0x2000; /* If set, and the currentInsertList is nonzero,
                                                        * then the specified glyph list will be inserted
                                                        * as a kashida-like insertion, either before or
                                                        * after the current glyph (depending on the state
                                                        * of the currentInsertBefore flag). If clear, and
                                                        * the currentInsertList is nonzero, then the
                                                        * specified glyph list will be inserted as a
                                                        * split-vowel-like insertion, either before or
                                                        * after the current glyph (depending on the state
                                                        * of the currentInsertBefore flag). */
const INSERTION_MARKED_IS_KASHIDA_LIKE: u16 = 0x1000; /* If set, and the markedInsertList is nonzero,
                                                       * then the specified glyph list will be inserted
                                                       * as a kashida-like insertion, either before or
                                                       * after the marked glyph (depending on the state
                                                       * of the markedInsertBefore flag). If clear, and
                                                       * the markedInsertList is nonzero, then the
                                                       * specified glyph list will be inserted as a
                                                       * split-vowel-like insertion, either before or
                                                       * after the marked glyph (depending on the state
                                                       * of the markedInsertBefore flag). */
const INSERTION_CURRENT_INSERT_BEFORE: u16 = 0x0800; /* If set, specifies that insertions are to be made
                                                      * to the left of the current glyph. If clear,
                                                      * they're made to the right of the current glyph. */
const INSERTION_MARKED_INSERT_BEFORE: u16 = 0x0400; /* If set, specifies that insertions are to be
                                                     * made to the left of the marked glyph. If clear,
                                                     * they're made to the right of the marked glyph. */
const INSERTION_CURRENT_INSERT_COUNT: u16 = 0x3E0; /* This 5-bit field is treated as a count of the
                                                    * number of glyphs to insert at the current
                                                    * position. Since zero means no insertions, the
                                                    * largest number of insertions at any given
                                                    * current location is 31 glyphs. */
const INSERTION_MARKED_INSERT_COUNT: u16 = 0x001F; /* This 5-bit field is treated as a count of the
                                                    * number of glyphs to insert at the marked
                                                    * position. Since zero means no insertions, the
                                                    * largest number of insertions at any given
                                                    * marked location is 31 glyphs. */

/// `InsertionSubtable::driver_context_t`
struct InsertionDriverContext<'a> {
    ret: bool,
    d: &'a [u8],
    mark: u32,
    /// `insertionAction` (`table+table->insertionAction`)
    insertion_action: usize,
}

impl InsertionDriverContext<'_> {
    /// The `count` glyphs at `insertionAction[start]` (none if they are
    /// out of range).
    fn glyphs(&self, c: &mut HbAatApplyContext, start: u16, count: u32) -> Vec<HbCodepoint> {
        let glyphs = self.insertion_action.wrapping_add(start as usize * 2);
        let count = if !c.sanitizer.check_array(glyphs, count, 2) {
            0
        } else {
            count
        };
        (0..count as usize)
            .map(|i| u16_at(self.d, glyphs + i * 2) as u32)
            .collect()
    }
}

impl DriverContext for InsertionDriverContext<'_> {
    const IN_PLACE: bool = false;
    const DONT_ADVANCE: u16 = INSERTION_DONT_ADVANCE;

    fn is_actionable(&self, _buffer: &HbBuffer, entry: &Entry) -> bool {
        (entry.flags & (INSERTION_CURRENT_INSERT_COUNT | INSERTION_MARKED_INSERT_COUNT)) != 0
            && (entry.data[0] != 0xFFFF || entry.data[1] != 0xFFFF)
    }

    fn transition(&mut self, c: &mut HbAatApplyContext, _num_glyphs: u32, entry: &Entry) {
        let flags = entry.flags;
        let (current_insert_index, marked_insert_index) = (entry.data[0], entry.data[1]);

        let mark_loc = c.buffer.out_len;

        if marked_insert_index != 0xFFFF {
            let count = (flags & INSERTION_MARKED_INSERT_COUNT) as u32;
            c.buffer.max_ops = c.buffer.max_ops.wrapping_sub(count as i32);
            if c.buffer.max_ops <= 0 {
                return;
            }
            let glyphs = self.glyphs(c, marked_insert_index, count);
            let count = glyphs.len() as u32;
            let buffer = &mut *c.buffer;

            let before = flags & INSERTION_MARKED_INSERT_BEFORE != 0;

            let end = buffer.out_len;
            if !buffer.move_to(self.mark) {
                return;
            }

            if buffer.idx < buffer.len && !before && !buffer.copy_glyph() {
                return;
            }
            /* TODO We ignore KashidaLike setting. */
            if !buffer.replace_glyphs(0, count, &glyphs) {
                return;
            }
            if buffer.idx < buffer.len && !before {
                buffer.skip_glyph();
            }

            if !buffer.move_to(end + count) {
                return;
            }

            let end = (buffer.idx + 1).min(buffer.len);
            buffer.unsafe_to_break_from_outbuffer(self.mark, end);
        }

        if flags & INSERTION_SET_MARK != 0 {
            self.mark = mark_loc;
        }

        if current_insert_index != 0xFFFF {
            let count = ((flags & INSERTION_CURRENT_INSERT_COUNT) >> 5) as u32;
            c.buffer.max_ops = c.buffer.max_ops.wrapping_sub(count as i32);
            if c.buffer.max_ops <= 0 {
                return;
            }
            let glyphs = self.glyphs(c, current_insert_index, count);
            let count = glyphs.len() as u32;
            let buffer = &mut *c.buffer;

            let before = flags & INSERTION_CURRENT_INSERT_BEFORE != 0;

            let end = buffer.out_len;

            if buffer.idx < buffer.len && !before && !buffer.copy_glyph() {
                return;
            }
            /* TODO We ignore KashidaLike setting. */
            if !buffer.replace_glyphs(0, count, &glyphs) {
                return;
            }
            if buffer.idx < buffer.len && !before {
                buffer.skip_glyph();
            }

            /* Humm. Not sure where to move to.  There's this wording under
             * DontAdvance flag:
             *
             * "If set, don't update the glyph index before going to the new state.
             * This does not mean that the glyph pointed to is the same one as
             * before. If you've made insertions immediately downstream of the
             * current glyph, the next glyph processed would in fact be the first
             * one inserted."
             *
             * This suggests that if DontAdvance is NOT set, we should move to
             * end+count.  If it *was*, then move to end, such that newly inserted
             * glyphs are now visible.
             *
             * https://github.com/harfbuzz/harfbuzz/issues/1224#issuecomment-427691417
             */
            let _ = buffer.move_to(if flags & INSERTION_DONT_ADVANCE != 0 {
                end
            } else {
                end + count
            });
        }
    }
}

/// `InsertionSubtable::apply`
fn insertion_apply(c: &mut HbAatApplyContext, d: &[u8], p: usize, extended: bool) -> bool {
    let field = p + StateTable::static_size(extended);
    let mut dc = InsertionDriverContext {
        ret: false,
        d,
        mark: 0,
        insertion_action: p + hbuint_at(d, field, extended) as usize,
    };

    let machine = StateTable::new(d, p, extended, INSERTION_ENTRY_DATA_SIZE);
    let num_glyphs = c.face.get_num_glyphs();
    drive(&machine, num_glyphs, &mut dc, c);

    dc.ret
}

/// `InsertionSubtable::sanitize`
fn insertion_sanitize(c: &mut HbSanitizeContext, p: usize, extended: bool) -> bool {
    /* The rest of array sanitizations are done at run-time. */
    let field = p + StateTable::static_size(extended);
    c.check_struct(p, StateTable::static_size(extended) + hbuint_size(extended))
        && StateTable::sanitize(c, p, extended, INSERTION_ENTRY_DATA_SIZE, None)
        && hbuint_at(&c.data, field, extended) != 0
}

/* Feature (featureType u16, featureSetting u16, enableFlags u32,
 * disableFlags u32: 12 bytes) */

/* hb_aat_layout_chain_accelerator_t */

/// `hb_aat_layout_chain_accelerator_t`: the digests of the chain's
/// subtables (`hb_accelerate_subtables_context_t::hb_applicable_t`).
#[derive(Debug)]
pub(crate) struct ChainAccel {
    pub(crate) subtables: Vec<HbSetDigest>,
}

impl ChainAccel {
    /// `create`
    fn create(d: &[u8], chain: usize, extended: bool, num_glyphs: u32) -> ChainAccel {
        let count = chain_subtable_count(d, chain, extended);

        /* The following is a calloc because when we are collecting subtables,
         * some of them might be invalid and hence not collect; as a result,
         * we might not fill in all the count entries of the subtables array.
         * Zeroing it allows the set digest to gatekeep it without having to
         * initialize it further. */
        let mut subtables = vec![HbSetDigest::default(); count as usize];

        /* hb_accelerate_subtables_context_t c_accelerate_subtables (thiz->subtables, num_glyphs);
         * chain.dispatch (&c_accelerate_subtables); */
        /* FIXME (upstream): ChainSubtable::dispatch only calls the context
         * for the known subtable types, so the digests of the subtables
         * after one of an unknown type land one entry early (and the last
         * ones stay empty) (danielpclark/SDL#140). */
        let mut i = 0;
        let mut subtable = chain_first_subtable(d, chain, extended);
        for _ in 0..count {
            let body = subtable + chain_subtable_header_size(extended);
            let digest = match chain_subtable_type(d, subtable, extended) {
                /* (the subtables with a machine collect its glyphs) */
                REARRANGEMENT => Some(machine_digest(d, body, extended, 0, num_glyphs)),
                CONTEXTUAL => Some(machine_digest(
                    d,
                    body,
                    extended,
                    CONTEXTUAL_ENTRY_DATA_SIZE,
                    num_glyphs,
                )),
                LIGATURE => Some(machine_digest(
                    d,
                    body,
                    extended,
                    if extended { 2 } else { 0 },
                    num_glyphs,
                )),
                NONCONTEXTUAL => Some(HbSetDigest::full()),
                INSERTION => Some(machine_digest(
                    d,
                    body,
                    extended,
                    INSERTION_ENTRY_DATA_SIZE,
                    num_glyphs,
                )),
                _ => None,
            };
            if let Some(digest) = digest {
                if let Some(entry) = subtables.get_mut(i) {
                    *entry = digest;
                }
                i += 1;
            }
            subtable = subtable.wrapping_add(chain_subtable_length(d, subtable, extended) as usize);
        }

        ChainAccel { subtables }
    }
}

/// `hb_applicable_t::init_` of a subtable with a machine.
fn machine_digest(
    d: &[u8],
    body: usize,
    extended: bool,
    entry_data_size: usize,
    num_glyphs: u32,
) -> HbSetDigest {
    let mut digest = HbSetDigest::default();
    StateTable::new(d, body, extended, entry_data_size).collect_glyphs(&mut digest, num_glyphs);
    digest
}

/* ChainSubtable */

/* Coverage */
const COVERAGE_VERTICAL: u32 = 0x80; /* If set, this subtable will only be applied
                                      * to vertical text. If clear, this subtable
                                      * will only be applied to horizontal text. */
const COVERAGE_BACKWARDS: u32 = 0x40; /* If set, this subtable will process glyphs
                                       * in descending order. If clear, it will
                                       * process the glyphs in ascending order. */
const COVERAGE_ALL_DIRECTIONS: u32 = 0x20; /* If set, this subtable will be applied to
                                            * both horizontal and vertical text (i.e.
                                            * the state of bit 0x80000000 is ignored). */
const COVERAGE_LOGICAL: u32 = 0x10; /* If set, this subtable will process glyphs
                                     * in logical order (or reverse logical order,
                                     * depending on the value of bit 0x80000000). */
/* Type */
const REARRANGEMENT: u32 = 0;
const CONTEXTUAL: u32 = 1;
const LIGATURE: u32 = 2;
const NONCONTEXTUAL: u32 = 4;
const INSERTION: u32 = 5;

/// `ChainSubtable::static_size` of the header (`length`, `coverage`,
/// `subFeatureFlags`).
#[inline]
fn chain_subtable_header_size(extended: bool) -> usize {
    2 * hbuint_size(extended) + 4
}
/// `get_size`
#[inline]
fn chain_subtable_length(d: &[u8], st: usize, extended: bool) -> u32 {
    hbuint_at(d, st, extended)
}
/// `coverage`
#[inline]
fn chain_subtable_coverage_field(d: &[u8], st: usize, extended: bool) -> u32 {
    hbuint_at(d, st + hbuint_size(extended), extended)
}
/// `get_type`
#[inline]
fn chain_subtable_type(d: &[u8], st: usize, extended: bool) -> u32 {
    chain_subtable_coverage_field(d, st, extended) & 0xFF
}
/// `get_coverage`
#[inline]
fn chain_subtable_get_coverage(d: &[u8], st: usize, extended: bool) -> u32 {
    chain_subtable_coverage_field(d, st, extended) >> (hbuint_size(extended) * 8 - 8)
}
/// `subFeatureFlags`
#[inline]
fn chain_subtable_sub_feature_flags(d: &[u8], st: usize, extended: bool) -> u32 {
    u32_at(d, st + 2 * hbuint_size(extended))
}

/// `ChainSubtable::apply` (`dispatch (c)`)
fn chain_subtable_apply(c: &mut HbAatApplyContext, d: &[u8], st: usize, extended: bool) -> bool {
    // Disabled for https://github.com/harfbuzz/harfbuzz/issues/4873
    //hb_sanitize_with_object_t with (&c->sanitizer, this);
    let body = st + chain_subtable_header_size(extended);
    match chain_subtable_type(d, st, extended) {
        REARRANGEMENT => rearrangement_apply(c, d, body, extended),
        CONTEXTUAL => contextual_apply(c, d, body, extended),
        LIGATURE => ligature_apply(c, d, body, extended),
        NONCONTEXTUAL => noncontextual_apply(c, d, body),
        INSERTION => insertion_apply(c, d, body, extended),
        _ => false,
    }
}

/// `ChainSubtable::sanitize`
fn chain_subtable_sanitize(c: &mut HbSanitizeContext, st: usize, extended: bool) -> bool {
    let size = hbuint_size(extended);
    if !(c.check_struct(st, size) && {
        let length = hbuint_at(&c.data, st, extended);
        length >= chain_subtable_header_size(extended) as u32 && c.check_range(st, length)
    }) {
        return false;
    }

    // Disabled for https://github.com/harfbuzz/harfbuzz/issues/4873
    //hb_sanitize_with_object_t with (c, this);
    let body = st + chain_subtable_header_size(extended);
    match chain_subtable_type(&c.data, st, extended) {
        REARRANGEMENT => rearrangement_sanitize(c, body, extended),
        CONTEXTUAL => contextual_sanitize(c, body, extended),
        LIGATURE => ligature_sanitize(c, body, extended),
        NONCONTEXTUAL => noncontextual_sanitize(c, body),
        INSERTION => insertion_sanitize(c, body, extended),
        _ => true,
    }
}

/* Chain */

/// `Chain::min_size`
#[inline]
fn chain_min_size(extended: bool) -> usize {
    8 + 2 * hbuint_size(extended)
}
/// `defaultFlags`
#[inline]
fn chain_default_flags(d: &[u8], chain: usize) -> u32 {
    u32_at(d, chain)
}
/// `get_size`
#[inline]
fn chain_length(d: &[u8], chain: usize) -> u32 {
    u32_at(d, chain + 4)
}
/// `featureCount`
#[inline]
fn chain_feature_count(d: &[u8], chain: usize, extended: bool) -> u32 {
    hbuint_at(d, chain + 8, extended)
}
/// `get_subtable_count`
#[inline]
fn chain_subtable_count(d: &[u8], chain: usize, extended: bool) -> u32 {
    hbuint_at(d, chain + 8 + hbuint_size(extended), extended)
}
/// `featureZ`
#[inline]
fn chain_features(chain: usize, extended: bool) -> usize {
    chain + chain_min_size(extended)
}
/// `&StructAfter<ChainSubtable<Types>> (featureZ.as_array (featureCount))`
#[inline]
fn chain_first_subtable(d: &[u8], chain: usize, extended: bool) -> usize {
    chain_features(chain, extended)
        .wrapping_add(chain_feature_count(d, chain, extended) as usize * 12)
}

/// `Chain::compile_flags`
fn chain_compile_flags(d: &[u8], chain: usize, extended: bool, map: &HbAatMapBuilder) -> HbMask {
    let mut flags = chain_default_flags(d, chain);
    {
        let count = chain_feature_count(d, chain, extended);
        let features = chain_features(chain, extended);
        for i in 0..count as usize {
            let feature = features + i * 12;
            let mut type_ = u16_at(d, feature) as u32;
            let mut setting = u16_at(d, feature + 2) as u32;
            let enable_flags = u32_at(d, feature + 4);
            let disable_flags = u32_at(d, feature + 8);
            loop {
                /* retry: */
                // Check whether this type/setting pair was requested in the map, and if so, apply its flags.
                // (The search here only looks at the type and setting fields of feature_info_t.)
                let info = FeatureInfo {
                    type_,
                    setting,
                    is_exclusive: false,
                    seq: 0,
                };
                if map.current_features_bsearch(&info) {
                    flags &= disable_flags;
                    flags |= enable_flags;
                } else if type_ == HB_AAT_LAYOUT_FEATURE_TYPE_LETTER_CASE
                    && setting == HB_AAT_LAYOUT_FEATURE_SELECTOR_SMALL_CAPS
                {
                    /* Deprecated. https://github.com/harfbuzz/harfbuzz/issues/1342 */
                    type_ = HB_AAT_LAYOUT_FEATURE_TYPE_LOWER_CASE;
                    setting = HB_AAT_LAYOUT_FEATURE_SELECTOR_LOWER_CASE_SMALL_CAPS;
                    continue;
                } else if type_ == HB_AAT_LAYOUT_FEATURE_TYPE_LANGUAGE_TAG_TYPE
                    && setting != 0
                    /* TODO: Rudimentary language matching. */
                    && hb_language_matches(
                        map.face.ltag().get_language(setting - 1),
                        map.props.language,
                    )
                {
                    flags &= disable_flags;
                    flags |= enable_flags;
                }
                break;
            }
        }
    }
    flags
}

/// `Chain::apply`
fn chain_apply(
    c: &mut HbAatApplyContext,
    d: &[u8],
    chain: usize,
    extended: bool,
    accel: Option<&ChainAccel>,
) {
    let mut subtable = chain_first_subtable(d, chain, extended);
    let count = chain_subtable_count(d, chain, extended);
    for i in 0..count as usize {
        'skip: {
            let sub_feature_flags = chain_subtable_sub_feature_flags(d, subtable, extended);
            if !c
                .range_flags
                .unwrap_or(&[])
                .iter()
                .any(|f| sub_feature_flags & f.flags != 0)
            {
                break 'skip;
            }
            c.subtable_flags = sub_feature_flags;
            c.machine_glyph_set = match accel {
                Some(accel) => accel.subtables.get(i).copied().unwrap_or_default(),
                None => HbSetDigest::full(),
            };

            let coverage = chain_subtable_get_coverage(d, subtable, extended);
            if (coverage & COVERAGE_ALL_DIRECTIONS) == 0
                && hb_direction_is_vertical(c.buffer.props.direction)
                    != ((coverage & COVERAGE_VERTICAL) != 0)
            {
                break 'skip;
            }

            /* Buffer contents is always in logical direction.  Determine if
            * we need to reverse before applying this subtable.  We reverse
            * back after if we did reverse indeed.
            *
            * Quoting the spac:
            * """
            * Bits 28 and 30 of the coverage field control the order in which
            * glyphs are processed when the subtable is run by the layout engine.
            * Bit 28 is used to indicate if the glyph processing direction is
            * the same as logical order or layout order. Bit 30 is used to
            * indicate whether glyphs are processed forwards or backwards within
            * that order.

                   Bit 30	Bit 28	Interpretation for Horizontal Text
                   0	0	The subtable is processed in layout order
                                   (the same order as the glyphs, which is
                                   always left-to-right).
                   1	0	The subtable is processed in reverse layout order
                                   (the order opposite that of the glyphs, which is
                                   always right-to-left).
                   0	1	The subtable is processed in logical order
                                   (the same order as the characters, which may be
                                   left-to-right or right-to-left).
                   1	1	The subtable is processed in reverse logical order
                                   (the order opposite that of the characters, which
                                   may be right-to-left or left-to-right).
            */
            let reverse = if coverage & COVERAGE_LOGICAL != 0 {
                coverage & COVERAGE_BACKWARDS != 0
            } else {
                (coverage & COVERAGE_BACKWARDS != 0)
                    != hb_direction_is_backward(c.buffer.props.direction)
            };

            /* (buffer->message: no message function) */

            if reverse {
                c.buffer.reverse();
            }

            chain_subtable_apply(c, d, subtable, extended);

            if reverse {
                c.buffer.reverse();
            }

            if !c.buffer.successful {
                return;
            }
        }

        /* skip: */
        subtable = subtable.wrapping_add(chain_subtable_length(d, subtable, extended) as usize);
        c.set_lookup_index(c.lookup_index + 1);
    }
}

/// `Chain::sanitize`
fn chain_sanitize(c: &mut HbSanitizeContext, chain: usize, extended: bool, version: u32) -> bool {
    if !(c.check_struct(chain + 4, 4) && {
        let length = chain_length(&c.data, chain);
        length >= chain_min_size(extended) as u32 && c.check_range(chain, length)
    }) {
        return false;
    }

    let feature_count = chain_feature_count(&c.data, chain, extended);
    if !c.check_array(chain_features(chain, extended), feature_count, 12) {
        return false;
    }

    let mut subtable = chain_first_subtable(&c.data, chain, extended);
    let count = chain_subtable_count(&c.data, chain, extended);
    for _ in 0..count {
        if !chain_subtable_sanitize(c, subtable, extended) {
            return false;
        }
        subtable += chain_subtable_length(&c.data, subtable, extended) as usize;
    }

    if version >= 3 {
        /* SubtableGlyphCoverage::sanitize (c, count) */
        if !subtable_glyph_coverage_sanitize(c, subtable, count) {
            return false;
        }
    }

    true
}

/*
 * The 'mort'/'morx' Table
 */

/// `mortmorx<T, Types, TAG>::accelerator_t`: the sanitized table and
/// its chains' accelerators.
#[derive(Debug)]
pub(crate) struct MortMorxAccel {
    pub(crate) table: Vec<u8>,
    /// `Types::extended` (`morx`)
    pub(crate) extended: bool,
    chain_count: u32,
    accels: Vec<OnceLock<ChainAccel>>,
}

impl MortMorxAccel {
    /// `accelerator_t (face)`
    pub(crate) fn new(face: &HbFace, extended: bool) -> MortMorxAccel {
        let tag = if extended {
            HB_AAT_TAG_MORX
        } else {
            HB_AAT_TAG_MORT
        };
        let table = hb_sanitize_blob(
            face.reference_table(tag),
            Some(face.get_num_glyphs()),
            |c| mortmorx_sanitize(c, extended),
        );

        let chain_count = u32_at(&table, 4); /* get_chain_count */

        let accels = (0..chain_count).map(|_| OnceLock::new()).collect();
        MortMorxAccel {
            table,
            extended,
            chain_count,
            accels,
        }
    }

    /// `get_blob`
    pub(crate) fn get_blob(&self) -> &[u8] {
        &self.table
    }

    /// `get_accel`
    fn get_accel(&self, chain_index: u32, chain: usize, num_glyphs: u32) -> Option<&ChainAccel> {
        if chain_index >= self.chain_count {
            return None;
        }

        Some(
            self.accels[chain_index as usize]
                .get_or_init(|| ChainAccel::create(&self.table, chain, self.extended, num_glyphs)),
        )
    }

    /// `has_data`
    pub(crate) fn has_data(&self) -> bool {
        u16_at(&self.table, 0) != 0
    }

    /// `compile_flags`
    pub(crate) fn compile_flags(&self, mapper: &HbAatMapBuilder, map: &mut HbAatMap) {
        let d = &self.table[..];
        let mut chain = 8; /* firstChain */
        let count = self.chain_count;
        map.chain_flags.resize(count as usize, Vec::new());
        for i in 0..count as usize {
            map.chain_flags[i].push(RangeFlags {
                flags: chain_compile_flags(d, chain, self.extended, mapper),
                cluster_first: mapper.range_first,
                cluster_last: mapper.range_last,
            });
            chain = chain.wrapping_add(chain_length(d, chain) as usize);
        }
    }

    /// `get_chain_count`
    pub(crate) fn get_chain_count(&self) -> u32 {
        self.chain_count
    }

    /// `apply`
    pub(crate) fn apply<'c>(&self, c: &mut HbAatApplyContext<'c, '_, '_>, map: &'c HbAatMap) {
        if !c.buffer.successful {
            return;
        }

        c.buffer.unsafe_to_concat(0, u32::MAX);

        c.set_lookup_index(0);
        let d = &self.table[..];
        let mut chain = 8; /* firstChain */
        let count = self.chain_count;
        for i in 0..count {
            let chain_accel = self.get_accel(i, chain, c.face.get_num_glyphs());
            c.range_flags = Some(
                map.chain_flags
                    .get(i as usize)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
            );
            chain_apply(c, d, chain, self.extended, chain_accel);
            if !c.buffer.successful {
                return;
            }
            chain = chain.wrapping_add(chain_length(d, chain) as usize);
        }
    }
}

/// `mortmorx::sanitize`
fn mortmorx_sanitize(c: &mut HbSanitizeContext, extended: bool) -> bool {
    if !(c.sanitize_u16(0) && c.u16(0) != 0 && c.check_struct(4, 4)) {
        return false;
    }

    let version = c.u16(0) as u32;
    let mut chain = 8; /* firstChain */
    let count = c.u32(4);
    for _ in 0..count {
        if !chain_sanitize(c, chain, extended, version) {
            return false;
        }
        chain += chain_length(&c.data, chain) as usize;
    }

    true
}
