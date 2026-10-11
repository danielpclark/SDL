// Rust translation of src/hb-aat-layout-ankr-table.hh from HarfBuzz
// (8.5.0, as SDL_ttf's external/harfbuzz pins it).
// Copyright © 2018  Ebrahim Byagowi
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).

//! ankr -- Anchor Point
//! <https://developer.apple.com/fonts/TrueType-Reference-Manual/RM06/Chap6ankr.html>

use super::hb_aat_layout_common::Lookup;
use super::hb_common::*;
use super::hb_open_type::*;
use super::hb_sanitize::HbSanitizeContext;

/// `HB_AAT_TAG_ankr`
pub(crate) const HB_AAT_TAG_ANKR: HbTag = hb_tag(b'a', b'n', b'k', b'r');

/* Anchor: xCoordinate, yCoordinate (FWORD): 4 bytes. */

/* typedef Array32Of<Anchor> GlyphAnchors; */

/// `ankr::get_anchor`: the anchor `i` of the glyph (x, y; the Null
/// anchor, (0, 0), when it has none).
pub(crate) fn ankr_get_anchor(
    d: &[u8],
    glyph_id: HbCodepoint,
    i: u32,
    num_glyphs: u32,
) -> (i16, i16) {
    /* this+lookupTable (an Offset32To: Null when 0) */
    let lookup_off = u32_at(d, 4) as usize;
    if lookup_off == 0 {
        return (0, 0); /* (the Null lookup has no values) */
    }
    let Some(offset) = Lookup::new(d, lookup_off, 2).get_value(glyph_id, num_glyphs) else {
        return (0, 0);
    };
    /* &(this+anchorData) + *offset */
    let anchors = (u32_at(d, 8) as usize).wrapping_add(u16_at(d, offset) as usize);
    /* anchors[i] (Null past the end) */
    let len = u32_at(d, anchors);
    if i >= len {
        return (0, 0);
    }
    let a = anchors.wrapping_add(4 + i as usize * 4);
    (i16_at(d, a), i16_at(d, a.wrapping_add(2)))
}

/// `ankr::sanitize`
pub(crate) fn ankr_sanitize(c: &mut HbSanitizeContext) -> bool {
    c.check_struct(0, 12)
        && c.u16(0) == 0 /* version */
        && {
            let anchor_data = c.u32(8);
            c.check_range(0, anchor_data)
        }
        && {
            /* lookupTable.sanitize (c, this, &(this+anchorData)) */
            let anchor_data = c.u32(8) as usize;
            c.sanitize_offset32(4, 0, |c, l| {
                Lookup::sanitize_with(c, l, 2, &mut |c, v| {
                    /* NNOffset16To<GlyphAnchors>::sanitize (c, base): GlyphAnchors::sanitize_shallow */
                    c.sanitize_nn_offset16(v, anchor_data, |c, a| c.sanitize_array32_shallow(a, 4))
                })
            })
        }
}
