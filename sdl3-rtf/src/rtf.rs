// Rust translation of include/SDL3_rtf/SDL_rtf.h and src/SDL_rtf.c from SDL_rtf.
// Copyright (C) 2003-2024 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* $Id$ */

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use sdl3::io::IoStream;
use sdl3::render::{Renderer, Texture};
use sdl3::video::{Color, Rect};
use sdl3::{Error, Result};

use crate::rtfdecl::*;
use crate::rtftype::*;

/* SDL_rtf.h */

/// The major version of SDL_rtf this crate translates.
/// Translation of `SDL_RTF_MAJOR_VERSION`.
pub const MAJOR_VERSION: u16 = 3;
/// The minor version. Translation of `SDL_RTF_MINOR_VERSION`.
pub const MINOR_VERSION: u16 = 0;
/// The micro (patch) version. Translation of `SDL_RTF_MICRO_VERSION`.
pub const MICRO_VERSION: u16 = 0;

/// This is the version number macro for the current SDL_rtf version.
/// Translation of `SDL_RTF_VERSION` (and, with
/// [`Version::at_least`](sdl3::Version::at_least), of
/// `SDL_RTF_VERSION_ATLEAST()`).
pub const VERSION: sdl3::Version = sdl3::Version::new(MAJOR_VERSION, MINOR_VERSION, MICRO_VERSION);

/// The upstream SDL_rtf revision this translation was made from (its
/// `main` branch, which has no release tag for SDL3).
pub const REVISION: &str = "SDL_rtf-main-0bdba67b48e67f2c7d80a08720dcce8b54b01018";

/// This function gets the version of the dynamically linked SDL_rtf
/// library. Translation of `RTF_Version()`.
pub fn version() -> sdl3::Version {
    VERSION
}

/// A font family (`RTF_FontFamily`): one of the associated constants, or
/// whatever value a document's font table gave it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct FontFamily(pub i32);

impl FontFamily {
    /// Unknown or default font. `RTF_FontDefault`
    pub const DEFAULT: FontFamily = FontFamily(0);
    /// Proportionally spaced serif fonts, e.g. Times New Roman, Palatino.
    /// `RTF_FontRoman`
    pub const ROMAN: FontFamily = FontFamily(1);
    /// Proportionally spaced sans serif fonts, e.g. Arial. `RTF_FontSwiss`
    pub const SWISS: FontFamily = FontFamily(2);
    /// Fixed pitch serif and sans serif fonts, e.g. Courier New, Pica.
    /// `RTF_FontModern`
    pub const MODERN: FontFamily = FontFamily(3);
    /// Script fonts, e.g. Cursive. `RTF_FontScript`
    pub const SCRIPT: FontFamily = FontFamily(4);
    /// Decorative fonts, e.g. Zapf Chancery. `RTF_FontDecor`
    pub const DECOR: FontFamily = FontFamily(5);
    /// Technical, symbol, and math fonts, e.g. Symbol. `RTF_FontTech`
    pub const TECH: FontFamily = FontFamily(6);
    /// Bidirectional fonts, like Arabic or Hebrew. `RTF_FontBidi`
    pub const BIDI: FontFamily = FontFamily(7);
}

/// A font style, a bitmask of the `FONT_*` constants (`RTF_FontStyle`).
pub type FontStyle = i32;
/// `RTF_FontNormal`
pub const FONT_NORMAL: FontStyle = 0x00;
/// `RTF_FontBold`
pub const FONT_BOLD: FontStyle = 0x01;
/// `RTF_FontItalic`
pub const FONT_ITALIC: FontStyle = 0x02;
/// `RTF_FontUnderline`
pub const FONT_UNDERLINE: FontStyle = 0x04;

/* Various functions that need to be provided to give SDL_rtf font support */

/// The version of the font engine interface. Translation of
/// `RTF_FONT_ENGINE_VERSION`.
pub const FONT_ENGINE_VERSION: i32 = 1;

/// The functions that need to be provided to give SDL_rtf font support
/// (`RTF_FontEngine`). [`TtfFontEngine`](crate::TtfFontEngine) is one,
/// over SDL_ttf.
pub trait FontEngine {
    /// A font the engine creates (the `void *` of the C callbacks).
    type Font;

    /// This should be [`FONT_ENGINE_VERSION`] (`version`).
    fn version(&self) -> i32 {
        FONT_ENGINE_VERSION
    }

    /// A function to create a font matching the requested parameters.
    /// The family is one of those listed in [`FontFamily`].
    /// The charset is a Windows character set.
    /// The size is in points.
    /// The style is a bitmask of the `FONT_*` [`FontStyle`] constants.
    /// (`CreateFont`)
    fn create_font(
        &mut self,
        name: &str,
        family: FontFamily,
        charset: i32,
        size: i32,
        style: FontStyle,
    ) -> Option<Self::Font>;

    /// Return the spacing in pixels between rows of text using this font
    /// (`GetLineSpacing`).
    fn line_spacing(&mut self, font: &Self::Font) -> i32;

    /// Fill in and return the byte and pixel offsets to each character
    /// within the given UTF-8 text (`GetCharacterOffsets`; `maxOffsets` is
    /// the length of the slices, one more than the text's length).
    fn character_offsets(
        &mut self,
        font: &Self::Font,
        text: &str,
        byte_offsets: &mut [i32],
        pixel_offsets: &mut [i32],
    ) -> i32;

    /// Create a texture containing a row of the given UTF-8 text
    /// (`RenderText`).
    fn render_text(
        &mut self,
        font: &Self::Font,
        renderer: &mut Renderer,
        text: &str,
        fg: Color,
    ) -> Option<Texture>;

    /// Free a font (`FreeFont`).
    fn free_font(&mut self, font: Self::Font) {
        drop(font);
    }
}

/* SDL_rtf.c */

impl<E: FontEngine> Context<E> {
    /// Create an RTF display context, with the given font engine.
    /// Once a context is created, it can be used to load and display
    /// text in Microsoft RTF format.
    ///
    /// `renderer` is the SDL renderer to use for drawing, `font_engine`
    /// the font engine to use for rendering text. Translation of
    /// `RTF_CreateContext()`.
    pub fn new(renderer: Rc<RefCell<Renderer>>, font_engine: E) -> Result<Context<E>> {
        if font_engine.version() != FONT_ENGINE_VERSION {
            return Err(Error::new("Unknown font engine version"));
        }

        Ok(Context {
            renderer,
            font_engine,
            fonts: Vec::new(),
            data: Vec::new(),
            datapos: 0,
            values: [0; 4],
            font_table: Vec::new(),
            color_table: Vec::new(),
            title: None,
            subject: None,
            author: None,
            c_group: 0,
            rds: Rds::Norm,
            ris: Ris::Norm,
            chp: Chp::default(),
            pap: Pap::default(),
            sep: Sep::default(),
            dop: Dop::default(),
            psave: Vec::new(),
            cb_bin: 0,
            l_param: 0,
            f_skip_dest_if_unk: false,
            nextch: 0,
            display_width: 0,
            display_height: 0,
            lines: Vec::new(),
        })
    }

    /// Set the text of an RTF context, with data loaded from an
    /// [`IoStream`]. This can be called multiple times to change the text
    /// displayed. (Closing the stream, `closeio`, is dropping it.)
    ///
    /// Translation of `RTF_Load_IO()`.
    pub fn load_io(&mut self, src: &mut IoStream<'_>) -> Result<()> {
        self.ec_clear_context();

        /* Set up the input stream for loading */
        self.rds = Rds::Norm;
        self.ris = Ris::Norm;
        self.cb_bin = 0;
        self.f_skip_dest_if_unk = false;
        self.nextch = -1;

        /* Parse the RTF text and clean up */
        let retval = match self.ec_rtf_parse(src) {
            EC_OK => Ok(()),
            EC_STACK_UNDERFLOW => Err(Error::new("Unmatched '}'")),
            EC_STACK_OVERFLOW => Err(Error::new("Too many '{' -- memory exhausted")),
            EC_UNMATCHED_BRACE => Err(Error::new("RTF ended during an open group")),
            EC_INVALID_HEX => Err(Error::new("Invalid hex character found in data")),
            EC_BAD_TABLE => Err(Error::new("RTF table (sym or prop) invalid")),
            EC_ASSERTION => Err(Error::new("Assertion failure")),
            EC_END_OF_FILE => Err(Error::new("End of file reached while reading RTF")),
            EC_FONT_NOT_FOUND => Err(Error::new("Couldn't find font for text")),
            _ => Err(Error::new("Unknown error")),
        };
        while !self.psave.is_empty() {
            self.ec_pop_rtf_state();
        }

        retval
    }

    /// Set the text of an RTF context, with data loaded from a filename.
    /// This can be called multiple times to change the text displayed.
    /// Translation of `RTF_Load()`.
    pub fn load(&mut self, file: impl AsRef<Path>) -> Result<()> {
        let mut src = IoStream::from_file(file, "rb")?;
        self.load_io(&mut src)
    }

    /// Get the title of an RTF document. Translation of `RTF_GetTitle()`.
    pub fn title(&self) -> &str {
        self.title.as_deref().unwrap_or("")
    }

    /// Get the subject of an RTF document. Translation of
    /// `RTF_GetSubject()`.
    pub fn subject(&self) -> &str {
        self.subject.as_deref().unwrap_or("")
    }

    /// Get the author of an RTF document. Translation of
    /// `RTF_GetAuthor()`.
    pub fn author(&self) -> &str {
        self.author.as_deref().unwrap_or("")
    }

    /// Get the height of an RTF render area given a certain width.
    /// The text is automatically reflowed to this new width, and should
    /// match the width of the clipping rectangle used for rendering later.
    /// Translation of `RTF_GetHeight()`.
    pub fn height(&mut self, width: i32) -> i32 {
        self.ec_reflow_text(width);
        self.display_height
    }

    /// Render the RTF document to a rectangle of the renderer (`None`: its
    /// whole viewport). The text is reflowed to match the width of the
    /// rectangle. The rendering is offset up (and clipped) by `y_offset`
    /// pixels. Translation of `RTF_Render()`.
    pub fn render(&mut self, rect: Option<&Rect>, y_offset: i32) {
        let rect = match rect {
            Some(rect) => *rect,
            None => {
                let mut full_rect = self.renderer.borrow().viewport();
                full_rect.x = 0;
                full_rect.y = 0;
                full_rect
            }
        };
        self.ec_render_text(&rect, y_offset.wrapping_neg());
    }

    /// The renderer the context draws with.
    pub fn renderer(&self) -> &Rc<RefCell<Renderer>> {
        &self.renderer
    }

    /// The context's font engine.
    pub fn font_engine(&self) -> &E {
        &self.font_engine
    }

    /// The context's font engine, to change. Fonts it already created stay
    /// in use until the next [`load`](Context::load).
    pub fn font_engine_mut(&mut self) -> &mut E {
        &mut self.font_engine
    }
}

/// Free an RTF display context: its document, its fonts (through the font
/// engine) and its textures. This does not destroy the associated
/// renderer, which can continue to draw and present. Translation of
/// `RTF_FreeContext()`.
impl<E: FontEngine> Drop for Context<E> {
    fn drop(&mut self) {
        /* Free it all! */
        self.ec_clear_context();
    }
}
