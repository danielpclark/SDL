// Rust translation of src/hb-aat-layout-kerx-table.hh from HarfBuzz
// (8.5.0, as SDL_ttf's external/harfbuzz pins it).
// Copyright © 2018  Ebrahim Byagowi
// Copyright © 2018  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod

//! kerx -- Extended Kerning
//! <https://developer.apple.com/fonts/TrueType-Reference-Manual/RM06/Chap6kerx.html>
//!
//! Translation notes: `KerxTable<T>` and the subtable formats that the
//! `kern` table shares (`KerxSubTableFormat0`, `1` and `2`) take the
//! kind of table they are in (`KerxKind`: C's `T` and
//! `KernSubTableHeader`); a subtable is its position in the table, at
//! its header.

use super::hb_aat_layout_common::*;
use super::hb_buffer::{HbBuffer, HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT};
use super::hb_common::*;
use super::hb_face::HbFace;
use super::hb_open_type::*;
use super::hb_ot_kern_table::{
    hb_kern_machine_kern, kern_subtable_apply, kern_subtable_collect_glyphs, kern_subtable_sanitize,
};
use super::hb_ot_layout_gpos::{ATTACH_TYPE_CURSIVE, ATTACH_TYPE_MARK, ATTACH_TYPE_NONE};
use super::hb_sanitize::{hb_sanitize_blob, HbSanitizeContext};
use super::hb_set_digest::HbSetDigest;

/// `HB_AAT_TAG_kerx`
pub(crate) const HB_AAT_TAG_KERX: HbTag = hb_tag(b'k', b'e', b'r', b'x');

/// The kind of a `KerxTable<T>`: `KernOT` and `KernAAT` (the two
/// versions of the `kern` table, `hb-ot-kern-table.hh`) and `kerx`, with
/// their subtable headers (`KernOTSubTableHeader`,
/// `KernAATSubTableHeader`, `KerxSubTableHeader`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KerxKind {
    KernOt,
    KernAat,
    Kerx,
}

impl KerxKind {
    /* The table */

    /// `T::Types::extended`
    #[inline]
    pub(crate) fn extended(self) -> bool {
        self == KerxKind::Kerx
    }

    /// `T::minVersion`
    #[inline]
    fn min_version(self) -> u32 {
        match self {
            KerxKind::KernOt => 0,
            KerxKind::KernAat => 0x00010000,
            KerxKind::Kerx => 2,
        }
    }

    /// The table's version (and the size of its field), its
    /// `tableCount` and the position of its first subtable.
    #[inline]
    pub(crate) fn table_layout(self, d: &[u8]) -> (u32, u32, usize) {
        match self {
            KerxKind::KernOt => (u16_at(d, 0) as u32, u16_at(d, 2) as u32, 4),
            KerxKind::KernAat => (u32_at(d, 0), u32_at(d, 4), 8),
            KerxKind::Kerx => (u16_at(d, 0) as u32, u32_at(d, 4), 8),
        }
    }

    /* The subtable header */

    /// `SubTableHeader::static_size`
    #[inline]
    pub(crate) fn header_size(self) -> usize {
        match self {
            KerxKind::KernOt => 6,
            KerxKind::KernAat => 8,
            KerxKind::Kerx => 12,
        }
    }

    /// `length`
    #[inline]
    pub(crate) fn length(self, d: &[u8], st: usize) -> u32 {
        match self {
            KerxKind::KernOt => u16_at(d, st + 2) as u32,
            KerxKind::KernAat | KerxKind::Kerx => u32_at(d, st),
        }
    }

    /// `coverage`
    #[inline]
    pub(crate) fn coverage(self, d: &[u8], st: usize) -> u32 {
        match self {
            KerxKind::KernOt => u8_at(d, st + 5) as u32,
            KerxKind::KernAat => u8_at(d, st + 4) as u32,
            KerxKind::Kerx => u32_at(d, st + 4),
        }
    }

    /// `get_type` (`format`, or `coverage & SubtableType`)
    #[inline]
    pub(crate) fn subtable_type(self, d: &[u8], st: usize) -> u32 {
        match self {
            KerxKind::KernOt => u8_at(d, st + 4) as u32,
            KerxKind::KernAat => u8_at(d, st + 5) as u32,
            KerxKind::Kerx => u32_at(d, st + 4) & 0x000000FF,
        }
    }

    /// `Coverage::CrossStream`
    #[inline]
    pub(crate) fn cross_stream(self) -> u32 {
        match self {
            KerxKind::KernOt => 0x04,
            KerxKind::KernAat => 0x40,
            KerxKind::Kerx => 0x40000000,
        }
    }

    /// `Coverage::Variation`
    #[inline]
    pub(crate) fn variation(self) -> u32 {
        match self {
            KerxKind::KernOt => 0x00, /* Not supported */
            KerxKind::KernAat => 0x20,
            KerxKind::Kerx => 0x20000000,
        }
    }

    /// `Coverage::Backwards`
    #[inline]
    pub(crate) fn backwards(self) -> u32 {
        match self {
            KerxKind::KernOt | KerxKind::KernAat => 0x00, /* Not supported */
            KerxKind::Kerx => 0x10000000,
        }
    }

    /// `is_horizontal`
    #[inline]
    pub(crate) fn is_horizontal(self, d: &[u8], st: usize) -> bool {
        match self {
            KerxKind::KernOt => self.coverage(d, st) & 0x01 != 0, /* Horizontal */
            KerxKind::KernAat => self.coverage(d, st) & 0x80 == 0, /* Vertical */
            KerxKind::Kerx => self.coverage(d, st) & 0x80000000 == 0, /* Vertical */
        }
    }

    /// `tuple_count`
    #[inline]
    pub(crate) fn tuple_count(self, d: &[u8], st: usize) -> u32 {
        match self {
            KerxKind::KernOt | KerxKind::KernAat => 0,
            KerxKind::Kerx => u32_at(d, st + 8),
        }
    }

    /// `sizeof (HBUINT)` of the subtables' `Types`
    #[inline]
    fn hbuint_size(self) -> usize {
        if self.extended() {
            4
        } else {
            2
        }
    }

    /// An `HBUINT` field of a subtable.
    #[inline]
    fn hbuint_at(self, d: &[u8], p: usize) -> u32 {
        if self.extended() {
            u32_at(d, p)
        } else {
            u16_at(d, p) as u32
        }
    }
}

/// `kerxTupleKern`: `base` is the position the tuple offsets are from.
fn kerx_tuple_kern(
    value: i32,
    tuple_count: u32,
    base: usize,
    c: Option<&mut HbSanitizeContext>,
    d: &[u8],
) -> i32 {
    let Some(c) = c else {
        return value;
    };
    if tuple_count == 0 {
        return value;
    }

    let offset = value as u32;
    let pv = base.wrapping_add(offset as usize);
    if !c.check_array(pv, tuple_count, 2) {
        return 0;
    }
    i16_at(d, pv) as i32
}

/* KernPair: left, right (HBGlyphID16), value (FWORD): 6 bytes. */

/* KerxSubTableFormat0 */

/// The size of the `BinSearchHeader<HBUINT>` of `pairs`.
#[inline]
fn bin_search_header_size(kind: KerxKind) -> usize {
    4 * kind.hbuint_size()
}

/// `KerxSubTableFormat0::get_kerning`
pub(crate) fn format0_get_kerning(
    d: &[u8],
    st: usize,
    kind: KerxKind,
    left: HbCodepoint,
    right: HbCodepoint,
    c: Option<&mut HbSanitizeContext>,
) -> i32 {
    let pairs = st + kind.header_size();
    let len = kind.hbuint_at(d, pairs) as usize; /* BinSearchHeader::len */
    let items = struct_at(d, pairs + bin_search_header_size(kind));
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
    kerx_tuple_kern(v, kind.tuple_count(d, st), st, c, d)
}

/// `KerxSubTableFormat0::apply`
pub(crate) fn format0_apply(
    c: &mut HbAatApplyContext,
    d: &[u8],
    st: usize,
    kind: KerxKind,
) -> bool {
    if !c.plan.requested_kerning {
        return false;
    }

    if kind.coverage(d, st) & kind.backwards() != 0 {
        return false;
    }

    /* accelerator_t accel (*this, c); */
    let cross_stream = kind.coverage(d, st) & kind.cross_stream() != 0;
    let (left_set, right_set) = (c.left_set, c.right_set);
    let sanitizer = &mut c.sanitizer;
    let mut driver =
        |_font: &mut super::hb_font::HbFont, left: HbCodepoint, right: HbCodepoint| -> i32 {
            /* accelerator_t::get_kerning */
            if !left_set.may_have(left) || !right_set.may_have(right) {
                return 0;
            }
            format0_get_kerning(d, st, kind, left, right, Some(&mut *sanitizer))
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

/// `KerxSubTableFormat0::collect_glyphs`
pub(crate) fn format0_collect_glyphs(
    d: &[u8],
    st: usize,
    kind: KerxKind,
    left_set: &mut HbSetDigest,
    right_set: &mut HbSetDigest,
) {
    let pairs = st + kind.header_size();
    let len = kind.hbuint_at(d, pairs) as usize;
    let items = pairs + bin_search_header_size(kind);
    for i in 0..len {
        left_set.add(u16_at(d, items + i * 6) as u32);
        right_set.add(u16_at(d, items + i * 6 + 2) as u32);
    }
}

/// `KerxSubTableFormat0::sanitize`
pub(crate) fn format0_sanitize(c: &mut HbSanitizeContext, st: usize, kind: KerxKind) -> bool {
    /* pairs.sanitize (c): BinSearchArrayOf<KernPair, HBUINT>; the
     * BinSearchHeader's min_size is 8 for both widths */
    let pairs = st + kind.header_size();
    c.check_struct(pairs, 8) && {
        let len = kind.hbuint_at(&c.data, pairs);
        c.check_array(pairs + bin_search_header_size(kind), len, 6)
    }
}

/* KerxSubTableFormat1 */

/* Format1Entry<true>::Flags */
const FORMAT1_PUSH: u16 = 0x8000; /* If set, push this glyph on the kerning stack. */
const FORMAT1_DONT_ADVANCE: u16 = 0x4000; /* If set, don't advance to the next glyph
                                           * before going to the new state. */
const FORMAT1_RESET: u16 = 0x2000; /* If set, reset the kerning data (clear the stack) */
const FORMAT1_RESERVED: u16 = 0x1FFF; /* Not used; set to 0. */
/* Format1Entry<false>::Flags */
const FORMAT1_OFFSET: u16 = 0x3FFF; /* Byte offset from beginning of subtable to the
                                     * value table for the glyphs on the kerning stack. */
const FORMAT1_RESET_OBSOLETE: u16 = 0x0000; /* Not supported? */

/// `Format1Entry<extended>::performAction`
#[inline]
fn format1_perform_action(extended: bool, entry: &Entry) -> bool {
    if extended {
        entry.data[0] != 0xFFFF
    } else {
        entry.flags & FORMAT1_OFFSET != 0
    }
}

/// `Format1Entry<extended>::kernActionIndex`
#[inline]
fn format1_kern_action_index(extended: bool, entry: &Entry) -> u32 {
    if extended {
        entry.data[0] as u32
    } else {
        (entry.flags & FORMAT1_OFFSET) as u32
    }
}

/// `KerxSubTableFormat1::driver_context_t`
struct Format1DriverContext<'a> {
    d: &'a [u8],
    extended: bool,
    tuple_count: u32,
    /// `&table->machine`
    machine: usize,
    /// `kernAction`
    kern_action: usize,
    stack: [u32; 8],
    depth: u32,
    cross_stream: bool,
}

impl DriverContext for Format1DriverContext<'_> {
    const IN_PLACE: bool = true;
    const DONT_ADVANCE: u16 = FORMAT1_DONT_ADVANCE;

    fn is_actionable(&self, _buffer: &HbBuffer, entry: &Entry) -> bool {
        format1_perform_action(self.extended, entry)
    }

    fn transition(&mut self, c: &mut HbAatApplyContext, _num_glyphs: u32, entry: &Entry) {
        let flags = entry.flags;
        let extended = self.extended;

        if flags
            & if extended {
                FORMAT1_RESET
            } else {
                FORMAT1_RESET_OBSOLETE
            }
            != 0
        {
            self.depth = 0;
        }

        if flags & FORMAT1_PUSH != 0 {
            if (self.depth as usize) < self.stack.len() {
                self.stack[self.depth as usize] = c.buffer.idx;
                self.depth += 1;
            } else {
                self.depth = 0; /* Probably not what CoreText does, but better? */
            }
        }

        if format1_perform_action(extended, entry) && self.depth != 0 {
            let tuple_count = 1u32.max(self.tuple_count);

            let mut kern_idx = format1_kern_action_index(extended, entry);
            kern_idx = byte_offset_to_index(
                extended,
                kern_idx,
                (self.kern_action - self.machine) as u32,
                2,
            );
            let mut actions = self.kern_action.wrapping_add(kern_idx as usize * 2);
            if !c
                .sanitizer
                .check_range3(actions, 2, self.depth, tuple_count)
            {
                self.depth = 0;
                return;
            }

            let kern_mask = c.plan.kern_mask;

            /* From Apple 'kern' spec:
             * "Each pops one glyph from the kerning stack and applies the kerning value to it.
             * The end of the list is marked by an odd value... */
            let mut last = false;
            while !last && self.depth != 0 {
                self.depth -= 1;
                let idx = self.stack[self.depth as usize];
                let mut v = i16_at(self.d, actions) as i32;
                actions = actions.wrapping_add(tuple_count as usize * 2);
                if idx >= c.buffer.len {
                    continue;
                }

                /* "The end of the list is marked by an odd value..." */
                last = v & 1 != 0;
                v &= !1;

                let buffer = &mut *c.buffer;
                let font = &*c.font;
                let horizontal = hb_direction_is_horizontal(buffer.props.direction);
                let mask = buffer.info[idx as usize].mask;
                let o = &mut buffer.pos[idx as usize];

                if horizontal {
                    if self.cross_stream {
                        /* The following flag is undocumented in the spec, but described
                         * in the 'kern' table example. */
                        if v == -0x8000 {
                            o.set_attach_type(ATTACH_TYPE_NONE);
                            o.set_attach_chain(0);
                            o.y_offset = 0;
                        } else if o.attach_type() != 0 {
                            o.y_offset = o.y_offset.wrapping_add(font.em_scale_y(v as i16));
                            buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT;
                        }
                    } else if mask & kern_mask != 0 {
                        o.x_advance = o.x_advance.wrapping_add(font.em_scale_x(v as i16));
                        o.x_offset = o.x_offset.wrapping_add(font.em_scale_x(v as i16));
                    }
                } else {
                    if self.cross_stream {
                        /* CoreText doesn't do crossStream kerning in vertical.  We do. */
                        if v == -0x8000 {
                            o.set_attach_type(ATTACH_TYPE_NONE);
                            o.set_attach_chain(0);
                            o.x_offset = 0;
                        } else if o.attach_type() != 0 {
                            o.x_offset = o.x_offset.wrapping_add(font.em_scale_x(v as i16));
                            buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT;
                        }
                    } else if mask & kern_mask != 0 {
                        o.y_advance = o.y_advance.wrapping_add(font.em_scale_y(v as i16));
                        o.y_offset = o.y_offset.wrapping_add(font.em_scale_y(v as i16));
                    }
                }
            }
        }
    }
}

/// `KerxSubTableFormat1::apply`
pub(crate) fn format1_apply(
    c: &mut HbAatApplyContext,
    d: &[u8],
    st: usize,
    kind: KerxKind,
) -> bool {
    let cross_stream = kind.coverage(d, st) & kind.cross_stream() != 0;
    if !c.plan.requested_kerning && !cross_stream {
        return false;
    }

    let extended = kind.extended();
    let machine = st + kind.header_size();
    let kern_action_field = machine + StateTable::static_size(extended);
    let mut dc = Format1DriverContext {
        d,
        extended,
        tuple_count: kind.tuple_count(d, st),
        machine,
        /* Apparently the offset kernAction is from the beginning of the state-machine,
         * similar to offsets in morx table, NOT from beginning of this table, like
         * other subtables in kerx.  Discovered via testing. */
        kern_action: machine + kind.hbuint_at(d, kern_action_field) as usize,
        stack: [0; 8],
        depth: 0,
        cross_stream,
    };

    let machine = StateTable::new(d, machine, extended, if extended { 2 } else { 0 });
    let num_glyphs = c.font.p.face.get_num_glyphs();
    drive(&machine, num_glyphs, &mut dc, c);

    true
}

/// `KerxSubTableFormat1::sanitize`
pub(crate) fn format1_sanitize(c: &mut HbSanitizeContext, st: usize, kind: KerxKind) -> bool {
    /* The rest of array sanitizations are done at run-time. */
    let extended = kind.extended();
    c.check_struct(
        st,
        kind.header_size() + StateTable::static_size(extended) + kind.hbuint_size(),
    ) && StateTable::sanitize(
        c,
        st + kind.header_size(),
        extended,
        if extended { 2 } else { 0 },
        None,
    )
}

/// `KerxSubTableFormat1::collect_glyphs`
pub(crate) fn format1_collect_glyphs(
    d: &[u8],
    st: usize,
    kind: KerxKind,
    left_set: &mut HbSetDigest,
    right_set: &mut HbSetDigest,
    num_glyphs: u32,
) {
    let extended = kind.extended();
    let mut set = HbSetDigest::default();
    StateTable::new(
        d,
        st + kind.header_size(),
        extended,
        if extended { 2 } else { 0 },
    )
    .collect_glyphs(&mut set, num_glyphs);
    left_set.union_(&set);
    right_set.union_(&set);
}

/* KerxSubTableFormat2 */

/// The class of `glyph` in the `Types::ClassTypeWide` at `ct` (0 for
/// none).
fn format2_class(d: &[u8], ct: usize, extended: bool, glyph: HbCodepoint, num_glyphs: u32) -> u32 {
    if extended {
        Lookup::new(d, ct, 2).get_class(glyph, num_glyphs, 0)
    } else {
        class_table_get_class(d, ct, 2, glyph, 0)
    }
}

/// `KerxSubTableFormat2::get_kerning`
pub(crate) fn format2_get_kerning(
    d: &[u8],
    st: usize,
    kind: KerxKind,
    left: HbCodepoint,
    right: HbCodepoint,
    c: &mut HbSanitizeContext,
) -> i32 {
    let extended = kind.extended();
    let size = kind.hbuint_size();
    let p = st + kind.header_size();
    let num_glyphs = c.get_num_glyphs();
    let l = format2_class(
        d,
        st + kind.hbuint_at(d, p + size) as usize,
        extended,
        left,
        num_glyphs,
    );
    let r = format2_class(
        d,
        st + kind.hbuint_at(d, p + 2 * size) as usize,
        extended,
        right,
        num_glyphs,
    );

    let array = kind.hbuint_at(d, p + 3 * size);
    let mut kern_idx = l.wrapping_add(r);
    kern_idx = offset_to_index(extended, kern_idx, array, 2);
    let v = st
        .wrapping_add(array as usize)
        .wrapping_add(kern_idx as usize * 2);
    if !c.check_struct(v, 2) {
        return 0;
    }

    kerx_tuple_kern(i16_at(d, v) as i32, kind.tuple_count(d, st), st, Some(c), d)
}

/// `KerxSubTableFormat2::apply`
pub(crate) fn format2_apply(
    c: &mut HbAatApplyContext,
    d: &[u8],
    st: usize,
    kind: KerxKind,
) -> bool {
    if !c.plan.requested_kerning {
        return false;
    }

    if kind.coverage(d, st) & kind.backwards() != 0 {
        return false;
    }

    let cross_stream = kind.coverage(d, st) & kind.cross_stream() != 0;
    let (left_set, right_set) = (c.left_set, c.right_set);
    let sanitizer = &mut c.sanitizer;
    let mut driver =
        |_font: &mut super::hb_font::HbFont, left: HbCodepoint, right: HbCodepoint| -> i32 {
            /* accelerator_t::get_kerning */
            if !left_set.may_have(left) || !right_set.may_have(right) {
                return 0;
            }
            format2_get_kerning(d, st, kind, left, right, sanitizer)
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

/// `KerxSubTableFormat2::collect_glyphs`
pub(crate) fn format2_collect_glyphs(
    d: &[u8],
    st: usize,
    kind: KerxKind,
    left_set: &mut HbSetDigest,
    right_set: &mut HbSetDigest,
    num_glyphs: u32,
) {
    let size = kind.hbuint_size();
    let p = st + kind.header_size();
    for (field, set) in [(p + size, left_set), (p + 2 * size, right_set)] {
        let ct = st + kind.hbuint_at(d, field) as usize;
        if kind.extended() {
            Lookup::new(d, ct, 2).collect_glyphs(set, num_glyphs);
        } else {
            class_table_collect_glyphs(d, ct, 2, set);
        }
    }
}

/// `KerxSubTableFormat2::sanitize`
pub(crate) fn format2_sanitize(c: &mut HbSanitizeContext, st: usize, kind: KerxKind) -> bool {
    let extended = kind.extended();
    let size = kind.hbuint_size();
    let p = st + kind.header_size();
    /* (leftClassTable / rightClassTable: NNOffsetTo<ClassTypeWide, HBUINT>) */
    let class_table_sanitize_at = |c: &mut HbSanitizeContext, field: usize| -> bool {
        c.check_struct(field, size) && {
            let ct = st + kind.hbuint_at(&c.data, field) as usize;
            if extended {
                Lookup::sanitize(c, ct, 2)
            } else {
                class_table_sanitize(c, ct, 2)
            }
        }
    };
    c.check_struct(st, kind.header_size() + 4 * size)
        && class_table_sanitize_at(c, p + size)
        && class_table_sanitize_at(c, p + 2 * size)
        && {
            let array = kind.hbuint_at(&c.data, p + 3 * size);
            c.check_range(st, array)
        }
}

/* KerxSubTableFormat4 */

/* EntryData: ankrActionIndex (Either 0xFFFF (for no action) or the index of
 * the action to perform.) */

/* Flags */
const FORMAT4_MARK: u16 = 0x8000; /* If set, remember this glyph as the marked glyph. */
const FORMAT4_DONT_ADVANCE: u16 = 0x4000; /* If set, don't advance to the next glyph before
                                           * going to the new state. */
const FORMAT4_RESERVED: u16 = 0x3FFF; /* Not used; set to 0. */

/* SubTableFlags */
const FORMAT4_ACTION_TYPE: u32 = 0xC0000000; /* A two-bit field containing the action type. */
const FORMAT4_UNUSED: u32 = 0x3F000000; /* Unused - must be zero. */
const FORMAT4_OFFSET: u32 = 0x00FFFFFF; /* Masks the offset in bytes from the beginning
                                         * of the subtable to the beginning of the control
                                         * point table. */

/// `KerxSubTableFormat4::driver_context_t`
struct Format4DriverContext<'a> {
    d: &'a [u8],
    action_type: u32,
    /// `ankrData`
    ankr_data: usize,
    mark_set: bool,
    mark: u32,
}

impl DriverContext for Format4DriverContext<'_> {
    const IN_PLACE: bool = true;
    const DONT_ADVANCE: u16 = FORMAT4_DONT_ADVANCE;

    fn is_actionable(&self, _buffer: &HbBuffer, entry: &Entry) -> bool {
        entry.data[0] != 0xFFFF
    }

    fn transition(&mut self, c: &mut HbAatApplyContext, _num_glyphs: u32, entry: &Entry) {
        let d = self.d;
        let ankr_action_index = entry.data[0] as usize;
        if self.mark_set && entry.data[0] != 0xFFFF && c.buffer.idx < c.buffer.len {
            let mark_glyph = c.buffer.info[self.mark as usize].codepoint;
            let cur_glyph = c.buffer.cur(0).codepoint;
            let (x_offset, y_offset);
            match self.action_type {
                0 => {
                    /* Control Point Actions.*/
                    /* Indexed into glyph outline. */
                    /* Each action (record in ankrData) contains two 16-bit fields, so we must
                    double the ankrActionIndex to get the correct offset here. */
                    let data = self.ankr_data.wrapping_add(ankr_action_index * 2 * 2);
                    if !c.sanitizer.check_array(data, 2, 2) {
                        return;
                    }
                    let mark_control_point = u16_at(d, data) as u32;
                    let curr_control_point = u16_at(d, data + 2) as u32;
                    let (mut mark_x, mut mark_y, mut curr_x, mut curr_y) = (0, 0, 0, 0);
                    if !c.font.get_glyph_contour_point_for_origin(
                        mark_glyph,
                        mark_control_point,
                        HB_DIRECTION_LTR, /*XXX*/
                        &mut mark_x,
                        &mut mark_y,
                    ) || !c.font.get_glyph_contour_point_for_origin(
                        cur_glyph,
                        curr_control_point,
                        HB_DIRECTION_LTR, /*XXX*/
                        &mut curr_x,
                        &mut curr_y,
                    ) {
                        return;
                    }

                    x_offset = mark_x.wrapping_sub(curr_x);
                    y_offset = mark_y.wrapping_sub(curr_y);
                }

                1 => {
                    /* Anchor Point Actions. */
                    /* Indexed into 'ankr' table. */
                    /* Each action (record in ankrData) contains two 16-bit fields, so we must
                    double the ankrActionIndex to get the correct offset here. */
                    let data = self.ankr_data.wrapping_add(ankr_action_index * 2 * 2);
                    if !c.sanitizer.check_array(data, 2, 2) {
                        return;
                    }
                    let mark_anchor_point = u16_at(d, data) as u32;
                    let curr_anchor_point = u16_at(d, data + 2) as u32;
                    let num_glyphs = c.sanitizer.get_num_glyphs();
                    let mark_anchor = super::hb_aat_layout_ankr_table::ankr_get_anchor(
                        c.ankr_table,
                        mark_glyph,
                        mark_anchor_point,
                        num_glyphs,
                    );
                    let curr_anchor = super::hb_aat_layout_ankr_table::ankr_get_anchor(
                        c.ankr_table,
                        cur_glyph,
                        curr_anchor_point,
                        num_glyphs,
                    );

                    x_offset = c
                        .font
                        .em_scale_x(mark_anchor.0)
                        .wrapping_sub(c.font.em_scale_x(curr_anchor.0));
                    y_offset = c
                        .font
                        .em_scale_y(mark_anchor.1)
                        .wrapping_sub(c.font.em_scale_y(curr_anchor.1));
                }

                2 => {
                    /* Control Point Coordinate Actions. */
                    /* Each action contains four 16-bit fields, so we multiply the ankrActionIndex
                    by 4 to get the correct offset for the given action. */
                    let data = self.ankr_data.wrapping_add(ankr_action_index * 4 * 2);
                    if !c.sanitizer.check_array(data, 4, 2) {
                        return;
                    }
                    let mark_x = i16_at(d, data);
                    let mark_y = i16_at(d, data + 2);
                    let curr_x = i16_at(d, data + 4);
                    let curr_y = i16_at(d, data + 6);

                    x_offset = c
                        .font
                        .em_scale_x(mark_x)
                        .wrapping_sub(c.font.em_scale_x(curr_x));
                    y_offset = c
                        .font
                        .em_scale_y(mark_y)
                        .wrapping_sub(c.font.em_scale_y(curr_y));
                }

                _ => {
                    /* (the offsets are left as they are) */
                    let o = c.buffer.cur_pos(0);
                    x_offset = o.x_offset;
                    y_offset = o.y_offset;
                }
            }
            let buffer = &mut *c.buffer;
            let idx = buffer.idx;
            let o = buffer.cur_pos_mut(0);
            o.x_offset = x_offset;
            o.y_offset = y_offset;
            o.set_attach_type(ATTACH_TYPE_MARK);
            o.set_attach_chain((self.mark as i32).wrapping_sub(idx as i32) as i16);
            buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT;
        }

        if entry.flags & FORMAT4_MARK != 0 {
            self.mark_set = true;
            self.mark = c.buffer.idx;
        }
    }
}

/// `KerxSubTableFormat4::apply`
fn format4_apply(c: &mut HbAatApplyContext, d: &[u8], st: usize, kind: KerxKind) -> bool {
    let machine = st + kind.header_size();
    let flags = u32_at(d, machine + StateTable::static_size(true));
    let mut dc = Format4DriverContext {
        d,
        action_type: (flags & FORMAT4_ACTION_TYPE) >> 30,
        ankr_data: machine + (flags & FORMAT4_OFFSET) as usize,
        mark_set: false,
        mark: 0,
    };

    let machine = StateTable::new(d, machine, true, 2);
    let num_glyphs = c.font.p.face.get_num_glyphs();
    drive(&machine, num_glyphs, &mut dc, c);

    true
}

/// `KerxSubTableFormat4::sanitize`
fn format4_sanitize(c: &mut HbSanitizeContext, st: usize, kind: KerxKind) -> bool {
    /* The rest of array sanitizations are done at run-time. */
    c.check_struct(st, kind.header_size() + StateTable::static_size(true) + 4)
        && StateTable::sanitize(c, st + kind.header_size(), true, 2, None)
}

/// `KerxSubTableFormat4::collect_glyphs`
fn format4_collect_glyphs(
    d: &[u8],
    st: usize,
    kind: KerxKind,
    left_set: &mut HbSetDigest,
    right_set: &mut HbSetDigest,
    num_glyphs: u32,
) {
    let mut set = HbSetDigest::default();
    StateTable::new(d, st + kind.header_size(), true, 2).collect_glyphs(&mut set, num_glyphs);
    left_set.union_(&set);
    right_set.union_(&set);
}

/* KerxSubTableFormat6 */

/* Flags */
const FORMAT6_VALUES_ARE_LONG: u32 = 0x00000001;

/// `is_long`
#[inline]
fn format6_is_long(d: &[u8], st: usize, kind: KerxKind) -> bool {
    u32_at(d, st + kind.header_size()) & FORMAT6_VALUES_ARE_LONG != 0
}

/// `KerxSubTableFormat6::get_kerning`
fn format6_get_kerning(
    d: &[u8],
    st: usize,
    kind: KerxKind,
    left: HbCodepoint,
    right: HbCodepoint,
    c: &mut HbSanitizeContext,
) -> i32 {
    let p = st + kind.header_size();
    let num_glyphs = c.get_num_glyphs();
    /* (u: rowIndexTable, columnIndexTable, array; then vector) */
    let row_index_table = st.wrapping_add(u32_at(d, p + 8) as usize);
    let column_index_table = st.wrapping_add(u32_at(d, p + 12) as usize);
    let array = st.wrapping_add(u32_at(d, p + 16) as usize);
    let vector = st.wrapping_add(u32_at(d, p + 20) as usize);
    if format6_is_long(d, st, kind) {
        let l = Lookup::new(d, row_index_table, 4).get_value_or_null(left, num_glyphs);
        let r = Lookup::new(d, column_index_table, 4).get_value_or_null(right, num_glyphs);
        let offset = l.wrapping_add(r);
        if offset < l {
            return 0; /* Addition overflow. */
        }
        if offset.checked_mul(4).is_none() {
            return 0;
        }
        let v = array.wrapping_add(offset as usize * 4);
        if !c.check_struct(v, 4) {
            return 0;
        }
        kerx_tuple_kern(i32_at(d, v), kind.tuple_count(d, st), vector, Some(c), d)
    } else {
        let l = Lookup::new(d, row_index_table, 2).get_value_or_null(left, num_glyphs);
        let r = Lookup::new(d, column_index_table, 2).get_value_or_null(right, num_glyphs);
        let offset = l + r;
        let v = array.wrapping_add(offset as usize * 2);
        if !c.check_struct(v, 2) {
            return 0;
        }
        kerx_tuple_kern(
            i16_at(d, v) as i32,
            kind.tuple_count(d, st),
            vector,
            Some(c),
            d,
        )
    }
}

/// `KerxSubTableFormat6::apply`
fn format6_apply(c: &mut HbAatApplyContext, d: &[u8], st: usize, kind: KerxKind) -> bool {
    if !c.plan.requested_kerning {
        return false;
    }

    if kind.coverage(d, st) & kind.backwards() != 0 {
        return false;
    }

    let cross_stream = kind.coverage(d, st) & kind.cross_stream() != 0;
    let (left_set, right_set) = (c.left_set, c.right_set);
    let sanitizer = &mut c.sanitizer;
    let mut driver =
        |_font: &mut super::hb_font::HbFont, left: HbCodepoint, right: HbCodepoint| -> i32 {
            /* accelerator_t::get_kerning */
            if !left_set.may_have(left) || !right_set.may_have(right) {
                return 0;
            }
            format6_get_kerning(d, st, kind, left, right, sanitizer)
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

/// `KerxSubTableFormat6::sanitize`
fn format6_sanitize(c: &mut HbSanitizeContext, st: usize, kind: KerxKind) -> bool {
    let p = st + kind.header_size();
    let value_size =
        if c.check_struct(st, kind.header_size() + 24) && format6_is_long(&c.data, st, kind) {
            4
        } else {
            2
        };
    /* NNOffset32To<Lookup<HBUINT32 or HBUINT16>>::sanitize (c, this) */
    let lookup_sanitize_at = |c: &mut HbSanitizeContext, field: usize| -> bool {
        c.check_struct(field, 4) && {
            let l = st + c.u32(field) as usize;
            Lookup::sanitize(c, l, value_size)
        }
    };
    c.check_struct(st, kind.header_size() + 24)
        && lookup_sanitize_at(c, p + 8)
        && lookup_sanitize_at(c, p + 12)
        && {
            let array = c.u32(p + 16);
            c.check_range(st, array)
        }
        && (kind.tuple_count(&c.data, st) == 0 || {
            let vector = c.u32(p + 20);
            c.check_range(st, vector)
        })
}

/// `KerxSubTableFormat6::collect_glyphs`
fn format6_collect_glyphs(
    d: &[u8],
    st: usize,
    kind: KerxKind,
    left_set: &mut HbSetDigest,
    right_set: &mut HbSetDigest,
    num_glyphs: u32,
) {
    let p = st + kind.header_size();
    let value_size = if format6_is_long(d, st, kind) { 4 } else { 2 };
    Lookup::new(d, st.wrapping_add(u32_at(d, p + 8) as usize), value_size)
        .collect_glyphs(left_set, num_glyphs);
    Lookup::new(d, st.wrapping_add(u32_at(d, p + 12) as usize), value_size)
        .collect_glyphs(right_set, num_glyphs);
}

/* KerxSubTableHeader: length (HBUINT32), coverage (HBUINT32), tupleCount
 * (HBUINT32): 12 bytes. See KerxKind. */

/* KerxSubTable */

/// `KerxSubTable::dispatch (hb_aat_apply_context_t)`
fn kerx_subtable_apply(c: &mut HbAatApplyContext, d: &[u8], st: usize) -> bool {
    let kind = KerxKind::Kerx;
    match kind.subtable_type(d, st) {
        0 => format0_apply(c, d, st, kind),
        1 => format1_apply(c, d, st, kind),
        2 => format2_apply(c, d, st, kind),
        4 => format4_apply(c, d, st, kind),
        6 => format6_apply(c, d, st, kind),
        _ => false,
    }
}

/// `KerxSubTable::collect_glyphs`
fn kerx_subtable_collect_glyphs(
    d: &[u8],
    st: usize,
    left_set: &mut HbSetDigest,
    right_set: &mut HbSetDigest,
    num_glyphs: u32,
) {
    let kind = KerxKind::Kerx;
    match kind.subtable_type(d, st) {
        0 => format0_collect_glyphs(d, st, kind, left_set, right_set),
        1 => format1_collect_glyphs(d, st, kind, left_set, right_set, num_glyphs),
        2 => format2_collect_glyphs(d, st, kind, left_set, right_set, num_glyphs),
        4 => format4_collect_glyphs(d, st, kind, left_set, right_set, num_glyphs),
        6 => format6_collect_glyphs(d, st, kind, left_set, right_set, num_glyphs),
        _ => {}
    }
}

/// `KerxSubTable::sanitize`
fn kerx_subtable_sanitize(c: &mut HbSanitizeContext, st: usize) -> bool {
    let kind = KerxKind::Kerx;
    if !(c.check_struct(st, 12) && {
        let length = kind.length(&c.data, st);
        length >= 12 && c.check_range(st, length)
    }) {
        return false;
    }

    match kind.subtable_type(&c.data, st) {
        0 => format0_sanitize(c, st, kind),
        1 => format1_sanitize(c, st, kind),
        2 => format2_sanitize(c, st, kind),
        4 => format4_sanitize(c, st, kind),
        6 => format6_sanitize(c, st, kind),
        _ => true,
    }
}

/* The subtables of a KerxTable<T> (T::SubTable) */

/// `SubTable::sanitize`
fn subtable_sanitize(c: &mut HbSanitizeContext, st: usize, kind: KerxKind) -> bool {
    match kind {
        KerxKind::Kerx => kerx_subtable_sanitize(c, st),
        _ => kern_subtable_sanitize(c, st, kind),
    }
}

/// `SubTable::collect_glyphs`
fn subtable_collect_glyphs(
    d: &[u8],
    st: usize,
    kind: KerxKind,
    left_set: &mut HbSetDigest,
    right_set: &mut HbSetDigest,
    num_glyphs: u32,
) {
    match kind {
        KerxKind::Kerx => kerx_subtable_collect_glyphs(d, st, left_set, right_set, num_glyphs),
        _ => kern_subtable_collect_glyphs(d, st, kind, left_set, right_set, num_glyphs),
    }
}

/// `SubTable::dispatch (hb_aat_apply_context_t)`
fn subtable_apply(c: &mut HbAatApplyContext, d: &[u8], st: usize, kind: KerxKind) -> bool {
    match kind {
        KerxKind::Kerx => kerx_subtable_apply(c, d, st),
        _ => kern_subtable_apply(c, d, st, kind),
    }
}

/*
 * The 'kerx' Table
 */

/// `kern_accelerator_data_t`
pub(crate) type KernAcceleratorData = Vec<(HbSetDigest, HbSetDigest)>;

/// `KerxTable<T>::has_state_machine`
pub(crate) fn kerx_table_has_state_machine(d: &[u8], kind: KerxKind) -> bool {
    let (_, count, mut st) = kind.table_layout(d);
    for _ in 0..count {
        if kind.subtable_type(d, st) == 1 {
            return true;
        }
        st = st.wrapping_add(kind.length(d, st) as usize);
    }
    false
}

/// `KerxTable<T>::has_cross_stream`
pub(crate) fn kerx_table_has_cross_stream(d: &[u8], kind: KerxKind) -> bool {
    let (_, count, mut st) = kind.table_layout(d);
    for _ in 0..count {
        if kind.coverage(d, st) & kind.cross_stream() != 0 {
            return true;
        }
        st = st.wrapping_add(kind.length(d, st) as usize);
    }
    false
}

/// `KerxTable<T>::apply`
pub(crate) fn kerx_table_apply(
    c: &mut HbAatApplyContext,
    d: &[u8],
    kind: KerxKind,
    accel_data: Option<&KernAcceleratorData>,
) -> bool {
    c.buffer.unsafe_to_concat(0, u32::MAX);

    let mut ret = false;
    let mut seen_cross_stream = false;
    c.set_lookup_index(0);
    let (_, count, mut st) = kind.table_layout(d);
    for i in 0..count {
        'skip: {
            if !kind.extended() && (kind.coverage(d, st) & kind.variation() != 0) {
                break 'skip;
            }

            if hb_direction_is_horizontal(c.buffer.props.direction) != kind.is_horizontal(d, st) {
                break 'skip;
            }

            let reverse = (kind.coverage(d, st) & kind.backwards() != 0)
                != hb_direction_is_backward(c.buffer.props.direction);

            /* (buffer->message: no message function) */

            if !seen_cross_stream && (kind.coverage(d, st) & kind.cross_stream() != 0) {
                /* Attach all glyphs into a chain. */
                seen_cross_stream = true;
                let count = c.buffer.len as usize;
                let forward = hb_direction_is_forward(c.buffer.props.direction);
                for p in &mut c.buffer.pos[..count] {
                    p.set_attach_type(ATTACH_TYPE_CURSIVE);
                    p.set_attach_chain(if forward { -1 } else { 1 });
                    /* We intentionally don't set HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT,
                     * since there needs to be a non-zero attachment for post-positioning to
                     * be needed. */
                }
            }

            if reverse {
                c.buffer.reverse();
            }

            if let Some(accel_data) = accel_data {
                let (left_set, right_set) = accel_data.get(i as usize).copied().unwrap_or_default();
                c.left_set = left_set;
                c.right_set = right_set;
            } else {
                c.left_set = HbSetDigest::full();
                c.right_set = HbSetDigest::full();
            }

            {
                /* See comment in sanitize() for conditional here. */
                if i < count - 1 {
                    c.sanitizer.set_object(st, kind.length(d, st) as usize);
                } else {
                    c.sanitizer.reset_object();
                }
                ret |= subtable_apply(c, d, st, kind);
                c.sanitizer.reset_object();
            }

            if reverse {
                c.buffer.reverse();
            }

            /* (buffer->message: no message function) */
        }

        /* skip: */
        st = st.wrapping_add(kind.length(d, st) as usize);
        c.set_lookup_index(c.lookup_index + 1);
    }

    ret
}

/// `KerxTable<T>::sanitize`
pub(crate) fn kerx_table_sanitize(c: &mut HbSanitizeContext, kind: KerxKind) -> bool {
    /* thiz()->version.sanitize (c), tableCount.sanitize (c) */
    let (version_size, count_at, count_size) = match kind {
        KerxKind::KernOt => (2, 2, 2),
        KerxKind::KernAat => (4, 4, 4),
        KerxKind::Kerx => (2, 4, 4),
    };
    if !(c.check_struct(0, version_size)
        && kind.table_layout(&c.data).0 >= kind.min_version()
        && c.check_struct(count_at, count_size))
    {
        return false;
    }

    let (version, count, mut st) = kind.table_layout(&c.data);
    for i in 0..count {
        if !c.check_struct(st, kind.header_size()) {
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
            let len = kind.length(&c.data, st) as usize;
            c.set_object(st, len);
        } else {
            c.reset_object();
        }
        let ok = subtable_sanitize(c, st, kind);
        c.reset_object();
        if !ok {
            return false;
        }

        st += kind.length(&c.data, st) as usize;
    }

    let mut major_version = version;
    if version_size == 4 {
        major_version >>= 16;
    }
    if major_version >= 3 {
        /* SubtableGlyphCoverage::sanitize (c, count) */
        if !subtable_glyph_coverage_sanitize(c, st, count) {
            return false;
        }
    }

    true
}

/// `KerxTable<T>::create_accelerator_data`
pub(crate) fn kerx_table_create_accelerator_data(
    d: &[u8],
    kind: KerxKind,
    num_glyphs: u32,
) -> KernAcceleratorData {
    let mut accel_data = Vec::new();

    let (_, count, mut st) = kind.table_layout(d);
    for _ in 0..count {
        let mut left_set = HbSetDigest::default();
        let mut right_set = HbSetDigest::default();
        subtable_collect_glyphs(d, st, kind, &mut left_set, &mut right_set, num_glyphs);
        accel_data.push((left_set, right_set));
        st = st.wrapping_add(kind.length(d, st) as usize);
    }

    accel_data
}

/// `kerx::accelerator_t`
#[derive(Debug)]
pub(crate) struct KerxAccel {
    pub(crate) table: Vec<u8>,
    accel_data: KernAcceleratorData,
}

impl KerxAccel {
    /// `accelerator_t (face)`
    pub(crate) fn new(face: &HbFace) -> KerxAccel {
        let table = hb_sanitize_blob(
            face.reference_table(HB_AAT_TAG_KERX),
            Some(face.get_num_glyphs()),
            |c| kerx_table_sanitize(c, KerxKind::Kerx),
        );
        let accel_data =
            kerx_table_create_accelerator_data(&table, KerxKind::Kerx, face.get_num_glyphs());
        KerxAccel { table, accel_data }
    }

    /// `get_blob`
    pub(crate) fn get_blob(&self) -> &[u8] {
        &self.table
    }

    /// `kerx::has_data`
    pub(crate) fn has_data(&self) -> bool {
        u16_at(&self.table, 0) != 0
    }

    /// `apply`
    pub(crate) fn apply(&self, c: &mut HbAatApplyContext) -> bool {
        kerx_table_apply(c, &self.table, KerxKind::Kerx, Some(&self.accel_data))
    }
}
