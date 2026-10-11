// Rust translation of src/hb-aat-ltag-table.hh from HarfBuzz (8.5.0, as
// SDL_ttf's external/harfbuzz pins it).
// Copyright © 2018  Ebrahim Byagowi
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).

//! ltag -- Language Tag
//! <https://developer.apple.com/fonts/TrueType-Reference-Manual/RM06/Chap6ltag.html>

use super::hb_common::*;
use super::hb_open_type::*;
use super::hb_sanitize::HbSanitizeContext;

/// `HB_AAT_TAG_ltag`
pub(crate) const HB_AAT_TAG_LTAG: HbTag = hb_tag(b'l', b't', b'a', b'g');

/* FTStringRange: tag (NNOffset16To<UnsizedArrayOf<HBUINT8>>, offset from
 * the start of the table to the beginning of the string), length
 * (HBUINT16, string length (in bytes)): 4 bytes. */

/// `FTStringRange::sanitize (c, base)`
fn ft_string_range_sanitize(c: &mut HbSanitizeContext, p: usize, base: usize) -> bool {
    c.check_struct(p, 4) && {
        let length = c.u16(p + 2) as u32;
        c.check_array(base + c.u16(p) as usize, length, 1)
    }
}

/// `ltag` (the sanitized table)
#[derive(Clone, Copy)]
pub(crate) struct Ltag<'a>(pub(crate) &'a [u8]);

impl Ltag<'_> {
    /// `get_language`
    pub(crate) fn get_language(&self, i: u32) -> HbLanguage {
        /* tagRanges[i] (the Null FTStringRange past the end) */
        let count = u32_at(self.0, 8);
        let (tag, length) = if i < count {
            let range = 12 + i as usize * 4;
            (
                u16_at(self.0, range) as usize,
                u16_at(self.0, range + 2) as usize,
            )
        } else {
            (0, 0)
        };
        /* hb_language_from_string ((const char *) (this+range.tag).arrayZ, range.length):
         * (a length is capped to the 63 bytes of its buffer) */
        let s = struct_at(self.0, tag);
        let len = length.min(63).min(s.len());
        hb_language_from_string(&s[..len])
    }

    /// `sanitize`
    pub(crate) fn sanitize(c: &mut HbSanitizeContext) -> bool {
        c.check_struct(0, 12) && c.u32(0) >= 1 && {
            /* tagRanges.sanitize (c, this) */
            c.sanitize_array32(8, 4, |c, p| ft_string_range_sanitize(c, p, 0))
        }
    }
}
