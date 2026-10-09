// Rust translation of src/hb-ft.h and src/hb-ft.cc (the font functions and
// font creation that SDL_ttf uses) and src/hb-cache.hh, from HarfBuzz
// (8.5.0, as SDL_ttf's external/harfbuzz pins it).
// Copyright © 2009  Red Hat, Inc.
// Copyright © 2009  Keith Stribley
// Copyright © 2015  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Red Hat Author(s): Behdad Esfahbod
// Google Author(s): Behdad Esfahbod

//! FreeType integration: font functions that query an `FT_Face`.
//!
//! In general, this file does a fine job of what it's supposed to do.
//! There are, however, things that need more work:
//!
//!   - FreeType works in 26.6 mode.  Clients can decide to use that mode,
//!     and everything would work fine.  However, we also abuse this API
//!     for performing in font-space, but don't pass the correct flags to
//!     FreeType.  We just abuse the no-hinting mode for that, such that no
//!     rounding etc happens.  As such, we don't set ppem, and pass
//!     NO_HINTING as load_flags.  Would be much better to use NO_SCALE, and
//!     scale ourselves.
//!
//!   - We don't handle / allow for emboldening / obliqueing.
//!
//!   - In the future, we should add constructors to create fonts in font
//!     space?
//!
//! Translation notes: the `FT_Face` is the caller's (see [`HbFont`]); the
//! face's tables are loaded with `FT_Load_Sfnt_Table` (C's
//! `_hb_ft_reference_table`; SDL_ttf's streams have a read function, so C
//! never takes the memory-blob path). The draw and paint functions,
//! `hb_ft_font_set_funcs` and `hb_ft_hb_font_changed` are not translated:
//! SDL_ttf does not use them.

use std::collections::HashMap;
use std::sync::Arc;

use super::hb_common::*;
use super::hb_face::*;
use super::hb_font::*;
use super::hb_ot_shaper_arabic_pua::{_hb_arabic_pua_simp_map, _hb_arabic_pua_trad_map};
use crate::freetype::base::ftadvanc::ft_get_advance;
use crate::freetype::base::ftcalc::ft_mul_fix;
use crate::freetype::base::ftmm::{ft_get_mm_var, ft_get_var_blend_coordinates};
use crate::freetype::base::ftobjs::*;
use crate::freetype::fttypes::*;

/// `hb_cache_t<16, 24, 8, false>`: a cache of 24-bit values for 16-bit
/// keys, with 256 slots.
#[derive(Debug, Clone)]
pub(crate) struct HbFtAdvanceCache {
    values: [i32; 256],
}

impl HbFtAdvanceCache {
    const KEY_BITS: u32 = 16;
    const VALUE_BITS: u32 = 24;
    const CACHE_BITS: u32 = 8;

    fn new() -> HbFtAdvanceCache {
        HbFtAdvanceCache { values: [-1; 256] }
    }

    /// `clear`
    pub(crate) fn clear(&mut self) {
        for v in self.values.iter_mut() {
            *v = -1;
        }
    }

    /// `get`
    fn get(&self, key: u32, value: &mut u32) -> bool {
        let k = key & ((1u32 << Self::CACHE_BITS) - 1);
        let v = self.values[k as usize] as u32;
        if v == u32::MAX || (v >> Self::VALUE_BITS) != (key >> Self::CACHE_BITS) {
            return false;
        }
        *value = v & ((1u32 << Self::VALUE_BITS) - 1);
        true
    }

    /// `set`
    fn set(&mut self, key: u32, value: u32) -> bool {
        if (key >> Self::KEY_BITS) != 0 || (value >> Self::VALUE_BITS) != 0 {
            return false; /* Overflows */
        }
        let k = key & ((1u32 << Self::CACHE_BITS) - 1);
        let v = ((key >> Self::CACHE_BITS) << Self::VALUE_BITS) | value;
        self.values[k as usize] = v as i32;
        true
    }
}

/// `hb_ft_font_t`
#[derive(Debug, Clone)]
pub struct HbFtFont {
    pub(crate) load_flags: i32,
    pub(crate) symbol: bool,    /* Whether selected cmap is symbol cmap. */
    pub(crate) transform: bool, /* Whether to apply FT_Face's transform. */

    pub(crate) cached_serial: u32,
    pub(crate) advance_cache: HbFtAdvanceCache,
}

/// `_hb_ft_font_create`
fn _hb_ft_font_create(symbol: bool) -> HbFtFont {
    HbFtFont {
        load_flags: FT_LOAD_DEFAULT | FT_LOAD_NO_HINTING,
        symbol,
        transform: false,
        cached_serial: u32::MAX,
        advance_cache: HbFtAdvanceCache::new(),
    }
}

/// The hb-ft data of a font (whose functions are hb-ft's).
fn ft_font<'b>(font: &'b mut HbFont) -> &'b mut HbFtFont {
    match &mut font.p.klass {
        HbFontKlass::Ft(f) => f,
        _ => unreachable!("not an hb-ft font"),
    }
}

/// `hb_ft_font_set_load_flags`
pub fn hb_ft_font_set_load_flags(font: &mut HbFontData, load_flags: i32) {
    if let HbFontKlass::Ft(ft_font) = &mut font.klass {
        ft_font.load_flags = load_flags;
    }
}

/// `hb_ft_font_get_load_flags`
pub fn hb_ft_font_get_load_flags(font: &HbFontData) -> i32 {
    match &font.klass {
        HbFontKlass::Ft(ft_font) => ft_font.load_flags,
        _ => 0,
    }
}

/// The x and y multipliers of the hb-ft functions (from the face's
/// transform, if hb-ft set one, with the font scale's sign).
fn ft_mults(font: &mut HbFont, want_x: bool, want_y: bool) -> (f32, f32) {
    let x_neg = font.p.x_scale < 0;
    let y_neg = font.p.y_scale < 0;
    let transform = ft_font(font).transform;
    let mut x_mult;
    let mut y_mult;
    if transform {
        let ft_face = font
            .ft_face
            .as_deref_mut()
            .expect("hb-ft font without its FT_Face");
        let (matrix, _) = ft_get_transform(ft_face);
        x_mult = if want_x {
            ((matrix.xx as f32) * (matrix.xx as f32) + (matrix.xy as f32) * (matrix.xy as f32))
                .sqrt()
                / 65536.0
        } else {
            1.0
        };
        x_mult *= if x_neg { -1.0 } else { 1.0 };
        y_mult = if want_y {
            ((matrix.yx as f32) * (matrix.yx as f32) + (matrix.yy as f32) * (matrix.yy as f32))
                .sqrt()
                / 65536.0
        } else {
            1.0
        };
        y_mult *= if y_neg { -1.0 } else { 1.0 };
    } else {
        x_mult = if x_neg { -1.0 } else { 1.0 };
        y_mult = if y_neg { -1.0 } else { 1.0 };
    }
    (x_mult, y_mult)
}

/// `hb_ft_get_nominal_glyph`
pub(crate) fn hb_ft_get_nominal_glyph(
    font: &mut HbFont,
    unicode: HbCodepoint,
    glyph: &mut HbCodepoint,
) -> bool {
    let symbol = ft_font(font).symbol;
    let face = font.p.face.clone();
    let ft_face = font
        .ft_face
        .as_deref_mut()
        .expect("hb-ft font without its FT_Face");
    let mut g = ft_get_char_index(ft_face, unicode as FtULong);

    if g == 0 {
        if symbol {
            match face.os2_get_font_page() {
                FONT_PAGE_NONE => {
                    if unicode <= 0x00FF {
                        /* For symbol-encoded OpenType fonts, we duplicate the
                         * U+F000..F0FF range at U+0000..U+00FF.  That's what
                         * Windows seems to do, and that's hinted about at:
                         * https://docs.microsoft.com/en-us/typography/opentype/spec/recom
                         * under "Non-Standard (Symbol) Fonts". */
                        g = ft_get_char_index(ft_face, (0xF000 + unicode) as FtULong);
                    }
                }
                FONT_PAGE_SIMP_ARABIC => {
                    g = ft_get_char_index(ft_face, _hb_arabic_pua_simp_map(unicode) as FtULong);
                }
                FONT_PAGE_TRAD_ARABIC => {
                    g = ft_get_char_index(ft_face, _hb_arabic_pua_trad_map(unicode) as FtULong);
                }
                _ => {}
            }
            if g == 0 {
                return false;
            }
        } else {
            return false;
        }
    }

    *glyph = g;
    true
}

/// `hb_ft_get_nominal_glyphs`
pub(crate) fn hb_ft_get_nominal_glyphs(
    font: &mut HbFont,
    unicodes: &[HbCodepoint],
    glyphs: &mut [HbCodepoint],
) -> u32 {
    let ft_face = font
        .ft_face
        .as_deref_mut()
        .expect("hb-ft font without its FT_Face");
    let mut done = 0;
    while done < unicodes.len() {
        glyphs[done] = ft_get_char_index(ft_face, unicodes[done] as FtULong);
        if glyphs[done] == 0 {
            break;
        }
        done += 1;
    }
    /* We don't need to do ft_font->symbol dance here, since HB calls the singular
     * nominal_glyph() for what we don't handle here. */
    done as u32
}

/// `hb_ft_get_variation_glyph`
pub(crate) fn hb_ft_get_variation_glyph(
    font: &mut HbFont,
    unicode: HbCodepoint,
    variation_selector: HbCodepoint,
    glyph: &mut HbCodepoint,
) -> bool {
    let ft_face = font
        .ft_face
        .as_deref_mut()
        .expect("hb-ft font without its FT_Face");
    let g =
        ft_face_get_char_variant_index(ft_face, unicode as FtULong, variation_selector as FtULong);

    if g == 0 {
        return false;
    }

    *glyph = g;
    true
}

/// `hb_ft_get_glyph_h_advances`
pub(crate) fn hb_ft_get_glyph_h_advances(
    font: &mut HbFont,
    glyphs: &[HbCodepoint],
    advances: &mut [HbPosition],
) {
    let (x_mult, _) = ft_mults(font, true, false);
    let load_flags = ft_font(font).load_flags;

    for i in 0..glyphs.len() {
        let mut v: FtFixed = 0;
        let glyph = glyphs[i];

        let mut cv = 0;
        if ft_font(font).advance_cache.get(glyph, &mut cv) {
            v = cv as FtFixed;
        } else {
            let ft_face = font
                .ft_face
                .as_deref_mut()
                .expect("hb-ft font without its FT_Face");
            if let Ok(a) = ft_get_advance(ft_face, glyph, load_flags) {
                v = a;
            }
            /* Work around bug that FreeType seems to return negative advance
             * for variable-set fonts if x_scale is negative! */
            v = (v as i32).wrapping_abs() as FtFixed;
            v = ((v as f32 * x_mult + (1 << 9) as f32) as i32 >> 10) as FtFixed;
            ft_font(font).advance_cache.set(glyph, v as u32);
        }

        advances[i] = v as HbPosition;
    }

    if font.p.x_strength != 0 && !font.p.embolden_in_place {
        /* Emboldening. */
        let x_strength = if font.p.x_scale >= 0 {
            font.p.x_strength
        } else {
            -font.p.x_strength
        };
        for a in advances.iter_mut() {
            *a += if *a != 0 { x_strength } else { 0 };
        }
    }
}

/// `hb_ft_get_glyph_v_advance`
pub(crate) fn hb_ft_get_glyph_v_advance(font: &mut HbFont, glyph: HbCodepoint) -> HbPosition {
    let (_, y_mult) = ft_mults(font, false, true);
    let load_flags = ft_font(font).load_flags;
    let ft_face = font
        .ft_face
        .as_deref_mut()
        .expect("hb-ft font without its FT_Face");

    let mut v = match ft_get_advance(ft_face, glyph, load_flags | FT_LOAD_VERTICAL_LAYOUT) {
        Ok(v) => v,
        Err(_) => return 0,
    };

    v = (y_mult * v as f32) as i32 as FtFixed;

    /* Note: FreeType's vertical metrics grows downward while other FreeType coordinates
     * have a Y growing upward.  Hence the extra negation. */

    let y_strength = if font.p.y_scale >= 0 {
        font.p.y_strength
    } else {
        -font.p.y_strength
    };
    (((-v + (1 << 9)) >> 10) as HbPosition)
        + if font.p.embolden_in_place {
            0
        } else {
            y_strength
        }
}

/// `hb_ft_get_glyph_v_origin`
pub(crate) fn hb_ft_get_glyph_v_origin(
    font: &mut HbFont,
    glyph: HbCodepoint,
    x: &mut HbPosition,
    y: &mut HbPosition,
) -> bool {
    let (x_mult, y_mult) = ft_mults(font, true, true);
    let load_flags = ft_font(font).load_flags;
    let ft_face = font
        .ft_face
        .as_deref_mut()
        .expect("hb-ft font without its FT_Face");

    if ft_load_glyph(ft_face, glyph, load_flags).is_err() {
        return false;
    }

    /* Note: FreeType's vertical metrics grows downward while other FreeType coordinates
     * have a Y growing upward.  Hence the extra negation. */
    let m = &ft_face.glyph.metrics;
    *x = (m.horiBearingX - m.vertBearingX) as HbPosition;
    *y = (m.horiBearingY - (-m.vertBearingY)) as HbPosition;

    *x = (x_mult * *x as f32) as HbPosition;
    *y = (y_mult * *y as f32) as HbPosition;

    true
}

/// `hb_ft_get_glyph_h_kerning`
pub(crate) fn hb_ft_get_glyph_h_kerning(
    font: &mut HbFont,
    left_glyph: HbCodepoint,
    right_glyph: HbCodepoint,
) -> HbPosition {
    let mode = if font.p.x_ppem != 0 {
        FT_KERNING_DEFAULT
    } else {
        FT_KERNING_UNFITTED
    };
    let ft_face = font
        .ft_face
        .as_deref_mut()
        .expect("hb-ft font without its FT_Face");
    match ft_get_kerning(ft_face, left_glyph, right_glyph, mode) {
        Ok(kerningv) => kerningv.x as HbPosition,
        Err(_) => 0,
    }
}

/// `hb_ft_get_glyph_extents`
pub(crate) fn hb_ft_get_glyph_extents(
    font: &mut HbFont,
    glyph: HbCodepoint,
    extents: &mut HbGlyphExtents,
) -> bool {
    let slant_xy = font.p.slant_xy;
    let (x_mult, y_mult) = ft_mults(font, true, true);
    let load_flags = ft_font(font).load_flags;
    let ft_face = font
        .ft_face
        .as_deref_mut()
        .expect("hb-ft font without its FT_Face");

    if ft_load_glyph(ft_face, glyph, load_flags).is_err() {
        return false;
    }

    /* Copied from hb_font_t::scale_glyph_extents. */

    let m = &ft_face.glyph.metrics;
    let mut x1 = x_mult * m.horiBearingX as f32;
    let y1 = y_mult * m.horiBearingY as f32;
    let mut x2 = x1 + x_mult * m.width as f32;
    let y2 = y1 + y_mult * -(m.height as f32);

    /* Apply slant. */
    if slant_xy != 0.0 {
        x1 += (y1 * slant_xy).min(y2 * slant_xy);
        x2 += (y1 * slant_xy).max(y2 * slant_xy);
    }

    extents.x_bearing = x1.floor() as HbPosition;
    extents.y_bearing = y1.floor() as HbPosition;
    extents.width = (x2.ceil() as HbPosition) - extents.x_bearing;
    extents.height = (y2.ceil() as HbPosition) - extents.y_bearing;

    if font.p.x_strength != 0 || font.p.y_strength != 0 {
        /* Y */
        let mut y_shift = font.p.y_strength;
        if font.p.y_scale < 0 {
            y_shift = -y_shift;
        }
        extents.y_bearing += y_shift;
        extents.height -= y_shift;

        /* X */
        let mut x_shift = font.p.x_strength;
        if font.p.x_scale < 0 {
            x_shift = -x_shift;
        }
        if font.p.embolden_in_place {
            extents.x_bearing -= x_shift / 2;
        }
        extents.width += x_shift;
    }

    true
}

/// `hb_ft_get_glyph_contour_point`
pub(crate) fn hb_ft_get_glyph_contour_point(
    font: &mut HbFont,
    glyph: HbCodepoint,
    point_index: u32,
    x: &mut HbPosition,
    y: &mut HbPosition,
) -> bool {
    let load_flags = ft_font(font).load_flags;
    let ft_face = font
        .ft_face
        .as_deref_mut()
        .expect("hb-ft font without its FT_Face");

    if ft_load_glyph(ft_face, glyph, load_flags).is_err() {
        return false;
    }

    if ft_face.glyph.format != FT_GLYPH_FORMAT_OUTLINE {
        return false;
    }

    if point_index >= ft_face.glyph.outline.n_points as u32 {
        return false;
    }

    *x = ft_face.glyph.outline.points[point_index as usize].x as HbPosition;
    *y = ft_face.glyph.outline.points[point_index as usize].y as HbPosition;

    true
}

/// `hb_ft_get_glyph_name` (the name, NUL excluded, of at most `size - 1`
/// bytes)
pub(crate) fn hb_ft_get_glyph_name(
    font: &mut HbFont,
    glyph: HbCodepoint,
    name: &mut Vec<u8>,
    size: usize,
) -> bool {
    let ft_face = font
        .ft_face
        .as_deref_mut()
        .expect("hb-ft font without its FT_Face");
    if size == 0 {
        return ft_get_glyph_name(ft_face, glyph, &mut []).is_ok();
    }
    let mut buf = vec![0u8; size];
    let mut ret = ft_get_glyph_name(ft_face, glyph, &mut buf).is_ok();
    if ret && buf[0] == 0 {
        ret = false;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    name.extend_from_slice(&buf[..end]);

    ret
}

/// `hb_ft_get_font_h_extents`
pub(crate) fn hb_ft_get_font_h_extents(font: &mut HbFont, metrics: &mut HbFontExtents) -> bool {
    let (_, y_mult) = ft_mults(font, false, true);
    let y_strength = font.p.y_strength;
    let ft_face = font
        .ft_face
        .as_deref_mut()
        .expect("hb-ft font without its FT_Face");

    if ft_face.units_per_EM != 0 {
        metrics.ascender =
            ft_mul_fix(ft_face.ascender as FtLong, ft_face.size.metrics.y_scale) as HbPosition;
        metrics.descender =
            ft_mul_fix(ft_face.descender as FtLong, ft_face.size.metrics.y_scale) as HbPosition;
        metrics.line_gap = (ft_mul_fix(ft_face.height as FtLong, ft_face.size.metrics.y_scale)
            as HbPosition)
            .wrapping_sub(metrics.ascender.wrapping_sub(metrics.descender));
    } else {
        /* Bitmap-only font, eg. color bitmap font. */
        metrics.ascender = ft_face.size.metrics.ascender as HbPosition;
        metrics.descender = ft_face.size.metrics.descender as HbPosition;
        metrics.line_gap = (ft_face.size.metrics.height as HbPosition)
            .wrapping_sub(metrics.ascender.wrapping_sub(metrics.descender));
    }

    metrics.ascender = (y_mult * (metrics.ascender + y_strength) as f32) as HbPosition;
    metrics.descender = (y_mult * metrics.descender as f32) as HbPosition;
    metrics.line_gap = (y_mult * metrics.line_gap as f32) as HbPosition;

    true
}

/// `_hb_ft_font_set_funcs`
fn _hb_ft_font_set_funcs(font: &mut HbFontData, ft_face: &FtFace) {
    let symbol = match ft_face.charmap {
        Some(cm) => ft_face.charmaps[cm].charmap.encoding == FT_ENCODING_MS_SYMBOL,
        None => false,
    };
    let ft_font = _hb_ft_font_create(symbol);

    /* hb_font_set_funcs */
    font.serial = font.serial.wrapping_add(1);
    font.klass = HbFontKlass::Ft(ft_font);
}

/// `_hb_ft_reference_table`
fn _hb_ft_reference_table(ft_face: &mut FtFace, tag: HbTag) -> Option<Vec<u8>> {
    let mut length: FtULong = 0;

    /* Note: FreeType like HarfBuzz uses the NONE tag for fetching the entire blob */

    if ft_load_sfnt_table(ft_face, tag as FtULong, 0, None, &mut length).is_err() {
        return None;
    }

    let mut buffer = vec![0u8; length as usize];

    if ft_load_sfnt_table(ft_face, tag as FtULong, 0, Some(&mut buffer), &mut length).is_err() {
        return None;
    }

    Some(buffer)
}

/// `hb_ft_face_create`: a face of the `FT_Face`'s tables.
pub fn hb_ft_face_create(ft_face: &mut FtFace) -> HbFace {
    let mut tables = HashMap::new();
    for &tag in HB_FACE_TABLES.iter() {
        if let Some(blob) = _hb_ft_reference_table(ft_face, tag) {
            tables.insert(tag, blob);
        }
    }

    HbFace::new_for_tables(
        tables,
        ft_face.face_index as u32,
        ft_face.units_per_EM as u32,
    )
}

/// `hb_ft_font_create`: an `hb_font_t` for the `FT_Face`.
pub fn hb_ft_font_create(ft_face: &mut FtFace) -> HbFontData {
    let face = Arc::new(hb_ft_face_create(ft_face));
    /* hb_font_create: (its hb-ot functions are replaced below; its named
     * instance, which hb_ft_font_changed overrides with the face's
     * coordinates, is not set) */
    let mut font = HbFontData::create(face);
    _hb_ft_font_set_funcs(&mut font, ft_face);
    hb_ft_font_changed(&mut font, ft_face);
    font
}

/// `hb_ft_font_changed`: refreshes the font's scale and variations from
/// the `FT_Face` (call when its size or variations change).
pub fn hb_ft_font_changed(font: &mut HbFontData, ft_face: &mut FtFace) {
    if !matches!(font.klass, HbFontKlass::Ft(_)) {
        return;
    }

    font.set_scale(
        (((ft_face.size.metrics.x_scale as u64).wrapping_mul(ft_face.units_per_EM as u64)
            + (1u64 << 15))
            >> 16) as i32,
        (((ft_face.size.metrics.y_scale as u64).wrapping_mul(ft_face.units_per_EM as u64)
            + (1u64 << 15))
            >> 16) as i32,
    );
    /* (hb-ft works in no-hinting model: the ppem is not set) */

    if let Ok(mm_var) = ft_get_mm_var(ft_face) {
        let num_axis = mm_var.num_axis as usize;
        let mut ft_coords = vec![0 as FtFixed; num_axis];
        let mut coords = vec![0i32; num_axis];
        if ft_get_var_blend_coordinates(ft_face, &mut ft_coords).is_ok() {
            let mut nonzero = false;

            for i in 0..num_axis {
                ft_coords[i] >>= 2;
                coords[i] = ft_coords[i] as i32;
                nonzero = nonzero || coords[i] != 0;
            }

            if nonzero {
                font.set_var_coords_normalized(&coords);
            } else {
                font.set_var_coords_normalized(&[]);
            }
        }
    }

    let serial = font.serial;
    if let HbFontKlass::Ft(ft_font) = &mut font.klass {
        ft_font.advance_cache.clear();
        ft_font.cached_serial = serial;
    }
}
