// Rust translation of src/rtftype.h from SDL_rtf.
// This is an altered (translated) version of the original software; see LICENSE.txt.

/*
 * This file was adapted from Microsoft Rich Text Format Specification 1.6
 * http://msdn.microsoft.com/library/default.asp?url=/library/en-us/dnrtfspec/html/rtfspec.asp
 */

use std::cell::RefCell;
use std::rc::Rc;

use sdl3::render::{Renderer, Texture};
use sdl3::video::Color;

use crate::rtf::{FontEngine, FontFamily};

/// `CHP`: CHaracter Properties. The `char` fields are `i8`, as `char` is
/// signed on the platforms SDL_rtf is mostly built for (x86, Windows):
/// `\cf200` is a negative color index there.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Chp {
    pub f_font_charset: i32,
    pub f_font: i32,
    pub f_font_size: i32,
    pub f_bg_color: i8,
    pub f_fg_color: i8,
    pub f_bold: i8,
    pub f_underline: i8,
    pub f_italic: i8,
}

/// `JUST`
pub(crate) type Just = i32;
pub(crate) const JUST_L: Just = 0;
pub(crate) const JUST_R: Just = 1;
pub(crate) const JUST_C: Just = 2;
pub(crate) const JUST_F: Just = 3;

/// `PAP`: PAragraph Properties
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Pap {
    pub xa_left: i32,  /* left indent in twips */
    pub xa_right: i32, /* right indent in twips */
    pub xa_first: i32, /* first line indent in twips */
    pub just: Just,    /* justification */
}

/// `SBK`
pub(crate) type Sbk = i32;
pub(crate) const SBK_NON: Sbk = 0;
pub(crate) const SBK_COL: Sbk = 1;
pub(crate) const SBK_EVN: Sbk = 2;
pub(crate) const SBK_ODD: Sbk = 3;
pub(crate) const SBK_PG: Sbk = 4;

/// `PGN`
pub(crate) type Pgn = i32;
pub(crate) const PG_DEC: Pgn = 0;
pub(crate) const PG_U_ROM: Pgn = 1;
pub(crate) const PG_L_ROM: Pgn = 2;
pub(crate) const PG_U_LTR: Pgn = 3;
pub(crate) const PG_L_LTR: Pgn = 4;

/// `SEP`: SEction Properties
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Sep {
    pub c_cols: i32,     /* number of columns */
    pub sbk: Sbk,        /* section break type */
    pub xa_pgn: i32,     /* x position of page number in twips */
    pub ya_pgn: i32,     /* y position of page number in twips */
    pub pgn_format: Pgn, /* how the page number is formatted */
}

/// `DOP`: DOcument Properties
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Dop {
    pub xa_page: i32,    /* page width in twips */
    pub ya_page: i32,    /* page height in twips */
    pub xa_left: i32,    /* left margin in twips */
    pub ya_top: i32,     /* top margin in twips */
    pub xa_right: i32,   /* right margin in twips */
    pub ya_bottom: i32,  /* bottom margin in twips */
    pub pgn_start: i32,  /* starting page number in twips */
    pub f_facingp: i8,   /* facing pages enabled? */
    pub f_landscape: i8, /* landscape or portrait?? */
}

/// `RDS`: Rtf Destination State
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Rds {
    #[default]
    Norm,
    Skip,
    FontTable,
    ColorTable,
    Info,
    Title,
    Subject,
    Author,
}

/// `RIS`: Rtf Internal State
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Ris {
    #[default]
    Norm,
    Bin,
    Hex,
}

/// `SAVE`: property save structure (the `pNext` list is a stack, its top
/// last)
#[derive(Clone, Copy, Debug)]
pub(crate) struct Save {
    pub chp: Chp,
    pub pap: Pap,
    pub sep: Sep,
    pub dop: Dop,
    pub rds: Rds,
    pub ris: Ris,
}

/* What types of properties are there? */
/// `IPROP`
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Iprop {
    FontFamily,
    FontCharset,
    ColorRed,
    ColorGreen,
    ColorBlue,
    Font,
    FontSize,
    BgColor,
    FgColor,
    Bold,
    Italic,
    Underline,
    LeftInd,
    RightInd,
    FirstInd,
    Cols,
    PgnX,
    PgnY,
    XaPage,
    YaPage,
    XaLeft,
    XaRight,
    YaTop,
    YaBottom,
    PgnStart,
    Sbk,
    PgnFormat,
    Facingp,
    Landscape,
    Just,
    Pard,
    Plain,
    Sectd,
}

/// `ipropMax`
pub(crate) const IPROP_MAX: usize = 33;

/// `ACTN`
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Actn {
    Spec,
    Byte,
    Word,
}

/// `PROPTYPE`
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PropType {
    Chp,
    Pap,
    Sep,
    Dop,
}

/// The field a [`Prop`] sets: the `offsetof()` of the C table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Field {
    None,
    ChpFontCharset,
    ChpFont,
    ChpFontSize,
    ChpBgColor,
    ChpFgColor,
    ChpBold,
    ChpItalic,
    ChpUnderline,
    PapXaLeft,
    PapXaRight,
    PapXaFirst,
    PapJust,
    SepCCols,
    SepXaPgn,
    SepYaPgn,
    SepSbk,
    SepPgnFormat,
    DopXaPage,
    DopYaPage,
    DopXaLeft,
    DopXaRight,
    DopYaTop,
    DopYaBottom,
    DopPgnStart,
    DopFFacingp,
    DopFLandscape,
}

/// `PROP`
#[derive(Clone, Copy, Debug)]
pub(crate) struct Prop {
    pub actn: Actn,     /* size of value */
    pub prop: PropType, /* structure containing value */
    pub offset: Field,  /* offset of value from base of structure */
}

/// `IPFN`
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Ipfn {
    Bin,
    Hex,
    SkipDest,
}

/// `IDEST`
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Idest {
    FontTable,
    ColorTable,
    Info,
    Title,
    Subject,
    Author,
    #[allow(dead_code)] // (unused upstream too: \pict is idestSkip)
    Pict,
    Skip,
}

/// `KWD`, with the `idx` of `SYM` that goes with each kind
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kwd {
    /* character to print if kwd == kwdChar */
    Char(i32),
    /* index into destination table if kwd == kwdDest */
    Dest(Idest),
    /* index into property table if kwd == kwdProp */
    Prop(Iprop),
    Spec(Ipfn),
}

/// `SYM`
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sym {
    pub sz_keyword: &'static str, /* RTF keyword */
    pub dflt: i32,                /* default value to use */
    pub f_pass_dflt: bool,        /* true to use default value from this table */
    pub kwd: Kwd,                 /* base action to take */
}

/// `FFAM`
pub(crate) const FNIL: i32 = 0;
pub(crate) const FROMAN: i32 = 1;
pub(crate) const FSWISS: i32 = 2;
pub(crate) const FMODERN: i32 = 3;
pub(crate) const FSCRIPT: i32 = 4;
pub(crate) const FDECOR: i32 = 5;
pub(crate) const FTECH: i32 = 6;
pub(crate) const FBIDI: i32 = 7;

/// `RTF_Font`: `font` indexes the context's fonts (the `void *` the font
/// engine created)
#[derive(Clone, Copy, Debug)]
pub(crate) struct RtfFont {
    pub font: usize,
    pub size: i32,
    pub style: i32,
}

/// `RTF_FontEntry` (the `next` list is a `Vec`, its head first; `fonts`
/// likewise)
#[derive(Clone, Debug)]
pub(crate) struct FontEntry {
    pub number: i32,
    pub name: String,
    pub family: FontFamily,
    pub charset: i32,
    pub fonts: Vec<RtfFont>,
}

/// `RTF_ColorEntry` (`color` is the `SDL_Color` that `RTF_CreateColor()`
/// allocates)
#[derive(Clone, Copy, Debug)]
pub(crate) struct ColorEntry {
    pub color: Option<Color>,
    #[allow(dead_code)]
    pub r: u8,
    #[allow(dead_code)]
    pub g: u8,
    #[allow(dead_code)]
    pub b: u8,
    #[allow(dead_code)]
    pub a: u8, /* unused */
}

/// `RTF_TextBlock`
#[derive(Clone, Debug)]
pub(crate) struct TextBlock {
    pub font: usize,

    pub color: Option<Color>,
    pub tabs: i32,
    pub text: String,
    pub num_chars: i32,
    pub byte_offsets: Vec<i32>,
    pub pixel_offsets: Vec<i32>,
    pub line_height: i32,
}

/// `RTF_Surface` (`surface` is the texture the font engine rendered)
#[derive(Clone, Copy, Debug)]
pub(crate) struct RtfSurface {
    pub x: i32,
    pub y: i32,
    pub surface: Texture,
}

/// `RTF_Line` (the text blocks and surfaces are `Vec`s: `start`/`last`
/// and `startSurface`/`lastSurface` are their ends)
#[derive(Clone, Debug)]
pub(crate) struct Line {
    pub pap: Pap,
    pub line_width: i32,
    pub line_height: i32,
    pub tabs: i32,
    pub blocks: Vec<TextBlock>,
    pub surfaces: Vec<RtfSurface>,
}

/// An RTF display context (`RTF_Context`): a document loaded with
/// [`load`](Context::load) or [`load_io`](Context::load_io), laid out and
/// rendered through a [`FontEngine`] to an [`sdl3::render::Renderer`].
///
/// Dropping it frees the document, its fonts (through the font engine)
/// and its textures. Translation of `struct _RTF_Context`.
pub struct Context<E: FontEngine> {
    pub(crate) renderer: Rc<RefCell<Renderer>>,
    pub(crate) font_engine: E,
    /// The fonts the font engine created (the `void *` of `RTF_Font`),
    /// freed by `ecClearFonts()`
    pub(crate) fonts: Vec<Option<E::Font>>,

    /* Storage for parsing data */
    /// `data`, `datamax` bytes long; empty while `data` is `NULL`
    pub(crate) data: Vec<u8>,
    pub(crate) datapos: usize,
    pub(crate) values: [i32; 4],

    pub(crate) font_table: Vec<FontEntry>,
    pub(crate) color_table: Vec<ColorEntry>,

    pub(crate) title: Option<String>,
    pub(crate) subject: Option<String>,
    pub(crate) author: Option<String>,

    pub(crate) c_group: i32,
    pub(crate) rds: Rds,
    pub(crate) ris: Ris,

    pub(crate) chp: Chp,
    pub(crate) pap: Pap,
    pub(crate) sep: Sep,
    pub(crate) dop: Dop,

    pub(crate) psave: Vec<Save>,
    pub(crate) cb_bin: i64,
    pub(crate) l_param: i64,
    pub(crate) f_skip_dest_if_unk: bool,

    /* Input data stream (can be non-seekable): passed along while
     * loading */
    pub(crate) nextch: i32,

    /* Display information */
    pub(crate) display_width: i32,
    pub(crate) display_height: i32,
    /// The lines (`start` to `last`)
    pub(crate) lines: Vec<Line>,
}

impl<E: FontEngine> std::fmt::Debug for Context<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("title", &self.title)
            .field("subject", &self.subject)
            .field("author", &self.author)
            .field("lines", &self.lines.len())
            .field("display_width", &self.display_width)
            .field("display_height", &self.display_height)
            .finish_non_exhaustive()
    }
}
