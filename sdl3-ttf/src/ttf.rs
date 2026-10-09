// Rust translation of src/SDL_ttf.c from SDL_ttf.
// Copyright (C) 2001-2025 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! SDL_ttf: A companion library to SDL for working with TrueType (tm)
//! fonts.
//!
//! The build translated is upstream's default one, with its bundled
//! FreeType and HarfBuzz and without PlutoSVG (`TTF_USE_HARFBUZZ` is 1,
//! `TTF_USE_PLUTOSVG` is 0): text is shaped by the translated HarfBuzz
//! (`crate::harfbuzz`), with the font's or text's direction, script and
//! language.
//!
//! Translation notes:
//!
//! * Glyph images and text surfaces keep the layout of an x86-64 build,
//!   whose SSE2 blitters set the alignment to 16 (`Get_Alignment`): glyph
//!   images are padded by it, and surface pitches are rounded to it. The
//!   blitters are the plain byte/pixel loops (`BG`, `BG_Blended`,
//!   `BG_Blended_Opaque`); the SSE2, NEON and word-sized variants of C
//!   compute the same pixels, and only differ by also OR-ing the zero
//!   padding of the glyph images into the surface.
//! * The glyph caches and the text references (`SDL_HashTable`s) are
//!   `HashMap`s; nothing depends on their iteration order.
//! * Fonts are shared (`TTF_Font *`) through reference counting: a
//!   [`Font`] owns its data, and fallback lists, glyph positions and text
//!   objects refer to it weakly. Dropping a [`Font`] is `TTF_CloseFont`.
//! * The font source stream is owned by the font (shared with its copies),
//!   instead of the `closeio` flag and the `TTF_PROP_IOSTREAM_REFCOUNT`
//!   property.

#![allow(clippy::needless_late_init, clippy::too_many_arguments)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;
use sdl3::video::{BlendMode, Color, PixelFormat, Surface};
use sdl3::{Error, Result};

use crate::freetype::base::ftcalc::ft_mul_fix;
use crate::freetype::base::ftglyph::*;
use crate::freetype::base::ftinit::{ft_done_freetype, ft_init_freetype};
use crate::freetype::base::ftobjs::*;
use crate::freetype::base::ftoutln::{ft_outline_transform, ft_outline_translate};
use crate::freetype::base::ftstream::FtStreamRec;
use crate::freetype::base::ftstroke::*;
use crate::freetype::fttypes::*;
use crate::freetype::tttables::{FtSfntTable, FT_SFNT_OS2};
use crate::harfbuzz::hb_buffer::HbBuffer;
use crate::harfbuzz::hb_common::*;
use crate::harfbuzz::hb_face::HbFace;
use crate::harfbuzz::hb_font::{HbFont, HbFontData};
use crate::harfbuzz::hb_ft::*;
use crate::harfbuzz::hb_shape::hb_shape;
use crate::harfbuzz::hb_unicode::hb_unicode_script;

/* Enable rendering with color
 * Freetype may need to be compiled with FT_CONFIG_OPTION_USE_PNG */
/* (FT_HAS_COLOR is defined: TTF_USE_COLOR is 1) */

// Enable Signed Distance Field rendering (requires latest FreeType version)
/* (FT_RASTER_FLAG_SDF is defined: TTF_USE_SDF is 1) */
pub(crate) const DEFAULT_SDF_SPREAD: i32 = 8;

const TTF_DEFAULT_DPI: i32 = 72;

/// ZERO WIDTH NO-BREAKSPACE (Unicode byte order mark)
const UNICODE_BOM_NATIVE: u32 = 0xFEFF;
const UNICODE_BOM_SWAPPED: u32 = 0xFFFE;

/// The alignment of glyph images and surface rows (`Get_Alignment()` of
/// an x86-64 build, with SSE2; see the module documentation)
const ALIGNMENT: i32 = 16;

/* FIXME: Right now we assume the gray-scale renderer Freetype is using
supports 256 shades of gray, but we should instead key off of num_grays
in the result FT_Bitmap after the FT_Render_Glyph() call. */
const NUM_GRAYS: i32 = 256;

/* x offset = cos(((90.0-12)/360) * 2 * M_PI), or 12 degree angle */
// same value as in FT_GlyphSlot_Oblique, fixed point 16.16
const GLYPH_ITALICS: FtLong = 0x0366A;

/// `FT_FLOOR`: Handy routines for converting from fixed point 26.6
#[inline]
pub(crate) fn ft_floor(x: i64) -> i32 {
    ((x & -64) / 64) as i32
}

/// `FT_CEIL`
#[inline]
pub(crate) fn ft_ceil(x: i64) -> i32 {
    ft_floor(x.wrapping_add(63))
}

/// `F26Dot6`: Handy routine for converting to fixed point 26.6
#[inline]
fn f26dot6(x: i32) -> i32 {
    x << 6
}

/// `DIVIDE_BY_255_SIGNED`: Faster divide by 255, with same result
/// in range [0; 255]:  (x + 1   + (x >> 8)) >> 8
/// in range [-255; 0]: (x + 255 + (x >> 8)) >> 8
#[inline]
fn divide_by_255_signed(x: i32, sign_val: i32) -> i32 {
    (x + sign_val + (x >> 8)) >> 8
}

/// `DIVIDE_BY_255`: When x positive
#[inline]
fn divide_by_255(x: u32) -> u32 {
    (x + 1 + (x >> 8)) >> 8
}

pub(crate) const CACHED_METRICS: i32 = 0x20;

pub(crate) const CACHED_BITMAP: i32 = 0x01;
pub(crate) const CACHED_PIXMAP: i32 = 0x02;
pub(crate) const CACHED_COLOR: i32 = 0x04;
pub(crate) const CACHED_LCD: i32 = 0x08;
pub(crate) const CACHED_SUBPIX: i32 = 0x10;

/* SDL_ttf.h */

/// The major version of SDL_ttf this crate translates.
/// Translation of `SDL_TTF_MAJOR_VERSION`.
pub const MAJOR_VERSION: u16 = 3;
/// The minor version. Translation of `SDL_TTF_MINOR_VERSION`.
pub const MINOR_VERSION: u16 = 2;
/// The micro (patch) version. Translation of `SDL_TTF_MICRO_VERSION`.
pub const MICRO_VERSION: u16 = 2;

/// The SDL_ttf version this crate translates. Translation of
/// `SDL_TTF_VERSION` (and, with [`Version::at_least`](sdl3::Version::at_least),
/// of `SDL_TTF_VERSION_ATLEAST()`).
pub const VERSION: sdl3::Version = sdl3::Version::new(MAJOR_VERSION, MINOR_VERSION, MICRO_VERSION);

/// The upstream SDL_ttf revision this translation was made from.
pub const REVISION: &str = "SDL_ttf-release-3.2.2-a1ce3670aec736ecbf0936c43f2f0cc53aa61e5b";

/// Font style flags (`TTF_FontStyleFlags`).
pub type FontStyleFlags = u32;
/// No special style. Translation of `TTF_STYLE_NORMAL`.
pub const STYLE_NORMAL: FontStyleFlags = 0x00;
/// Bold style. Translation of `TTF_STYLE_BOLD`.
pub const STYLE_BOLD: FontStyleFlags = 0x01;
/// Italic style. Translation of `TTF_STYLE_ITALIC`.
pub const STYLE_ITALIC: FontStyleFlags = 0x02;
/// Underlined text. Translation of `TTF_STYLE_UNDERLINE`.
pub const STYLE_UNDERLINE: FontStyleFlags = 0x04;
/// Strikethrough text. Translation of `TTF_STYLE_STRIKETHROUGH`.
pub const STYLE_STRIKETHROUGH: FontStyleFlags = 0x08;

/// Hinting flags. Translation of `TTF_HintingFlags`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hinting {
    /// `TTF_HINTING_INVALID`
    Invalid = -1,
    /// Normal hinting applies standard grid-fitting. `TTF_HINTING_NORMAL`
    Normal = 0,
    /// Light hinting applies subtle adjustments to improve rendering.
    /// `TTF_HINTING_LIGHT`
    Light,
    /// Monochrome hinting adjusts the font for better rendering at lower
    /// resolutions. `TTF_HINTING_MONO`
    Mono,
    /// No hinting, the font is rendered without any grid-fitting.
    /// `TTF_HINTING_NONE`
    None,
    /// Light hinting with subpixel rendering for more precise font edges.
    /// `TTF_HINTING_LIGHT_SUBPIXEL`
    LightSubpixel,
}

/// Named font weights. Translation of `TTF_FONT_WEIGHT_THIN`.
pub const FONT_WEIGHT_THIN: i32 = 100;
/// Translation of `TTF_FONT_WEIGHT_EXTRA_LIGHT`.
pub const FONT_WEIGHT_EXTRA_LIGHT: i32 = 200;
/// Translation of `TTF_FONT_WEIGHT_LIGHT`.
pub const FONT_WEIGHT_LIGHT: i32 = 300;
/// Translation of `TTF_FONT_WEIGHT_NORMAL`.
pub const FONT_WEIGHT_NORMAL: i32 = 400;
/// Translation of `TTF_FONT_WEIGHT_MEDIUM`.
pub const FONT_WEIGHT_MEDIUM: i32 = 500;
/// Translation of `TTF_FONT_WEIGHT_SEMI_BOLD`.
pub const FONT_WEIGHT_SEMI_BOLD: i32 = 600;
/// Translation of `TTF_FONT_WEIGHT_BOLD`.
pub const FONT_WEIGHT_BOLD: i32 = 700;
/// Translation of `TTF_FONT_WEIGHT_EXTRA_BOLD`.
pub const FONT_WEIGHT_EXTRA_BOLD: i32 = 800;
/// Translation of `TTF_FONT_WEIGHT_BLACK`.
pub const FONT_WEIGHT_BLACK: i32 = 900;
/// Translation of `TTF_FONT_WEIGHT_EXTRA_BLACK`.
pub const FONT_WEIGHT_EXTRA_BLACK: i32 = 950;

/// The horizontal alignment used when rendering wrapped text.
/// Translation of `TTF_HorizontalAlignment`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HorizontalAlignment {
    /// `TTF_HORIZONTAL_ALIGN_INVALID`
    Invalid = -1,
    /// `TTF_HORIZONTAL_ALIGN_LEFT`
    Left = 0,
    /// `TTF_HORIZONTAL_ALIGN_CENTER`
    Center,
    /// `TTF_HORIZONTAL_ALIGN_RIGHT`
    Right,
}

/// Direction flags. Translation of `TTF_Direction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// `TTF_DIRECTION_INVALID`
    Invalid = 0,
    /// Left to Right. `TTF_DIRECTION_LTR`
    Ltr = 4,
    /// Right to Left. `TTF_DIRECTION_RTL`
    Rtl,
    /// Top to Bottom. `TTF_DIRECTION_TTB`
    Ttb,
    /// Bottom to Top. `TTF_DIRECTION_BTT`
    Btt,
}

/// The type of data in a glyph image. Translation of `TTF_ImageType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageType {
    /// `TTF_IMAGE_INVALID`
    Invalid,
    /// The color channels are white. `TTF_IMAGE_ALPHA`
    Alpha,
    /// The color channels have image data. `TTF_IMAGE_COLOR`
    Color,
    /// The alpha channel has signed distance field information.
    /// `TTF_IMAGE_SDF`
    Sdf,
}

/// The line cap style of font outlines (an `FT_Stroker_LineCap`
/// number). Translation of `TTF_PROP_FONT_OUTLINE_LINE_CAP_NUMBER`.
pub const PROP_FONT_OUTLINE_LINE_CAP_NUMBER: &str = "SDL_ttf.font.outline.line_cap";
/// The line join style of font outlines (an `FT_Stroker_LineJoin`
/// number). Translation of `TTF_PROP_FONT_OUTLINE_LINE_JOIN_NUMBER`.
pub const PROP_FONT_OUTLINE_LINE_JOIN_NUMBER: &str = "SDL_ttf.font.outline.line_join";
/// The miter limit of font outlines (16.16 fixed point). Translation of
/// `TTF_PROP_FONT_OUTLINE_MITER_LIMIT_NUMBER`.
pub const PROP_FONT_OUTLINE_MITER_LIMIT_NUMBER: &str = "SDL_ttf.font.outline.miter_limit";

/// `TTF_Image`
#[derive(Debug, Clone, Default)]
pub(crate) struct TtfImage {
    /// the allocation (C's `buffer` points to its start; empty is NULL)
    pub buffer: Vec<u8>,
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub rows: i32,
    pub pitch: i32,
    pub is_color: i32,
}

/// `c_glyph`: Cached glyph information
#[derive(Debug, Clone, Default)]
pub(crate) struct CGlyph {
    pub stored: i32,
    pub index: FtUInt,
    pub bitmap: TtfImage,
    pub pixmap: TtfImage,
    pub sz_left: i32,
    pub sz_top: i32,
    pub sz_width: i32,
    pub sz_rows: i32,
    pub advance: i32,
    /// the union of `subpixel.lsb_minus_rsb` (TTF_HINTING_LIGHT_SUBPIXEL,
    /// only pixmap) and `kerning_smart.rsb_delta` (other hinting)
    pub u0: i32,
    /// the union of `subpixel.translation` and `kerning_smart.lsb_delta`
    pub u1: i32,
}

/// `GlyphPosition`: Internal buffer to store positions computed by
/// TTF_Size_Internal() for rendered string by Render_Line()
#[derive(Debug, Clone)]
pub(crate) struct GlyphPosition {
    /// `font` (`glyph` is the glyph `index` in its cache)
    pub font: Weak<RefCell<FontData>>,
    pub index: FtUInt,
    pub x_offset: i32,
    pub y_offset: i32,
    pub x_advance: i32,
    pub y_advance: i32,
    pub x: i32,
    pub y: i32,
    pub offset: i32,
}

/// `GlyphPositions`
#[derive(Debug, Clone, Default)]
pub(crate) struct GlyphPositions {
    pub pos: Vec<GlyphPosition>,
    pub width26dot6: i32,
    pub height26dot6: i32,
    pub num_clusters: i32,
}

/// `CachedGlyphPositions`
#[derive(Debug, Clone)]
struct CachedGlyphPositions {
    direction: Direction,
    script: u32,
    /// `text` (`None` is NULL)
    text: Option<Vec<u8>>,
    length: usize,
    positions: GlyphPositions,
}

impl Default for CachedGlyphPositions {
    fn default() -> Self {
        CachedGlyphPositions {
            direction: Direction::Invalid,
            script: 0,
            text: None,
            length: 0,
            positions: GlyphPositions::default(),
        }
    }
}

/// A font's shared data (`struct TTF_Font`).
pub(crate) type FontRc = Rc<RefCell<FontData>>;

/// The font source (`src`), shared by a font and its copies.
type FontSource = Arc<Mutex<IoStream<'static>>>;

/// The structure used to hold internal font information (`struct
/// TTF_Font`)
pub(crate) struct FontData {
    // The name of the font
    name: Option<String>,

    // Freetype2 maintains all sorts of useful info itself
    pub(crate) face: FtFace,
    face_index: i64,

    // Properties exposed to the application
    props: Option<Properties>,

    // The current font generation, changes when glyphs need to be rebuilt
    pub(crate) generation: u32,

    // Text objects using this font
    pub(crate) text: Vec<Weak<RefCell<crate::text::TextData>>>,

    // We'll cache these ourselves
    ptsize: f32,
    hdpi: i32,
    vdpi: i32,
    pub(crate) height: i32,
    pub(crate) ascent: i32,
    descent: i32,
    pub(crate) lineskip: i32,

    // The font style
    pub(crate) style: FontStyleFlags,
    weight: i32,
    pub(crate) outline: i32,
    stroker: Option<FtStroker>,

    // Whether kerning is desired
    enable_kerning: bool,

    // Extra width in glyph bounds for text styles
    glyph_overhang: i32,

    // Information in the font for underlining
    pub(crate) line_thickness: i32,
    pub(crate) underline_top_row: i32,
    pub(crate) strikethrough_top_row: i32,

    // Cache for style-transformed glyphs
    pub(crate) glyphs: HashMap<FtUInt, CGlyph>,
    glyph_indices: HashMap<u32, FtUInt>,

    // We are responsible for closing the font stream
    src: FontSource,
    src_offset: i64,

    /* Internal buffer to store positions computed by TTF_Size_Internal()
     * for rendered string by Render_Line() */
    next_cached_positions: usize,
    cached_positions: [CachedGlyphPositions; 8],
    /// `positions` (an index into `cached_positions`; `None` is NULL)
    pub(crate) positions: Option<usize>,

    // Hinting modes
    ft_load_target: i32,
    pub(crate) render_subpixel: i32,
    pub(crate) hb_font: HbFontData,
    hb_language: HbLanguage,
    pub(crate) script: u32, // ISO 15924 script tag
    pub(crate) direction: Direction,
    pub(crate) render_sdf: bool,

    // Extra layout setting for wrapped text
    pub(crate) horizontal_align: HorizontalAlignment,

    // Fallback fonts
    fallbacks: Vec<Weak<RefCell<FontData>>>,
    fallback_for: Vec<Weak<RefCell<FontData>>>,
}

impl std::fmt::Debug for FontData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontData")
            .field("name", &self.name)
            .field("ptsize", &self.ptsize)
            .finish()
    }
}

impl FontData {
    /// The current glyph positions (`font->positions`).
    pub(crate) fn current_positions(&self) -> Option<&GlyphPositions> {
        self.positions.map(|i| &self.cached_positions[i].positions)
    }
}

/// `TTF_HANDLE_STYLE_BOLD`: Tell if SDL_ttf has to handle the style
#[inline]
fn ttf_handle_style_bold(font: &FontData) -> bool {
    font.style & STYLE_BOLD != 0
}
/// `TTF_HANDLE_STYLE_ITALIC`
#[inline]
fn ttf_handle_style_italic(font: &FontData) -> bool {
    font.style & STYLE_ITALIC != 0
}
/// `TTF_HANDLE_STYLE_UNDERLINE`
#[inline]
pub(crate) fn ttf_handle_style_underline(font: &FontData) -> bool {
    font.style & STYLE_UNDERLINE != 0
}
/// `TTF_HANDLE_STYLE_STRIKETHROUGH`
#[inline]
pub(crate) fn ttf_handle_style_strikethrough(font: &FontData) -> bool {
    font.style & STYLE_STRIKETHROUGH != 0
}

// Font styles that does not impact glyph drawing
const TTF_STYLE_NO_GLYPH_CHANGE: FontStyleFlags = STYLE_UNDERLINE | STYLE_STRIKETHROUGH;

/// The FreeType font engine/library (`TTF_state`)
struct TtfState {
    refcount: AtomicI32,
    generation: AtomicU32,
    /// `init` and `library` (`Some` once initialized)
    library: Mutex<Option<FtLibrary>>,
}

static TTF_STATE: TtfState = TtfState {
    refcount: AtomicI32::new(0),
    generation: AtomicU32::new(0),
    library: Mutex::new(None),
};

/// The library, if initialized.
fn ttf_library() -> Option<FtLibrary> {
    TTF_STATE
        .library
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// `TTF_CHECK_INITIALIZED`
fn ttf_check_initialized() -> Result<()> {
    if ttf_library().is_none() {
        return Err(Error::new("Library not initialized"));
    }
    Ok(())
}

/// `render_mode_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenderMode {
    Solid = 0,
    Shaded,
    Blended,
    Lcd,
}

/// `TTF_GetNextFontGeneration`
fn ttf_get_next_font_generation() -> u32 {
    let mut id = TTF_STATE
        .generation
        .fetch_add(1, Ordering::SeqCst)
        .wrapping_add(1);
    if id == 0 {
        id = TTF_STATE
            .generation
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);
    }
    id
}

/* (USE_DUFFS_LOOP is not defined: the loops are plain) */

/// The surface a text line is rendered to (`SDL_Surface *textbuf`): its
/// size, pitch and pixels.
#[derive(Debug)]
pub(crate) struct TextBuf {
    pub w: i32,
    pub h: i32,
    pub pitch: i32,
    pub bpp: i32,
    pub pixels: Vec<u8>,
}

/// Read a native-endian `Uint32` at `off`.
#[inline]
fn rd32(b: &[u8], off: isize) -> u32 {
    let o = off as usize;
    u32::from_ne_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Write a native-endian `Uint32` at `off`.
#[inline]
fn wr32(b: &mut [u8], off: isize, v: u32) {
    let o = off as usize;
    b[o..o + 4].copy_from_slice(&v.to_ne_bytes());
}

/// A glyph image view to blit: the image's buffer, the offset of the
/// first pixel in it, and the size to blit.
#[derive(Debug, Clone, Copy)]
struct ImageView<'a> {
    buffer: &'a [u8],
    offset: isize,
    width: i32,
    rows: i32,
    pitch: i32,
    is_color: i32,
}

/// `BG_Blended_Color`: Blend colored glyphs
fn bg_blended_color(
    image: &ImageView<'_>,
    dst: &mut [u8],
    destination: isize,
    srcskip: i32,
    dstskip: i32,
    fg_alpha: u8,
) {
    let mut src = image.offset;
    let mut d = destination;
    let width = image.width as u32;
    let mut height = image.rows as u32;

    if fg_alpha == 255 {
        while height > 0 {
            height -= 1;
            for _ in 0..width {
                wr32(dst, d, rd32(image.buffer, src));
                d += 4;
                src += 4;
            }
            src += srcskip as isize;
            d += dstskip as isize;
        }
    } else {
        while height > 0 {
            height -= 1;
            for _ in 0..width {
                /* prevent misaligned load: tmp = *src++; */
                // eventually, we can expect the compiler to replace the memcpy call with something optimized
                let mut tmp = rd32(image.buffer, src);
                src += 4;
                let mut alpha = tmp >> 24;
                tmp &= !0xFF000000;
                alpha *= fg_alpha as u32;
                alpha = divide_by_255(alpha) << 24;
                wr32(dst, d, tmp | alpha);
                d += 4;
            }
            src += srcskip as isize;
            d += dstskip as isize;
        }
    }
}

/// `BG_Blended_LCD`: Blend with LCD rendering
fn bg_blended_lcd(
    image: &ImageView<'_>,
    dst: &mut [u8],
    destination: isize,
    srcskip: i32,
    dstskip: i32,
    fg: &Color,
) {
    let mut src = image.offset;
    let mut d = destination;
    let width = image.width as u32;
    let mut height = image.rows as u32;

    let fg_r = fg.r as u32;
    let fg_g = fg.g as u32;
    let fg_b = fg.b as u32;

    while height > 0 {
        height -= 1;
        for _ in 0..width {
            /* prevent misaligned load: tmp = *src++; */
            let tmp = rd32(image.buffer, src);
            src += 4;

            if tmp != 0 {
                let bg = rd32(dst, d);

                let bg_a = bg & 0xff000000;
                let bg_r = (bg >> 16) & 0xff;
                let bg_g = (bg >> 8) & 0xff;
                let bg_b = bg & 0xff;

                let mut r = (tmp >> 16) & 0xff;
                let mut g = (tmp >> 8) & 0xff;
                let mut b = tmp & 0xff;

                r = fg_r * r + bg_r * (255 - r) + 127;
                r = divide_by_255(r);

                g = fg_g * g + bg_g * (255 - g) + 127;
                g = divide_by_255(g);

                b = fg_b * b + bg_b * (255 - b) + 127;
                b = divide_by_255(b);

                r <<= 16;
                g <<= 8;

                wr32(dst, d, r | g | b | bg_a);
            }
            d += 4;
        }
        src += srcskip as isize;
        d += dstskip as isize;
    }
}

/// `BG_Blended_Opaque`: Blended Opaque
fn bg_blended_opaque(
    image: &ImageView<'_>,
    dst: &mut [u8],
    destination: isize,
    srcskip: i32,
    dstskip: i32,
) {
    let mut src = image.offset;
    let mut d = destination;
    let width = image.width as u32;
    let mut height = image.rows as u32;

    while height > 0 {
        height -= 1;
        for _ in 0..width {
            let v = rd32(dst, d) | ((image.buffer[src as usize] as u32) << 24);
            wr32(dst, d, v);
            d += 4;
            src += 1;
        }
        src += srcskip as isize;
        d += dstskip as isize;
    }
}

/// `BG_Blended`: Blended non-opaque
fn bg_blended(
    image: &ImageView<'_>,
    dst: &mut [u8],
    destination: isize,
    srcskip: i32,
    dstskip: i32,
    fg_alpha: u8,
) {
    let mut src = image.offset;
    let mut d = destination;
    let width = image.width as u32;
    let mut height = image.rows as u32;

    while height > 0 {
        height -= 1;
        for _ in 0..width {
            let tmp = fg_alpha as u32 * image.buffer[src as usize] as u32;
            src += 1;
            let v = rd32(dst, d) | (divide_by_255(tmp) << 24);
            wr32(dst, d, v);
            d += 4;
        }
        src += srcskip as isize;
        d += dstskip as isize;
    }
}

/// `BG`
fn bg(image: &ImageView<'_>, dst: &mut [u8], destination: isize, srcskip: i32, dstskip: i32) {
    let mut src = image.offset;
    let mut d = destination;
    let width = image.width as u32;
    let mut height = image.rows as u32;

    while height > 0 {
        height -= 1;
        for _ in 0..width {
            dst[d as usize] |= image.buffer[src as usize];
            d += 1;
            src += 1;
        }
        src += srcskip as isize;
        d += dstskip as isize;
    }
}

/// `Draw_Line`: Underline and Strikethrough style. Draw a line at the
/// given row.
pub(crate) fn draw_line(
    direction: Direction,
    textbuf: &mut TextBuf,
    column: i32,
    row: i32,
    line_width: i32,
    line_thickness: i32,
    color: u32,
    render_mode: RenderMode,
) {
    let tmp = row + line_thickness - textbuf.h;
    let x_offset = column * textbuf.bpp;
    let mut dst = row as isize * textbuf.pitch as isize + x_offset as isize;
    let mut line_thickness = line_thickness;

    // No Underline/Strikethrough style if direction is vertical
    if direction == Direction::Ttb || direction == Direction::Btt {
        return;
    }

    /* Not needed because of "font->height = SDL_max(font->height, bottom_row);".
     * But if you patch to render textshaping and break line in middle of a cluster,
     * (which is a bad usage and a corner case), you need this to prevent out of bounds.
     * You can get an "ystart" for the "whole line", which is different (and smaller)
     * than the ones of the "splitted lines". */
    if tmp > 0 {
        line_thickness -= tmp;
    }
    /* Previous case also happens with SDF (render_sdf) , because 'spread' property
     * requires to increase 'ystart'
     * Check for valid value anyway.  */
    if line_thickness <= 0 {
        return;
    }

    // Wrapped mode with an unbroken line: 'line_width' is greater that 'textbuf->w'
    let line_width = std::cmp::min(line_width, textbuf.w);

    if render_mode == RenderMode::Blended || render_mode == RenderMode::Lcd {
        while line_thickness > 0 {
            line_thickness -= 1;
            for i in 0..line_width.max(0) as isize {
                wr32(&mut textbuf.pixels, dst + 4 * i, color);
            }
            dst += textbuf.pitch as isize;
        }
    } else {
        while line_thickness > 0 {
            line_thickness -= 1;
            let start = dst as usize;
            textbuf.pixels[start..start + line_width.max(0) as usize].fill(color as u8);
            dst += textbuf.pitch as isize;
        }
    }
}

/// `clip_glyph`
fn clip_glyph(
    x_: &mut i32,
    y_: &mut i32,
    image: &mut ImageView<'_>,
    textbuf: &TextBuf,
    is_lcd: bool,
) {
    let mut x = *x_;
    let mut y = *y_;

    let mut srcbpp = 1;
    if image.is_color != 0 || is_lcd {
        srcbpp = 4;
    }

    // Don't go below x=0
    if x < 0 {
        let tmp = -x;
        x = 0;
        image.width -= tmp;
        image.offset += (srcbpp * tmp) as isize;
    }
    // Don't go above textbuf->w
    let above_w = x + image.width - textbuf.w;
    if above_w > 0 {
        image.width -= above_w;
    }
    // Don't go below y=0
    if y < 0 {
        let tmp = -y;
        y = 0;
        image.rows -= tmp;
        image.offset += tmp as isize * image.pitch as isize;
    }
    // Don't go above textbuf->h
    let above_h = y + image.rows - textbuf.h;
    if above_h > 0 {
        image.rows -= above_h;
    }
    // Could be negative if (x > textbuf->w), or if (x + width < 0)
    image.width = std::cmp::max(0, image.width);
    image.rows = std::cmp::max(0, image.rows);

    /* After 'image->width' clipping:
     * Make sure 'rows' is also 0, so it doesn't break USE_DUFFS_LOOP */
    if image.width == 0 {
        image.rows = 0;
    }

    *x_ = x;
    *y_ = y;
}

/// The glyph image to blit for a position (the `want` arguments of
/// `BUILD_RENDER_LINE`).
#[derive(Debug, Clone, Copy)]
struct RenderLineKind {
    is_blended: bool,
    is_blended_opaque: bool,
    is_lcd: bool,
    want_bitmap: i32,
    want_pixmap: i32,
    want_color: i32,
    want_lcd: i32,
    want_subpixel: i32,
}

/// `Render_Line_*` (`BUILD_RENDER_LINE`): renders the font's current
/// positions to `textbuf` at (`xstart`, `ystart`).
fn render_line_kind(
    this: &Weak<RefCell<FontData>>,
    font: &mut FontData,
    kind: RenderLineKind,
    textbuf: &mut TextBuf,
    xstart: i32,
    ystart: i32,
    fg: Option<&Color>,
) -> Result<()> {
    let bpp = if kind.is_blended || kind.is_lcd { 4 } else { 1 };
    let fg_alpha = fg.map_or(255, |c| c.a);

    let count = font.current_positions().map_or(0, |p| p.pos.len());
    for i in 0..count {
        let pos = font.current_positions().unwrap().pos[i].clone();
        let idx = pos.index;
        let mut x = pos.x;
        let mut y = pos.y;

        let x_frac = x & 63;
        let mut blit = |glyph_font: &mut FontData, textbuf: &mut TextBuf| -> Result<()> {
            find_glyph_by_index(
                glyph_font,
                idx,
                kind.want_bitmap,
                kind.want_pixmap,
                kind.want_color,
                kind.want_lcd,
                kind.want_subpixel,
                x_frac,
            )?;

            let glyph = glyph_font.glyphs.get(&idx).unwrap();
            let image = if kind.want_bitmap != 0 {
                &glyph.bitmap
            } else {
                &glyph.pixmap
            };

            let mut view = ImageView {
                buffer: &image.buffer,
                offset: (ALIGNMENT - 1) as isize,
                width: image.width,
                rows: image.rows,
                pitch: image.pitch,
                is_color: image.is_color,
            };

            /* Position updated after glyph rendering */
            x = xstart + ft_floor(x as i64) + image.left;
            y = ystart + ft_floor(y as i64) - image.top;

            /* Make sure glyph is inside textbuf */
            let above_w = x + view.width - textbuf.w;
            let above_h = y + view.rows - textbuf.h;

            if x >= 0 && y >= 0 && above_w <= 0 && above_h <= 0 {
                /* Most often, glyph is inside textbuf */
                /* (the aligned copies of C OR the zero padding of the image */
                /* too, which changes nothing; see the module documentation) */
                let dst = y as isize * textbuf.pitch as isize + (x * bpp) as isize;
                if kind.is_lcd {
                    let srcskip = view.pitch - 4 * view.width;
                    let dstskip = textbuf.pitch - view.width * bpp;
                    bg_blended_lcd(
                        &view,
                        &mut textbuf.pixels,
                        dst,
                        srcskip,
                        dstskip,
                        fg.unwrap(),
                    );
                } else if !kind.is_blended || image.is_color == 0 {
                    let srcskip = view.pitch - view.width;
                    let dstskip = textbuf.pitch - view.width * bpp;
                    if kind.is_blended_opaque {
                        bg_blended_opaque(&view, &mut textbuf.pixels, dst, srcskip, dstskip);
                    } else if kind.is_blended {
                        bg_blended(&view, &mut textbuf.pixels, dst, srcskip, dstskip, fg_alpha);
                    } else if image.is_color == 0 {
                        bg(&view, &mut textbuf.pixels, dst, srcskip, dstskip);
                    }
                } else if kind.is_blended && image.is_color != 0 {
                    let srcskip = view.pitch - 4 * view.width;
                    let dstskip = textbuf.pitch - view.width * bpp;
                    bg_blended_color(&view, &mut textbuf.pixels, dst, srcskip, dstskip, fg_alpha);
                }
            } else {
                /* Modify a copy, and clip it */
                /* Intersect image glyph at (x,y) with textbuf */
                clip_glyph(&mut x, &mut y, &mut view, textbuf, kind.is_lcd);
                /* Compute dst */
                let dst = y as isize * textbuf.pitch as isize + (x * bpp) as isize;
                /* Compute srcskip, dstskip */
                let mut srcskip = view.pitch - view.width;
                let dstskip = textbuf.pitch - view.width * bpp;
                /* Render glyph at (x, y) */
                if kind.is_lcd {
                    srcskip -= 3 * view.width;
                    bg_blended_lcd(
                        &view,
                        &mut textbuf.pixels,
                        dst,
                        srcskip,
                        dstskip,
                        fg.unwrap(),
                    );
                } else if !kind.is_blended || image.is_color == 0 {
                    if kind.is_blended_opaque {
                        bg_blended_opaque(&view, &mut textbuf.pixels, dst, srcskip, dstskip);
                    } else if kind.is_blended {
                        bg_blended(&view, &mut textbuf.pixels, dst, srcskip, dstskip, fg_alpha);
                    } else if image.is_color == 0 {
                        bg(&view, &mut textbuf.pixels, dst, srcskip, dstskip);
                    }
                } else if kind.is_blended && image.is_color != 0 {
                    srcskip -= 3 * view.width;
                    bg_blended_color(&view, &mut textbuf.pixels, dst, srcskip, dstskip, fg_alpha);
                }
            }
            Ok(())
        };

        if Weak::ptr_eq(&pos.font, this) {
            blit(font, textbuf)?;
        } else {
            let Some(glyph_font) = pos.font.upgrade() else {
                return Err(Error::new("Font has been closed"));
            };
            let mut glyph_font = glyph_font.borrow_mut();
            blit(&mut glyph_font, textbuf)?;
        }
    }

    Ok(())
}

/// `Render_Line`: Render line (positions) to textbuf at (xstart, ystart)
pub(crate) fn render_line(
    render_mode: RenderMode,
    subpixel: i32,
    this: &Weak<RefCell<FontData>>,
    font: &mut FontData,
    textbuf: &mut TextBuf,
    xstart: i32,
    ystart: i32,
    fg: Color,
) -> Result<()> {
    // Subpixel with RENDER_SOLID doesn't make sense.
    // (and 'cached->subpixel.translation' would need to distinguish bitmap/pixmap).
    let is_opaque = fg.a == 255;

    let subpix = if subpixel == 0 { 0 } else { CACHED_SUBPIX };
    let base = RenderLineKind {
        is_blended: false,
        is_blended_opaque: false,
        is_lcd: false,
        want_bitmap: 0,
        want_pixmap: 0,
        want_color: 0,
        want_lcd: 0,
        want_subpixel: subpix,
    };

    /* Call_Specific_Render_Line */
    match render_mode {
        RenderMode::Shaded => {
            let kind = RenderLineKind {
                want_pixmap: CACHED_PIXMAP,
                ..base
            };
            render_line_kind(this, font, kind, textbuf, xstart, ystart, None)
        }
        RenderMode::Blended => {
            if is_opaque {
                let kind = RenderLineKind {
                    is_blended: true,
                    is_blended_opaque: true,
                    want_color: CACHED_COLOR,
                    ..base
                };
                render_line_kind(this, font, kind, textbuf, xstart, ystart, None)
            } else {
                let kind = RenderLineKind {
                    is_blended: true,
                    want_color: CACHED_COLOR,
                    ..base
                };
                render_line_kind(this, font, kind, textbuf, xstart, ystart, Some(&fg))
            }
        }
        RenderMode::Lcd => {
            let kind = RenderLineKind {
                is_lcd: true,
                want_lcd: CACHED_LCD,
                ..base
            };
            render_line_kind(this, font, kind, textbuf, xstart, ystart, Some(&fg))
        }
        RenderMode::Solid => {
            let kind = RenderLineKind {
                want_bitmap: CACHED_BITMAP,
                want_subpixel: 0,
                ..base
            };
            render_line_kind(this, font, kind, textbuf, xstart, ystart, None)
        }
    }
}

/// `AllocateAlignedPixels`: Create a surface with memory:
/// - pitch is rounded to alignment
/// - address is aligned
///
/// If format is 4 bytes per pixel, bgcolor is used to initialize each
/// 4-byte word in the image data.
///
/// Otherwise, the low byte of format is used to initialize each byte in
/// the image data.
fn allocate_aligned_pixels(
    width: usize,
    height: usize,
    bytes_per_pixel: usize,
    bgcolor: u32,
) -> Option<TextBuf> {
    let alignment = (ALIGNMENT - 1) as usize;

    /*
     * 1/ Line size is "width * bytes_per_pixel"
     *
     * 2/ We add a right padding, because we process glyph from source to destination by
     * blocks of 'alignment + 1' bytes.  (Using SSE 128 instruction for instance when renderering,
     * but this isn't always the case for all modes).
     *
     * We need to make sure the last transfer doesn't go too much outside!
     *
     * Considerer also for instance, that when we read 1 block of 16 bytes from source, for the blended
     * format (bbp == 4), it writes(and reads) 4 blocks of 16 in the dest, like BG_Blended_SSE()).
     *
     * Remark: for Solid/Shaded, block ratio read/write is 1:1.
     * For Color / LCD / SDF, it is byte vs byte or int. They are also fallback for
     * Solid/Shaded/Blend, when it isn't contained in textbuf, see clip_glyph()
     *
     * So the pitch must contain "width * bytes_per_pixel", plus in the
     * worst case, writing at last pixel (1 * bytes_per_pixel), an extra "alignment * bytes_per_pixel".
     * (Using the destination bytes_per_pixel is a safe upper bound for the ratio).
     *
     * Also, we always write at a block-aligned adresses.
     * - address is aligned 'SDL_aligned_alloc((alignment + 1), size)'
     * - the pitch is aligned 'pitch &= ~alignment'
     * So that each line is aligned.
     *
     * Remark: we can safely align the pitch (pitch &= ~alignment), without adding more to pitch,
     * becaJuse we know we always write blocks at block-aligned addresses.
     * So pitch won't need to be more than a multiple of block size.
     *
     * So pitch is:
     *  ((width * bytes_per_pixel) + (alignment * bytes_per_pixel) ) & ~alignment
     *  ==
     *  ((width + alignment) * bytes_per_pixel ) & ~alignment
     *
     * which is different from:
     *  ((width + alignment) & ~alignment)) * bytes_per_pixel  (which fails.)
     *
     * Remark: to test memory issues, it is useful to patch SDL to use real memalign/free
     * so that valgrind check more precisely out of bounds.
     */
    if width > i32::MAX as usize || height > i32::MAX as usize {
        return None;
    }
    let mut pitch = width.checked_add(alignment)?.checked_mul(bytes_per_pixel)?;
    if pitch > i32::MAX as usize {
        return None;
    }
    pitch &= !alignment;

    let size = height.checked_mul(pitch)?;

    let mut pixels: Vec<u8> = Vec::new();
    if pixels.try_reserve_exact(size).is_err() {
        return None;
    }

    if bytes_per_pixel == 4 {
        pixels.resize(size / 4 * 4, 0);
        for chunk in pixels.chunks_exact_mut(4) {
            chunk.copy_from_slice(&bgcolor.to_ne_bytes());
        }
        pixels.resize(size, 0);
    } else {
        pixels.resize(size, (bgcolor & 0xff) as u8);
    }

    Some(TextBuf {
        w: width as i32,
        h: height as i32,
        pitch: pitch as i32,
        bpp: bytes_per_pixel as i32,
        pixels,
    })
}

/// A text surface under construction: the pixels, and how the surface
/// is set up when it is made (`Create_Surface_*`).
#[derive(Debug)]
pub(crate) struct TextSurface {
    pub buf: TextBuf,
    format: PixelFormat,
    /// the palette of an indexed surface (`None` keeps the default one)
    palette: Option<Vec<Color>>,
    color_key: bool,
    blend: bool,
}

impl TextSurface {
    /// Make the `SDL_Surface`.
    pub(crate) fn into_surface(self) -> Result<Surface<'static>> {
        let TextBuf {
            w,
            h,
            pitch,
            pixels,
            ..
        } = self.buf;
        let mut surface = Surface::from_vec(w, h, self.format, pixels, pitch)?;

        // Allocate a palette if needed
        if self.format == PixelFormat::INDEX8 {
            let palette = surface.create_palette()?;
            if let Some(colors) = self.palette {
                let mut p = palette.write().unwrap_or_else(|e| e.into_inner());
                p.colors_mut()[..colors.len()].copy_from_slice(&colors);
            }
        }

        if self.color_key {
            surface.set_color_key(Some(0))?;
        }

        if self.blend {
            surface.set_blend_mode(BlendMode::BLEND)?;
        }

        Ok(surface)
    }
}

/// The default palette of an 8-bit surface (`SDL_CreatePalette` sets
/// every color to opaque white).
fn default_palette() -> Vec<Color> {
    vec![
        Color {
            r: 255,
            g: 255,
            b: 255,
            a: 255
        };
        256
    ]
}

/// `Create_Surface_Solid`
pub(crate) fn create_surface_solid(
    width: i32,
    height: i32,
    fg: Color,
    color: &mut u32,
) -> Option<TextSurface> {
    let buf = allocate_aligned_pixels(width as usize, height as usize, 1, 0)?;

    // Underline/Strikethrough color style
    *color = 1;

    // Fill the palette: 1 is foreground
    let mut palette = default_palette();
    palette[0].r = 255 - fg.r;
    palette[0].g = 255 - fg.g;
    palette[0].b = 255 - fg.b;
    palette[1].r = fg.r;
    palette[1].g = fg.g;
    palette[1].b = fg.b;
    palette[1].a = fg.a;

    Some(TextSurface {
        buf,
        format: PixelFormat::INDEX8,
        palette: Some(palette),
        color_key: true,
        blend: false,
    })
}

/// `Create_Surface_Shaded`
pub(crate) fn create_surface_shaded(
    width: i32,
    height: i32,
    fg: Color,
    bg: Color,
    color: &mut u32,
) -> Option<TextSurface> {
    let buf = allocate_aligned_pixels(width as usize, height as usize, 1, 0)?;
    let bg_alpha = bg.a;
    let mut bg = bg;
    let mut blend = false;

    // Underline/Strikethrough color style
    *color = (NUM_GRAYS - 1) as u32;

    // Support alpha blending
    if fg.a != 255 || bg.a != 255 {
        blend = true;

        // Would disturb alpha palette
        if bg.a == 255 {
            bg.a = 0;
        }
    }

    // Fill the palette with NUM_GRAYS levels of shading from bg to fg
    let mut palette = default_palette();
    {
        let rdiff = fg.r as i32 - bg.r as i32;
        let gdiff = fg.g as i32 - bg.g as i32;
        let bdiff = fg.b as i32 - bg.b as i32;
        let adiff = fg.a as i32 - bg.a as i32;
        let sign_r = if rdiff >= 0 { 1 } else { 255 };
        let sign_g = if gdiff >= 0 { 1 } else { 255 };
        let sign_b = if bdiff >= 0 { 1 } else { 255 };
        let sign_a = if adiff >= 0 { 1 } else { 255 };

        for i in 0..NUM_GRAYS {
            /* Compute color[i] = (i * color_diff / 255) */
            let tmp_r = i * rdiff;
            let tmp_g = i * gdiff;
            let tmp_b = i * bdiff;
            let tmp_a = i * adiff;
            let c = &mut palette[i as usize];
            c.r = (bg.r as i32 + divide_by_255_signed(tmp_r, sign_r)) as u8;
            c.g = (bg.g as i32 + divide_by_255_signed(tmp_g, sign_g)) as u8;
            c.b = (bg.b as i32 + divide_by_255_signed(tmp_b, sign_b)) as u8;
            c.a = (bg.a as i32 + divide_by_255_signed(tmp_a, sign_a)) as u8;
        }

        // Make sure background has the correct alpha value
        palette[0].a = bg_alpha;
    }

    Some(TextSurface {
        buf,
        format: PixelFormat::INDEX8,
        palette: Some(palette),
        color_key: false,
        blend,
    })
}

/// `Create_Surface_Blended` (`None` with a zero `width`, where C returns
/// NULL without an error)
pub(crate) fn create_surface_blended(
    width: i32,
    height: i32,
    fg: Color,
    color: &mut u32,
) -> Option<TextSurface> {
    // Background color: initialize with fg and 0 alpha
    let bgcolor = ((fg.r as u32) << 16) | ((fg.g as u32) << 8) | fg.b as u32;

    // Underline/Strikethrough color style
    *color = bgcolor | ((fg.a as u32) << 24);

    // Create the target surface if required
    if width != 0 {
        let buf = allocate_aligned_pixels(width as usize, height as usize, 4, bgcolor)?;
        return Some(TextSurface {
            buf,
            format: PixelFormat::ARGB8888,
            palette: None,
            color_key: false,
            blend: false,
        });
    }

    None
}

/// `Create_Surface_LCD`
pub(crate) fn create_surface_lcd(
    width: i32,
    height: i32,
    fg: Color,
    bg: Color,
    color: &mut u32,
) -> Option<TextSurface> {
    // Background color
    let bgcolor =
        ((bg.a as u32) << 24) | ((bg.r as u32) << 16) | ((bg.g as u32) << 8) | bg.b as u32;

    // Underline/Strikethrough color style
    *color = ((bg.a as u32) << 24) | ((fg.r as u32) << 16) | ((fg.g as u32) << 8) | fg.b as u32;

    // Create the target surface if required
    if width != 0 {
        let buf = allocate_aligned_pixels(width as usize, height as usize, 4, bgcolor)?;
        return Some(TextSurface {
            buf,
            format: PixelFormat::ARGB8888,
            palette: None,
            color_key: false,
            blend: false,
        });
    }

    None
}

/// The version of SDL_ttf in use (`SDL_TTF_VERSION`). Translation of
/// `TTF_Version()`.
pub fn version() -> sdl3::Version {
    VERSION
}

/// `TTF_SetFTError` (`USE_FREETYPE_ERRORS` is not defined: the message
/// alone)
fn ttf_set_ft_error(msg: &'static str, _error: FtError) -> Error {
    Error::new(msg)
}

/// Initialize SDL_ttf. Translation of `TTF_Init()`.
pub fn init() -> Result<()> {
    TTF_STATE.refcount.fetch_add(1, Ordering::SeqCst);

    let mut library = TTF_STATE.library.lock().unwrap_or_else(|e| e.into_inner());
    if library.is_some() {
        return Ok(());
    }

    match ft_init_freetype() {
        Ok(lib) => {
            /* TTF_USE_SDF: the renderer properties are not set (#if 0) */
            *library = Some(lib);
            Ok(())
        }
        Err(error) => {
            TTF_STATE.refcount.fetch_sub(1, Ordering::SeqCst);
            Err(ttf_set_ft_error("Couldn't init FreeType engine", error))
        }
    }
}

/// Query the version of the FreeType library in use (`(0, 0, 0)` before
/// [`init`]). Translation of `TTF_GetFreeTypeVersion()`.
pub fn freetype_version() -> (i32, i32, i32) {
    match ttf_library() {
        Some(lib) => ft_library_version(&lib),
        None => (0, 0, 0),
    }
}

/// Query the version of the HarfBuzz library in use (the version of the
/// translated HarfBuzz). Translation of `TTF_GetHarfBuzzVersion()`.
pub fn harfbuzz_version() -> (i32, i32, i32) {
    let (hb_major, hb_minor, hb_micro) = hb_version();
    (hb_major as i32, hb_minor as i32, hb_micro as i32)
}

/// `IOread`
fn io_read(src: &FontSource, src_offset: i64, offset: u64, buffer: &mut [u8]) -> u64 {
    let mut src = src.lock().unwrap_or_else(|e| e.into_inner());
    let _ = src.seek(src_offset.wrapping_add(offset as i64), IoWhence::Set);
    src.read(buffer) as u64
}

/// The parameters of [`Font::open_with`]: the properties of
/// `TTF_OpenFontWithProperties()`.
#[derive(Debug, Default)]
pub struct FontOptions<'a> {
    /// The font to use as a template for all font properties
    /// (`TTF_PROP_FONT_CREATE_EXISTING_FONT`).
    pub existing_font: Option<&'a Font>,
    /// The font file to open, if no stream is given
    /// (`TTF_PROP_FONT_CREATE_FILENAME_STRING`).
    pub filename: Option<&'a str>,
    /// The stream containing the font data
    /// (`TTF_PROP_FONT_CREATE_IOSTREAM_POINTER`; the font owns it, as
    /// with `TTF_PROP_FONT_CREATE_IOSTREAM_AUTOCLOSE_BOOLEAN`).
    pub iostream: Option<IoStream<'static>>,
    /// The offset in the stream where the font data begins
    /// (`TTF_PROP_FONT_CREATE_IOSTREAM_OFFSET_NUMBER`).
    pub iostream_offset: i64,
    /// The point size of the font (`TTF_PROP_FONT_CREATE_SIZE_FLOAT`).
    pub size: f32,
    /// The face index of the font (`TTF_PROP_FONT_CREATE_FACE_NUMBER`;
    /// `None` is the default -1).
    pub face: Option<i64>,
    /// The horizontal DPI (`TTF_PROP_FONT_CREATE_HORIZONTAL_DPI_NUMBER`).
    pub hdpi: u32,
    /// The vertical DPI (`TTF_PROP_FONT_CREATE_VERTICAL_DPI_NUMBER`).
    pub vdpi: u32,
}

/// The internal structure containing font information. Translation of
/// `TTF_Font`; dropping it is `TTF_CloseFont()`.
pub struct Font {
    pub(crate) rc: FontRc,
}

impl std::fmt::Debug for Font {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Font").field(&self.rc.borrow()).finish()
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        close_font(&self.rc);
    }
}

impl Font {
    /// Create a font with the specified properties. Translation of
    /// `TTF_OpenFontWithProperties()`.
    pub fn open_with(options: FontOptions<'_>) -> Result<Font> {
        let FontOptions {
            existing_font,
            filename: file,
            iostream,
            iostream_offset,
            size,
            face,
            hdpi,
            vdpi,
        } = options;
        let mut src = iostream.map(|s| Arc::new(Mutex::new(s)));
        let mut src_offset = iostream_offset;
        let mut ptsize = size;
        let mut face_index = face.unwrap_or(-1);
        let mut hdpi = hdpi;
        let mut vdpi = vdpi;

        let Some(library) = ttf_library() else {
            return Err(Error::new("Library not initialized"));
        };

        let existing = existing_font.map(|f| f.rc.borrow());

        if let Some(existing) = existing.as_ref() {
            if src.is_none() {
                src = Some(existing.src.clone());
                src_offset = existing.src_offset;
            }
        } else if src.is_none() {
            let Some(file) = file else {
                return Err(Error::new(
                    "You must set either TTF_PROP_FONT_CREATE_FILENAME_STRING or TTF_PROP_FONT_CREATE_IOSTREAM_POINTER",
                ));
            };

            src = Some(Arc::new(Mutex::new(IoStream::from_file(file, "rb")?)));
        }
        let src = src.unwrap();

        // Check to make sure we can seek in this stream
        let position = src.lock().unwrap_or_else(|e| e.into_inner()).tell();
        if !matches!(position, Ok(p) if p >= 0) {
            return Err(Error::new("Can't seek in stream"));
        }

        let mut name = None;
        if let Some(existing) = existing.as_ref() {
            name = existing.name.clone();
            if face_index == -1 {
                face_index = existing.face_index;
            }
            if ptsize == 0.0 {
                ptsize = existing.ptsize;
            }
            if hdpi == 0 {
                hdpi = existing.hdpi as u32;
            }
            if vdpi == 0 {
                vdpi = existing.vdpi as u32;
            }
        } else if let Some(file) = file {
            let n = match file.rfind('/') {
                Some(i) => &file[i + 1..],
                None => match file.rfind('\\') {
                    Some(i) => &file[i + 1..],
                    None => file,
                },
            };
            name = Some(n.to_string());
        }
        if face_index < 0 {
            face_index = 0;
        }
        if hdpi == 0 {
            hdpi = TTF_DEFAULT_DPI as u32;
        }
        if vdpi == 0 {
            vdpi = TTF_DEFAULT_DPI as u32;
        }

        let io_size = src
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .size()
            .unwrap_or(-1);
        let stream_size = io_size.wrapping_sub(src_offset) as u64;
        let read_src = src.clone();
        let stream = FtStreamRec::open_callback(
            stream_size,
            Box::new(move |offset, buffer| io_read(&read_src, src_offset, offset, buffer)),
        );

        let args = FtOpenArgs {
            flags: FT_OPEN_STREAM,
            stream: Some(stream),
            ..Default::default()
        };

        let mut face = ft_open_face(&library, args, face_index as FtLong)
            .map_err(|e| ttf_set_ft_error("Couldn't load font file", e))?;

        // Set charmap for loaded font
        let mut found: Option<usize> = None;
        for (i, charmap) in face.charmaps.iter().enumerate() {
            if charmap.charmap.platform_id == 3 && charmap.charmap.encoding_id == 10 {
                // UCS-4 Unicode
                found = Some(i);
                break;
            }
        }
        if found.is_none() {
            for (i, charmap) in face.charmaps.iter().enumerate() {
                let (p, e) = (charmap.charmap.platform_id, charmap.charmap.encoding_id);
                if (p == 3 && e == 1) // Windows Unicode
                    || (p == 3 && e == 0) // Windows Symbol
                    || (p == 2 && e == 1) // ISO Unicode
                    || p == 0
                {
                    // Apple Unicode
                    found = Some(i);
                    break;
                }
            }
        }
        if let Some(found) = found {
            // If this fails, continue using the default charmap
            let _ = ft_set_charmap(&mut face, found);
        }

        let mut font = FontData {
            name,
            face,
            face_index,
            props: None,
            generation: ttf_get_next_font_generation(),
            text: Vec::new(),
            ptsize: 0.0,
            hdpi: 0,
            vdpi: 0,
            height: 0,
            ascent: 0,
            descent: 0,
            lineskip: 0,
            style: STYLE_NORMAL,
            weight: 0,
            outline: 0,
            stroker: None,
            enable_kerning: false,
            glyph_overhang: 0,
            line_thickness: 0,
            underline_top_row: 0,
            strikethrough_top_row: 0,
            glyphs: HashMap::with_capacity(128),
            glyph_indices: HashMap::with_capacity(128),
            src,
            src_offset,
            next_cached_positions: 0,
            cached_positions: Default::default(),
            positions: None,
            ft_load_target: 0,
            render_subpixel: 0,
            hb_font: HbFontData::create(Arc::new(HbFace::empty())),
            hb_language: HB_LANGUAGE_INVALID,
            script: 0,
            direction: Direction::Invalid,
            render_sdf: false,
            horizontal_align: HorizontalAlignment::Left,
            fallbacks: Vec::new(),
            fallback_for: Vec::new(),
        };

        // Set the default font style
        if let Some(existing) = existing.as_ref() {
            font.style = existing.style;
            font.weight = existing.weight;
            font.outline = existing.outline;
            font.ft_load_target = existing.ft_load_target;
            font.enable_kerning = existing.enable_kerning;
        } else {
            font.style = STYLE_NORMAL;
            font.outline = 0;
            font.ft_load_target = FT_LOAD_TARGET_NORMAL;
            set_font_kerning(&mut font, true);

            // Retrieve the weight from the OS2 TrueType table
            let os2_weight = match ft_get_sfnt_table(&font.face, FT_SFNT_OS2) {
                Some(FtSfntTable::Os2(os2)) => Some(os2.usWeightClass),
                _ => None,
            };
            if let Some(w) = os2_weight.filter(|&w| w != 0) {
                font.weight = w as i32;
            } else if font.face.style_flags & FT_STYLE_FLAG_BOLD != 0 {
                font.weight = FONT_WEIGHT_BOLD;
            } else {
                font.weight = FONT_WEIGHT_NORMAL;
            }
        }
        drop(existing);

        /* TTF_USE_HARFBUZZ */
        font.hb_font = hb_ft_font_create(&mut font.face);

        /* Default load-flags of hb_ft_font_create is no-hinting.
         * So unless you call hb_ft_font_set_load_flags to match what flags you use for rendering,
         * you will get mismatching advances and raster. */
        hb_ft_font_set_load_flags(&mut font.hb_font, FT_LOAD_DEFAULT | font.ft_load_target);

        font.hb_language = hb_language_from_string(b"");

        let rc = Rc::new(RefCell::new(font));
        let font = Font { rc };

        let weak = Rc::downgrade(&font.rc);
        if set_font_size_dpi(
            &weak,
            &mut font.rc.borrow_mut(),
            ptsize,
            hdpi as i32,
            vdpi as i32,
        )
        .is_err()
        {
            return Err(ttf_set_ft_error("Couldn't set font size", 0));
        }

        Ok(font)
    }

    /// Create a font from a file, using a specified point size. Translation
    /// of `TTF_OpenFont()`.
    pub fn open(file: &str, ptsize: f32) -> Result<Font> {
        Font::open_with(FontOptions {
            filename: Some(file),
            size: ptsize,
            ..Default::default()
        })
    }

    /// Create a font from an IoStream, using a specified point size (the
    /// font owns the stream). Translation of `TTF_OpenFontIO()`.
    pub fn open_io(src: IoStream<'static>, ptsize: f32) -> Result<Font> {
        Font::open_with(FontOptions {
            iostream: Some(src),
            size: ptsize,
            ..Default::default()
        })
    }

    /// Create a copy of an existing font. Translation of
    /// `TTF_CopyFont()`.
    pub fn copy(&self) -> Result<Font> {
        Font::open_with(FontOptions {
            existing_font: Some(self),
            ..Default::default()
        })
    }

    pub(crate) fn weak(&self) -> Weak<RefCell<FontData>> {
        Rc::downgrade(&self.rc)
    }

    /// Get the properties associated with a font. Translation of
    /// `TTF_GetFontProperties()`.
    pub fn properties(&self) -> Properties {
        let mut font = self.rc.borrow_mut();
        font.props.get_or_insert_with(Properties::new).clone()
    }

    /// Get the font generation, which changes when the glyphs need to be
    /// rebuilt. Translation of `TTF_GetFontGeneration()`.
    pub fn generation(&self) -> u32 {
        self.rc.borrow().generation
    }

    /// Add a fallback font. Translation of `TTF_AddFallbackFont()`.
    pub fn add_fallback_font(&self, fallback: &Font) -> Result<()> {
        self.rc.borrow_mut().fallbacks.push(fallback.weak());
        fallback.rc.borrow_mut().fallback_for.push(self.weak());

        update_font_text(&self.rc, None);
        Ok(())
    }

    /// Remove a fallback font. Translation of `TTF_RemoveFallbackFont()`.
    pub fn remove_fallback_font(&self, fallback: &Font) {
        remove_fallback_font(&self.rc, &fallback.rc);
    }

    /// Remove all fallback fonts. Translation of
    /// `TTF_ClearFallbackFonts()`.
    pub fn clear_fallback_fonts(&self) {
        clear_fallback_fonts(&self.rc);
    }
}

/// `UpdateFontTextCallback` over the font's texts
fn update_font_text_callback(font: &mut FontData) {
    font.text.retain(|t| t.strong_count() > 0);
    for text in &font.text {
        if let Some(text) = text.upgrade() {
            if let Ok(mut text) = text.try_borrow_mut() {
                text.needs_layout_update = true;
            }
        }
    }
}

/// `UpdateFontText`
pub(crate) fn update_font_text(font: &FontRc, initial_font: Option<&FontRc>) {
    let initial_font = match initial_font {
        None => font,
        Some(i) => {
            if Rc::ptr_eq(font, i) {
                // font fallback loop
                return;
            }
            i
        }
    };

    let fallback_for: Vec<Weak<RefCell<FontData>>> = match font.try_borrow_mut() {
        Ok(mut f) => {
            update_font_text_callback(&mut f);
            f.fallback_for.clone()
        }
        Err(_) => Vec::new(),
    };

    for list in fallback_for {
        if let Some(f) = list.upgrade() {
            update_font_text(&f, Some(initial_font));
        }
    }
}

/// `UpdateFontText` for a font whose data is borrowed by the caller.
pub(crate) fn update_font_text_borrowed(this: &Weak<RefCell<FontData>>, font: &mut FontData) {
    update_font_text_callback(font);

    for list in font.fallback_for.clone() {
        if let Some(f) = list.upgrade() {
            if let Some(this) = this.upgrade() {
                update_font_text(&f, Some(&this));
            }
        }
    }
}

/// `TTF_RemoveFallbackFont`
fn remove_fallback_font(font: &FontRc, fallback: &FontRc) {
    {
        let mut f = font.borrow_mut();
        if let Some(i) = f
            .fallbacks
            .iter()
            .position(|w| std::ptr::eq(w.as_ptr(), Rc::as_ptr(fallback)))
        {
            f.fallbacks.remove(i);
        }
    }

    {
        let mut fb = fallback.borrow_mut();
        if let Some(i) = fb
            .fallback_for
            .iter()
            .position(|w| std::ptr::eq(w.as_ptr(), Rc::as_ptr(font)))
        {
            fb.fallback_for.remove(i);
        }
    }

    update_font_text(font, None);
}

/// `TTF_ClearFallbackFonts`
fn clear_fallback_fonts(font: &FontRc) {
    loop {
        let first = font.borrow().fallbacks.first().cloned();
        let Some(first) = first else {
            break;
        };
        match first.upgrade() {
            Some(fallback) => remove_fallback_font(font, &fallback),
            None => {
                font.borrow_mut().fallbacks.remove(0);
            }
        }
    }
}

/// `TTF_InitFontMetrics`: Update font parameter depending on a style
/// change
fn ttf_init_font_metrics(font: &mut FontData) {
    let face = &font.face;
    let underline_offset;

    // Make sure that our font face is scalable (global metrics)
    if ft_is_scalable(face) {
        // Get the scalable font metrics for this font
        let scale = face.size.metrics.y_scale;
        font.ascent = ft_ceil(ft_mul_fix(face.ascender as FtLong, scale));
        font.descent = ft_ceil(ft_mul_fix(face.descender as FtLong, scale));
        font.height = ft_ceil(ft_mul_fix(
            face.ascender as FtLong - face.descender as FtLong,
            scale,
        ));
        font.lineskip = ft_ceil(ft_mul_fix(face.height as FtLong, scale));
        underline_offset = ft_floor(ft_mul_fix(face.underline_position as FtLong, scale));
        font.line_thickness = ft_floor(ft_mul_fix(face.underline_thickness as FtLong, scale));
    } else {
        // Get the font metrics for this font, for the selected size
        font.ascent = ft_ceil(face.size.metrics.ascender);
        font.descent = ft_ceil(face.size.metrics.descender);
        font.height = ft_ceil(face.size.metrics.height);
        font.lineskip = ft_ceil(face.size.metrics.height);
        /* face->underline_position and face->underline_height are only
         * relevant for scalable formats (see freetype.h FT_FaceRec) */
        underline_offset = font.descent / 2;
        font.line_thickness = 1;
    }

    if font.line_thickness < 1 {
        font.line_thickness = 1;
    }

    font.underline_top_row = font.ascent - underline_offset - 1;
    font.strikethrough_top_row = font.height / 2;

    // Adjust OutlineStyle, only for scalable fonts
    /* TTF_Size(): increase w and h by 2 * outline, translate positionning by 1 * outline */
    if font.outline > 0 {
        let fo = font.outline;
        font.line_thickness += 2 * fo;
        font.underline_top_row -= fo;
        font.strikethrough_top_row -= fo;
    }

    // Robustness: no negative values allowed
    font.underline_top_row = std::cmp::max(0, font.underline_top_row);
    font.strikethrough_top_row = std::cmp::max(0, font.strikethrough_top_row);

    // Update height according to the needs of the underline style
    if ttf_handle_style_underline(font) {
        let bottom_row = font.underline_top_row + font.line_thickness;
        font.height = std::cmp::max(font.height, bottom_row);
    }
    // Update height according to the needs of the strikethrough style
    if ttf_handle_style_strikethrough(font) {
        let bottom_row = font.strikethrough_top_row + font.line_thickness;
        font.height = std::cmp::max(font.height, bottom_row);
    }

    font.glyph_overhang = font.face.size.metrics.y_ppem as i32 / 10;
}

/// `Flush_Glyph_Image`
fn flush_glyph_image(image: &mut TtfImage) {
    image.buffer = Vec::new();
}

/// `Flush_Glyph`
fn flush_glyph(glyph: &mut CGlyph) {
    glyph.stored = 0;
    flush_glyph_image(&mut glyph.pixmap);
    flush_glyph_image(&mut glyph.bitmap);
}

/// `Flush_Cache`
pub(crate) fn flush_cache(font: &mut FontData) {
    for glyph in font.glyphs.values_mut() {
        /* FlushCacheCallback */
        if glyph.stored != 0 {
            flush_glyph(glyph);
        }
    }

    for cached in font.cached_positions.iter_mut() {
        if cached.text.is_some() {
            cached.text = None;
            cached.length = 0;
        }
        cached.positions.pos = Vec::new();
    }
    font.positions = None;

    font.generation = ttf_get_next_font_generation();
}

/// Read the byte at `i` of a decoded row (0 past the bitmap's end, where C
/// would read out of bounds for a negative pitch).
#[inline]
fn src_byte(buf: &[u8], i: isize) -> u8 {
    if i < 0 {
        0
    } else {
        buf.get(i as usize).copied().unwrap_or(0)
    }
}

/// `Load_Glyph`
fn load_glyph(font: &mut FontData, cached: &mut CGlyph, want: i32, translation: i32) -> Result<()> {
    let alignment = ALIGNMENT - 1;

    let mut ft_load = FT_LOAD_DEFAULT | font.ft_load_target;

    /* TTF_USE_COLOR */
    if want & CACHED_COLOR != 0 {
        ft_load |= FT_LOAD_COLOR;
    }

    if ft_has_svg(&font.face) {
        // We won't get metrics unless we add FT_LOAD_COLOR
        ft_load |= FT_LOAD_COLOR;
    }

    ft_load_glyph(&mut font.face, cached.index, ft_load)
        .map_err(|e| ttf_set_ft_error("FT_Load_Glyph() failed", e))?;

    // Get our glyph shortcut

    if want & CACHED_LCD != 0 && font.face.glyph.format == FT_GLYPH_FORMAT_BITMAP {
        return Err(Error::new("LCD mode not possible with bitmap font"));
    }

    // Get the glyph metrics, always needed
    if cached.stored == 0 {
        let slot = &font.face.glyph;
        cached.sz_left = slot.bitmap_left;
        cached.sz_top = slot.bitmap_top;
        cached.sz_rows = slot.bitmap.rows as i32;
        cached.sz_width = slot.bitmap.width as i32;

        /* Current version of freetype is 2.9.1, but on older freetype (2.8.1) this can be 0.
         * Try to get them from 'FT_Glyph_Metrics' */
        if cached.sz_left == 0 && cached.sz_top == 0 && cached.sz_rows == 0 && cached.sz_width == 0
        {
            let metrics = &slot.metrics;
            let minx = ft_floor(metrics.horiBearingX);
            let maxx = ft_ceil(metrics.horiBearingX.wrapping_add(metrics.width));
            let maxy = ft_floor(metrics.horiBearingY);
            let miny = maxy.wrapping_sub(ft_ceil(metrics.height));

            cached.sz_left = minx;
            cached.sz_top = maxy;
            cached.sz_rows = maxy.wrapping_sub(miny);
            cached.sz_width = maxx.wrapping_sub(minx);
        }

        // All FP 26.6 are 'long' but 'int' should be engouh
        cached.advance = slot.metrics.horiAdvance as i32; // FP 26.6

        if font.render_subpixel == 0 {
            // FT KERNING_MODE_SMART
            cached.u0 = slot.rsb_delta as i32; // FP 26.6
            cached.u1 = slot.lsb_delta as i32; // FP 26.6
        } else {
            // FT LCD_MODE_LIGHT_SUBPIXEL
            cached.u0 = slot.lsb_delta.wrapping_sub(slot.rsb_delta) as i32; // FP 26.6
            cached.u1 = 0; // FP 26.6
        }

        // Adjust for bold text
        if ttf_handle_style_bold(font) {
            cached.sz_width = cached.sz_width.wrapping_add(font.glyph_overhang);
            cached.advance = cached.advance.wrapping_add(f26dot6(font.glyph_overhang));
        }

        // Adjust for italic text
        if ttf_handle_style_italic(font) && font.face.glyph.format == FT_GLYPH_FORMAT_OUTLINE {
            cached.sz_width = cached
                .sz_width
                .wrapping_add(((GLYPH_ITALICS * font.height as FtLong) >> 16) as i32);
        }

        // Adjust for subpixel
        if font.render_subpixel != 0 {
            cached.sz_width = cached.sz_width.wrapping_add(1);
        }

        // Adjust for SDF
        if font.render_sdf {
            cached.sz_width = cached.sz_width.wrapping_add(2 * DEFAULT_SDF_SPREAD);
            cached.sz_rows = cached.sz_rows.wrapping_add(2 * DEFAULT_SDF_SPREAD);
        }

        cached.stored |= CACHED_METRICS;
    }

    if ((want & CACHED_BITMAP) != 0 && (cached.stored & CACHED_BITMAP) == 0)
        || ((want & CACHED_PIXMAP) != 0 && (cached.stored & CACHED_PIXMAP) == 0)
        || ((want & CACHED_COLOR) != 0 && (cached.stored & CACHED_COLOR) == 0)
        || ((want & CACHED_LCD) != 0 && (cached.stored & CACHED_LCD) == 0)
        || (want & CACHED_SUBPIX) != 0
    {
        let mono = want & CACHED_BITMAP != 0;
        let mut glyph: Option<FtGlyph> = None;
        let ft_render_mode;

        let slot_format = font.face.glyph.format;
        if mono && slot_format != FT_GLYPH_FORMAT_SVG {
            ft_render_mode = FT_RENDER_MODE_MONO;
        } else {
            let mut m = FT_RENDER_MODE_NORMAL;
            /* TTF_USE_SDF */
            if (want & CACHED_COLOR) != 0 && font.render_sdf {
                m = FT_RENDER_MODE_SDF;
            }
            if want & CACHED_LCD != 0 {
                m = FT_RENDER_MODE_LCD;
            }
            ft_render_mode = m;
        }

        // Subpixel translation, flush previous datas
        if want & CACHED_SUBPIX != 0 {
            flush_glyph_image(&mut cached.pixmap);
            ft_outline_translate(&mut font.face.glyph.outline, translation as FtPos, 0);
            cached.u1 = translation;
        }

        // Handle the italic style, only for scalable fonts
        if ttf_handle_style_italic(font) && slot_format == FT_GLYPH_FORMAT_OUTLINE {
            let shear = FtMatrix {
                xx: 1 << 16,
                xy: GLYPH_ITALICS,
                yx: 0,
                yy: 1 << 16,
            };
            ft_outline_transform(&mut font.face.glyph.outline, &shear);
        }

        let (src_left, src_top);

        // Render as outline
        if (font.outline > 0 && slot_format == FT_GLYPH_FORMAT_OUTLINE)
            || slot_format == FT_GLYPH_FORMAT_BITMAP
        {
            let g = ft_get_glyph(&mut font.face.glyph)
                .map_err(|e| ttf_set_ft_error("FT_Get_Glyph() failed", e))?;
            let mut g = Some(g);

            if font.outline > 0 {
                if let Some(stroker) = font.stroker.as_mut() {
                    let _ =
                        ft_glyph_stroke(&mut g, stroker, true /* delete the original glyph */);
                }
            }

            // Render the glyph
            let mut gl = g.unwrap();
            if let Err(e) = ft_glyph_to_bitmap(&mut gl, ft_render_mode, None, true) {
                ft_done_glyph(gl);
                return Err(ttf_set_ft_error("FT_Glyph_To_Bitmap() failed", e));
            }

            // Access bitmap content by typecasting
            let FtGlyph::Bitmap(bitmap_glyph) = &gl else {
                unreachable!()
            };

            // Get new metrics, from bitmap
            src_left = bitmap_glyph.left;
            src_top = bitmap_glyph.top;
            glyph = Some(gl);
        } else {
            // Render the glyph
            if ft_render_glyph(&mut font.face, ft_render_mode).is_err() {
                // Don't fail entirely, just use a 0x0 sized bitmap
                //return TTF_SetFTError("FT_Render_Glyph() failed", error);
            }

            // Access bitmap from slot

            // Get new metrics, from slot
            src_left = font.face.glyph.bitmap_left;
            src_top = font.face.glyph.bitmap_top;
        }

        let src: &FtBitmap = match &glyph {
            Some(FtGlyph::Bitmap(b)) => &b.bitmap,
            _ => &font.face.glyph.bitmap,
        };

        let dst = if mono {
            &mut cached.bitmap
        } else {
            &mut cached.pixmap
        };
        dst.left = src_left;
        dst.top = src_top;

        // Common metrics
        dst.width = src.width as i32;
        dst.rows = src.rows as i32;
        dst.buffer = Vec::new();

        /* FT can make small size glyph of 'width == 0', and 'rows != 0'.
         * Make sure 'rows' is also 0, so it doesn't break USE_DUFFS_LOOP */
        if dst.width == 0 {
            dst.rows = 0;
        }

        /* Some robustess: loading an SVG emoji (FT_LOAD_COLOR), and rendering it a solid (FT_RENDER_MODE_MONO)
         * makes previous FT_Render_Glyph() failed */
        if src.buffer.is_empty() {
            dst.width = 0;
            dst.rows = 0;
        }

        // Adjust for bold text
        if font.style & STYLE_BOLD != 0 {
            dst.width = dst.width.wrapping_add(font.glyph_overhang);
        }

        // Compute pitch: glyph is padded right to be able to read an 'aligned' size expanding on the right
        dst.pitch = dst.width.wrapping_add(alignment);
        /* TTF_USE_COLOR */
        if src.pixel_mode == FT_PIXEL_MODE_BGRA && (want & CACHED_COLOR) != 0 {
            dst.pitch = dst.pitch.wrapping_add(3 * dst.width);
        }
        if src.pixel_mode == FT_PIXEL_MODE_LCD {
            dst.pitch = dst.pitch.wrapping_add(3 * dst.width);
        }

        if dst.rows != 0 {
            /* Glyph buffer is NOT aligned,
             * Extra width so it can read an 'aligned' size expanding on the left */
            let total = (alignment as i64).checked_add(
                (dst.pitch as i64)
                    .checked_mul(dst.rows as i64)
                    .unwrap_or(-1),
            );
            let total = match total {
                Some(t) if t >= 0 => t as usize,
                _ => return Err(Error::out_of_memory()),
            };
            let mut buffer: Vec<u8> = Vec::new();
            if buffer.try_reserve_exact(total).is_err() {
                return Err(Error::out_of_memory());
            }

            // Memset
            buffer.resize(total, 0);

            // Shift, so that the glyph is decoded centered
            let base = alignment as isize;

            /* FT_Render_Glyph() and .fon fonts always generate a two-color (black and white)
             * glyphslot surface, even when rendered in FT_RENDER_MODE_NORMAL. */
            /* FT_IS_SCALABLE() means that the face contains outline glyphs, but does not imply
             * that outline is rendered as 8-bit grayscale, because embedded bitmap/graymap is
             * preferred (see FT_LOAD_DEFAULT section of FreeType2 API Reference).
             * FT_Render_Glyph() canreturn two-color bitmap or 4/16/256 color graymap
             * according to the format of embedded bitmap/graymap. */
            for i in 0..src.rows as isize {
                let mut srcp = i * src.pitch as isize;
                let mut dstp = (base + i * dst.pitch as isize) as usize;
                let (mut quotient, remainder);

                // Decode exactly the needed size from src->width
                if src.pixel_mode == FT_PIXEL_MODE_MONO {
                    quotient = src.width / 8;
                    remainder = src.width & 0x7;
                } else if src.pixel_mode == FT_PIXEL_MODE_GRAY2 {
                    quotient = src.width / 4;
                    remainder = src.width & 0x3;
                } else if src.pixel_mode == FT_PIXEL_MODE_GRAY4 {
                    quotient = src.width / 2;
                    remainder = src.width & 0x1;
                } else if src.pixel_mode == FT_PIXEL_MODE_BGRA {
                    /* TTF_USE_COLOR */
                    quotient = src.width;
                    remainder = 0;
                } else if src.pixel_mode == FT_PIXEL_MODE_LCD {
                    quotient = src.width / 3;
                    remainder = 0;
                } else {
                    quotient = src.width;
                    remainder = 0;
                }

                let sbuf = &src.buffer;
                let mut next = || {
                    let c = src_byte(sbuf, srcp);
                    srcp += 1;
                    c
                };

                // FT_RENDER_MODE_MONO and src->pixel_mode MONO
                macro_rules! mono_mono {
                    ($k_max:expr) => {
                        if $k_max != 0 {
                            let mut c = next();
                            for _ in 0..$k_max {
                                buffer[dstp] = (c & 0x80) >> 7;
                                dstp += 1;
                                c <<= 1;
                            }
                        }
                    };
                }

                // FT_RENDER_MODE_MONO and src->pixel_mode GRAY2
                macro_rules! mono_gray2 {
                    ($k_max:expr) => {
                        if $k_max != 0 {
                            let mut c = next();
                            for _ in 0..$k_max {
                                buffer[dstp] = if ((c & 0xA0) >> 6) >= 0x2 { 1 } else { 0 };
                                dstp += 1;
                                c <<= 2;
                            }
                        }
                    };
                }

                // FT_RENDER_MODE_MONO and src->pixel_mode GRAY4
                macro_rules! mono_gray4 {
                    ($k_max:expr) => {
                        if $k_max != 0 {
                            let mut c = next();
                            for _ in 0..$k_max {
                                buffer[dstp] = if ((c & 0xF0) >> 4) >= 0x8 { 1 } else { 0 };
                                dstp += 1;
                                c <<= 4;
                            }
                        }
                    };
                }

                // FT_RENDER_MODE_NORMAL and src->pixel_mode MONO
                macro_rules! normal_mono {
                    ($k_max:expr) => {
                        if $k_max != 0 {
                            let mut c = next();
                            for _ in 0..$k_max {
                                if ((c & 0x80) >> 7) != 0 {
                                    buffer[dstp] = (NUM_GRAYS - 1) as u8;
                                } else {
                                    buffer[dstp] = 0x00;
                                }
                                dstp += 1;
                                c <<= 1;
                            }
                        }
                    };
                }

                // FT_RENDER_MODE_NORMAL and src->pixel_mode GRAY2
                macro_rules! normal_gray2 {
                    ($k_max:expr) => {
                        if $k_max != 0 {
                            let mut c = next();
                            for _ in 0..$k_max {
                                if ((c & 0xA0) >> 6) != 0 {
                                    buffer[dstp] =
                                        (NUM_GRAYS * ((c as i32 & 0xA0) >> 6) / 3 - 1) as u8;
                                } else {
                                    buffer[dstp] = 0x00;
                                }
                                dstp += 1;
                                c <<= 2;
                            }
                        }
                    };
                }

                // FT_RENDER_MODE_NORMAL and src->pixel_mode GRAY4
                macro_rules! normal_gray4 {
                    ($k_max:expr) => {
                        if $k_max != 0 {
                            let mut c = next();
                            for _ in 0..$k_max {
                                if ((c & 0xF0) >> 4) != 0 {
                                    buffer[dstp] =
                                        (NUM_GRAYS * ((c as i32 & 0xF0) >> 4) / 15 - 1) as u8;
                                } else {
                                    buffer[dstp] = 0x00;
                                }
                                dstp += 1;
                                c <<= 4;
                            }
                        }
                    };
                }

                if mono {
                    if src.pixel_mode == FT_PIXEL_MODE_MONO {
                        while quotient > 0 {
                            quotient -= 1;
                            mono_mono!(8);
                        }
                        mono_mono!(remainder);
                    } else if src.pixel_mode == FT_PIXEL_MODE_GRAY2 {
                        while quotient > 0 {
                            quotient -= 1;
                            mono_gray2!(4);
                        }
                        mono_gray2!(remainder);
                    } else if src.pixel_mode == FT_PIXEL_MODE_GRAY4 {
                        while quotient > 0 {
                            quotient -= 1;
                            mono_gray4!(2);
                        }
                        mono_gray4!(remainder);
                    } else if src.pixel_mode == FT_PIXEL_MODE_BGRA {
                        while quotient > 0 {
                            quotient -= 1;
                            let b = next();
                            let g = next();
                            let r = next();
                            let a = next();
                            let mut c: u8 = 0;
                            if a != 0 {
                                // Technically we should do this in linear colorspace
                                // Y = 0.2126 * R + 0.7152 * G + 0.0722 * B
                                c = 255u8.wrapping_sub(
                                    ((r as i32 * 54) / 255
                                        + (g as i32 * 182) / 255
                                        + (b as i32 * 18) / 255)
                                        as u8,
                                );
                            }
                            buffer[dstp] = if c >= 0x80 { 1 } else { 0 };
                            dstp += 1;
                        }
                    } else {
                        while quotient > 0 {
                            quotient -= 1;
                            let c = next();
                            buffer[dstp] = if c >= 0x80 { 1 } else { 0 };
                            dstp += 1;
                        }
                    }
                } else if src.pixel_mode == FT_PIXEL_MODE_MONO {
                    /* This special case wouldn't be here if the FT_Render_Glyph()
                     * function wasn't buggy when it tried to render a .fon font with 256
                     * shades of gray.  Instead, it returns a black and white surface
                     * and we have to translate it back to a 256 gray shaded surface. */
                    while quotient > 0 {
                        quotient -= 1;
                        normal_mono!(8);
                    }
                    normal_mono!(remainder);
                } else if src.pixel_mode == FT_PIXEL_MODE_GRAY2 {
                    while quotient > 0 {
                        quotient -= 1;
                        normal_gray2!(4);
                    }
                    normal_gray2!(remainder);
                } else if src.pixel_mode == FT_PIXEL_MODE_GRAY4 {
                    while quotient > 0 {
                        quotient -= 1;
                        normal_gray4!(2);
                    }
                    normal_gray4!(remainder);
                } else if src.pixel_mode == FT_PIXEL_MODE_BGRA {
                    /* TTF_USE_COLOR */
                    if want & CACHED_COLOR != 0 {
                        for k in 0..4 * src.width as usize {
                            buffer[dstp + k] = src_byte(sbuf, srcp + k as isize);
                        }
                    } else {
                        // Convert to grayscale
                        while quotient > 0 {
                            quotient -= 1;
                            let b = next();
                            let g = next();
                            let r = next();
                            let a = next();
                            if a != 0 {
                                // Technically we should do this in linear colorspace
                                // Y = 0.2126 * R + 0.7152 * G + 0.0722 * B
                                buffer[dstp] = 255u8.wrapping_sub(
                                    ((r as i32 * 54) / 255
                                        + (g as i32 * 182) / 255
                                        + (b as i32 * 18) / 255)
                                        as u8,
                                );
                            } else {
                                buffer[dstp] = 0;
                            }
                            dstp += 1;
                        }
                    }
                } else if src.pixel_mode == FT_PIXEL_MODE_LCD {
                    while quotient > 0 {
                        quotient -= 1;
                        let alpha: u8 = 0;
                        let r = next();
                        let g = next();
                        let b = next();
                        buffer[dstp] = b;
                        buffer[dstp + 1] = g;
                        buffer[dstp + 2] = r;
                        buffer[dstp + 3] = alpha;
                        dstp += 4;
                    }
                } else {
                    for k in 0..src.width as usize {
                        buffer[dstp + k] = src_byte(sbuf, srcp + k as isize);
                    }
                }
            }

            // Handle the bold style
            if font.style & STYLE_BOLD != 0 {
                // The pixmap is a little hard, we have to add and clamp
                let mut row = dst.rows - 1;
                while row >= 0 {
                    let pixmap = (base + row as isize * dst.pitch as isize) as usize;
                    // Minimal memset
                    // SDL_memset(pixmap + dst->width - font->glyph_overhang, 0, font->glyph_overhang);
                    for _offset in 1..=font.glyph_overhang {
                        let mut col = dst.width - 1;
                        while col > 0 {
                            let (c, p) = (pixmap + col as usize, pixmap + col as usize - 1);
                            if mono {
                                buffer[c] |= buffer[p];
                            } else {
                                let mut pixel = buffer[c] as i32 + buffer[p] as i32;
                                if pixel > NUM_GRAYS - 1 {
                                    pixel = NUM_GRAYS - 1;
                                }
                                buffer[c] = pixel as u8;
                            }
                            col -= 1;
                        }
                    }
                    row -= 1;
                }
            }

            // Shift back
            dst.buffer = buffer;
        }

        /* TTF_USE_COLOR */
        if src.pixel_mode == FT_PIXEL_MODE_BGRA && (want & CACHED_COLOR) != 0 {
            dst.is_color = 1;
        } else {
            dst.is_color = 0;
        }
        let is_color = dst.is_color;
        let src_pixel_mode = src.pixel_mode;

        // Mark that we rendered this format
        if mono {
            cached.stored |= CACHED_BITMAP;
        } else if src_pixel_mode == FT_PIXEL_MODE_LCD {
            cached.stored |= CACHED_LCD;
        } else if want & CACHED_COLOR != 0 {
            /* TTF_USE_COLOR */
            cached.stored |= CACHED_COLOR;
            /* Most of the time, glyphs loaded with FT_LOAD_COLOR are non colored, so the cache is
            also suitable for Shaded rendering (eg, loaded without FT_LOAD_COLOR) */
            if is_color == 0 {
                cached.stored |= CACHED_PIXMAP;
            }
        } else {
            cached.stored |= CACHED_PIXMAP;
            // If font has no color information, Shaded/Pixmap cache is also suitable for Blend/Color
            if !ft_has_color(&font.face) {
                cached.stored |= CACHED_COLOR;
            }
        }

        // Free outlined glyph
        if let Some(glyph) = glyph {
            ft_done_glyph(glyph);
        }
    }

    // We're done, this glyph is cached since 'stored' is not 0
    Ok(())
}

/// `Find_GlyphByIndex`: the glyph and image are in `font.glyphs[idx]`
/// afterwards (the pixmap, or the bitmap with `want_bitmap`).
pub(crate) fn find_glyph_by_index(
    font: &mut FontData,
    idx: FtUInt,
    want_bitmap: i32,
    want_pixmap: i32,
    want_color: i32,
    want_lcd: i32,
    want_subpixel: i32,
    translation: i32,
) -> Result<()> {
    let mut glyph = match font.glyphs.remove(&idx) {
        Some(g) => g,
        None => {
            let mut g = CGlyph::default();
            if font.glyphs.try_reserve(1).is_err() {
                return Err(Error::out_of_memory());
            }
            g.index = idx;
            g
        }
    };

    let r = find_glyph_by_index_in(
        font,
        &mut glyph,
        want_bitmap,
        want_pixmap,
        want_color,
        want_lcd,
        want_subpixel,
        translation,
    );

    font.glyphs.insert(idx, glyph);
    r
}

#[allow(clippy::too_many_arguments)]
fn find_glyph_by_index_in(
    font: &mut FontData,
    glyph: &mut CGlyph,
    want_bitmap: i32,
    want_pixmap: i32,
    want_color: i32,
    want_lcd: i32,
    want_subpixel: i32,
    translation: i32,
) -> Result<()> {
    if want_subpixel != 0 {
        /* Not a real cache, but if it always advances by integer pixels (eg translation 0 or same as previous),
         * this allows to render as fast as normal mode. */
        let mut want =
            CACHED_METRICS | want_bitmap | want_pixmap | want_color | want_lcd | want_subpixel;

        if glyph.u1 == translation {
            want &= !CACHED_SUBPIX;
        }

        if (glyph.stored & want) == want {
            return Ok(());
        }

        if (want_color != 0 || want_pixmap != 0 || want_lcd != 0)
            && glyph.stored & (CACHED_COLOR | CACHED_PIXMAP | CACHED_LCD) != 0
        {
            flush_glyph(glyph);
        }

        load_glyph(font, glyph, want, translation)
    } else {
        let want = CACHED_METRICS | want_bitmap | want_pixmap | want_color | want_lcd;

        // Faster check as it gets inlined
        if want_pixmap != 0 {
            if glyph.stored & CACHED_PIXMAP != 0 {
                return Ok(());
            }
        } else if want_bitmap != 0 {
            if glyph.stored & CACHED_BITMAP != 0 {
                return Ok(());
            }
        } else if want_color != 0 {
            if glyph.stored & CACHED_COLOR != 0 {
                return Ok(());
            }
        } else if want_lcd != 0 {
            if glyph.stored & CACHED_LCD != 0 {
                return Ok(());
            }
        } else {
            // Get metrics
            if glyph.stored != 0 {
                return Ok(());
            }
        }

        /* Cache cannot contain both PIXMAP and COLOR (unless COLOR is actually not colored) and LCD
        So, if it's already used, clear it */
        if (want_color != 0 || want_pixmap != 0 || want_lcd != 0)
            && glyph.stored & (CACHED_COLOR | CACHED_PIXMAP | CACHED_LCD) != 0
        {
            flush_glyph(glyph);
        }

        load_glyph(font, glyph, want, 0)
    }
}

/// `get_char_index`
pub(crate) fn get_char_index(font: &mut FontData, ch: u32) -> FtUInt {
    match font.glyph_indices.get(&ch) {
        Some(&idx) => idx,
        None => {
            let idx = ft_get_char_index(&font.face, ch as FtULong);
            font.glyph_indices.insert(ch, idx);
            idx
        }
    }
}

/// `get_char_index_fallback`: returns the index and the font it is in
/// (`glyph_font`).
fn get_char_index_fallback(
    this: &Weak<RefCell<FontData>>,
    font: &mut FontData,
    ch: u32,
    initial_font: Option<&Weak<RefCell<FontData>>>,
) -> (FtUInt, Option<Weak<RefCell<FontData>>>) {
    let initial_font = match initial_font {
        None => this,
        Some(i) => {
            if Weak::ptr_eq(this, i) {
                // font fallback loop
                return (0, None);
            }
            i
        }
    };

    let mut idx = get_char_index(font, ch);
    let mut glyph_font = None;
    if idx > 0 {
        glyph_font = Some(this.clone());
    } else {
        for list in font.fallbacks.clone() {
            if Weak::ptr_eq(&list, initial_font) {
                /* (the recursion returns 0 for the loop) */
                continue;
            }
            let Some(fb) = list.upgrade() else {
                continue;
            };
            let Ok(mut fbd) = fb.try_borrow_mut() else {
                continue;
            };
            let (i, gf) = get_char_index_fallback(&list, &mut fbd, ch, Some(initial_font));
            idx = i;
            if idx > 0 {
                glyph_font = gf;
                break;
            }
        }
    }
    (idx, glyph_font)
}

/// `Find_GlyphMetrics`: returns the glyph's font and index.
fn find_glyph_metrics(
    this: &Weak<RefCell<FontData>>,
    font: &mut FontData,
    ch: u32,
    allow_fallback: bool,
) -> Result<(Weak<RefCell<FontData>>, FtUInt)> {
    let (glyph_font, idx) = if allow_fallback {
        let (idx, gf) = get_char_index_fallback(this, font, ch, None);
        (gf.unwrap_or_else(|| this.clone()), idx)
    } else {
        (this.clone(), get_char_index(font, ch))
    };

    if Weak::ptr_eq(&glyph_font, this) {
        find_glyph_by_index(font, idx, 0, 0, 0, 0, 0, 0)?;
    } else if let Some(gf) = glyph_font.upgrade() {
        find_glyph_by_index(&mut gf.borrow_mut(), idx, 0, 0, 0, 0, 0, 0)?;
    }
    Ok((glyph_font, idx))
}

/// Run `f` on the data of the font `which`, which is `this` (borrowed as
/// `font`) or another font.
pub(crate) fn with_font_data<R>(
    this: &Weak<RefCell<FontData>>,
    font: &mut FontData,
    which: &Weak<RefCell<FontData>>,
    f: impl FnOnce(&mut FontData) -> R,
) -> Option<R> {
    if Weak::ptr_eq(this, which) {
        Some(f(font))
    } else {
        let other = which.upgrade()?;
        let mut other = other.try_borrow_mut().ok()?;
        Some(f(&mut other))
    }
}

/// The state of a C string walk: SDL_StepUTF8() over `text[pos..]` with
/// `len` bytes left (bytes past the slice read as the NUL terminator).
pub(crate) fn step_utf8(text: &[u8], pos: &mut usize, len: &mut usize) -> u32 {
    let start = (*pos).min(text.len());
    let avail = (text.len() - start).min(*len);
    let mut s = &text[start..start + avail];
    let before = s.len();
    let c = sdl3::stdlib::string::step_utf8(&mut s);
    let advanced = before - s.len();
    *pos += advanced;
    *len -= advanced;
    c
}

/// `SDL_StepUTF8( &text, NULL )`: at most 4 bytes.
pub(crate) fn step_utf8_unbounded(text: &[u8], pos: &mut usize) -> u32 {
    let mut len = 4;
    step_utf8(text, pos, &mut len)
}

/// `CollectGlyphsFromFont`
fn collect_glyphs_from_font(
    this: &Weak<RefCell<FontData>>,
    font: &mut FontData,
    text: &[u8],
    length: usize,
    direction: Direction,
    script: u32,
    positions: &mut GlyphPositions,
) -> Result<()> {
    /* TTF_USE_HARFBUZZ */
    // Create a buffer for harfbuzz to use
    let mut hb_buffer = HbBuffer::new();

    // Set global configuration
    hb_buffer.set_language(font.hb_language);
    hb_buffer.set_direction(direction as HbDirection);
    hb_buffer.set_script(hb_script_from_iso15924_tag(script));

    // Layout the text
    hb_buffer.add_utf8(&text[..length.min(text.len())], 0, -1);
    hb_buffer.guess_segment_properties();

    let userfeatures = [HbFeature {
        tag: hb_tag(b'k', b'e', b'r', b'n'),
        value: font.enable_kerning as u32,
        start: HB_FEATURE_GLOBAL_START,
        end: HB_FEATURE_GLOBAL_END,
    }];

    {
        let mut hb_font = HbFont::new(&mut font.hb_font, Some(&mut font.face));
        hb_shape(&mut hb_font, &mut hb_buffer, &userfeatures);
    }

    // Get the result
    let hb_glyph_info: Vec<_> = hb_buffer.get_glyph_infos().to_vec();
    let hb_glyph_position: Vec<_> = hb_buffer.get_glyph_positions().to_vec();

    // Adjust for bold text
    let mut advance_if_bold = 0;
    if ttf_handle_style_bold(font) {
        advance_if_bold = f26dot6(font.glyph_overhang);
    }

    // Realloc, if needed
    let glyph_count = hb_glyph_info.len();
    positions.pos.clear();
    if positions.pos.try_reserve_exact(glyph_count).is_err() {
        return Err(Error::out_of_memory());
    }

    for i in 0..glyph_count {
        let index = hb_glyph_info[i].codepoint;
        positions.pos.push(GlyphPosition {
            font: this.clone(),
            index,
            x_advance: hb_glyph_position[i].x_advance.wrapping_add(advance_if_bold),
            y_advance: hb_glyph_position[i].y_advance,
            x_offset: hb_glyph_position[i].x_offset,
            y_offset: hb_glyph_position[i].y_offset,
            offset: hb_glyph_info[i].cluster as i32,
            x: 0,
            y: 0,
        });
        if find_glyph_by_index(font, index, 0, 0, 0, 0, 0, 0).is_err() {
            return Err(Error::new(format!("Couldn't find glyph {index} in font")));
        }
    }

    Ok(())
}

impl CGlyph {
    /// The glyph without its images (the metrics the layout reads).
    pub(crate) fn clone_metrics(&self) -> CGlyph {
        CGlyph {
            stored: self.stored,
            index: self.index,
            bitmap: TtfImage::default(),
            pixmap: TtfImage::default(),
            sz_left: self.sz_left,
            sz_top: self.sz_top,
            sz_width: self.sz_width,
            sz_rows: self.sz_rows,
            advance: self.advance,
            u0: self.u0,
            u1: self.u1,
        }
    }
}

/// `ReplaceGlyphPositions`
fn replace_glyph_positions(
    positions: &mut GlyphPositions,
    start: usize,
    length: usize,
    replacement: &GlyphPositions,
) -> Result<()> {
    let initial_offset = std::cmp::min(
        positions.pos[start].offset,
        positions.pos[start + length - 1].offset,
    );

    let reserve = replacement.pos.len().saturating_sub(length);
    if positions.pos.try_reserve(reserve).is_err() {
        return Err(Error::out_of_memory());
    }

    positions
        .pos
        .splice(start..start + length, replacement.pos.iter().cloned());

    for i in 0..replacement.pos.len() {
        positions.pos[start + i].offset += initial_offset;
    }
    Ok(())
}

/// `FillFallbackSpan`
#[allow(clippy::too_many_arguments)]
fn fill_fallback_span(
    fallback: &Weak<RefCell<FontData>>,
    text: &[u8],
    length: usize,
    direction: Direction,
    script: u32,
    positions: &mut GlyphPositions,
    initial_font: &Weak<RefCell<FontData>>,
    start: usize,
    stop: usize,
) -> usize {
    // Set the starting offset
    let mut min_offset = positions.pos[start].offset;
    let mut max_offset = min_offset;

    for i in start..stop {
        if positions.pos[i].offset < min_offset {
            min_offset = positions.pos[i].offset;
        }
        if positions.pos[i].offset > max_offset {
            max_offset = positions.pos[i].offset;
        }
    }
    if stop < positions.pos.len() && positions.pos[stop].offset > max_offset {
        max_offset = positions.pos[stop].offset;
    } else {
        let mut last_text = max_offset as usize;
        step_utf8_unbounded(text, &mut last_text);
        max_offset = last_text as i32;
    }
    if max_offset > length as i32 {
        max_offset = length as i32;
    }

    let mut span = GlyphPositions::default();
    let span_length = (max_offset - min_offset) as usize;
    let sub = &text[(min_offset as usize).min(text.len())..];
    let _ = collect_glyphs_with_fallbacks_weak(
        fallback,
        sub,
        span_length,
        direction,
        script,
        &mut span,
        Some(initial_font),
    );
    if !span.pos.is_empty() {
        let _ = replace_glyph_positions(positions, start, stop - start, &span);
    }
    span.pos.len()
}

/// `CollectGlyphsWithFallbacks` for a font that is not borrowed.
fn collect_glyphs_with_fallbacks_weak(
    font: &Weak<RefCell<FontData>>,
    text: &[u8],
    length: usize,
    direction: Direction,
    script: u32,
    positions: &mut GlyphPositions,
    initial_font: Option<&Weak<RefCell<FontData>>>,
) -> Result<()> {
    if let Some(i) = initial_font {
        if Weak::ptr_eq(font, i) {
            // font fallback loop
            return Ok(());
        }
    }
    let Some(rc) = font.upgrade() else {
        return Ok(());
    };
    let Ok(mut fd) = rc.try_borrow_mut() else {
        return Ok(());
    };
    collect_glyphs_with_fallbacks(
        font,
        &mut fd,
        text,
        length,
        direction,
        script,
        positions,
        initial_font,
    )
}

/// `CollectGlyphsWithFallbacks`
#[allow(clippy::too_many_arguments)]
fn collect_glyphs_with_fallbacks(
    this: &Weak<RefCell<FontData>>,
    font: &mut FontData,
    text: &[u8],
    length: usize,
    direction: Direction,
    script: u32,
    positions: &mut GlyphPositions,
    initial_font: Option<&Weak<RefCell<FontData>>>,
) -> Result<()> {
    let initial_font = match initial_font {
        None => this,
        Some(i) => {
            if Weak::ptr_eq(this, i) {
                // font fallback loop
                return Ok(());
            }
            i
        }
    };

    collect_glyphs_from_font(this, font, text, length, direction, script, positions)?;

    // Create spans of missing characters and fill them in from fallback fonts
    let mut complete = false;
    let fallbacks = font.fallbacks.clone();
    let mut fallback = fallbacks.iter();
    let mut current = fallback.next();
    while !complete {
        let Some(fb) = current else {
            break;
        };
        complete = true;
        let mut start: i32 = -1;
        let mut i: i32 = 0;
        while i < positions.pos.len() as i32 {
            let pos = &positions.pos[i as usize];
            if pos.index == 0 {
                complete = false;
                if start < 0 {
                    start = i;
                }
            } else if start >= 0 {
                // Fill in this span with the fallback font
                let replaced = fill_fallback_span(
                    fb,
                    text,
                    length,
                    direction,
                    script,
                    positions,
                    initial_font,
                    start as usize,
                    i as usize,
                );
                if replaced > 0 {
                    i = start + replaced as i32;
                }
                start = -1;
            }
            i += 1;
        }
        if start >= 0 {
            // Fill in this span with the fallback font
            let len = positions.pos.len();
            fill_fallback_span(
                fb,
                text,
                length,
                direction,
                script,
                positions,
                initial_font,
                start as usize,
                len,
            );
        }
        current = fallback.next();
    }

    Ok(())
}

/// `CollectGlyphs`
fn collect_glyphs(
    this: &Weak<RefCell<FontData>>,
    font: &mut FontData,
    text: &[u8],
    length: usize,
    direction: Direction,
    script: u32,
    positions: &mut GlyphPositions,
) -> Result<()> {
    collect_glyphs_with_fallbacks(this, font, text, length, direction, script, positions, None)?;

    // Calculate the glyph positions and number of clusters
    let mut x: i32 = 0;
    let mut y: i32 = 0;
    let mut last_offset = -1;
    positions.num_clusters = 0;
    for i in 0..positions.pos.len() {
        let pos = &mut positions.pos[i];

        // Missing characters use the tofu from the initial font
        if pos.index == 0 && !Weak::ptr_eq(&pos.font, this) {
            pos.font = this.clone();
            if find_glyph_by_index(font, pos.index, 0, 0, 0, 0, 0, 0).is_err() {
                return Err(Error::new(format!(
                    "Couldn't find glyph {} in font",
                    pos.index
                )));
            }
            pos.x_advance = font.glyphs[&pos.index].advance;
            pos.y_advance = 0;
            pos.x_offset = 0;
            pos.y_offset = 0;
        }

        // Compute positions
        let ascent = if Weak::ptr_eq(&pos.font, this) {
            font.ascent
        } else {
            pos.font
                .upgrade()
                .map_or(0, |f| f.try_borrow().map_or(0, |f| f.ascent))
        };
        pos.x = x.wrapping_add(pos.x_offset);
        pos.y = y.wrapping_add(f26dot6(ascent)).wrapping_sub(pos.y_offset);
        x = x.wrapping_add(pos.x_advance);
        y = y.wrapping_add(pos.y_advance);
        /* (TTF_USE_HARFBUZZ: no rounding) */

        // Save the number of clusters we've seen
        if pos.offset != last_offset {
            positions.num_clusters += 1;
            last_offset = pos.offset;
        }
    }
    positions.width26dot6 = x;
    positions.height26dot6 = y;

    Ok(())
}

/// `GetCachedGlyphPositions`: sets `font.positions` (`None` on failure,
/// as C's NULL, with no error set for a failed allocation).
pub(crate) fn get_cached_glyph_positions(
    this: &Weak<RefCell<FontData>>,
    font: &mut FontData,
    text: &[u8],
    length: usize,
    direction: Direction,
    script: u32,
) -> Result<()> {
    font.positions = None;
    for i in 0..font.cached_positions.len() {
        let cached = &font.cached_positions[i];
        let ctext = cached.text.as_deref().unwrap_or(&[]);
        if direction == cached.direction
            && script == cached.script
            && length == cached.length
            && text.get(..length).unwrap_or(text) == &ctext[..length.min(ctext.len())]
        {
            font.positions = Some(i);
            break;
        }
    }

    if font.positions.is_none() {
        // We could do something fancy like an LRU cache, but it's probably not worth the complexity
        let slot = font.next_cached_positions;
        font.next_cached_positions = (font.next_cached_positions + 1) % font.cached_positions.len();
        font.positions = Some(slot);

        let cached = &mut font.cached_positions[slot];
        cached.direction = direction;
        cached.script = script;
        let mut t = text[..length.min(text.len())].to_vec();
        t.resize(length, 0);
        cached.text = Some(t);
        cached.length = length;

        let mut positions = std::mem::take(&mut font.cached_positions[slot].positions);
        let r = collect_glyphs(this, font, text, length, direction, script, &mut positions);
        font.cached_positions[slot].positions = positions;
        if let Err(e) = r {
            font.cached_positions[slot].length = 0;
            font.positions = None;
            return Err(e);
        }
    }
    Ok(())
}

/// The results of `TTF_Size_Internal`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SizeResult {
    pub w: i32,
    pub h: i32,
    pub xstart: i32,
    pub ystart: i32,
    pub measured_width: i32,
    pub measured_length: usize,
}

/// `TTF_Size_Internal`
#[allow(clippy::too_many_arguments)]
pub(crate) fn ttf_size_internal(
    this: &Weak<RefCell<FontData>>,
    font: &mut FontData,
    text: &[u8],
    length: usize,
    direction: Direction,
    script: u32,
    measure_width: bool,
    max_width: i32,
    include_spread: bool,
) -> Result<SizeResult> {
    let mut x: i32 = 0;
    let mut r = SizeResult::default();

    ttf_check_initialized()?;

    r.measured_length = length;

    let spread_adjustment = if font.render_sdf && !include_spread {
        DEFAULT_SDF_SPREAD
    } else {
        0
    };

    get_cached_glyph_positions(this, font, text, length, direction, script)?;
    let slot = font.positions.unwrap();

    let mut minx = 0;
    let mut maxx = 0;
    let mut miny = 0;
    let mut maxy = font.height;

    if font.render_sdf {
        // The glyph top and left positions jitter around when doing SDF rendering,
        // but the overall bounding box height is stable.
        //
        // This is almost certainly not the right fix, but it seems to work.
        miny = i32::MAX;
    }

    let positions = font.cached_positions[slot].positions.clone();
    let metrics = |font: &mut FontData, pos: &GlyphPosition| -> CGlyph {
        with_font_data(this, font, &pos.font, |f| {
            f.glyphs.get(&pos.index).map(|g| g.clone_metrics())
        })
        .flatten()
        .unwrap_or_default()
    };

    if !positions.pos.is_empty() {
        if positions.pos[0].offset == 0 {
            // Left to right layout
            for pos in positions.pos.iter() {
                let glyph = metrics(font, pos);

                // Compute provisional global bounding box
                let pos_x = ft_floor(pos.x as i64) + glyph.sz_left + spread_adjustment;
                let pos_y = ft_floor(pos.y as i64) - glyph.sz_top + spread_adjustment;

                minx = std::cmp::min(minx, pos_x);
                maxx = std::cmp::max(
                    maxx,
                    pos_x.wrapping_add(glyph.sz_width) - 2 * spread_adjustment,
                );
                miny = std::cmp::min(miny, pos_y);
                maxy = std::cmp::max(
                    maxy,
                    pos_y.wrapping_add(glyph.sz_rows) - 2 * spread_adjustment,
                );

                x = x.wrapping_add(pos.x_advance);
                /* (TTF_USE_HARFBUZZ: no rounding) */
                // Measurement mode
                if measure_width {
                    let mut cw = std::cmp::max(maxx, ft_floor(x as i64)) - minx;
                    cw += 2 * font.outline;
                    if max_width == 0 || cw <= max_width {
                        r.measured_width = cw;
                    } else {
                        r.measured_length = pos.offset as usize;
                        break;
                    }
                }
            }
        } else {
            // Right to left layout
            x = positions.width26dot6;
            minx = ft_ceil(positions.width26dot6 as i64);
            for pos in positions.pos.iter().rev() {
                let glyph = metrics(font, pos);

                // Compute provisional global bounding box
                let pos_x = ft_floor(pos.x as i64) + glyph.sz_left + spread_adjustment;
                let pos_y = ft_floor(pos.y as i64) - glyph.sz_top + spread_adjustment;

                minx = std::cmp::min(minx, pos_x);
                maxx = std::cmp::max(
                    maxx,
                    pos_x.wrapping_add(glyph.sz_width) - 2 * spread_adjustment,
                );
                miny = std::cmp::min(miny, pos_y);
                maxy = std::cmp::max(
                    maxy,
                    pos_y.wrapping_add(glyph.sz_rows) - 2 * spread_adjustment,
                );

                // Measurement mode
                if measure_width {
                    let mut cw = std::cmp::max(maxx, ft_floor(x as i64)) - minx;
                    cw += 2 * font.outline;
                    if max_width == 0 || cw <= max_width {
                        r.measured_width = cw;
                    } else {
                        r.measured_length = pos.offset as usize;
                        break;
                    }
                }

                x = x.wrapping_sub(pos.x_advance);
                /* (TTF_USE_HARFBUZZ: no rounding) */
            }
            if minx > 0 {
                minx = 0;
            }
        }
    }

    // Allows to render a string with only one space (bug 4344).
    maxx = std::cmp::max(maxx, ft_floor(x as i64));

    /* Initial x start position: often 0, except when a glyph would be written at
     * a negative position. In this case an offset is needed for the whole line. */
    r.xstart = if minx < 0 { -minx } else { 0 };
    r.xstart += font.outline;
    if font.render_sdf && include_spread {
        r.xstart += DEFAULT_SDF_SPREAD;
    }

    // Initial y start: compensation for a negative y offset
    r.ystart = if miny < 0 { -miny } else { 0 };
    r.ystart += font.outline;

    // Fill the bounds rectangle
    r.w = maxx.wrapping_sub(minx);
    if r.w != 0 {
        r.w += 2 * font.outline;
    }
    r.h = maxy.wrapping_sub(miny);
    r.h += 2 * font.outline;
    if font.render_sdf && include_spread {
        r.h += 2 * DEFAULT_SDF_SPREAD;
    }
    Ok(r)
}

/// The length C gives a text: `SDL_strlen()` for a 0 length.
pub(crate) fn c_length(text: &[u8], length: usize) -> usize {
    if length == 0 {
        text.iter().position(|&b| b == 0).unwrap_or(text.len())
    } else {
        length
    }
}

/// `TTF_Render_Internal`
fn ttf_render_internal(
    font: &Font,
    text: &[u8],
    fg: Color,
    bg: Color,
    render_mode: RenderMode,
) -> Result<Surface<'static>> {
    ttf_check_initialized()?;

    let this = font.weak();
    let mut fd = font.rc.borrow_mut();
    let font = &mut *fd;

    let length = c_length(text, text.len());
    let mut fg = fg;

    if render_mode == RenderMode::Lcd && !ft_is_scalable(&font.face) {
        return Err(Error::new(
            "LCD rendering is not available for non-scalable font",
        ));
    }

    /* TTF_USE_SDF */
    // Invalid cache if we were using SDF
    if render_mode != RenderMode::Blended && font.render_sdf {
        font.render_sdf = false;
        flush_cache(font);
    }

    // Get the dimensions of the text surface
    let size = match ttf_size_internal(
        &this,
        font,
        text,
        length,
        font.direction,
        font.script,
        false,
        0,
        true,
    ) {
        Ok(s) if s.w != 0 => s,
        _ => return Err(Error::new("Text has zero width")),
    };
    let (width, height, xstart, ystart) = (size.w, size.h, size.xstart, size.ystart);

    // Create surface for rendering
    if fg.a == 0 {
        fg.a = 255;
    }
    let mut color: u32 = 0;
    let textbuf = match render_mode {
        RenderMode::Solid => create_surface_solid(width, height, fg, &mut color),
        RenderMode::Shaded => create_surface_shaded(width, height, fg, bg, &mut color),
        RenderMode::Blended => create_surface_blended(width, height, fg, &mut color),
        RenderMode::Lcd => create_surface_lcd(width, height, fg, bg, &mut color),
    };

    let Some(mut textbuf) = textbuf else {
        return Err(Error::out_of_memory());
    };

    // Render one text line to textbuf at (xstart, ystart)
    render_line(
        render_mode,
        font.render_subpixel,
        &this,
        font,
        &mut textbuf.buf,
        xstart,
        ystart,
        fg,
    )?;

    // Apply underline or strikethrough style, if needed
    if ttf_handle_style_underline(font) {
        draw_line(
            font.direction,
            &mut textbuf.buf,
            0,
            ystart + font.underline_top_row,
            width,
            font.line_thickness,
            color,
            render_mode,
        );
    }

    if ttf_handle_style_strikethrough(font) {
        draw_line(
            font.direction,
            &mut textbuf.buf,
            0,
            ystart + font.strikethrough_top_row,
            width,
            font.line_thickness,
            color,
            render_mode,
        );
    }

    textbuf.into_surface()
}

/// `SDL_UCS4ToUTF8()`, as C writes it (surrogates are encoded as they
/// are).
fn ucs4_to_utf8(ch: u32) -> Vec<u8> {
    let mut ch = ch;
    let mut v = Vec::with_capacity(4);
    if ch <= 0x7F {
        v.push(ch as u8);
    } else if ch <= 0x7FF {
        v.push(0xC0 | ((ch >> 6) & 0x1F) as u8);
        v.push(0x80 | (ch & 0x3F) as u8);
    } else if ch <= 0xFFFF {
        v.push(0xE0 | ((ch >> 12) & 0x0F) as u8);
        v.push(0x80 | ((ch >> 6) & 0x3F) as u8);
        v.push(0x80 | (ch & 0x3F) as u8);
    } else if ch <= 0x10FFFF {
        v.push(0xF0 | ((ch >> 18) & 0x07) as u8);
        v.push(0x80 | ((ch >> 12) & 0x3F) as u8);
        v.push(0x80 | ((ch >> 6) & 0x3F) as u8);
        v.push(0x80 | (ch & 0x3F) as u8);
    } else {
        ch = 0xFFFD;
        v.push(0xE0 | ((ch >> 12) & 0x0F) as u8);
        v.push(0x80 | ((ch >> 6) & 0x3F) as u8);
        v.push(0x80 | (ch & 0x3F) as u8);
    }
    v
}

/// `CharacterIsDelimiter`
pub(crate) fn character_is_delimiter(c: u32) -> bool {
    c == ' ' as u32 || c == '\t' as u32 || c == '\r' as u32 || c == '\n' as u32
}

/// `CharacterIsNewLine`
pub(crate) fn character_is_new_line(c: u32) -> bool {
    c == '\n' as u32
}

/// `TTF_Line` (an offset into the text, and a length)
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TtfLine {
    pub text: usize,
    pub length: usize,
}

/// The byte of a C string at `i` (0 past the end: the NUL terminator).
#[inline]
pub(crate) fn cbyte(text: &[u8], i: usize) -> u8 {
    text.get(i).copied().unwrap_or(0)
}

/// `GetWrappedLines`: returns the lines, `w` and `h`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn get_wrapped_lines(
    this: &Weak<RefCell<FontData>>,
    font: &mut FontData,
    text: &[u8],
    length: usize,
    direction: Direction,
    script: u32,
    xoffset: i32,
    wrap_width: i32,
    trim_whitespace: bool,
    include_spread: bool,
) -> Result<(Vec<TtfLine>, i32, i32)> {
    let mut xoffset = xoffset;
    let mut str_lines: Vec<TtfLine> = Vec::new();

    ttf_check_initialized()?;
    if wrap_width < 0 {
        return Err(Error::invalid_param("wrap_width"));
    }

    let length = c_length(text, length);

    // Get the dimensions of the text surface
    let (mut width, height) = match ttf_size_internal(
        this,
        font,
        text,
        length,
        direction,
        script,
        false,
        0,
        include_spread,
    ) {
        Ok(s) if s.w != 0 => (s.w, s.h),
        _ => return Err(Error::new("Text has zero width")),
    };

    if cbyte(text, 0) != 0 {
        let mut max_num_lines = 0;
        let mut spot = 0usize;
        let mut left = length;

        loop {
            let mut save_text: Option<usize> = None;
            let mut save_length: usize = usize::MAX;

            if str_lines.len() >= max_num_lines {
                if wrap_width == 0 {
                    max_num_lines += 32;
                } else {
                    max_num_lines += (width / wrap_width) as usize + 1;
                }
                if str_lines
                    .try_reserve(max_num_lines - str_lines.len())
                    .is_err()
                {
                    return Err(Error::out_of_memory());
                }
            }

            if trim_whitespace && spot > 0 && cbyte(text, spot - 1) != b'\n' {
                let mut next_spot = spot;
                let mut next_left = left;
                loop {
                    let c = step_utf8(text, &mut next_spot, &mut next_left);
                    if c == 0 || (c != ' ' as u32 && c != '\t' as u32) {
                        break;
                    }
                    spot = next_spot;
                    left = next_left;
                }
            }

            if let Some(last) = str_lines.last_mut() {
                last.length = spot - last.text;
            }
            if cbyte(text, spot) == 0 {
                break;
            }
            str_lines.push(TtfLine {
                text: spot,
                length: left,
            });

            let mut max_width = wrap_width;
            if max_width > 0 {
                max_width = std::cmp::max(max_width - xoffset, 1);
            }
            let sub = &text[spot.min(text.len())..];
            let mut max_length = match ttf_size_internal(
                this,
                font,
                sub,
                left,
                direction,
                script,
                true,
                max_width,
                include_spread,
            ) {
                Ok(s) => s.measured_length,
                Err(_) => return Err(Error::new("Error measure text")),
            };

            if wrap_width != 0 {
                // The first line can be empty if we have a text position that's
                // at the edge of the wrap length, but subsequent lines should have
                // at least one character per line.
                if max_length == 0 && str_lines.len() > 1 {
                    max_length = 1;
                }
            }

            let end = spot + max_length;
            while spot < end {
                let c = step_utf8(text, &mut spot, &mut left);

                if c == UNICODE_BOM_NATIVE || c == UNICODE_BOM_SWAPPED {
                    continue;
                }

                // With wrap_width == 0, normal text rendering but newline aware
                let is_delim = if wrap_width > 0 {
                    character_is_delimiter(c)
                } else {
                    character_is_new_line(c)
                };

                // Record last delimiter position
                if is_delim {
                    save_text = Some(spot);
                    save_length = left;
                    // Break, if new line
                    if c == '\n' as u32 || (c == '\r' as u32 && cbyte(text, spot) != b'\n') {
                        break;
                    }
                }

                if c == 0 && left > 0 {
                    /* (SDL_StepUTF8() does not advance at a NUL byte; C */
                    /* loops on it with the same pointer)                */
                    break;
                }
            }

            // Cut at last delimiter/new lines, otherwise in the middle of the word
            if let Some(s) = save_text {
                if left > 0 {
                    spot = s;
                    left = save_length;
                }
            }

            // First line is complete, start the next at offset 0
            xoffset = 0;

            if left == 0 {
                break;
            }
        }

        let num_lines = str_lines.len();
        for (i, line) in str_lines.iter_mut().enumerate() {
            if line.length == 0 {
                continue;
            }

            // The line doesn't include any delimiter that caused it to be wrapped.
            if character_is_new_line(cbyte(text, line.text + line.length - 1) as u32) {
                line.length -= 1;
                if line.length > 0 && cbyte(text, line.text + line.length - 1) == b'\r' {
                    line.length -= 1;
                }
            } else if i < num_lines - 1
                && character_is_delimiter(cbyte(text, line.text + line.length - 1) as u32)
            {
                line.length -= 1;
            }

            if trim_whitespace {
                while line.length > 0
                    && character_is_delimiter(cbyte(text, line.text + line.length - 1) as u32)
                {
                    line.length -= 1;
                }
            }
        }
    }

    let num_lines = str_lines.len() as i32;
    let row_height = std::cmp::max(height, font.lineskip);

    if wrap_width == 0 {
        // Find the max of all line lengths
        if num_lines > 1 {
            width = 0;
            for line in &str_lines {
                let sub = &text[line.text.min(text.len())..];
                if let Ok(s) = ttf_size_internal(
                    this,
                    font,
                    sub,
                    line.length,
                    font.direction,
                    font.script,
                    false,
                    0,
                    include_spread,
                ) {
                    width = std::cmp::max(s.w, width);
                }
            }
            // In case there are all newlines
            width = std::cmp::max(width, 1);
        }
    } else if num_lines <= 1 && font.horizontal_align == HorizontalAlignment::Left {
        // Don't go above wrap_width if you have only 1 line which hasn't been cut
        width = std::cmp::min(wrap_width, width);
    } else {
        width = wrap_width;
    }
    let height = row_height.wrapping_add(font.lineskip.wrapping_mul(num_lines - 1));

    Ok((str_lines, width, height))
}

/// `TTF_Render_Wrapped_Internal`
fn ttf_render_wrapped_internal(
    font: &Font,
    text: &[u8],
    fg: Color,
    bg: Color,
    wrap_width: i32,
    render_mode: RenderMode,
) -> Result<Surface<'static>> {
    let this = font.weak();
    let mut fd = font.rc.borrow_mut();
    let font = &mut *fd;
    let mut fg = fg;

    let length = text.len();
    let (direction, script) = (font.direction, font.script);
    let (str_lines, width, height) = get_wrapped_lines(
        &this, font, text, length, direction, script, 0, wrap_width, true, true,
    )?;

    if render_mode == RenderMode::Lcd && !ft_is_scalable(&font.face) {
        return Err(Error::new(
            "LCD rendering is not available for non-scalable font",
        ));
    }

    // Create surface for rendering
    if fg.a == 0 {
        fg.a = 255;
    }
    let mut color: u32 = 0;
    let textbuf = match render_mode {
        RenderMode::Solid => create_surface_solid(width, height, fg, &mut color),
        RenderMode::Shaded => create_surface_shaded(width, height, fg, bg, &mut color),
        RenderMode::Blended => create_surface_blended(width, height, fg, &mut color),
        RenderMode::Lcd => create_surface_lcd(width, height, fg, bg, &mut color),
    };

    let Some(mut textbuf) = textbuf else {
        return Err(Error::out_of_memory());
    };

    // Render each line
    for (i, line) in str_lines.iter().enumerate() {
        // Initialize xstart, ystart and compute positions
        let sub = &text[line.text.min(text.len())..];
        let s = ttf_size_internal(
            &this,
            font,
            sub,
            line.length,
            font.direction,
            font.script,
            false,
            0,
            true,
        )?;
        let (line_width, xstart) = (s.w, s.xstart);

        // Move to i-th line
        let ystart = s.ystart + i as i32 * font.lineskip;

        // Control left/right/center align of each bit of text
        let mut xoffset = match font.horizontal_align {
            HorizontalAlignment::Right => width - line_width,
            HorizontalAlignment::Center => (width - line_width) / 2,
            _ => 0,
        };
        xoffset = std::cmp::max(0, xoffset);

        // Render one text line to textbuf at (xstart, ystart)
        render_line(
            render_mode,
            font.render_subpixel,
            &this,
            font,
            &mut textbuf.buf,
            xstart + xoffset,
            ystart,
            fg,
        )?;

        // Apply underline or strikethrough style, if needed
        if ttf_handle_style_underline(font) {
            draw_line(
                font.direction,
                &mut textbuf.buf,
                xoffset,
                ystart + font.underline_top_row,
                line_width,
                font.line_thickness,
                color,
                render_mode,
            );
        }

        if ttf_handle_style_strikethrough(font) {
            draw_line(
                font.direction,
                &mut textbuf.buf,
                xoffset,
                ystart + font.strikethrough_top_row,
                line_width,
                font.line_thickness,
                color,
                render_mode,
            );
        }
    }

    textbuf.into_surface()
}

/// `TTF_SetFontSizeDPI`
fn set_font_size_dpi(
    this: &Weak<RefCell<FontData>>,
    font: &mut FontData,
    ptsize: f32,
    hdpi: i32,
    vdpi: i32,
) -> Result<()> {
    let (mut hdpi, mut vdpi) = (hdpi, vdpi);
    if ptsize <= 0.0 {
        return Err(Error::invalid_param("ptsize"));
    }

    if hdpi <= 0 && vdpi <= 0 {
        hdpi = font.hdpi;
        vdpi = font.vdpi;
    } else if hdpi <= 0 {
        hdpi = vdpi;
    } else if vdpi <= 0 {
        vdpi = hdpi;
    }

    if ptsize == font.ptsize && hdpi == font.hdpi && vdpi == font.vdpi {
        return Ok(());
    }

    // Make sure that our font face is scalable (global metrics)
    if ft_is_scalable(&font.face) {
        /* Set the character size using the provided DPI.  If a zero DPI
         * is provided, then the other DPI setting will be used.  If both
         * are zero, then Freetype's default 72 DPI will be used.  */
        ft_set_char_size(
            &mut font.face,
            0,
            (ptsize * 64.0).round() as i32 as FtF26Dot6,
            hdpi as FtUInt,
            vdpi as FtUInt,
        )
        .map_err(|e| ttf_set_ft_error("Couldn't set font size", e))?;
    } else {
        /* Non-scalable font case.  ptsize determines which family
         * or series of fonts to grab from the non-scalable format.
         * It is not the point size of the font.  */
        if font.face.num_fixed_sizes <= 0 {
            return Err(Error::new("Couldn't select size : no num_fixed_sizes"));
        }

        // within [0; num_fixed_sizes - 1]
        let mut index = ptsize as i32;
        index = std::cmp::max(index, 0);
        index = std::cmp::min(index, font.face.num_fixed_sizes - 1);

        ft_select_size(&mut font.face, index)
            .map_err(|e| ttf_set_ft_error("Couldn't select size", e))?;
    }

    ttf_init_font_metrics(font);

    font.ptsize = ptsize;
    font.hdpi = hdpi;
    font.vdpi = vdpi;

    flush_cache(font);
    update_font_text_borrowed(this, font);

    /* TTF_USE_HARFBUZZ */
    // Call when size or variations settings on underlying FT_Face change.
    hb_ft_font_changed(&mut font.hb_font, &mut font.face);

    Ok(())
}

/// `TTF_SetFontKerning` (on the font's data)
fn set_font_kerning(font: &mut FontData, enabled: bool) -> bool {
    if enabled == font.enable_kerning {
        return false;
    }

    font.enable_kerning = enabled;
    /* TTF_USE_HARFBUZZ: Harfbuzz can do kerning positioning even if the font hasn't the data */
    true
}

/// `RemoveOneTextCallback` and `TTF_CloseFont`
fn close_font(rc: &FontRc) {
    let texts = std::mem::take(&mut rc.borrow_mut().text);
    for text in texts {
        if let Some(text) = text.upgrade() {
            crate::text::detach_font(&text, rc);
        }
    }

    {
        let mut font = rc.borrow_mut();
        flush_cache(&mut font);
    }

    clear_fallback_fonts(rc);
    loop {
        let first = rc.borrow().fallback_for.first().cloned();
        let Some(first) = first else {
            break;
        };
        match first.upgrade() {
            Some(f) => remove_fallback_font(&f, rc),
            None => {
                rc.borrow_mut().fallback_for.remove(0);
            }
        }
    }

    /* (the face, stroker and stream go with the font data) */
}

impl Font {
    /// Set a font's size dynamically. Translation of `TTF_SetFontSize()`.
    pub fn set_size(&self, ptsize: f32) -> Result<()> {
        self.set_size_dpi(ptsize, 0, 0)
    }

    /// Set font size dynamically with target resolutions, in dots per
    /// inch. Translation of `TTF_SetFontSizeDPI()`.
    pub fn set_size_dpi(&self, ptsize: f32, hdpi: i32, vdpi: i32) -> Result<()> {
        let this = self.weak();
        set_font_size_dpi(&this, &mut self.rc.borrow_mut(), ptsize, hdpi, vdpi)
    }

    /// Get the size of a font. Translation of `TTF_GetFontSize()`.
    pub fn size(&self) -> f32 {
        self.rc.borrow().ptsize
    }

    /// Get font target resolutions, in dots per inch, as `(hdpi, vdpi)`.
    /// Translation of `TTF_GetFontDPI()`.
    pub fn dpi(&self) -> (i32, i32) {
        let font = self.rc.borrow();
        (font.hdpi, font.vdpi)
    }

    /// Set a font's current style. Translation of `TTF_SetFontStyle()`.
    pub fn set_style(&self, style: FontStyleFlags) {
        let this = self.weak();
        let mut fd = self.rc.borrow_mut();
        let font = &mut *fd;
        let mut style = style;

        let prev_style = font.style;
        let face_style = font.face.style_flags;

        // Don't add a style if already in the font, SDL_ttf doesn't need to handle them
        if face_style & FT_STYLE_FLAG_BOLD != 0 {
            style &= !STYLE_BOLD;
        }
        if face_style & FT_STYLE_FLAG_ITALIC != 0 {
            style &= !STYLE_ITALIC;
        }

        if font.style == style {
            return;
        }

        font.style = style;

        ttf_init_font_metrics(font);

        /* Flush the cache if styles that impact glyph drawing have changed */
        if (font.style | TTF_STYLE_NO_GLYPH_CHANGE) != (prev_style | TTF_STYLE_NO_GLYPH_CHANGE) {
            flush_cache(font);
        }
        update_font_text_borrowed(&this, font);
    }

    /// Query a font's current style. Translation of `TTF_GetFontStyle()`.
    pub fn style(&self) -> FontStyleFlags {
        let font = self.rc.borrow();

        let mut style = font.style;
        let face_style = font.face.style_flags;

        // Add the style already in the font
        if face_style & FT_STYLE_FLAG_BOLD != 0 {
            style |= STYLE_BOLD;
        }
        if face_style & FT_STYLE_FLAG_ITALIC != 0 {
            style |= STYLE_ITALIC;
        }

        style
    }

    /// Set a font's current outline. Translation of `TTF_SetFontOutline()`.
    pub fn set_outline(&self, outline: i32) -> Result<()> {
        let props = self.properties();
        let this = self.weak();
        let mut fd = self.rc.borrow_mut();
        let font = &mut *fd;

        let outline = std::cmp::max(0, outline);

        if outline == font.outline {
            return Ok(());
        }

        if outline > 0 {
            if font.stroker.is_none() {
                let Some(library) = ttf_library() else {
                    return Err(Error::new("Couldn't create font stroker"));
                };
                font.stroker = Some(
                    ft_stroker_new(&library)
                        .map_err(|e| ttf_set_ft_error("Couldn't create font stroker", e))?,
                );
            }

            let line_cap = props
                .get_number(PROP_FONT_OUTLINE_LINE_CAP_NUMBER)
                .unwrap_or(FT_STROKER_LINECAP_ROUND as i64);
            let line_join = props
                .get_number(PROP_FONT_OUTLINE_LINE_JOIN_NUMBER)
                .unwrap_or(FT_STROKER_LINEJOIN_ROUND as i64);
            let miter_limit = props
                .get_number(PROP_FONT_OUTLINE_MITER_LIMIT_NUMBER)
                .unwrap_or(0);
            ft_stroker_set(
                font.stroker.as_mut().unwrap(),
                outline as FtFixed * 64,
                line_cap as FtStrokerLineCap,
                line_join as FtStrokerLineJoin,
                miter_limit as FtFixed,
            );
        } else if let Some(stroker) = font.stroker.take() {
            ft_stroker_done(stroker);
        }

        font.outline = outline;

        ttf_init_font_metrics(font);
        flush_cache(font);
        update_font_text_borrowed(&this, font);

        Ok(())
    }

    /// Query a font's current outline. Translation of
    /// `TTF_GetFontOutline()`.
    pub fn outline(&self) -> i32 {
        self.rc.borrow().outline
    }

    /// Set a font's current hinter setting. Translation of
    /// `TTF_SetFontHinting()`.
    ///
    /// FIXME (upstream): the subpixel setting computed for
    /// [`Hinting::LightSubpixel`] is never stored, so that hinting renders
    /// as [`Hinting::Light`] (and is reported as such).
    pub fn set_hinting(&self, hinting: Hinting) {
        let this = self.weak();
        let mut fd = self.rc.borrow_mut();
        let font = &mut *fd;

        let ft_load_target = if hinting == Hinting::Light || hinting == Hinting::LightSubpixel {
            FT_LOAD_TARGET_LIGHT
        } else if hinting == Hinting::Mono {
            FT_LOAD_TARGET_MONO
        } else if hinting == Hinting::None {
            FT_LOAD_NO_HINTING
        } else {
            FT_LOAD_TARGET_NORMAL
        };

        let render_subpixel = if hinting == Hinting::LightSubpixel {
            1
        } else {
            0
        };

        if ft_load_target == font.ft_load_target && render_subpixel == font.render_subpixel {
            return;
        }

        font.ft_load_target = ft_load_target;

        /* TTF_USE_HARFBUZZ: update flag for HB */
        hb_ft_font_set_load_flags(&mut font.hb_font, FT_LOAD_DEFAULT | font.ft_load_target);

        flush_cache(font);
        update_font_text_borrowed(&this, font);
    }

    /// Query a font's current FreeType hinter setting. Translation of
    /// `TTF_GetFontHinting()`.
    pub fn hinting(&self) -> Hinting {
        let font = self.rc.borrow();

        if font.ft_load_target == FT_LOAD_TARGET_LIGHT {
            if font.render_subpixel == 0 {
                Hinting::Light
            } else {
                Hinting::LightSubpixel
            }
        } else if font.ft_load_target == FT_LOAD_TARGET_MONO {
            Hinting::Mono
        } else if font.ft_load_target == FT_LOAD_NO_HINTING {
            Hinting::None
        } else {
            Hinting::Normal
        }
    }

    /// Enable Signed Distance Field rendering for a font. Translation of
    /// `TTF_SetFontSDF()`.
    pub fn set_sdf(&self, enabled: bool) -> Result<()> {
        let this = self.weak();
        let mut fd = self.rc.borrow_mut();
        let font = &mut *fd;
        /* TTF_USE_SDF */
        if font.render_sdf != enabled {
            font.render_sdf = enabled;
            flush_cache(font);
            update_font_text_borrowed(&this, font);
        }
        Ok(())
    }

    /// Query whether Signed Distance Field rendering is enabled for a
    /// font. Translation of `TTF_GetFontSDF()`.
    pub fn sdf(&self) -> bool {
        self.rc.borrow().render_sdf
    }

    /// Query a font's weight, in terms of the lightness/heaviness of the
    /// strokes. Translation of `TTF_GetFontWeight()`.
    pub fn weight(&self) -> i32 {
        self.rc.borrow().weight
    }

    /// Set a font's current wrap alignment option. Translation of
    /// `TTF_SetFontWrapAlignment()`.
    pub fn set_wrap_alignment(&self, align: HorizontalAlignment) {
        let this = self.weak();
        let mut fd = self.rc.borrow_mut();
        let font = &mut *fd;

        if align == font.horizontal_align {
            return;
        }

        match align {
            HorizontalAlignment::Left
            | HorizontalAlignment::Center
            | HorizontalAlignment::Right => {
                font.horizontal_align = align;
            }
            _ => {
                // Ignore invalid values
            }
        }
        update_font_text_borrowed(&this, font);
    }

    /// Query a font's current wrap alignment option. Translation of
    /// `TTF_GetFontWrapAlignment()`.
    pub fn wrap_alignment(&self) -> HorizontalAlignment {
        self.rc.borrow().horizontal_align
    }

    /// Query the total height of a font. Translation of
    /// `TTF_GetFontHeight()`.
    pub fn height(&self) -> i32 {
        self.rc.borrow().height
    }

    /// Query the offset from the baseline to the top of a font.
    /// Translation of `TTF_GetFontAscent()`.
    pub fn ascent(&self) -> i32 {
        let font = self.rc.borrow();
        font.ascent + 2 * font.outline
    }

    /// Query the offset from the baseline to the bottom of a font.
    /// Translation of `TTF_GetFontDescent()`.
    pub fn descent(&self) -> i32 {
        self.rc.borrow().descent
    }

    /// Set the spacing between lines of text for a font. Translation of
    /// `TTF_SetFontLineSkip()`.
    pub fn set_line_skip(&self, lineskip: i32) {
        let this = self.weak();
        let mut fd = self.rc.borrow_mut();
        let font = &mut *fd;

        if lineskip == font.lineskip {
            return;
        }

        font.lineskip = lineskip;
        update_font_text_borrowed(&this, font);
    }

    /// Query the spacing between lines of text for a font. Translation of
    /// `TTF_GetFontLineSkip()`.
    pub fn line_skip(&self) -> i32 {
        self.rc.borrow().lineskip
    }

    /// Set if kerning is enabled for a font. Translation of
    /// `TTF_SetFontKerning()`.
    pub fn set_kerning(&self, enabled: bool) {
        let this = self.weak();
        let mut fd = self.rc.borrow_mut();
        if set_font_kerning(&mut fd, enabled) {
            update_font_text_borrowed(&this, &mut fd);
        }
    }

    /// Query whether or not kerning is enabled for a font. Translation of
    /// `TTF_GetFontKerning()`.
    pub fn kerning(&self) -> bool {
        self.rc.borrow().enable_kerning
    }

    /// Query the number of faces of a font. Translation of
    /// `TTF_GetNumFontFaces()`.
    pub fn num_faces(&self) -> i32 {
        self.rc.borrow().face.num_faces as i32
    }

    /// Query whether a font is fixed-width. Translation of
    /// `TTF_FontIsFixedWidth()`.
    pub fn is_fixed_width(&self) -> bool {
        ft_is_fixed_width(&self.rc.borrow().face)
    }

    /// Query whether a font is scalable or not. Translation of
    /// `TTF_FontIsScalable()`.
    pub fn is_scalable(&self) -> bool {
        ft_is_scalable(&self.rc.borrow().face)
    }

    /// Query a font's family name. Translation of
    /// `TTF_GetFontFamilyName()`.
    pub fn family_name(&self) -> Option<String> {
        self.rc.borrow().face.family_name.clone()
    }

    /// Query a font's style name. Translation of `TTF_GetFontStyleName()`.
    pub fn style_name(&self) -> Option<String> {
        self.rc.borrow().face.style_name.clone()
    }

    /// Set the direction to be used for text shaping by a font.
    /// Translation of `TTF_SetFontDirection()`.
    pub fn set_direction(&self, direction: Direction) -> Result<()> {
        let this = self.weak();
        let mut fd = self.rc.borrow_mut();
        let font = &mut *fd;

        if direction == font.direction {
            return Ok(());
        }

        font.direction = direction;
        update_font_text_borrowed(&this, font);
        Ok(())
    }

    /// Get the direction to be used for text shaping by a font.
    /// Translation of `TTF_GetFontDirection()`.
    pub fn direction(&self) -> Direction {
        self.rc.borrow().direction
    }

    /// Set the script to be used for text shaping by a font (an ISO 15924
    /// tag, see [`string_to_tag`]). Translation of `TTF_SetFontScript()`.
    pub fn set_script(&self, script: u32) -> Result<()> {
        let this = self.weak();
        let mut fd = self.rc.borrow_mut();
        let font = &mut *fd;

        /* TTF_USE_HARFBUZZ */
        font.script = script;
        update_font_text_borrowed(&this, font);
        Ok(())
    }

    /// Get the script used for text shaping a font. Translation of
    /// `TTF_GetFontScript()`.
    pub fn script(&self) -> u32 {
        self.rc.borrow().script
    }

    /// Set language to be used for text shaping by a font (a BCP 47 tag;
    /// `None` resets it). Translation of `TTF_SetFontLanguage()`.
    pub fn set_language(&self, language_bcp47: Option<&str>) -> Result<()> {
        let this = self.weak();
        let mut fd = self.rc.borrow_mut();
        let font = &mut *fd;

        /* TTF_USE_HARFBUZZ */
        let hb_language = match language_bcp47 {
            None => hb_language_from_string(b""),
            Some(l) => hb_language_from_string(l.as_bytes()),
        };

        if hb_language == font.hb_language {
            return Ok(());
        }

        font.hb_language = hb_language;
        update_font_text_borrowed(&this, font);
        Ok(())
    }

    /// Check whether a glyph is provided by the font for a UNICODE
    /// codepoint. Translation of `TTF_FontHasGlyph()`.
    pub fn has_glyph(&self, ch: u32) -> bool {
        let this = self.weak();
        let mut fd = self.rc.borrow_mut();
        get_char_index_fallback(&this, &mut fd, ch, None).0 > 0
    }

    /// Get the pixel image for a UNICODE codepoint, and the type of its
    /// data. Translation of `TTF_GetGlyphImage()`.
    pub fn glyph_image(&self, ch: u32) -> Result<(Surface<'static>, ImageType)> {
        let this = self.weak();
        let (idx, glyph_font) = {
            let mut fd = self.rc.borrow_mut();
            get_char_index_fallback(&this, &mut fd, ch, None)
        };
        if idx == 0 {
            return Err(Error::new("Codepoint not in font"));
        }

        let glyph_font = glyph_font.unwrap_or_else(|| this.clone());
        if Weak::ptr_eq(&glyph_font, &this) {
            glyph_image_for_index(&mut self.rc.borrow_mut(), idx)
        } else {
            let Some(gf) = glyph_font.upgrade() else {
                return Err(Error::new("Codepoint not in font"));
            };
            let mut gf = gf.borrow_mut();
            glyph_image_for_index(&mut gf, idx)
        }
    }

    /// Get the pixel image for a character index, and the type of its
    /// data. Translation of `TTF_GetGlyphImageForIndex()`.
    pub fn glyph_image_for_index(&self, glyph_index: u32) -> Result<(Surface<'static>, ImageType)> {
        glyph_image_for_index(&mut self.rc.borrow_mut(), glyph_index)
    }

    /// Query the metrics (dimensions) of a font's glyph for a UNICODE
    /// codepoint, as `(minx, maxx, miny, maxy, advance)`. Translation of
    /// `TTF_GetGlyphMetrics()`.
    pub fn glyph_metrics(&self, ch: u32) -> Result<(i32, i32, i32, i32, i32)> {
        let this = self.weak();
        let mut fd = self.rc.borrow_mut();
        let font = &mut *fd;

        let (gf, idx) = find_glyph_metrics(&this, font, ch, true)?;
        let glyph = with_font_data(&this, font, &gf, |f| {
            f.glyphs.get(&idx).map(|g| g.clone_metrics())
        })
        .flatten()
        .unwrap_or_default();

        let minx = glyph.sz_left;
        let maxx = glyph.sz_left + glyph.sz_width + 2 * font.outline;
        let miny = glyph.sz_top - glyph.sz_rows;
        let maxy = glyph.sz_top + 2 * font.outline;
        let advance = ft_ceil(glyph.advance as i64);
        Ok((minx, maxx, miny, maxy, advance))
    }

    /// Query the kerning size between the glyphs of two UNICODE
    /// codepoints. Translation of `TTF_GetGlyphKerning()`.
    pub fn glyph_kerning(&self, previous_ch: u32, ch: u32) -> Result<i32> {
        let this = self.weak();
        let mut fd = self.rc.borrow_mut();
        let font = &mut *fd;

        if ch == UNICODE_BOM_NATIVE || ch == UNICODE_BOM_SWAPPED {
            return Ok(0);
        }

        if previous_ch == UNICODE_BOM_NATIVE || previous_ch == UNICODE_BOM_SWAPPED {
            return Ok(0);
        }

        let mut kerning = 0;
        if let (Ok((_, glyph)), Ok((_, prev_glyph))) = (
            find_glyph_metrics(&this, font, ch, false),
            find_glyph_metrics(&this, font, previous_ch, false),
        ) {
            let delta = ft_get_kerning(&mut font.face, prev_glyph, glyph, FT_KERNING_DEFAULT)
                .map_err(|e| ttf_set_ft_error("Couldn't get glyph kerning", e))?;

            kerning = (delta.x >> 6) as i32;
        }
        Ok(kerning)
    }

    /// Calculate the dimensions of a rendered string of UTF-8 text, as
    /// `(w, h)`. Translation of `TTF_GetStringSize()`.
    pub fn string_size(&self, text: &str) -> Result<(i32, i32)> {
        let this = self.weak();
        let mut fd = self.rc.borrow_mut();
        let font = &mut *fd;
        let text = text.as_bytes();
        let length = c_length(text, text.len());
        let s = ttf_size_internal(
            &this,
            font,
            text,
            length,
            font.direction,
            font.script,
            false,
            0,
            true,
        )?;
        Ok((s.w, s.h))
    }

    /// Calculate how much of a UTF-8 string will fit in a given width, as
    /// `(measured_width, measured_length)`. Translation of
    /// `TTF_MeasureString()`.
    pub fn measure_string(&self, text: &str, max_width: i32) -> Result<(i32, usize)> {
        let this = self.weak();
        let mut fd = self.rc.borrow_mut();
        let font = &mut *fd;
        let text = text.as_bytes();
        let length = c_length(text, text.len());
        let s = ttf_size_internal(
            &this,
            font,
            text,
            length,
            font.direction,
            font.script,
            true,
            max_width,
            true,
        )?;
        Ok((s.measured_width, s.measured_length))
    }

    /// Render UTF-8 text at fast quality to a new 8-bit surface.
    /// Translation of `TTF_RenderText_Solid()`.
    pub fn render_text_solid(&self, text: &str, fg: Color) -> Result<Surface<'static>> {
        ttf_render_internal(
            self,
            text.as_bytes(),
            fg,
            fg, /* unused */
            RenderMode::Solid,
        )
    }

    /// Render a single UNICODE codepoint at fast quality to a new 8-bit
    /// surface. Translation of `TTF_RenderGlyph_Solid()`.
    pub fn render_glyph_solid(&self, ch: u32, fg: Color) -> Result<Surface<'static>> {
        ttf_render_internal(self, &ucs4_to_utf8(ch), fg, fg, RenderMode::Solid)
    }

    /// Render UTF-8 text at high quality to a new 8-bit surface.
    /// Translation of `TTF_RenderText_Shaded()`.
    pub fn render_text_shaded(&self, text: &str, fg: Color, bg: Color) -> Result<Surface<'static>> {
        ttf_render_internal(self, text.as_bytes(), fg, bg, RenderMode::Shaded)
    }

    /// Render a single UNICODE codepoint at high quality to a new 8-bit
    /// surface. Translation of `TTF_RenderGlyph_Shaded()`.
    pub fn render_glyph_shaded(&self, ch: u32, fg: Color, bg: Color) -> Result<Surface<'static>> {
        ttf_render_internal(self, &ucs4_to_utf8(ch), fg, bg, RenderMode::Shaded)
    }

    /// Render UTF-8 text at high quality to a new ARGB surface.
    /// Translation of `TTF_RenderText_Blended()`.
    pub fn render_text_blended(&self, text: &str, fg: Color) -> Result<Surface<'static>> {
        ttf_render_internal(
            self,
            text.as_bytes(),
            fg,
            fg, /* unused */
            RenderMode::Blended,
        )
    }

    /// Render a single UNICODE codepoint at high quality to a new ARGB
    /// surface. Translation of `TTF_RenderGlyph_Blended()`.
    pub fn render_glyph_blended(&self, ch: u32, fg: Color) -> Result<Surface<'static>> {
        ttf_render_internal(self, &ucs4_to_utf8(ch), fg, fg, RenderMode::Blended)
    }

    /// Render UTF-8 text at LCD subpixel quality to a new ARGB surface.
    /// Translation of `TTF_RenderText_LCD()`.
    pub fn render_text_lcd(&self, text: &str, fg: Color, bg: Color) -> Result<Surface<'static>> {
        ttf_render_internal(self, text.as_bytes(), fg, bg, RenderMode::Lcd)
    }

    /// Render a single UNICODE codepoint at LCD subpixel quality to a new
    /// ARGB surface. Translation of `TTF_RenderGlyph_LCD()`.
    pub fn render_glyph_lcd(&self, ch: u32, fg: Color, bg: Color) -> Result<Surface<'static>> {
        ttf_render_internal(self, &ucs4_to_utf8(ch), fg, bg, RenderMode::Lcd)
    }

    /// Calculate the dimensions of a rendered string of UTF-8 text, wrapped
    /// at `wrap_width` (0 wraps on newlines only), as `(w, h)`.
    /// Translation of `TTF_GetStringSizeWrapped()`.
    pub fn string_size_wrapped(&self, text: &str, wrap_width: i32) -> Result<(i32, i32)> {
        let this = self.weak();
        let mut fd = self.rc.borrow_mut();
        let font = &mut *fd;
        let text = text.as_bytes();
        let (direction, script) = (font.direction, font.script);
        let (_, w, h) = get_wrapped_lines(
            &this,
            font,
            text,
            text.len(),
            direction,
            script,
            0,
            wrap_width,
            true,
            true,
        )?;
        Ok((w, h))
    }

    /// Render word-wrapped UTF-8 text at fast quality to a new 8-bit
    /// surface. Translation of `TTF_RenderText_Solid_Wrapped()`.
    pub fn render_text_solid_wrapped(
        &self,
        text: &str,
        fg: Color,
        wrap_width: i32,
    ) -> Result<Surface<'static>> {
        ttf_render_wrapped_internal(
            self,
            text.as_bytes(),
            fg,
            fg, /* unused */
            wrap_width,
            RenderMode::Solid,
        )
    }

    /// Render word-wrapped UTF-8 text at high quality to a new 8-bit
    /// surface. Translation of `TTF_RenderText_Shaded_Wrapped()`.
    pub fn render_text_shaded_wrapped(
        &self,
        text: &str,
        fg: Color,
        bg: Color,
        wrap_width: i32,
    ) -> Result<Surface<'static>> {
        ttf_render_wrapped_internal(
            self,
            text.as_bytes(),
            fg,
            bg,
            wrap_width,
            RenderMode::Shaded,
        )
    }

    /// Render word-wrapped UTF-8 text at high quality to a new ARGB
    /// surface. Translation of `TTF_RenderText_Blended_Wrapped()`.
    pub fn render_text_blended_wrapped(
        &self,
        text: &str,
        fg: Color,
        wrap_width: i32,
    ) -> Result<Surface<'static>> {
        ttf_render_wrapped_internal(
            self,
            text.as_bytes(),
            fg,
            fg, /* unused */
            wrap_width,
            RenderMode::Blended,
        )
    }

    /// Render word-wrapped UTF-8 text at LCD subpixel quality to a new
    /// ARGB surface. Translation of `TTF_RenderText_LCD_Wrapped()`.
    pub fn render_text_lcd_wrapped(
        &self,
        text: &str,
        fg: Color,
        bg: Color,
        wrap_width: i32,
    ) -> Result<Surface<'static>> {
        ttf_render_wrapped_internal(self, text.as_bytes(), fg, bg, wrap_width, RenderMode::Lcd)
    }
}

/// `TTF_GetGlyphImageForIndex`
pub(crate) fn glyph_image_for_index(
    font: &mut FontData,
    glyph_index: u32,
) -> Result<(Surface<'static>, ImageType)> {
    let alignment = (ALIGNMENT - 1) as usize;

    find_glyph_by_index(font, glyph_index, 0, 0, CACHED_COLOR, 0, 0, 0)?;
    let image = &font.glyphs[&glyph_index].pixmap;

    if image.width == 0 || image.rows == 0 {
        return Ok((
            Surface::new(1, 1, PixelFormat::ARGB8888)?,
            ImageType::Invalid,
        ));
    }

    let mut surface = Surface::new(image.width, image.rows, PixelFormat::ARGB8888)?;
    let image_type;

    let pitch = surface.pitch() as usize;
    let w = surface.width() as usize;
    let pixels = surface
        .pixels_mut()
        .ok_or_else(|| Error::new("Surface has no pixels"))?;
    let mut src = alignment;

    if image.is_color != 0 {
        // We can't tell the difference between SDF data and say, color emoji
        // Hopefully the application sets the right mode on the font.
        image_type = if font.render_sdf {
            ImageType::Sdf
        } else {
            ImageType::Color
        };

        if pitch == image.pitch as usize {
            let n = image.rows as usize * image.pitch as usize;
            pixels[..n].copy_from_slice(&image.buffer[src..src + n]);
        } else {
            let mut dst = 0;
            let length = image.width as usize * 4;
            for _row in 0..image.rows {
                pixels[dst..dst + length].copy_from_slice(&image.buffer[src..src + length]);
                src += image.pitch as usize;
                dst += pitch;
            }
        }
    } else {
        image_type = ImageType::Alpha;

        let mut dst = 0;
        let skip = (pitch - w * 4) / 4;
        for _row in 0..image.rows {
            for col in 0..image.width as usize {
                let v = image.buffer[src + col] as u32;
                pixels[dst * 4..dst * 4 + 4]
                    .copy_from_slice(&(0x00FFFFFF | (v << 24)).to_ne_bytes());
                dst += 1;
            }
            src += image.pitch as usize;
            dst += skip;
        }
    }
    Ok((surface, image_type))
}

/// Convert from a 4 character string to a 32-bit tag. Translation of
/// `TTF_StringToTag()`.
pub fn string_to_tag(string: Option<&str>) -> u32 {
    let mut bytes = [0u8; 4];

    if let Some(string) = string {
        for (i, &b) in string.as_bytes().iter().take(4).enumerate() {
            if b == 0 {
                break;
            }
            bytes[i] = b;
        }
    }

    ((bytes[0] as u32) << 24)
        | ((bytes[1] as u32) << 16)
        | ((bytes[2] as u32) << 8)
        | bytes[3] as u32
}

/// Convert from a 32-bit tag to a 4 character string (C writes the four
/// bytes and a NUL terminator). Translation of `TTF_TagToString()`.
pub fn tag_to_string(tag: u32) -> [u8; 4] {
    let mut tag = tag;
    let mut string = [0u8; 4];
    for b in string.iter_mut() {
        *b = (tag >> 24) as u8;
        tag <<= 8;
    }
    string
}

/// Get the script used by a 32-bit codepoint (its ISO 15924 tag).
/// Translation of `TTF_GetGlyphScript()`.
pub fn glyph_script(ch: u32) -> Result<u32> {
    /* TTF_USE_HARFBUZZ */
    let hb_buffer = HbBuffer::new();
    let hb_unicode_functions = hb_buffer.get_unicode_funcs();
    let script = hb_script_to_iso15924_tag(hb_unicode_script(&hb_unicode_functions, ch));

    if script == 0 {
        return Err(Error::new("Unknown script"));
    }
    Ok(script)
}

/// Deinitialize SDL_ttf. Translation of `TTF_Quit()`.
pub fn quit() {
    let mut library = TTF_STATE.library.lock().unwrap_or_else(|e| e.into_inner());
    if library.is_none() {
        return;
    }

    if TTF_STATE.refcount.fetch_sub(1, Ordering::SeqCst) - 1 != 0 {
        return;
    }

    if let Some(lib) = library.take() {
        let _ = ft_done_freetype(lib);
    }
}

/// Check if SDL_ttf is initialized: the number of times [`init`] has been
/// called without a matching [`quit`]. Translation of `TTF_WasInit()`.
pub fn was_init() -> i32 {
    TTF_STATE.refcount.load(Ordering::SeqCst)
}
