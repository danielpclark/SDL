// Rust translation of src/hb-ot-font.cc, with the parts of
// src/hb-ot-metrics.cc (`_hb_ot_metrics_get_position_common`) and
// src/hb-ot-vorg-table.hh it uses, from HarfBuzz (8.5.0, as SDL_ttf's
// external/harfbuzz pins it).
// Copyright © 2011,2014  Google, Inc.
// Copyright © 2018-2019  Ebrahim Byagowi
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod, Roozbeh Pournader

//! hb-ot-font: the OpenType font functions, which fonts made with
//! `hb_font_create` use (in SDL_ttf, FreeType's auto-hinter's fonts).
//!
//! Translation notes: the face's tables for these functions (`table.cmap`,
//! `table.hmtx`, ...) are loaded together, on first use. The variation
//! deltas (`HVAR`, `VVAR`, `MVAR`, `gvar`; the fonts have no coordinates in
//! SDL_ttf), the caches, the glyph extents of color and bitmap glyphs
//! (`sbix`, `CBDT`, `COLR`) and of CFF glyphs, the glyph names (`post`,
//! `CFF`) and the draw and paint functions are not translated: those
//! glyphs have no extents here, and glyph names are unavailable.

use super::hb_common::*;
use super::hb_face::HbFace;
use super::hb_font::{HbFont, HbFontExtents, HbGlyphExtents};
use super::hb_open_type::*;
use super::hb_ot_cmap_table::CmapAccel;
use super::hb_ot_glyf_table::GlyfAccel;
use super::hb_ot_hmtx_table::*;
use super::hb_sanitize::hb_sanitize_blob;

/// `HB_OT_TAG_VORG`
pub(crate) const HB_OT_TAG_VORG: HbTag = hb_tag(b'V', b'O', b'R', b'G');

/// The tables HarfBuzz reads for the OpenType font functions (on top of
/// `HB_FACE_TABLES`).
pub(crate) const HB_OT_FONT_TABLES: [HbTag; 8] = [
    super::hb_ot_cmap_table::HB_OT_TAG_CMAP,
    HB_OT_TAG_HMTX,
    HB_OT_TAG_HHEA,
    HB_OT_TAG_VMTX,
    HB_OT_TAG_VHEA,
    super::hb_ot_glyf_table::HB_OT_TAG_GLYF,
    super::hb_ot_glyf_table::HB_OT_TAG_LOCA,
    HB_OT_TAG_VORG,
];

/// The face's tables for the OpenType font functions (`hb_ot_face_t`'s
/// `cmap`, `hmtx`, `vmtx`, `hhea`, `vhea`, `glyf` and `VORG`).
#[derive(Debug)]
pub(crate) struct HbOtFace {
    pub(crate) cmap: CmapAccel,
    pub(crate) hhea: HheaVhea,
    pub(crate) vhea: HheaVhea,
    pub(crate) hmtx: HmtxVmtxAccel,
    pub(crate) vmtx: HmtxVmtxAccel,
    pub(crate) glyf: GlyfAccel,
    /// the sanitized `VORG` table
    pub(crate) vorg: Vec<u8>,
}

impl HbOtFace {
    /// The face's tables, accelerated.
    pub(crate) fn new(face: &HbFace) -> HbOtFace {
        let hhea = HheaVhea::new(face, HB_OT_TAG_HHEA);
        let vhea = HheaVhea::new(face, HB_OT_TAG_VHEA);
        let hmtx = HmtxVmtxAccel::new(face, true, &hhea);
        let vmtx = HmtxVmtxAccel::new(face, false, &vhea);
        let vorg = hb_sanitize_blob(
            face.reference_table(HB_OT_TAG_VORG),
            Some(face.get_num_glyphs()),
            |c| {
                /* VORG::sanitize */
                c.check_struct(0, 8) && c.u16(0) == 1 && c.sanitize_array16_shallow(6, 4)
            },
        );
        HbOtFace {
            cmap: CmapAccel::new(face),
            hhea,
            vhea,
            hmtx,
            vmtx,
            glyf: GlyfAccel::new(face),
            vorg,
        }
    }

    /// `VORG::has_data`
    fn vorg_has_data(&self) -> bool {
        /* (version.to_int ()) */
        u32_at(&self.vorg, 0) != 0
    }

    /// `VORG::get_y_origin`
    fn vorg_get_y_origin(&self, glyph: HbCodepoint) -> i32 {
        let (len, items) = array16(struct_at(&self.vorg, 6));
        match hb_bsearch_impl(len as usize, |mid| {
            cmp_wide(glyph, u16_at(items, 4 * mid) as u32)
        }) {
            Ok(i) => i16_at(items, 4 * i as usize + 2) as i32,
            Err(_) => i16_at(&self.vorg, 4) as i32,
        }
    }
}

/// `hb_ot_font_t` (its caches are not translated)
#[derive(Debug, Clone, Default)]
pub(crate) struct HbOtFont;

/// `hb_ot_get_nominal_glyph`
pub(crate) fn hb_ot_get_nominal_glyph(
    font: &HbFont,
    unicode: HbCodepoint,
    glyph: &mut HbCodepoint,
) -> bool {
    font.p.face.ot().cmap.get_nominal_glyph(unicode, glyph)
}

/// `hb_ot_get_nominal_glyphs`
pub(crate) fn hb_ot_get_nominal_glyphs(
    font: &HbFont,
    unicodes: &[HbCodepoint],
    glyphs: &mut [HbCodepoint],
) -> u32 {
    font.p.face.ot().cmap.get_nominal_glyphs(unicodes, glyphs)
}

/// `hb_ot_get_variation_glyph`
pub(crate) fn hb_ot_get_variation_glyph(
    font: &HbFont,
    unicode: HbCodepoint,
    variation_selector: HbCodepoint,
    glyph: &mut HbCodepoint,
) -> bool {
    font.p
        .face
        .ot()
        .cmap
        .get_variation_glyph(unicode, variation_selector, glyph)
}

/// `hb_ot_get_glyph_h_advances`
pub(crate) fn hb_ot_get_glyph_h_advances(
    font: &HbFont,
    glyphs: &[HbCodepoint],
    advances: &mut [HbPosition],
) {
    let face = font.p.face.clone();
    let hmtx = &face.ot().hmtx;

    /* (no variation coordinates: the advance cache is not used) */
    for (glyph, advance) in glyphs.iter().zip(advances.iter_mut()) {
        *advance = font.em_scale_x(hmtx.get_advance_with_var_unscaled(*glyph) as i16);
    }

    if font.p.x_strength != 0 && !font.p.embolden_in_place {
        /* Emboldening. */
        let x_strength = if font.p.x_scale >= 0 {
            font.p.x_strength
        } else {
            -font.p.x_strength
        };
        for advance in advances.iter_mut().take(glyphs.len()) {
            *advance += if *advance != 0 { x_strength } else { 0 };
        }
    }
}

/// `hb_ot_get_glyph_v_advances`
pub(crate) fn hb_ot_get_glyph_v_advances(
    font: &mut HbFont,
    glyphs: &[HbCodepoint],
    advances: &mut [HbPosition],
) {
    let face = font.p.face.clone();
    let vmtx = &face.ot().vmtx;

    if vmtx.has_data() {
        for (glyph, advance) in glyphs.iter().zip(advances.iter_mut()) {
            *advance = font.em_scale_y(-(vmtx.get_advance_with_var_unscaled(*glyph) as i32) as i16);
        }
    } else {
        let mut font_extents = HbFontExtents::default();
        font.get_h_extents_with_fallback(&mut font_extents);
        let advance = -(font_extents.ascender - font_extents.descender);
        for a in advances.iter_mut().take(glyphs.len()) {
            *a = advance;
        }
    }

    if font.p.y_strength != 0 && !font.p.embolden_in_place {
        /* Emboldening. */
        let y_strength = if font.p.y_scale >= 0 {
            font.p.y_strength
        } else {
            -font.p.y_strength
        };
        for advance in advances.iter_mut().take(glyphs.len()) {
            *advance += if *advance != 0 { y_strength } else { 0 };
        }
    }
}

/// `hb_ot_get_glyph_v_origin`
pub(crate) fn hb_ot_get_glyph_v_origin(
    font: &mut HbFont,
    glyph: HbCodepoint,
    x: &mut HbPosition,
    y: &mut HbPosition,
) -> bool {
    let face = font.p.face.clone();
    let ot_face = face.ot();

    *x = font.get_glyph_h_advance(glyph) / 2;

    if ot_face.vorg_has_data() {
        let delta: f32 = 0.0;
        /* (no variation coordinates) */
        *y = font.em_scalef_y(ot_face.vorg_get_y_origin(glyph) as f32 + delta);
        return true;
    }

    let mut extents = HbGlyphExtents::default();
    if ot_face
        .glyf
        .get_extents(font, &ot_face.hmtx, glyph, &mut extents)
    {
        let vmtx = &ot_face.vmtx;
        let mut tsb = 0;
        if vmtx.get_leading_bearing_with_var_unscaled(glyph, &mut tsb) {
            *y = extents.y_bearing + font.em_scale_y(tsb as i16);
            return true;
        }

        let mut font_extents = HbFontExtents::default();
        font.get_h_extents_with_fallback(&mut font_extents);
        let advance = font_extents.ascender - font_extents.descender;
        let diff = advance - -extents.height;
        *y = extents.y_bearing + (diff >> 1);
        return true;
    }

    let mut font_extents = HbFontExtents::default();
    font.get_h_extents_with_fallback(&mut font_extents);
    *y = font_extents.ascender;

    true
}

/// `hb_ot_get_glyph_extents`
pub(crate) fn hb_ot_get_glyph_extents(
    font: &HbFont,
    glyph: HbCodepoint,
    extents: &mut HbGlyphExtents,
) -> bool {
    let face = font.p.face.clone();
    let ot_face = face.ot();

    /* (sbix, CBDT and COLR are not translated) */
    if ot_face
        .glyf
        .get_extents(font, &ot_face.hmtx, glyph, extents)
    {
        return true;
    }
    /* (CFF2 and CFF are not translated) */
    false
}

/// `_fix_ascender_descender`
fn _fix_ascender_descender(value: f32, metrics_tag: HbTag) -> f32 {
    if metrics_tag == HB_OT_METRICS_TAG_HORIZONTAL_ASCENDER
        || metrics_tag == HB_OT_METRICS_TAG_VERTICAL_ASCENDER
    {
        return (value as f64).abs() as f32;
    }
    if metrics_tag == HB_OT_METRICS_TAG_HORIZONTAL_DESCENDER
        || metrics_tag == HB_OT_METRICS_TAG_VERTICAL_DESCENDER
    {
        return -(value as f64).abs() as f32;
    }
    value
}

/// `HB_OT_METRICS_TAG_HORIZONTAL_ASCENDER`
pub(crate) const HB_OT_METRICS_TAG_HORIZONTAL_ASCENDER: HbTag = hb_tag(b'h', b'a', b's', b'c');
/// `HB_OT_METRICS_TAG_HORIZONTAL_DESCENDER`
pub(crate) const HB_OT_METRICS_TAG_HORIZONTAL_DESCENDER: HbTag = hb_tag(b'h', b'd', b's', b'c');
/// `HB_OT_METRICS_TAG_HORIZONTAL_LINE_GAP`
pub(crate) const HB_OT_METRICS_TAG_HORIZONTAL_LINE_GAP: HbTag = hb_tag(b'h', b'l', b'g', b'p');
/// `HB_OT_METRICS_TAG_VERTICAL_ASCENDER`
pub(crate) const HB_OT_METRICS_TAG_VERTICAL_ASCENDER: HbTag = hb_tag(b'v', b'a', b's', b'c');
/// `HB_OT_METRICS_TAG_VERTICAL_DESCENDER`
pub(crate) const HB_OT_METRICS_TAG_VERTICAL_DESCENDER: HbTag = hb_tag(b'v', b'd', b's', b'c');
/// `HB_OT_METRICS_TAG_VERTICAL_LINE_GAP`
pub(crate) const HB_OT_METRICS_TAG_VERTICAL_LINE_GAP: HbTag = hb_tag(b'v', b'l', b'g', b'p');

/// `_hb_ot_metrics_get_position_common` (hb-ot-metrics.cc)
pub(crate) fn _hb_ot_metrics_get_position_common(
    font: &HbFont,
    metrics_tag: HbTag,
    position: &mut HbPosition,
) -> bool {
    let face = font.p.face.clone();
    let ot_face = face.ot();
    /* GET_VAR: no variation coordinates, so MVAR gives .0f */
    let get_var = 0.0f32;
    let os2 = face.os2();
    /* OS2::has_data: usWeightClass || usWidthClass || usFirstCharIndex || usLastCharIndex */
    let os2_has_data =
        u16_at(os2, 4) != 0 || u16_at(os2, 6) != 0 || u16_at(os2, 64) != 0 || u16_at(os2, 66) != 0;
    /* OS2::use_typo_metrics: fsSelection & USE_TYPO_METRICS */
    let use_typo_metrics = u16_at(os2, 62) & (1 << 7) != 0;

    let mut get_metric_y = |has_data: bool, value: i16| -> bool {
        has_data && {
            *position =
                font.em_scalef_y(_fix_ascender_descender(value as f32 + get_var, metrics_tag));
            true
        }
    };

    match metrics_tag {
        HB_OT_METRICS_TAG_HORIZONTAL_ASCENDER => {
            (use_typo_metrics && get_metric_y(os2_has_data, i16_at(os2, 68)))
                || get_metric_y(ot_face.hhea.has_data(), ot_face.hhea.ascender())
        }
        HB_OT_METRICS_TAG_HORIZONTAL_DESCENDER => {
            (use_typo_metrics && get_metric_y(os2_has_data, i16_at(os2, 70)))
                || get_metric_y(ot_face.hhea.has_data(), ot_face.hhea.descender())
        }
        HB_OT_METRICS_TAG_HORIZONTAL_LINE_GAP => {
            (use_typo_metrics && get_metric_y(os2_has_data, i16_at(os2, 72)))
                || get_metric_y(ot_face.hhea.has_data(), ot_face.hhea.line_gap())
        }
        _ => {
            let value = match metrics_tag {
                HB_OT_METRICS_TAG_VERTICAL_ASCENDER => ot_face.vhea.ascender(),
                HB_OT_METRICS_TAG_VERTICAL_DESCENDER => ot_face.vhea.descender(),
                _ => ot_face.vhea.line_gap(),
            };
            /* GET_METRIC_X */
            ot_face.vhea.has_data() && {
                *position =
                    font.em_scalef_x(_fix_ascender_descender(value as f32 + get_var, metrics_tag));
                true
            }
        }
    }
}

/// `hb_ot_get_font_h_extents`
pub(crate) fn hb_ot_get_font_h_extents(font: &HbFont, metrics: &mut HbFontExtents) -> bool {
    let ret = _hb_ot_metrics_get_position_common(
        font,
        HB_OT_METRICS_TAG_HORIZONTAL_ASCENDER,
        &mut metrics.ascender,
    ) && _hb_ot_metrics_get_position_common(
        font,
        HB_OT_METRICS_TAG_HORIZONTAL_DESCENDER,
        &mut metrics.descender,
    ) && _hb_ot_metrics_get_position_common(
        font,
        HB_OT_METRICS_TAG_HORIZONTAL_LINE_GAP,
        &mut metrics.line_gap,
    );

    /* Embolden */
    let mut y_shift = font.p.y_strength;
    if font.p.y_scale < 0 {
        y_shift = -y_shift;
    }
    metrics.ascender += y_shift;

    ret
}

/// `hb_ot_get_font_v_extents`
pub(crate) fn hb_ot_get_font_v_extents(font: &HbFont, metrics: &mut HbFontExtents) -> bool {
    _hb_ot_metrics_get_position_common(
        font,
        HB_OT_METRICS_TAG_VERTICAL_ASCENDER,
        &mut metrics.ascender,
    ) && _hb_ot_metrics_get_position_common(
        font,
        HB_OT_METRICS_TAG_VERTICAL_DESCENDER,
        &mut metrics.descender,
    ) && _hb_ot_metrics_get_position_common(
        font,
        HB_OT_METRICS_TAG_VERTICAL_LINE_GAP,
        &mut metrics.line_gap,
    )
}
