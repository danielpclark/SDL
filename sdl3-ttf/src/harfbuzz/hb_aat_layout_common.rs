// Rust translation of src/hb-aat-layout-common.hh and of
// hb_aat_apply_context_t's constructor (src/hb-aat-layout.cc) from
// HarfBuzz (8.5.0, as SDL_ttf's external/harfbuzz pins it).
// Copyright © 2017  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod

//! The common structures of the AAT tables: the apply context, the lookup
//! tables, the (extended) state tables, their class tables, and the state
//! machine driver.
//!
//! Translation notes: as in [`super::hb_open_type`], a structure is its
//! position in the table's data (the blob, `d`); positions past the end
//! read as zeros (C reads them only after the sanitizer's checks, which
//! are the same here). The C++ templates over `ObsoleteTypes` (`mort`,
//! `kern`) and `ExtendedTypes` (`morx`, `kerx`) take an `extended` flag;
//! the state table's entry data (`Entry<Extra>`) is read into `Entry`
//! by its size. The driver contexts implement `DriverContext`.

use std::borrow::Cow;

use super::hb_buffer::HbBuffer;
use super::hb_common::*;
use super::hb_face::HbFace;
use super::hb_font::HbFont;
use super::hb_open_type::*;
use super::hb_ot_layout_gdef::Gdef;
use super::hb_ot_shape::HbOtShapePlan;
use super::hb_sanitize::{HbSanitizeContext, HB_SANITIZE_MAX_OPS_MAX};
use super::hb_set_digest::HbSetDigest;

/// `hb_aat_map_t::range_flags_t`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RangeFlags {
    pub(crate) flags: HbMask,
    pub(crate) cluster_first: u32,
    pub(crate) cluster_last: u32, // end - 1
}

/// `hb_aat_apply_context_t`
pub(crate) struct HbAatApplyContext<'c, 'f, 'd> {
    pub(crate) plan: &'c HbOtShapePlan,
    pub(crate) font: &'c mut HbFont<'f>,
    pub(crate) face: &'d HbFace,
    pub(crate) buffer: &'c mut HbBuffer,
    pub(crate) sanitizer: HbSanitizeContext<'d>,
    /// (the `ankr` table's data; Null: empty)
    pub(crate) ankr_table: &'d [u8],
    pub(crate) gdef_table: Gdef<'d>,
    pub(crate) range_flags: Option<&'c [RangeFlags]>,
    pub(crate) machine_glyph_set: HbSetDigest,
    pub(crate) left_set: HbSetDigest,
    pub(crate) right_set: HbSetDigest,
    pub(crate) subtable_flags: HbMask,

    /* Unused. For debug tracing only. */
    pub(crate) lookup_index: u32,
}

impl<'c, 'f, 'd> HbAatApplyContext<'c, 'f, 'd> {
    /* Note: This context is used for kerning, even without AAT, hence the condition. */

    /// `hb_aat_apply_context_t (plan, font, buffer, blob)`: `face` is
    /// the font's face, `blob` the data of the table it applies (empty
    /// for the Null blob).
    pub(crate) fn new(
        plan: &'c HbOtShapePlan,
        font: &'c mut HbFont<'f>,
        face: &'d HbFace,
        buffer: &'c mut HbBuffer,
        blob: &'d [u8],
    ) -> HbAatApplyContext<'c, 'f, 'd> {
        let mut sanitizer = HbSanitizeContext::new(Cow::Borrowed(blob));
        sanitizer.set_num_glyphs(face.get_num_glyphs());
        sanitizer.start_processing();
        sanitizer.max_ops = HB_SANITIZE_MAX_OPS_MAX as i32;
        HbAatApplyContext {
            plan,
            font,
            face,
            buffer,
            sanitizer,
            ankr_table: NULL,
            gdef_table: face.gdef().table(),
            range_flags: None,
            machine_glyph_set: HbSetDigest::full(),
            left_set: HbSetDigest::full(),
            right_set: HbSetDigest::full(),
            subtable_flags: 0,
            lookup_index: 0,
        }
    }

    /// `set_ankr_table`
    pub(crate) fn set_ankr_table(&mut self, ankr_table: &'d [u8]) {
        self.ankr_table = ankr_table;
    }

    /// `set_lookup_index`
    pub(crate) fn set_lookup_index(&mut self, i: u32) {
        self.lookup_index = i;
    }
}

/*
 * Lookup Table
 */

/// `DELETED_GLYPH`
pub(crate) const DELETED_GLYPH: u32 = 0xFFFF;

/* (the lookup tables' value types, T, are 16- or 32-bit integers or
 * offsets: `value_size` is their size) */

/// `VarSizedBinSearchArrayOf<Type>` at `p`: its unit size, its length
/// (`get_length`, without the terminator) and the position of its units.
fn var_sized_bin_search_array(
    d: &[u8],
    p: usize,
    termination_word_count: usize,
) -> (usize, u32, usize) {
    let unit_size = u16_at(d, p) as usize;
    let n_units = u16_at(d, p + 2) as u32;
    let bytes = p + 10;
    /* last_is_terminator */
    let mut last_is_terminator = n_units != 0;
    if last_is_terminator {
        /* Gah.
         *
         * "The number of termination values that need to be included is table-specific.
         * The value that indicates binary search termination is 0xFFFF." */
        let words = bytes + (n_units as usize - 1) * unit_size;
        for i in 0..termination_word_count {
            if u16_at(d, words + i * 2) != 0xFFFF {
                last_is_terminator = false;
                break;
            }
        }
    }
    (unit_size, n_units - last_is_terminator as u32, bytes)
}

/// `VarSizedBinSearchArrayOf::sanitize_shallow`
fn var_sized_bin_search_array_sanitize_shallow(
    c: &mut HbSanitizeContext,
    p: usize,
    item_size: u32,
) -> bool {
    c.check_struct(p, 10) && {
        let unit_size = c.u16(p) as u32;
        let n_units = c.u16(p + 2) as u32;
        item_size <= unit_size && c.check_range2(p + 10, n_units, unit_size)
    }
}

/// `VarSizedBinSearchArrayOf::sanitize`: the shallow check, then `f` on
/// each unit (when `f` is given: C skips it for trivially copyable
/// units without extra arguments).
fn var_sized_bin_search_array_sanitize(
    c: &mut HbSanitizeContext,
    p: usize,
    item_size: u32,
    termination_word_count: usize,
    f: Option<&mut dyn FnMut(&mut HbSanitizeContext, usize) -> bool>,
) -> bool {
    if !var_sized_bin_search_array_sanitize_shallow(c, p, item_size) {
        return false;
    }
    let Some(f) = f else {
        return true;
    };
    let (unit_size, count, bytes) = var_sized_bin_search_array(&c.data, p, termination_word_count);
    for i in 0..count as usize {
        if !f(c, bytes + i * unit_size) {
            return false;
        }
    }
    true
}

/// `VarSizedBinSearchArrayOf::bsearch` of segments (`cmp`: `g < first ?
/// -1 : g <= last ? 0 : +1`, the unit starting with `last`, `first`), or
/// of single glyphs (`segments` false): the position of the found unit.
fn var_sized_bin_search(d: &[u8], p: usize, segments: bool, g: HbCodepoint) -> Option<usize> {
    let (unit_size, len, bytes) = var_sized_bin_search_array(d, p, if segments { 2 } else { 1 });
    match hb_bsearch_impl(len as usize, |mid| {
        let u = bytes + mid * unit_size;
        if segments {
            let last = u16_at(d, u) as u32;
            let first = u16_at(d, u + 2) as u32;
            if g < first {
                -1
            } else if g <= last {
                0
            } else {
                1
            }
        } else {
            cmp_wide(g, u16_at(d, u) as u32)
        }
    }) {
        Ok(i) => Some(bytes + i as usize * unit_size),
        Err(_) => None,
    }
}

/// `Lookup<T>` at `l`.
#[derive(Clone, Copy)]
pub(crate) struct Lookup<'a> {
    pub(crate) d: &'a [u8],
    pub(crate) l: usize,
    /// `T::static_size`
    pub(crate) value_size: usize,
}

impl<'a> Lookup<'a> {
    pub(crate) fn new(d: &'a [u8], l: usize, value_size: usize) -> Lookup<'a> {
        Lookup { d, l, value_size }
    }

    #[inline]
    fn format(&self) -> u16 {
        u16_at(self.d, self.l)
    }

    /// `get_value`: the position of the glyph's value.
    pub(crate) fn get_value(&self, glyph_id: HbCodepoint, num_glyphs: u32) -> Option<usize> {
        let (d, l) = (self.d, self.l);
        match self.format() {
            0 => {
                /* LookupFormat0::get_value */
                if glyph_id >= num_glyphs {
                    return None;
                }
                Some(l + 2 + glyph_id as usize * self.value_size)
            }
            /* LookupFormat2::get_value: the segment's value */
            2 => var_sized_bin_search(d, l + 2, true, glyph_id).map(|v| v + 4),
            4 => {
                /* LookupFormat4::get_value: LookupSegmentArray::get_value */
                let v = var_sized_bin_search(d, l + 2, true, glyph_id)?;
                let last = u16_at(d, v) as u32;
                let first = u16_at(d, v + 2) as u32;
                if first <= glyph_id && glyph_id <= last {
                    Some(
                        l + u16_at(d, v + 4) as usize
                            + (glyph_id - first) as usize * self.value_size,
                    )
                } else {
                    None
                }
            }
            /* LookupFormat6::get_value: the entry's value */
            6 => var_sized_bin_search(d, l + 2, false, glyph_id).map(|v| v + 2),
            8 => {
                /* LookupFormat8::get_value */
                let first_glyph = u16_at(d, l + 2) as u32;
                let glyph_count = u16_at(d, l + 4) as u32;
                if first_glyph <= glyph_id && glyph_id - first_glyph < glyph_count {
                    Some(l + 6 + (glyph_id - first_glyph) as usize * self.value_size)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// The value at `v` (of `value_size` bytes).
    #[inline]
    fn value_at(&self, v: usize) -> u32 {
        if self.value_size == 4 {
            u32_at(self.d, v)
        } else {
            u16_at(self.d, v) as u32
        }
    }

    /// `get_value_or_null`
    pub(crate) fn get_value_or_null(&self, glyph_id: HbCodepoint, num_glyphs: u32) -> u32 {
        match self.format() {
            /* Format 10 cannot return a pointer. */
            10 => {
                /* LookupFormat10::get_value_or_null */
                let (d, l) = (self.d, self.l);
                let value_size = u16_at(d, l + 2) as u32;
                let first_glyph = u16_at(d, l + 4) as u32;
                let glyph_count = u16_at(d, l + 6) as u32;
                if !(first_glyph <= glyph_id && glyph_id - first_glyph < glyph_count) {
                    return 0; /* Null (T) */
                }

                let mut p = l + 8 + ((glyph_id - first_glyph) * value_size) as usize;

                let mut v: u32 = 0;
                let count = value_size;
                for _ in 0..count {
                    v = (v << 8) | u8_at(d, p) as u32;
                    p += 1;
                }

                v
            }
            _ => match self.get_value(glyph_id, num_glyphs) {
                Some(v) => self.value_at(v),
                None => 0, /* Null (T) */
            },
        }
    }

    /// `collect_glyphs`
    pub(crate) fn collect_glyphs(&self, glyphs: &mut HbSetDigest, num_glyphs: u32) {
        let (d, l) = (self.d, self.l);
        match self.format() {
            0 => {
                /* LookupFormat0::collect_glyphs */
                glyphs.add_range(0, num_glyphs.wrapping_sub(1));
            }
            2 | 4 => {
                /* LookupFormat2/4::collect_glyphs: LookupSegmentSingle/Array */
                let (unit_size, count, bytes) = var_sized_bin_search_array(d, l + 2, 2);
                for i in 0..count as usize {
                    let u = bytes + i * unit_size;
                    let last = u16_at(d, u) as u32;
                    let first = u16_at(d, u + 2) as u32;
                    if first == DELETED_GLYPH {
                        continue;
                    }
                    glyphs.add_range(first, last);
                }
            }
            6 => {
                /* LookupFormat6::collect_glyphs: LookupSingle */
                let (unit_size, count, bytes) = var_sized_bin_search_array(d, l + 2, 1);
                for i in 0..count as usize {
                    let glyph = u16_at(d, bytes + i * unit_size) as u32;
                    if glyph == DELETED_GLYPH {
                        continue;
                    }
                    glyphs.add(glyph);
                }
            }
            8 | 10 => {
                /* LookupFormat8/10::collect_glyphs */
                let (first_glyph, glyph_count) = if self.format() == 8 {
                    (u16_at(d, l + 2) as u32, u16_at(d, l + 4) as u32)
                } else {
                    (u16_at(d, l + 4) as u32, u16_at(d, l + 6) as u32)
                };
                if glyph_count == 0 {
                    return;
                }
                if first_glyph == DELETED_GLYPH {
                    return;
                }
                glyphs.add_range(first_glyph, first_glyph + glyph_count - 1);
            }
            _ => {}
        }
    }

    /// `get_class`
    pub(crate) fn get_class(
        &self,
        glyph_id: HbCodepoint,
        num_glyphs: u32,
        out_of_range: u32,
    ) -> u32 {
        match self.get_value(glyph_id, num_glyphs) {
            Some(v) => self.value_at(v),
            None => out_of_range,
        }
    }

    /// `sanitize`: the lookup at `l` of values of `value_size` bytes
    /// (whose `sanitize` is their `check_struct`).
    pub(crate) fn sanitize(c: &mut HbSanitizeContext, l: usize, value_size: u32) -> bool {
        if !c.sanitize_u16(l) {
            return false;
        }
        match c.u16(l) {
            /* LookupFormat0::sanitize: arrayZ.sanitize (c, c->get_num_glyphs ()) */
            0 => {
                let n = c.get_num_glyphs();
                c.check_array(l + 2, n, value_size)
            }
            /* LookupFormat2::sanitize: segments.sanitize (c) */
            2 => var_sized_bin_search_array_sanitize(c, l + 2, 4 + value_size, 2, None),
            /* LookupFormat4::sanitize: segments.sanitize (c, this) */
            4 => var_sized_bin_search_array_sanitize(
                c,
                l + 2,
                6,
                2,
                Some(&mut |c, u| lookup_segment_array_sanitize(c, u, l, value_size, None)),
            ),
            /* LookupFormat6::sanitize: entries.sanitize (c) */
            6 => var_sized_bin_search_array_sanitize(c, l + 2, 2 + value_size, 1, None),
            8 => {
                /* LookupFormat8::sanitize */
                c.check_struct(l, 6) && {
                    let glyph_count = c.u16(l + 4) as u32;
                    c.check_array(l + 6, glyph_count, value_size)
                }
            }
            10 => {
                /* LookupFormat10::sanitize */
                c.check_struct(l, 8) && {
                    let value_size = c.u16(l + 2) as u32;
                    let glyph_count = c.u16(l + 6) as u32;
                    value_size <= 4 && c.check_array(l + 8, glyph_count * value_size, 1)
                }
            }
            _ => true,
        }
    }

    /// `sanitize (c, base)`: the lookup at `l` of values of `value_size`
    /// bytes sanitized with `f` (the value's `sanitize (c, base)`).
    pub(crate) fn sanitize_with(
        c: &mut HbSanitizeContext,
        l: usize,
        value_size: u32,
        f: &mut dyn FnMut(&mut HbSanitizeContext, usize) -> bool,
    ) -> bool {
        if !c.sanitize_u16(l) {
            return false;
        }
        match c.u16(l) {
            0 => {
                /* LookupFormat0::sanitize (c, base): arrayZ.sanitize (c, c->get_num_glyphs (), base) */
                let n = c.get_num_glyphs();
                if !c.check_array(l + 2, n, value_size) {
                    return false;
                }
                for i in 0..n as usize {
                    if !f(c, l + 2 + i * value_size as usize) {
                        return false;
                    }
                }
                true
            }
            /* LookupFormat2::sanitize (c, base): LookupSegmentSingle::sanitize (c, base) */
            2 => var_sized_bin_search_array_sanitize(
                c,
                l + 2,
                4 + value_size,
                2,
                Some(&mut |c, u| c.check_struct(u, 4 + value_size as usize) && f(c, u + 4)),
            ),
            /* LookupFormat4::sanitize (c, base): segments.sanitize (c, this, base) */
            4 => var_sized_bin_search_array_sanitize(
                c,
                l + 2,
                6,
                2,
                Some(&mut |c, u| lookup_segment_array_sanitize(c, u, l, value_size, Some(f))),
            ),
            /* LookupFormat6::sanitize (c, base): LookupSingle::sanitize (c, base) */
            6 => var_sized_bin_search_array_sanitize(
                c,
                l + 2,
                2 + value_size,
                1,
                Some(&mut |c, u| c.check_struct(u, 2 + value_size as usize) && f(c, u + 2)),
            ),
            8 => {
                /* LookupFormat8::sanitize (c, base) */
                if !c.check_struct(l, 6) {
                    return false;
                }
                let glyph_count = c.u16(l + 4) as u32;
                if !c.check_array(l + 6, glyph_count, value_size) {
                    return false;
                }
                for i in 0..glyph_count as usize {
                    if !f(c, l + 6 + i * value_size as usize) {
                        return false;
                    }
                }
                true
            }
            10 => false, /* We don't support format10 here currently. */
            _ => true,
        }
    }
}

/// `LookupSegmentArray::sanitize (c, base, ...)`: the segment at `u`, its
/// values at `base+valuesZ`, sanitized with `f` (with the extra base)
/// when given.
fn lookup_segment_array_sanitize(
    c: &mut HbSanitizeContext,
    u: usize,
    base: usize,
    value_size: u32,
    f: Option<&mut dyn FnMut(&mut HbSanitizeContext, usize) -> bool>,
) -> bool {
    if !c.check_struct(u, 6) {
        return false;
    }
    let last = c.u16(u) as u32;
    let first = c.u16(u + 2) as u32;
    if first > last {
        return false;
    }
    let count = last - first + 1;
    /* valuesZ.sanitize (c, base, last - first + 1, ...) */
    c.sanitize_nn_offset16(u + 4, base, |c, values| {
        if !c.check_array(values, count, value_size) {
            return false;
        }
        match f {
            None => true,
            Some(f) => {
                for i in 0..count as usize {
                    if !f(c, values + i * value_size as usize) {
                        return false;
                    }
                }
                true
            }
        }
    })
}

/*
 * (Extended) State Table
 */

/// `Entry<T>`: an entry of a state table, with its data (`data`: the
/// entry data's 16-bit fields).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Entry {
    /// Byte offset from beginning of state table to the new state. Really?!?! Or just state
    /// number?  The latter in morx for sure.
    pub(crate) new_state: u16,
    /// Table specific.
    pub(crate) flags: u16,
    /// Optional offsets to per-glyph tables.
    pub(crate) data: [u16; 2],
}

/// `Class`
pub(crate) const CLASS_END_OF_TEXT: u32 = 0;
pub(crate) const CLASS_OUT_OF_BOUNDS: u32 = 1;
pub(crate) const CLASS_DELETED_GLYPH: u32 = 2;
pub(crate) const CLASS_END_OF_LINE: u32 = 3;

/// `StateTable::State`
pub(crate) const STATE_START_OF_TEXT: i32 = 0;
pub(crate) const STATE_START_OF_LINE: i32 = 1;

/// `StateTable<Types, Extra>` at `base`.
#[derive(Clone, Copy)]
pub(crate) struct StateTable<'a> {
    pub(crate) d: &'a [u8],
    pub(crate) base: usize,
    /// `Types::extended`
    pub(crate) extended: bool,
    /// `Extra::static_size` (0 for `void`)
    pub(crate) entry_data_size: usize,
}

impl<'a> StateTable<'a> {
    pub(crate) fn new(
        d: &'a [u8],
        base: usize,
        extended: bool,
        entry_data_size: usize,
    ) -> StateTable<'a> {
        StateTable {
            d,
            base,
            extended,
            entry_data_size,
        }
    }

    /// `static_size` (`4 * sizeof (HBUINT)`)
    pub(crate) fn static_size(extended: bool) -> usize {
        if extended {
            16
        } else {
            8
        }
    }

    /// The `HBUINT` field `i` of the header.
    #[inline]
    fn field(&self, i: usize) -> u32 {
        if self.extended {
            u32_at(self.d, self.base + i * 4)
        } else {
            u16_at(self.d, self.base + i * 2) as u32
        }
    }
    /// `nClasses`
    #[inline]
    fn n_classes(&self) -> u32 {
        self.field(0)
    }
    /// `this+classTable`
    #[inline]
    fn class_table(&self) -> usize {
        self.base.wrapping_add(self.field(1) as usize)
    }
    /// `stateArrayTable`
    #[inline]
    fn state_array_table(&self) -> u32 {
        self.field(2)
    }
    /// `this+entryTable`
    #[inline]
    fn entry_table(&self) -> usize {
        self.base.wrapping_add(self.field(3) as usize)
    }

    /// `collect_glyphs`
    pub(crate) fn collect_glyphs(&self, glyphs: &mut HbSetDigest, num_glyphs: u32) {
        if self.extended {
            Lookup::new(self.d, self.class_table(), 2).collect_glyphs(glyphs, num_glyphs);
        } else {
            class_table_collect_glyphs(self.d, self.class_table(), 1, glyphs);
        }
    }

    /// `new_state`
    pub(crate) fn new_state(&self, new_state: u32) -> i32 {
        if self.extended {
            new_state as i32
        } else {
            (new_state as i32 - self.state_array_table() as i32) / self.n_classes() as i32
        }
    }

    /// `get_class`
    pub(crate) fn get_class(
        &self,
        glyph_id: HbCodepoint,
        num_glyphs: u32,
        glyphs: &HbSetDigest,
    ) -> u32 {
        if glyph_id == DELETED_GLYPH {
            return CLASS_DELETED_GLYPH;
        }
        if !glyphs.may_have(glyph_id) {
            return CLASS_OUT_OF_BOUNDS;
        }
        if self.extended {
            Lookup::new(self.d, self.class_table(), 2).get_class(
                glyph_id,
                num_glyphs,
                CLASS_OUT_OF_BOUNDS,
            )
        } else {
            class_table_get_class(self.d, self.class_table(), 1, glyph_id, CLASS_OUT_OF_BOUNDS)
        }
    }

    /// `Entry<Extra>::static_size`
    #[inline]
    pub(crate) fn entry_size(&self) -> usize {
        4 + self.entry_data_size
    }

    /// The entry at `e` of the entry array (`get_entries ()[i]`).
    pub(crate) fn entry_at(&self, e: usize) -> Entry {
        let mut entry = Entry {
            new_state: u16_at(self.d, e),
            flags: u16_at(self.d, e.wrapping_add(2)),
            data: [0; 2],
        };
        for (i, v) in entry
            .data
            .iter_mut()
            .enumerate()
            .take(self.entry_data_size / 2)
        {
            *v = u16_at(self.d, e.wrapping_add(4 + i * 2));
        }
        entry
    }

    /// `get_entries ()[i]`
    pub(crate) fn get_entries(&self, i: u32) -> Entry {
        self.entry_at(
            self.entry_table()
                .wrapping_add(i as usize * self.entry_size()),
        )
    }

    /// `get_entry`
    pub(crate) fn get_entry(&self, state: i32, mut klass: u32) -> Entry {
        let n_classes = self.n_classes();
        if klass >= n_classes {
            klass = CLASS_OUT_OF_BOUNDS;
        }

        let states = self.base.wrapping_add(self.state_array_table() as usize);

        let i = (state as u32).wrapping_mul(n_classes).wrapping_add(klass) as usize;
        let entry = if self.extended {
            u16_at(self.d, states.wrapping_add(i * 2)) as u32
        } else {
            u8_at(self.d, states.wrapping_add(i)) as u32
        };

        self.get_entries(entry)
    }

    /// `sanitize`: the state table at `base`; `num_entries_out` gets
    /// the number of entries the states use.
    pub(crate) fn sanitize(
        c: &mut HbSanitizeContext,
        base: usize,
        extended: bool,
        entry_data_size: usize,
        num_entries_out: Option<&mut u32>,
    ) -> bool {
        let hbuint = if extended { 4 } else { 2 };
        if !(c.check_struct(base, 4 * hbuint)
            && {
                let st = StateTable::new(&c.data, base, extended, entry_data_size);
                st.n_classes() >= 4 /* Ensure pre-defined classes fit.  */
            }
            && {
                /* classTable.sanitize (c, this) */
                let off = if extended {
                    c.u32(base + 4) as usize
                } else {
                    c.u16(base + 2) as usize
                };
                if extended {
                    Lookup::sanitize(c, base + off, 2)
                } else {
                    class_table_sanitize(c, base + off, 1)
                }
            })
        {
            return false;
        }

        let st = StateTable::new(&[], base, extended, entry_data_size);
        let (n_classes, state_array_table, entry_table) = if extended {
            (c.u32(base), c.u32(base + 8), c.u32(base + 12))
        } else {
            (
                c.u16(base) as u32,
                c.u16(base + 4) as u32,
                c.u16(base + 6) as u32,
            )
        };
        let hbushort: u32 = if extended { 2 } else { 1 };
        let states = base + state_array_table as usize;
        let entries = base + entry_table as usize;
        let entry_size = st.entry_size() as u32;
        /* (new_state, on the data being sanitized) */
        let new_state = |new_state: u32| -> i32 {
            if extended {
                new_state as i32
            } else {
                (new_state as i32 - state_array_table as i32) / n_classes as i32
            }
        };
        let state_at = |c: &HbSanitizeContext, i: usize| -> u32 {
            if extended {
                c.u16(states.wrapping_add(i * 2)) as u32
            } else {
                c.u8(states.wrapping_add(i)) as u32
            }
        };

        let num_classes = n_classes;
        let Some(row_stride) = num_classes.checked_mul(hbushort) else {
            return false;
        };

        /* Apple 'kern' table has this peculiarity:
         *
         * "Because the stateTableOffset in the state table header is (strictly
         * speaking) redundant, some 'kern' tables use it to record an initial
         * state where that should not be StartOfText. To determine if this is
         * done, calculate what the stateTableOffset should be. If it's different
         * from the actual stateTableOffset, use it as the initial state."
         *
         * We implement this by calling the initial state zero, but allow *negative*
         * states if the start state indeed was not the first state.  Since the code
         * is shared, this will also apply to 'mort' table.  The 'kerx' / 'morx'
         * tables are not affected since those address states by index, not offset.
         */
        /* FIXME (upstream): hb_unsigned_mul_overflows (min_state, num_classes)
         * below converts the negative min_state to unsigned, which always
         * overflows: any negative state fails sanitizing. */

        let mut min_state: i32 = 0;
        let mut max_state: i32 = 0;
        let mut num_entries: u32 = 0;

        let mut state_pos: i32 = 0;
        let mut state_neg: i32 = 0;
        let mut entry: u32 = 0;
        while min_state < state_neg || state_pos <= max_state {
            if min_state < state_neg {
                /* Negative states. */
                if (min_state as u32).checked_mul(num_classes).is_none() {
                    return false;
                }
                let first = states.wrapping_add(
                    ((min_state as u32).wrapping_mul(num_classes) as usize) * hbushort as usize,
                );
                if !c.check_range2(first, (-min_state) as u32, row_stride) {
                    return false;
                }
                c.max_ops = c.max_ops.wrapping_sub(state_neg.wrapping_sub(min_state));
                if c.max_ops <= 0 {
                    return false;
                }
                {
                    /* Sweep new states. */
                    let stop = (min_state as u32).wrapping_mul(num_classes) as usize;
                    if first > states {
                        return false;
                    }
                    let mut p = 0usize;
                    while stop < p {
                        num_entries = num_entries.max(state_at(c, p - 1) + 1);
                        p -= 1;
                    }
                    state_neg = min_state;
                }
            }

            if state_pos <= max_state {
                /* Positive states. */
                if !c.check_range2(states, (max_state + 1) as u32, row_stride) {
                    return false;
                }
                c.max_ops = c
                    .max_ops
                    .wrapping_sub(max_state.wrapping_sub(state_pos).wrapping_add(1));
                if c.max_ops <= 0 {
                    return false;
                }
                {
                    /* Sweep new states. */
                    let Some(stop) = ((max_state + 1) as u32).checked_mul(num_classes) else {
                        return false;
                    };
                    let mut p = (state_pos as u32).wrapping_mul(num_classes) as usize;
                    while p < stop as usize {
                        num_entries = num_entries.max(state_at(c, p) + 1);
                        p += 1;
                    }
                    state_pos = max_state + 1;
                }
            }

            if !c.check_array(entries, num_entries, entry_size) {
                return false;
            }
            c.max_ops = c
                .max_ops
                .wrapping_sub(num_entries.wrapping_sub(entry) as i32);
            if c.max_ops <= 0 {
                return false;
            }
            {
                /* Sweep new entries. */
                for p in entry..num_entries {
                    let e = entries + p as usize * entry_size as usize;
                    let ns = new_state(c.u16(e) as u32);
                    min_state = min_state.min(ns);
                    max_state = max_state.max(ns);
                }
                entry = num_entries;
            }
        }

        if let Some(out) = num_entries_out {
            *out = num_entries;
        }

        true
    }
}

/// `ClassTable<HBUCHAR>::get_class` (`item_size`: 1 or 2)
pub(crate) fn class_table_get_class(
    d: &[u8],
    ct: usize,
    item_size: usize,
    glyph_id: HbCodepoint,
    out_of_range: u32,
) -> u32 {
    let first_glyph = u16_at(d, ct) as u32;
    let len = u16_at(d, ct + 2) as u32;
    let i = glyph_id.wrapping_sub(first_glyph);
    if i >= len {
        out_of_range
    } else if item_size == 2 {
        u16_at(d, ct + 4 + i as usize * 2) as u32
    } else {
        u8_at(d, ct + 4 + i as usize) as u32
    }
}

/// `ClassTable<HBUCHAR>::collect_glyphs`
pub(crate) fn class_table_collect_glyphs(
    d: &[u8],
    ct: usize,
    item_size: usize,
    glyphs: &mut HbSetDigest,
) {
    let first_glyph = u16_at(d, ct) as u32;
    let len = u16_at(d, ct + 2) as usize;
    for i in 0..len {
        let v = if item_size == 2 {
            u16_at(d, ct + 4 + i * 2) as u32
        } else {
            u8_at(d, ct + 4 + i) as u32
        };
        if v != CLASS_OUT_OF_BOUNDS {
            glyphs.add(first_glyph + i as u32);
        }
    }
}

/// `ClassTable<HBUCHAR>::sanitize`
pub(crate) fn class_table_sanitize(c: &mut HbSanitizeContext, ct: usize, item_size: u32) -> bool {
    c.check_struct(ct, 4) && c.sanitize_array16_shallow(ct + 2, item_size)
}

/// `SubtableGlyphCoverage::sanitize`
pub(crate) fn subtable_glyph_coverage_sanitize(
    c: &mut HbSanitizeContext,
    p: usize,
    subtable_count: u32,
) -> bool {
    if !c.check_array(p, subtable_count, 4) {
        return false;
    }

    let bytes = c.get_num_glyphs().div_ceil(8);
    for i in 0..subtable_count as usize {
        let offset = c.u32(p + i * 4);
        if offset == 0 || offset == 0xFFFFFFFF {
            continue;
        }
        /* subtableOffsets[i].sanitize (c, this, bytes) */
        if !(c.check_struct(p + i * 4, 4) && c.check_array(p + offset as usize, bytes, 1)) {
            return false;
        }
    }

    true
}

/// `ObsoleteTypes::offsetToIndex` / `ExtendedTypes::offsetToIndex`:
/// `array_off` is `array - base`, `size` `T::static_size`.
#[inline]
pub(crate) fn offset_to_index(extended: bool, offset: u32, array_off: u32, size: u32) -> u32 {
    if extended {
        return offset;
    }
    /* https://github.com/harfbuzz/harfbuzz/issues/3483 */
    /* If offset is less than base, return an offset that would
     * result in an address half a 32bit address-space away,
     * to make sure sanitize fails even on 32bit builds. */
    if offset < array_off {
        return i32::MAX as u32 / size;
    }

    /* https://github.com/harfbuzz/harfbuzz/issues/2816 */
    (offset - array_off) / size
}

/// `byteOffsetToIndex`
#[inline]
pub(crate) fn byte_offset_to_index(extended: bool, offset: u32, array_off: u32, size: u32) -> u32 {
    if extended {
        return offset / 2;
    }
    offset_to_index(false, offset, array_off, size)
}

/// `wordOffsetToIndex`
#[inline]
pub(crate) fn word_offset_to_index(extended: bool, offset: u32, array_off: u32, size: u32) -> u32 {
    if extended {
        return offset;
    }
    offset_to_index(false, offset.wrapping_mul(2), array_off, size)
}

/// A driver context of [`drive`] (`context_t`).
pub(crate) trait DriverContext {
    /// `in_place`
    const IN_PLACE: bool;
    /// `DontAdvance`
    const DONT_ADVANCE: u16;
    /// `is_actionable`
    fn is_actionable(&self, buffer: &HbBuffer, entry: &Entry) -> bool;
    /// `transition` (`num_glyphs`: the driver's)
    fn transition(&mut self, c: &mut HbAatApplyContext, num_glyphs: u32, entry: &Entry);
}

/// `StateTableDriver::drive`: `machine` driven over the buffer with
/// `dc` (`num_glyphs`: the face's, as `StateTableDriver` keeps it).
pub(crate) fn drive<C: DriverContext>(
    machine: &StateTable,
    num_glyphs: u32,
    dc: &mut C,
    ac: &mut HbAatApplyContext,
) {
    if !C::IN_PLACE {
        ac.buffer.clear_output();
    }

    let mut state = STATE_START_OF_TEXT;
    // If there's only one range, we already checked the flag.
    let range_flags = ac.range_flags;
    let mut last_range: Option<usize> = match range_flags {
        Some(r) if r.len() > 1 => Some(0),
        _ => None,
    };
    ac.buffer.idx = 0;
    while ac.buffer.successful {
        /* This block is copied in NoncontextualSubtable::apply. Keep in sync. */
        if let (Some(lr), Some(rf)) = (last_range, range_flags) {
            let mut range = lr;
            if ac.buffer.idx < ac.buffer.len {
                let cluster = ac.buffer.cur(0).cluster;
                while cluster < rf[range].cluster_first {
                    range -= 1;
                }
                while cluster > rf[range].cluster_last {
                    range += 1;
                }

                last_range = Some(range);
            }
            if rf[range].flags & ac.subtable_flags == 0 {
                if ac.buffer.idx == ac.buffer.len || !ac.buffer.successful {
                    break;
                }

                state = STATE_START_OF_TEXT;
                let _ = ac.buffer.next_glyph();
                continue;
            }
        }

        let buffer = &*ac.buffer;
        let klass = if buffer.idx < buffer.len {
            machine.get_class(buffer.cur(0).codepoint, num_glyphs, &ac.machine_glyph_set)
        } else {
            CLASS_END_OF_TEXT
        };
        let entry = machine.get_entry(state, klass);
        let next_state = machine.new_state(entry.new_state as u32);

        /* Conditions under which it's guaranteed safe-to-break before current glyph:
         *
         * 1. There was no action in this transition; and
         *
         * 2. If we break before current glyph, the results will be the same. That
         *    is guaranteed if:
         *
         *    2a. We were already in start-of-text state; or
         *
         *    2b. We are epsilon-transitioning to start-of-text state; or
         *
         *    2c. Starting from start-of-text state seeing current glyph:
         *
         *        2c'. There won't be any actions; and
         *
         *        2c". We would end up in the same state that we were going to end up
         *             in now, including whether epsilon-transitioning.
         *
         *    and
         *
         * 3. If we break before current glyph, there won't be any end-of-text action
         *    after previous glyph.
         *
         * This triples the transitions we need to look up, but is worth returning
         * granular unsafe-to-break results. See eg.:
         *
         *   https://github.com/harfbuzz/harfbuzz/issues/2860
         */

        let is_safe_to_break_extra = || -> bool {
            /* 2c. */
            let wouldbe_entry = machine.get_entry(STATE_START_OF_TEXT, klass);

            /* 2c'. */
            if dc.is_actionable(buffer, &wouldbe_entry) {
                return false;
            }

            /* 2c". */
            next_state == machine.new_state(wouldbe_entry.new_state as u32)
                && (entry.flags & C::DONT_ADVANCE) == (wouldbe_entry.flags & C::DONT_ADVANCE)
        };

        let is_safe_to_break = || -> bool {
            /* 1. */
            if dc.is_actionable(buffer, &entry) {
                return false;
            }

            /* 2. */
            // This one is meh, I know...
            let ok = state == STATE_START_OF_TEXT
                || ((entry.flags & C::DONT_ADVANCE) != 0 && next_state == STATE_START_OF_TEXT)
                || is_safe_to_break_extra();
            if !ok {
                return false;
            }

            /* 3. */
            !dc.is_actionable(buffer, &machine.get_entry(state, CLASS_END_OF_TEXT))
        };

        if !is_safe_to_break() && buffer.backtrack_len() != 0 && buffer.idx < buffer.len {
            let (start, end) = (buffer.backtrack_len() - 1, buffer.idx + 1);
            ac.buffer.unsafe_to_break_from_outbuffer(start, end);
        }

        dc.transition(ac, num_glyphs, &entry);

        state = next_state;

        if ac.buffer.idx == ac.buffer.len || !ac.buffer.successful {
            break;
        }

        if (entry.flags & C::DONT_ADVANCE) == 0 || {
            let max_ops = ac.buffer.max_ops;
            ac.buffer.max_ops = max_ops.wrapping_sub(1);
            max_ops <= 0
        } {
            let _ = ac.buffer.next_glyph();
        }
    }

    if !C::IN_PLACE {
        let _ = ac.buffer.sync();
    }
}
