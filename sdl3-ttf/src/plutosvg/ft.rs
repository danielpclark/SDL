// Rust translation of source/plutosvg-ft.h (and `plutosvg_ft_svg_hooks`
// of source/plutosvg.c, with PLUTOSVG_HAS_FREETYPE) from PlutoSVG (0.0.6,
// as SDL_ttf's external/plutosvg pins it).
// Copyright (c) 2020-2025 Samuel Ugochukwu <sammycageagle@gmail.com>
// This is an altered (translated) version of the original software; it is
// used under the MIT License (see PLUTOSVG-LICENSE and LICENSE.txt).

//! FreeType hooks for rendering SVG glyphs with PlutoSVG.
//!
//! This file implements the integration layer between FreeType's SVG
//! module (typically named "ot-svg") and PlutoSVG. It defines the
//! necessary functions to initialize and free the SVG rendering state,
//! render an SVG glyph into a glyph slot, load (and cache) SVG documents,
//! and pre-configure glyph slots with appropriate metrics and transforms.
//!
//! These functions are aggregated into the `plutosvg_ft_hooks` structure.
//!
//! Translation note: C's document cache recognizes a document by the
//! address and length of its data (an SVG table's document, or the
//! buffer FreeType decompresses a gzipped one into); this translation
//! recognizes it by its data and the face's units per EM. The two differ
//! only where C's addresses are reused for other data (FIXME (upstream):
//! C then renders the document it parsed before, from the new data).

use crate::freetype::base::ftcolor::{ft_palette_data_get, ft_palette_select};
use crate::freetype::base::ftobjs::*;
use crate::freetype::fttypes::*;
use crate::freetype::svg::ftsvg::{SvgRendererHooks, SvgState};
use crate::plutovg::canvas::*;
use crate::plutovg::matrix::*;
use crate::plutovg::surface::*;
use crate::plutovg::*;

use super::*;

/// `plutosvg_ft_document_entry_t`
#[derive(Debug)]
struct PlutosvgFtDocumentEntry {
    document: Document,
    units_per_em: FtUShort,
}

const PLUTOSVG_FT_MAX_DOCS: usize = 16;

/// `plutosvg_ft_state_t` (`document` is the index of the cached entry)
#[derive(Debug, Default)]
struct PlutosvgFtState {
    document: Option<usize>,
    matrix: Matrix,
    extents: Rect,
    entries: Vec<PlutosvgFtDocumentEntry>,
}

/// The state of the hooks (`*ft_state`).
fn ft_state(ft_state: &mut SvgState) -> Option<&mut PlutosvgFtState> {
    ft_state
        .as_mut()
        .and_then(|b| b.downcast_mut::<PlutosvgFtState>())
}

/// `plutosvg_ft_init`
fn plutosvg_ft_init(ft_state: &mut SvgState) -> FtResult<()> {
    let state = PlutosvgFtState::default();
    *ft_state = Some(Box::new(state));
    Ok(())
}

/// `plutosvg_ft_free`
fn plutosvg_ft_free(ft_state: &mut SvgState) {
    *ft_state = None;
}

const PLUTOSVG_FT_PALETTE_INDEX: FtUShort = 0;

/// `plutosvg_ft_palette_func`
fn plutosvg_ft_palette_func(ft_face: Option<&mut FtFace>, name: &[u8]) -> Option<Color> {
    let ft_face = ft_face?; /* FT_Palette_Data_Get( NULL ) fails */
    let length = name.len();
    if length < 5 || &name[..5] != b"color" {
        return None;
    }
    let ft_palette_data = ft_palette_data_get(ft_face).ok()?;
    let ft_palette = ft_palette_select(ft_face, PLUTOSVG_FT_PALETTE_INDEX).ok()?;

    let mut index: FtInt = 0;
    for &ch in &name[5..] {
        if !ch.is_ascii_digit() {
            return None;
        }
        index = index.wrapping_mul(10).wrapping_add((ch - b'0') as FtInt);
    }

    if index >= ft_palette_data.num_palette_entries as FtInt {
        return None;
    }
    /* FIXME (upstream): C reads before the palette for the negative
    indices a long digit string overflows to */
    let ft_color = ft_palette?.get(usize::try_from(index).ok()?)?;
    Some(Color {
        r: ft_color.red as f32 / 255.0,
        g: ft_color.green as f32 / 255.0,
        b: ft_color.blue as f32 / 255.0,
        a: ft_color.alpha as f32 / 255.0,
    })
}

/// `sprintf(buffer, "glyph%u", glyph_index)`
fn glyph_id(glyph_index: FtUInt) -> Vec<u8> {
    format!("glyph{glyph_index}").into_bytes()
}

/// `plutosvg_ft_render`
fn plutosvg_ft_render(
    ft_slot: &mut FtGlyphSlotRec,
    face: Option<&mut FtFace>,
    ft_state: &mut SvgState,
) -> FtResult<()> {
    let Some(state) = self::ft_state(ft_state) else {
        return Err(FT_ERR_INVALID_SVG_DOCUMENT);
    };
    let Some(document) = state.document else {
        return Err(FT_ERR_INVALID_SVG_DOCUMENT);
    };
    let buffer = std::mem::take(&mut ft_slot.bitmap.buffer);
    let surface = plutovg_surface_create_for_data(
        buffer,
        ft_slot.bitmap.width as i32,
        ft_slot.bitmap.rows as i32,
        ft_slot.bitmap.pitch,
    );
    let mut canvas = plutovg_canvas_create(surface);

    let (start_glyph_id, end_glyph_id) = match ft_slot.other.as_ref() {
        Some(d) => (d.start_glyph_id, d.end_glyph_id),
        None => (0, 0),
    };

    let id = if start_glyph_id < end_glyph_id {
        Some(glyph_id(ft_slot.glyph_index))
    } else {
        None
    };

    plutovg_canvas_translate(&mut canvas, -state.extents.x, -state.extents.y);
    plutovg_canvas_transform(&mut canvas, &state.matrix);
    let mut face = face;
    let mut palette = |name: &[u8]| plutosvg_ft_palette_func(face.as_deref_mut(), name);
    plutosvg_document_render(
        &state.entries[document].document,
        id.as_deref(),
        &mut canvas,
        None,
        Some(&mut palette),
    );

    ft_slot.bitmap.pixel_mode = FT_PIXEL_MODE_BGRA;
    ft_slot.bitmap.num_grays = 256;
    ft_slot.format = FT_GLYPH_FORMAT_BITMAP;

    ft_slot.bitmap.buffer = plutovg_surface_into_data(plutovg_canvas_into_surface(canvas));
    state.document = None;
    Ok(())
}

/// `plutosvg_ft_document_load`: the index of the (cached) document.
fn plutosvg_ft_document_load(
    state: &mut PlutosvgFtState,
    data: &[u8],
    units_per_em: FtUShort,
) -> Option<usize> {
    for i in 0..state.entries.len() {
        if state.entries[i].units_per_em == units_per_em && state.entries[i].document.data == data {
            let entry = state.entries.remove(i);
            state.entries.insert(0, entry);
            return Some(0);
        }
    }

    let document =
        plutosvg_document_load_from_data(data, units_per_em as f32, units_per_em as f32)?;
    if state.entries.len() == PLUTOSVG_FT_MAX_DOCS {
        state.entries.pop();
    }

    state.entries.insert(
        0,
        PlutosvgFtDocumentEntry {
            document,
            units_per_em,
        },
    );
    Some(0)
}

/// `plutosvg_ft_preset_slot`
fn plutosvg_ft_preset_slot(
    ft_slot: &mut FtGlyphSlotRec,
    ft_cache: bool,
    ft_state: &mut SvgState,
) -> FtResult<()> {
    let Some(state) = self::ft_state(ft_state) else {
        return Err(FT_ERR_INVALID_SVG_DOCUMENT);
    };

    let Some(ft_document) = ft_slot.other.as_ref() else {
        return Err(FT_ERR_INVALID_SVG_DOCUMENT);
    };
    let ft_metrics = ft_document.metrics;

    let start_glyph_id = ft_document.start_glyph_id;
    let end_glyph_id = ft_document.end_glyph_id;

    let length = (ft_document.svg_document_length as usize).min(ft_document.svg_document.len());
    let Some(document) = plutosvg_ft_document_load(
        state,
        &ft_document.svg_document[..length],
        ft_document.units_per_EM,
    ) else {
        return Err(FT_ERR_INVALID_SVG_DOCUMENT);
    };

    let doc = &state.entries[document].document;
    let document_width = plutosvg_document_get_width(doc);
    let document_height = plutosvg_document_get_height(doc);

    let transform = Matrix {
        a: ft_document.transform.xx as f32 / (1 << 16) as f32,
        b: -(ft_document.transform.xy as f32) / (1 << 16) as f32,
        c: -(ft_document.transform.yx as f32) / (1 << 16) as f32,
        d: ft_document.transform.yy as f32 / (1 << 16) as f32,
        e: ft_document.delta.x as f32 / 64.0 * document_width / ft_metrics.x_ppem as f32,
        f: -(ft_document.delta.y as f32) / 64.0 * document_height / ft_metrics.y_ppem as f32,
    };

    let x_svg_to_out = ft_metrics.x_ppem as f32 / document_width;
    let y_svg_to_out = ft_metrics.y_ppem as f32 / document_height;

    let mut matrix = Matrix::default();
    plutovg_matrix_init_scale(&mut matrix, x_svg_to_out, y_svg_to_out);
    let right = matrix;
    plutovg_matrix_multiply(&mut matrix, &transform, &right);

    let id = if start_glyph_id < end_glyph_id {
        Some(glyph_id(ft_slot.glyph_index))
    } else {
        None
    };

    let mut extents = Rect::default();
    if !plutosvg_document_extents(doc, id.as_deref(), &mut extents) {
        return Err(FT_ERR_INVALID_SVG_DOCUMENT);
    }

    let extents = plutovg_matrix_map_rect(&matrix, &extents);
    ft_slot.bitmap_left = cvt_f32_i32(extents.x);
    ft_slot.bitmap_top = cvt_f32_i32(-extents.y);

    ft_slot.bitmap.rows = cvt_f32_u32(extents.h.ceil());
    ft_slot.bitmap.width = cvt_f32_u32(extents.w.ceil());
    ft_slot.bitmap.pitch = (ft_slot.bitmap.width as i32).wrapping_mul(4);
    ft_slot.bitmap.pixel_mode = FT_PIXEL_MODE_BGRA;

    let metrics_width = extents.w;
    let metrics_height = extents.h;

    let hori_bearing_x = extents.x;
    let hori_bearing_y = -extents.y;

    let vert_bearing_x = ft_slot.metrics.horiBearingX as f32 / 64.0
        - ft_slot.metrics.horiAdvance as f32 / 64.0 / 2.0;
    let vert_bearing_y =
        (ft_slot.metrics.vertAdvance as f32 / 64.0 - ft_slot.metrics.height as f32 / 64.0) / 2.0;

    ft_slot.metrics.width = cvt_f32_i64((metrics_width * 64.0).round());
    ft_slot.metrics.height = cvt_f32_i64((metrics_height * 64.0).round());

    ft_slot.metrics.horiBearingX = cvt_f32_i64(hori_bearing_x * 64.0);
    ft_slot.metrics.horiBearingY = cvt_f32_i64(hori_bearing_y * 64.0);
    ft_slot.metrics.vertBearingX = cvt_f32_i64(vert_bearing_x * 64.0);
    ft_slot.metrics.vertBearingY = cvt_f32_i64(vert_bearing_y * 64.0);
    if ft_slot.metrics.vertAdvance == 0 {
        ft_slot.metrics.vertAdvance = cvt_f32_i64(metrics_height * 1.2 * 64.0);
    }
    if ft_cache {
        state.document = Some(document);
        state.extents = extents;
        state.matrix = matrix;
    }

    Ok(())
}

/// FreeType SVG renderer hooks (`plutosvg_ft_hooks`).
///
/// This structure is passed to FreeType via FT_Property_Set to delegate SVG
/// glyph rendering to PlutoSVG.
static PLUTOSVG_FT_HOOKS: SvgRendererHooks = SvgRendererHooks {
    init_svg: Some(plutosvg_ft_init),
    free_svg: Some(plutosvg_ft_free),
    render_svg: Some(plutosvg_ft_render),
    preset_slot: Some(plutosvg_ft_preset_slot),
};

/// `plutosvg_ft_svg_hooks`: the hooks to set with the `ot-svg` module's
/// `svg-hooks` property.
pub fn plutosvg_ft_svg_hooks() -> &'static SvgRendererHooks {
    &PLUTOSVG_FT_HOOKS
}
