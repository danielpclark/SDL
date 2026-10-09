// Rust translation of the glyph extents of src/OT/glyf/glyf.hh,
// src/OT/glyf/Glyph.hh and src/OT/glyf/GlyphHeader.hh, with
// src/OT/glyf/loca.hh, from HarfBuzz (8.5.0, as SDL_ttf's
// external/harfbuzz pins it).
// Copyright © 2015  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod, Garret Rieger, Roderick Sheeter

//! glyf -- TrueType Glyph Data, loca -- Index to Location
//! <https://docs.microsoft.com/en-us/typography/opentype/spec/glyf>
//! <https://docs.microsoft.com/en-us/typography/opentype/spec/loca>
//!
//! Translation notes: only the glyph extents without variations are
//! translated (the glyph headers' bounding boxes): the fonts these
//! functions serve have no variation coordinates in SDL_ttf (see
//! `hb_ot_hmtx_table`). The outlines (drawing, points, phantom points,
//! `gvar`) and the subsetter are not translated.

use super::hb_common::*;
use super::hb_face::HbFace;
use super::hb_font::{HbFont, HbGlyphExtents};
use super::hb_open_type::*;
use super::hb_ot_hmtx_table::HmtxVmtxAccel;
use super::hb_sanitize::hb_sanitize_blob;

/// `HB_OT_TAG_glyf`
pub(crate) const HB_OT_TAG_GLYF: HbTag = hb_tag(b'g', b'l', b'y', b'f');
/// `HB_OT_TAG_loca`
pub(crate) const HB_OT_TAG_LOCA: HbTag = hb_tag(b'l', b'o', b'c', b'a');

/// `glyf_accelerator_t`
#[derive(Debug)]
pub(crate) struct GlyfAccel {
    short_offset: bool,
    num_glyphs: u32,
    loca_table: Vec<u8>,
    glyf_table: Vec<u8>,
}

/// `Glyph::glyph_type_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GlyphType {
    Empty,
    Simple,
    Composite,
}

impl GlyfAccel {
    /// `glyf_accelerator_t (face)`, with `face->table.head`
    pub(crate) fn new(face: &HbFace) -> GlyfAccel {
        let mut accel = GlyfAccel {
            short_offset: false,
            num_glyphs: 0,
            loca_table: Vec::new(),
            glyf_table: Vec::new(),
        };
        let head = face.head();
        let index_to_loc_format = u16_at(head, 50);
        let glyph_data_format = u16_at(head, 52);
        /* glyf::has_valid_glyf_format */
        if !(index_to_loc_format <= 1 && glyph_data_format <= 1) {
            /* Unknown format.  Leave num_glyphs=0, that takes care of disabling us. */
            return accel;
        }
        accel.short_offset = 0 == index_to_loc_format;

        /* (loca::sanitize and glyf::sanitize accept anything) */
        accel.loca_table = hb_sanitize_blob(
            face.reference_table(HB_OT_TAG_LOCA),
            Some(face.get_num_glyphs()),
            |_| true,
        );
        accel.glyf_table = hb_sanitize_blob(
            face.reference_table(HB_OT_TAG_GLYF),
            Some(face.get_num_glyphs()),
            |_| true,
        );

        accel.num_glyphs =
            1u32.max(accel.loca_table.len() as u32 / if accel.short_offset { 2 } else { 4 }) - 1;
        accel.num_glyphs = accel.num_glyphs.min(face.get_num_glyphs());
        accel
    }

    /// `has_data`
    pub(crate) fn has_data(&self) -> bool {
        self.num_glyphs != 0
    }

    /// `glyph_for_gid`: the glyph's bytes (empty for the empty glyph)
    fn glyph_for_gid(&self, gid: HbCodepoint) -> &[u8] {
        if gid >= self.num_glyphs {
            return &[];
        }

        let (start_offset, end_offset) = if self.short_offset {
            (
                2 * u16_at(&self.loca_table, 2 * gid as usize) as u32,
                2 * u16_at(&self.loca_table, 2 * (gid as usize + 1)) as u32,
            )
        } else {
            (
                u32_at(&self.loca_table, 4 * gid as usize),
                u32_at(&self.loca_table, 4 * (gid as usize + 1)),
            )
        };

        if start_offset > end_offset || end_offset as usize > self.glyf_table.len() {
            return &[];
        }

        &self.glyf_table[start_offset as usize..end_offset as usize]
    }

    /// `get_extents` (without variation coordinates)
    pub(crate) fn get_extents(
        &self,
        font: &HbFont,
        hmtx: &HmtxVmtxAccel,
        gid: HbCodepoint,
        extents: &mut HbGlyphExtents,
    ) -> bool {
        if gid >= self.num_glyphs {
            return false;
        }

        /* Glyph::get_extents_without_var_scaled */
        let bytes = self.glyph_for_gid(gid);
        /* Glyph (bytes, gid): the header is the Null one for fewer than 10
         * bytes */
        let header: &[u8] = if bytes.len() < 10 { &[] } else { bytes };
        let num_contours = i16_at(header, 0) as i32;
        let glyph_type = if num_contours == 0 {
            GlyphType::Empty
        } else if num_contours > 0 {
            GlyphType::Simple
        } else if num_contours == -1 {
            GlyphType::Composite
        } else {
            /* (-2 is a VarComposite, but HB_NO_VAR_COMPOSITES is defined) */
            GlyphType::Empty // Spec deviation; Spec says COMPOSITE, but not seen in the wild.
        };
        if glyph_type == GlyphType::Empty {
            return true; /* Empty glyph; zero extents. */
        }

        /* GlyphHeader::get_extents_without_var_scaled */
        let x_min = i16_at(header, 2) as i32;
        let y_min = i16_at(header, 4) as i32;
        let x_max = i16_at(header, 6) as i32;
        let y_max = i16_at(header, 8) as i32;
        /* Undocumented rasterizer behavior: shift glyph to the left by (lsb - xMin), i.e., xMin = lsb */
        /* extents->x_bearing = hb_min (glyph_header.xMin, glyph_header.xMax); */
        let mut lsb = x_min.min(x_max);
        let _ = hmtx.get_leading_bearing_without_var_unscaled(gid, &mut lsb);
        extents.x_bearing = lsb;
        extents.y_bearing = y_min.max(y_max);
        extents.width = x_min.max(x_max) - x_min.min(x_max);
        extents.height = y_min.min(y_max) - y_min.max(y_max);

        font.scale_glyph_extents(extents);

        true
    }
}
