// Rust translation of src/hb-ot-hmtx-table.hh and src/hb-ot-hhea-table.hh
// (the parts the font functions use) from HarfBuzz (8.5.0, as SDL_ttf's
// external/harfbuzz pins it).
// Copyright © 2011,2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod, Roderick Sheeter

//! hmtx -- Horizontal Metrics, vmtx -- Vertical Metrics
//! <https://docs.microsoft.com/en-us/typography/opentype/spec/hmtx>
//! <https://docs.microsoft.com/en-us/typography/opentype/spec/vmtx>
//!
//! Translation notes: only the variation-less metrics are translated: the
//! fonts made with these functions (`hb_font_create`) never get
//! variation coordinates in SDL_ttf (FreeType's auto-hinter creates them
//! without), so `HVAR`/`VVAR` and the glyf phantom points are not used.
//! The subsetter's parts are not translated.

use super::hb_common::*;
use super::hb_face::HbFace;
use super::hb_open_type::*;
use super::hb_sanitize::hb_sanitize_blob;

/// `HB_OT_TAG_hmtx`
pub(crate) const HB_OT_TAG_HMTX: HbTag = hb_tag(b'h', b'm', b't', b'x');
/// `HB_OT_TAG_vmtx`
pub(crate) const HB_OT_TAG_VMTX: HbTag = hb_tag(b'v', b'm', b't', b'x');
/// `HB_OT_TAG_hhea`
pub(crate) const HB_OT_TAG_HHEA: HbTag = hb_tag(b'h', b'h', b'e', b'a');
/// `HB_OT_TAG_vhea`
pub(crate) const HB_OT_TAG_VHEA: HbTag = hb_tag(b'v', b'h', b'e', b'a');

/// `hheavhea`: the sanitized `hhea` or `vhea` table.
#[derive(Debug, Clone)]
pub(crate) struct HheaVhea(pub(crate) Vec<u8>);

impl HheaVhea {
    /// `face->table.hhea` (or `vhea`)
    pub(crate) fn new(face: &HbFace, tag: HbTag) -> HheaVhea {
        HheaVhea(hb_sanitize_blob(
            face.reference_table(tag),
            Some(face.get_num_glyphs()),
            |c| {
                /* hheavhea::sanitize */
                c.check_struct(0, 36) && c.u16(0) == 1
            },
        ))
    }

    /// `has_data`
    pub(crate) fn has_data(&self) -> bool {
        u16_at(&self.0, 0) != 0
    }

    /// `ascender`
    pub(crate) fn ascender(&self) -> i16 {
        i16_at(&self.0, 4)
    }

    /// `descender`
    pub(crate) fn descender(&self) -> i16 {
        i16_at(&self.0, 6)
    }

    /// `lineGap`
    pub(crate) fn line_gap(&self) -> i16 {
        i16_at(&self.0, 8)
    }

    /// `numberOfLongMetrics`
    pub(crate) fn number_of_long_metrics(&self) -> u32 {
        u16_at(&self.0, 34) as u32
    }
}

/// `hmtxvmtx<T, H, V>::accelerator_t`
#[derive(Debug)]
pub(crate) struct HmtxVmtxAccel {
    // 0 <= num_long_metrics <= num_bearings <= num_advances <= num_glyphs
    num_long_metrics: u32,
    num_bearings: u32,
    num_advances: u32,
    num_glyphs: u32,

    default_advance: u32,

    /// `table` (no sanitizing: "the users of the struct do all the hard
    /// work")
    table: Vec<u8>,
}

impl HmtxVmtxAccel {
    /// `accelerator_t (face)`: of `hmtx` (`horizontal`) or `vmtx`, with
    /// `hhea` or `vhea`.
    pub(crate) fn new(face: &HbFace, horizontal: bool, header: &HheaVhea) -> HmtxVmtxAccel {
        let table = hb_sanitize_blob(
            face.reference_table(if horizontal {
                HB_OT_TAG_HMTX
            } else {
                HB_OT_TAG_VMTX
            }),
            Some(face.get_num_glyphs()),
            |_| true,
        );

        let default_advance = if horizontal {
            face.get_upem() / 2
        } else {
            face.get_upem()
        };

        /* Populate count variables and sort them out as we go */

        let mut len = table.len() as u32;
        if len & 1 != 0 {
            len -= 1;
        }

        let mut num_long_metrics = header.number_of_long_metrics();
        if num_long_metrics * 4 > len {
            num_long_metrics = len / 4;
        }
        len -= num_long_metrics * 4;

        let mut num_bearings = face.get_num_glyphs();

        if num_bearings < num_long_metrics {
            num_bearings = num_long_metrics;
        }
        if (num_bearings - num_long_metrics) * 2 > len {
            num_bearings = num_long_metrics + len / 2;
        }
        len -= (num_bearings - num_long_metrics) * 2;

        /* We MUST set num_bearings to zero if num_long_metrics is zero.
         * Our get_advance() depends on that. */
        if num_long_metrics == 0 {
            num_bearings = 0;
            num_long_metrics = 0;
        }

        let num_advances = num_bearings + len / 2;
        let mut num_glyphs = face.get_num_glyphs();
        if num_glyphs < num_advances {
            num_glyphs = num_advances;
        }

        HmtxVmtxAccel {
            num_long_metrics,
            num_bearings,
            num_advances,
            num_glyphs,
            default_advance,
            table,
        }
    }

    /// `has_data`
    pub(crate) fn has_data(&self) -> bool {
        self.num_bearings != 0
    }

    /// `get_leading_bearing_without_var_unscaled`
    pub(crate) fn get_leading_bearing_without_var_unscaled(
        &self,
        glyph: HbCodepoint,
        lsb: &mut i32,
    ) -> bool {
        if glyph < self.num_long_metrics {
            *lsb = i16_at(&self.table, 4 * glyph as usize + 2) as i32;
            return true;
        }

        if glyph >= self.num_bearings {
            return false;
        }

        let bearings = 4 * self.num_long_metrics as usize;
        *lsb = i16_at(
            &self.table,
            bearings + 2 * (glyph - self.num_long_metrics) as usize,
        ) as i32;
        true
    }

    /// `get_leading_bearing_with_var_unscaled` (without variation
    /// coordinates)
    pub(crate) fn get_leading_bearing_with_var_unscaled(
        &self,
        glyph: HbCodepoint,
        lsb: &mut i32,
    ) -> bool {
        self.get_leading_bearing_without_var_unscaled(glyph, lsb)
    }

    /// `get_advance_without_var_unscaled`
    pub(crate) fn get_advance_without_var_unscaled(&self, glyph: HbCodepoint) -> u32 {
        /* OpenType case. */
        if glyph < self.num_bearings {
            return u16_at(
                &self.table,
                4 * glyph.min(self.num_long_metrics - 1) as usize,
            ) as u32;
        }

        /* If num_advances is zero, it means we don't have the metrics table
         * for this direction: return default advance.  Otherwise, there's a
         * well-defined answer. */
        if self.num_advances == 0 {
            return self.default_advance;
        }

        /* (HB_NO_BEYOND_64K is defined) */
        let _ = self.num_glyphs;
        0
    }

    /// `get_advance_with_var_unscaled` (without variation coordinates)
    pub(crate) fn get_advance_with_var_unscaled(&self, glyph: HbCodepoint) -> u32 {
        self.get_advance_without_var_unscaled(glyph)
    }
}
