// Rust translation of the font engine of examples/showrtf.c from SDL_rtf.
// Copyright (C) 2003-2024 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* A simple program to test the RTF rendering of the SDL_rtf library */

// (The program itself is examples/showrtf.rs.)

use sdl3::io::IoStream;
use sdl3::render::{Renderer, Texture};
use sdl3::video::Color;
use sdl3_ttf::{Font, STYLE_BOLD, STYLE_ITALIC, STYLE_NORMAL, STYLE_UNDERLINE};

use crate::rtf::{FontEngine, FontFamily, FontStyle, FONT_BOLD, FONT_ITALIC, FONT_UNDERLINE};

/// Where a [`TtfFontEngine`] opens a font from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontSource {
    /// A font file (`TTF_OpenFont()`)
    File(String),
    /// A font in memory (`TTF_OpenFontIO()` over `SDL_IOFromConstMem()`)
    Memory(&'static [u8]),
}

/// The font engine of upstream's showrtf example: each font family is
/// rendered with a font file (or a font in memory) opened with SDL_ttf at
/// the requested size, with its bold, italic and underline styles, the
/// default font standing in for the families without one. The character
/// set is ignored.
///
/// [`sdl3_ttf::init`] must have been called before fonts are created.
#[derive(Clone, Debug, Default)]
pub struct TtfFontEngine {
    /// The font of each family, indexed as by
    /// [`font_family_to_index`] (`FontList`)
    pub font_list: [Option<FontSource>; 8],
}

impl TtfFontEngine {
    /// An engine with `default` as the font of every family.
    pub fn new(default: FontSource) -> TtfFontEngine {
        let mut engine = TtfFontEngine::default();
        engine.font_list[0] = Some(default);
        engine
    }

    /// Set the font of a family.
    pub fn set_font(&mut self, family: FontFamily, source: FontSource) {
        self.font_list[font_family_to_index(family)] = Some(source);
    }
}

/* Note, this is only one way of looking up fonts */
/// The index of a family's font in [`TtfFontEngine::font_list`]
/// (`FontFamilyToIndex()`).
pub fn font_family_to_index(family: FontFamily) -> usize {
    match family {
        FontFamily::DEFAULT => 0,
        FontFamily::ROMAN => 1,
        FontFamily::SWISS => 2,
        FontFamily::MODERN => 3,
        FontFamily::SCRIPT => 4,
        FontFamily::DECOR => 5,
        FontFamily::TECH => 6,
        FontFamily::BIDI => 7,
        _ => 0,
    }
}

/// `UTF8_to_UNICODE()`, with its masks (the result goes through a
/// `Uint16`, which keeps the code points of up to three bytes right)
fn utf8_to_unicode(utf8: &[u8], advance: &mut usize) -> u32 {
    let mut i = 0;
    /* utf8[i] is a (signed) char */
    let at = |i: usize| utf8.get(i).map_or(0, |&c| c as i8 as i32);

    let mut ch = utf8.first().map_or(0, |&c| c as u32);
    if ch >= 0xF0 {
        ch = ((at(i) & 0x07) << 18) as u16 as u32;
        i += 1;
        ch |= ((at(i) & 0x3F) as u16 as u32) << 12;
        i += 1;
        ch |= ((at(i) & 0x3F) as u16 as u32) << 6;
        i += 1;
        ch |= (at(i) & 0x3F) as u16 as u32;
    } else if ch >= 0xE0 {
        ch = ((at(i) & 0x3F) as u16 as u32) << 12;
        i += 1;
        ch |= ((at(i) & 0x3F) as u16 as u32) << 6;
        i += 1;
        ch |= (at(i) & 0x3F) as u16 as u32;
    } else if ch >= 0xC0 {
        ch = ((at(i) & 0x3F) as u16 as u32) << 6;
        i += 1;
        ch |= (at(i) & 0x3F) as u16 as u32;
    }
    *advance = i + 1;
    ch
}

impl FontEngine for TtfFontEngine {
    type Font = Font;

    /// `CreateFont()`
    fn create_font(
        &mut self,
        _name: &str,
        family: FontFamily,
        _charset: i32,
        size: i32,
        style: FontStyle,
    ) -> Option<Font> {
        let mut index = font_family_to_index(family);
        if self.font_list[index].is_none() {
            index = 0;
        }

        let font = match self.font_list[index].as_ref()? {
            FontSource::File(file) => Font::open(file, size as f32),
            FontSource::Memory(mem) => Font::open_io(IoStream::from_const_mem(mem), size as f32),
        };
        let font = font.ok()?;
        let mut ttf_style = STYLE_NORMAL;
        if style & FONT_BOLD != 0 {
            ttf_style |= STYLE_BOLD;
        }
        if style & FONT_ITALIC != 0 {
            ttf_style |= STYLE_ITALIC;
        }
        if style & FONT_UNDERLINE != 0 {
            ttf_style |= STYLE_UNDERLINE;
        }
        font.set_style(ttf_style);

        /* FIXME: What do we do with the character set? */

        Some(font)
    }

    /// `GetLineSpacing()`
    fn line_spacing(&mut self, font: &Font) -> i32 {
        font.line_skip()
    }

    /// `GetCharacterOffsets()`
    fn character_offsets(
        &mut self,
        font: &Font,
        text: &str,
        byte_offsets: &mut [i32],
        pixel_offsets: &mut [i32],
    ) -> i32 {
        let max_offsets = byte_offsets.len().min(pixel_offsets.len());
        let text = text.as_bytes();
        let mut pos = 0;
        let mut i = 0;
        let mut bytes: i32 = 0;
        let mut pixels: i32 = 0;
        let mut advance = 0;
        while text.get(pos).is_some_and(|&c| c != 0) && i < max_offsets {
            byte_offsets[i] = bytes;
            pixel_offsets[i] = pixels;
            i += 1;

            let ch = utf8_to_unicode(&text[pos..], &mut advance) as u16;
            pos += advance;
            bytes = bytes.wrapping_add(advance as i32);
            let mut advance = advance as i32;
            if let Ok((_, _, _, _, a)) = font.glyph_metrics(ch as u32) {
                advance = a;
            }
            pixels = pixels.wrapping_add(advance);
        }
        if i < max_offsets {
            byte_offsets[i] = bytes;
            pixel_offsets[i] = pixels;
        }
        i as i32
    }

    /// `RenderText()`
    fn render_text(
        &mut self,
        font: &Font,
        renderer: &mut Renderer,
        text: &str,
        fg: Color,
    ) -> Option<Texture> {
        let mut surface = font.render_text_blended(text, fg).ok()?;
        renderer.create_texture_from_surface(&mut surface).ok()
    }

    /// `FreeFont()`
    fn free_font(&mut self, font: Font) {
        drop(font);
    }
}
