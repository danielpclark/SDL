// Rust translation of source/plutovg-paint.c (and the paint parts of
// source/plutovg-private.h) from PlutoVG (0.0.12, as SDL_ttf's
// external/plutovg pins it).
// Copyright (c) 2020-2024 Samuel Ugochukwu <sammycageagle@gmail.com>
// This is an altered (translated) version of the original software; it is
// used under the MIT License (see PLUTOVG-LICENSE and LICENSE.txt).

//! Colors (and the CSS color parser) and paints.

use std::rc::Rc;

use super::surface::Surface;
use super::*;

/// Initializes a color using RGB components in the 0-1 range.
pub fn plutovg_color_init_rgb(color: &mut Color, r: f32, g: f32, b: f32) {
    plutovg_color_init_rgba(color, r, g, b, 1.0);
}

/// Initializes a color using RGBA components in the 0-1 range.
pub fn plutovg_color_init_rgba(color: &mut Color, r: f32, g: f32, b: f32, a: f32) {
    color.r = plutovg_clamp(r, 0.0, 1.0);
    color.g = plutovg_clamp(g, 0.0, 1.0);
    color.b = plutovg_clamp(b, 0.0, 1.0);
    color.a = plutovg_clamp(a, 0.0, 1.0);
}

/// Initializes a color using RGB components in the 0-255 range.
pub fn plutovg_color_init_rgb8(color: &mut Color, r: i32, g: i32, b: i32) {
    plutovg_color_init_rgba8(color, r, g, b, 255);
}

/// Initializes a color using RGBA components in the 0-255 range.
pub fn plutovg_color_init_rgba8(color: &mut Color, r: i32, g: i32, b: i32, a: i32) {
    plutovg_color_init_rgba(
        color,
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    );
}

/// Initializes a color from a 32-bit unsigned RGBA value.
pub fn plutovg_color_init_rgba32(color: &mut Color, value: u32) {
    let r = ((value >> 24) & 0xFF) as u8;
    let g = ((value >> 16) & 0xFF) as u8;
    let b = ((value >> 8) & 0xFF) as u8;
    let a = (value & 0xFF) as u8;
    plutovg_color_init_rgba8(color, r as i32, g as i32, b as i32, a as i32);
}

/// Initializes a color from a 32-bit unsigned ARGB value.
pub fn plutovg_color_init_argb32(color: &mut Color, value: u32) {
    let a = ((value >> 24) & 0xFF) as u8;
    let r = ((value >> 16) & 0xFF) as u8;
    let g = ((value >> 8) & 0xFF) as u8;
    let b = (value & 0xFF) as u8;
    plutovg_color_init_rgba8(color, r as i32, g as i32, b as i32, a as i32);
}

/// Converts a color to a 32-bit unsigned RGBA value.
pub fn plutovg_color_to_rgba32(color: &Color) -> u32 {
    let r = lroundf(color.r * 255.0) as u32;
    let g = lroundf(color.g * 255.0) as u32;
    let b = lroundf(color.b * 255.0) as u32;
    let a = lroundf(color.a * 255.0) as u32;
    (r << 24) | (g << 16) | (b << 8) | a
}

/// Converts a color to a 32-bit unsigned ARGB value.
pub fn plutovg_color_to_argb32(color: &Color) -> u32 {
    let a = lroundf(color.a * 255.0) as u32;
    let r = lroundf(color.r * 255.0) as u32;
    let g = lroundf(color.g * 255.0) as u32;
    let b = lroundf(color.b * 255.0) as u32;
    (a << 24) | (r << 16) | (g << 8) | b
}

/// `hex_digit`
#[inline]
fn hex_digit(c: u8) -> u8 {
    if c.is_ascii_digit() {
        return c - b'0';
    }
    if (b'a'..=b'f').contains(&c) {
        return 10 + c - b'a';
    }
    if (b'A'..=b'F').contains(&c) {
        return 10 + c - b'A';
    }
    0
}

/// `hex_expand`
#[inline]
fn hex_expand(c: u8) -> u8 {
    let h = hex_digit(c);
    (h << 4) | h
}

/// `hex_combine`
#[inline]
fn hex_combine(c1: u8, c2: u8) -> u8 {
    let h1 = hex_digit(c1);
    let h2 = hex_digit(c2);
    (h1 << 4) | h2
}

const MAX_NAME: usize = 24;

/// `parse_rgb_component`
fn parse_rgb_component(data: &[u8], begin: &mut usize, end: usize, component: &mut i32) -> bool {
    let mut value: f32 = 0.0;
    if !plutovg_parse_number(data, begin, end, &mut value) {
        return false;
    }
    if plutovg_skip_delim(data, begin, end, b'%') {
        value *= 2.55;
    }
    value = plutovg_clamp(value, 0.0, 255.0);
    *component = lroundf(value) as i32;
    true
}

/// `parse_alpha_component`
fn parse_alpha_component(data: &[u8], begin: &mut usize, end: usize, component: &mut i32) -> bool {
    let mut value: f32 = 0.0;
    if !plutovg_parse_number(data, begin, end, &mut value) {
        return false;
    }
    if plutovg_skip_delim(data, begin, end, b'%') {
        value /= 100.0;
    }
    value = plutovg_clamp(value, 0.0, 1.0);
    *component = lroundf(value * 255.0) as i32;
    true
}

/// The named colors of `plutovg_color_parse` (`colormap`), sorted.
static COLORMAP: [(&str, u32); 148] = [
    ("aliceblue", 0xF0F8FF),
    ("antiquewhite", 0xFAEBD7),
    ("aqua", 0x00FFFF),
    ("aquamarine", 0x7FFFD4),
    ("azure", 0xF0FFFF),
    ("beige", 0xF5F5DC),
    ("bisque", 0xFFE4C4),
    ("black", 0x000000),
    ("blanchedalmond", 0xFFEBCD),
    ("blue", 0x0000FF),
    ("blueviolet", 0x8A2BE2),
    ("brown", 0xA52A2A),
    ("burlywood", 0xDEB887),
    ("cadetblue", 0x5F9EA0),
    ("chartreuse", 0x7FFF00),
    ("chocolate", 0xD2691E),
    ("coral", 0xFF7F50),
    ("cornflowerblue", 0x6495ED),
    ("cornsilk", 0xFFF8DC),
    ("crimson", 0xDC143C),
    ("cyan", 0x00FFFF),
    ("darkblue", 0x00008B),
    ("darkcyan", 0x008B8B),
    ("darkgoldenrod", 0xB8860B),
    ("darkgray", 0xA9A9A9),
    ("darkgreen", 0x006400),
    ("darkgrey", 0xA9A9A9),
    ("darkkhaki", 0xBDB76B),
    ("darkmagenta", 0x8B008B),
    ("darkolivegreen", 0x556B2F),
    ("darkorange", 0xFF8C00),
    ("darkorchid", 0x9932CC),
    ("darkred", 0x8B0000),
    ("darksalmon", 0xE9967A),
    ("darkseagreen", 0x8FBC8F),
    ("darkslateblue", 0x483D8B),
    ("darkslategray", 0x2F4F4F),
    ("darkslategrey", 0x2F4F4F),
    ("darkturquoise", 0x00CED1),
    ("darkviolet", 0x9400D3),
    ("deeppink", 0xFF1493),
    ("deepskyblue", 0x00BFFF),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1E90FF),
    ("firebrick", 0xB22222),
    ("floralwhite", 0xFFFAF0),
    ("forestgreen", 0x228B22),
    ("fuchsia", 0xFF00FF),
    ("gainsboro", 0xDCDCDC),
    ("ghostwhite", 0xF8F8FF),
    ("gold", 0xFFD700),
    ("goldenrod", 0xDAA520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xADFF2F),
    ("grey", 0x808080),
    ("honeydew", 0xF0FFF0),
    ("hotpink", 0xFF69B4),
    ("indianred", 0xCD5C5C),
    ("indigo", 0x4B0082),
    ("ivory", 0xFFFFF0),
    ("khaki", 0xF0E68C),
    ("lavender", 0xE6E6FA),
    ("lavenderblush", 0xFFF0F5),
    ("lawngreen", 0x7CFC00),
    ("lemonchiffon", 0xFFFACD),
    ("lightblue", 0xADD8E6),
    ("lightcoral", 0xF08080),
    ("lightcyan", 0xE0FFFF),
    ("lightgoldenrodyellow", 0xFAFAD2),
    ("lightgray", 0xD3D3D3),
    ("lightgreen", 0x90EE90),
    ("lightgrey", 0xD3D3D3),
    ("lightpink", 0xFFB6C1),
    ("lightsalmon", 0xFFA07A),
    ("lightseagreen", 0x20B2AA),
    ("lightskyblue", 0x87CEFA),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xB0C4DE),
    ("lightyellow", 0xFFFFE0),
    ("lime", 0x00FF00),
    ("limegreen", 0x32CD32),
    ("linen", 0xFAF0E6),
    ("magenta", 0xFF00FF),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66CDAA),
    ("mediumblue", 0x0000CD),
    ("mediumorchid", 0xBA55D3),
    ("mediumpurple", 0x9370DB),
    ("mediumseagreen", 0x3CB371),
    ("mediumslateblue", 0x7B68EE),
    ("mediumspringgreen", 0x00FA9A),
    ("mediumturquoise", 0x48D1CC),
    ("mediumvioletred", 0xC71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xF5FFFA),
    ("mistyrose", 0xFFE4E1),
    ("moccasin", 0xFFE4B5),
    ("navajowhite", 0xFFDEAD),
    ("navy", 0x000080),
    ("oldlace", 0xFDF5E6),
    ("olive", 0x808000),
    ("olivedrab", 0x6B8E23),
    ("orange", 0xFFA500),
    ("orangered", 0xFF4500),
    ("orchid", 0xDA70D6),
    ("palegoldenrod", 0xEEE8AA),
    ("palegreen", 0x98FB98),
    ("paleturquoise", 0xAFEEEE),
    ("palevioletred", 0xDB7093),
    ("papayawhip", 0xFFEFD5),
    ("peachpuff", 0xFFDAB9),
    ("peru", 0xCD853F),
    ("pink", 0xFFC0CB),
    ("plum", 0xDDA0DD),
    ("powderblue", 0xB0E0E6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xFF0000),
    ("rosybrown", 0xBC8F8F),
    ("royalblue", 0x4169E1),
    ("saddlebrown", 0x8B4513),
    ("salmon", 0xFA8072),
    ("sandybrown", 0xF4A460),
    ("seagreen", 0x2E8B57),
    ("seashell", 0xFFF5EE),
    ("sienna", 0xA0522D),
    ("silver", 0xC0C0C0),
    ("skyblue", 0x87CEEB),
    ("slateblue", 0x6A5ACD),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xFFFAFA),
    ("springgreen", 0x00FF7F),
    ("steelblue", 0x4682B4),
    ("tan", 0xD2B48C),
    ("teal", 0x008080),
    ("thistle", 0xD8BFD8),
    ("tomato", 0xFF6347),
    ("turquoise", 0x40E0D0),
    ("violet", 0xEE82EE),
    ("wheat", 0xF5DEB3),
    ("white", 0xFFFFFF),
    ("whitesmoke", 0xF5F5F5),
    ("yellow", 0xFFFF00),
    ("yellowgreen", 0x9ACD32),
];

/// Parses a color from a string in CSS color formats: returns the number
/// of bytes consumed, or 0 on failure.
pub fn plutovg_color_parse(color: &mut Color, data: &[u8]) -> i32 {
    let mut it = 0;
    let end = data.len();
    plutovg_skip_ws(data, &mut it, end);
    if plutovg_skip_delim(data, &mut it, end, b'#') {
        let r;
        let g;
        let b;
        let mut a = 255;
        let begin = it;
        while it < end && data[it].is_ascii_hexdigit() {
            it += 1;
        }
        let count = it - begin;
        let d = &data[begin..];
        if count == 3 || count == 4 {
            r = hex_expand(d[0]) as i32;
            g = hex_expand(d[1]) as i32;
            b = hex_expand(d[2]) as i32;
            if count == 4 {
                a = hex_expand(d[3]) as i32;
            }
        } else if count == 6 || count == 8 {
            r = hex_combine(d[0], d[1]) as i32;
            g = hex_combine(d[2], d[3]) as i32;
            b = hex_combine(d[4], d[5]) as i32;
            if count == 8 {
                a = hex_combine(d[6], d[7]) as i32;
            }
        } else {
            return 0;
        }

        plutovg_color_init_rgba8(color, r, g, b, a);
    } else {
        let mut name = Vec::with_capacity(MAX_NAME);
        while it < end && name.len() < MAX_NAME && data[it].is_ascii_alphabetic() {
            name.push(data[it].to_ascii_lowercase());
            it += 1;
        }

        if name == b"transparent" {
            plutovg_color_init_rgba(color, 0.0, 0.0, 0.0, 0.0);
        } else if name == b"rgb" || name == b"rgba" {
            if !plutovg_skip_ws_and_delim(data, &mut it, end, b'(') {
                return 0;
            }
            let mut r = 0;
            let mut g = 0;
            let mut b = 0;
            let mut a = 255;
            if !parse_rgb_component(data, &mut it, end, &mut r)
                || !plutovg_skip_ws_and_comma(data, &mut it, end)
                || !parse_rgb_component(data, &mut it, end, &mut g)
                || !plutovg_skip_ws_and_comma(data, &mut it, end)
                || !parse_rgb_component(data, &mut it, end, &mut b)
            {
                return 0;
            }

            if plutovg_skip_ws_and_comma(data, &mut it, end)
                && !parse_alpha_component(data, &mut it, end, &mut a)
            {
                return 0;
            }

            plutovg_skip_ws(data, &mut it, end);
            if !plutovg_skip_delim(data, &mut it, end, b')') {
                return 0;
            }
            plutovg_color_init_rgba8(color, r, g, b, a);
        } else {
            let entry = COLORMAP.binary_search_by(|(n, _)| n.as_bytes().cmp(&name[..]));
            let Ok(entry) = entry else {
                return 0;
            };
            plutovg_color_init_argb32(color, 0xFF000000 | COLORMAP[entry].1);
        }
    }

    plutovg_skip_ws(data, &mut it, end);
    it as i32
}

/// `plutovg_gradient_type_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientType {
    Linear,
    Radial,
}

/// `plutovg_gradient_paint_t`
#[derive(Debug, Clone)]
pub struct GradientPaint {
    pub type_: GradientType,
    pub spread: SpreadMethod,
    pub matrix: Matrix,
    pub stops: Vec<GradientStop>,
    pub values: [f32; 6],
}

/// `plutovg_texture_paint_t`
#[derive(Debug, Clone)]
pub struct TexturePaint {
    pub type_: TextureType,
    pub opacity: f32,
    pub matrix: Matrix,
    pub surface: Option<Rc<Surface>>,
}

/// `struct plutovg_paint` (with its type: `plutovg_solid_paint_t`,
/// `plutovg_gradient_paint_t` or `plutovg_texture_paint_t`)
#[derive(Debug, Clone)]
pub enum Paint {
    Color(Color),
    Gradient(GradientPaint),
    Texture(TexturePaint),
}

/// Creates a solid RGB paint.
pub fn plutovg_paint_create_rgb(r: f32, g: f32, b: f32) -> Rc<Paint> {
    plutovg_paint_create_rgba(r, g, b, 1.0)
}

/// Creates a solid RGBA paint.
pub fn plutovg_paint_create_rgba(r: f32, g: f32, b: f32, a: f32) -> Rc<Paint> {
    Rc::new(Paint::Color(Color {
        r: plutovg_clamp(r, 0.0, 1.0),
        g: plutovg_clamp(g, 0.0, 1.0),
        b: plutovg_clamp(b, 0.0, 1.0),
        a: plutovg_clamp(a, 0.0, 1.0),
    }))
}

/// Creates a solid color paint.
pub fn plutovg_paint_create_color(color: &Color) -> Rc<Paint> {
    plutovg_paint_create_rgba(color.r, color.g, color.b, color.a)
}

/// `plutovg_gradient_create`
fn plutovg_gradient_create(
    type_: GradientType,
    spread: SpreadMethod,
    stops: &[GradientStop],
    matrix: Option<&Matrix>,
) -> GradientPaint {
    let mut gradient = GradientPaint {
        type_,
        spread,
        matrix: matrix.copied().unwrap_or(PLUTOVG_IDENTITY_MATRIX),
        stops: Vec::with_capacity(stops.len()),
        values: [0.0; 6],
    };

    let mut prev_offset: f32 = 0.0;
    for stop in stops {
        let s = GradientStop {
            offset: plutovg_max(prev_offset, plutovg_clamp(stop.offset, 0.0, 1.0)),
            color: Color {
                r: plutovg_clamp(stop.color.r, 0.0, 1.0),
                g: plutovg_clamp(stop.color.g, 0.0, 1.0),
                b: plutovg_clamp(stop.color.b, 0.0, 1.0),
                a: plutovg_clamp(stop.color.a, 0.0, 1.0),
            },
        };
        prev_offset = s.offset;
        gradient.stops.push(s);
    }

    gradient
}

/// Creates a linear gradient paint.
pub fn plutovg_paint_create_linear_gradient(
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    spread: SpreadMethod,
    stops: &[GradientStop],
    matrix: Option<&Matrix>,
) -> Rc<Paint> {
    let mut gradient = plutovg_gradient_create(GradientType::Linear, spread, stops, matrix);
    gradient.values[0] = x1;
    gradient.values[1] = y1;
    gradient.values[2] = x2;
    gradient.values[3] = y2;
    Rc::new(Paint::Gradient(gradient))
}

/// Creates a radial gradient paint.
pub fn plutovg_paint_create_radial_gradient(
    cx: f32,
    cy: f32,
    cr: f32,
    fx: f32,
    fy: f32,
    fr: f32,
    spread: SpreadMethod,
    stops: &[GradientStop],
    matrix: Option<&Matrix>,
) -> Rc<Paint> {
    let mut gradient = plutovg_gradient_create(GradientType::Radial, spread, stops, matrix);
    gradient.values[0] = cx;
    gradient.values[1] = cy;
    gradient.values[2] = cr;
    gradient.values[3] = fx;
    gradient.values[4] = fy;
    gradient.values[5] = fr;
    Rc::new(Paint::Gradient(gradient))
}

/// Creates a texture paint from a surface.
pub fn plutovg_paint_create_texture(
    surface: Option<Rc<Surface>>,
    type_: TextureType,
    opacity: f32,
    matrix: Option<&Matrix>,
) -> Rc<Paint> {
    Rc::new(Paint::Texture(TexturePaint {
        type_,
        opacity: plutovg_clamp(opacity, 0.0, 1.0),
        matrix: matrix.copied().unwrap_or(PLUTOVG_IDENTITY_MATRIX),
        surface,
    }))
}
