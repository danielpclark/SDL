// Rust translation of src/SDL_rtfreadr.c and src/SDL_rtfreadr.h from SDL_rtf.
// Copyright (C) 2003-2024 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use std::cell::RefCell;

use sdl3::io::IoStream;
use sdl3::render::{Renderer, Texture};
use sdl3::video::{Color, FRect, Rect};

use crate::rtf::{FontEngine, FontFamily};
use crate::rtfdecl::*;
use crate::rtftype::*;

/*
 * %%Function: RTF_CreateColor
 */
pub(crate) fn rtf_create_color(r: i32, g: i32, b: i32) -> Color {
    Color {
        r: r as u8,
        g: g as u8,
        b: b as u8,
        a: sdl3::video::ALPHA_OPAQUE,
    }
}

/* RTF_FreeColor(): the colors are values */

/*
 * %%Function: RTF_GetChar
 */
pub(crate) fn rtf_get_char(stream: &mut IoStream<'_>, c: &mut u8) -> i32 {
    let mut buf = [0u8; 1];
    let n = stream.read(&mut buf);
    *c = buf[0];
    n as i32
}

impl<E: FontEngine> Context<E> {
    /*
     * %%Function: RTF_CreateFont
     */
    pub(crate) fn rtf_create_font(
        &mut self,
        name: &str,
        family: FontFamily,
        charset: i32,
        size: i32,
        style: i32,
    ) -> Option<E::Font> {
        self.font_engine
            .create_font(name, family, charset, size / 2, style)
    }

    /*
     * %%Function: RTF_FreeFont
     */
    pub(crate) fn rtf_free_font(&mut self, font: E::Font) {
        self.font_engine.free_font(font);
    }

    /*
     * %%Function: RTF_FreeSurface
     */
    pub(crate) fn rtf_free_surface(&mut self, surface: Texture) {
        free_surface(&self.renderer, surface);
    }

    /*
     * %%Function: RTF_GetLineSpacing
     */
    pub(crate) fn rtf_get_line_spacing(&mut self, font: usize) -> i32 {
        match self.fonts.get(font) {
            Some(Some(font)) => self.font_engine.line_spacing(font),
            _ => 0,
        }
    }

    /*
     * &&Function: RTF_GetCharacterOffsets
     */
    pub(crate) fn rtf_get_character_offsets(
        &mut self,
        font: usize,
        text: &str,
        byte_offsets: &mut [i32],
        pixel_offsets: &mut [i32],
    ) -> i32 {
        match self.fonts.get(font) {
            Some(Some(font)) => {
                self.font_engine
                    .character_offsets(font, text, byte_offsets, pixel_offsets)
            }
            _ => 0,
        }
    }

    /*
     * %%Function: ecReflowText
     *
     * Reflow the text to a new width
     */
    pub(crate) fn ec_reflow_text(&mut self, width: i32) -> i32 {
        if self.display_width == width {
            return EC_OK;
        }

        /* Reflow the text to the new width */
        self.display_width = width;
        self.display_height = 0;
        let Context {
            renderer,
            font_engine,
            fonts,
            lines,
            display_height,
            ..
        } = self;
        for line in lines.iter_mut() {
            *display_height =
                display_height.wrapping_add(reflow_line(renderer, font_engine, fonts, line, width));
        }
        EC_OK
    }

    /*
     * %%Function: ecReflowText
     *
     * Render the text to a surface
     */
    pub(crate) fn ec_render_text(&mut self, rect: &Rect, mut y_offset: i32) -> i32 {
        self.ec_reflow_text(rect.w);

        let mut renderer = self.renderer.borrow_mut();
        let saved_rect = renderer.clip_rect();
        let _ = renderer.set_clip_rect(Some(rect));
        for line in &self.lines {
            if y_offset >= rect.h {
                break;
            }
            if y_offset.wrapping_add(line.line_height) > 0 {
                render_line(&mut renderer, line, rect, y_offset);
            }
            y_offset = y_offset.wrapping_add(line.line_height);
        }
        // (with no clip rectangle set, as upstream, this sets an empty one)
        let _ = renderer.set_clip_rect(Some(&saved_rect));

        EC_OK
    }
}

/// `SDL_DestroyTexture()` on a texture of the context's renderer (left
/// to the renderer, which destroys its textures, if it is in use)
fn free_surface(renderer: &RefCell<Renderer>, surface: Texture) {
    if let Ok(mut renderer) = renderer.try_borrow_mut() {
        renderer.destroy_texture(surface);
    }
}

fn twips_to_pixels(twips: i32) -> i32 {
    /* twips are 1/20 of a pointsize, calculate pixels at 72 dpi */
    (((twips.wrapping_mul(64 * 72).wrapping_add(36 + 32 * 72)) / 72) / 20) / 64
}

fn create_surface<E: FontEngine>(
    renderer: &RefCell<Renderer>,
    font_engine: &mut E,
    fonts: &[Option<E::Font>],
    text_block: &TextBlock,
    offset: i32,
    num_chars: i32,
) -> Option<RtfSurface> {
    let start = *text_block.byte_offsets.get(offset as usize)?;
    let end = *text_block.byte_offsets.get((offset + num_chars) as usize)?;
    let text = text_block.text.get(start as usize..end as usize)?;
    let color = text_block.color.unwrap_or(Color {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    });
    let font = fonts.get(text_block.font)?.as_ref()?;
    let surface = font_engine.render_text(font, &mut renderer.borrow_mut(), text, color)?;
    Some(RtfSurface {
        x: 0,
        y: 0,
        surface,
    })
}

/// `SDL_isspace()`
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | 0x0c | 0x0b)
}

fn text_within_width(text_block: &TextBlock, offset: i32, width: i32, wrapped: &mut i32) -> i32 {
    let mut num = 0;
    let pixel_offset = |i: i32| {
        text_block
            .pixel_offsets
            .get(i as usize)
            .copied()
            .unwrap_or(0)
    };

    /* Fit as many characters as possible into the available width */
    *wrapped = 0;
    if offset + num < text_block.num_chars {
        while pixel_offset(offset + num + 1).wrapping_sub(pixel_offset(offset)) <= width {
            num += 1;
            if offset + num == text_block.num_chars {
                return num;
            }
        }

        /* Do word wrapping */
        let mut wrap_index = offset + num - 1;
        while wrap_index > offset {
            let byte = text_block
                .byte_offsets
                .get(wrap_index as usize)
                .and_then(|&b| text_block.text.as_bytes().get(b as usize));
            if byte.is_some_and(|&c| is_space(c)) {
                break;
            }
            wrap_index -= 1;
        }
        if wrap_index > offset {
            num = wrap_index - offset + 1;
            *wrapped = 1;
        }
    }
    num
}

/// The surfaces from `row_start` on, moved right by `offset`
fn shift_row(surfaces: &mut [RtfSurface], row_start: Option<usize>, offset: i32) {
    if let Some(row_start) = row_start {
        for surface in &mut surfaces[row_start..] {
            surface.x = surface.x.wrapping_add(offset);
        }
    }
}

fn reflow_line<E: FontEngine>(
    renderer: &RefCell<Renderer>,
    font_engine: &mut E,
    fonts: &[Option<E::Font>],
    line: &mut Line,
    mut width: i32,
) -> i32 {
    for surface in line.surfaces.drain(..) {
        free_surface(renderer, surface.surface);
    }
    if !line.blocks.is_empty() {
        let left_margin = twips_to_pixels(line.pap.xa_left);
        let right_margin = twips_to_pixels(line.pap.xa_right);
        let tab_stop = twips_to_pixels(720);
        let mut row_start: Option<usize> = None;
        let mut line_height = 0;
        let mut line_width = twips_to_pixels(line.pap.xa_first);

        width = width.wrapping_sub(left_margin);
        width = width.wrapping_sub(right_margin);
        line.line_width = 0;
        line.line_height = 0;
        for text_block in &line.blocks {
            let mut num;
            let mut wrapped = 0;
            let mut num_chars = 0;

            for _ in 0..text_block.tabs {
                let next_tab =
                    ((left_margin.wrapping_add(line_width) / tab_stop) + 1).wrapping_mul(tab_stop);
                line_width = next_tab.wrapping_sub(left_margin);
            }
            loop {
                num = text_within_width(
                    text_block,
                    num_chars,
                    width.wrapping_sub(line_width),
                    &mut wrapped,
                );
                if num > 0 {
                    let surface =
                        create_surface(renderer, font_engine, fonts, text_block, num_chars, num);
                    if let Some(mut surface) = surface {
                        if row_start.is_none() {
                            row_start = Some(line.surfaces.len());
                        }
                        surface.x = left_margin.wrapping_add(line_width);
                        surface.y = line.line_height;
                        line.surfaces.push(surface);
                    }
                    if line_height < text_block.line_height {
                        line_height = text_block.line_height;
                    }
                    let pixel_offset = |i: i32| {
                        text_block
                            .pixel_offsets
                            .get(i as usize)
                            .copied()
                            .unwrap_or(0)
                    };
                    line_width = line_width.wrapping_add(
                        pixel_offset(num_chars + num).wrapping_sub(pixel_offset(num_chars)),
                    );
                    num_chars += num;
                }
                if wrapped != 0 {
                    if line_width > line.line_width {
                        line.line_width = line_width;
                    }
                    line.line_height = line.line_height.wrapping_add(line_height);

                    if line.pap.just == JUST_C {
                        let offset = width.wrapping_sub(line_width) / 2;

                        shift_row(&mut line.surfaces, row_start, offset);
                    } else if line.pap.just == JUST_R {
                        let offset = left_margin.wrapping_add(width).wrapping_sub(line_width);

                        shift_row(&mut line.surfaces, row_start, offset);
                    }
                    row_start = None;

                    line_width = 0;
                    line_height = 0;
                }
                if num <= 0 {
                    break;
                }
            }
        }
        if line_width > line.line_width {
            line.line_width = line_width;
        }
        line.line_height = line.line_height.wrapping_add(line_height);

        if line.pap.just == JUST_C {
            let offset = width.wrapping_sub(line_width) / 2;

            shift_row(&mut line.surfaces, row_start, offset);
        } else if line.pap.just == JUST_R {
            let offset = left_margin.wrapping_add(width).wrapping_sub(line_width);

            shift_row(&mut line.surfaces, row_start, offset);
        }
    }
    line.line_height
}

fn render_line(renderer: &mut Renderer, line: &Line, rect: &Rect, y_offset: i32) {
    for surface in &line.surfaces {
        let texture = surface.surface;

        let Ok((w, h)) = renderer.texture_size(texture) else {
            continue;
        };
        let dst_rect = FRect {
            x: rect.x.wrapping_add(surface.x) as f32,
            y: rect.y.wrapping_add(y_offset).wrapping_add(surface.y) as f32,
            w,
            h,
        };
        let _ = renderer.render_texture(texture, None, Some(&dst_rect));
    }
}
