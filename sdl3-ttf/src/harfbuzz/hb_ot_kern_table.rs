// Rust translation of src/hb-ot-kern-table.hh, src/hb-kern.hh and the
// parts of src/hb-aat-layout-kerx-table.hh and src/hb-aat-layout-common.hh
// that the `kern` table uses, from HarfBuzz (8.5.0, as SDL_ttf's
// external/harfbuzz pins it).
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
//! Translation notes: subtables of format 0 (kerning pairs), 2 (class
//! kerning) and 3 (compact class kerning) are translated, of both the
//! OpenType and the Apple table versions. Format 1 (the state-machine
//! "contextual kerning" of Apple's fonts, which needs AAT's state tables)
//! is not: its subtables pass sanitizing on their headers alone and are
//! not applied.

use std::borrow::Cow;

use super::hb_buffer::*;
use super::hb_common::*;
use super::hb_face::*;
use super::hb_font::HbFont;
use super::hb_open_type::*;
use super::hb_ot_layout_common::LOOKUP_FLAG_IGNORE_MARKS;
use super::hb_ot_layout_gpos::ATTACH_TYPE_CURSIVE;
use super::hb_ot_layout_gsubgpos::{GsubGposAccel, GsubGposKind, HbOtApplyContext, Iter};
use super::hb_sanitize::{hb_sanitize_blob, HbSanitizeContext, HB_SANITIZE_MAX_OPS_MAX};
use super::hb_set_digest::HbSetDigest;

/// `KernOTSubTableHeader` / `KernAATSubTableHeader`: which kind of
/// subtable header the table has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KernHeader {
    /// `KernOTSubTableHeader` (6 bytes)
    Ot,
    /// `KernAATSubTableHeader` (8 bytes)
    Aat,
}

impl KernHeader {
    /// `static_size`
    fn static_size(self) -> usize {
        match self {
            KernHeader::Ot => 6,
            KernHeader::Aat => 8,
        }
    }
    /// `length`
    fn length(self, d: &[u8], st: usize) -> u32 {
        match self {
            KernHeader::Ot => u16_at(d, st + 2) as u32,
            KernHeader::Aat => u32_at(d, st),
        }
    }
    /// `format`
    fn format(self, d: &[u8], st: usize) -> u32 {
        match self {
            KernHeader::Ot => u8_at(d, st + 4) as u32,
            KernHeader::Aat => u8_at(d, st + 5) as u32,
        }
    }
    /// `coverage`
    fn coverage(self, d: &[u8], st: usize) -> u32 {
        match self {
            KernHeader::Ot => u8_at(d, st + 5) as u32,
            KernHeader::Aat => u8_at(d, st + 4) as u32,
        }
    }
    /// `Coverage::CrossStream`
    fn cross_stream(self) -> u32 {
        match self {
            KernHeader::Ot => 0x04,
            KernHeader::Aat => 0x40,
        }
    }
    /// `Coverage::Variation`
    fn variation(self) -> u32 {
        match self {
            KernHeader::Ot => 0x00, /* Not supported */
            KernHeader::Aat => 0x20,
        }
    }
    /// `Coverage::Backwards` (not supported in either)
    fn backwards(self) -> u32 {
        0x00
    }
    /// `is_horizontal`
    fn is_horizontal(self, d: &[u8], st: usize) -> bool {
        match self {
            KernHeader::Ot => self.coverage(d, st) & 0x01 != 0, /* Horizontal */
            KernHeader::Aat => self.coverage(d, st) & 0x80 == 0, /* Vertical */
        }
    }
    /// `tuple_count` (always 0: the tuples are not implemented)
    fn tuple_count(self) -> u32 {
        0
    }
}

/* KerxSubTableFormat0 (with KernPair) */

/// `KerxSubTableFormat0::get_kerning`: the pairs' binary search.
fn format0_get_kerning(
    d: &[u8],
    st: usize,
    h: KernHeader,
    left: HbCodepoint,
    right: HbCodepoint,
) -> i32 {
    let pairs = st + h.static_size();
    let len = u16_at(d, pairs) as usize; /* BinSearchHeader<HBUINT16>::len */
    let items = struct_at(d, pairs + 8);
    let r = hb_bsearch_impl(len, |mid| {
        let ret = cmp_wide(left, u16_at(items, mid * 6) as u32);
        if ret != 0 {
            return ret;
        }
        cmp_wide(right, u16_at(items, mid * 6 + 2) as u32)
    });
    let v = match r {
        Ok(i) => i16_at(items, i as usize * 6 + 4) as i32,
        Err(_) => 0, /* Null (KernPair) */
    };
    /* kerxTupleKern: tupleCount is 0 */
    v
}

fn format0_sanitize(c: &mut HbSanitizeContext, st: usize, h: KernHeader) -> bool {
    /* pairs.sanitize (c): BinSearchArrayOf<KernPair, HBUINT16> */
    let pairs = st + h.static_size();
    c.check_struct(pairs, 8) && {
        let len = c.u16(pairs) as u32;
        c.check_array(pairs + 8, len, 6)
    }
}

fn format0_collect_glyphs(
    d: &[u8],
    st: usize,
    h: KernHeader,
    left_set: &mut HbSetDigest,
    right_set: &mut HbSetDigest,
) {
    let pairs = st + h.static_size();
    let len = u16_at(d, pairs) as usize;
    for i in 0..len {
        left_set.add(u16_at(d, pairs + 8 + i * 6) as u32);
        right_set.add(u16_at(d, pairs + 8 + i * 6 + 2) as u32);
    }
}

/* KerxSubTableFormat2 (with ClassTable<HBUINT16>) */

/// `ClassTable<HBUINT16>::get_class`
fn class_table_get_class(d: &[u8], ct: usize, glyph_id: HbCodepoint, out_of_range: u32) -> u32 {
    let first_glyph = u16_at(d, ct) as u32;
    let len = u16_at(d, ct + 2) as u32;
    let i = glyph_id.wrapping_sub(first_glyph);
    if i >= len {
        out_of_range
    } else {
        u16_at(d, ct + 4 + i as usize * 2) as u32
    }
}

/// `ClassTable<HBUINT16>::collect_glyphs`
fn class_table_collect_glyphs(d: &[u8], ct: usize, glyphs: &mut HbSetDigest) {
    const CLASS_OUT_OF_BOUNDS: u32 = 1;
    let first_glyph = u16_at(d, ct) as u32;
    let len = u16_at(d, ct + 2) as usize;
    for i in 0..len {
        if u16_at(d, ct + 4 + i * 2) as u32 != CLASS_OUT_OF_BOUNDS {
            glyphs.add(first_glyph + i as u32);
        }
    }
}

/// `ClassTable<HBUINT16>::sanitize`
fn class_table_sanitize(c: &mut HbSanitizeContext, ct: usize) -> bool {
    c.check_struct(ct, 4) && c.sanitize_array16_shallow(ct + 2, 2)
}

/// `KerxSubTableFormat2::get_kerning`
fn format2_get_kerning(
    d: &[u8],
    st: usize,
    h: KernHeader,
    sanitizer: &mut HbSanitizeContext,
    left: HbCodepoint,
    right: HbCodepoint,
) -> i32 {
    let p = st + h.static_size();
    let l = class_table_get_class(d, st + u16_at(d, p + 2) as usize, left, 0);
    let r = class_table_get_class(d, st + u16_at(d, p + 4) as usize, right, 0);
    let array = u16_at(d, p + 6) as u32;
    let kern_idx = l + r;
    /* Types::offsetToIndex (kern_idx, this, arrayZ.arrayZ) */
    let kern_idx = if kern_idx < array {
        /* https://github.com/harfbuzz/harfbuzz/issues/3483 */
        (i32::MAX / 2) as u32
    } else {
        /* https://github.com/harfbuzz/harfbuzz/issues/2816 */
        (kern_idx - array) / 2
    };
    let v = st + array as usize + kern_idx as usize * 2;
    if !sanitizer.check_struct(v, 2) {
        return 0;
    }
    /* kerxTupleKern: tupleCount is 0 */
    i16_at(d, v) as i32
}

fn format2_sanitize(c: &mut HbSanitizeContext, st: usize, h: KernHeader) -> bool {
    let p = st + h.static_size();
    c.check_struct(st, h.static_size() + 8)
        && c.sanitize_nn_offset16(p + 2, st, class_table_sanitize)
        && c.sanitize_nn_offset16(p + 4, st, class_table_sanitize)
        && {
            let array = c.u16(p + 6) as u32;
            c.check_range(st, array)
        }
}

fn format2_collect_glyphs(
    d: &[u8],
    st: usize,
    h: KernHeader,
    left_set: &mut HbSetDigest,
    right_set: &mut HbSetDigest,
) {
    let p = st + h.static_size();
    class_table_collect_glyphs(d, st + u16_at(d, p + 2) as usize, left_set);
    class_table_collect_glyphs(d, st + u16_at(d, p + 4) as usize, right_set);
}

/* KernSubTableFormat3 */

/// `KernSubTableFormat3::get_kerning`
fn format3_get_kerning(
    d: &[u8],
    st: usize,
    h: KernHeader,
    left: HbCodepoint,
    right: HbCodepoint,
) -> i32 {
    let p = st + h.static_size();
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

fn format3_sanitize(c: &mut HbSanitizeContext, st: usize, h: KernHeader) -> bool {
    let p = st + h.static_size();
    c.check_struct(st, h.static_size() + 6) && {
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

fn format3_collect_glyphs(
    d: &[u8],
    st: usize,
    h: KernHeader,
    left_set: &mut HbSetDigest,
    right_set: &mut HbSetDigest,
) {
    let glyph_count = u16_at(d, st + h.static_size()) as u32;
    let mut set = HbSetDigest::default();
    if glyph_count != 0 {
        set.add_range(0, glyph_count - 1);
    }
    left_set.union_(&set);
    right_set.union_(&set);
}

/* KernSubTable */

/// `KernSubTable::sanitize`
fn subtable_sanitize(c: &mut HbSanitizeContext, st: usize, h: KernHeader) -> bool {
    if !(c.check_struct(st, h.static_size())
        && h.length(&c.data, st) >= h.static_size() as u32
        && c.check_range(st, h.length(&c.data, st)))
    {
        return false;
    }
    match h.format(&c.data, st) {
        0 => format0_sanitize(c, st, h),
        /* (format 1, the state machine, is not translated) */
        1 => true,
        2 => format2_sanitize(c, st, h),
        3 => format3_sanitize(c, st, h),
        _ => true,
    }
}

/// `KernSubTable::collect_glyphs`
fn subtable_collect_glyphs(
    d: &[u8],
    st: usize,
    h: KernHeader,
    left_set: &mut HbSetDigest,
    right_set: &mut HbSetDigest,
) {
    match h.format(d, st) {
        0 => format0_collect_glyphs(d, st, h, left_set, right_set),
        /* (format 1 is not translated) */
        2 => format2_collect_glyphs(d, st, h, left_set, right_set),
        3 => format3_collect_glyphs(d, st, h, left_set, right_set),
        _ => {}
    }
}

/* KerxTable<KernOT> / KerxTable<KernAAT> */

/// The table version (`KernOT`: 16 bits, `KernAAT`: 32 bits), the
/// subtable count and the offset of the first subtable.
fn kerx_table_layout(d: &[u8], h: KernHeader) -> (u32, u32, usize) {
    match h {
        KernHeader::Ot => (u16_at(d, 0) as u32, u16_at(d, 2) as u32, 4),
        KernHeader::Aat => (u32_at(d, 0), u32_at(d, 4), 8),
    }
}

/// `KerxTable::has_state_machine`
fn kerx_table_has_state_machine(d: &[u8], h: KernHeader) -> bool {
    let (_, count, mut st) = kerx_table_layout(d, h);
    for _ in 0..count {
        if h.format(d, st) == 1 {
            return true;
        }
        st += h.length(d, st) as usize;
    }
    false
}

/// `KerxTable::has_cross_stream`
fn kerx_table_has_cross_stream(d: &[u8], h: KernHeader) -> bool {
    let (_, count, mut st) = kerx_table_layout(d, h);
    for _ in 0..count {
        if h.coverage(d, st) & h.cross_stream() != 0 {
            return true;
        }
        st += h.length(d, st) as usize;
    }
    false
}

/// `KerxTable::sanitize`
fn kerx_table_sanitize(c: &mut HbSanitizeContext, h: KernHeader) -> bool {
    let (version_size, min_version) = match h {
        KernHeader::Ot => (2, 0u32),
        KernHeader::Aat => (4, 0x00010000u32),
    };
    if !(c.check_struct(0, version_size)
        && {
            let version = if version_size == 2 {
                c.u16(0) as u32
            } else {
                c.u32(0)
            };
            version >= min_version
        }
        && c.check_struct(version_size, version_size))
    {
        return false;
    }

    let (version, count, mut st) = kerx_table_layout(&c.data, h);
    for i in 0..count {
        if !c.check_struct(st, h.static_size()) {
            return false;
        }
        /* OpenType kern table has 2-byte subtable lengths.  That's limiting.
         * MS implementation also only supports one subtable, of format 0,
         * anyway.  Certain versions of some fonts, like Calibry, contain
         * kern subtable that exceeds 64kb.  Looks like, the subtable length
         * is simply ignored.  Which makes sense.  It's only needed if you
         * have multiple subtables.  To handle such fonts, we just ignore
         * the length for the last subtable. */
        if i < count - 1 {
            let len = h.length(&c.data, st) as usize;
            c.set_object(st, len);
        } else {
            c.reset_object();
        }
        let ok = subtable_sanitize(c, st, h);
        c.reset_object();
        if !ok {
            return false;
        }
        st += h.length(&c.data, st) as usize;
    }

    let mut major_version = version;
    if version_size == 4 {
        major_version >>= 16;
    }
    if major_version >= 3 {
        /* SubtableGlyphCoverage::sanitize (c, count) */
        if !c.check_array(st, count, 4) {
            return false;
        }
        let bytes = c.get_num_glyphs().div_ceil(8);
        for i in 0..count as usize {
            let offset = c.u32(st + i * 4);
            if offset == 0 || offset == 0xFFFFFFFF {
                continue;
            }
            /* NNOffset32To<UnsizedArrayOf<HBUINT8>>::sanitize (c, this, bytes) */
            if !(c.check_struct(st + i * 4, 4) && c.check_range(st + offset as usize, bytes)) {
                return false;
            }
        }
    }
    true
}

/* kern */

/// `kern::sanitize`
fn kern_sanitize(c: &mut HbSanitizeContext) -> bool {
    if !c.check_struct(0, 4) {
        return false;
    }
    match c.u16(0) {
        0 => kerx_table_sanitize(c, KernHeader::Ot),
        1 => kerx_table_sanitize(c, KernHeader::Aat),
        _ => true,
    }
}

/// `kern::accelerator_t`
#[derive(Debug)]
pub(crate) struct KernAccel {
    pub(crate) table: Vec<u8>,
    /// `kern_accelerator_data_t`: the left and right glyph digests of
    /// each subtable
    accel_data: Vec<(HbSetDigest, HbSetDigest)>,
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
        k.accel_data = k.create_accelerator_data();
        k
    }

    /// The header kind of the table (`get_type`), if it is a known one.
    fn header(&self) -> Option<KernHeader> {
        match u16_at(&self.table, 0) {
            0 => Some(KernHeader::Ot),
            1 => Some(KernHeader::Aat),
            _ => None,
        }
    }

    /// `create_accelerator_data`
    fn create_accelerator_data(&self) -> Vec<(HbSetDigest, HbSetDigest)> {
        let Some(h) = self.header() else {
            return Vec::new();
        };
        let d = &self.table[..];
        let (_, count, mut st) = kerx_table_layout(d, h);
        let mut accel_data = Vec::new();
        for _ in 0..count {
            let mut left_set = HbSetDigest::default();
            let mut right_set = HbSetDigest::default();
            subtable_collect_glyphs(d, st, h, &mut left_set, &mut right_set);
            accel_data.push((left_set, right_set));
            st += h.length(d, st) as usize;
        }
        accel_data
    }

    /// `has_data`
    pub(crate) fn has_data(&self) -> bool {
        u32_at(&self.table, 0) != 0
    }

    /// `has_state_machine`
    pub(crate) fn has_state_machine(&self) -> bool {
        match self.header() {
            Some(h) => kerx_table_has_state_machine(&self.table, h),
            None => false,
        }
    }

    /// `has_cross_stream`
    pub(crate) fn has_cross_stream(&self) -> bool {
        match self.header() {
            Some(h) => kerx_table_has_cross_stream(&self.table, h),
            None => false,
        }
    }

    /// `apply` (`KerxTable::apply`, with the `hb_aat_apply_context_t`'s
    /// plan values `requested_kerning` and `kern_mask`)
    pub(crate) fn apply(
        &self,
        face: &HbFace,
        font: &mut HbFont,
        buffer: &mut HbBuffer,
        requested_kerning: bool,
        kern_mask: HbMask,
    ) -> bool {
        let Some(h) = self.header() else {
            return false;
        };
        let d = &self.table[..];

        /* hb_aat_apply_context_t: the sanitizer on the blob */
        let mut sanitizer = HbSanitizeContext::new(Cow::Borrowed(d));
        sanitizer.set_num_glyphs(face.get_num_glyphs());
        sanitizer.start_processing();
        sanitizer.max_ops = HB_SANITIZE_MAX_OPS_MAX as i32;

        buffer.unsafe_to_concat(0, u32::MAX);

        let mut ret = false;
        let mut seen_cross_stream = false;

        let (_, count, mut st) = kerx_table_layout(d, h);
        for i in 0..count {
            'subtable: {
                if h.coverage(d, st) & h.variation() != 0 {
                    break 'subtable;
                }

                if hb_direction_is_horizontal(buffer.props.direction) != h.is_horizontal(d, st) {
                    break 'subtable;
                }

                let reverse = (h.coverage(d, st) & h.backwards() != 0)
                    != hb_direction_is_backward(buffer.props.direction);

                if !seen_cross_stream && (h.coverage(d, st) & h.cross_stream() != 0) {
                    /* Attach all glyphs into a chain. */
                    seen_cross_stream = true;
                    let count = buffer.len as usize;
                    let forward = hb_direction_is_forward(buffer.props.direction);
                    for p in &mut buffer.pos[..count] {
                        p.set_attach_type(ATTACH_TYPE_CURSIVE);
                        p.set_attach_chain(if forward { -1 } else { 1 });
                        /* We intentionally don't set HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT,
                         * since there needs to be a non-zero attachment for post-positioning to
                         * be needed. */
                    }
                }

                if reverse {
                    buffer.reverse();
                }

                let (left_set, right_set) = self.accel_data[i as usize];

                {
                    /* See comment in sanitize() for conditional here. */
                    if i < count - 1 {
                        sanitizer.set_object(st, h.length(d, st) as usize);
                    } else {
                        sanitizer.reset_object();
                    }
                    ret |= subtable_apply(
                        d,
                        st,
                        h,
                        face,
                        font,
                        buffer,
                        &mut sanitizer,
                        (left_set, right_set),
                        requested_kerning,
                        kern_mask,
                    );
                    sanitizer.reset_object();
                }

                if reverse {
                    buffer.reverse();
                }
            }
            /* skip: */
            st += h.length(d, st) as usize;
        }

        ret
    }
}

/// `KernSubTable::dispatch (hb_aat_apply_context_t)`: the subtable's
/// `apply`.
#[allow(clippy::too_many_arguments)]
fn subtable_apply(
    d: &[u8],
    st: usize,
    h: KernHeader,
    face: &HbFace,
    font: &mut HbFont,
    buffer: &mut HbBuffer,
    sanitizer: &mut HbSanitizeContext,
    sets: (HbSetDigest, HbSetDigest),
    requested_kerning: bool,
    kern_mask: HbMask,
) -> bool {
    let format = h.format(d, st);
    match format {
        0 | 2 | 3 => {
            if !requested_kerning {
                return false;
            }

            if h.coverage(d, st) & h.backwards() != 0 {
                return false;
            }

            let cross_stream = h.coverage(d, st) & h.cross_stream() != 0;
            let (left_set, right_set) = sets;
            let mut driver = |_font: &mut HbFont, left: HbCodepoint, right: HbCodepoint| -> i32 {
                match format {
                    0 => {
                        /* accelerator_t::get_kerning */
                        if !left_set.may_have(left) || !right_set.may_have(right) {
                            return 0;
                        }
                        format0_get_kerning(d, st, h, left, right)
                    }
                    2 => {
                        if !left_set.may_have(left) || !right_set.may_have(right) {
                            return 0;
                        }
                        format2_get_kerning(d, st, h, sanitizer, left, right)
                    }
                    _ => format3_get_kerning(d, st, h, left, right),
                }
            };
            hb_kern_machine_kern(
                &mut driver,
                cross_stream,
                face,
                font,
                buffer,
                kern_mask,
                true,
            );
            true
        }
        /* (format 1 is not translated) */
        _ => false,
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
