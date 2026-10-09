// Rust translation of src/hb-font.h, src/hb-font.hh and the parts of
// src/hb-font.cc that shaping uses, from HarfBuzz (8.5.0, as SDL_ttf's
// external/harfbuzz pins it).
// Copyright © 2009  Red Hat, Inc.
// Copyright © 2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Red Hat Author(s): Behdad Esfahbod
// Google Author(s): Behdad Esfahbod

//! `hb_font_t`: a font face at a size, with its font functions.
//!
//! Translation notes: a font's state (C's `hb_font_t`) is
//! [`HbFontData`]; its font functions (C's `klass`) are one of the sets
//! HarfBuzz has (FreeType's, from hb-ft, which SDL_ttf uses, or none:
//! the empty font's). As hb-ft's functions use the `FT_Face`, which the
//! caller owns, shaping takes an [`HbFont`]: the font's state together
//! with the `FT_Face`. The user-settable callbacks, parent fonts other
//! than the empty font, synthetic emboldening and slanting setters, the
//! draw and paint functions and the named-instance setter are not
//! translated: SDL_ttf uses none of them.

use std::sync::Arc;

use super::hb_common::*;
use super::hb_face::HbFace;
use super::hb_ft::HbFtFont;
use crate::freetype::base::ftobjs::FtFace;

/// `hb_font_extents_t`: font-wide extent values, measured in font units.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HbFontExtents {
    /// The height of typographic ascenders.
    pub ascender: HbPosition,
    /// The depth of typographic descenders.
    pub descender: HbPosition,
    /// The suggested line-spacing gap.
    pub line_gap: HbPosition,
}

/// `hb_glyph_extents_t`: glyph extent values, measured in font units.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HbGlyphExtents {
    /// Distance from the x-origin to the left extremum of the glyph.
    pub x_bearing: HbPosition,
    /// Distance from the top extremum of the glyph to the y-origin.
    pub y_bearing: HbPosition,
    /// Distance from the left extremum of the glyph to the right extremum.
    pub width: HbPosition,
    /// Distance from the top extremum of the glyph to the bottom extremum.
    pub height: HbPosition,
}

/// `HB_FONT_NO_VAR_NAMED_INSTANCE`
pub const HB_FONT_NO_VAR_NAMED_INSTANCE: u32 = 0xFFFFFFFF;

/// The font functions (`klass`) and their data (`user_data`).
#[derive(Debug, Clone)]
pub(crate) enum HbFontKlass {
    /// `hb_font_funcs_get_empty ()`: the nil functions
    Empty,
    /// hb-ft's functions
    Ft(HbFtFont),
}

/// `hb_font_t`: the state of a font.
#[derive(Debug, Clone)]
pub struct HbFontData {
    pub(crate) serial: u32,
    pub(crate) serial_coords: u32,

    pub(crate) face: Arc<HbFace>,

    pub(crate) x_scale: i32,
    pub(crate) y_scale: i32,

    pub(crate) x_embolden: f32,
    pub(crate) y_embolden: f32,
    pub(crate) embolden_in_place: bool,
    pub(crate) x_strength: i32, /* x_embolden, in scaled units. */
    pub(crate) y_strength: i32, /* y_embolden, in scaled units. */

    pub(crate) slant: f32,
    pub(crate) slant_xy: f32,

    pub(crate) x_multf: f32,
    pub(crate) y_multf: f32,
    pub(crate) x_mult: i64,
    pub(crate) y_mult: i64,

    pub(crate) x_ppem: u32,
    pub(crate) y_ppem: u32,

    pub(crate) ptem: f32,

    /* Font variation coordinates. */
    pub(crate) instance_index: u32,
    /// `coords` (2.14 normalized; `num_coords` is its length)
    pub(crate) coords: Vec<i32>,
    pub(crate) design_coords: Vec<f32>,

    pub(crate) klass: HbFontKlass,
}

/// A font being used: its state and the `FT_Face` its hb-ft functions
/// use.
#[derive(Debug)]
pub struct HbFont<'a> {
    pub(crate) p: &'a mut HbFontData,
    pub(crate) ft_face: Option<&'a mut FtFace>,
}

/// The Null font's state (the parent of every font): only its scale
/// matters, as the parent of the default functions.
const NULL_FONT_X_SCALE: i32 = 1000;
const NULL_FONT_Y_SCALE: i32 = 1000;

impl HbFontData {
    /// `_hb_font_create`
    pub(crate) fn create(face: Arc<HbFace>) -> HbFontData {
        let upem = face.get_upem() as i32;
        HbFontData {
            serial: 0,
            serial_coords: 0,
            face,
            x_scale: upem,
            y_scale: upem,
            x_embolden: 0.0,
            y_embolden: 0.0,
            embolden_in_place: true,
            x_strength: 0,
            y_strength: 0,
            slant: 0.0,
            slant_xy: 0.0,
            x_multf: 1.0,
            y_multf: 1.0,
            x_mult: 1 << 16,
            y_mult: 1 << 16,
            x_ppem: 0,
            y_ppem: 0,
            ptem: 0.0,
            instance_index: HB_FONT_NO_VAR_NAMED_INSTANCE,
            coords: Vec::new(),
            design_coords: Vec::new(),
            klass: HbFontKlass::Empty,
        }
    }

    /// `mults_changed`
    pub(crate) fn mults_changed(&mut self) {
        let upem = self.face.get_upem() as f32;

        self.x_multf = self.x_scale as f32 / upem;
        self.y_multf = self.y_scale as f32 / upem;
        let x_neg = self.x_scale < 0;
        self.x_mult = if x_neg {
            (-(((-(self.x_scale as i64)) << 16) as f32) / upem) as i64
        } else {
            (((self.x_scale as i64) << 16) as f32 / upem) as i64
        };
        let y_neg = self.y_scale < 0;
        self.y_mult = if y_neg {
            (-(((-(self.y_scale as i64)) << 16) as f32) / upem) as i64
        } else {
            (((self.y_scale as i64) << 16) as f32 / upem) as i64
        };

        self.x_strength = (self.x_scale as f32 * self.x_embolden).round().abs() as i32;
        self.y_strength = (self.y_scale as f32 * self.y_embolden).round().abs() as i32;

        self.slant_xy = if self.y_scale != 0 {
            self.slant * self.x_scale as f32 / self.y_scale as f32
        } else {
            0.0
        };
    }

    /// `hb_font_changed`
    pub fn changed(&mut self) {
        self.serial = self.serial.wrapping_add(1);
        self.mults_changed();
    }

    /// `hb_font_set_scale`
    pub fn set_scale(&mut self, x_scale: i32, y_scale: i32) {
        if self.x_scale == x_scale && self.y_scale == y_scale {
            return;
        }

        self.serial = self.serial.wrapping_add(1);

        self.x_scale = x_scale;
        self.y_scale = y_scale;
        self.mults_changed();
    }

    /// `hb_font_get_scale`
    pub fn get_scale(&self) -> (i32, i32) {
        (self.x_scale, self.y_scale)
    }

    /// `hb_font_set_ppem`
    pub fn set_ppem(&mut self, x_ppem: u32, y_ppem: u32) {
        if self.x_ppem == x_ppem && self.y_ppem == y_ppem {
            return;
        }

        self.serial = self.serial.wrapping_add(1);

        self.x_ppem = x_ppem;
        self.y_ppem = y_ppem;
    }

    /// `hb_font_set_ptem`
    pub fn set_ptem(&mut self, ptem: f32) {
        if self.ptem == ptem {
            return;
        }

        self.serial = self.serial.wrapping_add(1);

        self.ptem = ptem;
    }

    /// `hb_font_set_var_coords_normalized`
    ///
    /// C also simulates the design coordinates (with the 'avar' and
    /// 'fvar' tables); they are only read by hb-ft's
    /// `hb_ft_hb_font_changed`, which SDL_ttf does not call, so they are
    /// left at zero here.
    pub fn set_var_coords_normalized(&mut self, coords: &[i32]) {
        self.serial = self.serial.wrapping_add(1);
        self.serial_coords = self.serial;

        /* _hb_font_adopt_var_coords */
        self.coords = coords.to_vec();
        self.design_coords = vec![0.0; coords.len()];
        self.mults_changed(); // Easiest to call this to drop cached data
    }

    /// `hb_font_get_var_coords_normalized`
    pub fn get_var_coords_normalized(&self) -> &[i32] {
        &self.coords
    }

    /// `hb_font_get_face`
    pub fn get_face(&self) -> &Arc<HbFace> {
        &self.face
    }

    /// `num_coords`
    #[inline]
    pub(crate) fn num_coords(&self) -> u32 {
        self.coords.len() as u32
    }
}

impl<'a> HbFont<'a> {
    /// A font in use: its state, and the `FT_Face` of hb-ft's functions.
    pub fn new(p: &'a mut HbFontData, ft_face: Option<&'a mut FtFace>) -> HbFont<'a> {
        HbFont { p, ft_face }
    }

    /* Convert from font-space to user-space */

    /// `dir_mult`
    #[inline]
    pub(crate) fn dir_mult(&self, direction: HbDirection) -> i64 {
        if hb_direction_is_vertical(direction) {
            self.p.y_mult
        } else {
            self.p.x_mult
        }
    }
    /// `em_scale_x`
    #[inline]
    pub(crate) fn em_scale_x(&self, v: i16) -> HbPosition {
        Self::em_mult(v, self.p.x_mult)
    }
    /// `em_scale_y`
    #[inline]
    pub(crate) fn em_scale_y(&self, v: i16) -> HbPosition {
        Self::em_mult(v, self.p.y_mult)
    }
    /// `em_scalef_x`
    #[inline]
    pub(crate) fn em_scalef_x(&self, v: f32) -> HbPosition {
        Self::em_multf(v, self.p.x_multf)
    }
    /// `em_scalef_y`
    #[inline]
    pub(crate) fn em_scalef_y(&self, v: f32) -> HbPosition {
        Self::em_multf(v, self.p.y_multf)
    }
    /// `em_fscale_x`
    #[inline]
    pub(crate) fn em_fscale_x(&self, v: i16) -> f32 {
        Self::em_fmult(v, self.p.x_multf)
    }
    /// `em_fscale_y`
    #[inline]
    pub(crate) fn em_fscale_y(&self, v: i16) -> f32 {
        Self::em_fmult(v, self.p.y_multf)
    }
    /// `em_scale_dir`
    #[inline]
    pub(crate) fn em_scale_dir(&self, v: i16, direction: HbDirection) -> HbPosition {
        Self::em_mult(v, self.dir_mult(direction))
    }

    /// `em_mult`
    #[inline]
    pub(crate) fn em_mult(v: i16, mult: i64) -> HbPosition {
        ((v as i64).wrapping_mul(mult).wrapping_add(32768) >> 16) as HbPosition
    }
    /// `em_multf`
    #[inline]
    pub(crate) fn em_multf(v: f32, mult: f32) -> HbPosition {
        Self::em_fmultf(v, mult).round() as HbPosition
    }
    /// `em_fmultf`
    #[inline]
    pub(crate) fn em_fmultf(v: f32, mult: f32) -> f32 {
        v * mult
    }
    /// `em_fmult`
    #[inline]
    pub(crate) fn em_fmult(v: i16, mult: f32) -> f32 {
        v as f32 * mult
    }

    /* Convert from parent-font user-space to our user-space (the parent
     * is the empty font, of scale 1000) */

    /// `parent_scale_x_distance`
    #[inline]
    pub(crate) fn parent_scale_x_distance(&self, v: HbPosition) -> HbPosition {
        if NULL_FONT_X_SCALE != self.p.x_scale {
            return (v as i64 * self.p.x_scale as i64 / NULL_FONT_X_SCALE as i64) as HbPosition;
        }
        v
    }
    /// `parent_scale_y_distance`
    #[inline]
    pub(crate) fn parent_scale_y_distance(&self, v: HbPosition) -> HbPosition {
        if NULL_FONT_Y_SCALE != self.p.y_scale {
            return (v as i64 * self.p.y_scale as i64 / NULL_FONT_Y_SCALE as i64) as HbPosition;
        }
        v
    }

    /* Public getters */

    /// `get_font_h_extents`
    pub(crate) fn get_font_h_extents(&mut self, extents: &mut HbFontExtents) -> bool {
        *extents = HbFontExtents::default();
        match &self.p.klass {
            HbFontKlass::Ft(_) => super::hb_ft::hb_ft_get_font_h_extents(self, extents),
            /* hb_font_get_font_h_extents_nil */
            HbFontKlass::Empty => false,
        }
    }

    /// `get_font_v_extents`
    pub(crate) fn get_font_v_extents(&mut self, extents: &mut HbFontExtents) -> bool {
        *extents = HbFontExtents::default();
        /* hb-ft does not set it: hb_font_get_font_v_extents_default, whose
         * parent (the empty font) has none */
        false
    }

    /// `has_glyph`
    pub(crate) fn has_glyph(&mut self, unicode: HbCodepoint) -> bool {
        let mut glyph = 0;
        self.get_nominal_glyph(unicode, &mut glyph, 0)
    }

    /// `get_nominal_glyph`
    pub(crate) fn get_nominal_glyph(
        &mut self,
        unicode: HbCodepoint,
        glyph: &mut HbCodepoint,
        not_found: HbCodepoint,
    ) -> bool {
        *glyph = not_found;
        match &self.p.klass {
            HbFontKlass::Ft(_) => super::hb_ft::hb_ft_get_nominal_glyph(self, unicode, glyph),
            HbFontKlass::Empty => {
                *glyph = 0;
                false
            }
        }
    }

    /// `get_nominal_glyphs`: the glyphs of `unicodes` until the first
    /// one not found; returns the number found.
    pub(crate) fn get_nominal_glyphs(
        &mut self,
        unicodes: &[HbCodepoint],
        glyphs: &mut [HbCodepoint],
    ) -> u32 {
        match &self.p.klass {
            HbFontKlass::Ft(_) => super::hb_ft::hb_ft_get_nominal_glyphs(self, unicodes, glyphs),
            HbFontKlass::Empty => 0,
        }
    }

    /// `get_variation_glyph`
    pub(crate) fn get_variation_glyph(
        &mut self,
        unicode: HbCodepoint,
        variation_selector: HbCodepoint,
        glyph: &mut HbCodepoint,
        not_found: HbCodepoint,
    ) -> bool {
        *glyph = not_found;
        match &self.p.klass {
            HbFontKlass::Ft(_) => {
                super::hb_ft::hb_ft_get_variation_glyph(self, unicode, variation_selector, glyph)
            }
            HbFontKlass::Empty => {
                *glyph = 0;
                false
            }
        }
    }

    /// `get_glyph_h_advance`
    pub(crate) fn get_glyph_h_advance(&mut self, glyph: HbCodepoint) -> HbPosition {
        match &self.p.klass {
            /* hb_font_get_glyph_h_advance_default: the h_advances function is set */
            HbFontKlass::Ft(_) => {
                let mut ret = [0];
                self.get_glyph_h_advances(&[glyph], &mut ret);
                ret[0]
            }
            /* hb_font_get_glyph_h_advance_nil */
            HbFontKlass::Empty => self.p.x_scale,
        }
    }

    /// `get_glyph_v_advance`
    pub(crate) fn get_glyph_v_advance(&mut self, glyph: HbCodepoint) -> HbPosition {
        match &self.p.klass {
            HbFontKlass::Ft(_) => super::hb_ft::hb_ft_get_glyph_v_advance(self, glyph),
            /* TODO use font_extents.ascender+descender */
            HbFontKlass::Empty => self.p.y_scale,
        }
    }

    /// `get_glyph_h_advances`
    pub(crate) fn get_glyph_h_advances(
        &mut self,
        glyphs: &[HbCodepoint],
        advances: &mut [HbPosition],
    ) {
        match &self.p.klass {
            HbFontKlass::Ft(_) => super::hb_ft::hb_ft_get_glyph_h_advances(self, glyphs, advances),
            HbFontKlass::Empty => {
                /* hb_font_get_glyph_h_advances_default: the nil h_advance */
                for (a, &g) in advances.iter_mut().zip(glyphs) {
                    *a = self.get_glyph_h_advance(g);
                }
            }
        }
    }

    /// `get_glyph_v_advances`
    pub(crate) fn get_glyph_v_advances(
        &mut self,
        glyphs: &[HbCodepoint],
        advances: &mut [HbPosition],
    ) {
        /* hb_font_get_glyph_v_advances_default: the v_advance function is
         * set (hb-ft), or the nil one (the empty font) */
        for (a, &g) in advances.iter_mut().zip(glyphs) {
            *a = self.get_glyph_v_advance(g);
        }
    }

    /// `get_glyph_h_origin`
    pub(crate) fn get_glyph_h_origin(
        &mut self,
        _glyph: HbCodepoint,
        x: &mut HbPosition,
        y: &mut HbPosition,
    ) -> bool {
        *x = 0;
        *y = 0;
        match &self.p.klass {
            HbFontKlass::Ft(_) => {
                /* hb_font_get_glyph_h_origin_default: the parent's nil
                 * function (0, 0, true), scaled */
                *x = self.parent_scale_x_distance(*x);
                *y = self.parent_scale_y_distance(*y);
                true
            }
            /* hb_font_get_glyph_h_origin_nil */
            HbFontKlass::Empty => true,
        }
    }

    /// `get_glyph_v_origin`
    pub(crate) fn get_glyph_v_origin(
        &mut self,
        glyph: HbCodepoint,
        x: &mut HbPosition,
        y: &mut HbPosition,
    ) -> bool {
        *x = 0;
        *y = 0;
        match &self.p.klass {
            HbFontKlass::Ft(_) => super::hb_ft::hb_ft_get_glyph_v_origin(self, glyph, x, y),
            HbFontKlass::Empty => false,
        }
    }

    /// `get_glyph_h_kerning`
    pub(crate) fn get_glyph_h_kerning(
        &mut self,
        left_glyph: HbCodepoint,
        right_glyph: HbCodepoint,
    ) -> HbPosition {
        match &self.p.klass {
            HbFontKlass::Ft(_) => {
                super::hb_ft::hb_ft_get_glyph_h_kerning(self, left_glyph, right_glyph)
            }
            HbFontKlass::Empty => 0,
        }
    }

    /// `get_glyph_v_kerning`
    pub(crate) fn get_glyph_v_kerning(
        &mut self,
        _top_glyph: HbCodepoint,
        _bottom_glyph: HbCodepoint,
    ) -> HbPosition {
        /* hb-ft does not set it: the default, whose parent has none */
        0
    }

    /// `get_glyph_extents`
    pub(crate) fn get_glyph_extents(
        &mut self,
        glyph: HbCodepoint,
        extents: &mut HbGlyphExtents,
    ) -> bool {
        *extents = HbGlyphExtents::default();
        match &self.p.klass {
            HbFontKlass::Ft(_) => super::hb_ft::hb_ft_get_glyph_extents(self, glyph, extents),
            HbFontKlass::Empty => false,
        }
    }

    /// `get_glyph_contour_point`
    pub(crate) fn get_glyph_contour_point(
        &mut self,
        glyph: HbCodepoint,
        point_index: u32,
        x: &mut HbPosition,
        y: &mut HbPosition,
    ) -> bool {
        *x = 0;
        *y = 0;
        match &self.p.klass {
            HbFontKlass::Ft(_) => {
                super::hb_ft::hb_ft_get_glyph_contour_point(self, glyph, point_index, x, y)
            }
            HbFontKlass::Empty => false,
        }
    }

    /// `get_glyph_name`
    pub(crate) fn get_glyph_name(
        &mut self,
        glyph: HbCodepoint,
        name: &mut Vec<u8>,
        size: usize,
    ) -> bool {
        name.clear();
        match &self.p.klass {
            HbFontKlass::Ft(_) => super::hb_ft::hb_ft_get_glyph_name(self, glyph, name, size),
            HbFontKlass::Empty => false,
        }
    }

    /* A bit higher-level, and with fallback */

    /// `get_h_extents_with_fallback`
    pub(crate) fn get_h_extents_with_fallback(&mut self, extents: &mut HbFontExtents) {
        if !self.get_font_h_extents(extents) {
            extents.ascender = (self.p.y_scale as f64 * 0.8) as HbPosition;
            extents.descender = extents.ascender - self.p.y_scale;
            extents.line_gap = 0;
        }
    }
    /// `get_v_extents_with_fallback`
    pub(crate) fn get_v_extents_with_fallback(&mut self, extents: &mut HbFontExtents) {
        if !self.get_font_v_extents(extents) {
            extents.ascender = self.p.x_scale / 2;
            extents.descender = extents.ascender - self.p.x_scale;
            extents.line_gap = 0;
        }
    }

    /// `get_extents_for_direction`
    pub(crate) fn get_extents_for_direction(
        &mut self,
        direction: HbDirection,
        extents: &mut HbFontExtents,
    ) {
        if hb_direction_is_horizontal(direction) {
            self.get_h_extents_with_fallback(extents);
        } else {
            self.get_v_extents_with_fallback(extents);
        }
    }

    /// `get_glyph_advance_for_direction`
    pub(crate) fn get_glyph_advance_for_direction(
        &mut self,
        glyph: HbCodepoint,
        direction: HbDirection,
        x: &mut HbPosition,
        y: &mut HbPosition,
    ) {
        *x = 0;
        *y = 0;
        if hb_direction_is_horizontal(direction) {
            *x = self.get_glyph_h_advance(glyph);
        } else {
            *y = self.get_glyph_v_advance(glyph);
        }
    }

    /// `guess_v_origin_minus_h_origin`
    pub(crate) fn guess_v_origin_minus_h_origin(
        &mut self,
        glyph: HbCodepoint,
        x: &mut HbPosition,
        y: &mut HbPosition,
    ) {
        *x = self.get_glyph_h_advance(glyph) / 2;

        let mut extents = HbFontExtents::default();
        self.get_h_extents_with_fallback(&mut extents);
        *y = extents.ascender;
    }

    /// `get_glyph_h_origin_with_fallback`
    pub(crate) fn get_glyph_h_origin_with_fallback(
        &mut self,
        glyph: HbCodepoint,
        x: &mut HbPosition,
        y: &mut HbPosition,
    ) {
        if !self.get_glyph_h_origin(glyph, x, y) && self.get_glyph_v_origin(glyph, x, y) {
            let (mut dx, mut dy) = (0, 0);
            self.guess_v_origin_minus_h_origin(glyph, &mut dx, &mut dy);
            *x -= dx;
            *y -= dy;
        }
    }
    /// `get_glyph_v_origin_with_fallback`
    pub(crate) fn get_glyph_v_origin_with_fallback(
        &mut self,
        glyph: HbCodepoint,
        x: &mut HbPosition,
        y: &mut HbPosition,
    ) {
        if !self.get_glyph_v_origin(glyph, x, y) && self.get_glyph_h_origin(glyph, x, y) {
            let (mut dx, mut dy) = (0, 0);
            self.guess_v_origin_minus_h_origin(glyph, &mut dx, &mut dy);
            *x += dx;
            *y += dy;
        }
    }

    /// `get_glyph_origin_for_direction`
    pub(crate) fn get_glyph_origin_for_direction(
        &mut self,
        glyph: HbCodepoint,
        direction: HbDirection,
        x: &mut HbPosition,
        y: &mut HbPosition,
    ) {
        if hb_direction_is_horizontal(direction) {
            self.get_glyph_h_origin_with_fallback(glyph, x, y);
        } else {
            self.get_glyph_v_origin_with_fallback(glyph, x, y);
        }
    }

    /// `add_glyph_h_origin`
    pub(crate) fn add_glyph_h_origin(
        &mut self,
        glyph: HbCodepoint,
        x: &mut HbPosition,
        y: &mut HbPosition,
    ) {
        let (mut origin_x, mut origin_y) = (0, 0);
        self.get_glyph_h_origin_with_fallback(glyph, &mut origin_x, &mut origin_y);
        *x += origin_x;
        *y += origin_y;
    }
    /// `add_glyph_v_origin`
    pub(crate) fn add_glyph_v_origin(
        &mut self,
        glyph: HbCodepoint,
        x: &mut HbPosition,
        y: &mut HbPosition,
    ) {
        let (mut origin_x, mut origin_y) = (0, 0);
        self.get_glyph_v_origin_with_fallback(glyph, &mut origin_x, &mut origin_y);
        *x += origin_x;
        *y += origin_y;
    }
    /// `add_glyph_origin_for_direction`
    pub(crate) fn add_glyph_origin_for_direction(
        &mut self,
        glyph: HbCodepoint,
        direction: HbDirection,
        x: &mut HbPosition,
        y: &mut HbPosition,
    ) {
        let (mut origin_x, mut origin_y) = (0, 0);
        self.get_glyph_origin_for_direction(glyph, direction, &mut origin_x, &mut origin_y);
        *x += origin_x;
        *y += origin_y;
    }

    /// `subtract_glyph_h_origin`
    pub(crate) fn subtract_glyph_h_origin(
        &mut self,
        glyph: HbCodepoint,
        x: &mut HbPosition,
        y: &mut HbPosition,
    ) {
        let (mut origin_x, mut origin_y) = (0, 0);
        self.get_glyph_h_origin_with_fallback(glyph, &mut origin_x, &mut origin_y);
        *x -= origin_x;
        *y -= origin_y;
    }
    /// `subtract_glyph_v_origin`
    pub(crate) fn subtract_glyph_v_origin(
        &mut self,
        glyph: HbCodepoint,
        x: &mut HbPosition,
        y: &mut HbPosition,
    ) {
        let (mut origin_x, mut origin_y) = (0, 0);
        self.get_glyph_v_origin_with_fallback(glyph, &mut origin_x, &mut origin_y);
        *x -= origin_x;
        *y -= origin_y;
    }
    /// `subtract_glyph_origin_for_direction`
    pub(crate) fn subtract_glyph_origin_for_direction(
        &mut self,
        glyph: HbCodepoint,
        direction: HbDirection,
        x: &mut HbPosition,
        y: &mut HbPosition,
    ) {
        let (mut origin_x, mut origin_y) = (0, 0);
        self.get_glyph_origin_for_direction(glyph, direction, &mut origin_x, &mut origin_y);
        *x -= origin_x;
        *y -= origin_y;
    }

    /// `get_glyph_kerning_for_direction`
    pub(crate) fn get_glyph_kerning_for_direction(
        &mut self,
        first_glyph: HbCodepoint,
        second_glyph: HbCodepoint,
        direction: HbDirection,
        x: &mut HbPosition,
        y: &mut HbPosition,
    ) {
        if hb_direction_is_horizontal(direction) {
            *y = 0;
            *x = self.get_glyph_h_kerning(first_glyph, second_glyph);
        } else {
            *x = 0;
            *y = self.get_glyph_v_kerning(first_glyph, second_glyph);
        }
    }

    /// `get_glyph_extents_for_origin`
    pub(crate) fn get_glyph_extents_for_origin(
        &mut self,
        glyph: HbCodepoint,
        direction: HbDirection,
        extents: &mut HbGlyphExtents,
    ) -> bool {
        let ret = self.get_glyph_extents(glyph, extents);

        if ret {
            let (mut xb, mut yb) = (extents.x_bearing, extents.y_bearing);
            self.subtract_glyph_origin_for_direction(glyph, direction, &mut xb, &mut yb);
            extents.x_bearing = xb;
            extents.y_bearing = yb;
        }

        ret
    }

    /// `get_glyph_contour_point_for_origin`
    pub(crate) fn get_glyph_contour_point_for_origin(
        &mut self,
        glyph: HbCodepoint,
        point_index: u32,
        direction: HbDirection,
        x: &mut HbPosition,
        y: &mut HbPosition,
    ) -> bool {
        let ret = self.get_glyph_contour_point(glyph, point_index, x, y);

        if ret {
            self.subtract_glyph_origin_for_direction(glyph, direction, x, y);
        }

        ret
    }

    /// `glyph_to_string`: generates gidDDD if glyph has no name.
    pub(crate) fn glyph_to_string(&mut self, glyph: HbCodepoint, size: usize) -> Vec<u8> {
        let mut s = Vec::new();
        if self.get_glyph_name(glyph, &mut s, size) {
            return s;
        }
        let mut s = format!("gid{glyph}").into_bytes();
        s.truncate(size.saturating_sub(1));
        s
    }
}
