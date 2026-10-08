// Rust translation of src/SDL_surface_textengine.c from SDL_ttf.
// Copyright (C) 2001-2025 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The text engine for drawing text on SDL surfaces.
//!
//! Translation notes: the glyph data's reference counts are `Rc`s, and the
//! hash tables (`SDL_HashTable`, keyed by font pointers) are `HashMap`s
//! keyed by [`FontHandle`]s.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use sdl3::video::{Color, FColor, Rect, Surface};
use sdl3::{Error, Result};

use crate::text::*;
use crate::ttf::ImageType;

/// `TTF_SurfaceTextEngineGlyphData` (its `refcount` is the `Rc`'s)
#[derive(Debug)]
struct SurfaceTextEngineGlyphData {
    color: Color,
    surface: Surface<'static>,
    image_type: ImageType,
}

type GlyphData = Rc<RefCell<SurfaceTextEngineGlyphData>>;

/// `TTF_SurfaceTextEngineTextData`
#[derive(Debug)]
struct SurfaceTextEngineTextData {
    fcolor: FColor,
    color: Color,
    /// the drawing operations, with their glyphs (`op->copy.reserved`)
    ops: Vec<(DrawOperation, Option<GlyphData>)>,
}

/// `TTF_SurfaceTextEngineFontData`
#[derive(Debug)]
struct SurfaceTextEngineFontData {
    /* font: the key */
    generation: u32,
    glyphs: HashMap<(FontHandle, u32), GlyphData>,
}

/// The text engine for drawing text on SDL surfaces
/// (`TTF_SurfaceTextEngineData`, as the `userdata` of a `TTF_TextEngine`).
#[derive(Debug, Default)]
pub struct SurfaceTextEngine {
    fonts: RefCell<HashMap<FontHandle, SurfaceTextEngineFontData>>,
}

/// `CreateGlyphData`
fn create_glyph_data(surface: Surface<'static>, image_type: ImageType) -> GlyphData {
    Rc::new(RefCell::new(SurfaceTextEngineGlyphData {
        color: Color::new(0xFF, 0xFF, 0xFF, 0xFF),
        surface,
        image_type,
    }))
}

/// `GetGlyphData`
fn get_glyph_data(
    fontdata: &mut SurfaceTextEngineFontData,
    glyph_font: &FontHandle,
    glyph_index: u32,
) -> Result<GlyphData> {
    let key = (glyph_font.clone(), glyph_index);
    if let Some(data) = fontdata.glyphs.get(&key) {
        return Ok(data.clone());
    }

    let (surface, image_type) = glyph_font.glyph_image_for_index(glyph_index)?;
    let data = create_glyph_data(surface, image_type);
    fontdata.glyphs.insert(key, data.clone());
    Ok(data)
}

/// `CreateTextData`
fn create_text_data(
    fontdata: &mut SurfaceTextEngineFontData,
    ops: &[DrawOperation],
) -> Result<SurfaceTextEngineTextData> {
    let mut data = SurfaceTextEngineTextData {
        fcolor: FColor::default(),
        color: Color::new(0, 0, 0, 0),
        ops: Vec::new(),
    };

    if data.ops.try_reserve_exact(ops.len()).is_err() {
        return Err(Error::out_of_memory());
    }
    for op in ops {
        let glyph = match op {
            DrawOperation::Copy(copy) => Some(get_glyph_data(
                fontdata,
                &copy.glyph_font,
                copy.glyph_index,
            )?),
            _ => None,
        };
        data.ops.push((op.clone(), glyph));
    }
    Ok(data)
}

impl SurfaceTextEngine {
    /// Create a text engine for drawing text on SDL surfaces. Translation
    /// of `TTF_CreateSurfaceTextEngine()` (dropping the last reference is
    /// `TTF_DestroySurfaceTextEngine()`).
    pub fn new() -> Result<Rc<SurfaceTextEngine>> {
        Ok(Rc::new(SurfaceTextEngine::default()))
    }
}

impl TextEngine for SurfaceTextEngine {
    /// `CreateText`
    fn create_text(&self, text: &TextData) -> Result<Box<dyn Any>> {
        let font = text.font().ok_or_else(|| Error::invalid_param("font"))?;
        let font_generation = font.generation();
        let ops = text.ops();

        let mut fonts = self.fonts.borrow_mut();
        if let Some(fontdata) = fonts.get_mut(&font) {
            if font_generation != fontdata.generation {
                fontdata.glyphs.clear();
                fontdata.generation = font_generation;
            }
        } else {
            fonts.insert(
                font.clone(),
                SurfaceTextEngineFontData {
                    generation: font_generation,
                    glyphs: HashMap::new(),
                },
            );
        }
        let fontdata = fonts.get_mut(&font).expect("font data");

        let data = create_text_data(fontdata, ops)?;
        Ok(Box::new(data))
    }

    /// `DestroyText`
    fn destroy_text(&self, engine_text: Box<dyn Any>) {
        drop(engine_text);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `UpdateColor`
fn update_color(data: &mut SurfaceTextEngineTextData, color: &FColor) {
    let conv = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    data.color = Color::new(conv(color.r), conv(color.g), conv(color.b), conv(color.a));
    data.fcolor = *color;
}

/// `DrawFill`
fn draw_fill(
    data: &SurfaceTextEngineTextData,
    op: &FillOperation,
    x: i32,
    y: i32,
    surface: &mut Surface<'_>,
) {
    let color = surface.map_rgba(data.color.r, data.color.g, data.color.b, data.color.a);
    let mut dst = op.rect;
    dst.x += x;
    dst.y += y;
    let _ = surface.fill_rect(Some(&dst), color);
}

/// `DrawCopy`
fn draw_copy(
    data: &SurfaceTextEngineTextData,
    op: &CopyOperation,
    glyph: &GlyphData,
    x: i32,
    y: i32,
    surface: &mut Surface<'_>,
) {
    let mut glyph = glyph.borrow_mut();
    if data.color.r != glyph.color.r
        || data.color.g != glyph.color.g
        || data.color.b != glyph.color.b
        || data.color.a != glyph.color.a
    {
        if glyph.image_type == ImageType::Alpha {
            glyph
                .surface
                .set_color_mod(data.color.r, data.color.g, data.color.b);
        } else {
            // Don't alter the color data in the image
        }
        glyph.surface.set_alpha_mod(data.color.a);
        glyph.color = data.color;
    }

    let dst = Rect::new(op.dst.x + x, op.dst.y + y, op.dst.w, op.dst.h);
    let _ = glyph.surface.blit(Some(&op.src), surface, Some(&dst));
}

/// Draw text to an SDL surface (the text must use a
/// [`SurfaceTextEngine`]). Translation of `TTF_DrawSurfaceText()`.
pub fn draw_surface_text(text: &Text, x: i32, y: i32, surface: &mut Surface<'_>) -> Result<()> {
    let is_surface_engine = text
        .engine()
        .is_some_and(|e| e.as_any().downcast_ref::<SurfaceTextEngine>().is_some());
    if !is_surface_engine {
        return Err(Error::invalid_param("text"));
    }

    // Make sure the text is up to date
    text.update()?;

    let mut td = text.rc.borrow_mut();
    let text_color = td.color;
    let Some(data) = td
        .engine_text
        .as_mut()
        .and_then(|d| d.downcast_mut::<SurfaceTextEngineTextData>())
    else {
        // Empty string, nothing to do
        return Ok(());
    };

    if text_color.r != data.fcolor.r
        || text_color.g != data.fcolor.g
        || text_color.b != data.fcolor.b
        || text_color.a != data.fcolor.a
    {
        update_color(data, &text_color);
    }

    for (op, glyph) in &data.ops {
        match op {
            DrawOperation::Fill(fill) => draw_fill(data, fill, x, y, surface),
            DrawOperation::Copy(copy) => {
                if let Some(glyph) = glyph {
                    draw_copy(data, copy, glyph, x, y, surface);
                }
            }
            _ => {}
        }
    }
    Ok(())
}
