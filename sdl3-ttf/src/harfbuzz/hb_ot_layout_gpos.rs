// Rust translation of OT/Layout/GPOS/*.hh (src/hb-ot-layout-gpos-table.hh)
// from HarfBuzz (8.5.0, as SDL_ttf's external/harfbuzz pins it), without
// the subsetter and the glyph closure.
// Copyright © 2007,2008,2009,2010  Red Hat, Inc.
// Copyright © 2010,2012,2013  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Red Hat Author(s): Behdad Esfahbod
// Google Author(s): Behdad Esfahbod, Garret Rieger

//! GPOS -- Glyph Positioning
//! <https://docs.microsoft.com/en-us/typography/opentype/spec/gpos>
//!
//! As HarfBuzz's GSUB/GPOS accelerators sanitize the tables with
//! `lazy_some_gpos`, the device tables of value records and the anchors of
//! anchor matrices are sanitized when used, by the apply context's
//! sanitizer (which cannot edit); their positions in the table are those
//! of their slices in the table data.

use super::hb_algs::hb_roundf;
use super::hb_buffer::*;
use super::hb_common::*;
use super::hb_open_type::*;
use super::hb_ot_layout::*;
use super::hb_ot_layout_common::*;
use super::hb_ot_layout_gsubgpos::*;
use super::hb_sanitize::HbSanitizeContext;

/* PosLookupSubTable::Type */
pub(crate) const POS_SINGLE: u32 = 1;
pub(crate) const POS_PAIR: u32 = 2;
pub(crate) const POS_CURSIVE: u32 = 3;
pub(crate) const POS_MARK_BASE: u32 = 4;
pub(crate) const POS_MARK_LIG: u32 = 5;
pub(crate) const POS_MARK_MARK: u32 = 6;
pub(crate) const POS_CONTEXT: u32 = 7;
pub(crate) const POS_CHAIN_CONTEXT: u32 = 8;
pub(crate) const POS_EXTENSION: u32 = 9;

/* attach_type_t */
pub(crate) const ATTACH_TYPE_NONE: u8 = 0x00;
/* Each attachment should be either a mark or a cursive; can't be both. */
pub(crate) const ATTACH_TYPE_MARK: u8 = 0x01;
pub(crate) const ATTACH_TYPE_CURSIVE: u8 = 0x02;

/// The position of `s` (a slice of the table) in the table, if it is one.
fn pos_in_table(table: &[u8], s: &[u8]) -> Option<usize> {
    let t = table.as_ptr() as usize;
    let p = s.as_ptr() as usize;
    if !s.is_empty() || (p >= t && p <= t + table.len()) {
        if p >= t && p <= t + table.len() {
            return Some(p - t);
        }
    }
    None
}

/// `Offset16To<T>::sanitize (&c->sanitizer, base)` of the offset at `at`
/// in `base` (a slice of the table): false for the Null object (outside
/// the blob).
fn lazy_sanitize_offset16(
    c: &mut HbOtApplyContext,
    base: &[u8],
    at: usize,
    f: fn(&mut HbSanitizeContext, usize) -> bool,
) -> bool {
    let Some(p) = pos_in_table(&c.accel.table, base) else {
        return false;
    };
    c.sanitizer.sanitize_offset16(p + at, p, f)
}

/* ValueFormat */

/* Flags */
pub(crate) const VALUE_X_PLACEMENT: u32 = 0x0001; /* Includes horizontal adjustment for placement */
pub(crate) const VALUE_Y_PLACEMENT: u32 = 0x0002; /* Includes vertical adjustment for placement */
pub(crate) const VALUE_X_ADVANCE: u32 = 0x0004; /* Includes horizontal adjustment for advance */
pub(crate) const VALUE_Y_ADVANCE: u32 = 0x0008; /* Includes vertical adjustment for advance */
pub(crate) const VALUE_X_PLA_DEVICE: u32 = 0x0010; /* Includes horizontal Device table for placement */
pub(crate) const VALUE_Y_PLA_DEVICE: u32 = 0x0020; /* Includes vertical Device table for placement */
pub(crate) const VALUE_X_ADV_DEVICE: u32 = 0x0040; /* Includes horizontal Device table for advance */
pub(crate) const VALUE_Y_ADV_DEVICE: u32 = 0x0080; /* Includes vertical Device table for advance */
pub(crate) const VALUE_DEVICES: u32 = 0x00F0; /* Mask for having any Device table */

/// `ValueFormat`
#[derive(Clone, Copy)]
pub(crate) struct ValueFormat(pub(crate) u32);

impl ValueFormat {
    /// `get_len`
    #[inline]
    pub(crate) fn get_len(&self) -> u32 {
        (self.0 & 0xFFFF).count_ones()
    }
    /// `get_size`
    #[inline]
    pub(crate) fn get_size(&self) -> u32 {
        self.get_len() * 2
    }
    /// `has_device`
    #[inline]
    pub(crate) fn has_device(&self) -> bool {
        (self.0 & VALUE_DEVICES) != 0
    }

    /// `apply_value`: the values are the record `values` (a slice of
    /// `base`, the subtable).
    pub(crate) fn apply_value<'a>(
        &self,
        c: &mut HbOtApplyContext<'a, '_, '_>,
        base: &'a [u8],
        values: &'a [u8],
        glyph_pos: &mut HbGlyphPosition,
    ) -> bool {
        let mut ret = false;
        let format = self.0;
        if format == 0 {
            return ret;
        }

        let horizontal = hb_direction_is_horizontal(c.direction);
        let mut v = 0usize;

        /* get_short */
        let get_short = |v: &mut usize, ret: &mut bool| -> i16 {
            let x = i16_at(values, *v * 2);
            *ret |= x != 0;
            *v += 1;
            x
        };

        if format & VALUE_X_PLACEMENT != 0 {
            let s = get_short(&mut v, &mut ret);
            glyph_pos.x_offset += c.font.em_scale_x(s);
        }
        if format & VALUE_Y_PLACEMENT != 0 {
            let s = get_short(&mut v, &mut ret);
            glyph_pos.y_offset += c.font.em_scale_y(s);
        }
        if format & VALUE_X_ADVANCE != 0 {
            let s = get_short(&mut v, &mut ret);
            if horizontal {
                glyph_pos.x_advance += c.font.em_scale_x(s);
            }
        }
        /* y_advance values grow downward but font-space grows upward, hence negation */
        if format & VALUE_Y_ADVANCE != 0 {
            let s = get_short(&mut v, &mut ret);
            if !horizontal {
                glyph_pos.y_advance -= c.font.em_scale_y(s);
            }
        }

        if !self.has_device() {
            return ret;
        }

        let use_x_device = c.font.p.x_ppem != 0 || c.font.p.num_coords() != 0;
        let use_y_device = c.font.p.y_ppem != 0 || c.font.p.num_coords() != 0;

        if !use_x_device && !use_y_device {
            return ret;
        }

        let store = c.var_store;

        /* pixel -> fractional pixel */
        if format & VALUE_X_PLA_DEVICE != 0 {
            if use_x_device {
                let d = get_device(c, values, v, &mut ret, base);
                let mut cache = c.var_store_cache.take();
                glyph_pos.x_offset += d.get_x_delta(c.font, store, cache.as_deref_mut());
                c.var_store_cache = cache;
            }
            v += 1;
        }
        if format & VALUE_Y_PLA_DEVICE != 0 {
            if use_y_device {
                let d = get_device(c, values, v, &mut ret, base);
                let mut cache = c.var_store_cache.take();
                glyph_pos.y_offset += d.get_y_delta(c.font, store, cache.as_deref_mut());
                c.var_store_cache = cache;
            }
            v += 1;
        }
        if format & VALUE_X_ADV_DEVICE != 0 {
            if horizontal && use_x_device {
                let d = get_device(c, values, v, &mut ret, base);
                let mut cache = c.var_store_cache.take();
                glyph_pos.x_advance += d.get_x_delta(c.font, store, cache.as_deref_mut());
                c.var_store_cache = cache;
            }
            v += 1;
        }
        if format & VALUE_Y_ADV_DEVICE != 0 {
            /* y_advance values grow downward but font-space grows upward, hence negation */
            if !horizontal && use_y_device {
                let d = get_device(c, values, v, &mut ret, base);
                let mut cache = c.var_store_cache.take();
                glyph_pos.y_advance -= d.get_y_delta(c.font, store, cache.as_deref_mut());
                c.var_store_cache = cache;
            }
            let _ = v;
        }
        ret
    }

    /// `sanitize_value_devices`
    fn sanitize_value_devices(
        &self,
        c: &mut HbSanitizeContext,
        base: usize,
        values: usize,
    ) -> bool {
        let format = self.0;
        let mut v = values;

        if format & VALUE_X_PLACEMENT != 0 {
            v += 2;
        }
        if format & VALUE_Y_PLACEMENT != 0 {
            v += 2;
        }
        if format & VALUE_X_ADVANCE != 0 {
            v += 2;
        }
        if format & VALUE_Y_ADVANCE != 0 {
            v += 2;
        }

        for flag in [
            VALUE_X_PLA_DEVICE,
            VALUE_Y_PLA_DEVICE,
            VALUE_X_ADV_DEVICE,
            VALUE_Y_ADV_DEVICE,
        ] {
            if format & flag != 0 {
                if !c.sanitize_offset16(v, base, Device::sanitize) {
                    return false;
                }
                v += 2;
            }
        }

        true
    }

    /// `sanitize_value`
    pub(crate) fn sanitize_value(
        &self,
        c: &mut HbSanitizeContext,
        base: usize,
        values: usize,
    ) -> bool {
        if !c.check_range(values, self.get_size()) {
            return false;
        }

        if c.lazy_some_gpos {
            return true;
        }

        !self.has_device() || self.sanitize_value_devices(c, base, values)
    }

    /// `sanitize_values`
    pub(crate) fn sanitize_values(
        &self,
        c: &mut HbSanitizeContext,
        base: usize,
        values: usize,
        count: u32,
    ) -> bool {
        let size = self.get_size();

        if !c.check_range2(values, count, size) {
            return false;
        }

        if c.lazy_some_gpos {
            return true;
        }

        self.sanitize_values_stride_unsafe(c, base, values, count, size)
    }

    /// `sanitize_values_stride_unsafe`: just sanitize referenced Device
    /// tables.  Doesn't check the values themselves.
    pub(crate) fn sanitize_values_stride_unsafe(
        &self,
        c: &mut HbSanitizeContext,
        base: usize,
        values: usize,
        count: u32,
        stride: u32,
    ) -> bool {
        if !self.has_device() {
            return true;
        }

        let mut v = values;
        for _ in 0..count {
            if !self.sanitize_value_devices(c, base, v) {
                return false;
            }
            v += stride as usize;
        }

        true
    }
}

/// `get_device (value, worked, base, c->sanitizer)`: the Device of the
/// `i`th value of `values`, lazily sanitized (the Null Device if that
/// fails).
fn get_device<'a>(
    c: &mut HbOtApplyContext<'a, '_, '_>,
    values: &'a [u8],
    i: usize,
    worked: &mut bool,
    base: &'a [u8],
) -> Device<'a> {
    let off = u16_at(values, i * 2);
    *worked |= off != 0;
    /* offset.sanitize (&c, base): the offset is in the values, from base */
    let (Some(pb), Some(pv)) = (
        pos_in_table(&c.accel.table, base),
        pos_in_table(&c.accel.table, values),
    ) else {
        return Device(NULL);
    };
    if !c
        .sanitizer
        .sanitize_offset16(pv + i * 2, pb, Device::sanitize)
    {
        return Device(NULL);
    }
    /* base + offset */
    Device(if off == 0 {
        NULL
    } else {
        struct_at(base, off as usize)
    })
}

/* Anchor */

/// `Anchor`
#[derive(Clone, Copy)]
pub(crate) struct Anchor<'a>(pub(crate) &'a [u8]);

impl<'a> Anchor<'a> {
    /// `sanitize`
    pub(crate) fn sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
        if !c.sanitize_u16(p) {
            return false;
        }
        match c.u16(p) {
            /* AnchorFormat1::sanitize */
            1 => c.check_struct(p, 6),
            /* AnchorFormat2::sanitize */
            2 => c.check_struct(p, 8),
            /* AnchorFormat3::sanitize */
            3 => {
                if !c.check_struct(p, 10) {
                    return false;
                }
                c.sanitize_offset16(p + 6, p, Device::sanitize)
                    && c.sanitize_offset16(p + 8, p, Device::sanitize)
            }
            _ => true,
        }
    }

    /// `get_anchor`
    pub(crate) fn get_anchor(
        &self,
        c: &mut HbOtApplyContext<'a, '_, '_>,
        glyph_id: HbCodepoint,
        x: &mut f32,
        y: &mut f32,
    ) {
        *x = 0.0;
        *y = 0.0;
        let d = self.0;
        match u16_at(d, 0) {
            1 => {
                /* AnchorFormat1::get_anchor */
                *x = c.font.em_fscale_x(i16_at(d, 2));
                *y = c.font.em_fscale_y(i16_at(d, 4));
            }
            2 => {
                /* AnchorFormat2::get_anchor */
                let x_ppem = c.font.p.x_ppem;
                let y_ppem = c.font.p.y_ppem;
                let mut cx = 0;
                let mut cy = 0;

                let ret = (x_ppem != 0 || y_ppem != 0)
                    && c.font.get_glyph_contour_point_for_origin(
                        glyph_id,
                        u16_at(d, 6) as u32,
                        HB_DIRECTION_LTR,
                        &mut cx,
                        &mut cy,
                    );
                *x = if ret && x_ppem != 0 {
                    cx as f32
                } else {
                    c.font.em_fscale_x(i16_at(d, 2))
                };
                *y = if ret && y_ppem != 0 {
                    cy as f32
                } else {
                    c.font.em_fscale_y(i16_at(d, 4))
                };
            }
            3 => {
                /* AnchorFormat3::get_anchor */
                *x = c.font.em_fscale_x(i16_at(d, 2));
                *y = c.font.em_fscale_y(i16_at(d, 4));

                let store = c.var_store;
                if (c.font.p.x_ppem != 0 || c.font.p.num_coords() != 0)
                    && lazy_sanitize_offset16(c, d, 6, Device::sanitize)
                {
                    let mut cache = c.var_store_cache.take();
                    *x += Device(offset16_to(d, 6)).get_x_delta(c.font, store, cache.as_deref_mut())
                        as f32;
                    c.var_store_cache = cache;
                }
                if (c.font.p.y_ppem != 0 || c.font.p.num_coords() != 0)
                    && lazy_sanitize_offset16(c, d, 8, Device::sanitize)
                {
                    let mut cache = c.var_store_cache.take();
                    *y += Device(offset16_to(d, 8)).get_y_delta(c.font, store, cache.as_deref_mut())
                        as f32;
                    c.var_store_cache = cache;
                }
            }
            _ => {}
        }
    }
}

/* AnchorMatrix */

/// `AnchorMatrix::sanitize`
fn anchor_matrix_sanitize(c: &mut HbSanitizeContext, p: usize, cols: u32) -> bool {
    if !c.check_struct(p, 2) {
        return false;
    }
    let rows = c.u16(p) as u32;
    let Some(count) = rows.checked_mul(cols) else {
        return false;
    };
    if !c.check_array(p + 2, count, 2) {
        return false;
    }

    if c.lazy_some_gpos {
        return true;
    }

    for i in 0..count as usize {
        if !c.sanitize_offset16(p + 2 + i * 2, p, Anchor::sanitize) {
            return false;
        }
    }
    true
}

/// `AnchorMatrix::get_anchor`
fn anchor_matrix_get_anchor<'a>(
    c: &mut HbOtApplyContext<'a, '_, '_>,
    m: &'a [u8],
    row: u32,
    col: u32,
    cols: u32,
    found: &mut bool,
) -> Anchor<'a> {
    *found = false;

    let rows = u16_at(m, 0) as u32;
    if row >= rows || col >= cols {
        return Anchor(NULL);
    }
    let at = 2 + (row * cols + col) as usize * 2;
    if !lazy_sanitize_offset16(c, m, at, Anchor::sanitize) {
        return Anchor(NULL);
    }

    *found = u16_at(m, at) != 0;
    Anchor(offset16_to(m, at))
}

/* MarkArray */

/// `MarkArray::sanitize`
fn mark_array_sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
    c.sanitize_array16(p, 4, |c, q| {
        /* MarkRecord::sanitize */
        c.check_struct(q, 4) && c.sanitize_offset16(q + 2, p, Anchor::sanitize)
    })
}

/// `MarkArray::apply`
fn mark_array_apply<'a>(
    c: &mut HbOtApplyContext<'a, '_, '_>,
    mark_array: &'a [u8],
    mark_index: u32,
    glyph_index: u32,
    anchors: &'a [u8],
    class_count: u32,
    glyph_pos: u32,
) -> bool {
    let len = u16_at(mark_array, 0) as u32;
    let (mark_class, mark_anchor) = if mark_index < len {
        let r = 2 + mark_index as usize * 4;
        (
            u16_at(mark_array, r) as u32,
            Anchor(offset16_to(mark_array, r + 2)),
        )
    } else {
        (0, Anchor(NULL))
    };

    let mut found = false;
    let glyph_anchor =
        anchor_matrix_get_anchor(c, anchors, glyph_index, mark_class, class_count, &mut found);
    /* If this subtable doesn't have an anchor for this base and this class,
     * return false such that the subsequent subtables have a chance at it. */
    if !found {
        return false;
    }

    let (mut mark_x, mut mark_y, mut base_x, mut base_y) = (0.0, 0.0, 0.0, 0.0);

    let idx = c.buffer.idx;
    c.buffer.unsafe_to_break(glyph_pos, idx + 1);
    let cur_g = c.buffer.cur(0).codepoint;
    mark_anchor.get_anchor(c, cur_g, &mut mark_x, &mut mark_y);
    let base_g = c.buffer.info[glyph_pos as usize].codepoint;
    glyph_anchor.get_anchor(c, base_g, &mut base_x, &mut base_y);

    let idx = c.buffer.idx;
    let o = c.buffer.cur_pos_mut(0);
    o.x_offset = hb_roundf(base_x - mark_x) as HbPosition;
    o.y_offset = hb_roundf(base_y - mark_y) as HbPosition;
    o.set_attach_type(ATTACH_TYPE_MARK);
    o.set_attach_chain((glyph_pos as i32 - idx as i32) as i16);
    c.buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT;

    c.buffer.idx += 1;
    true
}

/* SinglePos */

/// `SinglePos::dispatch` of `sanitize`
fn single_pos_sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
    if !c.sanitize_u16(p) {
        return false;
    }
    match c.u16(p) {
        1 => {
            /* SinglePosFormat1::sanitize */
            c.check_struct(p, 6)
                && c.sanitize_offset16(p + 2, p, Coverage::sanitize)
                /* The coverage  table may use a range to represent a set
                 * of glyphs, which means a small number of bytes can
                 * generate a large glyph set. Manually modify the
                 * sanitizer max ops to take this into account.
                 *
                 * Note: This check *must* be right after coverage sanitize. */
                && {
                    let off = c.u16(p + 2) as usize;
                    let pop = if off == 0 { NOT_COVERED } else { coverage_population_at(c, p + off) };
                    c.check_ops(pop >> 1)
                }
                && ValueFormat(c.u16(p + 4) as u32).sanitize_value(c, p, p + 6)
        }
        2 => {
            /* SinglePosFormat2::sanitize */
            c.check_struct(p, 8)
                && c.sanitize_offset16(p + 2, p, Coverage::sanitize)
                && ValueFormat(c.u16(p + 4) as u32).sanitize_values(
                    c,
                    p,
                    p + 8,
                    c.u16(p + 6) as u32,
                )
        }
        _ => true,
    }
}

/// `Coverage::get_population` of the coverage at `p` in the sanitizer
/// (`NOT_COVERED` for the Null coverage, of format 0).
fn coverage_population_at(c: &HbSanitizeContext, p: usize) -> u32 {
    match c.u16(p) {
        1 => c.u16(p + 2) as u32,
        2 => {
            let count = c.u16(p + 2) as usize;
            let mut ret: u64 = 0;
            for i in 0..count {
                let q = p + 4 + i * 6;
                let first = c.u16(q) as u64;
                let last = c.u16(q + 2) as u64;
                if last >= first {
                    ret += last - first + 1;
                }
            }
            if ret > u32::MAX as u64 {
                u32::MAX
            } else {
                ret as u32
            }
        }
        _ => NOT_COVERED,
    }
}

/// `SinglePos::dispatch` of `apply`
fn single_pos_apply<'a>(c: &mut HbOtApplyContext<'a, '_, '_>, d: &'a [u8]) -> bool {
    match u16_at(d, 0) {
        1 => {
            let index = Coverage(offset16_to(d, 2)).get_coverage(c.buffer.cur(0).codepoint);
            if index == NOT_COVERED {
                return false;
            }

            let vf = ValueFormat(u16_at(d, 4) as u32);
            let mut pos = *c.buffer.cur_pos(0);
            vf.apply_value(c, d, struct_at(d, 6), &mut pos);
            *c.buffer.cur_pos_mut(0) = pos;

            c.buffer.idx += 1;
            true
        }
        2 => {
            let index = Coverage(offset16_to(d, 2)).get_coverage(c.buffer.cur(0).codepoint);
            if index == NOT_COVERED {
                return false;
            }

            let value_count = u16_at(d, 6) as u32;
            if index >= value_count {
                return false;
            }

            let vf = ValueFormat(u16_at(d, 4) as u32);
            let values = struct_at(d, 8 + (index * vf.get_len()) as usize * 2);
            let mut pos = *c.buffer.cur_pos(0);
            vf.apply_value(c, d, values, &mut pos);
            *c.buffer.cur_pos_mut(0) = pos;

            c.buffer.idx += 1;
            true
        }
        _ => false,
    }
}

/* PairPos */

/// `PairSet::sanitize`
fn pair_set_sanitize(
    c: &mut HbSanitizeContext,
    p: usize,
    value_formats: [ValueFormat; 2],
    len1: u32,
    stride: u32,
) -> bool {
    if !(c.check_struct(p, 2) && c.check_range2(p + 2, c.u16(p) as u32, stride)) {
        return false;
    }

    let count = c.u16(p) as u32;
    let record = p + 2;
    c.lazy_some_gpos
        || (value_formats[0].sanitize_values_stride_unsafe(c, p, record + 2, count, stride)
            && value_formats[1].sanitize_values_stride_unsafe(
                c,
                p,
                record + 2 + len1 as usize * 2,
                count,
                stride,
            ))
}

/// `PairSet::apply`
fn pair_set_apply<'a>(
    c: &mut HbOtApplyContext<'a, '_, '_>,
    set: &'a [u8],
    value_formats: [ValueFormat; 2],
    mut pos: u32,
) -> bool {
    let len1 = value_formats[0].get_len();
    let len2 = value_formats[1].get_len();
    let record_size = 2 + 2 * (len1 + len2) as usize; /* get_size */

    let len = u16_at(set, 0) as usize;
    let records = struct_at(set, 2);
    let g = c.buffer.info[pos as usize].codepoint;
    let found = hb_bsearch_impl(len, |mid| {
        cmp_wide(g, u16_at(records, mid * record_size) as u32)
    });

    if let Ok(i) = found {
        let record = struct_at(records, i as usize * record_size);
        let values = struct_at(record, 2);

        let mut cur = *c.buffer.cur_pos(0);
        let applied_first = len1 != 0 && value_formats[0].apply_value(c, set, values, &mut cur);
        *c.buffer.cur_pos_mut(0) = cur;
        let mut second = c.buffer.pos[pos as usize];
        let applied_second = len2 != 0
            && value_formats[1].apply_value(
                c,
                set,
                struct_at(values, len1 as usize * 2),
                &mut second,
            );
        c.buffer.pos[pos as usize] = second;

        if applied_first || applied_second {
            let idx = c.buffer.idx;
            c.buffer.unsafe_to_break(idx, pos + 1);
        }

        if len2 != 0 {
            pos += 1;
            // https://github.com/harfbuzz/harfbuzz/issues/3824
            // https://github.com/harfbuzz/harfbuzz/issues/3888#issuecomment-1326781116
            let idx = c.buffer.idx;
            c.buffer.unsafe_to_break(idx, pos + 1);
        }

        c.buffer.idx = pos;
        return true;
    }
    let idx = c.buffer.idx;
    c.buffer.unsafe_to_concat(idx, pos + 1);
    false
}

/// `PairPos::dispatch` of `sanitize`
fn pair_pos_sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
    if !c.sanitize_u16(p) {
        return false;
    }
    match c.u16(p) {
        1 => {
            /* PairPosFormat1_3::sanitize */
            if !c.check_struct(p, 10) {
                return false;
            }

            let vf = [
                ValueFormat(c.u16(p + 4) as u32),
                ValueFormat(c.u16(p + 6) as u32),
            ];
            let len1 = vf[0].get_len();
            let len2 = vf[1].get_len();
            let stride = 2 + 2 * (len1 + len2);

            c.sanitize_offset16(p + 2, p, Coverage::sanitize)
                && c.sanitize_array16_of_offset16(p + 8, p, |c, q| {
                    pair_set_sanitize(c, q, vf, len1, stride)
                })
        }
        2 => {
            /* PairPosFormat2_4::sanitize */
            if !(c.check_struct(p, 16)
                && c.sanitize_offset16(p + 2, p, Coverage::sanitize)
                && c.sanitize_offset16(p + 8, p, ClassDef::sanitize)
                && c.sanitize_offset16(p + 10, p, ClassDef::sanitize))
            {
                return false;
            }

            let vf1 = ValueFormat(c.u16(p + 4) as u32);
            let vf2 = ValueFormat(c.u16(p + 6) as u32);
            let len1 = vf1.get_len();
            let len2 = vf2.get_len();
            let stride = 2 * (len1 + len2);
            let count = (c.u16(p + 12) as u32).wrapping_mul(c.u16(p + 14) as u32);
            c.check_range2(p + 16, count, stride)
                && (c.lazy_some_gpos
                    || (vf1.sanitize_values_stride_unsafe(c, p, p + 16, count, stride)
                        && vf2.sanitize_values_stride_unsafe(
                            c,
                            p,
                            p + 16 + len1 as usize * 2,
                            count,
                            stride,
                        )))
        }
        _ => true,
    }
}

/// `PairPos::dispatch` of `apply`
fn pair_pos_apply<'a>(c: &mut HbOtApplyContext<'a, '_, '_>, d: &'a [u8]) -> bool {
    match u16_at(d, 0) {
        1 => {
            /* PairPosFormat1_3::apply */
            let index = Coverage(offset16_to(d, 2)).get_coverage(c.buffer.cur(0).codepoint);
            if index == NOT_COVERED {
                return false;
            }

            let idx = c.buffer.idx;
            c.iter_reset_fast(Iter::Input, idx);
            let mut unsafe_to = 0;
            if !c.iter_next(Iter::Input, Some(&mut unsafe_to)) {
                let idx = c.buffer.idx;
                c.buffer.unsafe_to_concat(idx, unsafe_to);
                return false;
            }

            let vf = [
                ValueFormat(u16_at(d, 4) as u32),
                ValueFormat(u16_at(d, 6) as u32),
            ];
            let len = u16_at(d, 8) as u32;
            let set = if index < len {
                offset16_to(d, 10 + index as usize * 2)
            } else {
                NULL
            };
            let sidx = c.iter_input.idx;
            pair_set_apply(c, set, vf, sidx)
        }
        2 => {
            /* PairPosFormat2_4::apply */
            let index = Coverage(offset16_to(d, 2)).get_coverage(c.buffer.cur(0).codepoint);
            if index == NOT_COVERED {
                return false;
            }

            let idx = c.buffer.idx;
            c.iter_reset_fast(Iter::Input, idx);
            let mut unsafe_to = 0;
            if !c.iter_next(Iter::Input, Some(&mut unsafe_to)) {
                let idx = c.buffer.idx;
                c.buffer.unsafe_to_concat(idx, unsafe_to);
                return false;
            }

            let mut sidx = c.iter_input.idx;
            let klass2 =
                ClassDef(offset16_to(d, 10)).get_class(c.buffer.info[sidx as usize].codepoint);
            if klass2 == 0 {
                let idx = c.buffer.idx;
                c.buffer.unsafe_to_concat(idx, sidx + 1);
                return false;
            }

            let class1_count = u16_at(d, 12) as u32;
            let class2_count = u16_at(d, 14) as u32;
            let klass1 = ClassDef(offset16_to(d, 8)).get_class(c.buffer.cur(0).codepoint);
            if klass1 >= class1_count || klass2 >= class2_count {
                let idx = c.buffer.idx;
                c.buffer.unsafe_to_concat(idx, sidx + 1);
                return false;
            }

            let vf1 = ValueFormat(u16_at(d, 4) as u32);
            let vf2 = ValueFormat(u16_at(d, 6) as u32);
            let len1 = vf1.get_len();
            let len2 = vf2.get_len();
            let record_len = len1 + len2;
            let v = struct_at(
                d,
                16 + (record_len * (klass1 * class2_count + klass2)) as usize * 2,
            );

            /* (the "simple kerning split" path is disabled in C: `if (false)`) */

            let mut cur = *c.buffer.cur_pos(0);
            let applied_first = len1 != 0 && vf1.apply_value(c, d, v, &mut cur);
            *c.buffer.cur_pos_mut(0) = cur;
            let mut second = c.buffer.pos[sidx as usize];
            let applied_second =
                len2 != 0 && vf2.apply_value(c, d, struct_at(v, len1 as usize * 2), &mut second);
            c.buffer.pos[sidx as usize] = second;

            let idx = c.buffer.idx;
            if applied_first || applied_second {
                c.buffer.unsafe_to_break(idx, sidx + 1);
            } else {
                c.buffer.unsafe_to_concat(idx, sidx + 1);
            }

            if len2 != 0 {
                sidx += 1;
                c.iter_input.idx = sidx;
                // https://github.com/harfbuzz/harfbuzz/issues/3824
                // https://github.com/harfbuzz/harfbuzz/issues/3888#issuecomment-1326781116
                c.buffer.unsafe_to_break(idx, sidx + 1);
            }

            c.buffer.idx = sidx;
            true
        }
        _ => false,
    }
}

/* CursivePos */

/// `reverse_cursive_minor_offset`
fn reverse_cursive_minor_offset(
    pos: &mut [HbGlyphPosition],
    i: u32,
    direction: HbDirection,
    new_parent: u32,
) {
    let chain = pos[i as usize].attach_chain() as i32;
    let type_ = pos[i as usize].attach_type();
    if chain == 0 || 0 == (type_ & ATTACH_TYPE_CURSIVE) {
        return;
    }

    pos[i as usize].set_attach_chain(0);

    let j = (i as i32 + chain) as u32;

    /* Stop if we see new parent in the chain. */
    if j == new_parent {
        return;
    }

    reverse_cursive_minor_offset(pos, j, direction, new_parent);

    if hb_direction_is_horizontal(direction) {
        pos[j as usize].y_offset = -pos[i as usize].y_offset;
    } else {
        pos[j as usize].x_offset = -pos[i as usize].x_offset;
    }

    pos[j as usize].set_attach_chain(-chain as i16);
    pos[j as usize].set_attach_type(type_);
}

/// `CursivePos::dispatch` of `sanitize`
fn cursive_pos_sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
    if !c.sanitize_u16(p) {
        return false;
    }
    match c.u16(p) {
        1 => {
            /* CursivePosFormat1::sanitize */
            if !c.sanitize_offset16(p + 2, p, Coverage::sanitize) {
                return false;
            }

            if c.lazy_some_gpos {
                c.sanitize_array16_shallow(p + 4, 4)
            } else {
                c.sanitize_array16(p + 4, 4, |c, q| {
                    /* EntryExitRecord::sanitize */
                    c.sanitize_offset16(q, p, Anchor::sanitize)
                        && c.sanitize_offset16(q + 2, p, Anchor::sanitize)
                })
            }
        }
        _ => true,
    }
}

/// `CursivePosFormat1::apply`
fn cursive_pos_apply<'a>(c: &mut HbOtApplyContext<'a, '_, '_>, d: &'a [u8]) -> bool {
    if u16_at(d, 0) != 1 {
        return false;
    }

    let coverage = Coverage(offset16_to(d, 2));
    let records_len = u16_at(d, 4) as u32;
    /* entryExitRecord[i] (the Null record out of range) */
    let record_at = |i: u32| -> Option<usize> {
        if i < records_len {
            Some(6 + i as usize * 4)
        } else {
            None
        }
    };

    let this_record = record_at(coverage.get_coverage(c.buffer.cur(0).codepoint));
    let this_entry = this_record.map(|r| u16_at(d, r)).unwrap_or(0);
    if this_entry == 0 || !lazy_sanitize_offset16(c, d, this_record.unwrap(), Anchor::sanitize) {
        return false;
    }

    let idx = c.buffer.idx;
    c.iter_reset_fast(Iter::Input, idx);
    let mut unsafe_from = 0;
    if !c.iter_prev(Iter::Input, Some(&mut unsafe_from)) {
        let idx = c.buffer.idx;
        c.buffer
            .unsafe_to_concat_from_outbuffer(unsafe_from, idx + 1);
        return false;
    }

    let sidx = c.iter_input.idx;
    let prev_record = record_at(coverage.get_coverage(c.buffer.info[sidx as usize].codepoint));
    let prev_exit = prev_record.map(|r| u16_at(d, r + 2)).unwrap_or(0);
    if prev_exit == 0 || !lazy_sanitize_offset16(c, d, prev_record.unwrap() + 2, Anchor::sanitize) {
        let idx = c.buffer.idx;
        c.buffer.unsafe_to_concat_from_outbuffer(sidx, idx + 1);
        return false;
    }

    let i = sidx;
    let j = c.buffer.idx;

    c.buffer.unsafe_to_break(i, j + 1);
    let (mut entry_x, mut entry_y, mut exit_x, mut exit_y) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let gi = c.buffer.info[i as usize].codepoint;
    Anchor(offset16_to(d, prev_record.unwrap() + 2)).get_anchor(c, gi, &mut exit_x, &mut exit_y);
    let gj = c.buffer.info[j as usize].codepoint;
    Anchor(offset16_to(d, this_record.unwrap())).get_anchor(c, gj, &mut entry_x, &mut entry_y);

    let direction = c.direction;
    let lookup_props = c.lookup_props;
    let pos = &mut c.buffer.pos;
    let (iu, ju) = (i as usize, j as usize);

    /* Main-direction adjustment */
    match direction {
        HB_DIRECTION_LTR => {
            pos[iu].x_advance = hb_roundf(exit_x) as HbPosition + pos[iu].x_offset;

            let d = hb_roundf(entry_x) as HbPosition + pos[ju].x_offset;
            pos[ju].x_advance -= d;
            pos[ju].x_offset -= d;
        }
        HB_DIRECTION_RTL => {
            let d = hb_roundf(exit_x) as HbPosition + pos[iu].x_offset;
            pos[iu].x_advance -= d;
            pos[iu].x_offset -= d;

            pos[ju].x_advance = hb_roundf(entry_x) as HbPosition + pos[ju].x_offset;
        }
        HB_DIRECTION_TTB => {
            pos[iu].y_advance = hb_roundf(exit_y) as HbPosition + pos[iu].y_offset;

            let d = hb_roundf(entry_y) as HbPosition + pos[ju].y_offset;
            pos[ju].y_advance -= d;
            pos[ju].y_offset -= d;
        }
        HB_DIRECTION_BTT => {
            let d = hb_roundf(exit_y) as HbPosition + pos[iu].y_offset;
            pos[iu].y_advance -= d;
            pos[iu].y_offset -= d;

            pos[ju].y_advance = hb_roundf(entry_y) as HbPosition;
        }
        _ => {}
    }

    /* Cross-direction adjustment */

    /* We attach child to parent (think graph theory and rooted trees whereas
     * the root stays on baseline and each node aligns itself against its
     * parent.
     *
     * Optimize things for the case of RightToLeft, as that's most common in
     * Arabic. */
    let mut child = i;
    let mut parent = j;
    let mut x_offset = hb_roundf(entry_x - exit_x) as HbPosition;
    let mut y_offset = hb_roundf(entry_y - exit_y) as HbPosition;
    if lookup_props & LOOKUP_FLAG_RIGHT_TO_LEFT == 0 {
        std::mem::swap(&mut child, &mut parent);
        x_offset = -x_offset;
        y_offset = -y_offset;
    }

    /* If child was already connected to someone else, walk through its old
     * chain and reverse the link direction, such that the whole tree of its
     * previous connection now attaches to new parent.  Watch out for case
     * where new parent is on the path from old chain...
     */
    reverse_cursive_minor_offset(pos, child, direction, parent);

    pos[child as usize].set_attach_type(ATTACH_TYPE_CURSIVE);
    pos[child as usize].set_attach_chain((parent as i32 - child as i32) as i16);
    if hb_direction_is_horizontal(direction) {
        pos[child as usize].y_offset = y_offset;
    } else {
        pos[child as usize].x_offset = x_offset;
    }

    /* If parent was attached to child, separate them.
     * https://github.com/harfbuzz/harfbuzz/issues/2469
     */
    if pos[parent as usize].attach_chain() as i32 == -(pos[child as usize].attach_chain() as i32) {
        pos[parent as usize].set_attach_chain(0);
        if hb_direction_is_horizontal(direction) {
            pos[parent as usize].y_offset = 0;
        } else {
            pos[parent as usize].x_offset = 0;
        }
    }

    c.buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT;
    c.buffer.idx += 1;
    true
}

/* MarkBasePos, MarkLigPos, MarkMarkPos */

/// `MarkBasePos`/`MarkLigPos`/`MarkMarkPos::dispatch` of `sanitize`
/// (all three have the same structure; the second array is an
/// AnchorMatrix, or a LigatureArray for MarkLigPos)
fn mark_pos_sanitize(c: &mut HbSanitizeContext, p: usize, is_lig: bool) -> bool {
    if !c.sanitize_u16(p) {
        return false;
    }
    match c.u16(p) {
        1 => {
            let class_count = c.u16(p + 6) as u32;
            c.check_struct(p, 12)
                && c.sanitize_offset16(p + 2, p, Coverage::sanitize)
                && c.sanitize_offset16(p + 4, p, Coverage::sanitize)
                && c.sanitize_offset16(p + 8, p, mark_array_sanitize)
                && c.sanitize_offset16(p + 10, p, |c, q| {
                    if is_lig {
                        /* LigatureArray::sanitize (c, classCount) */
                        c.sanitize_array16_of_offset16(q, q, |c, r| {
                            anchor_matrix_sanitize(c, r, class_count)
                        })
                    } else {
                        anchor_matrix_sanitize(c, q, class_count)
                    }
                })
        }
        _ => true,
    }
}

/// `MarkBasePosFormat1_2::accept`: we only want to attach to the first of
/// a MultipleSubst sequence.  <https://github.com/harfbuzz/harfbuzz/issues/740>
/// Reject others... ...but stop if we find a mark in the MultipleSubst
/// sequence: <https://github.com/harfbuzz/harfbuzz/issues/1020>
fn mark_base_accept(buffer: &HbBuffer, idx: u32) -> bool {
    let info = &buffer.info;
    let i = idx as usize;
    !_hb_glyph_info_multiplied(&info[i])
        || 0 == _hb_glyph_info_get_lig_comp(&info[i])
        || (idx == 0
            || _hb_glyph_info_is_mark(&info[i - 1])
            || !_hb_glyph_info_multiplied(&info[i - 1])
            || _hb_glyph_info_get_lig_id(&info[i]) != _hb_glyph_info_get_lig_id(&info[i - 1])
            || _hb_glyph_info_get_lig_comp(&info[i])
                != _hb_glyph_info_get_lig_comp(&info[i - 1]) + 1)
}

/// `MarkBasePosFormat1_2::apply`
fn mark_base_pos_apply<'a>(c: &mut HbOtApplyContext<'a, '_, '_>, d: &'a [u8]) -> bool {
    if u16_at(d, 0) != 1 {
        return false;
    }

    let mark_index = Coverage(offset16_to(d, 2)).get_coverage(c.buffer.cur(0).codepoint);
    if mark_index == NOT_COVERED {
        return false;
    }

    let base_coverage = Coverage(offset16_to(d, 4));

    /* Now we search backwards for a non-mark glyph.
     * We don't use skippy_iter.prev() to avoid O(n^2) behavior. */

    c.iter_input.set_lookup_props(LOOKUP_FLAG_IGNORE_MARKS);

    if c.last_base_until > c.buffer.idx {
        c.last_base_until = 0;
        c.last_base = -1;
    }
    let mut j = c.buffer.idx;
    while j > c.last_base_until {
        let mut m = c.iter_match(Iter::Input, j - 1);
        if m == IterMatch::Match {
            // https://github.com/harfbuzz/harfbuzz/issues/4124
            if !mark_base_accept(c.buffer, j - 1)
                && NOT_COVERED
                    == base_coverage.get_coverage(c.buffer.info[(j - 1) as usize].codepoint)
            {
                m = IterMatch::Skip;
            }
        }
        if m == IterMatch::Match {
            c.last_base = j as i32 - 1;
            break;
        }
        j -= 1;
    }
    c.last_base_until = c.buffer.idx;
    if c.last_base == -1 {
        let idx = c.buffer.idx;
        c.buffer.unsafe_to_concat_from_outbuffer(0, idx + 1);
        return false;
    }

    let idx = c.last_base as u32;

    /* Checking that matched glyph is actually a base glyph by GDEF is too strong; disabled */

    let base_index = base_coverage.get_coverage(c.buffer.info[idx as usize].codepoint);
    if base_index == NOT_COVERED {
        let bidx = c.buffer.idx;
        c.buffer.unsafe_to_concat_from_outbuffer(idx, bidx + 1);
        return false;
    }

    let class_count = u16_at(d, 6) as u32;
    mark_array_apply(
        c,
        offset16_to(d, 8),
        mark_index,
        base_index,
        offset16_to(d, 10),
        class_count,
        idx,
    )
}

/// `MarkLigPosFormat1_2::apply`
fn mark_lig_pos_apply<'a>(c: &mut HbOtApplyContext<'a, '_, '_>, d: &'a [u8]) -> bool {
    if u16_at(d, 0) != 1 {
        return false;
    }

    let mark_index = Coverage(offset16_to(d, 2)).get_coverage(c.buffer.cur(0).codepoint);
    if mark_index == NOT_COVERED {
        return false;
    }

    /* Now we search backwards for a non-mark glyph */

    c.iter_input.set_lookup_props(LOOKUP_FLAG_IGNORE_MARKS);

    if c.last_base_until > c.buffer.idx {
        c.last_base_until = 0;
        c.last_base = -1;
    }
    let mut j = c.buffer.idx;
    while j > c.last_base_until {
        let m = c.iter_match(Iter::Input, j - 1);
        if m == IterMatch::Match {
            c.last_base = j as i32 - 1;
            break;
        }
        j -= 1;
    }
    c.last_base_until = c.buffer.idx;
    if c.last_base == -1 {
        let idx = c.buffer.idx;
        c.buffer.unsafe_to_concat_from_outbuffer(0, idx + 1);
        return false;
    }

    let idx = c.last_base as u32;

    /* Checking that matched glyph is actually a ligature by GDEF is too strong; disabled */

    let lig_index = Coverage(offset16_to(d, 4)).get_coverage(c.buffer.info[idx as usize].codepoint);
    if lig_index == NOT_COVERED {
        let bidx = c.buffer.idx;
        c.buffer.unsafe_to_concat_from_outbuffer(idx, bidx + 1);
        return false;
    }

    let lig_array = offset16_to(d, 10);
    let lig_attach = if lig_index < u16_at(lig_array, 0) as u32 {
        offset16_to(lig_array, 2 + lig_index as usize * 2)
    } else {
        NULL
    };

    /* Find component to attach to */
    let comp_count = u16_at(lig_attach, 0) as u32;
    if comp_count == 0 {
        let bidx = c.buffer.idx;
        c.buffer.unsafe_to_concat_from_outbuffer(idx, bidx + 1);
        return false;
    }

    /* We must now check whether the ligature ID of the current mark glyph
     * is identical to the ligature ID of the found ligature.  If yes, we
     * can directly use the component index.  If not, we attach the mark
     * glyph to the last component of the ligature. */
    let lig_id = _hb_glyph_info_get_lig_id(&c.buffer.info[idx as usize]);
    let mark_id = _hb_glyph_info_get_lig_id(c.buffer.cur(0));
    let mark_comp = _hb_glyph_info_get_lig_comp(c.buffer.cur(0));
    let comp_index = if lig_id != 0 && lig_id == mark_id && mark_comp > 0 {
        comp_count.min(_hb_glyph_info_get_lig_comp(c.buffer.cur(0))) - 1
    } else {
        comp_count - 1
    };

    let class_count = u16_at(d, 6) as u32;
    mark_array_apply(
        c,
        offset16_to(d, 8),
        mark_index,
        comp_index,
        lig_attach,
        class_count,
        idx,
    )
}

/// `MarkMarkPosFormat1_2::apply`
fn mark_mark_pos_apply<'a>(c: &mut HbOtApplyContext<'a, '_, '_>, d: &'a [u8]) -> bool {
    if u16_at(d, 0) != 1 {
        return false;
    }

    let mark1_index = Coverage(offset16_to(d, 2)).get_coverage(c.buffer.cur(0).codepoint);
    if mark1_index == NOT_COVERED {
        return false;
    }

    /* now we search backwards for a suitable mark glyph until a non-mark glyph */
    let idx = c.buffer.idx;
    c.iter_reset_fast(Iter::Input, idx);
    let props = c.lookup_props & !LOOKUP_FLAG_IGNORE_FLAGS;
    c.iter_input.set_lookup_props(props);
    let mut unsafe_from = 0;
    if !c.iter_prev(Iter::Input, Some(&mut unsafe_from)) {
        let idx = c.buffer.idx;
        c.buffer
            .unsafe_to_concat_from_outbuffer(unsafe_from, idx + 1);
        return false;
    }

    let j = c.iter_input.idx;
    if !_hb_glyph_info_is_mark(&c.buffer.info[j as usize]) {
        let idx = c.buffer.idx;
        c.buffer.unsafe_to_concat_from_outbuffer(j, idx + 1);
        return false;
    }

    let id1 = _hb_glyph_info_get_lig_id(c.buffer.cur(0));
    let id2 = _hb_glyph_info_get_lig_id(&c.buffer.info[j as usize]);
    let comp1 = _hb_glyph_info_get_lig_comp(c.buffer.cur(0));
    let comp2 = _hb_glyph_info_get_lig_comp(&c.buffer.info[j as usize]);

    let good = if id1 == id2 {
        /* Marks belonging to the same base. */
        /* Marks belonging to the same ligature component. */
        id1 == 0 || comp1 == comp2
    } else {
        /* If ligature ids don't match, it may be the case that one of the marks
         * itself is a ligature.  In which case match. */
        (id1 > 0 && comp1 == 0) || (id2 > 0 && comp2 == 0)
    };

    if !good {
        /* Didn't match. */
        let idx = c.buffer.idx;
        c.buffer.unsafe_to_concat_from_outbuffer(j, idx + 1);
        return false;
    }

    let mark2_index = Coverage(offset16_to(d, 4)).get_coverage(c.buffer.info[j as usize].codepoint);
    if mark2_index == NOT_COVERED {
        let idx = c.buffer.idx;
        c.buffer.unsafe_to_concat_from_outbuffer(j, idx + 1);
        return false;
    }

    let class_count = u16_at(d, 6) as u32;
    mark_array_apply(
        c,
        offset16_to(d, 8),
        mark1_index,
        mark2_index,
        offset16_to(d, 10),
        class_count,
        j,
    )
}

/* PosLookupSubTable */

/// `PosLookupSubTable::dispatch` of `sanitize` (with the lookup type)
pub(crate) fn pos_lookup_subtable_sanitize(
    c: &mut HbSanitizeContext,
    p: usize,
    lookup_type: u32,
) -> bool {
    match lookup_type {
        POS_SINGLE => single_pos_sanitize(c, p),
        POS_PAIR => pair_pos_sanitize(c, p),
        POS_CURSIVE => cursive_pos_sanitize(c, p),
        POS_MARK_BASE => mark_pos_sanitize(c, p, false),
        POS_MARK_LIG => mark_pos_sanitize(c, p, true),
        POS_MARK_MARK => mark_pos_sanitize(c, p, false),
        POS_CONTEXT => Context::sanitize(c, p),
        POS_CHAIN_CONTEXT => ChainContext::sanitize(c, p),
        POS_EXTENSION => Extension::sanitize(c, p, POS_EXTENSION, pos_lookup_subtable_sanitize),
        _ => true,
    }
}

/// `PosLookupSubTable::dispatch` of `apply` (with the lookup type)
pub(crate) fn pos_lookup_subtable_apply<'a>(
    c: &mut HbOtApplyContext<'a, '_, '_>,
    st: &'a [u8],
    lookup_type: u32,
) -> bool {
    match lookup_type {
        POS_SINGLE => single_pos_apply(c, st),
        POS_PAIR => pair_pos_apply(c, st),
        POS_CURSIVE => cursive_pos_apply(c, st),
        POS_MARK_BASE => mark_base_pos_apply(c, st),
        POS_MARK_LIG => mark_lig_pos_apply(c, st),
        POS_MARK_MARK => mark_mark_pos_apply(c, st),
        POS_CONTEXT => Context::apply(c, st),
        POS_CHAIN_CONTEXT => ChainContext::apply(c, st),
        POS_EXTENSION => {
            let t = Extension::get_type(st);
            if t == POS_EXTENSION {
                return false;
            }
            pos_lookup_subtable_apply(c, Extension::get_subtable(st), t)
        }
        _ => false,
    }
}

/// `PosLookupSubTable::dispatch` of `collect_glyphs`
fn pos_lookup_subtable_collect_glyphs(c: &mut HbCollectGlyphsContext, st: &[u8], lookup_type: u32) {
    match lookup_type {
        POS_SINGLE => {
            if matches!(u16_at(st, 0), 1 | 2) {
                c.collect_coverage(CollectSet::Input, Coverage(offset16_to(st, 2)));
            }
        }
        POS_PAIR => match u16_at(st, 0) {
            1 => {
                if !c.collect_coverage(CollectSet::Input, Coverage(offset16_to(st, 2))) {
                    return;
                }
                let vf = [
                    ValueFormat(u16_at(st, 4) as u32),
                    ValueFormat(u16_at(st, 6) as u32),
                ];
                let record_size = 2 + 2 * (vf[0].get_len() + vf[1].get_len()) as usize;
                let count = u16_at(st, 8) as usize;
                for i in 0..count {
                    /* PairSet::collect_glyphs */
                    let set = offset16_to(st, 10 + i * 2);
                    let len = u16_at(set, 0) as usize;
                    for k in 0..len {
                        c.add(CollectSet::Input, u16_at(set, 2 + k * record_size) as u32);
                    }
                }
            }
            2 => {
                if !c.collect_coverage(CollectSet::Input, Coverage(offset16_to(st, 2))) {
                    return;
                }
                if let Some(s) = c.set(CollectSet::Input) {
                    ClassDef(offset16_to(st, 10)).collect_coverage(s);
                }
            }
            _ => {}
        },
        POS_CURSIVE => {
            if u16_at(st, 0) == 1 {
                c.collect_coverage(CollectSet::Input, Coverage(offset16_to(st, 2)));
            }
        }
        POS_MARK_BASE | POS_MARK_LIG | POS_MARK_MARK => {
            if u16_at(st, 0) == 1 {
                if !c.collect_coverage(CollectSet::Input, Coverage(offset16_to(st, 2))) {
                    return;
                }
                c.collect_coverage(CollectSet::Input, Coverage(offset16_to(st, 4)));
            }
        }
        POS_CONTEXT => Context::collect_glyphs(c, st),
        POS_CHAIN_CONTEXT => ChainContext::collect_glyphs(c, st),
        POS_EXTENSION => {
            let t = Extension::get_type(st);
            if t != POS_EXTENSION {
                pos_lookup_subtable_collect_glyphs(c, Extension::get_subtable(st), t);
            }
        }
        _ => {}
    }
}

/// `PosLookup::collect_glyphs`
pub(crate) fn pos_lookup_collect_glyphs(c: &mut HbCollectGlyphsContext, l: Lookup) {
    let t = l.get_type();
    for i in 0..l.get_subtable_count() {
        pos_lookup_subtable_collect_glyphs(c, l.get_subtable(i), t);
    }
}

/* GPOS */

/// `propagate_attachment_offsets`
fn propagate_attachment_offsets(
    pos: &mut [HbGlyphPosition],
    len: u32,
    i: u32,
    direction: HbDirection,
    nesting_level: u32,
) {
    /* Adjusts offsets of attached glyphs (both cursive and mark) to accumulate
     * offset of glyph they are attached to. */
    let chain = pos[i as usize].attach_chain() as i32;
    let type_ = pos[i as usize].attach_type();
    if chain == 0 {
        return;
    }

    pos[i as usize].set_attach_chain(0);

    let j = (i as i32 + chain) as u32;

    if j >= len {
        return;
    }

    if nesting_level == 0 {
        return;
    }

    propagate_attachment_offsets(pos, len, j, direction, nesting_level - 1);

    let (iu, ju) = (i as usize, j as usize);
    if type_ & ATTACH_TYPE_CURSIVE != 0 {
        if hb_direction_is_horizontal(direction) {
            pos[iu].y_offset += pos[ju].y_offset;
        } else {
            pos[iu].x_offset += pos[ju].x_offset;
        }
    } else
    /*if (type & GPOS_impl::ATTACH_TYPE_MARK)*/
    {
        pos[iu].x_offset += pos[ju].x_offset;
        pos[iu].y_offset += pos[ju].y_offset;

        if hb_direction_is_forward(direction) {
            for k in j..i {
                pos[iu].x_offset -= pos[k as usize].x_advance;
                pos[iu].y_offset -= pos[k as usize].y_advance;
            }
        } else {
            for k in j + 1..i + 1 {
                pos[iu].x_offset += pos[k as usize].x_advance;
                pos[iu].y_offset += pos[k as usize].y_advance;
            }
        }
    }
}

/// `GPOS::position_start`
pub(crate) fn gpos_position_start(buffer: &mut HbBuffer) {
    let count = buffer.len as usize;
    for i in 0..count {
        buffer.pos[i].set_attach_type(0);
        buffer.pos[i].set_attach_chain(0);
    }
}

/// `GPOS::position_finish_advances`
pub(crate) fn gpos_position_finish_advances(_buffer: &mut HbBuffer) {}

/// `GPOS::position_finish_offsets`
pub(crate) fn gpos_position_finish_offsets(slant: f32, slant_xy: f32, buffer: &mut HbBuffer) {
    if !buffer.have_positions {
        buffer.clear_positions();
    }
    let len = buffer.len;
    let direction = buffer.props.direction;

    /* Handle attachments */
    if buffer.scratch_flags & HB_BUFFER_SCRATCH_FLAG_HAS_GPOS_ATTACHMENT != 0 {
        for i in 0..len {
            propagate_attachment_offsets(&mut buffer.pos, len, i, direction, HB_MAX_NESTING_LEVEL);
        }
    }

    if slant != 0.0 {
        for i in 0..len as usize {
            if buffer.pos[i].y_offset != 0 {
                buffer.pos[i].x_offset +=
                    hb_roundf(slant_xy * buffer.pos[i].y_offset as f32) as HbPosition;
            }
        }
    }
}
