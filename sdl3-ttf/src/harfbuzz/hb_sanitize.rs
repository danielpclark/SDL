// Rust translation of src/hb-sanitize.hh from HarfBuzz (8.5.0, as SDL_ttf's
// external/harfbuzz pins it).
// Copyright © 2007,2008,2009,2010  Red Hat, Inc.
// Copyright © 2012,2018  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Red Hat Author(s): Behdad Esfahbod
// Google Author(s): Behdad Esfahbod

//! Sanitize.
//!
//! The sanitize machinery is at the core of HarfBuzz's zero-cost font
//! loading: a font table blob is sanitized before use, to ensure invalid
//! memory access does not happen. Sanitizing a blob of data with a type T
//! works as follows (with minor simplification):
//!
//!   - Call the sanitize() function of T on the blob,
//!   - If sanitize succeeded, use the blob.
//!   - Otherwise, if edits were requested (neutering bad offsets), make
//!     the blob writable and call sanitize() again; use the blob if
//!     sanitize succeeded (and a second round requests no edits).
//!   - Use the empty blob otherwise.
//!
//! Note that what sanitize() checks for might align with what the
//! specification describes as valid table data, but does not have to be:
//! it checks what the other table functions rely on.
//!
//! Translation notes: positions are byte indices into the blob (C's
//! pointers); the checks are those of a 64-bit build (`check_struct` only
//! checks that the end of the structure is in range, without counting
//! operations). The debug tracing is not translated.

use std::borrow::Cow;

/* This limits sanitizing time on really broken fonts. */
pub(crate) const HB_SANITIZE_MAX_EDITS: u32 = 32;
pub(crate) const HB_SANITIZE_MAX_OPS_FACTOR: u32 = 64;
pub(crate) const HB_SANITIZE_MAX_OPS_MIN: u32 = 16384;
pub(crate) const HB_SANITIZE_MAX_OPS_MAX: u32 = 0x3FFFFFFF;
pub(crate) const HB_SANITIZE_MAX_SUBTABLES: i32 = 0x4000;

/// `hb_sanitize_context_t`
#[derive(Debug)]
pub(crate) struct HbSanitizeContext<'d> {
    /// the blob's data (writable once `writable` is set)
    pub(crate) data: Cow<'d, [u8]>,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) length: usize,
    pub(crate) max_ops: i32,
    pub(crate) max_subtables: i32,
    pub(crate) recursion_depth: i32,
    pub(crate) writable: bool,
    pub(crate) edit_count: u32,
    pub(crate) num_glyphs: u32,
    pub(crate) num_glyphs_set: bool,
    pub(crate) lazy_some_gpos: bool,
}

impl<'d> HbSanitizeContext<'d> {
    /// `hb_sanitize_context_t ()` with `init (b)`
    pub(crate) fn new(data: Cow<'d, [u8]>) -> HbSanitizeContext<'d> {
        HbSanitizeContext {
            data,
            start: 0,
            end: 0,
            length: 0,
            max_ops: 0,
            max_subtables: 0,
            recursion_depth: 0,
            writable: false,
            edit_count: 0,
            num_glyphs: 65536,
            num_glyphs_set: false,
            lazy_some_gpos: false,
        }
    }

    /// `visit_subtables`
    pub(crate) fn visit_subtables(&mut self, count: u32) -> bool {
        self.max_subtables = self.max_subtables.wrapping_add(count as i32);
        self.max_subtables < HB_SANITIZE_MAX_SUBTABLES
    }

    /// `set_num_glyphs`
    pub(crate) fn set_num_glyphs(&mut self, num_glyphs: u32) {
        self.num_glyphs = num_glyphs;
        self.num_glyphs_set = true;
    }
    /// `get_num_glyphs`
    pub(crate) fn get_num_glyphs(&self) -> u32 {
        self.num_glyphs
    }

    /// `set_object`: restricts the range to the object at `obj` of
    /// `size` bytes.
    pub(crate) fn set_object(&mut self, obj: usize, size: usize) {
        self.reset_object();
        if obj < self.start || self.end <= obj {
            self.start = 0;
            self.end = 0;
            self.length = 0;
        } else {
            self.start = obj;
            self.end = obj + (self.end - obj).min(size);
            self.length = self.end - self.start;
        }
    }

    /// `reset_object`
    pub(crate) fn reset_object(&mut self) {
        self.start = 0;
        self.end = self.data.len();
        self.length = self.end - self.start;
    }

    /// `start_processing`
    pub(crate) fn start_processing(&mut self) {
        self.reset_object();
        let len = (self.end - self.start) as u64;
        let m = len * HB_SANITIZE_MAX_OPS_FACTOR as u64;
        if len > u32::MAX as u64 || m > u32::MAX as u64 {
            self.max_ops = HB_SANITIZE_MAX_OPS_MAX as i32;
        } else {
            self.max_ops =
                (m as u32).clamp(HB_SANITIZE_MAX_OPS_MIN, HB_SANITIZE_MAX_OPS_MAX) as i32;
        }
        self.edit_count = 0;
        self.recursion_depth = 0;
    }

    /// `get_edit_count`
    pub(crate) fn get_edit_count(&self) -> u32 {
        self.edit_count
    }

    /// `check_ops`
    pub(crate) fn check_ops(&mut self, count: u32) -> bool {
        /* Avoid underflow */
        if self.max_ops < 0 || count >= self.max_ops as u32 {
            self.max_ops = -1;
            return false;
        }
        self.max_ops -= count as i32;
        true
    }

    /// `check_range`
    #[inline]
    pub(crate) fn check_range(&mut self, p: usize, len: u32) -> bool {
        p >= self.start && p - self.start <= self.length && ((self.end - p) as u32) >= len && {
            self.max_ops = self.max_ops.wrapping_sub(len as i32);
            self.max_ops > 0
        }
    }

    /// `check_range_fast`
    #[inline]
    pub(crate) fn check_range_fast(&self, p: usize, len: u32) -> bool {
        p >= self.start && p - self.start <= self.length && ((self.end - p) as u32) >= len
    }

    /// `check_point`
    #[inline]
    pub(crate) fn check_point(&self, p: usize) -> bool {
        p >= self.start && p - self.start <= self.length
    }

    /// `check_range (base, a, b)`
    #[inline]
    pub(crate) fn check_range2(&mut self, p: usize, a: u32, b: u32) -> bool {
        match a.checked_mul(b) {
            Some(m) => self.check_range(p, m),
            None => false,
        }
    }

    /// `check_range (base, a, b, c)`
    #[inline]
    pub(crate) fn check_range3(&mut self, p: usize, a: u32, b: u32, c: u32) -> bool {
        match a.checked_mul(b) {
            Some(m) => self.check_range2(p, m, c),
            None => false,
        }
    }

    /// `check_array_sized`
    #[inline]
    pub(crate) fn check_array_sized(
        &mut self,
        p: usize,
        len: u32,
        item_size: u32,
        len_size: u32,
    ) -> bool {
        let len = if len_size >= 4 {
            match len.checked_mul(item_size) {
                Some(l) => l,
                None => return false,
            }
        } else {
            len.wrapping_mul(item_size)
        };
        self.check_range(p, len)
    }

    /// `check_array`
    #[inline]
    pub(crate) fn check_array(&mut self, p: usize, len: u32, item_size: u32) -> bool {
        self.check_range2(p, len, item_size)
    }

    /// `check_start_recursion`
    pub(crate) fn check_start_recursion(&mut self, max_depth: i32) -> bool {
        if self.recursion_depth >= max_depth {
            return false;
        }
        self.recursion_depth += 1;
        self.recursion_depth != 0
    }

    /// `end_recursion`
    pub(crate) fn end_recursion(&mut self, result: bool) -> bool {
        self.recursion_depth -= 1;
        result
    }

    /// `check_struct` (of a structure of `min_size` bytes at `p`)
    #[inline]
    pub(crate) fn check_struct(&self, p: usize, min_size: usize) -> bool {
        /* (a 64-bit build) */
        self.check_point(p + min_size)
    }

    /// `may_edit`
    pub(crate) fn may_edit(&mut self, _p: usize, _len: u32) -> bool {
        if self.edit_count >= HB_SANITIZE_MAX_EDITS {
            return false;
        }
        self.edit_count += 1;
        self.writable
    }

    /// `try_set` of a 16-bit value
    pub(crate) fn try_set_u16(&mut self, p: usize, v: u16) -> bool {
        if self.may_edit(p, 2) {
            if let Some(b) = self.data.to_mut().get_mut(p..p + 2) {
                b.copy_from_slice(&v.to_be_bytes());
            }
            return true;
        }
        false
    }

    /// `try_set` of a 24-bit value
    pub(crate) fn try_set_u24(&mut self, p: usize, v: u32) -> bool {
        if self.may_edit(p, 3) {
            if let Some(b) = self.data.to_mut().get_mut(p..p + 3) {
                b.copy_from_slice(&v.to_be_bytes()[1..]);
            }
            return true;
        }
        false
    }

    /// `try_set` of a 32-bit value
    pub(crate) fn try_set_u32(&mut self, p: usize, v: u32) -> bool {
        if self.may_edit(p, 4) {
            if let Some(b) = self.data.to_mut().get_mut(p..p + 4) {
                b.copy_from_slice(&v.to_be_bytes());
            }
            return true;
        }
        false
    }

    /* Reading the data (only after the corresponding checks) */

    #[inline]
    pub(crate) fn u8(&self, p: usize) -> u8 {
        self.data.get(p).copied().unwrap_or(0)
    }
    #[inline]
    pub(crate) fn u16(&self, p: usize) -> u16 {
        match self.data.get(p..p + 2) {
            Some(b) => u16::from_be_bytes([b[0], b[1]]),
            None => 0,
        }
    }
    #[inline]
    pub(crate) fn i16(&self, p: usize) -> i16 {
        self.u16(p) as i16
    }
    #[inline]
    pub(crate) fn u24(&self, p: usize) -> u32 {
        match self.data.get(p..p + 3) {
            Some(b) => u32::from_be_bytes([0, b[0], b[1], b[2]]),
            None => 0,
        }
    }
    #[inline]
    pub(crate) fn u32(&self, p: usize) -> u32 {
        match self.data.get(p..p + 4) {
            Some(b) => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            None => 0,
        }
    }

    /* The basic types' sanitize() */

    /// `HBUINT16::sanitize` (and any 2-byte `IntType`)
    #[inline]
    pub(crate) fn sanitize_u16(&self, p: usize) -> bool {
        self.check_struct(p, 2)
    }
    /// `HBUINT32::sanitize`
    #[inline]
    pub(crate) fn sanitize_u32(&self, p: usize) -> bool {
        self.check_struct(p, 4)
    }

    /// `OffsetTo<Type, HBUINT16>::sanitize`: the offset at `p`, from
    /// `base`, its target sanitized by `f`; neutered (set to 0) if that
    /// fails.
    #[inline]
    pub(crate) fn sanitize_offset16(
        &mut self,
        p: usize,
        base: usize,
        f: impl FnOnce(&mut HbSanitizeContext<'d>, usize) -> bool,
    ) -> bool {
        if !self.check_struct(p, 2) {
            return false;
        }
        let off = self.u16(p) as usize;
        off == 0 || f(self, base + off) || self.try_set_u16(p, 0)
    }

    /// `OffsetTo<Type, HBUINT16, BaseType, false>::sanitize` (an offset
    /// without null: never neutered)
    #[inline]
    pub(crate) fn sanitize_nn_offset16(
        &mut self,
        p: usize,
        base: usize,
        f: impl FnOnce(&mut HbSanitizeContext<'d>, usize) -> bool,
    ) -> bool {
        if !self.check_struct(p, 2) {
            return false;
        }
        let off = self.u16(p) as usize;
        f(self, base + off)
    }

    /// `OffsetTo<Type, HBUINT24>::sanitize`
    #[inline]
    pub(crate) fn sanitize_offset24(
        &mut self,
        p: usize,
        base: usize,
        f: impl FnOnce(&mut HbSanitizeContext<'d>, usize) -> bool,
    ) -> bool {
        if !self.check_struct(p, 3) {
            return false;
        }
        let off = self.u24(p) as usize;
        off == 0 || f(self, base + off) || self.try_set_u24(p, 0)
    }

    /// `OffsetTo<Type, HBUINT32>::sanitize`
    #[inline]
    pub(crate) fn sanitize_offset32(
        &mut self,
        p: usize,
        base: usize,
        f: impl FnOnce(&mut HbSanitizeContext<'d>, usize) -> bool,
    ) -> bool {
        if !self.check_struct(p, 4) {
            return false;
        }
        let off = self.u32(p) as usize;
        off == 0 || f(self, base + off) || self.try_set_u32(p, 0)
    }

    /// `ArrayOf<Type, HBUINT16>::sanitize_shallow`: the array at `p`, of
    /// `item_size`-byte items.
    #[inline]
    pub(crate) fn sanitize_array16_shallow(&mut self, p: usize, item_size: u32) -> bool {
        self.sanitize_u16(p) && {
            let len = self.u16(p) as u32;
            self.check_array_sized(p + 2, len, item_size, 2)
        }
    }

    /// `ArrayOf<Type, HBUINT32>::sanitize_shallow`
    #[inline]
    pub(crate) fn sanitize_array32_shallow(&mut self, p: usize, item_size: u32) -> bool {
        self.sanitize_u32(p) && {
            let len = self.u32(p);
            self.check_array_sized(p + 4, len, item_size, 4)
        }
    }

    /// `ArrayOf<Type, HBUINT16>::sanitize (c, ...)`: the shallow check,
    /// then `f` on each item's position.
    pub(crate) fn sanitize_array16(
        &mut self,
        p: usize,
        item_size: u32,
        mut f: impl FnMut(&mut HbSanitizeContext<'d>, usize) -> bool,
    ) -> bool {
        if !self.sanitize_array16_shallow(p, item_size) {
            return false;
        }
        let count = self.u16(p) as usize;
        for i in 0..count {
            if !f(self, p + 2 + i * item_size as usize) {
                return false;
            }
        }
        true
    }

    /// `ArrayOf<Type, HBUINT32>::sanitize (c, ...)`
    pub(crate) fn sanitize_array32(
        &mut self,
        p: usize,
        item_size: u32,
        mut f: impl FnMut(&mut HbSanitizeContext<'d>, usize) -> bool,
    ) -> bool {
        if !self.sanitize_array32_shallow(p, item_size) {
            return false;
        }
        let count = self.u32(p) as usize;
        for i in 0..count {
            if !f(self, p + 4 + i * item_size as usize) {
                return false;
            }
        }
        true
    }

    /// `Array16Of<Offset16To<Type>>::sanitize (c, base)`: each offset,
    /// from `base`, sanitized with `f`.
    pub(crate) fn sanitize_array16_of_offset16(
        &mut self,
        p: usize,
        base: usize,
        mut f: impl FnMut(&mut HbSanitizeContext<'d>, usize) -> bool,
    ) -> bool {
        self.sanitize_array16(p, 2, |c, q| c.sanitize_offset16(q, base, &mut f))
    }

    /// `Array16Of<Offset32To<Type>>::sanitize (c, base)`
    pub(crate) fn sanitize_array16_of_offset32(
        &mut self,
        p: usize,
        base: usize,
        mut f: impl FnMut(&mut HbSanitizeContext<'d>, usize) -> bool,
    ) -> bool {
        self.sanitize_array16(p, 4, |c, q| c.sanitize_offset32(q, base, &mut f))
    }

    /// `UnsizedArrayOf<Type>::sanitize_shallow`
    #[inline]
    pub(crate) fn sanitize_unsized_array_shallow(
        &mut self,
        p: usize,
        count: u32,
        item_size: u32,
    ) -> bool {
        self.check_array(p, count, item_size)
    }

    /// `HeadlessArrayOf<Type, HBUINT16>::sanitize_shallow`
    pub(crate) fn sanitize_headless_array16_shallow(&mut self, p: usize, item_size: u32) -> bool {
        self.sanitize_u16(p) && {
            let len_p1 = self.u16(p) as u32;
            len_p1 == 0 || self.check_array_sized(p + 2, len_p1 - 1, item_size, 2)
        }
    }
}

/// `sanitize_blob`: sanitizes `data` with `sanitize` (the table type's
/// `sanitize()`), returning the sanitized (possibly edited) data, or the
/// empty blob if it fails.
pub(crate) fn hb_sanitize_blob(
    data: Vec<u8>,
    num_glyphs: Option<u32>,
    sanitize: impl Fn(&mut HbSanitizeContext<'_>) -> bool,
) -> Vec<u8> {
    hb_sanitize_blob_with(data, num_glyphs, |_| {}, sanitize)
}

/// `sanitize_blob` with a set-up of the context (`lazy_some_gpos` etc.)
pub(crate) fn hb_sanitize_blob_with(
    data: Vec<u8>,
    num_glyphs: Option<u32>,
    setup: impl Fn(&mut HbSanitizeContext<'_>),
    sanitize: impl Fn(&mut HbSanitizeContext<'_>) -> bool,
) -> Vec<u8> {
    let mut c = HbSanitizeContext::new(Cow::Owned(data));
    if let Some(n) = num_glyphs {
        c.set_num_glyphs(n);
    }
    setup(&mut c);
    let mut sane;

    loop {
        /* retry: */
        c.start_processing();

        if c.data.is_empty() {
            /* (C's `!start`: an empty blob is returned as is) */
            return c.data.into_owned();
        }

        sane = sanitize(&mut c);
        if sane {
            if c.edit_count != 0 {
                /* sanitize again to ensure no toe-stepping */
                c.edit_count = 0;
                sane = sanitize(&mut c);
                if c.edit_count != 0 {
                    sane = false;
                }
            }
        } else if c.edit_count != 0 && !c.writable {
            /* (the blob is copied: it can always be made writable) */
            c.writable = true;
            /* ok, we made it writable by relocating.  try again */
            continue;
        }
        break;
    }

    if sane {
        c.data.into_owned()
    } else {
        Vec::new()
    }
}
