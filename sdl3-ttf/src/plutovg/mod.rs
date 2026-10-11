// Rust translation of include/plutovg.h and source/plutovg-utils.h from
// PlutoVG (0.0.12, as SDL_ttf's external/plutovg pins it: commit
// 3e6f922f453da1c9e7d1d7f66cac1d9724a18b47).
// Copyright (c) 2020-2024 Samuel Ugochukwu <sammycageagle@gmail.com>
// This is an altered (translated) version of the original software; it is
// used under the MIT License (see PLUTOVG-LICENSE and LICENSE.txt).

//! PlutoVG, the 2D vector graphics library PlutoSVG draws with, as SDL_ttf
//! builds it (`external/plutovg`): matrices, paths (with dashing and
//! flattening), colors and paints (solid colors, linear and radial
//! gradients, textures), ARGB32 premultiplied surfaces, the canvas with
//! its fill and stroke operations, the FreeType-derived span rasterizer
//! and stroker (`ft_*`, under the FreeType License: see their headers)
//! and the compositing of spans (`blend`).
//!
//! Translation notes:
//!
//! * Reference counting (`plutovg_*_reference`, `_destroy`,
//!   `_get_reference_count`) is Rust ownership: surfaces, paths and
//!   canvases are values, paints are shared with [`Rc`](std::rc::Rc).
//!   `plutovg_surface_create_for_data` takes ownership of the pixel buffer
//!   (`plutovg_surface_into_data` gives it back).
//! * The font and text parts (`plutovg-font.c` with stb_truetype, the text
//!   iterator, the canvas's font and text functions) are not translated:
//!   PlutoSVG does not use them. Neither are the image writers
//!   (stb_image_write); images are decoded by
//!   [`plutovg_surface_load_from_image_data`] (see there).
//! * Floats are converted to integers as C does on x86-64 (`cvt_*`):
//!   values out of range (and NaN) become the "integer indefinite" value.
//! * The C library math functions are those of the reference build
//!   ([`libm`]).

#![allow(
    clippy::too_many_arguments,
    clippy::needless_late_init,
    clippy::collapsible_else_if
)]

pub mod blend;
pub mod canvas;
pub mod ft_math;
pub mod ft_raster;
pub mod ft_stroker;
pub mod libm;
pub mod matrix;
pub mod paint;
pub mod path;
pub mod rasterize;
pub mod surface;

pub use canvas::*;
pub use matrix::*;
pub use paint::*;
pub use path::*;
pub use surface::*;

/* ---- plutovg.h ------------------------------------------------------- */

pub const PLUTOVG_VERSION_MAJOR: i32 = 0;
pub const PLUTOVG_VERSION_MINOR: i32 = 0;
pub const PLUTOVG_VERSION_MICRO: i32 = 12;

/// `PLUTOVG_VERSION_ENCODE`
pub const fn plutovg_version_encode(major: i32, minor: i32, micro: i32) -> i32 {
    (major * 10000) + (minor * 100) + micro
}

pub const PLUTOVG_VERSION: i32 = plutovg_version_encode(
    PLUTOVG_VERSION_MAJOR,
    PLUTOVG_VERSION_MINOR,
    PLUTOVG_VERSION_MICRO,
);
pub const PLUTOVG_VERSION_STRING: &str = "0.0.12";

/// Gets the version of the plutovg library.
pub fn plutovg_version() -> i32 {
    PLUTOVG_VERSION
}

/// Gets the version of the plutovg library as a string.
pub fn plutovg_version_string() -> &'static str {
    PLUTOVG_VERSION_STRING
}

#[allow(clippy::excessive_precision, clippy::approx_constant)]
pub const PLUTOVG_PI: f32 = 3.14159265358979323846;
#[allow(clippy::excessive_precision, clippy::approx_constant)]
pub const PLUTOVG_TWO_PI: f32 = 6.28318530717958647693;
#[allow(clippy::excessive_precision, clippy::approx_constant)]
pub const PLUTOVG_HALF_PI: f32 = 1.57079632679489661923;
#[allow(clippy::excessive_precision, clippy::approx_constant)]
pub const PLUTOVG_SQRT2: f32 = 1.41421356237309504880;
#[allow(clippy::excessive_precision, clippy::approx_constant)]
pub const PLUTOVG_KAPPA: f32 = 0.55228474983079339840;

/// `PLUTOVG_DEG2RAD`
#[inline]
pub fn plutovg_deg2rad(x: f32) -> f32 {
    x * (PLUTOVG_PI / 180.0)
}

/// `PLUTOVG_RAD2DEG`
#[inline]
pub fn plutovg_rad2deg(x: f32) -> f32 {
    x * (180.0 / PLUTOVG_PI)
}

/// A structure representing a point in 2D space.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Point {
    /// The x-coordinate of the point.
    pub x: f32,
    /// The y-coordinate of the point.
    pub y: f32,
}

/// A structure representing a rectangle in 2D space.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    /// The x-coordinate of the top-left corner of the rectangle.
    pub x: f32,
    /// The y-coordinate of the top-left corner of the rectangle.
    pub y: f32,
    /// The width of the rectangle.
    pub w: f32,
    /// The height of the rectangle.
    pub h: f32,
}

/// `PLUTOVG_MAKE_RECT`
#[inline]
pub const fn plutovg_make_rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

/// A structure representing a 2D transformation matrix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix {
    /// The horizontal scaling factor.
    pub a: f32,
    /// The vertical shearing factor.
    pub b: f32,
    /// The horizontal shearing factor.
    pub c: f32,
    /// The vertical scaling factor.
    pub d: f32,
    /// The horizontal translation offset.
    pub e: f32,
    /// The vertical translation offset.
    pub f: f32,
}

/// `PLUTOVG_IDENTITY_MATRIX`
pub const PLUTOVG_IDENTITY_MATRIX: Matrix = Matrix {
    a: 1.0,
    b: 0.0,
    c: 0.0,
    d: 1.0,
    e: 0.0,
    f: 0.0,
};

impl Default for Matrix {
    fn default() -> Self {
        PLUTOVG_IDENTITY_MATRIX
    }
}

/// Path commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathCommand {
    /// Moves the current point to a new position.
    MoveTo,
    /// Draws a straight line to a new point.
    LineTo,
    /// Draws a cubic Bézier curve to a new point.
    CubicTo,
    /// Closes the current path by drawing a line to the starting point.
    Close,
}

/// `plutovg_path_element_t`: a header (a command and the number of
/// elements including the header) or a point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathElement {
    /// Header for path commands.
    Header {
        /// The path command.
        command: PathCommand,
        /// Number of elements including the header.
        length: i32,
    },
    /// A coordinate point in the path.
    Point(Point),
}

impl PathElement {
    /// `element.point` (a header reads as the origin)
    #[inline]
    pub fn point(&self) -> Point {
        match *self {
            PathElement::Point(p) => p,
            PathElement::Header { .. } => Point::default(),
        }
    }
}

/// A structure representing a color with red, green, blue, and alpha
/// components, each from 0 to 1.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Color {
    /// Red component (0 to 1).
    pub r: f32,
    /// Green component (0 to 1).
    pub g: f32,
    /// Blue component (0 to 1).
    pub b: f32,
    /// Alpha (opacity) component (0 to 1).
    pub a: f32,
}

/// `PLUTOVG_MAKE_COLOR`
#[inline]
pub const fn plutovg_make_color(r: f32, g: f32, b: f32, a: f32) -> Color {
    Color { r, g, b, a }
}

pub const PLUTOVG_BLACK_COLOR: Color = plutovg_make_color(0.0, 0.0, 0.0, 1.0);
pub const PLUTOVG_WHITE_COLOR: Color = plutovg_make_color(1.0, 1.0, 1.0, 1.0);

/// Texture types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureType {
    /// Plain texture.
    Plain,
    /// Tiled texture.
    Tiled,
}

/// Gradient spread methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpreadMethod {
    /// Pad the gradient's edges.
    Pad,
    /// Reflect the gradient beyond its bounds.
    Reflect,
    /// Repeat the gradient pattern.
    Repeat,
}

/// A gradient stop.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GradientStop {
    /// The offset of the gradient stop, as a value between 0 and 1.
    pub offset: f32,
    /// The color of the gradient stop.
    pub color: Color,
}

/// Fill rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillRule {
    /// Non-zero winding fill rule.
    NonZero,
    /// Even-odd fill rule.
    EvenOdd,
}

/// Compositing operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    /// Clears the destination (resulting in a fully transparent image).
    Clear,
    /// Source replaces destination.
    Src,
    /// Destination is kept, source is ignored.
    Dst,
    /// Source is composited over destination.
    SrcOver,
    /// Destination is composited over source.
    DstOver,
    /// Source within destination.
    SrcIn,
    /// Destination within source.
    DstIn,
    /// Source outside destination.
    SrcOut,
    /// Destination outside source.
    DstOut,
    /// Source atop destination.
    SrcAtop,
    /// Destination atop source.
    DstAtop,
    /// Source and destination are combined, but their overlapping regions
    /// are cleared.
    Xor,
}

/// Line caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineCap {
    /// Flat edge at the end of the stroke.
    Butt,
    /// Rounded ends at the end of the stroke.
    Round,
    /// Square ends at the end of the stroke.
    Square,
}

/// Line joins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineJoin {
    /// Miter join with sharp corners.
    Miter,
    /// Rounded join.
    Round,
    /// Beveled join with a flattened corner.
    Bevel,
}

/* ---- plutovg-utils.h ------------------------------------------------- */

/// `PLUTOVG_IS_NUM`
#[inline]
pub fn plutovg_is_num(c: u8) -> bool {
    c.is_ascii_digit()
}

/// `PLUTOVG_IS_ALPHA`
#[inline]
pub fn plutovg_is_alpha(c: u8) -> bool {
    c.is_ascii_alphabetic()
}

/// `PLUTOVG_IS_ALNUM`
#[inline]
pub fn plutovg_is_alnum(c: u8) -> bool {
    plutovg_is_alpha(c) || plutovg_is_num(c)
}

/// `PLUTOVG_IS_WS`
#[inline]
pub fn plutovg_is_ws(c: u8) -> bool {
    c == b' ' || c == b'\t' || c == b'\n' || c == b'\r'
}

/// `plutovg_min` (C's `(a) < (b) ? (a) : (b)`)
#[inline]
pub fn plutovg_min<T: PartialOrd>(a: T, b: T) -> T {
    if a < b {
        a
    } else {
        b
    }
}

/// `plutovg_max` (C's `(a) > (b) ? (a) : (b)`)
#[inline]
pub fn plutovg_max<T: PartialOrd>(a: T, b: T) -> T {
    if a > b {
        a
    } else {
        b
    }
}

/// `plutovg_clamp` (C's `(v) < (lo) ? (lo) : (hi) < (v) ? (hi) : (v)`)
#[inline]
pub fn plutovg_clamp<T: PartialOrd>(v: T, lo: T, hi: T) -> T {
    if v < lo {
        lo
    } else if hi < v {
        hi
    } else {
        v
    }
}

/// `plutovg_alpha`
#[inline]
pub fn plutovg_alpha(c: u32) -> u32 {
    (c >> 24) & 0xff
}

/// `plutovg_red`
#[inline]
pub fn plutovg_red(c: u32) -> u32 {
    (c >> 16) & 0xff
}

/// `plutovg_green`
#[inline]
pub fn plutovg_green(c: u32) -> u32 {
    (c >> 8) & 0xff
}

/// `plutovg_blue`
#[inline]
pub fn plutovg_blue(c: u32) -> u32 {
    c & 0xff
}

/// `plutovg_div255`
#[inline]
pub fn plutovg_div255(x: u32) -> u32 {
    (x.wrapping_add(x >> 8).wrapping_add(0x80)) >> 8
}

/// `plutovg_premultiply_argb`
#[inline]
pub fn plutovg_premultiply_argb(color: u32) -> u32 {
    let a = plutovg_alpha(color);
    let mut r = plutovg_red(color);
    let mut g = plutovg_green(color);
    let mut b = plutovg_blue(color);
    if a != 255 {
        r = plutovg_div255(r * a);
        g = plutovg_div255(g * a);
        b = plutovg_div255(b * a);
    }

    (a << 24) | (r << 16) | (g << 8) | b
}

/// `plutovg_parse_number`: parses a number at `*begin` in `data` (whose
/// end is `end`).
pub fn plutovg_parse_number(data: &[u8], begin: &mut usize, end: usize, number: &mut f32) -> bool {
    let mut it = *begin;
    let mut fraction: f32 = 0.0;
    let mut integer: f32 = 0.0;
    let mut exponent: f32 = 0.0;
    let mut sign: i32 = 1;
    let mut expsign: i32 = 1;

    if it < end && data[it] == b'+' {
        it += 1;
    } else if it < end && data[it] == b'-' {
        it += 1;
        sign = -1;
    }

    if it >= end || (data[it] != b'.' && !plutovg_is_num(data[it])) {
        return false;
    }
    if plutovg_is_num(data[it]) {
        loop {
            integer = 10.0 * integer + (data[it] - b'0') as f32;
            it += 1;
            if !(it < end && plutovg_is_num(data[it])) {
                break;
            }
        }
    }

    if it < end && data[it] == b'.' {
        it += 1;
        if it >= end || !plutovg_is_num(data[it]) {
            return false;
        }
        let mut divisor: f32 = 1.0;
        loop {
            fraction = 10.0 * fraction + (data[it] - b'0') as f32;
            it += 1;
            divisor *= 10.0;
            if !(it < end && plutovg_is_num(data[it])) {
                break;
            }
        }
        fraction /= divisor;
    }

    if it < end && (data[it] == b'e' || data[it] == b'E') {
        it += 1;
        if it < end && data[it] == b'+' {
            it += 1;
        } else if it < end && data[it] == b'-' {
            it += 1;
            expsign = -1;
        }

        if it >= end || !plutovg_is_num(data[it]) {
            return false;
        }
        loop {
            exponent = 10.0 * exponent + (data[it] - b'0') as f32;
            it += 1;
            if !(it < end && plutovg_is_num(data[it])) {
                break;
            }
        }
    }

    *begin = it;
    *number = sign as f32 * (integer + fraction);
    if exponent != 0.0 {
        *number *= libm::powf(10.0, expsign as f32 * exponent);
    }
    *number >= -f32::MAX && *number <= f32::MAX
}

/// `plutovg_skip_delim`
#[inline]
pub fn plutovg_skip_delim(data: &[u8], begin: &mut usize, end: usize, delim: u8) -> bool {
    let it = *begin;
    if it < end && data[it] == delim {
        *begin = it + 1;
        return true;
    }

    false
}

/// `plutovg_skip_string`
#[inline]
pub fn plutovg_skip_string(data: &[u8], begin: &mut usize, end: usize, s: &[u8]) -> bool {
    let mut it = *begin;
    let mut k = 0;
    while it < end && k < s.len() && data[it] == s[k] {
        k += 1;
        it += 1;
    }

    if k == s.len() {
        *begin = it;
        return true;
    }

    false
}

/// `plutovg_skip_ws`
#[inline]
pub fn plutovg_skip_ws(data: &[u8], begin: &mut usize, end: usize) -> bool {
    let mut it = *begin;
    while it < end && plutovg_is_ws(data[it]) {
        it += 1;
    }
    *begin = it;
    it < end
}

/// `plutovg_skip_ws_and_delim`
#[inline]
pub fn plutovg_skip_ws_and_delim(data: &[u8], begin: &mut usize, end: usize, delim: u8) -> bool {
    let mut it = *begin;
    if plutovg_skip_ws(data, &mut it, end) {
        if !plutovg_skip_delim(data, &mut it, end, delim) {
            return false;
        }
        plutovg_skip_ws(data, &mut it, end);
    } else {
        return false;
    }

    *begin = it;
    it < end
}

/// `plutovg_skip_ws_and_comma`
#[inline]
pub fn plutovg_skip_ws_and_comma(data: &[u8], begin: &mut usize, end: usize) -> bool {
    plutovg_skip_ws_and_delim(data, begin, end, b',')
}

/// `plutovg_skip_ws_or_delim`
#[inline]
pub fn plutovg_skip_ws_or_delim(
    data: &[u8],
    begin: &mut usize,
    end: usize,
    delim: u8,
    has_delim: Option<&mut bool>,
) -> bool {
    let mut it = *begin;
    let mut found = false;
    if plutovg_skip_ws(data, &mut it, end) && plutovg_skip_delim(data, &mut it, end, delim) {
        found = true;
        plutovg_skip_ws(data, &mut it, end);
    }
    if let Some(h) = has_delim {
        *h = found;
    }

    if it == *begin {
        return false;
    }
    *begin = it;
    it < end
}

/// `plutovg_skip_ws_or_comma`
#[inline]
pub fn plutovg_skip_ws_or_comma(
    data: &[u8],
    begin: &mut usize,
    end: usize,
    has_comma: Option<&mut bool>,
) -> bool {
    plutovg_skip_ws_or_delim(data, begin, end, b',', has_comma)
}

/* ---- C's float to integer conversions on x86-64 ----------------------- */

/// `(int)x` as x86-64 converts (`cvttss2si`): truncation, and `INT_MIN`
/// for NaN and values out of range.
#[inline]
pub fn cvt_f32_i32(x: f32) -> i32 {
    if x.is_nan() || x >= 2147483648.0 || x < -2147483648.0 {
        i32::MIN
    } else {
        x as i32
    }
}

/// `(long)x` (64-bit) as x86-64 converts: truncation, and `LONG_MIN` for
/// NaN and values out of range.
#[inline]
pub fn cvt_f32_i64(x: f32) -> i64 {
    if x.is_nan() || x >= 9223372036854775808.0 || x < -9223372036854775808.0 {
        i64::MIN
    } else {
        x as i64
    }
}

/// `(long)x` of a double as x86-64 converts.
#[inline]
pub fn cvt_f64_i64(x: f64) -> i64 {
    if x.is_nan() || x >= 9223372036854775808.0 || x < -9223372036854775808.0 {
        i64::MIN
    } else {
        x as i64
    }
}

/// `(int)x` of a double as x86-64 converts.
#[inline]
pub fn cvt_f64_i32(x: f64) -> i32 {
    if x.is_nan() || x >= 2147483648.0 || x < -2147483648.0 {
        i32::MIN
    } else {
        x as i32
    }
}

/// `(unsigned int)x` as GCC converts on x86-64 (a 64-bit `cvttss2si`,
/// truncated to 32 bits).
#[inline]
pub fn cvt_f32_u32(x: f32) -> u32 {
    cvt_f32_i64(x) as u32
}

/// `lroundf`: rounding half away from zero (`LONG_MIN` for NaN and values
/// out of range, as x86-64's glibc returns).
#[inline]
pub fn lroundf(x: f32) -> i64 {
    cvt_f32_i64(x.round())
}
