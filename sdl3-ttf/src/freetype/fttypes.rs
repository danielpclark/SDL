// Rust translation of include/freetype/fttypes.h, ftimage.h, fterrdef.h
// and the plain types of freetype.h from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType's basic data types, the image (outline and bitmap) types, the
//! error codes and the face/size/glyph constants.
//!
//! The integer types are those of an LP64 build (as upstream's on 64-bit
//! Linux, which the tests compare against): `FT_Long`, `FT_ULong` and the
//! types built on them (`FT_Pos`, `FT_Fixed`, `FT_F26Dot6`) are 64-bit.

/// `FT_Bool`
pub type FtBool = bool;
/// `FT_FWord`
pub type FtFWord = i16;
/// `FT_UFWord`
pub type FtUFWord = u16;
/// `FT_Char`
pub type FtChar = i8;
/// `FT_Byte`
pub type FtByte = u8;
/// `FT_Tag`
pub type FtTag = u32;
/// `FT_Short`
pub type FtShort = i16;
/// `FT_UShort`
pub type FtUShort = u16;
/// `FT_Int`
pub type FtInt = i32;
/// `FT_UInt`
pub type FtUInt = u32;
/// `FT_Long` (LP64)
pub type FtLong = i64;
/// `FT_ULong` (LP64)
pub type FtULong = u64;
/// `FT_Int32`
pub type FtInt32 = i32;
/// `FT_UInt32`
pub type FtUInt32 = u32;
pub type FtUInt16 = u16;
/// `FT_F2Dot14`
pub type FtF2Dot14 = i16;
/// `FT_F26Dot6`
pub type FtF26Dot6 = i64;
/// `FT_Fixed`
pub type FtFixed = i64;
/// `FT_Pos`
pub type FtPos = i64;
/// `FT_Angle`
pub type FtAngle = i64;
/// `FT_Error`: 0 is success (`FT_Err_Ok`); the codes are the `FT_ERR_*`
/// constants. FreeType's module error bits are off
/// (`FT_CONFIG_OPTION_USE_MODULE_ERRORS` is undefined), so an error is its
/// base code.
pub type FtError = i32;

/// `FT_UnitVector`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FtUnitVector {
    pub x: FtF2Dot14,
    pub y: FtF2Dot14,
}

/// `FT_Matrix`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FtMatrix {
    pub xx: FtFixed,
    pub xy: FtFixed,
    pub yx: FtFixed,
    pub yy: FtFixed,
}

/// `FT_MAKE_TAG`
pub const fn ft_make_tag(x1: u8, x2: u8, x3: u8, x4: u8) -> FtTag {
    ((x1 as u32) << 24) | ((x2 as u32) << 16) | ((x3 as u32) << 8) | x4 as u32
}

/// `FT_ERROR_BASE`
pub const fn ft_error_base(x: FtError) -> FtError {
    x & 0xFF
}

/// `FT_ERR_EQ`
pub const fn ft_err_eq(x: FtError, e: FtError) -> bool {
    ft_error_base(x) == ft_error_base(e)
}

/// `FT_ERR_NEQ`
pub const fn ft_err_neq(x: FtError, e: FtError) -> bool {
    ft_error_base(x) != ft_error_base(e)
}

/* ftimage.h */

/// `FT_Vector`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FtVector {
    pub x: FtPos,
    pub y: FtPos,
}

/// `FT_BBox`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(non_snake_case)]
pub struct FtBBox {
    pub xMin: FtPos,
    pub yMin: FtPos,
    pub xMax: FtPos,
    pub yMax: FtPos,
}

/// `FT_Pixel_Mode`
pub type FtPixelMode = u8;
pub const FT_PIXEL_MODE_NONE: FtPixelMode = 0;
pub const FT_PIXEL_MODE_MONO: FtPixelMode = 1;
pub const FT_PIXEL_MODE_GRAY: FtPixelMode = 2;
pub const FT_PIXEL_MODE_GRAY2: FtPixelMode = 3;
pub const FT_PIXEL_MODE_GRAY4: FtPixelMode = 4;
pub const FT_PIXEL_MODE_LCD: FtPixelMode = 5;
pub const FT_PIXEL_MODE_LCD_V: FtPixelMode = 6;
pub const FT_PIXEL_MODE_BGRA: FtPixelMode = 7;
pub const FT_PIXEL_MODE_MAX: FtPixelMode = 8;

/// `FT_Bitmap`. The buffer is owned; `pitch` may be negative (rows going
/// up), in which case the first row is at the end of `buffer`, as with C's
/// `buffer` pointing at the start of the block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FtBitmap {
    pub rows: u32,
    pub width: u32,
    pub pitch: i32,
    pub buffer: Vec<u8>,
    pub num_grays: u16,
    pub pixel_mode: u8,
    pub palette_mode: u8,
}

/// `FT_Outline`. The arrays are owned, with `n_points` and `n_contours`
/// the counts in use (the vectors may be longer).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FtOutline {
    pub n_contours: i16,
    pub n_points: i16,
    pub points: Vec<FtVector>,
    pub tags: Vec<u8>,
    pub contours: Vec<i16>,
    pub flags: i32,
}

pub const FT_OUTLINE_CONTOURS_MAX: i32 = i16::MAX as i32;
pub const FT_OUTLINE_POINTS_MAX: i32 = i16::MAX as i32;

pub const FT_OUTLINE_NONE: i32 = 0x0;
pub const FT_OUTLINE_OWNER: i32 = 0x1;
pub const FT_OUTLINE_EVEN_ODD_FILL: i32 = 0x2;
pub const FT_OUTLINE_REVERSE_FILL: i32 = 0x4;
pub const FT_OUTLINE_IGNORE_DROPOUTS: i32 = 0x8;
pub const FT_OUTLINE_SMART_DROPOUTS: i32 = 0x10;
pub const FT_OUTLINE_INCLUDE_STUBS: i32 = 0x20;
pub const FT_OUTLINE_OVERLAP: i32 = 0x40;
pub const FT_OUTLINE_HIGH_PRECISION: i32 = 0x100;
pub const FT_OUTLINE_SINGLE_PASS: i32 = 0x200;

/// `FT_CURVE_TAG`
pub const fn ft_curve_tag(flag: u8) -> u8 {
    flag & 0x03
}
pub const FT_CURVE_TAG_ON: u8 = 0x01;
pub const FT_CURVE_TAG_CONIC: u8 = 0x00;
pub const FT_CURVE_TAG_CUBIC: u8 = 0x02;
pub const FT_CURVE_TAG_HAS_SCANMODE: u8 = 0x04;
pub const FT_CURVE_TAG_TOUCH_X: u8 = 0x08;
pub const FT_CURVE_TAG_TOUCH_Y: u8 = 0x10;
pub const FT_CURVE_TAG_TOUCH_BOTH: u8 = FT_CURVE_TAG_TOUCH_X | FT_CURVE_TAG_TOUCH_Y;

/// `FT_Glyph_Format`
pub type FtGlyphFormat = u32;
pub const FT_GLYPH_FORMAT_NONE: FtGlyphFormat = 0;
pub const FT_GLYPH_FORMAT_COMPOSITE: FtGlyphFormat = ft_make_tag(b'c', b'o', b'm', b'p');
pub const FT_GLYPH_FORMAT_BITMAP: FtGlyphFormat = ft_make_tag(b'b', b'i', b't', b's');
pub const FT_GLYPH_FORMAT_OUTLINE: FtGlyphFormat = ft_make_tag(b'o', b'u', b't', b'l');
pub const FT_GLYPH_FORMAT_PLOTTER: FtGlyphFormat = ft_make_tag(b'p', b'l', b'o', b't');
pub const FT_GLYPH_FORMAT_SVG: FtGlyphFormat = ft_make_tag(b'S', b'V', b'G', b' ');

/// `FT_Span`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FtSpan {
    pub x: i16,
    pub len: u16,
    pub coverage: u8,
}

pub const FT_RASTER_FLAG_DEFAULT: i32 = 0x0;
pub const FT_RASTER_FLAG_AA: i32 = 0x1;
pub const FT_RASTER_FLAG_DIRECT: i32 = 0x2;
pub const FT_RASTER_FLAG_CLIP: i32 = 0x4;
pub const FT_RASTER_FLAG_SDF: i32 = 0x8;

/* freetype.h */

/// `FT_Glyph_Metrics`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(non_snake_case)]
pub struct FtGlyphMetrics {
    pub width: FtPos,
    pub height: FtPos,
    pub horiBearingX: FtPos,
    pub horiBearingY: FtPos,
    pub horiAdvance: FtPos,
    pub vertBearingX: FtPos,
    pub vertBearingY: FtPos,
    pub vertAdvance: FtPos,
}

/// `FT_Bitmap_Size`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FtBitmapSize {
    pub height: FtShort,
    pub width: FtShort,
    pub size: FtPos,
    pub x_ppem: FtPos,
    pub y_ppem: FtPos,
}

/// `FT_Encoding`
pub type FtEncoding = u32;
pub const FT_ENCODING_NONE: FtEncoding = 0;
pub const FT_ENCODING_MS_SYMBOL: FtEncoding = ft_make_tag(b's', b'y', b'm', b'b');
pub const FT_ENCODING_UNICODE: FtEncoding = ft_make_tag(b'u', b'n', b'i', b'c');
pub const FT_ENCODING_SJIS: FtEncoding = ft_make_tag(b's', b'j', b'i', b's');
pub const FT_ENCODING_PRC: FtEncoding = ft_make_tag(b'g', b'b', b' ', b' ');
pub const FT_ENCODING_BIG5: FtEncoding = ft_make_tag(b'b', b'i', b'g', b'5');
pub const FT_ENCODING_WANSUNG: FtEncoding = ft_make_tag(b'w', b'a', b'n', b's');
pub const FT_ENCODING_JOHAB: FtEncoding = ft_make_tag(b'j', b'o', b'h', b'a');
pub const FT_ENCODING_ADOBE_STANDARD: FtEncoding = ft_make_tag(b'A', b'D', b'O', b'B');
pub const FT_ENCODING_ADOBE_EXPERT: FtEncoding = ft_make_tag(b'A', b'D', b'B', b'E');
pub const FT_ENCODING_ADOBE_CUSTOM: FtEncoding = ft_make_tag(b'A', b'D', b'B', b'C');
pub const FT_ENCODING_ADOBE_LATIN_1: FtEncoding = ft_make_tag(b'l', b'a', b't', b'1');
pub const FT_ENCODING_OLD_LATIN_2: FtEncoding = ft_make_tag(b'l', b'a', b't', b'2');
pub const FT_ENCODING_APPLE_ROMAN: FtEncoding = ft_make_tag(b'a', b'r', b'm', b'n');

pub const FT_FACE_FLAG_SCALABLE: FtLong = 1 << 0;
pub const FT_FACE_FLAG_FIXED_SIZES: FtLong = 1 << 1;
pub const FT_FACE_FLAG_FIXED_WIDTH: FtLong = 1 << 2;
pub const FT_FACE_FLAG_SFNT: FtLong = 1 << 3;
pub const FT_FACE_FLAG_HORIZONTAL: FtLong = 1 << 4;
pub const FT_FACE_FLAG_VERTICAL: FtLong = 1 << 5;
pub const FT_FACE_FLAG_KERNING: FtLong = 1 << 6;
pub const FT_FACE_FLAG_FAST_GLYPHS: FtLong = 1 << 7;
pub const FT_FACE_FLAG_MULTIPLE_MASTERS: FtLong = 1 << 8;
pub const FT_FACE_FLAG_GLYPH_NAMES: FtLong = 1 << 9;
pub const FT_FACE_FLAG_EXTERNAL_STREAM: FtLong = 1 << 10;
pub const FT_FACE_FLAG_HINTER: FtLong = 1 << 11;
pub const FT_FACE_FLAG_CID_KEYED: FtLong = 1 << 12;
pub const FT_FACE_FLAG_TRICKY: FtLong = 1 << 13;
pub const FT_FACE_FLAG_COLOR: FtLong = 1 << 14;
pub const FT_FACE_FLAG_VARIATION: FtLong = 1 << 15;
pub const FT_FACE_FLAG_SVG: FtLong = 1 << 16;
pub const FT_FACE_FLAG_SBIX: FtLong = 1 << 17;
pub const FT_FACE_FLAG_SBIX_OVERLAY: FtLong = 1 << 18;

pub const FT_STYLE_FLAG_ITALIC: FtLong = 1 << 0;
pub const FT_STYLE_FLAG_BOLD: FtLong = 1 << 1;

/// `FT_Size_Metrics`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FtSizeMetrics {
    pub x_ppem: FtUShort,
    pub y_ppem: FtUShort,
    pub x_scale: FtFixed,
    pub y_scale: FtFixed,
    pub ascender: FtPos,
    pub descender: FtPos,
    pub height: FtPos,
    pub max_advance: FtPos,
}

pub const FT_OPEN_MEMORY: u32 = 0x1;
pub const FT_OPEN_STREAM: u32 = 0x2;
pub const FT_OPEN_PATHNAME: u32 = 0x4;
pub const FT_OPEN_DRIVER: u32 = 0x8;
pub const FT_OPEN_PARAMS: u32 = 0x10;

/// `FT_Size_Request_Type`
pub type FtSizeRequestType = u32;
pub const FT_SIZE_REQUEST_TYPE_NOMINAL: FtSizeRequestType = 0;
pub const FT_SIZE_REQUEST_TYPE_REAL_DIM: FtSizeRequestType = 1;
pub const FT_SIZE_REQUEST_TYPE_BBOX: FtSizeRequestType = 2;
pub const FT_SIZE_REQUEST_TYPE_CELL: FtSizeRequestType = 3;
pub const FT_SIZE_REQUEST_TYPE_SCALES: FtSizeRequestType = 4;
pub const FT_SIZE_REQUEST_TYPE_MAX: FtSizeRequestType = 5;

/// `FT_Size_RequestRec`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(non_snake_case)]
pub struct FtSizeRequestRec {
    pub type_: FtSizeRequestType,
    pub width: FtLong,
    pub height: FtLong,
    pub horiResolution: FtUInt,
    pub vertResolution: FtUInt,
}

pub const FT_LOAD_DEFAULT: i32 = 0x0;
pub const FT_LOAD_NO_SCALE: i32 = 1 << 0;
pub const FT_LOAD_NO_HINTING: i32 = 1 << 1;
pub const FT_LOAD_RENDER: i32 = 1 << 2;
pub const FT_LOAD_NO_BITMAP: i32 = 1 << 3;
pub const FT_LOAD_VERTICAL_LAYOUT: i32 = 1 << 4;
pub const FT_LOAD_FORCE_AUTOHINT: i32 = 1 << 5;
pub const FT_LOAD_CROP_BITMAP: i32 = 1 << 6;
pub const FT_LOAD_PEDANTIC: i32 = 1 << 7;
pub const FT_LOAD_IGNORE_GLOBAL_ADVANCE_WIDTH: i32 = 1 << 9;
pub const FT_LOAD_NO_RECURSE: i32 = 1 << 10;
pub const FT_LOAD_IGNORE_TRANSFORM: i32 = 1 << 11;
pub const FT_LOAD_MONOCHROME: i32 = 1 << 12;
pub const FT_LOAD_LINEAR_DESIGN: i32 = 1 << 13;
pub const FT_LOAD_SBITS_ONLY: i32 = 1 << 14;
pub const FT_LOAD_NO_AUTOHINT: i32 = 1 << 15;
pub const FT_LOAD_COLOR: i32 = 1 << 20;
pub const FT_LOAD_COMPUTE_METRICS: i32 = 1 << 21;
pub const FT_LOAD_BITMAP_METRICS_ONLY: i32 = 1 << 22;
pub const FT_LOAD_NO_SVG: i32 = 1 << 24;
pub const FT_LOAD_ADVANCE_ONLY: i32 = 1 << 8;
pub const FT_LOAD_SVG_ONLY: i32 = 1 << 23;

/// `FT_LOAD_TARGET_`
pub const fn ft_load_target(x: FtRenderMode) -> i32 {
    ((x & 15) as i32) << 16
}
pub const FT_LOAD_TARGET_NORMAL: i32 = ft_load_target(FT_RENDER_MODE_NORMAL);
pub const FT_LOAD_TARGET_LIGHT: i32 = ft_load_target(FT_RENDER_MODE_LIGHT);
pub const FT_LOAD_TARGET_MONO: i32 = ft_load_target(FT_RENDER_MODE_MONO);
pub const FT_LOAD_TARGET_LCD: i32 = ft_load_target(FT_RENDER_MODE_LCD);
pub const FT_LOAD_TARGET_LCD_V: i32 = ft_load_target(FT_RENDER_MODE_LCD_V);

/// `FT_LOAD_TARGET_MODE`
pub const fn ft_load_target_mode(x: i32) -> FtRenderMode {
    ((x >> 16) & 15) as FtRenderMode
}

/// `FT_Render_Mode`
pub type FtRenderMode = u32;
pub const FT_RENDER_MODE_NORMAL: FtRenderMode = 0;
pub const FT_RENDER_MODE_LIGHT: FtRenderMode = 1;
pub const FT_RENDER_MODE_MONO: FtRenderMode = 2;
pub const FT_RENDER_MODE_LCD: FtRenderMode = 3;
pub const FT_RENDER_MODE_LCD_V: FtRenderMode = 4;
pub const FT_RENDER_MODE_SDF: FtRenderMode = 5;
pub const FT_RENDER_MODE_MAX: FtRenderMode = 6;

/// `FT_Kerning_Mode`
pub const FT_KERNING_DEFAULT: u32 = 0;
pub const FT_KERNING_UNFITTED: u32 = 1;
pub const FT_KERNING_UNSCALED: u32 = 2;

pub const FT_SUBGLYPH_FLAG_ARGS_ARE_WORDS: u16 = 1;
pub const FT_SUBGLYPH_FLAG_ARGS_ARE_XY_VALUES: u16 = 2;
pub const FT_SUBGLYPH_FLAG_ROUND_XY_TO_GRID: u16 = 4;
pub const FT_SUBGLYPH_FLAG_SCALE: u16 = 8;
pub const FT_SUBGLYPH_FLAG_XY_SCALE: u16 = 0x40;
pub const FT_SUBGLYPH_FLAG_2X2: u16 = 0x80;
pub const FT_SUBGLYPH_FLAG_USE_MY_METRICS: u16 = 0x200;

pub const FREETYPE_MAJOR: i32 = 2;
pub const FREETYPE_MINOR: i32 = 13;
pub const FREETYPE_PATCH: i32 = 2;

/* fterrdef.h */

/// Declares the error codes and their messages (`FT_ERRORDEF_`).
macro_rules! ft_errordefs {
    ($($name:ident = $val:literal, $msg:literal;)*) => {
        $(pub const $name: FtError = $val;)*

        /// The message of an error code, as `FT_Error_String()` (and
        /// SDL_ttf's own table built from fterrdef.h) gives it.
        pub fn ft_error_string(error_code: FtError) -> Option<&'static str> {
            match error_code {
                $($val => Some($msg),)*
                _ => None,
            }
        }
    };
}

ft_errordefs! {
    FT_ERR_OK = 0x00, "no error";
    FT_ERR_CANNOT_OPEN_RESOURCE = 0x01, "cannot open resource";
    FT_ERR_UNKNOWN_FILE_FORMAT = 0x02, "unknown file format";
    FT_ERR_INVALID_FILE_FORMAT = 0x03, "broken file";
    FT_ERR_INVALID_VERSION = 0x04, "invalid FreeType version";
    FT_ERR_LOWER_MODULE_VERSION = 0x05, "module version is too low";
    FT_ERR_INVALID_ARGUMENT = 0x06, "invalid argument";
    FT_ERR_UNIMPLEMENTED_FEATURE = 0x07, "unimplemented feature";
    FT_ERR_INVALID_TABLE = 0x08, "broken table";
    FT_ERR_INVALID_OFFSET = 0x09, "broken offset within table";
    FT_ERR_ARRAY_TOO_LARGE = 0x0A, "array allocation size too large";
    FT_ERR_MISSING_MODULE = 0x0B, "missing module";
    FT_ERR_MISSING_PROPERTY = 0x0C, "missing property";
    FT_ERR_INVALID_GLYPH_INDEX = 0x10, "invalid glyph index";
    FT_ERR_INVALID_CHARACTER_CODE = 0x11, "invalid character code";
    FT_ERR_INVALID_GLYPH_FORMAT = 0x12, "unsupported glyph image format";
    FT_ERR_CANNOT_RENDER_GLYPH = 0x13, "cannot render this glyph format";
    FT_ERR_INVALID_OUTLINE = 0x14, "invalid outline";
    FT_ERR_INVALID_COMPOSITE = 0x15, "invalid composite glyph";
    FT_ERR_TOO_MANY_HINTS = 0x16, "too many hints";
    FT_ERR_INVALID_PIXEL_SIZE = 0x17, "invalid pixel size";
    FT_ERR_INVALID_SVG_DOCUMENT = 0x18, "invalid SVG document";
    FT_ERR_INVALID_HANDLE = 0x20, "invalid object handle";
    FT_ERR_INVALID_LIBRARY_HANDLE = 0x21, "invalid library handle";
    FT_ERR_INVALID_DRIVER_HANDLE = 0x22, "invalid module handle";
    FT_ERR_INVALID_FACE_HANDLE = 0x23, "invalid face handle";
    FT_ERR_INVALID_SIZE_HANDLE = 0x24, "invalid size handle";
    FT_ERR_INVALID_SLOT_HANDLE = 0x25, "invalid glyph slot handle";
    FT_ERR_INVALID_CHARMAP_HANDLE = 0x26, "invalid charmap handle";
    FT_ERR_INVALID_CACHE_HANDLE = 0x27, "invalid cache manager handle";
    FT_ERR_INVALID_STREAM_HANDLE = 0x28, "invalid stream handle";
    FT_ERR_TOO_MANY_DRIVERS = 0x30, "too many modules";
    FT_ERR_TOO_MANY_EXTENSIONS = 0x31, "too many extensions";
    FT_ERR_OUT_OF_MEMORY = 0x40, "out of memory";
    FT_ERR_UNLISTED_OBJECT = 0x41, "unlisted object";
    FT_ERR_CANNOT_OPEN_STREAM = 0x51, "cannot open stream";
    FT_ERR_INVALID_STREAM_SEEK = 0x52, "invalid stream seek";
    FT_ERR_INVALID_STREAM_SKIP = 0x53, "invalid stream skip";
    FT_ERR_INVALID_STREAM_READ = 0x54, "invalid stream read";
    FT_ERR_INVALID_STREAM_OPERATION = 0x55, "invalid stream operation";
    FT_ERR_INVALID_FRAME_OPERATION = 0x56, "invalid frame operation";
    FT_ERR_NESTED_FRAME_ACCESS = 0x57, "nested frame access";
    FT_ERR_INVALID_FRAME_READ = 0x58, "invalid frame read";
    FT_ERR_RASTER_UNINITIALIZED = 0x60, "raster uninitialized";
    FT_ERR_RASTER_CORRUPTED = 0x61, "raster corrupted";
    FT_ERR_RASTER_OVERFLOW = 0x62, "raster overflow";
    FT_ERR_RASTER_NEGATIVE_HEIGHT = 0x63, "negative height while rastering";
    FT_ERR_TOO_MANY_CACHES = 0x70, "too many registered caches";
    FT_ERR_INVALID_OPCODE = 0x80, "invalid opcode";
    FT_ERR_TOO_FEW_ARGUMENTS = 0x81, "too few arguments";
    FT_ERR_STACK_OVERFLOW = 0x82, "stack overflow";
    FT_ERR_CODE_OVERFLOW = 0x83, "code overflow";
    FT_ERR_BAD_ARGUMENT = 0x84, "bad argument";
    FT_ERR_DIVIDE_BY_ZERO = 0x85, "division by zero";
    FT_ERR_INVALID_REFERENCE = 0x86, "invalid reference";
    FT_ERR_DEBUG_OPCODE = 0x87, "found debug opcode";
    FT_ERR_ENDF_IN_EXEC_STREAM = 0x88, "found ENDF opcode in execution stream";
    FT_ERR_NESTED_DEFS = 0x89, "nested DEFS";
    FT_ERR_INVALID_CODERANGE = 0x8A, "invalid code range";
    FT_ERR_EXECUTION_TOO_LONG = 0x8B, "execution context too long";
    FT_ERR_TOO_MANY_FUNCTION_DEFS = 0x8C, "too many function definitions";
    FT_ERR_TOO_MANY_INSTRUCTION_DEFS = 0x8D, "too many instruction definitions";
    FT_ERR_TABLE_MISSING = 0x8E, "SFNT font table missing";
    FT_ERR_HORIZ_HEADER_MISSING = 0x8F, "horizontal header (hhea) table missing";
    FT_ERR_LOCATIONS_MISSING = 0x90, "locations (loca) table missing";
    FT_ERR_NAME_TABLE_MISSING = 0x91, "name table missing";
    FT_ERR_CMAP_TABLE_MISSING = 0x92, "character map (cmap) table missing";
    FT_ERR_HMTX_TABLE_MISSING = 0x93, "horizontal metrics (hmtx) table missing";
    FT_ERR_POST_TABLE_MISSING = 0x94, "PostScript (post) table missing";
    FT_ERR_INVALID_HORIZ_METRICS = 0x95, "invalid horizontal metrics";
    FT_ERR_INVALID_CHARMAP_FORMAT = 0x96, "invalid character map (cmap) format";
    FT_ERR_INVALID_PPEM = 0x97, "invalid ppem value";
    FT_ERR_INVALID_VERT_METRICS = 0x98, "invalid vertical metrics";
    FT_ERR_COULD_NOT_FIND_CONTEXT = 0x99, "could not find context";
    FT_ERR_INVALID_POST_TABLE_FORMAT = 0x9A, "invalid PostScript (post) table format";
    FT_ERR_INVALID_POST_TABLE = 0x9B, "invalid PostScript (post) table";
    FT_ERR_DEF_IN_GLYF_BYTECODE = 0x9C, "found FDEF or IDEF opcode in glyf bytecode";
    FT_ERR_MISSING_BITMAP = 0x9D, "missing bitmap in strike";
    FT_ERR_MISSING_SVG_HOOKS = 0x9E, "SVG hooks have not been set";
    FT_ERR_SYNTAX_ERROR = 0xA0, "opcode syntax error";
    FT_ERR_STACK_UNDERFLOW = 0xA1, "argument stack underflow";
    FT_ERR_IGNORE = 0xA2, "ignore";
    FT_ERR_NO_UNICODE_GLYPH_NAME = 0xA3, "no Unicode glyph name found";
    FT_ERR_GLYPH_TOO_BIG = 0xA4, "glyph too big for hinting";
    FT_ERR_MISSING_STARTFONT_FIELD = 0xB0, "`STARTFONT' field missing";
    FT_ERR_MISSING_FONT_FIELD = 0xB1, "`FONT' field missing";
    FT_ERR_MISSING_SIZE_FIELD = 0xB2, "`SIZE' field missing";
    FT_ERR_MISSING_FONTBOUNDINGBOX_FIELD = 0xB3, "`FONTBOUNDINGBOX' field missing";
    FT_ERR_MISSING_CHARS_FIELD = 0xB4, "`CHARS' field missing";
    FT_ERR_MISSING_STARTCHAR_FIELD = 0xB5, "`STARTCHAR' field missing";
    FT_ERR_MISSING_ENCODING_FIELD = 0xB6, "`ENCODING' field missing";
    FT_ERR_MISSING_BBX_FIELD = 0xB7, "`BBX' field missing";
    FT_ERR_BBX_TOO_BIG = 0xB8, "`BBX' too big";
    FT_ERR_CORRUPTED_FONT_HEADER = 0xB9, "Font header corrupted or missing fields";
    FT_ERR_CORRUPTED_FONT_GLYPHS = 0xBA, "Font glyphs corrupted or missing fields";
}

/// `FT_Result`-style shorthand used throughout the translation: C's
/// `FT_Error` return becomes `Result<T, FtError>`, with `FT_Err_Ok` as `Ok`.
pub type FtResult<T> = Result<T, FtError>;

/// `ft_isdigit`
pub const fn ft_isdigit(x: u8) -> bool {
    x.wrapping_sub(b'0') < 10
}
/// `ft_isxdigit`
pub const fn ft_isxdigit(x: u8) -> bool {
    x.wrapping_sub(b'0') < 10 || x.wrapping_sub(b'a') < 6 || x.wrapping_sub(b'A') < 6
}
/// `ft_isupper`
pub const fn ft_isupper(x: u8) -> bool {
    x.wrapping_sub(b'A') < 26
}
/// `ft_islower`
pub const fn ft_islower(x: u8) -> bool {
    x.wrapping_sub(b'a') < 26
}
/// `ft_isalpha`
pub const fn ft_isalpha(x: u8) -> bool {
    ft_isupper(x) || ft_islower(x)
}
/// `ft_isalnum`
pub const fn ft_isalnum(x: u8) -> bool {
    ft_isdigit(x) || ft_isalpha(x)
}

/// `FT_PIX_FLOOR`
pub const fn ft_pix_floor(x: FtPos) -> FtPos {
    x & !63
}
/// `FT_PIX_ROUND`
pub const fn ft_pix_round(x: FtPos) -> FtPos {
    ft_pix_floor(x.wrapping_add(32))
}
/// `FT_PIX_CEIL`
pub const fn ft_pix_ceil(x: FtPos) -> FtPos {
    ft_pix_floor(x.wrapping_add(63))
}
/// `FT_PIX_ROUND_LONG`
pub const fn ft_pix_round_long(x: FtPos) -> FtPos {
    ft_pix_floor(x.wrapping_add(32))
}
/// `FT_PIX_CEIL_LONG`
pub const fn ft_pix_ceil_long(x: FtPos) -> FtPos {
    ft_pix_floor(x.wrapping_add(63))
}
/// `FT_PAD_FLOOR`
pub const fn ft_pad_floor(x: i64, n: i64) -> i64 {
    x & !(n - 1)
}
/// `FT_PAD_ROUND`
pub const fn ft_pad_round(x: i64, n: i64) -> i64 {
    ft_pad_floor(x.wrapping_add(n / 2), n)
}
/// `FT_PAD_CEIL`
pub const fn ft_pad_ceil(x: i64, n: i64) -> i64 {
    ft_pad_floor(x.wrapping_add(n - 1), n)
}
/// `FT_PIX_ROUND_INT32` (on an `FT_Int32`)
pub const fn ft_pix_round_int32(x: i32) -> i32 {
    x.wrapping_add(32) & !63
}
/// `FT_PIX_CEIL_INT32` (on an `FT_Int32`)
pub const fn ft_pix_ceil_int32(x: i32) -> i32 {
    x.wrapping_add(63) & !63
}

/// `FT_ABS` on an `FT_Long` (wrapping, as C's negation of `LONG_MIN` does
/// in practice)
pub const fn ft_abs(a: i64) -> i64 {
    if a < 0 {
        a.wrapping_neg()
    } else {
        a
    }
}

/// `FT_HYPOT`
pub const fn ft_hypot(x: i64, y: i64) -> i64 {
    let x = ft_abs(x);
    let y = ft_abs(y);
    if x > y {
        x.wrapping_add((3i64.wrapping_mul(y)) >> 3)
    } else {
        y.wrapping_add((3i64.wrapping_mul(x)) >> 3)
    }
}
