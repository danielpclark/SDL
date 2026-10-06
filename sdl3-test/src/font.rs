// Rust translation of src/test/SDL_test_font.c and include/SDL3/SDL_test_font.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Font related functions of SDL test framework.
//!
//! Text drawn with the renderer's built-in debug font
//! ([`Renderer::render_debug_text`]), and multi-line text output windows.

use std::fmt;

use sdl3::render::{Renderer, DEBUG_TEXT_FONT_CHARACTER_SIZE};
use sdl3::video::rect::FRect;
use sdl3::{Error, Result};

use crate::internal::truncate;

/// The width and height of a character, in pixels. Translation of
/// `FONT_CHARACTER_SIZE` (a variable upstream, which nothing changes).
pub const FONT_CHARACTER_SIZE: i32 = DEBUG_TEXT_FONT_CHARACTER_SIZE;

/// The distance between lines of text, in pixels. Translation of
/// `FONT_LINE_HEIGHT`.
pub const FONT_LINE_HEIGHT: i32 = FONT_CHARACTER_SIZE + 2;

fn utf8_is_trailing_byte(c: u8) -> bool {
    (0x80..=0xBF).contains(&c)
}

/// Draw a character in the currently set font, with its upper left corner
/// at (`x`, `y`). Translation of `SDLTest_DrawCharacter()`.
pub fn draw_character(renderer: &mut Renderer, x: f32, y: f32, c: u32) -> Result<()> {
    let mut str = [0u8; 4];
    let s = sdl3::stdlib::string::ucs4_to_utf8(c, &mut str);
    renderer.render_debug_text(x, y, s)
}

/// Draw a UTF-8 string in the currently set font, with its upper left
/// corner at (`x`, `y`).
///
/// The font currently only supports characters in the Basic Latin and
/// Latin-1 Supplement sets. Translation of `SDLTest_DrawString()`.
pub fn draw_string(renderer: &mut Renderer, x: f32, y: f32, s: &str) -> Result<()> {
    renderer.render_debug_text(x, y, s)
}

/// Data used for multi-line text output. Translation of
/// `SDLTest_TextWindow`; dropping it is `SDLTest_TextWindowDestroy()`.
#[derive(Clone, PartialEq, Debug)]
pub struct TextWindow {
    /// Where the window is drawn (only its position is used).
    pub rect: FRect,
    current: usize,
    lines: Vec<Option<String>>,
}

impl TextWindow {
    /// Create a multi-line text output window with its upper left corner at
    /// (`x`, `y`), as many lines high as fit in `h` (`w` is currently
    /// ignored).
    ///
    /// Returns an error if not even one line fits. Translation of
    /// `SDLTest_TextWindowCreate()`.
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Result<TextWindow> {
        let numlines = (h / FONT_LINE_HEIGHT as f32).ceil() as i32;
        if numlines < 1 {
            // Note (upstream): the C code makes a window without lines,
            // which the other functions then index out of bounds.
            return Err(Error::invalid_param("h"));
        }
        Ok(TextWindow {
            rect: FRect { x, y, w, h },
            current: 0,
            lines: vec![None; numlines as usize],
        })
    }

    /// The number of lines the window has. Translation of reading
    /// `SDLTest_TextWindow.numlines`.
    pub fn numlines(&self) -> usize {
        self.lines.len()
    }

    /// The line text is being added to. Translation of reading
    /// `SDLTest_TextWindow.current`.
    pub fn current(&self) -> usize {
        self.current
    }

    /// The text of the window's lines (`None` for lines without any).
    /// Translation of reading `SDLTest_TextWindow.lines`.
    pub fn lines(&self) -> &[Option<String>] {
        &self.lines
    }

    /// Display a multi-line text output window. This function should be
    /// called every frame to display the text. Translation of
    /// `SDLTest_TextWindowDisplay()`.
    pub fn display(&self, renderer: &mut Renderer) {
        let mut y = self.rect.y;
        for line in &self.lines {
            if let Some(line) = line {
                let _ = draw_string(renderer, self.rect.x, y, line);
            }
            y += FONT_LINE_HEIGHT as f32;
        }
    }

    /// Add text to a multi-line text output window, as
    /// [`add_text`](Self::add_text), formatted (`format_args!`) and cut at
    /// 1023 bytes. Translation of `SDLTest_TextWindowAddText()`.
    pub fn add_text_fmt(&mut self, args: fmt::Arguments<'_>) {
        let text = truncate(fmt::format(args), 1024);

        self.add_text(&text);
    }

    /// Add text to a multi-line text output window.
    ///
    /// Adds UTF-8 text to the end of the current text. A newline at the end
    /// starts a new line of text (scrolling the window up when it is on its
    /// last line). A backspace at the start deletes the last character or, if
    /// the line is empty, deletes the line and goes to the end of the
    /// previous line. Translation of `SDLTest_TextWindowAddTextWithLength()`.
    pub fn add_text(&mut self, text: &str) {
        let mut text = text;
        let mut newline = false;

        if let Some(stripped) = text.strip_suffix('\n') {
            text = stripped;
            newline = true;
        }

        let numlines = self.lines.len();
        let current = self.current;

        if text.as_bytes().first() == Some(&b'\x08') {
            match &mut self.lines[current] {
                Some(line) if !line.is_empty() => {
                    let bytes = line.as_bytes();
                    let mut existing = bytes.len();
                    while existing > 1 && utf8_is_trailing_byte(bytes[existing - 1]) {
                        existing -= 1;
                    }
                    existing -= 1;
                    line.truncate(existing);
                }
                _ => {
                    if self.current > 0 {
                        self.lines[current] = None;
                        self.current -= 1;
                    }
                }
            }
            return;
        }

        let line = self.lines[current].get_or_insert_with(String::new);
        line.push_str(text);
        if newline {
            if self.current == numlines - 1 {
                self.lines.remove(0);
                self.lines.push(None);
            } else {
                self.current += 1;
            }
        }
    }

    /// Clear the text in a multi-line text output window. Translation of
    /// `SDLTest_TextWindowClear()`.
    pub fn clear(&mut self) {
        self.lines.fill(None);
        self.current = 0;
    }
}

/// Cleanup textures used by font drawing functions: there are none (the
/// debug font's belong to each renderer). Translation of
/// `SDLTest_CleanupTextDrawing()`.
pub fn cleanup_text_drawing() {}

#[cfg(test)]
mod tests {
    use super::*;
    use sdl3::video::pixels::{Color, PixelFormat};
    use sdl3::video::surface::Surface;

    fn renderer(w: i32, h: i32) -> Renderer {
        let mut renderer =
            Renderer::software(Surface::new(w, h, PixelFormat::ARGB8888).unwrap()).unwrap();
        renderer.set_draw_color(0, 0, 0, 255);
        renderer.clear().unwrap();
        renderer.set_draw_color(255, 255, 255, 255);
        renderer
    }

    /// The pixels as rows of `#` (white) and `.` (black).
    fn picture(renderer: &mut Renderer) -> Vec<String> {
        let surface = renderer.read_pixels(None).unwrap();
        (0..surface.height())
            .map(|y| {
                (0..surface.width())
                    .map(|x| {
                        let c = surface.read_pixel(x, y).unwrap();
                        if c == Color::new(255, 255, 255, 255) {
                            '#'
                        } else {
                            assert_eq!(c, Color::new(0, 0, 0, 255));
                            '.'
                        }
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn glyphs() {
        // The debug font's 'A' and Latin-1 '©', drawn by code point, as in
        // upstream's SDL_render_debug_font.h; and 'é', which upstream's
        // DrawDebugCharacter() draws as the missing character, as it checks
        // for code points past its 190 glyphs before skipping the gap in the
        // middle (so U+00BE to U+00FF are all missing).
        let mut r = renderer(24, 8);
        draw_character(&mut r, 0.0, 0.0, 'A' as u32).unwrap();
        draw_character(&mut r, 8.0, 0.0, 0xA9).unwrap();
        draw_character(&mut r, 16.0, 0.0, 0xE9).unwrap();
        assert_eq!(
            picture(&mut r),
            [
                "..##......####..#.#.#.#.",
                ".####....#....#..#.#.#.#",
                "##..##..#..##..##.#.#.#.",
                "##..##..#.#....#.#.#.#.#",
                "######..#.#....##.#.#.#.",
                "##..##..#..##..#.#.#.#.#",
                "##..##...#....#.#.#.#.#.",
                "..........####...#.#.#.#",
            ]
        );

        // A string is the same as its characters one after another.
        let mut a = renderer(40, 10);
        draw_string(&mut a, 1.0, 2.0, "SDL ©!").unwrap();
        let mut b = renderer(40, 10);
        let mut x = 1.0;
        for c in "SDL ©!".chars() {
            draw_character(&mut b, x, 2.0, c as u32).unwrap();
            x += FONT_CHARACTER_SIZE as f32;
        }
        assert_eq!(picture(&mut a), picture(&mut b));

        // And the same as SDL_RenderDebugText().
        let mut c = renderer(40, 10);
        c.render_debug_text(1.0, 2.0, "SDL ©!").unwrap();
        assert_eq!(picture(&mut a), picture(&mut c));
        cleanup_text_drawing();
    }

    #[test]
    fn text_window() {
        let mut window = TextWindow::new(2.0, 3.0, 100.0, 25.0).unwrap();
        // ceil(25 / 10)
        assert_eq!(window.numlines(), 3);
        window.add_text("hello");
        window.add_text(" world\n");
        assert_eq!(window.current(), 1);
        window.add_text_fmt(format_args!("{}\n", 42));
        window.add_text("é");
        assert_eq!(
            window.lines(),
            [
                Some("hello world".to_owned()),
                Some("42".to_owned()),
                Some("é".to_owned())
            ]
        );
        // Backspace: the last character (both bytes of 'é'), then the line.
        window.add_text("\x08");
        assert_eq!(window.lines()[2].as_deref(), Some(""));
        window.add_text("\x08");
        assert_eq!(window.lines()[2], None);
        assert_eq!(window.current(), 1);
        window.add_text("\x08");
        assert_eq!(window.lines()[1].as_deref(), Some("4"));
        // A newline on the last line scrolls.
        window.add_text("\n");
        window.add_text("three\n");
        assert_eq!(window.current(), 2);
        assert_eq!(
            window.lines(),
            [Some("4".to_owned()), Some("three".to_owned()), None]
        );
        // A backspace on the first line's start does nothing.
        window.clear();
        window.add_text("\x08");
        assert_eq!(window.current(), 0);
        assert_eq!(window.lines(), [None, None, None]);
        // Long formatted text is cut at 1023 bytes.
        window.add_text_fmt(format_args!("{}", "x".repeat(2000)));
        assert_eq!(window.lines()[0].as_ref().unwrap().len(), 1023);

        assert!(TextWindow::new(0.0, 0.0, 10.0, 0.0).is_err());
    }

    #[test]
    fn text_window_display() {
        let mut window = TextWindow::new(1.0, 1.0, 30.0, 20.0).unwrap();
        window.add_text("AB\n");
        window.add_text("C");
        let mut a = renderer(24, 22);
        window.display(&mut a);
        let mut b = renderer(24, 22);
        draw_string(&mut b, 1.0, 1.0, "AB").unwrap();
        draw_string(&mut b, 1.0, 1.0 + FONT_LINE_HEIGHT as f32, "C").unwrap();
        assert_eq!(picture(&mut a), picture(&mut b));
        assert!(picture(&mut a).iter().any(|row| row.contains('#')));
    }
}
