// Rust translation of the TTF_Text part of src/SDL_ttf.c and of
// include/SDL3_ttf/SDL_textengine.h from SDL_ttf.
// Copyright (C) 2001-2025 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Text objects (`TTF_Text`) and the text engine interface
//! (`TTF_TextEngine`).
//!
//! Translation notes:
//!
//! * A [`Text`] owns its data; dropping it is `TTF_DestroyText()`. The
//!   font of a text and the texts of a font refer to each other weakly, as
//!   C's pointers do; closing a font detaches its texts.
//! * A text engine is an implementation of the [`TextEngine`] trait,
//!   shared by the texts that use it; its per-text representation
//!   (`engine_text`) is a boxed value of the engine's choice.
//! * The text's string is kept as bytes, as C keeps it: inserting and
//!   deleting work on byte offsets, which can split UTF-8 sequences.
//! * The drawing operations name the font of their glyph with a
//!   [`FontHandle`], a non-owning reference to a [`Font`].

use std::any::Any;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

use sdl3::properties::Properties;
use sdl3::video::{FColor, Point, Rect, Surface};
use sdl3::{Error, Result};

use crate::ttf::*;

/// `FT_FLOOR`
fn ft_floor(x: i32) -> i32 {
    (x & -64) / 64
}

/* SDL_textengine.h */

/// A font drawing operation command. Translation of `TTF_DrawCommand`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DrawCommand {
    /// `TTF_DRAW_COMMAND_NOOP`
    Noop,
    /// `TTF_DRAW_COMMAND_FILL`
    Fill,
    /// `TTF_DRAW_COMMAND_COPY`
    Copy,
}

/// A filled rectangle draw operation. Translation of
/// `TTF_FillOperation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FillOperation {
    /// The rectangle to fill, in pixels. The x coordinate is relative to
    /// the left side of the text area, going right, and the y coordinate
    /// is relative to the top side of the text area, going down.
    pub rect: Rect,
}

/// A non-owning reference to a [`Font`] (C's `TTF_Font *`), as the
/// drawing operations name the font of their glyph.
#[derive(Clone)]
pub struct FontHandle(pub(crate) Weak<RefCell<FontData>>);

impl std::fmt::Debug for FontHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("FontHandle")
            .field(&Weak::as_ptr(&self.0))
            .finish()
    }
}

impl PartialEq for FontHandle {
    fn eq(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for FontHandle {}

impl std::hash::Hash for FontHandle {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        Weak::as_ptr(&self.0).hash(state);
    }
}

impl FontHandle {
    /// The handle of `font`.
    pub fn of(font: &Font) -> FontHandle {
        FontHandle(font.weak())
    }

    /// Whether this is a handle of `font`.
    pub fn is(&self, font: &Font) -> bool {
        Weak::ptr_eq(&self.0, &font.weak())
    }

    /// Get the pixel image for a character index (an error if the font
    /// has been closed). Translation of `TTF_GetGlyphImageForIndex()`.
    pub fn glyph_image_for_index(&self, glyph_index: u32) -> Result<(Surface<'static>, ImageType)> {
        let font = self
            .0
            .upgrade()
            .ok_or_else(|| Error::invalid_param("font"))?;
        let mut font = font.borrow_mut();
        glyph_image_for_index(&mut font, glyph_index)
    }

    /// Get the font generation (0 if the font has been closed).
    /// Translation of `TTF_GetFontGeneration()`.
    pub fn generation(&self) -> u32 {
        self.0.upgrade().map_or(0, |f| f.borrow().generation)
    }
}

/// A texture copy draw operation. Translation of `TTF_CopyOperation`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyOperation {
    /// The offset in the text corresponding to this glyph.
    /// There may be multiple glyphs with the same text offset
    /// and the next text offset might be several Unicode codepoints
    /// later. In this case the glyphs and codepoints are grouped
    /// together and the group bounding box is the union of the dst
    /// rectangles for the corresponding glyphs.
    pub text_offset: i32,
    /// The font containing the glyph to be drawn, can be passed to
    /// `TTF_GetGlyphImageForIndex()`
    pub glyph_font: FontHandle,
    /// The glyph index of the glyph to be drawn, can be passed to
    /// `TTF_GetGlyphImageForIndex()`
    pub glyph_index: u32,
    /// The area within the glyph to be drawn
    pub src: Rect,
    /// The drawing coordinates of the glyph, in pixels. The x coordinate
    /// is relative to the left side of the text area, going right, and the
    /// y coordinate is relative to the top side of the text area, going
    /// down.
    pub dst: Rect,
    /* reserved: the engines keep their own data next to their copies */
}

/// A text engine draw operation. Translation of `TTF_DrawOperation`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrawOperation {
    /// `TTF_DRAW_COMMAND_NOOP`
    Noop,
    /// `TTF_DRAW_COMMAND_FILL`
    Fill(FillOperation),
    /// `TTF_DRAW_COMMAND_COPY`
    Copy(CopyOperation),
}

impl DrawOperation {
    /// The command of this operation (`cmd`).
    pub fn cmd(&self) -> DrawCommand {
        match self {
            DrawOperation::Noop => DrawCommand::Noop,
            DrawOperation::Fill(_) => DrawCommand::Fill,
            DrawOperation::Copy(_) => DrawCommand::Copy,
        }
    }
}

/// A text engine used to create text objects (`TTF_TextEngine`, with its
/// `CreateText` and `DestroyText` callbacks; `userdata` is the
/// implementing value).
pub trait TextEngine: Any + std::fmt::Debug {
    /// Create a text representation from draw instructions.
    ///
    /// All fields of `text` except its engine representation will already
    /// be filled out.
    ///
    /// The result is the engine's representation of the text
    /// (`internal->engine_text`).
    fn create_text(&self, text: &TextData) -> Result<Box<dyn Any>>;

    /// Destroy a text representation.
    fn destroy_text(&self, engine_text: Box<dyn Any>) {
        let _ = engine_text;
    }

    /// The engine as `Any`, to recognize it.
    fn as_any(&self) -> &dyn Any;
}

/* SDL_ttf.h */

/// Flags for [`SubString`]. Translation of `TTF_SubStringFlags`.
pub type SubStringFlags = u32;

/// The mask for the flow direction for this substring.
/// Translation of `TTF_SUBSTRING_DIRECTION_MASK`.
pub const SUBSTRING_DIRECTION_MASK: SubStringFlags = 0x000000FF;
/// This substring contains the beginning of the text.
/// Translation of `TTF_SUBSTRING_TEXT_START`.
pub const SUBSTRING_TEXT_START: SubStringFlags = 0x00000100;
/// This substring contains the beginning of line `line_index`.
/// Translation of `TTF_SUBSTRING_LINE_START`.
pub const SUBSTRING_LINE_START: SubStringFlags = 0x00000200;
/// This substring contains the end of line `line_index`.
/// Translation of `TTF_SUBSTRING_LINE_END`.
pub const SUBSTRING_LINE_END: SubStringFlags = 0x00000400;
/// This substring contains the end of the text.
/// Translation of `TTF_SUBSTRING_TEXT_END`.
pub const SUBSTRING_TEXT_END: SubStringFlags = 0x00000800;

/// The representation of a substring within text. Translation of
/// `TTF_SubString`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SubString {
    /// The flags for this substring
    pub flags: SubStringFlags,
    /// The byte offset from the beginning of the text
    pub offset: i32,
    /// The byte length starting at the offset
    pub length: i32,
    /// The index of the line that contains this substring
    pub line_index: i32,
    /// The internal cluster index, used for quickly iterating
    pub cluster_index: i32,
    /// The rectangle, relative to the top left of the text, containing the
    /// substring
    pub rect: Rect,
}

/// `SDL_GetRectUnion( a, b, a )` (which leaves `a` alone on an overflow)
fn rect_union_into(a: &mut Rect, b: &Rect) {
    if let Ok(u) = a.union(b) {
        *a = u;
    }
}

/* SDL_ttf.c */

/// `TTF_TextLayout`: private data in `TTF_Text`, to assist in text
/// measurement and layout
#[derive(Debug, Clone)]
pub(crate) struct TextLayout {
    direction: Direction,
    script: u32, // ISO 15924 script tag
    font_height: i32,
    wrap_length: i32,
    wrap_whitespace_visible: bool,
    /// `lines` (empty is NULL)
    lines: Vec<i32>,
}

impl Default for TextLayout {
    fn default() -> Self {
        TextLayout {
            direction: Direction::Invalid,
            script: 0,
            font_height: 0,
            wrap_length: 0,
            wrap_whitespace_visible: false,
            lines: Vec::new(),
        }
    }
}

/// The data of a text object (`TTF_Text` with its `TTF_TextData`, and
/// `TTF_TextLayout`); what text engines see of it.
pub struct TextData {
    /// `text`: A copy of the UTF-8 string that this text object
    /// represents (`None` is NULL).
    pub(crate) text: Option<Vec<u8>>,
    /// `num_lines`: The number of lines in the text, 0 if it's empty
    pub(crate) num_lines: i32,

    /// The font used by this text, read-only.
    pub(crate) font: Option<Weak<RefCell<FontData>>>,
    /// The color of the text, read-only.
    pub(crate) color: FColor,
    /// True if the layout needs to be updated
    pub(crate) needs_layout_update: bool,
    /// Cached layout information, read-only.
    pub(crate) layout: TextLayout,
    /// The x offset of the upper left corner of this text, in pixels,
    /// read-only.
    pub(crate) x: i32,
    /// The y offset of the upper left corner of this text, in pixels,
    /// read-only.
    pub(crate) y: i32,
    /// The width of this text, in pixels, read-only.
    pub(crate) w: i32,
    /// The height of this text, in pixels, read-only.
    pub(crate) h: i32,
    /// The drawing operations used to render this text, read-only.
    pub(crate) ops: Vec<DrawOperation>,
    /// Substrings representing clusters of glyphs in the string,
    /// read-only
    pub(crate) clusters: Vec<SubString>,
    /// Custom properties associated with this text, read-only. This field
    /// is created as-needed using TTF_GetTextProperties() and the
    /// properties may be then set and read normally
    pub(crate) props: Option<Properties>,
    /// True if the engine text needs to be updated
    pub(crate) needs_engine_update: bool,
    /// The engine used to render this text, read-only.
    pub(crate) engine: Option<Rc<dyn TextEngine>>,
    /// The implementation-specific representation of this text
    pub(crate) engine_text: Option<Box<dyn Any>>,
}

impl std::fmt::Debug for TextData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextData")
            .field(
                "text",
                &self.text.as_ref().map(|t| String::from_utf8_lossy(t)),
            )
            .field("num_lines", &self.num_lines)
            .field("w", &self.w)
            .field("h", &self.h)
            .finish()
    }
}

impl TextData {
    /// The font used by this text.
    pub fn font(&self) -> Option<FontHandle> {
        self.font.clone().map(FontHandle)
    }
    /// The color of the text.
    pub fn color(&self) -> FColor {
        self.color
    }
    /// The x offset of the upper left corner of this text, in pixels.
    pub fn x(&self) -> i32 {
        self.x
    }
    /// The y offset of the upper left corner of this text, in pixels.
    pub fn y(&self) -> i32 {
        self.y
    }
    /// The width of this text, in pixels.
    pub fn w(&self) -> i32 {
        self.w
    }
    /// The height of this text, in pixels.
    pub fn h(&self) -> i32 {
        self.h
    }
    /// The drawing operations used to render this text.
    pub fn ops(&self) -> &[DrawOperation] {
        &self.ops
    }
    /// Substrings representing clusters of glyphs in the string.
    pub fn clusters(&self) -> &[SubString] {
        &self.clusters
    }
    /// The string of the text (`text`), as bytes.
    pub fn text(&self) -> Option<&[u8]> {
        self.text.as_deref()
    }
    /// The number of lines in the text, 0 if it's empty.
    pub fn num_lines(&self) -> i32 {
        self.num_lines
    }
    /// The implementation-specific representation of this text.
    pub fn engine_text(&self) -> Option<&dyn Any> {
        self.engine_text.as_deref()
    }
    /// The implementation-specific representation of this text.
    pub fn engine_text_mut(&mut self) -> Option<&mut dyn Any> {
        self.engine_text.as_deref_mut()
    }
}

/// Text created with [`Text::new`]. Translation of `TTF_Text`; dropping
/// it is `TTF_DestroyText()`.
pub struct Text {
    pub(crate) rc: Rc<RefCell<TextData>>,
}

impl std::fmt::Debug for Text {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Text").field(&self.rc.borrow()).finish()
    }
}

/// `Draw_Line_TextEngine`
#[allow(clippy::too_many_arguments)]
fn draw_line_text_engine(
    direction: Direction,
    width: i32,
    height: i32,
    column: i32,
    row: i32,
    line_width: i32,
    line_thickness: i32,
    ops: &mut Vec<DrawOperation>,
) {
    let mut line_thickness = line_thickness;
    let mut line_width = line_width;
    let tmp = row + line_thickness - height;

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

    // Wrapped mode with an unbroken line: 'line_width' is greater that 'width'
    line_width = std::cmp::min(line_width, width);

    ops.push(DrawOperation::Fill(FillOperation {
        rect: Rect::new(column, row, line_width, line_thickness),
    }));
}

/// `Render_Line_TextEngine`
#[allow(clippy::too_many_arguments)]
fn render_line_text_engine(
    this: &Weak<RefCell<FontData>>,
    font: &mut FontData,
    direction: Direction,
    xstart: i32,
    ystart: i32,
    width: i32,
    height: i32,
    ops: &mut Vec<DrawOperation>,
    clusters: &mut Vec<SubString>,
    cluster_offset: i32,
    line_index: i32,
) -> Result<()> {
    let mut last_offset: i32 = -1;
    let mut cluster: Option<usize> = None;
    let mut bounds = Rect::new(xstart, ystart, 0, font.height);

    let positions = font.current_positions().cloned().unwrap_or_default();
    let outline = font.outline;

    for (i, pos) in positions.pos.iter().enumerate() {
        let glyph_font = &pos.font;
        let idx = pos.index;
        let mut x = pos.x;
        let mut y = pos.y;
        let offset = pos.offset;

        // the glyph (`pos->glyph`) and its font's SDF setting
        let Some((sz_left, sz_top, sz_width, sz_rows, glyph_render_sdf)) =
            with_font_data(this, font, glyph_font, |f| {
                f.glyphs
                    .get(&idx)
                    .map(|g| (g.sz_left, g.sz_top, g.sz_width, g.sz_rows, f.render_sdf))
            })
            .flatten()
        else {
            return Err(Error::new("Font has been closed"));
        };

        let mut glyph_x = 0;
        let mut glyph_y = 0;
        let mut glyph_width = sz_width;
        let mut glyph_rows = sz_rows;

        // Position updated after glyph rendering
        x = xstart + ft_floor(x) + sz_left;
        y = ystart + ft_floor(y) - sz_top;

        if !glyph_render_sdf {
            // Make sure glyph is inside text area
            let above_w = x + glyph_width - width;
            let above_h = y + glyph_rows - height;

            if x < 0 {
                let tmp = -x;
                x = 0;
                glyph_x += tmp;
                glyph_width -= tmp;
            }
            if above_w > 0 {
                glyph_width -= above_w;
            }
            if y < 0 {
                let tmp = -y;
                y = 0;
                glyph_y += tmp;
                glyph_rows -= tmp;
            }
            if above_h > 0 {
                glyph_rows -= above_h;
            }
        }

        if glyph_width > 0 && glyph_rows > 0 {
            let src = Rect::new(
                glyph_x,
                glyph_y,
                glyph_width + 2 * outline,
                glyph_rows + 2 * outline,
            );
            let mut dst = Rect::new(x, y, src.w, src.h);
            if glyph_render_sdf {
                dst.x -= DEFAULT_SDF_SPREAD;
                dst.y -= DEFAULT_SDF_SPREAD;
                dst.w -= DEFAULT_SDF_SPREAD;
                dst.h -= DEFAULT_SDF_SPREAD;
            }
            ops.push(DrawOperation::Copy(CopyOperation {
                text_offset: offset,
                glyph_font: FontHandle(glyph_font.clone()),
                glyph_index: idx,
                src,
                dst,
            }));
        } else {
            // Use the distance to the next glyph as our bounds width
            glyph_width = ft_floor(pos.x_advance) + 2 * outline;
        }

        bounds.x = x;
        bounds.w = glyph_width;

        if offset != last_offset {
            let mut c = SubString {
                offset: cluster_offset + offset,
                line_index,
                ..SubString::default()
            };
            if direction == Direction::Invalid {
                if last_offset == -1 {
                    if i < positions.pos.len() - 1 {
                        let next = &positions.pos[i + 1];
                        if offset < next.offset {
                            c.flags = Direction::Ltr as u32;
                        } else {
                            c.flags = Direction::Rtl as u32;
                        }
                    } else {
                        c.flags = Direction::Invalid as u32;
                    }
                } else if offset > last_offset {
                    c.flags = Direction::Ltr as u32;
                } else {
                    c.flags = Direction::Rtl as u32;
                }
            } else {
                c.flags = direction as u32;
            }
            c.rect = bounds;
            clusters.push(c);
            cluster = Some(clusters.len() - 1);
            last_offset = offset;
        } else if let Some(c) = cluster {
            rect_union_into(&mut clusters[c].rect, &bounds);
        }
    }

    Ok(())
}

/// `AddFontTextReference`
fn add_font_text_reference(font: &FontRc, text: &Rc<RefCell<TextData>>) {
    let mut font = font.borrow_mut();
    let weak = Rc::downgrade(text);
    if !font.text.iter().any(|t| Weak::ptr_eq(t, &weak)) {
        font.text.push(weak);
    }
}

/// `RemoveFontTextReference`
fn remove_font_text_reference(font: &FontRc, text: &Rc<RefCell<TextData>>) {
    let weak = Rc::downgrade(text);
    if let Ok(mut font) = font.try_borrow_mut() {
        font.text.retain(|t| !Weak::ptr_eq(t, &weak));
    }
}

impl Text {
    /// Create a text object from UTF-8 text and a text engine.
    /// Translation of `TTF_CreateText()`.
    pub fn new(
        engine: Option<Rc<dyn TextEngine>>,
        font: Option<&Font>,
        text: &str,
    ) -> Result<Text> {
        Text::new_bytes(engine, font, text.as_bytes())
    }

    /// [`Text::new`] with the text as bytes (`text` and `length` of
    /// `TTF_CreateText()`; a NUL ends it).
    pub fn new_bytes(
        engine: Option<Rc<dyn TextEngine>>,
        font: Option<&Font>,
        text: &[u8],
    ) -> Result<Text> {
        let mut data = TextData {
            text: None,
            num_lines: 0,
            font: font.map(|f| f.weak()),
            color: FColor {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            needs_layout_update: true,
            layout: TextLayout::default(),
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            ops: Vec::new(),
            clusters: Vec::new(),
            props: None,
            needs_engine_update: false,
            engine,
            engine_text: None,
        };

        let length = text.iter().position(|&c| c == 0).unwrap_or(text.len());
        if length > 0 {
            data.text = Some(text[..length].to_vec());
        }

        let result = Text {
            rc: Rc::new(RefCell::new(data)),
        };

        if let Some(font) = font {
            add_font_text_reference(&font.rc, &result.rc);
        }

        Ok(result)
    }

    /// The data of this text, as text engines see it.
    pub fn with_data<R>(&self, f: impl FnOnce(&TextData) -> R) -> R {
        f(&self.rc.borrow())
    }

    /// The string of the text, as bytes (C's `text->text`).
    pub fn text_bytes(&self) -> Option<Vec<u8>> {
        self.rc.borrow().text.clone()
    }

    /// The string of the text (with any invalid UTF-8 replaced).
    pub fn text(&self) -> Option<String> {
        self.rc
            .borrow()
            .text
            .as_ref()
            .map(|t| String::from_utf8_lossy(t).into_owned())
    }

    /// The number of lines in the text, 0 if it's empty (as of the last
    /// layout).
    pub fn num_lines(&self) -> i32 {
        self.rc.borrow().num_lines
    }
}

/// `SortClusters`
fn sort_clusters(a: &SubString, b: &SubString) -> i32 {
    if a.offset == b.offset {
        if a.flags & SUBSTRING_LINE_END != 0 {
            return -1;
        }
        if b.flags & SUBSTRING_LINE_END != 0 {
            return 1;
        }
        return 0;
    }
    a.offset - b.offset
}

/// `CalculateClusterLengths`
fn calculate_cluster_lengths(
    text: &TextData,
    font: &FontData,
    clusters: &mut [SubString],
    length: usize,
    lines: &mut [i32],
) -> usize {
    let num_clusters = clusters.len();
    crate::qsort::sdl_qsort(clusters, |a, b| sort_clusters(a, b).cmp(&0));

    let mut last: Option<usize> = None;
    let mut cluster_index: usize = 0;

    for i in 0..num_clusters {
        let src = clusters[i];

        // Merge zero width clusters
        if let Some(l) = last {
            if src.offset == clusters[l].offset && clusters[l].flags & SUBSTRING_LINE_END == 0 {
                clusters[l].flags |= src.flags;
                let mut r = clusters[l].rect;
                rect_union_into(&mut r, &src.rect);
                clusters[l].rect = r;
                continue;
            }
        }

        let ci = cluster_index;
        if i != ci {
            clusters[ci] = src;
        }
        if last.is_none() || clusters[ci].line_index != clusters[last.unwrap()].line_index {
            if last.is_none() {
                clusters[ci].flags |= SUBSTRING_TEXT_START;
            }
            clusters[ci].flags |= SUBSTRING_LINE_START;
            if !lines.is_empty() && clusters[ci].line_index > 0 {
                lines[clusters[ci].line_index as usize - 1] = ci as i32;
            }
        }
        clusters[ci].cluster_index = ci as i32;
        cluster_index += 1;

        if clusters[ci].flags & SUBSTRING_LINE_END != 0 {
            if clusters[ci].flags & SUBSTRING_LINE_START != 0 {
                clusters[ci].rect.y = clusters[ci].line_index * font.lineskip;
                clusters[ci].rect.h = font.height;
            } else {
                let mut r = clusters[clusters[ci].cluster_index as usize - 1].rect;
                if (clusters[ci].flags & SUBSTRING_DIRECTION_MASK) != Direction::Rtl as u32 {
                    r.x += r.w;
                }
                r.w = 0;
                clusters[ci].rect = r;
            }
        } else if clusters[ci].flags & SUBSTRING_TEXT_END != 0 {
            if let Some(l) = last {
                let lc = clusters[l];
                let t = text.text.as_deref().unwrap_or(&[]);
                if lc.length > 0 && t.get((lc.offset + lc.length - 1) as usize) == Some(&b'\n') {
                    clusters[ci].line_index = lc.line_index + 1;
                    clusters[ci].rect.y = clusters[ci].line_index * font.lineskip;
                    clusters[ci].rect.h = font.height;
                } else {
                    clusters[ci].line_index = lc.line_index;
                    let mut r = lc.rect;
                    if (clusters[ci].flags & SUBSTRING_DIRECTION_MASK) != Direction::Rtl as u32 {
                        r.x += r.w;
                    }
                    r.w = 0;
                    clusters[ci].rect = r;
                }
            } else {
                clusters[ci].rect.h = font.height;
            }
        }

        if i < num_clusters - 1 {
            clusters[ci].length = clusters[i + 1].offset - clusters[ci].offset;
        } else {
            debug_assert!(clusters[ci].flags & SUBSTRING_TEXT_END != 0);
            debug_assert!(clusters[ci].offset == length as i32);
        }
        last = Some(ci);
    }

    cluster_index
}

/// `GetPreviousClusterDirection`
fn get_previous_cluster_direction(
    clusters: &[SubString],
    num_clusters: usize,
    direction: Direction,
) -> SubStringFlags {
    if num_clusters > 1 {
        clusters[num_clusters - 1].flags & SUBSTRING_DIRECTION_MASK
    } else {
        direction as SubStringFlags
    }
}

/// `TTF_GetTextDirection` on the text's data
fn text_direction(text: &TextData) -> Direction {
    if text.layout.direction != Direction::Invalid {
        return text.layout.direction;
    }
    match text.font.as_ref().and_then(|f| f.upgrade()) {
        Some(font) => font.borrow().direction,
        None => Direction::Invalid,
    }
}

/// `TTF_GetTextScript` on the text's data
fn text_script(text: &TextData) -> u32 {
    if text.layout.script != 0 {
        return text.layout.script;
    }
    match text.font.as_ref().and_then(|f| f.upgrade()) {
        Some(font) => font.borrow().script,
        None => 0,
    }
}

/// The result of `LayoutText`
struct Layout {
    num_lines: i32,
    w: i32,
    h: i32,
    ops: Vec<DrawOperation>,
    clusters: Vec<SubString>,
    lines: Vec<i32>,
}

/// `LayoutText` (`Ok(None)` when the text has no layout: the wrapped lines
/// couldn't be had)
fn layout_text(text: &TextData) -> Result<Option<Layout>> {
    let Some(font_rc) = text.font.as_ref().and_then(|f| f.upgrade()) else {
        return Ok(None);
    };
    let this = Rc::downgrade(&font_rc);
    let direction = text_direction(text);
    let script = text_script(text);

    let mut fd = font_rc.borrow_mut();
    let font = &mut *fd;

    let wrap_width = text.layout.wrap_length;
    let trim_whitespace = !text.layout.wrap_whitespace_visible;
    let t = text.text.as_deref().unwrap_or(&[]);
    let length = t.len();
    let mut extra_ops = 0;
    let mut lines: Vec<i32> = Vec::new();

    let Ok((str_lines, width, mut height)) = get_wrapped_lines(
        &this,
        font,
        t,
        length,
        direction,
        script,
        text.x,
        wrap_width,
        trim_whitespace,
        false,
    ) else {
        return Ok(None);
    };
    let num_lines = str_lines.len() as i32;

    height += text.y;

    if ttf_handle_style_underline(font) {
        extra_ops += 1;
    }
    if ttf_handle_style_strikethrough(font) {
        extra_ops += 1;
    }

    if num_lines > 1 {
        if lines.try_reserve_exact(num_lines as usize - 1).is_err() {
            return Err(Error::out_of_memory());
        }
        lines.resize(num_lines as usize - 1, -1);
    }

    let mut ops: Vec<DrawOperation> = Vec::new();
    let mut clusters: Vec<SubString> = Vec::new();
    if clusters.try_reserve(num_lines as usize + 1).is_err() {
        return Err(Error::out_of_memory());
    }

    // Render each line
    for (i, line) in str_lines.iter().enumerate() {
        let i = i as i32;

        if line.length == 0 {
            let flags = get_previous_cluster_direction(&clusters, clusters.len(), direction)
                | SUBSTRING_LINE_END;
            clusters.push(SubString {
                flags,
                offset: line.text as i32,
                line_index: i,
                ..SubString::default()
            });
            continue;
        }

        // Initialize xstart, ystart and compute positions
        let sub = &t[line.text.min(t.len())..];
        let s = ttf_size_internal(
            &this,
            font,
            sub,
            line.length,
            direction,
            script,
            false,
            0,
            false,
        )?;
        let (line_width, xstart) = (s.w, s.xstart);

        // Move to i-th line
        let mut ystart = s.ystart + i * font.lineskip;
        ystart += text.y;

        // Control left/right/center align of each bit of text
        let mut xoffset = match font.horizontal_align {
            HorizontalAlignment::Right => width - line_width,
            HorizontalAlignment::Center => (width - line_width) / 2,
            _ => 0,
        };
        xoffset = std::cmp::max(0, xoffset);
        if i == 0 {
            xoffset += text.x;
        }

        // Allocate space for the operations on this line
        let additional_ops = font.current_positions().map_or(0, |p| p.pos.len()) + extra_ops;
        if ops.try_reserve(additional_ops).is_err() {
            return Err(Error::out_of_memory());
        }

        // Allocate space for the clusters on this line
        let num = font
            .current_positions()
            .map_or(0, |p| p.num_clusters as usize);
        if clusters.try_reserve(num).is_err() {
            return Err(Error::out_of_memory());
        }
        let cluster_offset = line.text as i32;

        // Create the text drawing operations
        render_line_text_engine(
            &this,
            font,
            direction,
            xstart + xoffset,
            ystart,
            width,
            height,
            &mut ops,
            &mut clusters,
            cluster_offset,
            i,
        )?;

        let flags = get_previous_cluster_direction(&clusters, clusters.len(), direction)
            | SUBSTRING_LINE_END;
        clusters.push(SubString {
            flags,
            offset: (line.text + line.length) as i32,
            line_index: i,
            ..SubString::default()
        });

        // Apply underline or strikethrough style, if needed
        if ttf_handle_style_underline(font) {
            draw_line_text_engine(
                direction,
                width,
                height,
                xoffset,
                ystart + font.underline_top_row,
                line_width,
                font.line_thickness,
                &mut ops,
            );
        }

        if ttf_handle_style_strikethrough(font) {
            draw_line_text_engine(
                direction,
                width,
                height,
                xoffset,
                ystart + font.strikethrough_top_row,
                line_width,
                font.line_thickness,
                &mut ops,
            );
        }
    }

    let flags =
        get_previous_cluster_direction(&clusters, clusters.len(), direction) | SUBSTRING_TEXT_END;
    clusters.push(SubString {
        flags,
        offset: length as i32,
        ..SubString::default()
    });

    let num_clusters = calculate_cluster_lengths(text, font, &mut clusters, length, &mut lines);
    clusters.truncate(num_clusters);

    Ok(Some(Layout {
        num_lines,
        w: width,
        h: height,
        ops,
        clusters,
        lines,
    }))
}

/// `DestroyEngineText`
fn destroy_engine_text(text: &mut TextData) {
    if let Some(engine) = text.engine.clone() {
        if let Some(engine_text) = text.engine_text.take() {
            engine.destroy_text(engine_text);
        }
    }
}

/// `CreateEngineText`
fn create_engine_text(text: &mut TextData) -> Result<()> {
    if let Some(engine) = text.engine.clone() {
        if !text.ops.is_empty() {
            text.engine_text = Some(engine.create_text(text)?);
        }
    }
    Ok(())
}

impl Text {
    /// Get the properties associated with a text object. Translation of
    /// `TTF_GetTextProperties()`.
    pub fn properties(&self) -> Properties {
        let mut text = self.rc.borrow_mut();
        text.props.get_or_insert_with(Properties::new).clone()
    }

    /// Set the text engine used by a text object. Translation of
    /// `TTF_SetTextEngine()`.
    pub fn set_engine(&self, engine: Option<Rc<dyn TextEngine>>) -> Result<()> {
        let mut text = self.rc.borrow_mut();

        let same = match (&engine, &text.engine) {
            (None, None) => true,
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            _ => false,
        };
        if same {
            return Ok(());
        }

        destroy_engine_text(&mut text);
        text.engine = engine;
        text.needs_engine_update = true;
        Ok(())
    }

    /// Get the text engine used by a text object. Translation of
    /// `TTF_GetTextEngine()`.
    pub fn engine(&self) -> Option<Rc<dyn TextEngine>> {
        self.rc.borrow().engine.clone()
    }

    /// Set the font used by a text object. Translation of
    /// `TTF_SetTextFont()`.
    pub fn set_font(&self, font: Option<&Font>) -> Result<()> {
        set_text_font(&self.rc, font.map(|f| &f.rc));
        Ok(())
    }

    /// Get the font used by a text object. Translation of
    /// `TTF_GetTextFont()`.
    pub fn font(&self) -> Option<FontHandle> {
        self.rc.borrow().font()
    }

    /// Set the direction to be used for text shaping a text object (without
    /// HarfBuzz, only left to right). Translation of
    /// `TTF_SetTextDirection()`.
    pub fn set_direction(&self, direction: Direction) -> Result<()> {
        let mut text = self.rc.borrow_mut();

        if direction == text.layout.direction {
            return Ok(());
        }

        /* !TTF_USE_HARFBUZZ */
        if direction != Direction::Invalid && direction != Direction::Ltr {
            return Err(Error::unsupported());
        }

        text.layout.direction = direction;
        text.needs_layout_update = true;
        Ok(())
    }

    /// Get the direction to be used for text shaping a text object.
    /// Translation of `TTF_GetTextDirection()`.
    pub fn direction(&self) -> Direction {
        text_direction(&self.rc.borrow())
    }

    /// Set the script to be used for text shaping a text object
    /// (unsupported without HarfBuzz). Translation of
    /// `TTF_SetTextScript()`.
    pub fn set_script(&self, _script: u32) -> Result<()> {
        /* !TTF_USE_HARFBUZZ */
        Err(Error::unsupported())
    }

    /// Get the script used for text shaping a text object. Translation of
    /// `TTF_GetTextScript()`.
    pub fn script(&self) -> u32 {
        text_script(&self.rc.borrow())
    }

    /// Set the color of a text object. Translation of
    /// `TTF_SetTextColor()`.
    pub fn set_color(&self, r: u8, g: u8, b: u8, a: u8) -> Result<()> {
        let f_r = r as f32 / 255.0;
        let f_g = g as f32 / 255.0;
        let f_b = b as f32 / 255.0;
        let f_a = a as f32 / 255.0;

        self.set_color_float(f_r, f_g, f_b, f_a)
    }

    /// Set the color of a text object. Translation of
    /// `TTF_SetTextColorFloat()`.
    pub fn set_color_float(&self, r: f32, g: f32, b: f32, a: f32) -> Result<()> {
        let mut text = self.rc.borrow_mut();
        text.color = FColor { r, g, b, a };
        Ok(())
    }

    /// Get the color of a text object, as `(r, g, b, a)`. Translation of
    /// `TTF_GetTextColor()`.
    pub fn color(&self) -> (u8, u8, u8, u8) {
        let (f_r, f_g, f_b, f_a) = self.color_float();
        let conv = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        (conv(f_r), conv(f_g), conv(f_b), conv(f_a))
    }

    /// Get the color of a text object, as `(r, g, b, a)`. Translation of
    /// `TTF_GetTextColorFloat()`.
    pub fn color_float(&self) -> (f32, f32, f32, f32) {
        let color = self.rc.borrow().color;
        (color.r, color.g, color.b, color.a)
    }

    /// Set the position of a text object. Translation of
    /// `TTF_SetTextPosition()`.
    pub fn set_position(&self, x: i32, y: i32) -> Result<()> {
        let mut text = self.rc.borrow_mut();

        if x != text.x || y != text.y {
            text.x = x;
            text.y = y;
            text.needs_layout_update = true;
        }
        Ok(())
    }

    /// Get the position of a text object. Translation of
    /// `TTF_GetTextPosition()`.
    pub fn position(&self) -> (i32, i32) {
        let text = self.rc.borrow();
        (text.x, text.y)
    }

    /// Set whether wrapping is enabled on a text object. Translation of
    /// `TTF_SetTextWrapWidth()`.
    pub fn set_wrap_width(&self, wrap_width: i32) -> Result<()> {
        let mut text = self.rc.borrow_mut();

        if wrap_width == text.layout.wrap_length {
            return Ok(());
        }

        text.layout.wrap_length = std::cmp::max(wrap_width, 0);
        text.needs_layout_update = true;
        Ok(())
    }

    /// Get whether wrapping is enabled on a text object. Translation of
    /// `TTF_GetTextWrapWidth()`.
    pub fn wrap_width(&self) -> i32 {
        self.rc.borrow().layout.wrap_length
    }

    /// Set whether whitespace should be visible when wrapping a text
    /// object. Translation of `TTF_SetTextWrapWhitespaceVisible()`.
    pub fn set_wrap_whitespace_visible(&self, visible: bool) -> Result<()> {
        let mut text = self.rc.borrow_mut();

        if visible == text.layout.wrap_whitespace_visible {
            return Ok(());
        }

        text.layout.wrap_whitespace_visible = visible;
        text.needs_layout_update = true;
        Ok(())
    }

    /// Return whether whitespace is shown when wrapping a text object.
    /// Translation of `TTF_TextWrapWhitespaceVisible()`.
    pub fn wrap_whitespace_visible(&self) -> bool {
        self.rc.borrow().layout.wrap_whitespace_visible
    }

    /// Set the UTF-8 text used by a text object. Translation of
    /// `TTF_SetTextString()`.
    pub fn set_string(&self, string: Option<&str>) -> Result<()> {
        self.set_string_bytes(string.map(str::as_bytes))
    }

    /// [`Text::set_string`] with the string as bytes (a NUL ends it).
    pub fn set_string_bytes(&self, string: Option<&[u8]>) -> Result<()> {
        let mut text = self.rc.borrow_mut();
        set_text_string(&mut text, string)
    }

    /// Insert UTF-8 text into a text object (at a byte offset; -1 appends).
    /// Translation of `TTF_InsertTextString()`.
    pub fn insert_string(&self, offset: i32, string: &str) -> Result<()> {
        self.insert_string_bytes(offset, string.as_bytes())
    }

    /// [`Text::insert_string`] with the string as bytes (a NUL ends it).
    pub fn insert_string_bytes(&self, offset: i32, string: &[u8]) -> Result<()> {
        let mut text = self.rc.borrow_mut();
        let mut offset = offset;

        let length = string.iter().position(|&c| c == 0).unwrap_or(string.len());
        if length == 0 {
            return Ok(());
        }
        let string = &string[..length];

        let Some(old) = text.text.as_mut() else {
            return set_text_string(&mut text, Some(string));
        };

        let old_length = old.len() as i32;
        if old.try_reserve(length).is_err() {
            return Err(Error::out_of_memory());
        }
        if offset < 0 {
            offset += old_length + 1;
            if offset < 0 {
                offset = 0;
            }
        } else if offset > old_length {
            offset = old_length;
        }
        let offset = offset as usize;
        old.splice(offset..offset, string.iter().copied());

        text.needs_layout_update = true;
        Ok(())
    }

    /// Append UTF-8 text to a text object. Translation of
    /// `TTF_AppendTextString()`.
    pub fn append_string(&self, string: &str) -> Result<()> {
        self.insert_string(-1, string)
    }

    /// Delete UTF-8 text from a text object (`length` bytes at a byte
    /// offset; a negative length deletes to the end). Translation of
    /// `TTF_DeleteTextString()`.
    pub fn delete_string(&self, offset: i32, length: i32) -> Result<()> {
        let mut text = self.rc.borrow_mut();
        let mut offset = offset;

        if length == 0 {
            return Ok(());
        }
        let Some(old) = text.text.as_mut() else {
            return Ok(());
        };

        let old_length = old.len() as i32;
        if offset < 0 {
            offset += old_length + 1;
            if offset < 0 {
                offset = 0;
            }
        } else if offset >= old_length {
            return Ok(());
        }

        if length < 0 || length >= (old_length - offset) {
            if offset == 0 {
                return set_text_string(&mut text, None);
            }
            old.truncate(offset as usize);
        } else {
            let start = offset as usize;
            old.drain(start..start + length as usize);
        }

        text.needs_layout_update = true;
        Ok(())
    }

    /// Get the size of a text object, as `(w, h)`. Translation of
    /// `TTF_GetTextSize()`.
    pub fn size(&self) -> Result<(i32, i32)> {
        self.update()?;
        let text = self.rc.borrow();
        Ok((text.w, text.h))
    }

    /// Get the substring of a text object that surrounds a text offset.
    /// Translation of `TTF_GetTextSubString()`.
    pub fn substring(&self, offset: i32) -> Result<SubString> {
        self.update()?;
        Ok(get_text_substring(&self.rc.borrow(), offset))
    }

    /// Get the substring of a text object that contains the given line.
    /// Translation of `TTF_GetTextSubStringForLine()`.
    pub fn substring_for_line(&self, line: i32) -> Result<SubString> {
        let mut substring = SubString::default();

        self.update()?;
        let text = self.rc.borrow();

        if text.clusters.is_empty() {
            substring.rect.h = text.layout.font_height;
            return Ok(substring);
        }

        let num_clusters = text.clusters.len();
        let clusters = &text.clusters;
        if line < 0 {
            substring = clusters[0];
            substring.length = 0;
            substring.rect.w = 0;
            return Ok(substring);
        }

        if line >= text.num_lines {
            return Ok(clusters[num_clusters - 1]);
        }

        let lines = &text.layout.lines;
        if line == 0 {
            substring = clusters[0];
        } else {
            substring = clusters[lines[line as usize - 1] as usize];
        }
        if line == text.num_lines - 1 {
            substring.length = text.text.as_ref().map_or(0, |t| t.len()) as i32 - substring.offset;
        } else {
            substring.length = clusters[lines[line as usize] as usize].offset - substring.offset;
        }

        for cluster in &clusters[(substring.cluster_index + 1) as usize..num_clusters] {
            if cluster.line_index != line {
                break;
            }
            substring.flags |= cluster.flags;
            rect_union_into(&mut substring.rect, &cluster.rect);
        }
        Ok(substring)
    }

    /// Get all substrings of a text object (`length` bytes from a byte
    /// offset; a negative length goes to the end). Translation of
    /// `TTF_GetTextSubStringsForRange()`.
    pub fn substrings_for_range(&self, offset: i32, length: i32) -> Result<Vec<SubString>> {
        let mut length = length;

        self.update()?;
        let text = self.rc.borrow();

        if text.clusters.is_empty() {
            let substring = SubString {
                rect: Rect::new(0, 0, 0, text.layout.font_height),
                ..SubString::default()
            };
            return Ok(vec![substring]);
        }

        if length < 0 {
            length = text.text.as_ref().map_or(0, |t| t.len()) as i32;
        }

        let offset1 = offset;
        let offset2 = offset + length;
        let substring1 = get_text_substring(&text, offset1);
        let substring2 = get_text_substring(&text, offset2);
        let substring2 = get_previous_text_substring(&text, &substring2)?;

        if substring1.cluster_index == substring2.cluster_index {
            let mut substring = substring1;
            if length == 0 {
                substring.length = 0;
                if (substring.flags & SUBSTRING_DIRECTION_MASK) != Direction::Rtl as u32 {
                    substring.rect.x += substring.rect.w;
                }
                substring.rect.w = 0;
            }
            return Ok(vec![substring]);
        }

        // Build a list of contiguous substrings
        let clusters = &text.clusters;
        let mut num_results = 1;
        let mut last = &clusters[substring1.cluster_index as usize];
        for cluster in
            (substring1.cluster_index + 1..=substring2.cluster_index).map(|i| &clusters[i as usize])
        {
            if cluster.line_index != last.line_index {
                num_results += 1;
                last = cluster;
            }
        }

        let mut result: Vec<SubString> = Vec::new();
        if result.try_reserve_exact(num_results).is_err() {
            return Err(Error::out_of_memory());
        }

        let mut substring = substring1;
        for cluster in
            (substring1.cluster_index + 1..=substring2.cluster_index).map(|i| &clusters[i as usize])
        {
            if cluster.line_index == substring.line_index {
                substring.flags |= cluster.flags;
                rect_union_into(&mut substring.rect, &cluster.rect);
            } else {
                substring.length = cluster.offset - substring.offset;
                result.push(substring);
                substring = *cluster;
            }
        }
        substring.length = (substring2.offset - substring.offset) + substring2.length;
        result.push(substring);

        Ok(result)
    }

    /// Get the portion of a text object that is closest to a point.
    /// Translation of `TTF_GetTextSubStringForPoint()`.
    pub fn substring_for_point(&self, x: i32, y: i32) -> Result<SubString> {
        let mut substring = SubString::default();

        self.update()?;
        let text = self.rc.borrow();

        if text.clusters.is_empty() {
            substring.rect.h = text.layout.font_height;
            return Ok(substring);
        }

        let direction = text_direction(&text);
        let prefer_row = direction != Direction::Ttb && direction != Direction::Btt;
        let mut closest: Option<&SubString> = None;
        let mut closest_dist = i32::MAX;
        let wrap_cost = 100;
        let point = Point::new(x, y);
        for cluster in &text.clusters {
            let center_x = cluster.rect.x + cluster.rect.w / 2;
            let center_y = cluster.rect.y + cluster.rect.h / 2;

            if cluster.flags & SUBSTRING_LINE_END != 0 {
                if prefer_row && (cluster.flags & SUBSTRING_LINE_END) != 0 {
                    let line_ends_left =
                        (cluster.flags & SUBSTRING_DIRECTION_MASK) == Direction::Rtl as u32;
                    if (y >= cluster.rect.y && y < (cluster.rect.y + cluster.rect.h))
                        && ((!line_ends_left && x >= cluster.rect.x)
                            || (line_ends_left && x <= cluster.rect.x))
                    {
                        closest = Some(cluster);
                        break;
                    }
                }
            } else {
                if prefer_row && (cluster.flags & SUBSTRING_LINE_START) != 0 {
                    let line_ends_left =
                        (cluster.flags & SUBSTRING_DIRECTION_MASK) == Direction::Rtl as u32;
                    if (y >= cluster.rect.y && y < (cluster.rect.y + cluster.rect.h))
                        && ((!line_ends_left && x < cluster.rect.x)
                            || (line_ends_left && x > cluster.rect.x))
                    {
                        closest = Some(cluster);
                        break;
                    }
                }
                if cluster.rect.contains(point) {
                    closest = Some(cluster);
                    break;
                }
            }

            let dist = if prefer_row {
                (center_y - y)
                    .wrapping_abs()
                    .wrapping_mul(wrap_cost)
                    .wrapping_add((center_x - x).wrapping_abs())
            } else {
                (center_x - x)
                    .wrapping_abs()
                    .wrapping_mul(wrap_cost)
                    .wrapping_add((center_y - y).wrapping_abs())
            };
            if dist < closest_dist {
                closest = Some(cluster);
                closest_dist = dist;
            }
        }

        if let Some(closest) = closest {
            substring = *closest;
        }
        Ok(substring)
    }

    /// Get the previous substring in a text object. Translation of
    /// `TTF_GetPreviousTextSubString()`.
    pub fn previous_substring(&self, substring: &SubString) -> Result<SubString> {
        get_previous_text_substring(&self.rc.borrow(), substring)
    }

    /// Get the next substring in a text object. Translation of
    /// `TTF_GetNextTextSubString()`.
    pub fn next_substring(&self, substring: &SubString) -> Result<SubString> {
        let text = self.rc.borrow();
        let num_clusters = text.clusters.len() as i32;
        let clusters = &text.clusters;

        if substring.cluster_index < 0 || substring.cluster_index >= num_clusters {
            return Err(Error::new("Cluster index out of range"));
        }

        if substring.offset != clusters[substring.cluster_index as usize].offset {
            return Err(Error::new("Stale substring"));
        }

        if substring.cluster_index == num_clusters - 1 {
            Ok(clusters[num_clusters as usize - 1])
        } else {
            Ok(clusters[substring.cluster_index as usize + 1])
        }
    }

    /// Update the layout of a text object. Translation of
    /// `TTF_UpdateText()`.
    pub fn update(&self) -> Result<()> {
        update_text(&self.rc)
    }
}

/// `TTF_SetTextFont`
fn set_text_font(text_rc: &Rc<RefCell<TextData>>, font: Option<&FontRc>) {
    let same = {
        let text = text_rc.borrow();
        match (font, &text.font) {
            (None, None) => true,
            (Some(f), Some(t)) => std::ptr::eq(Rc::as_ptr(f), Weak::as_ptr(t)),
            _ => false,
        }
    };
    if same {
        return;
    }

    let old = text_rc.borrow().font.as_ref().and_then(|f| f.upgrade());
    if let Some(old) = old {
        remove_font_text_reference(&old, text_rc);
    }

    let mut text = text_rc.borrow_mut();
    text.font = font.map(Rc::downgrade);
    if let Some(font) = font {
        drop(text);
        add_font_text_reference(font, text_rc);
        let height = font.borrow().height;
        let mut text = text_rc.borrow_mut();
        text.layout.font_height = height;
        text.needs_layout_update = true;
    } else {
        text.layout.font_height = 0;
        text.needs_layout_update = true;
    }
}

/// `TTF_SetTextString` on the text's data
fn set_text_string(text: &mut TextData, string: Option<&[u8]>) -> Result<()> {
    let string = string.map(|s| &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]);
    match string {
        None | Some([]) => {
            if text.text.is_none() {
                return Ok(());
            }
            text.text = None;
        }
        Some(string) => {
            if text.text.as_deref() == Some(string) {
                return Ok(());
            }

            let mut new_string = Vec::new();
            if new_string.try_reserve_exact(string.len()).is_err() {
                return Err(Error::out_of_memory());
            }
            new_string.extend_from_slice(string);
            text.text = Some(new_string);
        }
    }

    text.needs_layout_update = true;
    Ok(())
}

/// `TTF_GetTextSubString` on an updated text
fn get_text_substring(text: &TextData, offset: i32) -> SubString {
    let mut substring = SubString::default();

    if text.clusters.is_empty() {
        substring.rect.h = text.layout.font_height;
        return substring;
    }

    let num_clusters = text.clusters.len() as i32;
    let clusters = &text.clusters;
    if offset < 0 {
        substring = clusters[0];
        substring.length = 0;
        substring.rect.w = 0;
        return substring;
    }

    let length = text.text.as_ref().map_or(0, |t| t.len()) as i32;
    if offset >= length {
        return clusters[num_clusters as usize - 1];
    }

    // Make a quick guess that works for ASCII text
    if offset < num_clusters {
        let mut cluster = offset as usize;
        if clusters[cluster].offset == offset {
            if (clusters[cluster].flags & SUBSTRING_LINE_END) != 0
                && clusters[cluster].length == 0
                && offset < num_clusters - 1
            {
                cluster += 1;
            }
            return clusters[cluster];
        }
    }

    // Do a binary search to find the cluster
    let mut low = 0;
    let mut high = num_clusters - 1;
    while low <= high {
        let mid = low + (high - low) / 2;
        let mut cluster = mid as usize;

        // If we're a zero length line ending, check the next cluster
        if (clusters[cluster].flags & SUBSTRING_LINE_END) != 0
            && clusters[cluster].length == 0
            && mid < num_clusters - 1
        {
            cluster += 1;
        }

        if offset >= clusters[cluster].offset
            && offset < clusters[cluster].offset + clusters[cluster].length
        {
            substring = clusters[mid as usize];
            break;
        }

        if clusters[cluster].offset < offset {
            low = mid + 1;
        } else {
            high = mid - 1;
        }
    }
    substring
}

/// `TTF_GetPreviousTextSubString`
fn get_previous_text_substring(text: &TextData, substring: &SubString) -> Result<SubString> {
    let num_clusters = text.clusters.len() as i32;
    let clusters = &text.clusters;

    if substring.cluster_index < 0 || substring.cluster_index >= num_clusters {
        return Err(Error::new("Cluster index out of range"));
    }

    if substring.offset != clusters[substring.cluster_index as usize].offset {
        return Err(Error::new("Stale substring"));
    }

    if substring.cluster_index == 0 {
        let mut previous = clusters[0];
        previous.length = 0;
        previous.rect.w = 0;
        Ok(previous)
    } else {
        Ok(clusters[substring.cluster_index as usize - 1])
    }
}

/// `TTF_UpdateText`
pub(crate) fn update_text(text_rc: &Rc<RefCell<TextData>>) -> Result<()> {
    let mut text = text_rc.borrow_mut();

    if text.needs_layout_update {
        destroy_engine_text(&mut text);
        text.needs_engine_update = true;

        text.ops = Vec::new();
        text.clusters = Vec::new();
        text.layout.lines = Vec::new();
        text.num_lines = 0;
        text.w = 0;
        text.h = 0;

        let has_font = text.font.as_ref().is_some_and(|f| f.strong_count() > 0);
        if has_font && text.text.is_some() {
            if let Some(layout) = layout_text(&text)? {
                text.num_lines = layout.num_lines;
                text.w = layout.w;
                text.h = layout.h;
                text.ops = layout.ops;
                text.clusters = layout.clusters;
                text.layout.lines = layout.lines;
            }
        }
        text.needs_layout_update = false;
    }

    if text.needs_engine_update {
        create_engine_text(&mut text)?;
        text.needs_engine_update = false;
    }
    Ok(())
}

impl Drop for Text {
    /// `TTF_DestroyText`
    fn drop(&mut self) {
        {
            let mut text = self.rc.borrow_mut();
            destroy_engine_text(&mut text);
            text.ops = Vec::new();
            text.clusters = Vec::new();
            text.layout.lines = Vec::new();
        }
        set_text_font(&self.rc, None);
        let mut text = self.rc.borrow_mut();
        text.props = None;
        text.text = None;
    }
}

/// `RemoveOneTextCallback`: the font is closing
pub(crate) fn detach_font(text: &Rc<RefCell<TextData>>, font: &FontRc) {
    let is_font = text
        .borrow()
        .font
        .as_ref()
        .is_some_and(|f| std::ptr::eq(Weak::as_ptr(f), Rc::as_ptr(font)));
    if is_font {
        set_text_font(text, None);
    } else {
        remove_font_text_reference(font, text);
    }
}
