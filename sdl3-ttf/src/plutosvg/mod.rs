// Rust translation of source/plutosvg.c and source/plutosvg.h from
// PlutoSVG (0.0.6, as SDL_ttf's external/plutosvg pins it: commit
// 2983eb6919feea272d793bc386384e3f5b97b03c).
// Copyright (c) 2020-2025 Samuel Ugochukwu <sammycageagle@gmail.com>
// This is an altered (translated) version of the original software; it is
// used under the MIT License (see PLUTOSVG-LICENSE and LICENSE.txt).

//! PlutoSVG, the SVG library SDL_ttf renders OpenType SVG glyphs with (as
//! its CMake builds it: `PLUTOSVG_ENABLE_FREETYPE`, with the FreeType
//! hooks of [`ft`]): the SVG document parser (elements, attributes, the
//! `style` attribute's declarations, ids) and the renderer (`svg`, `g`,
//! `use`, `symbol`, `defs`, shapes, paths, `image`, linear and radial
//! gradients, fills and strokes, opacities, transforms, view boxes),
//! drawn with [`crate::plutovg`].
//!
//! Translation notes:
//!
//! * A document owns a copy of its data (C's refers to the caller's), and
//!   its elements, attributes and id table are arrays and indices.
//! * `render_use` sets the parent of the referenced element while it is
//!   rendered, as C does (the parents are `Cell`s).
//! * C renders an element tree recursively without a bound; this
//!   translation stops at a depth of [`MAX_RENDER_DEPTH`] (C's stack would
//!   overflow somewhat deeper).
//! * `plutosvg_document_load_from_file` is not translated (SDL_ttf does
//!   not use it).

#![allow(
    clippy::too_many_arguments,
    clippy::needless_late_init,
    clippy::collapsible_else_if
)]

pub mod ft;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use crate::plutovg::canvas::*;
use crate::plutovg::libm::{hypotf, powf};
use crate::plutovg::matrix::*;
use crate::plutovg::paint::*;
use crate::plutovg::path::*;
use crate::plutovg::surface::*;
use crate::plutovg::*;

pub const PLUTOSVG_VERSION_MAJOR: i32 = 0;
pub const PLUTOSVG_VERSION_MINOR: i32 = 0;
pub const PLUTOSVG_VERSION_MICRO: i32 = 6;
pub const PLUTOSVG_VERSION: i32 = plutovg_version_encode(
    PLUTOSVG_VERSION_MAJOR,
    PLUTOSVG_VERSION_MINOR,
    PLUTOSVG_VERSION_MICRO,
);
pub const PLUTOSVG_VERSION_STRING: &str = "0.0.6";

/// The depth of nested elements beyond which this translation does not
/// render (see the module documentation).
pub const MAX_RENDER_DEPTH: usize = 256;

/// `plutosvg_version`
pub fn plutosvg_version() -> i32 {
    PLUTOSVG_VERSION
}

/// `plutosvg_version_string`
pub fn plutosvg_version_string() -> &'static str {
    PLUTOSVG_VERSION_STRING
}

/// `plutosvg_palette_func_t`: the color of a palette entry (a CSS
/// variable name), if known.
pub type PaletteFunc<'a> = dyn FnMut(&[u8]) -> Option<Color> + 'a;

const TAG_UNKNOWN: i32 = 0;
const TAG_CIRCLE: i32 = 1;
const TAG_CLIP_PATH: i32 = 2; // TODO
const TAG_DEFS: i32 = 3;
const TAG_ELLIPSE: i32 = 4;
const TAG_G: i32 = 5;
const TAG_IMAGE: i32 = 6;
const TAG_LINE: i32 = 7;
const TAG_LINEAR_GRADIENT: i32 = 8;
const TAG_PATH: i32 = 9;
const TAG_POLYGON: i32 = 10;
const TAG_POLYLINE: i32 = 11;
const TAG_RADIAL_GRADIENT: i32 = 12;
const TAG_RECT: i32 = 13;
const TAG_STOP: i32 = 14;
const TAG_SVG: i32 = 15;
const TAG_SYMBOL: i32 = 16;
const TAG_USE: i32 = 17;

#[allow(dead_code)]
const ATTR_UNKNOWN: i32 = 0;
const ATTR_CLIP_PATH: i32 = 1;
const ATTR_CLIP_RULE: i32 = 2;
const ATTR_CLIP_PATH_UNITS: i32 = 3;
const ATTR_COLOR: i32 = 4;
const ATTR_CX: i32 = 5;
const ATTR_CY: i32 = 6;
const ATTR_D: i32 = 7;
const ATTR_DISPLAY: i32 = 8;
const ATTR_FILL: i32 = 9;
const ATTR_FILL_OPACITY: i32 = 10;
const ATTR_FILL_RULE: i32 = 11;
const ATTR_FX: i32 = 12;
const ATTR_FY: i32 = 13;
const ATTR_GRADIENT_TRANSFORM: i32 = 14;
const ATTR_GRADIENT_UNITS: i32 = 15;
const ATTR_HEIGHT: i32 = 16;
const ATTR_ID: i32 = 17;
const ATTR_OFFSET: i32 = 18;
const ATTR_OPACITY: i32 = 19;
const ATTR_POINTS: i32 = 20;
const ATTR_PRESERVE_ASPECT_RATIO: i32 = 21;
const ATTR_R: i32 = 22;
const ATTR_RX: i32 = 23;
const ATTR_RY: i32 = 24;
const ATTR_SPREAD_METHOD: i32 = 25;
const ATTR_STOP_COLOR: i32 = 26;
const ATTR_STOP_OPACITY: i32 = 27;
const ATTR_STROKE: i32 = 28;
const ATTR_STROKE_DASHARRAY: i32 = 29;
const ATTR_STROKE_DASHOFFSET: i32 = 30;
const ATTR_STROKE_LINECAP: i32 = 31;
const ATTR_STROKE_LINEJOIN: i32 = 32;
const ATTR_STROKE_MITERLIMIT: i32 = 33;
const ATTR_STROKE_OPACITY: i32 = 34;
const ATTR_STROKE_WIDTH: i32 = 35;
const ATTR_STYLE: i32 = 36;
const ATTR_TRANSFORM: i32 = 37;
const ATTR_VIEW_BOX: i32 = 38;
const ATTR_VISIBILITY: i32 = 39;
const ATTR_WIDTH: i32 = 40;
const ATTR_X: i32 = 41;
const ATTR_X1: i32 = 42;
const ATTR_X2: i32 = 43;
const ATTR_XLINK_HREF: i32 = 44;
const ATTR_Y: i32 = 45;
const ATTR_Y1: i32 = 46;
const ATTR_Y2: i32 = 47;

const MAX_NAME: usize = 20;

/// `lookupid`: the id of `data` in the sorted `table` (0 if not found).
fn lookupid(data: &[u8], table: &[(&str, i32)]) -> i32 {
    if data.len() > MAX_NAME {
        return 0;
    }

    match table.binary_search_by(|(name, _)| name.as_bytes().cmp(data)) {
        Ok(i) => table[i].1,
        Err(_) => 0,
    }
}

/// `elementid`
fn elementid(data: &[u8]) -> i32 {
    static TABLE: [(&str, i32); 17] = [
        ("circle", TAG_CIRCLE),
        ("clipPath", TAG_CLIP_PATH),
        ("defs", TAG_DEFS),
        ("ellipse", TAG_ELLIPSE),
        ("g", TAG_G),
        ("image", TAG_IMAGE),
        ("line", TAG_LINE),
        ("linearGradient", TAG_LINEAR_GRADIENT),
        ("path", TAG_PATH),
        ("polygon", TAG_POLYGON),
        ("polyline", TAG_POLYLINE),
        ("radialGradient", TAG_RADIAL_GRADIENT),
        ("rect", TAG_RECT),
        ("stop", TAG_STOP),
        ("svg", TAG_SVG),
        ("symbol", TAG_SYMBOL),
        ("use", TAG_USE),
    ];

    lookupid(data, &TABLE)
}

/// `attributeid`
fn attributeid(data: &[u8]) -> i32 {
    static TABLE: [(&str, i32); 47] = [
        ("clip-path", ATTR_CLIP_PATH),
        ("clip-rule", ATTR_CLIP_RULE),
        ("clipPathUnits", ATTR_CLIP_PATH_UNITS),
        ("color", ATTR_COLOR),
        ("cx", ATTR_CX),
        ("cy", ATTR_CY),
        ("d", ATTR_D),
        ("display", ATTR_DISPLAY),
        ("fill", ATTR_FILL),
        ("fill-opacity", ATTR_FILL_OPACITY),
        ("fill-rule", ATTR_FILL_RULE),
        ("fx", ATTR_FX),
        ("fy", ATTR_FY),
        ("gradientTransform", ATTR_GRADIENT_TRANSFORM),
        ("gradientUnits", ATTR_GRADIENT_UNITS),
        ("height", ATTR_HEIGHT),
        ("id", ATTR_ID),
        ("offset", ATTR_OFFSET),
        ("opacity", ATTR_OPACITY),
        ("points", ATTR_POINTS),
        ("preserveAspectRatio", ATTR_PRESERVE_ASPECT_RATIO),
        ("r", ATTR_R),
        ("rx", ATTR_RX),
        ("ry", ATTR_RY),
        ("spreadMethod", ATTR_SPREAD_METHOD),
        ("stop-color", ATTR_STOP_COLOR),
        ("stop-opacity", ATTR_STOP_OPACITY),
        ("stroke", ATTR_STROKE),
        ("stroke-dasharray", ATTR_STROKE_DASHARRAY),
        ("stroke-dashoffset", ATTR_STROKE_DASHOFFSET),
        ("stroke-linecap", ATTR_STROKE_LINECAP),
        ("stroke-linejoin", ATTR_STROKE_LINEJOIN),
        ("stroke-miterlimit", ATTR_STROKE_MITERLIMIT),
        ("stroke-opacity", ATTR_STROKE_OPACITY),
        ("stroke-width", ATTR_STROKE_WIDTH),
        ("style", ATTR_STYLE),
        ("transform", ATTR_TRANSFORM),
        ("viewBox", ATTR_VIEW_BOX),
        ("visibility", ATTR_VISIBILITY),
        ("width", ATTR_WIDTH),
        ("x", ATTR_X),
        ("x1", ATTR_X1),
        ("x2", ATTR_X2),
        ("xlink:href", ATTR_XLINK_HREF),
        ("y", ATTR_Y),
        ("y1", ATTR_Y1),
        ("y2", ATTR_Y2),
    ];

    lookupid(data, &TABLE)
}

/// `cssattributeid`
fn cssattributeid(data: &[u8]) -> i32 {
    static TABLE: [(&str, i32); 19] = [
        ("clip-path", ATTR_CLIP_PATH),
        ("clip-rule", ATTR_CLIP_RULE),
        ("color", ATTR_COLOR),
        ("display", ATTR_DISPLAY),
        ("fill", ATTR_FILL),
        ("fill-opacity", ATTR_FILL_OPACITY),
        ("fill-rule", ATTR_FILL_RULE),
        ("opacity", ATTR_OPACITY),
        ("stop-color", ATTR_STOP_COLOR),
        ("stop-opacity", ATTR_STOP_OPACITY),
        ("stroke", ATTR_STROKE),
        ("stroke-dasharray", ATTR_STROKE_DASHARRAY),
        ("stroke-dashoffset", ATTR_STROKE_DASHOFFSET),
        ("stroke-linecap", ATTR_STROKE_LINECAP),
        ("stroke-linejoin", ATTR_STROKE_LINEJOIN),
        ("stroke-miterlimit", ATTR_STROKE_MITERLIMIT),
        ("stroke-opacity", ATTR_STROKE_OPACITY),
        ("stroke-width", ATTR_STROKE_WIDTH),
        ("visibility", ATTR_VISIBILITY),
    ];

    lookupid(data, &TABLE)
}

/// `string_t`: a range of the document's data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Str {
    data: usize,
    length: usize,
}

/// `attribute_t`
#[derive(Debug, Clone, Copy)]
struct Attribute {
    id: i32,
    value: Str,
    next: Option<u32>,
}

/// `element_t`
#[derive(Debug, Clone)]
struct Element {
    id: i32,
    parent: Cell<Option<u32>>,
    last_child: Option<u32>,
    first_child: Option<u32>,
    next_sibling: Option<u32>,
    attributes: Option<u32>,
}

/// `struct plutosvg_document`
#[derive(Debug)]
pub struct Document {
    data: Vec<u8>,
    elements: Vec<Element>,
    attrs: Vec<Attribute>,
    path: RefCell<Path>,
    id_cache: Option<HashMap<Vec<u8>, u32>>,
    root_element: Option<u32>,
    width: f32,
    height: f32,
}

impl Document {
    #[inline]
    fn el(&self, e: u32) -> &Element {
        &self.elements[e as usize]
    }

    #[inline]
    fn s(&self, s: Str) -> &[u8] {
        &self.data[s.data..s.data + s.length]
    }
}

/// `find_attribute`
#[inline]
fn find_attribute(doc: &Document, element: u32, id: i32, inherit: bool) -> Option<Str> {
    let mut element = Some(element);
    let mut steps = 0;
    while let Some(e) = element {
        /* FIXME (upstream): `render_use` can make the parents a cycle (a
        `use` within the element it refers to); C loops forever then */
        steps += 1;
        if steps > doc.elements.len() {
            return None;
        }
        let mut attribute = doc.el(e).attributes;
        let mut skip = false;
        while let Some(a) = attribute {
            let a = &doc.attrs[a as usize];
            if a.id == id {
                let value = a.value;
                if inherit && doc.s(value) == b"inherit" {
                    skip = true;
                    break;
                }
                return Some(value);
            }

            attribute = a.next;
        }
        let _ = skip;

        element = doc.el(e).parent.get();
        if !inherit {
            break;
        }
    }
    None
}

/// `has_attribute`
#[inline]
fn has_attribute(doc: &Document, element: u32, id: i32) -> bool {
    let mut attribute = doc.el(element).attributes;
    while let Some(a) = attribute {
        let a = &doc.attrs[a as usize];
        if a.id == id {
            return true;
        }
        attribute = a.next;
    }

    false
}

/// `IS_NUM`
#[inline]
fn is_num(c: u8) -> bool {
    c.is_ascii_digit()
}

/// `parse_float`
#[inline]
fn parse_float(d: &[u8], begin: &mut usize, end: usize, number: &mut f32) -> bool {
    let mut it = *begin;
    let mut integer: f32 = 0.0;
    let mut fraction: f32 = 0.0;
    let mut exponent: f32 = 0.0;
    let mut sign: i32 = 1;
    let mut expsign: i32 = 1;

    if it < end && d[it] == b'+' {
        it += 1;
    } else if it < end && d[it] == b'-' {
        it += 1;
        sign = -1;
    }

    if it >= end || (d[it] != b'.' && !is_num(d[it])) {
        return false;
    }
    if is_num(d[it]) {
        loop {
            integer = 10.0 * integer + (d[it] - b'0') as f32;
            it += 1;
            if !(it < end && is_num(d[it])) {
                break;
            }
        }
    }

    if it < end && d[it] == b'.' {
        it += 1;
        if it >= end || !is_num(d[it]) {
            return false;
        }
        let mut divisor: f32 = 1.0;
        loop {
            fraction = 10.0 * fraction + (d[it] - b'0') as f32;
            it += 1;
            divisor *= 10.0;
            if !(it < end && is_num(d[it])) {
                break;
            }
        }
        fraction /= divisor;
    }

    if it + 1 < end && (d[it] == b'e' || d[it] == b'E') && (d[it + 1] != b'x' && d[it + 1] != b'm')
    {
        it += 1;
        if it < end && d[it] == b'+' {
            it += 1;
        } else if it < end && d[it] == b'-' {
            it += 1;
            expsign = -1;
        }

        if it >= end || !is_num(d[it]) {
            return false;
        }
        loop {
            exponent = 10.0 * exponent + (d[it] - b'0') as f32;
            it += 1;
            if !(it < end && is_num(d[it])) {
                break;
            }
        }
    }

    *begin = it;
    *number = sign as f32 * (integer + fraction);
    if exponent != 0.0 {
        *number *= powf(10.0, expsign as f32 * exponent);
    }
    *number >= -f32::MAX && *number <= f32::MAX
}

/// `skip_string`
#[inline]
fn skip_string(d: &[u8], begin: &mut usize, end: usize, s: &[u8]) -> bool {
    let mut it = *begin;
    let mut k = 0;
    while it < end && k < s.len() && d[it] == s[k] {
        k += 1;
        it += 1;
    }

    if k == s.len() {
        *begin = it;
        return true;
    }

    false
}

/// `string_find`
#[inline]
fn string_find(d: &[u8], mut it: usize, end: usize, s: &[u8]) -> Option<usize> {
    while it < end {
        let begin = it;
        if skip_string(d, &mut it, end, s) {
            return Some(begin);
        }
        it += 1;
    }

    None
}

/// `skip_delim`
#[inline]
fn skip_delim(d: &[u8], begin: &mut usize, end: usize, delim: u8) -> bool {
    let it = *begin;
    if it < end && d[it] == delim {
        *begin = it + 1;
        return true;
    }

    false
}

/// `IS_WS`
#[inline]
fn is_ws(c: u8) -> bool {
    c == b' ' || c == b'\t' || c == b'\n' || c == b'\r'
}

/// `skip_ws`
#[inline]
fn skip_ws(d: &[u8], begin: &mut usize, end: usize) -> bool {
    let mut it = *begin;
    while it < end && is_ws(d[it]) {
        it += 1;
    }
    *begin = it;
    it < end
}

/// `skip_ws_delim`
#[inline]
fn skip_ws_delim(d: &[u8], begin: &mut usize, end: usize, delim: u8) -> bool {
    let mut it = *begin;
    if it < end && !is_ws(d[it]) && d[it] != delim {
        return false;
    }
    if skip_ws(d, &mut it, end) && skip_delim(d, &mut it, end, delim) {
        skip_ws(d, &mut it, end);
    }

    *begin = it;
    it < end
}

/// `skip_ws_comma`
#[inline]
fn skip_ws_comma(d: &[u8], begin: &mut usize, end: usize) -> bool {
    skip_ws_delim(d, begin, end, b',')
}

/// `rtrim`
#[inline]
fn rtrim(d: &[u8], begin: usize, mut end: usize) -> usize {
    while end > begin && is_ws(d[end - 1]) {
        end -= 1;
    }
    end
}

/// `CLAMP`
#[inline]
fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if hi < v {
        hi
    } else {
        v
    }
}

/// `MIN`
#[inline]
fn min(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}

/// `MAX`
#[inline]
fn max(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

/// `parse_number`
fn parse_number(
    doc: &Document,
    element: u32,
    id: i32,
    number: &mut f32,
    percent: bool,
    inherit: bool,
) -> bool {
    let Some(value) = find_attribute(doc, element, id, inherit) else {
        return false;
    };
    let d = &doc.data;
    let mut it = value.data;
    let end = it + value.length;
    if !parse_float(d, &mut it, end, number) {
        return false;
    }
    if percent {
        if skip_delim(d, &mut it, end, b'%') {
            *number /= 100.0;
        }
        *number = clamp(*number, 0.0, 1.0);
    }

    true
}

/// `length_type_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LengthType {
    Unknown,
    Fixed,
    Percent,
}

/// `length_t`
#[derive(Debug, Clone, Copy)]
struct Length {
    value: f32,
    type_: LengthType,
}

/// `{v, length_type_fixed}`
const fn fixed(value: f32) -> Length {
    Length {
        value,
        type_: LengthType::Fixed,
    }
}

/// `{v, length_type_percent}`
const fn percent(value: f32) -> Length {
    Length {
        value,
        type_: LengthType::Percent,
    }
}

/// `is_length_zero`
#[inline]
fn is_length_zero(length: &Length) -> bool {
    length.value == 0.0
}

/// `is_length_valid`
#[inline]
fn is_length_valid(length: &Length) -> bool {
    length.type_ != LengthType::Unknown
}

/// `parse_length_value`
fn parse_length_value(
    d: &[u8],
    begin: &mut usize,
    end: usize,
    length: &mut Length,
    negative: bool,
) -> bool {
    let mut value: f32 = 0.0;
    let mut it = *begin;
    if !parse_float(d, &mut it, end, &mut value) {
        return false;
    }
    if !negative && value < 0.0 {
        return false;
    }

    let mut units = [0u8; 2];
    if it < end {
        units[0] = d[it];
    }
    if it + 1 < end {
        units[1] = d[it + 1];
    }

    const DPI: f32 = 96.0;
    match units[0] {
        b'%' => {
            length.value = value;
            length.type_ = LengthType::Percent;
            it += 1;
        }
        b'p' => {
            if units[1] == b'x' {
                length.value = value;
            } else if units[1] == b'c' {
                length.value = value * DPI / 6.0;
            } else if units[1] == b't' {
                length.value = value * DPI / 72.0;
            } else {
                return false;
            }
            length.type_ = LengthType::Fixed;
            it += 2;
        }
        b'i' => {
            if units[1] == b'n' {
                length.value = value * DPI;
            } else {
                return false;
            }
            length.type_ = LengthType::Fixed;
            it += 2;
        }
        b'c' => {
            if units[1] == b'm' {
                length.value = value * DPI / 2.54;
            } else {
                return false;
            }
            length.type_ = LengthType::Fixed;
            it += 2;
        }
        b'm' => {
            if units[1] == b'm' {
                length.value = value * DPI / 25.4;
            } else {
                return false;
            }
            length.type_ = LengthType::Fixed;
            it += 2;
        }
        _ => {
            length.value = value;
            length.type_ = LengthType::Fixed;
        }
    }

    *begin = it;
    true
}

/// `parse_length`
fn parse_length(
    doc: &Document,
    element: u32,
    id: i32,
    length: &mut Length,
    negative: bool,
    inherit: bool,
) -> bool {
    let Some(value) = find_attribute(doc, element, id, inherit) else {
        return false;
    };
    let mut it = value.data;
    let end = it + value.length;
    if parse_length_value(&doc.data, &mut it, end, length, negative) {
        return it == end;
    }
    false
}

/// `convert_length`
#[inline]
fn convert_length(length: &Length, maximum: f32) -> f32 {
    if length.type_ == LengthType::Percent {
        return length.value * maximum / 100.0;
    }
    length.value
}

/// `color_type_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorType {
    Fixed,
    Current,
}

/// `color_t`
#[derive(Debug, Clone, Copy)]
struct SvgColor {
    type_: ColorType,
    value: u32,
}

/// `paint_type_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PaintType {
    None,
    Color,
    Url,
    Var,
}

/// `paint_t`
#[derive(Debug, Clone, Copy)]
struct SvgPaint {
    type_: PaintType,
    color: SvgColor,
    id: Str,
}

/// `parse_color_value`
fn parse_color_value(d: &[u8], begin: &mut usize, end: usize, color: &mut SvgColor) -> bool {
    let mut it = *begin;
    if skip_string(d, &mut it, end, b"currentColor") {
        color.type_ = ColorType::Current;
        color.value = 0xFF000000;
    } else {
        let mut value = Color::default();
        let length = plutovg_color_parse(&mut value, &d[it..end]);
        if length == 0 {
            return false;
        }
        color.type_ = ColorType::Fixed;
        color.value = plutovg_color_to_argb32(&value);
        it += length as usize;
    }

    *begin = it;
    skip_ws(d, begin, end);
    true
}

/// `parse_color`
fn parse_color(doc: &Document, element: u32, id: i32, color: &mut SvgColor, inherit: bool) -> bool {
    let Some(value) = find_attribute(doc, element, id, inherit) else {
        return false;
    };
    let mut it = value.data;
    let end = it + value.length;
    if parse_color_value(&doc.data, &mut it, end, color) {
        return it == end;
    }
    false
}

/// `parse_url_value`
fn parse_url_value(d: &[u8], begin: &mut usize, end: usize, id: &mut Str) -> bool {
    let mut it = *begin;
    if !skip_string(d, &mut it, end, b"url")
        || !skip_ws(d, &mut it, end)
        || !skip_delim(d, &mut it, end, b'(')
        || !skip_ws(d, &mut it, end)
    {
        return false;
    }

    if !skip_delim(d, &mut it, end, b'#') {
        return false;
    }
    id.data = it;
    id.length = 0;
    while it < end && d[it] != b')' {
        id.length += 1;
        it += 1;
    }

    if !skip_delim(d, &mut it, end, b')') {
        return false;
    }
    *begin = it;
    skip_ws(d, begin, end);
    true
}

/// `IS_ALPHA`
#[inline]
fn is_alpha(c: u8) -> bool {
    c.is_ascii_alphabetic()
}

/// `IS_STARTNAMECHAR`
#[inline]
fn is_startnamechar(c: u8) -> bool {
    is_alpha(c) || c == b'_' || c == b':'
}

/// `IS_NAMECHAR`
#[inline]
fn is_namechar(c: u8) -> bool {
    is_startnamechar(c) || is_num(c) || c == b'-' || c == b'.'
}

/// `parse_paint`
fn parse_paint(doc: &Document, element: u32, id: i32, paint: &mut SvgPaint) -> bool {
    let Some(value) = find_attribute(doc, element, id, true) else {
        return false;
    };
    let d = &doc.data;
    let mut it = value.data;
    let end = it + value.length;
    if skip_string(d, &mut it, end, b"none") {
        paint.type_ = PaintType::None;
        return !skip_ws(d, &mut it, end);
    }

    if parse_url_value(d, &mut it, end, &mut paint.id) {
        paint.type_ = PaintType::Url;
        paint.color.value = 0x00000000;
        if skip_ws(d, &mut it, end) && !parse_color_value(d, &mut it, end, &mut paint.color) {
            return false;
        }

        return it == end;
    }

    if skip_string(d, &mut it, end, b"var") {
        if !skip_ws(d, &mut it, end)
            || !skip_delim(d, &mut it, end, b'(')
            || !skip_ws(d, &mut it, end)
        {
            return false;
        }

        if !skip_string(d, &mut it, end, b"--") {
            return false;
        }
        let begin = it;
        while it < end && is_namechar(d[it]) {
            it += 1;
        }
        paint.type_ = PaintType::Var;
        paint.id.data = begin;
        paint.id.length = it - begin;
        paint.color.value = 0x00000000;
        skip_ws(d, &mut it, end);
        if skip_delim(d, &mut it, end, b',') {
            skip_ws(d, &mut it, end);
            if !parse_color_value(d, &mut it, end, &mut paint.color) {
                return false;
            }
        }

        return skip_delim(d, &mut it, end, b')') && !skip_ws(d, &mut it, end);
    }

    if parse_color_value(d, &mut it, end, &mut paint.color) {
        paint.type_ = PaintType::Color;
        return it == end;
    }

    false
}

/// `parse_view_box`
fn parse_view_box(doc: &Document, element: u32, id: i32, view_box: &mut Rect) -> bool {
    let Some(value) = find_attribute(doc, element, id, false) else {
        return false;
    };
    let d = &doc.data;
    let mut it = value.data;
    let end = it + value.length;

    let mut x = 0.0;
    let mut y = 0.0;
    let mut w = 0.0;
    let mut h = 0.0;
    if !parse_float(d, &mut it, end, &mut x)
        || !skip_ws_comma(d, &mut it, end)
        || !parse_float(d, &mut it, end, &mut y)
        || !skip_ws_comma(d, &mut it, end)
        || !parse_float(d, &mut it, end, &mut w)
        || !skip_ws_comma(d, &mut it, end)
        || !parse_float(d, &mut it, end, &mut h)
        || skip_ws(d, &mut it, end)
    {
        return false;
    }

    if w <= 0.0 || h <= 0.0 {
        return false;
    }
    view_box.x = x;
    view_box.y = y;
    view_box.w = w;
    view_box.h = h;
    true
}

/// `view_align_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewAlign {
    None,
    XMinYMin,
    XMidYMin,
    XMaxYMin,
    XMinYMid,
    XMidYMid,
    XMaxYMid,
    XMinYMax,
    XMidYMax,
    XMaxYMax,
}

/// `view_scale_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewScale {
    Meet,
    Slice,
}

/// `view_position_t`
#[derive(Debug, Clone, Copy)]
struct ViewPosition {
    align: ViewAlign,
    scale: ViewScale,
}

/// `parse_view_position`
fn parse_view_position(doc: &Document, element: u32, id: i32, position: &mut ViewPosition) -> bool {
    let Some(value) = find_attribute(doc, element, id, false) else {
        return false;
    };
    let d = &doc.data;
    let mut it = value.data;
    let end = it + value.length;
    let aligns: [(&[u8], ViewAlign); 10] = [
        (b"none", ViewAlign::None),
        (b"xMinYMin", ViewAlign::XMinYMin),
        (b"xMidYMin", ViewAlign::XMidYMin),
        (b"xMaxYMin", ViewAlign::XMaxYMin),
        (b"xMinYMid", ViewAlign::XMinYMid),
        (b"xMidYMid", ViewAlign::XMidYMid),
        (b"xMaxYMid", ViewAlign::XMaxYMid),
        (b"xMinYMax", ViewAlign::XMinYMax),
        (b"xMidYMax", ViewAlign::XMidYMax),
        (b"xMaxYMax", ViewAlign::XMaxYMax),
    ];
    let mut found = false;
    for (name, align) in aligns {
        if skip_string(d, &mut it, end, name) {
            position.align = align;
            found = true;
            break;
        }
    }
    if !found {
        return false;
    }
    position.scale = ViewScale::Meet;
    if position.align != ViewAlign::None {
        skip_ws(d, &mut it, end);
        if skip_string(d, &mut it, end, b"meet") {
            position.scale = ViewScale::Meet;
        } else if skip_string(d, &mut it, end, b"slice") {
            position.scale = ViewScale::Slice;
        }
    }

    !skip_ws(d, &mut it, end)
}

/// `parse_transform`
fn parse_transform(doc: &Document, element: u32, id: i32, matrix: &mut Matrix) -> bool {
    let Some(value) = find_attribute(doc, element, id, false) else {
        return false;
    };
    plutovg_matrix_parse(matrix, doc.s(value))
}

/// `parse_points`
fn parse_points(doc: &Document, element: u32, id: i32, path: &mut Path) -> bool {
    let Some(value) = find_attribute(doc, element, id, false) else {
        return false;
    };
    let d = &doc.data;
    let mut it = value.data;
    let end = it + value.length;

    let mut requires_move = true;
    while it < end {
        let mut x = 0.0;
        let mut y = 0.0;
        if !parse_float(d, &mut it, end, &mut x)
            || !skip_ws_comma(d, &mut it, end)
            || !parse_float(d, &mut it, end, &mut y)
        {
            return false;
        }

        skip_ws_comma(d, &mut it, end);
        if requires_move {
            plutovg_path_move_to(path, x, y);
        } else {
            plutovg_path_line_to(path, x, y);
        }
        requires_move = false;
    }

    if doc.el(element).id == TAG_POLYGON {
        plutovg_path_close(path);
    }
    true
}

/// `parse_path`
fn parse_path(doc: &Document, element: u32, id: i32, path: &mut Path) -> bool {
    let Some(value) = find_attribute(doc, element, id, false) else {
        return false;
    };
    plutovg_path_parse(path, doc.s(value))
}

const MAX_DASHES: usize = 128;

/// `stroke_dash_array_t`
struct StrokeDashArray {
    data: [Length; MAX_DASHES],
    size: usize,
}

/// `parse_dash_array`
fn parse_dash_array(
    doc: &Document,
    element: u32,
    id: i32,
    dash_array: &mut StrokeDashArray,
) -> bool {
    let Some(value) = find_attribute(doc, element, id, true) else {
        return false;
    };
    let d = &doc.data;
    let mut it = value.data;
    let end = it + value.length;
    while it < end && dash_array.size < MAX_DASHES {
        if !parse_length_value(
            d,
            &mut it,
            end,
            &mut dash_array.data[dash_array.size],
            false,
        ) {
            return false;
        }
        skip_ws_comma(d, &mut it, end);
        dash_array.size += 1;
    }

    true
}

/// `parse_line_cap`
fn parse_line_cap(doc: &Document, element: u32, id: i32, line_cap: &mut LineCap) -> bool {
    let Some(value) = find_attribute(doc, element, id, true) else {
        return false;
    };
    let d = &doc.data;
    let mut it = value.data;
    let end = it + value.length;
    if skip_string(d, &mut it, end, b"butt") {
        *line_cap = LineCap::Butt;
    } else if skip_string(d, &mut it, end, b"round") {
        *line_cap = LineCap::Round;
    } else if skip_string(d, &mut it, end, b"square") {
        *line_cap = LineCap::Square;
    }
    !skip_ws(d, &mut it, end)
}

/// `parse_line_join`
fn parse_line_join(doc: &Document, element: u32, id: i32, line_join: &mut LineJoin) -> bool {
    let Some(value) = find_attribute(doc, element, id, true) else {
        return false;
    };
    let d = &doc.data;
    let mut it = value.data;
    let end = it + value.length;
    if skip_string(d, &mut it, end, b"miter") {
        *line_join = LineJoin::Miter;
    } else if skip_string(d, &mut it, end, b"round") {
        *line_join = LineJoin::Round;
    } else if skip_string(d, &mut it, end, b"bevel") {
        *line_join = LineJoin::Bevel;
    }
    !skip_ws(d, &mut it, end)
}

/// `parse_fill_rule`
fn parse_fill_rule(doc: &Document, element: u32, id: i32, fill_rule: &mut FillRule) -> bool {
    let Some(value) = find_attribute(doc, element, id, true) else {
        return false;
    };
    let d = &doc.data;
    let mut it = value.data;
    let end = it + value.length;
    if skip_string(d, &mut it, end, b"nonzero") {
        *fill_rule = FillRule::NonZero;
    } else if skip_string(d, &mut it, end, b"evenodd") {
        *fill_rule = FillRule::EvenOdd;
    }
    !skip_ws(d, &mut it, end)
}

/// `parse_spread_method`
fn parse_spread_method(
    doc: &Document,
    element: u32,
    id: i32,
    spread_method: &mut SpreadMethod,
) -> bool {
    let Some(value) = find_attribute(doc, element, id, false) else {
        return false;
    };
    let d = &doc.data;
    let mut it = value.data;
    let end = it + value.length;
    if skip_string(d, &mut it, end, b"pad") {
        *spread_method = SpreadMethod::Pad;
    } else if skip_string(d, &mut it, end, b"reflect") {
        *spread_method = SpreadMethod::Reflect;
    } else if skip_string(d, &mut it, end, b"repeat") {
        *spread_method = SpreadMethod::Repeat;
    }
    !skip_ws(d, &mut it, end)
}

/// `display_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Display {
    Inline,
    None,
}

/// `parse_display`
fn parse_display(doc: &Document, element: u32, id: i32, display: &mut Display) -> bool {
    let Some(value) = find_attribute(doc, element, id, false) else {
        return false;
    };
    let d = &doc.data;
    let mut it = value.data;
    let end = it + value.length;
    if skip_string(d, &mut it, end, b"inline") {
        *display = Display::Inline;
    } else if skip_string(d, &mut it, end, b"none") {
        *display = Display::None;
    }
    !skip_ws(d, &mut it, end)
}

/// `visibility_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Visibility {
    Visible,
    Hidden,
    Collapse,
}

/// `parse_visibility`
fn parse_visibility(doc: &Document, element: u32, id: i32, visibility: &mut Visibility) -> bool {
    let Some(value) = find_attribute(doc, element, id, true) else {
        return false;
    };
    let d = &doc.data;
    let mut it = value.data;
    let end = it + value.length;
    if skip_string(d, &mut it, end, b"visible") {
        *visibility = Visibility::Visible;
    } else if skip_string(d, &mut it, end, b"hidden") {
        *visibility = Visibility::Hidden;
    } else if skip_string(d, &mut it, end, b"collapse") {
        *visibility = Visibility::Collapse;
    }
    !skip_ws(d, &mut it, end)
}

/// `units_type_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnitsType {
    ObjectBoundingBox,
    UserSpaceOnUse,
}

/// `parse_units_type`
fn parse_units_type(doc: &Document, element: u32, id: i32, units_type: &mut UnitsType) -> bool {
    let Some(value) = find_attribute(doc, element, id, false) else {
        return false;
    };
    let d = &doc.data;
    let mut it = value.data;
    let end = it + value.length;
    if skip_string(d, &mut it, end, b"objectBoundingBox") {
        *units_type = UnitsType::ObjectBoundingBox;
    } else if skip_string(d, &mut it, end, b"userSpaceOnUse") {
        *units_type = UnitsType::UserSpaceOnUse;
    }
    !skip_ws(d, &mut it, end)
}

/// `plutosvg_document_create`
fn plutosvg_document_create(data: Vec<u8>, width: f32, height: f32) -> Document {
    Document {
        data,
        elements: Vec::new(),
        attrs: Vec::new(),
        path: RefCell::new(plutovg_path_create()),
        id_cache: None,
        root_element: None,
        width,
        height,
    }
}

/// `plutosvg_document_destroy`
pub fn plutosvg_document_destroy(document: Option<Document>) {
    drop(document);
}

/// `add_attribute`
fn add_attribute(element: u32, document: &mut Document, id: i32, data: usize, length: usize) {
    let next = document.elements[element as usize].attributes;
    document.attrs.push(Attribute {
        id,
        value: Str { data, length },
        next,
    });
    document.elements[element as usize].attributes = Some((document.attrs.len() - 1) as u32);
}

/// `IS_CSS_STARTNAMECHAR`
#[inline]
fn is_css_startnamechar(c: u8) -> bool {
    is_alpha(c) || c == b'_'
}

/// `IS_CSS_NAMECHAR`
#[inline]
fn is_css_namechar(c: u8) -> bool {
    is_css_startnamechar(c) || is_num(c) || c == b'-'
}

/// `parse_style`
fn parse_style(
    d: &[u8],
    data_start: usize,
    length: usize,
    element: Option<u32>,
    document: &mut Document,
) {
    let mut it = data_start;
    let end = it + length;
    while it < end && is_css_startnamechar(d[it]) {
        let mut data = it;
        it += 1;
        while it < end && is_css_namechar(d[it]) {
            it += 1;
        }
        let id = cssattributeid(&d[data..it]);
        skip_ws(d, &mut it, end);
        if it >= end || d[it] != b':' {
            return;
        }
        it += 1;
        skip_ws(d, &mut it, end);
        data = it;
        while it < end && d[it] != b';' {
            it += 1;
        }
        let length = rtrim(d, data, it) - data;
        if id != 0 {
            if let Some(element) = element {
                add_attribute(element, document, id, data, length);
            }
        }
        skip_ws_delim(d, &mut it, end, b';');
    }
}

/// `parse_attributes` (`document` is `None` for the XML declaration's)
fn parse_attributes(
    d: &[u8],
    begin: &mut usize,
    end: usize,
    element: Option<u32>,
    mut document: Option<&mut Document>,
) -> bool {
    let mut it = *begin;
    while it < end && is_startnamechar(d[it]) {
        let mut data = it;
        it += 1;
        while it < end && is_namechar(d[it]) {
            it += 1;
        }
        let id = attributeid(&d[data..it]);
        skip_ws(d, &mut it, end);
        if it >= end || d[it] != b'=' {
            return false;
        }
        it += 1;
        skip_ws(d, &mut it, end);
        if it >= end || (d[it] != b'"' && d[it] != b'\'') {
            return false;
        }
        let quote = d[it];
        it += 1;
        skip_ws(d, &mut it, end);
        data = it;
        while it < end && d[it] != quote {
            it += 1;
        }
        if it >= end || d[it] != quote {
            return false;
        }
        let length = rtrim(d, data, it) - data;
        if id != 0 {
            if let (Some(element), Some(document)) = (element, document.as_deref_mut()) {
                if id == ATTR_ID {
                    let id_cache = document.id_cache.get_or_insert_with(HashMap::new);
                    id_cache.insert(d[data..data + length].to_vec(), element);
                } else if id == ATTR_STYLE {
                    parse_style(d, data, length, Some(element), document);
                } else {
                    add_attribute(element, document, id, data, length);
                }
            }
        }

        it += 1;
        skip_ws(d, &mut it, end);
    }

    *begin = it;
    true
}

/// Loads an SVG document from data (copied): `width` and `height` are the
/// sizes percentages of the root element's are relative to. `None` if the
/// data is not a well-formed SVG document.
pub fn plutosvg_document_load_from_data(data: &[u8], width: f32, height: f32) -> Option<Document> {
    let mut owned = Vec::new();
    owned.try_reserve_exact(data.len()).ok()?;
    owned.extend_from_slice(data);
    let mut document = plutosvg_document_create(owned, width, height);
    if plutosvg_document_parse(&mut document) {
        Some(document)
    } else {
        /* error: */
        plutosvg_document_destroy(Some(document));
        None
    }
}

/// The parser of `plutosvg_document_load_from_data` (`false` for its
/// `goto error`).
fn plutosvg_document_parse(document: &mut Document) -> bool {
    /* (the data is moved out while the document is built) */
    let data = std::mem::take(&mut document.data);
    let ok = plutosvg_document_parse_data(&data, document);
    document.data = data;
    if !ok {
        return false;
    }

    let root = document.root_element.unwrap_or(0);
    let mut w = percent(100.0);
    let mut h = percent(100.0);

    parse_length(document, root, ATTR_WIDTH, &mut w, false, false);
    parse_length(document, root, ATTR_HEIGHT, &mut h, false, false);

    let mut intrinsic_width = convert_length(&w, document.width);
    let mut intrinsic_height = convert_length(&h, document.height);
    if intrinsic_width <= 0.0 || intrinsic_height <= 0.0 {
        let mut view_box = Rect::default();
        if parse_view_box(document, root, ATTR_VIEW_BOX, &mut view_box) {
            let intrinsic_ratio = view_box.w / view_box.h;
            if intrinsic_width <= 0.0 && intrinsic_height > 0.0 {
                intrinsic_width = intrinsic_height * intrinsic_ratio;
            } else if intrinsic_width > 0.0 && intrinsic_height <= 0.0 {
                intrinsic_height = intrinsic_width / intrinsic_ratio;
            } else {
                intrinsic_width = view_box.w;
                intrinsic_height = view_box.h;
            }
        } else {
            if intrinsic_width == -1.0 {
                intrinsic_width = 300.0;
            }
            if intrinsic_height == -1.0 {
                intrinsic_height = 150.0;
            }
        }
    }

    if intrinsic_width <= 0.0 || intrinsic_height <= 0.0 {
        return false;
    }
    document.width = intrinsic_width;
    document.height = intrinsic_height;
    true
}

/// The element and attribute parsing loop of
/// `plutosvg_document_load_from_data` (`true` if it reached the end of a
/// complete document).
fn plutosvg_document_parse_data(d: &[u8], document: &mut Document) -> bool {
    let mut it = 0;
    let end = d.len();

    let mut current: Option<u32> = None;
    let mut ignoring: i32 = 0;
    while it < end {
        if current.is_none() {
            while it < end && is_ws(d[it]) {
                it += 1;
            }
            if it >= end {
                break;
            }
        } else {
            while it < end && d[it] != b'<' {
                it += 1;
            }
        }

        if it >= end || d[it] != b'<' {
            return false;
        }
        it += 1;
        if it < end && d[it] == b'?' {
            it += 1;
            if !skip_string(d, &mut it, end, b"xml") {
                return false;
            }
            skip_ws(d, &mut it, end);
            if !parse_attributes(d, &mut it, end, None, None) {
                return false;
            }
            if !skip_string(d, &mut it, end, b"?>") {
                return false;
            }
            skip_ws(d, &mut it, end);
            continue;
        }

        if it < end && d[it] == b'!' {
            it += 1;
            if skip_string(d, &mut it, end, b"--") {
                let Some(begin) = string_find(d, it, end, b"-->") else {
                    return false;
                };
                it = begin + 3;
                skip_ws(d, &mut it, end);
                continue;
            }

            if skip_string(d, &mut it, end, b"[CDATA[") {
                let Some(begin) = string_find(d, it, end, b"]]>") else {
                    return false;
                };
                it = begin + 3;
                skip_ws(d, &mut it, end);
                continue;
            }

            if skip_string(d, &mut it, end, b"DOCTYPE") {
                while it < end && d[it] != b'>' {
                    if d[it] == b'[' {
                        it += 1;
                        let mut depth = 1;
                        while it < end && depth > 0 {
                            if d[it] == b'[' {
                                depth += 1;
                            } else if d[it] == b']' {
                                depth -= 1;
                            }
                            it += 1;
                        }
                    } else {
                        it += 1;
                    }
                }

                if !skip_delim(d, &mut it, end, b'>') {
                    return false;
                }
                skip_ws(d, &mut it, end);
                continue;
            }

            return false;
        }

        if it < end && d[it] == b'/' {
            if current.is_none() && ignoring == 0 {
                return false;
            }
            it += 1;
            if it >= end || !is_startnamechar(d[it]) {
                return false;
            }
            let begin = it;
            it += 1;
            while it < end && is_namechar(d[it]) {
                it += 1;
            }
            let name_end = it;
            skip_ws(d, &mut it, end);
            if it >= end || d[it] != b'>' {
                return false;
            }
            if ignoring == 0 {
                /* (C passes the length up to `it`, after the spaces) */
                let id = elementid(&d[begin..it]);
                let _ = name_end;
                let Some(cur) = current else {
                    return false;
                };
                if id != document.elements[cur as usize].id {
                    return false;
                }
                current = document.elements[cur as usize].parent.get();
            } else {
                ignoring -= 1;
            }

            it += 1;
            continue;
        }

        if it >= end || !is_startnamechar(d[it]) {
            return false;
        }
        let begin = it;
        it += 1;
        while it < end && is_namechar(d[it]) {
            it += 1;
        }
        let mut element: Option<u32> = None;
        if ignoring > 0 {
            ignoring += 1;
        } else {
            let id = elementid(&d[begin..it]);
            if id == TAG_UNKNOWN {
                ignoring = 1;
            } else {
                if document.root_element.is_some() && current.is_none() {
                    return false;
                }
                let index = document.elements.len() as u32;
                document.elements.push(Element {
                    id,
                    parent: Cell::new(None),
                    next_sibling: None,
                    first_child: None,
                    last_child: None,
                    attributes: None,
                });
                element = Some(index);
                if document.root_element.is_none() {
                    if id != TAG_SVG {
                        return false;
                    }
                    document.root_element = Some(index);
                } else {
                    let cur = current.unwrap_or(0);
                    document.elements[index as usize].parent.set(Some(cur));
                    if let Some(last) = document.elements[cur as usize].last_child {
                        document.elements[last as usize].next_sibling = Some(index);
                        document.elements[cur as usize].last_child = Some(index);
                    } else {
                        document.elements[cur as usize].last_child = Some(index);
                        document.elements[cur as usize].first_child = Some(index);
                    }
                }
            }
        }

        skip_ws(d, &mut it, end);
        if !parse_attributes(d, &mut it, end, element, Some(document)) {
            return false;
        }
        if it < end && d[it] == b'>' {
            if element.is_some() {
                current = element;
            }
            it += 1;
            continue;
        }

        if it < end && d[it] == b'/' {
            it += 1;
            if it >= end || d[it] != b'>' {
                return false;
            }
            if ignoring > 0 {
                ignoring -= 1;
            }
            it += 1;
            continue;
        }

        return false;
    }

    it == end && ignoring == 0 && current.is_none() && document.root_element.is_some()
}

/// `render_mode_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderMode {
    Painting,
    #[allow(dead_code)]
    Clipping,
    Bounding,
}

/// `render_state_t` (its parent is the caller's state; the chain of
/// elements is the context's stack)
#[derive(Debug, Clone, Copy)]
struct RenderState {
    element: u32,
    mode: RenderMode,
    opacity: f32,

    view_width: f32,
    view_height: f32,

    matrix: Matrix,
    extents: Rect,
}

/// `INVALID_RECT`
const INVALID_RECT: Rect = plutovg_make_rect(0.0, 0.0, -1.0, -1.0);
/// `EMPTY_RECT`
const EMPTY_RECT: Rect = plutovg_make_rect(0.0, 0.0, 0.0, 0.0);

/// `IS_INVALID_RECT`
#[inline]
fn is_invalid_rect(rect: &Rect) -> bool {
    rect.w < 0.0 || rect.h < 0.0
}

/// `render_context_t` (and the chain of the states' elements)
struct RenderContext<'a, 'c, 'p> {
    document: &'a Document,
    canvas: Option<&'c mut Canvas>,
    current_color: Option<Color>,
    palette_func: Option<&'c mut PaletteFunc<'p>>,
    stack: Vec<u32>,
}

/// `render_state_begin`: a state for `element`, child of `parent`.
fn render_state_begin(
    element: u32,
    parent: &RenderState,
    context: &mut RenderContext<'_, '_, '_>,
) -> RenderState {
    let doc = context.document;
    let mut state = RenderState {
        element,
        mode: parent.mode,
        opacity: parent.opacity,
        matrix: parent.matrix,
        extents: INVALID_RECT,

        view_width: parent.view_width,
        view_height: parent.view_height,
    };

    if doc.el(element).parent.get().is_some()
        && parse_transform(doc, element, ATTR_TRANSFORM, &mut state.matrix)
    {
        let left = state.matrix;
        plutovg_matrix_multiply(&mut state.matrix, &left, &parent.matrix);
    }
    if state.mode == RenderMode::Painting
        && parse_number(doc, element, ATTR_OPACITY, &mut state.opacity, true, false)
    {
        state.opacity *= parent.opacity;
    }
    context.stack.push(element);
    state
}

/// `render_state_end`
fn render_state_end(
    state: &RenderState,
    parent: &mut RenderState,
    context: &mut RenderContext<'_, '_, '_>,
) {
    context.stack.pop();
    if state.mode == RenderMode::Painting {
        return;
    }
    if is_invalid_rect(&state.extents) {
        return;
    }

    let mut matrix = Matrix::default();
    plutovg_matrix_invert(&parent.matrix, Some(&mut matrix));
    let left = matrix;
    plutovg_matrix_multiply(&mut matrix, &left, &state.matrix);

    let extents = plutovg_matrix_map_rect(&matrix, &state.extents);
    if is_invalid_rect(&parent.extents) {
        parent.extents = extents;
        return;
    }

    let l = min(parent.extents.x, extents.x);
    let t = min(parent.extents.y, extents.y);
    let r = max(parent.extents.x + parent.extents.w, extents.x + extents.w);
    let b = max(parent.extents.y + parent.extents.h, extents.y + extents.h);

    parent.extents.x = l;
    parent.extents.y = t;
    parent.extents.w = r - l;
    parent.extents.h = b - t;
}

/// `has_cycle_reference`
fn has_cycle_reference(context: &RenderContext<'_, '_, '_>, element: u32) -> bool {
    context.stack.contains(&element)
}

/// `resolve_length`
fn resolve_length(state: &RenderState, length: &Length, mode: u8) -> f32 {
    let mut maximum = 0.0;
    if length.type_ == LengthType::Percent {
        if mode == b'x' {
            maximum = state.view_width;
        } else if mode == b'y' {
            maximum = state.view_height;
        } else if mode == b'o' {
            maximum = hypotf(state.view_width, state.view_height) / PLUTOVG_SQRT2;
        }
    }

    convert_length(length, maximum)
}

/// `find_element`
fn find_element(document: &Document, id: &[u8]) -> Option<u32> {
    if let Some(id_cache) = &document.id_cache {
        if !id.is_empty() {
            return id_cache.get(id).copied();
        }
    }
    None
}

/// `resolve_href`
fn resolve_href(document: &Document, element: u32) -> Option<u32> {
    let value = find_attribute(document, element, ATTR_XLINK_HREF, false)?;
    let v = document.s(value);
    if v.len() > 1 && v[0] == b'#' {
        return find_element(document, &v[1..]);
    }

    None
}

/// `convert_color`
fn convert_color(color: &SvgColor) -> Color {
    let mut value = Color::default();
    plutovg_color_init_argb32(&mut value, color.value);
    value
}

/// `resolve_current_color`
fn resolve_current_color(context: &RenderContext<'_, '_, '_>, element: u32) -> Color {
    let doc = context.document;
    let mut element = element;
    let mut steps = 0;
    loop {
        /* (see find_attribute's FIXME) */
        steps += 1;
        if steps > doc.elements.len() {
            return PLUTOVG_BLACK_COLOR;
        }
        let mut color = SvgColor {
            type_: ColorType::Current,
            value: 0,
        };
        parse_color(doc, element, ATTR_COLOR, &mut color, true);
        if color.type_ == ColorType::Fixed {
            return convert_color(&color);
        }
        match doc.el(element).parent.get() {
            None => {
                if let Some(current_color) = context.current_color {
                    return current_color;
                }
                return PLUTOVG_BLACK_COLOR;
            }
            Some(parent) => element = parent,
        }
    }
}

/// `resolve_color`
fn resolve_color(context: &RenderContext<'_, '_, '_>, element: u32, color: &SvgColor) -> Color {
    if color.type_ == ColorType::Fixed {
        return convert_color(color);
    }
    resolve_current_color(context, element)
}

const MAX_STOPS: usize = 64;

/// `resolve_gradient_stops`
fn resolve_gradient_stops(
    context: &RenderContext<'_, '_, '_>,
    element: u32,
    stops: &mut Vec<GradientStop>,
) {
    let doc = context.document;
    let mut child = doc.el(element).first_child;
    while let Some(c) = child {
        if stops.len() >= MAX_STOPS {
            break;
        }
        if doc.el(c).id == TAG_STOP {
            let mut offset = 0.0;
            let mut stop_opacity = 1.0;
            let mut stop_color = SvgColor {
                type_: ColorType::Fixed,
                value: 0xFF000000,
            };

            parse_number(doc, c, ATTR_OFFSET, &mut offset, true, false);
            parse_number(doc, c, ATTR_STOP_OPACITY, &mut stop_opacity, true, false);
            parse_color(doc, c, ATTR_STOP_COLOR, &mut stop_color, false);

            let mut color = resolve_color(context, c, &stop_color);
            color.a *= stop_opacity;
            stops.push(GradientStop { offset, color });
        }

        child = doc.el(c).next_sibling;
    }
}

/// `resolve_gradient_length`
fn resolve_gradient_length(
    state: &RenderState,
    length: &Length,
    units: UnitsType,
    mode: u8,
) -> f32 {
    if units == UnitsType::UserSpaceOnUse {
        return resolve_length(state, length, mode);
    }
    convert_length(length, 1.0)
}

/// `gradient_attributes_t`
#[derive(Debug, Clone, Copy, Default)]
struct GradientAttributes {
    units: Option<u32>,
    spread: Option<u32>,
    transform: Option<u32>,
    stops: Option<u32>,
}

/// `collect_gradient_attributes`
fn collect_gradient_attributes(doc: &Document, element: u32, attributes: &mut GradientAttributes) {
    if attributes.units.is_none() && has_attribute(doc, element, ATTR_GRADIENT_UNITS) {
        attributes.units = Some(element);
    }
    if attributes.spread.is_none() && has_attribute(doc, element, ATTR_SPREAD_METHOD) {
        attributes.spread = Some(element);
    }
    if attributes.transform.is_none() && has_attribute(doc, element, ATTR_GRADIENT_TRANSFORM) {
        attributes.transform = Some(element);
    }
    if attributes.stops.is_none() {
        let mut child = doc.el(element).first_child;
        while let Some(c) = child {
            if doc.el(c).id == TAG_STOP {
                attributes.stops = Some(element);
                break;
            }
            child = doc.el(c).next_sibling;
        }
    }
}

/// `fill_gradient_attributes`
fn fill_gradient_attributes(element: u32, attributes: &mut GradientAttributes) {
    if attributes.units.is_none() {
        attributes.units = Some(element);
    }
    if attributes.spread.is_none() {
        attributes.spread = Some(element);
    }
    if attributes.transform.is_none() {
        attributes.transform = Some(element);
    }
    if attributes.stops.is_none() {
        attributes.stops = Some(element);
    }
}

/// `resolve_gradient_attributes`
fn resolve_gradient_attributes(
    context: &RenderContext<'_, '_, '_>,
    state: &RenderState,
    attributes: &GradientAttributes,
    units: &mut UnitsType,
    spread: &mut SpreadMethod,
    transform: &mut Matrix,
    stops: &mut Vec<GradientStop>,
) {
    let doc = context.document;
    parse_units_type(
        doc,
        attributes.units.unwrap_or(0),
        ATTR_GRADIENT_UNITS,
        units,
    );
    parse_spread_method(
        doc,
        attributes.spread.unwrap_or(0),
        ATTR_SPREAD_METHOD,
        spread,
    );
    parse_transform(
        doc,
        attributes.transform.unwrap_or(0),
        ATTR_GRADIENT_TRANSFORM,
        transform,
    );
    resolve_gradient_stops(context, attributes.stops.unwrap_or(0), stops);
    if *units == UnitsType::ObjectBoundingBox {
        let mut matrix = Matrix::default();
        plutovg_matrix_init_translate(&mut matrix, state.extents.x, state.extents.y);
        plutovg_matrix_scale(&mut matrix, state.extents.w, state.extents.h);
        let left = *transform;
        plutovg_matrix_multiply(transform, &left, &matrix);
    }
}

const MAX_GRADIENT_DEPTH: i32 = 128;

/// `apply_linear_gradient`
fn apply_linear_gradient(
    state: &RenderState,
    context: &mut RenderContext<'_, '_, '_>,
    element: u32,
) -> bool {
    let doc = context.document;
    let mut base = GradientAttributes::default();
    let (mut ax1, mut ay1, mut ax2, mut ay2): (Option<u32>, Option<u32>, Option<u32>, Option<u32>) =
        (None, None, None, None);
    let mut current = element;
    for _ in 0..MAX_GRADIENT_DEPTH {
        collect_gradient_attributes(doc, current, &mut base);
        if doc.el(current).id == TAG_LINEAR_GRADIENT {
            if ax1.is_none() && has_attribute(doc, current, ATTR_X1) {
                ax1 = Some(current);
            }
            if ay1.is_none() && has_attribute(doc, current, ATTR_Y1) {
                ay1 = Some(current);
            }
            if ax2.is_none() && has_attribute(doc, current, ATTR_X2) {
                ax2 = Some(current);
            }
            if ay2.is_none() && has_attribute(doc, current, ATTR_Y2) {
                ay2 = Some(current);
            }
        }

        let r = resolve_href(doc, current);
        match r {
            Some(r)
                if doc.el(r).id == TAG_LINEAR_GRADIENT || doc.el(r).id == TAG_RADIAL_GRADIENT =>
            {
                current = r
            }
            _ => break,
        }
    }

    if base.stops.is_none() {
        return false;
    }
    fill_gradient_attributes(element, &mut base);
    let ax1 = ax1.unwrap_or(element);
    let ay1 = ay1.unwrap_or(element);
    let ax2 = ax2.unwrap_or(element);
    let ay2 = ay2.unwrap_or(element);

    let mut units = UnitsType::ObjectBoundingBox;
    let mut spread = SpreadMethod::Pad;
    let mut transform = Matrix {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };
    let mut stops = Vec::new();
    resolve_gradient_attributes(
        context,
        state,
        &base,
        &mut units,
        &mut spread,
        &mut transform,
        &mut stops,
    );

    let mut x1 = fixed(0.0);
    let mut y1 = fixed(0.0);
    let mut x2 = percent(100.0);
    let mut y2 = fixed(0.0);

    parse_length(doc, ax1, ATTR_X1, &mut x1, true, false);
    parse_length(doc, ay1, ATTR_Y1, &mut y1, true, false);
    parse_length(doc, ax2, ATTR_X2, &mut x2, true, false);
    parse_length(doc, ay2, ATTR_Y2, &mut y2, true, false);

    let x1 = resolve_gradient_length(state, &x1, units, b'x');
    let y1 = resolve_gradient_length(state, &y1, units, b'y');
    let x2 = resolve_gradient_length(state, &x2, units, b'x');
    let y2 = resolve_gradient_length(state, &y2, units, b'y');

    if let Some(canvas) = context.canvas.as_deref_mut() {
        plutovg_canvas_set_linear_gradient(
            canvas,
            x1,
            y1,
            x2,
            y2,
            spread,
            &stops,
            Some(&transform),
        );
    }
    true
}

/// `apply_radial_gradient`
fn apply_radial_gradient(
    state: &RenderState,
    context: &mut RenderContext<'_, '_, '_>,
    element: u32,
) -> bool {
    let doc = context.document;
    let mut base = GradientAttributes::default();
    let mut acx: Option<u32> = None;
    let mut acy: Option<u32> = None;
    let mut ar: Option<u32> = None;
    let mut afx: Option<u32> = None;
    let mut afy: Option<u32> = None;
    let mut current = element;
    for _ in 0..MAX_GRADIENT_DEPTH {
        collect_gradient_attributes(doc, current, &mut base);
        if doc.el(current).id == TAG_RADIAL_GRADIENT {
            if acx.is_none() && has_attribute(doc, current, ATTR_CX) {
                acx = Some(current);
            }
            if acy.is_none() && has_attribute(doc, current, ATTR_CY) {
                acy = Some(current);
            }
            if ar.is_none() && has_attribute(doc, current, ATTR_R) {
                ar = Some(current);
            }
            if afx.is_none() && has_attribute(doc, current, ATTR_FX) {
                afx = Some(current);
            }
            if afy.is_none() && has_attribute(doc, current, ATTR_FY) {
                afy = Some(current);
            }
        }

        let r = resolve_href(doc, current);
        match r {
            Some(r)
                if doc.el(r).id == TAG_LINEAR_GRADIENT || doc.el(r).id == TAG_RADIAL_GRADIENT =>
            {
                current = r
            }
            _ => break,
        }
    }

    if base.stops.is_none() {
        return false;
    }
    fill_gradient_attributes(element, &mut base);
    let acx = acx.unwrap_or(element);
    let acy = acy.unwrap_or(element);
    let ar = ar.unwrap_or(element);

    let mut units = UnitsType::ObjectBoundingBox;
    let mut spread = SpreadMethod::Pad;
    let mut transform = Matrix {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };
    let mut stops = Vec::new();
    resolve_gradient_attributes(
        context,
        state,
        &base,
        &mut units,
        &mut spread,
        &mut transform,
        &mut stops,
    );

    let mut cx = percent(50.0);
    let mut cy = percent(50.0);
    let mut r = percent(50.0);
    let mut fx = percent(50.0);
    let mut fy = percent(50.0);

    parse_length(doc, acx, ATTR_CX, &mut cx, true, false);
    parse_length(doc, acy, ATTR_CY, &mut cy, true, false);
    parse_length(doc, ar, ATTR_R, &mut r, false, false);

    if let Some(afx) = afx {
        parse_length(doc, afx, ATTR_FX, &mut fx, true, false);
    } else {
        parse_length(doc, acx, ATTR_CX, &mut fx, true, false);
    }

    if let Some(afy) = afy {
        parse_length(doc, afy, ATTR_FY, &mut fy, true, false);
    } else {
        parse_length(doc, acy, ATTR_CY, &mut fy, true, false);
    }

    let cx = resolve_gradient_length(state, &cx, units, b'x');
    let cy = resolve_gradient_length(state, &cy, units, b'y');
    let r = resolve_gradient_length(state, &r, units, b'o');
    let fx = resolve_gradient_length(state, &fx, units, b'x');
    let fy = resolve_gradient_length(state, &fy, units, b'y');

    if let Some(canvas) = context.canvas.as_deref_mut() {
        plutovg_canvas_set_radial_gradient(
            canvas,
            cx,
            cy,
            r,
            fx,
            fy,
            0.0,
            spread,
            &stops,
            Some(&transform),
        );
    }
    true
}

/// `apply_paint`
fn apply_paint(
    state: &RenderState,
    context: &mut RenderContext<'_, '_, '_>,
    paint: &SvgPaint,
) -> bool {
    if paint.type_ == PaintType::None {
        return false;
    }
    if paint.type_ == PaintType::Color {
        let color = resolve_color(context, state.element, &paint.color);
        if let Some(canvas) = context.canvas.as_deref_mut() {
            plutovg_canvas_set_color(canvas, &color);
        }
        return true;
    }

    let doc = context.document;
    if paint.type_ == PaintType::Var {
        let name = doc.s(paint.id);
        let found = match context.palette_func.as_deref_mut() {
            Some(palette_func) => palette_func(name),
            None => None,
        };
        let color = match found {
            Some(color) => color,
            None => resolve_color(context, state.element, &paint.color),
        };
        if let Some(canvas) = context.canvas.as_deref_mut() {
            plutovg_canvas_set_color(canvas, &color);
        }
        return true;
    }

    let Some(r) = find_element(doc, doc.s(paint.id)) else {
        let color = resolve_color(context, state.element, &paint.color);
        if let Some(canvas) = context.canvas.as_deref_mut() {
            plutovg_canvas_set_color(canvas, &color);
        }
        return true;
    };

    if doc.el(r).id == TAG_LINEAR_GRADIENT {
        return apply_linear_gradient(state, context, r);
    }
    if doc.el(r).id == TAG_RADIAL_GRADIENT {
        return apply_radial_gradient(state, context, r);
    }
    false
}

/// `draw_shape`
fn draw_shape(element: u32, context: &mut RenderContext<'_, '_, '_>, state: &mut RenderState) {
    let doc = context.document;
    let mut stroke = SvgPaint {
        type_: PaintType::None,
        color: SvgColor {
            type_: ColorType::Fixed,
            value: 0,
        },
        id: Str::default(),
    };
    parse_paint(doc, element, ATTR_STROKE, &mut stroke);

    let mut stroke_width = fixed(1.0);
    let mut line_cap = LineCap::Butt;
    let mut line_join = LineJoin::Miter;
    let mut miter_limit = 4.0;

    if stroke.type_ > PaintType::None {
        parse_length(
            doc,
            element,
            ATTR_STROKE_WIDTH,
            &mut stroke_width,
            false,
            true,
        );
        parse_line_cap(doc, element, ATTR_STROKE_LINECAP, &mut line_cap);
        parse_line_join(doc, element, ATTR_STROKE_LINEJOIN, &mut line_join);
        parse_number(
            doc,
            element,
            ATTR_STROKE_MITERLIMIT,
            &mut miter_limit,
            false,
            true,
        );
    }

    if state.mode == RenderMode::Bounding {
        if stroke.type_ == PaintType::None {
            return;
        }
        let line_width = resolve_length(state, &stroke_width, b'o');
        let mut cap_limit = line_width / 2.0;
        if line_cap == LineCap::Square {
            cap_limit *= PLUTOVG_SQRT2;
        }
        let mut join_limit = line_width / 2.0;
        if line_join == LineJoin::Miter {
            join_limit *= miter_limit;
        }

        let delta = max(cap_limit, join_limit);
        state.extents.x -= delta;
        state.extents.y -= delta;
        state.extents.w += delta * 2.0;
        state.extents.h += delta * 2.0;
        return;
    }

    let mut fill = SvgPaint {
        type_: PaintType::Color,
        color: SvgColor {
            type_: ColorType::Fixed,
            value: 0xFF000000,
        },
        id: Str::default(),
    };
    parse_paint(doc, element, ATTR_FILL, &mut fill);

    if apply_paint(state, context, &fill) {
        let mut fill_opacity = 1.0;
        parse_number(
            doc,
            element,
            ATTR_FILL_OPACITY,
            &mut fill_opacity,
            true,
            true,
        );

        let mut fill_rule = FillRule::NonZero;
        parse_fill_rule(doc, element, ATTR_FILL_RULE, &mut fill_rule);

        if let Some(canvas) = context.canvas.as_deref_mut() {
            plutovg_canvas_set_fill_rule(canvas, fill_rule);
            plutovg_canvas_set_opacity(canvas, fill_opacity * state.opacity);
            plutovg_canvas_set_matrix(canvas, Some(&state.matrix));
            plutovg_canvas_fill_path(canvas, &doc.path.borrow());
        }
    }

    if apply_paint(state, context, &stroke) {
        let mut stroke_opacity = 1.0;
        parse_number(
            doc,
            element,
            ATTR_STROKE_OPACITY,
            &mut stroke_opacity,
            true,
            true,
        );

        let mut dash_offset = fixed(0.0);
        parse_length(
            doc,
            element,
            ATTR_STROKE_DASHOFFSET,
            &mut dash_offset,
            false,
            true,
        );

        let mut dash_array = StrokeDashArray {
            data: [fixed(0.0); MAX_DASHES],
            size: 0,
        };
        parse_dash_array(doc, element, ATTR_STROKE_DASHARRAY, &mut dash_array);

        let mut dashes = [0.0f32; MAX_DASHES];
        for (dash, length) in dashes.iter_mut().zip(&dash_array.data[..dash_array.size]) {
            *dash = resolve_length(state, length, b'o');
        }

        if let Some(canvas) = context.canvas.as_deref_mut() {
            plutovg_canvas_set_dash_offset(canvas, resolve_length(state, &dash_offset, b'o'));
            plutovg_canvas_set_dash_array(canvas, &dashes[..dash_array.size]);

            plutovg_canvas_set_line_width(canvas, resolve_length(state, &stroke_width, b'o'));
            plutovg_canvas_set_line_cap(canvas, line_cap);
            plutovg_canvas_set_line_join(canvas, line_join);
            plutovg_canvas_set_miter_limit(canvas, miter_limit);
            plutovg_canvas_set_opacity(canvas, stroke_opacity * state.opacity);
            plutovg_canvas_set_matrix(canvas, Some(&state.matrix));
            plutovg_canvas_stroke_path(canvas, &doc.path.borrow());
        }
    }
}

/// `is_display_none`
fn is_display_none(doc: &Document, element: u32) -> bool {
    let mut display = Display::Inline;
    parse_display(doc, element, ATTR_DISPLAY, &mut display);
    display == Display::None
}

/// `is_visibility_hidden`
fn is_visibility_hidden(doc: &Document, element: u32) -> bool {
    let mut visibility = Visibility::Visible;
    parse_visibility(doc, element, ATTR_VISIBILITY, &mut visibility);
    visibility != Visibility::Visible
}

/// `apply_view_transform`
fn apply_view_transform(doc: &Document, state: &mut RenderState, width: f32, height: f32) {
    let mut view_box = Rect::default();
    if !parse_view_box(doc, state.element, ATTR_VIEW_BOX, &mut view_box) {
        return;
    }
    let mut position = ViewPosition {
        align: ViewAlign::XMidYMid,
        scale: ViewScale::Meet,
    };
    parse_view_position(
        doc,
        state.element,
        ATTR_PRESERVE_ASPECT_RATIO,
        &mut position,
    );
    let scale_x = width / view_box.w;
    let scale_y = height / view_box.h;
    if position.align == ViewAlign::None {
        plutovg_matrix_scale(&mut state.matrix, scale_x, scale_y);
        plutovg_matrix_translate(&mut state.matrix, -view_box.x, -view_box.y);
    } else {
        let scale = if position.scale == ViewScale::Meet {
            min(scale_x, scale_y)
        } else {
            max(scale_x, scale_y)
        };
        let mut offset_x = -view_box.x * scale;
        let mut offset_y = -view_box.y * scale;
        let view_width = view_box.w * scale;
        let view_height = view_box.h * scale;
        match position.align {
            ViewAlign::XMidYMin | ViewAlign::XMidYMid | ViewAlign::XMidYMax => {
                offset_x += (width - view_width) * 0.5;
            }
            ViewAlign::XMaxYMin | ViewAlign::XMaxYMid | ViewAlign::XMaxYMax => {
                offset_x += width - view_width;
            }
            _ => {}
        }

        match position.align {
            ViewAlign::XMinYMid | ViewAlign::XMidYMid | ViewAlign::XMaxYMid => {
                offset_y += (height - view_height) * 0.5;
            }
            ViewAlign::XMinYMax | ViewAlign::XMidYMax | ViewAlign::XMaxYMax => {
                offset_y += height - view_height;
            }
            _ => {}
        }

        plutovg_matrix_translate(&mut state.matrix, offset_x, offset_y);
        plutovg_matrix_scale(&mut state.matrix, scale, scale);
    }

    state.view_width = view_box.w;
    state.view_height = view_box.h;
}

/// `render_symbol`
fn render_symbol(
    element: u32,
    context: &mut RenderContext<'_, '_, '_>,
    state: &mut RenderState,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) {
    let doc = context.document;
    if width <= 0.0 || height <= 0.0 || is_display_none(doc, element) {
        return;
    }
    let mut new_state = render_state_begin(element, state, context);

    new_state.view_width = width;
    new_state.view_height = height;
    plutovg_matrix_translate(&mut new_state.matrix, x, y);

    apply_view_transform(doc, &mut new_state, width, height);
    render_children(element, context, &mut new_state);
    render_state_end(&new_state, state, context);
}

/// `render_svg`
fn render_svg(element: u32, context: &mut RenderContext<'_, '_, '_>, state: &mut RenderState) {
    let doc = context.document;
    if doc.el(element).parent.get().is_none() {
        render_symbol(element, context, state, 0.0, 0.0, doc.width, doc.height);
        return;
    }

    let mut x = fixed(0.0);
    let mut y = fixed(0.0);

    let mut w = percent(100.0);
    let mut h = percent(100.0);

    parse_length(doc, element, ATTR_X, &mut x, true, false);
    parse_length(doc, element, ATTR_Y, &mut y, true, false);
    parse_length(doc, element, ATTR_WIDTH, &mut w, false, false);
    parse_length(doc, element, ATTR_HEIGHT, &mut h, false, false);

    let x = resolve_length(state, &x, b'x');
    let y = resolve_length(state, &y, b'y');
    let w = resolve_length(state, &w, b'x');
    let h = resolve_length(state, &h, b'y');
    render_symbol(element, context, state, x, y, w, h);
}

/// `render_use`
fn render_use(element: u32, context: &mut RenderContext<'_, '_, '_>, state: &mut RenderState) {
    let doc = context.document;
    if is_display_none(doc, element) || has_cycle_reference(context, element) {
        return;
    }
    let Some(r) = resolve_href(doc, element) else {
        return;
    };
    let mut x = fixed(0.0);
    let mut y = fixed(0.0);

    parse_length(doc, element, ATTR_X, &mut x, true, false);
    parse_length(doc, element, ATTR_Y, &mut y, true, false);

    let x = resolve_length(state, &x, b'x');
    let y = resolve_length(state, &y, b'y');

    let mut new_state = render_state_begin(element, state, context);
    plutovg_matrix_translate(&mut new_state.matrix, x, y);

    let parent = doc.el(r).parent.get();
    doc.el(r).parent.set(Some(element));
    if doc.el(r).id == TAG_SVG || doc.el(r).id == TAG_SYMBOL {
        render_svg(r, context, &mut new_state);
    } else {
        render_element(r, context, &mut new_state);
    }

    doc.el(r).parent.set(parent);
    render_state_end(&new_state, state, context);
}

/// `render_g`
fn render_g(element: u32, context: &mut RenderContext<'_, '_, '_>, state: &mut RenderState) {
    let doc = context.document;
    if is_display_none(doc, element) {
        return;
    }
    let mut new_state = render_state_begin(element, state, context);
    render_children(element, context, &mut new_state);
    render_state_end(&new_state, state, context);
}

/// `render_line`
fn render_line(element: u32, context: &mut RenderContext<'_, '_, '_>, state: &mut RenderState) {
    let doc = context.document;
    if is_display_none(doc, element) || is_visibility_hidden(doc, element) {
        return;
    }
    let mut x1 = fixed(0.0);
    let mut y1 = fixed(0.0);
    let mut x2 = fixed(0.0);
    let mut y2 = fixed(0.0);

    parse_length(doc, element, ATTR_X1, &mut x1, true, false);
    parse_length(doc, element, ATTR_Y1, &mut y1, true, false);
    parse_length(doc, element, ATTR_X2, &mut x2, true, false);
    parse_length(doc, element, ATTR_Y2, &mut y2, true, false);

    let x1 = resolve_length(state, &x1, b'x');
    let y1 = resolve_length(state, &y1, b'y');
    let x2 = resolve_length(state, &x2, b'x');
    let y2 = resolve_length(state, &y2, b'y');

    let mut new_state = render_state_begin(element, state, context);

    new_state.extents.x = min(x1, x2);
    new_state.extents.y = min(y1, y2);
    new_state.extents.w = (x2 - x1).abs();
    new_state.extents.h = (y2 - y1).abs();

    {
        let mut path = doc.path.borrow_mut();
        plutovg_path_reset(&mut path);
        plutovg_path_move_to(&mut path, x1, y1);
        plutovg_path_line_to(&mut path, x2, y2);
    }
    draw_shape(element, context, &mut new_state);
    render_state_end(&new_state, state, context);
}

/// `render_ellipse`
fn render_ellipse(element: u32, context: &mut RenderContext<'_, '_, '_>, state: &mut RenderState) {
    let doc = context.document;
    if is_display_none(doc, element) || is_visibility_hidden(doc, element) {
        return;
    }
    let mut rx = fixed(0.0);
    let mut ry = fixed(0.0);

    parse_length(doc, element, ATTR_RX, &mut rx, false, false);
    parse_length(doc, element, ATTR_RY, &mut ry, false, false);
    if is_length_zero(&rx) || is_length_zero(&ry) {
        return;
    }
    let mut cx = fixed(0.0);
    let mut cy = fixed(0.0);

    parse_length(doc, element, ATTR_CX, &mut cx, true, false);
    parse_length(doc, element, ATTR_CY, &mut cy, true, false);

    let cx = resolve_length(state, &cx, b'x');
    let cy = resolve_length(state, &cy, b'y');
    let rx = resolve_length(state, &rx, b'x');
    let ry = resolve_length(state, &ry, b'y');

    let mut new_state = render_state_begin(element, state, context);

    new_state.extents.x = cx - rx;
    new_state.extents.y = cy - ry;
    new_state.extents.w = rx + rx;
    new_state.extents.h = ry + ry;

    {
        let mut path = doc.path.borrow_mut();
        plutovg_path_reset(&mut path);
        plutovg_path_add_ellipse(&mut path, cx, cy, rx, ry);
    }
    draw_shape(element, context, &mut new_state);
    render_state_end(&new_state, state, context);
}

/// `render_circle`
fn render_circle(element: u32, context: &mut RenderContext<'_, '_, '_>, state: &mut RenderState) {
    let doc = context.document;
    if is_display_none(doc, element) || is_visibility_hidden(doc, element) {
        return;
    }
    let mut r = fixed(0.0);
    parse_length(doc, element, ATTR_R, &mut r, false, false);
    if is_length_zero(&r) {
        return;
    }
    let mut cx = fixed(0.0);
    let mut cy = fixed(0.0);

    parse_length(doc, element, ATTR_CX, &mut cx, true, false);
    parse_length(doc, element, ATTR_CY, &mut cy, true, false);

    let cx = resolve_length(state, &cx, b'x');
    let cy = resolve_length(state, &cy, b'y');
    let r = resolve_length(state, &r, b'o');

    let mut new_state = render_state_begin(element, state, context);

    new_state.extents.x = cx - r;
    new_state.extents.y = cy - r;
    new_state.extents.w = r + r;
    new_state.extents.h = r + r;

    {
        let mut path = doc.path.borrow_mut();
        plutovg_path_reset(&mut path);
        plutovg_path_add_circle(&mut path, cx, cy, r);
    }
    draw_shape(element, context, &mut new_state);
    render_state_end(&new_state, state, context);
}

/// `render_rect`
fn render_rect(element: u32, context: &mut RenderContext<'_, '_, '_>, state: &mut RenderState) {
    let doc = context.document;
    if is_display_none(doc, element) || is_visibility_hidden(doc, element) {
        return;
    }
    let mut w = fixed(0.0);
    let mut h = fixed(0.0);

    parse_length(doc, element, ATTR_WIDTH, &mut w, false, false);
    parse_length(doc, element, ATTR_HEIGHT, &mut h, false, false);
    if is_length_zero(&w) || is_length_zero(&h) {
        return;
    }
    let mut x = fixed(0.0);
    let mut y = fixed(0.0);

    parse_length(doc, element, ATTR_X, &mut x, true, false);
    parse_length(doc, element, ATTR_Y, &mut y, true, false);

    let x = resolve_length(state, &x, b'x');
    let y = resolve_length(state, &y, b'y');
    let w = resolve_length(state, &w, b'x');
    let h = resolve_length(state, &h, b'y');

    let mut rx = Length {
        value: 0.0,
        type_: LengthType::Unknown,
    };
    let mut ry = Length {
        value: 0.0,
        type_: LengthType::Unknown,
    };

    parse_length(doc, element, ATTR_RX, &mut rx, false, false);
    parse_length(doc, element, ATTR_RY, &mut ry, false, false);

    let mut rxv = resolve_length(state, &rx, b'x');
    let mut ryv = resolve_length(state, &ry, b'y');

    if !is_length_valid(&rx) {
        rxv = ryv;
    }
    if !is_length_valid(&ry) {
        ryv = rxv;
    }

    let mut new_state = render_state_begin(element, state, context);

    new_state.extents.x = x;
    new_state.extents.y = y;
    new_state.extents.w = w;
    new_state.extents.h = h;

    {
        let mut path = doc.path.borrow_mut();
        plutovg_path_reset(&mut path);
        plutovg_path_add_round_rect(&mut path, x, y, w, h, rxv, ryv);
    }
    draw_shape(element, context, &mut new_state);
    render_state_end(&new_state, state, context);
}

/// `render_poly`
fn render_poly(element: u32, context: &mut RenderContext<'_, '_, '_>, state: &mut RenderState) {
    let doc = context.document;
    if is_display_none(doc, element) || is_visibility_hidden(doc, element) {
        return;
    }
    let mut new_state = render_state_begin(element, state, context);

    {
        let mut path = doc.path.borrow_mut();
        plutovg_path_reset(&mut path);
        parse_points(doc, element, ATTR_POINTS, &mut path);
        plutovg_path_extents(&path, Some(&mut new_state.extents), false);
    }
    draw_shape(element, context, &mut new_state);
    render_state_end(&new_state, state, context);
}

/// `render_path`
fn render_path(element: u32, context: &mut RenderContext<'_, '_, '_>, state: &mut RenderState) {
    let doc = context.document;
    if is_display_none(doc, element) || is_visibility_hidden(doc, element) {
        return;
    }
    let mut new_state = render_state_begin(element, state, context);

    {
        let mut path = doc.path.borrow_mut();
        plutovg_path_reset(&mut path);
        parse_path(doc, element, ATTR_D, &mut path);
        plutovg_path_extents(&path, Some(&mut new_state.extents), false);
    }
    draw_shape(element, context, &mut new_state);
    render_state_end(&new_state, state, context);
}

/// `transform_view_rect`
fn transform_view_rect(position: &ViewPosition, dst_rect: &mut Rect, src_rect: &mut Rect) {
    if position.align == ViewAlign::None {
        return;
    }
    let view_width = dst_rect.w;
    let view_height = dst_rect.h;
    let image_width = src_rect.w;
    let image_height = src_rect.h;
    if position.scale == ViewScale::Meet {
        let scale = image_height / image_width;
        if view_height > view_width * scale {
            dst_rect.h = view_width * scale;
            match position.align {
                ViewAlign::XMinYMid | ViewAlign::XMidYMid | ViewAlign::XMaxYMid => {
                    dst_rect.y += (view_height - dst_rect.h) * 0.5;
                }
                ViewAlign::XMinYMax | ViewAlign::XMidYMax | ViewAlign::XMaxYMax => {
                    dst_rect.y += view_height - dst_rect.h;
                }
                _ => {}
            }
        }

        if view_width > view_height / scale {
            dst_rect.w = view_height / scale;
            match position.align {
                ViewAlign::XMidYMin | ViewAlign::XMidYMid | ViewAlign::XMidYMax => {
                    dst_rect.x += (view_width - dst_rect.w) * 0.5;
                }
                ViewAlign::XMaxYMin | ViewAlign::XMaxYMid | ViewAlign::XMaxYMax => {
                    dst_rect.x += view_width - dst_rect.w;
                }
                _ => {}
            }
        }
    } else if position.scale == ViewScale::Slice {
        let scale = image_height / image_width;
        if view_height < view_width * scale {
            src_rect.h = view_height * (image_width / view_width);
            match position.align {
                ViewAlign::XMinYMid | ViewAlign::XMidYMid | ViewAlign::XMaxYMid => {
                    src_rect.y += (image_height - src_rect.h) * 0.5;
                }
                ViewAlign::XMinYMax | ViewAlign::XMidYMax | ViewAlign::XMaxYMax => {
                    src_rect.y += image_height - src_rect.h;
                }
                _ => {}
            }
        }

        if view_width < view_height / scale {
            src_rect.w = view_width * (image_height / view_height);
            match position.align {
                ViewAlign::XMidYMin | ViewAlign::XMidYMid | ViewAlign::XMidYMax => {
                    src_rect.x += (image_width - src_rect.w) * 0.5;
                }
                ViewAlign::XMaxYMin | ViewAlign::XMaxYMid | ViewAlign::XMaxYMax => {
                    src_rect.x += image_width - src_rect.w;
                }
                _ => {}
            }
        }
    }
}

/// `load_image`
fn load_image(doc: &Document, element: u32) -> Option<Surface> {
    let value = find_attribute(doc, element, ATTR_XLINK_HREF, false)?;
    let d = &doc.data;
    let mut it = value.data;
    let end = it + value.length;
    if !skip_string(d, &mut it, end, b"data:image/png")
        && !skip_string(d, &mut it, end, b"data:image/jpg")
        && !skip_string(d, &mut it, end, b"data:image/jpeg")
    {
        return None;
    }

    if skip_string(d, &mut it, end, b";base64,") {
        return plutovg_surface_load_from_image_base64(&d[it..end]);
    }
    None
}

/// `draw_image`
fn draw_image(
    element: u32,
    context: &mut RenderContext<'_, '_, '_>,
    state: &RenderState,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) {
    if state.mode == RenderMode::Bounding {
        return;
    }
    let doc = context.document;
    let Some(image) = load_image(doc, element) else {
        return;
    };
    let image_width = plutovg_surface_get_width(&image) as f32;
    let image_height = plutovg_surface_get_height(&image) as f32;

    let mut dst_rect = Rect {
        x,
        y,
        w: width,
        h: height,
    };
    let mut src_rect = Rect {
        x: 0.0,
        y: 0.0,
        w: image_width,
        h: image_height,
    };
    let mut position = ViewPosition {
        align: ViewAlign::XMidYMid,
        scale: ViewScale::Meet,
    };

    parse_view_position(doc, element, ATTR_PRESERVE_ASPECT_RATIO, &mut position);
    transform_view_rect(&position, &mut dst_rect, &mut src_rect);

    let scale_x = dst_rect.w / src_rect.w;
    let scale_y = dst_rect.h / src_rect.h;
    let matrix = Matrix {
        a: scale_x,
        b: 0.0,
        c: 0.0,
        d: scale_y,
        e: -src_rect.x * scale_x,
        f: -src_rect.y * scale_y,
    };

    if let Some(canvas) = context.canvas.as_deref_mut() {
        plutovg_canvas_set_fill_rule(canvas, FillRule::NonZero);
        plutovg_canvas_set_opacity(canvas, state.opacity);
        plutovg_canvas_set_matrix(canvas, Some(&state.matrix));
        plutovg_canvas_translate(canvas, dst_rect.x, dst_rect.y);
        plutovg_canvas_set_texture(
            canvas,
            Some(std::rc::Rc::new(image)),
            TextureType::Plain,
            1.0,
            Some(&matrix),
        );
        plutovg_canvas_fill_rect(canvas, 0.0, 0.0, dst_rect.w, dst_rect.h);
    }
}

/// `render_image`
fn render_image(element: u32, context: &mut RenderContext<'_, '_, '_>, state: &mut RenderState) {
    let doc = context.document;
    if is_display_none(doc, element) || is_visibility_hidden(doc, element) {
        return;
    }
    let mut w = fixed(0.0);
    let mut h = fixed(0.0);

    parse_length(doc, element, ATTR_WIDTH, &mut w, false, false);
    parse_length(doc, element, ATTR_HEIGHT, &mut h, false, false);
    if is_length_zero(&w) || is_length_zero(&h) {
        return;
    }
    let mut x = fixed(0.0);
    let mut y = fixed(0.0);

    parse_length(doc, element, ATTR_X, &mut x, true, false);
    parse_length(doc, element, ATTR_Y, &mut y, true, false);

    let x = resolve_length(state, &x, b'x');
    let y = resolve_length(state, &y, b'y');
    let w = resolve_length(state, &w, b'x');
    let h = resolve_length(state, &h, b'y');

    let mut new_state = render_state_begin(element, state, context);

    new_state.extents.x = x;
    new_state.extents.y = y;
    new_state.extents.w = w;
    new_state.extents.h = h;
    draw_image(element, context, &new_state, x, y, w, h);
    render_state_end(&new_state, state, context);
}

/// `render_element`
fn render_element(element: u32, context: &mut RenderContext<'_, '_, '_>, state: &mut RenderState) {
    if context.stack.len() > MAX_RENDER_DEPTH {
        return;
    }
    match context.document.el(element).id {
        TAG_SVG => render_svg(element, context, state),
        TAG_USE => render_use(element, context, state),
        TAG_G => render_g(element, context, state),
        TAG_LINE => render_line(element, context, state),
        TAG_ELLIPSE => render_ellipse(element, context, state),
        TAG_CIRCLE => render_circle(element, context, state),
        TAG_RECT => render_rect(element, context, state),
        TAG_POLYLINE | TAG_POLYGON => render_poly(element, context, state),
        TAG_PATH => render_path(element, context, state),
        TAG_IMAGE => render_image(element, context, state),
        _ => {}
    }
}

/// `render_children`
fn render_children(element: u32, context: &mut RenderContext<'_, '_, '_>, state: &mut RenderState) {
    let mut child = context.document.el(element).first_child;
    while let Some(c) = child {
        render_element(c, context, state);
        child = context.document.el(c).next_sibling;
    }
}

/// Renders the document (or its element `id`) onto `canvas`, with the
/// current color and palette. Returns `false` if `id` is not found.
pub fn plutosvg_document_render(
    document: &Document,
    id: Option<&[u8]>,
    canvas: &mut Canvas,
    current_color: Option<&Color>,
    palette_func: Option<&mut PaletteFunc<'_>>,
) -> bool {
    let element = match id {
        None => document.root_element.unwrap_or(0),
        Some(id) => match find_element(document, id) {
            None => return false,
            Some(e) => e,
        },
    };
    let mut state = RenderState {
        element,
        mode: RenderMode::Painting,
        opacity: 1.0,
        extents: INVALID_RECT,
        view_width: document.width,
        view_height: document.height,
        matrix: plutovg_canvas_get_matrix(canvas),
    };

    let mut context = RenderContext {
        document,
        canvas: Some(canvas),
        current_color: current_color.copied(),
        palette_func,
        stack: vec![element],
    };
    render_element(state.element, &mut context, &mut state);
    true
}

/// Renders the document (or its element `id`) to a new surface of the
/// given size (or its intrinsic size if `width` and `height` are not
/// positive).
pub fn plutosvg_document_render_to_surface(
    document: &Document,
    id: Option<&[u8]>,
    mut width: i32,
    mut height: i32,
    current_color: Option<&Color>,
    palette_func: Option<&mut PaletteFunc<'_>>,
) -> Option<Surface> {
    let mut extents = Rect {
        x: 0.0,
        y: 0.0,
        w: document.width,
        h: document.height,
    };
    if id.is_some() && !plutosvg_document_extents(document, id, &mut extents) {
        return None;
    }
    if extents.w <= 0.0 || extents.h <= 0.0 {
        return None;
    }
    if width <= 0 && height <= 0 {
        width = cvt_f32_i32(extents.w.ceil());
        height = cvt_f32_i32(extents.h.ceil());
    } else if width > 0 && height <= 0 {
        height = cvt_f32_i32((width as f32 * extents.h / extents.w).ceil());
    } else if height > 0 && width <= 0 {
        width = cvt_f32_i32((height as f32 * extents.w / extents.h).ceil());
    }

    let surface = plutovg_surface_create(width, height)?;
    let mut canvas = plutovg_canvas_create(surface);
    plutovg_canvas_scale(
        &mut canvas,
        width as f32 / extents.w,
        height as f32 / extents.h,
    );
    plutovg_canvas_translate(&mut canvas, -extents.x, -extents.y);
    if !plutosvg_document_render(document, id, &mut canvas, current_color, palette_func) {
        return None;
    }

    Some(plutovg_canvas_into_surface(canvas))
}

/// Returns the intrinsic width of the document.
pub fn plutosvg_document_get_width(document: &Document) -> f32 {
    document.width
}

/// Returns the intrinsic height of the document.
pub fn plutosvg_document_get_height(document: &Document) -> f32 {
    document.height
}

/// Computes the bounding box of the document (or of its element `id`).
/// Returns `false` if `id` is not found.
pub fn plutosvg_document_extents(
    document: &Document,
    id: Option<&[u8]>,
    extents: &mut Rect,
) -> bool {
    let element = match id {
        None => document.root_element.unwrap_or(0),
        Some(id) => match find_element(document, id) {
            None => {
                *extents = EMPTY_RECT;
                return false;
            }
            Some(e) => e,
        },
    };
    let mut state = RenderState {
        element,
        mode: RenderMode::Bounding,
        opacity: 1.0,
        extents: INVALID_RECT,
        view_width: document.width,
        view_height: document.height,
        matrix: PLUTOVG_IDENTITY_MATRIX,
    };

    let mut context = RenderContext {
        document,
        canvas: None,
        current_color: None,
        palette_func: None,
        stack: vec![element],
    };
    render_element(state.element, &mut context, &mut state);
    if is_invalid_rect(&state.extents) {
        *extents = EMPTY_RECT;
    } else {
        *extents = state.extents;
    }

    true
}
